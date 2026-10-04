// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_chats`: one row per conversation. DM chats are keyed by peer.

use crate::{storage, Store};
use messenger_core::Result;

pub const KIND_DM: &str = "dm";

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct ChatRow {
    pub id: String,
    pub kind: String,
    pub peer_pubkey: Option<String>,
    pub unread: i64,
    pub last_message_at: Option<i64>,
    pub last_preview: Option<String>,
    pub pinned: bool,
    pub archived: bool,
    /// No sound from this chat; what comes is still counted and shown.
    pub muted: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

const COLS: &str = "id, kind, peer_pubkey, unread, last_message_at, last_preview, pinned, archived, muted, created_at, updated_at";

pub fn dm_chat_id(peer_pubkey: &str) -> String {
    format!("dm:{peer_pubkey}")
}

/// Create the DM chat with `peer` if missing; returns the row either way.
pub async fn ensure_dm(store: &Store, peer_pubkey: &str) -> Result<ChatRow> {
    let id = dm_chat_id(peer_pubkey);
    let now = crate::now();
    sqlx::query(
        "INSERT OR IGNORE INTO msg_chats (id, kind, peer_pubkey, unread, last_message_at, last_preview, pinned, archived, muted, created_at, updated_at)
         VALUES (?, 'dm', ?, 0, NULL, NULL, 0, 0, 0, ?, ?)",
    )
    .bind(&id)
    .bind(peer_pubkey)
    .bind(now)
    .bind(now)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    get(store, &id).await?.ok_or_else(|| messenger_core::MessengerError::Storage("chat vanished".into()))
}

pub async fn get(store: &Store, id: &str) -> Result<Option<ChatRow>> {
    sqlx::query_as::<_, ChatRow>(sqlx::AssertSqlSafe(format!("SELECT {COLS} FROM msg_chats WHERE id = ?")))
        .bind(id)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)
}

/// Pinned first, then newest activity. Archived chats only when asked.
pub async fn list(store: &Store, include_archived: bool) -> Result<Vec<ChatRow>> {
    let sql = if include_archived {
        format!("SELECT {COLS} FROM msg_chats ORDER BY pinned DESC, COALESCE(last_message_at, created_at) DESC")
    } else {
        format!("SELECT {COLS} FROM msg_chats WHERE archived = 0 ORDER BY pinned DESC, COALESCE(last_message_at, created_at) DESC")
    };
    sqlx::query_as::<_, ChatRow>(sqlx::AssertSqlSafe(sql))
        .fetch_all(store.pool())
        .await
        .map_err(storage)
}

/// New activity: move `last_message_at` forward (never back), refresh the
/// preview when this message is the newest, and bump unread if asked.
pub async fn touch(store: &Store, id: &str, at: i64, preview: Option<&str>, bump_unread: bool) -> Result<()> {
    let now = crate::now();
    sqlx::query(
        "UPDATE msg_chats SET
           last_preview = CASE WHEN last_message_at IS NULL OR ? >= last_message_at THEN ? ELSE last_preview END,
           last_message_at = MAX(COALESCE(last_message_at, 0), ?),
           unread = unread + ?,
           archived = 0,
           updated_at = ?
         WHERE id = ?",
    )
    .bind(at)
    .bind(preview)
    .bind(at)
    .bind(if bump_unread { 1 } else { 0 })
    .bind(now)
    .bind(id)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// Recompute `last_message_at`/`last_preview` from the visible messages
/// (after a delete or an edit of the newest message).
pub async fn recompute_last(store: &Store, id: &str) -> Result<()> {
    sqlx::query(
        "UPDATE msg_chats SET
           last_message_at = (SELECT MAX(created_at) FROM msg_messages WHERE chat_id = ? AND is_hidden = 0 AND content_type != 'system'),
           last_preview = (SELECT CASE WHEN deleted_at IS NOT NULL THEN NULL
                                       WHEN content_type = 'media' THEN '📎 ' || COALESCE(text, json_extract(media_json, '$.name'), '')
                                       ELSE text END FROM msg_messages
                           WHERE chat_id = ? AND is_hidden = 0 AND content_type != 'system' ORDER BY created_at DESC LIMIT 1),
           updated_at = ?
         WHERE id = ?",
    )
    .bind(id)
    .bind(id)
    .bind(crate::now())
    .bind(id)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

pub async fn mark_read(store: &Store, id: &str) -> Result<()> {
    sqlx::query("UPDATE msg_chats SET unread = 0, updated_at = ? WHERE id = ?")
        .bind(crate::now())
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn set_pinned(store: &Store, id: &str, pinned: bool) -> Result<()> {
    sqlx::query("UPDATE msg_chats SET pinned = ?, updated_at = ? WHERE id = ?")
        .bind(pinned)
        .bind(crate::now())
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn set_archived(store: &Store, id: &str, archived: bool) -> Result<()> {
    sqlx::query("UPDATE msg_chats SET archived = ?, updated_at = ? WHERE id = ?")
        .bind(archived)
        .bind(crate::now())
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn set_muted(store: &Store, id: &str, muted: bool) -> Result<()> {
    sqlx::query("UPDATE msg_chats SET muted = ?, updated_at = ? WHERE id = ?")
        .bind(muted)
        .bind(crate::now())
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// A chat that does not exist yet is not muted.
pub async fn is_muted(store: &Store, id: &str) -> Result<bool> {
    Ok(get(store, id).await?.map(|c| c.muted).unwrap_or(false))
}

/// Remove the chat and every message in it (local only; relays keep the
/// ciphertext, and history sync would bring visible rows back unless the
/// caller also records the retraction).
pub async fn delete(store: &Store, id: &str) -> Result<()> {
    let mut tx = store.pool().begin().await.map_err(storage)?;
    sqlx::query("DELETE FROM msg_messages WHERE chat_id = ?").bind(id).execute(&mut *tx).await.map_err(storage)?;
    sqlx::query("DELETE FROM msg_chats WHERE id = ?").bind(id).execute(&mut *tx).await.map_err(storage)?;
    tx.commit().await.map_err(storage)
}

/// What the app counts as waiting: a muted or archived chat keeps its own
/// counter but stays out of the total (the badge, the tray).
pub async fn total_unread(store: &Store) -> Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT COALESCE(SUM(unread), 0) FROM msg_chats WHERE archived = 0 AND muted = 0")
        .fetch_one(store.pool())
        .await
        .map_err(storage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn chat_lifecycle() {
        let s = Store::open_in_memory().await.unwrap();
        let a = ensure_dm(&s, "aa").await.unwrap();
        assert_eq!(a.id, "dm:aa");
        assert_eq!(ensure_dm(&s, "aa").await.unwrap(), a, "idempotent");
        ensure_dm(&s, "bb").await.unwrap();

        let t0 = crate::now() + 100;
        touch(&s, "dm:aa", t0, Some("hello"), true).await.unwrap();
        touch(&s, "dm:aa", t0 - 50, Some("older"), true).await.unwrap();
        let a = get(&s, "dm:aa").await.unwrap().unwrap();
        assert_eq!(a.unread, 2);
        assert_eq!(a.last_message_at, Some(t0));
        assert_eq!(a.last_preview.as_deref(), Some("hello"), "older message does not replace the preview");
        assert_eq!(total_unread(&s).await.unwrap(), 2);

        let l = list(&s, false).await.unwrap();
        assert_eq!(l[0].id, "dm:aa", "recent activity first");
        set_pinned(&s, "dm:bb", true).await.unwrap();
        assert_eq!(list(&s, false).await.unwrap()[0].id, "dm:bb", "pinned wins");

        mark_read(&s, "dm:aa").await.unwrap();
        assert_eq!(get(&s, "dm:aa").await.unwrap().unwrap().unread, 0);
        assert!(!is_muted(&s, "dm:aa").await.unwrap());
        assert!(!is_muted(&s, "dm:nobody").await.unwrap(), "a chat that does not exist is not muted");
        set_muted(&s, "dm:aa", true).await.unwrap();
        assert!(is_muted(&s, "dm:aa").await.unwrap());
        touch(&s, "dm:aa", t0 + 1, Some("still counted"), true).await.unwrap();
        assert_eq!(get(&s, "dm:aa").await.unwrap().unwrap().unread, 1, "muted chats still count unread");
        assert_eq!(total_unread(&s).await.unwrap(), 0, "but stay out of the total");
        mark_read(&s, "dm:aa").await.unwrap();
        set_archived(&s, "dm:aa", true).await.unwrap();
        assert_eq!(list(&s, false).await.unwrap().len(), 1);
        assert_eq!(list(&s, true).await.unwrap().len(), 2);
        touch(&s, "dm:aa", t0 + 100, Some("back"), false).await.unwrap();
        assert!(!get(&s, "dm:aa").await.unwrap().unwrap().archived, "activity unarchives");

        delete(&s, "dm:aa").await.unwrap();
        assert!(get(&s, "dm:aa").await.unwrap().is_none());
    }
}
