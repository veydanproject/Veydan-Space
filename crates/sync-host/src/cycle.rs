// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The cycle — push local changes, pull peers, apply, compact, collect
//! garbage — in the order the product's plan gives, and the scheduler that
//! starts it.

use crate::config::{self, build_storage, load_binding, load_config};
use crate::debug::{
    begin_debug_cycle, debug_on, ensure_debug_loaded, op_summary, short_id, trace, trace_skip,
};
use crate::registry::{
    Applied, Collected, Commit, ConflictInfo, Cycle, Handler, LeaseInfo, Part, Registry, Step,
    Table,
};
use crate::rows::{self, ApplyOutcome};
use crate::state::RowSyncState;
use crate::{edits, emit_progress, gc, join, progress_pct, state, Host, RunningGuard};
use crate::{SyncManager, EVENT_STATUS};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager};
use veydan_core::{settings, AppError, CmdResult, Core};
use veydan_sync::{Engine, HlcClock, LocalState, Op, PeerHead, Vmk};

const TICK_SEC: u64 = 1;
const COMPACT_AFTER_CHUNKS: usize = 64;
/// How far ahead of this device's clock an op's clock is named in the log of
/// the developer (audit A2-10).
const CLOCK_AHEAD_MS: u64 = 60 * 60 * 1000;
/// The cycles a second read of a log may stop below where the plain read of
/// it stands before the log counts as read all the same (audit A2-11).
const REREAD_STUCK_CYCLES: u32 = 5;
/// The pause between two tries of the start checks that failed.
const PREPARE_RETRY: std::time::Duration = std::time::Duration::from_secs(30);

/// One device found under `devices/` on the last cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageDevice {
    pub id: String,
    pub name: String,
    pub own: bool,
}

#[derive(Debug, Serialize)]
pub struct SyncStatus {
    pub enabled: bool,
    pub joined: bool,
    pub running: bool,
    pub vault_id: Option<String>,
    pub device_id: String,
    pub peers: usize,
    pub last_run: Option<String>,
    pub last_error: Option<String>,
    pub last_warning: Option<String>,
    pub conflicts: Vec<ConflictInfo>,
    /// Profiles whose files diverged (id, name).
    pub profile_conflicts: Vec<ConflictInfo>,
    pub profile_leases: Vec<LeaseInfo>,
    /// Remote ops received in the last cycle.
    pub last_applied: Option<u64>,
    /// Devices seen in storage on the last background cycle.
    pub storage_devices: Vec<StorageDevice>,
    pub gc_last: Option<String>,
    pub blobs_total: Option<u64>,
    pub blobs_removed_last_gc: Option<u64>,
    /// Large-file v2 objects (manifests + chunks) after the last GC.
    pub lf_total: Option<u64>,
    pub lf_removed_last_gc: Option<u64>,
    /// The device joined and has not read the vault in full yet: it sends
    /// nothing until then (`join::FRESH_BINDING`).
    pub first_read_pending: bool,
}

async fn cached_devices(
    db: &sqlx::Pool<sqlx::Sqlite>,
    own_id: &str,
    own_name: &str,
) -> Vec<StorageDevice> {
    let raw = settings::get(db, "sync_devices").await.unwrap_or_default();
    let mut list: Vec<StorageDevice> = serde_json::from_str(&raw).unwrap_or_default();
    for d in &mut list {
        d.own = d.id == own_id;
        if d.own && d.name.is_empty() {
            d.name = own_name.to_string();
        }
    }
    list
}

/// What `sync_status` answers; the conflicts and leases come from the modules
/// that registered them.
pub async fn status(core: &Core, sync: &SyncManager) -> CmdResult<SyncStatus> {
    let db = &core.db;
    let parts = sync.registry().status();
    let cfg = load_config(db).await;
    let binding = load_binding(db).await;
    let (local, _) = state::load_local_state(db).await?;
    let device_id = config::device_id(db).await?;
    let conflicts = match parts.conflicts {
        Some(conflicts) => conflicts(core).await?,
        None => Vec::new(),
    };
    let profile_conflicts = match parts.profile_conflicts {
        Some(conflicts) => conflicts(core).await?,
        None => Vec::new(),
    };
    let profile_leases = match parts.profile_leases {
        Some(leases) => leases(core, &device_id).await?,
        None => Vec::new(),
    };
    Ok(SyncStatus {
        enabled: cfg.enabled,
        joined: binding.is_some(),
        running: sync.is_running(),
        vault_id: binding.map(|b| b.vault_id),
        device_id: device_id.clone(),
        peers: local.peers.len(),
        storage_devices: cached_devices(db, &device_id, &cfg.device_name).await,
        last_run: settings::get(db, "sync_last_run").await,
        last_error: settings::get(db, "sync_last_error")
            .await
            .filter(|s| !s.is_empty()),
        last_warning: settings::get(db, "sync_last_warning")
            .await
            .filter(|s| !s.is_empty()),
        conflicts,
        profile_conflicts,
        profile_leases,
        last_applied: settings::get(db, "sync_last_applied")
            .await
            .and_then(|v| v.parse().ok()),
        gc_last: settings::get(db, gc::LAST_RUN_KEY)
            .await
            .and_then(|v| v.parse::<i64>().ok())
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|t| t.to_rfc3339()),
        blobs_total: settings::get(db, gc::BLOBS_TOTAL_KEY)
            .await
            .and_then(|v| v.parse().ok()),
        blobs_removed_last_gc: settings::get(db, gc::REMOVED_KEY)
            .await
            .and_then(|v| v.parse().ok()),
        lf_total: settings::get(db, gc::LF_TOTAL_KEY)
            .await
            .and_then(|v| v.parse().ok()),
        lf_removed_last_gc: settings::get(db, gc::LF_REMOVED_KEY)
            .await
            .and_then(|v| v.parse().ok()),
        first_read_pending: join::fresh_binding(db).await.is_some(),
    })
}

/// Engine for the joined vault.
pub async fn open_engine(core: &Core) -> CmdResult<Engine> {
    let db = &core.db;
    let cfg = load_config(db).await;
    let binding = load_binding(db)
        .await
        .ok_or_else(|| AppError::other("no vault joined"))?;
    let vmk = Vmk::from_base64(&binding.vmk_b64).map_err(AppError::other)?;
    let device = config::device_id(db).await?;
    Ok(Engine::with_key(
        build_storage(&cfg)?,
        &vmk,
        &binding.vault_id,
        &device,
    ))
}

/// Vault joined and background sync is on.
pub async fn vault_active(core: &Core) -> bool {
    let cfg = load_config(&core.db).await;
    cfg.enabled && load_binding(&core.db).await.is_some()
}

/// Run a cycle in the background unless one is already running.
pub fn trigger_cycle(app: &AppHandle, source: &str) {
    let app = app.clone();
    let source = source.to_string();
    tauri::async_runtime::spawn(async move {
        let core = app.state::<Core>();
        let sync = app.state::<SyncManager>();
        if sync.is_running() {
            trace_skip(&app, &source, "trigger", "already running");
            return;
        }
        if !load_config(&core.db).await.enabled {
            trace_skip(&app, &source, "trigger", "sync disabled");
            return;
        }
        // One that waits was told to the journal when the reason came.
        if let Err(e) = run_cycle(&app, &source).await {
            eprintln!("sync: {e}");
        }
    });
}

/// What a cycle asked for did: it ran (a failure of it is the `Err`), or
/// it waits for the backend — no cycle, nothing to tell again.
#[derive(Debug, PartialEq, Eq)]
pub enum Ran {
    Done,
    /// Why: the backend `cloud` signs with the identity of the messenger,
    /// which the app starts after sync and the user may switch off.
    Waits(String),
}

/// How a cycle starts, short of the app.
pub(crate) enum Begin<'a> {
    /// It waits ([`Ran::Waits`]) and is held: the scheduler runs it the
    /// tick the wait is over. `told` is the line for the journal when the
    /// reason changed — each reason is told once, not on every tick.
    Waits { why: String, told: Option<String> },
    /// Another runs.
    Busy,
    /// It runs while the guard lives; the last start is now and the last
    /// error none. `told` as above, when it no longer waits.
    Go { guard: RunningGuard<'a>, told: Option<String> },
}

/// The start of a cycle: a cycle that waits for the backend leaves the
/// last start and the last error as they were; one that goes takes the
/// held one with it.
pub(crate) async fn begin<'a>(db: &sqlx::SqlitePool, sync: &'a SyncManager) -> CmdResult<Begin<'a>> {
    let why = config::cloud_waiting(db).await;
    let told = sync.note_waiting(why.as_deref());
    if let Some(why) = why {
        return Ok(Begin::Waits { why, told });
    }
    if sync.running.swap(true, Ordering::SeqCst) {
        return Ok(Begin::Busy);
    }
    let guard = RunningGuard(&sync.running);
    sync.release_held();
    keep(db, "sync_last_started", &Utc::now().to_rfc3339()).await?;
    keep(db, "sync_last_error", "").await?;
    Ok(Begin::Go { guard, told })
}

/// A key the cycle keeps of itself — its start, its end, its error — is no
/// edit of the user: written unheard, it starts no cycle. Heard, the error of
/// a cycle that could not send a setting would start the next one, and so on.
pub(crate) async fn keep(db: &sqlx::SqlitePool, key: &str, value: &str) -> Result<(), AppError> {
    veydan_core::db::hearing(async {
        let _own = edits::Own::new();
        settings::set(db, key, value).await
    })
    .await
}

/// push local changes -> pull peers -> apply/merge -> compact -> persist.
/// A cycle that has to wait for the backend is no cycle: [`Ran::Waits`].
pub async fn run_cycle(app: &AppHandle, source: &str) -> CmdResult<Ran> {
    let core = app.state::<Core>();
    let sync = app.state::<SyncManager>();
    let tell = |line: Option<String>| {
        if let Some(line) = line {
            eprintln!("sync: {line}");
            trace_skip(app, source, "wait", &line);
        }
    };
    let _guard = match begin(&core.db, &sync).await? {
        Begin::Waits { why, told } => {
            tell(told);
            return Ok(Ran::Waits(why));
        }
        Begin::Busy => {
            trace_skip(app, source, "start", "already running");
            return Err(AppError::other("sync already running"));
        }
        Begin::Go { guard, told } => {
            tell(told);
            guard
        }
    };
    begin_debug_cycle(app, source);
    let db = &core.db;
    if debug_on(app) {
        let interval = settings::get(db, "sync_interval_sec")
            .await
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(config::DEFAULT_INTERVAL_SEC as i64);
        match (
            config::device_id(db).await,
            state::load_local_state(db).await,
        ) {
            (Ok(device), Ok((local, _))) => trace(
                app,
                "info",
                "start",
                &format!(
                    "interval {interval}s device {} peers {} seq {}",
                    short_id(&device),
                    local.peers.len(),
                    local.own_seq
                ),
            ),
            _ => trace(app, "info", "start", "cycle started"),
        }
    }
    // Let the UI show the running state for scheduled cycles too.
    let _ = app.emit(EVENT_STATUS, ());

    emit_progress(app, "collect", 0, 0, 0, "");
    let result = cycle(Host::App(app), &core, &sync).await;
    let now = Utc::now().to_rfc3339();
    match &result {
        Ok(warnings) => {
            keep(db, "sync_last_run", &now).await?;
            keep(db, "sync_last_error", "").await?;
            keep(db, "sync_last_warning", &warnings.join("; ")).await?;
            emit_progress(app, "done", 100, 0, 0, "");
            trace(app, "info", "done", "cycle finished");
        }
        Err(e) => {
            keep(db, "sync_last_error", &e.to_string()).await?;
            emit_progress(app, "error", 0, 0, 0, &e.to_string());
            trace(app, "error", "done", &e.to_string());
        }
    }
    let _ = app.emit(EVENT_STATUS, ());
    result.map(|_| Ran::Done)
}

/// Blob missing for an op older than the GC grace: it was collected, not delayed.
pub fn blob_gone(op: &Op, now_ms: u64) -> bool {
    now_ms.saturating_sub(op.hlc.wall_ms) >= gc::GRACE_MS as u64
}

/// A blob the storage gave back that does not open — its envelope, its key
/// or its form fails — for an op older than the GC grace (spec 9.3, "apply
/// errors: quarantine and retry"): no later delivery mends it, so the op is
/// skipped with a warning and the heads advance. A younger one may be a copy
/// still on its way (the client of a cloud folder) and is retried, as a
/// missing blob is.
pub fn blob_quarantined(op: &Op, e: &veydan_sync::SyncError, now_ms: u64) -> bool {
    use veydan_sync::SyncError::{Crypto, Format, Integrity};
    matches!(e, Integrity(_) | Crypto(_) | Format(_)) && blob_gone(op, now_ms)
}

/// Any failure of a handler's apply or finish (spec 9.3, "apply errors:
/// quarantine and retry"): recorded as "retry next cycle", which holds every
/// head so the ops come again (what was applied is cut there by its state's
/// HLC), and the other steps go on. A push inside the step that rotated the
/// device id ends the cycle: its engine writes to a log that is not this
/// device's any more.
async fn handler_failed<T: Default>(
    host: &Host<'_>,
    db: &sqlx::SqlitePool,
    engine: &Engine,
    step: &str,
    result: CmdResult<T>,
    deferred: &mut Option<String>,
) -> CmdResult<T> {
    match result {
        Ok(v) => Ok(v),
        Err(e) => {
            if config::device_id(db).await? != engine.device_id() {
                return Err(e);
            }
            log(host, "error", step, &e.to_string());
            deferred.get_or_insert_with(|| format!("{step}: {e}"));
            Ok(T::default())
        }
    }
}

/// The logs whose heads stay for the next cycle: a retry `from` the devices
/// named holds theirs alone, one that names none (`None`) holds them all.
fn hold(held: &mut Option<HashSet<String>>, from: &[String]) {
    match held {
        Some(set) if !from.is_empty() => set.extend(from.iter().cloned()),
        _ => *held = None,
    }
}

/// FOREIGN KEY failure in an apply step: record it as "retry next cycle" and go on.
fn fk_retry<T: Default>(
    step: &str,
    result: CmdResult<T>,
    deferred: &mut Option<String>,
) -> CmdResult<T> {
    match result {
        Ok(v) => Ok(v),
        Err(e) if rows::is_fk_error(&e) => {
            deferred.get_or_insert_with(|| format!("{step}: {e}"));
            Ok(T::default())
        }
        Err(e) => Err(e),
    }
}

/// A collision means another install writes to our log (restored backup, cloned
/// DB): move to a fresh device log; the next cycle re-collects and pushes there.
/// `at` is the position of the push that collided, which the storage holds.
pub async fn push_error(
    db: &sqlx::SqlitePool,
    registry: &Registry,
    at: Option<PeerHead>,
    e: veydan_sync::SyncError,
) -> AppError {
    if let veydan_sync::SyncError::OwnLogCollision(_) = e {
        return rotate(
            db,
            registry,
            at,
            "own log collision: device id rotated, changes are pushed on the next cycle",
        )
        .await;
    }
    AppError::other(e)
}

/// Rotate the device id for `why`, which the cycle ends with.
async fn rotate(
    db: &sqlx::SqlitePool,
    registry: &Registry,
    old_head: Option<PeerHead>,
    why: &str,
) -> AppError {
    if let Err(re) = config::rotate_device_id(db, registry, old_head).await {
        return re;
    }
    let _ = keep(db, "sync_last_warning", why).await;
    AppError::other(why)
}

/// The position of `local` as a peer head of the old log.
fn head_of(local: &LocalState) -> PeerHead {
    PeerHead {
        seq: local.own_seq,
        hash: local.own_head_hash.clone(),
    }
}

/// A push of the own log that did not go through.
#[derive(Debug)]
pub struct PushFailed {
    pub error: AppError,
    /// The log collided and the device id was rotated: the engine of the
    /// cycle writes to a log that is not this device's any more, and the
    /// cycle must end.
    pub rotated: bool,
}

impl From<PushFailed> for AppError {
    fn from(failed: PushFailed) -> AppError {
        failed.error
    }
}

/// Every push of the own log goes through here (spec 9.3, "the own log:
/// pending and reconciliation"): the chunk is kept locally before it is
/// written, the position moves with its removal in one transaction once it
/// is, and a collision rotates the device id. A lost answer leaves the
/// chunk kept, and the next cycle replays it with the same ops
/// (`reconcile_own_log`).
pub async fn push_own(
    engine: &Engine,
    db: &sqlx::SqlitePool,
    registry: &Registry,
    local: &mut LocalState,
    clock: &HlcClock,
    ops: Vec<Op>,
) -> Result<(), PushFailed> {
    let failed = |error| PushFailed {
        error,
        rotated: false,
    };
    if ops.is_empty() {
        return Ok(());
    }
    // Only a push that failed in this cycle leaves one; nothing is pushed
    // after it before the next cycle replays it.
    if state::load_own_pending(db).await.is_some() {
        return Err(failed(AppError::other(
            "a push of the own log waits for the next cycle to replay it",
        )));
    }
    let pending = state::OwnPending {
        vault_id: engine.vault_id().to_string(),
        device_id: engine.device_id().to_string(),
        seq: local.own_seq + 1,
        prev_hash: local.own_head_hash.clone(),
        ops,
    };
    state::save_own_pending(db, &pending)
        .await
        .map_err(failed)?;
    write_pending(engine, db, registry, local, clock, pending.ops).await
}

/// The push of the kept chunk, and what follows it.
async fn write_pending(
    engine: &Engine,
    db: &sqlx::SqlitePool,
    registry: &Registry,
    local: &mut LocalState,
    clock: &HlcClock,
    ops: Vec<Op>,
) -> Result<(), PushFailed> {
    let at = head_of(local);
    match engine.push(local, ops).await {
        Ok(()) => state::save_pushed(db, local, &clock.last())
            .await
            .map_err(|error| PushFailed {
                error,
                rotated: false,
            }),
        Err(e) => {
            let rotated = matches!(e, veydan_sync::SyncError::OwnLogCollision(_));
            Err(PushFailed {
                error: push_error(db, registry, Some(at), e).await,
                rotated,
            })
        }
    }
}

/// Once at the start of every cycle, before anything is collected or pushed
/// (spec 9.3, "the own log: pending and reconciliation"): the saved position
/// of the own log against what the storage holds.
///
/// - A kept chunk is pushed again with its ops: the storage either has it
///   (a lost answer, adopted) or not (written now). It is replayed only into
///   the slot after a chunk that continues the saved head, never into a
///   range a snapshot closed.
/// - The same seq with another hash whose chunk continues the saved head: the
///   trace of a save before it was one transaction; its hash is taken.
/// - The storage behind the position, confirmed by a GET of the saved chunk
///   by its key (a listing that lacks it is not believed): the device id
///   rotates, and what was pushed after the newest op the storage still
///   holds is published again under the new one. A rewind in place would
///   break the chain for a peer that read the lost chunks.
/// - Anything else — the storage ahead without a kept chunk (a restored
///   folder, a cloned machine), the chunk at the seq not ours — rotates the
///   device id too.
///
/// A rotation ends the cycle; the next one reads the old log as another
/// device's (from the position the storage confirms) and pushes under the
/// new id. Returns a warning for the cycle.
pub async fn reconcile_own_log(
    engine: &Engine,
    db: &sqlx::SqlitePool,
    registry: &Registry,
    local: &mut LocalState,
    clock: &mut HlcClock,
) -> CmdResult<Option<String>> {
    let pending = match state::load_own_pending(db).await {
        Some(p)
            if p.vault_id == engine.vault_id()
                && p.device_id == engine.device_id()
                && p.seq == local.own_seq + 1
                && p.prev_hash == local.own_head_hash =>
        {
            Some(p)
        }
        Some(_) => {
            // Not of this vault, this id or this position: its ops are
            // collected again.
            state::drop_own_pending(db).await?;
            None
        }
        None => None,
    };
    let (seq, head) = (local.own_seq, local.own_head_hash.clone());
    let stored = engine
        .own_log_status(seq, &head)
        .await
        .map_err(AppError::other)?;
    if let Some(newest) = &stored.newest {
        clock.observe(newest);
    }
    let own_chunk = |seq| async move { engine.own_chunk_hash(seq).await.map_err(AppError::other) };
    if stored.seq < seq {
        let listed_wrong = own_chunk(seq).await?.is_some()
            || (pending.is_some() && own_chunk(seq + 1).await?.is_some());
        if listed_wrong {
            if let Some(pending) = pending {
                replay(engine, db, registry, local, clock, pending).await?;
            }
            return Ok(Some(format!(
                "own log: the listing of the storage lacks chunk {seq}, which the storage holds; nothing was rewound"
            )));
        }
        let lost =
            state::forget_pushed_after(db, registry, engine.device_id(), stored.newest.as_ref())
                .await?;
        let tip = PeerHead {
            seq: stored.seq,
            hash: stored.head_hash,
        };
        return Err(rotate(
            db,
            registry,
            Some(tip),
            &format!(
                "own log: the storage lost the chunks of this device after {}, its data file knows {seq}: \
                 device id rotated, what they carried ({lost} rows and the files of the handlers) is pushed on the next cycle",
                stored.seq
            ),
        )
        .await);
    }
    if stored.seq == seq {
        if stored.head_hash != head {
            if stored.prev_hash.as_deref() != Some(head.as_str()) {
                return Err(rotate(
                    db,
                    registry,
                    None,
                    "own log: the chunk at this device's position is not its own; device id rotated, changes are pushed on the next cycle",
                )
                .await);
            }
            local.own_head_hash = stored.head_hash;
            // A kept chunk continues the hash it replaces; its ops are
            // collected again.
            state::save_pushed(db, local, &clock.last()).await?;
            return Ok(None);
        }
        if let Some(pending) = pending {
            replay(engine, db, registry, local, clock, pending).await?;
        }
        return Ok(None);
    }
    if let Some(pending) = pending.filter(|p| {
        stored.seq == seq + 1 && stored.prev_hash.as_deref() == Some(p.prev_hash.as_str())
    }) {
        replay(engine, db, registry, local, clock, pending).await?;
        return Ok(None);
    }
    // The old log goes on from the saved head only where the storage holds it.
    let confirmed =
        (own_chunk(seq).await?.as_deref() == Some(head.as_str())).then(|| head_of(local));
    Err(rotate(
        db,
        registry,
        confirmed,
        &format!(
            "own log: the storage holds {} chunks of this device, its data file knows {seq}: \
             a copy of this data file wrote them (a restored folder or a cloned machine); \
             device id rotated, changes are pushed on the next cycle",
            stored.seq
        ),
    )
    .await)
}

/// The kept chunk goes out again with its own ops.
async fn replay(
    engine: &Engine,
    db: &sqlx::SqlitePool,
    registry: &Registry,
    local: &mut LocalState,
    clock: &mut HlcClock,
    pending: state::OwnPending,
) -> CmdResult<()> {
    for op in &pending.ops {
        clock.observe(&op.hlc);
    }
    write_pending(engine, db, registry, local, clock, pending.ops).await?;
    Ok(())
}

/// The outcome of the rows steps of one stream, finished together.
struct Stream<'r> {
    name: &'static str,
    tables: Vec<&'r Table>,
    outcome: ApplyOutcome,
}

/// What a handler step of the collect left for later in the cycle.
pub struct Pending {
    pub label: &'static str,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// What the collect of a cycle gathered for one push.
#[derive(Default)]
pub struct Gathered {
    /// In the order of the plan's steps.
    pub ops: Vec<Op>,
    /// What the ops stand for, to save once the push went through.
    pub saves: Saves,
    pub pending: Vec<Pending>,
    /// `<label> <number of ops>` per step, for the developer log.
    pub counts: Vec<String>,
}

#[derive(Default)]
pub struct Saves {
    row_states: Vec<RowSyncState>,
    commits: Vec<Commit>,
}

impl Saves {
    /// The row states first, then the commits of the handlers in the order of the plan.
    pub async fn save(self, db: &sqlx::Pool<sqlx::Sqlite>) -> CmdResult<()> {
        for st in &self.row_states {
            state::save_row_state(db, st).await?;
        }
        for commit in self.commits {
            commit(db.clone()).await?;
        }
        Ok(())
    }
}

/// The local changes of every collect step of the plan, in its order. Data
/// first so a stuck profile upload cannot block notes. Without the app
/// (`Host::States`) it reports no progress.
pub async fn collect(cx: &mut Cycle<'_>) -> CmdResult<Gathered> {
    let registry = cx.sync.registry();
    let mut ops = Vec::new();
    let mut row_states = Vec::new();
    let mut commits = Vec::new();
    let mut pending = Vec::new();
    let mut counts = Vec::new();
    let reread = state::reread(&cx.core.db).await;
    for step in registry.plan().collect {
        let collected = match step.part {
            Part::Rows { entities, .. } => {
                collect_progress(cx, step);
                let changes = rows::collect_local_changes(
                    cx.core,
                    cx.clock,
                    &registry.rows(entities),
                    reread.as_deref(),
                )
                .await?;
                row_states.extend(changes.states);
                changes.ops
            }
            Part::Handler { name, .. } => {
                let Some(handler) = registry.handler_named(name) else {
                    continue;
                };
                collect_progress(cx, step);
                let Collected {
                    ops,
                    commit,
                    errors,
                    warnings,
                } = handler.collect(cx).await?;
                commits.extend(commit);
                pending.push(Pending {
                    label: step.label,
                    errors,
                    warnings,
                });
                ops
            }
        };
        counts.push(format!("{} {}", step.label, collected.len()));
        ops.extend(collected);
    }
    Ok(Gathered {
        ops,
        saves: Saves {
            row_states,
            commits,
        },
        pending,
        counts,
    })
}

/// Where the progress bar stands when `step` starts, for the app.
fn collect_progress(cx: &Cycle<'_>, step: &Step) {
    if let (Some(percent), Some(app)) = (step.progress, cx.host.app()) {
        emit_progress(app, "collect", percent, 0, 0, "");
    }
}

/// A line of the developer log; a cycle without the app writes none.
fn log(host: &Host<'_>, level: &str, step: &str, message: &str) {
    if let Some(app) = host.app() {
        trace(app, level, step, message);
    }
}

/// Where the progress bar stands; a cycle without the app shows none.
fn progress(host: &Host<'_>, phase: &str, percent: u32, current: u32, total: u32, detail: &str) {
    if let Some(app) = host.app() {
        emit_progress(app, phase, percent, current, total, detail);
    }
}

/// What the registries since the last full read skipped (spec 9.2, rule 3):
/// the logs read again from their start, less the ops `pulled` brings, less
/// the types and keys each of those registries took. Those ops were applied
/// as they came, and not every handler applies an op that comes again the
/// same way: a note older than its head can be merged anew, a lease older
/// than this device's takes the profile. So a log is read again in full
/// once: the logs of `done` were, and are read from where `local` stands.
async fn skipped_before(
    db: &sqlx::SqlitePool,
    engine: &Engine,
    pulled: &[Op],
    known: &[String],
    done: &[String],
    local: &LocalState,
) -> CmdResult<Reread> {
    let mut from_start = LocalState {
        peers: local
            .peers
            .iter()
            .filter(|(peer, _)| done.contains(peer))
            .map(|(peer, head)| (peer.clone(), head.clone()))
            .collect(),
        ..LocalState::default()
    };
    let all = engine
        .pull(&mut from_start, |_, _, _| {})
        .await
        .map_err(AppError::other)?;
    let fresh: HashSet<(String, &str, &str)> = pulled
        .iter()
        .map(|op| {
            (
                op.hlc.encode(),
                op.entity_type.as_str(),
                op.entity_id.as_str(),
            )
        })
        .collect();
    let ops = all
        .ops
        .into_iter()
        .filter(|op| {
            !state::took(known, &op.entity_type, &op.entity_id)
                && !fresh.contains(&(
                    op.hlc.encode(),
                    op.entity_type.as_str(),
                    op.entity_id.as_str(),
                ))
        })
        .collect();
    // A log is read again once its second read got as far as the plain read
    // of it stands (audit A2-11): to its end, or to a broken file no read
    // gets past — else one dead log would hold back the rows of the new
    // types for good. A read that stops below where the plain read stands
    // (chunks compacted while it read, a file that broke after the plain
    // read passed it) is tried again next cycle; after
    // `REREAD_STUCK_CYCLES` such cycles the log counts as read, with a
    // warning, for no read gets further. An error of the storage is tried
    // again each cycle, as before.
    let mut stuck = state::reread_stuck(db).await;
    let before = stuck.clone();
    let mut read = Vec::new();
    let mut errors = Vec::new();
    let mut broken = Vec::new();
    let mut given_up = Vec::new();
    for peer in &all.peers {
        let id = &peer.device_id;
        let reached = from_start.peers.get(id).map_or(0, |h| h.seq);
        let plain = local.peers.get(id).map_or(0, |h| h.seq);
        let counts = match &peer.error {
            Some(e) if !peer.broken => {
                errors.push((id.clone(), e.clone()));
                false
            }
            error if reached < plain => {
                let why = error.as_deref().unwrap_or("a chunk did not read");
                let line = format!(
                    "read again from its start, the log stops at {reached}, below {plain}: {why}"
                );
                let times = stuck.entry(id.clone()).or_insert(0);
                *times += 1;
                if *times < REREAD_STUCK_CYCLES {
                    errors.push((id.clone(), line));
                    false
                } else {
                    stuck.remove(id);
                    given_up.push((id.clone(), format!("{line} ({REREAD_STUCK_CYCLES} times)")));
                    true
                }
            }
            Some(e) => {
                stuck.remove(id);
                broken.push((id.clone(), e.clone()));
                true
            }
            None => {
                stuck.remove(id);
                true
            }
        };
        if counts && !done.contains(id) {
            read.push(id.clone());
        }
    }
    if stuck != before {
        state::set_reread_stuck(db, &stuck).await?;
    }
    Ok(Reread {
        ops,
        read,
        errors,
        broken,
        given_up,
    })
}

/// What a second read of the vault for a new registry brought.
struct Reread {
    ops: Vec<Op>,
    /// The logs read in full this time, or up to a broken file.
    read: Vec<String>,
    /// The logs that could not be read.
    errors: Vec<(String, String)>,
    /// The logs that stop at a broken file, and why.
    broken: Vec<(String, String)>,
    /// The logs that stopped below the plain read too often and count as
    /// read all the same, and why.
    given_up: Vec<(String, String)>,
}

/// One cycle over the states `host` reaches: what `run_cycle` runs in the
/// app, and what a test runs without it. Returns non-fatal warnings
/// (per-peer integrity problems).
pub async fn cycle(host: Host<'_>, core: &Core, sync: &SyncManager) -> CmdResult<Vec<String>> {
    // What the cycle applies is no edit of this device (`edits::Own`).
    veydan_core::db::hearing(steps(host, core, sync)).await
}

async fn steps(host: Host<'_>, core: &Core, sync: &SyncManager) -> CmdResult<Vec<String>> {
    if !sync.is_ready() {
        return Err(AppError::other(
            "sync is not ready: the checks of the data file at the start did not pass; they are repeated every 30 seconds",
        ));
    }
    let db = &core.db;
    let registry = sync.registry();
    let plan = registry.plan();
    let cfg = load_config(db).await;
    let engine = open_engine(core).await?;
    engine.verify_manifest().await.map_err(AppError::other)?;

    let (mut local, last_hlc) = state::load_local_state(db).await?;
    let mut clock = HlcClock::new(engine.device_id().to_string(), last_hlc.as_ref());

    let mut warnings: Vec<String> = Vec::new();
    // Every push of the cycle continues a position the storage agrees with,
    // whether the collect brings ops or a late step does (audit KA2-11).
    match reconcile_own_log(&engine, db, registry, &mut local, &mut clock).await {
        Ok(Some(w)) => {
            log(&host, "info", "push", &w);
            warnings.push(w);
        }
        Ok(None) => {}
        Err(e) => {
            log(&host, "error", "push", &e.to_string());
            return Err(e);
        }
    }
    // A key row made since the join was not compared with the vault's yet.
    let own = edits::Own::new();
    match join::settle_join_key(&host, &engine, core, registry).await {
        Ok(true) => join::key_row_changed(&host).await,
        Ok(false) => {}
        Err(e) => {
            log(&host, "error", "collect", &format!("key row: {e}"));
            warnings.push(format!("password vault key not published yet: {e}"));
        }
    }

    let mut cx = Cycle {
        host,
        core,
        sync,
        engine: &engine,
        config: &cfg,
        clock: &mut clock,
        local: &mut local,
        warnings: &mut warnings,
    };
    drop(own);
    edits::collecting(db).await;
    // A device that has just joined publishes nothing until it has read
    // the vault in full: what it holds would go out as new, over the vault.
    let fresh = join::fresh_binding(db).await;
    let Gathered {
        ops,
        saves,
        pending,
        counts,
    } = if fresh.is_some() {
        log(&host, "info", "collect", "joined: the vault is read first");
        Gathered::default()
    } else {
        collect(&mut cx).await?
    };
    log(&host, "info", "collect", &counts.join(", "));
    progress(&host, "push", 22, 0, 0, "");
    log(&host, "info", "push", &format!("{} ops", ops.len()));
    if !ops.is_empty() {
        if let Err(failed) = push_own(&engine, db, registry, &mut local, &clock, ops).await {
            log(&host, "error", "push", &failed.error.to_string());
            return Err(failed.error);
        }
        saves.save(db).await?;
    }

    progress(&host, "pull", 30, 0, 0, "devices/");
    let mut pulled = engine
        .pull(&mut local, |current, total, key| {
            progress(
                &host,
                "pull",
                progress_pct(30, 40, current, total),
                current,
                total,
                key,
            );
        })
        .await
        .map_err(|e| {
            log(&host, "error", "pull", &e.to_string());
            AppError::other(e)
        })?;
    let mut unread: Vec<String> = pulled.errors.iter().map(|(peer, _)| peer.clone()).collect();
    if pulled.peers.is_empty() {
        log(&host, "info", "pull", "no other devices");
    }
    for peer in &pulled.peers {
        let id = short_id(&peer.device_id);
        match &peer.error {
            Some(err) => log(
                &host,
                "error",
                "pull",
                &format!("{id} seq {} files {} {err}", peer.head_seq, peer.files),
            ),
            None => log(
                &host,
                "info",
                "pull",
                &format!(
                    "{id} seq {} files {} ops {}",
                    peer.head_seq, peer.files, peer.ops
                ),
            ),
        }
    }
    // The registry changed since the last start: what the earlier one did not
    // take is read again and applied with the pull.
    let mut reread = None;
    if let Some(known) = state::reread(db).await {
        progress(&host, "pull", 40, 0, 0, "devices/");
        let done = state::reread_peers(db).await;
        let again = skipped_before(db, &engine, &pulled.ops, &known, &done, &local)
            .await
            .inspect_err(|e| log(&host, "error", "pull", &e.to_string()))?;
        log(
            &host,
            "info",
            "pull",
            &format!(
                "read from the start: {} ops for a new registry",
                again.ops.len()
            ),
        );
        warnings.extend(
            again
                .errors
                .iter()
                .filter(|(peer, _)| !pulled.errors.iter().any(|(p, _)| p == peer))
                .map(|(peer, e)| format!("{peer}: {e}")),
        );
        // A log the plain read already names is not named twice.
        warnings.extend(
            again
                .broken
                .iter()
                .filter(|(peer, _)| !pulled.errors.iter().any(|(p, _)| p == peer))
                .map(|(peer, e)| {
                    format!(
                        "{peer}: {e}; what its log holds past this for the new types is not read"
                    )
                }),
        );
        warnings.extend(again.given_up.iter().map(|(peer, e)| {
            format!("{peer}: {e}; what its log holds past this for the new types is not read")
        }));
        if !again.ops.is_empty() {
            pulled.ops.extend(again.ops);
            pulled.ops.sort_by(|a, b| a.hlc.cmp(&b.hlc));
        }
        unread.extend(again.errors.iter().map(|(peer, _)| peer.clone()));
        reread = Some((again.read, again.errors.is_empty()));
    }
    // A clock far ahead takes this one along (spec 9.3, a risk taken, audit
    // A2-10): said in the log of the developer, once a cycle per device.
    let now_ms = Utc::now().timestamp_millis().max(0) as u64;
    let mut ahead: Vec<(String, u64)> = Vec::new();
    for op in &pulled.ops {
        clock.observe(&op.hlc);
        let by = op.hlc.wall_ms.saturating_sub(now_ms);
        if by > CLOCK_AHEAD_MS && !ahead.iter().any(|(d, _)| *d == op.hlc.device_id) {
            ahead.push((op.hlc.device_id.clone(), by));
        }
    }
    for (device, by) in &ahead {
        log(
            &host,
            "warn",
            "pull",
            &format!(
                "the clock of {} is {} h ahead of this device's: the clocks of the vault follow it",
                short_id(device),
                by / 3_600_000
            ),
        );
    }
    // Persist the advanced clock now: a failure while applying must not let the
    // next cycle stamp ops with an HLC below what it has already seen.
    state::save_own_state(db, &local, &clock.last()).await?;
    keep(db, "sync_last_applied", &pulled.ops.len().to_string()).await?;
    // A parent row missing on this device is not fatal: the ops come again next cycle.
    let mut deferred: Option<String> = None;
    let mut retries: Vec<(&str, Option<String>)> = Vec::new();
    // The logs whose heads stay for the next cycle (`hold`); `None` holds all.
    let mut held: Option<HashSet<String>> = Some(HashSet::new());
    let mut applied: Vec<(&dyn Handler, Applied)> = Vec::new();
    let mut streams: Vec<Stream> = Vec::new();
    // From here the cycle writes what it applies: no edit of this device.
    let _own = edits::Own::new();
    // Edits made since the collect, found before any step writes.
    let row_tables: Vec<_> = plan
        .apply
        .iter()
        .filter_map(|step| match step.part {
            Part::Rows { entities, .. } => Some(registry.rows(entities)),
            Part::Handler { .. } => None,
        })
        .flatten()
        .collect();
    let edited = rows::edited_rows(db, &row_tables, &pulled.ops).await?;
    for step in plan.apply {
        match step.part {
            Part::Handler { name, finish } => {
                let Some(handler) = registry.handler_named(name) else {
                    continue;
                };
                if let Some(percent) = step.progress {
                    progress(&host, "apply", percent, 0, 0, "");
                }
                let mut cx = Cycle {
                    host,
                    core,
                    sync,
                    engine: &engine,
                    config: &cfg,
                    clock: &mut clock,
                    local: &mut local,
                    warnings: &mut warnings,
                };
                let outcome = handler_failed(
                    &host,
                    db,
                    &engine,
                    step.label,
                    handler.apply(&mut cx, &pulled.ops).await,
                    &mut deferred,
                )
                .await?;
                handler_failed(
                    &host,
                    db,
                    &engine,
                    finish.unwrap_or(step.label),
                    handler.finish(&mut cx, &outcome).await,
                    &mut deferred,
                )
                .await?;
                if outcome.retry.is_some() {
                    hold(&mut held, &outcome.retry_from);
                }
                retries.push((step.label, outcome.retry.clone()));
                applied.push((handler, outcome));
            }
            Part::Rows { stream, entities } => {
                if let Some(percent) = step.progress {
                    progress(&host, "apply", percent, 0, 0, "");
                }
                let tables = registry.rows(entities);
                let outcome = fk_retry(
                    step.label,
                    rows::apply_ops_except(&host, core, &tables, &pulled.ops, &edited).await,
                    &mut deferred,
                )?;
                if outcome.retry.is_some() {
                    hold(&mut held, &[]);
                }
                retries.push((step.label, outcome.retry.clone()));
                match streams.iter_mut().find(|s| s.name == stream) {
                    Some(s) => {
                        s.tables.extend(tables);
                        s.outcome.changed.extend(outcome.changed);
                        s.outcome.settings.extend(outcome.settings);
                        if s.outcome.retry.is_none() {
                            s.outcome.retry = outcome.retry;
                        }
                    }
                    None => streams.push(Stream {
                        name: stream,
                        tables,
                        outcome,
                    }),
                }
            }
        }
    }
    for (handler, outcome) in &applied {
        handler.notify(&host, outcome);
    }
    for (label, retry) in &retries {
        if let Some(reason) = retry {
            log(&host, "retry", label, reason);
        }
    }
    for reason in applied.iter().flat_map(|(_, outcome)| &outcome.skipped) {
        log(&host, "warn", "apply", reason);
        warnings.push(reason.clone());
    }
    if let Some(reason) = &deferred {
        log(&host, "retry", "apply", reason);
    }
    log(&host, "info", "apply", &op_summary(&pulled.ops));
    for stream in &streams {
        rows::finish_apply(&host, core, &stream.tables, &stream.outcome).await;
    }

    for step in &pending {
        for e in &step.errors {
            log(&host, "error", step.label, e);
        }
    }
    warnings.extend(
        pulled
            .errors
            .into_iter()
            .map(|(peer, e)| format!("{peer}: {e}")),
    );
    for step in pending {
        warnings.extend(step.warnings);
    }
    let mut late_retry = None;
    for name in plan.late {
        let Some(handler) = registry.handler_named(name) else {
            continue;
        };
        let mut cx = Cycle {
            host,
            core,
            sync,
            engine: &engine,
            config: &cfg,
            clock: &mut clock,
            local: &mut local,
            warnings: &mut warnings,
        };
        let retry = handler.late(&mut cx, &pulled.ops).await?;
        late_retry = late_retry.or(retry);
    }

    state::save_own_state(db, &local, &clock.last()).await?;
    // The late steps and a failed step name no device: they hold every head.
    if late_retry.is_some() || deferred.is_some() {
        hold(&mut held, &[]);
    }
    let retry = retries
        .into_iter()
        .find_map(|(_, retry)| retry)
        .or(late_retry)
        .or(deferred);
    // A device that has just joined publishes from the cycle that read
    // and applied the vault in full.
    if let Some(fresh) = fresh {
        join::after_fresh_cycle(db, fresh, retry.as_deref(), &unread, &mut warnings).await?;
    }
    let clean = retry.is_none() && warnings.is_empty();
    match retry {
        None => {
            state::save_peer_heads(db, &local).await?;
            match &reread {
                Some((_, true)) => state::reread_done(db).await?,
                // A log that could not be read is read again next cycle;
                // those that were are not.
                Some((read, false)) => state::reread_peers_done(db, read).await?,
                None => {}
            }
        }
        Some(reason) => {
            // Every retry names the logs its ops came in: the heads of the
            // others advance, so a blob one device has not uploaded yet does
            // not hold the logs of all (audit SYNCNOTES-12, A2-08). An op
            // stamped by a device whose log this cycle did not read names
            // none it can be held in, and holds them all; a log read up to a
            // broken file was read (`PullPeer::broken`) and is held alone.
            let held = held.filter(|set| {
                set.iter().all(|d| {
                    pulled
                        .peers
                        .iter()
                        .any(|peer| peer.device_id == *d && (peer.error.is_none() || peer.broken))
                })
            });
            if let Some(held) = &held {
                let advanced = LocalState {
                    peers: local
                        .peers
                        .iter()
                        .filter(|(peer, _)| !held.contains(*peer))
                        .map(|(peer, head)| (peer.clone(), head.clone()))
                        .collect(),
                    ..LocalState::default()
                };
                state::save_peer_heads(db, &advanced).await?;
                if let Some((read, _)) = &reread {
                    let done: Vec<String> = read
                        .iter()
                        .filter(|peer| !held.contains(*peer))
                        .cloned()
                        .collect();
                    state::reread_peers_done(db, &done).await?;
                }
                let mut ids: Vec<String> = held.iter().map(|d| short_id(d)).collect();
                ids.sort_unstable();
                log(
                    &host,
                    "retry",
                    "done",
                    &format!("heads kept for {}, the others advance", ids.join(", ")),
                );
            }
            let line = format!("retry next cycle: {reason}");
            log(&host, "retry", "done", &line);
            warnings.push(line);
        }
    }

    if engine.own_chunk_count().await.map_err(AppError::other)? > COMPACT_AFTER_CHUNKS {
        progress(&host, "compact", 94, 0, 0, "");
        log(&host, "info", "compact", "compacting");
        // What the cycle did stands; the log is compacted on a later one.
        if let Err(e) = engine.compact(&local).await {
            log(&host, "error", "compact", &e.to_string());
            warnings.push(format!("compact postponed: {e}"));
        }
    }

    #[cfg(desktop)]
    if clean && gc::due(db).await {
        progress(&host, "gc", 97, 0, 0, "");
        log(&host, "info", "gc", "running");
        match gc::run(&engine, db, registry).await {
            Ok(outcome) if !outcome.unknown.is_empty() => log(
                &host,
                "info",
                "gc",
                &format!(
                    "blobs kept: the vault holds types this build does not sync: {}",
                    outcome.unknown.join(", ")
                ),
            ),
            Ok(_) => {}
            Err(e) => {
                log(&host, "error", "gc", &e.to_string());
                warnings.push(format!("gc skipped: {e}"));
            }
        }
    }
    #[cfg(mobile)]
    let _ = clean;

    if let Err(e) = engine.publish_device_name(&cfg.device_name).await {
        warnings.push(format!("device name: {e}"));
    }
    match engine.list_device_cards().await {
        Ok(cards) => {
            let own = engine.device_id();
            let devices: Vec<StorageDevice> = cards
                .into_iter()
                .map(|(id, name)| StorageDevice {
                    own: id == own,
                    name: if id == own && name.is_empty() {
                        cfg.device_name.clone()
                    } else {
                        name
                    },
                    id,
                })
                .collect();
            if let Ok(json) = serde_json::to_string(&devices) {
                settings::set(db, "sync_devices", &json).await?;
            }
        }
        Err(e) => warnings.push(format!("device list: {e}")),
    }

    Ok(warnings)
}

/// Check the install marker and the fingerprint of the registry, then start
/// the scheduler: once the modules whose data the cycle carries are set up.
/// A registry new to the data file — the first start of a build that syncs
/// labels among them — first has every owner publish its labels (spec 10.2),
/// and so does every later start until such a fill went through.
/// A check that fails is repeated by the scheduler until it passes; no cycle
/// runs before.
pub fn start(app: &AppHandle) {
    let core = app.state::<Core>();
    let sync = app.state::<SyncManager>();
    if let Err(e) = tauri::async_runtime::block_on(prepare(&core, &sync)) {
        eprintln!("sync: start checks failed, retrying: {e}");
    }
    edits::start(app.clone(), sync.registry());
    start_sync_scheduler(app.clone());
}

/// The checks of the start: the install marker, the labels of a new registry
/// and the registry itself. The cycles wait for all three (`SyncManager::ready`).
pub async fn prepare(core: &Core, sync: &SyncManager) -> CmdResult<()> {
    config::check_install_marker(&core.db, &core.app_data_dir, sync.registry())
        .await
        .map_err(|e| AppError::other(format!("install marker check failed: {e}")))?;
    state::fill_labels(&core.db, sync.registry(), core.directory.publish_all())
        .await
        .map_err(|e| AppError::other(format!("labels not published: {e}")))?;
    state::check_registry(&core.db, sync.registry())
        .await
        .map_err(|e| AppError::other(format!("registry check failed: {e}")))?;
    sync.mark_ready();
    Ok(())
}

/// Whether the scheduler runs a cycle on this tick: sync on, a vault
/// joined, and the interval since the last start over — measured from the
/// start so a failed cycle does not retry every tick — or a cycle held
/// while the cycles waited.
async fn due(db: &sqlx::SqlitePool, sync: &SyncManager) -> bool {
    if settings::get(db, "sync_enabled").await.as_deref() != Some("1")
        || load_binding(db).await.is_none()
    {
        return false;
    }
    if sync.is_held() {
        return true;
    }
    let interval_ms = settings::get(db, "sync_interval_sec")
        .await
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(config::DEFAULT_INTERVAL_SEC as i64)
        * 1000;
    match settings::get(db, "sync_last_started").await {
        None => true,
        Some(last) => chrono::DateTime::parse_from_rfc3339(&last)
            .map(|t| (Utc::now() - t.with_timezone(&Utc)).num_milliseconds() >= interval_ms)
            .unwrap_or(true),
    }
}

/// Background ticker. Does nothing while sync is disabled or no vault is joined.
fn start_sync_scheduler(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let core = app.state::<Core>();
        let sync = app.state::<SyncManager>();
        ensure_debug_loaded(&core, &sync).await;
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(TICK_SEC));
        let mut last_prepare = std::time::Instant::now();
        loop {
            ticker.tick().await;
            let running = sync.is_running();
            if !running {
                sync.sched_skip.store(false, Ordering::Relaxed);
            }
            if !sync.is_ready() {
                if last_prepare.elapsed() >= PREPARE_RETRY {
                    last_prepare = std::time::Instant::now();
                    if let Err(e) = prepare(&core, &sync).await {
                        eprintln!("sync: start checks failed, retrying: {e}");
                    }
                }
                continue;
            }
            let db = &core.db;
            if !due(db, &sync).await {
                continue;
            }
            if running {
                if !sync.sched_skip.swap(true, Ordering::Relaxed) {
                    trace_skip(&app, "scheduler", "scheduler", "already running");
                }
                continue;
            }
            // One that waits for the messenger is held: the first cycle of a
            // start goes the tick it is up, not an interval after a failure.
            if let Err(e) = run_cycle(&app, "scheduler").await {
                eprintln!("sync: {e}");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        register_sync_cloud_credentials, unregister_sync_cloud_credentials, CloudCredentials,
        CLOUD_BACKEND, CLOUD_SLOT_TURN,
    };
    use crate::Registry;
    use std::sync::{Arc, RwLock};
    use veydan_core::BoxFuture;
    use veydan_sync::{RequestSigner, SignFuture};
    use zeroize::Zeroizing;

    struct Signs;

    impl RequestSigner for Signs {
        fn authorization<'a>(
            &'a self,
            _: &'a str,
            _: &'a str,
            _: Option<[u8; 32]>,
            _: u64,
        ) -> SignFuture<'a> {
            Box::pin(async { Ok("Nostr e30=".to_string()) })
        }
    }

    /// The module `cloud` over a messenger that is not up while `why` is.
    struct Messenger {
        why: RwLock<Option<String>>,
    }

    impl Messenger {
        fn set(&self, why: Option<&str>) {
            *self.why.write().unwrap() = why.map(str::to_string);
        }
    }

    impl CloudCredentials for Messenger {
        fn signer(&self) -> Arc<dyn RequestSigner> {
            Arc::new(Signs)
        }
        fn refresh_hosts(&self) -> BoxFuture<'_, CmdResult<()>> {
            Box::pin(async { Ok(()) })
        }
        fn passphrase(&self) -> BoxFuture<'_, CmdResult<Zeroizing<String>>> {
            Box::pin(async { Ok(Zeroizing::new("derived".to_string())) })
        }
        fn waiting(&self) -> Option<String> {
            self.why.read().unwrap().clone()
        }
    }

    /// The order of a start on a phone (ROO-371): sync starts before the
    /// messenger. A cycle asked for then — by an edit, the background, the
    /// scheduler — waits: no start, no error, the reason told once however
    /// many times it is asked; it is held, and the scheduler runs it the
    /// tick the messenger is up, an interval since the last start or not.
    /// One switched off by the user waits the same way.
    #[tokio::test]
    async fn a_cycle_before_the_messenger_waits_and_goes_the_tick_it_is_up() {
        let _turn = CLOUD_SLOT_TURN.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let db = veydan_core::db::open(&dir.path().join("app.db"), &[veydan_core::SCHEMA])
            .await
            .unwrap();
        for (key, value) in [
            ("sync_enabled", "1"),
            ("sync_vault_id", "v1"),
            ("sync_vault_key", "a2V5"),
            ("sync_backend", CLOUD_BACKEND),
            ("sync_interval_sec", "300"),
        ] {
            settings::set(&db, key, value).await.unwrap();
        }
        let last = (Utc::now() - chrono::Duration::seconds(10)).to_rfc3339();
        settings::set(&db, "sync_last_started", &last).await.unwrap();
        settings::set(&db, "sync_last_error", "an earlier one").await.unwrap();
        let sync = SyncManager::new(Registry::new());
        let messenger = Arc::new(Messenger { why: RwLock::new(None) });
        messenger.set(Some("the messenger is starting"));
        register_sync_cloud_credentials(messenger.clone());

        assert!(!due(&db, &sync).await, "the interval is not over");
        let mut told = Vec::new();
        for _ in 0..5 {
            match begin(&db, &sync).await.unwrap() {
                Begin::Waits { why, told: line } => {
                    assert_eq!(why, "the messenger is starting");
                    told.extend(line);
                }
                _ => panic!("a cycle went before the messenger"),
            }
            assert!(!sync.is_running());
        }
        assert_eq!(told, ["waiting: the messenger is starting"], "told once");
        assert_eq!(settings::get(&db, "sync_last_started").await.as_deref(), Some(last.as_str()));
        assert_eq!(settings::get(&db, "sync_last_error").await.as_deref(), Some("an earlier one"));
        assert!(due(&db, &sync).await, "the held cycle does not wait for the interval");

        messenger.set(None);
        let Begin::Go { guard, told } = begin(&db, &sync).await.unwrap() else {
            panic!("the messenger is up and the cycle waits");
        };
        assert_eq!(told.as_deref(), Some("no longer waiting"));
        assert!(sync.is_running());
        assert_ne!(settings::get(&db, "sync_last_started").await.as_deref(), Some(last.as_str()));
        assert_eq!(settings::get(&db, "sync_last_error").await.as_deref(), Some(""));
        drop(guard);
        assert!(!due(&db, &sync).await, "the held cycle went");

        messenger.set(Some("the messenger is switched off"));
        let mut told = Vec::new();
        for _ in 0..3 {
            let Begin::Waits { told: line, .. } = begin(&db, &sync).await.unwrap() else {
                panic!("a cycle went with the messenger off");
            };
            told.extend(line);
        }
        assert_eq!(told, ["waiting: the messenger is switched off"]);
        assert!(due(&db, &sync).await);

        // Another backend never waits for the messenger.
        settings::set(&db, "sync_backend", "folder").await.unwrap();
        assert!(matches!(begin(&db, &sync).await.unwrap(), Begin::Go { .. }));
        unregister_sync_cloud_credentials();
        db.close().await;
    }
}
