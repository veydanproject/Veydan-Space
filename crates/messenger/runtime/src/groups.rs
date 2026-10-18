// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Groups in the runtime: what the host calls, and what has to happen
//! around the group service: publishing, listening to the right groups,
//! fetching their history, and looking after the keys.

use crate::relays::RelayService;
use crate::scopes;
use crate::MessengerRuntime;
use messenger_core::traits::{RelayState, SystemClock, UiEvent};
use messenger_core::{Clock, MessengerError, Outbound, PubKey, RelayUrl, Result, Scope, SubId, SyncItem, TextFormat, Timestamp, Transport};
use messenger_dm::{ChatView, MessageView};
use messenger_groups::{GroupKind, GroupService, GroupView, InviteView, OpBody, Outcome, Signal};
use messenger_ingress::{filters, Outbox};
use messenger_store::{cursors, events_raw, groups as repo, servers, settings, Store};
use std::collections::{BTreeMap, HashSet};
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

/// Ask the relays of `scope` (the server the groups live on) for
/// everything of `ids` since `since` that this device lacks: every local
/// id of the window goes along, so that Negentropy sends back only what is
/// missing.
pub(crate) async fn request_since(
    store: &Store,
    transport: &dyn Transport,
    scope: Scope,
    ids: &[String],
    since: Option<i64>,
) -> Result<()> {
    // A server hears of the ids of its own groups alone: what this device
    // holds of the groups of other servers is none of its business. A
    // private server's are those of `ids`; the home set's all but those of
    // the groups of private servers.
    let since_at = since.unwrap_or(0);
    let local = match &scope {
        Scope::Server { .. } => {
            let mut items = Vec::new();
            for id in ids {
                items.extend(events_raw::items_tagged_since(store, KIND_GROUP_EVENT, id, since_at).await.unwrap_or_default());
            }
            items
        }
        _ => {
            let mut items = events_raw::items_since(store, KIND_GROUP_EVENT, since_at).await.unwrap_or_default();
            let mut elsewhere = HashSet::new();
            for g in repo::list(store).await.unwrap_or_default() {
                // A group of a private server, or of one removed on
                // purpose (`servers::group_closed`): not the home set's.
                if !matches!(servers::of_group(store, &g.id).await, Ok(None)) {
                    let theirs = events_raw::items_tagged_since(store, KIND_GROUP_EVENT, &g.id, since_at).await.unwrap_or_default();
                    elsewhere.extend(theirs.into_iter().map(|(id, _)| id.as_hex().to_string()));
                }
            }
            if !elsewhere.is_empty() {
                items.retain(|(id, _)| !elsewhere.contains(id.as_hex()));
            }
            items
        }
    }
    .into_iter()
    .map(|(id, at)| SyncItem { id, created_at: Timestamp(at) })
    .collect();
    let filter = filters::group_events(ids, since.map(Timestamp));
    transport.send(Outbound::Sync { scope, filter, local }).await?;
    Ok(())
}

/// Ask the relays of `scope` (the group's server) for one window of a
/// group's past: the newest `limit` events up to `before` that this device
/// lacks. The local side of the comparison is this group's events of the
/// same window and nothing else, so the relays see the same set they hold.
/// How many of them this device already had.
pub(crate) async fn request_window(
    store: &Store,
    transport: &dyn Transport,
    scope: Scope,
    group_id: &str,
    before: i64,
    limit: u32,
) -> Result<usize> {
    let local = events_raw::items_tagged_until(store, KIND_GROUP_EVENT, group_id, before).await?;
    let known = local.len();
    let local = local.into_iter().map(|(id, at)| SyncItem { id, created_at: Timestamp(at) }).collect();
    let filter = filters::group_events_until(&[group_id.to_string()], Timestamp(before), limit as usize);
    transport.send(Outbound::Sync { scope, filter, local }).await?;
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
    /// The private servers whose groups are listened to now, each under a
    /// subscription of its own (the home set's is `SUB_GROUPS`).
    listened: Arc<Mutex<HashSet<String>>>,
    /// The private servers this session has taken up the groups of: those
    /// of its start, and each that came since (`servers_changed`).
    servers_known: Arc<Mutex<HashSet<String>>>,
}

impl GroupsDriver {
    pub(crate) fn new(
        groups: GroupService,

        store: Store,
        relays: Arc<RelayService>,
        outbox: Outbox,
        ui: broadcast::Sender<UiEvent>,
    ) -> Self {
        Self {
            groups,
            store,
            relays,
            outbox,
            ui,
            synced: Arc::default(),
            listened: Arc::default(),
            servers_known: Arc::default(),
        }
    }

    /// `ids` by the server each group lives on (`None`: the home set), in
    /// the order given. A group whose server is not read is left out: a
    /// group of a private server removed on purpose is closed
    /// (`servers::group_closed`), and none is asked of the home set in its
    /// stead.
    async fn by_server(&self, ids: &[String]) -> BTreeMap<Option<String>, Vec<String>> {
        let mut parts: BTreeMap<Option<String>, Vec<String>> = BTreeMap::new();
        for id in ids {
            match scopes::server_of_group(&self.store, id).await {
                Ok(server) => parts.entry(server).or_default().push(id.clone()),
                Err(e) if servers::is_unknown(&e) => {}
                Err(e) => eprintln!("messenger groups: the server of a group was not read, it is not asked: {e}"),
            }
        }
        parts
    }

    /// The list of my private servers changed (`UI_EVENT_SERVERS_UPDATED`:
    /// one put in or removed here, or by the snapshot of another device of
    /// mine). What the groups of a removed one left outside the store goes
    /// (`GroupService::forget_gone`); the pool takes the relays as they now
    /// are; the groups here that now live on a server that came (their
    /// relay was no server of mine) catch up its past by its cursor; and
    /// the subscriptions follow: `groups@<id>` of a server my groups live
    /// on, none of one they no longer do. Without a session only the first.
    /// Nothing more when the servers are those this session knows.
    pub(crate) async fn servers_changed(&self) {
        if let Err(e) = self.groups.forget_gone().await {
            eprintln!("messenger groups: what the groups of a removed server left was not forgotten: {e}");
        }
        if self.groups.signer().is_none() {
            return;
        }
        let now: HashSet<String> = scopes::private_servers(&self.store).await.into_iter().collect();
        let came: HashSet<String> = {
            let mut known = self.servers_known.lock().await;
            if *known == now {
                return;
            }
            let came = now.difference(&known).cloned().collect();
            *known = now;
            came
        };
        if let Err(e) = self.relays.reload().await {
            eprintln!("messenger groups: the relays were not reloaded: {e}");
        }
        if !came.is_empty() {
            // A welcome held for the relay of a server I had not is read
            // now that it is mine (`GroupService::on_welcome`).
            if let Some(keys) = self.groups.signer() {
                match self.groups.welcomes_of_servers_that_came(&keys).await {
                    Ok(o) => {
                        if let Err(e) = self.apply(o).await {
                            eprintln!("messenger groups: a held welcome was not carried out: {e}");
                        }
                    }
                    Err(e) => eprintln!("messenger groups: the held welcomes were not read: {e}"),
                }
            }
            let ids = self.groups.watched().await.unwrap_or_default();
            for (server, part) in self.by_server(&ids).await {
                if server.as_ref().is_some_and(|s| came.contains(s)) {
                    // Asked anew, of their server: what the home set gave
                    // them is not what it holds.
                    {
                        let mut synced = self.synced.lock().await;
                        for g in &part {
                            synced.remove(g);
                        }
                    }
                    self.catch_up(server, part).await;
                }
            }
        }
        self.resubscribe().await;
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
        // One subscription per server my groups live on, each to that
        // server alone; a server none of them lives on any more is no
        // longer listened to.
        let parts = self.by_server(&ids).await;
        let gone: Vec<String> = {
            let now: HashSet<String> = parts.keys().flatten().cloned().collect();
            let mut listened = self.listened.lock().await;
            let gone = listened.difference(&now).cloned().collect();
            *listened = now;
            gone
        };
        for server in gone {
            let _ = pool.send(Outbound::Unsubscribe { id: SubId(scopes::sub_id(filters::SUB_GROUPS, Some(&server))) }).await;
            let _ = pool.send(Outbound::Unsubscribe { id: SubId(scopes::sub_id(SUB_GROUP_PROFILES, Some(&server))) }).await;
        }
        if !parts.contains_key(&None) {
            let _ = pool.send(Outbound::Unsubscribe { id: SubId(filters::SUB_GROUPS.into()) }).await;
        }
        if ids.is_empty() {
            return;
        }
        let since = Timestamp(SystemClock.now().secs() - filters::GROUP_LIVE_MARGIN_SECS);
        for (server, part) in &parts {
            if let Err(e) = pool
                .send(Outbound::Subscribe {
                    id: SubId(scopes::sub_id(filters::SUB_GROUPS, server.as_deref())),
                    filter: filters::group_events(part, Some(since)),
                    scope: scopes::of_server(server.as_deref()),
                })
                .await
            {
                eprintln!("messenger groups: subscription failed: {e}");
            }
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

    /// Profiles of the people in my groups, so that they have names: those
    /// of the groups of each server asked of that server alone, so that the
    /// home set never hears who is in a group of a private server.
    async fn follow_members(&self) {
        let Some(me) = self.me() else { return };
        let mut parts: BTreeMap<Option<String>, Vec<PubKey>> = BTreeMap::new();
        for g in self.groups.list(&me).await.unwrap_or_default() {
            let authors = parts.entry(g.server_id.clone()).or_default();
            for m in g.members {
                if let Some(pk) = PubKey::parse(&m.pubkey) {
                    if !authors.contains(&pk) && authors.len() < MAX_PROFILE_AUTHORS {
                        authors.push(pk);
                    }
                }
            }
        }
        let pool = self.relays.pool().await;
        for (server, authors) in parts.into_iter().filter(|(_, a)| !a.is_empty()) {
            let _ = pool
                .send(Outbound::Subscribe {
                    id: SubId(scopes::sub_id(SUB_GROUP_PROFILES, server.as_deref())),
                    filter: filters::profiles(&authors),
                    scope: scopes::of_server(server.as_deref()),
                })
                .await;
        }
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
        // Each server for its own groups, with a cursor of its own.
        let mut asked = Vec::new();
        for (server, part) in self.by_server(&ids).await {
            match request_since(&self.store, pool.as_ref(), scopes::of_server(server.as_deref()), &part, since).await {
                Ok(()) => {
                    if since.is_some() {
                        let key = cursors::of_server(server.as_deref());
                        let _ = cursors::advance(&self.store, filters::CURSOR_GROUPS, key, started).await;
                    }
                    asked.extend(part);
                }
                Err(e) => {
                    eprintln!("messenger groups: history not fetched: {e}");
                    let mut synced = self.synced.lock().await;
                    for g in &part {
                        synced.remove(g);
                    }
                }
            }
        }
        if asked.is_empty() {
            return;
        }
        tokio::time::sleep(SETTLE_AFTER_SYNC).await;
        for g in asked {
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
        // Nothing of a closed group is asked for: it would not be taken.
        if self.groups.is_group_banned(group_id).await? {
            return Err(MessengerError::Invalid(messenger_groups::service::ERR_GROUP_BANNED.into()));
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
        let scope = scopes::of_server(scopes::server_of_group(&self.store, group_id).await?.as_deref());
        let known = request_window(&self.store, pool.as_ref(), scope, group_id, before, limit).await?;
        let received = settle_window(&self.store, group_id, before, known).await;
        note_window(&self.store, group_id, before, received).await;
        self.maintain_now(group_id).await;
        Ok(received)
    }

    /// A session began: listen, and catch up on what was missed.
    pub(crate) async fn session_started(&self) {
        self.synced.lock().await.clear();
        // The keys of groups a removal took while the secrets were locked.
        if let Err(e) = self.groups.forget_gone().await {
            eprintln!("messenger groups: what the groups of a removed server left was not forgotten: {e}");
        }
        // Each server that comes from now on is taken up by `servers_changed`.
        *self.servers_known.lock().await = scopes::private_servers(&self.store).await.into_iter().collect();
        let ids = self.groups.watched().await.unwrap_or_default();
        // Each server by its own cursor; the home set's always, so that it
        // starts even before the first group.
        let mut parts = self.by_server(&ids).await;
        parts.entry(None).or_default();
        for (server, part) in parts {
            self.catch_up(server, part).await;
        }
        self.resubscribe().await;
    }

    /// The catch-up of the groups `ids` of one server at the start of a
    /// session, by that server's cursor.
    async fn catch_up(&self, server: Option<String>, ids: Vec<String>) {
        let key = cursors::of_server(server.as_deref());
        let cursor = cursors::get(&self.store, filters::CURSOR_GROUPS, key).await.ok().flatten();
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
            let _ = cursors::advance(&self.store, filters::CURSOR_GROUPS, key, SystemClock.now().secs()).await;
        }
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

    /// What the handlers ask for, and the list of my private servers
    /// followed for as long as the runtime runs (`servers_changed`): a
    /// server put in or taken from the snapshot mid-session is heard for
    /// groups too, one removed lets go of them.
    pub(crate) async fn run(self, mut signals: mpsc::UnboundedReceiver<Signal>) {
        let mut told = self.ui.subscribe();
        let mut open = true;
        loop {
            tokio::select! {
                s = signals.recv() => match s {
                    Some(Signal::Resubscribe) => self.resubscribe().await,
                    Some(Signal::Maintain(g)) => self.maintain_later(g),
                    Some(Signal::Note(n)) => eprintln!("messenger groups: {n}"),
                    None => return,
                },
                ev = told.recv(), if open => match ev {
                    Ok(ev) if ev.name == crate::private::UI_EVENT_SERVERS_UPDATED => self.servers_changed().await,
                    Ok(_) => {}
                    // Missed events may have been one of them.
                    Err(broadcast::error::RecvError::Lagged(_)) => self.servers_changed().await,
                    Err(broadcast::error::RecvError::Closed) => open = false,
                },
            }
        }
    }
}

impl MessengerRuntime {
    pub fn groups(&self) -> &GroupService {
        &self.group_driver.groups
    }

    /// Where a new group lives (internal/messenger-wire.md, «Группы на
    /// сервере»): a write relay of the home set, or the relay of the private
    /// server `server` (`server_unknown` for one this device does not have),
    /// whose group it is from then on: everything of it goes there alone.
    async fn group_relay(&self, server: Option<&str>) -> Result<RelayUrl> {
        if let Some(id) = server {
            let row = servers::get(&self.store, id)
                .await?
                .filter(|s| s.is_private())
                .ok_or_else(|| MessengerError::Invalid(messenger_groups::service::ERR_SERVER_UNKNOWN.into()))?;
            return RelayUrl::parse(&row.relay_url).ok_or_else(|| MessengerError::Invalid("group_no_relay".into()));
        }
        self.relays
            .enabled_of(None)
            .await?
            .into_iter()
            .filter(|r| r.write)
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
                // A group the network closes keeps its chat here, as it
                // was, and reads as closed: `banned` says why nothing can
                // be written.
                chat.banned = self.groups().is_group_banned(id).await?;
                // The private server the group lives on, by its relay; the
                // id of the chat stays `group:<id>`. A group of a private
                // server removed on purpose names none, and nothing can be
                // written in it (`servers::group_closed`).
                let closed = match servers::private_of_relay(&self.store, &g.relay_url).await {
                    Ok(server) => {
                        chat.server_id = server;
                        false
                    }
                    Err(e) if servers::is_unknown(&e) => true,
                    Err(e) => return Err(e),
                };
                chat.can_send = g.membership == "joined" && !chat.banned && !closed;
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

    /// A new group: of the home set, or of the private server `server`
    /// (its id), where it lives and only there.
    pub async fn group_create(&self, kind: GroupKind, name: &str, about: &str, history_for_new: bool, server: Option<&str>) -> Result<GroupView> {
        let keys = self.session_keys().await?;
        let relay = self.group_relay(server.map(str::trim).filter(|s| !s.is_empty())).await?;
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

    /// Open a `veydan://group/…` link: join (public) or ask (private), on
    /// the server its `r=` names (of the home set or a private one of
    /// mine); a relay that is no server of mine is `server_unknown`, and
    /// nothing is written or sent for it.
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
        self.group_send_text_as(group_id, text, TextFormat::Plain, reply_to).await
    }

    pub async fn group_send_text_as(&self, group_id: &str, text: &str, format: TextFormat, reply_to: Option<&str>) -> Result<MessageView> {
        let keys = self.session_keys().await?;
        let (message, out) = {
            let lock = self.groups().lock_of(group_id).await;
            let _guard = lock.lock().await;
            self.group_driver.renew_stale_key(&keys, group_id).await?;
            self.groups().prepare_text_as(&keys, group_id, text, format, reply_to).await?
        };
        let id = message.id.clone();
        self.publish_group_message(message, out, &id, true).await
    }

    pub async fn group_edit(&self, message_id: &str, text: &str) -> Result<MessageView> {
        self.group_edit_as(message_id, text, TextFormat::Plain).await
    }

    pub async fn group_edit_as(&self, message_id: &str, text: &str, format: TextFormat) -> Result<MessageView> {
        let keys = self.session_keys().await?;
        let group_id = self.group_of_message(message_id).await?.ok_or_else(|| MessengerError::Invalid("unknown message".into()))?;
        let (message, tracking, out) = {
            let lock = self.groups().lock_of(&group_id).await;
            let _guard = lock.lock().await;
            self.group_driver.renew_stale_key(&keys, &group_id).await?;
            self.groups().prepare_edit_as(&keys, message_id, text, format).await?
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
            None => self.dm.delete_chat_by_me(chat_id).await,
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
        request_since(&store, &t, Scope::Own, std::slice::from_ref(&g), Some(500)).await.unwrap();
        let sent = t.sent();
        let (filter, local) = sync_of(&sent[0]);
        assert_eq!(local.len(), 30_000, "nothing is cut off");
        assert_eq!(filter["since"], 500);
        assert!(filter.get("until").is_none());
        let t = FakeTransport::new();
        request_since(&store, &t, Scope::Own, &[g], Some(21_000)).await.unwrap();
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
        let known = request_window(&store, &t, Scope::Own, &mine, 1_049, 200).await.unwrap();
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
