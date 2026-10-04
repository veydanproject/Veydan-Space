// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Blob garbage collection. A blob is live when the latest op of any entity on
//! any device references it; which blobs an op references, the handler of its
//! entity type says (`Handler::blob_refs`). Everything else is a candidate; a
//! candidate is deleted only when it was already a candidate one grace period
//! earlier, so a blob uploaded just before its op is pushed is never lost.
//! Only a build that knows every entity type in the vault can tell: while an
//! op of a type it does not register is there, the pass over blobs deletes
//! nothing (spec 9.2, rule 2) — a product without the browser would take the
//! files of profile snapshots for garbage.
//!
//! Large files v2 are swept by the same pass: the roots are the `lf_refs` of
//! the latest op of every entity type, known or not, so a client that does not
//! understand a future entity still keeps its files. Any unreadable op or
//! manifest aborts the whole pass (fail-closed). Ledger rows for v2 objects
//! store the full storage key, which tells them apart from v1 blob names.
//!
//! Runs on desktop only; whether a phone collects too is decided once the
//! rule above has served on desktops. Status keys are still read there.
#![cfg_attr(mobile, allow(dead_code, unused_imports))]

use crate::Registry;
use chrono::Utc;
use sqlx::{Pool, Sqlite};
use std::collections::{BTreeSet, HashMap};
use veydan_core::{settings, AppError, CmdResult};
use veydan_sync::{refs_from_payload, Engine, LargeFileStore, Op};

pub(crate) const GRACE_MS: i64 = 24 * 60 * 60 * 1000;
const INTERVAL_MS: i64 = 24 * 60 * 60 * 1000;

pub const LAST_RUN_KEY: &str = "sync_gc_last";
pub const BLOBS_TOTAL_KEY: &str = "sync_gc_blobs_total";
pub const REMOVED_KEY: &str = "sync_gc_removed";
pub const LF_TOTAL_KEY: &str = "sync_gc_lf_total";
pub const LF_REMOVED_KEY: &str = "sync_gc_lf_removed";

#[derive(Debug, Default)]
pub struct Outcome {
    pub blobs_total: usize,
    pub removed: usize,
    /// v2 manifests + chunks after the pass.
    pub lf_total: usize,
    pub lf_removed: usize,
    /// Entity types in the vault this build does not register; while there
    /// are any, no blob of format v1 is deleted.
    pub unknown: Vec<String>,
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// Once a day.
pub async fn due(db: &Pool<Sqlite>) -> bool {
    let last: i64 = settings::get(db, LAST_RUN_KEY)
        .await
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    now_ms() - last >= INTERVAL_MS
}

/// Blobs an op keeps alive: its handler knows. Table rows keep none.
async fn blob_refs(registry: &Registry, engine: &Engine, op: &Op) -> CmdResult<Vec<String>> {
    if op.deleted {
        return Ok(Vec::new());
    }
    match registry.handler_of(&op.entity_type) {
        Some(handler) => handler.blob_refs(engine, op).await,
        None => Ok(Vec::new()),
    }
}

/// Manifest ids every non-deleted op keeps alive, regardless of entity type.
fn large_file_roots(ops: &[Op]) -> BTreeSet<String> {
    ops.iter()
        .filter(|op| !op.deleted)
        .flat_map(|op| refs_from_payload(&op.payload))
        .collect()
}

/// One GC pass. Fails (and changes nothing) when some device's log is not fully readable.
pub async fn run(engine: &Engine, db: &Pool<Sqlite>, registry: &Registry) -> CmdResult<Outcome> {
    let ops = engine.all_latest_ops().await.map_err(AppError::other)?;
    let known: BTreeSet<&str> = registry.entities().into_iter().collect();
    let unknown: BTreeSet<String> = ops
        .iter()
        .filter(|op| !known.contains(op.entity_type.as_str()))
        .map(|op| op.entity_type.clone())
        .collect();
    let mut live: BTreeSet<String> = BTreeSet::new();
    if unknown.is_empty() {
        for op in &ops {
            live.extend(blob_refs(registry, engine, op).await?);
        }
    }
    let all = engine.list_blobs().await.map_err(AppError::other)?;

    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT blob, first_seen FROM sync_gc_candidates")
            .fetch_all(db)
            .await
            .map_err(AppError::db)?;
    let (lf_candidates, candidates): (HashMap<String, i64>, HashMap<String, i64>) = rows
        .into_iter()
        .partition(|(k, _)| LargeFileStore::is_v2_key(k));
    let now = now_ms();
    let mut outcome = Outcome {
        blobs_total: all.len(),
        ..Default::default()
    };

    // v2 first: it is fail-closed and must not run after v1 already deleted objects.
    let store = engine
        .large_files(Default::default())
        .map_err(AppError::other)?;
    let lf = store
        .gc_with_complete_root_set(&large_file_roots(&ops), &lf_candidates, now, GRACE_MS)
        .await
        .map_err(AppError::other)?;
    for (key, first_seen) in &lf.remembered {
        remember(db, key, *first_seen).await?;
    }
    for key in &lf.forgotten {
        forget(db, key).await?;
    }
    outcome.lf_removed = lf.removed.len();
    outcome.lf_total = lf.manifests_total + lf.chunks_total - lf.removed.len();

    // Blobs only an op of an unknown type names would look dead: the pass
    // over them waits, its candidates as they are, for a build that knows
    // every type.
    outcome.unknown = unknown.into_iter().collect();
    if outcome.unknown.is_empty() {
        outcome.removed = sweep(engine, db, &all, &live, candidates, now).await?;
    }

    outcome.blobs_total -= outcome.removed;
    settings::set(db, LAST_RUN_KEY, &now.to_string()).await?;
    settings::set(db, BLOBS_TOTAL_KEY, &outcome.blobs_total.to_string()).await?;
    settings::set(db, REMOVED_KEY, &outcome.removed.to_string()).await?;
    settings::set(db, LF_TOTAL_KEY, &outcome.lf_total.to_string()).await?;
    settings::set(db, LF_REMOVED_KEY, &outcome.lf_removed.to_string()).await?;
    Ok(outcome)
}

/// The pass over the blobs of format v1. Returns how many it deleted.
async fn sweep(
    engine: &Engine,
    db: &Pool<Sqlite>,
    all: &BTreeSet<String>,
    live: &BTreeSet<String>,
    mut candidates: HashMap<String, i64>,
    now: i64,
) -> CmdResult<usize> {
    let mut removed = 0;
    for blob in all {
        if live.contains(blob) {
            if candidates.remove(blob).is_some() {
                forget(db, blob).await?;
            }
            continue;
        }
        match candidates.remove(blob) {
            Some(first_seen) if now - first_seen >= GRACE_MS => {
                engine.delete_blob(blob).await.map_err(AppError::other)?;
                forget(db, blob).await?;
                removed += 1;
            }
            Some(_) => {}
            None => remember(db, blob, now).await?,
        }
    }
    // Whatever is left was deleted by another device.
    for blob in candidates.keys() {
        forget(db, blob).await?;
    }
    Ok(removed)
}

async fn remember(db: &Pool<Sqlite>, blob: &str, first_seen: i64) -> CmdResult<()> {
    sqlx::query("INSERT OR IGNORE INTO sync_gc_candidates (blob, first_seen) VALUES (?, ?)")
        .bind(blob)
        .bind(first_seen)
        .execute(db)
        .await
        .map_err(AppError::db)?;
    Ok(())
}

/// Start the grace periods over: the data whose blobs were candidates is
/// gone from this device, as when the demo data replaces everything. The
/// positions in the logs and the states of rows stay, so the next cycle
/// publishes the tombstones of what disappeared.
pub async fn forget_candidates(db: &Pool<Sqlite>) -> CmdResult<()> {
    sqlx::query("DELETE FROM sync_gc_candidates")
        .execute(db)
        .await
        .map_err(AppError::db)?;
    Ok(())
}

async fn forget(db: &Pool<Sqlite>, blob: &str) -> CmdResult<()> {
    sqlx::query("DELETE FROM sync_gc_candidates WHERE blob = ?")
        .bind(blob)
        .execute(db)
        .await
        .map_err(AppError::db)?;
    Ok(())
}
