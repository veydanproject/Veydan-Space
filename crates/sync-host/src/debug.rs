// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The developer sync log: a toggle kept in the settings and a ring of lines
//! in memory that the developer page reads.

use crate::SyncManager;
use chrono::Utc;
use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use veydan_core::{settings, Core};

const DEBUG_CAP: usize = 400;
pub(crate) const DEBUG_SETTING: &str = "sync_debug_log";

/// One line of the developer sync log.
#[derive(Debug, Clone, Serialize)]
pub struct SyncDebugEntry {
    pub seq: u64,
    pub at: String,
    pub cycle: u64,
    pub source: String,
    pub level: String,
    pub step: String,
    pub message: String,
}

/// Toggle plus the in-memory ring the developer page reads.
#[derive(Debug, Clone, Serialize)]
pub struct SyncDebugState {
    pub enabled: bool,
    pub entries: Vec<SyncDebugEntry>,
}

#[derive(Debug, Default)]
struct DebugLog {
    seq: u64,
    cycle: u64,
    source: String,
    entries: VecDeque<SyncDebugEntry>,
}

/// The log as the sync manager keeps it.
#[derive(Default)]
pub(crate) struct Debug {
    enabled: AtomicBool,
    /// True after the setting has been copied into `enabled`.
    loaded: Mutex<bool>,
    log: Mutex<DebugLog>,
}

impl Debug {
    pub(crate) fn state(&self) -> SyncDebugState {
        let enabled = self.enabled.load(Ordering::Relaxed);
        let entries = self
            .log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .iter()
            .cloned()
            .collect();
        SyncDebugState { enabled, entries }
    }

    /// The choice of the user; it stands from now on, the setting no longer counts.
    pub(crate) fn set_enabled(&self, enabled: bool) {
        let mut loaded = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
        self.enabled.store(enabled, Ordering::Relaxed);
        *loaded = true;
    }

    pub(crate) fn clear(&self) {
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .clear();
    }
}

/// Copy `sync_debug_log` into memory once. Later toggles update the flag directly.
pub(crate) async fn ensure_debug_loaded(core: &Core, sync: &SyncManager) {
    {
        let loaded = sync.debug.loaded.lock().unwrap_or_else(|e| e.into_inner());
        if *loaded {
            return;
        }
    }
    let on = settings::get(&core.db, DEBUG_SETTING).await.as_deref() == Some("1");
    let mut loaded = sync.debug.loaded.lock().unwrap_or_else(|e| e.into_inner());
    if *loaded {
        return;
    }
    sync.debug.enabled.store(on, Ordering::Relaxed);
    *loaded = true;
}

pub(crate) fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

/// Counts of pulled ops by entity type. Empty input is `0 ops`.
pub(crate) fn op_summary(ops: &[veydan_sync::Op]) -> String {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for op in ops {
        *counts.entry(op.entity_type.as_str()).or_default() += 1;
    }
    if counts.is_empty() {
        return "0 ops".into();
    }
    counts
        .into_iter()
        .map(|(kind, n)| format!("{kind} {n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn debug_on(app: &AppHandle) -> bool {
    app.state::<SyncManager>()
        .debug
        .enabled
        .load(Ordering::Relaxed)
}

/// Remember which cycle the following `trace` lines belong to.
pub(crate) fn begin_debug_cycle(app: &AppHandle, source: &str) {
    if !debug_on(app) {
        return;
    }
    let sync = app.state::<SyncManager>();
    let mut log = sync.debug.log.lock().unwrap_or_else(|e| e.into_inner());
    log.cycle += 1;
    log.source = source.to_string();
}

/// Append a line for the current cycle. No-op while the developer log is off.
pub fn trace(app: &AppHandle, level: &str, step: &str, message: &str) {
    if !debug_on(app) {
        return;
    }
    let sync = app.state::<SyncManager>();
    let (cycle, source) = {
        let log = sync.debug.log.lock().unwrap_or_else(|e| e.into_inner());
        (log.cycle, log.source.clone())
    };
    push_debug(app, level, step, &source, cycle, message);
}

/// Append a line that is not part of a cycle (a skipped start).
pub fn trace_skip(app: &AppHandle, source: &str, step: &str, message: &str) {
    push_debug(app, "skip", step, source, 0, message);
}

fn push_debug(app: &AppHandle, level: &str, step: &str, source: &str, cycle: u64, message: &str) {
    if !debug_on(app) {
        return;
    }
    let sync = app.state::<SyncManager>();
    let entry = {
        let mut log = sync.debug.log.lock().unwrap_or_else(|e| e.into_inner());
        log.seq += 1;
        let entry = SyncDebugEntry {
            seq: log.seq,
            at: Utc::now().to_rfc3339(),
            cycle,
            source: source.to_string(),
            level: level.to_string(),
            step: step.to_string(),
            message: message.to_string(),
        };
        if log.entries.len() >= DEBUG_CAP {
            log.entries.pop_front();
        }
        log.entries.push_back(entry.clone());
        entry
    };
    let _ = app.emit(crate::EVENT_DEBUG, entry);
}
