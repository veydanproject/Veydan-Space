// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Saved filters ("smart views"): a named, colored `NoteFilter` stored as JSON.

use super::models::{NoteFilter, NoteSmartView, SmartViewInput, SmartViewRow};
use chrono::Utc;
use uuid::Uuid;
use veydan_core::Core;
use veydan_core::{AppError, CmdResult};

const DEFAULT_COLOR: &str = "#8b7bff";

fn to_view(row: SmartViewRow) -> NoteSmartView {
    NoteSmartView {
        id: row.id,
        name: row.name,
        color: row.color,
        conditions: serde_json::from_str::<NoteFilter>(&row.conditions).unwrap_or_default(),
        sort_order: row.sort_order,
    }
}

async fn fetch_view(id: &str, core: &Core) -> Result<NoteSmartView, AppError> {
    sqlx::query_as::<_, SmartViewRow>("SELECT * FROM note_smart_views WHERE id = ?")
        .bind(id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?
        .map(to_view)
        .ok_or_else(|| AppError::not_found(format!("Smart view {id}")))
}

#[tauri::command]
pub async fn note_smart_view_list(core: tauri::State<'_, Core>) -> CmdResult<Vec<NoteSmartView>> {
    let rows = sqlx::query_as::<_, SmartViewRow>(
        "SELECT * FROM note_smart_views ORDER BY sort_order ASC, created_at ASC",
    )
    .fetch_all(&core.db)
    .await
    .map_err(AppError::db)?;
    Ok(rows.into_iter().map(to_view).collect())
}

#[tauri::command]
pub async fn note_smart_view_create(
    input: SmartViewInput,
    core: tauri::State<'_, Core>,
) -> CmdResult<NoteSmartView> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::other("Smart view name is required"));
    }
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let conditions = serde_json::to_string(&input.conditions).map_err(AppError::other)?;
    let next_order: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(sort_order), -1) + 1 FROM note_smart_views")
            .fetch_one(&core.db)
            .await
            .map_err(AppError::db)?;
    sqlx::query(
        "INSERT INTO note_smart_views (id, name, color, conditions, sort_order, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&name)
    .bind(input.color.as_deref().unwrap_or(DEFAULT_COLOR))
    .bind(&conditions)
    .bind(next_order)
    .bind(&now)
    .bind(&now)
    .execute(&core.db)
    .await
    .map_err(AppError::db)?;
    fetch_view(&id, &core).await
}

#[tauri::command]
pub async fn note_smart_view_update(
    id: String,
    input: SmartViewInput,
    core: tauri::State<'_, Core>,
) -> CmdResult<NoteSmartView> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::other("Smart view name is required"));
    }
    let conditions = serde_json::to_string(&input.conditions).map_err(AppError::other)?;
    sqlx::query(
        "UPDATE note_smart_views SET name = ?, color = COALESCE(?, color), conditions = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&name)
    .bind(input.color.as_deref())
    .bind(&conditions)
    .bind(Utc::now().to_rfc3339())
    .bind(&id)
    .execute(&core.db)
    .await
    .map_err(AppError::db)?;
    fetch_view(&id, &core).await
}

#[tauri::command]
pub async fn note_smart_view_delete(id: String, core: tauri::State<'_, Core>) -> CmdResult<()> {
    sqlx::query("DELETE FROM note_smart_views WHERE id = ?")
        .bind(&id)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;
    Ok(())
}
