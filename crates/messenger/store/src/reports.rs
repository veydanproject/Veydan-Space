// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_reports_outbox`: reports to the moderators of the project that wait
//! to be sent again. Meaning (what is sent, when, for how long) lives in
//! `messenger-runtime` (`report`); here are only the rows.

use crate::{storage, Store};
use messenger_core::Result;

pub const STATE_QUEUED: &str = "queued";
pub const STATE_SENT: &str = "sent";
pub const STATE_REFUSED: &str = "refused";
pub const STATE_EXPIRED: &str = "expired";

const COLUMNS: &str =
    "id, report_json, state, attempts, next_retry_at, last_error, created_at, updated_at";

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct ReportRow {
    pub id: String,
    /// The report as it is sent; empty once it is done.
    pub report_json: String,
    pub state: String,
    pub attempts: i64,
    pub next_retry_at: i64,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Keep a report that failed `attempts` times so far, to be tried at
/// `next_retry_at`.
pub async fn queue(
    store: &Store,
    id: &str,
    report_json: &str,
    now: i64,
    attempts: i64,
    next_retry_at: i64,
    last_error: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO msg_reports_outbox (id, report_json, state, attempts, next_retry_at, last_error, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(report_json)
    .bind(STATE_QUEUED)
    .bind(attempts)
    .bind(next_retry_at)
    .bind(last_error)
    .bind(now)
    .bind(now)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// The queued reports due by `now`, the oldest first.
pub async fn due(store: &Store, now: i64, limit: i64) -> Result<Vec<ReportRow>> {
    sqlx::query_as::<_, ReportRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM msg_reports_outbox WHERE state = 'queued' AND next_retry_at <= ?
         ORDER BY created_at ASC, id ASC LIMIT ?"
    )))
    .bind(now)
    .bind(limit)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

pub async fn get(store: &Store, id: &str) -> Result<Option<ReportRow>> {
    sqlx::query_as::<_, ReportRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM msg_reports_outbox WHERE id = ?"
    )))
    .bind(id)
    .fetch_optional(store.pool())
    .await
    .map_err(storage)
}

/// Another try failed: the count goes up, the next one is at `next_retry_at`.
pub async fn retry_later(
    store: &Store,
    id: &str,
    attempts: i64,
    next_retry_at: i64,
    error: &str,
    now: i64,
) -> Result<()> {
    sqlx::query("UPDATE msg_reports_outbox SET attempts = ?, next_retry_at = ?, last_error = ?, updated_at = ? WHERE id = ?")
        .bind(attempts)
        .bind(next_retry_at)
        .bind(error)
        .bind(now)
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// The report is done (`sent`, `refused`, `expired`): it is never tried
/// again, and what it carried goes.
pub async fn finish(
    store: &Store,
    id: &str,
    state: &str,
    error: Option<&str>,
    now: i64,
) -> Result<()> {
    sqlx::query("UPDATE msg_reports_outbox SET state = ?, report_json = '', last_error = COALESCE(?, last_error), updated_at = ? WHERE id = ?")
        .bind(state)
        .bind(error)
        .bind(now)
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// Forget the rows done before `before`. Returns how many went.
pub async fn purge_done(store: &Store, before: i64) -> Result<u64> {
    let res =
        sqlx::query("DELETE FROM msg_reports_outbox WHERE state <> 'queued' AND updated_at < ?")
            .bind(before)
            .execute(store.pool())
            .await
            .map_err(storage)?;
    Ok(res.rows_affected())
}

/// How many wait to be sent.
pub async fn count_queued(store: &Store) -> Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM msg_reports_outbox WHERE state = 'queued'")
        .fetch_one(store.pool())
        .await
        .map_err(storage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_report_waits_until_due_and_leaves_nothing_once_done() {
        let s = Store::open_in_memory().await.unwrap();
        queue(&s, "a", "{\"v\":1}", 100, 1, 160, Some("transport"))
            .await
            .unwrap();
        queue(&s, "b", "{\"v\":1}", 110, 1, 500, None)
            .await
            .unwrap();
        assert!(due(&s, 159, 10).await.unwrap().is_empty(), "not due yet");
        let now = due(&s, 160, 10).await.unwrap();
        assert_eq!(
            now.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["a"]
        );
        assert_eq!(count_queued(&s).await.unwrap(), 2);

        retry_later(&s, "a", 2, 300, "status 503", 160)
            .await
            .unwrap();
        let a = get(&s, "a").await.unwrap().unwrap();
        assert_eq!(
            (a.attempts, a.next_retry_at, a.last_error.as_deref()),
            (2, 300, Some("status 503"))
        );

        finish(&s, "a", STATE_SENT, None, 300).await.unwrap();
        let a = get(&s, "a").await.unwrap().unwrap();
        assert_eq!(
            (a.state.as_str(), a.report_json.as_str()),
            (STATE_SENT, ""),
            "the evidence goes with the try"
        );
        assert_eq!(
            a.last_error.as_deref(),
            Some("status 503"),
            "the last failure is remembered"
        );
        assert!(
            due(&s, i64::MAX, 10)
                .await
                .unwrap()
                .iter()
                .all(|r| r.id != "a"),
            "done: never tried again"
        );

        finish(&s, "b", STATE_REFUSED, Some("too_many_messages"), 600)
            .await
            .unwrap();
        assert_eq!(count_queued(&s).await.unwrap(), 0);
        assert_eq!(
            purge_done(&s, 301).await.unwrap(),
            1,
            "the one done before the bound"
        );
        assert!(get(&s, "a").await.unwrap().is_none());
        assert_eq!(
            get(&s, "b").await.unwrap().unwrap().last_error.as_deref(),
            Some("too_many_messages")
        );
    }
}
