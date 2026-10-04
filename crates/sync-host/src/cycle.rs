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
    Applied, Collected, Commit, ConflictInfo, Cycle, Handler, LeaseInfo, Part, Step, Table,
};
use crate::rows::{self, ApplyOutcome};
use crate::state::RowSyncState;
use crate::{emit_progress, gc, join, progress_pct, state, Host, RunningGuard};
use crate::{SyncManager, EVENT_STATUS};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager};
use veydan_core::{settings, AppError, CmdResult, Core};
use veydan_sync::{Engine, HlcClock, LocalState, Op, Vmk};

const TICK_SEC: u64 = 1;
const COMPACT_AFTER_CHUNKS: usize = 64;
const TOMBSTONE_TTL_MS: u64 = 180 * 24 * 60 * 60 * 1000;

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
        if let Err(e) = run_cycle(&app, &source).await {
            eprintln!("sync: {e}");
        }
    });
}

/// push local changes -> pull peers -> apply/merge -> compact -> persist.
pub async fn run_cycle(app: &AppHandle, source: &str) -> CmdResult<()> {
    let core = app.state::<Core>();
    let sync = app.state::<SyncManager>();
    if sync.running.swap(true, Ordering::SeqCst) {
        trace_skip(app, source, "start", "already running");
        return Err(AppError::other("sync already running"));
    }
    let _guard = RunningGuard(&sync.running);
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
    settings::set(db, "sync_last_started", &Utc::now().to_rfc3339()).await?;
    settings::set(db, "sync_last_error", "").await?;
    // Let the UI show the running state for scheduled cycles too.
    let _ = app.emit(EVENT_STATUS, ());

    emit_progress(app, "collect", 0, 0, 0, "");
    let result = cycle(Host::App(app), &core, &sync).await;
    let now = Utc::now().to_rfc3339();
    match &result {
        Ok(warnings) => {
            settings::set(db, "sync_last_run", &now).await?;
            settings::set(db, "sync_last_error", "").await?;
            settings::set(db, "sync_last_warning", &warnings.join("; ")).await?;
            emit_progress(app, "done", 100, 0, 0, "");
            trace(app, "info", "done", "cycle finished");
        }
        Err(e) => {
            settings::set(db, "sync_last_error", &e.to_string()).await?;
            emit_progress(app, "error", 0, 0, 0, &e.to_string());
            trace(app, "error", "done", &e.to_string());
        }
    }
    let _ = app.emit(EVENT_STATUS, ());
    result.map(|_| ())
}

/// Blob missing for an op older than the GC grace: it was collected, not delayed.
pub fn blob_gone(op: &Op, now_ms: u64) -> bool {
    now_ms.saturating_sub(op.hlc.wall_ms) >= gc::GRACE_MS as u64
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
pub async fn push_error(db: &sqlx::SqlitePool, e: veydan_sync::SyncError) -> AppError {
    if let veydan_sync::SyncError::OwnLogCollision(_) = e {
        if let Err(re) = config::rotate_device_id(db).await {
            return re;
        }
        let _ = settings::set(
            db,
            "sync_last_warning",
            "device id rotated after log collision",
        )
        .await;
        return AppError::other(
            "own log collision: device id rotated, changes are pushed on the next cycle",
        );
    }
    AppError::other(e)
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
pub struct Gathered {
    /// In the order of the plan's steps.
    pub ops: Vec<Op>,
    /// What the ops stand for, to save once the push went through.
    pub saves: Saves,
    pub pending: Vec<Pending>,
    /// `<label> <number of ops>` per step, for the developer log.
    pub counts: Vec<String>,
}

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
    Ok(Reread {
        ops,
        read: all
            .peers
            .into_iter()
            .filter(|peer| peer.error.is_none() && !done.contains(&peer.device_id))
            .map(|peer| peer.device_id)
            .collect(),
        errors: all.errors,
    })
}

/// What a second read of the vault for a new registry brought.
struct Reread {
    ops: Vec<Op>,
    /// The logs read in full this time.
    read: Vec<String>,
    /// The logs that could not be read.
    errors: Vec<(String, String)>,
}

/// One cycle over the states `host` reaches: what `run_cycle` runs in the
/// app, and what a test runs without it. Returns non-fatal warnings
/// (per-peer integrity problems).
pub async fn cycle(host: Host<'_>, core: &Core, sync: &SyncManager) -> CmdResult<Vec<String>> {
    let db = &core.db;
    let registry = sync.registry();
    let plan = registry.plan();
    let cfg = load_config(db).await;
    let engine = open_engine(core).await?;
    engine.verify_manifest().await.map_err(AppError::other)?;

    let (mut local, last_hlc) = state::load_local_state(db).await?;
    let mut clock = HlcClock::new(engine.device_id().to_string(), last_hlc.as_ref());

    let mut warnings: Vec<String> = Vec::new();
    // A key row made since the join was not compared with the vault's yet.
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
    let Gathered {
        ops,
        saves,
        pending,
        counts,
    } = collect(&mut cx).await?;
    log(&host, "info", "collect", &counts.join(", "));
    progress(&host, "push", 22, 0, 0, "");
    log(&host, "info", "push", &format!("{} ops", ops.len()));
    if !ops.is_empty() {
        if let Some(w) = engine
            .rewind_own_log(&mut local)
            .await
            .map_err(AppError::other)?
        {
            log(&host, "info", "push", &w);
            warnings.push(w);
        }
        if let Err(e) = engine.push(&mut local, ops).await {
            if matches!(e, veydan_sync::SyncError::OwnLogCollision(_)) {
                log(
                    &host,
                    "error",
                    "push",
                    "own log collision, rotating device id",
                );
            } else {
                log(&host, "error", "push", &e.to_string());
            }
            return Err(push_error(db, e).await);
        }
        state::save_own_state(db, &local, &clock.last()).await?;
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
        let again = skipped_before(&engine, &pulled.ops, &known, &done, &local)
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
        if !again.ops.is_empty() {
            pulled.ops.extend(again.ops);
            pulled.ops.sort_by(|a, b| a.hlc.cmp(&b.hlc));
        }
        reread = Some((again.read, again.errors.is_empty()));
    }
    for op in &pulled.ops {
        clock.observe(&op.hlc);
    }
    // Persist the advanced clock now: a failure while applying must not let the
    // next cycle stamp ops with an HLC below what it has already seen.
    state::save_own_state(db, &local, &clock.last()).await?;
    settings::set(db, "sync_last_applied", &pulled.ops.len().to_string()).await?;
    // A parent row missing on this device is not fatal: the ops come again next cycle.
    let mut deferred: Option<String> = None;
    let mut retries: Vec<(&str, Option<String>)> = Vec::new();
    let mut applied: Vec<(&dyn Handler, Applied)> = Vec::new();
    let mut streams: Vec<Stream> = Vec::new();
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
                let outcome = fk_retry(
                    step.label,
                    handler.apply(&mut cx, &pulled.ops).await,
                    &mut deferred,
                )?;
                fk_retry(
                    finish.unwrap_or(step.label),
                    handler.finish(&mut cx, &outcome).await,
                    &mut deferred,
                )?;
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
                    rows::apply_ops(&host, core, &tables, &pulled.ops).await,
                    &mut deferred,
                )?;
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
    let retry = retries
        .into_iter()
        .find_map(|(_, retry)| retry)
        .or(late_retry)
        .or(deferred);
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
            let line = format!("retry next cycle: {reason}");
            log(&host, "retry", "done", &line);
            warnings.push(line);
        }
    }

    if engine.own_chunk_count().await.map_err(AppError::other)? > COMPACT_AFTER_CHUNKS {
        progress(&host, "compact", 94, 0, 0, "");
        log(&host, "info", "compact", "compacting");
        let now_ms = Utc::now().timestamp_millis().max(0) as u64;
        engine
            .compact(&local, now_ms, TOMBSTONE_TTL_MS)
            .await
            .map_err(|e| {
                log(&host, "error", "compact", &e.to_string());
                AppError::other(e)
            })?;
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
pub fn start(app: &AppHandle) {
    let core = app.state::<Core>();
    let sync = app.state::<SyncManager>();
    let marker = config::check_install_marker(&core.db, &core.app_data_dir);
    if let Err(e) = tauri::async_runtime::block_on(marker) {
        eprintln!("sync: install marker check failed: {e}");
    }
    let fill = state::fill_labels(&core.db, sync.registry(), core.directory.publish_all());
    if let Err(e) = tauri::async_runtime::block_on(fill) {
        eprintln!("sync: labels not published: {e}");
    }
    let registry = state::check_registry(&core.db, sync.registry());
    if let Err(e) = tauri::async_runtime::block_on(registry) {
        eprintln!("sync: registry check failed: {e}");
    }
    start_sync_scheduler(app.clone());
}

/// Background ticker. Does nothing while sync is disabled or no vault is joined.
fn start_sync_scheduler(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let core = app.state::<Core>();
        let sync = app.state::<SyncManager>();
        ensure_debug_loaded(&core, &sync).await;
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(TICK_SEC));
        loop {
            ticker.tick().await;
            let running = sync.is_running();
            if !running {
                sync.sched_skip.store(false, Ordering::Relaxed);
            }
            let db = &core.db;
            if settings::get(db, "sync_enabled").await.as_deref() != Some("1")
                || load_binding(db).await.is_none()
            {
                continue;
            }
            let interval_ms = settings::get(db, "sync_interval_sec")
                .await
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(config::DEFAULT_INTERVAL_SEC as i64)
                * 1000;
            // Measured from the last cycle start so a failed cycle does not retry every tick.
            let due = match settings::get(db, "sync_last_started").await {
                None => true,
                Some(last) => chrono::DateTime::parse_from_rfc3339(&last)
                    .map(|t| (Utc::now() - t.with_timezone(&Utc)).num_milliseconds() >= interval_ms)
                    .unwrap_or(true),
            };
            if !due {
                continue;
            }
            if running {
                if !sync.sched_skip.swap(true, Ordering::Relaxed) {
                    trace_skip(&app, "scheduler", "scheduler", "already running");
                }
                continue;
            }
            if let Err(e) = run_cycle(&app, "scheduler").await {
                eprintln!("sync: {e}");
            }
        }
    });
}
