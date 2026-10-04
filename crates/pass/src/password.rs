// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The history of the password generator.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use veydan_core::{AppError, CmdResult, Core};

#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct PwEntry {
    pub id: String,
    pub password: String,
    pub created_at: String,
}

#[tauri::command]
pub async fn pwgen_history_list(core: tauri::State<'_, Core>) -> CmdResult<Vec<PwEntry>> {
    sqlx::query_as::<_, PwEntry>("SELECT * FROM password_history ORDER BY created_at DESC")
        .fetch_all(&core.db)
        .await
        .map_err(AppError::db)
}

#[tauri::command]
pub async fn pwgen_history_add(
    password: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<PwEntry> {
    let id = Uuid::new_v4().to_string();
    let created_at = Utc::now().to_rfc3339();

    sqlx::query("INSERT INTO password_history (id, password, created_at) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(&password)
        .bind(&created_at)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;

    Ok(PwEntry {
        id,
        password,
        created_at,
    })
}

#[tauri::command]
pub async fn pwgen_history_clear(core: tauri::State<'_, Core>) -> CmdResult<()> {
    sqlx::query("DELETE FROM password_history")
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;
    Ok(())
}

#[tauri::command]
pub async fn pwgen_history_trim(limit: i64, core: tauri::State<'_, Core>) -> CmdResult<()> {
    sqlx::query(
        "DELETE FROM password_history WHERE id NOT IN (
            SELECT id FROM password_history ORDER BY created_at DESC LIMIT ?
        )",
    )
    .bind(limit)
    .execute(&core.db)
    .await
    .map_err(AppError::db)?;
    Ok(())
}
