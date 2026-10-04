// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use crate::browser::launch as browser_launch;
use crate::browser::BrowserState;
use crate::error::{AppError, CmdResult};
use crate::models::{
    CreateWorkspaceColumnRequest, CreateWorkspaceRequest, Profile, UpdateWorkspaceColumnRequest,
    UpdateWorkspaceRequest, Workspace, WorkspaceColumn, WorkspaceStats,
};
use crate::modules::directory;
use chrono::Utc;
use uuid::Uuid;
use veydan_core::Core;

#[tauri::command]
pub async fn workspace_list(core: tauri::State<'_, Core>) -> CmdResult<Vec<Workspace>> {
    sqlx::query_as::<_, Workspace>(
        "SELECT * FROM workspaces ORDER BY is_default DESC, created_at ASC",
    )
    .fetch_all(&core.db)
    .await
    .map_err(AppError::db)
}

#[tauri::command]
pub async fn workspace_get(
    id: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<Option<Workspace>> {
    sqlx::query_as::<_, Workspace>("SELECT * FROM workspaces WHERE id = ?")
        .bind(&id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)
}

#[tauri::command]
pub async fn workspace_create(
    req: CreateWorkspaceRequest,
    core: tauri::State<'_, Core>,
) -> CmdResult<Workspace> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    sqlx::query(
        "INSERT INTO workspaces (id, name, description, color, icon, is_default, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, 0, ?, ?)",
    )
    .bind(&id)
    .bind(&req.name)
    .bind(&req.description)
    .bind(req.color.as_deref().unwrap_or("#6366f1"))
    .bind(req.icon.as_deref().unwrap_or("folder"))
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(AppError::db)?;
    directory::relabel(&core.directory, &mut tx, &directory::WORKSPACE, &id).await?;
    tx.commit().await.map_err(AppError::db)?;

    workspace_get(id, core)
        .await
        .and_then(|w| w.ok_or_else(|| AppError::not_found("Workspace not found after create")))
}

#[tauri::command]
pub async fn workspace_update(
    id: String,
    req: UpdateWorkspaceRequest,
    core: tauri::State<'_, Core>,
) -> CmdResult<Workspace> {
    let now = Utc::now().to_rfc3339();

    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    sqlx::query(
        "UPDATE workspaces SET
            name = COALESCE(?, name),
            description = COALESCE(?, description),
            color = COALESCE(?, color),
            icon = COALESCE(?, icon),
            notes = CASE WHEN ? IS NULL THEN notes WHEN ? = '' THEN NULL ELSE ? END,
            updated_at = ?
        WHERE id = ?",
    )
    .bind(&req.name)
    .bind(&req.description)
    .bind(&req.color)
    .bind(&req.icon)
    .bind(&req.notes)
    .bind(&req.notes)
    .bind(&req.notes)
    .bind(&now)
    .bind(&id)
    .execute(&mut *tx)
    .await
    .map_err(AppError::db)?;
    directory::relabel(&core.directory, &mut tx, &directory::WORKSPACE, &id).await?;
    tx.commit().await.map_err(AppError::db)?;

    workspace_get(id, core)
        .await
        .and_then(|w| w.ok_or_else(|| AppError::not_found("Workspace not found after update")))
}

#[tauri::command]
pub async fn workspace_delete(
    id: String,
    mode: String,
    core: tauri::State<'_, Core>,
    browser: tauri::State<'_, BrowserState>,
    app_handle: tauri::AppHandle,
) -> CmdResult<()> {
    if id == "default" {
        return Err(AppError::other("Cannot delete the Default workspace"));
    }

    if mode == "delete_all" {
        let profiles =
            sqlx::query_as::<_, Profile>("SELECT * FROM profiles WHERE workspace_id = ?")
                .bind(&id)
                .fetch_all(&core.db)
                .await
                .map_err(AppError::db)?;

        // Stop running browser processes before deleting files
        for profile in &profiles {
            if browser.running.is_running(&profile.id).await {
                browser_launch::stop(&profile.id, &browser.running)
                    .await
                    .ok();
            }
        }
        let running_ids = browser.running.running_ids().await;
        browser_launch::emit_running_changed(&app_handle, running_ids);

        for profile in &profiles {
            if std::path::Path::new(&profile.profile_path).exists() {
                let _ = std::fs::remove_dir_all(&profile.profile_path);
            }
        }
    }

    // The rows, and the labels of what they name, in one transaction.
    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    let ws_tag = format!("workspace:{}", id);
    let profiles: Vec<String> =
        sqlx::query_scalar("SELECT id FROM profiles WHERE workspace_id = ?")
            .bind(&id)
            .fetch_all(&mut *tx)
            .await
            .map_err(AppError::db)?;
    let mut proxies: Vec<String> = Vec::new();
    if mode == "delete_all" {
        sqlx::query("DELETE FROM profiles WHERE workspace_id = ?")
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(AppError::db)?;

        // Delete proxies tagged with this workspace
        proxies = sqlx::query_scalar(
            "SELECT id FROM proxies WHERE EXISTS (
                SELECT 1 FROM json_each(tags) WHERE value = ?
            )",
        )
        .bind(&ws_tag)
        .fetch_all(&mut *tx)
        .await
        .map_err(AppError::db)?;
        sqlx::query(
            "DELETE FROM proxies WHERE EXISTS (
                SELECT 1 FROM json_each(tags) WHERE value = ?
            )",
        )
        .bind(&ws_tag)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;
    } else {
        sqlx::query("UPDATE profiles SET workspace_id = 'default' WHERE workspace_id = ?")
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(AppError::db)?;

        // Replace workspace tag on proxies: workspace:{id} → workspace:default
        sqlx::query(
            "UPDATE proxies SET tags = (
                SELECT json_group_array(CASE WHEN value = ? THEN 'workspace:default' ELSE value END)
                FROM json_each(tags)
            ) WHERE EXISTS (SELECT 1 FROM json_each(tags) WHERE value = ?)",
        )
        .bind(&ws_tag)
        .bind(&ws_tag)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;
    }

    sqlx::query("DELETE FROM workspaces WHERE id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;
    directory::relabel(&core.directory, &mut tx, &directory::WORKSPACE, &id).await?;
    directory::relabel_all(&core.directory, &mut tx, &directory::PROFILE, &profiles).await?;
    directory::relabel_all(&core.directory, &mut tx, &directory::PROXY, &proxies).await?;
    tx.commit().await.map_err(AppError::db)?;

    Ok(())
}

#[tauri::command]
pub async fn workspace_stats(
    id: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<WorkspaceStats> {
    let (profile_count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM profiles WHERE workspace_id = ?")
            .bind(&id)
            .fetch_one(&core.db)
            .await
            .map_err(AppError::db)?;

    let ws_tag = format!("workspace:{}", id);
    let (proxy_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM proxies WHERE EXISTS (SELECT 1 FROM json_each(tags) WHERE value = ?)",
    )
    .bind(&ws_tag)
    .fetch_one(&core.db)
    .await
    .map_err(AppError::db)?;

    let (active_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM profiles WHERE workspace_id = ? AND status = 'running'",
    )
    .bind(&id)
    .fetch_one(&core.db)
    .await
    .map_err(AppError::db)?;

    Ok(WorkspaceStats {
        id,
        profile_count,
        proxy_count,
        active_count,
    })
}

#[tauri::command]
pub async fn profiles_list_by_workspace(
    workspace_id: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<Vec<Profile>> {
    sqlx::query_as::<_, Profile>(
        "SELECT * FROM profiles WHERE workspace_id = ? ORDER BY kanban_status, kanban_order, created_at",
    )
    .bind(&workspace_id)
    .fetch_all(&core.db)
    .await
    .map_err(AppError::db)
}

// ── Workspace Columns ─────────────────────────────────────────────────────────

#[tauri::command]
pub async fn workspace_column_list(
    workspace_id: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<Vec<WorkspaceColumn>> {
    sqlx::query_as::<_, WorkspaceColumn>(
        "SELECT * FROM workspace_columns WHERE workspace_id = ? ORDER BY position ASC, created_at ASC",
    )
    .bind(&workspace_id)
    .fetch_all(&core.db)
    .await
    .map_err(AppError::db)
}

#[tauri::command]
pub async fn workspace_column_create(
    workspace_id: String,
    req: CreateWorkspaceColumnRequest,
    core: tauri::State<'_, Core>,
) -> CmdResult<WorkspaceColumn> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    let (max_pos,): (Option<i64>,) =
        sqlx::query_as("SELECT MAX(position) FROM workspace_columns WHERE workspace_id = ?")
            .bind(&workspace_id)
            .fetch_one(&core.db)
            .await
            .map_err(AppError::db)?;
    let position = max_pos.unwrap_or(-1) + 1;

    sqlx::query(
        "INSERT INTO workspace_columns (id, workspace_id, name, tag_name, color, position, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&workspace_id)
    .bind(&req.name)
    .bind(&req.tag_name)
    .bind(req.color.as_deref().unwrap_or("#6366f1"))
    .bind(position)
    .bind(&now)
    .execute(&core.db)
    .await
    .map_err(AppError::db)?;

    sqlx::query_as::<_, WorkspaceColumn>("SELECT * FROM workspace_columns WHERE id = ?")
        .bind(&id)
        .fetch_one(&core.db)
        .await
        .map_err(AppError::db)
}

#[tauri::command]
pub async fn workspace_column_update(
    id: String,
    req: UpdateWorkspaceColumnRequest,
    core: tauri::State<'_, Core>,
) -> CmdResult<WorkspaceColumn> {
    sqlx::query(
        "UPDATE workspace_columns SET
            name = COALESCE(?, name),
            color = COALESCE(?, color),
            position = COALESCE(?, position)
        WHERE id = ?",
    )
    .bind(&req.name)
    .bind(&req.color)
    .bind(req.position)
    .bind(&id)
    .execute(&core.db)
    .await
    .map_err(AppError::db)?;

    sqlx::query_as::<_, WorkspaceColumn>("SELECT * FROM workspace_columns WHERE id = ?")
        .bind(&id)
        .fetch_one(&core.db)
        .await
        .map_err(AppError::db)
}

#[tauri::command]
pub async fn workspace_column_delete(id: String, core: tauri::State<'_, Core>) -> CmdResult<()> {
    let col = sqlx::query_as::<_, WorkspaceColumn>("SELECT * FROM workspace_columns WHERE id = ?")
        .bind(&id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Column not found"))?;

    // Remove this tag from all profiles in the workspace
    sqlx::query(
        "UPDATE profiles SET tags = (
            SELECT COALESCE(json_group_array(value), '[]') FROM json_each(tags) WHERE value != ?
        ) WHERE workspace_id = ?",
    )
    .bind(&col.tag_name)
    .bind(&col.workspace_id)
    .execute(&core.db)
    .await
    .map_err(AppError::db)?;

    sqlx::query("DELETE FROM workspace_columns WHERE id = ?")
        .bind(&id)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;
    Ok(())
}

// ── Profile Tags ──────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn profile_set_tags(
    id: String,
    tags: Vec<String>,
    core: tauri::State<'_, Core>,
) -> CmdResult<()> {
    let now = Utc::now().to_rfc3339();
    let tags_json = serde_json::to_string(&tags).map_err(AppError::other)?;
    sqlx::query("UPDATE profiles SET tags = ?, updated_at = ? WHERE id = ?")
        .bind(&tags_json)
        .bind(&now)
        .bind(&id)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;
    Ok(())
}

/// Move profile to a kanban column: replaces all column tags with the new column's tag_name.
/// Column tags are the tag_names of all workspace_columns for this profile's workspace.
#[tauri::command]
pub async fn profile_move_to_kanban_column(
    profile_id: String,
    target_tag: String,
    kanban_order: i64,
    core: tauri::State<'_, Core>,
) -> CmdResult<()> {
    let now = Utc::now().to_rfc3339();

    let profile =
        sqlx::query_as::<_, crate::models::Profile>("SELECT * FROM profiles WHERE id = ?")
            .bind(&profile_id)
            .fetch_optional(&core.db)
            .await
            .map_err(AppError::db)?
            .ok_or_else(|| AppError::not_found("Profile not found"))?;

    let workspace_id = profile.workspace_id.as_deref().unwrap_or("default");

    // Get all column tag_names for this workspace
    let column_tags: Vec<(String,)> =
        sqlx::query_as("SELECT tag_name FROM workspace_columns WHERE workspace_id = ?")
            .bind(workspace_id)
            .fetch_all(&core.db)
            .await
            .map_err(AppError::db)?;
    let column_tag_set: std::collections::HashSet<String> =
        column_tags.into_iter().map(|(t,)| t).collect();

    // Parse current tags, remove all column tags, add new one
    let current_tags: Vec<String> = serde_json::from_str(&profile.tags).unwrap_or_default();
    let mut new_tags: Vec<String> = current_tags
        .into_iter()
        .filter(|t| !column_tag_set.contains(t))
        .collect();
    if !target_tag.is_empty() {
        new_tags.push(target_tag);
    }

    let tags_json = serde_json::to_string(&new_tags).map_err(AppError::other)?;
    sqlx::query("UPDATE profiles SET tags = ?, kanban_order = ?, updated_at = ? WHERE id = ?")
        .bind(&tags_json)
        .bind(kanban_order)
        .bind(&now)
        .bind(&profile_id)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;

    Ok(())
}
