// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What arrives: direct messages of the group protocol (invitations,
//! requests, welcomes, keys) and the events of the groups themselves.
//!
//! Relays deliver in any order and more than once. Everything here can
//! be replayed: what cannot be understood yet waits in
//! `msg_group_pending` and is tried again when a key, the log, or a new
//! member shows up.

use crate::keys::{GroupKey, LinkSecret};
use crate::log::{KeyStatus, OpLog};
use crate::op::{GroupKind, KeyId, Op, OpBody};
use crate::roles::Role;
use crate::service::*;
use crate::wire::{self, InnerMessage, Opened, SecretEnvelope};
use messenger_core::envelope::{T_DELETE, T_EDIT, T_MEDIA, T_REACTION, T_RECEIPT_READ, T_TEXT};
use messenger_core::traits::{Body, Notice, UiEvent};
use messenger_core::{Context, DmInbound, Envelope, EventSource, GroupInbound, MessengerError, PubKey, Result};
use messenger_store::groups::{self as repo, GroupRow, PendingRow};
use messenger_store::messages::{self as msgs, NewMessage};
use messenger_store::chats;
use nostr::key::Keys;

/// Bucket of welcomes that arrived before my own consent did.
const BUCKET_WELCOME: &str = "welcome";
/// Opened, but the author is not known as a member yet.
const HELD: &str = "held:";
/// How long something of an unknown author is kept.
pub(crate) const HELD_TTL_SECS: i64 = 24 * 3600;
/// Clocks differ: a message may be dated a little before the join.
const JOIN_SLACK_SECS: i64 = 300;
/// Keys a device keeps for one group.
const MAX_KEYS: usize = 512;

struct Item {
    event_id: String,
    ciphertext: String,
    created_at: i64,
    historical: bool,
}

enum Verdict {
    Done,
    /// Try again when more of the group is known.
    Hold,
}

#[derive(PartialEq)]
enum Standing {
    Member,
    Muted,
    Outsider,
}

/// Was `who` a member (and allowed to speak) at the moment `at`?
fn standing(log: &OpLog, who: &PubKey, at: i64) -> Standing {
    let mut member_since: Option<i64> = None;
    let mut muted_since: Option<i64> = None;
    let mut result = Standing::Outsider;
    let close = |since: &mut Option<i64>, muted: &mut Option<i64>, end: i64, result: &mut Standing| {
        if let Some(start) = since.take() {
            if at >= start - JOIN_SLACK_SECS && at <= end {
                *result = if muted.is_some_and(|m| at >= m) { Standing::Muted } else { Standing::Member };
            }
        }
        *muted = None;
    };
    for op in log.ordered() {
        if log.rejection(&op.id()).is_some() {
            continue;
        }
        match &op.body {
            OpBody::Create { .. } | OpBody::Join if &op.author == who => member_since = member_since.or(Some(op.created_at)),
            OpBody::Admit { who: w } if w == who => member_since = member_since.or(Some(op.created_at)),
            OpBody::Leave if &op.author == who => close(&mut member_since, &mut muted_since, op.created_at, &mut result),
            OpBody::Remove { who: w } | OpBody::Ban { who: w } if w == who => {
                close(&mut member_since, &mut muted_since, op.created_at, &mut result)
            }
            OpBody::SetMuted { who: w, muted } if w == who => {
                // A muted stretch that ended still hides what was said in it.
                if *muted {
                    muted_since = muted_since.or(Some(op.created_at));
                } else if let Some(m) = muted_since.take() {
                    if at >= m && at < op.created_at {
                        return Standing::Muted;
                    }
                }
            }
            _ => {}
        }
    }
    close(&mut member_since, &mut muted_since, i64::MAX, &mut result);
    result
}

/// How I stopped being a member, by the last operation about me.
fn departure(log: &OpLog, me: &PubKey) -> &'static str {
    let mut how = MEMBERSHIP_REMOVED;
    for op in log.ordered() {
        if let Some(r) = log.rejection(&op.id()) {
            // I came by a link that was replaced meanwhile.
            if op.body == OpBody::Join && &op.author == me && r == &crate::state::Rejection::StaleLink {
                how = MEMBERSHIP_STALE;
            }
            continue;
        }
        match &op.body {
            OpBody::Leave if &op.author == me => how = MEMBERSHIP_LEFT,
            OpBody::Remove { who } if who == me => how = MEMBERSHIP_REMOVED,
            OpBody::Ban { who } if who == me => how = MEMBERSHIP_BANNED,
            _ => {}
        }
    }
    how
}

fn adds_people(body: &OpBody) -> bool {
    matches!(body, OpBody::Create { .. } | OpBody::Join | OpBody::Admit { .. } | OpBody::Unban { .. })
}

impl GroupService {
    // ─── Direct messages ────────────────────────────────────────────────────

    /// A direct message of the group protocol. `None` when the message
    /// is not one: it belongs to the chat then.
    pub async fn on_dm(&self, keys: &Keys, msg: &DmInbound, ctx: &Context) -> Result<Option<Outcome>> {
        let Ok(envelope) = Envelope::parse(&msg.content) else { return Ok(None) };
        if !envelope.t.starts_with("group.") {
            return Ok(None);
        }
        let me = &ctx.my_pubkey;
        let from_me = &msg.sender == me;
        if !from_me && self.dm.is_blocked(&msg.sender).await? {
            return Ok(Some(Outcome::default()));
        }
        let historical = msg.created_at < ctx.session_started_at || matches!(msg.envelope.source, EventSource::Sync { .. });
        let peer = if from_me { msg.recipients.iter().find(|p| *p != me).cloned() } else { Some(msg.sender.clone()) };
        let outcome = match envelope.t.as_str() {
            wire::T_INVITE => self.on_invite(wire::dm_body(&envelope)?, peer, from_me, historical).await,
            wire::T_INVITE_REPLY => self.on_invite_reply(keys, wire::dm_body(&envelope)?, peer, from_me, ctx).await,
            wire::T_JOIN_REQUEST => self.on_join_request(keys, wire::dm_body(&envelope)?, &msg.sender, from_me, historical, ctx).await,
            wire::T_REJECTED => self.on_rejected(wire::dm_body(&envelope)?, &msg.sender, from_me).await,
            wire::T_WELCOME => {
                let body: wire::Welcome = wire::dm_body(&envelope)?;
                self.on_welcome(keys, body, &msg.sender, msg.envelope.wire_id.as_hex(), msg.created_at.secs(), ctx).await
            }
            wire::T_KEYS => self.on_keys(keys, wire::dm_body(&envelope)?, &msg.sender, ctx).await,
            _ => Ok(Outcome::default()),
        };
        // A malformed message of someone else is theirs to fix, not ours
        // to fail on.
        Ok(Some(outcome.unwrap_or_else(|e| {
            Outcome { notes: vec![format!("group: direct message {} dropped: {e}", envelope.t)], ..Default::default() }
        })))
    }

    async fn on_invite(&self, body: wire::Invite, peer: Option<PubKey>, from_me: bool, historical: bool) -> Result<Outcome> {
        let Some(peer) = peer else { return Ok(Outcome::default()) };
        let now = self.now();
        if body.expires_at <= now || body.expires_at > body.created_at + wire::INVITE_TTL_SECS || body.group_id.len() != 64 {
            return Ok(Outcome::default());
        }
        if !from_me && repo::get(&self.store, &body.group_id).await?.is_some_and(|g| g.membership == MEMBERSHIP_JOINED) {
            return Ok(Outcome::default());
        }
        let new = repo::insert_invite(
            &self.store,
            &repo::InviteRow {
                invite_id: body.invite_id.clone(),
                group_id: body.group_id.clone(),
                direction: if from_me { "out".into() } else { "in".into() },
                peer: peer.as_hex().to_string(),
                status: if from_me { "sent".into() } else { "received".into() },
                payload_json: serde_json::to_string(&body)?,
                created_at: body.created_at,
                expires_at: body.expires_at,
                updated_at: 0,
            },
        )
        .await?;
        let mut out = Outcome::default();
        if !new {
            // The answer of my other device came first and left a stub:
            // now there is something to show for it.
            if let Some(row) = repo::invite(&self.store, &body.invite_id).await? {
                if row.payload_json == "{}" {
                    repo::set_invite_payload(&self.store, &body.invite_id, &serde_json::to_string(&body)?).await?;
                }
            }
            return Ok(out);
        }
        out.events.push(UiEvent {
            name: UI_EVENT_GROUP_INVITE.into(),
            payload: serde_json::json!({ "invite_id": body.invite_id, "group_id": body.group_id }),
        });
        if !from_me && !historical {
            out.notify.push(Notice {
                title: body.name.clone(),
                body: Some(Body::Invite { group_name: body.name.clone() }),
                chat_id: None,
                sender: Some(peer.as_hex().to_string()),
                request: false,
            });
        }
        Ok(out)
    }

    async fn on_invite_reply(
        &self,
        keys: &Keys,
        body: wire::InviteReply,
        peer: Option<PubKey>,
        from_me: bool,
        ctx: &Context,
    ) -> Result<Outcome> {
        let Some(peer) = peer else { return Ok(Outcome::default()) };
        let status = if body.accept { "accepted" } else { "declined" };
        if from_me {
            // Answered on another device of mine.
            match repo::invite(&self.store, &body.invite_id).await? {
                Some(row) if row.direction == "in" && row.status == "received" => {
                    repo::set_invite_status(&self.store, &body.invite_id, status).await?;
                }
                Some(_) => return Ok(Outcome::default()),
                None => {
                    let now = self.now();
                    repo::insert_invite(
                        &self.store,
                        &repo::InviteRow {
                            invite_id: body.invite_id.clone(),
                            group_id: body.group_id.clone(),
                            direction: "in".into(),
                            peer: peer.as_hex().to_string(),
                            status: status.into(),
                            payload_json: "{}".into(),
                            created_at: now,
                            expires_at: now + wire::INVITE_TTL_SECS,
                            updated_at: 0,
                        },
                    )
                    .await?;
                }
            }
            let mut out = Outcome { events: vec![Self::updated(&body.group_id)], ..Default::default() };
            if body.accept {
                out.merge(self.held_welcomes(keys, &body.group_id, ctx).await?);
            }
            return Ok(out);
        }
        let Some(row) = repo::invite(&self.store, &body.invite_id).await? else { return Ok(Outcome::default()) };
        if row.direction != "out" || row.peer != peer.as_hex() || row.group_id != body.group_id || row.status != "sent" {
            return Ok(Outcome::default());
        }
        if row.expires_at <= self.now() {
            repo::set_invite_status(&self.store, &body.invite_id, "expired").await?;
            return Ok(Outcome::default());
        }
        repo::set_invite_status(&self.store, &body.invite_id, status).await?;
        let mut out = Outcome { events: vec![Self::updated(&body.group_id)], ..Default::default() };
        if body.accept {
            let already = self.log(&body.group_id).await?.is_some_and(|l| l.state().is_member(&peer));
            if already {
                // Another device of mine, or another manager, was faster.
                out.publish.extend(self.welcome(keys, &body.group_id, &peer).await.unwrap_or_default());
            } else {
                match self.admit(keys, &body.group_id, &peer).await {
                    Ok(o) => out.merge(o),
                    Err(e) => out.notes.push(format!("group: an accepted invitation could not be honoured: {e}")),
                }
            }
        }
        Ok(out)
    }

    async fn on_join_request(
        &self,
        keys: &Keys,
        body: wire::JoinRequest,
        sender: &PubKey,
        from_me: bool,
        historical: bool,
        ctx: &Context,
    ) -> Result<Outcome> {
        if from_me {
            // Asked on another device of mine: the welcome is expected here too.
            let known = repo::request(&self.store, &body.group_id, sender.as_hex(), "out").await?;
            if known.is_none_or(|r| r.created_at < body.created_at && r.status != "pending") {
                repo::put_request(
                    &self.store,
                    &repo::RequestRow {
                        group_id: body.group_id.clone(),
                        requester: sender.as_hex().to_string(),
                        direction: "out".into(),
                        status: "pending".into(),
                        payload_json: "{}".into(),
                        created_at: body.created_at,
                        updated_at: 0,
                    },
                )
                .await?;
            }
            return self.held_welcomes(keys, &body.group_id, ctx).await;
        }
        let Some(row) = repo::get(&self.store, &body.group_id).await? else { return Ok(Outcome::default()) };
        let Some(log) = self.log(&body.group_id).await? else { return Ok(Outcome::default()) };
        let s = log.state();
        if row.membership != MEMBERSHIP_JOINED
            || !s.role_of(&ctx.my_pubkey).is_some_and(Role::is_manager)
            || s.kind != GroupKind::Private
            || s.is_banned(sender)
            || s.is_member(sender)
        {
            return Ok(Outcome::default());
        }
        if let Some(known) = repo::request(&self.store, &body.group_id, sender.as_hex(), "in").await? {
            if known.status == "pending" || known.created_at >= body.created_at {
                return Ok(Outcome::default());
            }
        }
        repo::put_request(
            &self.store,
            &repo::RequestRow {
                group_id: body.group_id.clone(),
                requester: sender.as_hex().to_string(),
                direction: "in".into(),
                status: "pending".into(),
                payload_json: serde_json::json!({ "note": body.note.chars().take(300).collect::<String>() }).to_string(),
                created_at: body.created_at,
                updated_at: 0,
            },
        )
        .await?;
        let mut out = Outcome { events: vec![Self::updated(&body.group_id)], ..Default::default() };
        out.events.push(UiEvent {
            name: UI_EVENT_GROUP_REQUEST.into(),
            payload: serde_json::json!({ "group_id": body.group_id, "requester": sender.as_hex() }),
        });
        if !historical {
            out.notify.push(Notice {
                title: row.name.clone(),
                body: Some(Body::JoinRequest { group_name: row.name }),
                chat_id: Some(repo::group_chat_id(&body.group_id)),
                sender: Some(sender.as_hex().to_string()),
                request: false,
            });
        }
        Ok(out)
    }

    async fn on_rejected(&self, body: wire::Rejected, sender: &PubKey, from_me: bool) -> Result<Outcome> {
        if from_me {
            return Ok(Outcome::default());
        }
        let pending: Vec<_> =
            repo::outgoing_requests(&self.store, "pending").await?.into_iter().filter(|r| r.group_id == body.group_id).collect();
        let Some(req) = pending.into_iter().next() else { return Ok(Outcome::default()) };
        // Only someone who was asked may refuse.
        let asked: Vec<String> = serde_json::from_str::<serde_json::Value>(&req.payload_json)
            .ok()
            .and_then(|v| v.get("asked").cloned())
            .and_then(|a| serde_json::from_value(a).ok())
            .unwrap_or_default();
        if !asked.is_empty() && !asked.iter().any(|a| a == sender.as_hex()) {
            return Ok(Outcome::default());
        }
        repo::put_request(&self.store, &repo::RequestRow { status: "rejected".into(), ..req }).await?;
        if let Some(mut row) = repo::get(&self.store, &body.group_id).await? {
            if row.membership == MEMBERSHIP_REQUESTED {
                row.membership = MEMBERSHIP_REJECTED.into();
                repo::upsert(&self.store, &row).await?;
            }
        }
        Ok(Outcome { events: vec![Self::updated(&body.group_id)], ..Default::default() })
    }

    /// Did I ask for this group, or agree to be in it?
    async fn consent(&self, group_id: &str, me: &PubKey) -> Result<bool> {
        if repo::request(&self.store, group_id, me.as_hex(), "out").await?.is_some_and(|r| r.status == "pending") {
            return Ok(true);
        }
        Ok(repo::invites(&self.store, "in", &["accepted"]).await?.iter().any(|i| i.group_id == group_id))
    }

    async fn consume_consent(&self, group_id: &str, me: &PubKey) -> Result<()> {
        if let Some(r) = repo::request(&self.store, group_id, me.as_hex(), "out").await? {
            if r.status == "pending" {
                repo::put_request(&self.store, &repo::RequestRow { status: "approved".into(), ..r }).await?;
            }
        }
        for i in repo::invites(&self.store, "in", &["accepted"]).await? {
            if i.group_id == group_id {
                repo::set_invite_status(&self.store, &i.invite_id, "joined").await?;
            }
        }
        Ok(())
    }

    async fn on_welcome(
        &self,
        keys: &Keys,
        body: wire::Welcome,
        sender: &PubKey,
        wire_id: &str,
        created_at: i64,
        ctx: &Context,
    ) -> Result<Outcome> {
        let me = &ctx.my_pubkey;
        let group_id = body.group_id.clone();
        let row = repo::get(&self.store, &group_id).await?;
        let from_me = sender == me;
        if let Some(r) = &row {
            match r.membership.as_str() {
                // Nothing new in a welcome for a group I am in, except keys.
                MEMBERSHIP_JOINED => {
                    return self.on_keys(keys, wire::KeyDelivery { group_id, keys: body.keys }, sender, ctx).await;
                }
                MEMBERSHIP_DISBANDED => return Ok(Outcome::default()),
                // A ban is lifted in the log, not here: a manager who admits
                // me again shows it, and the whole log decides in `settle`.
                MEMBERSHIP_LEFT | MEMBERSHIP_REMOVED | MEMBERSHIP_BANNED if from_me => return Ok(Outcome::default()),
                _ => {}
            }
        }
        if !from_me && !self.consent(&group_id, me).await? {
            repo::add_pending(
                &self.store,
                &PendingRow {
                    event_id: wire_id.to_string(),
                    group_id: group_id.clone(),
                    key_id: BUCKET_WELCOME.into(),
                    ciphertext: serde_json::json!({ "from": sender.as_hex(), "body": body }).to_string(),
                    created_at,
                    received_at: 0,
                },
            )
            .await?;
            return Ok(Outcome::default());
        }

        // The log as the welcome tells it, every operation verified.
        let mut verified: Vec<(Op, serde_json::Value)> = Vec::new();
        for signed in &body.ops {
            if let Ok(op) = wire::verify_op(signed, &group_id) {
                if matches!(op.body, OpBody::Create { .. }) && verified.iter().any(|(o, _)| matches!(o.body, OpBody::Create { .. })) {
                    continue;
                }
                verified.push((op, signed.clone()));
            }
        }
        let told = OpLog::from_ops(verified.iter().map(|(o, _)| o.clone())).map_err(rejection)?;
        let s = told.state();
        if let Some(r) = &row {
            if !r.owner.is_empty() && told.ordered().next().is_some_and(|c| c.author.as_hex() != r.owner) {
                return Err(MessengerError::Invalid("group_wrong_owner".into()));
            }
        }
        if !s.is_member(me) || s.disbanded || !(from_me || s.role_of(sender).is_some_and(Role::is_manager)) {
            return Err(MessengerError::Invalid("group_bad_welcome".into()));
        }
        let owner = told.ordered().next().map(|c| c.author.as_hex().to_string()).unwrap_or_default();
        repo::upsert(
            &self.store,
            &GroupRow {
                id: group_id.clone(),
                kind: if s.kind == GroupKind::Public { "public".into() } else { "private".into() },
                name: s.name.clone(),
                about: s.about.clone(),
                picture: s.picture.clone(),
                relay_url: row.as_ref().map(|r| r.relay_url.clone()).filter(|r| !r.is_empty()).unwrap_or(body.relay.clone()),
                owner,
                membership: MEMBERSHIP_JOINED.into(),
                my_role: s.role_of(me).map(|r| r.as_str().to_string()),
                members: s.members.len() as i64,
                link_epoch: s.link_epoch as i64,
                created_at: 0,
                updated_at: 0,
            },
        )
        .await?;
        self.forget_log(&group_id).await;
        for key in wire::decode_keys(&body.keys) {
            self.keep_key(&group_id, &key).await?;
        }
        for (op, signed) in &verified {
            if self.join_proven(&group_id, op).await? == Some(false) {
                continue;
            }
            self.keep_op(op, signed).await?;
        }
        for key in wire::decode_keys(&body.keys) {
            self.keep_key(&group_id, &key).await?;
        }
        if let (Some(link), GroupKind::Public) = (&body.link, s.kind) {
            if let Some(Ok(secret)) = wire::decode_secret(link).map(|b| LinkSecret::from_bytes(&b)) {
                if Some(secret.group_key(&group_id, s.link_epoch).id()) == s.current_key {
                    self.keep_link_secret(&group_id, s.link_epoch, &secret).await?;
                }
            }
        }
        self.consume_consent(&group_id, me).await?;

        // The row was rewritten above, so `settle` sees no change in it:
        // the screen has to hear of it here (a group I was banned in or
        // removed from is shown as such until it does).
        let mut out = Outcome { resubscribe: true, events: vec![Self::updated(&group_id)], ..Default::default() };
        self.drain(keys, &group_id, ctx, &mut out).await?;
        self.settle(keys, &group_id, true, ctx, &mut out).await?;
        if !from_me {
            out.notify.push(Notice {
                title: s.name.clone(),
                body: Some(Body::Welcome { group_name: s.name.clone() }),
                chat_id: Some(repo::group_chat_id(&group_id)),
                sender: Some(sender.as_hex().to_string()),
                request: false,
            });
        }
        Ok(out)
    }

    /// Welcomes that came before my own request or answer did.
    async fn held_welcomes(&self, keys: &Keys, group_id: &str, ctx: &Context) -> Result<Outcome> {
        let mut out = Outcome::default();
        for row in repo::take_pending(&self.store, group_id, BUCKET_WELCOME).await? {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&row.ciphertext) else { continue };
            let from = v.get("from").and_then(|f| f.as_str()).and_then(PubKey::parse);
            let body = v.get("body").cloned().and_then(|b| serde_json::from_value::<wire::Welcome>(b).ok());
            let (Some(from), Some(body)) = (from, body) else { continue };
            match self.on_welcome(keys, body, &from, &row.event_id, row.created_at, ctx).await {
                Ok(o) => out.merge(o),
                Err(e) => out.notes.push(format!("group: a held welcome was dropped: {e}")),
            }
        }
        Ok(out)
    }

    async fn on_keys(&self, keys: &Keys, body: wire::KeyDelivery, sender: &PubKey, ctx: &Context) -> Result<Outcome> {
        let me = &ctx.my_pubkey;
        let Some(row) = repo::get(&self.store, &body.group_id).await? else { return Ok(Outcome::default()) };
        if row.membership != MEMBERSHIP_JOINED {
            return Ok(Outcome::default());
        }
        let Some(log) = self.log(&body.group_id).await? else { return Ok(Outcome::default()) };
        if !(sender == me || log.state().role_of(sender).is_some_and(Role::is_manager)) {
            return Ok(Outcome::default());
        }
        let mut new = false;
        for key in wire::decode_keys(&body.keys).into_iter().take(256) {
            new |= self.keep_key(&body.group_id, &key).await?;
        }
        let mut out = Outcome::default();
        if new {
            self.drain(keys, &body.group_id, ctx, &mut out).await?;
            self.settle(keys, &body.group_id, true, ctx, &mut out).await?;
        }
        Ok(out)
    }

    // ─── Events of a group ──────────────────────────────────────────────────

    pub async fn on_event(&self, keys: &Keys, msg: GroupInbound, ctx: &Context) -> Result<Outcome> {
        let mut out = Outcome::default();
        let Some(row) = repo::get(&self.store, &msg.group_id).await? else { return Ok(out) };
        if !matches!(row.membership.as_str(), MEMBERSHIP_JOINED | MEMBERSHIP_JOINING) {
            return Ok(out);
        }
        let Some(key_id) = msg.key_id.clone().filter(|k| k.len() == 32 && k.bytes().all(|b| b.is_ascii_hexdigit())) else {
            return Ok(out);
        };
        let historical = msg.created_at < ctx.session_started_at || matches!(msg.envelope.source, EventSource::Sync { .. });
        let item = Item {
            event_id: msg.envelope.wire_id.as_hex().to_string(),
            ciphertext: msg.ciphertext,
            created_at: msg.created_at.secs(),
            historical,
        };
        let Some(key) = self.key(&msg.group_id, &KeyId(key_id.clone())).await? else {
            self.hold(&msg.group_id, &key_id, &item).await?;
            out.events.push(Self::updated(&msg.group_id));
            return Ok(out);
        };
        if self.process(keys, &msg.group_id, &key, &item, ctx, &mut out).await? {
            self.drain(keys, &msg.group_id, ctx, &mut out).await?;
        }
        self.settle(keys, &msg.group_id, historical, ctx, &mut out).await?;
        Ok(out)
    }

    async fn hold(&self, group_id: &str, bucket: &str, item: &Item) -> Result<()> {
        repo::add_pending(
            &self.store,
            &PendingRow {
                event_id: item.event_id.clone(),
                group_id: group_id.to_string(),
                key_id: bucket.to_string(),
                ciphertext: item.ciphertext.clone(),
                created_at: item.created_at,
                received_at: 0,
            },
        )
        .await
    }

    /// Open one event and act on it. `true` when the group became better
    /// known (a key, the log, a member): what waits may be tried again.
    async fn process(&self, keys: &Keys, group_id: &str, key: &GroupKey, item: &Item, ctx: &Context, out: &mut Outcome) -> Result<bool> {
        let opened = match wire::open(group_id, key, &item.ciphertext) {
            Ok(o) => o,
            Err(e) => {
                out.notes.push(format!("group: event dropped: {e}"));
                return Ok(false);
            }
        };
        match opened {
            Opened::Message(m) => {
                if let Verdict::Hold = self.on_message(group_id, m, item, ctx, out).await? {
                    self.hold(group_id, &format!("{HELD}{}", key.id().0), item).await?;
                }
                Ok(false)
            }
            Opened::Op { op, signed, envelopes } => {
                // A join has to show the key of the link that is current.
                match self.join_proven(group_id, &op).await? {
                    Some(true) => self.on_op(keys, group_id, op, signed, envelopes, out).await,
                    Some(false) => Ok(false),
                    None => {
                        self.hold(group_id, &format!("{HELD}{}", key.id().0), item).await?;
                        Ok(false)
                    }
                }
            }
            Opened::Chain(older) => {
                let mut new = false;
                if repo::key_ids(&self.store, group_id).await?.len() < MAX_KEYS {
                    for k in older {
                        new |= self.keep_key(group_id, &k).await?;
                    }
                }
                Ok(new)
            }
        }
    }

    /// `Some(true)`: not a join, or a join with a good proof. `None`: the
    /// key of that link is not here yet.
    pub(crate) async fn join_proven(&self, group_id: &str, op: &Op) -> Result<Option<bool>> {
        if op.body != OpBody::Join {
            return Ok(Some(true));
        }
        let Some(proof) = &op.proof else { return Ok(Some(false)) };
        let key = match self.link_secret(group_id, proof.epoch).await? {
            Some(link) => Some(link.group_key(group_id, proof.epoch)),
            None => {
                // Not my link: the log says which key it gave.
                let Some(log) = self.log(group_id).await? else { return Ok(None) };
                let id = log.ordered().filter(|o| log.rejection(&o.id()).is_none()).find_map(|o| match &o.body {
                    OpBody::Create { .. } if proof.epoch == 0 => o.key.clone(),
                    OpBody::RotateLink { link_epoch } if *link_epoch == proof.epoch => o.key.clone(),
                    _ => None,
                });
                match id {
                    Some(id) => self.key(group_id, &id).await?,
                    None => None,
                }
            }
        };
        Ok(key.map(|k| k.join_mac(group_id, &op.author, proof.epoch) == proof.mac))
    }

    /// Try again everything that waited, until nothing more is learned.
    pub(crate) async fn drain(&self, keys: &Keys, group_id: &str, ctx: &Context, out: &mut Outcome) -> Result<()> {
        let oldest = self.now() - HELD_TTL_SECS;
        for _ in 0..64 {
            let mut progress = false;
            for id in repo::key_ids(&self.store, group_id).await? {
                let Some(key) = self.key(group_id, &KeyId(id.clone())).await? else { continue };
                for (bucket, held) in [(id.clone(), false), (format!("{HELD}{id}"), true)] {
                    for row in repo::take_pending(&self.store, group_id, &bucket).await? {
                        if held && row.received_at < oldest {
                            continue;
                        }
                        let item = Item { event_id: row.event_id, ciphertext: row.ciphertext, created_at: row.created_at, historical: true };
                        progress |= self.process(keys, group_id, &key, &item, ctx, out).await?;
                    }
                }
            }
            if !progress {
                break;
            }
        }
        out.events.push(Self::updated(group_id));
        Ok(())
    }

    async fn on_op(
        &self,
        keys: &Keys,
        group_id: &str,
        op: Op,
        signed: serde_json::Value,
        envelopes: Vec<SecretEnvelope>,
        out: &mut Outcome,
    ) -> Result<bool> {
        let mut progress = false;
        if let Some(announced) = &op.key {
            if self.key(group_id, announced).await?.is_none() {
                if let Ok(Some(bytes)) = wire::open_envelope(keys, &op.author, &envelopes) {
                    match &op.body {
                        OpBody::RotateLink { link_epoch } => {
                            if let Ok(link) = LinkSecret::from_bytes(&bytes) {
                                if &link.group_key(group_id, *link_epoch).id() == announced {
                                    self.keep_link_secret(group_id, *link_epoch, &link).await?;
                                    progress = true;
                                }
                            }
                        }
                        _ => {
                            if let Ok(key) = wire::check_key(announced, &bytes) {
                                progress |= self.keep_key(group_id, &key).await?;
                            }
                        }
                    }
                }
            }
        }
        let had_log = self.log(group_id).await?.is_some();
        // A group is whose first operation it is. Anyone holding the key
        // can write a `Create`: only the first one, by the owner we were
        // told about, is ours.
        if matches!(op.body, OpBody::Create { .. }) {
            let owner = repo::get(&self.store, group_id).await?.map(|g| g.owner).unwrap_or_default();
            if had_log || (!owner.is_empty() && owner != op.author.as_hex()) {
                return Ok(progress);
            }
        }
        if !self.keep_op(&op, &signed).await? {
            return Ok(progress);
        }
        let Some(log) = self.log(group_id).await? else { return Ok(progress) };
        progress |= !had_log;
        let id = op.id();
        if log.contains(&id) && log.rejection(&id).is_none() && adds_people(&op.body) {
            progress = true;
        }
        self.lines(group_id, &log, out).await?;
        Ok(progress)
    }

    /// Lines in the chat for the operations that took effect.
    async fn lines(&self, group_id: &str, log: &OpLog, out: &mut Outcome) -> Result<()> {
        let todo: Vec<Op> = {
            let mut lined = self.lined.lock().await;
            let seen = lined.entry(group_id.to_string()).or_default();
            log.ordered().filter(|o| log.rejection(&o.id()).is_none() && seen.insert(o.id().0)).cloned().collect()
        };
        for op in todo {
            if let Some(ev) = self.system_line(group_id, &op).await? {
                out.events.push(ev);
            }
        }
        Ok(())
    }

    /// After something changed: the cached row, my own standing, joining
    /// a public group whose log has just arrived, care for the key.
    pub(crate) async fn settle(&self, keys: &Keys, group_id: &str, historical: bool, ctx: &Context, out: &mut Outcome) -> Result<()> {
        let me = &ctx.my_pubkey;
        let Some(before) = repo::get(&self.store, group_id).await? else { return Ok(()) };
        let Some(log) = self.log(group_id).await? else { return Ok(()) };
        self.lines(group_id, &log, out).await?;
        let gone = before.membership == MEMBERSHIP_JOINED && !log.state().is_member(me) && !log.state().disbanded;
        let row = self.refresh_row(group_id, me, gone.then(|| departure(&log, me))).await?;
        let Some(row) = row else { return Ok(()) };
        if row.membership != before.membership || row.name != before.name || row.members != before.members || row.my_role != before.my_role {
            out.events.push(Self::updated(group_id));
        }
        if row.membership != before.membership {
            out.resubscribe = true;
        }
        if row.membership == MEMBERSHIP_JOINING {
            match self.try_join(keys, group_id).await {
                Ok(o) => out.merge(o),
                Err(e) => out.notes.push(format!("group: joining failed: {e}")),
            }
        }
        if !historical
            && row.membership == MEMBERSHIP_JOINED
            && log.state().role_of(me).is_some_and(Role::is_manager)
            && log.key_status() != &KeyStatus::Good
            && !out.maintain.iter().any(|g| g == group_id)
        {
            out.maintain.push(group_id.to_string());
        }
        Ok(())
    }

    async fn on_message(&self, group_id: &str, m: InnerMessage, item: &Item, ctx: &Context, out: &mut Outcome) -> Result<Verdict> {
        let me = &ctx.my_pubkey;
        let Some(log) = self.log(group_id).await? else { return Ok(Verdict::Hold) };
        let s = log.state();
        if s.is_banned(&m.author) {
            return Ok(Verdict::Done);
        }
        let standing = standing(&log, &m.author, m.created_at);
        // Not a member as far as the log here knows: the op that let them
        // in may not have come yet. Held like anything they say, a reaction
        // too, and tried again when the log grows (or given up after
        // `HELD_TTL_SECS`).
        if standing == Standing::Outsider {
            return Ok(Verdict::Hold);
        }
        let from_me = &m.author == me;
        let chat_id = repo::group_chat_id(group_id);
        let id = m.id.as_hex().to_string();
        let Ok(envelope) = Envelope::parse(&m.content) else { return Ok(Verdict::Done) };
        // A reaction: only a member who may speak puts one, mine from
        // another device or my own echo too (the same word again changes
        // nothing). One of a muted member is dropped.
        if envelope.t == T_REACTION {
            let fields = (envelope.str_field("target"), envelope.str_field("emoji"), envelope.fields.get("set").and_then(|v| v.as_bool()));
            if let (Standing::Member, (Some(target), Some(emoji), Some(set))) = (&standing, fields) {
                for fx in self.dm.apply_reaction(&chat_id, &m.author, target, emoji, set, m.created_at).await? {
                    if let messenger_core::Effect::Emit(e) = fx {
                        out.events.push(e);
                    }
                }
            }
            return Ok(Verdict::Done);
        }
        // A member read the group: a mark, not a message. One who may not
        // speak may still read. Mine from another device says nothing new.
        if envelope.t == T_RECEIPT_READ {
            if let (false, Some(at)) = (from_me, envelope.fields.get("at").and_then(|v| v.as_i64())) {
                for fx in self.dm.peer_read(&chat_id, m.author.as_hex(), at, m.created_at).await? {
                    if let messenger_core::Effect::Emit(e) = fx {
                        out.events.push(e);
                    }
                }
            }
            return Ok(Verdict::Done);
        }
        if standing == Standing::Muted {
            return Ok(Verdict::Done);
        }
        let target = envelope.str_field("target").map(String::from);
        let (content_type, text, hidden, media_json): (&str, Option<String>, bool, Option<String>) = match envelope.t.as_str() {
            T_TEXT => (msgs::CT_TEXT, envelope.as_text().map(String::from), false, None),
            T_EDIT => (msgs::CT_EDIT, None, true, None),
            T_DELETE => (msgs::CT_DELETE, None, true, None),
            T_MEDIA => (
                msgs::CT_MEDIA,
                envelope.str_field("caption").map(String::from),
                false,
                Some(serde_json::Value::Object(envelope.fields.clone()).to_string()),
            ),
            _ => return Ok(Verdict::Done),
        };
        if text.as_ref().is_some_and(|t| t.len() > messenger_dm::service::MAX_TEXT_BYTES) {
            return Ok(Verdict::Done);
        }
        let inserted = msgs::insert(
            &self.store,
            &NewMessage {
                id: id.clone(),
                chat_id: chat_id.clone(),
                wire_id: Some(item.event_id.clone()),
                direction: if from_me { msgs::DIR_OUT.into() } else { msgs::DIR_IN.into() },
                status: if from_me { msgs::STATUS_SENT.into() } else { msgs::STATUS_RECEIVED.into() },
                content_type: content_type.into(),
                text: text.clone(),
                envelope_json: m.content.clone(),
                sender_pubkey: m.author.as_hex().to_string(),
                reply_to_id: m.reply_to.as_ref().map(|e| e.as_hex().to_string()),
                target_id: if hidden { target.clone() } else { None },
                created_at: m.created_at,
                is_hidden: hidden,
                outbox_local_id: None,
                media_json,
            },
        )
        .await?;
        if !inserted {
            // My own message came back: a relay has it.
            if from_me {
                if let Some(row) = msgs::get(&self.store, &id).await? {
                    if row.status == msgs::STATUS_QUEUED || row.status == msgs::STATUS_FAILED {
                        msgs::set_status(&self.store, &id, msgs::STATUS_SENT, None).await?;
                        if !row.is_hidden {
                            out.events.push(message_updated(&chat_id, &id));
                        }
                    }
                }
            }
            return Ok(Verdict::Done);
        }

        if hidden {
            let Some(target) = target else { return Ok(Verdict::Done) };
            let Some(row) = msgs::get(&self.store, &target).await? else { return Ok(Verdict::Done) };
            if row.is_hidden || row.chat_id != chat_id || row.deleted_at.is_some() || row.content_type == msgs::CT_SYSTEM {
                return Ok(Verdict::Done);
            }
            let new_text = envelope.str_field("text").map(|t| t.trim().to_string());
            if self.change(&log, &row, content_type, &m.author, m.created_at, new_text).await? {
                chats::recompute_last(&self.store, &chat_id).await?;
                out.events.push(message_updated(&chat_id, &target));
            }
            return Ok(Verdict::Done);
        }

        // Edits or a delete that overtook this message on the way.
        if let Some(row) = msgs::get(&self.store, &id).await? {
            for p in msgs::pending_for_target(&self.store, &id).await? {
                let Some(actor) = PubKey::parse(&p.sender_pubkey) else { continue };
                let new_text = Envelope::parse(&p.envelope_json).ok().and_then(|e| e.str_field("text").map(|t| t.trim().to_string()));
                self.change(&log, &row, &p.content_type, &actor, p.created_at, new_text).await?;
            }
        }
        // Removed for me on another device before it came here.
        let hidden_before = self.dm.hide_if_hidden(&id).await?;

        let line = if content_type == msgs::CT_MEDIA {
            let name = envelope.str_field("name").unwrap_or("file");
            format!("📎 {}", text.as_deref().map(messenger_dm::view::preview).unwrap_or_else(|| name.to_string()))
        } else {
            text.as_deref().map(messenger_dm::view::preview).unwrap_or_default()
        };
        let live = !from_me && !item.historical;
        let floor = self.dm.unread_floor();
        let joined_at = s.member(me).map(|x| x.joined_at).unwrap_or(i64::MAX);
        let unread =
            !from_me && !hidden_before && m.created_at >= joined_at && (live || (floor >= 0 && m.created_at >= floor));
        chats::touch(&self.store, &chat_id, m.created_at, Some(&line), unread).await?;
        chats::recompute_last(&self.store, &chat_id).await?;
        // Written on another device of mine: what came before is read there.
        if from_me && chats::read_up_to(&self.store, &chat_id, m.created_at).await? {
            out.events.push(UiEvent {
                name: messenger_dm::UI_EVENT_CHAT_READ.into(),
                payload: serde_json::json!({ "chat_id": chat_id }),
            });
        }
        let Some(view) = self.dm.message(&id).await? else { return Ok(Verdict::Done) };
        let deleted = view.deleted;
        out.events.push(UiEvent {
            name: messenger_dm::UI_EVENT_DM_MESSAGE.into(),
            payload: serde_json::json!({ "chat_id": chat_id, "message": view, "historical": item.historical }),
        });
        if live && !deleted && !chats::is_muted(&self.store, &chat_id).await? {
            out.notify.push(Notice {
                title: s.name.clone(),
                body: messenger_dm::body::body_of(&envelope),
                chat_id: Some(chat_id),
                sender: Some(m.author.as_hex().to_string()),
                request: false,
            });
        }
        Ok(Verdict::Done)
    }

    /// Apply an edit or a delete to a stored message, if its author may.
    async fn change(
        &self,
        log: &OpLog,
        row: &msgs::MessageRow,
        content_type: &str,
        actor: &PubKey,
        at: i64,
        new_text: Option<String>,
    ) -> Result<bool> {
        if at < row.created_at {
            return Ok(false);
        }
        match content_type {
            msgs::CT_EDIT => {
                if row.sender_pubkey != actor.as_hex() || row.content_type != msgs::CT_TEXT || row.edited_at.is_some_and(|e| e > at) {
                    return Ok(false);
                }
                let Some(t) = new_text.filter(|t| !t.is_empty() && t.len() <= messenger_dm::service::MAX_TEXT_BYTES) else {
                    return Ok(false);
                };
                msgs::set_text(&self.store, &row.id, &t, at).await?;
                Ok(true)
            }
            msgs::CT_DELETE => {
                let Some(author) = PubKey::parse(&row.sender_pubkey) else { return Ok(false) };
                // Roles as they are now; the author may always remove their own.
                if actor != &author && !log.state().can_delete_message(actor, &author) {
                    return Ok(false);
                }
                msgs::mark_deleted(&self.store, &row.id, at).await?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

fn message_updated(chat_id: &str, message_id: &str) -> UiEvent {
    UiEvent {
        name: messenger_dm::UI_EVENT_DM_UPDATED.into(),
        payload: serde_json::json!({ "chat_id": chat_id, "message_id": message_id }),
    }
}

/// The group a direct message of the group protocol is about; `None`
/// for every other direct message.
pub fn group_of_dm(content: &str) -> Option<String> {
    let e = Envelope::parse(content).ok()?;
    if !e.t.starts_with("group.") {
        return None;
    }
    Some(e.str_field("group_id").unwrap_or_default().to_string())
}
