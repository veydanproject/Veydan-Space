// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `communities.db`: the module's own database, opened and migrated the
//! way the messenger opens `messenger.db` (crates/messenger/store): one
//! SQLite file beside the others, WAL, migrations compiled in
//! (`schema`), so that the module can be switched off or removed without
//! touching anybody else's data.
//!
//! Repositories are plain functions over the pool, one file per part:
//! `channels` (the cache of a channel, its journal, keepers, invitations,
//! requests, admissions, sealed events that wait), `keys` (epochs and
//! bundles of a member), `content` (posts, comments, reactions, what was
//! read), `marks`, `snapshots` (snapshots and the cache of shards),
//! `avatar`; `keeper` (the state of a keeper as rows, its cycle);
//! `outbox` (what waits to be sent).

pub mod avatar;
pub mod channels;
pub mod cloud_keys;
pub mod content;
pub mod handover;
pub mod keeper;
pub mod keys;
pub mod levels;
pub mod marks;
pub mod outbox;
mod schema;
pub mod snapshots;
pub mod tickets;
pub mod transfers;
pub mod upload_keys;

use messenger_core::{MessengerError, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Pool, Sqlite};
use std::path::Path;
use std::str::FromStr;

/// The name of the file in the data folder of the app.
pub const FILE_NAME: &str = "communities.db";

/// Unix seconds now; the database stamps rows itself.
pub(crate) fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub(crate) fn storage(e: impl std::fmt::Display) -> MessengerError {
    MessengerError::Storage(e.to_string())
}

/// The open database.
#[derive(Clone)]
pub struct Db {
    pool: Pool<Sqlite>,
}

impl Db {
    /// Open (creating if needed) and migrate the database at `path`.
    pub async fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        let url = format!("sqlite://{}", path.to_string_lossy());
        let options = SqliteConnectOptions::from_str(&url)
            .map_err(storage)?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_secs(5));
        let pool = SqlitePoolOptions::new().max_connections(4).connect_with(options).await.map_err(storage)?;
        schema::migrations().run(&pool).await.map_err(storage)?;
        Ok(Self { pool })
    }

    /// In-memory database for tests. Same migrations, no file.
    pub async fn open_in_memory() -> Result<Self> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:").map_err(storage)?.foreign_keys(true);
        let pool = SqlitePoolOptions::new().max_connections(1).connect_with(options).await.map_err(storage)?;
        schema::migrations().run(&pool).await.map_err(storage)?;
        Ok(Self { pool })
    }

    /// Version of the last migration compiled into this crate.
    pub fn latest_migration() -> i64 {
        schema::migrations().iter().map(|m| m.version).max().unwrap_or(0)
    }

    pub fn pool(&self) -> &Pool<Sqlite> {
        &self.pool
    }

    /// A transaction that writes: `BEGIN IMMEDIATE`, so it takes the lock of
    /// writing at its start and waits for it (`busy_timeout`). A deferred
    /// one that read first fails at its first write with `database is
    /// locked` (517, a stale snapshot) whenever another connection of the
    /// pool wrote in between, with no wait at all (живой запуск 2026-10-23:
    /// the keeper dropped a join «not taken, storage error: … (code: 517)
    /// database is locked» while the listener wrote its rounds).
    pub async fn write_tx(&self) -> Result<sqlx::Transaction<'static, Sqlite>> {
        self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(storage)
    }

    /// Latest applied migration version.
    pub async fn schema_version(&self) -> Result<i64> {
        use sqlx::migrate::Migrate;
        let mut conn = self.pool.acquire().await.map_err(storage)?;
        // The table sqlx records migrations in, its documented default.
        let applied = conn.list_applied_migrations("_sqlx_migrations").await.map_err(storage)?;
        Ok(applied.iter().map(|m| m.version).max().unwrap_or(0))
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn opens_and_migrates_a_file_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join(FILE_NAME);
        let db = Db::open(&path).await.unwrap();
        assert_eq!(db.schema_version().await.unwrap(), Db::latest_migration());
        assert_eq!(Db::latest_migration(), 34);
        let tables: Vec<String> = sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'cm_%' ORDER BY name").fetch_all(db.pool()).await.unwrap();
        assert_eq!(
            tables,
            [
                "cm_admits", "cm_avatar", "cm_bundles", "cm_channels", "cm_cloud_keys", "cm_comments", "cm_epochs", "cm_epochs_heard", "cm_genesis", "cm_heartbeats", "cm_invites", "cm_joining", "cm_keeper", "cm_keeper_blobs", "cm_keeper_cloud_keys", "cm_keeper_events", "cm_keeper_flat", "cm_keeper_header", "cm_keeper_held_joins", "cm_keeper_inbox", "cm_keeper_invites",
                "cm_keeper_members", "cm_keeper_missed", "cm_keeper_ops", "cm_keeper_pending", "cm_keeper_readers", "cm_keeper_retired", "cm_keeper_revoked", "cm_keeper_snapshot", "cm_keeper_take", "cm_keeper_tickets", "cm_keeper_upload_keys", "cm_keeper_uploads", "cm_keeper_wraps",
                "cm_keepers", "cm_known_files", "cm_level_requests", "cm_levels", "cm_link_owed", "cm_local_files", "cm_marks", "cm_meta_owed", "cm_ops", "cm_outbox", "cm_pending",
                "cm_posts", "cm_reactions", "cm_read", "cm_readers", "cm_requests", "cm_scopes", "cm_snapshots", "cm_staff", "cm_sync", "cm_tickets", "cm_transfers", "cm_upload_keys", "cm_uploads"
            ]
        );
        // The highest epoch heard is kept across a restart (a take of the
        // keeper numbers past it), and goes with the channel forgotten.
        keys::note_epoch_heard(&db, "c1", 7).await.unwrap();
        keys::note_epoch_heard(&db, "c1", 5).await.unwrap();
        db.close().await;
        // Opened again: the migrations are not run twice.
        let again = Db::open(&path).await.unwrap();
        assert_eq!(again.schema_version().await.unwrap(), Db::latest_migration());
        assert_eq!(keys::epoch_heard(&again, "c1").await.unwrap(), 7);
        channels::delete(&again, "c1").await.unwrap();
        assert_eq!(keys::epoch_heard(&again, "c1").await.unwrap(), 0);
        again.close().await;
    }

    /// A transaction that reads, then writes after another connection wrote
    /// in between (живой запуск 2026-10-23: «(code: 517) database is
    /// locked», a join of the keeper dropped): `write_tx` holds the lock of
    /// writing from its start, the other writer waits for it, both land.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_write_transaction_that_read_first_is_not_refused_for_a_write_in_between() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join(FILE_NAME)).await.unwrap();
        let mut tx = db.write_tx().await.unwrap();
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM cm_joining").fetch_one(&mut *tx).await.unwrap();
        assert_eq!(n, 0);
        let other = db.clone();
        let between = tokio::spawn(async move { channels::put_joining(&other, "b", "{}", 1, 2).await });
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        sqlx::query("INSERT INTO cm_joining (community_id, join_json, first_at, next_at, attempts) VALUES ('a', '{}', 1, 2, 1)").execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        between.await.unwrap().unwrap();
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM cm_joining").fetch_one(db.pool()).await.unwrap();
        assert_eq!(n, 2);
        db.close().await;
    }
}
