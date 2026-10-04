// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Attachments in a DM chat. The DM module does not know how blobs are
//! stored: it gets a ready envelope from the host and only keeps the
//! message rows. While a file is uploading the chat shows a placeholder
//! row; once the blob is stored the placeholder is replaced by the real
//! message.

use crate::service::{DmService, Prepared};
use crate::view::MessageView;
use messenger_core::{Envelope, MessengerError, PubKey, Result};
use messenger_store::chats;
use messenger_store::messages::{self as repo, NewMessage};
use nostr::key::Keys;
use std::sync::atomic::{AtomicU64, Ordering};

fn placeholder_id() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("local:{t:x}-{:x}", N.fetch_add(1, Ordering::Relaxed))
}

impl DmService {
    /// A row for a file that is about to be uploaded. Fails with the
    /// relationship reason when media may not be sent to this peer now.
    pub async fn media_placeholder(
        &self,
        keys: &Keys,
        peer: &PubKey,
        media_json: serde_json::Value,
        caption: Option<&str>,
    ) -> Result<MessageView> {
        let me = keys.public_key().to_hex();
        let chat = chats::ensure_dm(&self.store, peer.as_hex()).await?;
        self.gate_outbound(&chat.id, peer, &me, false).await?;
        let id = placeholder_id();
        let created_at = {
            let now = self.clock.now().secs();
            match repo::last_created_at(&self.store, &chat.id).await? {
                Some(last) if last >= now => last + 1,
                _ => now,
            }
        };
        repo::insert(
            &self.store,
            &NewMessage {
                id: id.clone(),
                chat_id: chat.id.clone(),
                wire_id: None,
                direction: repo::DIR_OUT.into(),
                status: repo::STATUS_UPLOADING.into(),
                content_type: repo::CT_MEDIA.into(),
                text: caption.map(str::trim).filter(|c| !c.is_empty()).map(String::from),
                envelope_json: "{}".into(),
                sender_pubkey: me,
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
        chats::touch(&self.store, &chat.id, created_at, Some(&format!("📎 {name}")), false).await?;
        self.message(&id).await?.ok_or_else(|| MessengerError::Storage("placeholder vanished".into()))
    }

    /// The blob is stored: replace the placeholder by the real message.
    /// `media_json` is what this device keeps (descriptor plus local path).
    pub async fn media_finish(
        &self,
        keys: &Keys,
        placeholder: &str,
        envelope: Envelope,
        media_json: serde_json::Value,
    ) -> Result<Prepared> {
        let row = repo::get(&self.store, placeholder)
            .await?
            .ok_or_else(|| MessengerError::Invalid("the upload was removed".into()))?;
        let chat = chats::get(&self.store, &row.chat_id).await?.ok_or_else(|| MessengerError::Storage("chat missing".into()))?;
        let peer = chat
            .peer_pubkey
            .as_deref()
            .and_then(PubKey::parse)
            .ok_or_else(|| MessengerError::Storage("chat has no peer".into()))?;
        repo::delete(&self.store, placeholder).await?;
        let name = media_json.get("name").and_then(|v| v.as_str()).unwrap_or("file").to_string();
        let result = self
            .prepare_visible(keys, &peer, envelope, repo::CT_MEDIA, row.text.clone(), None, Some(media_json.to_string()))
            .await;
        match result {
            Ok(p) => {
                let line = match &row.text {
                    Some(c) => format!("📎 {c}"),
                    None => format!("📎 {name}"),
                };
                chats::touch(&self.store, &row.chat_id, p.message.created_at, Some(&line), false).await?;
                Ok(p)
            }
            Err(e) => {
                // Put the placeholder back so the user sees what failed.
                let mut back = NewMessage {
                    id: row.id.clone(),
                    chat_id: row.chat_id.clone(),
                    wire_id: None,
                    direction: row.direction.clone(),
                    status: repo::STATUS_FAILED.into(),
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
                };
                back.status = repo::STATUS_FAILED.into();
                repo::insert(&self.store, &back).await?;
                repo::set_status(&self.store, &row.id, repo::STATUS_FAILED, Some(&e.to_string())).await?;
                Err(e)
            }
        }
    }

    pub async fn media_set_status(&self, message_id: &str, status: &str, reason: Option<&str>) -> Result<()> {
        repo::set_status(&self.store, message_id, status, reason).await
    }

    /// Remove a placeholder (cancelled upload).
    pub async fn media_discard(&self, placeholder: &str) -> Result<()> {
        if let Some(row) = repo::get(&self.store, placeholder).await? {
            if !row.id.starts_with("local:") {
                return Err(MessengerError::Invalid("not an upload placeholder".into()));
            }
            repo::delete(&self.store, placeholder).await?;
            chats::recompute_last(&self.store, &row.chat_id).await?;
        }
        Ok(())
    }

    /// Remember where the file of a message lives on this device.
    pub async fn media_set_local_path(&self, message_id: &str, path: &str) -> Result<()> {
        let Some(row) = repo::get(&self.store, message_id).await? else { return Ok(()) };
        let mut v: serde_json::Value =
            row.media_json.as_deref().and_then(|j| serde_json::from_str(j).ok()).unwrap_or_else(|| serde_json::json!({}));
        v["local_path"] = serde_json::Value::String(path.to_string());
        repo::set_media_json(&self.store, message_id, &v.to_string()).await
    }
}
