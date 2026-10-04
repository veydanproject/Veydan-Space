// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_outbox`: outgoing requests persisted until a relay accepts them.
//! The stored `Outbound` already contains the signed event, so a retry
//! republishes the same event id.

use crate::{storage, Store};
use messenger_core::{Outbound, Result};

pub const STATE_QUEUED: &str = "queued";
pub const STATE_PUBLISHING: &str = "publishing";
pub const STATE_PUBLISHED: &str = "published";
pub const STATE_FAILED: &str = "failed";

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct OutboxRow {
    pub local_id: String,
    pub outbound_json: String,
    pub state: String,
    pub attempts: i64,
    pub next_retry_at: i64,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl OutboxRow {
    pub fn outbound(&self) -> Result<Outbound> {
        Ok(serde_json::from_str(&self.outbound_json)?)
    }
}

pub async fn enqueue(store: &Store, local_id: &str, out: &Outbound, next_retry_at: i64) -> Result<()> {
    let now = crate::now();
    sqlx::query(
        "INSERT INTO msg_outbox (local_id, outbound_json, state, attempts, next_retry_at, last_error, created_at, updated_at)
         VALUES (?, ?, ?, 0, ?, NULL, ?, ?)",
    )
    .bind(local_id)
    .bind(serde_json::to_string(out)?)
    .bind(STATE_QUEUED)
    .bind(next_retry_at)
    .bind(now)
    .bind(now)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// Rows that should be (re)tried now: queued or failed with a due retry
/// time, plus `publishing` rows older than `stale_after` (crashed mid-send).
pub async fn due(store: &Store, now: i64, stale_after: i64) -> Result<Vec<OutboxRow>> {
    sqlx::query_as::<_, OutboxRow>(
        "SELECT local_id, outbound_json, state, attempts, next_retry_at, last_error, created_at, updated_at
         FROM msg_outbox
         WHERE (state IN ('queued', 'failed') AND next_retry_at <= ?)
            OR (state = 'publishing' AND updated_at <= ?)
         ORDER BY created_at ASC LIMIT 100",
    )
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

pub async fn mark_failed(store: &Store, local_id: &str, error: &str, next_retry_at: i64) -> Result<()> {
    set_state(store, local_id, STATE_FAILED, Some(error), Some(next_retry_at), false).await
}

/// Manual retry: reset backoff and make it due now.
pub async fn retry_now(store: &Store, local_id: &str) -> Result<()> {
    sqlx::query("UPDATE msg_outbox SET state = 'queued', attempts = 0, next_retry_at = ?, updated_at = ? WHERE local_id = ?")
        .bind(crate::now())
        .bind(crate::now())
        .bind(local_id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn get(store: &Store, local_id: &str) -> Result<Option<OutboxRow>> {
    sqlx::query_as::<_, OutboxRow>(
        "SELECT local_id, outbound_json, state, attempts, next_retry_at, last_error, created_at, updated_at
         FROM msg_outbox WHERE local_id = ?",
    )
    .bind(local_id)
    .fetch_optional(store.pool())
    .await
    .map_err(storage)
}

pub async fn count_pending(store: &Store) -> Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM msg_outbox WHERE state != 'published'")
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
        enqueue(&s, "a", &out, now).await.unwrap();
        enqueue(&s, "b", &out, now + 3600).await.unwrap();

        let d = due(&s, now, 60).await.unwrap();
        assert_eq!(d.iter().map(|r| r.local_id.as_str()).collect::<Vec<_>>(), vec!["a"], "b is not due yet");
        assert!(matches!(d[0].outbound().unwrap(), Outbound::Unsubscribe { .. }));

        mark_publishing(&s, "a").await.unwrap();
        assert_eq!(get(&s, "a").await.unwrap().unwrap().attempts, 1);
        assert!(due(&s, now, 60).await.unwrap().is_empty(), "publishing rows are not due while fresh");
        assert_eq!(due(&s, now + 120, 60).await.unwrap().len(), 1, "stale publishing rows come back");

        mark_failed(&s, "a", "no relay", now + 5).await.unwrap();
        assert!(due(&s, now, 60).await.unwrap().is_empty());
        assert_eq!(due(&s, now + 5, 60).await.unwrap().len(), 1);
        assert_eq!(get(&s, "a").await.unwrap().unwrap().last_error.as_deref(), Some("no relay"));

        retry_now(&s, "a").await.unwrap();
        let a = get(&s, "a").await.unwrap().unwrap();
        assert_eq!(a.state, STATE_QUEUED);
        assert_eq!(a.attempts, 0);

        mark_published(&s, "a").await.unwrap();
        assert_eq!(count_pending(&s).await.unwrap(), 1);
        assert_eq!(prune_published(&s, -1).await.unwrap(), 1);
        assert!(get(&s, "a").await.unwrap().is_none());
    }
}
