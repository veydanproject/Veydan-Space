// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use super::files::*;
use super::index::rebuild_manifest;
use super::models::*;
use chrono::Utc;
use uuid::Uuid;
use veydan_core::Core;
use veydan_core::{AppError, CmdResult};

// ── Folder commands ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn note_folder_list(core: tauri::State<'_, Core>) -> CmdResult<Vec<NoteFolder>> {
    sqlx::query_as::<_, NoteFolder>(
        "SELECT id, name, parent_id, color, created_at, updated_at FROM note_folders ORDER BY name",
    )
    .fetch_all(&core.db)
    .await
    .map_err(AppError::db)
}

#[tauri::command]
pub async fn note_folder_create(
    name: String,
    parent_id: Option<String>,
    color: Option<String>,
    core: tauri::State<'_, Core>,
) -> CmdResult<NoteFolder> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::io("Folder name cannot be empty"));
    }
    if let Some(ref pid) = parent_id {
        let parent: Option<(Option<String>,)> =
            sqlx::query_as("SELECT parent_id FROM note_folders WHERE id = ?")
                .bind(pid)
                .fetch_optional(&core.db)
                .await
                .map_err(AppError::db)?;
        match parent {
            None => return Err(AppError::io("Parent folder not found")),
            Some((Some(_),)) => {
                return Err(AppError::io("Folder nesting is limited to 2 levels"));
            }
            Some((None,)) => {}
        }
    }

    let id = Uuid::new_v4().to_string();
    let color = color.unwrap_or_else(|| "#6366f1".to_string());
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO note_folders (id, name, parent_id, color, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&name)
    .bind(&parent_id)
    .bind(&color)
    .bind(&now)
    .bind(&now)
    .execute(&core.db)
    .await
    .map_err(AppError::db)?;

    sqlx::query_as::<_, NoteFolder>(
        "SELECT id, name, parent_id, color, created_at, updated_at FROM note_folders WHERE id = ?",
    )
    .bind(&id)
    .fetch_one(&core.db)
    .await
    .map_err(AppError::db)
}

#[tauri::command]
pub async fn note_folder_update(
    id: String,
    name: Option<String>,
    color: Option<String>,
    core: tauri::State<'_, Core>,
) -> CmdResult<NoteFolder> {
    let now = Utc::now().to_rfc3339();
    let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    sqlx::query(
        "UPDATE note_folders SET
            name       = COALESCE(?, name),
            color      = COALESCE(?, color),
            updated_at = ?
         WHERE id = ?",
    )
    .bind(&name)
    .bind(&color)
    .bind(&now)
    .bind(&id)
    .execute(&core.db)
    .await
    .map_err(AppError::db)?;

    sqlx::query_as::<_, NoteFolder>(
        "SELECT id, name, parent_id, color, created_at, updated_at FROM note_folders WHERE id = ?",
    )
    .bind(&id)
    .fetch_one(&core.db)
    .await
    .map_err(AppError::db)
}

#[tauri::command]
pub async fn note_folder_delete(id: String, core: tauri::State<'_, Core>) -> CmdResult<()> {
    // Collect all descendant folder IDs recursively using CTE
    let descendants: Vec<(String,)> = sqlx::query_as(
        "WITH RECURSIVE sub(id) AS (
            SELECT id FROM note_folders WHERE id = ?
            UNION ALL
            SELECT f.id FROM note_folders f JOIN sub ON f.parent_id = sub.id
         )
         SELECT id FROM sub",
    )
    .bind(&id)
    .fetch_all(&core.db)
    .await
    .map_err(AppError::db)?;

    for (fid,) in &descendants {
        sqlx::query("DELETE FROM note_folder_links WHERE folder_id = ?")
            .bind(fid)
            .execute(&core.db)
            .await
            .map_err(AppError::db)?;
    }

    // Delete all descendant folders (children first via CTE ordering handled by FK cascade
    // but SQLite may not enforce it, so delete in reverse BFS order via delete by ids)
    for (fid,) in descendants.iter().rev() {
        sqlx::query("DELETE FROM note_folders WHERE id = ?")
            .bind(fid)
            .execute(&core.db)
            .await
            .map_err(AppError::db)?;
    }

    Ok(())
}

/// Persist a new bindings list to the DB and the note file frontmatter.
pub(crate) async fn persist_bindings(
    note_id: &str,
    bindings: &[String],
    core: &Core,
) -> Result<(), AppError> {
    let new_json = serde_json::to_string(bindings).map_err(AppError::other)?;
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE notes SET bindings = ?, updated_at = ? WHERE id = ?")
        .bind(&new_json)
        .bind(&now)
        .bind(note_id)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;

    let Some(row) = sqlx::query_as::<_, NoteRow>("SELECT * FROM notes WHERE id = ?")
        .bind(note_id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?
    else {
        return Ok(());
    };

    let file_path = resolve_note_abs_path(&core.app_data_dir, &row.file_path);
    if file_path.exists() {
        let (_, tags, body) = read_note_file(&file_path)?;
        write_note_file(&file_path, &row, &tags, &body)?;
    }
    rebuild_manifest(&core.db, &core.app_data_dir).await
}

async fn load_bindings(note_id: &str, core: &Core) -> Result<Option<Vec<String>>, AppError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT bindings FROM notes WHERE id = ?")
        .bind(note_id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?;
    Ok(row.map(|(json,)| serde_json::from_str(&json).unwrap_or_default()))
}

#[tauri::command]
pub async fn note_add_binding(
    note_id: String,
    binding: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<()> {
    let Some(mut bindings) = load_bindings(&note_id, &core).await? else {
        return Ok(());
    };
    if !bindings.contains(&binding) {
        bindings.push(binding);
        persist_bindings(&note_id, &bindings, &core).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn note_remove_binding(
    note_id: String,
    binding: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<()> {
    let Some(mut bindings) = load_bindings(&note_id, &core).await? else {
        return Ok(());
    };
    bindings.retain(|b| b != &binding);
    persist_bindings(&note_id, &bindings, &core).await
}

#[tauri::command]
pub async fn note_add_folder(
    note_id: String,
    folder_id: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<()> {
    sqlx::query("INSERT OR IGNORE INTO note_folder_links (note_id, folder_id) VALUES (?, ?)")
        .bind(&note_id)
        .bind(&folder_id)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;
    Ok(())
}

#[tauri::command]
pub async fn note_remove_folder(
    note_id: String,
    folder_id: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<()> {
    sqlx::query("DELETE FROM note_folder_links WHERE note_id = ? AND folder_id = ?")
        .bind(&note_id)
        .bind(&folder_id)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;
    Ok(())
}

/// Single-folder assignment (mobile UX): replaces every folder link; `None` clears them.
#[tauri::command]
pub async fn note_set_folder(
    note_id: String,
    folder_id: Option<String>,
    core: tauri::State<'_, Core>,
) -> CmdResult<()> {
    sqlx::query("DELETE FROM note_folder_links WHERE note_id = ?")
        .bind(&note_id)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;
    if let Some(fid) = folder_id {
        sqlx::query("INSERT OR IGNORE INTO note_folder_links (note_id, folder_id) VALUES (?, ?)")
            .bind(&note_id)
            .bind(&fid)
            .execute(&core.db)
            .await
            .map_err(AppError::db)?;
    }
    Ok(())
}
