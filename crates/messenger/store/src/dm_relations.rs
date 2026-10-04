// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_dm_relations`: raw relationship state per peer. Meaning and
//! transitions live in `messenger-dm`; this is storage only.

use crate::{storage, Store};
use messenger_core::Result;

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct RelationRow {
    pub peer_pubkey: String,
    pub my_contact: String,
    pub blocked: bool,
    pub peer_signal: String,
    pub was_ever_mutual: bool,
    pub last_signal_at: i64,
    pub last_my_signal_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

pub async fn get(store: &Store, peer: &str) -> Result<Option<RelationRow>> {
    sqlx::query_as::<_, RelationRow>(
        "SELECT peer_pubkey, my_contact, blocked, peer_signal, was_ever_mutual, last_signal_at, last_my_signal_at, created_at, updated_at
         FROM msg_dm_relations WHERE peer_pubkey = ?",
    )
    .bind(peer)
    .fetch_optional(store.pool())
    .await
    .map_err(storage)
}

/// Insert or replace the whole state of one peer.
pub async fn put(store: &Store, r: &RelationRow) -> Result<()> {
    let now = crate::now();
    sqlx::query(
        "INSERT INTO msg_dm_relations (peer_pubkey, my_contact, blocked, peer_signal, was_ever_mutual, last_signal_at, last_my_signal_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(peer_pubkey) DO UPDATE SET
           my_contact = excluded.my_contact, blocked = excluded.blocked, peer_signal = excluded.peer_signal,
           was_ever_mutual = excluded.was_ever_mutual, last_signal_at = excluded.last_signal_at,
           last_my_signal_at = excluded.last_my_signal_at, updated_at = excluded.updated_at",
    )
    .bind(&r.peer_pubkey)
    .bind(&r.my_contact)
    .bind(r.blocked)
    .bind(&r.peer_signal)
    .bind(r.was_ever_mutual)
    .bind(r.last_signal_at)
    .bind(r.last_my_signal_at)
    .bind(now)
    .bind(now)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

pub async fn blocked_peers(store: &Store) -> Result<Vec<String>> {
    sqlx::query_scalar::<_, String>("SELECT peer_pubkey FROM msg_dm_relations WHERE blocked = 1 ORDER BY updated_at DESC")
        .fetch_all(store.pool())
        .await
        .map_err(storage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn put_get_and_blocked_list() {
        let s = Store::open_in_memory().await.unwrap();
        assert!(get(&s, "p").await.unwrap().is_none());
        let mut r = RelationRow {
            peer_pubkey: "p".into(),
            my_contact: "approved".into(),
            blocked: false,
            peer_signal: "none".into(),
            was_ever_mutual: false,
            last_signal_at: 0,
            last_my_signal_at: 0,
            created_at: 0,
            updated_at: 0,
        };
        put(&s, &r).await.unwrap();
        r.blocked = true;
        r.peer_signal = "approved".into();
        r.last_signal_at = 7;
        put(&s, &r).await.unwrap();
        let got = get(&s, "p").await.unwrap().unwrap();
        assert!(got.blocked);
        assert_eq!(got.peer_signal, "approved");
        assert_eq!(got.last_signal_at, 7);
        assert_eq!(blocked_peers(&s).await.unwrap(), vec!["p".to_string()]);
    }
}
