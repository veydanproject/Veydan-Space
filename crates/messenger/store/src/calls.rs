// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_calls`: the record of every call, one row by its id (migration
//! 020). The invitation, the answer and the end of a call all name the id,
//! from whichever device they come, so they meet in one row. The chat
//! shows a system row of `msg_messages` beside it (`messages::set_system`).

use crate::{storage, Store};
use messenger_core::Result;

pub const DIR_IN: &str = "in";
pub const DIR_OUT: &str = "out";

pub const MEDIA_AUDIO: &str = "audio";
pub const MEDIA_VIDEO: &str = "video";

/// Nobody took the call.
pub const OUTCOME_MISSED: &str = "missed";
pub const OUTCOME_DECLINED: &str = "declined";
pub const OUTCOME_BUSY: &str = "busy";
/// It was talked, and ended.
pub const OUTCOME_ENDED: &str = "ended";
/// No connection came, or it was lost and not restored.
pub const OUTCOME_FAILED: &str = "failed";

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct CallRow {
    pub call_id: String,
    pub chat_id: String,
    pub peer: String,
    pub direction: String,
    pub media: String,
    pub started_at: i64,
    pub answered_at: Option<i64>,
    pub ended_at: Option<i64>,
    /// `None` while the call is under way.
    pub outcome: Option<String>,
    pub via_relay: bool,
}

impl CallRow {
    /// How long it was talked, when it was.
    pub fn duration_secs(&self) -> Option<i64> {
        match (self.answered_at, self.ended_at) {
            (Some(a), Some(e)) if e >= a => Some(e - a),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct NewCall {
    pub call_id: String,
    pub chat_id: String,
    pub peer: String,
    pub direction: String,
    pub media: String,
    pub started_at: i64,
}

const COLS: &str = "call_id, chat_id, peer, direction, media, started_at, answered_at, ended_at, outcome, via_relay";

/// Insert; `false` when the call is already on record (a copy of its
/// invitation from another device).
pub async fn insert(store: &Store, c: &NewCall) -> Result<bool> {
    let res = sqlx::query(
        "INSERT OR IGNORE INTO msg_calls (call_id, chat_id, peer, direction, media, started_at, answered_at, ended_at, outcome, via_relay)
         VALUES (?, ?, ?, ?, ?, ?, NULL, NULL, NULL, 0)",
    )
    .bind(&c.call_id)
    .bind(&c.chat_id)
    .bind(&c.peer)
    .bind(&c.direction)
    .bind(&c.media)
    .bind(c.started_at)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(res.rows_affected() == 1)
}

pub async fn get(store: &Store, call_id: &str) -> Result<Option<CallRow>> {
    sqlx::query_as::<_, CallRow>(sqlx::AssertSqlSafe(format!("SELECT {COLS} FROM msg_calls WHERE call_id = ?")))
        .bind(call_id)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)
}

/// The calls of a chat, newest first, at most `limit`.
pub async fn list(store: &Store, chat_id: &str, limit: i64) -> Result<Vec<CallRow>> {
    sqlx::query_as::<_, CallRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM msg_calls WHERE chat_id = ? ORDER BY started_at DESC, call_id DESC LIMIT ?"
    )))
    .bind(chat_id)
    .bind(limit)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// The call was taken at `at`. Only the first word of it counts.
pub async fn set_answered(store: &Store, call_id: &str, at: i64) -> Result<()> {
    sqlx::query("UPDATE msg_calls SET answered_at = ? WHERE call_id = ? AND answered_at IS NULL")
        .bind(at)
        .bind(call_id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn set_via_relay(store: &Store, call_id: &str, via_relay: bool) -> Result<()> {
    sqlx::query("UPDATE msg_calls SET via_relay = ? WHERE call_id = ?")
        .bind(via_relay)
        .bind(call_id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// The call is over with `outcome`. The first end holds: a copy from
/// another device that comes later changes nothing.
pub async fn finish(store: &Store, call_id: &str, outcome: &str, ended_at: i64) -> Result<bool> {
    let res = sqlx::query("UPDATE msg_calls SET outcome = ?, ended_at = ? WHERE call_id = ? AND outcome IS NULL")
        .bind(outcome)
        .bind(ended_at)
        .bind(call_id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(res.rows_affected() == 1)
}

/// Another word of the end, whatever was written: a device that never
/// rang wrote "missed", and the device that took the call says otherwise.
pub async fn set_outcome(store: &Store, call_id: &str, outcome: &str, ended_at: i64) -> Result<()> {
    sqlx::query("UPDATE msg_calls SET outcome = ?, ended_at = ? WHERE call_id = ?")
        .bind(outcome)
        .bind(ended_at)
        .bind(call_id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

// ─── Group calls (migration 021) ───────────────────────────────────────────

/// A call between two people (the default of every row).
pub const KIND_DM: &str = "dm";
/// A call of a group, on an SFU room.
pub const KIND_GROUP: &str = "group";

/// What a row says of a group call beyond [`CallRow`].
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct GroupCallInfo {
    pub kind: String,
    /// Who made the room, hex.
    pub started_by: Option<String>,
    /// How many people were seen in the room, me included.
    pub participants: i64,
}

/// Insert the record of a group call: `chat_id` the group's chat, `peer`
/// the group id, `direction` `out` when `started_by` is me. `false` when
/// it is on record already (the start came again, or by another device).
pub async fn insert_group(store: &Store, c: &NewCall, started_by: &str) -> Result<bool> {
    let res = sqlx::query(
        "INSERT OR IGNORE INTO msg_calls (call_id, chat_id, peer, direction, media, started_at, answered_at, ended_at, outcome, via_relay, kind, started_by, participants)
         VALUES (?, ?, ?, ?, ?, ?, NULL, NULL, NULL, 1, ?, ?, 0)",
    )
    .bind(&c.call_id)
    .bind(&c.chat_id)
    .bind(&c.peer)
    .bind(&c.direction)
    .bind(&c.media)
    .bind(c.started_at)
    .bind(KIND_GROUP)
    .bind(started_by)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(res.rows_affected() == 1)
}

pub async fn group_info(store: &Store, call_id: &str) -> Result<Option<GroupCallInfo>> {
    sqlx::query_as::<_, GroupCallInfo>("SELECT kind, started_by, participants FROM msg_calls WHERE call_id = ?")
        .bind(call_id)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)
}

/// The count of people seen in the room only ever grows.
pub async fn set_participants(store: &Store, call_id: &str, participants: i64) -> Result<()> {
    sqlx::query("UPDATE msg_calls SET participants = ? WHERE call_id = ? AND participants < ?")
        .bind(participants)
        .bind(call_id)
        .bind(participants)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// The group calls of a chat that have no end on record yet, oldest first.
pub async fn open_group_calls(store: &Store, chat_id: &str) -> Result<Vec<CallRow>> {
    sqlx::query_as::<_, CallRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM msg_calls WHERE chat_id = ? AND kind = ? AND outcome IS NULL ORDER BY started_at ASC"
    )))
    .bind(chat_id)
    .bind(KIND_GROUP)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// The group calls that have no end on record although their room has
/// expired on the node by now (started before `started_before`): a call
/// whose `call.end` never came while this device was off, oldest first.
pub async fn stale_group_calls(store: &Store, started_before: i64) -> Result<Vec<CallRow>> {
    sqlx::query_as::<_, CallRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM msg_calls WHERE kind = ? AND outcome IS NULL AND started_at < ? ORDER BY started_at ASC"
    )))
    .bind(KIND_GROUP)
    .bind(started_before)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// Forget a call that never was (the losing half of a glare).
pub async fn delete(store: &Store, call_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM msg_calls WHERE call_id = ?").bind(call_id).execute(store.pool()).await.map_err(storage)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_call_is_recorded_once_and_ends_once() {
        let store = Store::open_in_memory().await.unwrap();
        let new = NewCall {
            call_id: "c1".into(),
            chat_id: "dm:aa".into(),
            peer: "aa".into(),
            direction: DIR_OUT.into(),
            media: MEDIA_AUDIO.into(),
            started_at: 100,
        };
        assert!(insert(&store, &new).await.unwrap());
        assert!(!insert(&store, &new).await.unwrap(), "a copy changes nothing");
        let row = get(&store, "c1").await.unwrap().unwrap();
        assert_eq!(row.outcome, None);
        assert_eq!(row.duration_secs(), None);
        assert!(!row.via_relay);

        set_answered(&store, "c1", 105).await.unwrap();
        set_answered(&store, "c1", 110).await.unwrap();
        set_via_relay(&store, "c1", true).await.unwrap();
        assert!(finish(&store, "c1", OUTCOME_ENDED, 165).await.unwrap());
        assert!(!finish(&store, "c1", OUTCOME_FAILED, 170).await.unwrap(), "the first end holds");
        let row = get(&store, "c1").await.unwrap().unwrap();
        assert_eq!(row.answered_at, Some(105));
        assert_eq!(row.outcome.as_deref(), Some(OUTCOME_ENDED));
        assert_eq!(row.duration_secs(), Some(60));
        assert!(row.via_relay);
        set_outcome(&store, "c1", OUTCOME_FAILED, 180).await.unwrap();
        assert_eq!(get(&store, "c1").await.unwrap().unwrap().outcome.as_deref(), Some(OUTCOME_FAILED), "a later word overrides");

        insert(&store, &NewCall { call_id: "c2".into(), started_at: 200, ..new.clone() }).await.unwrap();
        let ids: Vec<String> = list(&store, "dm:aa", 10).await.unwrap().into_iter().map(|r| r.call_id).collect();
        assert_eq!(ids, vec!["c2", "c1"]);
        delete(&store, "c2").await.unwrap();
        assert!(get(&store, "c2").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_group_call_is_a_row_of_its_own_kind() {
        let store = Store::open_in_memory().await.unwrap();
        let new = NewCall {
            call_id: "g1".into(),
            chat_id: "group:aa".into(),
            peer: "aa".into(),
            direction: DIR_IN.into(),
            media: MEDIA_AUDIO.into(),
            started_at: 100,
        };
        assert!(insert_group(&store, &new, "bb").await.unwrap());
        assert!(!insert_group(&store, &new, "cc").await.unwrap(), "the first start holds");
        let info = group_info(&store, "g1").await.unwrap().unwrap();
        assert_eq!(info, GroupCallInfo { kind: KIND_GROUP.into(), started_by: Some("bb".into()), participants: 0 });
        assert_eq!(group_info(&store, "nope").await.unwrap(), None);
        set_participants(&store, "g1", 3).await.unwrap();
        set_participants(&store, "g1", 2).await.unwrap();
        assert_eq!(group_info(&store, "g1").await.unwrap().unwrap().participants, 3, "the count only grows");
        assert_eq!(open_group_calls(&store, "group:aa").await.unwrap().len(), 1);
        assert_eq!(stale_group_calls(&store, 100).await.unwrap().len(), 0, "started at 100 is not before 100");
        assert_eq!(stale_group_calls(&store, 101).await.unwrap().len(), 1);
        assert!(finish(&store, "g1", OUTCOME_ENDED, 300).await.unwrap());
        assert!(open_group_calls(&store, "group:aa").await.unwrap().is_empty());
        assert!(stale_group_calls(&store, 101).await.unwrap().is_empty());

        // A call between two is of the kind `dm`, by the default of the column.
        insert(&store, &NewCall { call_id: "c1".into(), chat_id: "dm:aa".into(), ..new }).await.unwrap();
        assert_eq!(group_info(&store, "c1").await.unwrap().unwrap().kind, KIND_DM);
        assert!(open_group_calls(&store, "dm:aa").await.unwrap().is_empty());
    }
}
