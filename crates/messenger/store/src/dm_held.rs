// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_dm_held`: messages of a peer the DM gate stored hidden instead of
//! dropping them (a second message before approval), until a later end of
//! the episode lets one of them through. Meaning lives in `messenger-dm`.

use crate::{storage, Store};
use messenger_core::Result;

/// Mark a stored hidden row as held.
pub async fn hold(store: &Store, message_id: &str, chat_id: &str, created_at: i64) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO msg_dm_held (message_id, chat_id, created_at) VALUES (?, ?, ?)")
        .bind(message_id)
        .bind(chat_id)
        .bind(created_at)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn count(store: &Store, chat_id: &str) -> Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM msg_dm_held WHERE chat_id = ?")
        .bind(chat_id)
        .fetch_one(store.pool())
        .await
        .map_err(storage)
}

pub async fn is_held(store: &Store, message_id: &str) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>("SELECT 1 FROM msg_dm_held WHERE message_id = ?")
        .bind(message_id)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)?
        .is_some())
}

/// Held rows of the chat dated after `floor`, the oldest first.
pub async fn after(store: &Store, chat_id: &str, floor: i64) -> Result<Vec<String>> {
    sqlx::query_scalar::<_, String>(
        "SELECT message_id FROM msg_dm_held WHERE chat_id = ? AND created_at > ? ORDER BY created_at ASC, message_id ASC",
    )
    .bind(chat_id)
    .bind(floor)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// Show a held row: unmark it and make it visible. `false` when it was not
/// held (released already, or purged).
pub async fn release(store: &Store, message_id: &str) -> Result<bool> {
    let mut tx = store.pool().begin().await.map_err(storage)?;
    let was = sqlx::query("DELETE FROM msg_dm_held WHERE message_id = ?")
        .bind(message_id)
        .execute(&mut *tx)
        .await
        .map_err(storage)?
        .rows_affected()
        == 1;
    if was {
        sqlx::query("UPDATE msg_messages SET is_hidden = 0 WHERE id = ?")
            .bind(message_id)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
    }
    tx.commit().await.map_err(storage)?;
    Ok(was)
}

/// Forget the held rows of a chat, the hidden messages with them, as if they
/// had been dropped when they came. Returns how many there were.
pub async fn purge(store: &Store, chat_id: &str) -> Result<u64> {
    let mut tx = store.pool().begin().await.map_err(storage)?;
    sqlx::query("DELETE FROM msg_messages WHERE id IN (SELECT message_id FROM msg_dm_held WHERE chat_id = ?) AND is_hidden = 1")
        .bind(chat_id)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    let n = sqlx::query("DELETE FROM msg_dm_held WHERE chat_id = ?")
        .bind(chat_id)
        .execute(&mut *tx)
        .await
        .map_err(storage)?
        .rows_affected();
    tx.commit().await.map_err(storage)?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{self, NewMessage};

    fn row(id: &str, chat: &str, at: i64) -> NewMessage {
        NewMessage {
            id: id.into(),
            chat_id: chat.into(),
            wire_id: None,
            direction: messages::DIR_IN.into(),
            status: messages::STATUS_RECEIVED.into(),
            content_type: messages::CT_TEXT.into(),
            text: Some(id.into()),
            envelope_json: "{}".into(),
            sender_pubkey: "p".into(),
            reply_to_id: None,
            target_id: None,
            created_at: at,
            is_hidden: true,
            outbox_local_id: None,
            media_json: None,
        }
    }

    #[tokio::test]
    async fn held_rows_are_released_one_by_one_and_purged_per_chat() {
        let s = Store::open_in_memory().await.unwrap();
        for (id, chat, at) in [("a", "dm:p", 30), ("b", "dm:p", 20), ("c", "dm:p", 10), ("d", "dm:q", 40)] {
            messages::insert(&s, &row(id, chat, at)).await.unwrap();
            hold(&s, id, chat, at).await.unwrap();
        }
        hold(&s, "a", "dm:p", 30).await.unwrap();
        assert_eq!(count(&s, "dm:p").await.unwrap(), 3, "a mark is kept once");
        assert_eq!(after(&s, "dm:p", 10).await.unwrap(), vec!["b".to_string(), "a".to_string()], "oldest first, above the floor");

        assert!(release(&s, "b").await.unwrap());
        assert!(!release(&s, "b").await.unwrap(), "once");
        assert!(!messages::get(&s, "b").await.unwrap().unwrap().is_hidden);
        assert!(!is_held(&s, "b").await.unwrap());
        assert_eq!(messages::count_visible_incoming_since(&s, "dm:p", 0).await.unwrap(), 1);

        assert_eq!(purge(&s, "dm:p").await.unwrap(), 2);
        assert!(messages::get(&s, "a").await.unwrap().is_none() && messages::get(&s, "c").await.unwrap().is_none());
        assert!(messages::get(&s, "b").await.unwrap().is_some(), "a shown row is not purged");
        assert!(is_held(&s, "d").await.unwrap(), "another chat keeps its own");
    }
}
