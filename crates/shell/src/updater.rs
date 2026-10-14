// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The updater of a computer (internal/platform-spec.md 14.3), behind the
//! shell's own commands: the page holds no permission of the plugins
//! `updater` and `process`.
//!
//! An update is a version newer than this build ([`upgrade_only`]): the
//! version in `latest.json` is not signed, and the plugin's own check lets
//! the page ask for any other one (`allowDowngrades`). On Windows the
//! installer exits the app as soon as it starts (`std::process::exit` in the
//! plugin, no `RunEvent::Exit`): its hook `on_before_exit` stops the modules
//! first ([`Shell::shutdown`], as on exit), so no browser, watcher or
//! database of a module is left open under the installer. Elsewhere the
//! update replaces the files and the page asks for `app_relaunch`, which
//! stops them and restarts.

use crate::Shell;
use serde::Serialize;
use std::future::Future;
use std::time::Duration;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{RemoteRelease, Updater, UpdaterExt};
use veydan_core::{AppError, CmdResult};

/// How long the installer of Windows waits for the modules to stop.
const SHUTDOWN_LIMIT: Duration = Duration::from_secs(10);

/// The comparator of the updater: only a newer version is an update.
pub(crate) fn upgrade_only(current: semver::Version, release: RemoteRelease) -> bool {
    release.version > current
}

/// What `updater_install` tells the page while the update downloads.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum InstallProgress {
    /// `len` more bytes came, of `total` when the server tells it.
    Chunk { len: usize, total: Option<u64> },
    /// The download is whole; the install begins.
    Downloaded,
}

fn updater(app: &AppHandle) -> CmdResult<Updater> {
    let handle = app.clone();
    app.updater_builder()
        .version_comparator(upgrade_only)
        .on_before_exit(move || before_installer(&handle))
        .build()
        .map_err(AppError::other)
}

/// The version an update offers, or `None` when this build is the newest.
#[tauri::command]
pub async fn updater_check(app: AppHandle) -> CmdResult<Option<String>> {
    let update = updater(&app)?.check().await.map_err(AppError::other)?;
    Ok(update.map(|update| update.version))
}

/// Download the update and install it. On Windows the app exits into the
/// installer and this never answers; elsewhere the page then asks for
/// `app_relaunch`.
#[tauri::command]
pub async fn updater_install(
    app: AppHandle,
    on_progress: Channel<InstallProgress>,
) -> CmdResult<()> {
    let update = updater(&app)?
        .check()
        .await
        .map_err(AppError::other)?
        .ok_or_else(|| AppError::NotFound("no update".into()))?;
    update
        .download_and_install(
            |len, total| {
                let _ = on_progress.send(InstallProgress::Chunk { len, total });
            },
            || {
                let _ = on_progress.send(InstallProgress::Downloaded);
            },
        )
        .await
        .map_err(AppError::other)
}

/// Stop the modules and start the app again: after an update has replaced
/// the files, in place of the plugin `process`.
#[tauri::command]
pub async fn app_relaunch(app: AppHandle, shell: tauri::State<'_, Shell>) -> CmdResult<()> {
    shell.shutdown().await;
    // From a thread of the async runtime, so Tauri still sends
    // `RunEvent::Exit`; the shutdown there finds the modules stopped.
    app.restart()
}

/// The hook of the updater, run right before the installer of Windows
/// starts: the modules stop as on exit, then the cleanup the plugin's own
/// hook does.
fn before_installer(app: &AppHandle) {
    let handle = app.clone();
    let stop = async move {
        if let Some(shell) = handle.try_state::<Shell>() {
            shell.shutdown().await;
        }
    };
    if !wait_off_runtime(stop, SHUTDOWN_LIMIT) {
        eprintln!(
            "updater: the modules did not stop within {} s; the installer starts",
            SHUTDOWN_LIMIT.as_secs()
        );
    }
    app.cleanup_before_exit();
}

/// Run `work` to its end, at most `limit`, from a caller that must not go on
/// before it and runs on a thread of the async runtime (the updater calls
/// its hook from inside `download_and_install`), where a `block_on` of its
/// own would panic: `work` runs on a thread of its own. `false` when it did
/// not end in time (or panicked).
pub(crate) fn wait_off_runtime<F>(work: F, limit: Duration) -> bool
where
    F: Future<Output = ()> + Send + 'static,
{
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        tauri::async_runtime::block_on(work);
        let _ = done.send(());
    });
    finished.recv_timeout(limit).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    fn release(version: &str) -> RemoteRelease {
        serde_json::from_value(serde_json::json!({
            "version": version,
            "url": "https://example.invalid/app.tar.gz",
            "signature": "s",
        }))
        .unwrap()
    }

    #[test]
    fn only_a_newer_version_is_an_update() {
        let current = semver::Version::new(5, 1, 3);
        assert!(upgrade_only(current.clone(), release("5.1.4")));
        assert!(upgrade_only(current.clone(), release("6.0.0")));
        assert!(!upgrade_only(current.clone(), release("5.1.3")));
        assert!(!upgrade_only(current.clone(), release("5.1.2")));
        assert!(!upgrade_only(current, release("4.0.7")));
    }

    /// Called from a thread of the async runtime, as the updater calls its
    /// hook: the work ends before the call returns, and nothing panics.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_hook_waits_for_the_work_from_inside_the_runtime() {
        let ran = Arc::new(AtomicBool::new(false));
        let seen = ran.clone();
        let work = async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            seen.store(true, Ordering::SeqCst);
        };
        assert!(wait_off_runtime(work, Duration::from_secs(5)));
        assert!(ran.load(Ordering::SeqCst));
    }

    /// A stop that hangs does not keep the installer waiting for ever.
    #[test]
    fn a_hanging_stop_is_given_up_after_the_limit() {
        let started = std::time::Instant::now();
        assert!(!wait_off_runtime(
            std::future::pending(),
            Duration::from_millis(50)
        ));
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
