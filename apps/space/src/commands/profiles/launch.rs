// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use crate::browser::launch::{self as browser_launch, Running};
use crate::browser::{profile_launcher, BrowserState, LaunchingPayload};
use crate::error::{AppError, CmdResult};
use crate::models::Profile;
use std::sync::Arc;
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
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        launch_once(&browser.running, &id, || async {
            browser.launches.set(&app_handle, &id, Some(false));
            let result = launch_reserved(&id, force, &core, &browser, &app_handle).await;
            browser.launches.set(&app_handle, &id, None);
            result
        })
        .await
    })
    .await
}

/// Runs `launch` with the profile reserved: refused at once while the
/// profile runs or another launch of it is under way, so two clicks start
/// one browser and one launch's failure never undoes another's status or
/// lease. The reservation is given back whatever `launch` returns.
async fn launch_once<F, Fut>(running: &Arc<Running>, id: &str, launch: F) -> CmdResult<u32>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = CmdResult<u32>>,
{
    let Some(slot) = running.reserve(id).await else {
        return Err(AppError::other(if running.is_running(id).await {
            "Profile is already running"
        } else {
            "Profile is already being launched"
        }));
    };
    let result = launch().await;
    slot.release().await;
    result
}

async fn launch_reserved(
    id: &str,
    force: Option<bool>,
    core: &Core,
    browser: &BrowserState,
    app_handle: &tauri::AppHandle,
) -> CmdResult<u32> {
    // The files of the profile are not collected or applied by sync from
    // here until the process is registered (or the launch failed); a cycle
    // that holds them is waited for: a collect gives them up at once, an
    // apply is let finish. The UI shows the launch waiting for sync.
    let syncing = || browser.launches.set(app_handle, id, Some(true));
    let files = browser.files.lock_for_launch(id, syncing).await;
    crate::sync::before_profile_launch(app_handle, id, force.unwrap_or(false), syncing).await?;
    browser.launches.set(app_handle, id, Some(false));

    let profile = sqlx::query_as::<_, Profile>("SELECT * FROM profiles WHERE id = ?")
        .bind(id)
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
    .bind(id)
    .execute(&core.db)
    .await
    .map_err(AppError::db)?;

    let result = profile_launcher::launch_profile(
        &profile,
        proxy.as_ref(),
        core,
        browser,
        app_handle.clone(),
    )
    .await;
    drop(files);

    // Rollback status if launch failed. The reservation is this launch's, so
    // a process under the id is one this launch registered: it stays running.
    if result.is_err() && !browser.running.is_running(id).await {
        let _ = sqlx::query(
            "UPDATE profiles SET status = 'stopped', updated_at = datetime('now') WHERE id = ?",
        )
        .bind(id)
        .execute(&core.db)
        .await;
        crate::sync::on_profile_stopped(app_handle, id).await;
    }

    // Always emit so frontend reflects actual running state (empty on failure)
    emit_running_ids(app_handle, browser).await;

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

/// The profiles being launched and those of them waiting for sync; the
/// changes come as `profiles://launching-changed`.
#[tauri::command]
pub async fn profiles_launching(
    browser: tauri::State<'_, BrowserState>,
) -> CmdResult<LaunchingPayload> {
    Ok(browser.launches.payload())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn a_second_launch_during_the_first_is_refused_and_a_failure_frees_the_profile() {
        let running = Arc::new(Running::default());
        let (started_tx, started_rx) = oneshot::channel();
        let (go_tx, go_rx) = oneshot::channel::<()>();
        let first = tokio::spawn({
            let running = Arc::clone(&running);
            async move {
                launch_once(&running, "p", || async move {
                    started_tx.send(()).ok();
                    go_rx.await.ok();
                    Err(AppError::other("spawn failed"))
                })
                .await
            }
        });
        started_rx.await.unwrap();

        let second = launch_once(&running, "p", || async { Ok(2) }).await;
        let err = second.expect_err("one launch at a time").to_string();
        assert!(err.contains("already being launched"), "got {err}");
        assert_eq!(
            launch_once(&running, "q", || async { Ok(3) })
                .await
                .unwrap(),
            3
        );

        go_tx.send(()).unwrap();
        assert!(first.await.unwrap().is_err());
        assert_eq!(
            launch_once(&running, "p", || async { Ok(4) })
                .await
                .unwrap(),
            4
        );
    }

    #[tokio::test]
    async fn a_running_profile_is_not_launched_again() {
        let running = Arc::new(Running::default());
        running.processes.lock().await.insert(
            "p".into(),
            crate::browser::launch::BrowserProcess::for_test(1),
        );
        let err = launch_once(&running, "p", || async { Ok(2) })
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("already running"), "got {err}");
    }
}
