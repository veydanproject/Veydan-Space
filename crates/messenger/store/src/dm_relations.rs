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
    /// When I decided about the peer, my clock (migration 027); 0: never.
    pub decided_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

pub async fn get(store: &Store, peer: &str) -> Result<Option<RelationRow>> {
    sqlx::query_as::<_, RelationRow>(
        "SELECT peer_pubkey, my_contact, blocked, peer_signal, was_ever_mutual, last_signal_at, last_my_signal_at, request_floor, decided_at, created_at, updated_at
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
        "INSERT INTO msg_dm_relations (peer_pubkey, my_contact, blocked, peer_signal, was_ever_mutual, last_signal_at, last_my_signal_at, request_floor, decided_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(peer_pubkey) DO UPDATE SET
           my_contact = excluded.my_contact, blocked = excluded.blocked, peer_signal = excluded.peer_signal,
           was_ever_mutual = excluded.was_ever_mutual, last_signal_at = excluded.last_signal_at,
           last_my_signal_at = excluded.last_my_signal_at, request_floor = excluded.request_floor,
           decided_at = excluded.decided_at, updated_at = excluded.updated_at",
    )
    .bind(&r.peer_pubkey)
    .bind(&r.my_contact)
    .bind(r.blocked)
    .bind(&r.peer_signal)
    .bind(r.was_ever_mutual)
    .bind(r.last_signal_at)
    .bind(r.last_my_signal_at)
    .bind(r.request_floor)
    .bind(r.decided_at)
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

/// A relation in the state of a request to me, over the alias `r` of
/// `msg_dm_relations`: the peer approved, I did not (nor did the request
/// run out), and I do not block them. The one predicate for "request",
/// the same `screen_mode` computes (`messenger-dm` checks they agree):
/// the total unread, the queue and its count read it, and no flag of
/// `msg_chats` says it twice.
pub const REQUEST_SQL: &str = "r.blocked = 0 AND r.peer_signal = 'approved' AND r.my_contact NOT IN ('approved', 'expired')";

/// One chat of the queue of requests: its id, the peer, and its activity
/// (`ACTIVITY_SQL`), by which the queue is ordered, paged, aged and capped.
pub type RequestRow = (String, String, i64);

/// The DM chats in the state of a request, with the relation of their
/// peer: a request is a chat, so a relation without a chat (deleted here)
/// is not one.
const REQUEST_FROM: &str = "FROM msg_chats c JOIN msg_dm_relations r ON r.peer_pubkey = c.peer_pubkey WHERE c.kind = 'dm' AND ";

/// When a request was last active, over the alias `c` of `msg_chats`: the
/// time of its newest message, but no later than this device last wrote
/// the chat row. The message is dated by the peer, so one dated in the
/// future would neither run out nor ever be the oldest, and would push the
/// honest ones out; my own clock bounds it, and a backdated one only hurts
/// its sender.
const ACTIVITY_SQL: &str = "MIN(COALESCE(c.last_message_at, c.created_at), c.updated_at)";

/// The queue, newest activity first; `before` pages it by that activity
/// (the third of the row).
pub async fn request_rows(store: &Store, before: Option<i64>, limit: i64) -> Result<Vec<RequestRow>> {
    sqlx::query_as::<_, RequestRow>(sqlx::AssertSqlSafe(format!(
        "SELECT c.id, c.peer_pubkey, {ACTIVITY_SQL} {REQUEST_FROM} {REQUEST_SQL}
           AND (?1 IS NULL OR {ACTIVITY_SQL} < ?1)
         ORDER BY {ACTIVITY_SQL} DESC, c.id ASC LIMIT ?2"
    )))
    .bind(before)
    .bind(limit)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

pub async fn count_requests(store: &Store) -> Result<i64> {
    sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) {REQUEST_FROM} {REQUEST_SQL}")))
        .fetch_one(store.pool())
        .await
        .map_err(storage)
}

/// The `n` requests with the oldest activity, the oldest first: what runs
/// out first, and what the queue pushes out when it is full.
pub async fn oldest_requests(store: &Store, n: i64) -> Result<Vec<RequestRow>> {
    sqlx::query_as::<_, RequestRow>(sqlx::AssertSqlSafe(format!(
        "SELECT c.id, c.peer_pubkey, {ACTIVITY_SQL} {REQUEST_FROM} {REQUEST_SQL}
         ORDER BY {ACTIVITY_SQL} ASC, c.id ASC LIMIT ?"
    )))
    .bind(n)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

pub async fn blocked_peers(store: &Store) -> Result<Vec<String>> {
    sqlx::query_scalar::<_, String>("SELECT peer_pubkey FROM msg_dm_relations WHERE blocked = 1 ORDER BY updated_at DESC")
        .fetch_all(store.pool())
        .await
        .map_err(storage)
}

/// Every peer I decided about: blocked, `my_contact` other than none, or
/// a time of a decision (a contact removed, a block lifted: nothing else
/// is left of it). What the snapshot of my relations carries
/// (`messenger-dm::snapshot`); the rows of peers who only wrote to me are
/// not decisions.
pub async fn decided_rows(store: &Store) -> Result<Vec<RelationRow>> {
    sqlx::query_as::<_, RelationRow>(
        "SELECT peer_pubkey, my_contact, blocked, peer_signal, was_ever_mutual, last_signal_at, last_my_signal_at, request_floor, decided_at, created_at, updated_at
         FROM msg_dm_relations WHERE blocked = 1 OR my_contact != 'none' OR decided_at > 0 ORDER BY decided_at DESC, peer_pubkey ASC",
    )
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
            decided_at: 0,
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
        // Who only wrote to me is no decision of mine.
        r.peer_pubkey = "wrote".into();
        r.my_contact = "none".into();
        put(&s, &r).await.unwrap();
        r.peer_pubkey = "shut".into();
        r.blocked = true;
        r.decided_at = 9;
        put(&s, &r).await.unwrap();
        let decided: Vec<String> = decided_rows(&s).await.unwrap().into_iter().map(|r| r.peer_pubkey).collect();
        assert_eq!(decided, vec!["shut".to_string(), "p".into(), "q".into()], "the newest decision first");
        // A contact removed, a block lifted: nothing is left of them but
        // the time I decided, and that is a decision.
        r.peer_pubkey = "gone".into();
        r.blocked = false;
        r.decided_at = 4;
        put(&s, &r).await.unwrap();
        let decided: Vec<String> = decided_rows(&s).await.unwrap().into_iter().map(|r| r.peer_pubkey).collect();
        assert_eq!(decided, vec!["shut".to_string(), "gone".into(), "p".into(), "q".into()]);
    }

    #[tokio::test]
    async fn the_queue_of_requests_is_the_request_chats_newest_first() {
        let s = Store::open_in_memory().await.unwrap();
        let row = |peer: &str, mine: &str, blocked: bool, theirs: &str| RelationRow {
            peer_pubkey: peer.into(),
            my_contact: mine.into(),
            blocked,
            peer_signal: theirs.into(),
            was_ever_mutual: false,
            last_signal_at: 0,
            last_my_signal_at: 0,
            request_floor: 0,
            decided_at: 0,
            created_at: 0,
            updated_at: 0,
        };
        for (r, at) in [
            (row("new", "none", false, "approved"), 300),
            (row("old", "declined", false, "approved"), 100),
            (row("mid", "none", false, "approved"), 200),
            (row("mine", "approved", false, "approved"), 400),
            (row("blocked", "none", true, "approved"), 500),
            (row("expired", "expired", false, "approved"), 600),
            (row("silent", "none", false, "none"), 700),
        ] {
            put(&s, &r).await.unwrap();
            crate::chats::ensure_dm(&s, &r.peer_pubkey).await.unwrap();
            crate::chats::touch(&s, &format!("dm:{}", r.peer_pubkey), at, Some("hi"), true).await.unwrap();
        }
        // A request without its chat (deleted here) is not in the queue.
        put(&s, &row("no_chat", "none", false, "approved")).await.unwrap();
        let ids = |rows: Vec<RequestRow>| rows.into_iter().map(|(id, _, _)| id).collect::<Vec<_>>();
        assert_eq!(ids(request_rows(&s, None, 10).await.unwrap()), vec!["dm:new", "dm:mid", "dm:old"]);
        assert_eq!(ids(request_rows(&s, Some(300), 1).await.unwrap()), vec!["dm:mid"], "paged by the time before");
        assert_eq!(count_requests(&s).await.unwrap(), 3);
        assert_eq!(oldest_requests(&s, 2).await.unwrap(), vec![("dm:old".into(), "old".into(), 100), ("dm:mid".into(), "mid".into(), 200)]);
        assert_eq!(crate::chats::total_unread(&s).await.unwrap(), 4, "the requests are not counted as waiting");

        // A request whose text is dated in the future (the peer's clock is
        // theirs to set) is active when this device wrote its chat, not
        // when the peer says: it ages like the rest, and the honest ones
        // are never the oldest beside it.
        let before = crate::now();
        put(&s, &row("future", "none", false, "approved")).await.unwrap();
        crate::chats::ensure_dm(&s, "future").await.unwrap();
        crate::chats::touch(&s, "dm:future", 4_102_444_800, Some("from 2100"), true).await.unwrap();
        let rows = request_rows(&s, None, 10).await.unwrap();
        let activity = rows.iter().find(|(id, _, _)| id == "dm:future").unwrap().2;
        assert!(activity >= before && activity <= crate::now(), "{activity}");
        assert_eq!(ids(rows), vec!["dm:future", "dm:new", "dm:mid", "dm:old"], "newest by my clock, not by theirs");
        assert_eq!(ids(oldest_requests(&s, 3).await.unwrap()), vec!["dm:old", "dm:mid", "dm:new"]);
        assert_eq!(ids(request_rows(&s, Some(activity), 10).await.unwrap()), vec!["dm:new", "dm:mid", "dm:old"], "paged by the activity");
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
            decided_at: 0,
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
