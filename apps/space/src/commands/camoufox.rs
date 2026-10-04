// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

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
/// chrome.css rewrite, policies.json strip, download extraction) so concurrent
/// profile launches / downloads don't clobber each other's read-modify-write.
/// Lock with `.lock().await` from async code and `.blocking_lock()` from
/// `spawn_blocking` threads.
pub static INSTALL_DIR_LOCK: Mutex<()> = Mutex::const_new(());

/// Only release assets under this path are downloaded.
const CAMOUFOX_RELEASE_PREFIX: &str = "https://github.com/daijro/camoufox/releases/download/";

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
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

// ── Binary resolution ───────────────────────────────────────────────────────

pub fn local_binary_path(app_data_dir: &Path) -> PathBuf {
    let dir = app_data_dir.join("camoufox");
    if cfg!(target_os = "windows") {
        dir.join("camoufox.exe")
    } else {
        dir.join("camoufox")
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
const OMNI_PATCH_VERSION: u32 = 2;

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

/// Patches browser/omni.ja:
/// - clears search-config-overrides-v2.json to prevent parse errors;
/// - moves the web-extension check before Camoufox's pin-to-toolbar hack in browser-addons.js.
fn patch_browser_omni(install_dir: &Path) -> Result<(), String> {
    let omni_path = install_dir.join("browser").join("omni.ja");
    if !omni_path.exists() {
        return Ok(());
    }

    use std::io::Read;
    use zip::ZipArchive;

    let addons_target = "chrome/browser/content/browser/browser-addons.js";
    let src = std::fs::File::open(&omni_path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(src).map_err(|e| e.to_string())?;
    let patched_addons = match archive.by_name(addons_target) {
        Ok(mut entry) => {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).map_err(|e| e.to_string())?;
            let text = String::from_utf8_lossy(&buf).into_owned();
            // Not found = already patched or structure changed — leave untouched
            text.contains(ADDONS_PIN_BROKEN)
                .then(|| text.replace(ADDONS_PIN_BROKEN, ADDONS_PIN_FIXED))
        }
        Err(_) => None,
    };
    drop(archive);

    let empty_overrides: &[u8] = br#"{"data":[],"timestamp":0}"#;
    let mut replacements: Vec<(&str, &[u8])> = vec![(
        "defaults/settings/main/search-config-overrides-v2.json",
        empty_overrides,
    )];
    if let Some(addons) = &patched_addons {
        replacements.push((addons_target, addons.as_bytes()));
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

// ── Tauri commands ──────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct CamoufoxStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub camoufox_tag: Option<String>,
    pub path: Option<String>,
}

#[tauri::command]
pub async fn camoufox_status(core: tauri::State<'_, Core>) -> CmdResult<CamoufoxStatus> {
    let app_data_dir = core.app_data_dir.clone();
    // Blocking: spawns `camoufox --version` and reads version.txt
    tokio::task::spawn_blocking(move || match resolve_binary(&app_data_dir) {
        Some(path) => {
            let version = std::process::Command::new(&path)
                .arg("--version")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());

            let camoufox_tag =
                std::fs::read_to_string(app_data_dir.join("camoufox").join("version.txt"))
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());

            CamoufoxStatus {
                installed: true,
                version,
                camoufox_tag,
                path: Some(path.to_string_lossy().to_string()),
            }
        }
        None => CamoufoxStatus {
            installed: false,
            version: None,
            camoufox_tag: None,
            path: None,
        },
    })
    .await
    .map_err(AppError::other)
}

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

#[tauri::command]
pub async fn camoufox_download_cancel(browser: tauri::State<'_, BrowserState>) -> CmdResult<()> {
    let mut tx = browser.download.cancel_tx.lock().await;
    if let Some(sender) = tx.take() {
        sender.send(()).ok();
    }
    *browser.download.state.lock().await = DownloadState::Idle;
    Ok(())
}

#[tauri::command]
pub async fn camoufox_download(
    app: tauri::AppHandle,
    core: tauri::State<'_, Core>,
    browser: tauri::State<'_, BrowserState>,
) -> CmdResult<()> {
    // Don't start if already downloading. Claim the slot synchronously inside
    // the same lock as the check — the first chunk may take seconds to arrive,
    // and a second rapid invocation must not pass the guard in the meantime
    // (both would write camoufox_download.tmp.zip concurrently).
    {
        let mut current = browser.download.state.lock().await;
        if matches!(*current, DownloadState::Downloading { .. }) {
            return Err(AppError::other("Download already in progress"));
        }
        *current = DownloadState::Downloading {
            downloaded: 0,
            total: 0,
            percent: 0,
        };
    }

    let app_data_dir = core.app_data_dir.clone();
    let dl_state = Arc::clone(&browser.download.state);
    let cancel_arc = Arc::clone(&browser.download.cancel_tx);

    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
    *cancel_arc.lock().await = Some(cancel_tx);

    // Spawn background task — command returns immediately
    tokio::spawn(async move {
        let result = run_download(app.clone(), app_data_dir, dl_state.clone(), cancel_rx).await;

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
    dl_state: Arc<Mutex<DownloadState>>,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<String, DownloadError> {
    let client = reqwest::Client::builder()
        .user_agent("VeydanSpace/1.0")
        .build()
        .map_err(|e| e.to_string())?;

    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x86_64"
    };
    let os = if cfg!(target_os = "windows") {
        "win"
    } else if cfg!(target_os = "macos") {
        "mac"
    } else {
        "lin"
    };
    let target_suffix = format!("{os}.{arch}.zip");

    // Fetch up to 20 releases and find the latest one that has an asset for this platform.
    // Some releases may skip certain platforms (e.g. Windows builds missing in recent releases).
    let releases: Vec<GhRelease> = client
        .get("https://api.github.com/repos/daijro/camoufox/releases?per_page=20")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch release info: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Failed to parse release info: {e}"))?;

    let (release, asset) = releases
        .into_iter()
        .find_map(|r| {
            let a = r.assets.iter().find(|a| a.name.ends_with(&target_suffix))?;
            Some((r.tag_name.clone(), (a.browser_download_url.clone(), a.size)))
        })
        .ok_or_else(|| format!("No release found with asset for '{target_suffix}'"))?;

    let version = release;
    let (download_url, total_size) = asset;

    // The API response is untrusted input: only accept assets hosted under
    // the upstream release path.
    if !download_url.starts_with(CAMOUFOX_RELEASE_PREFIX) {
        return Err(format!("Unexpected asset URL: {download_url}").into());
    }

    let tmp_path = app_data_dir.join("camoufox_download.tmp.zip");
    let version_path = app_data_dir.join("camoufox_download.tmp.version");

    // Resume only a partial file that belongs to this exact version.
    let same_version = std::fs::read_to_string(&version_path)
        .map(|v| v.trim() == version)
        .unwrap_or(false);
    let existing_size = tmp_path.metadata().map(|m| m.len()).unwrap_or(0);
    let mut start_from = if same_version && existing_size > 0 && existing_size < total_size {
        existing_size
    } else {
        0
    };

    let mut req = client.get(&download_url);
    if start_from > 0 {
        req = req.header("Range", format!("bytes={start_from}-"));
    }

    let response = req
        .send()
        .await
        .map_err(|e| format!("Download failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Download failed: {e}"))?;

    // A server that ignores Range sends the whole file: restart instead of appending.
    if start_from > 0 && response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        start_from = 0;
    }

    // Open file: append if resuming, overwrite otherwise
    let mut file = if start_from > 0 {
        std::fs::OpenOptions::new().append(true).open(&tmp_path)
    } else {
        std::fs::write(&version_path, &version)
            .map_err(|e| format!("Cannot write version marker: {e}"))?;
        std::fs::File::create(&tmp_path)
    }
    .map_err(|e| format!("Cannot open temp file: {e}"))?;

    let mut downloaded = start_from;
    let mut stream = response.bytes_stream();

    loop {
        tokio::select! {
            chunk = stream.next() => {
                match chunk {
                    Some(Ok(data)) => {
                        file.write_all(&data).map_err(|e| format!("Write error: {e}"))?;
                        downloaded += data.len() as u64;

                        let percent = (downloaded * 100)
                            .checked_div(total_size)
                            .map_or(0, |p| p.min(100) as u8);

                        let state_val = DownloadState::Downloading { downloaded, total: total_size, percent };
                        *dl_state.lock().await = state_val.clone();
                        app.emit("camoufox://progress", state_val).ok();
                    }
                    Some(Err(e)) => return Err(format!("Stream error: {e}").into()),
                    None => break,
                }
            }
            _ = &mut cancel_rx => {
                return Err(DownloadError::Cancelled);
            }
        }
    }

    drop(file);

    // Size mismatch means a truncated or substituted archive: drop it so the
    // next attempt starts clean.
    if downloaded != total_size {
        let _ = std::fs::remove_file(&tmp_path);
        let _ = std::fs::remove_file(&version_path);
        return Err(
            format!("Downloaded size {downloaded} does not match asset size {total_size}").into(),
        );
    }

    // Extract zip + patch — heavy synchronous IO, moved off the async worker
    app.emit("camoufox://extracting", ()).ok();

    {
        let tmp_path = tmp_path.clone();
        let app_data_dir = app_data_dir.clone();
        let version = version.clone();
        tokio::task::spawn_blocking(move || {
            extract_and_install(&tmp_path, &app_data_dir, &version)
        })
        .await
        .map_err(|e| format!("Extraction task failed: {e}"))??;
    }

    Ok(version)
}

/// Synchronous tail of the download: zip validation + extraction, permissions,
/// omni.ja/chrome.css/policies patching and marker files. Runs inside
/// `spawn_blocking` — everything here is blocking `std::fs`/zip IO.
fn extract_and_install(tmp_path: &Path, app_data_dir: &Path, version: &str) -> Result<(), String> {
    // Runs on a spawn_blocking thread, so blocking_lock is safe here. Keeps a
    // concurrently launching profile from patching/reading the install dir
    // while it is being replaced.
    let _guard = INSTALL_DIR_LOCK.blocking_lock();

    let dest_dir = app_data_dir.join("camoufox");
    std::fs::create_dir_all(&dest_dir).map_err(|e| e.to_string())?;

    let zip_file = std::fs::File::open(tmp_path).map_err(|e| format!("Cannot open zip: {e}"))?;
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

    // Extract all entries
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Bad zip entry: {e}"))?;
        let entry_path = entry
            .enclosed_name()
            .ok_or("Invalid path in ZIP")?
            .to_owned();
        let dest = dest_dir.join(&entry_path);
        if entry.is_dir() {
            std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut out = std::fs::File::create(&dest).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        }
    }

    std::fs::remove_file(tmp_path).ok();
    std::fs::remove_file(tmp_path.with_extension("version")).ok();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let binary = local_binary_path(app_data_dir);
        if binary.exists() {
            let mut perms = std::fs::metadata(&binary)
                .map_err(|e| e.to_string())?
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&binary, perms).map_err(|e| e.to_string())?;
        }
    }

    let binary = local_binary_path(app_data_dir);
    if !binary.exists() {
        return Err(format!(
            "Binary not found after extraction at {}",
            binary.display()
        ));
    }

    // Propagate patch failures — a silently unpatched omni.ja ships a browser
    // with broken search/UI, which is far worse than a visible download error.
    patch_chrome_css(&dest_dir, "#6366f1", "#6366f1", "")
        .map_err(|e| format!("chrome.css patch failed: {e}"))?;
    patch_omni_ja(&dest_dir).map_err(|e| format!("omni.ja patch failed: {e}"))?;
    patch_policies_json(&dest_dir);

    // Store marker after initial patch so ensure_omni_patched skips on first launch
    write_patch_marker(&dest_dir);

    std::fs::write(dest_dir.join("version.txt"), version).ok();

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
