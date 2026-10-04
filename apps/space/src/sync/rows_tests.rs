// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The row entities are part of the vault's wire format, frozen at what a
//! 4.0.7 client writes and reads: a vault is shared with such clients. The
//! entities are those the modules of Space register; the machinery is
//! `veydan_sync_host::rows`.

use super::golden;
use crate::db::test_pool;
#[cfg(desktop)]
use crate::modules::{TestStates, SYNC_PLAN};
use serde_json::{json, Value};
use sqlx::{Pool, Sqlite};
use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;
use veydan_lock::{synced_lock, SyncedLock};
use veydan_sync_host::rows::{delete_plain, put_row, read_row, read_rows, Put};
use veydan_sync_host::{Delete, Host, Registry, States, Table, SETTING_ENTITY};
#[cfg(desktop)]
use {
    veydan_core::settings,
    veydan_sync::{Hlc, HlcClock, Op},
    veydan_sync_host::rows::{self, LocalChanges},
    veydan_sync_host::state::{load_row_state, save_row_state},
};

/// What Space syncs, built once for the tests of this file.
static REGISTRY: LazyLock<Registry> = LazyLock::new(crate::modules::sync_registry);

fn spec_for(entity: &str) -> Option<&'static Table> {
    REGISTRY.table_of(entity)
}

/// Payload keys of every row entity as 4.0.7 publishes and applies them:
/// the synced columns and the link arrays.
const KEYS_OF_4_0_7: &[(&str, &[&str])] = &[
    (
        "workspace",
        &[
            "name",
            "description",
            "color",
            "icon",
            "notes",
            "is_default",
            "created_at",
            "updated_at",
        ],
    ),
    (
        "workspace_column",
        &[
            "workspace_id",
            "name",
            "tag_name",
            "color",
            "position",
            "created_at",
        ],
    ),
    (
        "proxy",
        &[
            "name",
            "proxy_type",
            "host",
            "port",
            "username",
            "password",
            "country",
            "city",
            "private_key",
            "server_fingerprint",
            "tags",
            "created_at",
        ],
    ),
    (
        "ssh_key",
        &[
            "name",
            "algorithm",
            "bits",
            "comment",
            "private_key",
            "public_key",
            "passphrase",
            "fingerprint",
            "source",
            "created_at",
            "updated_at",
        ],
    ),
    (
        "profile",
        &[
            "name",
            "browser_type",
            "proxy_id",
            "fingerprint_preset",
            "user_agent",
            "platform",
            "timezone",
            "locale",
            "languages",
            "screen_width",
            "screen_height",
            "webrtc_mode",
            "geolocation_enabled",
            "latitude",
            "longitude",
            "notes",
            "workspace_id",
            "kanban_status",
            "kanban_order",
            "tags",
            "webgl_vendor",
            "webgl_renderer",
            "default_search_engine",
            "history_enabled",
            "created_at",
        ],
    ),
    (
        "ssh_connection",
        &[
            "name",
            "host",
            "port",
            "username",
            "auth_type",
            "password",
            "private_key",
            "key_passphrase",
            "requires_2fa",
            "totp_entry_id",
            "proxy_id",
            "ssh_key_id",
            "connect_timeout_sec",
            "keepalive_sec",
            "terminal_theme",
            "default_cols",
            "default_rows",
            "server_fingerprint",
            "created_at",
            "updated_at",
            "workspace_ids",
            "profile_ids",
        ],
    ),
    (
        "totp",
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
    (
        "password_vault",
        &[
            "vault_id",
            "crypto_version",
            "kdf_algorithm",
            "kdf_salt",
            "kdf_memory",
            "kdf_iterations",
            "kdf_parallelism",
            "wrapped_key",
            "recovery_salt",
            "recovery_wrapped_key",
            "lock_hash",
            "lock_kind",
            "lock_hint",
            "created_at",
            "updated_at",
        ],
    ),
    (
        "password",
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
    ("pw_history", &["password", "created_at"]),
    ("note_tag", &["name", "color", "created_at", "updated_at"]),
    (
        "note_folder",
        &["name", "parent_id", "color", "created_at", "updated_at"],
    ),
    (
        "note_smart_view",
        &[
            "name",
            "color",
            "conditions",
            "sort_order",
            "created_at",
            "updated_at",
        ],
    ),
    ("note_meta", &["pinned", "archived", "folder_ids"]),
    ("setting", &["value"]),
];

/// Setting keys 4.0.7 shares between devices.
const SETTINGS_OF_4_0_7: &[&str] = &[
    "ui_locale",
    "minimize_to_tray",
    "close_to_tray",
    "start_hidden",
    "notes_lock_timeout_min",
    "notes_capture_rules",
    "quick_capture_shortcut",
];

const T1: &str = "2026-09-01T10:00:00+00:00";
const T2: &str = "2026-09-02T11:30:00+00:00";

fn vault_payload(lock_hash: Value, lock_kind: Value, lock_hint: Value, recovery: bool) -> Value {
    json!({
        "vault_id": "0b0e3c1e-5a52-4d0e-9a53-0d7e3d7c1a01",
        "crypto_version": 1,
        "kdf_algorithm": "argon2id",
        "kdf_salt": "c2FsdHNhbHRzYWx0c2FsdA==",
        "kdf_memory": 65536,
        "kdf_iterations": 3,
        "kdf_parallelism": 1,
        "wrapped_key": "d3JhcHBlZC1rZXk=",
        "recovery_salt": if recovery { json!("cmVjb3Zlcnktc2FsdC0xNg==") } else { Value::Null },
        "recovery_wrapped_key": if recovery { json!("cmVjb3Zlcnktd3JhcA==") } else { Value::Null },
        "lock_hash": lock_hash,
        "lock_kind": lock_kind,
        "lock_hint": lock_hint,
        "created_at": T1,
        "updated_at": T2,
    })
}

/// One row of every entity as a 4.0.7 client published it: the first put of
/// each in the ops 4.0.7 wrote (`crate::sync::golden`). Parents come before
/// the rows that name them.
fn rows_of_4_0_7() -> Vec<(&'static str, &'static str, Value)> {
    const ROWS: &[(&str, &str)] = &[
        ("workspace", "ws-1"),
        ("workspace", "ws-2"),
        ("workspace_column", "col-1"),
        ("proxy", "px-1"),
        ("ssh_key", "key-1"),
        ("profile", "pr-1"),
        ("ssh_connection", "ssh-1"),
        ("totp", "totp-1"),
        ("password_vault", "default"),
        ("password", "pw-1"),
        ("pw_history", "h-1"),
        ("note_tag", "tag-1"),
        ("note_folder", "f-1"),
        ("note_folder", "f-2"),
        ("note_smart_view", "sv-1"),
        ("note_meta", "note-1"),
        ("setting", "ui_locale"),
    ];
    ROWS.iter()
        .map(|(entity, id)| (*entity, *id, first_put_of_4_0_7(entity, id)))
        .collect()
}

/// The first put 4.0.7 wrote of `entity` `id`.
fn first_put_of_4_0_7(entity: &str, id: &str) -> Value {
    golden::ops(entity)
        .into_iter()
        .find(|op| op.entity_id == id && !op.deleted)
        .unwrap_or_else(|| panic!("4.0.7 wrote no {entity} {id}"))
        .payload
}

/// `note_meta` updates the row the note stream created from the note file.
async fn insert_note_row(db: &Pool<Sqlite>, id: &str) {
    sqlx::query(
        "INSERT INTO notes (id, title, file_path, created_at, updated_at) VALUES (?, 'Deploy', '/n.md', ?, ?)",
    )
    .bind(id)
    .bind(T1)
    .bind(T1)
    .execute(db)
    .await
    .unwrap();
}

/// The lock of the data file: the hook of the key row asks it which key to keep.
struct Locked(veydan_lock::Lock);

impl States for Locked {
    fn state(&self, ty: std::any::TypeId) -> Option<&(dyn std::any::Any + Send + Sync)> {
        (ty == std::any::TypeId::of::<veydan_lock::Lock>()).then_some(&self.0 as _)
    }
}

async fn put(db: &Pool<Sqlite>, dir: &Path, entity: &str, id: &str, payload: &Value) -> Put {
    let spec = spec_for(entity).expect("entity");
    let payload = payload.as_object().cloned().expect("object");
    let lock = Locked(veydan_lock::Lock::with_key_users(
        db.clone(),
        crate::modules::key_users(),
    ));
    put_row(
        &Host::States(&lock),
        db,
        dir,
        spec,
        id,
        payload,
        &mut HashMap::new(),
    )
    .await
    .unwrap()
}

fn done_as(put: &Put, id: &str) -> bool {
    matches!(put, Put::Done { id: applied, absorbed: None, .. } if applied == id)
}

/// Every row op 4.0.7 wrote, not only the samples, has the keys of the list
/// and no other; its tombstones have none.
#[test]
fn the_sample_rows_have_exactly_the_keys_of_4_0_7() {
    let written = KEYS_OF_4_0_7.iter().flat_map(|(entity, _)| {
        golden::ops(entity).into_iter().map(move |op| {
            let payload = if op.deleted {
                assert_eq!(op.payload, json!({}), "{entity} {}", op.entity_id);
                first_put_of_4_0_7(entity, &op.entity_id)
            } else {
                op.payload
            };
            (*entity, payload)
        })
    });
    let samples = rows_of_4_0_7()
        .into_iter()
        .map(|(entity, _, payload)| (entity, payload));
    for (entity, payload) in samples.chain(written) {
        let (_, keys) = KEYS_OF_4_0_7
            .iter()
            .find(|(e, _)| *e == entity)
            .expect("entity");
        let mut expected: Vec<&str> = keys.to_vec();
        expected.sort_unstable();
        let got: Vec<&str> = payload
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(got, expected, "{entity}");
    }
}

#[tokio::test]
async fn rows_written_by_4_0_7_apply_and_lose_nothing() {
    let (db, dir) = test_pool().await;
    insert_note_row(&db, "note-1").await;

    for (entity, id, payload) in rows_of_4_0_7() {
        let outcome = put(&db, &dir, entity, id, &payload).await;
        assert!(done_as(&outcome, id), "{entity} {id}");
        let stored = read_row(&db, spec_for(entity).unwrap(), id).await.unwrap();
        assert_eq!(stored, Some(payload), "{entity} {id}");
    }

    db.close().await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn keys_this_schema_has_no_column_for_are_ignored() {
    let (db, dir) = test_pool().await;
    insert_note_row(&db, "note-1").await;

    for (entity, id, payload) in rows_of_4_0_7() {
        let mut extended = payload.clone();
        let map = extended.as_object_mut().unwrap();
        for key in ["scope", "profile_id", "folder_id", "from_a_later_version"] {
            map.insert(key.into(), json!("x"));
        }
        map.entry("workspace_id").or_insert(json!("ws-1"));
        let outcome = put(&db, &dir, entity, id, &extended).await;
        assert!(done_as(&outcome, id), "{entity} {id}");
        let stored = read_row(&db, spec_for(entity).unwrap(), id).await.unwrap();
        assert_eq!(stored, Some(payload), "{entity} {id}");
    }

    db.close().await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn every_password_vault_form_of_4_0_7_is_accepted() {
    let (db, dir) = test_pool().await;
    let hash = json!("$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA");
    assert!(matches!(synced_lock(&db).await.unwrap(), SyncedLock::NoRow));

    // A PIN or password is set; kind and hint are optional, and so is the recovery wrap.
    for (kind, hint, recovery) in [
        (json!("pin"), json!("the usual"), true),
        (json!("password"), Value::Null, true),
        (Value::Null, Value::Null, false),
    ] {
        let payload = vault_payload(hash.clone(), kind.clone(), hint.clone(), recovery);
        assert!(done_as(
            &put(&db, &dir, "password_vault", "default", &payload).await,
            "default"
        ));
        let SyncedLock::Meta(meta) = synced_lock(&db).await.unwrap() else {
            panic!("lock fields of {kind} were not read");
        };
        assert_eq!(json!(meta.hash), hash);
        assert_eq!(json!(meta.kind), kind);
        assert_eq!(json!(meta.hint), hint);
        let stored = read_row(&db, spec_for("password_vault").unwrap(), "default")
            .await
            .unwrap();
        assert_eq!(stored, Some(payload));
    }

    // No lock on any device: the vault is wrapped with the built-in secret.
    let payload = vault_payload(Value::Null, json!("none"), Value::Null, false);
    assert!(done_as(
        &put(&db, &dir, "password_vault", "default", &payload).await,
        "default"
    ));
    assert!(matches!(
        synced_lock(&db).await.unwrap(),
        SyncedLock::Default
    ));

    // The row as `ensure_key` leaves it until the lock is published.
    let payload = vault_payload(Value::Null, Value::Null, Value::Null, false);
    assert!(done_as(
        &put(&db, &dir, "password_vault", "default", &payload).await,
        "default"
    ));
    assert!(matches!(
        synced_lock(&db).await.unwrap(),
        SyncedLock::Unstated
    ));
    let stored = read_row(&db, spec_for("password_vault").unwrap(), "default")
        .await
        .unwrap();
    assert_eq!(stored, Some(payload));

    db.close().await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn every_entity_of_4_0_7_is_published_with_every_key_it_reads() {
    for (entity, keys) in KEYS_OF_4_0_7 {
        let spec = spec_for(entity).unwrap_or_else(|| panic!("{entity} is not synced"));
        let published: Vec<&str> = spec
            .spec
            .columns
            .iter()
            .copied()
            .chain(spec.spec.links.iter().map(|l| l.key))
            .collect();
        for key in *keys {
            assert!(published.contains(key), "{entity} lost {key}");
        }
    }
}

#[tokio::test]
async fn rows_published_here_carry_every_key_of_4_0_7() {
    let (db, dir) = test_pool().await;
    insert_note_row(&db, "note-1").await;
    for (entity, id, payload) in rows_of_4_0_7() {
        put(&db, &dir, entity, id, &payload).await;
    }

    for (entity, keys) in KEYS_OF_4_0_7 {
        let rows = read_rows(&db, spec_for(entity).unwrap()).await.unwrap();
        assert!(!rows.is_empty(), "{entity}");
        for (id, payload) in rows {
            let payload = payload.as_object().unwrap();
            for key in *keys {
                assert!(payload.contains_key(*key), "{entity} {id} lost {key}");
            }
        }
    }

    db.close().await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn the_shared_settings_are_those_of_4_0_7() {
    let (db, dir) = test_pool().await;
    let local = [
        "sync_device_id",
        "notes_lock_hash",
        "lock_kind",
        "notes_custom_dir",
    ];
    for key in SETTINGS_OF_4_0_7.iter().chain(local.iter()) {
        sqlx::query("INSERT INTO app_settings (key, value) VALUES (?, '1')")
            .bind(key)
            .execute(&db)
            .await
            .unwrap();
    }

    let mut shared: Vec<String> = read_rows(&db, spec_for(SETTING_ENTITY).unwrap())
        .await
        .unwrap()
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    shared.sort();
    let mut expected: Vec<String> = SETTINGS_OF_4_0_7.iter().map(|k| k.to_string()).collect();
    expected.sort();
    assert_eq!(shared, expected);
    let mut written: Vec<String> = golden::ops(SETTING_ENTITY)
        .into_iter()
        .map(|op| op.entity_id)
        .collect();
    written.sort();
    written.dedup();
    assert_eq!(written, expected, "the keys 4.0.7 wrote to a vault");
    let mut registered: Vec<String> = REGISTRY
        .setting_keys()
        .into_iter()
        .map(|(key, _)| key.to_string())
        .collect();
    registered.sort();
    assert_eq!(registered, expected);

    db.close().await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_delete_of_every_entity_runs_on_this_schema() {
    let (db, dir) = test_pool().await;
    insert_note_row(&db, "note-1").await;
    let rows = rows_of_4_0_7();
    for (entity, id, payload) in &rows {
        put(&db, &dir, entity, id, payload).await;
    }

    // Children first, as `apply_order` sorts tombstones.
    for (entity, id, _) in rows.iter().rev() {
        let spec = spec_for(entity).unwrap();
        let Delete::Plain(extra) = &spec.spec.delete else {
            continue;
        };
        delete_plain(&db, spec, extra, id).await.unwrap();
        assert_eq!(
            read_row(&db, spec, id).await.unwrap(),
            None,
            "{entity} {id}"
        );
    }

    db.close().await;
    let _ = std::fs::remove_dir_all(&dir);
}

/// What follows applies whole pulls and needs the states of the desktop.
#[cfg(desktop)]
fn op(entity: &str, id: &str, wall_ms: u64, payload: Option<&Value>) -> Op {
    Op {
        entity_type: entity.into(),
        entity_id: id.into(),
        hlc: Hlc {
            wall_ms,
            counter: 0,
            device_id: "peer-a".into(),
        },
        deleted: payload.is_none(),
        payload: payload.cloned().unwrap_or_else(|| json!({})),
    }
}

/// The sample rows a tombstone removes: `note_meta` is not deleted by its own op.
#[cfg(desktop)]
fn deletable_rows() -> Vec<(&'static str, &'static str, Value)> {
    rows_of_4_0_7()
        .into_iter()
        .filter(|(entity, _, _)| !matches!(spec_for(entity).unwrap().spec.delete, Delete::Ignore))
        .collect()
}

/// One pull applied the way a cycle does it, scope after scope.
#[cfg(desktop)]
async fn apply(state: &TestStates, ops: &[Op]) {
    for label in ["notes catalog", "notes meta", "app rows"] {
        let tables = REGISTRY.step_rows(SYNC_PLAN.apply, label);
        let outcome = rows::apply_ops(&state.host(), &state.core, &tables, ops)
            .await
            .unwrap();
        assert_eq!(outcome.retry, None);
    }
}

/// What the next cycle would publish of the sample rows, as (entity, id).
#[cfg(desktop)]
async fn next_push(state: &TestStates, rows: &[(&str, &str, Value)]) -> Vec<(String, String)> {
    let mut clock = HlcClock::new("this-device", None);
    let mut pushed = Vec::new();
    for label in ["note rows", "app rows"] {
        let tables = REGISTRY.step_rows(SYNC_PLAN.collect, label);
        let changes = rows::collect_local_changes(&state.core, &mut clock, &tables, None)
            .await
            .unwrap();
        pushed.extend(
            changes
                .ops
                .into_iter()
                .map(|op| (op.entity_type, op.entity_id)),
        );
    }
    pushed.retain(|(entity, id)| rows.iter().any(|(e, i, _)| e == entity && i == id));
    pushed
}

#[cfg(desktop)]
async fn assert_gone(state: &TestStates, rows: &[(&str, &str, Value)], head_ms: u64) {
    for (entity, id, _) in rows {
        let spec = spec_for(entity).unwrap();
        assert_eq!(
            read_row(&state.core.db, spec, id).await.unwrap(),
            None,
            "{entity} {id}"
        );
        let st = load_row_state(&state.core.db, entity, id)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{entity} {id} has no sync state"));
        assert!(st.deleted, "{entity} {id}");
        assert_eq!(
            st.head_hlc.map(|h| h.wall_ms),
            Some(head_ms),
            "{entity} {id}"
        );
    }
    assert_eq!(next_push(state, rows).await, Vec::new());
}

#[cfg(desktop)]
#[tokio::test]
async fn a_row_put_and_deleted_in_one_pull_is_gone_and_stays_unpublished() {
    let state = TestStates::new().await;
    let rows = deletable_rows();
    // In HLC order, as a pull hands them over.
    let mut ops: Vec<Op> = rows
        .iter()
        .map(|(entity, id, payload)| op(entity, id, 1000, Some(payload)))
        .collect();
    ops.extend(
        rows.iter()
            .map(|(entity, id, _)| op(entity, id, 2000, None)),
    );

    apply(&state, &ops).await;

    assert_gone(&state, &rows, 2000).await;
    state.core.db.close().await;
    let _ = std::fs::remove_dir_all(&state.core.app_data_dir);
}

#[cfg(desktop)]
#[tokio::test]
async fn a_tombstone_for_a_row_that_is_not_here_holds_against_an_older_put() {
    let state = TestStates::new().await;
    let rows = deletable_rows();
    let deletes: Vec<Op> = rows
        .iter()
        .map(|(entity, id, _)| op(entity, id, 2000, None))
        .collect();
    apply(&state, &deletes).await;
    assert_gone(&state, &rows, 2000).await;

    // The log of the device that wrote the rows becomes readable a pull later.
    let puts: Vec<Op> = rows
        .iter()
        .map(|(entity, id, payload)| op(entity, id, 1000, Some(payload)))
        .collect();
    apply(&state, &puts).await;

    assert_gone(&state, &rows, 2000).await;
    state.core.db.close().await;
    let _ = std::fs::remove_dir_all(&state.core.app_data_dir);
}

#[cfg(desktop)]
#[tokio::test]
async fn a_put_newer_than_a_tombstone_of_the_same_pull_wins() {
    let state = TestStates::new().await;
    let rows = deletable_rows();
    let mut ops: Vec<Op> = rows
        .iter()
        .map(|(entity, id, _)| op(entity, id, 1000, None))
        .collect();
    ops.extend(
        rows.iter()
            .map(|(entity, id, payload)| op(entity, id, 2000, Some(payload))),
    );

    apply(&state, &ops).await;

    for (entity, id, payload) in &rows {
        let spec = spec_for(entity).unwrap();
        let stored = read_row(&state.core.db, spec, id).await.unwrap();
        assert_eq!(stored.as_ref(), Some(payload), "{entity} {id}");
    }
    assert_eq!(next_push(&state, &rows).await, Vec::new());
    state.core.db.close().await;
    let _ = std::fs::remove_dir_all(&state.core.app_data_dir);
}

/// The registry of Space takes every row 4.0.7 publishes in one pull, with
/// none skipped or waiting: each of the seven shared settings, and the key
/// row in each of its forms, as 4.0.7 wrote them. A setting key 4.0.7 does
/// not share is not taken. The types with a handler of their own go through
/// the same registry in `compat_tests`, a whole cycle over a vault 4.0.7 wrote.
#[cfg(desktop)]
#[tokio::test]
async fn every_op_of_4_0_7_applies_through_the_registry_of_space() {
    let state = TestStates::new().await;
    let db = &state.core.db;
    insert_note_row(db, "note-1").await;
    let mut rows = rows_of_4_0_7();
    rows.extend(
        SETTINGS_OF_4_0_7
            .iter()
            .filter(|key| **key != "ui_locale")
            .map(|key| ("setting", *key, first_put_of_4_0_7("setting", key))),
    );
    let mut ops: Vec<Op> = rows
        .iter()
        .map(|(entity, id, payload)| op(entity, id, 1000, Some(payload)))
        .collect();
    ops.push(op(
        "setting",
        "sync_folder_path",
        1000,
        Some(&json!({ "value": "/theirs" })),
    ));

    apply(&state, &ops).await;

    for (entity, id, payload) in &rows {
        let spec = spec_for(entity).unwrap();
        let stored = read_row(db, spec, id).await.unwrap();
        assert_eq!(stored.as_ref(), Some(payload), "{entity} {id}");
        let st = load_row_state(db, entity, id).await.unwrap();
        assert_eq!(
            st.and_then(|s| s.head_hlc).map(|h| h.wall_ms),
            Some(1000),
            "{entity} {id}"
        );
    }
    assert_eq!(settings::get(db, "sync_folder_path").await, None);
    assert!(load_row_state(db, "setting", "sync_folder_path")
        .await
        .unwrap()
        .is_none());
    assert_eq!(next_push(&state, &rows).await, Vec::new());

    let forms: Vec<Value> = golden::ops("password_vault")
        .into_iter()
        .map(|op| op.payload)
        .collect();
    assert_eq!(forms.len(), 3);
    for (i, payload) in forms.iter().enumerate() {
        let wall_ms = 2000 + i as u64;
        apply(
            &state,
            &[op("password_vault", "default", wall_ms, Some(payload))],
        )
        .await;
        let spec = spec_for("password_vault").unwrap();
        assert_eq!(
            read_row(db, spec, "default").await.unwrap().as_ref(),
            Some(payload)
        );
        let st = load_row_state(db, "password_vault", "default")
            .await
            .unwrap();
        assert_eq!(
            st.and_then(|s| s.head_hlc).map(|h| h.wall_ms),
            Some(wall_ms)
        );
    }
    assert!(matches!(
        synced_lock(db).await.unwrap(),
        SyncedLock::Unstated
    ));

    state.core.db.close().await;
    let _ = std::fs::remove_dir_all(&state.core.app_data_dir);
}

/// What the next cycle would publish of the flags of `note-1`.
#[cfg(desktop)]
async fn flags_push(state: &TestStates) -> LocalChanges {
    let mut clock = HlcClock::new("this-device", None);
    let tables = REGISTRY.step_rows(SYNC_PLAN.collect, "note rows");
    let mut changes = rows::collect_local_changes(&state.core, &mut clock, &tables, None)
        .await
        .unwrap();
    changes.ops.retain(|op| op.entity_type == "note_meta");
    changes
}

#[cfg(desktop)]
#[tokio::test]
async fn flags_nobody_set_here_are_not_published() {
    let state = TestStates::new().await;
    let db = &state.core.db;
    insert_note_row(db, "note-1").await;
    assert!(flags_push(&state).await.ops.is_empty());

    // A pin set here is published, and so is its removal afterwards.
    sqlx::query("UPDATE notes SET pinned = 1 WHERE id = 'note-1'")
        .execute(db)
        .await
        .unwrap();
    let pinned = flags_push(&state).await;
    assert_eq!(pinned.ops.len(), 1);
    assert_eq!(
        pinned.ops[0].payload,
        json!({ "pinned": 1, "archived": 0, "folder_ids": [] })
    );
    for st in &pinned.states {
        save_row_state(db, st).await.unwrap();
    }
    sqlx::query("UPDATE notes SET pinned = 0 WHERE id = 'note-1'")
        .execute(db)
        .await
        .unwrap();
    let unpinned = flags_push(&state).await;
    assert_eq!(unpinned.ops.len(), 1);
    assert_eq!(
        unpinned.ops[0].payload,
        json!({ "pinned": 0, "archived": 0, "folder_ids": [] })
    );

    db.close().await;
    let _ = std::fs::remove_dir_all(&state.core.app_data_dir);
}

#[cfg(desktop)]
#[tokio::test]
async fn the_newest_flags_of_a_note_that_is_not_here_wait_for_it() {
    let state = TestStates::new().await;
    let db = &state.core.db;
    let pinned = json!({ "pinned": 1, "archived": 0, "folder_ids": [] });
    let archived = json!({ "pinned": 0, "archived": 1, "folder_ids": [] });
    // The newer op first, as when its author's log is readable a pull earlier.
    apply(&state, &[op("note_meta", "note-1", 2000, Some(&archived))]).await;
    apply(&state, &[op("note_meta", "note-1", 1000, Some(&pinned))]).await;
    let spec = spec_for("note_meta").unwrap();
    assert_eq!(read_row(db, spec, "note-1").await.unwrap(), None);
    assert!(load_row_state(db, "note_meta", "note-1")
        .await
        .unwrap()
        .is_none());

    // The note comes back; the next apply has no op for it.
    insert_note_row(db, "note-1").await;
    apply(&state, &[]).await;

    assert_eq!(read_row(db, spec, "note-1").await.unwrap(), Some(archived));
    let st = load_row_state(db, "note_meta", "note-1")
        .await
        .unwrap()
        .expect("sync state");
    assert_eq!(st.head_hlc.map(|h| h.wall_ms), Some(2000));
    assert!(flags_push(&state).await.ops.is_empty());
    let held = settings::get(db, spec.spec.hold.unwrap()).await;
    assert_eq!(held, None);

    db.close().await;
    let _ = std::fs::remove_dir_all(&state.core.app_data_dir);
}
