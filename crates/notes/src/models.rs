// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

// ── Models ────────────────────────────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Debug, Clone, FromRow)]
pub(crate) struct NoteRow {
    pub id: String,
    pub title: String,
    pub file_path: String,
    pub format: String,
    pub pinned: i64,
    pub archived: i64,
    pub deleted: i64,
    pub doc_status: String,
    pub version_base: Option<String>,
    pub fts_rowid: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
    pub file_mtime: Option<String>,
    pub content_hash: Option<String>,
    pub preview: String,
    /// JSON array of binding strings, e.g. ["workspace:id", "profile:id"]
    pub bindings: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub file_path: String,
    pub format: String,
    pub bindings: Vec<String>,
    pub tags: Vec<NoteTagInfo>,
    pub folder_ids: Vec<String>,
    pub pinned: bool,
    pub archived: bool,
    pub deleted: bool,
    pub doc_status: String,
    pub created_at: String,
    pub updated_at: String,
    pub content_hash: Option<String>,
    pub content: Option<String>,
    pub has_draft: bool,
    /// Absolute dir of the note file; relative attachment links resolve against it
    pub base_dir: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct NoteListItem {
    pub id: String,
    pub title: String,
    pub format: String,
    pub bindings: Vec<String>,
    pub tags: Vec<NoteTagInfo>,
    pub folder_ids: Vec<String>,
    pub pinned: bool,
    pub archived: bool,
    pub deleted: bool,
    pub doc_status: String,
    pub created_at: String,
    pub updated_at: String,
    pub has_draft: bool,
    pub preview: String,
    pub snippet: Option<String>,
}

#[derive(Debug, Serialize, Clone, FromRow)]
pub struct NoteFolder {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub color: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, FromRow)]
pub struct NoteTagInfo {
    pub id: String,
    pub name: String,
    pub color: String,
}

#[derive(Debug, Serialize, Clone, FromRow)]
pub struct NoteTag {
    pub id: String,
    pub name: String,
    pub color: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct NoteCreateInput {
    pub title: String,
    pub format: Option<String>,
    /// Context bindings, e.g. ["workspace:id", "profile:id"]
    pub bindings: Option<Vec<String>>,
    pub tag_names: Option<Vec<String>>,
    pub content: Option<String>,
    /// Note from the Templates folder whose rendered body becomes the content
    pub template_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct NoteUpdateInput {
    pub title: Option<String>,
    pub content: Option<String>,
    pub pinned: Option<bool>,
    /// Hash of the body the editor last loaded. Checked only together with `content`.
    #[serde(default)]
    pub base_hash: Option<String>,
}

/// List filter; also the persisted condition set of a smart view.
#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct NoteFilter {
    /// Filter notes that contain this binding, e.g. "workspace:id" or "profile:id"
    pub binding: Option<String>,
    pub tag_name: Option<String>,
    /// Folder and all of its descendants
    pub folder_id: Option<String>,
    pub pinned: Option<bool>,
    /// None = any, Some(x) = only notes with archived == x
    pub archived: Option<bool>,
    /// None/Some(false) = live notes, Some(true) = trash only
    pub deleted: Option<bool>,
    /// Tag `name` or any `name/...` sub-tag
    pub tag_prefix: Option<String>,
    /// Notes without workspace/profile bindings
    pub global_only: Option<bool>,
    /// At least one of these bindings
    pub bindings_any: Option<Vec<String>>,
    /// At least one of these tags
    pub tags_any: Option<Vec<String>>,
    /// All of these tags
    pub tags_all: Option<Vec<String>>,
    /// updated_at within the last N days
    pub updated_within_days: Option<i64>,
    pub has_attachments: Option<bool>,
    /// Body contains an unchecked `[ ]` task
    pub has_open_tasks: Option<bool>,
}

#[derive(Debug, Serialize, Clone, FromRow)]
pub(crate) struct SmartViewRow {
    pub id: String,
    pub name: String,
    pub color: String,
    pub conditions: String,
    pub sort_order: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct NoteSmartView {
    pub id: String,
    pub name: String,
    pub color: String,
    pub conditions: NoteFilter,
    pub sort_order: i64,
}

#[derive(Debug, Deserialize)]
pub struct SmartViewInput {
    pub name: String,
    pub color: Option<String>,
    pub conditions: NoteFilter,
}
