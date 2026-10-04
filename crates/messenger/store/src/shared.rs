// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a chat has shared, by section: pictures and videos, files, links,
//! voice. Read from `msg_messages`; nothing is stored twice.
//!
//! Links are found roughly here (a text that holds a scheme); what in it
//! is a link, and what that link means, is decided above the store.

use crate::messages::{MessageRow, CT_MEDIA, CT_TEXT};
use crate::{storage, Store};
use messenger_core::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    /// Pictures and videos.
    Visual,
    /// Files and music.
    Files,
    /// Texts and captions with a link in them.
    Links,
    /// Recorded in the app: voice and round videos.
    Voice,
}

impl Section {
    pub const ALL: [Section; 4] = [Section::Visual, Section::Files, Section::Links, Section::Voice];

    /// Media kinds of the section; empty for links.
    pub fn kinds(self) -> &'static [&'static str] {
        match self {
            Section::Visual => &["image", "video"],
            Section::Files => &["file", "audio"],
            Section::Voice => &["voice", "circle"],
            Section::Links => &[],
        }
    }
}

/// How many messages each section holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub visual: i64,
    pub files: i64,
    pub links: i64,
    pub voice: i64,
}

const COLS: &str = "id, chat_id, wire_id, direction, status, content_type, text, envelope_json, sender_pubkey, reply_to_id, target_id, created_at, received_at, edited_at, deleted_at, is_hidden, outbox_local_id, failure_reason, media_json";

/// Seen, not removed.
const LIVE: &str = "chat_id = ? AND is_hidden = 0 AND deleted_at IS NULL";

/// A text that may hold a link; the UI decides which part of it is one.
const HAS_LINK: &str = "(text LIKE '%http://%' OR text LIKE '%https://%' OR text LIKE '%veydan://%' OR text LIKE '%npub1%')";

fn kind_in(section: Section) -> String {
    let list = section.kinds().iter().map(|k| format!("'{k}'")).collect::<Vec<_>>().join(", ");
    format!("json_extract(media_json, '$.kind') IN ({list})")
}

fn condition(section: Section) -> String {
    match section {
        Section::Links => format!("content_type IN ('{CT_TEXT}', '{CT_MEDIA}') AND {HAS_LINK}"),
        s => format!("content_type = '{CT_MEDIA}' AND {}", kind_in(s)),
    }
}

pub async fn counts(store: &Store, chat_id: &str) -> Result<Counts> {
    let sum = |s: Section| format!("COALESCE(SUM(CASE WHEN {} THEN 1 ELSE 0 END), 0)", condition(s));
    let sql = format!(
        "SELECT {}, {}, {}, {} FROM msg_messages WHERE {LIVE}",
        sum(Section::Visual),
        sum(Section::Files),
        sum(Section::Links),
        sum(Section::Voice),
    );
    let (visual, files, links, voice) = sqlx::query_as::<_, (i64, i64, i64, i64)>(sqlx::AssertSqlSafe(sql))
        .bind(chat_id)
        .fetch_one(store.pool())
        .await
        .map_err(storage)?;
    Ok(Counts { visual, files, links, voice })
}

/// Messages of one section, newest first, at most `limit`, all strictly
/// older than `before` (created_at) when given.
pub async fn list(store: &Store, chat_id: &str, section: Section, before: Option<i64>, limit: i64) -> Result<Vec<MessageRow>> {
    let sql = format!(
        "SELECT {COLS} FROM msg_messages WHERE {LIVE} AND {} AND created_at < ?
         ORDER BY created_at DESC, id DESC LIMIT ?",
        condition(section)
    );
    sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(sql))
        .bind(chat_id)
        .bind(before.unwrap_or(i64::MAX))
        .bind(limit)
        .fetch_all(store.pool())
        .await
        .map_err(storage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{insert, mark_deleted, NewMessage, CT_EDIT, CT_SYSTEM, DIR_IN, STATUS_RECEIVED};

    fn row(id: &str, chat: &str, at: i64, content_type: &str, text: Option<&str>, kind: Option<&str>) -> NewMessage {
        NewMessage {
            id: id.into(),
            chat_id: chat.into(),
            wire_id: None,
            direction: DIR_IN.into(),
            status: STATUS_RECEIVED.into(),
            content_type: content_type.into(),
            text: text.map(String::from),
            envelope_json: "{}".into(),
            sender_pubkey: "s".into(),
            reply_to_id: None,
            target_id: None,
            created_at: at,
            is_hidden: false,
            outbox_local_id: None,
            media_json: kind.map(|k| format!(r#"{{"kind":"{k}","name":"n","mime":"x/y","size":1}}"#)),
        }
    }

    async fn fill(s: &Store, chat: &str) {
        let rows = [
            row("img", chat, 10, CT_MEDIA, None, Some("image")),
            row("vid", chat, 20, CT_MEDIA, Some("see https://a.example"), Some("video")),
            row("pdf", chat, 30, CT_MEDIA, None, Some("file")),
            row("mp3", chat, 40, CT_MEDIA, None, Some("audio")),
            row("voice", chat, 50, CT_MEDIA, None, Some("voice")),
            row("circle", chat, 60, CT_MEDIA, None, Some("circle")),
            row("web", chat, 70, CT_TEXT, Some("look: https://b.example/x"), None),
            row("group", chat, 80, CT_TEXT, Some("veydan://group/abc"), None),
            row("key", chat, 90, CT_TEXT, Some("nostr:npub1qqq"), None),
            row("plain", chat, 100, CT_TEXT, Some("just words"), None),
            row("sys", chat, 110, CT_SYSTEM, Some("https://not.shared"), None),
        ];
        for mut r in rows {
            // Ids are global: another chat's copy is another message.
            if chat != "c" {
                r.id = format!("{chat}/{}", r.id);
            }
            insert(s, &r).await.unwrap();
        }
    }

    fn ids(rows: &[MessageRow]) -> Vec<&str> {
        rows.iter().map(|r| r.id.as_str()).collect()
    }

    #[tokio::test]
    async fn sections_count_and_list_their_own() {
        let s = Store::open_in_memory().await.unwrap();
        fill(&s, "c").await;
        fill(&s, "group:g").await;

        assert_eq!(counts(&s, "c").await.unwrap(), Counts { visual: 2, files: 2, links: 4, voice: 2 });
        assert_eq!(counts(&s, "group:g").await.unwrap(), counts(&s, "c").await.unwrap(), "a group chat is a chat");
        assert_eq!(counts(&s, "empty").await.unwrap(), Counts::default());

        let at = |section| list(&s, "c", section, None, 50);
        assert_eq!(ids(&at(Section::Visual).await.unwrap()), vec!["vid", "img"], "newest first");
        assert_eq!(ids(&at(Section::Files).await.unwrap()), vec!["mp3", "pdf"]);
        assert_eq!(ids(&at(Section::Voice).await.unwrap()), vec!["circle", "voice"]);
        assert_eq!(ids(&at(Section::Links).await.unwrap()), vec!["key", "group", "web", "vid"], "captions count, system lines do not");
    }

    #[tokio::test]
    async fn hidden_and_deleted_are_not_shared() {
        let s = Store::open_in_memory().await.unwrap();
        fill(&s, "c").await;
        mark_deleted(&s, "img", 200).await.unwrap();
        mark_deleted(&s, "web", 200).await.unwrap();
        let mut edit = row("edit", "c", 120, CT_EDIT, Some("https://c.example"), None);
        edit.is_hidden = true;
        insert(&s, &edit).await.unwrap();

        let c = counts(&s, "c").await.unwrap();
        assert_eq!((c.visual, c.links), (1, 3));
        assert_eq!(ids(&list(&s, "c", Section::Visual, None, 50).await.unwrap()), vec!["vid"]);
    }

    #[tokio::test]
    async fn pages_go_back_in_time() {
        let s = Store::open_in_memory().await.unwrap();
        for i in 1..=5 {
            insert(&s, &row(&format!("p{i}"), "c", i * 10, CT_MEDIA, None, Some("image"))).await.unwrap();
        }
        let first = list(&s, "c", Section::Visual, None, 2).await.unwrap();
        assert_eq!(ids(&first), vec!["p5", "p4"]);
        let next = list(&s, "c", Section::Visual, Some(first[1].created_at), 2).await.unwrap();
        assert_eq!(ids(&next), vec!["p3", "p2"]);
    }
}
