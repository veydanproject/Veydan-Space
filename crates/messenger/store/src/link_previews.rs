// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Previews of pages outside that the user asked for (`msg_link_previews`).

use crate::{storage, Store};
use messenger_core::Result;

/// The stored preview, unless it is older than `not_before`.
pub async fn get(store: &Store, url: &str, not_before: i64) -> Result<Option<String>> {
    sqlx::query_scalar::<_, String>("SELECT preview_json FROM msg_link_previews WHERE url = ? AND fetched_at >= ?")
        .bind(url)
        .bind(not_before)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)
}

pub async fn put(store: &Store, url: &str, preview_json: &str, now: i64) -> Result<()> {
    sqlx::query(
        "INSERT INTO msg_link_previews (url, preview_json, fetched_at) VALUES (?, ?, ?)
         ON CONFLICT(url) DO UPDATE SET preview_json = excluded.preview_json, fetched_at = excluded.fetched_at",
    )
    .bind(url)
    .bind(preview_json)
    .bind(now)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// Drops what is older than `not_before` and, of the rest, all but the newest `keep`.
pub async fn prune(store: &Store, not_before: i64, keep: i64) -> Result<()> {
    sqlx::query(
        "DELETE FROM msg_link_previews WHERE fetched_at < ?
         OR url NOT IN (SELECT url FROM msg_link_previews ORDER BY fetched_at DESC, url LIMIT ?)",
    )
    .bind(not_before)
    .bind(keep)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn kept_replaced_and_pruned() {
        let store = Store::open_in_memory().await.unwrap();
        assert_eq!(get(&store, "https://a.example/", 0).await.unwrap(), None);
        put(&store, "https://a.example/", "{\"v\":1}", 100).await.unwrap();
        put(&store, "https://a.example/", "{\"v\":2}", 200).await.unwrap();
        assert_eq!(get(&store, "https://a.example/", 200).await.unwrap().as_deref(), Some("{\"v\":2}"));
        assert_eq!(get(&store, "https://a.example/", 201).await.unwrap(), None, "too old to be shown");

        put(&store, "https://b.example/", "{}", 300).await.unwrap();
        put(&store, "https://c.example/", "{}", 400).await.unwrap();
        prune(&store, 0, 2).await.unwrap();
        assert_eq!(get(&store, "https://a.example/", 0).await.unwrap(), None, "the oldest goes first");
        assert!(get(&store, "https://b.example/", 0).await.unwrap().is_some());
        prune(&store, 350, 10).await.unwrap();
        assert_eq!(get(&store, "https://b.example/", 0).await.unwrap(), None);
        assert!(get(&store, "https://c.example/", 0).await.unwrap().is_some());
    }
}
