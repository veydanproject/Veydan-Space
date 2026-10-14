// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The history of the password generator.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use veydan_core::{AppError, CmdResult, Core};
use veydan_lock::Lock;

/// The generated passwords are secrets: none of the commands of the history
/// runs while the app is locked, as the code of a TOTP entry.
async fn require_open(lock: &Lock) -> CmdResult<()> {
    if lock.is_locked().await {
        return Err(AppError::VaultLocked);
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct PwEntry {
    pub id: String,
    pub password: String,
    pub created_at: String,
}

#[tauri::command]
pub async fn pwgen_history_list(
    core: tauri::State<'_, Core>,
    lock: tauri::State<'_, Lock>,
) -> CmdResult<Vec<PwEntry>> {
    require_open(&lock).await?;
    sqlx::query_as::<_, PwEntry>("SELECT * FROM password_history ORDER BY created_at DESC")
        .fetch_all(&core.db)
        .await
        .map_err(AppError::db)
}

#[tauri::command]
pub async fn pwgen_history_add(
    password: String,
    core: tauri::State<'_, Core>,
    lock: tauri::State<'_, Lock>,
) -> CmdResult<PwEntry> {
    require_open(&lock).await?;
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
pub async fn pwgen_history_clear(
    core: tauri::State<'_, Core>,
    lock: tauri::State<'_, Lock>,
) -> CmdResult<()> {
    require_open(&lock).await?;
    sqlx::query("DELETE FROM password_history")
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;
    Ok(())
}

#[tauri::command]
pub async fn pwgen_history_trim(
    limit: i64,
    core: tauri::State<'_, Core>,
    lock: tauri::State<'_, Lock>,
) -> CmdResult<()> {
    require_open(&lock).await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::Manager;

    /// As the code of a TOTP entry (directory.rs): the history is out of
    /// reach while the app is locked, and back once it is open.
    #[tokio::test]
    async fn no_history_while_the_app_is_locked() {
        let (app, _dir) = crate::testing::app().await;
        let lock = app.state::<Lock>();
        lock.set(Some("4821".into()), None, Some("pin".into()), None)
            .await
            .unwrap();
        pwgen_history_add("s3cret".into(), app.state(), app.state())
            .await
            .unwrap();
        lock.lock();
        let locked = |r: CmdResult<()>| matches!(r, Err(AppError::VaultLocked));
        assert!(matches!(
            pwgen_history_list(app.state(), app.state()).await,
            Err(AppError::VaultLocked)
        ));
        assert!(locked(
            pwgen_history_add("x".into(), app.state(), app.state())
                .await
                .map(|_| ())
        ));
        assert!(locked(
            pwgen_history_trim(0, app.state(), app.state()).await
        ));
        assert!(locked(pwgen_history_clear(app.state(), app.state()).await));

        assert!(lock.unlock("4821").await.unwrap());
        let list = pwgen_history_list(app.state(), app.state()).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].password, "s3cret");
    }
}
