// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A file the user picked, as something to read from: a local path (given
//! as a `file://` URL by the picker of iOS), or an Android `content://` URI
//! that only the system can open.

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

/// A picked file as the commands take it: the path a `file://` URL names
/// (percent-decoded), anything else as it came. The picker of iOS hands the
/// page a `file://` URL of the copy it made in the app's caches, not a path,
/// and `PathBuf::from` of a URL names no file; an Android `content://` URI
/// stays a URI, for the system to open. Every command that takes a picked
/// file or a place the save dialog chose runs its argument through this
/// first, so a path is checked against the scope of fs as the dialog let
/// it in (the dialog allows the path of the URL, `FilePath::into_path`).
/// A URL that names no local file (one with a host) is left as it is and
/// refused further on as a path never picked.
pub fn picked_path(src: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    let is_file_url = src.get(..7).is_some_and(|s| s.eq_ignore_ascii_case("file://"));
    if !is_file_url {
        return Cow::Borrowed(src);
    }
    match tauri::Url::parse(src).ok().and_then(|url| url.to_file_path().ok()) {
        Some(path) => Cow::Owned(path.to_string_lossy().into_owned()),
        None => Cow::Borrowed(src),
    }
}

pub(crate) fn open(app: &tauri::AppHandle, src: &str) -> Result<FileSource, AppError> {
    let src = picked_path(src);
    let src = src.as_ref();
    #[cfg(target_os = "android")]
    if src.starts_with("content://") {
        return content_uri::source(app, src);
    }
    #[cfg(not(target_os = "ios"))]
    let cache = app.path().app_cache_dir().ok();
    #[cfg(target_os = "ios")]
    let cache = ios_cache_dir(app).or_else(|| app.path().app_cache_dir().ok());
    local(
        PathBuf::from(src),
        app.try_fs_scope().as_ref(),
        cache.as_deref(),
    )
}

/// The app's cache on iOS under the bundle's own identifier, where the
/// plugins of the phone write what they hand over (the gallery's
/// `Cache.root`). Tauri's `app_cache_dir` takes the identifier of the
/// config instead; an app signed under another name — a sideloaded build
/// carries the team in its name, `net.veydan.mobile.<team>` — keeps its
/// cache under that name (2026-11-04). The same lines in
/// tauri-plugin-veydan-media: neither crate may depend on the other.
#[cfg(target_os = "ios")]
fn ios_cache_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    let id = objc2_foundation::NSBundle::mainBundle().bundleIdentifier()?.to_string();
    Some(app.path().cache_dir().ok()?.join(id))
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

    /// The `file://` URL of the picker of iOS is the path it names, decoded:
    /// spaces and letters of any script come back as they were.
    #[cfg(unix)]
    #[test]
    fn a_file_url_is_the_path_it_names() {
        let url = "file:///private/var/mobile/Containers/Data/Application/AB12/Library/Caches/My%20Photo%20%D0%A4%D0%BE%D1%82%D0%BE%20%E5%86%99%E7%9C%9F.HEIC";
        assert_eq!(
            picked_path(url),
            "/private/var/mobile/Containers/Data/Application/AB12/Library/Caches/My Photo Фото 写真.HEIC"
        );
        assert_eq!(picked_path("FILE:///tmp/a%23b.txt"), "/tmp/a#b.txt");
    }

    /// A path, an Android URI and a URL of another host are left as they came.
    #[test]
    fn anything_else_is_left_as_it_came() {
        for src in [
            "/home/me/Pictures/My Photo.jpg",
            "C:\\Users\\me\\photo.jpg",
            "content://media/picker/0/com.android.providers.media.photopicker/media/1000012345",
            "content://net.veydan.mobile.fileprovider/capture/IMG_1.jpg",
            "file:",
            "",
            "фото.jpg",
        ] {
            assert!(matches!(picked_path(src), std::borrow::Cow::Borrowed(s) if s == src), "{src}");
        }
        #[cfg(unix)]
        assert_eq!(picked_path("file://server/share/a.txt"), "file://server/share/a.txt");
    }

    /// The dialog lets in the path of the URL it returns; the path made of
    /// that URL here is the one in the scope.
    #[test]
    fn a_picked_file_url_is_read_as_its_path() {
        let app = tauri::test::mock_app();
        let scope = tauri::fs::Scope::new(&app, &Default::default()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let picked = dir.path().join("Моё фото 1.HEIC");
        std::fs::write(&picked, b"heic").unwrap();
        scope.allow_file(&picked).unwrap();
        let url = tauri::Url::from_file_path(&picked).unwrap().to_string();
        assert!(url.contains("%20"), "{url}");
        let source = local(PathBuf::from(picked_path(&url).as_ref()), Some(&scope), None).unwrap();
        assert_eq!(source.name, "Моё фото 1.HEIC");
        assert_eq!(source.len, Some(4));
    }

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
