// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a device pushes for a given local state is part of what other
//! devices read: the ops, their order and their clock. `collect.golden.json`
//! holds what the collect of platform-stage-3 gathered for the data file
//! below, in two cycles; it was written by that code, before the entities
//! were registered by their modules. The registry and the plan of Space
//! gather the same, byte for byte. Stage 6 added the labels and nothing
//! else: the device is updated between the two cycles, the first start of
//! the build with labels publishes them, and the second cycle ends with
//! their ops — after every op of stage 3, whose clock they leave as it was.

use crate::modules::TestStates;
use veydan_sync::{Engine, Hlc, HlcClock, LocalDir, LocalState, Op, Vmk};
use veydan_sync_host::config::load_config;
use veydan_sync_host::Cycle;

const T1: &str = "2026-09-01T10:00:00+00:00";
const T2: &str = "2026-09-02T11:30:00+00:00";
const DEVICE: &str = "golden-device";
const VAULT_KEY: &str = "golden-vault-key";

async fn sql(state: &TestStates, statement: &str) {
    sqlx::query(sqlx::AssertSqlSafe(statement.to_string()))
        .execute(&state.core.db)
        .await
        .unwrap_or_else(|e| panic!("{statement}: {e}"));
}

fn documents(state: &TestStates) -> std::path::PathBuf {
    state.core.app_data_dir.join("notes").join("documents")
}

fn write(path: &std::path::Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn note_file(id: &str, title: &str, body: &str) -> Vec<u8> {
    format!(
        "---\nid: {id}\ntitle: \"{title}\"\nformat: md\nbindings: []\ntags: []\ncreated_at: {T1}\nupdated_at: {T2}\n---\n{body}\n"
    )
    .into_bytes()
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

/// A data file with a row of every table entity, three notes with their
/// files, a small and a large attachment, the shared settings and some
/// that stay local, a row the vault has and this device lost, and a lease
/// this device took. At most one tombstone per entity type: tombstones
/// come out of a hash map.
async fn fixture(state: &TestStates) {
    let db = &state.core.db;
    for (key, value) in [
        ("sync_device_id", DEVICE),
        ("ui_locale", "ru"),
        ("minimize_to_tray", "1"),
        ("close_to_tray", "0"),
        ("start_hidden", "0"),
        ("notes_lock_timeout_min", "5"),
        ("notes_capture_rules", "[{\"host\":\"example.com\"}]"),
        ("quick_capture_shortcut", "CommandOrControl+Shift+N"),
        ("notes_custom_dir", ""),
        ("sync_interval_sec", "60"),
        (
            "notes_attachment_policy",
            "{\"large_files_enabled\":true,\"threshold_mib\":1,\"max_file_gib\":10,\"download_on_sync\":true,\"ask_above_mib\":16}",
        ),
    ] {
        veydan_core::settings::set(db, key, value).await.unwrap();
    }
    // The schema leaves a default workspace stamped with the time of the test.
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
         ('pr-1', 'Brand A', 'stopped', '/somewhere/profiles/pr-1', 'camoufox', 'px-1', 'windows', 'Europe/Berlin', 'de-DE', 'de-DE,de,en',
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
    let attachments = documents(state).join("attachments");
    write(
        &attachments.join("note-1").join("scan.pdf"),
        b"%PDF-1.4 a small scan",
    );
    write(
        &attachments.join("note-2").join("video.bin"),
        &large_bytes(3 << 19),
    );
    sql(
        state,
        "INSERT INTO sync_row_state (entity_type, entity_id, head_hlc, synced_hash, deleted) VALUES
         ('proxy', 'px-gone', '0000018f00000000-00000000-peer', 'gone', 0)",
    )
    .await;
    sql(state, &format!(
        "INSERT INTO sync_profile_files_state (profile_id, lease_device, lease_name, lease_since, lease_synced) VALUES
         ('pr-1', '{DEVICE}', 'Laptop', '{T1}', 0)"
    )).await;
}

/// What changes on the device between the two cycles of the test.
async fn edit(state: &TestStates) {
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
    std::fs::remove_file(
        documents(state)
            .join("attachments")
            .join("note-1")
            .join("scan.pdf"),
    )
    .unwrap();
    sql(state, "UPDATE sync_profile_files_state SET lease_device = '', lease_name = '', lease_since = '', lease_synced = 0
         WHERE profile_id = 'pr-1'").await;
}

fn engine(vault: &std::path::Path) -> Engine {
    let vmk = Vmk::from_base64("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=").unwrap();
    Engine::with_key(Box::new(LocalDir::new(vault)), &vmk, "golden-vault", DEVICE)
}

/// Far ahead of the wall clock: every op of the test gets the same wall
/// time and the next counter.
fn clock() -> HlcClock {
    let start = Hlc {
        wall_ms: 4_102_444_800_000,
        counter: 0,
        device_id: DEVICE.into(),
    };
    HlcClock::new(DEVICE, Some(&start))
}

fn op_json(op: &Op) -> serde_json::Value {
    serde_json::json!({
        "entity_type": op.entity_type,
        "entity_id": op.entity_id,
        "hlc": op.hlc.encode(),
        "deleted": op.deleted,
        "payload": op.payload,
    })
}

async fn row_states_json(state: &TestStates) -> serde_json::Value {
    let rows: Vec<(String, String, String, String, i64)> = sqlx::query_as(
        "SELECT entity_type, entity_id, head_hlc, synced_hash, deleted FROM sync_row_state
         ORDER BY entity_type, entity_id",
    )
    .fetch_all(&state.core.db)
    .await
    .unwrap();
    serde_json::Value::Array(
        rows.into_iter()
            .map(|(entity, id, hlc, hash, deleted)| {
                serde_json::json!({ "entity": entity, "id": id, "hlc": hlc, "hash": hash, "deleted": deleted })
            })
            .collect(),
    )
}

/// The collect of a cycle, then what it saves once the push went through.
async fn collect(state: &TestStates, engine: &Engine, clock: &mut HlcClock) -> Vec<Op> {
    let config = load_config(&state.core.db).await;
    let mut local = LocalState::default();
    let mut warnings = Vec::new();
    let mut cx = Cycle {
        host: state.host(),
        core: &state.core,
        sync: &state.sync,
        engine,
        config: &config,
        clock,
        local: &mut local,
        warnings: &mut warnings,
    };
    let gathered = veydan_sync_host::collect(&mut cx).await.unwrap();
    for step in &gathered.pending {
        assert!(step.errors.is_empty(), "{}: {:?}", step.label, step.errors);
    }
    assert!(warnings.is_empty(), "{warnings:?}");
    gathered.saves.save(&state.core.db).await.unwrap();
    gathered.ops
}

#[tokio::test]
async fn a_cycle_pushes_what_it_pushed_at_stage_3() {
    let state = TestStates::new().await;
    let vault = crate::db::test_dir();
    let engine = engine(&vault);
    let mut clock = clock();
    fixture(&state).await;
    let golden: serde_json::Value =
        serde_json::from_str(include_str!("collect.golden.json")).unwrap();

    for (round, expected) in golden["rounds"].as_array().unwrap().iter().enumerate() {
        if round == 1 {
            edit(&state).await;
            state.publish_labels().await;
        }
        let ops: Vec<_> = collect(&state, &engine, &mut clock)
            .await
            .iter()
            .map(op_json)
            .collect();
        let expected_ops = expected["ops"].as_array().unwrap();
        for (i, (got, want)) in ops.iter().zip(expected_ops).enumerate() {
            assert_eq!(got, want, "cycle {round}, op {i}");
        }
        assert_eq!(ops.len(), expected_ops.len(), "cycle {round}");
        assert_eq!(
            row_states_json(&state).await,
            expected["row_states"],
            "cycle {round}"
        );
    }

    state.core.db.close().await;
    let _ = std::fs::remove_dir_all(&state.core.app_data_dir);
    let _ = std::fs::remove_dir_all(&vault);
}
