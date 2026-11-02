// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The state of the bench after a phase, in `D/state` (ROO-522): a run cut
//! by its night goes on the next one from the phase it was in, on the
//! state after the one before. What a phase reads lives in the keeper's
//! database (a file), in the fakes (the relay, the hosts, the clock: in
//! memory) and in the real readers (databases in memory, a cursor on the
//! relay each); all of it is written here and read back.
//!
//! Written to `state.tmp`, then put in the place of `state` (the one before
//! `state.old` meanwhile): a run killed while it saves keeps the state
//! before. A `state.old` alone (killed between the two renames) goes back
//! to `state` before the next is written: there is always one whole.

use super::{bad, Bench};
use crate::db::{self, Db};
use crate::testkit::{Cursor, MemSecrets, Node, World};
use messenger_core::Result;
use nostr::key::Keys;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const DIR: &str = "state";
const VERSION: u32 = 2;

/// What makes the world of a run: a state is resumed with the same.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Shape {
    pub n: u64,
    pub days: Vec<u32>,
    pub catch_up: u32,
    pub tick_every: u64,
    pub no_paths: bool,
    pub churn: bool,
    pub from_join: bool,
}

impl Shape {
    pub fn of(a: &super::Args) -> Self {
        Shape { n: a.n, days: a.days.clone(), catch_up: a.catch_up, tick_every: a.tick_every, no_paths: a.no_paths, churn: a.churn, from_join: a.from_join }
    }
}

#[derive(Serialize, Deserialize)]
struct NetCounts {
    sent: usize,
    wraps: usize,
    wrap_bytes: usize,
    wrap_max: usize,
    scoped: usize,
    scoped_bytes: usize,
}

#[derive(Serialize, Deserialize)]
struct NodeState {
    secret_hex: String,
    /// The file of its database in `state`.
    db: String,
    secrets: HashMap<String, String>,
    cursor: Cursor,
    net: NetCounts,
}

#[derive(Serialize, Deserialize)]
struct HostCounts {
    puts: usize,
    put_bytes: usize,
    gets: usize,
    get_bytes: usize,
    deletes: usize,
}

#[derive(Serialize, Deserialize)]
struct Meta {
    version: u32,
    /// The last phase done.
    after: String,
    shape: Shape,
    clock: i64,
    cid: String,
    /// The name of the keeper's database in `D` (it moves with a handover
    /// and a transfer).
    owner_db: String,
    /// What the keeper keeps in memory only (`Keeper::run_memory`).
    keeper_memory: crate::keeper::RunMemory,
    owner: NodeState,
    reader: NodeState,
    away: Vec<NodeState>,
    next_out: u64,
    next_in: u64,
    leveled: u64,
    t_s: u64,
    keep_inbox: Option<BTreeSet<String>>,
    hosts: HostCounts,
    down: BTreeSet<String>,
    no_paths: BTreeSet<String>,
}

fn io(what: &str, e: impl std::fmt::Display) -> messenger_core::MessengerError {
    bad(&format!("state: {what}: {e}"))
}

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A database and the files of its journal.
pub(super) fn db_files(p: &Path) -> [PathBuf; 3] {
    [p.to_path_buf(), PathBuf::from(format!("{}-wal", p.display())), PathBuf::from(format!("{}-shm", p.display()))]
}

/// Every saved state of `dir` and what is left of one being written.
pub(super) fn remove_all(dir: &Path) {
    for d in [DIR, "state.tmp", "state.old"] {
        let _ = std::fs::remove_dir_all(dir.join(d));
    }
}

/// The state saved in `dir`: `state`, or `state.old` when a run was
/// killed between the two renames.
fn found(dir: &Path) -> Result<PathBuf> {
    [DIR, "state.old"].iter().map(|d| dir.join(d)).find(|d| d.join("meta.json").is_file()).ok_or_else(|| bad(&format!("no state in {}", dir.display())))
}

/// The only whole state, `state.old` of a run killed between the renames,
/// back in the place of `state`: the save after removes no state before
/// the new one is in place.
pub(super) fn put_back(dir: &Path) -> Result<()> {
    let (cur, old) = (dir.join(DIR), dir.join("state.old"));
    if !cur.join("meta.json").is_file() && old.join("meta.json").is_file() {
        let _ = std::fs::remove_dir_all(&cur);
        std::fs::rename(&old, &cur).map_err(|e| io("rename", e))?;
    }
    Ok(())
}

/// A database, whole, into `to`. The keeper's file: its journal into the
/// file, then a copy (a `VACUUM INTO` of 15 GB rebuilds every index), or
/// `VACUUM INTO` while the journal is still read. One in memory: its rows
/// into a file of the same migrations (its `VACUUM INTO` writes into a
/// database of the memory VFS, not on disk).
async fn copy_db(db: &Db, file: Option<&Path>, to: &Path) -> Result<()> {
    match file {
        Some(file) => {
            let (busy, _, _): (i64, i64, i64) = sqlx::query_as("PRAGMA wal_checkpoint(TRUNCATE)").fetch_one(db.pool()).await.map_err(|e| io("checkpoint", e))?;
            if busy == 0 {
                std::fs::copy(file, to).map_err(|e| io("copy of the database", e))?;
            } else {
                sqlx::query("VACUUM INTO ?").bind(to.display().to_string()).execute(db.pool()).await.map_err(|e| io("vacuum into", e))?;
            }
        }
        None => {
            Db::open(to).await?.pool().close().await;
            rows(db, to, false).await?;
        }
    }
    Ok(())
}

/// The rows of every table between `db` and the file `other` (made by the
/// same migrations): from the file into `db` (`into_db`), or out of it.
async fn rows(db: &Db, other: &Path, into_db: bool) -> Result<()> {
    let (from, to) = if into_db { ("other", "main") } else { ("main", "other") };
    let mut c = db.pool().acquire().await.map_err(|e| io("connection", e))?;
    sqlx::query("PRAGMA foreign_keys = OFF").execute(&mut *c).await.map_err(|e| io("pragma", e))?;
    // A URI with its mode: the flags of a connection to a database in
    // memory (`SQLITE_OPEN_MEMORY`) would make the one attached another.
    let abs = std::path::absolute(other).map_err(|e| io("path", e))?;
    let uri = format!("file:{}?mode=rwc", abs.display().to_string().replace('%', "%25").replace('?', "%3f").replace('#', "%23"));
    sqlx::query("ATTACH DATABASE ? AS other").bind(uri).execute(&mut *c).await.map_err(|e| io("attach", e))?;
    let attached = "other";
    let tables: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT name FROM {attached}.sqlite_master WHERE type = 'table' AND name <> '_sqlx_migrations'"))).fetch_all(&mut *c).await.map_err(|e| io("tables", e))?;
    for t in tables {
        let t = t.replace('"', "\"\"");
        // The names are of the tables of our own file, quoted.
        sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {to}.\"{t}\""))).execute(&mut *c).await.map_err(|e| io(&t, e))?;
        sqlx::query(sqlx::AssertSqlSafe(format!("INSERT INTO {to}.\"{t}\" SELECT * FROM {from}.\"{t}\""))).execute(&mut *c).await.map_err(|e| io(&t, e))?;
    }
    sqlx::query("DETACH DATABASE other").execute(&mut *c).await.map_err(|e| io("detach", e))?;
    sqlx::query("PRAGMA foreign_keys = ON").execute(&mut *c).await.map_err(|e| io("pragma", e))?;
    Ok(())
}

fn net_counts(n: &Node) -> NetCounts {
    let l = |a: &AtomicUsize| a.load(Ordering::SeqCst);
    let n = &n.net;
    NetCounts { sent: l(&n.sent), wraps: l(&n.wraps), wrap_bytes: l(&n.wrap_bytes), wrap_max: l(&n.wrap_max), scoped: l(&n.scoped), scoped_bytes: l(&n.scoped_bytes) }
}

async fn save_node(n: &Node, file: Option<&Path>, dir: &Path, name: &str) -> Result<NodeState> {
    copy_db(n.svc.db(), file, &dir.join(name)).await?;
    Ok(NodeState {
        secret_hex: n.keys.secret_key().to_secret_hex(),
        db: name.into(),
        secrets: n.secrets.dump().into_iter().map(|(k, v)| (k, hex::encode(v))).collect(),
        cursor: n.cursor(),
        net: net_counts(n),
    })
}

/// A device of `s` on `db`, as it was saved.
fn load_node(w: &World, s: &NodeState, db: Db) -> Result<Node> {
    let keys = Keys::parse(&s.secret_hex).map_err(|e| io("key", e))?;
    let secrets = MemSecrets::default();
    secrets.load(s.secrets.iter().map(|(k, v)| hex::decode(v).map(|v| (k.clone(), v))).collect::<std::result::Result<_, _>>().map_err(|e| io("secrets", e))?);
    let n = w.device_with(keys, db, Arc::new(secrets));
    n.set_cursor(s.cursor.clone());
    let st = |a: &AtomicUsize, v: usize| a.store(v, Ordering::SeqCst);
    st(&n.net.sent, s.net.sent);
    st(&n.net.wraps, s.net.wraps);
    st(&n.net.wrap_bytes, s.net.wrap_bytes);
    st(&n.net.wrap_max, s.net.wrap_max);
    st(&n.net.scoped, s.net.scoped);
    st(&n.net.scoped_bytes, s.net.scoped_bytes);
    Ok(n)
}

async fn load_reader(w: &World, s: &NodeState, state: &Path) -> Result<Node> {
    let db = Db::open_in_memory().await?;
    rows(&db, &state.join(&s.db), true).await?;
    load_node(w, s, db)
}

fn create(p: &Path) -> Result<BufWriter<std::fs::File>> {
    Ok(BufWriter::with_capacity(1 << 20, std::fs::File::create(p).map_err(|e| io(&p.display().to_string(), e))?))
}

fn open(p: &Path) -> Result<BufReader<std::fs::File>> {
    Ok(BufReader::with_capacity(1 << 20, std::fs::File::open(p).map_err(|e| io(&p.display().to_string(), e))?))
}

/// The blobs of the hosts: per blob the host, the hash and the bytes, each
/// after its length.
fn write_blobs(w: &World, p: &Path) -> Result<()> {
    let mut f = create(p)?;
    let blobs = lock(&w.hosts.blobs);
    let e = |e| io("blobs", e);
    for (host, m) in blobs.iter() {
        for (sha, bytes) in m {
            for part in [host.as_bytes(), sha.as_bytes(), bytes.as_slice()] {
                f.write_all(&(part.len() as u64).to_le_bytes()).map_err(e)?;
                f.write_all(part).map_err(e)?;
            }
        }
    }
    f.flush().map_err(e)
}

fn read_blobs(w: &World, p: &Path) -> Result<()> {
    let mut f = open(p)?;
    let mut blobs = lock(&w.hosts.blobs);
    let e = |e| io("blobs", e);
    let part = |f: &mut BufReader<std::fs::File>| -> Result<Option<Vec<u8>>> {
        let mut len = [0u8; 8];
        match f.read_exact(&mut len) {
            Ok(()) => {}
            Err(x) if x.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(x) => return Err(e(x)),
        }
        let mut v = vec![0u8; usize::try_from(u64::from_le_bytes(len)).map_err(|x| io("blobs", x))?];
        f.read_exact(&mut v).map_err(e)?;
        Ok(Some(v))
    };
    while let Some(host) = part(&mut f)? {
        let (Some(sha), Some(bytes)) = (part(&mut f)?, part(&mut f)?) else { return Err(bad("state: blobs: cut short")) };
        let text = |v: Vec<u8>| String::from_utf8(v).map_err(|x| io("blobs", x));
        blobs.entry(text(host)?).or_default().insert(text(sha)?, bytes);
    }
    Ok(())
}

fn write_json(p: &Path, v: &impl Serialize) -> Result<()> {
    let mut f = create(p)?;
    serde_json::to_writer(&mut f, v).map_err(|e| io("json", e))?;
    f.flush().map_err(|e| io("json", e))
}

fn read_json<T: for<'de> Deserialize<'de>>(p: &Path) -> Result<T> {
    serde_json::from_reader(open(p)?).map_err(|e| io(&p.display().to_string(), e))
}

fn size_of(dir: &Path) -> u64 {
    std::fs::read_dir(dir).map(|d| d.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum()).unwrap_or(0)
}

impl Bench {
    /// The state after `after`, in `D/state` in the place of the one before;
    /// how long it took and its bytes.
    pub(super) async fn save(&self, after: &str) -> Result<(std::time::Duration, u64)> {
        let t = std::time::Instant::now();
        let dir = &self.args.dir;
        put_back(dir)?;
        let (tmp, cur, old) = (dir.join("state.tmp"), dir.join(DIR), dir.join("state.old"));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).map_err(|e| io("state.tmp", e))?;
        let owner = save_node(&self.owner, Some(&self.db_path), &tmp, "owner.sqlite").await?;
        let reader = save_node(&self.reader, None, &tmp, "reader.sqlite").await?;
        let mut away = Vec::new();
        for (i, r) in self.away.iter().enumerate() {
            away.push(save_node(r, None, &tmp, &format!("away-{i}.sqlite")).await?);
        }
        {
            let mut f = create(&tmp.join("events.jsonl"))?;
            for ev in lock(&self.w.relay.events).iter() {
                serde_json::to_writer(&mut f, ev).map_err(|e| io("events", e))?;
                f.write_all(b"\n").map_err(|e| io("events", e))?;
            }
            f.flush().map_err(|e| io("events", e))?;
        }
        write_json(&tmp.join("inbox.json"), &*lock(&self.w.relay.inbox))?;
        write_json(&tmp.join("paths.json"), &*lock(&self.w.hosts.paths))?;
        write_blobs(&self.w, &tmp.join("blobs.bin"))?;
        let keeper_memory = self.keeper().await?.lock().await.run_memory();
        let h = &self.w.hosts;
        let l = |a: &AtomicUsize| a.load(Ordering::SeqCst);
        let meta = Meta {
            keeper_memory,
            version: VERSION,
            after: after.into(),
            shape: Shape::of(&self.args),
            clock: self.w.clock.now(),
            cid: self.cid.clone(),
            owner_db: self.db_path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| db::FILE_NAME.into()),
            owner,
            reader,
            away,
            next_out: self.next_out,
            next_in: self.next_in,
            leveled: self.leveled,
            t_s: self.started.elapsed().as_secs(),
            keep_inbox: lock(&self.w.relay.keep_inbox).clone(),
            hosts: HostCounts { puts: l(&h.puts), put_bytes: l(&h.put_bytes), gets: l(&h.gets), get_bytes: l(&h.get_bytes), deletes: l(&h.deletes) },
            down: lock(&h.down).clone(),
            no_paths: lock(&h.no_paths).clone(),
        };
        // The meta last: a state without it is none.
        write_json(&tmp.join("meta.json"), &meta)?;
        let bytes = size_of(&tmp);
        let _ = std::fs::remove_dir_all(&old);
        if cur.exists() {
            std::fs::rename(&cur, &old).map_err(|e| io("rename", e))?;
        }
        std::fs::rename(&tmp, &cur).map_err(|e| io("rename", e))?;
        let _ = std::fs::remove_dir_all(&old);
        Ok((t.elapsed(), bytes))
    }

    /// The bench as `from` saved it, run on in `args.dir`: the keeper's
    /// database copied there, the rest in memory again.
    pub(super) async fn load(args: super::Args, from: &Path) -> Result<(Self, String)> {
        let state = found(from)?;
        let m: Meta = read_json(&state.join("meta.json"))?;
        if m.version != VERSION {
            return Err(bad(&format!("state: version {}, this bench reads {VERSION}", m.version)));
        }
        if m.shape != Shape::of(&args) {
            return Err(bad(&format!("state: saved with {:?}, asked {:?}", m.shape, Shape::of(&args))));
        }
        std::fs::create_dir_all(&args.dir).map_err(|e| io("dir", e))?;
        let w = World::new();
        w.clock.set(m.clock);
        {
            let mut events = lock(&w.relay.events);
            for line in open(&state.join("events.jsonl"))?.lines() {
                let line = line.map_err(|e| io("events", e))?;
                if !line.is_empty() {
                    events.push(serde_json::from_str(&line).map_err(|e| io("events", e))?);
                }
            }
        }
        *lock(&w.relay.inbox) = read_json(&state.join("inbox.json"))?;
        *lock(&w.relay.keep_inbox) = m.keep_inbox.clone();
        read_blobs(&w, &state.join("blobs.bin"))?;
        let paths: HashMap<String, BTreeMap<String, String>> = read_json(&state.join("paths.json"))?;
        w.hosts.set_all_paths(paths);
        let h = &w.hosts;
        let st = |a: &AtomicUsize, v: usize| a.store(v, Ordering::SeqCst);
        st(&h.puts, m.hosts.puts);
        st(&h.put_bytes, m.hosts.put_bytes);
        st(&h.gets, m.hosts.gets);
        st(&h.get_bytes, m.hosts.get_bytes);
        st(&h.deletes, m.hosts.deletes);
        *lock(&h.down) = m.down.clone();
        *lock(&h.no_paths) = m.no_paths.clone();
        // The keeper's database where the run left it; the files of the
        // devices a cut phase made after it are not of this state.
        let db_path = args.dir.join(&m.owner_db);
        for name in [db::FILE_NAME, "owner-b.sqlite", "owner-m.sqlite"] {
            for p in db_files(&args.dir.join(name)) {
                let _ = std::fs::remove_file(p);
            }
        }
        std::fs::copy(state.join(&m.owner.db), &db_path).map_err(|e| io("copy of the database", e))?;
        let owner = load_node(&w, &m.owner, Db::open(&db_path).await?)?;
        let reader = load_reader(&w, &m.reader, &state).await?;
        let mut away = Vec::new();
        for s in &m.away {
            away.push(load_reader(&w, s, &state).await?);
        }
        let started = std::time::Instant::now().checked_sub(std::time::Duration::from_secs(m.t_s)).unwrap_or_else(std::time::Instant::now);
        let mut b = Bench {
            args,
            w,
            owner,
            reader,
            away,
            cid: m.cid,
            db_path,
            next_out: m.next_out,
            next_in: m.next_in,
            leveled: m.leveled,
            started,
            cpu: (0.0, 0.0),
            resumed: true,
        };
        b.keeper().await?.lock().await.set_run_memory(m.keeper_memory);
        // The first phase counts its own CPU, not that of the load.
        b.cpu = super::cpu_s();
        Ok((b, m.after))
    }
}
