// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_bugs_outbox`: reports of a bug to the project that wait to be sent
//! again. Meaning (what is sent, when, for how long) lives in
//! `messenger-runtime` (`bug`); here are only the rows, by the queries of
//! every outbox to the project (`queued`).

use crate::queued::{self, Outbox};
use crate::Store;
use messenger_core::Result;

pub use crate::queued::{STATE_EXPIRED, STATE_QUEUED, STATE_REFUSED, STATE_SENT};

const OUTBOX: Outbox = Outbox {
    table: "msg_bugs_outbox",
    body: "body_json",
};

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct BugRow {
    pub id: String,
    /// The body as it is sent, byte for byte; empty once it is done.
    pub body_json: String,
    pub state: String,
    pub attempts: i64,
    pub next_retry_at: i64,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Keep a report of a bug that failed `attempts` times so far, to be tried
/// at `next_retry_at`.
pub async fn queue(
    store: &Store,
    id: &str,
    body_json: &str,
    now: i64,
    attempts: i64,
    next_retry_at: i64,
    last_error: Option<&str>,
) -> Result<()> {
    queued::queue(
        store,
        &OUTBOX,
        id,
        body_json,
        now,
        attempts,
        next_retry_at,
        last_error,
    )
    .await
}

/// The queued ones due by `now`, the oldest first.
pub async fn due(store: &Store, now: i64, limit: i64) -> Result<Vec<BugRow>> {
    queued::due(store, &OUTBOX, now, limit).await
}

pub async fn get(store: &Store, id: &str) -> Result<Option<BugRow>> {
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

/// Done (`sent`, `refused`, `expired`): never tried again, and the body goes.
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

    /// The outbox of bugs is a table of its own: what waits there is not a
    /// report of a person, and the other way round.
    #[tokio::test]
    async fn a_bug_waits_in_its_own_table_and_leaves_nothing_once_done() {
        let s = Store::open_in_memory().await.unwrap();
        queue(
            &s,
            "a",
            "{\"v\":1,\"text\":\"x\"}",
            100,
            1,
            160,
            Some("transport"),
        )
        .await
        .unwrap();
        crate::reports::queue(&s, "r", "{\"v\":1}", 100, 1, 160, None)
            .await
            .unwrap();
        assert!(due(&s, 159, 10).await.unwrap().is_empty(), "not due yet");
        let now = due(&s, 160, 10).await.unwrap();
        assert_eq!(
            now.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["a"]
        );
        assert_eq!(
            now[0].body_json, "{\"v\":1,\"text\":\"x\"}",
            "kept byte for byte"
        );
        assert_eq!(count_queued(&s).await.unwrap(), 1);
        assert_eq!(crate::reports::count_queued(&s).await.unwrap(), 1);

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
        assert_eq!((a.state.as_str(), a.body_json.as_str()), (STATE_SENT, ""));
        assert_eq!(count_queued(&s).await.unwrap(), 0);
        assert_eq!(purge_done(&s, 301).await.unwrap(), 1);
        assert!(get(&s, "a").await.unwrap().is_none());
        assert!(
            crate::reports::get(&s, "r").await.unwrap().is_some(),
            "the report stays"
        );
    }
}
