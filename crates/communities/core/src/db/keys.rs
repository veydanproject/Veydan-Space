// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `cm_epochs`, `cm_bundles`: what a member keeps of the keys of a scope
//! (internal/communities-wire.md §6). An epoch is a row from the moment
//! its event was taken (`epoch::Acceptance`), with its key once opened;
//! the acceptance and keydist's `MemberState` are rebuilt from the rows.

use super::{now, storage, Db};
use crate::epoch::Acceptance;
use keydist::{Bundle, EpochKey, Key32, MemberState, ScopeRef};
use messenger_core::Result;

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct EpochRow {
    pub community_id: String,
    pub scope_id: String,
    pub epoch: i64,
    /// Of the event's content, hex: two versions of an epoch differ here.
    pub digest: String,
    /// The newest generation the event carried.
    pub gen: Option<i64>,
    /// The epoch key, hex, once opened.
    pub key_hex: Option<String>,
    pub event_json: String,
    pub at: i64,
}

const E_COLS: &str = "community_id, scope_id, epoch, digest, gen, key_hex, event_json, at";

/// Keep an epoch taken; `false` when its event is here already. A row a
/// key made before its event came (`set_key`) gets the event.
pub async fn put_epoch(db: &Db, e: &EpochRow) -> Result<bool> {
    let res = sqlx::query(
        "INSERT INTO cm_epochs (community_id, scope_id, epoch, digest, gen, key_hex, event_json, at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(community_id, scope_id, epoch) DO UPDATE SET digest = excluded.digest, gen = excluded.gen,
           event_json = excluded.event_json, at = excluded.at
         WHERE cm_epochs.digest = ''",
    )
    .bind(&e.community_id)
    .bind(&e.scope_id)
    .bind(e.epoch)
    .bind(&e.digest)
    .bind(e.gen)
    .bind(&e.key_hex)
    .bind(&e.event_json)
    .bind(e.at)
    .execute(db.pool())
    .await
    .map_err(storage)?;
    Ok(res.rows_affected() == 1)
}

/// The key of an epoch, opened (from a shard, a chain, `channel.issue`).
/// A key arrives for an epoch whose event may not be here yet: the row
/// is made without it. A different key for an epoch is never written:
/// keydist refused it before (`Conflict`). `true` when the key was not
/// here before.
pub async fn set_key(db: &Db, community_id: &str, scope_id: &str, key: &EpochKey) -> Result<bool> {
    let res = sqlx::query(
        "INSERT INTO cm_epochs (community_id, scope_id, epoch, digest, gen, key_hex, event_json, at)
         VALUES (?, ?, ?, '', NULL, ?, '', 0)
         ON CONFLICT(community_id, scope_id, epoch) DO UPDATE SET key_hex = excluded.key_hex WHERE cm_epochs.key_hex IS NULL",
    )
    .bind(community_id)
    .bind(scope_id)
    .bind(i64::from(key.epoch))
    .bind(hex::encode(key.key.as_bytes()))
    .execute(db.pool())
    .await
    .map_err(storage)?;
    Ok(res.rows_affected() == 1)
}

pub async fn epochs(db: &Db, community_id: &str, scope_id: &str) -> Result<Vec<EpochRow>> {
    sqlx::query_as::<_, EpochRow>(sqlx::AssertSqlSafe(format!("SELECT {E_COLS} FROM cm_epochs WHERE community_id = ? AND scope_id = ? ORDER BY epoch")))
        .bind(community_id)
        .bind(scope_id)
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

/// Keep a bundle of generation `gen` (`Bundle::to_bytes`); at most the
/// two newest stay, and an emergency drops those before it.
pub async fn put_bundle(db: &Db, community_id: &str, scope_id: &str, gen: u32, bytes: &[u8], emergency: bool) -> Result<()> {
    let mut tx = db.write_tx().await?;
    sqlx::query(
        "INSERT INTO cm_bundles (community_id, scope_id, gen, bundle, emergency, received_at) VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(community_id, scope_id, gen) DO UPDATE SET bundle = excluded.bundle, emergency = excluded.emergency, received_at = excluded.received_at",
    )
    .bind(community_id)
    .bind(scope_id)
    .bind(i64::from(gen))
    .bind(bytes)
    .bind(emergency)
    .bind(now())
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    if emergency {
        sqlx::query("DELETE FROM cm_bundles WHERE community_id = ? AND scope_id = ? AND gen < ?")
            .bind(community_id)
            .bind(scope_id)
            .bind(i64::from(gen))
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
    }
    sqlx::query(
        "DELETE FROM cm_bundles WHERE community_id = ? AND scope_id = ? AND gen NOT IN
           (SELECT gen FROM cm_bundles WHERE community_id = ? AND scope_id = ? ORDER BY gen DESC LIMIT 2)",
    )
    .bind(community_id)
    .bind(scope_id)
    .bind(community_id)
    .bind(scope_id)
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    tx.commit().await.map_err(storage)
}

/// The bundles of a scope, oldest generation first, with whether each
/// came by an emergency.
pub async fn bundles(db: &Db, community_id: &str, scope_id: &str) -> Result<Vec<(i64, Vec<u8>, bool)>> {
    sqlx::query_as::<_, (i64, Vec<u8>, bool)>("SELECT gen, bundle, emergency FROM cm_bundles WHERE community_id = ? AND scope_id = ? ORDER BY gen")
        .bind(community_id)
        .bind(scope_id)
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

/// The current epoch of a scope: the newest whose event was taken.
pub async fn current_epoch(db: &Db, community_id: &str, scope_id: &str) -> Result<Option<u32>> {
    let max = sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(epoch) FROM cm_epochs WHERE community_id = ? AND scope_id = ? AND digest != ''")
        .bind(community_id)
        .bind(scope_id)
        .fetch_one(db.pool())
        .await
        .map_err(storage)?;
    Ok(max.and_then(|e| u32::try_from(e).ok()))
}

/// When the newest epoch of a channel taken here began (its keeper last
/// heard of), any scope.
pub async fn last_epoch_at(db: &Db, community_id: &str) -> Result<Option<i64>> {
    sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(at) FROM cm_epochs WHERE community_id = ? AND digest != ''")
        .bind(community_id)
        .fetch_one(db.pool())
        .await
        .map_err(storage)
}

/// The newest epoch taken here in any scope of a channel: where a new
/// keeper series may begin (wire §15: one number for every scope).
pub async fn newest_epoch(db: &Db, community_id: &str) -> Result<u32> {
    let max = sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(epoch) FROM cm_epochs WHERE community_id = ? AND digest != ''")
        .bind(community_id)
        .fetch_one(db.pool())
        .await
        .map_err(storage)?;
    Ok(max.and_then(|e| u32::try_from(e).ok()).unwrap_or(0))
}

/// The newest generation of a scope this device heard of, by the epochs
/// and its bundles (0: none): an emergency generation goes above it.
pub async fn newest_gen(db: &Db, community_id: &str, scope_id: &str) -> Result<u32> {
    let max = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(g) FROM (SELECT gen AS g FROM cm_epochs WHERE community_id = ? AND scope_id = ? UNION ALL SELECT gen AS g FROM cm_bundles WHERE community_id = ? AND scope_id = ?)",
    )
    .bind(community_id)
    .bind(scope_id)
    .bind(community_id)
    .bind(scope_id)
    .fetch_one(db.pool())
    .await
    .map_err(storage)?;
    Ok(max.and_then(|g| u32::try_from(g).ok()).unwrap_or(0))
}

/// Rebuild what a member holds of a scope from its rows: keydist's state
/// (bundles, keys, the newest emergency) and the epochs taken.
pub async fn load_member(db: &Db, community_id: &str, scope_id: &str, scope: ScopeRef) -> Result<(MemberState, Acceptance)> {
    let mut state = MemberState::new(scope);
    for (gen, bytes, emergency) in bundles(db, community_id, scope_id).await? {
        let b = Bundle::from_bytes(&bytes).map_err(|e| storage(format!("bundle of gen {gen}: {e}")))?;
        if emergency {
            state.emergency_gen = Some(state.emergency_gen.map_or(b.gen, |g| g.max(b.gen)));
        }
        state.bundles.push(b);
    }
    let mut acceptance = Acceptance::default();
    for row in epochs(db, community_id, scope_id).await? {
        let epoch = u32::try_from(row.epoch).map_err(storage)?;
        if let Some(k) = row.key_hex.as_deref() {
            let raw: [u8; 32] = hex::decode(k).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| storage("epoch key"))?;
            state.keys.insert(epoch, EpochKey { epoch, key: Key32(raw) });
        }
        if !row.digest.is_empty() {
            acceptance.seen.insert(epoch, row.digest.clone());
            acceptance.current = Some(acceptance.current.map_or(epoch, |c| c.max(epoch)));
            if let Some(g) = row.gen.and_then(|g| u32::try_from(g).ok()) {
                acceptance.newest_gen = Some(acceptance.newest_gen.map_or(g, |n| n.max(g)));
            }
        }
    }
    Ok((state, acceptance))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(epoch: i64, digest: &str) -> EpochRow {
        EpochRow {
            community_id: "c1".into(),
            scope_id: "everyone".into(),
            epoch,
            digest: digest.into(),
            gen: Some(1),
            key_hex: None,
            event_json: "{}".into(),
            at: 10,
        }
    }

    #[tokio::test]
    async fn epochs_and_keys_come_back_as_they_were_taken() {
        let db = Db::open_in_memory().await.unwrap();
        assert!(put_epoch(&db, &row(3, "d3")).await.unwrap());
        assert!(!put_epoch(&db, &row(3, "other")).await.unwrap(), "the first event of an epoch stays");
        put_epoch(&db, &row(5, "d5")).await.unwrap();
        let k3 = EpochKey { epoch: 3, key: Key32([3u8; 32]) };
        assert!(set_key(&db, "c1", "everyone", &k3).await.unwrap(), "a key new here");
        assert!(!set_key(&db, "c1", "everyone", &k3).await.unwrap(), "the same key again: not new");
        assert!(!set_key(&db, "c1", "everyone", &EpochKey { epoch: 3, key: Key32([9u8; 32]) }).await.unwrap());
        // A key before its event.
        let k4 = EpochKey { epoch: 4, key: Key32([4u8; 32]) };
        set_key(&db, "c1", "everyone", &k4).await.unwrap();
        let scope = crate::wire::scope_ref(crate::state::tests::CHANNEL, "everyone").unwrap();
        let (state, acceptance) = load_member(&db, "c1", "everyone", scope).await.unwrap();
        assert_eq!(state.key(3), Some(&k3), "a second key for an epoch is never written");
        assert_eq!(state.key(4), Some(&k4));
        assert_eq!(acceptance.current, Some(5));
        assert_eq!(acceptance.seen.len(), 2, "an epoch known by its key only was not taken as an event");
        assert_eq!(acceptance.newest_gen, Some(1));
    }

    #[tokio::test]
    async fn two_bundles_at_most_and_none_before_an_emergency() {
        let db = Db::open_in_memory().await.unwrap();
        for gen in 1..=3u32 {
            put_bundle(&db, "c1", "everyone", gen, &[gen as u8], false).await.unwrap();
        }
        assert_eq!(bundles(&db, "c1", "everyone").await.unwrap().iter().map(|b| b.0).collect::<Vec<_>>(), vec![2, 3]);
        put_bundle(&db, "c1", "everyone", 4, &[4], true).await.unwrap();
        assert_eq!(bundles(&db, "c1", "everyone").await.unwrap(), vec![(4, vec![4], true)]);
        assert!(bundles(&db, "c2", "everyone").await.unwrap().is_empty());
    }
}
