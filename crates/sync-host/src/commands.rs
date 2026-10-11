// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The `sync_*` commands. The shell declares them in its own module and
//! routes them (spec, section 22); the conflicts of notes and the files of
//! profiles have commands of their own in the modules that own them.

use crate::config::{self, build_storage, load_binding, load_config};
use crate::cycle::{run_cycle, status, trigger_cycle, SyncStatus};
use crate::debug::{ensure_debug_loaded, trace_skip, SyncDebugState, DEBUG_SETTING};
use crate::{join, Host, SyncConfig, SyncManager, EVENT_STATUS};
use chrono::Utc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use veydan_core::{settings, AppError, CmdResult, Core};
use veydan_sync::{Engine, Probe, Vmk};

/// Minimum gap between cycles started by `sync_trigger` (app resume, screen open).
const MIN_TRIGGER_GAP_SEC: i64 = 15;

/// How long `sync_change_storage` waits for a running cycle to end.
const CHANGE_STORAGE_WAIT: Duration = Duration::from_secs(20);

#[tauri::command]
pub async fn sync_get_config(core: tauri::State<'_, Core>) -> CmdResult<SyncConfig> {
    Ok(load_config(&core.db).await)
}

#[tauri::command]
pub async fn sync_set_config(cfg: SyncConfig, core: tauri::State<'_, Core>) -> CmdResult<()> {
    config::save_config(&core.db, &cfg).await
}

/// What the configured storage holds: "empty" | "vault" | "foreign".
#[tauri::command]
pub async fn sync_probe(core: tauri::State<'_, Core>) -> CmdResult<String> {
    config::refresh_cloud_hosts(&core.db).await?;
    let cfg = load_config(&core.db).await;
    let storage = build_storage(&cfg)?;
    Ok(match Engine::probe(storage.as_ref())
        .await
        .map_err(AppError::other)?
    {
        Probe::Empty => "empty",
        Probe::Vault(_) => "vault",
        Probe::Foreign => "foreign",
    }
    .to_string())
}

/// New vault in an empty storage. Refuses if anything is already there.
#[tauri::command]
pub async fn sync_create_vault(
    passphrase: String,
    app: AppHandle,
    core: tauri::State<'_, Core>,
    sync: tauri::State<'_, SyncManager>,
) -> CmdResult<SyncStatus> {
    let db = &core.db;
    // On the backend `cloud` the passphrase comes from the identity.
    let passphrase = config::vault_passphrase(db, passphrase).await?;
    if passphrase.len() < 8 {
        return Err(AppError::other("passphrase must be at least 8 characters"));
    }
    let cfg = load_config(db).await;
    let device = config::device_id(db).await?;
    let storage = build_storage(&cfg)?;
    let (engine, vmk) = Engine::create(storage, &passphrase, &device)
        .await
        .map_err(AppError::other)?;
    join::bind(db, sync.registry(), engine.vault_id(), &vmk).await?;
    start_first_cycle(&app);
    status(&core, &sync).await
}

/// Join the vault found in the configured storage.
#[tauri::command]
pub async fn sync_join_vault(
    passphrase: String,
    app: AppHandle,
    core: tauri::State<'_, Core>,
    sync: tauri::State<'_, SyncManager>,
) -> CmdResult<SyncStatus> {
    let host = Host::App(&app);
    let passphrase = config::vault_passphrase(&core.db, passphrase).await?;
    if join::join(&host, &core, sync.registry(), &passphrase).await? {
        join::key_row_changed(&host).await;
    }
    start_first_cycle(&app);
    status(&core, &sync).await
}

/// A joined device is pointed at another storage (`join::change_storage`):
/// "same" | "foreign" | "empty" | "vault". The slot of the cycle is held: a
/// cycle of the old vault that ended after the device joined the new one
/// would write its positions under the new binding, and what it had pushed
/// to the old storage would never reach the new one.
#[tauri::command]
pub async fn sync_change_storage(
    cfg: SyncConfig,
    app: AppHandle,
    core: tauri::State<'_, Core>,
    sync: tauri::State<'_, SyncManager>,
) -> CmdResult<String> {
    let _slot = sync.pause(CHANGE_STORAGE_WAIT).await.ok_or_else(|| {
        AppError::other("a sync is running: wait until it ends, then change the storage")
    })?;
    let found = join::change_storage(&core.db, sync.registry(), cfg).await?;
    let _ = app.emit(EVENT_STATUS, ());
    Ok(found.to_string())
}

/// After a binding was saved: tell the UI, start a cycle without blocking the caller.
fn start_first_cycle(app: &AppHandle) {
    let _ = app.emit(EVENT_STATUS, ());
    trigger_cycle(app, "join");
}

/// Forget the vault on this device. Nothing in the storage is touched.
#[tauri::command]
pub async fn sync_leave(
    app: AppHandle,
    core: tauri::State<'_, Core>,
    sync: tauri::State<'_, SyncManager>,
) -> CmdResult<SyncStatus> {
    join::leave(&core.db, sync.registry()).await?;
    let _ = app.emit(EVENT_STATUS, ());
    status(&core, &sync).await
}

#[tauri::command]
pub async fn sync_change_passphrase(
    old: String,
    new: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<()> {
    if new.len() < 8 {
        return Err(AppError::other("passphrase must be at least 8 characters"));
    }
    let db = &core.db;
    let cfg = load_config(db).await;
    if cfg.backend == config::CLOUD_BACKEND {
        // Every device derives it from the identity: a new one would lock them out.
        return Err(AppError::other(
            "the passphrase of Veydan Cloud comes from the identity and does not change",
        ));
    }
    let binding = load_binding(db)
        .await
        .ok_or_else(|| AppError::other("no vault joined"))?;
    let vmk = Vmk::from_base64(&binding.vmk_b64).map_err(AppError::other)?;
    let device = config::device_id(db).await?;
    let engine = Engine::with_key(build_storage(&cfg)?, &vmk, &binding.vault_id, &device);
    engine
        .change_passphrase(&vmk, &old, &new)
        .await
        .map_err(AppError::other)
}

#[tauri::command]
pub async fn sync_status(
    core: tauri::State<'_, Core>,
    sync: tauri::State<'_, SyncManager>,
) -> CmdResult<SyncStatus> {
    status(&core, &sync).await
}

/// Developer log: current toggle and the lines kept in memory.
#[tauri::command]
pub async fn sync_debug_get(
    core: tauri::State<'_, Core>,
    sync: tauri::State<'_, SyncManager>,
) -> CmdResult<SyncDebugState> {
    ensure_debug_loaded(&core, &sync).await;
    Ok(sync.debug.state())
}

/// Turn the developer sync log on or off. The choice survives a restart; the lines do not.
#[tauri::command]
pub async fn sync_debug_set(
    enabled: bool,
    core: tauri::State<'_, Core>,
    sync: tauri::State<'_, SyncManager>,
) -> CmdResult<()> {
    settings::set(&core.db, DEBUG_SETTING, if enabled { "1" } else { "0" }).await?;
    sync.debug.set_enabled(enabled);
    Ok(())
}

/// Drop the in-memory lines. The toggle stays as it is.
#[tauri::command]
pub async fn sync_debug_clear(sync: tauri::State<'_, SyncManager>) -> CmdResult<()> {
    sync.debug.clear();
    Ok(())
}

/// One full cycle right now. Errors are also recorded as `last_error`.
#[tauri::command]
pub async fn sync_run_now(
    app: AppHandle,
    core: tauri::State<'_, Core>,
    sync: tauri::State<'_, SyncManager>,
) -> CmdResult<SyncStatus> {
    run_cycle(&app, "manual").await?;
    status(&core, &sync).await
}

/// Start a cycle in the background (app resumed, screen opened). No-op when one
/// is running, sync is off or the last cycle started less than `MIN_TRIGGER_GAP_SEC` ago.
#[tauri::command]
pub async fn sync_trigger(app: AppHandle, core: tauri::State<'_, Core>) -> CmdResult<()> {
    if load_binding(&core.db).await.is_none() {
        return Ok(());
    }
    let due = match settings::get(&core.db, "sync_last_started").await {
        None => true,
        Some(last) => chrono::DateTime::parse_from_rfc3339(&last)
            .map(|t| (Utc::now() - t.with_timezone(&Utc)).num_seconds() >= MIN_TRIGGER_GAP_SEC)
            .unwrap_or(true),
    };
    if due {
        trigger_cycle(&app, "trigger");
    } else {
        trace_skip(
            &app,
            "trigger",
            "trigger",
            "last cycle started less than 15s ago",
        );
    }
    Ok(())
}
