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
/// preview when this message is the newest, and bump unread if asked and
/// the message is later than what was read on any of my devices.
pub async fn touch(store: &Store, id: &str, at: i64, preview: Option<&str>, bump_unread: bool) -> Result<()> {
    let now = crate::now();
    sqlx::query(
        "UPDATE msg_chats SET
           last_preview = CASE WHEN last_message_at IS NULL OR ? >= last_message_at THEN ? ELSE last_preview END,
           last_message_at = MAX(COALESCE(last_message_at, 0), ?),
           unread = unread + CASE WHEN ? AND ? > COALESCE((SELECT read_at FROM msg_own_read WHERE chat_id = ?), 0)
                                  THEN 1 ELSE 0 END,
           archived = 0,
           updated_at = ?
         WHERE id = ?",
    )
    .bind(at)
    .bind(preview)
    .bind(at)
    .bind(bump_unread)
    .bind(at)
    .bind(id)
    .bind(now)
    .bind(id)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// Recompute `last_message_at`/`last_preview` from the messages still shown
/// (after a delete or an edit of the newest message). A removed message
/// leaves its place in the chat but not in the list: the list shows the
/// newest one that is still there.
pub async fn recompute_last(store: &Store, id: &str) -> Result<()> {
    sqlx::query(
        "UPDATE msg_chats SET
           last_message_at = (SELECT MAX(created_at) FROM msg_messages
                               WHERE chat_id = ? AND is_hidden = 0 AND deleted_at IS NULL AND content_type != 'system'),
           last_preview = (SELECT CASE WHEN content_type = 'media' THEN '📎 ' || COALESCE(text, json_extract(media_json, '$.name'), '')
                                       WHEN content_type = 'contact' THEN '👤 ' || COALESCE(json_extract(media_json, '$.display_name'),
                                                                                         json_extract(media_json, '$.name'), '')
                                       ELSE text END FROM msg_messages
                           WHERE chat_id = ? AND is_hidden = 0 AND deleted_at IS NULL AND content_type != 'system'
                           ORDER BY created_at DESC LIMIT 1),
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

/// The chat was read here: nothing waits, and what is read moves up to the
/// newest message of the peer. Returns that time when it moved, so the
/// other devices can be told; `None` when they know it already. A mark that
/// moved is also owed to the peers (`receipts::take_due_read`).
pub async fn mark_read(store: &Store, id: &str) -> Result<Option<i64>> {
    sqlx::query("UPDATE msg_chats SET unread = 0, updated_at = ? WHERE id = ?")
        .bind(crate::now())
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    let newest = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(created_at) FROM msg_messages WHERE chat_id = ? AND direction = 'in' AND is_hidden = 0",
    )
    .bind(id)
    .fetch_one(store.pool())
    .await
    .map_err(storage)?;
    match newest {
        Some(at) if raise_read(store, id, at, true).await? => Ok(Some(at)),
        _ => Ok(None),
    }
}

/// Another device of mine read the chat up to `at`: what is not later stops
/// counting. Unread is only ever lowered here. `true` when it went down.
/// The peers are told by the device that read, not by this one.
pub async fn read_up_to(store: &Store, id: &str, at: i64) -> Result<bool> {
    if !raise_read(store, id, at, false).await? {
        return Ok(false);
    }
    let later = "(SELECT COUNT(*) FROM msg_messages
                  WHERE chat_id = ?1 AND direction = 'in' AND is_hidden = 0 AND deleted_at IS NULL
                    AND content_type != 'system' AND created_at > ?2)";
    let r = sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE msg_chats SET unread = {later}, updated_at = ?3 WHERE id = ?1 AND unread > {later}"
    )))
    .bind(id)
    .bind(at)
    .bind(crate::now())
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(r.rows_affected() > 0)
}

/// Move what is read in the chat up to `at`, never back. `true` when it moved.
/// `due`: read here, so a receipt is owed; a mark from elsewhere leaves a
/// receipt still owed as it was.
async fn raise_read(store: &Store, id: &str, at: i64, due: bool) -> Result<bool> {
    let r = sqlx::query(
        "INSERT INTO msg_own_read (chat_id, read_at, receipt_due) VALUES (?, ?, ?)
         ON CONFLICT(chat_id) DO UPDATE SET read_at = excluded.read_at,
                                            receipt_due = MAX(msg_own_read.receipt_due, excluded.receipt_due)
         WHERE excluded.read_at > msg_own_read.read_at",
    )
    .bind(id)
    .bind(at)
    .bind(due)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(r.rows_affected() > 0)
}

/// Up to when the chat is read on any of my devices; `0` when it never was.
pub async fn read_at(store: &Store, id: &str) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>("SELECT read_at FROM msg_own_read WHERE chat_id = ?")
        .bind(id)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)?
        .unwrap_or(0))
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

/// How far ahead of the clock `cleared_at` may sit (seconds).
pub const CLEARED_SKEW_SECS: i64 = 300;

/// Remove the chat, every message in it and every reaction that came in it
/// (local only). Relays keep the ciphertext, so a tombstone keeps it gone.
/// My own copies are dropped by time (`cleared_at`): my rumors can run ahead
/// of the clock (`next_created_at`), so it is never earlier than my newest
/// message, and never more than `CLEARED_SKEW_SECS` past the clock, so one
/// row dated in the future cannot hold back all that comes before it. The
/// rows themselves are remembered by id (`was_deleted`): that is all that
/// keeps the peer's messages out, so one of theirs this device never had
/// still comes in, whatever their clock says.
pub async fn delete(store: &Store, id: &str, now: i64) -> Result<()> {
    let mut tx = store.pool().begin().await.map_err(storage)?;
    sqlx::query(
        "INSERT INTO msg_chat_cleared (chat_id, cleared_at)
         VALUES (?1, MAX(?2, MIN(?2 + ?3, COALESCE(
             (SELECT MAX(created_at) FROM msg_messages WHERE chat_id = ?1 AND direction = 'out'), 0))))
         ON CONFLICT(chat_id) DO UPDATE SET cleared_at = MAX(msg_chat_cleared.cleared_at, excluded.cleared_at)",
    )
    .bind(id)
    .bind(now)
    .bind(CLEARED_SKEW_SECS)
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    // Held rows of the DM gate were never shown: gone as if dropped, not
    // remembered (as `dm_held::purge`).
    sqlx::query("DELETE FROM msg_messages WHERE id IN (SELECT message_id FROM msg_dm_held WHERE chat_id = ?) AND is_hidden = 1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    sqlx::query("DELETE FROM msg_dm_held WHERE chat_id = ?").bind(id).execute(&mut *tx).await.map_err(storage)?;
    // Every row, mine too: an own row dated past the cap stays gone by id.
    // System lines and placeholders have local ids no rumor carries.
    sqlx::query(
        "INSERT OR IGNORE INTO msg_chat_deleted_ids (message_id, chat_id, deleted_at)
         SELECT id, chat_id, ?2 FROM msg_messages WHERE chat_id = ?1",
    )
    .bind(id)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    sqlx::query("DELETE FROM msg_messages WHERE chat_id = ?").bind(id).execute(&mut *tx).await.map_err(storage)?;
    sqlx::query("DELETE FROM msg_reactions WHERE chat_id = ?").bind(id).execute(&mut *tx).await.map_err(storage)?;
    sqlx::query("DELETE FROM msg_chats WHERE id = ?").bind(id).execute(&mut *tx).await.map_err(storage)?;
    tx.commit().await.map_err(storage)
}

/// The rumor was in a chat when that chat was deleted here.
pub async fn was_deleted(store: &Store, message_id: &str) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>("SELECT 1 FROM msg_chat_deleted_ids WHERE message_id = ?")
        .bind(message_id)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)?
        .is_some())
}

/// Up to when (rumor time, inclusive) the chat was deleted on this device;
/// `0` when it never was.
pub async fn cleared_at(store: &Store, id: &str) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>("SELECT cleared_at FROM msg_chat_cleared WHERE chat_id = ?")
        .bind(id)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)?
        .unwrap_or(0))
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

        delete(&s, "dm:aa", crate::now()).await.unwrap();
        assert!(get(&s, "dm:aa").await.unwrap().is_none());
    }

    async fn incoming(s: &Store, id: &str, at: i64, text: &str) {
        row(s, id, at, text, "in").await
    }

    async fn row(s: &Store, id: &str, at: i64, text: &str, direction: &str) {
        let m = crate::messages::NewMessage {
            id: id.into(),
            chat_id: "dm:aa".into(),
            wire_id: None,
            direction: direction.into(),
            status: if direction == "in" { "received" } else { "sent" }.into(),
            content_type: "text".into(),
            text: Some(text.into()),
            envelope_json: "{}".into(),
            sender_pubkey: "aa".into(),
            reply_to_id: None,
            target_id: None,
            created_at: at,
            is_hidden: false,
            outbox_local_id: None,
            media_json: None,
        };
        crate::messages::insert(s, &m).await.unwrap();
        touch(s, "dm:aa", at, Some(text), true).await.unwrap();
    }

    #[tokio::test]
    async fn the_list_shows_the_newest_message_still_there() {
        let s = Store::open_in_memory().await.unwrap();
        ensure_dm(&s, "aa").await.unwrap();
        incoming(&s, "m1", 100, "one").await;
        incoming(&s, "m2", 200, "two").await;
        crate::messages::mark_deleted(&s, "m2", 300).await.unwrap();
        recompute_last(&s, "dm:aa").await.unwrap();
        let c = get(&s, "dm:aa").await.unwrap().unwrap();
        assert_eq!((c.last_preview.as_deref(), c.last_message_at), (Some("one"), Some(100)));
        crate::messages::mark_deleted(&s, "m1", 300).await.unwrap();
        recompute_last(&s, "dm:aa").await.unwrap();
        assert_eq!(get(&s, "dm:aa").await.unwrap().unwrap().last_preview, None, "nothing left");
    }

    #[tokio::test]
    async fn a_contact_card_shows_in_the_list_by_its_name() {
        let s = Store::open_in_memory().await.unwrap();
        ensure_dm(&s, "aa").await.unwrap();
        incoming(&s, "m1", 100, "one").await;
        let card = |id: &str, at: i64, json: &str| crate::messages::NewMessage {
            id: id.into(),
            chat_id: "dm:aa".into(),
            wire_id: None,
            direction: "in".into(),
            status: "received".into(),
            content_type: crate::messages::CT_CONTACT.into(),
            text: None,
            envelope_json: "{}".into(),
            sender_pubkey: "aa".into(),
            reply_to_id: None,
            target_id: None,
            created_at: at,
            is_hidden: false,
            outbox_local_id: None,
            media_json: Some(json.into()),
        };
        crate::messages::insert(&s, &card("c1", 200, r#"{"pubkey":"bb","name":"anna","display_name":"Анна"}"#)).await.unwrap();
        recompute_last(&s, "dm:aa").await.unwrap();
        assert_eq!(get(&s, "dm:aa").await.unwrap().unwrap().last_preview.as_deref(), Some("👤 Анна"));
        crate::messages::insert(&s, &card("c2", 300, r#"{"pubkey":"bb","name":"anna"}"#)).await.unwrap();
        recompute_last(&s, "dm:aa").await.unwrap();
        assert_eq!(get(&s, "dm:aa").await.unwrap().unwrap().last_preview.as_deref(), Some("👤 anna"));
        crate::messages::insert(&s, &card("c3", 400, r#"{"pubkey":"bb"}"#)).await.unwrap();
        recompute_last(&s, "dm:aa").await.unwrap();
        assert_eq!(get(&s, "dm:aa").await.unwrap().unwrap().last_preview.as_deref(), Some("👤 "));
    }

    #[tokio::test]
    async fn a_deleted_chat_leaves_a_tombstone_that_never_goes_back() {
        let s = Store::open_in_memory().await.unwrap();
        assert_eq!(cleared_at(&s, "dm:aa").await.unwrap(), 0, "never deleted");
        ensure_dm(&s, "aa").await.unwrap();
        incoming(&s, "m1", 100, "one").await;
        row(&s, "m2", 600, "mine, ahead of the clock", "out").await;
        incoming(&s, "m3", 650, "theirs, further ahead").await;
        delete(&s, "dm:aa", 500).await.unwrap();
        assert!(get(&s, "dm:aa").await.unwrap().is_none());
        assert_eq!(cleared_at(&s, "dm:aa").await.unwrap(), 600, "my newest message when it is later than the clock");
        for id in ["m1", "m2", "m3"] {
            assert!(was_deleted(&s, id).await.unwrap(), "{id} is remembered");
        }
        assert!(!was_deleted(&s, "m4").await.unwrap());
        delete(&s, "dm:aa", 550).await.unwrap();
        assert_eq!(cleared_at(&s, "dm:aa").await.unwrap(), 600, "a second delete never lowers it");
        delete(&s, "dm:aa", 700).await.unwrap();
        assert_eq!(cleared_at(&s, "dm:aa").await.unwrap(), 700, "a later one raises it");

        ensure_dm(&s, "bb").await.unwrap();
        delete(&s, "dm:bb", 500).await.unwrap();
        assert_eq!(cleared_at(&s, "dm:bb").await.unwrap(), 500, "an empty chat: the clock");
        assert_eq!(cleared_at(&s, "dm:cc").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn a_future_dated_row_moves_the_tombstone_only_so_far() {
        let s = Store::open_in_memory().await.unwrap();
        let now = 1_000_000;
        ensure_dm(&s, "aa").await.unwrap();
        incoming(&s, "theirs", now + 1_000_000, "from a clock far ahead").await;
        delete(&s, "dm:aa", now).await.unwrap();
        assert_eq!(cleared_at(&s, "dm:aa").await.unwrap(), now, "the peer's rows never move it");
        assert!(was_deleted(&s, "theirs").await.unwrap(), "that row stays gone by its id");

        ensure_dm(&s, "aa").await.unwrap();
        row(&s, "mine", now + 1_000_000, "pushed ahead by such a row", "out").await;
        delete(&s, "dm:aa", now).await.unwrap();
        let cleared = cleared_at(&s, "dm:aa").await.unwrap();
        assert!(cleared <= now + 300);
        assert_eq!(cleared, now + CLEARED_SKEW_SECS);
        assert!(was_deleted(&s, "mine").await.unwrap());
    }

    #[tokio::test]
    async fn migration_025_caps_a_future_tombstone() {
        let s = Store::open_in_memory().await.unwrap();
        let now = crate::now();
        for (chat, at) in [("dm:past", 500_i64), ("dm:future", 4_102_444_800)] {
            sqlx::query("INSERT INTO msg_chat_cleared (chat_id, cleared_at) VALUES (?, ?)")
                .bind(chat)
                .bind(at)
                .execute(s.pool())
                .await
                .unwrap();
        }
        sqlx::raw_sql(include_str!("../migrations/025_chat_deleted_ids.sql")).execute(s.pool()).await.unwrap();
        assert_eq!(cleared_at(&s, "dm:past").await.unwrap(), 500);
        let future = cleared_at(&s, "dm:future").await.unwrap();
        assert!((now + CLEARED_SKEW_SECS..=now + CLEARED_SKEW_SECS + 5).contains(&future), "{future}");
    }

    #[tokio::test]
    async fn read_on_any_device_holds_whatever_comes_first() {
        let s = Store::open_in_memory().await.unwrap();
        assert!(!read_up_to(&s, "dm:aa", 150).await.unwrap(), "no chat yet: kept, nothing to lower");
        ensure_dm(&s, "aa").await.unwrap();
        incoming(&s, "m1", 100, "one").await;
        incoming(&s, "m2", 200, "two").await;
        assert_eq!(get(&s, "dm:aa").await.unwrap().unwrap().unread, 1, "the earlier one was read elsewhere");
        assert!(!read_up_to(&s, "dm:aa", 120).await.unwrap(), "never back");
        assert_eq!(mark_read(&s, "dm:aa").await.unwrap(), Some(200));
        assert_eq!(mark_read(&s, "dm:aa").await.unwrap(), None, "the others know it");
        assert_eq!(read_at(&s, "dm:aa").await.unwrap(), 200);
        incoming(&s, "m3", 300, "three").await;
        assert!(read_up_to(&s, "dm:aa", 300).await.unwrap());
        assert_eq!(get(&s, "dm:aa").await.unwrap().unwrap().unread, 0);
    }
}
