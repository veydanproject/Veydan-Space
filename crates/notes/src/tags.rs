// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use super::models::*;
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;
use veydan_core::Core;
use veydan_core::{AppError, CmdResult};

// ── Tag helpers ───────────────────────────────────────────────────────────────

/// Color of a tag created by name only; a chosen color always wins over it.
pub const TAG_PLACEHOLDER_COLOR: &str = "#6366f1";

/// A tag name as it is stored: trimmed and lowercase, so "Работа" and
/// "работа" are one tag. Rust's lowercase, since SQLite's `lower()` and
/// `NOCASE` know ASCII only.
pub(crate) fn normalize_tag_name(name: &str) -> String {
    name.trim().to_lowercase()
}

/// The tags of a note as they are stored and written to its file: each
/// name normalized, once, empty ones dropped, in the order given.
pub(crate) fn normalize_tag_names(names: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(names.len());
    for name in names {
        let name = normalize_tag_name(name);
        if !name.is_empty() && !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

pub(crate) async fn fetch_note_tags(
    note_id: &str,
    db: &sqlx::Pool<sqlx::Sqlite>,
) -> Result<Vec<NoteTagInfo>, AppError> {
    sqlx::query_as::<_, NoteTagInfo>(
        "SELECT nt.id, nt.name, nt.color FROM note_tags nt
         JOIN note_tag_links ntl ON nt.id = ntl.tag_id
         WHERE ntl.note_id = ?
         ORDER BY nt.name",
    )
    .bind(note_id)
    .fetch_all(db)
    .await
    .map_err(AppError::db)
}

pub(crate) async fn fetch_all_note_tags_map(
    db: &sqlx::Pool<sqlx::Sqlite>,
) -> Result<HashMap<String, Vec<NoteTagInfo>>, AppError> {
    let rows = sqlx::query_as::<_, (String, String, String, String)>(
        "SELECT ntl.note_id, nt.id, nt.name, nt.color
         FROM note_tag_links ntl JOIN note_tags nt ON ntl.tag_id = nt.id",
    )
    .fetch_all(db)
    .await
    .map_err(AppError::db)?;

    let mut map: HashMap<String, Vec<NoteTagInfo>> = HashMap::new();
    for (note_id, tag_id, tag_name, tag_color) in rows {
        map.entry(note_id).or_default().push(NoteTagInfo {
            id: tag_id,
            name: tag_name,
            color: tag_color,
        });
    }
    Ok(map)
}

pub(crate) async fn fetch_note_folder_ids(
    note_id: &str,
    db: &sqlx::Pool<sqlx::Sqlite>,
) -> Result<Vec<String>, AppError> {
    sqlx::query_scalar::<_, String>("SELECT folder_id FROM note_folder_links WHERE note_id = ?")
        .bind(note_id)
        .fetch_all(db)
        .await
        .map_err(AppError::db)
}

pub(crate) async fn fetch_all_note_folder_ids_map(
    db: &sqlx::Pool<sqlx::Sqlite>,
) -> Result<HashMap<String, Vec<String>>, AppError> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT note_id, folder_id FROM note_folder_links")
            .fetch_all(db)
            .await
            .map_err(AppError::db)?;

    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for (note_id, folder_id) in rows {
        map.entry(note_id).or_default().push(folder_id);
    }
    Ok(map)
}

/// Find or create a tag by name, return its id.
pub(crate) async fn upsert_tag(
    name: &str,
    db: &sqlx::Pool<sqlx::Sqlite>,
) -> Result<String, AppError> {
    let name = normalize_tag_name(name);
    if name.is_empty() {
        return Err(AppError::io("Tag name cannot be empty"));
    }
    let now = Utc::now().to_rfc3339();
    let existing: Option<(String,)> = sqlx::query_as("SELECT id FROM note_tags WHERE name = ?")
        .bind(&name)
        .fetch_optional(db)
        .await
        .map_err(AppError::db)?;

    if let Some((id,)) = existing {
        return Ok(id);
    }

    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO note_tags (id, name, color, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&name)
    .bind(TAG_PLACEHOLDER_COLOR)
    .bind(&now)
    .bind(&now)
    .execute(db)
    .await
    .map_err(AppError::db)?;

    Ok(id)
}

/// Link the note to these tags, created when missing. Returns the names as
/// they are stored, for the note's file and its index.
pub(crate) async fn set_note_tag_links(
    note_id: &str,
    tag_names: &[String],
    db: &sqlx::Pool<sqlx::Sqlite>,
) -> Result<Vec<String>, AppError> {
    let tag_names = normalize_tag_names(tag_names);
    sqlx::query("DELETE FROM note_tag_links WHERE note_id = ?")
        .bind(note_id)
        .execute(db)
        .await
        .map_err(AppError::db)?;

    for name in &tag_names {
        let tag_id = upsert_tag(name, db).await?;
        sqlx::query("INSERT OR IGNORE INTO note_tag_links (note_id, tag_id) VALUES (?, ?)")
            .bind(note_id)
            .bind(&tag_id)
            .execute(db)
            .await
            .map_err(AppError::db)?;
    }
    Ok(tag_names)
}

/// Tags stored before every path lowercased the name (5.1.12): a name with
/// capitals goes into its lowercase twin, links and all, or is renamed when
/// there is none. The twin keeps its color unless it has only the
/// placeholder. Plain deletes and updates, as `note_tag_delete` makes them:
/// the next sync cycle publishes the tombstone and the new name. Returns how
/// many tags changed.
pub(crate) async fn merge_tag_case_twins(
    db: &sqlx::Pool<sqlx::Sqlite>,
) -> Result<usize, AppError> {
    let rows: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id, name, color FROM note_tags ORDER BY created_at, id")
            .fetch_all(db)
            .await
            .map_err(AppError::db)?;
    let mut changed = 0;
    for (id, name, color) in rows {
        let lower = normalize_tag_name(&name);
        if lower == name || lower.is_empty() {
            continue;
        }
        let now = Utc::now().to_rfc3339();
        let mut tx = db.begin().await.map_err(AppError::db)?;
        let twin: Option<(String, String)> =
            sqlx::query_as("SELECT id, color FROM note_tags WHERE name = ?")
                .bind(&lower)
                .fetch_optional(&mut *tx)
                .await
                .map_err(AppError::db)?;
        match twin {
            Some((twin_id, twin_color)) => {
                sqlx::query(
                    "INSERT OR IGNORE INTO note_tag_links (note_id, tag_id)
                     SELECT note_id, ? FROM note_tag_links WHERE tag_id = ?",
                )
                .bind(&twin_id)
                .bind(&id)
                .execute(&mut *tx)
                .await
                .map_err(AppError::db)?;
                if twin_color == TAG_PLACEHOLDER_COLOR && color != TAG_PLACEHOLDER_COLOR {
                    sqlx::query("UPDATE note_tags SET color = ?, updated_at = ? WHERE id = ?")
                        .bind(&color)
                        .bind(&now)
                        .bind(&twin_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(AppError::db)?;
                }
                sqlx::query("DELETE FROM note_tag_links WHERE tag_id = ?")
                    .bind(&id)
                    .execute(&mut *tx)
                    .await
                    .map_err(AppError::db)?;
                sqlx::query("DELETE FROM note_tags WHERE id = ?")
                    .bind(&id)
                    .execute(&mut *tx)
                    .await
                    .map_err(AppError::db)?;
            }
            None => {
                sqlx::query("UPDATE note_tags SET name = ?, updated_at = ? WHERE id = ?")
                    .bind(&lower)
                    .bind(&now)
                    .bind(&id)
                    .execute(&mut *tx)
                    .await
                    .map_err(AppError::db)?;
            }
        }
        tx.commit().await.map_err(AppError::db)?;
        changed += 1;
    }
    Ok(changed)
}

#[tauri::command]
pub async fn note_tag_list(core: tauri::State<'_, Core>) -> CmdResult<Vec<NoteTag>> {
    sqlx::query_as::<_, NoteTag>(
        "SELECT id, name, color, created_at, updated_at FROM note_tags ORDER BY name",
    )
    .fetch_all(&core.db)
    .await
    .map_err(AppError::db)
}

#[tauri::command]
pub async fn note_tag_create(
    name: String,
    color: Option<String>,
    core: tauri::State<'_, Core>,
) -> CmdResult<NoteTag> {
    let name = normalize_tag_name(&name);
    if name.is_empty() {
        return Err(AppError::io("Tag name cannot be empty"));
    }
    let color = color.unwrap_or_else(|| TAG_PLACEHOLDER_COLOR.to_string());
    let now = Utc::now().to_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();

    // Only a chosen color replaces the stored one; the placeholder keeps what is there.
    sqlx::query(
        "INSERT INTO note_tags (id, name, color, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(name) DO UPDATE SET
           color = CASE WHEN excluded.color = ? THEN note_tags.color ELSE excluded.color END,
           updated_at = excluded.updated_at",
    )
    .bind(&id)
    .bind(&name)
    .bind(&color)
    .bind(&now)
    .bind(&now)
    .bind(TAG_PLACEHOLDER_COLOR)
    .execute(&core.db)
    .await
    .map_err(AppError::db)?;

    let tag = sqlx::query_as::<_, NoteTag>(
        "SELECT id, name, color, created_at, updated_at FROM note_tags WHERE name = ?",
    )
    .bind(&name)
    .fetch_one(&core.db)
    .await
    .map_err(AppError::db)?;

    Ok(tag)
}

#[tauri::command]
pub async fn note_tag_delete(id: String, core: tauri::State<'_, Core>) -> CmdResult<()> {
    sqlx::query("DELETE FROM note_tag_links WHERE tag_id = ?")
        .bind(&id)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;
    sqlx::query("DELETE FROM note_tags WHERE id = ?")
        .bind(&id)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;
    Ok(())
}

#[tauri::command]
pub async fn note_tag_update(
    id: String,
    name: Option<String>,
    color: Option<String>,
    core: tauri::State<'_, Core>,
) -> CmdResult<NoteTag> {
    let now = Utc::now().to_rfc3339();
    let name = name
        .map(|n| normalize_tag_name(&n))
        .filter(|n| !n.is_empty());
    sqlx::query(
        "UPDATE note_tags SET
            name       = COALESCE(?, name),
            color      = COALESCE(?, color),
            updated_at = ?
         WHERE id = ?",
    )
    .bind(&name)
    .bind(&color)
    .bind(&now)
    .bind(&id)
    .execute(&core.db)
    .await
    .map_err(AppError::db)?;

    let tag = sqlx::query_as::<_, NoteTag>(
        "SELECT id, name, color, created_at, updated_at FROM note_tags WHERE id = ?",
    )
    .bind(&id)
    .fetch_one(&core.db)
    .await
    .map_err(AppError::db)?;

    Ok(tag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crud::{insert_note, note_set_tags, NewNote};
    use crate::files::{parse_note_file, resolve_note_abs_path};
    use crate::{testing, NotesState};
    use tauri::Manager;

    const RED: &str = "#f26d6d";

    async fn tags(db: &sqlx::Pool<sqlx::Sqlite>) -> Vec<(String, String, String)> {
        sqlx::query_as("SELECT id, name, color FROM note_tags ORDER BY name")
            .fetch_all(db)
            .await
            .unwrap()
    }

    async fn linked(note_id: &str, db: &sqlx::Pool<sqlx::Sqlite>) -> Vec<String> {
        fetch_note_tags(note_id, db)
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect()
    }

    async fn note(app: &tauri::App<tauri::test::MockRuntime>, id: &str, tags: &[&str]) {
        let now = Utc::now().to_rfc3339();
        insert_note(
            NewNote {
                id: id.into(),
                title: "Plan".into(),
                format: "md".into(),
                bindings: Vec::new(),
                tags: tags.iter().map(|t| t.to_string()).collect(),
                content: "Body".into(),
                created_at: now.clone(),
                updated_at: now,
            },
            &app.state::<Core>(),
            &app.state::<NotesState>(),
        )
        .await
        .unwrap();
    }

    async fn insert_tag(db: &sqlx::Pool<sqlx::Sqlite>, id: &str, name: &str, color: &str) {
        sqlx::query(
            "INSERT INTO note_tags (id, name, color, created_at, updated_at)
             VALUES (?, ?, ?, 't', 't')",
        )
        .bind(id)
        .bind(name)
        .bind(color)
        .execute(db)
        .await
        .unwrap();
    }

    #[test]
    fn a_name_is_trimmed_and_lowercase_and_keeps_its_hash() {
        assert_eq!(normalize_tag_name("  Работа "), "работа");
        assert_eq!(normalize_tag_name("#Inbox"), "#inbox");
        assert_eq!(
            normalize_tag_names(&["A".into(), " ".into(), "a".into(), "b".into()]),
            ["a", "b"]
        );
    }

    /// The picker creates the tag with its color, then sets the note's tags
    /// with the name as typed: one tag, with that color.
    #[tokio::test]
    async fn a_capitalized_name_is_one_tag_with_the_chosen_color() {
        let (app, _dir) = testing::app().await;
        let db = app.state::<Core>().db.clone();
        note(&app, "n-1", &[]).await;

        let created = note_tag_create("Работа".into(), Some(RED.into()), app.state())
            .await
            .unwrap();
        set_note_tag_links("n-1", &["Работа".into()], &db).await.unwrap();

        assert_eq!(
            tags(&db).await,
            [(created.id, "работа".to_string(), RED.to_string())]
        );
        assert_eq!(linked("n-1", &db).await, ["работа"]);
    }

    #[tokio::test]
    async fn a_name_in_capitals_finds_the_lowercase_tag() {
        let (app, _dir) = testing::app().await;
        let db = app.state::<Core>().db.clone();
        let id = upsert_tag("работа", &db).await.unwrap();

        assert_eq!(upsert_tag("РАБОТА", &db).await.unwrap(), id);
        assert_eq!(upsert_tag(" Работа ", &db).await.unwrap(), id);
        assert!(upsert_tag("  ", &db).await.is_err());
        assert_eq!(tags(&db).await.len(), 1);
    }

    #[tokio::test]
    async fn the_note_file_names_its_tags_in_lowercase() {
        let (app, _dir) = testing::app().await;
        let core = app.state::<Core>();
        note(&app, "n-1", &["Inbox", "inbox"]).await;
        let path: String = sqlx::query_scalar("SELECT file_path FROM notes WHERE id = 'n-1'")
            .fetch_one(&core.db)
            .await
            .unwrap();
        let path = resolve_note_abs_path(&core.app_data_dir, &path);
        let file = || parse_note_file(&std::fs::read_to_string(&path).unwrap()).1;
        assert_eq!(file(), ["inbox"]);

        note_set_tags(
            "n-1".into(),
            vec!["Работа".into(), "работа".into(), "Inbox".into()],
            app.state(),
        )
        .await
        .unwrap();

        assert_eq!(file(), ["работа", "inbox"]);
        assert_eq!(linked("n-1", &core.db).await, ["inbox", "работа"]);
    }

    /// A tag stored with capitals before 5.1.12 joins its lowercase twin:
    /// the links move, the chosen color stays.
    #[tokio::test]
    async fn a_capitalized_twin_is_merged_into_the_lowercase_tag() {
        let (app, _dir) = testing::app().await;
        let db = app.state::<Core>().db.clone();
        note(&app, "n-1", &[]).await;
        note(&app, "n-2", &[]).await;
        insert_tag(&db, "t-upper", "Работа", TAG_PLACEHOLDER_COLOR).await;
        insert_tag(&db, "t-lower", "работа", RED).await;
        sqlx::query(
            "INSERT INTO note_tag_links (note_id, tag_id) VALUES
             ('n-1', 't-upper'), ('n-2', 't-upper'), ('n-2', 't-lower')",
        )
        .execute(&db)
        .await
        .unwrap();

        assert_eq!(merge_tag_case_twins(&db).await.unwrap(), 1);

        assert_eq!(
            tags(&db).await,
            [("t-lower".to_string(), "работа".to_string(), RED.to_string())]
        );
        assert_eq!(linked("n-1", &db).await, ["работа"]);
        assert_eq!(linked("n-2", &db).await, ["работа"]);
        // Nothing is left to do the next time.
        assert_eq!(merge_tag_case_twins(&db).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn a_twin_with_the_placeholder_takes_the_chosen_color() {
        let (app, _dir) = testing::app().await;
        let db = app.state::<Core>().db.clone();
        insert_tag(&db, "t-upper", "Работа", RED).await;
        insert_tag(&db, "t-lower", "работа", TAG_PLACEHOLDER_COLOR).await;

        merge_tag_case_twins(&db).await.unwrap();

        assert_eq!(
            tags(&db).await,
            [("t-lower".to_string(), "работа".to_string(), RED.to_string())]
        );
    }

    #[tokio::test]
    async fn a_capitalized_tag_alone_is_renamed() {
        let (app, _dir) = testing::app().await;
        let db = app.state::<Core>().db.clone();
        note(&app, "n-1", &[]).await;
        insert_tag(&db, "t-1", "Работа", RED).await;
        insert_tag(&db, "t-2", "РАБОТА", TAG_PLACEHOLDER_COLOR).await;
        sqlx::query("INSERT INTO note_tag_links (note_id, tag_id) VALUES ('n-1', 't-2')")
            .execute(&db)
            .await
            .unwrap();

        assert_eq!(merge_tag_case_twins(&db).await.unwrap(), 2);

        let left = tags(&db).await;
        assert_eq!(left.len(), 1);
        assert_eq!((left[0].1.as_str(), left[0].2.as_str()), ("работа", RED));
        assert_eq!(linked("n-1", &db).await, ["работа"]);
    }
}
