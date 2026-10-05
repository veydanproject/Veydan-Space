// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The commands of the running tor: its settings, the list of instances,
//! starting and stopping one by hand, a new identity, the log, and the kinds
//! of built-in bridges the installed bundle has.

use veydan_core::{AppError, CmdResult, Core};

use crate::manager::{ExitSet, InstanceInfo, TorManager};
use crate::settings::{TorSettings, SETTINGS_KEY};
use crate::source;
use crate::torrc::PtConfig;

#[tauri::command]
pub fn tor_get_settings(manager: tauri::State<'_, TorManager>) -> TorSettings {
    manager.settings()
}

/// Checks, stores and applies the settings; returns them as stored (tidied
/// by [`TorSettings::validate`]).
#[tauri::command]
pub async fn tor_set_settings(
    core: tauri::State<'_, Core>,
    manager: tauri::State<'_, TorManager>,
    settings: TorSettings,
) -> CmdResult<TorSettings> {
    let settings = settings.validate().map_err(AppError::Other)?;
    let json = serde_json::to_string(&settings)?;
    core.settings.set(SETTINGS_KEY, &json).await?;
    manager.apply_settings(settings.clone()).await;
    Ok(settings)
}

#[tauri::command]
pub fn tor_instances(manager: tauri::State<'_, TorManager>) -> Vec<InstanceInfo> {
    manager.instances()
}

/// `exit`: the exit countries, `"de, nl"`; empty for any exit.
#[tauri::command]
pub async fn tor_start(
    manager: tauri::State<'_, TorManager>,
    exit: String,
) -> CmdResult<InstanceInfo> {
    let exit = ExitSet::parse(&exit)?;
    Ok(manager.start(&exit).await?)
}

#[tauri::command]
pub fn tor_stop(manager: tauri::State<'_, TorManager>, key: String) -> CmdResult<()> {
    Ok(manager.stop(&key)?)
}

#[tauri::command]
pub async fn tor_new_identity(
    manager: tauri::State<'_, TorManager>,
    key: String,
) -> CmdResult<()> {
    Ok(manager.new_identity(&key).await?)
}

#[tauri::command]
pub fn tor_log(manager: tauri::State<'_, TorManager>, key: String) -> CmdResult<Vec<String>> {
    Ok(manager.log(&key)?)
}

/// The kinds of built-in bridges in `pt_config.json` of the installed
/// bundle, sorted; empty when tor is not installed.
#[tauri::command]
pub fn tor_builtin_bridges(core: tauri::State<'_, Core>) -> Vec<String> {
    source::resolve(&core.app_data_dir)
        .and_then(|bundle| PtConfig::read(&bundle.pt_dir).ok())
        .map(|pt| pt.builtin_kinds())
        .unwrap_or_default()
}
