// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A session = a signer + a live ingress loop + an outbox pump + one
//! history catch-up + a loop that sends the receipts this device owes + the
//! presence loop (`crate::presence`) + the keeper of my avatar
//! (`crate::avatars`). It exists only while the runtime holds signing keys;
//! the runtime restarts it when the signer or the relay pool changes.

use async_trait::async_trait;
use messenger_core::traits::{RelayState, SystemClock, UiEvent};
use messenger_core::Clock;
use messenger_core::{Ack, Context, Outbound, PubKey, Result, Scope, SubId, SyncItem, Timestamp, Transport};
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
    history: JoinHandle<()>,
    /// Sends the receipts this device owes (`crate::receipts`).
    receipts: JoinHandle<()>,
    /// Tells my key, watches the contacts, beats (`crate::presence`).
    presence: JoinHandle<()>,
    /// Looks after my avatar (`crate::avatars`).
    avatars: JoinHandle<()>,
}

impl Session {
    /// Subscribe to the inbox, start the ingress loop, the outbox pump, the
    /// receipt loop, the presence loop and the history catch-up.
    #[allow(clippy::too_many_arguments)]
    pub async fn start(
        store: Store,
        pool: Arc<RelayPool>,
        keys: Keys,
        outbox: Outbox,
        ui: broadcast::Sender<UiEvent>,
        dispatcher: Arc<Dispatcher>,
        dm: DmService,
        groups: GroupService,
        presence: Arc<crate::presence::PresenceDriver>,
        avatars: crate::avatars::Avatars,
    ) -> Result<Self> {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let started_at = clock.now();
        let me = PubKey::parse(&keys.public_key().to_hex()).expect("valid pubkey");

        let since = Timestamp(started_at.secs() - filters::DM_LIVE_MARGIN_SECS);
        pool.send(Outbound::Subscribe {
            id: SubId(filters::SUB_DM_LIVE.into()),
            filter: filters::dm_inbox(&me, since),
            scope: Scope::Own,
        })
        .await?;

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
        let ingress = IngressLoop::spawn(pool.events(), store.clone(), Some(keys.clone()), dispatcher, ctx, sink);

        let receipts = tokio::spawn(crate::receipts::receipt_loop(dm.clone(), groups, outbox.clone(), keys.clone()));
        let presence = tokio::spawn(crate::presence::presence_loop(presence, pool.clone(), keys.clone()));
        let pump = tokio::spawn(pump_loop(store.clone(), pool.clone(), outbox, dm, ui.clone()));
        let quiet = pool.clone();
        avatars.session_started();
        let avatars = tokio::spawn(crate::avatars::keeper_loop(avatars, keys.clone(), move || quiet.is_silent()));
        let history = tokio::spawn(history_catch_up(store, pool, me, started_at, ui));
        Ok(Self { keys, started_at, ingress, pump, history, receipts, presence, avatars })
    }

    pub fn stats(&self) -> (u64, u64, u64, u64, u64) {
        self.ingress.stats.snapshot()
    }

    pub fn stop(&self) {
        self.ingress.abort();
        self.pump.abort();
        self.history.abort();
        self.receipts.abort();
        self.presence.abort();
        self.avatars.abort();
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
        if let Err(e) = outbox.pump(pool.as_ref()).await {
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

/// One history reconciliation per session: everything addressed to us
/// since the last completed catch-up (minus the gift-wrap time jitter), or
/// from the beginning on a fresh database.
async fn history_catch_up(
    store: Store,
    pool: Arc<RelayPool>,
    me: PubKey,
    started_at: Timestamp,
    ui: broadcast::Sender<UiEvent>,
) {
    let deadline = tokio::time::Instant::now() + HISTORY_WAIT_FOR_RELAY;
    loop {
        let connected = pool.status().await.relays.iter().any(|r| r.state == RelayState::Connected);
        if connected && !pool.is_silent() {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let cursor = cursors::get(&store, filters::CURSOR_DM_INBOX, cursors::ANY_RELAY).await.ok().flatten();
    let since = cursor.map(|c| c - filters::DM_LIVE_MARGIN_SECS).unwrap_or(0).max(0);
    let local = events_raw::items_since(&store, 1059, since, 20_000)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(id, at)| SyncItem { id, created_at: Timestamp(at) })
        .collect();
    let result = pool
        .send(Outbound::Sync { scope: Scope::Own, filter: filters::dm_history(&me, Timestamp(since)), local })
        .await;
    match result {
        Ok(_) => {
            let _ = cursors::advance(&store, filters::CURSOR_DM_INBOX, cursors::ANY_RELAY, started_at.secs()).await;
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
