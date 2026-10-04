// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Products made of different modules share one vault (spec 9.2, 9.4): what
//! a product does not register it neither takes nor deletes, its garbage
//! collection keeps what it cannot judge, and once it gains a module it
//! takes what it skipped. The devices run whole cycles of
//! `veydan_sync_host::cycle` over a vault in a folder, each on a data file
//! of its own.

use crate::modules::{sync_registry, sync_registry_of, TestStates, SYNC_PLAN};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use veydan_core::{settings, Label};
use veydan_sync::{Engine, Hlc, HlcClock, Op};
use veydan_sync_host::config::{self, build_storage, load_config};
use veydan_sync_host::{gc, join, open_engine, rows, state, Part, Registry, SyncManager};

const PASSPHRASE: &str = "correct horse battery";
const T1: &str = "2026-09-01T10:00:00+00:00";
const T2: &str = "2026-09-02T11:30:00+00:00";

/// The seven settings 4.0.7 shares between desktops.
const SHARED_SETTINGS: [(&str, &str); 7] = [
    ("ui_locale", "ru"),
    ("minimize_to_tray", "1"),
    ("close_to_tray", "0"),
    ("start_hidden", "0"),
    ("notes_lock_timeout_min", "5"),
    ("notes_capture_rules", "[{\"host\":\"example.com\"}]"),
    ("quick_capture_shortcut", "CommandOrControl+Shift+N"),
];

/// The system entities and those of the notes.
fn notes_like() -> Registry {
    sync_registry_of(&["notes"])
}

/// The system entities and those of passwords and TOTP.
fn pass_like() -> Registry {
    sync_registry_of(&["pass"])
}

// ── Devices ──────────────────────────────────────────────────────────────────

/// A started device of a product that syncs `registry`, pointed at the vault
/// in `vault`, with the key of its passwords open and no lock.
pub(super) async fn device(registry: Registry, vault: &Path) -> TestStates {
    device_owning(registry, &[], vault).await
}

/// The modules of Space that own kinds of the directory.
pub(super) fn space_owners() -> Vec<&'static str> {
    crate::modules::directory_parts::<tauri::Wry>()
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

/// The same where the modules named `owners` own their kinds of the directory.
pub(super) async fn device_owning(registry: Registry, owners: &[&str], vault: &Path) -> TestStates {
    let state = TestStates::owning(registry, owners).await;
    let db = &state.core.db;
    settings::set(db, "sync_backend", "folder").await.unwrap();
    settings::set(db, "sync_folder_path", &vault.to_string_lossy())
        .await
        .unwrap();
    state::check_registry(db, state.sync.registry())
        .await
        .unwrap();
    state.lock.open_default().await.unwrap();
    state
}

/// The app starts again with what `registry` syncs.
async fn restart(state: &mut TestStates, registry: Registry) -> bool {
    state.sync = SyncManager::new(registry);
    state::check_registry(&state.core.db, state.sync.registry())
        .await
        .unwrap()
}

async fn create_vault(state: &TestStates) {
    let db = &state.core.db;
    let storage = build_storage(&load_config(db).await).unwrap();
    let device = config::device_id(db).await.unwrap();
    let (engine, vmk) = Engine::create(storage, PASSPHRASE, &device).await.unwrap();
    join::bind(db, state.sync.registry(), engine.vault_id(), &vmk)
        .await
        .unwrap();
}

/// `sync_join_vault` without the app.
async fn join_vault(state: &TestStates) {
    if join::join(
        &state.host(),
        &state.core,
        state.sync.registry(),
        PASSPHRASE,
    )
    .await
    .unwrap()
    {
        state.lock.refresh_after_sync().await;
    }
}

/// One whole cycle, which must end clean.
pub(super) async fn cycle(state: &TestStates) {
    let warnings = veydan_sync_host::cycle(state.host(), &state.core, &state.sync)
        .await
        .unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
}

pub(super) async fn device_id(state: &TestStates) -> String {
    config::device_id(&state.core.db).await.unwrap()
}

/// How many chunks this device has pushed to its log.
async fn own_seq(state: &TestStates) -> u64 {
    state::load_local_state(&state.core.db)
        .await
        .unwrap()
        .0
        .own_seq
}

/// Collection of garbage a day after a first pass: what was a candidate
/// then and still is goes.
pub(super) async fn collect_garbage(state: &TestStates) -> gc::Outcome {
    let db = &state.core.db;
    let engine = open_engine(&state.core).await.unwrap();
    let registry = state.sync.registry();
    gc::run(&engine, db, registry).await.unwrap();
    sqlx::query("UPDATE sync_gc_candidates SET first_seen = 0")
        .execute(db)
        .await
        .unwrap();
    gc::run(&engine, db, registry).await.unwrap()
}

/// Every row the device publishes, by entity and id.
async fn published_rows(state: &TestStates) -> BTreeMap<(String, String), Value> {
    let mut out = BTreeMap::new();
    for table in state.sync.registry().tables().iter().filter(|t| t.push) {
        for (id, payload) in rows::read_rows(&state.core.db, table).await.unwrap() {
            out.insert((table.spec.entity.to_string(), id), payload);
        }
    }
    out
}

async fn setting_values(state: &TestStates) -> Vec<(&'static str, Option<String>)> {
    let mut out = Vec::new();
    for (key, _) in SHARED_SETTINGS {
        out.push((key, settings::get(&state.core.db, key).await));
    }
    out
}

/// The entity types this device holds a sync position of.
async fn row_state_types(state: &TestStates) -> BTreeSet<String> {
    sqlx::query_scalar("SELECT DISTINCT entity_type FROM sync_row_state")
        .fetch_all(&state.core.db)
        .await
        .unwrap()
        .into_iter()
        .collect()
}

/// The latest ops of the vault that `author` wrote.
async fn ops_of(state: &TestStates, author: &str) -> Vec<Op> {
    let engine = open_engine(&state.core).await.unwrap();
    engine
        .all_latest_ops()
        .await
        .unwrap()
        .into_iter()
        .filter(|op| op.hlc.device_id == author)
        .collect()
}

/// Every file of the vault but the logs of the devices, which grow.
fn vault_files(vault: &Path) -> BTreeSet<PathBuf> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeSet<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.insert(path.strip_prefix(root).unwrap().to_owned());
            }
        }
    }
    let mut out = BTreeSet::new();
    walk(vault, vault, &mut out);
    out.retain(|p| !p.starts_with("devices"));
    out
}

pub(super) async fn close(state: TestStates) {
    state.core.db.close().await;
    let _ = std::fs::remove_dir_all(&state.core.app_data_dir);
}

// ── What Space writes ────────────────────────────────────────────────────────

async fn sql(state: &TestStates, statement: &str) {
    sqlx::query(sqlx::AssertSqlSafe(statement.to_string()))
        .execute(&state.core.db)
        .await
        .unwrap_or_else(|e| panic!("{statement}: {e}"));
}

pub(super) fn documents(state: &TestStates) -> PathBuf {
    state.core.app_data_dir.join("notes").join("documents")
}

pub(super) fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn note_file(id: &str, title: &str, body: &str) -> Vec<u8> {
    format!(
        "---\nid: {id}\ntitle: \"{title}\"\nformat: md\nbindings: []\ntags: []\ncreated_at: {T1}\nupdated_at: {T2}\n---\n{body}\n"
    )
    .into_bytes()
}

/// A note as the notes screen leaves it: its row and its file.
async fn write_note(state: &TestStates, id: &str, title: &str, body: &str) {
    sql(
        state,
        &format!(
        "INSERT INTO notes (id, title, file_path, format, pinned, archived, created_at, updated_at)
         VALUES ('{id}', '{title}', 'notes/documents/{id}.md', 'md', 0, 0, '{T1}', '{T2}')"
    ),
    )
    .await;
    write(
        &documents(state).join(format!("{id}.md")),
        &note_file(id, title, body),
    );
}

/// The text of the note's file on this device, `None` without one.
async fn note_text(state: &TestStates, id: &str) -> Option<String> {
    let path: Option<String> = sqlx::query_scalar("SELECT file_path FROM notes WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.core.db)
        .await
        .unwrap();
    let path = veydan_notes::resolve_note_abs_path(&state.core.app_data_dir, &path?);
    std::fs::read_to_string(path).ok()
}

/// A file of `len` bytes that zstd does not shrink to nothing.
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

/// A password as `password_create` stores it.
async fn save_password(state: &TestStates, id: &str, secret: &str) {
    let (key, vault_id) = state.lock.ensure_key().await.unwrap();
    let enc = veydan_lock::encrypt_field(&key, id, "password", secret).unwrap();
    sqlx::query(
        "INSERT INTO passwords (id, title, password_enc, vault_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(id)
    .bind(enc)
    .bind(vault_id)
    .bind(T1)
    .bind(T2)
    .execute(&state.core.db)
    .await
    .unwrap();
}

/// The secret as `password_reveal` returns it.
async fn reveal(state: &TestStates, id: &str) -> String {
    let (key, _) = state.lock.require_open().unwrap();
    let enc: String = sqlx::query_scalar("SELECT password_enc FROM passwords WHERE id = ?")
        .bind(id)
        .fetch_one(&state.core.db)
        .await
        .unwrap();
    veydan_lock::decrypt_field(&key, id, "password", &enc).unwrap()
}

/// A row of every entity of Space, notes with tags, folders and both forms
/// of attachments, the seven shared settings and a profile with files.
async fn space_writes_everything(state: &TestStates) {
    let db = &state.core.db;
    for (key, value) in SHARED_SETTINGS {
        settings::set(db, key, value).await.unwrap();
    }
    settings::set(
        db,
        "notes_attachment_policy",
        "{\"large_files_enabled\":true,\"threshold_mib\":1,\"max_file_gib\":10,\"download_on_sync\":true,\"ask_above_mib\":16}",
    )
    .await
    .unwrap();
    sql(state, &format!(
        "INSERT INTO workspaces (id, name, description, color, icon, notes, is_default, created_at, updated_at) VALUES
         ('ws-1', 'SMM', 'Client accounts', '#22c55e', 'briefcase', NULL, 0, '{T1}', '{T2}')"
    )).await;
    sql(state, &format!(
        "INSERT INTO workspace_columns (id, workspace_id, name, tag_name, color, position, created_at) VALUES
         ('col-1', 'ws-1', 'Warm-up', 'warm-up', '#6366f1', 2, '{T1}')"
    )).await;
    sql(state, &format!(
        "INSERT INTO proxies (id, name, proxy_type, host, port, username, password, country, city, status, created_at, tags) VALUES
         ('px-1', 'DE residential', 'socks5', '10.0.0.7', 1080, 'user', 'secret', 'DE', 'Berlin', 'ok', '{T1}', '[]')"
    )).await;
    sql(state, &format!(
        "INSERT INTO ssh_keys (id, name, algorithm, bits, comment, private_key, public_key, passphrase, fingerprint, source, created_at, updated_at) VALUES
         ('key-1', 'deploy', 'ed25519', NULL, 'ci', '-----BEGIN OPENSSH PRIVATE KEY-----', 'ssh-ed25519 AAAA', NULL, 'SHA256:abc', 'generated', '{T1}', '{T2}')"
    )).await;
    let profile_dir = state.core.app_data_dir.join("profiles").join("pr-1");
    sql(state, &format!(
        "INSERT INTO profiles (id, name, status, profile_path, browser_type, proxy_id, fingerprint_preset, timezone, locale, languages,
           screen_width, screen_height, webrtc_mode, geolocation_enabled, notes, workspace_id, kanban_status,
           kanban_order, tags, created_at, updated_at, default_search_engine, history_enabled) VALUES
         ('pr-1', 'Brand A', 'stopped', '{}', 'camoufox', 'px-1', 'windows', 'Europe/Berlin', 'de-DE', 'de-DE,de,en',
           1920, 1080, 'real_ip', 1, 'main account', 'ws-1', 'new', 3, '[]', '{T1}', '{T2}', 'ddg', 1)",
        profile_dir.display()
    )).await;
    let firefox = profile_dir.join("firefox-profile");
    write(&firefox.join("prefs.js"), b"user_pref(\"a\", 1);");
    write(
        &firefox.join("storage").join("data.bin"),
        &large_bytes(4096),
    );
    sql(state, &format!(
        "INSERT INTO ssh_connections (id, name, host, port, username, auth_type, requires_2fa, totp_entry_id, proxy_id, ssh_key_id,
           connect_timeout_sec, keepalive_sec, default_cols, default_rows, created_at, updated_at) VALUES
         ('ssh-1', 'prod-web-01', '203.0.113.5', 22, 'deploy', 'key', 1, 'totp-1', 'px-1', 'key-1', 15, 30, 120, 32, '{T1}', '{T2}')"
    )).await;
    sql(state, "INSERT INTO ssh_connection_workspaces (connection_id, workspace_id) VALUES ('ssh-1', 'ws-1')").await;
    sql(state, &format!(
        "INSERT INTO totp_entries (id, name, issuer, secret, algorithm, digits, period, tags, created_at, updated_at) VALUES
         ('totp-1', 'deploy@prod', 'Veydan', 'JBSWY3DPEHPK3PXP', 'SHA1', 6, 30, '[]', '{T1}', '{T2}')"
    )).await;
    save_password(state, "pw-1", "hunter2").await;
    sql(state, &format!("INSERT INTO password_history (id, password, created_at) VALUES ('h-1', 'x9!kPq', '{T1}')")).await;
    sql(state, &format!(
        "INSERT INTO note_tags (id, name, color, created_at, updated_at) VALUES ('tag-1', 'runbook', '#f97316', '{T1}', '{T2}')"
    )).await;
    sql(
        state,
        &format!(
            "INSERT INTO note_folders (id, name, parent_id, color, created_at, updated_at) VALUES
         ('f-1', 'DevOps', NULL, '#6366f1', '{T1}', '{T2}')"
        ),
    )
    .await;
    sql(state, &format!(
        "INSERT INTO note_smart_views (id, name, color, conditions, sort_order, created_at, updated_at) VALUES
         ('sv-1', 'Open tasks', '#8b7bff', '{{\"has_open_tasks\":true}}', 1, '{T1}', '{T2}')"
    )).await;
    write_note(state, "note-1", "Deploy", "Text of Deploy.").await;
    write_note(state, "note-2", "Plain", "Text of Plain.").await;
    sql(state, "UPDATE notes SET pinned = 1 WHERE id = 'note-1'").await;
    sql(
        state,
        "INSERT INTO note_folder_links (note_id, folder_id) VALUES ('note-1', 'f-1')",
    )
    .await;
    sql(
        state,
        "INSERT INTO note_tag_links (note_id, tag_id) VALUES ('note-1', 'tag-1')",
    )
    .await;
    let attachments = documents(state).join("attachments");
    write(
        &attachments.join("note-1").join("scan.pdf"),
        b"%PDF-1.4 a small scan",
    );
    write(
        &attachments.join("note-2").join("video.bin"),
        &large_bytes(3 << 19),
    );
    // The browser of the profile ran here.
    let device = config::device_id(db).await.unwrap();
    sql(state, &format!(
        "INSERT INTO sync_profile_files_state (profile_id, lease_device, lease_name, lease_since, lease_synced)
         VALUES ('pr-1', '{device}', 'Laptop', '{T1}', 0)"
    )).await;
}

/// The entity types of the ops a device of Space published.
async fn types_published(state: &TestStates) -> BTreeSet<String> {
    let author = device_id(state).await;
    ops_of(state, &author)
        .await
        .into_iter()
        .map(|op| op.entity_type)
        .collect()
}

// ── Rule 1: a product takes only what it registered ──────────────────────────

fn peer_op(entity: &str, id: &str, wall_ms: u64, payload: Value) -> Op {
    Op {
        entity_type: entity.into(),
        entity_id: id.into(),
        hlc: Hlc {
            wall_ms,
            counter: 0,
            device_id: "space-device".into(),
        },
        deleted: false,
        payload,
    }
}

/// What the row steps of the next collect would publish, as (entity, id).
async fn rows_to_publish(state: &TestStates) -> Vec<(String, String)> {
    let registry = state.sync.registry();
    let mut clock = HlcClock::new("notes-device", None);
    let mut published = Vec::new();
    for step in SYNC_PLAN.collect {
        if let Part::Rows { entities, .. } = step.part {
            let changes = rows::collect_local_changes(
                &state.core,
                &mut clock,
                &registry.rows(entities),
                None,
            )
            .await
            .unwrap();
            published.extend(
                changes
                    .ops
                    .into_iter()
                    .map(|op| (op.entity_type, op.entity_id)),
            );
        }
    }
    published
}

async fn row_states(state: &TestStates, entity: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT entity_id FROM sync_row_state WHERE entity_type = ? ORDER BY entity_id",
    )
    .bind(entity)
    .fetch_all(&state.core.db)
    .await
    .unwrap()
}

/// Space's tray setting reaches a product without the tray. Taken, it would
/// be out of the filter of the next collect, which publishes the tombstone
/// of a row it no longer reads: every Space would lose the setting. An op of
/// a type the product does not register leaves nothing behind either.
#[tokio::test]
async fn what_the_product_does_not_register_is_neither_taken_nor_deleted() {
    let state = TestStates::syncing(notes_like()).await;
    let first = rows_to_publish(&state).await;

    let ops = [
        peer_op(
            "setting",
            "minimize_to_tray",
            1_000,
            json!({ "value": "1" }),
        ),
        peer_op("setting", "ui_locale", 1_000, json!({ "value": "ru" })),
        peer_op("workspace", "ws-1", 1_000, json!({ "name": "SMM" })),
    ];
    let registry = state.sync.registry();
    for step in SYNC_PLAN.apply {
        if let Part::Rows { entities, .. } = step.part {
            let outcome =
                rows::apply_ops(&state.host(), &state.core, &registry.rows(entities), &ops)
                    .await
                    .unwrap();
            assert_eq!(outcome.retry, None, "{}", step.label);
        }
    }

    let db = &state.core.db;
    assert_eq!(settings::get(db, "minimize_to_tray").await, None);
    assert_eq!(settings::get(db, "ui_locale").await.as_deref(), Some("ru"));
    assert_eq!(row_states(&state, "setting").await, ["ui_locale"]);
    assert!(row_states(&state, "workspace").await.is_empty());
    let published: Vec<_> = rows_to_publish(&state)
        .await
        .into_iter()
        .filter(|row| !first.contains(row))
        .collect();
    assert!(published.is_empty(), "{published:?}");

    close(state).await;
}

// ── Labels ───────────────────────────────────────────────────────────────────

/// Apply `ops` by the row steps of Space's plan, as a cycle does.
async fn apply_rows(state: &TestStates, ops: &[Op]) {
    let registry = state.sync.registry();
    for step in SYNC_PLAN.apply {
        if let Part::Rows { entities, .. } = step.part {
            let outcome =
                rows::apply_ops(&state.host(), &state.core, &registry.rows(entities), ops)
                    .await
                    .unwrap();
            assert_eq!(outcome.retry, None, "{}", step.label);
        }
    }
}

async fn labels(state: &TestStates) -> Vec<(String, String, Option<String>)> {
    sqlx::query_as("SELECT key, name, parent_id FROM labels ORDER BY key")
        .fetch_all(&state.core.db)
        .await
        .unwrap()
}

fn tombstone(entity: &str, id: &str, wall_ms: u64) -> Op {
    Op {
        deleted: true,
        ..peer_op(entity, id, wall_ms, json!({}))
    }
}

/// Spec 10.2: the owners of Space name what sync writes in the labels, in
/// a rename too, and retract what a tombstone takes away; a profile left
/// without its workspace loses it as its parent. A product without the
/// owners keeps none of it and publishes nothing.
#[tokio::test]
async fn what_sync_writes_its_owners_name() {
    let space = TestStates::owning(sync_registry(), &space_owners()).await;
    let notes = TestStates::owning(notes_like(), &["notes"]).await;
    let put = |entity: &str, id: &str, wall_ms: u64, payload: Value| {
        peer_op(entity, id, wall_ms, payload)
    };
    let workspace = |name: &str| json!({ "name": name, "color": "#22c55e", "is_default": 0, "created_at": T1, "updated_at": T2 });
    let password = |title: &str| json!({ "title": title, "username": "root", "password_enc": "x", "vault_id": "v", "created_at": T1, "updated_at": T2 });
    let ops = [
        put("workspace", "ws-1", 1_000, workspace("SMM")),
        put(
            "profile",
            "pr-1",
            1_000,
            json!({ "name": "Brand A", "workspace_id": "ws-1", "created_at": T1 }),
        ),
        put(
            "proxy",
            "px-1",
            1_000,
            json!({ "name": "DE", "proxy_type": "socks5", "host": "10.0.0.7", "port": 1080, "created_at": T1 }),
        ),
        put(
            "ssh_connection",
            "ssh-1",
            1_000,
            json!({ "name": "prod", "host": "203.0.113.5", "port": 22, "username": "deploy", "created_at": T1, "updated_at": T2 }),
        ),
        put(
            "totp",
            "totp-1",
            1_000,
            json!({ "name": "deploy@prod", "issuer": "Veydan", "secret": "JBSWY3DPEHPK3PXP", "created_at": T1, "updated_at": T2 }),
        ),
        put("password", "pw-1", 1_000, password("Hosting")),
    ];
    for state in [&space, &notes] {
        apply_rows(state, &ops).await;
    }
    let key = |kind: &str, id: &str, name: &str, parent: Option<&str>| {
        (
            format!("{kind}:{id}"),
            name.to_string(),
            parent.map(str::to_string),
        )
    };
    assert_eq!(
        labels(&space).await,
        [
            key("password", "pw-1", "Hosting", None),
            key("profile", "pr-1", "Brand A", Some("ws-1")),
            key("proxy", "px-1", "DE", None),
            key("ssh", "ssh-1", "prod", None),
            key("totp", "totp-1", "deploy@prod", None),
            key("workspace", "ws-1", "SMM", None),
        ]
    );
    assert!(labels(&notes).await.is_empty());

    apply_rows(
        &space,
        &[
            put("workspace", "ws-1", 2_000, workspace("SMM team")),
            put("password", "pw-1", 2_000, password("Hosting panel")),
            tombstone("proxy", "px-1", 2_000),
            tombstone("totp", "totp-1", 2_000),
            tombstone("ssh_connection", "ssh-1", 2_000),
        ],
    )
    .await;
    assert_eq!(
        labels(&space).await,
        [
            key("password", "pw-1", "Hosting panel", None),
            key("profile", "pr-1", "Brand A", Some("ws-1")),
            key("workspace", "ws-1", "SMM team", None),
        ]
    );
    apply_rows(&space, &[tombstone("workspace", "ws-1", 3_000)]).await;
    assert_eq!(
        labels(&space).await,
        [
            key("password", "pw-1", "Hosting panel", None),
            key("profile", "pr-1", "Brand A", None),
        ]
    );

    // The labels of Space reach the product without the owners, which
    // keeps them and sends none of them back.
    let mut clock = HlcClock::new("space-device", None);
    let tables = space.sync.registry().rows(&["label"]);
    let pushed = rows::collect_local_changes(&space.core, &mut clock, &tables, None)
        .await
        .unwrap();
    assert_eq!(pushed.ops.len(), 2);
    let first = rows_to_publish(&notes).await;
    apply_rows(&notes, &pushed.ops).await;
    assert_eq!(labels(&notes).await, labels(&space).await);
    let published: Vec<_> = rows_to_publish(&notes)
        .await
        .into_iter()
        .filter(|row| !first.contains(row))
        .collect();
    assert!(published.is_empty(), "{published:?}");

    close(space).await;
    close(notes).await;
}

// ── Two registries on one vault ──────────────────────────────────────────────

/// Space writes every entity it has and names it in the labels. Notes and
/// Pass join the same vault, cycle, edit what they have and collect
/// garbage; Space's data comes through untouched, and the edit of the note
/// reaches Space. Each names what the other modules own by the labels and
/// publishes none of them.
#[tokio::test]
async fn notes_and_pass_share_a_vault_with_space_and_delete_nothing_of_it() {
    let vault = crate::db::test_dir();
    let space = device_owning(sync_registry(), &space_owners(), &vault).await;
    create_vault(&space).await;
    space_writes_everything(&space).await;
    assert_eq!(space.publish_labels().await, 9);
    cycle(&space).await;
    cycle(&space).await;
    let space_id = device_id(&space).await;
    let rows_before = published_rows(&space).await;
    let settings_before = setting_values(&space).await;
    let files_before = vault_files(&vault);
    let space_types = types_published(&space).await;
    for entity in sync_registry().entities() {
        assert!(space_types.contains(entity), "Space published no {entity}");
    }

    let notes = device_owning(notes_like(), &["notes"], &vault).await;
    join_vault(&notes).await;
    let pass = device_owning(pass_like(), &["pass"], &vault).await;
    join_vault(&pass).await;
    for _ in 0..3 {
        cycle(&notes).await;
        cycle(&pass).await;
    }

    // Each took its own entities and nothing else.
    let notes_types: BTreeSet<String> = notes
        .sync
        .registry()
        .entities()
        .into_iter()
        .map(str::to_string)
        .collect();
    let pass_types: BTreeSet<String> = pass
        .sync
        .registry()
        .entities()
        .into_iter()
        .map(str::to_string)
        .collect();
    assert!(row_state_types(&notes).await.is_subset(&notes_types));
    assert!(row_state_types(&pass).await.is_subset(&pass_types));
    assert_eq!(
        row_states(&notes, "setting").await,
        [
            "notes_lock_timeout_min",
            "quick_capture_shortcut",
            "ui_locale"
        ]
    );
    assert_eq!(
        note_text(&notes, "note-1").await,
        note_text(&space, "note-1").await
    );
    assert!(notes
        .core
        .app_data_dir
        .join("notes/documents/attachments/note-1/scan.pdf")
        .is_file());
    assert_eq!(reveal(&pass, "pw-1").await, "hunter2");
    assert_eq!(
        settings::get(&notes.core.db, "minimize_to_tray").await,
        None
    );

    // Spec 10.2, 10.3: what another module owns is named by its label — the
    // name, the color of a workspace, the workspace of a profile — and has
    // no field to show.
    let label = |kind: &str, id: &str, name: &str, parent: Option<&str>, color: Option<&str>| {
        Some(Label {
            kind: kind.into(),
            id: id.into(),
            name: name.into(),
            parent: parent.map(|ws| ("workspace".to_string(), ws.to_string())),
            color: color.map(str::to_string),
        })
    };
    let directory = &notes.core.directory;
    assert_eq!(
        directory.label("workspace", "ws-1").await,
        label("workspace", "ws-1", "SMM", None, Some("#22c55e"))
    );
    assert_eq!(
        directory.label("profile", "pr-1").await,
        label("profile", "pr-1", "Brand A", Some("ws-1"), None)
    );
    for (kind, id, name) in [
        ("password", "pw-1", "pw-1"),
        ("totp", "totp-1", "deploy@prod"),
        ("proxy", "px-1", "DE residential"),
        ("ssh", "ssh-1", "prod-web-01"),
    ] {
        assert_eq!(
            directory.label(kind, id).await,
            label(kind, id, name, None, None)
        );
    }
    let bindings = ["password:pw-1", "profile:pr-1", "workspace:ws-1"].map(String::from);
    let named: Vec<(String, String, String)> =
        veydan_notes::entities::summaries(directory, &bindings)
            .await
            .into_iter()
            .map(|s| (s.binding, s.name, s.subtitle))
            .collect();
    assert_eq!(
        named,
        [
            ("password:pw-1".into(), "pw-1".into(), String::new()),
            ("profile:pr-1".into(), "Brand A".into(), String::new()),
            ("workspace:ws-1".into(), "SMM".into(), String::new()),
        ]
    );
    assert!(directory
        .action("password", "pw-1", "field", Some("username"))
        .await
        .is_err());
    assert_eq!(
        pass.core
            .directory
            .label("note", "note-1")
            .await
            .map(|l| l.name),
        Some("Deploy".to_string())
    );
    assert_eq!(
        pass.core
            .directory
            .label("workspace", "ws-1")
            .await
            .map(|l| l.name),
        Some("SMM".into())
    );

    // The note is edited where only the notes run, then both collect garbage.
    let edited = note_file("note-1", "Deploy", "Text of Deploy, edited in Notes.");
    write(&documents(&notes).join("note-1.md"), &edited);
    cycle(&notes).await;
    for state in [&notes, &pass] {
        let outcome = collect_garbage(state).await;
        assert_eq!(outcome.removed, 0);
        assert!(outcome.unknown.contains(&"profile_snapshot".to_string()));
    }
    assert!(vault_files(&vault).is_superset(&files_before));

    // Neither published a tombstone, nor an entity it does not own, nor a
    // label: what they own they did not rename, the rest is not theirs.
    for (state, own) in [(&notes, &notes_types), (&pass, &pass_types)] {
        let author = device_id(state).await;
        for op in ops_of(state, &author).await {
            assert!(!op.deleted, "{} {}", op.entity_type, op.entity_id);
            assert!(own.contains(&op.entity_type), "{}", op.entity_type);
            assert_ne!(op.entity_type, "label", "{}", op.entity_id);
        }
    }

    cycle(&space).await;
    assert_eq!(
        note_text(&space, "note-1").await.as_deref(),
        Some(std::str::from_utf8(&edited).unwrap())
    );
    assert_eq!(published_rows(&space).await, rows_before);
    assert_eq!(setting_values(&space).await, settings_before);
    let latest: Vec<Op> = open_engine(&space.core)
        .await
        .unwrap()
        .all_latest_ops()
        .await
        .unwrap();
    for op in latest.iter().filter(|op| op.entity_type != "note") {
        assert!(!op.deleted, "{} {}", op.entity_type, op.entity_id);
    }
    for op in latest.iter().filter(|op| {
        !notes_types.contains(&op.entity_type) && !pass_types.contains(&op.entity_type)
    }) {
        assert_eq!(op.hlc.device_id, space_id, "{}", op.entity_type);
    }
    // Space collects garbage as it did: what it knows nothing keeps goes.
    let outcome = collect_garbage(&space).await;
    assert!(outcome.unknown.is_empty(), "{:?}", outcome.unknown);
    assert!(space
        .core
        .app_data_dir
        .join("profiles/pr-1/firefox-profile/prefs.js")
        .is_file());

    for state in [space, notes, pass] {
        close(state).await;
    }
    let _ = std::fs::remove_dir_all(&vault);
}

/// What Veydan Pass syncs: the registry the shell builds for the product of
/// `apps/pass` — the crate of pass alone, the shell's default plan (9.1).
fn pass_product() -> Registry {
    let modules = || vec![veydan_pass::module()];
    let product = veydan_shell::Product {
        id: "pass",
        name: "Veydan Pass",
        desktop_entry: "veydanpass",
        icon: "veydanpass",
        sync: veydan_shell::default_plan(&modules()),
    };
    veydan_shell::sync_registry(product, modules()).unwrap()
}

/// Spec 10.3: a password of Space linked to a profile and its workspace
/// reaches Pass, which has no browser; the command `labels_resolve` names
/// both from the labels Space published and gives the workspace its color;
/// a rename and a recolor in Space reach Pass. Pass takes the labels with its own
/// plan and writes nothing of Space's: no label, no tombstone, no row.
#[tokio::test]
async fn pass_names_the_profile_a_password_of_space_is_linked_to() {
    let vault = crate::db::test_dir();
    let space = device_owning(sync_registry(), &space_owners(), &vault).await;
    create_vault(&space).await;
    space_writes_everything(&space).await;
    sql(
        &space,
        "UPDATE passwords SET tags = '[\"profile:pr-1\",\"workspace:ws-1\",\"profile:pr-gone\"]'
         WHERE id = 'pw-1'",
    )
    .await;
    assert_eq!(space.publish_labels().await, 9);
    cycle(&space).await;
    cycle(&space).await;
    let space_id = device_id(&space).await;
    let rows_before = published_rows(&space).await;
    let files_before = vault_files(&vault);

    let pass = device_owning(pass_product(), &["pass"], &vault).await;
    let ask = |items: &[(&str, &str)]| {
        let items = items
            .iter()
            .map(|(kind, id)| veydan_shell::LabelRef {
                kind: kind.to_string(),
                id: id.to_string(),
            })
            .collect();
        let directory = &pass.core.directory;
        async move {
            veydan_shell::resolve_labels(directory, items)
                .await
                .unwrap()
                .into_iter()
                .map(|answer| (answer.kind, answer.id, answer.name))
                .collect::<Vec<_>>()
        }
    };
    let colors = |items: &[(&str, &str)]| {
        let items = items
            .iter()
            .map(|(kind, id)| veydan_shell::LabelRef {
                kind: kind.to_string(),
                id: id.to_string(),
            })
            .collect();
        let directory = &pass.core.directory;
        async move {
            veydan_shell::resolve_labels(directory, items)
                .await
                .unwrap()
                .into_iter()
                .map(|answer| answer.color)
                .collect::<Vec<_>>()
        }
    };
    let links = [
        ("profile", "pr-1"),
        ("workspace", "ws-1"),
        ("profile", "pr-gone"),
    ];
    // Before sync brought the labels the links have no names (10.3).
    assert!(ask(&links).await.iter().all(|(_, _, name)| name.is_none()));
    assert!(colors(&links).await.iter().all(Option::is_none));

    join_vault(&pass).await;
    for _ in 0..3 {
        cycle(&pass).await;
    }
    let tags: String = sqlx::query_scalar("SELECT tags FROM passwords WHERE id = 'pw-1'")
        .fetch_one(&pass.core.db)
        .await
        .unwrap();
    assert_eq!(
        tags,
        "[\"profile:pr-1\",\"workspace:ws-1\",\"profile:pr-gone\"]"
    );
    assert_eq!(reveal(&pass, "pw-1").await, "hunter2");
    let name = |kind: &str, id: &str, name: Option<&str>| {
        (kind.to_string(), id.to_string(), name.map(str::to_string))
    };
    assert_eq!(
        ask(&links).await,
        [
            name("profile", "pr-1", Some("Brand A")),
            name("workspace", "ws-1", Some("SMM")),
            name("profile", "pr-gone", None),
        ]
    );
    // The workspace has the color it has in Space; a profile has none.
    assert_eq!(
        colors(&links).await,
        [None, Some("#22c55e".to_string()), None]
    );
    assert_eq!(
        ask(&[("note", "note-1"), ("ssh", "ssh-1"), ("proxy", "px-1")]).await,
        [
            name("note", "note-1", Some("Deploy")),
            name("ssh", "ssh-1", Some("prod-web-01")),
            name("proxy", "px-1", Some("DE residential")),
        ]
    );

    // Space renames the profile: Pass shows the new name after its cycle.
    let mut tx = space.core.db.begin().await.unwrap();
    sqlx::query("UPDATE profiles SET name = 'Brand B' WHERE id = 'pr-1'")
        .execute(&mut *tx)
        .await
        .unwrap();
    space
        .core
        .directory
        .publish(
            &mut tx,
            &Label {
                kind: "profile".into(),
                id: "pr-1".into(),
                name: "Brand B".into(),
                parent: Some(("workspace".into(), "ws-1".into())),
                color: None,
            },
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    cycle(&space).await;
    cycle(&pass).await;
    assert_eq!(
        ask(&[("profile", "pr-1")]).await,
        [name("profile", "pr-1", Some("Brand B"))]
    );

    // Space recolors the workspace as its command does (`relabel` in the
    // transaction of the change): Pass shows the new color after its cycle.
    let mut tx = space.core.db.begin().await.unwrap();
    sqlx::query("UPDATE workspaces SET color = '#f97316' WHERE id = 'ws-1'")
        .execute(&mut *tx)
        .await
        .unwrap();
    crate::modules::directory::relabel(
        &space.core.directory,
        &mut tx,
        &crate::modules::directory::WORKSPACE,
        "ws-1",
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    cycle(&space).await;
    cycle(&pass).await;
    assert_eq!(
        colors(&[("workspace", "ws-1")]).await,
        [Some("#f97316".to_string())]
    );
    assert_eq!(
        ask(&[("workspace", "ws-1")]).await,
        [name("workspace", "ws-1", Some("SMM"))]
    );

    // The three rules (9.2). Pass took its own types and the labels, kept
    // every file of the vault, and published neither a tombstone, nor a
    // label, nor an entity it does not own.
    let pass_types: BTreeSet<String> = pass
        .sync
        .registry()
        .entities()
        .into_iter()
        .map(str::to_string)
        .collect();
    assert!(pass_types.contains("label") && !pass_types.contains("profile"));
    assert!(row_state_types(&pass).await.is_subset(&pass_types));
    let outcome = collect_garbage(&pass).await;
    assert_eq!(outcome.removed, 0);
    assert!(outcome.unknown.contains(&"profile_snapshot".to_string()));
    assert!(vault_files(&vault).is_superset(&files_before));
    cycle(&pass).await;
    let pass_id = device_id(&pass).await;
    for op in ops_of(&pass, &pass_id).await {
        assert!(!op.deleted, "{} {}", op.entity_type, op.entity_id);
        assert!(pass_types.contains(&op.entity_type), "{}", op.entity_type);
        assert_ne!(op.entity_type, "label", "{}", op.entity_id);
    }

    // Space after it: its rows are what they were but the renamed profile
    // and the recolored workspace,
    // nothing of it is deleted, and what Pass does not sync is Space's still.
    cycle(&space).await;
    let mut rows_after = published_rows(&space).await;
    let mut expected = rows_before;
    for rows in [&mut rows_after, &mut expected] {
        rows.remove(&("profile".to_string(), "pr-1".to_string()));
        rows.remove(&("label".to_string(), "profile:pr-1".to_string()));
        rows.remove(&("workspace".to_string(), "ws-1".to_string()));
        rows.remove(&("label".to_string(), "workspace:ws-1".to_string()));
    }
    assert_eq!(rows_after, expected);
    let latest: Vec<Op> = open_engine(&space.core)
        .await
        .unwrap()
        .all_latest_ops()
        .await
        .unwrap();
    for op in &latest {
        assert!(!op.deleted, "{} {}", op.entity_type, op.entity_id);
        if op.entity_type == "label" || !pass_types.contains(&op.entity_type) {
            assert_eq!(op.hlc.device_id, space_id, "{}", op.entity_type);
        }
    }

    for state in [space, pass] {
        close(state).await;
    }
    let _ = std::fs::remove_dir_all(&vault);
}

/// A note written where only the notes run reaches Space, and Space's edit
/// of it comes back.
#[tokio::test]
async fn a_note_goes_from_notes_to_space_and_back() {
    let vault = crate::db::test_dir();
    let space = device(sync_registry(), &vault).await;
    create_vault(&space).await;
    space_writes_everything(&space).await;
    cycle(&space).await;
    let notes = device(notes_like(), &vault).await;
    join_vault(&notes).await;
    cycle(&notes).await;

    write_note(&notes, "note-n", "From Notes", "Written in Notes.").await;
    sql(
        &notes,
        "INSERT INTO note_folder_links (note_id, folder_id) VALUES ('note-n', 'f-1')",
    )
    .await;
    sql(&notes, "UPDATE notes SET pinned = 1 WHERE id = 'note-n'").await;
    cycle(&notes).await;
    cycle(&space).await;
    let text = note_text(&space, "note-n")
        .await
        .expect("the note in Space");
    assert!(text.contains("Written in Notes."), "{text}");
    let flags: (i64, String) = sqlx::query_as(
        "SELECT n.pinned, l.folder_id FROM notes n JOIN note_folder_links l ON l.note_id = n.id
         WHERE n.id = 'note-n'",
    )
    .fetch_one(&space.core.db)
    .await
    .unwrap();
    assert_eq!(flags, (1, "f-1".to_string()));

    let edited = note_file("note-n", "From Notes", "Written in Notes, edited in Space.");
    write(&documents(&space).join("note-n.md"), &edited);
    cycle(&space).await;
    let outcome = collect_garbage(&space).await;
    assert!(outcome.unknown.is_empty(), "{:?}", outcome.unknown);
    cycle(&notes).await;
    assert_eq!(
        note_text(&notes, "note-n").await.as_deref(),
        Some(std::str::from_utf8(&edited).unwrap())
    );

    for state in [space, notes] {
        close(state).await;
    }
    let _ = std::fs::remove_dir_all(&vault);
}

// ── Rule 2: blobs of format v1 and types this build does not know ────────────

/// A snapshot of a profile as Space publishes it: a manifest blob naming the
/// blob of each file. Returns the blobs.
async fn push_snapshot(state: &TestStates) -> Vec<String> {
    let db = &state.core.db;
    let engine = open_engine(&state.core).await.unwrap();
    let file = engine.put_blob(b"user_pref(\"a\", 1);").await.unwrap();
    let manifest =
        format!(r#"{{"files":[{{"path":"prefs.js","blob":"{file}","size":19,"hash":"h1"}}]}}"#);
    let manifest_blob = engine.put_blob(manifest.as_bytes()).await.unwrap();
    let (mut local, last) = state::load_local_state(db).await.unwrap();
    let mut clock = HlcClock::new(engine.device_id().to_string(), last.as_ref());
    let op = Op {
        entity_type: "profile_snapshot".into(),
        entity_id: "pr-1".into(),
        hlc: clock.now(),
        deleted: false,
        payload: json!({ "manifest_blob": manifest_blob, "taken_at": T1, "files": 1, "bytes": 19 }),
    };
    engine.push(&mut local, vec![op]).await.unwrap();
    state::save_own_state(db, &local, &clock.last())
        .await
        .unwrap();
    vec![manifest_blob, file]
}

async fn blobs(state: &TestStates) -> BTreeSet<String> {
    let engine = open_engine(&state.core).await.unwrap();
    engine.list_blobs().await.unwrap()
}

/// The desktop Notes would take the files of Space's profile snapshots for
/// garbage: no op it knows names them. While the vault holds an op of a type
/// it does not know, its pass over blobs deletes nothing at all; Space, which
/// knows every type, collects as before.
#[tokio::test]
async fn blobs_only_an_unknown_type_names_outlive_the_garbage_collection() {
    let vault = crate::db::test_dir();
    let space = device(sync_registry(), &vault).await;
    create_vault(&space).await;
    let snapshot = push_snapshot(&space).await;
    let orphan = open_engine(&space.core)
        .await
        .unwrap()
        .put_blob(b"named by nothing")
        .await
        .unwrap();
    let notes = device(notes_like(), &vault).await;
    join_vault(&notes).await;

    let outcome = collect_garbage(&notes).await;
    assert_eq!(outcome.unknown, ["profile_snapshot"]);
    assert_eq!(outcome.removed, 0);
    let left = blobs(&notes).await;
    for blob in snapshot.iter().chain([&orphan]) {
        assert!(left.contains(blob), "{blob}");
    }
    let candidates: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_gc_candidates")
        .fetch_one(&notes.core.db)
        .await
        .unwrap();
    assert_eq!(candidates, 0);

    let outcome = collect_garbage(&space).await;
    assert!(outcome.unknown.is_empty());
    assert_eq!(outcome.removed, 1);
    let left = blobs(&space).await;
    assert!(!left.contains(&orphan));
    for blob in &snapshot {
        assert!(left.contains(blob), "{blob}");
    }

    for state in [space, notes] {
        close(state).await;
    }
    let _ = std::fs::remove_dir_all(&vault);
}

/// In a vault of the notes alone, Notes knows every type and collects as
/// before: the blob of its note stays, a blob nothing names goes.
#[tokio::test]
async fn a_product_that_knows_every_type_collects_as_before() {
    let vault = crate::db::test_dir();
    let notes = device(notes_like(), &vault).await;
    create_vault(&notes).await;
    write_note(&notes, "note-1", "Deploy", "Text of Deploy.").await;
    cycle(&notes).await;
    let engine = open_engine(&notes.core).await.unwrap();
    let orphan = engine.put_blob(b"named by nothing").await.unwrap();
    let before = blobs(&notes).await;

    let outcome = collect_garbage(&notes).await;
    assert!(outcome.unknown.is_empty(), "{:?}", outcome.unknown);
    assert_eq!(outcome.removed, 1);
    let mut expected = before;
    expected.remove(&orphan);
    assert_eq!(blobs(&notes).await, expected);

    close(notes).await;
    let _ = std::fs::remove_dir_all(&vault);
}

// ── Rule 3: the fingerprint of the registry ──────────────────────────────────

#[tokio::test]
async fn the_fingerprint_stays_across_restarts() {
    let mut reversed = Registry::new();
    for module in crate::modules::all().into_iter().rev() {
        if let Some(sync) = module.sync {
            reversed.add(module.id, sync);
        }
    }
    let reversed = reversed.finish(&SYNC_PLAN).unwrap();
    let ids: Vec<&str> = crate::modules::all().iter().map(|m| m.id).collect();
    let space = sync_registry();
    assert_eq!(space.fingerprint(), sync_registry().fingerprint());
    assert_ne!(space.fingerprint(), notes_like().fingerprint());
    // The order the modules register in does not count; what they register does.
    assert_eq!(reversed.fingerprint(), sync_registry_of(&ids).fingerprint());
    assert_ne!(
        space.fingerprint(),
        reversed.fingerprint(),
        "the shell's keys"
    );

    let mut state = TestStates::new().await;
    let db = state.core.db.clone();
    assert!(!state::check_registry(&db, state.sync.registry())
        .await
        .unwrap());
    assert!(!restart(&mut state, sync_registry()).await);
    assert_eq!(state::reread(&db).await, None);

    // A product that lost modules reads nothing again for that, and keeps
    // what the earlier registry knew for a later one.
    assert!(restart(&mut state, notes_like()).await);
    assert_eq!(state::reread(&db).await, Some(sync_registry().synced()));
    assert!(!restart(&mut state, notes_like()).await);
    assert!(restart(&mut state, pass_like()).await);
    assert_eq!(state::reread(&db).await, Some(notes_like().synced()));

    close(state).await;
}

/// A product that gains modules takes, by reading the vault again, what its
/// cycles skipped, and pushes nothing for it. The ops of what it knew before
/// are not applied again: a note put and its tombstone stay as they were,
/// and so does the key row it adopted when it joined.
#[tokio::test]
async fn a_product_that_gains_modules_takes_what_it_skipped_and_pushes_nothing() {
    let vault = crate::db::test_dir();
    let space = device(sync_registry(), &vault).await;
    create_vault(&space).await;
    space_writes_everything(&space).await;
    write_note(&space, "note-gone", "Gone", "Written and deleted.").await;
    cycle(&space).await;
    sql(
        &space,
        "UPDATE notes SET deleted = 1 WHERE id = 'note-gone'",
    )
    .await;
    cycle(&space).await;

    let mut device_b = device(pass_like(), &vault).await;
    join_vault(&device_b).await;
    cycle(&device_b).await;
    cycle(&device_b).await;
    let db = device_b.core.db.clone();
    let key_row: Value = rows::read_row(
        &db,
        device_b.sync.registry().table_of("password_vault").unwrap(),
        "default",
    )
    .await
    .unwrap()
    .expect("the vault's key row");
    let key_state = state::load_row_state(&db, "password_vault", "default")
        .await
        .unwrap()
        .expect("adopted at the join")
        .head_hlc;
    let pushed = own_seq(&device_b).await;
    let peers = state::load_local_state(&db).await.unwrap().0.peers;
    assert!(note_text(&device_b, "note-1").await.is_none());
    assert_eq!(settings::get(&db, "notes_capture_rules").await, None);

    assert!(
        restart(
            &mut device_b,
            sync_registry_of(&["pass", "notes", "capture"])
        )
        .await
    );
    cycle(&device_b).await;

    // What the cycles of Pass skipped is here now.
    assert_eq!(
        note_text(&device_b, "note-1").await,
        note_text(&space, "note-1").await
    );
    assert!(device_b
        .core
        .app_data_dir
        .join("notes/documents/attachments/note-1/scan.pdf")
        .is_file());
    assert_eq!(
        settings::get(&db, "notes_capture_rules").await.as_deref(),
        Some("[{\"host\":\"example.com\"}]")
    );
    let pinned: i64 = sqlx::query_scalar("SELECT pinned FROM notes WHERE id = 'note-1'")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(pinned, 1);
    // A put and the tombstone after it: the note is not fetched.
    assert_eq!(note_text(&device_b, "note-gone").await, None);
    assert!(!documents(&device_b).join("note-gone.md").exists());
    // The key row and its position are what the join left.
    assert_eq!(
        rows::read_row(
            &db,
            device_b.sync.registry().table_of("password_vault").unwrap(),
            "default"
        )
        .await
        .unwrap(),
        Some(key_row)
    );
    assert_eq!(
        state::load_row_state(&db, "password_vault", "default")
            .await
            .unwrap()
            .unwrap()
            .head_hlc,
        key_state
    );
    assert_eq!(reveal(&device_b, "pw-1").await, "hunter2");
    // Nothing went out, the positions in the logs did not go back, and the
    // vault is not read again.
    assert_eq!(own_seq(&device_b).await, pushed);
    let after = state::load_local_state(&db).await.unwrap().0.peers;
    for (peer, head) in &peers {
        assert!(after[peer].seq >= head.seq, "{peer}");
    }
    assert_eq!(state::reread(&db).await, None);
    let history: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM note_history")
        .fetch_one(&db)
        .await
        .unwrap();
    cycle(&device_b).await;
    assert_eq!(own_seq(&device_b).await, pushed);
    let history_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM note_history")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(history_after, history);

    for state in [space, device_b] {
        close(state).await;
    }
    let _ = std::fs::remove_dir_all(&vault);
}

/// The ops of what the earlier registry took do not come again when the
/// vault is read again: a note older than its head would be taken for a
/// fork, written over the newer one and put into the history.
#[tokio::test]
async fn a_reread_leaves_alone_what_the_earlier_registry_took() {
    let vault = crate::db::test_dir();
    let space = device(sync_registry(), &vault).await;
    create_vault(&space).await;
    write_note(&space, "note-1", "Deploy", "Version 1.").await;
    cycle(&space).await;
    let mut device_b = device(notes_like(), &vault).await;
    join_vault(&device_b).await;
    cycle(&device_b).await;
    for version in ["Version 2.", "Version 3."] {
        write(
            &documents(&space).join("note-1.md"),
            &note_file("note-1", "Deploy", version),
        );
        cycle(&space).await;
    }
    cycle(&device_b).await;
    let db = device_b.core.db.clone();
    let text = note_text(&device_b, "note-1").await;
    assert!(text.as_deref().unwrap_or_default().contains("Version 3."));
    let history: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM note_history")
        .fetch_one(&db)
        .await
        .unwrap();
    let pushed = own_seq(&device_b).await;

    assert!(restart(&mut device_b, sync_registry_of(&["notes", "pass"])).await);
    cycle(&device_b).await;
    assert_eq!(note_text(&device_b, "note-1").await, text);
    let history_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM note_history")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(history_after, history);
    assert_eq!(own_seq(&device_b).await, pushed);

    for state in [space, device_b] {
        close(state).await;
    }
    let _ = std::fs::remove_dir_all(&vault);
}

/// A device that joined without a key row and starts with another registry
/// before its first cycle: the key row of the vault comes by the pull, the
/// next cycle ends the wait the join left, and nothing is pushed.
#[tokio::test]
async fn a_key_row_the_join_held_back_is_settled_once_after_a_new_registry() {
    let vault = crate::db::test_dir();
    let space = device(sync_registry(), &vault).await;
    create_vault(&space).await;
    save_password(&space, "pw-1", "hunter2").await;
    cycle(&space).await;

    let mut device_b = TestStates::syncing(pass_like()).await;
    let db = device_b.core.db.clone();
    settings::set(&db, "sync_backend", "folder").await.unwrap();
    settings::set(&db, "sync_folder_path", &vault.to_string_lossy())
        .await
        .unwrap();
    state::check_registry(&db, device_b.sync.registry())
        .await
        .unwrap();
    join_vault(&device_b).await;
    assert!(settings::get(&db, config::JOIN_PENDING).await.is_some());
    assert!(restart(&mut device_b, sync_registry_of(&["pass", "notes"])).await);
    cycle(&device_b).await;
    assert_eq!(state::reread(&db).await, None);
    cycle(&device_b).await;

    assert_eq!(settings::get(&db, config::JOIN_PENDING).await, None);
    device_b.lock.open_default().await.unwrap();
    assert_eq!(reveal(&device_b, "pw-1").await, "hunter2");
    assert_eq!(own_seq(&device_b).await, 0);

    for state in [space, device_b] {
        close(state).await;
    }
    let _ = std::fs::remove_dir_all(&vault);
}

/// A product that gains a setting key with a value of its own pushes nothing
/// for it while the vault is read again: the vault's value, though older,
/// is what every device has, and the local one would win everywhere with
/// the newer clock. A key the vault has no value of goes out once the vault
/// was read.
#[tokio::test]
async fn a_gained_key_waits_for_the_vault_s_value() {
    let vault = crate::db::test_dir();
    let space = device(sync_registry(), &vault).await;
    create_vault(&space).await;
    let rules = "[{\"host\":\"example.com\"}]";
    settings::set(&space.core.db, "notes_capture_rules", rules)
        .await
        .unwrap();
    cycle(&space).await;

    let mut notes = device(notes_like(), &vault).await;
    join_vault(&notes).await;
    cycle(&notes).await;
    let db = notes.core.db.clone();
    settings::set(&db, "notes_capture_rules", "[{\"host\":\"local.example\"}]")
        .await
        .unwrap();
    settings::set(&db, "minimize_to_tray", "1").await.unwrap();
    let pushed = own_seq(&notes).await;

    assert!(restart(&mut notes, sync_registry_of(&["notes", "capture"])).await);
    cycle(&notes).await;
    assert_eq!(own_seq(&notes).await, pushed, "nothing went out");
    assert_eq!(
        settings::get(&db, "notes_capture_rules").await.as_deref(),
        Some(rules)
    );
    assert_eq!(state::reread(&db).await, None);
    cycle(&notes).await;
    assert_eq!(own_seq(&notes).await, pushed);
    cycle(&space).await;
    assert_eq!(
        settings::get(&space.core.db, "notes_capture_rules")
            .await
            .as_deref(),
        Some(rules)
    );

    // The key of the shell has no value in the vault but this device's.
    let mut registry = veydan_sync_host::Registry::new();
    for module in crate::modules::all() {
        if let (true, Some(sync)) = (["notes", "capture"].contains(&module.id), module.sync) {
            registry.add(module.id, sync);
        }
    }
    registry.add("shell", |registry| registry.setting("minimize_to_tray"));
    let registry = registry.finish(&SYNC_PLAN).unwrap();
    assert!(restart(&mut notes, registry).await);
    cycle(&notes).await;
    assert_eq!(own_seq(&notes).await, pushed);
    cycle(&notes).await;
    assert!(own_seq(&notes).await > pushed);
    cycle(&space).await;
    assert_eq!(
        settings::get(&space.core.db, "minimize_to_tray")
            .await
            .as_deref(),
        Some("1")
    );

    for state in [space, notes] {
        close(state).await;
    }
    let _ = std::fs::remove_dir_all(&vault);
}

/// A cycle that ends with the warning of the log of `broken`, which cannot
/// be read; returns how many ops it applied.
async fn cycle_with_a_broken_log(state: &TestStates, broken: &str) -> String {
    let warnings = veydan_sync_host::cycle(state.host(), &state.core, &state.sync)
        .await
        .unwrap();
    assert!(!warnings.is_empty());
    assert!(warnings.iter().all(|w| w.contains(broken)), "{warnings:?}");
    settings::get(&state.core.db, "sync_last_applied")
        .await
        .unwrap()
}

/// A device whose log cannot be read does not make the others' logs come
/// again on every cycle: a log read in full for a new registry is not read
/// in full again, while the broken one is tried each cycle.
#[tokio::test]
async fn a_log_read_again_in_full_is_not_read_again() {
    let vault = crate::db::test_dir();
    let space = device(sync_registry(), &vault).await;
    create_vault(&space).await;
    save_password(&space, "pw-1", "hunter2").await;
    cycle(&space).await;
    let broken = device(sync_registry(), &vault).await;
    join_vault(&broken).await;
    save_password(&broken, "pw-2", "swordfish").await;
    cycle(&broken).await;
    let broken_id = device_id(&broken).await;
    // Its chunks no longer open.
    for chunk in std::fs::read_dir(vault.join("devices").join(&broken_id).join("log"))
        .unwrap()
        .flatten()
    {
        let mut bytes = std::fs::read(chunk.path()).unwrap();
        *bytes.last_mut().unwrap() ^= 0xff;
        std::fs::write(chunk.path(), bytes).unwrap();
    }

    let mut notes = device(notes_like(), &vault).await;
    join_vault(&notes).await;
    let db = notes.core.db.clone();
    cycle_with_a_broken_log(&notes, &broken_id).await;

    assert!(restart(&mut notes, sync_registry_of(&["notes", "pass"])).await);
    let applied = cycle_with_a_broken_log(&notes, &broken_id).await;
    assert_ne!(applied, "0", "what the earlier registry skipped");
    assert_eq!(reveal(&notes, "pw-1").await, "hunter2");
    assert_eq!(state::reread_peers(&db).await, [device_id(&space).await]);
    assert!(state::reread(&db).await.is_some());

    assert_eq!(cycle_with_a_broken_log(&notes, &broken_id).await, "0");
    assert_eq!(cycle_with_a_broken_log(&notes, &broken_id).await, "0");
    assert!(
        state::reread(&db).await.is_some(),
        "the broken log is still owed"
    );

    for state in [space, broken, notes] {
        close(state).await;
    }
    let _ = std::fs::remove_dir_all(&vault);
}

// ── What the registry never changes (spec 9.5) ───────────────────────────────

/// Item 4: blobs of format v1 are kept alive by `note`, `note_attachment`
/// and `profile_snapshot` alone; every other type keeps none, whatever its
/// payload names.
#[tokio::test]
async fn only_the_types_of_4_0_7_keep_blobs_of_format_v1() {
    let vault = crate::db::test_dir();
    let space = device(sync_registry(), &vault).await;
    create_vault(&space).await;
    let engine = open_engine(&space.core).await.unwrap();
    let file = engine.put_blob(b"a file").await.unwrap();
    let manifest = format!(r#"{{"files":[{{"path":"a","blob":"{file}","size":6,"hash":"h"}}]}}"#);
    let manifest_blob = engine.put_blob(manifest.as_bytes()).await.unwrap();
    let registry = space.sync.registry();
    let mut keeping = BTreeSet::new();
    for entity in registry.entities() {
        let op = peer_op(
            entity,
            "x-1",
            1_000,
            json!({ "blob": file, "parents": [file], "manifest_blob": manifest_blob, "lf_refs": [] }),
        );
        if let Some(handler) = registry.handler_of(entity) {
            if !handler.blob_refs(&engine, &op).await.unwrap().is_empty() {
                keeping.insert(entity);
            }
        }
    }
    assert_eq!(
        keeping.into_iter().collect::<Vec<_>>(),
        ["note", "note_attachment", "profile_snapshot"]
    );

    close(space).await;
    let _ = std::fs::remove_dir_all(&vault);
}

/// Item 7: the manifest a vault is created with says version 1, and no other
/// version opens.
#[tokio::test]
async fn the_vault_manifest_stays_at_version_1() {
    let vault = crate::db::test_dir();
    let space = device(sync_registry(), &vault).await;
    create_vault(&space).await;
    let path = vault.join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(manifest["version"], json!(1));

    manifest["version"] = json!(2);
    std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let storage = build_storage(&load_config(&space.core.db).await).unwrap();
    let refused = Engine::open(storage, PASSPHRASE, "other")
        .await
        .err()
        .unwrap();
    assert!(
        refused
            .to_string()
            .contains("unsupported manifest version 2"),
        "{refused}"
    );

    close(space).await;
    let _ = std::fs::remove_dir_all(&vault);
}
