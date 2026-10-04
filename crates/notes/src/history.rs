// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use super::files::*;
use super::index::*;
use super::merge::{merge3, MergeResult};
use super::models::*;
use super::tags::*;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;
use veydan_core::Core;
use veydan_core::{AppError, CmdResult};

pub(crate) fn compress_content(content: &str) -> Result<Vec<u8>, AppError> {
    zstd::encode_all(content.as_bytes(), 3).map_err(AppError::other)
}

pub(crate) fn decompress_content(blob: &[u8]) -> Result<String, AppError> {
    let bytes = zstd::decode_all(blob).map_err(AppError::other)?;
    String::from_utf8(bytes).map_err(AppError::other)
}

// ── Note History ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Clone)]
pub struct NoteHistoryEntry {
    pub id: String,
    pub note_id: String,
    pub parent_id: Option<String>,
    pub revision: i64,
    pub version_type: String,
    pub title: String,
    pub content: Option<String>,
    pub content_hash: String,
    pub author: Option<String>,
    pub device: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct DiffLine {
    pub kind: String, // "context" | "added" | "removed"
    pub content: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct DiffResult {
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Deserialize, Default)]
pub struct HistoryFilter {
    pub version_type: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
}

/// Fetch the last history entry metadata (id, created_at, revision) for a note.
pub(crate) async fn history_last_meta(
    note_id: &str,
    db: &sqlx::Pool<sqlx::Sqlite>,
) -> Result<Option<(String, String, i64)>, AppError> {
    sqlx::query_as::<_, (String, String, i64)>(
        "SELECT id, created_at, revision FROM note_history WHERE note_id = ? ORDER BY revision DESC LIMIT 1",
    )
    .bind(note_id)
    .fetch_optional(db)
    .await
    .map_err(AppError::db)
}

/// Create a snapshot of the given content. Inserts into note_history.
pub(crate) async fn history_snapshot(
    note_id: &str,
    title: &str,
    content: &str,
    version_type: &str,
    parent_id: Option<String>,
    db: &sqlx::Pool<sqlx::Sqlite>,
) -> Result<String, AppError> {
    history_snapshot_by(note_id, title, content, version_type, parent_id, None, db).await
}

/// Snapshot attributed to a device (sync versions coming from another machine).
pub(crate) async fn history_snapshot_by(
    note_id: &str,
    title: &str,
    content: &str,
    version_type: &str,
    parent_id: Option<String>,
    device: Option<&str>,
    db: &sqlx::Pool<sqlx::Sqlite>,
) -> Result<String, AppError> {
    let now = Utc::now().to_rfc3339();
    let id = Uuid::new_v4().to_string();
    let content_hash = compute_hash(content);
    let compressed = compress_content(content)?;

    let parent_id = match parent_id {
        Some(pid) => {
            let exists: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM note_history WHERE id = ?")
                .bind(&pid)
                .fetch_optional(db)
                .await
                .map_err(AppError::db)?;
            exists.map(|_| pid)
        }
        None => None,
    };

    let (next_revision,): (i64,) =
        sqlx::query_as("SELECT COALESCE(MAX(revision), 0) + 1 FROM note_history WHERE note_id = ?")
            .bind(note_id)
            .fetch_one(db)
            .await
            .map_err(AppError::db)?;

    sqlx::query(
        "INSERT INTO note_history
         (id, note_id, parent_id, revision, version_type, title, content, content_hash, device, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(note_id)
    .bind(&parent_id)
    .bind(next_revision)
    .bind(version_type)
    .bind(title)
    .bind(&compressed)
    .bind(&content_hash)
    .bind(device)
    .bind(&now)
    .execute(db)
    .await
    .map_err(AppError::db)?;

    history_cleanup(note_id, db).await?;

    Ok(id)
}

/// Keep last 100 entries; delete entries older than 90 days beyond the first 50.
pub(crate) async fn history_cleanup(
    note_id: &str,
    db: &sqlx::Pool<sqlx::Sqlite>,
) -> Result<(), AppError> {
    let cutoff = (Utc::now() - chrono::Duration::days(90)).to_rfc3339();
    // Nested SELECT so SQLite allows UPDATE/DELETE against the same table.
    sqlx::query(
        "UPDATE note_history SET parent_id = NULL WHERE parent_id IN (
            SELECT id FROM (
                SELECT id FROM note_history
                WHERE note_id = ?
                  AND created_at < ?
                  AND id NOT IN (
                      SELECT id FROM note_history WHERE note_id = ? ORDER BY revision DESC LIMIT 100
                  )
            )
        )",
    )
    .bind(note_id)
    .bind(&cutoff)
    .bind(note_id)
    .execute(db)
    .await
    .map_err(AppError::db)?;
    sqlx::query(
        "DELETE FROM note_history WHERE id IN (
            SELECT id FROM (
                SELECT id FROM note_history
                WHERE note_id = ?
                  AND created_at < ?
                  AND id NOT IN (
                      SELECT id FROM note_history WHERE note_id = ? ORDER BY revision DESC LIMIT 100
                  )
            )
        )",
    )
    .bind(note_id)
    .bind(&cutoff)
    .bind(note_id)
    .execute(db)
    .await
    .map_err(AppError::db)?;
    Ok(())
}

/// Called from note_update: apply snapshot policy and conditionally snapshot old content.
pub(crate) async fn maybe_snapshot(
    note_id: &str,
    title: &str,
    old_content: &str,
    version_type: &str,
    db: &sqlx::Pool<sqlx::Sqlite>,
) -> Result<(), AppError> {
    let last = history_last_meta(note_id, db).await?;

    let should_snapshot: bool;
    let parent_id: Option<String>;

    match last {
        None => {
            // First snapshot ever for this note
            should_snapshot = true;
            parent_id = None;
        }
        Some((last_id, last_created_at, _)) => {
            let last_dt = chrono::DateTime::parse_from_rfc3339(&last_created_at)
                .ok()
                .map(|dt| dt.with_timezone(&Utc));
            let seconds_since = last_dt
                .map(|ldt| (Utc::now() - ldt).num_seconds())
                .unwrap_or(i64::MAX);

            // Skip if last snapshot was made less than 60 seconds ago
            should_snapshot = seconds_since >= 60;
            parent_id = Some(last_id);
        }
    }

    if should_snapshot {
        history_snapshot(note_id, title, old_content, version_type, parent_id, db).await?;
    }

    Ok(())
}

/// Decompress and return content for a history entry by its id.
pub(crate) async fn history_content_by_id(
    history_id: &str,
    db: &sqlx::Pool<sqlx::Sqlite>,
) -> Result<String, AppError> {
    let row: Option<(Vec<u8>,)> = sqlx::query_as("SELECT content FROM note_history WHERE id = ?")
        .bind(history_id)
        .fetch_optional(db)
        .await
        .map_err(AppError::db)?;

    let (blob,) = row.ok_or_else(|| AppError::not_found(format!("History {history_id}")))?;
    decompress_content(&blob)
}

pub(crate) fn compute_diff_lines(old: &str, new: &str) -> Vec<DiffLine> {
    let patch = diffy::create_patch(old, new);
    let mut lines = Vec::new();
    for hunk in patch.hunks() {
        for line in hunk.lines() {
            let (kind, raw): (&str, String) = match line {
                diffy::Line::Context(s) => ("context", s.to_string()),
                diffy::Line::Delete(s) => ("removed", s.to_string()),
                diffy::Line::Insert(s) => ("added", s.to_string()),
            };
            lines.push(DiffLine {
                kind: kind.to_string(),
                content: raw.trim_end_matches('\n').to_string(),
            });
        }
    }
    lines
}

// ── History commands ──────────────────────────────────────────────────────────

/// id, note_id, parent_id, revision, version_type, title, content_hash,
/// author, device, created_at of a row of the history.
type HistoryRow = (
    String,
    String,
    Option<String>,
    i64,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
);

/// The same with the content, compressed, before its hash.
type HistoryContentRow = (
    String,
    String,
    Option<String>,
    i64,
    String,
    String,
    Vec<u8>,
    String,
    Option<String>,
    Option<String>,
    String,
);

#[tauri::command]
pub async fn note_history_list(
    note_id: String,
    filter: Option<HistoryFilter>,
    core: tauri::State<'_, Core>,
) -> CmdResult<Vec<NoteHistoryEntry>> {
    let filter = filter.unwrap_or_default();

    let rows: Vec<HistoryRow> =
        sqlx::query_as(
            "SELECT id, note_id, parent_id, revision, version_type, title, content_hash, author, device, created_at
             FROM note_history
             WHERE note_id = ?
               AND (? IS NULL OR version_type = ?)
               AND (? IS NULL OR created_at >= ?)
               AND (? IS NULL OR created_at <= ?)
             ORDER BY revision DESC",
        )
        .bind(&note_id)
        .bind(&filter.version_type).bind(&filter.version_type)
        .bind(&filter.date_from).bind(&filter.date_from)
        .bind(&filter.date_to).bind(&filter.date_to)
        .fetch_all(&core.db)
        .await
        .map_err(AppError::db)?;

    let entries = rows
        .into_iter()
        .map(
            |(
                id,
                note_id,
                parent_id,
                revision,
                version_type,
                title,
                content_hash,
                author,
                device,
                created_at,
            )| {
                NoteHistoryEntry {
                    id,
                    note_id,
                    parent_id,
                    revision,
                    version_type,
                    title,
                    content: None,
                    content_hash,
                    author,
                    device,
                    created_at,
                }
            },
        )
        .collect();

    Ok(entries)
}

#[tauri::command]
pub async fn note_history_get(
    history_id: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<NoteHistoryEntry> {
    let row: Option<HistoryContentRow> =
        sqlx::query_as(
            "SELECT id, note_id, parent_id, revision, version_type, title, content, content_hash, author, device, created_at
             FROM note_history WHERE id = ?",
        )
        .bind(&history_id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?;

    let (
        id,
        note_id,
        parent_id,
        revision,
        version_type,
        title,
        blob,
        content_hash,
        author,
        device,
        created_at,
    ) = row.ok_or_else(|| AppError::not_found(format!("History {history_id}")))?;

    let content = decompress_content(&blob)?;

    Ok(NoteHistoryEntry {
        id,
        note_id,
        parent_id,
        revision,
        version_type,
        title,
        content: Some(content),
        content_hash,
        author,
        device,
        created_at,
    })
}

#[tauri::command]
pub async fn note_history_diff(
    note_id: String,
    from_id: String,
    to_id: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<DiffResult> {
    async fn resolve_content(
        entry_id: &str,
        note_id: &str,
        core: &tauri::State<'_, Core>,
    ) -> Result<String, AppError> {
        if entry_id == "current" {
            let row = sqlx::query_as::<_, NoteRow>("SELECT * FROM notes WHERE id = ?")
                .bind(note_id)
                .fetch_optional(&core.db)
                .await
                .map_err(AppError::db)?
                .ok_or_else(|| AppError::not_found(format!("Note {note_id}")))?;
            let file_path = resolve_note_abs_path(&core.app_data_dir, &row.file_path);
            let (_, _, body) = read_note_file(&file_path).unwrap_or_default();
            Ok(body)
        } else {
            history_content_by_id(entry_id, &core.db).await
        }
    }

    let old_content = resolve_content(&from_id, &note_id, &core).await?;
    let new_content = resolve_content(&to_id, &note_id, &core).await?;

    Ok(DiffResult {
        lines: compute_diff_lines(&old_content, &new_content),
    })
}

#[tauri::command]
pub async fn note_history_restore(
    note_id: String,
    history_id: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<Note> {
    // Get history content to restore
    let history_content = history_content_by_id(&history_id, &core.db).await?;

    // Get history title
    let (hist_title,): (String,) = sqlx::query_as("SELECT title FROM note_history WHERE id = ?")
        .bind(&history_id)
        .fetch_one(&core.db)
        .await
        .map_err(AppError::db)?;

    // Get current note
    let mut row = sqlx::query_as::<_, NoteRow>("SELECT * FROM notes WHERE id = ?")
        .bind(&note_id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found(format!("Note {note_id}")))?;

    let file_path = resolve_note_abs_path(&core.app_data_dir, &row.file_path);
    let (_, _, current_content) = if file_path.exists() {
        read_note_file(&file_path)?
    } else {
        (HashMap::new(), vec![], String::new())
    };

    // Snapshot current state before restoring (type = "restore", parent = last history id)
    let parent_id = history_last_meta(&note_id, &core.db)
        .await?
        .map(|(pid, _, _)| pid);
    let _ = history_snapshot(
        &note_id,
        &row.title,
        &current_content,
        "restore",
        parent_id,
        &core.db,
    )
    .await;

    // Apply restored content
    let now = Utc::now().to_rfc3339();
    row.title = hist_title.clone();
    row.updated_at = now.clone();

    let content_hash = compute_hash(&history_content);
    let preview = make_preview(&history_content);

    let tag_names = fetch_note_tags(&note_id, &core.db)
        .await?
        .into_iter()
        .map(|t| t.name)
        .collect::<Vec<_>>();

    write_note_file(&file_path, &row, &tag_names, &history_content)?;

    let fts_rowid = fts_upsert(
        &note_id,
        &hist_title,
        &history_content,
        &tag_names,
        row.fts_rowid,
        &core.db,
    )
    .await?;

    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    sqlx::query(
        "UPDATE notes SET title=?, updated_at=?, content_hash=?, preview=?, fts_rowid=? WHERE id=?",
    )
    .bind(&hist_title)
    .bind(&now)
    .bind(&content_hash)
    .bind(&preview)
    .bind(fts_rowid)
    .bind(&note_id)
    .execute(&mut *tx)
    .await
    .map_err(AppError::db)?;
    crate::directory::relabel(&core, &mut tx, &note_id).await?;
    tx.commit().await.map_err(AppError::db)?;

    rebuild_manifest(&core.db, &core.app_data_dir).await?;

    let tags = fetch_note_tags(&note_id, &core.db).await?;
    let folder_ids = fetch_note_folder_ids(&note_id, &core.db).await?;
    let bindings: Vec<String> = serde_json::from_str(&row.bindings).unwrap_or_default();

    Ok(Note {
        id: row.id,
        title: hist_title,
        base_dir: note_base_dir(&core.app_data_dir, &row.file_path),
        file_path: row.file_path,
        format: row.format,
        bindings,
        tags,
        folder_ids,
        pinned: row.pinned != 0,
        archived: row.archived != 0,
        deleted: row.deleted != 0,
        doc_status: row.doc_status,
        created_at: row.created_at,
        updated_at: now,
        content_hash: Some(content_hash),
        content: Some(history_content),
        has_draft: false,
    })
}

#[tauri::command]
pub async fn note_history_merge(
    note_id: String,
    history_id: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<MergeResult> {
    // Get current note content
    let row = sqlx::query_as::<_, NoteRow>("SELECT * FROM notes WHERE id = ?")
        .bind(&note_id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found(format!("Note {note_id}")))?;

    let file_path = resolve_note_abs_path(&core.app_data_dir, &row.file_path);
    let (_, _, current_content) = if file_path.exists() {
        read_note_file(&file_path)?
    } else {
        (HashMap::new(), vec![], String::new())
    };

    // Get history content and its parent (ancestor for 3-way merge)
    let (history_content, parent_id) = {
        let r: Option<(Vec<u8>, Option<String>)> =
            sqlx::query_as("SELECT content, parent_id FROM note_history WHERE id = ?")
                .bind(&history_id)
                .fetch_optional(&core.db)
                .await
                .map_err(AppError::db)?;
        let (blob, pid) = r.ok_or_else(|| AppError::not_found(format!("History {history_id}")))?;
        (decompress_content(&blob)?, pid)
    };

    // Ancestor = parent of history entry (or empty string if none)
    let ancestor = if let Some(pid) = parent_id {
        history_content_by_id(&pid, &core.db)
            .await
            .unwrap_or_default()
    } else {
        String::new()
    };

    // 3-way merge: original=ancestor, ours=current, theirs=history
    Ok(merge3(&ancestor, &current_content, &history_content))
}
