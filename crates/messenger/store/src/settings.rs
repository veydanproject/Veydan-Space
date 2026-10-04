// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Key/value settings private to the messenger (`msg_settings`).
//! Keys are `<area>.<name>`, e.g. `relays.silent_mode`.

use crate::{storage, Store};
use messenger_core::Result;

pub async fn get(store: &Store, key: &str) -> Result<Option<String>> {
    sqlx::query_scalar::<_, String>("SELECT value FROM msg_settings WHERE key = ?")
        .bind(key)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)
}

pub async fn set(store: &Store, key: &str, value: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO msg_settings (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

pub async fn delete(store: &Store, key: &str) -> Result<()> {
    sqlx::query("DELETE FROM msg_settings WHERE key = ?")
        .bind(key)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn get_bool(store: &Store, key: &str, default: bool) -> Result<bool> {
    Ok(match get(store, key).await?.as_deref() {
        Some("1") | Some("true") => true,
        Some("0") | Some("false") => false,
        _ => default,
    })
}

pub async fn set_bool(store: &Store, key: &str, value: bool) -> Result<()> {
    set(store, key, if value { "1" } else { "0" }).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn set_get_delete_roundtrip() {
        let store = Store::open_in_memory().await.unwrap();
        assert_eq!(get(&store, "a.b").await.unwrap(), None);
        set(&store, "a.b", "1").await.unwrap();
        assert_eq!(get(&store, "a.b").await.unwrap().as_deref(), Some("1"));
        set(&store, "a.b", "2").await.unwrap();
        assert_eq!(get(&store, "a.b").await.unwrap().as_deref(), Some("2"));
        assert!(!get_bool(&store, "a.b", false).await.unwrap());
        set_bool(&store, "a.flag", true).await.unwrap();
        assert!(get_bool(&store, "a.flag", false).await.unwrap());
        assert!(get_bool(&store, "missing", true).await.unwrap());
        delete(&store, "a.b").await.unwrap();
        assert_eq!(get(&store, "a.b").await.unwrap(), None);
    }
}
