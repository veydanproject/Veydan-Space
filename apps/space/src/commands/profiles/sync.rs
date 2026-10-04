// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The two answers to profile files that diverged between devices. Logic of
//! the browser under the `sync_` prefix (spec, section 22).

use crate::browser::BrowserState;
use crate::error::CmdResult;
use crate::sync::profile_files;
use tauri::{AppHandle, Emitter};
use veydan_core::Core;
use veydan_sync_host::{open_engine, status, trigger_cycle, SyncManager, SyncStatus, EVENT_STATUS};

/// Conflict choice for profile files: replace local files with the remote snapshot.
#[tauri::command]
pub async fn sync_profile_files_take_remote(
    profile_id: String,
    app: AppHandle,
    core: tauri::State<'_, Core>,
    sync: tauri::State<'_, SyncManager>,
    browser: tauri::State<'_, BrowserState>,
) -> CmdResult<SyncStatus> {
    let engine = open_engine(&core).await?;
    profile_files::take_remote(&engine, &core, &sync, &browser, &app, &profile_id).await?;
    let _ = app.emit(EVENT_STATUS, ());
    status(&core, &sync).await
}

/// Conflict choice for profile files: keep local files and publish them.
#[tauri::command]
pub async fn sync_profile_files_push_mine(
    profile_id: String,
    app: AppHandle,
    core: tauri::State<'_, Core>,
    sync: tauri::State<'_, SyncManager>,
) -> CmdResult<SyncStatus> {
    profile_files::push_mine(&core, &profile_id).await?;
    trigger_cycle(&app, "profile-push");
    status(&core, &sync).await
}
