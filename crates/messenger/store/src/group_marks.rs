// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_group_marks`: the decisions of moderators in groups, as they came
//! (`mod.hide`, `mod.unhide`, `mod.approve`, `mod.reject`). A mark is kept
//! whether or not it is in force: `active` is decided by the journal as
//! it stands now and rewritten whenever the journal changes, so that a
//! moderator who was demoted loses the force of every mark, and one who
//! was raised gains it, on every device alike. A note that names several
//! messages is one row per message, each with its own `active` (the
//! author of each target has a role of their own). Nothing is deleted:
//! a mark that a later one overrides still tells the log what happened.

use crate::{storage, Store};
use messenger_core::Result;

pub const KIND_HIDE: &str = "hide";
pub const KIND_UNHIDE: &str = "unhide";
pub const KIND_APPROVE: &str = "approve";
pub const KIND_REJECT: &str = "reject";

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct MarkRow {
    /// The inner event of the note (signed by `by`).
    pub note_id: String,
    pub group_id: String,
    /// `hide` | `unhide` | `approve` | `reject`
    pub kind: String,
    /// The message the mark is about; empty for a mark about a span of an
    /// author's messages.
    pub target_id: String,
    /// Of a span: whose messages, between `from_at` and `to_at` (their
    /// `created_at`, inclusive).
    pub author: Option<String>,
    pub from_at: Option<i64>,
    pub to_at: Option<i64>,
    /// Of an approval: the version of the message it approves.
    pub version: Option<String>,
    pub reason: String,
    /// Hex pubkey of the moderator.
    pub by: String,
    /// The moderator's clock, as signed.
    pub at: i64,
    /// In force by the journal as it stands now.
    pub active: bool,
}

const COLS: &str = "note_id, group_id, kind, target_id, author, from_at, to_at, version, reason, by, at, active";

/// Keep a mark; `false` when this row (note and target) is here already.
pub async fn insert_if_new(store: &Store, m: &MarkRow) -> Result<bool> {
    let res = sqlx::query(
        "INSERT OR IGNORE INTO msg_group_marks (note_id, group_id, kind, target_id, author, from_at, to_at, version, reason, by, at, active)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&m.note_id)
    .bind(&m.group_id)
    .bind(&m.kind)
    .bind(&m.target_id)
    .bind(&m.author)
    .bind(m.from_at)
    .bind(m.to_at)
    .bind(&m.version)
    .bind(&m.reason)
    .bind(&m.by)
    .bind(m.at)
    .bind(m.active)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(res.rows_affected() == 1)
}

/// Every mark of a group, newest first (by the moderators' clocks).
pub async fn by_group(store: &Store, group_id: &str) -> Result<Vec<MarkRow>> {
    sqlx::query_as::<_, MarkRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM msg_group_marks WHERE group_id = ? ORDER BY at DESC, note_id ASC, target_id ASC"
    )))
    .bind(group_id)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// The marks about one message.
pub async fn by_target(store: &Store, group_id: &str, target_id: &str) -> Result<Vec<MarkRow>> {
    sqlx::query_as::<_, MarkRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM msg_group_marks WHERE group_id = ? AND target_id = ? ORDER BY at DESC, note_id ASC"
    )))
    .bind(group_id)
    .bind(target_id)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// The marks about a span of an author's messages that `created_at`
/// falls into.
pub async fn by_author_range(store: &Store, group_id: &str, author: &str, created_at: i64) -> Result<Vec<MarkRow>> {
    sqlx::query_as::<_, MarkRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM msg_group_marks
         WHERE group_id = ? AND target_id = '' AND author = ? AND from_at <= ? AND to_at >= ?
         ORDER BY at DESC, note_id ASC"
    )))
    .bind(group_id)
    .bind(author)
    .bind(created_at)
    .bind(created_at)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// The marks one moderator made in a group.
pub async fn by_author_of_marks(store: &Store, group_id: &str, by: &str) -> Result<Vec<MarkRow>> {
    sqlx::query_as::<_, MarkRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM msg_group_marks WHERE group_id = ? AND by = ? ORDER BY at DESC, note_id ASC, target_id ASC"
    )))
    .bind(group_id)
    .bind(by)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// Rewrite whether marks are in force: `(note_id, target_id, active)`
/// for each. Rows not named stay as they are.
pub async fn set_active_bulk(store: &Store, group_id: &str, changes: &[(String, String, bool)]) -> Result<()> {
    for (note_id, target_id, active) in changes {
        sqlx::query("UPDATE msg_group_marks SET active = ? WHERE group_id = ? AND note_id = ? AND target_id = ?")
            .bind(active)
            .bind(group_id)
            .bind(note_id)
            .bind(target_id)
            .execute(store.pool())
            .await
            .map_err(storage)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(note: &str, target: &str, kind: &str, at: i64) -> MarkRow {
        MarkRow {
            note_id: note.into(),
            group_id: "g".into(),
            kind: kind.into(),
            target_id: target.into(),
            author: None,
            from_at: None,
            to_at: None,
            version: None,
            reason: String::new(),
            by: "mod".into(),
            at,
            active: true,
        }
    }

    #[tokio::test]
    async fn marks_are_kept_once_per_note_and_target_and_found_by_target_or_span() {
        let s = Store::open_in_memory().await.unwrap();
        assert!(insert_if_new(&s, &mark("n1", "a", KIND_HIDE, 10)).await.unwrap());
        assert!(insert_if_new(&s, &mark("n1", "b", KIND_HIDE, 10)).await.unwrap(), "one note, two targets");
        assert!(!insert_if_new(&s, &mark("n1", "a", KIND_HIDE, 10)).await.unwrap(), "the same again");
        let span = MarkRow { author: Some("x".into()), from_at: Some(100), to_at: Some(200), ..mark("n2", "", KIND_HIDE, 20) };
        assert!(insert_if_new(&s, &span).await.unwrap());
        assert_eq!(by_target(&s, "g", "a").await.unwrap().len(), 1);
        assert_eq!(by_group(&s, "g").await.unwrap().iter().map(|m| m.note_id.as_str()).collect::<Vec<_>>(), vec!["n2", "n1", "n1"]);
        assert_eq!(by_author_range(&s, "g", "x", 150).await.unwrap().len(), 1);
        assert_eq!(by_author_range(&s, "g", "x", 201).await.unwrap().len(), 0, "outside the span");
        assert_eq!(by_author_range(&s, "g", "y", 150).await.unwrap().len(), 0, "another author");
        set_active_bulk(&s, "g", &[("n1".into(), "a".into(), false)]).await.unwrap();
        let rows = by_target(&s, "g", "a").await.unwrap();
        assert!(!rows[0].active);
        assert!(by_target(&s, "g", "b").await.unwrap()[0].active, "the other target of the note is untouched");
        assert_eq!(by_author_of_marks(&s, "g", "mod").await.unwrap().len(), 3);
    }
}
