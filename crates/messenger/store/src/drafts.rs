// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The unsent text of each conversation (`msg_drafts`), on this device
//! only: it outlives the app, goes nowhere, and is wiped with the account.

use crate::{now, storage, Store};
use messenger_core::Result;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DraftRow {
    pub chat: String,
    pub text: String,
    pub md: bool,
}

/// Every draft kept, the newest first.
pub async fn list(store: &Store) -> Result<Vec<DraftRow>> {
    let rows: Vec<(String, String, bool)> =
        sqlx::query_as("SELECT chat, text, md FROM msg_drafts ORDER BY updated_at DESC, chat")
            .fetch_all(store.pool())
            .await
            .map_err(storage)?;
    Ok(rows.into_iter().map(|(chat, text, md)| DraftRow { chat, text, md }).collect())
}

/// Keep `text` as the draft of `chat`; a text of blanks forgets it.
pub async fn set(store: &Store, chat: &str, text: &str, md: bool) -> Result<()> {
    if text.trim().is_empty() {
        return delete(store, chat).await;
    }
    sqlx::query(
        "INSERT INTO msg_drafts (chat, text, md, updated_at) VALUES (?, ?, ?, ?)
         ON CONFLICT(chat) DO UPDATE SET text = excluded.text, md = excluded.md, updated_at = excluded.updated_at",
    )
    .bind(chat)
    .bind(text)
    .bind(md)
    .bind(now())
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

pub async fn delete(store: &Store, chat: &str) -> Result<()> {
    sqlx::query("DELETE FROM msg_drafts WHERE chat = ?")
        .bind(chat)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_draft_is_kept_per_chat_and_blanks_forget_it() {
        let store = Store::open_in_memory().await.unwrap();
        assert!(list(&store).await.unwrap().is_empty());
        set(&store, "aa", "QA1105 draft", false).await.unwrap();
        set(&store, "group:bb", "**bold**", true).await.unwrap();
        set(&store, "aa", "QA1105 draft, more", false).await.unwrap();
        let mut kept = list(&store).await.unwrap();
        kept.sort_by(|a, b| a.chat.cmp(&b.chat));
        assert_eq!(
            kept,
            vec![
                DraftRow { chat: "aa".into(), text: "QA1105 draft, more".into(), md: false },
                DraftRow { chat: "group:bb".into(), text: "**bold**".into(), md: true },
            ]
        );
        set(&store, "aa", "  \n", false).await.unwrap();
        delete(&store, "group:bb").await.unwrap();
        assert!(list(&store).await.unwrap().is_empty());
    }
}
