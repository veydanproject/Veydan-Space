// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The commands of notes about sync: conflicts of note files, where a note
//! stands against the vault, a large attachment in flight. Logic of notes
//! under the `sync_` prefix (spec, section 22).

use super::{attachments, notes, state};
use crate::NotesState;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use veydan_core::Core;
use veydan_core::{AppError, CmdResult};
use veydan_sync_host::{run_cycle, status, trace_skip, SyncManager, SyncStatus, EVENT_STATUS};

#[derive(Debug, Serialize)]
pub struct NoteSyncInfo {
    /// The vault knows this note.
    pub tracked: bool,
    /// Local edits not yet pushed.
    pub pending: bool,
}

/// Structured merge blocks for the conflict UI plus a token for `sync_conflict_resolve`.
#[tauri::command]
pub async fn sync_conflict_get(
    note_id: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<notes::ConflictView> {
    notes::conflict_merge(&core, &note_id).await
}

/// Store the resolved text and push it right away. `token` must match `sync_conflict_get`.
#[tauri::command]
pub async fn sync_conflict_resolve(
    note_id: String,
    token: String,
    content: String,
    app: AppHandle,
    core: tauri::State<'_, Core>,
    sync: tauri::State<'_, SyncManager>,
    notes: tauri::State<'_, NotesState>,
) -> CmdResult<SyncStatus> {
    notes::resolve_conflict(&core, &notes, &note_id, &token, content).await?;
    let _ = app.emit(EVENT_STATUS, ());
    if !sync.is_running() {
        run_cycle(&app, "conflict").await?;
    } else {
        trace_skip(&app, "conflict", "trigger", "already running");
    }
    status(&core, &sync).await
}

/// Sync position of one note: `tracked` when the vault knows it, `pending` when
/// the local file differs from the last pushed/applied version.
#[tauri::command]
pub async fn note_sync_info(id: String, core: tauri::State<'_, Core>) -> CmdResult<NoteSyncInfo> {
    let Some(st) = state::load_note_state(&core.db, &id).await? else {
        return Ok(NoteSyncInfo {
            tracked: false,
            pending: true,
        });
    };
    let file_path: Option<String> =
        sqlx::query_scalar("SELECT file_path FROM notes WHERE id = ? AND deleted = 0")
            .bind(&id)
            .fetch_optional(&core.db)
            .await
            .map_err(AppError::db)?;
    let hash = file_path
        .and_then(|p| std::fs::read(crate::resolve_note_abs_path(&core.app_data_dir, &p)).ok())
        .map(|raw| veydan_sync::sha256_hex(&raw))
        .unwrap_or_default();
    Ok(NoteSyncInfo {
        tracked: true,
        pending: hash != st.synced_hash,
    })
}

/// Stop a large attachment transfer in flight. The file stays dirty and is
/// retried on the next cycle unless it was removed.
#[tauri::command]
pub async fn sync_attachment_cancel(
    note_id: String,
    name: String,
    sync: tauri::State<'_, SyncManager>,
) -> CmdResult<bool> {
    Ok(sync.cancel_transfer(&attachments::transfer_key(&note_id, &name)))
}
