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
    safe_file_name, unique_path, NoteAttachmentPolicy, NotesState,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
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

/// Attachment names in a directory (sanitizer-safe, no staging files). A
/// folder that cannot be listed is an error, not an empty folder; an entry
/// whose type cannot be read is listed, and its read reports it.
fn dir_names(dir: &Path) -> std::io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if !valid_name(&name) {
            continue;
        }
        match std::fs::metadata(entry.path()) {
            Ok(meta) if !meta.is_file() => {}
            // Gone since the listing, or a link to nothing.
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            _ => names.push(name),
        }
    }
    Ok(names)
}

/// Leave out of this cycle every attachment of the notes `withheld` takes:
/// their folder was not read, and what was not read is not deleted. True
/// when one of them was on this device (it would have gone as a tombstone).
fn withhold(
    states: &mut HashMap<(String, String), AttachmentSyncState>,
    withheld: impl Fn(&str) -> bool,
) -> bool {
    let live = states
        .iter()
        .any(|((id, _), s)| withheld(id) && !s.deleted && s.deferred.is_none());
    states.retain(|(id, _), _| !withheld(id));
    live
}

/// Name for a local file the vault does not know, kept beside the remote
/// version that takes its name: `<stem> (conflict).<ext>`.
fn conflict_name(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem} (conflict).{ext}"),
        _ => format!("{name} (conflict)"),
    }
}

/// Keep the local file at `path` under a conflict name before a remote
/// version is written there: a hard link (a copy where links are not
/// supported), so the name stays whole if the write fails, and the remote
/// write replaces the file, not the content of the copy. The next collect
/// publishes the copy as an attachment of its own.
fn keep_local_copy(path: &Path, reserved: &HashSet<String>) -> CmdResult<PathBuf> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str())) else {
        return Err(AppError::other("attachment path has no name"));
    };
    let copy = unique_path(dir, &conflict_name(name), reserved);
    if std::fs::hard_link(path, &copy).is_err() {
        std::fs::copy(path, &copy).map_err(AppError::io)?;
    }
    Ok(copy)
}

/// Names of a note's attachments that are only in the vault: a new local
/// file must not take one (it would stand for that file on every device).
pub(crate) async fn deferred_names(core: &Core, note_id: &str) -> CmdResult<HashSet<String>> {
    Ok(deferred_for_note(core, note_id)
        .await?
        .into_iter()
        .map(|d| d.name)
        .collect())
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
    // Only a note id names a directory: `.` or `..` would be `attachments/`
    // itself or the documents folder.
    // Each with the note file, when the note has a row.
    let mut dirs: Vec<(String, PathBuf, Option<PathBuf>)> = rows
        .iter()
        .filter(|(id, _)| valid_id(id))
        .map(|(id, fp)| {
            let note_file = resolve_note_abs_path(&core.app_data_dir, fp);
            (
                id.clone(),
                attachments_dir_for(&note_file, id),
                Some(note_file),
            )
        })
        .collect();
    let known: HashSet<&String> = rows.iter().map(|(id, _)| id).collect();
    let docs = docs_dir(core, notes);
    // False when a folder of a note without a row may have been missed.
    let mut listed_all = true;
    match std::fs::read_dir(docs.join("attachments")) {
        Ok(rd) => {
            for e in rd {
                let Ok(e) = e else {
                    listed_all = false;
                    continue;
                };
                let id = e.file_name().to_string_lossy().to_string();
                if e.path().is_dir() && valid_id(&id) && !known.contains(&id) {
                    dirs.push((id, e.path(), None));
                }
            }
        }
        Err(e) => listed_all = e.kind() == ErrorKind::NotFound && docs.is_dir(),
    }
    let scanned: HashSet<String> = dirs.iter().map(|(id, _, _)| id.clone()).collect();
    if !listed_all
        && withhold(&mut states, |id| {
            !scanned.contains(id) && !known.contains(&id.to_string())
        })
    {
        out.errors
            .push(format!("{}: not read", docs.join("attachments").display()));
    }

    for (note_id, dir, note_file) in &dirs {
        let names = match dir_names(dir) {
            Ok(names) => names,
            // No folder beside a note that is there: its attachments were
            // all removed. Without the note file too, the folder may have
            // moved or its disk is not mounted.
            Err(e)
                if e.kind() == ErrorKind::NotFound
                    && note_file.as_ref().is_some_and(|f| f.is_file()) =>
            {
                Vec::new()
            }
            Err(e) => {
                if withhold(&mut states, |id| id == note_id.as_str())
                    || e.kind() != ErrorKind::NotFound
                {
                    out.errors.push(format!("{}: {e}", dir.display()));
                }
                continue;
            }
        };
        for name in names {
            let path = dir.join(&name);
            let key = (note_id.clone(), name.clone());
            let hash = match sync.file_hashes.try_file_hash(&path) {
                Ok(hash) => hash,
                // Not read is not deleted: the file keeps its state. One gone
                // since the listing is a tombstone of the next cycle.
                Err(e) => {
                    states.remove(&key);
                    if e.kind() != ErrorKind::NotFound {
                        out.errors.push(format!("{name}: {e}"));
                    }
                    continue;
                }
            };
            let prev = states.remove(&key);
            // The vault holds a file of this name this device has not
            // downloaded (`fetch_deferred` clears `deferred`): this one is
            // another file, and its put would replace that one everywhere.
            if prev.as_ref().is_some_and(|s| s.deferred.is_some()) {
                out.errors.push(format!(
                    "{name}: not sent, the vault holds another file of this name; download or rename it"
                ));
                continue;
            }
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
                let raw = match std::fs::read(&path) {
                    Ok(raw) => raw,
                    Err(e) => {
                        out.errors.push(format!("{name}: {e}"));
                        continue;
                    }
                };
                // A full pool (`quota`) or a failed upload stops this file
                // only: the rest of the cycle goes on, as for large files.
                let blob = match engine.put_blob(&raw).await {
                    Ok(blob) => blob,
                    Err(e) => {
                        out.errors.push(format!("{name}: {e}"));
                        continue;
                    }
                };
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
    // never downloaded, so their absence is not a delete. A note that is
    // not a note id had no directory scanned: its files are not gone. The
    // notes whose folder was not read are out of `states` already.
    for (_, mut st) in states
        .into_iter()
        .filter(|(_, s)| !s.deleted && s.deferred.is_none() && valid_id(&s.note_id))
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
    /// The devices of the ops `retry` is for: only their heads stay.
    pub retry_from: Vec<String>,
    /// Blobs collected by GC or quarantined (they do not open). Reported as
    /// warnings; peer heads still advance.
    pub skipped: Vec<String>,
}

impl ApplyOutcome {
    /// `op` is delivered again next cycle; the log it came in keeps its head.
    fn retry_op(&mut self, op: &Op, reason: String) {
        self.retry.get_or_insert(reason);
        if !self.retry_from.contains(&op.hlc.device_id) {
            self.retry_from.push(op.hlc.device_id.clone());
        }
    }
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
                // A file that cannot be removed (in use, a share gone) holds
                // this op alone; the rest of the pull goes on.
                if let Err(e) = std::fs::remove_file(&path) {
                    outcome.retry_op(
                        op,
                        format!("attachment {}/{}: {e}", payload.note_id, payload.name),
                    );
                    continue;
                }
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

        let reserved = deferred_names(core, &payload.note_id).await?;
        let applied = if op.entity_type == ENTITY_V2 {
            let Some(lf) = payload.lf.as_ref() else {
                continue;
            };
            apply_large(
                &store, app, sync, &policy, op, &payload, lf, &path, &reserved, &mut st,
            )
            .await
        } else {
            apply_blob(engine, op, &payload, &path, &reserved, &mut st).await
        };
        // A copy beside the file, the write or the download failing holds
        // this op alone: the state is not saved and it comes again.
        let applied = applied.unwrap_or_else(|e| {
            Applied::Retry(format!(
                "attachment {}/{}: {e}",
                payload.note_id, payload.name
            ))
        });
        match applied {
            Applied::Written => outcome.changed_notes.push(payload.note_id.clone()),
            Applied::Unchanged => {}
            Applied::Gone(reason) => {
                outcome.skipped.push(reason);
                continue;
            }
            Applied::Retry(reason) => {
                outcome.retry_op(op, reason);
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

/// `reserved`: the names a conflict copy must not take (`deferred_names`).
async fn apply_blob(
    engine: &Engine,
    op: &Op,
    payload: &AttachmentPayload,
    path: &Path,
    reserved: &HashSet<String>,
    st: &mut AttachmentSyncState,
) -> CmdResult<Applied> {
    let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let fetched = match engine.get_blob(&payload.blob).await {
        Ok(fetched) => fetched,
        // Long after its op: quarantined, skipped and the heads advance.
        Err(e) if super::blob_quarantined(op, &e, now_ms) => {
            return Ok(Applied::Gone(format!(
                "blob for attachment {}/{} does not open, skipped: {e}",
                payload.note_id, payload.name
            )));
        }
        Err(e) => {
            return Ok(Applied::Retry(format!(
                "blob for attachment {}/{}: {e}",
                payload.note_id, payload.name
            )));
        }
    };
    let Some(raw) = fetched else {
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
    let local = match std::fs::read(path) {
        Ok(local) => Some(local),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        // A file that cannot be read is not overwritten unseen.
        Err(e) => {
            return Ok(Applied::Retry(format!(
                "attachment {}/{}: {e}",
                payload.note_id, payload.name
            )))
        }
    };
    let synced = std::mem::replace(&mut st.synced_hash, sha256_hex(&raw));
    st.head_blob = payload.blob.clone();
    if local.as_deref() == Some(raw.as_slice()) {
        return Ok(Applied::Unchanged);
    }
    // A local version this device never synced stays beside the remote one.
    let copy = match &local {
        Some(local) if synced.is_empty() || sha256_hex(local) != synced => {
            Some(keep_local_copy(path, reserved)?)
        }
        _ => None,
    };
    if let Err(e) = write_raw(&path.to_path_buf(), &raw) {
        if let Some(copy) = copy {
            let _ = std::fs::remove_file(copy);
        }
        return Err(e);
    }
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
    op: &Op,
    payload: &AttachmentPayload,
    lf: &LargeFileRef,
    path: &Path,
    reserved: &HashSet<String>,
    st: &mut AttachmentSyncState,
) -> CmdResult<Applied> {
    let head = head_for_v2(&lf.manifest_id);
    let current_hash = match sync.file_hashes.try_file_hash(path) {
        Ok(hash) => Some(hash),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        // A file that cannot be read is not overwritten unseen.
        Err(e) => {
            return Ok(Applied::Retry(format!(
                "attachment {}/{}: {e}",
                payload.note_id, payload.name
            )))
        }
    };
    if st.head_blob == head && current_hash.as_deref() == Some(st.synced_hash.as_str()) {
        return Ok(Applied::Unchanged);
    }
    if current_hash.is_none() && !policy.downloads_on_sync(lf.size) {
        st.deferred = Some(lf.clone());
        st.synced_hash.clear();
        st.head_blob = head;
        return Ok(Applied::Written);
    }
    let result = download_keeping_local(
        store,
        app,
        sync,
        (&payload.note_id, &payload.name),
        lf,
        path,
        current_hash.as_deref(),
        &st.synced_hash,
        reserved,
    )
    .await?;
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
        // A manifest or a chunk that does not open, long after its op, is
        // quarantined; anything else comes back next cycle.
        Err(e) => {
            let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
            let reason = format!("{}/{}: {e}", payload.note_id, payload.name);
            if super::blob_quarantined(op, &e, now_ms) {
                return Ok(Applied::Gone(format!(
                    "attachment {reason}: does not open, skipped"
                )));
            }
            return Ok(Applied::Retry(reason));
        }
    }
    st.synced_hash = sync.file_hashes.file_hash(path).unwrap_or_default();
    st.head_blob = head;
    st.deferred = None;
    Ok(Applied::Written)
}

/// `download_large` over a local file whose hash (`local`) is not `synced`
/// (empty: this device synced none): that file is kept as a conflict copy,
/// which goes again when the download fails or brought the same bytes.
#[allow(clippy::too_many_arguments)]
async fn download_keeping_local(
    store: &LargeFileStore<'_>,
    app: Option<&AppHandle>,
    sync: &SyncManager,
    (note_id, name): (&str, &str),
    lf: &LargeFileRef,
    path: &Path,
    local: Option<&str>,
    synced: &str,
    reserved: &HashSet<String>,
) -> CmdResult<veydan_sync::Result<()>> {
    let copy = match local {
        Some(hash) if synced.is_empty() || hash != synced => Some(keep_local_copy(path, reserved)?),
        _ => None,
    };
    let result = download_large(store, app, sync, note_id, name, lf, path).await;
    if let Some(copy) = copy {
        if result.is_err() || sync.file_hashes.file_hash(path).as_deref() == local {
            let _ = std::fs::remove_file(copy);
        }
    }
    Ok(result)
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
    // A file that took the name meanwhile is not the deferred one: it is kept.
    let local = match sync.file_hashes.try_file_hash(&path) {
        Ok(hash) => Some(hash),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => return Err(AppError::io(e)),
    };
    let reserved = deferred_names(core, note_id).await?;
    download_keeping_local(
        &store,
        Some(app),
        sync,
        (note_id, name),
        &lf,
        &path,
        local.as_deref(),
        "",
        &reserved,
    )
    .await?
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
            retry_from: outcome.retry_from.clone(),
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

    /// A file of an attachment pushed after `newest` was lost with its chunk: its state
    /// forgets the push, and the next collect publishes it again, or its
    /// tombstone.
    async fn forget_pushed_after(
        &self,
        tx: &mut sqlx::SqliteConnection,
        device: &str,
        newest: Option<&Hlc>,
    ) -> CmdResult<()> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE sync_attachment_state SET synced_hash = '', deleted = 0 WHERE {}",
            veydan_sync_host::state::pushed_after("head_hlc")
        )))
        .bind(device)
        .bind(veydan_sync_host::state::hlc_floor(newest))
        .execute(tx)
        .await
        .map_err(AppError::db)?;
        Ok(())
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

    /// A vault on Veydan Cloud whose pool `files` is full: `blobs/` answers
    /// `413 quota`, the journal still goes in (spec/vstore.md §9).
    struct FullFiles(veydan_sync::LocalDir);

    #[async_trait::async_trait]
    impl veydan_sync::Storage for FullFiles {
        async fn list(&self, prefix: &str) -> veydan_sync::Result<Vec<String>> {
            self.0.list(prefix).await
        }
        async fn get(&self, key: &str) -> veydan_sync::Result<Option<Vec<u8>>> {
            self.0.get(key).await
        }
        async fn put(&self, key: &str, data: &[u8]) -> veydan_sync::Result<()> {
            if key.starts_with("blobs/") {
                return Err(SyncError::Quota {
                    pool: "files".into(),
                    used: 10,
                    limit: 10,
                });
            }
            self.0.put(key, data).await
        }
        async fn delete(&self, key: &str) -> veydan_sync::Result<()> {
            self.0.delete(key).await
        }
    }

    /// Plan 4.7: a full pool stops the files, not the sync.
    #[tokio::test]
    async fn a_full_pool_stops_the_file_and_not_the_cycle() {
        let state = crate::testing::States::new().await;
        let vault = tempfile::tempdir().unwrap();
        let storage = FullFiles(veydan_sync::LocalDir::new(vault.path()));
        let (engine, _) = Engine::create(Box::new(storage), "passphrase", "dev-a")
            .await
            .unwrap();
        let dir = docs_dir(&state.core, &state.notes)
            .join("attachments")
            .join("n1");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("scan.txt"), b"a small file").unwrap();
        let mut clock = HlcClock::new("dev-a", None);

        let changes = collect_local_changes(
            &engine,
            None,
            &state.core,
            &state.sync,
            &state.notes,
            &mut clock,
        )
        .await
        .expect("the cycle goes on");
        assert!(changes.ops.is_empty() && changes.states.is_empty());
        assert_eq!(changes.errors.len(), 1, "{:?}", changes.errors);
        assert!(
            changes.errors[0].starts_with("scan.txt: quota"),
            "{:?}",
            changes.errors
        );
        // The rest of the cycle: the journal is in the other pool.
        let mut log = veydan_sync::LocalState::default();
        let op = make_op_v1("n1", "other.txt", clock.now(), "b7".into(), 1);
        engine
            .push(&mut log, vec![op])
            .await
            .expect("the journal goes");
    }

    async fn local_vault() -> (Engine, tempfile::TempDir) {
        let vault = tempfile::tempdir().unwrap();
        let storage = veydan_sync::LocalDir::new(vault.path());
        let (engine, _) = Engine::create(Box::new(storage), "passphrase", "dev-a")
            .await
            .unwrap();
        (engine, vault)
    }

    async fn collect(state: &crate::testing::States, engine: &Engine) -> LocalChanges {
        let mut clock = HlcClock::new("dev-a", None);
        collect_local_changes(
            engine,
            None,
            &state.core,
            &state.sync,
            &state.notes,
            &mut clock,
        )
        .await
        .unwrap()
    }

    async fn add_note(state: &crate::testing::States, id: &str) -> PathBuf {
        let file = docs_dir(&state.core, &state.notes).join(format!("{id}.md"));
        std::fs::write(&file, "body").unwrap();
        sqlx::query(
            "INSERT INTO notes (id, title, file_path, created_at, updated_at)
             VALUES (?, ?, ?, '2026-01-01', '2026-01-01')",
        )
        .bind(id)
        .bind(id)
        .bind(file.to_string_lossy().to_string())
        .execute(&state.core.db)
        .await
        .unwrap();
        file
    }

    async fn synced(state: &crate::testing::States, note_id: &str, name: &str) {
        let st = AttachmentSyncState {
            note_id: note_id.into(),
            name: name.into(),
            head_blob: "b1".into(),
            head_hlc: Some(hlc()),
            synced_hash: sha256_hex(b"png"),
            deleted: false,
            deferred: None,
        };
        save_attachment_state(&state.core.db, &st).await.unwrap();
    }

    /// SYNCCORE-06: a folder that cannot be read, or that is gone with its
    /// note file (moved, a disk not mounted), sends no tombstone; a folder
    /// gone beside a note that is there is the removal of its files.
    #[tokio::test]
    async fn an_attachment_folder_not_read_is_not_a_deletion() {
        let state = crate::testing::States::new().await;
        let (engine, _vault) = local_vault().await;
        let note = add_note(&state, "n1").await;
        synced(&state, "n1", "a.png").await;
        let dir = attachments_dir_for(&note, "n1");

        // The folder is a file.
        std::fs::create_dir_all(dir.parent().unwrap()).unwrap();
        std::fs::write(&dir, b"not a folder").unwrap();
        let changes = collect(&state, &engine).await;
        assert!(changes.ops.is_empty(), "{:?}", changes.ops);
        assert_eq!(changes.errors.len(), 1, "{:?}", changes.errors);

        // The folder and the note file are both gone.
        std::fs::remove_file(&dir).unwrap();
        std::fs::remove_file(&note).unwrap();
        let changes = collect(&state, &engine).await;
        assert!(changes.ops.is_empty(), "{:?}", changes.ops);
        assert_eq!(changes.errors.len(), 1, "{:?}", changes.errors);

        // The note is there and its folder is not: its files were removed.
        std::fs::write(&note, "body").unwrap();
        let changes = collect(&state, &engine).await;
        assert_eq!(changes.ops.len(), 1);
        assert!(changes.ops[0].deleted && changes.ops[0].entity_id == "n1/a.png");
        assert!(changes.errors.is_empty(), "{:?}", changes.errors);
    }

    /// SYNCROWS-10: a remote put over a local file of the same name that
    /// this device never synced keeps the local one beside it, and the next
    /// collect sends that copy; a file still as synced is replaced.
    #[tokio::test]
    async fn a_remote_put_keeps_the_local_file_beside_it() {
        let state = crate::testing::States::new().await;
        let (engine, _vault) = local_vault().await;
        let dir = docs_dir(&state.core, &state.notes)
            .join("attachments")
            .join("n1");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), b"local").unwrap();
        let mut clock = HlcClock::new("dev-b", None);

        let blob = engine.put_blob(b"remote").await.unwrap();
        let op = make_op_v1("n1", "a.txt", clock.now(), blob, 6);
        let outcome = apply_remote(&engine, None, &state.core, &state.sync, &state.notes, &[op])
            .await
            .unwrap();
        assert!(outcome.retry.is_none());
        assert_eq!(std::fs::read(dir.join("a.txt")).unwrap(), b"remote");
        assert_eq!(
            std::fs::read(dir.join("a (conflict).txt")).unwrap(),
            b"local"
        );

        // The file is as this device synced it: the next version replaces it.
        let blob = engine.put_blob(b"remote 2").await.unwrap();
        let op = make_op_v1("n1", "a.txt", clock.now(), blob, 8);
        apply_remote(&engine, None, &state.core, &state.sync, &state.notes, &[op])
            .await
            .unwrap();
        assert_eq!(std::fs::read(dir.join("a.txt")).unwrap(), b"remote 2");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);

        let changes = collect(&state, &engine).await;
        let ids: Vec<&str> = changes.ops.iter().map(|op| op.entity_id.as_str()).collect();
        assert_eq!(ids, ["n1/a (conflict).txt"]);
    }

    /// SYNCROWS-09: a file that cannot be written holds its op alone, and
    /// the retry names the device of that op; the attachment after it is
    /// written. The next delivery writes the held one, and the one applied
    /// already is cut by its head.
    #[tokio::test]
    async fn an_attachment_that_cannot_be_written_is_retried_alone() {
        let state = crate::testing::States::new().await;
        let (engine, _vault) = local_vault().await;
        let attachments = docs_dir(&state.core, &state.notes).join("attachments");
        // The temporary file of the write is a folder.
        let blocker = attachments.join("n1").join("a.tmp");
        std::fs::create_dir_all(&blocker).unwrap();
        let mut clock_b = HlcClock::new("dev-b", None);
        let mut clock_c = HlcClock::new("dev-c", None);
        let held = engine.put_blob(b"held").await.unwrap();
        let fine = engine.put_blob(b"fine").await.unwrap();
        let ops = [
            make_op_v1("n1", "a.txt", clock_b.now(), held, 4),
            make_op_v1("n2", "b.txt", clock_c.now(), fine, 4),
        ];

        let outcome = apply_remote(&engine, None, &state.core, &state.sync, &state.notes, &ops)
            .await
            .unwrap();

        assert!(outcome
            .retry
            .as_deref()
            .is_some_and(|r| r.contains("n1/a.txt")));
        assert_eq!(outcome.retry_from, ["dev-b"]);
        assert_eq!(outcome.changed_notes, ["n2"]);
        assert_eq!(
            std::fs::read(attachments.join("n2").join("b.txt")).unwrap(),
            b"fine"
        );
        assert!(!attachments.join("n1").join("a.txt").exists());

        std::fs::remove_dir(&blocker).unwrap();
        let outcome = apply_remote(&engine, None, &state.core, &state.sync, &state.notes, &ops)
            .await
            .unwrap();

        assert!(outcome.retry.is_none());
        assert_eq!(outcome.changed_notes, ["n1"]);
        assert_eq!(
            std::fs::read(attachments.join("n1").join("a.txt")).unwrap(),
            b"held"
        );
    }

    /// SYNCROWS-09: a blob that does not open, long after its op, is
    /// quarantined (a warning, no retry); a young one is retried.
    #[tokio::test]
    async fn an_attachment_blob_that_does_not_open_is_quarantined_once_old() {
        let state = crate::testing::States::new().await;
        let (engine, vault) = local_vault().await;
        let blob = engine.put_blob(b"broken").await.unwrap();
        let path = vault.path().join("blobs").join(&blob);
        let mut raw = std::fs::read(&path).unwrap();
        *raw.last_mut().unwrap() ^= 0xff;
        std::fs::write(&path, raw).unwrap();
        let now = chrono::Utc::now().timestamp_millis() as u64;
        let stamp = |wall_ms| Hlc {
            wall_ms,
            counter: 0,
            device_id: "dev-b".into(),
        };

        let young = make_op_v1("n1", "a.txt", stamp(now), blob.clone(), 6);
        let outcome = apply_remote(
            &engine,
            None,
            &state.core,
            &state.sync,
            &state.notes,
            &[young],
        )
        .await
        .unwrap();
        assert!(outcome.retry.is_some());
        assert_eq!(outcome.retry_from, ["dev-b"]);
        assert!(outcome.skipped.is_empty(), "{:?}", outcome.skipped);

        let old = make_op_v1("n1", "a.txt", stamp(now - 2 * 24 * 3600 * 1000), blob, 6);
        let outcome = apply_remote(
            &engine,
            None,
            &state.core,
            &state.sync,
            &state.notes,
            &[old],
        )
        .await
        .unwrap();
        assert!(outcome.retry.is_none());
        assert_eq!(outcome.skipped.len(), 1, "{:?}", outcome.skipped);
        assert!(
            outcome.skipped[0].contains("does not open"),
            "{:?}",
            outcome.skipped
        );
    }

    /// MOBILE-11: a file the vault holds and this device has not downloaded
    /// keeps its name: a new local file takes `a-1.png`, and a file put
    /// there by hand is not sent in its place.
    #[tokio::test]
    async fn a_deferred_name_is_not_taken_by_a_local_file() {
        let state = crate::testing::States::new().await;
        let (engine, _vault) = local_vault().await;
        let lf: LargeFileRef = serde_json::from_value(json!({
            "version": 1, "manifest_id": "m1", "size": 734003200u64, "file_id": "f1",
        }))
        .unwrap();
        let st = AttachmentSyncState {
            note_id: "n1".into(),
            name: "a.png".into(),
            head_blob: head_for_v2("m1"),
            head_hlc: Some(hlc()),
            deferred: Some(lf),
            ..Default::default()
        };
        save_attachment_state(&state.core.db, &st).await.unwrap();
        let note = docs_dir(&state.core, &state.notes).join("n1.md");

        let reserved = deferred_names(&state.core, "n1").await.unwrap();
        let added = crate::attachments::store_attachment_stream(
            &note,
            "n1",
            "a.png",
            &reserved,
            Some(3),
            None,
            &mut &b"png"[..],
        )
        .unwrap();
        assert_eq!(added.name, "a-1.png");

        std::fs::write(attachments_dir_for(&note, "n1").join("a.png"), b"other").unwrap();
        let changes = collect(&state, &engine).await;
        let ids: Vec<&str> = changes.ops.iter().map(|op| op.entity_id.as_str()).collect();
        assert_eq!(ids, ["n1/a-1.png"]);
        assert_eq!(changes.errors.len(), 1, "{:?}", changes.errors);
        assert!(
            changes.errors[0].starts_with("a.png: "),
            "{:?}",
            changes.errors
        );
    }
}
