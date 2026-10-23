// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! One bot: a person on one device, as the app runs it (the loop of
//! `crates/communities/src/state.rs`): its key, its `communities.db`, its
//! secrets in a file, a `ChannelService` over the hub; what comes is
//! taken in order, a `resubscribe` is a catch-up and the subscriptions
//! again, the keepers' cycle runs when it asks and at least every 30 s.

use crate::net::{BotId, BotNet, Hub, Item};
use crate::store::{Counters, CountingHosts, FileSecrets, Pacer};
use communities_core::db::{self, Db};
use communities_core::service::{ChannelService, Outcome};
use messenger_core::traits::SystemClock;
use messenger_core::{PubKey, Result};
use nostr::key::{Keys, SecretKey};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, Notify};
use tokio::time::Instant;

/// The keepers' cycle runs at least this often.
const TICK_AT_LEAST: Duration = Duration::from_secs(30);
/// A `resubscribe` waits this long for more of them.
const LISTEN_DEBOUNCE: Duration = Duration::from_millis(700);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct Bot {
    pub name: String,
    pub id: BotId,
    pub keys: Keys,
    pub me: PubKey,
    pub svc: ChannelService,
    pub hosts: Arc<CountingHosts>,
    pub hub: Arc<Hub>,
    pub counts: Counters,
    /// When the join went out and when the first key came.
    pub asked_at: Mutex<Option<Instant>>,
    pub keyed_at: Mutex<Option<Instant>>,
    /// The channel whose key is waited for.
    pub watch: Mutex<Option<String>>,
    listen: Notify,
    wake: Notify,
    stop: AtomicBool,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

/// The key of a bot, kept in its folder.
fn key_of(dir: &Path) -> Result<Keys> {
    let path = dir.join("key");
    if let Ok(hex) = std::fs::read_to_string(&path) {
        let sk = SecretKey::from_hex(hex.trim()).map_err(|e| messenger_core::MessengerError::Crypto(e.to_string()))?;
        return Ok(Keys::new(sk));
    }
    let keys = Keys::generate();
    std::fs::write(&path, keys.secret_key().to_secret_hex())?;
    Ok(keys)
}

impl Bot {
    /// Opens (or makes) the bot of folder `dir` and starts its loop.
    pub async fn open(hub: Arc<Hub>, dir: &Path, pacer: Arc<Pacer>) -> Result<Arc<Self>> {
        std::fs::create_dir_all(dir)?;
        let keys = key_of(dir)?;
        let me = PubKey::parse(&keys.public_key().to_hex()).ok_or_else(|| messenger_core::MessengerError::Invalid("key".into()))?;
        let db = Db::open(&dir.join(db::FILE_NAME)).await?;
        let secrets = Arc::new(FileSecrets::open(dir.join("secrets.json"))?);
        let hosts = Arc::new(CountingHosts::new(pacer)?);
        let (id, rx) = hub.register();
        let net = Arc::new(BotNet { hub: hub.clone(), id });
        let svc = ChannelService::new(db, secrets, net, hosts.clone(), Arc::new(SystemClock));
        svc.set_signer(Some(keys.clone()));
        // The cards of private servers this person has (wire §9.1):
        // `COMMUNITIES_APP_KEYS=https://media.example.org=KEY,…`.
        let app_keys: Vec<(String, String)> = std::env::var("COMMUNITIES_APP_KEYS")
            .unwrap_or_default()
            .split(',')
            .filter_map(|p| p.trim().split_once('=').map(|(h, k)| (h.to_string(), k.to_string())))
            .collect();
        svc.set_app_keys(&app_keys);
        let bot = Arc::new(Self {
            name: dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            id,
            keys,
            me,
            svc,
            hosts,
            hub,
            counts: Counters::default(),
            asked_at: Mutex::new(None),
            keyed_at: Mutex::new(None),
            watch: Mutex::new(None),
            listen: Notify::new(),
            wake: Notify::new(),
            stop: AtomicBool::new(false),
            task: Mutex::new(None),
        });
        let task = tokio::spawn(bot.clone().run(rx));
        *lock(&bot.task) = Some(task);
        bot.listen.notify_one();
        Ok(bot)
    }

    /// Stops the loop and closes the database (a device switched off).
    pub async fn close(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.wake.notify_one();
        let task = lock(&self.task).take();
        if let Some(t) = task {
            let _ = t.await;
        }
        self.close_inbox().await;
        for cid in self.svc.kept().await.unwrap_or_default() {
            if let Ok(Some(k)) = self.svc.keeper_pubkey(&cid).await {
                self.hub.close_inbox(k.as_hex()).await;
            }
        }
        self.hub.unregister(self.id);
        self.svc.db().close().await;
    }

    /// Opens the inbox of this person (the messenger of an app listens to
    /// its person from the start), where the notes to a keeper kept here
    /// come (`inbox` of the journal, wire §3); a keeper whose journal names
    /// its own key (journals before `inbox`) needs a connection of its own.
    pub async fn open_inboxes(&self) -> Result<()> {
        self.hub.open_inbox(self.id, &self.keys).await?;
        for cid in self.svc.kept().await? {
            let named = self.svc.log(&cid).await?.and_then(|l| l.state().keeper().map(|k| k.inbox().clone()));
            if let Some(k) = self.svc.keeper(&cid).await? {
                let keys = k.lock().await.keys().clone();
                if named.as_ref().is_some_and(|n| n.as_hex() == keys.public_key().to_hex()) {
                    self.hub.open_inbox(self.id, &keys).await?;
                }
            }
        }
        self.listen.notify_one();
        Ok(())
    }

    pub async fn close_inbox(&self) {
        self.hub.close_inbox(self.me.as_hex()).await;
    }

    pub fn pubkey(&self) -> String {
        self.me.as_hex().to_string()
    }

    /// Carries out what an action or an arrival led to.
    pub fn apply(&self, out: Outcome) {
        if out.resubscribe {
            self.listen.notify_one();
        }
        if out.wake.is_some() {
            self.wake.notify_one();
        }
        for n in out.notes {
            self.counts.add("note", 1);
            if std::env::var_os("TRIAL_VERBOSE").is_some() {
                eprintln!("{}: {n}", self.name);
            }
        }
    }

    async fn take(&self, item: Item) {
        self.counts.add(if item.live { "live" } else { "history" }, 1);
        let cid = item.json["tags"].as_array().and_then(|t| t.iter().find(|t| t[0] == "h" || t[0] == "d")).and_then(|t| t[1].as_str()).map(str::to_string);
        let r = match cid {
            Some(cid) => {
                let lock = self.svc.lock_of(&cid).await;
                let _g = lock.lock().await;
                self.svc.on_event(&self.keys, &item.json, item.live).await
            }
            None => {
                self.counts.add("wrap", 1);
                self.svc.on_event(&self.keys, &item.json, item.live).await
            }
        };
        match r {
            Ok(out) => self.apply(out),
            Err(e) => {
                self.counts.add("err", 1);
                eprintln!("{}: kind {}: {e}", self.name, item.json["kind"]);
            }
        }
        self.check_key().await;
    }

    /// The first key of the channel watched: when it came.
    async fn check_key(&self) {
        if lock(&self.keyed_at).is_some() {
            return;
        }
        let Some(cid) = lock(&self.watch).clone() else { return };
        if self.admitted(&cid).await {
            *lock(&self.keyed_at) = Some(Instant::now());
        }
    }

    /// The keeper let me in: its issue came (with the key of the current
    /// epoch under an open history; under `from_join` the bundle only, the
    /// key comes with the next epoch).
    pub async fn admitted(&self, cid: &str) -> bool {
        matches!(db::channels::get(self.svc.db(), cid).await, Ok(Some(r)) if r.membership == communities_core::service::MEMBERSHIP_JOINED)
    }

    async fn tick(&self) -> Option<u64> {
        match self.svc.tick().await {
            Ok(next) => next,
            Err(e) => {
                self.counts.add("tick_err", 1);
                eprintln!("{}: tick: {e}", self.name);
                None
            }
        }
    }

    async fn run(self: Arc<Self>, mut rx: mpsc::UnboundedReceiver<Item>) {
        let mut next_tick = Instant::now() + Duration::from_secs(2);
        let mut listen_at: Option<Instant> = None;
        while !self.stop.load(Ordering::SeqCst) {
            let listen_due = listen_at.unwrap_or_else(|| Instant::now() + Duration::from_secs(3600));
            tokio::select! {
                item = rx.recv() => match item {
                    Some(item) => self.take(item).await,
                    None => break,
                },
                _ = self.listen.notified() => {
                    listen_at.get_or_insert_with(|| Instant::now() + LISTEN_DEBOUNCE);
                }
                _ = self.wake.notified() => next_tick = Instant::now(),
                _ = tokio::time::sleep_until(listen_due), if listen_at.is_some() => {
                    listen_at = None;
                    if !self.hub.is_offline(self.id) {
                        self.counts.add("listen", 1);
                        if let Err(e) = self.svc.listen(&self.keys).await {
                            eprintln!("{}: listen: {e}", self.name);
                        }
                    }
                }
                _ = tokio::time::sleep_until(next_tick) => {
                    let wake = if self.hub.is_offline(self.id) { None } else { self.tick().await };
                    let mut next = Instant::now() + TICK_AT_LEAST;
                    if let Some(at) = wake {
                        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                        next = next.min(Instant::now() + Duration::from_secs(at.saturating_sub(now)));
                    }
                    next_tick = next.max(Instant::now() + Duration::from_millis(200));
                }
            }
        }
    }

    /// Off the network: nothing comes, nothing goes, no cycle.
    pub fn set_offline(&self, off: bool) {
        self.hub.set_offline(self.id, off);
        if !off {
            self.listen.notify_one();
            self.wake.notify_one();
        }
    }

    /// Asks the relay for the journal and the card of a channel not here
    /// yet and waits for the journal (`fetch_journal` of the module).
    pub async fn fetch_journal(&self, cid: &str, wait: Duration) -> Result<bool> {
        self.svc.ask_journal(cid).await?;
        let deadline = Instant::now() + wait;
        while Instant::now() < deadline {
            if self.svc.log(cid).await?.is_some() {
                return Ok(true);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        Ok(false)
    }

    /// Opens a link as the command of the module does.
    pub async fn join(&self, link: &str) -> Result<()> {
        let parsed = communities_core::ChannelLink::parse(link)?;
        let cid = parsed.community_id.clone();
        *lock(&self.watch) = Some(cid.clone());
        if self.svc.log(&cid).await?.is_none() && !self.fetch_journal(&cid, Duration::from_secs(60)).await? {
            return Err(messenger_core::MessengerError::Transport("the journal did not come".into()));
        }
        self.hub.open_inbox(self.id, &self.keys).await?;
        *lock(&self.asked_at) = Some(Instant::now());
        let out = {
            let lock = self.svc.lock_of(&cid).await;
            let _g = lock.lock().await;
            self.svc.open_link(&self.keys, link, "").await?
        };
        self.apply(out);
        Ok(())
    }

    /// Seconds from the join to the first key.
    pub fn join_secs(&self) -> Option<f64> {
        let a = (*lock(&self.asked_at))?;
        let k = (*lock(&self.keyed_at))?;
        Some(k.saturating_duration_since(a).as_secs_f64())
    }

    /// The key is here (seen by whoever looked first).
    pub fn mark_keyed(&self) {
        lock(&self.keyed_at).get_or_insert_with(Instant::now);
    }

    pub fn reset_join_clock(&self) {
        *lock(&self.asked_at) = None;
        *lock(&self.keyed_at) = None;
    }
}
