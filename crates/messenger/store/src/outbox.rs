// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_outbox`: outgoing requests persisted until a relay accepts them.
//! The stored `Outbound` already contains the signed event, so a retry
//! republishes the same event id.
//!
//! A row may have a deadline (`expires_at`): a message the user wrote is
//! tried until then and is `abandoned` after. A row without one is tried
//! until it leaves.

use crate::{storage, Store};
use messenger_core::{Outbound, Result};

pub const STATE_QUEUED: &str = "queued";
pub const STATE_PUBLISHING: &str = "publishing";
pub const STATE_PUBLISHED: &str = "published";
pub const STATE_FAILED: &str = "failed";
/// Given up: past its deadline, or refused by the relays. Never retried
/// unless the user asks.
pub const STATE_ABANDONED: &str = "abandoned";

const COLUMNS: &str =
    "local_id, outbound_json, state, attempts, next_retry_at, last_error, created_at, updated_at, expires_at, rejections";

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct OutboxRow {
    pub local_id: String,
    pub outbound_json: String,
    pub state: String,
    pub attempts: i64,
    pub next_retry_at: i64,
    pub last_error: Option<String>,
    /// When it was queued (or queued again by a retry).
    pub created_at: i64,
    pub updated_at: i64,
    /// Until when it is tried; `None`: until it leaves.
    pub expires_at: Option<i64>,
    /// How many times relays refused it outright.
    pub rejections: i64,
}

impl OutboxRow {
    pub fn outbound(&self) -> Result<Outbound> {
        Ok(serde_json::from_str(&self.outbound_json)?)
    }
}

/// Queues a request, due at once.
pub async fn enqueue(store: &Store, local_id: &str, out: &Outbound, now: i64, expires_at: Option<i64>) -> Result<()> {
    sqlx::query(
        "INSERT INTO msg_outbox (local_id, outbound_json, state, attempts, next_retry_at, last_error, created_at, updated_at, expires_at, rejections)
         VALUES (?, ?, ?, 0, ?, NULL, ?, ?, ?, 0)",
    )
    .bind(local_id)
    .bind(serde_json::to_string(out)?)
    .bind(STATE_QUEUED)
    .bind(now)
    .bind(now)
    .bind(now)
    .bind(expires_at)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// Rows that should be (re)tried now: queued or failed with a due retry
/// time, plus `publishing` rows older than `stale_after` (crashed mid-send).
pub async fn due(store: &Store, now: i64, stale_after: i64) -> Result<Vec<OutboxRow>> {
    sqlx::query_as::<_, OutboxRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM msg_outbox
         WHERE (state IN ('queued', 'failed') AND next_retry_at <= ?)
            OR (state = 'publishing' AND updated_at <= ?)
         ORDER BY created_at ASC LIMIT 100"
    )))
    .bind(now)
    .bind(now - stale_after)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

pub async fn mark_publishing(store: &Store, local_id: &str) -> Result<()> {
    set_state(store, local_id, STATE_PUBLISHING, None, None, true).await
}

pub async fn mark_published(store: &Store, local_id: &str) -> Result<()> {
    set_state(store, local_id, STATE_PUBLISHED, None, None, false).await
}

/// Not out this time; tried again at `next_retry_at`. `refused`: a relay
/// said no to it, which counts toward giving up; a network that is down
/// does not.
pub async fn mark_failed(store: &Store, local_id: &str, error: &str, next_retry_at: i64, refused: bool) -> Result<()> {
    set_state(store, local_id, STATE_FAILED, Some(error), Some(next_retry_at), false).await?;
    if refused {
        sqlx::query("UPDATE msg_outbox SET rejections = rejections + 1 WHERE local_id = ?")
            .bind(local_id)
            .execute(store.pool())
            .await
            .map_err(storage)?;
    }
    Ok(())
}

/// Given up on: not tried again unless the user asks.
pub async fn mark_abandoned(store: &Store, local_id: &str, error: &str) -> Result<()> {
    set_state(store, local_id, STATE_ABANDONED, Some(error), None, false).await
}

/// Manual retry: due now, counters reset, and a row with a deadline gets
/// `ttl` seconds from now; one without stays without.
pub async fn retry_now(store: &Store, local_id: &str, now: i64, ttl: i64) -> Result<()> {
    sqlx::query(
        "UPDATE msg_outbox SET state = 'queued', attempts = 0, rejections = 0, created_at = ?, next_retry_at = ?,
                expires_at = CASE WHEN expires_at IS NULL THEN NULL ELSE ? END, updated_at = ?
         WHERE local_id = ?",
    )
    .bind(now)
    .bind(now)
    .bind(now + ttl)
    .bind(now)
    .bind(local_id)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

pub async fn get(store: &Store, local_id: &str) -> Result<Option<OutboxRow>> {
    sqlx::query_as::<_, OutboxRow>(sqlx::AssertSqlSafe(format!("SELECT {COLUMNS} FROM msg_outbox WHERE local_id = ?")))
        .bind(local_id)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)
}

/// What is still on its way: neither out nor given up.
pub async fn count_pending(store: &Store) -> Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM msg_outbox WHERE state NOT IN ('published', 'abandoned')")
        .fetch_one(store.pool())
        .await
        .map_err(storage)
}

/// Drop published rows older than `older_than` seconds.
pub async fn prune_published(store: &Store, older_than: i64) -> Result<u64> {
    let res = sqlx::query("DELETE FROM msg_outbox WHERE state = 'published' AND updated_at < ?")
        .bind(crate::now() - older_than)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(res.rows_affected())
}

async fn set_state(
    store: &Store,
    local_id: &str,
    state: &str,
    error: Option<&str>,
    next_retry_at: Option<i64>,
    bump_attempts: bool,
) -> Result<()> {
    let now = crate::now();
    let sql = if bump_attempts {
        "UPDATE msg_outbox SET state = ?, last_error = ?, next_retry_at = COALESCE(?, next_retry_at), attempts = attempts + 1, updated_at = ? WHERE local_id = ?"
    } else {
        "UPDATE msg_outbox SET state = ?, last_error = ?, next_retry_at = COALESCE(?, next_retry_at), updated_at = ? WHERE local_id = ?"
    };
    sqlx::query(sql)
        .bind(state)
        .bind(error)
        .bind(next_retry_at)
        .bind(now)
        .bind(local_id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::SubId;

    #[tokio::test]
    async fn lifecycle() {
        let s = Store::open_in_memory().await.unwrap();
        let out = Outbound::Unsubscribe { id: SubId("x".into()) };
        let now = crate::now();
        enqueue(&s, "a", &out, now, Some(now + 3600)).await.unwrap();
        enqueue(&s, "b", &out, now + 3600, None).await.unwrap();

        let d = due(&s, now, 60).await.unwrap();
        assert_eq!(d.iter().map(|r| r.local_id.as_str()).collect::<Vec<_>>(), vec!["a"], "b is not due yet");
        assert!(matches!(d[0].outbound().unwrap(), Outbound::Unsubscribe { .. }));
        assert_eq!(d[0].expires_at, Some(now + 3600));
        assert_eq!(get(&s, "b").await.unwrap().unwrap().expires_at, None);

        mark_publishing(&s, "a").await.unwrap();
        assert_eq!(get(&s, "a").await.unwrap().unwrap().attempts, 1);
        assert!(due(&s, now, 60).await.unwrap().is_empty(), "publishing rows are not due while fresh");
        assert_eq!(due(&s, now + 120, 60).await.unwrap().len(), 1, "stale publishing rows come back");

        mark_failed(&s, "a", "no relay", now + 5, false).await.unwrap();
        assert!(due(&s, now, 60).await.unwrap().is_empty());
        assert_eq!(due(&s, now + 5, 60).await.unwrap().len(), 1);
        let a = get(&s, "a").await.unwrap().unwrap();
        assert_eq!(a.last_error.as_deref(), Some("no relay"));
        assert_eq!(a.rejections, 0, "a network that is down is not a refusal");
        mark_failed(&s, "a", "blocked: no", now + 5, true).await.unwrap();
        assert_eq!(get(&s, "a").await.unwrap().unwrap().rejections, 1);

        // Given up: not due, not pending.
        mark_abandoned(&s, "a", "expired: no relay").await.unwrap();
        assert!(due(&s, now + 10_000, 60).await.unwrap().iter().all(|r| r.local_id != "a"));
        assert_eq!(count_pending(&s).await.unwrap(), 1, "b alone");

        // A retry is a new start: due now, counters reset, a fresh deadline.
        let later = now + 7200;
        retry_now(&s, "a", later, 3600).await.unwrap();
        let a = get(&s, "a").await.unwrap().unwrap();
        assert_eq!((a.state.as_str(), a.attempts, a.rejections), (STATE_QUEUED, 0, 0));
        assert_eq!((a.created_at, a.next_retry_at, a.expires_at), (later, later, Some(later + 3600)));
        retry_now(&s, "b", later, 3600).await.unwrap();
        assert_eq!(get(&s, "b").await.unwrap().unwrap().expires_at, None, "no deadline stays none");

        mark_published(&s, "a").await.unwrap();
        assert_eq!(count_pending(&s).await.unwrap(), 1);
        assert_eq!(prune_published(&s, -1).await.unwrap(), 1);
        assert!(get(&s, "a").await.unwrap().is_none());
    }
}
