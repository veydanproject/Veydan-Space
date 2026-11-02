// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `communities-cli`: the engine of the channels on the real relay and
//! blob hosts, no UI (internal/communities-stage1.md §7, row 1.6). Every
//! bot is a person on one device with its own database under `--dir`;
//! the keeper lives in the owner's bot. Each command prints JSON lines;
//! the checks compare counts here and exit with 1 when they do not hold.
//!
//! The network comes from the environment, never from here:
//! `COMMUNITIES_TRIAL_RELAY` (or `MESSENGER_DEV_RELAY_URL`),
//! `COMMUNITIES_TRIAL_RELAY_KEY` (or `MESSENGER_DEV_RELAY_API_KEY`),
//! `COMMUNITIES_TRIAL_STORAGE` (blob hosts, comma-separated),
//! `COMMUNITIES_APP_KEYS` (the app keys of private servers this person has
//! the cards of: `https://media.example.org=KEY,…`). The stand's own
//! knobs, never the app's (ROO-474): `COMMUNITIES_TRIAL_JOINS_PER_MINUTE`
//! and `COMMUNITIES_TRIAL_MAX_READERS`, the keeper's limits
//! (`communities_core::stand`); `COMMUNITIES_TRIAL_TRAFFIC_DIR` and
//! `COMMUNITIES_TRIAL_TRAFFIC_LIMIT_MB`, the traffic of a run and where it
//! stops, `COMMUNITIES_TRIAL_UPLOADS`, the blobs put (`traffic`).
//!
//! ```text
//! communities-cli --dir D create [--name N] [--history open|from_join]
//! communities-cli --dir D keeper [--minutes M] [--off-after S --off-minutes M] [--synthetic N]
//!                                       (stdin: pause M | resume | restart | state | stats | synthetic n=N | ban-wave n=N minutes=M
//!                                        | handover | emergency | quit)
//! communities-cli --dir D [--bot admin] join <link> | post [--count N] [--marker M] [--file F] [--scope S] | comment --post-marker M [--text T]
//!                                       | react --target ID [--emoji E] | ban <pubkey>... | rotate-now [--scope S] | status
//!                                       | scope-add [--title T] | level-set [--id I] [--name N] [--scopes S,S] [--post 1]
//!                                       | set-levels [--levels L,L] <pubkey>...
//! communities-cli --dir D swarm --readers N [--link L] [--posts P] [--comments C] [--minutes M] [--rate R] [--inboxes K] [--pools K]
//!                                       [--offline-after S --return-after S [--offline-bots a-b]] [--stdin]
//!                                       (stdin: expect | scope scope=S [posts= rows= key= in= epoch= no-epoch=] | comment | join | inbox
//!                                        | offline bots=a-b | online bots=a-b [timeout=S] | ...)
//! communities-cli --dir D relay-stats [--cid C]
//! communities-cli --dir D blossom-auth --host H --sha S [--bot B] [--verb upload]
//! communities-cli --dir D cleanup        (the blobs of the run deleted; the filters of its events on the relay)
//! ```

mod bot;
mod conn;
mod look;
mod net;
mod store;
mod traffic;

use bot::Bot;
use communities_core::op::{History, JoinMode, LevelCan, OpBody};
use communities_core::roles::Role;
use communities_core::service::NewChannel;
use communities_core::state::SCOPE_EVERYONE;
use communities_core::{bench, stand};
use messenger_core::{MessengerError, PubKey, Result, TextFormat};
use net::{Hub, Target};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use store::Pacer;
use tokio::io::AsyncBufReadExt;
use tokio::time::Instant;

/// GETs a second from this address to the media hosts (their limit: 20).
const GETS_PER_SEC: u32 = 12;
/// Bots waiting for their key at once by default (`--inboxes`), each on a
/// connection of its own (the relay takes 20 from one address: 10 here, 3
/// shared, 2 of the keeper, 2 of a command).
const INBOXES: usize = 10;
/// A line `join_progress` every this many bots keyed.
const PROGRESS_EVERY: usize = 500;
/// How long one bot waits for its key in a pass.
const PER_BOT: Duration = Duration::from_secs(90);

fn out(v: Value) {
    let mut o = std::io::stdout().lock();
    let _ = writeln!(o, "{v}");
    let _ = o.flush();
}

fn bad(what: impl Into<String>) -> MessengerError {
    MessengerError::Invalid(what.into())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// The arguments: options `--name value`, flags `--name`, the rest in order.
struct Args {
    opts: BTreeMap<String, String>,
    flags: Vec<String>,
    rest: Vec<String>,
}

const FLAGS: &[&str] = &["stdin"];

impl Args {
    fn parse(raw: Vec<String>) -> Self {
        let (mut opts, mut flags, mut rest) = (BTreeMap::new(), Vec::new(), Vec::new());
        let mut it = raw.into_iter();
        while let Some(a) = it.next() {
            match a.strip_prefix("--") {
                Some(name) if FLAGS.contains(&name) => flags.push(name.to_string()),
                Some(name) => {
                    opts.insert(name.to_string(), it.next().unwrap_or_default());
                }
                None => rest.push(a),
            }
        }
        Self { opts, flags, rest }
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.opts.get(name).map(String::as_str)
    }

    fn num(&self, name: &str, default: u64) -> u64 {
        self.get(name).and_then(|v| v.parse().ok()).unwrap_or(default)
    }

    fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|f| f == name)
    }
}

fn target() -> Result<Target> {
    let var = |a: &str, b: &str| std::env::var(a).ok().or_else(|| std::env::var(b).ok()).filter(|v| !v.is_empty());
    let relay = var("COMMUNITIES_TRIAL_RELAY", "MESSENGER_DEV_RELAY_URL").ok_or_else(|| bad("COMMUNITIES_TRIAL_RELAY is not set"))?;
    let key = var("COMMUNITIES_TRIAL_RELAY_KEY", "MESSENGER_DEV_RELAY_API_KEY");
    let storage = std::env::var("COMMUNITIES_TRIAL_STORAGE").unwrap_or_default().split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect();
    Ok(Target { relay, key, storage })
}

/// What `create` leaves for the other commands.
fn channel_file(dir: &Path) -> PathBuf {
    dir.join("channel.json")
}

fn channel(dir: &Path) -> Result<Value> {
    let s = std::fs::read_to_string(channel_file(dir)).map_err(|_| bad("no channel.json: run create first"))?;
    Ok(serde_json::from_str(&s)?)
}

fn cid_of(dir: &Path, args: &Args) -> Result<String> {
    match args.get("cid") {
        Some(c) => Ok(c.to_string()),
        None => channel(dir)?["community_id"].as_str().map(str::to_string).ok_or_else(|| bad("community_id")),
    }
}

async fn wait_for<F, Fut>(timeout: Duration, mut f: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + timeout;
    loop {
        if f().await {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn has_key(bot: &Bot, cid: &str) -> bool {
    matches!(bot.svc.member(&bot.keys).newest_key(cid, SCOPE_EVERYONE).await, Ok(Some(_)))
}

async fn newest(bot: &Bot, cid: &str) -> Option<u32> {
    bot.svc.member(&bot.keys).newest_key(cid, SCOPE_EVERYONE).await.ok().flatten().map(|k| k.epoch)
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("runtime: {e}");
            std::process::exit(2);
        }
    };
    let code = rt.block_on(async move {
        match run(Args::parse(raw)).await {
            Ok(ok) => i32::from(!ok),
            Err(e) => {
                out(json!({ "error": e.to_string() }));
                2
            }
        }
    });
    rt.shutdown_timeout(Duration::from_secs(2));
    std::process::exit(code);
}

/// `Ok(false)`: a check did not hold.
async fn run(args: Args) -> Result<bool> {
    let dir = PathBuf::from(args.get("dir").ok_or_else(|| bad("--dir is required"))?);
    std::fs::create_dir_all(&dir)?;
    let cmd = args.rest.first().cloned().unwrap_or_default();
    if cmd == "blossom-auth" {
        // The header of an upload as the app signs it, for a probe of a
        // host by hand (curl): `--host H --sha S`, the key of `--bot`. No
        // connection to the relay (it takes 20 from one address).
        let name = args.get("bot").unwrap_or("admin");
        let hex = std::fs::read_to_string(dir.join(name).join("key")).map_err(|_| bad("no key of that bot"))?;
        let sk = nostr::key::SecretKey::from_hex(hex.trim()).map_err(|e| MessengerError::Crypto(e.to_string()))?;
        let b = messenger_media::backend::BlossomBackend::new(args.get("host").ok_or_else(|| bad("--host"))?, nostr::key::Keys::new(sk))?;
        let h = b.auth_header(args.get("verb").unwrap_or("upload"), args.get("sha").ok_or_else(|| bad("--sha"))?).map_err(|e| bad(e.message))?;
        out(json!({ "authorization": h }));
        return Ok(true);
    }
    if cmd == "cleanup" {
        return cleanup(&dir).await;
    }
    let target = target()?;
    traffic::watch(cmd.clone());
    let pools = args.num("pools", if cmd == "swarm" { 3 } else { 1 }) as usize;
    let hub = Hub::connect(&target, pools).await?;
    let pacer = Pacer::new(GETS_PER_SEC);
    let ok = match cmd.as_str() {
        "create" => create(&args, &dir, &target, hub.clone(), pacer).await?,
        "keeper" => keeper(&args, &dir, hub.clone(), pacer).await?,
        "swarm" => swarm(&args, &dir, hub.clone(), pacer).await?,
        "relay-stats" => relay_stats(&args, &dir, &hub).await?,
        "req" => {
            let f: Value = serde_json::from_str(args.rest.get(1).map_or("{}", String::as_str))?;
            let evs = hub.fetch_plain(&f).await?;
            if args.get("full").is_some() {
                for e in &evs {
                    out(e.clone());
                }
            }
            out(json!({ "event": "req", "events": evs.len(), "ids": evs.iter().take(5).map(|e| e["id"].clone()).collect::<Vec<_>>() }));
            true
        }
        "join" | "post" | "comment" | "react" | "ban" | "rotate-now" | "status" | "scope-add" | "level-set" | "set-levels" => one(&args, &dir, &cmd, hub.clone(), pacer).await?,
        other => return Err(bad(format!("unknown command {other:?}"))),
    };
    hub.shutdown().await;
    traffic::save(&cmd);
    Ok(ok)
}

// ─── create ─────────────────────────────────────────────────────────────────

async fn create(args: &Args, dir: &Path, target: &Target, hub: Arc<Hub>, pacer: Arc<Pacer>) -> Result<bool> {
    if target.storage.is_empty() {
        return Err(bad("COMMUNITIES_TRIAL_STORAGE is not set"));
    }
    let started = Instant::now();
    let owner = Bot::open(hub.clone(), &dir.join("owner"), pacer.clone()).await?;
    let admin = Bot::open(hub.clone(), &dir.join("admin"), pacer).await?;
    owner.open_inboxes().await?;
    let history = match args.get("history").unwrap_or("open") {
        "from_join" => History::FromJoin,
        _ => History::Open,
    };
    let new = NewChannel {
        name: args.get("name").unwrap_or("Trial").to_string(),
        about: "communities-cli".into(),
        picture: String::new(),
        join: JoinMode::Open,
        history,
        relays: vec![target.relay.clone()],
        storage: target.storage.clone(),
    };
    let (cid, o) = owner.svc.create(&owner.keys, new).await?;
    owner.apply(o);
    owner.open_inboxes().await?;
    let keyed = wait_for(Duration::from_secs(180), || has_key(&owner, &cid)).await;
    out(json!({ "event": "first_epoch", "ok": keyed, "secs": started.elapsed().as_secs_f64(), "relay": hub.stats.all(), "hosts": owner.hosts.counts.all(), "bot": owner.counts.all(), "keeper": keeper_state(&owner, &cid).await.as_ref().map(summary) }));
    if !keyed {
        return Ok(false);
    }
    let o = owner.svc.act(&owner.keys, &cid, OpBody::SetRole { who: admin.me.clone(), role: Role::Admin }).await?;
    owner.apply(o);
    let link = owner.svc.share_link(&cid).await?;
    admin.join(&link).await?;
    let joined = wait_for(Duration::from_secs(120), || admin.admitted(&cid)).await;
    let keeper = owner.svc.keeper_pubkey(&cid).await?.map(|k| k.as_hex().to_string()).unwrap_or_default();
    let info = json!({
        "community_id": cid,
        "link": link,
        "owner": owner.pubkey(),
        "admin": admin.pubkey(),
        "keeper": keeper,
        "relay": target.relay,
        "storage": target.storage,
        "history": args.get("history").unwrap_or("open"),
    });
    std::fs::write(channel_file(dir), serde_json::to_string_pretty(&info)?)?;
    out(json!({ "event": "created", "community_id": cid, "link": link, "admin_joined": joined, "secs": started.elapsed().as_secs_f64(),
        "owner_hosts": owner.hosts.counts.all(), "relay": hub.stats.all() }));
    tokio::time::sleep(Duration::from_secs(2)).await;
    owner.close().await;
    admin.close().await;
    Ok(joined)
}

// ─── keeper ─────────────────────────────────────────────────────────────────

async fn keeper_state(owner: &Bot, cid: &str) -> Option<keydist::KeeperState> {
    let k = owner.svc.keeper(cid).await.ok().flatten()?;
    let k = k.lock().await;
    k.scope(SCOPE_EVERYONE).map(|s| s.state().clone())
}

fn summary(s: &keydist::KeeperState) -> Value {
    json!({
        "members": s.members.len(),
        "published": s.published_head.as_ref().map(|k| k.epoch),
        "pending": s.pending.as_ref().map(|p| p.key.epoch),
        "next_epoch": s.next_epoch,
        "gens": s.gens.len(),
        "last_upload": s.last_upload,
        "due_at": s.schedule.due_at,
        "queued": s.schedule.queued,
        "waiting": s.schedule.waiting,
    })
}

/// Every scope the keeper keeps: its members, the epoch published last
/// and the one on its way, when the next is due.
async fn scopes_summary(owner: &Bot, cid: &str) -> Value {
    let Some(k) = owner.svc.keeper(cid).await.ok().flatten() else { return Value::Null };
    let k = k.lock().await;
    let mut m = serde_json::Map::new();
    for id in k.scope_ids() {
        if let Some(s) = k.scope_keeper(&id) {
            m.insert(id.clone(), json!({ "members": s.members_len(), "published": s.published_head().map(|e| e.epoch), "pending": s.pending_epoch(), "due_at": s.due_at() }));
        }
    }
    Value::Object(m)
}

/// The folder of the owner's device that keeps the channel now: `owner`,
/// until a handover or an emergency of the stand moved it (`channel.json`).
fn keeper_dir(dir: &Path) -> String {
    channel(dir).ok().and_then(|c| c["keeper_dir"].as_str().map(str::to_string)).unwrap_or_else(|| "owner".into())
}

/// Changes `channel.json` (what the other commands and the cleanup read).
fn note_channel(dir: &Path, f: impl FnOnce(&mut serde_json::Map<String, Value>)) -> Result<()> {
    let mut c = channel(dir)?;
    if let Some(o) = c.as_object_mut() {
        f(o);
    }
    std::fs::write(channel_file(dir), serde_json::to_string_pretty(&c)?)?;
    Ok(())
}

/// The synthetic readers asked so far, for the ban wave of a later
/// process and the cleanup (written before any of them joins).
fn note_synthetic(dir: &Path, want: u64) -> Result<()> {
    note_channel(dir, |c| {
        let had = c.get("synthetic").and_then(Value::as_u64).unwrap_or(0);
        c.insert("synthetic".into(), json!(had.max(want)));
    })
}

/// The keeper as loaded, with the limits of the stand (a keeper loaded
/// anew takes the app's).
async fn tuned(owner: &Bot, cid: &str, tuning: &stand::Tuning) -> Option<Arc<tokio::sync::Mutex<communities_core::keeper::Keeper>>> {
    let k = owner.svc.keeper(cid).await.ok().flatten()?;
    tuning.apply(&mut *k.lock().await);
    Some(k)
}

/// The readers without a device the keeper lets in (`--synthetic N`,
/// `synthetic n=N`): readers `0..want` ask to join at once (a link posted
/// somewhere), as fast as the keeper takes them; each one's issue is
/// awaited on the relay (`Hub::await_note`).
struct Synth {
    want: u64,
    next: u64,
    /// When the asked ones came.
    came: Instant,
    issued: u64,
    skipped: u64,
    refused: BTreeMap<String, u64>,
    /// From their coming to `on_join`'s issue (the queue of the keeper), s.
    queue_s: Vec<f64>,
    /// From their coming to the issue taken by the relay, s.
    to_key_s: Vec<f64>,
    busy_until: Option<Instant>,
    door: Option<stand::Door>,
    said: bool,
    /// When an issue last reached the relay.
    last_note: Instant,
}

/// At most this many joins in one beat of the keeper process.
const SYNTH_PER_BEAT: usize = 5000;

impl Synth {
    fn new(want: u64) -> Self {
        Self { want, next: 0, came: Instant::now(), issued: 0, skipped: 0, refused: BTreeMap::new(), queue_s: Vec::new(), to_key_s: Vec::new(), busy_until: None, door: None, said: want == 0, last_note: Instant::now() }
    }

    /// `n` more readers come now.
    fn more(&mut self, n: u64) {
        self.want += n;
        self.came = Instant::now();
        self.last_note = self.came;
        self.said = false;
        self.queue_s.clear();
        self.to_key_s.clear();
    }

    fn line(&self, event: &str, hub: &Hub) -> Value {
        let waiting = hub.awaiting();
        json!({ "event": event, "want": self.want, "asked": self.next, "issued": self.issued, "skipped": self.skipped, "refused": self.refused,
            "secs": self.came.elapsed().as_secs_f64(), "queue_s": bench::quantiles(self.queue_s.clone()), "to_key_s": bench::quantiles(self.to_key_s.clone()),
            "issues_waiting": waiting, "at": now_secs() })
    }

    /// One beat: joins until the keeper is busy; what reached the relay.
    async fn step(&mut self, owner: &Bot, cid: &str, tuning: &stand::Tuning) -> Result<Option<Value>> {
        let delivered = owner.hub.take_delivered();
        if !delivered.is_empty() {
            self.last_note = Instant::now();
        }
        self.to_key_s.extend(delivered);
        if self.next >= self.want {
            // All issues on the relay, or none more for 5 minutes (one
            // refused by the relay is told by the count left waiting).
            if !self.said && (owner.hub.awaiting() == 0 || self.last_note.elapsed() > Duration::from_secs(300)) {
                self.said = true;
                return Ok(Some(self.line("synthetic_done", &owner.hub)));
            }
            return Ok(None);
        }
        if self.busy_until.is_some_and(|t| Instant::now() < t) {
            return Ok(None);
        }
        self.busy_until = None;
        let Some(k) = tuned(owner, cid, tuning).await else { return Ok(None) };
        if self.door.is_none() {
            self.door = Some(stand::Door::of(&owner.svc, cid).await?);
        }
        let Some(door) = self.door.as_ref() else { return Ok(None) };
        let before = self.next;
        for _ in 0..SYNTH_PER_BEAT {
            if self.next >= self.want {
                break;
            }
            let i = self.next;
            match stand::join(&k, &owner.svc, door, cid, i, now_secs()).await? {
                stand::Synthetic::Issued => {
                    let pk = stand::synthetic_pk(cid, i)?.as_hex().to_string();
                    owner.hub.await_note(pk, self.came);
                    self.queue_s.push(self.came.elapsed().as_secs_f64());
                    self.issued += 1;
                }
                stand::Synthetic::Skipped => self.skipped += 1,
                stand::Synthetic::Busy => {
                    // The minute of the keeper slides: asked again in a beat.
                    self.busy_until = Some(Instant::now() + Duration::from_secs(4));
                    break;
                }
                stand::Synthetic::Refused(why) => {
                    *self.refused.entry(why.to_string()).or_insert(0) += 1;
                    // Full: the rest would be refused the same.
                    if why == "the channel is full" {
                        self.next = self.want;
                        break;
                    }
                }
            }
            self.next += 1;
        }
        owner.svc.outbox_notify().notify_one();
        let crossed = |every: u64| before / every != self.next / every;
        if crossed(10_000) || (self.next <= 10_000 && crossed(1000)) {
            return Ok(Some(self.line("synthetic", &owner.hub)));
        }
        Ok(None)
    }
}

/// A wave of bans (`ban-wave n=N minutes=M`): the owner bans `n` of the
/// synthetic readers in, evenly over `m` minutes, spread over the tree
/// as `communities-bench` spreads them (the worst cover); from each ban
/// to the first epoch published that leaves it out (built after it, to
/// the 5 s of a beat).
struct Wave {
    total: u64,
    secs: u64,
    pool: u64,
    k: u64,
    started: Instant,
    banned_at: Vec<Instant>,
    ban_ms: Vec<f64>,
    errors: BTreeMap<String, u64>,
    built_over: BTreeMap<u32, usize>,
    credited: usize,
    to_epoch_s: Vec<f64>,
    head: u32,
    head0: u32,
    pub0: u64,
}

impl Wave {
    async fn new(owner: &Bot, cid: &str, total: u64, minutes: u64, pool: u64, hub: &Hub) -> Self {
        let head = keeper_state(owner, cid).await.and_then(|s| s.published_head.map(|h| h.epoch)).unwrap_or(0);
        Self { total, secs: minutes.max(1) * 60, pool: pool.max(1), k: 0, started: Instant::now(), banned_at: Vec::new(), ban_ms: Vec::new(), errors: BTreeMap::new(),
            built_over: BTreeMap::new(), credited: 0, to_epoch_s: Vec::new(), head, head0: head, pub0: hub.stats.get("pub_bytes") }
    }

    /// One beat; the line of the wave once it is over.
    async fn step(&mut self, owner: &Bot, cid: &str) -> Result<Option<Value>> {
        // What the keeper built and published since the last beat.
        if let Some(s) = keeper_state(owner, cid).await {
            if let Some(p) = s.pending.as_ref().map(|p| p.key.epoch) {
                self.built_over.entry(p).or_insert(self.banned_at.len());
            }
            let now_head = s.published_head.as_ref().map_or(0, |h| h.epoch);
            if now_head != self.head {
                let over = self.built_over.get(&now_head).copied().unwrap_or(self.banned_at.len()).max(self.credited);
                self.to_epoch_s.extend(self.banned_at[self.credited..over].iter().map(|t| t.elapsed().as_secs_f64()));
                self.credited = over;
                self.head = now_head;
            }
        }
        let elapsed = self.started.elapsed().as_secs();
        let due = (self.total * elapsed.min(self.secs) / self.secs).min(self.total);
        if (self.banned_at.len() as u64) < due {
            let log = owner.svc.log(cid).await?.ok_or_else(|| bad("no journal"))?;
            let k = owner.svc.keeper(cid).await?.ok_or_else(|| bad("not kept here"))?;
            let mut tries = 0;
            while (self.banned_at.len() as u64) < due && tries < self.pool {
                tries += 1;
                let i = (self.k.wrapping_mul(999_983).wrapping_add(12_345)) % self.pool;
                self.k += 1;
                let who = stand::synthetic_pk(cid, i)?;
                if log.state().is_banned(&who) || !k.lock().await.is_member(&who) {
                    continue;
                }
                let t = Instant::now();
                let r = {
                    let lock = owner.svc.lock_of(cid).await;
                    let _g = lock.lock().await;
                    owner.svc.act(&owner.keys, cid, OpBody::Ban { who, reason: None }).await
                };
                match r {
                    Ok(o) => {
                        owner.apply(o);
                        self.banned_at.push(t);
                        self.ban_ms.push(t.elapsed().as_secs_f64() * 1000.0);
                    }
                    Err(e) => *self.errors.entry(e.to_string()).or_insert(0) += 1,
                }
            }
            if tries >= self.pool && (self.banned_at.len() as u64) < due {
                // Fewer readers in than bans asked: the wave is what it got.
                self.total = self.banned_at.len() as u64;
            }
        }
        let made = self.banned_at.len();
        let over = made as u64 >= self.total && self.credited >= made;
        let late = elapsed > self.secs + 15 * 60;
        if !(over || late) {
            return Ok(None);
        }
        Ok(Some(json!({ "event": "ban_wave", "ok": self.credited >= made && self.errors.is_empty(), "bans": made, "asked": self.total, "minutes": self.secs / 60,
            "secs": self.started.elapsed().as_secs_f64(), "epochs": self.head - self.head0.min(self.head), "bans_not_out": made - self.credited.min(made),
            "ban_ms": bench::quantiles(self.ban_ms.clone()), "ban_to_epoch_s": bench::quantiles(self.to_epoch_s.clone()), "errors": self.errors,
            "pub_mb": ((owner.hub.stats.get("pub_bytes") - self.pub0) as f64 / 1_048_576.0 * 10.0).round() / 10.0, "at": now_secs() })))
    }
}

/// The keeper to the owner's other device (wire §15), or, `emergency`,
/// to a new device of the owner after this one is lost (spec §7.6): in a
/// folder of its own with the owner's key, in this process; the process
/// keeps the channel with it from here on (`channel.json`).
async fn take_keeper(dir: &Path, cid: &str, hub: &Arc<Hub>, pacer: &Arc<Pacer>, owner: &mut Arc<Bot>, emergency: bool, tuning: &stand::Tuning) -> Result<Value> {
    let event = if emergency { "emergency" } else { "handover" };
    let from = keeper_dir(dir);
    let mut n = 2;
    while dir.join(format!("owner-{n}")).exists() {
        n += 1;
    }
    let name = format!("owner-{n}");
    std::fs::create_dir_all(dir.join(&name))?;
    std::fs::copy(dir.join(&from).join("key"), dir.join(&name).join("key"))?;
    let before = keeper_state(owner, cid).await;
    let epoch_before = before.as_ref().and_then(|s| s.published_head.as_ref().map(|h| h.epoch)).unwrap_or(0);
    let members = before.as_ref().map_or(0, |s| s.members.len());
    let (pub0, wraps0) = (hub.stats.get("pub_bytes"), hub.stats.get(&format!("pub_{}", communities_core::notes::KIND_WRAP)));
    let started = Instant::now();
    let b = Bot::open(hub.clone(), &dir.join(&name), pacer.clone()).await?;
    b.open_inboxes().await?;
    let fail = |what: &str, b: &Bot| json!({ "event": event, "ok": false, "folder": name, "failed": what, "secs": started.elapsed().as_secs_f64(), "bot": b.counts.all() });
    if !b.fetch_journal(cid, Duration::from_secs(60)).await? {
        b.close().await;
        return Ok(fail("the journal did not come", &b));
    }
    // The device has been the owner's for a while: in the channel, with
    // the key of the newest epoch and its snapshot (an emergency rebuilds
    // the readers from it).
    let ready = wait_for(Duration::from_secs(600), || async {
        b.admitted(cid).await && newest(&b, cid).await.is_some_and(|e| e >= epoch_before) && b.svc.member(&b.keys).latest_snapshot(cid).await.ok().flatten().is_some()
    })
    .await;
    let ready_s = started.elapsed().as_secs_f64();
    if !ready {
        b.close().await;
        return Ok(fail("the device did not catch up", &b));
    }
    if emergency {
        // Lost: from here on it hears and sends nothing. Its inbox was the
        // new device's connection too.
        owner.close().await;
        b.open_inboxes().await?;
    }
    let r = b.svc.keeper_take_at_head(&b.keys, cid, emergency, false, Duration::from_secs(60)).await;
    let took = match r {
        Ok(o) => {
            b.apply(o);
            None
        }
        Err(e) => Some(e.to_string()),
    };
    let take_s = started.elapsed().as_secs_f64();
    if let Some(e) = took {
        let mut v = fail("take", &b);
        v["error"] = json!(e);
        b.close().await;
        if emergency {
            // Found again: the stand goes on with the keeper it had.
            *owner = Bot::open(hub.clone(), &dir.join(&from), pacer.clone()).await?;
            owner.open_inboxes().await?;
        }
        return Ok(v);
    }
    // A hands its state over on its turn (it heard the journal name the
    // next series); in an emergency nothing comes from it.
    let moved = wait_for(Duration::from_secs(600), || async { b.svc.keeper(cid).await.ok().flatten().is_some() && (emergency || owner.svc.keeper(cid).await.ok().flatten().is_none()) }).await;
    let moved_s = started.elapsed().as_secs_f64();
    let series = b.svc.log(cid).await?.and_then(|l| l.state().keeper().cloned());
    let from_epoch = series.as_ref().map_or(0, |s| s.from_epoch);
    let _ = tuned(&b, cid, tuning).await;
    let first = moved && wait_for(Duration::from_secs(900), || async { keeper_state(&b, cid).await.and_then(|s| s.published_head.map(|h| h.epoch)).is_some_and(|e| e >= from_epoch) }).await;
    let first_s = started.elapsed().as_secs_f64();
    if !emergency {
        owner.close().await;
        b.open_inboxes().await?;
    }
    let after = keeper_state(&b, cid).await;
    let keeper = series.as_ref().map(|s| s.pubkey.as_hex().to_string()).unwrap_or_default();
    note_channel(dir, |c| {
        c.insert("keeper_dir".into(), json!(name));
        let mut all: Vec<Value> = c.get("keepers").and_then(Value::as_array).cloned().unwrap_or_default();
        all.push(json!(keeper));
        c.insert("keepers".into(), Value::Array(all));
    })?;
    let v = json!({ "event": event, "ok": moved && first, "folder": name, "from": from, "serial": series.as_ref().map(|s| s.serial), "from_epoch": from_epoch,
        "epoch_before": epoch_before, "members_before": members, "state": after.as_ref().map(summary),
        "ready_s": ready_s, "take_s": take_s, "moved_s": moved_s, "first_epoch_s": first_s, "notes": hub.stats.get(&format!("pub_{}", communities_core::notes::KIND_WRAP)) - wraps0,
        "pub_mb": ((hub.stats.get("pub_bytes") - pub0) as f64 / 1_048_576.0 * 10.0).round() / 10.0, "at": now_secs() });
    *owner = b;
    Ok(v)
}

async fn keeper(args: &Args, dir: &Path, hub: Arc<Hub>, pacer: Arc<Pacer>) -> Result<bool> {
    let cid = cid_of(dir, args)?;
    let tuning = stand::Tuning::from_env();
    let saved_path = dir.join("keeper-state.json");
    let mut owner = Bot::open(hub.clone(), &dir.join(keeper_dir(dir)), pacer.clone()).await?;
    owner.open_inboxes().await?;
    let mut ok = true;
    let loaded = keeper_state(&owner, &cid).await.ok_or_else(|| bad("this folder keeps no keeper of the channel"))?;
    let limits = tuned(&owner, &cid, &tuning).await.map(|k| async move { stand::Tuning::now_of(&*k.lock().await) });
    let limits = match limits {
        Some(f) => Some(f.await),
        None => None,
    };
    let limits = limits.map(|(j, r)| json!({ "joins_per_minute": j, "max_readers": r }));
    if let Ok(s) = std::fs::read_to_string(&saved_path) {
        let saved: keydist::KeeperState = serde_json::from_str(&s)?;
        let equal = saved == loaded;
        ok &= equal;
        out(json!({ "event": "loaded", "equal_to_saved": equal, "state": summary(&loaded), "limits": limits }));
    } else {
        out(json!({ "event": "loaded", "state": summary(&loaded), "limits": limits }));
    }
    let minutes = args.num("minutes", 600);
    let end = Instant::now() + Duration::from_secs(minutes * 60);
    let mut off_at = args.get("off-after").and_then(|v| v.parse::<u64>().ok()).map(|s| Instant::now() + Duration::from_secs(s));
    let mut off_minutes = args.num("off-minutes", 0);
    let mut back_at: Option<Instant> = None;
    let mut synth = Synth::new(args.num("synthetic", 0));
    note_synthetic(dir, synth.want)?;
    let mut wave: Option<Wave> = None;
    let (tx, mut lines) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let mut stdin = tokio::io::BufReader::new(tokio::io::stdin()).lines();
        while let Ok(Some(l)) = stdin.next_line().await {
            if tx.send(l).is_err() {
                break;
            }
        }
    });
    let mut last = Value::Null;
    let mut beat = tokio::time::interval(Duration::from_secs(5));
    loop {
        tokio::select! {
            _ = beat.tick() => {}
            line = lines.recv() => {
                let line = line.unwrap_or_else(|| "eof".into());
                let a = kv(&line);
                let mut w = line.split_whitespace();
                match w.next().unwrap_or("") {
                    "pause" => {
                        off_minutes = w.next().and_then(|v| v.parse().ok()).unwrap_or(10);
                        off_at = Some(Instant::now());
                    }
                    "resume" => back_at = Some(Instant::now()),
                    "state" => {
                        if let Some(s) = keeper_state(&owner, &cid).await {
                            let snap = communities_core::db::keeper::snapshot(owner.svc.db(), &cid).await?.map(|r| r.at);
                            out(json!({ "event": "state", "state": summary(&s), "scopes": scopes_summary(&owner, &cid).await, "snapshot_at": snap, "now": now_secs() }));
                        }
                    }
                    "stats" => out(json!({ "event": "stats", "relay": hub.stats.all(), "hosts": owner.hosts.counts.all(), "bot": owner.counts.all(),
                        "synthetic": synth.line("synthetic", &hub), "traffic": traffic::mine(), "run_traffic": traffic::run_total() })),
                    "synthetic" => {
                        synth.more(a.get("n").and_then(|v| v.parse().ok()).unwrap_or(0));
                        note_synthetic(dir, synth.want)?;
                        out(synth.line("synthetic", &hub));
                    }
                    "ban-wave" => {
                        let n = a.get("n").and_then(|v| v.parse().ok()).unwrap_or(0);
                        let m = a.get("minutes").and_then(|v| v.parse().ok()).unwrap_or(60);
                        // The synthetic readers of this process and of those before it.
                        let pool = synth.want.max(channel(dir)?["synthetic"].as_u64().unwrap_or(0));
                        wave = Some(Wave::new(&owner, &cid, n, m, pool, &hub).await);
                        out(json!({ "event": "ban_wave_started", "bans": n, "minutes": m, "pool": pool, "at": now_secs() }));
                    }
                    "handover" | "emergency" => {
                        let emergency = line.trim() == "emergency";
                        let v = take_keeper(dir, &cid, &hub, &pacer, &mut owner, emergency, &tuning).await?;
                        ok &= v["ok"].as_bool().unwrap_or(false);
                        // A new device has no door of its own until it is asked.
                        synth.door = None;
                        out(v);
                    }
                    "restart" => {
                        // The device goes off and comes back: the service
                        // anew from its rows, nothing kept in memory.
                        let before = keeper_state(&owner, &cid).await;
                        owner.close().await;
                        owner = Bot::open(hub.clone(), &dir.join(keeper_dir(dir)), pacer.clone()).await?;
                        owner.open_inboxes().await?;
                        let after = keeper_state(&owner, &cid).await;
                        let equal = before.is_some() && before == after;
                        ok &= equal;
                        out(json!({ "event": "restart", "equal": equal, "state": after.as_ref().map(summary) }));
                    }
                    "quit" | "eof" => break,
                    "" => {}
                    other => out(json!({ "error": format!("unknown {other}") })),
                }
            }
        }
        if let Some(at) = off_at {
            if Instant::now() >= at {
                owner.set_offline(true);
                off_at = None;
                back_at = Some(Instant::now() + Duration::from_secs(off_minutes * 60));
                out(json!({ "event": "paused", "minutes": off_minutes, "at": now_secs() }));
            }
        }
        if let Some(at) = back_at {
            if Instant::now() >= at && hub.is_offline(owner.id) {
                owner.set_offline(false);
                back_at = None;
                out(json!({ "event": "resumed", "at": now_secs() }));
            }
        }
        if !hub.is_offline(owner.id) {
            let _ = tuned(&owner, &cid, &tuning).await;
            if let Some(v) = synth.step(&owner, &cid, &tuning).await? {
                out(v);
            }
            if let Some(w) = wave.as_mut() {
                if let Some(v) = w.step(&owner, &cid).await? {
                    ok &= v["ok"].as_bool().unwrap_or(false);
                    out(v);
                    wave = None;
                }
            }
        }
        if let Some(s) = keeper_state(&owner, &cid).await {
            let sum = json!({ "state": summary(&s), "scopes": scopes_summary(&owner, &cid).await });
            if sum != last {
                out(json!({ "event": "keeper", "at": now_secs(), "state": sum["state"], "scopes": sum["scopes"] }));
                last = sum;
            }
        }
        if Instant::now() > end {
            break;
        }
    }
    if let Some(s) = keeper_state(&owner, &cid).await {
        std::fs::write(&saved_path, serde_json::to_string(&s)?)?;
    }
    out(json!({ "event": "stopped", "relay": hub.stats.all(), "hosts": owner.hosts.counts.all(), "bot": owner.counts.all(), "synthetic": synth.line("synthetic", &hub), "traffic": traffic::mine() }));
    owner.close().await;
    Ok(ok)
}

// ─── One action of one bot ──────────────────────────────────────────────────

async fn one(args: &Args, dir: &Path, cmd: &str, hub: Arc<Hub>, pacer: Arc<Pacer>) -> Result<bool> {
    let name = args.get("bot").unwrap_or("admin");
    let bot = Bot::open(hub.clone(), &dir.join(name), pacer).await?;
    if cmd != "join" {
        bot.open_inboxes().await?;
    }
    let started = Instant::now();
    let mut ok = true;
    if cmd == "join" {
        let link = args.rest.get(1).cloned().ok_or_else(|| bad("join <link>"))?;
        let cid = communities_core::ChannelLink::parse(&link)?.community_id;
        bot.join(&link).await?;
        ok = wait_for(Duration::from_secs(args.num("timeout", 120)), || bot.admitted(&cid)).await;
        out(json!({ "event": "join", "bot": name, "ok": ok, "secs": started.elapsed().as_secs_f64(), "epoch": newest(&bot, &cid).await }));
        bot.close().await;
        return Ok(ok);
    }
    let cid = cid_of(dir, args)?;
    // Caught up: the journal and a key (a reader who left has none).
    // Caught up: the journal, and the key of the newest epoch heard of (a
    // device seals with the newest key it holds; one a step behind would
    // hide a post from those who came with the new epoch, `from_join`).
    // The newest epoch on the relay too: a process just started may not
    // have heard the rotation a moment ago (run h of the trial).
    let on_relay = relay_newest_epoch(&hub, &cid).await;
    let caught = wait_for(Duration::from_secs(60), || async {
        let newest_heard = look::known_epochs(&bot, &cid).await.ok().and_then(|e| e.last().copied());
        let held = newest(&bot, &cid).await.map(i64::from);
        bot.svc.log(&cid).await.ok().flatten().is_some() && held == newest_heard && newest_heard.is_some() && held >= on_relay
    })
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let lock = bot.svc.lock_of(&cid).await;
    match cmd {
        "post" => {
            let marker = args.get("marker").unwrap_or("trial-post");
            if let Some(file) = args.get("file") {
                let bytes = std::fs::read(file)?;
                let sha = communities_core::blobs::sha256_hex(&bytes);
                let (row, o) = bot.svc.media_placeholder(&bot.keys, &cid, None, None, Path::new(file), Some(marker)).await?;
                bot.apply(o);
                let t = Instant::now();
                let r = bot.svc.media_send(&bot.keys, &row.id, None, &communities_core::media::Sink(|_| {})).await;
                let (id, ok_send) = match r {
                    Ok((id, o)) => {
                        bot.apply(o);
                        (id, true)
                    }
                    Err(e) => (format!("error: {e}"), false),
                };
                ok &= ok_send;
                out(json!({ "event": "media", "ok": ok_send, "id": id, "sha256": sha, "bytes": bytes.len(), "secs": t.elapsed().as_secs_f64(), "hosts": bot.hosts.counts.all() }));
            } else {
                let n = args.num("count", 1);
                let mut sent = 0;
                let mut errors = Vec::new();
                for i in 0..n {
                    let _g = lock.lock().await;
                    match bot.svc.send_post_in(&bot.keys, &cid, args.get("scope"), &format!("{marker} {i}"), TextFormat::Plain).await {
                        Ok((_, o)) => {
                            bot.apply(o);
                            sent += 1;
                        }
                        Err(e) => errors.push(e.to_string()),
                    }
                }
                ok &= sent == n;
                out(json!({ "event": "posts", "sent": sent, "want": n, "errors": errors, "secs": started.elapsed().as_secs_f64() }));
            }
        }
        "comment" => {
            let marker = args.get("post-marker").unwrap_or("trial-post");
            let post = match args.get("post") {
                Some(p) => p.to_string(),
                None => look::post_ids(&bot, &cid, marker).await?.last().cloned().ok_or_else(|| bad("no post"))?,
            };
            let _g = lock.lock().await;
            let r = bot.svc.send_comment(&bot.keys, &cid, &post, None, args.get("text").unwrap_or("trial-comment by hand"), TextFormat::Plain).await;
            ok = r.is_ok();
            out(json!({ "event": "comment", "ok": ok, "result": r.map(|(id, _)| id).unwrap_or_else(|e| e.to_string()) }));
        }
        "react" => {
            let target = args.get("target").ok_or_else(|| bad("--target"))?;
            let _g = lock.lock().await;
            let r = bot.svc.send_reaction(&bot.keys, &cid, target, args.get("emoji").unwrap_or("👍"), true).await;
            ok = r.is_ok();
            out(json!({ "event": "react", "ok": ok, "result": r.map(|(id, _)| id).unwrap_or_else(|e| e.to_string()) }));
        }
        "ban" => {
            let mut done = Vec::new();
            for who in args.rest.iter().skip(1) {
                let pk = PubKey::parse(who).ok_or_else(|| bad("pubkey"))?;
                let t = Instant::now();
                let _g = lock.lock().await;
                let r = bot.svc.act(&bot.keys, &cid, OpBody::Ban { who: pk, reason: None }).await;
                ok &= r.is_ok();
                done.push(json!({ "who": &who[..12], "ok": r.is_ok(), "ms": t.elapsed().as_millis() as u64, "error": r.err().map(|e| e.to_string()) }));
            }
            out(json!({ "event": "ban", "ok": ok, "done": done, "at": now_secs() }));
        }
        "scope-add" => {
            let t = Instant::now();
            let _g = lock.lock().await;
            let r = bot.svc.scope_add(&bot.keys, &cid, args.get("title").unwrap_or("Trial section"), keydist::SchemeId::SdV1, History::Open).await;
            ok = r.is_ok();
            let (scope_id, error) = match r {
                Ok((id, o)) => {
                    bot.apply(o);
                    (Some(id), None)
                }
                Err(e) => (None, Some(e.to_string())),
            };
            out(json!({ "event": "scope_add", "ok": ok, "scope_id": scope_id, "error": error, "ms": t.elapsed().as_millis() as u64, "at": now_secs() }));
        }
        "level-set" => {
            let t = Instant::now();
            let scopes: Vec<String> = args.get("scopes").unwrap_or("").split(',').filter(|s| !s.is_empty()).map(str::to_string).collect();
            let can = LevelCan { comment: true, discuss: true, post: args.num("post", 0) == 1 };
            let id = args.get("id").and_then(|v| v.parse::<u8>().ok());
            let _g = lock.lock().await;
            let r = bot.svc.level_set(&bot.keys, &cid, id, args.get("name").unwrap_or("Trial level"), scopes, can).await;
            ok = r.is_ok();
            let (level, error) = match r {
                Ok((id, o)) => {
                    bot.apply(o);
                    (Some(id), None)
                }
                Err(e) => (None, Some(e.to_string())),
            };
            out(json!({ "event": "level_set", "ok": ok, "level": level, "error": error, "ms": t.elapsed().as_millis() as u64, "at": now_secs() }));
        }
        "set-levels" => {
            // `--levels 1,2` (none: the base level alone) for every pubkey
            // given, one operation each.
            let levels: Vec<u8> = args.get("levels").unwrap_or("").split(',').filter_map(|v| v.parse().ok()).collect();
            let t = Instant::now();
            let (mut done, mut ms, mut errors) = (0u64, Vec::new(), BTreeMap::<String, u64>::new());
            for who in args.rest.iter().skip(1) {
                let pk = PubKey::parse(who).ok_or_else(|| bad("pubkey"))?;
                let t1 = Instant::now();
                let _g = lock.lock().await;
                match bot.svc.set_levels(&bot.keys, &cid, &pk, &levels).await {
                    Ok(o) => {
                        bot.apply(o);
                        done += 1;
                    }
                    Err(e) => *errors.entry(e.to_string()).or_insert(0) += 1,
                }
                ms.push(t1.elapsed().as_secs_f64() * 1000.0);
            }
            ok = errors.is_empty();
            out(json!({ "event": "set_levels", "ok": ok, "done": done, "errors": errors, "levels": levels, "secs": t.elapsed().as_secs_f64(),
                "ms_p50": percentile(&mut ms, 0.5), "ms_max": percentile(&mut ms, 1.0), "at": now_secs() }));
        }
        "rotate-now" => {
            let _g = lock.lock().await;
            let r = bot.svc.act(&bot.keys, &cid, OpBody::RotateNow { scope_id: args.get("scope").map(str::to_string) }).await;
            ok = r.is_ok();
            out(json!({ "event": "rotate_now", "ok": ok, "at": now_secs(), "error": r.err().map(|e| e.to_string()) }));
        }
        _ => {
            let l = look::look(&bot, &cid, args.get("post-marker").unwrap_or("trial-post"), args.get("comment-marker").unwrap_or("trial-comment")).await?;
            let ops = communities_core::db::channels::ops(bot.svc.db(), &cid).await?.len();
            let pending = communities_core::db::channels::count_pending(bot.svc.db(), &cid).await?;
            let snap = bot.svc.member(&bot.keys).latest_snapshot(&cid).await?.map(|s| json!({ "at": s.at, "members": s.members, "shards": s.shards.len() }));
            let staff: Vec<Value> = communities_core::db::channels::staff(bot.svc.db(), &cid).await?.iter().map(|s| json!(format!("{s:?}"))).collect();
            out(json!({ "event": "status", "caught_up": caught, "look": l, "journal_ops": ops, "known_epochs": look::known_epochs(&bot, &cid).await?, "pending": pending, "snapshot": snap, "staff": staff, "shards": look::shards(&bot, &cid).await?, "hosts": bot.hosts.counts.all() }));
        }
    }
    // What was sent has gone; what it led to has come back.
    tokio::time::sleep(Duration::from_secs(3)).await;
    bot.close().await;
    Ok(ok)
}

/// The newest epoch of the channel on the relay.
async fn relay_newest_epoch(hub: &Hub, cid: &str) -> Option<i64> {
    hub.fetch_plain(&json!({ "kinds": [communities_core::wire::KIND_EPOCH], "#h": [cid], "limit": 1 }))
        .await
        .ok()
        .and_then(|evs| evs.iter().filter_map(|e| e["tags"].as_array()?.iter().find(|t| t[0] == "n")?[1].as_str()?.parse::<i64>().ok()).max())
}

// ─── cleanup ────────────────────────────────────────────────────────────────

/// Files named `name` under `dir`, at any depth.
fn files_named(dir: &Path, name: &str, into: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            files_named(&p, name, into);
        } else if p.file_name().is_some_and(|n| n == name) {
            into.push(p);
        }
    }
}

/// What a run of the stand left on the test servers (ROO-474): every
/// blob it put, deleted on its host by the bot that put it
/// (`COMMUNITIES_TRIAL_UPLOADS`, else `uploads.jsonl` of `--dir`); and
/// the events on the relay, as filters for `strfry delete` on the server
/// (`{"filter": …}` lines; a relay deletes for its operator, not for a
/// client): the channels by `#h`/`#d`, everything the bots and keepers
/// signed, every note to a bot, a keeper or a synthetic reader.
async fn cleanup(dir: &Path) -> Result<bool> {
    let mut keys: BTreeMap<String, nostr::key::Keys> = BTreeMap::new();
    let mut found = Vec::new();
    files_named(dir, "key", &mut found);
    for p in found {
        let Ok(hex) = std::fs::read_to_string(&p) else { continue };
        let Ok(sk) = nostr::key::SecretKey::from_hex(hex.trim()) else { continue };
        let k = nostr::key::Keys::new(sk);
        keys.insert(k.public_key().to_hex(), k);
    }
    let uploads = std::env::var_os("COMMUNITIES_TRIAL_UPLOADS").filter(|p| !p.is_empty()).map(PathBuf::from).unwrap_or_else(|| dir.join("uploads.jsonl"));
    let mut puts = std::collections::BTreeSet::new();
    for l in std::fs::read_to_string(&uploads).unwrap_or_default().lines() {
        let Ok(v) = serde_json::from_str::<Value>(l) else { continue };
        let f = |k: &str| v[k].as_str().unwrap_or("").to_string();
        puts.insert((f("host"), f("sha256"), f("uploader")));
    }
    let hosts = store::CountingHosts::new(Pacer::new(GETS_PER_SEC))?;
    let app_keys: Vec<(String, String)> = std::env::var("COMMUNITIES_APP_KEYS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|p| p.trim().split_once('=').map(|(h, k)| (h.to_string(), k.to_string())))
        .collect();
    let (mut deleted, mut unknown, mut errors) = (0u64, 0u64, BTreeMap::<String, u64>::new());
    for (host, sha, by) in &puts {
        let Some(k) = keys.get(by) else {
            unknown += 1;
            continue;
        };
        // A host of the kit lets a blob go with the app key of its server.
        let key = app_keys.iter().find(|(h, _)| h.trim_end_matches('/') == host.trim_end_matches('/')).map(|(_, k)| k.as_str());
        match communities_core::blobs::BlobHosts::delete_keyed(&hosts, k, key, host, sha).await {
            Ok(()) => deleted += 1,
            Err(e) => *errors.entry(e.to_string().chars().take(120).collect()).or_insert(0) += 1,
        }
    }
    let mut channels = Vec::new();
    files_named(dir, "channel.json", &mut channels);
    let (mut cids, mut signers, mut synthetic) = (Vec::new(), keys.keys().cloned().collect::<Vec<_>>(), 0u64);
    let mut to: Vec<String> = signers.clone();
    for p in channels {
        let Ok(c) = std::fs::read_to_string(&p).map_err(|_| ()).and_then(|s| serde_json::from_str::<Value>(&s).map_err(|_| ())) else { continue };
        let Some(cid) = c["community_id"].as_str() else { continue };
        cids.push(cid.to_string());
        let keepers = std::iter::once(&c["keeper"]).chain(c["keepers"].as_array().into_iter().flatten()).filter_map(Value::as_str).filter(|k| !k.is_empty());
        for k in keepers {
            signers.push(k.to_string());
            to.push(k.to_string());
        }
        let n = c["synthetic"].as_u64().unwrap_or(0);
        synthetic += n;
        for i in 0..n {
            to.push(stand::synthetic_pk(cid, i)?.as_hex().to_string());
        }
    }
    signers.sort();
    signers.dedup();
    to.sort();
    to.dedup();
    let mut filters = vec![json!({ "#h": cids }), json!({ "#d": cids })];
    filters.extend(signers.chunks(500).map(|c| json!({ "authors": c })));
    filters.extend(to.chunks(500).map(|c| json!({ "kinds": [communities_core::notes::KIND_WRAP], "#p": c })));
    if cids.is_empty() {
        filters.clear();
    }
    for f in &filters {
        out(json!({ "filter": f }));
    }
    let ok = errors.is_empty() && unknown == 0;
    out(json!({ "event": "cleanup", "ok": ok, "blobs_put": puts.len(), "blobs_deleted": deleted, "unknown_uploader": unknown, "errors": errors,
        "channels": cids.len(), "signers": signers.len(), "recipients": to.len(), "synthetic": synthetic, "filters": filters.len() }));
    Ok(ok)
}

// ─── relay-stats ────────────────────────────────────────────────────────────

/// What the relay holds of a channel, by kind: every page of 500.
async fn relay_stats(args: &Args, dir: &Path, hub: &Hub) -> Result<bool> {
    use communities_core::wire::{KIND_CONTENT, KIND_EPOCH, KIND_JOURNAL, KIND_SNAPSHOT};
    let cid = cid_of(dir, args)?;
    let mut by = serde_json::Map::new();
    for (name, kind) in [("journal", KIND_JOURNAL), ("epochs", KIND_EPOCH), ("snapshots", KIND_SNAPSHOT), ("content", KIND_CONTENT)] {
        let (mut n, mut bytes, mut until) = (0u64, 0u64, now_secs() + 3600);
        let mut seen = std::collections::HashSet::new();
        loop {
            let page = hub.fetch_plain(&json!({ "kinds": [kind], "#h": [cid], "until": until, "limit": 500 })).await?;
            let mut fresh = 0;
            for ev in &page {
                if seen.insert(ev["id"].as_str().unwrap_or("").to_string()) {
                    fresh += 1;
                    n += 1;
                    bytes += ev.to_string().len() as u64;
                }
                until = until.min(ev["created_at"].as_i64().unwrap_or(until));
            }
            if page.len() < 500 || fresh == 0 {
                break;
            }
        }
        by.insert(name.into(), json!({ "events": n, "bytes": bytes }));
    }
    out(json!({ "event": "relay_stats", "community_id": cid, "kinds": by }));
    Ok(true)
}

// ─── swarm ──────────────────────────────────────────────────────────────────

/// Bots `a-b` of the swarm (`all`, `a-b`, `a`), as indices.
fn pick(spec: Option<&str>, n: usize) -> Vec<usize> {
    let Some(spec) = spec.filter(|s| *s != "all") else { return (0..n).collect() };
    let mut out = Vec::new();
    for part in spec.split(',') {
        let (a, b) = part.split_once('-').unwrap_or((part, part));
        let (Ok(a), Ok(b)) = (a.parse::<usize>(), b.parse::<usize>()) else { continue };
        out.extend((a..=b.min(n.saturating_sub(1))).filter(|i| *i < n));
    }
    out
}

fn kv(line: &str) -> BTreeMap<String, String> {
    line.split_whitespace().skip(1).filter_map(|w| w.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

struct Swarm {
    dir: PathBuf,
    prefix: String,
    hub: Arc<Hub>,
    pacer: Arc<Pacer>,
    cid: String,
    link: String,
    bots: Vec<Arc<Bot>>,
    /// Joins in flight at most (each holds an inbox connection).
    inboxes: usize,
    /// Bots keyed by the joins of this process, for the progress lines.
    keyed: std::sync::atomic::AtomicUsize,
}

/// A join in flight: its bot, the task that starts it, when that ended.
struct Flight {
    i: usize,
    task: tokio::task::JoinHandle<()>,
    started: Option<Instant>,
}

fn percentile(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let i = ((v.len() - 1) as f64 * p).round() as usize;
    v[i]
}

impl Swarm {
    async fn open_bots(&mut self, from: usize, to: usize) -> Result<()> {
        for i in from..to {
            let bot = Bot::open(self.hub.clone(), &self.dir.join(format!("{}{i:04}", self.prefix)), self.pacer.clone()).await?;
            *bot.watch.lock().unwrap_or_else(|e| e.into_inner()) = Some(self.cid.clone());
            self.bots.push(bot);
        }
        Ok(())
    }

    /// The bots `idx` not in the channel yet open the link, `rate` a
    /// second, at most `inboxes` of them waiting at once (each waits on an
    /// inbox of its own, a connection); a bot whose key has not come in
    /// `PER_BOT` gives its connection back and is tried once more at the
    /// end (its inbox brings two days of notes back).
    async fn join(&self, idx: &[usize], rate: u64, timeout: Duration) -> Result<Value> {
        let started = Instant::now();
        let deadline = started + timeout;
        let mut todo = Vec::new();
        for &i in idx {
            let bot = &self.bots[i];
            // In, or put out (a ban): not asked again.
            if bot.admitted(&self.cid).await || look::membership(bot, &self.cid).await? == communities_core::service::MEMBERSHIP_LEFT {
                continue;
            }
            bot.reset_join_clock();
            todo.push(i);
        }
        let asked = todo.clone();
        let mut retried = 0;
        for pass in 0..2 {
            todo = self.join_pass(todo, rate, pass == 1, deadline).await?;
            if pass == 0 {
                retried = todo.len();
            }
            if todo.is_empty() {
                break;
            }
        }
        let mut secs: Vec<f64> = asked.iter().filter_map(|&i| self.bots[i].join_secs()).collect();
        let keyed = secs.len();
        Ok(json!({
            "phase": "join", "ok": todo.is_empty(), "asked": asked.len(), "admitted": keyed, "retried": retried, "secs": started.elapsed().as_secs_f64(),
            "p50": percentile(&mut secs, 0.5), "p95": percentile(&mut secs, 0.95), "p99": percentile(&mut secs, 0.99), "max": percentile(&mut secs, 1.0),
        }))
    }

    /// One pass over `todo`; the bots still without a key. A join is
    /// started as its own task, `rate` a second, while fewer than
    /// `inboxes` are in flight: the journal, the inbox connection (paced
    /// by the hub) and the join of one bot overlap with the next ones. A
    /// bot's `PER_BOT` counts from the end of its start, so that a start
    /// still connecting is never cut (its inbox would outlive the pass).
    async fn join_pass(&self, todo: Vec<usize>, rate: u64, again: bool, deadline: Instant) -> Result<Vec<usize>> {
        let gap = Duration::from_secs_f64(1.0 / rate.max(1) as f64);
        let total = todo.len();
        let mut queue: std::collections::VecDeque<usize> = todo.into();
        let mut flight: Vec<Flight> = Vec::new();
        let mut left = Vec::new();
        let mut last_launch: Option<Instant> = None;
        let started = Instant::now();
        while !queue.is_empty() || !flight.is_empty() {
            let mut still = Vec::new();
            for mut f in flight.drain(..) {
                if f.started.is_none() && f.task.is_finished() {
                    f.started = Some(Instant::now());
                }
                let bot = &self.bots[f.i];
                let Some(t0) = f.started else {
                    still.push(f);
                    continue;
                };
                if bot.admitted(&self.cid).await {
                    bot.mark_keyed();
                    bot.close_inbox().await;
                    let keyed = self.keyed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    if keyed.is_multiple_of(PROGRESS_EVERY) {
                        out(json!({ "phase": "join_progress", "keyed": keyed, "pass_total": total, "in_flight": still.len(), "inboxes_open": self.hub.inboxes_open().await, "secs": started.elapsed().as_secs_f64(), "relay": self.hub.stats.all() }));
                    }
                } else if t0.elapsed() > PER_BOT || Instant::now() > deadline {
                    bot.close_inbox().await;
                    left.push(f.i);
                } else {
                    still.push(f);
                }
            }
            flight = still;
            if Instant::now() > deadline {
                left.extend(queue.drain(..));
                if flight.is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
            let paced = last_launch.is_none_or(|t| t.elapsed() >= gap);
            if flight.len() < self.inboxes && paced {
                if let Some(i) = queue.pop_front() {
                    last_launch = Some(Instant::now());
                    let bot = self.bots[i].clone();
                    let (cid, link) = (self.cid.clone(), self.link.clone());
                    let task = tokio::spawn(async move {
                        // Again: the join went out (its answer may wait in the inbox),
                        // or it never did (no connection, no journal): sent now.
                        let sent = matches!(look::membership(&bot, &cid).await.as_deref(), Ok("joining" | "rejected"));
                        let r = if again && sent { bot.open_inboxes().await } else { bot.join(&link).await };
                        if let Err(e) = r {
                            eprintln!("{}: join: {e}", bot.name);
                        }
                    });
                    flight.push(Flight { i, task, started: None });
                    continue;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(left)
    }

    /// Waits until every bot of `idx` agrees with `want`; the bots that
    /// do not, at the end.
    async fn expect(&self, idx: &[usize], want: &BTreeMap<String, String>) -> Result<Value> {
        let started = Instant::now();
        let timeout = Duration::from_secs(want.get("timeout").and_then(|v| v.parse().ok()).unwrap_or(120));
        let pm = want.get("post-marker").map_or("trial-post", String::as_str);
        let cm = want.get("comment-marker").map_or("trial-comment", String::as_str);
        // Comments of these bots of the swarm do not count (`except=a-b`).
        let except: Vec<String> = pick(want.get("except").map(String::as_str), self.bots.len()).into_iter().filter(|_| want.contains_key("except")).map(|i| self.bots[i].name.clone()).collect();
        let mut want = want.clone();
        // The counts of another bot of the swarm, as it holds them now.
        if let Some(other) = want.get("same-as").and_then(|v| v.parse::<usize>().ok()).and_then(|i| self.bots.get(i)) {
            let l = look::look(other, &self.cid, pm, cm).await?;
            want.insert("posts".into(), l.posts.to_string());
            want.insert("comments".into(), look::comments_except(other, &self.cid, cm, &except).await?.to_string());
        }
        let num = |k: &str| want.get(k).and_then(|v| v.parse::<i64>().ok());
        let check = |l: &look::Look| -> bool {
            num("posts").is_none_or(|n| l.posts == n)
                && num("comments").is_none_or(|n| l.comments == n)
                && num("epoch").is_none_or(|n| l.held.last() == Some(&n))
                && num("min-epoch").is_none_or(|n| l.held.first().is_none_or(|f| *f >= n))
                && num("no-epoch").is_none_or(|n| !l.held.contains(&n))
                && num("key").is_none_or(|k| (k != 0) == !l.held.is_empty())
                && want.get("membership").is_none_or(|m| &l.membership == m)
        };
        // The readers the keeper let in (or a ban put out); the others are
        // counted apart, unless `all=1`.
        let everyone = want.get("all").is_some_and(|v| v == "1");
        let mut bad_ones = Vec::new();
        let mut not_in: usize;
        // When each bot first agreed.
        let mut good_at: BTreeMap<usize, f64> = BTreeMap::new();
        // `open=1`: a bot that does not agree yet opens its inbox
        // (`inboxes` at once, each for `PER_BOT`), where a bundle of the
        // keeper comes (an emergency generation): a reader of the stand
        // holds no inbox after its join, a device of the app does.
        let opening = want.get("open").is_some_and(|v| v == "1");
        let mut opened: BTreeMap<usize, Instant> = BTreeMap::new();
        loop {
            bad_ones.clear();
            not_in = 0;
            for &i in idx {
                let mut l = look::look(&self.bots[i], &self.cid, pm, cm).await?;
                if !everyone && l.membership != communities_core::service::MEMBERSHIP_JOINED && l.membership != communities_core::service::MEMBERSHIP_LEFT {
                    not_in += 1;
                    continue;
                }
                if !except.is_empty() {
                    l.comments = look::comments_except(&self.bots[i], &self.cid, cm, &except).await?;
                }
                if !check(&l) {
                    bad_ones.push(l);
                    if opening {
                        match opened.get(&i) {
                            Some(t) if t.elapsed() > PER_BOT => {
                                self.bots[i].close_inbox().await;
                                opened.remove(&i);
                            }
                            None if opened.len() < self.inboxes => {
                                if let Err(e) = self.bots[i].open_inboxes().await {
                                    eprintln!("{}: inbox: {e}", self.bots[i].name);
                                }
                                opened.insert(i, Instant::now());
                            }
                            _ => {}
                        }
                    }
                } else {
                    good_at.entry(i).or_insert_with(|| started.elapsed().as_secs_f64());
                    if opened.remove(&i).is_some() {
                        self.bots[i].close_inbox().await;
                    }
                }
            }
            if bad_ones.is_empty() || started.elapsed() > timeout {
                break;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        for i in opened.into_keys() {
            self.bots[i].close_inbox().await;
        }
        let sample: Vec<Value> = bad_ones.iter().take(5).map(|l| json!(l)).collect();
        Ok(json!({ "check": &want, "ok": bad_ones.is_empty(), "bots": idx.len(), "not_in": not_in, "bad": bad_ones.len(), "sample": sample, "secs": started.elapsed().as_secs_f64(),
            "to_good_s": bench::quantiles(good_at.into_values().collect()) }))
    }

    /// Waits until every bot of `idx` agrees on a scope (`scope=`): `posts`
    /// opened (text of `post-marker`), `rows` held at all, `key=1|0` a key
    /// of it or none, `in=1|0` the journal gives it or not (the UI shows a
    /// section not given as closed), `epoch` its newest key, `no-epoch` a
    /// key not held. When each bot first agreed: p50, p95, max. `open=1`:
    /// a bot that does not agree yet opens its inbox (`inboxes` at once,
    /// each for `PER_BOT`), where the keeper's issue of a section comes;
    /// a reader of the stand holds no inbox after its join.
    async fn expect_scope(&self, idx: &[usize], want: &BTreeMap<String, String>) -> Result<Value> {
        let started = Instant::now();
        let timeout = Duration::from_secs(want.get("timeout").and_then(|v| v.parse().ok()).unwrap_or(120));
        let scope = want.get("scope").ok_or_else(|| bad("scope="))?.clone();
        let pm = want.get("post-marker").map_or("trial-post", String::as_str);
        let num = |k: &str| want.get(k).and_then(|v| v.parse::<i64>().ok());
        let opening = want.get("open").is_some_and(|v| v == "1");
        let mut opened: BTreeMap<usize, Instant> = BTreeMap::new();
        let mut done: BTreeMap<usize, f64> = BTreeMap::new();
        let mut bad_ones: Vec<Value>;
        loop {
            bad_ones = Vec::new();
            for &i in idx {
                if done.contains_key(&i) {
                    continue;
                }
                let b = &self.bots[i];
                let held = look::held_in(b, &self.cid, &scope).await?;
                let (rows, open) = look::posts_in(b, &self.cid, &scope, pm).await?;
                let given = b.svc.log(&self.cid).await?.is_some_and(|l| l.state().in_scope(&b.me, &scope));
                let good = num("posts").is_none_or(|n| open == n)
                    && num("rows").is_none_or(|n| rows == n)
                    && num("key").is_none_or(|k| (k != 0) == !held.is_empty())
                    && num("in").is_none_or(|k| (k != 0) == given)
                    && num("epoch").is_none_or(|e| held.last() == Some(&e))
                    && num("no-epoch").is_none_or(|e| !held.contains(&e));
                if good {
                    done.insert(i, started.elapsed().as_secs_f64());
                    if opened.remove(&i).is_some() {
                        b.close_inbox().await;
                    }
                } else {
                    bad_ones.push(json!({ "bot": b.name, "held": held, "rows": rows, "open": open, "in": given }));
                    if opening {
                        match opened.get(&i) {
                            Some(t) if t.elapsed() > PER_BOT => {
                                b.close_inbox().await;
                                opened.remove(&i);
                            }
                            None if opened.len() < self.inboxes => {
                                if let Err(e) = b.open_inboxes().await {
                                    eprintln!("{}: inbox: {e}", b.name);
                                }
                                opened.insert(i, Instant::now());
                            }
                            _ => {}
                        }
                    }
                }
            }
            if bad_ones.is_empty() || started.elapsed() > timeout {
                break;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        for i in opened.into_keys() {
            self.bots[i].close_inbox().await;
        }
        let mut secs: Vec<f64> = done.values().copied().collect();
        let sample: Vec<Value> = bad_ones.iter().take(5).cloned().collect();
        Ok(json!({ "check": want, "ok": bad_ones.is_empty(), "bots": idx.len(), "good": done.len(), "bad": bad_ones.len(), "sample": sample,
            "p50": percentile(&mut secs, 0.5), "p95": percentile(&mut secs, 0.95), "max": percentile(&mut secs, 1.0), "secs": started.elapsed().as_secs_f64() }))
    }

    /// `n` comments by the bots `idx` in turn, over the posts of
    /// `post-marker`, `rate` a second.
    async fn comment(&self, idx: &[usize], n: usize, marker: &str, post_marker: &str, rate: u64) -> Result<Value> {
        let started = Instant::now();
        let first = idx.first().copied().ok_or_else(|| bad("no bots"))?;
        let posts = look::post_ids(&self.bots[first], &self.cid, post_marker).await?;
        if posts.is_empty() {
            return Err(bad("no posts to comment"));
        }
        let gap = Duration::from_secs_f64(1.0 / rate.max(1) as f64);
        let (mut sent, mut refused, mut errors) = (0, 0, BTreeMap::<String, u64>::new());
        for j in 0..n {
            let t = Instant::now();
            let bot = &self.bots[idx[j % idx.len()]];
            let post = &posts[j % posts.len()];
            let r = {
                let lock = bot.svc.lock_of(&self.cid).await;
                let _g = lock.lock().await;
                bot.svc.send_comment(&bot.keys, &self.cid, post, None, &format!("{marker} {} {j}", bot.name), TextFormat::Plain).await
            };
            match r {
                Ok((_, o)) => {
                    bot.apply(o);
                    sent += 1;
                }
                Err(e) => {
                    refused += 1;
                    *errors.entry(e.to_string()).or_insert(0) += 1;
                }
            }
            tokio::time::sleep(gap.saturating_sub(t.elapsed())).await;
        }
        Ok(json!({ "done": "comment", "sent": sent, "refused": refused, "errors": errors, "secs": started.elapsed().as_secs_f64() }))
    }

    /// Comments sealed by hand under each bot's newest key, past the
    /// checks of its own service (a client that does not obey the
    /// journal): the others must refuse them.
    async fn forge(&self, idx: &[usize], marker: &str, post_marker: &str) -> Result<Value> {
        use communities_core::wire::{self, Links, Place};
        let mut sent = 0;
        let mut no_key = 0;
        for &i in idx {
            let bot = &self.bots[i];
            let Some(post) = look::post_ids(bot, &self.cid, post_marker).await?.last().cloned() else { continue };
            let Some(key) = bot.svc.member(&bot.keys).newest_key(&self.cid, SCOPE_EVERYONE).await? else {
                no_key += 1;
                continue;
            };
            let place = Place::feed(&self.cid);
            let links = Links { thread: Some(post), ..Links::default() };
            let at = now_secs();
            let inner = wire::sign_inner(&bot.keys, &place, &links, &messenger_core::Envelope::text(&format!("{marker} {}", bot.name)).encode(), at)?;
            let payload = wire::Payload::event(wire::T_MSG, &inner)?;
            let outer = wire::seal_content(&place, &key, links.outer_target(), &payload, at)?;
            let ev = messenger_core::outbound::WireEvent { id: messenger_core::EventId::parse(&outer.id.to_hex()).ok_or_else(|| bad("id"))?, json: serde_json::to_value(&outer)? };
            let net = net::BotNet { hub: self.hub.clone(), id: bot.id };
            let ack = messenger_core::Transport::send(&net, messenger_core::Outbound::PublishScoped { scope: messenger_core::Scope::Channel { id: self.cid.clone() }, event: ev }).await?;
            if ack.is_delivered() {
                sent += 1;
            }
        }
        Ok(json!({ "done": "forge", "sent": sent, "no_key": no_key }))
    }

    async fn fetch(&self, idx: &[usize], post_marker: &str, sha: &str) -> Result<Value> {
        let started = Instant::now();
        let tmp = self.dir.join("fetched");
        std::fs::create_dir_all(&tmp)?;
        let gets_before: u64 = idx.iter().map(|&i| self.bots[i].hosts.gets()).sum();
        let mut tasks = Vec::new();
        for &i in idx {
            let bot = self.bots[i].clone();
            let (cid, pm, sha, dir) = (self.cid.clone(), post_marker.to_string(), sha.to_string(), tmp.join(&bot.name));
            tasks.push(tokio::spawn(async move {
                let t = Instant::now();
                let Some(id) = look::post_ids(&bot, &cid, &pm).await.ok().and_then(|v| v.last().cloned()) else { return (false, 0.0, "no post".to_string()) };
                let _ = std::fs::create_dir_all(&dir);
                let r = bot.svc.media_fetch(&cid, &id, &dir, &communities_core::media::Sink(|_| {})).await;
                match r {
                    Ok(path) => {
                        let got = std::fs::read(&path).map(|b| communities_core::blobs::sha256_hex(&b)).unwrap_or_default();
                        let _ = std::fs::remove_dir_all(&dir);
                        (got == sha, t.elapsed().as_secs_f64(), if got == sha { String::new() } else { "hash".into() })
                    }
                    Err(e) => (false, t.elapsed().as_secs_f64(), e.to_string()),
                }
            }));
        }
        let (mut ok, mut secs, mut errors) = (0, Vec::new(), BTreeMap::<String, u64>::new());
        for t in tasks {
            let (good, s, e) = t.await.map_err(|e| bad(e.to_string()))?;
            if good {
                ok += 1;
                secs.push(s);
            } else {
                *errors.entry(e).or_insert(0) += 1;
            }
        }
        let gets: u64 = idx.iter().map(|&i| self.bots[i].hosts.gets()).sum::<u64>() - gets_before;
        Ok(json!({ "done": "fetch", "ok": ok == idx.len(), "equal": ok, "bots": idx.len(), "errors": errors, "gets": gets,
            "p50": percentile(&mut secs, 0.5), "max": percentile(&mut secs, 1.0), "secs": started.elapsed().as_secs_f64() }))
    }

    /// Scrolls the feed and every thread back to the start, as the page
    /// does: the relay gives 500 events a REQ.
    async fn load(&self, idx: &[usize]) -> Result<Value> {
        let started = Instant::now();
        for &i in idx {
            let bot = &self.bots[i];
            // A page asks the relay for what is older than the oldest event
            // held (comments included): on until nothing more comes.
            let (mut before, mut last, mut still, mut pages) = (None, -1, 0, 0);
            while still < 2 && pages < 200 {
                pages += 1;
                let page = bot.svc.history(&bot.keys, &self.cid, 50, before).await?;
                if page.asked {
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                }
                let n = look::rows(bot, &self.cid).await?;
                if page.items.len() == 50 {
                    before = page.next_before;
                    still = 0;
                } else if n == last {
                    still += 1;
                } else {
                    still = 0;
                }
                last = n;
            }
            for post in look::post_ids(bot, &self.cid, "").await? {
                let mut before = None;
                for _ in 0..20 {
                    let page = bot.svc.thread(&bot.keys, &self.cid, &post, 100, before).await?;
                    if page.asked {
                        tokio::time::sleep(Duration::from_millis(800)).await;
                    }
                    let page = if page.asked { bot.svc.thread(&bot.keys, &self.cid, &post, 100, before).await? } else { page };
                    if page.items.len() < 100 || page.next_before == before {
                        break;
                    }
                    before = page.next_before;
                }
            }
        }
        Ok(json!({ "done": "load", "bots": idx.len(), "secs": started.elapsed().as_secs_f64() }))
    }

    async fn stats(&self) -> Value {
        let mut hosts = BTreeMap::new();
        let mut bots = BTreeMap::new();
        let mut gets: Vec<f64> = Vec::new();
        for b in &self.bots {
            store::sum_into(&mut hosts, &b.hosts.counts.all());
            store::sum_into(&mut bots, &b.counts.all());
            gets.push(b.hosts.gets() as f64);
        }
        let avg = if gets.is_empty() { 0.0 } else { gets.iter().sum::<f64>() / gets.len() as f64 };
        // From each bot's join to its first key, of the joins of this process.
        let to_key: Vec<f64> = self.bots.iter().filter_map(|b| b.join_secs()).collect();
        json!({ "stats": "swarm", "bots": self.bots.len(), "inboxes_open": self.hub.inboxes_open().await, "relay": self.hub.stats.all(), "hosts": hosts, "events": bots,
            "gets_per_bot": { "min": percentile(&mut gets, 0.0), "avg": avg, "max": percentile(&mut gets, 1.0) }, "time_to_key_s": bench::quantiles(to_key),
            "offline": self.bots.iter().filter(|b| self.hub.is_offline(b.id)).count(), "traffic": traffic::mine() })
    }

    /// The bots `idx` off the network (a phone in a drawer): nothing comes
    /// to them, nothing goes, no cycle.
    fn offline(&self, idx: &[usize]) -> Value {
        for &i in idx {
            self.bots[i].set_offline(true);
        }
        json!({ "done": "offline", "bots": idx.len(), "at": now_secs() })
    }

    /// The bots `idx` back on the network: each catches up on its own, and
    /// the time until it holds the newest epoch on the relay now, with the
    /// GETs and the events it took meanwhile.
    async fn online(&self, idx: &[usize], timeout: Duration) -> Result<Value> {
        let target = relay_newest_epoch(&self.hub, &self.cid).await.unwrap_or(0);
        let sum = |k: &str| -> u64 { idx.iter().map(|&i| self.bots[i].counts.get(k)).sum() };
        let gets0: u64 = idx.iter().map(|&i| self.bots[i].hosts.gets()).sum();
        let (hist0, live0) = (sum("history"), sum("live"));
        let started = Instant::now();
        for &i in idx {
            self.bots[i].set_offline(false);
        }
        let mut done: BTreeMap<usize, f64> = BTreeMap::new();
        loop {
            for &i in idx {
                if done.contains_key(&i) {
                    continue;
                }
                let held = look::held(&self.bots[i], &self.cid).await?;
                // Put out meanwhile (a ban): caught up when the journal says so.
                let out = look::membership(&self.bots[i], &self.cid).await? == communities_core::service::MEMBERSHIP_LEFT;
                if out || held.last().is_some_and(|e| *e >= target) {
                    done.insert(i, started.elapsed().as_secs_f64());
                }
            }
            if done.len() == idx.len() || started.elapsed() > timeout {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let gets: u64 = idx.iter().map(|&i| self.bots[i].hosts.gets()).sum::<u64>() - gets0;
        Ok(json!({ "done": "online", "ok": done.len() == idx.len(), "bots": idx.len(), "caught_up": done.len(), "target_epoch": target,
            "to_epoch_s": bench::quantiles(done.into_values().collect()), "gets": gets, "history_events": sum("history") - hist0, "live_events": sum("live") - live0,
            "secs": started.elapsed().as_secs_f64() }))
    }

    async fn status(&self) -> Result<Value> {
        let mut membership = BTreeMap::<String, u64>::new();
        let mut newest_epoch = BTreeMap::<String, u64>::new();
        for b in &self.bots {
            *membership.entry(look::membership(b, &self.cid).await?).or_insert(0) += 1;
            let e = look::held(b, &self.cid).await?.last().map_or("none".to_string(), i64::to_string);
            *newest_epoch.entry(e).or_insert(0) += 1;
        }
        Ok(json!({ "status": "swarm", "bots": self.bots.len(), "membership": membership, "newest_epoch": newest_epoch }))
    }

    async fn command(&mut self, line: &str) -> Result<(Value, bool)> {
        let a = kv(line);
        let n = self.bots.len();
        let idx = pick(a.get("bots").map(String::as_str), n);
        let pm = a.get("post-marker").map_or("trial-post", String::as_str).to_string();
        let marker = a.get("marker").map_or("trial-comment", String::as_str).to_string();
        let num = |k: &str, d: u64| a.get(k).and_then(|v| v.parse().ok()).unwrap_or(d);
        let v = match line.split_whitespace().next().unwrap_or("") {
            "expect" => {
                let v = self.expect(&idx, &a).await?;
                let ok = v["ok"].as_bool().unwrap_or(false);
                return Ok((v, ok));
            }
            "scope" => {
                let v = self.expect_scope(&idx, &a).await?;
                let ok = v["ok"].as_bool().unwrap_or(false);
                return Ok((v, ok));
            }
            "comment" => self.comment(&idx, num("n", idx.len() as u64) as usize, &marker, &pm, num("rate", 10)).await?,
            "forge" => self.forge(&idx, &marker, &pm).await?,
            "join" => {
                let k = num("n", 1) as usize;
                self.open_bots(n, n + k).await?;
                let fresh: Vec<usize> = (n..n + k).collect();
                let mut v = self.join(&fresh, num("rate", 5), Duration::from_secs(num("timeout", 300))).await?;
                v["first"] = json!(n);
                let ok = v["ok"].as_bool().unwrap_or(false);
                return Ok((v, ok));
            }
            "inbox" => {
                // The bots of `idx` without a key open their inbox again
                // (an app opened): what the keeper sent meanwhile comes.
                let started = Instant::now();
                let mut todo = Vec::new();
                for &i in &idx {
                    if !self.bots[i].admitted(&self.cid).await {
                        todo.push(i);
                    }
                }
                let asked = todo.len();
                let deadline = Instant::now() + Duration::from_secs(num("timeout", 180));
                let left = self.join_pass(todo, num("rate", 5), true, deadline).await?;
                let v = json!({ "done": "inbox", "ok": left.is_empty(), "asked": asked, "admitted": asked - left.len(), "secs": started.elapsed().as_secs_f64() });
                return Ok((v, left.is_empty()));
            }
            "fetch" => {
                let v = self.fetch(&idx, &pm, a.get("sha").map_or("", String::as_str)).await?;
                let ok = v["ok"].as_bool().unwrap_or(false);
                return Ok((v, ok));
            }
            "load" => self.load(&idx).await?,
            "shards" => {
                let mut out = Vec::new();
                for &i in &idx {
                    let (c, t) = look::shards(&self.bots[i], &self.cid).await?;
                    out.push(json!({ "bot": self.bots[i].name, "cached": c, "shards": t, "gets": self.bots[i].hosts.gets() }));
                }
                json!({ "done": "shards", "bots": out })
            }
            "offline" => self.offline(&idx),
            "online" => {
                let v = self.online(&idx, Duration::from_secs(num("timeout", 900))).await?;
                let ok = v["ok"].as_bool().unwrap_or(false);
                return Ok((v, ok));
            }
            "pubkeys" => json!({ "pubkeys": idx.iter().map(|&i| self.bots[i].pubkey()).collect::<Vec<_>>() }),
            "status" => self.status().await?,
            "stats" => self.stats().await,
            "" => return Ok((Value::Null, true)),
            other => json!({ "error": format!("unknown {other}") }),
        };
        Ok((v, true))
    }
}

async fn swarm(args: &Args, dir: &Path, hub: Arc<Hub>, pacer: Arc<Pacer>) -> Result<bool> {
    let link = match args.get("link") {
        Some(l) => l.to_string(),
        None => channel(dir)?["link"].as_str().map(str::to_string).ok_or_else(|| bad("link"))?,
    };
    let cid = communities_core::ChannelLink::parse(&link)?.community_id;
    let n = args.num("readers", 10) as usize;
    let minutes = args.num("minutes", 30);
    let mut s = Swarm { dir: dir.join("readers"), prefix: args.get("prefix").unwrap_or("r").to_string(), hub, pacer, cid, link, bots: Vec::new(), inboxes: args.num("inboxes", INBOXES as u64).max(1) as usize, keyed: Default::default() };
    let started = Instant::now();
    s.open_bots(0, n).await?;
    out(json!({ "phase": "open", "bots": n, "secs": started.elapsed().as_secs_f64() }));
    let all: Vec<usize> = (0..n).collect();
    let v = s.join(&all, args.num("rate", 5), Duration::from_secs(minutes * 60)).await?;
    let mut ok = v["ok"].as_bool().unwrap_or(false);
    out(v);
    let posts = args.num("posts", 0);
    if posts > 0 {
        let want: BTreeMap<String, String> = [("posts".to_string(), posts.to_string()), ("timeout".to_string(), (minutes * 60).to_string())].into();
        let v = s.expect(&all, &want).await?;
        ok &= v["ok"].as_bool().unwrap_or(false);
        out(json!({ "phase": "posts", "result": v }));
    }
    let comments = args.num("comments", 0) as usize;
    if comments > 0 {
        let v = s.comment(&all, comments, "trial-comment", "trial-post", args.num("rate", 5)).await?;
        out(json!({ "phase": "comments_sent", "result": v }));
        let want: BTreeMap<String, String> = [("comments".to_string(), comments.to_string()), ("timeout".to_string(), (minutes * 60).to_string())].into();
        let v = s.expect(&all, &want).await?;
        ok &= v["ok"].as_bool().unwrap_or(false);
        out(json!({ "phase": "comments", "result": v }));
    }
    if let Some(after) = args.get("offline-after").and_then(|v| v.parse::<u64>().ok()) {
        // Away and back by the clock: `--offline-bots` (all) go off the
        // network `after` s past the joins, come back `--return-after` s
        // later and catch up.
        let idx = pick(args.get("offline-bots"), n);
        tokio::time::sleep(Duration::from_secs(after)).await;
        out(json!({ "phase": "offline", "result": s.offline(&idx) }));
        tokio::time::sleep(Duration::from_secs(args.num("return-after", 3600))).await;
        let v = s.online(&idx, Duration::from_secs(minutes * 60)).await?;
        ok &= v["ok"].as_bool().unwrap_or(false);
        out(json!({ "phase": "offline_return", "result": v }));
    }
    out(s.stats().await);
    if args.flag("stdin") {
        let mut stdin = tokio::io::BufReader::new(tokio::io::stdin()).lines();
        out(json!({ "ready": true }));
        while let Ok(Some(line)) = stdin.next_line().await {
            if line.trim() == "quit" {
                break;
            }
            // Every answer names its command (`line`): a driver waits for it.
            match s.command(&line).await {
                Ok((mut v, good)) => {
                    ok &= good;
                    if let Some(o) = v.as_object_mut() {
                        o.insert("line".into(), json!(line));
                    }
                    if !v.is_null() {
                        out(v);
                    }
                }
                Err(e) => out(json!({ "error": e.to_string(), "line": line })),
            }
        }
    }
    for b in &s.bots {
        b.close().await;
    }
    Ok(ok)
}
