// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Groups in the runtime: what the host calls, and what has to happen
//! around the group service: publishing, listening to the right groups,
//! fetching their history, and looking after the keys.

use crate::relays::RelayService;
use crate::MessengerRuntime;
use messenger_core::traits::{RelayState, SystemClock, UiEvent};
use messenger_core::{Clock, MessengerError, Outbound, PubKey, RelayUrl, Result, Scope, SubId, SyncItem, Timestamp, Transport};
use messenger_dm::{ChatView, MessageView};
use messenger_groups::{GroupKind, GroupService, GroupView, InviteView, OpBody, Outcome, Signal};
use messenger_ingress::{filters, Outbox};
use messenger_store::{cursors, events_raw, groups as repo, settings, Store};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, Mutex};

const SUB_GROUP_PROFILES: &str = "group-profiles";
const WAIT_FOR_RELAY: Duration = Duration::from_secs(60);
/// After the history was asked for: time for it to be read before the
/// keys are looked after.
const SETTLE_AFTER_SYNC: Duration = Duration::from_secs(4);
/// How often the store is asked whether the events of a window of the
/// past have all arrived, and for how long at most: a window that brings
/// nothing answers in half a second, a full one when its events are in.
const WINDOW_POLL: Duration = Duration::from_millis(250);
const WINDOW_SETTLE: Duration = Duration::from_secs(20);
/// Windows in a row that brought nothing before the group's past counts
/// as fetched to the end.
const WINDOW_MISSES_ENOUGH: u32 = 2;
/// At most this many events are asked for in one window of the past.
pub const HISTORY_WINDOW_MAX: u32 = 1_000;
pub(crate) const MAX_PROFILE_AUTHORS: usize = 500;
const KIND_GROUP_EVENT: u16 = 9;

/// Where «load earlier» of a group stands (`msg_settings`): the oldest
/// bound asked for and how many windows in a row brought nothing, as
/// `<before> <misses>`.
fn oldest_fetched_key(group_id: &str) -> String {
    format!("group.{group_id}.oldest_fetched")
}

async fn oldest_fetched(store: &Store, group_id: &str) -> Option<(i64, u32)> {
    let value = settings::get(store, &oldest_fetched_key(group_id)).await.ok()??;
    let mut parts = value.split(' ');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

/// A window of the past was answered: a full one moves the bound down and
/// starts the count of misses again, an empty one at the bound counts.
async fn note_window(store: &Store, group_id: &str, before: i64, received: u32) {
    let (bound, misses) = oldest_fetched(store, group_id).await.unwrap_or((i64::MAX, 0));
    let misses = if received > 0 { 0 } else { misses + 1 };
    let _ = settings::set(store, &oldest_fetched_key(group_id), &format!("{} {misses}", before.min(bound))).await;
}

/// Ask the relays for everything of `ids` since `since` that this device
/// lacks: every local id of the window goes along, so that Negentropy
/// sends back only what is missing.
pub(crate) async fn request_since(store: &Store, transport: &dyn Transport, ids: &[String], since: Option<i64>) -> Result<()> {
    let local = events_raw::items_since(store, KIND_GROUP_EVENT, since.unwrap_or(0))
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(id, at)| SyncItem { id, created_at: Timestamp(at) })
        .collect();
    let filter = filters::group_events(ids, since.map(Timestamp));
    transport.send(Outbound::Sync { scope: Scope::Own, filter, local }).await?;
    Ok(())
}

/// Ask the relays for one window of a group's past: the newest `limit`
/// events up to `before` that this device lacks. The local side of the
/// comparison is this group's events of the same window and nothing
/// else, so the relays see the same set they hold. How many of them this
/// device already had.
pub(crate) async fn request_window(store: &Store, transport: &dyn Transport, group_id: &str, before: i64, limit: u32) -> Result<usize> {
    let local = events_raw::items_tagged_until(store, KIND_GROUP_EVENT, group_id, before).await?;
    let known = local.len();
    let local = local.into_iter().map(|(id, at)| SyncItem { id, created_at: Timestamp(at) }).collect();
    let filter = filters::group_events_until(&[group_id.to_string()], Timestamp(before), limit as usize);
    transport.send(Outbound::Sync { scope: Scope::Own, filter, local }).await?;
    Ok(known)
}

/// What the relays sent arrives on the stream and reaches the store once
/// it is applied, so the window is counted until the count stands still.
/// How many events the window brought.
async fn settle_window(store: &Store, group_id: &str, before: i64, known: usize) -> u32 {
    let deadline = tokio::time::Instant::now() + WINDOW_SETTLE;
    let mut last = known;
    let mut still = 0;
    loop {
        tokio::time::sleep(WINDOW_POLL).await;
        let now = events_raw::items_tagged_until(store, KIND_GROUP_EVENT, group_id, before).await.map(|v| v.len()).unwrap_or(last);
        if now == last {
            still += 1;
            if still >= 2 {
                break;
            }
        } else {
            still = 0;
            last = now;
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
    }
    last.saturating_sub(known) as u32
}

/// Everything the groups need from the runtime, cloneable into tasks.
#[derive(Clone)]
pub struct GroupsDriver {
    pub(crate) groups: GroupService,
    pub(crate) store: Store,
    pub(crate) relays: Arc<RelayService>,
    pub(crate) outbox: Outbox,
    pub(crate) ui: broadcast::Sender<UiEvent>,
    /// Groups whose history was asked for in this session.
    synced: Arc<Mutex<HashSet<String>>>,
}

impl GroupsDriver {
    pub(crate) fn new(
        groups: GroupService,

        store: Store,
        relays: Arc<RelayService>,
        outbox: Outbox,
        ui: broadcast::Sender<UiEvent>,
    ) -> Self {
        Self { groups, store, relays, outbox, ui, synced: Arc::default() }
    }

    fn me(&self) -> Option<PubKey> {
        self.groups.signer().and_then(|k| PubKey::parse(&k.public_key().to_hex()))
    }

    /// Carry out what an action led to.
    pub(crate) async fn apply(&self, outcome: Outcome) -> Result<()> {
        let publish = !outcome.publish.is_empty();
        for out in outcome.publish {
            self.outbox.enqueue(out).await?;
        }
        if publish {
            self.outbox.kick();
        }
        for ev in outcome.events {
            let _ = self.ui.send(ev);
        }
        for notice in outcome.notify {
            let _ = self.ui.send(UiEvent { name: "notify".into(), payload: serde_json::to_value(notice).unwrap_or_default() });
        }
        for n in outcome.notes {
            eprintln!("messenger groups: {n}");
        }
        if outcome.resubscribe {
            self.resubscribe().await;
        }
        for g in outcome.maintain {
            self.maintain_later(g);
        }
        Ok(())
    }

    /// Listen to the groups I am in; ask for the history of those that
    /// are new to this session.
    pub(crate) fn resubscribe(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(self.resubscribe_inner())
    }

    async fn resubscribe_inner(&self) {
        if self.groups.signer().is_none() {
            return;
        }
        let ids = match self.groups.watched().await {
            Ok(ids) => ids,
            Err(e) => {
                eprintln!("messenger groups: {e}");
                return;
            }
        };
        let pool = self.relays.pool().await;
        if ids.is_empty() {
            let _ = pool.send(Outbound::Unsubscribe { id: SubId(filters::SUB_GROUPS.into()) }).await;
            return;
        }
        let since = Timestamp(SystemClock.now().secs() - filters::GROUP_LIVE_MARGIN_SECS);
        if let Err(e) = pool
            .send(Outbound::Subscribe {
                id: SubId(filters::SUB_GROUPS.into()),
                filter: filters::group_events(&ids, Some(since)),
                scope: Scope::Own,
            })
            .await
        {
            eprintln!("messenger groups: subscription failed: {e}");
        }
        self.follow_members().await;

        let fresh: Vec<String> = {
            let mut synced = self.synced.lock().await;
            // A group I stopped listening to has missed things: when I listen
            // again (a lifted ban, a second join), its history is asked anew.
            synced.retain(|g| ids.contains(g));
            ids.into_iter().filter(|g| synced.insert(g.clone())).collect()
        };
        if !fresh.is_empty() {
            let this = self.clone();
            tokio::spawn(async move { this.history(fresh, None).await });
        }
    }

    /// Profiles of the people in my groups, so that they have names.
    async fn follow_members(&self) {
        let Some(me) = self.me() else { return };
        let mut authors: Vec<PubKey> = Vec::new();
        for g in self.groups.list(&me).await.unwrap_or_default() {
            for m in g.members {
                if let Some(pk) = PubKey::parse(&m.pubkey) {
                    if !authors.contains(&pk) && authors.len() < MAX_PROFILE_AUTHORS {
                        authors.push(pk);
                    }
                }
            }
        }
        if authors.is_empty() {
            return;
        }
        let pool = self.relays.pool().await;
        let _ = pool
            .send(Outbound::Subscribe { id: SubId(SUB_GROUP_PROFILES.into()), filter: filters::profiles(&authors), scope: Scope::Own })
            .await;
    }

    async fn wait_for_relay(&self) -> bool {
        let pool = self.relays.pool().await;
        let deadline = tokio::time::Instant::now() + WAIT_FOR_RELAY;
        loop {
            let connected = pool.status().await.relays.iter().any(|r| r.state == RelayState::Connected);
            if connected && !pool.is_silent() {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Fetch what the relays hold for these groups, then look after the
    /// keys of the groups I manage.
    async fn history(&self, ids: Vec<String>, since: Option<i64>) {
        if !self.wait_for_relay().await {
            // Not asked: the next session (or reconnect) tries again.
            let mut synced = self.synced.lock().await;
            for g in &ids {
                synced.remove(g);
            }
            return;
        }
        let started = SystemClock.now().secs();
        let pool = self.relays.pool().await;
        match request_since(&self.store, pool.as_ref(), &ids, since).await {
            Ok(()) => {
                if since.is_some() {
                    let _ = cursors::advance(&self.store, filters::CURSOR_GROUPS, cursors::ANY_RELAY, started).await;
                }
            }
            Err(e) => {
                eprintln!("messenger groups: history not fetched: {e}");
                let mut synced = self.synced.lock().await;
                for g in &ids {
                    synced.remove(g);
                }
                return;
            }
        }
        tokio::time::sleep(SETTLE_AFTER_SYNC).await;
        for g in ids {
            self.maintain_now(&g).await;
        }
    }

    /// «Load earlier»: one window of the group's past, the newest `limit`
    /// events up to `before`, from the relays. How many of them this
    /// device did not have. A bound the relays twice had nothing under is
    /// not asked again (the UI stops offering it after the same two
    /// answers); a new join forgets that (`group_forget`).
    pub(crate) async fn history_before(&self, group_id: &str, before: i64, limit: u32) -> Result<u32> {
        if self.groups.signer().is_none() {
            return Err(MessengerError::NotLoggedIn);
        }
        if repo::get(&self.store, group_id).await?.is_none() {
            return Err(MessengerError::Invalid("group_unknown".into()));
        }
        if let Some((bound, misses)) = oldest_fetched(&self.store, group_id).await {
            if misses >= WINDOW_MISSES_ENOUGH && before >= bound {
                return Ok(0);
            }
        }
        if !self.wait_for_relay().await {
            return Err(MessengerError::Transport("no relay connected".into()));
        }
        let pool = self.relays.pool().await;
        let known = request_window(&self.store, pool.as_ref(), group_id, before, limit).await?;
        let received = settle_window(&self.store, group_id, before, known).await;
        note_window(&self.store, group_id, before, received).await;
        self.maintain_now(group_id).await;
        Ok(received)
    }

    /// A session began: listen, and catch up on what was missed.
    pub(crate) async fn session_started(&self) {
        self.synced.lock().await.clear();
        let ids = self.groups.watched().await.unwrap_or_default();
        let cursor = cursors::get(&self.store, filters::CURSOR_GROUPS, cursors::ANY_RELAY).await.ok().flatten();
        if let (Some(cursor), false) = (cursor, ids.is_empty()) {
            // Known groups: only what is newer than the last catch-up.
            // Groups that joined since have no history here and get all of it.
            let mut known = Vec::new();
            for g in &ids {
                let has_log = !repo::ops(&self.store, g).await.unwrap_or_default().is_empty();
                if has_log {
                    known.push(g.clone());
                }
            }
            {
                let mut synced = self.synced.lock().await;
                synced.extend(known.iter().cloned());
            }
            if !known.is_empty() {
                let this = self.clone();
                let since = (cursor - filters::GROUP_LIVE_MARGIN_SECS).max(0);
                tokio::spawn(async move { this.history(known, Some(since)).await });
            }
        } else if !ids.is_empty() {
            // First catch-up ever: everything, and the cursor starts here.
            {
                let mut synced = self.synced.lock().await;
                synced.extend(ids.iter().cloned());
            }
            let this = self.clone();
            tokio::spawn(async move { this.history(ids, Some(0)).await });
        } else {
            let _ = cursors::advance(&self.store, filters::CURSOR_GROUPS, cursors::ANY_RELAY, SystemClock.now().secs()).await;
        }
        self.resubscribe().await;
    }

    async fn maintain_now(&self, group_id: &str) {
        let Some(keys) = self.groups.signer() else { return };
        let lock = self.groups.lock_of(group_id).await;
        let outcome = {
            let _guard = lock.lock().await;
            self.groups.maintain(&keys, group_id).await
        };
        match outcome {
            Ok(o) => {
                if let Err(e) = self.apply(o).await {
                    eprintln!("messenger groups: {e}");
                }
            }
            Err(e) => eprintln!("messenger groups: key care failed: {e}"),
        }
    }

    /// Before something of mine is sealed with the key of `group_id`: a
    /// key someone who left still knows is replaced first when I manage the
    /// group; otherwise the sealing refuses (`group_key_stale`). The caller
    /// holds the group's lock and seals under it.
    pub(crate) async fn renew_stale_key(&self, keys: &nostr::key::Keys, group_id: &str) -> Result<()> {
        let outcome = self.groups.renew_stale_key(keys, group_id).await?;
        self.apply(outcome).await
    }

    /// Several managers may be online; the owner goes first and the
    /// others a little later, when there is most likely nothing left to do.
    pub(crate) fn maintain_later(&self, group_id: String) {
        let this = self.clone();
        tokio::spawn(async move {
            let owner = match (this.me(), repo::get(&this.store, &group_id).await) {
                (Some(me), Ok(Some(row))) => row.owner == me.as_hex(),
                _ => false,
            };
            let mut jitter = [0u8; 2];
            let _ = getrandom::fill(&mut jitter);
            let wait = if owner { 300 } else { 2_000 + u64::from(u16::from_le_bytes(jitter)) % 6_000 };
            tokio::time::sleep(Duration::from_millis(wait)).await;
            this.maintain_now(&group_id).await;
        });
    }

    /// What the handlers ask for.
    pub(crate) async fn run(self, mut signals: mpsc::UnboundedReceiver<Signal>) {
        while let Some(s) = signals.recv().await {
            match s {
                Signal::Resubscribe => self.resubscribe().await,
                Signal::Maintain(g) => self.maintain_later(g),
                Signal::Note(n) => eprintln!("messenger groups: {n}"),
            }
        }
    }
}

impl MessengerRuntime {
    pub fn groups(&self) -> &GroupService {
        &self.group_driver.groups
    }

    async fn group_relay(&self) -> Result<RelayUrl> {
        self.relays
            .list()
            .await?
            .into_iter()
            .filter(|r| r.enabled && r.write)
            .find_map(|r| RelayUrl::parse(&r.url))
            .ok_or_else(|| MessengerError::Invalid("group_no_relay".into()))
    }

    // ─── Chats of every kind ────────────────────────────────────────────────

    async fn with_group(&self, mut chat: ChatView) -> Result<ChatView> {
        if let Some(id) = chat.id.strip_prefix("group:") {
            if let Some(g) = repo::get(&self.store, id).await? {
                chat.title = g.name;
                chat.picture = Some(g.picture).filter(|p| !p.is_empty());
                chat.mode = "group".into();
                chat.can_send = g.membership == "joined";
            }
        }
        Ok(chat)
    }

    /// Chats and groups in one list.
    pub async fn chats(&self, include_archived: bool) -> Result<Vec<ChatView>> {
        let mut out = Vec::new();
        for c in self.dm.list_chats(include_archived).await? {
            out.push(self.with_group(c).await?);
        }
        Ok(out)
    }

    pub async fn chat(&self, chat_id: &str) -> Result<Option<ChatView>> {
        match self.dm.chat(chat_id).await? {
            Some(c) => Ok(Some(self.with_group(c).await?)),
            None => Ok(None),
        }
    }

    // ─── Groups ─────────────────────────────────────────────────────────────

    pub async fn group_list(&self) -> Result<Vec<GroupView>> {
        let me = self.session_pubkey().await.ok_or(MessengerError::NotLoggedIn)?;
        self.groups().list(&me).await
    }

    pub async fn group_get(&self, group_id: &str) -> Result<GroupView> {
        let me = self.session_pubkey().await.ok_or(MessengerError::NotLoggedIn)?;
        self.groups().get(group_id, &me).await?.ok_or_else(|| MessengerError::Invalid("group_unknown".into()))
    }

    pub async fn group_create(&self, kind: GroupKind, name: &str, about: &str, history_for_new: bool) -> Result<GroupView> {
        let keys = self.session_keys().await?;
        let relay = self.group_relay().await?;
        let (view, outcome) = self.groups().create(&keys, kind, name, about, history_for_new, &relay).await?;
        self.group_driver.apply(outcome).await?;
        Ok(view)
    }

    pub async fn group_invite(&self, group_id: &str, who: &str) -> Result<InviteView> {
        let keys = self.session_keys().await?;
        let who = messenger_contacts::book::parse_key(who)?;
        let lock = self.groups().lock_of(group_id).await;
        let _guard = lock.lock().await;
        let (view, outcome) = self.groups().invite(&keys, group_id, &who).await?;
        self.group_driver.apply(outcome).await?;
        Ok(view)
    }

    /// `in`: invitations for me; `out`: those I sent.
    pub async fn group_invites(&self, direction: &str) -> Result<Vec<InviteView>> {
        self.groups().invites(direction).await
    }

    pub async fn group_answer_invite(&self, invite_id: &str, accept: bool) -> Result<()> {
        let keys = self.session_keys().await?;
        let outcome = self.groups().answer_invite(&keys, invite_id, accept).await?;
        self.group_driver.apply(outcome).await
    }

    /// Open a `veydan://group/…` link: join (public) or ask (private).
    pub async fn group_open_link(&self, link: &str, note: &str) -> Result<GroupView> {
        let keys = self.session_keys().await?;
        let (view, outcome) = self.groups().open_link(&keys, link.trim(), note).await?;
        self.group_driver.apply(outcome).await?;
        self.group_get(&view.id).await
    }

    pub async fn group_answer_request(&self, group_id: &str, requester: &str, approve: bool) -> Result<GroupView> {
        let keys = self.session_keys().await?;
        let who = messenger_contacts::book::parse_key(requester)?;
        {
            let lock = self.groups().lock_of(group_id).await;
            let _guard = lock.lock().await;
            let outcome = if approve {
                self.groups().approve_request(&keys, group_id, &who).await?
            } else {
                self.groups().reject_request(&keys, group_id, &who).await?
            };
            self.group_driver.apply(outcome).await?;
        }
        self.group_get(group_id).await
    }

    /// Remove, ban, role, mute, settings, transfer, leave, disband.
    pub async fn group_act(&self, group_id: &str, body: OpBody) -> Result<GroupView> {
        let keys = self.session_keys().await?;
        {
            let lock = self.groups().lock_of(group_id).await;
            let _guard = lock.lock().await;
            let outcome = self.groups().act(&keys, group_id, body).await?;
            self.group_driver.apply(outcome).await?;
        }
        self.group_get(group_id).await
    }

    /// A new link for a public group: the old one stops opening what
    /// is said from now on.
    pub async fn group_rotate_link(&self, group_id: &str) -> Result<GroupView> {
        let epoch = repo::get(&self.store, group_id)
            .await?
            .map(|g| g.link_epoch as u32 + 1)
            .ok_or_else(|| MessengerError::Invalid("group_unknown".into()))?;
        self.group_act(group_id, OpBody::RotateLink { link_epoch: epoch }).await
    }

    /// Remove from this device a group I am no longer in.
    pub async fn group_forget(&self, group_id: &str) -> Result<()> {
        let lock = self.groups().lock_of(group_id).await;
        let _guard = lock.lock().await;
        let outcome = self.groups().forget(group_id).await?;
        self.group_driver.synced.lock().await.remove(group_id);
        // Its events are forgotten with it: a later join starts over.
        let _ = settings::delete(&self.store, &oldest_fetched_key(group_id)).await;
        let _ = self.ui.send(UiEvent { name: "chats.updated".into(), payload: serde_json::json!({}) });
        self.group_driver.apply(outcome).await
    }

    /// «Load earlier» in a group: the newest `limit` (at most
    /// `HISTORY_WINDOW_MAX`) events before `before` that this device
    /// lacks, from the relays. How many came; the caller then reads the
    /// messages before `before` again.
    pub async fn group_history(&self, group_id: &str, before: i64, limit: u32) -> Result<u32> {
        self.session_keys().await?;
        self.group_driver.history_before(group_id, before, limit.clamp(1, HISTORY_WINDOW_MAX)).await
    }

    /// `expiring`: a new message, tried for an hour; an edit is tried until
    /// it leaves.
    pub(crate) async fn publish_group_message(
        &self,
        message: MessageView,
        out: Outbound,
        tracking_id: &str,
        expiring: bool,
    ) -> Result<MessageView> {
        let local_id = if expiring { self.outbox.enqueue_message(out).await? } else { self.outbox.enqueue(out).await? };
        self.dm.attach_outbox(tracking_id, &local_id).await?;
        self.outbox.kick();
        Ok(self.dm.message(&message.id).await?.unwrap_or(message))
    }

    pub async fn group_send_text(&self, group_id: &str, text: &str, reply_to: Option<&str>) -> Result<MessageView> {
        let keys = self.session_keys().await?;
        let (message, out) = {
            let lock = self.groups().lock_of(group_id).await;
            let _guard = lock.lock().await;
            self.group_driver.renew_stale_key(&keys, group_id).await?;
            self.groups().prepare_text(&keys, group_id, text, reply_to).await?
        };
        let id = message.id.clone();
        self.publish_group_message(message, out, &id, true).await
    }

    pub async fn group_edit(&self, message_id: &str, text: &str) -> Result<MessageView> {
        let keys = self.session_keys().await?;
        let group_id = self.group_of_message(message_id).await?.ok_or_else(|| MessengerError::Invalid("unknown message".into()))?;
        let (message, tracking, out) = {
            let lock = self.groups().lock_of(&group_id).await;
            let _guard = lock.lock().await;
            self.group_driver.renew_stale_key(&keys, &group_id).await?;
            self.groups().prepare_edit(&keys, message_id, text).await?
        };
        self.publish_group_message(message, out, &tracking, false).await
    }

    /// Remove a message for everyone in the group: my own, or as a
    /// moderator.
    pub async fn group_delete(&self, message_id: &str) -> Result<()> {
        let keys = self.session_keys().await?;
        let group_id = self.group_of_message(message_id).await?.ok_or_else(|| MessengerError::Invalid("unknown message".into()))?;
        let (chat_id, tracking, out) = {
            let lock = self.groups().lock_of(&group_id).await;
            let _guard = lock.lock().await;
            self.group_driver.renew_stale_key(&keys, &group_id).await?;
            self.groups().prepare_delete(&keys, message_id).await?
        };
        let local_id = self.outbox.enqueue(out).await?;
        self.dm.attach_outbox(&tracking, &local_id).await?;
        self.outbox.kick();
        let _ = self.ui.send(UiEvent {
            name: messenger_dm::UI_EVENT_DM_UPDATED.into(),
            payload: serde_json::json!({ "chat_id": chat_id, "message_id": message_id }),
        });
        Ok(())
    }
}

impl MessengerRuntime {
    pub(crate) async fn is_group_message(&self, message_id: &str) -> Result<bool> {
        Ok(self.group_of_message(message_id).await?.is_some())
    }

    /// The group whose chat holds this message.
    pub(crate) async fn group_of_message(&self, message_id: &str) -> Result<Option<String>> {
        Ok(self.dm.message(message_id).await?.and_then(|m| m.chat_id.strip_prefix("group:").map(String::from)))
    }
}

impl MessengerRuntime {
    /// The link of a group as a QR code (SVG), for those who may share it.
    pub async fn group_link_qr(&self, group_id: &str) -> Result<String> {
        let link = self.group_get(group_id).await?.link.ok_or_else(|| MessengerError::Invalid("group_not_permitted".into()))?;
        messenger_groups::qr::link_svg(&link)
    }
}

impl MessengerRuntime {
    /// Remove a chat from this device. A group has to be left first.
    /// Its transfers end first, while their messages are still here.
    pub async fn chat_delete(&self, chat_id: &str) -> Result<()> {
        if let Some(group) = chat_id.strip_prefix("group:") {
            let me = self.session_pubkey().await;
            let joined = match &me {
                Some(me) => self.groups().get(group, me).await?.is_some_and(|g| g.membership == messenger_groups::service::MEMBERSHIP_JOINED),
                None => false,
            };
            if joined {
                return Err(MessengerError::Invalid("group_leave_first".into()));
            }
        }
        self.cancel_transfers_of_chat(chat_id).await;
        match chat_id.strip_prefix("group:") {
            Some(group) => self.group_forget(group).await,
            None => self.dm.delete_chat(chat_id).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_testkit::FakeTransport;

    /// `n` events of `group`, one per second from `from`, straight into the
    /// store: the row-by-row insert is too slow for tens of thousands.
    async fn fill(store: &Store, group: &str, from: i64, n: i64) {
        for chunk in (0..n).collect::<Vec<_>>().chunks(500) {
            let values = chunk.iter().map(|_| "(?, 9, ?, ?, 0, '{}', ?)").collect::<Vec<_>>().join(", ");
            let mut q = sqlx::query(sqlx::AssertSqlSafe(format!(
                "INSERT INTO msg_events_raw (event_id, kind, pubkey, created_at, received_at, source_json, raw_json) VALUES {values}"
            )));
            for i in chunk {
                let id = format!("{}{:016x}{:016x}", &group[..32], from, *i);
                q = q.bind(id).bind("ab".repeat(32)).bind(from + i).bind(format!("{{\"tags\":[[\"h\",\"{group}\"]]}}"));
            }
            q.execute(store.pool()).await.unwrap();
        }
    }

    fn sync_of(out: &Outbound) -> (&serde_json::Value, &Vec<SyncItem>) {
        match out {
            Outbound::Sync { filter, local, .. } => (&filter.0, local),
            other => panic!("not a sync: {other:?}"),
        }
    }

    /// The reconciliation by cursor carries every local id since the
    /// cursor, thirty thousand of them, so the relays send back only what
    /// is missing.
    #[tokio::test]
    async fn the_reconciliation_by_cursor_sends_every_local_id() {
        let store = Store::open_in_memory().await.unwrap();
        let g = "c".repeat(64);
        fill(&store, &g, 1_000, 30_000).await;
        let t = FakeTransport::new();
        request_since(&store, &t, std::slice::from_ref(&g), Some(500)).await.unwrap();
        let sent = t.sent();
        let (filter, local) = sync_of(&sent[0]);
        assert_eq!(local.len(), 30_000, "nothing is cut off");
        assert_eq!(filter["since"], 500);
        assert!(filter.get("until").is_none());
        let t = FakeTransport::new();
        request_since(&store, &t, &[g], Some(21_000)).await.unwrap();
        assert_eq!(sync_of(&t.sent()[0]).1.len(), 10_000, "by the cursor: what is newer than it");
    }

    /// One window of the past: bounded above, limited, of this group alone,
    /// with the local ids of the same window beside it.
    #[tokio::test]
    async fn a_window_of_the_past_is_one_group_up_to_the_bound() {
        let store = Store::open_in_memory().await.unwrap();
        let mine = "d".repeat(64);
        let other = "e".repeat(64);
        fill(&store, &mine, 1_000, 100).await;
        fill(&store, &other, 1_000, 100).await;
        let t = FakeTransport::new();
        let known = request_window(&store, &t, &mine, 1_049, 200).await.unwrap();
        assert_eq!(known, 50);
        let sent = t.sent();
        let (filter, local) = sync_of(&sent[0]);
        assert_eq!(filter, &serde_json::json!({ "kinds": [9], "#h": [mine], "until": 1_049, "limit": 200 }));
        assert_eq!(local.len(), 50);
        assert!(local.iter().all(|i| i.created_at.secs() <= 1_049));
        assert_eq!(settle_window(&store, &mine, 1_049, known).await, 0, "nothing came");
    }

    /// Two empty windows in a row mark the past as fetched; a full one
    /// starts the count again and keeps the lowest bound.
    #[tokio::test]
    async fn the_misses_of_a_group_are_counted_at_its_lowest_bound() {
        let store = Store::open_in_memory().await.unwrap();
        let g = "f".repeat(64);
        assert_eq!(oldest_fetched(&store, &g).await, None);
        note_window(&store, &g, 1_049, 0).await;
        assert_eq!(oldest_fetched(&store, &g).await, Some((1_049, 1)));
        note_window(&store, &g, 1_020, 0).await;
        assert_eq!(oldest_fetched(&store, &g).await, Some((1_020, 2)));
        note_window(&store, &g, 1_030, 7).await;
        assert_eq!(oldest_fetched(&store, &g).await, Some((1_020, 0)));
        settings::delete(&store, &oldest_fetched_key(&g)).await.unwrap();
        assert_eq!(oldest_fetched(&store, &g).await, None, "forgotten with the group");
    }
}
