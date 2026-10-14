// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Note attachments: files stored next to the note as `attachments/{note_id}/{name}`
//! and referenced from Markdown by that relative path.

use super::files::*;
use super::settings::load_attachment_policy;
use super::NotesState;
use serde::Serialize;
use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use tauri::Manager;
use veydan_core::Core;
use veydan_core::{AppError, CmdResult};
use veydan_sync::{available_space, PathSink};
use veydan_sync_host::SyncManager;

const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "avif"];
/// In-progress local copy; never listed as an attachment.
const PART_SUFFIX: &str = ".part";
/// Free space that must remain after a copy.
const DISK_RESERVE: u64 = 64 * 1024 * 1024;

/// Files the attachment directory holds that are not attachments.
pub(crate) fn is_staging_name(name: &str) -> bool {
    name.ends_with(PART_SUFFIX) || name.ends_with(".tmp") || PathSink::is_staging_file(name)
}

#[derive(Debug, Clone, Serialize)]
pub struct NoteAttachment {
    pub name: String,
    /// Path relative to the note file, ready for a Markdown link
    pub rel_path: String,
    pub size: u64,
    pub is_image: bool,
    /// False for a chunked file that is still only in the vault.
    pub present: bool,
}

#[cfg(desktop)]
#[derive(Debug, Clone, Serialize)]
pub struct OrphanAttachment {
    pub note_id: String,
    pub name: String,
    pub size: u64,
}

// ── Paths ─────────────────────────────────────────────────────────────────────

/// `attachments/{note_id}` next to the note file.
pub(crate) fn attachments_dir_for(note_file: &Path, note_id: &str) -> PathBuf {
    note_file
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default()
        .join("attachments")
        .join(note_id)
}

/// Remove the attachments of a note: only a note id names a folder, and only
/// a folder inside `attachments/` (not it, and not one a link leads out of).
pub(crate) fn remove_note_attachments(note_file: &Path, note_id: &str) {
    if !valid_id(note_id) {
        return;
    }
    let dir = attachments_dir_for(note_file, note_id);
    let Some(root) = dir.parent() else {
        return;
    };
    let (Ok(dir), Ok(root)) = (dir.canonicalize(), root.canonicalize()) else {
        return;
    };
    if dir != root && dir.starts_with(&root) {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Let the webview load files from this documents dir via the asset protocol.
#[cfg(desktop)]
pub fn allow_asset_dir(app: &tauri::AppHandle, docs_dir: &Path) {
    if let Err(e) = app.asset_protocol_scope().allow_directory(docs_dir, true) {
        eprintln!("notes: asset scope for {} failed: {e}", docs_dir.display());
    }
}

async fn note_file_path(note_id: &str, core: &Core) -> Result<PathBuf, AppError> {
    let (file_path,): (String,) = sqlx::query_as("SELECT file_path FROM notes WHERE id = ?")
        .bind(note_id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found(format!("Note {note_id}")))?;
    Ok(resolve_note_abs_path(&core.app_data_dir, &file_path))
}

fn is_image(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Strip directory components and characters unsafe for file names.
pub(crate) fn safe_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.');
    if trimmed.is_empty() {
        "file".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Append `-1`, `-2`, … before the extension until the name is free: no file
/// on disk and none of `reserved`, the names of the note's attachments that
/// are still only in the vault (a file of such a name would take their place).
pub(crate) fn unique_path(dir: &Path, name: &str, reserved: &HashSet<String>) -> PathBuf {
    let free = |n: &str| !reserved.contains(n) && !dir.join(n).exists();
    if free(name) {
        return dir.join(name);
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    (1..)
        .map(|i| format!("{stem}-{i}{ext}"))
        .find(|n| free(n))
        .map(|n| dir.join(n))
        .expect("unbounded iterator")
}

fn to_attachment(note_id: &str, path: &Path) -> Option<NoteAttachment> {
    let name = path.file_name()?.to_str()?.to_string();
    let size = std::fs::metadata(path).ok()?.len();
    Some(NoteAttachment {
        rel_path: format!("attachments/{note_id}/{name}"),
        is_image: is_image(&name),
        name,
        size,
        present: true,
    })
}

fn deferred_attachment(note_id: &str, name: String, size: u64) -> NoteAttachment {
    NoteAttachment {
        rel_path: format!("attachments/{note_id}/{name}"),
        is_image: is_image(&name),
        name,
        size,
        present: false,
    }
}

/// Store bytes as a new attachment of a note just made (capture, import):
/// none of its files is in the vault yet, so no name is reserved.
#[cfg(desktop)]
pub(crate) fn store_attachment(
    note_file: &Path,
    note_id: &str,
    file_name: &str,
    data: &[u8],
) -> Result<NoteAttachment, AppError> {
    store_attachment_stream(
        note_file,
        note_id,
        file_name,
        &HashSet::new(),
        Some(data.len() as u64),
        None,
        &mut &data[..],
    )
}

/// Stream `reader` into `attachments/{note_id}/{name}` through a `.part` file
/// that is renamed only when the copy completed. `max` is the product limit;
/// `reserved` are names the new file must not take (see `unique_path`).
pub(crate) fn store_attachment_stream(
    note_file: &Path,
    note_id: &str,
    file_name: &str,
    reserved: &HashSet<String>,
    len_hint: Option<u64>,
    max: Option<u64>,
    reader: &mut dyn Read,
) -> Result<NoteAttachment, AppError> {
    let dir = attachments_dir_for(note_file, note_id);
    std::fs::create_dir_all(&dir).map_err(AppError::io)?;
    if let (Some(need), Some(free)) = (len_hint, available_space(&dir)) {
        if free < need.saturating_add(DISK_RESERVE) {
            return Err(AppError::io(format!(
                "not enough free space: need {need} bytes, {free} available"
            )));
        }
    }
    let dest = unique_path(&dir, &safe_file_name(file_name), reserved);
    let part = PathBuf::from(format!("{}{PART_SUFFIX}", dest.display()));
    let copied = (|| -> Result<u64, AppError> {
        let mut out = std::io::BufWriter::with_capacity(
            1 << 20,
            std::fs::File::create(&part).map_err(AppError::io)?,
        );
        let limit = max.map(|m| m + 1).unwrap_or(u64::MAX);
        let n = std::io::copy(&mut reader.take(limit), &mut out).map_err(AppError::io)?;
        if max.is_some_and(|m| n > m) {
            return Err(AppError::other(format!(
                "attachment exceeds the {} byte limit",
                max.unwrap_or(0)
            )));
        }
        out.into_inner()
            .map_err(|e| AppError::io(e.into_error()))?
            .sync_all()
            .map_err(AppError::io)?;
        Ok(n)
    })();
    if let Err(e) = copied {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    std::fs::rename(&part, &dest).map_err(AppError::io)?;
    to_attachment(note_id, &dest).ok_or_else(|| AppError::io("attachment write failed"))
}

/// Writable stream for a `content://` URI picked in the system save dialog.
#[cfg(target_os = "android")]
fn content_uri_writer(app: &tauri::AppHandle, uri: &str) -> CmdResult<std::fs::File> {
    use tauri_plugin_fs::{FilePath, FsExt, OpenOptions};
    let url = tauri::Url::parse(uri).map_err(AppError::other)?;
    let mut opts = OpenOptions::new();
    opts.write(true).truncate(true);
    app.fs()
        .open(FilePath::Url(url), opts)
        .map_err(AppError::io)
}

/// Decode the percent-encoded ASCII header value used for file names.
#[cfg(desktop)]
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(desktop)]
fn is_link_boundary(c: char) -> bool {
    matches!(c, ')' | ' ' | '"' | '\'' | '\n' | '>' | ']')
}

/// `encodeURIComponent`, which leaves `'` `(` `)` raw.
#[cfg(desktop)]
fn encode_uri_component(name: &str) -> String {
    encode_path_name(name, true)
}

/// Same encoding, but `'` `(` `)` are percent-encoded so the link can end after the name.
#[cfg(desktop)]
fn encode_attachment_name(name: &str) -> String {
    encode_path_name(name, false)
}

#[cfg(desktop)]
fn encode_path_name(name: &str, keep_parens: bool) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        let plain = ch.is_ascii_alphanumeric()
            || matches!(ch, '-' | '_' | '.' | '!' | '~' | '*')
            || (keep_parens && matches!(ch, '\'' | '(' | ')'));
        if plain {
            out.push(ch);
            continue;
        }
        let mut buf = [0; 4];
        for b in ch.encode_utf8(&mut buf).bytes() {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// True when `name` appears as a whole path segment after `attachments/{id}/`.
#[cfg(desktop)]
fn body_references(body: &str, note_id: &str, name: &str) -> bool {
    let prefix = format!("attachments/{note_id}/");
    let needles = [
        format!("{prefix}{name}"),
        format!("{prefix}{}", encode_uri_component(name)),
        format!("{prefix}{}", encode_attachment_name(name)),
    ];
    needles.iter().any(|needle| contains_bounded(body, needle))
}

#[cfg(desktop)]
fn contains_bounded(body: &str, needle: &str) -> bool {
    let mut start = 0;
    while let Some(rel) = body[start..].find(needle) {
        let at = start + rel;
        let after = at + needle.len();
        let boundary = body[after..].chars().next();
        if boundary.map(is_link_boundary).unwrap_or(true) {
            return true;
        }
        start = at + 1;
    }
    false
}

// ── Commands ──────────────────────────────────────────────────────────────────

/// Raw-body upload: bytes in the request body, `x-note-id` and `x-file-name`
/// (percent-encoded) in headers.
#[cfg(desktop)]
#[tauri::command]
pub async fn note_attachment_add(
    request: tauri::ipc::Request<'_>,
    core: tauri::State<'_, Core>,
) -> CmdResult<NoteAttachment> {
    let header = |name: &str| -> Result<String, AppError> {
        request
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .ok_or_else(|| AppError::other(format!("missing header {name}")))
    };
    let note_id = header("x-note-id")?;
    let file_name = percent_decode(&header("x-file-name")?);
    let tauri::ipc::InvokeBody::Raw(data) = request.body() else {
        return Err(AppError::other("attachment body must be raw bytes"));
    };
    let note_file = note_file_path(&note_id, &core).await?;
    let reserved = crate::sync::attachments::deferred_names(&core, &note_id).await?;
    store_attachment_stream(
        &note_file,
        &note_id,
        &file_name,
        &reserved,
        Some(data.len() as u64),
        None,
        &mut &data[..],
    )
}

/// File bytes as base64 for webviews without the asset protocol.
#[tauri::command]
pub async fn note_attachment_read(
    note_id: String,
    name: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<String> {
    use base64::Engine;
    let note_file = note_file_path(&note_id, &core).await?;
    let path = attachments_dir_for(&note_file, &note_id).join(safe_file_name(&name));
    if !path.is_file() {
        return Err(AppError::not_found(name));
    }
    let bytes = std::fs::read(&path).map_err(AppError::io)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Local file paths currently held by the OS clipboard (files copied in a file manager).
/// They are what the user copied, as a picked file is what the user picked: each
/// file is let into the bound of the shell's `open_source`, which
/// `note_attachment_add_from_path` reads through.
#[cfg(desktop)]
#[tauri::command]
pub async fn clipboard_file_paths(app: tauri::AppHandle) -> CmdResult<Vec<String>> {
    let paths = tokio::task::spawn_blocking(|| {
        let mut cb = arboard::Clipboard::new().map_err(AppError::io)?;
        Ok::<_, AppError>(cb.get().file_list().unwrap_or_default())
    })
    .await
    .map_err(AppError::io)??;
    // uri-list entries are CRLF-terminated; arboard leaves the '\r' on the path
    let paths: Vec<String> = paths
        .into_iter()
        .map(|p| p.to_string_lossy().trim().to_owned())
        .filter(|p| !p.is_empty())
        .collect();
    let shell = app.state::<veydan_shell::Shell>();
    for path in &paths {
        if Path::new(path).is_file() {
            shell.allow_source(Path::new(path));
        }
    }
    Ok(paths)
}

/// Copy an existing file (picker, drag-and-drop, Android content URI) into the
/// note's attachments without loading it into memory.
#[tauri::command]
pub async fn note_attachment_add_from_path(
    note_id: String,
    src_path: String,
    app: tauri::AppHandle,
    core: tauri::State<'_, Core>,
) -> CmdResult<NoteAttachment> {
    let note_file = note_file_path(&note_id, &core).await?;
    let policy = load_attachment_policy(&core.db).await;
    let source = app.state::<veydan_shell::Shell>().open_source(&src_path)?;
    policy.check_size(source.len)?;
    let max = policy.max_file_bytes();
    let reserved = crate::sync::attachments::deferred_names(&core, &note_id).await?;
    tokio::task::spawn_blocking(move || {
        let mut reader = (source.open)().map_err(AppError::io)?;
        store_attachment_stream(
            &note_file,
            &note_id,
            &source.name,
            &reserved,
            source.len,
            max,
            &mut reader,
        )
    })
    .await
    .map_err(AppError::io)?
}

#[tauri::command]
pub async fn note_attachment_list(
    note_id: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<Vec<NoteAttachment>> {
    let note_file = note_file_path(&note_id, &core).await?;
    let dir = attachments_dir_for(&note_file, &note_id);
    let mut list: Vec<NoteAttachment> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_file() && !is_staging_name(&e.file_name().to_string_lossy()))
        .filter_map(|e| to_attachment(&note_id, &e.path()))
        .collect();
    // Vault-only files the policy did not download; a local copy wins.
    for d in crate::sync::attachments::deferred_for_note(&core, &note_id).await? {
        if !list.iter().any(|a| a.name == d.name) {
            list.push(deferred_attachment(&note_id, d.name, d.size));
        }
    }
    list.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(list)
}

/// Download a vault-only attachment to the note's directory (progress via transfer events).
#[tauri::command]
pub async fn note_attachment_fetch(
    note_id: String,
    name: String,
    app: tauri::AppHandle,
    core: tauri::State<'_, Core>,
    sync: tauri::State<'_, SyncManager>,
    notes: tauri::State<'_, NotesState>,
) -> CmdResult<NoteAttachment> {
    let note_file = note_file_path(&note_id, &core).await?;
    let name = safe_file_name(&name);
    crate::sync::attachments::fetch_deferred(&app, &core, &sync, &notes, &note_id, &name).await?;
    let path = attachments_dir_for(&note_file, &note_id).join(&name);
    to_attachment(&note_id, &path).ok_or_else(|| AppError::io("attachment download failed"))
}

#[tauri::command]
pub async fn note_attachment_delete(
    note_id: String,
    name: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<()> {
    let note_file = note_file_path(&note_id, &core).await?;
    let path = attachments_dir_for(&note_file, &note_id).join(safe_file_name(&name));
    if path.is_file() {
        std::fs::remove_file(&path).map_err(AppError::io)?;
    }
    Ok(())
}

#[cfg(desktop)]
#[tauri::command]
pub async fn note_attachment_open(
    note_id: String,
    name: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<()> {
    let note_file = note_file_path(&note_id, &core).await?;
    let path = attachments_dir_for(&note_file, &note_id).join(safe_file_name(&name));
    if !path.is_file() {
        return Err(AppError::not_found(name));
    }
    open_path(&path)
}

/// Copy an attachment to a user-chosen destination (Save As): a path, or an
/// Android `content://` URI from the system save dialog.
#[tauri::command]
pub async fn note_attachment_save(
    note_id: String,
    name: String,
    dest: String,
    app: tauri::AppHandle,
    core: tauri::State<'_, Core>,
) -> CmdResult<()> {
    let note_file = note_file_path(&note_id, &core).await?;
    let path = attachments_dir_for(&note_file, &note_id).join(safe_file_name(&name));
    if !path.is_file() {
        return Err(AppError::not_found(name));
    }
    tokio::task::spawn_blocking(move || {
        #[cfg(target_os = "android")]
        if dest.starts_with("content://") {
            let mut out = content_uri_writer(&app, &dest)?;
            std::io::copy(
                &mut std::fs::File::open(&path).map_err(AppError::io)?,
                &mut out,
            )
            .map_err(AppError::io)?;
            return Ok(());
        }
        #[cfg(not(target_os = "android"))]
        let _ = app;
        let dest_path = PathBuf::from(&dest);
        if let Some(parent) = dest_path.parent() {
            std::fs::create_dir_all(parent).map_err(AppError::io)?;
        }
        std::fs::copy(&path, &dest_path).map_err(AppError::io)?;
        Ok(())
    })
    .await
    .map_err(AppError::io)?
}

/// Find attachments no note body references. With `delete`, remove them.
#[cfg(desktop)]
#[tauri::command]
pub async fn note_attachments_gc(
    delete: bool,
    core: tauri::State<'_, Core>,
) -> CmdResult<Vec<OrphanAttachment>> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT id, file_path FROM notes")
        .fetch_all(&core.db)
        .await
        .map_err(AppError::db)?;
    Ok(collect_orphans(&core.app_data_dir, &rows, delete))
}

/// The attachments of `rows` (id, file path) their note does not reference.
/// Only what was read counts: a note whose file cannot be read is skipped,
/// not taken for an empty body, and a file newer than its note stays (the
/// editor may hold its link unsaved, or sync brought it before the note).
#[cfg(desktop)]
fn collect_orphans(
    app_data_dir: &Path,
    rows: &[(String, String)],
    delete: bool,
) -> Vec<OrphanAttachment> {
    let mut orphans = Vec::new();
    for (id, file_path) in rows {
        // `.` or `..` would name `attachments/` itself or the documents folder.
        if !valid_id(id) {
            continue;
        }
        let note_file = resolve_note_abs_path(app_data_dir, file_path);
        let dir = attachments_dir_for(&note_file, id);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let Ok((_, _, body)) = read_note_file(&note_file) else {
            continue;
        };
        let Ok(note_mtime) = std::fs::metadata(&note_file).and_then(|m| m.modified()) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
            else {
                continue;
            };
            if body_references(&body, id, &name) || is_staging_name(&name) {
                continue;
            }
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            if meta.modified().map_or(true, |m| m > note_mtime) {
                continue;
            }
            if delete {
                let _ = std::fs::remove_file(&path);
            }
            orphans.push(OrphanAttachment {
                note_id: id.clone(),
                name,
                size: meta.len(),
            });
        }
        if delete
            && std::fs::read_dir(&dir)
                .map(|mut d| d.next().is_none())
                .unwrap_or(false)
        {
            let _ = std::fs::remove_dir(&dir);
        }
    }
    orphans
}

#[cfg(all(test, desktop))]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn touch(path: &Path, at: SystemTime) {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(at)
            .unwrap();
    }

    /// A note with `old.png` older and `new.png` newer than its file, both
    /// unreferenced. Returns the row.
    fn note(root: &Path, id: &str, body: &[u8]) -> (String, String) {
        let file = root.join(format!("{id}.md"));
        std::fs::write(&file, body).unwrap();
        let at = SystemTime::now() - Duration::from_secs(3600);
        touch(&file, at);
        let dir = attachments_dir_for(&file, id);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, mtime) in [
            ("old.png", at - Duration::from_secs(60)),
            ("new.png", at + Duration::from_secs(60)),
        ] {
            std::fs::write(dir.join(name), b"png").unwrap();
            touch(&dir.join(name), mtime);
        }
        (id.to_string(), file.to_string_lossy().to_string())
    }

    /// A4-08: a file newer than its note may be linked by an edit not saved
    /// yet; it is neither listed nor removed.
    #[test]
    fn the_gc_keeps_a_file_newer_than_its_note() {
        let root = tempfile::tempdir().unwrap();
        let row = note(root.path(), "n1", b"no links");

        let orphans = collect_orphans(root.path(), std::slice::from_ref(&row), true);
        let names: Vec<&str> = orphans.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["old.png"]);
        let dir = root.path().join("attachments/n1");
        assert!(!dir.join("old.png").exists());
        assert!(dir.join("new.png").exists());
    }

    /// SYNCROWS-11: a note whose file cannot be read is not an empty body:
    /// none of its attachments is removed.
    #[test]
    fn the_gc_removes_nothing_of_a_note_it_could_not_read() {
        let root = tempfile::tempdir().unwrap();
        // Not UTF-8: the file is there and its body cannot be read.
        let unreadable = note(root.path(), "n1", &[0xff, 0xfe, 0x00]);
        let gone = note(root.path(), "n2", b"no links");
        std::fs::remove_file(&gone.1).unwrap();

        let orphans = collect_orphans(root.path(), &[unreadable, gone], true);
        assert!(orphans.is_empty(), "{orphans:?}");
        for id in ["n1", "n2"] {
            let dir = root.path().join("attachments").join(id);
            assert!(dir.join("old.png").exists() && dir.join("new.png").exists());
        }
    }
}
