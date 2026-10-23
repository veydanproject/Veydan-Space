// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `cm_scopes`, `cm_levels`, `cm_meta_owed`: the sealed names of the levels
//! and titles of the scopes of a channel (internal/communities-wire.md
//! §14.1), one row per operation that named one; which row names a level
//! or a scope now is the journal's (`ChannelService::names`).

use super::{storage, Db};
use messenger_core::Result;

/// A name (or a title) as its author sealed it: of the level `id` or the
/// scope `id`, made by the operation `op_id`; `look`: the JSON of the look
/// of a scope (`wire::ScopeLook`), `{}` for a level.
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct NameRow {
    pub id: String,
    pub op_id: String,
    pub signer: String,
    pub name: String,
    pub at: i64,
    pub look: String,
}

/// Keeps the title and the look of a scope named by an operation; `false`
/// when it is here already.
#[allow(clippy::too_many_arguments)]
pub async fn put_scope_title(db: &Db, community_id: &str, scope_id: &str, op_id: &str, signer: &str, title: &str, look_json: &str, at: i64) -> Result<bool> {
    let res = sqlx::query("INSERT OR IGNORE INTO cm_scopes (community_id, scope_id, op_id, signer, title, look_json, at) VALUES (?, ?, ?, ?, ?, ?, ?)")
        .bind(community_id)
        .bind(scope_id)
        .bind(op_id)
        .bind(signer)
        .bind(title)
        .bind(look_json)
        .bind(at)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(res.rows_affected() == 1)
}

/// Keeps the name of a level named by an operation; `false` when it is
/// here already.
pub async fn put_level_name(db: &Db, community_id: &str, level: u8, op_id: &str, signer: &str, name: &str, at: i64) -> Result<bool> {
    let res = sqlx::query("INSERT OR IGNORE INTO cm_levels (community_id, level_id, op_id, signer, name, at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(community_id)
        .bind(i64::from(level))
        .bind(op_id)
        .bind(signer)
        .bind(name)
        .bind(at)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(res.rows_affected() == 1)
}

/// Every title of a scope of a channel kept here.
pub async fn scope_titles(db: &Db, community_id: &str) -> Result<Vec<NameRow>> {
    sqlx::query_as::<_, NameRow>("SELECT scope_id AS id, op_id, signer, title AS name, at, look_json AS look FROM cm_scopes WHERE community_id = ?")
        .bind(community_id)
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

/// Every name of a level of a channel kept here.
pub async fn level_names(db: &Db, community_id: &str) -> Result<Vec<NameRow>> {
    sqlx::query_as::<_, NameRow>("SELECT CAST(level_id AS TEXT) AS id, op_id, signer, name, at, '{}' AS look FROM cm_levels WHERE community_id = ?")
        .bind(community_id)
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

/// A meta of mine that waits for a key of its channel: of `everyone`, or of
/// the hidden section it is sealed in (`scope_id`).
pub async fn owe_meta(db: &Db, community_id: &str, op_id: &str, meta_json: &str, scope_id: &str) -> Result<()> {
    sqlx::query("INSERT OR REPLACE INTO cm_meta_owed (community_id, op_id, meta_json, scope_id) VALUES (?, ?, ?, ?)")
        .bind(community_id)
        .bind(op_id)
        .bind(meta_json)
        .bind(scope_id)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// A meta of mine that waits, and the feed it goes into.
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct OwedMeta {
    pub community_id: String,
    pub op_id: String,
    pub meta_json: String,
    pub scope_id: String,
}

/// Every meta of mine that waits.
pub async fn metas_owed(db: &Db) -> Result<Vec<OwedMeta>> {
    sqlx::query_as::<_, OwedMeta>("SELECT community_id, op_id, meta_json, scope_id FROM cm_meta_owed ORDER BY community_id, op_id")
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

/// A meta that went out (or will never go).
pub async fn meta_paid(db: &Db, community_id: &str, op_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM cm_meta_owed WHERE community_id = ? AND op_id = ?").bind(community_id).bind(op_id).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}

// ─── Requests for a level ───────────────────────────────────────────────────

pub const REQ_PENDING: &str = "pending";
pub const REQ_GRANTED: &str = "granted";
pub const REQ_REFUSED: &str = "refused";

/// A request for a level, one that came to me (`in`) or mine (`out`).
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct LevelRequestRow {
    pub community_id: String,
    pub requester: String,
    pub level: i64,
    pub direction: String,
    pub status: String,
    pub note: String,
    pub reason: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Keeps a request, or a new one in place of the one before it (the
/// same person asking again for the same level).
pub async fn put_level_request(db: &Db, r: &LevelRequestRow) -> Result<()> {
    sqlx::query(
        "INSERT INTO cm_level_requests (community_id, requester, level, direction, status, note, reason, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(community_id, requester, level, direction) DO UPDATE SET status = excluded.status, note = excluded.note,
           reason = excluded.reason, created_at = excluded.created_at, updated_at = excluded.updated_at",
    )
    .bind(&r.community_id)
    .bind(&r.requester)
    .bind(r.level)
    .bind(&r.direction)
    .bind(&r.status)
    .bind(&r.note)
    .bind(&r.reason)
    .bind(r.created_at)
    .bind(r.updated_at)
    .execute(db.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// The requests of a channel in one direction and status, oldest first.
pub async fn level_requests(db: &Db, community_id: &str, direction: &str, status: &str) -> Result<Vec<LevelRequestRow>> {
    sqlx::query_as::<_, LevelRequestRow>(
        "SELECT community_id, requester, level, direction, status, note, reason, created_at, updated_at FROM cm_level_requests
         WHERE community_id = ? AND direction = ? AND status = ? ORDER BY created_at, requester, level",
    )
    .bind(community_id)
    .bind(direction)
    .bind(status)
    .fetch_all(db.pool())
    .await
    .map_err(storage)
}

/// What became of a request; `false` when there is no such request (its
/// key is four columns, hence the arguments).
#[allow(clippy::too_many_arguments)]
pub async fn set_level_request(db: &Db, community_id: &str, requester: &str, level: u8, direction: &str, status: &str, reason: Option<&str>, at: i64) -> Result<bool> {
    let res = sqlx::query("UPDATE cm_level_requests SET status = ?, reason = ?, updated_at = ? WHERE community_id = ? AND requester = ? AND level = ? AND direction = ?")
        .bind(status)
        .bind(reason)
        .bind(at)
        .bind(community_id)
        .bind(requester)
        .bind(i64::from(level))
        .bind(direction)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(res.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn names_are_kept_once_per_operation() {
        let db = Db::open_in_memory().await.unwrap();
        let cid = "aa".repeat(32);
        assert!(put_level_name(&db, &cid, 2, &"01".repeat(32), "s", "Gold", 10).await.unwrap());
        assert!(!put_level_name(&db, &cid, 2, &"01".repeat(32), "s", "Other", 11).await.unwrap(), "the same operation names once");
        assert!(put_level_name(&db, &cid, 2, &"02".repeat(32), "s", "Platinum", 12).await.unwrap());
        let names = level_names(&db, &cid).await.unwrap();
        assert_eq!(names.len(), 2);
        assert!(names.iter().all(|n| n.id == "2"));
        assert!(put_scope_title(&db, &cid, &"cd".repeat(16), &"03".repeat(32), "s", "Premium", r#"{"color":3}"#, 13).await.unwrap());
        let titles = scope_titles(&db, &cid).await.unwrap();
        assert_eq!((titles[0].name.as_str(), titles[0].look.as_str()), ("Premium", r#"{"color":3}"#));
        owe_meta(&db, &cid, &"03".repeat(32), "{}", &"cd".repeat(16)).await.unwrap();
        assert_eq!(metas_owed(&db).await.unwrap().iter().map(|m| m.scope_id.clone()).collect::<Vec<_>>(), ["cd".repeat(16)]);
        meta_paid(&db, &cid, &"03".repeat(32)).await.unwrap();
        assert!(metas_owed(&db).await.unwrap().is_empty());
    }
}
