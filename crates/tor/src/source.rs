// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Where the files of tor are: the binary, its pluggable transports, the
//! GeoIP files and the libraries it loads. Whoever runs tor asks
//! [`resolve`] and does not care how the files got there.
//!
//! On a computer they are the Tor Expert Bundle unpacked into
//! `<data>/tor/bundle/` — downloaded by the app (`install`) or put there by
//! hand. On Android the binary ships in the APK; that is a second function
//! behind [`resolve`], not written yet.

use std::path::{Path, PathBuf};

/// The files of one tor, by their paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    /// The `tor` executable.
    pub tor: PathBuf,
    /// The pluggable transports: `lyrebird` (obfs4, meek, snowflake,
    /// webtunnel) and `conjure-client`, with `pt_config.json`.
    pub pt_dir: PathBuf,
    pub geoip: PathBuf,
    pub geoip6: PathBuf,
    /// The shared libraries tor is linked against (Linux: libevent, OpenSSL;
    /// macOS: libevent). The Linux binary has no RPATH: whoever starts it
    /// puts this directory into `LD_LIBRARY_PATH` ([`Bundle::command`]).
    pub lib_dir: PathBuf,
}

/// Where the app keeps the bundle on a computer.
pub fn bundle_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("tor").join("bundle")
}

/// Serializes changes of `<data>/tor/bundle/` (the swap of an install, a
/// removal) with the starts of tor from it: whoever starts tor holds it
/// while it reads the files. `.lock().await` from async code,
/// `.blocking_lock()` from `spawn_blocking` threads.
pub static INSTALL_DIR_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The name of the executable on this platform.
const TOR_EXE: &str = if cfg!(target_os = "windows") {
    "tor.exe"
} else {
    "tor"
};

impl Bundle {
    /// The layout of the Tor Expert Bundle unpacked into `root`: `tor/` with
    /// the binary, its libraries and `pluggable_transports/`, `data/` with
    /// the GeoIP files.
    pub fn at(root: &Path) -> Self {
        let tor_dir = root.join("tor");
        let data = root.join("data");
        Self {
            tor: tor_dir.join(TOR_EXE),
            pt_dir: tor_dir.join("pluggable_transports"),
            geoip: data.join("geoip"),
            geoip6: data.join("geoip6"),
            lib_dir: tor_dir,
        }
    }

    /// A command that starts this tor: it finds its libraries, and on
    /// Windows it opens no console window.
    pub fn command(&self) -> std::process::Command {
        #[cfg_attr(
            not(any(target_os = "linux", target_os = "windows")),
            allow(unused_mut)
        )]
        let mut command = std::process::Command::new(&self.tor);
        #[cfg(target_os = "linux")]
        {
            let mut paths = vec![self.lib_dir.clone()];
            if let Some(old) = std::env::var_os("LD_LIBRARY_PATH") {
                paths.extend(std::env::split_paths(&old));
            }
            if let Ok(joined) = std::env::join_paths(paths) {
                command.env("LD_LIBRARY_PATH", joined);
            }
        }
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command
    }
}

/// The tor this platform has, if any.
pub fn resolve(app_data_dir: &Path) -> Option<Bundle> {
    if cfg!(mobile) {
        // Android: the binary of the APK, a later stage.
        return None;
    }
    let bundle = Bundle::at(&bundle_dir(app_data_dir));
    bundle.tor.is_file().then_some(bundle)
}

/// The version of tor from the output of `tor --version`: its first line is
/// `Tor version 0.4.9.13 (git-3c575400909efe65).`
pub fn parse_tor_version(output: &str) -> Option<String> {
    let rest = output
        .lines()
        .find_map(|line| line.trim().strip_prefix("Tor version "))?;
    let version = rest.split_whitespace().next()?.trim_end_matches('.');
    (!version.is_empty()).then(|| version.to_string())
}

/// Asks the binary of `bundle` for its version.
pub fn tor_version(bundle: &Bundle) -> Option<String> {
    let output = bundle.command().arg("--version").output().ok()?;
    parse_tor_version(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_is_read_from_the_first_line() {
        let out = "Tor version 0.4.9.13 (git-3c575400909efe65).\nThis build of Tor is covered by the GNU General Public License\n";
        assert_eq!(parse_tor_version(out).as_deref(), Some("0.4.9.13"));
        assert_eq!(
            parse_tor_version("Tor version 0.4.8.1.\n").as_deref(),
            Some("0.4.8.1")
        );
        assert_eq!(parse_tor_version("command not found"), None);
    }

    #[test]
    fn nothing_is_resolved_without_the_binary() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(resolve(dir.path()), None);
        let bundle = Bundle::at(&bundle_dir(dir.path()));
        std::fs::create_dir_all(bundle.tor.parent().unwrap()).unwrap();
        std::fs::write(&bundle.tor, b"").unwrap();
        assert_eq!(resolve(dir.path()), Some(bundle));
    }
}
