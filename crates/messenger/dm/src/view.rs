// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Host-facing shapes. Plain data, no secrets.

pub use messenger_contacts::CardView;
use messenger_contacts::ContactCard;
use messenger_store::messages::{MessageRow, CT_CONTACT};
pub use messenger_store::reactions::ReactionView;
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
    /// While `queued`: when it was put in the outbox (or put back by a
    /// retry). The app shows it as sent for a moment counted from here.
    pub queued_at: Option<i64>,
    /// Mine only: when a device of the peer said it has the message, or
    /// the peer read past it. Groups have no delivery receipts: a read is
    /// the first word of it there.
    pub delivered_at: Option<i64>,
    /// Mine only: the read mark that covers it: the peer's in a direct
    /// chat, the newest of the members' in a group. `None` while read
    /// receipts are off.
    pub read_at: Option<i64>,
    /// Mine in a group: who has read it (hex keys). Empty while read
    /// receipts are off.
    #[serde(default)]
    pub seen_by: Vec<String>,
    /// What stands under the message, one entry per emoji in the order the
    /// emoji first came. Empty on a deleted message.
    #[serde(default)]
    pub reactions: Vec<ReactionView>,
    /// A contact card (`content_type` `contact`), as the UI shows it; its
    /// `media` is then empty.
    #[serde(default)]
    pub card: Option<CardView>,
}

impl MessageView {
    pub fn from_row(r: MessageRow, reply_to: Option<ReplyPreview>) -> Self {
        let is_card = r.content_type == CT_CONTACT;
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
            media: r.media_json.filter(|_| !is_card).and_then(|j| serde_json::from_str(&j).ok()),
            queued_at: None,
            delivered_at: None,
            read_at: None,
            seen_by: vec![],
            reactions: vec![],
            card: None,
        }
    }
}

/// The line of a checked contact card for chat lists: the name it shows
/// (one line already), as the store writes it when it looks again
/// (`chats::recompute_last`).
pub fn card_line(card: &ContactCard) -> String {
    format!("👤 {}", card.display_name.as_deref().or(card.name.as_deref()).unwrap_or_default())
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
