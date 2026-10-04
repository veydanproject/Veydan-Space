// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Browser -> notes capture bridge.
//!
//! Camoufox page --(WebExtension)--> native messaging host (this executable,
//! started by the browser with the extension id in argv) --(local socket)-->
//! IPC server inside the running Tauri app --> notes commands.
//!
//! - `extension` — builds the per-profile .xpi and registers the native host manifest
//! - `host`      — stdio native-messaging host mode (no Tauri runtime)
//! - `server`    — local socket server inside the app
//! - `rules`     — the rules a captured page is filed by, and their commands
//!
//! The request and response types the three share are those notes accept
//! (`veydan_notes::capture::protocol`).

pub mod extension;
pub mod host;
pub mod rules;
pub mod server;

/// Gecko extension id; also the argv marker the browser passes to the native host.
pub const EXTENSION_ID: &str = "notes@veydan.net";
/// Native messaging host name (manifest file name and `sendNativeMessage` target).
pub const HOST_NAME: &str = "veydan_notes";

/// True when this process was started by the browser as the native messaging host.
pub fn is_host_invocation() -> bool {
    std::env::args().skip(1).any(|a| a == EXTENSION_ID)
}

/// Set by the app on the browsers it launches and inherited by the native host,
/// so the host reaches the app instance (`--workdir` profile) that owns the browser.
pub const ENDPOINT_ENV: &str = "VEYDAN_CAPTURE_ENDPOINT";

/// Per-user local IPC endpoint the app listens on; per profile under `--workdir`.
pub fn ipc_endpoint() -> String {
    let suffix = veydan_shell::workdir::current()
        .map(|w| format!("-{}", w.tag))
        .unwrap_or_default();
    #[cfg(windows)]
    {
        format!(r"\\.\pipe\veydan-capture{suffix}")
    }
    #[cfg(not(windows))]
    {
        // XDG_RUNTIME_DIR is already per-user; the /tmp fallback is suffixed with the user name.
        if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
            return format!("{}/veydan-capture{suffix}.sock", dir.trim_end_matches('/'));
        }
        let dir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
        let user = std::env::var("USER").unwrap_or_else(|_| "default".to_string());
        format!(
            "{}/veydan-capture-{user}{suffix}.sock",
            dir.trim_end_matches('/')
        )
    }
}

/// Endpoint the native host connects to: the one its browser was launched with.
pub fn host_endpoint() -> String {
    std::env::var(ENDPOINT_ENV)
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(ipc_endpoint)
}
