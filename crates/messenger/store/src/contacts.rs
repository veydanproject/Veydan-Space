// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_private_contacts` (address book, local and private) and
//! `msg_follows` (our public kind-3 follow list). DM relationship state is
//! added to the address book in stage 5b.

use crate::{storage, Store};
use messenger_core::Result;

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct ContactRow {
    pub pubkey: String,
    pub nickname: Option<String>,
    pub note: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
}

#[derive(Clone, Debug, Default)]
pub struct ContactPatch {
    pub nickname: Option<Option<String>>,
    pub note: Option<Option<String>>,
}

const COLS: &str = "pubkey, nickname, note, created_at, updated_at, deleted_at";

pub async fn list_active(store: &Store) -> Result<Vec<ContactRow>> {
    sqlx::query_as::<_, ContactRow>(
        "SELECT pubkey, nickname, note, created_at, updated_at, deleted_at
         FROM msg_private_contacts WHERE deleted_at IS NULL ORDER BY updated_at DESC",
    )
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

pub async fn get(store: &Store, pubkey: &str) -> Result<Option<ContactRow>> {
    sqlx::query_as::<_, ContactRow>(
        "SELECT pubkey, nickname, note, created_at, updated_at, deleted_at
         FROM msg_private_contacts WHERE pubkey = ?",
    )
    .bind(pubkey)
    .fetch_optional(store.pool())
    .await
    .map_err(storage)
}

/// Add (or undelete) a contact. Existing settings are kept.
pub async fn add(store: &Store, pubkey: &str, nickname: Option<&str>) -> Result<ContactRow> {
    let now = crate::now();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO msg_private_contacts ({COLS}) VALUES (?, ?, NULL, ?, ?, NULL)
         ON CONFLICT(pubkey) DO UPDATE SET
           nickname = COALESCE(excluded.nickname, msg_private_contacts.nickname),
           deleted_at = NULL, updated_at = excluded.updated_at"
    )))
    .bind(pubkey)
    .bind(nickname)
    .bind(now)
    .bind(now)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    get(store, pubkey).await?.ok_or_else(|| messenger_core::MessengerError::Storage("contact vanished".into()))
}

pub async fn update(store: &Store, pubkey: &str, patch: &ContactPatch) -> Result<()> {
    let Some(mut row) = get(store, pubkey).await? else {
        return Err(messenger_core::MessengerError::Invalid("unknown contact".into()));
    };
    if let Some(v) = &patch.nickname {
        row.nickname = v.clone();
    }
    if let Some(v) = &patch.note {
        row.note = v.clone();
    }
    sqlx::query("UPDATE msg_private_contacts SET nickname = ?, note = ?, updated_at = ? WHERE pubkey = ?")
        .bind(&row.nickname)
        .bind(&row.note)
        .bind(crate::now())
        .bind(pubkey)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// Soft delete: the row stays so per-peer state survives a re-add.
pub async fn remove(store: &Store, pubkey: &str) -> Result<()> {
    sqlx::query("UPDATE msg_private_contacts SET deleted_at = ?, updated_at = ? WHERE pubkey = ?")
        .bind(crate::now())
        .bind(crate::now())
        .bind(pubkey)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

// ─── Follow list (kind 3) ────────────────────────────────────────────────────

pub async fn follows(store: &Store) -> Result<Vec<String>> {
    sqlx::query_scalar::<_, String>("SELECT pubkey FROM msg_follows ORDER BY added_at, pubkey")
        .fetch_all(store.pool())
        .await
        .map_err(storage)
}

/// Replace the whole follow list (what a kind-3 event means).
pub async fn replace_follows(store: &Store, pubkeys: &[String]) -> Result<()> {
    let mut tx = store.pool().begin().await.map_err(storage)?;
    sqlx::query("DELETE FROM msg_follows").execute(&mut *tx).await.map_err(storage)?;
    let now = crate::now();
    for pk in pubkeys {
        sqlx::query("INSERT OR IGNORE INTO msg_follows (pubkey, added_at) VALUES (?, ?)")
            .bind(pk)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
    }
    tx.commit().await.map_err(storage)
}

pub async fn follow(store: &Store, pubkey: &str) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO msg_follows (pubkey, added_at) VALUES (?, ?)")
        .bind(pubkey)
        .bind(crate::now())
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn unfollow(store: &Store, pubkey: &str) -> Result<()> {
    sqlx::query("DELETE FROM msg_follows WHERE pubkey = ?")
        .bind(pubkey)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn add_update_remove_readd_keeps_settings() {
        let s = Store::open_in_memory().await.unwrap();
        let c = add(&s, "a", Some("Al")).await.unwrap();
        assert_eq!(c.nickname.as_deref(), Some("Al"));
        update(&s, "a", &ContactPatch { note: Some(Some("n".into())), ..Default::default() }).await.unwrap();
        assert_eq!(get(&s, "a").await.unwrap().unwrap().note.as_deref(), Some("n"));
        remove(&s, "a").await.unwrap();
        assert!(list_active(&s).await.unwrap().is_empty());
        let again = add(&s, "a", None).await.unwrap();
        assert_eq!(again.nickname.as_deref(), Some("Al"), "settings survive re-add");
        assert_eq!(again.note.as_deref(), Some("n"));
        assert!(again.deleted_at.is_none());
        assert!(update(&s, "nope", &ContactPatch::default()).await.is_err());
    }

    #[tokio::test]
    async fn follow_list_replace_and_edit() {
        let s = Store::open_in_memory().await.unwrap();
        replace_follows(&s, &["b".into(), "a".into(), "b".into()]).await.unwrap();
        assert_eq!(follows(&s).await.unwrap().len(), 2);
        follow(&s, "c").await.unwrap();
        unfollow(&s, "a").await.unwrap();
        let f = follows(&s).await.unwrap();
        assert!(f.contains(&"b".to_string()) && f.contains(&"c".to_string()) && !f.contains(&"a".to_string()));
        replace_follows(&s, &[]).await.unwrap();
        assert!(follows(&s).await.unwrap().is_empty());
    }
}
