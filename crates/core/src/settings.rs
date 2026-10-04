// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The table `app_settings`: one row per key, the value a string.
//!
//! The functions of this module read and write through any executor, so they
//! work inside the caller's transaction. [`Settings`] is the handle kept in
//! [`crate::Core`]: the same reads and writes on the pool, plus watchers that
//! hear of a key that changed.

use crate::error::AppError;
use crate::BoxFuture;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sqlx::{Pool, Sqlite, SqliteExecutor};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// The stored value, or the reason it could not be read.
pub async fn try_get(db: impl SqliteExecutor<'_>, key: &str) -> Result<Option<String>, AppError> {
    sqlx::query_scalar::<_, String>("SELECT value FROM app_settings WHERE key = ?")
        .bind(key)
        .fetch_optional(db)
        .await
        .map_err(AppError::db)
}

/// The stored value; `None` when the key is not set and when it cannot be read.
pub async fn get(db: impl SqliteExecutor<'_>, key: &str) -> Option<String> {
    try_get(db, key).await.ok().flatten()
}

pub async fn set(db: impl SqliteExecutor<'_>, key: &str, value: &str) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO app_settings (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(db)
    .await
    .map_err(AppError::db)?;
    Ok(())
}

pub async fn delete(db: impl SqliteExecutor<'_>, key: &str) -> Result<(), AppError> {
    sqlx::query("DELETE FROM app_settings WHERE key = ?")
        .bind(key)
        .execute(db)
        .await
        .map_err(AppError::db)?;
    Ok(())
}

/// Store the value, or remove the key when there is none.
pub async fn set_opt(
    db: impl SqliteExecutor<'_>,
    key: &str,
    value: Option<&str>,
) -> Result<(), AppError> {
    match value {
        Some(value) => set(db, key, value).await,
        None => delete(db, key).await,
    }
}

/// A flag stored as "1" / "0"; `false` when the key is not set.
pub async fn get_bool(db: impl SqliteExecutor<'_>, key: &str) -> bool {
    get(db, key).await.is_some_and(|v| v == "1")
}

pub async fn set_bool(db: impl SqliteExecutor<'_>, key: &str, on: bool) -> Result<(), AppError> {
    set(db, key, if on { "1" } else { "0" }).await
}

/// A value stored as JSON; `None` when the key is not set and when what is
/// stored does not parse.
pub async fn get_json<T: DeserializeOwned>(db: impl SqliteExecutor<'_>, key: &str) -> Option<T> {
    get(db, key)
        .await
        .and_then(|json| serde_json::from_str(&json).ok())
}

pub async fn set_json<T: Serialize + ?Sized>(
    db: impl SqliteExecutor<'_>,
    key: &str,
    value: &T,
) -> Result<(), AppError> {
    let json = serde_json::to_string(value).map_err(AppError::other)?;
    set(db, key, &json).await
}

/// Every key that starts with `prefix`, with its value.
pub async fn with_prefix(
    db: impl SqliteExecutor<'_>,
    prefix: &str,
) -> Result<Vec<(String, String)>, AppError> {
    sqlx::query_as("SELECT key, value FROM app_settings WHERE substr(key, 1, length(?)) = ?")
        .bind(prefix)
        .bind(prefix)
        .fetch_all(db)
        .await
        .map_err(AppError::db)
}

/// Called with the new value of a watched key, `None` when the key was deleted.
type Watcher = Arc<dyn Fn(Option<String>) -> BoxFuture<'static, ()> + Send + Sync>;

/// The settings of the app, with watchers per key.
pub struct Settings {
    db: Pool<Sqlite>,
    watchers: RwLock<HashMap<&'static str, Vec<Watcher>>>,
}

impl Settings {
    pub fn new(db: Pool<Sqlite>) -> Self {
        Self {
            db,
            watchers: RwLock::new(HashMap::new()),
        }
    }

    pub async fn get(&self, key: &str) -> Option<String> {
        get(&self.db, key).await
    }

    /// Store the value and tell the watchers of the key.
    pub async fn set(&self, key: &str, value: &str) -> Result<(), AppError> {
        set(&self.db, key, value).await?;
        self.notify(key, Some(value.to_owned())).await;
        Ok(())
    }

    /// Remove the key and tell its watchers.
    pub async fn delete(&self, key: &str) -> Result<(), AppError> {
        delete(&self.db, key).await?;
        self.notify(key, None).await;
        Ok(())
    }

    /// Hear of every change of `key` made through this handle or reported
    /// with [`Settings::changed`]. Called in a module's setup.
    pub fn subscribe(
        &self,
        key: &'static str,
        watcher: impl Fn(Option<String>) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    ) {
        self.watchers
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .entry(key)
            .or_default()
            .push(Arc::new(watcher));
    }

    /// Tell the watchers of `key` that its row was written by other means —
    /// a transaction of the caller, sync applying a remote value — once that
    /// write is committed.
    pub async fn changed(&self, key: &str) {
        if !self.watched(key) {
            return;
        }
        let value = self.get(key).await;
        self.notify(key, value).await;
    }

    fn watched(&self, key: &str) -> bool {
        self.watchers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(key)
    }

    async fn notify(&self, key: &str, value: Option<String>) {
        let watchers = self
            .watchers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .cloned()
            .unwrap_or_default();
        for watcher in watchers {
            watcher(value.clone()).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use std::sync::Mutex;

    async fn pool() -> (Pool<Sqlite>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::open(&dir.path().join(db::DB_FILE), &[db::SCHEMA])
            .await
            .unwrap();
        (pool, dir)
    }

    #[tokio::test]
    async fn a_key_is_absent_until_set_and_after_delete() {
        let (db, _dir) = pool().await;
        assert_eq!(get(&db, "k").await, None);
        assert_eq!(try_get(&db, "k").await.unwrap(), None);

        set(&db, "k", "one").await.unwrap();
        set(&db, "k", "two").await.unwrap();
        assert_eq!(get(&db, "k").await.as_deref(), Some("two"));

        delete(&db, "k").await.unwrap();
        delete(&db, "k").await.unwrap();
        assert_eq!(get(&db, "k").await, None);

        set_opt(&db, "k", Some("three")).await.unwrap();
        assert_eq!(get(&db, "k").await.as_deref(), Some("three"));
        set_opt(&db, "k", None).await.unwrap();
        assert_eq!(get(&db, "k").await, None);
        db.close().await;
    }

    /// An empty string is a value: callers that treat it as unset say so themselves.
    #[tokio::test]
    async fn an_empty_value_is_stored_as_it_is() {
        let (db, _dir) = pool().await;
        set(&db, "k", "").await.unwrap();
        assert_eq!(get(&db, "k").await.as_deref(), Some(""));
        db.close().await;
    }

    #[tokio::test]
    async fn a_flag_is_true_only_for_one() {
        let (db, _dir) = pool().await;
        assert!(!get_bool(&db, "flag").await);
        set_bool(&db, "flag", true).await.unwrap();
        assert_eq!(get(&db, "flag").await.as_deref(), Some("1"));
        assert!(get_bool(&db, "flag").await);
        set_bool(&db, "flag", false).await.unwrap();
        assert_eq!(get(&db, "flag").await.as_deref(), Some("0"));
        assert!(!get_bool(&db, "flag").await);
        set(&db, "flag", "true").await.unwrap();
        assert!(!get_bool(&db, "flag").await);
        db.close().await;
    }

    #[tokio::test]
    async fn json_that_does_not_parse_reads_as_unset() {
        let (db, _dir) = pool().await;
        assert_eq!(get_json::<Vec<u32>>(&db, "list").await, None);
        set_json(&db, "list", &[1u32, 2]).await.unwrap();
        assert_eq!(get(&db, "list").await.as_deref(), Some("[1,2]"));
        assert_eq!(get_json::<Vec<u32>>(&db, "list").await, Some(vec![1, 2]));
        set(&db, "list", "not json").await.unwrap();
        assert_eq!(get_json::<Vec<u32>>(&db, "list").await, None);
        db.close().await;
    }

    #[tokio::test]
    async fn a_read_that_fails_is_none_or_an_error() {
        let (db, _dir) = pool().await;
        sqlx::query("DROP TABLE app_settings")
            .execute(&db)
            .await
            .unwrap();
        assert_eq!(get(&db, "k").await, None);
        assert!(!get_bool(&db, "k").await);
        assert!(matches!(try_get(&db, "k").await, Err(AppError::Db(_))));
        assert!(matches!(set(&db, "k", "v").await, Err(AppError::Db(_))));
        db.close().await;
    }

    #[tokio::test]
    async fn a_write_in_a_transaction_goes_with_it() {
        let (db, _dir) = pool().await;
        let mut tx = db.begin().await.unwrap();
        set(&mut *tx, "k", "v").await.unwrap();
        assert_eq!(get(&mut *tx, "k").await.as_deref(), Some("v"));
        tx.rollback().await.unwrap();
        assert_eq!(get(&db, "k").await, None);
        db.close().await;
    }

    #[tokio::test]
    async fn keys_are_listed_by_prefix() {
        let (db, _dir) = pool().await;
        for (key, value) in [("secret:a", "1"), ("secret:b", "2"), ("secre", "3"), ("other", "4")] {
            set(&db, key, value).await.unwrap();
        }
        let mut found = with_prefix(&db, "secret:").await.unwrap();
        found.sort();
        assert_eq!(
            found,
            vec![
                ("secret:a".to_string(), "1".to_string()),
                ("secret:b".to_string(), "2".to_string())
            ]
        );
        db.close().await;
    }

    #[tokio::test]
    async fn watchers_hear_of_their_key_only() {
        let (db, _dir) = pool().await;
        let settings = Settings::new(db.clone());
        let heard: Arc<Mutex<Vec<Option<String>>>> = Arc::default();
        let sink = heard.clone();
        settings.subscribe("watched", move |value| {
            let sink = sink.clone();
            Box::pin(async move { sink.lock().unwrap().push(value) })
        });

        settings.set("other", "x").await.unwrap();
        settings.changed("other").await;
        settings.set("watched", "a").await.unwrap();
        assert_eq!(settings.get("watched").await.as_deref(), Some("a"));
        // Written past the handle, then reported.
        set(&db, "watched", "b").await.unwrap();
        settings.changed("watched").await;
        settings.delete("watched").await.unwrap();

        assert_eq!(
            *heard.lock().unwrap(),
            vec![Some("a".to_string()), Some("b".to_string()), None]
        );
        db.close().await;
    }
}
