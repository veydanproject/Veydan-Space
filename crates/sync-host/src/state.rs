// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Persistence of the engine's `LocalState` and of the sync positions of
//! table rows, and the fingerprint of the registry. Handlers keep the
//! positions of their entities in tables of their own
//! (`Handler::state_tables`).

use crate::Registry;
use sqlx::{Pool, Sqlite};
use std::collections::HashMap;
use veydan_core::{settings, AppError, CmdResult};
use veydan_sync::{Hlc, LocalState, PeerHead};

/// Forget every sync position; the next cycle starts from scratch.
pub async fn clear_all_states(db: &Pool<Sqlite>, registry: &Registry) -> CmdResult<()> {
    let own = ["sync_row_state", "sync_gc_candidates", "sync_peers"];
    for table in registry.handler_state_tables().chain(own) {
        sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table}")))
            .execute(db)
            .await
            .map_err(AppError::db)?;
    }
    Ok(())
}

// ── Table rows ───────────────────────────────────────────────────────────────

/// Where a local table row stands relative to the vault (LWW, no merge).
#[derive(Debug, Clone, Default)]
pub struct RowSyncState {
    pub entity: String,
    pub id: String,
    pub head_hlc: Option<Hlc>,
    /// Hash of the synced columns when the row was last pushed or applied.
    pub synced_hash: String,
    pub deleted: bool,
}

type RowStateRow = (String, String, String, String, i64);

fn row_to_row_state(r: RowStateRow) -> RowSyncState {
    RowSyncState {
        entity: r.0,
        id: r.1,
        head_hlc: Hlc::decode(&r.2),
        synced_hash: r.3,
        deleted: r.4 != 0,
    }
}

pub async fn load_row_states(
    db: &Pool<Sqlite>,
    entity: &str,
) -> CmdResult<HashMap<String, RowSyncState>> {
    let rows: Vec<RowStateRow> = sqlx::query_as(
        "SELECT entity_type, entity_id, head_hlc, synced_hash, deleted FROM sync_row_state WHERE entity_type = ?",
    )
    .bind(entity)
    .fetch_all(db)
    .await
    .map_err(AppError::db)?;
    Ok(rows
        .into_iter()
        .map(row_to_row_state)
        .map(|s| (s.id.clone(), s))
        .collect())
}

pub async fn load_row_state(
    db: &Pool<Sqlite>,
    entity: &str,
    id: &str,
) -> CmdResult<Option<RowSyncState>> {
    let row: Option<RowStateRow> = sqlx::query_as(
        "SELECT entity_type, entity_id, head_hlc, synced_hash, deleted FROM sync_row_state
         WHERE entity_type = ? AND entity_id = ?",
    )
    .bind(entity)
    .bind(id)
    .fetch_optional(db)
    .await
    .map_err(AppError::db)?;
    Ok(row.map(row_to_row_state))
}

pub async fn save_row_state(db: &Pool<Sqlite>, s: &RowSyncState) -> CmdResult<()> {
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
    .execute(db)
    .await
    .map_err(AppError::db)?;
    Ok(())
}

pub async fn load_local_state(db: &Pool<Sqlite>) -> CmdResult<(LocalState, Option<Hlc>)> {
    let own_seq = settings::get(db, "sync_own_seq")
        .await
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let own_head_hash = settings::get(db, "sync_own_head").await.unwrap_or_default();
    let hlc = settings::get(db, "sync_hlc")
        .await
        .and_then(|v| Hlc::decode(&v));
    let rows: Vec<(String, i64, String)> =
        sqlx::query_as("SELECT device_id, seq, head_hash FROM sync_peers")
            .fetch_all(db)
            .await
            .map_err(AppError::db)?;
    let peers = rows
        .into_iter()
        .map(|(d, seq, hash)| {
            (
                d,
                PeerHead {
                    seq: seq as u64,
                    hash,
                },
            )
        })
        .collect();
    Ok((
        LocalState {
            own_seq,
            own_head_hash,
            peers,
        },
        hlc,
    ))
}

/// Own log position and clock. Called right after every push so a crash can
/// never reuse a sequence number.
pub async fn save_own_state(db: &Pool<Sqlite>, s: &LocalState, hlc: &Hlc) -> CmdResult<()> {
    settings::set(db, "sync_own_seq", &s.own_seq.to_string()).await?;
    settings::set(db, "sync_own_head", &s.own_head_hash).await?;
    settings::set(db, "sync_hlc", &hlc.encode()).await
}

/// Peer heads. Called only after their ops were fully applied.
pub async fn save_peer_heads(db: &Pool<Sqlite>, s: &LocalState) -> CmdResult<()> {
    for (device_id, head) in &s.peers {
        sqlx::query(
            "INSERT INTO sync_peers (device_id, seq, head_hash) VALUES (?, ?, ?)
             ON CONFLICT(device_id) DO UPDATE SET seq = excluded.seq, head_hash = excluded.head_hash",
        )
        .bind(device_id)
        .bind(head.seq as i64)
        .bind(&head.hash)
        .execute(db)
        .await
        .map_err(AppError::db)?;
    }
    Ok(())
}

// ── The registry ─────────────────────────────────────────────────────────────

/// The fingerprint of the registry of the last start, and what it was made of.
const FINGERPRINT_KEY: &str = "sync_registry_fingerprint";
const SYNCED_KEY: &str = "sync_registry";
/// Set while the vault is to be read again for what an earlier registry did
/// not take: the types and keys every registry since the last full read
/// synced (`Registry::synced`). A new binding reads everything anyway.
pub const REREAD_KEY: &str = "sync_reread";
/// The devices whose logs were read again in full since `REREAD_KEY` was
/// set; their ops are not read a second time.
pub const REREAD_PEERS_KEY: &str = "sync_reread_peers";

/// At start (spec 9.2, rule 3): when the registry is not the one of the last
/// start, the next cycle reads the vault again from the start of every log
/// and applies what the earlier registry skipped. The positions in the logs
/// and the state of the own log stay. Returns whether the vault is read
/// again. A data file without a fingerprint was last opened by a build
/// before it, which synced what this one does.
pub async fn check_registry(db: &Pool<Sqlite>, registry: &Registry) -> CmdResult<bool> {
    let fingerprint = registry.fingerprint();
    let stored = settings::get(db, FINGERPRINT_KEY).await;
    if stored.as_deref() == Some(fingerprint.as_str()) {
        return Ok(false);
    }
    let mut tx = db.begin().await.map_err(AppError::db)?;
    if stored.is_some() {
        let before: Vec<String> = settings::get_json(&mut *tx, SYNCED_KEY)
            .await
            .unwrap_or_default();
        let known: Vec<String> = match settings::get_json::<Vec<String>>(&mut *tx, REREAD_KEY).await
        {
            Some(pending) => pending.into_iter().filter(|s| before.contains(s)).collect(),
            None => before,
        };
        settings::set_json(&mut *tx, REREAD_KEY, &known).await?;
        settings::delete(&mut *tx, REREAD_PEERS_KEY).await?;
    }
    settings::set_json(&mut *tx, SYNCED_KEY, &registry.synced()).await?;
    settings::set(&mut *tx, FINGERPRINT_KEY, &fingerprint).await?;
    tx.commit().await.map_err(AppError::db)?;
    Ok(stored.is_some())
}

/// Whether the registry is not the one the last start recorded, or none was:
/// what this build syncs is new to the data file.
async fn registry_is_new(db: &Pool<Sqlite>, registry: &Registry) -> bool {
    settings::get(db, FINGERPRINT_KEY).await.as_deref() != Some(registry.fingerprint().as_str())
}

/// Set from the start of a first fill of the labels until a fill went
/// through: `check_registry` records the registry whatever the fill did.
const LABELS_FILL_KEY: &str = "sync_labels_fill";

/// At start, before `check_registry` (spec 10.2): `fill` — every owner
/// publishing its labels — when the registry is new to the data file, and at
/// every later start until a fill went through. Returns whether it ran.
pub async fn fill_labels<F>(db: &Pool<Sqlite>, registry: &Registry, fill: F) -> CmdResult<bool>
where
    F: std::future::Future<Output = CmdResult<usize>>,
{
    if !registry_is_new(db, registry).await && settings::get(db, LABELS_FILL_KEY).await.is_none() {
        return Ok(false);
    }
    settings::set(db, LABELS_FILL_KEY, "1").await?;
    fill.await?;
    settings::delete(db, LABELS_FILL_KEY).await?;
    Ok(true)
}

/// What the registries since the last full read synced, while the vault is
/// to be read again.
pub async fn reread(db: &Pool<Sqlite>) -> Option<Vec<String>> {
    settings::get_json(db, REREAD_KEY).await
}

/// Whether a registry that synced `known` took an op of `entity` for `id`:
/// its type, and for a setting its key.
pub fn took(known: &[String], entity: &str, id: &str) -> bool {
    let has = |item: String| known.contains(&item);
    has(format!("entity:{entity}"))
        && (entity != crate::SETTING_ENTITY || has(format!("setting:{id}")))
}

/// The devices whose logs were read again in full since the registry changed.
pub async fn reread_peers(db: &Pool<Sqlite>) -> Vec<String> {
    settings::get_json(db, REREAD_PEERS_KEY)
        .await
        .unwrap_or_default()
}

/// The logs of `peers` were read again in full and what they brought applied.
pub async fn reread_peers_done(db: &Pool<Sqlite>, peers: &[String]) -> CmdResult<()> {
    let mut done = reread_peers(db).await;
    for peer in peers {
        if !done.contains(peer) {
            done.push(peer.clone());
        }
    }
    done.sort_unstable();
    settings::set_json(db, REREAD_PEERS_KEY, &done).await
}

/// The vault was read again without a gap.
pub async fn reread_done(db: &Pool<Sqlite>) -> CmdResult<()> {
    settings::delete(db, REREAD_KEY).await?;
    settings::delete(db, REREAD_PEERS_KEY).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Hooks, Plan, Step, TableSpec};
    use veydan_core::db;

    const ROWS: &[&str] = &["password_vault", "setting", "label", "thing"];
    const PLAN: Plan = Plan {
        collect: &[Step::rows("rows", None, "app", ROWS)],
        apply: &[Step::rows("rows", None, "app", ROWS)],
        late: &[],
    };

    fn registry(things: bool) -> Registry {
        let mut registry = Registry::new();
        if things {
            registry.add("things", |registry| {
                registry.table(
                    TableSpec::plain("thing", "things", &["name"]),
                    Hooks::default(),
                )
            });
        }
        registry.finish(&PLAN).unwrap()
    }

    #[tokio::test]
    async fn a_new_registry_reads_again_what_every_registry_since_did_not_take() {
        let dir = tempfile::tempdir().unwrap();
        let db = db::open(
            &dir.path().join(db::DB_FILE),
            &[veydan_core::SCHEMA, crate::SCHEMA],
        )
        .await
        .unwrap();
        // The data of a build before the fingerprint, then a restart.
        assert!(!check_registry(&db, &registry(true)).await.unwrap());
        assert!(!check_registry(&db, &registry(true)).await.unwrap());
        assert_eq!(reread(&db).await, None);

        assert!(check_registry(&db, &registry(false)).await.unwrap());
        assert_eq!(reread(&db).await, Some(registry(true).synced()));
        // Back before a cycle read the vault: only what both took counts.
        assert!(check_registry(&db, &registry(true)).await.unwrap());
        assert_eq!(reread(&db).await, Some(registry(false).synced()));

        reread_done(&db).await.unwrap();
        assert!(!check_registry(&db, &registry(true)).await.unwrap());
        assert_eq!(reread(&db).await, None);
    }

    /// Spec 10.2: the owners publish every label on the first start of a
    /// registry the data file has not seen — a new file too — and not again.
    #[tokio::test]
    async fn a_registry_is_new_until_its_start_recorded_it() {
        let dir = tempfile::tempdir().unwrap();
        let db = db::open(
            &dir.path().join(db::DB_FILE),
            &[veydan_core::SCHEMA, crate::SCHEMA],
        )
        .await
        .unwrap();
        assert!(registry_is_new(&db, &registry(true)).await);
        check_registry(&db, &registry(true)).await.unwrap();
        assert!(!registry_is_new(&db, &registry(true)).await);
        assert!(registry_is_new(&db, &registry(false)).await);
        check_registry(&db, &registry(false)).await.unwrap();
        assert!(!registry_is_new(&db, &registry(false)).await);
    }

    /// A first fill of the labels that failed runs again at the next start,
    /// though that start recorded the registry, until one went through.
    #[tokio::test]
    async fn a_fill_of_the_labels_that_failed_runs_again_at_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        let db = db::open(
            &dir.path().join(db::DB_FILE),
            &[veydan_core::SCHEMA, crate::SCHEMA],
        )
        .await
        .unwrap();
        let start = |fill: CmdResult<usize>| {
            let db = db.clone();
            async move {
                let ran = fill_labels(&db, &registry(true), async { fill }).await;
                check_registry(&db, &registry(true)).await.unwrap();
                ran
            }
        };
        assert!(start(Err(AppError::other("disk full"))).await.is_err());
        assert!(start(Ok(4)).await.unwrap());
        assert!(!start(Ok(0)).await.unwrap());
        // A registry the data file has not recorded fills again.
        assert!(fill_labels(&db, &registry(false), async { Ok(0) })
            .await
            .unwrap());
    }
}
