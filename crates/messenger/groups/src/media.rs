// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Attachments in a group. As in a chat of two, the module does not
//! know how blobs are stored: while a file is uploading the group shows
//! a placeholder row, and once the blob is stored the placeholder is
//! replaced by the real message. The file key travels inside the
//! message, under the group key.

use crate::service::{me_of, GroupService, MEMBERSHIP_JOINED};
use messenger_core::{Envelope, MessengerError, Outbound, Result};
use messenger_dm::MessageView;
use messenger_store::groups as repo;
use messenger_store::messages::{self as msgs, NewMessage};
use messenger_store::chats;
use nostr::key::Keys;
use std::sync::atomic::{AtomicU64, Ordering};

fn placeholder_id() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("local:g{t:x}-{:x}", N.fetch_add(1, Ordering::Relaxed))
}

impl GroupService {
    /// A row for a file that is about to be uploaded. Fails when I may
    /// not write into the group now.
    pub async fn media_placeholder(
        &self,
        keys: &Keys,
        group_id: &str,
        media_json: serde_json::Value,
        caption: Option<&str>,
    ) -> Result<MessageView> {
        let me = me_of(keys);
        let row = repo::get(&self.store, group_id).await?.ok_or_else(|| MessengerError::Invalid("group_unknown".into()))?;
        let log = self.need_log(group_id).await?;
        if row.membership != MEMBERSHIP_JOINED || !log.state().can_post(&me) {
            let muted = log.state().member(&me).is_some_and(|m| m.muted);
            return Err(MessengerError::Invalid(if muted { "group_muted".into() } else { "group_not_member".into() }));
        }
        let chat_id = repo::group_chat_id(group_id);
        let id = placeholder_id();
        let now = self.now();
        let created_at = match msgs::last_created_at(&self.store, &chat_id).await? {
            Some(last) if last >= now => last + 1,
            _ => now,
        };
        msgs::insert(
            &self.store,
            &NewMessage {
                id: id.clone(),
                chat_id: chat_id.clone(),
                wire_id: None,
                direction: msgs::DIR_OUT.into(),
                status: msgs::STATUS_UPLOADING.into(),
                content_type: msgs::CT_MEDIA.into(),
                text: caption.map(str::trim).filter(|c| !c.is_empty()).map(String::from),
                envelope_json: "{}".into(),
                sender_pubkey: me.as_hex().to_string(),
                reply_to_id: None,
                target_id: None,
                created_at,
                is_hidden: false,
                outbox_local_id: None,
                media_json: Some(media_json.to_string()),
            },
        )
        .await?;
        let name = media_json.get("name").and_then(|v| v.as_str()).unwrap_or("file");
        chats::touch(&self.store, &chat_id, created_at, Some(&format!("📎 {name}")), false).await?;
        self.dm.message(&id).await?.ok_or_else(|| MessengerError::Storage("placeholder vanished".into()))
    }

    /// The blob is stored: replace the placeholder by the real message.
    /// `media_json` is what this device keeps (descriptor plus local path).
    pub async fn media_finish(
        &self,
        keys: &Keys,
        placeholder: &str,
        envelope: Envelope,
        media_json: serde_json::Value,
    ) -> Result<(MessageView, Outbound)> {
        let row = msgs::get(&self.store, placeholder)
            .await?
            .ok_or_else(|| MessengerError::Invalid("the upload was removed".into()))?;
        let group_id = row
            .chat_id
            .strip_prefix("group:")
            .ok_or_else(|| MessengerError::Invalid("not a group message".into()))?
            .to_string();
        msgs::delete(&self.store, placeholder).await?;
        let result = self
            .prepare_message(keys, &group_id, envelope, msgs::CT_MEDIA, row.text.clone(), None, Some(media_json.to_string()))
            .await;
        if let Err(e) = &result {
            // Put the placeholder back so the user sees what failed.
            msgs::insert(
                &self.store,
                &NewMessage {
                    id: row.id.clone(),
                    chat_id: row.chat_id.clone(),
                    wire_id: None,
                    direction: row.direction.clone(),
                    status: msgs::STATUS_FAILED.into(),
                    content_type: row.content_type.clone(),
                    text: row.text.clone(),
                    envelope_json: row.envelope_json.clone(),
                    sender_pubkey: row.sender_pubkey.clone(),
                    reply_to_id: None,
                    target_id: None,
                    created_at: row.created_at,
                    is_hidden: false,
                    outbox_local_id: None,
                    media_json: row.media_json.clone(),
                },
            )
            .await?;
            msgs::set_status(&self.store, &row.id, msgs::STATUS_FAILED, Some(&e.to_string())).await?;
        }
        result
    }
}
