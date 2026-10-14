// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use super::files::*;
use super::models::*;
use super::tags::*;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::Emitter;
use veydan_core::{AppError, Core};

// ── FTS helpers ───────────────────────────────────────────────────────────────

pub(crate) async fn fts_upsert(
    note_id: &str,
    title: &str,
    content: &str,
    tags: &[String],
    old_fts_rowid: Option<i64>,
    db: &sqlx::Pool<sqlx::Sqlite>,
) -> Result<i64, AppError> {
    let mut conn = db.acquire().await.map_err(AppError::db)?;

    if let Some(rowid) = old_fts_rowid {
        sqlx::query("DELETE FROM notes_fts WHERE rowid = ?")
            .bind(rowid)
            .execute(&mut *conn)
            .await
            .map_err(AppError::db)?;
    }
    sqlx::query("INSERT INTO notes_fts(note_id, title, content, tags) VALUES (?, ?, ?, ?)")
        .bind(note_id)
        .bind(title)
        .bind(content)
        .bind(tags.join(" "))
        .execute(&mut *conn)
        .await
        .map_err(AppError::db)?;

    let (rowid,): (i64,) = sqlx::query_as("SELECT last_insert_rowid()")
        .fetch_one(&mut *conn)
        .await
        .map_err(AppError::db)?;

    Ok(rowid)
}

pub(crate) async fn fts_delete(
    fts_rowid: Option<i64>,
    db: &sqlx::Pool<sqlx::Sqlite>,
) -> Result<(), AppError> {
    if let Some(rowid) = fts_rowid {
        sqlx::query("DELETE FROM notes_fts WHERE rowid = ?")
            .bind(rowid)
            .execute(db)
            .await
            .map_err(AppError::db)?;
    }
    Ok(())
}

// ── Manifest ──────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
pub(crate) struct ManifestEntry {
    id: String,
    title: String,
    file_path: String,
    format: String,
    bindings: Vec<String>,
    tags: Vec<String>,
    created_at: String,
    updated_at: String,
    content_hash: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Manifest {
    version: u32,
    updated_at: String,
    notes: Vec<ManifestEntry>,
}

pub(crate) async fn rebuild_manifest(
    db: &sqlx::Pool<sqlx::Sqlite>,
    app_data_dir: &Path,
) -> Result<(), AppError> {
    let rows = sqlx::query_as::<_, NoteRow>(
        "SELECT * FROM notes WHERE deleted = 0 ORDER BY updated_at DESC",
    )
    .fetch_all(db)
    .await
    .map_err(AppError::db)?;

    let tags_map = fetch_all_note_tags_map(db).await?;

    let entries: Vec<ManifestEntry> = rows
        .into_iter()
        .map(|r| {
            let tags = tags_map
                .get(&r.id)
                .map(|v| v.iter().map(|t| t.name.clone()).collect())
                .unwrap_or_default();
            let bindings: Vec<String> = serde_json::from_str(&r.bindings).unwrap_or_default();
            ManifestEntry {
                id: r.id,
                title: r.title,
                file_path: r.file_path,
                format: r.format,
                bindings,
                tags,
                created_at: r.created_at,
                updated_at: r.updated_at,
                content_hash: r.content_hash,
            }
        })
        .collect();

    let manifest = Manifest {
        version: 1,
        updated_at: Utc::now().to_rfc3339(),
        notes: entries,
    };

    let json = serde_json::to_string_pretty(&manifest).map_err(AppError::other)?;
    atomic_write(&manifest_path(app_data_dir), &json)?;
    Ok(())
}

// ── Sync ──────────────────────────────────────────────────────────────────────

/// Bindings of a note file as a JSON array: its `bindings` field, or, in a
/// file that has none, its `workspace_id` / `profile_id` fields. Such a file
/// reaches this device as a note blob of a sync vault.
fn frontmatter_bindings(kv: &HashMap<String, String>) -> String {
    if let Some(bindings) = kv.get("bindings") {
        return bindings.clone();
    }
    let mut bindings: Vec<String> = Vec::new();
    if let Some(ws) = kv.get("workspace_id").filter(|v| *v != "null") {
        bindings.push(format!("workspace:{ws}"));
    }
    if let Some(pr) = kv.get("profile_id").filter(|v| *v != "null") {
        bindings.push(format!("profile:{pr}"));
    }
    serde_json::to_string(&bindings).unwrap_or_else(|_| "[]".to_string())
}

/// Bring the rows of the notes in line with their files: a changed file
/// gives its row the title it holds, and the label follows; a new file gets a
/// row. Then the search index and the manifest are built anew.
pub async fn sync_notes_index(
    core: &Core,
    custom_docs_dir: Option<&PathBuf>,
) -> Result<(), AppError> {
    let (db, app_data_dir) = (&core.db, core.app_data_dir.as_path());
    let docs_dir = effective_docs_dir(app_data_dir, custom_docs_dir);
    if !docs_dir.exists() {
        return Ok(());
    }

    // An older index took `..md` and `...md` as the notes `.` and `..`: their
    // paths are the folders of attachments and of documents. Their rows go,
    // and no file is touched for them.
    let bad: Vec<(String, Option<i64>)> =
        sqlx::query_as("SELECT id, fts_rowid FROM notes WHERE id IN ('', '.', '..')")
            .fetch_all(db)
            .await
            .map_err(AppError::db)?;
    for (id, fts_rowid) in &bad {
        super::crud::delete_note_rows(id, *fts_rowid, core).await?;
    }

    let existing: Vec<NoteRow> = sqlx::query_as::<_, NoteRow>("SELECT * FROM notes")
        .fetch_all(db)
        .await
        .map_err(AppError::db)?;

    let mut indexed: HashMap<String, NoteRow> =
        existing.into_iter().map(|r| (r.id.clone(), r)).collect();

    let now = Utc::now().to_rfc3339();

    // Scan files
    let entries = std::fs::read_dir(&docs_dir).map_err(AppError::io)?;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let note_id = stem.to_string();
        // Only a note file named by a note id becomes a note: not a csv or
        // json of the folder, and no name that would turn into a path. A
        // file an older index took keeps its row while it is that row's file.
        let indexed_before = indexed
            .get(&note_id)
            .is_some_and(|r| resolve_note_abs_path(app_data_dir, &r.file_path) == path);
        if !indexed_before && !(is_note_ext(ext) && valid_id(stem)) {
            continue;
        }

        let meta = std::fs::metadata(&path).ok();
        let mtime = meta
            .and_then(|m| m.modified().ok())
            .map(|t| chrono::DateTime::<Utc>::from(t).to_rfc3339());

        if let Some(row) = indexed.remove(&note_id) {
            // The row may name another file: while that one is there it is
            // the note's, and this file of the same name is a stranger that
            // changes nothing; when it is gone, the note moved here.
            let row_path = resolve_note_abs_path(app_data_dir, &row.file_path);
            let moved = row_path != path;
            if moved && row_path.exists() {
                continue;
            }
            if moved {
                sqlx::query("UPDATE notes SET file_path=? WHERE id=?")
                    .bind(path.to_string_lossy().to_string())
                    .bind(&note_id)
                    .execute(db)
                    .await
                    .map_err(AppError::db)?;
            }
            // File exists, check if changed
            let file_changed = moved || mtime.as_deref() != row.file_mtime.as_deref();
            if file_changed {
                if let Ok((kv, _, _)) = read_note_file(&path) {
                    let title = kv
                        .get("title")
                        .cloned()
                        .unwrap_or_else(|| row.title.clone());
                    let updated_at = kv.get("updated_at").cloned().unwrap_or_else(|| now.clone());
                    let content_str = std::fs::read_to_string(&path).ok();
                    let content_hash = content_str.as_deref().map(compute_hash);
                    let preview = content_str.as_deref().map(make_preview).unwrap_or_default();

                    let mut tx = db.begin().await.map_err(AppError::db)?;
                    sqlx::query(
                        "UPDATE notes SET title=?, updated_at=?, file_mtime=?, content_hash=?, preview=?, doc_status='active' WHERE id=?",
                    )
                    .bind(&title)
                    .bind(&updated_at)
                    .bind(&mtime)
                    .bind(&content_hash)
                    .bind(&preview)
                    .bind(&note_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(AppError::db)?;
                    super::directory::relabel(core, &mut tx, &note_id).await?;
                    tx.commit().await.map_err(AppError::db)?;
                }
            }
        } else {
            // New file not in DB — read frontmatter and insert
            let Ok((kv, tags, body)) = read_note_file(&path) else {
                continue;
            };
            let title = kv.get("title").cloned().unwrap_or_else(|| note_id.clone());
            let format = ext.to_string();
            let created_at = kv.get("created_at").cloned().unwrap_or_else(|| now.clone());
            let updated_at = kv.get("updated_at").cloned().unwrap_or_else(|| now.clone());
            let file_path = path.to_string_lossy().to_string();
            let content_hash = compute_hash(&body);
            let preview = make_preview(&body);

            let bindings_json = frontmatter_bindings(&kv);

            let mut tx = db.begin().await.map_err(AppError::db)?;
            sqlx::query(
                "INSERT OR IGNORE INTO notes
                 (id, title, file_path, format, bindings, created_at, updated_at, file_mtime, content_hash, preview, doc_status)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'active')",
            )
            .bind(&note_id)
            .bind(&title)
            .bind(&file_path)
            .bind(&format)
            .bind(&bindings_json)
            .bind(&created_at)
            .bind(&updated_at)
            .bind(&mtime)
            .bind(&content_hash)
            .bind(&preview)
            .execute(&mut *tx)
            .await
            .map_err(AppError::db)?;
            super::directory::relabel(core, &mut tx, &note_id).await?;
            tx.commit().await.map_err(AppError::db)?;

            // Insert tags
            set_note_tag_links(&note_id, &tags, db).await?;
        }
    }

    // Remaining in indexed = files missing from disk
    // (a row whose file stands where it names, in the scanned folder or not,
    // is not missing)
    for (id, row) in &indexed {
        if resolve_note_abs_path(app_data_dir, &row.file_path).exists() {
            continue;
        }
        sqlx::query("UPDATE notes SET doc_status='missing' WHERE id=?")
            .bind(id)
            .execute(db)
            .await
            .map_err(AppError::db)?;
    }

    // Rebuild FTS
    sqlx::query("DELETE FROM notes_fts")
        .execute(db)
        .await
        .map_err(AppError::db)?;
    // The rows of deleted notes keep no rowid of a table that was emptied: a
    // rowid taken again by a live note would be dropped with them.
    sqlx::query("UPDATE notes SET fts_rowid = NULL WHERE deleted = 1")
        .execute(db)
        .await
        .map_err(AppError::db)?;

    let all_notes = sqlx::query_as::<_, NoteRow>("SELECT * FROM notes WHERE deleted = 0")
        .fetch_all(db)
        .await
        .map_err(AppError::db)?;

    for row in &all_notes {
        let file_path = resolve_note_abs_path(app_data_dir, &row.file_path);
        let (_, tags_list, body) = read_note_file(&file_path).unwrap_or_default();
        let tags_str = tags_list.join(" ");
        super::links::reindex_links(&row.id, &body, db).await?;

        let mut conn = db.acquire().await.map_err(AppError::db)?;
        sqlx::query("INSERT INTO notes_fts(note_id, title, content, tags) VALUES (?, ?, ?, ?)")
            .bind(&row.id)
            .bind(&row.title)
            .bind(&body)
            .bind(&tags_str)
            .execute(&mut *conn)
            .await
            .map_err(AppError::db)?;

        let (rowid,): (i64,) = sqlx::query_as("SELECT last_insert_rowid()")
            .fetch_one(&mut *conn)
            .await
            .map_err(AppError::db)?;

        sqlx::query("UPDATE notes SET fts_rowid=? WHERE id=?")
            .bind(rowid)
            .bind(&row.id)
            .execute(db)
            .await
            .map_err(AppError::db)?;
    }

    rebuild_manifest(db, app_data_dir).await?;
    Ok(())
}

// ── File watcher ──────────────────────────────────────────────────────────────

/// Starts watching the notes documents dir. The returned watcher owns the
/// directory handle; dropping it stops the watch and ends the event thread.
pub fn start_notes_watcher(
    app_handle: tauri::AppHandle,
    app_data_dir: PathBuf,
    custom_dir: Option<PathBuf>,
) -> Option<notify::RecommendedWatcher> {
    use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

    let (tx, rx) = std::sync::mpsc::channel::<notify::Result<Event>>();
    let docs_dir = effective_docs_dir(&app_data_dir, custom_dir.as_ref());

    let Ok(mut watcher) = RecommendedWatcher::new(tx, Config::default()) else {
        return None;
    };

    if docs_dir.exists() {
        let _ = watcher.watch(&docs_dir, RecursiveMode::NonRecursive);
    }

    std::thread::spawn(move || {
        for event in rx.into_iter().flatten() {
            if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                for path in &event.paths {
                    // Skip tmp files generated by our own atomic write
                    if path.extension().and_then(|e| e.to_str()) == Some("tmp") {
                        continue;
                    }
                    if let Some(stem) = path.file_stem() {
                        let note_id = stem.to_string_lossy().to_string();
                        let _ = app_handle.emit("notes://external-change", &note_id);
                    }
                }
            }
        }
    });
    Some(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::Manager;

    #[test]
    fn bindings_are_read_from_either_form_of_a_note_file() {
        let (kv, tags, body) = parse_note_file(
            "---\nid: n1\ntitle: Deploy\nformat: md\nbindings: [\"workspace:w1\",\"profile:p1\"]\ntags:\n  - runbook\ncreated_at: 2026-09-01T10:00:00+00:00\nupdated_at: 2026-09-02T11:30:00+00:00\n---\n\nbody",
        );
        assert_eq!(
            frontmatter_bindings(&kv),
            "[\"workspace:w1\",\"profile:p1\"]"
        );
        assert_eq!(tags, vec!["runbook"]);
        assert_eq!(body, "body");

        // A file a 4.0.7 device never saved again still says where it belongs this way.
        let (kv, _, _) = parse_note_file(
            "---\nid: n2\ntitle: Old\nformat: md\nscope: profile\nworkspace_id: w1\nprofile_id: p1\ntags:\n  []\ncreated_at: 2026-01-01T00:00:00+00:00\nupdated_at: 2026-01-01T00:00:00+00:00\n---\n\nbody",
        );
        assert_eq!(
            frontmatter_bindings(&kv),
            "[\"workspace:w1\",\"profile:p1\"]"
        );

        let (kv, _, _) = parse_note_file(
            "---\nid: n3\ntitle: Global\nformat: md\nscope: global\nworkspace_id: null\nprofile_id: null\n---\n\nbody",
        );
        assert_eq!(frontmatter_bindings(&kv), "[]");
    }

    async fn ids(db: &sqlx::Pool<sqlx::Sqlite>) -> Vec<String> {
        sqlx::query_scalar("SELECT id FROM notes ORDER BY id")
            .fetch_all(db)
            .await
            .unwrap()
    }

    /// `..md` and `...md` have the stems `.` and `..`, a path and not an id;
    /// a csv of the folder is not a note. None of them gets a row.
    #[tokio::test]
    async fn a_file_not_named_by_a_note_id_gets_no_row() {
        let (app, dir) = crate::testing::app().await;
        let core = app.state::<Core>();
        let docs = documents_dir(dir.path());
        for name in ["..md", "...md", "readme.csv", "My note.md"] {
            std::fs::write(docs.join(name), "---\ntitle: x\n---\n\nbody").unwrap();
        }
        std::fs::write(docs.join("n-1.md"), "---\ntitle: One\n---\n\nbody").unwrap();

        sync_notes_index(&core, None).await.unwrap();

        assert_eq!(ids(&core.db).await, ["n-1"]);
    }

    /// The rows `.` and `..` an older index made go without a file touched;
    /// a row with another name the rule would refuse now stays, active.
    #[tokio::test]
    async fn the_rows_dot_and_dot_dot_go_and_an_older_row_stays() {
        let (app, dir) = crate::testing::app().await;
        let core = app.state::<Core>();
        let docs = documents_dir(dir.path());
        std::fs::write(docs.join("...md"), "body").unwrap();
        std::fs::write(docs.join("readme.csv"), "a,b").unwrap();
        std::fs::create_dir_all(docs.join("attachments/n-2")).unwrap();
        std::fs::write(docs.join("attachments/n-2/a.png"), "png").unwrap();
        for (id, file) in [("..", "...md"), (".", "..md"), ("readme", "readme.csv")] {
            sqlx::query(
                "INSERT INTO notes (id, title, file_path, created_at, updated_at)
                 VALUES (?, ?, ?, '2026-01-01', '2026-01-01')",
            )
            .bind(id)
            .bind(id)
            .bind(docs.join(file).to_string_lossy().to_string())
            .execute(&core.db)
            .await
            .unwrap();
        }

        sync_notes_index(&core, None).await.unwrap();

        assert_eq!(ids(&core.db).await, ["readme"]);
        let status: String = sqlx::query_scalar("SELECT doc_status FROM notes WHERE id = 'readme'")
            .fetch_one(&core.db)
            .await
            .unwrap();
        assert_eq!(status, "active");
        assert!(docs.join("...md").exists());
        assert!(docs.join("readme.csv").exists());
        assert!(docs.join("attachments/n-2/a.png").exists());
    }

    async fn row_of(db: &sqlx::Pool<sqlx::Sqlite>, id: &str) -> (String, String, String) {
        sqlx::query_as("SELECT title, file_path, doc_status FROM notes WHERE id = ?")
            .bind(id)
            .fetch_one(db)
            .await
            .unwrap()
    }

    async fn insert_row(
        db: &sqlx::Pool<sqlx::Sqlite>,
        id: &str,
        title: &str,
        file: &std::path::Path,
    ) {
        sqlx::query(
            "INSERT INTO notes (id, title, file_path, created_at, updated_at)
             VALUES (?, ?, ?, '2026-01-01', '2026-01-01')",
        )
        .bind(id)
        .bind(title)
        .bind(file.to_string_lossy().to_string())
        .execute(db)
        .await
        .unwrap();
    }

    /// A row whose file stands elsewhere by an absolute path is not missing,
    /// and a file of the same name in the folder does not change its title.
    #[tokio::test]
    async fn a_file_elsewhere_is_not_missing_and_a_stranger_changes_nothing() {
        let (app, dir) = crate::testing::app().await;
        let core = app.state::<Core>();
        let docs = documents_dir(dir.path());
        let other = dir.path().join("elsewhere");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("n-1.md"), "---\ntitle: Mine\n---\n\nbody").unwrap();
        std::fs::write(other.join("n-2.md"), "---\ntitle: Solo\n---\n\nbody").unwrap();
        insert_row(&core.db, "n-1", "Mine", &other.join("n-1.md")).await;
        insert_row(&core.db, "n-2", "Solo", &other.join("n-2.md")).await;
        std::fs::write(docs.join("n-1.md"), "---\ntitle: Stranger\n---\n\nother").unwrap();

        sync_notes_index(&core, None).await.unwrap();

        let (title, path, status) = row_of(&core.db, "n-1").await;
        assert_eq!(title, "Mine");
        assert_eq!(path, other.join("n-1.md").to_string_lossy());
        assert_eq!(status, "active");
        assert_eq!(row_of(&core.db, "n-2").await.2, "active");
    }

    /// A note moved by hand into the folder keeps its row and gets the new
    /// path; a note whose file is nowhere is missing.
    #[tokio::test]
    async fn a_moved_file_updates_its_path_and_a_lost_one_is_missing() {
        let (app, dir) = crate::testing::app().await;
        let core = app.state::<Core>();
        let docs = documents_dir(dir.path());
        let old = dir.path().join("old");
        insert_row(&core.db, "n-1", "Moved", &old.join("n-1.md")).await;
        insert_row(&core.db, "n-2", "Lost", &old.join("n-2.md")).await;
        std::fs::write(docs.join("n-1.md"), "---\ntitle: Moved\n---\n\nbody").unwrap();

        sync_notes_index(&core, None).await.unwrap();

        let (_, path, status) = row_of(&core.db, "n-1").await;
        assert_eq!(path, docs.join("n-1.md").to_string_lossy());
        assert_eq!(status, "active");
        assert_eq!(row_of(&core.db, "n-2").await.2, "missing");
    }

    /// The hash of a new file is that of its body, as every save computes it.
    #[tokio::test]
    async fn the_hash_of_a_new_file_is_the_hash_of_its_body() {
        let (app, dir) = crate::testing::app().await;
        let core = app.state::<Core>();
        let docs = documents_dir(dir.path());
        std::fs::write(docs.join("n-1.md"), "---\ntitle: One\n---\n\nthe body").unwrap();

        sync_notes_index(&core, None).await.unwrap();

        let hash: String = sqlx::query_scalar("SELECT content_hash FROM notes WHERE id='n-1'")
            .fetch_one(&core.db)
            .await
            .unwrap();
        assert_eq!(hash, compute_hash("the body"));
    }
}
