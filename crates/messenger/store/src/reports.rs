// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_reports_outbox`: reports to the moderators of the project that wait
//! to be sent again. Meaning (what is sent, when, for how long) lives in
//! `messenger-runtime` (`report`); here are only the rows.

use crate::queued::{self, Outbox};
use crate::Store;
use messenger_core::Result;

pub use crate::queued::{STATE_EXPIRED, STATE_QUEUED, STATE_REFUSED, STATE_SENT};

/// The queries are those of every outbox to the project (`queued`).
const OUTBOX: Outbox = Outbox {
    table: "msg_reports_outbox",
    body: "report_json",
};

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
    queued::queue(store, &OUTBOX, id, report_json, now, attempts, next_retry_at, last_error).await
}

/// The queued reports due by `now`, the oldest first.
pub async fn due(store: &Store, now: i64, limit: i64) -> Result<Vec<ReportRow>> {
    queued::due(store, &OUTBOX, now, limit).await
}

pub async fn get(store: &Store, id: &str) -> Result<Option<ReportRow>> {
    queued::get(store, &OUTBOX, id).await
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
    queued::retry_later(store, &OUTBOX, id, attempts, next_retry_at, error, now).await
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
    queued::finish(store, &OUTBOX, id, state, error, now).await
}

/// Forget the rows done before `before`. Returns how many went.
pub async fn purge_done(store: &Store, before: i64) -> Result<u64> {
    queued::purge_done(store, &OUTBOX, before).await
}

/// How many wait to be sent.
pub async fn count_queued(store: &Store) -> Result<i64> {
    queued::count_queued(store, &OUTBOX).await
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
