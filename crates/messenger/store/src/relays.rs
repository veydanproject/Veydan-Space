// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_relays`: the configured relay set. Two sources: rows managed by the
//! infrastructure manifest (with a stable `relay_id`) and rows the user
//! added by hand. `migrate_url` rewrites a relay address everywhere it is
//! referenced; tables that store relay URLs must be listed in
//! `RELAY_URL_REFS` so a manifest rename never leaves orphans.

use crate::{storage, Store};
use messenger_core::Result;
use sqlx::AssertSqlSafe;

pub const SOURCE_MANIFEST: &str = "manifest";
pub const SOURCE_USER: &str = "user";

/// `(table, column)` pairs that reference `msg_relays.url`. Extend when a
/// new table stores relay URLs (scoped relays, DM routes, …). Identifiers
/// only; they are spliced into SQL with `AssertSqlSafe`.
pub const RELAY_URL_REFS: &[(&str, &str)] = &[];

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct RelayRow {
    pub url: String,
    pub relay_id: Option<String>,
    pub source: String,
    pub regions_json: String,
    pub read: bool,
    pub write: bool,
    pub enabled: bool,
    pub auth_type: Option<String>,
    /// API key for `auth_type = api_key`; never exposed to the UI.
    pub auth_secret: Option<String>,
    pub failures: i64,
    pub last_ok_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl RelayRow {
    pub fn regions(&self) -> Vec<String> {
        serde_json::from_str(&self.regions_json).unwrap_or_default()
    }
}

/// Fields the caller decides; timestamps are filled in here.
#[derive(Clone, Debug)]
pub struct RelayUpsert {
    pub url: String,
    pub relay_id: Option<String>,
    pub source: String,
    pub regions: Vec<String>,
    pub read: bool,
    pub write: bool,
    pub enabled: bool,
    pub auth_type: Option<String>,
    pub auth_secret: Option<String>,
}

pub async fn list(store: &Store) -> Result<Vec<RelayRow>> {
    sqlx::query_as::<_, RelayRow>(
        "SELECT url, relay_id, source, regions_json, read, write, enabled, auth_type, auth_secret, failures, last_ok_at, created_at, updated_at
         FROM msg_relays ORDER BY source, url",
    )
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

pub async fn get(store: &Store, url: &str) -> Result<Option<RelayRow>> {
    sqlx::query_as::<_, RelayRow>(
        "SELECT url, relay_id, source, regions_json, read, write, enabled, auth_type, auth_secret, failures, last_ok_at, created_at, updated_at
         FROM msg_relays WHERE url = ?",
    )
    .bind(url)
    .fetch_optional(store.pool())
    .await
    .map_err(storage)
}

/// Insert or update by url. `enabled` of an existing row is preserved
/// unless `overwrite_enabled` is set.
pub async fn upsert(store: &Store, r: &RelayUpsert, overwrite_enabled: bool) -> Result<()> {
    let now = crate::now();
    let regions = serde_json::to_string(&r.regions).unwrap_or_else(|_| "[]".into());
    let sql = if overwrite_enabled {
        "INSERT INTO msg_relays (url, relay_id, source, regions_json, read, write, enabled, auth_type, auth_secret, failures, last_ok_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0, NULL, ?, ?)
         ON CONFLICT(url) DO UPDATE SET
           relay_id = excluded.relay_id, source = excluded.source, regions_json = excluded.regions_json,
           read = excluded.read, write = excluded.write, enabled = excluded.enabled,
           auth_type = excluded.auth_type, auth_secret = excluded.auth_secret, updated_at = excluded.updated_at"
    } else {
        "INSERT INTO msg_relays (url, relay_id, source, regions_json, read, write, enabled, auth_type, auth_secret, failures, last_ok_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0, NULL, ?, ?)
         ON CONFLICT(url) DO UPDATE SET
           relay_id = excluded.relay_id, source = excluded.source, regions_json = excluded.regions_json,
           read = excluded.read, write = excluded.write, enabled = msg_relays.enabled,
           auth_type = excluded.auth_type, auth_secret = excluded.auth_secret, updated_at = excluded.updated_at"
    };
    sqlx::query(sql)
        .bind(&r.url)
        .bind(&r.relay_id)
        .bind(&r.source)
        .bind(regions)
        .bind(r.read)
        .bind(r.write)
        .bind(r.enabled)
        .bind(&r.auth_type)
        .bind(&r.auth_secret)
        .bind(now)
        .bind(now)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn delete(store: &Store, url: &str) -> Result<()> {
    sqlx::query("DELETE FROM msg_relays WHERE url = ?")
        .bind(url)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn set_enabled(store: &Store, url: &str, enabled: bool) -> Result<()> {
    sqlx::query("UPDATE msg_relays SET enabled = ?, updated_at = ? WHERE url = ?")
        .bind(enabled)
        .bind(crate::now())
        .bind(url)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// Rename a relay address in `msg_relays` and every referencing table, in
/// one transaction. Used when a manifest keeps a `relay_id` but changes its
/// url.
pub async fn migrate_url(store: &Store, old: &str, new: &str) -> Result<()> {
    let mut tx = store.pool().begin().await.map_err(storage)?;
    // If the new url already exists as its own row, drop the old row instead of colliding.
    let exists = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM msg_relays WHERE url = ?")
        .bind(new)
        .fetch_one(&mut *tx)
        .await
        .map_err(storage)?;
    if exists > 0 {
        sqlx::query("DELETE FROM msg_relays WHERE url = ?")
            .bind(old)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
    } else {
        sqlx::query("UPDATE msg_relays SET url = ?, updated_at = ? WHERE url = ?")
            .bind(new)
            .bind(crate::now())
            .bind(old)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
    }
    for (table, col) in RELAY_URL_REFS {
        sqlx::query(AssertSqlSafe(format!("UPDATE OR IGNORE {table} SET {col} = ? WHERE {col} = ?")))
            .bind(new)
            .bind(old)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        sqlx::query(AssertSqlSafe(format!("DELETE FROM {table} WHERE {col} = ?")))
            .bind(old)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
    }
    tx.commit().await.map_err(storage)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn up(url: &str, id: Option<&str>, source: &str) -> RelayUpsert {
        RelayUpsert {
            url: url.into(),
            relay_id: id.map(String::from),
            source: source.into(),
            regions: vec!["default".into()],
            read: true,
            write: true,
            enabled: true,
            auth_type: None,
            auth_secret: None,
        }
    }

    #[tokio::test]
    async fn upsert_preserves_enabled_unless_told() {
        let s = Store::open_in_memory().await.unwrap();
        upsert(&s, &up("wss://a.example", Some("a"), SOURCE_MANIFEST), true).await.unwrap();
        set_enabled(&s, "wss://a.example", false).await.unwrap();
        upsert(&s, &up("wss://a.example", Some("a"), SOURCE_MANIFEST), false).await.unwrap();
        assert!(!get(&s, "wss://a.example").await.unwrap().unwrap().enabled);
        upsert(&s, &up("wss://a.example", Some("a"), SOURCE_MANIFEST), true).await.unwrap();
        assert!(get(&s, "wss://a.example").await.unwrap().unwrap().enabled);
        assert_eq!(list(&s).await.unwrap().len(), 1);
        assert_eq!(list(&s).await.unwrap()[0].regions(), vec!["default".to_string()]);
    }

    #[tokio::test]
    async fn migrate_url_renames_and_handles_collision() {
        let s = Store::open_in_memory().await.unwrap();
        upsert(&s, &up("wss://old.example", Some("x"), SOURCE_MANIFEST), true).await.unwrap();
        migrate_url(&s, "wss://old.example", "wss://new.example").await.unwrap();
        let rows = list(&s).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].url, "wss://new.example");
        assert_eq!(rows[0].relay_id.as_deref(), Some("x"));

        upsert(&s, &up("wss://user.example", None, SOURCE_USER), true).await.unwrap();
        migrate_url(&s, "wss://new.example", "wss://user.example").await.unwrap();
        let rows = list(&s).await.unwrap();
        assert_eq!(rows.len(), 1, "collision drops the old row");
        assert_eq!(rows[0].url, "wss://user.example");
    }

    #[tokio::test]
    async fn relay_id_is_unique_among_manifest_rows() {
        let s = Store::open_in_memory().await.unwrap();
        upsert(&s, &up("wss://a.example", Some("same"), SOURCE_MANIFEST), true).await.unwrap();
        assert!(upsert(&s, &up("wss://b.example", Some("same"), SOURCE_MANIFEST), true).await.is_err());
        upsert(&s, &up("wss://u1.example", None, SOURCE_USER), true).await.unwrap();
        upsert(&s, &up("wss://u2.example", None, SOURCE_USER), true).await.unwrap();
        assert_eq!(list(&s).await.unwrap().len(), 3);
        delete(&s, "wss://u1.example").await.unwrap();
        assert_eq!(list(&s).await.unwrap().len(), 2);
    }
}
