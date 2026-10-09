// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Reactions on messages. One row per message, author and emoji; the later
//! `created_at` wins, and a taken-back reaction stays as `removed` so that
//! an older put coming after it still loses. Rows are kept by message id,
//! not joined to `msg_messages`: a reaction may come before its message.
//! What is shown is read by chat as well, so a reaction that names a
//! message of another chat shows nowhere.
//!
//! Every word that comes is kept, whatever stands already: of one author's
//! emoji standing on a message only the earliest `MAX_PER_AUTHOR` count.
//! A rule checked on arrival would depend on what had arrived before, and
//! devices hearing the same notes in another order would show different
//! things for good; read from the rows, every device shows the same.

use crate::{storage, Store};
use messenger_core::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Emoji of one author on one message that count: the earliest standing.
pub const MAX_PER_AUTHOR: i64 = 3;

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct ReactionRow {
    pub message_id: String,
    /// The chat the reaction came in: `dm:<peer>` or `group:<id>`.
    pub chat_id: String,
    /// Hex key of who reacted.
    pub author: String,
    pub emoji: String,
    pub created_at: i64,
    pub removed: bool,
}

/// One emoji under a message, for the host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactionView {
    pub emoji: String,
    /// How many people have it on the message.
    pub count: i64,
    /// I am one of them.
    pub mine: bool,
}

/// The most ids one aggregate statement binds; a page is far smaller, a
/// longer list is read in several.
const IDS_PER_QUERY: usize = 500;

/// Keep what a reaction says if it is newer than what is stored: the later
/// `created_at` wins the whole row. On the same second a take-back wins
/// over a put, so two devices that did both at once still agree; the very
/// same event again changes nothing. `now` is when this device first keeps
/// the row (`prune_orphans`). `true` when the stored state changed.
pub async fn put(store: &Store, row: &ReactionRow, now: i64) -> Result<bool> {
    let r = sqlx::query(
        "INSERT INTO msg_reactions (message_id, chat_id, author, emoji, created_at, removed, kept_at) VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(message_id, author, emoji) DO UPDATE SET
           chat_id = excluded.chat_id, created_at = excluded.created_at, removed = excluded.removed
         WHERE excluded.created_at > msg_reactions.created_at
            OR (excluded.created_at = msg_reactions.created_at AND excluded.removed > msg_reactions.removed)",
    )
    .bind(&row.message_id)
    .bind(&row.chat_id)
    .bind(&row.author)
    .bind(&row.emoji)
    .bind(row.created_at)
    .bind(row.removed)
    .bind(now)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(r.rows_affected() > 0)
}

/// The stored row of one author's emoji on a message, taken back or not.
/// The sender stamps its next change later than this one, or a quick
/// second tap in the same second would lose.
pub async fn get(store: &Store, message_id: &str, author: &str, emoji: &str) -> Result<Option<ReactionRow>> {
    sqlx::query_as::<_, ReactionRow>(
        "SELECT message_id, chat_id, author, emoji, created_at, removed FROM msg_reactions
         WHERE message_id = ? AND author = ? AND emoji = ?",
    )
    .bind(message_id)
    .bind(author)
    .bind(emoji)
    .fetch_optional(store.pool())
    .await
    .map_err(storage)
}

/// The reactions that count on a message in a chat (standing, and among
/// the earliest `MAX_PER_AUTHOR` of their author), the oldest first.
pub async fn active(store: &Store, message_id: &str, chat_id: &str) -> Result<Vec<ReactionRow>> {
    sqlx::query_as::<_, ReactionRow>(
        "SELECT message_id, chat_id, author, emoji, created_at, removed FROM (
           SELECT message_id, chat_id, author, emoji, created_at, removed,
                  ROW_NUMBER() OVER (PARTITION BY author ORDER BY created_at, emoji) AS n
           FROM msg_reactions
           WHERE message_id = ? AND chat_id = ? AND removed = 0)
         WHERE n <= ?
         ORDER BY created_at, author, emoji",
    )
    .bind(message_id)
    .bind(chat_id)
    .bind(MAX_PER_AUTHOR)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// What stands under each of `message_ids` in a chat, read in one go for a
/// whole page: per message, one entry per emoji, in the order the emoji
/// first appeared; `mine` when `me` is among its authors. Only what counts
/// is summed (`active`). A message with nothing on it has no entry.
pub async fn aggregate_many(
    store: &Store,
    chat_id: &str,
    message_ids: &[String],
    me: &str,
) -> Result<HashMap<String, Vec<ReactionView>>> {
    let mut out: HashMap<String, Vec<ReactionView>> = HashMap::new();
    for ids in message_ids.chunks(IDS_PER_QUERY) {
        // Plain `?` throughout: sqlx numbers a bare one from 1 even after a
        // `?2`, so mixing them binds the wrong values.
        let marks = vec!["?"; ids.len()].join(", ");
        let sql = format!(
            "SELECT message_id, emoji, COUNT(*), MAX(author = ?), MIN(created_at) AS first_at
             FROM (
               SELECT message_id, author, emoji, created_at,
                      ROW_NUMBER() OVER (PARTITION BY message_id, author ORDER BY created_at, emoji) AS n
               FROM msg_reactions
               WHERE chat_id = ? AND removed = 0 AND message_id IN ({marks}))
             WHERE n <= ?
             GROUP BY message_id, emoji
             ORDER BY message_id, first_at, emoji"
        );
        let mut q = sqlx::query_as::<_, (String, String, i64, bool, i64)>(sqlx::AssertSqlSafe(sql)).bind(me).bind(chat_id);
        for id in ids {
            q = q.bind(id);
        }
        q = q.bind(MAX_PER_AUTHOR);
        for (message_id, emoji, count, mine, _) in q.fetch_all(store.pool()).await.map_err(storage)? {
            out.entry(message_id).or_default().push(ReactionView { emoji, count, mine });
        }
    }
    Ok(out)
}

/// Forget reactions kept before `kept_before` whose message is not in the
/// chat they came in: it never came, it was removed, or the note named a
/// message of another chat. Returns how many went.
pub async fn prune_orphans(store: &Store, kept_before: i64) -> Result<u64> {
    let r = sqlx::query(
        "DELETE FROM msg_reactions WHERE kept_at < ? AND NOT EXISTS (
           SELECT 1 FROM msg_messages m WHERE m.id = msg_reactions.message_id AND m.chat_id = msg_reactions.chat_id)",
    )
    .bind(kept_before)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(r.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(message: &str, chat: &str, author: &str, emoji: &str, at: i64, removed: bool) -> ReactionRow {
        ReactionRow {
            message_id: message.into(),
            chat_id: chat.into(),
            author: author.into(),
            emoji: emoji.into(),
            created_at: at,
            removed,
        }
    }

    fn view(emoji: &str, count: i64, mine: bool) -> ReactionView {
        ReactionView { emoji: emoji.into(), count, mine }
    }

    #[tokio::test]
    async fn the_later_word_wins_whatever_the_order() {
        let s = Store::open_in_memory().await.unwrap();
        assert!(put(&s, &row("m1", "dm:bb", "bb", "👍", 100, false), 0).await.unwrap());
        assert!(!put(&s, &row("m1", "dm:bb", "bb", "👍", 100, false), 0).await.unwrap(), "the same event again");
        assert!(put(&s, &row("m1", "dm:bb", "bb", "👍", 200, true), 0).await.unwrap(), "taken back later");
        assert!(!put(&s, &row("m1", "dm:bb", "bb", "👍", 150, false), 0).await.unwrap(), "an older put replayed loses");
        assert!(active(&s, "m1", "dm:bb").await.unwrap().is_empty());
        assert_eq!(get(&s, "m1", "bb", "👍").await.unwrap(), Some(row("m1", "dm:bb", "bb", "👍", 200, true)));
        assert!(!put(&s, &row("m1", "dm:bb", "bb", "👍", 200, true), 0).await.unwrap(), "equal keeps");
        assert!(put(&s, &row("m1", "dm:bb", "bb", "👍", 300, false), 0).await.unwrap(), "put again");
        assert_eq!(active(&s, "m1", "dm:bb").await.unwrap(), vec![row("m1", "dm:bb", "bb", "👍", 300, false)]);

        // Taken back before the put arrives: the put is old news.
        assert!(put(&s, &row("m2", "dm:bb", "bb", "🔥", 500, true), 0).await.unwrap());
        assert!(!put(&s, &row("m2", "dm:bb", "bb", "🔥", 400, false), 0).await.unwrap());
        assert!(active(&s, "m2", "dm:bb").await.unwrap().is_empty());
        assert_eq!(get(&s, "m9", "bb", "🔥").await.unwrap(), None);
    }

    #[tokio::test]
    async fn on_the_same_second_a_take_back_wins_both_ways() {
        let a = Store::open_in_memory().await.unwrap();
        let b = Store::open_in_memory().await.unwrap();
        let on = row("m1", "dm:bb", "aa", "👍", 100, false);
        let off = row("m1", "dm:bb", "aa", "👍", 100, true);
        assert!(put(&a, &on, 0).await.unwrap());
        assert!(put(&a, &off, 0).await.unwrap());
        assert!(put(&b, &off, 0).await.unwrap());
        assert!(!put(&b, &on, 0).await.unwrap(), "an equal put keeps the take-back");
        assert_eq!(get(&a, "m1", "aa", "👍").await.unwrap(), get(&b, "m1", "aa", "👍").await.unwrap());
    }

    #[tokio::test]
    async fn active_reads_one_chat() {
        let s = Store::open_in_memory().await.unwrap();
        put(&s, &row("m1", "dm:bb", "bb", "😂", 300, false), 0).await.unwrap();
        put(&s, &row("m1", "dm:bb", "aa", "👍", 100, false), 0).await.unwrap();
        put(&s, &row("m1", "dm:bb", "bb", "👍", 200, false), 0).await.unwrap();
        put(&s, &row("m1", "dm:bb", "bb", "🔥", 250, true), 0).await.unwrap();
        put(&s, &row("m1", "dm:cc", "cc", "💩", 50, false), 0).await.unwrap();

        let got: Vec<_> = active(&s, "m1", "dm:bb").await.unwrap().into_iter().map(|r| (r.author, r.emoji)).collect();
        let theirs: Vec<_> = active(&s, "m1", "dm:cc").await.unwrap().into_iter().map(|r| (r.author, r.emoji)).collect();
        assert_eq!(theirs, vec![("cc".into(), "💩".into())], "another chat naming the same message");
        assert_eq!(got, vec![("aa".into(), "👍".into()), ("bb".into(), "👍".into()), ("bb".into(), "😂".into())]);
    }

    #[tokio::test]
    async fn a_page_is_summed_per_message_in_the_order_emoji_came() {
        let s = Store::open_in_memory().await.unwrap();
        let g = "group:g";
        put(&s, &row("m1", g, "bb", "😂", 300, false), 0).await.unwrap();
        put(&s, &row("m1", g, "cc", "👍", 200, false), 0).await.unwrap();
        put(&s, &row("m1", g, "aa", "👍", 400, false), 0).await.unwrap();
        put(&s, &row("m1", g, "dd", "🔥", 100, true), 0).await.unwrap();
        put(&s, &row("m2", g, "bb", "❤️", 500, false), 0).await.unwrap();
        put(&s, &row("m2", g, "aa", "🔥", 600, true), 0).await.unwrap();
        // Another chat naming a message of this one: not shown here.
        put(&s, &row("m1", "dm:ee", "ee", "💩", 10, false), 0).await.unwrap();
        // Not asked for.
        put(&s, &row("m3", g, "bb", "👍", 10, false), 0).await.unwrap();

        let ids = vec!["m1".to_string(), "m2".to_string(), "m4".to_string()];
        let got = aggregate_many(&s, g, &ids, "aa").await.unwrap();
        assert_eq!(got.len(), 2, "nothing on m4, m3 not asked: {got:?}");
        assert_eq!(got["m1"], vec![view("👍", 2, true), view("😂", 1, false)], "👍 came first; the taken-back 🔥 is gone");
        assert_eq!(got["m2"], vec![view("❤️", 1, false)], "my 🔥 was taken back");

        let theirs = aggregate_many(&s, "dm:ee", &ids, "aa").await.unwrap();
        assert_eq!(theirs["m1"], vec![view("💩", 1, false)]);
        assert!(aggregate_many(&s, g, &[], "aa").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_long_list_is_read_in_parts() {
        let s = Store::open_in_memory().await.unwrap();
        let ids: Vec<String> = (0..(IDS_PER_QUERY + 3)).map(|i| format!("m{i}")).collect();
        put(&s, &row("m0", "dm:bb", "bb", "👍", 1, false), 0).await.unwrap();
        put(&s, &row(&ids[IDS_PER_QUERY + 2], "dm:bb", "bb", "👍", 1, false), 0).await.unwrap();
        let got = aggregate_many(&s, "dm:bb", &ids, "aa").await.unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[&ids[IDS_PER_QUERY + 2]], vec![view("👍", 1, false)]);
    }

    /// Bob's own three, and a fourth from a device of his that did not know
    /// of the take-back yet: every order of arrival shows the same.
    #[tokio::test]
    async fn an_authors_earliest_three_count_whatever_the_order() {
        let g = "group:g";
        let notes = [
            row("m1", g, "bb", "👍", 100, false),
            row("m1", g, "bb", "❤️", 200, false),
            row("m1", g, "bb", "😂", 300, false),
            row("m1", g, "bb", "😂", 500, true),
            row("m1", g, "bb", "🔥", 600, false),
            row("m1", g, "cc", "😂", 50, false),
        ];
        let want = vec![view("😂", 1, false), view("👍", 1, true), view("❤️", 1, true), view("🔥", 1, true)];
        let orders: [[usize; 6]; 4] = [[0, 1, 2, 3, 4, 5], [4, 0, 1, 2, 3, 5], [0, 1, 2, 4, 5, 3], [5, 4, 3, 2, 1, 0]];
        for order in orders {
            let s = Store::open_in_memory().await.unwrap();
            for i in order {
                put(&s, &notes[i], 0).await.unwrap();
            }
            let got = aggregate_many(&s, g, &["m1".to_string()], "bb").await.unwrap();
            assert_eq!(got["m1"], want, "{order:?}");
            let bobs: Vec<_> = active(&s, "m1", g).await.unwrap().into_iter().filter(|r| r.author == "bb").map(|r| r.emoji).collect();
            assert_eq!(bobs, vec!["👍", "❤️", "🔥"], "{order:?}");
        }

        // Over the limit, from devices that did not hear of each other: the
        // earliest three show, the fourth waits for one of them to go.
        let s = Store::open_in_memory().await.unwrap();
        for (i, e) in ["👍", "❤️", "😂", "🔥"].into_iter().enumerate().rev() {
            put(&s, &row("m1", g, "bb", e, 100 + i as i64, false), 0).await.unwrap();
        }
        let shown = |s: Store| async move { aggregate_many(&s, g, &["m1".to_string()], "aa").await.unwrap()["m1"].clone() };
        assert_eq!(shown(s.clone()).await, vec![view("👍", 1, false), view("❤️", 1, false), view("😂", 1, false)]);
        put(&s, &row("m1", g, "bb", "❤️", 200, true), 0).await.unwrap();
        assert_eq!(shown(s.clone()).await, vec![view("👍", 1, false), view("😂", 1, false), view("🔥", 1, false)]);
    }

    fn message(id: &str, chat: &str) -> crate::messages::NewMessage {
        crate::messages::NewMessage {
            id: id.into(),
            chat_id: chat.into(),
            wire_id: None,
            direction: "in".into(),
            status: "received".into(),
            content_type: "text".into(),
            text: Some("hi".into()),
            envelope_json: "{}".into(),
            sender_pubkey: "bb".into(),
            reply_to_id: None,
            target_id: None,
            created_at: 10,
            is_hidden: false,
            outbox_local_id: None,
            media_json: None,
        }
    }

    #[tokio::test]
    async fn what_names_no_message_of_its_chat_is_swept_after_a_while() {
        let s = Store::open_in_memory().await.unwrap();
        crate::messages::insert(&s, &message("m1", "dm:bb")).await.unwrap();
        put(&s, &row("m1", "dm:bb", "bb", "👍", 100, false), 1_000).await.unwrap();
        put(&s, &row("m1", "dm:bb", "aa", "🔥", 100, true), 1_000).await.unwrap();
        // Of a stranger, naming ids that never come; and a message of another chat.
        put(&s, &row("x1", "dm:ss", "ss", "👍", 100, false), 1_000).await.unwrap();
        put(&s, &row("m1", "dm:ss", "ss", "💩", 100, false), 1_000).await.unwrap();
        // Before its message, kept late: it may still come.
        put(&s, &row("m2", "dm:bb", "bb", "❤️", 100, false), 5_000).await.unwrap();
        // A later word does not make the row young again.
        put(&s, &row("x1", "dm:ss", "ss", "👍", 200, true), 4_000).await.unwrap();

        assert_eq!(prune_orphans(&s, 2_000).await.unwrap(), 2);
        assert!(get(&s, "x1", "ss", "👍").await.unwrap().is_none());
        assert!(get(&s, "m1", "ss", "💩").await.unwrap().is_none());
        assert!(get(&s, "m1", "aa", "🔥").await.unwrap().is_some(), "a take-back on a message here stays");
        assert!(get(&s, "m2", "bb", "❤️").await.unwrap().is_some());
        assert_eq!(prune_orphans(&s, 2_000).await.unwrap(), 0);

        crate::messages::insert(&s, &message("m2", "dm:bb")).await.unwrap();
        assert_eq!(prune_orphans(&s, 9_000).await.unwrap(), 0, "its message came");
        assert_eq!(aggregate_many(&s, "dm:bb", &["m2".to_string()], "aa").await.unwrap()["m2"], vec![view("❤️", 1, false)]);
    }

    #[tokio::test]
    async fn a_deleted_chat_takes_its_reactions_along() {
        let s = Store::open_in_memory().await.unwrap();
        crate::messages::insert(&s, &message("m1", "dm:bb")).await.unwrap();
        put(&s, &row("m1", "dm:bb", "bb", "👍", 100, false), 0).await.unwrap();
        put(&s, &row("x1", "dm:bb", "bb", "👍", 100, false), 0).await.unwrap();
        put(&s, &row("m1", "dm:cc", "cc", "👍", 100, false), 0).await.unwrap();
        crate::chats::delete(&s, "dm:bb", 1_000).await.unwrap();
        assert!(get(&s, "m1", "bb", "👍").await.unwrap().is_none());
        assert!(get(&s, "x1", "bb", "👍").await.unwrap().is_none());
        assert!(get(&s, "m1", "cc", "👍").await.unwrap().is_some(), "another chat's stays");
    }
}
