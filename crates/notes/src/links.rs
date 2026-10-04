// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Wiki links `[[Title]]` / `[[Title|alias]]` / `[[id]]` between notes.
//! `[[kind:id|Name]]` with an entity binding kind is a mention of a Veydan
//! object (proxy, ssh, ...) and goes to `note_mentions` instead of `note_links`.
//! Both tables are rebuilt from the body on every save;
//! renaming a note rewrites `[[Old title]]` in the notes that link to it.

use super::binding::{is_entity_binding, BindingKind};
use super::files::{read_note_file, resolve_note_abs_path, row_to_list_item, write_note_file};
use super::models::{NoteListItem, NoteRow};
use super::tags::{fetch_all_note_folder_ids_map, fetch_all_note_tags_map};
use std::collections::HashSet;
use veydan_core::Core;
use veydan_core::{AppError, CmdResult};

type Db = sqlx::Pool<sqlx::Sqlite>;

const WIKI_OPEN: &str = "[[";
const WIKI_CLOSE: &str = "]]";

/// One wiki link occurrence and the byte offset where its inner text begins.
struct WikiSpan<'a> {
    inner_start: usize,
    inner: &'a str,
}

/// All wiki links in `content`, in document order.
fn wiki_spans(content: &str) -> Vec<WikiSpan<'_>> {
    let mut spans = Vec::new();
    let mut pos = 0;
    while pos < content.len() {
        let rest = &content[pos..];
        let Some(open_at) = rest.find(WIKI_OPEN) else {
            break;
        };
        let after = &rest[open_at + WIKI_OPEN.len()..];
        let Some(close_at) = after.find(WIKI_CLOSE) else {
            pos += open_at + WIKI_OPEN.len();
            continue;
        };
        let inner = &after[..close_at];
        let start = pos + open_at;
        let end = start + WIKI_OPEN.len() + close_at + WIKI_CLOSE.len();
        if !inner.contains('\n') {
            spans.push(WikiSpan {
                inner_start: start + WIKI_OPEN.len(),
                inner,
            });
        }
        pos = end;
    }
    spans
}

/// Target part of a link's inner text (before `|`, trimmed).
fn inner_target(inner: &str) -> &str {
    inner.split('|').next().unwrap_or("").trim()
}

/// Distinct link targets (text before `|`, trimmed) matching `keep`.
fn distinct_targets(content: &str, keep: impl Fn(&str) -> bool) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for span in wiki_spans(content) {
        let target = inner_target(span.inner);
        if !target.is_empty() && keep(target) && seen.insert(target.to_lowercase()) {
            out.push(target.to_string());
        }
    }
    out
}

/// Note targets in the body, deduplicated; entity mentions are excluded.
pub(crate) fn extract_targets(content: &str) -> Vec<String> {
    distinct_targets(content, |t| !is_entity_binding(t))
}

/// Entity mentions in the body, e.g. `ssh:{id}`, deduplicated.
pub(crate) fn extract_mentions(content: &str) -> Vec<String> {
    distinct_targets(content, is_entity_binding)
}

/// Resolve a target to a live note id: exact id first, then case-insensitive title.
pub(crate) async fn resolve_target(target: &str, db: &Db) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT id FROM notes WHERE deleted = 0 AND (id = ? OR lower(title) = lower(?))
         ORDER BY CASE WHEN id = ? THEN 0 ELSE 1 END, updated_at DESC LIMIT 1",
    )
    .bind(target)
    .bind(target)
    .bind(target)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

/// Replace outgoing links and entity mentions of a note with those found in `content`.
pub(crate) async fn reindex_links(note_id: &str, content: &str, db: &Db) -> Result<(), AppError> {
    sqlx::query("DELETE FROM note_mentions WHERE note_id = ?")
        .bind(note_id)
        .execute(db)
        .await
        .map_err(AppError::db)?;
    for binding in extract_mentions(content) {
        sqlx::query("INSERT OR IGNORE INTO note_mentions (note_id, binding) VALUES (?, ?)")
            .bind(note_id)
            .bind(&binding)
            .execute(db)
            .await
            .map_err(AppError::db)?;
    }
    sqlx::query("DELETE FROM note_links WHERE from_id = ?")
        .bind(note_id)
        .execute(db)
        .await
        .map_err(AppError::db)?;
    for target in extract_targets(content) {
        let Some(to_id) = resolve_target(&target, db).await else {
            continue;
        };
        if to_id == note_id {
            continue;
        }
        sqlx::query("INSERT OR IGNORE INTO note_links (from_id, to_id) VALUES (?, ?)")
            .bind(note_id)
            .bind(&to_id)
            .execute(db)
            .await
            .map_err(AppError::db)?;
    }
    Ok(())
}

/// Rewrite `[[old]]` -> `[[new]]` (any case, with or without alias) in one body.
pub(crate) fn rewrite_title_links(body: &str, old_title: &str, new_title: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut last = 0;
    for span in wiki_spans(body) {
        let (target, alias) = match span.inner.find('|') {
            Some(p) => (&span.inner[..p], Some(&span.inner[p..])),
            None => (span.inner, None),
        };
        if !target.trim().eq_ignore_ascii_case(old_title.trim()) {
            continue;
        }
        out.push_str(&body[last..span.inner_start]);
        out.push_str(new_title);
        if let Some(a) = alias {
            out.push_str(a);
        }
        last = span.inner_start + span.inner.len();
    }
    out.push_str(&body[last..]);
    out
}

/// After a title change: update files of all notes linking to `note_id`.
pub(crate) async fn propagate_rename(
    note_id: &str,
    old_title: &str,
    new_title: &str,
    core: &Core,
) -> Result<(), AppError> {
    if old_title.trim().eq_ignore_ascii_case(new_title.trim()) {
        return Ok(());
    }
    let sources = sqlx::query_as::<_, NoteRow>(
        "SELECT n.* FROM notes n JOIN note_links l ON l.from_id = n.id WHERE l.to_id = ? AND n.deleted = 0",
    )
    .bind(note_id)
    .fetch_all(&core.db)
    .await
    .map_err(AppError::db)?;

    for row in sources {
        let path = resolve_note_abs_path(&core.app_data_dir, &row.file_path);
        let Ok((_, tags, body)) = read_note_file(&path) else {
            continue;
        };
        let rewritten = rewrite_title_links(&body, old_title, new_title);
        if rewritten == body {
            continue;
        }
        write_note_file(&path, &row, &tags, &rewritten)?;
        let hash = super::files::compute_hash(&rewritten);
        sqlx::query("UPDATE notes SET content_hash = ?, preview = ? WHERE id = ?")
            .bind(&hash)
            .bind(super::files::make_preview(&rewritten))
            .bind(&row.id)
            .execute(&core.db)
            .await
            .map_err(AppError::db)?;
        let fts_rowid = super::index::fts_upsert(
            &row.id,
            &row.title,
            &rewritten,
            &tags,
            row.fts_rowid,
            &core.db,
        )
        .await?;
        sqlx::query("UPDATE notes SET fts_rowid = ? WHERE id = ?")
            .bind(fts_rowid)
            .bind(&row.id)
            .execute(&core.db)
            .await
            .map_err(AppError::db)?;
    }
    Ok(())
}

async fn rows_to_items(rows: Vec<NoteRow>, core: &Core) -> Result<Vec<NoteListItem>, AppError> {
    let tags = fetch_all_note_tags_map(&core.db).await?;
    let folders = fetch_all_note_folder_ids_map(&core.db).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let t = tags.get(&r.id).cloned().unwrap_or_default();
            let f = folders.get(&r.id).cloned().unwrap_or_default();
            row_to_list_item(r, t, f, false)
        })
        .collect())
}

#[derive(Debug, serde::Serialize)]
pub struct NoteLinks {
    /// Notes this note links to.
    pub outgoing: Vec<NoteListItem>,
    /// Notes linking to this note.
    pub backlinks: Vec<NoteListItem>,
    /// Link targets in the body that match no live note.
    pub unresolved: Vec<String>,
}

/// Targets in the note body that do not resolve to an existing note.
async fn unresolved_targets(id: &str, core: &Core) -> Result<Vec<String>, AppError> {
    let Some(file_path) =
        sqlx::query_scalar::<_, String>("SELECT file_path FROM notes WHERE id = ?")
            .bind(id)
            .fetch_optional(&core.db)
            .await
            .map_err(AppError::db)?
    else {
        return Ok(vec![]);
    };
    let Ok((_, _, body)) = read_note_file(&resolve_note_abs_path(&core.app_data_dir, &file_path))
    else {
        return Ok(vec![]);
    };
    let mut out = Vec::new();
    for target in extract_targets(&body) {
        if resolve_target(&target, &core.db).await.is_none() {
            out.push(target);
        }
    }
    Ok(out)
}

/// Outgoing links, backlinks and unresolved targets in one call.
#[tauri::command]
pub async fn note_links(id: String, core: tauri::State<'_, Core>) -> CmdResult<NoteLinks> {
    let outgoing = sqlx::query_as::<_, NoteRow>(
        "SELECT n.* FROM notes n JOIN note_links l ON l.to_id = n.id
         WHERE l.from_id = ? AND n.deleted = 0 ORDER BY n.updated_at DESC",
    )
    .bind(&id)
    .fetch_all(&core.db)
    .await
    .map_err(AppError::db)?;
    let backlinks = sqlx::query_as::<_, NoteRow>(
        "SELECT n.* FROM notes n JOIN note_links l ON l.from_id = n.id
         WHERE l.to_id = ? AND n.deleted = 0 ORDER BY n.updated_at DESC",
    )
    .bind(&id)
    .fetch_all(&core.db)
    .await
    .map_err(AppError::db)?;
    Ok(NoteLinks {
        outgoing: rows_to_items(outgoing, &core).await?,
        backlinks: rows_to_items(backlinks, &core).await?,
        unresolved: unresolved_targets(&id, &core).await?,
    })
}

/// Id of the note a `[[target]]` points to, if it exists.
#[tauri::command]
pub async fn note_resolve_link(
    target: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<Option<String>> {
    Ok(resolve_target(target.trim(), &core.db).await)
}

/// Notes that link to this note.
#[tauri::command]
pub async fn note_backlinks(
    id: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<Vec<NoteListItem>> {
    let rows = sqlx::query_as::<_, NoteRow>(
        "SELECT n.* FROM notes n JOIN note_links l ON l.from_id = n.id
         WHERE l.to_id = ? AND n.deleted = 0 ORDER BY n.updated_at DESC",
    )
    .bind(&id)
    .fetch_all(&core.db)
    .await
    .map_err(AppError::db)?;
    rows_to_items(rows, &core).await
}

/// Notes sharing a domain / profile / workspace binding, most specific first.
#[tauri::command]
pub async fn note_related(
    id: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<Vec<NoteListItem>> {
    let Some(bindings_json) =
        sqlx::query_scalar::<_, String>("SELECT bindings FROM notes WHERE id = ?")
            .bind(&id)
            .fetch_optional(&core.db)
            .await
            .map_err(AppError::db)?
    else {
        return Ok(vec![]);
    };
    let bindings: Vec<String> = serde_json::from_str(&bindings_json).unwrap_or_default();
    let mut ordered: Vec<&String> = Vec::new();
    for kind in BindingKind::ALL {
        ordered.extend(bindings.iter().filter(|b| b.starts_with(kind.prefix())));
    }
    if ordered.is_empty() {
        return Ok(vec![]);
    }

    let mut seen = HashSet::from([id.clone()]);
    let mut rows = Vec::new();
    for b in ordered {
        let pattern = format!("%{}%", serde_json::to_string(b).unwrap_or_default());
        let found = sqlx::query_as::<_, NoteRow>(
            "SELECT * FROM notes WHERE deleted = 0 AND archived = 0 AND bindings LIKE ? ORDER BY updated_at DESC LIMIT 20",
        )
        .bind(pattern)
        .fetch_all(&core.db)
        .await
        .map_err(AppError::db)?;
        for r in found {
            if seen.insert(r.id.clone()) {
                rows.push(r);
            }
        }
        if rows.len() >= 10 {
            break;
        }
    }
    rows.truncate(10);
    rows_to_items(rows, &core).await
}

/// Live notes bound to or mentioning an entity binding, e.g. `ssh:{id}`.
#[tauri::command]
pub async fn note_entity_notes(
    binding: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<Vec<NoteListItem>> {
    let pattern = format!("%{}%", serde_json::to_string(&binding).unwrap_or_default());
    let rows = sqlx::query_as::<_, NoteRow>(
        "SELECT n.* FROM notes n
         WHERE n.deleted = 0
           AND (n.bindings LIKE ? OR n.id IN (SELECT note_id FROM note_mentions WHERE binding = ?))
         ORDER BY n.pinned DESC, n.updated_at DESC LIMIT 50",
    )
    .bind(pattern)
    .bind(&binding)
    .fetch_all(&core.db)
    .await
    .map_err(AppError::db)?;
    rows_to_items(rows, &core).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_note_targets_and_entity_mentions() {
        let body = "[[Alpha]] [[ssh:abc|Prod]] [[proxy:1]] [[domain:x.com]] [[ssh:]]";
        assert_eq!(extract_targets(body), vec!["Alpha", "domain:x.com", "ssh:"]);
        assert_eq!(extract_mentions(body), vec!["ssh:abc", "proxy:1"]);
    }

    #[test]
    fn extracts_targets_without_alias_and_dedups() {
        let t = extract_targets("see [[Alpha]] and [[alpha|the alias]] plus [[Beta]]");
        assert_eq!(t, vec!["Alpha", "Beta"]);
    }

    #[test]
    fn trims_targets_and_skips_what_is_not_a_link() {
        let t = extract_targets("[[Alpha]] and [Beta] and [[alpha|x]] and [[ Gamma ]] [[");
        assert_eq!(t, vec!["Alpha", "Gamma"]);
    }

    #[test]
    fn rewrites_only_matching_links() {
        let body = "[[Old]] [[old|alias]] [[Other]] [[";
        assert_eq!(
            rewrite_title_links(body, "Old", "New"),
            "[[New]] [[New|alias]] [[Other]] [["
        );
    }

    #[test]
    fn rewrites_links_and_leaves_other_text_alone() {
        let body = "[[Old]] x [[old|alias]] Old [Old] [[Other]]";
        assert_eq!(
            rewrite_title_links(body, "Old", "New"),
            "[[New]] x [[New|alias]] Old [Old] [[Other]]"
        );
    }
}
