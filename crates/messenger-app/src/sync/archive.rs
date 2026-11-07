// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The handler of the archive: segments (`messenger_segment`) here, the
//! secrets of groups (`messenger_secret`) in `secrets`.
//!
//! A segment travels as a large file v2, like `note_attachment_v2`: its op
//! carries the summary of the segment, the reference of the file (`lf`) and
//! its manifest in `lf_refs`, which keeps the file alive in the collection
//! of every client, 4.0.7 included (platform-spec 9.5, 9.6):
//!
//! ```json
//! { "owner": "<hex key>", "scope": "group:ab12", "month": "2026-10", "count": 412,
//!   "first_at": 1790812800, "last_at": 1793491100, "size": 183204,
//!   "hash": "<sha256 of the plaintext file>",
//!   "lf": { "version": 2, "manifest_id": "…", "size": 183204, "file_id": "…" },
//!   "lf_refs": ["…"] }
//! ```
//!
//! The entity id is `<owner>/<scope>/<yyyy-mm>`; an op of another owner's
//! archive is kept in the table and never queued (a storage two identities
//! share). A tombstone (the segment of a scope removed) is an op with
//! `deleted` and the payload `{}`.
//!
//! Writing (collect): every segment the archive says is due is built, or —
//! when the storage holds a version this device has not merged — that
//! version is merged first (from what is held when this device read it
//! already, else downloaded), its own events replayed (D7); the union goes
//! up only when it is more than the storage holds (§4.2). A cycle moves at
//! most [`BUDGET_SEGMENTS`] segments or [`BUDGET_BYTES`] bytes of the
//! archive, so the other types of the vault never wait long for it.
//! Reading (apply): every newer op is recorded and queued; the queue is
//! downloaded and replayed by a background task (`drain`), which the cycle
//! kicks and which takes no slot of the cycle, newest months first, the
//! open chat before all.
//!
//! A storage that refuses writes — `413 quota`, `403 read_only` of a Cloud
//! account in grace or suspended — pauses the writing for an hour, one that
//! does not answer until the next cycle; reading goes on; the cycle and the
//! other types never fail for it (plan §4.5, decision 4).

use super::{idle, now, secrets, ArchiveStore, ArchiveSync, Built, Enqueued, SEGMENT_STATE};
use messenger_archive::{
    is_format_error, is_parked_reason, is_too_large, too_large, ArchivePhase, ArchiveProgress, ArchiveScope, PauseReason, QueueItem,
    SegmentId, SegmentMeta, CHUNK_SIZE, ENTITY_SECRET, ENTITY_SEGMENT, FORMAT_VERSION, MAX_SEGMENT_BYTES,
};
use messenger_core::{MessengerError, PubKey, Timestamp};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::AppHandle;
use veydan_core::{settings, AppError, CmdResult, Core};
use veydan_sync::{
    sha256_hex, Engine, Hlc, LargeFileConfig, LargeFileRef, LargeFileSource, LargeFileStore, Op, PathSink,
    Progress, SyncError,
};
use veydan_sync_host::{emit_progress, Applied, Collected, Cycle, Handler, SyncConfig, SyncManager};

/// Chunks of a segment in flight at once.
const PARALLELISM: usize = 3;
/// The app setting holding the vault and the owner the segments of this
/// device were written to and for (`<vault id>|<hex key>`): another one of
/// either gets them all again, and nothing of the one before is reused.
const WRITTEN_TO_KEY: &str = "messenger_archive_vault";
/// The folder of the messenger's data where a download is staged; a
/// download stopped half way goes on from there (large files v2 resume).
const STAGING: &str = "archive-staging";
/// The app setting holding when the closed segments were last compacted
/// (D5), unix seconds.
const COMPACTED_AT_KEY: &str = "messenger_archive_compacted_at";
/// The closed segments are compacted this often.
const COMPACT_EVERY_SECS: i64 = 30 * 86400;
/// What one collect moves of the archive at most: segments, and bytes up
/// or down. The rest stays due for the next cycle.
pub(crate) const BUDGET_SEGMENTS: u32 = 5;
pub(crate) const BUDGET_BYTES: u64 = 20 * 1024 * 1024;
/// Rounds of the restore in one drain: a round that replayed something
/// may have brought the welcome a parked group waited for.
const DRAIN_PASSES: u32 = 4;

/// The large files of the archive: 1 MiB chunks, so that appending to a
/// month changes its last chunk alone (plan §4.2).
fn lf_config() -> LargeFileConfig {
    LargeFileConfig { chunk_size: CHUNK_SIZE, parallelism: PARALLELISM, resume: true }
}

/// The build that reads or refuses a file: the app's version and the
/// version of the format. A refused segment is tried again by another.
pub(crate) fn build_of(app: Option<&AppHandle>) -> String {
    match app {
        Some(app) => format!("{}/{FORMAT_VERSION}", app.package_info().version),
        None => format!("dev/{FORMAT_VERSION}"),
    }
}

// ── Payload ──────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SegmentPayload {
    #[serde(flatten)]
    pub meta: SegmentMeta,
    pub lf: LargeFileRef,
    pub lf_refs: Vec<String>,
}

impl SegmentPayload {
    fn new(meta: SegmentMeta, lf: LargeFileRef) -> Self {
        Self { meta, lf_refs: vec![lf.manifest_id.clone()], lf }
    }

    /// The payload names the segment of its id, owner included, and keeps
    /// its own file.
    fn is_for(&self, seg: &SegmentId) -> bool {
        self.meta.owner == seg.owner.as_hex()
            && self.meta.scope == seg.scope.as_key()
            && self.meta.month == seg.month.to_string()
            && self.lf_refs == [self.lf.manifest_id.clone()]
            && self.lf.size == self.meta.size
            && !self.meta.hash.is_empty()
    }
}

fn segment_op(entity_id: &str, hlc: Hlc, payload: SegmentPayload) -> Op {
    Op {
        entity_type: ENTITY_SEGMENT.into(),
        entity_id: entity_id.into(),
        hlc,
        deleted: false,
        payload: serde_json::to_value(payload).unwrap_or_default(),
    }
}

fn tombstone(entity_type: &str, entity_id: &str, hlc: Hlc) -> Op {
    Op {
        entity_type: entity_type.into(),
        entity_id: entity_id.into(),
        hlc,
        deleted: true,
        payload: serde_json::json!({}),
    }
}

// ── State ────────────────────────────────────────────────────────────────────

/// A row of `messenger_archive_segments`.
#[derive(Clone, Debug, Default)]
pub(crate) struct SegmentState {
    pub entity_id: String,
    pub head_hlc: Option<Hlc>,
    pub hash: String,
    pub lf: Option<LargeFileRef>,
    pub meta: Option<SegmentMeta>,
    pub merged_hash: String,
    pub replayed_hash: String,
    /// `<hash>@<build>` of a version a build could not merge.
    pub refused: String,
    pub deleted: bool,
    pub tomb: bool,
    pub republish: bool,
}

impl SegmentState {
    /// The storage holds a version this device has not merged into its own.
    fn unmerged(&self) -> Option<&LargeFileRef> {
        self.lf.as_ref().filter(|_| !self.deleted && !self.tomb && !self.hash.is_empty() && self.hash != self.merged_hash)
    }

    /// The storage holds a version this device has not replayed.
    fn unreplayed(&self) -> Option<&LargeFileRef> {
        self.lf.as_ref().filter(|_| !self.deleted && !self.tomb && !self.hash.is_empty() && self.hash != self.replayed_hash)
    }

    /// Whose archive the segment is.
    fn owner(&self) -> Option<PubKey> {
        SegmentId::parse(&self.entity_id).map(|seg| seg.owner)
    }

    fn is_of(&self, owner: &PubKey) -> bool {
        self.owner().as_ref() == Some(owner)
    }
}

/// The mark of a version a build could not merge.
fn refused_mark(hash: &str, build: &str) -> String {
    format!("{hash}@{build}")
}

type Row = (String, String, String, String, String, String, String, String, i64, i64, i64);

const SELECT: &str = "SELECT entity_id, head_hlc, hash, lf, meta, merged_hash, replayed_hash, refused, deleted, tomb, republish
    FROM messenger_archive_segments";

fn from_row(r: Row) -> SegmentState {
    SegmentState {
        entity_id: r.0,
        head_hlc: Hlc::decode(&r.1),
        hash: r.2,
        lf: serde_json::from_str(&r.3).ok(),
        meta: serde_json::from_str(&r.4).ok(),
        merged_hash: r.5,
        replayed_hash: r.6,
        refused: r.7,
        deleted: r.8 != 0,
        tomb: r.9 != 0,
        republish: r.10 != 0,
    }
}

pub(crate) async fn load(db: &Pool<Sqlite>, entity_id: &str) -> CmdResult<Option<SegmentState>> {
    let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!("{SELECT} WHERE entity_id = ?")))
        .bind(entity_id)
        .fetch_optional(db)
        .await
        .map_err(AppError::db)?;
    Ok(row.map(from_row))
}

pub(crate) async fn load_all(db: &Pool<Sqlite>) -> CmdResult<Vec<SegmentState>> {
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!("{SELECT} ORDER BY entity_id")))
        .fetch_all(db)
        .await
        .map_err(AppError::db)?;
    Ok(rows.into_iter().map(from_row).collect())
}

/// The segments of `owner`'s archive.
pub(crate) async fn load_of(db: &Pool<Sqlite>, owner: &PubKey) -> CmdResult<Vec<SegmentState>> {
    Ok(load_all(db).await?.into_iter().filter(|st| st.is_of(owner)).collect())
}

pub(crate) async fn save(db: &Pool<Sqlite>, st: &SegmentState) -> CmdResult<()> {
    let json = |v: Option<String>| v.unwrap_or_default();
    sqlx::query(
        "INSERT INTO messenger_archive_segments
            (entity_id, head_hlc, hash, lf, meta, merged_hash, replayed_hash, refused, deleted, tomb, republish)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(entity_id) DO UPDATE SET head_hlc = excluded.head_hlc, hash = excluded.hash,
            lf = excluded.lf, meta = excluded.meta, merged_hash = excluded.merged_hash,
            replayed_hash = excluded.replayed_hash, refused = excluded.refused, deleted = excluded.deleted,
            tomb = excluded.tomb, republish = excluded.republish",
    )
    .bind(&st.entity_id)
    .bind(st.head_hlc.as_ref().map(Hlc::encode).unwrap_or_default())
    .bind(&st.hash)
    .bind(json(st.lf.as_ref().and_then(|lf| serde_json::to_string(lf).ok())))
    .bind(json(st.meta.as_ref().and_then(|m| serde_json::to_string(m).ok())))
    .bind(&st.merged_hash)
    .bind(&st.replayed_hash)
    .bind(&st.refused)
    .bind(st.deleted as i64)
    .bind(st.tomb as i64)
    .bind(st.republish as i64)
    .execute(db)
    .await
    .map_err(AppError::db)?;
    Ok(())
}

/// The version `hash` of the segment is replayed (or merged): unless a newer
/// op took the row meanwhile.
async fn mark(db: &Pool<Sqlite>, entity_id: &str, hash: &str, column: &str) -> CmdResult<()> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE messenger_archive_segments SET {column} = ? WHERE entity_id = ? AND hash = ?"
    )))
    .bind(hash)
    .bind(entity_id)
    .bind(hash)
    .execute(db)
    .await
    .map_err(AppError::db)?;
    Ok(())
}

/// The version `hash` could not be merged by `build`: not downloaded again
/// by that build.
async fn mark_refused(db: &Pool<Sqlite>, entity_id: &str, hash: &str, build: &str) -> CmdResult<()> {
    sqlx::query("UPDATE messenger_archive_segments SET refused = ? WHERE entity_id = ? AND hash = ?")
        .bind(refused_mark(hash, build))
        .bind(entity_id)
        .bind(hash)
        .execute(db)
        .await
        .map_err(AppError::db)?;
    Ok(())
}

/// The segments of a removed scope go as tombstones with the next collect:
/// those the archive named and those the table knows of the scope.
pub(crate) async fn mark_tombs(db: &Pool<Sqlite>, scope: &ArchiveScope, ids: &[String]) -> CmdResult<()> {
    let infix = format!("/{}/", scope.as_key());
    let mut all: Vec<String> = load_all(db)
        .await?
        .into_iter()
        .filter(|st| st.entity_id.contains(&infix) && !st.deleted)
        .map(|st| st.entity_id)
        .collect();
    all.extend(ids.iter().filter(|id| id.contains(&infix)).cloned());
    all.sort();
    all.dedup();
    for id in all {
        sqlx::query(
            "INSERT INTO messenger_archive_segments (entity_id, tomb) VALUES (?, 1)
             ON CONFLICT(entity_id) DO UPDATE SET tomb = 1, republish = 0",
        )
        .bind(&id)
        .execute(db)
        .await
        .map_err(AppError::db)?;
    }
    Ok(())
}

/// The segments of a scope count as not merged nor replayed: what this
/// device held of them is gone (a private server removed), the storage's
/// files stay and are restored anew.
pub(crate) async fn forget_replayed(db: &Pool<Sqlite>, scope: &ArchiveScope) -> CmdResult<()> {
    sqlx::query(
        "UPDATE messenger_archive_segments SET merged_hash = '', replayed_hash = '', refused = ''
          WHERE instr(entity_id, ?) > 0",
    )
    .bind(format!("/{}/", scope.as_key()))
    .execute(db)
    .await
    .map_err(AppError::db)?;
    Ok(())
}

/// The scopes the store forgot since the last look — a private server
/// removed here or by the snapshot of another device, a group forgotten
/// (`remove_scope_in` marks them): their segments count as not merged nor
/// replayed, and the restore takes them anew once their server or group is
/// here. How many.
async fn take_forgotten(store: &dyn ArchiveStore, db: &Pool<Sqlite>) -> CmdResult<usize> {
    let scopes = store.take_forgotten_scopes().await.map_err(|e| AppError::other(e.to_string()))?;
    for scope in &scopes {
        forget_replayed(db, scope).await?;
    }
    Ok(scopes.len())
}

/// Nothing this device held of the archive is held any more (another
/// storage, or another identity): every version counts as not merged nor
/// replayed, no tombstone of before is owed, and the secrets are taken and
/// published anew.
async fn reset_positions(db: &Pool<Sqlite>) -> CmdResult<()> {
    sqlx::query(
        "UPDATE messenger_archive_segments SET merged_hash = '', replayed_hash = '', refused = '', republish = 0, tomb = 0",
    )
    .execute(db)
    .await
    .map_err(AppError::db)?;
    sqlx::query("DELETE FROM messenger_archive_secrets").execute(db).await.map_err(AppError::db)?;
    Ok(())
}

/// Bytes, first and last month of what the storage holds of `owner`'s
/// archive; nothing without an owner.
pub(crate) async fn extent(db: &Pool<Sqlite>, owner: Option<&PubKey>) -> CmdResult<(u64, Option<String>, Option<String>)> {
    let Some(owner) = owner else { return Ok((0, None, None)) };
    let metas: Vec<SegmentMeta> = load_of(db, owner)
        .await?
        .into_iter()
        .filter(|st| !st.deleted)
        .filter_map(|st| st.meta)
        .collect();
    let size = metas.iter().map(|m| m.size).sum();
    let from = metas.iter().map(|m| m.month.clone()).min();
    let to = metas.iter().map(|m| m.month.clone()).max();
    Ok((size, from, to))
}

/// The months of `owner`'s archive this device does not take, as their
/// files are over the bounds of the format (`MAX_SEGMENT_BYTES`): they stay
/// in the queue, refused, and the UI says which they are.
pub(crate) async fn too_large_months(db: &Pool<Sqlite>, owner: Option<&PubKey>) -> CmdResult<Vec<String>> {
    let Some(owner) = owner else { return Ok(Vec::new()) };
    let mut months: Vec<String> = load_of(db, owner)
        .await?
        .into_iter()
        .filter(|st| st.unreplayed().is_some())
        .filter_map(|st| st.meta)
        .filter(|m| m.size > MAX_SEGMENT_BYTES)
        .map(|m| m.month)
        .collect();
    months.sort();
    months.dedup();
    Ok(months)
}

/// A file over the bounds of the format, by the size its op names: not
/// downloaded (a phone would hold it whole in memory).
fn over_bounds(entity_id: &str, lf: &LargeFileRef) -> Option<MessengerError> {
    (lf.size > MAX_SEGMENT_BYTES).then(|| too_large(format!("{entity_id}: a file of {} bytes", lf.size)))
}

// ── Storage ──────────────────────────────────────────────────────────────────

/// No storage the archive uses: a folder on a phone, as attachments of notes
/// have none there (plan §4.5).
pub(crate) fn no_storage(cfg: &SyncConfig) -> bool {
    cfg!(mobile) && cfg.backend == "folder"
}

/// Why the storage stops the writing, if it does: `413 quota`, `403
/// read_only` (`storage_cloud::fail` keeps the code in the message of
/// `Storage`, and `until=<unix seconds>` when the server said), or no
/// answer at all. Anything else is the failure of one segment.
pub(crate) fn pause_reason(e: &SyncError) -> Option<PauseReason> {
    match e {
        SyncError::Quota { .. } => Some(PauseReason::Quota),
        SyncError::Storage(m) if m.contains("read_only") => {
            let until = m.split_whitespace().find_map(|w| w.strip_prefix("until=")).and_then(|v| v.parse::<i64>().ok());
            Some(PauseReason::ReadOnly { until })
        }
        SyncError::Storage(m) => {
            let m = m.to_lowercase();
            let offline = ["error sending request", "connect", "timed out", "timeout", "dns", "unreachable", "bridge is down"];
            offline.iter().any(|w| m.contains(w)).then_some(PauseReason::Offline)
        }
        _ => None,
    }
}

/// The bytes of a segment as a source of a large file.
struct Bytes(Arc<Vec<u8>>);

/// The tail of the bytes from an offset (an upload that resumes).
struct Tail {
    bytes: Arc<Vec<u8>>,
    from: usize,
}

impl AsRef<[u8]> for Tail {
    fn as_ref(&self) -> &[u8] {
        &self.bytes[self.from..]
    }
}

#[async_trait::async_trait]
impl LargeFileSource for Bytes {
    fn len_hint(&self) -> Option<u64> {
        Some(self.0.len() as u64)
    }

    async fn open_at(&self, offset: u64) -> veydan_sync::Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>> {
        let from = usize::try_from(offset).unwrap_or(usize::MAX).min(self.0.len());
        Ok(Box::new(std::io::Cursor::new(Tail { bytes: self.0.clone(), from })))
    }
}

/// Who moves a file: the collect of the cycle, or the background restore.
/// Each has its transfer and its staging file, so the two never share one.
#[derive(Clone, Copy)]
pub(crate) enum Who {
    Collect,
    Drain,
}

impl Who {
    fn as_str(self) -> &'static str {
        match self {
            Self::Collect => "collect",
            Self::Drain => "drain",
        }
    }
}

/// The key of a transfer of the archive among those of `SyncManager`.
fn transfer_key(who: Who, entity_id: &str) -> String {
    format!("{ENTITY_SEGMENT}/{}/{entity_id}", who.as_str())
}

fn staging_root(core: &Core) -> PathBuf {
    core.app_data_dir.join(crate::commands::DATA_SUBDIR).join(STAGING)
}

/// The staging files of one mover: the collect and the restore each keep
/// their own, and each empties its own.
fn staging_dir(core: &Core, who: Who) -> PathBuf {
    staging_root(core).join(who.as_str())
}

/// Empty the staging files of `who`: at its first run of a session (what a
/// process killed half way left), and once it has nothing more to move.
/// The restore takes the loose files of a build before this one too.
pub(crate) fn sweep_staging(core: &Core, who: Who) {
    let dir = staging_dir(core, who);
    if dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            eprintln!("messenger: archive: the staging files were not removed: {e}");
        }
    }
    if matches!(who, Who::Drain) {
        if let Ok(entries) = std::fs::read_dir(staging_root(core)) {
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|t| t.is_file()) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
}

async fn upload(
    store: &LargeFileStore<'_>,
    sync: &SyncManager,
    app: Option<&AppHandle>,
    (entity_id, month): (&str, &str),
    bytes: Vec<u8>,
) -> veydan_sync::Result<LargeFileRef> {
    let key = transfer_key(Who::Collect, entity_id);
    let cancel = sync.begin_transfer(&key);
    let progress = |p: Progress| {
        if let Some(app) = app {
            let pct = p.total.filter(|t| *t > 0).map(|t| (p.done * 100 / t) as u32).unwrap_or(0);
            emit_progress(app, "archive_up", pct, 0, 0, month);
        }
    };
    let result = store.upload(&Bytes(Arc::new(bytes)), &progress, &cancel).await;
    sync.end_transfer(&key);
    result
}

/// Download the version `hash` of a segment through a staging file of the
/// messenger's folder, so that a download cut off goes on where it
/// stopped; the file is the caller's and the version's alone.
async fn download(
    store: &LargeFileStore<'_>,
    sync: &SyncManager,
    staging: &Path,
    who: Who,
    (entity_id, hash): (&str, &str),
    lf: &LargeFileRef,
    progress: &(dyn Fn(Progress) + Send + Sync),
) -> veydan_sync::Result<Vec<u8>> {
    std::fs::create_dir_all(staging).map_err(|e| SyncError::Disk(e.to_string()))?;
    let dest = staging.join(&sha256_hex(format!("{}:{entity_id}:{hash}", who.as_str()).as_bytes())[..32]);
    let key = transfer_key(who, entity_id);
    let cancel = sync.begin_transfer(&key);
    let result = store.download(lf, &PathSink::new(&dest), progress, &cancel).await;
    sync.end_transfer(&key);
    result?;
    // The engine wrote what the manifest names; a file bigger than the
    // bound is not read into memory, whatever it is.
    let bytes = match std::fs::metadata(&dest) {
        Ok(m) if m.len() > MAX_SEGMENT_BYTES => Err(SyncError::Integrity(format!("the segment is {} bytes, over the bound", m.len()))),
        _ => tokio::fs::read(&dest).await.map_err(|e| SyncError::Disk(e.to_string())),
    };
    let _ = std::fs::remove_file(&dest);
    checked(bytes?, hash)
}

/// A download that brought the file the op names.
fn checked(bytes: Vec<u8>, hash: &str) -> veydan_sync::Result<Vec<u8>> {
    if sha256_hex(&bytes) == hash {
        Ok(bytes)
    } else {
        Err(SyncError::Integrity("the segment is not the file its op names".into()))
    }
}

fn messenger(e: MessengerError) -> String {
    e.to_string()
}

/// A failure that does not stop the cycle: the developer log and the
/// warnings of the cycle.
pub(crate) fn warn(out: &mut Collected, line: String) {
    out.warnings.push(format!("messenger archive: {line}"));
    out.errors.push(line);
}

/// What one collect may still move of the archive.
struct Budget {
    segments: u32,
    bytes: u64,
}

impl Budget {
    fn new() -> Self {
        Self { segments: 0, bytes: 0 }
    }

    /// Take `size` bytes for one more transfer; `false` when the cycle has
    /// moved enough (one transfer is always allowed).
    fn take(&mut self, size: u64) -> bool {
        if self.segments > 0 && (self.segments >= BUDGET_SEGMENTS || self.bytes + size > BUDGET_BYTES) {
            return false;
        }
        self.segments += 1;
        self.bytes += size;
        true
    }
}

// ── Collect ──────────────────────────────────────────────────────────────────

/// What the push of the collect stands for, saved once it went through.
#[derive(Default)]
pub(crate) struct Saves {
    pub segments: Vec<(SegmentState, Option<(SegmentId, SegmentMeta)>)>,
    pub secrets: Vec<secrets::SecretState>,
}

/// The storage or the identity is another than the one this device wrote
/// its segments to and for: every segment counts as unwritten, nothing
/// held of the archive before is reused.
async fn check_vault(db: &Pool<Sqlite>, store: &dyn ArchiveStore, vault_id: &str, owner: &PubKey) -> CmdResult<()> {
    let mark = format!("{vault_id}|{}", owner.as_hex());
    if settings::get(db, WRITTEN_TO_KEY).await.as_deref() == Some(mark.as_str()) {
        return Ok(());
    }
    store.forget_written().await.map_err(|e| AppError::other(e.to_string()))?;
    reset_positions(db).await?;
    settings::set(db, WRITTEN_TO_KEY, &mark).await
}

/// The monthly pass of compaction (plan §4.2, D5): every closed segment this
/// device knows loses the events deletions pointed at. One that dropped
/// anything is dirty and due at once; the collect below writes it as a
/// rewrite. Once per `COMPACT_EVERY_SECS`, whatever the pass found.
async fn compact_closed(db: &Pool<Sqlite>, store: &dyn ArchiveStore, states: &[SegmentState], now: Timestamp, out: &mut Collected) -> CmdResult<()> {
    let last = settings::get(db, COMPACTED_AT_KEY).await.and_then(|v| v.parse::<i64>().ok());
    if last.is_some_and(|at| at + COMPACT_EVERY_SECS > now.secs()) {
        return Ok(());
    }
    for st in states.iter().filter(|st| !st.deleted && !st.tomb) {
        let Some(seg) = SegmentId::parse(&st.entity_id) else { continue };
        if !seg.month.is_closed_at(now) {
            continue;
        }
        if let Err(e) = store.compact(&seg).await {
            warn(out, format!("{}: compact: {}", st.entity_id, messenger(e)));
        }
    }
    settings::set(db, COMPACTED_AT_KEY, &now.secs().to_string()).await
}

pub(crate) async fn collect_segments(cx: &mut Cycle<'_>, arch: &ArchiveSync, saves: &mut Saves, out: &mut Collected) -> CmdResult<()> {
    let app = cx.host.app();
    // The runtime records into the archive whatever the storage is.
    arch.install();
    if no_storage(cx.config) {
        arch.set_paused(Some(PauseReason::NoStorage));
        return Ok(());
    }
    let Some(store) = arch.store() else { return Ok(()) };
    let owner = match store.owner().await {
        Ok(Some(owner)) => owner,
        // No identity yet: nothing to write for.
        Ok(None) => return Ok(()),
        Err(e) => {
            warn(out, format!("owner: {}", messenger(e)));
            return Ok(());
        }
    };
    let db = &cx.core.db;
    if let Err(e) = check_vault(db, &*store, cx.engine.vault_id(), &owner).await {
        warn(out, format!("storage: {e}"));
        return Ok(());
    }
    // A pause of `read_only` or `quota` holds for an hour; the rest lasted
    // one cycle, and this one tries again.
    let held = arch.pause_holds();
    if held.is_none() {
        arch.set_paused(None);
    }
    let states = load_of(db, &owner).await?;

    // Tombstones of the scopes removed here.
    for st in states.iter().filter(|st| st.tomb) {
        let hlc = cx.clock.now();
        out.ops.push(tombstone(ENTITY_SEGMENT, &st.entity_id, hlc.clone()));
        let saved = SegmentState { head_hlc: Some(hlc), deleted: true, tomb: false, republish: false, ..st.clone() };
        saves.segments.push((saved, None));
    }

    compact_closed(db, &*store, &states, now(), out).await?;
    let mut due = match store.due(now(), arch.take_flush()).await {
        Ok(due) => due,
        Err(e) => {
            warn(out, format!("due: {}", messenger(e)));
            return Ok(());
        }
    };
    // Its own ops the storage lost: written again, whatever `due` says.
    for st in states.iter().filter(|st| st.republish && !st.deleted && !st.tomb) {
        if let Some(seg) = SegmentId::parse(&st.entity_id) {
            if !due.contains(&seg) {
                due.push(seg);
            }
        }
    }
    if arch.first_collect() || due.is_empty() {
        // Nothing of before is in flight: what a cut download left goes.
        sweep_staging(cx.core, Who::Collect);
    }
    if due.is_empty() {
        return Ok(());
    }
    if let Some(reason) = held {
        // The storage said no not long ago: the writing waits, the reading
        // goes on in the background.
        arch.tell(app, ArchiveProgress { phase: ArchivePhase::Paused, reason: Some(reason), ..idle() });
        return Ok(());
    }
    let build = build_of(app);
    let lfs = cx.engine.large_files(lf_config()).map_err(AppError::other)?;
    let staging = staging_dir(cx.core, Who::Collect);
    let total = due.len() as u32;
    let mut paused: Option<PauseReason> = None;
    let mut budget = Budget::new();
    for (i, seg) in due.iter().enumerate() {
        let id = seg.entity_id();
        let month = seg.month.to_string();
        let st = load(db, &id).await?.unwrap_or_else(|| SegmentState { entity_id: id.clone(), ..Default::default() });
        if st.tomb {
            // The scope is gone here: its tombstone goes, nothing else.
            continue;
        }
        arch.tell(
            app,
            ArchiveProgress { phase: ArchivePhase::Writing, done: i as u32, total, bytes_done: 0, bytes_total: 0, month: Some(month.clone()), reason: None },
        );
        // The storage holds a version this one has not merged: read it,
        // replay what only it has, write the union if it differs.
        let candidate: Option<Built> = match st.unmerged() {
            Some(lf) => {
                if st.refused == refused_mark(&st.hash, &build) {
                    // This build could not read it; another will.
                    continue;
                }
                // The merge replays: it waits for the runtime.
                let Some(replayer) = arch.replayer() else { continue };
                // A version this device read already is merged from what
                // is held; another is downloaded.
                let held = match store.holds(seg, &st.hash).await {
                    Ok(true) => match store.merge_held(seg, &st.hash, &*replayer).await {
                        Ok(merged) => merged,
                        Err(e) => {
                            warn(out, format!("{id}: merge: {}", messenger(e)));
                            continue;
                        }
                    },
                    Ok(false) => None,
                    Err(e) => {
                        warn(out, format!("{id}: {}", messenger(e)));
                        continue;
                    }
                };
                if let (None, Some(e)) = (&held, over_bounds(&id, lf)) {
                    // Not this build's to hold in memory: refused, and the
                    // segment waits; the restore says which month it is.
                    mark_refused(db, &id, &st.hash, &build).await?;
                    if let Err(e) = store.defer(seg).await {
                        warn(out, format!("{id}: {}", messenger(e)));
                    }
                    warn(out, format!("{id}: merge: {}", messenger(e)));
                    continue;
                }
                let merged = match held {
                    Some(merged) => merged,
                    None => {
                        if !budget.take(lf.size) {
                            break;
                        }
                        let progress = |_: Progress| {
                            if let Some(app) = app {
                                emit_progress(app, "archive_down", 20, i as u32, total, &month);
                            }
                        };
                        let bytes = download(&lfs, cx.sync, &staging, Who::Collect, (&id, &st.hash), lf, &progress).await;
                        let bytes = match bytes {
                            Ok(bytes) => bytes,
                            Err(e) => {
                                warn(out, format!("{id}: {e}"));
                                if matches!(pause_reason(&e), Some(PauseReason::Offline)) {
                                    paused = Some(PauseReason::Offline);
                                    break;
                                }
                                continue;
                            }
                        };
                        match store.merge(seg, &bytes, &*replayer).await {
                            Ok(merged) => merged,
                            Err(e) => {
                                if is_format_error(&e) {
                                    // Not this build's to read: not downloaded
                                    // again by it, and the segment waits.
                                    mark_refused(db, &id, &st.hash, &build).await?;
                                    if let Err(e) = store.defer(seg).await {
                                        warn(out, format!("{id}: {}", messenger(e)));
                                    }
                                }
                                warn(out, format!("{id}: merge: {}", messenger(e)));
                                continue;
                            }
                        }
                    }
                };
                mark(db, &id, &st.hash, "merged_hash").await?;
                mark(db, &id, &st.hash, "replayed_hash").await?;
                // `None`: the storage holds the union, the archive knows it.
                merged.upload
            }
            None => match store.build(seg).await {
                Ok(built) => Some(built),
                Err(e) => {
                    if is_too_large(&e) {
                        // The month is over the bounds of the format: it
                        // waits the flush interval, not every cycle, and
                        // stays dirty: the status names it as in no backup
                        // (`Archive::unwritable`).
                        if let Err(e) = store.defer(seg).await {
                            warn(out, format!("{id}: {}", messenger(e)));
                        }
                    }
                    warn(out, format!("{id}: build: {}", messenger(e)));
                    continue;
                }
            },
        };
        let Some(built) = candidate else { continue };
        let live = !st.deleted && !st.hash.is_empty();
        if built.meta.count == 0 {
            // Every event was compacted away (D5): a tombstone in place of
            // an empty file; one goes in the journal, not in the pool of
            // files, so a wall does not hold it.
            if live {
                let hlc = cx.clock.now();
                out.ops.push(tombstone(ENTITY_SEGMENT, &id, hlc.clone()));
                let saved = SegmentState { head_hlc: Some(hlc), deleted: true, tomb: false, republish: false, ..st.clone() };
                saves.segments.push((saved, Some((seg.clone(), built.meta))));
            } else if let Err(e) = store.written(seg, &built.meta).await {
                warn(out, format!("{id}: {}", messenger(e)));
            }
            continue;
        }
        if live && built.meta.hash == st.hash && !st.republish {
            // What the storage holds already.
            if let Err(e) = store.written(seg, &built.meta).await {
                warn(out, format!("{id}: {}", messenger(e)));
            }
            continue;
        }
        if paused.is_some() {
            // Reading goes on; the writing waits for the next cycle.
            continue;
        }
        if !budget.take(built.meta.size) {
            break;
        }
        let meta = built.meta.clone();
        match upload(&lfs, cx.sync, app, (&id, &month), built.bytes).await {
            Ok(lf) => {
                let hlc = cx.clock.now();
                out.ops.push(segment_op(&id, hlc.clone(), SegmentPayload::new(meta.clone(), lf.clone())));
                let saved = SegmentState {
                    entity_id: id.clone(),
                    head_hlc: Some(hlc),
                    hash: meta.hash.clone(),
                    lf: Some(lf),
                    meta: Some(meta.clone()),
                    merged_hash: meta.hash.clone(),
                    replayed_hash: meta.hash.clone(),
                    refused: String::new(),
                    deleted: false,
                    tomb: false,
                    republish: false,
                };
                saves.segments.push((saved, Some((seg.clone(), meta))));
            }
            Err(e) => {
                warn(out, format!("{id}: {e}"));
                if let Some(reason) = pause_reason(&e) {
                    paused = Some(reason);
                }
            }
        }
    }
    match paused {
        Some(reason) => {
            arch.set_paused(Some(reason.clone()));
            arch.tell(app, ArchiveProgress { phase: ArchivePhase::Paused, reason: Some(reason), ..idle() });
        }
        None if !arch.is_draining() => arch.tell(app, idle()),
        None => {}
    }
    Ok(())
}

/// Save what the push stood for: the positions, and the archive told what
/// the storage holds now.
pub(crate) async fn commit(db: &Pool<Sqlite>, store: Option<Arc<dyn ArchiveStore>>, saves: Saves) -> CmdResult<()> {
    for st in &saves.secrets {
        secrets::save(db, st).await?;
    }
    for (st, written) in saves.segments {
        save(db, &st).await?;
        if let (Some(store), Some((seg, meta))) = (&store, written) {
            // The archive asks again next time; the op is out either way.
            if let Err(e) = store.written(&seg, &meta).await {
                eprintln!("messenger: archive {}: {e}", st.entity_id);
            }
        }
    }
    Ok(())
}

// ── Apply ────────────────────────────────────────────────────────────────────

/// Record every newer op of a segment and queue what is not replayed of
/// the owner's archive; another owner's segments are recorded and left.
/// Returns whether something of the owner's waits to be restored.
pub(crate) async fn apply_segments(cx: &mut Cycle<'_>, arch: &ArchiveSync, ops: &[Op], applied: &mut Applied) -> CmdResult<bool> {
    let db = &cx.core.db;
    let store = arch.store();
    let owner = match &store {
        Some(store) => store.owner().await.ok().flatten(),
        None => None,
    };
    for op in ops.iter().filter(|op| op.entity_type == ENTITY_SEGMENT) {
        let Some(seg) = SegmentId::parse(&op.entity_id) else {
            applied.skipped.push(format!("archive segment {}: not a segment id", op.entity_id));
            continue;
        };
        let mine = owner.as_ref() == Some(&seg.owner);
        let prev = load(db, &op.entity_id).await?;
        if prev.as_ref().and_then(|st| st.head_hlc.as_ref()).is_some_and(|h| *h >= op.hlc) {
            continue;
        }
        let mut st = prev.unwrap_or_else(|| SegmentState { entity_id: op.entity_id.clone(), ..Default::default() });
        st.head_hlc = Some(op.hlc.clone());
        st.tomb = false;
        st.republish = false;
        if op.deleted {
            let was = std::mem::take(&mut st.hash);
            st.deleted = true;
            save(db, &st).await?;
            if let (Some(store), true) = (&store, mine) {
                // The version queued, if any, is not to come.
                if let Err(e) = store.done(&op.entity_id, &was).await {
                    cx.warnings.push(format!("messenger archive: {}: {e}", op.entity_id));
                }
            }
            continue;
        }
        let payload = match serde_json::from_value::<SegmentPayload>(op.payload.clone()) {
            Ok(payload) if payload.is_for(&seg) => payload,
            _ => {
                applied.skipped.push(format!("archive segment {}: the payload does not name it", op.entity_id));
                continue;
            }
        };
        st.deleted = false;
        st.hash = payload.meta.hash.clone();
        st.lf = Some(payload.lf);
        st.meta = Some(payload.meta.clone());
        save(db, &st).await?;
        if !mine || st.unreplayed().is_none() {
            continue;
        }
        if let Some(store) = &store {
            match store.enqueue(&op.entity_id, &payload.meta).await {
                Ok(Enqueued::Queued | Enqueued::Known) => {}
                // This device holds that very version, replayed whole.
                Ok(Enqueued::Held { unreplayed: false }) => mark(db, &op.entity_id, &st.hash, "replayed_hash").await?,
                // Held, with events still to replay: the restore takes them
                // from what is held.
                Ok(Enqueued::Held { unreplayed: true }) => {}
                Err(e) => cx.warnings.push(format!("messenger archive: {}: {e}", op.entity_id)),
            }
        }
    }
    let Some(owner) = owner else { return Ok(false) };
    Ok(load_of(db, &owner).await?.iter().any(|st| st.unreplayed().is_some()))
}

// ── Restore ──────────────────────────────────────────────────────────────────

/// What a try of one item came to.
enum Tried {
    Replayed,
    /// The storage did not give the file.
    Download(SyncError),
    /// The archive or the runtime refused.
    Replay(MessengerError),
}

/// Whether the restore may go on: the vault is still joined and the same,
/// and the Cloud is not waiting on anything. Always in a test (no app).
async fn may_go_on(core: &Core, engine: &Engine, app: Option<&AppHandle>) -> bool {
    if app.is_none() {
        return true;
    }
    if !veydan_sync_host::vault_active(core).await || veydan_sync_host::config::cloud_waiting(&core.db).await.is_some() {
        return false;
    }
    veydan_sync_host::config::load_binding(&core.db).await.is_some_and(|b| b.vault_id == engine.vault_id())
}

/// Download and replay the queue, newest months first, the open chat before
/// all, until it is empty, the storage does not answer, the storage
/// changes or the runtime is between sessions. Runs in the background with
/// an engine of its own (`super::kick`).
///
/// A version this device holds already (read before, cut short) is replayed
/// from what is held, without the file. A segment the runtime refuses for
/// now — a group this device has no row for yet (`group_unknown`: its
/// welcome is in a month of `dm` not replayed yet), a private server not
/// known here (`server_unknown`: parked before its file comes down) — is
/// parked and tried again after a round that replayed something (the
/// welcome may have come), when a key comes, when the runtime wakes the
/// handler (a welcome landed, a server was put in: `super::woke`), at the
/// start of a run and once an hour; a file this build does not read is
/// refused and tried again by another build. Parked and refused segments
/// do not hold the restore up: the runtime hears `archive_restored`, the
/// UI `archive.synced`, when nothing else is left. A scope the store forgot
/// since (a server removed, here or by a snapshot) is taken first: its
/// segments count as not replayed from here on.
pub(crate) async fn drain(arch: &ArchiveSync, engine: &Engine, core: &Core, sync: &SyncManager, app: Option<&AppHandle>) -> CmdResult<()> {
    let (Some(store), Some(replayer)) = (arch.store(), arch.replayer()) else { return Ok(()) };
    let db = &core.db;
    let err = |e: MessengerError| AppError::other(e.to_string());
    let Some(owner) = store.owner().await.map_err(err)? else { return Ok(()) };
    // A restore may run before the first collect of a fresh device (the
    // kick of `provide_runtime`): the vault is bound here as there, so
    // that the collect does not start the positions over after it.
    check_vault(db, &*store, engine.vault_id(), &owner).await?;
    take_forgotten(&*store, db).await?;
    let build = build_of(app);
    if !arch.told_restored() {
        // The first run of a session: what a process killed half way left
        // staged goes (a download starts over), and what another build
        // refused and what waited for a group or a server gets its try.
        sweep_staging(core, Who::Drain);
        store.retry_refused(&build).await.map_err(err)?;
        store.retry_parked().await.map_err(err)?;
    } else if arch.parked_retry_due() {
        store.retry_parked().await.map_err(err)?;
    }
    // The queue follows the table: what is not replayed is in it, also
    // after a restart or ops taken while the messenger did not run.
    let pending: Vec<SegmentState> = load_of(db, &owner).await?.into_iter().filter(|st| st.unreplayed().is_some()).collect();
    for st in &pending {
        if let Some(meta) = &st.meta {
            store.enqueue(&st.entity_id, meta).await.map_err(err)?;
        }
    }
    let mut replayed = false;
    if !pending.is_empty() {
        let lfs = engine.large_files(lf_config()).map_err(AppError::other)?;
        let staging = staging_dir(core, Who::Drain);
        // What failed in an earlier round is tried in this one.
        store.retry_failed().await.map_err(err)?;
        'passes: for pass in 0..DRAIN_PASSES {
            let mut progressed = false;
            let mut tried = HashSet::new();
            while let Some(item) = store.dequeue(arch.open().as_ref()).await.map_err(err)? {
                if !may_go_on(core, engine, app).await {
                    break 'passes;
                }
                if !tried.insert((item.entity_id.clone(), item.hash.clone())) {
                    break;
                }
                let st = load(db, &item.entity_id).await?;
                let Some((st, lf)) = st.as_ref().and_then(|st| st.unreplayed().map(|lf| (st, lf.clone()))).filter(|(st, _)| st.hash == item.hash) else {
                    // Gone, replayed meanwhile, or a version the table no
                    // longer names.
                    store.done(&item.entity_id, &item.hash).await.map_err(err)?;
                    continue;
                };
                if let ArchiveScope::DmServer { id } = &item.segment.scope {
                    // A server this device does not have (removed, or not
                    // taken from the snapshot yet): parked before its file
                    // comes down, so nothing of it is put back here.
                    if !store.has_server(id).await.map_err(err)? {
                        store.park(&item.entity_id, &item.hash, messenger_archive::REFUSED_SERVER_UNKNOWN).await.map_err(err)?;
                        continue;
                    }
                }
                let month = item.segment.month.to_string();
                let restoring = ArchiveProgress { phase: ArchivePhase::Restoring, month: Some(month.clone()), ..idle() };
                let base = super::counts(store.progress().await.map_err(err)?, restoring);
                let (done, total, bytes_done) = (base.done, base.total, base.bytes_done);
                arch.tell(app, base.clone());
                let progress = |p: Progress| {
                    arch.tell(app, ArchiveProgress { bytes_done: bytes_done + p.done, ..base.clone() });
                    if let Some(app) = app {
                        let pct = veydan_sync_host::progress_pct(0, 100, done, total.max(1));
                        emit_progress(app, "archive_down", pct, done, total, &month);
                    }
                };
                let tried_item = replay_item(&*store, &*replayer, &lfs, sync, &staging, &item, &lf, &st.hash, &progress).await;
                match tried_item {
                    Tried::Replayed => {
                        store.done(&item.entity_id, &item.hash).await.map_err(err)?;
                        mark(db, &item.entity_id, &st.hash, "replayed_hash").await?;
                        replayed = true;
                        progressed = true;
                    }
                    Tried::Download(e) => {
                        store.failed(&item.entity_id, &item.hash, &e.to_string()).await.map_err(err)?;
                        if matches!(e, SyncError::Cancelled) || matches!(pause_reason(&e), Some(PauseReason::Offline)) {
                            break 'passes;
                        }
                    }
                    // Between sessions: the segment stays pending, as it is.
                    Tried::Replay(MessengerError::NotLoggedIn) => break 'passes,
                    Tried::Replay(e) if is_parked_reason(&e) => store.park(&item.entity_id, &item.hash, &e.to_string()).await.map_err(err)?,
                    Tried::Replay(e) if is_format_error(&e) => {
                        store.refuse(&item.entity_id, &item.hash, &e.to_string(), &build).await.map_err(err)?
                    }
                    Tried::Replay(e) => store.failed(&item.entity_id, &item.hash, &e.to_string()).await.map_err(err)?,
                }
            }
            // Another round when something came that a parked segment may
            // have waited for, or when a failure of the storage is to be
            // tried once more.
            let parked_back = progressed && store.retry_parked().await.map_err(err)? > 0;
            let failed_back = pass == 0 && store.retry_failed().await.map_err(err)? > 0;
            if !parked_back && !failed_back {
                break;
            }
        }
    }
    let blocked: HashSet<String> = store.blocked().await.map_err(err)?.into_iter().collect();
    let left = load_of(db, &owner).await?.iter().any(|st| st.unreplayed().is_some() && !blocked.contains(&st.entity_id));
    arch.tell(app, idle());
    if !left {
        sweep_staging(core, Who::Drain);
        if replayed {
            // The next restore counts from zero.
            store.clear_done().await.map_err(err)?;
        }
        arch.restored(app, replayed);
    }
    Ok(())
}

/// One item of the queue: from what is held when the version is held,
/// else the file downloaded and replayed.
#[allow(clippy::too_many_arguments)]
async fn replay_item(
    store: &dyn ArchiveStore,
    replayer: &dyn messenger_archive::Replayer,
    lfs: &LargeFileStore<'_>,
    sync: &SyncManager,
    staging: &Path,
    item: &QueueItem,
    lf: &LargeFileRef,
    hash: &str,
    progress: &(dyn Fn(Progress) + Send + Sync),
) -> Tried {
    match store.replay_held_of(item, replayer).await {
        Ok(Some(_)) => return Tried::Replayed,
        Ok(None) => {}
        Err(e) => return Tried::Replay(e),
    }
    if let Some(e) = over_bounds(&item.entity_id, lf) {
        // Refused before it comes down: it stays in the queue for a build
        // that reads it, and the UI names the month.
        return Tried::Replay(e);
    }
    let bytes = match download(lfs, sync, staging, Who::Drain, (&item.entity_id, hash), lf, progress).await {
        Ok(bytes) => bytes,
        Err(e) => return Tried::Download(e),
    };
    match store.replay_segment(item, &bytes, replayer).await {
        Ok(_) => Tried::Replayed,
        Err(e) => Tried::Replay(e),
    }
}

// ── The handler ──────────────────────────────────────────────────────────────

/// Both entities of the archive, with a handler of their own.
pub(crate) struct MessengerArchive;

#[async_trait::async_trait]
impl Handler for MessengerArchive {
    fn entities(&self) -> &'static [&'static str] {
        &[ENTITY_SECRET, ENTITY_SEGMENT]
    }

    fn state_tables(&self) -> &'static [&'static str] {
        &[super::SECRET_STATE, SEGMENT_STATE]
    }

    /// The secrets of groups first: a device that reads the segments needs
    /// their keys. Nothing here fails the cycle: what cannot be written
    /// now is a warning and is written by a later one.
    async fn collect(&self, cx: &mut Cycle<'_>) -> CmdResult<Collected> {
        let Some(arch) = cx.host.state::<ArchiveSync>() else { return Ok(Collected::default()) };
        let mut out = Collected::default();
        let mut saves = Saves::default();
        secrets::collect(cx, arch, &mut saves, &mut out).await?;
        collect_segments(cx, arch, &mut saves, &mut out).await?;
        if !saves.segments.is_empty() || !saves.secrets.is_empty() {
            let store = arch.store();
            out.commit = Some(Box::new(move |db| Box::pin(commit_owned(db, store, saves))));
        }
        Ok(out)
    }

    /// Keys before segments; the segments are recorded and queued here and
    /// restored by the background task. A cycle without the app (a test)
    /// restores them in place. A key put in the box may be what a parked
    /// segment waited for: those go back to the queue.
    async fn apply(&self, cx: &mut Cycle<'_>, ops: &[Op]) -> CmdResult<Applied> {
        let Some(arch) = cx.host.state::<ArchiveSync>() else { return Ok(Applied::default()) };
        let mut applied = Applied::default();
        // The vault is bound before anything of it is recorded here, as
        // the collect binds it: a device whose first contact with a vault
        // is what it reads must not start its positions over at the next
        // collect (and send the secrets it just took back).
        if let Some(store) = arch.store() {
            match store.owner().await {
                Ok(Some(owner)) => {
                    if let Err(e) = check_vault(&cx.core.db, &*store, cx.engine.vault_id(), &owner).await {
                        cx.warnings.push(format!("messenger archive: storage: {e}"));
                    }
                }
                Ok(None) => {}
                Err(e) => cx.warnings.push(format!("messenger archive: {e}")),
            }
        }
        let keys_put = secrets::apply(cx, arch, ops, &mut applied).await?;
        let mut pending = apply_segments(cx, arch, ops, &mut applied).await?;
        if let Some(store) = arch.store() {
            if keys_put > 0 {
                match store.retry_parked().await {
                    Ok(n) => pending |= n > 0,
                    Err(e) => cx.warnings.push(format!("messenger archive: {e}")),
                }
            }
            // A scope forgotten since (a server removed by a snapshot):
            // its segments are to be looked at anew.
            match take_forgotten(&*store, &cx.core.db).await {
                Ok(n) => pending |= n > 0,
                Err(e) => cx.warnings.push(format!("messenger archive: {e}")),
            }
        }
        // Also with nothing to restore, once a run: the runtime waits to
        // hear that the restore is over.
        if pending || !arch.told_restored() {
            match cx.host.app() {
                Some(app) => super::kick(app),
                None => drain(arch, cx.engine, cx.core, cx.sync, None).await?,
            }
        }
        Ok(applied)
    }

    /// What this device pushed after `newest` was lost with the chunks of
    /// its log: its segments are written again and its tombstones sent
    /// again with the next collect; so are its secrets.
    async fn forget_pushed_after(&self, tx: &mut sqlx::SqliteConnection, device: &str, newest: Option<&Hlc>) -> CmdResult<()> {
        let after = veydan_sync_host::state::pushed_after("head_hlc");
        let floor = veydan_sync_host::state::hlc_floor(newest);
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE messenger_archive_segments SET republish = 1 - deleted, tomb = MAX(tomb, deleted) WHERE {after}"
        )))
        .bind(device)
        .bind(&floor)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE messenger_archive_secrets SET hash = '', tomb = MAX(tomb, deleted) WHERE {after}"
        )))
        .bind(device)
        .bind(&floor)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;
        Ok(())
    }

    // `device_rotated` keeps the default: the archive holds nothing under the
    // id of this device. Its positions are HLCs, which order the ops of the
    // old id and of the new alike, and the old log is read as another
    // device's from where the storage confirms it (`config::rotate_device_id`).
}

async fn commit_owned(db: Pool<Sqlite>, store: Option<Arc<dyn ArchiveStore>>, saves: Saves) -> CmdResult<()> {
    commit(&db, store, saves).await
}
