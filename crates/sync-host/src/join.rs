// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Binding this device to a vault, and the key row of the lock a joining
//! device takes from it.

use crate::config::{self, build_storage, load_config, SyncConfig, VaultBinding};
use crate::rows::{delete_row, payload_hash, put_row, read_row, read_rows, Put};
use crate::state::{self, load_row_state, save_row_state, RowSyncState};
use crate::system::VAULT_KEY_ENTITY;
use crate::{Host, Registry, SyncManager};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sqlx::{Pool, Sqlite, SqliteConnection};
use std::collections::HashMap;
use std::time::Duration;
use veydan_core::{settings, AppError, CmdResult, Core};
use veydan_sync::{Engine, Hlc, Op, Probe, Vmk};

/// The refusal of a join while the vault is not all there to read: a chunk
/// a cloud client is still delivering, a storage that stopped answering.
/// The UI says it in words (ui/src/lib/core/join-error.ts); the cause follows.
pub const ERR_VAULT_UNREAD: &str = "the vault could not be read in full, try again later";

/// The refusal of a change of the binding while a cycle holds its slot.
pub const ERR_SYNC_RUNNING: &str = "a sync is running: wait until it ends, then try again";

/// How long the commands that change the binding wait for a running cycle.
pub const SLOT_WAIT: Duration = Duration::from_secs(20);

/// Set by the binding of a device that joins a vault ([`Fresh`]), until a
/// cycle has read every log the join read and applied it without a retry.
/// While it is set the cycle collects and pushes nothing: the states of the
/// handlers (notes, attachments, leases, snapshots) went with the old
/// binding, and what they hold would go out as new, over the vault (spec
/// 9.3, "Подключение: хранилище — истина").
pub const FRESH_BINDING: &str = "sync_fresh_binding";

/// The cycles that may end without reading the vault in full before the
/// device sends what it holds all the same, with a warning.
const FRESH_CYCLES: u32 = 5;
/// The same after this long since the join, from the second such cycle.
const FRESH_WAIT_MS: i64 = 10 * 60 * 1000;

/// What [`FRESH_BINDING`] holds.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Fresh {
    /// The logs the join found broken, named in its warning. Like any log a
    /// pull stops in, each holds the flag until a pull reads past its broken
    /// file (a copy still on its way has landed) or the wait ends: a row whose
    /// only op in the vault lies past that file has no state here yet, and
    /// would go out as new over it.
    #[serde(default)]
    pub broken: Vec<String>,
    /// When the device joined, in ms.
    #[serde(default)]
    pub since: i64,
    /// The cycles since that did not read the vault in full.
    #[serde(default)]
    pub cycles: u32,
}

/// The device joined a vault and has not read it in full since.
pub async fn fresh_binding(db: &Pool<Sqlite>) -> Option<Fresh> {
    let value = settings::get(db, FRESH_BINDING).await?;
    Some(serde_json::from_str(&value).unwrap_or_default())
}

/// After a cycle of a device that has just joined: the flag goes once the
/// cycle applied everything without a retry and read every log in full,
/// those the join found broken too. Otherwise it stays, for [`FRESH_CYCLES`]
/// cycles or [`FRESH_WAIT_MS`]: then the device sends what it holds anyway
/// and says so, rather than keep everything made here on this device alone
/// (a log that stays broken, a retry that does not end).
pub async fn after_fresh_cycle(
    db: &Pool<Sqlite>,
    mut fresh: Fresh,
    retry: Option<&str>,
    unread: &[String],
    warnings: &mut Vec<String>,
) -> CmdResult<()> {
    let missing: Vec<&str> = unread.iter().map(String::as_str).collect();
    if retry.is_none() && missing.is_empty() {
        return settings::delete(db, FRESH_BINDING).await;
    }
    fresh.cycles += 1;
    let waited = chrono::Utc::now().timestamp_millis() - fresh.since;
    if fresh.cycles >= FRESH_CYCLES || (fresh.cycles >= 2 && waited >= FRESH_WAIT_MS) {
        let why = match retry {
            _ if !missing.is_empty() => format!("logs not read: {}", missing.join(", ")),
            Some(reason) => reason.to_string(),
            None => String::new(),
        };
        warnings.push(format!(
            "the first read of the vault since the join did not finish ({why}): this device sends what it holds from now on"
        ));
        return settings::delete(db, FRESH_BINDING).await;
    }
    let value = serde_json::to_string(&fresh).map_err(AppError::other)?;
    settings::set(db, FRESH_BINDING, &value).await
}

/// What a join read of the vault ([`read_vault`]).
pub struct VaultRead {
    /// The latest op of each entity over the logs, a broken one up to the
    /// file it stops at.
    pub latest: Vec<Op>,
    /// The logs that stop at a file that does not open or does not continue
    /// the chain, by device id.
    pub broken: Vec<String>,
    /// What the device is told about them, when there are any.
    pub warning: Option<String>,
}

/// Every log of the vault from its start, as the first pull will read it.
/// A join takes the vault as the truth for what it holds, so a vault that
/// is not all there yet (a chunk listed and not readable, or missing before
/// one that is; a storage that fails) refuses it with [`ERR_VAULT_UNREAD`].
/// A log that stops at a file that does not open or does not continue its
/// chain does not: the join takes what comes before that file, as every
/// pull will, and says so; no pull reads past it.
pub async fn read_vault(engine: &Engine) -> CmdResult<VaultRead> {
    let read = engine
        .read_every_log()
        .await
        .map_err(|e| AppError::other(format!("{ERR_VAULT_UNREAD}: {e}")))?;
    let warning = if read.broken.is_empty() {
        None
    } else {
        let names: HashMap<String, String> = engine
            .list_device_cards()
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();
        let logs: Vec<String> = read
            .broken
            .iter()
            .map(
                |(device, why)| match names.get(device).filter(|n| !n.is_empty()) {
                    Some(name) => format!("{name} ({device}): {why}"),
                    None => format!("{device}: {why}"),
                },
            )
            .collect();
        Some(format!(
            "the log of a device cannot be read in full and its changes past the broken file are not taken: {}",
            logs.join("; ")
        ))
    };
    Ok(VaultRead {
        latest: read.ops,
        broken: read.broken.into_iter().map(|(device, _)| device).collect(),
        warning,
    })
}

/// [`join`] with the slot of the cycle held, as `sync_join_vault` runs it:
/// a cycle of the old binding that ended after the join would write its
/// states and positions under the new one. Waits up to `wait` for a running
/// cycle; the slot is free again when this returns, before the first cycle.
pub async fn join_held(
    host: &Host<'_>,
    core: &Core,
    sync: &SyncManager,
    passphrase: &str,
    wait: Duration,
) -> CmdResult<bool> {
    let _slot = sync
        .pause(wait)
        .await
        .ok_or_else(|| AppError::other(ERR_SYNC_RUNNING))?;
    join(host, core, sync.registry(), passphrase).await
}

/// Open the vault in the configured storage and bind this device to it. The
/// vault is the authority for what it holds: it is read in full first, and a
/// row this device has under an id the vault holds takes the vault's value at
/// the first pull, unless it was changed here since it last synced with this
/// vault (`bind_joined`). The key row is settled now: a device that has one
/// of its own takes the vault's before it is bound, or is refused. Only a
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
    let read = read_vault(&engine).await?;
    let key_row = adopt_vault_key(host, &read.latest, core, registry, true).await?;
    bind_joined(db, registry, engine.vault_id(), &vmk, &read, &key_row).await?;
    Ok(matches!(key_row, JoinKey::Vaults(_)))
}

/// What this device last synced of each row with the vault `vault_id`, by
/// type and id: its hash and whether it was a tombstone. The states of the
/// binding when it is to that vault (a join again), else those its leave
/// kept (`sync_left_row_state`); none for another vault.
async fn synced_before(
    db: &Pool<Sqlite>,
    vault_id: &str,
) -> CmdResult<HashMap<(String, String), (String, bool)>> {
    let bound = config::load_binding(db).await.map(|b| b.vault_id);
    let rows: Vec<(String, String, String, i64)> = if bound.as_deref() == Some(vault_id) {
        sqlx::query_as("SELECT entity_type, entity_id, synced_hash, deleted FROM sync_row_state")
            .fetch_all(db)
            .await
    } else {
        sqlx::query_as(
            "SELECT entity_type, entity_id, synced_hash, deleted FROM sync_left_row_state
             WHERE vault_id = ?",
        )
        .bind(vault_id)
        .fetch_all(db)
        .await
    }
    .map_err(AppError::db)?;
    Ok(rows
        .into_iter()
        .map(|(entity, id, hash, deleted)| ((entity, id), (hash, deleted != 0)))
        .collect())
}

/// [`state::save_row_state`] inside a transaction.
async fn put_state(tx: &mut SqliteConnection, s: &RowSyncState) -> CmdResult<()> {
    sqlx::query(
        "INSERT INTO sync_row_state (entity_type, entity_id, head_hlc, synced_hash, deleted)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(entity_type, entity_id) DO UPDATE SET
           head_hlc = excluded.head_hlc, synced_hash = excluded.synced_hash, deleted = excluded.deleted",
    )
    .bind(&s.entity)
    .bind(&s.id)
    .bind(s.head_hlc.as_ref().map(Hlc::encode).unwrap_or_default())
    .bind(&s.synced_hash)
    .bind(s.deleted as i64)
    .execute(&mut *tx)
    .await
    .map_err(AppError::db)?;
    Ok(())
}

/// Bind a device that joins the vault it read (`read`), once
/// `adopt_vault_key` settled the key row. A fresh binding has no states, so
/// its first collect would publish every local row as new with the clock of
/// now, over what the other devices wrote since: a leave, then a join of the
/// same vault, undid their edits and brought back what they deleted. So each
/// local row under an id the vault holds gets a state with no head:
/// - with the hash of what it holds, so that the collect sees no change and
///   the first pull applies the vault's op: a row as this device last synced
///   it with this vault, any row of another vault or of a device new to it,
///   and any row whose latest op in the vault is a tombstone;
/// - with the hash this device last synced (empty for a row it synced as
///   deleted) where the row changed here since, so that the pull leaves it
///   (`rows::edited_rows`) and the next collect publishes it, newer by LWW.
///
/// A row the vault does not hold is published as before; a row gone here
/// takes the vault's again (a clear of this device alone deletes nowhere
/// else). [`FRESH_BINDING`] holds back the collects of the handlers, whose
/// states went with the old binding, until the vault was read. The flag, the
/// states and the key row go in one transaction, before the binding: a
/// binding is never there without them.
pub async fn bind_joined(
    db: &Pool<Sqlite>,
    registry: &Registry,
    vault_id: &str,
    vmk: &Vmk,
    read: &VaultRead,
    key_row: &JoinKey,
) -> CmdResult<()> {
    let synced = synced_before(db, vault_id).await?;
    unbind(db, registry).await?;
    let latest: HashMap<(&str, &str), &Op> = read
        .latest
        .iter()
        .map(|op| ((op.entity_type.as_str(), op.entity_id.as_str()), op))
        .collect();
    let mut states = Vec::new();
    // The key row is settled by `adopt_vault_key`, below.
    for table in registry
        .tables()
        .iter()
        .filter(|t| t.push && t.spec.entity != VAULT_KEY_ENTITY)
    {
        let entity = table.spec.entity;
        for (id, payload) in read_rows(db, table).await? {
            let Some(op) = latest.get(&(entity, id.as_str())) else {
                continue;
            };
            let hash = payload_hash(&payload);
            let synced_hash = match synced.get(&(entity.to_string(), id.clone())) {
                Some((_, true)) if !op.deleted => String::new(),
                Some((old, false)) if !op.deleted && *old != hash => old.clone(),
                _ => hash,
            };
            states.push(RowSyncState {
                entity: entity.into(),
                id,
                head_hlc: None,
                synced_hash,
                deleted: false,
            });
        }
    }
    let fresh = Fresh {
        broken: read.broken.clone(),
        since: chrono::Utc::now().timestamp_millis(),
        cycles: 0,
    };
    let mut tx = db.begin().await.map_err(AppError::db)?;
    let value = serde_json::to_string(&fresh).map_err(AppError::other)?;
    settings::set(&mut *tx, FRESH_BINDING, &value).await?;
    if let Some(warning) = &read.warning {
        settings::set(&mut *tx, "sync_last_warning", warning).await?;
    }
    for st in &states {
        put_state(&mut tx, st).await?;
    }
    match key_row {
        JoinKey::Vaults(st) => put_state(&mut tx, st).await?,
        // The vault was not read for the key row. A key row this device
        // makes later waits for `settle_join_key`: the vault may hold one.
        JoinKey::Unread => settings::set(&mut *tx, config::JOIN_PENDING, "1").await?,
        JoinKey::Own => {
            if let Some(st) = own_key_state(&read.latest) {
                put_state(&mut tx, &st).await?;
            }
        }
    }
    sqlx::query("DELETE FROM sync_left_row_state")
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;
    tx.commit().await.map_err(AppError::db)?;
    save_bound(db, vault_id, vmk).await
}

/// The state of the key row of this device that stands over the vault's op
/// of it (`JoinKey::Own`), when the vault has one: that op is put behind
/// the row's head, so no pull applies it, and the empty hash has the next
/// collect publish the row.
fn own_key_state(latest: &[Op]) -> Option<RowSyncState> {
    let op = latest
        .iter()
        .find(|op| op.entity_type == VAULT_KEY_ENTITY && op.entity_id == veydan_lock::ROW_ID)?;
    Some(RowSyncState {
        entity: VAULT_KEY_ENTITY.into(),
        id: op.entity_id.clone(),
        head_hlc: Some(op.hlc.clone()),
        synced_hash: String::new(),
        deleted: false,
    })
}

/// A fresh binding starts from scratch: no peer heads, no entity positions.
async fn unbind(db: &Pool<Sqlite>, registry: &Registry) -> CmdResult<()> {
    state::clear_all_states(db, registry).await?;
    config::clear_binding(db, registry).await
}

async fn save_bound(db: &Pool<Sqlite>, vault_id: &str, vmk: &Vmk) -> CmdResult<()> {
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

pub async fn bind(
    db: &Pool<Sqlite>,
    registry: &Registry,
    vault_id: &str,
    vmk: &Vmk,
) -> CmdResult<()> {
    unbind(db, registry).await?;
    save_bound(db, vault_id, vmk).await
}

/// `sync_leave` on the database alone. What this device last synced of each
/// row stays (`sync_left_row_state`, the last vault left only): a join of
/// the same vault later tells an edit made here meanwhile from a row the
/// vault is the truth for (`bind_joined`).
pub async fn leave(db: &Pool<Sqlite>, registry: &Registry) -> CmdResult<()> {
    if let Some(binding) = config::load_binding(db).await {
        let mut tx = db.begin().await.map_err(AppError::db)?;
        sqlx::query("DELETE FROM sync_left_row_state")
            .execute(&mut *tx)
            .await
            .map_err(AppError::db)?;
        sqlx::query(
            "INSERT INTO sync_left_row_state (vault_id, entity_type, entity_id, synced_hash, deleted)
             SELECT ?, entity_type, entity_id, synced_hash, deleted FROM sync_row_state",
        )
        .bind(&binding.vault_id)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;
        tx.commit().await.map_err(AppError::db)?;
    }
    state::clear_all_states(db, registry).await?;
    config::clear_binding(db, registry).await?;
    settings::set(db, "sync_enabled", "0").await
}

/// Point this device at the storage of `cfg`, a config the form has not
/// saved yet. The binding of a joined device is its vault's: saved alone,
/// another storage leaves every cycle looking for that vault where it is
/// not (`no vault in this storage`). So the device leaves its vault before
/// the config is saved, and a create or a join in the new storage is the
/// caller's next step. The storage is read first: one that does not answer
/// is an error and one that holds somebody's files is `foreign`, and
/// neither changes anything. Returns what the storage holds: `same` — the
/// vault this device has joined, at another address: the config is saved
/// and the binding stays; `empty`, `vault` — the device left its vault and
/// the config is saved.
pub async fn change_storage(
    db: &sqlx::Pool<sqlx::Sqlite>,
    registry: &Registry,
    cfg: SyncConfig,
) -> CmdResult<&'static str> {
    let cfg = config::completed(db, cfg).await?;
    let storage = build_storage(&cfg)?;
    let joined = config::load_binding(db).await.map(|b| b.vault_id);
    let found = match Engine::probe(storage.as_ref())
        .await
        .map_err(AppError::other)?
    {
        Probe::Foreign => return Ok("foreign"),
        Probe::Vault(m) if joined.as_deref() == Some(m.vault_id.as_str()) => {
            config::save_config(db, &cfg).await?;
            return Ok("same");
        }
        Probe::Vault(_) => "vault",
        Probe::Empty => "empty",
    };
    if joined.is_some() {
        leave(db, registry).await?;
    }
    // Sync is off until the vault of the new storage is joined, as after a leave.
    config::save_config(
        db,
        &SyncConfig {
            enabled: false,
            ..cfg
        },
    )
    .await?;
    Ok(found)
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
    /// The device has no key row, so the vault was not read for it.
    Unread,
    /// The device's row stays and is published like any row: the vault has
    /// none, or has an older wrap of the same key.
    Own,
    /// The vault's row took the place of the device's, or its tombstone
    /// deleted it. The state records it as applied, so the row is not
    /// published.
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
/// is not undone. A tombstone is the vault's last word at a join too: the
/// local row goes, its key kept by the hook of the table as for any deleted
/// key row; published, the row would come back on every device. A row made
/// after the join stands over it. `latest` is what [`read_vault`] read;
/// `joining` is false for `settle_join_key`.
pub async fn adopt_vault_key(
    host: &Host<'_>,
    latest: &[Op],
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
    let Some(op) = latest
        .iter()
        .find(|op| op.entity_type == VAULT_KEY_ENTITY && op.entity_id == veydan_lock::ROW_ID)
        .cloned()
    else {
        return Ok(JoinKey::Own);
    };
    if op.deleted {
        // A key row made after the join is newer than any deletion the
        // vault held then.
        if !joining {
            return Ok(JoinKey::Own);
        }
        // No key takes the place of this device's: one that holds data
        // behind a lock is refused, as for a row with another key.
        crate::system::lock(host)?.check_replaceable("").await?;
        delete_row(host, core, table, &op.entity_id).await?;
        return Ok(JoinKey::Vaults(RowSyncState {
            entity: VAULT_KEY_ENTITY.into(),
            id: op.entity_id,
            head_hlc: Some(op.hlc),
            synced_hash: String::new(),
            deleted: true,
        }));
    }
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
        // No key row yet: nothing to compare, and the vault is not read.
        let table = registry
            .table_of(VAULT_KEY_ENTITY)
            .expect("the key row is a synced entity");
        if read_row(db, table, veydan_lock::ROW_ID).await?.is_none() {
            return Ok(false);
        }
        let latest = engine.all_latest_ops().await.map_err(AppError::other)?;
        match adopt_vault_key(host, &latest, core, registry, false).await? {
            JoinKey::Unread => return Ok(false),
            JoinKey::Own => {
                if let Some(st) = own_key_state(&latest) {
                    save_row_state(db, &st).await?;
                }
            }
            JoinKey::Vaults(st) => {
                save_row_state(db, &st).await?;
                replaced = true;
            }
        }
    }
    settings::delete(db, config::JOIN_PENDING).await?;
    Ok(replaced)
}
