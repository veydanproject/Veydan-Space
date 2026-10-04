// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Notes as a sync entity. The note file (frontmatter + body) is the payload;
//! it travels as a blob, the op only references it.
//!
//! Op payload:
//! `{ "blob": <name>, "parents": [<blob>..], "format": "md", "title": ".." }`
//! A delete op carries only `parents`.
//!
//! Apply rules for a remote op against the local head:
//! - op.hlc <= head.hlc: skip only when that blob is already in our head.
//!   A divergent older put still merges, so both devices show the conflict.
//! - delete: apply (fast-forward or newer-wins). A note without a row yet
//!   loses the file an earlier put left, so the index never takes it for new.
//! - put older than a peer's tombstone that is our head: dropped. So is a put
//!   of a note never held here that a later tombstone of the same pull removes.
//! - put whose parents include our head: fast-forward.
//! - put that diverged: 3-way merge from the exact common ancestor; a clean
//!   merge is pushed with both parents, overlapping edits become a conflict
//!   that keeps the local file untouched and is resolved through the UI.

use super::state::{load_note_state, save_note_state, NoteSyncState};
use crate::{
    effective_docs_dir, history_content_by_id, history_snapshot_by, merge3, parse_note_file,
    resolve_note_abs_path, restore_note, set_note_tag_links, sync_notes_index, trash_note,
    update_note, write_note_file, MergeResult, NoteRow, NoteUpdateInput, NotesState,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};
use veydan_core::{settings, Core};
use veydan_core::{AppError, CmdResult};
use veydan_sync::{sha256_hex, Engine, Hlc, HlcClock, LocalState, Op};
use veydan_sync_host::{Applied, Collected, Cycle, Handler};

pub const ENTITY: &str = "note";

#[derive(Debug, Default, Serialize, Deserialize)]
struct NotePayload {
    #[serde(default)]
    blob: String,
    #[serde(default)]
    parents: Vec<String>,
    #[serde(default = "default_format")]
    format: String,
    #[serde(default)]
    title: String,
}

fn default_format() -> String {
    "md".into()
}

#[derive(sqlx::FromRow)]
struct NoteHead {
    id: String,
    title: String,
    file_path: String,
    format: String,
    deleted: i64,
}

/// Note ids are UUIDs; anything else must not become a file name.
pub(super) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn put_op(id: &str, hlc: Hlc, blob: String, parents: Vec<String>, format: &str, title: &str) -> Op {
    Op {
        entity_type: ENTITY.into(),
        entity_id: id.into(),
        hlc,
        deleted: false,
        payload: serde_json::to_value(NotePayload {
            blob,
            parents,
            format: format.into(),
            title: title.into(),
        })
        .unwrap_or_default(),
    }
}

fn delete_op(id: &str, hlc: Hlc, parents: Vec<String>) -> Op {
    Op {
        entity_type: ENTITY.into(),
        entity_id: id.into(),
        hlc,
        deleted: true,
        payload: serde_json::to_value(NotePayload {
            parents,
            ..Default::default()
        })
        .unwrap_or_default(),
    }
}

pub(super) fn docs_dir(core: &Core, notes: &NotesState) -> PathBuf {
    let custom = notes.custom_dir.read().ok().and_then(|g| g.clone());
    effective_docs_dir(&core.app_data_dir, custom.as_ref())
}

/// File stems (= note ids) present in the documents directory.
fn file_stems(dir: &PathBuf) -> HashSet<String> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().is_file())
                .filter_map(|e| {
                    e.path()
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Extension for a new note file; anything odd falls back to `md`.
fn safe_format(format: &str) -> &str {
    if !format.is_empty() && format.len() <= 8 && format.chars().all(|c| c.is_ascii_alphanumeric())
    {
        format
    } else {
        "md"
    }
}

// ── Push ─────────────────────────────────────────────────────────────────────

/// Local changes not yet in the vault. Blobs are uploaded here; the returned
/// states must be saved only after the ops were pushed.
pub struct LocalChanges {
    pub ops: Vec<Op>,
    pub states: Vec<NoteSyncState>,
}

pub async fn collect_local_changes(
    engine: &Engine,
    core: &Core,
    notes: &NotesState,
    clock: &mut HlcClock,
) -> CmdResult<LocalChanges> {
    let db = &core.db;
    let rows: Vec<NoteHead> =
        sqlx::query_as("SELECT id, title, file_path, format, deleted FROM notes")
            .fetch_all(db)
            .await
            .map_err(AppError::db)?;
    let mut states = super::state::load_note_states(db).await?;
    let mut out = LocalChanges {
        ops: Vec::new(),
        states: Vec::new(),
    };

    for row in &rows {
        let prev = states.remove(&row.id);
        if row.deleted != 0 {
            // Tombstone once; a note the vault never saw gets one without parents.
            if prev.as_ref().map(|s| s.deleted).unwrap_or(false) {
                continue;
            }
            let mut st = prev.unwrap_or_else(|| NoteSyncState {
                note_id: row.id.clone(),
                ..Default::default()
            });
            let hlc = clock.now();
            out.ops
                .push(delete_op(&row.id, hlc.clone(), head_parents(&st)));
            st.deleted = true;
            st.head_hlc = Some(hlc);
            out.states.push(st);
            continue;
        }
        // An unresolved conflict holds the note back; resolving clears the flag.
        if prev.as_ref().map(|s| s.conflict).unwrap_or(false) {
            continue;
        }
        let path = resolve_note_abs_path(&core.app_data_dir, &row.file_path);
        let Ok(raw) = std::fs::read(&path) else {
            continue;
        };
        let hash = sha256_hex(&raw);
        // A resolved conflict is pushed even when the text stayed local, so the vault learns the merge.
        let unchanged = prev
            .as_ref()
            .map(|s| s.synced_hash == hash && !s.deleted && s.conflict_remote_blob.is_empty());
        if unchanged.unwrap_or(false) {
            continue;
        }
        let blob = engine.put_blob(&raw).await.map_err(AppError::other)?;
        // Parents: our head, plus the remote version a resolved conflict merged in.
        let parents: Vec<String> = prev
            .iter()
            .flat_map(|s| [s.head_blob.clone(), s.conflict_remote_blob.clone()])
            .filter(|b| !b.is_empty())
            .collect();
        let hlc = clock.now();
        out.ops.push(put_op(
            &row.id,
            hlc.clone(),
            blob.clone(),
            parents.clone(),
            &row.format,
            &row.title,
        ));
        out.states.push(NoteSyncState {
            note_id: row.id.clone(),
            head_blob: blob,
            head_parents: parents,
            head_hlc: Some(hlc),
            synced_hash: hash,
            ..Default::default()
        });
    }

    // Rows gone entirely (hard delete) while the vault still has them. A file
    // that is still on disk only means the index has not caught up yet.
    let on_disk = file_stems(&docs_dir(core, notes));
    for (_, mut st) in states
        .into_iter()
        .filter(|(id, s)| !s.deleted && !on_disk.contains(id))
    {
        let hlc = clock.now();
        out.ops.push(delete_op(
            &st.note_id,
            hlc.clone(),
            vec![st.head_blob.clone()],
        ));
        st.deleted = true;
        st.head_hlc = Some(hlc);
        out.states.push(st);
    }
    Ok(out)
}

// ── Pull ─────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ApplyOutcome {
    /// Notes whose file changed; the index and UI must be refreshed.
    pub changed: Vec<String>,
    /// Notes taken out of the trash during apply; the UI is told to refresh them.
    pub restored: Vec<String>,
    /// Set when an op was skipped on a transient condition; peer heads must not
    /// advance so the skipped ops are delivered again next cycle.
    pub retry: Option<String>,
    /// Blobs collected by GC. Reported as warnings; peer heads still advance.
    pub skipped: Vec<String>,
}

async fn load_head(db: &sqlx::Pool<sqlx::Sqlite>, id: &str) -> CmdResult<Option<NoteHead>> {
    sqlx::query_as("SELECT id, title, file_path, format, deleted FROM notes WHERE id = ?")
        .bind(id)
        .fetch_optional(db)
        .await
        .map_err(AppError::db)
}

/// Atomic write: tmp file next to the target, then rename.
pub(super) fn write_raw(path: &PathBuf, raw: &[u8]) -> CmdResult<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(AppError::io)?;
    }
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::File::create(&tmp).map_err(AppError::io)?;
    f.write_all(raw).map_err(AppError::io)?;
    f.sync_all().map_err(AppError::io)?;
    drop(f);
    std::fs::rename(&tmp, path).map_err(AppError::io)?;
    Ok(())
}

/// Remove the file a put left for a note the index has not seen. Only a file
/// that still holds what the put wrote is removed: `written_by_sync` tells.
fn remove_unindexed(
    dir: &PathBuf,
    id: &str,
    written_by_sync: impl Fn(&[u8]) -> bool,
) -> CmdResult<bool> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Ok(false);
    };
    let mut removed = false;
    for path in rd.filter_map(|e| e.ok()).map(|e| e.path()) {
        let left_by_put = path.is_file()
            && path.file_stem().is_some_and(|s| s.to_string_lossy() == id)
            && std::fs::read(&path).is_ok_and(|raw| written_by_sync(&raw));
        if left_by_put {
            std::fs::remove_file(&path).map_err(AppError::io)?;
            removed = true;
        }
    }
    Ok(removed)
}

/// Union keeping local order, then remote extras.
fn merge_tags(local: &[String], remote: &[String]) -> Vec<String> {
    let mut out = local.to_vec();
    for t in remote {
        if !out.contains(t) {
            out.push(t.clone());
        }
    }
    out
}

/// Apply remote ops. Ops from the same pull are already HLC-sorted.
/// Merge results are pushed immediately so a later failure cannot strand them.
/// Without the app no progress goes out.
pub async fn apply_remote(
    engine: &Engine,
    app: Option<&AppHandle>,
    core: &Core,
    notes: &NotesState,
    ops: Vec<Op>,
    clock: &mut HlcClock,
    local: &mut LocalState,
) -> CmdResult<ApplyOutcome> {
    let progress = |current: u32, total: u32, id: &str| {
        if let Some(app) = app {
            super::emit_progress(
                app,
                "apply",
                super::progress_pct(48, 52, current, total.max(1)),
                current,
                total,
                id,
            );
        }
    };
    apply_ops(engine, core, notes, ops, clock, local, progress).await
}

/// The apply itself, on the state alone and without the running app.
/// `progress(current, total, note id)` is called before each put.
async fn apply_ops(
    engine: &Engine,
    core: &Core,
    notes: &NotesState,
    ops: Vec<Op>,
    clock: &mut HlcClock,
    local: &mut LocalState,
    progress: impl Fn(u32, u32, &str),
) -> CmdResult<ApplyOutcome> {
    let db = &core.db;
    let mut outcome = ApplyOutcome::default();
    let total = ops
        .iter()
        .filter(|op| op.entity_type == ENTITY && !op.deleted && valid_id(&op.entity_id))
        .count() as u32;
    let mut current = 0u32;
    // Notes trashed on this device that a newer remote put tried to revive.
    let mut reassert: Vec<String> = Vec::new();
    // The last tombstone of each note in this pull. A put before it, of a note
    // this device never held, is not fetched only to be removed again.
    let mut tombstones: HashMap<String, Hlc> = HashMap::new();
    for op in ops
        .iter()
        .filter(|op| op.entity_type == ENTITY && op.deleted)
    {
        match tombstones.get(&op.entity_id) {
            Some(latest) if *latest >= op.hlc => {}
            _ => {
                tombstones.insert(op.entity_id.clone(), op.hlc.clone());
            }
        }
    }

    // Stems of the files in the documents directory, listed once: when a
    // tombstone first finds neither a row nor a state for its note.
    let mut leftovers: Option<HashSet<String>> = None;

    for op in ops {
        if op.entity_type != ENTITY || !valid_id(&op.entity_id) {
            continue;
        }
        let id = op.entity_id.clone();
        let payload: NotePayload = serde_json::from_value(op.payload.clone()).unwrap_or_default();
        let prev = load_note_state(db, &id).await?;
        // Older than our head: drop it only if we already have that version.
        // A fork with an older clock still merges, so the conflict shows here too.
        let stale = prev
            .as_ref()
            .and_then(|s| s.head_hlc.as_ref())
            .is_some_and(|h| *h >= op.hlc);
        let already = prev
            .as_ref()
            .is_some_and(|s| s.head_blob == payload.blob || s.head_parents.contains(&payload.blob));
        if stale && (op.deleted || already) {
            continue;
        }
        let row = load_head(db, &id).await?;

        if op.deleted {
            match &row {
                Some(r) if r.deleted == 0 => {
                    trash_note(&id, core).await?;
                    outcome.changed.push(id.clone());
                }
                Some(_) => {}
                // No row to move to the trash. The file an earlier put left
                // must go, or the index takes it for a note written here.
                None => {
                    let dir = docs_dir(core, notes);
                    let removed = match &prev {
                        Some(st) if st.deleted || st.synced_hash.is_empty() => false,
                        Some(st) => {
                            remove_unindexed(&dir, &id, |raw| sha256_hex(raw) == st.synced_hash)?
                        }
                        // An apply that wrote the file stopped before it saved
                        // the state: the file is the version this op deletes.
                        None => {
                            let leftovers = leftovers.get_or_insert_with(|| file_stems(&dir));
                            leftovers.contains(&id)
                                && remove_unindexed(&dir, &id, |raw| {
                                    payload.parents.contains(&engine.blob_name(raw))
                                })?
                        }
                    };
                    if removed {
                        outcome.changed.retain(|c| *c != id);
                    }
                }
            }
            let mut st = prev.unwrap_or_else(|| NoteSyncState {
                note_id: id.clone(),
                ..Default::default()
            });
            st.deleted = true;
            st.head_hlc = Some(op.hlc.clone());
            save_note_state(db, &st).await?;
            continue;
        }

        if prev.is_none() && row.is_none() && tombstones.get(&id).is_some_and(|h| *h > op.hlc) {
            continue;
        }
        current += 1;
        progress(current, total, &id);
        // Our own delete wins over a newer remote put; a peer's tombstone may be
        // undone by a put, since that is how a restore on another device arrives.
        if deleted_here(engine, prev.as_ref(), row.as_ref()) {
            if !reassert.contains(&id) {
                reassert.push(id);
            }
            continue;
        }
        // A put older than the tombstone held here lost to it: applied now, it
        // would bring the note back on this device alone.
        if stale && prev.as_ref().is_some_and(|s| s.deleted) {
            continue;
        }
        let Some(remote_raw) = engine
            .get_blob(&payload.blob)
            .await
            .map_err(AppError::other)?
        else {
            let now_ms = Utc::now().timestamp_millis().max(0) as u64;
            if super::blob_gone(&op, now_ms) {
                outcome
                    .skipped
                    .push(format!("blob for note {id} was collected"));
            } else {
                // Chunk arrived before its blob; deliver the rest again next cycle.
                outcome.retry = Some(format!("blob for note {id} not available yet"));
            }
            continue;
        };
        let remote_hash = sha256_hex(&remote_raw);

        let path = match &row {
            Some(r) => resolve_note_abs_path(&core.app_data_dir, &r.file_path),
            None => docs_dir(core, notes).join(format!("{id}.{}", safe_format(&payload.format))),
        };
        let local_raw = std::fs::read(&path).ok();
        let local_changed = match (&prev, &local_raw) {
            (Some(s), Some(raw)) if !s.deleted && !s.head_blob.is_empty() => {
                sha256_hex(raw) != s.synced_hash
            }
            _ => false,
        };
        let remote_includes_ours = match &prev {
            None => true,
            Some(s) => {
                s.deleted
                    || s.head_blob.is_empty()
                    || s.head_blob == payload.blob
                    || payload.parents.contains(&s.head_blob)
            }
        };

        // No DB row yet: the file came from an earlier op of this pull, nothing local to merge.
        if (remote_includes_ours || row.is_none()) && !local_changed {
            let same_bytes = local_raw.as_deref() == Some(remote_raw.as_slice());
            if !same_bytes {
                write_raw(&path, &remote_raw)?;
            }
            // Un-trash before the state is saved: if this cycle dies later the
            // op is skipped next time (hlc <= head) and the note would stay deleted.
            if row.as_ref().map(|r| r.deleted != 0).unwrap_or(false) {
                restore_note(&id, core).await?;
                outcome.restored.push(id.clone());
            }
            save_note_state(
                db,
                &NoteSyncState {
                    note_id: id.clone(),
                    head_blob: payload.blob.clone(),
                    head_parents: payload.parents.clone(),
                    head_hlc: Some(op.hlc.clone()),
                    synced_hash: remote_hash,
                    ..Default::default()
                },
            )
            .await?;
            if !same_bytes {
                outcome.changed.push(id);
            }
            continue;
        }

        let mut st = prev.expect("diverged implies a known head");
        if remote_includes_ours && !st.conflict {
            // Plain race with the editor: next cycle pushes the edit first, then merges.
            outcome.retry = Some(format!("note {id} changed during sync"));
            continue;
        }
        let local_raw = local_raw.unwrap_or_default();
        let remote_device = op.hlc.device_id.clone();

        // A newer remote version of an already conflicted note replaces the remote side.
        if st.conflict && !remote_includes_ours {
            let (rk, _, rbody) = parse_note_file(&String::from_utf8_lossy(&remote_raw));
            let title = rk.get("title").cloned().unwrap_or_default();
            st.conflict_remote_id = history_snapshot_by(
                &id,
                &title,
                &rbody,
                "conflict",
                None,
                Some(&remote_device),
                db,
            )
            .await?;
            st.conflict_remote_blob = payload.blob.clone();
            st.head_hlc = Some(op.hlc.clone());
            save_note_state(db, &st).await?;
            continue;
        }

        // Exact common ancestor only: our head when the remote builds on it,
        // otherwise a blob both heads list as parent.
        let ancestor_name = if remote_includes_ours {
            Some(st.head_blob.clone())
        } else {
            payload
                .parents
                .iter()
                .find(|p| st.head_parents.contains(p))
                .cloned()
        };
        let ancestor_raw = match &ancestor_name {
            Some(name) => engine.get_blob(name).await.map_err(AppError::other)?,
            None => None,
        };

        let (lk, ltags, lbody) = parse_note_file(&String::from_utf8_lossy(&local_raw));
        let (rk, rtags, rbody) = parse_note_file(&String::from_utf8_lossy(&remote_raw));
        let local_title = lk.get("title").cloned().unwrap_or_default();
        let remote_title = rk.get("title").cloned().unwrap_or_default();

        // Ancestor unknown (compacted away or never shared) and nothing edited
        // here since the last sync: the newer op wins on both devices, the
        // local version stays recoverable from history. A fast-forward would
        // let two real forks swap contents forever.
        let Some(ancestor_raw) = ancestor_raw else {
            if !local_changed {
                history_snapshot_by(&id, &local_title, &lbody, "conflict", None, None, db).await?;
                write_raw(&path, &remote_raw)?;
                save_note_state(
                    db,
                    &NoteSyncState {
                        note_id: id.clone(),
                        head_blob: payload.blob.clone(),
                        head_parents: payload.parents.clone(),
                        head_hlc: Some(op.hlc.clone()),
                        synced_hash: remote_hash,
                        ..Default::default()
                    },
                )
                .await?;
                outcome.changed.push(id);
                continue;
            }
            record_conflict(
                &mut st,
                &id,
                "",
                &local_title,
                &lbody,
                &remote_title,
                &rbody,
                &remote_device,
                &payload.blob,
                &op.hlc,
                db,
            )
            .await?;
            continue;
        };

        let (ak, _, abody) = parse_note_file(&String::from_utf8_lossy(&ancestor_raw));
        let merged = merge3(&abody, &lbody, &rbody);
        if merged.has_conflicts {
            record_conflict(
                &mut st,
                &id,
                &abody,
                &local_title,
                &lbody,
                &remote_title,
                &rbody,
                &remote_device,
                &payload.blob,
                &op.hlc,
                db,
            )
            .await?;
            continue;
        }

        // Independent edits: apply the merge and keep every side restorable.
        let title = if lk.get("title") != ak.get("title") {
            local_title.clone()
        } else {
            remote_title.clone()
        };
        let tags = merge_tags(&ltags, &rtags);
        let mut note_row = sqlx::query_as::<_, NoteRow>("SELECT * FROM notes WHERE id = ?")
            .bind(&id)
            .fetch_optional(db)
            .await
            .map_err(AppError::db)?
            .ok_or_else(|| AppError::not_found(format!("Note {id}")))?;
        note_row.title = title.clone();
        note_row.updated_at = Utc::now().to_rfc3339();

        let local_id =
            history_snapshot_by(&id, &local_title, &lbody, "sync", None, None, db).await?;
        history_snapshot_by(
            &id,
            &remote_title,
            &rbody,
            "sync",
            None,
            Some(&remote_device),
            db,
        )
        .await?;
        history_snapshot_by(
            &id,
            &title,
            &merged.content,
            "merge",
            Some(local_id),
            None,
            db,
        )
        .await?;
        write_note_file(&path, &note_row, &tags, &merged.content)?;

        let merged_raw = std::fs::read(&path).map_err(AppError::io)?;
        let blob = engine
            .put_blob(&merged_raw)
            .await
            .map_err(AppError::other)?;
        let parents = vec![st.head_blob.clone(), payload.blob.clone()];
        let hlc = clock.now();
        engine
            .push(
                local,
                vec![put_op(
                    &id,
                    hlc.clone(),
                    blob.clone(),
                    parents.clone(),
                    &note_row.format,
                    &title,
                )],
            )
            .await
            .map_err(AppError::other)?;
        veydan_sync_host::state::save_own_state(db, local, &clock.last()).await?;
        save_note_state(
            db,
            &NoteSyncState {
                note_id: id.clone(),
                head_blob: blob,
                head_parents: parents,
                head_hlc: Some(hlc),
                synced_hash: sha256_hex(&merged_raw),
                ..Default::default()
            },
        )
        .await?;
        outcome.changed.push(id);
    }

    // The clock already observed every pulled op, so these tombstones sort after the puts.
    if !reassert.is_empty() {
        let mut ops = Vec::new();
        let mut states = Vec::new();
        for id in &reassert {
            let mut st = load_note_state(db, id)
                .await?
                .unwrap_or_else(|| NoteSyncState {
                    note_id: id.clone(),
                    ..Default::default()
                });
            let hlc = clock.now();
            ops.push(delete_op(id, hlc.clone(), head_parents(&st)));
            st.deleted = true;
            st.conflict = false;
            st.head_hlc = Some(hlc);
            states.push(st);
        }
        engine.push(local, ops).await.map_err(AppError::other)?;
        veydan_sync_host::state::save_own_state(db, local, &clock.last()).await?;
        for st in &states {
            save_note_state(db, st).await?;
        }
    }
    Ok(outcome)
}

/// Keep the local file readable; ancestor and both sides go to history for the UI.
#[allow(clippy::too_many_arguments)]
async fn record_conflict(
    st: &mut NoteSyncState,
    id: &str,
    abody: &str,
    local_title: &str,
    lbody: &str,
    remote_title: &str,
    rbody: &str,
    remote_device: &str,
    remote_blob: &str,
    hlc: &Hlc,
    db: &sqlx::SqlitePool,
) -> CmdResult<()> {
    let ancestor_id =
        history_snapshot_by(id, local_title, abody, "conflict", None, None, db).await?;
    let local_id = history_snapshot_by(
        id,
        local_title,
        lbody,
        "conflict",
        Some(ancestor_id.clone()),
        None,
        db,
    )
    .await?;
    let remote_id = history_snapshot_by(
        id,
        remote_title,
        rbody,
        "conflict",
        Some(ancestor_id.clone()),
        Some(remote_device),
        db,
    )
    .await?;
    st.conflict = true;
    st.conflict_ancestor_id = ancestor_id;
    st.conflict_local_id = local_id;
    st.conflict_remote_id = remote_id;
    st.conflict_remote_blob = remote_blob.to_string();
    st.head_hlc = Some(hlc.clone());
    save_note_state(db, st).await
}

/// Tombstone parents: the last known head, if any.
fn head_parents(st: &NoteSyncState) -> Vec<String> {
    std::iter::once(st.head_blob.clone())
        .filter(|b| !b.is_empty())
        .collect()
}

/// True when this device trashed the note: its own tombstone is the head, or
/// the row went to the trash after the tombstone was collected.
fn deleted_here(engine: &Engine, prev: Option<&NoteSyncState>, row: Option<&NoteHead>) -> bool {
    match prev {
        Some(s) if s.deleted => s
            .head_hlc
            .as_ref()
            .map(|h| h.device_id == engine.device_id())
            .unwrap_or(false),
        _ => row.map(|r| r.deleted != 0).unwrap_or(false),
    }
}

/// Conflict snapshot for the UI; `token` ties a resolution to this exact state.
#[derive(Debug, Serialize)]
pub struct ConflictView {
    pub token: String,
    pub merge: MergeResult,
    /// Name from sync settings. Empty when the user has not set one.
    pub local_device: String,
    /// Name of the device that wrote the remote side. Empty when unknown.
    pub remote_device: String,
}

/// Unresolved conflict state plus the current local body.
async fn load_conflict(core: &Core, note_id: &str) -> CmdResult<(NoteSyncState, String)> {
    let db = &core.db;
    let st = load_note_state(db, note_id)
        .await?
        .filter(|s| s.conflict)
        .ok_or_else(|| AppError::not_found(format!("conflict for note {note_id}")))?;
    let row = load_head(db, note_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Note {note_id}")))?;
    let path = resolve_note_abs_path(&core.app_data_dir, &row.file_path);
    let (_, _, local) = parse_note_file(&std::fs::read_to_string(&path).unwrap_or_default());
    Ok((st, local))
}

/// Changes when either side of the conflict or the local text changes.
fn conflict_token(st: &NoteSyncState, local: &str) -> String {
    let mut seed = String::new();
    for part in [
        &st.conflict_ancestor_id,
        &st.conflict_local_id,
        &st.conflict_remote_id,
        &st.conflict_remote_blob,
    ] {
        seed.push_str(part);
        seed.push('\n');
    }
    seed.push_str(&sha256_hex(local.as_bytes()));
    sha256_hex(seed.as_bytes())
}

/// Device id stored on the remote conflict snapshot.
async fn history_device(db: &sqlx::SqlitePool, history_id: &str) -> String {
    if history_id.is_empty() {
        return String::new();
    }
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT device FROM note_history WHERE id = ?")
            .bind(history_id)
            .fetch_optional(db)
            .await
            .unwrap_or(None);
    row.and_then(|(id,)| id).unwrap_or_default()
}

/// Human name for a device id from the last sync device list. Never the raw id.
fn cached_device_name(raw: &str, device_id: &str) -> String {
    if device_id.is_empty() {
        return String::new();
    }
    #[derive(Deserialize)]
    struct Named {
        id: String,
        #[serde(default)]
        name: String,
    }
    let list: Vec<Named> = serde_json::from_str(raw).unwrap_or_default();
    list.into_iter()
        .find(|d| d.id == device_id)
        .map(|d| d.name.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_default()
}

/// Merge data for the conflict UI: ancestor and remote from history, local from the file.
pub async fn conflict_merge(core: &Core, note_id: &str) -> CmdResult<ConflictView> {
    let db = &core.db;
    let (st, local) = load_conflict(core, note_id).await?;
    let ancestor = history_content_by_id(&st.conflict_ancestor_id, db)
        .await
        .unwrap_or_default();
    let remote = history_content_by_id(&st.conflict_remote_id, db).await?;
    let local_device = settings::get(db, "sync_device_name")
        .await
        .unwrap_or_default()
        .trim()
        .to_string();
    let remote_id = history_device(db, &st.conflict_remote_id).await;
    let devices = settings::get(db, "sync_devices").await.unwrap_or_default();
    Ok(ConflictView {
        token: conflict_token(&st, &local),
        merge: merge3(&ancestor, &local, &remote).or_whole_texts(&local, &remote),
        local_device,
        remote_device: cached_device_name(&devices, &remote_id),
    })
}

/// Write the user's resolution and release the note for the next push,
/// which records the remote version as second parent. Refuses a stale token.
pub async fn resolve_conflict(
    core: &Core,
    notes: &NotesState,
    note_id: &str,
    token: &str,
    content: String,
) -> CmdResult<()> {
    let db = &core.db;
    let (mut st, local) = load_conflict(core, note_id).await?;
    if conflict_token(&st, &local) != token {
        return Err(AppError::conflict_changed(format!("note {note_id}")));
    }
    update_note(
        note_id,
        NoteUpdateInput {
            title: None,
            content: Some(content),
            pinned: None,
            base_hash: None,
        },
        core,
        notes,
    )
    .await?;
    st.conflict = false;
    st.conflict_ancestor_id.clear();
    st.conflict_local_id.clear();
    st.conflict_remote_id.clear();
    save_note_state(db, &st).await
}

/// Reindex changed files, relink tags from frontmatter, notify the UI of the app.
pub async fn finish_apply(
    app: Option<&AppHandle>,
    core: &Core,
    notes: &NotesState,
    outcome: &ApplyOutcome,
) -> CmdResult<()> {
    reindex(core, notes, outcome).await?;
    let Some(app) = app else {
        return Ok(());
    };
    for id in outcome.changed.iter().chain(
        outcome
            .restored
            .iter()
            .filter(|r| !outcome.changed.contains(r)),
    ) {
        let _ = app.emit("notes://external-change", id);
    }
    Ok(())
}

/// Bring the index and the tag links in line with the files an apply changed.
async fn reindex(core: &Core, notes: &NotesState, outcome: &ApplyOutcome) -> CmdResult<()> {
    if outcome.changed.is_empty() {
        return Ok(());
    }
    let custom = notes.custom_dir.read().ok().and_then(|g| g.clone());
    sync_notes_index(core, custom.as_ref()).await?;

    let mut tags_by_note: HashMap<String, Vec<String>> = HashMap::new();
    for id in &outcome.changed {
        if let Some(row) = load_head(&core.db, id).await? {
            let path = resolve_note_abs_path(&core.app_data_dir, &row.file_path);
            if let Ok(raw) = std::fs::read_to_string(&path) {
                let (kv, tags, _) = parse_note_file(&raw);
                // Skip files we could not parse so we do not wipe existing links.
                if kv.contains_key("id") {
                    tags_by_note.insert(id.clone(), tags);
                }
            }
        }
    }
    for (id, tags) in &tags_by_note {
        set_note_tag_links(id, tags, &core.db).await?;
    }
    Ok(())
}

/// Note files as an entity of sync, with a handler of their own.
pub(crate) struct Notes;

#[async_trait::async_trait]
impl Handler for Notes {
    fn entities(&self) -> &'static [&'static str] {
        &[ENTITY]
    }

    fn state_tables(&self) -> &'static [&'static str] {
        &["sync_note_state"]
    }

    async fn collect(&self, cx: &mut Cycle<'_>) -> CmdResult<Collected> {
        let notes = cx
            .host
            .state::<NotesState>()
            .ok_or_else(|| AppError::other("notes are not set up"))?;
        let changes = collect_local_changes(cx.engine, cx.core, notes, cx.clock).await?;
        let states = changes.states;
        Ok(Collected {
            ops: changes.ops,
            commit: Some(Box::new(move |db| {
                Box::pin(async move {
                    for st in &states {
                        save_note_state(&db, st).await?;
                    }
                    Ok(())
                })
            })),
            ..Collected::default()
        })
    }

    async fn apply(&self, cx: &mut Cycle<'_>, ops: &[Op]) -> CmdResult<Applied> {
        let notes = cx
            .host
            .state::<NotesState>()
            .ok_or_else(|| AppError::other("notes are not set up"))?;
        let outcome = apply_remote(
            cx.engine,
            cx.host.app(),
            cx.core,
            notes,
            ops.to_vec(),
            cx.clock,
            cx.local,
        )
        .await?;
        Ok(Applied {
            retry: outcome.retry.clone(),
            skipped: outcome.skipped.clone(),
            detail: Some(Box::new(outcome)),
        })
    }

    async fn finish(&self, cx: &mut Cycle<'_>, applied: &Applied) -> CmdResult<()> {
        let notes = cx
            .host
            .state::<NotesState>()
            .ok_or_else(|| AppError::other("notes are not set up"))?;
        let none = ApplyOutcome::default();
        let outcome = applied
            .detail
            .as_ref()
            .and_then(|detail| detail.downcast_ref::<ApplyOutcome>())
            .unwrap_or(&none);
        finish_apply(cx.host.app(), cx.core, notes, outcome).await
    }

    /// The note and its parents: the ancestor stays available for 3-way merges.
    async fn blob_refs(&self, _engine: &Engine, op: &Op) -> CmdResult<Vec<String>> {
        let mut refs: Vec<String> = op
            .payload
            .get("blob")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .into_iter()
            .collect();
        if let Some(parents) = op.payload.get("parents").and_then(|v| v.as_array()) {
            refs.extend(
                parents
                    .iter()
                    .filter_map(|p| p.as_str())
                    .map(str::to_string),
            );
        }
        Ok(refs)
    }
}

/// The `note` op is part of the vault's wire format, frozen at what a 4.0.7
/// client writes and reads.
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hlc() -> Hlc {
        Hlc {
            wall_ms: 1,
            counter: 0,
            device_id: "dev-a".into(),
        }
    }

    #[test]
    fn a_put_of_4_0_7_is_read_and_written_alike() {
        let wire =
            json!({ "blob": "b2", "parents": ["b1", "b0"], "format": "md", "title": "Deploy" });

        let read: NotePayload = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(read.blob, "b2");
        assert_eq!(read.parents, vec!["b1", "b0"]);
        assert_eq!(read.format, "md");
        assert_eq!(read.title, "Deploy");

        let op = put_op(
            "n1",
            hlc(),
            "b2".into(),
            vec!["b1".into(), "b0".into()],
            "md",
            "Deploy",
        );
        assert_eq!(op.entity_type, "note");
        assert_eq!(op.entity_id, "n1");
        assert!(!op.deleted);
        assert_eq!(op.payload, wire);
    }

    #[test]
    fn a_delete_of_4_0_7_is_read_and_written_alike() {
        let wire = json!({ "blob": "", "parents": ["b2"], "format": "", "title": "" });

        let read: NotePayload = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(read.parents, vec!["b2"]);

        let op = delete_op("n1", hlc(), vec!["b2".into()]);
        assert_eq!(op.entity_type, "note");
        assert!(op.deleted);
        assert_eq!(op.payload, wire);
    }

    /// Every `note` op 4.0.7 wrote to a vault, puts and tombstones, reads
    /// into the payload and comes out of it as 4.0.7 wrote it.
    #[test]
    fn every_note_op_4_0_7_wrote_is_read_and_written_alike() {
        let ops = crate::sync::golden::ops(ENTITY);
        assert!(ops.iter().any(|op| op.deleted) && ops.iter().any(|op| !op.deleted));
        for theirs in ops {
            let read: NotePayload = serde_json::from_value(theirs.payload.clone()).unwrap();
            let ours = if theirs.deleted {
                delete_op(&theirs.entity_id, theirs.hlc.clone(), read.parents)
            } else {
                put_op(
                    &theirs.entity_id,
                    theirs.hlc.clone(),
                    read.blob,
                    read.parents,
                    &read.format,
                    &read.title,
                )
            };
            assert_eq!(
                serde_json::to_value(&ours).unwrap(),
                serde_json::to_value(&theirs).unwrap()
            );
        }
    }

    #[test]
    fn a_payload_with_fields_missing_or_unknown_is_read() {
        let read: NotePayload =
            serde_json::from_value(json!({ "blob": "b1", "added_later": true })).unwrap();
        assert_eq!(read.blob, "b1");
        assert!(read.parents.is_empty());
        assert_eq!(read.format, "md");
    }
}

/// What a pull leaves on this device, against a vault in a folder.
#[cfg(all(test, desktop))]
mod apply_tests {
    use super::*;
    use crate::testing::States as TestStates;
    use crate::testing::PLAN as SYNC_PLAN;
    use serde_json::{json, Value};
    use veydan_sync::{LocalDir, Vmk};
    use veydan_sync_host::rows;

    const NOTE: &str = "0b0e3c1e-5a52-4d0e-9a53-0d7e3d7c1a01";

    fn at(wall_ms: u64, device: &str) -> Hlc {
        Hlc {
            wall_ms,
            counter: 0,
            device_id: device.into(),
        }
    }

    fn note_file(body: &str) -> Vec<u8> {
        format!(
            "---\nid: {NOTE}\ntitle: Deploy\nformat: md\nbindings: []\ntags:\n  []\ncreated_at: 2026-09-01T10:00:00+00:00\nupdated_at: 2026-09-02T11:30:00+00:00\n---\n\n{body}"
        )
        .into_bytes()
    }

    /// A vault in a folder of its own and the devices that write to it.
    struct Vault {
        dir: PathBuf,
        vmk: Vmk,
        id: String,
    }

    impl Vault {
        async fn new() -> Self {
            let dir =
                std::env::temp_dir().join(format!("veydan-vault-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let (engine, vmk) =
                Engine::create(Box::new(LocalDir::new(&dir)), "passphrase", "creator")
                    .await
                    .unwrap();
            Self {
                id: engine.vault_id().to_string(),
                dir,
                vmk,
            }
        }

        fn device(&self, id: &str) -> Engine {
            Engine::with_key(Box::new(LocalDir::new(&self.dir)), &self.vmk, &self.id, id)
        }
    }

    /// This device: its state, its engine and what a cycle carries over.
    struct Device {
        state: TestStates,
        engine: Engine,
        local: LocalState,
        clock: HlcClock,
    }

    impl Device {
        async fn joined(vault: &Vault) -> Self {
            Self {
                state: TestStates::new().await,
                engine: vault.device("joiner"),
                local: LocalState::default(),
                clock: HlcClock::new("joiner", None),
            }
        }

        /// Pull and apply, as a cycle does; `index` is what follows a clean apply.
        async fn pull(&mut self, index: bool) -> ApplyOutcome {
            let ops = self.pulled().await;
            self.apply(ops, index).await
        }

        /// Pull and apply the notes and their rows in the order of a cycle:
        /// tags and folders, the notes, the index, then flags and folder membership.
        async fn pull_with_rows(&mut self) {
            let ops = self.pulled().await;
            let catalog = self.apply_rows("notes catalog", &ops).await;
            assert_eq!(catalog.retry, None);
            self.apply(ops.clone(), true).await;
            let meta = self.apply_rows("notes meta", &ops).await;
            assert_eq!(meta.retry, None);
        }

        /// The rows of the apply step `label` of the cycle.
        async fn apply_rows(&self, label: &str, ops: &[Op]) -> rows::ApplyOutcome {
            let tables = self.state.sync.registry().step_rows(SYNC_PLAN.apply, label);
            rows::apply_ops(&self.state.host(), &self.state.core, &tables, ops)
                .await
                .unwrap()
        }

        async fn pulled(&mut self) -> Vec<Op> {
            let pulled = self
                .engine
                .pull(&mut self.local, |_, _, _| {})
                .await
                .unwrap();
            assert!(pulled.errors.is_empty(), "{:?}", pulled.errors);
            for op in &pulled.ops {
                self.clock.observe(&op.hlc);
            }
            pulled.ops
        }

        async fn apply(&mut self, ops: Vec<Op>, index: bool) -> ApplyOutcome {
            let outcome = apply_ops(
                &self.engine,
                &self.state.core,
                &self.state.notes,
                ops,
                &mut self.clock,
                &mut self.local,
                |_, _, _| {},
            )
            .await
            .unwrap();
            if index {
                reindex(&self.state.core, &self.state.notes, &outcome)
                    .await
                    .unwrap();
            }
            outcome
        }

        /// What the next cycle would publish of the rows that belong to notes.
        async fn next_row_push(&mut self) -> Vec<Op> {
            let tables = self
                .state
                .sync
                .registry()
                .step_rows(SYNC_PLAN.collect, "note rows");
            rows::collect_local_changes(&self.state.core, &mut self.clock, &tables, None)
                .await
                .unwrap()
                .ops
        }

        /// The pin and the folders of the note, `None` while it has no row.
        async fn flags(&self) -> Option<(i64, Vec<String>)> {
            let pinned: Option<i64> = sqlx::query_scalar("SELECT pinned FROM notes WHERE id = ?")
                .bind(NOTE)
                .fetch_optional(&self.state.core.db)
                .await
                .unwrap();
            let folders: Vec<String> =
                sqlx::query_scalar("SELECT folder_id FROM note_folder_links WHERE note_id = ?")
                    .bind(NOTE)
                    .fetch_all(&self.state.core.db)
                    .await
                    .unwrap();
            pinned.map(|p| (p, folders))
        }

        /// The index scan the notes screen runs, then what the next cycle would push.
        async fn next_push(&mut self) -> Vec<Op> {
            sync_notes_index(&self.state.core, None).await.unwrap();
            collect_local_changes(
                &self.engine,
                &self.state.core,
                &self.state.notes,
                &mut self.clock,
            )
            .await
            .unwrap()
            .ops
        }

        fn files(&self) -> HashSet<String> {
            file_stems(&docs_dir(&self.state.core, &self.state.notes))
        }

        async fn row_deleted(&self) -> Option<i64> {
            sqlx::query_scalar("SELECT deleted FROM notes WHERE id = ?")
                .bind(NOTE)
                .fetch_optional(&self.state.core.db)
                .await
                .unwrap()
        }

        async fn note_state(&self) -> NoteSyncState {
            load_note_state(&self.state.core.db, NOTE)
                .await
                .unwrap()
                .expect("sync state")
        }

        async fn close(self, vault: Vault) {
            self.state.core.db.close().await;
            let _ = std::fs::remove_dir_all(&self.state.core.app_data_dir);
            let _ = std::fs::remove_dir_all(&vault.dir);
        }
    }

    /// A peer that published the note and later deleted it.
    async fn publish_then_delete(vault: &Vault) -> String {
        let peer = vault.device("peer-a");
        let mut log = LocalState::default();
        let blob = peer.put_blob(&note_file("body")).await.unwrap();
        let put = put_op(
            NOTE,
            at(1000, "peer-a"),
            blob.clone(),
            vec![],
            "md",
            "Deploy",
        );
        peer.push(&mut log, vec![put]).await.unwrap();
        let delete = delete_op(NOTE, at(2000, "peer-a"), vec![blob.clone()]);
        peer.push(&mut log, vec![delete]).await.unwrap();
        blob
    }

    /// The labels of the notes on this device, as `(key, name)`.
    async fn labels(device: &Device) -> Vec<(String, String)> {
        sqlx::query_as("SELECT key, name FROM labels ORDER BY key")
            .fetch_all(&device.state.core.db)
            .await
            .unwrap()
    }

    /// Spec 10.2: a note that comes from sync, or is renamed there, is named
    /// in the labels as one written here is; the tombstone of a note another
    /// device moved to its trash moves it to the trash, where it keeps its
    /// label.
    #[tokio::test]
    async fn a_note_from_sync_is_labelled_with_its_title() {
        let vault = Vault::new().await;
        let peer = vault.device("peer-a");
        let mut log = LocalState::default();
        let first = peer.put_blob(&note_file("body")).await.unwrap();
        let put = put_op(
            NOTE,
            at(1000, "peer-a"),
            first.clone(),
            vec![],
            "md",
            "Deploy",
        );
        peer.push(&mut log, vec![put]).await.unwrap();
        let mut device = Device::joined(&vault).await;

        device.pull(true).await;
        assert_eq!(
            labels(&device).await,
            [(format!("note:{NOTE}"), "Deploy".to_string())]
        );

        let renamed = String::from_utf8(note_file("body, edited"))
            .unwrap()
            .replace("title: Deploy", "title: Deploy to prod");
        let second = peer.put_blob(renamed.as_bytes()).await.unwrap();
        let put = put_op(
            NOTE,
            at(2000, "peer-a"),
            second.clone(),
            vec![first],
            "md",
            "Deploy to prod",
        );
        peer.push(&mut log, vec![put]).await.unwrap();
        device.pull(true).await;
        assert_eq!(
            labels(&device).await,
            [(format!("note:{NOTE}"), "Deploy to prod".to_string())]
        );

        let delete = delete_op(NOTE, at(3000, "peer-a"), vec![second]);
        peer.push(&mut log, vec![delete]).await.unwrap();
        device.pull(true).await;
        assert_eq!(device.row_deleted().await, Some(1));
        assert_eq!(labels(&device).await.len(), 1);

        device.close(vault).await;
    }

    /// The label of `NOTE` as an op of the device that owns the note.
    fn label_op(hlc: Hlc, title: Option<&str>) -> Op {
        Op {
            entity_type: veydan_sync_host::LABEL_ENTITY.into(),
            entity_id: format!("note:{NOTE}"),
            hlc,
            deleted: title.is_none(),
            payload: title.map_or(Value::Null, |title| {
                json!({ "kind": "note", "id": NOTE, "name": title,
                        "parent_kind": null, "parent_id": null, "color": null })
            }),
        }
    }

    /// Spec 10.2: a note deleted for good on another device goes to the trash
    /// here, and the tombstone of its label that comes with it takes the
    /// label: the deleting device's word. Taking the note out of the trash
    /// names it again.
    #[tokio::test]
    async fn a_note_deleted_for_good_elsewhere_is_in_the_trash_here_without_its_label() {
        let vault = Vault::new().await;
        let peer = vault.device("peer-a");
        let mut log = LocalState::default();
        let blob = peer.put_blob(&note_file("body")).await.unwrap();
        let put = put_op(
            NOTE,
            at(1000, "peer-a"),
            blob.clone(),
            vec![],
            "md",
            "Deploy",
        );
        let label = label_op(at(1001, "peer-a"), Some("Deploy"));
        peer.push(&mut log, vec![put, label]).await.unwrap();
        let mut device = Device::joined(&vault).await;
        let ops = device.pulled().await;
        device.apply(ops.clone(), true).await;
        assert_eq!(device.apply_rows("system rows", &ops).await.retry, None);
        assert_eq!(
            labels(&device).await,
            [(format!("note:{NOTE}"), "Deploy".to_string())]
        );

        let delete = delete_op(NOTE, at(2000, "peer-a"), vec![blob]);
        let retract = label_op(at(2001, "peer-a"), None);
        peer.push(&mut log, vec![delete, retract]).await.unwrap();
        let ops = device.pulled().await;
        device.apply(ops.clone(), true).await;
        assert_eq!(device.apply_rows("system rows", &ops).await.retry, None);
        assert_eq!(device.row_deleted().await, Some(1));
        assert!(labels(&device).await.is_empty());

        crate::crud::restore_note(NOTE, &device.state.core)
            .await
            .unwrap();
        assert_eq!(
            labels(&device).await,
            [(format!("note:{NOTE}"), "Deploy".to_string())]
        );

        device.close(vault).await;
    }

    #[tokio::test]
    async fn a_note_deleted_in_the_vault_stays_deleted_on_a_device_that_joins() {
        let vault = Vault::new().await;
        publish_then_delete(&vault).await;
        let mut device = Device::joined(&vault).await;

        device.pull(true).await;

        let pushed = device.next_push().await;
        assert!(pushed.is_empty(), "the note is published again: {pushed:?}");
        assert!(device.files().is_empty(), "{:?}", device.files());
        assert_eq!(device.row_deleted().await, None);
        let st = device.note_state().await;
        assert!(st.deleted);
        assert_eq!(st.head_hlc, Some(at(2000, "peer-a")));

        device.close(vault).await;
    }

    #[tokio::test]
    async fn a_tombstone_removes_the_file_of_a_cycle_that_died_before_the_index() {
        let vault = Vault::new().await;
        let peer = vault.device("peer-a");
        let mut log = LocalState::default();
        let blob = peer.put_blob(&note_file("body")).await.unwrap();
        let put = put_op(
            NOTE,
            at(1000, "peer-a"),
            blob.clone(),
            vec![],
            "md",
            "Deploy",
        );
        peer.push(&mut log, vec![put]).await.unwrap();
        let mut device = Device::joined(&vault).await;

        device.pull(false).await;
        assert_eq!(device.files(), HashSet::from([NOTE.to_string()]));
        assert_eq!(device.row_deleted().await, None);

        let delete = delete_op(NOTE, at(2000, "peer-a"), vec![blob]);
        peer.push(&mut log, vec![delete]).await.unwrap();
        device.pull(true).await;

        let pushed = device.next_push().await;
        assert!(pushed.is_empty(), "the note is published again: {pushed:?}");
        assert!(device.files().is_empty());
        assert!(device.note_state().await.deleted);

        device.close(vault).await;
    }

    #[tokio::test]
    async fn a_put_that_arrives_after_the_tombstone_it_lost_to_is_dropped() {
        let vault = Vault::new().await;
        let (author, deleter) = (vault.device("peer-a"), vault.device("peer-b"));
        let blob = author.put_blob(&note_file("body")).await.unwrap();
        let mut device = Device::joined(&vault).await;

        // The log of the device that deleted the note is readable first.
        let delete = delete_op(NOTE, at(2000, "peer-b"), vec![blob.clone()]);
        deleter
            .push(&mut LocalState::default(), vec![delete])
            .await
            .unwrap();
        device.pull(true).await;
        assert!(device.note_state().await.deleted);

        let put = put_op(NOTE, at(1000, "peer-a"), blob, vec![], "md", "Deploy");
        author
            .push(&mut LocalState::default(), vec![put])
            .await
            .unwrap();
        device.pull(true).await;

        assert!(device.next_push().await.is_empty());
        assert!(device.files().is_empty(), "{:?}", device.files());
        assert_eq!(device.row_deleted().await, None);
        let st = device.note_state().await;
        assert!(st.deleted);
        assert_eq!(st.head_hlc, Some(at(2000, "peer-b")));

        device.close(vault).await;
    }

    #[tokio::test]
    async fn a_note_this_device_shows_goes_to_the_trash_with_its_file() {
        let vault = Vault::new().await;
        let peer = vault.device("peer-a");
        let mut log = LocalState::default();
        let blob = peer.put_blob(&note_file("body")).await.unwrap();
        let put = put_op(
            NOTE,
            at(1000, "peer-a"),
            blob.clone(),
            vec![],
            "md",
            "Deploy",
        );
        peer.push(&mut log, vec![put]).await.unwrap();
        let mut device = Device::joined(&vault).await;
        device.pull(true).await;
        assert_eq!(device.row_deleted().await, Some(0));

        let delete = delete_op(NOTE, at(2000, "peer-a"), vec![blob]);
        peer.push(&mut log, vec![delete]).await.unwrap();
        device.pull(true).await;

        assert_eq!(device.row_deleted().await, Some(1));
        assert_eq!(device.files(), HashSet::from([NOTE.to_string()]));
        assert!(device.next_push().await.is_empty());

        device.close(vault).await;
    }

    #[tokio::test]
    async fn a_tombstone_leaves_a_file_sync_did_not_write() {
        let vault = Vault::new().await;
        let peer = vault.device("peer-a");
        let mut log = LocalState::default();
        let blob = peer.put_blob(&note_file("body")).await.unwrap();
        let put = put_op(
            NOTE,
            at(1000, "peer-a"),
            blob.clone(),
            vec![],
            "md",
            "Deploy",
        );
        peer.push(&mut log, vec![put]).await.unwrap();
        let mut device = Device::joined(&vault).await;
        device.pull(false).await;
        // Same id, other text: the user put this file here, no pull did.
        let own = docs_dir(&device.state.core, &device.state.notes).join(format!("{NOTE}.txt"));
        std::fs::write(&own, note_file("written here")).unwrap();

        let delete = delete_op(NOTE, at(2000, "peer-a"), vec![blob]);
        peer.push(&mut log, vec![delete]).await.unwrap();
        device.pull(true).await;

        assert_eq!(std::fs::read(&own).unwrap(), note_file("written here"));
        assert!(!docs_dir(&device.state.core, &device.state.notes)
            .join(format!("{NOTE}.md"))
            .exists());

        device.close(vault).await;
    }

    #[tokio::test]
    async fn a_put_superseded_in_the_same_pull_is_not_fetched() {
        let vault = Vault::new().await;
        let peer = vault.device("peer-a");
        // The blob is not in the storage: fetching it would ask for a retry.
        let put = put_op(
            NOTE,
            at(1000, "peer-a"),
            "absent".into(),
            vec![],
            "md",
            "Deploy",
        );
        let delete = delete_op(NOTE, at(2000, "peer-a"), vec!["absent".into()]);
        peer.push(&mut LocalState::default(), vec![put, delete])
            .await
            .unwrap();
        let mut device = Device::joined(&vault).await;
        let own = docs_dir(&device.state.core, &device.state.notes).join(format!("{NOTE}.md"));
        std::fs::write(&own, note_file("written here")).unwrap();

        let outcome = device.pull(true).await;

        assert_eq!(outcome.retry, None);
        assert!(outcome.skipped.is_empty(), "{:?}", outcome.skipped);
        assert!(device.note_state().await.deleted);
        assert_eq!(std::fs::read(&own).unwrap(), note_file("written here"));

        device.close(vault).await;
    }

    #[tokio::test]
    async fn a_put_against_the_tombstone_of_this_device_is_answered_with_a_tombstone() {
        let vault = Vault::new().await;
        let mut device = Device::joined(&vault).await;
        save_note_state(
            &device.state.core.db,
            &NoteSyncState {
                note_id: NOTE.into(),
                head_blob: "b0".into(),
                head_hlc: Some(at(3000, "joiner")),
                deleted: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let peer = vault.device("peer-a");
        let blob = peer.put_blob(&note_file("edited elsewhere")).await.unwrap();
        let put = put_op(
            NOTE,
            at(1000, "peer-a"),
            blob,
            vec!["b0".into()],
            "md",
            "Deploy",
        );
        peer.push(&mut LocalState::default(), vec![put])
            .await
            .unwrap();
        device.pull(true).await;

        assert_eq!(device.local.own_seq, 1);
        let st = device.note_state().await;
        assert!(st.deleted);
        assert_eq!(st.head_hlc.map(|h| h.device_id), Some("joiner".to_string()));
        assert!(device.files().is_empty());

        device.close(vault).await;
    }

    #[tokio::test]
    async fn a_tombstone_removes_a_file_of_the_deleted_version_left_without_a_state() {
        let vault = Vault::new().await;
        let blob = publish_then_delete(&vault).await;
        let mut device = Device::joined(&vault).await;
        // An apply that stopped between writing the file and saving its state.
        let left = docs_dir(&device.state.core, &device.state.notes).join(format!("{NOTE}.md"));
        write_raw(&left, &note_file("body")).unwrap();
        assert_eq!(device.engine.blob_name(&note_file("body")), blob);

        device.pull(true).await;

        let pushed = device.next_push().await;
        assert!(pushed.is_empty(), "the note is published again: {pushed:?}");
        assert!(device.files().is_empty(), "{:?}", device.files());
        assert!(device.note_state().await.deleted);

        device.close(vault).await;
    }

    fn row_op(entity: &str, id: &str, hlc: Hlc, payload: Value) -> Op {
        Op {
            entity_type: entity.into(),
            entity_id: id.into(),
            hlc,
            deleted: false,
            payload,
        }
    }

    /// A peer that published the note in a folder and pinned, then deleted it.
    /// Returns the peer, its log and the blob of the note.
    async fn publish_pinned_then_delete(vault: &Vault) -> (Engine, LocalState, String) {
        let peer = vault.device("peer-a");
        let mut log = LocalState::default();
        let blob = peer.put_blob(&note_file("body")).await.unwrap();
        let folder = json!({
            "name": "DevOps", "parent_id": null, "color": "#6366f1",
            "created_at": "2026-09-01T10:00:00+00:00", "updated_at": "2026-09-01T10:00:00+00:00",
        });
        let flags = json!({ "pinned": 1, "archived": 0, "folder_ids": ["f-1"] });
        let ops = vec![
            put_op(
                NOTE,
                at(1000, "peer-a"),
                blob.clone(),
                vec![],
                "md",
                "Deploy",
            ),
            row_op("note_folder", "f-1", at(1100, "peer-a"), folder),
            row_op("note_meta", NOTE, at(1200, "peer-a"), flags),
            delete_op(NOTE, at(2000, "peer-a"), vec![blob.clone()]),
        ];
        peer.push(&mut log, ops).await.unwrap();
        (peer, log, blob)
    }

    #[tokio::test]
    async fn a_note_restored_elsewhere_has_the_flags_it_had_before_this_device_joined() {
        let vault = Vault::new().await;
        let (peer, mut log, blob) = publish_pinned_then_delete(&vault).await;
        let mut device = Device::joined(&vault).await;
        device.pull_with_rows().await;
        assert_eq!(device.flags().await, None);
        assert!(device.next_row_push().await.is_empty());

        // The device that held the note takes it out of the trash.
        let restore = put_op(
            NOTE,
            at(3000, "peer-a"),
            blob.clone(),
            vec![blob],
            "md",
            "Deploy",
        );
        peer.push(&mut log, vec![restore]).await.unwrap();
        device.pull_with_rows().await;

        assert_eq!(device.row_deleted().await, Some(0));
        assert_eq!(device.flags().await, Some((1, vec!["f-1".to_string()])));
        let pushed = device.next_row_push().await;
        assert!(
            pushed.is_empty(),
            "flags set nowhere are published: {pushed:?}"
        );

        device.close(vault).await;
    }
}
