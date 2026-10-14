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
//! The mark is taken twice (spec 9.3, "GC: двухфазная разметка"): the logs
//! are read again before the first delete and between batches of deletes
//! (`Pace`), and an object goes only when it is dead in every read — a
//! content-addressed blob another device took up again and referenced
//! after the first read stays. What this process uploaded or reused lately
//! (`veydan_sync::large_files::touch`) stays too, its grace started again.
//! A pass whose storage does not list the log of a device this one has read
//! (a folder a cloud client has not fully downloaded) deletes nothing. The
//! latest ops whose blobs are not all in the vault are kept in
//! `MISSING_KEY`, for the handler that can upload them again.
//!
//! Runs on desktop only; whether a phone collects too is decided once the
//! rule above has served on desktops. Status keys are still read there.
#![cfg_attr(mobile, allow(dead_code, unused_imports))]

use crate::{state, Registry};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};
use veydan_core::{settings, AppError, CmdResult};
use veydan_sync::large_files::{self, GcRoots};
use veydan_sync::{refs_from_payload, Engine, Hlc, LargeFileStore, Op, SyncError};

pub(crate) const GRACE_MS: i64 = 24 * 60 * 60 * 1000;
const INTERVAL_MS: i64 = 24 * 60 * 60 * 1000;

pub const LAST_RUN_KEY: &str = "sync_gc_last";
pub const BLOBS_TOTAL_KEY: &str = "sync_gc_blobs_total";
pub const REMOVED_KEY: &str = "sync_gc_removed";
pub const LF_TOTAL_KEY: &str = "sync_gc_lf_total";
pub const LF_REMOVED_KEY: &str = "sync_gc_lf_removed";
/// JSON list of `MissingRef`: what the last pass found referenced and gone.
pub const MISSING_KEY: &str = "sync_gc_missing";

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

/// How often a pass reads the logs again while it deletes: before the first
/// delete, then after `every` deletes or `after` since the last read,
/// whichever comes first.
#[derive(Debug, Clone, Copy)]
pub struct Pace {
    pub every: usize,
    pub after: Duration,
}

impl Default for Pace {
    fn default() -> Self {
        Self {
            every: 256,
            after: Duration::from_secs(30),
        }
    }
}

/// The latest op of an entity that names a blob of format v1 the vault no
/// longer holds, or whose blobs could not be read at all.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MissingRef {
    pub entity_type: String,
    pub entity_id: String,
    pub hlc: Hlc,
}

/// What the last pass found missing; empty before the first one.
pub async fn missing_refs(db: &Pool<Sqlite>) -> Vec<MissingRef> {
    settings::get(db, MISSING_KEY)
        .await
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
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

/// What the latest ops keep alive, by every read of the logs in one pass:
/// an object live in any of them is live.
struct Marks<'a> {
    engine: &'a Engine,
    registry: &'a Registry,
    pace: Pace,
    /// The latest op of each entity already read, by its HLC.
    seen: HashMap<(String, String), Hlc>,
    /// Blobs of format v1.
    live: BTreeSet<String>,
    /// Manifest ids of large files v2.
    roots: BTreeSet<String>,
    unknown: BTreeSet<String>,
    /// The blobs each op of the first read names; `None`: not readable.
    refs: Vec<(MissingRef, Option<Vec<String>>)>,
    /// The last read of the sweep; `None` before the first delete.
    read_at: Option<Instant>,
    since_read: usize,
}

impl<'a> Marks<'a> {
    fn new(engine: &'a Engine, registry: &'a Registry, pace: Pace) -> Self {
        Self {
            engine,
            registry,
            pace,
            seen: HashMap::new(),
            live: BTreeSet::new(),
            roots: BTreeSet::new(),
            unknown: BTreeSet::new(),
            refs: Vec::new(),
            read_at: None,
            since_read: 0,
        }
    }

    /// Reads every log and adds what its latest ops keep. Returns the first
    /// error of a handler: the op is in `refs` without blobs.
    async fn read(&mut self) -> CmdResult<Option<String>> {
        let ops = self
            .engine
            .all_latest_ops()
            .await
            .map_err(AppError::other)?;
        let known: BTreeSet<&str> = self.registry.entities().into_iter().collect();
        self.unknown.extend(
            ops.iter()
                .filter(|op| !known.contains(op.entity_type.as_str()))
                .map(|op| op.entity_type.clone()),
        );
        self.roots.extend(large_file_roots(&ops));
        if !self.unknown.is_empty() {
            return Ok(None);
        }
        let mut failed = None;
        for op in ops {
            let entity = (op.entity_type.clone(), op.entity_id.clone());
            if self.seen.get(&entity) == Some(&op.hlc) {
                continue;
            }
            let at = MissingRef {
                entity_type: op.entity_type.clone(),
                entity_id: op.entity_id.clone(),
                hlc: op.hlc.clone(),
            };
            match blob_refs(self.registry, self.engine, &op).await {
                Ok(refs) if refs.is_empty() => {}
                Ok(refs) => {
                    self.live.extend(refs.iter().cloned());
                    self.refs.push((at, Some(refs)));
                }
                Err(e) => {
                    failed.get_or_insert_with(|| e.to_string());
                    self.refs.push((at, None));
                }
            }
            self.seen.insert(entity, op.hlc);
        }
        Ok(failed)
    }

    /// Before a delete: the logs are read again when the pace says so.
    /// Returns whether they were.
    async fn before_delete(&mut self) -> CmdResult<bool> {
        let again = match self.read_at {
            None => true,
            Some(at) => at.elapsed() >= self.pace.after || self.since_read >= self.pace.every,
        };
        if again {
            if let Some(e) = self.read().await? {
                return Err(AppError::other(e));
            }
            self.read_at = Some(Instant::now());
            self.since_read = 0;
        }
        self.since_read += 1;
        Ok(again)
    }
}

#[async_trait::async_trait]
impl GcRoots for Marks<'_> {
    async fn recheck(&mut self) -> veydan_sync::Result<Option<BTreeSet<String>>> {
        match self.before_delete().await {
            Ok(true) => Ok(Some(self.roots.clone())),
            Ok(false) => Ok(None),
            Err(e) => Err(SyncError::Storage(e.to_string())),
        }
    }
}

/// The ops of the first read whose blobs are not all in `all`, for
/// `missing_refs`.
async fn record_missing(
    db: &Pool<Sqlite>,
    marks: &Marks<'_>,
    all: &BTreeSet<String>,
) -> CmdResult<()> {
    let missing: Vec<&MissingRef> = marks
        .refs
        .iter()
        .filter(|(_, refs)| match refs {
            Some(refs) => refs.iter().any(|b| !all.contains(b)),
            None => true,
        })
        .map(|(at, _)| at)
        .collect();
    let json = serde_json::to_string(&missing).map_err(AppError::other)?;
    settings::set(db, MISSING_KEY, &json).await
}

/// Every log this device has read must be in the listing: a folder a cloud
/// client has not fully downloaded hides a device, and every blob only its
/// ops name would look dead.
async fn check_listing(engine: &Engine, db: &Pool<Sqlite>) -> CmdResult<()> {
    let (local, _) = state::load_local_state(db).await?;
    let listed: BTreeSet<String> = engine
        .list_device_cards()
        .await
        .map_err(AppError::other)?
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    let mut read: Vec<&str> = local
        .peers
        .iter()
        .filter(|(_, head)| head.seq > 0)
        .map(|(id, _)| id.as_str())
        .collect();
    if local.own_seq > 0 {
        read.push(engine.device_id());
    }
    let mut absent: Vec<&str> = read
        .into_iter()
        .filter(|id| !listed.contains(*id))
        .collect();
    if absent.is_empty() {
        return Ok(());
    }
    absent.sort_unstable();
    Err(AppError::other(format!(
        "the storage lists no log of {}, which this device has read: it is not whole here, nothing is deleted",
        absent.join(", ")
    )))
}

/// One GC pass. Fails (and changes nothing) when some device's log is not fully readable.
pub async fn run(engine: &Engine, db: &Pool<Sqlite>, registry: &Registry) -> CmdResult<Outcome> {
    run_paced(engine, db, registry, Pace::default()).await
}

/// `run` with the logs read again at `pace` while it deletes.
pub async fn run_paced(
    engine: &Engine,
    db: &Pool<Sqlite>,
    registry: &Registry,
    pace: Pace,
) -> CmdResult<Outcome> {
    check_listing(engine, db).await?;
    let mut marks = Marks::new(engine, registry, pace);
    let failed = marks.read().await?;
    let all = engine.list_blobs().await.map_err(AppError::other)?;
    record_missing(db, &marks, &all).await?;
    if let Some(e) = failed {
        return Err(AppError::other(e));
    }
    marks.refs.clear();

    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT blob, first_seen FROM sync_gc_candidates")
            .fetch_all(db)
            .await
            .map_err(AppError::db)?;
    let (lf_candidates, candidates): (HashMap<String, i64>, HashMap<String, i64>) = rows
        .into_iter()
        .partition(|(k, _)| LargeFileStore::is_v2_key(k));
    let now = now_ms();
    large_files::forget_touches_before(now - GRACE_MS);
    let mut outcome = Outcome {
        blobs_total: all.len(),
        ..Default::default()
    };

    // v2 first: it is fail-closed and must not run after v1 already deleted objects.
    let store = engine
        .large_files(Default::default())
        .map_err(AppError::other)?;
    let roots = marks.roots.clone();
    let lf = store
        .gc_with_complete_root_set(&roots, &lf_candidates, now, GRACE_MS, &mut marks)
        .await
        .map_err(AppError::other)?;
    // Forgotten first: a key in both starts its grace again.
    for key in &lf.forgotten {
        forget(db, key).await?;
    }
    for (key, first_seen) in &lf.remembered {
        remember(db, key, *first_seen).await?;
    }
    outcome.lf_removed = lf.removed.len();
    outcome.lf_total = lf.manifests_total + lf.chunks_total - lf.removed.len();

    // Blobs only an op of an unknown type names would look dead: the pass
    // over them waits, its candidates as they are, for a build that knows
    // every type.
    if marks.unknown.is_empty() {
        outcome.removed = sweep(engine, db, &all, &mut marks, candidates, now).await?;
    }
    outcome.unknown = marks.unknown.iter().cloned().collect();

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
    marks: &mut Marks<'_>,
    mut candidates: HashMap<String, i64>,
    now: i64,
) -> CmdResult<usize> {
    let touched = |blob: &str| large_files::touched_since(blob, now - GRACE_MS);
    let mut due = Vec::new();
    for blob in all {
        if marks.live.contains(blob) {
            if candidates.remove(blob).is_some() {
                forget(db, blob).await?;
            }
            continue;
        }
        match candidates.remove(blob) {
            Some(_) if touched(blob) => {
                forget(db, blob).await?;
                remember(db, blob, now).await?;
            }
            Some(first_seen) if now - first_seen >= GRACE_MS => due.push(blob),
            Some(_) => {}
            None => remember(db, blob, now).await?,
        }
    }
    // Whatever is left was deleted by another device.
    for blob in candidates.keys() {
        forget(db, blob).await?;
    }
    // What was dead at the first read must be dead at the last one.
    let mut removed = 0;
    for blob in due {
        marks.before_delete().await?;
        if !marks.unknown.is_empty() {
            break;
        }
        if marks.live.contains(blob) {
            forget(db, blob).await?;
            continue;
        }
        if touched(blob) {
            forget(db, blob).await?;
            remember(db, blob, now).await?;
            continue;
        }
        engine.delete_blob(blob).await.map_err(AppError::other)?;
        forget(db, blob).await?;
        removed += 1;
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
