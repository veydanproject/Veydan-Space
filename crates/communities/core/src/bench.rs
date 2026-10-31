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
//! communities-bench --n N --dir D [--no-from-join] [--catch-up E]
//! ```
//!
//! Every number is measured once and printed as a JSON line
//! (`{"phase": …, …}`); the lines are also written to `D/results.jsonl`.
//! The readers past the first are made here and let in by
//! `Keeper::on_join` with a proof of the link, as a join note would after
//! the queue; one real reader (a `Node`) joins by the link through the
//! service and reads the epochs. The keeper's own limits (65 536 readers,
//! 120 joins a minute) are as in the app, except `max_readers`, lifted to
//! go past one snapshot (reported as such).

use crate::db::{self, Db};
use crate::keeper::{Joined, Keeper};
use crate::log::OpLog;
use crate::op::{History, JoinMode, OpBody};
use crate::service::NewChannel;
use crate::snapshot::{self as snap, MAX_SHARDS, SHARD_ENTRIES};
use crate::state::SCOPE_EVERYONE;
use crate::testkit::{Node, World, H1, H2};
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
}

impl Args {
    pub fn parse(args: &[String]) -> Result<Self> {
        let mut a = Args { n: 10_000, dir: PathBuf::from("bench-out"), from_join: true, catch_up: 288 };
        let mut it = args.iter();
        while let Some(x) = it.next() {
            let mut val = || it.next().cloned().ok_or_else(|| bad(&format!("{x} wants a value")));
            match x.as_str() {
                "--n" => a.n = val()?.replace('_', "").parse().map_err(|_| bad("--n"))?,
                "--dir" => a.dir = PathBuf::from(val()?),
                "--no-from-join" => a.from_join = false,
                "--catch-up" => a.catch_up = val()?.parse().map_err(|_| bad("--catch-up"))?,
                other => return Err(bad(&format!("unknown argument {other}"))),
            }
        }
        Ok(a)
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

struct Bench {
    args: Args,
    w: World,
    owner: Node,
    reader: Node,
    cid: String,
    db_path: PathBuf,
    /// The next synthetic reader to put out (bans and leaves take them in
    /// order; the owner and the real reader are never put out).
    next_out: u64,
    started: Instant,
}

impl Bench {
    fn out(&mut self, phase: &str, mut v: Value) {
        let (rss, hwm) = mem_mb();
        if let Some(o) = v.as_object_mut() {
            o.insert("phase".into(), json!(phase));
            o.insert("n".into(), json!(self.args.n));
            o.insert("rss_mb".into(), json!(rss.round()));
            o.insert("hwm_mb".into(), json!(hwm.round()));
            o.insert("t_s".into(), json!(self.started.elapsed().as_secs()));
        }
        let mut o = std::io::stdout().lock();
        let _ = writeln!(o, "{v}");
        let _ = o.flush();
        // Line by line: a run cut short keeps what it measured.
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(self.args.dir.join("results.jsonl")) {
            let _ = writeln!(f, "{v}");
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

    async fn keeper(&self) -> Result<Arc<Mutex<Keeper>>> {
        self.owner.svc.keeper(&self.cid).await?.ok_or_else(|| bad("not kept here"))
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
        count!("cm_keeper_members", "cm_keeper_revoked", "cm_keeper_readers", "cm_keeper_ops", "cm_keeper_retired", "cm_keeper_uploads", "cm_keeper_missed", "cm_keeper_blobs", "cm_ops", "cm_epochs");
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
        Ok(self.reader.svc.member(&self.reader.keys).newest_key(&self.cid, SCOPE_EVERYONE).await?.map_or(0, |k| k.epoch))
    }

    async fn reader_keys_held(&self) -> Result<usize> {
        Ok(db::keys::epochs(self.reader.svc.db(), &self.cid, SCOPE_EVERYONE).await?.iter().filter(|r| r.key_hex.is_some()).count())
    }

    // ─── 0. The channel, the owner and one real reader ──────────────────────

    async fn setup(args: Args) -> Result<Self> {
        std::fs::create_dir_all(&args.dir).map_err(|e| bad(&e.to_string()))?;
        let db_path = args.dir.join(db::FILE_NAME);
        for p in [args.dir.join("results.jsonl"), db_path.clone(), PathBuf::from(format!("{}-wal", db_path.display())), PathBuf::from(format!("{}-shm", db_path.display()))] {
            let _ = std::fs::remove_file(p);
        }
        let w = World::new();
        let owner_keys = Keys::generate();
        let reader_keys = Keys::generate();
        // Only the owner's (the keeper's inbox) and the real reader's wraps
        // are kept; a million issues are counted and dropped.
        if let Ok(mut k) = w.relay.keep_inbox.lock() {
            *k = Some([owner_keys.public_key().to_hex(), reader_keys.public_key().to_hex()].into_iter().collect());
        }
        let owner = w.device_on(owner_keys, Db::open(&db_path).await?);
        let reader = w.device(reader_keys).await;
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
        w.pump(&[&reader]).await;
        reader.svc.open_link(&reader.keys, &link, "").await?;
        w.pump(&[&owner, &reader]).await;
        let mut b = Bench { args, w, owner, reader, cid, db_path, next_out: 0, started: Instant::now() };
        let newest = b.reader_newest().await?;
        if newest == 0 {
            return Err(bad("the real reader did not get its key"));
        }
        let machine = machine();
        b.out("setup", json!({ "reader_epoch": newest, "machine": machine, "errors": b.owner.errors().len() + b.reader.errors().len() }));
        Ok(b)
    }

    // ─── 1. Joins ───────────────────────────────────────────────────────────

    async fn joins(&mut self) -> Result<()> {
        let n = self.args.n;
        let log = self.log().await?;
        let link_epoch = log.state().link_epoch;
        let secret = crate::secrets::links(self.owner.svc.env.secrets.as_ref(), &self.cid).await?.get(&link_epoch).cloned().ok_or_else(|| bad("no link secret"))?;
        let k = self.keeper().await?;
        {
            let mut g = k.lock().await;
            // The app refuses the 65 537th reader (`MAX_READERS`: what one
            // snapshot names); lifted here to measure past it.
            g.limits.max_readers = usize::MAX;
        }
        let batch: u64 = 10_000.min(n);
        let c0 = self.c();
        let t0 = Instant::now();
        let mut busy = 0u64;
        let mut joins_ms = 0.0;
        let mut ticks_ms = 0.0;
        let mut i = 0u64;
        while i < n {
            let tb = Instant::now();
            {
                let mut g = k.lock().await;
                let end = (i + batch).min(n);
                while i < end {
                    let keys = reader_keys(i)?;
                    let pk = PubKey::parse(&keys.public_key().to_hex()).ok_or_else(|| bad("pubkey"))?;
                    loop {
                        let now = self.w.clock.now();
                        let join = Join {
                            community_id: self.cid.clone(),
                            proof: Some(JoinProof { link_epoch, mac: secret.join_mac(&self.cid, &pk, link_epoch) }),
                            admit_id: None,
                            created_at: now,
                        };
                        match g.on_join(&log, &pk, &join, now).await? {
                            Joined::Issued => break,
                            // 120 a minute: the next minute.
                            Joined::Busy => {
                                busy += 1;
                                self.w.clock.advance(60);
                            }
                            other => return Err(bad(&format!("join {i}: {other:?}"))),
                        }
                    }
                    i += 1;
                }
            }
            let joined = tb.elapsed();
            joins_ms += ms(joined);
            // A turn of the keeper after each batch, as the app's loop
            // would make (the snapshot of the readers is part of it).
            let tt = Instant::now();
            let turn = {
                let mut g = k.lock().await;
                let now = self.w.clock.now();
                let mut t = self.owner.svc.keeper_tick(&mut g, &log, now).await?;
                drop(g);
                // The jobs of the hosts run off the keeper, as the module runs them.
                let jobs = self.owner.svc.take_jobs(std::mem::take(&mut t.jobs));
                self.owner.svc.run_jobs(jobs).await;
                t.snapshot
            };
            let ticked = tt.elapsed();
            ticks_ms += ms(ticked);
            if i.is_multiple_of(100_000) || i == n || i <= 20_000 {
                self.out("joins_batch", json!({ "joined": i, "batch_ms": ms(joined), "per_join_us": (joined.as_secs_f64() * 1e6 / batch as f64).round(), "tick_ms": ms(ticked), "snapshot": format!("{turn:?}") }));
            }
        }
        let c = self.c().since(c0);
        let days = (self.w.clock.now() - crate::testkit::T0) as f64 / 86_400.0;
        let db = self.db_stats().await?;
        self.out(
            "joins",
            json!({
                "joins": n, "wall_s": t0.elapsed().as_secs(), "joins_ms": joins_ms.round(), "ticks_ms": ticks_ms.round(),
                "per_10k_s": (joins_ms / (n as f64 / 10_000.0) / 1000.0 * 100.0).round() / 100.0,
                "busy_minutes": busy, "fake_days": (days * 100.0).round() / 100.0,
                "issue_avg_bytes": c.wrap_bytes.checked_div(c.wraps).unwrap_or(0), "net": c.json(), "db": db, "fakes": self.fakes_mb(),
            }),
        );
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
        );
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
    /// published; the real reader opens it.
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
        self.w.clock.advance(1);
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
        let db = self.db_stats().await?;
        self.out(
            "epoch_one",
            json!({
                "bans": bans, "ban_acts_ms": ban_ms, "per_ban_ms": (ban_ms / bans as f64 * 100.0).round() / 100.0, "reader_ops_ms": reader_ops_ms,
                "build_upload_ms": build_ms, "publish_tick_ms": publish_ms, "built": built,
                "upload": c1.since(c0).json(), "publish": c2.since(c1).json(),
                "reader": { "ms": read_ms, "gets": r.gets, "get_kb": (r.get_bytes as f64 / 1024.0).round(), "newest": newest },
                "db": db,
            }),
        );
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
        for _ in 0..epochs {
            let this = self.ban(bans).await?;
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
        self.out(
            "epochs_day",
            json!({
                "epochs": made, "bans_each": bans, "ban_acts_ms_avg": (ban_ms / (f64::from(epochs) * bans as f64) * 100.0).round() / 100.0,
                "ban_ms_first_batch": (batches.first().copied().unwrap_or(0.0) / bans as f64 * 100.0).round() / 100.0, "ban_ms_last_batch": (batches.last().copied().unwrap_or(0.0) / bans as f64 * 100.0).round() / 100.0,
                "epoch_tick_ms_avg": avg(&epoch_ms), "epoch_tick_ms_max": max(&epoch_ms),
                "put_mb_per_epoch_avg": mb(put_avg), "put_mb_per_epoch_last": mb(puts_per_epoch.last().copied().unwrap_or(0)),
                "epoch_blobs_kb_avg": epoch_avg / 1024, "snapshots": snaps, "snapshot_put_mb_avg": mb(c.put_bytes.saturating_sub(epoch_bytes) / snaps.max(1)),
                "epoch_blobs_gb_per_day_80": (epoch_avg as f64 * 80.0 / 1e9 * 1000.0).round() / 1000.0,
                "put_gb_per_day_80": (put_avg as f64 * 80.0 / 1e9 * 100.0).round() / 100.0, "put_gb_per_day_288": (put_avg as f64 * 288.0 / 1e9 * 100.0).round() / 100.0,
                "revoked": revoked, "net": c.json(),
                "reader": { "ops_ms": read_ops_ms.round(), "epoch_ms_avg": avg(&read_ms), "gets_per_epoch": gets as f64 / made.max(1) as f64, "get_kb_per_epoch": (get_bytes as f64 / made.max(1) as f64 / 1024.0).round(), "newest": newest },
                "db": db, "fakes": self.fakes_mb(),
            }),
        );
        Ok(())
    }

    // ─── 5. A reader one day offline ────────────────────────────────────────

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

    async fn catch_up(&mut self, label: &str) -> Result<()> {
        let epochs = self.args.catch_up;
        // Leaves of an open scope at 0.3 % of the readers a day, spread
        // over the epochs; each epoch asked for now (`rotate_now`).
        let leaves = (self.args.n * 3 / 1000 / u64::from(epochs)).max(1);
        self.w.pump(&[&self.reader]).await;
        self.reader.skip_all(&self.w.relay);
        let start = self.w.relay.events.lock().map(|e| e.len()).unwrap_or(0);
        let held_before = self.reader_keys_held().await?;
        let c0 = self.c();
        let t = Instant::now();
        for _ in 0..epochs {
            self.leave(leaves).await?;
            self.w.clock.advance(300);
            self.owner.svc.act(&self.owner.keys, &self.cid, OpBody::RotateNow { scope_id: None }).await?;
        }
        let keeper_ms = ms(t.elapsed());
        let kc = self.c().since(c0);
        let evs: Vec<Value> = self.w.relay.events.lock().map(|e| e[start..].iter().filter(|e| e["kind"].as_u64() == Some(u64::from(KIND_EPOCH))).cloned().collect()).unwrap_or_default();
        // The reader comes back: the catch-up hands the newest first.
        let r0 = self.c();
        let t = Instant::now();
        for ev in evs.iter().rev() {
            self.reader.svc.on_event(&self.reader.keys, ev, false).await?;
        }
        let read_ms = ms(t.elapsed());
        let r = self.c().since(r0);
        let held = self.reader_keys_held().await? - held_before;
        let event_bytes: usize = evs.iter().map(|e| serde_json::to_string(e).map_or(0, |s| s.len())).sum();
        // The rest of what came while away (the rotate_now of the journal,
        // the epochs again), not measured: a journal with holes would
        // hold every later operation of the reader back.
        self.w.pump(&[&self.reader]).await;
        self.out(
            label,
            json!({
                "epochs": evs.len(), "leaves_each": leaves, "keeper_ms": keeper_ms, "keeper": kc.json(),
                "reader": { "ms": read_ms, "keys_opened": held, "gets": r.gets, "get_mb": mb(r.get_bytes), "events": evs.len(), "events_kb": event_bytes / 1024 },
                "fakes": self.fakes_mb(),
            }),
        );
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
        self.w.pump(&[&self.reader]).await;
        let seen = self.reader.svc.log(&self.cid).await?.map(|l| l.state().history);
        if seen != Some(h) {
            return Err(bad(&format!("the reader holds history {seen:?}, not {h:?}")));
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
        {
            let mut g = k.lock().await;
            let now = self.w.clock.now();
            let t = self.owner.svc.keeper_tick(&mut g, &log, now).await?;
            drop(g);
            self.owner.svc.run_jobs(self.owner.svc.take_jobs(t.jobs)).await;
        }
        let tick_ms = ms(t.elapsed());
        // A second turn: what every turn after costs (nothing changed).
        let t = Instant::now();
        {
            let mut g = k.lock().await;
            let now = self.w.clock.now();
            let t = self.owner.svc.keeper_tick(&mut g, &log, now).await?;
            drop(g);
            self.owner.svc.run_jobs(self.owner.svc.take_jobs(t.jobs)).await;
        }
        let tick2_ms = ms(t.elapsed());
        let members = k.lock().await.scope(SCOPE_EVERYONE).map(|s| s.stats().members).unwrap_or(0);
        self.out("restart", json!({ "load_ms": load_ms, "rss_after_load_mb": rss_loaded.round(), "first_tick_ms": tick_ms, "next_tick_ms": tick2_ms, "members": members }));
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
        let c = self.c().since(c0);
        // The first epoch under two generations.
        self.w.clock.advance(300);
        self.owner.svc.act(&self.owner.keys, &self.cid, OpBody::RotateNow { scope_id: None }).await?;
        let e = self.c().since(c0).since(c);
        let t = Instant::now();
        self.w.pump(&[&self.reader]).await;
        let read_ms = ms(t.elapsed());
        let newest = self.reader_newest().await?;
        self.out(
            "migration",
            json!({
                "handed": handed, "revoked_before": revoked, "leaves": leaves, "leave_ms": leave_ms, "due_at_0_1": due_10, "due_by_policy": due_default,
                "migrate_ms": migrate_ms, "reissue_ms": reissue_ms, "reissue": c.json(), "issue_avg_bytes": c.wrap_bytes.checked_div(c.wraps).unwrap_or(0),
                "hwm_before_mb": hwm0.round(), "hwm_after_mb": hwm1.round(), "epoch_two_gens": e.json(), "reader": { "ms": read_ms, "newest": newest },
                "db": self.db_stats().await?,
            }),
        );
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
        let c = self.c().since(c0);
        let per = c.wrap_bytes.checked_div(c.wraps).unwrap_or(0);
        self.out(
            "recover",
            json!({
                "load_ms": load_ms, "gen": gen, "kind": kind, "members": members, "hwm_before_mb": hwm0.round(), "hwm_after_mb": hwm1.round(),
                "rebundles": c.wraps, "rebundle_ms": rebundle_ms, "per_rebundle_us": (rebundle_ms * 1000.0 / c.wraps.max(1) as f64).round(), "bundle_bytes": per,
                "all_members_gb": (per as f64 * members as f64 / 1e9 * 100.0).round() / 100.0,
            }),
        );
        Ok(())
    }
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

/// Runs every phase in order for one N.
pub async fn run(args: &[String]) -> Result<()> {
    let args = Args::parse(args)?;
    let mut b = Bench::setup(args.clone()).await?;
    b.joins().await?;
    b.snapshot().await?;
    b.first_epoch().await?;
    // What depends on N first, the journal of a day of bans (which does
    // not) after: a run cut by its time still says the most.
    b.restart().await?;
    b.catch_up("catch_up_open").await?;
    // 100 bans an epoch, at most a tenth of the readers in the day.
    b.day(100, (args.n / 1000).clamp(1, 100)).await?;
    if args.from_join {
        b.set_history(History::FromJoin).await?;
        b.catch_up("catch_up_from_join").await?;
        b.set_history(History::Open).await?;
    }
    b.migration().await?;
    b.recover().await?;
    let errors = b.owner.errors().len() + b.reader.errors().len();
    b.out("done", json!({ "errors": errors, "reader_errors": b.reader.errors().into_iter().take(5).collect::<Vec<_>>() }));
    Ok(())
}
