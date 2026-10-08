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
    /// With the notifications set to show no content, a bundle for calls
    /// alone (`KeyBundle::for_calls_only`): nothing of a message is
    /// opened, but a phone without keys would never ring. Whether the
    /// handler gets any bundle at all is the lock's word, not this one's
    /// (`messenger-app::commands::push`).
    pub async fn notify_bundle(&self) -> Result<Option<KeyBundle>> {
        let Ok(keys) = self.session_keys().await else {
            return Ok(None);
        };
        if self.notify_calls_only().await? {
            return Ok(Some(KeyBundle::for_calls_only(&keys)));
        }
        let groups = self.groups().export_keys().await?.iter().map(|(g, k)| GroupKeyEntry::of(g, k)).collect();
        Ok(Some(KeyBundle::new(&keys, groups)))
    }

    /// Changes when `notify_bundle` would; costs no secret.
    pub async fn notify_fingerprint(&self) -> Result<Option<String>> {
        let Ok(keys) = self.session_keys().await else {
            return Ok(None);
        };
        let me = keys.public_key().to_hex();
        if self.notify_calls_only().await? {
            return Ok(Some(format!("{me}|calls")));
        }
        Ok(Some(format!("{}|{}", me, self.groups().keys_fingerprint().await?)))
    }

    async fn notify_calls_only(&self) -> Result<bool> {
        Ok(self.notify_settings().await?.content == Content::None)
    }

    pub async fn notify_settings(&self) -> Result<Settings> {
        Settings::load(self.store()).await
    }

    pub async fn notify_set_settings(&self, settings: Settings) -> Result<Settings> {
        settings.save(self.store()).await?;
        Ok(settings)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::servers;
    use messenger_core::MessengerConfig;
    use messenger_testkit::MemorySecretStore;
    use std::sync::Arc;

    /// "No content" takes the keys of the groups from the handler and
    /// leaves it the identity's, for calls alone: a phone without keys
    /// would never ring. The fingerprint says so, so the handler is told.
    #[tokio::test]
    async fn no_content_gives_the_handler_a_bundle_for_calls_alone() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let rt = MessengerRuntime::start(cfg, Arc::new(MemorySecretStore::unlocked())).await.unwrap();
        rt.relays().set_silent(true).await.unwrap();
        servers::use_veydan_offline(&rt).await;
        assert!(rt.notify_bundle().await.unwrap().is_none(), "no session, no keys");
        assert!(rt.notify_fingerprint().await.unwrap().is_none());

        rt.identity().create("pw").await.unwrap();
        assert!(rt.refresh_signer().await.unwrap());
        let full = rt.notify_bundle().await.unwrap().unwrap();
        assert!(!full.calls_only, "the default shows the content");
        let full_print = rt.notify_fingerprint().await.unwrap().unwrap();

        rt.notify_set_settings(Settings { content: Content::None, lockscreen_hidden: false }).await.unwrap();
        let calls = rt.notify_bundle().await.unwrap().unwrap();
        assert!(calls.calls_only);
        assert!(calls.groups.is_empty());
        assert_eq!(calls.keys().unwrap().public_key(), full.keys().unwrap().public_key(), "the same identity");
        let calls_print = rt.notify_fingerprint().await.unwrap().unwrap();
        assert_ne!(calls_print, full_print, "the handler is told of the change");

        rt.notify_set_settings(Settings { content: Content::Sender, lockscreen_hidden: true }).await.unwrap();
        assert!(!rt.notify_bundle().await.unwrap().unwrap().calls_only);
        assert_eq!(rt.notify_fingerprint().await.unwrap().unwrap(), full_print);
        rt.shutdown().await;
    }
}
