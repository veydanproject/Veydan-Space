// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The Tor Expert Bundle on a computer: downloaded from torproject.org or
//! taken from an archive the user chose, checked against the checksums
//! pinned here and unpacked into `<data>/tor/bundle/`.
//!
//! A download resumes where the last one stopped (a marker names the file
//! the partial download belongs to), can be cancelled, and runs one at a
//! time. Nothing is unpacked before the SHA-256 of the whole file matches
//! the pin. The archive is unpacked into a sibling directory and swapped in,
//! so a failed unpack leaves the installed bundle as it was. A bundle with
//! `version.txt` came from a pinned archive; one without it was put there
//! by hand or from an archive of unknown checksum (source "manual").

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::Emitter;
use tokio::sync::Mutex;
use veydan_core::{AppError, CmdResult, Core};

use crate::manager::TorManager;
use crate::source::{self, Bundle};

// ── The pinned bundle ──────────────────────────────────────────────────────

/// The version of the Tor Expert Bundle the app installs: the stable release
/// of Tor Browser it belongs to.
pub const BUNDLE_VERSION: &str = "15.0.24";

/// One archive of the pinned release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pinned {
    /// `<os>-<arch>` as torproject.org names it.
    pub target: &'static str,
    pub file: &'static str,
    pub size: u64,
    /// From `sha256sums-signed-build.txt` of the release.
    pub sha256: &'static str,
}

/// The archives of [`BUNDLE_VERSION`], one per desktop target. Sizes are
/// the `Content-Length` of dist.torproject.org; checksums are those of
/// `https://dist.torproject.org/torbrowser/15.0.24/sha256sums-signed-build.txt`.
pub const PINNED: &[Pinned] = &[
    Pinned {
        target: "linux-x86_64",
        file: "tor-expert-bundle-linux-x86_64-15.0.24.tar.gz",
        size: 32_348_376,
        sha256: "8e012ec6815d7899cb64011582e2dade88e74119c6661068a2a3252de0ccd7f2",
    },
    Pinned {
        target: "windows-x86_64",
        file: "tor-expert-bundle-windows-x86_64-15.0.24.tar.gz",
        size: 22_440_870,
        sha256: "e9dc6ccc93cd6afa507193f4de284d6424233ff5102155cd2c94b259e8a22b65",
    },
    Pinned {
        target: "macos-x86_64",
        file: "tor-expert-bundle-macos-x86_64-15.0.24.tar.gz",
        size: 19_356_806,
        sha256: "8acb0b590f6be34084dcb6d84009ac0c61cc7c5261b7a19d2ab94845aa9bd5b6",
    },
    Pinned {
        target: "macos-aarch64",
        file: "tor-expert-bundle-macos-aarch64-15.0.24.tar.gz",
        size: 18_724_201,
        sha256: "d47afd04b6c751129978390ad003d74ac8b88adfbb939350f0f89999e6570644",
    },
];

/// Where a pinned file is downloaded from, in order: the dist server keeps
/// the current releases, the archive keeps every release for good.
fn urls(file: &str) -> [String; 2] {
    [
        format!("https://dist.torproject.org/torbrowser/{BUNDLE_VERSION}/{file}"),
        format!(
            "https://archive.torproject.org/tor-package-archive/torbrowser/{BUNDLE_VERSION}/{file}"
        ),
    ]
}

/// The target of this build as torproject.org names it; `None` where the
/// Tor Project publishes no bundle the app pins.
pub fn current_target() -> Option<&'static str> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("linux-x86_64")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("windows-x86_64")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("macos-x86_64")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("macos-aarch64")
    } else {
        None
    }
}

pub fn pinned_for(target: &str) -> Option<&'static Pinned> {
    PINNED.iter().find(|p| p.target == target)
}

// ── Paths ──────────────────────────────────────────────────────────────────

/// `<data>/tor/`: the bundle, the partial download and the sibling
/// directories of a swap.
fn tor_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("tor")
}

fn download_tmp(app_data_dir: &Path) -> PathBuf {
    tor_dir(app_data_dir).join("download.tmp.tar.gz")
}

/// Names the file a partial download belongs to.
fn download_marker(app_data_dir: &Path) -> PathBuf {
    tor_dir(app_data_dir).join("download.tmp.file")
}

fn version_file(bundle_dir: &Path) -> PathBuf {
    bundle_dir.join("version.txt")
}

/// The bundle version an installed bundle came with; `None` for a manual one.
fn read_version(bundle_dir: &Path) -> Option<String> {
    std::fs::read_to_string(version_file(bundle_dir))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub use crate::source::INSTALL_DIR_LOCK;

// ── State ──────────────────────────────────────────────────────────────────

#[derive(Clone, Serialize, PartialEq, Debug)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DownloadState {
    Idle,
    Downloading {
        downloaded: u64,
        total: u64,
        percent: u8,
    },
    Done {
        version: String,
    },
    Failed {
        error: String,
    },
}

/// The state of the module: the download and the way to cancel it.
pub struct Installer {
    state: Arc<Mutex<DownloadState>>,
    cancel_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
}

impl Default for Installer {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(DownloadState::Idle)),
            cancel_tx: Arc::new(Mutex::new(None)),
        }
    }
}

#[derive(Clone, Copy, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Unpacked from a pinned archive: `version.txt` names its version.
    Download,
    /// Put in place by hand, or from an archive of unknown checksum.
    Manual,
}

#[derive(Serialize, PartialEq, Debug)]
pub struct TorStatus {
    pub installed: bool,
    pub source: Option<Source>,
    /// The bundle version of `version.txt`.
    pub version: Option<String>,
    /// The version of tor itself, from `tor --version`.
    pub tor_version: Option<String>,
    pub pinned_version: String,
    /// A downloaded bundle older or newer than the pinned one.
    pub update_available: bool,
    /// The tor executable.
    pub path: Option<String>,
    pub install_dir: String,
}

#[derive(Serialize, PartialEq, Debug)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum InstallOutcome {
    /// `version`: the bundle version for a pinned archive, `None` for one of
    /// unknown checksum the user allowed.
    Installed { version: Option<String> },
    /// The checksum is not pinned and the user has not allowed it yet;
    /// nothing was installed.
    UnknownHash { sha256: String },
}

// ── Status ─────────────────────────────────────────────────────────────────

/// What is installed under `app_data_dir`. Blocking: runs `tor --version`.
pub fn status(app_data_dir: &Path) -> TorStatus {
    let dir = source::bundle_dir(app_data_dir);
    let bundle = source::resolve(app_data_dir);
    let version = bundle.as_ref().and_then(|_| read_version(&dir));
    let source = bundle.as_ref().map(|_| {
        if version.is_some() {
            Source::Download
        } else {
            Source::Manual
        }
    });
    TorStatus {
        installed: bundle.is_some(),
        source,
        update_available: version.as_deref().is_some_and(|v| v != BUNDLE_VERSION),
        version,
        tor_version: bundle.as_ref().and_then(source::tor_version),
        pinned_version: BUNDLE_VERSION.to_string(),
        path: bundle.map(|b| b.tor.to_string_lossy().into_owned()),
        install_dir: dir.to_string_lossy().into_owned(),
    }
}

// ── Checksum ───────────────────────────────────────────────────────────────

/// The SHA-256 of a file, in lower-case hex.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Holds a complete download against its pin. A file that does not match is
/// deleted with its marker, so the next attempt starts clean.
fn verify_download(tmp: &Path, marker: &Path, pinned: &Pinned) -> Result<(), String> {
    let actual = sha256_file(tmp).map_err(|e| format!("cannot read the download: {e}"))?;
    if actual != pinned.sha256 {
        let _ = std::fs::remove_file(tmp);
        let _ = std::fs::remove_file(marker);
        return Err(format!(
            "checksum mismatch for {}: expected {}, got {actual}",
            pinned.file, pinned.sha256
        ));
    }
    Ok(())
}

// ── Unpacking ──────────────────────────────────────────────────────────────

/// The top directories of the bundle the app keeps; `debug/` (unstripped
/// copies of the Linux binaries, some 40 MB) is left out.
const KEPT: &[&str] = &["tor", "data", "docs"];

/// The path of an entry made relative and checked: no `..`, no root, no
/// drive. `None` for an entry of a directory the app does not keep.
fn entry_path(raw: &Path) -> Result<Option<PathBuf>, String> {
    let mut clean = PathBuf::new();
    for component in raw.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(format!(
                    "invalid path in the archive (traversal): {}",
                    raw.display()
                ))
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "invalid path in the archive (absolute): {}",
                    raw.display()
                ))
            }
        }
    }
    let top = clean.components().next().and_then(|c| match c {
        Component::Normal(part) => part.to_str(),
        _ => None,
    });
    Ok(match top {
        Some(top) if KEPT.contains(&top) => Some(clean),
        _ => None,
    })
}

/// Unpacks a `.tar.gz` into `dest`, which must not exist yet. Only regular
/// files and directories are taken; any path that would leave `dest` fails
/// the whole unpack.
fn unpack(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| format!("cannot open the archive: {e}"))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(std::io::BufReader::new(file)));
    std::fs::create_dir_all(dest).map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
    let entries = tar.entries().map_err(|e| format!("bad archive: {e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("bad archive: {e}"))?;
        let raw = entry
            .path()
            .map_err(|e| format!("bad path in the archive: {e}"))?
            .into_owned();
        let Some(rel) = entry_path(&raw)? else {
            continue;
        };
        let target = dest.join(&rel);
        match entry.header().entry_type() {
            tar::EntryType::Directory => {
                std::fs::create_dir_all(&target)
                    .map_err(|e| format!("cannot create {}: {e}", target.display()))?;
            }
            tar::EntryType::Regular | tar::EntryType::Continuous => {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
                }
                let mut out = std::fs::File::create(&target)
                    .map_err(|e| format!("cannot write {}: {e}", target.display()))?;
                std::io::copy(&mut entry, &mut out)
                    .map_err(|e| format!("cannot unpack {}: {e}", rel.display()))?;
                out.flush().map_err(|e| e.to_string())?;
            }
            other => {
                return Err(format!(
                    "unexpected entry in the archive ({other:?}): {}",
                    raw.display()
                ))
            }
        }
    }
    Ok(())
}

/// tor and the pluggable transports become executable; the archive gives
/// them mode 0700 and a file unpacked by hand may have lost it.
#[cfg(unix)]
fn make_executable(bundle: &Bundle) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut files = vec![bundle.tor.clone()];
    if let Ok(dir) = std::fs::read_dir(&bundle.pt_dir) {
        for entry in dir.flatten() {
            let path = entry.path();
            let data = matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("json" | "md" | "txt")
            );
            if path.is_file() && !data {
                files.push(path);
            }
        }
    }
    for file in files {
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("cannot make {} executable: {e}", file.display()))?;
    }
    Ok(())
}

/// Installs a verified archive: unpacks it next to the bundle, writes
/// `version.txt` when the archive is a pinned one, and swaps the new
/// directory in. Whatever fails before the swap leaves the installed bundle
/// untouched. Blocking.
pub fn install_archive(
    archive: &Path,
    app_data_dir: &Path,
    version: Option<&str>,
) -> Result<(), String> {
    let _guard = INSTALL_DIR_LOCK.blocking_lock();
    install_archive_locked(archive, app_data_dir, version)
}

/// [`install_archive`] for a caller that holds [`INSTALL_DIR_LOCK`].
fn install_archive_locked(
    archive: &Path,
    app_data_dir: &Path,
    version: Option<&str>,
) -> Result<(), String> {
    let dir = source::bundle_dir(app_data_dir);
    let parent = tor_dir(app_data_dir);
    let fresh = parent.join("bundle.new");
    let old = parent.join("bundle.old");
    let _ = std::fs::remove_dir_all(&fresh);
    let _ = std::fs::remove_dir_all(&old);

    let prepared = (|| {
        unpack(archive, &fresh)?;
        let bundle = Bundle::at(&fresh);
        if !bundle.tor.is_file() {
            return Err(format!(
                "the archive holds no {}: not a Tor Expert Bundle for this platform",
                bundle
                    .tor
                    .strip_prefix(&fresh)
                    .unwrap_or(&bundle.tor)
                    .display()
            ));
        }
        #[cfg(unix)]
        make_executable(&bundle)?;
        if let Some(version) = version {
            std::fs::write(version_file(&fresh), version)
                .map_err(|e| format!("cannot write version.txt: {e}"))?;
        }
        Ok(())
    })();
    if let Err(e) = prepared {
        let _ = std::fs::remove_dir_all(&fresh);
        return Err(e);
    }

    // The swap: the old bundle steps aside, the new one takes its name, and
    // the old one comes back if that fails.
    let had_old = dir.exists();
    if had_old {
        std::fs::rename(&dir, &old).map_err(|e| {
            let _ = std::fs::remove_dir_all(&fresh);
            format!("cannot replace the installed bundle (is tor running?): {e}")
        })?;
    }
    if let Err(e) = std::fs::rename(&fresh, &dir) {
        if had_old {
            let _ = std::fs::rename(&old, &dir);
        }
        let _ = std::fs::remove_dir_all(&fresh);
        return Err(format!("cannot move the new bundle in place: {e}"));
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

/// Deletes the installed bundle and what a swap may have left beside it.
/// A caller that runs tor from the bundle stops it first (`tor_remove` does,
/// through [`TorManager::release_bundle`]). Blocking.
pub fn remove_bundle(app_data_dir: &Path) -> Result<(), String> {
    let _guard = INSTALL_DIR_LOCK.blocking_lock();
    remove_bundle_locked(app_data_dir)
}

fn remove_bundle_locked(app_data_dir: &Path) -> Result<(), String> {
    let parent = tor_dir(app_data_dir);
    for name in ["bundle.new", "bundle.old"] {
        let _ = std::fs::remove_dir_all(parent.join(name));
    }
    let dir = source::bundle_dir(app_data_dir);
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("cannot delete {}: {e}", dir.display())),
    }
}

/// An archive the user chose: a pinned one of this platform is installed
/// with its version; one of unknown checksum only when `allow_unknown`,
/// and then as a manual install. Blocking.
pub fn install_from_archive(
    archive: &Path,
    app_data_dir: &Path,
    allow_unknown: bool,
) -> Result<InstallOutcome, String> {
    match check_archive(archive, allow_unknown)? {
        Checked::Unknown(sha256) => Ok(InstallOutcome::UnknownHash { sha256 }),
        Checked::Install(version) => {
            install_archive(archive, app_data_dir, version)?;
            Ok(InstallOutcome::Installed {
                version: version.map(str::to_string),
            })
        }
    }
}

/// What an archive the user chose turned out to be.
enum Checked {
    /// To be installed, with the bundle version of a pinned archive.
    Install(Option<&'static str>),
    /// Of unknown checksum, and the user has not allowed it.
    Unknown(String),
}

/// Holds the archive against the pins. Blocking: hashes the whole file.
fn check_archive(archive: &Path, allow_unknown: bool) -> Result<Checked, String> {
    let sha256 = sha256_file(archive).map_err(|e| format!("cannot read the archive: {e}"))?;
    Ok(match PINNED.iter().find(|p| p.sha256 == sha256) {
        Some(pinned) => {
            let here = current_target().unwrap_or("unknown");
            if pinned.target != here {
                return Err(format!(
                    "this is the bundle for {}, this computer needs {here}",
                    pinned.target
                ));
            }
            Checked::Install(Some(BUNDLE_VERSION))
        }
        None if allow_unknown => Checked::Install(None),
        None => Checked::Unknown(sha256),
    })
}

/// Changes the bundle with nothing running from it: under
/// [`INSTALL_DIR_LOCK`] the instances of tor nobody uses are stopped first
/// ([`TorManager::release_bundle`]); while consumers hold tor, it is
/// refused with `tor_in_use`. On Windows a running `tor.exe` could not be
/// replaced at all. Then the instance that starts with the app comes back.
async fn with_bundle_released<T: Send + 'static>(
    manager: &TorManager,
    change: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let guard = INSTALL_DIR_LOCK.lock().await;
    manager.release_bundle().await.map_err(|e| e.to_string())?;
    let result = tokio::task::spawn_blocking(move || {
        let _guard = guard;
        change()
    })
    .await
    .map_err(|e| format!("install task failed: {e}"))?;
    manager.start_with_app().await;
    result
}

// ── Download ───────────────────────────────────────────────────────────────

/// A failed download. `Cancelled` is its own variant so cancellation is not
/// told by the text of an error.
#[derive(Debug)]
enum DownloadError {
    Cancelled,
    Other(String),
}

impl From<String> for DownloadError {
    fn from(s: String) -> Self {
        Self::Other(s)
    }
}

/// The end of one attempt at one URL.
enum Attempt {
    Complete,
    /// The server did not answer or the stream broke; the next URL resumes.
    Failed(String),
}

async fn run_download(
    app: tauri::AppHandle,
    manager: TorManager,
    app_data_dir: PathBuf,
    state: Arc<Mutex<DownloadState>>,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<String, DownloadError> {
    let target = current_target()
        .ok_or_else(|| "the Tor Project publishes no bundle for this platform".to_string())?;
    let pinned = pinned_for(target)
        .ok_or_else(|| format!("no pinned bundle for {target}"))?;

    std::fs::create_dir_all(tor_dir(&app_data_dir))
        .map_err(|e| format!("cannot create the tor directory: {e}"))?;
    let tmp = download_tmp(&app_data_dir);
    let marker = download_marker(&app_data_dir);

    // A partial file is resumed only when it belongs to this very file.
    let same_file = std::fs::read_to_string(&marker)
        .map(|v| v.trim() == pinned.file)
        .unwrap_or(false);
    let existing = tmp.metadata().map(|m| m.len()).unwrap_or(0);
    if !same_file || existing > pinned.size {
        std::fs::write(&marker, pinned.file)
            .map_err(|e| format!("cannot write the download marker: {e}"))?;
        std::fs::File::create(&tmp).map_err(|e| format!("cannot create the temp file: {e}"))?;
    }

    // The dist server is slow at times: no limit on the whole transfer, a
    // limit on silence.
    let client = reqwest::Client::builder()
        .user_agent("VeydanSpace/1.0")
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(90))
        .build()
        .map_err(|e| e.to_string())?;

    let mut errors = Vec::new();
    let mut complete = false;
    for url in urls(pinned.file) {
        match fetch(&client, &url, &tmp, pinned.size, &app, &state, &mut cancel_rx).await? {
            Attempt::Complete => {
                complete = true;
                break;
            }
            Attempt::Failed(e) => errors.push(e),
        }
    }
    if !complete {
        return Err(format!("download failed: {}", errors.join("; ")).into());
    }

    {
        let (tmp, marker) = (tmp.clone(), marker.clone());
        tokio::task::spawn_blocking(move || verify_download(&tmp, &marker, pinned))
            .await
            .map_err(|e| format!("checksum task failed: {e}"))??;
    }

    app.emit("tor://extracting", ()).ok();
    {
        // Refused while tor is in use: the verified file stays, and the
        // next `tor_download` installs it without fetching it again.
        let (tmp, app_data_dir) = (tmp.clone(), app_data_dir.clone());
        with_bundle_released(&manager, move || {
            install_archive_locked(&tmp, &app_data_dir, Some(BUNDLE_VERSION))
        })
        .await?;
    }
    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::remove_file(&marker);
    Ok(BUNDLE_VERSION.to_string())
}

/// Brings `tmp` to `total` bytes from `url`, resuming what it holds.
async fn fetch(
    client: &reqwest::Client,
    url: &str,
    tmp: &Path,
    total: u64,
    app: &tauri::AppHandle,
    state: &Mutex<DownloadState>,
    cancel_rx: &mut tokio::sync::oneshot::Receiver<()>,
) -> Result<Attempt, DownloadError> {
    let mut start = tmp.metadata().map(|m| m.len()).unwrap_or(0);
    if start == total {
        return Ok(Attempt::Complete);
    }
    let mut req = client.get(url);
    if start > 0 {
        req = req.header("Range", format!("bytes={start}-"));
    }
    let response = tokio::select! {
        response = req.send() => response,
        _ = &mut *cancel_rx => return Err(DownloadError::Cancelled),
    };
    let response = match response.and_then(|r| r.error_for_status()) {
        Ok(response) => response,
        Err(e) => return Ok(Attempt::Failed(format!("{url}: {e}"))),
    };
    // A server that ignores Range sends the whole file: start over.
    if start > 0 && response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        start = 0;
    }
    let mut file = if start > 0 {
        std::fs::OpenOptions::new().append(true).open(tmp)
    } else {
        std::fs::File::create(tmp)
    }
    .map_err(|e| format!("cannot open the temp file: {e}"))?;

    let mut downloaded = start;
    let mut last_percent = None;
    let mut stream = response.bytes_stream();
    loop {
        tokio::select! {
            chunk = stream.next() => match chunk {
                Some(Ok(data)) => {
                    if downloaded + data.len() as u64 > total {
                        drop(file);
                        let _ = std::fs::remove_file(tmp);
                        return Err(format!("{url}: the server sends more than the pinned {total} bytes").into());
                    }
                    file.write_all(&data).map_err(|e| format!("write error: {e}"))?;
                    downloaded += data.len() as u64;
                    let percent = (downloaded * 100).checked_div(total).map_or(0, |p| p.min(100) as u8);
                    let now = DownloadState::Downloading { downloaded, total, percent };
                    *state.lock().await = now.clone();
                    if last_percent != Some(percent) {
                        last_percent = Some(percent);
                        app.emit("tor://progress", now).ok();
                    }
                }
                Some(Err(e)) => return Ok(Attempt::Failed(format!("{url}: {e}"))),
                None => break,
            },
            _ = &mut *cancel_rx => return Err(DownloadError::Cancelled),
        }
    }
    file.flush().map_err(|e| format!("write error: {e}"))?;
    if downloaded != total {
        return Ok(Attempt::Failed(format!(
            "{url}: the stream ended at {downloaded} of {total} bytes"
        )));
    }
    Ok(Attempt::Complete)
}

// ── Tauri commands ─────────────────────────────────────────────────────────

#[tauri::command]
pub async fn tor_status(core: tauri::State<'_, Core>) -> CmdResult<TorStatus> {
    let app_data_dir = core.app_data_dir.clone();
    tokio::task::spawn_blocking(move || status(&app_data_dir))
        .await
        .map_err(AppError::other)
}

#[tauri::command]
pub async fn tor_download_state(installer: tauri::State<'_, Installer>) -> CmdResult<DownloadState> {
    Ok(installer.state.lock().await.clone())
}

#[tauri::command]
pub async fn tor_download_cancel(installer: tauri::State<'_, Installer>) -> CmdResult<()> {
    if let Some(sender) = installer.cancel_tx.lock().await.take() {
        sender.send(()).ok();
    }
    *installer.state.lock().await = DownloadState::Idle;
    Ok(())
}

/// Starts the download of the pinned bundle and returns at once; the
/// progress comes as `tor://progress`, the end as `tor://done` or
/// `tor://error`.
#[tauri::command]
pub async fn tor_download(
    app: tauri::AppHandle,
    core: tauri::State<'_, Core>,
    installer: tauri::State<'_, Installer>,
    manager: tauri::State<'_, TorManager>,
) -> CmdResult<()> {
    // The slot is claimed under the same lock as the check: the first chunk
    // may take seconds, and a second call must not pass in the meantime.
    {
        let mut current = installer.state.lock().await;
        if matches!(*current, DownloadState::Downloading { .. }) {
            return Err(AppError::other("Download already in progress"));
        }
        *current = DownloadState::Downloading {
            downloaded: 0,
            total: current_target()
                .and_then(pinned_for)
                .map_or(0, |p| p.size),
            percent: 0,
        };
    }

    let app_data_dir = core.app_data_dir.clone();
    let manager = manager.inner().clone();
    let state = Arc::clone(&installer.state);
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
    *installer.cancel_tx.lock().await = Some(cancel_tx);

    tauri::async_runtime::spawn(async move {
        match run_download(app.clone(), manager, app_data_dir, state.clone(), cancel_rx).await {
            Ok(version) => {
                *state.lock().await = DownloadState::Done {
                    version: version.clone(),
                };
                app.emit("tor://done", version).ok();
            }
            Err(DownloadError::Cancelled) => {
                *state.lock().await = DownloadState::Idle;
                app.emit("tor://error", "Download cancelled".to_string()).ok();
            }
            Err(DownloadError::Other(e)) => {
                *state.lock().await = DownloadState::Failed { error: e.clone() };
                app.emit("tor://error", e).ok();
            }
        }
    });
    Ok(())
}

/// Installs a `.tar.gz` of the Tor Expert Bundle the user chose; refused
/// with `tor_in_use` while consumers hold tor.
#[tauri::command]
pub async fn tor_install_from_archive(
    core: tauri::State<'_, Core>,
    installer: tauri::State<'_, Installer>,
    manager: tauri::State<'_, TorManager>,
    path: String,
    allow_unknown: bool,
) -> CmdResult<InstallOutcome> {
    if matches!(
        *installer.state.lock().await,
        DownloadState::Downloading { .. }
    ) {
        return Err(AppError::other("Download in progress"));
    }
    let app_data_dir = core.app_data_dir.clone();
    let archive = PathBuf::from(path);
    let checked = {
        let archive = archive.clone();
        tokio::task::spawn_blocking(move || check_archive(&archive, allow_unknown))
            .await
            .map_err(AppError::other)?
            .map_err(AppError::other)?
    };
    let version = match checked {
        Checked::Unknown(sha256) => return Ok(InstallOutcome::UnknownHash { sha256 }),
        Checked::Install(version) => version,
    };
    with_bundle_released(&manager, move || {
        install_archive_locked(&archive, &app_data_dir, version)
    })
    .await
    .map_err(AppError::other)?;
    Ok(InstallOutcome::Installed {
        version: version.map(str::to_string),
    })
}

/// Removes the bundle; instances nobody uses are stopped first, and while
/// consumers hold tor it is refused with `tor_in_use`.
#[tauri::command]
pub async fn tor_remove(
    core: tauri::State<'_, Core>,
    manager: tauri::State<'_, TorManager>,
) -> CmdResult<()> {
    let app_data_dir = core.app_data_dir.clone();
    with_bundle_released(&manager, move || remove_bundle_locked(&app_data_dir))
        .await
        .map_err(AppError::other)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// A tar.gz of `(path, contents)` pairs; the names go into the headers
    /// byte for byte, so paths the builder of `tar` refuses can be planted.
    fn tar_gz(path: &Path, files: &[(&str, &[u8])]) {
        let gz = flate2::write::GzEncoder::new(
            std::fs::File::create(path).unwrap(),
            flate2::Compression::fast(),
        );
        let mut builder = tar::Builder::new(gz);
        for (name, data) in files {
            let mut header = tar::Header::new_old();
            let bytes = name.as_bytes();
            header.as_old_mut().name[..bytes.len()].copy_from_slice(bytes);
            header.set_size(data.len() as u64);
            header.set_mode(0o600);
            header.set_entry_type(tar::EntryType::Regular);
            header.set_cksum();
            builder.append(&header, *data).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();
    }

    fn tor_name() -> String {
        format!(
            "tor/{}",
            if cfg!(target_os = "windows") {
                "tor.exe"
            } else {
                "tor"
            }
        )
    }

    /// The layout of a real bundle, small.
    fn good_bundle(path: &Path, marker: &[u8]) {
        tar_gz(
            path,
            &[
                (&tor_name(), marker),
                ("tor/pluggable_transports/lyrebird", b"pt"),
                ("tor/pluggable_transports/pt_config.json", b"{}"),
                ("data/geoip", b"geo"),
                ("data/geoip6", b"geo6"),
                ("debug/tor", b"unstripped"),
            ],
        );
    }

    #[test]
    fn every_desktop_target_has_a_pinned_archive() {
        for target in [
            "linux-x86_64",
            "windows-x86_64",
            "macos-x86_64",
            "macos-aarch64",
        ] {
            let pinned = pinned_for(target).unwrap_or_else(|| panic!("{target}"));
            assert_eq!(
                pinned.file,
                format!("tor-expert-bundle-{target}-{BUNDLE_VERSION}.tar.gz")
            );
            assert_eq!(pinned.sha256.len(), 64);
            assert!(pinned.sha256.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
            assert!(pinned.size > 0);
        }
        assert!(current_target().is_none_or(|t| pinned_for(t).is_some()));
    }

    #[test]
    fn a_download_of_another_checksum_is_refused_and_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path().join("download.tmp.tar.gz");
        let marker = dir.path().join("download.tmp.file");
        good_bundle(&tmp, b"tor");
        std::fs::write(&marker, "x").unwrap();
        let err = verify_download(&tmp, &marker, &PINNED[0]).unwrap_err();
        assert!(err.contains("checksum mismatch"), "{err}");
        assert!(!tmp.exists() && !marker.exists());

        // The same file under its own checksum passes.
        good_bundle(&tmp, b"tor");
        let pinned = Pinned {
            sha256: Box::leak(sha256_file(&tmp).unwrap().into_boxed_str()),
            ..PINNED[0]
        };
        verify_download(&tmp, &marker, &pinned).unwrap();
        assert!(tmp.exists());
    }

    #[test]
    fn an_archive_with_paths_out_of_its_directory_is_refused() {
        for bad in ["tor/../../evil", "/etc/evil", "../evil"] {
            let dir = tempfile::tempdir().unwrap();
            let archive = dir.path().join("a.tar.gz");
            tar_gz(&archive, &[(&tor_name(), b"tor"), (bad, b"x")]);
            let data = dir.path().join("data");
            let err = install_archive(&archive, &data, None).unwrap_err();
            assert!(err.contains("invalid path"), "{bad}: {err}");
            assert!(source::resolve(&data).is_none());
            assert!(!data.join("tor/bundle.new").exists());
            assert!(!dir.path().join("evil").exists());
        }
    }

    #[test]
    fn a_good_archive_installs_and_is_resolved() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.tar.gz");
        good_bundle(&archive, b"tor");
        let data = dir.path().join("data");
        install_archive(&archive, &data, Some(BUNDLE_VERSION)).unwrap();

        let bundle = source::resolve(&data).unwrap();
        assert_eq!(bundle, Bundle::at(&data.join("tor/bundle")));
        assert_eq!(std::fs::read(&bundle.tor).unwrap(), b"tor");
        assert!(bundle.geoip.is_file() && bundle.geoip6.is_file());
        assert!(bundle.pt_dir.join("lyrebird").is_file());
        assert!(!data.join("tor/bundle/debug").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&bundle.tor), 0o755);
            assert_eq!(mode(&bundle.pt_dir.join("lyrebird")), 0o755);
            assert_eq!(mode(&bundle.pt_dir.join("pt_config.json")) & 0o111, 0);
        }

        let status = status(&data);
        assert!(status.installed);
        assert_eq!(status.source, Some(Source::Download));
        assert_eq!(status.version.as_deref(), Some(BUNDLE_VERSION));
        assert!(!status.update_available);
        assert_eq!(status.pinned_version, BUNDLE_VERSION);

        remove_bundle(&data).unwrap();
        assert!(source::resolve(&data).is_none());
        assert!(!super::status(&data).installed);
    }

    #[test]
    fn files_put_in_place_by_hand_are_a_manual_install() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = Bundle::at(&source::bundle_dir(dir.path()));
        std::fs::create_dir_all(bundle.tor.parent().unwrap()).unwrap();
        // A script that answers like tor, where a script can stand for it.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::write(
                &bundle.tor,
                "#!/bin/sh\necho 'Tor version 0.4.9.13 (git-3c575400909efe65).'\n",
            )
            .unwrap();
            std::fs::set_permissions(&bundle.tor, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        #[cfg(not(unix))]
        std::fs::write(&bundle.tor, b"").unwrap();

        let status = status(dir.path());
        assert!(status.installed);
        assert_eq!(status.source, Some(Source::Manual));
        assert_eq!(status.version, None);
        assert!(!status.update_available);
        #[cfg(unix)]
        assert_eq!(status.tor_version.as_deref(), Some("0.4.9.13"));
        assert_eq!(
            status.path.as_deref(),
            Some(bundle.tor.to_string_lossy().as_ref())
        );

        // An old download is one to update.
        std::fs::write(source::bundle_dir(dir.path()).join("version.txt"), "15.0.1").unwrap();
        let status = super::status(dir.path());
        assert_eq!(status.source, Some(Source::Download));
        assert!(status.update_available);
    }

    #[test]
    fn a_failed_unpack_keeps_the_installed_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let good = dir.path().join("good.tar.gz");
        good_bundle(&good, b"old tor");
        install_archive(&good, &data, Some(BUNDLE_VERSION)).unwrap();

        // An archive with no tor in it, one that breaks off, one with a bad path.
        let empty = dir.path().join("empty.tar.gz");
        tar_gz(&empty, &[("data/geoip", b"geo")]);
        let cut = dir.path().join("cut.tar.gz");
        good_bundle(&cut, &[7u8; 4096]);
        let bytes = std::fs::read(&cut).unwrap();
        std::fs::write(&cut, &bytes[..bytes.len() / 2]).unwrap();
        let evil = dir.path().join("evil.tar.gz");
        tar_gz(&evil, &[(&tor_name(), b"new tor"), ("../evil", b"x")]);

        for archive in [&empty, &cut, &evil] {
            assert!(install_archive(archive, &data, None).is_err(), "{}", archive.display());
            let bundle = source::resolve(&data).unwrap();
            assert_eq!(std::fs::read(&bundle.tor).unwrap(), b"old tor");
            assert_eq!(read_version(&source::bundle_dir(&data)).as_deref(), Some(BUNDLE_VERSION));
            assert!(!data.join("tor/bundle.new").exists());
        }
    }

    #[test]
    fn an_archive_of_unknown_checksum_needs_consent_and_is_manual() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let archive = dir.path().join("a.tar.gz");
        good_bundle(&archive, b"tor");
        let sha256 = sha256_file(&archive).unwrap();

        assert_eq!(
            install_from_archive(&archive, &data, false).unwrap(),
            InstallOutcome::UnknownHash { sha256 }
        );
        assert!(source::resolve(&data).is_none());

        assert_eq!(
            install_from_archive(&archive, &data, true).unwrap(),
            InstallOutcome::Installed { version: None }
        );
        assert!(source::resolve(&data).is_some());
        assert_eq!(read_version(&source::bundle_dir(&data)), None);
    }

    #[test]
    fn the_shapes_the_ui_reads() {
        assert_eq!(json(&DownloadState::Idle), r#"{"state":"idle"}"#);
        assert_eq!(
            json(&DownloadState::Downloading { downloaded: 1, total: 4, percent: 25 }),
            r#"{"state":"downloading","downloaded":1,"total":4,"percent":25}"#
        );
        assert_eq!(
            json(&InstallOutcome::UnknownHash { sha256: "ab".into() }),
            r#"{"state":"unknown_hash","sha256":"ab"}"#
        );
        assert_eq!(
            json(&InstallOutcome::Installed { version: None }),
            r#"{"state":"installed","version":null}"#
        );
        assert_eq!(json(&Source::Manual), r#""manual""#);
    }

    fn json<T: Serialize>(value: &T) -> String {
        serde_json::to_string(value).unwrap()
    }
}
