// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A session = a signer + a live ingress loop + an outbox pump + the
//! opening of the inbox (the list of my relations first, then the
//! subscriptions, then one history catch-up; `crate::relations`) + the
//! publisher of that list + a loop that sends the receipts this device
//! owes + the presence loop (`crate::presence`) + the keeper of my avatar
//! (`crate::avatars`) + the follower of profiles (`crate::meta`) + the
//! keeper of the queue of requests (`crate::moderation`). It exists only
//! while the runtime holds signing keys; the runtime restarts it when the
//! signer or the relay pool changes.

use async_trait::async_trait;
use messenger_core::traits::{SystemClock, UiEvent};
use messenger_core::Clock;
use messenger_core::{Ack, Context, Outbound, PubKey, Result, SyncItem, Timestamp, Transport};
use messenger_dm::DmService;
use messenger_groups::GroupService;
use messenger_ingress::{filters, Dispatcher, EffectSink, IngressLoop, Outbox};
use messenger_store::{cursors, events_raw, settings, Store};
use messenger_transport::RelayPool;
use nostr::key::Keys;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

pub const UI_EVENT_HISTORY_SYNCED: &str = "history.synced";

/// How long the history task waits for a first relay connection.
const HISTORY_WAIT_FOR_RELAY: Duration = Duration::from_secs(60);
const PUMP_INTERVAL: Duration = Duration::from_secs(5);
/// Setting that remembers when this device was last online.
const KEY_LAST_SEEN: &str = "session.last_seen";
const LAST_SEEN_EVERY_SECS: i64 = 30;

/// Effects go to the outbox (with an immediate pump) and to the UI channel.
pub struct RuntimeSink {
    pub pool: Arc<RelayPool>,
    pub outbox: Outbox,
    pub ui: broadcast::Sender<UiEvent>,
}

#[async_trait]
impl EffectSink for RuntimeSink {
    async fn send(&self, out: Outbound) -> Result<Ack> {
        // Persist first so a crash between here and the relay ack cannot
        // lose the request; the pump publishes it in the background.
        self.outbox.enqueue(out).await?;
        self.outbox.kick();
        Ok(Ack { accepted_by: vec![], rejected_by: vec![] })
    }

    fn emit(&self, event: UiEvent) {
        let _ = self.ui.send(event);
    }

    fn notify(&self, notice: messenger_core::Notice) {
        let _ = self.ui.send(UiEvent { name: "notify".into(), payload: serde_json::to_value(notice).unwrap_or_default() });
    }
}

pub struct Session {
    pub keys: Keys,
    pub started_at: Timestamp,
    ingress: IngressLoop,
    pump: JoinHandle<()>,
    /// Reads my list, subscribes, catches the history up (`crate::relations`).
    inbox: JoinHandle<()>,
    /// Publishes my list after a decision (`crate::relations`).
    relations: JoinHandle<()>,
    /// Sends the receipts this device owes (`crate::receipts`).
    receipts: JoinHandle<()>,
    /// Tells my key, watches the contacts, beats (`crate::presence`).
    presence: JoinHandle<()>,
    /// Looks after my avatar (`crate::avatars`).
    avatars: JoinHandle<()>,
    /// Follows the profiles of whom the relays bring (`crate::meta`).
    meta: JoinHandle<()>,
    /// Keeps the queue of requests: its term and its size (`crate::moderation`).
    requests: JoinHandle<()>,
}

impl Session {
    /// Subscribe to the inbox, start the ingress loop, the outbox pump, the
    /// receipt loop, the presence loop and the history catch-up.
    #[allow(clippy::too_many_arguments)]
    pub async fn start(
        store: Store,
        relays: Arc<crate::RelayService>,
        pool: Arc<RelayPool>,
        keys: Keys,
        outbox: Outbox,
        ui: broadcast::Sender<UiEvent>,
        dispatcher: Arc<Dispatcher>,
        dm: DmService,
        groups: GroupService,
        presence: Arc<crate::presence::PresenceDriver>,
        avatars: crate::avatars::Avatars,
        meta: crate::meta::MetaFollow,
    ) -> Result<Self> {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let started_at = clock.now();
        let me = PubKey::parse(&keys.public_key().to_hex()).expect("valid pubkey");

        // What arrived while we were offline counts as unread; on a fresh
        // database (no previous session) restored history does not.
        if let Ok(Some(last_seen)) = settings::get(&store, KEY_LAST_SEEN).await {
            if let Ok(at) = last_seen.parse::<i64>() {
                dm.set_unread_floor(at);
            }
        }
        let _ = settings::set(&store, KEY_LAST_SEEN, &started_at.secs().to_string()).await;

        let sink: Arc<dyn EffectSink> =
            Arc::new(RuntimeSink { pool: pool.clone(), outbox: outbox.clone(), ui: ui.clone() });
        let ctx = Context { my_pubkey: me.clone(), session_started_at: started_at, clock };
        // Listening before ingress starts, so nothing it emits is missed.
        let meta = tokio::spawn(meta.follow_loop(me.clone(), ui.subscribe()));
        let ingress = IngressLoop::spawn(pool.events(), store.clone(), Some(keys.clone()), dispatcher, ctx, sink);

        let receipts = tokio::spawn(crate::receipts::receipt_loop(dm.clone(), groups, outbox.clone(), keys.clone()));
        let presence = tokio::spawn(crate::presence::presence_loop(presence, pool.clone(), keys.clone()));
        let requests = tokio::spawn(crate::moderation::sweep_loop(
            dm.clone(),
            outbox.clone(),
            ui.clone(),
            crate::moderation::SWEEP_WAIT_FOR_HISTORY,
            crate::moderation::SWEEP_EVERY,
        ));
        let pump = tokio::spawn(pump_loop(store.clone(), pool.clone(), outbox.clone(), dm.clone(), ui.clone()));
        let quiet = pool.clone();
        avatars.session_started();
        let avatars = tokio::spawn(crate::avatars::keeper_loop(avatars, keys.clone(), move || quiet.is_silent()));
        // The list of my relations first, then the inbox and the history;
        // and a decision taken here rings the publisher of the list.
        let owed = Arc::new(tokio::sync::Notify::new());
        let ring = owed.clone();
        dm.set_relations_hook(Some(Arc::new(move || ring.notify_one())));
        let relations = tokio::spawn(crate::relations::publish_loop(dm.clone(), outbox.clone(), keys.clone(), owed));
        let inbox = tokio::spawn(crate::relations::open_inbox(store, relays, pool, keys.clone(), dm, outbox, me, started_at, ui));
        Ok(Self { keys, started_at, ingress, pump, inbox, relations, receipts, presence, avatars, meta, requests })
    }

    /// Whether the keeper of the queue of requests is still running.
    #[cfg(test)]
    pub(crate) fn sweeps_requests(&self) -> bool {
        !self.requests.is_finished()
    }

    pub fn stats(&self) -> (u64, u64, u64, u64, u64) {
        self.ingress.stats.snapshot()
    }

    pub fn stop(&self) {
        self.ingress.abort();
        self.pump.abort();
        self.inbox.abort();
        self.relations.abort();
        self.receipts.abort();
        self.presence.abort();
        self.avatars.abort();
        self.meta.abort();
        self.requests.abort();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Retry what is due, then turn publish results into message statuses.
async fn pump_loop(store: Store, pool: Arc<RelayPool>, outbox: Outbox, dm: DmService, ui: broadcast::Sender<UiEvent>) {
    let mut last_seen_at = 0i64;
    // A group's request goes to the server of the group.
    let routed = crate::scopes::Routed { store: store.clone(), pool };
    loop {
        // Whichever comes first: something was queued, or it is time to
        // retry what failed earlier.
        tokio::select! {
            _ = outbox.kicked() => {}
            _ = tokio::time::sleep(PUMP_INTERVAL) => {}
        }
        let now = SystemClock.now().secs();
        if now - last_seen_at >= LAST_SEEN_EVERY_SECS {
            last_seen_at = now;
            let _ = settings::set(&store, KEY_LAST_SEEN, &now.to_string()).await;
        }
        if let Err(e) = outbox.pump(&routed).await {
            eprintln!("messenger outbox: pump failed: {e}");
        }
        match dm.sync_statuses().await {
            Ok(events) => {
                for ev in events {
                    let _ = ui.send(ev);
                }
            }
            Err(e) => eprintln!("messenger dm: status sync failed: {e}"),
        }
    }
}

/// The local side of the DM history reconciliation with `server` (`None`:
/// the home set): the gift wraps since `since` that came from it, and no
/// others, so a server learns of the ids and times of its own wraps alone
/// (internal/messenger-wire.md, «Серверы»). A private server's are those
/// that came by its relay; the home set's are all the rest but those of a
/// relay of a private server — wraps of the home relays, and those a push
/// left (the relay the push named, else none of a server: of the home set,
/// as a push is). A wrap of the same id that came by both sides is listed
/// on the side of the copy kept, the first that came; the other side then
/// sends it once more, and the ingress drops it as seen.
pub(crate) async fn dm_local_set(store: &Store, server: Option<&str>, since: i64) -> Vec<SyncItem> {
    let private: std::collections::HashSet<String> = crate::scopes::private_servers(store).await.into_iter().collect();
    events_raw::items_since_with_relay(store, 1059, since)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, _, relay)| {
            let of = relay.as_deref().map(messenger_store::servers::id_of);
            match server {
                Some(id) => of.as_deref() == Some(id),
                None => !of.is_some_and(|of| private.contains(&of)),
            }
        })
        .map(|(id, at, _)| SyncItem { id, created_at: Timestamp(at) })
        .collect()
}

/// One history reconciliation per session and server (`None`: the home
/// set): everything addressed to us there since the last completed
/// catch-up of that server (minus the gift-wrap time jitter), or from the
/// beginning on a fresh database. After the list of my relations was read
/// (`crate::relations::open_inbox`), so the history meets it.
pub(crate) async fn history_catch_up(
    store: Store,
    pool: Arc<RelayPool>,
    me: PubKey,
    started_at: Timestamp,
    ui: broadcast::Sender<UiEvent>,
    server: Option<String>,
) {
    let scope = crate::scopes::of_server(server.as_deref());
    let deadline = tokio::time::Instant::now() + HISTORY_WAIT_FOR_RELAY;
    loop {
        if pool.connected_in(&scope).await > 0 && !pool.is_silent() {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let cursor_key = cursors::of_server(server.as_deref());
    let cursor = cursors::get(&store, filters::CURSOR_DM_INBOX, cursor_key).await.ok().flatten();
    let since = cursor.map(|c| c - filters::DM_LIVE_MARGIN_SECS).unwrap_or(0).max(0);
    let local = dm_local_set(&store, server.as_deref(), since).await;
    let result = pool.send(Outbound::Sync { scope, filter: filters::dm_history(&me, Timestamp(since)), local }).await;
    match result {
        Ok(_) => {
            let _ = cursors::advance(&store, filters::CURSOR_DM_INBOX, cursor_key, started_at.secs()).await;
            let _ = ui.send(UiEvent { name: UI_EVENT_HISTORY_SYNCED.into(), payload: serde_json::json!({ "since": since }) });
        }
        Err(e) => {
            let _ = ui.send(UiEvent {
                name: "error".into(),
                payload: serde_json::json!({ "family": "history", "error": e.to_string() }),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::{EventId, EventSource, RawEvent, RelayUrl};
    use messenger_store::servers;

    async fn wrap_by(store: &Store, n: u64, at: i64, source: EventSource) -> String {
        let id = format!("{n:064x}");
        let raw = RawEvent {
            id: EventId::parse(&id).unwrap(),
            kind: 1059,
            pubkey: PubKey::parse(&"b".repeat(64)).unwrap(),
            created_at: Timestamp(at),
            json: serde_json::json!({ "id": id }),
            source,
        };
        assert!(events_raw::insert_if_new(store, &raw).await.unwrap());
        id
    }

    fn by(url: &str) -> EventSource {
        EventSource::Relay { url: RelayUrl::parse(url).unwrap() }
    }

    fn synced(url: &str) -> EventSource {
        EventSource::Sync { url: RelayUrl::parse(url).unwrap() }
    }

    fn ids(items: &[SyncItem]) -> Vec<String> {
        let mut v: Vec<String> = items.iter().map(|i| i.id.as_hex().to_string()).collect();
        v.sort();
        v
    }

    /// The DM history with a server is reconciled against its own wraps
    /// alone: S1 never hears of the ids of the wraps of the home relays or
    /// of S2, nor the home set of those of S1 and S2; a push's wrap and one
    /// of a relay that is no server's go with the home set.
    #[tokio::test]
    async fn the_local_set_of_a_server_holds_its_own_wraps_alone() {
        let store = Store::open_in_memory().await.unwrap();
        let (home, s1, s2) = ("wss://home.example", "wss://s1.example", "wss://s2.example");
        let row = |url: &str, kind: &str| servers::ServerRow {
            id: String::new(),
            kind: kind.into(),
            name: None,
            relay_url: url.into(),
            relay_key: None,
            media_url: None,
            call_node: None,
            push_mode: servers::PUSH_OFF.into(),
            source: None,
            added_at: 0,
        };
        servers::put(&store, &row(home, servers::KIND_HOME)).await.unwrap();
        let id1 = servers::put_private(&store, &row(s1, servers::KIND_PRIVATE)).await.unwrap();
        let id2 = servers::put_private(&store, &row(s2, servers::KIND_PRIVATE)).await.unwrap();

        let at_home = wrap_by(&store, 1, 1_000, by(home)).await;
        let history_home = wrap_by(&store, 2, 1_001, synced(home)).await;
        let of_s1 = wrap_by(&store, 3, 1_002, by("wss://S1.example/")).await;
        let history_s1 = wrap_by(&store, 4, 1_003, synced(s1)).await;
        let of_s2 = wrap_by(&store, 5, 1_004, by(s2)).await;
        let pushed = wrap_by(&store, 6, 1_005, synced("wss://push.invalid")).await;
        let elsewhere = wrap_by(&store, 7, 1_006, by("wss://nobody.example")).await;
        let old_home = wrap_by(&store, 8, 10, by(home)).await;

        let mut want_s1 = vec![of_s1, history_s1];
        want_s1.sort();
        assert_eq!(ids(&dm_local_set(&store, Some(&id1), 500).await), want_s1, "S1 hears of its own wraps");
        assert_eq!(ids(&dm_local_set(&store, Some(&id2), 500).await), vec![of_s2]);
        let mut want_home = vec![at_home, history_home, pushed, elsewhere];
        want_home.sort();
        assert_eq!(ids(&dm_local_set(&store, None, 500).await), want_home, "the home set, nothing of S1 or S2");
        assert!(ids(&dm_local_set(&store, None, 0).await).contains(&old_home), "since the cursor");
    }
}
