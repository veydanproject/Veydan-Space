// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Host-facing shapes. Plain data, no secrets.

use messenger_store::messages::MessageRow;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatView {
    pub id: String,
    pub kind: String,
    pub peer_pubkey: Option<String>,
    pub peer_npub: Option<String>,
    /// Nickname → profile name → short npub.
    pub title: String,
    pub picture: Option<String>,
    pub is_contact: bool,
    pub is_muted: bool,
    pub unread: i64,
    pub last_message_at: Option<i64>,
    pub last_preview: Option<String>,
    pub pinned: bool,
    pub archived: bool,
    /// Relationship screen mode (stage 5b); `full_chat` while the gate is off.
    pub mode: String,
    /// Whether the composer may be used right now.
    pub can_send: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyPreview {
    pub id: String,
    pub sender_pubkey: String,
    pub text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageView {
    pub id: String,
    pub chat_id: String,
    /// `in` | `out`
    pub direction: String,
    /// `queued` | `sent` | `failed` | `received`
    pub status: String,
    /// `text` | `system` | `media` | anything newer clients invent
    pub content_type: String,
    pub text: Option<String>,
    pub sender_pubkey: String,
    pub reply_to: Option<ReplyPreview>,
    pub created_at: i64,
    pub edited_at: Option<i64>,
    pub deleted: bool,
    pub failure_reason: Option<String>,
    pub media: Option<serde_json::Value>,
}

impl MessageView {
    pub fn from_row(r: MessageRow, reply_to: Option<ReplyPreview>) -> Self {
        Self {
            id: r.id,
            chat_id: r.chat_id,
            direction: r.direction,
            status: r.status,
            content_type: r.content_type,
            text: r.text,
            sender_pubkey: r.sender_pubkey,
            reply_to,
            created_at: r.created_at,
            edited_at: r.edited_at,
            deleted: r.deleted_at.is_some(),
            failure_reason: r.failure_reason,
            media: r.media_json.and_then(|j| serde_json::from_str(&j).ok()),
        }
    }
}

/// One line for chat lists and notifications.
pub fn preview(text: &str) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 120 {
        let cut: String = flat.chars().take(119).collect();
        format!("{cut}…")
    } else {
        flat
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_flattens_and_truncates_on_char_boundaries() {
        assert_eq!(preview("a\n\n b\t c"), "a b c");
        let long = "я".repeat(300);
        let p = preview(&long);
        assert_eq!(p.chars().count(), 120);
        assert!(p.ends_with('…'));
    }
}
