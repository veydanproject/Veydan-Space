// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The keys the network bans (`banned_keys` of the manifest,
//! internal/messenger-wire.md §1) follow the manifest in use.
//!
//! The manifest is the truth: the signed event stored as it came
//! (`relays::KEY_SIGNED`) is read again at every start, so a build that
//! learns the field finds it in a copy an older build kept, and no copy
//! stripped of it is ever taken. What the manifest names is written into
//! `msg_banned_keys`, where every gate asks (`DmService::is_banned`) and
//! where the predicates of the store look; this side only keeps the two
//! in step and tells the screens of the chats that changed.

use crate::presence::UI_EVENT_PRESENCE_KEYS_CHANGED;
use crate::MessengerRuntime;
use messenger_core::traits::UiEvent;
use messenger_core::{PubKey, Result};
use messenger_dm::UI_EVENT_DM_UPDATED;
use messenger_store::{banned, chats};

impl MessengerRuntime {
    /// Make the banned keys those of the manifest in use. At the start and
    /// whenever the manifest may have changed (`servers_changed`). A key
    /// that came or went: its direct chat is told its mode again, the
    /// groups it wrote in count and show without it (or with it again),
    /// the queue of requests is counted again, and, for a contact that
    /// was told my presence key, the key moves on. Nothing of what the
    /// device holds is deleted, and no relation is changed: a lifted ban
    /// leaves the person as the user had them.
    ///
    /// The groups the manifest closes follow it the same way
    /// ([`Self::banned_groups_follow`]).
    pub(crate) async fn banned_follow_manifest(&self) -> Result<()> {
        let (manifest, _) = self.relays.current_manifest().await?;
        self.banned_groups_follow(&manifest.banned_groups).await?;
        let keys: Vec<String> = manifest.banned().iter().map(|k| k.as_hex().to_string()).collect();
        let changed = banned::replace(&self.store, &keys).await?;
        if changed.is_empty() {
            return Ok(());
        }
        let mut events = Vec::new();
        let mut contact_banned = false;
        // A ban is of the person: every direct chat with them reads as
        // blocked, the private ones too.
        let mut servers: Vec<Option<String>> = vec![None];
        servers.extend(crate::scopes::private_servers(&self.store).await.into_iter().map(Some));
        for hex in &changed {
            let Some(peer) = PubKey::parse(hex) else { continue };
            for chat_id in chats::group_chats_follow_block(&self.store, hex).await? {
                events.push(UiEvent { name: UI_EVENT_DM_UPDATED.into(), payload: serde_json::json!({ "chat_id": chat_id }) });
            }
            for server in &servers {
                let chat = messenger_dm::DmPeer::on(peer.clone(), server.as_deref());
                let chat_id = chat.chat_id();
                if chats::get(&self.store, &chat_id).await?.is_some() {
                    events.push(self.dm.relationship_event(&chat_id, &chat).await?);
                    events.push(UiEvent { name: UI_EVENT_DM_UPDATED.into(), payload: serde_json::json!({ "chat_id": chat_id }) });
                }
            }
            if self.contacts.is_contact(&peer).await? && self.dm.is_banned(&peer).await? {
                contact_banned = true;
            }
        }
        // Somebody now banned may have been told my presence key: it moves
        // on, as after a block of mine, and the new one goes to the others.
        if contact_banned {
            if let Err(e) = self.presence_rotate().await {
                eprintln!("messenger presence: key not moved after a ban: {e}");
            }
            events.push(UiEvent { name: UI_EVENT_PRESENCE_KEYS_CHANGED.into(), payload: serde_json::json!({}) });
        }
        if let Some(peer) = changed.first().and_then(|h| PubKey::parse(h)) {
            events.push(self.dm.request_event(&peer).await?);
        }
        for ev in events {
            let _ = self.ui.send(ev);
        }
        let _ = self.ui.send(UiEvent { name: "chats.updated".into(), payload: serde_json::json!({}) });
        Ok(())
    }

    /// Make the closed groups those of the manifest in use. A group that
    /// came or went: its chat and its page are told again (its chat reads
    /// as closed, `ChatView::banned`, or open again), and the groups
    /// listened to are chosen anew: a closed one is not listened to, and
    /// one opened again is, its history asked for from where it stopped.
    /// Nothing the device holds of the group is deleted.
    pub(crate) async fn banned_groups_follow(&self, groups: &[String]) -> Result<()> {
        let changed = banned::group_replace(&self.store, groups).await?;
        if changed.is_empty() {
            return Ok(());
        }
        for group_id in &changed {
            let chat_id = messenger_store::groups::group_chat_id(group_id);
            let _ = self.ui.send(UiEvent {
                name: messenger_groups::service::UI_EVENT_GROUP_UPDATED.into(),
                payload: serde_json::json!({ "group_id": group_id, "chat_id": chat_id }),
            });
            let _ = self.ui.send(UiEvent { name: UI_EVENT_DM_UPDATED.into(), payload: serde_json::json!({ "chat_id": chat_id }) });
        }
        let _ = self.ui.send(UiEvent { name: "chats.updated".into(), payload: serde_json::json!({}) });
        self.group_driver.resubscribe().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::MessengerRuntime;
    use messenger_core::{MessengerConfig, MessengerError};
    use messenger_testkit::MemorySecretStore;
    use std::sync::Arc;

    /// The manifest closes a group: its chat stays and reads as closed,
    /// nothing can be written, it is not listened to; the next manifest
    /// without it opens it again.
    #[tokio::test]
    async fn a_group_the_manifest_closes_reads_as_closed_until_the_next_one() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let rt = MessengerRuntime::start(cfg, Arc::new(MemorySecretStore::unlocked())).await.unwrap();
        rt.relays().set_silent(true).await.unwrap();
        crate::servers::use_veydan_offline(&rt).await;
        rt.identity().create("pw").await.unwrap();
        assert!(rt.refresh_signer().await.unwrap());
        let g = rt.group_create(messenger_groups::GroupKind::Private, "Den", "", true, None).await.unwrap();
        rt.group_send_text(&g.id, "before", None).await.unwrap();
        let chat = || async { rt.chat(&g.chat_id).await.unwrap().expect("the chat of the group") };
        assert!(!chat().await.banned && chat().await.can_send);

        let (mut manifest, _) = rt.relays().current_manifest().await.unwrap();
        manifest.serial += 1;
        manifest.banned_groups = vec![g.id.clone()];
        rt.relays().adopt_for_test(&manifest, true).await.unwrap();
        let mut events = rt.ui_events();
        rt.banned_follow_manifest().await.unwrap();
        let mut told = Vec::new();
        while let Ok(ev) = events.try_recv() {
            told.push(ev.name);
        }
        assert!(told.iter().any(|n| n == messenger_groups::service::UI_EVENT_GROUP_UPDATED), "{told:?}");
        assert!(told.iter().any(|n| n == "chats.updated"), "{told:?}");
        assert!(rt.groups().is_group_banned(&g.id).await.unwrap());
        let c = chat().await;
        assert!(c.banned && !c.can_send, "{c:?}");
        assert_eq!(c.last_preview.as_deref(), Some("before"), "what the chat holds stays");
        assert!(rt.chats(false).await.unwrap().iter().any(|c| c.id == g.chat_id && c.banned), "and so it is listed");
        assert!(!rt.group_get(&g.id).await.unwrap().can_post);
        assert!(
            matches!(rt.group_send_text(&g.id, "during", None).await, Err(MessengerError::Invalid(ref w)) if w == "group_banned")
        );
        assert!(
            matches!(rt.group_history(&g.id, i64::MAX, 50).await, Err(MessengerError::Invalid(ref w)) if w == "group_banned")
        );
        assert!(!rt.groups().watched().await.unwrap().contains(&g.id));

        manifest.serial += 1;
        manifest.banned_groups.clear();
        rt.relays().adopt_for_test(&manifest, true).await.unwrap();
        rt.banned_follow_manifest().await.unwrap();
        let c = chat().await;
        assert!(!c.banned && c.can_send);
        assert!(rt.groups().watched().await.unwrap().contains(&g.id));
        rt.group_send_text(&g.id, "after", None).await.unwrap();
        rt.shutdown().await;
    }
}
