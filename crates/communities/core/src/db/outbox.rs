// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `cm_outbox` (`025_outbox`): what a command or a keeper left to send,
//! one row an event, sent by the worker (`outbox.rs`) off the lock of the
//! channel and again after a failure, in the order the wire needs
//! (internal/communities-wire.md §4, §14.5, §15):
//!
//! - lane `channel`: the journal operations (1391) and the sealed content
//!   (1390) of a channel, strictly in `seq` order; one the relays refuse
//!   holds those after it (content that relies on a right an operation
//!   grants is dropped at a reader who hears it first).
//! - lane `replaceable`: the card of the owner (30390); a newer row of a
//!   kind replaces the older, they go out on their own.
//! - lane `keeper`: the epochs and snapshots of a keeper kept here, each
//!   row on its own (`LANE_KEEPER`).
//! - lane `inbox`: a note by NIP-59 to somebody, wrapped anew with the
//!   keys of `signer` at every send; `after_seq` holds it until every
//!   channel row of its channel up to that `seq` is gone: the keeper's
//!   `channel.issue` of a scope follows the operation that made it.
//!
//! A row sent is deleted; `state` is a label for the page (`queued`, or
//! `failed` after many attempts, still retried).

use super::{storage, Db};
use messenger_core::Result;
use std::collections::HashSet;

pub const LANE_CHANNEL: &str = "channel";
pub const LANE_REPLACEABLE: &str = "replaceable";
pub const LANE_INBOX: &str = "inbox";
/// The events of the keeper of a channel (an epoch 1392, a snapshot 1393):
/// each row on its own, nothing replaced and nothing held behind another
/// (an epoch no relay takes holds back no snapshot, and no post of the
/// owner). The order the wire needs, the `keeper` operation before the
/// first epoch of its series (wire §15), is the cycle's: it queues nothing
/// while a journal operation of the channel waits (`tick_one`).
pub const LANE_KEEPER: &str = "keeper";

/// Whose keys wrap a note of the inbox lane: the user's, or the keeper's
/// of the channel (read from the secret store by the channel's id).
pub const SIGNER_ME: &str = "me";
pub const SIGNER_KEEPER: &str = "keeper";

pub const STATE_QUEUED: &str = "queued";
pub const STATE_FAILED: &str = "failed";

/// One row of the outbox.
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct OutboxRow {
    pub seq: i64,
    pub community_id: String,
    pub lane: String,
    pub kind: Option<i64>,
    pub event_id: String,
    pub ref_id: Option<String>,
    pub recipient: Option<String>,
    pub signer: String,
    pub hint_json: Option<String>,
    pub payload: String,
    pub after_seq: i64,
    pub state: String,
    pub attempts: i64,
    pub next_at: i64,
    pub error: Option<String>,
    pub created_at: i64,
}

/// A note of the inbox lane as it is queued.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewNote {
    /// Hex pubkey.
    pub recipient: String,
    /// `sha256(recipient:content)`: the same note is queued once.
    pub out_id: String,
    /// The note envelope (`wire::note_envelope`), wrapped at send time.
    pub content: String,
    /// The relays hinted to the pool, JSON list.
    pub hint_json: String,
    pub signer: String,
    /// The channel rows up to this `seq` go first.
    pub after_seq: i64,
    /// What names the note while it waits (`has_ref`): a join of mine
    /// is `join:<channel>`, so that it is not queued twice.
    pub ref_id: Option<String>,
}

/// What a channel has in the outbox, for its page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub queued: i64,
    pub failed: i64,
    /// What the relays said of the oldest row that failed.
    pub oldest_error: Option<String>,
}

const COLS: &str = "seq, community_id, lane, kind, event_id, ref_id, recipient, signer, hint_json, payload, after_seq, state, attempts, next_at, error, created_at";

/// Queues a signed event (`json`); its `seq`, or that of the same event
/// queued already. A replaceable row of the kind takes the older one's
/// place.
#[allow(clippy::too_many_arguments)]
pub async fn enqueue_event(db: &Db, community_id: &str, lane: &str, kind: u16, event_id: &str, ref_id: Option<&str>, json: &str, now: i64) -> Result<i64> {
    let mut tx = db.write_tx().await?;
    if lane == LANE_REPLACEABLE {
        sqlx::query("DELETE FROM cm_outbox WHERE community_id = ? AND lane = ? AND kind = ?")
            .bind(community_id)
            .bind(lane)
            .bind(i64::from(kind))
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
    }
    sqlx::query("INSERT OR IGNORE INTO cm_outbox (community_id, lane, kind, event_id, ref_id, signer, payload, next_at, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(community_id)
        .bind(lane)
        .bind(i64::from(kind))
        .bind(event_id)
        .bind(ref_id)
        .bind(SIGNER_ME)
        .bind(json)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    let seq: i64 = sqlx::query_scalar("SELECT seq FROM cm_outbox WHERE community_id = ? AND lane = ? AND event_id = ?")
        .bind(community_id)
        .bind(lane)
        .bind(event_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(storage)?;
    tx.commit().await.map_err(storage)?;
    Ok(seq)
}

/// Queues a note to somebody; its `seq`, or that of the same note queued
/// already (sent again then, with its attempts).
pub async fn enqueue_note(db: &Db, community_id: &str, note: &NewNote, now: i64) -> Result<i64> {
    let mut tx = db.write_tx().await?;
    sqlx::query(
        "INSERT OR IGNORE INTO cm_outbox (community_id, lane, kind, event_id, ref_id, recipient, signer, hint_json, payload, after_seq, next_at, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(community_id)
    .bind(LANE_INBOX)
    .bind(i64::from(crate::notes::KIND_WRAP))
    .bind(&note.out_id)
    .bind(&note.ref_id)
    .bind(&note.recipient)
    .bind(&note.signer)
    .bind(&note.hint_json)
    .bind(&note.content)
    .bind(note.after_seq)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    let seq: i64 = sqlx::query_scalar("SELECT seq FROM cm_outbox WHERE community_id = ? AND lane = ? AND event_id = ?")
        .bind(community_id)
        .bind(LANE_INBOX)
        .bind(&note.out_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(storage)?;
    tx.commit().await.map_err(storage)?;
    Ok(seq)
}

/// The newest `seq` of the channel lane of a channel (0: none waits): what
/// a note queued now must follow.
pub async fn last_channel_seq(db: &Db, community_id: &str) -> Result<i64> {
    sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) FROM cm_outbox WHERE community_id = ? AND lane = ?")
        .bind(community_id)
        .bind(LANE_CHANNEL)
        .fetch_one(db.pool())
        .await
        .map_err(storage)
}

/// The oldest row of the channel lane of a channel: the one that goes next.
pub async fn channel_head(db: &Db, community_id: &str) -> Result<Option<i64>> {
    sqlx::query_scalar("SELECT MIN(seq) FROM cm_outbox WHERE community_id = ? AND lane = ?")
        .bind(community_id)
        .bind(LANE_CHANNEL)
        .fetch_one(db.pool())
        .await
        .map_err(storage)
}

/// How many rows of the channel lane of a channel wait up to `seq`.
pub async fn channel_rows_up_to(db: &Db, community_id: &str, seq: i64) -> Result<i64> {
    sqlx::query_scalar("SELECT count(*) FROM cm_outbox WHERE community_id = ? AND lane = ? AND seq <= ?")
        .bind(community_id)
        .bind(LANE_CHANNEL)
        .bind(seq)
        .fetch_one(db.pool())
        .await
        .map_err(storage)
}

/// How many rows of a kind in a lane of a channel wait.
pub async fn count_kind(db: &Db, community_id: &str, lane: &str, kind: u16) -> Result<i64> {
    sqlx::query_scalar("SELECT count(*) FROM cm_outbox WHERE community_id = ? AND lane = ? AND kind = ?")
        .bind(community_id)
        .bind(lane)
        .bind(i64::from(kind))
        .fetch_one(db.pool())
        .await
        .map_err(storage)
}

/// How many rows of a lane of a channel wait.
pub async fn count(db: &Db, community_id: &str, lane: &str) -> Result<i64> {
    sqlx::query_scalar("SELECT count(*) FROM cm_outbox WHERE community_id = ? AND lane = ?")
        .bind(community_id)
        .bind(lane)
        .fetch_one(db.pool())
        .await
        .map_err(storage)
}

/// The rows whose time came, by channel, lane and `seq`.
pub async fn due(db: &Db, now: i64) -> Result<Vec<OutboxRow>> {
    sqlx::query_as::<_, OutboxRow>(sqlx::AssertSqlSafe(format!("SELECT {COLS} FROM cm_outbox WHERE next_at <= ? ORDER BY community_id, lane, seq")))
        .bind(now)
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

/// Every row, by channel, lane and `seq` (the outbox is small: what
/// waits for the relays).
pub async fn all(db: &Db) -> Result<Vec<OutboxRow>> {
    sqlx::query_as::<_, OutboxRow>(sqlx::AssertSqlSafe(format!("SELECT {COLS} FROM cm_outbox ORDER BY community_id, lane, seq")))
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

/// A row went out.
pub async fn sent(db: &Db, seq: i64) -> Result<()> {
    sqlx::query("DELETE FROM cm_outbox WHERE seq = ?").bind(seq).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}

/// A queued event no longer wanted (a snapshot sealed under a key a newer
/// epoch replaced): gone, unless it went already. Whether it was there.
pub async fn withdraw(db: &Db, community_id: &str, lane: &str, event_id: &str) -> Result<bool> {
    let res = sqlx::query("DELETE FROM cm_outbox WHERE community_id = ? AND lane = ? AND event_id = ?")
        .bind(community_id)
        .bind(lane)
        .bind(event_id)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(res.rows_affected() > 0)
}

/// Whether a row with this `ref_id` waits in a channel's outbox.
pub async fn has_ref(db: &Db, community_id: &str, ref_id: &str) -> Result<bool> {
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM cm_outbox WHERE community_id = ? AND ref_id = ?")
        .bind(community_id)
        .bind(ref_id)
        .fetch_one(db.pool())
        .await
        .map_err(storage)?;
    Ok(n > 0)
}

/// A row did not go out: tried again at `next_at`.
pub async fn retry_later(db: &Db, seq: i64, attempts: i64, next_at: i64, state: &str, error: Option<&str>) -> Result<()> {
    sqlx::query("UPDATE cm_outbox SET attempts = ?, next_at = ?, state = ?, error = ? WHERE seq = ?")
        .bind(attempts)
        .bind(next_at)
        .bind(state)
        .bind(error)
        .bind(seq)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// Every row of a channel tried again now (the user asked); how many.
pub async fn retry_now(db: &Db, community_id: &str, now: i64) -> Result<u64> {
    let res = sqlx::query("UPDATE cm_outbox SET next_at = ?, state = ? WHERE community_id = ?")
        .bind(now)
        .bind(STATE_QUEUED)
        .bind(community_id)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(res.rows_affected())
}

/// Every row still to go and waiting for a later try goes at `now` (a
/// relay connected: what waited for one goes, its count of tries as it
/// was); how many.
pub async fn due_now(db: &Db, now: i64) -> Result<u64> {
    let res = sqlx::query("UPDATE cm_outbox SET next_at = ? WHERE state = ? AND next_at > ?")
        .bind(now)
        .bind(STATE_QUEUED)
        .bind(now)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(res.rows_affected())
}

/// The notes queued before `before` are given up (a note lives a week,
/// wire §7); how many.
pub async fn drop_notes_before(db: &Db, before: i64) -> Result<u64> {
    let res = sqlx::query("DELETE FROM cm_outbox WHERE lane = ? AND created_at < ?").bind(LANE_INBOX).bind(before).execute(db.pool()).await.map_err(storage)?;
    Ok(res.rows_affected())
}

/// Everything of a channel (it is forgotten).
pub async fn delete_of(db: &Db, community_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM cm_outbox WHERE community_id = ?").bind(community_id).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}

pub async fn summary(db: &Db, community_id: &str) -> Result<Summary> {
    let (queued, failed): (i64, i64) = sqlx::query_as("SELECT COALESCE(SUM(state = ?), 0), COALESCE(SUM(state = ?), 0) FROM cm_outbox WHERE community_id = ?")
        .bind(STATE_QUEUED)
        .bind(STATE_FAILED)
        .bind(community_id)
        .fetch_one(db.pool())
        .await
        .map_err(storage)?;
    let oldest_error: Option<String> = sqlx::query_scalar("SELECT error FROM cm_outbox WHERE community_id = ? AND error IS NOT NULL ORDER BY seq LIMIT 1")
        .bind(community_id)
        .fetch_optional(db.pool())
        .await
        .map_err(storage)?;
    Ok(Summary { queued, failed, oldest_error })
}

/// What the page shows as pending in a channel: the `ref_id`s waiting.
pub async fn pending_refs(db: &Db, community_id: &str) -> Result<HashSet<String>> {
    let refs: Vec<String> = sqlx::query_scalar("SELECT ref_id FROM cm_outbox WHERE community_id = ? AND ref_id IS NOT NULL")
        .bind(community_id)
        .fetch_all(db.pool())
        .await
        .map_err(storage)?;
    Ok(refs.into_iter().collect())
}

/// The state of a queued event (none: sent, or never queued).
pub async fn state_of(db: &Db, community_id: &str, lane: &str, event_id: &str) -> Result<Option<String>> {
    sqlx::query_scalar("SELECT state FROM cm_outbox WHERE community_id = ? AND lane = ? AND event_id = ?")
        .bind(community_id)
        .bind(lane)
        .bind(event_id)
        .fetch_optional(db.pool())
        .await
        .map_err(storage)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CID: &str = crate::state::tests::CHANNEL;

    fn note(to: &str, content: &str, after_seq: i64) -> NewNote {
        NewNote { recipient: to.into(), out_id: format!("{to}:{content}"), content: content.into(), hint_json: "[]".into(), signer: SIGNER_KEEPER.into(), after_seq, ref_id: None }
    }

    /// The lanes keep their order: channel rows by `seq`, the card once
    /// (the newer in the older's place), a note behind the channel rows up
    /// to its `after_seq`; the same event queued twice is one row.
    #[tokio::test]
    async fn the_rows_keep_the_order_of_the_wire() {
        let db = Db::open_in_memory().await.unwrap();
        let op = enqueue_event(&db, CID, LANE_CHANNEL, 1391, "e1", Some("op1"), "{}", 10).await.unwrap();
        let post = enqueue_event(&db, CID, LANE_CHANNEL, 1390, "e2", Some("m1"), "{}", 11).await.unwrap();
        assert!(post > op);
        assert_eq!(enqueue_event(&db, CID, LANE_CHANNEL, 1390, "e2", Some("m1"), "{}", 12).await.unwrap(), post, "queued once");
        assert_eq!(last_channel_seq(&db, CID).await.unwrap(), post);
        assert_eq!(channel_head(&db, CID).await.unwrap(), Some(op));
        enqueue_event(&db, CID, LANE_REPLACEABLE, 30390, "c1", None, "{}", 12).await.unwrap();
        let card = enqueue_event(&db, CID, LANE_REPLACEABLE, 30390, "c2", None, "{}", 13).await.unwrap();
        assert!(state_of(&db, CID, LANE_REPLACEABLE, "c1").await.unwrap().is_none(), "the older card is gone");
        let issue = enqueue_note(&db, CID, &note("r", "issue", op), 14).await.unwrap();
        assert_eq!(enqueue_note(&db, CID, &note("r", "issue", op), 15).await.unwrap(), issue);
        let due = due(&db, 20).await.unwrap();
        assert_eq!(due.iter().map(|r| r.seq).collect::<Vec<_>>(), vec![op, post, issue, card], "channel, then inbox, then replaceable");
        assert_eq!(channel_rows_up_to(&db, CID, op).await.unwrap(), 1, "the note waits for the operation");
        sent(&db, op).await.unwrap();
        assert_eq!(channel_rows_up_to(&db, CID, op).await.unwrap(), 0, "and goes once it is out");
        assert_eq!(channel_head(&db, CID).await.unwrap(), Some(post));
        assert_eq!(pending_refs(&db, CID).await.unwrap(), HashSet::from(["m1".to_string()]));
        assert_eq!(count(&db, CID, LANE_CHANNEL).await.unwrap(), 1);
        // A note named by a `ref_id` is known to wait; an event withdrawn is gone, once.
        let mut join = note("k", "join", 0);
        join.ref_id = Some("join:c".into());
        enqueue_note(&db, CID, &join, 16).await.unwrap();
        assert!(has_ref(&db, CID, "join:c").await.unwrap());
        assert!(!has_ref(&db, CID, "join:d").await.unwrap());
        assert!(withdraw(&db, CID, LANE_CHANNEL, "e2").await.unwrap());
        assert!(!withdraw(&db, CID, LANE_CHANNEL, "e2").await.unwrap());
        assert_eq!(count(&db, CID, LANE_CHANNEL).await.unwrap(), 0);
        delete_of(&db, CID).await.unwrap();
        assert!(all(&db).await.unwrap().is_empty());
    }

    /// A failure is counted and waits for its time; many of them label the
    /// row failed for the page, with the oldest reason; the user's retry
    /// makes every row due now; old notes are given up.
    #[tokio::test]
    async fn a_failure_waits_and_the_page_sees_it() {
        let db = Db::open_in_memory().await.unwrap();
        let a = enqueue_event(&db, CID, LANE_CHANNEL, 1390, "a", Some("a"), "{}", 10).await.unwrap();
        let b = enqueue_event(&db, CID, LANE_CHANNEL, 1390, "b", Some("b"), "{}", 10).await.unwrap();
        retry_later(&db, a, 1, 25, STATE_QUEUED, Some("refused")).await.unwrap();
        assert_eq!(due(&db, 20).await.unwrap().iter().map(|r| r.seq).collect::<Vec<_>>(), vec![b]);
        assert_eq!(summary(&db, CID).await.unwrap(), Summary { queued: 2, failed: 0, oldest_error: Some("refused".into()) });
        retry_later(&db, a, 6, 600, STATE_FAILED, Some("still refused")).await.unwrap();
        retry_later(&db, b, 1, 25, STATE_QUEUED, Some("later")).await.unwrap();
        assert_eq!(summary(&db, CID).await.unwrap(), Summary { queued: 1, failed: 1, oldest_error: Some("still refused".into()) });
        assert_eq!(retry_now(&db, CID, 30).await.unwrap(), 2);
        assert_eq!(due(&db, 30).await.unwrap().len(), 2);
        assert_eq!(state_of(&db, CID, LANE_CHANNEL, "a").await.unwrap().as_deref(), Some(STATE_QUEUED));
        enqueue_note(&db, CID, &note("r", "old", 0), 1).await.unwrap();
        enqueue_note(&db, CID, &note("r", "new", 0), 100).await.unwrap();
        assert_eq!(drop_notes_before(&db, 50).await.unwrap(), 1);
        assert_eq!(all(&db).await.unwrap().len(), 3);
    }

    /// A relay connected: what waits for a later try goes now, its tries
    /// counted as they were; a row given up stays so (the user's retry).
    #[tokio::test]
    async fn a_relay_that_came_makes_the_waiting_rows_due() {
        let db = Db::open_in_memory().await.unwrap();
        let a = enqueue_event(&db, CID, LANE_CHANNEL, 1390, "a", Some("a"), "{}", 10).await.unwrap();
        let b = enqueue_event(&db, CID, LANE_CHANNEL, 1390, "b", Some("b"), "{}", 10).await.unwrap();
        let c = enqueue_event(&db, CID, LANE_CHANNEL, 1390, "c", Some("c"), "{}", 10).await.unwrap();
        retry_later(&db, a, 2, 70, STATE_QUEUED, Some("transport error: no relay connected")).await.unwrap();
        retry_later(&db, b, 6, 600, STATE_FAILED, Some("refused")).await.unwrap();
        assert!(due(&db, 20).await.unwrap().iter().all(|r| r.seq == c));
        assert_eq!(due_now(&db, 20).await.unwrap(), 1, "the waiting one, not the due one nor the failed one");
        let rows = due(&db, 20).await.unwrap();
        assert_eq!(rows.iter().map(|r| r.seq).collect::<Vec<_>>(), vec![a, c]);
        assert_eq!(rows[0].attempts, 2);
        assert_eq!(state_of(&db, CID, LANE_CHANNEL, "b").await.unwrap().as_deref(), Some(STATE_FAILED));
    }
}
