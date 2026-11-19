// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! «Save as» on iOS (platform-spec 13.6), for every module through
//! `ui/src/lib/core/ui/saveAs.ts`. iOS has no place outside the app to write
//! to (the address a save dialog returns needs a security-scoped access), so
//! the page writes the file into a staged copy in the app's cache
//! (`files_stage`), and the system's document picker for exporting
//! (`UIDocumentPickerViewController(forExporting:asCopy:)`) lets the user pick
//! a folder in Files, where the system copies it itself (`files_save`). A
//! computer and Android save through their dialog and never call these: they
//! answer that there is no such place.

#[cfg(target_os = "ios")]
mod ios;

/// The folder of the app's cache where a file waits to be saved to Files,
/// each in a folder of its own; emptied when the app starts.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
const STAGE_DIR: &str = "export-stage";

/// What the commands answer where the page saves through a dialog.
#[cfg(not(target_os = "ios"))]
const NO_STAGE: &str = "no staging of files here: the save dialog writes the file";

/// Step one: a place in the app's cache for the file the page is about to
/// write under `name` (the path of it), and the scope of fs lets that one
/// file be written, as it lets the file a save dialog returns on a computer.
#[tauri::command]
pub async fn files_stage(name: String, app: tauri::AppHandle) -> Result<String, String> {
    #[cfg(target_os = "ios")]
    {
        let root = stage_root(&app).ok_or("no cache directory")?;
        let path = stage(&root, &name).map_err(|e| e.to_string())?;
        crate::source::allow(&app, &path);
        Ok(path.to_string_lossy().into_owned())
    }
    #[cfg(not(target_os = "ios"))]
    {
        let _ = (name, app);
        Err(NO_STAGE.into())
    }
}

/// Step two: the file written at `path` (what `files_stage` answered) is
/// offered to the system's document picker for exporting; the user picks a
/// folder in Files and a copy lands there. `true` when saved, `false` when
/// the user gave up. The staged folder is deleted either way; with `discard`
/// nothing is shown and it is only deleted (the write failed: the file may
/// hold a secret, the recovery key). A path `files_stage` did not make is
/// refused with `not_staged`.
#[tauri::command]
pub async fn files_save(path: String, discard: Option<bool>, window: tauri::WebviewWindow) -> Result<bool, String> {
    #[cfg(target_os = "ios")]
    {
        use tauri::Manager;
        let root = stage_root(window.app_handle()).ok_or("no cache directory")?;
        let dir = staged_folder(&root, std::path::Path::new(&path)).ok_or("not_staged")?;
        if discard == Some(true) {
            let _ = std::fs::remove_dir_all(&dir);
            return Ok(false);
        }
        let saved = present(&window, std::path::PathBuf::from(&path)).await;
        let _ = std::fs::remove_dir_all(&dir);
        saved
    }
    #[cfg(not(target_os = "ios"))]
    {
        let _ = (path, discard, window);
        Err(NO_STAGE.into())
    }
}

/// What an export left when the app was closed in the middle of it goes at
/// the start.
pub(crate) fn clear_stage(app: &tauri::AppHandle) {
    #[cfg(target_os = "ios")]
    if let Some(root) = stage_root(app) {
        let _ = std::fs::remove_dir_all(root);
    }
    #[cfg(not(target_os = "ios"))]
    let _ = app;
}

#[cfg(target_os = "ios")]
fn stage_root(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    let cache = crate::source::ios_cache_dir(app).or_else(|| app.path().app_cache_dir().ok())?;
    Some(cache.join(STAGE_DIR))
}

/// The document picker over the page, on the main thread; its answer once
/// the user picks a folder or gives up.
#[cfg(target_os = "ios")]
async fn present(window: &tauri::WebviewWindow, file: std::path::PathBuf) -> Result<bool, String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<bool, &'static str>>();
    window
        .with_webview(move |webview| {
            ios::export(webview.view_controller(), &file, Box::new(move |answer| {
                let _ = tx.send(answer);
            }));
        })
        .map_err(|e| e.to_string())?;
    match rx.await {
        Ok(answer) => answer.map_err(String::from),
        // The sender went without an answer: the picker was not shown.
        Err(_) => Err("no_screen".into()),
    }
}

/// A new folder of its own in `root` and the path of `name` in it.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
fn stage(root: &std::path::Path, name: &str) -> std::io::Result<std::path::PathBuf> {
    let dir = root.join(unique_name());
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join(clean_name(name)))
}

/// A name for a folder that two exports never share.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
fn unique_name() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static N: AtomicU64 = AtomicU64::new(0);
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{t:x}-{:x}", N.fetch_add(1, Ordering::Relaxed))
}

/// `name` as a file name, so that the file keeps the name the user sees in
/// Files: no folders, no control characters, not hidden, at most 120 bytes,
/// `file` when nothing is left.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
fn clean_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or("");
    let kept: String = last.chars().filter(|c| !c.is_control() && *c != ':').collect();
    let mut out = kept.trim().trim_start_matches('.').to_string();
    while out.len() > 120 {
        out.pop();
    }
    if out.is_empty() {
        "file".into()
    } else {
        out
    }
}

/// The folder of the staged file `path` (a file one folder deep in `root`,
/// links and `..` resolved), `None` for anything else.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
fn staged_folder(root: &std::path::Path, path: &std::path::Path) -> Option<std::path::PathBuf> {
    let real = std::fs::canonicalize(path).ok()?;
    let root = std::fs::canonicalize(root).ok()?;
    let dir = real.parent()?;
    (real.is_file() && dir.parent() == Some(root.as_path())).then(|| dir.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_a_file_name_the_user_sees() {
        assert_eq!(clean_name("Отчёт 2026.pdf"), "Отчёт 2026.pdf");
        assert_eq!(clean_name("../../etc/passwd"), "passwd");
        assert_eq!(clean_name("a\\b\\c.txt"), "c.txt");
        assert_eq!(clean_name(".hidden"), "hidden");
        assert_eq!(clean_name("a:b\u{7}.txt"), "ab.txt");
        assert_eq!(clean_name("  "), "file");
        assert_eq!(clean_name("dir/"), "file");
        let long = "я".repeat(100);
        let cut = clean_name(&long);
        assert!(cut.len() <= 120 && cut.chars().all(|c| c == 'я'));
    }

    #[test]
    fn only_a_staged_file_is_saved() {
        let cache = tempfile::tempdir().unwrap();
        let root = cache.path().join(STAGE_DIR);
        let one = stage(&root, "key.txt").unwrap();
        let two = stage(&root, "key.txt").unwrap();
        assert_ne!(one.parent(), two.parent());
        // Not written yet: nothing to save.
        assert_eq!(staged_folder(&root, &one), None);
        std::fs::write(&one, b"secret").unwrap();
        assert_eq!(staged_folder(&root, &one), Some(std::fs::canonicalize(one.parent().unwrap()).unwrap()));
        // A file elsewhere, right in the root, or out of it by `..`, is not staged.
        let outside = cache.path().join("other.txt");
        std::fs::write(&outside, b"x").unwrap();
        assert_eq!(staged_folder(&root, &outside), None);
        let flat = root.join("flat.txt");
        std::fs::write(&flat, b"x").unwrap();
        assert_eq!(staged_folder(&root, &flat), None);
        let up = one.parent().unwrap().join("..").join("..").join("other.txt");
        assert_eq!(staged_folder(&root, &up), None);
    }
}
