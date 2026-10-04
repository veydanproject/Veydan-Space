// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The sync of a product: entities replicated through an E2E-encrypted vault
//! that lives in a plain folder, an S3 bucket or a WebDAV collection.
//!
//! - `registry` — what the modules sync and the order the product gives
//! - `rows`     — table rows as sync entities (LWW), for every registered table
//! - `join`     — binding to a vault; the vault's key row of the lock
//! - `config`   — settings + storage adapter factory
//! - `state`    — persistence of log positions and per-row state
//! - `gc`       — blob garbage collection
//! - `commands` — the `sync_*` commands; the shell routes them
//!
//! The crate knows no table of a module: the modules register their
//! entities in the [`Registry`], the system entities — the key row of the
//! lock and the shared settings — are registered here. Nothing runs unless
//! `sync_enabled` is "1" and a vault was joined.

pub mod commands;
pub mod config;
mod cycle;
mod debug;
mod fs_hash;
pub mod gc;
pub mod join;
mod registry;
pub mod rows;
pub mod state;
mod system;

pub use config::{check_install_marker, SyncConfig};
pub use cycle::{
    blob_gone, collect, cycle, open_engine, push_error, run_cycle, start, status, trigger_cycle,
    vault_active, Gathered, Pending, Saves, StorageDevice, SyncStatus,
};
pub use debug::{debug_on, trace, trace_skip, SyncDebugEntry, SyncDebugState};
pub use fs_hash::HashCache;
pub use registry::{
    AfterApply, AfterRow, Applied, BeforeUpsert, Collected, Commit, ConflictInfo, Conflicts, Cycle,
    Delete, Deletion, Handler, Hooks, LeaseInfo, Leases, LinkSpec, OnDelete, Part, Plan,
    PublishNew, Ref, Registry, Sanitize, StatusParts, Step, Table, TableSpec, Twin, Upsert,
};
pub use rows::EVENT_CHANGED;
pub use system::{LABEL_ENTITY, SETTING_ENTITY, VAULT_KEY_ENTITY};

use serde::Serialize;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use veydan_core::Schema;
use veydan_sync::CancelFlag;

pub const EVENT_STATUS: &str = "sync://status";
pub const EVENT_PROGRESS: &str = "sync://progress";
pub const EVENT_DEBUG: &str = "sync://debug";

/// The sync loop's own tables (unused while sync is disabled).
pub const SCHEMA: Schema = Schema {
    module: "sync",
    steps: &[concat!(
        // Last verified position in each peer device's log.
        "CREATE TABLE sync_peers (
            device_id TEXT PRIMARY KEY NOT NULL,
            seq       INTEGER NOT NULL DEFAULT 0,
            head_hash TEXT NOT NULL DEFAULT ''
        );",
        // Per-row sync position for table entities (profiles, proxies, ...).
        "CREATE TABLE sync_row_state (
            entity_type TEXT NOT NULL,
            entity_id   TEXT NOT NULL,
            head_hlc    TEXT NOT NULL DEFAULT '',
            synced_hash TEXT NOT NULL DEFAULT '',
            deleted     INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (entity_type, entity_id)
        );",
        // Unreferenced vault blobs and when they were first seen; deleted after a grace period.
        "CREATE TABLE sync_gc_candidates (
            blob       TEXT PRIMARY KEY NOT NULL,
            first_seen INTEGER NOT NULL
        );",
    )],
};

/// The states the setups of the modules put in place, without the app: what
/// the hooks and handlers of a test reach.
pub trait States: Send + Sync {
    fn state(&self, ty: TypeId) -> Option<&(dyn Any + Send + Sync)>;
}

/// Where hooks and handlers find the states of the modules: the running app,
/// or the states a test made without it. Events go to the app only.
#[derive(Clone, Copy)]
pub enum Host<'a> {
    App(&'a AppHandle),
    States(&'a dyn States),
}

impl<'a> Host<'a> {
    pub fn state<T: Send + Sync + 'static>(&self) -> Option<&'a T> {
        match *self {
            Host::App(app) => app.try_state::<T>().map(|s| s.inner()),
            Host::States(states) => states.state(TypeId::of::<T>())?.downcast_ref(),
        }
    }

    pub fn app(&self) -> Option<&'a AppHandle> {
        match *self {
            Host::App(app) => Some(app),
            Host::States(_) => None,
        }
    }

    pub fn emit<S: Serialize + Clone>(&self, event: &str, payload: S) {
        if let Host::App(app) = self {
            let _ = app.emit(event, payload);
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncProgress {
    pub phase: String,
    pub percent: u32,
    pub current: u32,
    pub total: u32,
    pub detail: String,
}

pub fn emit_progress(
    app: &AppHandle,
    phase: &str,
    percent: u32,
    current: u32,
    total: u32,
    detail: &str,
) {
    let _ = app.emit(
        EVENT_PROGRESS,
        SyncProgress {
            phase: phase.to_string(),
            percent: percent.min(100),
            current,
            total,
            detail: detail.to_string(),
        },
    );
}

pub fn progress_pct(lo: u32, hi: u32, current: u32, total: u32) -> u32 {
    if total == 0 || hi <= lo {
        return lo;
    }
    lo + (hi - lo).saturating_mul(current) / total
}

/// Guards against overlapping cycles (manual + scheduled). Holds what the
/// modules of the product registered.
pub struct SyncManager {
    running: AtomicBool,
    /// File hashes keyed by path, valid while mtime and size match.
    pub file_hashes: HashCache,
    /// Cancel flags of large-file transfers in flight, keyed by `note_id/name`.
    transfers: Mutex<HashMap<String, CancelFlag>>,
    debug: debug::Debug,
    /// Scheduler already reported "already running" for the current overlap.
    sched_skip: AtomicBool,
    registry: Registry,
}

impl SyncManager {
    pub fn new(registry: Registry) -> Self {
        Self {
            running: AtomicBool::new(false),
            file_hashes: HashCache::default(),
            transfers: Mutex::new(HashMap::new()),
            debug: debug::Debug::default(),
            sched_skip: AtomicBool::new(false),
            registry,
        }
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// A cycle runs, or the slot is held by `pause`.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Register a transfer; the returned flag is what the UI can cancel.
    pub fn begin_transfer(&self, key: &str) -> CancelFlag {
        let flag = CancelFlag::default();
        self.transfers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.to_string(), flag.clone());
        flag
    }

    pub fn end_transfer(&self, key: &str) {
        self.transfers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
    }

    /// Ask a running transfer to stop; returns whether one was in flight.
    pub fn cancel_transfer(&self, key: &str) -> bool {
        match self
            .transfers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
        {
            Some(flag) => {
                flag.cancel();
                true
            }
            None => false,
        }
    }

    /// Hold the cycle slot so a bulk data rewrite cannot interleave with a sync.
    /// Waits up to `max_wait` for a running cycle to finish.
    pub async fn pause(&self, max_wait: std::time::Duration) -> Option<RunningGuard<'_>> {
        let deadline = std::time::Instant::now() + max_wait;
        loop {
            if !self.running.swap(true, Ordering::SeqCst) {
                return Some(RunningGuard(&self.running));
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }
}

pub struct RunningGuard<'a>(&'a AtomicBool);
impl Drop for RunningGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
