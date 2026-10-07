// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Builds the per-profile extension package and registers the native messaging host.
//!
//! Spike results (Camoufox 152, omni.ja inspection):
//! - `MOZ_REQUIRE_SIGNING=false`, so `xpinstall.signatures.required=false` is honored;
//!   `extensions.autoDisableScopes=0` / `enabledScopes=5` are already in camoufox.cfg,
//!   hence `firefox-profile/extensions/{id}.xpi` loads enabled without a prompt.
//! - Native manifests are looked up under the stock Mozilla locations
//!   (`~/.mozilla/native-messaging-hosts`, `~/Library/Application Support/Mozilla/
//!   NativeMessagingHosts`, `HKCU\Software\Mozilla\NativeMessagingHosts`).
//! - Fingerprint surface: no content scripts, no web_accessible_resources, no network.

use super::{EXTENSION_ID, HOST_NAME};
use std::io::Write;
use std::path::{Path, PathBuf};

const MANIFEST: &str = include_str!("../../extension/manifest.json");

/// Static files bundled as-is.
const FILES: &[(&str, &str)] = &[
    (
        "background.js",
        include_str!("../../extension/background.js"),
    ),
    (
        "article.js",
        include_str!("../../extension/article.js"),
    ),
    (
        "popup.html",
        include_str!("../../extension/popup.html"),
    ),
    (
        "popup.js",
        include_str!("../../extension/popup.js"),
    ),
    (
        "i18n.js",
        include_str!("../../extension/i18n.js"),
    ),
];

/// Toolbar/add-on manager icons, reused from the app icon set.
const ICONS: &[(&str, &[u8])] = &[
    ("icons/32.png", include_bytes!("../../icons/32x32.png")),
    ("icons/64.png", include_bytes!("../../icons/64x64.png")),
    ("icons/128.png", include_bytes!("../../icons/128x128.png")),
    // The CJK fonts of the UI (ui/src/lib/core/fonts, scripts/fonts-cjk.mjs):
    // the popup in Chinese, Japanese or Korean on a system without such fonts.
    ("fonts/cjk-sc.woff2", include_bytes!("../../../../ui/src/lib/core/fonts/cjk-sc.woff2")),
    ("fonts/cjk-jp.woff2", include_bytes!("../../../../ui/src/lib/core/fonts/cjk-jp.woff2")),
    ("fonts/cjk-kr.woff2", include_bytes!("../../../../ui/src/lib/core/fonts/cjk-kr.woff2")),
];

fn build_xpi(profile_id: &str, locale: &str) -> Result<Vec<u8>, String> {
    use zip::write::SimpleFileOptions;
    let manifest = MANIFEST.replace("__VERSION__", env!("CARGO_PKG_VERSION"));
    let json = |s: &str| serde_json::to_string(s).map_err(|e| e.to_string());
    // Per-profile constants read by background.js and popup.js
    let config = format!(
        "const PROFILE_ID = {};\nconst LOCALE = {};\n",
        json(profile_id)?,
        json(locale)?
    );

    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let opts =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let generated = [
            ("manifest.json", manifest.as_str()),
            ("config.js", config.as_str()),
        ];
        let texts = generated
            .iter()
            .chain(FILES.iter())
            .map(|(n, b)| (*n, b.as_bytes()));
        for (name, body) in texts.chain(ICONS.iter().copied()) {
            zip.start_file(name, opts).map_err(|e| e.to_string())?;
            zip.write_all(body).map_err(|e| e.to_string())?;
        }
        zip.finish().map_err(|e| e.to_string())?;
    }
    Ok(buf.into_inner())
}

/// Write `extensions/{id}.xpi` into the Firefox profile; skipped when unchanged
/// so the add-on manager does not see a modified file on every launch.
pub fn install_extension(
    firefox_profile_dir: &Path,
    profile_id: &str,
    locale: &str,
) -> Result<(), String> {
    let dir = firefox_profile_dir.join("extensions");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let target = dir.join(format!("{EXTENSION_ID}.xpi"));
    let bytes = build_xpi(profile_id, locale)?;
    if std::fs::read(&target)
        .map(|cur| cur == bytes)
        .unwrap_or(false)
    {
        return Ok(());
    }
    std::fs::write(&target, bytes).map_err(|e| e.to_string())
}

/// Executable the browser must start as the host: the AppImage itself when packaged that way.
fn host_executable() -> Option<PathBuf> {
    std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
}

fn manifest_json(exe: &Path) -> String {
    serde_json::json!({
        "name": HOST_NAME,
        "description": "Veydan notes capture host",
        "path": exe.to_string_lossy(),
        "type": "stdio",
        "allowed_extensions": [EXTENSION_ID],
    })
    .to_string()
}

#[cfg(target_os = "linux")]
fn manifest_path(_app_data_dir: &Path) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".mozilla")
            .join("native-messaging-hosts")
            .join(format!("{HOST_NAME}.json")),
    )
}

#[cfg(target_os = "macos")]
fn manifest_path(_app_data_dir: &Path) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library/Application Support/Mozilla/NativeMessagingHosts")
            .join(format!("{HOST_NAME}.json")),
    )
}

#[cfg(windows)]
fn manifest_path(app_data_dir: &Path) -> Option<PathBuf> {
    Some(app_data_dir.join(format!("{HOST_NAME}.json")))
}

/// Register (or refresh) the native messaging host manifest for the current executable.
pub fn register_native_host(app_data_dir: &Path) -> Result<(), String> {
    let exe = host_executable().ok_or("cannot resolve executable path")?;
    let path = manifest_path(app_data_dir).ok_or("cannot resolve manifest location")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = manifest_json(&exe);
    if std::fs::read_to_string(&path)
        .map(|cur| cur == json)
        .unwrap_or(false)
    {
        return Ok(());
    }
    std::fs::write(&path, json).map_err(|e| e.to_string())?;

    #[cfg(windows)]
    {
        // Firefox on Windows finds the manifest through the registry, not a fixed dir.
        let key = format!(r"HKCU\Software\Mozilla\NativeMessagingHosts\{HOST_NAME}");
        std::process::Command::new("reg")
            .args([
                "add",
                &key,
                "/ve",
                "/t",
                "REG_SZ",
                "/d",
                &path.to_string_lossy(),
                "/f",
            ])
            .output()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
