// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Mark-and-sweep over v2 manifests and chunks.
//!
//! Fail-closed: a root manifest that cannot be read aborts the pass before
//! anything is deleted. Orphans are deleted only once they were already seen
//! orphaned `grace_ms` earlier; the caller persists that ledger.
//!
//! The mark is taken more than once. Before the first delete, and again
//! between batches of deletes, the sweep asks the caller for the roots as
//! the vault holds them now (`GcRoots::recheck`) and opens the new ones: an
//! object goes only when no read found it live, so a reference another
//! device published since the first read keeps what it names. An object
//! this process wrote or found in place for a reference of its own
//! recently (`touch`) is not deleted either: its grace starts again.

use super::crypto::{self, CHUNKS_PREFIX, MANIFESTS_PREFIX};
use super::{LargeFileRef, LargeFileStore, FORMAT_VERSION};
use crate::Result;
use async_trait::async_trait;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Mutex;

#[derive(Debug, Default)]
pub struct GcOutcome {
    pub manifests_total: usize,
    pub chunks_total: usize,
    /// Object keys deleted in this pass.
    pub removed: Vec<String>,
    /// New orphan candidates: (key, first_seen). Applied after `forgotten`,
    /// so a key in both starts its grace again.
    pub remembered: Vec<(String, i64)>,
    /// Candidates that are live again or were deleted (here or elsewhere).
    pub forgotten: Vec<String>,
}

/// The roots of a sweep, read again while it deletes.
#[async_trait]
pub trait GcRoots: Send {
    /// Asked before every delete. `Some` with every live manifest id of the
    /// vault read just now (more than were passed before is fine), `None`
    /// while the last read is recent enough.
    async fn recheck(&mut self) -> Result<Option<BTreeSet<String>>>;
}

/// When this process last wrote an object or took one already in place for
/// a reference of its own: v1 blob names and v2 storage keys, as the ledger
/// of the GC keys them. Kept in memory only; it covers a GC of this device
/// running while an upload of its own has not published its op yet.
static TOUCHED: Mutex<BTreeMap<String, i64>> = Mutex::new(BTreeMap::new());

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// This process uses `key` now: a GC pass of this device keeps it and starts
/// its grace again.
pub fn touch(key: &str) {
    if let Ok(mut touched) = TOUCHED.lock() {
        touched.insert(key.to_string(), now_ms());
    }
}

/// Whether this process touched `key` at `since` or later.
pub fn touched_since(key: &str, since: i64) -> bool {
    TOUCHED
        .lock()
        .map(|t| t.get(key).is_some_and(|at| *at >= since))
        .unwrap_or(false)
}

/// Forget the touches older than `before`.
pub fn forget_touches_before(before: i64) {
    if let Ok(mut touched) = TOUCHED.lock() {
        touched.retain(|_, at| *at >= before);
    }
}

/// Opens every root not opened yet; its manifest and chunks are live.
async fn mark(
    store: &LargeFileStore<'_>,
    roots: &BTreeSet<String>,
    opened: &mut HashSet<String>,
    live: &mut HashSet<String>,
) -> Result<()> {
    for id in roots {
        if opened.contains(id) {
            continue;
        }
        let key = crypto::manifest_key(id);
        let bytes =
            store.storage.get(&key).await?.ok_or_else(|| {
                crate::SyncError::Integrity(format!("root manifest {id} missing"))
            })?;
        let manifest = crypto::open_manifest(store.keys, store.vault_id, id, &bytes)?;
        live.insert(key);
        live.extend(manifest.chunks.iter().map(|c| crypto::chunk_key(&c.id)));
        opened.insert(id.clone());
    }
    Ok(())
}

pub(super) async fn run(
    store: &LargeFileStore<'_>,
    roots: &BTreeSet<String>,
    candidates: &HashMap<String, i64>,
    now_ms: i64,
    grace_ms: i64,
    again: &mut dyn GcRoots,
) -> Result<GcOutcome> {
    let manifests = store.storage.list(MANIFESTS_PREFIX).await?;
    let chunks = store.storage.list(CHUNKS_PREFIX).await?;

    // Mark: every root manifest must open, or nothing is swept.
    let mut live: HashSet<String> = HashSet::new();
    let mut opened: HashSet<String> = HashSet::new();
    mark(store, roots, &mut opened, &mut live).await?;

    let mut outcome = GcOutcome {
        manifests_total: manifests.len(),
        chunks_total: chunks.len(),
        ..Default::default()
    };
    let mut pending: HashMap<&String, i64> = candidates.iter().map(|(k, v)| (k, *v)).collect();
    let touched_since_grace = |key: &str| touched_since(key, now_ms - grace_ms);

    // Manifests first: a chunk of a manifest deleted now is caught next pass.
    let mut due: Vec<&String> = Vec::new();
    for key in manifests.iter().chain(chunks.iter()) {
        if live.contains(key) {
            if pending.remove(key).is_some() {
                outcome.forgotten.push(key.clone());
            }
            continue;
        }
        match pending.remove(key) {
            Some(_) if touched_since_grace(key) => {
                outcome.forgotten.push(key.clone());
                outcome.remembered.push((key.clone(), now_ms));
            }
            Some(first_seen) if now_ms - first_seen >= grace_ms => due.push(key),
            Some(_) => {}
            None => outcome.remembered.push((key.clone(), now_ms)),
        }
    }
    // Left-over candidates no longer exist in storage.
    outcome
        .forgotten
        .extend(pending.keys().map(|k| (*k).clone()));

    // Sweep: what was dead at the first read must be dead at the last one.
    for key in due {
        if let Some(roots) = again.recheck().await? {
            mark(store, &roots, &mut opened, &mut live).await?;
        }
        if live.contains(key) {
            outcome.forgotten.push(key.clone());
            continue;
        }
        if touched_since_grace(key) {
            outcome.forgotten.push(key.clone());
            outcome.remembered.push((key.clone(), now_ms));
            continue;
        }
        store.storage.delete(key).await?;
        outcome.removed.push(key.clone());
        outcome.forgotten.push(key.clone());
    }
    Ok(outcome)
}

/// Manifest ids in an op payload's `lf_refs`, or from a nested `LargeFileRef`.
pub fn refs_from_payload(payload: &serde_json::Value) -> Vec<String> {
    let mut out: Vec<String> = payload
        .get(super::LF_REFS_FIELD)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if let Some(r) = payload
        .get("lf")
        .and_then(|v| serde_json::from_value::<LargeFileRef>(v.clone()).ok())
    {
        if r.version == FORMAT_VERSION && !out.contains(&r.manifest_id) {
            out.push(r.manifest_id);
        }
    }
    out
}
