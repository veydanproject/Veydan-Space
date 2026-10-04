// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Binding this device to a vault, and the key row of the lock a joining
//! device takes from it.

use crate::config::{self, build_storage, load_config, VaultBinding};
use crate::rows::{put_row, read_row, Put};
use crate::state::{self, load_row_state, save_row_state, RowSyncState};
use crate::system::VAULT_KEY_ENTITY;
use crate::{Host, Registry};
use serde_json::{Map, Value};
use std::collections::HashMap;
use veydan_core::{settings, AppError, CmdResult, Core};
use veydan_sync::{Engine, Vmk};

/// Open the vault in the configured storage and bind this device to it. The
/// vault is the authority for the password vault key: a device that has a key
/// row of its own takes the vault's before it is bound, or is refused. Only a
/// later wrap of the vault's own key stays, see `adopt_vault_key`.
/// Returns whether the key row was replaced.
pub async fn join(
    host: &Host<'_>,
    core: &Core,
    registry: &Registry,
    passphrase: &str,
) -> CmdResult<bool> {
    let db = &core.db;
    let cfg = load_config(db).await;
    let device = config::device_id(db).await?;
    let storage = build_storage(&cfg)?;
    let (engine, vmk) = Engine::open(storage, passphrase, &device)
        .await
        .map_err(AppError::other)?;
    let key_row = adopt_vault_key(host, &engine, core, registry, true).await?;
    bind(db, registry, engine.vault_id(), &vmk).await?;
    match &key_row {
        JoinKey::Vaults(st) => save_row_state(db, st).await?,
        // The vault was not read. A key row this device makes later waits
        // for `settle_join_key`: the vault may hold one.
        JoinKey::Unread => settings::set(db, config::JOIN_PENDING, "1").await?,
        JoinKey::Own => {}
    }
    Ok(matches!(key_row, JoinKey::Vaults(_)))
}

pub async fn bind(
    db: &sqlx::Pool<sqlx::Sqlite>,
    registry: &Registry,
    vault_id: &str,
    vmk: &Vmk,
) -> CmdResult<()> {
    // A fresh binding starts from scratch: no peer heads, no entity positions.
    state::clear_all_states(db, registry).await?;
    config::clear_binding(db, registry).await?;
    config::save_binding(
        db,
        &VaultBinding {
            vault_id: vault_id.to_string(),
            vmk_b64: vmk.to_base64(),
        },
    )
    .await?;
    settings::set(db, "sync_enabled", "1").await
}

/// `sync_leave` on the database alone.
pub async fn leave(db: &sqlx::Pool<sqlx::Sqlite>, registry: &Registry) -> CmdResult<()> {
    state::clear_all_states(db, registry).await?;
    config::clear_binding(db, registry).await?;
    settings::set(db, "sync_enabled", "0").await
}

/// The key row is another one now: the lock reopens or closes the vault in
/// memory, takes over the lock the row carries and tells whoever listens.
pub async fn key_row_changed(host: &Host<'_>) {
    if let Some(lock) = host.state::<veydan_lock::Lock>() {
        lock.refresh_after_sync().await;
    }
}

/// What became of the key row of a device that joins a vault.
pub enum JoinKey {
    /// The device has no key row, so the vault was not read.
    Unread,
    /// The device's row stays and is published like any row: the vault has
    /// none, or has an older wrap of the same key.
    Own,
    /// The vault's row took the place of the device's. The state records it
    /// as applied, so the row is not published.
    Vaults(RowSyncState),
}

/// Whether the key row `own` was written after `other`, by the `updated_at` both carry.
fn written_later(own: &Value, other: &Map<String, Value>) -> bool {
    let at = |stamp: Option<&Value>| {
        stamp
            .and_then(Value::as_str)
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
    };
    match (at(own.get("updated_at")), at(other.get("updated_at"))) {
        (Some(own), Some(other)) => own > other,
        _ => false,
    }
}

/// What a joining device does about the vault's key row, before the binding
/// and so before its first cycle. That cycle would push a local row that was
/// never synced as the newer one, and every device would lose the key its
/// passwords are encrypted with. The vault's row replaces the local one here
/// instead; its state is to be saved once the binding has cleared the old
/// states. Two wraps of the same key lose nothing either way, so there the
/// row written later stands: a lock changed on this device and not pushed yet
/// is not undone. `joining` is false for `settle_join_key`.
pub async fn adopt_vault_key(
    host: &Host<'_>,
    engine: &Engine,
    core: &Core,
    registry: &Registry,
    joining: bool,
) -> CmdResult<JoinKey> {
    let db = &core.db;
    let table = registry
        .table_of(VAULT_KEY_ENTITY)
        .expect("the key row is a synced entity");
    let Some(own) = read_row(db, table, veydan_lock::ROW_ID).await? else {
        return Ok(JoinKey::Unread);
    };
    let latest = engine.all_latest_ops().await.map_err(AppError::other)?;
    let Some(op) = latest.into_iter().find(|op| {
        op.entity_type == VAULT_KEY_ENTITY && op.entity_id == veydan_lock::ROW_ID && !op.deleted
    }) else {
        return Ok(JoinKey::Own);
    };
    let payload = op.payload.as_object().cloned().unwrap_or_default();
    let vault_key = payload
        .get("vault_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let same_key = own.get("vault_id").and_then(Value::as_str) == Some(vault_key.as_str());
    if same_key && written_later(&own, &payload) {
        return Ok(JoinKey::Own);
    }
    // A key row made after the join gives way without the question a join
    // asks: its key is still open in this session, or stays kept.
    if joining {
        crate::system::lock(host)?
            .check_replaceable(&vault_key)
            .await?;
    }
    let put = put_row(
        host,
        db,
        &core.app_data_dir,
        table,
        &op.entity_id,
        payload,
        &mut HashMap::new(),
    )
    .await?;
    let Put::Done { synced_hash, .. } = put else {
        return Err(AppError::other("the vault's key row could not be stored"));
    };
    Ok(JoinKey::Vaults(RowSyncState {
        entity: VAULT_KEY_ENTITY.into(),
        id: op.entity_id,
        head_hlc: Some(op.hlc),
        synced_hash,
        deleted: false,
    }))
}

/// A device that joined without a key row left the vault unread, and the key
/// row it makes later was never compared with the vault's. The comparison is
/// made here, before the row could be published over the vault's: with a key
/// row in the vault the device takes it; with none, its own is free to go. A
/// row that came by a pull settles it as well. An unreadable vault is an
/// error, and the row keeps waiting. Returns whether the key row was replaced.
pub async fn settle_join_key(
    host: &Host<'_>,
    engine: &Engine,
    core: &Core,
    registry: &Registry,
) -> CmdResult<bool> {
    let db = &core.db;
    if settings::get(db, config::JOIN_PENDING).await.is_none() {
        return Ok(false);
    }
    let mut replaced = false;
    let from_vault = load_row_state(db, VAULT_KEY_ENTITY, veydan_lock::ROW_ID)
        .await?
        .is_some();
    if !from_vault {
        match adopt_vault_key(host, engine, core, registry, false).await? {
            JoinKey::Unread => return Ok(false),
            JoinKey::Own => {}
            JoinKey::Vaults(st) => {
                save_row_state(db, &st).await?;
                replaced = true;
            }
        }
    }
    settings::delete(db, config::JOIN_PENDING).await?;
    Ok(replaced)
}
