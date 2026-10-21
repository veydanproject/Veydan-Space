// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

pub mod launch;
pub mod profile_launcher;
pub mod userjs;

use crate::commands::camoufox::DownloadManager;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tauri::Emitter;
use tokio::sync::watch;

/// What the browser module keeps while the app runs.
#[derive(Default)]
pub struct BrowserState {
    /// The browsers that are running.
    pub running: Arc<launch::Running>,
    /// The download of Camoufox.
    pub download: DownloadManager,
    /// The locks of the profiles' `firefox-profile/` folders.
    pub files: ProfileLocks,
    /// The launches under way, as the UI shows them.
    pub launches: Launches,
}

/// One lock per profile over its `firefox-profile/` folder. Sync holds it
/// while it collects or applies the files of the profile; a launch from
/// before it applies a waiting snapshot until its process is registered; a
/// stop while it marks the files changed; a delete while it removes the
/// folder. So a browser never starts on files half written, and sync never
/// reads or writes the files of a browser that started meanwhile: whoever
/// holds the lock looks at `running` again.
///
/// A launch that has to wait says so (`lock_for_launch`): the collect of a
/// snapshot, which may upload for minutes, watches for it (`LaunchWaits`)
/// and gives the profile up; the files stay changed and go out later.
pub struct ProfileLocks {
    locks: std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Profile id → the launches waiting for its lock.
    waiting: watch::Sender<HashMap<String, usize>>,
}

impl Default for ProfileLocks {
    fn default() -> Self {
        Self {
            locks: Default::default(),
            waiting: watch::channel(HashMap::new()).0,
        }
    }
}

impl ProfileLocks {
    fn of(&self, profile_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut map = self.locks.lock().unwrap_or_else(|e| e.into_inner());
        Arc::clone(map.entry(profile_id.to_string()).or_default())
    }

    /// Waits for the lock of `profile_id`. Not reentrant: whoever holds it
    /// calls nothing that takes it again.
    pub async fn lock(&self, profile_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        self.of(profile_id).lock_owned().await
    }

    /// The lock of `profile_id` for a launch. When it is held, `on_wait`
    /// runs and the launch is seen waiting until it gets the lock (or the
    /// future is dropped), so that a collect gives the profile up.
    pub async fn lock_for_launch(
        &self,
        profile_id: &str,
        on_wait: impl FnOnce(),
    ) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = self.of(profile_id);
        if let Ok(guard) = Arc::clone(&lock).try_lock_owned() {
            return guard;
        }
        on_wait();
        let _waiting = Waiting::new(&self.waiting, profile_id);
        lock.lock_owned().await
    }

    /// Watches the launches that wait for a lock.
    pub fn launch_waits(&self) -> LaunchWaits {
        LaunchWaits(self.waiting.subscribe())
    }

    /// A launch seen waiting for the lock of `profile_id` while the value
    /// lives, whoever holds the lock.
    #[cfg(test)]
    pub fn launch_waiting(&self, profile_id: &str) -> Waiting<'_> {
        Waiting::new(&self.waiting, profile_id)
    }
}

/// A launch counted as waiting for the lock of its profile while it lives.
pub struct Waiting<'a> {
    waiting: &'a watch::Sender<HashMap<String, usize>>,
    profile_id: String,
}

impl<'a> Waiting<'a> {
    fn new(waiting: &'a watch::Sender<HashMap<String, usize>>, profile_id: &str) -> Self {
        waiting.send_modify(|m| *m.entry(profile_id.to_string()).or_default() += 1);
        Self {
            waiting,
            profile_id: profile_id.to_string(),
        }
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.waiting.send_modify(|m| {
            if let Some(n) = m.get_mut(&self.profile_id) {
                *n -= 1;
                if *n == 0 {
                    m.remove(&self.profile_id);
                }
            }
        });
    }
}

/// The launches waiting for the lock of a profile, as a holder sees them.
#[derive(Clone)]
pub struct LaunchWaits(watch::Receiver<HashMap<String, usize>>);

impl LaunchWaits {
    /// A launch waits for the lock of `profile_id` now.
    pub fn waits_for(&self, profile_id: &str) -> bool {
        self.0.borrow().contains_key(profile_id)
    }

    /// A launch waits for the lock of one of `profile_ids` now.
    pub fn waits_for_any(&self, profile_ids: &[String]) -> bool {
        let m = self.0.borrow();
        profile_ids.iter().any(|id| m.contains_key(id))
    }

    /// Resolves once a launch waits for the lock of one of `profile_ids`.
    pub async fn until_any(&self, profile_ids: &[String]) {
        let mut rx = self.0.clone();
        let found = rx.wait_for(|m| profile_ids.iter().any(|id| m.contains_key(id)));
        if found.await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// The profiles being launched; `true` while the launch waits for sync
/// (the files of the profile are collected or applied, or a snapshot that
/// waited is applied before the start).
#[derive(Default)]
pub struct Launches(std::sync::Mutex<BTreeMap<String, bool>>);

#[derive(Clone, Serialize, Default, Debug, PartialEq)]
pub struct LaunchingPayload {
    pub launching: Vec<String>,
    pub syncing: Vec<String>,
}

impl Launches {
    /// Puts the launch of `profile_id` at `syncing`, or ends it with `None`,
    /// and tells the UI.
    pub fn set(&self, app: &tauri::AppHandle, profile_id: &str, syncing: Option<bool>) {
        {
            let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
            match syncing {
                Some(s) => map.insert(profile_id.to_string(), s),
                None => map.remove(profile_id),
            };
        }
        app.emit("profiles://launching-changed", self.payload())
            .ok();
    }

    pub fn payload(&self) -> LaunchingPayload {
        let map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        LaunchingPayload {
            launching: map.keys().cloned().collect(),
            syncing: map
                .iter()
                .filter(|(_, s)| **s)
                .map(|(id, _)| id.clone())
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn a_launch_that_waits_is_seen_until_it_gets_the_lock() {
        let locks = Arc::new(ProfileLocks::default());
        let waits = locks.launch_waits();
        let held = locks.lock("p").await;
        assert!(!waits.waits_for("p"));

        let launch = tokio::spawn({
            let locks = Arc::clone(&locks);
            async move {
                let mut waited = false;
                let _g = locks.lock_for_launch("p", || waited = true).await;
                waited
            }
        });
        let ids = ["q".to_string(), "p".to_string()];
        tokio::time::timeout(Duration::from_secs(5), waits.until_any(&ids))
            .await
            .expect("the launch is seen waiting");
        assert!(waits.waits_for("p"));
        assert!(!waits.waits_for("q"));
        assert!(waits.waits_for_any(&ids));
        assert!(!waits.waits_for_any(&ids[..1]));

        drop(held);
        assert!(launch.await.unwrap(), "on_wait ran");
        assert!(!waits.waits_for("p"));
    }

    #[tokio::test]
    async fn a_free_lock_is_taken_without_waiting() {
        let locks = ProfileLocks::default();
        let waits = locks.launch_waits();
        let mut waited = false;
        let _g = locks.lock_for_launch("p", || waited = true).await;
        assert!(!waited);
        assert!(!waits.waits_for("p"));
    }

    #[tokio::test]
    async fn a_launch_dropped_while_waiting_is_not_seen_any_more() {
        let locks = ProfileLocks::default();
        let waits = locks.launch_waits();
        let _held = locks.lock("p").await;
        let launch = locks.lock_for_launch("p", || {});
        let _ = tokio::time::timeout(Duration::from_millis(50), launch).await;
        assert!(!waits.waits_for("p"));
    }
}
