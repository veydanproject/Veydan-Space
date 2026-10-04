// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What `app_clear_data` empties, module by module. The Default workspace
//! row, the configurations of sync and backup, the Camoufox runtime, the
//! preferences of the UI and the lock with its key stay.

#[cfg(desktop)]
use crate::browser::BrowserState;
use crate::error::{AppError, CmdResult};
use chrono::Utc;
use std::path::Path;
use veydan_core::Core;

/// Stop running browsers before deleting profile dirs.
#[cfg(desktop)]
pub async fn stop_browsers(browser: &BrowserState) {
    for id in browser.running.running_ids().await {
        let _ = crate::browser::launch::stop(&id, &browser.running).await;
    }
}

/// Profiles with their folders, proxies, columns and extra workspaces; the
/// Default workspace as a fresh install has it. On the desktop
/// `stop_browsers` comes first.
pub async fn clear_browser(core: &Core) -> CmdResult<()> {
    // Profiles then proxies (proxy_id FK is soft)
    let profile_paths: Vec<(String,)> = sqlx::query_as("SELECT profile_path FROM profiles")
        .fetch_all(&core.db)
        .await
        .unwrap_or_default();
    let _ = sqlx::query("DELETE FROM profiles").execute(&core.db).await;
    let _ = sqlx::query("DELETE FROM proxies").execute(&core.db).await;

    let _ = sqlx::query("DELETE FROM workspace_columns")
        .execute(&core.db)
        .await;
    let _ = sqlx::query("DELETE FROM workspaces WHERE id != 'default'")
        .execute(&core.db)
        .await;

    // Reset Default workspace
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE workspaces SET name = 'Default', description = NULL, color = '#6366f1',
         icon = 'folder', notes = NULL, updated_at = ? WHERE id = 'default'",
    )
    .bind(&now)
    .execute(&core.db)
    .await
    .map_err(AppError::db)?;

    for (path,) in profile_paths {
        let p = Path::new(&path);
        if p.exists() {
            let _ = std::fs::remove_dir_all(p);
        }
    }
    let profiles_root = core.app_data_dir.join("profiles");
    if profiles_root.exists() {
        let _ = std::fs::remove_dir_all(&profiles_root);
        let _ = std::fs::create_dir_all(&profiles_root);
    }
    Ok(())
}

/// Connections with their links, and keys. They point at profiles,
/// workspaces and proxies, so they go before those.
#[cfg(desktop)]
pub async fn clear_ssh(core: &Core) -> CmdResult<()> {
    let _ = sqlx::query("DELETE FROM ssh_connection_profiles")
        .execute(&core.db)
        .await;
    let _ = sqlx::query("DELETE FROM ssh_connection_workspaces")
        .execute(&core.db)
        .await;
    let _ = sqlx::query("DELETE FROM ssh_connections")
        .execute(&core.db)
        .await;
    let _ = sqlx::query("DELETE FROM ssh_keys").execute(&core.db).await;
    Ok(())
}

/// The rules of the web clipper.
#[cfg(desktop)]
pub async fn clear_capture(core: &Core) -> CmdResult<()> {
    let _ = veydan_core::settings::delete(&core.db, crate::capture::rules::KEY).await;
    Ok(())
}
