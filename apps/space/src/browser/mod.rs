// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

pub mod launch;
pub mod profile_launcher;
pub mod userjs;

use crate::commands::camoufox::DownloadManager;
use std::collections::HashMap;
use std::sync::Arc;

/// What the browser module keeps while the app runs.
#[derive(Default)]
pub struct BrowserState {
    /// The browsers that are running.
    pub running: Arc<launch::Running>,
    /// The download of Camoufox.
    pub download: DownloadManager,
    /// The locks of the profiles' `firefox-profile/` folders.
    pub files: ProfileLocks,
}

/// One lock per profile over its `firefox-profile/` folder. Sync holds it
/// while it collects or applies the files of the profile; a launch from
/// before it applies a waiting snapshot until its process is registered; a
/// stop while it marks the files changed; a delete while it removes the
/// folder. So a browser never starts on files half written, and sync never
/// reads or writes the files of a browser that started meanwhile: whoever
/// holds the lock looks at `running` again.
#[derive(Default)]
pub struct ProfileLocks(std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>);

impl ProfileLocks {
    /// Waits for the lock of `profile_id`. Not reentrant: whoever holds it
    /// calls nothing that takes it again.
    pub async fn lock(&self, profile_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
            Arc::clone(map.entry(profile_id.to_string()).or_default())
        };
        lock.lock_owned().await
    }
}
