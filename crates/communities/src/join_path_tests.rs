// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The join of a channel through two apps, as the module wires them: the
//! owner's app keeps the channel, another app opens its link
//! (живой запуск 2026-10-23: «Ожидание хранителя» for good). Each app has
//! its own pool — the relays it is configured with, its scopes resolved as
//! messenger-transport resolves them (`Own`: the home set; `Channel`: the
//! module's routes among the configured relays), a request with no relay of
//! its scope refused with `no relay connected`, the events of `channel:…`
//! subscriptions handed to the module — over one relay that keeps every
//! event and serves a gift wrap only to the connection signed in as its
//! `#p` (vguard).

use super::*;
use communities_core::blobs::BlobHosts;
use communities_core::service::NewChannel;
use communities_core::JoinMode;
use messenger_core::traits::{RelayState, RelayStatus, RelayStatusSnapshot};
use messenger_core::{Ack, EventId, PubKey, Timestamp};
use std::collections::BTreeMap;
use std::sync::Mutex as StdMutex;

const RELAY: &str = "wss://r.example";
const RELAY2: &str = "wss://r2.example";
const HOST: &str = "https://h1.example";

fn lock<T>(m: &StdMutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// NIP-01 matching of the filters the module makes.
fn matches(filter: &serde_json::Value, e: &serde_json::Value) -> bool {
    let Some(f) = filter.as_object() else { return false };
    f.iter().all(|(k, v)| match k.as_str() {
        "kinds" => v.as_array().is_some_and(|ks| ks.iter().any(|x| x.as_u64() == e["kind"].as_u64())),
        "ids" => v.as_array().is_some_and(|ids| ids.iter().any(|x| x.as_str() == e["id"].as_str())),
        "authors" => v.as_array().is_some_and(|a| a.iter().any(|x| x.as_str() == e["pubkey"].as_str())),
        "since" => e["created_at"].as_i64() >= v.as_i64(),
        "until" => e["created_at"].as_i64() <= v.as_i64(),
        "limit" => true,
        tag if tag.starts_with('#') => {
            let want: Vec<&str> = v.as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
            e["tags"].as_array().is_some_and(|tags| tags.iter().any(|t| t[0].as_str() == Some(&tag[1..]) && t[1].as_str().is_some_and(|x| want.contains(&x))))
        }
        _ => false,
    })
}

/// vguard: a wrap goes only to the connection signed in as its `#p`.
fn served_to(e: &serde_json::Value, who: &str) -> bool {
    if e["kind"].as_u64() != Some(u64::from(communities_core::notes::KIND_WRAP)) {
        return true;
    }
    e["tags"].as_array().is_some_and(|tags| tags.iter().any(|t| t[0] == "p" && t[1] == who))
}

/// The relays of the test: every event with the relays it reached, and the
/// pools that listen.
#[derive(Default)]
struct Network {
    events: StdMutex<Vec<(serde_json::Value, HashSet<String>)>>,
    pools: StdMutex<Vec<Arc<FakePool>>>,
}

impl Network {
    fn publish(&self, json: serde_json::Value, relays: HashSet<String>) {
        lock(&self.events).push((json.clone(), relays.clone()));
        for pool in lock(&self.pools).iter() {
            pool.hear(&json, &relays);
        }
    }
}

/// One subscription of a pool: its filter and the relays it went to.
struct Sub {
    filter: serde_json::Value,
    relays: HashSet<String>,
}

/// The messenger's pool of one app.
struct FakePool {
    network: Arc<Network>,
    /// The key the connections are signed in as.
    me: String,
    /// The relays configured now; empty until `connect` (the pool of an app
    /// that just started).
    configured: StdMutex<HashSet<String>>,
    routes: Routes,
    subs: StdMutex<BTreeMap<String, Sub>>,
    /// The module's own events (`ChannelRoutes::events`).
    events: mpsc::UnboundedSender<RawEvent>,
    /// Written: every catch-up waits, as a relay that pages days of events
    /// to a debug build of a phone.
    slow: tokio::sync::RwLock<()>,
    /// The ids of the subscriptions as they went out, each time.
    asked: StdMutex<Vec<String>>,
    /// The gift wraps a relay handed back on a subscription, as it does
    /// what it holds of a filter before the new ones: live, to the module.
    replayed: std::sync::atomic::AtomicUsize,
    /// The gift wraps a catch-up brought.
    paged: std::sync::atomic::AtomicUsize,
    /// The ids of every event a catch-up brought, each time.
    paged_ids: StdMutex<Vec<String>>,
    /// The ids of every event handed to the module as live.
    live_ids: StdMutex<Vec<String>>,
    /// How long every request but a subscription waits for its relays (a
    /// publish, a `Sync`): a phone on a slow network.
    delay: StdMutex<Duration>,
    /// The filter of every `Sync`, as it was asked.
    syncs: StdMutex<Vec<serde_json::Value>>,
}

impl FakePool {
    fn connect(&self) {
        *lock(&self.configured) = HashSet::from([RELAY.to_string()]);
    }

    /// Configured with the second relay too.
    fn connect_both(&self) {
        *lock(&self.configured) = HashSet::from([RELAY.to_string(), RELAY2.to_string()]);
    }

    /// The relays a scope reaches, as `RelayPool::members` finds them.
    fn members(&self, scope: &Scope) -> HashSet<String> {
        let configured = lock(&self.configured).clone();
        match scope {
            Scope::Channel { id } => self.routes.of(id).iter().map(|u| u.as_str().to_string()).filter(|u| configured.contains(u)).collect(),
            Scope::Own | Scope::Group { .. } => configured,
            Scope::Server { .. } => HashSet::new(),
        }
    }

    fn raw(json: &serde_json::Value, source: EventSource) -> RawEvent {
        RawEvent {
            id: EventId::parse(json["id"].as_str().unwrap_or_default()).expect("id"),
            kind: u16::try_from(json["kind"].as_u64().unwrap_or(0)).unwrap_or(0),
            pubkey: PubKey::parse(json["pubkey"].as_str().unwrap_or_default()).expect("pubkey"),
            created_at: Timestamp(json["created_at"].as_i64().unwrap_or(0)),
            json: json.clone(),
            source,
        }
    }

    /// An event published now: to the module when a `channel:…`
    /// subscription of a relay it reached takes it.
    fn hear(&self, json: &serde_json::Value, relays: &HashSet<String>) {
        if !served_to(json, &self.me) {
            return;
        }
        let subs = lock(&self.subs);
        let hit = subs.iter().any(|(id, s)| id.starts_with(messenger_transport::CHANNEL_SUB_PREFIX) && !s.relays.is_disjoint(relays) && matches(&s.filter, json));
        if hit {
            lock(&self.live_ids).push(json["id"].as_str().unwrap_or_default().to_string());
            let _ = self.events.send(Self::raw(json, EventSource::Relay { url: RelayUrl::parse(RELAY).expect("url") }));
        }
    }

    fn none() -> MessengerError {
        MessengerError::Transport("no relay connected".into())
    }

    fn accepted(relays: &HashSet<String>) -> Ack {
        Ack { accepted_by: relays.iter().filter_map(|r| RelayUrl::parse(r)).collect(), rejected_by: vec![] }
    }
}

#[async_trait::async_trait]
impl Transport for FakePool {
    async fn send(&self, out: Outbound) -> messenger_core::Result<Ack> {
        let delay = *lock(&self.delay);
        if !delay.is_zero() && !matches!(out, Outbound::Subscribe { .. } | Outbound::Unsubscribe { .. }) {
            tokio::time::sleep(delay).await;
        }
        match out {
            Outbound::PublishScoped { scope, event } => {
                let to = self.members(&scope);
                if to.is_empty() {
                    return Err(Self::none());
                }
                self.network.publish(event.json, to.clone());
                Ok(Self::accepted(&to))
            }
            Outbound::PublishOwn { event } => {
                let to = self.members(&Scope::Own);
                if to.is_empty() {
                    return Err(Self::none());
                }
                self.network.publish(event.json, to.clone());
                Ok(Self::accepted(&to))
            }
            // The hints, connected or reached for a moment; none: the home set.
            Outbound::PublishToInbox { event, hint_relays, .. } => {
                let mut to: HashSet<String> = hint_relays.iter().map(|r| r.as_str().to_string()).collect();
                if to.is_empty() {
                    to = self.members(&Scope::Own);
                }
                if to.is_empty() {
                    return Err(Self::none());
                }
                self.network.publish(event.json, to.clone());
                Ok(Self::accepted(&to))
            }
            Outbound::Subscribe { id, filter, scope } => {
                lock(&self.subs).remove(&id.0);
                let relays = self.members(&scope);
                if relays.is_empty() {
                    return Err(Self::none());
                }
                // strfry takes an id of 64 bytes at most (NIP-01): a longer
                // one is answered `CLOSED` («bad req: invalid subscription id
                // length») after the pool took the REQ out, and nothing of it
                // ever comes.
                if id.0.len() > 64 {
                    lock(&self.asked).push(id.0.clone());
                    return Ok(Self::accepted(&relays));
                }
                // What the relays hold of the filter comes first, as live:
                // all of it but with `limit` 0.
                let held: Vec<serde_json::Value> = if filter.0["limit"].as_i64() == Some(0) {
                    Vec::new()
                } else {
                    lock(&self.network.events).iter().filter(|(e, on)| !on.is_disjoint(&relays) && served_to(e, &self.me) && matches(&filter.0, e)).map(|(e, _)| e.clone()).collect()
                };
                lock(&self.asked).push(id.0.clone());
                lock(&self.subs).insert(id.0, Sub { filter: filter.0, relays: relays.clone() });
                for e in held {
                    if e["kind"].as_u64() == Some(u64::from(communities_core::notes::KIND_WRAP)) {
                        self.replayed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                    let _ = self.events.send(Self::raw(&e, EventSource::Relay { url: RelayUrl::parse(RELAY).expect("url") }));
                }
                Ok(Self::accepted(&relays))
            }
            Outbound::Unsubscribe { id } => {
                lock(&self.subs).remove(&id.0);
                Ok(Ack { accepted_by: vec![], rejected_by: vec![] })
            }
            Outbound::Sync { scope, filter, .. } => {
                lock(&self.syncs).push(filter.0.clone());
                let _slow = self.slow.read().await;
                let relays = self.members(&scope);
                if relays.is_empty() {
                    return Err(Self::none());
                }
                let found: Vec<serde_json::Value> = lock(&self.network.events)
                    .iter()
                    .filter(|(e, on)| !on.is_disjoint(&relays) && served_to(e, &self.me) && matches(&filter.0, e))
                    .map(|(e, _)| e.clone())
                    .collect();
                for e in found {
                    lock(&self.paged_ids).push(e["id"].as_str().unwrap_or_default().to_string());
                    if e["kind"].as_u64() == Some(u64::from(communities_core::notes::KIND_WRAP)) {
                        self.paged.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                    let _ = self.events.send(Self::raw(&e, EventSource::Sync { url: RelayUrl::parse(RELAY).expect("url") }));
                }
                Ok(Ack { accepted_by: vec![], rejected_by: vec![] })
            }
        }
    }

    fn events(&self) -> mpsc::Receiver<RawEvent> {
        mpsc::channel(1).1
    }

    async fn status(&self) -> RelayStatusSnapshot {
        let relays = lock(&self.configured).iter().filter_map(|r| RelayUrl::parse(r)).map(|url| RelayStatus { url, state: RelayState::Connected }).collect();
        RelayStatusSnapshot { relays }
    }
}

/// Blob hosts in memory; refusing every put as a host bound to accounts
/// does a key it has none of, while `refusing` is set; every put waits
/// while `hang` holds a notice (a host that never answers), until it is
/// told to go on (`notify_waiters`).
#[derive(Default)]
pub(super) struct MemHosts {
    blobs: StdMutex<HashMap<(String, String), Vec<u8>>>,
    pub(super) refusing: std::sync::atomic::AtomicBool,
    pub(super) hang: StdMutex<Option<Arc<tokio::sync::Notify>>>,
    /// Every get waits while this holds a semaphore with no permit (a host
    /// that never answers a read), until it is closed.
    pub(super) get_gate: StdMutex<Option<Arc<tokio::sync::Semaphore>>>,
}

#[async_trait::async_trait]
impl BlobHosts for MemHosts {
    async fn put(&self, _signer: &Keys, host: &str, sha256: &str, bytes: &[u8]) -> messenger_core::Result<()> {
        if self.refusing.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(MessengerError::Transport("err.auth_failed: blossom 403: no_account".into()));
        }
        let gate = lock(&self.hang).clone();
        if let Some(gate) = gate {
            gate.notified().await;
        }
        lock(&self.blobs).insert((host.into(), sha256.into()), bytes.to_vec());
        Ok(())
    }
    async fn get(&self, host: &str, sha256: &str) -> messenger_core::Result<Option<Vec<u8>>> {
        let gate = lock(&self.get_gate).clone();
        if let Some(gate) = gate {
            let _ = gate.acquire().await;
        }
        Ok(lock(&self.blobs).get(&(host.to_string(), sha256.to_string())).cloned())
    }
    async fn delete(&self, _signer: &Keys, host: &str, sha256: &str) -> messenger_core::Result<()> {
        lock(&self.blobs).remove(&(host.to_string(), sha256.to_string()));
        Ok(())
    }
}

/// One app: its pool, its service, its listener and the loop that takes
/// what the pool hands the module (`Driver::on_event`, `Driver::apply`).
struct App {
    keys: Keys,
    pool: Arc<FakePool>,
    service: ChannelService,
    listener: Listener,
    lines: Arc<StdMutex<Vec<String>>>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    /// Those waiting for a card or a journal: offered what the pool hands
    /// the module before the loop gets it (`Shared::open`).
    previews: Previews,
    /// The sender of the outbox, ended with the app.
    outbox: Outbox,
    /// The keepers' turn beside the loop, what asks for it, and the jobs
    /// of the hosts it spawns, as the module runs them.
    keepers: Keepers,
    wake: Wake,
    uploads: Uploads,
    /// Held: the loop is busy (a round's catch-up, a keeper's turn) and
    /// takes nothing meanwhile.
    busy: Arc<tokio::sync::Mutex<()>>,
    /// How long the loop took each event, by its kind.
    took: Arc<StdMutex<Vec<(u64, Duration)>>>,
    /// Each event that held the loop past `SLOW_STEP`: its kind, how long,
    /// and the step it was at by then (`Steps`, as the module's watch says it).
    held_at: HeldAt,
}

impl App {
    async fn start(network: &Arc<Network>, keys: Keys, db: Db, secrets: Arc<dyn SecretStore>, hosts: Arc<MemHosts>, waits: Vec<Duration>) -> Self {
        let (tx, mut handed) = mpsc::unbounded_channel::<RawEvent>();
        let (to_loop, mut rx) = mpsc::unbounded_channel::<RawEvent>();
        let previews = Previews::default();
        let offered = previews.clone();
        let hand_off = tokio::spawn(async move {
            while let Some(raw) = handed.recv().await {
                offered.offer(&raw.json);
                if to_loop.send(raw).is_err() {
                    break;
                }
            }
        });
        let busy: Arc<tokio::sync::Mutex<()>> = Arc::default();
        let routes = Routes::default();
        let pool = Arc::new(FakePool {
            network: network.clone(),
            me: keys.public_key().to_hex(),
            configured: StdMutex::default(),
            routes: routes.clone(),
            subs: StdMutex::default(),
            events: tx,
            slow: tokio::sync::RwLock::new(()),
            asked: StdMutex::default(),
            replayed: Default::default(),
            paged: Default::default(),
            paged_ids: StdMutex::default(),
            live_ids: StdMutex::default(),
            delay: StdMutex::default(),
            syncs: StdMutex::default(),
        });
        lock(&network.pools).push(pool.clone());
        let service = ChannelService::new(db, secrets, pool.clone(), hosts, Arc::new(SystemClock));
        service.set_signer(Some(keys.clone()));
        let lines: Arc<StdMutex<Vec<String>>> = Arc::default();
        // What the module writes on the terminal (`communities: …`).
        let said = lines.clone();
        service.set_say(Some(Arc::new(move |line: &str| lock(&said).push(line.to_string()))));
        let mut listener = Listener::new(service.clone(), pool.clone(), routes);
        listener.waits = waits.into();
        listener.refresh_routes().await;
        // The outbox task and the keepers' turn, as the module runs them: what
        // a command or an event queued goes out beside the loop, and a turn
        // asked for (`Outcome::wake`) runs right after the event.
        let (signals, mut wakes) = mpsc::unbounded_channel::<Signal>();
        let outbox = Outbox::default();
        outbox.start(service.clone(), signals, |_| {});
        let (keepers, wake, uploads) = (Keepers::default(), Wake::default(), Uploads::default());
        let (svc, held, up, wk, k, said) = (service.clone(), busy.clone(), uploads.clone(), wake.clone(), keys.clone(), lines.clone());
        keepers.start(wake.clone(), move || {
            let (svc, held, up, wk, k, said) = (svc.clone(), held.clone(), up.clone(), wk.clone(), k.clone(), said.clone());
            async move {
                let _busy = held.lock().await;
                let Ok(turned) = svc.turn().await else { return None };
                for job in turned.jobs {
                    let (s, w) = (svc.clone(), wk.clone());
                    up.spawn(&job.key(), async move {
                        s.run_job(job).await;
                        w.at(now_secs());
                    });
                }
                for job in turned.fetches {
                    spawn_fetch(&svc, &up, &k, &wk, &said, job);
                }
                spawn_asks(&svc, &up);
                turned.wake
            }
        });
        // What the outbox task signals (an operation out) wakes the turn.
        let wk = wake.clone();
        let turns = tokio::spawn(async move {
            while let Some(signal) = wakes.recv().await {
                if let Signal::Wake(at) = signal {
                    wk.at(at);
                }
            }
        });
        let took: Arc<StdMutex<Vec<(u64, Duration)>>> = Arc::default();
        let held_at: HeldAt = Arc::default();
        let (svc, lis, log, k, held, wk, up, tk, ha) = (service.clone(), listener.clone(), lines.clone(), keys.clone(), busy.clone(), wake.clone(), uploads.clone(), took.clone(), held_at.clone());
        let pump = tokio::spawn(async move {
            while let Some(raw) = rx.recv().await {
                let _busy = held.lock().await;
                let live = !matches!(raw.source, EventSource::Sync { .. });
                let kind = raw.json["kind"].as_u64().unwrap_or(0);
                let started = std::time::Instant::now();
                // The step the event is at once it held the loop past
                // `SLOW_STEP`, as the module's watch reads it.
                let steps = communities_core::keeper::Steps::default();
                steps.set(Some(format!("an event of kind {kind}")));
                let probe = {
                    let steps = steps.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(SLOW_STEP).await;
                        steps.now()
                    })
                };
                // One channel at a time, as `Driver::on_event`.
                let cid = raw.json["tags"].as_array().and_then(|tags| tags.iter().find(|t| t[0] == "h" || t[0] == "d").and_then(|t| t[1].as_str()).map(str::to_string));
                let taken = steps
                    .run(async {
                        match cid {
                            Some(cid) => {
                                let lock = svc.lock_of(&cid).await;
                                let _guard = lock.lock().await;
                                svc.on_event(&k, &raw.json, live).await
                            }
                            None => svc.on_event(&k, &raw.json, live).await,
                        }
                    })
                    .await;
                let took_now = started.elapsed();
                lock(&tk).push((kind, took_now));
                if took_now >= SLOW_STEP {
                    let at = probe.await.ok().flatten();
                    lock(&ha).push((kind, took_now, at));
                } else {
                    probe.abort();
                }
                match taken {
                    Ok(outcome) => {
                        lock(&log).extend(outcome.notes.iter().cloned());
                        if outcome.resubscribe {
                            lis.resubscribe();
                        }
                        if let Some(at) = outcome.wake {
                            wk.at(at);
                        }
                    }
                    Err(e) => lock(&log).push(format!("an event was not taken: {e}")),
                }
                spawn_fetches(&svc, &up, &k, &wk, &log);
                spawn_asks(&svc, &up);
            }
        });
        Self { keys, pool, service, listener, lines, tasks: vec![hand_off, pump, turns], previews, busy, took, held_at, outbox, keepers, wake, uploads }
    }

    fn stop(self) {
        self.keepers.close();
        self.uploads.close();
        self.outbox.close();
        for t in self.tasks {
            t.abort();
        }
    }
}

/// Each event that held the loop past `SLOW_STEP`: its kind, how long, and
/// the step it was at by then.
type HeldAt = Arc<StdMutex<Vec<(u64, Duration, Option<String>)>>>;

/// How long an event may hold the loop of the harness before the step it
/// is at is kept (`App::held_at`).
const SLOW_STEP: Duration = Duration::from_millis(250);

/// What the events wanted of the relays, sent by a task of its own, as
/// `Driver::spawn_asks` sends it.
fn spawn_asks(svc: &ChannelService, up: &Uploads) {
    let asks = svc.take_asks();
    if asks.is_empty() {
        return;
    }
    static ASKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let s = svc.clone();
    up.spawn(&format!("asks:{}", ASKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)), async move {
        s.send_asks(asks).await;
    });
}

/// The fetches of the member's side due now, each a task of its own, as
/// `Driver::spawn_fetches` runs them.
fn spawn_fetches(svc: &ChannelService, up: &Uploads, keys: &Keys, wake: &Wake, lines: &Arc<StdMutex<Vec<String>>>) {
    for job in svc.take_fetches() {
        spawn_fetch(svc, up, keys, wake, lines, job);
    }
}

fn spawn_fetch(svc: &ChannelService, up: &Uploads, keys: &Keys, wake: &Wake, lines: &Arc<StdMutex<Vec<String>>>, job: communities_core::member::fetch::FetchJob) {
    let (s, u, k, w, l) = (svc.clone(), up.clone(), keys.clone(), wake.clone(), lines.clone());
    up.spawn(&job.key(), async move {
        let out = s.run_fetch(&k, job).await;
        lock(&l).extend(out.notes.iter().cloned());
        if let Some(at) = out.wake {
            w.at(at);
        }
        spawn_fetches(&s, &u, &k, &w, &l);
    });
}

async fn has_key(app: &App, cid: &str) -> bool {
    db::keys::epochs(app.service.db(), cid, communities_core::state::SCOPE_EVERYONE).await.unwrap_or_default().iter().any(|r| r.key_hex.is_some())
}

/// A long wait before a round is tried again: the test of the relays'
/// watch must not be passed by the retry.
const NO_RETRY: Duration = Duration::from_secs(600);

/// The run of the owner's app before: the channel made, its keeper here.
/// The id of the channel and its link.
async fn made_before(network: &Arc<Network>, hosts: &Arc<MemHosts>, path: &Path, owner: &Keys, secrets: &Arc<dyn SecretStore>) -> (String, String) {
    let first = App::start(network, owner.clone(), Db::open(path).await.unwrap(), secrets.clone(), hosts.clone(), vec![NO_RETRY]).await;
    first.pool.connect();
    let new = NewChannel {
        name: "Channel".into(),
        about: String::new(),
        picture: String::new(),
        join: JoinMode::Open,
        history: Default::default(),
        relays: vec![RELAY.into()],
        storage: vec![HOST.into()],
    };
    let cid = {
        let _creating = first.listener.routes.creating(vec![RelayUrl::parse(RELAY).unwrap()]).await;
        let cid = first.service.create(owner, new).await.unwrap().0;
        // The row is here: its route is known before the outbox sends.
        first.listener.refresh_routes().await;
        cid
    };
    let link = first.service.share_link(&cid).await.unwrap();
    // Its journal and card on the relay before the run ends.
    first.service.flush_outbox().await.unwrap();
    first.service.db().close().await;
    lock(&network.pools).clear();
    first.stop();
    (cid, link)
}

/// The owner made the channel in a run of the app before; the app starts
/// again and listens before its pool has a relay (seven times `no relay
/// connected` in the owner's terminal, живой запуск 2026-10-23), the
/// relays come a moment later. Another app opens the link and joins: the
/// owner's keeper hears the join on the subscription the module makes,
/// issues, and the joiner holds the key of the channel. The owner's
/// terminal tells the way of the join.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_join_reaches_the_keeper_of_an_app_that_listened_before_its_relays_came() {
    let network = Arc::new(Network::default());
    let hosts = Arc::new(MemHosts::default());
    let dir = std::env::temp_dir().join(format!("communities-join-{}-{}", std::process::id(), now_secs()));
    let owner = Keys::generate();
    let owner_secrets: Arc<dyn SecretStore> = Arc::new(super::tests::MemSecrets::default());
    let (cid, link) = made_before(&network, &hosts, &dir.join("owner.db"), &owner, &owner_secrets).await;

    // The app again: it listens at once, its pool has no relay yet.
    let mut a = App::start(&network, owner.clone(), Db::open(&dir.join("owner.db")).await.unwrap(), owner_secrets, hosts.clone(), vec![NO_RETRY]).await;
    a.listener.resubscribe();
    until_quiet(&a.listener).await;
    assert!(lock(&a.pool.subs).is_empty(), "nothing subscribed without a relay");
    // The session watcher of the module, as it looks every few seconds.
    let watched = a.listener.clone();
    a.tasks.push(tokio::spawn(async move {
        loop {
            watched.watch_relays().await;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }));
    a.pool.connect();

    // The joiner: its app runs, its relays are up; it opens the link as
    // `channel_open_link` does.
    let b = App::start(&network, Keys::generate(), Db::open_in_memory().await.unwrap(), Arc::new(super::tests::MemSecrets::default()), hosts.clone(), vec![NO_RETRY]).await;
    b.pool.connect();
    b.listener.resubscribe();
    until_quiet(&b.listener).await;
    let parsed = communities_core::link::ChannelLink::parse(&link).unwrap();
    let route = b.listener.routes.hold(&cid, parsed.relay.clone());
    b.service.expect_owner(&cid, &parsed.owner).await.unwrap();
    b.service.ask_journal(&cid).await.unwrap();
    wait_for(Duration::from_secs(5), || async { b.service.log(&cid).await.unwrap().is_some() }).await;
    let outcome = b.service.open_link(&b.keys, &link, "").await.unwrap();
    route.keep();
    if outcome.resubscribe {
        b.listener.resubscribe();
    }

    let joined = wait_until(Duration::from_secs(10), || async { has_key(&b, &cid).await }).await;
    let owner_lines = lock(&a.lines).clone();
    assert!(joined, "the joiner never got the key; the owner's log: {owner_lines:?}");
    let joiner = b.keys.public_key().to_hex();
    let of_join = |end: &str| owner_lines.iter().any(|l| l.starts_with(&format!("channel.join from {} for {}", &joiner[..8], &cid[..8])) && l.ends_with(end));
    assert!(of_join(": to the keeper"), "the owner sees the join come: {owner_lines:?}");
    assert!(of_join(": taken, a reader of the channel"), "and what became of it: {owner_lines:?}");
    assert!(owner_lines.iter().any(|l| l.starts_with(&format!("channel.issue to {} for {}: taken by {RELAY}", &joiner[..8], &cid[..8]))), "{owner_lines:?}");
    let joiner_lines = lock(&b.lines).clone();
    assert!(joiner_lines.iter().any(|l| l.starts_with("channel.join to ") && l.contains("taken by")), "{joiner_lines:?}");
    a.stop();
    b.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Хосты канала отказывают хранителю (403 `no_account`, живой запуск
/// 2026-10-23): эпоха не уходит, но вступление проходит — вступивший
/// получает ключ, а терминал владельца говорит об отказе хоста один раз,
/// сколько бы кругов ни прошло, и состояние хранителя несёт ответ хоста.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_keeper_whose_hosts_refuse_answers_a_join_and_says_the_refusal_once() {
    let network = Arc::new(Network::default());
    let hosts = Arc::new(MemHosts::default());
    let dir = std::env::temp_dir().join(format!("communities-refuse-{}-{}", std::process::id(), now_secs()));
    let owner = Keys::generate();
    let owner_secrets: Arc<dyn SecretStore> = Arc::new(super::tests::MemSecrets::default());
    let (cid, link) = made_before(&network, &hosts, &dir.join("owner.db"), &owner, &owner_secrets).await;

    let a = App::start(&network, owner.clone(), Db::open(&dir.join("owner.db")).await.unwrap(), owner_secrets, hosts.clone(), vec![NO_RETRY]).await;
    a.pool.connect();
    a.listener.resubscribe();
    until_quiet(&a.listener).await;
    hosts.refusing.store(true, std::sync::atomic::Ordering::SeqCst);
    a.service.rotate_now(&cid).await.unwrap();
    for _ in 0..3 {
        a.service.tick().await.unwrap();
    }
    let status = a.service.keeper_status(&cid).unwrap();
    assert!(status.pending_epoch.is_some(), "the epoch waits for its hosts");
    assert!(status.storage_error.as_deref().is_some_and(|e| e.contains(HOST) && e.contains("no_account")), "{status:?}");
    // Each refusal of the hosts (a blob of the epoch, a shard of the
    // snapshot the turn at the start put) said once, however many turns
    // pass. The lines are read once: a message of a failure must not lock
    // them again on this thread.
    let repeated = |lines: &[String]| {
        let mut seen = HashSet::new();
        let refusals: Vec<&String> = lines.iter().filter(|l| l.starts_with(&format!("keeper {}: the blob hosts: ", &cid[..8])) && l.contains("no_account")).collect();
        (refusals.len(), refusals.iter().filter(|l| !seen.insert(l.as_str())).count())
    };
    let said = lock(&a.lines).clone();
    let (refusals, again) = repeated(&said);
    assert!(refusals >= 1 && again == 0, "said once: {said:?}");

    let b = App::start(&network, Keys::generate(), Db::open_in_memory().await.unwrap(), Arc::new(super::tests::MemSecrets::default()), hosts.clone(), vec![NO_RETRY]).await;
    b.pool.connect();
    b.listener.resubscribe();
    until_quiet(&b.listener).await;
    let parsed = communities_core::link::ChannelLink::parse(&link).unwrap();
    let route = b.listener.routes.hold(&cid, parsed.relay.clone());
    b.service.expect_owner(&cid, &parsed.owner).await.unwrap();
    b.service.ask_journal(&cid).await.unwrap();
    wait_for(Duration::from_secs(5), || async { b.service.log(&cid).await.unwrap().is_some() }).await;
    let outcome = b.service.open_link(&b.keys, &link, "").await.unwrap();
    route.keep();
    if outcome.resubscribe {
        b.listener.resubscribe();
    }
    let joined = wait_until(Duration::from_secs(10), || async { has_key(&b, &cid).await }).await;
    let owner_lines = lock(&a.lines).clone();
    assert!(joined, "the joiner never got the key while the hosts refused; the owner's log: {owner_lines:?}");
    assert!(owner_lines.iter().any(|l| l.contains("channel.join from") && l.ends_with(": taken, a reader of the channel")), "{owner_lines:?}");
    a.service.tick().await.unwrap();
    let said = lock(&a.lines).clone();
    assert_eq!(repeated(&said).1, 0, "still once: {said:?}");
    a.stop();
    b.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

/// A host that never answers a put (живой запуск 2026-10-23: a turn of
/// the keeper held the loop across its uploads, «an event of kind 1059
/// holds the loop for 83 s») holds no event: the keeper's turn hands the
/// blobs of the next epoch to a job of its own, and a post of a reader
/// published meanwhile is heard and kept by the owner's loop while the
/// job still hangs; once the host answers, the epoch goes out and the
/// reader holds its key.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_upload_that_hangs_holds_no_event_and_the_epoch_goes_out_after() {
    let network = Arc::new(Network::default());
    let hosts = Arc::new(MemHosts::default());
    let dir = std::env::temp_dir().join(format!("communities-hang-{}-{}", std::process::id(), now_secs()));
    let owner = Keys::generate();
    let owner_secrets: Arc<dyn SecretStore> = Arc::new(super::tests::MemSecrets::default());
    let (cid, link) = made_before(&network, &hosts, &dir.join("owner.db"), &owner, &owner_secrets).await;

    let a = App::start(&network, owner.clone(), Db::open(&dir.join("owner.db")).await.unwrap(), owner_secrets, hosts.clone(), vec![NO_RETRY]).await;
    a.pool.connect();
    a.listener.resubscribe();
    until_quiet(&a.listener).await;
    // The first epoch, as the module asks for a turn at a start.
    a.wake.at(0);
    wait_for(Duration::from_secs(10), || async { has_key(&a, &cid).await }).await;

    let b = App::start(&network, Keys::generate(), Db::open_in_memory().await.unwrap(), Arc::new(super::tests::MemSecrets::default()), hosts.clone(), vec![NO_RETRY]).await;
    b.pool.connect();
    b.listener.resubscribe();
    until_quiet(&b.listener).await;
    let parsed = communities_core::link::ChannelLink::parse(&link).unwrap();
    let route = b.listener.routes.hold(&cid, parsed.relay.clone());
    b.service.expect_owner(&cid, &parsed.owner).await.unwrap();
    b.service.ask_journal(&cid).await.unwrap();
    wait_for(Duration::from_secs(5), || async { b.service.log(&cid).await.unwrap().is_some() }).await;
    let outcome = b.service.open_link(&b.keys, &link, "").await.unwrap();
    route.keep();
    if outcome.resubscribe {
        b.listener.resubscribe();
    }
    wait_for(Duration::from_secs(10), || async { has_key(&b, &cid).await }).await;
    let (post, _) = a.service.send_post(&owner, &cid, "the post", messenger_core::TextFormat::Plain).await.unwrap();
    wait_for(Duration::from_secs(10), || async { db::content::get(b.service.db(), &post).await.unwrap().is_some() }).await;

    // The hosts stop answering; an epoch is asked for: its blobs hang.
    let gate = Arc::new(tokio::sync::Notify::new());
    *lock(&hosts.hang) = Some(gate.clone());
    a.service.rotate_now(&cid).await.unwrap();
    a.wake.at(0);
    let put_key = |cid: &str| communities_core::keeper::upload::put_key(cid, communities_core::state::SCOPE_EVERYONE, 2);
    wait_for(Duration::from_secs(10), || async { a.service.jobs_running().contains(&put_key(&cid)) }).await;
    let status = a.service.keeper_status(&cid).unwrap();
    assert_eq!((status.epoch, status.pending_epoch), (Some(1), Some(2)), "the epoch waits for its hosts: {status:?}");

    // The reader comments meanwhile (a reader posts nothing in a channel
    // of authors): the owner's loop takes it at once.
    let (comment, _) = b.service.send_comment(&b.keys, &cid, &post, None, "while the hosts hang", messenger_core::TextFormat::Plain).await.unwrap();
    // Taken by the owner's loop: kept, parked until the snapshot names the
    // reader, or judged by the snapshot held (said either way).
    let heard = wait_until(Duration::from_secs(10), || async {
        db::content::get(a.service.db(), &comment).await.unwrap().is_some()
            || db::channels::count_pending(a.service.db(), &cid).await.unwrap() > 0
            || lock(&a.lines).iter().any(|l| l.starts_with("content waits") || l.starts_with("message refused"))
    })
    .await;
    assert!(a.service.jobs_running().contains(&put_key(&cid)), "the upload still hangs");
    assert!(heard, "the post was not heard while the upload hung: {:?}", lock(&a.lines));
    assert_eq!(a.service.keeper_status(&cid).unwrap().pending_epoch, Some(2), "and the epoch still waits");

    // The host answers: the epoch goes out, the reader holds its key.
    *lock(&hosts.hang) = None;
    gate.notify_waiters();
    let out = wait_until(Duration::from_secs(15), || async {
        lock(&network.events).iter().any(|(e, _)| e["kind"] == communities_core::wire::KIND_EPOCH && e["tags"].as_array().is_some_and(|t| t.iter().any(|t| t[0] == "n" && t[1] == "2")))
    })
    .await;
    assert!(out, "the epoch never went out: {:?}", lock(&a.lines));
    let keyed = wait_until(Duration::from_secs(10), || async {
        db::keys::epochs(b.service.db(), &cid, communities_core::state::SCOPE_EVERYONE).await.unwrap_or_default().iter().any(|r| r.epoch == 2 && r.key_hex.is_some())
    })
    .await;
    assert!(keyed, "the reader holds no key of epoch 2: {:?}", lock(&b.lines));
    assert!(!a.service.jobs_running().contains(&put_key(&cid)), "the job ended");
    a.stop();
    b.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Hosts that never answer a read (живой запуск 2026-10-23: «an event of
/// kind 1059 holds the loop for 34 s», every minute: an epoch or a shard
/// of a snapshot fetched inline from slow hosts) hold no event of a
/// reader: the epoch event is taken at once, its shard is fetched by a
/// task of its own, a post under the new epoch is heard and parked
/// meanwhile, and once the hosts answer the epoch opens and the post is
/// read.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hosts_that_never_answer_a_read_hold_no_event_of_a_reader() {
    let network = Arc::new(Network::default());
    let hosts = Arc::new(MemHosts::default());
    let dir = std::env::temp_dir().join(format!("communities-get-hang-{}-{}", std::process::id(), now_secs()));
    let owner = Keys::generate();
    let owner_secrets: Arc<dyn SecretStore> = Arc::new(super::tests::MemSecrets::default());
    let (cid, link) = made_before(&network, &hosts, &dir.join("owner.db"), &owner, &owner_secrets).await;

    let a = App::start(&network, owner.clone(), Db::open(&dir.join("owner.db")).await.unwrap(), owner_secrets, hosts.clone(), vec![NO_RETRY]).await;
    a.pool.connect();
    a.listener.resubscribe();
    until_quiet(&a.listener).await;
    a.wake.at(0);
    wait_for(Duration::from_secs(10), || async { has_key(&a, &cid).await }).await;

    let b = App::start(&network, Keys::generate(), Db::open_in_memory().await.unwrap(), Arc::new(super::tests::MemSecrets::default()), hosts.clone(), vec![NO_RETRY]).await;
    b.pool.connect();
    b.listener.resubscribe();
    until_quiet(&b.listener).await;
    let parsed = communities_core::link::ChannelLink::parse(&link).unwrap();
    let route = b.listener.routes.hold(&cid, parsed.relay.clone());
    b.service.expect_owner(&cid, &parsed.owner).await.unwrap();
    b.service.ask_journal(&cid).await.unwrap();
    wait_for(Duration::from_secs(5), || async { b.service.log(&cid).await.unwrap().is_some() }).await;
    let outcome = b.service.open_link(&b.keys, &link, "").await.unwrap();
    route.keep();
    if outcome.resubscribe {
        b.listener.resubscribe();
    }
    wait_for(Duration::from_secs(10), || async { has_key(&b, &cid).await }).await;

    // Every read of the hosts hangs from now on; an epoch goes out.
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *lock(&hosts.get_gate) = Some(gate.clone());
    lock(&b.took).clear();
    a.service.rotate_now(&cid).await.unwrap();
    a.wake.at(0);
    let row = |n: i64| {
        let db = b.service.db().clone();
        let cid = cid.clone();
        async move { db::keys::epochs(&db, &cid, communities_core::state::SCOPE_EVERYONE).await.unwrap_or_default().into_iter().find(|r| r.epoch == n).map(|r| r.key_hex.is_some()) }
    };
    wait_for(Duration::from_secs(15), || async { row(2).await.is_some() }).await;
    let wanted = format!("{cid}:fetch:epoch:{}:2", communities_core::state::SCOPE_EVERYONE);
    wait_for(Duration::from_secs(5), || async { b.service.fetches_wanted().contains(&wanted) }).await;
    // A post under the new epoch: heard by the reader's loop while the
    // fetch hangs, and parked for its key.
    let (post, _) = a.service.send_post(&owner, &cid, "under epoch 2", messenger_core::TextFormat::Plain).await.unwrap();
    let parked = wait_until(Duration::from_secs(10), || async { db::channels::count_pending(b.service.db(), &cid).await.unwrap() > 0 }).await;
    assert!(parked, "the post was not heard while the hosts hung: {:?}", lock(&b.lines));
    assert_eq!(row(2).await, Some(false), "the key waits for the hosts");
    assert!(b.service.fetches_wanted().contains(&wanted), "the fetch still hangs");
    let took = lock(&b.took).clone();
    assert!(took.iter().any(|(k, _)| *k == u64::from(communities_core::wire::KIND_EPOCH)), "the epoch was heard: {took:?}");
    let worst = took.iter().map(|(_, d)| *d).max().unwrap_or_default();
    assert!(worst < Duration::from_millis(300), "an event held the loop {worst:?}: {took:?}");

    // The hosts answer: the epoch opens, the post is read.
    *lock(&hosts.get_gate) = None;
    gate.close();
    let read = wait_until(Duration::from_secs(10), || async { db::content::get(b.service.db(), &post).await.unwrap().is_some() }).await;
    assert!(read, "the post was never read: {:?}", lock(&b.lines));
    assert_eq!(row(2).await, Some(true));
    assert!(b.service.fetches_wanted().is_empty());
    a.stop();
    b.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

/// The first «Войти» right after a start (живой запуск 2026-10-23: the
/// journal came 32 s late behind a round's catch-up, «Канал не ответил»):
/// the joiner's pool has no relay yet and its loop takes nothing (a
/// round's catch-up, a keeper's turn), and the journal of the link is
/// taken all the same, as the relay hands it, in a second or so; a relay
/// that answers without the journal is said as soon as it answered, not
/// after a fixed wait; one never reached is said when the wait runs out.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_journal_of_a_link_is_taken_past_a_busy_loop_and_a_relay_without_it_is_said_at_once() {
    let network = Arc::new(Network::default());
    let hosts = Arc::new(MemHosts::default());
    let dir = std::env::temp_dir().join(format!("communities-journal-{}-{}", std::process::id(), now_secs()));
    let owner = Keys::generate();
    let owner_secrets: Arc<dyn SecretStore> = Arc::new(super::tests::MemSecrets::default());
    let (cid, link) = made_before(&network, &hosts, &dir.join("owner.db"), &owner, &owner_secrets).await;

    let b = App::start(&network, Keys::generate(), Db::open_in_memory().await.unwrap(), Arc::new(super::tests::MemSecrets::default()), hosts.clone(), vec![NO_RETRY]).await;
    let busy = b.busy.clone();
    let _held = busy.lock().await;
    let parsed = communities_core::link::ChannelLink::parse(&link).unwrap();
    let route = b.listener.routes.hold(&cid, parsed.relay.clone());
    b.service.expect_owner(&cid, &parsed.owner).await.unwrap();
    // The relays come a moment after the press.
    let pool = b.pool.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        pool.connect();
    });
    let started = tokio::time::Instant::now();
    let outcomes = take_journal(&b.service, &b.previews, &cid, JOURNAL_WAIT).await.expect("the journal is taken");
    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    assert!(!outcomes.is_empty());
    assert!(b.service.log(&cid).await.unwrap().is_some(), "the journal is here though the loop took nothing");
    b.service.open_link(&b.keys, &link, "").await.expect("the link opens");
    route.keep();

    // A channel the relay holds nothing of: said once the relay answered.
    let other = "e".repeat(64);
    let _other_route = b.listener.routes.hold(&other, RelayUrl::parse(RELAY).unwrap());
    let started = tokio::time::Instant::now();
    let e = take_journal(&b.service, &b.previews, &other, JOURNAL_WAIT).await.expect_err("no journal");
    assert!(started.elapsed() < Duration::from_secs(4), "{:?}", started.elapsed());
    assert!(e.to_string().contains("channel_not_here_yet") && e.to_string().contains("holds no journal"), "{e}");
    // No relay reached: when the wait runs out, with the reason.
    lock(&b.pool.configured).clear();
    let e = take_journal(&b.service, &b.previews, &other, Duration::from_millis(1500)).await.expect_err("no relay");
    assert!(e.to_string().contains("channel_not_here_yet") && e.to_string().contains("not reached"), "{e}");
    b.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

/// The phone's keeper took one join 92 s late, from a catch-up (живой
/// запуск 2026-10-23, ~5 min after the app started): the app starts again
/// and its first round of listening pages days of events (held here for
/// the whole test), and the owner makes a new channel on another relay
/// meanwhile. A join of it published once the listening was asked for
/// again is heard live and taken by the keeper in seconds, while that
/// catch-up still runs: the subscriptions never wait for it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_join_is_heard_live_while_the_catch_up_of_a_start_still_runs() {
    let network = Arc::new(Network::default());
    let hosts = Arc::new(MemHosts::default());
    let dir = std::env::temp_dir().join(format!("communities-live-{}-{}", std::process::id(), now_secs()));
    let owner = Keys::generate();
    let owner_secrets: Arc<dyn SecretStore> = Arc::new(super::tests::MemSecrets::default());
    made_before(&network, &hosts, &dir.join("owner.db"), &owner, &owner_secrets).await;

    // The app again; every catch-up of it is held.
    let a = App::start(&network, owner.clone(), Db::open(&dir.join("owner.db")).await.unwrap(), owner_secrets, hosts.clone(), vec![NO_RETRY]).await;
    a.pool.connect_both();
    let slow = a.pool.slow.write().await;
    a.listener.resubscribe();
    wait_for(Duration::from_secs(5), || async { lock(&a.pool.subs).contains_key(&own_notes_sub().0) }).await;
    assert!(a.listener.busy(), "the catch-up of the start runs");

    // A new channel of the owner, on the second relay only.
    let new = NewChannel {
        name: "Second".into(),
        about: String::new(),
        picture: String::new(),
        join: JoinMode::Open,
        history: Default::default(),
        relays: vec![RELAY2.into()],
        storage: vec![HOST.into()],
    };
    let (cid, outcome) = {
        let _creating = a.listener.routes.creating(vec![RelayUrl::parse(RELAY2).unwrap()]).await;
        let made = a.service.create(&owner, new).await.unwrap();
        a.listener.refresh_routes().await;
        made
    };
    assert!(outcome.resubscribe, "a new channel is listened to");
    a.listener.resubscribe();
    let notes = communities_core::transport::sub_id(&cid, PART_NOTES).0;
    wait_for(Duration::from_secs(5), || async { lock(&a.pool.subs).get(&notes).is_some_and(|s| s.relays.contains(RELAY2)) }).await;
    let link = a.service.share_link(&cid).await.unwrap();

    // The joiner, its relays up, opens the link.
    let b = App::start(&network, Keys::generate(), Db::open_in_memory().await.unwrap(), Arc::new(super::tests::MemSecrets::default()), hosts.clone(), vec![NO_RETRY]).await;
    b.pool.connect_both();
    b.listener.resubscribe();
    until_quiet(&b.listener).await;
    let parsed = communities_core::link::ChannelLink::parse(&link).unwrap();
    let route = b.listener.routes.hold(&cid, parsed.relay.clone());
    b.service.expect_owner(&cid, &parsed.owner).await.unwrap();
    take_journal(&b.service, &b.previews, &cid, JOURNAL_WAIT).await.expect("the journal");
    let outcome = b.service.open_link(&b.keys, &link, "").await.unwrap();
    route.keep();
    if outcome.resubscribe {
        b.listener.resubscribe();
    }

    let joined = wait_until(Duration::from_secs(10), || async { has_key(&b, &cid).await }).await;
    let owner_lines = lock(&a.lines).clone();
    assert!(joined, "the joiner never got the key; the owner's log: {owner_lines:?}");
    let joiner = b.keys.public_key().to_hex();
    let heard = format!("channel.join from {} for {}: to the keeper", &joiner[..8], &cid[..8]);
    assert!(owner_lines.contains(&heard), "heard live, not from a catch-up: {owner_lines:?}");
    assert!(a.listener.busy(), "the catch-up of the start was still held");
    drop(slow);
    a.stop();
    b.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

/// The phone's keeper took a join 53–84 s late, three runs of three, while
/// old joins were «heard already» again and again (живой запуск
/// 2026-10-23): the subscription of the notes to me on my relays asked
/// from where they were heard, and the relay handed two days of every wrap
/// to me back on it, as live, ahead of the join. The owner's app starts
/// again over a relay that holds days of wraps to the owner (the
/// messenger's): no subscription brings any of them back (the catch-up
/// pages them), and a join published after the start comes on the live
/// subscription and is taken at once, with no page asked for it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_join_comes_on_the_live_subscription_and_no_subscription_replays_old_wraps() {
    let network = Arc::new(Network::default());
    let hosts = Arc::new(MemHosts::default());
    let dir = std::env::temp_dir().join(format!("communities-replay-{}-{}", std::process::id(), now_secs()));
    let owner = Keys::generate();
    let owner_secrets: Arc<dyn SecretStore> = Arc::new(super::tests::MemSecrets::default());
    let (cid, link) = made_before(&network, &hosts, &dir.join("owner.db"), &owner, &owner_secrets).await;
    // Two days of the messenger's wraps to the owner on the relay.
    let owner_pk = PubKey::parse(&owner.public_key().to_hex()).unwrap();
    let now = i64::try_from(now_secs()).unwrap();
    const OLD: usize = 40;
    for i in 0..OLD {
        let content = messenger_core::Envelope::new("receipt").encode();
        let at = now - i64::try_from(i).unwrap() * 3600;
        let wrap = communities_core::notes::wrap_note(&Keys::generate(), &owner_pk, &content, at).unwrap();
        network.publish(wrap.json, HashSet::from([RELAY.to_string()]));
    }

    let a = App::start(&network, owner.clone(), Db::open(&dir.join("owner.db")).await.unwrap(), owner_secrets, hosts.clone(), vec![NO_RETRY]).await;
    a.pool.connect();
    a.listener.resubscribe();
    until_quiet(&a.listener).await;
    let replayed = || a.pool.replayed.load(Ordering::SeqCst);
    let paged = || a.pool.paged.load(Ordering::SeqCst);
    assert_eq!(replayed(), 0, "no subscription hands old wraps back as live");
    assert!(paged() >= OLD, "the catch-up pages them: {}", paged());
    // Asked again (a new channel, a wake): nothing comes back either.
    a.listener.resubscribe_anew();
    until_quiet(&a.listener).await;
    assert_eq!(replayed(), 0, "a subscription sent anew brings nothing back");
    let subscribed = || lock(&a.pool.asked).len();
    let (paged_before, asked_before) = (paged(), subscribed());

    let b = App::start(&network, Keys::generate(), Db::open_in_memory().await.unwrap(), Arc::new(super::tests::MemSecrets::default()), hosts.clone(), vec![NO_RETRY]).await;
    b.pool.connect();
    b.listener.resubscribe();
    until_quiet(&b.listener).await;
    let parsed = communities_core::link::ChannelLink::parse(&link).unwrap();
    let route = b.listener.routes.hold(&cid, parsed.relay.clone());
    b.service.expect_owner(&cid, &parsed.owner).await.unwrap();
    take_journal(&b.service, &b.previews, &cid, JOURNAL_WAIT).await.expect("the journal");
    let outcome = b.service.open_link(&b.keys, &link, "").await.unwrap();
    route.keep();
    if outcome.resubscribe {
        b.listener.resubscribe();
    }
    let joined = wait_until(Duration::from_secs(10), || async { has_key(&b, &cid).await }).await;
    let owner_lines = lock(&a.lines).clone();
    assert!(joined, "the joiner never got the key; the owner's log: {owner_lines:?}");
    let joiner = b.keys.public_key().to_hex();
    let heard = format!("channel.join from {} for {}: to the keeper", &joiner[..8], &cid[..8]);
    assert!(owner_lines.contains(&heard), "heard live: {owner_lines:?}");
    assert!(!owner_lines.iter().any(|l| l.ends_with(": heard already")), "nothing heard twice: {owner_lines:?}");
    assert_eq!(paged(), paged_before, "no page of the notes for the join");
    assert_eq!(subscribed(), asked_before, "and no subscription");
    a.stop();
    b.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

/// The owner makes a channel on the phone in this run of the app, posts,
/// a reader joins by the link and comments the post (живой запуск
/// 2026-10-23: the comment never reached the phone, neither live nor by
/// the catch-up after it). The owner's module hears the comment on the
/// live subscription of the new channel while every catch-up is held,
/// and a round of catch-up after it pages it too.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_comment_of_a_reader_reaches_the_owner_of_a_channel_made_in_this_run() {
    let network = Arc::new(Network::default());
    let hosts = Arc::new(MemHosts::default());
    let owner = Keys::generate();
    let a = App::start(&network, owner.clone(), Db::open_in_memory().await.unwrap(), Arc::new(super::tests::MemSecrets::default()), hosts.clone(), vec![NO_RETRY]).await;
    a.pool.connect_both();
    a.listener.resubscribe();
    until_quiet(&a.listener).await;
    // From here on every catch-up of the owner waits: what comes, comes live.
    let slow = a.pool.slow.write().await;

    // `channel_create`, as the command does it.
    let new = NewChannel {
        name: "Made now".into(),
        about: String::new(),
        picture: String::new(),
        join: JoinMode::Open,
        history: Default::default(),
        relays: vec![RELAY.into(), RELAY2.into()],
        storage: vec![HOST.into()],
    };
    let relays = vec![RelayUrl::parse(RELAY).unwrap(), RelayUrl::parse(RELAY2).unwrap()];
    let (cid, outcome) = {
        let _creating = a.listener.routes.creating(relays).await;
        let made = a.service.create(&owner, new).await.unwrap();
        a.listener.refresh_routes().await;
        made
    };
    assert!(outcome.resubscribe);
    a.listener.resubscribe();
    let feed = communities_core::transport::sub_id(&cid, communities_core::transport::PART_FEED).0;
    wait_for(Duration::from_secs(5), || async { lock(&a.pool.subs).get(&feed).is_some_and(|s| s.relays.len() == 2) }).await;
    // The keeper's first epoch, and the owner's post under it.
    wait_for(Duration::from_secs(10), || async {
        let _ = a.service.tick().await;
        has_key(&a, &cid).await
    })
    .await;
    let (post, _) = a.service.send_post(&owner, &cid, "the first post", messenger_core::TextFormat::Plain).await.unwrap();
    let link = a.service.share_link(&cid).await.unwrap();

    // The reader joins by the link and comments the post.
    let b = App::start(&network, Keys::generate(), Db::open_in_memory().await.unwrap(), Arc::new(super::tests::MemSecrets::default()), hosts.clone(), vec![NO_RETRY]).await;
    b.pool.connect();
    b.listener.resubscribe();
    until_quiet(&b.listener).await;
    let parsed = communities_core::link::ChannelLink::parse(&link).unwrap();
    let route = b.listener.routes.hold(&cid, parsed.relay.clone());
    b.service.expect_owner(&cid, &parsed.owner).await.unwrap();
    take_journal(&b.service, &b.previews, &cid, JOURNAL_WAIT).await.expect("the journal");
    let outcome = b.service.open_link(&b.keys, &link, "").await.unwrap();
    route.keep();
    if outcome.resubscribe {
        b.listener.resubscribe();
    }
    let joined = wait_until(Duration::from_secs(10), || async { has_key(&b, &cid).await }).await;
    assert!(joined, "the reader never got the key; the owner's log: {:?}", lock(&a.lines).clone());
    wait_for(Duration::from_secs(10), || async { db::content::get(b.service.db(), &post).await.ok().flatten().is_some() }).await;
    b.service.send_comment(&b.keys, &cid, &post, None, "a comment of the reader", messenger_core::TextFormat::Plain).await.unwrap();
    let on_relay = || {
        lock(&network.events)
            .iter()
            .find(|(e, _)| e["kind"].as_u64() == Some(u64::from(communities_core::wire::KIND_CONTENT)) && e["tags"].as_array().is_some_and(|t| t.iter().any(|t| t[0] == "e" && t[1] == post.as_str())))
            .map(|(e, _)| e["id"].as_str().unwrap_or_default().to_string())
    };
    wait_for(Duration::from_secs(10), || async { on_relay().is_some() }).await;
    let comment = on_relay().unwrap();

    // The owner holds it, heard live: no catch-up went through meanwhile.
    let comments = || async { db::content::comments(a.service.db(), &cid, &post).await.unwrap_or_default().len() };
    let heard = wait_until(Duration::from_secs(10), || async { comments().await == 1 }).await;
    assert!(heard, "the owner never got the comment; live: {}; the owner's log: {:?}", lock(&a.pool.live_ids).contains(&comment), lock(&a.lines).clone());
    assert!(lock(&a.pool.live_ids).contains(&comment), "heard live");
    assert!(!lock(&a.pool.paged_ids).contains(&comment), "not paged: every catch-up was held");

    // And a catch-up round once the relays answer again finds it as well.
    drop(slow);
    a.listener.resubscribe_anew();
    wait_for(Duration::from_secs(10), || async { lock(&a.pool.paged_ids).contains(&comment) }).await;
    until_quiet(&a.listener).await;
    assert_eq!(comments().await, 1, "the same comment, once");
    a.stop();
    b.stop();
}

/// A round that did not go out whole is tried again by itself, the waits
/// growing, until one goes out: with no relay at the start, the
/// subscriptions of the channel are made once the relays are there,
/// though nothing else asks for a round.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_round_that_did_not_go_out_is_tried_again() {
    let network = Arc::new(Network::default());
    let hosts = Arc::new(MemHosts::default());
    let dir = std::env::temp_dir().join(format!("communities-retry-{}-{}", std::process::id(), now_secs()));
    let owner = Keys::generate();
    let secrets: Arc<dyn SecretStore> = Arc::new(super::tests::MemSecrets::default());
    let (cid, _) = made_before(&network, &hosts, &dir.join("owner.db"), &owner, &secrets).await;

    let waits = vec![Duration::from_millis(30), Duration::from_millis(60)];
    let a = App::start(&network, owner, Db::open(&dir.join("owner.db")).await.unwrap(), secrets, hosts, waits).await;
    a.listener.resubscribe();
    wait_for(Duration::from_secs(5), || async { a.listener.flight.failed.load(Ordering::SeqCst) >= 3 }).await;
    assert!(lock(&a.pool.subs).is_empty());
    a.pool.connect();
    let notes = communities_core::transport::sub_id(&cid, PART_NOTES).0;
    wait_for(Duration::from_secs(5), || async { lock(&a.pool.subs).contains_key(&notes) && lock(&a.pool.subs).contains_key(&own_notes_sub().0) }).await;
    wait_for(Duration::from_secs(5), || async { a.listener.flight.failed.load(Ordering::SeqCst) == 0 }).await;
    // Whole now: no try waits any more.
    let rounds = || lock(&a.pool.subs).len();
    let before = rounds();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(rounds(), before);
    assert!(hold(&a.listener.flight.retry).is_none(), "no try waits once a round went out whole");
    a.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

/// A relay that dropped and came back between two looks is listened to
/// again; one that stays connected asks for nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_relay_that_comes_back_is_listened_to_again() {
    let network = Arc::new(Network::default());
    let hosts = Arc::new(MemHosts::default());
    let a = App::start(&network, Keys::generate(), Db::open_in_memory().await.unwrap(), Arc::new(super::tests::MemSecrets::default()), hosts, vec![NO_RETRY]).await;
    a.pool.connect();
    a.listener.resubscribe();
    until_quiet(&a.listener).await;
    let own = own_notes_sub().0;
    assert!(lock(&a.pool.subs).contains_key(&own));
    a.listener.watch_relays().await;
    until_quiet(&a.listener).await;
    lock(&a.pool.subs).clear();
    a.listener.watch_relays().await;
    until_quiet(&a.listener).await;
    assert!(lock(&a.pool.subs).is_empty(), "connected all along: no round");
    lock(&a.pool.configured).clear();
    a.listener.watch_relays().await;
    a.pool.connect();
    lock(&a.pool.asked).clear();
    a.listener.watch_relays().await;
    until_quiet(&a.listener).await;
    assert!(lock(&a.pool.subs).contains_key(&own), "back: listened to again");
    let asked = lock(&a.pool.asked).clone();
    assert_eq!(asked.iter().filter(|id| **id == own).count(), 1, "once: {asked:?}");
    a.listener.watch_relays().await;
    until_quiet(&a.listener).await;
    assert_eq!(lock(&a.pool.asked).len(), asked.len(), "and not again while it stays");

    // The inbox relay of a peer, reached for a moment to send there (the
    // pool's `publish_to_inbox`): nothing of the module is on it.
    lock(&a.pool.configured).insert("wss://peer.example".into());
    a.listener.watch_relays().await;
    lock(&a.pool.configured).remove("wss://peer.example");
    a.listener.watch_relays().await;
    until_quiet(&a.listener).await;
    assert_eq!(lock(&a.pool.asked).len(), asked.len(), "a relay the module listens to nothing on asks for no round");
    a.stop();
}

/// The owner's phone right after a channel and its section were made
/// (живой запуск 2026-10-23 02:16: «an event of kind 1059 held the loop
/// 56 s», 26–56 s in every run of that night, and a join by the link
/// waited behind it). The owner has more than a page of the messenger's
/// wraps (two days of its receipts and presence); the new channel's
/// relays are a set whose notes to me were not paged in this run, so the
/// round after the making pages them there, and the wrap that filled the
/// page asked for the page before it of every channel followed, on the
/// loop, each a `Sync` the pool awaits to its end. Over relays that
/// answer every request but a subscription in 5 s and hosts that never
/// answer, every event of the owner's loop — the pages of wraps, the
/// echoes of my own issues and of the copy of my keys, the operations —
/// is taken in < 300 ms, and the page before a full one is still asked
/// for.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_channel_made_now_holds_no_event_of_its_owner_over_slow_relays_and_hosts() {
    let network = Arc::new(Network::default());
    let hosts = Arc::new(MemHosts::default());
    let dir = std::env::temp_dir().join(format!("communities-made-now-{}-{}", std::process::id(), now_secs()));
    let owner = Keys::generate();
    let me = PubKey::parse(&owner.public_key().to_hex()).unwrap();
    // More than a page of the messenger's peer notes to me, on both relays.
    let friend = Keys::generate();
    let now = i64::try_from(now_secs()).unwrap();
    let both: HashSet<String> = HashSet::from([RELAY.to_string(), RELAY2.to_string()]);
    let page = communities_core::transport::CATCH_UP_LIMIT;
    for i in 0..page + 40 {
        let w = communities_core::notes::wrap_note(&friend, &me, &format!("{{\"t\":\"receipt\",\"n\":{i}}}"), now - i * 60).unwrap();
        network.publish(w.json, both.clone());
    }
    let a = App::start(&network, owner.clone(), Db::open(&dir.join("owner.db")).await.unwrap(), Arc::new(super::tests::MemSecrets::default()), hosts.clone(), vec![NO_RETRY]).await;
    a.pool.connect_both();
    a.listener.resubscribe();
    until_quiet(&a.listener).await;
    // The round of the start paged them on my own relays: no channel yet.
    let paged = || a.pool.paged.load(std::sync::atomic::Ordering::SeqCst);
    wait_for(Duration::from_secs(10), || async { paged() >= usize::try_from(page).unwrap() }).await;
    let taken = || lock(&a.took).len();
    wait_for(Duration::from_secs(20), || async { taken() >= paged() }).await;

    // From here on every request but a subscription waits 5 s for its
    // relays, and no host answers a put or a get.
    *lock(&a.pool.delay) = Duration::from_secs(5);
    *lock(&hosts.hang) = Some(Arc::new(tokio::sync::Notify::new()));
    *lock(&hosts.get_gate) = Some(Arc::new(tokio::sync::Semaphore::new(0)));
    lock(&a.took).clear();
    let (syncs_before, published_before) = (lock(&a.pool.syncs).len(), lock(&network.events).len());

    // `channel_create`, as the command does it, on one of my two relays.
    let new = NewChannel {
        name: "Made now".into(),
        about: String::new(),
        picture: String::new(),
        join: JoinMode::Open,
        history: Default::default(),
        relays: vec![RELAY.into()],
        storage: vec![HOST.into()],
    };
    let started = std::time::Instant::now();
    let (cid, outcome) = {
        let _creating = a.listener.routes.creating(vec![RelayUrl::parse(RELAY).unwrap()]).await;
        let made = a.service.create(&owner, new).await.unwrap();
        a.listener.refresh_routes().await;
        made
    };
    assert!(outcome.resubscribe);
    a.listener.resubscribe();
    // And its section, as `channel_scope_add` does it.
    a.service.scope_add(&owner, &cid, "Section", communities_core::SchemeId::SdV1, communities_core::History::Open).await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(1), "the commands answer before the relays: {:?}", started.elapsed());

    // The round after the making pages the notes to me on the channel's
    // relays (a set not paged in this run), and the page before the full
    // one is asked for.
    let next_page = || lock(&a.pool.syncs)[syncs_before..].iter().any(|f| f.get("until").is_some() && f["#p"].as_array().is_some_and(|p| p.iter().any(|k| k.as_str() == Some(me.as_hex()))));
    let pages = wait_until(Duration::from_secs(40), || async { next_page() }).await;
    // My own issue and the copy of my keys, out by the outbox and heard
    // back live on the subscriptions of the notes to me.
    let mine = || {
        let events = lock(&network.events);
        let live = lock(&a.pool.live_ids);
        events[published_before..]
            .iter()
            .filter(|(e, _)| e["kind"].as_u64() == Some(u64::from(communities_core::notes::KIND_WRAP)) && served_to(e, me.as_hex()))
            .filter(|(e, _)| live.iter().any(|id| e["id"].as_str() == Some(id)))
            .count()
    };
    let echoed = wait_until(Duration::from_secs(60), || async { mine() >= 2 }).await;
    // Every event handed to the loop by now taken.
    let quiet = wait_until(Duration::from_secs(20), || async {
        let before = taken();
        tokio::time::sleep(Duration::from_millis(300)).await;
        taken() == before
    })
    .await;
    let took = lock(&a.took).clone();
    let held = lock(&a.held_at).clone();
    let worst = took.iter().map(|(_, d)| *d).max().unwrap_or_default();
    assert!(held.is_empty() && worst < Duration::from_millis(300), "events held the loop (kind, how long, at): {held:?}; worst {worst:?} of {}", took.len());
    assert!(pages, "the page before a full page of the notes to me was never asked for: {:?}", &lock(&a.pool.syncs)[syncs_before..]);
    assert!(echoed, "my own issue and keys were not heard back live: {}; the log: {:?}", mine(), lock(&a.lines));
    assert!(quiet, "the loop never came to rest");
    a.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Until the listener ran every round asked for.
async fn until_quiet(listener: &Listener) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    tokio::time::sleep(Duration::from_millis(20)).await;
    while listener.busy() {
        assert!(tokio::time::Instant::now() < deadline, "the round never ended");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn wait_for<F, Fut>(patience: Duration, what: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    assert!(wait_until(patience, what).await, "waited too long");
}

async fn wait_until<F, Fut>(patience: Duration, what: F) -> bool
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + patience;
    while tokio::time::Instant::now() < deadline {
        if what().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}
