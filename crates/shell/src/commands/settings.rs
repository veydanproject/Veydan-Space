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

/// Whether a value is the code of a language of the UI as
/// `ui/src/lib/core/languages.json` writes them: `en`, `de`, `pt-BR`, `zh-CN`.
/// The list itself is the UI's: a newer version's language passes through an
/// older one (the value syncs) and is shown in English there.
fn is_locale_code(code: &str) -> bool {
    let (lang, region) = match code.split_once('-') {
        Some((lang, region)) => (lang, Some(region)),
        None => (code, None),
    };
    (2..=3).contains(&lang.len())
        && lang.bytes().all(|b| b.is_ascii_lowercase())
        && region.is_none_or(|r| r.len() == 2 && r.bytes().all(|b| b.is_ascii_uppercase()))
}

/// Persist the UI language the user chose, so the browser extension and the
/// other computers (the value syncs) follow it.
#[tauri::command]
pub async fn app_locale_set(locale: String, core: tauri::State<'_, Core>) -> CmdResult<()> {
    if !is_locale_code(&locale) {
        return Err(AppError::Other(format!("not a language code: {locale}")));
    }
    settings::set(&core.db, UI_LOCALE_KEY, &locale).await
}

/// The language chosen here or on another computer; none when no one chose
/// one (the UI then follows the system's).
#[tauri::command]
pub async fn app_locale_get(core: tauri::State<'_, Core>) -> CmdResult<Option<String>> {
    Ok(settings::get(&core.db, UI_LOCALE_KEY).await.filter(|c| is_locale_code(c)))
}

/// Stored UI language, `en` when never set.
pub async fn app_locale(db: &Pool<Sqlite>) -> String {
    settings::get(db, UI_LOCALE_KEY)
        .await
        .filter(|c| is_locale_code(c))
        .unwrap_or_else(|| "en".to_string())
}

/// Hand the tray the active locale's menu strings and refresh it.
#[tauri::command]
pub async fn tray_set_labels(labels: TrayLabels, shell: tauri::State<'_, Shell>) -> CmdResult<()> {
    shell.tray.set_labels(labels)?;
    shell.tray_refresh();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_locale_code;

    #[test]
    fn language_codes_of_the_ui_pass_and_nothing_else() {
        for code in ["en", "ru", "uk", "de", "pt-BR", "zh-CN", "fil"] {
            assert!(is_locale_code(code), "{code}");
        }
        for code in ["", "e", "EN", "pt_BR", "pt-br", "zh-Hans", "english", "de-", "-DE", "de-DE-x"] {
            assert!(!is_locale_code(code), "{code}");
        }
    }
}
