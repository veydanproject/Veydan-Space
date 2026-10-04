// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The parts of the demo data of the modules this crate still holds —
//! browser, ssh and capture — for video recordings. Pass and notes bring
//! their own parts; the driver that clears and seeds them is the shell's
//! (`veydan_shell::demo`).

mod bulk;
mod clear;
mod content;
mod seed;

#[cfg(desktop)]
use crate::browser::BrowserState;
use tauri::{Manager, Runtime};
use veydan_core::Core;
use veydan_shell::DemoPart;

/// Workspaces, proxies and profiles with their folders.
pub(crate) fn browser<R: Runtime>() -> DemoPart<R> {
    DemoPart {
        seed: |app, locale| {
            Box::pin(async move { seed::seed_browser(&app.state::<Core>(), &locale).await })
        },
        clear: |app| {
            Box::pin(async move {
                #[cfg(desktop)]
                clear::stop_browsers(&app.state::<BrowserState>()).await;
                clear::clear_browser(&app.state::<Core>()).await
            })
        },
    }
}

/// SSH keys and connections.
#[cfg(desktop)]
pub(crate) fn ssh<R: Runtime>() -> DemoPart<R> {
    DemoPart {
        seed: |app, locale| {
            Box::pin(async move { seed::seed_ssh(&app.state::<Core>(), &locale).await })
        },
        clear: |app| Box::pin(async move { clear::clear_ssh(&app.state::<Core>()).await }),
    }
}

/// The rules of the web clipper.
#[cfg(desktop)]
pub(crate) fn capture<R: Runtime>() -> DemoPart<R> {
    DemoPart {
        seed: |app, locale| {
            Box::pin(async move { seed::seed_capture(&app.state::<Core>(), &locale).await })
        },
        clear: |app| Box::pin(async move { clear::clear_capture(&app.state::<Core>()).await }),
    }
}

#[cfg(all(test, desktop))]
mod tests {
    use crate::modules::TestStates;
    use std::collections::HashMap;
    use tauri::test::MockRuntime;
    use tauri::Manager;
    use veydan_core::Core;
    use veydan_lock::Lock;

    /// What the demo data of platform-stage-4 left in a fresh data file, the
    /// same in both locales: the rows of each table, the files of notes and
    /// profiles. Captured with the code of that stage.
    const STAGE_4_COUNTS: [(&str, i64); 26] = [
        ("app_settings", 1),
        ("lock_secrets", 0),
        ("note_folder_links", 180),
        ("note_folders", 15),
        ("note_history", 0),
        ("note_links", 5),
        ("note_mentions", 0),
        ("note_smart_views", 4),
        ("note_tag_links", 334),
        ("note_tags", 11),
        ("notes", 180),
        ("notes_fts", 178),
        ("password_history", 0),
        ("password_vault", 1),
        ("passwords", 37),
        ("profiles", 70),
        ("proxies", 40),
        ("ssh_connection_profiles", 2),
        ("ssh_connection_workspaces", 25),
        ("ssh_connections", 25),
        ("ssh_keys", 6),
        ("totp_entries", 60),
        ("workspace_columns", 22),
        ("workspaces", 7),
        ("files: notes/documents", 180),
        ("files: profiles", 70),
    ];

    /// SHA-256 of the tables of pass as [`pass_tables`] writes them, after
    /// the demo data of platform-stage-4 in each locale.
    const STAGE_4_PASS: [(&str, &str); 2] = [
        (
            "en",
            "2f998702e7b1956877b793da38aa8e45af6718d631c434f72e92cfa49ee7cf1b",
        ),
        (
            "ru",
            "d2621bebcbc668e2c0266cfe22f6f62cfd4cea99d139d9d66738655d12b33f89",
        ),
    ];

    /// SHA-256 of the tables and files of notes as [`notes_tables`] writes
    /// them, after the demo data of platform-stage-5 in each locale.
    const STAGE_5_NOTES: [(&str, &str); 2] = [
        (
            "en",
            "735bf3e5c731d27e8c6158b0d619e850e73f57cd80d2ce476930e51bf08a0efa",
        ),
        (
            "ru",
            "cd545e80deadc26954b15161b7d4975f5ae7d36476857d8372f1a175a354242a",
        ),
    ];

    /// The rows of every table but those of sync, of the search index inside
    /// and the labels, which name what the parts wrote, and the files of
    /// notes and profiles.
    async fn counts(core: &Core) -> Vec<(String, i64)> {
        let db = &core.db;
        let tables: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type = 'table'
             AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'notes_fts_%'
             AND name NOT LIKE 'sync_%' AND name NOT IN ('schema_modules', 'labels')
             ORDER BY name",
        )
        .fetch_all(db)
        .await
        .unwrap();
        let mut counts = Vec::new();
        for table in tables {
            let n: i64 =
                sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM {table}")))
                    .fetch_one(db)
                    .await
                    .unwrap();
            counts.push((table, n));
        }
        let files =
            |dir: std::path::PathBuf| std::fs::read_dir(dir).map(|d| d.count()).unwrap_or(0) as i64;
        let data = &core.app_data_dir;
        counts.push((
            "files: notes/documents".into(),
            files(data.join("notes").join("documents")),
        ));
        counts.push(("files: profiles".into(), files(data.join("profiles"))));
        counts
    }

    /// The tables of pass in a form that does not depend on the run: the
    /// secrets decrypted with the key in use, the time of the seed left out,
    /// a note a password names by its title instead of its id.
    async fn pass_tables(core: &Core, lock: &Lock) -> String {
        let db = &core.db;
        let titles: HashMap<String, String> =
            sqlx::query_as::<_, (String, String)>("SELECT id, title FROM notes")
                .fetch_all(db)
                .await
                .unwrap()
                .into_iter()
                .collect();
        let named = |raw: &str| -> String {
            let list: Vec<String> = serde_json::from_str(raw).unwrap();
            let list: Vec<String> = list
                .into_iter()
                .map(|t| match t.strip_prefix("note:") {
                    Some(id) => format!(
                        "note:{}",
                        titles.get(id).cloned().unwrap_or_else(|| format!("? {id}"))
                    ),
                    None => t,
                })
                .collect();
            serde_json::to_string(&list).unwrap()
        };
        let (key, vault_id) = lock.require_open().unwrap();
        let mut text = String::new();
        type Password = (
            String,
            String,
            Option<String>,
            Option<String>,
            String,
            Option<String>,
            String,
            String,
            String,
        );
        let passwords: Vec<Password> = sqlx::query_as(
            "SELECT id, title, username, url, password_enc, note_enc, totp_ids, tags, vault_id
             FROM passwords ORDER BY id",
        )
        .fetch_all(db)
        .await
        .unwrap();
        for (id, title, username, url, password_enc, note_enc, totp_ids, tags, row_vault) in
            passwords
        {
            assert_eq!(row_vault, vault_id, "{id}");
            let password =
                veydan_lock::decrypt_field(&key, &id, "password", &password_enc).unwrap();
            let note = note_enc.map(|n| veydan_lock::decrypt_field(&key, &id, "note", &n).unwrap());
            text.push_str(&format!(
                "password {id} {title:?} {username:?} {url:?} {password:?} {note:?} {totp_ids} {}\n",
                named(&tags)
            ));
        }
        type Totp = (
            String,
            String,
            Option<String>,
            String,
            String,
            i64,
            i64,
            String,
            Option<String>,
        );
        let totp: Vec<Totp> = sqlx::query_as(
            "SELECT id, name, issuer, secret, algorithm, digits, period, tags, last_used_at
             FROM totp_entries ORDER BY id",
        )
        .fetch_all(db)
        .await
        .unwrap();
        for row in totp {
            text.push_str(&format!("totp {row:?}\n"));
        }
        let history: Vec<(String, String)> =
            sqlx::query_as("SELECT id, password FROM password_history ORDER BY id")
                .fetch_all(db)
                .await
                .unwrap();
        for row in history {
            text.push_str(&format!("pw_history {row:?}\n"));
        }
        text
    }

    /// The tables and files of notes in a form that does not depend on the
    /// run: the time of the seed and the data directory left out of rows and
    /// files, tags by name, smart views without their random ids.
    async fn notes_tables(core: &Core) -> String {
        let db = &core.db;
        let times: Vec<String> =
            sqlx::query_scalar("SELECT created_at FROM notes UNION SELECT updated_at FROM notes")
                .fetch_all(db)
                .await
                .unwrap();
        assert_eq!(times.len(), 1, "{times:?}");
        let now = &times[0];
        let mut text = String::new();
        type Note = (
            String,
            String,
            String,
            String,
            i64,
            i64,
            i64,
            String,
            String,
            String,
        );
        let notes: Vec<Note> = sqlx::query_as(
            "SELECT id, title, file_path, format, pinned, archived, deleted, doc_status, preview, bindings
             FROM notes ORDER BY id",
        )
        .fetch_all(db)
        .await
        .unwrap();
        for row in notes {
            text.push_str(&format!("note {row:?}\n"));
        }
        let rows = |sql: &'static str| async move {
            sqlx::query_as::<_, (String, String)>(sql)
                .fetch_all(db)
                .await
                .unwrap()
        };
        for (label, sql) in [
            ("tag", "SELECT name, color FROM note_tags ORDER BY name"),
            (
                "tag link",
                "SELECT l.note_id, t.name FROM note_tag_links l JOIN note_tags t ON t.id = l.tag_id
                 ORDER BY l.note_id, t.name",
            ),
            (
                "folder link",
                "SELECT note_id, folder_id FROM note_folder_links ORDER BY note_id, folder_id",
            ),
            (
                "link",
                "SELECT from_id, to_id FROM note_links ORDER BY from_id, to_id",
            ),
            (
                "mention",
                "SELECT note_id, binding FROM note_mentions ORDER BY note_id, binding",
            ),
        ] {
            for row in rows(sql).await {
                text.push_str(&format!("{label} {row:?}\n"));
            }
        }
        let folders: Vec<(String, String, Option<String>, String)> =
            sqlx::query_as("SELECT id, name, parent_id, color FROM note_folders ORDER BY id")
                .fetch_all(db)
                .await
                .unwrap();
        for row in folders {
            text.push_str(&format!("folder {row:?}\n"));
        }
        let views: Vec<(String, String, String, i64)> = sqlx::query_as(
            "SELECT name, color, conditions, sort_order FROM note_smart_views ORDER BY sort_order",
        )
        .fetch_all(db)
        .await
        .unwrap();
        for row in views {
            text.push_str(&format!("smart view {row:?}\n"));
        }
        let fts: Vec<(String, String, String, String)> =
            sqlx::query_as("SELECT note_id, title, content, tags FROM notes_fts ORDER BY note_id")
                .fetch_all(db)
                .await
                .unwrap();
        for row in fts {
            text.push_str(&format!("fts {row:?}\n"));
        }
        let notes_dir = core.app_data_dir.join("notes");
        for sub in ["documents", "attachments", "drafts"] {
            let mut names: Vec<_> = std::fs::read_dir(notes_dir.join(sub))
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            names.sort();
            for path in names {
                let name = path.strip_prefix(&notes_dir).unwrap().display().to_string();
                let body = std::fs::read_to_string(&path).unwrap_or_else(|_| "<dir>".into());
                text.push_str(&format!("file {name}\n{}\n", body.replace(now, "<now>")));
            }
        }
        text.replace(&core.app_data_dir.display().to_string(), "<data>")
    }

    fn sha256(text: &str) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(text.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// The states the parts take, in an app without a window. Every module
    /// of Space owns its kinds of the directory, as in the product: the
    /// demo's notes bind only to kinds an owner answers for.
    async fn app() -> tauri::App<MockRuntime> {
        let owners: Vec<&str> = crate::modules::directory_parts::<MockRuntime>()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        let TestStates {
            core,
            lock,
            notes,
            browser,
            sync: _,
        } = TestStates::owning(crate::modules::sync_registry(), &owners).await;
        let app = tauri::test::mock_app();
        app.manage(core);
        app.manage(lock);
        app.manage(notes);
        app.manage(browser);
        app
    }

    /// The driver of the shell over the parts of Space — pass's, notes' and
    /// those of this crate — leaves what the demo data of platform-stage-4 left:
    /// nothing seeded twice, nothing missed; and the notes are those of
    /// platform-stage-5, file for file. `DUMP_DEMO_NOTES=<dir>` writes the
    /// text the checksum of notes is taken over, to compare a failing run.
    #[tokio::test]
    async fn the_demo_data_of_the_parts_is_that_of_stage_4() {
        let app = app().await;
        let parts = crate::modules::demo_parts::<MockRuntime>();
        let expected: Vec<(String, i64)> = STAGE_4_COUNTS
            .iter()
            .map(|(table, n)| (table.to_string(), *n))
            .collect();
        for (locale, checksum) in STAGE_4_PASS {
            veydan_shell::demo::seed(app.handle(), &parts, locale)
                .await
                .unwrap();
            let core = app.state::<Core>();
            assert_eq!(counts(&core).await, expected, "{locale}");
            let pass = pass_tables(&core, &app.state::<Lock>()).await;
            assert!(
                !pass.contains("note:? "),
                "{locale}: a link to a note that is not there\n{pass}"
            );
            assert_eq!(sha256(&pass), checksum, "{locale}:\n{pass}");
            let notes = notes_tables(&core).await;
            if let Ok(dir) = std::env::var("DUMP_DEMO_NOTES") {
                std::fs::write(format!("{dir}/notes-{locale}.txt"), &notes).unwrap();
            }
            let notes_checksum = STAGE_5_NOTES.iter().find(|(l, _)| *l == locale).unwrap().1;
            assert_eq!(sha256(&notes), notes_checksum, "{locale}");
        }

        veydan_shell::demo::clear(app.handle(), &parts)
            .await
            .unwrap();
        let core = app.state::<Core>();
        let left: Vec<(String, i64)> = counts(&core)
            .await
            .into_iter()
            .filter(|(_, n)| *n > 0)
            .collect();
        // The key row stays; so does the Default workspace.
        assert_eq!(
            left,
            [
                ("password_vault".to_string(), 1),
                ("workspaces".to_string(), 1)
            ]
        );
        let workspaces: Vec<String> = sqlx::query_scalar("SELECT id FROM workspaces")
            .fetch_all(&core.db)
            .await
            .unwrap();
        assert_eq!(workspaces, vec!["default".to_string()]);

        core.db.close().await;
        let _ = std::fs::remove_dir_all(&core.app_data_dir);
    }
}
