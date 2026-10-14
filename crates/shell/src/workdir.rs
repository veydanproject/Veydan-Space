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
    #[cfg(test)]
    fn new(dir: PathBuf, product: &str) -> Self {
        let root = dir.join(product);
        let key = root.clone();
        Self::keyed(dir, root, &key)
    }

    /// `key` names the directory the tag is derived from: the canonical
    /// form of `root` once it exists (`identity_key`), so a symlink, `..`
    /// or another spelling of one directory is one profile: one lock, one
    /// webview store.
    fn keyed(dir: PathBuf, root: PathBuf, key: &Path) -> Self {
        // Windows paths are case-insensitive: one directory, one profile.
        #[cfg(windows)]
        let key = key.to_string_lossy().to_lowercase();
        #[cfg(not(windows))]
        let key = key.to_string_lossy().into_owned();

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
        // older systems fall back to the shared default store (`init` says
        // so in the log).
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
    #[cfg(target_os = "macos")]
    if let Some(workdir) = &workdir {
        warn_on_a_shared_webview_store(workdir);
    }
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
/// against the current directory. `root` is not `canonicalize`d: its `\\?\`
/// paths on Windows break the browser launch. The tag is: it is derived from
/// the canonical root (`identity_key`), so an alias of the directory (a
/// symlink, `a/../b`) cannot open the same data as a second profile.
fn prepare(dir: &Path, product: &str) -> Result<Workdir, String> {
    let path = std::path::absolute(dir)
        .map_err(|e| format!("--{FLAG}: bad path {}: {e}", dir.display()))?;
    // Drops the trailing separator of `vasa/`, so both spellings give one tag.
    let path: PathBuf = path.components().collect();
    let root = path.join(product);
    std::fs::create_dir_all(&root)
        .map_err(|e| format!("--{FLAG}: cannot create {}: {e}", root.display()))?;
    let key = identity_key(&root);
    Ok(Workdir::keyed(path, root, &key))
}

/// The canonical form of an existing `root`, for the tag only. On Windows
/// the verbatim prefix `canonicalize` adds is taken off again (`\\?\C:\x`
/// → `C:\x`, `\\?\UNC\srv\x` → `\\srv\x`), so a directory reached without
/// aliases keeps the tag it had. Falls back to `root` as given.
fn identity_key(root: &Path) -> PathBuf {
    let Ok(canonical) = std::fs::canonicalize(root) else {
        return root.to_path_buf();
    };
    #[cfg(windows)]
    {
        let text = canonical.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    canonical
}

/// Whether a macOS of this `sw_vers -productVersion` gives a `--workdir`
/// profile the shared default webview store: a data store of its own needs
/// macOS 14. `None` when the version cannot be read.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn shares_webview_store(product_version: &str) -> Option<bool> {
    let major: u32 = product_version.trim().split('.').next()?.parse().ok()?;
    Some(major < 14)
}

/// On macOS 13 and older the localStorage and IndexedDB of a profile are
/// those of the default one: said in the log, the profile still starts.
#[cfg(target_os = "macos")]
fn warn_on_a_shared_webview_store(workdir: &Workdir) {
    let version = std::process::Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .unwrap_or_default();
    if shares_webview_store(&version) == Some(true) {
        eprintln!(
            "--{FLAG} {}: macOS {} has no separate webview data store (needs 14); \
             this profile shares localStorage and IndexedDB with the default one",
            workdir.name,
            version.trim()
        );
    }
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

    #[cfg(unix)]
    #[test]
    fn an_alias_of_the_directory_is_the_same_profile() {
        let base = std::env::temp_dir().join(format!("veydan-workdir-alias-{}", std::process::id()));
        let real = base.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = base.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let plain = prepare(&real, "net.veydan.space").unwrap();
        let via_link = prepare(&link, "net.veydan.space").unwrap();
        let dotted = prepare(&base.join("link/../real"), "net.veydan.space").unwrap();
        assert_eq!(plain.tag, via_link.tag);
        assert_eq!(plain.store_id, via_link.store_id);
        assert_eq!(plain.tag, dotted.tag);
        // The paths the webview and the data are given stay as spelled.
        assert_eq!(via_link.root, link.join("net.veydan.space"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_profile_without_aliases_keeps_its_tag() {
        let base = std::env::temp_dir().join(format!("veydan-workdir-tag-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let base = std::fs::canonicalize(&base).unwrap();
        let prepared = prepare(&base, "net.veydan.space").unwrap();
        let before = Workdir::new(base.clone(), "net.veydan.space");
        assert_eq!(prepared.tag, before.tag);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn macos_before_14_shares_the_webview_store() {
        assert_eq!(shares_webview_store("13.6.1\n"), Some(true));
        assert_eq!(shares_webview_store("12.7"), Some(true));
        assert_eq!(shares_webview_store("14.0"), Some(false));
        assert_eq!(shares_webview_store("15.1.1"), Some(false));
        assert_eq!(shares_webview_store(""), None);
    }
}
