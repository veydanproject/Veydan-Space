// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_avatar_cache`: the avatars of others by their address. A row says
//! which file holds the picture (`avatars/<sha256>.jpg`) and when it was
//! fetched, or how often and when the fetch last failed, and when it was
//! last shown. Several addresses may name the same file. Addresses not
//! shown for long are forgotten (`sweep`).

use crate::{storage, Store};
use messenger_core::Result;

#[derive(Clone, Debug, Default, PartialEq, Eq, sqlx::FromRow)]
pub struct AvatarCacheRow {
    pub url: String,
    /// The file's name; None while nothing was fetched.
    pub sha256: Option<String>,
    pub fetched_at: i64,
    pub failed_at: i64,
    /// Failures since the last success.
    pub attempts: i64,
    /// When the picture was last shown (`mark_used`).
    pub used_at: i64,
}

pub async fn get(store: &Store, url: &str) -> Result<Option<AvatarCacheRow>> {
    sqlx::query_as::<_, AvatarCacheRow>(
        "SELECT url, sha256, fetched_at, failed_at, attempts, used_at FROM msg_avatar_cache WHERE url = ?",
    )
    .bind(url)
    .fetch_optional(store.pool())
    .await
    .map_err(storage)
}

/// The picture at `url` is now the file `sha256`; failures are forgotten.
pub async fn put_fetched(store: &Store, url: &str, sha256: &str, at: i64) -> Result<()> {
    sqlx::query(
        "INSERT INTO msg_avatar_cache (url, sha256, fetched_at, failed_at, attempts, used_at) VALUES (?, ?, ?, 0, 0, ?)
         ON CONFLICT(url) DO UPDATE SET sha256 = excluded.sha256, fetched_at = excluded.fetched_at, failed_at = 0, attempts = 0,
         used_at = max(msg_avatar_cache.used_at, excluded.used_at)",
    )
    .bind(url)
    .bind(sha256)
    .bind(at)
    .bind(at)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// A fetch of `url` failed at `at`. A file fetched before stays named.
/// Returns the failures since the last success.
pub async fn put_failed(store: &Store, url: &str, at: i64) -> Result<i64> {
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO msg_avatar_cache (url, failed_at, attempts) VALUES (?, ?, 1)
         ON CONFLICT(url) DO UPDATE SET failed_at = excluded.failed_at, attempts = msg_avatar_cache.attempts + 1
         RETURNING attempts",
    )
    .bind(url)
    .bind(at)
    .fetch_one(store.pool())
    .await
    .map_err(storage)
}

/// The picture of `url` was shown at `at`.
pub async fn mark_used(store: &Store, url: &str, at: i64) -> Result<()> {
    sqlx::query("UPDATE msg_avatar_cache SET used_at = ? WHERE url = ? AND used_at < ?")
        .bind(at)
        .bind(url)
        .bind(at)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn remove(store: &Store, url: &str) -> Result<()> {
    sqlx::query("DELETE FROM msg_avatar_cache WHERE url = ?")
        .bind(url)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// Whether any address still names the file `sha256`.
pub async fn sha_in_use(store: &Store, sha256: &str) -> Result<bool> {
    sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM msg_avatar_cache WHERE sha256 = ?)")
        .bind(sha256)
        .fetch_one(store.pool())
        .await
        .map(|n| n != 0)
        .map_err(storage)
}

/// Forget the addresses last shown, fetched or tried before `before`, and
/// all but the `keep` used most lately. Returns the files no address names
/// any more, to be deleted.
pub async fn sweep(store: &Store, before: i64, keep: i64) -> Result<Vec<String>> {
    let mut tx = store.pool().begin_with("BEGIN IMMEDIATE").await.map_err(storage)?;
    let gone: Vec<Option<String>> = sqlx::query_scalar(
        "DELETE FROM msg_avatar_cache
         WHERE max(used_at, fetched_at, failed_at) < ?
            OR url NOT IN (SELECT url FROM msg_avatar_cache ORDER BY max(used_at, fetched_at, failed_at) DESC, url LIMIT ?)
         RETURNING sha256",
    )
    .bind(before)
    .bind(keep.max(0))
    .fetch_all(&mut *tx)
    .await
    .map_err(storage)?;
    let mut free = Vec::new();
    for sha in gone.into_iter().flatten() {
        if free.contains(&sha) {
            continue;
        }
        let named: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM msg_avatar_cache WHERE sha256 = ?)")
            .bind(&sha)
            .fetch_one(&mut *tx)
            .await
            .map_err(storage)?;
        if named == 0 {
            free.push(sha);
        }
    }
    tx.commit().await.map_err(storage)?;
    Ok(free)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fetched_failed_and_removed() {
        let s = Store::open_in_memory().await.unwrap();
        let (u1, u2, sha) = ("https://a.example/1.png", "https://b.example/2.png", "ab".repeat(32));
        assert_eq!(get(&s, u1).await.unwrap(), None);

        assert_eq!(put_failed(&s, u1, 10).await.unwrap(), 1);
        assert_eq!(put_failed(&s, u1, 20).await.unwrap(), 2);
        let row = get(&s, u1).await.unwrap().unwrap();
        assert_eq!((row.sha256, row.failed_at, row.attempts), (None, 20, 2));

        put_fetched(&s, u1, &sha, 30).await.unwrap();
        let row = get(&s, u1).await.unwrap().unwrap();
        assert_eq!(row, AvatarCacheRow { url: u1.into(), sha256: Some(sha.clone()), fetched_at: 30, failed_at: 0, attempts: 0, used_at: 30 });

        // A later failure keeps the file it has.
        assert_eq!(put_failed(&s, u1, 40).await.unwrap(), 1);
        assert_eq!(get(&s, u1).await.unwrap().unwrap().sha256.as_deref(), Some(sha.as_str()));

        put_fetched(&s, u2, &sha, 50).await.unwrap();
        remove(&s, u1).await.unwrap();
        assert_eq!(get(&s, u1).await.unwrap(), None);
        assert!(sha_in_use(&s, &sha).await.unwrap(), "u2 still names it");
        remove(&s, u2).await.unwrap();
        assert!(!sha_in_use(&s, &sha).await.unwrap());
    }

    #[tokio::test]
    async fn the_cache_forgets_what_nobody_looks_at() {
        let s = Store::open_in_memory().await.unwrap();
        let (a, b, c) = ("aa".repeat(32), "bb".repeat(32), "cc".repeat(32));
        put_fetched(&s, "https://x.example/old", &a, 10).await.unwrap();
        put_fetched(&s, "https://x.example/shown", &b, 10).await.unwrap();
        put_fetched(&s, "https://y.example/same-file", &b, 10).await.unwrap();
        put_fetched(&s, "https://x.example/new", &c, 90).await.unwrap();
        put_failed(&s, "https://x.example/failed", 10).await.unwrap();
        mark_used(&s, "https://x.example/shown", 95).await.unwrap();
        mark_used(&s, "https://x.example/shown", 50).await.unwrap();
        assert_eq!(get(&s, "https://x.example/shown").await.unwrap().unwrap().used_at, 95, "never back");

        let mut free = sweep(&s, 50, 100).await.unwrap();
        free.sort();
        assert_eq!(free, vec![a.clone()], "the other address still names b");
        assert_eq!(get(&s, "https://x.example/old").await.unwrap(), None);
        assert_eq!(get(&s, "https://x.example/failed").await.unwrap(), None);
        assert!(get(&s, "https://x.example/shown").await.unwrap().is_some());
        assert!(sha_in_use(&s, &b).await.unwrap());

        // At most `keep`, the latest used kept.
        assert_eq!(sweep(&s, 0, 1).await.unwrap(), vec![c.clone()]);
        assert!(get(&s, "https://x.example/shown").await.unwrap().is_some());
        assert_eq!(get(&s, "https://x.example/new").await.unwrap(), None);
        assert_eq!(sweep(&s, 100, 10).await.unwrap(), vec![b], "the last ones");
        assert!(sweep(&s, 100, 10).await.unwrap().is_empty());
    }
}
