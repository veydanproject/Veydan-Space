// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_own_avatar`: my avatar as uploaded. One row, or none while I have
//! no avatar. `copies_json` is the runtime's own list of where the blob
//! was put; the store keeps it as given.

use crate::{storage, Store};
use messenger_core::Result;

#[derive(Clone, Debug, Default, PartialEq, Eq, sqlx::FromRow)]
pub struct OwnAvatar {
    /// Of the JPEG, lowercase hex; the file is `avatars/own/<sha256>.jpg`.
    pub sha256: String,
    /// What kind 0 says as `picture`.
    pub url: String,
    /// The server `url` is on.
    pub server_id: Option<String>,
    pub copies_json: String,
    pub set_at: i64,
    /// When its presence on the servers was last checked.
    pub checked_at: i64,
    /// When it was last fetched to keep it alive.
    pub touched_at: i64,
}

const COLS: &str = "sha256, url, server_id, copies_json, set_at, checked_at, touched_at";

pub async fn get(store: &Store) -> Result<Option<OwnAvatar>> {
    sqlx::query_as::<_, OwnAvatar>(sqlx::AssertSqlSafe(format!("SELECT {COLS} FROM msg_own_avatar WHERE id = 1")))
        .fetch_optional(store.pool())
        .await
        .map_err(storage)
}

/// Replaces whatever was there.
pub async fn set(store: &Store, avatar: &OwnAvatar) -> Result<()> {
    sqlx::query(
        "INSERT INTO msg_own_avatar (id, sha256, url, server_id, copies_json, set_at, checked_at, touched_at)
         VALUES (1, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET sha256 = excluded.sha256, url = excluded.url, server_id = excluded.server_id,
           copies_json = excluded.copies_json, set_at = excluded.set_at, checked_at = excluded.checked_at,
           touched_at = excluded.touched_at",
    )
    .bind(&avatar.sha256)
    .bind(&avatar.url)
    .bind(&avatar.server_id)
    .bind(&avatar.copies_json)
    .bind(avatar.set_at)
    .bind(avatar.checked_at)
    .bind(avatar.touched_at)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

pub async fn clear(store: &Store) -> Result<()> {
    sqlx::query("DELETE FROM msg_own_avatar").execute(store.pool()).await.map_err(storage)?;
    Ok(())
}

/// The blob was found on its servers at `at`.
pub async fn mark_checked(store: &Store, at: i64) -> Result<()> {
    sqlx::query("UPDATE msg_own_avatar SET checked_at = ? WHERE id = 1")
        .bind(at)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// The blob was fetched at `at` to keep it alive.
pub async fn mark_touched(store: &Store, at: i64) -> Result<()> {
    sqlx::query("UPDATE msg_own_avatar SET touched_at = ? WHERE id = 1")
        .bind(at)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn set_replace_mark_and_clear() {
        let s = Store::open_in_memory().await.unwrap();
        assert_eq!(get(&s).await.unwrap(), None);
        mark_checked(&s, 5).await.unwrap();
        assert_eq!(get(&s).await.unwrap(), None, "marks make no row");

        let a = OwnAvatar {
            sha256: "ab".repeat(32),
            url: format!("https://media.example/{}", "ab".repeat(32)),
            server_id: Some("s1".into()),
            copies_json: r#"[{"server_id":"s1"}]"#.into(),
            set_at: 100,
            ..Default::default()
        };
        set(&s, &a).await.unwrap();
        assert_eq!(get(&s).await.unwrap(), Some(a.clone()));

        mark_checked(&s, 200).await.unwrap();
        mark_touched(&s, 300).await.unwrap();
        let got = get(&s).await.unwrap().unwrap();
        assert_eq!((got.checked_at, got.touched_at), (200, 300));

        let b = OwnAvatar { sha256: "cd".repeat(32), url: "https://other.example/x".into(), server_id: None, set_at: 400, ..a };
        set(&s, &b).await.unwrap();
        assert_eq!(get(&s).await.unwrap(), Some(b));

        clear(&s).await.unwrap();
        assert_eq!(get(&s).await.unwrap(), None);
    }
}
