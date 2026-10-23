// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The handover of a channel to a new owner as this device knows it
//! (internal/communities-wire.md §16): the offer that came to me and the
//! one I made, one per channel and side.

use super::{storage, Db};
use messenger_core::Result;

pub const IN: &str = "in";
pub const OUT: &str = "out";
pub const OFFERED: &str = "offered";
pub const ACCEPTED: &str = "accepted";
pub const DECLINED: &str = "declined";

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct TransferRow {
    pub community_id: String,
    pub direction: String,
    pub peer: String,
    pub status: String,
    pub payload_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

pub async fn put(db: &Db, row: &TransferRow) -> Result<()> {
    sqlx::query(
        "INSERT OR REPLACE INTO cm_transfers (community_id, direction, peer, status, payload_json, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&row.community_id)
    .bind(&row.direction)
    .bind(&row.peer)
    .bind(&row.status)
    .bind(&row.payload_json)
    .bind(row.created_at)
    .bind(row.updated_at)
    .execute(db.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

pub async fn get(db: &Db, community_id: &str, direction: &str) -> Result<Option<TransferRow>> {
    sqlx::query_as::<_, TransferRow>(
        "SELECT community_id, direction, peer, status, payload_json, created_at, updated_at FROM cm_transfers WHERE community_id = ? AND direction = ?",
    )
    .bind(community_id)
    .bind(direction)
    .fetch_optional(db.pool())
    .await
    .map_err(storage)
}

pub async fn set_status(db: &Db, community_id: &str, direction: &str, status: &str, now: i64) -> Result<()> {
    sqlx::query("UPDATE cm_transfers SET status = ?, updated_at = ? WHERE community_id = ? AND direction = ?")
        .bind(status)
        .bind(now)
        .bind(community_id)
        .bind(direction)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}
