// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Fakes of the world a channel lives in, for the tests of the crate: a
//! relay that keeps every event and an inbox per key, blob hosts that can
//! go down or refuse after a number of puts, a clock moved by hand, a
//! secret store in memory, and a `Node` (one device) that pumps what came
//! to it into its `ChannelService`.

use crate::blobs::BlobHosts;
use crate::db::Db;
use crate::service::ChannelService;
use async_trait::async_trait;
use messenger_core::traits::RelayStatusSnapshot;
use messenger_core::{Ack, Clock, MessengerError, Outbound, PubKey, RawEvent, RelayUrl, Result, SecretStore, Timestamp, Transport};
use nostr::key::Keys;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use zeroize::Zeroizing;

pub const T0: i64 = 1_760_000_000;
pub const H1: &str = "https://h1.example";
pub const H2: &str = "https://h2.example";
pub const H3: &str = "https://h3.example";

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A clock moved by hand, shared by every node of a test.
#[derive(Debug)]
pub struct FakeClock(AtomicI64);

impl FakeClock {
    pub fn new() -> Arc<Self> {
        Arc::new(Self(AtomicI64::new(T0)))
    }

    pub fn advance(&self, secs: i64) {
        self.0.fetch_add(secs, Ordering::SeqCst);
    }

    pub fn now(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.0.load(Ordering::SeqCst))
    }
}

/// The secret store of one device.
#[derive(Default)]
pub struct MemSecrets(Mutex<HashMap<String, Vec<u8>>>);

#[async_trait]
impl SecretStore for MemSecrets {
    async fn get(&self, key: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        Ok(lock(&self.0).get(key).cloned().map(Zeroizing::new))
    }

    async fn put(&self, key: &str, value: &[u8]) -> Result<()> {
        lock(&self.0).insert(key.to_string(), value.to_vec());
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<()> {
        lock(&self.0).remove(key);
        Ok(())
    }

    async fn is_unlocked(&self) -> bool {
        true
    }
}

/// The relays of a test: every scoped event in order, and the gift wraps
/// by the key they are for.
#[derive(Default)]
pub struct Relay {
    pub events: Mutex<Vec<serde_json::Value>>,
    pub inbox: Mutex<HashMap<String, Vec<serde_json::Value>>>,
    /// Kinds the relay refuses for now (a publish that fails).
    pub refuse: Mutex<BTreeSet<u16>>,
    /// The most events the relay hands back for one request (strfry: 500).
    pub cap: Mutex<Option<usize>>,
    /// Only the gift wraps to these keys are kept (`None`: all); the rest
    /// are counted by the sender's `Net` and dropped (a bench of a million
    /// issues would hold gigabytes).
    pub keep_inbox: Mutex<Option<BTreeSet<String>>>,
}

impl Relay {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The events of a kind on the relay.
    pub fn of_kind(&self, kind: u16) -> Vec<serde_json::Value> {
        lock(&self.events).iter().filter(|e| e["kind"].as_u64() == Some(u64::from(kind))).cloned().collect()
    }

    /// The epochs (tag `n`) of the epoch events on the relay.
    pub fn epochs(&self) -> Vec<u32> {
        self.of_kind(crate::wire::KIND_EPOCH)
            .iter()
            .filter_map(|e| e["tags"].as_array()?.iter().find(|t| t[0] == "n")?[1].as_str()?.parse().ok())
            .collect()
    }

    /// The epochs of one scope (tag `s`).
    pub fn epochs_of(&self, scope_id: &str) -> Vec<u32> {
        self.of_kind(crate::wire::KIND_EPOCH)
            .iter()
            .filter(|e| e["tags"].as_array().is_some_and(|t| t.iter().any(|t| t[0] == "s" && t[1] == scope_id)))
            .filter_map(|e| e["tags"].as_array()?.iter().find(|t| t[0] == "n")?[1].as_str()?.parse().ok())
            .collect()
    }

    pub fn refuse(&self, kind: u16, on: bool) {
        let mut r = lock(&self.refuse);
        if on {
            r.insert(kind);
        } else {
            r.remove(&kind);
        }
    }
}

/// Does a NIP-01 filter take this event: `kinds`, `ids`, `since`,
/// `until` and every `#x` tag filter.
pub fn matches(filter: &serde_json::Value, e: &serde_json::Value) -> bool {
    let Some(f) = filter.as_object() else { return false };
    for (k, v) in f {
        let ok = match k.as_str() {
            "kinds" => v.as_array().is_some_and(|ks| ks.iter().any(|x| x.as_u64() == e["kind"].as_u64())),
            "ids" => v.as_array().is_some_and(|ids| ids.iter().any(|x| x.as_str() == e["id"].as_str())),
            "since" => e["created_at"].as_i64() >= v.as_i64(),
            "until" => e["created_at"].as_i64() <= v.as_i64(),
            "limit" => true,
            tag if tag.starts_with('#') => {
                let name = &tag[1..];
                let want: Vec<&str> = v.as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
                e["tags"].as_array().is_some_and(|tags| tags.iter().any(|t| t[0].as_str() == Some(name) && t[1].as_str().is_some_and(|x| want.contains(&x))))
            }
            _ => false,
        };
        if !ok {
            return false;
        }
    }
    true
}

/// The transport of one device: everything goes to the shared relay; a
/// note to myself goes to my own inbox. A `Sync` is answered from the
/// relay into `synced` (read at the next pump as history); a
/// `Subscribe` is only written down: every device hears everything.
pub struct Net {
    relay: Arc<Relay>,
    me: PubKey,
    pub sent: AtomicUsize,
    pub subscribed: Mutex<Vec<(String, serde_json::Value)>>,
    pub syncs: Mutex<Vec<serde_json::Value>>,
    /// Every note to an inbox: the recipient and how many relays it
    /// named as the hint.
    pub hints: Mutex<Vec<(String, usize)>>,
    synced: Mutex<Vec<serde_json::Value>>,
    /// Gift wraps sent and their bytes (the JSON of the event).
    pub wraps: AtomicUsize,
    pub wrap_bytes: AtomicUsize,
    /// Events of a channel published (`PublishScoped`) and their bytes.
    pub scoped: AtomicUsize,
    pub scoped_bytes: AtomicUsize,
}

fn json_len(v: &serde_json::Value) -> usize {
    serde_json::to_string(v).map_or(0, |s| s.len())
}

impl Net {
    /// What the `Sync`s brought since the last call (a pump reads it as
    /// history too), for a test that reads it itself.
    pub fn take_synced(&self) -> Vec<serde_json::Value> {
        lock(&self.synced).drain(..).collect()
    }
}

#[async_trait]
impl Transport for Net {
    async fn send(&self, out: Outbound) -> Result<Ack> {
        self.sent.fetch_add(1, Ordering::SeqCst);
        let relay = RelayUrl::parse("wss://r1.example").ok_or_else(|| MessengerError::Other("relay".into()))?;
        match out {
            Outbound::PublishScoped { event, .. } => {
                let kind = event.json["kind"].as_u64().and_then(|k| u16::try_from(k).ok()).unwrap_or(0);
                if lock(&self.relay.refuse).contains(&kind) {
                    return Ok(Ack { accepted_by: vec![], rejected_by: vec![(relay, "refused".into())] });
                }
                self.scoped.fetch_add(1, Ordering::SeqCst);
                self.scoped_bytes.fetch_add(json_len(&event.json), Ordering::SeqCst);
                lock(&self.relay.events).push(event.json);
            }
            Outbound::PublishToInbox { recipient, event, hint_relays } => {
                if lock(&self.relay.refuse).contains(&crate::notes::KIND_WRAP) {
                    return Ok(Ack { accepted_by: vec![], rejected_by: vec![(relay, "timeout".into())] });
                }
                self.wraps.fetch_add(1, Ordering::SeqCst);
                self.wrap_bytes.fetch_add(json_len(&event.json), Ordering::SeqCst);
                let keep = lock(&self.relay.keep_inbox).as_ref().is_none_or(|k| k.contains(recipient.as_hex()));
                if !keep {
                    return Ok(Ack { accepted_by: vec![relay], rejected_by: vec![] });
                }
                lock(&self.hints).push((recipient.as_hex().to_string(), hint_relays.len()));
                lock(&self.relay.inbox).entry(recipient.as_hex().to_string()).or_default().push(event.json);
            }
            Outbound::PublishOwn { event } => {
                lock(&self.relay.inbox).entry(self.me.as_hex().to_string()).or_default().push(event.json);
            }
            Outbound::Subscribe { id, filter, .. } => lock(&self.subscribed).push((id.0, filter.0)),
            Outbound::Sync { filter, .. } => {
                let mut found: Vec<serde_json::Value> = lock(&self.relay.events).iter().filter(|e| matches(&filter.0, e)).cloned().collect();
                found.extend(lock(&self.relay.inbox).values().flatten().filter(|e| matches(&filter.0, e)).cloned());
                found.sort_by_key(|e| std::cmp::Reverse(e["created_at"].as_i64().unwrap_or(0)));
                if let Some(n) = filter.0["limit"].as_u64() {
                    found.truncate(usize::try_from(n).unwrap_or(usize::MAX));
                }
                if let Some(cap) = *lock(&self.relay.cap) {
                    found.truncate(cap);
                }
                lock(&self.syncs).push(filter.0);
                lock(&self.synced).extend(found);
            }
            _ => {}
        }
        Ok(Ack { accepted_by: vec![relay], rejected_by: vec![] })
    }

    fn events(&self) -> tokio::sync::mpsc::Receiver<RawEvent> {
        tokio::sync::mpsc::channel(1).1
    }

    async fn status(&self) -> RelayStatusSnapshot {
        RelayStatusSnapshot::default()
    }
}

/// Blob hosts in memory, with a host that is down and a limit of puts
/// after which every put fails (a crash in the middle of an upload).
#[derive(Default)]
pub struct MemHosts {
    pub blobs: Mutex<HashMap<String, BTreeMap<String, Vec<u8>>>>,
    pub down: Mutex<BTreeSet<String>>,
    pub puts: AtomicUsize,
    pub deletes: AtomicUsize,
    pub gets: AtomicUsize,
    /// The bytes of the puts taken and of the gets that found a blob.
    pub put_bytes: AtomicUsize,
    pub get_bytes: AtomicUsize,
    /// Puts left before every put fails; `None`: no limit.
    pub budget: Mutex<Option<usize>>,
    /// Puts left before every put is refused as a host refuses a key over
    /// its plan (no retry helps); `None`: no limit.
    pub refuse_after: Mutex<Option<usize>>,
    /// Hosts that refuse every put as a host bound to accounts refuses a
    /// key it does not know (vstore §10.2).
    pub refusing: Mutex<BTreeSet<String>>,
    /// Who signed each put, by host: (host, pubkey hex).
    pub signers: Mutex<Vec<(String, String)>>,
    /// Every put waits while this holds a notice (a host that never
    /// answers), until `notify_waiters`.
    pub hang: Mutex<Option<Arc<tokio::sync::Notify>>>,
    /// Every put takes this long first (a slow host).
    pub put_delay: Mutex<Option<std::time::Duration>>,
    /// Every get waits while this holds a semaphore with no permit (a host
    /// that never answers a read), until it is closed.
    pub get_gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
    /// Hosts that are servers of the kit with `MEDIA_UPLOAD_KEY=required`
    /// (wire §9.1): their app key, and whether they take upload keys.
    pub kit: Mutex<HashMap<String, (String, bool)>>,
    /// Upload keys revoked on a host: `(host, label, pubkey)` → when.
    pub revoked: Mutex<HashMap<(String, String, String), i64>>,
    /// Writes a host of the kit refused for the key: (host, pubkey hex).
    pub key_refused: Mutex<Vec<(String, String)>>,
    /// The clock of the world, for the life of an upload key.
    pub clock: Mutex<Option<Arc<FakeClock>>>,
}

impl MemHosts {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn set_down(&self, host: &str, down: bool) {
        let mut d = lock(&self.down);
        if down {
            d.insert(host.to_string());
        } else {
            d.remove(host);
        }
    }

    /// `host` a server of the kit that needs `app_key` (`scoped`: it takes
    /// upload keys too).
    pub fn set_kit(&self, host: &str, app_key: &str, scoped: bool) {
        lock(&self.kit).insert(host.to_string(), (app_key.to_string(), scoped));
    }

    /// Whether a write by `signer` with `key` passes the gate of the kit on
    /// `host` (`upload.js`): the app key, or an upload key of the signer,
    /// alive and not revoked.
    fn passes(&self, signer: &Keys, key: Option<&str>, host: &str) -> bool {
        let Some((app, scoped)) = lock(&self.kit).get(host).cloned() else { return true };
        let pubkey = signer.public_key().to_hex();
        let now = lock(&self.clock).as_ref().map_or(T0, |c| c.now());
        let ok = match key {
            Some(k) if k == app => true,
            Some(k) if scoped && crate::upload_keys::verify(&app, host, k, &pubkey, now) => {
                let p = crate::upload_keys::parse(k).unwrap_or_else(|| unreachable!("verified"));
                lock(&self.revoked).get(&(host.to_string(), p.label, p.pubkey)).is_none_or(|at| p.iat > *at)
            }
            _ => false,
        };
        if !ok {
            lock(&self.key_refused).push((host.to_string(), pubkey));
        }
        ok
    }

    /// How many blobs a host holds.
    pub fn count(&self, host: &str) -> usize {
        lock(&self.blobs).get(host).map_or(0, BTreeMap::len)
    }
}

#[async_trait]
impl BlobHosts for MemHosts {
    async fn put(&self, signer: &Keys, host: &str, sha256: &str, bytes: &[u8]) -> Result<()> {
        self.put_keyed(signer, None, host, sha256, bytes).await
    }

    async fn put_typed_keyed(&self, signer: &Keys, key: Option<&str>, host: &str, sha256: &str, bytes: &[u8], _content_type: &str) -> Result<()> {
        self.put_keyed(signer, key, host, sha256, bytes).await
    }

    async fn gate(&self, host: &str) -> Option<crate::upload_keys::Gate> {
        if lock(&self.down).contains(host) {
            return None;
        }
        Some(match lock(&self.kit).get(host) {
            Some((_, scoped)) => crate::upload_keys::Gate::Key { scoped: *scoped },
            None => crate::upload_keys::Gate::Open,
        })
    }

    async fn revoke(&self, app_key: &str, host: &str, label: &str, pubkey: &str) -> Result<()> {
        match lock(&self.kit).get(host).cloned() {
            Some((app, true)) if app == app_key => {
                let now = lock(&self.clock).as_ref().map_or(T0, |c| c.now());
                lock(&self.revoked).insert((host.to_string(), label.to_string(), pubkey.to_string()), now);
                Ok(())
            }
            Some((app, _)) if app != app_key => Err(MessengerError::Transport("err.auth_failed: http 401".into())),
            _ => Ok(()),
        }
    }

    async fn put_keyed(&self, signer: &Keys, key: Option<&str>, host: &str, sha256: &str, bytes: &[u8]) -> Result<()> {
        if lock(&self.down).contains(host) {
            return Err(MessengerError::Transport(format!("err.network: {host} is down")));
        }
        if !self.passes(signer, key, host) {
            return Err(MessengerError::Transport("err.auth_failed: nginx 401".into()));
        }
        if lock(&self.refusing).contains(host) {
            return Err(MessengerError::Transport("err.auth_failed: blossom 403: no_account".into()));
        }
        lock(&self.signers).push((host.to_string(), signer.public_key().to_hex()));
        let delay = *lock(&self.put_delay);
        if let Some(d) = delay {
            tokio::time::sleep(d).await;
        }
        let gate = lock(&self.hang).clone();
        if let Some(gate) = gate {
            gate.notified().await;
        }
        {
            let mut left = lock(&self.refuse_after);
            if let Some(left) = left.as_mut() {
                if *left == 0 {
                    return Err(MessengerError::Transport("err.auth_failed: blossom 403: plan_limit".into()));
                }
                *left -= 1;
            }
        }
        {
            let mut budget = lock(&self.budget);
            if let Some(left) = budget.as_mut() {
                if *left == 0 {
                    return Err(MessengerError::Transport("err.network: crashed".into()));
                }
                *left -= 1;
            }
        }
        if crate::blobs::sha256_hex(bytes) != sha256 {
            return Err(MessengerError::Invalid("hash".into()));
        }
        self.puts.fetch_add(1, Ordering::SeqCst);
        self.put_bytes.fetch_add(bytes.len(), Ordering::SeqCst);
        lock(&self.blobs).entry(host.to_string()).or_default().insert(sha256.to_string(), bytes.to_vec());
        Ok(())
    }

    async fn get(&self, host: &str, sha256: &str) -> Result<Option<Vec<u8>>> {
        self.gets.fetch_add(1, Ordering::SeqCst);
        let gate = lock(&self.get_gate).clone();
        if let Some(gate) = gate {
            let _ = gate.acquire().await;
        }
        if lock(&self.down).contains(host) {
            return Err(MessengerError::Transport(format!("err.network: {host} is down")));
        }
        let found = lock(&self.blobs).get(host).and_then(|b| b.get(sha256).cloned());
        self.get_bytes.fetch_add(found.as_ref().map_or(0, Vec::len), Ordering::SeqCst);
        Ok(found)
    }

    async fn delete(&self, signer: &Keys, host: &str, sha256: &str) -> Result<()> {
        self.delete_keyed(signer, None, host, sha256).await
    }

    async fn delete_keyed(&self, signer: &Keys, key: Option<&str>, host: &str, sha256: &str) -> Result<()> {
        if lock(&self.down).contains(host) {
            return Err(MessengerError::Transport(format!("err.network: {host} is down")));
        }
        if !self.passes(signer, key, host) {
            return Err(MessengerError::Transport("err.auth_failed: nginx 401".into()));
        }
        self.deletes.fetch_add(1, Ordering::SeqCst);
        if let Some(b) = lock(&self.blobs).get_mut(host) {
            b.remove(sha256);
        }
        Ok(())
    }
}

/// The world of a test: one relay, one set of hosts, one clock.
pub struct World {
    pub relay: Arc<Relay>,
    pub hosts: Arc<MemHosts>,
    pub clock: Arc<FakeClock>,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    pub fn new() -> Self {
        let (hosts, clock) = (MemHosts::new(), FakeClock::new());
        *lock(&hosts.clock) = Some(clock.clone());
        Self { relay: Relay::new(), hosts, clock }
    }

    /// A device of a new person.
    pub async fn node(&self) -> Node {
        self.device(Keys::generate()).await
    }

    /// A device of the person of `keys` (a second device of somebody).
    pub async fn device(&self, keys: Keys) -> Node {
        let db = Db::open_in_memory().await.unwrap_or_else(|e| panic!("db: {e}"));
        self.device_on(keys, db)
    }

    /// A device of the person of `keys` on a database of the caller's (a
    /// file, for the bench).
    pub fn device_on(&self, keys: Keys, db: Db) -> Node {
        let me = PubKey::parse(&keys.public_key().to_hex()).unwrap_or_else(|| unreachable!("a generated key parses"));
        let net = Arc::new(Net {
            relay: self.relay.clone(),
            me: me.clone(),
            sent: AtomicUsize::new(0),
            subscribed: Mutex::new(Vec::new()),
            syncs: Mutex::new(Vec::new()),
            hints: Mutex::new(Vec::new()),
            synced: Mutex::new(Vec::new()),
            wraps: AtomicUsize::new(0),
            wrap_bytes: AtomicUsize::new(0),
            scoped: AtomicUsize::new(0),
            scoped_bytes: AtomicUsize::new(0),
        });
        // No automatic attempts of a file: a failure is told at once.
        let svc = ChannelService::new(db, Arc::new(MemSecrets::default()), net.clone(), self.hosts.clone(), self.clock.clone()).with_retry_delays(Vec::new());
        svc.set_signer(Some(keys.clone()));
        Node { keys, me, svc, net, events_seen: Mutex::new(0), inbox_seen: Mutex::new(HashMap::new()), hold: Mutex::new(BTreeSet::new()), held: Mutex::new(Vec::new()), errors: Mutex::new(Vec::new()), wake_at: Mutex::new(None) }
    }

    /// Pumps until nothing new arrives or goes out anywhere.
    pub async fn pump(&self, nodes: &[&Node]) {
        for _ in 0..30 {
            let mut any = false;
            for n in nodes {
                any |= n.sync().await;
            }
            if !any {
                return;
            }
        }
    }
}

/// One device: its key, its service, what it has taken of the relay.
pub struct Node {
    pub keys: Keys,
    pub me: PubKey,
    pub svc: ChannelService,
    pub net: Arc<Net>,
    events_seen: Mutex<usize>,
    inbox_seen: Mutex<HashMap<String, usize>>,
    /// Kinds this device does not hear for now (kept for `release`).
    hold: Mutex<BTreeSet<u16>>,
    held: Mutex<Vec<serde_json::Value>>,
    pub errors: Mutex<Vec<String>>,
    /// When the keepers' turn is due, as the last one and the outcomes
    /// asked (the loop's `wake_at`).
    wake_at: Mutex<Option<u64>>,
}

impl Node {
    /// Stops (`true`) or resumes hearing events of a kind; resumed, what
    /// was held is heard at the next pump.
    pub fn hold(&self, kind: u16, on: bool) {
        let mut h = lock(&self.hold);
        if on {
            h.insert(kind);
        } else {
            h.remove(&kind);
        }
    }

    /// Skips everything on the relay so far (a device offline).
    pub fn skip_all(&self, relay: &Relay) {
        *lock(&self.events_seen) = lock(&relay.events).len();
    }

    /// Skips every gift wrap on the relay so far (a device that was off
    /// while they came: only a catch-up brings them).
    pub fn skip_inboxes(&self, relay: &Relay) {
        let lens: Vec<(String, usize)> = lock(&relay.inbox).iter().map(|(k, v)| (k.clone(), v.len())).collect();
        lock(&self.inbox_seen).extend(lens);
    }

    fn note_err(&self, e: MessengerError) {
        lock(&self.errors).push(e.to_string());
    }

    /// A turn of the keepers is due: a command asked for one, an outcome
    /// did, or the last one's time came.
    fn turn_due(&self, now: u64) -> bool {
        self.svc.take_turn_asked() || lock(&self.wake_at).is_some_and(|w| w <= now)
    }

    fn wake(&self, at: Option<u64>) {
        if let Some(at) = at {
            let mut w = lock(&self.wake_at);
            *w = Some(w.map_or(at, |have| have.min(at)));
        }
    }

    /// What the outbox has to send, sent; the keepers' turn once the
    /// operations went (as the worker wakes the loop). `true` when
    /// anything went out.
    async fn flush(&self) -> bool {
        match self.svc.flush_outbox().await {
            Ok(f) => {
                self.wake(f.wake);
                if !f.woke.is_empty() {
                    self.wake(Some(0));
                }
                f.sent > 0
            }
            Err(e) => {
                self.note_err(e);
                false
            }
        }
    }

    /// One pass of the device as the module runs it: the outbox sent,
    /// what came taken in (new events of the relay, in order; events held
    /// before and released; gift wraps to me and to every keeper kept
    /// here), the keepers' turn when due, what it left to send sent.
    /// `true` when anything came or went out.
    pub async fn sync(&self) -> bool {
        let relay = &self.net.relay;
        let mut any = self.flush().await;
        let held: Vec<serde_json::Value> = {
            let hold = lock(&self.hold).clone();
            let mut h = lock(&self.held);
            let (now, keep): (Vec<_>, Vec<_>) = h.drain(..).partition(|e| !hold.contains(&(e["kind"].as_u64().unwrap_or(0) as u16)));
            *h = keep;
            now
        };
        let fresh: Vec<serde_json::Value> = {
            let mut seen = lock(&self.events_seen);
            let all = lock(&relay.events);
            let out = all[*seen..].to_vec();
            *seen = all.len();
            out
        };
        // What a Sync brought: history, not live.
        let synced: Vec<serde_json::Value> = lock(&self.net.synced).drain(..).collect();
        for ev in synced {
            any = true;
            match self.svc.on_event(&self.keys, &ev, false).await {
                Ok(out) => self.wake(out.wake),
                Err(e) => self.note_err(e),
            }
        }
        for ev in held.into_iter().chain(fresh) {
            let kind = ev["kind"].as_u64().unwrap_or(0) as u16;
            if lock(&self.hold).contains(&kind) {
                lock(&self.held).push(ev);
                continue;
            }
            any = true;
            match self.svc.on_event(&self.keys, &ev, true).await {
                Ok(out) => self.wake(out.wake),
                Err(e) => self.note_err(e),
            }
        }
        // My inbox, and a keeper's own only where its journal names that
        // key as the inbox (before `inbox`, wire §3): a relay with vguard
        // serves a `#p` only to the connection signed in as it.
        let mut inboxes = vec![self.me.as_hex().to_string()];
        if let Ok(kept) = self.svc.kept().await {
            for cid in kept {
                let named = self.svc.log(&cid).await.ok().flatten().and_then(|l| l.state().keeper().map(|k| k.inbox().clone()));
                if let Ok(Some(k)) = self.svc.keeper_pubkey(&cid).await {
                    if named.as_ref() == Some(&k) {
                        inboxes.push(k.as_hex().to_string());
                    }
                }
            }
        }
        for who in inboxes {
            let wraps: Vec<serde_json::Value> = {
                let mut seen = lock(&self.inbox_seen);
                let at = seen.entry(who.clone()).or_insert(0);
                let all = lock(&relay.inbox);
                let list = all.get(&who).cloned().unwrap_or_default();
                let out = list[(*at).min(list.len())..].to_vec();
                *at = list.len();
                out
            };
            for w in wraps {
                any = true;
                match self.svc.on_wrap(&self.keys, &w).await {
                    Ok(out) => self.wake(out.and_then(|o| o.wake)),
                    Err(e) => self.note_err(e),
                }
            }
        }
        // What the events wanted of the relays (the page before a full
        // one, the heads a snapshot names) and the fetches of the member's
        // side (an epoch to open, a shard of a snapshot), as the module runs
        // them beside the loop: here one after the other, once the events
        // are taken; what the asks bring is taken by the next call.
        any |= self.svc.run_asks().await > 0;
        any |= self.fetch().await;
        // The keepers' turn when due, as the loop runs it; what it left
        // to send (an issue, an epoch's wake) goes out at once.
        let now = u64::try_from(self.svc.now()).unwrap_or(0);
        if self.turn_due(now) {
            *lock(&self.wake_at) = None;
            match self.svc.turn().await {
                Ok(turned) => {
                    self.wake(turned.wake);
                    // The jobs of the hosts, as the module runs them beside
                    // the loop: here one after the other, before the rest.
                    any |= self.svc.run_jobs(turned.jobs).await > 0;
                    for job in turned.fetches {
                        let out = self.svc.run_fetch(&self.keys, job).await;
                        self.wake(out.wake);
                        any = true;
                    }
                    any |= self.fetch().await;
                    any |= self.svc.run_asks().await > 0;
                }
                Err(e) => self.note_err(e),
            }
            any |= self.flush().await;
        }
        any
    }

    /// Every fetch of the member's side due now, run (`run_fetch`), until
    /// none is; `true` when any ran.
    async fn fetch(&self) -> bool {
        let mut any = false;
        loop {
            let jobs = self.svc.take_fetches();
            if jobs.is_empty() {
                return any;
            }
            for job in jobs {
                let out = self.svc.run_fetch(&self.keys, job).await;
                self.wake(out.wake);
                any = true;
            }
        }
    }

    pub fn errors(&self) -> Vec<String> {
        lock(&self.errors).clone()
    }
}
