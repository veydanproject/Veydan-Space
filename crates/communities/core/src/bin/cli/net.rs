// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The relays of the trial: a few connections (`conn`) shared by the
//! bots of the process. A relay of the network takes at most 20
//! connections from one address (nginx `limit_conn`), so a thousand bots
//! on one machine cannot each hold their own; a real reader would.
//!
//! What anybody may read (journal, card, epochs, snapshots, the feed) is
//! one live subscription per part of a channel on the shared pools,
//! handed to every bot that asked for that part. Gift wraps are not: the
//! relay's guard (vguard) answers a REQ for kind 1059 only when every
//! `#p` is the key the connection signed in with, one key per
//! connection. A bot reads its notes through an inbox of its own, a pool
//! signed in with its key, opened while it waits for a note (a join) or
//! for good (the owner, the keeper key, the admin).
//!
//! A `Sync` of a bot is one REQ answered to that bot only; the same
//! filter asked again within `CACHE` is answered from the last REQ (a
//! thousand bots catching up the same journal at once).

use crate::store::Counters;
use async_trait::async_trait;
use messenger_core::outbound::WireEvent;
use messenger_core::traits::RelayStatusSnapshot;
use crate::conn::Conn;
use messenger_core::{Ack, MessengerError, Outbound, RawEvent, Result, Transport};
use nostr::key::Keys;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;

pub type BotId = usize;

/// The answer to a filter: when, from which `since`, the events.
type Answer = (Instant, i64, Arc<Vec<Value>>);

/// An event for a bot: live, or history of a catch-up.
pub struct Item {
    pub json: Value,
    pub live: bool,
}

/// How long the answer to a filter serves the next bot that asks it.
const CACHE: Duration = Duration::from_secs(3);
/// How long a REQ waits for its EOSE.
const PATIENCE: Duration = Duration::from_secs(20);
/// NIP-59 dates a wrap up to two days back.
const WRAP_BACK: i64 = 2 * 24 * 3600 + 600;
/// New connections a second from this address (nginx: 10 r/s, burst 20).
const CONNECTS_PER_SEC: f64 = 6.0;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn tag(json: &Value, name: &str) -> Option<String> {
    json["tags"].as_array()?.iter().find(|t| t[0] == name).and_then(|t| t[1].as_str()).map(str::to_string)
}

/// The channel and the part an event belongs to, as the core's
/// subscriptions name them (`transport::filters`).
fn part_of(json: &Value) -> Option<(String, &'static str)> {
    use communities_core::wire::{KIND_CARD, KIND_CONTENT, KIND_EPOCH, KIND_JOURNAL, KIND_SNAPSHOT};
    let kind = u16::try_from(json["kind"].as_u64()?).ok()?;
    Some(match kind {
        KIND_JOURNAL => (tag(json, "h")?, "journal"),
        KIND_CARD => (tag(json, "d")?, "card"),
        KIND_EPOCH => (tag(json, "h")?, "epochs"),
        KIND_SNAPSHOT => (tag(json, "h")?, "snapshots"),
        KIND_CONTENT => (tag(json, "h")?, "feed"),
        _ => return None,
    })
}

#[derive(Default)]
struct Routes {
    parts: HashMap<(String, String), BTreeSet<BotId>>,
    inbox: HashMap<String, BTreeSet<BotId>>,
}

pub struct Hub {
    target: Target,
    pools: Vec<Arc<Conn>>,
    /// The inboxes: a pool signed in with each key.
    inboxes: tokio::sync::Mutex<HashMap<String, Arc<Conn>>>,
    connects: tokio::sync::Mutex<Instant>,
    rr: AtomicUsize,
    routes: RwLock<Routes>,
    sinks: RwLock<HashMap<BotId, mpsc::UnboundedSender<Item>>>,
    offline: RwLock<HashSet<BotId>>,
    channel_subs: Mutex<HashSet<(String, String)>>,
    cache: Mutex<HashMap<String, Answer>>,
    fetching: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Wraps already handed to a bot (an inbox opened again brings two
    /// days of them back).
    seen_wraps: Mutex<HashSet<(BotId, String)>>,
    next_id: AtomicUsize,
    pub stats: Counters,
}

/// Where the trial goes: the relay (with its key), the blob hosts.
#[derive(Clone, Debug)]
pub struct Target {
    pub relay: String,
    pub key: Option<String>,
    pub storage: Vec<String>,
}

impl Hub {
    /// `n` shared connections to the relay.
    pub async fn connect(target: &Target, n: usize) -> Result<Arc<Self>> {
        let hub = Arc::new(Self {
            target: target.clone(),
            pools: Vec::new(),
            inboxes: tokio::sync::Mutex::default(),
            connects: tokio::sync::Mutex::new(Instant::now()),
            rr: AtomicUsize::new(0),
            routes: RwLock::default(),
            sinks: RwLock::default(),
            offline: RwLock::default(),
            channel_subs: Mutex::default(),
            cache: Mutex::default(),
            fetching: Mutex::default(),
            seen_wraps: Mutex::default(),
            next_id: AtomicUsize::new(0),
            stats: Counters::default(),
        });
        let mut pools = Vec::new();
        for _ in 0..n.max(1) {
            pools.push(hub.open_pool(None).await?);
        }
        let mut hub = hub;
        match Arc::get_mut(&mut hub) {
            Some(h) => h.pools = pools,
            None => return Err(MessengerError::Other("hub".into())),
        }
        for pool in &hub.pools {
            hub.forward(pool);
        }
        Ok(hub)
    }

    /// One connection, signed in with `signer` when given, paced.
    async fn open_pool(&self, signer: Option<Keys>) -> Result<Arc<Conn>> {
        let at = {
            let mut next = self.connects.lock().await;
            let at = (*next).max(Instant::now());
            *next = at + Duration::from_secs_f64(1.0 / CONNECTS_PER_SEC);
            at
        };
        tokio::time::sleep_until(at).await;
        self.stats.add("connect", 1);
        match Conn::open(&self.target.relay, self.target.key.as_deref(), signer, Duration::from_secs(30)).await {
            Ok(c) => Ok(Arc::new(c)),
            Err(e) => {
                self.stats.add("connect_failed", 1);
                Err(e)
            }
        }
    }

    /// The events of a pool go to the bots.
    fn forward(self: &Arc<Self>, pool: &Arc<Conn>) {
        let h = Arc::downgrade(self);
        pool.forward(&["c:", "w:"], move |json| {
            let Some(hub) = h.upgrade() else { return false };
            hub.dispatch(&json);
            true
        });
    }

    pub async fn shutdown(&self) {
        for p in self.inboxes.lock().await.drain().map(|(_, p)| p) {
            p.shutdown().await;
        }
        for p in &self.pools {
            p.shutdown().await;
        }
    }

    fn pool(&self) -> &Arc<Conn> {
        let i = self.rr.fetch_add(1, Ordering::Relaxed) % self.pools.len();
        &self.pools[i]
    }

    /// A new bot: its id and the stream of what comes to it.
    pub fn register(&self) -> (BotId, mpsc::UnboundedReceiver<Item>) {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::unbounded_channel();
        self.sinks.write().unwrap_or_else(|e| e.into_inner()).insert(id, tx);
        (id, rx)
    }

    pub fn unregister(&self, id: BotId) {
        self.sinks.write().unwrap_or_else(|e| e.into_inner()).remove(&id);
    }

    /// A bot off the network (`true`) or back on.
    pub fn set_offline(&self, id: BotId, off: bool) {
        let mut o = self.offline.write().unwrap_or_else(|e| e.into_inner());
        if off {
            o.insert(id);
        } else {
            o.remove(&id);
        }
    }

    pub fn is_offline(&self, id: BotId) -> bool {
        self.offline.read().unwrap_or_else(|e| e.into_inner()).contains(&id)
    }

    fn send_to(&self, id: BotId, json: &Value, live: bool) {
        if self.is_offline(id) {
            self.stats.add("dropped_offline", 1);
            return;
        }
        // Live only: a catch-up hands its page whole, as the pool of the
        // messenger does; the core counts what it heard of a page to ask
        // for the one before it (wire §7), and a page thinned here looked
        // short, so the notes past the newest 500 were never asked for
        // (scenario d: 8 of 20 joins sent while the keeper was off).
        if live && json["kind"].as_u64() == Some(u64::from(communities_core::notes::KIND_WRAP)) {
            let wid = json["id"].as_str().unwrap_or("").to_string();
            if !lock(&self.seen_wraps).insert((id, wid)) {
                return;
            }
        }
        if let Some(tx) = self.sinks.read().unwrap_or_else(|e| e.into_inner()).get(&id) {
            let _ = tx.send(Item { json: json.clone(), live });
        }
    }

    /// An event of a live subscription.
    fn dispatch(&self, json: &Value) {
        self.stats.add("recv", 1);
        self.stats.add("recv_bytes", json.to_string().len() as u64);
        let targets: Vec<BotId> = {
            let routes = self.routes.read().unwrap_or_else(|e| e.into_inner());
            if json["kind"].as_u64() == Some(u64::from(communities_core::notes::KIND_WRAP)) {
                self.stats.add("recv_wrap", 1);
                tag(json, "p").and_then(|p| routes.inbox.get(&p).cloned()).map(|s| s.into_iter().collect()).unwrap_or_default()
            } else if let Some((cid, part)) = part_of(json) {
                routes.parts.get(&(cid, part.to_string())).map(|s| s.iter().copied().collect()).unwrap_or_default()
            } else {
                vec![]
            }
        };
        for id in targets {
            self.send_to(id, json, true);
        }
    }

    /// Opens the inbox of `keys` for `bot`: a connection signed in with
    /// them, subscribed to their wraps of the last two days.
    pub async fn open_inbox(self: &Arc<Self>, bot: BotId, keys: &Keys) -> Result<()> {
        let pk = keys.public_key().to_hex();
        self.routes.write().unwrap_or_else(|e| e.into_inner()).inbox.entry(pk.clone()).or_default().insert(bot);
        if self.inboxes.lock().await.contains_key(&pk) {
            return Ok(());
        }
        let pool = self.open_pool(Some(keys.clone())).await?;
        self.forward(&pool);
        let filter = serde_json::json!({ "kinds": [communities_core::notes::KIND_WRAP], "#p": [pk], "since": now_secs() - WRAP_BACK });
        self.stats.add("sub", 1);
        pool.subscribe(&format!("w:{}", &pk[..12]), &filter).await?;
        // Opened twice at once (a join and a resubscribe): one stays.
        let mut open = self.inboxes.lock().await;
        if open.contains_key(&pk) {
            drop(open);
            pool.shutdown().await;
            return Ok(());
        }
        open.insert(pk, pool);
        Ok(())
    }

    /// The inbox connections open now.
    pub async fn inboxes_open(&self) -> usize {
        self.inboxes.lock().await.len()
    }

    /// Closes the inbox of a key (its connection goes back to the budget).
    pub async fn close_inbox(&self, pubkey: &str) {
        let pool = self.inboxes.lock().await.remove(pubkey);
        if let Some(p) = pool {
            p.shutdown().await;
        }
    }

    async fn subscribe(&self, bot: BotId, sub: &str, filter: &Value) {
        // The id names the channel by a digest (`transport::sub_id`), its
        // filter by `#h` (`#d` of a card, a heartbeat).
        let mut it = sub.splitn(3, ':');
        let (Some("channel"), Some(_), Some(part)) = (it.next(), it.next(), it.next()) else { return };
        let Some(cid) = ["#h", "#d"].iter().find_map(|t| filter[*t].get(0).and_then(Value::as_str)) else { return };
        if part.starts_with("notes") {
            // Each key's inbox subscribes to its own wraps.
            self.stats.add("notes_sub_skipped", 1);
            return;
        }
        self.routes.write().unwrap_or_else(|e| e.into_inner()).parts.entry((cid.to_string(), part.to_string())).or_default().insert(bot);
        let n = {
            let mut subs = lock(&self.channel_subs);
            subs.insert((cid.to_string(), part.to_string())).then_some(subs.len())
        };
        if let Some(n) = n {
            let pool = &self.pools[n % self.pools.len()];
            self.stats.add("sub", 1);
            let id = format!("c:{}:{part}", &cid[..cid.len().min(12)]);
            if let Err(e) = pool.subscribe(&id, filter).await {
                eprintln!("subscription {part}: {e}");
            }
        }
    }

    /// One REQ for a bot's `Sync`, or the answer of one made for the same
    /// filter a moment ago.
    async fn fetch(&self, filter: &Value) -> Result<Arc<Vec<Value>>> {
        self.stats.add("sync_asked", 1);
        let since = filter["since"].as_i64().unwrap_or(0);
        let mut key = filter.clone();
        if let Some(o) = key.as_object_mut() {
            o.remove("since");
        }
        let key = key.to_string();
        let gate = lock(&self.fetching).entry(key.clone()).or_default().clone();
        let _g = gate.lock().await;
        if let Some((at, s, evs)) = lock(&self.cache).get(&key) {
            if at.elapsed() < CACHE && *s <= since {
                self.stats.add("sync_cached", 1);
                return Ok(evs.clone());
            }
        }
        self.stats.add("req", 1);
        let evs = Arc::new(self.pool().fetch(filter, PATIENCE).await?);
        self.note_answer(evs.len());
        lock(&self.cache).insert(key, (Instant::now(), since, evs.clone()));
        Ok(evs)
    }

    fn note_answer(&self, n: usize) {
        self.stats.add("req_events", n as u64);
        if n >= 500 {
            self.stats.add("req_capped_500", 1);
        }
    }

    /// The catch-up of notes: each key of `#p` through its own inbox; a
    /// key without one is not asked (the relay would refuse it).
    async fn fetch_notes(&self, filter: &Value) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        for p in filter["#p"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            let pool = self.inboxes.lock().await.get(p).cloned();
            let Some(pool) = pool else {
                self.stats.add("notes_sync_no_inbox", 1);
                continue;
            };
            let mut f = filter.clone();
            f["#p"] = serde_json::json!([p]);
            self.stats.add("req", 1);
            let evs = pool.fetch(&f, PATIENCE).await?;
            self.note_answer(evs.len());
            out.extend(evs);
        }
        Ok(out)
    }

    /// A REQ of the trial's own (counting what the relay holds).
    pub async fn fetch_plain(&self, filter: &Value) -> Result<Vec<Value>> {
        self.stats.add("req", 1);
        self.pool().fetch(filter, PATIENCE).await
    }

    async fn publish(&self, event: WireEvent) -> Result<Ack> {
        let kind = event.json["kind"].as_u64().unwrap_or(0);
        let bytes = event.json.to_string().len() as u64;
        let ack = self.pool().publish(&event.json).await;
        match &ack {
            Ok(a) if a.is_delivered() => {
                self.stats.add(&format!("pub_{kind}"), 1);
                self.stats.add("pub_bytes", bytes);
            }
            Ok(a) => {
                self.stats.add("pub_refused", 1);
                eprintln!("publish {kind} refused: {:?}", a.rejected_by.iter().map(|(_, why)| why).collect::<Vec<_>>());
            }
            Err(e) => {
                self.stats.add("pub_err", 1);
                eprintln!("publish {kind}: {e}");
            }
        }
        ack
    }

    fn route_part(&self, bot: BotId, cid: String, part: &str) {
        self.routes.write().unwrap_or_else(|e| e.into_inner()).parts.entry((cid, part.to_string())).or_default().insert(bot);
    }
}

/// The transport of one bot over the hub.
pub struct BotNet {
    pub hub: Arc<Hub>,
    pub id: BotId,
}

#[async_trait]
impl Transport for BotNet {
    async fn send(&self, out: Outbound) -> Result<Ack> {
        let empty = Ack { accepted_by: vec![], rejected_by: vec![] };
        if self.hub.is_offline(self.id) {
            return Err(MessengerError::Transport("offline".into()));
        }
        match out {
            Outbound::PublishScoped { event, .. } | Outbound::PublishOwn { event } | Outbound::PublishToInbox { event, .. } => self.hub.publish(event).await,
            Outbound::Subscribe { id, filter, .. } => {
                self.hub.subscribe(self.id, &id.0, &filter.0).await;
                Ok(empty)
            }
            Outbound::Unsubscribe { .. } => Ok(empty),
            Outbound::Sync { filter, local, .. } => {
                let notes = filter.0["kinds"].as_array().is_some_and(|k| k.iter().any(|k| k.as_u64() == Some(u64::from(communities_core::notes::KIND_WRAP))));
                let evs: Arc<Vec<Value>> = if notes {
                    Arc::new(self.hub.fetch_notes(&filter.0).await?)
                } else {
                    // What a bot asks history of, it hears live too.
                    if let Some((cid, part)) = sync_part(&filter.0) {
                        self.hub.route_part(self.id, cid, part);
                    }
                    self.hub.fetch(&filter.0).await?
                };
                let known: HashSet<String> = local.iter().map(|i| i.id.as_hex().to_string()).collect();
                let since = filter.0["since"].as_i64().unwrap_or(0);
                for ev in evs.iter() {
                    let id = ev["id"].as_str().unwrap_or("");
                    if known.contains(id) || ev["created_at"].as_i64().unwrap_or(0) < since {
                        continue;
                    }
                    self.hub.send_to(self.id, ev, false);
                }
                Ok(empty)
            }
        }
    }

    fn events(&self) -> mpsc::Receiver<RawEvent> {
        mpsc::channel(1).1
    }

    async fn status(&self) -> RelayStatusSnapshot {
        RelayStatusSnapshot::default()
    }
}

/// The part of a channel a catch-up filter asks for.
fn sync_part(f: &Value) -> Option<(String, &'static str)> {
    use communities_core::wire::{KIND_CARD, KIND_CONTENT, KIND_EPOCH, KIND_JOURNAL, KIND_SNAPSHOT};
    let kind = u16::try_from(f["kinds"].as_array()?.first()?.as_u64()?).ok()?;
    let first = |t: &str| f[t].as_array().and_then(|a| a.first()).and_then(Value::as_str).map(str::to_string);
    Some(match kind {
        KIND_JOURNAL => (first("#h")?, "journal"),
        KIND_CARD => (first("#d")?, "card"),
        KIND_EPOCH => (first("#h")?, "epochs"),
        KIND_SNAPSHOT => (first("#h")?, "snapshots"),
        KIND_CONTENT if f.get("#c").is_some() => (first("#h")?, "feed"),
        _ => return None,
    })
}
