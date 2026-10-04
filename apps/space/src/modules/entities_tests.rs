// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What notes show of the entities of other modules — the names and
//! subtitles of bindings and of the picker, the values of the placeholders
//! of a template, the workspaces and profiles of the navigation, the
//! workspace of a captured page's profile — comes from the entity directory
//! now. Over the demo data of both locales and rows that test the edges,
//! each is what the queries of platform-stage-5 into the tables of pass,
//! browser and ssh gave.

use super::TestStates;
use veydan_notes::entities;
use sqlx::{Pool, Sqlite};
use std::collections::HashMap;
use tauri::test::MockRuntime;
use tauri::Manager;
use veydan_core::{Core, Directory};

/// The binding kinds of entities, with the query platform-stage-5 named
/// them by: `id, name, subtitle`, public fields only.
const STAGE_5_SUMMARIES: [(&str, &str); 6] = [
    ("workspace", "SELECT id, name, '' AS subtitle FROM workspaces"),
    ("profile", "SELECT id, name, browser_type AS subtitle FROM profiles"),
    (
        "proxy",
        "SELECT id, name, upper(proxy_type) || CASE WHEN coalesce(country, '') = '' THEN '' ELSE ' · ' || country END AS subtitle FROM proxies",
    ),
    (
        "ssh",
        "SELECT id, name, username || '@' || host AS subtitle FROM ssh_connections",
    ),
    (
        "totp",
        "SELECT id, name, coalesce(issuer, '') AS subtitle FROM totp_entries",
    ),
    (
        "password",
        "SELECT id, title AS name, coalesce(username, '') AS subtitle FROM passwords",
    ),
];

type Summary = (String, String, String, String);

fn stage_5_sql(kind: &str) -> &'static str {
    STAGE_5_SUMMARIES
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, sql)| *sql)
        .unwrap()
}

/// `note_binding_summaries` of platform-stage-5 for one binding.
async fn stage_5_summary(db: &Pool<Sqlite>, binding: &str) -> Option<Summary> {
    let (kind, id) = binding.split_once(':')?;
    let sql = STAGE_5_SUMMARIES.iter().find(|(k, _)| *k == kind)?.1;
    let row: Option<(String, String, String)> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!("{sql} WHERE id = ?")))
            .bind(id)
            .fetch_optional(db)
            .await
            .unwrap();
    row.map(|(id, name, subtitle)| (format!("{kind}:{id}"), kind.to_string(), name, subtitle))
}

/// `note_entity_search` of platform-stage-5.
async fn stage_5_search(db: &Pool<Sqlite>, kind: &str, query: &str) -> Vec<Summary> {
    let pattern = format!("%{}%", query.trim().to_lowercase());
    let rows: Vec<(String, String, String)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT * FROM ({}) AS entity WHERE lower(name) LIKE ? OR lower(subtitle) LIKE ? ORDER BY name LIMIT 30",
        stage_5_sql(kind)
    )))
    .bind(&pattern)
    .bind(&pattern)
    .fetch_all(db)
    .await
    .unwrap();
    rows.into_iter()
        .map(|(id, name, subtitle)| (format!("{kind}:{id}"), kind.to_string(), name, subtitle))
        .collect()
}

/// The placeholders of entities `TemplateVars::from_bindings` filled at
/// platform-stage-5.
async fn stage_5_placeholders(db: &Pool<Sqlite>, bindings: &[String]) -> HashMap<String, String> {
    let mut vars: HashMap<String, String> = [
        "profile",
        "workspace",
        "proxy",
        "proxy_type",
        "proxy_host",
        "proxy_port",
        "proxy_region",
        "ssh",
        "ssh_host",
        "ssh_port",
        "ssh_user",
        "totp",
        "password",
    ]
    .into_iter()
    .map(|key| (key.to_string(), String::new()))
    .collect();
    let name = |table: &'static str, column: &'static str, id: String| async move {
        sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(format!(
            "SELECT {column} FROM {table} WHERE id = ?"
        )))
        .bind(id)
        .fetch_optional(db)
        .await
        .unwrap()
        .unwrap_or_default()
    };
    for binding in bindings {
        let Some((kind, id)) = binding.split_once(':') else {
            continue;
        };
        let id = id.to_string();
        match kind {
            "profile" => *vars.get_mut("profile").unwrap() = name("profiles", "name", id).await,
            "workspace" => {
                *vars.get_mut("workspace").unwrap() = name("workspaces", "name", id).await
            }
            "totp" => *vars.get_mut("totp").unwrap() = name("totp_entries", "name", id).await,
            "password" => *vars.get_mut("password").unwrap() = name("passwords", "title", id).await,
            "proxy" => {
                type Proxy = (String, String, String, i64, Option<String>, Option<String>);
                let row: Option<Proxy> = sqlx::query_as(
                    "SELECT name, proxy_type, host, port, country, city FROM proxies WHERE id = ?",
                )
                .bind(&id)
                .fetch_optional(db)
                .await
                .unwrap();
                if let Some((name, proxy_type, host, port, country, city)) = row {
                    vars.insert("proxy".into(), name);
                    vars.insert("proxy_type".into(), proxy_type);
                    vars.insert("proxy_host".into(), host);
                    vars.insert("proxy_port".into(), port.to_string());
                    let region = [country, city]
                        .into_iter()
                        .flatten()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(", ");
                    vars.insert("proxy_region".into(), region);
                }
            }
            "ssh" => {
                let row: Option<(String, String, i64, String)> = sqlx::query_as(
                    "SELECT name, host, port, username FROM ssh_connections WHERE id = ?",
                )
                .bind(&id)
                .fetch_optional(db)
                .await
                .unwrap();
                if let Some((name, host, port, username)) = row {
                    vars.insert("ssh".into(), name);
                    vars.insert("ssh_host".into(), host);
                    vars.insert("ssh_port".into(), port.to_string());
                    vars.insert("ssh_user".into(), username);
                }
            }
            _ => {}
        }
    }
    vars
}

/// The rows the demo data does not have: no user name, no issuer, an empty
/// and a missing country, a profile in no workspace, two connections of
/// one name, names with the wildcards of `LIKE` and with letters outside
/// ASCII in both cases.
async fn edges(db: &Pool<Sqlite>) {
    for sql in [
        "INSERT INTO passwords (id, title, username, url, password_enc, vault_id, created_at, updated_at) VALUES
         ('edge-pw', 'Zeta 100%_off', NULL, NULL, 'x', 'v', 't', 't')",
        "INSERT INTO totp_entries (id, name, issuer, secret, created_at, updated_at) VALUES
         ('edge-totp', 'Ünïcode ВХОД', NULL, 'JBSWY3DPEHPK3PXP', 't', 't')",
        "INSERT INTO proxies (id, name, proxy_type, host, port, country, city, created_at) VALUES
         ('edge-px-null', 'px null', 'http', 'null.example', 3128, NULL, NULL, 't'),
         ('edge-px-empty', 'Px Empty', 'socks5', 'empty.example', 1080, '', 'Berlin', 't')",
        "INSERT INTO profiles (id, name, profile_path, workspace_id, created_at, updated_at) VALUES
         ('edge-pr', 'Ничей профиль', '/nowhere', NULL, 't', 't')",
        "INSERT INTO ssh_connections (id, name, host, port, username, created_at, updated_at) VALUES
         ('edge-ssh-a', 'Twin', 'a.example', 22, 'root', 't', 't'),
         ('edge-ssh-b', 'Twin', 'b.example', 2222, 'deploy', 't', 't')",
    ] {
        sqlx::query(sql).execute(db).await.unwrap();
    }
}

/// The demo data of `locale` and the edge rows, and a directory with the
/// kinds of Space over them.
async fn seeded(locale: &str) -> (tauri::App<MockRuntime>, Directory<MockRuntime>, Vec<String>) {
    let TestStates {
        core,
        lock,
        notes,
        browser,
        sync: _,
    } = TestStates::new().await;
    let app = tauri::test::mock_app();
    app.manage(core);
    app.manage(lock);
    app.manage(notes);
    app.manage(browser);
    veydan_shell::demo::seed(app.handle(), &super::demo_parts::<MockRuntime>(), locale)
        .await
        .unwrap();
    let db = app.state::<Core>().db.clone();
    edges(&db).await;

    let mut directory = Directory::default();
    for (_, provide) in super::directory_parts::<MockRuntime>() {
        provide(&mut directory);
    }
    directory.attach(app.handle().clone());

    let mut bindings = Vec::new();
    for (kind, sql) in STAGE_5_SUMMARIES {
        let ids: Vec<(String,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT id FROM ({sql}) ORDER BY id"
        )))
        .fetch_all(&db)
        .await
        .unwrap();
        assert!(!ids.is_empty(), "{locale}: no {kind}");
        bindings.extend(ids.into_iter().map(|(id,)| format!("{kind}:{id}")));
        bindings.push(format!("{kind}:no-such-{kind}"));
        bindings.push(format!("{kind}:"));
    }
    bindings.extend([
        "url:https://example.com/a".into(),
        "domain:example.com".into(),
    ]);
    (app, directory, bindings)
}

async fn close(app: tauri::App<MockRuntime>) {
    let core = app.state::<Core>();
    core.db.close().await;
    let _ = std::fs::remove_dir_all(&core.app_data_dir);
}

fn tuples(summaries: Vec<veydan_notes::BindingSummary>) -> Vec<Summary> {
    summaries
        .into_iter()
        .map(|s| (s.binding, s.kind, s.name, s.subtitle))
        .collect()
}

#[tokio::test]
async fn the_names_and_subtitles_of_bindings_are_those_of_stage_5() {
    for locale in ["en", "ru"] {
        let (app, directory, bindings) = seeded(locale).await;
        let db = app.state::<Core>().db.clone();
        let mut expected = Vec::new();
        for binding in &bindings {
            let ours = tuples(entities::summaries(&directory, std::slice::from_ref(binding)).await);
            let theirs: Vec<Summary> = stage_5_summary(&db, binding).await.into_iter().collect();
            assert_eq!(ours, theirs, "{locale}: {binding}");
            expected.extend(theirs);
        }
        // All at once, in the order asked.
        assert_eq!(
            tuples(entities::summaries(&directory, &bindings).await),
            expected
        );
        assert!(expected.len() > 200, "{locale}: {}", expected.len());
        close(app).await;
    }
}

#[tokio::test]
async fn a_search_of_the_picker_finds_what_stage_5_found() {
    let queries = [
        "",
        " ",
        "a",
        "A",
        "e",
        "o",
        "pro",
        "PROD",
        "@",
        "@1",
        "root@",
        "1",
        ".",
        "-",
        "%",
        "_",
        "0%",
        "%_",
        "socks",
        "SOCKS5 · ",
        "us",
        "de",
        "twin",
        "zeta",
        "ü",
        "Ü",
        "вход",
        "ВХОД",
        "ра",
        "Ра",
        "dev",
        " dev ",
        "github",
        "no-such-thing",
    ];
    for locale in ["en", "ru"] {
        let (app, directory, _) = seeded(locale).await;
        let db = app.state::<Core>().db.clone();
        let mut found = 0;
        for (kind, _) in STAGE_5_SUMMARIES {
            for query in queries {
                let ours = tuples(entities::search(&directory, kind, query).await);
                let theirs = stage_5_search(&db, kind, query).await;
                assert_eq!(ours, theirs, "{locale}: {kind} {query:?}");
                found += ours.len();
            }
        }
        assert!(found > 500, "{locale}: {found}");
        for kind in ["url", "domain", "note", "nothing"] {
            assert!(
                entities::search(&directory, kind, "").await.is_empty(),
                "{kind}"
            );
        }
        close(app).await;
    }
}

#[tokio::test]
async fn the_placeholders_of_a_template_are_those_of_stage_5() {
    for locale in ["en", "ru"] {
        let (app, directory, bindings) = seeded(locale).await;
        let db = app.state::<Core>().db.clone();
        let ours = |bindings: Vec<String>| {
            let directory = &directory;
            async move {
                let mut values =
                    veydan_notes::placeholder_values(directory, "Title", &bindings).await;
                assert_eq!(values.remove("title").as_deref(), Some("Title"));
                for key in ["date", "time", "datetime", "url", "domain"] {
                    values.remove(key);
                }
                values
            }
        };
        for binding in &bindings {
            let one = vec![binding.clone()];
            assert_eq!(
                ours(one.clone()).await,
                stage_5_placeholders(&db, &one).await,
                "{locale}: {binding}"
            );
        }
        // One binding of each kind, the last of a kind winning.
        assert_eq!(
            ours(bindings.clone()).await,
            stage_5_placeholders(&db, &bindings).await,
            "{locale}"
        );
        // The first entity of each kind together; none of their values is empty.
        let mut first = Vec::new();
        for (kind, _) in STAGE_5_SUMMARIES {
            first.extend(
                bindings
                    .iter()
                    .find(|b| b.starts_with(&format!("{kind}:")))
                    .cloned(),
            );
        }
        let together = ours(first.clone()).await;
        assert_eq!(
            together,
            stage_5_placeholders(&db, &first).await,
            "{locale}"
        );
        for key in [
            "profile",
            "workspace",
            "proxy",
            "proxy_port",
            "ssh",
            "ssh_port",
            "ssh_user",
            "totp",
            "password",
        ] {
            assert!(!together[key].is_empty(), "{locale}: {key} {together:?}");
        }
        close(app).await;
    }
}

#[tokio::test]
async fn the_navigation_and_the_capture_see_the_workspaces_and_profiles_of_stage_5() {
    for locale in ["en", "ru"] {
        let (app, directory, _) = seeded(locale).await;
        let db = app.state::<Core>().db.clone();
        let workspaces: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT id, name, color FROM workspaces ORDER BY is_default DESC, created_at ASC",
        )
        .fetch_all(&db)
        .await
        .unwrap();
        let profiles: Vec<(String, String, Option<String>)> =
            sqlx::query_as("SELECT id, name, workspace_id FROM profiles ORDER BY name")
                .fetch_all(&db)
                .await
                .unwrap();
        assert!(workspaces.len() > 5 && profiles.len() > 50, "{locale}");
        assert!(profiles.iter().any(|(_, _, w)| w.is_none()), "{locale}");
        assert_eq!(
            entities::workspaces(&directory).await,
            workspaces,
            "{locale}"
        );
        assert_eq!(entities::profiles(&directory).await, profiles, "{locale}");

        for (id, _, workspace) in &profiles {
            assert_eq!(
                &entities::workspace_of_profile(&directory, id).await,
                workspace,
                "{locale}: {id}"
            );
        }
        assert_eq!(
            entities::workspace_of_profile(&directory, "no-such").await,
            None
        );
        close(app).await;
    }
}

/// The lists of the tree as the phone's screen shows them: each by name.
fn as_shown(tree: &veydan_notes::NoteNav) -> serde_json::Value {
    fn by_name(list: &mut serde_json::Value) {
        let items = list.as_array_mut().unwrap();
        for item in items.iter_mut() {
            if item.get("profiles").is_some() {
                by_name(&mut item["profiles"]);
            }
        }
        items.sort_by_key(|item| (item["name"].to_string(), item["id"].to_string()));
    }
    let mut tree = serde_json::to_value(tree).unwrap();
    for list in ["workspaces", "all_workspaces", "all_profiles"] {
        by_name(&mut tree[list]);
    }
    tree
}

/// Spec 9.1, 10.2: a phone has no browser, and its navigation of notes
/// builds the workspaces, with their colors, and the profiles under them
/// from the labels Space published. Over the demo data and the edge rows the
/// tree is the one the owner's rows gave — the rows a phone mirrored until
/// stage 6 — as the screen shows it, sorted by name; the capture finds the
/// same workspace of each profile.
#[tokio::test]
async fn a_phone_builds_the_navigation_of_notes_from_the_labels() {
    for locale in ["en", "ru"] {
        let (app, mut space, _) = seeded(locale).await;
        let db = app.state::<Core>().db.clone();
        // Notes bound to a workspace and to profiles in and out of one.
        sqlx::query(
            "UPDATE notes SET bindings = '[\"workspace:default\"]'
             WHERE id = (SELECT id FROM notes ORDER BY id LIMIT 1)",
        )
        .execute(&db)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE notes SET bindings = '[\"profile:edge-pr\"]'
             WHERE id = (SELECT id FROM notes ORDER BY id LIMIT 1 OFFSET 1)",
        )
        .execute(&db)
        .await
        .unwrap();
        space.keep_labels(db.clone());
        assert!(space.publish_all().await.unwrap() > 100, "{locale}");

        let mut phone = Directory::default();
        for (_, provide) in super::directory_parts::<MockRuntime>()
            .into_iter()
            .filter(|(id, _)| !matches!(*id, "browser" | "ssh"))
        {
            provide(&mut phone);
        }
        phone.attach(app.handle().clone());
        phone.keep_labels(db.clone());
        assert!(!phone.kinds().contains(&"workspace"));

        let owner = veydan_notes::nav(&db, &space).await.unwrap();
        let labels = veydan_notes::nav(&db, &phone).await.unwrap();
        assert!(
            owner.workspaces.iter().any(|ws| !ws.profiles.is_empty()),
            "{locale}: no profile in the tree"
        );
        assert!(owner.all_profiles.len() > 50, "{locale}");
        assert_eq!(as_shown(&labels), as_shown(&owner), "{locale}");

        for profile in &owner.all_profiles {
            assert_eq!(
                entities::workspace_of_profile(&phone, &profile.id).await,
                entities::workspace_of_profile(&space, &profile.id).await,
                "{locale}: {}",
                profile.id
            );
        }
        close(app).await;
    }
}
