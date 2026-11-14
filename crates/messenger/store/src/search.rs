// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The search of the conversation (049): the words of every message I hold
//! in an FTS5 index, found by the start of a word in any script and case,
//! with or without their marks.
//!
//! SQLite folds the case of Latin letters only and its tokenizer strips the
//! marks of Latin letters only, so the words are folded here ([`fold`], the
//! rule of `fold()` in the UI's `shared/search.ts`) before they are put in,
//! and a query is folded alike. What a message holds is put in wherever its
//! row changes ([`index_in`], [`forget_in`], [`forget_chat_in`]); whether
//! it is shown is asked when searching, by the rules the chat lists it by
//! (`messages::SHOWN`, `SHOWN_TO_MEMBER`), since moderation, a block or a
//! removal for me come after the message did. The rows that came before the
//! index are put in by [`backfill_batch`], in the background.

use sqlx::SqliteConnection;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

use crate::messages::{MessageRow, CT_MEDIA, CT_TEXT, COLS, SHOWN, SHOWN_TO_MEMBER};
use crate::{storage, Store};
use messenger_core::Result;

/// Up to where the rows that came before the index are in it:
/// `after:<msg_messages.id>` (the last row put in, by id), `done` when all
/// are. Not a setting of the account: a wipe leaves an empty store, whole
/// (`account::wipe` marks it so).
pub const KEY_BACKFILLED_TO: &str = "search.backfilled_to";
/// The rule of [`fold`] the index was written by. Another rule ahead of
/// it: the index is emptied and written again by the backfill ([`prepare`]).
pub const KEY_FOLD_VERSION: &str = "search.fold_version";
pub const FOLD_VERSION: &str = "1";
/// Rows put in by one transaction of the backfill.
pub const BACKFILL_BATCH: i64 = 2000;
pub(crate) const BACKFILL_WHOLE: &str = "done";
const BACKFILL_AFTER: &str = "after:";

/// Lower case without marks: «José» is «jose», «Ёлка» is «елка», «й» is
/// «и». NFD, every `General_Category=Mark` dropped, lower case: what
/// `fold()` of the UI does (`s.normalize('NFD').replace(/\p{M}/gu,
/// '').toLowerCase()`), so the two sides agree on what matches; the same
/// examples hold in the tests of both.
pub fn fold(s: &str) -> String {
    s.nfd().filter(|c| !is_combining_mark(*c)).collect::<String>().to_lowercase()
}

/// What of a message is searched, folded: its text (a Markdown one without
/// its marks), a caption, the name of a file. `None` for a row with no
/// words of a person: an edit, a deletion, a note, a line of the app, a
/// card, a voice message or a round video (their names are made up by the
/// app), and a message deleted.
pub fn body_of(content_type: &str, text: Option<&str>, plain: Option<&str>, media_json: Option<&str>, deleted: bool) -> Option<String> {
    if deleted || (content_type != CT_TEXT && content_type != CT_MEDIA) {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    if let Some(t) = plain.or(text).filter(|t| !t.trim().is_empty()) {
        parts.push(t.to_string());
    }
    if content_type == CT_MEDIA {
        let v: serde_json::Value = media_json.and_then(|j| serde_json::from_str(j).ok()).unwrap_or_default();
        let made_up = matches!(v.get("kind").and_then(|k| k.as_str()), Some("voice" | "circle"));
        if let Some(name) = v.get("name").and_then(|n| n.as_str()).filter(|n| !made_up && !n.trim().is_empty()) {
            parts.push(name.to_string());
        }
    }
    (!parts.is_empty()).then(|| fold(&parts.join("\n")))
}

/// The FTS5 query of what a person typed: every word folded, cut where the
/// tokenizer cuts the words of a message (at every character that is no
/// letter or digit: «прив-мир», «a,b», «отчёт.pdf» are two words each),
/// each quoted, so that none is an operator (`OR`, `NEAR`), and taken as
/// the start of a word, all of them required, in any order. `None` when no
/// word is left.
pub fn match_query(query: &str) -> Option<String> {
    let words: Vec<String> = fold(query)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\"*"))
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

/// The row of `msg_messages` as the index reads it.
#[derive(sqlx::FromRow)]
struct Source {
    id: String,
    content_type: String,
    text: Option<String>,
    plain: Option<String>,
    media_json: Option<String>,
    deleted_at: Option<i64>,
    search_rowid: Option<i64>,
}

const SOURCE_COLS: &str = "id, content_type, text, plain, media_json, deleted_at, search_rowid";

/// Put the words of message `msg_id` in the index as its row holds them
/// now, in the transaction that changed it; a message with no words to
/// search is taken out.
pub(crate) async fn index_in(conn: &mut SqliteConnection, msg_id: &str) -> Result<()> {
    let row = sqlx::query_as::<_, Source>(sqlx::AssertSqlSafe(format!("SELECT {SOURCE_COLS} FROM msg_messages WHERE id = ?")))
        .bind(msg_id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(storage)?;
    match row {
        Some(r) => put_in(conn, &r).await,
        None => Ok(()),
    }
}

async fn put_in(conn: &mut SqliteConnection, r: &Source) -> Result<()> {
    let body = body_of(&r.content_type, r.text.as_deref(), r.plain.as_deref(), r.media_json.as_deref(), r.deleted_at.is_some());
    let Some(body) = body else { return forget_in(conn, &r.id).await };
    let rowid = match r.search_rowid {
        Some(rowid) => {
            sqlx::query("DELETE FROM msg_search WHERE rowid = ?").bind(rowid).execute(&mut *conn).await.map_err(storage)?;
            rowid
        }
        None => {
            // Past every rowid the index holds, the words a row gone
            // without them left behind included.
            let last: Option<i64> = sqlx::query_scalar("SELECT rowid FROM msg_search ORDER BY rowid DESC LIMIT 1")
                .fetch_optional(&mut *conn)
                .await
                .map_err(storage)?;
            let rowid = last.unwrap_or(0) + 1;
            sqlx::query("UPDATE msg_messages SET search_rowid = ? WHERE id = ?")
                .bind(rowid)
                .bind(&r.id)
                .execute(&mut *conn)
                .await
                .map_err(storage)?;
            rowid
        }
    };
    sqlx::query("INSERT INTO msg_search (rowid, body) VALUES (?, ?)")
        .bind(rowid)
        .bind(body)
        .execute(&mut *conn)
        .await
        .map_err(storage)?;
    Ok(())
}

/// Take message `msg_id` out of the index: before its row goes, or as its
/// words do.
pub(crate) async fn forget_in(conn: &mut SqliteConnection, msg_id: &str) -> Result<()> {
    let rowid: Option<i64> = sqlx::query_scalar("SELECT search_rowid FROM msg_messages WHERE id = ?")
        .bind(msg_id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(storage)?
        .flatten();
    if let Some(rowid) = rowid {
        sqlx::query("DELETE FROM msg_search WHERE rowid = ?").bind(rowid).execute(&mut *conn).await.map_err(storage)?;
        sqlx::query("UPDATE msg_messages SET search_rowid = NULL WHERE id = ?").bind(msg_id).execute(&mut *conn).await.map_err(storage)?;
    }
    Ok(())
}

/// The statement that takes out of the index the messages that `rows`, a
/// condition on `msg_messages`, names: run before they go, with the binds
/// of `rows`.
pub(crate) fn forget_sql(rows: &str) -> String {
    format!("DELETE FROM msg_search WHERE rowid IN (SELECT search_rowid FROM msg_messages WHERE ({rows}) AND search_rowid IS NOT NULL)")
}

/// The statement that takes out of the index every message of the chats
/// `chat_id {CHAT}` names (`servers::OF_A_CHAT`), before they go.
pub(crate) const FORGET_OF_A_CHAT: &str =
    "DELETE FROM msg_search WHERE rowid IN (SELECT search_rowid FROM msg_messages WHERE chat_id {CHAT} AND search_rowid IS NOT NULL)";

/// Take every message of a chat out of the index, in the transaction that
/// deletes them, before it does.
pub(crate) async fn forget_chat_in(conn: &mut SqliteConnection, chat_id: &str) -> Result<()> {
    sqlx::query(sqlx::AssertSqlSafe(FORGET_OF_A_CHAT.replace("{CHAT}", "= ?")))
        .bind(chat_id)
        .execute(&mut *conn)
        .await
        .map_err(storage)?;
    Ok(())
}

// ─── Searching ──────────────────────────────────────────────────────────────

/// `COLS` of `msg_messages` as the alias `m`.
fn m_cols() -> String {
    COLS.split(", ").map(|c| format!("m.{c}")).collect::<Vec<_>>().join(", ")
}

/// The rows the index found that the chat shows: `SHOWN` (to a moderator
/// of its group) or `SHOWN_TO_MEMBER`, not deleted, not removed for me.
fn visible(moderator: bool) -> String {
    let shown = if moderator { SHOWN } else { SHOWN_TO_MEMBER };
    format!("{shown} AND m.deleted_at IS NULL AND NOT EXISTS (SELECT 1 FROM msg_own_hidden h WHERE h.message_id = m.id)")
}

/// The rows of `msg_messages m` whose words match the query bound here.
/// The full-text query runs once, as a subquery of its own: joined, SQLite
/// may walk the rows of a chat and run it again for each of them.
const HITS: &str = "m.search_rowid IN (SELECT rowid FROM msg_search WHERE msg_search MATCH ?)";

/// Where a page of the search in a chat ends: the newest first by
/// (`created_at`, `id`), the next page strictly past this pair, so that
/// messages of the same second are neither lost nor read twice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub created_at: i64,
    pub id: String,
}

/// The messages of a chat that hold every word of `query` (each as the
/// start of a word), as the chat shows them, the newest first, at most
/// `limit`, all past `before` when given; and how many there are in all.
/// Both read in one snapshot. Nothing for a query with no word.
pub async fn in_chat(store: &Store, chat_id: &str, query: &str, before: Option<&Cursor>, limit: i64, moderator: bool) -> Result<(Vec<MessageRow>, i64)> {
    let Some(q) = match_query(query) else { return Ok((Vec::new(), 0)) };
    let visible = visible(moderator);
    let mut tx = store.pool().begin().await.map_err(storage)?;
    let rows = sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {} FROM msg_messages m WHERE {HITS} AND m.chat_id = ? AND {visible}
           AND (m.created_at < ? OR (m.created_at = ? AND m.id < ?))
         ORDER BY m.created_at DESC, m.id DESC LIMIT ?",
        m_cols()
    )))
    .bind(&q)
    .bind(chat_id)
    .bind(before.map_or(i64::MAX, |b| b.created_at))
    .bind(before.map_or(i64::MAX, |b| b.created_at))
    .bind(before.map_or("", |b| b.id.as_str()))
    .bind(limit)
    .fetch_all(&mut *tx)
    .await
    .map_err(storage)?;
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT COUNT(*) FROM msg_messages m WHERE {HITS} AND m.chat_id = ? AND {visible}"
    )))
    .bind(&q)
    .bind(chat_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(storage)?;
    tx.commit().await.map_err(storage)?;
    Ok((rows, total))
}

/// The chats of groups whose messages hold the words of `query`, shown or
/// not: whom the caller asks whether I moderate them before
/// [`across_chats`].
pub async fn groups_matching(store: &Store, query: &str) -> Result<Vec<String>> {
    let Some(q) = match_query(query) else { return Ok(Vec::new()) };
    sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(format!(
        "SELECT DISTINCT m.chat_id FROM msg_messages m WHERE {HITS} AND m.chat_id LIKE 'group:%' ORDER BY m.chat_id"
    )))
    .bind(&q)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// A chat the search found: how many of its messages match and the
/// newest of them.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct ChatHitRow {
    pub chat_id: String,
    #[sqlx(rename = "n")]
    pub count: i64,
    #[sqlx(flatten)]
    pub last: MessageRow,
}

/// The chats whose shown messages hold every word of `query`, the one with
/// the newest match first, at most `limit`. `moderated`: the chats of the
/// groups I moderate, where what waits for a moderator is found too. The
/// newest match of a chat is read by the statement that asks whether it
/// is shown: hidden or edited since, it is not the one given.
pub async fn across_chats(store: &Store, query: &str, limit: i64, moderated: &[String]) -> Result<Vec<ChatHitRow>> {
    let Some(q) = match_query(query) else { return Ok(Vec::new()) };
    let marks = vec!["?"; moderated.len()].join(", ");
    let sql = format!(
        "SELECT * FROM (
           SELECT {cols}, COUNT(*) OVER (PARTITION BY m.chat_id) AS n,
                  ROW_NUMBER() OVER (PARTITION BY m.chat_id ORDER BY m.created_at DESC, m.id DESC) AS nth
           FROM msg_messages m
           WHERE {HITS} AND EXISTS (SELECT 1 FROM msg_chats c WHERE c.id = m.chat_id)
             AND (({member}) OR (m.chat_id IN ({marks}) AND {moderator})))
         WHERE nth = 1 ORDER BY created_at DESC, chat_id LIMIT ?",
        cols = m_cols(),
        member = visible(false),
        moderator = visible(true),
    );
    let mut query = sqlx::query_as::<_, ChatHitRow>(sqlx::AssertSqlSafe(sql)).bind(&q);
    for chat in moderated {
        query = query.bind(chat);
    }
    query.bind(limit).fetch_all(store.pool()).await.map_err(storage)
}

// ─── The rows from before the index ─────────────────────────────────────────

/// Index written by another rule of [`fold`] (or by none: the store from
/// before 049): emptied, to be written again by the backfill from the
/// first row. Done at the start of every backfill.
pub async fn prepare(store: &Store) -> Result<()> {
    let mut tx = store.pool().begin_with("BEGIN IMMEDIATE").await.map_err(storage)?;
    if setting_in(&mut tx, KEY_FOLD_VERSION).await?.as_deref() != Some(FOLD_VERSION) {
        sqlx::query("DELETE FROM msg_search").execute(&mut *tx).await.map_err(storage)?;
        sqlx::query("UPDATE msg_messages SET search_rowid = NULL WHERE search_rowid IS NOT NULL")
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        sqlx::query("DELETE FROM msg_settings WHERE key = ?").bind(KEY_BACKFILLED_TO).execute(&mut *tx).await.map_err(storage)?;
        set_in(&mut tx, KEY_FOLD_VERSION, FOLD_VERSION).await?;
    }
    tx.commit().await.map_err(storage)
}

/// Whether every message is in the index: else what a search answers may
/// miss the older ones (`partial`).
pub async fn is_whole(store: &Store) -> Result<bool> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT key, value FROM msg_settings WHERE key IN (?, ?)")
        .bind(KEY_FOLD_VERSION)
        .bind(KEY_BACKFILLED_TO)
        .fetch_all(store.pool())
        .await
        .map_err(storage)?;
    let get = |k: &str| rows.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    Ok(get(KEY_FOLD_VERSION) == Some(FOLD_VERSION) && get(KEY_BACKFILLED_TO) == Some(BACKFILL_WHOLE))
}

/// What one batch of the backfill came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// This many rows were read; more are to come.
    More(u64),
    /// Every row is in the index: this batch read the last ones (this
    /// many), or none were left.
    Whole(u64),
    /// The store is no longer of the owner the backfill started for (a
    /// wipe, another key) or the index was reset: stop, the next start
    /// goes on.
    Stopped,
}

/// One batch of the backfill: the next [`BACKFILL_BATCH`] rows by id after
/// `search.backfilled_to`, put in and the cursor moved, in one
/// transaction that holds the store for writing from its first statement,
/// so that no change of a row and no wipe falls between the read and the
/// write. `owner` is `account.owner` as it was when the backfill started:
/// a store of another owner now is left alone. The last batch also takes
/// out what a deletion left in the index (words of a row gone without
/// them).
pub async fn backfill_batch(store: &Store, owner: Option<&str>) -> Result<Step> {
    let mut tx = store.pool().begin_with("BEGIN IMMEDIATE").await.map_err(storage)?;
    let now_owner = setting_in(&mut tx, crate::account::KEY_OWNER).await?.filter(|o| !o.is_empty());
    if now_owner.as_deref() != owner || setting_in(&mut tx, KEY_FOLD_VERSION).await?.as_deref() != Some(FOLD_VERSION) {
        return Ok(Step::Stopped);
    }
    let cursor = setting_in(&mut tx, KEY_BACKFILLED_TO).await?;
    if cursor.as_deref() == Some(BACKFILL_WHOLE) {
        return Ok(Step::Whole(0));
    }
    let after = cursor.as_deref().and_then(|c| c.strip_prefix(BACKFILL_AFTER)).unwrap_or("").to_string();
    let rows = sqlx::query_as::<_, Source>(sqlx::AssertSqlSafe(format!(
        "SELECT {SOURCE_COLS} FROM msg_messages WHERE id > ? ORDER BY id LIMIT ?"
    )))
    .bind(&after)
    .bind(BACKFILL_BATCH)
    .fetch_all(&mut *tx)
    .await
    .map_err(storage)?;
    for r in &rows {
        put_in(&mut tx, r).await?;
    }
    let step = if (rows.len() as i64) < BACKFILL_BATCH {
        sqlx::query("DELETE FROM msg_search WHERE rowid NOT IN (SELECT search_rowid FROM msg_messages WHERE search_rowid IS NOT NULL)")
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        set_in(&mut tx, KEY_BACKFILLED_TO, BACKFILL_WHOLE).await?;
        Step::Whole(rows.len() as u64)
    } else {
        let last = rows.last().map(|r| r.id.as_str()).unwrap_or_default();
        set_in(&mut tx, KEY_BACKFILLED_TO, &format!("{BACKFILL_AFTER}{last}")).await?;
        Step::More(rows.len() as u64)
    };
    tx.commit().await.map_err(storage)?;
    Ok(step)
}

async fn setting_in(conn: &mut SqliteConnection, key: &str) -> Result<Option<String>> {
    sqlx::query_scalar("SELECT value FROM msg_settings WHERE key = ?")
        .bind(key)
        .fetch_optional(&mut *conn)
        .await
        .map_err(storage)
}

pub(crate) async fn set_in(conn: &mut SqliteConnection, key: &str, value: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO msg_settings (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(&mut *conn)
    .await
    .map_err(storage)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{self, NewMessage, DIR_IN, DIR_OUT, MOD_HIDDEN, REVIEW_PENDING, STATUS_RECEIVED};
    use messenger_core::envelope::TextFormat;

    fn said(id: &str, chat: &str, text: &str, at: i64) -> NewMessage {
        NewMessage {
            id: id.into(),
            chat_id: chat.into(),
            wire_id: Some(format!("w{id}")),
            direction: DIR_IN.into(),
            status: STATUS_RECEIVED.into(),
            content_type: CT_TEXT.into(),
            text: Some(text.into()),
            envelope_json: "{}".into(),
            sender_pubkey: "u".into(),
            reply_to_id: None,
            target_id: None,
            created_at: at,
            is_hidden: false,
            outbox_local_id: None,
            media_json: None,
        }
    }

    async fn chat(s: &Store, id: &str) {
        sqlx::query("INSERT OR IGNORE INTO msg_chats (id, kind, created_at, updated_at) VALUES (?, ?, 1, 1)")
            .bind(id)
            .bind(if id.starts_with("group:") { "group" } else { "dm" })
            .execute(s.pool())
            .await
            .unwrap();
    }

    async fn found(s: &Store, chat: &str, q: &str) -> Vec<String> {
        in_chat(s, chat, q, None, 50, false).await.unwrap().0.into_iter().map(|r| r.id).collect()
    }

    /// (messages with words in the index, rows of the index)
    async fn indexed(s: &Store) -> (i64, i64) {
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM msg_messages WHERE search_rowid IS NOT NULL").fetch_one(s.pool()).await.unwrap();
        let words: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM msg_search").fetch_one(s.pool()).await.unwrap();
        (rows, words)
    }

    /// The rule of `fold()` in the UI: the same examples hold on both sides
    /// (`ui/src/lib/messenger/shared/search.test.ts`).
    #[test]
    fn a_word_folds_as_the_ui_folds_it() {
        for (from, to) in [
            ("Ёлка", "елка"),
            ("Ёлена", "елена"),
            ("José", "jose"),
            ("Привет", "привет"),
            ("Йогурт", "иогурт"),
            ("ÅNGSTRÖM", "angstrom"),
            ("Straße", "straße"),
            ("İstanbul", "istanbul"),
            ("ΟΔΥΣΣΕΥΣ", "οδυσσευς"),
        ] {
            assert_eq!(fold(from), to, "{from}");
        }
    }

    #[test]
    fn a_query_is_words_and_nothing_of_it_is_an_operator() {
        assert_eq!(match_query("  "), None);
        assert_eq!(match_query("... --"), None, "no letter, no word");
        assert_eq!(match_query("Прив Мир").as_deref(), Some(r#""прив"* "мир"*"#));
        assert_eq!(match_query(r#"a"b OR NEAR(c"#).as_deref(), Some(r#""a"* "b"* "or"* "near"* "c"*"#));
        // Cut as the tokenizer cuts a message: every word on its own.
        assert_eq!(match_query("прив-мир").as_deref(), Some(r#""прив"* "мир"*"#));
        assert_eq!(match_query("один,два").as_deref(), Some(r#""один"* "два"*"#));
        assert_eq!(match_query("Отчёт_2026.pdf").as_deref(), Some(r#""отчет"* "2026"* "pdf"*"#));
    }

    #[test]
    fn what_of_a_message_is_searched() {
        assert_eq!(body_of(CT_TEXT, Some("Привет"), None, None, false).as_deref(), Some("привет"));
        assert_eq!(body_of(CT_TEXT, Some("**Жирный**"), Some("Жирный"), None, false).as_deref(), Some("жирныи"), "Markdown without its marks");
        assert_eq!(body_of(CT_TEXT, Some("x"), None, None, true), None, "deleted");
        assert_eq!(body_of("edit", Some("x"), None, None, false), None);
        assert_eq!(body_of("system", Some("x"), None, None, false), None);
        let file = r#"{"kind":"file","name":"Договор.pdf"}"#;
        assert_eq!(body_of(CT_MEDIA, Some("подпись"), None, Some(file), false).as_deref(), Some("подпись\nдоговор.pdf"));
        assert_eq!(body_of(CT_MEDIA, None, None, Some(file), false).as_deref(), Some("договор.pdf"));
        assert_eq!(body_of(CT_MEDIA, None, None, Some(r#"{"kind":"voice","name":"voice-1.ogg"}"#), false), None, "a name the app made up");
        assert_eq!(body_of("contact", None, None, Some(file), false), None);
    }

    #[tokio::test]
    async fn found_by_the_start_of_a_word_in_any_case_and_script() {
        let s = Store::open_in_memory().await.unwrap();
        messages::insert(&s, &said("a", "dm:bb", "Привет, как дела?", 10)).await.unwrap();
        messages::insert(&s, &said("b", "dm:bb", "елка в лесу", 20)).await.unwrap();
        messages::insert(&s, &said("c", "dm:bb", "José was here", 30)).await.unwrap();
        messages::insert(&s, &said("d", "dm:cc", "привет из другого чата", 40)).await.unwrap();
        assert_eq!(found(&s, "dm:bb", "прив").await, ["a"]);
        assert_eq!(found(&s, "dm:bb", "ПРИВЕТ").await, ["a"]);
        assert!(found(&s, "dm:bb", "ивет").await.is_empty(), "the start of a word, not a part of it");
        assert_eq!(found(&s, "dm:bb", "Ёлка").await, ["b"]);
        assert_eq!(found(&s, "dm:bb", "jose").await, ["c"]);
        assert_eq!(found(&s, "dm:bb", "josé WAS").await, ["c"], "every word");
        assert!(found(&s, "dm:bb", "jose елка").await.is_empty());
        assert!(found(&s, "dm:bb", r#"" OR * NEAR("#).await.is_empty(), "no operator gets through");
        assert!(found(&s, "dm:bb", "").await.is_empty());
        assert_eq!(found(&s, "dm:cc", "прив").await, ["d"], "one chat at a time");
        messages::insert(&s, &said("e", "dm:ee", "привет всему миру", 50)).await.unwrap();
        messages::insert(&s, &said("f", "dm:ee", "миру привет", 51)).await.unwrap();
        messages::insert(&s, &said("h", "dm:ee", "мирный,приветливый; отчет-2026.pdf", 52)).await.unwrap();
        for q in ["прив-мир", "прив мир", "мир,прив", "мир.прив"] {
            assert_eq!(found(&s, "dm:ee", q).await, ["h", "f", "e"], "{q}: every word, in any order, not next to each other");
        }
        assert_eq!(found(&s, "dm:ee", "отчет-2026.pdf").await, ["h"]);
        assert_eq!(found(&s, "dm:ee", "2026 pdf").await, ["h"]);

        // Newest first, a page strictly older than `before`, the count of all.
        for i in 0..5 {
            messages::insert(&s, &said(&format!("p{i}"), "dm:pp", "page", 100 + i)).await.unwrap();
        }
        let (page, total) = in_chat(&s, "dm:pp", "page", None, 2, false).await.unwrap();
        assert_eq!((page.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), total), (vec!["p4", "p3"], 5));
        let at = |c: i64, id: &str| Cursor { created_at: c, id: id.into() };
        let (page, _) = in_chat(&s, "dm:pp", "page", Some(&at(103, "p3")), 10, false).await.unwrap();
        assert_eq!(page.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["p2", "p1", "p0"]);

        // Of the same second across the end of a page: none lost, none twice.
        for i in 0..51 {
            messages::insert(&s, &said(&format!("t{i:02}"), "dm:tt", "тик", 100)).await.unwrap();
        }
        let (first, total) = in_chat(&s, "dm:tt", "тик", None, 50, false).await.unwrap();
        assert_eq!((first.len(), total), (50, 51));
        let last = first.last().unwrap();
        let (next, _) = in_chat(&s, "dm:tt", "тик", Some(&at(last.created_at, &last.id)), 50, false).await.unwrap();
        assert_eq!(next.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["t00"]);
    }

    #[tokio::test]
    async fn an_edit_a_deletion_and_a_file_change_what_is_found() {
        let s = Store::open_in_memory().await.unwrap();
        messages::insert(&s, &said("a", "dm:bb", "старый текст", 10)).await.unwrap();
        messages::set_text(&s, "a", "новый **текст**", TextFormat::Markdown, 11).await.unwrap();
        assert!(found(&s, "dm:bb", "старый").await.is_empty(), "the old words leave");
        assert_eq!(found(&s, "dm:bb", "новый текст").await, ["a"]);
        assert_eq!(indexed(&s).await, (1, 1), "one row of words per message");
        messages::mark_deleted(&s, "a", 12).await.unwrap();
        assert!(found(&s, "dm:bb", "новый").await.is_empty());
        assert_eq!(indexed(&s).await, (0, 0));

        let mut file = said("f", "dm:bb", "", 20);
        file.content_type = CT_MEDIA.into();
        file.media_json = Some(r#"{"kind":"file","name":"отчёт.pdf"}"#.into());
        messages::insert(&s, &file).await.unwrap();
        assert_eq!(found(&s, "dm:bb", "отчет").await, ["f"]);
        messages::set_media_json(&s, "f", r#"{"kind":"file","name":"смета.xlsx"}"#).await.unwrap();
        assert!(found(&s, "dm:bb", "отчет").await.is_empty());
        assert_eq!(found(&s, "dm:bb", "смета").await, ["f"]);
        messages::delete(&s, "f").await.unwrap();
        assert_eq!(indexed(&s).await, (0, 0));
        assert!(messages::insert(&s, &said("g", "dm:bb", "дубль", 30)).await.unwrap());
        assert!(!messages::insert(&s, &said("g", "dm:bb", "дубль", 30)).await.unwrap(), "a copy again");
        assert_eq!(indexed(&s).await, (1, 1), "indexed once");
    }

    #[tokio::test]
    async fn found_as_the_chat_shows_it() {
        use crate::dm_relations::{put, RelationRow};
        let s = Store::open_in_memory().await.unwrap();
        let g = "group:g";
        for (id, who) in [("ok", "u"), ("hid", "u"), ("wait", "u"), ("blk", "x"), ("gone", "u")] {
            messages::insert(&s, &NewMessage { sender_pubkey: who.into(), ..said(id, g, "слово", 10) }).await.unwrap();
        }
        let mut mine = said("mine", g, "слово", 11);
        mine.direction = DIR_OUT.into();
        messages::insert(&s, &mine).await.unwrap();
        messages::set_mod_state(&s, "hid", MOD_HIDDEN).await.unwrap();
        messages::set_review(&s, "wait", REVIEW_PENDING).await.unwrap();
        messages::set_review(&s, "mine", REVIEW_PENDING).await.unwrap();
        messages::remember_hidden(&s, "gone", 12).await.unwrap();
        put(
            &s,
            &RelationRow {
                peer_pubkey: "x".into(),
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
            },
        )
        .await
        .unwrap();
        let ids = |rows: Vec<MessageRow>| {
            let mut v: Vec<String> = rows.into_iter().map(|r| r.id).collect();
            v.sort();
            v
        };
        let member = in_chat(&s, g, "слово", None, 50, false).await.unwrap();
        assert_eq!((ids(member.0), member.1), (vec!["mine".to_string(), "ok".into()], 2), "a member: not hidden, not waiting, not blocked, not removed for me");
        let moderator = in_chat(&s, g, "слово", None, 50, true).await.unwrap();
        assert_eq!(ids(moderator.0), ["mine", "ok", "wait"], "a moderator: what waits too, never what is hidden");

        // A held request of a stranger is found once let in.
        messages::insert(&s, &NewMessage { is_hidden: true, ..said("held", "dm:st", "незнакомец", 20) }).await.unwrap();
        crate::dm_held::hold(&s, "held", "dm:st", 20).await.unwrap();
        assert!(found(&s, "dm:st", "незнакомец").await.is_empty());
        crate::dm_held::release(&s, "held").await.unwrap();
        assert_eq!(found(&s, "dm:st", "незнакомец").await, ["held"]);
    }

    #[tokio::test]
    async fn across_chats_by_the_newest_match_with_their_counts() {
        let s = Store::open_in_memory().await.unwrap();
        for c in ["dm:aa", "dm:bb", "group:g"] {
            chat(&s, c).await;
        }
        messages::insert(&s, &said("a1", "dm:aa", "встреча завтра", 10)).await.unwrap();
        messages::insert(&s, &said("a2", "dm:aa", "Встреча отменена", 30)).await.unwrap();
        messages::insert(&s, &said("b1", "dm:bb", "встреча в 10", 20)).await.unwrap();
        messages::insert(&s, &said("g1", "group:g", "встреча группы", 40)).await.unwrap();
        messages::set_review(&s, "g1", REVIEW_PENDING).await.unwrap();
        messages::insert(&s, &said("n1", "dm:nochat", "встреча", 50)).await.unwrap();
        assert_eq!(groups_matching(&s, "встре").await.unwrap(), ["group:g"]);
        let hits = |h: Vec<ChatHitRow>| h.into_iter().map(|h| (h.chat_id, h.count, h.last.id)).collect::<Vec<_>>();
        let t = |c: &str, n: i64, l: &str| (c.to_string(), n, l.to_string());
        assert_eq!(hits(across_chats(&s, "встре", 10, &[]).await.unwrap()), [t("dm:aa", 2, "a2"), t("dm:bb", 1, "b1")], "a chat that is not there is not listed");
        assert_eq!(
            hits(across_chats(&s, "встре", 10, &["group:g".into()]).await.unwrap()),
            [t("group:g", 1, "g1"), t("dm:aa", 2, "a2"), t("dm:bb", 1, "b1")],
            "what waits, in a group I moderate"
        );
        assert_eq!(hits(across_chats(&s, "встре", 1, &[]).await.unwrap()), [t("dm:aa", 2, "a2")]);
        assert!(across_chats(&s, " ", 10, &[]).await.unwrap().is_empty());

        // The newest match is the newest shown: hidden by a moderator, or
        // edited into waiting for one, it is not the one given, nor its
        // text.
        messages::set_mod_state(&s, "a2", MOD_HIDDEN).await.unwrap();
        let a = across_chats(&s, "встре", 10, &[]).await.unwrap().into_iter().find(|h| h.chat_id == "dm:aa").unwrap();
        assert_eq!((a.count, a.last.id.as_str(), a.last.text.as_deref()), (1, "a1", Some("встреча завтра")));
        messages::set_text(&s, "b1", "встреча перенесена", TextFormat::Plain, 21).await.unwrap();
        messages::set_review(&s, "b1", REVIEW_PENDING).await.unwrap();
        assert_eq!(hits(across_chats(&s, "встре", 10, &[]).await.unwrap()), [t("dm:aa", 1, "a1")]);
    }

    #[tokio::test]
    async fn the_words_go_with_their_chat_and_with_the_account() {
        let s = Store::open_in_memory().await.unwrap();
        crate::chats::ensure_dm(&s, "bb", None).await.unwrap();
        messages::insert(&s, &said("a", "dm:bb", "слово", 10)).await.unwrap();
        messages::insert(&s, &said("k", "dm:kk", "слово", 10)).await.unwrap();
        crate::chats::delete(&s, "dm:bb", 100).await.unwrap();
        assert!(found(&s, "dm:bb", "слово").await.is_empty());
        assert_eq!(indexed(&s).await, (1, 1), "the other chat keeps its words");

        // A deletion from another device of mine: up to its times.
        crate::chats::ensure_dm(&s, "cc", None).await.unwrap();
        messages::insert(&s, &said("c1", "dm:cc", "слово", 10)).await.unwrap();
        messages::insert(&s, &said("c2", "dm:cc", "слово", 50)).await.unwrap();
        crate::chats::delete_copy(&s, "dm:cc", 20, 20, 100).await.unwrap();
        assert_eq!(found(&s, "dm:cc", "слово").await, ["c2"]);

        // Held rows of a stranger purged; a group forgotten.
        messages::insert(&s, &NewMessage { is_hidden: true, ..said("h", "dm:st", "слово", 10) }).await.unwrap();
        crate::dm_held::hold(&s, "h", "dm:st", 10).await.unwrap();
        crate::dm_held::purge(&s, "dm:st").await.unwrap();
        messages::insert(&s, &said("g", "group:gg", "слово", 10)).await.unwrap();
        crate::groups::delete(&s, "gg").await.unwrap();
        let left: Vec<String> =
            sqlx::query_scalar("SELECT id FROM msg_messages WHERE search_rowid IS NOT NULL ORDER BY id").fetch_all(s.pool()).await.unwrap();
        assert_eq!(left, ["c2", "k"]);
        assert_eq!(indexed(&s).await, (2, 2), "no words left behind");

        crate::account::wipe(&s).await.unwrap();
        assert_eq!(indexed(&s).await, (0, 0));
        assert_eq!(crate::settings::get(&s, KEY_BACKFILLED_TO).await.unwrap().as_deref(), Some(BACKFILL_WHOLE), "an empty store is whole");
    }

    /// Rows put in `msg_messages` behind the index's back, as a store from
    /// before 049 holds them.
    async fn unindexed(s: &Store, n: usize) {
        let mut tx = s.pool().begin().await.unwrap();
        for i in 0..n {
            sqlx::query(
                "INSERT INTO msg_messages (id, chat_id, direction, status, content_type, text, envelope_json, sender_pubkey, created_at, received_at)
                 VALUES (?, 'dm:bb', 'in', 'received', 'text', ?, '{}', 'bb', ?, 1)",
            )
            .bind(format!("m{i:06}"))
            .bind(format!("слово номер{i}"))
            .bind(i as i64)
            .execute(&mut *tx)
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();
    }

    #[tokio::test]
    async fn the_backfill_goes_on_from_where_it_stopped_for_its_owner_alone() {
        let s = Store::open_in_memory().await.unwrap();
        crate::account::set_owner(&s, "aa").await.unwrap();
        unindexed(&s, 4500).await;
        prepare(&s).await.unwrap();
        assert!(!is_whole(&s).await.unwrap());
        assert_eq!(backfill_batch(&s, Some("aa")).await.unwrap(), Step::More(2000));
        assert_eq!(crate::settings::get(&s, KEY_BACKFILLED_TO).await.unwrap().as_deref(), Some("after:m001999"));
        assert_eq!(in_chat(&s, "dm:bb", "слово", None, 1, false).await.unwrap().1, 2000, "partial, so far");
        // Killed here: the next start prepares again and goes on.
        prepare(&s).await.unwrap();
        assert_eq!(backfill_batch(&s, Some("bb")).await.unwrap(), Step::Stopped, "another owner's store is left alone");
        assert_eq!(backfill_batch(&s, Some("aa")).await.unwrap(), Step::More(2000));
        // A row gone without its words is swept by the last batch; a new
        // message takes no rowid of those words meanwhile.
        sqlx::query("DELETE FROM msg_messages WHERE id = 'm000000'").execute(s.pool()).await.unwrap();
        messages::insert(&s, &said("new", "dm:bb", "свежее", 9000)).await.unwrap();
        assert_eq!(backfill_batch(&s, Some("aa")).await.unwrap(), Step::Whole(501));
        assert!(is_whole(&s).await.unwrap());
        assert_eq!(indexed(&s).await, (4500, 4500));
        assert_eq!(found(&s, "dm:bb", "номер4499").await, ["m004499"]);
        assert_eq!(found(&s, "dm:bb", "свеж").await, ["new"]);
        assert_eq!(backfill_batch(&s, Some("aa")).await.unwrap(), Step::Whole(0));

        // Another rule of folding: written again from the first row.
        crate::settings::set(&s, KEY_FOLD_VERSION, "0").await.unwrap();
        prepare(&s).await.unwrap();
        assert_eq!(indexed(&s).await, (0, 0));
        assert!(!is_whole(&s).await.unwrap());

        // A wipe while it runs: the next batch finds an empty store, whole.
        crate::account::wipe(&s).await.unwrap();
        assert_eq!(backfill_batch(&s, Some("aa")).await.unwrap(), Step::Stopped);
        assert_eq!(backfill_batch(&s, None).await.unwrap(), Step::Whole(0));
        assert!(is_whole(&s).await.unwrap());
    }
}
