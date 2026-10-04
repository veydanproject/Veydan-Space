// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! App-level UI settings: tray behavior, tray menu labels, the UI language.

use crate::module::TrayLabels;
use crate::services::Shell;
use crate::tray::{self, TraySettings};
use sqlx::{Pool, Sqlite};
use veydan_core::{settings, AppError, CmdResult, Core};

#[tauri::command]
pub async fn tray_settings_get(shell: tauri::State<'_, Shell>) -> CmdResult<TraySettings> {
    Ok(shell.tray_settings())
}

#[tauri::command]
pub async fn tray_settings_set(
    minimize_to_tray: bool,
    close_to_tray: bool,
    start_hidden: bool,
    shell: tauri::State<'_, Shell>,
) -> CmdResult<()> {
    shell
        .set_tray_settings(TraySettings {
            minimize_to_tray,
            close_to_tray,
            start_hidden,
        })
        .await
}

/// Minimize action for the custom titlebar's "–" button. With minimize-to-tray
/// on, the main window is hidden to the tray (leaves the taskbar) instead of
/// being iconified. Other windows (e.g. notes) always minimize themselves.
#[tauri::command]
pub async fn window_minimize(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    shell: tauri::State<'_, Shell>,
) -> CmdResult<()> {
    if shell.tray_settings().minimize_to_tray && window.label() == "main" {
        tray::hide_main_window(&app);
        return Ok(());
    }
    let win = window.clone();
    app.run_on_main_thread(move || {
        let _ = win.minimize();
    })
    .map_err(AppError::other)?;
    Ok(())
}

const UI_LOCALE_KEY: &str = "ui_locale";

/// Persist the UI language so non-frontend consumers (browser extension) can follow it.
#[tauri::command]
pub async fn app_locale_set(locale: String, core: tauri::State<'_, Core>) -> CmdResult<()> {
    let locale = match locale.as_str() {
        "ru" => "ru",
        _ => "en",
    };
    settings::set(&core.db, UI_LOCALE_KEY, locale).await
}

#[tauri::command]
pub async fn app_locale_get(core: tauri::State<'_, Core>) -> CmdResult<String> {
    Ok(app_locale(&core.db).await)
}

/// Stored UI language, `en` when never set.
pub async fn app_locale(db: &Pool<Sqlite>) -> String {
    settings::get(db, UI_LOCALE_KEY)
        .await
        .unwrap_or_else(|| "en".to_string())
}

/// Hand the tray the active locale's menu strings and refresh it.
#[tauri::command]
pub async fn tray_set_labels(labels: TrayLabels, shell: tauri::State<'_, Shell>) -> CmdResult<()> {
    shell.tray.set_labels(labels)?;
    shell.tray_refresh();
    Ok(())
}
