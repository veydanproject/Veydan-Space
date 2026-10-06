// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use crate::browser::{launch as browser_launch, userjs, BrowserState};
use crate::models::{Profile, Proxy};
use std::path::PathBuf;
use std::sync::Arc;
use veydan_core::Core;

pub struct LaunchResult {
    pub pid: u32,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The size of a browser window that has none saved (CSS pixels).
const DEFAULT_WINDOW_SIZE: (i64, i64) = (1280, 760);
/// The smallest saved size of a browser window that is kept (CSS pixels).
const MIN_WINDOW_SIZE: (i64, i64) = (640, 400);

/// Gives the browser window a size to open with in xulstore.json, unless a
/// usable one is saved there; the saved state (maximized or not) and position
/// stay. Camoufox does not size a new window itself, and its resize at start
/// (removed from omni.ja, see `DEFAULT_SIZE_BROKEN`) left sizes like 516×200
/// behind as the size a maximized window returns to.
fn ensure_window_size(firefox_profile_dir: &std::path::Path) {
    let path = firefox_profile_dir.join("xulstore.json");
    let mut json = match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
            Ok(json) if json.is_object() => json,
            _ => return,
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(_) => return,
    };
    // Indexing a value that is neither an object nor null panics
    let doc = &mut json["chrome://browser/content/browser.xhtml"];
    if !doc.is_object() {
        *doc = serde_json::json!({});
    }
    let win = &mut doc["main-window"];
    if !win.is_object() {
        *win = serde_json::json!({});
    }
    let size = |key: &str| win.get(key)?.as_str()?.parse::<i64>().ok();
    if let (Some(w), Some(h)) = (size("width"), size("height")) {
        if w >= MIN_WINDOW_SIZE.0 && h >= MIN_WINDOW_SIZE.1 {
            return;
        }
    }
    win["width"] = DEFAULT_WINDOW_SIZE.0.to_string().into();
    win["height"] = DEFAULT_WINDOW_SIZE.1.to_string().into();
    std::fs::write(&path, json.to_string()).ok();
}

const UI_STATE_PREF: &str = "user_pref(\"browser.uiCustomization.state\", \"";
const TABSTRIP_WIDGETS: [&str; 2] = ["new-tab-button", "alltabs-button"];
const BOOKMARKS_WIDGET: &str = "personal-bookmarks";
/// CustomizableUI id of the extension's browser_action: add-on id with non [a-z0-9_-] replaced by `_`.
const NOTES_WIDGET: &str = "notes_veydan_net-browser-action";

/// Decodes a JS string literal body ("\\" and "\"" escapes) from prefs.js.
fn unescape_pref(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn escape_pref(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Moves tab-strip and bookmarks widgets back from nav-bar if a previous
/// Camoufox build pinned them there. Returns true if placements were changed.
fn repair_placements(state: &mut serde_json::Value) -> bool {
    let Some(placements) = state.get_mut("placements").and_then(|p| p.as_object_mut()) else {
        return false;
    };
    let Some(nav_bar) = placements.get_mut("nav-bar").and_then(|v| v.as_array_mut()) else {
        return false;
    };
    let misplaced = |id: &serde_json::Value| {
        id.as_str()
            .is_some_and(|s| TABSTRIP_WIDGETS.contains(&s) || s == BOOKMARKS_WIDGET)
    };
    if !nav_bar.iter().any(misplaced) {
        return false;
    }
    nav_bar.retain(|id| !misplaced(id));
    placements.insert(
        "TabsToolbar".into(),
        serde_json::json!(["tabbrowser-tabs", "new-tab-button", "alltabs-button"]),
    );
    placements.insert(
        "PersonalToolbar".into(),
        serde_json::json!([BOOKMARKS_WIDGET]),
    );
    true
}

/// Pins the notes extension button to nav-bar (before the extensions menu) so it is
/// visible right away instead of hidden in the unified extensions panel.
/// Returns true if placements were changed.
fn pin_notes_widget(state: &mut serde_json::Value) -> bool {
    let Some(nav_bar) = state
        .get_mut("placements")
        .and_then(|p| p.get_mut("nav-bar"))
        .and_then(|v| v.as_array_mut())
    else {
        return false;
    };
    let widget = serde_json::Value::String(NOTES_WIDGET.to_string());
    if nav_bar.contains(&widget) {
        return false;
    }
    match nav_bar
        .iter()
        .position(|id| id.as_str() == Some("unified-extensions-button"))
    {
        Some(pos) => nav_bar.insert(pos, widget),
        None => nav_bar.push(widget),
    }
    true
}

/// Fixes browser.uiCustomization.state in prefs.js before launch (browser is not running).
fn repair_ui_customization_state(firefox_profile_dir: &std::path::Path) {
    let prefs_path = firefox_profile_dir.join("prefs.js");
    let Ok(content) = std::fs::read_to_string(&prefs_path) else {
        return;
    };
    let Some(start) = content.find(UI_STATE_PREF) else {
        return;
    };
    let value_start = start + UI_STATE_PREF.len();
    let rest = &content[value_start..];
    let line = rest
        .split('\n')
        .next()
        .unwrap_or(rest)
        .trim_end_matches('\r');
    let Some(raw) = line.strip_suffix("\");") else {
        return;
    };
    let value_len = raw.len();
    let Ok(mut state) = serde_json::from_str::<serde_json::Value>(&unescape_pref(raw)) else {
        return;
    };
    let changed = repair_placements(&mut state);
    if !(pin_notes_widget(&mut state) || changed) {
        return;
    }
    let fixed = format!(
        "{}{}{}",
        &content[..value_start],
        escape_pref(&state.to_string()),
        &content[value_start + value_len..]
    );
    std::fs::write(&prefs_path, fixed).ok();
}

/// Core launch orchestrator: proxy setup → user.js → binary resolution → spawn.
/// Does not touch the DB — callers handle DB reads and status updates.
pub async fn launch_profile(
    profile: &Profile,
    proxy: Option<&Proxy>,
    core: &Core,
    browser: &BrowserState,
    app_handle: tauri::AppHandle,
) -> Result<LaunchResult, String> {
    let profile_path = PathBuf::from(&profile.profile_path);
    let firefox_profile_dir = profile_path.join("firefox-profile");
    std::fs::create_dir_all(&firefox_profile_dir).map_err(err)?;
    repair_ui_customization_state(&firefox_profile_dir);

    let tor = tauri::Manager::state::<veydan_tor::TorManager>(&app_handle)
        .inner()
        .clone();
    let (effective_proxy, local_proxy_stop) =
        setup_proxy(proxy, &core.db, &tor, &profile.id).await?;

    // The effective proxy is the local relay whatever stands behind it, so
    // that the relay leads to Tor is told apart here.
    let via_tor = proxy.is_some_and(crate::proxy::tor::is_tor);
    let user_js_content = userjs::generate_with(profile, effective_proxy.as_ref(), via_tor);
    std::fs::write(firefox_profile_dir.join("user.js"), user_js_content).map_err(err)?;

    // Notes capture extension, carries this profile's id and the app UI language
    let locale = veydan_shell::app_locale(&core.db).await;
    crate::capture::extension::install_extension(&firefox_profile_dir, &profile.id, &locale)
        .unwrap_or_else(|e| eprintln!("install_extension failed: {e}"));

    if profile.browser_type == "camoufox" {
        let app_name = crate::commands::camoufox::resolve_binary(&core.app_data_dir)
            .and_then(|bin| bin.parent().map(crate::commands::camoufox::read_app_name))
            .unwrap_or_else(|| "Camoufox".to_string());
        crate::commands::camoufox::write_search_engine_to_profile(
            &firefox_profile_dir,
            &profile.default_search_engine,
            &app_name,
        )
        .unwrap_or_else(|e| {
            eprintln!("write_search_engine_to_profile failed: {e}");
        });
    }

    let (binary_path, camoufox_config) = resolve_binary_and_config(
        profile,
        &firefox_profile_dir,
        effective_proxy.as_ref(),
        core,
    )
    .await?;

    let pid = browser_launch::launch(
        profile.id.clone(),
        profile.locale.clone(),
        profile_path,
        binary_path,
        profile.timezone.clone(),
        camoufox_config,
        local_proxy_stop,
        Arc::clone(&browser.running),
        core.db.clone(),
        app_handle,
    )
    .await
    .map_err(err)?;

    Ok(LaunchResult { pid })
}

/// Wraps any proxy type in a local HTTP proxy on 127.0.0.1.
/// Returns the effective proxy (pointing to 127.0.0.1) and a stop channel.
async fn setup_proxy(
    proxy: Option<&Proxy>,
    db: &sqlx::SqlitePool,
    tor: &veydan_tor::TorManager,
    profile_id: &str,
) -> Result<(Option<Proxy>, Option<tokio::sync::oneshot::Sender<()>>), String> {
    match proxy {
        // Tor: the instance of the row's exit countries is started and
        // connected first. Tor that is not installed or does not connect
        // fails the launch; nothing below runs without the lease.
        Some(p) if crate::proxy::tor::is_tor(p) => {
            let lease = crate::proxy::tor::lease(tor, p, profile_id).await?;
            let upstream = crate::proxy::local::Upstream::Tor { lease };
            match crate::proxy::local::spawn(upstream).await {
                Ok((local_port, stop_tx)) => {
                    let mut local_p = p.clone();
                    local_p.proxy_type = "http".to_string();
                    local_p.host = "127.0.0.1".to_string();
                    local_p.port = local_port as i64;
                    local_p.username = None;
                    local_p.password = None;
                    Ok((Some(local_p), Some(stop_tx)))
                }
                Err(e) => Err(format!("Failed to start local proxy: {e}")),
            }
        }
        Some(p) if matches!(p.proxy_type.as_str(), "http" | "https") => {
            let upstream = crate::proxy::local::Upstream::Http {
                host: p.host.clone(),
                port: p.port as u16,
                username: p.username.clone().unwrap_or_default(),
                password: p.password.clone().unwrap_or_default(),
            };
            match crate::proxy::local::spawn(upstream).await {
                Ok((local_port, stop_tx)) => {
                    let mut local_p = p.clone();
                    local_p.host = "127.0.0.1".to_string();
                    local_p.port = local_port as i64;
                    local_p.username = None;
                    local_p.password = None;
                    Ok((Some(local_p), Some(stop_tx)))
                }
                Err(e) => Err(format!("Failed to start local proxy: {e}")),
            }
        }
        Some(p) if p.proxy_type == "socks5" => {
            let upstream = crate::proxy::local::Upstream::Socks5 {
                host: p.host.clone(),
                port: p.port as u16,
                username: p.username.clone().filter(|u| !u.is_empty()),
                password: p.password.clone().filter(|pw| !pw.is_empty()),
            };
            match crate::proxy::local::spawn(upstream).await {
                Ok((local_port, stop_tx)) => {
                    let mut local_p = p.clone();
                    local_p.proxy_type = "http".to_string();
                    local_p.host = "127.0.0.1".to_string();
                    local_p.port = local_port as i64;
                    local_p.username = None;
                    local_p.password = None;
                    Ok((Some(local_p), Some(stop_tx)))
                }
                Err(e) => Err(format!("Failed to start local proxy: {e}")),
            }
        }
        Some(p) if p.proxy_type == "ssh" => {
            let auth = if let Some(key) = &p.private_key {
                if !key.is_empty() {
                    crate::proxy::ssh::SshAuth::PrivateKey(key.clone())
                } else {
                    crate::proxy::ssh::SshAuth::Password(p.password.clone().unwrap_or_default())
                }
            } else {
                crate::proxy::ssh::SshAuth::Password(p.password.clone().unwrap_or_default())
            };
            let username = p.username.clone().unwrap_or_default();
            match crate::proxy::ssh::SshSession::connect(
                &p.host,
                p.port as u16,
                &username,
                auth,
                p.server_fingerprint.clone(),
            )
            .await
            {
                Ok(r) => {
                    crate::commands::proxies::pin_ssh_fingerprint(db, p, &r).await;
                    let upstream = crate::proxy::local::Upstream::Ssh { session: r.session };
                    match crate::proxy::local::spawn(upstream).await {
                        Ok((local_port, stop_tx)) => {
                            let mut local_p = p.clone();
                            local_p.proxy_type = "http".to_string();
                            local_p.host = "127.0.0.1".to_string();
                            local_p.port = local_port as i64;
                            local_p.username = None;
                            local_p.password = None;
                            Ok((Some(local_p), Some(stop_tx)))
                        }
                        Err(e) => Err(format!("Failed to start SSH local proxy: {e}")),
                    }
                }
                Err(e) => Err(format!("SSH connection failed: {e}")),
            }
        }
        None => Ok((None, None)),
        // Never hand an unhandled type through to `userjs::apply_proxy_prefs`:
        // its own fallback emits no proxy prefs at all, so the browser would go
        // out directly while the profile still claims to use a proxy.
        // `proxies::resolve_required` rejects these before launch.
        Some(p) => Err(format!(
            "Proxy '{}' has type '{}', which the browser launcher cannot route \
             through — refusing to launch with a direct connection.",
            p.name, p.proxy_type
        )),
    }
}

async fn resolve_binary_and_config(
    profile: &Profile,
    firefox_profile_dir: &std::path::Path,
    effective_proxy: Option<&Proxy>,
    core: &Core,
) -> Result<(PathBuf, Option<serde_json::Value>), String> {
    match profile.browser_type.as_str() {
        "camoufox" => {
            let bin = crate::commands::camoufox::resolve_binary(&core.app_data_dir)
                .ok_or("Camoufox not found. Please download it in Settings.")?;

            if let Some(install_dir) = bin.parent() {
                // Serialize shared-install-dir mutations across concurrent
                // launches: two launches must not both repack omni.ja (and
                // clear startup caches) while a starting browser reads it.
                let _guard = crate::commands::camoufox::INSTALL_DIR_LOCK.lock().await;
                crate::commands::camoufox::ensure_omni_patched(install_dir, &core.app_data_dir);
            }

            let (wcolor, wname) = if let Some(wid) = &profile.workspace_id {
                let row = sqlx::query_as::<_, (String, String)>(
                    "SELECT color, name FROM workspaces WHERE id = ?",
                )
                .bind(wid)
                .fetch_optional(&core.db)
                .await
                .map_err(err)?;
                row.unwrap_or_else(|| ("#6366f1".to_string(), String::new()))
            } else {
                ("#6366f1".to_string(), String::new())
            };

            let tags: Vec<String> = serde_json::from_str(&profile.tags).unwrap_or_default();
            let tcolor = if let (Some(first_tag), Some(wid)) = (tags.first(), &profile.workspace_id)
            {
                sqlx::query_scalar::<_, String>(
                    "SELECT color FROM workspace_columns WHERE workspace_id = ? AND tag_name = ?",
                )
                .bind(wid)
                .bind(first_tag)
                .fetch_optional(&core.db)
                .await
                .map_err(err)?
                .unwrap_or_else(|| wcolor.clone())
            } else {
                wcolor.clone()
            };

            let label = if wname.is_empty() {
                profile.name.clone()
            } else {
                format!("{} · {}", wname, profile.name)
            };

            let chrome_dir = firefox_profile_dir.join("chrome");
            std::fs::create_dir_all(&chrome_dir).map_err(err)?;
            std::fs::write(
                chrome_dir.join("userChrome.css"),
                userjs::camoufox_user_chrome(&wcolor, &tcolor, &label),
            )
            .map_err(err)?;

            if let Some(install_dir) = bin.parent() {
                // chrome.css is shared between all profiles — same lock as above.
                let _guard = crate::commands::camoufox::INSTALL_DIR_LOCK.lock().await;
                crate::commands::camoufox::patch_chrome_css(install_dir, &wcolor, &tcolor, &label)
                    .unwrap_or_else(|e| {
                        eprintln!("patch_chrome_css failed: {e}");
                    });
            }

            let _ = effective_proxy; // proxy already encoded in user.js

            ensure_window_size(firefox_profile_dir);
            let cfg = crate::commands::profiles::build_camoufox_config(profile);
            Ok((bin, Some(cfg)))
        }
        _ => {
            let bin = which::which("firefox")
                .map_err(|_| "Firefox not found in PATH. Please install it first.".to_string())?;
            Ok((bin, None))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile_dir(name: &str, xulstore: Option<&str>) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "veydan-window-size-{name}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::remove_file(dir.join("xulstore.json")).ok();
        if let Some(text) = xulstore {
            std::fs::write(dir.join("xulstore.json"), text).unwrap();
        }
        dir
    }

    fn main_window(dir: &std::path::Path) -> serde_json::Value {
        let text = std::fs::read_to_string(dir.join("xulstore.json")).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        json["chrome://browser/content/browser.xhtml"]["main-window"].clone()
    }

    #[test]
    fn a_new_profile_opens_with_the_default_size() {
        let dir = profile_dir("new", None);
        ensure_window_size(&dir);
        let win = main_window(&dir);
        assert_eq!(win["width"], "1280");
        assert_eq!(win["height"], "760");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_tiny_saved_size_is_replaced_and_the_state_kept() {
        let dir = profile_dir(
            "tiny",
            Some(r#"{"chrome://browser/content/browser.xhtml":{"main-window":{"screenX":"0","screenY":"0","width":"516","height":"200","sizemode":"maximized"}},"chrome://browser/content/places/places.xhtml":{"places":{"width":"800"}}}"#),
        );
        ensure_window_size(&dir);
        let win = main_window(&dir);
        assert_eq!(win["width"], "1280");
        assert_eq!(win["height"], "760");
        assert_eq!(win["sizemode"], "maximized");
        assert_eq!(win["screenX"], "0");
        let text = std::fs::read_to_string(dir.join("xulstore.json")).unwrap();
        assert!(text.contains("places.xhtml"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_size_to_work_in_stays_as_it_is() {
        let text = r#"{"chrome://browser/content/browser.xhtml":{"main-window":{"width":"1600","height":"900","sizemode":"normal"}}}"#;
        let dir = profile_dir("fine", Some(text));
        ensure_window_size(&dir);
        assert_eq!(
            std::fs::read_to_string(dir.join("xulstore.json")).unwrap(),
            text
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_file_that_is_not_json_is_left_alone() {
        let dir = profile_dir("broken", Some("{not json"));
        ensure_window_size(&dir);
        assert_eq!(
            std::fs::read_to_string(dir.join("xulstore.json")).unwrap(),
            "{not json"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
