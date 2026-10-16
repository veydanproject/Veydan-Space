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
    /// The pinned key of the bot of Veydan Cloud (messenger-wire.md §3,
    /// "Карточки бота"), 64 hex: its DMs land in its chat without a request
    /// of a stranger, an envelope of it as a card, a plain text as a
    /// message. None: no key is a bot.
    #[serde(default)]
    pub bot_pubkey: Option<String>,
}

fn yes() -> bool {
    true
}

impl MessengerConfig {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self { data_dir: data_dir.into(), resume_transfers: true, bot_pubkey: None }
    }

    /// See `bot_pubkey`.
    pub fn with_bot(mut self, pubkey_hex: Option<&str>) -> Self {
        self.bot_pubkey = pubkey_hex.map(str::to_ascii_lowercase);
        self
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

    /// Events the push handler opened and showed, left for the app to take
    /// (`messenger_ingress::push_inbox`).
    pub fn push_inbox_dir(&self) -> PathBuf {
        self.data_dir.join("push-inbox")
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }
}
