// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use super::attachments::remove_note_attachments;
use super::files::*;
use super::filter::FilterContext;
use super::history::*;
use super::index::*;
use super::links::{propagate_rename, reindex_links};
use super::models::*;
use super::tags::*;
use super::templates::{render_template, TemplateVars};
use super::NotesState;
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;
use veydan_core::Core;
use veydan_core::{AppError, CmdResult};

// ── Tauri commands ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn note_list(
    filter: NoteFilter,
    core: tauri::State<'_, Core>,
) -> CmdResult<Vec<NoteListItem>> {
    let rows =
        sqlx::query_as::<_, NoteRow>("SELECT * FROM notes ORDER BY pinned DESC, updated_at DESC")
            .fetch_all(&core.db)
            .await
            .map_err(AppError::db)?;

    let ctx = FilterContext::load(&filter, &core).await?;
    Ok(list_items(rows, &filter, &ctx, &core.app_data_dir))
}

/// Apply the filter and build list items from rows.
fn list_items(
    rows: Vec<NoteRow>,
    filter: &NoteFilter,
    ctx: &FilterContext,
    app_data_dir: &std::path::Path,
) -> Vec<NoteListItem> {
    let drafts = drafts_dir(app_data_dir);
    rows.into_iter()
        .filter(|r| ctx.matches(r, filter))
        .map(|r| {
            let tags = ctx.tags.get(&r.id).cloned().unwrap_or_default();
            let folder_ids = ctx.folders.get(&r.id).cloned().unwrap_or_default();
            let has_draft = drafts.join(format!("{}.draft", r.id)).exists();
            row_to_list_item(r, tags, folder_ids, has_draft)
        })
        .collect()
}

#[tauri::command]
pub async fn note_get(id: String, core: tauri::State<'_, Core>) -> CmdResult<Note> {
    let row = sqlx::query_as::<_, NoteRow>("SELECT * FROM notes WHERE id = ?")
        .bind(&id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found(format!("Note {id}")))?;

    let tags = fetch_note_tags(&id, &core.db).await?;
    let folder_ids = fetch_note_folder_ids(&id, &core.db).await?;
    let file_path = resolve_note_abs_path(&core.app_data_dir, &row.file_path);

    // Hash the file body, not the indexed column: an external edit may not be reindexed yet.
    let (content, content_hash) = if file_path.exists() {
        let (_, _, body) = read_note_file(&file_path)?;
        let hash = compute_hash(&body);
        (Some(body), Some(hash))
    } else {
        (None, row.content_hash)
    };

    let has_draft = draft_file_path(&core.app_data_dir, &id).is_some_and(|p| p.exists());

    let bindings: Vec<String> = serde_json::from_str(&row.bindings).unwrap_or_default();

    Ok(Note {
        id: row.id,
        title: row.title,
        base_dir: note_base_dir(&core.app_data_dir, &row.file_path),
        file_path: row.file_path,
        format: row.format,
        bindings,
        tags,
        folder_ids,
        pinned: row.pinned != 0,
        archived: row.archived != 0,
        deleted: row.deleted != 0,
        doc_status: row.doc_status,
        created_at: row.created_at,
        updated_at: row.updated_at,
        content_hash,
        content,
        has_draft,
    })
}

/// Fully specified note to persist (used by create and import).
pub(crate) struct NewNote {
    pub id: String,
    pub title: String,
    pub format: String,
    pub bindings: Vec<String>,
    pub tags: Vec<String>,
    pub content: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Current documents directory (custom or default).
pub(crate) fn current_docs_dir(core: &Core, notes: &NotesState) -> std::path::PathBuf {
    let custom_dir = notes.custom_dir.read().ok().and_then(|g| g.clone());
    effective_docs_dir(&core.app_data_dir, custom_dir.as_ref())
}

/// Write the note file, insert the DB row, index FTS and refresh the manifest.
pub(crate) async fn insert_note(
    mut new: NewNote,
    core: &Core,
    notes: &NotesState,
) -> Result<Note, AppError> {
    // The file names the tags as the database stores them.
    new.tags = normalize_tag_names(&new.tags);
    let bindings_json = serde_json::to_string(&new.bindings).map_err(AppError::other)?;
    let docs_dir = current_docs_dir(core, notes);
    let abs_path = docs_dir.join(format!("{}.{}", new.id, new.format));
    // The id and the format make one file name in the documents folder, never a path.
    if abs_path.parent() != Some(docs_dir.as_path()) {
        return Err(AppError::other(format!(
            "note file {} is outside the documents folder",
            abs_path.display()
        )));
    }
    let stored_path = abs_path.to_string_lossy().to_string();

    let row = NoteRow {
        id: new.id.clone(),
        title: new.title.clone(),
        file_path: stored_path.clone(),
        format: new.format.clone(),
        pinned: 0,
        archived: 0,
        deleted: 0,
        doc_status: "active".to_string(),
        version_base: None,
        fts_rowid: None,
        created_at: new.created_at.clone(),
        updated_at: new.updated_at.clone(),
        file_mtime: None,
        content_hash: None,
        preview: String::new(),
        bindings: bindings_json.clone(),
    };

    write_note_file(&abs_path, &row, &new.tags, &new.content)?;

    let content_hash = compute_hash(&new.content);
    let preview = make_preview(&new.content);

    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    sqlx::query(
        "INSERT INTO notes (id, title, file_path, format, bindings, pinned, archived, deleted, doc_status, created_at, updated_at, content_hash, preview)
         VALUES (?, ?, ?, ?, ?, 0, 0, 0, 'active', ?, ?, ?, ?)",
    )
    .bind(&new.id)
    .bind(&new.title)
    .bind(&stored_path)
    .bind(&new.format)
    .bind(&bindings_json)
    .bind(&new.created_at)
    .bind(&new.updated_at)
    .bind(&content_hash)
    .bind(&preview)
    .execute(&mut *tx)
    .await
    .map_err(AppError::db)?;
    crate::directory::relabel(core, &mut tx, &new.id).await?;
    tx.commit().await.map_err(AppError::db)?;

    set_note_tag_links(&new.id, &new.tags, &core.db).await?;
    reindex_links(&new.id, &new.content, &core.db).await?;

    let fts_rowid =
        fts_upsert(&new.id, &new.title, &new.content, &new.tags, None, &core.db).await?;
    sqlx::query("UPDATE notes SET fts_rowid=? WHERE id=?")
        .bind(fts_rowid)
        .bind(&new.id)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;

    let tags = fetch_note_tags(&new.id, &core.db).await?;
    rebuild_manifest(&core.db, &core.app_data_dir).await?;

    Ok(Note {
        id: new.id,
        title: new.title,
        base_dir: note_base_dir(&core.app_data_dir, &stored_path),
        file_path: stored_path,
        format: new.format,
        bindings: new.bindings,
        tags,
        folder_ids: Vec::new(),
        pinned: false,
        archived: false,
        deleted: false,
        doc_status: "active".to_string(),
        created_at: new.created_at,
        updated_at: new.updated_at,
        content_hash: Some(content_hash),
        content: Some(new.content),
        has_draft: false,
    })
}

#[tauri::command]
pub async fn note_create(
    input: NoteCreateInput,
    core: tauri::State<'_, Core>,
    notes: tauri::State<'_, NotesState>,
) -> CmdResult<Note> {
    // The format becomes the extension of the file: a note extension only.
    let format = input.format.unwrap_or_else(|| "md".to_string());
    if !is_note_ext(&format) {
        return Err(AppError::other(format!("unknown note format {format:?}")));
    }
    let now = Utc::now().to_rfc3339();
    let bindings = input.bindings.unwrap_or_default();
    let mut content = input.content.unwrap_or_default();
    if let Some(template_id) = input.template_id.filter(|t| !t.is_empty()) {
        let vars = TemplateVars::from_bindings(&input.title, &bindings, &core.directory).await;
        let rendered = render_template(&template_id, &vars, &core).await?;
        content = if content.is_empty() {
            rendered
        } else {
            format!("{rendered}\n{content}")
        };
    }
    let new = NewNote {
        id: Uuid::new_v4().to_string(),
        title: input.title,
        format,
        bindings,
        tags: input.tag_names.unwrap_or_default(),
        content,
        created_at: now.clone(),
        updated_at: now,
    };
    insert_note(new, &core, &notes).await
}

#[tauri::command]
pub async fn note_update(
    id: String,
    input: NoteUpdateInput,
    core: tauri::State<'_, Core>,
    notes: tauri::State<'_, NotesState>,
) -> CmdResult<Note> {
    update_note(&id, input, &core, &notes).await
}

/// Apply title/pinned/content changes: snapshot, rewrite file, reindex.
/// A new body needs `base_hash`, the hash of the body it replaces: a body
/// written blind would drop whatever was saved since its writer read the file.
pub(crate) async fn update_note(
    id: &str,
    input: NoteUpdateInput,
    core: &Core,
    notes: &NotesState,
) -> Result<Note, AppError> {
    let id = id.to_string();
    let now = Utc::now().to_rfc3339();
    if input.content.is_some() && input.base_hash.is_none() {
        return Err(AppError::other(format!(
            "note {id}: a new body needs the hash of the body it replaces"
        )));
    }

    // Held for the whole read-check-write sequence below. The `base_hash` check
    // reads the file, then several awaits happen before the write, so without
    // this two concurrent saves both pass the check and the later write
    // silently drops the earlier one. Every writer of a note file takes it.
    // Not reentrant: nothing called below may take it again.
    let _save_guard = notes.save.lock().await;

    let mut row = sqlx::query_as::<_, NoteRow>("SELECT * FROM notes WHERE id = ?")
        .bind(&id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found(format!("Note {id}")))?;

    let old_title = row.title.clone();
    if let Some(title) = input.title {
        row.title = title;
    }
    if let Some(pinned) = input.pinned {
        row.pinned = if pinned { 1 } else { 0 };
    }
    row.updated_at = now.clone();

    let file_path = resolve_note_abs_path(&core.app_data_dir, &row.file_path);
    let (_, old_tags_list, old_body) = if file_path.exists() {
        read_note_file(&file_path)?
    } else {
        (HashMap::new(), vec![], String::new())
    };

    let old_content = old_body.clone();
    // Reject a stale buffer. A call without a body (title, pin) keeps the body it read.
    let old_hash = compute_hash(&old_content);
    if input.content.is_some() && input.base_hash.as_deref() != Some(old_hash.as_str()) {
        return Err(AppError::conflict_changed(format!("note {id}")));
    }
    let content = input.content.unwrap_or(old_body);
    let content_hash = compute_hash(&content);
    let preview = make_preview(&content);

    let tag_names = fetch_note_tags(&id, &core.db)
        .await?
        .into_iter()
        .map(|t| t.name)
        .collect::<Vec<_>>();

    let _ = old_tags_list;

    // Snapshot the previous content before overwriting (if content actually changed).
    // A failed snapshot must not block the save itself, but don't lose it silently.
    if content != old_content {
        if let Err(e) = maybe_snapshot(&id, &row.title, &old_content, "save", &core.db).await {
            eprintln!("notes: version snapshot failed for note {id}: {e}");
        }
    }

    // An external edit since the read above is refused here, not overwritten.
    write_note_checked(&file_path, &row, &tag_names, &content, &old_hash)?;
    reindex_links(&id, &content, &core.db).await?;
    if old_title != row.title {
        propagate_rename(&id, &old_title, &row.title, core).await?;
    }

    let fts_rowid = fts_upsert(
        &id,
        &row.title,
        &content,
        &tag_names,
        row.fts_rowid,
        &core.db,
    )
    .await?;

    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    sqlx::query(
        "UPDATE notes SET title=?, pinned=?, updated_at=?, content_hash=?, preview=?, fts_rowid=? WHERE id=?",
    )
    .bind(&row.title)
    .bind(row.pinned)
    .bind(&now)
    .bind(&content_hash)
    .bind(&preview)
    .bind(fts_rowid)
    .bind(&id)
    .execute(&mut *tx)
    .await
    .map_err(AppError::db)?;
    crate::directory::relabel(core, &mut tx, &id).await?;
    tx.commit().await.map_err(AppError::db)?;

    // Delete draft after successful save
    if let Some(draft) = draft_file_path(&core.app_data_dir, &id) {
        if draft.exists() {
            let _ = std::fs::remove_file(&draft);
        }
    }

    rebuild_manifest(&core.db, &core.app_data_dir).await?;

    let tags = fetch_note_tags(&id, &core.db).await?;
    let folder_ids = fetch_note_folder_ids(&id, &core.db).await?;

    let bindings: Vec<String> = serde_json::from_str(&row.bindings).unwrap_or_default();

    Ok(Note {
        id: row.id,
        title: row.title,
        base_dir: note_base_dir(&core.app_data_dir, &row.file_path),
        file_path: row.file_path,
        format: row.format,
        bindings,
        tags,
        folder_ids,
        pinned: row.pinned != 0,
        archived: row.archived != 0,
        deleted: row.deleted != 0,
        doc_status: row.doc_status,
        created_at: row.created_at,
        updated_at: now,
        content_hash: Some(content_hash),
        content: Some(content),
        has_draft: false,
    })
}

/// Hard delete: file, attachments, draft, history and every index row.
async fn hard_delete(id: &str, core: &Core) -> Result<(), AppError> {
    let row = sqlx::query_as::<_, NoteRow>("SELECT * FROM notes WHERE id = ?")
        .bind(id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?;

    let fts_rowid = row.as_ref().and_then(|r| r.fts_rowid);
    if let Some(r) = row {
        let file_path = resolve_note_abs_path(&core.app_data_dir, &r.file_path);
        if file_path.exists() {
            let _ = std::fs::remove_file(&file_path);
        }
        remove_note_attachments(&file_path, id);
    }

    // Sensitive leftovers: unsaved draft and the version history
    if let Some(draft) = draft_file_path(&core.app_data_dir, id) {
        let _ = std::fs::remove_file(draft);
    }
    delete_note_rows(id, fts_rowid, core).await
}

/// Every row of a note: its search entry, history, links and the note
/// itself. No file is touched.
pub(crate) async fn delete_note_rows(
    id: &str,
    fts_rowid: Option<i64>,
    core: &Core,
) -> Result<(), AppError> {
    fts_delete(fts_rowid, &core.db).await?;
    sqlx::query("UPDATE note_history SET parent_id = NULL WHERE note_id = ?")
        .bind(id)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;
    sqlx::query("DELETE FROM note_history WHERE note_id = ?")
        .bind(id)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;

    sqlx::query("DELETE FROM note_tag_links WHERE note_id = ?")
        .bind(id)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;

    sqlx::query("DELETE FROM note_folder_links WHERE note_id = ?")
        .bind(id)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;

    sqlx::query("DELETE FROM note_links WHERE from_id = ? OR to_id = ?")
        .bind(id)
        .bind(id)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;
    sqlx::query("DELETE FROM note_mentions WHERE note_id = ?")
        .bind(id)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;

    // Whoever links the note by its id (pass: tags `note:{id}`) lets go of
    // it in the transaction that deletes it, and its label goes.
    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    core.deletions.emit(&mut tx, crate::KIND_NOTE, id).await?;
    sqlx::query("DELETE FROM notes WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;
    crate::directory::relabel(core, &mut tx, id).await?;
    tx.commit().await.map_err(AppError::db)?;
    Ok(())
}

/// Soft delete: the row stays for sync, only the search index entry goes.
async fn soft_delete(id: &str, core: &Core) -> Result<(), AppError> {
    sqlx::query("UPDATE notes SET deleted=1, updated_at=? WHERE id=?")
        .bind(Utc::now().to_rfc3339())
        .bind(id)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;

    let row = sqlx::query_as::<_, NoteRow>("SELECT * FROM notes WHERE id = ?")
        .bind(id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?;
    if let Some(r) = row {
        fts_delete(r.fts_rowid, &core.db).await?;
        sqlx::query("UPDATE notes SET fts_rowid = NULL WHERE id = ?")
            .bind(id)
            .execute(&core.db)
            .await
            .map_err(AppError::db)?;
    }
    Ok(())
}

/// Move a note to the trash, for a caller that holds the state and not the app.
pub(crate) async fn trash_note(id: &str, core: &Core) -> Result<(), AppError> {
    soft_delete(id, core).await?;
    rebuild_manifest(&core.db, &core.app_data_dir).await
}

#[tauri::command]
pub async fn note_delete(
    id: String,
    hard: Option<bool>,
    core: tauri::State<'_, Core>,
) -> CmdResult<()> {
    if hard.unwrap_or(false) {
        hard_delete(&id, &core).await?;
    } else {
        soft_delete(&id, &core).await?;
    }
    rebuild_manifest(&core.db, &core.app_data_dir).await?;
    Ok(())
}

/// Move several notes to the trash with a single manifest rebuild.
#[tauri::command]
pub async fn note_delete_many(ids: Vec<String>, core: tauri::State<'_, Core>) -> CmdResult<()> {
    for id in &ids {
        soft_delete(id, &core).await?;
    }
    rebuild_manifest(&core.db, &core.app_data_dir).await?;
    Ok(())
}

/// Hard-delete everything in the trash; returns how many notes were removed.
#[tauri::command]
pub async fn note_trash_empty(core: tauri::State<'_, Core>) -> CmdResult<usize> {
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM notes WHERE deleted = 1")
        .fetch_all(&core.db)
        .await
        .map_err(AppError::db)?;
    for id in &ids {
        hard_delete(id, &core).await?;
    }
    rebuild_manifest(&core.db, &core.app_data_dir).await?;
    Ok(ids.len())
}

#[tauri::command]
pub async fn note_archive(id: String, core: tauri::State<'_, Core>) -> CmdResult<()> {
    sqlx::query("UPDATE notes SET archived=1, updated_at=? WHERE id=?")
        .bind(Utc::now().to_rfc3339())
        .bind(&id)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;
    Ok(())
}

#[tauri::command]
pub async fn note_restore(id: String, core: tauri::State<'_, Core>) -> CmdResult<()> {
    restore_note(&id, &core).await
}

/// Take a note out of the trash or the archive.
pub(crate) async fn restore_note(id: &str, core: &Core) -> Result<(), AppError> {
    let row = sqlx::query_as::<_, NoteRow>("SELECT * FROM notes WHERE id = ?")
        .bind(id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found(format!("Note {id}")))?;

    // The label too: another device may have retracted it when it deleted the
    // note for good.
    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    sqlx::query(
        "UPDATE notes SET archived=0, deleted=0, doc_status='active', updated_at=? WHERE id=?",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(AppError::db)?;
    crate::directory::relabel(core, &mut tx, id).await?;
    tx.commit().await.map_err(AppError::db)?;

    // Soft delete drops the FTS row; put it back when leaving the trash.
    if row.deleted != 0 {
        let file_path = resolve_note_abs_path(&core.app_data_dir, &row.file_path);
        let (_, tags_list, body) = read_note_file(&file_path).unwrap_or_default();
        let fts_rowid =
            fts_upsert(id, &row.title, &body, &tags_list, row.fts_rowid, &core.db).await?;
        sqlx::query("UPDATE notes SET fts_rowid=? WHERE id=?")
            .bind(fts_rowid)
            .bind(id)
            .execute(&core.db)
            .await
            .map_err(AppError::db)?;
        rebuild_manifest(&core.db, &core.app_data_dir).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn note_set_tags(
    id: String,
    tag_names: Vec<String>,
    core: tauri::State<'_, Core>,
    notes: tauri::State<'_, NotesState>,
) -> CmdResult<()> {
    // The file is rewritten with the body read here: a save in flight lands
    // first, or this write would bring the older body back.
    let _save_guard = notes.save.lock().await;
    let now = Utc::now().to_rfc3339();
    let tag_names = set_note_tag_links(&id, &tag_names, &core.db).await?;

    sqlx::query("UPDATE notes SET updated_at=? WHERE id=?")
        .bind(&now)
        .bind(&id)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;

    // Rewrite file frontmatter with new tags
    let row = sqlx::query_as::<_, NoteRow>("SELECT * FROM notes WHERE id = ?")
        .bind(&id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found(format!("Note {id}")))?;

    let file_path = resolve_note_abs_path(&core.app_data_dir, &row.file_path);
    if file_path.exists() {
        let (_, _, body) = read_note_file(&file_path)?;
        let mut updated_row = row.clone();
        updated_row.updated_at = now.clone();
        write_note_file(&file_path, &updated_row, &tag_names, &body)?;

        let fts_rowid =
            fts_upsert(&id, &row.title, &body, &tag_names, row.fts_rowid, &core.db).await?;

        sqlx::query("UPDATE notes SET fts_rowid=? WHERE id=?")
            .bind(fts_rowid)
            .bind(&id)
            .execute(&core.db)
            .await
            .map_err(AppError::db)?;
    }

    rebuild_manifest(&core.db, &core.app_data_dir).await?;
    Ok(())
}

/// FTS5 MATCH treats quotes, parens, NEAR/AND/OR etc. as query syntax, so raw
/// user input can produce SQL errors. Wrap each whitespace-separated term in
/// double quotes (doubling embedded quotes) so it matches literally; the
/// trailing `*` keeps the last term a prefix search. `None` for blank input.
pub(crate) fn fts_match_query(query: &str) -> Option<String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut fts_query = trimmed
        .split_whitespace()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ");
    fts_query.push('*');
    Some(fts_query)
}

/// Escape a raw FTS snippet and turn the \u{1}/\u{2} markers into <mark> tags.
pub(crate) fn snippet_html(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 16);
    for c in raw.chars() {
        match c {
            '\u{1}' => out.push_str("<mark>"),
            '\u{2}' => out.push_str("</mark>"),
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

#[tauri::command]
pub async fn note_search(
    query: String,
    filter: NoteFilter,
    core: tauri::State<'_, Core>,
) -> CmdResult<Vec<NoteListItem>> {
    let Some(fts_query) = fts_match_query(&query) else {
        return Ok(vec![]);
    };

    // Control chars mark the match; the raw text is HTML-escaped before the
    // markers become <mark> tags, so note content never reaches the UI as HTML.
    let matched: Vec<(String, String)> =
        sqlx::query_as("SELECT note_id, snippet(notes_fts, 2, char(1), char(2), '…', 12) FROM notes_fts WHERE notes_fts MATCH ? ORDER BY rank LIMIT 100")
            .bind(&fts_query)
            .fetch_all(&core.db)
            .await
            .map_err(AppError::db)?;

    if matched.is_empty() {
        return Ok(vec![]);
    }

    let snippets_map: std::collections::HashMap<String, String> = matched
        .iter()
        .map(|(id, snip)| (id.clone(), snippet_html(snip)))
        .collect();
    let ids: Vec<String> = matched.into_iter().map(|(id, _)| id).collect();
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT * FROM notes WHERE id IN ({}) ORDER BY pinned DESC, updated_at DESC",
        placeholders
    );

    // Safe: only `?` placeholders are interpolated; ids are bound below.
    let mut q = sqlx::query_as::<_, NoteRow>(sqlx::AssertSqlSafe(sql));
    for id in &ids {
        q = q.bind(id);
    }
    let rows = q.fetch_all(&core.db).await.map_err(AppError::db)?;

    // Archived notes are searchable only when the archive filter is active.
    let mut filter = filter;
    if filter.archived.is_none() {
        filter.archived = Some(false);
    }
    let ctx = FilterContext::load(&filter, &core).await?;
    let mut items = list_items(rows, &filter, &ctx, &core.app_data_dir);
    for item in &mut items {
        item.snippet = snippets_map.get(&item.id).cloned();
    }
    Ok(items)
}

#[tauri::command]
pub async fn note_sync(
    core: tauri::State<'_, Core>,
    notes: tauri::State<'_, NotesState>,
) -> CmdResult<()> {
    let custom_dir = notes.custom_dir.read().ok().and_then(|g| g.clone());
    sync_notes_index(&core, custom_dir.as_ref()).await
}

#[tauri::command]
pub async fn note_reindex(core: tauri::State<'_, Core>) -> CmdResult<()> {
    // Full FTS rebuild
    sqlx::query("DELETE FROM notes_fts")
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;
    sqlx::query("UPDATE notes SET fts_rowid = NULL WHERE deleted = 1")
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;

    let rows = sqlx::query_as::<_, NoteRow>("SELECT * FROM notes WHERE deleted = 0")
        .fetch_all(&core.db)
        .await
        .map_err(AppError::db)?;

    for row in rows {
        let file_path = resolve_note_abs_path(&core.app_data_dir, &row.file_path);
        let (_, tags_list, body) = read_note_file(&file_path).unwrap_or_default();
        let tags_str = tags_list.join(" ");

        let mut conn = core.db.acquire().await.map_err(AppError::db)?;
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
            .execute(&core.db)
            .await
            .map_err(AppError::db)?;
    }

    Ok(())
}

#[cfg(desktop)]
#[tauri::command]
pub async fn note_open_folder(
    core: tauri::State<'_, Core>,
    notes: tauri::State<'_, NotesState>,
) -> CmdResult<()> {
    let custom_dir = notes.custom_dir.read().ok().and_then(|g| g.clone());
    let dir = effective_docs_dir(&core.app_data_dir, custom_dir.as_ref());
    open_path(&dir)
}

#[cfg(desktop)]
#[tauri::command]
pub async fn note_open_external(id: String, core: tauri::State<'_, Core>) -> CmdResult<()> {
    let row = sqlx::query_as::<_, NoteRow>("SELECT * FROM notes WHERE id = ?")
        .bind(&id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found(format!("Note {id}")))?;

    let file_path = resolve_note_abs_path(&core.app_data_dir, &row.file_path);
    open_path(&file_path)
}

/// The draft file of a note the webview names; an id that is not a note id is refused.
fn draft_path(core: &Core, id: &str) -> Result<std::path::PathBuf, AppError> {
    draft_file_path(&core.app_data_dir, id)
        .ok_or_else(|| AppError::other(format!("invalid note id {id:?}")))
}

#[tauri::command]
pub async fn note_draft_save(
    id: String,
    content: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<()> {
    let draft_path = draft_path(&core, &id)?;
    std::fs::write(&draft_path, &content).map_err(AppError::io)?;
    Ok(())
}

#[tauri::command]
pub async fn note_draft_get(id: String, core: tauri::State<'_, Core>) -> CmdResult<Option<String>> {
    let draft_path = draft_path(&core, &id)?;
    if draft_path.exists() {
        let content = std::fs::read_to_string(&draft_path).map_err(AppError::io)?;
        Ok(Some(content))
    } else {
        Ok(None)
    }
}

#[tauri::command]
pub async fn note_draft_discard(id: String, core: tauri::State<'_, Core>) -> CmdResult<()> {
    let draft_path = draft_path(&core, &id)?;
    if draft_path.exists() {
        std::fs::remove_file(&draft_path).map_err(AppError::io)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::Manager;

    /// A row `..` (an older index took it from `...md`) is deleted without
    /// its id reaching a path: the documents and the attachments of other
    /// notes stay.
    #[tokio::test]
    async fn hard_delete_of_the_row_dot_dot_keeps_the_documents() {
        let (app, dir) = crate::testing::app().await;
        let core = app.state::<Core>();
        let docs = documents_dir(dir.path());
        std::fs::write(docs.join("n-1.md"), "body").unwrap();
        std::fs::write(docs.join("...md"), "body").unwrap();
        std::fs::create_dir_all(docs.join("attachments/n-1")).unwrap();
        std::fs::write(docs.join("attachments/n-1/a.png"), "png").unwrap();
        sqlx::query(
            "INSERT INTO notes (id, title, file_path, created_at, updated_at)
             VALUES ('..', '..', ?, '2026-01-01', '2026-01-01')",
        )
        .bind(docs.join("...md").to_string_lossy().to_string())
        .execute(&core.db)
        .await
        .unwrap();

        hard_delete("..", &core).await.unwrap();

        assert!(docs.join("n-1.md").exists());
        assert!(docs.join("attachments/n-1/a.png").exists());
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notes")
            .fetch_one(&core.db)
            .await
            .unwrap();
        assert_eq!(left, 0);
    }

    #[tokio::test]
    async fn a_format_that_is_not_a_note_extension_is_refused() {
        let (app, dir) = crate::testing::app().await;
        let input = |format: &str| NoteCreateInput {
            title: "x".into(),
            format: Some(format.into()),
            bindings: None,
            tag_names: None,
            content: None,
            template_id: None,
        };

        let err = note_create(input("md/../../x"), app.state(), app.state()).await;

        assert!(err.is_err());
        assert!(!dir.path().join("notes/x").exists());
        assert!(!dir.path().join("x").exists());
        let note = note_create(input("txt"), app.state(), app.state())
            .await
            .unwrap();
        assert!(note.file_path.ends_with(&format!("{}.txt", note.id)));
    }

    #[tokio::test]
    async fn a_draft_of_an_id_that_is_a_path_is_refused() {
        let (app, dir) = crate::testing::app().await;

        assert!(note_draft_save("../x".into(), "c".into(), app.state())
            .await
            .is_err());
        assert!(note_draft_get("../x".into(), app.state()).await.is_err());
        assert!(note_draft_discard("../x".into(), app.state())
            .await
            .is_err());
        assert!(!dir.path().join("notes/x.draft").exists());

        note_draft_save("n-1".into(), "c".into(), app.state())
            .await
            .unwrap();
        assert_eq!(
            note_draft_get("n-1".into(), app.state()).await.unwrap(),
            Some("c".into())
        );
    }

    async fn new_note_with(app: &tauri::App<tauri::test::MockRuntime>, id: &str, content: &str) {
        let now = Utc::now().to_rfc3339();
        insert_note(
            NewNote {
                id: id.into(),
                title: id.into(),
                format: "md".into(),
                bindings: Vec::new(),
                tags: Vec::new(),
                content: content.into(),
                created_at: now.clone(),
                updated_at: now,
            },
            &app.state::<Core>(),
            &app.state::<NotesState>(),
        )
        .await
        .unwrap();
    }

    async fn fts_hits(core: &Core, word: &str) -> Vec<String> {
        sqlx::query_scalar("SELECT note_id FROM notes_fts WHERE notes_fts MATCH ?")
            .bind(word)
            .fetch_all(&core.db)
            .await
            .unwrap()
    }

    /// A deleted note keeps no rowid of the search table: after a rebuild a
    /// rowid a live note took must not go with the first note's final delete.
    #[tokio::test]
    async fn a_deleted_note_leaves_no_stale_search_row() {
        let (app, _dir) = crate::testing::app().await;
        let core = app.state::<Core>();
        new_note_with(&app, "n-1", "alphaword").await;
        soft_delete("n-1", &core).await.unwrap();
        let rowid: Option<i64> = sqlx::query_scalar("SELECT fts_rowid FROM notes WHERE id='n-1'")
            .fetch_one(&core.db)
            .await
            .unwrap();
        assert_eq!(rowid, None);

        note_reindex(app.state()).await.unwrap();
        new_note_with(&app, "n-2", "betaword").await;
        hard_delete("n-1", &core).await.unwrap();

        assert_eq!(fts_hits(&core, "betaword").await, ["n-2"]);
    }

    /// An older database: a soft-deleted row still holds its old rowid; the
    /// rebuild lets it go.
    #[tokio::test]
    async fn a_rebuild_clears_the_rowid_of_a_deleted_row() {
        let (app, _dir) = crate::testing::app().await;
        let core = app.state::<Core>();
        new_note_with(&app, "n-1", "alphaword").await;
        sqlx::query("UPDATE notes SET deleted=1 WHERE id='n-1'")
            .execute(&core.db)
            .await
            .unwrap();

        note_reindex(app.state()).await.unwrap();

        let rowid: Option<i64> = sqlx::query_scalar("SELECT fts_rowid FROM notes WHERE id='n-1'")
            .fetch_one(&core.db)
            .await
            .unwrap();
        assert_eq!(rowid, None);
        assert!(fts_hits(&core, "alphaword").await.is_empty());
    }
    /// A note `n-1` with the body "Body", and the path of its file.
    async fn body_note(app: &tauri::App<tauri::test::MockRuntime>) -> std::path::PathBuf {
        let now = Utc::now().to_rfc3339();
        let core = app.state::<Core>();
        let note = insert_note(
            NewNote {
                id: "n-1".into(),
                title: "Plan".into(),
                format: "md".into(),
                bindings: Vec::new(),
                tags: Vec::new(),
                content: "Body".into(),
                created_at: now.clone(),
                updated_at: now,
            },
            &core,
            &app.state::<NotesState>(),
        )
        .await
        .unwrap();
        resolve_note_abs_path(&core.app_data_dir, &note.file_path)
    }

    fn body_of(path: &std::path::PathBuf) -> String {
        read_note_file(path).unwrap().2
    }

    fn new_body(content: &str, base: Option<&str>) -> NoteUpdateInput {
        NoteUpdateInput {
            title: None,
            content: Some(content.into()),
            pinned: None,
            base_hash: base.map(compute_hash),
        }
    }

    /// A new body is written only over the body it was made from.
    #[tokio::test]
    async fn a_new_body_without_the_hash_it_replaces_is_refused() {
        let (app, _dir) = crate::testing::app().await;
        let path = body_note(&app).await;
        let (core, notes) = (app.state::<Core>(), app.state::<NotesState>());

        let blind = update_note("n-1", new_body("Blind", None), &core, &notes).await;
        assert!(matches!(blind, Err(AppError::Other(_))), "{blind:?}");
        let stale = update_note("n-1", new_body("Stale", Some("Old")), &core, &notes).await;
        assert!(
            matches!(stale, Err(AppError::ConflictChanged(_))),
            "{stale:?}"
        );
        assert_eq!(body_of(&path), "Body");

        update_note("n-1", new_body("Edited", Some("Body")), &core, &notes)
            .await
            .unwrap();
        assert_eq!(body_of(&path), "Edited");
        // A title alone keeps the body and needs no hash.
        let rename = NoteUpdateInput {
            title: Some("Plan B".into()),
            content: None,
            pinned: None,
            base_hash: None,
        };
        update_note("n-1", rename, &core, &notes).await.unwrap();
        assert_eq!(body_of(&path), "Edited");
    }

    /// The tags rewrite the file with the body saved before them, never with
    /// an older one, and wait while a save holds the lock.
    #[tokio::test]
    async fn the_tags_after_a_save_keep_its_body_and_wait_for_the_lock() {
        let (app, _dir) = crate::testing::app().await;
        let path = body_note(&app).await;
        let (core, notes) = (app.state::<Core>(), app.state::<NotesState>());
        update_note("n-1", new_body("Saved", Some("Body")), &core, &notes)
            .await
            .unwrap();

        let held = notes.save.lock().await;
        let tags = note_set_tags("n-1".into(), vec!["work".into()], app.state(), app.state());
        let waited = tokio::time::timeout(std::time::Duration::from_millis(200), tags).await;
        assert!(waited.is_err(), "the tags were written under a held lock");
        drop(held);

        note_set_tags("n-1".into(), vec!["work".into()], app.state(), app.state())
            .await
            .unwrap();
        let (_, tags, body) = read_note_file(&path).unwrap();
        assert_eq!(body, "Saved");
        assert_eq!(tags, ["work"]);
    }

    /// Every other writer of a note file waits for the lock too.
    #[tokio::test]
    async fn the_bindings_and_a_restore_wait_for_the_lock() {
        let (app, _dir) = crate::testing::app().await;
        let path = body_note(&app).await;
        let (core, notes) = (app.state::<Core>(), app.state::<NotesState>());
        let version = history_snapshot("n-1", "Plan", "Older", "save", None, &core.db)
            .await
            .unwrap();
        let wait = std::time::Duration::from_millis(200);

        let held = notes.save.lock().await;
        let bind = crate::folders::note_add_binding(
            "n-1".into(),
            "profile:p-1".into(),
            app.state(),
            app.state(),
        );
        assert!(tokio::time::timeout(wait, bind).await.is_err());
        let restore = crate::history::note_history_restore(
            "n-1".into(),
            version.clone(),
            app.state(),
            app.state(),
        );
        assert!(tokio::time::timeout(wait, restore).await.is_err());
        drop(held);

        crate::history::note_history_restore("n-1".into(), version, app.state(), app.state())
            .await
            .unwrap();
        assert_eq!(body_of(&path), "Older");
    }

    /// An edit on disk after the restore read the file is refused, not lost.
    #[test]
    fn a_checked_write_refuses_a_body_changed_since_the_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n-1.md");
        let row = NoteRow {
            id: "n-1".into(),
            title: "Plan".into(),
            file_path: String::new(),
            format: "md".into(),
            pinned: 0,
            archived: 0,
            deleted: 0,
            doc_status: "active".into(),
            version_base: None,
            fts_rowid: None,
            created_at: "2026-09-30T10:00:00+00:00".into(),
            updated_at: "2026-09-30T11:00:00+00:00".into(),
            file_mtime: None,
            content_hash: None,
            preview: String::new(),
            bindings: "[]".into(),
        };

        write_note_checked(&path, &row, &[], "First", &compute_hash("")).unwrap();
        assert_eq!(body_of(&path), "First");
        let err = write_note_checked(&path, &row, &[], "Second", &compute_hash("Other"));
        assert!(matches!(err, Err(AppError::ConflictChanged(_))), "{err:?}");
        assert_eq!(body_of(&path), "First");
        write_note_checked(&path, &row, &[], "Second", &compute_hash("First")).unwrap();
        assert_eq!(body_of(&path), "Second");
    }
}
