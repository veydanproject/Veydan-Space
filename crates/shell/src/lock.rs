// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The lock of the app, as the shell runs it: the [`Lock`] is in place
//! before any module is set up, its commands are the shell's, its events
//! reach the UI as Tauri events, and the auto-lock ticks here.

use serde::Serialize;
use std::time::Duration;
use tauri::{Emitter, Manager};
use tokio::sync::broadcast::{error::RecvError, Receiver};
use veydan_core::{settings, CmdResult, Core, Schema};
use veydan_lock::{Event, KeyUser, Lock, LockStatus};

/// The tables of the lock in the data file, under the module name `lock`.
pub(crate) const SCHEMA: Schema = Schema {
    module: "lock",
    steps: veydan_lock::SCHEMA_STEPS,
};

/// Minutes of inactivity before the lock closes: a setting of the core. The
/// name is older than the lock of the whole app; the key is synced, and
/// synced keys are part of the frozen vault format.
pub const TIMEOUT_KEY: &str = "notes_lock_timeout_min";
/// How often the auto-lock looks at the session.
const TICK: Duration = Duration::from_secs(10);

pub const EVENT_LOCKED: &str = "lock://locked";
pub const EVENT_UNLOCKED: &str = "lock://unlocked";
/// The key row or what is under the key changed: the passwords reload.
pub const EVENT_VAULT_CHANGED: &str = "passwords://vault-changed";

/// The event of the UI for an event of the lock. A reset changes what is
/// under the key as well.
fn event_name(event: Event) -> &'static str {
    match event {
        Event::Locked => EVENT_LOCKED,
        Event::Unlocked => EVENT_UNLOCKED,
        Event::Changed | Event::Reset => EVENT_VAULT_CHANGED,
    }
}

fn timeout_of(value: Option<&str>) -> u32 {
    value
        .and_then(|v| v.parse().ok())
        .unwrap_or(veydan_lock::DEFAULT_TIMEOUT_MIN)
}

/// Put the lock of the data file in place, knowing what the modules keep
/// under its key: take over what an earlier build kept elsewhere, open the
/// vault without a lock, follow the timeout setting, pass the events on to
/// the UI and start the auto-lock. Runs once `Core` is managed and before
/// the setups of the modules, inside the setup of the app: the lock is
/// managed open, before the first command is served, on every platform.
pub(crate) fn start(app: &tauri::App, key_users: Vec<KeyUser>) {
    let core = app.state::<Core>();
    let lock = Lock::with_key_users(core.db.clone(), key_users);
    // Subscribed first: what the opening tells reaches the UI too.
    let events = lock.subscribe();
    tauri::async_runtime::block_on(prepare(&lock));
    let timeout = tauri::async_runtime::block_on(settings::get(&core.db, TIMEOUT_KEY));
    lock.set_timeout(timeout_of(timeout.as_deref()));
    let handle = app.handle().clone();
    core.settings.subscribe(TIMEOUT_KEY, move |value| {
        let app = handle.clone();
        Box::pin(async move {
            if let Some(lock) = app.try_state::<Lock>() {
                lock.set_timeout(timeout_of(value.as_deref()));
            }
        })
    });
    forward_events(app.handle().clone(), events);
    app.manage(lock);
    auto_lock(app.handle().clone());
}

/// Take over what an earlier build kept elsewhere and, without a PIN, open
/// the vault with the built-in secret. Awaited before the lock is managed:
/// opened in the background, a command right after the start (the first
/// identity of the messenger, a password) met a vault that was still
/// closed (`vault_locked`; 19, № 54). The page waits for it: its window is
/// built once the setup of the app is done (`app::open_windows`).
async fn prepare(lock: &Lock) {
    // On failure the lock stays closed for this run, and the next start tries again.
    if let Err(e) = lock.upgrade().await {
        eprintln!("lock: what an earlier build kept was not taken over: {e}");
    }
    if let Err(e) = lock.open_default().await {
        eprintln!("lock: {e}");
    }
}

fn forward_events(app: tauri::AppHandle, mut events: Receiver<Event>) {
    tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    let _ = app.emit(event_name(event), ());
                }
                // Events were lost: the UI hears where the lock is now.
                Err(RecvError::Lagged(_)) => {
                    let Some(lock) = app.try_state::<Lock>() else {
                        continue;
                    };
                    for name in caught_up(lock.is_locked().await) {
                        let _ = app.emit(name, ());
                    }
                }
                Err(RecvError::Closed) => break,
            }
        }
    });
}

/// What the UI hears after events were lost: the state of the session, and
/// that the key may have changed.
fn caught_up(locked: bool) -> [&'static str; 2] {
    let session = if locked { EVENT_LOCKED } else { EVENT_UNLOCKED };
    [session, EVENT_VAULT_CHANGED]
}

/// The session closes once the timeout passes.
fn auto_lock(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let lock = app.state::<Lock>();
        loop {
            tokio::time::sleep(TICK).await;
            lock.tick();
        }
    });
}

/// Status plus the recovery key when one was just created. The key is shown once.
#[derive(Debug, Serialize)]
pub struct LockSetResult {
    #[serde(flatten)]
    pub status: LockStatus,
    pub recovery_key: Option<String>,
}

#[tauri::command]
pub async fn lock_status(lock: tauri::State<'_, Lock>) -> CmdResult<LockStatus> {
    Ok(lock.status().await)
}

/// Enable, change or (with `password = None`) remove the lock.
/// `current` is required whenever a secret is already set.
/// The first enable also creates the recovery key and returns it once.
#[tauri::command]
pub async fn lock_set(
    password: Option<String>,
    current: Option<String>,
    kind: Option<String>,
    hint: Option<String>,
    lock: tauri::State<'_, Lock>,
) -> CmdResult<LockSetResult> {
    let recovery_key = lock.set(password, current, kind, hint).await?;
    Ok(LockSetResult {
        status: lock.status().await,
        recovery_key,
    })
}

/// Replace the recovery key. The old one stops working.
#[tauri::command]
pub async fn lock_recovery_regenerate(
    current: String,
    lock: tauri::State<'_, Lock>,
) -> CmdResult<String> {
    Ok(lock.recovery_regenerate(&current).await?)
}

/// Step 1 of recovery: confirm the code opens the vault, nothing changes.
#[tauri::command]
pub async fn lock_recovery_check(code: String, lock: tauri::State<'_, Lock>) -> CmdResult<()> {
    Ok(lock.recovery_check(&code).await?)
}

/// Step 2 of recovery: set a new secret with the code, unlock, and hand out
/// a fresh recovery key. The used code stops working.
#[tauri::command]
pub async fn lock_recover(
    code: String,
    password: String,
    kind: Option<String>,
    hint: Option<String>,
    lock: tauri::State<'_, Lock>,
) -> CmdResult<LockSetResult> {
    let recovery_key = lock.recover(&code, password, kind, hint).await?;
    Ok(LockSetResult {
        status: lock.status().await,
        recovery_key: Some(recovery_key),
    })
}

#[tauri::command]
pub async fn lock_timeout_set(
    minutes: u32,
    core: tauri::State<'_, Core>,
    lock: tauri::State<'_, Lock>,
) -> CmdResult<LockStatus> {
    core.settings
        .set(TIMEOUT_KEY, &minutes.to_string())
        .await?;
    lock.set_timeout(minutes);
    Ok(lock.status().await)
}

#[tauri::command]
pub async fn lock_unlock(password: String, lock: tauri::State<'_, Lock>) -> CmdResult<LockStatus> {
    lock.unlock(&password).await?;
    Ok(lock.status().await)
}

#[tauri::command]
pub async fn lock_lock(lock: tauri::State<'_, Lock>) -> CmdResult<LockStatus> {
    lock.lock();
    Ok(lock.status().await)
}

/// Frontend reports user activity so the inactivity timer restarts.
#[tauri::command]
pub async fn lock_touch(lock: tauri::State<'_, Lock>) -> CmdResult<()> {
    lock.touch();
    Ok(())
}

/// How many vault keys that a synced row replaced are still closed: what this
/// device encrypted with them cannot be read until their secret is given.
#[tauri::command]
pub async fn vault_replaced_keys(lock: tauri::State<'_, Lock>) -> CmdResult<usize> {
    Ok(lock.closed_keys().await?)
}

/// Open the replaced keys with the PIN, password or recovery key this device
/// had before the synced row came. Returns how many stay closed.
#[tauri::command]
pub async fn vault_replaced_keys_open(
    secret: String,
    lock: tauri::State<'_, Lock>,
) -> CmdResult<usize> {
    Ok(lock.open_replaced_keys(secret.trim()).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ui_hears_the_lock_under_the_names_of_section_8() {
        assert_eq!(event_name(Event::Locked), "lock://locked");
        assert_eq!(event_name(Event::Unlocked), "lock://unlocked");
        assert_eq!(event_name(Event::Changed), "passwords://vault-changed");
        assert_eq!(event_name(Event::Reset), "passwords://vault-changed");
    }

    #[test]
    fn after_lost_events_the_ui_hears_the_state_of_the_lock() {
        assert_eq!(
            caught_up(true),
            ["lock://locked", "passwords://vault-changed"]
        );
        assert_eq!(
            caught_up(false),
            ["lock://unlocked", "passwords://vault-changed"]
        );
    }

    /// The lock of a new data file comes out of `prepare` open: the box of
    /// the messenger takes an identity at once. Without it the vault stays
    /// closed until a background task gets to it.
    #[tokio::test]
    async fn the_vault_of_a_new_data_file_is_open_once_prepared() {
        let dir = tempfile::tempdir().unwrap();
        let db = veydan_core::db::open(
            &dir.path().join(veydan_core::db::DB_FILE),
            &[veydan_core::SCHEMA, SCHEMA],
        )
        .await
        .unwrap();
        let unprepared = Lock::with_key_users(db.clone(), Vec::new());
        assert!(!unprepared.secret_box("messenger").is_unlocked());
        assert!(unprepared
            .secret_box("messenger")
            .put("identity", b"nsec")
            .await
            .is_err());

        let lock = Lock::with_key_users(db.clone(), Vec::new());
        prepare(&lock).await;
        assert!(lock.secret_box("messenger").is_unlocked());
        lock.secret_box("messenger")
            .put("identity", b"nsec")
            .await
            .unwrap();
        assert!(!lock.is_locked().await);
        db.close().await;
    }

    #[test]
    fn a_timeout_that_does_not_parse_is_the_default() {
        assert_eq!(timeout_of(Some("15")), 15);
        assert_eq!(timeout_of(Some("0")), 0);
        assert_eq!(timeout_of(Some("soon")), veydan_lock::DEFAULT_TIMEOUT_MIN);
        assert_eq!(timeout_of(None), veydan_lock::DEFAULT_TIMEOUT_MIN);
    }
}
