// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Sync positions of the files of notes and of their attachments, and the
//! conflicts notes add to the status of sync. The tables are notes'.

use sqlx::{Pool, Sqlite};
use std::collections::HashMap;
use veydan_core::{AppError, BoxFuture, CmdResult, Core};
use veydan_sync::{Hlc, LargeFileRef};
use veydan_sync_host::ConflictInfo;

/// Where the local note file stands relative to the vault.
#[derive(Debug, Clone, Default)]
pub struct NoteSyncState {
    pub note_id: String,
    /// Blob of the version this device last pushed or applied.
    pub head_blob: String,
    pub head_parents: Vec<String>,
    pub head_hlc: Option<Hlc>,
    /// SHA-256 of the local file when `head_blob` was pushed/applied.
    pub synced_hash: String,
    pub deleted: bool,
    /// A remote version overlaps local edits; the note is not pushed until resolved.
    pub conflict: bool,
    pub conflict_ancestor_id: String,
    pub conflict_local_id: String,
    pub conflict_remote_id: String,
    /// Remote blob to record as second parent once the conflict is resolved.
    pub conflict_remote_blob: String,
}

type NoteStateRow = (
    String,
    String,
    String,
    String,
    String,
    i64,
    i64,
    String,
    String,
    String,
    String,
);

const SELECT_NOTE_STATE: &str = "SELECT note_id, head_blob, head_parents, head_hlc, synced_hash, deleted, conflict,
    conflict_ancestor_id, conflict_local_id, conflict_remote_id, conflict_remote_blob FROM sync_note_state";

fn row_to_state(r: NoteStateRow) -> NoteSyncState {
    NoteSyncState {
        note_id: r.0,
        head_blob: r.1,
        head_parents: serde_json::from_str(&r.2).unwrap_or_default(),
        head_hlc: Hlc::decode(&r.3),
        synced_hash: r.4,
        deleted: r.5 != 0,
        conflict: r.6 != 0,
        conflict_ancestor_id: r.7,
        conflict_local_id: r.8,
        conflict_remote_id: r.9,
        conflict_remote_blob: r.10,
    }
}

pub async fn load_note_states(db: &Pool<Sqlite>) -> CmdResult<HashMap<String, NoteSyncState>> {
    let rows: Vec<NoteStateRow> = sqlx::query_as(SELECT_NOTE_STATE)
        .fetch_all(db)
        .await
        .map_err(AppError::db)?;
    Ok(rows
        .into_iter()
        .map(row_to_state)
        .map(|s| (s.note_id.clone(), s))
        .collect())
}

pub async fn load_note_state(db: &Pool<Sqlite>, note_id: &str) -> CmdResult<Option<NoteSyncState>> {
    let row: Option<NoteStateRow> = sqlx::query_as(
        "SELECT note_id, head_blob, head_parents, head_hlc, synced_hash, deleted, conflict,
         conflict_ancestor_id, conflict_local_id, conflict_remote_id, conflict_remote_blob
         FROM sync_note_state WHERE note_id = ?",
    )
    .bind(note_id)
    .fetch_optional(db)
    .await
    .map_err(AppError::db)?;
    Ok(row.map(row_to_state))
}

pub async fn save_note_state(db: &Pool<Sqlite>, s: &NoteSyncState) -> CmdResult<()> {
    sqlx::query(
        "INSERT INTO sync_note_state (note_id, head_blob, head_parents, head_hlc, synced_hash, deleted, conflict,
           conflict_ancestor_id, conflict_local_id, conflict_remote_id, conflict_remote_blob)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(note_id) DO UPDATE SET
           head_blob = excluded.head_blob, head_parents = excluded.head_parents, head_hlc = excluded.head_hlc,
           synced_hash = excluded.synced_hash, deleted = excluded.deleted, conflict = excluded.conflict,
           conflict_ancestor_id = excluded.conflict_ancestor_id, conflict_local_id = excluded.conflict_local_id,
           conflict_remote_id = excluded.conflict_remote_id, conflict_remote_blob = excluded.conflict_remote_blob",
    )
    .bind(&s.note_id)
    .bind(&s.head_blob)
    .bind(serde_json::to_string(&s.head_parents).unwrap_or_else(|_| "[]".into()))
    .bind(s.head_hlc.as_ref().map(Hlc::encode).unwrap_or_default())
    .bind(&s.synced_hash)
    .bind(s.deleted as i64)
    .bind(s.conflict as i64)
    .bind(&s.conflict_ancestor_id)
    .bind(&s.conflict_local_id)
    .bind(&s.conflict_remote_id)
    .bind(&s.conflict_remote_blob)
    .execute(db)
    .await
    .map_err(AppError::db)?;
    Ok(())
}

// ── Attachments ──────────────────────────────────────────────────────────────

/// Where a local attachment file stands relative to the vault (LWW, no merge).
#[derive(Debug, Clone, Default)]
pub struct AttachmentSyncState {
    pub note_id: String,
    pub name: String,
    pub head_blob: String,
    pub head_hlc: Option<Hlc>,
    pub synced_hash: String,
    pub deleted: bool,
    /// Chunked file accepted from the vault but kept remote until the user asks for it.
    pub deferred: Option<LargeFileRef>,
}

type AttachmentStateRow = (String, String, String, String, String, i64, String);

fn row_to_attachment_state(r: AttachmentStateRow) -> AttachmentSyncState {
    AttachmentSyncState {
        note_id: r.0,
        name: r.1,
        head_blob: r.2,
        head_hlc: Hlc::decode(&r.3),
        synced_hash: r.4,
        deleted: r.5 != 0,
        deferred: serde_json::from_str(&r.6).ok(),
    }
}

pub async fn load_attachment_states(
    db: &Pool<Sqlite>,
) -> CmdResult<HashMap<(String, String), AttachmentSyncState>> {
    let rows: Vec<AttachmentStateRow> =
        sqlx::query_as("SELECT note_id, name, head_blob, head_hlc, synced_hash, deleted, deferred_ref FROM sync_attachment_state")
            .fetch_all(db)
            .await
            .map_err(AppError::db)?;
    Ok(rows
        .into_iter()
        .map(row_to_attachment_state)
        .map(|s| ((s.note_id.clone(), s.name.clone()), s))
        .collect())
}

pub async fn load_attachment_state(
    db: &Pool<Sqlite>,
    note_id: &str,
    name: &str,
) -> CmdResult<Option<AttachmentSyncState>> {
    let row: Option<AttachmentStateRow> = sqlx::query_as(
        "SELECT note_id, name, head_blob, head_hlc, synced_hash, deleted, deferred_ref FROM sync_attachment_state
         WHERE note_id = ? AND name = ?",
    )
    .bind(note_id)
    .bind(name)
    .fetch_optional(db)
    .await
    .map_err(AppError::db)?;
    Ok(row.map(row_to_attachment_state))
}

/// Deferred (not yet downloaded) attachments of one note.
pub async fn load_deferred_attachments(
    db: &Pool<Sqlite>,
    note_id: &str,
) -> CmdResult<Vec<AttachmentSyncState>> {
    let rows: Vec<AttachmentStateRow> = sqlx::query_as(
        "SELECT note_id, name, head_blob, head_hlc, synced_hash, deleted, deferred_ref FROM sync_attachment_state
         WHERE note_id = ? AND deleted = 0 AND deferred_ref != ''",
    )
    .bind(note_id)
    .fetch_all(db)
    .await
    .map_err(AppError::db)?;
    Ok(rows.into_iter().map(row_to_attachment_state).collect())
}

pub async fn save_attachment_state(db: &Pool<Sqlite>, s: &AttachmentSyncState) -> CmdResult<()> {
    let deferred = s
        .deferred
        .as_ref()
        .map(|r| serde_json::to_string(r).unwrap_or_default())
        .unwrap_or_default();
    sqlx::query(
        "INSERT INTO sync_attachment_state (note_id, name, head_blob, head_hlc, synced_hash, deleted, deferred_ref)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(note_id, name) DO UPDATE SET
           head_blob = excluded.head_blob, head_hlc = excluded.head_hlc,
           synced_hash = excluded.synced_hash, deleted = excluded.deleted,
           deferred_ref = excluded.deferred_ref",
    )
    .bind(&s.note_id)
    .bind(&s.name)
    .bind(&s.head_blob)
    .bind(s.head_hlc.as_ref().map(Hlc::encode).unwrap_or_default())
    .bind(&s.synced_hash)
    .bind(s.deleted as i64)
    .bind(deferred)
    .execute(db)
    .await
    .map_err(AppError::db)?;
    Ok(())
}

/// Notes with an unresolved sync conflict, for `sync_status`: (id, title).
pub(crate) fn conflicts(core: &Core) -> BoxFuture<'_, CmdResult<Vec<ConflictInfo>>> {
    Box::pin(async move {
        let rows = sqlx::query_as::<_, (String, String)>(
            "SELECT s.note_id, COALESCE(n.title, s.note_id) FROM sync_note_state s
             LEFT JOIN notes n ON n.id = s.note_id
             WHERE s.conflict = 1 AND s.deleted = 0",
        )
        .fetch_all(&core.db)
        .await
        .map_err(AppError::db)?;
        Ok(conflict_infos(rows))
    })
}

fn conflict_infos(rows: Vec<(String, String)>) -> Vec<ConflictInfo> {
    rows.into_iter()
        .map(|(note_id, title)| ConflictInfo { note_id, title })
        .collect()
}
