// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The handler of the archive over a vault in a folder, with an archive and
//! a runtime made up for the test: a segment is a list of event ids, one a
//! line; merging is their union; a replay hands every id to the replayer.
//! The fake keeps the rules of the real archive that the handler leans on:
//! a version read is held, but its item is done only when the handler says
//! so; what of a held version is not replayed yet is replayed from what is
//! held; a status changes for its version alone.

use super::archive::{self, pause_reason, MessengerArchive};
use super::*;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use messenger_archive::{ArchivedEvent, Month, Replayed, ENTITY_SECRET, ENTITY_SEGMENT, REFUSED_GROUP_UNKNOWN, REFUSED_SERVER_UNKNOWN};
use messenger_core::{EventId, EventSource, MessengerError, RawEvent};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU8, AtomicUsize};
use veydan_core::{db, Directory};
use veydan_lock::Lock;
use veydan_sync::{Engine, HlcClock, LocalDir, LocalState, Op, SyncError};
use veydan_sync_host::{Applied, Collected, Cycle, Handler, Plan, Step, SyncConfig, SyncManager};

/// The owner of every archive of the test.
fn owner() -> PubKey {
    PubKey::parse(&"ab".repeat(32)).unwrap()
}

fn other_owner() -> PubKey {
    PubKey::parse(&"cd".repeat(32)).unwrap()
}

// ── The archive and the runtime of the test ──────────────────────────────────

/// A row of the queue of the test.
#[derive(Clone, Debug)]
struct Queued {
    meta: SegmentMeta,
    status: &'static str,
    build: String,
}

#[derive(Default)]
struct Inner {
    /// What this device holds of each segment: event ids.
    local: BTreeMap<String, BTreeSet<String>>,
    /// The ids the runtime was handed already, by segment.
    replayed: BTreeMap<String, BTreeSet<String>>,
    due: Vec<SegmentId>,
    written: Vec<(String, String)>,
    /// The versions this device wrote or read, by the segment each is of.
    held: BTreeMap<String, String>,
    /// Files that came down (replayed or merged from bytes).
    downloads: u32,
    forgot: u32,
    deferred: Vec<String>,
    queue: BTreeMap<String, Queued>,
    removed: Vec<String>,
    /// The segments the monthly pass compacted, in order.
    compacted: Vec<String>,
    /// The private servers this device has.
    servers: BTreeSet<String>,
    /// The scopes forgotten and not yet taken by the handler (the marks of
    /// `remove_scope_in`).
    forgotten: Vec<String>,
    /// The groups the store has a row of.
    groups: BTreeSet<String>,
    /// The segments over the bounds of the format: `build` refuses them.
    too_big: BTreeSet<String>,
}

#[derive(Default)]
struct FakeStore(std::sync::Mutex<Inner>);

/// A file of the test: the segment's id on the first line, then the ids
/// of its events.
fn body(seg: &SegmentId, ids: &BTreeSet<String>) -> Vec<u8> {
    let mut out = format!("#{}\n", seg.entity_id());
    for id in ids {
        out.push_str(id);
        out.push('\n');
    }
    out.into_bytes()
}

fn read(bytes: &[u8]) -> (Option<SegmentId>, BTreeSet<String>) {
    let text = String::from_utf8_lossy(bytes);
    let seg = text.lines().next().and_then(|l| l.strip_prefix('#')).and_then(SegmentId::parse);
    (seg, text.lines().filter(|l| !l.starts_with('#')).map(str::to_string).collect())
}

fn built(seg: &SegmentId, ids: &BTreeSet<String>) -> Built {
    let bytes = body(seg, ids);
    let meta = SegmentMeta {
        owner: seg.owner.as_hex().to_string(),
        scope: seg.scope.as_key(),
        month: seg.month.to_string(),
        count: ids.len() as u64,
        first_at: seg.month.start().0,
        last_at: seg.month.start().0,
        size: bytes.len() as u64,
        hash: veydan_sync::sha256_hex(&bytes),
    };
    Built { bytes, meta }
}

fn event(id: &str, scope: ArchiveScope) -> ArchivedEvent {
    let raw = RawEvent {
        id: EventId::parse(id).expect("a test id is hex"),
        kind: 9,
        pubkey: PubKey::parse(&"ab".repeat(32)).expect("a test key is hex"),
        created_at: Timestamp(1_790_000_000),
        json: serde_json::json!({}),
        source: EventSource::Archive,
    };
    ArchivedEvent::wire(raw, scope, Timestamp(1_790_000_000))
}

impl FakeStore {
    fn hold(&self, seg: &SegmentId, ids: &[&str]) {
        let mut inner = self.0.lock().unwrap();
        inner.local.insert(seg.entity_id(), ids.iter().map(|s| s.to_string()).collect());
        if !inner.due.contains(seg) {
            inner.due.push(seg.clone());
        }
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap()
    }

    fn statuses(&self, status: &str) -> Vec<String> {
        self.inner().queue.iter().filter(|(_, q)| q.status == status).map(|(id, _)| id.clone()).collect()
    }

    fn set_status(&self, entity_id: &str, hash: &str, status: &'static str, build: &str) {
        if let Some(q) = self.inner().queue.get_mut(entity_id).filter(|q| q.meta.hash == hash) {
            q.status = status;
            q.build = build.into();
        }
    }

    fn retry(&self, from: &str) -> u64 {
        let mut n = 0;
        for q in self.inner().queue.values_mut().filter(|q| q.status == from) {
            q.status = "pending";
            n += 1;
        }
        n
    }

    /// Hand the ids of a segment not handed yet to the runtime, as the
    /// archive does, marking each once the runtime took it.
    async fn replay_held(&self, seg: &SegmentId, replayer: &dyn Replayer) -> messenger_core::Result<ReplayReport> {
        let fresh: Vec<String> = {
            let inner = self.inner();
            let local = inner.local.get(&seg.entity_id()).cloned().unwrap_or_default();
            let seen = inner.replayed.get(&seg.entity_id()).cloned().unwrap_or_default();
            local.into_iter().filter(|id| !seen.contains(id)).collect()
        };
        let mut report = ReplayReport::default();
        for id in fresh {
            match replayer.replay(event(&id, seg.scope.clone())).await? {
                Replayed::Applied => report.applied += 1,
                Replayed::Duplicate => report.duplicate += 1,
                Replayed::Ignored => report.ignored += 1,
            }
            self.inner().replayed.entry(seg.entity_id()).or_default().insert(id);
        }
        Ok(report)
    }

    /// Keep the events of a file (the version is held from here on) and
    /// hand those not handed yet to the runtime.
    async fn absorb(&self, seg: &SegmentId, bytes: &[u8], replayer: &dyn Replayer) -> messenger_core::Result<ReplayReport> {
        let (_, ids) = read(bytes);
        {
            let mut inner = self.inner();
            inner.held.insert(veydan_sync::sha256_hex(bytes), seg.entity_id());
            inner.downloads += 1;
            inner.local.entry(seg.entity_id()).or_default().extend(ids.iter().cloned());
        }
        self.replay_held(seg, replayer).await
    }
}

#[async_trait]
impl ArchiveStore for FakeStore {
    async fn owner(&self) -> messenger_core::Result<Option<PubKey>> {
        Ok(Some(owner()))
    }
    async fn due(&self, _now: Timestamp, _force: bool) -> messenger_core::Result<Vec<SegmentId>> {
        Ok(self.inner().due.clone())
    }
    async fn build(&self, seg: &SegmentId) -> messenger_core::Result<Built> {
        if self.inner().too_big.contains(&seg.entity_id()) {
            return Err(messenger_archive::too_large(format!("{}: a file of 17 MiB", seg.entity_id())));
        }
        Ok(built(seg, self.inner().local.get(&seg.entity_id()).unwrap_or(&BTreeSet::new())))
    }
    async fn compact(&self, seg: &SegmentId) -> messenger_core::Result<u64> {
        self.inner().compacted.push(seg.entity_id());
        Ok(0)
    }
    async fn merge(&self, seg: &SegmentId, remote: &[u8], replayer: &dyn Replayer) -> messenger_core::Result<Merged> {
        let (of, theirs) = read(remote);
        if of.as_ref() != Some(seg) {
            return Err(messenger_archive::format_error("not a file of the test"));
        }
        let mine = self.inner().local.get(&seg.entity_id()).cloned().unwrap_or_default();
        // What this device wrote of its own is not handed to its runtime.
        self.inner().replayed.entry(seg.entity_id()).or_default().extend(mine.iter().cloned());
        let replay = self.absorb(seg, remote, replayer).await?;
        let union = self.inner().local.get(&seg.entity_id()).cloned().unwrap_or_default();
        let upload = if union != theirs {
            Some(built(seg, &union))
        } else {
            let mut inner = self.inner();
            inner.due.retain(|s| s != seg);
            None
        };
        Ok(Merged { replay, upload })
    }
    async fn merge_held(&self, seg: &SegmentId, hash: &str, replayer: &dyn Replayer) -> messenger_core::Result<Option<Merged>> {
        if !self.inner().held.contains_key(hash) {
            return Ok(None);
        }
        let replay = self.replay_held(seg, replayer).await?;
        // The fake does not keep the ids of a version: the union of what is
        // held goes up as the file.
        let union = self.inner().local.get(&seg.entity_id()).cloned().unwrap_or_default();
        Ok(Some(Merged { replay, upload: Some(built(seg, &union)) }))
    }
    async fn holds(&self, _seg: &SegmentId, hash: &str) -> messenger_core::Result<bool> {
        Ok(self.inner().held.contains_key(hash))
    }
    async fn written(&self, seg: &SegmentId, meta: &SegmentMeta) -> messenger_core::Result<()> {
        let mut inner = self.inner();
        inner.written.push((seg.entity_id(), meta.hash.clone()));
        inner.held.insert(meta.hash.clone(), seg.entity_id());
        // What it wrote of its own is no news to its runtime.
        let mine = inner.local.get(&seg.entity_id()).cloned().unwrap_or_default();
        inner.replayed.entry(seg.entity_id()).or_default().extend(mine);
        inner.due.retain(|s| s != seg);
        Ok(())
    }
    async fn defer(&self, seg: &SegmentId) -> messenger_core::Result<()> {
        let mut inner = self.inner();
        inner.deferred.push(seg.entity_id());
        inner.due.retain(|s| s != seg);
        Ok(())
    }
    async fn forget_written(&self) -> messenger_core::Result<()> {
        self.inner().forgot += 1;
        Ok(())
    }
    async fn high_water(&self, _scope: &ArchiveScope) -> messenger_core::Result<Option<Timestamp>> {
        Ok(None)
    }
    async fn enqueue(&self, entity_id: &str, meta: &SegmentMeta) -> messenger_core::Result<Enqueued> {
        SegmentId::parse(entity_id).ok_or_else(|| MessengerError::Invalid("id".into()))?;
        let mut inner = self.inner();
        if inner.held.contains_key(&meta.hash) {
            let local = inner.local.get(entity_id).cloned().unwrap_or_default();
            let seen = inner.replayed.get(entity_id).cloned().unwrap_or_default();
            let unreplayed = local.iter().any(|id| !seen.contains(id));
            if unreplayed {
                let row = inner.queue.entry(entity_id.into()).or_insert_with(|| Queued { meta: meta.clone(), status: "pending", build: String::new() });
                if row.meta.hash != meta.hash || row.status == "done" {
                    *row = Queued { meta: meta.clone(), status: "pending", build: String::new() };
                }
            }
            return Ok(Enqueued::Held { unreplayed });
        }
        match inner.queue.get(entity_id) {
            Some(row) if row.meta.hash == meta.hash => Ok(Enqueued::Known),
            _ => {
                inner.queue.insert(entity_id.into(), Queued { meta: meta.clone(), status: "pending", build: String::new() });
                Ok(Enqueued::Queued)
            }
        }
    }
    async fn dequeue(&self, open: Option<&ArchiveScope>) -> messenger_core::Result<Option<QueueItem>> {
        let inner = self.inner();
        let mut items: Vec<(SegmentId, &SegmentMeta, &String)> = inner
            .queue
            .iter()
            .filter(|(_, q)| q.status == "pending")
            .filter_map(|(id, q)| SegmentId::parse(id).map(|seg| (seg, &q.meta, id)))
            .collect();
        // The open chat first, then the newest months.
        items.sort_by(|a, b| {
            let other = |s: &SegmentId| Some(&s.scope) != open;
            other(&a.0).cmp(&other(&b.0)).then(b.0.month.cmp(&a.0.month))
        });
        Ok(items.first().map(|(seg, meta, id)| QueueItem {
            entity_id: (*id).clone(),
            segment: seg.clone(),
            count: meta.count,
            size: meta.size,
            hash: meta.hash.clone(),
        }))
    }
    async fn done(&self, entity_id: &str, hash: &str) -> messenger_core::Result<()> {
        self.set_status(entity_id, hash, "done", "");
        Ok(())
    }
    async fn failed(&self, entity_id: &str, hash: &str, _error: &str) -> messenger_core::Result<()> {
        self.set_status(entity_id, hash, "failed", "");
        Ok(())
    }
    async fn park(&self, entity_id: &str, hash: &str, _why: &str) -> messenger_core::Result<()> {
        self.set_status(entity_id, hash, "parked", "");
        Ok(())
    }
    async fn refuse(&self, entity_id: &str, hash: &str, _error: &str, build: &str) -> messenger_core::Result<()> {
        self.set_status(entity_id, hash, "refused", build);
        Ok(())
    }
    async fn retry_failed(&self) -> messenger_core::Result<u64> {
        Ok(self.retry("failed"))
    }
    async fn retry_parked(&self) -> messenger_core::Result<u64> {
        Ok(self.retry("parked"))
    }
    async fn retry_refused(&self, build: &str) -> messenger_core::Result<u64> {
        let mut n = 0;
        for q in self.inner().queue.values_mut().filter(|q| q.status == "refused" && q.build != build) {
            q.status = "pending";
            n += 1;
        }
        Ok(n)
    }
    async fn blocked(&self) -> messenger_core::Result<Vec<String>> {
        let mut out = self.statuses("parked");
        out.extend(self.statuses("refused"));
        Ok(out)
    }
    async fn restoring_scopes(&self) -> messenger_core::Result<Vec<String>> {
        let inner = self.inner();
        let scopes: BTreeSet<String> = inner
            .queue
            .values()
            .filter(|q| matches!(q.status, "pending" | "failed" | "parked"))
            .map(|q| q.meta.scope.clone())
            .collect();
        Ok(scopes.into_iter().collect())
    }
    async fn refused_too_large(&self) -> messenger_core::Result<Vec<String>> {
        Ok(Vec::new())
    }
    async fn unwritable(&self) -> messenger_core::Result<Vec<String>> {
        let inner = self.inner();
        Ok(inner.too_big.iter().filter_map(|id| SegmentId::parse(id)).map(|seg| seg.month.to_string()).collect())
    }
    async fn clear_done(&self) -> messenger_core::Result<()> {
        self.inner().queue.retain(|_, q| q.status != "done");
        Ok(())
    }
    async fn progress(&self) -> messenger_core::Result<QueueProgress> {
        let inner = self.inner();
        let open: Vec<&Queued> = inner.queue.values().filter(|q| matches!(q.status, "pending" | "done" | "failed")).collect();
        let done = open.iter().filter(|q| q.status == "done").count() as i64;
        let bytes_total = open.iter().map(|q| q.meta.size as i64).sum();
        Ok(QueueProgress { done, total: open.len() as i64, bytes_done: 0, bytes_total })
    }
    async fn replay_segment(&self, item: &QueueItem, bytes: &[u8], replayer: &dyn Replayer) -> messenger_core::Result<ReplayReport> {
        let (seg, _) = read(bytes);
        let seg = seg.ok_or_else(|| messenger_archive::format_error("not a file of the test"))?;
        if seg != item.segment {
            return Err(MessengerError::Invalid("not the item's file".into()));
        }
        self.absorb(&seg, bytes, replayer).await
    }
    async fn replay_held_of(&self, item: &QueueItem, replayer: &dyn Replayer) -> messenger_core::Result<Option<ReplayReport>> {
        if !self.inner().held.contains_key(&item.hash) {
            return Ok(None);
        }
        Ok(Some(self.replay_held(&item.segment, replayer).await?))
    }
    async fn remove_scope(&self, scope: &ArchiveScope) -> messenger_core::Result<Vec<String>> {
        let infix = format!("/{}/", scope.as_key());
        let mut inner = self.inner();
        let ids: Vec<String> = inner.local.keys().filter(|id| id.contains(&infix)).cloned().collect();
        inner.local.retain(|id, _| !id.contains(&infix));
        inner.replayed.retain(|id, _| !id.contains(&infix));
        inner.queue.retain(|id, _| !id.contains(&infix));
        inner.held.retain(|_, of| !of.contains(&infix));
        inner.due.retain(|s| s.scope != *scope);
        inner.removed.extend(ids.iter().cloned());
        let key = scope.as_key();
        if !inner.forgotten.contains(&key) {
            inner.forgotten.push(key);
        }
        Ok(ids)
    }
    async fn has_server(&self, id: &str) -> messenger_core::Result<bool> {
        Ok(self.inner().servers.contains(id))
    }
    async fn take_forgotten_scopes(&self) -> messenger_core::Result<Vec<ArchiveScope>> {
        let taken = std::mem::take(&mut self.inner().forgotten);
        Ok(taken.iter().filter_map(|k| ArchiveScope::parse(k)).collect())
    }
    async fn group_ids(&self) -> messenger_core::Result<Vec<String>> {
        Ok(self.inner().groups.iter().cloned().collect())
    }
}

/// The way in of the runtime of the test: what it was handed, in order.
/// An event of a group waits for the group's welcome (`welcome`), as a
/// device with no row for the group answers `group_unknown`; one of a
/// private server the device does not have (`missing_servers`) is
/// `server_unknown`; between sessions it answers `NotLoggedIn`, from the
/// call `logged_out_at` on when that is set; its store is busy from the
/// call `busy_from` on when that is set.
#[derive(Default)]
struct FakeReplayer {
    seen: std::sync::Mutex<Vec<String>>,
    welcome: std::sync::Mutex<Option<String>>,
    missing_servers: std::sync::Mutex<BTreeSet<String>>,
    logged_out: AtomicBool,
    logged_out_at: std::sync::Mutex<Option<usize>>,
    busy_from: std::sync::Mutex<Option<usize>>,
    calls: AtomicUsize,
}

#[async_trait]
impl Replayer for FakeReplayer {
    async fn replay(&self, event: ArchivedEvent) -> messenger_core::Result<Replayed> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if self.logged_out.load(Ordering::SeqCst) || self.logged_out_at.lock().unwrap().is_some_and(|at| call >= at) {
            return Err(MessengerError::NotLoggedIn);
        }
        if self.busy_from.lock().unwrap().is_some_and(|at| call >= at) {
            return Err(MessengerError::Storage("database is locked".into()));
        }
        if let ArchiveScope::DmServer { id } = &event.scope {
            if self.missing_servers.lock().unwrap().contains(id) {
                return Err(MessengerError::Invalid(REFUSED_SERVER_UNKNOWN.into()));
            }
        }
        let mut seen = self.seen.lock().unwrap();
        let welcome = self.welcome.lock().unwrap().clone();
        if matches!(event.scope, ArchiveScope::Group { .. }) && welcome.is_some_and(|w| !seen.contains(&w)) {
            return Err(MessengerError::Invalid(REFUSED_GROUP_UNKNOWN.into()));
        }
        let id = event.id.as_hex().to_string();
        let new = !seen.contains(&id);
        seen.push(id);
        Ok(if new { Replayed::Applied } else { Replayed::Duplicate })
    }
}

/// The runtime of the test: what the handler told it.
#[derive(Default)]
struct FakeRuntime {
    replayer: Arc<FakeReplayer>,
    installed: std::sync::Mutex<Vec<bool>>,
    backfills: AtomicU8,
    /// The walk of the past fails this many times before it goes through.
    backfill_fails: AtomicU8,
    restored: AtomicU8,
    keys: std::sync::Mutex<Vec<String>>,
}

#[async_trait]
impl ArchiveRuntime for FakeRuntime {
    fn replayer(&self) -> Arc<dyn Replayer> {
        self.replayer.clone()
    }
    fn set_archive(&self, installed: bool) {
        self.installed.lock().unwrap().push(installed);
    }
    fn archive_backfill(&self) -> messenger_core::Result<BoxFuture<'static, messenger_core::Result<u64>>> {
        self.backfills.fetch_add(1, Ordering::SeqCst);
        let fail = self.backfill_fails.load(Ordering::SeqCst) > 0;
        if fail {
            self.backfill_fails.fetch_sub(1, Ordering::SeqCst);
        }
        Ok(Box::pin(async move { if fail { Err(MessengerError::Storage("busy".into())) } else { Ok(0) } }))
    }
    fn archive_restored(&self) {
        self.restored.fetch_add(1, Ordering::SeqCst);
    }
    async fn group_secret_arrived(&self, name: &str) -> messenger_core::Result<()> {
        self.keys.lock().unwrap().push(name.into());
        Ok(())
    }
}

fn id(n: u8) -> String {
    format!("{n:064x}")
}

// ── A device ─────────────────────────────────────────────────────────────────

const SYSTEM: &[&str] = &["password_vault", "setting", "label"];
const PLAN: Plan = Plan {
    collect: &[Step::rows("system rows", None, "app", SYSTEM), Step::handler(HANDLER, None, HANDLER)],
    apply: &[Step::rows("system rows", None, "app", SYSTEM), Step::handler(HANDLER, None, HANDLER)],
    late: &[],
};

const LOCK: Schema = Schema { module: "lock", steps: veydan_lock::SCHEMA_STEPS };

struct Device {
    core: Core,
    lock: Lock,
    sync: SyncManager,
    arch: ArchiveSync,
    store: Arc<FakeStore>,
    runtime: Arc<FakeRuntime>,
    clock: std::sync::Mutex<Option<HlcClock>>,
    _dir: tempfile::TempDir,
}

impl veydan_sync_host::States for Device {
    fn state(&self, ty: std::any::TypeId) -> Option<&(dyn std::any::Any + Send + Sync)> {
        let states: [&(dyn std::any::Any + Send + Sync); 4] = [&self.core, &self.lock, &self.sync, &self.arch];
        states.into_iter().find(|state| std::any::Any::type_id(*state) == ty)
    }
}

impl Device {
    async fn new(name: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let schemas = [veydan_core::SCHEMA, LOCK, veydan_sync_host::SCHEMA, SCHEMA];
        let pool = db::open(&dir.path().join(db::DB_FILE), &schemas).await.unwrap();
        let mut registry = Registry::new();
        registry.add("messenger", register);
        let lock = Lock::new(pool.clone());
        lock.open_default().await.unwrap();
        let device = Self {
            lock,
            core: Core::new(pool, dir.path().to_owned(), Directory::default()),
            sync: SyncManager::new(registry.finish(&PLAN).unwrap()),
            arch: ArchiveSync::default(),
            store: Arc::new(FakeStore::default()),
            runtime: Arc::new(FakeRuntime::default()),
            clock: std::sync::Mutex::new(Some(HlcClock::new(name, None))),
            _dir: dir,
        };
        device.arch.provide_store(Some(device.store.clone()));
        device.arch.provide_runtime(Some(device.runtime.clone()), None);
        device
    }

    fn host(&self) -> veydan_sync_host::Host<'_> {
        veydan_sync_host::Host::States(self)
    }

    /// One collect of the handler, its commit run as after a push that
    /// went through; with the warnings of the cycle.
    async fn collect(&self, engine: &Engine) -> (Vec<Op>, Vec<String>) {
        let (out, _) = self.collect_only(engine).await;
        let Collected { ops, commit, warnings, .. } = out;
        if let Some(commit) = commit {
            commit(self.core.db.clone()).await.unwrap();
        }
        (ops, warnings)
    }

    async fn collect_only(&self, engine: &Engine) -> (Collected, Vec<String>) {
        let cfg = SyncConfig::default();
        let mut clock = self.clock.lock().unwrap().take().unwrap();
        let mut local = LocalState::default();
        let mut warnings = Vec::new();
        let out = {
            let mut cx = Cycle {
                host: self.host(),
                core: &self.core,
                sync: &self.sync,
                engine,
                config: &cfg,
                clock: &mut clock,
                local: &mut local,
                warnings: &mut warnings,
            };
            MessengerArchive.collect(&mut cx).await.unwrap()
        };
        *self.clock.lock().unwrap() = Some(clock);
        (out, warnings)
    }

    async fn apply(&self, engine: &Engine, ops: &[Op]) -> (Applied, Vec<String>) {
        let cfg = SyncConfig::default();
        let mut clock = HlcClock::new("apply", None);
        let mut local = LocalState::default();
        let mut warnings = Vec::new();
        let applied = {
            let mut cx = Cycle {
                host: self.host(),
                core: &self.core,
                sync: &self.sync,
                engine,
                config: &cfg,
                clock: &mut clock,
                local: &mut local,
                warnings: &mut warnings,
            };
            MessengerArchive.apply(&mut cx, ops).await.unwrap()
        };
        (applied, warnings)
    }

    async fn drain(&self, engine: &Engine) {
        archive::drain(&self.arch, engine, &self.core, &self.sync, None).await.unwrap();
    }

    fn secrets(&self) -> veydan_lock::SecretBox<'_> {
        self.lock.secret_box(crate::commands::SECRET_BOX)
    }

    fn replayed(&self) -> Vec<String> {
        self.runtime.replayer.seen.lock().unwrap().clone()
    }

    /// The private server `id` is here (`present`) or not: the store's
    /// list and the runtime's answer agree.
    fn server_present(&self, id: &str, present: bool) {
        let mut inner = self.store.inner();
        let mut missing = self.runtime.replayer.missing_servers.lock().unwrap();
        if present {
            inner.servers.insert(id.into());
            missing.remove(id);
        } else {
            inner.servers.remove(id);
            missing.insert(id.into());
        }
    }
}

/// How the storage of the test answers a write of a large file.
const OPEN: u8 = 0;
const FULL: u8 = 1;
const READ_ONLY: u8 = 2;

/// A folder that answers the writes of large files as Veydan Cloud does
/// for an account over its quota (413) or in grace (403 `read_only`).
struct Walled {
    dir: LocalDir,
    wall: Arc<AtomicU8>,
}

#[async_trait]
impl veydan_sync::Storage for Walled {
    async fn list(&self, prefix: &str) -> veydan_sync::Result<Vec<String>> {
        self.dir.list(prefix).await
    }
    async fn get(&self, key: &str) -> veydan_sync::Result<Option<Vec<u8>>> {
        self.dir.get(key).await
    }
    async fn put(&self, key: &str, data: &[u8]) -> veydan_sync::Result<()> {
        if key.starts_with("large-files/") {
            match self.wall.load(Ordering::SeqCst) {
                FULL => return Err(SyncError::Quota { pool: "files".into(), used: 10, limit: 10 }),
                READ_ONLY => return Err(SyncError::Storage("put: 403 Forbidden read_only: the plan ended until=1800000000".into())),
                _ => {}
            }
        }
        self.dir.put(key, data).await
    }
    async fn delete(&self, key: &str) -> veydan_sync::Result<()> {
        self.dir.delete(key).await
    }
}

struct Vault {
    engine: Engine,
    wall: Arc<AtomicU8>,
    _dir: tempfile::TempDir,
}

async fn vault() -> Vault {
    let dir = tempfile::tempdir().unwrap();
    let wall = Arc::new(AtomicU8::new(OPEN));
    let storage = Walled { dir: LocalDir::new(dir.path()), wall: wall.clone() };
    let (engine, _) = Engine::create(Box::new(storage), "passphrase", "dev-a").await.unwrap();
    Vault { engine, wall, _dir: dir }
}

fn seg(scope: ArchiveScope, year: i32, month: u8) -> SegmentId {
    SegmentId { owner: owner(), scope, month: Month { year, month } }
}

fn group(id: &str) -> ArchiveScope {
    ArchiveScope::Group { id: id.into() }
}

fn later(op: &Op) -> veydan_sync::Hlc {
    veydan_sync::Hlc { wall_ms: op.hlc.wall_ms + 1000, counter: 0, device_id: "dev-c".into() }
}

fn eid(s: &str) -> String {
    format!("{}/{s}", owner().as_hex())
}

// ── Segments ─────────────────────────────────────────────────────────────────

/// A due segment goes up as a large file of 1 MiB chunks; its op carries
/// the owner, the summary and the manifest in `lf_refs`, which keeps the
/// file for the collection of every client. The archive hears that it is
/// written, and a segment without news goes up no second time.
#[tokio::test]
async fn a_due_segment_goes_up_as_a_large_file_and_its_op_keeps_the_file() {
    let v = vault().await;
    let engine = &v.engine;
    let a = Device::new("dev-a").await;
    let oct = seg(ArchiveScope::Dm, 2026, 10);
    a.store.hold(&oct, &[&id(1), &id(2)]);

    let (ops, warnings) = a.collect(engine).await;

    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(ops.len(), 1);
    let op = &ops[0];
    assert_eq!((op.entity_type.as_str(), op.entity_id.as_str(), op.deleted), (ENTITY_SEGMENT, eid("dm/2026-10").as_str(), false));
    let expected = built(&oct, &[id(1), id(2)].into_iter().collect());
    let p = &op.payload;
    assert_eq!(p["owner"], owner().as_hex());
    assert_eq!(p["scope"], "dm");
    assert_eq!(p["month"], "2026-10");
    assert_eq!(p["count"], 2);
    assert_eq!(p["size"], expected.meta.size);
    assert_eq!(p["hash"], expected.meta.hash.as_str());
    assert_eq!(p["lf"]["size"], expected.meta.size);
    assert_eq!(p["lf_refs"], serde_json::json!([p["lf"]["manifest_id"]]));
    assert_eq!(veydan_sync::refs_from_payload(p), vec![p["lf"]["manifest_id"].as_str().unwrap().to_string()]);
    // The file in the storage is the segment, in chunks of 1 MiB.
    let lf: veydan_sync::LargeFileRef = serde_json::from_value(p["lf"].clone()).unwrap();
    let meta = engine.large_files(veydan_sync::LargeFileConfig::default()).unwrap().inspect(&lf).await.unwrap();
    assert_eq!((meta.size, meta.chunk_size), (expected.meta.size, messenger_archive::CHUNK_SIZE));
    assert_eq!(a.store.inner().written, [(eid("dm/2026-10"), expected.meta.hash.clone())]);
    assert_eq!(a.store.inner().forgot, 1, "a vault new to the device: everything is unwritten");

    // Nothing new: nothing goes up, the archive hears it is written.
    a.store.inner().due.push(oct.clone());
    let (ops, _) = a.collect(engine).await;
    assert!(ops.is_empty(), "{ops:?}");
    assert_eq!(a.store.inner().written.len(), 2);
    assert_eq!(a.store.inner().forgot, 1);
}

/// A segment whose events were all compacted away (D5) goes as a
/// tombstone, not as an empty file; one never published sends nothing.
#[tokio::test]
async fn a_segment_compacted_to_nothing_goes_as_a_tombstone() {
    let v = vault().await;
    let engine = &v.engine;
    let a = Device::new("dev-a").await;
    let oct = seg(ArchiveScope::Dm, 2026, 10);
    let nov = seg(ArchiveScope::Dm, 2026, 11);
    a.store.hold(&oct, &[&id(1)]);
    let (ops, _) = a.collect(engine).await;
    assert_eq!(ops.len(), 1);

    a.store.hold(&oct, &[]);
    a.store.hold(&nov, &[]);
    let (ops, warnings) = a.collect(engine).await;

    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(ops.len(), 1, "{ops:?}");
    assert_eq!((ops[0].entity_id.as_str(), ops[0].deleted), (eid("dm/2026-10").as_str(), true));
    assert_eq!(ops[0].payload, serde_json::json!({}));
    assert!(a.store.inner().due.is_empty(), "both are written");
    let (again, _) = a.collect(engine).await;
    assert!(again.is_empty(), "{again:?}");
}

/// Once in thirty days the closed segments the device knows are compacted
/// (D5): the open month is left alone, and a pass that just ran is not run
/// again by the next cycle.
#[tokio::test]
async fn the_closed_segments_are_compacted_once_a_month() {
    let v = vault().await;
    let engine = &v.engine;
    let a = Device::new("dev-a").await;
    let aug = seg(ArchiveScope::Dm, 2026, 8);
    let open = seg(ArchiveScope::Dm, Month::of(now()).year, Month::of(now()).month);
    a.store.hold(&aug, &[&id(1)]);
    a.store.hold(&open, &[&id(2)]);
    let (ops, _) = a.collect(engine).await;
    assert_eq!(ops.len(), 2);
    // The first pass found no segment yet known; it is stamped all the same.
    assert!(a.store.inner().compacted.is_empty());
    let stamped = veydan_core::settings::get(&a.core.db, "messenger_archive_compacted_at").await;
    assert!(stamped.is_some_and(|at| at.parse::<i64>().unwrap() > 0));

    let (ops, _) = a.collect(engine).await;
    assert!(ops.is_empty(), "{ops:?}");
    assert!(a.store.inner().compacted.is_empty(), "the pass ran today");

    // Thirty days on: the closed month is compacted, the open one is not.
    veydan_core::settings::set(&a.core.db, "messenger_archive_compacted_at", "0").await.unwrap();
    let (ops, warnings) = a.collect(engine).await;
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(ops.is_empty(), "nothing dropped: nothing to write ({ops:?})");
    assert_eq!(a.store.inner().compacted, [eid("dm/2026-08")]);
}

/// A cycle moves a bounded part of the archive: the rest stays due.
#[tokio::test]
async fn a_cycle_moves_a_bounded_number_of_segments() {
    let v = vault().await;
    let engine = &v.engine;
    let a = Device::new("dev-a").await;
    for m in 1..=8u8 {
        a.store.hold(&seg(ArchiveScope::Dm, 2025, m), &[&id(m)]);
    }
    let (ops, warnings) = a.collect(engine).await;
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(ops.len() as u32, archive::BUDGET_SEGMENTS);
    assert_eq!(a.store.inner().due.len(), 3, "the rest wait");
    let (ops, _) = a.collect(engine).await;
    assert_eq!(ops.len(), 3);
    assert!(a.store.inner().due.is_empty());
}

/// The chat on screen names the scope of the archive to restore first.
#[test]
fn the_scope_of_a_chat_is_read_from_its_id() {
    assert_eq!(scope_of_chat("dm:ab12"), Some(ArchiveScope::Dm));
    assert_eq!(scope_of_chat("dm:ab12@srv1"), Some(ArchiveScope::DmServer { id: "srv1".into() }));
    assert_eq!(scope_of_chat("group:g1"), Some(ArchiveScope::Group { id: "g1".into() }));
    assert_eq!(scope_of_chat("group:"), None);
    assert_eq!(scope_of_chat("dm:"), None);
    assert_eq!(scope_of_chat("channel:c1"), None);
}

/// Segments another device wrote are queued and restored newest months
/// first, the open chat before all; when the queue is empty the UI hears
/// `archive.synced`. The same ops again restore nothing twice.
#[tokio::test]
async fn segments_of_another_device_are_restored_newest_first_the_open_chat_before_all() {
    let v = vault().await;
    let engine = &v.engine;
    let a = Device::new("dev-a").await;
    a.store.hold(&seg(ArchiveScope::Dm, 2026, 9), &[&id(1)]);
    a.store.hold(&seg(ArchiveScope::Dm, 2026, 10), &[&id(2)]);
    a.store.hold(&seg(group("g1"), 2026, 8), &[&id(3)]);
    let (ops, _) = a.collect(engine).await;
    assert_eq!(ops.len(), 3);

    let b = Device::new("dev-b").await;
    b.arch.set_open(Some(group("g1")));
    let (applied, warnings) = b.apply(engine, &ops).await;

    assert!(applied.retry.is_none() && applied.skipped.is_empty(), "{:?} {:?}", applied.retry, applied.skipped);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(b.replayed(), [id(3), id(2), id(1)]);
    assert!(b.store.inner().queue.is_empty(), "done and cleared");
    let told = b.arch.told();
    assert_eq!(told.last().map(|e| e.name.as_str()), Some(EVENT_SYNCED));
    let phases: Vec<String> = told
        .iter()
        .filter(|e| e.name == EVENT_PROGRESS)
        .map(|e| e.payload["phase"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(phases.first().map(String::as_str), Some("restoring"));
    assert_eq!(phases.last().map(String::as_str), Some("idle"));
    let first = told.iter().find(|e| e.name == EVENT_PROGRESS).unwrap();
    assert_eq!(first.payload["month"], "2026-08");
    assert_eq!(first.payload["total"], 3);

    // The ops come again (another read of the logs): nothing is restored twice.
    b.apply(engine, &ops).await;
    assert_eq!(b.replayed().len(), 3);

    // The status names what the storage holds.
    let status = status(&b.arch, &b.core).await.unwrap();
    assert_eq!(status.from_month.as_deref(), Some("2026-08"));
    assert_eq!(status.to_month.as_deref(), Some("2026-10"));
    let sizes: u64 = ops.iter().map(|op| op.payload["size"].as_u64().unwrap()).sum();
    assert_eq!(status.size_bytes, sizes);
    assert!(status.restoring_scopes.is_empty());
}

/// A month whose file is over the bounds of the format (ROO-682): not
/// downloaded, so a phone never holds it in memory; the rest of the
/// archive comes, the restore ends, the month stays in the queue (refused,
/// for a build that reads it) and the status names it. The collect does
/// not merge it either.
#[tokio::test]
async fn a_segment_over_the_bounds_is_refused_before_it_comes_down() {
    let v = vault().await;
    let engine = &v.engine;
    let a = Device::new("dev-a").await;
    a.store.hold(&seg(ArchiveScope::Dm, 2026, 9), &[&id(1)]);
    a.store.hold(&seg(ArchiveScope::Dm, 2026, 10), &[&id(2)]);
    let (mut ops, _) = a.collect(engine).await;
    let big = ops.iter_mut().find(|op| op.entity_id == eid("dm/2026-10")).unwrap();
    let size = messenger_archive::MAX_SEGMENT_BYTES + 1;
    big.payload["size"] = size.into();
    big.payload["lf"]["size"] = size.into();

    let b = Device::new("dev-b").await;
    let (applied, _) = b.apply(engine, &ops).await;

    assert!(applied.skipped.is_empty(), "{:?}", applied.skipped);
    assert_eq!(b.replayed(), [id(1)], "the rest of the archive came");
    assert_eq!(b.store.inner().downloads, 1, "the big one never came down");
    assert_eq!(b.store.statuses("refused"), [eid("dm/2026-10")], "it stays in the queue");
    assert_eq!(b.runtime.restored.load(Ordering::SeqCst), 1, "and does not hold the restore up");
    let status = status(&b.arch, &b.core).await.unwrap();
    assert_eq!(status.too_large, ["2026-10"]);

    // A collect of B does not merge it either: refused, and deferred.
    b.store.inner().due.push(seg(ArchiveScope::Dm, 2026, 10));
    let (_, warnings) = b.collect(engine).await;
    assert!(warnings.iter().any(|w| w.contains("too large")), "{warnings:?}");
    assert_eq!(b.store.inner().downloads, 1);
    assert_eq!(b.store.inner().deferred, [eid("dm/2026-10")]);
}

/// A month of my own over the bounds of the format (ROO-692): the writer
/// does not publish it, the month waits the flush interval and the status
/// names it as in no backup, on the device that holds it.
#[tokio::test]
async fn a_month_too_large_to_write_is_named_as_not_backed_up() {
    let v = vault().await;
    let engine = &v.engine;
    let a = Device::new("dev-a").await;
    a.store.hold(&seg(ArchiveScope::Dm, 2026, 9), &[&id(1)]);
    a.store.hold(&seg(ArchiveScope::Dm, 2026, 10), &[&id(2)]);
    a.store.inner().too_big.insert(eid("dm/2026-10"));

    let (ops, warnings) = a.collect(engine).await;

    assert!(warnings.iter().any(|w| w.contains("too large")), "{warnings:?}");
    assert!(ops.iter().all(|op| op.entity_id != eid("dm/2026-10")), "not published");
    assert!(ops.iter().any(|op| op.entity_id == eid("dm/2026-09")), "the rest is");
    assert_eq!(a.store.inner().deferred, [eid("dm/2026-10")]);
    let status = status(&a.arch, &a.core).await.unwrap();
    assert_eq!(status.not_backed_up, ["2026-10"]);
    assert!(status.too_large.is_empty());
}

/// The staging files of the archive: what a download cut off left (a
/// process killed) goes at the next start, and the folder is empty once
/// the restore is over.
#[tokio::test]
async fn the_staging_folder_is_empty_after_a_restore_and_a_restart() {
    let v = vault().await;
    let engine = &v.engine;
    let a = Device::new("dev-a").await;
    a.store.hold(&seg(ArchiveScope::Dm, 2026, 10), &[&id(1)]);
    let (ops, _) = a.collect(engine).await;

    let b = Device::new("dev-b").await;
    let root = b.core.app_data_dir.join(crate::commands::DATA_SUBDIR).join("archive-staging");
    // Left by a run before: a loose file of an older build, a cut download.
    std::fs::create_dir_all(root.join("drain")).unwrap();
    std::fs::write(root.join("0123abcd"), b"cut").unwrap();
    std::fs::write(root.join("drain").join("4567ef01.part"), b"cut").unwrap();
    std::fs::create_dir_all(root.join("collect")).unwrap();
    std::fs::write(root.join("collect").join("89ab.part"), b"cut").unwrap();

    b.apply(engine, &ops).await;
    assert_eq!(b.replayed(), [id(1)]);
    b.collect(engine).await;

    let left: Vec<_> = walk(&root);
    assert!(left.is_empty(), "{left:?}");
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

/// A version of another device that this one has not merged is read
/// first: the events only it has are replayed (D7) and the union goes up.
#[tokio::test]
async fn the_version_of_another_device_is_merged_before_the_union_goes_up() {
    let v = vault().await;
    let engine = &v.engine;
    let oct = seg(ArchiveScope::Dm, 2026, 10);
    let a = Device::new("dev-a").await;
    a.store.hold(&oct, &[&id(1), &id(2)]);
    let (ops, _) = a.collect(engine).await;

    let b = Device::new("dev-b").await;
    // B takes A's op while its messenger does not run: nothing replayed yet.
    b.arch.provide_runtime(None, None);
    b.apply(engine, &ops).await;
    assert!(b.replayed().is_empty());
    b.arch.provide_runtime(Some(b.runtime.clone()), None);
    b.store.hold(&oct, &[&id(2), &id(3)]);

    let (mine, warnings) = b.collect(engine).await;

    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(b.replayed(), [id(1)], "what only A had is replayed");
    let union = built(&oct, &[id(1), id(2), id(3)].into_iter().collect());
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].payload["hash"], union.meta.hash.as_str());
    assert_eq!(mine[0].payload["count"], 3);
    // Merged and replayed: the background restore has nothing of it left.
    let st = archive::load(&b.core.db, &eid("dm/2026-10")).await.unwrap().unwrap();
    assert_eq!((st.hash.as_str(), st.replayed_hash.as_str()), (union.meta.hash.as_str(), union.meta.hash.as_str()));

    // A takes the union: nothing of it is new to A but id 3.
    a.apply(engine, &mine).await;
    assert_eq!(a.replayed(), [id(3)]);
}

/// A version this device read already (the restore replayed it) is merged
/// from what is held: no second download.
#[tokio::test]
async fn a_version_read_by_the_restore_is_merged_without_a_download() {
    let v = vault().await;
    let engine = &v.engine;
    let oct = seg(ArchiveScope::Dm, 2026, 10);
    let a = Device::new("dev-a").await;
    a.store.hold(&oct, &[&id(1)]);
    let (ops, _) = a.collect(engine).await;

    let b = Device::new("dev-b").await;
    b.apply(engine, &ops).await;
    assert_eq!(b.replayed(), [id(1)]);
    assert_eq!(b.store.inner().downloads, 1);
    // B's own event of the month: the storage's version is held, the
    // union goes up without the file coming down again.
    b.store.hold(&oct, &[&id(1), &id(2)]);
    let (mine, warnings) = b.collect(engine).await;
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].payload["count"], 2);
    assert_eq!(b.store.inner().downloads, 1, "no download");
}

/// A tombstone takes the segment out of the queue; a scope removed here
/// sends its segments as tombstones, once.
#[tokio::test]
async fn a_removed_scope_sends_tombstones_and_a_tombstone_takes_the_segment_out() {
    let v = vault().await;
    let engine = &v.engine;
    let a = Device::new("dev-a").await;
    a.store.hold(&seg(group("g1"), 2026, 9), &[&id(1)]);
    a.store.hold(&seg(group("g1"), 2026, 10), &[&id(2)]);
    a.store.hold(&seg(ArchiveScope::Dm, 2026, 10), &[&id(3)]);
    let (ops, _) = a.collect(engine).await;
    assert_eq!(ops.len(), 3);

    a.arch.forget_scope(&a.core.db, &group("g1")).await.unwrap();
    let (tombs, _) = a.collect(engine).await;

    let mut ids: Vec<&str> = tombs.iter().map(|op| op.entity_id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, [eid("group:g1/2026-09"), eid("group:g1/2026-10")]);
    assert!(tombs.iter().all(|op| op.deleted && op.payload == serde_json::json!({}) && op.entity_type == ENTITY_SEGMENT));
    assert_eq!(a.store.inner().removed.len(), 2);
    let (again, _) = a.collect(engine).await;
    assert!(again.is_empty(), "{again:?}");

    // B has the segment queued, its messenger not running; the tombstone
    // takes it out, and nothing of it is restored later.
    let b = Device::new("dev-b").await;
    b.arch.provide_runtime(None, None);
    b.apply(engine, &ops).await;
    b.apply(engine, &tombs).await;
    b.arch.provide_runtime(Some(b.runtime.clone()), None);
    b.drain(engine).await;
    assert_eq!(b.replayed(), [id(3)]);
    let st = archive::load(&b.core.db, &eid("group:g1/2026-09")).await.unwrap().unwrap();
    assert!(st.deleted);
    let status = status(&b.arch, &b.core).await.unwrap();
    assert_eq!((status.from_month.as_deref(), status.to_month.as_deref()), (Some("2026-10"), Some("2026-10")));
}

/// A private server removed here — by hand, or by the snapshot of another
/// device, where the store alone forgets the scope and leaves its mark:
/// the handler takes the mark, the segments count as not replayed, and,
/// the server being absent, are parked before their file comes down, so
/// nothing of the server is put back. The server put in again (the runtime
/// wakes the handler: the parked go back, the restore looks) restores them
/// anew; the home chat is left alone; no tombstone goes to the storage.
#[tokio::test]
async fn a_removed_server_is_restored_again_when_it_comes_back() {
    let v = vault().await;
    let engine = &v.engine;
    let srv = ArchiveScope::DmServer { id: "s1".into() };
    let segment = eid("dm@s1/2026-10");
    let a = Device::new("dev-a").await;
    a.server_present("s1", true);
    a.store.hold(&seg(srv.clone(), 2026, 10), &[&id(1)]);
    a.store.hold(&seg(ArchiveScope::Dm, 2026, 10), &[&id(2)]);
    let (ops, _) = a.collect(engine).await;

    let b = Device::new("dev-b").await;
    b.server_present("s1", true);
    b.apply(engine, &ops).await;
    assert_eq!(b.replayed(), [id(2), id(1)]);
    assert_eq!(b.store.inner().downloads, 2);

    // Removed by hand: the app forgets the scope at once, mark and all.
    b.server_present("s1", false);
    b.arch.forget_scope_locally(&b.core.db, &srv).await.unwrap();
    assert_eq!(b.store.inner().removed, std::slice::from_ref(&segment));
    assert!(b.store.inner().forgotten.is_empty(), "the mark is taken with the forgetting");
    let st = archive::load(&b.core.db, &segment).await.unwrap().unwrap();
    assert!(st.replayed_hash.is_empty() && !st.hash.is_empty(), "the storage's version, not replayed");
    // The restore looks (as the removal kicks it): the server is absent, so
    // the segment is parked before its file comes down.
    b.drain(engine).await;
    assert_eq!(b.replayed(), [id(2), id(1)], "nothing of the server comes back while it is absent");
    assert_eq!(b.store.inner().downloads, 2, "parked before the file comes down");
    assert_eq!(b.store.statuses("parked"), std::slice::from_ref(&segment));
    assert!(!b.store.inner().local.contains_key(&segment), "nothing of it put back");
    assert_eq!(b.runtime.restored.load(Ordering::SeqCst), 1, "a parked segment does not hold the restore");

    // The server comes back: the runtime wakes the handler.
    b.server_present("s1", true);
    assert_eq!(b.store.retry_parked().await.unwrap(), 1);
    b.drain(engine).await;
    assert_eq!(b.replayed(), [id(2), id(1), id(1)]);
    assert_eq!(b.store.inner().downloads, 3);
    let (ops, _) = b.collect(engine).await;
    assert!(ops.iter().all(|op| !op.deleted), "{ops:?}");

    // Removed by the snapshot of another device: the store forgets the
    // scope (`servers::remove_private`) and marks it; the handler's
    // position still says replayed until it takes the mark.
    b.server_present("s1", false);
    b.store.remove_scope(&srv).await.unwrap();
    assert_eq!(b.store.inner().forgotten, ["dm@s1"]);
    let st = archive::load(&b.core.db, &segment).await.unwrap().unwrap();
    assert!(!st.replayed_hash.is_empty(), "stale until the handler runs");
    b.drain(engine).await;
    assert!(b.store.inner().forgotten.is_empty(), "taken");
    let st = archive::load(&b.core.db, &segment).await.unwrap().unwrap();
    assert!(st.replayed_hash.is_empty(), "reset by the mark");
    assert_eq!(b.store.statuses("parked"), std::slice::from_ref(&segment));
    assert_eq!(b.replayed().len(), 3, "nothing put back");
    assert_eq!(b.store.inner().downloads, 3);

    b.server_present("s1", true);
    b.store.retry_parked().await.unwrap();
    b.drain(engine).await;
    assert_eq!(b.replayed(), [id(2), id(1), id(1), id(1)]);
}

/// The handler could not run on an event (its store busy): what was taken
/// is marked, the event and the rest are not, the item fails and the next
/// restore takes the rest from what is held, without the file; only then
/// is the restore over. An event the handler refuses is left behind
/// instead (`messenger_archive`'s tests tell the two apart).
#[tokio::test]
async fn a_handler_that_could_not_run_keeps_the_segment_for_the_next_restore() {
    let v = vault().await;
    let engine = &v.engine;
    let a = Device::new("dev-a").await;
    a.store.hold(&seg(ArchiveScope::Dm, 2026, 10), &[&id(1), &id(2), &id(3)]);
    let (ops, _) = a.collect(engine).await;

    let b = Device::new("dev-b").await;
    *b.runtime.replayer.busy_from.lock().unwrap() = Some(1);
    b.apply(engine, &ops).await;
    assert_eq!(b.replayed(), [id(1)], "taken before the store was busy");
    assert_eq!(b.store.statuses("failed"), [eid("dm/2026-10")]);
    let st = archive::load(&b.core.db, &eid("dm/2026-10")).await.unwrap().unwrap();
    assert!(st.replayed_hash.is_empty(), "not marked replayed");
    assert_eq!(b.runtime.restored.load(Ordering::SeqCst), 0, "the restore is not over");

    *b.runtime.replayer.busy_from.lock().unwrap() = None;
    b.drain(engine).await;
    assert_eq!(b.replayed(), [id(1), id(2), id(3)]);
    assert_eq!(b.store.inner().downloads, 1, "from what is held");
    assert!(b.store.inner().queue.is_empty());
    assert_eq!(b.runtime.restored.load(Ordering::SeqCst), 1);
}

/// A full quota and a read-only account (grace) pause the writing: the
/// cycle goes on, the reason is kept for the card and told to the UI, the
/// segments of other devices are still restored, and the writing waits an
/// hour (or the app leaving the screen) before the storage is asked again
/// (plan §4.5).
#[tokio::test]
async fn a_storage_that_refuses_writes_pauses_the_writing_not_the_reading_nor_the_cycle() {
    let v = vault().await;
    let engine = &v.engine;
    let a = Device::new("dev-a").await;
    a.store.hold(&seg(ArchiveScope::Dm, 2026, 10), &[&id(1)]);
    let (theirs, _) = a.collect(engine).await;

    // Each round its own events: a file the storage has already goes up
    // without a write.
    for (how, reason, mine) in [(FULL, PauseReason::Quota, 2), (READ_ONLY, PauseReason::ReadOnly { until: Some(1_800_000_000) }, 4)] {
        v.wall.store(how, Ordering::SeqCst);
        let b = Device::new("dev-b").await;
        b.store.hold(&seg(ArchiveScope::Dm, 2026, 11), &[&id(mine)]);

        let (ops, warnings) = b.collect(engine).await;

        assert!(ops.is_empty(), "{ops:?}");
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(b.arch.paused(), Some(reason.clone()));
        let told = b.arch.told();
        let last = told.iter().rev().find(|e| e.name == EVENT_PROGRESS).unwrap();
        assert_eq!(last.payload["phase"], "paused");
        assert_eq!(serde_json::from_value::<PauseReason>(last.payload["reason"].clone()).unwrap(), reason);
        assert_eq!(b.store.inner().due.len(), 1, "the segment waits");

        // Reading goes on.
        b.apply(engine, &theirs).await;
        assert_eq!(b.replayed(), [id(1)]);

        // The wall is gone, but the storage said no a moment ago: the
        // writing waits for the backoff, without a try.
        v.wall.store(OPEN, Ordering::SeqCst);
        let (ops, warnings) = b.collect(engine).await;
        assert!(ops.is_empty() && warnings.is_empty(), "{ops:?} {warnings:?}");
        assert_eq!(b.arch.paused(), Some(reason.clone()));
        // The app leaves the screen: the pause is tried again, and the
        // write goes through.
        b.arch.flush_soon();
        let (ops, _) = b.collect(engine).await;
        assert_eq!(ops.len(), 1);
        assert_eq!(b.arch.paused(), None);
    }
}

#[test]
fn the_reasons_of_a_pause_come_from_the_errors_of_the_storage() {
    let quota = SyncError::Quota { pool: "files".into(), used: 1, limit: 1 };
    assert_eq!(pause_reason(&quota), Some(PauseReason::Quota));
    let grace = SyncError::Storage("put: 403 Forbidden read_only: grace (request r1)".into());
    assert_eq!(pause_reason(&grace), Some(PauseReason::ReadOnly { until: None }));
    let dated = SyncError::Storage("put: 403 Forbidden read_only: grace until=1800000000 (request r1)".into());
    assert_eq!(pause_reason(&dated), Some(PauseReason::ReadOnly { until: Some(1_800_000_000) }));
    let down = SyncError::Storage("Veydan Cloud host h: error sending request for url".into());
    assert_eq!(pause_reason(&down), Some(PauseReason::Offline));
    assert_eq!(pause_reason(&SyncError::Storage("put: 500 Internal Server Error".into())), None);
    assert_eq!(pause_reason(&SyncError::Integrity("x".into())), None);
}

/// Without the seams of the archive (the messenger never ran) the handler
/// publishes nothing of the segments and keeps what it takes until then.
#[tokio::test]
async fn without_an_archive_the_ops_wait_in_the_table() {
    let v = vault().await;
    let engine = &v.engine;
    let a = Device::new("dev-a").await;
    a.store.hold(&seg(ArchiveScope::Dm, 2026, 10), &[&id(1)]);
    let (ops, _) = a.collect(engine).await;

    let b = Device::new("dev-b").await;
    b.arch.provide_store(None);
    b.apply(engine, &ops).await;
    assert!(b.store.inner().queue.is_empty());
    let (mine, _) = b.collect(engine).await;
    assert!(mine.is_empty());

    // The archive comes (Э5): the restore takes what the table kept.
    b.arch.provide_store(Some(b.store.clone()));
    b.drain(engine).await;
    assert_eq!(b.replayed(), [id(1)]);
}

/// Another identity's segments in the same storage are recorded and left:
/// not queued, not replayed, not counted; this identity's are.
#[tokio::test]
async fn segments_of_another_owner_are_left_alone() {
    let v = vault().await;
    let engine = &v.engine;
    let a = Device::new("dev-a").await;
    a.store.hold(&seg(ArchiveScope::Dm, 2026, 10), &[&id(1)]);
    let (mut ops, _) = a.collect(engine).await;
    // The same op under another owner: as another identity's device would write it.
    let mut theirs = ops[0].clone();
    theirs.entity_id = format!("{}/dm/2026-10", other_owner().as_hex());
    theirs.payload["owner"] = serde_json::Value::String(other_owner().as_hex().to_string());
    theirs.hlc = later(&ops[0]);
    ops.push(theirs);

    let b = Device::new("dev-b").await;
    let (applied, _) = b.apply(engine, &ops).await;
    assert!(applied.skipped.is_empty(), "{:?}", applied.skipped);
    assert_eq!(b.replayed(), [id(1)], "mine alone");
    assert_eq!(archive::load_all(&b.core.db).await.unwrap().len(), 2, "both recorded");
    let status = status(&b.arch, &b.core).await.unwrap();
    assert_eq!(status.size_bytes, ops[0].payload["size"].as_u64().unwrap(), "mine alone is counted");
    assert_eq!(b.runtime.restored.load(Ordering::SeqCst), 1, "the other's does not hold the restore");
}

/// The vault or the identity changes under the handler: nothing held of
/// the archive before is reused — every segment is written again, every
/// version counts as not replayed.
#[tokio::test]
async fn a_change_of_identity_on_the_same_vault_starts_the_positions_over() {
    let v = vault().await;
    let engine = &v.engine;
    let a = Device::new("dev-a").await;
    a.store.hold(&seg(ArchiveScope::Dm, 2026, 10), &[&id(1)]);
    let (ops, _) = a.collect(engine).await;
    let b = Device::new("dev-b").await;
    b.apply(engine, &ops).await;
    assert_eq!(b.replayed(), [id(1)]);
    b.collect(engine).await;
    assert_eq!(b.store.inner().forgot, 1);

    // As the binding to the same vault by another key leaves it.
    veydan_core::settings::set(&b.core.db, "messenger_archive_vault", &format!("{}|{}", engine.vault_id(), other_owner().as_hex())).await.unwrap();
    b.collect(engine).await;
    assert_eq!(b.store.inner().forgot, 2, "every segment is unwritten again");
    let st = archive::load(&b.core.db, &eid("dm/2026-10")).await.unwrap().unwrap();
    assert!(st.replayed_hash.is_empty() && st.merged_hash.is_empty(), "nothing of before counts as held");
}

/// A month of a group newer than the month of `dm` that holds its welcome
/// is dequeued first and refused (`group_unknown`); it is parked, goes
/// again after the `dm` month came in the same drain, and only then is the
/// restore over: the runtime hears `archive_restored` once. A group whose
/// welcome never comes stays parked and does not hold the restore up; it
/// goes again when a key of a group comes.
#[tokio::test]
async fn a_group_refused_before_its_welcome_is_parked_and_tried_again() {
    let v = vault().await;
    let engine = &v.engine;
    let a = Device::new("dev-a").await;
    a.store.hold(&seg(ArchiveScope::Dm, 2026, 10), &[&id(1)]);
    a.store.hold(&seg(group("g1"), 2026, 11), &[&id(5)]);
    let (ops, _) = a.collect(engine).await;

    let b = Device::new("dev-b").await;
    *b.runtime.replayer.welcome.lock().unwrap() = Some(id(1));
    let (applied, _) = b.apply(engine, &ops).await;

    assert!(applied.retry.is_none());
    assert_eq!(b.replayed(), [id(1), id(5)]);
    assert!(b.store.statuses("parked").is_empty(), "the second round took it");
    assert_eq!(b.runtime.restored.load(Ordering::SeqCst), 1);
    assert_eq!(b.arch.told().last().map(|e| e.name.as_str()), Some(EVENT_SYNCED));
    let st = archive::load(&b.core.db, &eid("group:g1/2026-11")).await.unwrap().unwrap();
    assert_eq!(st.replayed_hash, st.hash);

    // A group whose welcome never comes: parked, the restore is over
    // without it, the open chat of the group says so.
    let c = Device::new("dev-c").await;
    *c.runtime.replayer.welcome.lock().unwrap() = Some(id(9));
    c.apply(engine, &ops).await;
    assert_eq!(c.replayed(), [id(1)]);
    assert_eq!(c.store.statuses("parked"), [eid("group:g1/2026-11")]);
    assert_eq!(c.runtime.restored.load(Ordering::SeqCst), 1, "over without the parked one");
    assert_eq!(c.arch.told().last().map(|e| e.name.as_str()), Some(EVENT_SYNCED));
    let shown = status(&c.arch, &c.core).await.unwrap();
    assert_eq!(shown.restoring_scopes, ["group:g1"]);
    assert_eq!(shown.progress.total, 0, "not counted");
    assert_eq!(c.store.inner().downloads, 2, "both files came down once");

    // The welcome comes (the row of the group appears) with a key of the
    // group: the parked segment goes again, from what is held.
    *c.runtime.replayer.welcome.lock().unwrap() = None;
    let key = Op {
        entity_type: ENTITY_SECRET.into(),
        entity_id: format!("{}/group.g1.key.k1", owner().as_hex()),
        hlc: later(&ops[0]),
        deleted: false,
        payload: serde_json::json!({ "owner": owner().as_hex(), "value": B64.encode(b"key one") }),
    };
    c.apply(engine, &[key]).await;
    assert_eq!(c.replayed(), [id(1), id(5)]);
    assert!(c.store.statuses("parked").is_empty());
    assert_eq!(c.store.inner().downloads, 2, "not downloaded again");
    assert!(status(&c.arch, &c.core).await.unwrap().restoring_scopes.is_empty());
}

/// Between two sessions of the runtime (`NotLoggedIn`) a segment stays
/// pending, not failed; the next restore takes it.
#[tokio::test]
async fn between_sessions_a_segment_stays_pending() {
    let v = vault().await;
    let engine = &v.engine;
    let a = Device::new("dev-a").await;
    a.store.hold(&seg(ArchiveScope::Dm, 2026, 10), &[&id(1)]);
    let (ops, _) = a.collect(engine).await;

    let b = Device::new("dev-b").await;
    b.runtime.replayer.logged_out.store(true, Ordering::SeqCst);
    b.apply(engine, &ops).await;
    assert!(b.replayed().is_empty());
    assert_eq!(b.store.statuses("pending"), [eid("dm/2026-10")]);
    assert_eq!(b.runtime.restored.load(Ordering::SeqCst), 0);

    b.runtime.replayer.logged_out.store(false, Ordering::SeqCst);
    b.drain(engine).await;
    assert_eq!(b.replayed(), [id(1)]);
    assert_eq!(b.runtime.restored.load(Ordering::SeqCst), 1);
}

/// The runtime goes between sessions in the middle of a segment (the
/// process is killed, the identity locks): the events taken stay taken,
/// the item stays pending, and the next restore replays the rest from
/// what is held — without downloading the file again — and only then is
/// the restore over.
#[tokio::test]
async fn a_replay_cut_in_the_middle_resumes_without_the_file() {
    let v = vault().await;
    let engine = &v.engine;
    let a = Device::new("dev-a").await;
    a.store.hold(&seg(ArchiveScope::Dm, 2026, 10), &[&id(1), &id(2), &id(3)]);
    let (ops, _) = a.collect(engine).await;

    let b = Device::new("dev-b").await;
    *b.runtime.replayer.logged_out_at.lock().unwrap() = Some(1);
    b.apply(engine, &ops).await;
    assert_eq!(b.replayed(), [id(1)], "one taken, then between sessions");
    assert_eq!(b.store.statuses("pending"), [eid("dm/2026-10")], "not done");
    assert_eq!(b.runtime.restored.load(Ordering::SeqCst), 0);
    assert_eq!(b.store.inner().downloads, 1);

    // The ops come again meanwhile (another read of the logs): the version
    // is held, with events to replay; nothing is marked replayed.
    *b.runtime.replayer.logged_out_at.lock().unwrap() = Some(0);
    b.apply(engine, &ops).await;
    let st = archive::load(&b.core.db, &eid("dm/2026-10")).await.unwrap().unwrap();
    assert!(st.replayed_hash.is_empty(), "held, not replayed");

    *b.runtime.replayer.logged_out_at.lock().unwrap() = None;
    b.drain(engine).await;
    assert_eq!(b.replayed(), [id(1), id(2), id(3)]);
    assert_eq!(b.store.inner().downloads, 1, "from what is held: no download");
    assert!(b.store.inner().queue.is_empty());
    assert_eq!(b.runtime.restored.load(Ordering::SeqCst), 1);
}

/// `done` of a version marks that version alone: a newer op of the same
/// segment, taken while the older file was replayed, stays queued and is
/// replayed next.
#[tokio::test]
async fn a_newer_version_taken_during_the_replay_is_not_lost() {
    let v = vault().await;
    let engine = &v.engine;
    let oct = seg(ArchiveScope::Dm, 2026, 10);
    let a = Device::new("dev-a").await;
    a.store.hold(&oct, &[&id(1)]);
    let (first, _) = a.collect(engine).await;
    a.store.hold(&oct, &[&id(1), &id(2)]);
    let (second, _) = a.collect(engine).await;

    let b = Device::new("dev-b").await;
    // Both versions come while the messenger does not run: the restore
    // of the first (dequeued here as the drain would) and the second meet.
    b.arch.provide_runtime(None, None);
    b.apply(engine, &first).await;
    let item = b.store.dequeue(None).await.unwrap().unwrap();
    assert_eq!(item.hash, first[0].payload["hash"]);
    b.apply(engine, &second).await;
    // As the drain would, after replaying the first file: done of the
    // first version leaves the second pending.
    b.store.done(&item.entity_id, &item.hash).await.unwrap();
    assert_eq!(b.store.statuses("pending"), [eid("dm/2026-10")]);
    assert_eq!(b.store.inner().queue[&eid("dm/2026-10")].meta.hash, second[0].payload["hash"]);
    b.arch.provide_runtime(Some(b.runtime.clone()), None);
    b.drain(engine).await;
    assert_eq!(b.replayed(), [id(1), id(2)], "the second version is restored next");
}

/// A storage the archive can use installs the archive in the runtime and
/// asks for the backfill of its history until a walk goes through; a device
/// with nothing to restore tells the runtime so once a run, so that its
/// catch-up does not wait. The storage left keeps the archive recording.
#[tokio::test]
async fn the_archive_is_installed_once_and_an_empty_restore_is_told_once() {
    let v = vault().await;
    let engine = &v.engine;
    let a = Device::new("dev-a").await;
    a.runtime.backfill_fails.store(1, Ordering::SeqCst);
    a.collect(engine).await;
    tokio::task::yield_now().await;
    a.collect(engine).await;
    tokio::task::yield_now().await;
    a.collect(engine).await;
    assert_eq!(*a.runtime.installed.lock().unwrap(), [true]);
    assert_eq!(a.runtime.backfills.load(Ordering::SeqCst), 2, "failed once, then went through, then no more");

    a.apply(engine, &[]).await;
    a.apply(engine, &[]).await;
    assert_eq!(a.runtime.restored.load(Ordering::SeqCst), 1);

    // The storage is left: the archive stays installed (recording is
    // local), the card says why nothing is written.
    a.arch.storage_gone();
    assert_eq!(*a.runtime.installed.lock().unwrap(), [true]);
    assert_eq!(a.arch.paused(), Some(PauseReason::NoStorage));
    // The runtime stops: the archive is taken out.
    a.arch.uninstall();
    assert_eq!(*a.runtime.installed.lock().unwrap(), [true, false]);
}

// ── Secrets ──────────────────────────────────────────────────────────────────

/// The keys and link secrets of groups travel under the owner's id, the
/// nsec and the rest of the box do not; a secret the box has is never
/// overwritten, a forgotten group takes its keys from the other devices, a
/// closed box holds the ops, another owner's secret is left alone.
#[tokio::test]
async fn the_keys_of_groups_travel_and_nothing_else_of_the_box() {
    let v = vault().await;
    let engine = &v.engine;
    let a = Device::new("dev-a").await;
    a.store.inner().groups.insert("g1".into());
    a.secrets().put("group.g1.key.k1", b"key one").await.unwrap();
    a.secrets().put("group.g1.link.2", b"link two").await.unwrap();
    a.secrets().put("identity.nsec", b"never").await.unwrap();
    a.secrets().put("media.host.secret", b"never").await.unwrap();

    let (ops, warnings) = a.collect(engine).await;

    assert!(warnings.is_empty(), "{warnings:?}");
    let mut names: Vec<&str> = ops.iter().map(|op| op.entity_id.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, [eid("group.g1.key.k1"), eid("group.g1.link.2")]);
    let key = ops.iter().find(|op| op.entity_id == eid("group.g1.key.k1")).unwrap();
    assert_eq!((key.entity_type.as_str(), key.deleted), (ENTITY_SECRET, false));
    assert_eq!(key.payload, serde_json::json!({ "owner": owner().as_hex(), "value": B64.encode(b"key one") }));
    let (again, _) = a.collect(engine).await;
    assert!(again.is_empty(), "{again:?}");

    // B holds its own link secret: it stays, and is not sent back.
    let b = Device::new("dev-b").await;
    b.secrets().put("group.g1.link.2", b"mine").await.unwrap();
    let (applied, _) = b.apply(engine, &ops).await;
    assert!(applied.retry.is_none());
    assert_eq!(b.secrets().get("group.g1.key.k1").await.unwrap().unwrap().as_slice(), b"key one");
    assert_eq!(b.secrets().get("group.g1.link.2").await.unwrap().unwrap().as_slice(), b"mine");
    assert_eq!(*b.runtime.keys.lock().unwrap(), ["group.g1.key.k1"], "the runtime hears of the key put");
    let (back, _) = b.collect(engine).await;
    assert!(back.is_empty(), "{back:?}");

    // An op that names the nsec is refused; one of another owner is left.
    let nsec = Op { entity_id: eid("identity.nsec"), hlc: later(key), ..key.clone() };
    let theirs = Op { entity_id: format!("{}/group.g2.key.k1", other_owner().as_hex()), hlc: later(key), ..key.clone() };
    let (applied, _) = b.apply(engine, &[nsec, theirs]).await;
    assert_eq!(applied.skipped.len(), 2, "{:?}", applied.skipped);
    assert_eq!(b.secrets().get("identity.nsec").await.unwrap(), None);
    assert_eq!(b.secrets().get("group.g2.key.k1").await.unwrap(), None);

    // A forgets the group: its secrets go as tombstones and leave B's box.
    a.arch.forget_scope(&a.core.db, &group("g1")).await.unwrap();
    let (tombs, _) = a.collect(engine).await;
    assert_eq!(tombs.len(), 2);
    assert!(tombs.iter().all(|op| op.deleted && op.payload == serde_json::json!({})));
    b.apply(engine, &tombs).await;
    assert_eq!(b.secrets().get("group.g1.key.k1").await.unwrap(), None);
    assert_eq!(b.secrets().get("group.g1.link.2").await.unwrap(), None);
    assert_eq!(a.secrets().get("group.g1.key.k1").await.unwrap().unwrap().as_slice(), b"key one", "the box of A is the runtime's");
    let (again, _) = a.collect(engine).await;
    assert!(again.is_empty(), "a forgotten group is not published again: {again:?}");

    // A closed box: the ops wait for the next cycle, the keys wait, the
    // cycle hears of it once.
    let c = Device::new("dev-c").await;
    c.lock.set(Some("4821".into()), None, Some("pin".into()), None).await.unwrap();
    c.lock.lock();
    let (applied, warnings) = c.apply(engine, &ops).await;
    assert!(applied.retry.is_some());
    assert_eq!(applied.retry_from, ["dev-a"]);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(c.secrets().names().await.unwrap().is_empty());
}

/// Another key imported on a device that had groups (ROO-681): the store
/// is wiped, the box is not, and the positions start over. The secrets of
/// the groups before stay out of the archive of the new identity: their
/// groups have no row in its store. Its own group goes as before.
#[tokio::test]
async fn the_secrets_of_the_identity_before_are_not_published_for_the_new_one() {
    let v = vault().await;
    let engine = &v.engine;
    let a = Device::new("dev-a").await;
    // What the identity before left in the box: a key and links of its group.
    a.secrets().put("group.ga.key.k1", b"key of A").await.unwrap();
    a.secrets().put("group.ga.link.0", b"link of A").await.unwrap();
    a.secrets().put("group.ga.link.1", b"link of A, rotated").await.unwrap();
    // The new identity's store, as `account::wipe` left it, with a group of its own.
    a.store.inner().groups.insert("gb".into());
    a.secrets().put("group.gb.key.k9", b"key of B").await.unwrap();

    let (ops, warnings) = a.collect(engine).await;

    assert!(warnings.is_empty(), "{warnings:?}");
    let names: Vec<&str> = ops.iter().filter(|op| op.entity_type == ENTITY_SECRET).map(|op| op.entity_id.as_str()).collect();
    assert_eq!(names, [eid("group.gb.key.k9")], "nothing of the groups of A");
    assert!(ops.iter().all(|op| !op.entity_id.contains("group.ga.")), "{ops:?}");
}

// ── Status ───────────────────────────────────────────────────────────────────

/// What `messenger_archive_status` answers, as the UI reads it.
#[tokio::test]
async fn the_status_has_the_shape_the_ui_reads() {
    let d = Device::new("dev-a").await;
    let status = status(&d.arch, &d.core).await.unwrap();
    // No storage joined in the test.
    assert!(!status.available);
    let json = serde_json::to_value(&status).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "available": false,
            "progress": { "phase": "idle", "done": 0, "total": 0, "bytes_done": 0, "bytes_total": 0 },
            "size_bytes": 0,
            "from_month": null,
            "to_month": null,
            "paused": { "why": "no_storage" },
            "restoring_scopes": [],
            "too_large": [],
            "not_backed_up": [],
        })
    );
    let decl = <ArchiveStatus as TS>::decl(&ts_rs::Config::default());
    assert!(decl.contains("size_bytes: number"), "{decl}");
    assert!(decl.contains("paused: PauseReason | null"), "{decl}");
    assert!(decl.contains("restoring_scopes: Array<string>"), "{decl}");
    assert!(decl.contains("too_large: Array<string>"), "{decl}");
    assert!(decl.contains("not_backed_up: Array<string>"), "{decl}");
}

/// Progress while bytes move is told at most once a second; a change of
/// phase at once.
#[test]
fn progress_is_told_at_most_once_a_second_a_change_of_phase_at_once() {
    let arch = ArchiveSync::default();
    let restoring = |done| ArchiveProgress { phase: ArchivePhase::Restoring, done, total: 9, ..idle() };
    arch.tell(None, restoring(1));
    arch.tell(None, restoring(2));
    arch.tell(None, restoring(3));
    arch.tell(None, idle());
    let told = arch.told();
    assert_eq!(told.len(), 2, "{told:?}");
    assert_eq!(told[0].payload["done"], 1);
    assert_eq!(told[1].payload["phase"], "idle");
    assert_eq!(arch.last_progress(), idle());
}
