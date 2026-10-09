// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Backend evaluation of `NoteFilter` against note rows.
//! Everything that needs extra data (tags, folders, content, attachments) is
//! loaded once into `FilterContext`, then `matches` is a pure check per row.

use super::attachments::attachments_dir_for;
use super::binding::is_scope_binding;
use super::files::resolve_note_abs_path;
use super::models::{NoteFilter, NoteFolder, NoteRow, NoteTagInfo};
use super::tags::{fetch_all_note_folder_ids_map, fetch_all_note_tags_map, normalize_tag_name};
use std::collections::{HashMap, HashSet};
use veydan_core::AppError;
use veydan_core::Core;

pub(crate) struct FilterContext {
    pub tags: HashMap<String, Vec<NoteTagInfo>>,
    pub folders: HashMap<String, Vec<String>>,
    /// Folder ids matched by `filter.folder_id` including descendants
    folder_scope: Option<HashSet<String>>,
    /// Ids of notes with an unchecked task, loaded only when requested
    open_tasks: Option<HashSet<String>>,
    updated_after: Option<String>,
    app_data_dir: std::path::PathBuf,
}

/// Folder plus all descendants.
pub(crate) fn folder_descendants(root: &str, folders: &[NoteFolder]) -> HashSet<String> {
    let mut out = HashSet::from([root.to_string()]);
    let mut stack = vec![root.to_string()];
    while let Some(cur) = stack.pop() {
        for f in folders
            .iter()
            .filter(|f| f.parent_id.as_deref() == Some(cur.as_str()))
        {
            if out.insert(f.id.clone()) {
                stack.push(f.id.clone());
            }
        }
    }
    out
}

impl FilterContext {
    pub(crate) async fn load(filter: &NoteFilter, core: &Core) -> Result<Self, AppError> {
        let tags = fetch_all_note_tags_map(&core.db).await?;
        let folders = fetch_all_note_folder_ids_map(&core.db).await?;

        let folder_scope = match &filter.folder_id {
            Some(id) => {
                let all = sqlx::query_as::<_, NoteFolder>("SELECT * FROM note_folders")
                    .fetch_all(&core.db)
                    .await
                    .map_err(AppError::db)?;
                Some(folder_descendants(id, &all))
            }
            None => None,
        };

        let open_tasks = if filter.has_open_tasks.is_some() {
            let ids: Vec<(String,)> = sqlx::query_as(
                "SELECT note_id FROM notes_fts WHERE content LIKE '%- [ ]%' OR content LIKE '%* [ ]%'",
            )
            .fetch_all(&core.db)
            .await
            .map_err(AppError::db)?;
            Some(ids.into_iter().map(|(id,)| id).collect())
        } else {
            None
        };

        let updated_after = filter
            .updated_within_days
            .map(|days| (chrono::Utc::now() - chrono::Duration::days(days.max(0))).to_rfc3339());

        Ok(Self {
            tags,
            folders,
            folder_scope,
            open_tasks,
            updated_after,
            app_data_dir: core.app_data_dir.clone(),
        })
    }

    fn has_attachments(&self, row: &NoteRow) -> bool {
        let note_file = resolve_note_abs_path(&self.app_data_dir, &row.file_path);
        std::fs::read_dir(attachments_dir_for(&note_file, &row.id))
            .map(|mut d| d.next().is_some())
            .unwrap_or(false)
    }

    pub(crate) fn matches(&self, row: &NoteRow, filter: &NoteFilter) -> bool {
        if (row.deleted != 0) != filter.deleted.unwrap_or(false) {
            return false;
        }
        if let Some(archived) = filter.archived {
            if (row.archived != 0) != archived {
                return false;
            }
        }
        if let Some(pinned) = filter.pinned {
            if (row.pinned != 0) != pinned {
                return false;
            }
        }

        let bindings: Vec<String> = serde_json::from_str(&row.bindings).unwrap_or_default();
        if let Some(b) = &filter.binding {
            if !bindings.contains(b) {
                return false;
            }
        }
        if let Some(any) = &filter.bindings_any {
            if !any.is_empty() && !any.iter().any(|b| bindings.contains(b)) {
                return false;
            }
        }
        if filter.global_only == Some(true) && bindings.iter().any(|b| is_scope_binding(b)) {
            return false;
        }

        let empty = Vec::new();
        let tags = self.tags.get(&row.id).unwrap_or(&empty);
        // A saved view may name a tag with the capitals it had before 5.1.12.
        let has = |name: &str| {
            let name = normalize_tag_name(name);
            tags.iter().any(|t| t.name == name)
        };
        if let Some(name) = &filter.tag_name {
            if !has(name) {
                return false;
            }
        }
        if let Some(prefix) = &filter.tag_prefix {
            let prefix = normalize_tag_name(prefix);
            let sub = format!("{prefix}/");
            if !tags
                .iter()
                .any(|t| t.name == prefix || t.name.starts_with(&sub))
            {
                return false;
            }
        }
        if let Some(any) = &filter.tags_any {
            if !any.is_empty() && !any.iter().any(|n| has(n)) {
                return false;
            }
        }
        if let Some(all) = &filter.tags_all {
            if !all.iter().all(|n| has(n)) {
                return false;
            }
        }

        if let Some(scope) = &self.folder_scope {
            let folders = self.folders.get(&row.id);
            if !folders
                .map(|v| v.iter().any(|f| scope.contains(f)))
                .unwrap_or(false)
            {
                return false;
            }
        }

        if let Some(after) = &self.updated_after {
            if row.updated_at.as_str() < after.as_str() {
                return false;
            }
        }
        if let Some(want) = filter.has_open_tasks {
            let has = self
                .open_tasks
                .as_ref()
                .map(|s| s.contains(&row.id))
                .unwrap_or(false);
            if has != want {
                return false;
            }
        }
        if let Some(want) = filter.has_attachments {
            if self.has_attachments(row) != want {
                return false;
            }
        }
        true
    }
}
