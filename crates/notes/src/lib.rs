// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The module Notes: note files with their frontmatter and the index over
//! them (FTS), folders, tags, wiki links and mentions, history, drafts,
//! templates, smart views, attachments, and their sync; on a computer the
//! standalone notes window and the quick capture window.
//!
//! Notes know no other module. What a note shows of an entity of another
//! module — the name of a password, the color of a workspace, the host of a
//! proxy for a template — notes ask the entity directory for (`entities`),
//! and a binding to an entity that is gone stays as it is. A page the web
//! clipper captured comes in through [`capture::handle_capture`], which the
//! bridge of the product calls. Split by concern:
//! - `models`   — shared DTO/row structs
//! - `files`    — on-disk note files (frontmatter, atomic write, hashing, preview)
//! - `tags`     — tag helpers + tag CRUD commands
//! - `index`    — FTS index, manifest, filesystem sync + watcher
//! - `crud`     — note CRUD / search / draft commands
//! - `folders`  — folder CRUD + note↔folder / note↔binding commands
//! - `nav`      — navigation tree with counts (mobile sidebar)
//! - `settings` — notes directory and attachment policy commands
//! - `history`  — version history (snapshot, diff, restore, merge) + commands
//! - `merge`    — EOL-normalized 3-way merge with structured conflict blocks
//! - `window`   — standalone notes window (desktop)
//! - `attachments` — files stored next to notes, referenced by relative links
//! - `transfer` — export/import of Markdown + attachments (desktop)
//! - `capture`  — requests from the browser extension (desktop)
//! - `filter`   — backend evaluation of `NoteFilter`
//! - `smart_views` — saved filters
//! - `quick_capture` — small always-on-top window, from the tray and the palette (desktop)
//! - `links`    — wiki links, backlinks, related notes, entity mentions
//! - `binding`  — registry of `kind:value` binding kinds
//! - `entities` — names and public fields of the entities of other modules, from the directory
//! - `templates` — notes from the Templates folder with placeholders
//! - `sync`     — the entities of notes in sync, their handlers, conflicts and positions
//! - `directory` — the kind `note` in the entity directory
//! - `demo`     — the part of notes in the demo data

mod attachments;
mod binding;
#[cfg(desktop)]
pub mod capture;
mod crud;
mod demo;
mod directory;
pub mod entities;
mod files;
mod filter;
mod folders;
mod history;
mod index;
mod links;
mod merge;
mod models;
mod nav;
#[cfg(desktop)]
mod quick_capture;
mod settings;
mod smart_views;
mod sync;
mod tags;
mod templates;
#[cfg(desktop)]
mod transfer;
#[cfg(desktop)]
mod window;

pub use binding::BindingSummary;
pub use files::resolve_note_abs_path;
pub use nav::{nav, NoteNav};
pub use templates::placeholder_values;

#[cfg(desktop)]
use attachments::allow_asset_dir;
use attachments::{attachments_dir_for, is_staging_name, safe_file_name};
use crud::{insert_note, restore_note, trash_note, update_note, NewNote};
use files::{effective_docs_dir, parse_note_file, write_note_file};
use history::{history_content_by_id, history_snapshot_by};
use index::{rebuild_manifest, start_notes_watcher, sync_notes_index};
use links::reindex_links;
use merge::{merge3, MergeResult};
use models::{NoteFilter, NoteRow, NoteUpdateInput};
use settings::{load_attachment_policy, NoteAttachmentPolicy};
use tags::set_note_tag_links;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{Manager, Runtime};
use veydan_core::{AppError, BoxFuture, Core, Directory, Schema};
use veydan_shell::{BackupPart, BackupPath, DemoPart, Module, SetupResult};
#[cfg(desktop)]
use veydan_shell::{TrayGroup, TrayItem, TrayLabels, TrayPart};

/// The kind a note is in the entity directory and on the deletion hooks.
pub const KIND_NOTE: &str = "note";

/// The tables of notes. A data file made while these tables belonged to the
/// product crate holds them under the same module name and version: the
/// steps are those, unchanged, and nothing is applied again.
pub const SCHEMA: Schema = Schema {
    module: "notes",
    steps: &[concat!(
        "CREATE TABLE notes (
            id           TEXT PRIMARY KEY NOT NULL,
            title        TEXT NOT NULL,
            file_path    TEXT NOT NULL,
            format       TEXT NOT NULL DEFAULT 'md',
            pinned       INTEGER NOT NULL DEFAULT 0,
            archived     INTEGER NOT NULL DEFAULT 0,
            deleted      INTEGER NOT NULL DEFAULT 0,
            doc_status   TEXT NOT NULL DEFAULT 'active',
            version_base TEXT NULL,
            fts_rowid    INTEGER NULL,
            created_at   TEXT NOT NULL,
            updated_at   TEXT NOT NULL,
            file_mtime   TEXT NULL,
            content_hash TEXT NULL,
            preview      TEXT NOT NULL DEFAULT '',
            bindings     TEXT NOT NULL DEFAULT '[]'
        );",
        "CREATE TABLE note_tags (
            id         TEXT PRIMARY KEY NOT NULL,
            name       TEXT NOT NULL UNIQUE,
            color      TEXT NOT NULL DEFAULT '#6366f1',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
        "CREATE TABLE note_tag_links (
            note_id TEXT NOT NULL,
            tag_id  TEXT NOT NULL,
            PRIMARY KEY (note_id, tag_id)
        );",
        "CREATE VIRTUAL TABLE notes_fts
         USING fts5(note_id UNINDEXED, title, content, tags);",
        "CREATE TABLE note_folders (
            id         TEXT PRIMARY KEY NOT NULL,
            name       TEXT NOT NULL,
            parent_id  TEXT NULL REFERENCES note_folders(id),
            color      TEXT NOT NULL DEFAULT '#6366f1',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
        // Many-to-many notes ↔ folders
        "CREATE TABLE note_folder_links (
            note_id   TEXT NOT NULL,
            folder_id TEXT NOT NULL,
            PRIMARY KEY (note_id, folder_id)
        );",
        // Wiki links between notes, rebuilt from the body on save
        "CREATE TABLE note_links (
            from_id TEXT NOT NULL,
            to_id   TEXT NOT NULL,
            PRIMARY KEY (from_id, to_id)
        );",
        "CREATE INDEX idx_note_links_to ON note_links(to_id);",
        // Entity mentions `[[kind:id]]` in note bodies, rebuilt from the body on save
        "CREATE TABLE note_mentions (
            note_id TEXT NOT NULL,
            binding TEXT NOT NULL,
            PRIMARY KEY (note_id, binding)
        );",
        "CREATE INDEX idx_note_mentions_binding ON note_mentions(binding);",
        // Saved filters: conditions is a NoteFilter JSON
        "CREATE TABLE note_smart_views (
            id         TEXT PRIMARY KEY NOT NULL,
            name       TEXT NOT NULL,
            color      TEXT NOT NULL DEFAULT '#8b7bff',
            conditions TEXT NOT NULL DEFAULT '{}',
            sort_order INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
        // Note version history (DAG: parent_id links versions into a tree)
        "CREATE TABLE note_history (
            id           TEXT PRIMARY KEY,
            note_id      TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
            parent_id    TEXT NULL REFERENCES note_history(id),
            revision     INTEGER NOT NULL,
            version_type TEXT NOT NULL DEFAULT 'save',
            title        TEXT NOT NULL,
            content      BLOB NOT NULL,
            content_hash TEXT NOT NULL,
            author       TEXT NULL,
            device       TEXT NULL,
            created_at   TEXT NOT NULL
        );",
        "CREATE INDEX idx_nh_note_created ON note_history(note_id, created_at DESC);",
        "CREATE INDEX idx_nh_note_revision ON note_history(note_id, revision DESC);",
        // Per-note sync position: which vault version the local file corresponds
        // to. The `conflict_*` columns hold a pending conflict: history snapshots
        // of both sides and the remote blob to merge with.
        "CREATE TABLE sync_note_state (
            note_id              TEXT PRIMARY KEY NOT NULL,
            head_blob            TEXT NOT NULL DEFAULT '',
            head_parents         TEXT NOT NULL DEFAULT '[]',
            head_hlc             TEXT NOT NULL DEFAULT '',
            synced_hash          TEXT NOT NULL DEFAULT '',
            deleted              INTEGER NOT NULL DEFAULT 0,
            conflict             INTEGER NOT NULL DEFAULT 0,
            conflict_ancestor_id TEXT NOT NULL DEFAULT '',
            conflict_local_id    TEXT NOT NULL DEFAULT '',
            conflict_remote_id   TEXT NOT NULL DEFAULT '',
            conflict_remote_blob TEXT NOT NULL DEFAULT ''
        );",
        // Per-attachment sync position: which vault blob the local file corresponds
        // to. `deferred_ref` is a chunked attachment accepted from the vault but
        // not downloaded yet (LargeFileRef JSON).
        "CREATE TABLE sync_attachment_state (
            note_id      TEXT NOT NULL,
            name         TEXT NOT NULL,
            head_blob    TEXT NOT NULL DEFAULT '',
            head_hlc     TEXT NOT NULL DEFAULT '',
            synced_hash  TEXT NOT NULL DEFAULT '',
            deleted      INTEGER NOT NULL DEFAULT 0,
            deferred_ref TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (note_id, name)
        );",
    )],
};

pub fn module() -> Module {
    Module {
        start: Some(start),
        stop: Some(stop),
        schema: Some(SCHEMA),
        directory: Some(provide),
        sync: Some(sync::register),
        demo: Some(demo()),
        backup: Some(backup()),
        #[cfg(desktop)]
        tray: Some(TrayPart {
            items: tray_items,
            on_click: tray_click,
            tooltip: None,
        }),
        ..veydan_shell::module! {
            id: "notes",
            setup: setup,
            commands: [
                sync::commands::sync_conflict_get,
                sync::commands::sync_conflict_resolve,
                sync::commands::note_sync_info,
                crud::note_list,
                nav::note_nav,
                crud::note_get,
                crud::note_create,
                crud::note_update,
                crud::note_delete,
                crud::note_delete_many,
                crud::note_trash_empty,
                crud::note_archive,
                crud::note_restore,
                crud::note_set_tags,
                crud::note_search,
                crud::note_sync,
                crud::note_reindex,
                #[cfg(desktop)]
                crud::note_open_folder,
                #[cfg(desktop)]
                crud::note_open_external,
                crud::note_draft_save,
                crud::note_draft_get,
                crud::note_draft_discard,
                tags::note_tag_list,
                tags::note_tag_create,
                tags::note_tag_delete,
                tags::note_tag_update,
                folders::note_folder_list,
                folders::note_folder_create,
                folders::note_folder_update,
                folders::note_folder_delete,
                folders::note_add_folder,
                folders::note_remove_folder,
                folders::note_set_folder,
                folders::note_add_binding,
                folders::note_remove_binding,
                #[cfg(desktop)]
                settings::notes_get_dir,
                #[cfg(desktop)]
                settings::notes_set_dir,
                smart_views::note_smart_view_list,
                smart_views::note_smart_view_create,
                smart_views::note_smart_view_update,
                smart_views::note_smart_view_delete,
                #[cfg(desktop)]
                quick_capture::open_quick_capture,
                links::note_backlinks,
                links::note_links,
                links::note_entity_notes,
                binding::note_binding_summaries,
                binding::note_entity_search,
                templates::note_placeholder_values,
                links::note_related,
                links::note_resolve_link,
                #[cfg(desktop)]
                window::note_open_window,
                history::note_history_list,
                history::note_history_get,
                history::note_history_diff,
                history::note_history_restore,
                history::note_history_merge,
                #[cfg(desktop)]
                attachments::note_attachment_add,
                attachments::note_attachment_add_from_path,
                attachments::note_attachment_list,
                attachments::note_attachment_fetch,
                attachments::note_attachment_read,
                attachments::note_attachment_delete,
                #[cfg(desktop)]
                attachments::note_attachment_open,
                attachments::note_attachment_save,
                #[cfg(desktop)]
                attachments::note_attachments_gc,
                settings::notes_attachment_policy_get,
                settings::notes_attachment_policy_set,
                sync::commands::sync_attachment_cancel,
                #[cfg(desktop)]
                attachments::clipboard_file_paths,
                #[cfg(desktop)]
                transfer::note_export,
                #[cfg(desktop)]
                transfer::note_import,
            ],
        }
    }
}

/// The part of notes in the demo data, on any runtime; `module()` holds it
/// on the app's.
pub fn demo<R: Runtime>() -> DemoPart<R> {
    DemoPart {
        seed: |app, locale| {
            Box::pin(async move {
                demo::seed(&app.state::<Core>(), &app.state::<NotesState>(), &locale).await
            })
        },
        clear: |app| Box::pin(async move { demo::clear(&app.state::<Core>()).await }),
    }
}

/// The name of the user's folder of notes in a backup.
pub const BACKUP_CUSTOM_DIR: &str = "notes_custom";

/// The part of notes in a backup, on any runtime; `module()` holds it on the
/// app's: the folder `notes/` of the data directory (documents, attachments,
/// drafts) and the folder the user keeps the notes in, when there is one.
pub fn backup<R: Runtime>() -> BackupPart<R> {
    BackupPart {
        paths: backup_paths::<R>,
    }
}

fn backup_paths<R: Runtime>(app: &tauri::AppHandle<R>) -> Vec<BackupPath> {
    let mut paths = vec![BackupPath {
        name: "notes",
        path: PathBuf::from("notes"),
        external: false,
    }];
    let custom = app
        .try_state::<NotesState>()
        .and_then(|notes| notes.custom_dir.read().ok().and_then(|dir| dir.clone()));
    if let Some(dir) = custom {
        paths.push(BackupPath {
            name: BACKUP_CUSTOM_DIR,
            path: dir,
            external: true,
        });
    }
    paths
}

/// The kind of notes in the entity directory, on any runtime; `module()`
/// holds it on the app's.
pub fn provide<R: Runtime>(directory: &mut Directory<R>) {
    directory.provide(KIND_NOTE, directory::notes());
}

/// What the notes keep while the app runs.
pub struct NotesState {
    /// The folder the user keeps the notes in instead of the data directory.
    pub custom_dir: Arc<std::sync::RwLock<Option<PathBuf>>>,
    /// Notes dir watcher: none while the module is switched off; dropped
    /// before a backup restore releases its handle.
    pub watcher: Arc<Mutex<Option<notify::RecommendedWatcher>>>,
    /// Serializes note saves. `update_note` checks `base_hash` against the file,
    /// then awaits several times before writing; without this lock two concurrent
    /// saves can both pass the check and the later one silently overwrites the
    /// earlier. Matters for multiple windows, the capture bridge and sync apply.
    pub save: Arc<tokio::sync::Mutex<()>>,
}

impl NotesState {
    pub fn new(custom_dir: Option<PathBuf>) -> Self {
        Self {
            custom_dir: Arc::new(std::sync::RwLock::new(custom_dir)),
            watcher: Arc::new(Mutex::new(None)),
            save: Arc::new(tokio::sync::Mutex::new(())),
        }
    }
}

/// Create the notes directory layout under `data_dir`.
pub fn ensure_dirs(data_dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir.join("notes").join("documents"))?;
    std::fs::create_dir_all(data_dir.join("notes").join("attachments"))?;
    std::fs::create_dir_all(data_dir.join("notes").join("drafts"))
}

fn setup(app: &mut tauri::App) -> SetupResult {
    let data_dir = app.state::<Core>().app_data_dir.clone();
    ensure_dirs(&data_dir)?;
    // Load custom notes dir from settings (if set)
    #[cfg(desktop)]
    let custom_dir = {
        let db = app.state::<Core>().db.clone();
        tauri::async_runtime::block_on(veydan_core::settings::get(&db, "notes_custom_dir"))
            .map(PathBuf::from)
    };
    #[cfg(mobile)]
    let custom_dir = None;
    app.manage(NotesState::new(custom_dir.clone()));

    // Attachments are served through the asset protocol from both dirs.
    #[cfg(desktop)]
    {
        let handle = app.handle();
        allow_asset_dir(handle, &data_dir.join("notes").join("documents"));
        if let Some(ref custom) = custom_dir {
            allow_asset_dir(handle, custom);
        }
    }

    Ok(())
}

/// The id of the module, as the product lists it: its switch is the shell's.
#[cfg(desktop)]
const MODULE_ID: &str = "notes";

/// The module is on: the watcher of the notes folder. Nothing happens to
/// what runs already.
fn start(app: tauri::AppHandle) -> BoxFuture<'static, Result<(), AppError>> {
    Box::pin(async move { watch(&app) })
}

/// The watcher of the notes folder, unless one runs.
fn watch(app: &tauri::AppHandle) -> Result<(), AppError> {
    let notes = app.state::<NotesState>();
    let mut slot = notes
        .watcher
        .lock()
        .map_err(|e| AppError::other(e.to_string()))?;
    if slot.is_none() {
        let custom_dir = notes.custom_dir.read().ok().and_then(|dir| dir.clone());
        let data_dir = app.state::<Core>().app_data_dir.clone();
        *slot = start_notes_watcher(app.clone(), data_dir, custom_dir);
    }
    Ok(())
}

/// The module is off, or the app quits: the watcher goes, and on a computer
/// the windows of notes (quick capture, the notes window).
fn stop(app: tauri::AppHandle) -> BoxFuture<'static, Result<(), AppError>> {
    Box::pin(async move {
        if let Ok(mut slot) = app.state::<NotesState>().watcher.lock() {
            slot.take();
        }
        #[cfg(desktop)]
        {
            for label in [quick_capture::WINDOW_LABEL, window::NOTES_WINDOW_LABEL] {
                if let Some(window) = tauri::Manager::get_webview_window(&app, label) {
                    let _ = window.close();
                }
            }
        }
        Ok(())
    })
}

#[cfg(desktop)]
fn tray_items(_app: tauri::AppHandle, labels: TrayLabels) -> BoxFuture<'static, Vec<TrayItem>> {
    use veydan_shell::tray_label;
    Box::pin(async move {
        vec![
            TrayItem::entry(
                TrayGroup::Sections,
                "nav:/notes",
                tray_label(&labels, "section_notes", "Notes"),
            ),
            TrayItem::entry(
                TrayGroup::Actions,
                "quick_capture",
                tray_label(&labels, "quick_capture", "Quick note"),
            ),
        ]
    })
}

#[cfg(desktop)]
fn tray_click(app: &tauri::AppHandle, id: &str) {
    use tauri::Emitter;
    if id == "quick_capture" {
        quick_capture::show_quick_capture(app);
    } else if let Some(route) = id.strip_prefix("nav:") {
        // A page of the app: the main window shows it.
        app.state::<veydan_shell::Shell>().show_main_window();
        let _ = app.emit("tray://navigate", route.to_string());
    }
}

/// A data file with the tables of notes and of what they lean on, and the
/// states notes take from the app — in a mock app, or as the states the
/// hooks and handlers of sync reach without one.
#[cfg(test)]
pub(crate) mod testing {
    use tauri::test::MockRuntime;
    use tauri::Manager;
    use veydan_core::{db, Core, Directory, Schema};
    use veydan_sync_host::{Plan, Registry, Step, SyncManager};

    pub(crate) const LOCK: Schema = Schema {
        module: "lock",
        steps: veydan_lock::SCHEMA_STEPS,
    };

    /// The steps of notes in the cycle of Space, and the system rows.
    pub(crate) const PLAN: Plan = Plan {
        collect: &[
            Step::rows(
                "note rows",
                None,
                "notes",
                &["note_tag", "note_folder", "note_smart_view", "note_meta"],
            ),
            Step::handler("attachments", None, "attachments"),
            Step::handler("notes", None, "notes"),
            Step::rows(
                "system rows",
                None,
                "app",
                &["password_vault", "setting", "label"],
            ),
        ],
        apply: &[
            Step::handler("attachments", None, "attachments"),
            Step::rows(
                "notes catalog",
                None,
                "notes",
                &["note_tag", "note_folder", "note_smart_view"],
            ),
            Step::handler_then("notes", None, "notes", "notes index"),
            Step::rows("notes meta", None, "notes", &["note_meta"]),
            Step::rows(
                "system rows",
                None,
                "app",
                &["password_vault", "setting", "label"],
            ),
        ],
        late: &[],
    };

    async fn data_file(dir: &std::path::Path) -> sqlx::Pool<sqlx::Sqlite> {
        let schemas = [
            veydan_core::SCHEMA,
            LOCK,
            veydan_sync_host::SCHEMA,
            super::SCHEMA,
        ];
        let pool = db::open(&dir.join(db::DB_FILE), &schemas).await.unwrap();
        super::ensure_dirs(dir).unwrap();
        pool
    }

    /// The directory of the core: notes own their kind, so what they publish
    /// lands in the labels.
    fn directory() -> Directory {
        let mut directory = Directory::default();
        super::provide(&mut directory);
        directory
    }

    pub(crate) async fn app() -> (tauri::App<MockRuntime>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let pool = data_file(dir.path()).await;
        let app = tauri::test::mock_app();
        app.manage(Core::new(pool, dir.path().to_owned(), directory()));
        app.manage(super::NotesState::new(None));
        (app, dir)
    }

    /// The states of a product of notes alone, without the app.
    pub(crate) struct States {
        pub core: Core,
        pub lock: veydan_lock::Lock,
        pub notes: super::NotesState,
        pub sync: SyncManager,
        _dir: tempfile::TempDir,
    }

    impl States {
        pub(crate) async fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let pool = data_file(dir.path()).await;
            let mut registry = Registry::new();
            registry.add("notes", super::sync::register);
            Self {
                lock: veydan_lock::Lock::new(pool.clone()),
                core: Core::new(pool, dir.path().to_owned(), directory()),
                notes: super::NotesState::new(None),
                sync: SyncManager::new(registry.finish(&PLAN).unwrap()),
                _dir: dir,
            }
        }

        /// Where the hooks and handlers of sync find these states.
        pub(crate) fn host(&self) -> veydan_sync_host::Host<'_> {
            veydan_sync_host::Host::States(self)
        }
    }

    impl veydan_sync_host::States for States {
        fn state(&self, ty: std::any::TypeId) -> Option<&(dyn std::any::Any + Send + Sync)> {
            let states: [&(dyn std::any::Any + Send + Sync); 4] =
                [&self.core, &self.lock, &self.notes, &self.sync];
            states
                .into_iter()
                .find(|state| std::any::Any::type_id(*state) == ty)
        }
    }
}
