// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What the UI asks of the host it runs on.

/// Runtime facts for the settings screen.
#[derive(serde::Serialize)]
pub struct HostInfo {
    os: &'static str,
    arch: &'static str,
    version: String,
}

/// The version is the product's: `CARGO_PKG_VERSION` here would be the
/// shell's, the same in every product.
#[tauri::command]
pub fn host_info(app: tauri::AppHandle) -> HostInfo {
    HostInfo {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        version: app.package_info().version.to_string(),
    }
}

/// Open an http(s) URL in the default browser, or a mailto: link in the mail client.
/// The opener plugin never goes through a shell, so URL contents cannot become commands.
#[cfg(desktop)]
#[tauri::command]
pub fn open_url(url: String, app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let parsed = url::Url::parse(&url).map_err(|e| e.to_string())?;
    if !matches!(parsed.scheme(), "http" | "https" | "mailto") {
        return Err("only http(s) and mailto URLs are allowed".into());
    }
    app.opener()
        .open_url(parsed.as_str(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// Put text on the OS clipboard. Unlike `navigator.clipboard` it does not need a
/// user-gesture context, so it works after awaited backend calls.
#[cfg(desktop)]
#[tauri::command]
pub async fn clipboard_write_text(text: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
        cb.set_text(text).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Whether the updater can install updates in-place for this install method.
/// On Linux only AppImage is updatable; deb/rpm installs must download manually.
#[cfg(desktop)]
#[tauri::command]
pub fn update_supported() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("APPIMAGE").is_some()
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}
