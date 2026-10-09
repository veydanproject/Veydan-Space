// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `RelayPool`: `core::Transport` over `nostr_sdk::Client`.
//!
//! The pool owns the relay connections, forwards every event it sees as a
//! `RawEvent`, and executes `Outbound` requests. It does not interpret
//! events. Silent mode drops all connections and refuses every send.

use async_trait::async_trait;
use futures_util::StreamExt;
use messenger_core::traits::{RelayState, RelayStatus, RelayStatusSnapshot};
use messenger_core::{
    Ack, EventId, EventSource, MessengerError, Outbound, PubKey, RawEvent, RelayUrl, Result, Timestamp,
    Transport,
};
use nostr_sdk::prelude::*;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

/// One relay to keep in the pool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelayConfig {
    pub url: RelayUrl,
    pub read: bool,
    pub write: bool,
    /// Static gate key sent as `?key=` on the connect URL. Never logged.
    pub api_key: Option<String>,
}

impl RelayConfig {
    /// The URL handed to the WebSocket layer: base url plus the key query.
    pub fn connect_url(&self) -> String {
        match &self.api_key {
            Some(k) if !k.is_empty() => format!("{}/?key={}", self.url.as_str(), k),
            _ => self.url.as_str().to_string(),
        }
    }
}

pub struct RelayPool {
    client: Client,
    silent: AtomicBool,
    tx: mpsc::Sender<RawEvent>,
    rx: Mutex<Option<mpsc::Receiver<RawEvent>>>,
    /// Configured relays; nostr-sdk keeps the live set.
    configured: Mutex<Vec<RelayConfig>>,
    /// The websockets open now, whichever way they went.
    sockets: Arc<crate::ws::Sockets>,
}

/// How long a relay that dropped waits before it is tried again. Fixed:
/// `nostr-sdk` would stretch it from failures counted over the whole life
/// of the pool, and a night of a sleeping phone holds every reconnect back a
/// minute for good.
const RETRY_INTERVAL: Duration = Duration::from_secs(3);

impl RelayPool {
    /// `signer` enables NIP-42 authentication and is required for relays
    /// that demand it. Without a signer the pool is read-only for such relays.
    pub fn new(signer: Option<Keys>) -> Self {
        crate::ensure_crypto_provider();
        // Every relay is connected by the transport that asks which way the
        // relay is reached: directly, or through a bridge.
        let transport = crate::ws::BridgedTransport::default();
        let sockets = transport.sockets();
        let builder = Client::builder().websocket_transport(transport);
        let client = match signer {
            Some(keys) => builder.authenticator(SignerAuthenticator::new(keys)).build(),
            None => builder.build(),
        };
        let (tx, rx) = mpsc::channel(1024);
        let pool = Self {
            client,
            silent: AtomicBool::new(false),
            tx,
            rx: Mutex::new(Some(rx)),
            configured: Mutex::new(Vec::new()),
            sockets,
        };
        pool.spawn_forwarder();
        pool
    }

    fn spawn_forwarder(&self) {
        let mut stream = self.client.notifications();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            while let Some(n) = stream.next().await {
                match n {
                    ClientNotification::Event { relay_url, event, .. } => {
                        let Some(raw) = to_raw(&event, EventSource::Relay { url: to_core_url(&relay_url) }) else {
                            continue;
                        };
                        if tx.send(raw).await.is_err() {
                            break;
                        }
                    }
                    ClientNotification::Shutdown => break,
                    _ => {}
                }
            }
        });
    }

    /// Replace the relay set. Existing connections to relays that stay are
    /// kept; removed ones are dropped; new ones are added and connected
    /// unless silent mode is on.
    pub async fn set_relays(&self, relays: Vec<RelayConfig>) -> Result<()> {
        let wanted: HashSet<String> = relays.iter().map(|r| r.url.as_str().to_string()).collect();
        let current = self.client.relays().await;
        for (url, _) in current {
            if !wanted.contains(to_core_url(&url).as_str()) {
                let _ = self.client.remove_relay(url.clone()).await;
            }
        }
        for r in &relays {
            self.client
                .add_relay(r.connect_url())
                .retry_interval(RETRY_INTERVAL)
                .adjust_retry_interval(false)
                .await
                .map_err(|e| MessengerError::Transport(e.to_string()))?;
        }
        *self.configured.lock().unwrap() = relays;
        if !self.is_silent() {
            self.client.connect().await;
        }
        Ok(())
    }

    pub fn configured(&self) -> Vec<RelayConfig> {
        self.configured.lock().unwrap().clone()
    }

    pub async fn connect(&self) {
        if !self.is_silent() {
            self.client.connect().await;
        }
    }

    pub async fn disconnect(&self) {
        self.client.disconnect().await;
    }

    /// Asks every relay connection for a sign of life and closes the ones
    /// that give none within `patience`; `nostr-sdk` connects them anew.
    /// Called when the device wakes: a socket that slept may look open and
    /// carry nothing, and the pings of `nostr-sdk` notice it only after two
    /// rounds of 55 seconds. Returns how many were closed.
    pub async fn revive(&self, patience: Duration) -> usize {
        self.sockets.probe(patience).await
    }

    /// Closes every relay connection; `nostr-sdk` makes each anew, the way
    /// the rule of the bridges says now. Unlike `disconnect` and `connect`
    /// one after the other, this does not race the connection task of
    /// `nostr-sdk`, which a terminate it has not yet seen leaves stuck.
    pub fn reopen(&self) {
        self.sockets.close_all();
    }

    pub fn is_silent(&self) -> bool {
        self.silent.load(Ordering::Relaxed)
    }

    /// Silent mode: no connections, no sends. Turning it off reconnects.
    pub async fn set_silent(&self, on: bool) {
        self.silent.store(on, Ordering::Relaxed);
        if on {
            self.client.disconnect().await;
        } else {
            self.client.connect().await;
        }
    }

    pub async fn shutdown(&self) {
        self.client.shutdown().await;
    }

    fn guard_send(&self) -> Result<()> {
        if self.is_silent() {
            return Err(MessengerError::Transport("silent mode".into()));
        }
        Ok(())
    }

    fn parse_event(json: &serde_json::Value) -> Result<Event> {
        let event: Event = serde_json::from_value(json.clone())
            .map_err(|e| MessengerError::Invalid(format!("wire event: {e}")))?;
        event
            .verify()
            .map_err(|_| MessengerError::Invalid("wire event has an invalid signature".into()))?;
        Ok(event)
    }

    fn parse_filter(f: &messenger_core::outbound::Filter) -> Result<Filter> {
        serde_json::from_value(f.0.clone()).map_err(|e| MessengerError::Invalid(format!("filter: {e}")))
    }
}


/// How long a relay may take to answer the first Negentropy message
/// before we fall back to a plain REQ.
const NEGENTROPY_PROBE_TIMEOUT: Duration = Duration::from_secs(4);
/// How long a plain-REQ history fetch may take per call.
const HISTORY_FETCH_TIMEOUT: Duration = Duration::from_secs(20);
/// How long to wait for a peer's inbox relay that is not in the pool.
const EPHEMERAL_CONNECT_TIMEOUT: Duration = Duration::from_secs(6);
/// At most this many foreign inbox relays are tried per message.
const MAX_EPHEMERAL_RELAYS: usize = 3;

impl RelayPool {
    /// History catch-up. Per relay: Negentropy against `local`; when the
    /// relay does not speak NIP-77 (or the attempt fails), a plain REQ with
    /// the same filter. Everything found is pushed onto the event stream
    /// with `EventSource::Sync`. Returns how many events were forwarded.
    async fn reconcile(&self, filter: Filter, local: Vec<messenger_core::SyncItem>) -> Result<usize> {
        let items: Vec<(nostr_sdk::prelude::EventId, nostr_sdk::prelude::Timestamp)> = local
            .iter()
            .filter_map(|i| {
                Some((
                    nostr_sdk::prelude::EventId::from_hex(i.id.as_hex()).ok()?,
                    nostr_sdk::prelude::Timestamp::from(i.created_at.secs().max(0) as u64),
                ))
            })
            .collect();
        let known: HashSet<String> = local.iter().map(|i| i.id.as_hex().to_string()).collect();
        let opts = SyncOptions::default()
            .direction(SyncDirection::Down)
            .initial_timeout(NEGENTROPY_PROBE_TIMEOUT);
        let mut forwarded = 0usize;
        let mut any_relay = false;
        for (url, relay) in self.client.relays().await {
            if relay.status() != nostr_sdk::relay::RelayStatus::Connected {
                continue;
            }
            any_relay = true;
            let source = EventSource::Sync { url: to_core_url(&url) };
            let events: Vec<Event> = match relay.sync(filter.clone()).items(items.clone()).opts(opts.clone()).await {
                Ok(summary) => {
                    let mut found = Vec::with_capacity(summary.received.len());
                    for id in &summary.received {
                        if let Ok(Some(ev)) = self.client.database().event_by_id(id).await {
                            found.push(ev);
                        }
                    }
                    found
                }
                Err(_) => match relay.fetch_events(filter.clone()).timeout(HISTORY_FETCH_TIMEOUT).await {
                    Ok(events) => events.into_iter().collect(),
                    Err(_) => continue,
                },
            };
            for ev in events {
                if known.contains(&ev.id.to_hex()) {
                    continue;
                }
                if let Some(raw) = to_raw(&ev, source.clone()) {
                    if self.tx.send(raw).await.is_ok() {
                        forwarded += 1;
                    }
                }
            }
        }
        if !any_relay {
            return Err(MessengerError::Transport("no relay connected".into()));
        }
        Ok(forwarded)
    }

    /// Connect urls of the relays that are connected and accept writes.
    async fn connected_write_targets(&self) -> Result<Vec<String>> {
        let configured = self.configured();
        let targets: Vec<String> = self
            .client
            .relays()
            .await
            .iter()
            .filter(|(_, relay)| relay.status() == nostr_sdk::relay::RelayStatus::Connected)
            .filter(|(url, _)| {
                let core = to_core_url(url);
                configured.iter().find(|c| c.url == core).map(|c| c.write).unwrap_or(true)
            })
            .map(|(url, _)| url.to_string())
            .collect();
        if targets.is_empty() {
            return Err(MessengerError::Transport("no relay connected".into()));
        }
        Ok(targets)
    }

    /// Deliver to a peer: their inbox relays when we know them (relays of
    /// ours are used directly, foreign ones through a short-lived
    /// connection), otherwise every write relay of ours.
    async fn publish_to_inbox(&self, ev: &Event, hints: &[RelayUrl]) -> Result<Ack> {
        // Pool relays by their core url: how to address them, and whether
        // they are connected right now.
        let pool: std::collections::HashMap<String, (String, bool)> = self
            .client
            .relays()
            .await
            .iter()
            .map(|(url, relay)| {
                let connected = relay.status() == nostr_sdk::relay::RelayStatus::Connected;
                (to_core_url(url).as_str().to_string(), (url.to_string(), connected))
            })
            .collect();
        let mut targets: Vec<String> = Vec::new();
        let mut ephemeral: Vec<String> = Vec::new();
        for h in hints {
            match pool.get(h.as_str()) {
                // Relays that are not connected right now are left out:
                // waiting for them would hold the message back for their
                // whole timeout. The outbox retries when nothing accepted.
                Some((address, true)) => targets.push(address.clone()),
                Some(_) => {}
                None if ephemeral.len() < MAX_EPHEMERAL_RELAYS => ephemeral.push(h.as_str().to_string()),
                None => {}
            }
        }
        for url in &ephemeral {
            if self.client.add_relay(url.as_str()).await.is_ok()
                && self.client.try_connect_relay(url.as_str(), EPHEMERAL_CONNECT_TIMEOUT).await.is_ok()
            {
                targets.push(url.clone());
            }
        }
        if targets.is_empty() {
            targets = self.connected_write_targets().await.unwrap_or_default();
        }
        if targets.is_empty() {
            for url in &ephemeral {
                let _ = self.client.remove_relay(url.as_str()).await;
            }
            return Err(MessengerError::Transport("no relay connected".into()));
        }
        // Not into the client's store either (see `send`): a wrap to me (a
        // note to self) must come back to ingress.
        let res = self.client.send_event(ev).to(targets).save_into_database(false).await;
        for url in &ephemeral {
            let _ = self.client.remove_relay(url.as_str()).await;
        }
        let res = res.map_err(|e| MessengerError::Transport(e.to_string()))?;
        Ok(ack_from(&res))
    }
}

/// Core relay identity is the base url: the `?key=` query used for gated
/// relays is stripped so keys never travel into status, events or logs.
fn to_core_url(u: &nostr::types::url::RelayUrl) -> RelayUrl {
    let s = u.as_str_without_trailing_slash();
    let base = s.split('?').next().unwrap_or(s);
    RelayUrl::parse(base)
        .unwrap_or_else(|| RelayUrl::parse("wss://invalid.invalid").expect("static url"))
}

fn to_raw(event: &Event, source: EventSource) -> Option<RawEvent> {
    Some(RawEvent {
        id: EventId::parse(&event.id.to_hex())?,
        kind: event.kind.as_u16(),
        pubkey: PubKey::parse(&event.pubkey.to_hex())?,
        created_at: Timestamp(event.created_at.as_secs() as i64),
        json: serde_json::to_value(event).ok()?,
        source,
    })
}

fn ack_from<T, S>(out: &Output<T, S>) -> Ack {
    Ack {
        accepted_by: out.success.keys().map(to_core_url).collect(),
        rejected_by: out.failed.iter().map(|(u, e)| (to_core_url(u), e.clone())).collect(),
    }
}

fn map_state(s: nostr_sdk::relay::RelayStatus) -> RelayState {
    use nostr_sdk::relay::RelayStatus as S;
    match s {
        S::Connected => RelayState::Connected,
        S::Pending | S::Connecting => RelayState::Connecting,
        S::Banned | S::Sleeping => RelayState::Paused,
        _ => RelayState::Disconnected,
    }
}

#[async_trait]
impl Transport for RelayPool {
    async fn send(&self, out: Outbound) -> Result<Ack> {
        // Silent mode blocks anything that would reach a relay. Subscriptions
        // only record intent (nostr-sdk applies them on connect), so they pass.
        if !matches!(out, Outbound::Subscribe { .. } | Outbound::Unsubscribe { .. }) {
            self.guard_send()?;
        }
        match out {
            Outbound::PublishOwn { event } | Outbound::PublishScoped { event, .. } => {
                let ev = Self::parse_event(&event.json)?;
                let targets = self.connected_write_targets().await?;
                // Not into the client's store: an event found there is not
                // told again, so our own copy coming back on our
                // subscription would never reach ingress and be fetched
                // again by every new session.
                let res = self
                    .client
                    .send_event(&ev)
                    .to(targets)
                    .save_into_database(false)
                    .await
                    .map_err(|e| MessengerError::Transport(e.to_string()))?;
                Ok(ack_from(&res))
            }
            Outbound::PublishToInbox { event, hint_relays, .. } => {
                let ev = Self::parse_event(&event.json)?;
                self.publish_to_inbox(&ev, &hint_relays).await
            }
            Outbound::Subscribe { id, filter, .. } => {
                let f = Self::parse_filter(&filter)?;
                let id = SubscriptionId::new(id.0);
                // A subscription is sent again to change its filter (a group
                // joined, a contact added). nostr-sdk keeps the first filter
                // of an ID and refuses the next one, reporting it only per
                // relay: without closing the old one first, nothing changes
                // until the app restarts.
                let _ = self.client.unsubscribe(&id).await;
                let out = self
                    .client
                    .subscribe(f)
                    .with_id(id)
                    .await
                    .map_err(|e| MessengerError::Transport(e.to_string()))?;
                Ok(ack_from(&out))
            }
            Outbound::Unsubscribe { id } => {
                // Unsubscribing from a relay that already dropped the
                // subscription is not an error worth surfacing.
                let _ = self.client.unsubscribe(&SubscriptionId::new(id.0)).await;
                Ok(Ack { accepted_by: vec![], rejected_by: vec![] })
            }
            Outbound::Sync { filter, local, .. } => {
                let f = Self::parse_filter(&filter)?;
                self.reconcile(f, local).await?;
                Ok(Ack { accepted_by: vec![], rejected_by: vec![] })
            }
        }
    }

    fn events(&self) -> mpsc::Receiver<RawEvent> {
        self.rx.lock().unwrap().take().expect("events() may be called once")
    }

    async fn status(&self) -> RelayStatusSnapshot {
        let relays = self.client.relays().await;
        let mut list: Vec<RelayStatus> = relays
            .iter()
            .map(|(url, relay)| RelayStatus { url: to_core_url(url), state: map_state(relay.status()) })
            .collect();
        list.sort_by(|a, b| a.url.as_str().cmp(b.url.as_str()));
        RelayStatusSnapshot { relays: list }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::outbound::{Filter as CoreFilter, WireEvent};
    use messenger_core::{Scope, SubId};
    use nostr_sdk::local_relay::LocalRelay;
    use std::time::Duration;

    async fn local_relay() -> (LocalRelay, RelayUrl) {
        let relay = LocalRelay::builder().build();
        relay.run().await.expect("local relay starts");
        let url = relay.url().await;
        (relay, RelayUrl::parse(url.as_str_without_trailing_slash()).unwrap())
    }

    fn signed_note(keys: &Keys, text: &str) -> WireEvent {
        let ev = EventBuilder::new(Kind::from(1u16), text).finalize(keys).unwrap();
        WireEvent { id: EventId::parse(&ev.id.to_hex()).unwrap(), json: serde_json::to_value(&ev).unwrap() }
    }

    #[tokio::test]
    async fn publish_subscribe_roundtrip_through_local_relay() {
        let (_relay, url) = local_relay().await;

        let alice = Keys::generate();
        let sender = RelayPool::new(Some(alice.clone()));
        sender
            .set_relays(vec![RelayConfig { url: url.clone(), read: true, write: true, api_key: None }])
            .await
            .unwrap();

        let receiver = RelayPool::new(None);
        let mut inbox = receiver.events();
        receiver
            .set_relays(vec![RelayConfig { url: url.clone(), read: true, write: true, api_key: None }])
            .await
            .unwrap();

        // Wait for both to connect.
        for _ in 0..50 {
            let a = sender.status().await.relays.iter().all(|r| r.state == RelayState::Connected);
            let b = receiver.status().await.relays.iter().all(|r| r.state == RelayState::Connected);
            if a && b {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(sender.status().await.relays[0].state == RelayState::Connected);

        let filter = CoreFilter(serde_json::json!({ "kinds": [1], "authors": [alice.public_key().to_hex()] }));
        receiver
            .send(Outbound::Subscribe { id: SubId("t".into()), filter, scope: Scope::Own })
            .await
            .unwrap();

        let ack = sender.send(Outbound::PublishOwn { event: signed_note(&alice, "hello") }).await.unwrap();
        assert!(ack.is_delivered(), "relay must accept the note: {ack:?}");

        let got = tokio::time::timeout(Duration::from_secs(5), inbox.recv()).await.expect("event arrives").unwrap();
        assert_eq!(got.kind, 1);
        assert_eq!(got.pubkey.as_hex(), alice.public_key().to_hex());
        assert!(matches!(got.source, EventSource::Relay { .. }));

        sender.shutdown().await;
        receiver.shutdown().await;
    }

    /// What we publish comes back on our own subscription: the copy for my
    /// devices must reach ingress, or every new session fetches it again.
    #[tokio::test]
    async fn our_own_event_comes_back_on_our_subscription() {
        let (_relay, url) = local_relay().await;
        let alice = Keys::generate();
        let pool = RelayPool::new(Some(alice.clone()));
        let mut inbox = pool.events();
        pool.set_relays(vec![RelayConfig { url, read: true, write: true, api_key: None }]).await.unwrap();
        for _ in 0..50 {
            if pool.status().await.relays.iter().all(|r| r.state == RelayState::Connected) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let filter = CoreFilter(serde_json::json!({ "kinds": [1], "authors": [alice.public_key().to_hex()] }));
        pool.send(Outbound::Subscribe { id: SubId("own".into()), filter, scope: Scope::Own }).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;

        let note = signed_note(&alice, "to myself");
        assert!(pool.send(Outbound::PublishOwn { event: note.clone() }).await.unwrap().is_delivered());
        let got = tokio::time::timeout(Duration::from_secs(5), inbox.recv()).await.expect("our copy comes back").unwrap();
        assert_eq!(got.id, note.id);
        pool.shutdown().await;
    }

    /// A group joined while the app runs is added to the subscription that
    /// already exists: the same ID with a new filter must replace the old one.
    #[tokio::test]
    async fn subscribing_again_under_the_same_id_replaces_the_filter() {
        let (_relay, url) = local_relay().await;
        let config = || vec![RelayConfig { url: url.clone(), read: true, write: true, api_key: None }];

        let alice = Keys::generate();
        let bob = Keys::generate();
        let alice_out = RelayPool::new(Some(alice.clone()));
        let bob_out = RelayPool::new(Some(bob.clone()));
        let receiver = RelayPool::new(None);
        let mut inbox = receiver.events();
        for pool in [&alice_out, &bob_out, &receiver] {
            pool.set_relays(config()).await.unwrap();
        }
        for _ in 0..50 {
            let mut all = true;
            for pool in [&alice_out, &bob_out, &receiver] {
                all &= pool.status().await.relays.iter().all(|r| r.state == RelayState::Connected);
            }
            if all {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        let by = |keys: &Keys| CoreFilter(serde_json::json!({ "kinds": [1], "authors": [keys.public_key().to_hex()] }));
        let id = || SubId("same".into());
        receiver.send(Outbound::Subscribe { id: id(), filter: by(&alice), scope: Scope::Own }).await.unwrap();
        receiver.send(Outbound::Subscribe { id: id(), filter: by(&bob), scope: Scope::Own }).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;

        let ack = bob_out.send(Outbound::PublishOwn { event: signed_note(&bob, "from bob") }).await.unwrap();
        assert!(ack.is_delivered(), "{ack:?}");
        let got = tokio::time::timeout(Duration::from_secs(5), inbox.recv())
            .await
            .expect("the second filter is the one the relay has")
            .unwrap();
        assert_eq!(got.pubkey.as_hex(), bob.public_key().to_hex());

        for pool in [alice_out, bob_out, receiver] {
            pool.shutdown().await;
        }
    }

    #[tokio::test]
    async fn silent_mode_refuses_sends_and_disconnects() {
        let (_relay, url) = local_relay().await;
        let keys = Keys::generate();
        let pool = RelayPool::new(Some(keys.clone()));
        pool.set_relays(vec![RelayConfig { url, read: true, write: true, api_key: None }]).await.unwrap();
        pool.set_silent(true).await;
        let err = pool.send(Outbound::PublishOwn { event: signed_note(&keys, "x") }).await.unwrap_err();
        assert!(matches!(err, MessengerError::Transport(m) if m.contains("silent")));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(pool.status().await.relays.iter().all(|r| r.state != RelayState::Connected));
        pool.shutdown().await;
    }

    #[tokio::test]
    async fn set_relays_replaces_the_set() {
        let (_r1, u1) = local_relay().await;
        let (_r2, u2) = local_relay().await;
        let pool = RelayPool::new(None);
        pool.set_relays(vec![RelayConfig { url: u1.clone(), read: true, write: true, api_key: None }]).await.unwrap();
        assert_eq!(pool.status().await.relays.len(), 1);
        pool.set_relays(vec![RelayConfig { url: u2.clone(), read: true, write: true, api_key: None }]).await.unwrap();
        let st = pool.status().await;
        assert_eq!(st.relays.len(), 1);
        assert_eq!(st.relays[0].url, u2);
        pool.shutdown().await;
    }

    /// Real-network check against the project test relay. Run by hand:
    /// `source dev-server.env && cargo test -p messenger-transport -- --ignored e2e`
    #[tokio::test]
    #[ignore]
    async fn e2e_connects_to_gated_project_relay() {
        let url = std::env::var("MESSENGER_DEV_RELAY_URL").expect("MESSENGER_DEV_RELAY_URL");
        let key = std::env::var("MESSENGER_DEV_RELAY_API_KEY").expect("MESSENGER_DEV_RELAY_API_KEY");
        let keys = Keys::generate();
        let pool = RelayPool::new(Some(keys.clone()));
        pool.set_relays(vec![RelayConfig {
            url: RelayUrl::parse(&url).unwrap(),
            read: true,
            write: true,
            api_key: Some(key),
        }])
        .await
        .unwrap();
        let mut connected = false;
        for _ in 0..100 {
            if pool.status().await.relays.iter().any(|r| r.state == RelayState::Connected) {
                connected = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(connected, "relay never reached Connected: {:?}", pool.status().await);
        let ack = pool.send(Outbound::PublishOwn { event: signed_note(&keys, "veydan e2e ping") }).await.unwrap();
        assert!(ack.is_delivered(), "relay rejected the note: {ack:?}");
        pool.shutdown().await;
    }

    /// A way to the relay that can be made to swallow everything, both
    /// ways, with the sockets left open: what a phone finds after a sleep.
    async fn black_hole(to: &RelayUrl) -> (RelayUrl, Arc<AtomicBool>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let target = to.as_str().trim_start_matches("ws://").to_string();
        let swallow = Arc::new(AtomicBool::new(false));
        let on = swallow.clone();
        tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                let Ok(server) = tokio::net::TcpStream::connect(&target).await else { continue };
                let (cr, cw) = client.into_split();
                let (sr, sw) = server.into_split();
                for (mut from, mut to) in [(cr, sw), (sr, cw)] {
                    let on = on.clone();
                    tokio::spawn(async move {
                        let mut buf = vec![0u8; 16 * 1024];
                        while let Ok(n) = from.read(&mut buf).await {
                            if n == 0 {
                                break;
                            }
                            if !on.load(Ordering::SeqCst) && to.write_all(&buf[..n]).await.is_err() {
                                break;
                            }
                        }
                    });
                }
            }
        });
        (RelayUrl::parse(&format!("ws://127.0.0.1:{port}")).unwrap(), swallow)
    }

    async fn wait_connected(pool: &RelayPool, secs: u64) -> bool {
        for _ in 0..secs * 10 {
            if pool.status().await.relays.iter().all(|r| r.state == RelayState::Connected) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        false
    }

    /// After a sleep a socket may look open and carry nothing: `revive`
    /// leaves a live one alone, closes a silent one, and the relay is
    /// connected again in seconds, not after minutes of pings.
    #[tokio::test]
    async fn a_silent_connection_is_found_and_made_anew() {
        let (_relay, relay_url) = local_relay().await;
        let (url, swallow) = black_hole(&relay_url).await;
        let pool = RelayPool::new(None);
        pool.set_relays(vec![RelayConfig { url, read: true, write: true, api_key: None }]).await.unwrap();
        assert!(wait_connected(&pool, 5).await);

        assert_eq!(pool.revive(Duration::from_secs(2)).await, 0, "a live connection is left alone");
        assert_eq!(pool.status().await.relays[0].state, RelayState::Connected);

        swallow.store(true, Ordering::SeqCst);
        assert_eq!(pool.revive(Duration::from_secs(1)).await, 1, "a silent one is closed");
        swallow.store(false, Ordering::SeqCst);
        assert!(wait_connected(&pool, RETRY_INTERVAL.as_secs() + 3).await, "and connected again at once");
        assert_eq!(pool.sockets.count(), 1);
        pool.shutdown().await;
    }

    /// `reopen` makes every connection anew: what a change of the way to the
    /// servers needs, without the race of `disconnect` and `connect`.
    #[tokio::test]
    async fn reopen_connects_again() {
        let (_relay, url) = local_relay().await;
        let pool = RelayPool::new(None);
        pool.set_relays(vec![RelayConfig { url, read: true, write: true, api_key: None }]).await.unwrap();
        assert!(wait_connected(&pool, 5).await);
        for _ in 0..3 {
            pool.reopen();
            tokio::time::sleep(Duration::from_millis(200)).await;
            assert!(wait_connected(&pool, RETRY_INTERVAL.as_secs() + 3).await);
        }
        pool.shutdown().await;
    }

    #[test]
    fn connect_url_carries_key_and_core_url_strips_it() {
        let url = RelayUrl::parse("wss://gated.example").unwrap();
        let plain = RelayConfig { url: url.clone(), read: true, write: true, api_key: None };
        assert_eq!(plain.connect_url(), "wss://gated.example");
        let gated = RelayConfig { url, read: true, write: true, api_key: Some("abc123".into()) };
        assert_eq!(gated.connect_url(), "wss://gated.example/?key=abc123");
        let parsed = nostr::types::url::RelayUrl::parse(&gated.connect_url()).unwrap();
        assert_eq!(to_core_url(&parsed).as_str(), "wss://gated.example");
    }

    #[test]
    fn tampered_wire_event_is_rejected() {
        let keys = Keys::generate();
        let mut ev = signed_note(&keys, "a");
        ev.json["content"] = serde_json::Value::String("b".into());
        assert!(RelayPool::parse_event(&ev.json).is_err());
    }
}
