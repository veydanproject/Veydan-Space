// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from crates/messenger/store/src/group_marks.rs at 1f2ef31d, then changed for channels.

//! `cm_marks`: the decisions of moderators in channels, as they came
//! (`cmod.hide`, `cmod.hide_author`, `cmod.unhide`, `cmod.approve`,
//! `cmod.reject`). A mark is kept
//! whether or not it is in force: `active` is decided by the journal as
//! it stands now and rewritten whenever the journal changes, so that a
//! moderator who was demoted loses the force of every mark, and one who
//! was raised gains it, on every device alike. A note that names several
//! messages is one row per message, each with its own `active` (the
//! author of each target has a role of their own). Nothing is deleted:
//! a mark that a later one overrides still tells the log what happened.

use super::{storage, Db};
use crate::moderation::MarkStore;
use async_trait::async_trait;
use messenger_core::Result;

pub use crate::marks::{MarkRow, KIND_APPROVE, KIND_HIDE, KIND_REJECT, KIND_UNHIDE};

const COLS: &str = "note_id, community_id, kind, target_id, author, from_at, to_at, version, reason, by, at, active";

/// Keep a mark; `false` when this row (note and target) is here already.
pub async fn insert_if_new(db: &Db, m: &MarkRow) -> Result<bool> {
    let res = sqlx::query(
        "INSERT OR IGNORE INTO cm_marks (note_id, community_id, kind, target_id, author, from_at, to_at, version, reason, by, at, active)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&m.note_id)
    .bind(&m.community_id)
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
    .execute(db.pool())
    .await
    .map_err(storage)?;
    Ok(res.rows_affected() == 1)
}

/// Every mark of a channel, newest first (by the moderators' clocks).
pub async fn by_channel(db: &Db, community_id: &str) -> Result<Vec<MarkRow>> {
    sqlx::query_as::<_, MarkRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM cm_marks WHERE community_id = ? ORDER BY at DESC, note_id ASC, target_id ASC"
    )))
    .bind(community_id)
    .fetch_all(db.pool())
    .await
    .map_err(storage)
}

/// The marks about one message.
pub async fn by_target(db: &Db, community_id: &str, target_id: &str) -> Result<Vec<MarkRow>> {
    sqlx::query_as::<_, MarkRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM cm_marks WHERE community_id = ? AND target_id = ? ORDER BY at DESC, note_id ASC"
    )))
    .bind(community_id)
    .bind(target_id)
    .fetch_all(db.pool())
    .await
    .map_err(storage)
}

/// The marks about a span of an author's messages that `created_at`
/// falls into.
pub async fn by_author_range(db: &Db, community_id: &str, author: &str, created_at: i64) -> Result<Vec<MarkRow>> {
    sqlx::query_as::<_, MarkRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM cm_marks
         WHERE community_id = ? AND target_id = '' AND author = ? AND from_at <= ? AND to_at >= ?
         ORDER BY at DESC, note_id ASC"
    )))
    .bind(community_id)
    .bind(author)
    .bind(created_at)
    .bind(created_at)
    .fetch_all(db.pool())
    .await
    .map_err(storage)
}

/// The marks one moderator made in a channel.
pub async fn by_author_of_marks(db: &Db, community_id: &str, by: &str) -> Result<Vec<MarkRow>> {
    sqlx::query_as::<_, MarkRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM cm_marks WHERE community_id = ? AND by = ? ORDER BY at DESC, note_id ASC, target_id ASC"
    )))
    .bind(community_id)
    .bind(by)
    .fetch_all(db.pool())
    .await
    .map_err(storage)
}

/// Rewrite whether marks are in force: `(note_id, target_id, active)`
/// for each. Rows not named stay as they are.
pub async fn set_active_bulk(db: &Db, community_id: &str, changes: &[(String, String, bool)]) -> Result<()> {
    for (note_id, target_id, active) in changes {
        sqlx::query("UPDATE cm_marks SET active = ? WHERE community_id = ? AND note_id = ? AND target_id = ?")
            .bind(active)
            .bind(community_id)
            .bind(note_id)
            .bind(target_id)
            .execute(db.pool())
            .await
            .map_err(storage)?;
    }
    Ok(())
}

/// The marks of moderation as `moderation` asks for them.
#[async_trait]
impl MarkStore for Db {
    async fn insert_mark(&self, mark: &MarkRow) -> Result<bool> {
        insert_if_new(self, mark).await
    }
    async fn marks_of(&self, community_id: &str) -> Result<Vec<MarkRow>> {
        by_channel(self, community_id).await
    }
    async fn marks_about(&self, community_id: &str, target_id: &str) -> Result<Vec<MarkRow>> {
        by_target(self, community_id, target_id).await
    }
    async fn marks_spanning(&self, community_id: &str, author: &str, created_at: i64) -> Result<Vec<MarkRow>> {
        by_author_range(self, community_id, author, created_at).await
    }
    async fn set_marks_active(&self, community_id: &str, changes: &[(String, String, bool)]) -> Result<()> {
        set_active_bulk(self, community_id, changes).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(note: &str, target: &str, kind: &str, at: i64) -> MarkRow {
        MarkRow {
            note_id: note.into(),
            community_id: "g".into(),
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
        let s = Db::open_in_memory().await.unwrap();
        assert!(insert_if_new(&s, &mark("n1", "a", KIND_HIDE, 10)).await.unwrap());
        assert!(insert_if_new(&s, &mark("n1", "b", KIND_HIDE, 10)).await.unwrap(), "one note, two targets");
        assert!(!insert_if_new(&s, &mark("n1", "a", KIND_HIDE, 10)).await.unwrap(), "the same again");
        let span = MarkRow { author: Some("x".into()), from_at: Some(100), to_at: Some(200), ..mark("n2", "", KIND_HIDE, 20) };
        assert!(insert_if_new(&s, &span).await.unwrap());
        assert_eq!(by_target(&s, "g", "a").await.unwrap().len(), 1);
        assert_eq!(by_channel(&s, "g").await.unwrap().iter().map(|m| m.note_id.as_str()).collect::<Vec<_>>(), vec!["n2", "n1", "n1"]);
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
