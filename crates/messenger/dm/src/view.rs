// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Host-facing shapes. Plain data, no secrets.

pub use messenger_contacts::CardView;
use messenger_contacts::ContactCard;
use crate::botcard::BotCard;
use messenger_store::messages::{MessageRow, CT_BOT_CARD, CT_CONTACT, DIR_IN};
pub use messenger_store::reactions::ReactionView;
use messenger_core::TextFormat;
pub use messenger_richtext::md::{self, MdBlock};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatView {
    pub id: String,
    pub kind: String,
    /// The private server of a private chat (internal/messenger-wire.md,
    /// «Частный чат»); absent for the home set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
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
    /// What the newest message is, in facts (`messenger_core::Body`): the UI
    /// words it in the user's language as a push does ("📷 Photo", "🎤 Voice
    /// message (0:12)"), where `last_preview` names a file. Absent when it
    /// cannot be said (a card of the bot, a message the line no longer
    /// speaks of).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_body: Option<messenger_core::Body>,
    pub pinned: bool,
    pub archived: bool,
    /// Relationship screen mode (stage 5b); `full_chat` while the gate is off.
    pub mode: String,
    /// Whether the composer may be used right now.
    pub can_send: bool,
    /// The chat with the pinned bot of Veydan Cloud (`DmService::with_bot`):
    /// a full chat where nothing but messages goes, so the UI offers no
    /// call, no reaction and no «add to contacts» in it.
    #[serde(default)]
    pub bot: bool,
    /// The network bans the peer of a direct chat (`DmService::is_banned`):
    /// `mode` is `blocked`, and the screen may say that the person was
    /// removed from the network. Of a `group:` chat: the network closes
    /// the group (`GroupService::is_group_banned`, set by the runtime):
    /// `can_send` is false, and the screen may say that the group was
    /// removed from the network. What the chat holds stays, as it was.
    #[serde(default)]
    pub banned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyPreview {
    pub id: String,
    pub sender_pubkey: String,
    pub text: Option<String>,
    /// Taken back for everyone. A quoted photo has no text either, so
    /// "no text" alone cannot mean "deleted".
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub content_type: String,
    /// Of a quoted media message that is not deleted: `kind`, `name`,
    /// `duration_ms`, enough to say "Photo" or "Voice message".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageView {
    pub id: String,
    pub chat_id: String,
    /// `in` | `out`
    pub direction: String,
    /// `queued` | `sent` | `failed` | `received`
    pub status: String,
    /// `text` | `system` | `media` | `contact` | `bot_card` | `unknown`
    /// (any other type of the wire; its name stays in the envelope)
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
    /// A card of the bot of Veydan Cloud (`content_type` `bot_card`): its
    /// type and the values the app words it from; `text` and `media` are
    /// then empty. Only on a message the pinned key sealed.
    #[serde(default)]
    pub bot_card: Option<BotCard>,
    /// In a group: `ok`, `pending` (waits for a moderator: listed for the
    /// moderators and for its author only, counted nowhere, fetched
    /// nowhere) or `rejected`. `ok` in a direct chat, and for what a
    /// moderator approved. A message hidden by a moderator is never
    /// listed, so nothing says it is.
    #[serde(default = "review_ok")]
    pub review: String,
    /// What an approval names while the message waits (`pending`): the id
    /// of the edit in force, else the message's own id
    /// (`messages::version_of`). The UI cannot tell it, since the edits
    /// are rows it never sees; absent on every other message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// `md` when `text` is written in Markdown; absent for plain text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fmt: Option<String>,
    /// Of a Markdown `text`: its blocks, to be rendered as elements and
    /// text nodes. Absent when the text is plain, or too long or too
    /// many pieces to be shown as Markdown (`md::parse`): it is then
    /// shown as plain text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rich: Option<Vec<MdBlock>>,
}

fn review_ok() -> String {
    "ok".into()
}

impl MessageView {
    pub fn from_row(r: MessageRow, reply_to: Option<ReplyPreview>) -> Self {
        let is_card = r.content_type == CT_CONTACT;
        // The row of a card is written for the pinned key alone
        // (`DmService::apply_inbound`), in its direct chat, and always with
        // the card it read. Anything else under that type is no card: a
        // build before the cards kept the type of the wire as it came, so
        // a row without a card is a stranger's `{"t":"bot_card"}` and stays
        // a message of a type this app does not know.
        let is_bot_card = r.content_type == CT_BOT_CARD
            && r.direction == DIR_IN
            && messenger_store::chats::parse_dm_chat_id(&r.chat_id) == Some((r.sender_pubkey.as_str(), None));
        let bot_card = if is_bot_card { r.media_json.as_deref().map(BotCard::stored) } else { None };
        // What `media_json` holds of a card is not a file.
        let typed = is_card || r.content_type == CT_BOT_CARD;
        let markdown = TextFormat::from_name(Some(&r.fmt)).is_markdown();
        let rich = if markdown { r.text.as_deref().and_then(md::parse) } else { None };
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
            media: r.media_json.filter(|_| !typed).and_then(|j| serde_json::from_str(&j).ok()),
            queued_at: None,
            delivered_at: None,
            read_at: None,
            seen_by: vec![],
            reactions: vec![],
            card: None,
            bot_card,
            // The page tells `approved` from `ok` by nothing: both are shown.
            review: if r.review == "pending" || r.review == "rejected" { r.review } else { "ok".into() },
            version: None,
            fmt: markdown.then(|| TextFormat::Markdown.name().to_string()),
            rich,
        }
    }
}

/// The line of a checked contact card for chat lists: the name it shows
/// (one line already), as the store writes it when it looks again
/// (`chats::recompute_last`).
pub fn card_line(card: &ContactCard) -> String {
    format!("👤 {}", card.display_name.as_deref().or(card.name.as_deref()).unwrap_or_default())
}

/// The words of a text, an edit or a caption as one line: a Markdown one
/// without its marks.
pub fn preview_of(text: &str, format: TextFormat) -> String {
    let plain = if format.is_markdown() { md::plain(text) } else { String::new() };
    // Marks and nothing else (`---`): the text as it was typed.
    preview(if plain.is_empty() { text } else { &plain })
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
