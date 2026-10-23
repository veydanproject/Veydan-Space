// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from crates/messenger/store/src/groups.rs at 1f2ef31d, then changed for channels.

//! Channel tables. Meaning lives in the engine (`state`, `log`); this is
//! storage: the cache of a channel, its journal, the keeper series, the
//! staff for lists, sealed events that wait for their key, invitations,
//! requests and admissions.

use super::{now, storage, Db};
use crate::state::ChannelState;
use messenger_core::Result;

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct ChannelRow {
    pub id: String,
    pub name: String,
    pub about: String,
    pub picture: String,
    pub relays_json: String,
    pub storage_json: String,
    pub owner: String,
    pub join_mode: String,
    pub membership: String,
    pub my_role: Option<String>,
    pub members: i64,
    pub link_epoch: i64,
    pub created_at: i64,
    pub updated_at: i64,
    /// Something of the channel is newer than this build reads
    /// (`set_needs_update`); not touched by `upsert`.
    pub needs_update: bool,
    /// Why I am out (`OUT_*` of the service: left, removed, banned,
    /// disbanded); none while in. Not touched by `upsert`.
    pub out_reason: Option<String>,
    /// When the keeper was last seen by every member: the newest of the
    /// time (`at`) of an epoch event taken here and of its sign of life
    /// (wire §18). Read only (from `cm_epochs`, `cm_heartbeats`).
    pub keeper_last_seen: Option<i64>,
}

const COLS: &str =
    "id, name, about, picture, relays_json, storage_json, owner, join_mode, membership, my_role, members, link_epoch, created_at, updated_at, needs_update, out_reason,
     (SELECT MAX(s.at) FROM (SELECT e.at FROM cm_epochs e WHERE e.community_id = cm_channels.id AND e.digest != ''
        UNION ALL SELECT k.at FROM cm_heartbeats k WHERE k.community_id = cm_channels.id) s) AS keeper_last_seen";

/// Insert or refresh the cached facts of a channel.
pub async fn upsert(db: &Db, c: &ChannelRow) -> Result<()> {
    sqlx::query(
        "INSERT INTO cm_channels (id, name, about, picture, relays_json, storage_json, owner, join_mode, membership, my_role, members, link_epoch, created_at, updated_at, needs_update)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, about = excluded.about, picture = excluded.picture,
           relays_json = excluded.relays_json, storage_json = excluded.storage_json, owner = excluded.owner,
           join_mode = excluded.join_mode, membership = excluded.membership, my_role = excluded.my_role,
           members = excluded.members, link_epoch = excluded.link_epoch, updated_at = excluded.updated_at",
    )
    .bind(&c.id)
    .bind(&c.name)
    .bind(&c.about)
    .bind(&c.picture)
    .bind(&c.relays_json)
    .bind(&c.storage_json)
    .bind(&c.owner)
    .bind(&c.join_mode)
    .bind(&c.membership)
    .bind(&c.my_role)
    .bind(c.members)
    .bind(c.link_epoch)
    .bind(c.created_at)
    .bind(now())
    .bind(c.needs_update)
    .execute(db.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

pub async fn get(db: &Db, id: &str) -> Result<Option<ChannelRow>> {
    sqlx::query_as::<_, ChannelRow>(sqlx::AssertSqlSafe(format!("SELECT {COLS} FROM cm_channels WHERE id = ?")))
        .bind(id)
        .fetch_optional(db.pool())
        .await
        .map_err(storage)
}

pub async fn list(db: &Db) -> Result<Vec<ChannelRow>> {
    sqlx::query_as::<_, ChannelRow>(sqlx::AssertSqlSafe(format!("SELECT {COLS} FROM cm_channels ORDER BY updated_at DESC, id")))
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

/// Whether something of the channel waits for a newer build of the app.
pub async fn set_needs_update(db: &Db, id: &str, needs_update: bool) -> Result<()> {
    sqlx::query("UPDATE cm_channels SET needs_update = ? WHERE id = ?").bind(needs_update).bind(id).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}

/// Where I stand in a channel (`joined`, `left`, …); `false` when the
/// channel is not here.
pub async fn set_membership(db: &Db, id: &str, membership: &str) -> Result<bool> {
    let res = sqlx::query("UPDATE cm_channels SET membership = ?, updated_at = ? WHERE id = ?").bind(membership).bind(now()).bind(id).execute(db.pool()).await.map_err(storage)?;
    Ok(res.rows_affected() == 1)
}

/// Why I am out of a channel (none: in).
pub async fn set_out_reason(db: &Db, id: &str, reason: Option<&str>) -> Result<()> {
    sqlx::query("UPDATE cm_channels SET out_reason = ? WHERE id = ?").bind(reason).bind(id).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}

/// Forget a channel on this device: every row of it, in every table.
pub async fn delete(db: &Db, id: &str) -> Result<()> {
    let mut tx = db.write_tx().await?;
    for sql in [
        "DELETE FROM cm_staff WHERE community_id = ?",
        "DELETE FROM cm_ops WHERE community_id = ?",
        "DELETE FROM cm_keepers WHERE community_id = ?",
        "DELETE FROM cm_pending WHERE community_id = ?",
        "DELETE FROM cm_epochs WHERE community_id = ?",
        "DELETE FROM cm_bundles WHERE community_id = ?",
        "DELETE FROM cm_posts WHERE community_id = ?",
        "DELETE FROM cm_comments WHERE community_id = ?",
        "DELETE FROM cm_reactions WHERE community_id = ?",
        "DELETE FROM cm_marks WHERE community_id = ?",
        "DELETE FROM cm_invites WHERE community_id = ?",
        "DELETE FROM cm_requests WHERE community_id = ?",
        "DELETE FROM cm_admits WHERE community_id = ?",
        "DELETE FROM cm_snapshots WHERE community_id = ?",
        "DELETE FROM cm_readers WHERE community_id = ?",
        "DELETE FROM cm_read WHERE community_id = ?",
        "DELETE FROM cm_avatar WHERE community_id = ?",
        "DELETE FROM cm_joining WHERE community_id = ?",
        "DELETE FROM cm_link_owed WHERE community_id = ?",
        "DELETE FROM cm_sync WHERE community_id = ?",
        "DELETE FROM cm_uploads WHERE community_id = ?",
        "DELETE FROM cm_genesis WHERE community_id = ?",
        "DELETE FROM cm_scopes WHERE community_id = ?",
        "DELETE FROM cm_levels WHERE community_id = ?",
        "DELETE FROM cm_meta_owed WHERE community_id = ?",
        "DELETE FROM cm_level_requests WHERE community_id = ?",
        "DELETE FROM cm_heartbeats WHERE community_id = ?",
        "DELETE FROM cm_outbox WHERE community_id = ?",
        "DELETE FROM cm_channels WHERE id = ?",
    ] {
        sqlx::query(sql).bind(id).execute(&mut *tx).await.map_err(storage)?;
    }
    tx.commit().await.map_err(storage)
}

// ─── The staff and the keepers, as the journal says ─────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct StaffRow {
    pub community_id: String,
    pub pubkey: String,
    pub role: String,
    pub muted: bool,
    pub muted_until: Option<i64>,
    pub trusted: bool,
    pub levels: i64,
    pub since: i64,
    pub added_by: String,
}

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct KeeperRow {
    pub community_id: String,
    pub serial: i64,
    pub pubkey: String,
    pub from_epoch: i64,
}

/// Write what the state of the journal says of people and keepers, in
/// place of what was there: the cache follows the journal whole.
pub async fn replace_from_state(db: &Db, state: &ChannelState) -> Result<()> {
    let id = &state.community_id;
    let mut tx = db.write_tx().await?;
    sqlx::query("DELETE FROM cm_staff WHERE community_id = ?").bind(id).execute(&mut *tx).await.map_err(storage)?;
    sqlx::query("DELETE FROM cm_keepers WHERE community_id = ?").bind(id).execute(&mut *tx).await.map_err(storage)?;
    for (pubkey, m) in &state.staff {
        sqlx::query(
            "INSERT INTO cm_staff (community_id, pubkey, role, muted, muted_until, trusted, levels, since, added_by)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(pubkey)
        .bind(m.role.as_str())
        .bind(m.muted)
        .bind(m.muted_until)
        .bind(m.trusted)
        // Levels are a bit set: kept as the same 64 bits.
        .bind(m.levels as i64)
        .bind(m.since)
        .bind(m.added_by.as_hex())
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    }
    for k in &state.keepers {
        sqlx::query("INSERT INTO cm_keepers (community_id, serial, pubkey, from_epoch) VALUES (?, ?, ?, ?)")
            .bind(id)
            .bind(i64::from(k.serial))
            .bind(k.pubkey.as_hex())
            .bind(i64::from(k.from_epoch))
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
    }
    tx.commit().await.map_err(storage)
}

pub async fn staff(db: &Db, community_id: &str) -> Result<Vec<StaffRow>> {
    sqlx::query_as::<_, StaffRow>(
        "SELECT community_id, pubkey, role, muted, muted_until, trusted, levels, since, added_by FROM cm_staff
         WHERE community_id = ? ORDER BY since, pubkey",
    )
    .bind(community_id)
    .fetch_all(db.pool())
    .await
    .map_err(storage)
}

pub async fn keepers(db: &Db, community_id: &str) -> Result<Vec<KeeperRow>> {
    sqlx::query_as::<_, KeeperRow>("SELECT community_id, serial, pubkey, from_epoch FROM cm_keepers WHERE community_id = ? ORDER BY serial")
        .bind(community_id)
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

// ─── The journal ────────────────────────────────────────────────────────────

/// `false` when the operation was already stored. `op_json` is the
/// signed event, so that it can be handed on as it came.
pub async fn insert_op(db: &Db, community_id: &str, op_id: &str, op_json: &str) -> Result<bool> {
    let res = sqlx::query("INSERT OR IGNORE INTO cm_ops (op_id, community_id, op_json, received_at) VALUES (?, ?, ?, ?)")
        .bind(op_id)
        .bind(community_id)
        .bind(op_json)
        .bind(now())
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(res.rows_affected() == 1)
}

pub async fn ops(db: &Db, community_id: &str) -> Result<Vec<String>> {
    sqlx::query_scalar::<_, String>("SELECT op_json FROM cm_ops WHERE community_id = ? ORDER BY received_at, op_id")
        .bind(community_id)
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

/// How many operations of a channel are kept.
pub async fn count_ops(db: &Db, community_id: &str) -> Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM cm_ops WHERE community_id = ?").bind(community_id).fetch_one(db.pool()).await.map_err(storage)
}

/// The owner a link or an accepted invitation named for a channel: the
/// only author whose `create` is taken for it (wire §3).
pub async fn set_genesis_owner(db: &Db, community_id: &str, owner: &str) -> Result<()> {
    sqlx::query("INSERT INTO cm_genesis (community_id, owner, set_at) VALUES (?, ?, ?) ON CONFLICT(community_id) DO UPDATE SET owner = excluded.owner, set_at = excluded.set_at")
        .bind(community_id)
        .bind(owner)
        .bind(now())
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn genesis_owner(db: &Db, community_id: &str) -> Result<Option<String>> {
    sqlx::query_scalar::<_, String>("SELECT owner FROM cm_genesis WHERE community_id = ?").bind(community_id).fetch_optional(db.pool()).await.map_err(storage)
}

/// Drops a journal taken under a `create` that is not the channel's, and
/// what was cached from it; invitations and requests stay.
pub async fn drop_journal(db: &Db, community_id: &str) -> Result<()> {
    let mut tx = db.write_tx().await?;
    for sql in [
        "DELETE FROM cm_ops WHERE community_id = ?",
        "DELETE FROM cm_staff WHERE community_id = ?",
        "DELETE FROM cm_keepers WHERE community_id = ?",
        "DELETE FROM cm_snapshots WHERE community_id = ?",
        "DELETE FROM cm_readers WHERE community_id = ?",
        "DELETE FROM cm_sync WHERE community_id = ?",
        "DELETE FROM cm_channels WHERE id = ?",
    ] {
        sqlx::query(sql).bind(community_id).execute(&mut *tx).await.map_err(storage)?;
    }
    tx.commit().await.map_err(storage)
}

// ─── Sealed events that wait for their key ──────────────────────────────────

/// How long a sealed event waits for its key (§4).
pub const PENDING_TTL_SECS: i64 = 7 * 24 * 3600;
/// How many sealed events of a channel wait at most.
pub const MAX_PENDING: i64 = 2000;

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct PendingRow {
    pub event_id: String,
    pub community_id: String,
    pub scope_id: String,
    pub epoch: i64,
    pub key_id: String,
    pub event_json: String,
    pub created_at: i64,
    pub received_at: i64,
}

const P_COLS: &str = "event_id, community_id, scope_id, epoch, key_id, event_json, created_at, received_at";

pub async fn add_pending(db: &Db, p: &PendingRow) -> Result<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO cm_pending (event_id, community_id, scope_id, epoch, key_id, event_json, created_at, received_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&p.event_id)
    .bind(&p.community_id)
    .bind(&p.scope_id)
    .bind(p.epoch)
    .bind(&p.key_id)
    .bind(&p.event_json)
    .bind(p.created_at)
    .bind(now())
    .execute(db.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// Take (and remove) everything that waits for this key, oldest first.
/// One statement that writes from its start: a read and then a delete in
/// one transaction failed on its stale snapshot when another task of the
/// device wrote between them (SQLITE_BUSY_SNAPSHOT, which no busy timeout
/// waits out), and the event that took them was lost.
pub async fn take_pending(db: &Db, community_id: &str, key_id: &str) -> Result<Vec<PendingRow>> {
    let mut rows = sqlx::query_as::<_, PendingRow>(sqlx::AssertSqlSafe(format!("DELETE FROM cm_pending WHERE community_id = ? AND key_id = ? RETURNING {P_COLS}")))
        .bind(community_id)
        .bind(key_id)
        .fetch_all(db.pool())
        .await
        .map_err(storage)?;
    rows.sort_by(|a, b| (a.created_at, &a.event_id).cmp(&(b.created_at, &b.event_id)));
    Ok(rows)
}

/// Whether an event of an epoch past `epoch` waits under `key_id` (an
/// epoch ahead of the ones taken).
pub async fn has_pending_after(db: &Db, community_id: &str, key_id: &str, epoch: i64) -> Result<bool> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cm_pending WHERE community_id = ? AND key_id = ? AND epoch > ?")
        .bind(community_id)
        .bind(key_id)
        .bind(epoch)
        .fetch_one(db.pool())
        .await
        .map_err(storage)?;
    Ok(n > 0)
}

/// The epochs something waits for, to be asked for (`n`).
pub async fn pending_epochs(db: &Db, community_id: &str) -> Result<Vec<(String, i64)>> {
    sqlx::query_as::<_, (String, i64)>("SELECT DISTINCT scope_id, epoch FROM cm_pending WHERE community_id = ? ORDER BY scope_id, epoch")
        .bind(community_id)
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

pub async fn delete_pending(db: &Db, event_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM cm_pending WHERE event_id = ?").bind(event_id).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}

/// Drop what waited longer than `PENDING_TTL_SECS` (by its receipt here,
/// never by the date its author wrote on it); how many went.
pub async fn expire_pending(db: &Db, now: i64) -> Result<u64> {
    let res = sqlx::query("DELETE FROM cm_pending WHERE received_at <= ?").bind(now - PENDING_TTL_SECS).execute(db.pool()).await.map_err(storage)?;
    Ok(res.rows_affected())
}

pub async fn count_pending(db: &Db, community_id: &str) -> Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM cm_pending WHERE community_id = ?").bind(community_id).fetch_one(db.pool()).await.map_err(storage)
}

// ─── Joining and links owed ─────────────────────────────────────────────────

/// A `channel.join` sent and not answered yet (wire §7).
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct JoiningRow {
    pub community_id: String,
    pub join_json: String,
    pub first_at: i64,
    pub next_at: i64,
    pub attempts: i64,
}

/// A join sent now: asked again at `next_at` unless an issue comes.
pub async fn put_joining(db: &Db, community_id: &str, join_json: &str, now: i64, next_at: i64) -> Result<()> {
    sqlx::query(
        "INSERT INTO cm_joining (community_id, join_json, first_at, next_at, attempts) VALUES (?, ?, ?, ?, 1)
         ON CONFLICT(community_id) DO UPDATE SET join_json = excluded.join_json, first_at = excluded.first_at,
           next_at = excluded.next_at, attempts = 1",
    )
    .bind(community_id)
    .bind(join_json)
    .bind(now)
    .bind(next_at)
    .execute(db.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// The join of a channel due at `now`, whatever its time was: its keeper
/// is heard at another inbox now. True when a join waits.
pub async fn joining_due(db: &Db, community_id: &str, now: i64) -> Result<bool> {
    let done = sqlx::query("UPDATE cm_joining SET next_at = MIN(next_at, ?) WHERE community_id = ?").bind(now).bind(community_id).execute(db.pool()).await.map_err(storage)?;
    Ok(done.rows_affected() > 0)
}

/// The join of a channel sent again: the next time and one more attempt.
pub async fn joining_sent(db: &Db, community_id: &str, next_at: i64) -> Result<()> {
    sqlx::query("UPDATE cm_joining SET next_at = ?, attempts = attempts + 1 WHERE community_id = ?").bind(next_at).bind(community_id).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}

pub async fn joining(db: &Db) -> Result<Vec<JoiningRow>> {
    sqlx::query_as::<_, JoiningRow>("SELECT community_id, join_json, first_at, next_at, attempts FROM cm_joining ORDER BY next_at")
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

/// The keeper answered (an issue, or a refusal), or the join was given up.
pub async fn joining_done(db: &Db, community_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM cm_joining WHERE community_id = ?").bind(community_id).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}

/// The secret of `link_epoch` is mine to put in the feed (wire §10).
/// `after_epoch`: the secret goes out under a key of a later epoch only
/// (one built after every revoke pending when the link was rotated).
pub async fn owe_link(db: &Db, community_id: &str, link_epoch: u32, after_epoch: u32) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO cm_link_owed (community_id, link_epoch, after_epoch) VALUES (?, ?, ?)")
        .bind(community_id)
        .bind(i64::from(link_epoch))
        .bind(i64::from(after_epoch))
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// The link epochs I owe the feed, by channel:
/// `(community_id, link_epoch, after_epoch)`.
pub async fn links_owed(db: &Db) -> Result<Vec<(String, i64, i64)>> {
    sqlx::query_as::<_, (String, i64, i64)>("SELECT community_id, link_epoch, after_epoch FROM cm_link_owed ORDER BY community_id, link_epoch")
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

pub async fn link_paid(db: &Db, community_id: &str, link_epoch: i64) -> Result<()> {
    sqlx::query("DELETE FROM cm_link_owed WHERE community_id = ? AND link_epoch = ?").bind(community_id).bind(link_epoch).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}

// ─── How far the relays were heard ──────────────────────────────────────────

/// An event of `part` dated `at` was heard: the cursor moves forward only.
pub async fn heard(db: &Db, community_id: &str, part: &str, at: i64) -> Result<()> {
    sqlx::query(
        "INSERT INTO cm_sync (community_id, part, last_at) VALUES (?, ?, ?)
         ON CONFLICT(community_id, part) DO UPDATE SET last_at = MAX(cm_sync.last_at, excluded.last_at)",
    )
    .bind(community_id)
    .bind(part)
    .bind(at)
    .execute(db.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// The newest sign of life of the keeper of a channel held here (wire §18).
pub async fn keeper_seen(db: &Db, community_id: &str) -> Result<Option<i64>> {
    sqlx::query_scalar::<_, i64>("SELECT at FROM cm_heartbeats WHERE community_id = ?").bind(community_id).fetch_optional(db.pool()).await.map_err(storage)
}

/// A sign of life of the keeper of a channel, kept when newer.
pub async fn put_keeper_seen(db: &Db, community_id: &str, at: i64, serial: u32) -> Result<()> {
    sqlx::query(
        "INSERT INTO cm_heartbeats (community_id, at, serial) VALUES (?, ?, ?)
         ON CONFLICT(community_id) DO UPDATE SET at = excluded.at, serial = excluded.serial WHERE excluded.at > cm_heartbeats.at",
    )
    .bind(community_id)
    .bind(at)
    .bind(i64::from(serial))
    .execute(db.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// The newest date heard of `part`, if any.
pub async fn heard_at(db: &Db, community_id: &str, part: &str) -> Result<Option<i64>> {
    sqlx::query_scalar::<_, i64>("SELECT last_at FROM cm_sync WHERE community_id = ? AND part = ?").bind(community_id).bind(part).fetch_optional(db.pool()).await.map_err(storage)
}

/// The round of a catch-up of one part of a channel (`013_sync_rounds`).
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct Round {
    pub last_at: i64,
    pub round_since: Option<i64>,
    pub round_started: i64,
    pub page_count: i64,
    pub page_min: Option<i64>,
    pub round_max: Option<i64>,
}

pub async fn round(db: &Db, community_id: &str, part: &str) -> Result<Option<Round>> {
    sqlx::query_as::<_, Round>("SELECT last_at, round_since, round_started, page_count, page_min, round_max FROM cm_sync WHERE community_id = ? AND part = ?")
        .bind(community_id)
        .bind(part)
        .fetch_optional(db.pool())
        .await
        .map_err(storage)
}

/// The cursor moves to what the round heard (it came back whole), and
/// the round closes.
pub async fn round_done(db: &Db, community_id: &str, part: &str) -> Result<()> {
    sqlx::query(
        "UPDATE cm_sync SET last_at = MAX(last_at, COALESCE(round_max, last_at)), round_since = NULL, page_count = 0, page_min = NULL, round_max = NULL
         WHERE community_id = ? AND part = ?",
    )
    .bind(community_id)
    .bind(part)
    .execute(db.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// A round of catch-up starts from `since` (0: from the start). The
/// newest event heard so far stays: it moves the cursor when this round
/// comes back whole.
pub async fn round_start(db: &Db, community_id: &str, part: &str, since: i64, now: i64) -> Result<()> {
    sqlx::query(
        "INSERT INTO cm_sync (community_id, part, last_at, round_since, round_started) VALUES (?, ?, 0, ?, ?)
         ON CONFLICT(community_id, part) DO UPDATE SET round_since = excluded.round_since, round_started = excluded.round_started, page_count = 0, page_min = NULL",
    )
    .bind(community_id)
    .bind(part)
    .bind(since)
    .bind(now)
    .execute(db.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// An event of a part heard at `at`; `page`: it came in a catch-up (not
/// live). When the page in a round is full (`limit`), `Some((since,
/// until))`: the next page to ask for, and the page count starts again.
pub async fn round_heard(db: &Db, community_id: &str, part: &str, at: i64, page: bool, limit: i64) -> Result<Option<(i64, i64)>> {
    sqlx::query(
        "INSERT INTO cm_sync (community_id, part, last_at, round_max) VALUES (?, ?, 0, ?)
         ON CONFLICT(community_id, part) DO UPDATE SET round_max = MAX(COALESCE(cm_sync.round_max, excluded.round_max), excluded.round_max)",
    )
    .bind(community_id)
    .bind(part)
    .bind(at)
    .execute(db.pool())
    .await
    .map_err(storage)?;
    if !page {
        return Ok(None);
    }
    sqlx::query("UPDATE cm_sync SET page_count = page_count + 1, page_min = MIN(COALESCE(page_min, ?1), ?1) WHERE community_id = ?2 AND part = ?3 AND round_since IS NOT NULL")
        .bind(at)
        .bind(community_id)
        .bind(part)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    let Some(r) = round(db, community_id, part).await? else { return Ok(None) };
    match (r.round_since, r.page_min) {
        (Some(since), Some(min)) if r.page_count >= limit => {
            sqlx::query("UPDATE cm_sync SET page_count = 0, page_min = NULL WHERE community_id = ? AND part = ?").bind(community_id).bind(part).execute(db.pool()).await.map_err(storage)?;
            Ok(Some((since, min)))
        }
        _ => Ok(None),
    }
}

/// The signed operations of a channel as `(id, created_at)`: what this
/// device holds of the journal, for a reconciliation.
pub async fn op_items(db: &Db, community_id: &str) -> Result<Vec<(String, i64)>> {
    let rows = ops(db, community_id).await?;
    Ok(rows
        .iter()
        .filter_map(|j| serde_json::from_str::<serde_json::Value>(j).ok())
        .filter_map(|v| Some((v["id"].as_str()?.to_string(), v["created_at"].as_i64()?)))
        .collect())
}

// ─── Invites ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct InviteRow {
    pub invite_id: String,
    pub community_id: String,
    pub direction: String,
    pub peer: String,
    pub status: String,
    pub payload_json: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub updated_at: i64,
    /// Of an invitation that came: the journal here says its sender is a
    /// moderator. One that came before the journal is not, until it does
    /// (and one the journal does not bear out is dropped then).
    pub verified: bool,
}

const I_COLS: &str = "invite_id, community_id, direction, peer, status, payload_json, created_at, expires_at, updated_at, verified";

/// `false` when an invite with this id exists already.
pub async fn insert_invite(db: &Db, i: &InviteRow) -> Result<bool> {
    let res = sqlx::query(
        "INSERT OR IGNORE INTO cm_invites (invite_id, community_id, direction, peer, status, payload_json, created_at, expires_at, updated_at, verified)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&i.invite_id)
    .bind(&i.community_id)
    .bind(&i.direction)
    .bind(&i.peer)
    .bind(&i.status)
    .bind(&i.payload_json)
    .bind(i.created_at)
    .bind(i.expires_at)
    .bind(now())
    .bind(i.verified)
    .execute(db.pool())
    .await
    .map_err(storage)?;
    Ok(res.rows_affected() == 1)
}

pub async fn invite(db: &Db, invite_id: &str) -> Result<Option<InviteRow>> {
    sqlx::query_as::<_, InviteRow>(sqlx::AssertSqlSafe(format!("SELECT {I_COLS} FROM cm_invites WHERE invite_id = ?")))
        .bind(invite_id)
        .fetch_optional(db.pool())
        .await
        .map_err(storage)
}

pub async fn invites(db: &Db, direction: &str, statuses: &[&str]) -> Result<Vec<InviteRow>> {
    let all = sqlx::query_as::<_, InviteRow>(sqlx::AssertSqlSafe(format!("SELECT {I_COLS} FROM cm_invites WHERE direction = ? ORDER BY created_at DESC")))
        .bind(direction)
        .fetch_all(db.pool())
        .await
        .map_err(storage)?;
    Ok(all.into_iter().filter(|i| statuses.is_empty() || statuses.contains(&i.status.as_str())).collect())
}

pub async fn set_invite_status(db: &Db, invite_id: &str, status: &str) -> Result<()> {
    sqlx::query("UPDATE cm_invites SET status = ?, updated_at = ? WHERE invite_id = ?").bind(status).bind(now()).bind(invite_id).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}

/// Open invites past their time become `expired`.
pub async fn expire_invites(db: &Db, now: i64) -> Result<u64> {
    let res = sqlx::query("UPDATE cm_invites SET status = 'expired', updated_at = ? WHERE status IN ('sent', 'received') AND expires_at <= ?")
        .bind(now)
        .bind(now)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(res.rows_affected())
}

/// Invitations that came of a channel whose journal had not come:
/// `verified` set where `moderator` says so, the others dropped.
pub async fn verify_invites(db: &Db, community_id: &str, moderator: impl Fn(&str) -> bool) -> Result<()> {
    let rows = sqlx::query_as::<_, (String, String)>("SELECT invite_id, peer FROM cm_invites WHERE community_id = ? AND direction = 'in' AND verified = 0")
        .bind(community_id)
        .fetch_all(db.pool())
        .await
        .map_err(storage)?;
    for (id, peer) in rows {
        let sql = if moderator(&peer) { "UPDATE cm_invites SET verified = 1 WHERE invite_id = ?" } else { "DELETE FROM cm_invites WHERE invite_id = ?" };
        sqlx::query(sql).bind(&id).execute(db.pool()).await.map_err(storage)?;
    }
    Ok(())
}

pub async fn delete_invite(db: &Db, invite_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM cm_invites WHERE invite_id = ?").bind(invite_id).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}

// ─── Requests ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct RequestRow {
    pub community_id: String,
    pub requester: String,
    pub direction: String,
    pub status: String,
    pub payload_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

const R_COLS: &str = "community_id, requester, direction, status, payload_json, created_at, updated_at";

pub async fn put_request(db: &Db, r: &RequestRow) -> Result<()> {
    sqlx::query(
        "INSERT INTO cm_requests (community_id, requester, direction, status, payload_json, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(community_id, requester, direction) DO UPDATE SET status = excluded.status,
           payload_json = excluded.payload_json, created_at = excluded.created_at, updated_at = excluded.updated_at",
    )
    .bind(&r.community_id)
    .bind(&r.requester)
    .bind(&r.direction)
    .bind(&r.status)
    .bind(&r.payload_json)
    .bind(r.created_at)
    .bind(now())
    .execute(db.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

pub async fn request(db: &Db, community_id: &str, requester: &str, direction: &str) -> Result<Option<RequestRow>> {
    sqlx::query_as::<_, RequestRow>(sqlx::AssertSqlSafe(format!("SELECT {R_COLS} FROM cm_requests WHERE community_id = ? AND requester = ? AND direction = ?")))
        .bind(community_id)
        .bind(requester)
        .bind(direction)
        .fetch_optional(db.pool())
        .await
        .map_err(storage)
}

pub async fn requests(db: &Db, community_id: &str, direction: &str, status: &str) -> Result<Vec<RequestRow>> {
    sqlx::query_as::<_, RequestRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {R_COLS} FROM cm_requests WHERE community_id = ? AND direction = ? AND status = ? ORDER BY created_at"
    )))
    .bind(community_id)
    .bind(direction)
    .bind(status)
    .fetch_all(db.pool())
    .await
    .map_err(storage)
}

// ─── Admissions ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct AdmitRow {
    pub admit_id: String,
    pub community_id: String,
    pub who: String,
    pub by: String,
    pub levels: i64,
    pub invite_id: Option<String>,
    pub expires_at: i64,
    /// `open` | `used` | `expired`
    pub status: String,
    pub created_at: i64,
}

const A_COLS: &str = "admit_id, community_id, who, by, levels, invite_id, expires_at, status, created_at";

/// `false` when the admission, or another of the same invitation, is
/// here already: an `invite_id` is used once (§7.1).
pub async fn insert_admit(db: &Db, a: &AdmitRow) -> Result<bool> {
    let res = sqlx::query(
        "INSERT OR IGNORE INTO cm_admits (admit_id, community_id, who, by, levels, invite_id, expires_at, status, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&a.admit_id)
    .bind(&a.community_id)
    .bind(&a.who)
    .bind(&a.by)
    .bind(a.levels)
    .bind(&a.invite_id)
    .bind(a.expires_at)
    .bind(&a.status)
    .bind(a.created_at)
    .execute(db.pool())
    .await
    .map_err(storage)?;
    Ok(res.rows_affected() == 1)
}

pub async fn admit(db: &Db, admit_id: &str) -> Result<Option<AdmitRow>> {
    sqlx::query_as::<_, AdmitRow>(sqlx::AssertSqlSafe(format!("SELECT {A_COLS} FROM cm_admits WHERE admit_id = ?")))
        .bind(admit_id)
        .fetch_optional(db.pool())
        .await
        .map_err(storage)
}

/// Mark an open admission used; `false` when it was not open (used,
/// expired, unknown): the keeper issues once for one admission.
pub async fn use_admit(db: &Db, admit_id: &str, now: i64) -> Result<bool> {
    let res = sqlx::query("UPDATE cm_admits SET status = 'used' WHERE admit_id = ? AND status = 'open' AND expires_at > ?")
        .bind(admit_id)
        .bind(now)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(res.rows_affected() == 1)
}

pub async fn expire_admits(db: &Db, now: i64) -> Result<u64> {
    let res = sqlx::query("UPDATE cm_admits SET status = 'expired' WHERE status = 'open' AND expires_at <= ?").bind(now).execute(db.pool()).await.map_err(storage)?;
    Ok(res.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op::OpBody;
    use crate::roles::Role;
    use crate::state::tests::{op, pk, team};

    fn channel(id: &str) -> ChannelRow {
        ChannelRow {
            id: id.into(),
            name: "News".into(),
            about: String::new(),
            picture: String::new(),
            relays_json: r#"["wss://r.example"]"#.into(),
            storage_json: r#"["https://s.example"]"#.into(),
            owner: "o".into(),
            join_mode: "open".into(),
            membership: "joined".into(),
            my_role: Some("owner".into()),
            members: 1,
            link_epoch: 0,
            created_at: 0,
            updated_at: 0,
            needs_update: false,
            out_reason: None,
            keeper_last_seen: None,
        }
    }

    #[tokio::test]
    async fn channels_ops_staff_pending() {
        let db = Db::open_in_memory().await.unwrap();
        upsert(&db, &channel("c1")).await.unwrap();
        let mut c = channel("c1");
        c.name = "Renamed".into();
        c.members = 3;
        upsert(&db, &c).await.unwrap();
        assert_eq!(get(&db, "c1").await.unwrap().unwrap().name, "Renamed");
        assert_eq!(list(&db).await.unwrap().len(), 1);

        assert!(insert_op(&db, "c1", "op1", "{}").await.unwrap());
        assert!(!insert_op(&db, "c1", "op1", "{}").await.unwrap());
        insert_op(&db, "c1", "op2", "{\"a\":1}").await.unwrap();
        assert_eq!(ops(&db, "c1").await.unwrap().len(), 2);

        // The staff and the keepers follow the state whole.
        let mut s = team();
        s.community_id = "c1".into();
        replace_from_state(&db, &s).await.unwrap();
        assert_eq!(staff(&db, "c1").await.unwrap().len(), 4);
        assert_eq!(keepers(&db, "c1").await.unwrap().len(), 1);
        s.community_id = crate::state::tests::CHANNEL.into();
        s.apply(&op(&pk("0"), OpBody::SetRole { who: pk("e"), role: Role::Member })).unwrap();
        s.apply(&op(&pk("b"), OpBody::SetMuted { who: pk("c"), muted: true, until: Some(900) })).unwrap();
        s.community_id = "c1".into();
        replace_from_state(&db, &s).await.unwrap();
        let rows = staff(&db, "c1").await.unwrap();
        assert_eq!(rows.len(), 4, "the author went, the muted reader came");
        assert!(rows.iter().any(|r| r.pubkey == pk("c").as_hex() && r.muted && r.muted_until == Some(900) && r.role == "member"));

        for (id, at, key) in [("e2", 20, "k1"), ("e1", 10, "k1"), ("e3", 5, "k2")] {
            add_pending(&db, &PendingRow { event_id: id.into(), community_id: "c1".into(), scope_id: "everyone".into(), epoch: 4, key_id: key.into(), event_json: "{}".into(), created_at: at, received_at: 0 })
                .await
                .unwrap();
        }
        assert_eq!(count_pending(&db, "c1").await.unwrap(), 3);
        assert_eq!(pending_epochs(&db, "c1").await.unwrap(), vec![("everyone".to_string(), 4)]);
        let taken = take_pending(&db, "c1", "k1").await.unwrap();
        assert_eq!(taken.iter().map(|p| p.event_id.as_str()).collect::<Vec<_>>(), vec!["e1", "e2"]);
        assert_eq!(count_pending(&db, "c1").await.unwrap(), 1);
        assert_eq!(expire_pending(&db, now() + PENDING_TTL_SECS - 10).await.unwrap(), 0, "not seven days yet");
        assert_eq!(expire_pending(&db, now() + PENDING_TTL_SECS + 10).await.unwrap(), 1);

        assert!(!get(&db, "c1").await.unwrap().unwrap().needs_update);
        set_needs_update(&db, "c1", true).await.unwrap();
        upsert(&db, &channel("c1")).await.unwrap();
        assert!(get(&db, "c1").await.unwrap().unwrap().needs_update, "a refresh of the row does not clear it");

        delete(&db, "c1").await.unwrap();
        assert!(get(&db, "c1").await.unwrap().is_none());
        assert!(ops(&db, "c1").await.unwrap().is_empty());
        assert!(staff(&db, "c1").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn invites_requests_admits() {
        let db = Db::open_in_memory().await.unwrap();
        let inv = InviteRow {
            invite_id: "i1".into(),
            community_id: "c1".into(),
            direction: "in".into(),
            peer: "p".into(),
            status: "received".into(),
            payload_json: "{}".into(),
            created_at: 100,
            expires_at: 200,
            updated_at: 0,
            verified: true,
        };
        assert!(insert_invite(&db, &inv).await.unwrap());
        assert!(!insert_invite(&db, &inv).await.unwrap(), "same id twice");
        assert_eq!(invites(&db, "in", &["received"]).await.unwrap().len(), 1);
        assert_eq!(expire_invites(&db, 150).await.unwrap(), 0);
        assert_eq!(expire_invites(&db, 200).await.unwrap(), 1);
        assert_eq!(invite(&db, "i1").await.unwrap().unwrap().status, "expired");
        set_invite_status(&db, "i1", "declined").await.unwrap();
        delete_invite(&db, "i1").await.unwrap();
        assert!(invite(&db, "i1").await.unwrap().is_none());

        let req = RequestRow { community_id: "c1".into(), requester: "x".into(), direction: "in".into(), status: "pending".into(), payload_json: "{}".into(), created_at: 1, updated_at: 0 };
        put_request(&db, &req).await.unwrap();
        assert_eq!(requests(&db, "c1", "in", "pending").await.unwrap().len(), 1);
        put_request(&db, &RequestRow { status: "approved".into(), ..req }).await.unwrap();
        assert!(requests(&db, "c1", "in", "pending").await.unwrap().is_empty());
        assert_eq!(request(&db, "c1", "x", "in").await.unwrap().unwrap().status, "approved");

        let admit_row = AdmitRow {
            admit_id: "a1".into(),
            community_id: "c1".into(),
            who: "x".into(),
            by: "m".into(),
            levels: 0,
            invite_id: Some("i9".into()),
            expires_at: 1_000,
            status: "open".into(),
            created_at: 1,
        };
        assert!(insert_admit(&db, &admit_row).await.unwrap());
        assert!(!insert_admit(&db, &AdmitRow { admit_id: "a2".into(), ..admit_row.clone() }).await.unwrap(), "an invitation is used once");
        assert!(insert_admit(&db, &AdmitRow { admit_id: "a3".into(), invite_id: None, ..admit_row.clone() }).await.unwrap());
        assert!(!use_admit(&db, "a1", 1_000).await.unwrap(), "expired");
        assert!(use_admit(&db, "a1", 999).await.unwrap());
        assert!(!use_admit(&db, "a1", 999).await.unwrap(), "once");
        assert_eq!(admit(&db, "a1").await.unwrap().unwrap().status, "used");
        assert_eq!(expire_admits(&db, 2_000).await.unwrap(), 1);
    }
}
