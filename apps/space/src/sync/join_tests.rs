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
use veydan_sync_host::{join, open_engine, rows, state, Step, SyncConfig, Table};

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

/// The config of `state` with another folder, as the form sends it unsaved.
async fn pointed_at(state: &TestStates, dir: &Path) -> SyncConfig {
    SyncConfig {
        folder_path: dir.to_string_lossy().into_owned(),
        ..load_config(&state.core.db).await
    }
}

/// `sync_change_storage` without the app.
async fn change_storage(state: &TestStates, dir: &Path) -> &'static str {
    let cfg = pointed_at(state, dir).await;
    join::change_storage(&state.core.db, state.sync.registry(), cfg)
        .await
        .unwrap()
}

async fn joined_vault(state: &TestStates) -> Option<String> {
    load_binding(&state.core.db).await.map(|b| b.vault_id)
}

#[tokio::test]
async fn a_device_moved_to_an_empty_storage_fills_the_vault_it_creates_there() {
    let old = crate::db::test_dir();
    let new = crate::db::test_dir();
    let (mover, _) = owner(&old, false).await;
    let db = &mover.core.db;
    let old_vault = joined_vault(&mover).await;
    assert!(old_vault.is_some());

    // Somebody's files: the device stays where it is.
    let foreign = crate::db::test_dir();
    std::fs::write(foreign.join("report.txt"), b"not a vault").unwrap();
    assert_eq!(change_storage(&mover, &foreign).await, "foreign");
    assert_eq!(joined_vault(&mover).await, old_vault);
    assert_eq!(load_config(db).await.folder_path, old.to_string_lossy());

    // Its own vault at another address: the address alone is saved.
    let alias = old.join(".");
    assert_eq!(change_storage(&mover, &alias).await, "same");
    assert_eq!(joined_vault(&mover).await, old_vault);
    assert_eq!(load_config(db).await.folder_path, alias.to_string_lossy());
    assert!(load_config(db).await.enabled);

    // An empty storage: the device leaves its vault, and nothing syncs
    // until it has one there.
    assert_eq!(change_storage(&mover, &new).await, "empty");
    assert_eq!(joined_vault(&mover).await, None);
    let cfg = load_config(db).await;
    assert_eq!(cfg.folder_path, new.to_string_lossy());
    assert!(!cfg.enabled);

    // The vault it creates there gets everything the device holds.
    create_vault(&mover).await;
    cycle(&mover).await;
    assert_ne!(joined_vault(&mover).await, old_vault);
    let joiner = device(&new).await;
    join_vault(&joiner).await.unwrap();
    cycle(&joiner).await;
    assert_eq!(reveal(&joiner, "pw-old").await.unwrap(), "kept since 4.x");
    // The old storage keeps its vault.
    assert!(old.join("manifest.json").exists());

    let _ = std::fs::remove_dir_all(&foreign);
    let _ = std::fs::remove_dir_all(&old);
    close(vec![mover, joiner], new).await;
}

#[tokio::test]
async fn a_device_moved_to_a_storage_with_a_vault_leaves_its_own_and_joins_that_one() {
    let old = crate::db::test_dir();
    let new = crate::db::test_dir();
    let (there, key) = owner(&new, false).await;
    let mover = device(&old).await;
    create_vault(&mover).await;
    cycle(&mover).await;
    let old_vault = joined_vault(&mover).await;

    assert_eq!(change_storage(&mover, &new).await, "vault");
    assert_eq!(joined_vault(&mover).await, None);
    join_vault(&mover).await.unwrap();
    cycle(&mover).await;

    assert_ne!(joined_vault(&mover).await, old_vault);
    assert_eq!(joined_vault(&mover).await, joined_vault(&there).await);
    assert_eq!(key_id(&mover).await, Some(key));
    assert_eq!(reveal(&mover, "pw-old").await.unwrap(), "kept since 4.x");

    let _ = std::fs::remove_dir_all(&old);
    close(vec![there, mover], new).await;
}

/// A whole cycle as the app runs it (`veydan_sync_host::cycle`): a device
/// that has just joined publishes nothing before it read the vault.
async fn whole_cycle(state: &TestStates) -> Vec<String> {
    state.sync.mark_ready();
    veydan_sync_host::cycle(state.host(), &state.core, &state.sync)
        .await
        .unwrap()
}

async fn password_title(state: &TestStates, id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT title FROM passwords WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.core.db)
        .await
        .unwrap()
}

/// The latest op of the vault for a row, over the logs that read.
async fn latest_op(state: &TestStates, entity: &str, id: &str) -> Option<veydan_sync::Op> {
    let engine = open_engine(&state.core).await.unwrap();
    engine
        .read_every_log()
        .await
        .unwrap()
        .ops
        .into_iter()
        .find(|op| op.entity_type == entity && op.entity_id == id)
}

/// Spec 9.3, "Leave, then join": what the other devices changed while this
/// one was away stands. Its own copies of those rows are older: published
/// with the clock of the join, they would undo the edit and bring the
/// deleted row back on every device.
#[tokio::test]
async fn a_device_that_joins_again_takes_what_changed_while_it_was_away() {
    let dir = crate::db::test_dir();
    let (owner, _) = owner(&dir, false).await;
    let away = device(&dir).await;
    join_vault(&away).await.unwrap();
    whole_cycle(&away).await;
    save_password(&owner, "pw-gone", "deleted while the device was away").await;
    whole_cycle(&owner).await;
    whole_cycle(&away).await;
    assert_eq!(
        password_title(&away, "pw-gone").await.as_deref(),
        Some("pw-gone")
    );
    join::leave(&away.core.db, away.sync.registry())
        .await
        .unwrap();

    sqlx::query(
        "UPDATE passwords SET title = 'edited while away', updated_at = '2026-09-03T09:00:00+00:00'
         WHERE id = 'pw-old'",
    )
    .execute(&owner.core.db)
    .await
    .unwrap();
    sqlx::query("DELETE FROM passwords WHERE id = 'pw-gone'")
        .execute(&owner.core.db)
        .await
        .unwrap();
    whole_cycle(&owner).await;
    let edit = latest_op(&owner, "password", "pw-old").await.unwrap().hlc;

    join_vault(&away).await.unwrap();
    whole_cycle(&away).await;
    whole_cycle(&owner).await;
    whole_cycle(&away).await;
    whole_cycle(&owner).await;

    for state in [&owner, &away] {
        assert_eq!(
            password_title(state, "pw-old").await.as_deref(),
            Some("edited while away")
        );
        assert_eq!(password_title(state, "pw-gone").await, None);
    }
    assert_eq!(
        latest_op(&owner, "password", "pw-old").await.unwrap().hlc,
        edit
    );
    assert!(
        latest_op(&owner, "password", "pw-gone")
            .await
            .unwrap()
            .deleted
    );
    assert_eq!(reveal(&away, "pw-old").await.unwrap(), "kept since 4.x");

    close(vec![owner, away], dir).await;
}

/// The vault deleted its key row: a device that joins with a key row of
/// its own gives it up and does not publish it, and what its key encrypted
/// is kept for the key it makes next.
#[tokio::test]
async fn a_key_row_the_vault_deleted_is_not_brought_back_by_a_join() {
    let dir = crate::db::test_dir();
    let (owner, vault_key) = owner(&dir, false).await;
    let owner_id = config::device_id(&owner.core.db).await.unwrap();
    sqlx::query("DELETE FROM password_vault")
        .execute(&owner.core.db)
        .await
        .unwrap();
    whole_cycle(&owner).await;
    assert!(
        latest_op(&owner, "password_vault", veydan_lock::ROW_ID)
            .await
            .unwrap()
            .deleted
    );

    let joiner = device(&dir).await;
    save_password(&joiner, "pw-new", "saved before the join").await;
    let own_key = key_id(&joiner).await.expect("a key row of its own");
    assert_ne!(own_key, vault_key);
    join_vault(&joiner).await.unwrap();
    assert_eq!(key_id(&joiner).await, None);
    whole_cycle(&joiner).await;
    whole_cycle(&joiner).await;
    whole_cycle(&owner).await;

    let op = latest_op(&joiner, "password_vault", veydan_lock::ROW_ID)
        .await
        .unwrap();
    assert!(op.deleted, "{:?}", op.payload);
    assert_eq!(op.hlc.device_id, owner_id);
    assert_eq!(key_id(&owner).await, None);
    // The next key of the device takes over what the old one encrypted.
    joiner.lock.ensure_key().await.unwrap();
    assert_ne!(key_id(&joiner).await, Some(own_key));
    assert_eq!(
        reveal(&joiner, "pw-new").await.unwrap(),
        "saved before the join"
    );

    close(vec![owner, joiner], dir).await;
}

/// `sync_join_vault` holds the slot of the cycle: a join while a cycle runs
/// waits for its end, or is refused, and a cycle of the old binding never
/// writes under the new one. The slot is free again for the first cycle.
#[tokio::test]
async fn a_join_waits_for_the_cycle_that_runs() {
    use std::time::Duration;
    let dir = crate::db::test_dir();
    let (owner, key) = owner(&dir, false).await;
    let joiner = device(&dir).await;
    let running = joiner.sync.pause(Duration::ZERO).await.expect("the slot");

    let refused = join::join_held(
        &joiner.host(),
        &joiner.core,
        &joiner.sync,
        PASSPHRASE,
        Duration::from_millis(300),
    )
    .await
    .unwrap_err();
    assert_eq!(refused.to_string(), join::ERR_SYNC_RUNNING);
    assert!(load_binding(&joiner.core.db).await.is_none());

    let host = joiner.host();
    let (joined, ()) = tokio::join!(
        join::join_held(
            &host,
            &joiner.core,
            &joiner.sync,
            PASSPHRASE,
            Duration::from_secs(20),
        ),
        async {
            tokio::time::sleep(Duration::from_millis(500)).await;
            assert!(
                load_binding(&joiner.core.db).await.is_none(),
                "the join waits for the cycle"
            );
            drop(running);
        }
    );
    if joined.unwrap() {
        key_row_changed(&joiner).await;
    }
    assert!(load_binding(&joiner.core.db).await.is_some());
    assert!(!joiner.sync.is_running());
    whole_cycle(&joiner).await;
    assert_eq!(key_id(&joiner).await, Some(key));
    assert_eq!(reveal(&joiner, "pw-old").await.unwrap(), "kept since 4.x");

    close(vec![owner, joiner], dir).await;
}

/// A join reads the whole vault. A log that is not all there yet (a chunk
/// listed after one that is not: a cloud client still delivering) refuses
/// the join with the words of a wait, and nothing changes here; once the
/// vault reads, the join goes through, and what only this device holds is
/// published after the first cycle has read the vault.
#[tokio::test]
async fn a_join_waits_until_the_whole_vault_can_be_read() {
    let dir = crate::db::test_dir();
    let (owner, key) = owner(&dir, false).await;
    let joiner = device(&dir).await;
    save_password(&joiner, "pw-new", "saved before the join").await;
    let own_key = key_id(&joiner).await;
    let another = dir.join("devices").join("another");
    std::fs::create_dir_all(another.join("log")).unwrap();
    // Its second chunk is listed, the first is not there yet.
    std::fs::write(another.join("log").join("00000002.bin"), b"a chunk").unwrap();

    let refused = join_vault(&joiner).await.unwrap_err();
    assert!(
        refused.to_string().starts_with(join::ERR_VAULT_UNREAD),
        "{refused}"
    );
    assert!(load_binding(&joiner.core.db).await.is_none());
    assert_eq!(key_id(&joiner).await, own_key);

    std::fs::remove_dir_all(&another).unwrap();
    join_vault(&joiner).await.unwrap();
    whole_cycle(&joiner).await;
    assert!(latest_op(&joiner, "password", "pw-new").await.is_none());
    whole_cycle(&joiner).await;
    whole_cycle(&owner).await;
    assert_eq!(key_id(&joiner).await, Some(key.clone()));
    assert_eq!(key_id(&owner).await, Some(key));
    assert_eq!(
        reveal(&owner, "pw-new").await.unwrap(),
        "saved before the join"
    );

    close(vec![owner, joiner], dir).await;
}

/// The first cycle after a join publishes nothing, and its pull does not
/// take the vault's older wrap of the same key over a lock changed here:
/// the change stands and reaches the vault with the next cycle.
#[tokio::test]
async fn a_lock_changed_here_stands_through_the_first_cycle_after_the_join() {
    let dir = crate::db::test_dir();
    let (owner, key) = owner(&dir, true).await;
    let joiner = device(&dir).await;
    join_vault(&joiner).await.unwrap();
    whole_cycle(&joiner).await;
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
    whole_cycle(&joiner).await;
    joiner.lock.lock();
    assert!(joiner.lock.unlock(PIN).await.is_err());
    assert!(joiner.lock.unlock(OWN_PIN).await.unwrap());
    whole_cycle(&joiner).await;
    whole_cycle(&owner).await;
    assert_eq!(key_id(&owner).await, Some(key));
    assert_eq!(lock_hash(&owner).await, lock_hash(&joiner).await);
    assert!(owner.lock.unlock(OWN_PIN).await.unwrap());
    assert_eq!(reveal(&owner, "pw-old").await.unwrap(), "kept since 4.x");

    close(vec![owner, joiner], dir).await;
}

/// What `sync_status` says of the first read of the vault.
async fn first_read_pending(state: &TestStates) -> bool {
    veydan_sync_host::status(&state.core, &state.sync)
        .await
        .unwrap()
        .first_read_pending
}

/// A log that does not open is no reason to keep a device out of the join:
/// it says which one. But the file may be a copy still on its way, and a
/// row whose only op lies past it has no state here yet: the log holds the
/// first read as any log a pull stops in, until a cycle reads past it.
#[tokio::test]
async fn a_log_broken_at_the_join_holds_the_first_read_until_a_pull_reads_past_it() {
    let dir = crate::db::test_dir();
    let (owner, key) = owner(&dir, false).await;
    let joiner = device(&dir).await;
    save_password(&joiner, "pw-new", "saved before the join").await;
    let another = dir.join("devices").join("another").join("log");
    std::fs::create_dir_all(&another).unwrap();
    std::fs::write(another.join("00000001.bin"), b"half a chunk").unwrap();

    join_vault(&joiner).await.unwrap();
    let warning = settings::get(&joiner.core.db, "sync_last_warning")
        .await
        .unwrap_or_default();
    assert!(warning.contains("another"), "{warning}");
    assert!(first_read_pending(&joiner).await);
    whole_cycle(&joiner).await;
    assert!(first_read_pending(&joiner).await, "the broken log holds it");
    assert!(latest_op(&joiner, "password", "pw-new").await.is_none());
    // The half chunk goes (its client is done with it): the wait ends.
    std::fs::remove_file(another.join("00000001.bin")).unwrap();
    whole_cycle(&joiner).await;
    assert!(!first_read_pending(&joiner).await);
    whole_cycle(&joiner).await;
    whole_cycle(&owner).await;
    assert_eq!(key_id(&joiner).await, Some(key));
    assert_eq!(
        reveal(&owner, "pw-new").await.unwrap(),
        "saved before the join"
    );

    close(vec![owner, joiner], dir).await;
}

/// A log that breaks after the join keeps the first read from ending. The
/// device does not wait for it for ever, sending nothing: after a few
/// cycles it says so and sends what it holds.
#[tokio::test]
async fn a_log_that_breaks_after_the_join_holds_the_device_back_a_few_cycles_only() {
    let dir = crate::db::test_dir();
    let (owner, _) = owner(&dir, false).await;
    let owner_id = config::device_id(&owner.core.db).await.unwrap();
    let joiner = device(&dir).await;
    save_password(&joiner, "pw-new", "saved before the join").await;
    join_vault(&joiner).await.unwrap();
    for chunk in std::fs::read_dir(dir.join("devices").join(&owner_id).join("log"))
        .unwrap()
        .flatten()
    {
        let mut bytes = std::fs::read(chunk.path()).unwrap();
        *bytes.last_mut().unwrap() ^= 0xff;
        std::fs::write(chunk.path(), bytes).unwrap();
    }

    for _ in 1..5 {
        let warnings = whole_cycle(&joiner).await;
        assert!(warnings.iter().all(|w| !w.contains("did not finish")));
        assert!(first_read_pending(&joiner).await);
        assert!(latest_op(&joiner, "password", "pw-new").await.is_none());
    }
    let warnings = whole_cycle(&joiner).await;
    assert!(
        warnings.iter().any(|w| w.contains("did not finish")),
        "{warnings:?}"
    );
    assert!(!first_read_pending(&joiner).await);
    whole_cycle(&joiner).await;
    assert!(latest_op(&joiner, "password", "pw-new").await.is_some());

    close(vec![owner, joiner], dir).await;
}

/// What this device changed while it was away from the vault is not lost
/// when it joins the same vault again: the leave kept what it last synced,
/// the edited row differs from it, and the edit goes out. A row it deleted
/// meanwhile takes the vault's again: a clear of this device alone deletes
/// nowhere else.
#[tokio::test]
async fn an_edit_made_while_away_from_the_vault_goes_out_at_the_join() {
    let dir = crate::db::test_dir();
    let (owner, _) = owner(&dir, false).await;
    save_password(&owner, "pw-kept", "kept everywhere").await;
    whole_cycle(&owner).await;
    let away = device(&dir).await;
    join_vault(&away).await.unwrap();
    whole_cycle(&away).await;
    whole_cycle(&away).await;
    assert_eq!(
        password_title(&away, "pw-kept").await.as_deref(),
        Some("pw-kept")
    );
    join::leave(&away.core.db, away.sync.registry())
        .await
        .unwrap();

    sqlx::query(
        "UPDATE passwords SET title = 'edited while away', updated_at = '2026-09-03T09:00:00+00:00'
         WHERE id = 'pw-old'",
    )
    .execute(&away.core.db)
    .await
    .unwrap();
    sqlx::query("DELETE FROM passwords WHERE id = 'pw-kept'")
        .execute(&away.core.db)
        .await
        .unwrap();
    join_vault(&away).await.unwrap();
    whole_cycle(&away).await;
    whole_cycle(&away).await;
    whole_cycle(&owner).await;

    for state in [&owner, &away] {
        assert_eq!(
            password_title(state, "pw-old").await.as_deref(),
            Some("edited while away")
        );
        assert_eq!(
            password_title(state, "pw-kept").await.as_deref(),
            Some("pw-kept")
        );
    }
    assert_eq!(reveal(&owner, "pw-old").await.unwrap(), "kept since 4.x");

    close(vec![owner, away], dir).await;
}
