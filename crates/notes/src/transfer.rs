// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Export notes (Markdown + frontmatter + attachments) to a folder or zip,
//! and import Markdown files/folders produced by us or other editors.

use super::attachments::{attachments_dir_for, safe_file_name, store_attachment};
use super::crud::{current_docs_dir, insert_note, NewNote};
use super::files::*;
use super::history::history_snapshot;
use super::models::*;
use super::NotesState;
use age::secrecy::SecretString;
use chrono::Utc;
use serde::Serialize;
use std::collections::HashSet;
use std::io::{Cursor, Read, Seek, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use veydan_core::Core;
use veydan_core::{AppError, CmdResult};

const NOTE_EXTS: &[&str] = &["md", "markdown", "txt"];
/// Same cap as backups: accept archives made on faster machines.
const MAX_SCRYPT_WORK_FACTOR: u8 = 22;

#[derive(Debug, Serialize)]
pub struct ExportResult {
    pub count: usize,
    pub path: String,
}

// ── Export ────────────────────────────────────────────────────────────────────

/// Unique `{title}.{ext}` inside the export.
fn export_file_name(title: &str, ext: &str, taken: &mut HashSet<String>) -> String {
    let base = safe_file_name(title).replace('/', "_");
    let mut name = format!("{base}.{ext}");
    let mut i = 2;
    while taken.contains(&name) {
        name = format!("{base}-{i}.{ext}");
        i += 1;
    }
    taken.insert(name.clone());
    name
}

/// (archive-relative path, source file) pairs for the given notes.
async fn collect_export_entries(
    ids: &[String],
    core: &Core,
) -> Result<Vec<(String, PathBuf)>, AppError> {
    let mut entries = Vec::new();
    let mut taken = HashSet::new();
    for id in ids {
        let Some(row) = sqlx::query_as::<_, NoteRow>("SELECT * FROM notes WHERE id = ?")
            .bind(id)
            .fetch_optional(&core.db)
            .await
            .map_err(AppError::db)?
        else {
            continue;
        };

        let file = resolve_note_abs_path(&core.app_data_dir, &row.file_path);
        if !file.is_file() {
            continue;
        }
        let ext = if row.format == "txt" { "txt" } else { "md" };
        entries.push((export_file_name(&row.title, ext, &mut taken), file.clone()));

        let att_dir = attachments_dir_for(&file, &row.id);
        if let Ok(files) = std::fs::read_dir(&att_dir) {
            for f in files.flatten().filter(|f| f.path().is_file()) {
                let name = f.file_name().to_string_lossy().to_string();
                entries.push((format!("attachments/{}/{}", row.id, name), f.path()));
            }
        }
    }
    Ok(entries)
}

fn write_zip<W: Write + Seek>(out: W, entries: &[(String, PathBuf)]) -> Result<W, AppError> {
    use zip::write::SimpleFileOptions;
    let mut zip = zip::ZipWriter::new(out);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, src) in entries {
        zip.start_file(name, opts).map_err(AppError::io)?;
        let data = std::fs::read(src).map_err(AppError::io)?;
        zip.write_all(&data).map_err(AppError::io)?;
    }
    zip.finish().map_err(AppError::io)
}

/// Zip the entries in memory and write them passphrase-encrypted with age.
fn write_encrypted_zip(
    dest: &Path,
    password: &str,
    entries: &[(String, PathBuf)],
) -> Result<(), AppError> {
    let zipped = write_zip(Cursor::new(Vec::new()), entries)?.into_inner();
    let file = std::fs::File::create(dest).map_err(AppError::io)?;
    let encryptor = age::Encryptor::with_user_passphrase(SecretString::from(password.to_owned()));
    let mut writer = encryptor.wrap_output(file).map_err(AppError::other)?;
    writer.write_all(&zipped).map_err(AppError::io)?;
    writer.finish().map_err(AppError::io)?;
    Ok(())
}

/// Decrypt an age file into memory; `None` when the password is wrong.
fn decrypt_age(src: &Path, password: &str) -> Result<Vec<u8>, AppError> {
    let file = std::fs::File::open(src).map_err(AppError::io)?;
    let decryptor = age::Decryptor::new(file)
        .map_err(|e| AppError::other(format!("Cannot read archive: {e}")))?;
    let mut identity = age::scrypt::Identity::new(SecretString::from(password.to_owned()));
    identity.set_max_work_factor(MAX_SCRYPT_WORK_FACTOR);
    let mut reader = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|_| AppError::other("Wrong password or corrupt archive"))?;
    let mut out = Vec::new();
    reader.read_to_end(&mut out).map_err(AppError::io)?;
    Ok(out)
}

/// Unpack a zip (plain or age-encrypted) into `dest`, rejecting paths that escape it.
fn unpack_zip(src: &Path, password: Option<&str>, dest: &Path) -> Result<(), AppError> {
    let bytes = match password {
        Some(pw) => decrypt_age(src, pw)?,
        None => std::fs::read(src).map_err(AppError::io)?,
    };
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(AppError::io)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(AppError::io)?;
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        let target = dest.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(AppError::io)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(AppError::io)?;
        }
        let mut out = std::fs::File::create(&target).map_err(AppError::io)?;
        std::io::copy(&mut entry, &mut out).map_err(AppError::io)?;
    }
    Ok(())
}

fn is_archive(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("zip" | "age")
    )
}

fn write_dir(dest: &Path, entries: &[(String, PathBuf)]) -> Result<(), AppError> {
    for (name, src) in entries {
        let target = dest.join(name);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(AppError::io)?;
        }
        std::fs::copy(src, &target).map_err(AppError::io)?;
    }
    Ok(())
}

/// Export notes to `dest`: a directory, a `.zip` file when `as_zip`,
/// or an age-encrypted zip when `password` is given.
#[tauri::command]
pub async fn note_export(
    ids: Vec<String>,
    dest: String,
    as_zip: bool,
    password: Option<String>,
    core: tauri::State<'_, Core>,
) -> CmdResult<ExportResult> {
    let entries = collect_export_entries(&ids, &core).await?;
    let dest_path = PathBuf::from(&dest);
    let password = password.filter(|p| !p.is_empty());
    if let Some(pw) = password {
        write_encrypted_zip(&dest_path, &pw, &entries)?;
    } else if as_zip {
        write_zip(
            std::fs::File::create(&dest_path).map_err(AppError::io)?,
            &entries,
        )?;
    } else {
        std::fs::create_dir_all(&dest_path).map_err(AppError::io)?;
        write_dir(&dest_path, &entries)?;
    }
    let count = entries
        .iter()
        .filter(|(n, _)| !n.starts_with("attachments/"))
        .count();
    Ok(ExportResult { count, path: dest })
}

// ── Import ────────────────────────────────────────────────────────────────────

fn is_note_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| NOTE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Recursively collect note files, skipping hidden dirs and `attachments/`.
fn collect_note_files(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_file() {
        if is_note_file(path) {
            out.push(path.to_path_buf());
        }
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            if name.starts_with('.') || name == "attachments" {
                continue;
            }
            collect_note_files(&p, out);
        } else if is_note_file(&p) {
            out.push(p);
        }
    }
}

fn is_external_target(target: &str) -> bool {
    target.is_empty()
        || target.starts_with('#')
        || target.contains("://")
        || target.starts_with("mailto:")
        || target.starts_with("data:")
}

/// Resolve a link target relative to the source note; also look in a sibling `attachments/`.
fn resolve_local(src_dir: &Path, target: &str) -> Option<PathBuf> {
    let decoded = percent_decode_simple(target);
    let candidates = [
        src_dir.join(&decoded),
        src_dir.join("attachments").join(&decoded),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

fn percent_decode_simple(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Copy locally referenced files into the new note's attachments and rewrite links.
/// Handles `[x](rel/path)` / `![x](rel/path)` and Obsidian `![[file]]` embeds.
fn rewrite_links(
    body: &str,
    src_dir: &Path,
    note_file: &Path,
    note_id: &str,
) -> Result<String, AppError> {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;

    loop {
        // Pick whichever construct comes first
        let md_idx = rest.find("](").map(|i| i + 2);
        let wiki_idx = rest.find("![[").map(|i| i + 3);
        let (start, is_wiki) = match (md_idx, wiki_idx) {
            (Some(m), Some(w)) if w < m => (w, true),
            (Some(m), _) => (m, false),
            (None, Some(w)) => (w, true),
            (None, None) => break,
        };
        out.push_str(&rest[..start]);
        rest = &rest[start..];

        let close = if is_wiki {
            rest.find("]]")
        } else {
            rest.find(')')
        };
        let Some(close) = close else { break };
        let raw_target = &rest[..close];
        let target = if is_wiki {
            raw_target.split('|').next().unwrap_or("")
        } else {
            raw_target.split(' ').next().unwrap_or("")
        };

        let replacement = if is_external_target(target) {
            None
        } else {
            resolve_local(src_dir, target).and_then(|src| {
                let data = std::fs::read(&src).ok()?;
                let name = src.file_name()?.to_str()?;
                store_attachment(note_file, note_id, name, &data)
                    .ok()
                    .map(|a| a.rel_path)
            })
        };

        match replacement {
            Some(rel) if is_wiki => {
                // Turn `![[file]]` into a standard image/file link; drop the leading `![[`
                out.truncate(out.len() - 3);
                let label = target.rsplit('/').next().unwrap_or(target);
                out.push_str(&format!("![{label}]({rel})"));
                rest = &rest[close + 2..];
            }
            Some(rel) => {
                out.push_str(&rel);
                out.push_str(&raw_target[target.len()..]);
                out.push(')');
                rest = &rest[close + 1..];
            }
            None => {
                let end = if is_wiki { close + 2 } else { close + 1 };
                out.push_str(&rest[..end]);
                rest = &rest[end..];
            }
        }
    }
    out.push_str(rest);
    Ok(out)
}

fn title_from_body(body: &str, fallback: &str) -> (String, String) {
    let mut lines = body.lines();
    if let Some(first) = lines.next() {
        if let Some(h) = first.strip_prefix("# ") {
            let remaining = body[first.len()..].trim_start_matches('\n').to_string();
            return (h.trim().to_string(), remaining);
        }
    }
    (fallback.to_string(), body.to_string())
}

async fn id_is_free(id: &str, core: &Core) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM notes WHERE id = ?")
        .bind(id)
        .fetch_one(&core.db)
        .await
        .map(|n| n == 0)
        .unwrap_or(false)
}

/// Import `.md`/`.txt` files, directories, or `.zip`/`.age` archives made by
/// `note_export` (`password` unlocks `.age`). Returns created note ids.
#[tauri::command]
pub async fn note_import(
    paths: Vec<String>,
    bindings: Vec<String>,
    password: Option<String>,
    core: tauri::State<'_, Core>,
    notes: tauri::State<'_, NotesState>,
) -> CmdResult<Vec<String>> {
    let password = password.filter(|p| !p.is_empty());
    let unpack_root = notes_dir(&core.app_data_dir).join(format!(".import-{}", Uuid::new_v4()));
    let mut files = Vec::new();
    for p in &paths {
        let path = Path::new(p);
        if path.is_file() && is_archive(path) {
            let dir = unpack_root.join(files.len().to_string());
            std::fs::create_dir_all(&dir).map_err(AppError::io)?;
            let pw = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("age"))
                .unwrap_or(false);
            if let Err(e) = unpack_zip(path, if pw { password.as_deref() } else { None }, &dir) {
                let _ = std::fs::remove_dir_all(&unpack_root);
                return Err(e);
            }
            collect_note_files(&dir, &mut files);
        } else {
            collect_note_files(path, &mut files);
        }
    }

    let result = import_files(files, bindings, &core, &notes).await;
    let _ = std::fs::remove_dir_all(&unpack_root);
    result
}

async fn import_files(
    files: Vec<PathBuf>,
    bindings: Vec<String>,
    core: &Core,
    notes: &NotesState,
) -> CmdResult<Vec<String>> {
    let docs_dir = current_docs_dir(core, notes);
    let mut created = Vec::new();

    for src in files {
        let Ok(raw) = std::fs::read_to_string(&src) else {
            continue;
        };
        let (kv, tags, body) = parse_note_file(&raw);
        let stem = src
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Imported")
            .to_string();

        let (title, body) = match kv.get("title") {
            Some(t) => (t.clone(), body),
            None => title_from_body(&body, &stem),
        };

        let id = match kv.get("id") {
            Some(existing)
                if Uuid::parse_str(existing).is_ok() && id_is_free(existing, core).await =>
            {
                existing.clone()
            }
            _ => Uuid::new_v4().to_string(),
        };

        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("md")
            .to_ascii_lowercase();
        let format = if ext == "txt" { "txt" } else { "md" }.to_string();

        let mtime = std::fs::metadata(&src)
            .and_then(|m| m.modified())
            .map(|t| chrono::DateTime::<Utc>::from(t).to_rfc3339())
            .unwrap_or_else(|_| Utc::now().to_rfc3339());
        let created_at = kv
            .get("created_at")
            .cloned()
            .unwrap_or_else(|| mtime.clone());
        let updated_at = kv.get("updated_at").cloned().unwrap_or(mtime);

        let note_bindings: Vec<String> = kv
            .get("bindings")
            .and_then(|b| serde_json::from_str(b).ok())
            .unwrap_or_else(|| bindings.clone());

        let note_file = docs_dir.join(format!("{id}.{format}"));
        let src_dir = src.parent().unwrap_or(Path::new("."));
        let content = rewrite_links(&body, src_dir, &note_file, &id)?;

        let note = insert_note(
            NewNote {
                id: id.clone(),
                title: title.clone(),
                format,
                bindings: note_bindings,
                tags,
                content: content.clone(),
                created_at,
                updated_at,
            },
            core,
            notes,
        )
        .await?;
        let _ = history_snapshot(&note.id, &title, &content, "import", None, &core.db).await;
        created.push(id);
    }

    Ok(created)
}
