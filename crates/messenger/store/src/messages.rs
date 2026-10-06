// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_messages`: every message row, visible or hidden. The primary key is
//! the rumor id, so the peer's copy, our self-copy and a second device's
//! copy of the same message all collapse into one row.

use crate::{storage, Store};
use messenger_core::Result;

pub const DIR_IN: &str = "in";
pub const DIR_OUT: &str = "out";

pub const STATUS_QUEUED: &str = "queued";
pub const STATUS_SENT: &str = "sent";
pub const STATUS_FAILED: &str = "failed";
pub const STATUS_RECEIVED: &str = "received";

pub const CT_TEXT: &str = "text";
pub const CT_EDIT: &str = "edit";
pub const CT_DELETE: &str = "delete";
pub const CT_CONTROL: &str = "control";
pub const CT_SYSTEM: &str = "system";
pub const CT_MEDIA: &str = "media";

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct MessageRow {
    pub id: String,
    pub chat_id: String,
    pub wire_id: Option<String>,
    pub direction: String,
    pub status: String,
    pub content_type: String,
    pub text: Option<String>,
    pub envelope_json: String,
    pub sender_pubkey: String,
    pub reply_to_id: Option<String>,
    pub target_id: Option<String>,
    pub created_at: i64,
    pub received_at: i64,
    pub edited_at: Option<i64>,
    pub deleted_at: Option<i64>,
    pub is_hidden: bool,
    pub outbox_local_id: Option<String>,
    pub failure_reason: Option<String>,
    pub media_json: Option<String>,
}

/// What a handler or the sender knows when inserting.
#[derive(Clone, Debug)]
pub struct NewMessage {
    pub id: String,
    pub chat_id: String,
    pub wire_id: Option<String>,
    pub direction: String,
    pub status: String,
    pub content_type: String,
    pub text: Option<String>,
    pub envelope_json: String,
    pub sender_pubkey: String,
    pub reply_to_id: Option<String>,
    pub target_id: Option<String>,
    pub created_at: i64,
    pub is_hidden: bool,
    pub outbox_local_id: Option<String>,
    pub media_json: Option<String>,
}

const COLS: &str = "id, chat_id, wire_id, direction, status, content_type, text, envelope_json, sender_pubkey, reply_to_id, target_id, created_at, received_at, edited_at, deleted_at, is_hidden, outbox_local_id, failure_reason, media_json";

/// Insert; `false` when a row with this id already exists (a duplicate copy).
pub async fn insert(store: &Store, m: &NewMessage) -> Result<bool> {
    let res = sqlx::query(
        "INSERT OR IGNORE INTO msg_messages (id, chat_id, wire_id, direction, status, content_type, text, envelope_json, sender_pubkey, reply_to_id, target_id, created_at, received_at, edited_at, deleted_at, is_hidden, outbox_local_id, failure_reason, media_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, ?, ?, NULL, ?)",
    )
    .bind(&m.id)
    .bind(&m.chat_id)
    .bind(&m.wire_id)
    .bind(&m.direction)
    .bind(&m.status)
    .bind(&m.content_type)
    .bind(&m.text)
    .bind(&m.envelope_json)
    .bind(&m.sender_pubkey)
    .bind(&m.reply_to_id)
    .bind(&m.target_id)
    .bind(m.created_at)
    .bind(crate::now())
    .bind(m.is_hidden)
    .bind(&m.outbox_local_id)
    .bind(&m.media_json)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(res.rows_affected() == 1)
}

pub async fn get(store: &Store, id: &str) -> Result<Option<MessageRow>> {
    sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!("SELECT {COLS} FROM msg_messages WHERE id = ?")))
        .bind(id)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)
}

/// Visible rows of a chat, oldest first, at most `limit`, all strictly
/// older than `before` (created_at) when given. Deleted rows are included
/// (rendered as tombstones); hidden rows never.
pub async fn list(store: &Store, chat_id: &str, before: Option<i64>, limit: i64) -> Result<Vec<MessageRow>> {
    let mut rows = sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM msg_messages WHERE chat_id = ? AND is_hidden = 0 AND created_at < ?
         ORDER BY created_at DESC, id DESC LIMIT ?"
    )))
    .bind(chat_id)
    .bind(before.unwrap_or(i64::MAX))
    .bind(limit)
    .fetch_all(store.pool())
    .await
    .map_err(storage)?;
    rows.reverse();
    Ok(rows)
}

/// Newest application time in the chat; new outgoing rumors must be later.
pub async fn last_created_at(store: &Store, chat_id: &str) -> Result<Option<i64>> {
    sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(created_at) FROM msg_messages WHERE chat_id = ?")
        .bind(chat_id)
        .fetch_one(store.pool())
        .await
        .map_err(storage)
}

/// Visible incoming rows in the chat (relationship matrix: "at most one
/// visible message before approval").
pub async fn count_visible_incoming(store: &Store, chat_id: &str) -> Result<i64> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM msg_messages WHERE chat_id = ? AND direction = 'in' AND is_hidden = 0 AND content_type != 'system'",
    )
    .bind(chat_id)
    .fetch_one(store.pool())
    .await
    .map_err(storage)
}

pub async fn count_visible(store: &Store, chat_id: &str) -> Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM msg_messages WHERE chat_id = ? AND is_hidden = 0")
        .bind(chat_id)
        .fetch_one(store.pool())
        .await
        .map_err(storage)
}

pub async fn set_text(store: &Store, id: &str, text: &str, edited_at: i64) -> Result<()> {
    sqlx::query("UPDATE msg_messages SET text = ?, edited_at = ? WHERE id = ?")
        .bind(text)
        .bind(edited_at)
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// Tombstone: content is dropped, the row stays so the order is stable.
pub async fn mark_deleted(store: &Store, id: &str, deleted_at: i64) -> Result<()> {
    sqlx::query("UPDATE msg_messages SET text = NULL, media_json = NULL, deleted_at = ? WHERE id = ?")
        .bind(deleted_at)
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn set_status(store: &Store, id: &str, status: &str, failure_reason: Option<&str>) -> Result<()> {
    sqlx::query("UPDATE msg_messages SET status = ?, failure_reason = ? WHERE id = ?")
        .bind(status)
        .bind(failure_reason)
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn set_outbox_local_id(store: &Store, id: &str, local_id: Option<&str>) -> Result<()> {
    sqlx::query("UPDATE msg_messages SET outbox_local_id = ? WHERE id = ?")
        .bind(local_id)
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// Hidden edit/delete rows that arrived before their target.
pub async fn pending_for_target(store: &Store, target_id: &str) -> Result<Vec<MessageRow>> {
    sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM msg_messages WHERE target_id = ? AND is_hidden = 1 ORDER BY created_at ASC"
    )))
    .bind(target_id)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// Ids of messages that changed status because their outbox row finished:
/// `(now sent, now failed)`. Out is sent, whatever was shown before; given
/// up (past its deadline, or refused) is failed. A row that is only being
/// retried leaves the message waiting.
pub async fn sync_outbox_status(store: &Store) -> Result<(Vec<String>, Vec<String>)> {
    let sent = sqlx::query_scalar::<_, String>(
        "SELECT m.id FROM msg_messages m JOIN msg_outbox o ON o.local_id = m.outbox_local_id
         WHERE m.status IN ('queued', 'failed') AND o.state = 'published'",
    )
    .fetch_all(store.pool())
    .await
    .map_err(storage)?;
    for id in &sent {
        set_status(store, id, STATUS_SENT, None).await?;
    }
    let failed = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT m.id, o.last_error FROM msg_messages m JOIN msg_outbox o ON o.local_id = m.outbox_local_id
         WHERE m.status = 'queued' AND o.state = 'abandoned'",
    )
    .fetch_all(store.pool())
    .await
    .map_err(storage)?;
    let mut failed_ids = Vec::with_capacity(failed.len());
    for (id, err) in failed {
        set_status(store, &id, STATUS_FAILED, err.as_deref()).await?;
        failed_ids.push(id);
    }
    Ok((sent, failed_ids))
}

/// Wire ids and application times of the messages we hold, for history
/// reconciliation. Only rows that came from the wire have a `wire_id`.
pub async fn count_all(store: &Store) -> Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM msg_messages")
        .fetch_one(store.pool())
        .await
        .map_err(storage)
}

/// Visible messages we sent in this chat (system rows excluded).
pub async fn count_visible_outgoing(store: &Store, chat_id: &str) -> Result<i64> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM msg_messages WHERE chat_id = ? AND direction = 'out' AND is_hidden = 0 AND content_type != 'system'",
    )
    .bind(chat_id)
    .fetch_one(store.pool())
    .await
    .map_err(storage)
}

/// The message was removed for me (here or on another device of mine):
/// remembered by id, so a copy that comes later is removed as well.
pub async fn remember_hidden(store: &Store, id: &str, at: i64) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO msg_own_hidden (message_id, hidden_at) VALUES (?, ?)")
        .bind(id)
        .bind(at)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// When the message was removed for me, if it was.
pub async fn hidden_at(store: &Store, id: &str) -> Result<Option<i64>> {
    sqlx::query_scalar::<_, i64>("SELECT hidden_at FROM msg_own_hidden WHERE message_id = ?")
        .bind(id)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)
}

pub const STATUS_UPLOADING: &str = "uploading";

/// Remove one row for good (placeholders of uploads).
pub async fn delete(store: &Store, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM msg_messages WHERE id = ?").bind(id).execute(store.pool()).await.map_err(storage)?;
    Ok(())
}

pub async fn set_media_json(store: &Store, id: &str, media_json: &str) -> Result<()> {
    sqlx::query("UPDATE msg_messages SET media_json = ? WHERE id = ?")
        .bind(media_json)
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// After a restart no upload is running: placeholders show as paused.
pub async fn pause_uploading(store: &Store) -> Result<u64> {
    let res = sqlx::query("UPDATE msg_messages SET status = 'paused' WHERE status = 'uploading'")
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(res.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(id: &str, chat: &str, dir: &str, at: i64) -> NewMessage {
        NewMessage {
            id: id.into(),
            chat_id: chat.into(),
            wire_id: None,
            direction: dir.into(),
            status: if dir == DIR_OUT { STATUS_QUEUED.into() } else { STATUS_RECEIVED.into() },
            content_type: CT_TEXT.into(),
            text: Some(format!("m{id}")),
            envelope_json: "{}".into(),
            sender_pubkey: "s".into(),
            reply_to_id: None,
            target_id: None,
            created_at: at,
            is_hidden: false,
            outbox_local_id: None,
            media_json: None,
        }
    }

    #[tokio::test]
    async fn insert_dedup_list_and_pagination() {
        let s = Store::open_in_memory().await.unwrap();
        for i in 1..=5 {
            assert!(insert(&s, &text(&i.to_string(), "c", DIR_IN, i * 10)).await.unwrap());
        }
        assert!(!insert(&s, &text("3", "c", DIR_IN, 30)).await.unwrap(), "duplicate copy ignored");
        let mut hidden = text("h", "c", DIR_IN, 35);
        hidden.is_hidden = true;
        hidden.content_type = CT_EDIT.into();
        hidden.target_id = Some("3".into());
        insert(&s, &hidden).await.unwrap();

        let page = list(&s, "c", None, 2).await.unwrap();
        assert_eq!(page.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["4", "5"], "newest page, oldest first");
        let older = list(&s, "c", Some(40), 10).await.unwrap();
        assert_eq!(older.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["1", "2", "3"], "hidden rows never listed");
        assert_eq!(last_created_at(&s, "c").await.unwrap(), Some(50));
        assert_eq!(count_visible_incoming(&s, "c").await.unwrap(), 5);
        assert_eq!(pending_for_target(&s, "3").await.unwrap().len(), 1);

        set_text(&s, "3", "changed", 99).await.unwrap();
        let m = get(&s, "3").await.unwrap().unwrap();
        assert_eq!(m.text.as_deref(), Some("changed"));
        assert_eq!(m.edited_at, Some(99));
        mark_deleted(&s, "3", 100).await.unwrap();
        let m = get(&s, "3").await.unwrap().unwrap();
        assert!(m.text.is_none() && m.deleted_at == Some(100));
        assert_eq!(count_visible_incoming(&s, "c").await.unwrap(), 5, "a retracted request still was a request");
        assert_eq!(count_visible_outgoing(&s, "c").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn statuses_follow_the_outbox() {
        use crate::outbox;
        use messenger_core::{Outbound, SubId};
        let s = Store::open_in_memory().await.unwrap();
        let out = Outbound::Unsubscribe { id: SubId("x".into()) };
        for (id, local) in [("a", "ok"), ("b", "bad"), ("c", "slow"), ("d", "late")] {
            outbox::enqueue(&s, local, &out, 0, Some(3600)).await.unwrap();
            let mut m = text(id, "c", DIR_OUT, 1);
            m.outbox_local_id = Some(local.into());
            insert(&s, &m).await.unwrap();
        }

        assert_eq!(sync_outbox_status(&s).await.unwrap(), (vec![], vec![]));
        outbox::mark_published(&s, "ok").await.unwrap();
        for _ in 0..5 {
            outbox::mark_publishing(&s, "slow").await.unwrap();
            outbox::mark_failed(&s, "slow", "transport error: no relay connected", 0, false).await.unwrap();
        }
        outbox::mark_abandoned(&s, "bad", "expired: offline").await.unwrap();
        let (sent, failed) = sync_outbox_status(&s).await.unwrap();
        assert_eq!(sent, vec!["a".to_string()]);
        assert_eq!(failed, vec!["b".to_string()]);
        assert_eq!(get(&s, "b").await.unwrap().unwrap().failure_reason.as_deref(), Some("expired: offline"));
        assert_eq!(get(&s, "c").await.unwrap().unwrap().status, STATUS_QUEUED, "retried, not given up: waiting");
        assert_eq!(sync_outbox_status(&s).await.unwrap(), (vec![], vec![]), "idempotent");

        // Shown as failed, then out after all (a retry, or the last try): sent.
        set_status(&s, "d", STATUS_FAILED, Some("offline")).await.unwrap();
        outbox::mark_published(&s, "late").await.unwrap();
        assert_eq!(sync_outbox_status(&s).await.unwrap(), (vec!["d".to_string()], vec![]));
        let d = get(&s, "d").await.unwrap().unwrap();
        assert_eq!((d.status.as_str(), d.failure_reason), (STATUS_SENT, None));
    }
}
