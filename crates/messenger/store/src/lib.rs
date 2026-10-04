// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Messenger persistence.
//!
//! The messenger owns a separate SQLite file (`<data_dir>/messenger.db`) so
//! the module can be removed or extracted without touching host data.
//! Migrations live in `migrations/` and are embedded at compile time.
//!
//! Repositories are plain functions over the pool, grouped per module
//! (`settings`, later `identity`, `relays`, `messages`, …).

pub mod chats;
pub mod contacts;
pub mod cursors;
pub mod dm_relations;
pub mod dm_routes;
pub mod events_raw;
pub mod groups;
pub mod identity;
pub mod link_previews;
pub mod media;
pub mod messages;
pub mod outbox;
pub mod profiles;
pub mod relays;
pub mod settings;
pub mod shared;

/// Unix seconds now; the store stamps rows itself.
pub(crate) fn now() -> i64 {
    use messenger_core::Clock;
    messenger_core::traits::SystemClock.now().secs()
}

use messenger_core::{MessengerConfig, MessengerError, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Pool, Sqlite};
use std::str::FromStr;

/// Open (creating if needed) and migrate the messenger database.
#[derive(Clone)]
pub struct Store {
    pool: Pool<Sqlite>,
}

impl Store {
    pub async fn open(config: &MessengerConfig) -> Result<Self> {
        tokio::fs::create_dir_all(config.data_dir()).await?;
        let path = config.db_path();
        let url = format!("sqlite://{}", path.to_string_lossy());
        let options = SqliteConnectOptions::from_str(&url)
            .map_err(storage)?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .map_err(storage)?;
        sqlx::migrate!("./migrations").run(&pool).await.map_err(storage)?;
        Ok(Self { pool })
    }

    /// The database of a running app, opened by somebody else in the same
    /// process or another (a push handler): one connection, no writes, no
    /// migrations. A schema newer or older than this code knows is refused,
    /// since a push may come before the app has migrated after an update.
    pub async fn open_read_only(config: &MessengerConfig) -> Result<Self> {
        let options = SqliteConnectOptions::new()
            .filename(config.db_path())
            .read_only(true)
            .create_if_missing(false)
            .busy_timeout(std::time::Duration::from_secs(2));
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(storage)?;
        let store = Self { pool };
        let have = store.schema_version().await?;
        let want = Self::latest_migration();
        if have != want {
            store.close().await;
            return Err(MessengerError::Storage(format!("schema is at {have}, this code knows {want}")));
        }
        Ok(store)
    }

    /// Version of the last migration compiled into this crate.
    pub fn latest_migration() -> i64 {
        sqlx::migrate!("./migrations").migrations.iter().map(|m| m.version).max().unwrap_or(0)
    }

    /// In-memory database for tests. Same migrations, no file.
    pub async fn open_in_memory() -> Result<Self> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .map_err(storage)?
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(storage)?;
        sqlx::migrate!("./migrations").run(&pool).await.map_err(storage)?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &Pool<Sqlite> {
        &self.pool
    }

    /// Latest applied migration version, for `status()`.
    pub async fn schema_version(&self) -> Result<i64> {
        sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations")
            .fetch_one(&self.pool)
            .await
            .map_err(storage)
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }
}

pub(crate) fn storage(e: impl std::fmt::Display) -> MessengerError {
    MessengerError::Storage(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn opens_and_migrates_a_file_database() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("msg"));
        let store = Store::open(&cfg).await.unwrap();
        assert!(cfg.db_path().exists());
        assert!(store.schema_version().await.unwrap() >= 1);
        store.close().await;

        // Reopening is idempotent.
        let again = Store::open(&cfg).await.unwrap();
        assert!(again.schema_version().await.unwrap() >= 1);
    }

    #[tokio::test]
    async fn a_reader_sees_what_the_app_wrote_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("msg"));
        assert!(Store::open_read_only(&cfg).await.is_err(), "no database yet");

        let app = Store::open(&cfg).await.unwrap();
        chats::ensure_dm(&app, "aa").await.unwrap();
        let reader = Store::open_read_only(&cfg).await.unwrap();
        assert_eq!(reader.schema_version().await.unwrap(), Store::latest_migration());
        assert!(chats::get(&reader, "dm:aa").await.unwrap().is_some());
        assert!(chats::set_pinned(&reader, "dm:aa", true).await.is_err(), "read-only");

        // Written after the reader opened: still seen (WAL, no snapshot held).
        chats::ensure_dm(&app, "bb").await.unwrap();
        assert!(chats::get(&reader, "dm:bb").await.unwrap().is_some());
        reader.close().await;

        // A database from another version of the app is refused.
        sqlx::query("UPDATE _sqlx_migrations SET version = version + 1000 WHERE version = (SELECT MAX(version) FROM _sqlx_migrations)")
            .execute(app.pool())
            .await
            .unwrap();
        assert!(Store::open_read_only(&cfg).await.is_err());
    }
}
