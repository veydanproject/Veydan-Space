// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Local-filesystem commands for the two-panel file browser.
//!
//! Deliberately mirrors the shape of `commands::sftp` (same `FileEntry`
//! model) so the frontend treats a local panel and a remote panel uniformly.

use crate::error::{AppError, CmdResult};
use crate::models::{format_octal, format_permissions, FileEntry};
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

fn entry_for(path: &Path) -> Option<FileEntry> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    // symlink_metadata: don't follow, so broken links still list
    let md = std::fs::symlink_metadata(path).ok()?;
    let is_symlink = md.file_type().is_symlink();
    // For symlinks report the *target*'s kind so navigation works; a broken
    // link falls back to the link's own metadata.
    let target_md = if is_symlink {
        std::fs::metadata(path).unwrap_or(md.clone())
    } else {
        md.clone()
    };

    #[cfg(unix)]
    let (mode, owner, group) = (
        md.mode(),
        Some(md.uid().to_string()),
        Some(md.gid().to_string()),
    );
    #[cfg(not(unix))]
    let (mode, owner, group): (u32, Option<String>, Option<String>) = (0, None, None);

    let mtime = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64);

    Some(FileEntry {
        name,
        path: path.to_string_lossy().into_owned(),
        is_dir: target_md.is_dir(),
        is_symlink,
        size: md.len(),
        mtime,
        mode,
        permissions: format_permissions(mode),
        octal: format_octal(mode),
        owner,
        group,
    })
}

#[tauri::command]
pub async fn fs_home() -> CmdResult<String> {
    // HOME on unix; Windows sets USERPROFILE instead.
    ["HOME", "USERPROFILE"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|h| !h.is_empty()))
        .ok_or_else(|| AppError::other("Neither HOME nor USERPROFILE is set"))
}

/// Stat a single local path; `None` if it doesn't exist. Used for
/// conflict detection before transfers.
#[tauri::command]
pub async fn fs_stat(path: String) -> CmdResult<Option<FileEntry>> {
    tokio::task::spawn_blocking(move || Ok(entry_for(Path::new(&path))))
        .await
        .map_err(AppError::other)?
}

#[tauri::command]
pub async fn fs_list(path: String) -> CmdResult<Vec<FileEntry>> {
    tokio::task::spawn_blocking(move || {
        let dir = Path::new(&path);
        let read = std::fs::read_dir(dir).map_err(AppError::io)?;
        let mut entries: Vec<FileEntry> = read
            .filter_map(|r| r.ok())
            .filter_map(|e| entry_for(&e.path()))
            .collect();
        entries.sort_by_key(|e| e.name.to_lowercase());
        Ok(entries)
    })
    .await
    .map_err(AppError::other)?
}

// ── Mutating operations ────────────────────────────────────────────────────────

#[tauri::command]
pub async fn fs_mkdir(path: String) -> CmdResult<()> {
    tokio::fs::create_dir(&path).await.map_err(AppError::io)
}

#[tauri::command]
pub async fn fs_create_file(path: String) -> CmdResult<()> {
    // create_new: fail if the file already exists rather than truncating it
    tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .await
        .map(|_| ())
        .map_err(AppError::io)
}

#[tauri::command]
pub async fn fs_rename(from: String, to: String) -> CmdResult<()> {
    if tokio::fs::symlink_metadata(&to).await.is_ok() {
        return Err(AppError::other(format!("Target already exists: {to}")));
    }
    tokio::fs::rename(&from, &to).await.map_err(AppError::io)
}

#[tauri::command]
pub async fn fs_delete(path: String) -> CmdResult<()> {
    tokio::task::spawn_blocking(move || {
        let p = Path::new(&path);
        // symlink_metadata: never follow — remove the link itself
        let md = std::fs::symlink_metadata(p).map_err(AppError::io)?;
        if md.is_dir() {
            std::fs::remove_dir_all(p).map_err(AppError::io)
        } else {
            std::fs::remove_file(p).map_err(AppError::io)
        }
    })
    .await
    .map_err(AppError::other)?
}

#[tauri::command]
pub async fn fs_chmod(path: String, mode: u32) -> CmdResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode & 0o7777))
            .await
            .map_err(AppError::io)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Err(AppError::other("chmod is not supported on this platform"))
    }
}
