// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A device joins a vault that already holds a key row. The join is
//! `veydan_sync_host::join`; the states and the passwords are Space's.

use crate::error::{AppError, CmdResult};
use crate::modules::{TestStates, SYNC_PLAN};
use std::path::{Path, PathBuf};
use veydan_core::settings;
use veydan_lock::{SyncedLock, ERR_JOIN_OWN_LOCK};
use veydan_sync::{Engine, HlcClock};
use veydan_sync_host::config::{self, build_storage, load_binding, load_config};
use veydan_sync_host::{join, open_engine, rows, state, Step, Table};

/// The tables of the step of the cycle that carries the key row.
fn rows_of<'s>(state: &'s TestStates, steps: &[Step]) -> Vec<&'s Table> {
    state.sync.registry().step_rows(steps, "app rows")
}

const PASSPHRASE: &str = "correct horse battery";
const PIN: &str = "4821";
const OWN_PIN: &str = "7777";

/// A started device pointed at the vault's folder, no lock set.
async fn device(vault_dir: &Path) -> TestStates {
    let state = TestStates::new().await;
    let db = &state.core.db;
    settings::set(db, "sync_backend", "folder").await.unwrap();
    settings::set(db, "sync_folder_path", &vault_dir.to_string_lossy())
        .await
        .unwrap();
    state.lock.open_default().await.unwrap();
    state
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

/// The row streams of one cycle in the order `cycle_inner` runs them, then
/// what the finish of the rows does about the key row.
async fn cycle(state: &TestStates) {
    run(state, true).await;
}

/// A cycle; `applied` is false for one that ends in a retry, which is
/// every step but the saving of the peer heads.
async fn run(state: &TestStates, applied: bool) {
    let db = &state.core.db;
    let engine = open_engine(&state.core).await.unwrap();
    let (mut local, last_hlc) = state::load_local_state(db).await.unwrap();
    let mut clock = HlcClock::new(engine.device_id().to_string(), last_hlc.as_ref());
    if join::settle_join_key(&state.host(), &engine, &state.core, state.sync.registry())
        .await
        .unwrap()
    {
        key_row_changed(state).await;
    }
    let changes = rows::collect_local_changes(
        &state.core,
        &mut clock,
        &rows_of(state, SYNC_PLAN.collect),
        None,
    )
    .await
    .unwrap();
    if !changes.ops.is_empty() {
        engine.push(&mut local, changes.ops).await.unwrap();
        state::save_own_state(db, &local, &clock.last())
            .await
            .unwrap();
        for st in &changes.states {
            state::save_row_state(db, st).await.unwrap();
        }
    }
    let pulled = engine.pull(&mut local, |_, _, _| {}).await.unwrap();
    assert!(pulled.errors.is_empty(), "{:?}", pulled.errors);
    for op in &pulled.ops {
        clock.observe(&op.hlc);
    }
    let outcome = rows::apply_ops(
        &state.host(),
        &state.core,
        &rows_of(state, SYNC_PLAN.apply),
        &pulled.ops,
    )
    .await
    .unwrap();
    assert_eq!(outcome.retry, None);
    if outcome.changed.contains("password_vault") {
        key_row_changed(state).await;
    }
    state::save_own_state(db, &local, &clock.last())
        .await
        .unwrap();
    if applied {
        state::save_peer_heads(db, &local).await.unwrap();
    }
}

/// `join::key_row_changed` without the app.
async fn key_row_changed(state: &TestStates) {
    state.lock.refresh_after_sync().await;
}

/// `sync_join_vault` without the app.
async fn join_vault(state: &TestStates) -> CmdResult<()> {
    if join::join(
        &state.host(),
        &state.core,
        state.sync.registry(),
        PASSPHRASE,
    )
    .await?
    {
        key_row_changed(state).await;
    }
    Ok(())
}

/// How a 4.0.7 client joins: it binds, and its first cycle pushes what it has.
async fn join_as_4_0_7(state: &TestStates) {
    let db = &state.core.db;
    let storage = build_storage(&load_config(db).await).unwrap();
    let (engine, vmk) = Engine::open(storage, PASSPHRASE, "unbound").await.unwrap();
    join::bind(db, state.sync.registry(), engine.vault_id(), &vmk)
        .await
        .unwrap();
}

/// Whose op is the latest for the key row in the vault.
async fn key_row_author(state: &TestStates) -> String {
    let engine = open_engine(&state.core).await.unwrap();
    let ops = engine.all_latest_ops().await.unwrap();
    let op = ops
        .iter()
        .find(|op| op.entity_type == "password_vault")
        .expect("key row in the vault");
    op.hlc.device_id.clone()
}

/// A password as `password_create` stores it.
async fn save_password(state: &TestStates, id: &str, secret: &str) {
    let (key, vault_id) = state.lock.ensure_key().await.unwrap();
    let enc = veydan_lock::encrypt_field(&key, id, "password", secret).unwrap();
    sqlx::query(
        "INSERT INTO passwords (id, title, password_enc, vault_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, '2026-09-01T10:00:00+00:00', '2026-09-01T10:00:00+00:00')",
    )
    .bind(id)
    .bind(id)
    .bind(enc)
    .bind(vault_id)
    .execute(&state.core.db)
    .await
    .unwrap();
}

/// The secret as `password_reveal` returns it.
async fn reveal(state: &TestStates, id: &str) -> Result<String, AppError> {
    let (key, vault_id) = state.lock.require_open()?;
    let (row_vault, enc): (String, String) =
        sqlx::query_as("SELECT vault_id, password_enc FROM passwords WHERE id = ?")
            .bind(id)
            .fetch_one(&state.core.db)
            .await
            .map_err(AppError::db)?;
    if row_vault != vault_id {
        return Err(AppError::DecryptFailed);
    }
    Ok(veydan_lock::decrypt_field(&key, id, "password", &enc)?)
}

/// A secret as the messenger stores it: in its box of the lock, under the
/// vault key, on this device only.
async fn put_secret(state: &TestStates, name: &str, value: &str) {
    state
        .lock
        .secret_box("messenger")
        .put(name, value.as_bytes())
        .await
        .unwrap();
}

async fn get_secret(state: &TestStates, name: &str) -> Result<String, AppError> {
    let stored = state.lock.secret_box("messenger").get(name).await?;
    let stored = stored.expect("stored secret");
    Ok(String::from_utf8(stored.to_vec()).unwrap())
}

async fn key_id(state: &TestStates) -> Option<String> {
    sqlx::query_scalar("SELECT vault_id FROM password_vault")
        .fetch_optional(&state.core.db)
        .await
        .unwrap()
}

/// The hash of the lock the key row names.
async fn lock_hash(state: &TestStates) -> Option<String> {
    match veydan_lock::synced_lock(&state.core.db).await.unwrap() {
        SyncedLock::Meta(meta) => Some(meta.hash),
        _ => None,
    }
}

/// The device that owns the vault: one password under the vault's key,
/// behind `PIN` when `locked`. Returns the device and the id of that key.
async fn owner(vault_dir: &Path, locked: bool) -> (TestStates, String) {
    let state = device(vault_dir).await;
    if locked {
        state
            .lock
            .set(Some(PIN.into()), None, Some("pin".into()), None)
            .await
            .unwrap();
    }
    save_password(&state, "pw-old", "kept since 4.x").await;
    create_vault(&state).await;
    cycle(&state).await;
    let key = key_id(&state).await.expect("key row");
    (state, key)
}

async fn close(devices: Vec<TestStates>, vault_dir: PathBuf) {
    for state in devices {
        state.core.db.close().await;
        let _ = std::fs::remove_dir_all(&state.core.app_data_dir);
    }
    let _ = std::fs::remove_dir_all(&vault_dir);
}

#[tokio::test]
async fn a_device_without_a_key_row_takes_the_vault_s() {
    let dir = crate::db::test_dir();
    let (owner, key) = owner(&dir, false).await;
    let joiner = device(&dir).await;

    join_vault(&joiner).await.unwrap();
    cycle(&joiner).await;
    cycle(&owner).await;

    assert_eq!(key_id(&owner).await, Some(key.clone()));
    assert_eq!(key_id(&joiner).await, Some(key));
    assert_eq!(reveal(&owner, "pw-old").await.unwrap(), "kept since 4.x");
    assert_eq!(reveal(&joiner, "pw-old").await.unwrap(), "kept since 4.x");

    close(vec![owner, joiner], dir).await;
}

#[tokio::test]
async fn messenger_secrets_stored_before_the_join_move_to_the_vault_s_key() {
    let dir = crate::db::test_dir();
    let (owner, key) = owner(&dir, false).await;
    let joiner = device(&dir).await;
    put_secret(&joiner, "nsec", "the messenger key").await;

    join_vault(&joiner).await.unwrap();
    cycle(&joiner).await;
    cycle(&owner).await;
    cycle(&joiner).await;

    assert_eq!(
        key_id(&owner).await,
        Some(key.clone()),
        "the vault's key was replaced"
    );
    assert_eq!(key_id(&joiner).await, Some(key));
    assert_eq!(reveal(&owner, "pw-old").await.unwrap(), "kept since 4.x");
    assert_eq!(reveal(&joiner, "pw-old").await.unwrap(), "kept since 4.x");
    assert_eq!(
        get_secret(&joiner, "nsec").await.unwrap(),
        "the messenger key"
    );

    close(vec![owner, joiner], dir).await;
}

#[tokio::test]
async fn a_lock_set_before_the_join_gives_way_to_the_vault_s() {
    let dir = crate::db::test_dir();
    let (owner, key) = owner(&dir, true).await;
    let joiner = device(&dir).await;
    joiner
        .lock
        .set(Some(OWN_PIN.into()), None, Some("pin".into()), None)
        .await
        .unwrap();

    join_vault(&joiner).await.unwrap();
    cycle(&joiner).await;
    cycle(&owner).await;

    assert_eq!(
        key_id(&owner).await,
        Some(key.clone()),
        "the vault's key was replaced"
    );
    assert_eq!(key_id(&joiner).await, Some(key));
    assert_eq!(reveal(&owner, "pw-old").await.unwrap(), "kept since 4.x");
    assert_eq!(lock_hash(&joiner).await, lock_hash(&owner).await);
    assert!(joiner.lock.unlock(OWN_PIN).await.is_err());
    assert!(joiner.lock.unlock(PIN).await.unwrap());
    assert_eq!(reveal(&joiner, "pw-old").await.unwrap(), "kept since 4.x");

    close(vec![owner, joiner], dir).await;
}

#[tokio::test]
async fn a_lock_set_before_the_join_goes_when_the_vault_s_row_names_none() {
    let dir = crate::db::test_dir();
    // A key row as the messenger leaves it on a device without a lock:
    // wrapped with the built-in secret, the lock fields empty.
    let owner = device(&dir).await;
    put_secret(&owner, "nsec", "the messenger key").await;
    create_vault(&owner).await;
    cycle(&owner).await;
    let key = key_id(&owner).await.expect("key row");
    let joiner = device(&dir).await;
    joiner
        .lock
        .set(Some(OWN_PIN.into()), None, Some("pin".into()), None)
        .await
        .unwrap();

    join_vault(&joiner).await.unwrap();
    cycle(&joiner).await;
    cycle(&owner).await;

    assert_eq!(key_id(&owner).await, Some(key.clone()));
    assert_eq!(
        get_secret(&owner, "nsec").await.unwrap(),
        "the messenger key"
    );
    // The lock wrapped a key that is gone; the vault's key is open without one.
    assert_eq!(lock_hash(&joiner).await, None);
    let (_, open_key) = joiner.lock.require_open().unwrap();
    assert_eq!(open_key, key);

    close(vec![owner, joiner], dir).await;
}

#[tokio::test]
async fn a_password_saved_before_the_join_is_read_on_every_device() {
    let dir = crate::db::test_dir();
    let (owner, key) = owner(&dir, true).await;
    let joiner = device(&dir).await;
    save_password(&joiner, "pw-new", "saved in 5.0").await;
    put_secret(&joiner, "nsec", "the messenger key").await;

    join_vault(&joiner).await.unwrap();
    cycle(&joiner).await;
    cycle(&owner).await;
    assert_eq!(
        key_id(&owner).await,
        Some(key.clone()),
        "the vault's key was replaced"
    );
    assert_eq!(reveal(&owner, "pw-old").await.unwrap(), "kept since 4.x");

    // The vault's lock is this device's lock now; its secret opens both keys.
    assert!(joiner.lock.unlock(PIN).await.unwrap());
    assert_eq!(reveal(&joiner, "pw-old").await.unwrap(), "kept since 4.x");
    assert_eq!(reveal(&joiner, "pw-new").await.unwrap(), "saved in 5.0");
    assert_eq!(
        get_secret(&joiner, "nsec").await.unwrap(),
        "the messenger key"
    );

    cycle(&joiner).await;
    cycle(&owner).await;
    assert_eq!(key_id(&owner).await, Some(key));
    assert_eq!(reveal(&owner, "pw-new").await.unwrap(), "saved in 5.0");

    close(vec![owner, joiner], dir).await;
}

#[tokio::test]
async fn nothing_of_a_joining_device_is_pushed_over_the_vault_s_key_row() {
    let dir = crate::db::test_dir();
    let (owner, _) = owner(&dir, true).await;
    let owner_id = config::device_id(&owner.core.db).await.unwrap();
    let joiner = device(&dir).await;
    save_password(&joiner, "pw-new", "saved in 5.0").await;

    join_vault(&joiner).await.unwrap();
    cycle(&joiner).await;
    assert_eq!(key_row_author(&joiner).await, owner_id);

    // Not after the unlock that moves the local password to the vault's key either.
    assert!(joiner.lock.unlock(PIN).await.unwrap());
    cycle(&joiner).await;
    assert_eq!(key_row_author(&joiner).await, owner_id);

    close(vec![owner, joiner], dir).await;
}

#[tokio::test]
async fn a_join_is_refused_while_a_lock_of_the_device_guards_its_data() {
    let dir = crate::db::test_dir();
    let (owner, key) = owner(&dir, true).await;
    let joiner = device(&dir).await;
    joiner
        .lock
        .set(Some(OWN_PIN.into()), None, Some("pin".into()), None)
        .await
        .unwrap();
    save_password(&joiner, "pw-new", "saved in 5.0").await;
    let own_key = key_id(&joiner).await;

    let refused = join_vault(&joiner).await.unwrap_err();
    assert_eq!(refused.to_string(), ERR_JOIN_OWN_LOCK);
    assert!(load_binding(&joiner.core.db).await.is_none());
    assert_eq!(key_id(&joiner).await, own_key);
    assert_eq!(reveal(&joiner, "pw-new").await.unwrap(), "saved in 5.0");
    cycle(&owner).await;
    assert_eq!(key_id(&owner).await, Some(key.clone()));

    // With the lock off the key of the device can be opened later, and the join goes through.
    joiner
        .lock
        .set(None, Some(OWN_PIN.into()), None, None)
        .await
        .unwrap();
    join_vault(&joiner).await.unwrap();
    cycle(&joiner).await;
    assert!(joiner.lock.unlock(PIN).await.unwrap());
    assert_eq!(key_id(&joiner).await, Some(key));
    assert_eq!(reveal(&joiner, "pw-new").await.unwrap(), "saved in 5.0");
    assert_eq!(reveal(&joiner, "pw-old").await.unwrap(), "kept since 4.x");

    close(vec![owner, joiner], dir).await;
}

#[tokio::test]
async fn a_device_with_data_joins_a_vault_a_fresh_device_created() {
    let dir = crate::db::test_dir();
    // The fresh device holds a messenger secret, so the vault has its key row.
    let creator = device(&dir).await;
    put_secret(&creator, "nsec", "the messenger key").await;
    create_vault(&creator).await;
    cycle(&creator).await;
    let key = key_id(&creator).await.expect("key row");

    let joiner = device(&dir).await;
    save_password(&joiner, "pw-old", "kept since 4.x").await;
    join_vault(&joiner).await.unwrap();
    cycle(&joiner).await;
    cycle(&creator).await;

    assert_eq!(key_id(&creator).await, Some(key.clone()));
    assert_eq!(key_id(&joiner).await, Some(key));
    assert_eq!(
        get_secret(&creator, "nsec").await.unwrap(),
        "the messenger key"
    );
    assert_eq!(reveal(&creator, "pw-old").await.unwrap(), "kept since 4.x");
    assert_eq!(reveal(&joiner, "pw-old").await.unwrap(), "kept since 4.x");

    close(vec![creator, joiner], dir).await;
}

#[tokio::test]
async fn a_key_row_a_4_0_7_client_pushed_over_this_one_loses_nothing_here() {
    let dir = crate::db::test_dir();
    let creator = device(&dir).await;
    put_secret(&creator, "nsec", "the messenger key").await;
    save_password(&creator, "pw-new", "saved in 5.0").await;
    create_vault(&creator).await;
    cycle(&creator).await;

    let old_client = device(&dir).await;
    save_password(&old_client, "pw-old", "kept since 4.x").await;
    let key = key_id(&old_client).await.expect("key row");
    join_as_4_0_7(&old_client).await;
    cycle(&old_client).await;

    // Its row is the newer one and wins here; what this key encrypted moves to that row's key.
    cycle(&creator).await;
    assert_eq!(key_id(&creator).await, Some(key.clone()));
    assert_eq!(
        get_secret(&creator, "nsec").await.unwrap(),
        "the messenger key"
    );
    assert_eq!(reveal(&creator, "pw-new").await.unwrap(), "saved in 5.0");
    assert_eq!(reveal(&creator, "pw-old").await.unwrap(), "kept since 4.x");

    cycle(&creator).await;
    cycle(&old_client).await;
    assert_eq!(key_id(&old_client).await, Some(key));
    assert_eq!(reveal(&old_client, "pw-new").await.unwrap(), "saved in 5.0");
    assert_eq!(
        reveal(&old_client, "pw-old").await.unwrap(),
        "kept since 4.x"
    );

    close(vec![creator, old_client], dir).await;
}

#[tokio::test]
async fn a_key_row_made_while_the_first_cycle_runs_does_not_replace_the_vault_s() {
    let dir = crate::db::test_dir();
    let (owner, key) = owner(&dir, false).await;
    let joiner = device(&dir).await;

    // No key row at the join; the messenger stores its secret before the first pull.
    join_vault(&joiner).await.unwrap();
    put_secret(&joiner, "nsec", "the messenger key").await;
    assert_ne!(key_id(&joiner).await, Some(key.clone()));
    cycle(&joiner).await;
    cycle(&owner).await;

    assert_eq!(key_id(&owner).await, Some(key.clone()));
    assert_eq!(key_id(&joiner).await, Some(key));
    assert_eq!(reveal(&owner, "pw-old").await.unwrap(), "kept since 4.x");
    assert_eq!(
        get_secret(&joiner, "nsec").await.unwrap(),
        "the messenger key"
    );

    close(vec![owner, joiner], dir).await;
}

#[tokio::test]
async fn the_key_row_of_a_device_that_joins_a_vault_without_one_is_published() {
    let dir = crate::db::test_dir();
    // The owner never used the lock, passwords or the messenger: no key row.
    let owner = device(&dir).await;
    create_vault(&owner).await;
    cycle(&owner).await;
    let joiner = device(&dir).await;
    save_password(&joiner, "pw-new", "saved in 5.0").await;
    let key = key_id(&joiner).await;

    join_vault(&joiner).await.unwrap();
    cycle(&joiner).await;
    cycle(&joiner).await;
    cycle(&owner).await;

    assert_eq!(key_id(&owner).await, key);
    assert_eq!(reveal(&owner, "pw-new").await.unwrap(), "saved in 5.0");

    close(vec![owner, joiner], dir).await;
}

#[tokio::test]
async fn a_key_row_pushed_over_a_device_open_behind_its_lock_loses_nothing_here() {
    let dir = crate::db::test_dir();
    let creator = device(&dir).await;
    creator
        .lock
        .set(Some(PIN.into()), None, Some("pin".into()), None)
        .await
        .unwrap();
    put_secret(&creator, "nsec", "the messenger key").await;
    save_password(&creator, "pw-new", "saved in 5.0").await;
    create_vault(&creator).await;
    cycle(&creator).await;

    let old_client = device(&dir).await;
    save_password(&old_client, "pw-old", "kept since 4.x").await;
    let key = key_id(&old_client).await.expect("key row");
    join_as_4_0_7(&old_client).await;
    cycle(&old_client).await;

    // The row names no lock and another key; the session here holds the key it replaces.
    assert_eq!(reveal(&creator, "pw-new").await.unwrap(), "saved in 5.0");
    cycle(&creator).await;
    assert_eq!(key_id(&creator).await, Some(key));
    assert_eq!(lock_hash(&creator).await, None);
    assert_eq!(
        get_secret(&creator, "nsec").await.unwrap(),
        "the messenger key"
    );
    assert_eq!(reveal(&creator, "pw-new").await.unwrap(), "saved in 5.0");
    assert_eq!(reveal(&creator, "pw-old").await.unwrap(), "kept since 4.x");

    cycle(&creator).await;
    cycle(&old_client).await;
    assert_eq!(reveal(&old_client, "pw-new").await.unwrap(), "saved in 5.0");

    close(vec![creator, old_client], dir).await;
}

#[tokio::test]
async fn the_key_row_of_a_joining_device_is_published_while_cycles_end_in_a_retry() {
    let dir = crate::db::test_dir();
    let owner = device(&dir).await;
    create_vault(&owner).await;
    cycle(&owner).await;
    let joiner = device(&dir).await;
    save_password(&joiner, "pw-new", "saved in 5.0").await;
    let key = key_id(&joiner).await;

    join_vault(&joiner).await.unwrap();
    run(&joiner, false).await;
    cycle(&owner).await;

    assert_eq!(key_id(&owner).await, key);
    assert_eq!(reveal(&owner, "pw-new").await.unwrap(), "saved in 5.0");

    close(vec![owner, joiner], dir).await;
}

#[tokio::test]
async fn a_lock_changed_here_and_not_pushed_yet_stands_when_the_device_joins_again() {
    let dir = crate::db::test_dir();
    let (owner, key) = owner(&dir, true).await;
    let joiner = device(&dir).await;
    join_vault(&joiner).await.unwrap();
    cycle(&joiner).await;
    assert!(joiner.lock.unlock(PIN).await.unwrap());
    joiner
        .lock
        .set(
            Some(OWN_PIN.into()),
            Some(PIN.into()),
            Some("pin".into()),
            None,
        )
        .await
        .unwrap();

    join_vault(&joiner).await.unwrap();
    joiner.lock.lock();
    assert!(joiner.lock.unlock(PIN).await.is_err());
    assert!(joiner.lock.unlock(OWN_PIN).await.unwrap());
    assert_eq!(reveal(&joiner, "pw-old").await.unwrap(), "kept since 4.x");

    // The change reaches the vault as it would have without the second join.
    cycle(&joiner).await;
    cycle(&owner).await;
    assert_eq!(key_id(&owner).await, Some(key));
    assert_eq!(lock_hash(&owner).await, lock_hash(&joiner).await);
    assert!(owner.lock.unlock(OWN_PIN).await.unwrap());
    assert_eq!(reveal(&owner, "pw-old").await.unwrap(), "kept since 4.x");

    close(vec![owner, joiner], dir).await;
}

#[tokio::test]
async fn turning_off_the_lock_of_a_vault_left_behind_opens_what_this_device_kept() {
    let dir = crate::db::test_dir();
    let (owner, _) = owner(&dir, true).await;
    let joiner = device(&dir).await;
    put_secret(&joiner, "nsec", "the messenger key").await;
    // Joined, never unlocked with the vault's PIN, left.
    join_vault(&joiner).await.unwrap();
    cycle(&joiner).await;
    join::leave(&joiner.core.db, joiner.sync.registry())
        .await
        .unwrap();

    let other_dir = crate::db::test_dir();
    let (other, other_key) = self::owner(&other_dir, false).await;
    settings::set(
        &joiner.core.db,
        "sync_folder_path",
        &other_dir.to_string_lossy(),
    )
    .await
    .unwrap();
    let refused = join_vault(&joiner).await.unwrap_err();
    assert_eq!(refused.to_string(), ERR_JOIN_OWN_LOCK);

    joiner
        .lock
        .set(None, Some(PIN.into()), None, None)
        .await
        .unwrap();
    assert_eq!(
        get_secret(&joiner, "nsec").await.unwrap(),
        "the messenger key"
    );
    join_vault(&joiner).await.unwrap();
    cycle(&joiner).await;
    assert_eq!(key_id(&joiner).await, Some(other_key));
    assert_eq!(
        get_secret(&joiner, "nsec").await.unwrap(),
        "the messenger key"
    );

    close(vec![owner, joiner], dir).await;
    close(vec![other], other_dir).await;
}

/// A device behind `PIN` whose key row a 4.0.7 client replaced with one
/// that names no lock, while the device was locked. Returns the device,
/// the client and the recovery key the device got with its lock.
async fn replaced_while_locked(dir: &Path) -> (TestStates, TestStates, String) {
    let creator = device(dir).await;
    let recovery = creator
        .lock
        .set(Some(PIN.into()), None, Some("pin".into()), None)
        .await
        .unwrap()
        .expect("recovery key");
    put_secret(&creator, "nsec", "the messenger key").await;
    save_password(&creator, "pw-new", "saved in 5.0").await;
    create_vault(&creator).await;
    cycle(&creator).await;
    creator.lock.lock();

    let old_client = device(dir).await;
    save_password(&old_client, "pw-old", "kept since 4.x").await;
    join_as_4_0_7(&old_client).await;
    cycle(&old_client).await;
    cycle(&creator).await;

    // The vault's key is open without a lock; what the PIN guarded is not.
    assert_eq!(lock_hash(&creator).await, None);
    assert_eq!(reveal(&creator, "pw-old").await.unwrap(), "kept since 4.x");
    assert!(get_secret(&creator, "nsec").await.is_err());
    assert!(reveal(&creator, "pw-new").await.is_err());
    assert_eq!(creator.lock.closed_keys().await.unwrap(), 1);
    (creator, old_client, recovery)
}

#[tokio::test]
async fn a_key_replaced_while_the_device_was_locked_opens_with_the_pin_it_had() {
    let dir = crate::db::test_dir();
    let (creator, old_client, _) = replaced_while_locked(&dir).await;

    assert!(creator.lock.open_replaced_keys(OWN_PIN).await.is_err());
    assert_eq!(creator.lock.closed_keys().await.unwrap(), 1);
    assert_eq!(creator.lock.open_replaced_keys(PIN).await.unwrap(), 0);

    assert_eq!(creator.lock.closed_keys().await.unwrap(), 0);
    assert_eq!(
        get_secret(&creator, "nsec").await.unwrap(),
        "the messenger key"
    );
    assert_eq!(reveal(&creator, "pw-new").await.unwrap(), "saved in 5.0");
    cycle(&creator).await;
    cycle(&old_client).await;
    assert_eq!(reveal(&old_client, "pw-new").await.unwrap(), "saved in 5.0");

    close(vec![creator, old_client], dir).await;
}

#[tokio::test]
async fn a_key_replaced_while_the_device_was_locked_opens_with_its_recovery_key() {
    let dir = crate::db::test_dir();
    let (creator, old_client, recovery) = replaced_while_locked(&dir).await;

    assert_eq!(creator.lock.open_replaced_keys(&recovery).await.unwrap(), 0);

    assert_eq!(
        get_secret(&creator, "nsec").await.unwrap(),
        "the messenger key"
    );
    assert_eq!(reveal(&creator, "pw-new").await.unwrap(), "saved in 5.0");

    close(vec![creator, old_client], dir).await;
}

#[tokio::test]
async fn a_key_replaced_by_a_row_behind_another_lock_moves_at_the_next_unlock() {
    let dir = crate::db::test_dir();
    let creator = device(&dir).await;
    creator
        .lock
        .set(Some(PIN.into()), None, Some("pin".into()), None)
        .await
        .unwrap();
    put_secret(&creator, "nsec", "the messenger key").await;
    create_vault(&creator).await;
    cycle(&creator).await;

    let other = device(&dir).await;
    other
        .lock
        .set(Some(OWN_PIN.into()), None, Some("pin".into()), None)
        .await
        .unwrap();
    save_password(&other, "pw-old", "kept since 4.x").await;
    let key = key_id(&other).await.expect("key row");
    join_as_4_0_7(&other).await;
    cycle(&other).await;

    // The session here is open; the row that comes is behind the other PIN.
    cycle(&creator).await;
    assert_eq!(key_id(&creator).await, Some(key));
    assert!(get_secret(&creator, "nsec").await.is_err());
    assert!(creator.lock.unlock(PIN).await.is_err());
    assert!(creator.lock.unlock(OWN_PIN).await.unwrap());
    assert_eq!(
        get_secret(&creator, "nsec").await.unwrap(),
        "the messenger key"
    );
    assert_eq!(reveal(&creator, "pw-old").await.unwrap(), "kept since 4.x");
    assert_eq!(creator.lock.closed_keys().await.unwrap(), 0);

    close(vec![creator, other], dir).await;
}

#[tokio::test]
async fn a_key_row_made_after_the_join_is_published_when_the_vault_has_none() {
    let dir = crate::db::test_dir();
    let owner = device(&dir).await;
    create_vault(&owner).await;
    cycle(&owner).await;
    let joiner = device(&dir).await;

    join_vault(&joiner).await.unwrap();
    save_password(&joiner, "pw-new", "saved in 5.0").await;
    let key = key_id(&joiner).await;
    run(&joiner, false).await;
    cycle(&owner).await;

    assert_eq!(key_id(&owner).await, key);
    assert_eq!(reveal(&owner, "pw-new").await.unwrap(), "saved in 5.0");

    close(vec![owner, joiner], dir).await;
}

#[tokio::test]
async fn a_key_row_made_after_the_join_waits_while_the_vault_cannot_be_read() {
    let dir = crate::db::test_dir();
    let (owner, _) = owner(&dir, false).await;
    let joiner = device(&dir).await;
    join_vault(&joiner).await.unwrap();
    put_secret(&joiner, "nsec", "the messenger key").await;
    // A chunk of some device is listed and cannot be read.
    let log = dir.join("devices").join("another").join("log");
    std::fs::create_dir_all(&log).unwrap();
    std::fs::write(log.join("00000001.bin"), b"half a chunk").unwrap();

    let engine = open_engine(&joiner.core).await.unwrap();
    assert!(join::settle_join_key(
        &joiner.host(),
        &engine,
        &joiner.core,
        joiner.sync.registry()
    )
    .await
    .is_err());
    let mut clock = HlcClock::new(engine.device_id().to_string(), None);
    let changes = rows::collect_local_changes(
        &joiner.core,
        &mut clock,
        &rows_of(&joiner, SYNC_PLAN.collect),
        None,
    )
    .await
    .unwrap();
    assert!(changes
        .ops
        .iter()
        .all(|op| op.entity_type != "password_vault"));

    close(vec![owner, joiner], dir).await;
}

#[tokio::test]
async fn a_lock_changed_in_the_vault_while_the_device_was_away_is_taken_at_the_join() {
    let dir = crate::db::test_dir();
    let (owner, key) = owner(&dir, true).await;
    let owner_id = config::device_id(&owner.core.db).await.unwrap();
    let joiner = device(&dir).await;
    join_vault(&joiner).await.unwrap();
    cycle(&joiner).await;
    assert!(joiner.lock.unlock(PIN).await.unwrap());
    join::leave(&joiner.core.db, joiner.sync.registry())
        .await
        .unwrap();

    owner
        .lock
        .set(
            Some(OWN_PIN.into()),
            Some(PIN.into()),
            Some("pin".into()),
            None,
        )
        .await
        .unwrap();
    cycle(&owner).await;
    join_vault(&joiner).await.unwrap();
    cycle(&joiner).await;

    assert_eq!(key_row_author(&joiner).await, owner_id);
    assert_eq!(key_id(&joiner).await, Some(key));
    assert!(joiner.lock.unlock(PIN).await.is_err());
    assert!(joiner.lock.unlock(OWN_PIN).await.unwrap());
    assert_eq!(reveal(&joiner, "pw-old").await.unwrap(), "kept since 4.x");

    close(vec![owner, joiner], dir).await;
}
