// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The vault's format is frozen at what 4.0.7 writes and reads, and a vault
//! may be shared with 4.0.7 clients (spec 9.3–9.5). These tests read what
//! the sync code of 4.0.7 itself wrote and made of fixed data, kept in
//! `crates/sync-host/tests/golden/` (how it was made: `capture-4.0.7.rs`):
//!
//! - `<type>.json` — every op of one entity type of the vault below, puts and
//!   tombstones, as a device pulls them: the three forms of the key row,
//!   the seven settings, attachments v1 and v2;
//! - `vault/` — the vault in a folder a 4.0.7 desktop left after three
//!   cycles, with its passphrase in `vault.json`;
//! - `apply.json` — what a second 4.0.7 desktop made of that vault in one
//!   cycle;
//! - `collect.json` — what 4.0.7 pushes for the data file of `collect_tests`.

use super::golden::{self, read_json};
use super::registries_tests::{
    close, collect_garbage, cycle, device_id, device_owning, documents, space_owners, write,
};
use crate::modules::{sync_registry, TestStates};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use veydan_core::settings;
use veydan_sync::{Hlc, Op};
use veydan_sync_host::{join, open_engine, rows};

/// The newest op of each entity of `ops`.
fn latest(ops: Vec<Op>) -> BTreeMap<(String, String), Op> {
    let mut out: BTreeMap<(String, String), Op> = BTreeMap::new();
    for op in ops {
        let key = (op.entity_type.clone(), op.entity_id.clone());
        if out.get(&key).is_none_or(|kept| kept.hlc < op.hlc) {
            out.insert(key, op);
        }
    }
    out
}

#[test]
fn the_goldens_hold_every_type_and_form_of_4_0_7() {
    let registry = sync_registry();
    let mut registered: BTreeSet<String> = registry
        .entities()
        .into_iter()
        .map(str::to_string)
        .collect();
    for entity in golden::NEW_IN_5 {
        assert!(registered.remove(entity), "{entity} is not registered");
    }
    assert_eq!(golden::types(), registered);
    for entity in &registered {
        let ops = golden::ops(entity);
        assert!(ops.iter().any(|op| !op.deleted), "{entity} has no put");
        assert!(
            ops.iter().all(|op| op.entity_type == *entity),
            "{entity}.json holds another type"
        );
    }

    let forms: Vec<(Value, Value)> = golden::ops("password_vault")
        .into_iter()
        .map(|op| {
            (
                op.payload["lock_kind"].clone(),
                op.payload["lock_hash"].clone(),
            )
        })
        .collect();
    assert_eq!(forms.len(), 3);
    assert!(forms[0].0 == json!("pin") && forms[0].1.is_string());
    assert!(forms[1].0 == json!("none") && forms[1].1.is_null());
    assert!(forms[2].0.is_null() && forms[2].1.is_null());

    let keys: BTreeSet<String> = golden::ops("setting")
        .into_iter()
        .map(|op| op.entity_id)
        .collect();
    let registered_keys: BTreeSet<String> = registry
        .setting_keys()
        .into_iter()
        .map(|(key, _)| key.to_string())
        .collect();
    assert_eq!(keys, registered_keys);
    assert_eq!(keys.len(), 7);
}

// ── A vault 4.0.7 wrote ──────────────────────────────────────────────────────

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

/// A copy of the vault 4.0.7 wrote, in a fresh directory.
fn vault_of_4_0_7() -> PathBuf {
    let vault = crate::db::test_dir();
    copy_dir(&golden::dir().join("vault"), &vault);
    vault
}

/// A device of Space joined to `vault` that ran its first cycle, which must
/// end clean: nothing waits for the next cycle. Its modules own their kinds
/// and published their labels at the start, as a build with labels does.
async fn space_on(vault: &Path) -> TestStates {
    let state = device_owning(sync_registry(), &space_owners(), vault).await;
    state.publish_labels().await;
    let passphrase = read_json("vault.json")["passphrase"]
        .as_str()
        .unwrap()
        .to_string();
    if join::join(
        &state.host(),
        &state.core,
        state.sync.registry(),
        &passphrase,
    )
    .await
    .unwrap()
    {
        state.lock.refresh_after_sync().await;
    }
    cycle(&state).await;
    state
}

/// The files under `dir`, by path: the text of a small one, else its size and hash.
fn files_under(dir: &Path) -> Value {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Value>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                walk(root, &path, out);
                continue;
            }
            let bytes = std::fs::read(&path).unwrap();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let value = match String::from_utf8(bytes.clone()) {
                Ok(text) if text.len() < 4096 => json!(text),
                _ => json!({ "size": bytes.len(), "sha256": veydan_sync::sha256_hex(&bytes) }),
            };
            out.insert(rel, value);
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    json!(out)
}

/// What a device holds of the synced data, in the shape of `apply.json`:
/// the synced columns of every row of a type 4.0.7 knows, the notes and
/// their attachments, the files of the profiles, the leases.
async fn holds(state: &TestStates) -> Value {
    let db = &state.core.db;
    let mut tables = Map::new();
    for table in state.sync.registry().tables() {
        if golden::NEW_IN_5.contains(&table.spec.entity) {
            continue;
        }
        let mut rows_of = Map::new();
        for (id, payload) in rows::read_rows(db, table).await.unwrap() {
            rows_of.insert(id, payload);
        }
        tables.insert(table.spec.entity.into(), Value::Object(rows_of));
    }
    let notes: Vec<(String, String, i64)> =
        sqlx::query_as("SELECT id, file_path, deleted FROM notes ORDER BY id")
            .fetch_all(db)
            .await
            .unwrap();
    let mut note_files = Map::new();
    for (id, file_path, deleted) in notes {
        let path =
            veydan_notes::resolve_note_abs_path(&state.core.app_data_dir, &file_path);
        note_files.insert(
            id,
            json!({ "deleted": deleted, "text": std::fs::read_to_string(&path).ok() }),
        );
    }
    let leases: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT profile_id, lease_device, lease_name, lease_since FROM sync_profile_files_state
         ORDER BY profile_id",
    )
    .fetch_all(db)
    .await
    .unwrap();
    let profiles: Vec<(String, String)> =
        sqlx::query_as("SELECT id, profile_path FROM profiles ORDER BY id")
            .fetch_all(db)
            .await
            .unwrap();
    let mut profile_files = Map::new();
    for (id, path) in profiles {
        profile_files.insert(id, files_under(&Path::new(&path).join("firefox-profile")));
    }
    json!({
        "rows": tables,
        "notes": note_files,
        "attachments": files_under(&documents(state).join("attachments")),
        "profile_files": profile_files,
        "leases": leases
            .into_iter()
            .map(|(p, d, n, s)| json!({ "profile_id": p, "device": d, "name": n, "since": s }))
            .collect::<Vec<_>>(),
    })
}

/// The head this device recorded for the entity of `op`, by the state of
/// whoever applied it.
async fn head_of(state: &TestStates, op: &Op) -> Option<Hlc> {
    let db = &state.core.db;
    let id = op.entity_id.as_str();
    let head: Option<String> = match op.entity_type.as_str() {
        "note" => sqlx::query_scalar("SELECT head_hlc FROM sync_note_state WHERE note_id = ?")
            .bind(id)
            .fetch_optional(db)
            .await
            .unwrap(),
        "note_attachment" | "note_attachment_v2" => {
            let (note_id, name) = id.split_once('/').unwrap();
            sqlx::query_scalar(
                "SELECT head_hlc FROM sync_attachment_state WHERE note_id = ? AND name = ?",
            )
            .bind(note_id)
            .bind(name)
            .fetch_optional(db)
            .await
            .unwrap()
        }
        "profile_lease" => sqlx::query_scalar(
            "SELECT lease_hlc FROM sync_profile_files_state WHERE profile_id = ?",
        )
        .bind(id)
        .fetch_optional(db)
        .await
        .unwrap(),
        "profile_snapshot" => {
            sqlx::query_scalar("SELECT head_hlc FROM sync_profile_files_state WHERE profile_id = ?")
                .bind(id)
                .fetch_optional(db)
                .await
                .unwrap()
        }
        entity => sqlx::query_scalar(
            "SELECT head_hlc FROM sync_row_state WHERE entity_type = ? AND entity_id = ?",
        )
        .bind(entity)
        .bind(id)
        .fetch_optional(db)
        .await
        .unwrap(),
    };
    head.and_then(|h| Hlc::decode(&h))
}

/// What a device of Space holds of the vault 4.0.7 wrote, against what a
/// 4.0.7 desktop held of it (`apply.json`). One difference is deliberate
/// (platform stage 1, `f9f9baf`): a note created and deleted before the
/// device joined does not come back here, nor do its flags, where 4.0.7
/// brought it back as a live note. The default workspace of either device
/// is its own, stamped with the time it was made.
async fn holds_as_4_0_7_held(state: &TestStates) -> (Value, Value) {
    let own_default = |held: &mut Value| {
        for key in ["created_at", "updated_at"] {
            held["rows"]["workspace"]["default"][key] = json!("made here");
        }
    };
    let mut theirs = read_json("apply.json");
    own_default(&mut theirs);
    theirs["notes"].as_object_mut().unwrap().remove("note-3");
    theirs["rows"]["note_meta"]
        .as_object_mut()
        .unwrap()
        .remove("note-3");
    let mut ours = holds(state).await;
    own_default(&mut ours);
    (ours, theirs)
}

/// Spec 9.4: the registry of Space takes every op of every type 4.0.7
/// wrote — the three forms of the key row, the seven settings, notes,
/// attachments v1 and v2, leases and snapshots — in one cycle that ends
/// with nothing skipped or waiting for the next one, and leaves the device
/// as 4.0.7 left it.
#[tokio::test]
async fn a_vault_4_0_7_wrote_is_applied_as_4_0_7_applied_it() {
    let vault = vault_of_4_0_7();
    let state = space_on(&vault).await;
    let db = &state.core.db;

    let mut all = Vec::new();
    for entity in golden::types() {
        all.extend(golden::ops(&entity));
    }
    let applied: usize = settings::get(db, "sync_last_applied")
        .await
        .and_then(|n| n.parse().ok())
        .unwrap();
    assert_eq!(applied, all.len());
    for ((entity, id), op) in latest(all) {
        if (entity.as_str(), id.as_str()) == ("note_meta", "note-3") {
            // The flags wait for a note that was deleted before the join.
            let held = settings::get(db, "sync_held_note_flags").await.unwrap();
            assert!(held.contains("note-3"), "{held}");
            continue;
        }
        // The default workspace of this device was published before the
        // pull and is newer than 4.0.7's.
        let head = head_of(&state, &op).await;
        assert!(
            head.as_ref().is_some_and(|head| *head >= op.hlc),
            "{entity} {id} was not taken: {head:?}"
        );
    }
    let (ours, theirs) = holds_as_4_0_7_held(&state).await;
    assert_eq!(ours, theirs);

    // Spec 10.2: what came from 4.0.7 is named in the labels by its owners,
    // by sync as by a start.
    let labels: Vec<(String, String, Option<String>, Option<String>)> =
        sqlx::query_as("SELECT key, name, parent_id, color FROM labels ORDER BY key")
            .fetch_all(db)
            .await
            .unwrap();
    let named: Vec<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT 'workspace:' || id, name, NULL, color FROM workspaces
         UNION ALL SELECT 'profile:' || id, name, workspace_id, NULL FROM profiles
         UNION ALL SELECT 'proxy:' || id, name, NULL, NULL FROM proxies
         UNION ALL SELECT 'ssh:' || id, name, NULL, NULL FROM ssh_connections
         UNION ALL SELECT 'password:' || id, title, NULL, NULL FROM passwords
         UNION ALL SELECT 'totp:' || id, name, NULL, NULL FROM totp_entries
         UNION ALL SELECT 'note:' || id, title, NULL, NULL FROM notes
         ORDER BY 1",
    )
    .fetch_all(db)
    .await
    .unwrap();
    assert_eq!(named.len(), 10, "{named:?}");
    assert_eq!(labels, named);

    close(state).await;
    let _ = std::fs::remove_dir_all(&vault);
}

/// After the garbage collection of a Space desktop, every blob and large
/// file a live op of 4.0.7 names is still in the vault and reads back as
/// what 4.0.7 put there, and the device holds what it held.
#[tokio::test]
async fn the_garbage_collection_of_space_keeps_what_4_0_7_needs() {
    let vault = vault_of_4_0_7();
    let space = space_on(&vault).await;
    let outcome = collect_garbage(&space).await;
    assert!(outcome.unknown.is_empty(), "{:?}", outcome.unknown);
    assert!(outcome.removed > 0, "the blobs of deleted attachments go");

    let engine = open_engine(&space.core).await.unwrap();
    let live: Vec<Op> = engine
        .all_latest_ops()
        .await
        .unwrap()
        .into_iter()
        .filter(|op| !op.deleted)
        .collect();
    let mut blobs: Vec<String> = Vec::new();
    for op in &live {
        let payload = &op.payload;
        match op.entity_type.as_str() {
            "note" => {
                blobs.push(payload["blob"].as_str().unwrap().into());
                blobs.extend(
                    payload["parents"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|p| p.as_str().unwrap().to_string()),
                );
            }
            "note_attachment" => blobs.push(payload["blob"].as_str().unwrap().into()),
            "profile_snapshot" => {
                let manifest = payload["manifest_blob"].as_str().unwrap();
                let text = engine.get_blob(manifest).await.unwrap().expect("manifest");
                let files: Value = serde_json::from_slice(&text).unwrap();
                blobs.push(manifest.into());
                blobs.extend(
                    files["files"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|f| f["blob"].as_str().unwrap().to_string()),
                );
            }
            _ => {}
        }
    }
    // Two notes, the parent of one, an attachment, a manifest and its two files.
    assert_eq!(blobs.len(), 7, "{blobs:?}");
    for blob in &blobs {
        assert!(
            engine.get_blob(blob).await.unwrap().is_some(),
            "{blob} is gone"
        );
    }
    let store = engine.large_files(Default::default()).unwrap();
    let held = read_json("apply.json");
    let mut large = 0;
    for op in live
        .iter()
        .filter(|op| op.entity_type == "note_attachment_v2")
    {
        let lf: veydan_sync::LargeFileRef =
            serde_json::from_value(op.payload["lf"].clone()).unwrap();
        let dest = crate::db::test_dir().join("large");
        store
            .download(
                &lf,
                &veydan_sync::PathSink::new(&dest),
                &|_| {},
                &Default::default(),
            )
            .await
            .unwrap();
        let bytes = std::fs::read(&dest).unwrap();
        assert_eq!(
            held["attachments"][&op.entity_id]["sha256"],
            json!(veydan_sync::sha256_hex(&bytes)),
            "{}",
            op.entity_id
        );
        let _ = std::fs::remove_dir_all(dest.parent().unwrap());
        large += 1;
    }
    assert_eq!(large, 1);

    cycle(&space).await;
    let (ours, theirs) = holds_as_4_0_7_held(&space).await;
    assert_eq!(ours, theirs);

    close(space).await;
    let _ = std::fs::remove_dir_all(&vault);
}

/// The type of a JSON value, as the frozen format knows it.
fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// `ours` has every key of `theirs` with a value of the same type; more
/// keys are allowed (spec 9.4).
fn carries_every_key(ours: &Value, theirs: &Value) -> Result<(), String> {
    let (Some(ours), Some(theirs)) = (ours.as_object(), theirs.as_object()) else {
        return Err("not an object".into());
    };
    for (key, value) in theirs {
        match ours.get(key) {
            None => return Err(format!("no `{key}`")),
            Some(mine) if kind(mine) != kind(value) => {
                return Err(format!("`{key}` is a {} here", kind(mine)))
            }
            Some(_) => {}
        }
    }
    Ok(())
}

/// Spec 9.4: what Space publishes of every type carries every key of what
/// 4.0.7 published of it, with values of the same types. The rows are those
/// Space applied from the vault; notes, attachments, a lease and a snapshot
/// are changed here and pushed by a cycle.
#[tokio::test]
async fn what_space_publishes_carries_every_key_of_4_0_7() {
    let vault = vault_of_4_0_7();
    let space = space_on(&vault).await;

    for table in space.sync.registry().tables() {
        let entity = table.spec.entity;
        if golden::NEW_IN_5.contains(&entity) {
            continue;
        }
        let published: BTreeMap<String, Value> = rows::read_rows(&space.core.db, table)
            .await
            .unwrap()
            .into_iter()
            .collect();
        for ((_, id), op) in latest(golden::ops(entity)) {
            // The default workspace of this device is its own (see above).
            if op.deleted || head_of(&space, &op).await.as_ref() != Some(&op.hlc) {
                continue;
            }
            let ours = published
                .get(&id)
                .unwrap_or_else(|| panic!("{entity} {id} is not published"));
            assert_eq!(ours, &op.payload, "{entity} {id}");
        }
    }

    // Space's own changes of the types with a handler, puts then tombstones.
    settings::set(
        &space.core.db,
        "notes_attachment_policy",
        "{\"large_files_enabled\":true,\"threshold_mib\":1,\"max_file_gib\":10,\"download_on_sync\":true,\"ask_above_mib\":16}",
    )
    .await
    .unwrap();
    let attachments = documents(&space).join("attachments");
    write(
        &documents(&space).join("note-1.md"),
        b"---\nid: note-1\ntitle: \"Deploy\"\nformat: md\nbindings: []\ntags: []\ncreated_at: 2026-09-01T10:00:00+00:00\nupdated_at: 2026-10-01T10:00:00+00:00\n---\nText of Deploy, edited here.\n",
    );
    write(
        &attachments.join("note-1").join("small.txt"),
        b"a small one",
    );
    write(
        &attachments.join("note-1").join("large.txt"),
        &"a large attachment line\n".repeat(50_000).into_bytes(),
    );
    crate::sync::profile_files::acquire_lease(&space.core, "pr-1")
        .await
        .unwrap();
    let profile_path: String =
        sqlx::query_scalar("SELECT profile_path FROM profiles WHERE id = 'pr-1'")
            .fetch_one(&space.core.db)
            .await
            .unwrap();
    write(
        &Path::new(&profile_path)
            .join("firefox-profile")
            .join("prefs.js"),
        b"user_pref(\"browser.startup.page\", 1);\n",
    );
    sqlx::query("UPDATE sync_profile_files_state SET dirty = 1 WHERE profile_id = 'pr-1'")
        .execute(&space.core.db)
        .await
        .unwrap();
    cycle(&space).await;
    std::fs::remove_file(attachments.join("note-1").join("small.txt")).unwrap();
    std::fs::remove_file(attachments.join("note-1").join("large.txt")).unwrap();
    sqlx::query("UPDATE notes SET deleted = 1 WHERE id = 'note-1'")
        .execute(&space.core.db)
        .await
        .unwrap();
    crate::sync::profile_files::on_profile_stopped(&space.core, "pr-1")
        .await
        .unwrap();
    cycle(&space).await;

    let author = device_id(&space).await;
    let ours = ops_of_all(&space, &author).await;
    for entity in [
        "note",
        "note_attachment",
        "note_attachment_v2",
        "profile_lease",
        "profile_snapshot",
    ] {
        for deleted in [false, true] {
            let theirs: Vec<Op> = golden::ops(entity)
                .into_iter()
                .filter(|op| op.deleted == deleted)
                .collect();
            if theirs.is_empty() {
                continue;
            }
            let mine: Vec<&Op> = ours
                .iter()
                .filter(|op| op.entity_type == entity && op.deleted == deleted)
                .collect();
            assert!(
                !mine.is_empty(),
                "Space published no {entity} (deleted: {deleted})"
            );
            for op in &mine {
                for golden_op in &theirs {
                    if let Err(e) = carries_every_key(&op.payload, &golden_op.payload) {
                        panic!("{entity} (deleted: {deleted}): {e}");
                    }
                }
            }
        }
    }

    close(space).await;
    let _ = std::fs::remove_dir_all(&vault);
}

/// Every op `author` wrote to the vault, not only the newest of each entity.
async fn ops_of_all(state: &TestStates, author: &str) -> Vec<Op> {
    let engine = open_engine(&state.core).await.unwrap();
    let mut from_start = veydan_sync::LocalState::default();
    let mut ops = Vec::new();
    let reader = veydan_sync::Engine::with_key(
        Box::new(veydan_sync::LocalDir::new(
            veydan_sync_host::config::load_config(&state.core.db)
                .await
                .folder_path,
        )),
        &veydan_sync::Vmk::from_base64(
            &veydan_sync_host::config::load_binding(&state.core.db)
                .await
                .unwrap()
                .vmk_b64,
        )
        .unwrap(),
        engine.vault_id(),
        "a-reader-of-the-vault",
    );
    let pulled = reader.pull(&mut from_start, |_, _, _| {}).await.unwrap();
    ops.extend(
        pulled
            .ops
            .into_iter()
            .filter(|op| op.hlc.device_id == author),
    );
    ops
}

/// What a device pushes for a given local state is what 4.0.7 pushes for
/// it, op for op and in the same order: `collect.json` is what 4.0.7
/// collected from the data file of `collect_tests`, `collect.golden.json`
/// what Space collects. One difference is deliberate (platform stage 1,
/// `f9f9baf`): the flags of a note that has none set and no folder are not
/// published, so later ops of the first cycle take a counter less. The ops of
/// the types 5.x added, which 4.0.7 skips, come after all of them.
#[test]
fn a_cycle_pushes_what_4_0_7_pushes() {
    let theirs = read_json("collect.json");
    let ours: Value = serde_json::from_str(include_str!("collect.golden.json")).unwrap();
    let unset_flags = |op: &Value| {
        op["entity_type"] == "note_meta"
            && op["payload"] == json!({ "pinned": 0, "archived": 0, "folder_ids": [] })
    };
    let without_clock = |op: &Value| {
        let mut op = op.clone();
        op.as_object_mut().unwrap().remove("hlc");
        op
    };
    let rounds = |file: &Value, skip: &dyn Fn(&Value) -> bool| -> Vec<Vec<Value>> {
        file["rounds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|round| {
                round["ops"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|op| !skip(op))
                    .map(without_clock)
                    .collect()
            })
            .collect()
    };
    let theirs_rounds = rounds(&theirs, &unset_flags);
    let new_in_5 = |op: &Value| {
        golden::NEW_IN_5
            .iter()
            .any(|entity| op["entity_type"] == *entity)
    };
    let ours_rounds = rounds(&ours, &new_in_5);
    assert_eq!(theirs_rounds.len(), 2);
    for (round, (t, o)) in theirs_rounds.iter().zip(&ours_rounds).enumerate() {
        assert_eq!(t, o, "cycle {round}");
    }
    let skipped: usize = theirs["rounds"][0]["ops"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|op| unset_flags(op))
        .count();
    assert_eq!(
        skipped, 1,
        "4.0.7 published the flags of one note with none set"
    );
}
