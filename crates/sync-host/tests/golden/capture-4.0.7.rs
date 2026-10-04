// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! How the files of this directory were made: by the sync code of 4.0.7
//! (`main`, 34a56fb), not by this branch. Not compiled here. To run it again:
//!
//! 1. `git archive main | tar -x -C <dir>`; `mkdir <dir>/build` with an
//!    `index.html` in it (the context of the app embeds that folder);
//! 2. copy this file to `<dir>/src-tauri/src/sync/capture_407.rs`, add
//!    `#[cfg(test)] mod capture_407;` to `src/sync/mod.rs` and make
//!    `read_rows` of `src/sync/rows.rs` `pub(super)` — nothing else of
//!    4.0.7 changes;
//! 3. inside `build-env.sh`, from `<dir>/src-tauri`, one test per process
//!    (one app each), in this order:
//!    `CAPTURE_OUT=<out> cargo test --no-default-features --lib sync::capture_407::<test> -- --exact`
//!    for `collect`, `write_vault`, `apply_vault`;
//! 4. `<out>/golden/*.json`, `<out>/vault/`, `vault.json`, `apply.json` and
//!    `collect.json` are the files here.
//!
//! The app is a real one without a window (`Builder::any_thread`), so the
//! cycle of 4.0.7 runs as it does in the app: `cycle_inner` pushes, pulls,
//! applies and collects garbage. Its clock is the wall clock, so a new run
//! writes other HLCs, ids of devices and vaults, and other ciphertexts.

use super::config::{self, load_config};
use super::state;
use super::{attachments, notes, profile_files, rows};
use crate::AppState;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::Manager;
use veydan_sync::{Engine, Hlc, HlcClock, LocalDir, LocalState, Op, Vmk};

const T1: &str = "2026-09-01T10:00:00+00:00";
const T2: &str = "2026-09-02T11:30:00+00:00";
const DEVICE: &str = "golden-device";
const VAULT_KEY: &str = "golden-vault-key";
const PASSPHRASE: &str = "correct horse battery";
const WRITER: &str = "0a407a40-7a40-4a40-8a40-7a407a407a40";
const READER: &str = "0b407b40-7b40-4b40-8b40-7b407b407b40";

fn out_dir() -> PathBuf {
    PathBuf::from(std::env::var("CAPTURE_OUT").expect("CAPTURE_OUT"))
}

/// A tauri App with no window, on the thread of the test.
fn app() -> tauri::App {
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    tauri::Builder::default()
        .any_thread()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .build(context)
        .expect("app")
}

async fn state_in(data_dir: &Path) -> AppState {
    std::fs::create_dir_all(data_dir.join("profiles")).unwrap();
    crate::ensure_notes_dirs(data_dir).unwrap();
    let db = crate::db::init_pool(&data_dir.join("profiles.db"))
        .await
        .expect("db");
    AppState {
        db,
        app_data_dir: data_dir.to_owned(),
        notes_custom_dir: Arc::new(std::sync::RwLock::new(None)),
        notes_watcher: Arc::new(Mutex::new(None)),
        notes_lock: crate::commands::notes::NotesLock::default(),
        notes_save: Arc::new(tokio::sync::Mutex::new(())),
        vault: crate::vault::VaultState::default(),
        sync: Arc::new(super::SyncManager::default()),
        browser: Arc::new(crate::browser::launch::BrowserState::default()),
        download: crate::commands::camoufox::DownloadManager::default(),
        ssh_sessions: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
        sftp_sessions: Arc::new(crate::commands::sftp::SftpState::default()),
        backup: Arc::new(crate::commands::backup::BackupManager::default()),
        tray_settings: Arc::new(crate::TraySettings::default()),
        tray_labels: Arc::new(Mutex::new(crate::tray::TrayLabels::default())),
        tray: Arc::new(Mutex::new(None)),
    }
}

async fn sql(state: &AppState, statement: &str) {
    sqlx::query(sqlx::AssertSqlSafe(statement.to_string()))
        .execute(&state.db)
        .await
        .unwrap_or_else(|e| panic!("{statement}: {e}"));
}

fn documents(state: &AppState) -> PathBuf {
    state.app_data_dir.join("notes").join("documents")
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn note_file(id: &str, title: &str, body: &str) -> Vec<u8> {
    format!(
        "---\nid: {id}\ntitle: \"{title}\"\nformat: md\nbindings: []\ntags: []\ncreated_at: {T1}\nupdated_at: {T2}\n---\n{body}\n"
    )
    .into_bytes()
}

fn large_bytes(len: usize) -> Vec<u8> {
    let mut x: u32 = 0x9e37_79b9;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

/// A file of `len` bytes zstd makes small: lines of text.
fn text_bytes(len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let mut i = 0u32;
    while out.len() < len {
        out.extend_from_slice(format!("line {i} of a large attachment\n").as_bytes());
        i += 1;
    }
    out.truncate(len);
    out
}

async fn settings(state: &AppState, device: &str, large: &[(&str, &str)]) {
    for (key, value) in [
        ("sync_device_id", device),
        ("ui_locale", "ru"),
        ("minimize_to_tray", "1"),
        ("close_to_tray", "0"),
        ("start_hidden", "0"),
        ("notes_lock_timeout_min", "5"),
        ("notes_capture_rules", "[{\"host\":\"example.com\"}]"),
        ("quick_capture_shortcut", "CommandOrControl+Shift+N"),
        ("notes_custom_dir", ""),
        ("sync_interval_sec", "60"),
    ]
    .iter()
    .chain(large)
    {
        config::set_setting(&state.db, key, value).await.unwrap();
    }
}

const POLICY: (&str, &str) = (
    "notes_attachment_policy",
    "{\"large_files_enabled\":true,\"threshold_mib\":1,\"max_file_gib\":10,\"download_on_sync\":true,\"ask_above_mib\":16}",
);

/// The rows every fixture has: one of each table entity of 4.0.7.
async fn rows_fixture(state: &AppState, profile_path: &str) {
    sql(
        state,
        &format!("UPDATE workspaces SET created_at = '{T1}', updated_at = '{T1}'"),
    )
    .await;
    sql(state, &format!(
        "INSERT INTO workspaces (id, name, description, color, icon, notes, is_default, created_at, updated_at) VALUES
         ('ws-1', 'SMM', 'Client accounts', '#22c55e', 'briefcase', NULL, 0, '{T1}', '{T2}'),
         ('ws-2', 'Ops', NULL, '#6366f1', 'folder', 'team notes', 1, '{T1}', '{T1}')"
    )).await;
    sql(state, &format!(
        "INSERT INTO workspace_columns (id, workspace_id, name, tag_name, color, position, created_at) VALUES
         ('col-1', 'ws-1', 'Warm-up', 'warm-up', '#6366f1', 2, '{T1}')"
    )).await;
    sql(state, &format!(
        "INSERT INTO proxies (id, name, proxy_type, host, port, username, password, country, city, status, created_at, tags) VALUES
         ('px-1', 'DE residential', 'socks5', '10.0.0.7', 1080, 'user', 'secret', 'DE', 'Berlin', 'ok', '{T1}', '[\"workspace:ws-1\"]')"
    )).await;
    sql(state, &format!(
        "INSERT INTO ssh_keys (id, name, algorithm, bits, comment, private_key, public_key, passphrase, fingerprint, source, created_at, updated_at) VALUES
         ('key-1', 'deploy', 'ed25519', NULL, 'ci', '-----BEGIN OPENSSH PRIVATE KEY-----', 'ssh-ed25519 AAAA', NULL, 'SHA256:abc', 'generated', '{T1}', '{T2}')"
    )).await;
    sql(state, &format!(
        "INSERT INTO profiles (id, name, status, profile_path, browser_type, proxy_id, fingerprint_preset, timezone, locale, languages,
           screen_width, screen_height, webrtc_mode, geolocation_enabled, latitude, longitude, notes, workspace_id, kanban_status,
           kanban_order, tags, created_at, updated_at, last_launch_at, default_search_engine, history_enabled) VALUES
         ('pr-1', 'Brand A', 'stopped', '{profile_path}', 'camoufox', 'px-1', 'windows', 'Europe/Berlin', 'de-DE', 'de-DE,de,en',
           1920, 1080, 'real_ip', 1, 52.52, 13.4, 'main account', 'ws-1', 'new', 3, '[\"warm-up\"]', '{T1}', '{T2}', '{T2}', 'ddg', 1)"
    )).await;
    sql(state, &format!(
        "INSERT INTO ssh_connections (id, name, host, port, username, auth_type, requires_2fa, totp_entry_id, proxy_id, ssh_key_id,
           connect_timeout_sec, keepalive_sec, default_cols, default_rows, server_fingerprint, last_connected_at, created_at, updated_at) VALUES
         ('ssh-1', 'prod-web-01', '203.0.113.5', 22, 'deploy', 'key', 1, 'totp-1', 'px-1', 'key-1', 15, 30, 120, 32, 'SHA256:def', '{T2}', '{T1}', '{T2}')"
    )).await;
    sql(state, "INSERT INTO ssh_connection_workspaces (connection_id, workspace_id) VALUES ('ssh-1', 'ws-1'), ('ssh-1', 'ws-2')").await;
    sql(
        state,
        "INSERT INTO ssh_connection_profiles (connection_id, profile_id) VALUES ('ssh-1', 'pr-1')",
    )
    .await;
    sql(state, &format!(
        "INSERT INTO totp_entries (id, name, issuer, secret, algorithm, digits, period, tags, created_at, updated_at, last_used_at) VALUES
         ('totp-1', 'deploy@prod', 'Veydan', 'JBSWY3DPEHPK3PXP', 'SHA1', 6, 30, '[\"profile:pr-1\"]', '{T1}', '{T2}', '{T2}')"
    )).await;
    sql(state, &format!(
        "INSERT INTO password_vault (id, vault_id, crypto_version, kdf_algorithm, kdf_salt, kdf_memory, kdf_iterations, kdf_parallelism,
           wrapped_key, created_at, updated_at, recovery_salt, recovery_wrapped_key, lock_hash, lock_kind, lock_hint) VALUES
         ('default', '{VAULT_KEY}', 1, 'argon2id', 'c2FsdHNhbHRzYWx0c2FsdA==', 65536, 3, 1, 'd3JhcHBlZC1rZXk=', '{T1}', '{T2}',
           'cmVjb3Zlcnktc2FsdC0xNg==', 'cmVjb3Zlcnktd3JhcA==', '$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA', 'pin', 'the usual')"
    )).await;
    sql(state, &format!(
        "INSERT INTO passwords (id, title, username, url, password_enc, note_enc, totp_ids, tags, vault_id, created_at, updated_at) VALUES
         ('pw-1', 'Hosting panel', 'admin', 'https://panel.example', 'v1:bm9uY2U=:Y2lwaGVy', NULL, '[\"totp-1\"]', '[\"note:note-1\"]', '{VAULT_KEY}', '{T1}', '{T2}'),
         ('pw-2', 'Mail', NULL, NULL, 'v1:bm9uY2U=:bWFpbA==', 'v1:bm9uY2U=:bm90ZQ==', '[]', '[]', '{VAULT_KEY}', '{T1}', '{T1}')"
    )).await;
    sql(state, &format!("INSERT INTO password_history (id, password, created_at) VALUES ('h-1', 'x9!kPq', '{T1}')")).await;
    sql(state, &format!(
        "INSERT INTO note_tags (id, name, color, created_at, updated_at) VALUES
         ('tag-1', 'runbook', '#f97316', '{T1}', '{T2}'), ('tag-2', 'inbox', '#6366f1', '{T1}', '{T1}')"
    )).await;
    sql(state, &format!(
        "INSERT INTO note_folders (id, name, parent_id, color, created_at, updated_at) VALUES
         ('f-1', 'DevOps', NULL, '#6366f1', '{T1}', '{T2}'), ('f-2', 'Runbooks', 'f-1', '#6366f1', '{T1}', '{T2}')"
    )).await;
    sql(state, &format!(
        "INSERT INTO note_smart_views (id, name, color, conditions, sort_order, created_at, updated_at) VALUES
         ('sv-1', 'Open tasks', '#8b7bff', '{{\"has_open_tasks\":true}}', 1, '{T1}', '{T2}')"
    )).await;
    for (id, title, pinned, archived) in [
        ("note-1", "Deploy", 1, 0),
        ("note-2", "Plain", 0, 0),
        ("note-3", "Old", 0, 1),
    ] {
        sql(state, &format!(
            "INSERT INTO notes (id, title, file_path, format, pinned, archived, created_at, updated_at) VALUES
             ('{id}', '{title}', 'notes/documents/{id}.md', 'md', {pinned}, {archived}, '{T1}', '{T2}')"
        )).await;
        write(
            &documents(state).join(format!("{id}.md")),
            &note_file(id, title, &format!("Text of {title}.")),
        );
    }
    sql(state, "INSERT INTO note_folder_links (note_id, folder_id) VALUES ('note-1', 'f-1'), ('note-1', 'f-2')").await;
    sql(
        state,
        "INSERT INTO note_tag_links (note_id, tag_id) VALUES ('note-1', 'tag-1')",
    )
    .await;
}

/// The edits between the two collects of `collect_tests` in 5.x.
async fn edit(state: &AppState) {
    sql(
        state,
        "UPDATE workspaces SET name = 'SMM team' WHERE id = 'ws-1'",
    )
    .await;
    sql(state, "DELETE FROM password_history WHERE id = 'h-1'").await;
    sql(state, "DELETE FROM passwords WHERE id = 'pw-2'").await;
    sql(
        state,
        "UPDATE app_settings SET value = 'en' WHERE key = 'ui_locale'",
    )
    .await;
    sql(state, "DELETE FROM app_settings WHERE key = 'start_hidden'").await;
    sql(
        state,
        "UPDATE app_settings SET value = '/elsewhere' WHERE key = 'notes_custom_dir'",
    )
    .await;
    sql(state, "UPDATE notes SET pinned = 0 WHERE id = 'note-1'").await;
    sql(
        state,
        "DELETE FROM note_folder_links WHERE note_id = 'note-1' AND folder_id = 'f-2'",
    )
    .await;
    sql(state, "UPDATE notes SET archived = 0 WHERE id = 'note-2'").await;
    sql(state, "DELETE FROM note_smart_views WHERE id = 'sv-1'").await;
    sql(
        state,
        "UPDATE note_tags SET color = '#ef4444' WHERE id = 'tag-2'",
    )
    .await;
    sql(state, "UPDATE notes SET deleted = 1 WHERE id = 'note-3'").await;
    write(
        &documents(state).join("note-2.md"),
        &note_file("note-2", "Plain", "Text of Plain, edited."),
    );
}

fn op_json(op: &Op) -> Value {
    json!({
        "entity_type": op.entity_type,
        "entity_id": op.entity_id,
        "hlc": op.hlc.encode(),
        "deleted": op.deleted,
        "payload": op.payload,
    })
}

async fn row_states_json(state: &AppState) -> Value {
    let rows: Vec<(String, String, String, String, i64)> = sqlx::query_as(
        "SELECT entity_type, entity_id, head_hlc, synced_hash, deleted FROM sync_row_state
         ORDER BY entity_type, entity_id",
    )
    .fetch_all(&state.db)
    .await
    .unwrap();
    Value::Array(
        rows.into_iter()
            .map(|(entity, id, hlc, hash, deleted)| {
                json!({ "entity": entity, "id": id, "hlc": hlc, "hash": hash, "deleted": deleted })
            })
            .collect(),
    )
}

fn save(name: &str, value: &Value) {
    let path = out_dir().join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, serde_json::to_string_pretty(value).unwrap() + "\n").unwrap();
}

/// The collect steps of `cycle_inner`, in its order, with the states saved
/// as after a push.
async fn collect_407(
    app: &tauri::AppHandle,
    state: &AppState,
    engine: &Engine,
    clock: &mut HlcClock,
) -> Vec<Op> {
    let note_rows = rows::collect_local_changes(state, clock, rows::RowScope::Notes)
        .await
        .unwrap();
    let att = attachments::collect_local_changes(engine, app, state, clock)
        .await
        .unwrap();
    assert!(att.errors.is_empty(), "{:?}", att.errors);
    let changes = notes::collect_local_changes(engine, state, clock)
        .await
        .unwrap();
    let table_rows = rows::collect_local_changes(state, clock, rows::RowScope::App)
        .await
        .unwrap();
    let leases = profile_files::collect_leases(state, clock).await.unwrap();
    let db = &state.db;
    for st in note_rows.states.iter().chain(&table_rows.states) {
        state::save_row_state(db, st).await.unwrap();
    }
    for st in &att.states {
        state::save_attachment_state(db, st).await.unwrap();
    }
    for st in &changes.states {
        state::save_note_state(db, st).await.unwrap();
    }
    for st in &leases.states {
        state::save_profile_files_state(db, st).await.unwrap();
    }
    let mut ops = note_rows.ops;
    ops.extend(att.ops);
    ops.extend(changes.ops);
    ops.extend(table_rows.ops);
    ops.extend(leases.ops);
    ops
}

/// The data file of `collect_tests` in 5.x, collected by 4.0.7 in two rounds
/// with the same clock: what 4.0.7 pushes for that local state.
#[test]
fn collect() {
    let app = app();
    let handle = app.handle().clone();
    tauri::async_runtime::block_on(async move {
        let dir = out_dir().join("collect-data");
        let _ = std::fs::remove_dir_all(&dir);
        let state = state_in(&dir).await;
        handle.manage(state);
        let state = handle.state::<AppState>();
        settings(&state, DEVICE, &[POLICY]).await;
        rows_fixture(&state, "/somewhere/profiles/pr-1").await;
        let attachments_dir = documents(&state).join("attachments");
        write(
            &attachments_dir.join("note-1").join("scan.pdf"),
            b"%PDF-1.4 a small scan",
        );
        write(
            &attachments_dir.join("note-2").join("video.bin"),
            &large_bytes(3 << 19),
        );
        sql(
            &state,
            "INSERT INTO sync_row_state (entity_type, entity_id, head_hlc, synced_hash, deleted) VALUES
             ('proxy', 'px-gone', '0000018f00000000-00000000-peer', 'gone', 0)",
        )
        .await;
        sql(&state, &format!(
            "INSERT INTO sync_profile_files_state (profile_id, lease_device, lease_name, lease_since, lease_synced) VALUES
             ('pr-1', '{DEVICE}', 'Laptop', '{T1}', 0)"
        )).await;

        let vault = out_dir().join("collect-vault");
        let _ = std::fs::remove_dir_all(&vault);
        let vmk = Vmk::from_base64("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=").unwrap();
        let engine = Engine::with_key(Box::new(LocalDir::new(&vault)), &vmk, "golden-vault", DEVICE);
        let start = Hlc {
            wall_ms: 4_102_444_800_000,
            counter: 0,
            device_id: DEVICE.into(),
        };
        let mut clock = HlcClock::new(DEVICE, Some(&start));
        let mut rounds = Vec::new();
        for round in 0..2 {
            if round == 1 {
                edit(&state).await;
                std::fs::remove_file(attachments_dir.join("note-1").join("scan.pdf")).unwrap();
                sql(&state, "UPDATE sync_profile_files_state SET lease_device = '', lease_name = '', lease_since = '', lease_synced = 0
                     WHERE profile_id = 'pr-1'").await;
            }
            let ops: Vec<Value> = collect_407(&handle, &state, &engine, &mut clock)
                .await
                .iter()
                .map(op_json)
                .collect();
            rounds.push(json!({ "ops": ops, "row_states": row_states_json(&state).await }));
        }
        save("collect.json", &json!({ "rounds": rounds }));
    });
}

// ── A vault written by 4.0.7 ─────────────────────────────────────────────────

fn vault_dir() -> PathBuf {
    out_dir().join("vault")
}

async fn use_vault(state: &AppState, vault: &Path) {
    config::set_setting(&state.db, "sync_backend", "folder").await.unwrap();
    config::set_setting(&state.db, "sync_folder_path", &vault.to_string_lossy())
        .await
        .unwrap();
}

async fn cycle(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let warnings = super::cycle_inner(app, &state).await.expect("cycle");
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// Three cycles of a 4.0.7 desktop over a vault in a folder: every entity
/// type, the forms of the key row, puts and tombstones.
#[test]
fn write_vault() {
    let app = app();
    let handle = app.handle().clone();
    tauri::async_runtime::block_on(async move {
        let dir = out_dir().join("writer-data");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(vault_dir());
        std::fs::create_dir_all(vault_dir()).unwrap();
        let state = state_in(&dir).await;
        handle.manage(state);
        let state = handle.state::<AppState>();
        settings(&state, WRITER, &[POLICY, ("sync_device_name", "Writer 4.0.7")]).await;
        let profile_path = dir.join("profiles").join("pr-1");
        rows_fixture(&state, &profile_path.to_string_lossy()).await;
        let ff = profile_path.join("firefox-profile");
        write(&ff.join("prefs.js"), b"user_pref(\"browser.startup.page\", 3);\n");
        write(&ff.join("bookmarks").join("places.json"), b"{\"places\":[\"https://panel.example\"]}");
        let attachments_dir = documents(&state).join("attachments");
        write(
            &attachments_dir.join("note-1").join("scan.pdf"),
            b"%PDF-1.4 a small scan",
        );
        write(
            &attachments_dir.join("note-1").join("gone.txt"),
            b"an attachment deleted later",
        );
        write(
            &attachments_dir.join("note-2").join("video.txt"),
            &text_bytes(3 << 19),
        );
        write(
            &attachments_dir.join("note-2").join("old.txt"),
            &text_bytes((1 << 20) + 17),
        );
        // Rows deleted in round 2: a tombstone of every table entity.
        let doomed = [
            format!("INSERT INTO workspaces (id, name, description, color, icon, notes, is_default, created_at, updated_at) VALUES
             ('ws-3', 'Gone', NULL, '#6366f1', 'folder', NULL, 0, '{T1}', '{T1}')"),
            format!("INSERT INTO workspace_columns (id, workspace_id, name, tag_name, color, position, created_at) VALUES
             ('col-2', 'ws-1', 'Gone', 'gone', '#6366f1', 3, '{T1}')"),
            format!("INSERT INTO proxies (id, name, proxy_type, host, port, created_at, tags) VALUES
             ('px-2', 'Gone', 'http', '10.0.0.8', 8080, '{T1}', '[]')"),
            format!("INSERT INTO ssh_keys (id, name, algorithm, private_key, public_key, fingerprint, source, created_at, updated_at) VALUES
             ('key-2', 'gone', 'ed25519', 'k', 'p', 'SHA256:x', 'generated', '{T1}', '{T1}')"),
            format!("INSERT INTO profiles (id, name, status, profile_path, browser_type, fingerprint_preset, created_at, updated_at) VALUES
             ('pr-2', 'Gone', 'stopped', '{}', 'camoufox', 'windows', '{T1}', '{T1}')", dir.join("profiles").join("pr-2").to_string_lossy()),
            format!("INSERT INTO ssh_connections (id, name, host, port, username, auth_type, created_at, updated_at) VALUES
             ('ssh-2', 'gone', '203.0.113.9', 22, 'root', 'password', '{T1}', '{T1}')"),
            format!("INSERT INTO totp_entries (id, name, issuer, secret, algorithm, digits, period, tags, created_at, updated_at) VALUES
             ('totp-2', 'gone', NULL, 'JBSWY3DPEHPK3PXQ', 'SHA1', 6, 30, '[]', '{T1}', '{T1}')"),
            format!("INSERT INTO note_tags (id, name, color, created_at, updated_at) VALUES ('tag-3', 'gone', '#6366f1', '{T1}', '{T1}')"),
            format!("INSERT INTO note_folders (id, name, parent_id, color, created_at, updated_at) VALUES ('f-3', 'Gone', NULL, '#6366f1', '{T1}', '{T1}')"),
        ];
        for statement in &doomed {
            sql(&state, statement).await;
        }

        use_vault(&state, &vault_dir()).await;
        let storage = config::build_storage(&load_config(&state.db).await).unwrap();
        let (engine, vmk) = Engine::create(storage, PASSPHRASE, WRITER).await.unwrap();
        super::bind(&state.db, engine.vault_id(), &vmk).await.unwrap();
        drop(engine);
        profile_files::acquire_lease(&state, "pr-1").await.unwrap();
        cycle(&handle).await;

        // Round 2: tombstones of every kind, the key row without a lock.
        edit(&state).await;
        std::fs::remove_file(attachments_dir.join("note-1").join("gone.txt")).unwrap();
        std::fs::remove_file(attachments_dir.join("note-2").join("old.txt")).unwrap();
        profile_files::on_profile_stopped(&state, "pr-1").await.unwrap();
        for (table, id) in [
            ("workspaces", "ws-3"),
            ("workspace_columns", "col-2"),
            ("proxies", "px-2"),
            ("ssh_keys", "key-2"),
            ("profiles", "pr-2"),
            ("ssh_connections", "ssh-2"),
            ("totp_entries", "totp-2"),
            ("note_tags", "tag-3"),
            ("note_folders", "f-3"),
        ] {
            sql(&state, &format!("DELETE FROM {table} WHERE id = '{id}'")).await;
        }
        sql(&state, "UPDATE password_vault SET lock_hash = NULL, lock_kind = 'none', lock_hint = NULL,
             recovery_salt = NULL, recovery_wrapped_key = NULL").await;
        cycle(&handle).await;

        // Round 3: the key row as before a lock is set; the lease is taken again.
        sql(&state, "UPDATE password_vault SET lock_kind = NULL").await;
        config::set_setting(&state.db, "start_hidden", "1").await.unwrap();
        profile_files::acquire_lease(&state, "pr-1").await.unwrap();
        cycle(&handle).await;

        // Every op of the vault, by type, as a reader pulls it.
        let binding = config::load_binding(&state.db).await.unwrap();
        let reader = Engine::with_key(
            Box::new(LocalDir::new(&vault_dir())),
            &Vmk::from_base64(&binding.vmk_b64).unwrap(),
            &binding.vault_id,
            READER,
        );
        let pulled = reader.pull(&mut LocalState::default(), |_, _, _| {}).await.unwrap();
        assert!(pulled.errors.is_empty(), "{:?}", pulled.errors);
        let mut by_type: std::collections::BTreeMap<String, Vec<Value>> = Default::default();
        for op in &pulled.ops {
            by_type.entry(op.entity_type.clone()).or_default().push(op_json(op));
        }
        for (entity, ops) in &by_type {
            save(&format!("golden/{entity}.json"), &Value::Array(ops.clone()));
        }
        save(
            "vault.json",
            &json!({ "passphrase": PASSPHRASE, "vault_id": binding.vault_id, "writer": WRITER }),
        );
    });
}

async fn files_under(dir: &Path) -> Value {
    fn walk(root: &Path, dir: &Path, out: &mut std::collections::BTreeMap<String, Value>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let bytes = std::fs::read(&path).unwrap();
                let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                let value = match String::from_utf8(bytes.clone()) {
                    Ok(text) if text.len() < 4096 => json!(text),
                    _ => json!({ "size": bytes.len(), "sha256": veydan_sync::sha256_hex(&bytes) }),
                };
                out.insert(rel, value);
            }
        }
    }
    let mut out = Default::default();
    walk(dir, dir, &mut out);
    json!(out)
}

/// A 4.0.7 desktop joins the vault `write_vault` left and runs one cycle:
/// what 4.0.7 makes of every op of it.
#[test]
fn apply_vault() {
    let app = app();
    let handle = app.handle().clone();
    tauri::async_runtime::block_on(async move {
        let dir = out_dir().join("reader-data");
        let _ = std::fs::remove_dir_all(&dir);
        let vault = out_dir().join("reader-vault");
        let _ = std::fs::remove_dir_all(&vault);
        copy_dir(&vault_dir(), &vault);
        let state = state_in(&dir).await;
        handle.manage(state);
        let state = handle.state::<AppState>();
        config::set_setting(&state.db, "sync_device_id", READER).await.unwrap();
        config::set_setting(&state.db, "sync_device_name", "Reader 4.0.7").await.unwrap();
        use_vault(&state, &vault).await;
        let storage = config::build_storage(&load_config(&state.db).await).unwrap();
        let (engine, vmk) = Engine::open(storage, PASSPHRASE, READER).await.unwrap();
        super::bind(&state.db, engine.vault_id(), &vmk).await.unwrap();
        drop(engine);
        cycle(&handle).await;
        save("apply.json", &dump(&state).await);
    });
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let path = e.path();
        if path.is_dir() {
            copy_dir(&path, &to.join(e.file_name()));
        } else {
            std::fs::copy(&path, to.join(e.file_name())).unwrap();
        }
    }
}

/// What a device holds of the synced data: the synced columns of every row,
/// the notes and their attachments, the files of the profiles, the leases.
async fn dump(state: &AppState) -> Value {
    let db = &state.db;
    let mut tables = serde_json::Map::new();
    for spec in rows::SPECS {
        let mut rows_of = serde_json::Map::new();
        for (id, payload) in rows::read_rows(db, spec).await.unwrap() {
            rows_of.insert(id, payload);
        }
        tables.insert(spec.entity.into(), Value::Object(rows_of));
    }
    let notes: Vec<(String, String, i64)> =
        sqlx::query_as("SELECT id, file_path, deleted FROM notes ORDER BY id")
            .fetch_all(db)
            .await
            .unwrap();
    let mut note_files = serde_json::Map::new();
    for (id, file_path, deleted) in notes {
        let path = crate::commands::notes::resolve_note_abs_path(&state.app_data_dir, &file_path);
        note_files.insert(
            id,
            json!({
                "deleted": deleted,
                "text": std::fs::read_to_string(&path).ok(),
            }),
        );
    }
    let leases: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT profile_id, lease_device, lease_name, lease_since FROM sync_profile_files_state ORDER BY profile_id",
    )
    .fetch_all(db)
    .await
    .unwrap();
    let profiles: Vec<(String, String)> =
        sqlx::query_as("SELECT id, profile_path FROM profiles ORDER BY id")
            .fetch_all(db)
            .await
            .unwrap();
    let mut profile_files = serde_json::Map::new();
    for (id, path) in profiles {
        profile_files.insert(id, files_under(&Path::new(&path).join("firefox-profile")).await);
    }
    json!({
        "rows": tables,
        "notes": note_files,
        "attachments": files_under(&documents(state).join("attachments")).await,
        "profile_files": profile_files,
        "leases": leases
            .into_iter()
            .map(|(p, d, n, s)| json!({ "profile_id": p, "device": d, "name": n, "since": s }))
            .collect::<Vec<_>>(),
    })
}
