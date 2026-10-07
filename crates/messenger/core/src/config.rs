// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Everything the host passes in. The runtime derives all paths from
/// `data_dir` and never reads host state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessengerConfig {
    /// Root of messenger data: `messenger.db`, `media/`, `avatars/`, `tmp/`.
    pub data_dir: PathBuf,
    /// Transfers the closing of the app interrupted start again by
    /// themselves once a session runs (the app). A tool that runs one
    /// command and ends leaves them to the user (the CLI).
    #[serde(default = "yes")]
    pub resume_transfers: bool,
}

fn yes() -> bool {
    true
}

impl MessengerConfig {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self { data_dir: data_dir.into(), resume_transfers: true }
    }

    /// See `resume_transfers`.
    pub fn without_resume(mut self) -> Self {
        self.resume_transfers = false;
        self
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("messenger.db")
    }

    pub fn media_dir(&self) -> PathBuf {
        self.data_dir.join("media")
    }

    pub fn avatars_dir(&self) -> PathBuf {
        self.data_dir.join("avatars")
    }

    pub fn tmp_dir(&self) -> PathBuf {
        self.data_dir.join("tmp")
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }
}
