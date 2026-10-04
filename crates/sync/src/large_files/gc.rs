// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Mark-and-sweep over v2 manifests and chunks.
//!
//! Fail-closed: a root manifest that cannot be read aborts the pass before
//! anything is deleted. Orphans are deleted only once they were already seen
//! orphaned `grace_ms` earlier; the caller persists that ledger.

use super::crypto::{self, CHUNKS_PREFIX, MANIFESTS_PREFIX};
use super::{LargeFileRef, LargeFileStore, FORMAT_VERSION};
use crate::Result;
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Debug, Default)]
pub struct GcOutcome {
    pub manifests_total: usize,
    pub chunks_total: usize,
    /// Object keys deleted in this pass.
    pub removed: Vec<String>,
    /// New orphan candidates: (key, first_seen).
    pub remembered: Vec<(String, i64)>,
    /// Candidates that are live again or were deleted (here or elsewhere).
    pub forgotten: Vec<String>,
}

pub(super) async fn run(
    store: &LargeFileStore<'_>,
    roots: &BTreeSet<String>,
    candidates: &HashMap<String, i64>,
    now_ms: i64,
    grace_ms: i64,
) -> Result<GcOutcome> {
    let manifests = store.storage.list(MANIFESTS_PREFIX).await?;
    let chunks = store.storage.list(CHUNKS_PREFIX).await?;

    // Mark: every root manifest must open, or nothing is swept.
    let mut live: HashSet<String> = HashSet::new();
    for id in roots {
        let key = crypto::manifest_key(id);
        let bytes =
            store.storage.get(&key).await?.ok_or_else(|| {
                crate::SyncError::Integrity(format!("root manifest {id} missing"))
            })?;
        let manifest = crypto::open_manifest(store.keys, store.vault_id, id, &bytes)?;
        live.insert(key);
        live.extend(manifest.chunks.iter().map(|c| crypto::chunk_key(&c.id)));
    }

    let mut outcome = GcOutcome {
        manifests_total: manifests.len(),
        chunks_total: chunks.len(),
        ..Default::default()
    };
    let mut pending: HashMap<&String, i64> = candidates.iter().map(|(k, v)| (k, *v)).collect();

    // Sweep manifests first: a chunk of a manifest deleted now is caught next pass.
    for key in manifests.iter().chain(chunks.iter()) {
        if live.contains(key) {
            if pending.remove(key).is_some() {
                outcome.forgotten.push(key.clone());
            }
            continue;
        }
        match pending.remove(key) {
            Some(first_seen) if now_ms - first_seen >= grace_ms => {
                store.storage.delete(key).await?;
                outcome.removed.push(key.clone());
                outcome.forgotten.push(key.clone());
            }
            Some(_) => {}
            None => outcome.remembered.push((key.clone(), now_ms)),
        }
    }
    // Left-over candidates no longer exist in storage.
    outcome
        .forgotten
        .extend(pending.keys().map(|k| (*k).clone()));
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
