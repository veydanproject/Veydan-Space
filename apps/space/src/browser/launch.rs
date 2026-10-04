// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Emitter;
use tokio::sync::Mutex;

#[derive(Clone, Serialize)]
pub struct RunningChangedPayload {
    pub running_ids: Vec<String>,
}

pub fn emit_running_changed(app_handle: &tauri::AppHandle, ids: Vec<String>) {
    app_handle
        .emit(
            "profiles://running-changed",
            RunningChangedPayload { running_ids: ids },
        )
        .ok();
}

pub struct BrowserProcess {
    pub pid: u32,
    stop_tx: tokio::sync::oneshot::Sender<()>,
    local_proxy_stop: Option<tokio::sync::oneshot::Sender<()>>,
    /// Monitor task that owns the child and performs the actual kill + cleanup.
    /// Awaited (bounded) in `stop_all` so browsers are dead before the app exits.
    monitor: Option<tokio::task::JoinHandle<()>>,
}

/// The browsers that are running, by profile id.
pub struct Running {
    pub processes: Arc<Mutex<HashMap<String, BrowserProcess>>>,
}

impl Default for Running {
    fn default() -> Self {
        Self {
            processes: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Running {
    pub async fn is_running(&self, profile_id: &str) -> bool {
        self.processes.lock().await.contains_key(profile_id)
    }

    pub async fn get_pid(&self, profile_id: &str) -> Option<u32> {
        self.processes.lock().await.get(profile_id).map(|p| p.pid)
    }

    pub async fn running_ids(&self) -> Vec<String> {
        self.processes.lock().await.keys().cloned().collect()
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn launch(
    profile_id: String,
    launched_locale: String,
    profile_path: PathBuf,
    browser_binary: PathBuf,
    timezone: Option<String>,
    camoufox_config: Option<serde_json::Value>,
    local_proxy_stop: Option<tokio::sync::oneshot::Sender<()>>,
    state: Arc<Running>,
    db: sqlx::Pool<sqlx::Sqlite>,
    app_handle: tauri::AppHandle,
) -> Result<u32> {
    let firefox_profile_dir = profile_path.join("firefox-profile");
    std::fs::create_dir_all(&firefox_profile_dir)
        .context("Failed to create firefox profile dir")?;

    #[cfg(target_os = "windows")]
    {
        // A stale parent.lock left over from a crash would make the exit monitor
        // below believe the browser is still running. A live Firefox holds the
        // lock open exclusively on Windows, so this delete fails harmlessly when
        // the profile really is in use.
        std::fs::remove_file(firefox_profile_dir.join("parent.lock")).ok();
    }

    let mut cmd = tokio::process::Command::new(&browser_binary);
    cmd.arg("--profile")
        .arg(&firefox_profile_dir)
        .arg("--no-remote")
        .arg("--new-instance")
        // Safety net: if the monitor task is ever dropped without an explicit
        // kill (e.g. runtime teardown), take the child down with it.
        .kill_on_drop(true)
        // The capture native host is started by the browser and inherits this.
        .env(crate::capture::ENDPOINT_ENV, crate::capture::ipc_endpoint());

    if let Some(tz) = timezone {
        cmd.env("TZ", tz);
    }

    if let Some(cfg) = camoufox_config {
        cmd.env("CAMOU_CONFIG_1", cfg.to_string());
    }

    let mut child = cmd
        .spawn()
        .with_context(|| format!("Failed to spawn {}", browser_binary.display()))?;

    let pid = child.id().unwrap_or(0);
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();

    {
        let mut procs = state.processes.lock().await;
        if procs.contains_key(&profile_id) {
            // Concurrent launch race: kill the duplicate process and bail
            child.kill().await.ok();
            return Err(anyhow::anyhow!(
                "Profile already running (concurrent launch)"
            ));
        }
        procs.insert(
            profile_id.clone(),
            BrowserProcess {
                pid,
                stop_tx,
                local_proxy_stop,
                monitor: None,
            },
        );
    } // lock released before tokio::spawn

    let state_clone = Arc::clone(&state);
    let profile_id_clone = profile_id.clone();
    let db_clone = db.clone();
    let app_handle_clone = app_handle.clone();
    let firefox_profile_dir_clone = firefox_profile_dir.clone();

    let monitor = tokio::spawn(async move {
        let mut stop_rx = stop_rx;

        // Phase 1: wait for the spawned process itself, with stop live.
        let process_exited = tokio::select! {
            _ = child.wait() => true,
            _ = &mut stop_rx => {
                child.kill().await.ok();
                child.wait().await.ok();
                false
            }
        };

        // Phase 2 (Windows only): camoufox.exe is a launcher that exits right
        // after spawning the real browser, so phase 1 ending does not mean the
        // browser is gone. Track the real browser via the profile lock file —
        // keeping stop_rx live so profile_stop still works in this phase.
        #[cfg(target_os = "windows")]
        if process_exited {
            let lock_file = firefox_profile_dir_clone.join("parent.lock");
            // Grace period for the real browser to claim the lock.
            tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
            let wait_lock_release = async {
                while lock_file.exists() {
                    // A running Firefox holds parent.lock open exclusively on
                    // Windows, so a successful delete means the lock is a stale
                    // crash leftover and nothing owns the profile anymore —
                    // without this check a stale lock would make us poll forever.
                    if std::fs::remove_file(&lock_file).is_ok() {
                        break;
                    }
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                }
            };
            tokio::select! {
                _ = wait_lock_release => {}
                _ = &mut stop_rx => {
                    // The launcher is long dead and Windows' parent.lock stores
                    // no PID, so the real browser can only be found by matching
                    // the profile path in its command line (best effort). If the
                    // match misses, the profile is still marked stopped below and
                    // the user has to close the browser window manually.
                    kill_windows_browser_by_profile(&firefox_profile_dir_clone).await;
                }
            }
        }
        #[cfg(not(target_os = "windows"))]
        let _ = process_exited;

        // Browser writes the chosen UI language into prefs.js on exit.
        import_browser_locale(
            &firefox_profile_dir_clone,
            &launched_locale,
            &profile_id_clone,
            &db_clone,
            &app_handle_clone,
        )
        .await;

        // Stop local proxy (if any) when browser exits
        if let Some(proc) = state_clone.processes.lock().await.remove(&profile_id_clone) {
            if let Some(ps) = proc.local_proxy_stop {
                ps.send(()).ok();
            }
        }
        sqlx::query(
            "UPDATE profiles SET status = 'stopped', updated_at = datetime('now') WHERE id = ?",
        )
        .bind(&profile_id_clone)
        .execute(&db_clone)
        .await
        .ok();
        let ids = state_clone.running_ids().await;
        emit_running_changed(&app_handle_clone, ids);
        crate::sync::on_profile_stopped(&app_handle_clone, &profile_id_clone).await;
    });

    // Store the monitor handle so stop_all can await the actual kill on app
    // exit. If the browser already exited (entry removed by the monitor), the
    // handle is simply dropped and the task finishes detached.
    if let Some(p) = state.processes.lock().await.get_mut(&profile_id) {
        p.monitor = Some(monitor);
    }

    Ok(pid)
}

/// If the browser UI language changed during the session, store it on the profile.
async fn import_browser_locale(
    firefox_profile_dir: &std::path::Path,
    launched_locale: &str,
    profile_id: &str,
    db: &sqlx::Pool<sqlx::Sqlite>,
    app_handle: &tauri::AppHandle,
) {
    let Ok(prefs) = std::fs::read_to_string(firefox_profile_dir.join("prefs.js")) else {
        return;
    };
    let Some(requested) = super::userjs::read_pref_string(&prefs, "intl.locale.requested") else {
        return;
    };
    let Some((locale, languages)) = super::userjs::match_locale(&requested) else {
        return;
    };
    if locale.eq_ignore_ascii_case(launched_locale) {
        return;
    }
    if sqlx::query(
        "UPDATE profiles SET locale = ?, languages = ?, updated_at = datetime('now') WHERE id = ?",
    )
    .bind(locale)
    .bind(languages)
    .bind(profile_id)
    .execute(db)
    .await
    .is_err()
    {
        return;
    }
    app_handle
        .emit(crate::sync::EVENT_CHANGED, vec!["profile".to_string()])
        .ok();
}

/// Best-effort kill of the real Camoufox/Firefox browser on Windows once the
/// launcher process is gone: finds processes whose command line references this
/// profile directory and force-stops them.
#[cfg(target_os = "windows")]
async fn kill_windows_browser_by_profile(firefox_profile_dir: &std::path::Path) {
    // PowerShell single-quoted strings escape ' by doubling it.
    let needle = firefox_profile_dir.to_string_lossy().replace('\'', "''");
    let script = format!(
        "Get-CimInstance Win32_Process -Filter \"Name LIKE '%camoufox%' OR Name LIKE '%firefox%'\" | \
         Where-Object {{ $_.CommandLine -like '*{needle}*' }} | \
         ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force }}"
    );
    let mut cmd = tokio::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
    // CREATE_NO_WINDOW — don't flash a console window.
    cmd.creation_flags(0x0800_0000);
    cmd.output().await.ok();
}

pub async fn stop(profile_id: &str, state: &Running) -> Result<()> {
    let process = state.processes.lock().await.remove(profile_id);
    if let Some(p) = process {
        if let Some(ps) = p.local_proxy_stop {
            ps.send(()).ok();
        }
        p.stop_tx.send(()).ok();
    }
    Ok(())
}

/// Signals every running browser to stop and waits (bounded) for the monitor
/// tasks to actually kill the processes. Called on app exit so children are
/// dead before the process goes away instead of racing it.
pub async fn stop_all(state: &Running) {
    let processes: Vec<_> = state.processes.lock().await.drain().collect();
    let mut monitors = Vec::new();
    for (_, p) in processes {
        if let Some(ps) = p.local_proxy_stop {
            ps.send(()).ok();
        }
        p.stop_tx.send(()).ok();
        if let Some(m) = p.monitor {
            monitors.push(m);
        }
    }
    if !monitors.is_empty() {
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            futures_util::future::join_all(monitors),
        )
        .await
        .ok();
    }
}
