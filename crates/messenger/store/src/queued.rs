// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The rows of an outbox to the project: something signed by the identity
//! that did not reach the server at once, kept as it is sent and tried
//! again. The reports of a person (`msg_reports_outbox`, [`crate::reports`])
//! and the reports of a bug (`msg_bugs_outbox`, [`crate::bugs`]) are two
//! tables of the same shape; the queries are written here once, by the
//! name of the table and of the column that holds the body.

use crate::{storage, Store};
use messenger_core::Result;
use sqlx::sqlite::SqliteRow;

pub const STATE_QUEUED: &str = "queued";
pub const STATE_SENT: &str = "sent";
pub const STATE_REFUSED: &str = "refused";
pub const STATE_EXPIRED: &str = "expired";

/// One outbox: its table and the column of the body. Both are names
/// written in this crate, never input.
pub(crate) struct Outbox {
    pub table: &'static str,
    pub body: &'static str,
}

impl Outbox {
    fn columns(&self) -> String {
        format!(
            "id, {}, state, attempts, next_retry_at, last_error, created_at, updated_at",
            self.body
        )
    }
}

/// A row as the query reads it.
pub(crate) trait Row: for<'r> sqlx::FromRow<'r, SqliteRow> + Send + Unpin {}
impl<T: for<'r> sqlx::FromRow<'r, SqliteRow> + Send + Unpin> Row for T {}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn queue(
    store: &Store,
    outbox: &Outbox,
    id: &str,
    body: &str,
    now: i64,
    attempts: i64,
    next_retry_at: i64,
    last_error: Option<&str>,
) -> Result<()> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO {} ({}) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        outbox.table,
        outbox.columns()
    )))
    .bind(id)
    .bind(body)
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

pub(crate) async fn due<R: Row>(
    store: &Store,
    outbox: &Outbox,
    now: i64,
    limit: i64,
) -> Result<Vec<R>> {
    sqlx::query_as::<_, R>(sqlx::AssertSqlSafe(format!(
        "SELECT {} FROM {} WHERE state = 'queued' AND next_retry_at <= ?
         ORDER BY created_at ASC, id ASC LIMIT ?",
        outbox.columns(),
        outbox.table
    )))
    .bind(now)
    .bind(limit)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

pub(crate) async fn get<R: Row>(store: &Store, outbox: &Outbox, id: &str) -> Result<Option<R>> {
    sqlx::query_as::<_, R>(sqlx::AssertSqlSafe(format!(
        "SELECT {} FROM {} WHERE id = ?",
        outbox.columns(),
        outbox.table
    )))
    .bind(id)
    .fetch_optional(store.pool())
    .await
    .map_err(storage)
}

pub(crate) async fn retry_later(
    store: &Store,
    outbox: &Outbox,
    id: &str,
    attempts: i64,
    next_retry_at: i64,
    error: &str,
    now: i64,
) -> Result<()> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE {} SET attempts = ?, next_retry_at = ?, last_error = ?, updated_at = ? WHERE id = ?",
        outbox.table
    )))
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

/// Done: never tried again, and the body goes.
pub(crate) async fn finish(
    store: &Store,
    outbox: &Outbox,
    id: &str,
    state: &str,
    error: Option<&str>,
    now: i64,
) -> Result<()> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE {} SET state = ?, {} = '', last_error = COALESCE(?, last_error), updated_at = ? WHERE id = ?",
        outbox.table, outbox.body
    )))
    .bind(state)
    .bind(error)
    .bind(now)
    .bind(id)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

pub(crate) async fn purge_done(store: &Store, outbox: &Outbox, before: i64) -> Result<u64> {
    let res = sqlx::query(sqlx::AssertSqlSafe(format!(
        "DELETE FROM {} WHERE state <> 'queued' AND updated_at < ?",
        outbox.table
    )))
    .bind(before)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(res.rows_affected())
}

pub(crate) async fn count_queued(store: &Store, outbox: &Outbox) -> Result<i64> {
    sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!(
        "SELECT COUNT(*) FROM {} WHERE state = 'queued'",
        outbox.table
    )))
    .fetch_one(store.pool())
    .await
    .map_err(storage)
}
