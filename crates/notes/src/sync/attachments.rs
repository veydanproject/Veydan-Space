// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Note attachments as a sync entity. Each file under `attachments/<note_id>/`
//! is one logical entity `<note_id>/<name>` that travels in one of two forms:
//!
//! - `note_attachment` (v1): `{ note_id, name, blob, size }`, bytes in one blob;
//! - `note_attachment_v2`: `{ note_id, name, size, lf: LargeFileRef, lf_refs: [manifest_id] }`,
//!   bytes as large-file chunks. Old clients ignore the unknown entity type.
//!
//! The notes policy picks the form per file (size threshold). Both forms share
//! one sync state row and are reconciled by HLC (LWW, no merge). A tombstone
//! uses the form of the last published version so old clients see v1 deletes.

use super::config::load_config;
use super::notes::{docs_dir, valid_id, write_raw};
use super::state::{
    load_attachment_state, load_attachment_states, load_deferred_attachments,
    save_attachment_state, AttachmentSyncState,
};
use super::SyncManager;
use crate::{
    attachments_dir_for, is_staging_name, load_attachment_policy, resolve_note_abs_path,
    safe_file_name, NoteAttachmentPolicy, NotesState,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};
use veydan_core::Core;
use veydan_core::{AppError, CmdResult};
use veydan_sync::{
    sha256_hex, Engine, Hlc, HlcClock, LargeFileRef, LargeFileStore, Op, PathSink, PathSource,
    Progress, SyncError,
};
use veydan_sync_host::{Collected, Cycle, Handler, Host};

pub const ENTITY: &str = "note_attachment";
pub const ENTITY_V2: &str = "note_attachment_v2";
pub const EVENT_CHANGED: &str = "notes://attachments-changed";
/// Per-file transfer progress for the editor UI.
pub const EVENT_TRANSFER: &str = "notes://attachment-transfer";
/// `head_blob` marker for a v2 head: `lf:<manifest_id>`.
const LF_HEAD_PREFIX: &str = "lf:";

#[derive(Debug, Default, Serialize, Deserialize)]
struct AttachmentPayload {
    #[serde(default)]
    note_id: String,
    #[serde(default)]
    name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    blob: String,
    #[serde(default)]
    size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lf: Option<LargeFileRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    lf_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TransferEvent {
    pub note_id: String,
    pub name: String,
    /// "up" | "down"
    pub direction: &'static str,
    pub phase: String,
    pub done: u64,
    pub total: Option<u64>,
    pub error: Option<String>,
    pub finished: bool,
}

/// A name is safe when the sanitizer leaves it untouched.
fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 255 && safe_file_name(name) == name && !is_staging_name(name)
}

fn entity_id(note_id: &str, name: &str) -> String {
    format!("{note_id}/{name}")
}

pub fn transfer_key(note_id: &str, name: &str) -> String {
    entity_id(note_id, name)
}

fn head_for_v2(manifest_id: &str) -> String {
    format!("{LF_HEAD_PREFIX}{manifest_id}")
}

fn is_v2_head(head_blob: &str) -> bool {
    head_blob.starts_with(LF_HEAD_PREFIX)
}

fn op_with(
    entity_type: &str,
    note_id: &str,
    name: &str,
    hlc: Hlc,
    payload: AttachmentPayload,
) -> Op {
    Op {
        entity_type: entity_type.into(),
        entity_id: entity_id(note_id, name),
        hlc,
        deleted: false,
        payload: serde_json::to_value(payload).unwrap_or_default(),
    }
}

fn base_payload(note_id: &str, name: &str) -> AttachmentPayload {
    AttachmentPayload {
        note_id: note_id.into(),
        name: name.into(),
        ..Default::default()
    }
}

fn make_op_v1(note_id: &str, name: &str, hlc: Hlc, blob: String, size: u64) -> Op {
    op_with(
        ENTITY,
        note_id,
        name,
        hlc,
        AttachmentPayload {
            blob,
            size,
            ..base_payload(note_id, name)
        },
    )
}

fn make_op_v2(note_id: &str, name: &str, hlc: Hlc, lf: LargeFileRef) -> Op {
    let payload = AttachmentPayload {
        size: lf.size,
        lf_refs: vec![lf.manifest_id.clone()],
        lf: Some(lf),
        ..base_payload(note_id, name)
    };
    op_with(ENTITY_V2, note_id, name, hlc, payload)
}

fn delete_op(entity_type: &str, note_id: &str, name: &str, hlc: Hlc) -> Op {
    let mut op = op_with(entity_type, note_id, name, hlc, base_payload(note_id, name));
    op.deleted = true;
    op
}

/// Attachment directory: next to the note file, or the default location for a
/// note that has not arrived yet.
async fn attachment_dir(core: &Core, notes: &NotesState, note_id: &str) -> CmdResult<PathBuf> {
    let row: Option<(String,)> = sqlx::query_as("SELECT file_path FROM notes WHERE id = ?")
        .bind(note_id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?;
    Ok(match row {
        Some((file_path,)) => attachments_dir_for(
            &resolve_note_abs_path(&core.app_data_dir, &file_path),
            note_id,
        ),
        None => docs_dir(core, notes).join("attachments").join(note_id),
    })
}

/// Attachment names in a directory (sanitizer-safe, no staging files).
fn dir_names(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|name| valid_name(name))
        .collect()
}

fn emit_transfer(
    app: &AppHandle,
    note_id: &str,
    name: &str,
    direction: &'static str,
    p: &Progress,
) {
    let _ = app.emit(
        EVENT_TRANSFER,
        TransferEvent {
            note_id: note_id.into(),
            name: name.into(),
            direction,
            phase: format!("{:?}", p.phase).to_lowercase(),
            done: p.done,
            total: p.total,
            error: None,
            finished: false,
        },
    );
}

fn emit_transfer_end(
    app: &AppHandle,
    note_id: &str,
    name: &str,
    direction: &'static str,
    error: Option<String>,
) {
    let _ = app.emit(
        EVENT_TRANSFER,
        TransferEvent {
            note_id: note_id.into(),
            name: name.into(),
            direction,
            phase: "done".into(),
            done: 0,
            total: None,
            error,
            finished: true,
        },
    );
}

/// Large-file store for the joined vault with the device-local transfer settings.
async fn open_store<'a>(engine: &'a Engine, core: &Core) -> CmdResult<LargeFileStore<'a>> {
    let cfg = load_config(&core.db).await.large_files.to_config()?;
    engine.large_files(cfg).map_err(AppError::other)
}

// ── Push ─────────────────────────────────────────────────────────────────────

pub struct LocalChanges {
    pub ops: Vec<Op>,
    pub states: Vec<AttachmentSyncState>,
    /// Files that could not be uploaded this cycle; they stay dirty and come back.
    pub errors: Vec<String>,
}

/// Scan every attachment directory: new/changed files become put ops,
/// files that disappeared become tombstones. Without the app no transfer
/// events go out.
pub async fn collect_local_changes(
    engine: &Engine,
    app: Option<&AppHandle>,
    core: &Core,
    sync: &SyncManager,
    notes: &NotesState,
    clock: &mut HlcClock,
) -> CmdResult<LocalChanges> {
    let db = &core.db;
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT id, file_path FROM notes")
        .fetch_all(db)
        .await
        .map_err(AppError::db)?;
    let mut states = load_attachment_states(db).await?;
    let mut out = LocalChanges {
        ops: Vec::new(),
        states: Vec::new(),
        errors: Vec::new(),
    };
    let policy = load_attachment_policy(db).await;
    let store = open_store(engine, core).await?;

    // Directories to scan: next to each known note, plus default-location dirs
    // of notes whose row has not been created yet.
    let mut dirs: Vec<(String, PathBuf)> = rows
        .iter()
        .map(|(id, fp)| {
            (
                id.clone(),
                attachments_dir_for(&resolve_note_abs_path(&core.app_data_dir, fp), id),
            )
        })
        .collect();
    let known: HashSet<&String> = rows.iter().map(|(id, _)| id).collect();
    if let Ok(rd) = std::fs::read_dir(docs_dir(core, notes).join("attachments")) {
        for e in rd.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()) {
            let id = e.file_name().to_string_lossy().to_string();
            if valid_id(&id) && !known.contains(&id) {
                dirs.push((id, e.path()));
            }
        }
    }

    for (note_id, dir) in &dirs {
        for name in dir_names(dir) {
            let path = dir.join(&name);
            let Some(hash) = sync.file_hashes.file_hash(&path) else {
                continue;
            };
            let prev = states.remove(&(note_id.clone(), name.clone()));
            if prev
                .as_ref()
                .map(|s| s.synced_hash == hash && !s.deleted)
                .unwrap_or(false)
            {
                continue;
            }
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let published = if policy.uses_large_files(size) {
                match upload_large(&store, app, sync, &policy, note_id, &name, &path, size).await {
                    Ok(lf) => (
                        make_op_v2(note_id, &name, clock.now(), lf.clone()),
                        head_for_v2(&lf.manifest_id),
                    ),
                    Err(e) => {
                        out.errors.push(format!("{name}: {e}"));
                        continue;
                    }
                }
            } else {
                let Ok(raw) = std::fs::read(&path) else {
                    continue;
                };
                let blob = engine.put_blob(&raw).await.map_err(AppError::other)?;
                (
                    make_op_v1(note_id, &name, clock.now(), blob.clone(), raw.len() as u64),
                    blob,
                )
            };
            let (op, head_blob) = published;
            out.states.push(AttachmentSyncState {
                note_id: note_id.clone(),
                name,
                head_blob,
                head_hlc: Some(op.hlc.clone()),
                synced_hash: hash,
                deleted: false,
                deferred: None,
            });
            out.ops.push(op);
        }
    }

    // Whatever is left in `states` is no longer on disk. Deferred files were
    // never downloaded, so their absence is not a delete.
    for (_, mut st) in states
        .into_iter()
        .filter(|(_, s)| !s.deleted && s.deferred.is_none())
    {
        let hlc = clock.now();
        let entity_type = if is_v2_head(&st.head_blob) {
            ENTITY_V2
        } else {
            ENTITY
        };
        out.ops
            .push(delete_op(entity_type, &st.note_id, &st.name, hlc.clone()));
        st.deleted = true;
        st.head_hlc = Some(hlc);
        out.states.push(st);
    }
    Ok(out)
}

/// Chunked upload with progress events; errors are reported, not fatal.
#[allow(clippy::too_many_arguments)]
async fn upload_large(
    store: &LargeFileStore<'_>,
    app: Option<&AppHandle>,
    sync: &SyncManager,
    policy: &NoteAttachmentPolicy,
    note_id: &str,
    name: &str,
    path: &Path,
    size: u64,
) -> CmdResult<LargeFileRef> {
    let key = transfer_key(note_id, name);
    let cancel = sync.begin_transfer(&key);
    let result = match (policy.check_size(Some(size)), PathSource::new(path)) {
        (Err(e), _) => Err(e),
        (Ok(()), Err(e)) => Err(AppError::other(e)),
        (Ok(()), Ok(source)) => {
            let progress = |p: Progress| {
                if let Some(app) = app {
                    emit_transfer(app, note_id, name, "up", &p);
                    super::emit_progress(app, "attachments_up", 20, 0, 0, name);
                }
            };
            store
                .upload(&source, &progress, &cancel)
                .await
                .map_err(AppError::other)
        }
    };
    sync.end_transfer(&key);
    if let Some(app) = app {
        emit_transfer_end(
            app,
            note_id,
            name,
            "up",
            result.as_ref().err().map(|e| e.to_string()),
        );
    }
    result
}

// ── Pull ─────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ApplyOutcome {
    /// Notes whose attachment set changed; the editor refreshes its list.
    pub changed_notes: Vec<String>,
    /// Set when a blob was not yet available; peer heads must not advance.
    pub retry: Option<String>,
    /// Blobs collected by GC. Reported as warnings; peer heads still advance.
    pub skipped: Vec<String>,
}

/// Apply remote attachment ops of both forms (LWW by HLC on the shared state row).
/// Without the app no progress or transfer events go out.
pub async fn apply_remote(
    engine: &Engine,
    app: Option<&AppHandle>,
    core: &Core,
    sync: &SyncManager,
    notes: &NotesState,
    ops: &[Op],
) -> CmdResult<ApplyOutcome> {
    let db = &core.db;
    let mut outcome = ApplyOutcome::default();
    let is_ours = |op: &Op| op.entity_type == ENTITY || op.entity_type == ENTITY_V2;
    let total = ops.iter().filter(|op| is_ours(op) && !op.deleted).count() as u32;
    let mut current = 0u32;
    let store = open_store(engine, core).await?;
    let policy = load_attachment_policy(db).await;

    for op in ops.iter().filter(|op| is_ours(op)) {
        let payload: AttachmentPayload =
            serde_json::from_value(op.payload.clone()).unwrap_or_default();
        if !valid_id(&payload.note_id) || !valid_name(&payload.name) {
            continue;
        }
        let prev = load_attachment_state(db, &payload.note_id, &payload.name).await?;
        if prev
            .as_ref()
            .and_then(|s| s.head_hlc.as_ref())
            .map(|h| *h >= op.hlc)
            .unwrap_or(false)
        {
            continue;
        }
        let path = attachment_dir(core, notes, &payload.note_id)
            .await?
            .join(&payload.name);
        let mut st = prev.unwrap_or_else(|| AttachmentSyncState {
            note_id: payload.note_id.clone(),
            name: payload.name.clone(),
            ..Default::default()
        });

        if op.deleted {
            if path.is_file() {
                std::fs::remove_file(&path).map_err(AppError::io)?;
                outcome.changed_notes.push(payload.note_id.clone());
            }
            if st.deferred.take().is_some() {
                outcome.changed_notes.push(payload.note_id.clone());
            }
            st.deleted = true;
            st.head_hlc = Some(op.hlc.clone());
            save_attachment_state(db, &st).await?;
            continue;
        }

        current += 1;
        if let Some(app) = app {
            super::emit_progress(
                app,
                "apply",
                super::progress_pct(40, 44, current, total.max(1)),
                current,
                total,
                &payload.name,
            );
        }

        let applied = if op.entity_type == ENTITY_V2 {
            let Some(lf) = payload.lf.as_ref() else {
                continue;
            };
            apply_large(&store, app, sync, &policy, &payload, lf, &path, &mut st).await?
        } else {
            apply_blob(engine, op, &payload, &path, &mut st).await?
        };
        match applied {
            Applied::Written => outcome.changed_notes.push(payload.note_id.clone()),
            Applied::Unchanged => {}
            Applied::Gone(reason) => {
                outcome.skipped.push(reason);
                continue;
            }
            Applied::Retry(reason) => {
                outcome.retry.get_or_insert(reason);
                continue;
            }
        }
        st.head_hlc = Some(op.hlc.clone());
        st.deleted = false;
        save_attachment_state(db, &st).await?;
    }
    outcome.changed_notes.sort();
    outcome.changed_notes.dedup();
    Ok(outcome)
}

enum Applied {
    Written,
    Unchanged,
    Retry(String),
    Gone(String),
}

async fn apply_blob(
    engine: &Engine,
    op: &Op,
    payload: &AttachmentPayload,
    path: &Path,
    st: &mut AttachmentSyncState,
) -> CmdResult<Applied> {
    let Some(raw) = engine
        .get_blob(&payload.blob)
        .await
        .map_err(AppError::other)?
    else {
        let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
        if super::blob_gone(op, now_ms) {
            return Ok(Applied::Gone(format!(
                "blob for attachment {}/{} was collected",
                payload.note_id, payload.name
            )));
        }
        return Ok(Applied::Retry(format!(
            "blob for attachment {}/{} not available yet",
            payload.note_id, payload.name
        )));
    };
    st.synced_hash = sha256_hex(&raw);
    st.head_blob = payload.blob.clone();
    if std::fs::read(path).ok().as_deref() == Some(raw.as_slice()) {
        return Ok(Applied::Unchanged);
    }
    write_raw(&path.to_path_buf(), &raw)?;
    Ok(Applied::Written)
}

/// Download into `.veydanpart` staging; the file appears only after every chunk verified.
/// Files the policy keeps remote are recorded as deferred unless a local copy already exists.
#[allow(clippy::too_many_arguments)]
async fn apply_large(
    store: &LargeFileStore<'_>,
    app: Option<&AppHandle>,
    sync: &SyncManager,
    policy: &NoteAttachmentPolicy,
    payload: &AttachmentPayload,
    lf: &LargeFileRef,
    path: &Path,
    st: &mut AttachmentSyncState,
) -> CmdResult<Applied> {
    let head = head_for_v2(&lf.manifest_id);
    let current_hash = sync.file_hashes.file_hash(path);
    if st.head_blob == head && current_hash.as_deref() == Some(st.synced_hash.as_str()) {
        return Ok(Applied::Unchanged);
    }
    if current_hash.is_none() && !policy.downloads_on_sync(lf.size) {
        st.deferred = Some(lf.clone());
        st.synced_hash.clear();
        st.head_blob = head;
        return Ok(Applied::Written);
    }
    let result = download_large(store, app, sync, &payload.note_id, &payload.name, lf, path).await;
    match result {
        Ok(()) => {}
        // Not fully published yet, or the user paused it: come back next cycle.
        Err(SyncError::Storage(msg)) => {
            return Ok(Applied::Retry(format!(
                "{}/{}: {msg}",
                payload.note_id, payload.name
            )))
        }
        Err(SyncError::Cancelled) => {
            return Ok(Applied::Retry(format!(
                "{}/{} cancelled",
                payload.note_id, payload.name
            )))
        }
        Err(e) => return Err(AppError::other(e)),
    }
    st.synced_hash = sync.file_hashes.file_hash(path).unwrap_or_default();
    st.head_blob = head;
    st.deferred = None;
    Ok(Applied::Written)
}

/// Chunked download with progress events and a cancel flag registered for the UI.
async fn download_large(
    store: &LargeFileStore<'_>,
    app: Option<&AppHandle>,
    sync: &SyncManager,
    note_id: &str,
    name: &str,
    lf: &LargeFileRef,
    path: &Path,
) -> veydan_sync::Result<()> {
    let key = transfer_key(note_id, name);
    let cancel = sync.begin_transfer(&key);
    let sink = PathSink::new(path);
    let progress = |p: Progress| {
        if let Some(app) = app {
            emit_transfer(app, note_id, name, "down", &p);
            super::emit_progress(app, "attachments_down", 42, 0, 0, name);
        }
    };
    let result = store.download(lf, &sink, &progress, &cancel).await;
    sync.end_transfer(&key);
    if let Some(app) = app {
        emit_transfer_end(
            app,
            note_id,
            name,
            "down",
            result.as_ref().err().map(|e| e.to_string()),
        );
    }
    result
}

/// A remote-only attachment as the editor lists it.
pub struct DeferredAttachment {
    pub name: String,
    pub size: u64,
}

pub async fn deferred_for_note(core: &Core, note_id: &str) -> CmdResult<Vec<DeferredAttachment>> {
    Ok(load_deferred_attachments(&core.db, note_id)
        .await?
        .into_iter()
        .filter_map(|s| {
            s.deferred.map(|lf| DeferredAttachment {
                name: s.name,
                size: lf.size,
            })
        })
        .collect())
}

/// Download one deferred attachment on request; the file then syncs like any other.
pub async fn fetch_deferred(
    app: &AppHandle,
    core: &Core,
    sync: &SyncManager,
    notes: &NotesState,
    note_id: &str,
    name: &str,
) -> CmdResult<()> {
    let mut st = load_attachment_state(&core.db, note_id, name)
        .await?
        .filter(|s| !s.deleted)
        .ok_or_else(|| AppError::not_found(format!("attachment {note_id}/{name}")))?;
    let lf = st
        .deferred
        .clone()
        .ok_or_else(|| AppError::other("attachment is already downloaded"))?;
    let engine = super::open_engine(core).await?;
    let store = open_store(&engine, core).await?;
    let path = attachment_dir(core, notes, note_id).await?.join(name);
    download_large(&store, Some(app), sync, note_id, name, &lf, &path)
        .await
        .map_err(AppError::other)?;
    st.synced_hash = sync.file_hashes.file_hash(&path).unwrap_or_default();
    st.deferred = None;
    save_attachment_state(&core.db, &st).await?;
    let _ = app.emit(EVENT_CHANGED, note_id);
    Ok(())
}

/// Tell the editor which notes got new or removed attachments.
pub fn notify(app: &AppHandle, outcome: &ApplyOutcome) {
    for id in &outcome.changed_notes {
        let _ = app.emit(EVENT_CHANGED, id);
    }
}

/// Both forms of attachments as entities of sync, with a handler of their own.
pub(crate) struct Attachments;

#[async_trait::async_trait]
impl Handler for Attachments {
    fn entities(&self) -> &'static [&'static str] {
        &[ENTITY, ENTITY_V2]
    }

    fn state_tables(&self) -> &'static [&'static str] {
        &["sync_attachment_state"]
    }

    async fn collect(&self, cx: &mut Cycle<'_>) -> CmdResult<Collected> {
        let notes = cx
            .host
            .state::<NotesState>()
            .ok_or_else(|| AppError::other("notes are not set up"))?;
        let changes =
            collect_local_changes(cx.engine, cx.host.app(), cx.core, cx.sync, notes, cx.clock)
                .await?;
        let states = changes.states;
        Ok(Collected {
            ops: changes.ops,
            commit: Some(Box::new(move |db| {
                Box::pin(async move {
                    for st in &states {
                        save_attachment_state(&db, st).await?;
                    }
                    Ok(())
                })
            })),
            warnings: changes
                .errors
                .iter()
                .map(|e| format!("attachment upload: {e}"))
                .collect(),
            errors: changes.errors,
        })
    }

    async fn apply(&self, cx: &mut Cycle<'_>, ops: &[Op]) -> CmdResult<veydan_sync_host::Applied> {
        let notes = cx
            .host
            .state::<NotesState>()
            .ok_or_else(|| AppError::other("notes are not set up"))?;
        let outcome = apply_remote(cx.engine, cx.host.app(), cx.core, cx.sync, notes, ops).await?;
        Ok(veydan_sync_host::Applied {
            retry: outcome.retry.clone(),
            skipped: outcome.skipped.clone(),
            detail: Some(Box::new(outcome)),
        })
    }

    fn notify(&self, host: &Host<'_>, applied: &veydan_sync_host::Applied) {
        let outcome = applied
            .detail
            .as_ref()
            .and_then(|detail| detail.downcast_ref::<ApplyOutcome>());
        if let (Some(app), Some(outcome)) = (host.app(), outcome) {
            notify(app, outcome);
        }
    }

    /// A v1 attachment keeps its blob; a v2 one keeps large files, which the
    /// collection of large files finds by `lf_refs`.
    async fn blob_refs(&self, _engine: &Engine, op: &Op) -> CmdResult<Vec<String>> {
        if op.entity_type != ENTITY {
            return Ok(Vec::new());
        }
        Ok(op
            .payload
            .get("blob")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .into_iter()
            .collect())
    }
}

/// Both attachment entities are part of the vault's wire format, frozen at
/// what a 4.0.7 client writes and reads.
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hlc() -> Hlc {
        Hlc {
            wall_ms: 1,
            counter: 0,
            device_id: "dev-a".into(),
        }
    }

    #[test]
    fn a_v1_attachment_of_4_0_7_is_read_and_written_alike() {
        let wire = json!({ "note_id": "n1", "name": "scan.pdf", "blob": "b7", "size": 2048 });

        let read: AttachmentPayload = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(
            (read.note_id.as_str(), read.name.as_str()),
            ("n1", "scan.pdf")
        );
        assert_eq!((read.blob.as_str(), read.size), ("b7", 2048));
        assert!(read.lf.is_none() && read.lf_refs.is_empty());

        let op = make_op_v1("n1", "scan.pdf", hlc(), "b7".into(), 2048);
        assert_eq!(op.entity_type, "note_attachment");
        assert_eq!(op.entity_id, "n1/scan.pdf");
        assert_eq!(op.payload, wire);
    }

    #[test]
    fn a_v2_attachment_of_4_0_7_is_read_and_written_alike() {
        let wire = json!({
            "note_id": "n1",
            "name": "video.mp4",
            "size": 734003200u64,
            "lf": { "version": 1, "manifest_id": "m1", "size": 734003200u64, "file_id": "f1" },
            "lf_refs": ["m1"],
        });

        let read: AttachmentPayload = serde_json::from_value(wire.clone()).unwrap();
        let lf = read.lf.expect("large file ref");
        assert_eq!(
            (lf.version, lf.manifest_id.as_str(), lf.file_id.as_str()),
            (1, "m1", "f1")
        );
        assert_eq!(read.lf_refs, vec!["m1"]);
        assert!(read.blob.is_empty());

        let op = make_op_v2("n1", "video.mp4", hlc(), lf);
        assert_eq!(op.entity_type, "note_attachment_v2");
        assert_eq!(op.entity_id, "n1/video.mp4");
        assert_eq!(op.payload, wire);
    }

    /// Every attachment op 4.0.7 wrote to a vault, of both forms, puts and
    /// tombstones, reads into the payload and comes out of it as 4.0.7
    /// wrote it.
    #[test]
    fn every_attachment_op_4_0_7_wrote_is_read_and_written_alike() {
        for entity in [ENTITY, ENTITY_V2] {
            let ops = crate::sync::golden::ops(entity);
            assert!(ops.iter().any(|op| op.deleted) && ops.iter().any(|op| !op.deleted));
            for theirs in ops {
                let read: AttachmentPayload =
                    serde_json::from_value(theirs.payload.clone()).unwrap();
                assert_eq!(theirs.entity_id, format!("{}/{}", read.note_id, read.name));
                let hlc = theirs.hlc.clone();
                let ours = match (theirs.deleted, read.lf) {
                    (true, _) => delete_op(entity, &read.note_id, &read.name, hlc),
                    (false, Some(lf)) => make_op_v2(&read.note_id, &read.name, hlc, lf),
                    (false, None) => {
                        make_op_v1(&read.note_id, &read.name, hlc, read.blob, read.size)
                    }
                };
                assert_eq!(
                    serde_json::to_value(&ours).unwrap(),
                    serde_json::to_value(&theirs).unwrap()
                );
            }
        }
    }
}
