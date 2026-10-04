// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `--workdir <dir>`: an independent profile of the whole app.
//!
//! Everything the app persists lives in `<dir>/<product id>/`: the data
//! directory `data/` (database, browser profiles, notes, messenger), the
//! webview storage `webview/` and the cache `cache/`. The per-user resources (single-instance lock,
//! capture socket, tray id) are keyed by that directory, so several profiles
//! run side by side, and so do two products given the same `<dir>`. Without
//! the flag nothing changes.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tauri::{Manager, Runtime, WebviewWindowBuilder};

const FLAG: &str = "workdir";
/// Same as the flag; the flag wins when both are given.
pub const ENV_VAR: &str = "VEYDAN_WORKDIR";

static CURRENT: OnceLock<Option<Workdir>> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct Workdir {
    /// `<dir>/<product id>`, absolute: stands in for the app data directory
    /// the OS would give.
    pub root: PathBuf,
    /// Short stable id derived from `root`; suffixes per-user resource names.
    pub tag: String,
    /// Name of `<dir>`, shown in the window title and the tray.
    pub name: String,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    store_id: [u8; 16],
}

impl Workdir {
    fn new(dir: PathBuf, product: &str) -> Self {
        let root = dir.join(product);
        // Windows paths are case-insensitive: one directory, one profile.
        #[cfg(windows)]
        let key = root.to_string_lossy().to_lowercase();
        #[cfg(not(windows))]
        let key = root.to_string_lossy().into_owned();

        let digest = Sha256::digest(key.as_bytes());
        let tag = digest[..4].iter().map(|b| format!("{b:02x}")).collect();
        let mut store_id = [0u8; 16];
        store_id.copy_from_slice(&digest[..16]);
        let name = dir
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| dir.to_string_lossy().into_owned());
        Self {
            root,
            tag,
            name,
            store_id,
        }
    }

    /// Webview storage (localStorage etc.) of this profile.
    pub fn webview_dir(&self) -> PathBuf {
        self.root.join("webview")
    }

    /// Stands in for the app cache directory the OS would give.
    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }

    /// Key the single-instance lock by the profile and name the config
    /// windows after it: `product_name` starts their titles. The shell builds
    /// them in code, where their data directory is set (`apply_to_window`).
    pub fn apply_to_context<R: Runtime>(&self, context: &mut tauri::Context<R>, product_name: &str) {
        let config = context.config_mut();
        // `w`: a D-Bus name element must not start with a digit.
        config.identifier = format!("{}.w{}", config.identifier, self.tag);
        for window in &mut config.app.windows {
            window.title = format!("{product_name} — {}", self.name);
        }
    }

    fn isolate<'a, R: Runtime, M: Manager<R>>(
        &self,
        builder: WebviewWindowBuilder<'a, R, M>,
    ) -> WebviewWindowBuilder<'a, R, M> {
        // WKWebView has no data directory; the data store needs macOS 14+,
        // older systems fall back to the shared default store.
        #[cfg(target_os = "macos")]
        {
            builder.data_store_identifier(self.store_id)
        }
        #[cfg(not(target_os = "macos"))]
        {
            builder.data_directory(self.webview_dir())
        }
    }
}

/// Profile of this process; `None` for the default one.
pub fn current() -> Option<&'static Workdir> {
    CURRENT.get().and_then(|w| w.as_ref())
}

/// Resolve the profile from the command line / environment. Call once at startup.
/// `product` is the app identifier: the name of the product's folder in the directory.
pub fn init(product: &str) -> Result<Option<&'static Workdir>, String> {
    let requested = match parse_args(std::env::args().skip(1))? {
        Some(dir) => Some(dir),
        None => std::env::var(ENV_VAR).ok().filter(|v| !v.trim().is_empty()),
    };
    let workdir = match requested {
        Some(dir) => Some(prepare(Path::new(&dir), product)?),
        None => None,
    };
    Ok(CURRENT.get_or_init(|| workdir).as_ref())
}

/// Give a window built in code the profile's webview storage.
pub fn apply_to_window<'a, R: Runtime, M: Manager<R>>(
    builder: WebviewWindowBuilder<'a, R, M>,
) -> WebviewWindowBuilder<'a, R, M> {
    match current() {
        Some(workdir) => workdir.isolate(builder),
        None => builder,
    }
}

/// Append the profile name to a tray caption.
pub fn caption(text: String) -> String {
    match current() {
        Some(workdir) => format!("{text} [{}]", workdir.name),
        None => text,
    }
}

/// Accepts `--workdir DIR`, `--workdir=DIR` and the single-dash spelling.
fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<String>, String> {
    let mut args = args;
    let mut found = None;
    while let Some(arg) = args.next() {
        let Some(rest) = arg
            .strip_prefix("--")
            .or_else(|| arg.strip_prefix('-'))
            .and_then(|a| a.strip_prefix(FLAG))
        else {
            continue;
        };
        let value = match rest.strip_prefix('=') {
            Some(v) => v.to_string(),
            None if rest.is_empty() => args.next().unwrap_or_default(),
            // `--workdirs`, `--workdir-x`: not ours.
            None => continue,
        };
        if value.trim().is_empty() {
            return Err(format!("--{FLAG} requires a directory"));
        }
        found = Some(value);
    }
    Ok(found)
}

/// The profile in `dir`, with its root created: the webview is pointed at
/// `root/webview` before anything else makes `root`. Relative paths resolve
/// against the current directory. Not `canonicalize`: its `\\?\` paths on
/// Windows break the browser launch.
fn prepare(dir: &Path, product: &str) -> Result<Workdir, String> {
    let path = std::path::absolute(dir)
        .map_err(|e| format!("--{FLAG}: bad path {}: {e}", dir.display()))?;
    // Drops the trailing separator of `vasa/`, so both spellings give one tag.
    let path: PathBuf = path.components().collect();
    let workdir = Workdir::new(path, product);
    std::fs::create_dir_all(&workdir.root)
        .map_err(|e| format!("--{FLAG}: cannot create {}: {e}", workdir.root.display()))?;
    Ok(workdir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Option<String>, String> {
        parse_args(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn no_flag() {
        assert_eq!(parse(&[]), Ok(None));
        assert_eq!(parse(&["--other", "x", "--workdirs=y"]), Ok(None));
    }

    #[test]
    fn all_spellings() {
        for args in [
            &["--workdir", "vasa"][..],
            &["--workdir=vasa"],
            &["-workdir", "vasa"],
            &["-workdir=vasa"],
            &["--first", "--workdir", "vasa", "--last"],
        ] {
            assert_eq!(parse(args), Ok(Some("vasa".into())), "{args:?}");
        }
    }

    #[test]
    fn missing_value() {
        assert!(parse(&["--workdir"]).is_err());
        assert!(parse(&["--workdir="]).is_err());
    }

    #[test]
    fn tag_is_stable_and_dbus_safe() {
        let a = Workdir::new(PathBuf::from("/tmp/vasa"), "net.veydan.space");
        let b = Workdir::new(PathBuf::from("/tmp/vasa"), "net.veydan.space");
        let c = Workdir::new(PathBuf::from("/tmp/petya"), "net.veydan.space");
        assert_eq!(a.tag, b.tag);
        assert_ne!(a.tag, c.tag);
        assert_eq!(a.tag.len(), 8);
        assert!(a.tag.chars().all(|ch| ch.is_ascii_hexdigit()));
        assert_eq!(a.name, "vasa");
    }

    #[test]
    fn two_products_share_a_directory_without_sharing_anything() {
        let space = Workdir::new(PathBuf::from("/tmp/vasa"), "net.veydan.space");
        let notes = Workdir::new(PathBuf::from("/tmp/vasa"), "net.veydan.notes");
        assert_eq!(space.root, PathBuf::from("/tmp/vasa/net.veydan.space"));
        assert_eq!(
            space.webview_dir(),
            PathBuf::from("/tmp/vasa/net.veydan.space/webview")
        );
        assert_ne!(space.root, notes.root);
        assert_ne!(space.tag, notes.tag);
        assert_eq!(space.name, notes.name);
    }

    #[test]
    fn trailing_separator_is_the_same_profile() {
        let base = std::env::temp_dir().join(format!("veydan-workdir-{}", std::process::id()));
        let plain = prepare(&base, "net.veydan.space").unwrap();
        let slashed = prepare(
            Path::new(&format!("{}/", base.display())),
            "net.veydan.space",
        )
        .unwrap();
        assert_eq!(plain.root, slashed.root);
        assert_eq!(plain.tag, slashed.tag);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_root_exists_before_any_window_is_created() {
        let base = std::env::temp_dir().join(format!("veydan-workdir-root-{}", std::process::id()));
        let workdir = prepare(&base, "net.veydan.space").unwrap();
        assert_eq!(workdir.root, base.join("net.veydan.space"));
        assert!(workdir.root.is_dir());
        assert_eq!(workdir.webview_dir().parent(), Some(workdir.root.as_path()));
        assert_eq!(workdir.cache_dir(), workdir.root.join("cache"));
        let _ = std::fs::remove_dir_all(&base);
    }
}
