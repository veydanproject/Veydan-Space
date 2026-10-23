// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The upload keys of a channel's own server (internal/communities-wire.md
//! §9.1): those the keeper kept here made (`cm_keeper_upload_keys`) and
//! those given to me (`cm_upload_keys`).

use super::{storage, Db};
use messenger_core::Result;

/// A key the keeper made for a member on a host.
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct MadeRow {
    pub community_id: String,
    pub host: String,
    pub who: String,
    pub iat: i64,
    pub expires_at: i64,
}

pub async fn put_made(db: &Db, row: &MadeRow) -> Result<()> {
    sqlx::query("INSERT OR REPLACE INTO cm_keeper_upload_keys (community_id, host, who, iat, expires_at) VALUES (?, ?, ?, ?, ?)")
        .bind(&row.community_id)
        .bind(&row.host)
        .bind(&row.who)
        .bind(row.iat)
        .bind(row.expires_at)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn made(db: &Db, community_id: &str) -> Result<Vec<MadeRow>> {
    sqlx::query_as::<_, MadeRow>("SELECT community_id, host, who, iat, expires_at FROM cm_keeper_upload_keys WHERE community_id = ? ORDER BY host, who")
        .bind(community_id)
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

/// The key of `who` on `host` revoked (or of no use any more): gone.
pub async fn drop_made(db: &Db, community_id: &str, host: &str, who: &str) -> Result<()> {
    sqlx::query("DELETE FROM cm_keeper_upload_keys WHERE community_id = ? AND host = ? AND who = ?")
        .bind(community_id)
        .bind(host)
        .bind(who)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// A key given to me.
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct GivenRow {
    pub community_id: String,
    pub host: String,
    pub key: String,
    pub expires_at: i64,
    pub received_at: i64,
}

pub async fn put_given(db: &Db, row: &GivenRow) -> Result<()> {
    sqlx::query("INSERT OR REPLACE INTO cm_upload_keys (community_id, host, key, expires_at, received_at) VALUES (?, ?, ?, ?, ?)")
        .bind(&row.community_id)
        .bind(&row.host)
        .bind(&row.key)
        .bind(row.expires_at)
        .bind(row.received_at)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// Every key given to me, of every channel.
pub async fn given(db: &Db) -> Result<Vec<GivenRow>> {
    sqlx::query_as::<_, GivenRow>("SELECT community_id, host, key, expires_at, received_at FROM cm_upload_keys ORDER BY community_id, host")
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

/// The keys of a channel given to me: gone (I lost the role), or those
/// expired before `now` of every channel when `community_id` is `None`.
pub async fn drop_given(db: &Db, community_id: Option<&str>, now: i64) -> Result<()> {
    match community_id {
        Some(cid) => sqlx::query("DELETE FROM cm_upload_keys WHERE community_id = ?").bind(cid).execute(db.pool()).await,
        None => sqlx::query("DELETE FROM cm_upload_keys WHERE expires_at <= ?").bind(now).execute(db.pool()).await,
    }
    .map_err(storage)?;
    Ok(())
}
