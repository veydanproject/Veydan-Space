// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The tickets of the moderators (internal/communities-wire.md §17): those
//! the keeper kept here minted (`cm_keeper_tickets`) and those I hold as a
//! moderator (`cm_tickets`).

use super::{storage, Db};
use messenger_core::Result;

pub const MINTED: &str = "minted";
pub const USED: &str = "used";
pub const REVOKED: &str = "revoked";
pub const HELD: &str = "held";

/// A ticket the keeper minted.
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct MintedRow {
    pub community_id: String,
    pub ticket_id: String,
    pub moderator: String,
    pub member: String,
    pub expires_at: i64,
    pub status: String,
    pub used_by: Option<String>,
    pub created_at: i64,
}

// The columns are a constant: `AssertSqlSafe` below holds for it.
const MINTED_COLS: &str = "community_id, ticket_id, moderator, member, expires_at, status, used_by, created_at";

pub async fn put_minted(db: &Db, row: &MintedRow) -> Result<()> {
    sqlx::query(
        "INSERT OR REPLACE INTO cm_keeper_tickets (community_id, ticket_id, moderator, member, expires_at, status, used_by, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&row.community_id)
    .bind(&row.ticket_id)
    .bind(&row.moderator)
    .bind(&row.member)
    .bind(row.expires_at)
    .bind(&row.status)
    .bind(&row.used_by)
    .bind(row.created_at)
    .execute(db.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

pub async fn minted(db: &Db, community_id: &str, ticket_id: &str) -> Result<Option<MintedRow>> {
    sqlx::query_as::<_, MintedRow>(sqlx::AssertSqlSafe(format!("SELECT {MINTED_COLS} FROM cm_keeper_tickets WHERE community_id = ? AND ticket_id = ?")))
        .bind(community_id)
        .bind(ticket_id)
        .fetch_optional(db.pool())
        .await
        .map_err(storage)
}

/// The tickets not used nor revoked: of one moderator (`Some`) or of all.
pub async fn open(db: &Db, community_id: &str, moderator: Option<&str>) -> Result<Vec<MintedRow>> {
    sqlx::query_as::<_, MintedRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {MINTED_COLS} FROM cm_keeper_tickets WHERE community_id = ? AND status = 'minted' AND (? IS NULL OR moderator = ?) ORDER BY created_at, ticket_id"
    )))
    .bind(community_id)
    .bind(moderator)
    .bind(moderator)
    .fetch_all(db.pool())
    .await
    .map_err(storage)
}

pub async fn set_minted_status(db: &Db, community_id: &str, ticket_id: &str, status: &str, used_by: Option<&str>) -> Result<()> {
    sqlx::query("UPDATE cm_keeper_tickets SET status = ?, used_by = COALESCE(?, used_by) WHERE community_id = ? AND ticket_id = ?")
        .bind(status)
        .bind(used_by)
        .bind(community_id)
        .bind(ticket_id)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// A ticket I hold as a moderator.
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct HeldRow {
    pub community_id: String,
    pub ticket_id: String,
    pub issued: String,
    pub expires_at: i64,
    pub status: String,
    pub used_by: Option<String>,
    pub received_at: i64,
}

/// Keeps a ticket that came; one held already stays as it is.
pub async fn put_held(db: &Db, row: &HeldRow) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO cm_tickets (community_id, ticket_id, issued, expires_at, status, used_by, received_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
        .bind(&row.community_id)
        .bind(&row.ticket_id)
        .bind(&row.issued)
        .bind(row.expires_at)
        .bind(&row.status)
        .bind(&row.used_by)
        .bind(row.received_at)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// How many tickets I hold that are good until after `until`.
pub async fn held_count(db: &Db, community_id: &str, until: i64) -> Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM cm_tickets WHERE community_id = ? AND status = 'held' AND expires_at > ?")
        .bind(community_id)
        .bind(until)
        .fetch_one(db.pool())
        .await
        .map_err(storage)
}

/// The ticket that expires first among those good until after `until`,
/// marked used by `who`; none when I hold none.
pub async fn use_one(db: &Db, community_id: &str, until: i64, who: &str) -> Result<Option<HeldRow>> {
    let row = sqlx::query_as::<_, HeldRow>(
        "SELECT community_id, ticket_id, issued, expires_at, status, used_by, received_at FROM cm_tickets
         WHERE community_id = ? AND status = 'held' AND expires_at > ? ORDER BY expires_at, ticket_id LIMIT 1",
    )
    .bind(community_id)
    .bind(until)
    .fetch_optional(db.pool())
    .await
    .map_err(storage)?;
    let Some(row) = row else { return Ok(None) };
    sqlx::query("UPDATE cm_tickets SET status = 'used', used_by = ? WHERE community_id = ? AND ticket_id = ?")
        .bind(who)
        .bind(community_id)
        .bind(&row.ticket_id)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(Some(row))
}

/// Drops the tickets I hold that expired before `before`.
pub async fn drop_expired_held(db: &Db, community_id: &str, before: i64) -> Result<()> {
    sqlx::query("DELETE FROM cm_tickets WHERE community_id = ? AND expires_at < ?").bind(community_id).bind(before).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}
