// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use crate::browser::launch::Running;
use crate::browser::BrowserState;
use crate::error::{AppError, CmdResult};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::Emitter;
use tokio::sync::Mutex;
use veydan_core::Core;

const MOZLZ4_MAGIC: &[u8] = b"mozLz40\x00";

/// Serializes mutations of the shared Camoufox install dir (omni.ja repack,
/// chrome.css rewrite, policies.json strip, the install of a download, which
/// checks under it that no browser runs) so concurrent profile launches /
/// downloads don't clobber each other's read-modify-write.
/// Lock with `.lock().await` from async code and `.blocking_lock()` from
/// `spawn_blocking` threads.
pub static INSTALL_DIR_LOCK: Mutex<()> = Mutex::const_new(());

// ── The pinned release ─────────────────────────────────────────────────────

/// The release of Camoufox the app installs, as its tag on GitHub. A new
/// Camoufox comes with a release of the app: the tag and the pins below
/// change together.
pub const CAMOUFOX_VERSION: &str = "v152.0.4-beta.30";

/// One archive of the pinned release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pinned {
    /// `<os>.<arch>` as the asset names of Camoufox have it.
    pub target: &'static str,
    pub file: &'static str,
    pub size: u64,
    /// The SHA-256 of the asset. Empty: not pinned yet, and then this target
    /// gets no download at all (never an unchecked one).
    pub sha256: &'static str,
}

/// The archives of [`CAMOUFOX_VERSION`], one per desktop target. Camoufox
/// publishes no checksums or signatures of its own: each value is the
/// SHA-256 and the size of the asset the owner downloaded and checked.
pub const PINNED: &[Pinned] = &[
    Pinned {
        target: "lin.x86_64",
        file: "camoufox-152.0.4-beta.30-lin.x86_64.zip",
        size: 663467670,
        sha256: "5720d45b894ce1770543de024c6f10d514b38be560fa2dc3226b3d8586caf672",
    },
    Pinned {
        target: "lin.arm64",
        file: "camoufox-152.0.4-beta.30-lin.arm64.zip",
        size: 653870625,
        sha256: "60447260af8bebdb0ec3f2aa72f687b879e5598367303de2e3fdbc7a5be8c124",
    },
    Pinned {
        target: "win.x86_64",
        file: "camoufox-152.0.4-beta.30-win.x86_64.zip",
        size: 493141328,
        sha256: "ea52a02fb1cfb1813ef6a326bea03fb2b650c9774143d953a94a27bfc8f10072",
    },
    Pinned {
        target: "mac.x86_64",
        file: "camoufox-152.0.4-beta.30-mac.x86_64.zip",
        size: 319886717,
        sha256: "f12f90a3650478e670064a2f071c13b998fdd1125f79dc9f6cad5a9b9395b4bc",
    },
    Pinned {
        target: "mac.arm64",
        file: "camoufox-152.0.4-beta.30-mac.arm64.zip",
        size: 312665777,
        sha256: "3b43e766574f286a6a63296cf58b660b7a3120952086c869b4df4c9a71604bc3",
    },
];

/// Where a pinned file is downloaded from: the assets of the pinned tag,
/// never "the latest release".
fn download_url(file: &str) -> String {
    format!("https://github.com/daijro/camoufox/releases/download/{CAMOUFOX_VERSION}/{file}")
}

/// The target of this build as the asset names of Camoufox have it.
pub fn current_target() -> Option<&'static str> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("lin.x86_64")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("lin.arm64")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("win.x86_64")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("mac.x86_64")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("mac.arm64")
    } else {
        None
    }
}

pub fn pinned_for(target: &str) -> Option<&'static Pinned> {
    PINNED.iter().find(|p| p.target == target)
}

/// The pinned archive of this computer, or why there is none: a target
/// whose pin is not filled in is not downloadable.
fn pinned_here() -> Result<&'static Pinned, String> {
    let target = current_target().ok_or_else(|| {
        "camoufox_unavailable: Camoufox publishes no build for this platform".to_string()
    })?;
    pinned_for(target)
        .filter(|p| !p.sha256.is_empty() && p.size > 0)
        .ok_or_else(|| {
            format!("camoufox_unavailable: this build of the app has no checked Camoufox archive for {target}")
        })
}

/// Refused while a browser runs: the files of a running Camoufox are not
/// replaced under it. The UI tells it by this prefix.
const IN_USE: &str = "camoufox_in_use: close the running browser profiles first; the checked download is kept and installed on the next try";

/// Writes `data` to a temp file in the same directory and renames it over
/// `path`, so readers (e.g. a browser that is starting up) never observe a
/// partially written file.
fn write_atomic(path: &Path, data: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp.new");
    std::fs::write(&tmp, data).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename to {}: {e}", path.display()))
}

// ── Download state ─────────────────────────────────────────────────────────

#[derive(Clone, Serialize, PartialEq, Debug)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DownloadState {
    Idle,
    Downloading {
        downloaded: u64,
        total: u64,
        percent: u8,
    },
    /// The checked archive is being installed; it cannot be cancelled.
    Extracting,
    Done {
        version: String,
    },
    Failed {
        error: String,
    },
}

pub struct DownloadManager {
    pub state: Arc<Mutex<DownloadState>>,
    cancel_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
}

impl Default for DownloadManager {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(DownloadState::Idle)),
            cancel_tx: Arc::new(Mutex::new(None)),
        }
    }
}

// ── GitHub types ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
}

// ── Binary resolution ───────────────────────────────────────────────────────

/// `<data>/camoufox/`: the installed Camoufox; `camoufox.new` and
/// `camoufox.old` beside it are the two sides of a swap.
fn install_root(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("camoufox")
}

/// Where the files of a Camoufox install are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// The executable a profile is started with.
    pub binary: PathBuf,
    /// The directory of omni.ja, chrome.css, application.ini and
    /// distribution/policies.json: what the app patches.
    pub resources: PathBuf,
}

impl Layout {
    /// The layout of this platform under the root of an install.
    pub fn of(root: &Path) -> Self {
        let os = if cfg!(target_os = "windows") {
            "win"
        } else if cfg!(target_os = "macos") {
            "mac"
        } else {
            "lin"
        };
        Self::for_os(root, os)
    }

    /// Windows and Linux: everything at the root of the archive. macOS: an
    /// app bundle, the executable in `Contents/MacOS`, the rest in
    /// `Contents/Resources` (as Camoufox's own launcher has it).
    /// TODO(owner): check on macOS against a real archive.
    fn for_os(root: &Path, os: &str) -> Self {
        match os {
            "win" => Self {
                binary: root.join("camoufox.exe"),
                resources: root.to_path_buf(),
            },
            "mac" => {
                let contents = root.join("Camoufox.app").join("Contents");
                Self {
                    binary: contents.join("MacOS").join("camoufox"),
                    resources: contents.join("Resources"),
                }
            }
            _ => Self {
                binary: root.join("camoufox"),
                resources: root.to_path_buf(),
            },
        }
    }
}

pub fn local_binary_path(app_data_dir: &Path) -> PathBuf {
    Layout::of(&install_root(app_data_dir)).binary
}

/// The directory of the files to patch for a Camoufox binary: beside it, or
/// `Contents/Resources` for one in `Contents/MacOS` of an app bundle.
pub fn resources_dir(binary: &Path) -> PathBuf {
    let parent = binary.parent().unwrap_or(Path::new(""));
    let contents = parent.parent();
    match (parent.file_name(), contents) {
        (Some(name), Some(contents))
            if name == "MacOS" && contents.file_name().is_some_and(|c| c == "Contents") =>
        {
            contents.join("Resources")
        }
        _ => parent.to_path_buf(),
    }
}

pub fn resolve_binary(app_data_dir: &Path) -> Option<PathBuf> {
    let local = local_binary_path(app_data_dir);
    if local.exists() {
        return Some(local);
    }
    which::which("camoufox").ok()
}

// ── omni.ja patching ────────────────────────────────────────────────────────

/// Search engine config injected into SearchEngineSelector.sys.mjs.
/// Replaces the broken hardcoded `if (true) { return [...] }` block in Camoufox.
/// Engines: DuckDuckGo (default), Google, Brave Search, Startpage.
const SEARCH_ENGINE_BLOCK: &str = r#"if (true) {
      return [
        {"recordType":"engine","identifier":"ddg","id":"ddg-engine","last_modified":1702906502241,"base":{"aliases":["duckduckgo","ddg"],"classification":"general","name":"DuckDuckGo","urls":{"search":{"base":"https://duckduckgo.com/","searchTermParamName":"q"}}},"variants":[{"environment":{"allRegionsAndLocales":true}}]},
        {"recordType":"engine","identifier":"google","id":"google-engine","last_modified":1702906502241,"base":{"aliases":["google"],"classification":"general","name":"Google","urls":{"search":{"base":"https://www.google.com/search","searchTermParamName":"q"}}},"variants":[{"environment":{"allRegionsAndLocales":true}}]},
        {"recordType":"engine","identifier":"brave","id":"brave-engine","last_modified":1702906502241,"base":{"aliases":["brave"],"classification":"general","name":"Brave Search","urls":{"search":{"base":"https://search.brave.com/search","searchTermParamName":"q"}}},"variants":[{"environment":{"allRegionsAndLocales":true}}]},
        {"recordType":"engine","identifier":"startpage","id":"startpage-engine","last_modified":1702906502241,"base":{"aliases":["startpage"],"classification":"general","name":"Startpage","urls":{"search":{"base":"https://www.startpage.com/search","searchTermParamName":"q"}}},"variants":[{"environment":{"allRegionsAndLocales":true}}]},
        {"recordType":"defaultEngines","id":"default-engines","last_modified":1702906502241,"globalDefault":"ddg","specificDefaults":[]},
        {"recordType":"availableLocales","id":"available-locales","last_modified":1702906502241,"locales":[]}
      ];
    }"#;

/// Bump when the set of omni.ja patches changes so already-patched installs get re-patched.
const OMNI_PATCH_VERSION: u32 = 3;

/// Marker file storing "<omni.ja mtime secs>:<OMNI_PATCH_VERSION>" after the last successful patch.
fn patch_marker_path(install_dir: &Path) -> PathBuf {
    install_dir.join(".rb_patch_mtime")
}

/// Reads the marker as (mtime, patch version). A marker holding the mtime
/// alone yields version 0: a Camoufox found on PATH may carry one.
fn read_patch_marker(marker: &Path) -> (u64, u32) {
    let text = std::fs::read_to_string(marker).unwrap_or_default();
    let mut parts = text.trim().split(':');
    let mtime = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let version = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (mtime, version)
}

/// Records the current omni.ja mtime and patch version after a successful patch.
fn write_patch_marker(install_dir: &Path) {
    let mtime = file_mtime_secs(&install_dir.join("omni.ja"));
    std::fs::write(
        patch_marker_path(install_dir),
        format!("{mtime}:{OMNI_PATCH_VERSION}"),
    )
    .ok();
}

fn file_mtime_secs(path: &Path) -> u64 {
    path.metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Reads all zip entries into memory and writes a new zip replacing specified files.
fn patch_zip_file(zip_path: &Path, replacements: &[(&str, &[u8])]) -> Result<(), String> {
    use std::io::Read;
    use zip::write::SimpleFileOptions;
    use zip::ZipArchive;
    use zip::ZipWriter;

    // Read all entries into memory
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    {
        let src = std::fs::File::open(zip_path)
            .map_err(|e| format!("open {}: {e}", zip_path.display()))?;
        let mut archive =
            ZipArchive::new(src).map_err(|e| format!("read zip {}: {e}", zip_path.display()))?;

        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).map_err(|e| format!("entry {i}: {e}"))?;
            let name = entry.name().to_string();
            let mut buf = Vec::new();
            entry
                .read_to_end(&mut buf)
                .map_err(|e| format!("read {name}: {e}"))?;
            entries.push((name, buf));
        }
    }

    let replacement_map: std::collections::HashMap<&str, &[u8]> =
        replacements.iter().copied().collect();

    let tmp_path = zip_path.with_extension("tmp");
    {
        let dst = std::fs::File::create(&tmp_path).map_err(|e| format!("create tmp: {e}"))?;
        let mut writer = ZipWriter::new(dst);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        for (name, content) in &entries {
            writer
                .start_file(name, options)
                .map_err(|e| format!("start_file {name}: {e}"))?;
            if let Some(&new_content) = replacement_map.get(name.as_str()) {
                writer
                    .write_all(new_content)
                    .map_err(|e| format!("write replacement {name}: {e}"))?;
            } else {
                writer
                    .write_all(content)
                    .map_err(|e| format!("write {name}: {e}"))?;
            }
        }
        writer.finish().map_err(|e| format!("zip finish: {e}"))?;
    }

    std::fs::rename(&tmp_path, zip_path).map_err(|e| format!("rename: {e}"))?;
    Ok(())
}

/// Patches SearchEngineSelector.sys.mjs and Utils.sys.mjs inside toolkit/omni.ja.
///
/// SearchEngineSelector: replaces the broken `if (true) { return [...] }` block with
/// a valid engine config (DDG, Google, Brave, Startpage).
///
/// Utils.sys.mjs: patches LOAD_DUMPS getter to always return `true`, so local
/// search config dumps from omni.ja are used WITHOUT requiring a live Remote Settings
/// server URL. This prevents live Mozilla RS syncs from overriding our engine config.
fn patch_toolkit_omni(install_dir: &Path) -> Result<(), String> {
    let omni_path = install_dir.join("omni.ja");
    if !omni_path.exists() {
        return Ok(());
    }

    use std::io::Read;
    use zip::ZipArchive;

    let selector_target = "moz-src/toolkit/components/search/SearchEngineSelector.sys.mjs";
    let utils_target = "modules/services-settings/Utils.sys.mjs";

    let src = std::fs::File::open(&omni_path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(src).map_err(|e| e.to_string())?;

    // Patch SearchEngineSelector.sys.mjs
    let patched_selector = {
        let mut entry = match archive.by_name(selector_target) {
            Ok(e) => e,
            Err(_) => return Ok(()),
        };
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&buf);

        let marker = "if (true) {";
        let Some(start) = text.find(marker) else {
            return Ok(()); // already patched or structure changed
        };

        let mut depth = 0usize;
        let mut end = start;
        for (i, ch) in text[start..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = start + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }

        format!("{}{}{}", &text[..start], SEARCH_ENGINE_BLOCK, &text[end..])
    };

    // Patch Utils.sys.mjs: force LOAD_DUMPS to always return true.
    // This enables local omni.ja dumps without needing a live RS server URL,
    // preventing Mozilla's live search config from overriding our engine config.
    let patched_utils = {
        let mut entry = match archive.by_name(utils_target) {
            Ok(e) => e,
            Err(_) => None.ok_or("Utils.sys.mjs not found")?,
        };
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&buf).into_owned();

        // Force the LOAD_DUMPS getter to always `return true`. Camoufox has shipped two
        // shapes over time:
        //   legacy:  return [ <urls> ].includes(server)
        //   v150+:   return ( AppConstants.REMOTE_SETTINGS_SERVER_URLS.includes(this.SERVER_URL) || ... )
        // Rather than match either expression, replace the whole getter body by brace matching.
        if let Some(getter) = text.find("get LOAD_DUMPS()") {
            if let Some(open) = text[getter..].find('{') {
                let body_start = getter + open;
                let mut depth = 0usize;
                let mut body_end = body_start;
                for (i, ch) in text[body_start..].char_indices() {
                    match ch {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                body_end = body_start + i + 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                format!(
                    "{}{{\n    return true;\n  }}{}",
                    &text[..body_start],
                    &text[body_end..]
                )
            } else {
                text
            }
        } else {
            text // getter not found — structure changed, leave untouched
        }
    };

    drop(archive);

    patch_zip_file(
        &omni_path,
        &[
            (selector_target, patched_selector.as_bytes()),
            (utils_target, patched_utils.as_bytes()),
        ],
    )
}

/// Camoufox pin-addons hack: pins ANY widget to nav-bar before the web-extension check.
/// On Firefox 152 `onAreaNodeRegistered` calls this for every widget of every toolbar,
/// so new-tab-button / alltabs-button / personal-bookmarks end up in nav-bar on each start.
const ADDONS_PIN_BROKEN: &str = "    inPanel = false;\n    this.pinToToolbar(aWidgetId, true);\n    if (!CustomizableUI.isWebExtensionWidget(aWidgetId)) {\n      return;\n    }\n";
const ADDONS_PIN_FIXED: &str = "    if (!CustomizableUI.isWebExtensionWidget(aWidgetId)) {\n      return;\n    }\n    inPanel = false;\n    this.pinToToolbar(aWidgetId, true);\n";

/// Camoufox resizes every browser window to 1280×1040 once it is laid out, over
/// the size and the state Firefox restored from xulstore.json. A window restored
/// maximized stays maximized for the system but small on the screen (Windows),
/// so the maximize button restores it instead. Without the line Firefox keeps
/// what it restored, or sizes a first window to the screen itself.
const DEFAULT_SIZE_BROKEN: &str = "    // Set default size\n    window.resizeTo(1280, 1040);\n";
const DEFAULT_SIZE_FIXED: &str = "";

/// Text fixes of browser/omni.ja: (entry, Camoufox's text, ours).
const BROWSER_TEXT_PATCHES: &[(&str, &str, &str)] = &[
    (
        "chrome/browser/content/browser/browser-addons.js",
        ADDONS_PIN_BROKEN,
        ADDONS_PIN_FIXED,
    ),
    (
        "chrome/browser/content/browser/browser-init.js",
        DEFAULT_SIZE_BROKEN,
        DEFAULT_SIZE_FIXED,
    ),
];

/// Patches browser/omni.ja:
/// - clears search-config-overrides-v2.json to prevent parse errors;
/// - applies `BROWSER_TEXT_PATCHES`.
fn patch_browser_omni(install_dir: &Path) -> Result<(), String> {
    let omni_path = install_dir.join("browser").join("omni.ja");
    if !omni_path.exists() {
        return Ok(());
    }

    use std::io::Read;
    use zip::ZipArchive;

    let src = std::fs::File::open(&omni_path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(src).map_err(|e| e.to_string())?;
    let mut patched_texts: Vec<(&str, String)> = Vec::new();
    for &(target, broken, fixed) in BROWSER_TEXT_PATCHES {
        let Ok(mut entry) = archive.by_name(target) else {
            continue;
        };
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&buf).into_owned();
        // Not found = already patched or structure changed — leave untouched
        if text.contains(broken) {
            patched_texts.push((target, text.replace(broken, fixed)));
        }
    }
    drop(archive);

    let empty_overrides: &[u8] = br#"{"data":[],"timestamp":0}"#;
    let mut replacements: Vec<(&str, &[u8])> = vec![(
        "defaults/settings/main/search-config-overrides-v2.json",
        empty_overrides,
    )];
    for (target, text) in &patched_texts {
        replacements.push((target, text.as_bytes()));
    }
    patch_zip_file(&omni_path, &replacements)
}

/// Patches both omni.ja files. Safe to call even if already patched (mtime guard in ensure_omni_patched).
pub fn patch_omni_ja(install_dir: &Path) -> Result<(), String> {
    patch_toolkit_omni(install_dir)?;
    patch_browser_omni(install_dir)?;
    Ok(())
}

/// Strips Camoufox's `SearchEngines` enterprise policy from distribution/policies.json.
///
/// Camoufox ships a policy that Removes every real engine, Adds a dummy "None" engine
/// (POST to http://127.0.0.1) and forces it as the locked Default. Enterprise policies
/// have the highest priority in Firefox and cannot be changed by the user, so this block
/// overrides EVERYTHING we inject via omni.ja and search.json.mozlz4:
///   - the address bar "searches" by POSTing to 127.0.0.1 → nothing happens;
///   - the engine picker is locked ("managed by your organization") → looks empty;
///   - when our per-profile `defaultEngineId` disagrees with the policy default, Firefox
///     flags search settings corrupt (browser.search.lastSettingsCorruptTime) and reverts
///     to `policy-None`.
///
/// Removing the `SearchEngines` key lets our config engines (DDG/Google/Brave/Startpage)
/// and the per-profile default take effect. Returns true if the file was modified.
pub fn patch_policies_json(install_dir: &Path) -> bool {
    let path = install_dir.join("distribution").join("policies.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return false;
    };
    let Ok(mut doc) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    let Some(policies) = doc.get_mut("policies").and_then(|v| v.as_object_mut()) else {
        return false;
    };
    if policies.remove("SearchEngines").is_none() {
        return false; // already stripped — nothing to do
    }
    match serde_json::to_string_pretty(&doc) {
        Ok(out) => write_atomic(&path, out.as_bytes()).is_ok(),
        Err(_) => false,
    }
}

/// Deletes startupCache and Remote Settings IndexedDB for all profiles.
/// Called after patching omni.ja so Firefox rebuilds JS caches and settings from scratch.
pub fn clear_all_startup_caches(app_data_dir: &Path) {
    let profiles_dir = app_data_dir.join("profiles");
    let Ok(entries) = std::fs::read_dir(&profiles_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let ff_profile = entry.path().join("firefox-profile");

        let startup_cache = ff_profile.join("startupCache");
        if startup_cache.is_dir() {
            std::fs::remove_dir_all(&startup_cache).ok();
        }

        // Clear Remote Settings IndexedDB so stale overrides don't block new engines
        let idb_dir = ff_profile
            .join("storage")
            .join("permanent")
            .join("chrome")
            .join("idb");
        if idb_dir.is_dir() {
            std::fs::remove_dir_all(&idb_dir).ok();
        }
    }
}

/// Checks whether omni.ja has changed since the last patch (by mtime) or the
/// patch set version was bumped. If so, re-patches and clears caches. Idempotent. Callers must hold
/// `INSTALL_DIR_LOCK` — the mtime check + repack is a read-modify-write on the
/// shared install dir and must not run concurrently with another launch.
pub fn ensure_omni_patched(install_dir: &Path, app_data_dir: &Path) {
    // Always neutralize the SearchEngines enterprise policy first — it overrides every
    // engine we inject, and Camoufox re-ships it on each update. Idempotent (no-op once
    // stripped). If it changed here, profile caches must be cleared so the forced
    // `policy-None` default is recomputed from our config.
    let policies_changed = patch_policies_json(install_dir);

    let omni_path = install_dir.join("omni.ja");
    if !omni_path.exists() {
        if policies_changed {
            clear_all_startup_caches(app_data_dir);
        }
        return;
    }

    let current_mtime = file_mtime_secs(&omni_path);
    let (stored_mtime, stored_version) = read_patch_marker(&patch_marker_path(install_dir));

    if current_mtime == stored_mtime && stored_version == OMNI_PATCH_VERSION {
        if policies_changed {
            clear_all_startup_caches(app_data_dir);
        }
        return; // omni.ja unchanged and patch set current — skip re-patching
    }

    if let Err(e) = patch_omni_ja(install_dir) {
        eprintln!("[Veydan Space] omni.ja patch failed: {e}");
        if policies_changed {
            clear_all_startup_caches(app_data_dir);
        }
        return;
    }

    write_patch_marker(install_dir);

    clear_all_startup_caches(app_data_dir);
}

/// Height of the profile color stripe above the tab strip (px).
pub const STRIPE_HEIGHT: u32 = 14;

/// Builds the #TabsToolbar background: gradient stripe with an optional SVG label.
pub fn build_stripe_bg(color_left: &str, color_right: &str, label: &str) -> String {
    let gradient = format!("linear-gradient(to right, {color_left}, {color_right})");
    if label.is_empty() {
        return gradient;
    }
    let safe = label
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 {STRIPE_HEIGHT}'>\
         <text x='50' y='11' text-anchor='middle' fill='white' fill-opacity='0.8' \
         font-size='10' font-family='system-ui,sans-serif'>{safe}</text></svg>"
    );
    let encoded = svg
        .replace('<', "%3C")
        .replace('>', "%3E")
        .replace('#', "%23");
    format!("url(\"data:image/svg+xml,{encoded}\"), {gradient}")
}

/// Patches shared chrome.css in the Camoufox install dir.
/// Applies structural fixes + full UI block + profile-specific color stripe.
/// Idempotent — safe to call on every launch. Callers must hold
/// `INSTALL_DIR_LOCK` (the file is shared between all profiles).
pub fn patch_chrome_css(
    install_dir: &std::path::Path,
    color_left: &str,
    color_right: &str,
    label: &str,
) -> Result<(), String> {
    let css_path = install_dir.join("chrome.css");
    let content = match std::fs::read_to_string(&css_path) {
        Ok(c) => c,
        // No chrome.css shipped — nothing to patch.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("read {}: {e}", css_path.display())),
    };

    // Remove any previous block of ours (idempotent cleanup). The second marker
    // can only sit in a Camoufox found on PATH: a download into the data
    // directory is always patched by this version.
    let mut patched = content.clone();
    if let Some(start) = ["\n/* Veydan Space:", "\n/* Veydan Browser:"]
        .iter()
        .filter_map(|marker| patched.find(marker))
        .min()
    {
        patched.truncate(start);
    }

    let mut patched = patched
        // Show tab close button (hidden by camoufox minimalistic theme)
        .replace(
            ".tab-close-button,\n#TabsToolbar .toolbarbutton-1 {",
            "#TabsToolbar .toolbarbutton-1 {",
        )
        // Exclude new-tab buttons from the toolbarbutton-1 hide rule
        .replace(
            "#TabsToolbar .toolbarbutton-1 {\n  display: none !important;\n}",
            "#TabsToolbar .toolbarbutton-1:not(#new-tab-button):not(#tabs-newtab-button) {\n  display: none !important;\n}",
        )
        // Re-enable pointer events so tabs are clickable and draggable
        .replace(
            "  pointer-events: none !important;",
            "  pointer-events: auto !important;",
        )
        // Prevent tabs from acting as window-drag areas
        .replace(
            "  -moz-window-dragging: inherit !important;",
            "  -moz-window-dragging: no-drag !important;",
        )
        // Fix tabs being stretched to full window width
        .replace(
            "  max-width: 100% !important;",
            "  max-width: 240px !important;",
        );

    let stripe_bg = build_stripe_bg(color_left, color_right, label);
    let stripe_h = STRIPE_HEIGHT;
    let stripe_pad = STRIPE_HEIGHT + 4;

    patched.push_str(&format!(
        r#"
/* Veydan Space: UI overrides */

.browser-titlebar,
#main-menubar {{
  background-color: #282829 !important;
}}

:root {{
  --tab-min-height: 22px !important;
  --tab-max-height: 22px !important;
  --toolbar-field-height: 28px !important;
}}
.tabbrowser-tab,
.tab-background {{
  max-height: unset !important;
}}

.tab-background {{
  border-radius: 4px 4px 0 0 !important;
  margin-inline: 1px !important;
  border: none !important;
  box-shadow: none !important;
}}

.tabbrowser-tab[selected="true"] .tab-background {{
  background-color: #494949 !important;
  outline: none !important;
  box-shadow: none !important;
}}

.tabbrowser-tab:not([selected]) .tab-background {{
  background-color: transparent !important;
  border: none !important;
}}
.tabbrowser-tab:not([selected]):hover .tab-background {{
  background-color: rgba(255,255,255,0.08) !important;
}}

/* Profile color stripe + padding */
#TabsToolbar {{
  background-image: {stripe_bg} !important;
  background-size: 100% {stripe_h}px !important;
  background-position: top !important;
  background-repeat: no-repeat !important;
  padding-block-start: {stripe_pad}px !important;
  padding-block-end: 0 !important;
  border-block-end: none !important;
}}

#navigator-toolbox #nav-bar {{
  min-height: unset !important;
  max-height: unset !important;
  padding-block: 4px !important;
  margin-block-start: 0 !important;
  border-block-start: 1px solid rgba(255,255,255,0.08) !important;
}}

.titlebar-spacer[type="pre-tabs"]  {{ width: 0 !important; }}
.titlebar-spacer[type="post-tabs"] {{ width: 0 !important; }}

.titlebar-buttonbox-container {{
  width: auto !important;
  border-inline-start: 1px solid rgba(255,255,255,0.12) !important;
  padding-inline-start: 8px !important;
  margin-inline-start: 4px !important;
}}

/* New tab "+" button — force visible, override tabs.css attribute-based hide rules */
#new-tab-button,
#tabs-newtab-button {{
  display: -moz-box !important;
  visibility: visible !important;
  color: rgba(255,255,255,0.6) !important;
  width: 26px !important;
  min-width: 26px !important;
  padding: 0 !important;
  margin-inline-start: 2px !important;
  border-radius: 4px !important;
  list-style-image: url(chrome://global/skin/icons/plus.svg) !important;
}}
#new-tab-button:hover,
#tabs-newtab-button:hover {{
  background: rgba(255,255,255,0.1) !important;
  color: rgba(255,255,255,0.9) !important;
}}

/* Bookmarks toolbar */
#PersonalToolbar {{
  display: flex !important;
  visibility: visible !important;
}}

/* Add-to-bookmarks star button in URL bar */
#star-button-box {{
  display: flex !important;
  visibility: visible !important;
}}

/* Override Camoufox dimension locks (set via inline style when window.outerWidth/outerHeight
   is passed in CAMOU_CONFIG_1). Camoufox sets height on both :root and #browser.
   :root gets height:100% (fills GTK window) instead of auto (which would collapse).
   #browser gets flex:1 so it fills remaining space after toolbars. */
html, :root {{ width: auto !important; height: 100% !important; }}
#browser {{ width: auto !important; height: auto !important; flex: 1 1 auto !important; min-height: 0 !important; }}
"#
    ));

    write_atomic(&css_path, patched.as_bytes())
}

// ── Install ─────────────────────────────────────────────────────────────────

/// Beside `version.txt` at the root of an install from a pinned archive: the
/// SHA-256 of that archive. An install without it, or with another one, was
/// not checked by this build.
fn archive_sha_file(root: &Path) -> PathBuf {
    root.join("archive.sha256")
}

fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn download_tmp(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("camoufox_download.tmp.zip")
}

/// Names the file a partial download belongs to.
fn download_marker(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("camoufox_download.tmp.version")
}

/// Unpacks a zip into `dest`. Every path is checked before anything is
/// written: one that would leave `dest` fails the whole unpack. A file the
/// archive marks executable stays executable (the helpers of a macOS app
/// bundle need it).
fn unzip(archive_path: &Path, dest: &Path) -> Result<(), String> {
    let zip_file =
        std::fs::File::open(archive_path).map_err(|e| format!("Cannot open zip: {e}"))?;
    let mut archive = zip::ZipArchive::new(zip_file).map_err(|e| format!("Bad zip: {e}"))?;

    // Security: validate all paths before extracting (prevent Zip Slip)
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("Bad zip entry: {e}"))?;
        let entry_path = entry.enclosed_name().ok_or("Invalid path in ZIP")?;
        if entry_path.is_absolute() {
            return Err("Invalid path in ZIP: absolute path".to_string());
        }
        for component in entry_path.components() {
            if let std::path::Component::ParentDir = component {
                return Err("Invalid path in ZIP: path traversal attempt".to_string());
            }
        }
    }

    std::fs::create_dir_all(dest).map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Bad zip entry: {e}"))?;
        let entry_path = entry
            .enclosed_name()
            .ok_or("Invalid path in ZIP")?
            .to_owned();
        let target = dest.join(&entry_path);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        #[cfg(unix)]
        let executable = entry.unix_mode().is_some_and(|m| m & 0o111 != 0);
        let mut out = std::fs::File::create(&target).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out)
            .map_err(|e| format!("cannot unpack {}: {e}", entry_path.display()))?;
        out.flush().map_err(|e| e.to_string())?;
        #[cfg(unix)]
        if executable {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Installs a checked archive under [`INSTALL_DIR_LOCK`] when no browser
/// runs: a launch takes the same lock to patch the install, so it waits for
/// the swap; a running browser makes it `camoufox_in_use`, and the caller
/// keeps the archive. Blocking.
fn install_when_idle(
    archive: &Path,
    app_data_dir: &Path,
    running: &Running,
    version: &str,
    sha256: &str,
) -> Result<(), String> {
    let _guard = INSTALL_DIR_LOCK.blocking_lock();
    if !running.processes.blocking_lock().is_empty() {
        return Err(IN_USE.to_string());
    }
    install_archive_locked(archive, app_data_dir, version, sha256)
}

/// Unpacks a checked archive next to the install (`camoufox.new`), checks
/// the binary, patches it and writes its markers there, and only then swaps
/// it in. Whatever fails before the swap leaves the installed Camoufox
/// untouched. The caller holds [`INSTALL_DIR_LOCK`]. Blocking.
fn install_archive_locked(
    archive: &Path,
    app_data_dir: &Path,
    version: &str,
    sha256: &str,
) -> Result<(), String> {
    let dir = install_root(app_data_dir);
    let fresh = app_data_dir.join("camoufox.new");
    let old = app_data_dir.join("camoufox.old");
    let _ = std::fs::remove_dir_all(&fresh);
    let _ = std::fs::remove_dir_all(&old);

    let prepared = (|| {
        unzip(archive, &fresh)?;
        let layout = Layout::of(&fresh);
        if !layout.binary.is_file() {
            return Err(format!(
                "the archive holds no {}: not a Camoufox build for this platform",
                layout
                    .binary
                    .strip_prefix(&fresh)
                    .unwrap_or(&layout.binary)
                    .display()
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&layout.binary, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| format!("cannot make {} executable: {e}", layout.binary.display()))?;
        }

        // Propagate patch failures — a silently unpatched omni.ja ships a browser
        // with broken search/UI, which is far worse than a visible download error.
        patch_chrome_css(&layout.resources, "#6366f1", "#6366f1", "")
            .map_err(|e| format!("chrome.css patch failed: {e}"))?;
        patch_omni_ja(&layout.resources).map_err(|e| format!("omni.ja patch failed: {e}"))?;
        patch_policies_json(&layout.resources);
        // Store marker after initial patch so ensure_omni_patched skips on first launch
        write_patch_marker(&layout.resources);

        std::fs::write(fresh.join("version.txt"), version)
            .map_err(|e| format!("cannot write version.txt: {e}"))?;
        std::fs::write(archive_sha_file(&fresh), sha256)
            .map_err(|e| format!("cannot write archive.sha256: {e}"))?;
        Ok(())
    })();
    if let Err(e) = prepared {
        let _ = std::fs::remove_dir_all(&fresh);
        return Err(e);
    }

    // The swap: the old install steps aside, the new one takes its name, and
    // the old one comes back if that fails.
    let had_old = dir.exists();
    if had_old {
        std::fs::rename(&dir, &old).map_err(|e| {
            let _ = std::fs::remove_dir_all(&fresh);
            format!("cannot replace the installed Camoufox (is a browser running?): {e}")
        })?;
    }
    if let Err(e) = std::fs::rename(&fresh, &dir) {
        if had_old {
            let _ = std::fs::rename(&old, &dir);
        }
        let _ = std::fs::remove_dir_all(&fresh);
        return Err(format!("cannot move the new Camoufox in place: {e}"));
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

// ── Tauri commands ──────────────────────────────────────────────────────────

#[derive(Serialize, PartialEq, Debug)]
pub struct CamoufoxStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub camoufox_tag: Option<String>,
    pub path: Option<String>,
    /// Installed by the app from the archive pinned in this build: the tag
    /// and the checksum of the archive match the pins.
    pub verified: bool,
    /// The release this build installs.
    pub pinned_version: String,
    /// This build pins an archive for this computer.
    pub download_available: bool,
    /// Installed but not verified (an older download, one from before the
    /// pins, one found on PATH) while the pinned one can be installed.
    pub update_available: bool,
}

/// What is installed, without asking the binary its version. `pinned` is
/// the archive of this computer, `None` where there is none to download.
fn install_status(app_data_dir: &Path, pinned: Option<&Pinned>) -> CamoufoxStatus {
    let binary = resolve_binary(app_data_dir);
    let root = install_root(app_data_dir);
    let local = binary
        .as_ref()
        .is_some_and(|b| *b == local_binary_path(app_data_dir));
    let camoufox_tag = if local {
        read_trimmed(&root.join("version.txt"))
    } else {
        None
    };
    let verified = local
        && pinned.is_some_and(|p| {
            !p.sha256.is_empty()
                && camoufox_tag.as_deref() == Some(CAMOUFOX_VERSION)
                && read_trimmed(&archive_sha_file(&root)).as_deref() == Some(p.sha256)
        });
    CamoufoxStatus {
        installed: binary.is_some(),
        version: None,
        camoufox_tag,
        update_available: binary.is_some() && !verified && pinned.is_some(),
        path: binary.map(|p| p.to_string_lossy().to_string()),
        verified,
        pinned_version: CAMOUFOX_VERSION.to_string(),
        download_available: pinned.is_some(),
    }
}

#[tauri::command]
pub async fn camoufox_status(core: tauri::State<'_, Core>) -> CmdResult<CamoufoxStatus> {
    let app_data_dir = core.app_data_dir.clone();
    // Blocking: spawns `camoufox --version` and reads version.txt
    tokio::task::spawn_blocking(move || {
        let mut status = install_status(&app_data_dir, pinned_here().ok());
        status.version = status.path.as_ref().and_then(|path| {
            std::process::Command::new(path)
                .arg("--version")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });
        status
    })
    .await
    .map_err(AppError::other)
}

/// The newest release upstream, for the information of the user only: the
/// download installs [`CAMOUFOX_VERSION`], whatever this says.
#[tauri::command]
pub async fn camoufox_latest_version() -> CmdResult<String> {
    let client = reqwest::Client::builder()
        .user_agent("VeydanSpace/1.0")
        .build()
        .map_err(AppError::other)?;

    let release: GhRelease = client
        .get("https://api.github.com/repos/daijro/camoufox/releases/latest")
        .send()
        .await
        .map_err(|e| AppError::other(format!("Network error: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::other(format!("Parse error: {e}")))?;

    Ok(release.tag_name)
}

#[tauri::command]
pub async fn camoufox_download_state(
    browser: tauri::State<'_, BrowserState>,
) -> CmdResult<DownloadState> {
    Ok(browser.download.state.lock().await.clone())
}

/// Stops a download. An install under way is not stopped halfway: the
/// download moves to `Extracting` under the same lock of the state, so a
/// cancel either comes before it and stops the download, or does nothing.
#[tauri::command]
pub async fn camoufox_download_cancel(browser: tauri::State<'_, BrowserState>) -> CmdResult<()> {
    browser.download.cancel().await;
    Ok(())
}

impl DownloadManager {
    async fn cancel(&self) {
        let mut state = self.state.lock().await;
        if matches!(*state, DownloadState::Extracting) {
            return;
        }
        if let Some(sender) = self.cancel_tx.lock().await.take() {
            sender.send(()).ok();
        }
        *state = DownloadState::Idle;
    }
}

/// Starts the download of the pinned archive and returns at once; the
/// progress comes as `camoufox://progress`, the install as
/// `camoufox://extracting`, the end as `camoufox://done` or
/// `camoufox://error`.
#[tauri::command]
pub async fn camoufox_download(
    app: tauri::AppHandle,
    core: tauri::State<'_, Core>,
    browser: tauri::State<'_, BrowserState>,
) -> CmdResult<()> {
    let pinned = pinned_here().map_err(AppError::other)?;
    // Don't start if already downloading or installing. Claim the slot
    // synchronously inside the same lock as the check — the first chunk may
    // take seconds to arrive, and a second rapid invocation must not pass the
    // guard in the meantime (both would write camoufox_download.tmp.zip
    // concurrently).
    {
        let mut current = browser.download.state.lock().await;
        if matches!(
            *current,
            DownloadState::Downloading { .. } | DownloadState::Extracting
        ) {
            return Err(AppError::other("Download already in progress"));
        }
        *current = DownloadState::Downloading {
            downloaded: 0,
            total: pinned.size,
            percent: 0,
        };
    }

    let app_data_dir = core.app_data_dir.clone();
    let running = Arc::clone(&browser.running);
    let dl_state = Arc::clone(&browser.download.state);
    let cancel_arc = Arc::clone(&browser.download.cancel_tx);

    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
    *cancel_arc.lock().await = Some(cancel_tx);

    // Spawn background task — command returns immediately
    tokio::spawn(async move {
        let result = run_download(
            app.clone(),
            app_data_dir,
            running,
            pinned,
            dl_state.clone(),
            cancel_rx,
        )
        .await;

        match result {
            Ok(version) => {
                *dl_state.lock().await = DownloadState::Done {
                    version: version.clone(),
                };
                app.emit("camoufox://done", version).ok();
            }
            Err(DownloadError::Cancelled) => {
                *dl_state.lock().await = DownloadState::Idle;
                app.emit("camoufox://error", "Download cancelled".to_string())
                    .ok();
            }
            Err(DownloadError::Other(e)) => {
                *dl_state.lock().await = DownloadState::Failed { error: e.clone() };
                app.emit("camoufox://error", e).ok();
            }
        }
    });

    Ok(())
}

// ── Core download logic ─────────────────────────────────────────────────────

/// Download failure. `Cancelled` is a first-class variant so cancellation is
/// not detected by string matching on the error message.
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

async fn run_download(
    app: tauri::AppHandle,
    app_data_dir: PathBuf,
    running: Arc<Running>,
    pinned: &'static Pinned,
    dl_state: Arc<Mutex<DownloadState>>,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<String, DownloadError> {
    std::fs::create_dir_all(&app_data_dir)
        .map_err(|e| format!("Cannot create the data directory: {e}"))?;
    let tmp = download_tmp(&app_data_dir);
    let marker = download_marker(&app_data_dir);

    // A partial file is resumed only when it belongs to this very file; a
    // complete one that was checked but not installed (a browser was running)
    // is installed without fetching it again.
    let same_file = std::fs::read_to_string(&marker)
        .map(|v| v.trim() == pinned.file)
        .unwrap_or(false);
    let existing = tmp.metadata().map(|m| m.len()).unwrap_or(0);
    if !same_file || existing > pinned.size {
        std::fs::write(&marker, pinned.file)
            .map_err(|e| format!("Cannot write the download marker: {e}"))?;
        std::fs::File::create(&tmp).map_err(|e| format!("Cannot create the temp file: {e}"))?;
    }

    // No limit on the whole transfer, a limit on silence.
    let client = reqwest::Client::builder()
        .user_agent("VeydanSpace/1.0")
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(90))
        .build()
        .map_err(|e| e.to_string())?;
    let url = download_url(pinned.file);
    fetch(
        &client,
        &url,
        &tmp,
        pinned.size,
        &app,
        &dl_state,
        &mut cancel_rx,
    )
    .await?;

    {
        let (tmp, marker) = (tmp.clone(), marker.clone());
        tokio::task::spawn_blocking(move || {
            veydan_tor::checksum::verify_download(&tmp, &marker, pinned.file, pinned.sha256)
        })
        .await
        .map_err(|e| format!("Checksum task failed: {e}"))??;
    }

    // From here on the install is not cancelled: the state turns to
    // Extracting under its lock, after the last look at the cancel.
    {
        let mut state = dl_state.lock().await;
        if cancel_rx.try_recv().is_ok() {
            return Err(DownloadError::Cancelled);
        }
        *state = DownloadState::Extracting;
    }
    app.emit("camoufox://extracting", ()).ok();

    {
        // Refused while a browser runs: the checked file stays, and the
        // next `camoufox_download` installs it without fetching it again.
        let (tmp, app_data_dir) = (tmp.clone(), app_data_dir.clone());
        tokio::task::spawn_blocking(move || {
            install_when_idle(
                &tmp,
                &app_data_dir,
                &running,
                CAMOUFOX_VERSION,
                pinned.sha256,
            )
        })
        .await
        .map_err(|e| format!("Extraction task failed: {e}"))??;
    }
    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::remove_file(&marker);
    Ok(CAMOUFOX_VERSION.to_string())
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
) -> Result<(), DownloadError> {
    let mut start = tmp.metadata().map(|m| m.len()).unwrap_or(0);
    if start == total {
        return Ok(());
    }
    let mut req = client.get(url);
    if start > 0 {
        req = req.header("Range", format!("bytes={start}-"));
    }
    let response = tokio::select! {
        response = req.send() => response,
        _ = &mut *cancel_rx => return Err(DownloadError::Cancelled),
    };
    let response = response
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Download failed: {e}"))?;
    // A server that ignores Range sends the whole file: restart instead of appending.
    if start > 0 && response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        start = 0;
    }
    let mut file = if start > 0 {
        std::fs::OpenOptions::new().append(true).open(tmp)
    } else {
        std::fs::File::create(tmp)
    }
    .map_err(|e| format!("Cannot open temp file: {e}"))?;

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
                        return Err(format!("The server sends more than the pinned {total} bytes").into());
                    }
                    file.write_all(&data).map_err(|e| format!("Write error: {e}"))?;
                    downloaded += data.len() as u64;
                    let percent = (downloaded * 100).checked_div(total).map_or(0, |p| p.min(100) as u8);
                    let now = DownloadState::Downloading { downloaded, total, percent };
                    *state.lock().await = now.clone();
                    if last_percent != Some(percent) {
                        last_percent = Some(percent);
                        app.emit("camoufox://progress", now).ok();
                    }
                }
                Some(Err(e)) => return Err(format!("Stream error: {e}").into()),
                None => break,
            },
            _ = &mut *cancel_rx => return Err(DownloadError::Cancelled),
        }
    }
    file.flush().map_err(|e| format!("Write error: {e}"))?;
    if downloaded != total {
        return Err(format!("The download ended at {downloaded} of {total} bytes").into());
    }
    Ok(())
}

/// Reads a mozlz4 file and returns the decompressed bytes.
fn read_mozlz4(path: &Path) -> Result<Vec<u8>, String> {
    let raw = std::fs::read(path).map_err(|e| e.to_string())?;
    if !raw.starts_with(MOZLZ4_MAGIC) {
        return Err("invalid mozlz4 magic".into());
    }
    let size_bytes: [u8; 4] = raw[8..12].try_into().map_err(|_| "truncated header")?;
    let orig_size = u32::from_le_bytes(size_bytes) as usize;
    lz4_flex::decompress(&raw[12..], orig_size).map_err(|e| e.to_string())
}

/// Writes bytes as a mozlz4 file.
fn write_mozlz4(path: &Path, data: &[u8]) -> Result<(), String> {
    let compressed = lz4_flex::compress(data);
    let mut out = Vec::with_capacity(12 + compressed.len());
    out.extend_from_slice(MOZLZ4_MAGIC);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&compressed);
    std::fs::write(path, out).map_err(|e| e.to_string())
}

/// The disclaimer text Firefox mixes into the search-settings verification hash.
/// `$appName` is substituted with the running app's name (e.g. "Camoufox").
/// Must match SearchUtils.getVerificationHash in toolkit/omni.ja byte-for-byte.
const SEARCH_HASH_DISCLAIMER: &str = "By modifying this file, I agree that I am doing so \
only within $appName itself, using official, user-driven search \
engine selection processes, and in a way which does not circumvent \
user consent. I acknowledge that any attempt to change this file \
from outside of $appName is a malicious act, and will be responded \
to accordingly.";

/// Reads the application display name from `application.ini` (e.g. "Camoufox").
/// This is what `Services.appinfo.name` returns and what the search verification
/// hash is salted with. Falls back to "Camoufox" if the file can't be read.
pub fn read_app_name(install_dir: &Path) -> String {
    std::fs::read_to_string(install_dir.join("application.ini"))
        .ok()
        .and_then(|ini| {
            ini.lines()
                .find_map(|l| l.strip_prefix("Name=").map(|v| v.trim().to_string()))
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Camoufox".to_string())
}

/// Computes Firefox's search-settings verification hash for `name`:
/// `base64( SHA256( <profileDirName> + name + disclaimer.replace("$appName", app_name) ) )`.
/// Firefox rejects a `defaultEngineId` whose hash doesn't match this and reverts to the
/// config default — so we must write the exact hash, not omit it.
fn search_verification_hash(firefox_profile_dir: &Path, name: &str, app_name: &str) -> String {
    use base64::Engine;
    use sha2::{Digest, Sha256};

    let profile_filename = firefox_profile_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let disclaimer = SEARCH_HASH_DISCLAIMER.replace("$appName", app_name);
    let salt = format!("{profile_filename}{name}{disclaimer}");
    let digest = Sha256::digest(salt.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(digest)
}

/// The config engines we inject via omni.ja (id, display name), in `search.json.mozlz4`
/// shape. Must stay in sync with `SEARCH_ENGINE_BLOCK` and `search_engine_display_name`.
fn config_engines_json() -> serde_json::Value {
    serde_json::json!([
        {"id": "ddg",       "_name": "DuckDuckGo",   "_isConfigEngine": true, "_metaData": {}},
        {"id": "google",    "_name": "Google",       "_isConfigEngine": true, "_metaData": {}},
        {"id": "brave",     "_name": "Brave Search", "_isConfigEngine": true, "_metaData": {}},
        {"id": "startpage", "_name": "Startpage",    "_isConfigEngine": true, "_metaData": {}}
    ])
}

/// Sets `defaultEngineId` (+ private) in `search.json.mozlz4` for the given Firefox profile,
/// together with the matching verification hash. Without a valid hash Firefox treats the
/// default as external tampering, discards it, and falls back to the config default — which
/// is why an app-selected engine never applied and in-browser choices reset on relaunch.
///
/// The `engines` array must be non-empty: Firefox's `SearchSettings.get()` throws
/// "no engine in the file" on an empty list, declares the whole file corrupt, backs it up
/// to `.bak`, and resets — silently dropping our default. So a freshly created file is
/// seeded with the injected config engines.
pub fn write_search_engine_to_profile(
    firefox_profile_dir: &Path,
    engine_id: &str,
    app_name: &str,
) -> Result<(), String> {
    let search_json = firefox_profile_dir.join("search.json.mozlz4");

    let mut doc: serde_json::Value = if search_json.exists() {
        let bytes = read_mozlz4(&search_json)?;
        serde_json::from_slice(&bytes).map_err(|e| e.to_string())?
    } else {
        serde_json::json!({
            "version": 13,
            "metaData": {
                "locale": "en-US",
                "region": "unknown",
                "channel": "default",
                "experiment": "",
                "distroID": "",
                "appDefaultEngineId": "ddg",
                "useSavedOrder": false
            },
            "engines": config_engines_json()
        })
    };

    // Never leave `engines` empty — Firefox would treat the file as corrupt and discard
    // our default. Seed the config engines if the loaded file has none.
    let engines_empty = doc
        .get("engines")
        .and_then(|v| v.as_array())
        .map(|a| a.is_empty())
        .unwrap_or(true);
    if engines_empty {
        doc["engines"] = config_engines_json();
    }

    let hash = search_verification_hash(firefox_profile_dir, engine_id, app_name);

    let meta = doc
        .get_mut("metaData")
        .and_then(|v| v.as_object_mut())
        .ok_or("missing metaData")?;

    meta.insert(
        "defaultEngineId".into(),
        serde_json::Value::String(engine_id.into()),
    );
    meta.insert(
        "privateDefaultEngineId".into(),
        serde_json::Value::String(engine_id.into()),
    );
    meta.insert(
        "defaultEngineIdHash".into(),
        serde_json::Value::String(hash.clone()),
    );
    meta.insert(
        "privateDefaultEngineIdHash".into(),
        serde_json::Value::String(hash),
    );

    let json_bytes = serde_json::to_vec(&doc).map_err(|e| e.to_string())?;
    write_mozlz4(&search_json, &json_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    const INIT_JS: &str = "chrome/browser/content/browser/browser-init.js";

    fn install_dir_with_browser_init(name: &str, text: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("veydan-omni-{name}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("browser")).unwrap();
        let file = std::fs::File::create(dir.join("browser").join("omni.ja")).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(INIT_JS, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
        zip.finish().unwrap();
        dir
    }

    fn read_browser_init(dir: &Path) -> String {
        let file = std::fs::File::open(dir.join("browser").join("omni.ja")).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut text = String::new();
        archive
            .by_name(INIT_JS)
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        text
    }

    #[test]
    fn the_window_keeps_the_size_it_is_restored_with() {
        let before =
            format!("    }}\n\n{DEFAULT_SIZE_BROKEN}\n    // Hijack the outer window size.\n");
        let dir = install_dir_with_browser_init("size", &before);
        patch_browser_omni(&dir).unwrap();
        let after = read_browser_init(&dir);
        assert!(!after.contains("window.resizeTo(1280, 1040)"));
        assert!(after.contains("// Hijack the outer window size."));
        // A second patch finds nothing to change
        patch_browser_omni(&dir).unwrap();
        assert_eq!(read_browser_init(&dir), after);
        std::fs::remove_dir_all(&dir).ok();
    }

    // ── The pinned download and its install ──────────────────────────────

    /// A directory of its own under the temp dir, removed when dropped.
    struct TmpDir(PathBuf);

    impl TmpDir {
        fn new(name: &str) -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static N: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "veydan-camoufox-{name}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A path of the layout of this platform, relative to the root of an
    /// archive, with the separators of a zip.
    fn rel(path: PathBuf) -> String {
        path.to_string_lossy().replace('\\', "/")
    }

    fn binary_name() -> String {
        rel(Layout::of(Path::new("")).binary)
    }

    fn policies_name() -> String {
        let resources = rel(Layout::of(Path::new("")).resources);
        if resources.is_empty() {
            "distribution/policies.json".to_string()
        } else {
            format!("{resources}/distribution/policies.json")
        }
    }

    /// A zip of `(name, contents)` pairs, names written as they are.
    fn zip_of(path: &Path, files: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, data) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }

    /// The layout of a real build, small: the binary and the policy that
    /// the install strips.
    fn good_archive(path: &Path, binary: &[u8]) {
        zip_of(
            path,
            &[
                (&binary_name(), binary),
                (
                    &policies_name(),
                    br#"{"policies":{"SearchEngines":{"Remove":["Google"]},"DisableTelemetry":true}}"#,
                ),
                ("application.ini", b"[App]\nName=Camoufox\n"),
            ],
        )
    }

    /// The pin of a file made here.
    fn pin_of(file: &Path) -> Pinned {
        Pinned {
            sha256: Box::leak(
                veydan_tor::checksum::sha256_file(file)
                    .unwrap()
                    .into_boxed_str(),
            ),
            size: std::fs::metadata(file).unwrap().len(),
            ..PINNED[0]
        }
    }

    fn installed_binary(data: &Path) -> Vec<u8> {
        std::fs::read(local_binary_path(data)).unwrap()
    }

    #[test]
    fn every_desktop_target_has_a_pin_or_an_honest_todo() {
        let tag = CAMOUFOX_VERSION.strip_prefix('v').unwrap();
        for target in [
            "lin.x86_64",
            "lin.arm64",
            "win.x86_64",
            "mac.x86_64",
            "mac.arm64",
        ] {
            let pinned = pinned_for(target).unwrap_or_else(|| panic!("{target}"));
            assert_eq!(pinned.file, format!("camoufox-{tag}-{target}.zip"));
            if pinned.sha256.is_empty() {
                // Not filled in yet: no download for it, not an unchecked one.
                assert_eq!(pinned.size, 0, "{target}");
            } else {
                assert_eq!(pinned.sha256.len(), 64, "{target}");
                assert!(pinned
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
                assert!(pinned.size > 0, "{target}");
            }
        }
        assert_eq!(PINNED.len(), 5);
        assert!(current_target().is_none_or(|t| pinned_for(t).is_some()));
        // Where the pin of this computer is empty, the download is refused
        // with a clear error, before anything is fetched.
        match current_target().and_then(pinned_for) {
            Some(p) if !p.sha256.is_empty() => assert_eq!(pinned_here().unwrap(), p),
            _ => assert!(pinned_here()
                .unwrap_err()
                .starts_with("camoufox_unavailable")),
        }
        assert_eq!(
            download_url("camoufox-x.zip"),
            format!("https://github.com/daijro/camoufox/releases/download/{CAMOUFOX_VERSION}/camoufox-x.zip")
        );
    }

    #[test]
    fn a_download_of_another_checksum_is_refused_and_deleted() {
        let dir = TmpDir::new("checksum");
        let tmp = download_tmp(&dir.0);
        let marker = download_marker(&dir.0);
        good_archive(&tmp, b"camoufox");
        let pinned = pin_of(&tmp);

        // Another file under the same name, and a pin left empty.
        for expected in ["0".repeat(64).as_str(), ""] {
            good_archive(&tmp, b"something else");
            std::fs::write(&marker, pinned.file).unwrap();
            let err = veydan_tor::checksum::verify_download(&tmp, &marker, pinned.file, expected)
                .unwrap_err();
            assert!(err.contains("checksum mismatch"), "{err}");
            assert!(!tmp.exists() && !marker.exists());
        }

        good_archive(&tmp, b"camoufox");
        veydan_tor::checksum::verify_download(&tmp, &marker, pinned.file, pinned.sha256).unwrap();
        assert!(tmp.exists());
    }

    #[test]
    fn a_zip_with_paths_out_of_its_directory_is_refused() {
        for bad in ["../evil", "a/../../evil", "camoufox/../../evil"] {
            let dir = TmpDir::new("slip");
            let archive = dir.0.join("a.zip");
            zip_of(&archive, &[(&binary_name(), b"camoufox"), (bad, b"x")]);
            let data = dir.0.join("data");
            let err =
                install_when_idle(&archive, &data, &Running::default(), "v1", "ab").unwrap_err();
            assert!(err.contains("Invalid path in ZIP"), "{bad}: {err}");
            assert!(!local_binary_path(&data).exists());
            assert!(!data.join("camoufox.new").exists());
            assert!(!dir.0.join("evil").exists() && !data.join("evil").exists());
        }

        // A name from the root is taken as relative: it lands in the install.
        let dir = TmpDir::new("slip-root");
        let archive = dir.0.join("a.zip");
        zip_of(
            &archive,
            &[
                (&binary_name(), b"camoufox"),
                ("/veydan-zip-slip/evil", b"x"),
            ],
        );
        let data = dir.0.join("data");
        install_when_idle(&archive, &data, &Running::default(), "v1", "ab").unwrap();
        assert!(!Path::new("/veydan-zip-slip").exists());
        assert!(install_root(&data).join("veydan-zip-slip/evil").is_file());
    }

    #[test]
    fn a_good_archive_is_patched_in_staging_and_installed_verified() {
        let dir = TmpDir::new("good");
        let archive = dir.0.join("a.zip");
        good_archive(&archive, b"camoufox");
        let pinned = pin_of(&archive);
        let data = dir.0.join("data");
        install_when_idle(
            &archive,
            &data,
            &Running::default(),
            CAMOUFOX_VERSION,
            pinned.sha256,
        )
        .unwrap();

        let layout = Layout::of(&install_root(&data));
        assert_eq!(installed_binary(&data), b"camoufox");
        let policies =
            std::fs::read_to_string(layout.resources.join("distribution/policies.json")).unwrap();
        assert!(!policies.contains("SearchEngines") && policies.contains("DisableTelemetry"));
        assert!(patch_marker_path(&layout.resources).is_file());
        assert!(!data.join("camoufox.new").exists() && !data.join("camoufox.old").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&layout.binary)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o755);
        }

        let status = install_status(&data, Some(&pinned));
        assert!(status.installed && status.verified && status.download_available);
        assert!(!status.update_available);
        assert_eq!(status.camoufox_tag.as_deref(), Some(CAMOUFOX_VERSION));
        assert_eq!(status.pinned_version, CAMOUFOX_VERSION);
        assert_eq!(
            status.path.as_deref(),
            Some(local_binary_path(&data).to_string_lossy().as_ref())
        );

        // A second install replaces the first one whole.
        good_archive(&archive, b"camoufox 2");
        let pinned = pin_of(&archive);
        install_when_idle(
            &archive,
            &data,
            &Running::default(),
            CAMOUFOX_VERSION,
            pinned.sha256,
        )
        .unwrap();
        assert_eq!(installed_binary(&data), b"camoufox 2");
        assert!(install_status(&data, Some(&pinned)).verified);
    }

    #[test]
    fn a_failed_unpack_keeps_the_installed_camoufox() {
        let dir = TmpDir::new("failed");
        let data = dir.0.join("data");
        let good = dir.0.join("good.zip");
        good_archive(&good, b"old camoufox");
        let pinned = pin_of(&good);
        install_when_idle(
            &good,
            &data,
            &Running::default(),
            CAMOUFOX_VERSION,
            pinned.sha256,
        )
        .unwrap();

        // An archive with no binary, one that breaks off, one with a bad path.
        let empty = dir.0.join("empty.zip");
        zip_of(&empty, &[("application.ini", b"[App]\n")]);
        let cut = dir.0.join("cut.zip");
        good_archive(&cut, &[7u8; 4096]);
        let bytes = std::fs::read(&cut).unwrap();
        std::fs::write(&cut, &bytes[..bytes.len() / 2]).unwrap();
        let evil = dir.0.join("evil.zip");
        zip_of(
            &evil,
            &[(&binary_name(), b"new camoufox"), ("../evil", b"x")],
        );

        for archive in [&empty, &cut, &evil] {
            assert!(
                install_when_idle(archive, &data, &Running::default(), "v0", "00").is_err(),
                "{}",
                archive.display()
            );
            assert_eq!(installed_binary(&data), b"old camoufox");
            assert!(install_status(&data, Some(&pinned)).verified);
            assert!(!data.join("camoufox.new").exists());
        }
    }

    #[test]
    fn the_install_is_refused_while_a_browser_runs() {
        let dir = TmpDir::new("running");
        let data = dir.0.join("data");
        let old = dir.0.join("old.zip");
        good_archive(&old, b"old camoufox");
        let pinned = pin_of(&old);
        install_when_idle(
            &old,
            &data,
            &Running::default(),
            CAMOUFOX_VERSION,
            pinned.sha256,
        )
        .unwrap();

        let archive = dir.0.join("new.zip");
        good_archive(&archive, b"new camoufox");
        let running = Running::default();
        running.processes.blocking_lock().insert(
            "p1".into(),
            crate::browser::launch::BrowserProcess::for_test(1),
        );
        let err = install_when_idle(&archive, &data, &running, CAMOUFOX_VERSION, "ab").unwrap_err();
        assert!(err.starts_with("camoufox_in_use"), "{err}");
        assert_eq!(installed_binary(&data), b"old camoufox");
        // The archive stays for the next try.
        assert!(archive.is_file());

        running.processes.blocking_lock().clear();
        let pinned = pin_of(&archive);
        install_when_idle(&archive, &data, &running, CAMOUFOX_VERSION, pinned.sha256).unwrap();
        assert_eq!(installed_binary(&data), b"new camoufox");
    }

    #[test]
    fn an_install_not_from_the_pin_is_not_verified() {
        let dir = TmpDir::new("manual");
        let data = dir.0.join("data");
        let binary = local_binary_path(&data);
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, b"camoufox").unwrap();
        let archive = dir.0.join("a.zip");
        good_archive(&archive, b"camoufox");
        let pinned = pin_of(&archive);

        // Put in place by hand: no version, no checksum.
        let status = install_status(&data, Some(&pinned));
        assert!(status.installed && !status.verified && status.update_available);
        assert_eq!(status.camoufox_tag, None);

        // A download from before the pins: a tag, no checksum.
        let root = install_root(&data);
        std::fs::write(root.join("version.txt"), CAMOUFOX_VERSION).unwrap();
        assert!(install_status(&data, Some(&pinned)).update_available);

        // An older pinned download.
        std::fs::write(root.join("version.txt"), "v135.0.1-beta.24").unwrap();
        std::fs::write(archive_sha_file(&root), pinned.sha256).unwrap();
        let status = install_status(&data, Some(&pinned));
        assert!(!status.verified && status.update_available);

        // The pinned one.
        std::fs::write(root.join("version.txt"), CAMOUFOX_VERSION).unwrap();
        assert!(install_status(&data, Some(&pinned)).verified);

        // Without a pin for this computer nothing is verified, and there is
        // nothing to update to.
        let status = install_status(&data, None);
        assert!(!status.verified && !status.update_available && !status.download_available);
    }

    #[test]
    fn the_files_to_patch_of_a_macos_bundle_are_in_its_resources() {
        let root = Path::new("/data/camoufox");
        let mac = Layout::for_os(root, "mac");
        assert_eq!(
            mac.binary,
            root.join("Camoufox.app/Contents/MacOS/camoufox")
        );
        assert_eq!(mac.resources, root.join("Camoufox.app/Contents/Resources"));
        assert_eq!(resources_dir(&mac.binary), mac.resources);

        for os in ["lin", "win"] {
            let layout = Layout::for_os(root, os);
            assert_eq!(layout.resources, root);
            assert_eq!(resources_dir(&layout.binary), root);
        }
        // A Camoufox found on PATH: the files beside it.
        assert_eq!(
            resources_dir(Path::new("/usr/lib/camoufox/camoufox")),
            Path::new("/usr/lib/camoufox")
        );
    }

    #[tokio::test]
    async fn a_cancel_does_not_stop_an_install_under_way() {
        let manager = DownloadManager::default();
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        *manager.cancel_tx.lock().await = Some(tx);
        *manager.state.lock().await = DownloadState::Extracting;
        manager.cancel().await;
        assert_eq!(*manager.state.lock().await, DownloadState::Extracting);
        assert!(rx.try_recv().is_err());
        assert!(manager.cancel_tx.lock().await.is_some());

        *manager.state.lock().await = DownloadState::Downloading {
            downloaded: 1,
            total: 2,
            percent: 50,
        };
        manager.cancel().await;
        assert_eq!(*manager.state.lock().await, DownloadState::Idle);
        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn the_shapes_the_ui_reads() {
        assert_eq!(
            serde_json::to_string(&DownloadState::Extracting).unwrap(),
            r#"{"state":"extracting"}"#
        );
        let dir = TmpDir::new("shape");
        let status = serde_json::to_value(install_status(&dir.0, None)).unwrap();
        for key in [
            "verified",
            "pinned_version",
            "download_available",
            "update_available",
        ] {
            assert!(status.get(key).is_some(), "{key}");
        }
    }
}
