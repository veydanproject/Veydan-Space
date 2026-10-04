// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Camera / microphone access for the webview.
//!
//! WebKitGTK denies `getUserMedia` unless the app answers the
//! `permission-request` signal; WebView2, WKWebView and Android WebView
//! prompt the user on their own.

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

use tauri::WebviewWindow;

/// Window labels whose webview already answers media permission requests.
static GRANTED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Mutex::default);

/// Allow media capture requests from the app's own pages in this window.
#[tauri::command]
pub fn media_grant_access(window: WebviewWindow) -> Result<(), String> {
    grant_access(&window)
}

/// The body of the command, also behind `Shell::grant_media_access`.
pub(crate) fn grant_access(window: &WebviewWindow) -> Result<(), String> {
    let first = GRANTED
        .lock()
        .map_err(|e| e.to_string())?
        .insert(window.label().to_string());
    if !first {
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    window
        .with_webview(|webview| {
            use webkit2gtk::glib::ObjectExt;
            use webkit2gtk::{PermissionRequestExt, UserMediaPermissionRequest, WebViewExt};

            webview.inner().connect_permission_request(|_, req| {
                if req.is::<UserMediaPermissionRequest>() {
                    req.allow();
                    true
                } else {
                    false
                }
            });
        })
        .map_err(|e| e.to_string())?;

    Ok(())
}
