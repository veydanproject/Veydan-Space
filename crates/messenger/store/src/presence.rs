// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Presence: the keys my peers beat from, their newest beats, and which
//! epoch of my own key each peer was told. Keys go by the later `since`,
//! beats only forward, so notes and beats may come in any order.

use crate::{storage, Store};
use messenger_core::Result;
use sqlx::{Sqlite, Transaction};

/// What `msg_presence_keys` holds for a peer that stopped sharing: no key,
/// and the `since` of the withdrawal, so a key told before it and heard
/// after it does not come back.
const WITHDRAWN: &str = "";

/// `peer` told me its presence key as of `since`; `None` means it stopped
/// sharing. The later `since` wins, whatever the order the notes come in.
/// At the same `since` a withdrawal beats a key, and of two keys the
/// larger one wins, so every device of mine ends with the same. `true`
/// when the key I watch for the peer changed: a first key, another one, or
/// none any more. A later `since` for the same key, or for a withdrawal, is
/// kept but is no change. The beats of a key that is gone go with it.
pub async fn put_key(store: &Store, peer: &str, presence_pubkey: Option<&str>, since: i64) -> Result<bool> {
    // It reads before it writes: a deferred transaction would fail (busy,
    // not waited for) when another connection writes in between.
    let mut tx = store.pool().begin_with("BEGIN IMMEDIATE").await.map_err(storage)?;
    let old = sqlx::query_as::<_, (String, i64)>("SELECT presence_pubkey, since FROM msg_presence_keys WHERE peer = ?")
        .bind(peer)
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage)?;
    let new = presence_pubkey.unwrap_or(WITHDRAWN);
    let wins = match &old {
        None => true,
        Some((old_key, old_since)) => {
            since > *old_since || (since == *old_since && old_key.as_str() != WITHDRAWN && (new == WITHDRAWN || new > old_key.as_str()))
        }
    };
    if !wins {
        tx.rollback().await.map_err(storage)?;
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO msg_presence_keys (peer, presence_pubkey, since) VALUES (?, ?, ?)
         ON CONFLICT(peer) DO UPDATE SET presence_pubkey = excluded.presence_pubkey, since = excluded.since",
    )
    .bind(peer)
    .bind(new)
    .bind(since)
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    let old_key = old.map_or(WITHDRAWN.to_string(), |(k, _)| k);
    let changed = old_key != new;
    if changed && old_key != WITHDRAWN {
        drop_orphan_beat(&mut tx, &old_key).await?;
    }
    tx.commit().await.map_err(storage)?;
    Ok(changed)
}

/// The beat of a key no peer names any more is nobody's.
async fn drop_orphan_beat(tx: &mut Transaction<'_, Sqlite>, presence_pubkey: &str) -> Result<()> {
    sqlx::query(
        "DELETE FROM msg_presence WHERE presence_pubkey = ?1
         AND NOT EXISTS (SELECT 1 FROM msg_presence_keys WHERE presence_pubkey = ?1)",
    )
    .bind(presence_pubkey)
    .execute(&mut **tx)
    .await
    .map_err(storage)?;
    Ok(())
}

/// The presence key `peer` told me and since when; `None` when it told
/// none or stopped sharing.
pub async fn key_of(store: &Store, peer: &str) -> Result<Option<(String, i64)>> {
    sqlx::query_as::<_, (String, i64)>(
        "SELECT presence_pubkey, since FROM msg_presence_keys WHERE peer = ? AND presence_pubkey <> ''",
    )
    .bind(peer)
    .fetch_optional(store.pool())
    .await
    .map_err(storage)
}

/// Every `(peer, presence_pubkey)` I know, by peer: what to subscribe to.
pub async fn keys(store: &Store) -> Result<Vec<(String, String)>> {
    sqlx::query_as::<_, (String, String)>(
        "SELECT peer, presence_pubkey FROM msg_presence_keys WHERE presence_pubkey <> '' ORDER BY peer",
    )
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// Whose presence key this is. Should two peers name the same key, the one
/// that named it first keeps it: a later claim cannot take over another
/// contact's beats.
pub async fn peer_of(store: &Store, presence_pubkey: &str) -> Result<Option<String>> {
    if presence_pubkey == WITHDRAWN {
        return Ok(None);
    }
    sqlx::query_scalar::<_, String>(
        "SELECT peer FROM msg_presence_keys WHERE presence_pubkey = ? ORDER BY since, peer LIMIT 1",
    )
    .bind(presence_pubkey)
    .fetch_optional(store.pool())
    .await
    .map_err(storage)
}

/// A beat of `presence_pubkey` made at `seen_at`, online until
/// `online_until`. Only a later beat moves it; `true` when it moved.
pub async fn seen(store: &Store, presence_pubkey: &str, seen_at: i64, online_until: i64) -> Result<bool> {
    let r = sqlx::query(
        "INSERT INTO msg_presence (presence_pubkey, seen_at, online_until) VALUES (?, ?, ?)
         ON CONFLICT(presence_pubkey) DO UPDATE SET seen_at = excluded.seen_at, online_until = excluded.online_until
         WHERE excluded.seen_at > msg_presence.seen_at",
    )
    .bind(presence_pubkey)
    .bind(seen_at)
    .bind(online_until)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(r.rows_affected() > 0)
}

/// `(peer, seen_at, online_until)` of every peer with a key and a beat, by
/// peer.
pub async fn snapshot(store: &Store) -> Result<Vec<(String, i64, i64)>> {
    sqlx::query_as::<_, (String, i64, i64)>(
        "SELECT k.peer, p.seen_at, p.online_until FROM msg_presence_keys k
         JOIN msg_presence p ON p.presence_pubkey = k.presence_pubkey
         WHERE k.presence_pubkey <> '' ORDER BY k.peer",
    )
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// The epoch of my presence key `peer` was last told, if any.
pub async fn told(store: &Store, peer: &str) -> Result<Option<u32>> {
    let epoch = sqlx::query_scalar::<_, i64>("SELECT epoch FROM msg_presence_told WHERE peer = ?")
        .bind(peer)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)?;
    Ok(epoch.and_then(|e| u32::try_from(e).ok()))
}

/// `peer` was told my key of `epoch`.
pub async fn mark_told(store: &Store, peer: &str, epoch: u32) -> Result<()> {
    sqlx::query(
        "INSERT INTO msg_presence_told (peer, epoch) VALUES (?, ?)
         ON CONFLICT(peer) DO UPDATE SET epoch = excluded.epoch",
    )
    .bind(peer)
    .bind(i64::from(epoch))
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// Nobody was told the current key: after a rotation everyone is told anew.
pub async fn forget_told(store: &Store) -> Result<()> {
    sqlx::query("DELETE FROM msg_presence_told").execute(store.pool()).await.map_err(storage)?;
    Ok(())
}

/// Every peer that was told some key of mine, by peer: whom to tell that
/// I stopped sharing.
pub async fn told_peers(store: &Store) -> Result<Vec<String>> {
    sqlx::query_scalar::<_, String>("SELECT peer FROM msg_presence_told ORDER BY peer")
        .fetch_all(store.pool())
        .await
        .map_err(storage)
}

/// Another device of mine moved to `epoch` and tells the new key itself:
/// whom this device told before counts as told, so it does not tell anyone
/// the other device left out on purpose (a contact just removed there).
pub async fn carry_told(store: &Store, epoch: u32) -> Result<()> {
    sqlx::query("UPDATE msg_presence_told SET epoch = ?")
        .bind(i64::from(epoch))
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// Everything of `peer`: its key, its beat, what it was told. On remove or
/// block, so a contact that comes back starts clean.
pub async fn forget_peer(store: &Store, peer: &str) -> Result<()> {
    let mut tx = store.pool().begin().await.map_err(storage)?;
    let old = sqlx::query_scalar::<_, String>("DELETE FROM msg_presence_keys WHERE peer = ? RETURNING presence_pubkey")
        .bind(peer)
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage)?;
    if let Some(key) = old {
        drop_orphan_beat(&mut tx, &key).await?;
    }
    sqlx::query("DELETE FROM msg_presence_told WHERE peer = ?")
        .bind(peer)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    tx.commit().await.map_err(storage)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const KA: &str = "aa";
    const KB: &str = "bb";

    #[tokio::test]
    async fn a_key_goes_by_the_later_since() {
        let s = Store::open_in_memory().await.unwrap();
        assert!(put_key(&s, "p", Some(KA), 10).await.unwrap(), "first key");
        assert_eq!(key_of(&s, "p").await.unwrap(), Some((KA.into(), 10)));
        assert!(!put_key(&s, "p", Some(KB), 9).await.unwrap(), "older loses");
        assert!(!put_key(&s, "p", Some(KA), 10).await.unwrap(), "the same again");
        assert!(!put_key(&s, "p", Some(KA), 11).await.unwrap(), "a later since of the same key is no change");
        assert_eq!(key_of(&s, "p").await.unwrap(), Some((KA.into(), 11)));
        assert!(put_key(&s, "p", Some(KB), 11).await.unwrap(), "same since, the larger key");
        assert_eq!(key_of(&s, "p").await.unwrap(), Some((KB.into(), 11)));
        assert!(!put_key(&s, "p", Some(KA), 11).await.unwrap(), "same since, the smaller key: whatever the order");
        assert_eq!(key_of(&s, "p").await.unwrap(), Some((KB.into(), 11)));
        assert!(put_key(&s, "p", Some(KA), 12).await.unwrap(), "newer wins");
        assert_eq!(key_of(&s, "p").await.unwrap(), Some((KA.into(), 12)));
    }

    #[tokio::test]
    async fn none_withdraws_a_key_not_older_than_itself() {
        let s = Store::open_in_memory().await.unwrap();
        assert!(!put_key(&s, "p", None, 5).await.unwrap(), "nothing to withdraw");
        put_key(&s, "p", Some(KA), 10).await.unwrap();
        assert!(!put_key(&s, "p", None, 9).await.unwrap(), "older withdrawal loses");
        assert!(key_of(&s, "p").await.unwrap().is_some());
        assert!(put_key(&s, "p", None, 10).await.unwrap(), "same since withdraws");
        assert_eq!(key_of(&s, "p").await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_key_told_before_a_withdrawal_and_heard_after_it_stays_out() {
        let s = Store::open_in_memory().await.unwrap();
        assert!(!put_key(&s, "p", None, 10).await.unwrap(), "nothing was watched");
        assert!(!put_key(&s, "p", Some(KA), 5).await.unwrap(), "the older key, late");
        assert!(!put_key(&s, "p", Some(KA), 10).await.unwrap(), "a key of the same second as the withdrawal");
        assert_eq!(key_of(&s, "p").await.unwrap(), None);
        assert!(keys(&s).await.unwrap().is_empty());
        assert!(!put_key(&s, "p", None, 12).await.unwrap(), "a later withdrawal is no change");
        assert!(!put_key(&s, "p", Some(KA), 11).await.unwrap(), "it moved the mark");
        assert!(put_key(&s, "p", Some(KB), 13).await.unwrap(), "a key after it");
        assert_eq!(keys(&s).await.unwrap(), vec![("p".into(), KB.into())]);
        seen(&s, KB, 100, 180).await.unwrap();
        assert!(put_key(&s, "p", None, 14).await.unwrap());
        assert!(snapshot(&s).await.unwrap().is_empty() && keys(&s).await.unwrap().is_empty());
        assert_eq!(peer_of(&s, "").await.unwrap(), None, "a withdrawal is nobody's key");
        // A peer that comes back starts clean.
        forget_peer(&s, "p").await.unwrap();
        assert!(put_key(&s, "p", Some(KA), 1).await.unwrap());
    }

    #[tokio::test]
    async fn a_replaced_or_withdrawn_key_takes_its_beat() {
        let s = Store::open_in_memory().await.unwrap();
        put_key(&s, "p", Some(KA), 10).await.unwrap();
        seen(&s, KA, 100, 180).await.unwrap();
        put_key(&s, "p", Some(KB), 11).await.unwrap();
        assert!(snapshot(&s).await.unwrap().is_empty(), "the old beat is gone");
        assert!(seen(&s, KA, 100, 180).await.unwrap(), "so the old key starts afresh");
        seen(&s, KB, 200, 280).await.unwrap();
        put_key(&s, "p", None, 12).await.unwrap();
        assert!(seen(&s, KB, 200, 280).await.unwrap(), "the withdrawn key's beat went too");
    }

    #[tokio::test]
    async fn keys_and_peer_of() {
        let s = Store::open_in_memory().await.unwrap();
        put_key(&s, "q", Some(KB), 1).await.unwrap();
        put_key(&s, "p", Some(KA), 1).await.unwrap();
        assert_eq!(keys(&s).await.unwrap(), vec![("p".into(), KA.into()), ("q".into(), KB.into())]);
        assert_eq!(peer_of(&s, KA).await.unwrap().as_deref(), Some("p"));
        assert_eq!(peer_of(&s, "cc").await.unwrap(), None);
        // A later claim of somebody else's key does not take it over.
        put_key(&s, "r", Some(KA), 5).await.unwrap();
        assert_eq!(peer_of(&s, KA).await.unwrap().as_deref(), Some("p"));
    }

    #[tokio::test]
    async fn a_beat_only_moves_forward() {
        let s = Store::open_in_memory().await.unwrap();
        assert!(seen(&s, KA, 100, 180).await.unwrap());
        assert!(!seen(&s, KA, 100, 999).await.unwrap(), "same time");
        assert!(!seen(&s, KA, 90, 170).await.unwrap(), "older");
        assert!(seen(&s, KA, 130, 210).await.unwrap());
        put_key(&s, "p", Some(KA), 1).await.unwrap();
        assert_eq!(snapshot(&s).await.unwrap(), vec![("p".into(), 130, 210)]);
    }

    #[tokio::test]
    async fn snapshot_joins_keys_and_beats() {
        let s = Store::open_in_memory().await.unwrap();
        put_key(&s, "q", Some(KB), 1).await.unwrap();
        put_key(&s, "p", Some(KA), 1).await.unwrap();
        put_key(&s, "r", Some("cc"), 1).await.unwrap(); // no beat yet
        seen(&s, KA, 100, 180).await.unwrap();
        seen(&s, KB, 50, 130).await.unwrap();
        seen(&s, "dd", 70, 150).await.unwrap(); // nobody's key
        assert_eq!(snapshot(&s).await.unwrap(), vec![("p".into(), 100, 180), ("q".into(), 50, 130)]);
    }

    #[tokio::test]
    async fn told_epochs() {
        let s = Store::open_in_memory().await.unwrap();
        assert_eq!(told(&s, "p").await.unwrap(), None);
        mark_told(&s, "p", 0).await.unwrap();
        mark_told(&s, "q", 0).await.unwrap();
        assert_eq!(told(&s, "p").await.unwrap(), Some(0));
        mark_told(&s, "p", 3).await.unwrap();
        assert_eq!(told(&s, "p").await.unwrap(), Some(3));
        mark_told(&s, "p", u32::MAX).await.unwrap();
        assert_eq!(told(&s, "p").await.unwrap(), Some(u32::MAX));
        forget_told(&s).await.unwrap();
        assert_eq!(told(&s, "p").await.unwrap(), None);
        assert_eq!(told(&s, "q").await.unwrap(), None);
    }

    #[tokio::test]
    async fn told_peers_and_a_carried_epoch() {
        let s = Store::open_in_memory().await.unwrap();
        assert!(told_peers(&s).await.unwrap().is_empty());
        mark_told(&s, "q", 0).await.unwrap();
        mark_told(&s, "p", 1).await.unwrap();
        assert_eq!(told_peers(&s).await.unwrap(), vec!["p".to_string(), "q".to_string()]);
        carry_told(&s, 2).await.unwrap();
        assert_eq!((told(&s, "p").await.unwrap(), told(&s, "q").await.unwrap()), (Some(2), Some(2)));
        assert_eq!(told(&s, "r").await.unwrap(), None, "nobody new is told by a carry");
    }

    #[tokio::test]
    async fn forget_peer_takes_everything_of_one_peer() {
        let s = Store::open_in_memory().await.unwrap();
        put_key(&s, "p", Some(KA), 1).await.unwrap();
        put_key(&s, "q", Some(KB), 1).await.unwrap();
        seen(&s, KA, 100, 180).await.unwrap();
        seen(&s, KB, 100, 180).await.unwrap();
        mark_told(&s, "p", 0).await.unwrap();
        mark_told(&s, "q", 0).await.unwrap();
        forget_peer(&s, "p").await.unwrap();
        assert_eq!(key_of(&s, "p").await.unwrap(), None);
        assert_eq!(told(&s, "p").await.unwrap(), None);
        assert!(seen(&s, KA, 100, 180).await.unwrap(), "its beat is gone");
        assert_eq!(snapshot(&s).await.unwrap(), vec![("q".into(), 100, 180)]);
        assert_eq!(told(&s, "q").await.unwrap(), Some(0));
        forget_peer(&s, "nobody").await.unwrap();
    }
}
