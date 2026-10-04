// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

#[cfg(desktop)]
use super::attachments::allow_asset_dir;
#[cfg(desktop)]
use super::files::*;
#[cfg(desktop)]
use super::index::{start_notes_watcher, sync_notes_index};
#[cfg(desktop)]
use super::NotesState;
use serde::{Deserialize, Serialize};
#[cfg(desktop)]
use std::path::PathBuf;
#[cfg(desktop)]
use tauri::Manager;
use veydan_core::{settings, Core};
use veydan_core::{AppError, CmdResult};

// ── Attachment policy: when a note attachment becomes a large file ───────────

const POLICY_KEY: &str = "notes_attachment_policy";
const MAX_THRESHOLD_MIB: u64 = 4096;
const MAX_FILE_GIB: u64 = 1024;
const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

/// Notes-domain policy; the large-files module never reads it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteAttachmentPolicy {
    /// Off: every attachment stays a v1 blob, whatever its size.
    pub large_files_enabled: bool,
    /// Files at or above this size sync as v2 large files.
    pub threshold_mib: u64,
    /// Product limit for one attachment; 0 = no limit (backend quota still applies).
    pub max_file_gib: u64,
    /// Off: chunked attachments at or above `ask_above_mib` stay in the vault until requested.
    #[serde(default = "default_true")]
    pub download_on_sync: bool,
    /// Size from which a missing attachment prompts for download when the note opens.
    #[serde(default = "default_ask_above_mib")]
    pub ask_above_mib: u64,
}

fn default_true() -> bool {
    true
}

fn default_ask_above_mib() -> u64 {
    16
}

impl Default for NoteAttachmentPolicy {
    fn default() -> Self {
        Self {
            large_files_enabled: true,
            threshold_mib: 16,
            max_file_gib: 10,
            download_on_sync: true,
            ask_above_mib: default_ask_above_mib(),
        }
    }
}

impl NoteAttachmentPolicy {
    pub fn validate(&self) -> CmdResult<()> {
        if !(1..=MAX_THRESHOLD_MIB).contains(&self.threshold_mib) {
            return Err(AppError::other(format!(
                "threshold must be 1..{MAX_THRESHOLD_MIB} MiB"
            )));
        }
        if self.max_file_gib > MAX_FILE_GIB {
            return Err(AppError::other(format!(
                "max file size must be 0..{MAX_FILE_GIB} GiB"
            )));
        }
        if !(1..=MAX_THRESHOLD_MIB).contains(&self.ask_above_mib) {
            return Err(AppError::other(format!(
                "download prompt threshold must be 1..{MAX_THRESHOLD_MIB} MiB"
            )));
        }
        Ok(())
    }

    pub fn uses_large_files(&self, size: u64) -> bool {
        self.large_files_enabled && size >= self.threshold_mib * MIB
    }

    /// Whether a remote chunked attachment of `size` is fetched during sync.
    pub fn downloads_on_sync(&self, size: u64) -> bool {
        self.download_on_sync || size < self.ask_above_mib * MIB
    }

    /// Product limit in bytes; `None` when unlimited.
    pub fn max_file_bytes(&self) -> Option<u64> {
        (self.max_file_gib > 0).then(|| self.max_file_gib * GIB)
    }

    /// Product limit check; `None` size (unknown source length) passes.
    pub fn check_size(&self, size: Option<u64>) -> CmdResult<()> {
        match (size, self.max_file_bytes()) {
            (Some(n), Some(max)) if n > max => Err(AppError::other(format!(
                "attachment exceeds the {} GiB limit",
                self.max_file_gib
            ))),
            _ => Ok(()),
        }
    }
}

pub(crate) async fn load_attachment_policy(db: &sqlx::Pool<sqlx::Sqlite>) -> NoteAttachmentPolicy {
    settings::get_json(db, POLICY_KEY).await.unwrap_or_default()
}

#[tauri::command]
pub async fn notes_attachment_policy_get(
    core: tauri::State<'_, Core>,
) -> CmdResult<NoteAttachmentPolicy> {
    Ok(load_attachment_policy(&core.db).await)
}

#[tauri::command]
pub async fn notes_attachment_policy_set(
    policy: NoteAttachmentPolicy,
    core: tauri::State<'_, Core>,
) -> CmdResult<NoteAttachmentPolicy> {
    policy.validate()?;
    settings::set_json(&core.db, POLICY_KEY, &policy).await?;
    Ok(policy)
}

// ── Notes directory settings ──────────────────────────────────────────────────

#[cfg(desktop)]
#[derive(Debug, Serialize)]
pub struct NotesDirInfo {
    pub current: String,
    pub is_custom: bool,
}

#[cfg(desktop)]
#[tauri::command]
pub async fn notes_get_dir(
    core: tauri::State<'_, Core>,
    notes: tauri::State<'_, NotesState>,
) -> CmdResult<NotesDirInfo> {
    let custom = notes.custom_dir.read().ok().and_then(|g| g.clone());
    let (current, is_custom) = if let Some(ref p) = custom {
        (p.to_string_lossy().to_string(), true)
    } else {
        (
            documents_dir(&core.app_data_dir)
                .to_string_lossy()
                .to_string(),
            false,
        )
    };
    Ok(NotesDirInfo { current, is_custom })
}

#[cfg(desktop)]
#[tauri::command]
pub async fn notes_set_dir(
    path: Option<String>,
    app: tauri::AppHandle,
    core: tauri::State<'_, Core>,
    notes: tauri::State<'_, NotesState>,
) -> CmdResult<NotesDirInfo> {
    let new_custom: Option<PathBuf> = path.as_deref().and_then(|p| {
        let trimmed = p.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(PathBuf::from(trimmed))
        }
    });

    if let Some(ref p) = new_custom {
        std::fs::create_dir_all(p).map_err(AppError::io)?;
    }

    // Persist to DB
    let stored = new_custom.as_ref().map(|p| p.to_string_lossy());
    settings::set_opt(&core.db, "notes_custom_dir", stored.as_deref()).await?;

    // Update in-memory state
    if let Ok(mut lock) = notes.custom_dir.write() {
        *lock = new_custom.clone();
    }

    // Re-point the file watcher and index whatever already lives in the new dir.
    if let Some(ref custom) = new_custom {
        allow_asset_dir(&app, custom);
    }
    // A module that is switched off has no watcher, and gets none here; one
    // that is on gets one also when its last watcher could not be made.
    if app
        .state::<veydan_shell::Shell>()
        .module_enabled(crate::MODULE_ID)
    {
        if let Ok(mut slot) = notes.watcher.lock() {
            *slot = start_notes_watcher(app, core.app_data_dir.clone(), new_custom.clone());
        }
    }
    sync_notes_index(&core, new_custom.as_ref()).await?;

    let (current, is_custom) = if let Some(ref p) = new_custom {
        (p.to_string_lossy().to_string(), true)
    } else {
        (
            documents_dir(&core.app_data_dir)
                .to_string_lossy()
                .to_string(),
            false,
        )
    };
    Ok(NotesDirInfo { current, is_custom })
}
