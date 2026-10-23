// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from crates/messenger/store/src/group_avatar.rs at 1f2ef31d, then changed for channels.

//! `cm_avatar`: the pictures of the channels I manage, as this device
//! looks after them. One row per channel, or none. `copies_json` is the
//! runtime's own list of where the blob was put; the store keeps it as
//! given.

use super::{storage, Db};
use messenger_core::Result;

#[derive(Clone, Debug, Default, PartialEq, Eq, sqlx::FromRow)]
pub struct ChannelAvatar {
    pub community_id: String,
    /// Of the JPEG, lowercase hex; the file is `avatars/channel/<sha256>.jpg`.
    pub sha256: String,
    /// What the journal of the channel says as `picture`.
    pub url: String,
    /// The server of mine `url` is on; none for a server of somebody else.
    pub server_id: Option<String>,
    pub copies_json: String,
    pub set_at: i64,
    /// When its presence on the server was last checked.
    pub checked_at: i64,
    /// When it was last fetched to keep it alive.
    pub touched_at: i64,
}

const COLS: &str = "community_id, sha256, url, server_id, copies_json, set_at, checked_at, touched_at";

pub async fn get(db: &Db, community_id: &str) -> Result<Option<ChannelAvatar>> {
    sqlx::query_as::<_, ChannelAvatar>(sqlx::AssertSqlSafe(format!("SELECT {COLS} FROM cm_avatar WHERE community_id = ?")))
        .bind(community_id)
        .fetch_optional(db.pool())
        .await
        .map_err(storage)
}

pub async fn list(db: &Db) -> Result<Vec<ChannelAvatar>> {
    sqlx::query_as::<_, ChannelAvatar>(sqlx::AssertSqlSafe(format!("SELECT {COLS} FROM cm_avatar ORDER BY community_id")))
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

/// Replaces whatever the channel had.
pub async fn set(db: &Db, avatar: &ChannelAvatar) -> Result<()> {
    sqlx::query(
        "INSERT INTO cm_avatar (community_id, sha256, url, server_id, copies_json, set_at, checked_at, touched_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(community_id) DO UPDATE SET sha256 = excluded.sha256, url = excluded.url, server_id = excluded.server_id,
           copies_json = excluded.copies_json, set_at = excluded.set_at, checked_at = excluded.checked_at,
           touched_at = excluded.touched_at",
    )
    .bind(&avatar.community_id)
    .bind(&avatar.sha256)
    .bind(&avatar.url)
    .bind(&avatar.server_id)
    .bind(&avatar.copies_json)
    .bind(avatar.set_at)
    .bind(avatar.checked_at)
    .bind(avatar.touched_at)
    .execute(db.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

pub async fn clear(db: &Db, community_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM cm_avatar WHERE community_id = ?").bind(community_id).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}

/// The blob was found on its server at `at`.
pub async fn mark_checked(db: &Db, community_id: &str, at: i64) -> Result<()> {
    sqlx::query("UPDATE cm_avatar SET checked_at = ? WHERE community_id = ?")
        .bind(at)
        .bind(community_id)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// The blob was fetched at `at` to keep it alive.
pub async fn mark_touched(db: &Db, community_id: &str, at: i64) -> Result<()> {
    sqlx::query("UPDATE cm_avatar SET touched_at = ? WHERE community_id = ?")
        .bind(at)
        .bind(community_id)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rows_per_channel() {
        let s = Db::open_in_memory().await.unwrap();
        assert_eq!(get(&s, "g1").await.unwrap(), None);
        mark_checked(&s, "g1", 5).await.unwrap();
        assert_eq!(get(&s, "g1").await.unwrap(), None, "marks make no row");

        let a = ChannelAvatar {
            community_id: "g1".into(),
            sha256: "ab".repeat(32),
            url: format!("https://media.example/{}", "ab".repeat(32)),
            server_id: Some("s1".into()),
            copies_json: r#"[{"server_id":"s1"}]"#.into(),
            set_at: 100,
            ..Default::default()
        };
        let b = ChannelAvatar { community_id: "g2".into(), sha256: "cd".repeat(32), server_id: None, ..a.clone() };
        set(&s, &a).await.unwrap();
        set(&s, &b).await.unwrap();
        assert_eq!(list(&s).await.unwrap(), vec![a.clone(), b.clone()]);

        mark_checked(&s, "g1", 200).await.unwrap();
        mark_touched(&s, "g1", 300).await.unwrap();
        let got = get(&s, "g1").await.unwrap().unwrap();
        assert_eq!((got.checked_at, got.touched_at), (200, 300));
        assert_eq!(get(&s, "g2").await.unwrap(), Some(b.clone()), "the other channel is untouched");

        let c = ChannelAvatar { sha256: "ef".repeat(32), url: "https://other.example/x".into(), set_at: 400, ..a };
        set(&s, &c).await.unwrap();
        assert_eq!(get(&s, "g1").await.unwrap(), Some(c));

        clear(&s, "g1").await.unwrap();
        assert_eq!(get(&s, "g1").await.unwrap(), None);
        assert_eq!(list(&s).await.unwrap(), vec![b]);
    }
}
