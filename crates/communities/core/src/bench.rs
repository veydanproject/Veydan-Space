// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `communities-bench`: the keeper of one channel and one member at N
//! readers, through the engine (not keydist alone), over the fakes of
//! `testkit` (a relay and blob hosts that count events, PUTs, GETs and
//! bytes, a clock moved by hand) and a real SQLite file. Stage 4 of
//! internal/communities-stage1.md §9, the offline half: the network
//! cannot take a million bots from one machine, the keeper can be
//! measured at a million without them.
//!
//! ```text
//! communities-bench --n N --dir D [--no-from-join] [--catch-up E] [--days 1,7,30]
//!                   [--tick-every J] [--no-paths] [--churn]
//!                   [--phases P,Q | --from P] [--resume R]
//! ```
//!
//! Every number is measured once and printed as a JSON line
//! (`{"phase": …, …}`); the lines are also written to `D/results.jsonl`.
//! Each line carries the memory (`rss_mb`, `hwm_mb`) and the CPU of the
//! process (`cpu_user_s`, `cpu_sys_s` since the start, `cpu_s` since the
//! line before; `/proc/self/stat` at 100 ticks a second), the generation
//! of `everyone` at the keeper (`gen`: an emergency one at the wall moves
//! it) and the notes the keeper sent so far (`keeper_wraps`).
//!
//! The phases (ROO-397 §2): the joins (time to the key by the fake clock,
//! p50/p95/p99), the snapshot, the first epoch, a restart, the catch-up of
//! readers away 1, 7 and 30 days (`--days`) under `open` and `from_join`,
//! a day of bans, a wave of 2000 bans in an hour, the hosts down for 90
//! minutes with a ban every 5, a level taken away (the key of its
//! section), a planned generation, a recovery, the keeper handed to the
//! owner's other device (wire §15) and the channel to another owner
//! (§16). `--no-paths`: hosts that serve no paths (a plain Blossom host),
//! whose generation at the wall is replaced by an emergency one; with
//! `--churn` every reader put out is replaced by a new one, so that the
//! leaves of a generation run out as in a channel whose readers come and
//! go.
//!
//! The joins of the `n` readers are the ground of every phase and always
//! run; `--phases` runs only the phases named after them, `--from` the
//! phases from one on (`PHASES`). After each phase the state of the bench
//! is saved in `D/state`, in the place of the one before (`state`), and
//! `--resume R` goes on from the state saved in `R/state` (in `R`, or in
//! `--dir` when given) with the phases after the one it was saved after:
//! a night cut by its time goes on the next one where it stopped. Its
//! lines say `"resumed": true`; the time of each save and its size are a
//! line `{"state": "saved", …}` of the output, not of `results.jsonl`.
//! The readers past the first are made here and let in by
//! `Keeper::on_join` with a proof of the link, as a join note would after
//! the queue; one real reader (a `Node`) joins by the link through the
//! service and reads the epochs. The keeper's own limits (65 536 readers,
//! 120 joins a minute) are as in the app, except `max_readers`, lifted to
//! go past one snapshot (reported as such).

mod state;

use crate::db::{self, Db};
use crate::keeper::{Joined, Keeper, OWED_FIRST_WAIT_SECS};
use crate::link::LinkSecret;
use crate::log::OpLog;
use crate::op::{History, JoinMode, LevelCan, OpBody};
use crate::roles::Role;
use crate::service::NewChannel;
use crate::snapshot::{self as snap, MAX_SHARDS, SHARD_ENTRIES};
use crate::state::SCOPE_EVERYONE;
use crate::testkit::{Node, World, H1, H2};
use keydist::SchemeId;
use crate::wire::{Join, JoinProof, Rebundle, KIND_EPOCH, KIND_SNAPSHOT};
use keydist::{KeyDistribution, Thresholds};
use messenger_core::{MessengerError, PubKey, Result};
use nostr::key::{Keys, SecretKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

/// What the bench runs.
#[derive(Clone, Debug)]
pub struct Args {
    pub n: u64,
    pub dir: PathBuf,
    /// Run the catch-up under `from_join` too.
    pub from_join: bool,
    /// Epochs of one day offline (288 at five minutes).
    pub catch_up: u32,
    /// The days a reader is away in the catch-up, each one reader.
    pub days: Vec<u32>,
    /// Joins between two turns of the keeper in the joins (each turn
    /// pays the digest of the snapshot of every reader, O(N)).
    pub tick_every: u64,
    /// The hosts serve no paths (a plain Blossom host).
    pub no_paths: bool,
    /// Every reader put out is replaced by a new one.
    pub churn: bool,
    /// The phases to run (`None`: all of `PHASES`).
    pub phases: Option<Vec<String>>,
    /// The first phase to run.
    pub from: Option<String>,
    /// The folder of a saved state to go on from.
    pub resume: Option<PathBuf>,
}

/// The phases after the joins, in the order they run: what depends on N
/// first, the journal of a day of bans (which does not) after, so that a
/// run cut by its time still says the most.
pub const PHASES: &[&str] = &[
    "snapshot",
    "epoch_one",
    "restart",
    "catch_up_open",
    "epochs_day",
    "ban_wave",
    "outage",
    "catch_up_from_join",
    "levels",
    "migration",
    "recover",
    "keeper_handover",
    "owner_transfer",
];

/// The joins: what every state is saved after at least.
const JOINS: &str = "joins";

fn phase_at(name: &str) -> Result<usize> {
    PHASES.iter().position(|p| *p == name).ok_or_else(|| bad(&format!("no phase {name} (the phases: {})", PHASES.join(","))))
}

impl Args {
    pub fn parse(args: &[String]) -> Result<Self> {
        let mut a = Args {
            n: 10_000,
            dir: PathBuf::from("bench-out"),
            from_join: true,
            catch_up: 288,
            days: vec![1, 7, 30],
            tick_every: 10_000,
            no_paths: false,
            churn: false,
            phases: None,
            from: None,
            resume: None,
        };
        let mut dir = None;
        let mut it = args.iter();
        while let Some(x) = it.next() {
            let mut val = || it.next().cloned().ok_or_else(|| bad(&format!("{x} wants a value")));
            match x.as_str() {
                "--n" => a.n = val()?.replace('_', "").parse().map_err(|_| bad("--n"))?,
                "--dir" => dir = Some(PathBuf::from(val()?)),
                "--no-from-join" => a.from_join = false,
                "--catch-up" => a.catch_up = val()?.parse().map_err(|_| bad("--catch-up"))?,
                "--days" => {
                    let mut d: Vec<u32> = val()?.split(',').map(|x| x.trim().parse().map_err(|_| bad("--days"))).collect::<Result<_>>()?;
                    d.sort_unstable();
                    d.dedup();
                    if d.first() == Some(&0) || d.is_empty() {
                        return Err(bad("--days wants days of 1 or more"));
                    }
                    a.days = d;
                }
                "--tick-every" => a.tick_every = val()?.replace('_', "").parse::<u64>().map_err(|_| bad("--tick-every"))?.max(1),
                "--no-paths" => a.no_paths = true,
                "--churn" => a.churn = true,
                "--phases" => {
                    let p: Vec<String> = val()?.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
                    for x in &p {
                        phase_at(x)?;
                    }
                    a.phases = Some(p);
                }
                "--from" => {
                    let p = val()?;
                    phase_at(&p)?;
                    a.from = Some(p);
                }
                "--resume" => a.resume = Some(PathBuf::from(val()?)),
                other => return Err(bad(&format!("unknown argument {other}"))),
            }
        }
        if a.phases.is_some() && a.from.is_some() {
            return Err(bad("--phases or --from, not both"));
        }
        a.dir = dir.or_else(|| a.resume.clone()).unwrap_or(a.dir);
        Ok(a)
    }

    /// Whether the phase `i` of `PHASES` is asked for.
    fn wants(&self, i: usize) -> bool {
        let name = PHASES[i];
        (name != "catch_up_from_join" || self.from_join)
            && self.phases.as_ref().is_none_or(|p| p.iter().any(|x| x == name))
            && self.from.as_deref().is_none_or(|f| PHASES.iter().position(|p| *p == f).is_some_and(|at| i >= at))
    }
}

fn bad(what: &str) -> MessengerError {
    MessengerError::Invalid(format!("bench: {what}"))
}

/// The key of synthetic reader `i`, the same every time (nothing is kept
/// of a million keys but their number).
fn reader_keys(i: u64) -> Result<Keys> {
    let mut h = Sha256::new();
    h.update(b"communities-bench reader");
    h.update(i.to_le_bytes());
    let raw: [u8; 32] = h.finalize().into();
    let sk = SecretKey::from_slice(&raw).map_err(|e| MessengerError::Crypto(e.to_string()))?;
    Ok(Keys::new(sk))
}

fn reader_pk(i: u64) -> Result<PubKey> {
    let k = reader_keys(i)?;
    PubKey::parse(&k.public_key().to_hex()).ok_or_else(|| bad("pubkey"))
}

/// VmRSS and VmHWM of this process, MB.
pub fn mem_mb() -> (f64, f64) {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |name: &str| {
        s.lines().find(|l| l.starts_with(name)).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<f64>().ok()).map_or(0.0, |kb| kb / 1024.0)
    };
    (field("VmRSS:"), field("VmHWM:"))
}

/// The CPU time of this process, user and system, seconds
/// (`/proc/self/stat`, fields 14 and 15, at `USER_HZ` = 100).
pub fn cpu_s() -> (f64, f64) {
    let s = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    // After the name of the command, which may hold spaces: field 3 on.
    let rest = s.rsplit_once(')').map_or("", |(_, r)| r);
    let f: Vec<&str> = rest.split_whitespace().collect();
    let tick = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok()).map_or(0.0, |t| t / 100.0);
    (tick(11), tick(12))
}

/// p50, p95, p99 and the largest of `v` (nearest rank), rounded.
pub fn quantiles(mut v: Vec<f64>) -> Value {
    if v.is_empty() {
        return json!({ "count": 0 });
    }
    v.sort_by(f64::total_cmp);
    let at = |q: f64| {
        let i = ((q * v.len() as f64).ceil() as usize).clamp(1, v.len()) - 1;
        (v[i] * 100.0).round() / 100.0
    };
    json!({ "count": v.len(), "p50": at(0.50), "p95": at(0.95), "p99": at(0.99), "max": at(1.0) })
}

fn mb(bytes: usize) -> f64 {
    (bytes as f64 / 1_048_576.0 * 100.0).round() / 100.0
}

fn ms(d: std::time::Duration) -> f64 {
    (d.as_secs_f64() * 1e5).round() / 100.0
}

/// The counters of the fakes at a moment.
#[derive(Clone, Copy, Debug, Default)]
struct Counters {
    wraps: usize,
    wrap_bytes: usize,
    scoped: usize,
    scoped_bytes: usize,
    puts: usize,
    put_bytes: usize,
    gets: usize,
    get_bytes: usize,
    deletes: usize,
}

impl Counters {
    fn of(w: &World, keeper: &Node) -> Self {
        let n = &keeper.net;
        let h = &w.hosts;
        Counters {
            wraps: n.wraps.load(Ordering::SeqCst),
            wrap_bytes: n.wrap_bytes.load(Ordering::SeqCst),
            scoped: n.scoped.load(Ordering::SeqCst),
            scoped_bytes: n.scoped_bytes.load(Ordering::SeqCst),
            puts: h.puts.load(Ordering::SeqCst),
            put_bytes: h.put_bytes.load(Ordering::SeqCst),
            gets: h.gets.load(Ordering::SeqCst),
            get_bytes: h.get_bytes.load(Ordering::SeqCst),
            deletes: h.deletes.load(Ordering::SeqCst),
        }
    }

    fn since(self, before: Counters) -> Counters {
        Counters {
            wraps: self.wraps - before.wraps,
            wrap_bytes: self.wrap_bytes - before.wrap_bytes,
            scoped: self.scoped - before.scoped,
            scoped_bytes: self.scoped_bytes - before.scoped_bytes,
            puts: self.puts - before.puts,
            put_bytes: self.put_bytes - before.put_bytes,
            gets: self.gets - before.gets,
            get_bytes: self.get_bytes - before.get_bytes,
            deletes: self.deletes - before.deletes,
        }
    }

    fn json(&self) -> Value {
        json!({
            "wraps": self.wraps, "wrap_mb": mb(self.wrap_bytes),
            "events": self.scoped, "event_kb": (self.scoped_bytes as f64 / 1024.0).round(),
            "puts": self.puts, "put_mb": mb(self.put_bytes),
            "gets": self.gets, "get_kb": (self.get_bytes as f64 / 1024.0).round(),
            "deletes": self.deletes,
        })
    }
}

/// The link of the channel, as a synthetic reader proves it in a join.
struct Door {
    link_epoch: u32,
    secret: LinkSecret,
}

/// What the joins of a phase cost and waited.
#[derive(Default)]
struct JoinStats {
    joined: u64,
    /// Minutes the keeper said `Busy` (120 a minute): the clock moved on.
    busy_minutes: u64,
    /// Turns of the keeper a join at the wall waited for (`Busy` with the
    /// emergency generation to come).
    wall_turns: u64,
    /// When each join came, by the fake clock, of the issues not yet sent.
    unsent: Vec<i64>,
    /// From the join to its issue on the relay, by the fake clock, s.
    to_key_s: Vec<f64>,
    /// The keeper's `on_join` of each, µs.
    join_us: Vec<f64>,
}

struct Bench {
    args: Args,
    w: World,
    owner: Node,
    reader: Node,
    /// Readers away for the catch-up, one per `Args::days`.
    away: Vec<Node>,
    cid: String,
    db_path: PathBuf,
    /// The next synthetic reader to put out (bans and leaves take them in
    /// order; the owner and the real reader are never put out).
    next_out: u64,
    /// The next synthetic reader to let in past the first `n` (`--churn`).
    next_in: u64,
    /// Readers taken by a level (`levels`), counted from the last down.
    leveled: u64,
    started: Instant,
    cpu: (f64, f64),
    /// Went on from a saved state (`--resume`).
    resumed: bool,
}

impl Bench {
    async fn out(&mut self, phase: &str, mut v: Value) {
        let (rss, hwm) = mem_mb();
        let (user, sys) = cpu_s();
        let spent = (user + sys) - (self.cpu.0 + self.cpu.1);
        self.cpu = (user, sys);
        let gen = self.gen().await;
        let wraps = self.owner.net.wraps.load(Ordering::SeqCst);
        if let Some(o) = v.as_object_mut() {
            o.insert("phase".into(), json!(phase));
            o.insert("n".into(), json!(self.args.n));
            o.insert("rss_mb".into(), json!(rss.round()));
            o.insert("hwm_mb".into(), json!(hwm.round()));
            o.insert("cpu_user_s".into(), json!((user * 100.0).round() / 100.0));
            o.insert("cpu_sys_s".into(), json!((sys * 100.0).round() / 100.0));
            o.insert("cpu_s".into(), json!((spent * 100.0).round() / 100.0));
            o.insert("gen".into(), json!(gen));
            o.insert("keeper_wraps".into(), json!(wraps));
            o.insert("t_s".into(), json!(self.started.elapsed().as_secs()));
            o.insert("resumed".into(), json!(self.resumed));
        }
        let mut o = std::io::stdout().lock();
        let _ = writeln!(o, "{v}");
        let _ = o.flush();
        // Line by line: a run cut short keeps what it measured.
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(self.args.dir.join("results.jsonl")) {
            let _ = writeln!(f, "{v}");
        }
    }

    /// The generation of `everyone` at the keeper (0: not kept here).
    async fn gen(&self) -> u32 {
        match self.keeper().await {
            Ok(k) => k.lock().await.scope(SCOPE_EVERYONE).map_or(0, |s| s.generation().gen),
            Err(_) => 0,
        }
    }

    /// The `k`-th reader put out, spread over the whole tree as keydist's
    /// simulator spreads them (the worst case of a cover): put out in the
    /// order they came, they would be one run of leaves and a cover of a
    /// dozen subsets. 999 983 is a prime, so this is a permutation.
    fn spread(&self, k: u64) -> u64 {
        (k.wrapping_mul(999_983).wrapping_add(12_345)) % self.args.n
    }

    fn c(&self) -> Counters {
        Counters::of(&self.w, &self.owner)
    }

    /// The keeper, its limit of readers lifted again: one loaded anew (a
    /// restart, a handover) takes the app's, and `--churn` lets readers in
    /// past it after.
    async fn keeper(&self) -> Result<Arc<Mutex<Keeper>>> {
        let k = self.owner.svc.keeper(&self.cid).await?.ok_or_else(|| bad("not kept here"))?;
        // The app refuses the 65 537th reader (`MAX_READERS`: what one
        // snapshot names); lifted here to measure past it.
        k.lock().await.limits.max_readers = usize::MAX;
        Ok(k)
    }

    async fn log(&self) -> Result<OpLog> {
        self.owner.svc.log(&self.cid).await?.ok_or_else(|| bad("no journal"))
    }

    /// What the fakes hold in memory: blob bytes on the hosts, events and
    /// wraps on the relay (to tell the keeper's RAM from theirs).
    fn fakes_mb(&self) -> Value {
        let blobs: usize = self.w.hosts.blobs.lock().map(|b| b.values().flat_map(|m| m.values()).map(Vec::len).sum()).unwrap_or(0);
        let events: usize = self.w.relay.events.lock().map(|e| e.len()).unwrap_or(0);
        json!({ "host_blobs_mb": mb(blobs), "relay_events": events })
    }

    /// The size of the database and the rows of the keeper's tables.
    async fn db_stats(&self) -> Result<Value> {
        let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        let wal = PathBuf::from(format!("{}-wal", self.db_path.display()));
        let mut rows = serde_json::Map::new();
        macro_rules! count {
            ($($t:literal),*) => {$(
                let n: i64 = sqlx::query_scalar(concat!("SELECT COUNT(*) FROM ", $t)).fetch_one(self.owner.svc.db().pool()).await.map_err(|e| bad(&e.to_string()))?;
                rows.insert($t.into(), json!(n));
            )*};
        }
        count!("cm_keeper_members", "cm_keeper_revoked", "cm_keeper_readers", "cm_keeper_ops", "cm_keeper_retired", "cm_keeper_uploads", "cm_keeper_missed", "cm_keeper_blobs", "cm_ops", "cm_epochs", "cm_outbox");
        Ok(json!({ "db_mb": mb(usize::try_from(size(&self.db_path)).unwrap_or(0)), "wal_mb": mb(usize::try_from(size(&wal)).unwrap_or(0)), "rows": rows }))
    }

    async fn tick(&self) -> Result<()> {
        self.owner.svc.tick().await.map(|_| ())
    }

    fn epochs_on_relay(&self) -> usize {
        self.w.relay.of_kind(KIND_EPOCH).len()
    }

    /// The bytes of the blobs of the newest published epoch, on every host.
    async fn epoch_blob_bytes(&self) -> Result<usize> {
        let k = self.keeper().await?;
        let hashes = k.lock().await.scope_keeper(SCOPE_EVERYONE).map(|s| s.published_blobs()).unwrap_or_default();
        let blobs = self.w.hosts.blobs.lock().map_err(|_| bad("hosts"))?;
        Ok(blobs.values().map(|m| hashes.iter().filter_map(|h| m.get(&hex::encode(h))).map(Vec::len).sum::<usize>()).sum())
    }

    async fn reader_newest(&self) -> Result<u32> {
        newest_of(&self.reader, &self.cid, SCOPE_EVERYONE).await
    }

    /// The newest epoch of `everyone` the keeper published, and the one
    /// pending.
    async fn head(&self) -> Result<(u32, Option<u32>)> {
        let k = self.keeper().await?;
        let g = k.lock().await;
        let sk = g.scope_keeper(SCOPE_EVERYONE).ok_or_else(|| bad("scope"))?;
        Ok((sk.published_head().map_or(0, |h| h.epoch), sk.pending_epoch()))
    }

    async fn door(&self) -> Result<Door> {
        let link_epoch = self.log().await?.state().link_epoch;
        let secret = crate::secrets::links(self.owner.svc.env.secrets.as_ref(), &self.cid).await?.get(&link_epoch).cloned().ok_or_else(|| bad("no link secret"))?;
        Ok(Door { link_epoch, secret })
    }

    /// The outbox sent; the issues it held reached the relay now.
    async fn flush(&self, st: &mut JoinStats) -> Result<()> {
        self.owner.svc.flush_outbox().await?;
        let now = self.w.clock.now();
        st.to_key_s.extend(st.unsent.drain(..).map(|at| (now - at) as f64));
        Ok(())
    }

    /// A turn of the keeper, as the app's loop makes it, with the jobs of
    /// the hosts run off the keeper; whether a snapshot was asked for.
    async fn keeper_turn(&self, k: &Arc<Mutex<Keeper>>, log: &OpLog) -> Result<String> {
        let mut g = k.lock().await;
        let now = self.w.clock.now();
        let mut t = self.owner.svc.keeper_tick(&mut g, log, now).await?;
        drop(g);
        let jobs = self.owner.svc.take_jobs(std::mem::take(&mut t.jobs));
        self.owner.svc.run_jobs(jobs).await;
        Ok(format!("{:?}", t.snapshot))
    }

    /// Synthetic reader `i` let in by `Keeper::on_join` with a proof of
    /// the link, as a join note would after the queue, `arrived` by the
    /// fake clock. 120 a minute: past them the next minute, the issues of
    /// this one sent first; at the wall with the emergency generation to
    /// come, a turn of the keeper.
    async fn join_one(&self, k: &Arc<Mutex<Keeper>>, log: &OpLog, door: &Door, i: u64, arrived: i64, st: &mut JoinStats) -> Result<()> {
        let keys = reader_keys(i)?;
        let pk = PubKey::parse(&keys.public_key().to_hex()).ok_or_else(|| bad("pubkey"))?;
        loop {
            let now = self.w.clock.now();
            let join = Join {
                community_id: self.cid.clone(),
                proof: Some(JoinProof { link_epoch: door.link_epoch, mac: door.secret.join_mac(&self.cid, &pk, door.link_epoch) }),
                admit_id: None,
                created_at: now,
            };
            let t = Instant::now();
            let (joined, walled) = {
                let mut g = k.lock().await;
                let j = g.on_join(log, &pk, &join, now).await?;
                let walled = matches!(j, Joined::Busy) && g.wall(log.state(), SCOPE_EVERYONE).await.is_some();
                (j, walled)
            };
            match joined {
                Joined::Issued => {
                    st.join_us.push(t.elapsed().as_secs_f64() * 1e6);
                    st.unsent.push(arrived);
                    st.joined += 1;
                    return Ok(());
                }
                Joined::Busy if walled => {
                    st.wall_turns += 1;
                    if st.wall_turns > 100 {
                        return Err(bad(&format!("join {i}: the keeper stays at the wall")));
                    }
                    self.keeper_turn(k, log).await?;
                }
                Joined::Busy => {
                    st.busy_minutes += 1;
                    self.flush(st).await?;
                    self.w.clock.advance(60);
                }
                other => return Err(bad(&format!("join {i}: {other:?}"))),
            }
        }
    }

    /// `--churn`: a new reader for each one put out.
    async fn replace(&mut self, count: u64) -> Result<JoinStats> {
        let mut st = JoinStats::default();
        if !self.args.churn || count == 0 {
            return Ok(st);
        }
        let (k, log, door) = (self.keeper().await?, self.log().await?, self.door().await?);
        for _ in 0..count {
            let i = self.next_in;
            self.next_in += 1;
            let now = self.w.clock.now();
            self.join_one(&k, &log, &door, i, now, &mut st).await?;
        }
        self.flush(&mut st).await?;
        Ok(st)
    }

    // ─── 0. The channel, the owner and the real readers ─────────────────────

    async fn setup(args: Args) -> Result<Self> {
        std::fs::create_dir_all(&args.dir).map_err(|e| bad(&e.to_string()))?;
        let db_path = args.dir.join(db::FILE_NAME);
        let mut old = vec![args.dir.join("results.jsonl")];
        for name in [db::FILE_NAME, "owner-b.sqlite", "owner-m.sqlite"] {
            old.extend(state::db_files(&args.dir.join(name)));
        }
        for p in old {
            let _ = std::fs::remove_file(p);
        }
        state::remove_all(&args.dir);
        let w = World::new();
        if args.no_paths {
            w.hosts.set_paths(H1, false);
            w.hosts.set_paths(H2, false);
        }
        let owner_keys = Keys::generate();
        let reader_keys = Keys::generate();
        let away_keys: Vec<Keys> = args.days.iter().map(|_| Keys::generate()).collect();
        // Only the owner's (the keeper's inbox) and the real readers' wraps
        // are kept; a million issues are counted and dropped.
        if let Ok(mut k) = w.relay.keep_inbox.lock() {
            *k = Some([&owner_keys, &reader_keys].into_iter().chain(&away_keys).map(|k| k.public_key().to_hex()).collect());
        }
        let owner = w.device_on(owner_keys, Db::open(&db_path).await?);
        let reader = w.device(reader_keys).await;
        let mut away = Vec::new();
        for k in away_keys {
            away.push(w.device(k).await);
        }
        let new = NewChannel {
            name: "Bench".into(),
            about: String::new(),
            picture: String::new(),
            join: JoinMode::Open,
            history: History::Open,
            relays: vec!["wss://r1.example".into()],
            storage: vec![H1.into(), H2.into()],
        };
        let (cid, _) = owner.svc.create(&owner.keys, new).await?;
        w.pump(&[&owner]).await;
        let link = owner.svc.share_link(&cid).await?;
        for r in std::iter::once(&reader).chain(&away) {
            w.pump(&[r]).await;
            r.svc.open_link(&r.keys, &link, "").await?;
            w.pump(&[&owner, r]).await;
        }
        let next_in = args.n;
        let mut b = Bench { args, w, owner, reader, away, cid, db_path, next_out: 0, next_in, leveled: 0, started: Instant::now(), cpu: cpu_s(), resumed: false };
        let newest = b.reader_newest().await?;
        if newest == 0 {
            return Err(bad("the real reader did not get its key"));
        }
        let machine = machine();
        let errors = b.owner.errors().len() + b.reader.errors().len();
        let flags = json!({ "no_paths": b.args.no_paths, "churn": b.args.churn, "tick_every": b.args.tick_every, "days": b.args.days, "catch_up": b.args.catch_up });
        b.out("setup", json!({ "reader_epoch": newest, "machine": machine, "args": flags, "errors": errors })).await;
        Ok(b)
    }

    // ─── 1. Joins ───────────────────────────────────────────────────────────

    /// All `n` readers come by the link at once (a link posted somewhere):
    /// the keeper takes 120 a minute, and the time to the key of each is
    /// from then to its issue on the relay, by the fake clock (the outbox
    /// sends at every minute of the queue).
    async fn joins(&mut self) -> Result<()> {
        let n = self.args.n;
        let log = self.log().await?;
        let door = self.door().await?;
        let k = self.keeper().await?;
        let batch = self.args.tick_every.min(n);
        let c0 = self.c();
        let t0 = Instant::now();
        let came = self.w.clock.now();
        let mut st = JoinStats::default();
        let mut joins_ms = 0.0;
        let mut ticks_ms = 0.0;
        let mut i = 0u64;
        let mut reported = 0u64;
        while i < n {
            let tb = Instant::now();
            let end = (i + batch).min(n);
            while i < end {
                self.join_one(&k, &log, &door, i, came, &mut st).await?;
                i += 1;
            }
            let joined = tb.elapsed();
            joins_ms += ms(joined);
            // A turn of the keeper after each batch, as the app's loop
            // would make (the snapshot of the readers is part of it).
            let tt = Instant::now();
            let turn = self.keeper_turn(&k, &log).await?;
            let ticked = tt.elapsed();
            ticks_ms += ms(ticked);
            if i / 100_000 > reported / 100_000 || i == n || i <= 20_000 {
                reported = i;
                self.out("joins_batch", json!({ "joined": i, "batch_ms": ms(joined), "per_join_us": (joined.as_secs_f64() * 1e6 / batch as f64).round(), "tick_ms": ms(ticked), "snapshot": turn })).await;
            }
        }
        self.flush(&mut st).await?;
        let c = self.c().since(c0);
        let days = (self.w.clock.now() - crate::testkit::T0) as f64 / 86_400.0;
        let db = self.db_stats().await?;
        let to_key_min: Vec<f64> = st.to_key_s.iter().map(|s| s / 60.0).collect();
        let fakes = self.fakes_mb();
        self.out(
            "joins",
            json!({
                "joins": n, "wall_s": t0.elapsed().as_secs(), "joins_ms": joins_ms.round(), "ticks_ms": ticks_ms.round(),
                "per_10k_s": (joins_ms / (n as f64 / 10_000.0) / 1000.0 * 100.0).round() / 100.0,
                "busy_minutes": st.busy_minutes, "wall_turns": st.wall_turns, "fake_days": (days * 100.0).round() / 100.0,
                "time_to_key_min": quantiles(to_key_min), "on_join_us": quantiles(st.join_us),
                "issue_avg_bytes": c.wrap_bytes.checked_div(c.wraps).unwrap_or(0), "net": c.json(), "db": db, "fakes": fakes,
            }),
        )
        .await;
        Ok(())
    }

    // ─── 3. Snapshot ────────────────────────────────────────────────────────

    async fn snapshot(&mut self) -> Result<()> {
        let log = self.log().await?;
        let k = self.keeper().await?;
        let mut g = k.lock().await;
        // What every turn of the keeper pays before it knows whether the
        // readers changed: the entries and the digest of every shard.
        let t = Instant::now();
        let entries = g.entries(log.state()).await?;
        let t_entries = t.elapsed();
        let t = Instant::now();
        let mut h = Sha256::new();
        for c in entries.chunks(SHARD_ENTRIES) {
            h.update(snap::encode_shard(c)?);
        }
        let _digest = h.finalize();
        let t_digest = t.elapsed();
        // Sealed under the newest key, as `snapshot_if_due` seals them;
        // past 64 shards the engine makes none (TooLarge).
        let key = g.scope_keeper(SCOPE_EVERYONE).and_then(|s| s.published_head().cloned()).ok_or_else(|| bad("no epoch"))?;
        let t = Instant::now();
        let mut bytes = 0usize;
        let mut leaves = Vec::new();
        let chunks: Vec<&[snap::Entry]> = entries.chunks(SHARD_ENTRIES).collect();
        for c in &chunks {
            let sealed = snap::seal_shard(&self.cid, &key, c)?;
            bytes += sealed.len();
            leaves.push(<[u8; 32]>::from(Sha256::digest(&sealed)));
        }
        let _root = snap::merkle_root(&leaves);
        let t_seal = t.elapsed();
        drop(g);
        self.out(
            "snapshot",
            json!({
                "entries": entries.len(), "entries_ms": ms(t_entries), "digest_ms": ms(t_digest), "seal_ms": ms(t_seal),
                "shards": chunks.len(), "shards_mb": mb(bytes), "engine_allows": chunks.len() <= MAX_SHARDS,
            }),
        )
        .await;
        Ok(())
    }

    // ─── 2. Epochs ──────────────────────────────────────────────────────────

    async fn ban(&mut self, count: u64) -> Result<f64> {
        let t = Instant::now();
        for _ in 0..count {
            let who = reader_pk(self.spread(self.next_out))?;
            self.next_out += 1;
            self.owner.svc.act(&self.owner.keys, &self.cid, OpBody::Ban { who, reason: None }).await?;
        }
        Ok(ms(t.elapsed()))
    }

    /// One epoch with 0.1 % of the readers banned: built and uploaded with
    /// the event held back by the relay (to look at what was built), then
    /// published once the outbox tries the event again; the real reader
    /// opens it.
    async fn first_epoch(&mut self) -> Result<()> {
        let bans = (self.args.n / 1000).max(1);
        let ban_ms = self.ban(bans).await?;
        // The reader takes the bans of the journal first: its open of the
        // epoch is measured alone.
        let t = Instant::now();
        self.w.pump(&[&self.reader]).await;
        let reader_ops_ms = ms(t.elapsed());
        self.w.clock.advance(300);
        self.w.relay.refuse(KIND_EPOCH, true);
        let c0 = self.c();
        let t = Instant::now();
        self.tick().await?;
        let build_ms = ms(t.elapsed());
        let c1 = self.c();
        let built = {
            let k = self.keeper().await?;
            let g = k.lock().await;
            let sd = g.scope(SCOPE_EVERYONE).ok_or_else(|| bad("scope"))?;
            let out = sd.pending().ok_or_else(|| bad("nothing built"))?;
            let shards: Vec<usize> = out.blobs.iter().filter(|b| b.kind == keydist::BlobKind::Shard).map(|b| b.bytes.len()).collect();
            let index: usize = out.blobs.iter().filter(|b| b.kind != keydist::BlobKind::Shard).map(|b| b.bytes.len()).sum();
            let st = sd.stats();
            json!({
                "epoch": out.key.epoch, "cover": out.cover, "shards": shards.len(), "shards_bytes": shards.iter().sum::<usize>(),
                "shard_max_bytes": shards.iter().max().copied().unwrap_or(0), "index_bytes": index,
                "revoked": st.revoked, "members": st.members,
            })
        };
        self.w.relay.refuse(KIND_EPOCH, false);
        // The event the relay refused is tried again by the outbox after
        // `OWED_FIRST_WAIT_SECS` (on 58240e3d9 the bench moved the clock a
        // second, and no reader of any N opened this epoch).
        self.w.clock.advance(OWED_FIRST_WAIT_SECS);
        let t = Instant::now();
        self.tick().await?;
        let publish_ms = ms(t.elapsed());
        let c2 = self.c();
        let r0 = self.c();
        let t = Instant::now();
        self.w.pump(&[&self.reader]).await;
        let read_ms = ms(t.elapsed());
        let r = self.c().since(r0);
        let newest = self.reader_newest().await?;
        let (head, _) = self.head().await?;
        let db = self.db_stats().await?;
        self.out(
            "epoch_one",
            json!({
                "bans": bans, "ban_acts_ms": ban_ms, "per_ban_ms": (ban_ms / bans as f64 * 100.0).round() / 100.0, "reader_ops_ms": reader_ops_ms,
                "build_upload_ms": build_ms, "publish_tick_ms": publish_ms, "built": built,
                "upload": c1.since(c0).json(), "publish": c2.since(c1).json(),
                "reader": { "ms": read_ms, "gets": r.gets, "get_kb": (r.get_bytes as f64 / 1024.0).round(), "newest": newest, "opened": newest == head },
                "db": db,
            }),
        )
        .await;
        if newest != head {
            return Err(bad(&format!("epoch_one: the reader holds epoch {newest}, the keeper published {head}")));
        }
        Ok(())
    }

    /// The daily pattern: `epochs` epochs with `bans` bans each, the real
    /// reader online through them.
    async fn day(&mut self, epochs: u32, bans: u64) -> Result<()> {
        let c0 = self.c();
        let mut ban_ms = 0.0;
        let mut batches: Vec<f64> = Vec::new();
        let mut epoch_ms: Vec<f64> = Vec::new();
        let mut read_ms: Vec<f64> = Vec::new();
        let mut read_ops_ms = 0.0;
        let (mut gets, mut get_bytes) = (0usize, 0usize);
        let mut puts_per_epoch: Vec<usize> = Vec::new();
        let before = self.epochs_on_relay();
        let mut epoch_bytes = 0usize;
        let snaps_before = self.w.relay.of_kind(KIND_SNAPSHOT).len();
        let mut joined = 0;
        for _ in 0..epochs {
            let this = self.ban(bans).await?;
            joined += self.replace(bans).await?.joined;
            batches.push(this);
            ban_ms += this;
            let t = Instant::now();
            self.w.pump(&[&self.reader]).await;
            read_ops_ms += ms(t.elapsed());
            self.w.clock.advance(300);
            let p0 = self.c();
            let t = Instant::now();
            self.tick().await?;
            epoch_ms.push(ms(t.elapsed()));
            puts_per_epoch.push(self.c().since(p0).put_bytes);
            epoch_bytes += self.epoch_blob_bytes().await?;
            let r0 = self.c();
            let t = Instant::now();
            self.w.pump(&[&self.reader]).await;
            read_ms.push(ms(t.elapsed()));
            let r = self.c().since(r0);
            gets += r.gets;
            get_bytes += r.get_bytes;
        }
        let made = self.epochs_on_relay() - before;
        let snaps = self.w.relay.of_kind(KIND_SNAPSHOT).len() - snaps_before;
        let epoch_avg = epoch_bytes / made.max(1);
        let c = self.c().since(c0);
        let avg = |v: &[f64]| (v.iter().sum::<f64>() / v.len().max(1) as f64 * 100.0).round() / 100.0;
        let max = |v: &[f64]| v.iter().copied().fold(0.0, f64::max);
        let put_avg = c.put_bytes / made.max(1);
        let newest = self.reader_newest().await?;
        let db = self.db_stats().await?;
        let k = self.keeper().await?;
        let revoked = k.lock().await.scope(SCOPE_EVERYONE).map(|s| s.stats().revoked).unwrap_or(0);
        let fakes = self.fakes_mb();
        self.out(
            "epochs_day",
            json!({
                "epochs": made, "bans_each": bans, "joined": joined, "ban_acts_ms_avg": (ban_ms / (f64::from(epochs) * bans as f64) * 100.0).round() / 100.0,
                "ban_ms_first_batch": (batches.first().copied().unwrap_or(0.0) / bans as f64 * 100.0).round() / 100.0, "ban_ms_last_batch": (batches.last().copied().unwrap_or(0.0) / bans as f64 * 100.0).round() / 100.0,
                "epoch_tick_ms_avg": avg(&epoch_ms), "epoch_tick_ms_max": max(&epoch_ms),
                "put_mb_per_epoch_avg": mb(put_avg), "put_mb_per_epoch_last": mb(puts_per_epoch.last().copied().unwrap_or(0)),
                "epoch_blobs_kb_avg": epoch_avg / 1024, "snapshots": snaps, "snapshot_put_mb_avg": mb(c.put_bytes.saturating_sub(epoch_bytes) / snaps.max(1)),
                "epoch_blobs_gb_per_day_80": (epoch_avg as f64 * 80.0 / 1e9 * 1000.0).round() / 1000.0,
                "put_gb_per_day_80": (put_avg as f64 * 80.0 / 1e9 * 100.0).round() / 100.0, "put_gb_per_day_288": (put_avg as f64 * 288.0 / 1e9 * 100.0).round() / 100.0,
                "revoked": revoked, "net": c.json(),
                "reader": { "ops_ms": read_ops_ms.round(), "epoch_ms_avg": avg(&read_ms), "gets_per_epoch": gets as f64 / made.max(1) as f64, "get_kb_per_epoch": (get_bytes as f64 / made.max(1) as f64 / 1024.0).round(), "newest": newest },
                "db": db, "fakes": fakes,
            }),
        )
        .await;
        Ok(())
    }

    // ─── 2b. A wave of bans ─────────────────────────────────────────────────

    /// `total` bans spread over `minutes` (ROO-397 §2, 2: 2000 in an
    /// hour), the keeper turning every minute as the loop of the app turns
    /// on each: from each ban to the first epoch published without the one
    /// banned (by the fake clock; the real reader opens it in the same
    /// minute).
    async fn ban_wave(&mut self, total: u64, minutes: u64) -> Result<()> {
        let c0 = self.c();
        let before = self.epochs_on_relay();
        let mut banned_at: Vec<i64> = Vec::new();
        // The number of bans made when each epoch was built.
        let mut built_over: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
        let mut credited = 0usize;
        let mut to_epoch_s: Vec<f64> = Vec::new();
        let (mut head, _) = self.head().await?;
        let mut tick_ms: Vec<f64> = Vec::new();
        let mut read_ms: Vec<f64> = Vec::new();
        let mut ban_ms = 0.0;
        let mut joined = 0;
        for m in 0..minutes {
            let count = (m + 1) * total / minutes - m * total / minutes;
            let start = self.w.clock.now();
            for _ in 0..count {
                banned_at.push(self.w.clock.now());
                ban_ms += self.ban(1).await?;
                if self.w.clock.now() - start < 59 {
                    self.w.clock.advance(1);
                }
            }
            joined += self.replace(count).await?.joined;
            let rest = 60 - (self.w.clock.now() - start);
            self.w.clock.advance(rest.max(0));
            let t = Instant::now();
            self.tick().await?;
            tick_ms.push(ms(t.elapsed()));
            let (now_head, pending) = self.head().await?;
            if let Some(p) = pending {
                built_over.entry(p).or_insert(banned_at.len());
            }
            if now_head != head {
                let over = built_over.get(&now_head).copied().unwrap_or(banned_at.len());
                let now = self.w.clock.now();
                to_epoch_s.extend(banned_at[credited..over.max(credited)].iter().map(|at| (now - at) as f64));
                credited = over.max(credited);
                head = now_head;
            }
            let t = Instant::now();
            self.w.pump(&[&self.reader]).await;
            read_ms.push(ms(t.elapsed()));
        }
        // What is left after the hour: the epochs that take the last bans.
        for _ in 0..12 {
            if credited >= banned_at.len() {
                break;
            }
            self.w.clock.advance(60);
            self.tick().await?;
            let (now_head, pending) = self.head().await?;
            if let Some(p) = pending {
                built_over.entry(p).or_insert(banned_at.len());
            }
            if now_head != head {
                let over = built_over.get(&now_head).copied().unwrap_or(banned_at.len());
                let now = self.w.clock.now();
                to_epoch_s.extend(banned_at[credited..over.max(credited)].iter().map(|at| (now - at) as f64));
                credited = over.max(credited);
                head = now_head;
            }
            self.w.pump(&[&self.reader]).await;
        }
        let c = self.c().since(c0);
        let made = self.epochs_on_relay() - before;
        let newest = self.reader_newest().await?;
        let k = self.keeper().await?;
        let revoked = k.lock().await.scope(SCOPE_EVERYONE).map(|s| s.stats().revoked).unwrap_or(0);
        let avg = |v: &[f64]| (v.iter().sum::<f64>() / v.len().max(1) as f64 * 100.0).round() / 100.0;
        let db = self.db_stats().await?;
        self.out(
            "ban_wave",
            json!({
                "bans": banned_at.len(), "minutes": minutes, "joined": joined, "epochs": made, "ban_act_ms_avg": (ban_ms / banned_at.len().max(1) as f64 * 100.0).round() / 100.0,
                "ban_to_epoch_s": quantiles(to_epoch_s), "bans_not_out": banned_at.len() - credited,
                "tick_ms_avg": avg(&tick_ms), "tick_ms_max": tick_ms.iter().copied().fold(0.0, f64::max),
                "put_mb_per_epoch": mb(c.put_bytes / made.max(1)), "revoked": revoked, "net": c.json(),
                "reader": { "minute_ms_avg": avg(&read_ms), "newest": newest, "opened": newest == head },
                "db": db,
            }),
        )
        .await;
        Ok(())
    }

    // ─── 2c. The hosts down ─────────────────────────────────────────────────

    /// Every host down for `minutes`, a ban every `every` minutes (ROO-397
    /// §2, 3; the audit of ROO-396, 1.2): the channel lives when no epoch
    /// published opens a gap past `MAX_GAP` and the reader holds the newest
    /// epoch once the hosts are back, which leaves out every reader banned
    /// meanwhile.
    async fn outage(&mut self, minutes: i64, every: i64) -> Result<()> {
        let c0 = self.c();
        let (head0, _) = self.head().await?;
        let start = self.w.clock.now();
        self.w.hosts.set_down(H1, true);
        self.w.hosts.set_down(H2, true);
        let mut bans = 0u64;
        let mut tick_ms: Vec<f64> = Vec::new();
        while self.w.clock.now() - start < minutes * 60 {
            self.ban(1).await?;
            bans += 1;
            self.w.clock.advance(every * 60);
            let t = Instant::now();
            self.tick().await?;
            tick_ms.push(ms(t.elapsed()));
            self.w.pump(&[&self.reader]).await;
        }
        let (head_down, pending_down) = self.head().await?;
        let next_down = {
            let k = self.keeper().await?;
            let g = k.lock().await;
            g.scope_keeper(SCOPE_EVERYONE).map_or(0, |s| s.next_epoch())
        };
        let banned_last = reader_pk(self.spread(self.next_out - 1))?;
        self.w.hosts.set_down(H1, false);
        self.w.hosts.set_down(H2, false);
        let up = self.w.clock.now();
        let t = Instant::now();
        // Back: the pending epoch goes out, the bans that waited make the
        // one after it; the keeper turns each minute until nothing waits.
        let mut back_s = None;
        for _ in 0..30 {
            self.tick().await?;
            self.w.pump(&[&self.reader]).await;
            let (head, pending) = self.head().await?;
            let k = self.keeper().await?;
            let (out, due) = {
                let g = k.lock().await;
                (!g.is_member(&banned_last), g.scope_keeper(SCOPE_EVERYONE).and_then(|s| s.due_at()))
            };
            if out && pending.is_none() && due.is_none() && head > head_down && self.reader_newest().await? == head {
                back_s = Some(self.w.clock.now() - up);
                break;
            }
            self.w.clock.advance(60);
        }
        let back_ms = ms(t.elapsed());
        let published: Vec<u32> = self.w.relay.epochs_of(SCOPE_EVERYONE).into_iter().filter(|e| *e >= head0).collect();
        let max_gap = published.windows(2).map(|w| w[1].saturating_sub(w[0])).max().unwrap_or(0);
        let (head, _) = self.head().await?;
        let newest = self.reader_newest().await?;
        let c = self.c().since(c0);
        let alive = back_s.is_some() && max_gap <= crate::epoch::MAX_GAP && newest == head;
        self.out(
            "outage",
            json!({
                "minutes": minutes, "ban_every_min": every, "bans": bans, "head_before": head0, "head_while_down": head_down, "pending_while_down": pending_down,
                "next_epoch_while_down": next_down, "burnt": next_down.saturating_sub(head_down + 1), "published": published, "max_gap": max_gap,
                "tick_ms_down_avg": (tick_ms.iter().sum::<f64>() / tick_ms.len().max(1) as f64 * 100.0).round() / 100.0,
                "back_s": back_s, "back_ms": back_ms, "net": c.json(), "reader": { "newest": newest, "head": head }, "alive": alive,
            }),
        )
        .await;
        if !alive {
            return Err(bad(&format!("outage: the channel did not come back (gap {max_gap}, reader {newest}, head {head}, back {back_s:?})")));
        }
        Ok(())
    }

    // ─── 5. Readers away 1, 7, 30 days ──────────────────────────────────────

    async fn leave(&mut self, count: u64) -> Result<f64> {
        let k = self.keeper().await?;
        let mut g = k.lock().await;
        let t = Instant::now();
        for _ in 0..count {
            let who = reader_pk(self.spread(self.next_out))?;
            self.next_out += 1;
            g.on_leave(&who, self.w.clock.now()).await?;
        }
        Ok(ms(t.elapsed()))
    }

    /// The readers of `away` go offline together; `days` days of epochs
    /// (`catch_up` a day, each asked for now) with leaves of 0.3 % of the
    /// readers a day; each reader comes back after its days, and the
    /// catch-up hands it the epochs since, newest first.
    async fn catch_up(&mut self, label: &str) -> Result<()> {
        let per_day = u64::from(self.args.catch_up);
        let days = self.args.days.clone();
        let total = per_day * u64::from(days.last().copied().unwrap_or(1));
        let leaves_a_day = (self.args.n * 3 / 1000).max(1);
        // In step first (what came since the last catch-up, not measured).
        let all: Vec<&Node> = self.away.iter().chain(std::iter::once(&self.reader)).collect();
        self.w.pump(&all).await;
        for r in &self.away {
            r.skip_all(&self.w.relay);
        }
        let start = self.w.relay.events.lock().map(|e| e.len()).unwrap_or(0);
        let mut held_before = Vec::new();
        for r in &self.away {
            held_before.push(keys_held(r, &self.cid).await?);
        }
        // The hosts keep the shards these readers read alone (`keep`):
        // every epoch puts every shard anew, and a history from the join
        // keeps them a month. What went to the hosts is counted whole.
        let watched: Vec<&Node> = self.away.iter().chain(std::iter::once(&self.reader)).collect();
        let leaves = leaves_of(&watched, &self.cid).await?;
        *self.w.hosts.keep.lock().map_err(|_| bad("hosts"))? = Some(keep_shards_of(leaves));
        let c0 = self.c();
        let mut keeper_ms = 0.0;
        let mut back = 0;
        let mut leaves = 0;
        let mut joined = 0;
        for e in 0..total {
            let te = Instant::now();
            // 0.3 % a day, spread evenly over its epochs.
            let day_e = e % per_day;
            let now_leaves = (day_e + 1) * leaves_a_day / per_day - day_e * leaves_a_day / per_day;
            self.leave(now_leaves).await?;
            joined += self.replace(now_leaves).await?.joined;
            leaves += now_leaves;
            self.w.clock.advance(300);
            // The keeper's own mark, as its timer asks every five minutes:
            // an operation of the journal at every epoch made a journal
            // of months no channel has.
            self.owner.svc.rotate_now(&self.cid).await?;
            self.tick().await?;
            // Who signed each put, kept for the tests: millions of puts.
            self.w.hosts.signers.lock().map_err(|_| bad("hosts"))?.clear();
            // The keeper's side alone: the reads of the readers back are
            // measured on their own.
            keeper_ms += ms(te.elapsed());
            let done_days = (e + 1) / per_day;
            if (e + 1) % per_day == 0 && back < days.len() && u64::from(days[back]) == done_days {
                let kc = self.c().since(c0);
                let evs: Vec<Value> = self.w.relay.events.lock().map(|e| e[start..].iter().filter(|e| e["kind"].as_u64() == Some(u64::from(KIND_EPOCH))).cloned().collect()).unwrap_or_default();
                let reader = &self.away[back];
                let r0 = self.c();
                let tr = Instant::now();
                // The notes to me first, as a catch-up brings them: the
                // bundle of a generation the keeper made while I was away
                // (on hosts without paths, every member's at the wall).
                let notes = reader.hear_inbox(&reader.keys.public_key().to_hex()).await;
                for ev in evs.iter().rev() {
                    reader.svc.on_event(&reader.keys, ev, false).await?;
                }
                // The epochs heard ask the hosts for their blobs, as the
                // module runs those fetches beside the loop.
                reader.svc.run_fetches(&reader.keys).await;
                let read_ms = ms(tr.elapsed());
                let r = self.c().since(r0);
                let held = keys_held(reader, &self.cid).await?.saturating_sub(held_before[back]);
                let newest = newest_of(reader, &self.cid, SCOPE_EVERYONE).await?;
                let event_bytes: usize = evs.iter().map(|e| serde_json::to_string(e).map_or(0, |s| s.len())).sum();
                let epoch_rows = db::keys::epochs(reader.svc.db(), &self.cid, SCOPE_EVERYONE).await?.len();
                // The rest of what came while away (the rotate_now of the
                // journal, the epochs again), not measured: a journal with
                // holes would hold every later operation of the reader back.
                self.w.pump(&[&self.away[back]]).await;
                let (head, _) = self.head().await?;
                let fakes = self.fakes_mb();
                let d = days[back];
                self.out(
                    &format!("{label}_{d}d"),
                    json!({
                        "days": d, "epochs": evs.len(), "leaves": leaves, "joined": joined, "keeper_ms": keeper_ms, "keeper": kc.json(),
                        "reader": { "ms": read_ms, "notes": notes, "keys_opened": held, "newest": newest, "head": head, "gets": r.gets, "get_mb": mb(r.get_bytes), "events": evs.len(), "events_kb": event_bytes / 1024, "cm_epochs": epoch_rows },
                        "fakes": fakes,
                    }),
                )
                .await;
                if newest != head {
                    return Err(bad(&format!("{label}: the reader back after {d} days holds epoch {newest}, the keeper published {head}")));
                }
                back += 1;
            }
        }
        self.w.pump(&[&self.reader]).await;
        *self.w.hosts.keep.lock().map_err(|_| bad("hosts"))? = None;
        Ok(())
    }

    async fn set_history(&mut self, h: History) -> Result<()> {
        let body = OpBody::Settings {
            name: None,
            about: None,
            picture: None,
            history: Some(h),
            relays: None,
            storage: None,
            post_mode: None,
            premod: None,
            schedule: None,
            comments: None,
            join: None,
        };
        self.owner.svc.act(&self.owner.keys, &self.cid, body).await?;
        // The operation is in the outbox: the keeper's turn sends it (on
        // 58240e3d9 the bench asked the reader before it went out).
        self.tick().await?;
        self.w.pump(&[&self.reader]).await;
        let seen = self.reader.svc.log(&self.cid).await?.map(|l| l.state().history);
        if seen != Some(h) {
            return Err(bad(&format!("the reader holds history {seen:?}, not {h:?}")));
        }
        Ok(())
    }

    // ─── 5b. A level taken away ─────────────────────────────────────────────

    /// A section of a level given to a tenth of the readers (at most
    /// 10 000) and the real one: the time to its key; then the level taken
    /// away from a hundredth of them: the epoch of the section that leaves
    /// them out, which the real reader opens, `everyone` unmoved.
    async fn levels(&mut self) -> Result<()> {
        let keys = self.owner.keys.clone();
        let (scope, _) = self.owner.svc.scope_add(&keys, &self.cid, "Bench", SchemeId::SdV1, History::Open).await?;
        self.tick().await?;
        let (lvl, _) = self.owner.svc.level_set(&keys, &self.cid, None, "Bench", vec![scope.clone()], LevelCan::BASE).await?;
        self.tick().await?;
        let given = (self.args.n / 10).clamp(1, 10_000);
        let everyone = self.w.relay.epochs_of(SCOPE_EVERYONE);
        let c0 = self.c();
        let t = Instant::now();
        let mut who = Vec::new();
        for _ in 0..given {
            // From the last reader down: the bans and leaves take them from the first.
            let i = self.spread(self.args.n - 1 - self.leveled);
            self.leveled += 1;
            let pk = reader_pk(i)?;
            self.owner.svc.set_levels(&keys, &self.cid, &pk, &[lvl]).await?;
            who.push(pk);
        }
        self.owner.svc.set_levels(&keys, &self.cid, &self.reader.me, &[lvl]).await?;
        let grant_ms = ms(t.elapsed());
        let t = Instant::now();
        self.tick().await?;
        let grant_tick_ms = ms(t.elapsed());
        let t = Instant::now();
        self.w.pump(&[&self.reader]).await;
        let read_ms = ms(t.elapsed());
        let first = newest_of(&self.reader, &self.cid, &scope).await?;
        let cg = self.c().since(c0);
        let members = {
            let k = self.keeper().await?;
            let g = k.lock().await;
            g.scope_keeper(&scope).map_or(0, |s| s.members_len())
        };
        // Taken away from a hundredth.
        let taken = (given / 100).max(1);
        let c1 = self.c();
        let t = Instant::now();
        for pk in who.iter().take(usize::try_from(taken).unwrap_or(usize::MAX)) {
            self.owner.svc.set_levels(&keys, &self.cid, pk, &[]).await?;
        }
        let take_ms = ms(t.elapsed());
        self.w.clock.advance(300);
        let t = Instant::now();
        self.tick().await?;
        let epoch_tick_ms = ms(t.elapsed());
        let t = Instant::now();
        self.w.pump(&[&self.reader]).await;
        let epoch_read_ms = ms(t.elapsed());
        let after = newest_of(&self.reader, &self.cid, &scope).await?;
        let ct = self.c().since(c1);
        let head = {
            let k = self.keeper().await?;
            let g = k.lock().await;
            g.scope_keeper(&scope).and_then(|s| s.published_head().map(|h| h.epoch)).unwrap_or(0)
        };
        let moved = self.w.relay.epochs_of(SCOPE_EVERYONE) != everyone;
        self.out(
            "levels",
            json!({
                "given": given + 1, "members": members, "grant_acts_ms": grant_ms, "grant_tick_ms": grant_tick_ms, "grant": cg.json(),
                "reader": { "ms": read_ms, "section_epoch": first, "after_take": after, "head": head },
                "taken": taken, "take_acts_ms": take_ms, "section_epoch_tick_ms": epoch_tick_ms, "section_epoch_read_ms": epoch_read_ms, "take": ct.json(),
                "everyone_moved": moved,
            }),
        )
        .await;
        if first == 0 || after <= first || after != head {
            return Err(bad(&format!("levels: the reader holds epochs {first} → {after} of the section, the keeper published {head}")));
        }
        Ok(())
    }

    // ─── 6. Restart ─────────────────────────────────────────────────────────

    async fn restart(&mut self) -> Result<()> {
        self.owner.svc.unload_keepers().await;
        let t = Instant::now();
        let k = self.keeper().await?;
        let load_ms = ms(t.elapsed());
        let (rss_loaded, _) = mem_mb();
        let log = self.log().await?;
        let t = Instant::now();
        self.keeper_turn(&k, &log).await?;
        let tick_ms = ms(t.elapsed());
        // A second turn: what every turn after costs (nothing changed).
        let t = Instant::now();
        self.keeper_turn(&k, &log).await?;
        let tick2_ms = ms(t.elapsed());
        let members = k.lock().await.scope(SCOPE_EVERYONE).map(|s| s.stats().members).unwrap_or(0);
        self.out("restart", json!({ "load_ms": load_ms, "rss_after_load_mb": rss_loaded.round(), "first_tick_ms": tick_ms, "next_tick_ms": tick2_ms, "members": members })).await;
        Ok(())
    }

    // ─── 4. A planned generation at 10 % revoked ────────────────────────────

    async fn migration(&mut self) -> Result<()> {
        let k = self.keeper().await?;
        let (revoked, handed) = {
            let g = k.lock().await;
            let sd = g.scope(SCOPE_EVERYONE).ok_or_else(|| bad("scope"))?;
            let gen = sd.generation();
            (gen.revoked.len() as u64, u64::from(gen.next_leaf))
        };
        let want = handed / 10 + 1;
        let leaves = want.saturating_sub(revoked);
        let leave_ms = self.leave(leaves).await?;
        let t10 = Thresholds { revoked_ratio: 0.1, grace_secs: 7 * 86_400, ..Thresholds::default() };
        let (due_10, due_default) = {
            let g = k.lock().await;
            let sd = g.scope(SCOPE_EVERYONE).ok_or_else(|| bad("scope"))?;
            (sd.migration_due(&t10).map(|r| format!("{r:?}")), sd.migration_due(&sd.state().policy.thresholds).map(|r| format!("{r:?}")))
        };
        // The engine's own wake: a migration due by the policy of the
        // scope makes `next_wake` say "now" every turn.
        let log = self.log().await?;
        let (_, hwm0) = mem_mb();
        let c0 = self.c();
        let t = Instant::now();
        {
            let mut g = k.lock().await;
            let now = self.w.clock.now();
            g.migrate_all(7 * 86_400, now).await?;
        }
        let migrate_ms = ms(t.elapsed());
        let t = Instant::now();
        {
            let mut g = k.lock().await;
            let now = self.w.clock.now();
            g.reissue_all(&log, now).await?;
        }
        let reissue_ms = ms(t.elapsed());
        let (_, hwm1) = mem_mb();
        // The bundles go out of the outbox (the module's worker).
        let t = Instant::now();
        self.owner.svc.flush_outbox().await?;
        let send_ms = ms(t.elapsed());
        let c = self.c().since(c0);
        // The first epoch under two generations.
        self.w.clock.advance(300);
        self.owner.svc.act(&self.owner.keys, &self.cid, OpBody::RotateNow { scope_id: None }).await?;
        self.tick().await?;
        let e = self.c().since(c0).since(c);
        let t = Instant::now();
        self.w.pump(&[&self.reader]).await;
        let read_ms = ms(t.elapsed());
        let newest = self.reader_newest().await?;
        let (head, _) = self.head().await?;
        let db = self.db_stats().await?;
        self.out(
            "migration",
            json!({
                "handed": handed, "revoked_before": revoked, "leaves": leaves, "leave_ms": leave_ms, "due_at_0_1": due_10, "due_by_policy": due_default,
                "migrate_ms": migrate_ms, "reissue_ms": reissue_ms, "send_ms": send_ms, "reissue": c.json(), "issue_avg_bytes": c.wrap_bytes.checked_div(c.wraps).unwrap_or(0),
                "hwm_before_mb": hwm0.round(), "hwm_after_mb": hwm1.round(), "epoch_two_gens": e.json(), "reader": { "ms": read_ms, "newest": newest, "head": head },
                "db": db,
            }),
        )
        .await;
        Ok(())
    }

    // ─── 6b. Recovery ───────────────────────────────────────────────────────

    async fn recover(&mut self) -> Result<()> {
        // A row that breaks an invariant (a revoked leaf never handed out):
        // the rows do not make a state, the keeper recovers.
        let live = {
            let k = self.keeper().await?;
            let g = k.lock().await;
            g.scope(SCOPE_EVERYONE).map_or(1, |s| s.generation().gen)
        };
        sqlx::query("INSERT INTO cm_keeper_revoked (community_id, scope_id, gen, leaf) VALUES (?, 'everyone', ?, 16777215)")
            .bind(&self.cid)
            .bind(i64::from(live))
            .execute(self.owner.svc.db().pool())
            .await
            .map_err(|e| bad(&e.to_string()))?;
        self.owner.svc.unload_keepers().await;
        let (_, hwm0) = mem_mb();
        let t = Instant::now();
        let k = self.keeper().await?;
        let load_ms = ms(t.elapsed());
        let (_, hwm1) = mem_mb();
        let (gen, kind, members) = {
            let g = k.lock().await;
            let sd = g.scope(SCOPE_EVERYONE).ok_or_else(|| bad("scope"))?;
            (sd.generation().gen, format!("{:?}", sd.generation().kind), sd.stats().members)
        };
        // Every member asks for the new bundle: a sample, then the formula.
        let sample = 10_000.min(members.saturating_sub(10));
        let log = self.log().await?;
        let c0 = self.c();
        let t = Instant::now();
        {
            let mut g = k.lock().await;
            let mut i = self.next_out;
            let mut done = 0;
            while done < sample && i < self.args.n {
                let who = reader_pk(self.spread(i))?;
                i += 1;
                let r = Rebundle { community_id: self.cid.clone(), scope_id: SCOPE_EVERYONE.into(), gen };
                if g.on_rebundle(&log, &who, &r, false, self.w.clock.now()).await? {
                    done += 1;
                }
            }
        }
        let rebundle_ms = ms(t.elapsed());
        self.owner.svc.flush_outbox().await?;
        let c = self.c().since(c0);
        let per = c.wrap_bytes.checked_div(c.wraps).unwrap_or(0);
        // The real reader asks for its bundle itself, and reads on.
        self.w.clock.advance(300);
        self.owner.svc.act(&self.owner.keys, &self.cid, OpBody::RotateNow { scope_id: None }).await?;
        for _ in 0..3 {
            self.tick().await?;
            self.w.pump(&[&self.reader]).await;
        }
        let newest = self.reader_newest().await?;
        let (head, _) = self.head().await?;
        self.out(
            "recover",
            json!({
                "load_ms": load_ms, "gen": gen, "kind": kind, "members": members, "hwm_before_mb": hwm0.round(), "hwm_after_mb": hwm1.round(),
                "rebundles": c.wraps, "rebundle_ms": rebundle_ms, "per_rebundle_us": (rebundle_ms * 1000.0 / c.wraps.max(1) as f64).round(), "bundle_bytes": per,
                "all_members_gb": (per as f64 * members as f64 / 1e9 * 100.0).round() / 100.0, "reader": { "newest": newest, "head": head },
            }),
        )
        .await;
        Ok(())
    }

    // ─── 7. The keeper to another device, the channel to another owner ──────

    /// The owner's other device takes the keeper (wire §15): the state of
    /// the keeper sealed in one blob, the pause of the readers' keys (from
    /// the take to the first epoch of the new device the reader opens, by
    /// the fake clock), and the device runs on as the keeper.
    async fn keeper_handover(&mut self) -> Result<()> {
        let path = self.args.dir.join("owner-b.sqlite");
        let b = self.w.device_on(self.owner.keys.clone(), Db::open(&path).await?);
        let t = Instant::now();
        self.w.pump(&[&self.owner, &b, &self.reader]).await;
        let in_step_ms = ms(t.elapsed());
        let before = self.reader_newest().await?;
        let h0 = (self.w.hosts.puts.load(Ordering::SeqCst), self.w.hosts.put_bytes.load(Ordering::SeqCst));
        let taken_at = self.w.clock.now();
        let t = Instant::now();
        b.svc.keeper_take(&b.keys, &self.cid, false).await?;
        self.w.pump(&[&self.owner, &b, &self.reader]).await;
        let take_ms = ms(t.elapsed());
        let h1 = (self.w.hosts.puts.load(Ordering::SeqCst), self.w.hosts.put_bytes.load(Ordering::SeqCst));
        let kept_b = b.svc.keeper(&self.cid).await?.is_some();
        let kept_a = self.owner.svc.keeper(&self.cid).await?.is_some();
        if !kept_b || kept_a {
            return Err(bad(&format!("handover: the other device keeps {kept_b}, the first {kept_a}")));
        }
        let (_, hwm) = mem_mb();
        let old = std::mem::replace(&mut self.owner, b);
        self.db_path = path;
        // A ban, the next window: the reader opens the new device's epoch.
        let mut pause_s = None;
        self.ban(1).await?;
        for _ in 0..12 {
            self.w.clock.advance(60);
            self.w.pump(&[&self.owner, &self.reader]).await;
            let newest = self.reader_newest().await?;
            if newest > before {
                pause_s = Some(self.w.clock.now() - taken_at);
                break;
            }
        }
        let newest = self.reader_newest().await?;
        drop(old);
        let members = {
            let k = self.keeper().await?;
            let g = k.lock().await;
            g.scope(SCOPE_EVERYONE).map_or(0, |s| s.stats().members)
        };
        let db = self.db_stats().await?;
        self.out(
            "keeper_handover",
            json!({
                "in_step_ms": in_step_ms, "take_ms": take_ms, "state_puts": h1.0 - h0.0, "state_mb": mb(h1.1 - h0.1), "hwm_mb_after": hwm.round(),
                "members": members, "reader": { "before": before, "newest": newest, "pause_s": pause_s }, "db": db,
            }),
        )
        .await;
        if pause_s.is_none() {
            return Err(bad(&format!("handover: the reader holds epoch {newest}, {before} before the take")));
        }
        Ok(())
    }

    /// The channel to another person (wire §16): a reader made moderator,
    /// offered the channel, accepts; its keeper makes a generation of its
    /// root and hands every member a bundle of it.
    async fn owner_transfer(&mut self) -> Result<()> {
        let path = self.args.dir.join("owner-m.sqlite");
        let mk = Keys::generate();
        if let Ok(mut k) = self.w.relay.keep_inbox.lock() {
            if let Some(k) = k.as_mut() {
                k.insert(mk.public_key().to_hex());
            }
        }
        let m = self.w.device_on(mk, Db::open(&path).await?);
        let link = self.owner.svc.share_link(&self.cid).await?;
        self.w.pump(&[&m]).await;
        m.svc.open_link(&m.keys, &link, "").await?;
        self.w.pump(&[&self.owner, &m]).await;
        self.owner.svc.act(&self.owner.keys, &self.cid, OpBody::SetRole { who: m.me.clone(), role: Role::Moderator }).await?;
        self.owner.svc.transfer(&self.owner.keys, &self.cid, &m.me).await?;
        self.w.pump(&[&self.owner, &m, &self.reader]).await;
        let gen_before = db::keys::newest_gen(self.reader.svc.db(), &self.cid, SCOPE_EVERYONE).await.unwrap_or(0);
        let before = self.reader_newest().await?;
        let w0 = m.net.wraps.load(Ordering::SeqCst);
        let wb0 = m.net.wrap_bytes.load(Ordering::SeqCst);
        let at = self.w.clock.now();
        let t = Instant::now();
        m.svc.transfer_answer(&m.keys, &self.cid, true).await?;
        self.w.pump(&[&self.owner, &m, &self.reader]).await;
        let accept_ms = ms(t.elapsed());
        let wraps = m.net.wraps.load(Ordering::SeqCst) - w0;
        let wrap_bytes = m.net.wrap_bytes.load(Ordering::SeqCst) - wb0;
        let kept = m.svc.keeper(&self.cid).await?.is_some();
        let gens = match m.svc.keeper(&self.cid).await? {
            Some(k) => k.lock().await.scope_keeper(SCOPE_EVERYONE).map_or(0, |s| s.generations()),
            None => 0,
        };
        let gen_after = db::keys::newest_gen(self.reader.svc.db(), &self.cid, SCOPE_EVERYONE).await.unwrap_or(0);
        let (_, hwm) = mem_mb();
        let old = std::mem::replace(&mut self.owner, m);
        self.db_path = path;
        let mut pause_s = None;
        self.ban(1).await?;
        for _ in 0..12 {
            self.w.clock.advance(60);
            self.w.pump(&[&self.owner, &self.reader]).await;
            if self.reader_newest().await? > before {
                pause_s = Some(self.w.clock.now() - at);
                break;
            }
        }
        drop(old);
        let newest = self.reader_newest().await?;
        let db = self.db_stats().await?;
        self.out(
            "owner_transfer",
            json!({
                "accept_ms": accept_ms, "kept_by_new_owner": kept, "generations": gens, "bundles": wraps, "bundles_mb": mb(wrap_bytes), "hwm_mb_after": hwm.round(),
                "reader": { "gen_before": gen_before, "gen_after": gen_after, "before": before, "newest": newest, "pause_s": pause_s }, "db": db,
            }),
        )
        .await;
        if !kept || gen_after <= gen_before || pause_s.is_none() {
            return Err(bad(&format!("transfer: kept {kept}, the reader's generation {gen_before} → {gen_after}, epoch {before} → {newest}")));
        }
        Ok(())
    }
}

async fn newest_of(n: &Node, cid: &str, scope_id: &str) -> Result<u32> {
    Ok(n.svc.member(&n.keys).newest_key(cid, scope_id).await?.map_or(0, |k| k.epoch))
}

/// The leaf of each bundle of `nodes`, by generation.
async fn leaves_of(nodes: &[&Node], cid: &str) -> Result<Vec<(u32, u32)>> {
    let mut out = Vec::new();
    for n in nodes {
        for (_, bytes, _) in db::keys::bundles(n.svc.db(), cid, SCOPE_EVERYONE).await? {
            let b = keydist::Bundle::from_bytes(&bytes).map_err(|e| bad(&format!("bundle: {e}")))?;
            out.push((b.gen, b.leaf));
        }
    }
    Ok(out)
}

/// `MemHosts::keep` of the shards that hold one of `leaves` (as
/// `keydist::member::shard_of` finds them), and of every blob that is no
/// shard or is one of a generation none of them holds a leaf of (an
/// emergency on the way).
fn keep_shards_of(leaves: Vec<(u32, u32)>) -> crate::testkit::KeepFn {
    Arc::new(move |bytes: &[u8]| match keydist::shard::parse_header(bytes) {
        Ok(h) => {
            let mut of_gen = leaves.iter().filter(|(g, _)| *g == h.gen).peekable();
            of_gen.peek().is_none() || of_gen.any(|(_, leaf)| leaf / h.leaves_per_shard.max(1) == h.shard_no)
        }
        Err(_) => true,
    })
}

async fn keys_held(n: &Node, cid: &str) -> Result<usize> {
    Ok(db::keys::epochs(n.svc.db(), cid, SCOPE_EVERYONE).await?.iter().filter(|r| r.key_hex.is_some()).count())
}

/// The machine: CPUs, memory, the disk of the data folder.
fn machine() -> Value {
    let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0);
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let model = cpuinfo.lines().find(|l| l.starts_with("model name")).and_then(|l| l.split(':').nth(1)).map(|s| s.trim().to_string()).unwrap_or_default();
    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let total = meminfo.lines().find(|l| l.starts_with("MemTotal:")).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) / 1024;
    json!({ "cpus": cpus, "model": model, "mem_mb": total })
}

impl Bench {
    async fn phase(&mut self, name: &str) -> Result<()> {
        let n = self.args.n;
        match name {
            "snapshot" => self.snapshot().await,
            "epoch_one" => self.first_epoch().await,
            "restart" => self.restart().await,
            "catch_up_open" => self.catch_up("catch_up_open").await,
            // 100 bans an epoch, at most a tenth of the readers in the day.
            "epochs_day" => self.day(100, (n / 1000).clamp(1, 100)).await,
            "ban_wave" => self.ban_wave(2000.min(n / 5), 60).await,
            "outage" => self.outage(90, 5).await,
            "catch_up_from_join" => {
                self.set_history(History::FromJoin).await?;
                self.catch_up("catch_up_from_join").await?;
                self.set_history(History::Open).await
            }
            "levels" => self.levels().await,
            "migration" => self.migration().await,
            "recover" => self.recover().await,
            "keeper_handover" => self.keeper_handover().await,
            "owner_transfer" => self.owner_transfer().await,
            other => Err(bad(&format!("no phase {other}"))),
        }
    }

    /// The state after `after` saved, its time and size told.
    async fn saved(&mut self, after: &str) -> Result<()> {
        let (took, bytes) = self.save(after).await?;
        // The next phase counts its own CPU, not that of the save.
        self.cpu = cpu_s();
        let (rss, hwm) = mem_mb();
        let line = json!({ "state": "saved", "after": after, "save_ms": ms(took), "state_mb": mb(usize::try_from(bytes).unwrap_or(usize::MAX)), "rss_mb": rss.round(), "hwm_mb": hwm.round(), "t_s": self.started.elapsed().as_secs() });
        let mut o = std::io::stdout().lock();
        let _ = writeln!(o, "{line}");
        let _ = o.flush();
        Ok(())
    }
}

/// Runs the joins and the phases asked for, in order, for one N (or goes
/// on from a saved state), the state saved after each.
pub async fn run(args: &[String]) -> Result<()> {
    let args = Args::parse(args)?;
    let (mut b, after) = match &args.resume {
        Some(from) => {
            let t = Instant::now();
            let (b, after) = Bench::load(args.clone(), from).await?;
            let load_ms = ms(t.elapsed());
            // The keeper is loaded with the state, as it was when it was
            // saved: the first phase does not pay its load.
            let (rss, hwm) = mem_mb();
            let line = json!({ "state": "loaded", "after": after, "load_ms": load_ms, "rss_mb": rss.round(), "hwm_mb": hwm.round() });
            println!("{line}");
            (b, after)
        }
        None => {
            let mut b = Bench::setup(args.clone()).await?;
            b.joins().await?;
            b.saved(JOINS).await?;
            (b, JOINS.to_string())
        }
    };
    let done = if after == JOINS { None } else { Some(phase_at(&after)?) };
    if let (Some(d), Some(f)) = (done, args.from.as_deref()) {
        if phase_at(f)? <= d {
            return Err(bad(&format!("--from {f}: the state is saved after {after}")));
        }
    }
    for (i, name) in PHASES.iter().enumerate() {
        if done.is_some_and(|d| i <= d) || !args.wants(i) {
            continue;
        }
        b.phase(name).await?;
        // After the last there is nothing to go on with.
        if i + 1 < PHASES.len() {
            b.saved(name).await?;
        }
    }
    let errors = b.owner.errors().len() + b.reader.errors().len();
    let reader_errors: Vec<String> = b.reader.errors().into_iter().take(5).collect();
    b.out("done", json!({ "errors": errors, "reader_errors": reader_errors })).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &str) -> Args {
        Args::parse(&a.split_whitespace().map(String::from).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn the_phases_asked_for() {
        let all = args("--n 10");
        assert!((0..PHASES.len()).all(|i| all.wants(i)));
        let two = args("--phases levels,outage");
        let asked: Vec<&str> = (0..PHASES.len()).filter(|i| two.wants(*i)).map(|i| PHASES[i]).collect();
        assert_eq!(asked, ["outage", "levels"]);
        let from = args("--from recover --no-from-join");
        let asked: Vec<&str> = (0..PHASES.len()).filter(|i| from.wants(*i)).map(|i| PHASES[i]).collect();
        assert_eq!(asked, ["recover", "keeper_handover", "owner_transfer"]);
        let no_join = args("--no-from-join");
        assert!(!no_join.wants(phase_at("catch_up_from_join").unwrap()));
        // The state is read in the folder of `--resume` unless `--dir` says.
        assert_eq!(args("--resume r").dir, PathBuf::from("r"));
        assert_eq!(args("--resume r --dir d").dir, PathBuf::from("d"));
        for bad in ["--phases nope", "--from joins", "--phases levels --from levels"] {
            assert!(Args::parse(&bad.split_whitespace().map(String::from).collect::<Vec<_>>()).is_err(), "{bad}");
        }
    }

    /// Saved after the joins and loaded: the keeper, the reader and the
    /// fakes as they were, and the reader opens the next epoch.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_state_saved_is_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let a = args(&format!("--n 30 --days 1 --catch-up 2 --dir {}", dir.path().display()));
        let mut b = Bench::setup(a.clone()).await.unwrap();
        b.joins().await.unwrap();
        b.first_epoch().await.unwrap();
        let before = (b.reader_newest().await.unwrap(), b.head().await.unwrap(), b.w.clock.now(), b.c().puts, b.epochs_on_relay(), b.next_out);
        let memory = keeper_memory(&b).await;
        b.save("epoch_one").await.unwrap();
        drop(b);
        let resumed = Args { resume: Some(dir.path().into()), ..a.clone() };
        let (mut b, after) = Bench::load(resumed, dir.path()).await.unwrap();
        assert_eq!(after, "epoch_one");
        assert!(b.resumed);
        assert_eq!((b.reader_newest().await.unwrap(), b.head().await.unwrap(), b.w.clock.now(), b.c().puts, b.epochs_on_relay(), b.next_out), before);
        // What the keeper keeps in memory, as the run left it (not that of
        // a keeper loaded anew: every reader in step again, no join counted).
        assert_eq!(keeper_memory(&b).await, memory);
        b.first_epoch().await.unwrap();
        assert!(b.reader_newest().await.unwrap() > before.0);
        // Another shape is not this state.
        let other = Args { n: 31, resume: Some(dir.path().into()), ..a };
        assert!(Bench::load(other, dir.path()).await.is_err());
    }

    /// What `RunMemory` holds that a reload would lose, the sets aside.
    async fn keeper_memory(b: &Bench) -> Value {
        let mut v = serde_json::to_value(b.keeper().await.unwrap().lock().await.run_memory()).unwrap();
        for unordered in ["ops_done", "not_let_in_told"] {
            v.as_object_mut().unwrap().remove(unordered);
        }
        assert!(!v["recent_joins"].as_array().unwrap().is_empty());
        v
    }

    /// Killed between the two renames, then again while the next state is
    /// written: the state before is still there to go on from.
    #[tokio::test(flavor = "multi_thread")]
    async fn killed_twice_keeps_a_state() {
        let dir = tempfile::tempdir().unwrap();
        let a = args(&format!("--n 20 --days 1 --catch-up 2 --dir {}", dir.path().display()));
        let mut b = Bench::setup(a.clone()).await.unwrap();
        b.joins().await.unwrap();
        b.save(JOINS).await.unwrap();
        let d = dir.path();
        // The first kill: `state` gone to `state.old`, the new one not in place.
        std::fs::rename(d.join("state"), d.join("state.old")).unwrap();
        std::fs::create_dir_all(d.join("state.tmp")).unwrap();
        // The next save begins so, and is killed while `state.tmp` is written.
        state::put_back(d).unwrap();
        assert!(d.join("state/meta.json").is_file() && !d.join("state.old").exists());
        b.first_epoch().await.unwrap();
        b.save("epoch_one").await.unwrap();
        assert!(!d.join("state.old").exists() && !d.join("state.tmp").exists());
        drop(b);
        let (_, after) = Bench::load(Args { resume: Some(d.into()), ..a.clone() }, d).await.unwrap();
        assert_eq!(after, "epoch_one");
    }
}
