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
use crate::op::{GroupKind, KeyId, Op, OpBody, PostMode, Premod, Schedule};
use crate::roles::Role;
use crate::service::*;
use crate::state::{self, Closed, GroupState};
use crate::wire::{self, InnerMessage, Opened, SecretEnvelope};
use messenger_core::envelope::{T_CONTACT, T_DELETE, T_EDIT, T_MEDIA, T_REACTION, T_RECEIPT_READ, T_TEXT, T_TYPING};
use messenger_core::traits::{Body, Notice, UiEvent};
use messenger_core::{Context, DmInbound, Envelope, EventSource, GroupInbound, MessengerError, PubKey, Result, TextFormat};
use messenger_store::groups::{self as repo, GroupRow, PendingRow};
use messenger_store::messages::{self as msgs, NewMessage};
use messenger_store::{chats, servers};
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
pub(crate) const MAX_KEYS: usize = 512;
/// Unvouched invitations received and not answered yet (`InviteStanding`):
/// in all, of one group, of one inviter. One more pushes out the first.
const MAX_OPEN_INVITES: usize = 50;
const MAX_OPEN_INVITES_PER_GROUP: usize = 4;
const MAX_OPEN_INVITES_PER_PEER: usize = 10;
/// Requests to join one group that wait for a manager.
const MAX_OPEN_REQUESTS: usize = 20;

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

/// Where the FUTURE bucket's rows of a key are: operations of a version
/// newer than this build reads (`wire::OP_TOO_NEW`), kept for after an
/// update; nothing in it goes stale.
const FUTURE: &str = "future:";
/// Welcomes whose log has such operations, and me admitted after them:
/// kept whole, read again after an update (`replay_parked_welcomes`).
/// Not under the FUTURE prefix: the two are bounded apart, so that a
/// member who floods the group with what this build cannot read does
/// not push out the welcome a manager sent me.
const FUTURE_WELCOME: &str = "welcome:future";
/// What one group may park under each of those: rows, and bytes of
/// ciphertext. Past that, what came first makes room.
pub(crate) const MAX_FUTURE_ROWS: i64 = 1000;
pub(crate) const MAX_FUTURE_BYTES: i64 = 16 * 1024 * 1024;

/// `MAX_FUTURE_ROWS`, which a test may lower to see the bound at work.
fn max_future_rows() -> i64 {
    #[cfg(test)]
    if let Some(n) = FUTURE_ROWS_CAP.with(|c| c.get()) {
        return n;
    }
    MAX_FUTURE_ROWS
}

#[cfg(test)]
thread_local! {
    pub(crate) static FUTURE_ROWS_CAP: std::cell::Cell<Option<i64>> = const { std::cell::Cell::new(None) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Standing {
    Member,
    Muted,
    Outsider,
}

/// What the log says of one person at one moment (`rules_at`): whether
/// they were a member and could speak, what they were, and the rules of
/// posting in force then.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Rules {
    pub(crate) standing: Standing,
    /// Of a mute in force: when it lifts by itself.
    muted_until: Option<i64>,
    role: Role,
    trusted: bool,
    post_mode: PostMode,
    schedule: Option<Schedule>,
    premod: Premod,
    premod_since: Option<u64>,
    joined_pos: Option<u64>,
}

impl Rules {
    /// Why this person may not speak at `at`, if they may not: not a
    /// member, muted, or held by the rules of the group (`state::gate`).
    pub(crate) fn closed(&self, at: i64) -> Option<Closed> {
        match self.standing {
            Standing::Outsider => Some(Closed::NotMember),
            Standing::Muted => Some(Closed::Muted { until: self.muted_until }),
            Standing::Member => state::gate(self.role, self.post_mode, self.schedule.as_ref(), at).err(),
        }
    }

    /// Do this person's messages wait for a moderator: the rule of
    /// `GroupState::subject_to_premod`, as it stood then. Messages of one
    /// held so wait for a moderator (`moderation`).
    pub(crate) fn subject_to_premod(&self) -> bool {
        if self.standing != Standing::Member || self.role >= Role::Moderator || self.trusted {
            return false;
        }
        match self.premod {
            Premod::Off => false,
            Premod::All => true,
            Premod::Newcomers => matches!((self.premod_since, self.joined_pos), (Some(since), Some(pos)) if pos > since),
        }
    }
}

/// One stay in the group: when it began, and the mute in force during
/// it (when it began, when it lifts by itself).
#[derive(Default)]
struct Stay {
    since: Option<i64>,
    mute: Option<(i64, Option<i64>)>,
}

impl Stay {
    /// The stay ends at `end`: what `at` was inside it, if it was. Clocks
    /// differ: a message may be dated a little before the join.
    fn end(&mut self, at: i64, end: i64) -> Option<(Standing, Option<i64>)> {
        let start = self.since.take()?;
        let mute = self.mute.take();
        if at < start - JOIN_SLACK_SECS || at >= end {
            return None;
        }
        Some(match mute {
            Some((m, until)) if at >= m && until.is_none_or(|u| at < u) => (Standing::Muted, until),
            _ => (Standing::Member, None),
        })
    }
}

/// What held for `who` at the moment `at`. A removal, a ban or a leave
/// ends the stay at its own second: what is dated then is outside. A
/// mute with an end lifts at that end, with no operation to say so. The
/// roles and the rules of posting are those set by the operations dated
/// up to `at`; the places in the log (`joined_pos`, `premod_since`) are
/// those of the applied order, the same on every device.
pub(crate) fn rules_at(log: &OpLog, who: &PubKey, at: i64) -> Rules {
    let mut stay = Stay::default();
    let mut rules = Rules {
        standing: Standing::Outsider,
        muted_until: None,
        role: Role::Member,
        trusted: false,
        post_mode: PostMode::All,
        schedule: None,
        premod: Premod::Off,
        premod_since: None,
        joined_pos: None,
    };
    let mut inside: Option<(Standing, Option<i64>)> = None;
    let applied = log.ordered().filter(|o| log.rejection(&o.id()).is_none());
    for (pos, op) in applied.enumerate() {
        let pos = pos as u64;
        let counts = op.created_at <= at;
        match &op.body {
            OpBody::Create { .. } | OpBody::Join if &op.author == who => {
                stay.since = stay.since.or(Some(op.created_at));
                if counts {
                    rules.role = if op.body == OpBody::Join { Role::Member } else { Role::Owner };
                    rules.trusted = false;
                    rules.joined_pos = Some(pos);
                }
            }
            OpBody::Admit { who: w } if w == who => {
                stay.since = stay.since.or(Some(op.created_at));
                if counts {
                    rules.role = Role::Member;
                    rules.trusted = false;
                    rules.joined_pos = Some(pos);
                }
            }
            OpBody::Leave if &op.author == who => inside = inside.or(stay.end(at, op.created_at)),
            OpBody::Remove { who: w } | OpBody::Ban { who: w } if w == who => inside = inside.or(stay.end(at, op.created_at)),
            OpBody::SetMuted { who: w, muted, until } if w == who => {
                if *muted {
                    stay.mute = match stay.mute {
                        // A mute that had lifted by itself: a new one, and
                        // what was said under the old one stays hidden.
                        Some((m, Some(u))) if u <= op.created_at => {
                            if at >= m && at < u {
                                inside = Some((Standing::Muted, Some(u)));
                            }
                            Some((op.created_at, *until))
                        }
                        // One in force keeps its start and takes the new end.
                        Some((m, _)) => Some((m, *until)),
                        None => Some((op.created_at, *until)),
                    };
                } else if let Some((m, until)) = stay.mute.take() {
                    // A muted stretch that ended still hides what was said in it.
                    let end = until.map_or(op.created_at, |u| u.min(op.created_at));
                    if at >= m && at < end {
                        inside = Some((Standing::Muted, until));
                    }
                }
            }
            OpBody::SetRole { who: w, role } if w == who && counts => rules.role = *role,
            OpBody::TransferOwnership { to } if counts => {
                if to == who {
                    rules.role = Role::Owner;
                } else if &op.author == who {
                    rules.role = Role::Admin;
                }
            }
            OpBody::SetTrusted { who: w, trusted } if w == who && counts => rules.trusted = *trusted,
            OpBody::EditSettings { post_mode, post_schedule, premod, .. } if counts => {
                if let Some(p) = post_mode {
                    rules.post_mode = *p;
                }
                if let Some(s) = post_schedule {
                    rules.schedule = if s.is_empty() { None } else { Some(s.clone()) };
                }
                if let Some(p) = premod {
                    match (rules.premod, *p) {
                        (_, Premod::Off) => rules.premod_since = None,
                        (Premod::Off, _) => rules.premod_since = Some(pos),
                        _ => {}
                    }
                    rules.premod = *p;
                }
            }
            _ => {}
        }
    }
    // What was found first stands: a mute that ended, with `at` under it,
    // is not undone by the stay going on after it.
    inside = inside.or(stay.end(at, i64::MAX));
    if let Some((standing, until)) = inside {
        rules.standing = standing;
        rules.muted_until = until;
    }
    rules
}

/// Newest `created_at` of the operations the log took: `before_time` of
/// the service, without the rejected ones, as `standing` goes without
/// them. A live message is judged no earlier than this: by the log as it
/// stands now.
fn head_time(log: &OpLog) -> i64 {
    log.ordered().filter(|o| log.rejection(&o.id()).is_none()).map(|o| o.created_at).max().unwrap_or(0)
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

/// The stored form of an invitation: the body as it came (cleaned), and
/// whether its inviter was vouched for (`vouched`, unknown to the wire:
/// the body reads it back without it).
fn invite_payload(body: &wire::Invite, vouched: bool) -> Result<String> {
    let mut v = serde_json::to_value(body)?;
    v["vouched"] = serde_json::Value::Bool(vouched);
    Ok(v.to_string())
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
        // A key I block, or one the network bans, is not heard in the
        // protocol of groups either: no invitation, no request to join, no
        // welcome, no keys. The block is the one of the home chat with them
        // (`msg_dm_relations.server_id = ''`), whatever server the group
        // lives on: a block is of a person in groups, and a private chat
        // with them on a server keeps its own relation for that chat alone.
        if !from_me && (self.dm.is_blocked(&msg.sender).await? || self.dm.is_banned(&msg.sender).await?) {
            return Ok(Some(Outcome::default()));
        }
        // A group the network closes hears nothing of its protocol: no
        // invitation is kept, nobody asks in or is welcomed, no answer is
        // taken. Only the keys its managers hand out are put by, without a
        // word, so that the group reads again when the ban lifts: the
        // wrap that brought them counts as seen and never comes again.
        if let Some(group_id) = envelope.str_field("group_id") {
            if self.is_group_banned(group_id).await? {
                if matches!(envelope.t.as_str(), wire::T_KEYS | wire::T_WELCOME) {
                    self.keys_of_a_closed_group(&envelope, &msg.sender, me).await?;
                }
                return Ok(Some(Outcome::default()));
            }
        }
        // A word of a group comes from where the group lives (the wire
        // document, «Группы на сервере»): the server of the wrap,
        // told as for a message of a chat (`srv`, else the relay it came by,
        // else home), is the group's. A server this device does not have
        // brings no word of any group, and one of another server than the
        // group's is dropped.
        let Some(came_by) = self.dm.server_of_inbound(msg).await? else { return Ok(Some(Outcome::default())) };
        if let Some(group_id) = envelope.str_field("group_id") {
            if !self.heard_where_it_lives(&envelope, group_id, came_by.as_deref()).await? {
                return Ok(Some(Outcome {
                    notes: vec![format!("group: {} by a server not the group's, dropped", envelope.t)],
                    ..Default::default()
                }));
            }
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

    /// Whether a word of the group protocol about `group_id` that came by
    /// `came_by` (`None`: the home set) came from the group's server: that
    /// of the relay of its row; for a group with no row here, that of the
    /// relay an invitation or a welcome names (the one the group will be
    /// kept on). Any other word of a group not known here is taken as
    /// before: it is an answer, or a copy of my own, about a group it
    /// cannot bring in.
    async fn heard_where_it_lives(&self, envelope: &Envelope, group_id: &str, came_by: Option<&str>) -> Result<bool> {
        let lives_on = match repo::get(&self.store, group_id).await? {
            Some(row) => self.server_of_relay(&row.relay_url).await?,
            None if matches!(envelope.t.as_str(), wire::T_INVITE | wire::T_WELCOME) => {
                self.server_of_relay(envelope.str_field("relay").unwrap_or("")).await?
            }
            None => return Ok(true),
        };
        Ok(lives_on.as_deref() == came_by)
    }

    /// The keys a key delivery or a welcome carries for a group the network
    /// closes, kept as `on_keys` keeps them (a group I am in, from me or a
    /// manager by the log here) and nothing else: no drain of what waits,
    /// no settling, nothing told. A malformed body is passed by.
    async fn keys_of_a_closed_group(&self, envelope: &Envelope, sender: &PubKey, me: &PubKey) -> Result<()> {
        let (group_id, encoded) = match envelope.t.as_str() {
            wire::T_KEYS => match wire::dm_body::<wire::KeyDelivery>(envelope) {
                Ok(b) => (b.group_id, b.keys),
                Err(_) => return Ok(()),
            },
            _ => match wire::dm_body::<wire::Welcome>(envelope) {
                Ok(b) => (b.group_id, b.keys),
                Err(_) => return Ok(()),
            },
        };
        let Some(row) = repo::get(&self.store, &group_id).await? else { return Ok(()) };
        if row.membership != MEMBERSHIP_JOINED {
            return Ok(());
        }
        let Some(log) = self.log(&group_id).await? else { return Ok(()) };
        if !(sender == me || log.state().role_of(sender).is_some_and(Role::is_manager)) {
            return Ok(());
        }
        for key in wire::decode_keys(&encoded).into_iter().take(256) {
            self.keep_key(&group_id, &key).await?;
        }
        Ok(())
    }

    async fn on_invite(&self, mut body: wire::Invite, peer: Option<PubKey>, from_me: bool, historical: bool) -> Result<Outcome> {
        let Some(peer) = peer else { return Ok(Outcome::default()) };
        let now = self.now();
        if let Err(what) = body.check(now) {
            return Ok(Outcome { notes: vec![format!("group: an invitation dropped: bad {what}")], ..Default::default() });
        }
        body.name = body.shown_name();
        body.about = messenger_richtext::normalize(&body.about);
        // Is the inviter one who may invite (`invite_standing`, the same
        // rule as the push's)? A manager by the log I hold, or my contact,
        // is told of; anyone else's word is kept to be answered, unheard.
        let mut vouched = false;
        if !from_me {
            let row = repo::get(&self.store, &body.group_id).await?;
            let log = self.log(&body.group_id).await?;
            let contact = self.dm.load_relation(&peer).await?.my_contact == messenger_dm::relationship::MyContact::Approved;
            match invite_standing(row.as_ref(), log.as_ref(), &peer, contact) {
                InviteStanding::Drop => return Ok(Outcome::default()),
                InviteStanding::Vouched => vouched = true,
                InviteStanding::Unvouched if repo::invite(&self.store, &body.invite_id).await?.is_none() => {
                    self.make_room_for_unvouched(&body, &peer, now).await?
                }
                InviteStanding::Unvouched => {}
            }
        }
        let new = repo::insert_invite(
            &self.store,
            &repo::InviteRow {
                invite_id: body.invite_id.clone(),
                group_id: body.group_id.clone(),
                direction: if from_me { "out".into() } else { "in".into() },
                peer: peer.as_hex().to_string(),
                status: if from_me { "sent".into() } else { "received".into() },
                payload_json: invite_payload(&body, vouched)?,
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
                if row.payload_json == "{}" && row.peer == peer.as_hex() && row.group_id == body.group_id {
                    repo::set_invite_payload(&self.store, &body.invite_id, &invite_payload(&body, vouched)?).await?;
                }
            }
            return Ok(out);
        }
        out.events.push(UiEvent {
            name: UI_EVENT_GROUP_INVITE.into(),
            payload: serde_json::json!({ "invite_id": body.invite_id, "group_id": body.group_id }),
        });
        // Nothing is told of a person whose chat is muted.
        if !from_me && !historical && vouched && !chats::is_muted(&self.store, &chats::dm_chat_id(peer.as_hex(), None)).await? {
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

    /// Unvouched invitations that wait are few: in all, of one group, of
    /// one inviter. A new one takes the place of the one that came first
    /// (by my clock) among those it would go over: whoever fills the
    /// places with strangers' keys loses their own first, and nothing of a
    /// contact or a manager is ever counted or pushed out.
    async fn make_room_for_unvouched(&self, body: &wire::Invite, peer: &PubKey, now: i64) -> Result<()> {
        // What lapsed goes for good: a copy of it coming again is past its
        // date and refused by `check`.
        repo::delete_lapsed_invites(&self.store, now).await?;
        let mut open: Vec<_> = repo::invites(&self.store, "in", &["received"])
            .await?
            .into_iter()
            .filter(|i| i.expires_at > now && i.invite_id != body.invite_id && !vouched_of(i))
            .collect();
        open.sort_by(|a, b| (a.updated_at, a.created_at, &a.invite_id).cmp(&(b.updated_at, b.created_at, &b.invite_id)));
        // The same inviter's earlier word on the same group: this one stands
        // in its place.
        let again: Vec<String> =
            open.iter().filter(|i| i.peer == peer.as_hex() && i.group_id == body.group_id).map(|i| i.invite_id.clone()).collect();
        for id in &again {
            repo::delete_invite(&self.store, id).await?;
        }
        open.retain(|i| !again.contains(&i.invite_id));
        loop {
            let of_peer = |i: &&repo::InviteRow| i.peer == peer.as_hex();
            let of_group = |i: &&repo::InviteRow| i.group_id == body.group_id;
            let victim = if open.iter().filter(of_peer).count() >= MAX_OPEN_INVITES_PER_PEER {
                open.iter().find(of_peer)
            } else if open.iter().filter(of_group).count() >= MAX_OPEN_INVITES_PER_GROUP {
                open.iter().find(of_group)
            } else if open.len() >= MAX_OPEN_INVITES {
                open.first()
            } else {
                return Ok(());
            };
            let Some(id) = victim.map(|v| v.invite_id.clone()) else { return Ok(()) };
            // Taken away whole, not marked: rows of strangers do not pile up,
            // and an answer from my other device to it finds no row and
            // leaves the stub it leaves for one it never heard of.
            repo::delete_invite(&self.store, &id).await?;
            open.retain(|i| i.invite_id != id);
        }
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
                // One that lapsed here may have been answered there in time.
                Some(row) if row.direction == "in" && (row.status == "received" || row.status == "expired") => {
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
            let note: String = body.note.chars().take(300).collect();
            if let Some(pending) = known.as_ref().filter(|r| r.status == "pending" && r.created_at < body.created_at) {
                // Asked again on the other device with new words: the
                // people asked stay, the note and the date are the new ones.
                let mut payload = serde_json::from_str::<serde_json::Value>(&pending.payload_json).unwrap_or_else(|_| serde_json::json!({}));
                payload["note"] = note.clone().into();
                let row = repo::RequestRow { payload_json: payload.to_string(), created_at: body.created_at, ..pending.clone() };
                repo::put_request(&self.store, &row).await?;
            } else if known.is_none_or(|r| r.created_at < body.created_at && r.status != "pending") {
                repo::put_request(
                    &self.store,
                    &repo::RequestRow {
                        group_id: body.group_id.clone(),
                        requester: sender.as_hex().to_string(),
                        direction: "out".into(),
                        status: "pending".into(),
                        payload_json: serde_json::json!({ "note": note }).to_string(),
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
        // A request dated ahead of my clock would stand above every later one.
        if body.created_at > self.now() + wire::INVITE_SKEW_SECS {
            return Ok(Outcome::default());
        }
        let note: String = body.note.chars().take(300).collect();
        if let Some(known) = repo::request(&self.store, &body.group_id, sender.as_hex(), "in").await? {
            if known.created_at >= body.created_at {
                return Ok(Outcome::default());
            }
            if known.status == "pending" {
                // Asked again while the first request waits: the newer words
                // replace the older ones. The request keeps its place, so
                // the limit does not apply, and it does not call again.
                let row = repo::RequestRow { payload_json: serde_json::json!({ "note": note }).to_string(), created_at: body.created_at, ..known };
                repo::put_request(&self.store, &row).await?;
                return Ok(Outcome { events: vec![Self::updated(&body.group_id)], ..Default::default() });
            }
        }
        // Requests that wait are few: the rest is dropped unseen. One taken
        // in more than a week ago (by my clock: `updated_at`, the date of
        // the request is the sender's) no longer holds a place.
        let since = self.now() - wire::INVITE_TTL_SECS;
        let waiting = repo::requests(&self.store, &body.group_id, "in", "pending")
            .await?
            .into_iter()
            .filter(|r| r.updated_at > since && PubKey::parse(&r.requester).is_some_and(|p| !s.is_member(&p) && !s.is_banned(&p)))
            .count();
        if waiting >= MAX_OPEN_REQUESTS {
            return Ok(Outcome { notes: vec!["group: a request over the limit dropped".into()], ..Default::default() });
        }
        repo::put_request(
            &self.store,
            &repo::RequestRow {
                group_id: body.group_id.clone(),
                requester: sender.as_hex().to_string(),
                direction: "in".into(),
                status: "pending".into(),
                payload_json: serde_json::json!({ "note": note }).to_string(),
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
        if !historical && !chats::is_muted(&self.store, &repo::group_chat_id(&body.group_id)).await? {
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
        // A group lives on the server of its relay: a welcome to a group
        // not here whose relay is no server of mine (a welcome of my own
        // from a device that has that server, a consent of before the
        // check) would make it a group of the home set. It is held, as one
        // that came before my consent is, and read again when a server
        // comes (`welcomes_of_servers_that_came`).
        let new_here = row.as_ref().is_none_or(|r| r.relay_url.trim().is_empty());
        let unknown_relay = new_here && !self.relay_is_mine(&body.relay).await?;
        if unknown_relay || (!from_me && !self.consent(&group_id, me).await?) {
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
            let notes = if unknown_relay {
                vec!["group: a welcome to a group on a relay that is no server of mine, held until it is".into()]
            } else {
                Vec::new()
            };
            return Ok(Outcome { notes, ..Default::default() });
        }

        // The log as the welcome tells it, every operation verified. One
        // of a version newer than this build reads is kept as it came
        // (`keep_op_ahead`), for the log to take after an update.
        let mut verified: Vec<(Op, serde_json::Value)> = Vec::new();
        let mut ahead: Vec<serde_json::Value> = Vec::new();
        for signed in &body.ops {
            match wire::verify_op(signed, &group_id) {
                Ok(op) => {
                    if matches!(op.body, OpBody::Create { .. }) && verified.iter().any(|(o, _)| matches!(o.body, OpBody::Create { .. })) {
                        continue;
                    }
                    verified.push((op, signed.clone()));
                }
                Err(e) if wire::is_op_too_new(&e) => ahead.push(signed.clone()),
                Err(_) => {}
            }
        }
        let told = OpLog::from_ops(verified.iter().map(|(o, _)| o.clone())).map_err(rejection)?;
        let s = told.state();
        if let Some(r) = &row {
            if !r.owner.is_empty() && told.ordered().next().is_some_and(|c| c.author.as_hex() != r.owner) {
                return Err(MessengerError::Invalid("group_wrong_owner".into()));
            }
        }
        if s.disbanded || !(from_me || s.role_of(sender).is_some_and(Role::is_manager)) {
            return Err(MessengerError::Invalid("group_bad_welcome".into()));
        }
        if !s.is_member(me) {
            // My admission came after an operation this build cannot read:
            // the welcome is kept whole for after an update, and the group
            // is listed as one I am getting into.
            if !ahead.is_empty() {
                return self.park_welcome(&body, sender, wire_id, created_at, s, row.as_ref()).await;
            }
            return Err(MessengerError::Invalid("group_bad_welcome".into()));
        }
        let owner = told.ordered().next().map(|c| c.author.as_hex().to_string()).unwrap_or_default();
        repo::upsert(
            &self.store,
            &GroupRow {
                id: group_id.clone(),
                kind: if s.kind == GroupKind::Public { "public".into() } else { "private".into() },
                name: s.name.clone(),
                about: messenger_richtext::normalize(&s.about),
                picture: s.picture.clone(),
                relay_url: row.as_ref().map(|r| r.relay_url.clone()).filter(|r| !r.is_empty()).unwrap_or(body.relay.clone()),
                owner,
                membership: MEMBERSHIP_JOINED.into(),
                my_role: s.role_of(me).map(|r| r.as_str().to_string()),
                members: s.members.len() as i64,
                link_epoch: s.link_epoch as i64,
                created_at: 0,
                updated_at: 0,
                needs_update: row.as_ref().is_some_and(|r| r.needs_update),
            },
        )
        .await?;
        self.forget_log(&group_id).await;
        for key in wire::decode_keys(&body.keys) {
            self.store_key(&group_id, &key).await?;
        }
        for (op, signed) in &verified {
            if self.join_proven(&group_id, op).await? == Some(false) {
                continue;
            }
            self.keep_op(op, signed).await?;
        }
        for signed in &ahead {
            self.keep_op_ahead(&group_id, signed).await?;
        }
        if !ahead.is_empty() {
            repo::set_needs_update(&self.store, &group_id, true).await?;
        }
        for key in wire::decode_keys(&body.keys) {
            self.keep_key(&group_id, &key).await?;
        }
        // The first pass kept the keys before the log was here: the trim
        // comes now, when the log names its own.
        self.trim_keys(&group_id, None).await?;
        if let (Some(link), GroupKind::Public) = (&body.link, s.kind) {
            if let Some(Ok(secret)) = wire::decode_secret(link).map(|b| LinkSecret::from_bytes(&b)) {
                if Some(secret.group_key(&group_id, s.link_epoch).id()) == s.current_key {
                    self.keep_link_secret(&group_id, s.link_epoch, &secret).await?;
                }
            }
        }
        self.consume_consent(&group_id, me).await?;
        // A welcome of my own is one my next new device can be pointed to
        // (`welcome_myself`): none is sent anew for it.
        if from_me && repo::self_welcome(&self.store, &group_id).await?.is_none() {
            repo::set_self_welcome(&self.store, &group_id, wire_id).await?;
            self.dm.mark_relations_owed().await?;
        }

        // The row was rewritten above, so `settle` sees no change in it:
        // the screen has to hear of it here (a group I was banned in or
        // removed from is shown as such until it does).
        let mut out = Outcome { resubscribe: true, events: vec![Self::updated(&group_id)], ..Default::default() };
        self.drain(keys, &group_id, ctx, &mut out).await?;
        self.settle(keys, &group_id, true, ctx, &mut out).await?;
        if !from_me && !chats::is_muted(&self.store, &repo::group_chat_id(&group_id)).await? {
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

    /// A server came (by hand, a card, the snapshot): the welcomes held
    /// because their group's relay was no server of mine (`on_welcome`)
    /// are read again for each group whose relay now is one. One still
    /// waiting for my consent is held again, as before.
    pub async fn welcomes_of_servers_that_came(&self, keys: &Keys) -> Result<Outcome> {
        let me = PubKey::parse(&keys.public_key().to_hex()).ok_or_else(|| MessengerError::Crypto("bad key".into()))?;
        let ctx = Context { my_pubkey: me, session_started_at: self.clock.now(), clock: self.clock.clone() };
        let mut out = Outcome::default();
        for group_id in repo::pending_groups(&self.store, BUCKET_WELCOME).await? {
            let mut now_known = false;
            for row in repo::pending_in(&self.store, &group_id, BUCKET_WELCOME).await? {
                let relay = serde_json::from_str::<serde_json::Value>(&row.ciphertext)
                    .ok()
                    .and_then(|v| v["body"]["relay"].as_str().map(String::from))
                    .unwrap_or_default();
                now_known |= !relay.trim().is_empty() && servers::by_relay_url(&self.store, &relay).await?.is_some();
            }
            if now_known {
                out.merge(self.held_welcomes(keys, &group_id, &ctx).await?);
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
        // Nothing of a group the network closes is taken in, not even held
        // for a key. The ingress drops it before it is stored as seen, so a
        // lifted ban asks for it again; this is for whoever calls here
        // without the ingress.
        if self.is_group_banned(&msg.group_id).await? {
            return Ok(out);
        }
        let Some(row) = repo::get(&self.store, &msg.group_id).await? else { return Ok(out) };
        if !matches!(row.membership.as_str(), MEMBERSHIP_JOINED | MEMBERSHIP_JOINING) {
            return Ok(out);
        }
        // A group lives on its server, and only there (the wire
        // document, «Группы на сервере»): an event by a relay of another
        // server is not the group's word. The ingress drops it before it is
        // stored as seen (`servers::group_heard_on`); this is for whoever
        // calls here without the ingress.
        let relay = msg.envelope.source.relay_url().map(|u| u.as_str());
        if !servers::group_heard_on(&self.store, &msg.group_id, relay).await? {
            out.notes.push("group: an event by a relay not of the group's server, dropped".into());
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

    /// Park something of a version newer than this build reads (an event
    /// under the key's FUTURE bucket, a welcome under `FUTURE_WELCOME`):
    /// it is tried again after an update, and the group says it waits for
    /// one. Few of them, by rows and by bytes, the events under one bound
    /// and the welcomes under another; past that, what came first (by its
    /// receipt here, not by the date its author wrote) makes room.
    async fn hold_future(&self, group_id: &str, bucket: &str, item: &Item) -> Result<()> {
        let scope = if bucket == FUTURE_WELCOME { FUTURE_WELCOME } else { FUTURE };
        let (mut rows, mut bytes) = repo::pending_prefix_size(&self.store, group_id, scope).await?;
        let size = item.ciphertext.len() as i64;
        while rows >= max_future_rows() || bytes + size > MAX_FUTURE_BYTES {
            let Some(dropped) = repo::delete_oldest_pending(&self.store, group_id, scope).await? else { break };
            rows -= 1;
            bytes -= dropped;
        }
        self.hold(group_id, bucket, item).await?;
        repo::set_needs_update(&self.store, group_id, true).await
    }

    /// A welcome this build cannot read whole (`on_welcome`): kept for
    /// after an update, its keys stored so that what the group says
    /// meanwhile is opened and kept too, and a row for the group as one I
    /// am getting into, so that the runtime listens to it and comes back
    /// to it at the start of every session (`maintain`).
    async fn park_welcome(&self, body: &wire::Welcome, sender: &PubKey, wire_id: &str, created_at: i64, s: &GroupState, row: Option<&GroupRow>) -> Result<Outcome> {
        let group_id = &body.group_id;
        let item = Item {
            event_id: wire_id.to_string(),
            ciphertext: serde_json::json!({ "from": sender.as_hex(), "body": body }).to_string(),
            created_at,
            historical: true,
        };
        if row.is_none_or(|r| r.membership != MEMBERSHIP_JOINED) {
            repo::upsert(
                &self.store,
                &GroupRow {
                    id: group_id.clone(),
                    kind: if s.kind == GroupKind::Public { "public".into() } else { "private".into() },
                    name: s.name.clone(),
                    about: messenger_richtext::normalize(&s.about),
                    picture: s.picture.clone(),
                    relay_url: row.map(|r| r.relay_url.clone()).filter(|r| !r.is_empty()).unwrap_or(body.relay.clone()),
                    owner: s.owner.as_hex().to_string(),
                    membership: MEMBERSHIP_JOINING.into(),
                    my_role: None,
                    members: s.members.len() as i64,
                    link_epoch: s.link_epoch as i64,
                    created_at: 0,
                    updated_at: 0,
                    needs_update: true,
                },
            )
            .await?;
        }
        for key in wire::decode_keys(&body.keys) {
            self.store_key(group_id, &key).await?;
        }
        self.hold_future(group_id, FUTURE_WELCOME, &item).await?;
        Ok(Outcome { resubscribe: true, events: vec![Self::updated(group_id)], ..Default::default() })
    }

    /// Welcomes kept for after an update (`park_welcome`), read again
    /// when every operation in them is one this build reads.
    pub(crate) async fn replay_parked_welcomes(&self, keys: &Keys, group_id: &str, ctx: &Context) -> Result<Outcome> {
        let mut out = Outcome::default();
        for row in repo::pending_in(&self.store, group_id, FUTURE_WELCOME).await? {
            let parsed = serde_json::from_str::<serde_json::Value>(&row.ciphertext).ok();
            let from = parsed.as_ref().and_then(|v| v.get("from")).and_then(|f| f.as_str()).and_then(PubKey::parse);
            let body = parsed.as_ref().and_then(|v| v.get("body").cloned()).and_then(|b| serde_json::from_value::<wire::Welcome>(b).ok());
            let (Some(from), Some(body)) = (from, body) else {
                repo::delete_pending(&self.store, &row.event_id).await?;
                continue;
            };
            if body.ops.iter().any(wire::is_ahead) {
                continue;
            }
            repo::delete_pending(&self.store, &row.event_id).await?;
            match self.on_welcome(keys, body, &from, &row.event_id, row.created_at, ctx).await {
                Ok(o) => out.merge(o),
                Err(e) => out.notes.push(format!("group: a welcome kept for an update was dropped: {e}")),
            }
        }
        Ok(out)
    }

    /// Whether the group still waits for an update: something is parked
    /// (`hold_future`), or an operation of the log here is one this build
    /// cannot read (`keep_op_ahead`). The row is told.
    pub(crate) async fn refresh_needs_update(&self, group_id: &str) -> Result<bool> {
        let Some(row) = repo::get(&self.store, group_id).await? else { return Ok(false) };
        let (parked, _) = repo::pending_prefix_size(&self.store, group_id, FUTURE).await?;
        let (welcomes, _) = repo::pending_prefix_size(&self.store, group_id, FUTURE_WELCOME).await?;
        // The stored operations are read only when the flag may clear:
        // a log of hundreds is not parsed on every drain for nothing.
        let needs = parked + welcomes > 0 || (row.needs_update && ops_ahead(&self.store, group_id).await? > 0);
        if needs != row.needs_update {
            repo::set_needs_update(&self.store, group_id, needs).await?;
        }
        Ok(needs)
    }

    /// Open one event and act on it. `true` when the group became better
    /// known (a key, the log, a member): what waits may be tried again.
    async fn process(&self, keys: &Keys, group_id: &str, key: &GroupKey, item: &Item, ctx: &Context, out: &mut Outcome) -> Result<bool> {
        let opened = match wire::open(group_id, key, &item.ciphertext) {
            Ok(o) => o,
            Err(e) if wire::is_op_too_new(&e) => {
                self.hold_future(group_id, &format!("{FUTURE}{}", key.id().0), item).await?;
                out.events.push(Self::updated(group_id));
                return Ok(false);
            }
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
                // What was ahead of this build is looked at where it lies:
                // most of it stays until an update, and what the build reads
                // now (it was updated) goes the usual way.
                for row in repo::pending_in(&self.store, group_id, &format!("{FUTURE}{id}")).await? {
                    if wire::open(group_id, &key, &row.ciphertext).is_err_and(|e| wire::is_op_too_new(&e)) {
                        continue;
                    }
                    repo::delete_pending(&self.store, &row.event_id).await?;
                    let item = Item { event_id: row.event_id, ciphertext: row.ciphertext, created_at: row.created_at, historical: true };
                    progress |= self.process(keys, group_id, &key, &item, ctx, out).await?;
                }
            }
            if !progress {
                break;
            }
        }
        self.refresh_needs_update(group_id).await?;
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
            if let Some(ev) = crate::moderation::op_event(group_id, &op) {
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
        // The marks of moderation are in force by the journal as it stands:
        // read again whenever it changed.
        self.marks_follow_log(group_id, &log, out).await?;
        let gone = before.membership == MEMBERSHIP_JOINED && !log.state().is_member(me) && !log.state().disbanded;
        let row = self.refresh_row(group_id, me, gone.then(|| departure(&log, me))).await?;
        let Some(row) = row else { return Ok(()) };
        let mut changed =
            row.membership != before.membership || row.name != before.name || row.members != before.members || row.my_role != before.my_role;
        if row.membership == MEMBERSHIP_JOINED && log.state().role_of(me).is_some_and(Role::is_manager) {
            changed |= self.close_answered_requests(group_id, log.state()).await?;
        }
        if changed {
            out.events.push(Self::updated(group_id));
        }
        if row.membership != before.membership {
            out.resubscribe = true;
        }
        if row.membership != before.membership || row.members != before.members {
            // Who is in the group changed: the core of calls looks at its
            // room again (a removed member is heard no more, the keys turn).
            if let Some(sink) = self.call_sink() {
                sink.on_members_changed(group_id).await;
            }
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
        // An honest client dates the event and the message inside it alike
        // (`wire::seal_inner`): one far from the other is somebody's own
        // making, and the outer time is what says what is history.
        if (item.created_at - m.created_at).abs() > JOIN_SLACK_SECS {
            return Ok(Verdict::Done);
        }
        let Ok(envelope) = Envelope::parse(&m.content) else { return Ok(Verdict::Done) };
        // A note of moderation is kept whatever its author's standing:
        // whether it is in force is the journal's to say, now and after
        // every change of it (`marks`). One of a type this build does not
        // know is dropped quietly, like every unknown note.
        if envelope.t.starts_with(crate::marks::T_PREFIX) {
            self.on_mark(group_id, &log, &m, &envelope, out).await?;
            return Ok(Verdict::Done);
        }
        // The time inside is the author's word. What comes live is judged by
        // the log as it stands: one who was removed, left or was muted does
        // not speak again by dating a message back under a key they kept.
        // History is judged by its date.
        let at = if item.historical { m.created_at } else { m.created_at.max(head_time(&log)) };
        let rules = rules_at(&log, &m.author, at);
        // A member is writing: a word for the screen alone, kept nowhere
        // and never held (an outsider's is nothing; mine says nothing new).
        if envelope.t == T_TYPING {
            if rules.standing != Standing::Outsider && &m.author != me {
                out.events.push(UiEvent {
                    name: crate::service::UI_EVENT_GROUP_TYPING.into(),
                    payload: serde_json::json!({ "group_id": group_id, "member": m.author.as_hex(), "until": m.created_at + envelope.typing_ttl() }),
                });
            }
            return Ok(Verdict::Done);
        }
        if rules.standing == Standing::Outsider {
            // A member by the date, not now: dropped, not held, for what
            // waits is tried again as history, by its date.
            if at != m.created_at && rules_at(&log, &m.author, m.created_at).standing != Standing::Outsider {
                return Ok(Verdict::Done);
            }
            // Not a member as far as the log here knows: the op that let them
            // in may not have come yet. Held like anything they say, a reaction
            // too, and tried again when the log grows (or given up after
            // `HELD_TTL_SECS`).
            return Ok(Verdict::Hold);
        }
        let from_me = &m.author == me;
        let chat_id = repo::group_chat_id(group_id);
        let id = m.id.as_hex().to_string();
        // Whether the author may speak at `at`: a member, not muted, and
        // the group open to them (only moderators and above write, or the
        // hours of the schedule). What a closed group hears is dropped.
        let speaks = rules.closed(at).is_none();
        // A reaction: only a member who may speak puts one, mine from
        // another device or my own echo too (the same word again changes
        // nothing). One of a muted member is dropped.
        if envelope.t == T_REACTION {
            let fields = (envelope.str_field("target"), envelope.str_field("emoji"), envelope.fields.get("set").and_then(|v| v.as_bool()));
            if let (true, (Some(target), Some(emoji), Some(set))) = (speaks, fields) {
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
        // A word of a group call (`call.*`): for the core of calls, which
        // keeps the room and the banner; no message of the chat. A muted
        // member may be in a call.
        if envelope.is_call() {
            if let Some(sink) = self.call_sink() {
                sink.on_group_call(group_id, &m.author, &envelope, m.created_at, item.historical).await;
            }
            return Ok(Verdict::Done);
        }
        if !speaks {
            return Ok(Verdict::Done);
        }
        let target = envelope.str_field("target").map(String::from);
        let mut card_line = None;
        let (content_type, text, hidden, media_json): (&str, Option<String>, bool, Option<String>) = match envelope.t.as_str() {
            T_TEXT => (msgs::CT_TEXT, envelope.as_text().map(String::from), false, None),
            T_EDIT => (msgs::CT_EDIT, None, true, None),
            T_DELETE => (msgs::CT_DELETE, None, true, None),
            T_MEDIA => (
                msgs::CT_MEDIA,
                envelope.str_field("caption").map(String::from),
                false,
                Some(messenger_dm::media::received_media_json(&envelope)),
            ),
            // A card that is no card is dropped; one about somebody else
            // loses its phone here, before anything keeps it.
            T_CONTACT => match messenger_dm::cards::received_card(&envelope, m.author.as_hex()) {
                Some(card) => {
                    card_line = Some(messenger_dm::view::card_line(&card));
                    (msgs::CT_CONTACT, None, false, Some(card.to_json().to_string()))
                }
                None => return Ok(Verdict::Done),
            },
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
                envelope_json: match (&card_line, &media_json) {
                    // A card is kept as it was checked, the envelope too.
                    (Some(_), Some(json)) => Envelope::contact(serde_json::from_str(json)?).encode(),
                    _ => m.content.clone(),
                },
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
            let new_text = envelope.str_field("text").map(|t| (t.trim().to_string(), envelope.format()));
            if self.change(&log, &row, content_type, &m.author, m.created_at, new_text).await? {
                chats::recompute_last(&self.store, &chat_id).await?;
                out.events.push(message_updated(&chat_id, &target));
            }
            return Ok(Verdict::Done);
        }

        // Held for a moderator by the rules at `at` (`Rules::subject_to_premod`):
        // the row waits, listed for the moderators and its author only,
        // counted and told nowhere, until a mark says otherwise. Judged by
        // the journal as it is here now; one that outran the operation
        // holding its author is judged again when that comes
        // (`outrun_rows`), so every device ends alike.
        if rules.subject_to_premod() {
            msgs::set_review(&self.store, &id, msgs::REVIEW_PENDING).await?;
        }
        // Edits or a delete that overtook this message on the way.
        if let Some(row) = msgs::get(&self.store, &id).await? {
            for p in msgs::pending_for_target(&self.store, &id).await? {
                let Some(actor) = PubKey::parse(&p.sender_pubkey) else { continue };
                let new_text = Envelope::parse(&p.envelope_json).ok().and_then(|e| e.str_field("text").map(|t| (t.trim().to_string(), e.format())));
                self.change(&log, &row, &p.content_type, &actor, p.created_at, new_text).await?;
            }
        }
        // Removed for me on another device before it came here.
        let hidden_before = self.dm.hide_if_hidden(&id).await?;
        // The marks of moderation that waited for it.
        if let Some(row) = msgs::get(&self.store, &id).await? {
            self.marks_on_insert(group_id, s, &row, out).await?;
        }
        let Some(row) = msgs::get(&self.store, &id).await? else { return Ok(Verdict::Done) };
        // Whether I see it: not hidden by a moderator, not of someone I
        // block in direct messages (the one thing a block does in a
        // group), and not waiting for a moderator unless I am one, or it
        // is mine. A key the network bans is blocked here as by me, all it
        // wrote, without a note of a moderator: a rule of this device. What
        // I do not see counts nowhere and wakes nobody; the row stays, for
        // the unblock, the lifted ban, the approval or the unhide to show.
        // The block is the home chat's, in a group of any server (above, `on_dm`).
        let blocked = !from_me && (self.dm.is_blocked(&m.author).await? || self.dm.is_banned(&m.author).await?);
        let waits = row.review == msgs::REVIEW_PENDING;
        let moderator = s.role_of(me).is_some_and(|r| r >= Role::Moderator);
        let shown = row.mod_state == msgs::MOD_SHOWN && !blocked && (!waits || moderator || from_me);
        // The count of what waits is a moderator's, and the author's for
        // their own badge; a member's page is told nothing of it.
        if waits && (moderator || from_me) {
            out.events.push(self.queue_event(group_id).await?);
        }
        if !shown {
            return Ok(Verdict::Done);
        }

        let line = if let Some(line) = card_line {
            line
        } else if content_type == msgs::CT_MEDIA {
            let name = envelope.str_field("name").unwrap_or("file");
            format!("📎 {}", text.as_deref().map(|t| messenger_dm::view::preview_of(t, envelope.format())).unwrap_or_else(|| name.to_string()))
        } else {
            text.as_deref().map(|t| messenger_dm::view::preview_of(t, envelope.format())).unwrap_or_default()
        };
        let live = !from_me && !item.historical;
        let floor = self.dm.unread_floor();
        let joined_at = s.member(me).map(|x| x.joined_at).unwrap_or(i64::MAX);
        // What waits for a moderator is read by nobody yet: not counted,
        // not the line of the chat list, and nobody woken.
        let unread =
            !from_me && !hidden_before && !waits && m.created_at >= joined_at && (live || (floor >= 0 && m.created_at >= floor));
        if !waits {
            chats::touch(&self.store, &chat_id, m.created_at, Some(&line), unread).await?;
        }
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
        // Nobody is woken for one who is not a member now, nor for what
        // waits for a moderator.
        if live && !deleted && !waits && s.member(&m.author).is_some() && !chats::is_muted(&self.store, &chat_id).await? {
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
        new_text: Option<(String, TextFormat)>,
    ) -> Result<bool> {
        if at < row.created_at {
            return Ok(false);
        }
        match content_type {
            msgs::CT_EDIT => {
                // Text and captions only: a card has no text to replace.
                if row.sender_pubkey != actor.as_hex()
                    || !messenger_dm::service::editable_type(&row.content_type)
                    || row.edited_at.is_some_and(|e| e > at)
                {
                    return Ok(false);
                }
                let Some((t, format)) = new_text.filter(|(t, _)| !t.is_empty() && t.len() <= messenger_dm::service::MAX_TEXT_BYTES) else {
                    return Ok(false);
                };
                msgs::set_text(&self.store, &row.id, &t, format, at).await?;
                // A new version: what a moderator approved was the old one.
                self.after_edit(log.group_id(), log, row, at).await?;
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

#[cfg(test)]
mod rules_tests {
    use super::*;
    use crate::log::Insert;
    use crate::op::tests::Settings;
    use crate::state::tests::{create, key, pk, utc};

    /// A log with an owner `o`, an admin `a`, a moderator `m` and a
    /// member `u`, history shown; every operation dated as asked.
    fn team() -> (OpLog, [PubKey; 4]) {
        let who = [pk("0"), pk("a"), pk("b"), pk("c")];
        let [o, a, m, u] = who.clone();
        let mut log = OpLog::create(create(GroupKind::Private, &o, true)).unwrap();
        for (i, member) in [&a, &m, &u].into_iter().enumerate() {
            apply(&mut log, &o, 110 + i as i64, OpBody::Admit { who: member.clone() }, None);
        }
        apply(&mut log, &o, 120, OpBody::SetRole { who: a, role: Role::Admin }, None);
        apply(&mut log, &o, 121, OpBody::SetRole { who: m, role: Role::Moderator }, None);
        (log, who)
    }

    fn apply(log: &mut OpLog, author: &PubKey, at: i64, body: OpBody, with_key: Option<u8>) {
        let mut op = log.next(author, at, body);
        if let Some(k) = with_key {
            op = op.with_key(key(k));
        }
        assert_eq!(log.insert(op.clone()), Insert::Applied, "{:?}", op.body);
    }

    #[test]
    fn a_timed_mute_lifts_by_itself_in_the_replay() {
        let (mut log, [_, _, m, u]) = team();
        apply(&mut log, &m, 300, OpBody::SetMuted { who: u.clone(), muted: true, until: Some(600) }, None);
        assert_eq!(rules_at(&log, &u, 299).standing, Standing::Member);
        assert_eq!(rules_at(&log, &u, 300).closed(300), Some(Closed::Muted { until: Some(600) }));
        assert_eq!(rules_at(&log, &u, 599).standing, Standing::Muted);
        assert_eq!(rules_at(&log, &u, 600).closed(600), None, "lifted with no operation to say so");
        assert_eq!(rules_at(&log, &u, 10_000).standing, Standing::Member);
        // Lifted early by a moderator: what was said under the mute stays hidden.
        apply(&mut log, &m, 450, OpBody::SetMuted { who: u.clone(), muted: false, until: None }, None);
        assert_eq!(rules_at(&log, &u, 400).standing, Standing::Muted);
        assert_eq!(rules_at(&log, &u, 500).standing, Standing::Member);
        // Muted again, with an end, and then for good after that end had
        // passed: the gap between the two is free.
        apply(&mut log, &m, 700, OpBody::SetMuted { who: u.clone(), muted: true, until: Some(800) }, None);
        apply(&mut log, &m, 900, OpBody::SetMuted { who: u.clone(), muted: true, until: None }, None);
        assert_eq!(rules_at(&log, &u, 750).closed(750), Some(Closed::Muted { until: Some(800) }));
        assert_eq!(rules_at(&log, &u, 850).standing, Standing::Member);
        assert_eq!(rules_at(&log, &u, 950).closed(950), Some(Closed::Muted { until: None }));
        // A mute in force takes a new end from a second mute.
        apply(&mut log, &m, 1_000, OpBody::SetMuted { who: u.clone(), muted: true, until: Some(1_100) }, None);
        assert_eq!(rules_at(&log, &u, 1_050).closed(1_050), Some(Closed::Muted { until: Some(1_100) }));
        assert_eq!(rules_at(&log, &u, 1_100).standing, Standing::Member);
        // The moderator was never muted, and an outsider is outside.
        assert_eq!(rules_at(&log, &m, 950).closed(950), None);
        assert_eq!(rules_at(&log, &pk("f"), 950).closed(950), Some(Closed::NotMember));
    }

    #[test]
    fn the_rules_of_posting_are_those_in_force_at_the_moment() {
        let (mut log, [o, a, m, u]) = team();
        let friday = utc(2026, 10, 16, 10, 0);
        apply(&mut log, &a, friday, Settings { post_mode: Some(PostMode::Managers), ..Settings::default() }.body(), None);
        assert_eq!(rules_at(&log, &u, friday - 1).closed(friday - 1), None, "before the rule");
        assert_eq!(rules_at(&log, &u, friday).closed(friday), Some(Closed::ManagersOnly));
        for who in [&o, &a, &m] {
            assert_eq!(rules_at(&log, who, friday).closed(friday), None);
        }
        // Raised beside the rule: a moderator from that moment on.
        apply(&mut log, &o, friday + 100, OpBody::SetRole { who: u.clone(), role: Role::Moderator }, None);
        assert_eq!(rules_at(&log, &u, friday + 50).closed(friday + 50), Some(Closed::ManagersOnly));
        assert_eq!(rules_at(&log, &u, friday + 100).closed(friday + 100), None);
        apply(&mut log, &o, friday + 200, OpBody::SetRole { who: u.clone(), role: Role::Member }, None);
        apply(&mut log, &a, friday + 300, Settings { post_mode: Some(PostMode::All), ..Settings::default() }.body(), None);
        assert_eq!(rules_at(&log, &u, friday + 300).closed(friday + 300), None);

        // The hours: Fridays 09:00 to 18:00 UTC.
        let sched = Schedule { tz: "UTC".into(), days: vec![5], from: "09:00".into(), to: "18:00".into() };
        apply(&mut log, &a, friday + 400, Settings { post_schedule: Some(sched.clone()), ..Settings::default() }.body(), None);
        let evening = utc(2026, 10, 16, 18, 0);
        assert_eq!(rules_at(&log, &u, friday + 400).closed(friday + 400), None);
        assert_eq!(rules_at(&log, &u, evening).closed(evening), Some(Closed::Schedule { opens_at: utc(2026, 10, 23, 9, 0) }));
        assert_eq!(rules_at(&log, &m, evening).closed(evening), None);
        // A mute is said before the hours.
        apply(&mut log, &m, evening + 10, OpBody::SetMuted { who: u.clone(), muted: true, until: Some(evening + 500) }, None);
        assert_eq!(rules_at(&log, &u, evening + 20).closed(evening + 20), Some(Closed::Muted { until: Some(evening + 500) }));
        // The schedule taken away: open again.
        apply(&mut log, &a, evening + 600, Settings { post_schedule: Some(Schedule { days: vec![], ..sched }), ..Settings::default() }.body(), None);
        assert_eq!(rules_at(&log, &u, evening + 600).closed(evening + 600), None);
        assert_eq!(rules_at(&log, &u, evening + 550).closed(evening + 550), Some(Closed::Schedule { opens_at: utc(2026, 10, 23, 9, 0) }), "history is judged by its date");
        assert_eq!(rules_at(&log, &u, evening + 100).closed(evening + 100), Some(Closed::Muted { until: Some(evening + 500) }), "the mute before the hours, then too");
    }

    #[test]
    fn premod_and_the_line_of_newcomers() {
        let (mut log, [o, a, m, u]) = team();
        let x = pk("d");
        let live = 100_000;
        assert!(!rules_at(&log, &u, live).subject_to_premod());
        apply(&mut log, &a, 300, Settings { premod: Some(Premod::Newcomers), ..Settings::default() }.body(), None);
        assert!(!rules_at(&log, &u, live).subject_to_premod(), "from before the rule");
        // Let in by an operation dated before the rule, ordered after it.
        apply(&mut log, &a, 250, OpBody::Admit { who: x.clone() }, None);
        let rules = rules_at(&log, &x, live);
        assert_eq!(rules.standing, Standing::Member);
        assert!(rules.subject_to_premod(), "a newcomer by the order of the log, whatever the clock says");
        assert!(rules.joined_pos.unwrap() > rules.premod_since.unwrap());
        // Trusted: not held. Raised: not held. Everyone: the old member too.
        apply(&mut log, &m, 400, OpBody::SetTrusted { who: x.clone(), trusted: true }, None);
        assert!(!rules_at(&log, &x, live).subject_to_premod());
        assert!(rules_at(&log, &x, 350).subject_to_premod(), "not trusted yet then");
        apply(&mut log, &m, 450, OpBody::SetTrusted { who: x.clone(), trusted: false }, None);
        apply(&mut log, &o, 500, OpBody::SetRole { who: x.clone(), role: Role::Moderator }, None);
        assert!(!rules_at(&log, &x, live).subject_to_premod());
        apply(&mut log, &a, 600, Settings { premod: Some(Premod::All), ..Settings::default() }.body(), None);
        assert!(rules_at(&log, &u, live).subject_to_premod() && !rules_at(&log, &m, live).subject_to_premod());
        assert!(!rules_at(&log, &pk("f"), live).subject_to_premod(), "an outsider is nobody's to hold");
        apply(&mut log, &a, 700, Settings { premod: Some(Premod::Off), ..Settings::default() }.body(), None);
        assert!(!rules_at(&log, &u, live).subject_to_premod());
        // The same answers as the state's, for a live message.
        let s = log.state();
        for who in [&o, &a, &m, &u, &x] {
            assert_eq!(rules_at(&log, who, live).subject_to_premod(), s.subject_to_premod(who), "{who:?}");
        }
    }

    #[test]
    fn the_hold_is_the_same_on_every_device_whatever_order_the_operations_came_in() {
        // Three operations made at once, each not knowing the others: the
        // rule turned on by the owner, x let in by an admin (dated before
        // the rule), and then x trusted. Devices hear of them in every
        // order; the place of each in the applied order (the owner's goes
        // first, so x is a newcomer), and so the hold, is the same on each.
        let (log, [o, a, m, _]) = team();
        let x = pk("d");
        let base: Vec<crate::op::Op> = log.ordered().cloned().collect();
        let rule = log.next(&o, 300, Settings { premod: Some(Premod::Newcomers), ..Settings::default() }.body());
        let admit = log.next(&a, 250, OpBody::Admit { who: x.clone() });
        let mut with_x = log.clone();
        assert_eq!(with_x.insert(admit.clone()), Insert::Applied);
        let trust = with_x.next(&m, 400, OpBody::SetTrusted { who: x.clone(), trusted: true });
        let extra = [rule, admit, trust];
        let mut answers = std::collections::HashSet::new();
        for order in [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
            let mut l = OpLog::from_ops(base.iter().cloned()).unwrap();
            for i in order {
                l.insert(extra[i].clone());
            }
            assert_eq!(l.len(), base.len() + 3, "{order:?}: every operation is in, in whatever order it came");
            let now = rules_at(&l, &x, 100_000);
            let then = rules_at(&l, &x, 350);
            answers.insert((now.subject_to_premod(), then.subject_to_premod(), now.joined_pos, now.premod_since, l.state().subject_to_premod(&x)));
        }
        assert_eq!(answers.len(), 1, "one answer for every order: {answers:?}");
        let (now, then, joined, since, state) = answers.into_iter().next().unwrap();
        assert!(!now, "trusted by now");
        assert!(then, "not trusted at 350, and a newcomer: the rule stands first in the applied order, the admission after it");
        assert!(joined > since);
        assert_eq!(state, now);
    }
}
