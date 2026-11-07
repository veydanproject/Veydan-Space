// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The tables of the archive of the conversation (migration 043):
//! `msg_archive_events`, `msg_archive_state`, `msg_archive_queue`. Rows of
//! plain strings and numbers; what they mean is `messenger-archive`'s
//! (`internal/messenger-archive-plan.md` §4, `internal/messenger-wire.md`
//! §8). A segment is named by its scope key and its month, `yyyy-mm`; the
//! owner of the archive is the store's one identity and is not kept here.

use crate::{storage, Store};
use messenger_core::Result;
use sqlx::{Sqlite, Transaction};
use std::collections::{HashMap, HashSet};

/// One event of a segment, as the archive keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventRow {
    pub id: String,
    pub scope: String,
    pub month: String,
    pub at: i64,
    pub received_at: i64,
    pub kind: i64,
    /// The rumor id of a personal message (`msg_messages.id`).
    pub msg_id: Option<String>,
    /// The record as the segment file holds it.
    pub event_json: String,
}

/// An event in the order of the file to write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteRow {
    pub id: String,
    pub at: i64,
    pub event_json: String,
}

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct StateRow {
    pub scope: String,
    pub month: String,
    pub dirty: bool,
    pub local_count: i64,
    pub written_count: Option<i64>,
    pub written_hash: Option<String>,
    pub written_at: Option<i64>,
    pub remote_count: Option<i64>,
    pub remote_hash: Option<String>,
}

/// To download and replay.
pub const QUEUE_PENDING: &str = "pending";
/// Replayed whole.
pub const QUEUE_DONE: &str = "done";
/// The storage did not give it, or the ingress broke off: tried again with
/// the next round.
pub const QUEUE_FAILED: &str = "failed";
/// The runtime refuses it for now (a group this device has no row for, a
/// private server it does not know): tried again when such a row comes.
pub const QUEUE_PARKED: &str = "parked";
/// A file this build does not understand: tried again by another build.
pub const QUEUE_REFUSED: &str = "refused";

/// The statuses of a segment still to come: what the progress counts.
const OPEN_STATUSES: &str = "('pending', 'done', 'failed')";

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct QueueRow {
    pub entity_id: String,
    pub scope: String,
    pub month: String,
    pub count: i64,
    pub size: i64,
    pub hash: String,
    pub status: String,
    pub priority: i64,
    pub error: Option<String>,
    /// The build that refused it (`QUEUE_REFUSED`), '' otherwise.
    pub build: String,
}

/// What the queue has done, in segments and in bytes; parked and refused
/// segments are not counted: the restore ends without them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueueProgress {
    pub done: i64,
    pub total: i64,
    pub bytes_done: i64,
    pub bytes_total: i64,
}

async fn ensure_state(tx: &mut Transaction<'_, Sqlite>, scope: &str, month: &str) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO msg_archive_state (scope, month) VALUES (?, ?)")
        .bind(scope)
        .bind(month)
        .execute(&mut **tx)
        .await
        .map_err(storage)?;
    Ok(())
}

async fn recount(tx: &mut Transaction<'_, Sqlite>, scope: &str, month: &str) -> Result<()> {
    sqlx::query(
        "UPDATE msg_archive_state SET local_count =
           (SELECT COUNT(*) FROM msg_archive_events WHERE scope = ?1 AND month = ?2 AND dropped = 0)
         WHERE scope = ?1 AND month = ?2",
    )
    .bind(scope)
    .bind(month)
    .execute(&mut **tx)
    .await
    .map_err(storage)?;
    Ok(())
}

/// An event the device received: kept if its id is new, and its segment
/// is then dirty. One already held (by a merge that has not replayed it
/// yet) is marked replayed: the ingress has it now. `true` when new.
pub async fn record(store: &Store, row: &EventRow) -> Result<bool> {
    let mut tx = store.pool().begin().await.map_err(storage)?;
    let inserted = sqlx::query(
        "INSERT OR IGNORE INTO msg_archive_events (id, scope, month, at, received_at, kind, msg_id, event_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&row.id)
    .bind(&row.scope)
    .bind(&row.month)
    .bind(row.at)
    .bind(row.received_at)
    .bind(row.kind)
    .bind(&row.msg_id)
    .bind(&row.event_json)
    .execute(&mut *tx)
    .await
    .map_err(storage)?
    .rows_affected()
        == 1;
    if inserted {
        sqlx::query(
            "INSERT INTO msg_archive_state (scope, month, dirty, local_count) VALUES (?, ?, 1, 1)
             ON CONFLICT(scope, month) DO UPDATE SET dirty = 1, local_count = local_count + 1",
        )
        .bind(&row.scope)
        .bind(&row.month)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    } else {
        sqlx::query("UPDATE msg_archive_events SET replayed = 1 WHERE id = ? AND replayed = 0")
            .bind(&row.id)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
    }
    tx.commit().await.map_err(storage)?;
    Ok(inserted)
}

/// The file of a segment the storage holds, read: its events the device
/// lacks are kept (not yet replayed), the order of the file becomes the
/// order of the segment (its records first, as the file has them, dropped
/// ones among them, then the device's own live events), and the file is
/// remembered as the storage's. The segment is dirty when the device holds
/// a live event the file lacks: the storage misses it, whoever wrote last
/// (plan §4.2, «проигравший допишет»). The queue is not touched: a replay
/// of the file is done only once its events went through the ingress
/// ([`set_status`]). Returns the ids kept anew.
pub async fn absorb(store: &Store, scope: &str, month: &str, rows: &[EventRow], remote_hash: &str) -> Result<Vec<String>> {
    let mut tx = store.pool().begin().await.map_err(storage)?;
    ensure_state(&mut tx, scope, month).await?;
    let mut fresh = Vec::new();
    for row in rows {
        let n = sqlx::query(
            "INSERT OR IGNORE INTO msg_archive_events (id, scope, month, at, received_at, kind, msg_id, replayed, event_json)
             VALUES (?, ?, ?, ?, ?, ?, ?, 0, ?)",
        )
        .bind(&row.id)
        .bind(scope)
        .bind(month)
        .bind(row.at)
        .bind(row.received_at)
        .bind(row.kind)
        .bind(&row.msg_id)
        .bind(&row.event_json)
        .execute(&mut *tx)
        .await
        .map_err(storage)?
        .rows_affected();
        if n == 1 {
            fresh.push(row.id.clone());
        }
    }

    // Every row of the segment: live ones in their order, dropped ones too.
    let held: Vec<(String, Option<i64>, bool)> = sqlx::query_as(
        "SELECT id, seq, dropped FROM msg_archive_events WHERE scope = ? AND month = ?
         ORDER BY seq IS NULL, seq, at, id",
    )
    .bind(scope)
    .bind(month)
    .fetch_all(&mut *tx)
    .await
    .map_err(storage)?;
    let had: HashMap<&str, (Option<i64>, bool)> = held.iter().map(|(id, seq, dropped)| (id.as_str(), (*seq, *dropped))).collect();
    let mut placed: HashSet<&str> = HashSet::new();
    let mut order: Vec<&str> = Vec::with_capacity(held.len());
    // The file's records first, in its order, whatever became of them here.
    for row in rows {
        if had.contains_key(row.id.as_str()) && placed.insert(row.id.as_str()) {
            order.push(row.id.as_str());
        }
    }
    // The file still holds what was dropped here (a device behind on the
    // deletion wrote it): the segment is written again without it, or the
    // body would stay in the storage, and come back to a device that reads
    // the file before it learns of the deletion.
    let mut extra_live = rows.iter().any(|r| had.get(r.id.as_str()).is_some_and(|(_, dropped)| *dropped));
    for (id, _, dropped) in &held {
        if placed.contains(id.as_str()) {
            continue;
        }
        if *dropped {
            // Not in the file, not alive: no place in it.
            if had[id.as_str()].0.is_some() {
                sqlx::query("UPDATE msg_archive_events SET seq = NULL WHERE id = ?")
                    .bind(id)
                    .execute(&mut *tx)
                    .await
                    .map_err(storage)?;
            }
            continue;
        }
        extra_live = true;
        placed.insert(id.as_str());
        order.push(id.as_str());
    }
    for (seq, id) in order.iter().enumerate() {
        let seq = seq as i64;
        if had[id].0 != Some(seq) {
            sqlx::query("UPDATE msg_archive_events SET seq = ? WHERE id = ?")
                .bind(seq)
                .bind(*id)
                .execute(&mut *tx)
                .await
                .map_err(storage)?;
        }
    }

    recount(&mut tx, scope, month).await?;
    sqlx::query(
        "UPDATE msg_archive_state SET remote_count = ?, remote_hash = ?, dirty = MAX(dirty, ?)
          WHERE scope = ? AND month = ?",
    )
    .bind(rows.len() as i64)
    .bind(remote_hash)
    .bind(extra_live as i64)
    .bind(scope)
    .bind(month)
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    tx.commit().await.map_err(storage)?;
    Ok(fresh)
}

/// The live events of a segment, the bytes of their records and of the
/// longest one: what its file would be, told before it is made.
pub async fn live_size(store: &Store, scope: &str, month: &str) -> Result<LiveSize> {
    let (count, bytes, longest) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(length(CAST(event_json AS BLOB)) + 4), 0), COALESCE(MAX(length(CAST(event_json AS BLOB))), 0)
           FROM msg_archive_events WHERE scope = ? AND month = ? AND dropped = 0",
    )
    .bind(scope)
    .bind(month)
    .fetch_one(store.pool())
    .await
    .map_err(storage)?;
    Ok(LiveSize { count, bytes, longest })
}

/// What [`live_size`] tells of a segment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveSize {
    pub count: i64,
    /// Of the records with their lengths, without the header of the file.
    pub bytes: i64,
    /// The longest record.
    pub longest: i64,
}

/// [`live_size`] of every dirty segment, `(scope, month, size)`: what a
/// month too large to be written is told by (it stays dirty).
pub async fn dirty_sizes(store: &Store) -> Result<Vec<(String, String, LiveSize)>> {
    let rows: Vec<(String, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT e.scope, e.month, COUNT(*), SUM(length(CAST(e.event_json AS BLOB)) + 4), MAX(length(CAST(e.event_json AS BLOB)))
           FROM msg_archive_state s JOIN msg_archive_events e ON e.scope = s.scope AND e.month = s.month AND e.dropped = 0
          WHERE s.dirty = 1
          GROUP BY e.scope, e.month ORDER BY e.month, e.scope",
    )
    .fetch_all(store.pool())
    .await
    .map_err(storage)?;
    Ok(rows.into_iter().map(|(scope, month, count, bytes, longest)| (scope, month, LiveSize { count, bytes, longest })).collect())
}

/// The segments of the queue refused, `(scope, month, error)`.
pub async fn refused(store: &Store) -> Result<Vec<(String, String, String)>> {
    sqlx::query_as("SELECT scope, month, COALESCE(error, '') FROM msg_archive_queue WHERE status = 'refused' ORDER BY month, scope")
        .fetch_all(store.pool())
        .await
        .map_err(storage)
}

/// The live events of a segment in the order of its file: the events
/// placed before keep their place, the others follow by `(at, id)` and
/// are placed now, so that every later build has them at the same place.
pub async fn order_for_write(store: &Store, scope: &str, month: &str) -> Result<Vec<WriteRow>> {
    let mut tx = store.pool().begin().await.map_err(storage)?;
    let rows: Vec<(String, i64, Option<i64>, String)> = sqlx::query_as(
        "SELECT id, at, seq, event_json FROM msg_archive_events WHERE scope = ? AND month = ? AND dropped = 0
         ORDER BY seq IS NULL, seq, at, id",
    )
    .bind(scope)
    .bind(month)
    .fetch_all(&mut *tx)
    .await
    .map_err(storage)?;
    let mut next = rows.iter().filter_map(|r| r.2).max().map_or(0, |m| m + 1);
    for (id, _, seq, _) in &rows {
        if seq.is_none() {
            sqlx::query("UPDATE msg_archive_events SET seq = ? WHERE id = ?")
                .bind(next)
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(storage)?;
            next += 1;
        }
    }
    tx.commit().await.map_err(storage)?;
    Ok(rows.into_iter().map(|(id, at, _, event_json)| WriteRow { id, at, event_json }).collect())
}

/// The ids of a segment and whether compaction dropped each.
pub async fn ids(store: &Store, scope: &str, month: &str) -> Result<Vec<(String, bool)>> {
    sqlx::query_as("SELECT id, dropped FROM msg_archive_events WHERE scope = ? AND month = ?")
        .bind(scope)
        .bind(month)
        .fetch_all(store.pool())
        .await
        .map_err(storage)
}

/// The ids a file of `count` records holds, by their places: the first
/// `count` places are the file's, as [`absorb`] and [`order_for_write`]
/// give them (dropped rows of the file among them).
pub async fn ids_in_file(store: &Store, scope: &str, month: &str, count: i64) -> Result<Vec<String>> {
    sqlx::query_scalar("SELECT id FROM msg_archive_events WHERE scope = ? AND month = ? AND seq IS NOT NULL AND seq < ?")
        .bind(scope)
        .bind(month)
        .bind(count)
        .fetch_all(store.pool())
        .await
        .map_err(storage)
}

/// The events of a segment merged from the storage and not yet through
/// the ingress, `(id, event_json)` by `(at, id)`: the order a reader
/// replays in (`messenger-wire.md` §8).
pub async fn unreplayed(store: &Store, scope: &str, month: &str) -> Result<Vec<(String, String)>> {
    sqlx::query_as(
        "SELECT id, event_json FROM msg_archive_events
         WHERE scope = ? AND month = ? AND replayed = 0 AND dropped = 0 ORDER BY at, id",
    )
    .bind(scope)
    .bind(month)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// Whether a segment holds events not yet through the ingress.
pub async fn has_unreplayed(store: &Store, scope: &str, month: &str) -> Result<bool> {
    let n: i64 = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM msg_archive_events WHERE scope = ? AND month = ? AND replayed = 0 AND dropped = 0)",
    )
    .bind(scope)
    .bind(month)
    .fetch_one(store.pool())
    .await
    .map_err(storage)?;
    Ok(n != 0)
}

/// The events went through the ingress: one transaction for the batch.
pub async fn mark_replayed(store: &Store, ids: &[String]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut tx = store.pool().begin().await.map_err(storage)?;
    for id in ids {
        sqlx::query("UPDATE msg_archive_events SET replayed = 1 WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
    }
    tx.commit().await.map_err(storage)
}

const STATE_COLUMNS: &str =
    "scope, month, dirty, local_count, written_count, written_hash, written_at, remote_count, remote_hash";

pub async fn state(store: &Store, scope: &str, month: &str) -> Result<Option<StateRow>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {STATE_COLUMNS} FROM msg_archive_state WHERE scope = ? AND month = ?"
    )))
    .bind(scope)
    .bind(month)
    .fetch_optional(store.pool())
    .await
    .map_err(storage)
}

/// Every segment with events the storage may lack.
pub async fn dirty(store: &Store) -> Result<Vec<StateRow>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {STATE_COLUMNS} FROM msg_archive_state WHERE dirty = 1 ORDER BY month DESC, scope"
    )))
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// The storage holds a file of `count` events with `hash` that this device
/// wrote, or found to be its own. `dirty` says whether the device holds
/// more than that file by now (the caller compares the sets, never the
/// counts: an event that came and one dropped since the build cancel out
/// in a count).
pub async fn set_written(store: &Store, scope: &str, month: &str, count: i64, hash: &str, at: i64, dirty: bool) -> Result<()> {
    let mut tx = store.pool().begin().await.map_err(storage)?;
    ensure_state(&mut tx, scope, month).await?;
    sqlx::query(
        "UPDATE msg_archive_state
            SET written_count = ?1, written_hash = ?2, written_at = ?3, remote_count = ?1, remote_hash = ?2, dirty = ?4
          WHERE scope = ?5 AND month = ?6",
    )
    .bind(count)
    .bind(hash)
    .bind(at)
    .bind(dirty as i64)
    .bind(scope)
    .bind(month)
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    tx.commit().await.map_err(storage)
}

/// The storage's file of a segment could not be merged (a format this
/// build does not read): the segment waits as it is, not due until the
/// flush interval passes again; a closed month is left to the queue.
pub async fn defer(store: &Store, scope: &str, month: &str, now: i64) -> Result<()> {
    sqlx::query("UPDATE msg_archive_state SET written_at = ? WHERE scope = ? AND month = ?")
        .bind(now)
        .bind(scope)
        .bind(month)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// Compaction (`internal/messenger-archive-plan.md` §4.2, D5): the events
/// of a segment whose message was deleted for everyone, the edits of such
/// a message, and the personal messages of a deleted chat lose their body.
/// A personal message is found by its rumor id (`msg_messages.id`), a
/// group event by its wire id (`msg_messages.wire_id`: the id of a group
/// message is the one sealed inside). The deletion itself stays: a device
/// that still holds the message learns of it from the segment. The rows
/// stay, without their body, so that a merge does not bring them back. The
/// segment is dirty when anything was dropped. Returns how many were.
pub async fn drop_dead(store: &Store, scope: &str, month: &str) -> Result<u64> {
    let mut tx = store.pool().begin().await.map_err(storage)?;
    let dropped = sqlx::query(
        "WITH gone(id) AS (SELECT id FROM msg_messages WHERE deleted_at IS NOT NULL),
              dead(id, wire_id) AS (
                SELECT id, wire_id FROM msg_messages WHERE deleted_at IS NOT NULL
                UNION ALL
                SELECT id, wire_id FROM msg_messages WHERE content_type = 'edit' AND target_id IN (SELECT id FROM gone))
         UPDATE msg_archive_events SET dropped = 1, event_json = ''
          WHERE scope = ?1 AND month = ?2 AND dropped = 0 AND (
                (msg_id IS NOT NULL AND (msg_id IN (SELECT id FROM dead)
                                         OR msg_id IN (SELECT message_id FROM msg_chat_deleted_ids)))
             OR id IN (SELECT wire_id FROM dead WHERE wire_id IS NOT NULL))",
    )
    .bind(scope)
    .bind(month)
    .execute(&mut *tx)
    .await
    .map_err(storage)?
    .rows_affected();
    if dropped > 0 {
        recount(&mut tx, scope, month).await?;
        sqlx::query("UPDATE msg_archive_state SET dirty = 1 WHERE scope = ? AND month = ?")
            .bind(scope)
            .bind(month)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
    }
    tx.commit().await.map_err(storage)?;
    Ok(dropped)
}

/// Compaction of the notes of my own devices that are snapshots of a state
/// (plan D17): of the events of a segment `key_of` gives one key, the
/// newest by `(at, id)` stays and the rest are dropped like deleted ones
/// (replaying the newest alone gives the same state). `key_of` reads the
/// record as the file holds it and names the state it is a snapshot of,
/// or nothing for an event that is kept whatever came after it. Only
/// records that may be such a note are read (`own.` in the text). How
/// many were dropped; the segment is dirty then.
pub async fn drop_superseded(store: &Store, scope: &str, month: &str, key_of: &(dyn Fn(&str) -> Option<String> + Sync)) -> Result<u64> {
    let rows: Vec<(String, i64, String)> = sqlx::query_as(
        "SELECT id, at, event_json FROM msg_archive_events
          WHERE scope = ? AND month = ? AND dropped = 0 AND event_json LIKE '%own.%'",
    )
    .bind(scope)
    .bind(month)
    .fetch_all(store.pool())
    .await
    .map_err(storage)?;
    let mut newest: HashMap<String, (i64, String)> = HashMap::new();
    let mut keyed: Vec<(String, i64, String)> = Vec::new();
    for (id, at, event_json) in rows {
        let Some(key) = key_of(&event_json) else { continue };
        let candidate = (at, id);
        match newest.get(&key) {
            Some(best) if *best >= candidate => {}
            _ => {
                newest.insert(key.clone(), candidate.clone());
            }
        }
        keyed.push((key, candidate.0, candidate.1));
    }
    let gone: Vec<String> = keyed
        .into_iter()
        .filter(|(key, at, id)| newest.get(key).is_some_and(|best| *best != (*at, id.clone())))
        .map(|(_, _, id)| id)
        .collect();
    if gone.is_empty() {
        return Ok(0);
    }
    let mut tx = store.pool().begin().await.map_err(storage)?;
    let mut dropped = 0;
    for id in &gone {
        dropped += sqlx::query("UPDATE msg_archive_events SET dropped = 1, event_json = '' WHERE id = ? AND dropped = 0")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(storage)?
            .rows_affected();
    }
    if dropped > 0 {
        recount(&mut tx, scope, month).await?;
        sqlx::query("UPDATE msg_archive_state SET dirty = 1 WHERE scope = ? AND month = ?")
            .bind(scope)
            .bind(month)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
    }
    tx.commit().await.map_err(storage)?;
    Ok(dropped)
}

/// The personal messages that bring a group I forgot back (`group.welcome`,
/// `group.keys`, of `group_id`, dated at or before `upto`) are dropped from
/// every personal scope, as compaction drops what was deleted: the rows
/// stay without their body, so that a merge of a file of before does not
/// bring them back, and each segment that lost one is dirty, written again
/// without them. How many were dropped.
pub(crate) async fn drop_group_protocol_in(tx: &mut Transaction<'_, Sqlite>, group_id: &str, upto: i64) -> Result<u64> {
    // The text is narrowed by SQL, the message is told by its envelope: a
    // personal text that only names the group and these words stays.
    let candidates: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT id, scope, month, event_json FROM msg_archive_events
          WHERE (scope = 'dm' OR scope LIKE 'dm@%') AND dropped = 0 AND at <= ?2
            AND instr(event_json, ?1) > 0
            AND (instr(event_json, 'group.welcome') > 0 OR instr(event_json, 'group.keys') > 0)",
    )
    .bind(group_id)
    .bind(upto)
    .fetch_all(&mut **tx)
    .await
    .map_err(storage)?;
    let hit: Vec<(String, String, String)> = candidates
        .into_iter()
        .filter(|(_, _, _, json)| is_group_protocol(json, group_id))
        .map(|(id, scope, month, _)| (id, scope, month))
        .collect();
    let mut segments: Vec<(String, String)> = Vec::new();
    for (id, scope, month) in &hit {
        sqlx::query("UPDATE msg_archive_events SET dropped = 1, event_json = '' WHERE id = ?")
            .bind(id)
            .execute(&mut **tx)
            .await
            .map_err(storage)?;
        if !segments.contains(&(scope.clone(), month.clone())) {
            segments.push((scope.clone(), month.clone()));
        }
    }
    for (scope, month) in &segments {
        recount(tx, scope, month).await?;
        sqlx::query("UPDATE msg_archive_state SET dirty = 1 WHERE scope = ? AND month = ?")
            .bind(scope)
            .bind(month)
            .execute(&mut **tx)
            .await
            .map_err(storage)?;
    }
    Ok(hit.len() as u64)
}

/// Whether a personal message of the archive (`ArchivedEvent` of the form
/// `dm`, its rumor in `body.inbound.content`) is a `group.welcome` or a
/// `group.keys` of `group_id`.
fn is_group_protocol(event_json: &str, group_id: &str) -> bool {
    let Ok(event) = serde_json::from_str::<serde_json::Value>(event_json) else { return false };
    let Some(content) = event.pointer("/body/inbound/content").and_then(serde_json::Value::as_str) else { return false };
    let Ok(envelope) = messenger_core::Envelope::parse(content) else { return false };
    matches!(envelope.t.as_str(), "group.welcome" | "group.keys") && envelope.str_field("group_id") == Some(group_id)
}

/// The high-water mark of a scope: the latest `at`, each capped at the
/// time the event was received plus `skew`. `at` is the sender's clock
/// (the rumor's `created_at`): one event dated far ahead would otherwise
/// move the relay catch-up of every device past the present for good.
pub async fn high_water(store: &Store, scope: &str, skew: i64) -> Result<Option<i64>> {
    sqlx::query_scalar("SELECT MAX(MIN(at, received_at + ?)) FROM msg_archive_events WHERE scope = ?")
        .bind(skew)
        .bind(scope)
        .fetch_one(store.pool())
        .await
        .map_err(storage)
}

/// The storage is another than the one the segments were written to (a
/// new binding): no segment counts as written or read, and every one with
/// live events is dirty. The events and their places in the files stay.
pub async fn forget_written(store: &Store) -> Result<()> {
    sqlx::query(
        "UPDATE msg_archive_state
            SET written_count = NULL, written_hash = NULL, written_at = NULL,
                remote_count = NULL, remote_hash = NULL,
                dirty = (local_count > 0)",
    )
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// Forget a scope: its live events, segments and queue. The rows
/// compaction dropped stay as tombstones: a group joined again, or a
/// server added again, merges the storage's files, and what was deleted
/// must not come back with them. Returns the months it had, newest first.
pub async fn remove_scope(store: &Store, scope: &str) -> Result<Vec<String>> {
    let mut tx = store.pool().begin().await.map_err(storage)?;
    let months = remove_scope_in(&mut tx, scope).await?;
    tx.commit().await.map_err(storage)?;
    Ok(months)
}

/// [`remove_scope`] inside a transaction of the caller's (the removal of a
/// private server takes its scope with it).
pub async fn remove_scope_in(tx: &mut sqlx::SqliteConnection, scope: &str) -> Result<Vec<String>> {
    let months: Vec<String> = sqlx::query_scalar(
        "SELECT month FROM msg_archive_events WHERE scope = ?1 AND dropped = 0
         UNION SELECT month FROM msg_archive_state WHERE scope = ?1
         UNION SELECT month FROM msg_archive_queue WHERE scope = ?1
         ORDER BY 1 DESC",
    )
    .bind(scope)
    .fetch_all(&mut *tx)
    .await
    .map_err(storage)?;
    sqlx::query("DELETE FROM msg_archive_events WHERE scope = ? AND dropped = 0")
        .bind(scope)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    sqlx::query("UPDATE msg_archive_events SET seq = NULL WHERE scope = ?")
        .bind(scope)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    for table in ["msg_archive_state", "msg_archive_queue"] {
        sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table} WHERE scope = ?")))
            .bind(scope)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
    }
    // The mark of the forgetting, for the handler of the sync entities:
    // its positions of the scope's segments live in another database and
    // are reset when it next runs (`take_forgotten`), whichever way the
    // scope went — by hand here, or by the snapshot of another device.
    sqlx::query(
        "INSERT INTO msg_archive_forgotten (scope, at) VALUES (?, CAST(strftime('%s', 'now') AS INTEGER))
         ON CONFLICT(scope) DO UPDATE SET at = excluded.at",
    )
    .bind(scope)
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    Ok(months)
}

/// The scopes [`remove_scope_in`] forgot since the last call, oldest
/// first, taken once: the handler of the sync entities resets what it
/// holds of their segments.
pub async fn take_forgotten(store: &Store) -> Result<Vec<String>> {
    let mut tx = store.pool().begin_with("BEGIN IMMEDIATE").await.map_err(storage)?;
    let scopes: Vec<String> = sqlx::query_scalar("SELECT scope FROM msg_archive_forgotten ORDER BY at, scope")
        .fetch_all(&mut *tx)
        .await
        .map_err(storage)?;
    sqlx::query("DELETE FROM msg_archive_forgotten").execute(&mut *tx).await.map_err(storage)?;
    tx.commit().await.map_err(storage)?;
    Ok(scopes)
}

/// Put a version of a segment in the queue as pending: a new row, or the
/// row of the segment when it named another version or, with `also_done`,
/// when it was done (its events are to be replayed again from what is
/// held). `true` when the row changed.
pub async fn upsert_pending(store: &Store, row: &QueueRow, now: i64, also_done: bool) -> Result<bool> {
    let n = sqlx::query(
        "INSERT INTO msg_archive_queue (entity_id, scope, month, count, size, hash, status, priority, error, build, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, 'pending', ?, NULL, '', ?)
         ON CONFLICT(entity_id) DO UPDATE SET
            count = excluded.count, size = excluded.size, hash = excluded.hash, status = 'pending',
            priority = excluded.priority, error = NULL, build = '', updated_at = excluded.updated_at
          WHERE msg_archive_queue.hash <> excluded.hash OR (?9 AND msg_archive_queue.status = 'done')",
    )
    .bind(&row.entity_id)
    .bind(&row.scope)
    .bind(&row.month)
    .bind(row.count)
    .bind(row.size)
    .bind(&row.hash)
    .bind(row.priority)
    .bind(now)
    .bind(also_done)
    .execute(store.pool())
    .await
    .map_err(storage)?
    .rows_affected();
    Ok(n == 1)
}

const QUEUE_COLUMNS: &str = "entity_id, scope, month, count, size, hash, status, priority, error, build";

/// The next pending segment: those of `open_scope` first, then by
/// priority.
pub async fn next(store: &Store, open_scope: Option<&str>) -> Result<Option<QueueRow>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {QUEUE_COLUMNS} FROM msg_archive_queue
          WHERE status = 'pending'
          ORDER BY (scope = ?1) DESC, priority DESC, entity_id LIMIT 1"
    )))
    .bind(open_scope.unwrap_or(""))
    .fetch_optional(store.pool())
    .await
    .map_err(storage)
}

pub async fn queue_row(store: &Store, entity_id: &str) -> Result<Option<QueueRow>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {QUEUE_COLUMNS} FROM msg_archive_queue WHERE entity_id = ?")))
        .bind(entity_id)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)
}

/// The status of the version `hash` of a segment; a row that names another
/// version by now (a newer op took it) is left as it is.
pub async fn set_status(store: &Store, entity_id: &str, hash: &str, status: &str, error: Option<&str>, build: &str, now: i64) -> Result<()> {
    sqlx::query("UPDATE msg_archive_queue SET status = ?, error = ?, build = ?, updated_at = ? WHERE entity_id = ? AND hash = ?")
        .bind(status)
        .bind(error)
        .bind(build)
        .bind(now)
        .bind(entity_id)
        .bind(hash)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// Segments of `from` go back to pending; how many.
pub async fn retry(store: &Store, from: &str, now: i64) -> Result<u64> {
    Ok(sqlx::query("UPDATE msg_archive_queue SET status = 'pending', error = NULL, updated_at = ? WHERE status = ?")
        .bind(now)
        .bind(from)
        .execute(store.pool())
        .await
        .map_err(storage)?
        .rows_affected())
}

/// Segments refused by another build than `build` go back to pending.
pub async fn retry_refused(store: &Store, build: &str, now: i64) -> Result<u64> {
    Ok(sqlx::query(
        "UPDATE msg_archive_queue SET status = 'pending', error = NULL, updated_at = ? WHERE status = 'refused' AND build <> ?",
    )
    .bind(now)
    .bind(build)
    .execute(store.pool())
    .await
    .map_err(storage)?
    .rows_affected())
}

/// Forget the segments done, once a restore is over: the next one counts
/// from zero.
pub async fn clear_done(store: &Store) -> Result<()> {
    sqlx::query("DELETE FROM msg_archive_queue WHERE status = 'done'")
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn progress(store: &Store) -> Result<QueueProgress> {
    let (done, total, bytes_done, bytes_total): (i64, i64, i64, i64) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT COALESCE(SUM(status = 'done'), 0), COUNT(*),
                COALESCE(SUM(CASE WHEN status = 'done' THEN size ELSE 0 END), 0), COALESCE(SUM(size), 0)
           FROM msg_archive_queue WHERE status IN {OPEN_STATUSES}"
    )))
    .fetch_one(store.pool())
    .await
    .map_err(storage)?;
    Ok(QueueProgress { done, total, bytes_done, bytes_total })
}

/// The segments the restore does not wait for: parked and refused.
pub async fn blocked(store: &Store) -> Result<Vec<String>> {
    sqlx::query_scalar("SELECT entity_id FROM msg_archive_queue WHERE status IN ('parked', 'refused') ORDER BY entity_id")
        .fetch_all(store.pool())
        .await
        .map_err(storage)
}

/// The scopes with a segment still to replay (pending, failed or parked):
/// what an open chat of them shows as «earlier messages are coming».
pub async fn restoring_scopes(store: &Store) -> Result<Vec<String>> {
    sqlx::query_scalar(
        "SELECT DISTINCT scope FROM msg_archive_queue WHERE status IN ('pending', 'failed', 'parked') ORDER BY scope",
    )
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scope removed leaves a mark, one per scope, until the handler
    /// takes it.
    #[tokio::test]
    async fn a_removed_scope_is_marked_forgotten_until_taken() {
        let s = Store::open_in_memory().await.unwrap();
        assert!(take_forgotten(&s).await.unwrap().is_empty());
        remove_scope(&s, "dm@s1").await.unwrap();
        remove_scope(&s, "dm@s1").await.unwrap();
        remove_scope(&s, "group:g1").await.unwrap();
        assert_eq!(take_forgotten(&s).await.unwrap(), ["dm@s1", "group:g1"]);
        assert!(take_forgotten(&s).await.unwrap().is_empty(), "taken once");
    }

    fn row(id: &str, at: i64) -> EventRow {
        EventRow {
            id: id.into(),
            scope: "dm".into(),
            month: "2026-11".into(),
            at,
            received_at: at,
            kind: 1059,
            msg_id: Some(format!("r{id}")),
            event_json: format!("{{\"id\":\"{id}\"}}"),
        }
    }

    /// A group forgotten: the personal messages that would bring it back
    /// (its welcome and keys, dated up to the forgetting) lose their body
    /// and their segment is dirty; a message of another group, a later
    /// welcome and a text that names the group (and the words of its
    /// protocol) stay. The mark only rises.
    #[tokio::test]
    async fn a_forgotten_group_takes_its_welcomes_out_of_the_archive() {
        let s = Store::open_in_memory().await.unwrap();
        let g = "ab".repeat(32);
        let other = "cd".repeat(32);
        let body = |t: &str, group: &str| format!(r#"{{"body":{{"inbound":{{"content":"{{\"v\":1,\"t\":\"{t}\",\"group_id\":\"{group}\"}}"}}}}}}"#);
        let said = |text: &str| format!(r#"{{"body":{{"inbound":{{"content":"{{\"v\":1,\"t\":\"text\",\"text\":\"{text}\"}}"}}}}}}"#);
        let put = |id: &str, at: i64, json: String| EventRow { event_json: json, ..row(id, at) };
        for r in [
            put("welcome", 10, body("group.welcome", &g)),
            put("keys", 11, body("group.keys", &g)),
            put("theirs", 12, body("group.welcome", &other)),
            put("later", 30, body("group.welcome", &g)),
            put("text", 13, format!(r#"{{"content":"see {g}"}}"#)),
            put("example", 14, said(&format!("Пример group.welcome для группы {g}, и group.keys"))),
        ] {
            record(&s, &r).await.unwrap();
        }
        set_written(&s, "dm", "2026-11", 5, "h", 1, false).await.unwrap();

        assert!(crate::groups::mark_forgotten(&s, &g, 20).await.unwrap());

        let live: Vec<String> = ids(&s, "dm", "2026-11").await.unwrap().into_iter().filter(|(_, d)| !d).map(|(id, _)| id).collect();
        assert_eq!(live.iter().map(String::as_str).collect::<HashSet<_>>(), HashSet::from(["theirs", "later", "text", "example"]));
        let st = state(&s, "dm", "2026-11").await.unwrap().unwrap();
        assert!(st.dirty, "written again without them");
        assert_eq!(st.local_count, 4);
        assert_eq!(crate::groups::forgotten_at(&s, &g).await.unwrap(), Some(20));
        assert!(!crate::groups::mark_forgotten(&s, &g, 15).await.unwrap(), "never lowered");
        assert_eq!(crate::groups::forgotten(&s).await.unwrap(), vec![(g.clone(), 20)]);
    }

    #[tokio::test]
    async fn record_keeps_a_new_event_once_and_dirties_its_segment() {
        let s = Store::open_in_memory().await.unwrap();
        assert!(record(&s, &row("a", 2)).await.unwrap());
        assert!(!record(&s, &row("a", 2)).await.unwrap());
        assert!(record(&s, &row("b", 1)).await.unwrap());
        let st = state(&s, "dm", "2026-11").await.unwrap().unwrap();
        assert!(st.dirty);
        assert_eq!(st.local_count, 2);
        let order = order_for_write(&s, "dm", "2026-11").await.unwrap();
        assert_eq!(order.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        set_written(&s, "dm", "2026-11", 2, "h", 10, false).await.unwrap();
        assert!(!state(&s, "dm", "2026-11").await.unwrap().unwrap().dirty);
        assert_eq!(ids_in_file(&s, "dm", "2026-11", 2).await.unwrap().len(), 2);
    }

    /// A file of a device behind on a deletion still holds the event that
    /// was dropped here: the row stays dropped, and the segment is dirty,
    /// written again without it.
    #[tokio::test]
    async fn absorb_dirties_a_segment_whose_file_holds_what_was_dropped() {
        let s = Store::open_in_memory().await.unwrap();
        record(&s, &row("a", 1)).await.unwrap();
        record(&s, &row("b", 2)).await.unwrap();
        sqlx::query("UPDATE msg_archive_events SET dropped = 1, event_json = '' WHERE id = 'a'").execute(s.pool()).await.unwrap();
        set_written(&s, "dm", "2026-11", 1, "mine", 10, false).await.unwrap();
        assert!(!state(&s, "dm", "2026-11").await.unwrap().unwrap().dirty);

        absorb(&s, "dm", "2026-11", &[row("a", 1), row("b", 2)], "behind").await.unwrap();

        assert!(state(&s, "dm", "2026-11").await.unwrap().unwrap().dirty);
        assert!(ids(&s, "dm", "2026-11").await.unwrap().contains(&("a".to_string(), true)), "still dropped");
        let order = order_for_write(&s, "dm", "2026-11").await.unwrap();
        assert_eq!(order.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["b"]);
    }

    /// A file that lacks a live event of this device leaves the segment
    /// dirty; one that holds everything does not; the file's records take
    /// the first places, the device's own follow.
    #[tokio::test]
    async fn absorb_dirties_the_segment_the_file_lacks_part_of() {
        let s = Store::open_in_memory().await.unwrap();
        record(&s, &row("a", 1)).await.unwrap();
        record(&s, &row("b", 2)).await.unwrap();
        set_written(&s, "dm", "2026-11", 2, "mine", 10, false).await.unwrap();
        assert!(!state(&s, "dm", "2026-11").await.unwrap().unwrap().dirty);
        // The storage's file: b and c, not a.
        let fresh = absorb(&s, "dm", "2026-11", &[row("b", 2), row("c", 3)], "theirs").await.unwrap();
        assert_eq!(fresh, ["c"]);
        let st = state(&s, "dm", "2026-11").await.unwrap().unwrap();
        assert!(st.dirty, "a is missing from the storage");
        assert_eq!((st.remote_count, st.remote_hash.as_deref()), (Some(2), Some("theirs")));
        assert_eq!(ids_in_file(&s, "dm", "2026-11", 2).await.unwrap(), ["b", "c"]);
        let order: Vec<String> = order_for_write(&s, "dm", "2026-11").await.unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(order, ["b", "c", "a"]);
        assert!(has_unreplayed(&s, "dm", "2026-11").await.unwrap());
        mark_replayed(&s, &["c".to_string()]).await.unwrap();
        assert!(!has_unreplayed(&s, "dm", "2026-11").await.unwrap());

        // A file that holds all: not dirtied (it was dirty: it stays so
        // until the caller says what the storage holds).
        set_written(&s, "dm", "2026-11", 3, "union", 11, false).await.unwrap();
        absorb(&s, "dm", "2026-11", &[row("b", 2), row("c", 3), row("a", 1)], "union").await.unwrap();
        assert!(!state(&s, "dm", "2026-11").await.unwrap().unwrap().dirty);
    }

    #[tokio::test]
    async fn a_deleted_message_and_its_edits_are_dropped_and_the_deletion_stays() {
        let s = Store::open_in_memory().await.unwrap();
        for (id, ct, target, deleted) in [
            ("ra", "text", None, Some(5)),
            ("re", "edit", Some("ra"), None),
            ("rd", "delete", Some("ra"), None),
            ("rk", "text", None, None),
        ] {
            sqlx::query(
                "INSERT INTO msg_messages (id, chat_id, direction, status, content_type, envelope_json, sender_pubkey, target_id, created_at, received_at, deleted_at)
                 VALUES (?, 'dm:bb', 'in', 'received', ?, '{}', 'bb', ?, 1, 1, ?)",
            )
            .bind(id)
            .bind(ct)
            .bind(target)
            .bind(deleted)
            .execute(s.pool())
            .await
            .unwrap();
        }
        for id in ["a", "e", "d", "k"] {
            record(&s, &row(id, 1)).await.unwrap();
        }
        set_written(&s, "dm", "2026-11", 4, "h", 10, false).await.unwrap();
        assert_eq!(drop_dead(&s, "dm", "2026-11").await.unwrap(), 2);
        let live: Vec<String> = order_for_write(&s, "dm", "2026-11").await.unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(live, ["d", "k"]);
        let st = state(&s, "dm", "2026-11").await.unwrap().unwrap();
        assert!(st.dirty);
        assert_eq!(st.local_count, 2);
        // The dropped rows stay as tombstones: a merge does not bring them back.
        assert!(absorb(&s, "dm", "2026-11", &[row("a", 1)], "x").await.unwrap().is_empty());
        assert_eq!(drop_dead(&s, "dm", "2026-11").await.unwrap(), 0);
        // A dropped row in the file keeps its place in it; one not in it has none.
        assert_eq!(ids_in_file(&s, "dm", "2026-11", 1).await.unwrap(), ["a"]);
        // Forgetting the scope keeps the tombstones.
        remove_scope(&s, "dm").await.unwrap();
        let left: Vec<(String, bool)> = ids(&s, "dm", "2026-11").await.unwrap();
        assert!(left.iter().all(|(_, dropped)| *dropped) && left.len() == 2, "{left:?}");
        assert!(state(&s, "dm", "2026-11").await.unwrap().is_none());
    }

    /// An event dated far ahead does not move the mark past the present.
    #[tokio::test]
    async fn the_high_water_mark_is_capped_by_the_time_received() {
        let s = Store::open_in_memory().await.unwrap();
        record(&s, &row("a", 1_000)).await.unwrap();
        assert_eq!(high_water(&s, "dm", 300).await.unwrap(), Some(1_000));
        let mut future = row("z", 4_102_444_800); // 2100-01-01
        future.received_at = 2_000;
        record(&s, &future).await.unwrap();
        assert_eq!(high_water(&s, "dm", 300).await.unwrap(), Some(2_300));
        assert_eq!(high_water(&s, "group:g", 300).await.unwrap(), None);
    }

    /// The status of a version changes only for that version; parked and
    /// refused segments are outside the count of the restore.
    #[tokio::test]
    async fn the_queue_is_kept_by_version() {
        let s = Store::open_in_memory().await.unwrap();
        let q = |hash: &str| QueueRow {
            entity_id: "o/dm/2026-11".into(),
            scope: "dm".into(),
            month: "2026-11".into(),
            count: 1,
            size: 10,
            hash: hash.into(),
            status: String::new(),
            priority: 1,
            error: None,
            build: String::new(),
        };
        assert!(upsert_pending(&s, &q("h1"), 1, false).await.unwrap());
        assert!(!upsert_pending(&s, &q("h1"), 2, false).await.unwrap(), "the same version");
        set_status(&s, "o/dm/2026-11", "h0", QUEUE_DONE, None, "", 3).await.unwrap();
        assert_eq!(next(&s, None).await.unwrap().unwrap().status, "pending", "another version's done is not this one's");
        set_status(&s, "o/dm/2026-11", "h1", QUEUE_DONE, None, "", 3).await.unwrap();
        assert!(next(&s, None).await.unwrap().is_none());
        assert!(upsert_pending(&s, &q("h1"), 4, true).await.unwrap(), "done, but to be replayed from what is held");
        assert!(upsert_pending(&s, &q("h2"), 5, false).await.unwrap(), "a newer version");
        assert_eq!(queue_row(&s, "o/dm/2026-11").await.unwrap().unwrap().hash, "h2");

        set_status(&s, "o/dm/2026-11", "h2", QUEUE_PARKED, Some("group_unknown"), "", 6).await.unwrap();
        assert_eq!(progress(&s).await.unwrap().total, 0, "parked: not counted");
        assert_eq!(blocked(&s).await.unwrap(), ["o/dm/2026-11"]);
        assert_eq!(restoring_scopes(&s).await.unwrap(), ["dm"]);
        assert_eq!(retry(&s, QUEUE_PARKED, 7).await.unwrap(), 1);
        set_status(&s, "o/dm/2026-11", "h2", QUEUE_REFUSED, Some("version 2"), "1.0", 8).await.unwrap();
        assert_eq!(retry_refused(&s, "1.0", 9).await.unwrap(), 0, "the same build");
        assert_eq!(retry_refused(&s, "1.1", 9).await.unwrap(), 1, "another build tries again");
        assert_eq!(progress(&s).await.unwrap().total, 1);
    }
}
