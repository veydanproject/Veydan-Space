// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `RelayPool`: `core::Transport` over `nostr_sdk::Client`.
//!
//! The pool owns the relay connections, forwards every event it sees as a
//! `RawEvent`, and executes `Outbound` requests. It does not interpret
//! events. Silent mode drops all connections and refuses every send.
//!
//! Every relay belongs to a server (`RelayConfig::server_id`, none for the
//! home set), and every request goes by its scope (internal/messenger-wire.md,
//! «Серверы»): `Scope::Own` reaches the relays of the home set,
//! `Scope::Server` exactly the relays of that server. A group's scope is
//! resolved to its server's by the runtime before it gets here; one that
//! was not means the home set, as every scope did before servers. So a
//! private server hears only what names it.
//!
//! A channel of a community (`Scope::Channel`) is on the relays its journal
//! names, which the module of the channels knows and the messenger does
//! not: that module registers [`ChannelRoutes`], and a channel's scope is
//! then the relays of the pool among them, of any server; the events of its
//! subscriptions (ids `channel:…`) and of its catch-up go to the module, not
//! to the stream the ingress reads.

use async_trait::async_trait;
use futures_util::StreamExt;
use messenger_core::traits::{RelayState, RelayStatus, RelayStatusSnapshot};
use messenger_core::{
    Ack, EventId, EventSource, MessengerError, Outbound, PubKey, RawEvent, RelayUrl, Result, Scope, Timestamp,
    Transport,
};
use nostr_sdk::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::sync::mpsc;

/// The prefix of the subscription ids of the channels of a community.
pub const CHANNEL_SUB_PREFIX: &str = "channel:";

/// The relays of a channel by its id.
pub type ChannelRelays = Arc<dyn Fn(&str) -> Vec<RelayUrl> + Send + Sync>;

/// Where the requests and the events of the channels go
/// (internal/communities-stage1.md §6).
#[derive(Clone)]
pub struct ChannelRoutes {
    /// The relays of a channel, as its journal names them: the relays of
    /// the pool among them are its scope. None reaches no relay.
    pub relays: ChannelRelays,
    /// The events of the subscriptions `channel:…` and of the catch-up and
    /// the pages of a channel's scope. Unbounded: the pool's one loop of
    /// notifications carries the messenger's events too, and the module's
    /// catch-up waits for the pool while the module drains this queue, so
    /// a bounded queue that filled would stall both for good.
    pub events: mpsc::UnboundedSender<RawEvent>,
}

/// One for the process: the host makes the pool anew whenever its signer
/// changes, and the module of the channels, another module of the app,
/// cannot see that. A process holds one messenger.
static CHANNEL_ROUTES: RwLock<Option<ChannelRoutes>> = RwLock::new(None);

/// Puts the routes of the channels in place, or takes them away (`None`:
/// a channel's scope is the home set again, its events go to the stream).
pub fn route_channels(routes: Option<ChannelRoutes>) {
    *CHANNEL_ROUTES.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = routes;
}

fn channel_routes() -> Option<ChannelRoutes> {
    CHANNEL_ROUTES.read().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
}

/// Where the events of a subscription go: the channels' own, when routed.
fn channel_sink_of_sub(sub_id: &str) -> Option<mpsc::UnboundedSender<RawEvent>> {
    if sub_id.starts_with(CHANNEL_SUB_PREFIX) {
        channel_routes().map(|r| r.events)
    } else {
        None
    }
}

/// Where what a request brings goes: the ingress's stream, or the
/// channels' own queue, which never makes the pool wait.
enum Sink {
    Stream(mpsc::Sender<RawEvent>),
    Channels(mpsc::UnboundedSender<RawEvent>),
}

impl Sink {
    /// Whether it was taken.
    async fn send(&self, raw: RawEvent) -> bool {
        match self {
            Sink::Stream(tx) => tx.send(raw).await.is_ok(),
            Sink::Channels(tx) => tx.send(raw).is_ok(),
        }
    }
}

/// One relay to keep in the pool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelayConfig {
    pub url: RelayUrl,
    pub read: bool,
    pub write: bool,
    /// Static gate key sent as `?key=` on the connect URL. Never logged.
    pub api_key: Option<String>,
    /// The server the relay belongs to; `None` is the home set.
    pub server_id: Option<String>,
}

/// The server a scope names: `None` for the home set. A group or channel
/// scope that reaches the pool unresolved goes to the home set.
fn server_of(scope: &Scope) -> Option<&str> {
    match scope {
        Scope::Server { id } => Some(id.as_str()),
        Scope::Own | Scope::Group { .. } | Scope::Channel { .. } => None,
    }
}

/// What a request that found no relay of its scope says.
fn none_connected(scope: &Scope) -> MessengerError {
    match server_of(scope) {
        None => MessengerError::Transport("no relay connected".into()),
        Some(_) => MessengerError::Transport("no relay of this server connected".into()),
    }
}

/// One relay of the pool as a scope sees it.
struct Member {
    /// How the pool addresses it: the connect url, with its key.
    address: nostr_sdk::prelude::RelayUrl,
    relay: Relay,
    write: bool,
}

impl Member {
    fn connected(&self) -> bool {
        self.relay.status() == nostr_sdk::relay::RelayStatus::Connected
    }
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
    /// Configured relays, each with its server; nostr-sdk keeps the live
    /// set. What a scope reaches is read from here (`members`).
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

    /// Every event a relay sends, each copy of each relay, goes to the
    /// stream: `nostr-sdk` tells an event once per client
    /// (`ClientNotification::Event`, its tracker of ids), so a copy that
    /// comes by a second relay, or again after the ingress forgot it, would
    /// never reach the ingress — the copy of a group's own server after
    /// one by a wrong relay was dropped, the history of a private server
    /// put in again. The ingress dedups (`msg_events_raw`).
    fn spawn_forwarder(&self) {
        let mut stream = self.client.notifications();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            while let Some(n) = stream.next().await {
                match n {
                    ClientNotification::Message { relay_url, message } => {
                        let RelayMessage::Event { event, subscription_id } = *message else { continue };
                        let Some(raw) = to_raw(&event, EventSource::Relay { url: to_core_url(&relay_url) }) else {
                            continue;
                        };
                        if let Some(channels) = channel_sink_of_sub(subscription_id.as_str()) {
                            // A module that stopped listening loses its own
                            // events; the messenger's go on. Never awaited:
                            // this loop is the messenger's too.
                            let _ = channels.send(raw);
                            continue;
                        }
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

    /// The relays of the pool that `scope` reaches, connected or not: those
    /// configured for its server, by the address the pool knows them under.
    /// A relay the pool holds for a moment only (a peer's inbox relay,
    /// `publish_to_inbox`) belongs to no scope.
    async fn members(&self, scope: &Scope) -> Vec<Member> {
        let wanted: HashMap<String, bool> = match (scope, channel_routes()) {
            (Scope::Channel { id }, Some(routes)) => {
                let of_channel: HashSet<String> = (routes.relays)(id).iter().map(|u| u.as_str().to_string()).collect();
                self.configured()
                    .into_iter()
                    .filter(|c| of_channel.contains(c.url.as_str()))
                    .map(|c| (c.url.as_str().to_string(), c.write))
                    .collect()
            }
            _ => {
                let server = server_of(scope);
                self.configured()
                    .into_iter()
                    .filter(|c| c.server_id.as_deref() == server)
                    .map(|c| (c.url.as_str().to_string(), c.write))
                    .collect()
            }
        };
        self.client
            .relays()
            .await
            .into_iter()
            .filter_map(|(address, relay)| {
                let write = *wanted.get(to_core_url(&address).as_str())?;
                Some(Member { address, relay, write })
            })
            .collect()
    }

    /// Where what a request of `scope` brings goes: the channels' own
    /// events for a channel's scope when they are routed, else the stream.
    fn sink_of(&self, scope: &Scope) -> Sink {
        match (scope, channel_routes()) {
            (Scope::Channel { .. }, Some(routes)) => Sink::Channels(routes.events),
            _ => Sink::Stream(self.tx.clone()),
        }
    }

    /// How many relays of `scope` are connected right now.
    pub async fn connected_in(&self, scope: &Scope) -> usize {
        self.members(scope).await.iter().filter(|m| m.connected()).count()
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

    /// An event that came another way (a push), handed to whoever reads
    /// `events()` as if a relay had sent it: the same dedup, checks and
    /// handlers. Also in silent mode, since nothing goes out.
    pub async fn inject(&self, raw: RawEvent) -> Result<()> {
        self.tx.send(raw).await.map_err(|_| MessengerError::Transport("nobody reads the events".into()))
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
    /// History catch-up. Per connected relay of `scope`: Negentropy against
    /// `local`; when the relay does not speak NIP-77 (or the attempt
    /// fails), a plain REQ with the same filter. Everything found is pushed
    /// onto the event stream with `EventSource::Sync`. Returns how many
    /// events were forwarded.
    async fn reconcile(&self, scope: &Scope, filter: Filter, local: Vec<messenger_core::SyncItem>) -> Result<usize> {
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
        let sink = self.sink_of(scope);
        // Negentropy names what is missing and leaves the events to the
        // stream of the client, which the ingress reads: a channel's are
        // fetched instead, so they reach the channel's own stream.
        let routed = matches!(scope, Scope::Channel { .. }) && channel_routes().is_some();
        for Member { address: url, relay, .. } in self.members(scope).await {
            if relay.status() != nostr_sdk::relay::RelayStatus::Connected {
                continue;
            }
            any_relay = true;
            let source = EventSource::Sync { url: to_core_url(&url) };
            if routed {
                let Ok(events) = relay.fetch_events(filter.clone()).timeout(HISTORY_FETCH_TIMEOUT).await else { continue };
                for ev in events.into_iter().filter(|ev| !known.contains(&ev.id.to_hex())) {
                    if let Some(raw) = to_raw(&ev, source.clone()) {
                        if sink.send(raw).await {
                            forwarded += 1;
                        }
                    }
                }
                continue;
            }
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
                    if sink.send(raw).await {
                        forwarded += 1;
                    }
                }
            }
        }
        if !any_relay {
            return Err(none_connected(scope));
        }
        Ok(forwarded)
    }

    /// Connect urls of the relays of `scope` that are connected and accept
    /// writes.
    async fn connected_write_targets(&self, scope: &Scope) -> Result<Vec<String>> {
        let targets: Vec<String> = self
            .members(scope)
            .await
            .into_iter()
            .filter(|m| m.connected() && m.write)
            .map(|m| m.address.to_string())
            .collect();
        if targets.is_empty() {
            return Err(none_connected(scope));
        }
        Ok(targets)
    }

    /// Deliver to a peer: their inbox relays when we know them (relays of
    /// ours are used directly, foreign ones through a short-lived
    /// connection), otherwise every write relay of the home set.
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
            targets = self.connected_write_targets(&Scope::Own).await.unwrap_or_default();
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

impl RelayPool {
    /// How many relays of the pool are connected right now.
    pub async fn connected(&self) -> usize {
        self.client
            .relays()
            .await
            .values()
            .filter(|relay| relay.status() == nostr_sdk::relay::RelayStatus::Connected)
            .count()
    }

    /// One REQ to the relays of `scope`, answered until each says EOSE or
    /// `patience` runs out: the events found, each checked, as JSON. For a
    /// lookup that wants its answer here and not on the event stream (the
    /// code of a device link, messenger-wire.md §3).
    pub async fn fetch(
        &self,
        filter: &messenger_core::outbound::Filter,
        scope: &Scope,
        patience: Duration,
    ) -> Result<Vec<serde_json::Value>> {
        self.guard_send()?;
        let f = Self::parse_filter(filter)?;
        let members = self.members(scope).await;
        if !members.iter().any(Member::connected) {
            return Err(none_connected(scope));
        }
        let targets: Vec<(nostr_sdk::prelude::RelayUrl, Vec<Filter>)> =
            members.into_iter().map(|m| (m.address, vec![f.clone()])).collect();
        let events = self
            .client
            .fetch_events(ReqTarget::manual(targets))
            .timeout(patience)
            .await
            .map_err(|e| MessengerError::Transport(e.to_string()))?;
        Ok(events
            .iter()
            .filter(|ev| ev.verify().is_ok())
            .filter_map(|ev| serde_json::to_value(ev).ok())
            .collect())
    }
}

/// What one relay answered to a page of the past (`RelayPool::fetch_page`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelayPage {
    pub relay: RelayUrl,
    /// The ids of the events it sent, checked, each once.
    pub ids: Vec<String>,
    /// The oldest `created_at` among them; `None` when it sent nothing.
    pub oldest: Option<i64>,
}

impl RelayPool {
    /// One page of the past: a plain REQ with `filter` (its `until` and
    /// `limit` are what make the page, so no Negentropy) to every connected
    /// relay of `scope`, each answered until EOSE or `patience`. Every
    /// event found goes onto the event stream as history
    /// (`EventSource::Sync` with the relay it came by), exactly as the
    /// catch-up's do, so the ingress takes it the same way. Per relay: what
    /// it sent. A relay that did not answer in time with anything is left
    /// out (a REQ cut by its timeout ends as quietly as one at EOSE, so an
    /// empty answer counts only when it came before `patience` ran out);
    /// none answering is an error, never an empty page.
    pub async fn fetch_page(
        &self,
        filter: &messenger_core::outbound::Filter,
        scope: &Scope,
        patience: Duration,
    ) -> Result<Vec<RelayPage>> {
        self.guard_send()?;
        let f = Self::parse_filter(filter)?;
        let members: Vec<Member> = self.members(scope).await.into_iter().filter(Member::connected).collect();
        if members.is_empty() {
            return Err(none_connected(scope));
        }
        let asks = members.iter().map(|m| {
            let f = f.clone();
            async move {
                let started = tokio::time::Instant::now();
                let answer = m.relay.fetch_events(f).timeout(patience).await;
                (to_core_url(&m.address), answer, started.elapsed() < patience)
            }
        });
        let mut pages = Vec::new();
        let mut forwarded: HashSet<String> = HashSet::new();
        let sink = self.sink_of(scope);
        for (url, answer, in_time) in futures_util::future::join_all(asks).await {
            let Ok(events) = answer else { continue };
            if events.is_empty() && !in_time {
                continue;
            }
            let source = EventSource::Sync { url: url.clone() };
            let mut page = RelayPage { relay: url, ids: Vec::new(), oldest: None };
            for ev in events.into_iter().filter(|ev| ev.verify().is_ok()) {
                let id = ev.id.to_hex();
                let at = ev.created_at.as_secs() as i64;
                page.oldest = Some(page.oldest.map_or(at, |o| o.min(at)));
                // The copy of a second relay adds nothing the ingress has
                // not been given already.
                if forwarded.insert(id.clone()) {
                    if let Some(raw) = to_raw(&ev, source.clone()) {
                        let _ = sink.send(raw).await;
                    }
                }
                page.ids.push(id);
            }
            pages.push(page);
        }
        if pages.is_empty() {
            return Err(MessengerError::Transport("no relay answered".into()));
        }
        Ok(pages)
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
            Outbound::PublishOwn { event } => self.publish(&Scope::Own, &event).await,
            Outbound::PublishScoped { scope, event } => self.publish(&scope, &event).await,
            Outbound::PublishToInbox { event, hint_relays, .. } => {
                let ev = Self::parse_event(&event.json)?;
                self.publish_to_inbox(&ev, &hint_relays).await
            }
            Outbound::Subscribe { id, filter, scope } => {
                let f = Self::parse_filter(&filter)?;
                let id = SubscriptionId::new(id.0);
                // A subscription is sent again to change its filter (a group
                // joined, a contact added). nostr-sdk keeps the first filter
                // of an ID and refuses the next one, reporting it only per
                // relay: without closing the old one first, nothing changes
                // until the app restarts.
                let _ = self.client.unsubscribe(&id).await;
                // Every relay of the scope, connected or not, as a REQ to
                // every relay of the pool went before servers.
                let targets: Vec<(nostr_sdk::prelude::RelayUrl, Vec<Filter>)> =
                    self.members(&scope).await.into_iter().map(|m| (m.address, vec![f.clone()])).collect();
                if targets.is_empty() {
                    return Err(none_connected(&scope));
                }
                let out = self
                    .client
                    .subscribe(ReqTarget::manual(targets))
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
            Outbound::Sync { scope, filter, local } => {
                let f = Self::parse_filter(&filter)?;
                self.reconcile(&scope, f, local).await?;
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

impl RelayPool {
    /// To the connected write relays of `scope`, and to no other.
    async fn publish(&self, scope: &Scope, event: &messenger_core::outbound::WireEvent) -> Result<Ack> {
        let ev = Self::parse_event(&event.json)?;
        let targets = self.connected_write_targets(scope).await?;
        // Not into the client's store: an event found there is not told
        // again, so our own copy coming back on our subscription would never
        // reach ingress and be fetched again by every new session.
        let res = self
            .client
            .send_event(&ev)
            .to(targets)
            .save_into_database(false)
            .await
            .map_err(|e| MessengerError::Transport(e.to_string()))?;
        Ok(ack_from(&res))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::outbound::{Filter as CoreFilter, WireEvent};
    use messenger_core::SubId;
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
            .set_relays(vec![RelayConfig { url: url.clone(), read: true, write: true, api_key: None, server_id: None }])
            .await
            .unwrap();

        let receiver = RelayPool::new(None);
        let mut inbox = receiver.events();
        receiver
            .set_relays(vec![RelayConfig { url: url.clone(), read: true, write: true, api_key: None, server_id: None }])
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
        pool.set_relays(vec![RelayConfig { url, read: true, write: true, api_key: None, server_id: None }]).await.unwrap();
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
        let config = || vec![RelayConfig { url: url.clone(), read: true, write: true, api_key: None, server_id: None }];

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
        pool.set_relays(vec![RelayConfig { url, read: true, write: true, api_key: None, server_id: None }]).await.unwrap();
        pool.set_silent(true).await;
        let err = pool.send(Outbound::PublishOwn { event: signed_note(&keys, "x") }).await.unwrap_err();
        assert!(matches!(err, MessengerError::Transport(m) if m.contains("silent")));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(pool.status().await.relays.iter().all(|r| r.state != RelayState::Connected));
        pool.shutdown().await;
    }

    fn home(url: &RelayUrl) -> RelayConfig {
        RelayConfig { url: url.clone(), read: true, write: true, api_key: None, server_id: None }
    }

    fn of_server(url: &RelayUrl, id: &str) -> RelayConfig {
        RelayConfig { server_id: Some(id.into()), ..home(url) }
    }

    fn server(id: &str) -> Scope {
        Scope::Server { id: id.into() }
    }

    /// A pool on `relays`, connected to all of them.
    async fn connected_pool(keys: Option<Keys>, relays: Vec<RelayConfig>) -> RelayPool {
        let pool = RelayPool::new(keys);
        pool.set_relays(relays).await.unwrap();
        assert!(wait_connected(&pool, 5).await);
        pool
    }

    /// What one relay holds of `author`'s notes, read by a pool of its own.
    async fn notes_on(url: &RelayUrl, author: &Keys) -> Vec<String> {
        let reader = connected_pool(None, vec![home(url)]).await;
        let filter = CoreFilter(serde_json::json!({ "kinds": [1], "authors": [author.public_key().to_hex()] }));
        let found = reader.fetch(&filter, &Scope::Own, Duration::from_secs(3)).await.unwrap();
        reader.shutdown().await;
        let mut texts: Vec<String> = found.iter().filter_map(|j| j["content"].as_str().map(String::from)).collect();
        texts.sort();
        texts
    }

    /// The home set and a private server in one pool: what is scoped to the
    /// server lands on its relay only, what goes to the home set never
    /// reaches it, and a server the pool has no relay of refuses.
    #[tokio::test]
    async fn a_publish_reaches_the_relays_of_its_scope_and_no_other() {
        let (_a, home_url) = local_relay().await;
        let (_b, private_url) = local_relay().await;
        let alice = Keys::generate();
        let pool = connected_pool(Some(alice.clone()), vec![home(&home_url), of_server(&private_url, "b")]).await;

        let ack = pool.send(Outbound::PublishScoped { scope: server("b"), event: signed_note(&alice, "to b") }).await.unwrap();
        assert_eq!(ack.accepted_by, vec![private_url.clone()], "{ack:?}");
        let ack = pool.send(Outbound::PublishOwn { event: signed_note(&alice, "home own") }).await.unwrap();
        assert_eq!(ack.accepted_by, vec![home_url.clone()], "{ack:?}");
        let ack = pool.send(Outbound::PublishScoped { scope: Scope::Own, event: signed_note(&alice, "home scoped") }).await.unwrap();
        assert_eq!(ack.accepted_by, vec![home_url.clone()], "{ack:?}");
        // A group the runtime did not resolve goes where it went before servers.
        let group = Scope::Group { id: "g".repeat(64) };
        let ack = pool.send(Outbound::PublishScoped { scope: group, event: signed_note(&alice, "home group") }).await.unwrap();
        assert_eq!(ack.accepted_by, vec![home_url.clone()], "{ack:?}");
        let err = pool.send(Outbound::PublishScoped { scope: server("c"), event: signed_note(&alice, "to c") }).await.unwrap_err();
        assert!(matches!(err, MessengerError::Transport(_)), "{err:?}");

        assert_eq!(notes_on(&private_url, &alice).await, vec!["to b".to_string()]);
        assert_eq!(notes_on(&home_url, &alice).await, vec!["home group", "home own", "home scoped"]);
        assert_eq!(pool.connected_in(&Scope::Own).await, 1);
        assert_eq!(pool.connected_in(&server("b")).await, 1);
        assert_eq!(pool.connected_in(&server("c")).await, 0);
        pool.shutdown().await;
    }

    /// A subscription scoped to a server hears that server's relay alone,
    /// one of the home set the home relays alone; a fetch and a sync the same.
    #[tokio::test]
    async fn a_subscription_hears_the_relays_of_its_scope_only() {
        let (_a, home_url) = local_relay().await;
        let (_b, private_url) = local_relay().await;
        let alice = Keys::generate();
        let to_home = connected_pool(Some(alice.clone()), vec![home(&home_url)]).await;
        let to_private = connected_pool(Some(alice.clone()), vec![home(&private_url)]).await;
        let receiver = RelayPool::new(None);
        let mut inbox = receiver.events();
        receiver.set_relays(vec![home(&home_url), of_server(&private_url, "b")]).await.unwrap();
        assert!(wait_connected(&receiver, 5).await);

        let by_alice = || CoreFilter(serde_json::json!({ "kinds": [1], "authors": [alice.public_key().to_hex()] }));
        receiver.send(Outbound::Subscribe { id: SubId("only-b".into()), filter: by_alice(), scope: server("b") }).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(to_home.send(Outbound::PublishOwn { event: signed_note(&alice, "on home") }).await.unwrap().is_delivered());
        assert!(to_private.send(Outbound::PublishOwn { event: signed_note(&alice, "on b") }).await.unwrap().is_delivered());
        let got = tokio::time::timeout(Duration::from_secs(5), inbox.recv()).await.expect("b's note arrives").unwrap();
        assert_eq!(got.json["content"], "on b");
        assert_eq!(got.source, EventSource::Relay { url: private_url.clone() });
        assert!(tokio::time::timeout(Duration::from_millis(700), inbox.recv()).await.is_err(), "nothing of the home relay");

        // The home scope under its own id: the home relay's note, not b's.
        receiver.send(Outbound::Unsubscribe { id: SubId("only-b".into()) }).await.unwrap();
        receiver.send(Outbound::Subscribe { id: SubId("home".into()), filter: by_alice(), scope: Scope::Own }).await.unwrap();
        let got = tokio::time::timeout(Duration::from_secs(5), inbox.recv()).await.expect("the home note arrives").unwrap();
        assert_eq!(got.json["content"], "on home");
        assert!(tokio::time::timeout(Duration::from_millis(700), inbox.recv()).await.is_err(), "nothing of b");

        let fetched = receiver.fetch(&by_alice(), &server("b"), Duration::from_secs(3)).await.unwrap();
        assert_eq!(fetched.iter().map(|j| j["content"].as_str().unwrap()).collect::<Vec<_>>(), vec!["on b"]);
        assert!(receiver.fetch(&by_alice(), &server("c"), Duration::from_secs(1)).await.is_err());

        // A sync, by a pool that has seen nothing yet.
        let fresh = RelayPool::new(None);
        let mut synced = fresh.events();
        fresh.set_relays(vec![home(&home_url), of_server(&private_url, "b")]).await.unwrap();
        assert!(wait_connected(&fresh, 5).await);
        fresh.send(Outbound::Sync { scope: server("b"), filter: by_alice(), local: vec![] }).await.unwrap();
        // What the reconciliation fetched comes as the relay sent it (and
        // as found by the sync, when the relay speaks NIP-77): all of it b's.
        let mut seen = Vec::new();
        while let Ok(Some(got)) = tokio::time::timeout(Duration::from_millis(1500), synced.recv()).await {
            seen.push((got.json["content"].as_str().unwrap_or_default().to_string(), got.source));
        }
        assert!(!seen.is_empty(), "the sync brings b's note");
        let from_b = |s: &EventSource| matches!(s, EventSource::Relay { url } | EventSource::Sync { url } if *url == private_url);
        assert!(seen.iter().all(|(text, s)| text == "on b" && from_b(s)), "nothing of the home relay: {seen:?}");

        for pool in [to_home, to_private, receiver, fresh] {
            pool.shutdown().await;
        }
    }

    /// A channel's scope is the relays its routes name, of whichever
    /// server, and what its subscriptions and its catch-up bring goes to
    /// the routes' stream, not to the ingress's; without routes it is the
    /// home set again. The routes are the process's: no other test names
    /// a channel.
    #[tokio::test]
    async fn a_channel_reaches_the_relays_of_its_routes_and_is_heard_on_their_stream() {
        let (_a, home_url) = local_relay().await;
        let (_b, channel_url) = local_relay().await;
        let alice = Keys::generate();
        let pool = connected_pool(Some(alice.clone()), vec![home(&home_url), of_server(&channel_url, "b")]).await;
        let mut ingress = pool.events();
        let (tx, mut channels) = mpsc::unbounded_channel();
        let cid = "c".repeat(64);
        let (of_cid, url) = (cid.clone(), channel_url.clone());
        let relays = move |id: &str| if id == of_cid { vec![url.clone()] } else { Vec::new() };
        route_channels(Some(ChannelRoutes { relays: Arc::new(relays), events: tx }));
        let channel = Scope::Channel { id: cid.clone() };

        let ack = pool.send(Outbound::PublishScoped { scope: channel.clone(), event: signed_note(&alice, "in the channel") }).await.unwrap();
        assert_eq!(ack.accepted_by, vec![channel_url.clone()], "{ack:?}");
        let unknown = Scope::Channel { id: "d".repeat(64) };
        let err = pool.send(Outbound::PublishScoped { scope: unknown, event: signed_note(&alice, "nowhere") }).await.unwrap_err();
        assert!(matches!(err, MessengerError::Transport(_)), "{err:?}");

        let by_alice = || CoreFilter(serde_json::json!({ "kinds": [1], "authors": [alice.public_key().to_hex()] }));
        let sub = SubId(format!("{CHANNEL_SUB_PREFIX}{cid}:feed"));
        pool.send(Outbound::Subscribe { id: sub, filter: by_alice(), scope: channel.clone() }).await.unwrap();
        let got = tokio::time::timeout(Duration::from_secs(5), channels.recv()).await.expect("the channel hears it").unwrap();
        assert_eq!(got.json["content"], "in the channel");
        assert_eq!(got.source, EventSource::Relay { url: channel_url.clone() });
        assert!(tokio::time::timeout(Duration::from_millis(700), ingress.recv()).await.is_err(), "nothing of the channel reaches the ingress");

        // A catch-up, by a pool that has seen nothing yet: the routes are
        // the process's, and every pool follows them.
        let fresh = connected_pool(None, vec![home(&home_url), of_server(&channel_url, "b")]).await;
        fresh.send(Outbound::Sync { scope: channel.clone(), filter: by_alice(), local: vec![] }).await.unwrap();
        fresh.shutdown().await;
        let mut synced = false;
        while let Ok(Some(got)) = tokio::time::timeout(Duration::from_millis(1500), channels.recv()).await {
            assert_eq!(got.json["content"], "in the channel");
            synced |= got.source == (EventSource::Sync { url: channel_url.clone() });
        }
        assert!(synced, "the catch-up of the channel comes to its stream");

        // A backlog nobody reads yet: the live subscription and a catch-up
        // hand it over without waiting for the module (ревью модуля п. 1).
        const BACKLOG: usize = 120;
        for i in 0..BACKLOG {
            pool.send(Outbound::PublishScoped { scope: channel.clone(), event: signed_note(&alice, &format!("backlog {i}")) }).await.unwrap();
        }
        let fresh = connected_pool(None, vec![home(&home_url), of_server(&channel_url, "b")]).await;
        let caught = tokio::time::timeout(Duration::from_secs(10), fresh.send(Outbound::Sync { scope: channel.clone(), filter: by_alice(), local: vec![] })).await;
        assert!(matches!(caught, Ok(Ok(_))), "the catch-up does not wait for the module: {caught:?}");
        fresh.shutdown().await;
        let mut held = 0;
        while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(1500), channels.recv()).await {
            held += 1;
        }
        assert!(held > BACKLOG, "the live copies and the catch-up are all held: {held}");

        route_channels(None);
        let ack = pool.send(Outbound::PublishScoped { scope: channel, event: signed_note(&alice, "home again") }).await.unwrap();
        assert_eq!(ack.accepted_by, vec![home_url.clone()], "{ack:?}");
        pool.shutdown().await;
    }

    /// Each relay's copy of an event reaches the stream, and a copy sent
    /// again (a second subscription) too: `nostr-sdk` tells an event once
    /// per client, and the ingress, which dedups, must still see the copy
    /// of the right relay after one by a wrong one, and a history it forgot.
    #[tokio::test]
    async fn every_copy_of_every_relay_reaches_the_stream() {
        let (_a, url_a) = local_relay().await;
        let (_b, url_b) = local_relay().await;
        let alice = Keys::generate();
        let writer = connected_pool(Some(alice.clone()), vec![home(&url_a), home(&url_b)]).await;
        let note = signed_note(&alice, "on both");
        let ack = writer.send(Outbound::PublishOwn { event: note.clone() }).await.unwrap();
        assert_eq!(ack.accepted_by.len(), 2, "{ack:?}");

        let receiver = RelayPool::new(None);
        let mut inbox = receiver.events();
        receiver.set_relays(vec![home(&url_a), of_server(&url_b, "b")]).await.unwrap();
        assert!(wait_connected(&receiver, 5).await);
        let by_alice = || CoreFilter(serde_json::json!({ "kinds": [1], "authors": [alice.public_key().to_hex()] }));
        async fn copies(inbox: &mut mpsc::Receiver<RawEvent>) -> Vec<(String, EventSource)> {
            let mut got = Vec::new();
            while let Ok(Some(raw)) = tokio::time::timeout(Duration::from_millis(1500), inbox.recv()).await {
                got.push((raw.id.as_hex().to_string(), raw.source));
            }
            got
        }
        receiver.send(Outbound::Subscribe { id: SubId("a".into()), filter: by_alice(), scope: Scope::Own }).await.unwrap();
        let first = copies(&mut inbox).await;
        assert_eq!(first, vec![(note.id.as_hex().to_string(), EventSource::Relay { url: url_a.clone() })]);
        // The same event by the second relay: told again, by that relay.
        receiver.send(Outbound::Subscribe { id: SubId("b".into()), filter: by_alice(), scope: server("b") }).await.unwrap();
        let second = copies(&mut inbox).await;
        assert_eq!(second, vec![(note.id.as_hex().to_string(), EventSource::Relay { url: url_b.clone() })]);
        // And asked again of the first one, again.
        receiver.send(Outbound::Unsubscribe { id: SubId("a".into()) }).await.unwrap();
        receiver.send(Outbound::Subscribe { id: SubId("a2".into()), filter: by_alice(), scope: Scope::Own }).await.unwrap();
        let third = copies(&mut inbox).await;
        assert_eq!(third, vec![(note.id.as_hex().to_string(), EventSource::Relay { url: url_a.clone() })]);
        for pool in [writer, receiver] {
            pool.shutdown().await;
        }
    }

    /// A page of the past: the newest `limit` at or before `until` of the
    /// relays of the scope alone, told per relay with its oldest time, and
    /// put on the stream as history of that relay.
    #[tokio::test]
    async fn a_page_of_the_past_comes_from_the_relays_of_its_scope() {
        let (_a, home_url) = local_relay().await;
        let (_b, private_url) = local_relay().await;
        let alice = Keys::generate();
        let note_at = |text: &str, at: u64| {
            let ev = EventBuilder::new(Kind::from(1u16), text).custom_created_at(nostr::types::Timestamp::from_secs(at)).finalize(&alice).unwrap();
            WireEvent { id: EventId::parse(&ev.id.to_hex()).unwrap(), json: serde_json::to_value(&ev).unwrap() }
        };
        let to_home = connected_pool(Some(alice.clone()), vec![home(&home_url)]).await;
        let to_private = connected_pool(Some(alice.clone()), vec![home(&private_url)]).await;
        to_home.send(Outbound::PublishOwn { event: note_at("home", 1_500) }).await.unwrap();
        let mut on_b = Vec::new();
        for at in [1_000u64, 1_100, 1_200, 1_300] {
            let note = note_at(&format!("b {at}"), at);
            on_b.push((at, note.id.as_hex().to_string()));
            to_private.send(Outbound::PublishOwn { event: note }).await.unwrap();
        }

        let receiver = RelayPool::new(None);
        let mut inbox = receiver.events();
        receiver.set_relays(vec![home(&home_url), of_server(&private_url, "b")]).await.unwrap();
        assert!(wait_connected(&receiver, 5).await);
        let window = |until: i64| {
            CoreFilter(serde_json::json!({ "kinds": [1], "authors": [alice.public_key().to_hex()], "until": until, "limit": 2 }))
        };
        let pages = receiver.fetch_page(&window(1_250), &server("b"), Duration::from_secs(3)).await.unwrap();
        assert_eq!(pages.len(), 1, "{pages:?}");
        assert_eq!(pages[0].relay, private_url);
        assert_eq!(pages[0].oldest, Some(1_100), "the newest two at or before 1250");
        let mut ids = pages[0].ids.clone();
        ids.sort();
        let mut want = vec![on_b[1].1.clone(), on_b[2].1.clone()];
        want.sort();
        assert_eq!(ids, want);
        let mut synced = Vec::new();
        while let Ok(Some(raw)) = tokio::time::timeout(Duration::from_millis(1000), inbox.recv()).await {
            if raw.source == (EventSource::Sync { url: private_url.clone() }) {
                synced.push(raw.id.as_hex().to_string());
            }
        }
        synced.sort();
        assert_eq!(synced, want, "on the stream as history of b");

        let below = receiver.fetch_page(&window(999), &server("b"), Duration::from_secs(3)).await.unwrap();
        assert_eq!((below[0].ids.len(), below[0].oldest), (0, None), "nothing older: an empty page, not an error");
        assert!(receiver.fetch_page(&window(2_000), &server("c"), Duration::from_secs(1)).await.is_err());
        for pool in [to_home, to_private, receiver] {
            pool.shutdown().await;
        }
    }

    #[tokio::test]
    async fn set_relays_replaces_the_set() {
        let (_r1, u1) = local_relay().await;
        let (_r2, u2) = local_relay().await;
        let pool = RelayPool::new(None);
        pool.set_relays(vec![RelayConfig { url: u1.clone(), read: true, write: true, api_key: None, server_id: None }]).await.unwrap();
        assert_eq!(pool.status().await.relays.len(), 1);
        pool.set_relays(vec![RelayConfig { url: u2.clone(), read: true, write: true, api_key: None, server_id: None }]).await.unwrap();
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
            server_id: None,
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
        pool.set_relays(vec![RelayConfig { url, read: true, write: true, api_key: None, server_id: None }]).await.unwrap();
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
        pool.set_relays(vec![RelayConfig { url, read: true, write: true, api_key: None, server_id: None }]).await.unwrap();
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
        let plain = RelayConfig { url: url.clone(), read: true, write: true, api_key: None, server_id: None };
        assert_eq!(plain.connect_url(), "wss://gated.example");
        let gated = RelayConfig { url, read: true, write: true, api_key: Some("abc123".into()), server_id: None };
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
