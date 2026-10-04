// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Deletion hooks: how a module clears its references to an entity another
//! module owns, without either knowing the other.
//!
//! The owner reports "the entity of kind K with id X is deleted" inside the
//! transaction that deletes it; every subscriber of K cleans its own tables
//! in that same transaction, so the deletion and the clean-up are atomic.

use crate::error::AppError;
use crate::BoxFuture;
use sqlx::{Sqlite, Transaction};
use std::collections::HashMap;
use std::sync::RwLock;

/// Clears the subscriber's references to the deleted entity `id`.
pub type DeletionHook = for<'a> fn(
    &'a mut Transaction<'_, Sqlite>,
    &'a str,
) -> BoxFuture<'a, Result<(), AppError>>;

#[derive(Default)]
pub struct Deletions {
    hooks: RwLock<HashMap<&'static str, Vec<DeletionHook>>>,
}

impl Deletions {
    /// Subscribe to deletions of entities of `kind` (a kind of the entity
    /// directory). Called in a module's setup.
    pub fn subscribe(&self, kind: &'static str, hook: DeletionHook) {
        self.hooks
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .entry(kind)
            .or_default()
            .push(hook);
    }

    /// Called by the owner in the transaction in which it deletes the entity,
    /// for a local deletion and for one applied from sync alike. A kind
    /// nobody subscribed to is not an error. The first hook that fails stops
    /// the rest; the owner then drops the transaction.
    pub async fn emit(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        kind: &str,
        id: &str,
    ) -> Result<(), AppError> {
        let hooks = self
            .hooks
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(kind)
            .cloned()
            .unwrap_or_default();
        for hook in hooks {
            hook(tx, id).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::Pool;

    /// The owner's table and two tables of subscribers that point into it.
    async fn pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for sql in [
            "CREATE TABLE notes (id TEXT PRIMARY KEY NOT NULL)",
            "CREATE TABLE tags (note_id TEXT NOT NULL)",
            "CREATE TABLE pins (note_id TEXT NOT NULL)",
            "INSERT INTO notes VALUES ('n1'), ('n2')",
            "INSERT INTO tags VALUES ('n1'), ('n2')",
            "INSERT INTO pins VALUES ('n1'), ('n2')",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        pool
    }

    fn clear_tags<'a>(
        tx: &'a mut Transaction<'_, Sqlite>,
        id: &'a str,
    ) -> BoxFuture<'a, Result<(), AppError>> {
        Box::pin(async move {
            sqlx::query("DELETE FROM tags WHERE note_id = ?")
                .bind(id)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    }

    fn clear_pins<'a>(
        tx: &'a mut Transaction<'_, Sqlite>,
        id: &'a str,
    ) -> BoxFuture<'a, Result<(), AppError>> {
        Box::pin(async move {
            sqlx::query("DELETE FROM pins WHERE note_id = ?")
                .bind(id)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    }

    fn refuse<'a>(
        _tx: &'a mut Transaction<'_, Sqlite>,
        _id: &'a str,
    ) -> BoxFuture<'a, Result<(), AppError>> {
        Box::pin(async { Err(AppError::other("refused")) })
    }

    async fn ids(pool: &Pool<Sqlite>, table: &str) -> Vec<String> {
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT * FROM {table} ORDER BY 1"
        )))
        .fetch_all(pool)
        .await
        .unwrap()
    }

    async fn delete_note(pool: &Pool<Sqlite>, deletions: &Deletions, id: &str) -> Result<(), AppError> {
        let mut tx = pool.begin().await?;
        sqlx::query("DELETE FROM notes WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        deletions.emit(&mut tx, "note", id).await?;
        tx.commit().await?;
        Ok(())
    }

    #[tokio::test]
    async fn every_subscriber_of_the_kind_cleans_up_in_the_owner_s_transaction() {
        let pool = pool().await;
        let deletions = Deletions::default();
        deletions.subscribe("note", clear_tags);
        deletions.subscribe("note", clear_pins);
        deletions.subscribe("password", refuse);

        delete_note(&pool, &deletions, "n1").await.unwrap();

        assert_eq!(ids(&pool, "notes").await, vec!["n2"]);
        assert_eq!(ids(&pool, "tags").await, vec!["n2"]);
        assert_eq!(ids(&pool, "pins").await, vec!["n2"]);
    }

    #[tokio::test]
    async fn a_kind_without_subscribers_is_not_an_error() {
        let pool = pool().await;
        let deletions = Deletions::default();

        delete_note(&pool, &deletions, "n1").await.unwrap();

        assert_eq!(ids(&pool, "notes").await, vec!["n2"]);
        assert_eq!(ids(&pool, "tags").await, vec!["n1", "n2"]);
    }

    #[tokio::test]
    async fn a_hook_that_fails_takes_the_deletion_back() {
        let pool = pool().await;
        let deletions = Deletions::default();
        deletions.subscribe("note", clear_tags);
        deletions.subscribe("note", refuse);
        deletions.subscribe("note", clear_pins);

        let failed = delete_note(&pool, &deletions, "n1").await;

        assert!(matches!(failed, Err(AppError::Other(m)) if m == "refused"));
        assert_eq!(ids(&pool, "notes").await, vec!["n1", "n2"]);
        assert_eq!(ids(&pool, "tags").await, vec!["n1", "n2"]);
        assert_eq!(ids(&pool, "pins").await, vec!["n1", "n2"]);
    }
}
