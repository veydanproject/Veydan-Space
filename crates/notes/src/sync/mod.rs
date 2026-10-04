// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Notes in sync. Tags, folders, smart views and the flags of notes are
//! table entities; the files of notes and their attachments go through
//! handlers of their own:
//! - `notes`       — note files plus their merge and conflicts
//! - `attachments` — note attachments as sync entity (push / pull, LWW)
//! - `state`       — the sync positions of both
//! - `commands`    — conflicts, the sync position of a note, a transfer in flight

pub(crate) mod attachments;
pub(crate) mod commands;
#[cfg(test)]
mod golden;
pub(crate) mod notes;
pub(crate) mod state;

use veydan_sync_host::{blob_gone, config, emit_progress, open_engine, progress_pct, SyncManager};

use crate::tags::TAG_PLACEHOLDER_COLOR;
use serde_json::Value;
use sqlx::{Pool, Sqlite};
use veydan_core::{AppError, BoxFuture, CmdResult, Core};
use veydan_sync_host::{Delete, Deletion, Hooks, Host, LinkSpec, Ref, Registry, TableSpec, Upsert};

/// Pin, archive flag and folders of a note: columns and links of the row
/// the note stream owns. Flags nobody set on this device are not
/// published, and an op for a note that is not here waits for the note.
const NOTE_META: TableSpec = TableSpec {
    links: &[LinkSpec {
        table: "note_folder_links",
        parent_col: "note_id",
        child_col: "folder_id",
        key: "folder_ids",
        child_table: None,
    }],
    insert: false,
    delete: Delete::Ignore,
    hold: Some("sync_held_note_flags"),
    ..TableSpec::plain("note_meta", "notes", &["pinned", "archived"])
};

pub(crate) fn register(registry: &mut Registry) {
    registry.table(
        TableSpec {
            unique: Some(&["name"]),
            delete: Delete::Plain(&["DELETE FROM note_tag_links WHERE tag_id = ?"]),
            ..TableSpec::plain(
                "note_tag",
                "note_tags",
                &["name", "color", "created_at", "updated_at"],
            )
        },
        Hooks {
            before_upsert: Some(tag_keeps_its_color),
            on_delete: Some(tag_stays_while_linked),
            ..Hooks::default()
        },
    );
    registry.table(
        TableSpec {
            refs: &[Ref {
                key: "parent_id",
                table: "note_folders",
            }],
            delete: Delete::Plain(&[
                "DELETE FROM note_folder_links WHERE folder_id = ?",
                "UPDATE note_folders SET parent_id = NULL WHERE parent_id = ?",
            ]),
            ..TableSpec::plain(
                "note_folder",
                "note_folders",
                &["name", "parent_id", "color", "created_at", "updated_at"],
            )
        },
        Hooks::default(),
    );
    registry.table(
        TableSpec::plain(
            "note_smart_view",
            "note_smart_views",
            &[
                "name",
                "color",
                "conditions",
                "sort_order",
                "created_at",
                "updated_at",
            ],
        ),
        Hooks::default(),
    );
    registry.table(
        NOTE_META,
        Hooks {
            publish_new: Some(flags_set),
            ..Hooks::default()
        },
    );
    registry.handler("attachments", attachments::Attachments);
    registry.handler("notes", notes::Notes);
    registry.conflicts(state::conflicts);
    #[cfg(desktop)]
    registry.setting(QUICK_CAPTURE_SHORTCUT);
}

/// The key of the quick capture shortcut of 4.x and stage 10. The shortcut
/// is gone (platform-spec 20.18), but the key is one of the seven synced
/// setting keys, which the format of 4.0.7 freezes (9.5): it stays
/// registered, a value that comes is applied, and nothing reads or writes it.
#[cfg(desktop)]
const QUICK_CAPTURE_SHORTCUT: &str = "quick_capture_shortcut";

/// A tag auto-created by name carries the placeholder color; it must not
/// erase a chosen one.
fn tag_keeps_its_color<'a>(cx: &'a mut Upsert<'_>) -> BoxFuture<'a, CmdResult<()>> {
    Box::pin(async move {
        if cx.payload.get("color").and_then(Value::as_str) == Some(TAG_PLACEHOLDER_COLOR) {
            cx.keep_local.push("color");
        }
        Ok(())
    })
}

/// A tag still linked to local notes survives a tombstone: it was merged
/// by name under this id.
fn tag_stays_while_linked<'a>(
    _host: &'a Host<'a>,
    core: &'a Core,
    id: &'a str,
) -> BoxFuture<'a, CmdResult<Deletion>> {
    Box::pin(async move {
        Ok(if tag_in_use(&core.db, id).await? {
            Deletion::Keep
        } else {
            Deletion::Plain
        })
    })
}

async fn tag_in_use(db: &Pool<Sqlite>, tag_id: &str) -> CmdResult<bool> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM note_tag_links WHERE tag_id = ? LIMIT 1")
            .bind(tag_id)
            .fetch_optional(db)
            .await
            .map_err(AppError::db)?;
    Ok(row.is_some())
}

/// Flags nobody set here assert nothing. Published, they would be the
/// newer ones and replace what the vault holds for the note.
fn flags_set<'a>(_db: &'a Pool<Sqlite>, payload: &'a Value) -> BoxFuture<'a, bool> {
    Box::pin(async move { !no_flags(payload) })
}

/// A `note_meta` payload as a new note has it: not pinned, not archived, in no folder.
fn no_flags(payload: &Value) -> bool {
    let unset = |key: &str| payload.get(key).and_then(Value::as_i64).unwrap_or(0) == 0;
    let folders = payload.get("folder_ids").and_then(Value::as_array);
    unset("pinned") && unset("archived") && folders.is_none_or(|ids| ids.is_empty())
}
