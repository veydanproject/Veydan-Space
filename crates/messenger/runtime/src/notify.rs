// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What the phone's notification handler gets from the app: the keys it
//! opens events with, as a bundle the host keeps in the phone's own key
//! store, and the settings of what a notification may say.

use crate::MessengerRuntime;
use messenger_core::{PubKey, Result};
use messenger_notify::{Content, DesktopSettings, Face, GroupKeyEntry, KeyBundle, Outcome, Settings};

impl MessengerRuntime {
    /// The keys a push handler needs today, or None without a session.
    pub async fn notify_bundle(&self) -> Result<Option<KeyBundle>> {
        let Ok(keys) = self.session_keys().await else {
            return Ok(None);
        };
        let groups = self.groups().export_keys().await?.iter().map(|(g, k)| GroupKeyEntry::of(g, k)).collect();
        Ok(Some(KeyBundle::new(&keys, groups)))
    }

    /// Changes when `notify_bundle` would; costs no secret.
    pub async fn notify_fingerprint(&self) -> Result<Option<String>> {
        let Ok(keys) = self.session_keys().await else {
            return Ok(None);
        };
        Ok(Some(format!("{}|{}", keys.public_key().to_hex(), self.groups().keys_fingerprint().await?)))
    }

    pub async fn notify_settings(&self) -> Result<Settings> {
        Settings::load(self.store()).await
    }

    pub async fn notify_set_settings(&self, settings: Settings) -> Result<Settings> {
        settings.save(self.store()).await?;
        Ok(settings)
    }

    /// Whether the handler is to have the keys at all.
    pub async fn notify_wants_keys(&self) -> Result<bool> {
        Ok(self.notify_settings().await?.content != Content::None)
    }
}

/// The running app's own notifications (a computer: no push, the app is up).
impl MessengerRuntime {
    /// A notice the session raised, worded as a push would be: the sender's
    /// face looked up, the settings applied. `locked`: a PIN guards the app.
    pub async fn live_notice(&self, notice: &messenger_core::Notice, locked: bool) -> Result<Outcome> {
        let settings = self.notify_settings().await?;
        let face = self.face_of_notice(notice).await?;
        Ok(messenger_notify::live(notice, face, &settings, locked))
    }

    async fn face_of_notice(&self, notice: &messenger_core::Notice) -> Result<Face> {
        if let Some(chat) = notice.chat_id.as_deref().filter(|c| c.starts_with("dm:")) {
            if let Some(view) = self.dm().chat(chat).await? {
                return Ok(Face { name: view.title, picture: view.picture });
            }
        }
        match notice.sender.as_deref().and_then(PubKey::parse) {
            Some(pk) => {
                let (name, picture) = self.contacts().face_of(&pk).await?;
                Ok(Face { name, picture })
            }
            None => Ok(Face { name: notice.title.clone(), picture: None }),
        }
    }

    pub async fn desktop_notify_settings(&self) -> Result<DesktopSettings> {
        DesktopSettings::load(self.store()).await
    }

    pub async fn desktop_notify_set_settings(&self, settings: DesktopSettings) -> Result<DesktopSettings> {
        settings.save(self.store()).await?;
        Ok(settings)
    }
}
