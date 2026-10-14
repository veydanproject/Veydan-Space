// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A file the user picked, as something to read from: a local path, or an
//! Android `content://` URI that only the system can open.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::Manager;
use tauri_plugin_fs::FsExt;
use veydan_core::AppError;

#[cfg(target_os = "android")]
mod content_uri;

/// Opens the file anew on each call, so a reader can start over.
pub type Opener = Arc<dyn Fn() -> std::io::Result<std::fs::File> + Send + Sync>;

/// Streaming input: nothing of the file is read until `open` is called.
pub struct FileSource {
    /// The name to show and to store the file under.
    pub name: String,
    /// The size, when the source tells it.
    pub len: Option<u64>,
    pub open: Opener,
}

pub(crate) fn open(app: &tauri::AppHandle, src: &str) -> Result<FileSource, AppError> {
    #[cfg(target_os = "android")]
    if src.starts_with("content://") {
        return content_uri::source(app, src);
    }
    let cache = app.path().app_cache_dir().ok();
    local(
        PathBuf::from(src),
        app.try_fs_scope().as_ref(),
        cache.as_deref(),
    )
}

/// Let a file the user gave by another way than the picker (the clipboard)
/// into the bound `open` reads within.
pub(crate) fn allow(app: &tauri::AppHandle, path: &Path) {
    if let Some(scope) = app.try_fs_scope() {
        let _ = scope.allow_file(path);
    }
}

/// A local path is read only when the user picked it: the file dialog and a
/// drop of files on a window let it into the scope of the fs plugin (a module
/// that takes paths from elsewhere, such as the clipboard, lets them in
/// itself). A file inside the app's own cache is read too: the fs plugin
/// lets a page write there and nowhere else, and the phone, which has no raw
/// IPC, hands bytes over through a file there. Any other path a page names
/// is refused, so a script cannot copy `~/.ssh` or the data file into an
/// attachment.
fn local(
    path: PathBuf,
    scope: Option<&tauri::fs::Scope>,
    cache: Option<&Path>,
) -> Result<FileSource, AppError> {
    let in_cache = || {
        let (Some(cache), Ok(real)) = (cache, std::fs::canonicalize(&path)) else {
            return false;
        };
        std::fs::canonicalize(cache).is_ok_and(|cache| real.starts_with(cache))
    };
    if !scope.is_some_and(|s| s.is_allowed(&path)) && !in_cache() {
        return Err(AppError::Other("the file was not picked".into()));
    }
    let len = std::fs::metadata(&path).map_err(AppError::io)?.len();
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    Ok(FileSource {
        name,
        len: Some(len),
        open: Arc::new(move || std::fs::File::open(&path)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_path_in_the_scope_is_read() {
        let app = tauri::test::mock_app();
        let scope = tauri::fs::Scope::new(&app, &Default::default()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let picked = dir.path().join("picked.txt");
        let other = dir.path().join("other.txt");
        std::fs::write(&picked, b"hello").unwrap();
        std::fs::write(&other, b"secret").unwrap();

        assert!(
            local(picked.clone(), None, None).is_err(),
            "no fs plugin: nothing is read"
        );
        assert!(
            local(picked.clone(), Some(&scope), None).is_err(),
            "not picked yet"
        );

        scope.allow_file(&picked).unwrap();
        let source = local(picked.clone(), Some(&scope), None).unwrap();
        assert_eq!(source.name, "picked.txt");
        assert_eq!(source.len, Some(5));
        // The file next to it, and a way to it through `..`, stay out.
        assert!(local(other.clone(), Some(&scope), None).is_err());
        let around = dir.path().join("x/../other.txt");
        assert!(local(around, Some(&scope), None).is_err());
    }

    /// A file the page wrote into the app's cache is read without a pick;
    /// one beside the cache, or named through the cache with `..`, is not.
    #[test]
    fn a_file_in_the_apps_cache_is_read_without_a_pick() {
        let app = tauri::test::mock_app();
        let scope = tauri::fs::Scope::new(&app, &Default::default()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(cache.join("deep")).unwrap();
        let written = cache.join("deep/photo.jpg");
        let outside = dir.path().join("outside.txt");
        std::fs::write(&written, b"img").unwrap();
        std::fs::write(&outside, b"no").unwrap();

        assert!(
            local(written.clone(), Some(&scope), None).is_err(),
            "no cache named: not read"
        );
        let source = local(written.clone(), Some(&scope), Some(&cache)).unwrap();
        assert_eq!(source.name, "photo.jpg");
        assert_eq!(source.len, Some(3));
        assert!(local(outside.clone(), Some(&scope), Some(&cache)).is_err());
        let around = cache.join("../outside.txt");
        assert!(local(around, Some(&scope), Some(&cache)).is_err());
        let missing = cache.join("gone.txt");
        assert!(local(missing, Some(&scope), Some(&cache)).is_err());
    }
}
