// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
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
    /// Which launch of the profile this is (`Running::next_generation`): the
    /// monitor of an older process must not take a newer one off the map.
    generation: u64,
    stop_tx: tokio::sync::oneshot::Sender<()>,
    local_proxy_stop: Option<tokio::sync::oneshot::Sender<()>>,
    /// Monitor task that owns the child and performs the actual kill + cleanup.
    /// Awaited (bounded) in `stop_all` so browsers are dead before the app exits.
    monitor: Option<tokio::task::JoinHandle<()>>,
}

#[cfg(test)]
impl BrowserProcess {
    /// A running browser as the map holds it, with no process behind it.
    pub fn for_test(pid: u32) -> Self {
        let (stop_tx, _) = tokio::sync::oneshot::channel();
        Self {
            pid,
            generation: 0,
            stop_tx,
            local_proxy_stop: None,
            monitor: None,
        }
    }
}

/// The running browsers by profile id (the map this derefs to) and, under
/// the same lock, the profiles reserved for a launch or for the end of one
/// (`LaunchSlot`), so that the check and the reservation are one step.
#[derive(Default)]
pub struct Processes {
    map: HashMap<String, BrowserProcess>,
    /// Profile id → the token of the slot that holds it.
    launching: HashMap<String, u64>,
}

impl Processes {
    fn give_back(&mut self, profile_id: &str, token: u64) {
        if self.launching.get(profile_id) == Some(&token) {
            self.launching.remove(profile_id);
        }
    }

    /// Puts the process of a launch on the map: `false`, and nothing changed,
    /// when a process of the profile is already there. The reservation of
    /// `profile_launch` (the one there can be while it runs) passes to the
    /// process: from now on its monitor owns the end.
    fn register(&mut self, profile_id: &str, process: BrowserProcess) -> bool {
        if self.map.contains_key(profile_id) {
            return false;
        }
        self.map.insert(profile_id.to_string(), process);
        self.launching.remove(profile_id);
        true
    }
}

impl std::ops::Deref for Processes {
    type Target = HashMap<String, BrowserProcess>;
    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

impl std::ops::DerefMut for Processes {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.map
    }
}

/// The browsers that are running, by profile id.
#[derive(Default)]
pub struct Running {
    pub processes: Arc<Mutex<Processes>>,
    next_generation: AtomicU64,
}

/// A profile reserved (`Running::reserve`): while it is held no launch of
/// the profile starts, and the monitor of a previous process of the profile
/// leaves the status and the lease alone. A launch's reservation passes to
/// the process when `launch` registers it. Given back by `release`; dropped
/// without it, it is given back all the same. Either gives back only this
/// slot's own reservation.
pub struct LaunchSlot {
    running: Arc<Running>,
    profile_id: String,
    token: u64,
    released: bool,
}

impl LaunchSlot {
    pub async fn release(mut self) {
        self.running
            .processes
            .lock()
            .await
            .give_back(&self.profile_id, self.token);
        self.released = true;
    }
}

impl Drop for LaunchSlot {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        if let Ok(mut procs) = self.running.processes.try_lock() {
            procs.give_back(&self.profile_id, self.token);
            return;
        }
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            let running = Arc::clone(&self.running);
            let id = std::mem::take(&mut self.profile_id);
            let token = self.token;
            rt.spawn(async move {
                running.processes.lock().await.give_back(&id, token);
            });
        }
    }
}

impl Running {
    /// Reserves the profile for a launch: `None` while it runs or another
    /// launch of it is under way. The lock of the profile's files against
    /// sync (`BrowserState::files`) is taken next to it, by `profile_launch`
    /// after the reservation, and held until the process is registered.
    pub async fn reserve(self: &Arc<Self>, profile_id: &str) -> Option<LaunchSlot> {
        let mut procs = self.processes.lock().await;
        self.slot_in(&mut procs, profile_id)
    }

    fn slot_in(self: &Arc<Self>, procs: &mut Processes, profile_id: &str) -> Option<LaunchSlot> {
        if procs.contains_key(profile_id) || procs.launching.contains_key(profile_id) {
            return None;
        }
        let token = self.next_generation();
        procs.launching.insert(profile_id.to_string(), token);
        Some(LaunchSlot {
            running: Arc::clone(self),
            profile_id: profile_id.to_string(),
            token,
            released: false,
        })
    }

    fn next_generation(&self) -> u64 {
        self.next_generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// The process `generation` of the profile is gone: takes it off the map
    /// (and stops its local proxy) when it is still there. When the profile
    /// is now stopped, the status, the event and the lease are the monitor's
    /// to set, and it gets the profile reserved while it sets them, so that
    /// a launch cannot slip in between; `None` when a newer process of the
    /// profile runs or a launch of it is under way.
    async fn retire(self: &Arc<Self>, profile_id: &str, generation: u64) -> Option<LaunchSlot> {
        let mut procs = self.processes.lock().await;
        if procs
            .get(profile_id)
            .is_some_and(|p| p.generation == generation)
        {
            if let Some(ps) = procs.remove(profile_id).and_then(|p| p.local_proxy_stop) {
                ps.send(()).ok();
            }
        }
        self.slot_in(&mut procs, profile_id)
    }

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
        .kill_on_drop(true);
    // The capture native host is started by the browser and inherits these.
    match crate::capture::bridge(
        &tauri::Manager::state::<veydan_core::Core>(&app_handle).app_data_dir,
    ) {
        Ok(bridge) => {
            cmd.env(crate::capture::ENDPOINT_ENV, bridge.endpoint)
                .env(crate::capture::TOKEN_ENV, bridge.token);
        }
        Err(e) => eprintln!("capture: no bridge for the browser: {e}"),
    }

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
    let generation = state.next_generation();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();

    {
        let mut procs = state.processes.lock().await;
        let process = BrowserProcess {
            pid,
            generation,
            stop_tx,
            local_proxy_stop,
            monitor: None,
        };
        if !procs.register(&profile_id, process) {
            // Concurrent launch race: kill the duplicate process and bail
            child.kill().await.ok();
            return Err(anyhow::anyhow!(
                "Profile already running (concurrent launch)"
            ));
        }
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

        // Stop local proxy (if any) when browser exits. A profile stopped and
        // launched again belongs to the newer launch: this monitor neither
        // takes it off the map nor marks it stopped or gives up its lease.
        let Some(slot) = state_clone.retire(&profile_id_clone, generation).await else {
            return;
        };
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
        slot.release().await;
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
    let mut cmd = tokio::process::Command::from(windows_kill_command(firefox_profile_dir));
    // CREATE_NO_WINDOW — don't flash a console window.
    cmd.creation_flags(0x0800_0000);
    cmd.output().await.ok();
}

/// The variable that hands the profile directory to the PowerShell of
/// `windows_kill_command`, so the path is never part of the script.
#[cfg(any(target_os = "windows", test))]
const NEEDLE_ENV: &str = "VEYDAN_NEEDLE";

/// Keeps the processes whose command line names the directory in
/// `$env:VEYDAN_NEEDLE` as a whole argument: the text itself, any case,
/// with a quote, a space or an end on both sides. A path is not a pattern
/// (`[`, `]`, `*` match only themselves), and `…\1\firefox-profile` does not
/// match `…\1\firefox-profile-old`.
#[cfg(any(target_os = "windows", test))]
const NAMES_THE_PROFILE: &str = "{ \
    $n = $env:VEYDAN_NEEDLE; $c = $_.CommandLine; $q = [string][char]34; $hit = $false; \
    if ($c -and $n) { \
        $i = $c.IndexOf($n, [System.StringComparison]::OrdinalIgnoreCase); \
        while ($i -ge 0) { \
            $e = $i + $n.Length; \
            $b = if ($i -eq 0) { ' ' } else { [string]$c[$i - 1] }; \
            $a = if ($e -ge $c.Length) { ' ' } else { [string]$c[$e] }; \
            if (($b -eq ' ' -or $b -eq $q) -and ($a -eq ' ' -or $a -eq $q)) { $hit = $true; break }; \
            $i = $c.IndexOf($n, $i + 1, [System.StringComparison]::OrdinalIgnoreCase) \
        } \
    }; \
    $hit }";

/// The PowerShell that force-stops the Camoufox/Firefox processes of the
/// profile directory; the directory goes in the environment, as it is.
#[cfg(any(target_os = "windows", test))]
fn windows_kill_command(firefox_profile_dir: &std::path::Path) -> std::process::Command {
    let script = format!(
        "Get-CimInstance Win32_Process -Filter \"Name LIKE '%camoufox%' OR Name LIKE '%firefox%'\" | \
         Where-Object {NAMES_THE_PROFILE} | \
         ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force }}"
    );
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .env(NEEDLE_ENV, firefox_profile_dir);
    cmd
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;

    /// A process of launch `generation` with a local proxy, and the end of
    /// that proxy's stop channel.
    fn process(generation: u64) -> (BrowserProcess, oneshot::Receiver<()>) {
        let (proxy_tx, proxy_rx) = oneshot::channel();
        let mut p = BrowserProcess::for_test(7);
        p.generation = generation;
        p.local_proxy_stop = Some(proxy_tx);
        (p, proxy_rx)
    }

    #[tokio::test]
    async fn an_old_monitor_leaves_the_new_process_alone() {
        let running = Arc::new(Running::default());
        let (p, mut proxy) = process(2);
        assert!(running.processes.lock().await.register("p", p));

        // The monitor of launch 1 ends after a stop and launch 2.
        assert!(running.retire("p", 1).await.is_none());
        assert!(running.is_running("p").await);
        assert!(
            proxy.try_recv().is_err(),
            "the proxy of the new process stays up"
        );
    }

    #[tokio::test]
    async fn the_monitor_of_the_process_takes_it_off_and_holds_the_profile_meanwhile() {
        let running = Arc::new(Running::default());
        let (p, mut proxy) = process(1);
        assert!(running.processes.lock().await.register("p", p));

        let slot = running
            .retire("p", 1)
            .await
            .expect("the end is this monitor's");
        assert!(!running.is_running("p").await);
        assert!(proxy.try_recv().is_ok(), "its proxy is stopped");
        assert!(
            running.reserve("p").await.is_none(),
            "no launch while the status is set"
        );
        slot.release().await;
        assert!(running.reserve("p").await.is_some());
    }

    #[tokio::test]
    async fn a_monitor_after_a_stop_leaves_a_launch_under_way_alone() {
        let running = Arc::new(Running::default());
        let launch = running.reserve("p").await.unwrap();
        // `stop` took the old process off the map; its monitor ends now.
        assert!(running.retire("p", 1).await.is_none());
        launch.release().await;
        assert!(running.retire("p", 1).await.is_some());
    }

    #[tokio::test]
    async fn a_profile_is_reserved_once_and_given_back_on_drop() {
        let running = Arc::new(Running::default());
        let slot = running.reserve("p").await.unwrap();
        assert!(running.reserve("p").await.is_none());
        assert!(
            running.reserve("q").await.is_some(),
            "another profile is free"
        );
        drop(slot);
        assert!(running.reserve("p").await.is_some());

        let (p, _proxy) = process(1);
        assert!(running.processes.lock().await.register("r", p));
        assert!(
            running.reserve("r").await.is_none(),
            "a running profile is not reserved"
        );
    }

    #[tokio::test]
    async fn a_launch_gives_back_only_its_own_reservation() {
        let running = Arc::new(Running::default());
        let launch = running.reserve("p").await.unwrap();
        let (p, _proxy) = process(1);
        assert!(running.processes.lock().await.register("p", p));
        // The browser dies before `profile_launch` returns: its monitor holds
        // the profile while it marks it stopped.
        let monitor = running.retire("p", 1).await.unwrap();
        launch.release().await;
        assert!(
            running.reserve("p").await.is_none(),
            "the monitor's reservation stays"
        );
        monitor.release().await;
        assert!(running.reserve("p").await.is_some());
    }

    #[test]
    fn the_windows_kill_hands_the_path_over_as_it_is() {
        let dir = std::path::Path::new(r"C:\Users\a b\Veydan\profiles\[1] 'x'\firefox-profile");
        let cmd = windows_kill_command(dir);
        let envs: Vec<_> = cmd.get_envs().collect();
        assert_eq!(
            envs,
            [(std::ffi::OsStr::new(NEEDLE_ENV), Some(dir.as_os_str()))]
        );
        let script = cmd.get_args().last().unwrap().to_str().unwrap();
        assert!(
            !script.contains("profiles"),
            "the path is not in the script: {script}"
        );
        assert!(!script.contains("-like"), "no pattern: {script}");
        assert!(script.contains("$env:VEYDAN_NEEDLE"));
        assert!(script.contains(".IndexOf($n, [System.StringComparison]::OrdinalIgnoreCase)"));
        assert!(script.contains("Where-Object { $n = $env:VEYDAN_NEEDLE;"));
        assert!(script.contains("ForEach-Object { Stop-Process -Id $_.ProcessId -Force }"));
    }
}
