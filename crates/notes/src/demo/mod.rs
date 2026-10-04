// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The part of notes in the demo data, for video recordings: tags, folders,
//! notes with their files and smart views. The notes others link to are
//! known by their ids: `demo-note-NN` is the note at that place of the
//! pack, `demo-note-bulk-NNN` a bulk note; a note binds the workspaces and
//! profiles of the demo data of browser by their ids, and reads no table
//! of another module. In a product without browser (Notes) there are no such
//! workspaces and profiles: the notes are seeded without those bindings
//! ([`bindable`]), not with references nothing can name (spec 10.3).

mod bulk;
mod content;

use crate::binding::BindingKind;
use crate::{insert_note, rebuild_manifest, reindex_links, NewNote, NoteFilter, NotesState};
use chrono::Utc;
use content::{pack, NotesPack, SmartKind};
use std::path::Path;
use uuid::Uuid;
use veydan_core::{AppError, CmdResult, Core};

/// The bindings of a demo note whose kind has an owner in this product; a
/// value (`domain:`) stays. The demo's entities are the owners' demo data,
/// so an entity of a kind nobody here owns does not exist.
pub(super) fn bindable(core: &Core, bindings: impl IntoIterator<Item = String>) -> Vec<String> {
    let owned = core.directory.kinds();
    bindings
        .into_iter()
        .filter(|b| match BindingKind::parse(b) {
            Some((kind, _)) if kind.is_entity() => owned.contains(&kind.name()),
            _ => true,
        })
        .collect()
}

/// Whether the demo's notes may name the workspaces and tools of browser:
/// only where a module of this product owns workspaces (Space).
pub(super) fn names_workspaces(core: &Core) -> bool {
    core.directory.kinds().contains(&"workspace")
}

/// Lines of the handcrafted pack that describe Space's tools, and what a
/// product without them (Notes) shows instead.
const SPACE_ONLY_LINES: &[(&str, &str)] = &[
    (
        "profiles + notes + TOTP + SSH in one vault",
        "notes, files and tasks in one vault",
    ),
    (
        "профили + заметки + TOTP + SSH в одном vault",
        "заметки, файлы и задачи в одном vault",
    ),
    (
        "Search notes + TOTP names + SSH hosts in one box.",
        "Search notes, tags and attachments in one box.",
    ),
    (
        "Искать заметки + имена TOTP + SSH-хосты в одном поле.",
        "Искать заметки, теги и вложения в одном поле.",
    ),
    (
        "Idea: show TOTP badge on kanban cards — already shipped, verify on mobile.",
        "Idea: open the last note on start — check on mobile.",
    ),
    (
        "Идея: бейдж TOTP на kanban — уже есть, проверить на mobile.",
        "Идея: открывать последнюю заметку при запуске — проверить на mobile.",
    ),
    ("- SSH key store\n- TOTP vault\n", "- Note attachments\n- Note templates\n"),
    (
        "- Хранилище SSH-ключей\n- TOTP vault\n",
        "- Вложения в заметках\n- Шаблоны заметок\n",
    ),
];

/// The text of a handcrafted note as this product shows it.
fn product_content(content: &str, space: bool) -> String {
    if space {
        return content.to_string();
    }
    SPACE_ONLY_LINES
        .iter()
        .fold(content.to_string(), |text, (from, to)| text.replace(from, to))
}

/// Tags, folders, notes and smart views.
pub(crate) async fn seed(core: &Core, notes: &NotesState, locale: &str) -> CmdResult<()> {
    let pack = pack(locale);
    let now = Utc::now().to_rfc3339();
    seed_tags(core, &pack, &now).await?;
    seed_folders(core, &pack, &now).await?;
    seed_note_pack(core, notes, &pack, &now).await?;
    seed_smart_views(core, &pack, &now).await?;
    bulk::seed_notes(core, notes, locale, &now).await?;
    rebuild_manifest(&core.db, &core.app_data_dir).await?;
    Ok(())
}

async fn seed_tags(core: &Core, pack: &NotesPack, now: &str) -> CmdResult<()> {
    for tag in &pack.tags {
        // A tag recreated by name (sync, reindex) keeps its id; the demo color still lands.
        sqlx::query(
            "INSERT INTO note_tags (id, name, color, created_at, updated_at) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(name) DO UPDATE SET color = excluded.color, updated_at = excluded.updated_at",
        )
        .bind(tag.id)
        .bind(tag.name)
        .bind(tag.color)
        .bind(now)
        .bind(now)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;
    }
    Ok(())
}

async fn seed_folders(core: &Core, pack: &NotesPack, now: &str) -> CmdResult<()> {
    for f in pack.folders.iter().filter(|f| f.parent_id.is_none()) {
        insert_folder(core, f.id, f.name, None, f.color, now).await?;
    }
    for f in pack.folders.iter().filter(|f| f.parent_id.is_some()) {
        insert_folder(core, f.id, f.name, f.parent_id, f.color, now).await?;
    }
    Ok(())
}

async fn insert_folder(
    core: &Core,
    id: &str,
    name: &str,
    parent_id: Option<&str>,
    color: &str,
    now: &str,
) -> CmdResult<()> {
    sqlx::query(
        "INSERT INTO note_folders (id, name, parent_id, color, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(name)
    .bind(parent_id)
    .bind(color)
    .bind(now)
    .bind(now)
    .execute(&core.db)
    .await
    .map_err(AppError::db)?;
    Ok(())
}

async fn seed_note_pack(
    core: &Core,
    notes: &NotesState,
    pack: &NotesPack,
    now: &str,
) -> CmdResult<()> {
    // Create notes that are wiki targets first (no `[[`), then the rest — insert_note reindexes each time
    let mut ordered: Vec<_> = pack.notes.iter().enumerate().collect();
    ordered.sort_by_key(|(_, n)| n.content.contains("[["));

    let mut created: Vec<(String, String, bool, bool, bool)> = Vec::new();
    let space = names_workspaces(core);

    for (place, n) in ordered {
        let note = insert_note(
            NewNote {
                id: format!("demo-note-{place:02}"),
                title: n.title.to_string(),
                format: "md".into(),
                bindings: bindable(core, n.bindings.iter().map(|s| s.to_string())),
                tags: n.tags.iter().map(|s| s.to_string()).collect(),
                content: product_content(n.content, space),
                created_at: now.to_string(),
                updated_at: now.to_string(),
            },
            core,
            notes,
        )
        .await?;

        if let Some(fid) = n.folder_id {
            sqlx::query(
                "INSERT OR IGNORE INTO note_folder_links (note_id, folder_id) VALUES (?, ?)",
            )
            .bind(&note.id)
            .bind(fid)
            .execute(&core.db)
            .await
            .map_err(AppError::db)?;
        }

        created.push((
            note.id,
            note.content.unwrap_or_default(),
            n.pinned,
            n.archived,
            n.deleted,
        ));
    }

    // Reindex wiki links now that all titles exist
    for (id, content, _, _, _) in &created {
        if content.contains("[[") {
            reindex_links(id, content, &core.db).await?;
        }
    }

    for (id, _, pinned, archived, deleted) in &created {
        if *pinned {
            sqlx::query("UPDATE notes SET pinned = 1 WHERE id = ?")
                .bind(id)
                .execute(&core.db)
                .await
                .map_err(AppError::db)?;
        }
        if *archived {
            sqlx::query("UPDATE notes SET archived = 1 WHERE id = ?")
                .bind(id)
                .execute(&core.db)
                .await
                .map_err(AppError::db)?;
        }
        if *deleted {
            // Match soft_delete: mark trash and drop FTS row
            let fts: Option<(Option<i64>,)> =
                sqlx::query_as("SELECT fts_rowid FROM notes WHERE id = ?")
                    .bind(id)
                    .fetch_optional(&core.db)
                    .await
                    .map_err(AppError::db)?;
            sqlx::query("UPDATE notes SET deleted = 1, updated_at = ? WHERE id = ?")
                .bind(now)
                .bind(id)
                .execute(&core.db)
                .await
                .map_err(AppError::db)?;
            if let Some((Some(rowid),)) = fts {
                let _ = sqlx::query("DELETE FROM notes_fts WHERE rowid = ?")
                    .bind(rowid)
                    .execute(&core.db)
                    .await;
            }
        }
    }

    Ok(())
}

async fn seed_smart_views(core: &Core, pack: &NotesPack, now: &str) -> CmdResult<()> {
    for (i, v) in pack.smart_views.iter().enumerate() {
        let conditions = match v.kind {
            SmartKind::OpenTasks => NoteFilter {
                has_open_tasks: Some(true),
                archived: Some(false),
                ..Default::default()
            },
            SmartKind::Pinned => NoteFilter {
                pinned: Some(true),
                archived: Some(false),
                ..Default::default()
            },
            SmartKind::Recent7d => NoteFilter {
                updated_within_days: Some(7),
                archived: Some(false),
                ..Default::default()
            },
            SmartKind::HasAttachments => NoteFilter {
                has_attachments: Some(true),
                archived: Some(false),
                ..Default::default()
            },
        };
        let json = serde_json::to_string(&conditions).map_err(AppError::other)?;
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO note_smart_views (id, name, color, conditions, sort_order, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(v.name)
        .bind(v.color)
        .bind(&json)
        .bind(i as i64)
        .bind(now)
        .bind(now)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;
    }
    Ok(())
}

/// The notes graph and the files of notes.
pub(crate) async fn clear(core: &Core) -> CmdResult<()> {
    let _ = sqlx::query("DELETE FROM note_links")
        .execute(&core.db)
        .await;
    let _ = sqlx::query("DELETE FROM note_mentions")
        .execute(&core.db)
        .await;
    let _ = sqlx::query("DELETE FROM note_tag_links")
        .execute(&core.db)
        .await;
    let _ = sqlx::query("DELETE FROM note_folder_links")
        .execute(&core.db)
        .await;
    let _ = sqlx::query("UPDATE note_history SET parent_id = NULL")
        .execute(&core.db)
        .await;
    let _ = sqlx::query("DELETE FROM note_history")
        .execute(&core.db)
        .await;
    let _ = sqlx::query("DELETE FROM notes_fts").execute(&core.db).await;
    let _ = sqlx::query("DELETE FROM notes").execute(&core.db).await;
    let _ = sqlx::query("DELETE FROM note_smart_views")
        .execute(&core.db)
        .await;
    let _ = sqlx::query("DELETE FROM note_tags").execute(&core.db).await;
    let _ = sqlx::query("DELETE FROM note_folders")
        .execute(&core.db)
        .await;

    clear_notes_dirs(&core.app_data_dir)?;

    rebuild_manifest(&core.db, &core.app_data_dir).await.ok();
    Ok(())
}

fn clear_notes_dirs(app_data_dir: &Path) -> CmdResult<()> {
    let notes = app_data_dir.join("notes");
    for sub in ["documents", "attachments", "drafts"] {
        let dir = notes.join(sub);
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
        std::fs::create_dir_all(&dir).map_err(AppError::io)?;
    }
    let manifest = notes.join("notes_manifest.json");
    if manifest.exists() {
        let _ = std::fs::remove_file(manifest);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{bindable, content::pack, product_content, SPACE_ONLY_LINES};
    use crate::testing::States;

    /// Every line Notes rewrites is in the pack (a reworded pack would leave
    /// Space's tools in Notes unnoticed), and Space keeps its text.
    #[test]
    fn notes_alone_shows_no_line_about_space_tools() {
        let texts: Vec<&str> = ["en", "ru"]
            .iter()
            .flat_map(|l| pack(l).notes.into_iter().map(|n| n.content))
            .collect();
        for (from, _) in SPACE_ONLY_LINES {
            assert!(texts.iter().any(|t| t.contains(from)), "not in the pack: {from}");
        }
        for text in texts {
            assert_eq!(product_content(text, true), text);
            let notes = product_content(text, false);
            for (from, _) in SPACE_ONLY_LINES {
                assert!(!notes.contains(from));
            }
        }
    }

    /// Notes alone own no workspaces or profiles: the demo's notes are not
    /// bound to the demo workspaces of browser, which are not there; a site
    /// stays.
    #[tokio::test]
    async fn a_product_without_browser_seeds_no_workspace_bindings() {
        let states = States::new().await;
        let bindings = [
            "workspace:demo-ws-dev",
            "profile:demo-pr-gh",
            "ssh:demo-ssh-1",
            "domain:github.com",
        ]
        .map(String::from);
        assert_eq!(bindable(&states.core, bindings), ["domain:github.com"]);
    }
}
