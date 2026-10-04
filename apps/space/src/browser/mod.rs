// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

pub mod launch;
pub mod profile_launcher;
pub mod userjs;

use crate::commands::camoufox::DownloadManager;
use std::sync::Arc;

/// What the browser module keeps while the app runs.
#[derive(Default)]
pub struct BrowserState {
    /// The browsers that are running.
    pub running: Arc<launch::Running>,
    /// The download of Camoufox.
    pub download: DownloadManager,
}
