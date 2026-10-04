// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The module Pass: password entries, TOTP and the history of the password
//! generator.
//!
//! The secrets of an entry are encrypted with the key of the lock
//! (`veydan_lock`); pass has no state of its own and takes `Core` and `Lock`
//! from the app. It knows no other module: a note it is linked to is a tag
//! `note:{id}` it drops when the note's owner reports the note deleted, and
//! what others need of its entries — names, public fields, the current code
//! of a TOTP — they ask the entity directory for.

mod demo;
mod directory;
mod password;
mod passwords;
mod totp;

use tauri::Runtime;
use veydan_core::{Core, Directory, Schema};
use veydan_shell::{DemoPart, Module, SetupResult};
#[cfg(desktop)]
use veydan_shell::{TrayGroup, TrayItem, TrayLabels, TrayPart};
use veydan_sync_host::{Hooks, Registry, TableSpec};

/// The entity kinds pass owns in the directory.
pub const KIND_PASSWORD: &str = "password";
pub const KIND_TOTP: &str = "totp";

/// The tables of pass. A data file made while these tables belonged to the
/// product crate holds them under the same module name and version: the
/// steps are those, unchanged, and nothing is applied again.
pub const SCHEMA: Schema = Schema {
    module: "pass",
    steps: &[concat!(
        "CREATE TABLE passwords (
            id           TEXT PRIMARY KEY NOT NULL,
            title        TEXT NOT NULL,
            username     TEXT,
            url          TEXT,
            password_enc TEXT NOT NULL,
            note_enc     TEXT,
            totp_ids     TEXT NOT NULL DEFAULT '[]',
            tags         TEXT NOT NULL DEFAULT '[]',
            vault_id     TEXT NOT NULL,
            created_at   TEXT NOT NULL,
            updated_at   TEXT NOT NULL
        );",
        "CREATE TABLE totp_entries (
            id          TEXT PRIMARY KEY NOT NULL,
            name        TEXT NOT NULL,
            issuer      TEXT,
            secret      TEXT NOT NULL,
            algorithm   TEXT NOT NULL DEFAULT 'SHA1',
            digits      INTEGER NOT NULL DEFAULT 6,
            period      INTEGER NOT NULL DEFAULT 30,
            tags        TEXT NOT NULL DEFAULT '[]',
            created_at  TEXT NOT NULL,
            updated_at  TEXT NOT NULL,
            last_used_at TEXT
        );",
        // Passwords the generator produced
        "CREATE TABLE password_history (
            id TEXT PRIMARY KEY NOT NULL,
            password TEXT NOT NULL,
            created_at TEXT NOT NULL
        );",
    )],
};

pub fn module() -> Module {
    Module {
        schema: Some(SCHEMA),
        directory: Some(provide),
        sync: Some(sync),
        key_user: Some(passwords::KEY_USER),
        demo: Some(demo()),
        #[cfg(desktop)]
        tray: Some(TrayPart {
            items: tray_items,
            on_click: tray_click,
            tooltip: None,
        }),
        ..veydan_shell::module! {
            id: "pass",
            setup: setup,
            commands: [
                password::pwgen_history_list,
                password::pwgen_history_add,
                password::pwgen_history_clear,
                password::pwgen_history_trim,
                passwords::password_list,
                passwords::password_get,
                passwords::password_create,
                passwords::password_update,
                passwords::password_delete,
                passwords::password_reveal,
                #[cfg(desktop)]
                passwords::password_copy,
                passwords::password_vault_reset,
                totp::totp_list,
                totp::totp_add,
                totp::totp_update,
                totp::totp_delete,
                totp::totp_generate_code,
                totp::totp_generate_codes,
                totp::totp_preview_uri,
            ],
        }
    }
}

/// The part of pass in the demo data, on any runtime; `module()` holds it
/// on the app's.
pub fn demo<R: Runtime>() -> DemoPart<R> {
    DemoPart {
        seed: demo::seed,
        clear: demo::clear,
    }
}

/// The kinds of pass in the entity directory, on any runtime; `module()`
/// holds it on the app's.
pub fn provide<R: Runtime>(directory: &mut Directory<R>) {
    directory.provide(KIND_PASSWORD, directory::passwords());
    directory.provide(KIND_TOTP, directory::totp());
}

/// TOTP entries and passwords; a tombstone of either is reported on the
/// deletion hooks, and what sync writes or removes is named in the labels.
/// The history of the generator stays on a phone as 4.0.7 left it there:
/// neither pulled nor pushed.
fn sync(registry: &mut Registry) {
    registry.table(
        TableSpec::plain(
            "totp",
            "totp_entries",
            &[
                "name",
                "issuer",
                "secret",
                "algorithm",
                "digits",
                "period",
                "tags",
                "created_at",
                "updated_at",
            ],
        ),
        Hooks {
            on_delete: Some(totp::delete_synced),
            after_row: Some(directory::totp_synced),
            ..Hooks::default()
        },
    );
    registry.table(
        TableSpec::plain(
            "password",
            "passwords",
            &[
                "title",
                "username",
                "url",
                "password_enc",
                "note_enc",
                "totp_ids",
                "tags",
                "vault_id",
                "created_at",
                "updated_at",
            ],
        ),
        Hooks {
            on_delete: Some(passwords::delete_synced),
            after_row: Some(directory::password_synced),
            ..Hooks::default()
        },
    );
    #[cfg(desktop)]
    registry.table(
        TableSpec::plain(
            "pw_history",
            "password_history",
            &["password", "created_at"],
        ),
        Hooks::default(),
    );
}

/// The kind of the entity directory a note is.
const KIND_NOTE: &str = "note";

fn setup(app: &mut tauri::App) -> SetupResult {
    use tauri::Manager;
    // An entry linked to a note drops the link when the note goes.
    app.state::<Core>()
        .deletions
        .subscribe(KIND_NOTE, passwords::forget_note);
    // The camera reads the QR code of a TOTP secret.
    #[cfg(mobile)]
    app.handle().plugin(tauri_plugin_barcode_scanner::init())?;
    Ok(())
}

#[cfg(desktop)]
fn tray_items(
    _app: tauri::AppHandle,
    labels: TrayLabels,
) -> veydan_core::BoxFuture<'static, Vec<TrayItem>> {
    Box::pin(async move {
        vec![TrayItem::entry(
            TrayGroup::Actions,
            "pwgen",
            veydan_shell::tray_label(&labels, "password_generator", "Password generator"),
        )]
    })
}

#[cfg(desktop)]
fn tray_click(app: &tauri::AppHandle, id: &str) {
    use tauri::{Emitter, Manager};
    if id == "pwgen" {
        app.state::<veydan_shell::Shell>().show_main_window();
        let _ = app.emit("tray://open-pwgen", ());
    }
}

/// A data file with the tables of pass and of what they lean on, and the
/// states pass takes from the app, in a mock app.
#[cfg(test)]
pub(crate) mod testing {
    use tauri::test::MockRuntime;
    use tauri::Manager;
    use veydan_core::{db, Core, Directory, Schema};
    use veydan_lock::Lock;

    pub(crate) const LOCK: Schema = Schema {
        module: "lock",
        steps: veydan_lock::SCHEMA_STEPS,
    };

    /// The core knows pass as the owner of its kinds: what pass publishes
    /// lands in the labels.
    pub(crate) async fn app() -> (tauri::App<MockRuntime>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let schemas = [
            veydan_core::SCHEMA,
            LOCK,
            veydan_sync_host::SCHEMA,
            super::SCHEMA,
        ];
        let pool = db::open(&dir.path().join(db::DB_FILE), &schemas)
            .await
            .unwrap();
        let app = tauri::test::mock_app();
        let lock = Lock::with_key_users(pool.clone(), vec![super::passwords::KEY_USER]);
        lock.open_default().await.unwrap();
        app.manage(lock);
        let mut directory = Directory::default();
        super::provide(&mut directory);
        app.manage(Core::new(pool, dir.path().to_owned(), directory));
        (app, dir)
    }
}
