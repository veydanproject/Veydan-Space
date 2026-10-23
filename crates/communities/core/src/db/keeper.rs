// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The keeper's tables (`002_keeper`): keydist's state as rows of its
//! change log, and what the cycle of publishing needs to go on after a
//! crash (internal/communities-spec.md §8.2, §8.3).
//!
//! The rows are written in one transaction before anything the call that
//! made them returned is sent (keydist's contract: save first). The
//! masters of the generations never reach a row: the header is stored
//! with zeros in their place and gets them back from the caller
//! (`masters`) when the state is put together.

use super::{storage, Db};
use keydist::keeper::MemberRec;
use keydist::{Change, EpochKey, Key32, KeeperHeader, MemberId};
use messenger_core::Result;
use std::collections::BTreeSet;

/// What stands in a stored header for a master.
const NO_MASTER: [u8; 32] = [0u8; 32];

/// The keeper row of a channel.
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct KeeperRow {
    pub community_id: String,
    pub keeper_pubkey: String,
    pub link_sent: Option<i64>,
    pub link_wait_epoch: Option<i64>,
    pub link_wait_since: Option<i64>,
    pub created_at: i64,
}

pub async fn put_keeper(db: &Db, community_id: &str, keeper_pubkey: &str, now: i64) -> Result<()> {
    sqlx::query(
        "INSERT INTO cm_keeper (community_id, keeper_pubkey, created_at) VALUES (?, ?, ?)
         ON CONFLICT(community_id) DO UPDATE SET keeper_pubkey = excluded.keeper_pubkey",
    )
    .bind(community_id)
    .bind(keeper_pubkey)
    .bind(now)
    .execute(db.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

pub async fn keeper(db: &Db, community_id: &str) -> Result<Option<KeeperRow>> {
    sqlx::query_as::<_, KeeperRow>(
        "SELECT community_id, keeper_pubkey, link_sent, link_wait_epoch, link_wait_since, created_at FROM cm_keeper WHERE community_id = ?",
    )
    .bind(community_id)
    .fetch_optional(db.pool())
    .await
    .map_err(storage)
}

/// The channels this device keeps.
pub async fn kept(db: &Db) -> Result<Vec<String>> {
    sqlx::query_scalar::<_, String>("SELECT community_id FROM cm_keeper ORDER BY community_id").fetch_all(db.pool()).await.map_err(storage)
}

pub async fn set_link_sent(db: &Db, community_id: &str, link_epoch: u32) -> Result<()> {
    sqlx::query("UPDATE cm_keeper SET link_sent = ? WHERE community_id = ?").bind(i64::from(link_epoch)).bind(community_id).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}

/// A `rotate_link` seen without its secret (`None`: the wait is over).
pub async fn set_link_wait(db: &Db, community_id: &str, wait: Option<(u32, i64)>) -> Result<()> {
    sqlx::query("UPDATE cm_keeper SET link_wait_epoch = ?, link_wait_since = ? WHERE community_id = ?")
        .bind(wait.map(|w| i64::from(w.0)))
        .bind(wait.map(|w| w.1))
        .bind(community_id)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

// ─── The state as rows ──────────────────────────────────────────────────────

fn header_json(h: &KeeperHeader) -> Result<String> {
    let mut h = h.clone();
    for g in &mut h.gens {
        g.master = keydist::GenerationMaster::from_bytes(NO_MASTER);
    }
    Ok(serde_json::to_string(&h)?)
}

fn member_hex(m: &MemberId) -> String {
    hex::encode(m.0)
}

fn member_of(hex_key: &str) -> Result<MemberId> {
    let raw: [u8; 32] = hex::decode(hex_key).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| storage("keeper member"))?;
    Ok(MemberId(raw))
}

/// Writes the rows of `changes` for one scope, in one transaction.
pub async fn write_changes(db: &Db, community_id: &str, scope_id: &str, changes: &[Change]) -> Result<()> {
    if changes.is_empty() {
        return Ok(());
    }
    let mut tx = db.write_tx().await?;
    for ch in changes {
        let q = match ch {
            Change::Header(h) => {
                let live: Vec<i64> = h.gens.iter().map(|g| i64::from(g.gen)).collect();
                // The revoked leaves of a generation the header no longer
                // names go with it (keydist drops them the same way).
                let rows = sqlx::query_as::<_, (i64,)>("SELECT DISTINCT gen FROM cm_keeper_revoked WHERE community_id = ? AND scope_id = ?")
                    .bind(community_id)
                    .bind(scope_id)
                    .fetch_all(&mut *tx)
                    .await
                    .map_err(storage)?;
                for (gen,) in rows.into_iter().filter(|(g,)| !live.contains(g)) {
                    sqlx::query("DELETE FROM cm_keeper_revoked WHERE community_id = ? AND scope_id = ? AND gen = ?")
                        .bind(community_id)
                        .bind(scope_id)
                        .bind(gen)
                        .execute(&mut *tx)
                        .await
                        .map_err(storage)?;
                }
                sqlx::query(
                    "INSERT INTO cm_keeper_header (community_id, scope_id, header_json) VALUES (?, ?, ?)
                     ON CONFLICT(community_id, scope_id) DO UPDATE SET header_json = excluded.header_json",
                )
                .bind(community_id)
                .bind(scope_id)
                .bind(header_json(h)?)
            }
            Change::MemberPut { member, rec } => sqlx::query(
                "INSERT INTO cm_keeper_members (community_id, scope_id, member, rec_json) VALUES (?, ?, ?, ?)
                 ON CONFLICT(community_id, scope_id, member) DO UPDATE SET rec_json = excluded.rec_json",
            )
            .bind(community_id)
            .bind(scope_id)
            .bind(member_hex(member))
            .bind(serde_json::to_string(rec)?),
            Change::MemberRemove { member } => sqlx::query("DELETE FROM cm_keeper_members WHERE community_id = ? AND scope_id = ? AND member = ?")
                .bind(community_id)
                .bind(scope_id)
                .bind(member_hex(member)),
            Change::Revoked { gen, leaf } => sqlx::query("INSERT OR IGNORE INTO cm_keeper_revoked (community_id, scope_id, gen, leaf) VALUES (?, ?, ?, ?)")
                .bind(community_id)
                .bind(scope_id)
                .bind(i64::from(*gen))
                .bind(i64::from(*leaf)),
            Change::Pending(r) => sqlx::query(
                "INSERT INTO cm_keeper_pending (community_id, scope_id, recipe_json) VALUES (?, ?, ?)
                 ON CONFLICT(community_id, scope_id) DO UPDATE SET recipe_json = excluded.recipe_json",
            )
            .bind(community_id)
            .bind(scope_id)
            .bind(serde_json::to_string(r)?),
            Change::PendingCleared => sqlx::query("DELETE FROM cm_keeper_pending WHERE community_id = ? AND scope_id = ?").bind(community_id).bind(scope_id),
            Change::Missed(k) => sqlx::query("INSERT OR REPLACE INTO cm_keeper_missed (community_id, scope_id, epoch, key_hex) VALUES (?, ?, ?, ?)")
                .bind(community_id)
                .bind(scope_id)
                .bind(i64::from(k.epoch))
                .bind(hex::encode(k.key.as_bytes())),
            Change::MissedCleared => sqlx::query("DELETE FROM cm_keeper_missed WHERE community_id = ? AND scope_id = ?").bind(community_id).bind(scope_id),
            Change::PublishedBlobs { hashes } => {
                sqlx::query("DELETE FROM cm_keeper_blobs WHERE community_id = ? AND scope_id = ?")
                    .bind(community_id)
                    .bind(scope_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(storage)?;
                for (pos, h) in hashes.iter().enumerate() {
                    sqlx::query("INSERT OR IGNORE INTO cm_keeper_blobs (community_id, scope_id, sha256, pos) VALUES (?, ?, ?, ?)")
                        .bind(community_id)
                        .bind(scope_id)
                        .bind(hex::encode(h))
                        .bind(i64::try_from(pos).unwrap_or(i64::MAX))
                        .execute(&mut *tx)
                        .await
                        .map_err(storage)?;
                }
                continue;
            }
        };
        q.execute(&mut *tx).await.map_err(storage)?;
    }
    tx.commit().await.map_err(storage)
}

/// Every row of a scope as keydist's change log, the header first, the
/// masters put back from `masters` (by generation). `None` when the scope has no
/// header (never kept here).
pub async fn read_changes(
    db: &Db,
    community_id: &str,
    scope_id: &str,
    masters: &std::collections::BTreeMap<u32, keydist::GenerationMaster>,
) -> Result<Option<Vec<Change>>> {
    let Some(json) = sqlx::query_scalar::<_, String>("SELECT header_json FROM cm_keeper_header WHERE community_id = ? AND scope_id = ?")
        .bind(community_id)
        .bind(scope_id)
        .fetch_optional(db.pool())
        .await
        .map_err(storage)?
    else {
        return Ok(None);
    };
    let mut header: KeeperHeader = serde_json::from_str(&json).map_err(storage)?;
    for g in &mut header.gens {
        g.master = masters.get(&g.gen).cloned().ok_or_else(|| messenger_core::MessengerError::SecretMissing(format!("master of generation {}", g.gen)))?;
    }
    let mut out = vec![Change::Header(header)];
    let members = sqlx::query_as::<_, (String, String)>("SELECT member, rec_json FROM cm_keeper_members WHERE community_id = ? AND scope_id = ? ORDER BY member")
        .bind(community_id)
        .bind(scope_id)
        .fetch_all(db.pool())
        .await
        .map_err(storage)?;
    for (m, rec) in members {
        let rec: MemberRec = serde_json::from_str(&rec).map_err(storage)?;
        out.push(Change::MemberPut { member: member_of(&m)?, rec });
    }
    let revoked = sqlx::query_as::<_, (i64, i64)>("SELECT gen, leaf FROM cm_keeper_revoked WHERE community_id = ? AND scope_id = ? ORDER BY gen, leaf")
        .bind(community_id)
        .bind(scope_id)
        .fetch_all(db.pool())
        .await
        .map_err(storage)?;
    for (gen, leaf) in revoked {
        out.push(Change::Revoked { gen: u32::try_from(gen).map_err(storage)?, leaf: u32::try_from(leaf).map_err(storage)? });
    }
    if let Some(r) = sqlx::query_scalar::<_, String>("SELECT recipe_json FROM cm_keeper_pending WHERE community_id = ? AND scope_id = ?")
        .bind(community_id)
        .bind(scope_id)
        .fetch_optional(db.pool())
        .await
        .map_err(storage)?
    {
        out.push(Change::Pending(serde_json::from_str(&r).map_err(storage)?));
    }
    let missed = sqlx::query_as::<_, (i64, String)>("SELECT epoch, key_hex FROM cm_keeper_missed WHERE community_id = ? AND scope_id = ? ORDER BY epoch")
        .bind(community_id)
        .bind(scope_id)
        .fetch_all(db.pool())
        .await
        .map_err(storage)?;
    for (epoch, k) in missed {
        let raw: [u8; 32] = hex::decode(&k).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| storage("missed key"))?;
        out.push(Change::Missed(EpochKey { epoch: u32::try_from(epoch).map_err(storage)?, key: Key32(raw) }));
    }
    let blobs = sqlx::query_scalar::<_, String>("SELECT sha256 FROM cm_keeper_blobs WHERE community_id = ? AND scope_id = ? ORDER BY pos")
        .bind(community_id)
        .bind(scope_id)
        .fetch_all(db.pool())
        .await
        .map_err(storage)?;
    let hashes = blobs.iter().map(|h| hex::decode(h).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| storage("blob hash"))).collect::<Result<Vec<[u8; 32]>>>()?;
    out.push(Change::PublishedBlobs { hashes });
    Ok(Some(out))
}

/// The members a scope's rows name (for a recovery when the rows do not
/// add up to a state).
pub async fn members(db: &Db, community_id: &str, scope_id: &str) -> Result<Vec<MemberId>> {
    let rows = sqlx::query_scalar::<_, String>("SELECT member FROM cm_keeper_members WHERE community_id = ? AND scope_id = ? ORDER BY member")
        .bind(community_id)
        .bind(scope_id)
        .fetch_all(db.pool())
        .await
        .map_err(storage)?;
    rows.iter().map(|m| member_of(m)).collect()
}

/// The newest generation and epoch number the rows of a scope ever named
/// (a recovery starts above both).
pub async fn high_marks(db: &Db, community_id: &str, scope_id: &str) -> Result<(u32, u32)> {
    let Some(json) = sqlx::query_scalar::<_, String>("SELECT header_json FROM cm_keeper_header WHERE community_id = ? AND scope_id = ?")
        .bind(community_id)
        .bind(scope_id)
        .fetch_optional(db.pool())
        .await
        .map_err(storage)?
    else {
        return Ok((0, 0));
    };
    let v: serde_json::Value = serde_json::from_str(&json).map_err(storage)?;
    let gen = v["gens"].as_array().and_then(|g| g.iter().filter_map(|g| g["gen"].as_u64()).max()).unwrap_or(0);
    let epoch = v["next_epoch"].as_u64().unwrap_or(1).saturating_sub(1);
    Ok((u32::try_from(gen).unwrap_or(u32::MAX), u32::try_from(epoch).unwrap_or(u32::MAX)))
}

/// Whether a tree of the scope was ever written here.
pub async fn has_state(db: &Db, community_id: &str, scope_id: &str) -> Result<bool> {
    let n = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM cm_keeper_header WHERE community_id = ? AND scope_id = ?")
        .bind(community_id)
        .bind(scope_id)
        .fetch_one(db.pool())
        .await
        .map_err(storage)?;
    Ok(n > 0)
}

/// The whole state of a flat scope (keydist `FlatState`, JSON).
pub async fn flat_state(db: &Db, community_id: &str, scope_id: &str) -> Result<Option<String>> {
    sqlx::query_scalar::<_, String>("SELECT state_json FROM cm_keeper_flat WHERE community_id = ? AND scope_id = ?")
        .bind(community_id)
        .bind(scope_id)
        .fetch_optional(db.pool())
        .await
        .map_err(storage)
}

/// Saves the whole state of a flat scope.
pub async fn put_flat(db: &Db, community_id: &str, scope_id: &str, state_json: &str) -> Result<()> {
    sqlx::query("INSERT OR REPLACE INTO cm_keeper_flat (community_id, scope_id, state_json) VALUES (?, ?, ?)")
        .bind(community_id)
        .bind(scope_id)
        .bind(state_json)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// Forgets a flat scope (deleted by the journal).
pub async fn delete_flat(db: &Db, community_id: &str, scope_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM cm_keeper_flat WHERE community_id = ? AND scope_id = ?").bind(community_id).bind(scope_id).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}

/// Drops every row of a scope's state (before the rows of a recovery).
pub async fn clear_state(db: &Db, community_id: &str, scope_id: &str) -> Result<()> {
    let mut tx = db.write_tx().await?;
    for t in ["cm_keeper_header", "cm_keeper_members", "cm_keeper_revoked", "cm_keeper_pending", "cm_keeper_missed", "cm_keeper_blobs", "cm_keeper_uploads", "cm_keeper_events"] {
        sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {t} WHERE community_id = ? AND scope_id = ?")))
            .bind(community_id)
            .bind(scope_id)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
    }
    tx.commit().await.map_err(storage)
}

/// Forgets everything of a channel this device kept.
pub async fn delete_keeper(db: &Db, community_id: &str) -> Result<()> {
    let mut tx = db.write_tx().await?;
    for t in [
        "cm_keeper_header",
        "cm_keeper_members",
        "cm_keeper_revoked",
        "cm_keeper_pending",
        "cm_keeper_missed",
        "cm_keeper_blobs",
        "cm_keeper_uploads",
        "cm_keeper_events",
        "cm_keeper_retired",
        "cm_keeper_invites",
        "cm_keeper_ops",
        "cm_keeper_readers",
        "cm_keeper_snapshot",
        "cm_keeper_held_joins",
        "cm_keeper_wraps",
        "cm_keeper_inbox",
        "cm_keeper_flat",
        "cm_keeper_tickets",
        "cm_keeper",
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {t} WHERE community_id = ?"))).bind(community_id).execute(&mut *tx).await.map_err(storage)?;
    }
    // Its notes not sent yet: nobody's to wrap once its keys are gone.
    sqlx::query("DELETE FROM cm_outbox WHERE community_id = ? AND lane = ? AND signer = ?")
        .bind(community_id)
        .bind(super::outbox::LANE_INBOX)
        .bind(super::outbox::SIGNER_KEEPER)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    tx.commit().await.map_err(storage)
}

// ─── The cycle ──────────────────────────────────────────────────────────────

/// `(host, sha256)` of the blobs of `epoch` each host holds.
pub async fn uploads(db: &Db, community_id: &str, scope_id: &str, epoch: u32) -> Result<BTreeSet<(String, String)>> {
    let rows = sqlx::query_as::<_, (String, String)>("SELECT host, sha256 FROM cm_keeper_uploads WHERE community_id = ? AND scope_id = ? AND epoch = ?")
        .bind(community_id)
        .bind(scope_id)
        .bind(i64::from(epoch))
        .fetch_all(db.pool())
        .await
        .map_err(storage)?;
    Ok(rows.into_iter().collect())
}

pub async fn add_upload(db: &Db, community_id: &str, scope_id: &str, epoch: u32, host: &str, sha256: &str) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO cm_keeper_uploads (community_id, scope_id, epoch, host, sha256) VALUES (?, ?, ?, ?, ?)")
        .bind(community_id)
        .bind(scope_id)
        .bind(i64::from(epoch))
        .bind(host)
        .bind(sha256)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// Forgets the uploads of every epoch but `keep`.
pub async fn drop_uploads_except(db: &Db, community_id: &str, scope_id: &str, keep: Option<u32>) -> Result<()> {
    sqlx::query("DELETE FROM cm_keeper_uploads WHERE community_id = ? AND scope_id = ? AND epoch != ?")
        .bind(community_id)
        .bind(scope_id)
        .bind(keep.map_or(-1, i64::from))
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn event(db: &Db, community_id: &str, scope_id: &str, epoch: u32) -> Result<Option<String>> {
    sqlx::query_scalar::<_, String>("SELECT event_json FROM cm_keeper_events WHERE community_id = ? AND scope_id = ? AND epoch = ?")
        .bind(community_id)
        .bind(scope_id)
        .bind(i64::from(epoch))
        .fetch_optional(db.pool())
        .await
        .map_err(storage)
}

pub async fn put_event(db: &Db, community_id: &str, scope_id: &str, epoch: u32, event_json: &str) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO cm_keeper_events (community_id, scope_id, epoch, event_json) VALUES (?, ?, ?, ?)")
        .bind(community_id)
        .bind(scope_id)
        .bind(i64::from(epoch))
        .bind(event_json)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// The signed event of an epoch kept (`put_event`) and queued for the
/// relays (`db::outbox`, the keeper lane) in one transaction: a row of
/// the event with no row of the outbox means a relay took it, after a
/// restart too.
pub async fn put_event_queued(db: &Db, community_id: &str, scope_id: &str, epoch: u32, event_id: &str, event_json: &str, now: i64) -> Result<()> {
    let mut tx = db.write_tx().await?;
    sqlx::query("INSERT OR IGNORE INTO cm_keeper_events (community_id, scope_id, epoch, event_json) VALUES (?, ?, ?, ?)")
        .bind(community_id)
        .bind(scope_id)
        .bind(i64::from(epoch))
        .bind(event_json)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    sqlx::query("INSERT OR IGNORE INTO cm_outbox (community_id, lane, kind, event_id, signer, payload, next_at, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(community_id)
        .bind(super::outbox::LANE_KEEPER)
        .bind(i64::from(crate::wire::KIND_EPOCH))
        .bind(event_id)
        .bind(super::outbox::SIGNER_ME)
        .bind(event_json)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    tx.commit().await.map_err(storage)
}

/// Every signed event a keeper kept of a channel: of the series before,
/// whose key is not this keeper's (wire §15).
pub async fn drop_events_all(db: &Db, community_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM cm_keeper_events WHERE community_id = ?").bind(community_id).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}

pub async fn drop_events(db: &Db, community_id: &str, scope_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM cm_keeper_events WHERE community_id = ? AND scope_id = ?").bind(community_id).bind(scope_id).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}

// ─── The snapshot of the readers ────────────────────────────────────────────

/// A reader let in now; one let in before keeps its first date.
pub async fn reader_in(db: &Db, community_id: &str, pubkey: &str, since: i64) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO cm_keeper_readers (community_id, pubkey, since) VALUES (?, ?, ?)")
        .bind(community_id)
        .bind(pubkey)
        .bind(since)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// A reader put out: a later join is a new date.
pub async fn reader_out(db: &Db, community_id: &str, pubkey: &str) -> Result<()> {
    sqlx::query("DELETE FROM cm_keeper_readers WHERE community_id = ? AND pubkey = ?").bind(community_id).bind(pubkey).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}

/// When one reader was let in, by hex key; none when out.
pub async fn reader_since(db: &Db, community_id: &str, pubkey: &str) -> Result<Option<i64>> {
    sqlx::query_scalar::<_, i64>("SELECT since FROM cm_keeper_readers WHERE community_id = ? AND pubkey = ?").bind(community_id).bind(pubkey).fetch_optional(db.pool()).await.map_err(storage)
}

/// When each reader was let in, by hex key.
pub async fn readers_since(db: &Db, community_id: &str) -> Result<std::collections::HashMap<String, i64>> {
    let rows = sqlx::query_as::<_, (String, i64)>("SELECT pubkey, since FROM cm_keeper_readers WHERE community_id = ?").bind(community_id).fetch_all(db.pool()).await.map_err(storage)?;
    Ok(rows.into_iter().collect())
}

/// The last snapshot a keeper published.
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct SnapshotRow {
    pub community_id: String,
    pub at: i64,
    pub digest: String,
    pub shards_json: String,
    pub hosts_json: String,
    pub prev_shards_json: String,
}

pub async fn snapshot(db: &Db, community_id: &str) -> Result<Option<SnapshotRow>> {
    sqlx::query_as::<_, SnapshotRow>("SELECT community_id, at, digest, shards_json, hosts_json, prev_shards_json FROM cm_keeper_snapshot WHERE community_id = ?")
        .bind(community_id)
        .fetch_optional(db.pool())
        .await
        .map_err(storage)
}

pub async fn put_snapshot(db: &Db, row: &SnapshotRow) -> Result<()> {
    sqlx::query(
        "INSERT INTO cm_keeper_snapshot (community_id, at, digest, shards_json, hosts_json, prev_shards_json) VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(community_id) DO UPDATE SET at = excluded.at, digest = excluded.digest, shards_json = excluded.shards_json,
           hosts_json = excluded.hosts_json, prev_shards_json = excluded.prev_shards_json",
    )
    .bind(&row.community_id)
    .bind(row.at)
    .bind(&row.digest)
    .bind(&row.shards_json)
    .bind(&row.hosts_json)
    .bind(&row.prev_shards_json)
    .execute(db.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// A blob to remove from `host` from `due_at` on.
pub async fn retire(db: &Db, community_id: &str, host: &str, sha256: &str, due_at: i64) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO cm_keeper_retired (community_id, host, sha256, due_at) VALUES (?, ?, ?, ?)")
        .bind(community_id)
        .bind(host)
        .bind(sha256)
        .bind(due_at)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// `(host, sha256)` due for removal by `now`.
pub async fn retired_due(db: &Db, community_id: &str, now: i64) -> Result<Vec<(String, String)>> {
    sqlx::query_as::<_, (String, String)>("SELECT host, sha256 FROM cm_keeper_retired WHERE community_id = ? AND due_at <= ? ORDER BY due_at, sha256")
        .bind(community_id)
        .bind(now)
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

pub async fn retired_count(db: &Db, community_id: &str) -> Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM cm_keeper_retired WHERE community_id = ?").bind(community_id).fetch_one(db.pool()).await.map_err(storage)
}

pub async fn retired_done(db: &Db, community_id: &str, host: &str, sha256: &str) -> Result<()> {
    sqlx::query("DELETE FROM cm_keeper_retired WHERE community_id = ? AND host = ? AND sha256 = ?")
        .bind(community_id)
        .bind(host)
        .bind(sha256)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// Marks an invitation used by an admission; `false` when another
/// admission used it before (the same admission again is fine).
pub async fn use_invite(db: &Db, community_id: &str, invite_id: &str, admit_id: &str, who: &str, now: i64) -> Result<bool> {
    let res = sqlx::query("INSERT OR IGNORE INTO cm_keeper_invites (community_id, invite_id, admit_id, who, used_at) VALUES (?, ?, ?, ?, ?)")
        .bind(community_id)
        .bind(invite_id)
        .bind(admit_id)
        .bind(who)
        .bind(now)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    if res.rows_affected() == 1 {
        return Ok(true);
    }
    let held = sqlx::query_scalar::<_, String>("SELECT admit_id FROM cm_keeper_invites WHERE community_id = ? AND invite_id = ?")
        .bind(community_id)
        .bind(invite_id)
        .fetch_optional(db.pool())
        .await
        .map_err(storage)?;
    Ok(held.as_deref() == Some(admit_id))
}

/// Marks an operation acted on; `false` when it was before.
pub async fn mark_op(db: &Db, community_id: &str, op_id: &str) -> Result<bool> {
    let res = sqlx::query("INSERT OR IGNORE INTO cm_keeper_ops (community_id, op_id) VALUES (?, ?)").bind(community_id).bind(op_id).execute(db.pool()).await.map_err(storage)?;
    Ok(res.rows_affected() == 1)
}

/// Marks a gift wrap taken by the keeper; `false` when it was before.
/// Those older than `before` are forgotten: a note is not taken past its
/// week anyway.
pub async fn take_wrap(db: &Db, community_id: &str, wrap_id: &str, now: i64, before: i64) -> Result<bool> {
    sqlx::query("DELETE FROM cm_keeper_wraps WHERE community_id = ? AND at < ?").bind(community_id).bind(before).execute(db.pool()).await.map_err(storage)?;
    let res = sqlx::query("INSERT OR IGNORE INTO cm_keeper_wraps (community_id, wrap_id, at) VALUES (?, ?, ?)")
        .bind(community_id)
        .bind(wrap_id)
        .bind(now)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(res.rows_affected() == 1)
}

// ─── Joins that wait for their admission ────────────────────────────────────

/// Holds a join of `who` until the admission it names comes (one per
/// person: a join sent again replaces it). `false` past `cap` held.
pub async fn hold_join(db: &Db, community_id: &str, who: &str, admit_id: &str, join_json: &str, now: i64, cap: i64) -> Result<bool> {
    let held = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM cm_keeper_held_joins WHERE community_id = ? AND who != ?")
        .bind(community_id)
        .bind(who)
        .fetch_one(db.pool())
        .await
        .map_err(storage)?;
    if held >= cap {
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO cm_keeper_held_joins (community_id, who, admit_id, join_json, held_at) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(community_id, who) DO UPDATE SET admit_id = excluded.admit_id, join_json = excluded.join_json, held_at = excluded.held_at",
    )
    .bind(community_id)
    .bind(who)
    .bind(admit_id)
    .bind(join_json)
    .bind(now)
    .execute(db.pool())
    .await
    .map_err(storage)?;
    Ok(true)
}

/// Takes out the join held for an admission: `(who, join_json)`.
pub async fn take_held_join(db: &Db, community_id: &str, admit_id: &str) -> Result<Option<(String, String)>> {
    let row = sqlx::query_as::<_, (String, String)>("SELECT who, join_json FROM cm_keeper_held_joins WHERE community_id = ? AND admit_id = ?")
        .bind(community_id)
        .bind(admit_id)
        .fetch_optional(db.pool())
        .await
        .map_err(storage)?;
    if let Some((who, _)) = &row {
        sqlx::query("DELETE FROM cm_keeper_held_joins WHERE community_id = ? AND who = ?").bind(community_id).bind(who).execute(db.pool()).await.map_err(storage)?;
    }
    Ok(row)
}

/// Drops the joins held since before `before`: their admission never came.
pub async fn expire_held_joins(db: &Db, community_id: &str, before: i64) -> Result<u64> {
    let res = sqlx::query("DELETE FROM cm_keeper_held_joins WHERE community_id = ? AND held_at < ?").bind(community_id).bind(before).execute(db.pool()).await.map_err(storage)?;
    Ok(res.rows_affected())
}

// ─── Notes that wait for the keeper's turn ──────────────────────────────────

/// A note to the keeper as it came: its rumor id, who sealed it, its
/// envelope and its date.
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct QueuedNote {
    pub note_id: String,
    pub sender: String,
    pub content: String,
    pub created_at: i64,
}

/// Puts a note in the keeper's queue; `false` when it is there already or
/// the queue holds `cap` notes of the channel.
pub async fn queue_note(db: &Db, community_id: &str, note: &QueuedNote, now: i64, cap: i64) -> Result<bool> {
    let held = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM cm_keeper_inbox WHERE community_id = ?").bind(community_id).fetch_one(db.pool()).await.map_err(storage)?;
    if held >= cap {
        return Ok(false);
    }
    let res = sqlx::query("INSERT OR IGNORE INTO cm_keeper_inbox (community_id, note_id, sender, content, created_at, queued_at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(community_id)
        .bind(&note.note_id)
        .bind(&note.sender)
        .bind(&note.content)
        .bind(note.created_at)
        .bind(now)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(res.rows_affected() == 1)
}

/// The oldest `limit` notes of the queue past the first `skip`, by date.
pub async fn queued_notes(db: &Db, community_id: &str, skip: i64, limit: i64) -> Result<Vec<QueuedNote>> {
    sqlx::query_as::<_, QueuedNote>("SELECT note_id, sender, content, created_at FROM cm_keeper_inbox WHERE community_id = ? ORDER BY created_at, note_id LIMIT ? OFFSET ?")
        .bind(community_id)
        .bind(limit)
        .bind(skip)
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

/// A note of the queue taken (or dropped).
pub async fn unqueue_note(db: &Db, community_id: &str, note_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM cm_keeper_inbox WHERE community_id = ? AND note_id = ?").bind(community_id).bind(note_id).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}

/// How many notes wait in the queue of a channel.
pub async fn queued_count(db: &Db, community_id: &str) -> Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM cm_keeper_inbox WHERE community_id = ?").bind(community_id).fetch_one(db.pool()).await.map_err(storage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use keydist::{GenerationMaster, KeyDistribution, Policy, Revoke, RootSecret, SchemeId, SdKeeper};

    const CID: &str = crate::state::tests::CHANNEL;

    /// Every kind of row goes in and comes back as the state it was, the
    /// masters derived again from the root and never written.
    #[tokio::test]
    async fn the_rows_put_the_state_back_together() {
        let db = Db::open_in_memory().await.unwrap();
        let root = RootSecret::from_bytes([7u8; 32]);
        let scope = crate::wire::scope_ref(CID, "everyone").unwrap();
        let master = |gen| GenerationMaster::derive(&root, &scope, SchemeId::SdV1, gen);
        let mut k = SdKeeper::new(scope, Policy::closed().with_min_epoch_interval(0), crate::keeper::TREE, master(1), 1_000).unwrap();
        let ms: Vec<MemberId> = (1u8..=5).map(|i| MemberId([i; 32])).collect();
        for m in &ms {
            k.join(*m, 1_000).unwrap();
        }
        assert!(k.rotate(1_000, false).built().is_some());
        write_changes(&db, CID, "everyone", &k.take_changes()).await.unwrap();
        assert!(k.published(1));
        k.revoke(&ms[..2], Revoke::Ban, 1_100);
        k.rotate(2_000, true).built().unwrap();
        write_changes(&db, CID, "everyone", &k.take_changes()).await.unwrap();

        let header: String = sqlx::query_scalar("SELECT header_json FROM cm_keeper_header").fetch_one(db.pool()).await.unwrap();
        assert!(!header.contains(&hex::encode(master(1).as_bytes())), "no master in a row");
        let all: std::collections::BTreeMap<u32, GenerationMaster> = (1..=2).map(|g| (g, master(g))).collect();
        let rows = read_changes(&db, CID, "everyone", &all).await.unwrap().unwrap();
        let back = keydist::KeeperState::replay(rows).unwrap();
        assert_eq!(&back, k.state());
        // Without the master the state is not put together.
        assert!(read_changes(&db, CID, "everyone", &Default::default()).await.is_err());
        assert_eq!(high_marks(&db, CID, "everyone").await.unwrap(), (1, 2));
        assert_eq!(members(&db, CID, "everyone").await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn an_invitation_is_used_once_and_an_operation_acted_on_once() {
        let db = Db::open_in_memory().await.unwrap();
        assert!(use_invite(&db, CID, "i1", "a1", "w", 1).await.unwrap());
        assert!(use_invite(&db, CID, "i1", "a1", "w", 2).await.unwrap(), "the same admission again");
        assert!(!use_invite(&db, CID, "i1", "a2", "x", 3).await.unwrap(), "another admission of the same invitation");
        assert!(mark_op(&db, CID, "op1").await.unwrap());
        assert!(!mark_op(&db, CID, "op1").await.unwrap());
    }
}
