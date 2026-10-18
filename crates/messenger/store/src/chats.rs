// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_chats`: one row per conversation. DM chats are keyed by peer.

use crate::{storage, Store};
use messenger_core::Result;

pub const KIND_DM: &str = "dm";

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct ChatRow {
    pub id: String,
    pub kind: String,
    pub peer_pubkey: Option<String>,
    pub unread: i64,
    pub last_message_at: Option<i64>,
    pub last_preview: Option<String>,
    pub pinned: bool,
    pub archived: bool,
    /// No notifications from this chat; what comes is still counted and
    /// shown in it, and stays out of the total.
    pub muted: bool,
    pub created_at: i64,
    pub updated_at: i64,
    /// The private server a private chat lives on; `None` for the home set.
    pub server_id: Option<String>,
}

const COLS: &str = "id, kind, peer_pubkey, unread, last_message_at, last_preview, pinned, archived, muted, created_at, updated_at, server_id";

/// The id of the direct chat with `peer_pubkey`: `dm:<peer>` on the home
/// set, `dm:<peer>@<server>` for the private chat on `server`. A server
/// id is hex (`servers::id_of`), so the `@` is never part of a key.
pub fn dm_chat_id(peer_pubkey: &str, server: Option<&str>) -> String {
    match server {
        Some(id) => format!("dm:{peer_pubkey}@{id}"),
        None => format!("dm:{peer_pubkey}"),
    }
}

/// The peer and the private server of a direct chat id (`dm_chat_id`
/// read back); `None` when the id is no direct chat. The one place a peer
/// is read out of a chat id, so a private chat is never taken for the
/// home chat with the same person.
pub fn parse_dm_chat_id(id: &str) -> Option<(&str, Option<&str>)> {
    let rest = id.strip_prefix("dm:")?;
    match rest.split_once('@') {
        Some((peer, server)) if !peer.is_empty() && !server.is_empty() => Some((peer, Some(server))),
        Some(_) => None,
        None if !rest.is_empty() => Some((rest, None)),
        None => None,
    }
}

/// Create the DM chat with `peer` (on `server` for a private chat) if
/// missing; returns the row either way.
pub async fn ensure_dm(store: &Store, peer_pubkey: &str, server: Option<&str>) -> Result<ChatRow> {
    let id = dm_chat_id(peer_pubkey, server);
    let now = crate::now();
    let made = sqlx::query(
        "INSERT OR IGNORE INTO msg_chats (id, kind, peer_pubkey, unread, last_message_at, last_preview, pinned, archived, muted, created_at, updated_at, server_id)
         VALUES (?, 'dm', ?, 0, NULL, NULL, 0, 0, 0, ?, ?, ?)",
    )
    .bind(&id)
    .bind(peer_pubkey)
    .bind(now)
    .bind(now)
    .bind(server)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    if made.rows_affected() > 0 {
        take_prefs(store, &id).await?;
    }
    get(store, &id).await?.ok_or_else(|| messenger_core::MessengerError::Storage("chat vanished".into()))
}

pub async fn get(store: &Store, id: &str) -> Result<Option<ChatRow>> {
    sqlx::query_as::<_, ChatRow>(sqlx::AssertSqlSafe(format!("SELECT {COLS} FROM msg_chats WHERE id = ?")))
        .bind(id)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)
}

/// Pinned first, then newest activity. Archived chats only when asked.
pub async fn list(store: &Store, include_archived: bool) -> Result<Vec<ChatRow>> {
    let sql = if include_archived {
        format!("SELECT {COLS} FROM msg_chats ORDER BY pinned DESC, COALESCE(last_message_at, created_at) DESC")
    } else {
        format!("SELECT {COLS} FROM msg_chats WHERE archived = 0 ORDER BY pinned DESC, COALESCE(last_message_at, created_at) DESC")
    };
    sqlx::query_as::<_, ChatRow>(sqlx::AssertSqlSafe(sql))
        .fetch_all(store.pool())
        .await
        .map_err(storage)
}

/// New activity: move `last_message_at` forward (never back), refresh the
/// preview when this message is the newest, and bump unread if asked and
/// the message is later than what was read on any of my devices. A message
/// later than the archiving takes the chat out of the archive; an older
/// one (history a new device catches up on) leaves it there.
pub async fn touch(store: &Store, id: &str, at: i64, preview: Option<&str>, bump_unread: bool) -> Result<()> {
    let now = crate::now();
    sqlx::query(
        "UPDATE msg_chats SET
           last_preview = CASE WHEN last_message_at IS NULL OR ? >= last_message_at THEN ? ELSE last_preview END,
           last_message_at = MAX(COALESCE(last_message_at, 0), ?),
           unread = unread + CASE WHEN ? AND ? > COALESCE((SELECT read_at FROM msg_own_read WHERE chat_id = ?), 0)
                                  THEN 1 ELSE 0 END,
           archived = CASE WHEN ? > COALESCE((SELECT archived_at FROM msg_chat_prefs WHERE chat_id = ? AND archived = 1), 0)
                           THEN 0 ELSE archived END,
           updated_at = ?
         WHERE id = ?",
    )
    .bind(at)
    .bind(preview)
    .bind(at)
    .bind(bump_unread)
    .bind(at)
    .bind(id)
    .bind(at)
    .bind(id)
    .bind(now)
    .bind(id)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// Recompute `last_message_at`/`last_preview` from the messages still shown
/// (after a delete or an edit of the newest message). A removed message
/// leaves its place in the chat but not in the list: the list shows the
/// newest one that is still there.
pub async fn recompute_last(store: &Store, id: &str) -> Result<()> {
    sqlx::query(
        "UPDATE msg_chats SET
           last_message_at = (SELECT MAX(created_at) FROM msg_messages
                               WHERE chat_id = ? AND is_hidden = 0 AND mod_state = '' AND review <> 'pending' AND deleted_at IS NULL AND content_type != 'system' AND NOT (chat_id LIKE 'group:%' AND (EXISTS (SELECT 1 FROM msg_dm_relations r WHERE r.peer_pubkey = msg_messages.sender_pubkey AND r.server_id = '' AND r.blocked = 1) OR EXISTS (SELECT 1 FROM msg_banned_keys b WHERE b.pubkey = msg_messages.sender_pubkey)))),
           last_preview = (SELECT CASE WHEN content_type = 'media' THEN '📎 ' || COALESCE(plain, text, json_extract(media_json, '$.name'), '')
                                       WHEN content_type = 'contact' THEN '👤 ' || COALESCE(json_extract(media_json, '$.display_name'),
                                                                                         json_extract(media_json, '$.name'), '')
                                       ELSE COALESCE(plain, text) END FROM msg_messages
                           WHERE chat_id = ? AND is_hidden = 0 AND mod_state = '' AND review <> 'pending' AND deleted_at IS NULL AND content_type != 'system' AND NOT (chat_id LIKE 'group:%' AND (EXISTS (SELECT 1 FROM msg_dm_relations r WHERE r.peer_pubkey = msg_messages.sender_pubkey AND r.server_id = '' AND r.blocked = 1) OR EXISTS (SELECT 1 FROM msg_banned_keys b WHERE b.pubkey = msg_messages.sender_pubkey)))
                           ORDER BY created_at DESC LIMIT 1),
           updated_at = ?
         WHERE id = ?",
    )
    .bind(id)
    .bind(id)
    .bind(crate::now())
    .bind(id)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// The chat was read here: nothing waits, and what is read moves up to the
/// newest message of the peer. Returns that time when it moved, so the
/// other devices can be told; `None` when they know it already. A mark that
/// moved is also owed to the peers (`receipts::take_due_read`).
pub async fn mark_read(store: &Store, id: &str) -> Result<Option<i64>> {
    sqlx::query("UPDATE msg_chats SET unread = 0, updated_at = ? WHERE id = ?")
        .bind(crate::now())
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    let newest = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(created_at) FROM msg_messages WHERE chat_id = ? AND direction = 'in' AND is_hidden = 0 AND mod_state = ''",
    )
    .bind(id)
    .fetch_one(store.pool())
    .await
    .map_err(storage)?;
    match newest {
        Some(at) if raise_read(store, id, at, true).await? => Ok(Some(at)),
        _ => Ok(None),
    }
}

/// Another device of mine read the chat up to `at`: what is not later stops
/// counting. Unread is only ever lowered here. `true` when it went down.
/// The peers are told by the device that read, not by this one.
pub async fn read_up_to(store: &Store, id: &str, at: i64) -> Result<bool> {
    if !raise_read(store, id, at, false).await? {
        return Ok(false);
    }
    let later = format!("(SELECT COUNT(*) FROM msg_messages WHERE chat_id = ?1 AND {COUNTED} AND created_at > ?2)");
    let r = sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE msg_chats SET unread = {later}, updated_at = ?3 WHERE id = ?1 AND unread > {later}"
    )))
    .bind(id)
    .bind(at)
    .bind(crate::now())
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(r.rows_affected() > 0)
}

/// A row of `msg_messages` that counts as unread once it is later than
/// what was read: incoming, shown, not waiting for a moderator, and in a
/// group not of someone I block in direct messages nor of a banned key.
const COUNTED: &str = "direction = 'in' AND is_hidden = 0 AND mod_state = '' AND review <> 'pending' AND deleted_at IS NULL AND content_type != 'system'
    AND NOT (chat_id LIKE 'group:%' AND (EXISTS (SELECT 1 FROM msg_dm_relations r WHERE r.peer_pubkey = msg_messages.sender_pubkey AND r.server_id = '' AND r.blocked = 1) OR EXISTS (SELECT 1 FROM msg_banned_keys b WHERE b.pubkey = msg_messages.sender_pubkey)))";

/// A block in direct messages took effect, or was lifted: the group
/// chats the peer wrote in show their line of the list without them
/// (`recompute_last`) and count unread without them, never more than
/// they counted before (as `read_up_to`: what was never counted, old
/// history under the floor of a new device, does not start to count by
/// an unblock). The chats touched, for the screen to be told.
pub async fn group_chats_follow_block(store: &Store, peer_pubkey: &str) -> Result<Vec<String>> {
    let ids = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT chat_id FROM msg_messages WHERE sender_pubkey = ? AND chat_id LIKE 'group:%' AND is_hidden = 0",
    )
    .bind(peer_pubkey)
    .fetch_all(store.pool())
    .await
    .map_err(storage)?;
    for id in &ids {
        recompute_last(store, id).await?;
        let counted = format!(
            "(SELECT COUNT(*) FROM msg_messages WHERE chat_id = ?1 AND {COUNTED}
               AND created_at > COALESCE((SELECT read_at FROM msg_own_read WHERE chat_id = ?1), 0))"
        );
        sqlx::query(sqlx::AssertSqlSafe(format!("UPDATE msg_chats SET unread = {counted}, updated_at = ?2 WHERE id = ?1 AND unread > {counted}")))
            .bind(id)
            .bind(crate::now())
            .execute(store.pool())
            .await
            .map_err(storage)?;
    }
    Ok(ids)
}

/// Move what is read in the chat up to `at`, never back. `true` when it moved.
/// `due`: read here, so a receipt is owed; a mark from elsewhere leaves a
/// receipt still owed as it was.
async fn raise_read(store: &Store, id: &str, at: i64, due: bool) -> Result<bool> {
    let r = sqlx::query(
        "INSERT INTO msg_own_read (chat_id, read_at, receipt_due) VALUES (?, ?, ?)
         ON CONFLICT(chat_id) DO UPDATE SET read_at = excluded.read_at,
                                            receipt_due = MAX(msg_own_read.receipt_due, excluded.receipt_due)
         WHERE excluded.read_at > msg_own_read.read_at",
    )
    .bind(id)
    .bind(at)
    .bind(due)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(r.rows_affected() > 0)
}

/// Up to when the chat is read on any of my devices; `0` when it never was.
pub async fn read_at(store: &Store, id: &str) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>("SELECT read_at FROM msg_own_read WHERE chat_id = ?")
        .bind(id)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)?
        .unwrap_or(0))
}

/// A choice of a chat that my devices share (`msg_chat_prefs`, the
/// snapshot's `chats`): each is dated by itself, so two devices that
/// chose different ones of a chat both keep theirs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pref {
    /// No notifications from the chat: no sound, nothing shown, and its
    /// unread stays out of the total.
    Muted,
    Pinned,
    Archived,
}

impl Pref {
    pub const ALL: [Pref; 3] = [Pref::Muted, Pref::Pinned, Pref::Archived];

    /// The column, in `msg_chats` and in `msg_chat_prefs` (with `_at`).
    fn col(self) -> &'static str {
        match self {
            Pref::Muted => "muted",
            Pref::Pinned => "pinned",
            Pref::Archived => "archived",
        }
    }
}

/// What was chosen of one chat, and when (my clock; 0 = never).
#[derive(Clone, Debug, Default, PartialEq, Eq, sqlx::FromRow)]
pub struct PrefsRow {
    pub chat_id: String,
    pub muted: bool,
    pub muted_at: i64,
    pub pinned: bool,
    pub pinned_at: i64,
    pub archived: bool,
    pub archived_at: i64,
}

impl PrefsRow {
    /// The choice and its time.
    pub fn get(&self, pref: Pref) -> (bool, i64) {
        match pref {
            Pref::Muted => (self.muted, self.muted_at),
            Pref::Pinned => (self.pinned, self.pinned_at),
            Pref::Archived => (self.archived, self.archived_at),
        }
    }
}

/// Chosen here at `at`: the chat shows it at once, and the choice is dated
/// after the one it replaces, so it wins on my other devices even when
/// this clock runs behind theirs.
pub async fn set_pref(store: &Store, id: &str, pref: Pref, on: bool, at: i64) -> Result<()> {
    let c = pref.col();
    let mut tx = store.pool().begin().await.map_err(storage)?;
    sqlx::query(sqlx::AssertSqlSafe(format!("UPDATE msg_chats SET {c} = ?, updated_at = ? WHERE id = ?")))
        .bind(on)
        .bind(crate::now())
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO msg_chat_prefs (chat_id, {c}, {c}_at) VALUES (?, ?, ?)
         ON CONFLICT(chat_id) DO UPDATE SET {c} = excluded.{c}, {c}_at = MAX(excluded.{c}_at, msg_chat_prefs.{c}_at + 1)"
    )))
    .bind(id)
    .bind(on)
    .bind(at)
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    tx.commit().await.map_err(storage)
}

pub async fn set_pinned(store: &Store, id: &str, pinned: bool, at: i64) -> Result<()> {
    set_pref(store, id, Pref::Pinned, pinned, at).await
}

pub async fn set_archived(store: &Store, id: &str, archived: bool, at: i64) -> Result<()> {
    set_pref(store, id, Pref::Archived, archived, at).await
}

pub async fn set_muted(store: &Store, id: &str, muted: bool, at: i64) -> Result<()> {
    set_pref(store, id, Pref::Muted, muted, at).await
}

/// A choice of another device of mine (the snapshot): taken when it is
/// newer than the one here, and of one second `on` wins. The chat shows
/// it when this device has it (one made later takes it, `take_prefs`); a
/// chat archived before its newest message stays out of the archive, as
/// that message took it out on the device that archived it (`touch`).
/// `true` when taken.
pub async fn merge_pref(store: &Store, id: &str, pref: Pref, on: bool, at: i64) -> Result<bool> {
    if at <= 0 {
        return Ok(false);
    }
    let c = pref.col();
    let mut tx = store.pool().begin().await.map_err(storage)?;
    let r = sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO msg_chat_prefs (chat_id, {c}, {c}_at) VALUES (?, ?, ?)
         ON CONFLICT(chat_id) DO UPDATE SET {c} = excluded.{c}, {c}_at = excluded.{c}_at
         WHERE excluded.{c}_at > msg_chat_prefs.{c}_at
            OR (excluded.{c}_at = msg_chat_prefs.{c}_at AND excluded.{c} > msg_chat_prefs.{c})"
    )))
    .bind(id)
    .bind(on)
    .bind(at)
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    if r.rows_affected() == 0 {
        return Ok(false);
    }
    let shown = if pref == Pref::Archived { "CASE WHEN ?1 AND COALESCE(last_message_at, 0) <= ?2 THEN 1 ELSE 0 END" } else { "?1" };
    sqlx::query(sqlx::AssertSqlSafe(format!("UPDATE msg_chats SET {c} = {shown}, updated_at = ?3 WHERE id = ?4")))
        .bind(on)
        .bind(at)
        .bind(crate::now())
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    tx.commit().await.map_err(storage)?;
    Ok(true)
}

/// A chat just made takes what my devices chose of it before it was here.
pub async fn take_prefs(store: &Store, id: &str) -> Result<()> {
    sqlx::query(
        "UPDATE msg_chats SET
           muted = (SELECT muted FROM msg_chat_prefs WHERE chat_id = ?1),
           pinned = (SELECT pinned FROM msg_chat_prefs WHERE chat_id = ?1),
           archived = (SELECT archived FROM msg_chat_prefs WHERE chat_id = ?1)
         WHERE id = ?1 AND EXISTS (SELECT 1 FROM msg_chat_prefs WHERE chat_id = ?1)",
    )
    .bind(id)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// Every chat something was chosen of, the newest choice first.
pub async fn prefs(store: &Store) -> Result<Vec<PrefsRow>> {
    sqlx::query_as::<_, PrefsRow>(
        "SELECT chat_id, muted, muted_at, pinned, pinned_at, archived, archived_at FROM msg_chat_prefs
         WHERE muted_at > 0 OR pinned_at > 0 OR archived_at > 0
         ORDER BY MAX(muted_at, pinned_at, archived_at) DESC, chat_id",
    )
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// Whether nothing of the chat is to be told. A chat this device does not
/// have yet is muted when one of my devices muted it.
pub async fn is_muted(store: &Store, id: &str) -> Result<bool> {
    sqlx::query_scalar::<_, bool>(
        "SELECT COALESCE((SELECT muted FROM msg_chats WHERE id = ?1), (SELECT muted FROM msg_chat_prefs WHERE chat_id = ?1), 0)",
    )
    .bind(id)
    .fetch_one(store.pool())
    .await
    .map_err(storage)
}

/// How far ahead of the clock `cleared_at` may sit (seconds).
pub const CLEARED_SKEW_SECS: i64 = 300;

/// Remove the chat, every message in it and every reaction that came in it.
/// Relays keep the ciphertext, so a tombstone keeps it gone, here and on my
/// other devices (the snapshot's `deleted`, `delete_copy`). My own copies
/// are dropped by time (`cleared_at`): my rumors can run ahead of the clock
/// (`next_created_at`), so it is never earlier than my newest message, and
/// never more than `CLEARED_SKEW_SECS` past the clock, so one row dated in
/// the future cannot hold back all that comes before it. The peer's are
/// dropped up to their newest message the chat held (`peer_upto`; one
/// dated past my clock and the skew is left out), not up to my clock:
/// theirs may run behind, and what they write after the deletion is new.
/// The rows themselves are remembered by id too (`was_deleted`).
pub async fn delete(store: &Store, id: &str, now: i64) -> Result<()> {
    let mut tx = store.pool().begin().await.map_err(storage)?;
    sqlx::query(
        "INSERT INTO msg_chat_cleared (chat_id, cleared_at, peer_upto)
         VALUES (?1, MAX(?2, MIN(?2 + ?3, COALESCE(
             (SELECT MAX(created_at) FROM msg_messages WHERE chat_id = ?1 AND direction = 'out'), 0))),
             COALESCE(
             (SELECT MAX(created_at) FROM msg_messages WHERE chat_id = ?1 AND direction = 'in' AND created_at <= ?2 + ?3), 0))
         ON CONFLICT(chat_id) DO UPDATE SET cleared_at = MAX(msg_chat_cleared.cleared_at, excluded.cleared_at),
                                            peer_upto = MAX(msg_chat_cleared.peer_upto, excluded.peer_upto)",
    )
    .bind(id)
    .bind(now)
    .bind(CLEARED_SKEW_SECS)
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    // Held rows of the DM gate were never shown: gone as if dropped, not
    // remembered (as `dm_held::purge`).
    sqlx::query("DELETE FROM msg_messages WHERE id IN (SELECT message_id FROM msg_dm_held WHERE chat_id = ?) AND is_hidden = 1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    sqlx::query("DELETE FROM msg_dm_held WHERE chat_id = ?").bind(id).execute(&mut *tx).await.map_err(storage)?;
    // Every row, mine too: an own row dated past the cap stays gone by id.
    // System lines and placeholders have local ids no rumor carries.
    sqlx::query(
        "INSERT OR IGNORE INTO msg_chat_deleted_ids (message_id, chat_id, deleted_at)
         SELECT id, chat_id, ?2 FROM msg_messages WHERE chat_id = ?1",
    )
    .bind(id)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    sqlx::query("DELETE FROM msg_messages WHERE chat_id = ?").bind(id).execute(&mut *tx).await.map_err(storage)?;
    sqlx::query("DELETE FROM msg_reactions WHERE chat_id = ?").bind(id).execute(&mut *tx).await.map_err(storage)?;
    sqlx::query("DELETE FROM msg_chats WHERE id = ?").bind(id).execute(&mut *tx).await.map_err(storage)?;
    tx.commit().await.map_err(storage)
}

/// The rumor was in a chat when that chat was deleted here.
pub async fn was_deleted(store: &Store, message_id: &str) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>("SELECT 1 FROM msg_chat_deleted_ids WHERE message_id = ?")
        .bind(message_id)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)?
        .is_some())
}

/// Up to when (rumor time, inclusive) the chat was deleted on this device;
/// `0` when it never was.
pub async fn cleared_at(store: &Store, id: &str) -> Result<i64> {
    Ok(tombstone(store, id).await?.0)
}

/// `(cleared_at, peer_upto)` of the chat: up to when my rows and the
/// peer's are gone (rumor times, inclusive); `(0, 0)` when it was never
/// deleted.
pub async fn tombstone(store: &Store, id: &str) -> Result<(i64, i64)> {
    Ok(sqlx::query_as::<_, (i64, i64)>("SELECT cleared_at, peer_upto FROM msg_chat_cleared WHERE chat_id = ?")
        .bind(id)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)?
        .unwrap_or((0, 0)))
}

/// Every direct chat deleted on this device or on another of mine, the
/// newest deletion first: `(chat_id, cleared_at, peer_upto)`.
pub async fn tombstones(store: &Store) -> Result<Vec<(String, i64, i64)>> {
    sqlx::query_as::<_, (String, i64, i64)>(
        "SELECT chat_id, cleared_at, peer_upto FROM msg_chat_cleared WHERE chat_id LIKE 'dm:%' ORDER BY cleared_at DESC, chat_id",
    )
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// A deletion of the chat on another device of mine (the snapshot): what
/// it held up to then goes here too — my rows up to `cleared_at`, the
/// peer's up to `peer_upto`, remembered by id as `delete` does — and what
/// came after it stays; the chat goes when nothing stays. Times are taken
/// only upward. `true` when the tombstone moved.
pub async fn delete_copy(store: &Store, id: &str, cleared_at: i64, peer_upto: i64, now: i64) -> Result<bool> {
    let (mine, theirs) = tombstone(store, id).await?;
    let (cleared_at, peer_upto) = (cleared_at.max(mine), peer_upto.max(theirs));
    if (cleared_at, peer_upto) == (mine, theirs) {
        return Ok(false);
    }
    let mut tx = store.pool().begin().await.map_err(storage)?;
    sqlx::query(
        "INSERT INTO msg_chat_cleared (chat_id, cleared_at, peer_upto) VALUES (?1, ?2, ?3)
         ON CONFLICT(chat_id) DO UPDATE SET cleared_at = excluded.cleared_at, peer_upto = excluded.peer_upto",
    )
    .bind(id)
    .bind(cleared_at)
    .bind(peer_upto)
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    sqlx::query(
        "DELETE FROM msg_messages WHERE id IN (SELECT message_id FROM msg_dm_held WHERE chat_id = ?1 AND created_at <= ?2) AND is_hidden = 1",
    )
    .bind(id)
    .bind(peer_upto)
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    sqlx::query("DELETE FROM msg_dm_held WHERE chat_id = ?1 AND created_at <= ?2")
        .bind(id)
        .bind(peer_upto)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    const GONE: &str = "chat_id = ?1 AND ((direction = 'out' AND created_at <= ?2) OR (direction = 'in' AND created_at <= ?3))";
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT OR IGNORE INTO msg_chat_deleted_ids (message_id, chat_id, deleted_at)
         SELECT id, chat_id, ?4 FROM msg_messages WHERE {GONE}"
    )))
    .bind(id)
    .bind(cleared_at)
    .bind(peer_upto)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DELETE FROM msg_reactions WHERE chat_id = ?1 AND message_id IN (SELECT id FROM msg_messages WHERE {GONE})"
    )))
    .bind(id)
    .bind(cleared_at)
    .bind(peer_upto)
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM msg_messages WHERE {GONE}")))
        .bind(id)
        .bind(cleared_at)
        .bind(peer_upto)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM msg_messages WHERE chat_id = ?")
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(storage)?;
    if left == 0 {
        sqlx::query("DELETE FROM msg_reactions WHERE chat_id = ?").bind(id).execute(&mut *tx).await.map_err(storage)?;
        sqlx::query("DELETE FROM msg_chats WHERE id = ?").bind(id).execute(&mut *tx).await.map_err(storage)?;
    }
    tx.commit().await.map_err(storage)?;
    if left > 0 {
        recompute_last(store, id).await?;
        let counted = format!(
            "(SELECT COUNT(*) FROM msg_messages WHERE chat_id = ?1 AND {COUNTED}
               AND created_at > COALESCE((SELECT read_at FROM msg_own_read WHERE chat_id = ?1), 0))"
        );
        sqlx::query(sqlx::AssertSqlSafe(format!("UPDATE msg_chats SET unread = {counted}, updated_at = ?2 WHERE id = ?1")))
            .bind(id)
            .bind(now)
            .execute(store.pool())
            .await
            .map_err(storage)?;
    }
    Ok(true)
}

/// What the app counts as waiting: a muted or archived chat keeps its own
/// counter but stays out of the total (the badge, the tray), and so does a
/// request of a stranger (`dm_relations::REQUEST_SQL`): the queue of
/// requests has a count of its own. The chat of a key the network bans
/// (`banned`) stays out of it too: it reads as blocked, and nothing of
/// that person calls for the user any more.
pub async fn total_unread(store: &Store) -> Result<i64> {
    sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!(
        "SELECT COALESCE(SUM(c.unread), 0) FROM msg_chats c
         LEFT JOIN msg_dm_relations r ON r.peer_pubkey = c.peer_pubkey AND r.server_id = COALESCE(c.server_id, '')
         WHERE c.archived = 0 AND c.muted = 0 AND NOT (c.kind = 'dm' AND r.peer_pubkey IS NOT NULL AND {})
           AND NOT (c.kind = 'dm' AND EXISTS (SELECT 1 FROM msg_banned_keys b WHERE b.pubkey = c.peer_pubkey))",
        crate::dm_relations::REQUEST_SQL
    )))
    .fetch_one(store.pool())
    .await
    .map_err(storage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn chat_lifecycle() {
        let s = Store::open_in_memory().await.unwrap();
        let a = ensure_dm(&s, "aa", None).await.unwrap();
        assert_eq!(a.id, "dm:aa");
        assert_eq!(ensure_dm(&s, "aa", None).await.unwrap(), a, "idempotent");
        ensure_dm(&s, "bb", None).await.unwrap();

        let t0 = crate::now() + 100;
        touch(&s, "dm:aa", t0, Some("hello"), true).await.unwrap();
        touch(&s, "dm:aa", t0 - 50, Some("older"), true).await.unwrap();
        let a = get(&s, "dm:aa").await.unwrap().unwrap();
        assert_eq!(a.unread, 2);
        assert_eq!(a.last_message_at, Some(t0));
        assert_eq!(a.last_preview.as_deref(), Some("hello"), "older message does not replace the preview");
        assert_eq!(total_unread(&s).await.unwrap(), 2);

        let l = list(&s, false).await.unwrap();
        assert_eq!(l[0].id, "dm:aa", "recent activity first");
        set_pinned(&s, "dm:bb", true, t0).await.unwrap();
        assert_eq!(list(&s, false).await.unwrap()[0].id, "dm:bb", "pinned wins");

        mark_read(&s, "dm:aa").await.unwrap();
        assert_eq!(get(&s, "dm:aa").await.unwrap().unwrap().unread, 0);
        assert!(!is_muted(&s, "dm:aa").await.unwrap());
        assert!(!is_muted(&s, "dm:nobody").await.unwrap(), "a chat that does not exist is not muted");
        set_muted(&s, "dm:aa", true, t0).await.unwrap();
        assert!(is_muted(&s, "dm:aa").await.unwrap());
        touch(&s, "dm:aa", t0 + 1, Some("still counted"), true).await.unwrap();
        assert_eq!(get(&s, "dm:aa").await.unwrap().unwrap().unread, 1, "muted chats still count unread");
        assert_eq!(total_unread(&s).await.unwrap(), 0, "but stay out of the total");
        mark_read(&s, "dm:aa").await.unwrap();
        set_archived(&s, "dm:aa", true, t0 + 50).await.unwrap();
        assert_eq!(list(&s, false).await.unwrap().len(), 1);
        assert_eq!(list(&s, true).await.unwrap().len(), 2);
        touch(&s, "dm:aa", t0 + 100, Some("back"), false).await.unwrap();
        assert!(!get(&s, "dm:aa").await.unwrap().unwrap().archived, "activity unarchives");

        delete(&s, "dm:aa", crate::now()).await.unwrap();
        assert!(get(&s, "dm:aa").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn the_choices_of_a_chat_go_by_their_own_time() {
        let s = Store::open_in_memory().await.unwrap();
        ensure_dm(&s, "aa", None).await.unwrap();
        set_muted(&s, "dm:aa", true, 1_000).await.unwrap();
        let p = |s: Store| async move { prefs(&s).await.unwrap().into_iter().find(|r| r.chat_id == "dm:aa").unwrap() };
        assert_eq!(p(s.clone()).await.get(Pref::Muted), (true, 1_000));

        // Older, or of the same second and off: the choice here stands.
        assert!(!merge_pref(&s, "dm:aa", Pref::Muted, false, 999).await.unwrap());
        assert!(!merge_pref(&s, "dm:aa", Pref::Muted, false, 1_000).await.unwrap());
        assert!(is_muted(&s, "dm:aa").await.unwrap());
        // Newer: taken, and one choice leaves the others as they were.
        assert!(merge_pref(&s, "dm:aa", Pref::Pinned, true, 900).await.unwrap());
        assert!(merge_pref(&s, "dm:aa", Pref::Muted, false, 1_001).await.unwrap());
        let row = get(&s, "dm:aa").await.unwrap().unwrap();
        assert!(!row.muted && row.pinned);
        // A choice made here goes after the one it replaces, whatever the clock says.
        set_muted(&s, "dm:aa", true, 10).await.unwrap();
        assert_eq!(p(s.clone()).await.get(Pref::Muted), (true, 1_002));

        // A chat this device does not have yet: muted already, and made so.
        assert!(merge_pref(&s, "dm:bb", Pref::Muted, true, 500).await.unwrap());
        assert!(is_muted(&s, "dm:bb").await.unwrap());
        assert!(ensure_dm(&s, "bb", None).await.unwrap().muted);
        assert!(!merge_pref(&s, "dm:bb", Pref::Muted, true, 0).await.unwrap(), "no time, no choice");

        // An archive: history older than it leaves the chat there, a newer
        // message takes it out; a copy of an archive the chat outlived is
        // taken without hiding the chat.
        assert!(merge_pref(&s, "dm:bb", Pref::Archived, true, 2_000).await.unwrap());
        assert!(get(&s, "dm:bb").await.unwrap().unwrap().archived);
        touch(&s, "dm:bb", 1_500, Some("old"), true).await.unwrap();
        assert!(get(&s, "dm:bb").await.unwrap().unwrap().archived, "history stays archived");
        touch(&s, "dm:bb", 2_500, Some("new"), true).await.unwrap();
        assert!(!get(&s, "dm:bb").await.unwrap().unwrap().archived, "a new message brings it back");
        assert!(merge_pref(&s, "dm:bb", Pref::Archived, true, 2_100).await.unwrap());
        assert!(!get(&s, "dm:bb").await.unwrap().unwrap().archived, "archived before its newest message");
    }

    async fn incoming(s: &Store, id: &str, at: i64, text: &str) {
        row(s, id, at, text, "in").await
    }

    async fn row(s: &Store, id: &str, at: i64, text: &str, direction: &str) {
        let m = crate::messages::NewMessage {
            id: id.into(),
            chat_id: "dm:aa".into(),
            wire_id: None,
            direction: direction.into(),
            status: if direction == "in" { "received" } else { "sent" }.into(),
            content_type: "text".into(),
            text: Some(text.into()),
            envelope_json: "{}".into(),
            sender_pubkey: "aa".into(),
            reply_to_id: None,
            target_id: None,
            created_at: at,
            is_hidden: false,
            outbox_local_id: None,
            media_json: None,
        };
        crate::messages::insert(s, &m).await.unwrap();
        touch(s, "dm:aa", at, Some(text), true).await.unwrap();
    }

    #[tokio::test]
    async fn the_list_shows_the_newest_message_still_there() {
        let s = Store::open_in_memory().await.unwrap();
        ensure_dm(&s, "aa", None).await.unwrap();
        incoming(&s, "m1", 100, "one").await;
        incoming(&s, "m2", 200, "two").await;
        crate::messages::mark_deleted(&s, "m2", 300).await.unwrap();
        recompute_last(&s, "dm:aa").await.unwrap();
        let c = get(&s, "dm:aa").await.unwrap().unwrap();
        assert_eq!((c.last_preview.as_deref(), c.last_message_at), (Some("one"), Some(100)));
        crate::messages::mark_deleted(&s, "m1", 300).await.unwrap();
        recompute_last(&s, "dm:aa").await.unwrap();
        assert_eq!(get(&s, "dm:aa").await.unwrap().unwrap().last_preview, None, "nothing left");
    }

    #[tokio::test]
    async fn a_contact_card_shows_in_the_list_by_its_name() {
        let s = Store::open_in_memory().await.unwrap();
        ensure_dm(&s, "aa", None).await.unwrap();
        incoming(&s, "m1", 100, "one").await;
        let card = |id: &str, at: i64, json: &str| crate::messages::NewMessage {
            id: id.into(),
            chat_id: "dm:aa".into(),
            wire_id: None,
            direction: "in".into(),
            status: "received".into(),
            content_type: crate::messages::CT_CONTACT.into(),
            text: None,
            envelope_json: "{}".into(),
            sender_pubkey: "aa".into(),
            reply_to_id: None,
            target_id: None,
            created_at: at,
            is_hidden: false,
            outbox_local_id: None,
            media_json: Some(json.into()),
        };
        crate::messages::insert(&s, &card("c1", 200, r#"{"pubkey":"bb","name":"anna","display_name":"Анна"}"#)).await.unwrap();
        recompute_last(&s, "dm:aa").await.unwrap();
        assert_eq!(get(&s, "dm:aa").await.unwrap().unwrap().last_preview.as_deref(), Some("👤 Анна"));
        crate::messages::insert(&s, &card("c2", 300, r#"{"pubkey":"bb","name":"anna"}"#)).await.unwrap();
        recompute_last(&s, "dm:aa").await.unwrap();
        assert_eq!(get(&s, "dm:aa").await.unwrap().unwrap().last_preview.as_deref(), Some("👤 anna"));
        crate::messages::insert(&s, &card("c3", 400, r#"{"pubkey":"bb"}"#)).await.unwrap();
        recompute_last(&s, "dm:aa").await.unwrap();
        assert_eq!(get(&s, "dm:aa").await.unwrap().unwrap().last_preview.as_deref(), Some("👤 "));
    }

    #[tokio::test]
    async fn a_deleted_chat_leaves_a_tombstone_that_never_goes_back() {
        let s = Store::open_in_memory().await.unwrap();
        assert_eq!(cleared_at(&s, "dm:aa").await.unwrap(), 0, "never deleted");
        ensure_dm(&s, "aa", None).await.unwrap();
        incoming(&s, "m1", 100, "one").await;
        row(&s, "m2", 600, "mine, ahead of the clock", "out").await;
        incoming(&s, "m3", 650, "theirs, further ahead").await;
        delete(&s, "dm:aa", 500).await.unwrap();
        assert!(get(&s, "dm:aa").await.unwrap().is_none());
        assert_eq!(cleared_at(&s, "dm:aa").await.unwrap(), 600, "my newest message when it is later than the clock");
        for id in ["m1", "m2", "m3"] {
            assert!(was_deleted(&s, id).await.unwrap(), "{id} is remembered");
        }
        assert!(!was_deleted(&s, "m4").await.unwrap());
        delete(&s, "dm:aa", 550).await.unwrap();
        assert_eq!(cleared_at(&s, "dm:aa").await.unwrap(), 600, "a second delete never lowers it");
        delete(&s, "dm:aa", 700).await.unwrap();
        assert_eq!(cleared_at(&s, "dm:aa").await.unwrap(), 700, "a later one raises it");

        ensure_dm(&s, "bb", None).await.unwrap();
        delete(&s, "dm:bb", 500).await.unwrap();
        assert_eq!(cleared_at(&s, "dm:bb").await.unwrap(), 500, "an empty chat: the clock");
        assert_eq!(cleared_at(&s, "dm:cc").await.unwrap(), 0);
        assert_eq!(tombstone(&s, "dm:aa").await.unwrap(), (700, 650), "the peer's newest it held, within the skew");
        assert_eq!(tombstone(&s, "dm:bb").await.unwrap(), (500, 0), "none of theirs");
    }

    /// Deleted on my other device: what the chat held then goes, what came
    /// after stays, and the chat goes only when nothing stays.
    #[tokio::test]
    async fn a_deletion_from_my_other_device_takes_what_the_chat_held_then() {
        let s = Store::open_in_memory().await.unwrap();
        ensure_dm(&s, "aa", None).await.unwrap();
        incoming(&s, "m1", 100, "before").await;
        row(&s, "m2", 200, "mine, before", "out").await;
        incoming(&s, "m3", 400, "after").await;
        assert!(delete_copy(&s, "dm:aa", 250, 150, 1_000).await.unwrap());
        assert!(was_deleted(&s, "m1").await.unwrap() && was_deleted(&s, "m2").await.unwrap());
        let c = get(&s, "dm:aa").await.unwrap().expect("a later message keeps the chat");
        assert_eq!((c.last_preview.as_deref(), c.unread), (Some("after"), 1));
        assert!(!delete_copy(&s, "dm:aa", 200, 100, 1_000).await.unwrap(), "an older one changes nothing");
        assert!(delete_copy(&s, "dm:aa", 450, 400, 1_000).await.unwrap());
        assert!(get(&s, "dm:aa").await.unwrap().is_none(), "nothing left: the chat goes");
        assert_eq!(tombstone(&s, "dm:aa").await.unwrap(), (450, 400));
        // A chat this device never had: the tombstone waits for its history.
        assert!(delete_copy(&s, "dm:zz", 300, 280, 1_000).await.unwrap());
        assert_eq!(tombstones(&s).await.unwrap(), vec![("dm:aa".into(), 450, 400), ("dm:zz".into(), 300, 280)]);
    }

    #[tokio::test]
    async fn a_future_dated_row_moves_the_tombstone_only_so_far() {
        let s = Store::open_in_memory().await.unwrap();
        let now = 1_000_000;
        ensure_dm(&s, "aa", None).await.unwrap();
        incoming(&s, "theirs", now + 1_000_000, "from a clock far ahead").await;
        delete(&s, "dm:aa", now).await.unwrap();
        assert_eq!(cleared_at(&s, "dm:aa").await.unwrap(), now, "the peer's rows never move it");
        assert!(was_deleted(&s, "theirs").await.unwrap(), "that row stays gone by its id");

        ensure_dm(&s, "aa", None).await.unwrap();
        row(&s, "mine", now + 1_000_000, "pushed ahead by such a row", "out").await;
        delete(&s, "dm:aa", now).await.unwrap();
        let cleared = cleared_at(&s, "dm:aa").await.unwrap();
        assert!(cleared <= now + 300);
        assert_eq!(cleared, now + CLEARED_SKEW_SECS);
        assert!(was_deleted(&s, "mine").await.unwrap());
    }

    #[tokio::test]
    async fn migration_025_caps_a_future_tombstone() {
        let s = Store::open_in_memory().await.unwrap();
        let now = crate::now();
        for (chat, at) in [("dm:past", 500_i64), ("dm:future", 4_102_444_800)] {
            sqlx::query("INSERT INTO msg_chat_cleared (chat_id, cleared_at) VALUES (?, ?)")
                .bind(chat)
                .bind(at)
                .execute(s.pool())
                .await
                .unwrap();
        }
        sqlx::raw_sql(include_str!("../migrations/025_chat_deleted_ids.sql")).execute(s.pool()).await.unwrap();
        assert_eq!(cleared_at(&s, "dm:past").await.unwrap(), 500);
        let future = cleared_at(&s, "dm:future").await.unwrap();
        assert!((now + CLEARED_SKEW_SECS..=now + CLEARED_SKEW_SECS + 5).contains(&future), "{future}");
    }

    #[tokio::test]
    async fn an_unread_request_is_not_counted_as_waiting() {
        let s = Store::open_in_memory().await.unwrap();
        ensure_dm(&s, "aa", None).await.unwrap();
        incoming(&s, "m1", 100, "may I?").await;
        assert_eq!(total_unread(&s).await.unwrap(), 1, "a chat without a relation row counts");
        let mut r = crate::dm_relations::RelationRow {
            peer_pubkey: "aa".into(),
            server_id: String::new(),
            my_contact: "none".into(),
            blocked: false,
            peer_signal: "approved".into(),
            was_ever_mutual: false,
            last_signal_at: 0,
            last_my_signal_at: 0,
            request_floor: 0,
            decided_at: 0,
            created_at: 0,
            updated_at: 0,
        };
        crate::dm_relations::put(&s, &r).await.unwrap();
        assert_eq!(get(&s, "dm:aa").await.unwrap().unwrap().unread, 1, "the chat keeps its counter");
        assert_eq!(total_unread(&s).await.unwrap(), 0, "a request stays out of the total");
        r.my_contact = "approved".into();
        crate::dm_relations::put(&s, &r).await.unwrap();
        assert_eq!(total_unread(&s).await.unwrap(), 1, "accepted: it counts");
    }

    #[tokio::test]
    async fn read_on_any_device_holds_whatever_comes_first() {
        let s = Store::open_in_memory().await.unwrap();
        assert!(!read_up_to(&s, "dm:aa", 150).await.unwrap(), "no chat yet: kept, nothing to lower");
        ensure_dm(&s, "aa", None).await.unwrap();
        incoming(&s, "m1", 100, "one").await;
        incoming(&s, "m2", 200, "two").await;
        assert_eq!(get(&s, "dm:aa").await.unwrap().unwrap().unread, 1, "the earlier one was read elsewhere");
        assert!(!read_up_to(&s, "dm:aa", 120).await.unwrap(), "never back");
        assert_eq!(mark_read(&s, "dm:aa").await.unwrap(), Some(200));
        assert_eq!(mark_read(&s, "dm:aa").await.unwrap(), None, "the others know it");
        assert_eq!(read_at(&s, "dm:aa").await.unwrap(), 200);
        incoming(&s, "m3", 300, "three").await;
        assert!(read_up_to(&s, "dm:aa", 300).await.unwrap());
        assert_eq!(get(&s, "dm:aa").await.unwrap().unwrap().unread, 0);
    }

    #[tokio::test]
    async fn a_group_chat_counts_and_shows_nothing_of_someone_i_block() {
        use crate::dm_relations::{put, RelationRow};
        let s = Store::open_in_memory().await.unwrap();
        sqlx::query("INSERT INTO msg_chats (id, kind, peer_pubkey, unread, last_message_at, last_preview, pinned, archived, muted, created_at, updated_at) VALUES ('group:g', 'group', NULL, 0, NULL, NULL, 0, 0, 0, 1, 1)")
            .execute(s.pool())
            .await
            .unwrap();
        let said = |id: &str, who: &str, at: i64| crate::messages::NewMessage {
            chat_id: "group:g".into(),
            sender_pubkey: who.into(),
            text: Some(format!("{who}{at}")),
            ..crate::messages::NewMessage {
                id: id.into(),
                chat_id: String::new(),
                wire_id: None,
                direction: "in".into(),
                status: "received".into(),
                content_type: "text".into(),
                text: None,
                envelope_json: "{}".into(),
                sender_pubkey: String::new(),
                reply_to_id: None,
                target_id: None,
                created_at: at,
                is_hidden: false,
                outbox_local_id: None,
                media_json: None,
            }
        };
        for (id, who, at) in [("a", "u", 10), ("b", "v", 20), ("c", "u", 30)] {
            crate::messages::insert(&s, &said(id, who, at)).await.unwrap();
            touch(&s, "group:g", at, Some(&format!("{who}{at}")), true).await.unwrap();
        }
        let chat = get(&s, "group:g").await.unwrap().unwrap();
        assert_eq!((chat.unread, chat.last_preview.as_deref()), (3, Some("u30")));
        let mut r = RelationRow {
            peer_pubkey: "u".into(),
            server_id: String::new(),
            my_contact: "none".into(),
            blocked: true,
            peer_signal: "none".into(),
            was_ever_mutual: false,
            last_signal_at: 0,
            last_my_signal_at: 0,
            request_floor: 0,
            decided_at: 0,
            created_at: 0,
            updated_at: 0,
        };
        put(&s, &r).await.unwrap();
        assert_eq!(group_chats_follow_block(&s, "u").await.unwrap(), vec!["group:g".to_string()]);
        let chat = get(&s, "group:g").await.unwrap().unwrap();
        assert_eq!((chat.unread, chat.last_preview.as_deref(), chat.last_message_at), (1, Some("v20"), Some(20)), "blocked: neither counted nor shown");
        assert!(group_chats_follow_block(&s, "nobody").await.unwrap().is_empty());
        // Read elsewhere while blocked: what they wrote does not count there
        // either (the counter was bumped for both here, as if they were not).
        crate::messages::insert(&s, &said("d", "u", 40)).await.unwrap();
        touch(&s, "group:g", 40, Some("u40"), true).await.unwrap();
        crate::messages::insert(&s, &said("e", "v", 50)).await.unwrap();
        touch(&s, "group:g", 50, Some("v50"), true).await.unwrap();
        assert_eq!(get(&s, "group:g").await.unwrap().unwrap().unread, 3);
        assert!(read_up_to(&s, "group:g", 15).await.unwrap(), "lowered: only v20 and v50 count");
        assert_eq!(get(&s, "group:g").await.unwrap().unwrap().unread, 2);
        // Unblocked: shown again in the list; what was never counted does not start to count.
        r.blocked = false;
        put(&s, &r).await.unwrap();
        group_chats_follow_block(&s, "u").await.unwrap();
        let chat = get(&s, "group:g").await.unwrap().unwrap();
        assert_eq!((chat.unread, chat.last_preview.as_deref()), (2, Some("v50")));
    }
}
