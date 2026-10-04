// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use crate::browser::launch as browser_launch;
use crate::browser::{profile_launcher, BrowserState};
use crate::error::{AppError, CmdResult};
use crate::models::Profile;
use veydan_core::Core;

pub async fn emit_running_ids(app_handle: &tauri::AppHandle, browser: &BrowserState) {
    let ids = browser.running.running_ids().await;
    browser_launch::emit_running_changed(app_handle, ids);
}

/// `force` launches even when another device holds the sync lease.
#[tauri::command]
pub async fn profile_launch(
    id: String,
    force: Option<bool>,
    core: tauri::State<'_, Core>,
    browser: tauri::State<'_, BrowserState>,
    app_handle: tauri::AppHandle,
) -> CmdResult<u32> {
    if browser.running.is_running(&id).await {
        return Err(AppError::other("Profile is already running"));
    }
    crate::sync::before_profile_launch(&app_handle, &id, force.unwrap_or(false)).await?;

    let profile = sqlx::query_as::<_, Profile>("SELECT * FROM profiles WHERE id = ?")
        .bind(&id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Profile not found"))?;

    // Fails closed: a profile that names a proxy must not launch with a direct
    // connection just because the row vanished (see `proxies::resolve_required`).
    // Resolved before the status flips to 'running' so a failure needs no rollback.
    let proxy =
        crate::commands::proxies::resolve_required(&core.db, profile.proxy_id.as_deref()).await?;

    sqlx::query(
        "UPDATE profiles SET status = 'running', last_launch_at = datetime('now'), updated_at = datetime('now') WHERE id = ?",
    )
    .bind(&id)
    .execute(&core.db)
    .await
    .map_err(AppError::db)?;

    let result = profile_launcher::launch_profile(
        &profile,
        proxy.as_ref(),
        &core,
        &browser,
        app_handle.clone(),
    )
    .await;

    if result.is_err() {
        // Rollback status if launch failed
        let _ = sqlx::query(
            "UPDATE profiles SET status = 'stopped', updated_at = datetime('now') WHERE id = ?",
        )
        .bind(&id)
        .execute(&core.db)
        .await;
        crate::sync::on_profile_stopped(&app_handle, &id).await;
    }

    // Always emit so frontend reflects actual running state (empty on failure)
    emit_running_ids(&app_handle, &browser).await;

    Ok(result.map_err(AppError::browser)?.pid)
}

#[tauri::command]
pub async fn profile_stop(
    id: String,
    core: tauri::State<'_, Core>,
    browser: tauri::State<'_, BrowserState>,
    app_handle: tauri::AppHandle,
) -> CmdResult<()> {
    browser_launch::stop(&id, &browser.running)
        .await
        .map_err(AppError::browser)?;
    // Write stopped status immediately so workspace_stats reflects reality
    sqlx::query(
        "UPDATE profiles SET status = 'stopped', updated_at = datetime('now') WHERE id = ?",
    )
    .bind(&id)
    .execute(&core.db)
    .await
    .map_err(AppError::db)?;
    emit_running_ids(&app_handle, &browser).await;
    Ok(())
}

#[tauri::command]
pub async fn profile_is_running(
    id: String,
    browser: tauri::State<'_, BrowserState>,
) -> CmdResult<bool> {
    Ok(browser.running.is_running(&id).await)
}

#[tauri::command]
pub async fn profiles_running_ids(
    browser: tauri::State<'_, BrowserState>,
) -> CmdResult<Vec<String>> {
    Ok(browser.running.running_ids().await)
}
