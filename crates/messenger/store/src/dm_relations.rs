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
    /// Peer's rumor time where the last episode ended (migration 022).
    pub request_floor: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

pub async fn get(store: &Store, peer: &str) -> Result<Option<RelationRow>> {
    sqlx::query_as::<_, RelationRow>(
        "SELECT peer_pubkey, my_contact, blocked, peer_signal, was_ever_mutual, last_signal_at, last_my_signal_at, request_floor, created_at, updated_at
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
        "INSERT INTO msg_dm_relations (peer_pubkey, my_contact, blocked, peer_signal, was_ever_mutual, last_signal_at, last_my_signal_at, request_floor, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(peer_pubkey) DO UPDATE SET
           my_contact = excluded.my_contact, blocked = excluded.blocked, peer_signal = excluded.peer_signal,
           was_ever_mutual = excluded.was_ever_mutual, last_signal_at = excluded.last_signal_at,
           last_my_signal_at = excluded.last_my_signal_at, request_floor = excluded.request_floor,
           updated_at = excluded.updated_at",
    )
    .bind(&r.peer_pubkey)
    .bind(&r.my_contact)
    .bind(r.blocked)
    .bind(&r.peer_signal)
    .bind(r.was_ever_mutual)
    .bind(r.last_signal_at)
    .bind(r.last_my_signal_at)
    .bind(r.request_floor)
    .bind(now)
    .bind(now)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// The time of an episode end of the peer that no stored message of theirs
/// has accounted for yet (migration 026); 0 when none waits.
pub async fn end_pending(store: &Store, peer: &str) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>("SELECT request_end_pending FROM msg_dm_relations WHERE peer_pubkey = ?")
        .bind(peer)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)?
        .unwrap_or(0))
}

/// Set (or with 0 clear) the waiting episode end of a peer. `put` keeps it.
pub async fn set_end_pending(store: &Store, peer: &str, at: i64) -> Result<()> {
    let now = crate::now();
    sqlx::query(
        "INSERT INTO msg_dm_relations (peer_pubkey, request_end_pending, created_at, updated_at) VALUES (?, ?, ?, ?)
         ON CONFLICT(peer_pubkey) DO UPDATE SET request_end_pending = excluded.request_end_pending",
    )
    .bind(peer)
    .bind(at)
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

/// Peers I approved and did not block: who belongs in my address book.
pub async fn approved_peers(store: &Store) -> Result<Vec<String>> {
    sqlx::query_scalar::<_, String>(
        "SELECT peer_pubkey FROM msg_dm_relations WHERE my_contact = 'approved' AND blocked = 0 ORDER BY updated_at DESC",
    )
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn put_get_and_the_blocked_and_approved_lists() {
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
            request_floor: 0,
            created_at: 0,
            updated_at: 0,
        };
        put(&s, &r).await.unwrap();
        r.blocked = true;
        r.peer_signal = "approved".into();
        r.last_signal_at = 7;
        r.request_floor = 5;
        put(&s, &r).await.unwrap();
        let got = get(&s, "p").await.unwrap().unwrap();
        assert!(got.blocked);
        assert_eq!(got.peer_signal, "approved");
        assert_eq!(got.last_signal_at, 7);
        assert_eq!(got.request_floor, 5);
        assert_eq!(blocked_peers(&s).await.unwrap(), vec!["p".to_string()]);
        assert!(approved_peers(&s).await.unwrap().is_empty(), "blocked is not approved");
        r.blocked = false;
        put(&s, &r).await.unwrap();
        r.peer_pubkey = "q".into();
        r.my_contact = "declined".into();
        put(&s, &r).await.unwrap();
        assert_eq!(approved_peers(&s).await.unwrap(), vec!["p".to_string()]);
    }

    #[tokio::test]
    async fn a_waiting_episode_end_survives_a_put_and_is_cleared_apart() {
        let s = Store::open_in_memory().await.unwrap();
        assert_eq!(end_pending(&s, "p").await.unwrap(), 0);
        set_end_pending(&s, "p", 70).await.unwrap();
        assert_eq!(end_pending(&s, "p").await.unwrap(), 70, "a row is made for it");
        let r = get(&s, "p").await.unwrap().unwrap();
        assert_eq!((r.my_contact.as_str(), r.peer_signal.as_str(), r.request_floor), ("none", "none", 0));
        put(&s, &RelationRow { last_signal_at: 9, ..r }).await.unwrap();
        assert_eq!(end_pending(&s, "p").await.unwrap(), 70, "put keeps it");
        set_end_pending(&s, "p", 0).await.unwrap();
        assert_eq!(end_pending(&s, "p").await.unwrap(), 0);
        assert_eq!(get(&s, "p").await.unwrap().unwrap().last_signal_at, 9);
    }

    #[tokio::test]
    async fn migration_024_undoes_the_floor_of_my_no_and_caps_a_future_one() {
        let s = Store::open_in_memory().await.unwrap();
        let now = crate::now();
        let row = |peer: &str, mine: &str, theirs: &str, floor: i64| RelationRow {
            peer_pubkey: peer.into(),
            my_contact: mine.into(),
            blocked: false,
            peer_signal: theirs.into(),
            was_ever_mutual: false,
            last_signal_at: 0,
            last_my_signal_at: 0,
            request_floor: floor,
            created_at: 0,
            updated_at: 0,
        };
        for r in [
            row("declined", "declined", "none", 500),
            row("declined-then-left", "declined", "left", 500),
            row("removed", "none", "left", 600),
            row("future", "none", "revoked", 4_102_444_800),
        ] {
            put(&s, &r).await.unwrap();
        }
        sqlx::raw_sql(include_str!("../migrations/024_dm_request_floor_fix.sql")).execute(s.pool()).await.unwrap();
        let floor = |p: &'static str| {
            let s = s.clone();
            async move { get(&s, p).await.unwrap().unwrap().request_floor }
        };
        assert_eq!(floor("declined").await, 0, "my no ends no episode");
        assert_eq!(floor("declined-then-left").await, 500, "their removal after my no does");
        assert_eq!(floor("removed").await, 600);
        let capped = floor("future").await;
        assert!(capped >= now + 300 && capped <= crate::now() + 300, "{capped}");
    }
}
