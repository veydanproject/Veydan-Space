// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `cm_posts`, `cm_comments`, `cm_reactions`, `cm_read`: the content of
//! the feed as opened on this device, and what was read here (never on
//! the wire: §4, spec §10.4). `review` and `mod_state` are what the marks
//! of moderation say (`marks::resolve`); `version` is what an approval
//! names: the id of the newest edit, or the message's own.

use super::{now, storage, Db};
use crate::marks::{REVIEW_OK, REVIEW_PENDING};
use messenger_core::Result;

/// `mod_state` of a message the marks hide.
pub const MOD_HIDDEN: &str = "hidden";
/// `mod_state` of a message they show.
pub const MOD_SHOWN: &str = "";

/// A post, or a comment (`post_id` set).
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct MessageRow {
    pub id: String,
    pub community_id: String,
    /// Of a comment: the post it is under. Empty for a post.
    pub post_id: String,
    /// Of a reply in a branch: the comment it answers.
    pub parent_id: Option<String>,
    pub author: String,
    pub created_at: i64,
    pub epoch: i64,
    pub envelope_json: String,
    pub text: Option<String>,
    pub media_json: Option<String>,
    pub version: String,
    pub review: String,
    pub mod_state: String,
    pub deleted: bool,
    pub received_at: i64,
    /// The scope (section) it is sealed in; a comment is in its post's.
    pub scope_id: String,
    /// The sealed event (kind 1390) it came in, when known: named to the
    /// relays as held when a page of the past is asked for.
    pub outer_id: Option<String>,
    /// When the edit in force was written (its author's clock, unix
    /// seconds); 0 for a message never edited. Not written by `insert`.
    pub edited_at: i64,
}

impl MessageRow {
    pub fn is_post(&self) -> bool {
        self.post_id.is_empty()
    }
}

const POST_COLS: &str = "id, community_id, '' AS post_id, NULL AS parent_id, author, created_at, epoch, envelope_json, text, media_json, version, review, mod_state, deleted, received_at, scope_id, outer_id, edited_at";
const COMMENT_COLS: &str = "id, community_id, post_id, parent_id, author, created_at, epoch, envelope_json, text, media_json, version, review, mod_state, deleted, received_at, scope_id, outer_id, edited_at";

/// Keep a message; `false` when it is here already. Posts and comments go
/// to their own tables, by `post_id`.
pub async fn insert(db: &Db, m: &MessageRow) -> Result<bool> {
    let res = if m.is_post() {
        sqlx::query(
            "INSERT OR IGNORE INTO cm_posts (id, community_id, author, created_at, epoch, envelope_json, text, media_json, version, review, mod_state, deleted, received_at, scope_id, outer_id)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&m.id)
        .bind(&m.community_id)
        .bind(&m.author)
        .bind(m.created_at)
        .bind(m.epoch)
        .bind(&m.envelope_json)
        .bind(&m.text)
        .bind(&m.media_json)
        .bind(&m.version)
        .bind(&m.review)
        .bind(&m.mod_state)
        .bind(m.deleted)
        .bind(now())
        .bind(&m.scope_id)
        .bind(&m.outer_id)
        .execute(db.pool())
        .await
    } else {
        sqlx::query(
            "INSERT OR IGNORE INTO cm_comments (id, community_id, post_id, parent_id, author, created_at, epoch, envelope_json, text, media_json, version, review, mod_state, deleted, received_at, scope_id, outer_id)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&m.id)
        .bind(&m.community_id)
        .bind(&m.post_id)
        .bind(&m.parent_id)
        .bind(&m.author)
        .bind(m.created_at)
        .bind(m.epoch)
        .bind(&m.envelope_json)
        .bind(&m.text)
        .bind(&m.media_json)
        .bind(&m.version)
        .bind(&m.review)
        .bind(&m.mod_state)
        .bind(m.deleted)
        .bind(now())
        .bind(&m.scope_id)
        .bind(&m.outer_id)
        .execute(db.pool())
        .await
    };
    Ok(res.map_err(storage)?.rows_affected() == 1)
}

/// A post or a comment by its id.
pub async fn get(db: &Db, id: &str) -> Result<Option<MessageRow>> {
    let post = sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!("SELECT {POST_COLS} FROM cm_posts WHERE id = ?")))
        .bind(id)
        .fetch_optional(db.pool())
        .await
        .map_err(storage)?;
    if post.is_some() {
        return Ok(post);
    }
    sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!("SELECT {COMMENT_COLS} FROM cm_comments WHERE id = ?")))
        .bind(id)
        .fetch_optional(db.pool())
        .await
        .map_err(storage)
}

/// The posts and comments of `author` in a channel sealed under an epoch
/// below `epoch` and written at `from` or later: what a device an epoch
/// behind sealed after a newer one began (`ChannelService::reseal`).
pub async fn written_behind(db: &Db, community_id: &str, scope_id: &str, author: &str, epoch: i64, from: i64) -> Result<Vec<MessageRow>> {
    let mut out = Vec::new();
    for cols_table in [(POST_COLS, "cm_posts"), (COMMENT_COLS, "cm_comments")] {
        let (cols, table) = cols_table;
        out.extend(
            sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!("SELECT {cols} FROM {table} WHERE community_id = ? AND scope_id = ? AND author = ? AND epoch < ? AND created_at >= ? AND deleted = 0")))
                .bind(community_id)
                .bind(scope_id)
                .bind(author)
                .bind(epoch)
                .bind(from)
                .fetch_all(db.pool())
                .await
                .map_err(storage)?,
        );
    }
    Ok(out)
}

/// A message of mine sealed again under `epoch`.
pub async fn set_epoch(db: &Db, id: &str, epoch: i64) -> Result<()> {
    for table in ["cm_posts", "cm_comments"] {
        sqlx::query(sqlx::AssertSqlSafe(format!("UPDATE {table} SET epoch = ? WHERE id = ?"))).bind(epoch).bind(id).execute(db.pool()).await.map_err(storage)?;
    }
    Ok(())
}

/// Posts of the feed, newest first, strictly older than `before`.
pub async fn posts(db: &Db, community_id: &str, before: Option<i64>, limit: i64) -> Result<Vec<MessageRow>> {
    sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {POST_COLS} FROM cm_posts WHERE community_id = ? AND created_at < ? ORDER BY created_at DESC, id LIMIT ?"
    )))
    .bind(community_id)
    .bind(before.unwrap_or(i64::MAX))
    .bind(limit.clamp(1, 500))
    .fetch_all(db.pool())
    .await
    .map_err(storage)
}

/// The sealed events (kind 1390) the posts and comments of a channel came
/// in, the newest `limit` by their date: what the relays need not send
/// again when a page of the past is asked for.
pub async fn outer_items(db: &Db, community_id: &str, limit: i64) -> Result<Vec<(String, i64)>> {
    sqlx::query_as::<_, (String, i64)>(
        "SELECT outer_id, created_at FROM (
            SELECT outer_id, created_at FROM cm_posts WHERE community_id = ? AND outer_id IS NOT NULL
            UNION ALL
            SELECT outer_id, created_at FROM cm_comments WHERE community_id = ? AND outer_id IS NOT NULL
         ) ORDER BY created_at DESC LIMIT ?",
    )
    .bind(community_id)
    .bind(community_id)
    .bind(limit.clamp(1, 10_000))
    .fetch_all(db.pool())
    .await
    .map_err(storage)
}

/// What a reader sees of a message: released by the marks and not
/// hidden by them, or one of their own. A moderator (`viewer` none) sees
/// everything, with what the marks say.
const VISIBLE: &str = "((review IN ('ok', 'approved', 'released') AND mod_state = '') OR author = ?)";

/// A page of the feed, newest first, strictly before `before` in the
/// order `(created_at, id)`, as `viewer` sees it (none: a moderator, who
/// sees everything). The id in the cursor: posts of one second at the
/// edge of a page are not skipped.
pub async fn feed(db: &Db, community_id: &str, before: Option<(i64, &str)>, limit: i64, viewer: Option<&str>) -> Result<Vec<MessageRow>> {
    feed_in(db, community_id, before, limit, viewer, None).await
}

/// The newest post of every channel at once, as `feed(.., 1, viewer)` gives
/// it for each but a deleted one: the channels of `seeing_all` (a
/// moderator and above) show every post, the others what is released and
/// the posts of `me`. One query for the list of chats, which asked once
/// per channel. By the date of the post alone: what came under it
/// (comments, reactions, edits, marks) and reading it never move it.
pub async fn last_posts(db: &Db, me: &str, seeing_all: &[String]) -> Result<std::collections::HashMap<String, MessageRow>> {
    let all = serde_json::to_string(seeing_all).unwrap_or_else(|_| "[]".into());
    let rows = sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!(
        "SELECT * FROM (SELECT {POST_COLS}, ROW_NUMBER() OVER (PARTITION BY community_id ORDER BY created_at DESC, id DESC) AS rn
           FROM cm_posts WHERE deleted = 0 AND (community_id IN (SELECT value FROM json_each(?)) OR {VISIBLE})) WHERE rn = 1"
    )))
    .bind(&all)
    .bind(me)
    .fetch_all(db.pool())
    .await
    .map_err(storage)?;
    Ok(rows.into_iter().map(|m| (m.community_id.clone(), m)).collect())
}

/// `last_posts` of one channel, of the posts of `scopes` only: the newest
/// not deleted that `viewer` (none: a moderator) sees in the sections he
/// reads (wire §14.3).
pub async fn last_post_in(db: &Db, community_id: &str, viewer: Option<&str>, scopes: &[String]) -> Result<Option<MessageRow>> {
    let shown = if viewer.is_some() { VISIBLE } else { "(1 = 1 OR author = ?)" };
    let scopes = serde_json::to_string(scopes).unwrap_or_else(|_| "[]".into());
    sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {POST_COLS} FROM cm_posts WHERE community_id = ? AND deleted = 0 AND {shown} AND scope_id IN (SELECT value FROM json_each(?))
         ORDER BY created_at DESC, id DESC LIMIT 1"
    )))
    .bind(community_id)
    .bind(viewer.unwrap_or(""))
    .bind(&scopes)
    .fetch_optional(db.pool())
    .await
    .map_err(storage)
}

/// The scopes a page shows, as a JSON list bound to `IN_SCOPES` (empty:
/// every scope).
fn scopes_json(scopes: Option<&[String]>) -> String {
    scopes.map(|s| serde_json::to_string(s).unwrap_or_else(|_| "[]".into())).unwrap_or_default()
}

/// Of the scopes of the list bound twice (none: every scope).
const IN_SCOPES: &str = "(? = '' OR scope_id IN (SELECT value FROM json_each(?)))";

/// `feed`, of the posts of `scopes` only (wire §14.3: a reader sees the
/// sections they belong to; none: every scope).
pub async fn feed_in(db: &Db, community_id: &str, before: Option<(i64, &str)>, limit: i64, viewer: Option<&str>, scopes: Option<&[String]>) -> Result<Vec<MessageRow>> {
    let shown = if viewer.is_some() { VISIBLE } else { "(1 = 1 OR author = ?)" };
    let (at, id) = before.unwrap_or((i64::MAX, ""));
    let scopes = scopes_json(scopes);
    sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {POST_COLS} FROM cm_posts WHERE community_id = ? AND (created_at < ? OR (created_at = ? AND id < ?)) AND {shown} AND {IN_SCOPES}
         ORDER BY created_at DESC, id DESC LIMIT ?"
    )))
    .bind(community_id)
    .bind(at)
    .bind(at)
    .bind(id)
    .bind(viewer.unwrap_or(""))
    .bind(&scopes)
    .bind(&scopes)
    .bind(limit.clamp(1, 500))
    .fetch_all(db.pool())
    .await
    .map_err(storage)
}

/// A page of the comments under a post: the newest `limit` strictly
/// before `before` in the order `(created_at, id)`, given oldest first,
/// as `viewer` sees them.
pub async fn thread(db: &Db, community_id: &str, post_id: &str, before: Option<(i64, &str)>, limit: i64, viewer: Option<&str>) -> Result<Vec<MessageRow>> {
    thread_in(db, community_id, post_id, before, limit, viewer, None).await
}

/// `thread`, of the comments of `scopes` only (none: every scope).
pub async fn thread_in(
    db: &Db,
    community_id: &str,
    post_id: &str,
    before: Option<(i64, &str)>,
    limit: i64,
    viewer: Option<&str>,
    scopes: Option<&[String]>,
) -> Result<Vec<MessageRow>> {
    let shown = if viewer.is_some() { VISIBLE } else { "(1 = 1 OR author = ?)" };
    let (at, id) = before.unwrap_or((i64::MAX, ""));
    let scopes = scopes_json(scopes);
    let mut rows = sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COMMENT_COLS} FROM cm_comments WHERE community_id = ? AND post_id = ? AND (created_at < ? OR (created_at = ? AND id < ?)) AND {shown} AND {IN_SCOPES}
         ORDER BY created_at DESC, id DESC LIMIT ?"
    )))
    .bind(community_id)
    .bind(post_id)
    .bind(at)
    .bind(at)
    .bind(id)
    .bind(viewer.unwrap_or(""))
    .bind(&scopes)
    .bind(&scopes)
    .bind(limit.clamp(1, 500))
    .fetch_all(db.pool())
    .await
    .map_err(storage)?;
    rows.reverse();
    Ok(rows)
}

/// The oldest date of the stream of the feed held here (posts and
/// comments) before `before`: where the next page asked of the relays
/// ends. The relays cannot be asked for posts alone (a comment differs
/// from a post only by a tag a filter cannot ask to be absent), so the
/// pages walk back through the stream. What waits here (for a key, for a
/// snapshot) is of the stream too: a reader new to a busy channel may hold
/// nothing else of the newest page.
pub async fn oldest_in_feed(db: &Db, community_id: &str, before: Option<i64>) -> Result<Option<i64>> {
    sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MIN(created_at) FROM (SELECT created_at FROM cm_posts WHERE community_id = ?1 AND created_at < ?2
           UNION ALL SELECT created_at FROM cm_comments WHERE community_id = ?1 AND created_at < ?2
           UNION ALL SELECT created_at FROM cm_pending WHERE community_id = ?1 AND created_at < ?2)",
    )
    .bind(community_id)
    .bind(before.unwrap_or(i64::MAX))
    .fetch_one(db.pool())
    .await
    .map_err(storage)
}

/// How many comments a post has (all of them, as kept here), in its
/// channel: a post id named by a comment of another channel is not it.
pub async fn count_comments(db: &Db, community_id: &str, post_id: &str) -> Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM cm_comments WHERE community_id = ? AND post_id = ? AND deleted = 0")
        .bind(community_id)
        .bind(post_id)
        .fetch_one(db.pool())
        .await
        .map_err(storage)
}

/// `count_comments` of the posts of a page at once, by post (one with none
/// is not named): a page asked one count a post (a switch of sections,
/// 2026-10-23).
pub async fn count_comments_of(db: &Db, community_id: &str, post_ids: &[&str]) -> Result<std::collections::HashMap<String, i64>> {
    if post_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let ids = serde_json::to_string(post_ids).unwrap_or_else(|_| "[]".into());
    let rows = sqlx::query_as::<_, (String, i64)>(
        "SELECT post_id, COUNT(*) FROM cm_comments WHERE community_id = ? AND post_id IN (SELECT value FROM json_each(?)) AND deleted = 0 GROUP BY post_id",
    )
    .bind(community_id)
    .bind(ids)
    .fetch_all(db.pool())
    .await
    .map_err(storage)?;
    Ok(rows.into_iter().collect())
}

/// Comments under a post, oldest first.
pub async fn comments(db: &Db, community_id: &str, post_id: &str) -> Result<Vec<MessageRow>> {
    sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!("SELECT {COMMENT_COLS} FROM cm_comments WHERE community_id = ? AND post_id = ? ORDER BY created_at, id")))
        .bind(community_id)
        .bind(post_id)
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

/// What the marks say of a message changed. `true` when a row changed.
pub async fn set_resolution(db: &Db, id: &str, review: &str, mod_state: &str) -> Result<bool> {
    let mut changed = 0;
    for table in ["cm_posts", "cm_comments"] {
        changed += sqlx::query(sqlx::AssertSqlSafe(format!("UPDATE {table} SET review = ?, mod_state = ? WHERE id = ? AND (review != ? OR mod_state != ?)")))
            .bind(review)
            .bind(mod_state)
            .bind(id)
            .bind(review)
            .bind(mod_state)
            .execute(db.pool())
            .await
            .map_err(storage)?
            .rows_affected();
    }
    Ok(changed > 0)
}

/// An edit came: the new text and the new version (the edit's id),
/// dated `at`; `media_json`: of a file, its envelope with the new caption
/// (none leaves it as it is). Taken only over an older edit (the same
/// date: the larger id), never on a deleted message: any reader with the
/// key of the epoch can seal an old signed edit again, and catch-up brings
/// edits in any order. `true` when the message changed.
pub async fn edit(db: &Db, id: &str, envelope_json: &str, text: Option<&str>, media_json: Option<&str>, version: &str, at: i64) -> Result<bool> {
    let mut changed = 0;
    for table in ["cm_posts", "cm_comments"] {
        changed += sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE {table} SET envelope_json = ?, text = ?, media_json = COALESCE(?, media_json), version = ?, edited_at = ?
             WHERE id = ? AND deleted = 0 AND (edited_at < ? OR (edited_at = ? AND version < ?))"
        )))
        .bind(envelope_json)
        .bind(text)
        .bind(media_json)
        .bind(version)
        .bind(at)
        .bind(id)
        .bind(at)
        .bind(at)
        .bind(version)
        .execute(db.pool())
        .await
        .map_err(storage)?
        .rows_affected();
    }
    Ok(changed > 0)
}

/// Deleted by its author: the row stays (comments hang on it), its text goes.
pub async fn mark_deleted(db: &Db, id: &str) -> Result<()> {
    for table in ["cm_posts", "cm_comments"] {
        sqlx::query(sqlx::AssertSqlSafe(format!("UPDATE {table} SET deleted = 1, text = NULL, media_json = NULL WHERE id = ?")))
            .bind(id)
            .execute(db.pool())
            .await
            .map_err(storage)?;
    }
    Ok(())
}

/// What waits for a moderator in a channel, oldest first.
pub async fn pending(db: &Db, community_id: &str) -> Result<Vec<MessageRow>> {
    let mut rows = sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!("SELECT {POST_COLS} FROM cm_posts WHERE community_id = ? AND review = ?")))
        .bind(community_id)
        .bind(REVIEW_PENDING)
        .fetch_all(db.pool())
        .await
        .map_err(storage)?;
    rows.extend(
        sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!("SELECT {COMMENT_COLS} FROM cm_comments WHERE community_id = ? AND review = ?")))
            .bind(community_id)
            .bind(REVIEW_PENDING)
            .fetch_all(db.pool())
            .await
            .map_err(storage)?,
    );
    rows.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    Ok(rows)
}

/// Messages never held, dated `since` or later: those the rule of
/// pre-moderation may have to hold after all (`moderation::outran_the_rule`).
pub async fn ok_since(db: &Db, community_id: &str, since: i64) -> Result<Vec<MessageRow>> {
    let mut rows = sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!("SELECT {POST_COLS} FROM cm_posts WHERE community_id = ? AND review = ? AND created_at >= ?")))
        .bind(community_id)
        .bind(REVIEW_OK)
        .bind(since)
        .fetch_all(db.pool())
        .await
        .map_err(storage)?;
    rows.extend(
        sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!("SELECT {COMMENT_COLS} FROM cm_comments WHERE community_id = ? AND review = ? AND created_at >= ?")))
            .bind(community_id)
            .bind(REVIEW_OK)
            .bind(since)
            .fetch_all(db.pool())
            .await
            .map_err(storage)?,
    );
    Ok(rows)
}

// ─── Reactions ──────────────────────────────────────────────────────────────

/// Set or take back a reaction; `true` when that changed anything (one
/// heard again does not).
pub async fn set_reaction(db: &Db, community_id: &str, target_id: &str, author: &str, emoji: &str, set: bool, at: i64) -> Result<bool> {
    let res = if set {
        sqlx::query(
            "INSERT INTO cm_reactions (community_id, target_id, author, emoji, at) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(target_id, author, emoji) DO UPDATE SET at = excluded.at WHERE excluded.at > cm_reactions.at",
        )
        .bind(community_id)
        .bind(target_id)
        .bind(author)
        .bind(emoji)
        .bind(at)
        .execute(db.pool())
        .await
        .map_err(storage)?
    } else {
        sqlx::query("DELETE FROM cm_reactions WHERE target_id = ? AND author = ? AND emoji = ?").bind(target_id).bind(author).bind(emoji).execute(db.pool()).await.map_err(storage)?
    };
    Ok(res.rows_affected() > 0)
}

/// The reactions to a message: emoji, how many, and whether `me` (hex) is
/// one of them, so that a page marks mine and a tap on it takes it back.
pub async fn reactions(db: &Db, target_id: &str, me: &str) -> Result<Vec<(String, i64, bool)>> {
    let rows = sqlx::query_as::<_, (String, i64, i64)>(
        "SELECT emoji, COUNT(*), MAX(author = ?) FROM cm_reactions WHERE target_id = ? GROUP BY emoji ORDER BY COUNT(*) DESC, emoji",
    )
    .bind(me)
    .bind(target_id)
    .fetch_all(db.pool())
    .await
    .map_err(storage)?;
    Ok(rows.into_iter().map(|(emoji, n, mine)| (emoji, n, mine != 0)).collect())
}

/// `reactions` of the messages of a page at once, by message (one without
/// any is not named), each in the same order.
pub async fn reactions_of(db: &Db, target_ids: &[&str], me: &str) -> Result<std::collections::HashMap<String, Vec<(String, i64, bool)>>> {
    let mut out: std::collections::HashMap<String, Vec<(String, i64, bool)>> = std::collections::HashMap::new();
    if target_ids.is_empty() {
        return Ok(out);
    }
    let ids = serde_json::to_string(target_ids).unwrap_or_else(|_| "[]".into());
    let rows = sqlx::query_as::<_, (String, String, i64, i64)>(
        "SELECT target_id, emoji, COUNT(*), MAX(author = ?) FROM cm_reactions WHERE target_id IN (SELECT value FROM json_each(?))
         GROUP BY target_id, emoji ORDER BY target_id, COUNT(*) DESC, emoji",
    )
    .bind(me)
    .bind(ids)
    .fetch_all(db.pool())
    .await
    .map_err(storage)?;
    for (target, emoji, n, mine) in rows {
        out.entry(target).or_default().push((emoji, n, mine != 0));
    }
    Ok(out)
}

// ─── Uploads ────────────────────────────────────────────────────────────────

pub const UPLOAD_UPLOADING: &str = "uploading";
pub const UPLOAD_FAILED: &str = "failed";
/// Stopped by the user: its state stays, `media_retry` goes on from there.
pub const UPLOAD_PAUSED: &str = "paused";

/// A file of mine on its way to the hosts.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct UploadRow {
    pub id: String,
    pub community_id: String,
    pub post_id: String,
    pub parent_id: Option<String>,
    pub path: String,
    pub caption: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub done: i64,
    pub total: i64,
    pub created_at: i64,
    /// The scope (section) the post goes into; a comment's is its post's.
    pub scope_id: String,
    /// Files sent by one action (an album): the messenger's `batch` of
    /// the descriptor, the same in each of them; none for a file alone.
    pub batch: Option<String>,
    /// The messenger's `UploadState` as JSON (its key, the chunks the hosts
    /// hold), `{}` before the first chunk; written by `set_upload_state`.
    #[serde(default)]
    pub state_json: String,
    /// What the file carries besides its chunks (`media::UploadMeta`), `{}`
    /// when nothing.
    #[serde(default)]
    pub meta_json: String,
}

const U_COLS: &str = "id, community_id, post_id, parent_id, path, caption, status, error, done, total, created_at, scope_id, batch, state_json, meta_json";

/// Puts the row; on one that is there already only its status, error and
/// counts change (its state is `set_upload_state`'s).
pub async fn put_upload(db: &Db, u: &UploadRow) -> Result<()> {
    sqlx::query(
        "INSERT INTO cm_uploads (id, community_id, post_id, parent_id, path, caption, status, error, done, total, created_at, scope_id, batch, state_json, meta_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET status = excluded.status, error = excluded.error, done = excluded.done, total = excluded.total",
    )
    .bind(&u.id)
    .bind(&u.community_id)
    .bind(&u.post_id)
    .bind(&u.parent_id)
    .bind(&u.path)
    .bind(&u.caption)
    .bind(&u.status)
    .bind(&u.error)
    .bind(u.done)
    .bind(u.total)
    .bind(u.created_at)
    .bind(&u.scope_id)
    .bind(&u.batch)
    .bind(if u.state_json.is_empty() { "{}" } else { u.state_json.as_str() })
    .bind(if u.meta_json.is_empty() { "{}" } else { u.meta_json.as_str() })
    .execute(db.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// The file an upload sends now (a photo made smaller), its size and what
/// it carries.
pub async fn set_upload_file(db: &Db, id: &str, path: &str, total: i64, meta_json: &str) -> Result<()> {
    sqlx::query("UPDATE cm_uploads SET path = ?, total = ?, meta_json = ? WHERE id = ?").bind(path).bind(total).bind(meta_json).bind(id).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}

/// The state of an upload after a chunk, with the bytes the hosts hold.
/// Whether the row is still there (a cancel removes it).
pub async fn set_upload_state(db: &Db, id: &str, done: i64, state_json: &str) -> Result<bool> {
    let res = sqlx::query("UPDATE cm_uploads SET done = ?, state_json = ? WHERE id = ?").bind(done).bind(state_json).bind(id).execute(db.pool()).await.map_err(storage)?;
    Ok(res.rows_affected() > 0)
}

/// The status of an upload and why it stopped, its counts and state as
/// they are.
pub async fn set_upload_status(db: &Db, id: &str, status: &str, error: Option<&str>) -> Result<()> {
    sqlx::query("UPDATE cm_uploads SET status = ?, error = ? WHERE id = ?").bind(status).bind(error).bind(id).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}

pub async fn upload(db: &Db, id: &str) -> Result<Option<UploadRow>> {
    sqlx::query_as::<_, UploadRow>(sqlx::AssertSqlSafe(format!("SELECT {U_COLS} FROM cm_uploads WHERE id = ?"))).bind(id).fetch_optional(db.pool()).await.map_err(storage)
}

/// The files of mine on their way into a channel, oldest first.
pub async fn uploads(db: &Db, community_id: &str) -> Result<Vec<UploadRow>> {
    sqlx::query_as::<_, UploadRow>(sqlx::AssertSqlSafe(format!("SELECT {U_COLS} FROM cm_uploads WHERE community_id = ? ORDER BY created_at, id")))
        .bind(community_id)
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

/// The uploads still marked `uploading`, of every channel, oldest first:
/// at a start, those the end of the last run interrupted.
pub async fn uploading(db: &Db) -> Result<Vec<UploadRow>> {
    sqlx::query_as::<_, UploadRow>(sqlx::AssertSqlSafe(format!("SELECT {U_COLS} FROM cm_uploads WHERE status = ? ORDER BY created_at, id")))
        .bind(UPLOAD_UPLOADING)
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

pub async fn delete_upload(db: &Db, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM cm_uploads WHERE id = ?").bind(id).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}

// ─── Files of this device ───────────────────────────────────────────────────

/// A file of mine is where it was sent from: the post shows it from there.
pub async fn put_local_file(db: &Db, message_id: &str, path: &str) -> Result<()> {
    sqlx::query("INSERT INTO cm_local_files (message_id, path) VALUES (?, ?) ON CONFLICT(message_id) DO UPDATE SET path = excluded.path")
        .bind(message_id)
        .bind(path)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn local_file(db: &Db, message_id: &str) -> Result<Option<String>> {
    sqlx::query_scalar::<_, String>("SELECT path FROM cm_local_files WHERE message_id = ?").bind(message_id).fetch_optional(db.pool()).await.map_err(storage)
}

/// The files some row still names: a file of mine on its way, or one a
/// post of mine shows from where it was sent.
pub async fn named_files(db: &Db) -> Result<Vec<String>> {
    sqlx::query_scalar::<_, String>("SELECT path FROM cm_uploads UNION SELECT path FROM cm_local_files").fetch_all(db.pool()).await.map_err(storage)
}

/// What a file was stored as on the hosts (its descriptor as JSON), by its
/// plaintext hash and size; the latest store is kept (its blobs live
/// longest), its time with it.
pub async fn put_known_file(db: &Db, sha256: &str, size: i64, descriptor_json: &str, stored_at: i64) -> Result<()> {
    sqlx::query(
        "INSERT INTO cm_known_files (sha256, size, descriptor_json, stored_at) VALUES (?, ?, ?, ?)
         ON CONFLICT(sha256, size) DO UPDATE SET descriptor_json = excluded.descriptor_json, stored_at = excluded.stored_at WHERE excluded.stored_at > cm_known_files.stored_at",
    )
    .bind(sha256)
    .bind(size)
    .bind(descriptor_json)
    .bind(stored_at)
    .execute(db.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// The files known of `size` stored since `since`: (sha256, descriptor JSON).
pub async fn known_files(db: &Db, size: i64, since: i64) -> Result<Vec<(String, String)>> {
    sqlx::query_as::<_, (String, String)>("SELECT sha256, descriptor_json FROM cm_known_files WHERE size = ? AND stored_at >= ? ORDER BY stored_at DESC")
        .bind(size)
        .bind(since)
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

// ─── Read ───────────────────────────────────────────────────────────────────

/// Read up to `at`; never back.
pub async fn set_read(db: &Db, community_id: &str, at: i64) -> Result<()> {
    sqlx::query("INSERT INTO cm_read (community_id, read_at) VALUES (?, ?) ON CONFLICT(community_id) DO UPDATE SET read_at = MAX(cm_read.read_at, excluded.read_at)")
        .bind(community_id)
        .bind(at)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn read_at(db: &Db, community_id: &str) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>("SELECT read_at FROM cm_read WHERE community_id = ?").bind(community_id).fetch_optional(db.pool()).await.map_err(storage)?.unwrap_or(0))
}

/// Posts after what was read, by others, as released (a post held for a
/// moderator or rejected is no news: the feed shows it to no reader, who
/// could not read the badge away; `me`: my own from another
/// device are read), in a channel I am in (`joined`: not one left,
/// removed, disbanded or only looked at). One rule for the list and the
/// tray, so that their counts agree.
pub async fn unread(db: &Db, community_id: &str, me: &str) -> Result<i64> {
    let read = read_at(db, community_id).await?;
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM cm_posts WHERE community_id = ? AND created_at > ? AND deleted = 0 AND mod_state = '' AND review IN ('ok', 'approved', 'released') AND author != ?
           AND EXISTS (SELECT 1 FROM cm_channels c WHERE c.id = cm_posts.community_id AND c.membership = 'joined')",
    )
    .bind(community_id)
    .bind(read)
    .bind(me)
    .fetch_one(db.pool())
    .await
    .map_err(storage)
}

/// What `unread` counts, of every channel at once: one query for the count
/// on the tray and for the list of chats, which asked once per channel.
/// The same rule as `unread`: a change of one is a change of both.
pub async fn unread_all(db: &Db, me: &str) -> Result<std::collections::HashMap<String, i64>> {
    let rows = sqlx::query_as::<_, (String, i64)>(
        "SELECT p.community_id, COUNT(*) FROM cm_posts p LEFT JOIN cm_read r ON r.community_id = p.community_id \
         WHERE p.created_at > COALESCE(r.read_at, 0) AND p.deleted = 0 AND p.mod_state = '' AND p.review IN ('ok', 'approved', 'released') AND p.author != ? \
           AND EXISTS (SELECT 1 FROM cm_channels c WHERE c.id = p.community_id AND c.membership = 'joined') \
         GROUP BY p.community_id",
    )
    .bind(me)
    .fetch_all(db.pool())
    .await
    .map_err(storage)?;
    Ok(rows.into_iter().collect())
}

/// What `unread` counts, by the section each post is in (2026-10-23, the
/// tiles of the sections): the same rule, the read mark of the channel.
pub async fn unread_by_scope(db: &Db, community_id: &str, me: &str) -> Result<std::collections::HashMap<String, i64>> {
    let read = read_at(db, community_id).await?;
    let rows = sqlx::query_as::<_, (String, i64)>(
        "SELECT scope_id, COUNT(*) FROM cm_posts WHERE community_id = ? AND created_at > ? AND deleted = 0 AND mod_state = '' AND review IN ('ok', 'approved', 'released') AND author != ?
           AND EXISTS (SELECT 1 FROM cm_channels c WHERE c.id = cm_posts.community_id AND c.membership = 'joined')
         GROUP BY scope_id",
    )
    .bind(community_id)
    .bind(read)
    .bind(me)
    .fetch_all(db.pool())
    .await
    .map_err(storage)?;
    Ok(rows.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marks::REVIEW_APPROVED;

    /// The newest post of every channel at once is what `feed` gives for each.
    #[tokio::test]
    async fn the_last_posts_of_all_channels_at_once_are_those_of_each() {
        let db = Db::open_in_memory().await.unwrap();
        for (id, channel, at, review) in [("p1", "c1", 10, REVIEW_OK), ("p2", "c1", 20, REVIEW_OK), ("p3", "c2", 30, REVIEW_OK), ("p4", "c3", 5, REVIEW_OK), ("p5", "c2", 40, "held")] {
            insert(&db, &MessageRow { community_id: channel.into(), ..message(id, "", at, review) }).await.unwrap();
        }
        let seeing_all = vec!["c3".to_string(), "c2".to_string()];
        let all = last_posts(&db, "x", &seeing_all).await.unwrap();
        assert_eq!(all.len(), 3);
        for channel in ["c1", "c2", "c3"] {
            let viewer = if seeing_all.iter().any(|c| c == channel) { None } else { Some("x") };
            let one = feed(&db, channel, None, 1, viewer).await.unwrap();
            assert_eq!(all.get(channel).map(|m| m.id.clone()), one.first().map(|m| m.id.clone()), "{channel}");
        }
        assert_eq!(all["c2"].id, "p5", "a moderator sees the held post");
        let reader = last_posts(&db, "x", &[]).await.unwrap();
        assert_eq!(reader["c2"].id, "p3", "a reader does not see a held post");
    }

    /// A deleted post is not the last of its channel (the row would say
    /// «gone» and keep its place): the one before it is, and a comment,
    /// however new, never is. `last_post_in` of every scope is the same.
    #[tokio::test]
    async fn the_last_post_is_neither_deleted_nor_a_comment() {
        let db = Db::open_in_memory().await.unwrap();
        for (id, post, at) in [("p1", "", 10), ("p2", "", 20), ("k1", "p1", 30)] {
            insert(&db, &message(id, post, at, REVIEW_OK)).await.unwrap();
        }
        assert_eq!(last_posts(&db, "x", &[]).await.unwrap()["c1"].id, "p2");
        mark_deleted(&db, "p2").await.unwrap();
        assert_eq!(last_posts(&db, "x", &[]).await.unwrap()["c1"].id, "p1", "the one before the deleted");
        let every = vec!["everyone".to_string()];
        assert_eq!(last_post_in(&db, "c1", Some("x"), &every).await.unwrap().map(|m| m.id).as_deref(), Some("p1"));
        assert_eq!(last_post_in(&db, "c1", None, &["gold".to_string()]).await.unwrap(), None, "none of that scope");
        mark_deleted(&db, "p1").await.unwrap();
        assert!(last_posts(&db, "x", &[]).await.unwrap().is_empty(), "no post left: none");
    }

    /// The count of every channel at once is that of each (ревью модуля п. 11).
    #[tokio::test]
    async fn the_unread_of_all_channels_at_once_is_that_of_each() {
        let db = Db::open_in_memory().await.unwrap();
        for (id, channel, at) in [("p1", "c1", 10), ("p2", "c1", 20), ("p3", "c2", 30), ("p4", "c3", 5)] {
            insert(&db, &MessageRow { community_id: channel.into(), ..message(id, "", at, REVIEW_OK) }).await.unwrap();
        }
        mark_deleted(&db, "p3").await.unwrap();
        set_read(&db, "c1", 15).await.unwrap();
        set_read(&db, "c3", 5).await.unwrap();
        for c in ["c1", "c2", "c3"] {
            sqlx::query("INSERT INTO cm_channels (id, name, owner, join_mode, membership, created_at, updated_at) VALUES (?, 'n', 'o', 'open', 'joined', 0, 0)").bind(c).execute(db.pool()).await.unwrap();
        }
        let all = unread_all(&db, "x").await.unwrap();
        for channel in ["c1", "c2", "c3"] {
            assert_eq!(all.get(channel).copied().unwrap_or(0), unread(&db, channel, "x").await.unwrap(), "{channel}");
        }
        assert_eq!(all.get("c1"), Some(&1));
        assert!(!all.contains_key("c2") && !all.contains_key("c3"), "none: not there");
    }

    /// What the feed does not show a reader is not unread (a post held for
    /// a moderator, one rejected): the badge counted them, the feed could
    /// never be read down to them, and the badge stuck. Comments are no
    /// posts and never count.
    #[tokio::test]
    async fn what_a_reader_cannot_see_is_not_unread_and_a_comment_is_not_a_post() {
        use crate::marks::{REVIEW_PENDING, REVIEW_REJECTED};
        let db = Db::open_in_memory().await.unwrap();
        sqlx::query("INSERT INTO cm_channels (id, name, owner, join_mode, membership, created_at, updated_at) VALUES ('c1', 'n', 'o', 'open', 'joined', 0, 0)").execute(db.pool()).await.unwrap();
        for (id, at, review) in [("p1", 10, REVIEW_OK), ("p4", 20, REVIEW_APPROVED), ("p2", 30, REVIEW_PENDING), ("p3", 40, REVIEW_REJECTED)] {
            insert(&db, &message(id, "", at, review)).await.unwrap();
        }
        insert(&db, &message("k1", "p1", 50, REVIEW_OK)).await.unwrap();
        // The reader's feed: p4 and p1; read down to the newest of it.
        let shown = feed(&db, "c1", None, 10, Some("x")).await.unwrap();
        assert_eq!(shown.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["p4", "p1"]);
        assert_eq!(unread(&db, "c1", "x").await.unwrap(), 2, "the two the feed shows");
        assert_eq!(unread_all(&db, "x").await.unwrap().get("c1"), Some(&2));
        set_read(&db, "c1", shown[0].created_at).await.unwrap();
        assert_eq!(unread(&db, "c1", "x").await.unwrap(), 0, "read down to the newest shown: nothing stays");
        assert!(!unread_all(&db, "x").await.unwrap().contains_key("c1"));
    }

    /// The unread of each section (the tiles, 2026-10-23): the rule of the
    /// channel's count, split by the scope a post is in.
    #[tokio::test]
    async fn the_unread_of_each_section_adds_up_to_that_of_the_channel() {
        let db = Db::open_in_memory().await.unwrap();
        sqlx::query("INSERT INTO cm_channels (id, name, owner, join_mode, membership, created_at, updated_at) VALUES ('c1', 'n', 'o', 'open', 'joined', 0, 0)").execute(db.pool()).await.unwrap();
        let gold = "ab".repeat(16);
        insert(&db, &message("p1", "", 10, REVIEW_OK)).await.unwrap();
        insert(&db, &MessageRow { scope_id: gold.clone(), ..message("p2", "", 20, REVIEW_OK) }).await.unwrap();
        insert(&db, &MessageRow { scope_id: gold.clone(), ..message("p3", "", 30, REVIEW_OK) }).await.unwrap();
        insert(&db, &MessageRow { scope_id: gold.clone(), ..message("p4", "", 40, "held") }).await.unwrap();
        insert(&db, &MessageRow { scope_id: gold.clone(), ..message("k1", "p2", 50, REVIEW_OK) }).await.unwrap();
        let by = unread_by_scope(&db, "c1", "x").await.unwrap();
        assert_eq!((by.get("everyone").copied(), by.get(&gold).copied()), (Some(1), Some(2)));
        assert_eq!(by.values().sum::<i64>(), unread(&db, "c1", "x").await.unwrap());
        set_read(&db, "c1", 20).await.unwrap();
        let by = unread_by_scope(&db, "c1", "x").await.unwrap();
        assert_eq!((by.get("everyone").copied(), by.get(&gold).copied()), (None, Some(1)));
    }

    fn message(id: &str, post: &str, at: i64, review: &str) -> MessageRow {
        MessageRow {
            id: id.into(),
            community_id: "c1".into(),
            post_id: post.into(),
            parent_id: None,
            author: "a".into(),
            created_at: at,
            epoch: 1,
            envelope_json: "{}".into(),
            text: Some("t".into()),
            media_json: None,
            version: id.into(),
            review: review.into(),
            mod_state: MOD_SHOWN.into(),
            deleted: false,
            received_at: 0,
            scope_id: "everyone".into(),
            outer_id: None,
            edited_at: 0,
        }
    }

    /// A page of the sections a reader belongs to (wire §14.3).
    #[tokio::test]
    async fn a_page_shows_the_scopes_asked_for() {
        let db = Db::open_in_memory().await.unwrap();
        let gold = "ab".repeat(16);
        insert(&db, &message("p1", "", 10, REVIEW_OK)).await.unwrap();
        insert(&db, &MessageRow { scope_id: gold.clone(), ..message("p2", "", 11, REVIEW_OK) }).await.unwrap();
        insert(&db, &MessageRow { scope_id: gold.clone(), ..message("c1", "p2", 12, REVIEW_OK) }).await.unwrap();
        let ids = |rows: Vec<MessageRow>| rows.into_iter().map(|r| r.id).collect::<Vec<_>>();
        assert_eq!(ids(feed_in(&db, "c1", None, 10, None, None).await.unwrap()), ["p2", "p1"]);
        assert_eq!(ids(feed_in(&db, "c1", None, 10, None, Some(&["everyone".to_string()])).await.unwrap()), ["p1"]);
        assert_eq!(ids(feed_in(&db, "c1", None, 10, None, Some(std::slice::from_ref(&gold))).await.unwrap()), ["p2"]);
        assert!(feed_in(&db, "c1", None, 10, None, Some(&[])).await.unwrap().is_empty(), "no scope, nothing");
        assert_eq!(get(&db, "c1").await.unwrap().map(|r| r.scope_id), Some(gold.clone()));
        assert!(thread_in(&db, "c1", "p2", None, 10, None, Some(&["everyone".to_string()])).await.unwrap().is_empty());
        assert_eq!(ids(thread_in(&db, "c1", "p2", None, 10, None, Some(&[gold])).await.unwrap()), ["c1"]);
    }

    #[tokio::test]
    async fn posts_comments_reactions_and_what_was_read() {
        let db = Db::open_in_memory().await.unwrap();
        assert!(insert(&db, &message("p1", "", 10, REVIEW_OK)).await.unwrap());
        assert!(!insert(&db, &message("p1", "", 10, REVIEW_OK)).await.unwrap());
        insert(&db, &message("p2", "", 20, REVIEW_PENDING)).await.unwrap();
        insert(&db, &message("c1", "p1", 15, REVIEW_OK)).await.unwrap();
        insert(&db, &MessageRow { parent_id: Some("c1".into()), ..message("c2", "p1", 16, REVIEW_PENDING) }).await.unwrap();
        assert_eq!(posts(&db, "c1", None, 10).await.unwrap().iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec!["p2", "p1"]);
        assert_eq!(posts(&db, "c1", Some(20), 10).await.unwrap().len(), 1);
        assert_eq!(comments(&db, "c1", "p1").await.unwrap().len(), 2);
        assert!(comments(&db, "c9", "p1").await.unwrap().is_empty(), "of its own channel only");
        assert_eq!(count_comments(&db, "c9", "p1").await.unwrap(), 0);
        // The counts of a page at once are those of each post.
        let counts = count_comments_of(&db, "c1", &["p1", "p2"]).await.unwrap();
        assert_eq!((counts.get("p1").copied(), counts.get("p2")), (Some(count_comments(&db, "c1", "p1").await.unwrap()), None));
        assert!(count_comments_of(&db, "c9", &["p1"]).await.unwrap().is_empty(), "of its own channel only");
        assert!(thread(&db, "c9", "p1", None, 10, None).await.unwrap().is_empty());
        assert!(get(&db, "c2").await.unwrap().unwrap().parent_id.is_some());
        assert!(get(&db, "p1").await.unwrap().unwrap().is_post());
        assert_eq!(pending(&db, "c1").await.unwrap().iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec!["c2", "p2"]);
        assert_eq!(ok_since(&db, "c1", 12).await.unwrap().len(), 1);

        assert!(set_resolution(&db, "p2", REVIEW_APPROVED, MOD_SHOWN).await.unwrap());
        assert!(!set_resolution(&db, "p2", REVIEW_APPROVED, MOD_SHOWN).await.unwrap(), "nothing changed");
        assert!(set_resolution(&db, "c2", REVIEW_PENDING, MOD_HIDDEN).await.unwrap());
        assert!(edit(&db, "p1", "{\"t\":\"edit\"}", Some("new"), None, "e1", 20).await.unwrap());
        assert!(!edit(&db, "p1", "{\"t\":\"edit\"}", Some("old"), None, "e0", 19).await.unwrap(), "an older edit");
        let edited = get(&db, "p1").await.unwrap().unwrap();
        assert_eq!((edited.version.as_str(), edited.edited_at), ("e1", 20));
        mark_deleted(&db, "c1").await.unwrap();
        let gone = get(&db, "c1").await.unwrap().unwrap();
        assert!(gone.deleted && gone.text.is_none());

        set_reaction(&db, "c1", "p1", "a", "👍", true, 1).await.unwrap();
        set_reaction(&db, "c1", "p1", "b", "👍", true, 2).await.unwrap();
        set_reaction(&db, "c1", "p1", "b", "🔥", true, 2).await.unwrap();
        set_reaction(&db, "c1", "p1", "b", "🔥", false, 3).await.unwrap();
        assert_eq!(reactions(&db, "p1", "b").await.unwrap(), vec![("👍".to_string(), 2, true)]);
        assert_eq!(reactions(&db, "p1", "c").await.unwrap(), vec![("👍".to_string(), 2, false)]);
        // Of a page at once: each message as alone, one without any not named.
        set_reaction(&db, "c1", "p2", "a", "🔥", true, 4).await.unwrap();
        set_reaction(&db, "c1", "p2", "c", "🔥", true, 4).await.unwrap();
        set_reaction(&db, "c1", "p2", "c", "👍", true, 4).await.unwrap();
        let of = reactions_of(&db, &["p1", "p2", "c1"], "c").await.unwrap();
        assert_eq!(of.get("p1"), Some(&reactions(&db, "p1", "c").await.unwrap()));
        assert_eq!(of.get("p2"), Some(&reactions(&db, "p2", "c").await.unwrap()));
        assert_eq!(of["p2"], vec![("🔥".to_string(), 2, true), ("👍".to_string(), 1, true)]);
        assert!(!of.contains_key("c1"));

        assert_eq!(unread(&db, "c1", "x").await.unwrap(), 0, "a channel not joined here");
        sqlx::query("INSERT INTO cm_channels (id, name, owner, join_mode, membership, created_at, updated_at) VALUES ('c1', 'n', 'o', 'open', 'joined', 0, 0)").execute(db.pool()).await.unwrap();
        assert_eq!(unread(&db, "c1", "x").await.unwrap(), 2);
        assert_eq!(unread(&db, "c1", "a").await.unwrap(), 0, "my own from another device");
        set_read(&db, "c1", 15).await.unwrap();
        set_read(&db, "c1", 5).await.unwrap();
        assert_eq!(read_at(&db, "c1").await.unwrap(), 15, "never back");
        assert_eq!(unread(&db, "c1", "x").await.unwrap(), 1);
        sqlx::query("UPDATE cm_channels SET membership = 'left'").execute(db.pool()).await.unwrap();
        assert_eq!(unread(&db, "c1", "x").await.unwrap(), 0, "left");
    }
}
