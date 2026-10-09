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
use messenger_store::{cursors, events_raw, groups as repo, Store};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, Mutex};

const SUB_GROUP_PROFILES: &str = "group-profiles";
const WAIT_FOR_RELAY: Duration = Duration::from_secs(60);
/// After the history was asked for: time for it to be read before the
/// keys are looked after.
const SETTLE_AFTER_SYNC: Duration = Duration::from_secs(4);
pub(crate) const MAX_PROFILE_AUTHORS: usize = 500;
const KIND_GROUP_EVENT: u16 = 9;

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
        let local = events_raw::items_since(&self.store, KIND_GROUP_EVENT, since.unwrap_or(0), 20_000)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|(id, at)| SyncItem { id, created_at: Timestamp(at) })
            .collect();
        let pool = self.relays.pool().await;
        let filter = filters::group_events(&ids, since.map(Timestamp));
        match pool.send(Outbound::Sync { scope: Scope::Own, filter, local }).await {
            Ok(_) => {
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
        let _ = self.ui.send(UiEvent { name: "chats.updated".into(), payload: serde_json::json!({}) });
        self.group_driver.apply(outcome).await
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
        let (message, out) = self.groups().prepare_text(&keys, group_id, text, reply_to).await?;
        let id = message.id.clone();
        self.publish_group_message(message, out, &id, true).await
    }

    pub async fn group_edit(&self, message_id: &str, text: &str) -> Result<MessageView> {
        let keys = self.session_keys().await?;
        let (message, tracking, out) = self.groups().prepare_edit(&keys, message_id, text).await?;
        self.publish_group_message(message, out, &tracking, false).await
    }

    /// Remove a message for everyone in the group: my own, or as a
    /// moderator.
    pub async fn group_delete(&self, message_id: &str) -> Result<()> {
        let keys = self.session_keys().await?;
        let (chat_id, tracking, out) = self.groups().prepare_delete(&keys, message_id).await?;
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
        Ok(self.dm.message(message_id).await?.is_some_and(|m| m.chat_id.starts_with("group:")))
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
