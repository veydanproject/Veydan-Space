// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The iPhone's half of the bridge `VeydanChrome` (platform-spec 13.6):
//! what Android's `ActivityDelegate` gives the page as a JavaScript object,
//! iOS gives as these commands, and the page's adapter
//! (`ui/src/lib/core/ui/chrome.ts`) shows the callers the same names.
//! - the look of the status bar and the colour behind the page from the
//!   page's theme (`chrome_bars`);
//! - a picture or a PDF on the clipboard, written to the app's cache for the
//!   composer to attach (`chrome_clipboard_has_image`, `chrome_clipboard_image`):
//!   only when the web view's own `paste` handed over no file;
//! - the edges of the screen the system waits on while the page holds a drag
//!   that starts there (`chrome_edges`).
//!
//! Elsewhere they do nothing: a computer has no bars and reads its clipboard
//! in the runtime, and Android's page talks to its own bridge.

#[cfg(target_os = "ios")]
mod ios;

/// The status bar readable on the page's theme: dark text on a `light`
/// theme, light text on a dark one; `background` (`#rrggbb`, the page's
/// `--bg`) behind the web view, where the safe areas, a rotation or the
/// keyboard's move show the window.
#[tauri::command]
pub async fn chrome_bars(window: tauri::WebviewWindow, light: bool, background: Option<String>) -> Result<(), String> {
    let color = background.as_deref().and_then(hex_color);
    #[cfg(target_os = "ios")]
    {
        // On the main thread, where UIKit is touched.
        window
            .with_webview(move |webview| ios::bars(webview.inner(), webview.view_controller(), light, color))
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "ios"))]
    {
        let _ = (window, light, color);
        Ok(())
    }
}

/// The edges of the screen whose system gestures wait for a second swipe
/// (`UIRectEdge`: 1 top, 2 left, 4 bottom, 8 right; 0 gives them back): the
/// page holds a drag that starts there.
#[tauri::command]
pub async fn chrome_edges(window: tauri::WebviewWindow, edges: u8) -> Result<(), String> {
    let edges = edges & EDGES_ALL;
    #[cfg(target_os = "ios")]
    {
        window
            .with_webview(move |webview| ios::defer_edges(webview.view_controller(), edges))
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "ios"))]
    {
        let _ = (window, edges);
        Ok(())
    }
}

/// Whether the clipboard holds a picture or a PDF. Only what it holds is
/// asked, not the thing itself, which iOS does not announce.
#[tauri::command]
pub async fn chrome_clipboard_has_image() -> bool {
    #[cfg(target_os = "ios")]
    {
        tauri::async_runtime::spawn_blocking(ios::clipboard_has)
            .await
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "ios"))]
    {
        false
    }
}

/// The picture (or PDF) on the clipboard as a file of the app's cache,
/// let into the scope of fs, as `messenger_media_import` takes a picked
/// file; `None` when there is none, or the user said no to iOS's «Allow
/// Paste». Read on the user's tap only: the read is what iOS asks about,
/// and it waits off the main thread until the user answers.
#[tauri::command]
pub async fn chrome_clipboard_image(app: tauri::AppHandle) -> Result<Option<String>, String> {
    #[cfg(target_os = "ios")]
    {
        let Some(dir) = crate::source::ios_cache_dir(&app) else {
            return Err("no cache directory".into());
        };
        let dir = dir.join(CLIPBOARD_DIR);
        let written = tauri::async_runtime::spawn_blocking(move || {
            let Some((bytes, ext)) = ios::clipboard_take() else {
                return Ok(None);
            };
            write_paste(&dir, &bytes, ext, std::time::SystemTime::now()).map(Some)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
        Ok(written.map(|path| {
            crate::source::allow(&app, &path);
            path.to_string_lossy().into_owned()
        }))
    }
    #[cfg(not(target_os = "ios"))]
    {
        let _ = app;
        Ok(None)
    }
}

/// Every edge `chrome_edges` takes.
const EDGES_ALL: u8 = 0b1111;

/// The folder of the cache the pastes are written to.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
const CLIPBOARD_DIR: &str = "clipboard";

/// How long a paste stays in the cache: a draft holds its path until it is
/// sent, so an older paste goes only with the next one, and a day later.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
const PASTE_KEEP: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// `#rrggbb` (any case, spaces around) as its three bytes; anything else is
/// no colour.
fn hex_color(css: &str) -> Option<[u8; 3]> {
    let hex = css.trim().strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// `bytes` as a new file `paste-<ms>.<ext>` in `dir` (made when missing),
/// after the pastes older than [`PASTE_KEEP`] there go.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
fn write_paste(
    dir: &std::path::Path,
    bytes: &[u8],
    ext: &str,
    now: std::time::SystemTime,
) -> std::io::Result<std::path::PathBuf> {
    std::fs::create_dir_all(dir)?;
    for entry in std::fs::read_dir(dir)?.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|at| now.duration_since(at).is_ok_and(|age| age > PASTE_KEEP));
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let ms = now
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    // Two pastes in one millisecond keep apart.
    let mut path = dir.join(format!("paste-{ms}.{ext}"));
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("paste-{ms}-{n}.{ext}"));
        n += 1;
    }
    std::fs::write(&path, bytes)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_css_hex_colour_is_its_three_bytes() {
        assert_eq!(hex_color("#15171e"), Some([0x15, 0x17, 0x1e]));
        assert_eq!(hex_color("  #ECEEF3 "), Some([0xec, 0xee, 0xf3]));
        assert_eq!(hex_color("#fff"), None);
        assert_eq!(hex_color("rgb(1, 2, 3)"), None);
        assert_eq!(hex_color("#12345g"), None);
        assert_eq!(hex_color(""), None);
    }

    #[test]
    fn a_paste_is_a_new_file_and_the_old_ones_go() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().join(CLIPBOARD_DIR);
        let now = std::time::SystemTime::now();
        let first = write_paste(&dir, b"one", "png", now).unwrap();
        let second = write_paste(&dir, b"two", "png", now).unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&first).unwrap(), b"one");
        assert_eq!(std::fs::read(&second).unwrap(), b"two");
        assert!(first.file_name().unwrap().to_string_lossy().starts_with("paste-"));
        // A day and more later both are old: only the new paste is left.
        let later = now + PASTE_KEEP + std::time::Duration::from_secs(60);
        let third = write_paste(&dir, b"three", "jpg", later).unwrap();
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).collect();
        assert_eq!(left, vec![third]);
    }
}
