// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The secrets of groups as the entity `messenger_secret` (plan §4.3,
//! platform-spec 9.6): the keys and link secrets of groups in the box
//! `messenger` of the lock — `group.<id>.key.<kid>`, `group.<id>.link.<epoch>`
//! (`messenger_archive::secret_is_archived`), never the nsec or the secrets
//! of media hosts, call nodes and push.
//!
//! The id of the entity is `<owner>/<name of the secret>` (D13); the
//! payload holds the owner and the bytes in standard base64:
//!
//! ```json
//! { "owner": "<hex key>", "value": "q83vEjRWeJA…" }
//! ```
//!
//! A deletion is an op with `deleted` and the payload `{}`. It is sent only
//! for a group forgotten here (`ArchiveSync::forget_scope`), never because a
//! secret is missing from the box: a box emptied by a reset of the lock
//! must not take the keys from the other devices.
//!
//! Apply puts a secret the box lacks and never overwrites one it has, and
//! tells the runtime of a key put (`group_secret_arrived`: the messages
//! waiting for it open); a closed box holds the op for the next cycle (the
//! op is read again from its device; the key is not written anywhere but
//! the box). A secret of another owner's archive is left where it is. A
//! secret whose deletion was sent or taken is not published again, even if
//! the box still holds it: a group joined again brings new keys.

use super::archive::{warn, Saves};
use super::ArchiveSync;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use messenger_archive::{parse_secret_id, secret_entity_id, secret_is_archived, ENTITY_SECRET};
use messenger_core::PubKey;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use std::collections::{HashMap, HashSet};
use veydan_core::{AppError, CmdResult};
use veydan_lock::Lock;
use veydan_sync::{sha256_hex, Hlc, Op};
use veydan_sync_host::{Applied, Collected, Cycle};

/// The box of the lock the messenger keeps its secrets in.
const BOX: &str = crate::commands::SECRET_BOX;

#[derive(Serialize, Deserialize)]
struct SecretPayload {
    #[serde(default)]
    owner: String,
    value: String,
}

/// A row of `messenger_archive_secrets`, by the name of the secret.
#[derive(Clone, Debug, Default)]
pub(crate) struct SecretState {
    pub name: String,
    pub head_hlc: Option<Hlc>,
    pub hash: String,
    pub deleted: bool,
    pub tomb: bool,
}

type Row = (String, String, String, i64, i64);

fn from_row(r: Row) -> SecretState {
    SecretState { name: r.0, head_hlc: Hlc::decode(&r.1), hash: r.2, deleted: r.3 != 0, tomb: r.4 != 0 }
}

async fn load_all(db: &Pool<Sqlite>) -> CmdResult<HashMap<String, SecretState>> {
    let rows: Vec<Row> = sqlx::query_as("SELECT name, head_hlc, hash, deleted, tomb FROM messenger_archive_secrets")
        .fetch_all(db)
        .await
        .map_err(AppError::db)?;
    Ok(rows.into_iter().map(from_row).map(|st| (st.name.clone(), st)).collect())
}

async fn load(db: &Pool<Sqlite>, name: &str) -> CmdResult<Option<SecretState>> {
    let row: Option<Row> =
        sqlx::query_as("SELECT name, head_hlc, hash, deleted, tomb FROM messenger_archive_secrets WHERE name = ?")
            .bind(name)
            .fetch_optional(db)
            .await
            .map_err(AppError::db)?;
    Ok(row.map(from_row))
}

pub(crate) async fn save(db: &Pool<Sqlite>, st: &SecretState) -> CmdResult<()> {
    sqlx::query(
        "INSERT INTO messenger_archive_secrets (name, head_hlc, hash, deleted, tomb) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(name) DO UPDATE SET head_hlc = excluded.head_hlc, hash = excluded.hash,
            deleted = excluded.deleted, tomb = excluded.tomb",
    )
    .bind(&st.name)
    .bind(st.head_hlc.as_ref().map(Hlc::encode).unwrap_or_default())
    .bind(&st.hash)
    .bind(st.deleted as i64)
    .bind(st.tomb as i64)
    .execute(db)
    .await
    .map_err(AppError::db)?;
    Ok(())
}

/// The secrets of a group forgotten here go as tombstones with the next
/// collect: those it published or took.
pub(crate) async fn mark_tombs(db: &Pool<Sqlite>, group_id: &str) -> CmdResult<()> {
    let prefix = format!("group.{group_id}.");
    for st in load_all(db).await?.into_values() {
        if st.name.starts_with(&prefix) && !st.deleted {
            save(db, &SecretState { tomb: true, ..st }).await?;
        }
    }
    Ok(())
}

/// The group of a secret's name, `group.<id>.…`.
fn group_of(name: &str) -> Option<&str> {
    name.strip_prefix("group.")?.split_once('.').map(|(id, _)| id)
}

fn op(owner: &PubKey, name: &str, hlc: Hlc, value: Option<&[u8]>) -> Op {
    Op {
        entity_type: ENTITY_SECRET.into(),
        entity_id: secret_entity_id(owner, name),
        hlc,
        deleted: value.is_none(),
        payload: match value {
            Some(v) => serde_json::to_value(SecretPayload { owner: owner.as_hex().to_string(), value: B64.encode(v) }).unwrap_or_default(),
            None => serde_json::json!({}),
        },
    }
}

/// Whose archive: the owner of the running archive; `None` before the
/// messenger runs or has a key (nothing is published, and the ops wait).
async fn owner_of(arch: &ArchiveSync) -> Option<PubKey> {
    arch.store()?.owner().await.ok().flatten()
}

/// The secrets of groups the box holds and the storage does not, and the
/// tombstones of groups forgotten here.
pub(crate) async fn collect(cx: &mut Cycle<'_>, arch: &ArchiveSync, saves: &mut Saves, out: &mut Collected) -> CmdResult<()> {
    let Some(lock) = cx.host.state::<Lock>() else { return Ok(()) };
    let Some(owner) = owner_of(arch).await else { return Ok(()) };
    let db = &cx.core.db;
    let states = load_all(db).await?;
    for st in states.values().filter(|st| st.tomb) {
        let hlc = cx.clock.now();
        out.ops.push(op(&owner, &st.name, hlc.clone(), None));
        saves.secrets.push(SecretState { head_hlc: Some(hlc), deleted: true, tomb: false, ..st.clone() });
    }
    let secrets = lock.secret_box(BOX);
    let names = match secrets.names().await {
        Ok(names) => names,
        Err(e) => {
            warn(out, format!("the keys of groups: {e}"));
            return Ok(());
        }
    };
    // A secret with a row is published or taken already (a key never
    // changes under its name); one whose deletion was sent or taken stays
    // deleted, even if the box still holds it: a group joined again brings
    // new keys. Only what is new to the storage needs the box open.
    let names: Vec<String> = names.into_iter().filter(|name| secret_is_archived(name) && !states.contains_key(name)).collect();
    if names.is_empty() {
        return Ok(());
    }
    // Only the groups of this identity: the box outlives the store when
    // another key is imported (`account::wipe` empties the store, and the
    // secrets of the groups before go only with `forget_gone`), and a key
    // or a link of the identity before must not go into the archive of
    // this one.
    let groups: HashSet<String> = match arch.store() {
        Some(store) => match store.group_ids().await {
            Ok(ids) => ids.into_iter().collect(),
            Err(e) => {
                warn(out, format!("the groups: {e}"));
                return Ok(());
            }
        },
        None => return Ok(()),
    };
    let names: Vec<String> = names.into_iter().filter(|name| group_of(name).is_some_and(|g| groups.contains(g))).collect();
    if names.is_empty() {
        return Ok(());
    }
    if !secrets.is_unlocked() {
        warn(out, format!("the lock is closed: {} keys of groups wait for it", names.len()));
        return Ok(());
    }
    for name in names {
        let value = match secrets.get(&name).await {
            Ok(Some(value)) => value,
            Ok(None) => continue,
            Err(e) => {
                warn(out, format!("{name}: {e}"));
                continue;
            }
        };
        let hash = sha256_hex(&value);
        let hlc = cx.clock.now();
        out.ops.push(op(&owner, &name, hlc.clone(), Some(&value)));
        saves.secrets.push(SecretState { name, head_hlc: Some(hlc), hash, deleted: false, tomb: false });
    }
    Ok(())
}

/// Put the secrets the box lacks; take away those of a group forgotten on
/// another device. How many keys of groups were put (what a parked segment
/// may have waited for).
pub(crate) async fn apply(cx: &mut Cycle<'_>, arch: &ArchiveSync, ops: &[Op], applied: &mut Applied) -> CmdResult<u32> {
    let Some(lock) = cx.host.state::<Lock>() else { return Ok(0) };
    let ops: Vec<&Op> = ops.iter().filter(|op| op.entity_type == ENTITY_SECRET).collect();
    if ops.is_empty() {
        return Ok(0);
    }
    let db = &cx.core.db;
    let runtime = arch.runtime();
    let owner = owner_of(arch).await;
    let secrets = lock.secret_box(BOX);
    let mut keys_put = 0;
    let mut closed_told = false;
    for op in ops {
        let Some((of, name)) = parse_secret_id(&op.entity_id) else {
            applied.skipped.push(format!("messenger secret {}: not an id of a secret", op.entity_id));
            continue;
        };
        // An op naming another secret (the nsec) is never put in the box.
        if !secret_is_archived(name) {
            applied.skipped.push(format!("messenger secret {name}: not a secret of a group"));
            continue;
        }
        let hold = |applied: &mut Applied, why: String| {
            applied.retry.get_or_insert(why);
            if !applied.retry_from.contains(&op.hlc.device_id) {
                applied.retry_from.push(op.hlc.device_id.clone());
            }
        };
        let Some(owner) = &owner else {
            // Whose it is cannot be told before the messenger runs with a
            // key: the op is read again then.
            hold(applied, "messenger secret: the messenger does not run yet".into());
            continue;
        };
        if of != *owner {
            // Another identity's archive in the same storage.
            applied.skipped.push(format!("messenger secret {name}: of another owner"));
            continue;
        }
        let prev = load(db, name).await?;
        if prev.as_ref().and_then(|st| st.head_hlc.as_ref()).is_some_and(|h| *h >= op.hlc) {
            continue;
        }
        if op.deleted {
            // Needs no key: works behind a closed lock.
            if let Err(e) = secrets.delete(name).await {
                hold(applied, format!("messenger secret {name}: {e}"));
                continue;
            }
            save(db, &SecretState { name: name.into(), head_hlc: Some(op.hlc.clone()), hash: String::new(), deleted: true, tomb: false }).await?;
            continue;
        }
        let payload = serde_json::from_value::<SecretPayload>(op.payload.clone()).ok();
        if payload.as_ref().is_some_and(|p| !p.owner.is_empty() && p.owner != owner.as_hex()) {
            applied.skipped.push(format!("messenger secret {name}: the payload names another owner"));
            continue;
        }
        let value = payload.and_then(|p| B64.decode(p.value).ok()).filter(|v| !v.is_empty());
        let Some(value) = value else {
            applied.skipped.push(format!("messenger secret {name}: the payload holds no value"));
            continue;
        };
        if !secrets.is_unlocked() {
            hold(applied, "messenger secret: the lock is closed".into());
            if !closed_told {
                cx.warnings.push("messenger archive: the lock is closed: the keys of groups wait for it".into());
                closed_told = true;
            }
            continue;
        }
        let mut put = false;
        let hash = match secrets.get(name).await {
            // The box has it: what it holds stays, and is what the state
            // names, so that the collect does not send it back.
            Ok(Some(held)) => sha256_hex(&held),
            Ok(None) => {
                if let Err(e) = secrets.put(name, &value).await {
                    hold(applied, format!("messenger secret {name}: {e}"));
                    continue;
                }
                put = true;
                sha256_hex(&value)
            }
            Err(e) => {
                hold(applied, format!("messenger secret {name}: {e}"));
                continue;
            }
        };
        save(db, &SecretState { name: name.into(), head_hlc: Some(op.hlc.clone()), hash, deleted: false, tomb: false }).await?;
        // Messages of the group waiting for this key open now.
        if put && name.contains(".key.") {
            keys_put += 1;
            if let Some(rt) = runtime.as_ref() {
                if let Err(e) = rt.group_secret_arrived(name).await {
                    cx.warnings.push(format!("messenger archive: {name}: {e}"));
                }
            }
        }
    }
    Ok(keys_put)
}
