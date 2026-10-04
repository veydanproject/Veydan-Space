// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Groups on a device: the log and the keys in storage, what the user
//! does, what goes out. Everything that arrives is in `inbound`.
//!
//! Like the DM service, this one never talks to relays: it returns what
//! has to be published and the events for the UI.

use crate::keys::{GroupKey, GroupLink, LinkSecret};
use crate::log::{KeyStatus, OpLog};
use crate::op::{GroupKind, KeyId, Op, OpBody};
use crate::roles::Role;
use crate::state::{GroupState, Rejection};
use crate::wire::{self, SecretEnvelope};
use messenger_core::traits::{Notice, UiEvent};
use messenger_core::{Clock, Envelope, MessengerError, Outbound, PubKey, RelayUrl, Result, Scope, SecretStore};
use messenger_dm::pushtags;
use messenger_dm::wrap::{wrap_as, Wake};
use messenger_dm::{DmService, MessageView};
use messenger_store::groups::{self as repo, GroupRow};
use messenger_store::messages::{self as msgs, NewMessage};
use messenger_store::{chats, Store};
use nostr::key::Keys;
use nostr::prelude::Event;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

pub const UI_EVENT_GROUP_UPDATED: &str = "group.updated";
pub const UI_EVENT_GROUP_INVITE: &str = "group.invite";
pub const UI_EVENT_GROUP_REQUEST: &str = "group.request";

pub const MEMBERSHIP_JOINED: &str = "joined";
/// A public link was opened; the log is being fetched.
pub const MEMBERSHIP_JOINING: &str = "joining";
/// A private group was asked; waiting for a manager.
pub const MEMBERSHIP_REQUESTED: &str = "requested";
/// A manager said no.
pub const MEMBERSHIP_REJECTED: &str = "rejected";
/// The link I came by was replaced before I got in.
pub const MEMBERSHIP_STALE: &str = "stale_link";
pub const MEMBERSHIP_LEFT: &str = "left";
pub const MEMBERSHIP_REMOVED: &str = "removed";
pub const MEMBERSHIP_BANNED: &str = "banned";
pub const MEMBERSHIP_DISBANDED: &str = "disbanded";

/// Members a private group can have: every key goes to each of them
/// inside one event, and an event has a size limit.
pub const MAX_PRIVATE_MEMBERS: usize = 300;

/// What an action or an arrival leads to.
#[derive(Debug, Default)]
pub struct Outcome {
    /// To publish, in order.
    pub publish: Vec<Outbound>,
    pub events: Vec<UiEvent>,
    /// Someone to tell the user about.
    pub notify: Vec<Notice>,
    /// The set of groups to listen to changed.
    pub resubscribe: bool,
    /// Groups whose key needs a manager (me): new key or delivery.
    pub maintain: Vec<String>,
    /// For the log of whoever runs the service.
    pub notes: Vec<String>,
}

impl Outcome {
    pub(crate) fn merge(&mut self, mut other: Outcome) {
        self.publish.append(&mut other.publish);
        self.events.append(&mut other.events);
        self.notify.append(&mut other.notify);
        self.resubscribe |= other.resubscribe;
        self.notes.append(&mut other.notes);
        for g in other.maintain {
            if !self.maintain.contains(&g) {
                self.maintain.push(g);
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberView {
    pub pubkey: String,
    pub role: String,
    pub muted: bool,
    pub joined_at: i64,
    pub is_me: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupView {
    pub id: String,
    pub chat_id: String,
    /// `public` | `private`
    pub kind: String,
    pub name: String,
    pub about: String,
    pub picture: String,
    pub relay: String,
    pub owner: String,
    pub membership: String,
    pub my_role: Option<String>,
    pub muted: bool,
    pub can_post: bool,
    pub history_for_new: bool,
    pub members: Vec<MemberView>,
    pub banned: Vec<String>,
    /// Requests waiting for a manager (managers only).
    pub requests: Vec<String>,
    /// Link or QR content, for those who may share it.
    pub link: Option<String>,
    /// Events that wait for a key this device does not have yet.
    pub undecrypted: i64,
    /// The key messages are sealed with now (members only).
    pub key: Option<KeyView>,
}

/// What a member may know about the current key: never the key itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyView {
    /// Short fingerprint: the first bytes of a hash of the key.
    pub id: String,
    /// 1 for the key the group was created with, then one more per change.
    pub version: u32,
    pub cipher: String,
    /// `random` (private: handed to each member) | `link` (public: derived from the link).
    pub source: String,
    pub link_epoch: u32,
    /// The operation that brought it, by its author's clock.
    pub since: i64,
    pub by: String,
    /// `create` | `rotate_key` | `rotate_link` | `remove` | `ban` | `admit`
    pub reason: String,
    /// This device holds it.
    pub held: bool,
    /// `good` | `rotate` (a former member knows it) | `deliver` (someone still waits for it)
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteView {
    pub invite_id: String,
    pub group_id: String,
    pub name: String,
    pub about: String,
    pub picture: String,
    pub members: u32,
    /// Who invited (incoming) or who was invited (outgoing).
    pub peer: String,
    pub direction: String,
    pub status: String,
    pub created_at: i64,
    pub expires_at: i64,
}

#[derive(Clone)]
pub struct GroupService {
    pub(crate) store: Store,
    pub(crate) secrets: Arc<dyn SecretStore>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) dm: DmService,
    pub(crate) logs: Arc<Mutex<HashMap<String, OpLog>>>,
    /// Operations that already have their line in the chat.
    pub(crate) lined: Arc<Mutex<HashMap<String, std::collections::HashSet<String>>>>,
    signer: Arc<std::sync::Mutex<Option<Keys>>>,
    locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}

fn key_ref(group_id: &str, key_id: &KeyId) -> String {
    format!("group.{group_id}.key.{}", key_id.0)
}

fn link_ref(group_id: &str, epoch: u32) -> String {
    format!("group.{group_id}.link.{epoch}")
}

pub(crate) fn me_of(keys: &Keys) -> PubKey {
    PubKey::parse(&keys.public_key().to_hex()).expect("valid pubkey")
}

pub(crate) fn rejection(r: Rejection) -> MessengerError {
    let code = serde_json::to_value(&r).ok().and_then(|v| v.get("reason").and_then(|x| x.as_str()).map(String::from));
    MessengerError::Invalid(format!("group_{}", code.unwrap_or_else(|| "rejected".into())))
}

fn new_group_id() -> Result<String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| MessengerError::Crypto(e.to_string()))?;
    Ok(hex::encode(b))
}

impl GroupService {
    pub fn new(store: Store, secrets: Arc<dyn SecretStore>, clock: Arc<dyn Clock>, dm: DmService) -> Self {
        Self { store, secrets, clock, dm, logs: Arc::default(), lined: Arc::default(), signer: Arc::default(), locks: Arc::default() }
    }

    /// Keys of the running session: handlers act with them.
    pub fn set_signer(&self, keys: Option<Keys>) {
        *self.signer.lock().unwrap() = keys;
    }

    pub fn signer(&self) -> Option<Keys> {
        self.signer.lock().unwrap().clone()
    }

    /// Whoever changes a group holds its lock: what the user does and what
    /// arrives do not interleave.
    pub async fn lock_of(&self, group_id: &str) -> Arc<Mutex<()>> {
        self.locks.lock().await.entry(group_id.to_string()).or_default().clone()
    }

    pub(crate) fn now(&self) -> i64 {
        self.clock.now().secs()
    }

    // ─── Storage: keys, secrets, log ────────────────────────────────────────

    pub(crate) async fn key(&self, group_id: &str, key_id: &KeyId) -> Result<Option<GroupKey>> {
        match self.secrets.get(&key_ref(group_id, key_id)).await? {
            Some(bytes) => Ok(Some(GroupKey::from_bytes(&bytes)?)),
            None => Ok(None),
        }
    }

    /// Store a key; `true` when it is new on this device.
    pub(crate) async fn keep_key(&self, group_id: &str, key: &GroupKey) -> Result<bool> {
        let id = key.id();
        if self.key(group_id, &id).await?.is_some() {
            repo::add_key(&self.store, group_id, &id.0).await?;
            return Ok(false);
        }
        self.secrets.put(&key_ref(group_id, &id), key.as_bytes()).await?;
        repo::add_key(&self.store, group_id, &id.0).await?;
        Ok(true)
    }

    pub(crate) async fn all_keys(&self, group_id: &str) -> Result<Vec<GroupKey>> {
        let mut out = Vec::new();
        for id in repo::key_ids(&self.store, group_id).await? {
            if let Some(k) = self.key(group_id, &KeyId(id)).await? {
                out.push(k);
            }
        }
        Ok(out)
    }

    /// Every key of every group this device is in, for a copy kept where
    /// a process without the vault can read it (the push handler).
    pub async fn export_keys(&self) -> Result<Vec<(String, GroupKey)>> {
        let mut out = Vec::new();
        for row in repo::list(&self.store).await? {
            if row.membership != MEMBERSHIP_JOINED {
                continue;
            }
            for key in self.all_keys(&row.id).await? {
                out.push((row.id.clone(), key));
            }
        }
        Ok(out)
    }

    /// What `export_keys` would give, without the secrets: a fingerprint
    /// that changes when a key is added or a group joined or left.
    pub async fn keys_fingerprint(&self) -> Result<String> {
        let mut ids = Vec::new();
        for row in repo::list(&self.store).await? {
            if row.membership != MEMBERSHIP_JOINED {
                continue;
            }
            for id in repo::key_ids(&self.store, &row.id).await? {
                ids.push(format!("{}:{}", row.id, id));
            }
        }
        ids.sort();
        Ok(ids.join(","))
    }

    /// The key replaced not long ago, if this device holds it. For a while
    /// both keys make the group's mark, in what is sent and in what the
    /// push server is told to take (`messenger_dm::pushtags`).
    async fn grace_key(&self, group_id: &str, state: &GroupState) -> Result<Option<GroupKey>> {
        match &state.replaced_key {
            Some(replaced) if self.now().saturating_sub(replaced.at) < pushtags::GROUP_GRACE_SECS => {
                self.key(group_id, &replaced.key).await
            }
            _ => Ok(None),
        }
    }

    /// What this device's push server is given to tell the events of those
    /// in the group from the events of strangers: the keys of the marks,
    /// the current one first. None of them opens anything. Empty while
    /// this device waits for the group's key: the phone could not open
    /// what comes either.
    pub async fn push_keys(&self, group_id: &str) -> Result<Vec<String>> {
        let log = self.need_log(group_id).await?;
        let state = log.state();
        let mut out = Vec::new();
        if let Some(id) = &state.current_key {
            if let Some(key) = self.key(group_id, id).await? {
                out.push(pushtags::group_push_key(key.as_bytes()));
            }
        }
        if let Some(key) = self.grace_key(group_id, state).await? {
            out.push(pushtags::group_push_key(key.as_bytes()));
        }
        Ok(out)
    }

    pub(crate) async fn link_secret(&self, group_id: &str, epoch: u32) -> Result<Option<LinkSecret>> {
        match self.secrets.get(&link_ref(group_id, epoch)).await? {
            Some(bytes) => Ok(Some(LinkSecret::from_bytes(&bytes)?)),
            None => Ok(None),
        }
    }

    pub(crate) async fn keep_link_secret(&self, group_id: &str, epoch: u32, secret: &LinkSecret) -> Result<()> {
        self.secrets.put(&link_ref(group_id, epoch), secret.as_bytes()).await?;
        self.keep_key(group_id, &secret.group_key(group_id, epoch)).await?;
        Ok(())
    }

    /// The log of a group, rebuilt from storage when not in memory.
    /// `None` until the first operation of the group is here.
    pub(crate) async fn log(&self, group_id: &str) -> Result<Option<OpLog>> {
        if let Some(l) = self.logs.lock().await.get(group_id) {
            return Ok(Some(l.clone()));
        }
        let mut ops = Vec::new();
        for json in repo::ops(&self.store, group_id).await? {
            let Ok(event) = serde_json::from_str::<Event>(&json) else { continue };
            let Ok(op) = serde_json::from_str::<Op>(&event.content) else { continue };
            ops.push(op);
        }
        let Ok(log) = OpLog::from_ops(ops) else { return Ok(None) };
        self.logs.lock().await.insert(group_id.to_string(), log.clone());
        Ok(Some(log))
    }

    pub(crate) async fn need_log(&self, group_id: &str) -> Result<OpLog> {
        self.log(group_id).await?.ok_or_else(|| MessengerError::Invalid("group_unknown".into()))
    }

    pub(crate) async fn forget_log(&self, group_id: &str) {
        self.logs.lock().await.remove(group_id);
    }

    /// Store a verified signed operation; `true` when it is new.
    pub(crate) async fn keep_op(&self, op: &Op, signed: &serde_json::Value) -> Result<bool> {
        let new = repo::insert_op(&self.store, &op.group_id, &op.id().0, &signed.to_string()).await?;
        if new {
            self.forget_log(&op.group_id).await;
        }
        Ok(new)
    }

    /// Signed operations of a group, for a welcome.
    pub(crate) async fn signed_ops(&self, group_id: &str) -> Result<Vec<serde_json::Value>> {
        Ok(repo::ops(&self.store, group_id).await?.iter().filter_map(|j| serde_json::from_str(j).ok()).collect())
    }

    /// Bring the cached row in line with the log.
    pub(crate) async fn refresh_row(&self, group_id: &str, me: &PubKey, membership: Option<&str>) -> Result<Option<GroupRow>> {
        let Some(mut row) = repo::get(&self.store, group_id).await? else { return Ok(None) };
        if let Some(log) = self.log(group_id).await? {
            let s = log.state();
            row.kind = if s.kind == GroupKind::Public { "public".into() } else { "private".into() };
            row.name = s.name.clone();
            row.about = s.about.clone();
            row.picture = s.picture.clone();
            row.owner = s.owner.as_hex().to_string();
            row.members = s.members.len() as i64;
            row.link_epoch = s.link_epoch as i64;
            row.my_role = s.role_of(me).map(|r| r.as_str().to_string());
            if row.membership == MEMBERSHIP_JOINED || row.membership == MEMBERSHIP_JOINING {
                if s.disbanded {
                    row.membership = MEMBERSHIP_DISBANDED.into();
                } else if row.membership == MEMBERSHIP_JOINED && s.is_banned(me) {
                    // Joining, what I know may be older than the ban's
                    // lifting: `maintain` decides once the history is here.
                    row.membership = MEMBERSHIP_BANNED.into();
                } else if row.membership == MEMBERSHIP_JOINED && !s.is_member(me) {
                    row.membership = MEMBERSHIP_REMOVED.into();
                }
            }
        }
        if let Some(m) = membership {
            row.membership = m.to_string();
        }
        repo::upsert(&self.store, &row).await?;
        Ok(Some(row))
    }

    pub(crate) fn updated(group_id: &str) -> UiEvent {
        UiEvent {
            name: UI_EVENT_GROUP_UPDATED.into(),
            payload: serde_json::json!({ "group_id": group_id, "chat_id": repo::group_chat_id(group_id) }),
        }
    }

    // ─── Views ──────────────────────────────────────────────────────────────

    pub async fn list(&self, me: &PubKey) -> Result<Vec<GroupView>> {
        let mut out = Vec::new();
        for row in repo::list(&self.store).await? {
            out.push(self.view_of(row, me).await?);
        }
        Ok(out)
    }

    pub async fn get(&self, group_id: &str, me: &PubKey) -> Result<Option<GroupView>> {
        match repo::get(&self.store, group_id).await? {
            Some(row) => Ok(Some(self.view_of(row, me).await?)),
            None => Ok(None),
        }
    }

    async fn view_of(&self, row: GroupRow, me: &PubKey) -> Result<GroupView> {
        let log = self.log(&row.id).await?;
        let state: Option<&GroupState> = log.as_ref().map(|l| l.state());
        let joined = row.membership == MEMBERSHIP_JOINED;
        let my_role = state.and_then(|s| s.role_of(me));
        let manager = joined && my_role.is_some_and(Role::is_manager);
        let members = state
            .map(|s| {
                let mut list: Vec<MemberView> = s
                    .members
                    .iter()
                    .map(|(k, m)| MemberView {
                        pubkey: k.clone(),
                        role: m.role.as_str().into(),
                        muted: m.muted,
                        joined_at: m.joined_at,
                        is_me: k == me.as_hex(),
                    })
                    .collect();
                list.sort_by(|a, b| {
                    let rank = |v: &MemberView| Role::parse(&v.role).map(|r| r.rank()).unwrap_or(0);
                    rank(b).cmp(&rank(a)).then(a.joined_at.cmp(&b.joined_at)).then(a.pubkey.cmp(&b.pubkey))
                });
                list
            })
            .unwrap_or_default();
        let requests = if manager {
            repo::requests(&self.store, &row.id, "in", "pending").await?.into_iter().map(|r| r.requester).collect()
        } else {
            vec![]
        };
        let link = match (state, joined) {
            (Some(s), true) => match log.as_ref().and_then(|l| l.ordered().next()).map(|c| c.author.clone()) {
                Some(creator) => self.link_for(&row, s, &creator, manager).await?,
                None => None,
            },
            _ => None,
        };
        let key = match (log.as_ref(), joined) {
            (Some(l), true) => self.key_view(l).await?,
            _ => None,
        };
        Ok(GroupView {
            chat_id: repo::group_chat_id(&row.id),
            kind: row.kind.clone(),
            name: row.name.clone(),
            about: row.about.clone(),
            picture: row.picture.clone(),
            relay: row.relay_url.clone(),
            owner: row.owner.clone(),
            membership: row.membership.clone(),
            my_role: my_role.map(|r| r.as_str().into()),
            muted: state.and_then(|s| s.member(me)).is_some_and(|m| m.muted),
            can_post: joined && state.is_some_and(|s| s.can_post(me)),
            history_for_new: state.is_some_and(|s| s.history_for_new),
            members,
            banned: if manager { state.map(|s| s.banned.iter().cloned().collect()).unwrap_or_default() } else { vec![] },
            requests,
            link,
            // What was said before I came is not mine to read when the
            // group keeps its history closed: that is not a missing key.
            undecrypted: repo::count_pending(&self.store, &row.id, state.and_then(|s| s.member(me)).map(|m| m.joined_at).unwrap_or(0)).await?,
            key,
            id: row.id,
        })
    }

    async fn key_view(&self, log: &OpLog) -> Result<Option<KeyView>> {
        let s = log.state();
        let Some(current) = s.current_key.clone() else { return Ok(None) };
        // The last applied operation that brought it: a key may come back
        // only by an operation that carries it again.
        let Some(op) = log
            .ordered()
            .filter(|o| o.key.as_ref() == Some(&current) && log.rejection(&o.id()).is_none())
            .last()
        else {
            return Ok(None);
        };
        let reason = match &op.body {
            OpBody::Create { .. } => "create",
            OpBody::RotateKey => "rotate_key",
            OpBody::RotateLink { .. } => "rotate_link",
            OpBody::Remove { .. } => "remove",
            OpBody::Ban { .. } => "ban",
            OpBody::Admit { .. } => "admit",
            _ => "other",
        };
        let version = s.keys.iter().position(|k| k == &current).map_or(s.keys.len(), |i| i + 1);
        Ok(Some(KeyView {
            id: current.0.clone(),
            version: version as u32,
            cipher: "AES-256-GCM".into(),
            source: if s.kind == GroupKind::Public { "link" } else { "random" }.into(),
            link_epoch: s.link_epoch,
            since: op.created_at,
            by: op.author.as_hex().to_string(),
            reason: reason.into(),
            held: self.key(&s.group_id, &current).await?.is_some(),
            status: match log.key_status() {
                KeyStatus::Good => "good",
                KeyStatus::Rotate => "rotate",
                KeyStatus::Deliver(_) => "deliver",
            }
            .into(),
        }))
    }

    /// A public link is shared by any member. A private one (it only says
    /// whom to ask) by managers.
    /// The link names the creator: that is how a newcomer tells the group
    /// from an imitation. Who to ask is in the list of managers.
    async fn link_for(&self, row: &GroupRow, s: &GroupState, creator: &PubKey, manager: bool) -> Result<Option<String>> {
        let Some(relay) = RelayUrl::parse(&row.relay_url) else { return Ok(None) };
        let base = GroupLink {
            group_id: row.id.clone(),
            kind: s.kind,
            relay,
            name: s.name.clone(),
            owner: creator.clone(),
            managers: vec![],
            secret: None,
            link_epoch: s.link_epoch,
        };
        Ok(match s.kind {
            GroupKind::Public => self.link_secret(&row.id, s.link_epoch).await?.map(|secret| GroupLink { secret: Some(secret), ..base }.encode()),
            GroupKind::Private if manager => {
                let mut managers = vec![s.owner.clone()];
                managers.extend(s.managers().into_iter().filter(|p| p != &s.owner));
                Some(GroupLink { managers, ..base }.encode())
            }
            GroupKind::Private => None,
        })
    }

    pub async fn invites(&self, direction: &str) -> Result<Vec<InviteView>> {
        repo::expire_invites(&self.store, self.now()).await?;
        let statuses: &[&str] = if direction == "in" { &["received"] } else { &["sent", "accepted"] };
        Ok(repo::invites(&self.store, direction, statuses)
            .await?
            .into_iter()
            .filter_map(|r| {
                let p: wire::Invite = serde_json::from_str(&r.payload_json).ok()?;
                Some(InviteView {
                    invite_id: r.invite_id,
                    group_id: r.group_id,
                    name: p.name,
                    about: p.about,
                    picture: p.picture,
                    members: p.members,
                    peer: r.peer,
                    direction: r.direction,
                    status: r.status,
                    created_at: r.created_at,
                    expires_at: r.expires_at,
                })
            })
            .collect())
    }

    /// Groups whose events this device wants from the relays.
    pub async fn watched(&self) -> Result<Vec<String>> {
        Ok(repo::list(&self.store)
            .await?
            .into_iter()
            .filter(|g| matches!(g.membership.as_str(), MEMBERSHIP_JOINED | MEMBERSHIP_JOINING))
            .map(|g| g.id)
            .collect())
    }

    // ─── Building what goes out ─────────────────────────────────────────────

    /// A direct message of the group protocol, with a copy for my other
    /// devices when they have to know.
    ///
    /// `wake` says whether the one it is for should hear of it at once: an
    /// invitation is for a person, a key is for the app.
    pub(crate) async fn dm(
        &self,
        keys: &Keys,
        to: &PubKey,
        content: String,
        self_copy: bool,
        wake: Wake,
    ) -> Result<Vec<Outbound>> {
        let w = wrap_as(keys, to, &content, self.now(), None, wake)?;
        let mut out = vec![Outbound::PublishToInbox { recipient: to.clone(), event: w.to_peer, hint_relays: self.dm.hints(to).await? }];
        if self_copy {
            if let Some(event) = w.to_self {
                out.push(Outbound::PublishOwn { event });
            }
        }
        Ok(out)
    }

    pub(crate) fn scoped(group_id: &str, event: messenger_core::outbound::WireEvent) -> Outbound {
        Outbound::PublishScoped { scope: Scope::Group { id: group_id.to_string() }, event }
    }

    /// A line in the chat about something that happened to the group.
    pub(crate) async fn system_line(&self, group_id: &str, op: &Op) -> Result<Option<UiEvent>> {
        let (what, target): (&str, Option<&PubKey>) = match &op.body {
            OpBody::Create { .. } => ("group_created", None),
            OpBody::Admit { who } => ("group_admitted", Some(who)),
            OpBody::Join => ("group_joined", None),
            OpBody::Leave => ("group_left", None),
            OpBody::Remove { who } => ("group_removed", Some(who)),
            OpBody::Ban { who } => ("group_banned", Some(who)),
            OpBody::Unban { who } => ("group_unbanned", Some(who)),
            OpBody::SetRole { who, .. } => ("group_role", Some(who)),
            OpBody::SetMuted { who, muted } => (if *muted { "group_muted" } else { "group_unmuted" }, Some(who)),
            OpBody::EditSettings { .. } => ("group_edited", None),
            OpBody::TransferOwnership { to } => ("group_owner", Some(to)),
            OpBody::RotateLink { .. } => ("group_link_changed", None),
            OpBody::Disband => ("group_disbanded", None),
            OpBody::RotateKey => return Ok(None),
        };
        let chat_id = repo::group_chat_id(group_id);
        let id = format!("sys:{}", op.id().0);
        let details = serde_json::json!({
            "actor": op.author.as_hex(),
            "target": target.map(|p| p.as_hex()),
            "role": if let OpBody::SetRole { role, .. } = &op.body { Some(role.as_str()) } else { None },
        });
        let inserted = msgs::insert(
            &self.store,
            &NewMessage {
                id: id.clone(),
                chat_id: chat_id.clone(),
                wire_id: None,
                direction: msgs::DIR_OUT.into(),
                status: msgs::STATUS_SENT.into(),
                content_type: msgs::CT_SYSTEM.into(),
                text: Some(what.into()),
                envelope_json: "{}".into(),
                sender_pubkey: op.author.as_hex().to_string(),
                reply_to_id: None,
                target_id: None,
                created_at: op.created_at,
                is_hidden: false,
                outbox_local_id: None,
                media_json: Some(details.to_string()),
            },
        )
        .await?;
        if !inserted {
            return Ok(None);
        }
        let view = self.dm.message(&id).await?;
        Ok(view.map(|v| UiEvent {
            name: messenger_dm::UI_EVENT_DM_MESSAGE.into(),
            payload: serde_json::json!({ "chat_id": chat_id, "message": v, "historical": true }),
        }))
    }

    /// Make an operation of mine, with a new key when the rules ask for
    /// one, check it, apply it, store it, and seal it for the relay.
    pub(crate) async fn operate(&self, keys: &Keys, group_id: &str, body: OpBody) -> Result<(Op, Outcome)> {
        let me = me_of(keys);
        let row = repo::get(&self.store, group_id).await?.ok_or_else(|| MessengerError::Invalid("group_unknown".into()))?;
        if row.membership != MEMBERSHIP_JOINED && !(row.membership == MEMBERSHIP_JOINING && body == OpBody::Join) {
            return Err(MessengerError::Invalid("group_not_member".into()));
        }
        let log = self.need_log(group_id).await?;
        let before = log.state().clone();
        // Sealed with the key members hold now, so that those who are
        // leaving with this operation can still read that they are.
        let sealing = match &before.current_key {
            Some(id) => self.key(group_id, id).await?,
            None => None,
        }
        .ok_or_else(|| MessengerError::Invalid("group_no_key".into()))?;

        let mut op = log.next(&me, self.now().max(before_time(&log) + 1), body);
        if op.body == OpBody::Join {
            let epoch = before.link_epoch;
            let link = self.link_secret(group_id, epoch).await?.ok_or_else(|| MessengerError::Invalid("group_stale_link".into()))?;
            let mac = link.group_key(group_id, epoch).join_mac(group_id, &me, epoch);
            op = op.with_proof(crate::op::JoinProof { epoch, mac });
        }
        let mut secret: Option<Vec<u8>> = None;
        let mut new_key: Option<GroupKey> = None;
        if before.requires_key(&op.body) {
            if !before.role_of(&me).is_some_and(Role::is_manager) {
                return Err(rejection(Rejection::NotPermitted));
            }
            let key = match &op.body {
                OpBody::RotateLink { link_epoch } => {
                    let link = LinkSecret::generate()?;
                    let key = link.group_key(group_id, *link_epoch);
                    secret = Some(link.as_bytes().to_vec());
                    self.keep_link_secret(group_id, *link_epoch, &link).await?;
                    key
                }
                _ => {
                    let key = GroupKey::generate()?;
                    secret = Some(key.as_bytes().to_vec());
                    key
                }
            };
            op = op.with_key(key.id());
            new_key = Some(key);
        }
        log.check(&op).map_err(rejection)?;
        let mut after = before.clone();
        after.apply(&op).map_err(rejection)?;
        if after.kind == GroupKind::Private && after.members.len() > MAX_PRIVATE_MEMBERS {
            return Err(MessengerError::Invalid("group_full".into()));
        }

        let mut envelopes: Vec<SecretEnvelope> = Vec::new();
        if let Some(bytes) = &secret {
            for holder in after.key_holders() {
                envelopes.push(wire::envelope_for(keys, &holder, bytes)?);
            }
        }
        let signed = wire::sign_op(keys, &op)?;
        let sealed = wire::seal_op(group_id, &sealing, &signed, envelopes, keys)?;
        if let Some(k) = &new_key {
            self.keep_key(group_id, k).await?;
        }
        self.keep_op(&op, &serde_json::to_value(&signed)?).await?;

        let mut outcome = Outcome { publish: vec![Self::scoped(group_id, sealed)], ..Default::default() };
        if let (OpBody::RotateLink { .. }, Some(new)) = (&op.body, &new_key) {
            // Whoever comes by the new link reads the group from its start.
            let mut older: Vec<GroupKey> = self.all_keys(group_id).await?.into_iter().filter(|k| k.id() != new.id()).collect();
            let skip = older.len().saturating_sub(wire::MAX_CHAIN_KEYS);
            older.drain(..skip);
            outcome.publish.push(Self::scoped(group_id, wire::seal_chain(group_id, new, &older, op.created_at, keys)?));
        }
        let membership = match &op.body {
            OpBody::Leave => Some(MEMBERSHIP_LEFT),
            OpBody::Join => Some(MEMBERSHIP_JOINED),
            _ => None,
        };
        self.refresh_row(group_id, &me, membership).await?;
        if let Some(ev) = self.system_line(group_id, &op).await? {
            outcome.events.push(ev);
        }
        outcome.events.push(Self::updated(group_id));
        outcome.resubscribe = matches!(op.body, OpBody::Leave | OpBody::Disband | OpBody::Join);
        Ok((op, outcome))
    }

    /// The welcome for someone who was just admitted.
    pub(crate) async fn welcome(&self, keys: &Keys, group_id: &str, to: &PubKey) -> Result<Vec<Outbound>> {
        let row = repo::get(&self.store, group_id).await?.ok_or_else(|| MessengerError::Invalid("group_unknown".into()))?;
        let log = self.need_log(group_id).await?;
        let s = log.state();
        let held = if s.history_for_new {
            self.all_keys(group_id).await?
        } else {
            match &s.current_key {
                Some(id) => self.key(group_id, id).await?.into_iter().collect(),
                None => vec![],
            }
        };
        let body = wire::Welcome {
            group_id: group_id.to_string(),
            relay: row.relay_url,
            ops: self.signed_ops(group_id).await?,
            keys: wire::encode_keys(&held),
            link: match s.kind {
                GroupKind::Public => self.link_secret(group_id, s.link_epoch).await?.map(|l| wire::encode_secret(l.as_bytes())),
                GroupKind::Private => None,
            },
        };
        self.dm(keys, to, wire::dm_envelope(wire::T_WELCOME, &body)?, false, Wake::Peer).await
    }

    // ─── What the user does ─────────────────────────────────────────────────

    pub async fn create(
        &self,
        keys: &Keys,
        kind: GroupKind,
        name: &str,
        about: &str,
        history_for_new: bool,
        relay: &RelayUrl,
    ) -> Result<(GroupView, Outcome)> {
        let me = me_of(keys);
        let group_id = new_group_id()?;
        let (key, link) = match kind {
            GroupKind::Public => {
                let link = LinkSecret::generate()?;
                (link.group_key(&group_id, 0), Some(link))
            }
            GroupKind::Private => (GroupKey::generate()?, None),
        };
        let op = Op::new(
            &group_id,
            &me,
            vec![],
            self.now(),
            OpBody::Create { kind, name: name.trim().to_string(), about: about.to_string(), picture: String::new(), history_for_new },
        )
        .with_key(key.id());
        OpLog::create(op.clone()).map_err(rejection)?;

        repo::upsert(
            &self.store,
            &GroupRow {
                id: group_id.clone(),
                kind: if kind == GroupKind::Public { "public".into() } else { "private".into() },
                name: name.trim().to_string(),
                about: about.to_string(),
                picture: String::new(),
                relay_url: relay.as_str().to_string(),
                owner: me.as_hex().to_string(),
                membership: MEMBERSHIP_JOINED.into(),
                my_role: Some("owner".into()),
                members: 1,
                link_epoch: 0,
                created_at: 0,
                updated_at: 0,
            },
        )
        .await?;
        match &link {
            Some(l) => self.keep_link_secret(&group_id, 0, l).await?,
            None => {
                self.keep_key(&group_id, &key).await?;
            }
        }
        let signed = wire::sign_op(keys, &op)?;
        // My own envelope: my other devices learn the key from the log.
        let envelopes = match kind {
            GroupKind::Private => vec![wire::envelope_for(keys, &me, key.as_bytes())?],
            GroupKind::Public => vec![],
        };
        let sealed = wire::seal_op(&group_id, &key, &signed, envelopes, keys)?;
        self.keep_op(&op, &serde_json::to_value(&signed)?).await?;

        let mut outcome = Outcome { publish: vec![Self::scoped(&group_id, sealed)], resubscribe: true, ..Default::default() };
        // The welcome to myself is how my other devices get the group.
        outcome.publish.extend(self.welcome(keys, &group_id, &me).await?);
        if let Some(ev) = self.system_line(&group_id, &op).await? {
            outcome.events.push(ev);
        }
        outcome.events.push(Self::updated(&group_id));
        let view = self.get(&group_id, &me).await?.ok_or_else(|| MessengerError::Storage("group vanished".into()))?;
        Ok((view, outcome))
    }

    /// Any operation on the group: remove, ban, role, mute, settings,
    /// transfer, leave, disband, new link.
    pub async fn act(&self, keys: &Keys, group_id: &str, body: OpBody) -> Result<Outcome> {
        // In a public group the key is the link: there is no other.
        if body == OpBody::RotateKey && self.need_log(group_id).await?.state().kind == GroupKind::Public {
            return Err(rejection(Rejection::WrongKind));
        }
        if matches!(body, OpBody::Create { .. } | OpBody::Join | OpBody::Admit { .. }) {
            return Err(MessengerError::Invalid("group_wrong_action".into()));
        }
        Ok(self.operate(keys, group_id, body).await?.1)
    }

    /// Admit someone and send them the welcome.
    pub(crate) async fn admit(&self, keys: &Keys, group_id: &str, who: &PubKey) -> Result<Outcome> {
        let (_, mut outcome) = self.operate(keys, group_id, OpBody::Admit { who: who.clone() }).await?;
        outcome.publish.extend(self.welcome(keys, group_id, who).await?);
        Ok(outcome)
    }

    pub async fn invite(&self, keys: &Keys, group_id: &str, who: &PubKey) -> Result<(InviteView, Outcome)> {
        let me = me_of(keys);
        let row = repo::get(&self.store, group_id).await?.ok_or_else(|| MessengerError::Invalid("group_unknown".into()))?;
        let log = self.need_log(group_id).await?;
        let s = log.state();
        if row.membership != MEMBERSHIP_JOINED || !s.role_of(&me).is_some_and(Role::is_manager) {
            return Err(MessengerError::Invalid("group_not_permitted".into()));
        }
        if who == &me || s.is_member(who) {
            return Err(MessengerError::Invalid("group_already_member".into()));
        }
        if s.is_banned(who) {
            return Err(MessengerError::Invalid("group_target_banned".into()));
        }
        let now = self.now();
        let mut id = [0u8; 16];
        getrandom::fill(&mut id).map_err(|e| MessengerError::Crypto(e.to_string()))?;
        let body = wire::Invite {
            invite_id: hex::encode(id),
            group_id: group_id.to_string(),
            name: s.name.clone(),
            about: s.about.clone(),
            picture: s.picture.clone(),
            relay: row.relay_url.clone(),
            members: s.members.len() as u32,
            created_at: now,
            expires_at: now + wire::INVITE_TTL_SECS,
        };
        repo::insert_invite(
            &self.store,
            &repo::InviteRow {
                invite_id: body.invite_id.clone(),
                group_id: group_id.to_string(),
                direction: "out".into(),
                peer: who.as_hex().to_string(),
                status: "sent".into(),
                payload_json: serde_json::to_string(&body)?,
                created_at: now,
                expires_at: body.expires_at,
                updated_at: 0,
            },
        )
        .await?;
        let publish = self.dm(keys, who, wire::dm_envelope(wire::T_INVITE, &body)?, false, Wake::Peer).await?;
        let view = self.invites("out").await?.into_iter().find(|i| i.invite_id == body.invite_id);
        Ok((
            view.ok_or_else(|| MessengerError::Storage("invite vanished".into()))?,
            Outcome { publish, events: vec![Self::updated(group_id)], ..Default::default() },
        ))
    }

    /// Accept or decline an invitation I received.
    pub async fn answer_invite(&self, keys: &Keys, invite_id: &str, accept: bool) -> Result<Outcome> {
        let inv = repo::invite(&self.store, invite_id)
            .await?
            .filter(|i| i.direction == "in")
            .ok_or_else(|| MessengerError::Invalid("group_invite_unknown".into()))?;
        if inv.status != "received" {
            return Err(MessengerError::Invalid("group_invite_answered".into()));
        }
        if inv.expires_at <= self.now() {
            repo::set_invite_status(&self.store, invite_id, "expired").await?;
            return Err(MessengerError::Invalid("group_invite_expired".into()));
        }
        let inviter = PubKey::parse(&inv.peer).ok_or_else(|| MessengerError::Storage("bad inviter".into()))?;
        repo::set_invite_status(&self.store, invite_id, if accept { "accepted" } else { "declined" }).await?;
        let reply = wire::InviteReply { invite_id: invite_id.to_string(), group_id: inv.group_id.clone(), accept };
        let publish = self.dm(keys, &inviter, wire::dm_envelope(wire::T_INVITE_REPLY, &reply)?, true, Wake::Nobody).await?;
        Ok(Outcome { publish, events: vec![Self::updated(&inv.group_id)], ..Default::default() })
    }

    /// Open a link. Public: the group appears at once and joins as soon as
    /// its log is here. Private: the managers named in the link are asked.
    pub async fn open_link(&self, keys: &Keys, link: &str, note: &str) -> Result<(GroupView, Outcome)> {
        let me = me_of(keys);
        let link = GroupLink::parse(link)?;
        if let Some(existing) = repo::get(&self.store, &link.group_id).await? {
            if existing.membership == MEMBERSHIP_JOINED {
                let view = self.view_of(existing, &me).await?;
                return Ok((view, Outcome::default()));
            }
            // A ban here is only what I knew when I stopped listening: it
            // may be lifted since. The group's log answers, not this row.
        }
        let mut row = GroupRow {
            id: link.group_id.clone(),
            kind: if link.kind == GroupKind::Public { "public".into() } else { "private".into() },
            name: if link.name.trim().is_empty() { "…".into() } else { link.name.clone() },
            about: String::new(),
            picture: String::new(),
            relay_url: link.relay.as_str().to_string(),
            owner: link.owner.as_hex().to_string(),
            membership: MEMBERSHIP_REQUESTED.into(),
            my_role: None,
            members: 0,
            link_epoch: link.link_epoch as i64,
            created_at: 0,
            updated_at: 0,
        };
        let mut outcome = Outcome::default();
        match (&link.kind, &link.secret) {
            (GroupKind::Public, Some(secret)) => {
                row.membership = MEMBERSHIP_JOINING.into();
                repo::upsert(&self.store, &row).await?;
                self.forget_log(&link.group_id).await;
                self.keep_link_secret(&link.group_id, link.link_epoch, secret).await?;
                outcome.resubscribe = true;
                // The log may be here already (a second attempt).
                outcome.merge(self.try_join(keys, &link.group_id).await?);
            }
            _ => {
                repo::upsert(&self.store, &row).await?;
                let mut ask: Vec<PubKey> = vec![link.owner.clone()];
                ask.extend(link.managers.iter().filter(|m| *m != &link.owner).cloned());
                let body = wire::JoinRequest { group_id: link.group_id.clone(), note: note.chars().take(300).collect(), created_at: self.now() };
                repo::put_request(
                    &self.store,
                    &repo::RequestRow {
                        group_id: link.group_id.clone(),
                        requester: me.as_hex().to_string(),
                        direction: "out".into(),
                        status: "pending".into(),
                        payload_json: serde_json::json!({ "asked": ask.iter().map(|p| p.as_hex()).collect::<Vec<_>>() }).to_string(),
                        created_at: self.now(),
                        updated_at: 0,
                    },
                )
                .await?;
                let content = wire::dm_envelope(wire::T_JOIN_REQUEST, &body)?;
                for (i, manager) in ask.iter().enumerate() {
                    outcome.publish.extend(self.dm(keys, manager, content.clone(), i == 0, Wake::Peer).await?);
                }
            }
        }
        outcome.events.push(Self::updated(&link.group_id));
        let view = self.get(&link.group_id, &me).await?.ok_or_else(|| MessengerError::Storage("group vanished".into()))?;
        Ok((view, outcome))
    }

    /// Public group whose link was opened: join once the log is here.
    pub(crate) async fn try_join(&self, keys: &Keys, group_id: &str) -> Result<Outcome> {
        let me = me_of(keys);
        let Some(row) = repo::get(&self.store, group_id).await? else { return Ok(Outcome::default()) };
        if row.membership != MEMBERSHIP_JOINING {
            return Ok(Outcome::default());
        }
        let Some(log) = self.log(group_id).await? else { return Ok(Outcome::default()) };
        let s = log.state();
        if s.kind != GroupKind::Public || s.disbanded {
            self.refresh_row(group_id, &me, None).await?;
            return Ok(Outcome { events: vec![Self::updated(group_id)], ..Default::default() });
        }
        if s.is_banned(&me) {
            // Perhaps not any more: the history that is coming may lift it.
            // `maintain` gives the answer once it is here.
            return Ok(Outcome::default());
        }
        if s.is_member(&me) {
            self.refresh_row(group_id, &me, Some(MEMBERSHIP_JOINED)).await?;
            return Ok(Outcome { events: vec![Self::updated(group_id)], ..Default::default() });
        }
        if self.link_secret(group_id, s.link_epoch).await?.is_none() {
            // The group has a newer link than the one I hold.
            self.refresh_row(group_id, &me, Some(MEMBERSHIP_STALE)).await?;
            return Ok(Outcome { events: vec![Self::updated(group_id)], resubscribe: true, ..Default::default() });
        }
        Ok(self.operate(keys, group_id, OpBody::Join).await?.1)
    }

    pub async fn approve_request(&self, keys: &Keys, group_id: &str, requester: &PubKey) -> Result<Outcome> {
        let req = repo::request(&self.store, group_id, requester.as_hex(), "in")
            .await?
            .filter(|r| r.status == "pending")
            .ok_or_else(|| MessengerError::Invalid("group_request_unknown".into()))?;
        let outcome = self.admit(keys, group_id, requester).await?;
        repo::put_request(&self.store, &repo::RequestRow { status: "approved".into(), ..req }).await?;
        Ok(outcome)
    }

    pub async fn reject_request(&self, keys: &Keys, group_id: &str, requester: &PubKey) -> Result<Outcome> {
        let me = me_of(keys);
        let log = self.need_log(group_id).await?;
        if !log.state().role_of(&me).is_some_and(Role::is_manager) {
            return Err(MessengerError::Invalid("group_not_permitted".into()));
        }
        let req = repo::request(&self.store, group_id, requester.as_hex(), "in")
            .await?
            .filter(|r| r.status == "pending")
            .ok_or_else(|| MessengerError::Invalid("group_request_unknown".into()))?;
        repo::put_request(&self.store, &repo::RequestRow { status: "rejected".into(), ..req }).await?;
        let body = wire::Rejected { group_id: group_id.to_string() };
        let publish = self.dm(keys, requester, wire::dm_envelope(wire::T_REJECTED, &body)?, false, Wake::Nobody).await?;
        Ok(Outcome { publish, events: vec![Self::updated(group_id)], ..Default::default() })
    }

    /// Remove a group from this device (after leaving it, or a request
    /// that was never answered).
    pub async fn forget(&self, group_id: &str) -> Result<Outcome> {
        if repo::get(&self.store, group_id).await?.is_some_and(|g| g.membership == MEMBERSHIP_JOINED) {
            return Err(MessengerError::Invalid("group_leave_first".into()));
        }
        for id in repo::key_ids(&self.store, group_id).await? {
            let _ = self.secrets.delete(&key_ref(group_id, &KeyId(id))).await;
        }
        repo::delete(&self.store, group_id).await?;
        messenger_store::events_raw::forget_tagged(&self.store, wire::KIND_GROUP_EVENT, group_id).await?;
        self.forget_log(group_id).await;
        Ok(Outcome { events: vec![Self::updated(group_id)], resubscribe: true, ..Default::default() })
    }

    /// What a manager's device does by itself when the key needs care.
    /// Also, the history of a group I am joining is here: if its log
    /// still bans me, that is the answer.
    pub async fn maintain(&self, keys: &Keys, group_id: &str) -> Result<Outcome> {
        let me = me_of(keys);
        let Some(row) = repo::get(&self.store, group_id).await? else { return Ok(Outcome::default()) };
        if row.membership == MEMBERSHIP_JOINING {
            if !self.log(group_id).await?.is_some_and(|l| l.state().is_banned(&me)) {
                return Ok(Outcome::default());
            }
            self.refresh_row(group_id, &me, Some(MEMBERSHIP_BANNED)).await?;
            return Ok(Outcome { events: vec![Self::updated(group_id)], resubscribe: true, ..Default::default() });
        }
        if row.membership != MEMBERSHIP_JOINED {
            return Ok(Outcome::default());
        }
        let Some(log) = self.log(group_id).await? else { return Ok(Outcome::default()) };
        if !log.state().role_of(&me).is_some_and(Role::is_manager) {
            return Ok(Outcome::default());
        }
        match log.key_status().clone() {
            KeyStatus::Good => Ok(Outcome::default()),
            KeyStatus::Rotate => Ok(self.operate(keys, group_id, OpBody::RotateKey).await?.1),
            KeyStatus::Deliver(list) => {
                let held = if log.state().history_for_new {
                    self.all_keys(group_id).await?
                } else {
                    match &log.state().current_key {
                        Some(id) => self.key(group_id, id).await?.into_iter().collect(),
                        None => vec![],
                    }
                };
                let body = wire::KeyDelivery { group_id: group_id.to_string(), keys: wire::encode_keys(&held) };
                let content = wire::dm_envelope(wire::T_KEYS, &body)?;
                let mut outcome = Outcome::default();
                for who in list {
                    outcome.publish.extend(self.dm(keys, &who, content.clone(), false, Wake::Nobody).await?);
                }
                Ok(outcome)
            }
        }
    }

    // ─── Messages ───────────────────────────────────────────────────────────

    /// Store and seal a message of mine. Returns the stored message, what
    /// to publish, and the id whose publication the message status follows.
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_message(
        &self,
        keys: &Keys,
        group_id: &str,
        envelope: Envelope,
        content_type: &str,
        text: Option<String>,
        reply_to: Option<&str>,
        media_json: Option<String>,
    ) -> Result<(MessageView, Outbound)> {
        let me = me_of(keys);
        let row = repo::get(&self.store, group_id).await?.ok_or_else(|| MessengerError::Invalid("group_unknown".into()))?;
        if row.membership != MEMBERSHIP_JOINED {
            return Err(MessengerError::Invalid("group_not_member".into()));
        }
        let log = self.need_log(group_id).await?;
        let s = log.state();
        if !s.can_post(&me) {
            return Err(MessengerError::Invalid(if s.member(&me).is_some_and(|m| m.muted) { "group_muted".into() } else { "group_not_member".into() }));
        }
        let key = match &s.current_key {
            Some(id) => self.key(group_id, id).await?,
            None => None,
        }
        .ok_or_else(|| MessengerError::Invalid("group_no_key".into()))?;
        let chat_id = repo::group_chat_id(group_id);
        if let Some(id) = reply_to {
            match msgs::get(&self.store, id).await? {
                Some(t) if t.chat_id == chat_id && !t.is_hidden => {}
                _ => return Err(MessengerError::Invalid("reply target is not in this chat".into())),
            }
        }
        let now = self.now();
        let created_at = match msgs::last_created_at(&self.store, &chat_id).await? {
            Some(last) if last >= now => last + 1,
            _ => now,
        };
        let content = envelope.encode();
        let signed = wire::sign_message(keys, group_id, &content, created_at, reply_to)?;
        let grace = self.grace_key(group_id, s).await?;
        let sealed = wire::seal_message(group_id, &key, grace.as_ref(), &signed, keys)?;
        let id = signed.id.to_hex();
        let hidden = matches!(content_type, msgs::CT_EDIT | msgs::CT_DELETE);
        msgs::insert(
            &self.store,
            &NewMessage {
                id: id.clone(),
                chat_id: chat_id.clone(),
                wire_id: Some(sealed.id.as_hex().to_string()),
                direction: msgs::DIR_OUT.into(),
                status: msgs::STATUS_QUEUED.into(),
                content_type: content_type.into(),
                text: text.clone(),
                envelope_json: content,
                sender_pubkey: me.as_hex().to_string(),
                reply_to_id: reply_to.map(String::from),
                target_id: envelope.str_field("target").map(String::from),
                created_at,
                is_hidden: hidden,
                outbox_local_id: None,
                media_json,
            },
        )
        .await?;
        if !hidden {
            let line = match (&text, content_type) {
                (Some(t), msgs::CT_MEDIA) => format!("📎 {}", messenger_dm::view::preview(t)),
                (Some(t), _) => messenger_dm::view::preview(t),
                (None, _) => format!("📎 {}", envelope.str_field("name").unwrap_or("file")),
            };
            chats::touch(&self.store, &chat_id, created_at, Some(&line), false).await?;
        }
        let view = self.dm.message(&id).await?.ok_or_else(|| MessengerError::Storage("message vanished".into()))?;
        Ok((view, Self::scoped(group_id, sealed)))
    }

    pub async fn prepare_text(&self, keys: &Keys, group_id: &str, text: &str, reply_to: Option<&str>) -> Result<(MessageView, Outbound)> {
        let text = text.trim();
        if text.is_empty() {
            return Err(MessengerError::Invalid("message is empty".into()));
        }
        if text.len() > messenger_dm::service::MAX_TEXT_BYTES {
            return Err(MessengerError::Invalid("message is too long".into()));
        }
        self.prepare_message(keys, group_id, Envelope::text(text), msgs::CT_TEXT, Some(text.to_string()), reply_to, None).await
    }

    fn group_of(chat_id: &str) -> Result<&str> {
        chat_id.strip_prefix("group:").ok_or_else(|| MessengerError::Invalid("not a group message".into()))
    }

    /// Change the text of my own message.
    pub async fn prepare_edit(&self, keys: &Keys, message_id: &str, text: &str) -> Result<(MessageView, String, Outbound)> {
        let me = me_of(keys);
        let text = text.trim();
        if text.is_empty() || text.len() > messenger_dm::service::MAX_TEXT_BYTES {
            return Err(MessengerError::Invalid("message is empty or too long".into()));
        }
        let row = msgs::get(&self.store, message_id).await?.filter(|r| !r.is_hidden).ok_or_else(|| MessengerError::Invalid("unknown message".into()))?;
        if row.sender_pubkey != me.as_hex() || row.deleted_at.is_some() || row.content_type != msgs::CT_TEXT {
            return Err(MessengerError::Invalid("only your own text messages can be edited".into()));
        }
        let group_id = Self::group_of(&row.chat_id)?.to_string();
        let (hidden, out) = self.prepare_message(keys, &group_id, Envelope::edit(message_id, text), msgs::CT_EDIT, None, None, None).await?;
        msgs::set_text(&self.store, message_id, text, hidden.created_at).await?;
        chats::recompute_last(&self.store, &row.chat_id).await?;
        let view = self.dm.message(message_id).await?.ok_or_else(|| MessengerError::Storage("message vanished".into()))?;
        Ok((view, hidden.id, out))
    }

    /// Remove a message for everyone: my own, or as a moderator.
    pub async fn prepare_delete(&self, keys: &Keys, message_id: &str) -> Result<(String, String, Outbound)> {
        let me = me_of(keys);
        let row = msgs::get(&self.store, message_id).await?.filter(|r| !r.is_hidden).ok_or_else(|| MessengerError::Invalid("unknown message".into()))?;
        if row.deleted_at.is_some() || row.content_type == msgs::CT_SYSTEM {
            return Err(MessengerError::Invalid("nothing to delete".into()));
        }
        let group_id = Self::group_of(&row.chat_id)?.to_string();
        let author = PubKey::parse(&row.sender_pubkey).ok_or_else(|| MessengerError::Storage("bad author".into()))?;
        if !self.need_log(&group_id).await?.state().can_delete_message(&me, &author) {
            return Err(MessengerError::Invalid("group_not_permitted".into()));
        }
        let (hidden, out) = self.prepare_message(keys, &group_id, Envelope::delete(message_id), msgs::CT_DELETE, None, None, None).await?;
        msgs::mark_deleted(&self.store, message_id, hidden.created_at).await?;
        chats::recompute_last(&self.store, &row.chat_id).await?;
        Ok((row.chat_id, hidden.id, out))
    }
}

/// Newest `created_at` in the log: my next operation is dated after it,
/// so people see events in the order they happened.
fn before_time(log: &OpLog) -> i64 {
    log.ordered().map(|o| o.created_at).max().unwrap_or(0)
}
