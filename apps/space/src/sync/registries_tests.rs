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
    state.sync.mark_ready();
    state.lock.open_default().await.unwrap();
    state
}

/// The app starts again with what `registry` syncs.
async fn restart(state: &mut TestStates, registry: Registry) -> bool {
    state.sync = SyncManager::new(registry);
    let changed = state::check_registry(&state.core.db, state.sync.registry())
        .await
        .unwrap();
    state.sync.mark_ready();
    changed
}

pub(super) async fn create_vault(state: &TestStates) {
    let db = &state.core.db;
    let storage = build_storage(&load_config(db).await).unwrap();
    let device = config::device_id(db).await.unwrap();
    let (engine, vmk) = Engine::create(storage, PASSPHRASE, &device).await.unwrap();
    join::bind(db, state.sync.registry(), engine.vault_id(), &vmk)
        .await
        .unwrap();
}

/// `sync_join_vault` without the app.
pub(super) async fn join_vault(state: &TestStates) {
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

// ── Two marks of the garbage collection ──────────────────────────────────────

/// A snapshot op of `id` naming `manifest_blob`, pushed by `engine`.
async fn publish_snapshot(engine: &Engine, db: &sqlx::SqlitePool, id: &str, manifest_blob: &str) {
    let (mut local, last) = state::load_local_state(db).await.unwrap();
    let mut clock = HlcClock::new(engine.device_id().to_string(), last.as_ref());
    let op = Op {
        entity_type: "profile_snapshot".into(),
        entity_id: id.into(),
        hlc: clock.now(),
        deleted: false,
        payload: json!({ "manifest_blob": manifest_blob, "taken_at": T1, "files": 1, "bytes": 3 }),
    };
    engine.push(&mut local, vec![op]).await.unwrap();
    state::save_own_state(db, &local, &clock.last())
        .await
        .unwrap();
}

type DeleteHook = Box<dyn FnOnce(String) -> veydan_core::BoxFuture<'static, ()> + Send>;

/// A storage whose first `delete` runs a hook with the key just before.
struct OnDelete {
    inner: Box<dyn veydan_sync::Storage>,
    hook: tokio::sync::Mutex<Option<DeleteHook>>,
}

#[async_trait::async_trait]
impl veydan_sync::Storage for OnDelete {
    async fn list(&self, prefix: &str) -> veydan_sync::Result<Vec<String>> {
        self.inner.list(prefix).await
    }
    async fn get(&self, key: &str) -> veydan_sync::Result<Option<Vec<u8>>> {
        self.inner.get(key).await
    }
    async fn put(&self, key: &str, data: &[u8]) -> veydan_sync::Result<()> {
        self.inner.put(key, data).await
    }
    async fn delete(&self, key: &str) -> veydan_sync::Result<()> {
        let hook = self.hook.lock().await.take();
        if let Some(hook) = hook {
            hook(key.to_string()).await;
        }
        self.inner.delete(key).await
    }
    async fn exists(&self, key: &str) -> veydan_sync::Result<bool> {
        self.inner.exists(key).await
    }
    async fn put_if_absent(&self, key: &str, data: &[u8]) -> veydan_sync::Result<bool> {
        self.inner.put_if_absent(key, data).await
    }
}

/// The engine of `state` over `storage` in place of its own.
async fn engine_over(state: &TestStates, storage: Box<dyn veydan_sync::Storage>) -> Engine {
    let db = &state.core.db;
    let binding = config::load_binding(db).await.unwrap();
    let vmk = veydan_sync::Vmk::from_base64(&binding.vmk_b64).unwrap();
    let device = config::device_id(db).await.unwrap();
    Engine::with_key(storage, &vmk, &binding.vault_id, &device)
}

/// Two snapshots' worth of blobs, named by nothing, are candidates of B
/// since a day. B's pass deletes the first of them; just then A takes up the
/// other pair (the same content, the same names, nothing uploaded) and
/// publishes a snapshot naming it. The read of the logs before B's next
/// delete finds that snapshot: the pair it names stays.
#[tokio::test]
async fn a_snapshot_published_while_the_garbage_is_collected_keeps_its_blobs() {
    let vault = crate::db::test_dir();
    let b = device(sync_registry(), &vault).await;
    create_vault(&b).await;
    let a = device(sync_registry(), &vault).await;
    join_vault(&a).await;

    let a_engine = open_engine(&a.core).await.unwrap();
    let mut pairs = Vec::new();
    for content in ["one", "two"] {
        let file = a_engine.put_blob(content.as_bytes()).await.unwrap();
        let manifest = format!(
            r#"{{"files":[{{"path":"prefs.js","blob":"{file}","size":3,"hash":"{content}"}}]}}"#
        );
        let manifest = a_engine.put_blob(manifest.as_bytes()).await.unwrap();
        pairs.push([manifest, file]);
    }
    let db = &b.core.db;
    let registry = b.sync.registry();
    gc::run(&open_engine(&b.core).await.unwrap(), db, registry)
        .await
        .unwrap();
    let day_ago = chrono::Utc::now().timestamp_millis() - 25 * 60 * 60 * 1000;
    sqlx::query("UPDATE sync_gc_candidates SET first_seen = ?")
        .bind(day_ago)
        .execute(db)
        .await
        .unwrap();

    let first: std::sync::Arc<std::sync::Mutex<Option<String>>> = Default::default();
    let hook: DeleteHook = {
        let (first, pairs, a_db) = (first.clone(), pairs.clone(), a.core.db.clone());
        Box::new(move |key: String| {
            Box::pin(async move {
                let name = key.strip_prefix("blobs/").unwrap().to_string();
                let other = pairs.iter().find(|p| !p.contains(&name)).unwrap();
                publish_snapshot(&a_engine, &a_db, "pr-x", &other[0]).await;
                *first.lock().unwrap() = Some(name);
            })
        })
    };
    let storage = OnDelete {
        inner: build_storage(&load_config(db).await).unwrap(),
        hook: tokio::sync::Mutex::new(Some(hook)),
    };
    let engine = engine_over(&b, Box::new(storage)).await;
    let pace = gc::Pace {
        every: 1,
        after: std::time::Duration::from_secs(3600),
    };
    let outcome = gc::run_paced(&engine, db, registry, pace).await.unwrap();

    let first = first.lock().unwrap().clone().expect("a delete");
    let gone = pairs.iter().find(|p| p.contains(&first)).unwrap();
    let kept = pairs.iter().find(|p| !p.contains(&first)).unwrap();
    let left = blobs(&b).await;
    assert_eq!(outcome.removed, 2);
    for blob in gone {
        assert!(!left.contains(blob), "{blob}");
    }
    for blob in kept {
        assert!(left.contains(blob), "{blob}");
    }
    let candidates: Vec<String> = sqlx::query_scalar("SELECT blob FROM sync_gc_candidates")
        .fetch_all(db)
        .await
        .unwrap();
    assert!(candidates.is_empty(), "{candidates:?}");

    for state in [a, b] {
        close(state).await;
    }
    let _ = std::fs::remove_dir_all(&vault);
}

/// The blob of a profile file the snapshot of A names is gone: another
/// device's GC took it between its mark and the reference. A's next pass
/// finds the snapshot naming what is not there, the next cycle sends the
/// profile again and uploads the blob, and B, joining later, gets the file.
#[tokio::test]
async fn a_blob_a_foreign_gc_took_under_a_snapshot_is_uploaded_again() {
    let vault = crate::db::test_dir();
    let a = device(sync_registry(), &vault).await;
    create_vault(&a).await;
    space_writes_everything(&a).await;
    cycle(&a).await;
    let db = &a.core.db;
    let (manifest, head): (String, String) = sqlx::query_as(
        "SELECT manifest_json, head_hlc FROM sync_profile_files_state WHERE profile_id = 'pr-1'",
    )
    .fetch_one(db)
    .await
    .unwrap();
    let manifest: Value = serde_json::from_str(&manifest).unwrap();
    let prefs = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == "prefs.js")
        .unwrap()["blob"]
        .as_str()
        .unwrap()
        .to_string();
    let engine = open_engine(&a.core).await.unwrap();
    engine.delete_blob(&prefs).await.unwrap();

    gc::run(&engine, db, a.sync.registry()).await.unwrap();
    let missing = gc::missing_refs(db).await;
    assert_eq!(missing.len(), 1, "{missing:?}");
    assert_eq!(
        (
            missing[0].entity_type.as_str(),
            missing[0].entity_id.as_str()
        ),
        ("profile_snapshot", "pr-1")
    );
    assert_eq!(missing[0].hlc.encode(), head);

    cycle(&a).await;
    assert!(engine.blob_exists(&prefs).await.unwrap());
    let healed: String = sqlx::query_scalar(
        "SELECT head_hlc FROM sync_profile_files_state WHERE profile_id = 'pr-1'",
    )
    .fetch_one(db)
    .await
    .unwrap();
    assert!(
        Hlc::decode(&healed) > Hlc::decode(&head),
        "{healed} / {head}"
    );

    let b = device(sync_registry(), &vault).await;
    join_vault(&b).await;
    cycle(&b).await;
    let path: String = sqlx::query_scalar("SELECT profile_path FROM profiles WHERE id = 'pr-1'")
        .fetch_one(&b.core.db)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(Path::new(&path).join("firefox-profile/prefs.js")).unwrap(),
        b"user_pref(\"a\", 1);"
    );

    for state in [a, b] {
        close(state).await;
    }
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
    device_b.sync.mark_ready();
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

/// A device whose log cannot be read holds back neither the others' logs nor
/// the second read of the vault for a new registry: its log is read as far
/// as any read of it gets and counts as read, and the rows of the new types
/// go out (audit A2-11).
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
    // The first cycle after the join reads the vault, the second pushes.
    cycle(&broken).await;
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

    // The join takes that log up to its broken file and says so. The log
    // holds the wait of the join as any log a pull stops in (a copy may be
    // still on its way), until the wait of five cycles ends with a warning.
    let mut notes = device(notes_like(), &vault).await;
    join_vault(&notes).await;
    let db = notes.core.db.clone();
    let warning = settings::get(&db, "sync_last_warning")
        .await
        .unwrap_or_default();
    assert!(warning.contains(&broken_id), "{warning}");
    for _ in 0..4 {
        cycle_with_a_broken_log(&notes, &broken_id).await;
        assert!(join::fresh_binding(&db).await.is_some());
    }
    cycle_with_a_broken_log(&notes, &broken_id).await;
    assert!(join::fresh_binding(&db).await.is_none());

    assert!(restart(&mut notes, sync_registry_of(&["notes", "pass"])).await);
    let applied = cycle_with_a_broken_log(&notes, &broken_id).await;
    assert_ne!(applied, "0", "what the earlier registry skipped");
    assert_eq!(reveal(&notes, "pw-1").await, "hunter2");
    // The broken log was read as far as any read of it gets: it does not
    // hold the vault's second read open (audit A2-11).
    assert_eq!(state::reread(&db).await, None);
    assert!(state::reread_peers(&db).await.is_empty());
    assert_eq!(cycle_with_a_broken_log(&notes, &broken_id).await, "0");

    // What the new registry takes goes out from here on.
    let pushed = own_seq(&notes).await;
    save_password(&notes, "pw-3", "tiger").await;
    cycle_with_a_broken_log(&notes, &broken_id).await;
    assert!(own_seq(&notes).await > pushed);
    cycle_with_a_broken_log(&space, &broken_id).await;
    assert_eq!(reveal(&space, "pw-3").await, "tiger");

    for state in [space, broken, notes] {
        close(state).await;
    }
    let _ = std::fs::remove_dir_all(&vault);
}

/// A log whose first chunk broke after this device read past it: read again
/// for a new registry it stops below where the plain read stands. It is
/// tried again for five cycles, then counts as read with a warning — no
/// read gets further — and the vault's second read ends (audit A2-11).
#[tokio::test]
async fn a_log_broken_below_the_plain_read_counts_as_read_after_five_tries() {
    let vault = crate::db::test_dir();
    let space = device(sync_registry(), &vault).await;
    create_vault(&space).await;
    save_password(&space, "pw-1", "hunter2").await;
    cycle(&space).await;
    let broken = device(sync_registry(), &vault).await;
    join_vault(&broken).await;
    save_password(&broken, "pw-2", "swordfish").await;
    cycle(&broken).await;
    cycle(&broken).await;
    let broken_id = device_id(&broken).await;

    let mut notes = device(notes_like(), &vault).await;
    join_vault(&notes).await;
    cycle(&notes).await;
    let db = notes.core.db.clone();
    assert!(join::fresh_binding(&db).await.is_none());

    // Its first chunk is no envelope any more; the plain read is past it.
    let first = vault
        .join("devices")
        .join(&broken_id)
        .join("log")
        .join("00000001.bin");
    std::fs::write(&first, b"").unwrap();
    assert!(restart(&mut notes, sync_registry_of(&["notes", "pass"])).await);
    for _ in 0..4 {
        cycle_with_a_broken_log(&notes, &broken_id).await;
        assert!(state::reread(&db).await.is_some());
    }
    let warnings = veydan_sync_host::cycle(notes.host(), &notes.core, &notes.sync)
        .await
        .unwrap();
    assert!(
        warnings
            .iter()
            .any(|w| w.contains(&broken_id) && w.contains("(5 times)")),
        "{warnings:?}"
    );
    assert_eq!(state::reread(&db).await, None);
    assert_eq!(reveal(&notes, "pw-1").await, "hunter2");

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

/// A tombstone survives the compaction of the log of the device that
/// deleted the row, however long ago it was made: a device that joins later
/// reads the insert of the first device and must still see it deleted.
#[tokio::test]
async fn a_deleted_password_stays_deleted_after_the_log_is_compacted() {
    let vault = crate::db::test_dir();
    let a = device(sync_registry(), &vault).await;
    create_vault(&a).await;
    save_password(&a, "pw-1", "hunter2").await;
    cycle(&a).await;

    let b = device(sync_registry(), &vault).await;
    join_vault(&b).await;
    cycle(&b).await;
    assert_eq!(reveal(&b, "pw-1").await, "hunter2");
    sql(&b, "DELETE FROM passwords WHERE id = 'pw-1'").await;
    cycle(&b).await;

    // The snapshot is folded long after the deletion; nothing is dropped.
    let (local, _) = state::load_local_state(&b.core.db).await.unwrap();
    open_engine(&b.core)
        .await
        .unwrap()
        .compact(&local)
        .await
        .unwrap();

    let c = device(sync_registry(), &vault).await;
    join_vault(&c).await;
    cycle(&c).await;
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM passwords WHERE id = 'pw-1'")
        .fetch_one(&c.core.db)
        .await
        .unwrap();
    assert_eq!(left, 0, "the password came back");

    for state in [a, b, c] {
        close(state).await;
    }
    let _ = std::fs::remove_dir_all(&vault);
}

/// A cycle before the checks of the start went through refuses and
/// publishes nothing; it runs once they have.
#[tokio::test]
async fn a_cycle_waits_for_the_checks_of_the_start() {
    let vault = crate::db::test_dir();
    let mut space = device(sync_registry(), &vault).await;
    create_vault(&space).await;
    save_password(&space, "pw-1", "hunter2").await;

    space.sync = SyncManager::new(sync_registry());
    assert!(!space.sync.is_ready());
    let refused = veydan_sync_host::cycle(space.host(), &space.core, &space.sync)
        .await
        .expect_err("a cycle before the checks must refuse");
    assert!(refused.to_string().contains("not ready"), "{refused}");
    assert_eq!(own_seq(&space).await, 0, "nothing was published");

    veydan_sync_host::prepare(&space.core, &space.sync)
        .await
        .unwrap();
    assert!(space.sync.is_ready());
    cycle(&space).await;
    assert!(own_seq(&space).await > 0);

    close(space).await;
    let _ = std::fs::remove_dir_all(&vault);
}

// ── Ids of profiles and their folders ────────────────────────────────────────

/// Audit SYNCROWS-13: the folder of a profile is its id under `profiles/`,
/// and its tombstone removes that folder. A row or a tombstone with the id
/// `..` is never applied, so the data directory stays; a row an older build
/// left with the data directory as its path loses its row to a tombstone,
/// never the directory; an id that is not a plain name (`a.b`, `C:x`, the
/// drive on Windows) gets a folder by its hash inside `profiles/`.
#[tokio::test]
async fn no_id_of_a_profile_reaches_a_folder_above_profiles() {
    let state = TestStates::new().await;
    let data = state.core.app_data_dir.clone();
    let tables = state.sync.registry().step_rows(SYNC_PLAN.apply, "app rows");
    let profile = |id: &str, wall_ms: u64, put: bool| Op {
        entity_type: "profile".into(),
        entity_id: id.into(),
        hlc: Hlc {
            wall_ms,
            counter: 0,
            device_id: "dev-a".into(),
        },
        deleted: !put,
        payload: if put {
            json!({
                "name": id, "browser_type": "camoufox", "fingerprint_preset": "windows",
                "created_at": T1,
            })
        } else {
            json!({})
        },
    };
    let apply = |ops: Vec<Op>| {
        let (state, tables) = (&state, &tables);
        async move {
            rows::apply_ops(&state.host(), &state.core, tables, &ops)
                .await
                .unwrap();
        }
    };
    let path_of = |id: &'static str| {
        let db = state.core.db.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT profile_path FROM profiles WHERE id = ?")
                .bind(id)
                .fetch_optional(&db)
                .await
                .unwrap()
        }
    };
    let alive = data.join("alive.txt");
    write(&alive, b"the data directory");

    apply(vec![profile("..", 1000, true), profile("..", 2000, false)]).await;
    assert!(path_of("..").await.is_none());
    assert!(alive.is_file());
    assert!(data.join("profiles").is_dir());

    for (id, path) in [
        ("pr-old", data.join("profiles").join("..")),
        ("pr-root", data.join("profiles")),
    ] {
        sql(&state, &format!(
            "INSERT INTO profiles (id, name, status, profile_path, browser_type, fingerprint_preset, created_at, updated_at)
             VALUES ('{id}', 'Old', 'stopped', '{}', 'camoufox', 'windows', '{T1}', '{T1}')",
            path.display()
        ))
        .await;
    }
    apply(vec![
        profile("pr-old", 3000, false),
        profile("pr-root", 3000, false),
    ])
    .await;
    assert!(path_of("pr-old").await.is_none());
    assert!(path_of("pr-root").await.is_none());
    assert!(alive.is_file());
    assert!(data.join("profiles").is_dir());

    apply(vec![
        profile("pr-1", 4000, true),
        profile("a.b", 4000, true),
        profile("C:x", 4000, true),
    ])
    .await;
    assert_eq!(
        path_of("pr-1").await.map(PathBuf::from),
        Some(data.join("profiles").join("pr-1"))
    );
    let mut hashed = Vec::new();
    for id in ["a.b", "C:x"] {
        let path = PathBuf::from(path_of(id).await.expect(id));
        assert_eq!(path.parent(), Some(data.join("profiles").as_path()), "{id}");
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("h-") && name.len() == 18, "{name}");
        assert!(path.is_dir(), "{id}");
        hashed.push(path);
    }
    assert_ne!(hashed[0], hashed[1]);
    apply(vec![
        profile("a.b", 5000, false),
        profile("C:x", 5000, false),
    ])
    .await;
    for path in &hashed {
        assert!(!path.exists(), "{}", path.display());
    }
    assert!(data.join("profiles").join("pr-1").is_dir());
    assert!(alive.is_file());

    close(state).await;
}

async fn password_title(state: &TestStates, id: &str) -> String {
    sqlx::query_scalar("SELECT title FROM passwords WHERE id = ?")
        .bind(id)
        .fetch_one(&state.core.db)
        .await
        .unwrap()
}

/// An edit made after the collect of a cycle, before its apply brings the
/// newer op of another device for the same row, is not written over: the
/// next cycle publishes it, later than the other device's, and both devices
/// end with it.
#[tokio::test]
async fn an_edit_between_the_collect_and_the_apply_is_not_lost() {
    let vault = crate::db::test_dir();
    let a = device(sync_registry(), &vault).await;
    create_vault(&a).await;
    save_password(&a, "pw-1", "hunter2").await;
    cycle(&a).await;
    let b = device(sync_registry(), &vault).await;
    join_vault(&b).await;
    cycle(&b).await;
    assert_eq!(password_title(&b, "pw-1").await, "pw-1");

    sql(
        &a,
        "UPDATE passwords SET title = 'from a' WHERE id = 'pw-1'",
    )
    .await;
    cycle(&a).await;
    // B collected already; the user edits; the apply of the cycle comes.
    sql(
        &b,
        "UPDATE passwords SET title = 'from b' WHERE id = 'pw-1'",
    )
    .await;
    let from_a = ops_of(&b, &device_id(&a).await).await;
    assert!(from_a.iter().any(|op| op.entity_id == "pw-1"));
    apply_rows(&b, &from_a).await;
    assert_eq!(password_title(&b, "pw-1").await, "from b");

    // The edit of B is later than A's op on any clock.
    std::thread::sleep(std::time::Duration::from_millis(5));
    cycle(&b).await;
    cycle(&a).await;
    assert_eq!(password_title(&a, "pw-1").await, "from b");
    assert_eq!(password_title(&b, "pw-1").await, "from b");

    for state in [a, b] {
        close(state).await;
    }
    let _ = std::fs::remove_dir_all(&vault);
}

// ── The own log (spec 9.3, "the own log: pending and reconciliation") ────────

/// A storage whose answer to the next conditional write is lost: the write
/// lands, the device hears an error.
struct LostAnswer {
    inner: veydan_sync::LocalDir,
    lose: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl veydan_sync::Storage for LostAnswer {
    async fn list(&self, prefix: &str) -> veydan_sync::Result<Vec<String>> {
        self.inner.list(prefix).await
    }
    async fn get(&self, key: &str) -> veydan_sync::Result<Option<Vec<u8>>> {
        self.inner.get(key).await
    }
    async fn put(&self, key: &str, data: &[u8]) -> veydan_sync::Result<()> {
        self.inner.put(key, data).await
    }
    async fn delete(&self, key: &str) -> veydan_sync::Result<()> {
        self.inner.delete(key).await
    }
    async fn put_if_absent(&self, key: &str, data: &[u8]) -> veydan_sync::Result<bool> {
        let written = self.inner.put_if_absent(key, data).await?;
        if self.lose.swap(false, std::sync::atomic::Ordering::SeqCst) {
            return Err(veydan_sync::SyncError::Storage("timeout".into()));
        }
        Ok(written)
    }
}

/// An engine over the vault in `vault` whose answer to the next chunk
/// write is lost.
async fn lossy_engine(state: &TestStates, vault: &Path, id: &str) -> Engine {
    let binding = config::load_binding(&state.core.db).await.unwrap();
    Engine::with_key(
        Box::new(LostAnswer {
            inner: veydan_sync::LocalDir::new(vault),
            lose: true.into(),
        }),
        &veydan_sync::Vmk::from_base64(&binding.vmk_b64).unwrap(),
        &binding.vault_id,
        id,
    )
}

/// The position of the own log as the data file holds it.
async fn own_position(state: &TestStates) -> (Option<String>, Option<String>) {
    let db = &state.core.db;
    (
        settings::get(db, "sync_own_seq").await,
        settings::get(db, "sync_own_head").await,
    )
}

async fn set_own_position(state: &TestStates, (seq, head): &(Option<String>, Option<String>)) {
    let db = &state.core.db;
    settings::set(db, "sync_own_seq", seq.as_deref().unwrap())
        .await
        .unwrap();
    settings::set(db, "sync_own_head", head.as_deref().unwrap())
        .await
        .unwrap();
}

async fn retitle(state: &TestStates, id: &str, title: &str) {
    sql(
        state,
        &format!("UPDATE passwords SET title = '{title}' WHERE id = '{id}'"),
    )
    .await;
}

/// The chunk of a push whose answer was lost is in the vault; the next
/// cycle replays the kept chunk and takes it as its own instead of colliding
/// with it, so the device keeps its id and the other device gets the edit
/// (audit A2-07, SYNCNOTES-01).
#[tokio::test]
async fn a_push_whose_answer_was_lost_does_not_rotate_the_device() {
    let vault = crate::db::test_dir();
    let a = device(sync_registry(), &vault).await;
    create_vault(&a).await;
    save_password(&a, "pw-1", "hunter2").await;
    cycle(&a).await;
    let b = device(sync_registry(), &vault).await;
    join_vault(&b).await;
    cycle(&b).await;
    let id = device_id(&a).await;
    let seq = own_seq(&a).await;

    retitle(&a, "pw-1", "lost answer").await;
    let db = &a.core.db;
    let lossy = lossy_engine(&a, &vault, &id).await;
    let (mut local, last) = state::load_local_state(db).await.unwrap();
    let mut clock = HlcClock::new(id.clone(), last.as_ref());
    let tables: Vec<_> = a.sync.registry().tables().iter().collect();
    let changes = rows::collect_local_changes(&a.core, &mut clock, &tables, None)
        .await
        .unwrap();
    assert!(!changes.ops.is_empty());
    let failed = veydan_sync_host::push_own(
        &lossy,
        db,
        a.sync.registry(),
        &mut local,
        &clock,
        changes.ops,
    )
    .await
    .unwrap_err();
    assert!(!failed.rotated, "{}", failed.error);
    assert_eq!(own_seq(&a).await, seq);
    assert!(state::load_own_pending(db).await.is_some());

    cycle(&a).await;
    assert_eq!(device_id(&a).await, id, "no rotation");
    assert!(state::load_own_pending(db).await.is_none());
    // The replayed chunk, then the rows collected again.
    assert_eq!(own_seq(&a).await, seq + 2);
    assert!(config::former_device_ids(db).await.is_empty());
    cycle(&b).await;
    assert_eq!(password_title(&b, "pw-1").await, "lost answer");
    let devices = std::fs::read_dir(vault.join("devices")).unwrap().count();
    assert_eq!(devices, 2);

    for state in [a, b] {
        close(state).await;
    }
    let _ = std::fs::remove_dir_all(&vault);
}

/// A profile row of Space at `pr-<n>`.
async fn add_profile(state: &TestStates, id: &str) {
    let path = state.core.app_data_dir.join("profiles").join(id);
    sql(
        state,
        &format!(
            "INSERT INTO profiles (id, name, status, profile_path, browser_type, fingerprint_preset,
               created_at, updated_at)
             VALUES ('{id}', 'Brand {id}', 'stopped', '{}', 'camoufox', 'windows', '{T1}', '{T1}')",
            path.display()
        ),
    )
    .await;
}

/// The device another device's lease of `id` names, as this one sees it.
async fn lease_of(state: &TestStates, id: &str) -> Option<String> {
    super::profile_files::foreign_lease(&state.core, id)
        .await
        .unwrap()
        .map(|(device, _)| device)
}

/// A cycle that rotates the device id; its error.
async fn rotating_cycle(state: &TestStates) -> String {
    let before = device_id(state).await;
    let err = veydan_sync_host::cycle(state.host(), &state.core, &state.sync)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("device id rotated"), "{err}");
    assert_ne!(device_id(state).await, before);
    err
}

/// A data file put back from a copy (a restored folder, a cloned machine)
/// knows less of its log than the vault holds, the chunks past it folded
/// into the snapshot: a chunk pushed from there would land in a range
/// nobody reads. The cycle rotates the device id instead, and the next one
/// pushes under the new id. The lease of a profile that runs here goes with
/// the device; the lease of one that does not was the copy's and is let go
/// here, and the old log read back never takes a profile again (audit
/// KA2-02, A2-07, A2-19).
#[tokio::test]
async fn a_data_file_behind_its_log_rotates_the_device_and_moves_only_running_leases() {
    let vault = crate::db::test_dir();
    let a = device(sync_registry(), &vault).await;
    create_vault(&a).await;
    save_password(&a, "pw-1", "hunter2").await;
    for id in ["pr-1", "pr-2"] {
        add_profile(&a, id).await;
        super::profile_files::acquire_lease(&a.core, id)
            .await
            .unwrap();
    }
    cycle(&a).await;
    let b = device(sync_registry(), &vault).await;
    join_vault(&b).await;
    cycle(&b).await;
    let old = device_id(&a).await;
    assert_eq!(lease_of(&b, "pr-1").await, Some(old.clone()));

    let copy = own_position(&a).await;
    for title in ["one", "two", "three"] {
        retitle(&a, "pw-1", title).await;
        cycle(&a).await;
    }
    let (local, _) = state::load_local_state(&a.core.db).await.unwrap();
    open_engine(&a.core)
        .await
        .unwrap()
        .compact(&local)
        .await
        .unwrap();
    set_own_position(&a, &copy).await;
    retitle(&a, "pw-1", "after the copy").await;
    sql(
        &a,
        "UPDATE profiles SET status = 'running' WHERE id = 'pr-1'",
    )
    .await;

    rotating_cycle(&a).await;
    let new = device_id(&a).await;
    assert_eq!(config::former_device_ids(&a.core.db).await, [old.as_str()]);
    assert_eq!(own_seq(&a).await, 0);

    cycle(&a).await;
    cycle(&b).await;
    assert_eq!(password_title(&b, "pw-1").await, "after the copy");
    assert_eq!(lease_of(&b, "pr-1").await, Some(new.clone()));
    // The vault still says the old id holds pr-2: here it is free.
    assert_eq!(lease_of(&b, "pr-2").await, Some(old.clone()));
    // Its old log, read back as another device's, takes nothing from it.
    cycle(&a).await;
    assert_eq!(lease_of(&a, "pr-1").await, None);
    assert_eq!(lease_of(&a, "pr-2").await, None);
    assert_eq!(password_title(&a, "pw-1").await, "after the copy");
    cycle(&b).await;
    assert_eq!(lease_of(&b, "pr-1").await, Some(new));

    for state in [a, b] {
        close(state).await;
    }
    let _ = std::fs::remove_dir_all(&vault);
}

/// A chunk the vault lost (a folder put back, a file deleted) leaves the
/// device ahead of its log, and another device may have read it already.
/// The next cycle — with nothing new to push — rotates the device id
/// rather than writing a different chunk under the same seq, which would
/// break the chain for that reader, and the cycle after it publishes again
/// what the lost chunk carried, the rows and a note alike (audit A2-12,
/// KA2-11).
#[tokio::test]
async fn a_chunk_the_vault_lost_is_published_again_under_a_new_id() {
    let vault = crate::db::test_dir();
    let a = device(sync_registry(), &vault).await;
    create_vault(&a).await;
    save_password(&a, "pw-1", "hunter2").await;
    save_password(&a, "pw-2", "swordfish").await;
    cycle(&a).await;
    let b = device(sync_registry(), &vault).await;
    join_vault(&b).await;
    cycle(&b).await;
    let c = device(sync_registry(), &vault).await;
    join_vault(&c).await;
    cycle(&c).await;

    retitle(&a, "pw-1", "lost").await;
    write_note(&a, "note-1", "Lost", "In the lost chunk.").await;
    cycle(&a).await;
    let old = device_id(&a).await;
    let seq = own_seq(&a).await;
    // B reads the chunk before the vault loses it; C does not.
    cycle(&b).await;
    assert_eq!(password_title(&b, "pw-1").await, "lost");
    std::fs::remove_file(
        vault
            .join("devices")
            .join(&old)
            .join("log")
            .join(format!("{seq:08}.bin")),
    )
    .unwrap();

    rotating_cycle(&a).await;
    let new = device_id(&a).await;
    // The old log is read from what the vault still holds of it.
    let peer: (i64,) = sqlx::query_as("SELECT seq FROM sync_peers WHERE device_id = ?")
        .bind(&old)
        .fetch_one(&a.core.db)
        .await
        .unwrap();
    assert_eq!(peer.0 as u64, seq - 1);
    cycle(&a).await;
    let republished: BTreeSet<String> = ops_of(&a, &new)
        .await
        .into_iter()
        .map(|op| format!("{}:{}", op.entity_type, op.entity_id))
        .collect();
    assert!(republished.contains("password:pw-1"), "{republished:?}");
    assert!(republished.contains("note:note-1"), "{republished:?}");
    assert!(!republished.contains("password:pw-2"), "{republished:?}");
    for peer in [&b, &c] {
        cycle(peer).await;
        assert_eq!(password_title(peer, "pw-1").await, "lost");
        assert_eq!(password_title(peer, "pw-2").await, "pw-2");
        assert!(note_text(peer, "note-1")
            .await
            .unwrap()
            .contains("In the lost chunk."));
    }

    for state in [a, b, c] {
        close(state).await;
    }
    let _ = std::fs::remove_dir_all(&vault);
}

/// A kept chunk of a copy whose other half already took that chunk and
/// compacted past it: the slot is free again in the storage, but the
/// snapshot closed it, and a chunk written there would be a second version
/// of it with another hash. The copy rotates instead of replaying (audit
/// S-B, skeptic N4).
#[tokio::test]
async fn a_kept_chunk_is_not_replayed_into_a_range_a_snapshot_closed() {
    let vault = crate::db::test_dir();
    let a = device(sync_registry(), &vault).await;
    create_vault(&a).await;
    save_password(&a, "pw-1", "hunter2").await;
    cycle(&a).await;
    let b = device(sync_registry(), &vault).await;
    join_vault(&b).await;
    cycle(&b).await;
    let id = device_id(&a).await;
    let seq = own_seq(&a).await;

    retitle(&a, "pw-1", "kept").await;
    let db = &a.core.db;
    let lossy = lossy_engine(&a, &vault, &id).await;
    let (mut local, last) = state::load_local_state(db).await.unwrap();
    let mut collecting = HlcClock::new(id.clone(), last.as_ref());
    let tables: Vec<_> = a.sync.registry().tables().iter().collect();
    let changes = rows::collect_local_changes(&a.core, &mut collecting, &tables, None)
        .await
        .unwrap();
    veydan_sync_host::push_own(
        &lossy,
        db,
        a.sync.registry(),
        &mut local,
        &collecting,
        changes.ops,
    )
    .await
    .unwrap_err();
    // The other half of the copy took the chunk and compacted up to it.
    let engine = open_engine(&a.core).await.unwrap();
    let stored = engine.own_log_status(0, "").await.unwrap();
    assert_eq!(stored.seq, seq + 1);
    engine
        .compact(&veydan_sync::LocalState {
            own_seq: stored.seq,
            own_head_hash: stored.head_hash.clone(),
            ..Default::default()
        })
        .await
        .unwrap();
    let slot = vault
        .join("devices")
        .join(&id)
        .join("log")
        .join(format!("{:08}.bin", seq + 1));
    assert!(!slot.exists());

    rotating_cycle(&a).await;
    assert!(!slot.exists(), "nothing written into the closed range");
    cycle(&a).await;
    cycle(&b).await;
    assert_eq!(password_title(&b, "pw-1").await, "kept");

    for state in [a, b] {
        close(state).await;
    }
    let _ = std::fs::remove_dir_all(&vault);
}

/// A kept chunk of another vault or another device id (a leave and a join
/// at once, an older build in between) is dropped, not replayed (audit S-B,
/// skeptic N5).
#[tokio::test]
async fn a_kept_chunk_of_another_binding_is_dropped() {
    let vault = crate::db::test_dir();
    let a = device(sync_registry(), &vault).await;
    create_vault(&a).await;
    save_password(&a, "pw-1", "hunter2").await;
    cycle(&a).await;
    let id = device_id(&a).await;
    let seq = own_seq(&a).await;
    let db = &a.core.db;
    let (local, _) = state::load_local_state(db).await.unwrap();
    let op = Op {
        entity_type: "password".into(),
        entity_id: "pw-other".into(),
        hlc: Hlc {
            wall_ms: 1,
            counter: 0,
            device_id: id.clone(),
        },
        deleted: false,
        payload: json!({ "title": "of another vault" }),
    };
    for (vault_id, device) in [("another vault", id.clone()), ("", "another id".into())] {
        let vault_id = if vault_id.is_empty() {
            config::load_binding(db).await.unwrap().vault_id
        } else {
            vault_id.to_string()
        };
        state::save_own_pending(
            db,
            &state::OwnPending {
                vault_id,
                device_id: device,
                seq: local.own_seq + 1,
                prev_hash: local.own_head_hash.clone(),
                ops: vec![op.clone()],
            },
        )
        .await
        .unwrap();
        cycle(&a).await;
        assert!(state::load_own_pending(db).await.is_none());
        assert_eq!(own_seq(&a).await, seq);
        assert_eq!(device_id(&a).await, id);
    }

    close(a).await;
    let _ = std::fs::remove_dir_all(&vault);
}

/// A storage whose listing of the logs is empty (a WebDAV server answering
/// 404 for the folder) makes the own log look shorter than it is: the GET
/// of the saved chunk by its key shows it is there, and nothing is rewound
/// or rotated (audit S-B, skeptic N3).
#[tokio::test]
async fn a_listing_that_lacks_the_own_chunks_changes_nothing() {
    struct NoLog(veydan_sync::LocalDir);

    #[async_trait::async_trait]
    impl veydan_sync::Storage for NoLog {
        async fn list(&self, prefix: &str) -> veydan_sync::Result<Vec<String>> {
            if prefix.contains("/log") {
                return Ok(Vec::new());
            }
            self.0.list(prefix).await
        }
        async fn get(&self, key: &str) -> veydan_sync::Result<Option<Vec<u8>>> {
            self.0.get(key).await
        }
        async fn put(&self, key: &str, data: &[u8]) -> veydan_sync::Result<()> {
            self.0.put(key, data).await
        }
        async fn delete(&self, key: &str) -> veydan_sync::Result<()> {
            self.0.delete(key).await
        }
    }

    let vault = crate::db::test_dir();
    let a = device(sync_registry(), &vault).await;
    create_vault(&a).await;
    save_password(&a, "pw-1", "hunter2").await;
    cycle(&a).await;
    let id = device_id(&a).await;
    let seq = own_seq(&a).await;
    let db = &a.core.db;
    let binding = config::load_binding(db).await.unwrap();
    let blind = Engine::with_key(
        Box::new(NoLog(veydan_sync::LocalDir::new(&vault))),
        &veydan_sync::Vmk::from_base64(&binding.vmk_b64).unwrap(),
        &binding.vault_id,
        &id,
    );
    let (mut local, last) = state::load_local_state(db).await.unwrap();
    let mut clock = HlcClock::new(id.clone(), last.as_ref());
    let warning =
        veydan_sync_host::reconcile_own_log(&blind, db, a.sync.registry(), &mut local, &mut clock)
            .await
            .unwrap()
            .unwrap();
    assert!(warning.contains("listing"), "{warning}");
    assert_eq!(local.own_seq, seq);
    assert_eq!(own_seq(&a).await, seq);
    assert_eq!(device_id(&a).await, id);
    assert!(rows_to_publish(&a).await.is_empty());

    close(a).await;
    let _ = std::fs::remove_dir_all(&vault);
}

/// After a rotation the old log is read as another device's; an older
/// version of a note in it is this device's own past and does not replace
/// the file (audit A2-20).
#[tokio::test]
async fn the_old_log_read_back_does_not_bring_an_old_note_back() {
    let vault = crate::db::test_dir();
    let a = device(sync_registry(), &vault).await;
    create_vault(&a).await;
    for body in ["First.", "Second."] {
        write(
            &documents(&a).join("note-1.md"),
            &note_file("note-1", "Note", body),
        );
        if body == "First." {
            write_note(&a, "note-1", "Note", body).await;
        }
        cycle(&a).await;
    }
    config::rotate_device_id(&a.core.db, a.sync.registry(), None)
        .await
        .unwrap();
    // An edit under the new id; the old log comes back in the same cycle.
    write(
        &documents(&a).join("note-1.md"),
        &note_file("note-1", "Note", "Third."),
    );
    cycle(&a).await;
    cycle(&a).await;
    assert!(note_text(&a, "note-1").await.unwrap().contains("Third."));

    close(a).await;
    let _ = std::fs::remove_dir_all(&vault);
}

async fn conflicts(state: &TestStates) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM note_history WHERE version_type = 'conflict'")
        .fetch_one(&state.core.db)
        .await
        .unwrap()
}

/// Spec 9.3, "Leave, then join": a note edited on another device while
/// this one was away comes to it as edited. Its own older copy, published
/// at the join without a parent, would have looked like a fork with no
/// common ancestor: the newer op written over the edit and a conflict in
/// the history of both devices.
#[tokio::test]
async fn a_note_edited_while_a_device_was_away_comes_back_without_a_conflict() {
    let vault = crate::db::test_dir();
    let space = device(sync_registry(), &vault).await;
    create_vault(&space).await;
    write_note(&space, "note-1", "Deploy", "Version 1.").await;
    cycle(&space).await;
    let away = device(sync_registry(), &vault).await;
    join_vault(&away).await;
    cycle(&away).await;
    cycle(&away).await;
    assert!(note_text(&away, "note-1")
        .await
        .unwrap()
        .contains("Version 1."));
    join::leave(&away.core.db, away.sync.registry())
        .await
        .unwrap();

    write(
        &documents(&space).join("note-1.md"),
        &note_file("note-1", "Deploy", "Version 2."),
    );
    cycle(&space).await;
    join_vault(&away).await;
    cycle(&away).await;
    cycle(&away).await;
    cycle(&space).await;

    for state in [&space, &away] {
        let text = note_text(state, "note-1").await.unwrap();
        assert!(text.contains("Version 2."), "{text}");
        assert_eq!(conflicts(state).await, 0);
    }

    for state in [space, away] {
        close(state).await;
    }
    let _ = std::fs::remove_dir_all(&vault);
}

/// A blob one device has not uploaded yet holds the log of that device
/// alone: the cycle ends in a retry, the note of the other device is
/// applied and its head saved, and the held log is read again once the blob
/// is there (audit SYNCNOTES-12, A2-08).
#[tokio::test]
async fn a_blob_one_device_has_not_uploaded_yet_holds_only_its_log() {
    let vault = crate::db::test_dir();
    let a = device(notes_like(), &vault).await;
    create_vault(&a).await;
    write_note(&a, "note-a", "From A", "Written on A.").await;
    cycle(&a).await;
    let b = device(notes_like(), &vault).await;
    join_vault(&b).await;
    cycle(&b).await;
    write_note(&b, "note-b", "From B", "Written on B.").await;
    cycle(&b).await;
    let (id_a, id_b) = (device_id(&a).await, device_id(&b).await);
    let blob = ops_of(&a, &id_a)
        .await
        .into_iter()
        .find(|op| op.entity_type == "note" && op.entity_id == "note-a")
        .and_then(|op| op.payload["blob"].as_str().map(str::to_string))
        .expect("the put of note-a");
    let blob_path = vault.join("blobs").join(&blob);
    let bytes = std::fs::read(&blob_path).unwrap();
    std::fs::remove_file(&blob_path).unwrap();
    let heads = |state: &TestStates| {
        let db = state.core.db.clone();
        async move {
            let rows: Vec<(String, i64)> = sqlx::query_as("SELECT device_id, seq FROM sync_peers")
                .fetch_all(&db)
                .await
                .unwrap();
            rows.into_iter().collect::<BTreeMap<String, i64>>()
        }
    };

    let c = device(notes_like(), &vault).await;
    join_vault(&c).await;
    let warnings = veydan_sync_host::cycle(c.host(), &c.core, &c.sync)
        .await
        .unwrap();

    assert!(
        warnings
            .iter()
            .any(|w| w.starts_with("retry next cycle:") && w.contains("note-a")),
        "{warnings:?}"
    );
    assert!(note_text(&c, "note-b")
        .await
        .unwrap()
        .contains("Written on B."));
    assert_eq!(note_text(&c, "note-a").await, None);
    let saved = heads(&c).await;
    assert!(saved.get(&id_b).is_some_and(|seq| *seq > 0), "{saved:?}");
    assert!(!saved.contains_key(&id_a), "{saved:?}");

    write(&blob_path, &bytes);
    cycle(&c).await;
    assert!(note_text(&c, "note-a")
        .await
        .unwrap()
        .contains("Written on A."));
    assert!(heads(&c).await.get(&id_a).is_some_and(|seq| *seq > 0));

    for state in [a, b, c] {
        close(state).await;
    }
    let _ = std::fs::remove_dir_all(&vault);
}

/// A log read up to a broken file is held alone like any log read: a blob
/// missing for an op that came before the broken file keeps the head of
/// that log, the head of the other device's log is saved.
#[tokio::test]
async fn a_log_read_up_to_a_broken_file_is_held_alone() {
    let vault = crate::db::test_dir();
    let a = device(notes_like(), &vault).await;
    create_vault(&a).await;
    write_note(&a, "note-a", "From A", "Written on A.").await;
    cycle(&a).await;
    let b = device(notes_like(), &vault).await;
    join_vault(&b).await;
    cycle(&b).await;
    write_note(&b, "note-b", "From B", "Written on B.").await;
    cycle(&b).await;
    write_note(&a, "note-a2", "Later on A", "Written later.").await;
    cycle(&a).await;
    let (id_a, id_b) = (device_id(&a).await, device_id(&b).await);
    let blob = ops_of(&a, &id_a)
        .await
        .into_iter()
        .find(|op| op.entity_type == "note" && op.entity_id == "note-a")
        .and_then(|op| op.payload["blob"].as_str().map(str::to_string))
        .expect("the put of note-a");
    std::fs::remove_file(vault.join("blobs").join(&blob)).unwrap();
    // The last chunk of A is no envelope any more.
    let mut chunks: Vec<PathBuf> = std::fs::read_dir(vault.join("devices").join(&id_a).join("log"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    chunks.sort();
    std::fs::write(chunks.last().unwrap(), b"").unwrap();

    let c = device(notes_like(), &vault).await;
    join_vault(&c).await;
    let warnings = veydan_sync_host::cycle(c.host(), &c.core, &c.sync)
        .await
        .unwrap();
    assert!(
        warnings
            .iter()
            .any(|w| w.starts_with("retry next cycle:") && w.contains("note-a")),
        "{warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains(&id_a) && w.contains("format")),
        "{warnings:?}"
    );
    assert!(note_text(&c, "note-b")
        .await
        .unwrap()
        .contains("Written on B."));
    let saved: BTreeMap<String, i64> = sqlx::query_as("SELECT device_id, seq FROM sync_peers")
        .fetch_all(&c.core.db)
        .await
        .unwrap()
        .into_iter()
        .collect();
    assert!(saved.get(&id_b).is_some_and(|seq| *seq > 0), "{saved:?}");
    assert!(!saved.contains_key(&id_a), "{saved:?}");

    for state in [a, b, c] {
        close(state).await;
    }
    let _ = std::fs::remove_dir_all(&vault);
}
