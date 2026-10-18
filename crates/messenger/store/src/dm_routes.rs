// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_dm_routes`: where each peer wants DMs delivered. NIP-17 inbox
//! lists (kind 10050) win over NIP-65 relay lists (kind 10002); within a
//! source only a newer event replaces the stored set.
//!
//! Kept per server (migration 040): a list heard on a private server names
//! that server's relays, and is kept apart from the home set's, which is
//! the one home messages are delivered by (`for_peer` with `None`). A
//! private chat needs none: it goes to every relay of its server.

use crate::dm_relations::server_key;
use crate::{storage, Store};
use messenger_core::Result;

pub const SOURCE_NIP17: &str = "nip17";
pub const SOURCE_NIP65: &str = "nip65";

/// Replace the peer's routes from `source`, as heard on `server` (`None`:
/// the home set), when `event_created_at` is newer than what we have for
/// that source there. Returns `true` when applied.
pub async fn replace(
    store: &Store,
    peer: &str,
    server: Option<&str>,
    source: &str,
    event_created_at: i64,
    relays: &[String],
) -> Result<bool> {
    let server = server_key(server);
    let newest = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(event_created_at) FROM msg_dm_routes WHERE peer_pubkey = ? AND server_id = ? AND source = ?",
    )
    .bind(peer)
    .bind(server)
    .bind(source)
    .fetch_one(store.pool())
    .await
    .map_err(storage)?;
    if newest.is_some_and(|n| n >= event_created_at) {
        return Ok(false);
    }
    let mut tx = store.pool().begin().await.map_err(storage)?;
    sqlx::query("DELETE FROM msg_dm_routes WHERE peer_pubkey = ? AND server_id = ? AND source = ?")
        .bind(peer)
        .bind(server)
        .bind(source)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    let now = crate::now();
    for url in relays {
        sqlx::query(
            "INSERT INTO msg_dm_routes (peer_pubkey, server_id, relay_url, source, event_created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(peer_pubkey, server_id, relay_url) DO UPDATE SET source = excluded.source,
               event_created_at = excluded.event_created_at, updated_at = excluded.updated_at",
        )
        .bind(peer)
        .bind(server)
        .bind(url)
        .bind(source)
        .bind(event_created_at)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    }
    tx.commit().await.map_err(storage)?;
    Ok(true)
}

/// Delivery candidates on `server` (`None`: the home set): NIP-17 inbox
/// relays if any, else NIP-65 ones.
pub async fn for_peer(store: &Store, peer: &str, server: Option<&str>) -> Result<Vec<String>> {
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT relay_url, source FROM msg_dm_routes WHERE peer_pubkey = ? AND server_id = ? ORDER BY updated_at DESC",
    )
    .bind(peer)
    .bind(server_key(server))
    .fetch_all(store.pool())
    .await
    .map_err(storage)?;
    let nip17: Vec<String> = rows.iter().filter(|(_, s)| s == SOURCE_NIP17).map(|(u, _)| u.clone()).collect();
    if !nip17.is_empty() {
        return Ok(nip17);
    }
    Ok(rows.into_iter().map(|(u, _)| u).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn nip17_wins_and_only_newer_events_apply() {
        let s = Store::open_in_memory().await.unwrap();
        assert!(replace(&s, "p", None, SOURCE_NIP65, 10, &["wss://a".into(), "wss://b".into()]).await.unwrap());
        assert_eq!(for_peer(&s, "p", None).await.unwrap().len(), 2);
        assert!(replace(&s, "p", None, SOURCE_NIP17, 5, &["wss://inbox".into()]).await.unwrap());
        assert_eq!(for_peer(&s, "p", None).await.unwrap(), vec!["wss://inbox".to_string()]);
        assert!(!replace(&s, "p", None, SOURCE_NIP17, 5, &["wss://old".into()]).await.unwrap(), "same age ignored");
        assert!(!replace(&s, "p", None, SOURCE_NIP17, 4, &["wss://older".into()]).await.unwrap());
        assert!(replace(&s, "p", None, SOURCE_NIP17, 6, &[]).await.unwrap(), "newer empty list clears nip17");
        assert_eq!(for_peer(&s, "p", None).await.unwrap().len(), 2, "falls back to nip65");
        assert!(for_peer(&s, "unknown", None).await.unwrap().is_empty());
    }

    /// A list the peer keeps on a private server never takes the place of
    /// the home one, whatever its age.
    #[tokio::test]
    async fn a_private_servers_list_is_kept_apart() {
        let s = Store::open_in_memory().await.unwrap();
        assert!(replace(&s, "p", None, SOURCE_NIP17, 5, &["wss://home".into()]).await.unwrap());
        assert!(replace(&s, "p", Some("s1"), SOURCE_NIP17, 9, &["wss://s1".into()]).await.unwrap());
        assert_eq!(for_peer(&s, "p", None).await.unwrap(), vec!["wss://home".to_string()]);
        assert_eq!(for_peer(&s, "p", Some("s1")).await.unwrap(), vec!["wss://s1".to_string()]);
        assert!(replace(&s, "p", None, SOURCE_NIP17, 6, &["wss://home2".into()]).await.unwrap(), "each server has its own age");
    }
}
