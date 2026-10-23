// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The keys of the staff in the owner's Veydan Cloud
//! (internal/communities-wire.md §9.2): those the keeper kept here registered
//! (`cm_keeper_cloud_keys`) and the one given to me per channel
//! (`cm_cloud_keys`).

use super::{storage, Db};
use messenger_core::Result;

/// A key of the channel the keeper registered for a member.
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct MadeRow {
    pub community_id: String,
    pub who: String,
    pub pubkey: String,
    pub owner: String,
    pub expires_at: i64,
    /// 1 once its secret went to the member.
    pub sent: i64,
}

pub async fn put_made(db: &Db, row: &MadeRow) -> Result<()> {
    sqlx::query("INSERT OR REPLACE INTO cm_keeper_cloud_keys (community_id, who, pubkey, owner, expires_at, sent) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(&row.community_id)
        .bind(&row.who)
        .bind(&row.pubkey)
        .bind(&row.owner)
        .bind(row.expires_at)
        .bind(row.sent)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn made(db: &Db, community_id: &str) -> Result<Vec<MadeRow>> {
    sqlx::query_as::<_, MadeRow>("SELECT community_id, who, pubkey, owner, expires_at, sent FROM cm_keeper_cloud_keys WHERE community_id = ? ORDER BY who")
        .bind(community_id)
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

/// The secret of `who`'s key is to go again (a new device of his).
pub async fn unsend(db: &Db, community_id: &str, who: &str) -> Result<()> {
    sqlx::query("UPDATE cm_keeper_cloud_keys SET sent = 0 WHERE community_id = ? AND who = ?")
        .bind(community_id)
        .bind(who)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// The key of `who` revoked on the Cloud: gone.
pub async fn drop_made(db: &Db, community_id: &str, who: &str) -> Result<()> {
    sqlx::query("DELETE FROM cm_keeper_cloud_keys WHERE community_id = ? AND who = ?").bind(community_id).bind(who).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}

/// The key of a channel given to me (its secret is in the secret store).
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct GivenRow {
    pub community_id: String,
    pub pubkey: String,
    pub owner: String,
    pub expires_at: i64,
    pub received_at: i64,
}

pub async fn put_given(db: &Db, row: &GivenRow) -> Result<()> {
    sqlx::query("INSERT OR REPLACE INTO cm_cloud_keys (community_id, pubkey, owner, expires_at, received_at) VALUES (?, ?, ?, ?, ?)")
        .bind(&row.community_id)
        .bind(&row.pubkey)
        .bind(&row.owner)
        .bind(row.expires_at)
        .bind(row.received_at)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// Every key given to me.
pub async fn given(db: &Db) -> Result<Vec<GivenRow>> {
    sqlx::query_as::<_, GivenRow>("SELECT community_id, pubkey, owner, expires_at, received_at FROM cm_cloud_keys ORDER BY community_id")
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

/// The key of a channel given to me: gone.
pub async fn drop_given(db: &Db, community_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM cm_cloud_keys WHERE community_id = ?").bind(community_id).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}
