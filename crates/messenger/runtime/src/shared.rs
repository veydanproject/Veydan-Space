// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a chat has shared, by section, for the panel about a chat: the
//! same for a conversation of two and for a group.

use crate::MessengerRuntime;
use messenger_core::Result;
use messenger_dm::MessageView;
use messenger_store::shared::{Counts, Section};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SharedSection {
    /// Pictures and videos.
    Visual,
    /// Files and music.
    Files,
    /// Texts and captions with a link in them.
    Links,
    /// Voice and round videos recorded in the app.
    Voice,
}

impl From<SharedSection> for Section {
    fn from(s: SharedSection) -> Self {
        match s {
            SharedSection::Visual => Section::Visual,
            SharedSection::Files => Section::Files,
            SharedSection::Links => Section::Links,
            SharedSection::Voice => Section::Voice,
        }
    }
}

/// How many messages each section holds. Links are counted by message.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SharedCounts {
    pub visual: u32,
    pub files: u32,
    pub links: u32,
    pub voice: u32,
}

impl From<Counts> for SharedCounts {
    fn from(c: Counts) -> Self {
        let n = |v: i64| u32::try_from(v).unwrap_or(u32::MAX);
        Self { visual: n(c.visual), files: n(c.files), links: n(c.links), voice: n(c.voice) }
    }
}

impl MessengerRuntime {
    pub async fn shared_counts(&self, chat_id: &str) -> Result<SharedCounts> {
        Ok(self.dm().shared_counts(chat_id).await?.into())
    }

    /// One page of a section, newest first, strictly older than `before`.
    pub async fn shared(&self, chat_id: &str, section: SharedSection, before: Option<i64>, limit: i64) -> Result<Vec<MessageView>> {
        self.dm().shared(chat_id, section.into(), before, limit).await
    }
}
