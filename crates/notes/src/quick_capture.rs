// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Quick capture: a small always-on-top window, opened from the tray, the
//! command palette and the notes page. There is no global shortcut: Space
//! and Notes on one computer asked for the same key (platform-spec 20.18).

use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use veydan_core::{AppError, CmdResult};

pub(crate) const WINDOW_LABEL: &str = "quick-capture";

/// Show the quick capture window, creating it on first use. Safe from any thread.
pub fn show_quick_capture(app: &tauri::AppHandle) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = app2.get_webview_window(WINDOW_LABEL) {
            let _ = w.show();
            let _ = w.unminimize();
            let _ = w.set_focus();
            let _ = w.emit("quick-capture://shown", ());
            return;
        }
        let builder =
            WebviewWindowBuilder::new(&app2, WINDOW_LABEL, WebviewUrl::App("/notes/quick".into()))
                .title("Quick capture")
                .inner_size(480.0, 340.0)
                .min_inner_size(360.0, 240.0)
                .always_on_top(true)
                .skip_taskbar(true)
                .center();
        let builder = veydan_shell::workdir::apply_to_window(builder);
        // Rebound, not mutated: on Windows and macOS nothing changes it, and
        // a `mut` would be unused there.
        #[cfg(target_os = "linux")]
        let builder = builder.decorations(false).transparent(true);
        if let Err(e) = builder.build() {
            eprintln!("[quick-capture] failed to open window: {e}");
        }
    });
}

/// Refused while the module is switched off: quick capture writes into the notes.
#[tauri::command]
pub async fn open_quick_capture(app: tauri::AppHandle) -> CmdResult<()> {
    if !app
        .state::<veydan_shell::Shell>()
        .module_enabled(super::MODULE_ID)
    {
        return Err(AppError::other("notes_off"));
    }
    show_quick_capture(&app);
    Ok(())
}
