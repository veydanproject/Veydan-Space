// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use std::time::Duration;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use veydan_core::{AppError, CmdResult};

pub(crate) const NOTES_WINDOW_LABEL: &str = "notes";

/// Open the standalone notes window, or focus it if it already exists.
#[tauri::command]
pub async fn note_open_window(title: String, app: tauri::AppHandle) -> CmdResult<()> {
    open_notes_window(app, title, None).await
}

/// Bring a window above other apps; X11 ignores plain focus requests from a background app.
pub(crate) fn raise_window(w: &WebviewWindow) {
    let _ = w.unminimize();
    let _ = w.show();
    let _ = w.set_always_on_top(true);
    let _ = w.set_focus();
    let w = w.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(400)).await;
        let _ = w.set_always_on_top(false);
    });
}

/// `open_id`: note to show. A new window gets it via the URL (the page is not loaded yet
/// to receive events); an existing window gets a `notes://open` event.
pub(crate) async fn open_notes_window(
    app: tauri::AppHandle,
    title: String,
    open_id: Option<String>,
) -> CmdResult<()> {
    let title = if title.trim().is_empty() {
        "Notes".to_string()
    } else {
        title
    };
    let app2 = app.clone();
    app.run_on_main_thread(move || {
        if let Some(w) = app2.get_webview_window(NOTES_WINDOW_LABEL) {
            raise_window(&w);
            if let Some(id) = open_id {
                let _ = w.emit("notes://open", &id);
            }
            return;
        }

        let url = match open_id {
            Some(id) => format!("/notes?open={id}"),
            None => "/notes".to_string(),
        };
        let builder =
            WebviewWindowBuilder::new(&app2, NOTES_WINDOW_LABEL, WebviewUrl::App(url.into()))
                .title(title)
                .inner_size(960.0, 700.0)
                .min_inner_size(640.0, 480.0);
        let builder = veydan_shell::workdir::apply_to_window(builder);
        // Rebound, not mutated: on Windows and macOS nothing changes it, and
        // a `mut` would be unused there.
        #[cfg(target_os = "linux")]
        let builder = builder.decorations(false).transparent(true);

        match builder.build() {
            Ok(w) => raise_window(&w),
            Err(e) => eprintln!("[notes] failed to open window: {e}"),
        }
    })
    .map_err(AppError::other)?;
    Ok(())
}
