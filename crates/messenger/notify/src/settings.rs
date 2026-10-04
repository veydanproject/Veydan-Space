// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What the user wants a notification to give away. Kept in the
//! messenger's settings, written by the app, read here.

use messenger_core::Result;
use messenger_store::{settings, Store};

pub const KEY_CONTENT: &str = "notify.content";
pub const KEY_LOCKSCREEN_HIDDEN: &str = "notify.lockscreen_hidden";

/// How much of a message a notification shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Content {
    /// Who wrote and what.
    #[default]
    SenderText,
    /// Who wrote; the text stays in the app.
    Sender,
    /// Only that something came.
    None,
}

impl Content {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SenderText => "sender_text",
            Self::Sender => "sender",
            Self::None => "none",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "sender_text" => Some(Self::SenderText),
            "sender" => Some(Self::Sender),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Settings {
    pub content: Content,
    /// On the lock screen the notification says only that something came.
    pub lockscreen_hidden: bool,
}

impl Settings {
    pub async fn load(store: &Store) -> Result<Self> {
        let content = settings::get(store, KEY_CONTENT).await?.and_then(|s| Content::parse(&s)).unwrap_or_default();
        let lockscreen_hidden = settings::get_bool(store, KEY_LOCKSCREEN_HIDDEN, false).await?;
        Ok(Self { content, lockscreen_hidden })
    }

    pub async fn save(&self, store: &Store) -> Result<()> {
        settings::set(store, KEY_CONTENT, self.content.as_str()).await?;
        settings::set_bool(store, KEY_LOCKSCREEN_HIDDEN, self.lockscreen_hidden).await
    }
}

pub const KEY_DESKTOP_ENABLED: &str = "notify.desktop.enabled";
pub const KEY_DESKTOP_SOUND: &str = "notify.desktop.sound";

/// Notifications of a computer: the app itself shows them while it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DesktopSettings {
    pub enabled: bool,
    pub sound: bool,
}

impl Default for DesktopSettings {
    fn default() -> Self {
        Self { enabled: true, sound: true }
    }
}

impl DesktopSettings {
    pub async fn load(store: &Store) -> Result<Self> {
        Ok(Self {
            enabled: settings::get_bool(store, KEY_DESKTOP_ENABLED, true).await?,
            sound: settings::get_bool(store, KEY_DESKTOP_SOUND, true).await?,
        })
    }

    pub async fn save(&self, store: &Store) -> Result<()> {
        settings::set_bool(store, KEY_DESKTOP_ENABLED, self.enabled).await?;
        settings::set_bool(store, KEY_DESKTOP_SOUND, self.sound).await
    }
}
