// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use super::models::*;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use veydan_core::AppError;

// ── File helpers ──────────────────────────────────────────────────────────────

pub(crate) fn notes_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("notes")
}

pub(crate) fn documents_dir(app_data_dir: &Path) -> PathBuf {
    notes_dir(app_data_dir).join("documents")
}

pub(crate) fn drafts_dir(app_data_dir: &Path) -> PathBuf {
    notes_dir(app_data_dir).join("drafts")
}

pub(crate) fn draft_file_path(app_data_dir: &Path, id: &str) -> PathBuf {
    drafts_dir(app_data_dir).join(format!("{}.draft", id))
}

pub(crate) fn manifest_path(app_data_dir: &Path) -> PathBuf {
    notes_dir(app_data_dir).join("notes_manifest.json")
}

/// Returns the effective documents directory: custom if set, otherwise default.
pub(crate) fn effective_docs_dir(app_data_dir: &Path, custom_dir: Option<&PathBuf>) -> PathBuf {
    custom_dir
        .cloned()
        .unwrap_or_else(|| documents_dir(app_data_dir))
}

/// Resolves a stored file_path to an absolute path.
/// Notes are stored with absolute paths; a relative one is taken from the data directory.
pub fn resolve_note_abs_path(app_data_dir: &Path, file_path: &str) -> PathBuf {
    let p = PathBuf::from(file_path);
    if p.is_absolute() {
        p
    } else {
        app_data_dir.join(file_path)
    }
}

/// Absolute directory containing the note file (base for relative attachment links).
pub(crate) fn note_base_dir(app_data_dir: &Path, file_path: &str) -> String {
    resolve_note_abs_path(app_data_dir, file_path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default()
}

pub(crate) fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    // sha2 0.11 returns a hybrid_array `Array` which no longer implements
    // LowerHex; encode to a lowercase hex string manually.
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// Extract plain-text preview from note content (strips frontmatter + markdown symbols)
pub(crate) fn make_preview(content: &str) -> String {
    // Skip YAML frontmatter block (--- ... ---)
    let body = if let Some(rest) = content.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            rest[end + 4..].trim_start()
        } else {
            content
        }
    } else {
        content
    };
    // Strip common markdown: headers, bold/italic markers, links, code fences
    let mut result = String::new();
    for line in body.lines() {
        let stripped = line
            .trim_start_matches('#')
            .trim_start_matches('>')
            .trim_start_matches('-')
            .trim_start_matches('*')
            .replace("**", "")
            .replace('`', "")
            .replace("__", "");
        let stripped = stripped.trim();
        if !stripped.is_empty() {
            if !result.is_empty() {
                result.push(' ');
            }
            result.push_str(stripped);
            if result.chars().count() >= 120 {
                break;
            }
        }
    }
    result.chars().take(120).collect()
}

/// Serialize a note title for its single-line frontmatter field.
///
/// Plain titles are written as-is (keeps existing note files byte-identical);
/// titles that would corrupt the frontmatter structure — newlines (frontmatter
/// terminator injection), quotes/backslashes, or leading/trailing whitespace
/// (lost by the trimming parser) — are written as a double-quoted string with
/// `\\` `\"` `\n` `\r` escapes. `decode_fm_title` is the exact inverse.
pub(crate) fn encode_fm_title(title: &str) -> String {
    let needs_quoting = title.contains(['\n', '\r', '"', '\\'])
        || title.starts_with(char::is_whitespace)
        || title.ends_with(char::is_whitespace);
    if !needs_quoting {
        return title.to_string();
    }
    let mut out = String::with_capacity(title.len() + 2);
    out.push('"');
    for c in title.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Inverse of `encode_fm_title`: unquotes + unescapes a quoted title value;
/// unquoted values (all legacy notes) pass through unchanged.
pub(crate) fn decode_fm_title(raw: &str) -> String {
    let Some(inner) = raw
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .filter(|_| raw.len() >= 2)
    else {
        return raw.to_string();
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some(other) => out.push(other), // covers \\ and \"
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Write note file with frontmatter — atomic (tmp → fsync → rename)
pub(crate) fn write_note_file(
    path: &PathBuf,
    row: &NoteRow,
    tags: &[String],
    content: &str,
) -> Result<(), AppError> {
    let bindings_vec: Vec<String> = serde_json::from_str(&row.bindings).unwrap_or_default();
    let bindings_json = serde_json::to_string(&bindings_vec).unwrap_or_else(|_| "[]".to_string());

    let tags_yaml = if tags.is_empty() {
        "  []\n".to_string()
    } else {
        tags.iter().map(|t| format!("  - {}\n", t)).collect()
    };

    let frontmatter = format!(
        "---\nid: {}\ntitle: {}\nformat: {}\nbindings: {}\ntags:\n{}created_at: {}\nupdated_at: {}\n---\n",
        row.id, encode_fm_title(&row.title), row.format, bindings_json,
        tags_yaml, row.created_at, row.updated_at
    );

    let full = format!("{}{}", frontmatter, content);
    atomic_write(path, &full)
}

/// Atomic write: unique tmp next to the target, fsync, rename.
/// A shared `*.tmp` name races when several notes update the manifest at once.
pub(crate) fn atomic_write(path: &PathBuf, content: &str) -> Result<(), AppError> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(AppError::io)?;
        }
    }
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("note");
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_file_name(format!(".{name}.{}.{n}.tmp", std::process::id()));
    let write = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if write.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    write.map_err(AppError::io)
}

/// Parse frontmatter + body from file content.
/// Returns (kv map, tags list, body).
pub(crate) fn parse_note_file(raw: &str) -> (HashMap<String, String>, Vec<String>, String) {
    // CRLF files (edited on Windows) are parsed as LF; note files are always written with LF.
    if raw.starts_with("---\r\n") {
        return parse_note_file(&raw.replace("\r\n", "\n"));
    }
    if !raw.starts_with("---\n") {
        return (HashMap::new(), vec![], raw.to_string());
    }
    let rest = &raw[4..];
    let Some(end_idx) = rest.find("\n---\n") else {
        return (HashMap::new(), vec![], raw.to_string());
    };
    let fm_str = &rest[..end_idx];
    let raw_body = &rest[end_idx + 5..];
    // Strip exactly one leading newline that separates frontmatter from content
    let body = raw_body.strip_prefix('\n').unwrap_or(raw_body).to_string();

    let mut kv: HashMap<String, String> = HashMap::new();
    let mut tags: Vec<String> = Vec::new();
    let mut in_tags = false;

    for line in fm_str.lines() {
        if in_tags {
            if let Some(tag) = line.strip_prefix("  - ") {
                tags.push(tag.trim().to_string());
                continue;
            } else if line.trim() == "[]" {
                in_tags = false;
                continue;
            } else {
                in_tags = false;
            }
        }
        // `tags:` is written without a space; other keys use `: `.
        if let Some((key, val)) = line.split_once(':') {
            let key = key.trim();
            let val = val.trim();
            if key == "tags" {
                in_tags = val.is_empty() || val == "[]";
                continue;
            }
            let val = if key == "title" {
                decode_fm_title(val)
            } else {
                val.to_string()
            };
            kv.insert(key.to_string(), val);
        }
    }

    (kv, tags, body)
}

/// A note file read: frontmatter keys and values, tags, body content.
pub(crate) type NoteFile = (HashMap<String, String>, Vec<String>, String);

/// Read note file: returns (kv, tags, body content)
pub(crate) fn read_note_file(path: &PathBuf) -> Result<NoteFile, AppError> {
    let raw = std::fs::read_to_string(path).map_err(AppError::io)?;
    Ok(parse_note_file(&raw))
}

// ── Helpers for row → public structs ─────────────────────────────────────────

pub(crate) fn row_to_list_item(
    row: NoteRow,
    tags: Vec<NoteTagInfo>,
    folder_ids: Vec<String>,
    has_draft: bool,
) -> NoteListItem {
    let bindings: Vec<String> = serde_json::from_str(&row.bindings).unwrap_or_default();
    NoteListItem {
        id: row.id,
        title: row.title,
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
        has_draft,
        preview: row.preview,
        snippet: None,
    }
}

#[cfg(test)]
mod title_escaping_tests {
    use super::*;

    #[test]
    fn plain_titles_unchanged_and_legacy_parse_intact() {
        assert_eq!(encode_fm_title("My note: draft #2"), "My note: draft #2");
        // Legacy file with an unquoted title still parses to the same value.
        let raw = "---\nid: n1\ntitle: My note: draft #2\nformat: md\n---\n\nbody";
        let (kv, _, body) = parse_note_file(raw);
        assert_eq!(
            kv.get("title").map(String::as_str),
            Some("My note: draft #2")
        );
        assert_eq!(body, "body");
    }

    #[test]
    fn hostile_titles_round_trip_without_breaking_frontmatter() {
        for title in [
            "line1\n---\ninjected: yes",
            "quote \" and \\ backslash",
            "  padded  ",
            "---",
        ] {
            let encoded = encode_fm_title(title);
            assert!(
                !encoded.contains('\n'),
                "encoded title must stay single-line"
            );
            let raw = format!("---\nid: n1\ntitle: {}\nformat: md\n---\n\nbody", encoded);
            let (kv, _, body) = parse_note_file(&raw);
            assert_eq!(kv.get("title").map(String::as_str), Some(title));
            assert_eq!(body, "body", "frontmatter must terminate at the real ---");
        }
    }
}

/// The note file travels as the blob of the `note` sync entity, so its text
/// is part of the vault format shared with 4.0.7.
#[cfg(test)]
mod wire_format_tests {
    use super::*;

    fn row(title: &str, bindings: &str) -> NoteRow {
        NoteRow {
            id: "n1".into(),
            title: title.into(),
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
            bindings: bindings.into(),
        }
    }

    fn written(row: &NoteRow, tags: &[&str], content: &str) -> String {
        let dir = std::env::temp_dir().join(format!("veydan-note-file-{}", uuid::Uuid::new_v4()));
        let path = dir.join("n1.md");
        let tags: Vec<String> = tags.iter().map(|t| t.to_string()).collect();
        write_note_file(&path, row, &tags, content).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        text
    }

    #[test]
    fn a_note_file_is_written_as_4_0_7_writes_it() {
        let text = written(
            &row("Deploy", r#"["workspace:w1","profile:p1"]"#),
            &["ops", "infra"],
            "\n# Deploy\n",
        );
        assert_eq!(
            text,
            "---\n\
             id: n1\n\
             title: Deploy\n\
             format: md\n\
             bindings: [\"workspace:w1\",\"profile:p1\"]\n\
             tags:\n\
             \x20 - ops\n\
             \x20 - infra\n\
             created_at: 2026-09-30T10:00:00+00:00\n\
             updated_at: 2026-09-30T11:00:00+00:00\n\
             ---\n\
             \n# Deploy\n"
        );

        let bare = written(&row("line1\nline2", "[]"), &[], "");
        assert_eq!(
            bare,
            "---\n\
             id: n1\n\
             title: \"line1\\nline2\"\n\
             format: md\n\
             bindings: []\n\
             tags:\n\
             \x20 []\n\
             created_at: 2026-09-30T10:00:00+00:00\n\
             updated_at: 2026-09-30T11:00:00+00:00\n\
             ---\n"
        );
    }

    #[test]
    fn a_note_file_of_4_0_7_is_read() {
        let raw = "---\n\
                   id: n1\n\
                   title: \"Deploy: \\\"prod\\\"\"\n\
                   format: md\n\
                   bindings: [\"workspace:w1\"]\n\
                   tags:\n\
                   \x20 - ops\n\
                   created_at: 2026-09-30T10:00:00+00:00\n\
                   updated_at: 2026-09-30T11:00:00+00:00\n\
                   ---\n\
                   \n# Deploy\n";
        let (kv, tags, body) = parse_note_file(raw);

        let mut keys: Vec<&str> = kv.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "bindings",
                "created_at",
                "format",
                "id",
                "title",
                "updated_at"
            ]
        );
        assert_eq!(kv["id"], "n1");
        assert_eq!(kv["title"], "Deploy: \"prod\"");
        assert_eq!(kv["format"], "md");
        assert_eq!(kv["bindings"], r#"["workspace:w1"]"#);
        assert_eq!(kv["created_at"], "2026-09-30T10:00:00+00:00");
        assert_eq!(kv["updated_at"], "2026-09-30T11:00:00+00:00");
        assert_eq!(tags, ["ops"]);
        assert_eq!(body, "# Deploy\n");
    }
}

#[cfg(desktop)]
pub(crate) fn open_path(path: &std::path::Path) -> Result<(), AppError> {
    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(path)
        .spawn()
        .map_err(AppError::io)?;
    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg(path)
        .spawn()
        .map_err(AppError::io)?;
    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer")
        .arg(path)
        .spawn()
        .map_err(AppError::io)?;
    Ok(())
}
