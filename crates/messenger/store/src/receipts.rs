// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Receipts both ways. What the peers told me (`msg_dm_delivered`,
//! `msg_peer_read`) is kept by its own key, so a receipt that comes before
//! the message it names still holds. What I owe them is read off the
//! messages (`acked_at`) and off my own read marks (`receipt_due`), and
//! taken once: a second take finds nothing.

use crate::{storage, Store};
use messenger_core::Result;

/// The most ids one delivery receipt names; the rest wait for the next take.
pub const DELIVERED_IDS_PER_CHAT: i64 = 100;

/// A message of mine reached one of `peer`'s devices. `true` when this is
/// the first word of it.
pub async fn mark_delivered(store: &Store, message_id: &str, peer: &str, at: i64) -> Result<bool> {
    let r = sqlx::query("INSERT OR IGNORE INTO msg_dm_delivered (message_id, peer, delivered_at) VALUES (?, ?, ?)")
        .bind(message_id)
        .bind(peer)
        .bind(at)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(r.rows_affected() > 0)
}

/// When `peer` first said it has the message. A receipt from anybody else
/// does not count.
pub async fn delivered_at(store: &Store, message_id: &str, peer: &str) -> Result<Option<i64>> {
    sqlx::query_scalar::<_, i64>("SELECT delivered_at FROM msg_dm_delivered WHERE message_id = ? AND peer = ?")
        .bind(message_id)
        .bind(peer)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)
}

/// `member` read the chat up to `at`; never moves back, since receipts come
/// in any order. `true` when it moved.
pub async fn raise_peer_read(store: &Store, chat_id: &str, member: &str, at: i64) -> Result<bool> {
    let r = sqlx::query(
        "INSERT INTO msg_peer_read (chat_id, member, read_at) VALUES (?, ?, ?)
         ON CONFLICT(chat_id, member) DO UPDATE SET read_at = excluded.read_at WHERE excluded.read_at > msg_peer_read.read_at",
    )
    .bind(chat_id)
    .bind(member)
    .bind(at)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(r.rows_affected() > 0)
}

/// Up to when each member read the chat, by member.
pub async fn peer_reads(store: &Store, chat_id: &str) -> Result<Vec<(String, i64)>> {
    sqlx::query_as::<_, (String, i64)>("SELECT member, read_at FROM msg_peer_read WHERE chat_id = ? ORDER BY member")
        .bind(chat_id)
        .fetch_all(store.pool())
        .await
        .map_err(storage)
}

/// What makes an incoming message owe a delivery receipt, `?1` being the
/// window floor. The first three terms are those of the partial index
/// `msg_messages_unacked` (migration 015), word for word: SQLite uses a
/// partial index only for a query that repeats its terms.
const OWED_DELIVERED: &str = "acked_at IS NULL AND direction = 'in' AND substr(chat_id, 1, 3) = 'dm:'
    AND created_at >= ?1 AND is_hidden = 0 AND deleted_at IS NULL AND content_type != 'system'";

/// The incoming DM messages that still owe a delivery receipt, by chat, the
/// oldest first; marked as acknowledged at `now` in the same statement, so
/// that two takes never hand out the same id. Messages older than
/// `window_floor` are left alone: history synced on a new login is not news
/// to the peer. At most `DELIVERED_IDS_PER_CHAT` ids per chat. When nothing
/// is owed, which is nearly always, the take only reads the index and takes
/// no write lock.
pub async fn take_due_delivered(store: &Store, now: i64, window_floor: i64) -> Result<Vec<(String, Vec<String>)>> {
    let owed = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!("SELECT EXISTS (SELECT 1 FROM msg_messages WHERE {OWED_DELIVERED})")))
        .bind(window_floor)
        .fetch_one(store.pool())
        .await
        .map_err(storage)?;
    if owed == 0 {
        return Ok(vec![]);
    }
    let mut rows = sqlx::query_as::<_, (String, String, i64)>(sqlx::AssertSqlSafe(format!(
        "UPDATE msg_messages SET acked_at = ?2
         WHERE id IN (
           SELECT id FROM (
             SELECT id, ROW_NUMBER() OVER (PARTITION BY chat_id ORDER BY created_at, id) AS n
             FROM msg_messages WHERE {OWED_DELIVERED}
           ) WHERE n <= ?3
         )
         RETURNING chat_id, id, created_at"
    )))
    .bind(window_floor)
    .bind(now)
    .bind(DELIVERED_IDS_PER_CHAT)
    .fetch_all(store.pool())
    .await
    .map_err(storage)?;
    // RETURNING keeps no order.
    rows.sort_by(|a, b| (&a.0, a.2, &a.1).cmp(&(&b.0, b.2, &b.1)));
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for (chat_id, id, _) in rows {
        match out.last_mut() {
            Some((last, ids)) if *last == chat_id => ids.push(id),
            _ => out.push((chat_id, vec![id])),
        }
    }
    Ok(out)
}

/// The receipt for these taken ids could not be queued: they are owed
/// again, and the next take hands them out.
pub async fn owe_delivered_again(store: &Store, ids: &[String]) -> Result<()> {
    for id in ids {
        sqlx::query("UPDATE msg_messages SET acked_at = NULL WHERE id = ?")
            .bind(id)
            .execute(store.pool())
            .await
            .map_err(storage)?;
    }
    Ok(())
}

/// The message will get no receipt (too old when it came): it is owed none.
pub async fn set_acked(store: &Store, message_id: &str, at: i64) -> Result<()> {
    sqlx::query("UPDATE msg_messages SET acked_at = ? WHERE id = ?")
        .bind(at)
        .bind(message_id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// The chats read on this device whose peers have not been told, with the
/// mark to tell them; the flag is cleared in the same statement. Nothing
/// owed: only a read, no write lock.
pub async fn take_due_read(store: &Store) -> Result<Vec<(String, i64)>> {
    let owed = sqlx::query_scalar::<_, i64>("SELECT EXISTS (SELECT 1 FROM msg_own_read WHERE receipt_due = 1)")
        .fetch_one(store.pool())
        .await
        .map_err(storage)?;
    if owed == 0 {
        return Ok(vec![]);
    }
    let mut rows = sqlx::query_as::<_, (String, i64)>(
        "UPDATE msg_own_read SET receipt_due = 0 WHERE receipt_due = 1 RETURNING chat_id, read_at",
    )
    .fetch_all(store.pool())
    .await
    .map_err(storage)?;
    rows.sort();
    Ok(rows)
}

/// The read receipt of a taken chat could not be queued: it is owed again,
/// with whatever the mark is at the next take.
pub async fn owe_read_again(store: &Store, chat_id: &str) -> Result<()> {
    sqlx::query("UPDATE msg_own_read SET receipt_due = 1 WHERE chat_id = ?")
        .bind(chat_id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{chats, messages};

    async fn message(s: &Store, id: &str, chat: &str, direction: &str, at: i64) {
        let m = messages::NewMessage {
            id: id.into(),
            chat_id: chat.into(),
            wire_id: None,
            direction: direction.into(),
            status: if direction == "in" { "received" } else { "sent" }.into(),
            content_type: "text".into(),
            text: Some(id.into()),
            envelope_json: "{}".into(),
            sender_pubkey: "aa".into(),
            reply_to_id: None,
            target_id: None,
            created_at: at,
            is_hidden: false,
            outbox_local_id: None,
            media_json: None,
        };
        messages::insert(s, &m).await.unwrap();
    }

    #[tokio::test]
    async fn delivered_is_kept_once_and_by_the_peer_who_said_it() {
        let s = Store::open_in_memory().await.unwrap();
        assert_eq!(delivered_at(&s, "m1", "bb").await.unwrap(), None);
        assert!(mark_delivered(&s, "m1", "bb", 100).await.unwrap(), "before the message itself: kept");
        assert!(!mark_delivered(&s, "m1", "bb", 200).await.unwrap(), "a second device of the peer says it again");
        assert_eq!(delivered_at(&s, "m1", "bb").await.unwrap(), Some(100), "the first word stays");
        assert_eq!(delivered_at(&s, "m1", "cc").await.unwrap(), None, "not from somebody else");
    }

    #[tokio::test]
    async fn a_peer_read_only_moves_forward() {
        let s = Store::open_in_memory().await.unwrap();
        assert!(peer_reads(&s, "group:g").await.unwrap().is_empty());
        assert!(raise_peer_read(&s, "group:g", "bb", 200).await.unwrap());
        assert!(!raise_peer_read(&s, "group:g", "bb", 150).await.unwrap(), "never back");
        assert!(!raise_peer_read(&s, "group:g", "bb", 200).await.unwrap(), "nor in place");
        assert!(raise_peer_read(&s, "group:g", "aa", 120).await.unwrap());
        assert!(raise_peer_read(&s, "dm:bb", "bb", 50).await.unwrap(), "another chat");
        assert!(raise_peer_read(&s, "group:g", "bb", 300).await.unwrap());
        assert_eq!(peer_reads(&s, "group:g").await.unwrap(), vec![("aa".into(), 120), ("bb".into(), 300)]);
        assert_eq!(peer_reads(&s, "dm:bb").await.unwrap(), vec![("bb".into(), 50)]);
    }

    #[tokio::test]
    async fn delivery_receipts_are_owed_once_for_what_came_lately() {
        let s = Store::open_in_memory().await.unwrap();
        message(&s, "a2", "dm:aa", "in", 200).await;
        message(&s, "a1", "dm:aa", "in", 100).await;
        message(&s, "old", "dm:aa", "in", 10).await;
        message(&s, "mine", "dm:aa", "out", 150).await;
        message(&s, "b1", "dm:bb", "in", 300).await;
        message(&s, "g1", "group:g", "in", 300).await;
        message(&s, "gone", "dm:bb", "in", 310).await;
        messages::mark_deleted(&s, "gone", 320).await.unwrap();
        message(&s, "stale", "dm:bb", "in", 320).await;
        set_acked(&s, "stale", 320).await.unwrap();

        let due = take_due_delivered(&s, 1_000, 50).await.unwrap();
        assert_eq!(
            due,
            vec![("dm:aa".into(), vec!["a1".into(), "a2".into()]), ("dm:bb".into(), vec!["b1".into()])],
            "the oldest first; not mine, not too old, not a group, not deleted, not already acked"
        );
        assert!(take_due_delivered(&s, 1_001, 50).await.unwrap().is_empty(), "taken once");
        let acked = sqlx::query_scalar::<_, Option<i64>>("SELECT acked_at FROM msg_messages WHERE id = 'a1'")
            .fetch_one(s.pool())
            .await
            .unwrap();
        assert_eq!(acked, Some(1_000));

        message(&s, "b2", "dm:bb", "in", 400).await;
        assert_eq!(take_due_delivered(&s, 1_002, 50).await.unwrap(), vec![("dm:bb".into(), vec!["b2".into()])]);
    }

    #[tokio::test]
    async fn a_long_backlog_goes_a_hundred_at_a_time() {
        let s = Store::open_in_memory().await.unwrap();
        for i in 0..(DELIVERED_IDS_PER_CHAT + 5) {
            message(&s, &format!("m{i:03}"), "dm:aa", "in", 1_000 + i).await;
        }
        let first = take_due_delivered(&s, 5_000, 0).await.unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].1.len(), DELIVERED_IDS_PER_CHAT as usize);
        assert_eq!(first[0].1[0], "m000", "the oldest go first");
        let rest = take_due_delivered(&s, 5_002, 0).await.unwrap();
        assert_eq!(rest, vec![("dm:aa".into(), (100..105).map(|i| format!("m{i:03}")).collect())]);
        assert!(take_due_delivered(&s, 5_004, 0).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_read_here_is_owed_to_the_peer_and_one_from_elsewhere_is_not() {
        let s = Store::open_in_memory().await.unwrap();
        chats::ensure_dm(&s, "aa").await.unwrap();
        chats::ensure_dm(&s, "bb").await.unwrap();
        message(&s, "a1", "dm:aa", "in", 100).await;
        message(&s, "b1", "dm:bb", "in", 100).await;

        chats::read_up_to(&s, "dm:bb", 100).await.unwrap();
        assert_eq!(chats::read_at(&s, "dm:bb").await.unwrap(), 100, "read on another device");
        assert_eq!(chats::mark_read(&s, "dm:bb").await.unwrap(), None, "already read elsewhere: nothing moved");
        assert!(take_due_read(&s).await.unwrap().is_empty(), "the device that read it tells the peer");

        assert_eq!(chats::mark_read(&s, "dm:aa").await.unwrap(), Some(100));
        assert_eq!(take_due_read(&s).await.unwrap(), vec![("dm:aa".into(), 100)]);
        assert!(take_due_read(&s).await.unwrap().is_empty(), "taken once");

        // Read here, then a later mark from another device before the take:
        // still owed, with the later mark.
        message(&s, "a2", "dm:aa", "in", 200).await;
        assert_eq!(chats::mark_read(&s, "dm:aa").await.unwrap(), Some(200));
        chats::read_up_to(&s, "dm:aa", 250).await.unwrap();
        assert_eq!(take_due_read(&s).await.unwrap(), vec![("dm:aa".into(), 250)]);
        assert_eq!(chats::mark_read(&s, "dm:aa").await.unwrap(), None);
        assert!(take_due_read(&s).await.unwrap().is_empty(), "nothing moved, nothing owed");
    }

    #[tokio::test]
    async fn what_could_not_be_queued_is_owed_again() {
        let s = Store::open_in_memory().await.unwrap();
        chats::ensure_dm(&s, "aa").await.unwrap();
        message(&s, "a1", "dm:aa", "in", 100).await;
        message(&s, "a2", "dm:aa", "in", 200).await;
        let due = take_due_delivered(&s, 1_000, 50).await.unwrap();
        assert_eq!(due, vec![("dm:aa".into(), vec!["a1".into(), "a2".into()])]);
        owe_delivered_again(&s, &due[0].1).await.unwrap();
        assert_eq!(take_due_delivered(&s, 1_002, 50).await.unwrap(), due, "the next take hands them out again");

        assert_eq!(chats::mark_read(&s, "dm:aa").await.unwrap(), Some(200));
        let read = take_due_read(&s).await.unwrap();
        assert_eq!(read, vec![("dm:aa".into(), 200)]);
        owe_read_again(&s, "dm:aa").await.unwrap();
        assert_eq!(take_due_read(&s).await.unwrap(), read);
        assert!(take_due_read(&s).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_take_reads_the_index_of_what_is_owed() {
        let s = Store::open_in_memory().await.unwrap();
        let plan = |sql: String| {
            let s = s.clone();
            async move {
                sqlx::query_as::<_, (i64, i64, i64, String)>(sqlx::AssertSqlSafe(format!("EXPLAIN QUERY PLAN {sql}")))
                    .bind(0i64)
                    .bind(0i64)
                    .bind(DELIVERED_IDS_PER_CHAT)
                    .fetch_all(s.pool())
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|r| r.3)
                    .collect::<Vec<_>>()
            }
        };
        let probe = plan(format!("SELECT EXISTS (SELECT 1 FROM msg_messages WHERE {OWED_DELIVERED})")).await;
        assert!(probe.iter().any(|d| d.contains("msg_messages_unacked")), "{probe:?}");
        let take = plan(format!(
            "SELECT id FROM (SELECT id, ROW_NUMBER() OVER (PARTITION BY chat_id ORDER BY created_at, id) AS n
             FROM msg_messages WHERE {OWED_DELIVERED}) WHERE n <= ?3 AND ?2 = ?2"
        ))
        .await;
        assert!(take.iter().any(|d| d.contains("msg_messages_unacked")), "{take:?}");
    }
}
