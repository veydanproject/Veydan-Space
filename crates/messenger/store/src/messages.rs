// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_messages`: every message row, visible or hidden. The primary key is
//! the rumor id, so the peer's copy, our self-copy and a second device's
//! copy of the same message all collapse into one row.

use std::collections::{HashMap, HashSet};

use crate::{storage, Store};
use messenger_core::envelope::{self, TextFormat};
use messenger_core::Result;
use messenger_richtext::md;

pub const DIR_IN: &str = "in";
pub const DIR_OUT: &str = "out";

pub const STATUS_QUEUED: &str = "queued";
pub const STATUS_SENT: &str = "sent";
pub const STATUS_FAILED: &str = "failed";
pub const STATUS_RECEIVED: &str = "received";

pub const CT_TEXT: &str = "text";
pub const CT_EDIT: &str = "edit";
pub const CT_DELETE: &str = "delete";
pub const CT_CONTROL: &str = "control";
pub const CT_SYSTEM: &str = "system";
pub const CT_MEDIA: &str = "media";
/// A contact card (`messenger_contacts::card`): `media_json` holds the
/// checked card, `text` is empty.
pub const CT_CONTACT: &str = "contact";
/// A card of the bot of Veydan Cloud (`messenger_dm::botcard`), written
/// only for a message the pinned key sealed: `media_json` holds the card
/// as it was read, `text` is empty. Never a type of the wire.
pub const CT_BOT_CARD: &str = "bot_card";
/// What a message is kept as when its wire type is a name only this side
/// writes (`system`, `bot_card`): nobody else makes a line or a card.
pub const CT_FOREIGN: &str = "unknown";

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct MessageRow {
    pub id: String,
    pub chat_id: String,
    pub wire_id: Option<String>,
    pub direction: String,
    pub status: String,
    pub content_type: String,
    pub text: Option<String>,
    pub envelope_json: String,
    pub sender_pubkey: String,
    pub reply_to_id: Option<String>,
    pub target_id: Option<String>,
    pub created_at: i64,
    pub received_at: i64,
    pub edited_at: Option<i64>,
    pub deleted_at: Option<i64>,
    pub is_hidden: bool,
    pub outbox_local_id: Option<String>,
    pub failure_reason: Option<String>,
    pub media_json: Option<String>,
    /// What the moderators of a group decided of it: `` (shown) or
    /// `hidden`. Never set in a direct chat.
    pub mod_state: String,
    /// Whether it waits for a moderator: `ok` (never held), `pending`,
    /// `approved`, `rejected`. `ok` in a direct chat.
    pub review: String,
    /// How `text` is written: `` plain, `md` Markdown (036).
    pub fmt: String,
    /// A Markdown `text` without its marks; `None` for plain text.
    pub plain: Option<String>,
}

/// What a handler or the sender knows when inserting.
#[derive(Clone, Debug)]
pub struct NewMessage {
    pub id: String,
    pub chat_id: String,
    pub wire_id: Option<String>,
    pub direction: String,
    pub status: String,
    pub content_type: String,
    pub text: Option<String>,
    pub envelope_json: String,
    pub sender_pubkey: String,
    pub reply_to_id: Option<String>,
    pub target_id: Option<String>,
    pub created_at: i64,
    pub is_hidden: bool,
    pub outbox_local_id: Option<String>,
    pub media_json: Option<String>,
}

pub const COLS: &str = "id, chat_id, wire_id, direction, status, content_type, text, envelope_json, sender_pubkey, reply_to_id, target_id, created_at, received_at, edited_at, deleted_at, is_hidden, outbox_local_id, failure_reason, media_json, mod_state, review, fmt, plain";

pub const MOD_SHOWN: &str = "";
pub const MOD_HIDDEN: &str = "hidden";
pub const REVIEW_OK: &str = "ok";
pub const REVIEW_PENDING: &str = "pending";
pub const REVIEW_APPROVED: &str = "approved";
pub const REVIEW_REJECTED: &str = "rejected";
/// Held when it came, shown now because the journal no longer holds its
/// author (pre-moderation off, the author trusted or raised); it waits
/// again should the journal hold them again. Shown like `ok`, but it
/// remembers that it was held, so that a decision that comes late still
/// counts and every device ends alike.
pub const REVIEW_RELEASED: &str = "released";

/// A row the chat shows: not an edit or a delete, not hidden by a
/// moderator (except my own that a moderator rejected: its author alone
/// is shown it, with the word that it was), and in a group not of someone
/// I block in direct messages (the one effect of a block in groups; the
/// unblock shows them again) nor of a key the network bans (`banned`:
/// all they wrote, while the manifest names them). The block is that of the
/// home chat (`server_id = ''`) in a group of any server: a private chat
/// on a server has a relation of its own, for that chat alone. `m` is the
/// alias of `msg_messages` in the query.
pub const SHOWN: &str = "m.is_hidden = 0 AND (m.mod_state = '' OR (m.review = 'rejected' AND m.direction = 'out')) AND NOT (m.chat_id LIKE 'group:%' AND (EXISTS (SELECT 1 FROM msg_dm_relations r WHERE r.peer_pubkey = m.sender_pubkey AND r.server_id = '' AND r.blocked = 1) OR EXISTS (SELECT 1 FROM msg_banned_keys b WHERE b.pubkey = m.sender_pubkey)))";

/// What a member sees of a group: `SHOWN`, and nothing that still waits
/// for a moderator unless it is mine. A moderator sees the waiting rows
/// too (`list` with `moderator`).
pub const SHOWN_TO_MEMBER: &str = "m.is_hidden = 0 AND (m.mod_state = '' OR (m.review = 'rejected' AND m.direction = 'out')) AND NOT (m.chat_id LIKE 'group:%' AND (EXISTS (SELECT 1 FROM msg_dm_relations r WHERE r.peer_pubkey = m.sender_pubkey AND r.server_id = '' AND r.blocked = 1) OR EXISTS (SELECT 1 FROM msg_banned_keys b WHERE b.pubkey = m.sender_pubkey))) AND (m.review <> 'pending' OR m.direction = 'out')";

/// Insert; `false` when a row with this id already exists (a duplicate copy).
pub async fn insert(store: &Store, m: &NewMessage) -> Result<bool> {
    let fmt = format_of(&m.envelope_json);
    let plain = plain_of(m.text.as_deref(), fmt);
    let res = sqlx::query(
        "INSERT OR IGNORE INTO msg_messages (id, chat_id, wire_id, direction, status, content_type, text, envelope_json, sender_pubkey, reply_to_id, target_id, created_at, received_at, edited_at, deleted_at, is_hidden, outbox_local_id, failure_reason, media_json, mod_state, review, fmt, plain)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, ?, ?, NULL, ?, '', 'ok', ?, ?)",
    )
    .bind(&m.id)
    .bind(&m.chat_id)
    .bind(&m.wire_id)
    .bind(&m.direction)
    .bind(&m.status)
    .bind(&m.content_type)
    .bind(&m.text)
    .bind(&m.envelope_json)
    .bind(&m.sender_pubkey)
    .bind(&m.reply_to_id)
    .bind(&m.target_id)
    .bind(m.created_at)
    .bind(crate::now())
    .bind(m.is_hidden)
    .bind(&m.outbox_local_id)
    .bind(&m.media_json)
    .bind(fmt.name())
    .bind(plain)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(res.rows_affected() == 1)
}

pub async fn get(store: &Store, id: &str) -> Result<Option<MessageRow>> {
    sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!("SELECT {COLS} FROM msg_messages WHERE id = ?")))
        .bind(id)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)
}

/// Visible rows of a chat, oldest first, at most `limit`, all strictly
/// older than `before` (created_at) when given. Deleted rows are included
/// (rendered as tombstones); hidden rows never (`SHOWN`). What waits for a
/// moderator is listed for a `moderator` and for its author, not for the
/// other members.
pub async fn list(store: &Store, chat_id: &str, before: Option<i64>, limit: i64, moderator: bool) -> Result<Vec<MessageRow>> {
    let shown = if moderator { SHOWN } else { SHOWN_TO_MEMBER };
    let mut rows = sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM msg_messages m WHERE m.chat_id = ? AND {shown} AND m.created_at < ?
         ORDER BY m.created_at DESC, m.id DESC LIMIT ?"
    )))
    .bind(chat_id)
    .bind(before.unwrap_or(i64::MAX))
    .bind(limit)
    .fetch_all(store.pool())
    .await
    .map_err(storage)?;
    rows.reverse();
    Ok(rows)
}

/// How many rows of a chat `list` would show (to whoever is not a
/// moderator) strictly older than `before`: whether an older page exists.
pub async fn count_before(store: &Store, chat_id: &str, before: i64) -> Result<i64> {
    sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!(
        "SELECT COUNT(*) FROM msg_messages m WHERE m.chat_id = ? AND {SHOWN_TO_MEMBER} AND m.created_at < ?"
    )))
    .bind(chat_id)
    .bind(before)
    .fetch_one(store.pool())
    .await
    .map_err(storage)
}

/// What a report attaches of a chat (`messenger_runtime::report`): the last
/// `limit` messages written by one of the two `authors` that came over the
/// wire (they have a `wire_id`) and are still there to see — not hidden,
/// not deleted, no line of the app's own — up to and including `until`
/// (`(created_at, id)` of the message the report starts from), the oldest
/// first. Whatever else the chat holds (in a group, the words of the other
/// members) is never a report on a person's to hand over.
///
/// `authors` `None` is a report on a whole group: there the conduct is the
/// group's, so the messages of any member count, each under its own
/// author's signature.
pub async fn evidence_run(
    store: &Store,
    chat_id: &str,
    authors: Option<(&str, &str)>,
    until: Option<(i64, &str)>,
    limit: i64,
) -> Result<Vec<MessageRow>> {
    let (until_at, until_id) = until.unwrap_or((i64::MAX, ""));
    let (any, (first, second)) = match authors {
        Some(two) => (0, two),
        None => (1, ("", "")),
    };
    let mut rows = sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM msg_messages
         WHERE chat_id = ? AND (? = 1 OR sender_pubkey IN (?, ?)) AND is_hidden = 0 AND deleted_at IS NULL
           AND content_type <> 'system' AND wire_id IS NOT NULL
           AND (created_at < ? OR (created_at = ? AND (? = '' OR id <= ?)))
         ORDER BY created_at DESC, id DESC LIMIT ?"
    )))
    .bind(chat_id)
    .bind(any)
    .bind(first)
    .bind(second)
    .bind(until_at)
    .bind(until_at)
    .bind(until_id)
    .bind(until_id)
    .bind(limit)
    .fetch_all(store.pool())
    .await
    .map_err(storage)?;
    rows.reverse();
    Ok(rows)
}

// ─── Moderation of groups ───────────────────────────────────────────────────

/// What the moderators decided of a message: `MOD_SHOWN` or `MOD_HIDDEN`.
/// `true` when it changed.
pub async fn set_mod_state(store: &Store, id: &str, mod_state: &str) -> Result<bool> {
    let r = sqlx::query("UPDATE msg_messages SET mod_state = ? WHERE id = ? AND mod_state <> ?")
        .bind(mod_state)
        .bind(id)
        .bind(mod_state)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(r.rows_affected() > 0)
}

/// Whether a message waits for a moderator (`REVIEW_*`). `true` when it
/// changed.
pub async fn set_review(store: &Store, id: &str, review: &str) -> Result<bool> {
    let r = sqlx::query("UPDATE msg_messages SET review = ? WHERE id = ? AND review <> ?")
        .bind(review)
        .bind(id)
        .bind(review)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(r.rows_affected() > 0)
}

/// The version of a message that an approval names: the id of the edit
/// in force (the one dated `edited_at`, by the author; the smaller id of
/// two dated alike), or the id of the message itself.
pub async fn version_of(store: &Store, row: &MessageRow) -> Result<String> {
    let Some(edited_at) = row.edited_at else { return Ok(row.id.clone()) };
    let edit = sqlx::query_scalar::<_, String>(
        "SELECT id FROM msg_messages WHERE target_id = ? AND content_type = 'edit' AND is_hidden = 1 AND sender_pubkey = ? AND created_at = ?
         ORDER BY id ASC LIMIT 1",
    )
    .bind(&row.id)
    .bind(&row.sender_pubkey)
    .bind(edited_at)
    .fetch_optional(store.pool())
    .await
    .map_err(storage)?;
    Ok(edit.unwrap_or_else(|| row.id.clone()))
}

/// Shown rows of a chat written by `author` and dated inside `from..=to`:
/// what a mark about a span of an author's messages falls on. Rows of
/// every state, hidden ones too: a span lifted shows them again.
pub async fn of_author_between(store: &Store, chat_id: &str, author: &str, from: i64, to: i64) -> Result<Vec<MessageRow>> {
    sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM msg_messages WHERE chat_id = ? AND sender_pubkey = ? AND is_hidden = 0 AND content_type <> 'system'
           AND created_at >= ? AND created_at <= ? ORDER BY created_at ASC, id ASC"
    )))
    .bind(chat_id)
    .bind(author)
    .bind(from)
    .bind(to)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// The rows of a chat that wait for a moderator: shown, not deleted,
/// `review = pending`.
const PENDING: &str = "m.chat_id = ? AND m.review = 'pending' AND m.is_hidden = 0 AND m.mod_state = '' AND m.deleted_at IS NULL";

/// What waits for a moderator, oldest first, of one author.
pub async fn pending_for_author(store: &Store, chat_id: &str, author: &str) -> Result<Vec<MessageRow>> {
    sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM msg_messages m WHERE {PENDING} AND m.sender_pubkey = ? ORDER BY m.created_at ASC, m.id ASC"
    )))
    .bind(chat_id)
    .bind(author)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// The rows of a chat that were held for a moderator and that nobody
/// decided: those that wait (`pending`) and those shown because the
/// journal no longer holds their author (`released`). What the journal
/// says of them is read again whenever it changes.
pub async fn held_rows(store: &Store, chat_id: &str) -> Result<Vec<MessageRow>> {
    sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM msg_messages WHERE chat_id = ? AND review IN ('pending', 'released') AND is_hidden = 0 AND content_type <> 'system'
         ORDER BY created_at ASC, id ASC"
    )))
    .bind(chat_id)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// The rows of a chat that were never held for a moderator (`review =
/// ok`) and are dated `since` or later: what a rule of pre-moderation
/// that came after them has still to be read against.
pub async fn ok_rows_since(store: &Store, chat_id: &str, since: i64) -> Result<Vec<MessageRow>> {
    sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM msg_messages WHERE chat_id = ? AND review = 'ok' AND created_at >= ? AND is_hidden = 0 AND content_type <> 'system' AND deleted_at IS NULL
         ORDER BY created_at ASC, id ASC"
    )))
    .bind(chat_id)
    .bind(since)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// How many rows of a chat wait for a moderator.
pub async fn count_pending(store: &Store, chat_id: &str) -> Result<i64> {
    sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM msg_messages m WHERE {PENDING}")))
        .bind(chat_id)
        .fetch_one(store.pool())
        .await
        .map_err(storage)
}

/// What of the queue to show; a field left out means "any".
#[derive(Clone, Debug, Default)]
pub struct QueueFilter {
    pub author: Option<String>,
    pub since: Option<i64>,
    pub until: Option<i64>,
    /// What a moderator searches for, case-insensitively in any script: a
    /// part of the text, or of the author's name (my nickname for them,
    /// the names and the NIP-05 of their profile), or of their key in hex.
    pub text: Option<String>,
    /// Every row its own line: the messages of one author, not the authors.
    pub each: bool,
}

/// One line of the queue: the oldest waiting row of an author and how
/// many more of theirs wait behind it (none when every row is a line).
#[derive(Clone, Debug)]
pub struct QueueLine {
    pub row: MessageRow,
    pub more_from_author: i64,
}

/// A waiting row as the queue sorts and searches it.
#[derive(sqlx::FromRow)]
struct Waiting {
    id: String,
    sender_pubkey: String,
    created_at: i64,
    text: Option<String>,
}

/// The queue of a group as a moderator pages through it: one line per
/// author (their oldest waiting row), newest first by that row, all
/// strictly older than `before`; `total` is how many rows wait in all
/// under the filter. The grouping is of the showing only: nothing is
/// dropped for it. A full page takes along every further line dated to
/// the same second as its last one, so that the next page, strictly
/// older, passes none of them by: the page is at least `limit` long, not
/// at most. The search is read in Rust, not by `LIKE`: SQLite folds the
/// case of Latin letters only, and a name lives in other tables.
pub async fn queue(store: &Store, chat_id: &str, filter: &QueueFilter, before: Option<i64>, limit: i64) -> Result<(Vec<QueueLine>, i64)> {
    let mut sql = format!("SELECT m.id, m.sender_pubkey, m.created_at, COALESCE(m.plain, m.text) AS text FROM msg_messages m WHERE {PENDING}");
    if filter.author.is_some() {
        sql.push_str(" AND m.sender_pubkey = ?");
    }
    if filter.since.is_some() {
        sql.push_str(" AND m.created_at >= ?");
    }
    if filter.until.is_some() {
        sql.push_str(" AND m.created_at <= ?");
    }
    sql.push_str(" ORDER BY m.created_at ASC, m.id ASC");
    let mut q = sqlx::query_as::<_, Waiting>(sqlx::AssertSqlSafe(sql)).bind(chat_id);
    if let Some(a) = &filter.author {
        q = q.bind(a);
    }
    if let Some(s) = filter.since {
        q = q.bind(s);
    }
    if let Some(u) = filter.until {
        q = q.bind(u);
    }
    let mut rows = q.fetch_all(store.pool()).await.map_err(storage)?;
    if let Some(needle) = filter.text.as_deref().map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty()) {
        let mut authors: Vec<&str> = rows.iter().map(|r| r.sender_pubkey.as_str()).collect();
        authors.sort_unstable();
        authors.dedup();
        let named = authors_named(store, &authors, &needle).await?;
        rows.retain(|r| named.contains(&r.sender_pubkey) || r.text.as_deref().is_some_and(|t| t.to_lowercase().contains(&needle)));
    }
    let total = rows.len() as i64;
    // (the row of the line, how many more behind it): the first row of an
    // author is their oldest, as the rows come oldest first.
    let mut lines: Vec<(&Waiting, i64)> = Vec::new();
    if filter.each {
        lines.extend(rows.iter().map(|r| (r, 0)));
    } else {
        let mut at: HashMap<&str, usize> = HashMap::new();
        for r in &rows {
            match at.get(r.sender_pubkey.as_str()) {
                Some(&i) => lines[i].1 += 1,
                None => {
                    at.insert(&r.sender_pubkey, lines.len());
                    lines.push((r, 0));
                }
            }
        }
    }
    lines.sort_by(|(x, _), (y, _)| (y.created_at, &y.id).cmp(&(x.created_at, &x.id)));
    let before = before.unwrap_or(i64::MAX);
    let mut page: Vec<(&Waiting, i64)> = Vec::new();
    for l in lines.into_iter().filter(|(r, _)| r.created_at < before) {
        if page.len() as i64 >= limit && page.last().is_some_and(|(last, _)| last.created_at != l.0.created_at) {
            break;
        }
        page.push(l);
    }
    let mut out = Vec::with_capacity(page.len());
    for (w, more) in page {
        if let Some(row) = get(store, &w.id).await? {
            out.push(QueueLine { row, more_from_author: more });
        }
    }
    Ok((out, total))
}

/// Which of `authors` the search names: by a part of my nickname for
/// them, of their profile's names or NIP-05, or of their key in hex.
async fn authors_named(store: &Store, authors: &[&str], needle: &str) -> Result<HashSet<String>> {
    let mut named = HashSet::new();
    let hex = needle.len() >= 4 && needle.bytes().all(|b| b.is_ascii_hexdigit());
    for chunk in authors.chunks(400) {
        let marks = vec!["?"; chunk.len()].join(", ");
        let sql = format!(
            "SELECT p.pubkey, p.name, p.display_name, p.nip05 FROM msg_profiles p WHERE p.pubkey IN ({marks})
             UNION ALL SELECT c.pubkey, c.nickname, NULL, NULL FROM msg_private_contacts c WHERE c.deleted_at IS NULL AND c.pubkey IN ({marks})"
        );
        let mut q = sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<String>)>(sqlx::AssertSqlSafe(sql));
        for a in chunk.iter().chain(chunk.iter()) {
            q = q.bind(*a);
        }
        for (pubkey, a, b, c) in q.fetch_all(store.pool()).await.map_err(storage)? {
            if [a, b, c].into_iter().flatten().any(|v| v.to_lowercase().contains(needle)) {
                named.insert(pubkey);
            }
        }
        if hex {
            named.extend(chunk.iter().filter(|a| a.contains(needle)).map(|a| a.to_string()));
        }
    }
    Ok(named)
}

/// The newest message the list of chats speaks of: the row
/// `chats::recompute_last` takes its line from (shown, not taken back, not
/// a system line, not waiting for a moderator, in a group not of a key
/// blocked or banned).
pub async fn last_shown(store: &Store, chat_id: &str) -> Result<Option<MessageRow>> {
    sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM msg_messages
         WHERE chat_id = ? AND is_hidden = 0 AND mod_state = '' AND review <> 'pending' AND deleted_at IS NULL AND content_type != 'system'
           AND NOT (chat_id LIKE 'group:%' AND (EXISTS (SELECT 1 FROM msg_dm_relations r WHERE r.peer_pubkey = msg_messages.sender_pubkey AND r.server_id = '' AND r.blocked = 1) OR EXISTS (SELECT 1 FROM msg_banned_keys b WHERE b.pubkey = msg_messages.sender_pubkey)))
         ORDER BY created_at DESC LIMIT 1"
    )))
    .bind(chat_id)
    .fetch_optional(store.pool())
    .await
    .map_err(storage)
}

/// Newest application time in the chat; new outgoing rumors must be later.
pub async fn last_created_at(store: &Store, chat_id: &str) -> Result<Option<i64>> {
    sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(created_at) FROM msg_messages WHERE chat_id = ?")
        .bind(chat_id)
        .fetch_one(store.pool())
        .await
        .map_err(storage)
}

/// Visible incoming rows in the chat newer than `floor` (relationship
/// matrix: "at most one visible message before approval", counted from
/// where the last episode of the relationship ended).
pub async fn count_visible_incoming_since(store: &Store, chat_id: &str, floor: i64) -> Result<i64> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM msg_messages
         WHERE chat_id = ? AND direction = 'in' AND is_hidden = 0 AND content_type != 'system' AND created_at > ?",
    )
    .bind(chat_id)
    .bind(floor)
    .fetch_one(store.pool())
    .await
    .map_err(storage)
}

/// The oldest visible incoming row newer than `floor`: the one message of
/// the peer's current request, as the queue of requests shows it.
pub async fn first_visible_incoming_since(store: &Store, chat_id: &str, floor: i64) -> Result<Option<MessageRow>> {
    sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM msg_messages
         WHERE chat_id = ? AND direction = 'in' AND is_hidden = 0 AND content_type != 'system' AND created_at > ?
         ORDER BY created_at ASC, id ASC LIMIT 1"
    )))
    .bind(chat_id)
    .bind(floor)
    .fetch_optional(store.pool())
    .await
    .map_err(storage)
}

/// Rumor time of the newest visible incoming row: the peer's clock, so it
/// can mark where an episode that I end now stops.
pub async fn last_visible_incoming_at(store: &Store, chat_id: &str) -> Result<Option<i64>> {
    sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(created_at) FROM msg_messages
         WHERE chat_id = ? AND direction = 'in' AND is_hidden = 0 AND content_type != 'system'",
    )
    .bind(chat_id)
    .fetch_one(store.pool())
    .await
    .map_err(storage)
}

pub async fn count_visible(store: &Store, chat_id: &str) -> Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM msg_messages WHERE chat_id = ? AND is_hidden = 0")
        .bind(chat_id)
        .fetch_one(store.pool())
        .await
        .map_err(storage)
}

/// Every text of a message or a transfer that holds `needle` (`outgoing`):
/// the paths of files the app keeps, read at once by the sweep of
/// `outgoing/` rather than a scan for each of its folders.
pub async fn paths_mentioning(store: &Store, needle: &str) -> Result<Vec<String>> {
    let like = format!("%{needle}%");
    sqlx::query_scalar::<_, String>(
        "SELECT media_json FROM msg_messages WHERE media_json LIKE ?1
         UNION ALL SELECT local_path FROM msg_transfers WHERE local_path LIKE ?1
         UNION ALL SELECT state_json FROM msg_transfers WHERE state_json LIKE ?1",
    )
    .bind(like)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// Every row of a chat, hidden ones included: what a key left behind in
/// the store, shown or not.
pub async fn count_in_chat(store: &Store, chat_id: &str) -> Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM msg_messages WHERE chat_id = ?")
        .bind(chat_id)
        .fetch_one(store.pool())
        .await
        .map_err(storage)
}

/// How the words of a row are written, read from the envelope it came in
/// (or, for a file still uploading, from the `fmt` its placeholder keeps
/// in `envelope_json`): the store never holds a format the wire did not.
pub fn format_of(envelope_json: &str) -> TextFormat {
    let v: serde_json::Value = serde_json::from_str(envelope_json).unwrap_or_default();
    TextFormat::from_name(v.get(envelope::FIELD_FMT).and_then(|f| f.as_str()))
}

/// The `plain` column: the text of a Markdown row without its marks.
fn plain_of(text: Option<&str>, fmt: TextFormat) -> Option<String> {
    // Marks and nothing else (`---`, `[](…)`): the text itself, not an empty line.
    fmt.is_markdown().then(|| text.map(md::plain)).flatten().filter(|p| !p.is_empty())
}

/// An edit: the text and how it is written, both from the edit.
pub async fn set_text(store: &Store, id: &str, text: &str, fmt: TextFormat, edited_at: i64) -> Result<()> {
    sqlx::query("UPDATE msg_messages SET text = ?, fmt = ?, plain = ?, edited_at = ? WHERE id = ?")
        .bind(text)
        .bind(fmt.name())
        .bind(plain_of(Some(text), fmt))
        .bind(edited_at)
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// Tombstone: content is dropped, the row stays so the order is stable.
pub async fn mark_deleted(store: &Store, id: &str, deleted_at: i64) -> Result<()> {
    sqlx::query("UPDATE msg_messages SET text = NULL, plain = NULL, media_json = NULL, deleted_at = ? WHERE id = ?")
        .bind(deleted_at)
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn set_status(store: &Store, id: &str, status: &str, failure_reason: Option<&str>) -> Result<()> {
    sqlx::query("UPDATE msg_messages SET status = ?, failure_reason = ? WHERE id = ?")
        .bind(status)
        .bind(failure_reason)
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn set_outbox_local_id(store: &Store, id: &str, local_id: Option<&str>) -> Result<()> {
    sqlx::query("UPDATE msg_messages SET outbox_local_id = ? WHERE id = ?")
        .bind(local_id)
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// Hidden edit/delete rows that arrived before their target.
pub async fn pending_for_target(store: &Store, target_id: &str) -> Result<Vec<MessageRow>> {
    sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM msg_messages WHERE target_id = ? AND is_hidden = 1 ORDER BY created_at ASC"
    )))
    .bind(target_id)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// Ids of messages that changed status because their outbox row finished:
/// `(now sent, now failed)`. Out is sent, whatever was shown before; given
/// up (past its deadline, or refused) is failed. A row that is only being
/// retried leaves the message waiting.
pub async fn sync_outbox_status(store: &Store) -> Result<(Vec<String>, Vec<String>)> {
    let sent = sqlx::query_scalar::<_, String>(
        "SELECT m.id FROM msg_messages m JOIN msg_outbox o ON o.local_id = m.outbox_local_id
         WHERE m.status IN ('queued', 'failed') AND o.state = 'published'",
    )
    .fetch_all(store.pool())
    .await
    .map_err(storage)?;
    for id in &sent {
        set_status(store, id, STATUS_SENT, None).await?;
    }
    let failed = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT m.id, o.last_error FROM msg_messages m JOIN msg_outbox o ON o.local_id = m.outbox_local_id
         WHERE m.status = 'queued' AND o.state = 'abandoned'",
    )
    .fetch_all(store.pool())
    .await
    .map_err(storage)?;
    let mut failed_ids = Vec::with_capacity(failed.len());
    for (id, err) in failed {
        set_status(store, &id, STATUS_FAILED, err.as_deref()).await?;
        failed_ids.push(id);
    }
    Ok((sent, failed_ids))
}

/// Wire ids and application times of the messages we hold, for history
/// reconciliation. Only rows that came from the wire have a `wire_id`.
pub async fn count_all(store: &Store) -> Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM msg_messages")
        .fetch_one(store.pool())
        .await
        .map_err(storage)
}

/// Visible messages we sent in this chat (system rows excluded).
pub async fn count_visible_outgoing(store: &Store, chat_id: &str) -> Result<i64> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM msg_messages WHERE chat_id = ? AND direction = 'out' AND is_hidden = 0 AND content_type != 'system'",
    )
    .bind(chat_id)
    .fetch_one(store.pool())
    .await
    .map_err(storage)
}

/// The message was removed for me (here or on another device of mine):
/// remembered by id, so a copy that comes later is removed as well.
pub async fn remember_hidden(store: &Store, id: &str, at: i64) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO msg_own_hidden (message_id, hidden_at) VALUES (?, ?)")
        .bind(id)
        .bind(at)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// When the message was removed for me, if it was.
pub async fn hidden_at(store: &Store, id: &str) -> Result<Option<i64>> {
    sqlx::query_scalar::<_, i64>("SELECT hidden_at FROM msg_own_hidden WHERE message_id = ?")
        .bind(id)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)
}

pub const STATUS_UPLOADING: &str = "uploading";

/// Remove one row for good (placeholders of uploads).
pub async fn delete(store: &Store, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM msg_messages WHERE id = ?").bind(id).execute(store.pool()).await.map_err(storage)?;
    Ok(())
}

pub async fn set_media_json(store: &Store, id: &str, media_json: &str) -> Result<()> {
    sqlx::query("UPDATE msg_messages SET media_json = ? WHERE id = ?")
        .bind(media_json)
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// After a restart no upload is running: placeholders show as paused, all
/// but these (another process with the data folder may be sending them).
pub async fn pause_uploading_except(store: &Store, ids: &[String]) -> Result<u64> {
    let marks = vec!["?"; ids.len()].join(", ");
    let sql = format!("UPDATE msg_messages SET status = 'paused' WHERE status = 'uploading' AND id NOT IN ({marks})");
    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
    for id in ids {
        q = q.bind(id);
    }
    Ok(q.execute(store.pool()).await.map_err(storage)?.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(id: &str, chat: &str, dir: &str, at: i64) -> NewMessage {
        NewMessage {
            id: id.into(),
            chat_id: chat.into(),
            wire_id: None,
            direction: dir.into(),
            status: if dir == DIR_OUT { STATUS_QUEUED.into() } else { STATUS_RECEIVED.into() },
            content_type: CT_TEXT.into(),
            text: Some(format!("m{id}")),
            envelope_json: "{}".into(),
            sender_pubkey: "s".into(),
            reply_to_id: None,
            target_id: None,
            created_at: at,
            is_hidden: false,
            outbox_local_id: None,
            media_json: None,
        }
    }

    /// A report's run: the two parties' messages off the wire, newest ones,
    /// up to the one it starts from, nothing hidden, deleted or local.
    #[tokio::test]
    async fn the_run_of_a_report_is_of_the_two_and_ends_where_it_starts() {
        let s = Store::open_in_memory().await.unwrap();
        let row = |id: &str, by: &str, at: i64| NewMessage { wire_id: Some(format!("w{id}")), sender_pubkey: by.into(), ..text(id, "g", DIR_IN, at) };
        for (id, by, at) in [("a1", "acc", 10), ("m1", "me", 20), ("o1", "other", 30), ("a2", "acc", 40), ("a3", "acc", 40), ("m2", "me", 50)] {
            insert(&s, &row(id, by, at)).await.unwrap();
        }
        insert(&s, &NewMessage { is_hidden: true, ..row("h1", "acc", 45) }).await.unwrap();
        insert(&s, &NewMessage { content_type: CT_SYSTEM.into(), ..row("s1", "acc", 46) }).await.unwrap();
        insert(&s, &NewMessage { wire_id: None, ..row("u1", "me", 47) }).await.unwrap();
        insert(&s, &row("d1", "acc", 48)).await.unwrap();
        mark_deleted(&s, "d1", 49).await.unwrap();
        insert(&s, &NewMessage { chat_id: "elsewhere".into(), ..row("x1", "acc", 60) }).await.unwrap();

        let ids = |rows: Vec<MessageRow>| rows.into_iter().map(|r| r.id).collect::<Vec<_>>();
        let all = evidence_run(&s, "g", Some(("acc", "me")), None, 10).await.unwrap();
        assert_eq!(ids(all), vec!["a1", "m1", "a2", "a3", "m2"], "the two only, oldest first");
        let last = evidence_run(&s, "g", Some(("acc", "me")), None, 2).await.unwrap();
        assert_eq!(ids(last), vec!["a3", "m2"], "the newest ones");
        let upto = evidence_run(&s, "g", Some(("acc", "me")), Some((40, "a2")), 10).await.unwrap();
        assert_eq!(ids(upto), vec!["a1", "m1", "a2"], "inclusive of the start, not its twin of the same second");
        let group = evidence_run(&s, "g", None, None, 10).await.unwrap();
        assert_eq!(ids(group), vec!["a1", "m1", "o1", "a2", "a3", "m2"], "on a group: every member's, the same rows left out");
    }

    #[tokio::test]
    async fn insert_dedup_list_and_pagination() {
        let s = Store::open_in_memory().await.unwrap();
        for i in 1..=5 {
            assert!(insert(&s, &text(&i.to_string(), "c", DIR_IN, i * 10)).await.unwrap());
        }
        assert!(!insert(&s, &text("3", "c", DIR_IN, 30)).await.unwrap(), "duplicate copy ignored");
        let mut hidden = text("h", "c", DIR_IN, 35);
        hidden.is_hidden = true;
        hidden.content_type = CT_EDIT.into();
        hidden.target_id = Some("3".into());
        insert(&s, &hidden).await.unwrap();

        let page = list(&s, "c", None, 2, false).await.unwrap();
        assert_eq!(page.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["4", "5"], "newest page, oldest first");
        let older = list(&s, "c", Some(40), 10, false).await.unwrap();
        assert_eq!(older.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["1", "2", "3"], "hidden rows never listed");
        assert_eq!(last_created_at(&s, "c").await.unwrap(), Some(50));
        assert_eq!(count_visible_incoming_since(&s, "c", 0).await.unwrap(), 5);
        assert_eq!(count_visible_incoming_since(&s, "c", 30).await.unwrap(), 2, "only what came after the floor");
        assert_eq!(last_visible_incoming_at(&s, "c").await.unwrap(), Some(50));
        assert_eq!(last_visible_incoming_at(&s, "nobody").await.unwrap(), None);
        assert_eq!(pending_for_target(&s, "3").await.unwrap().len(), 1);

        set_text(&s, "3", "changed", TextFormat::Plain, 99).await.unwrap();
        let m = get(&s, "3").await.unwrap().unwrap();
        assert_eq!(m.text.as_deref(), Some("changed"));
        assert_eq!(m.edited_at, Some(99));
        mark_deleted(&s, "3", 100).await.unwrap();
        let m = get(&s, "3").await.unwrap().unwrap();
        assert!(m.text.is_none() && m.deleted_at == Some(100));
        assert_eq!(count_visible_incoming_since(&s, "c", 0).await.unwrap(), 5, "a retracted request still was a request");
        assert_eq!(count_visible_outgoing(&s, "c").await.unwrap(), 0);
    }

    /// A row of a group chat by `who`, dated `at`.
    fn said(id: &str, who: &str, at: i64) -> NewMessage {
        NewMessage { sender_pubkey: who.into(), ..text(id, "group:g", DIR_IN, at) }
    }

    #[tokio::test]
    async fn what_moderators_decided_and_what_waits_for_them_is_listed_by_role() {
        let s = Store::open_in_memory().await.unwrap();
        for (id, who, at) in [("a", "u", 10), ("b", "u", 20), ("c", "v", 30), ("d", "v", 40)] {
            insert(&s, &said(id, who, at)).await.unwrap();
        }
        let mut mine = said("m", "me", 50);
        mine.direction = DIR_OUT.into();
        insert(&s, &mine).await.unwrap();
        let ids = |rows: Vec<MessageRow>| rows.into_iter().map(|r| r.id).collect::<Vec<_>>();
        assert_eq!(ids(list(&s, "group:g", None, 10, false).await.unwrap()), vec!["a", "b", "c", "d", "m"]);
        assert_eq!(get(&s, "a").await.unwrap().unwrap().review, REVIEW_OK, "never held");
        // Hidden by a moderator: listed for nobody. Waiting: for a moderator
        // and for its author.
        assert!(set_mod_state(&s, "a", MOD_HIDDEN).await.unwrap());
        assert!(!set_mod_state(&s, "a", MOD_HIDDEN).await.unwrap(), "unchanged");
        assert!(set_review(&s, "b", REVIEW_PENDING).await.unwrap());
        assert!(set_review(&s, "c", REVIEW_PENDING).await.unwrap());
        assert!(set_review(&s, "m", REVIEW_PENDING).await.unwrap());
        assert_eq!(ids(list(&s, "group:g", None, 10, false).await.unwrap()), vec!["d", "m"], "a member: what is shown and my own");
        assert_eq!(ids(list(&s, "group:g", None, 10, true).await.unwrap()), vec!["b", "c", "d", "m"], "a moderator: the waiting too");
        assert_eq!(count_pending(&s, "group:g").await.unwrap(), 3);
        assert_eq!(ids(pending_for_author(&s, "group:g", "u").await.unwrap()), vec!["b"]);
        // The queue: one line per author, their oldest, newest line first.
        let (lines, total) = queue(&s, "group:g", &QueueFilter::default(), None, 10).await.unwrap();
        assert_eq!(total, 3);
        assert_eq!(lines.iter().map(|l| (l.row.id.as_str(), l.more_from_author)).collect::<Vec<_>>(), vec![("m", 0), ("c", 0), ("b", 0)]);
        insert(&s, &said("e", "v", 60)).await.unwrap();
        set_review(&s, "e", REVIEW_PENDING).await.unwrap();
        let (lines, total) = queue(&s, "group:g", &QueueFilter::default(), None, 10).await.unwrap();
        assert_eq!(total, 4);
        assert_eq!(lines.iter().map(|l| (l.row.id.as_str(), l.more_from_author)).collect::<Vec<_>>(), vec![("m", 0), ("c", 1), ("b", 0)]);
        let (page, _) = queue(&s, "group:g", &QueueFilter::default(), Some(50), 1).await.unwrap();
        assert_eq!(page[0].row.id, "c", "strictly older than `before`, one at a time");
        let by_text = QueueFilter { text: Some("ME".into()), ..Default::default() };
        let (lines, total) = queue(&s, "group:g", &by_text, None, 10).await.unwrap();
        assert_eq!((total, lines.len(), lines[0].row.id.as_str()), (1, 1, "e"), "case does not matter; `%` and `_` are no wildcards");
        let wild = QueueFilter { text: Some("%".into()), ..Default::default() };
        assert_eq!(queue(&s, "group:g", &wild, None, 10).await.unwrap().1, 0);
        set_text(&s, "c", "Привет, Мир", TextFormat::Plain, 31).await.unwrap();
        let cyrillic = QueueFilter { text: Some("мир".into()), ..Default::default() };
        assert_eq!(queue(&s, "group:g", &cyrillic, None, 10).await.unwrap().0[0].row.id, "c", "the case of any script, not of Latin alone");
        sqlx::query("INSERT INTO msg_profiles (pubkey, display_name, fetched_at) VALUES ('v', 'Вера', 1)").execute(s.pool()).await.unwrap();
        let by_name = QueueFilter { text: Some("вер".into()), ..Default::default() };
        let (lines, total) = queue(&s, "group:g", &by_name, None, 10).await.unwrap();
        assert_eq!((total, lines.len(), lines[0].row.id.as_str(), lines[0].more_from_author), (2, 1, "c", 1), "an author by their name: everything of theirs");
        let each = QueueFilter { author: Some("v".into()), each: true, ..Default::default() };
        let (lines, total) = queue(&s, "group:g", &each, None, 10).await.unwrap();
        assert_eq!((total, lines.iter().map(|l| (l.row.id.as_str(), l.more_from_author)).collect::<Vec<_>>()), (2, vec![("e", 0), ("c", 0)]), "every row its own line");
        let of_v = QueueFilter { author: Some("v".into()), since: Some(31), until: Some(60), ..Default::default() };
        let (lines, total) = queue(&s, "group:g", &of_v, None, 10).await.unwrap();
        assert_eq!((total, lines[0].row.id.as_str(), lines[0].more_from_author), (1, "e", 0), "the count behind a line is under the filter too");
        // The span of an author's messages, hidden ones included.
        assert_eq!(ids(of_author_between(&s, "group:g", "u", 0, 15).await.unwrap()), vec!["a"]);
        assert_eq!(ids(of_author_between(&s, "group:g", "u", 0, 100).await.unwrap()), vec!["a", "b"]);
        // The version: the message, or the edit in force.
        let b = get(&s, "b").await.unwrap().unwrap();
        assert_eq!(version_of(&s, &b).await.unwrap(), "b");
        let mut e1 = text("e1", "group:g", DIR_IN, 25);
        e1.sender_pubkey = "u".into();
        e1.is_hidden = true;
        e1.content_type = CT_EDIT.into();
        e1.target_id = Some("b".into());
        insert(&s, &e1).await.unwrap();
        set_text(&s, "b", "changed", TextFormat::Plain, 25).await.unwrap();
        let b = get(&s, "b").await.unwrap().unwrap();
        assert_eq!(version_of(&s, &b).await.unwrap(), "e1");
        let mut e0 = e1.clone();
        e0.id = "e0".into();
        insert(&s, &e0).await.unwrap();
        assert_eq!(version_of(&s, &b).await.unwrap(), "e0", "dated alike: the smaller id");
    }

    #[tokio::test]
    async fn someone_i_block_is_not_listed_in_a_group_until_the_unblock() {
        use crate::dm_relations::{put, RelationRow};
        let s = Store::open_in_memory().await.unwrap();
        insert(&s, &said("a", "u", 10)).await.unwrap();
        insert(&s, &said("b", "v", 20)).await.unwrap();
        insert(&s, &text("d", "dm:u", DIR_IN, 30)).await.unwrap();
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
        let ids = |rows: Vec<MessageRow>| rows.into_iter().map(|r| r.id).collect::<Vec<_>>();
        assert_eq!(ids(list(&s, "group:g", None, 10, false).await.unwrap()), vec!["b"]);
        assert_eq!(ids(list(&s, "group:g", None, 10, true).await.unwrap()), vec!["b"], "a moderator blocks like anyone");
        assert_eq!(ids(list(&s, "dm:u", None, 10, false).await.unwrap()), vec!["d"], "the direct chat keeps its rows");
        r.blocked = false;
        put(&s, &r).await.unwrap();
        assert_eq!(ids(list(&s, "group:g", None, 10, false).await.unwrap()), vec!["a", "b"]);
    }

    #[tokio::test]
    async fn every_placeholder_but_those_named_is_paused() {
        let s = Store::open_in_memory().await.unwrap();
        for id in ["mine", "lost", "theirs", "sent"] {
            let mut m = text(id, "c", DIR_OUT, 1);
            m.status = if id == "sent" { STATUS_SENT.into() } else { STATUS_UPLOADING.into() };
            insert(&s, &m).await.unwrap();
        }
        let named = ["theirs".to_string(), "sent".to_string()];
        assert_eq!(pause_uploading_except(&s, &named).await.unwrap(), 2);
        let status = async |id| get(&s, id).await.unwrap().unwrap().status;
        assert_eq!(status("mine").await, "paused");
        assert_eq!(status("lost").await, "paused", "nothing named sends it");
        assert_eq!(status("theirs").await, STATUS_UPLOADING, "another process may be sending it");
        assert_eq!(status("sent").await, STATUS_SENT);
        set_status(&s, "mine", STATUS_UPLOADING, None).await.unwrap();
        assert_eq!(pause_uploading_except(&s, &[]).await.unwrap(), 2, "with none named, all");
    }

    #[tokio::test]
    async fn statuses_follow_the_outbox() {
        use crate::outbox;
        use messenger_core::{Outbound, SubId};
        let s = Store::open_in_memory().await.unwrap();
        let out = Outbound::Unsubscribe { id: SubId("x".into()) };
        for (id, local) in [("a", "ok"), ("b", "bad"), ("c", "slow"), ("d", "late")] {
            outbox::enqueue(&s, local, &out, 0, Some(3600)).await.unwrap();
            let mut m = text(id, "c", DIR_OUT, 1);
            m.outbox_local_id = Some(local.into());
            insert(&s, &m).await.unwrap();
        }

        assert_eq!(sync_outbox_status(&s).await.unwrap(), (vec![], vec![]));
        outbox::mark_published(&s, "ok").await.unwrap();
        for _ in 0..5 {
            outbox::mark_publishing(&s, "slow").await.unwrap();
            outbox::mark_failed(&s, "slow", "transport error: no relay connected", 0, false).await.unwrap();
        }
        outbox::mark_abandoned(&s, "bad", "expired: offline").await.unwrap();
        let (sent, failed) = sync_outbox_status(&s).await.unwrap();
        assert_eq!(sent, vec!["a".to_string()]);
        assert_eq!(failed, vec!["b".to_string()]);
        assert_eq!(get(&s, "b").await.unwrap().unwrap().failure_reason.as_deref(), Some("expired: offline"));
        assert_eq!(get(&s, "c").await.unwrap().unwrap().status, STATUS_QUEUED, "retried, not given up: waiting");
        assert_eq!(sync_outbox_status(&s).await.unwrap(), (vec![], vec![]), "idempotent");

        // Shown as failed, then out after all (a retry, or the last try): sent.
        set_status(&s, "d", STATUS_FAILED, Some("offline")).await.unwrap();
        outbox::mark_published(&s, "late").await.unwrap();
        assert_eq!(sync_outbox_status(&s).await.unwrap(), (vec!["d".to_string()], vec![]));
        let d = get(&s, "d").await.unwrap().unwrap();
        assert_eq!((d.status.as_str(), d.failure_reason), (STATUS_SENT, None));
    }
}
