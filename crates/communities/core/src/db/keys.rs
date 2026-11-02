// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `cm_epochs`, `cm_bundles`: what a member keeps of the keys of a scope
//! (internal/communities-wire.md §6). An epoch is a row from the moment
//! its event was taken (`epoch::Acceptance`), with its key once opened;
//! the acceptance and keydist's `MemberState` are rebuilt from the rows,
//! whose keys and digests are kept beside them once read (`Mirrors`).

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

/// What `load_member` builds of the epoch rows of each scope, read once
/// and kept as the writes here change the rows (`put_epoch`, `set_key`,
/// `channels::delete`): every call read every row (8640 of a month) and
/// a member loads its keys several times an event, so that a reader back
/// after three days spent minutes on it (the bench of ROO-419). A scope
/// is put in only when nothing was written since its rows were read
/// (`writes`); each change applied here is one a row read again would
/// give anyway, so one applied twice changes nothing.
#[derive(Default)]
pub(crate) struct Mirrors {
    writes: u64,
    by_scope: std::collections::HashMap<(String, String), Mirror>,
}

#[derive(Clone)]
pub(crate) struct Mirror {
    keys: std::collections::BTreeMap<u32, EpochKey>,
    acceptance: Acceptance,
}

impl Mirror {
    /// The keys held, by epoch.
    pub(crate) fn keys(&self) -> &std::collections::BTreeMap<u32, EpochKey> {
        &self.keys
    }

    /// The digest of each event taken, by epoch.
    pub(crate) fn digests(&self) -> &std::collections::BTreeMap<u32, String> {
        &self.acceptance.seen
    }
}

impl Mirrors {
    fn changed(&mut self, community_id: &str, scope_id: &str) -> Option<&mut Mirror> {
        self.writes += 1;
        self.by_scope.get_mut(&(community_id.to_string(), scope_id.to_string()))
    }

    fn forget_scope(&mut self, community_id: &str, scope_id: &str) {
        self.by_scope.remove(&(community_id.to_string(), scope_id.to_string()));
    }

    /// A channel forgotten (`channels::delete`).
    pub(crate) fn forget(&mut self, community_id: &str) {
        self.writes += 1;
        self.by_scope.retain(|(c, _), _| c != community_id);
    }
}

fn key_of_hex(epoch: u32, k: &str) -> Result<EpochKey> {
    let raw: [u8; 32] = hex::decode(k).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| storage("epoch key"))?;
    Ok(EpochKey { epoch, key: Key32(raw) })
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
    let taken = res.rows_affected() == 1;
    let mut mirrors = db.mirrors();
    let epoch = u32::try_from(e.epoch).ok().filter(|_| e.key_hex.is_none());
    match (mirrors.changed(&e.community_id, &e.scope_id), epoch) {
        (Some(m), Some(epoch)) if taken && !e.digest.is_empty() => {
            m.acceptance.seen.insert(epoch, e.digest.clone());
            m.acceptance.current = Some(m.acceptance.current.map_or(epoch, |c| c.max(epoch)));
            if let Some(g) = e.gen.and_then(|g| u32::try_from(g).ok()) {
                m.acceptance.newest_gen = Some(m.acceptance.newest_gen.map_or(g, |n| n.max(g)));
            }
        }
        // A key with the event: a new row has it, one there already keeps
        // its own; read again.
        (Some(_), None) if taken => mirrors.forget_scope(&e.community_id, &e.scope_id),
        _ => {}
    }
    Ok(taken)
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
    let new = res.rows_affected() == 1;
    if let Some(m) = db.mirrors().changed(community_id, scope_id) {
        if new {
            m.keys.insert(key.epoch, key.clone());
        }
    }
    Ok(new)
}

pub async fn epochs(db: &Db, community_id: &str, scope_id: &str) -> Result<Vec<EpochRow>> {
    sqlx::query_as::<_, EpochRow>(sqlx::AssertSqlSafe(format!("SELECT {E_COLS} FROM cm_epochs WHERE community_id = ? AND scope_id = ? ORDER BY epoch")))
        .bind(community_id)
        .bind(scope_id)
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

/// `epochs` without the events (`event_json` empty in every row): what
/// a call that reads the keys and the dates needs, at every epoch of a
/// channel of months (the bench of ROO-419: the events of 8640 epochs
/// read at every turn).
pub async fn epochs_bare(db: &Db, community_id: &str, scope_id: &str) -> Result<Vec<EpochRow>> {
    sqlx::query_as::<_, EpochRow>("SELECT community_id, scope_id, epoch, digest, gen, key_hex, '' AS event_json, at FROM cm_epochs WHERE community_id = ? AND scope_id = ? ORDER BY epoch")
        .bind(community_id)
        .bind(scope_id)
        .fetch_all(db.pool())
        .await
        .map_err(storage)
}

/// The rows whose event is here and whose key is not, by epoch.
/// Which epochs those are taken from `Mirrors` (an event taken gives a
/// digest), then read one by one: the bundles looked for at every turn
/// read every row of a channel of months.
pub async fn epochs_unopened(db: &Db, community_id: &str, scope_id: &str) -> Result<Vec<EpochRow>> {
    let epochs = unopened(db, community_id, scope_id).await?;
    let mut out = Vec::new();
    for epoch in epochs {
        if let Some(row) = epoch_row(db, community_id, scope_id, i64::from(epoch)).await?.filter(|r| r.key_hex.is_none() && !r.event_json.is_empty()) {
            out.push(row);
        }
    }
    Ok(out)
}

/// The row of one epoch.
pub async fn epoch_row(db: &Db, community_id: &str, scope_id: &str, epoch: i64) -> Result<Option<EpochRow>> {
    sqlx::query_as::<_, EpochRow>(sqlx::AssertSqlSafe(format!("SELECT {E_COLS} FROM cm_epochs WHERE community_id = ? AND scope_id = ? AND epoch = ?")))
        .bind(community_id)
        .bind(scope_id)
        .bind(epoch)
        .fetch_optional(db.pool())
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
    let max = sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(epoch) FROM (SELECT epoch FROM cm_epochs WHERE community_id = ? AND scope_id = ? AND digest != '' ORDER BY epoch DESC LIMIT 1)")
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
/// keeper series may begin (wire §15: one number for every scope). Each
/// scope by the primary key from its newest row down, the scopes one
/// after the other by it too: asked at every epoch event heard, and
/// `MAX(epoch)` over the channel read every row of it.
pub async fn newest_epoch(db: &Db, community_id: &str) -> Result<u32> {
    let max = sqlx::query_scalar::<_, Option<i64>>(
        "WITH RECURSIVE s(scope) AS (
           SELECT MIN(scope_id) FROM cm_epochs WHERE community_id = ?1
           UNION ALL
           SELECT (SELECT MIN(scope_id) FROM cm_epochs WHERE community_id = ?1 AND scope_id > s.scope) FROM s WHERE s.scope IS NOT NULL
         )
         SELECT MAX((SELECT epoch FROM cm_epochs WHERE community_id = ?1 AND scope_id = s.scope AND digest != '' ORDER BY epoch DESC LIMIT 1)) FROM s WHERE s.scope IS NOT NULL",
    )
    .bind(community_id)
    .fetch_one(db.pool())
    .await
    .map_err(storage)?;
    Ok(max.and_then(|e| u32::try_from(e).ok()).unwrap_or(0))
}

/// The newest epoch number heard of a channel in any epoch event of a
/// keeper, taken or not (`ChannelService::note_epoch_heard`): 0 when none.
pub async fn epoch_heard(db: &Db, community_id: &str) -> Result<u32> {
    let e = sqlx::query_scalar::<_, i64>("SELECT epoch FROM cm_epochs_heard WHERE community_id = ?")
        .bind(community_id)
        .fetch_optional(db.pool())
        .await
        .map_err(storage)?;
    Ok(e.and_then(|e| u32::try_from(e).ok()).unwrap_or(0))
}

/// `epoch_heard` raised to `epoch` (never lowered).
pub async fn note_epoch_heard(db: &Db, community_id: &str, epoch: u32) -> Result<()> {
    sqlx::query("INSERT INTO cm_epochs_heard (community_id, epoch) VALUES (?, ?) ON CONFLICT(community_id) DO UPDATE SET epoch = MAX(epoch, excluded.epoch)")
        .bind(community_id)
        .bind(i64::from(epoch))
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
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

/// `f` of what the rows of a scope give (`Mirrors`), read of them the
/// first time.
pub(crate) async fn with_mirror<T>(db: &Db, community_id: &str, scope_id: &str, f: impl FnOnce(&Mirror) -> T) -> Result<T> {
    let at = (community_id.to_string(), scope_id.to_string());
    let writes = {
        let m = db.mirrors();
        if let Some(mirror) = m.by_scope.get(&at) {
            return Ok(f(mirror));
        }
        m.writes
    };
    let mut m = Mirror { keys: Default::default(), acceptance: Acceptance::default() };
    for row in epochs_bare(db, community_id, scope_id).await? {
        let epoch = u32::try_from(row.epoch).map_err(storage)?;
        if let Some(k) = row.key_hex.as_deref() {
            m.keys.insert(epoch, key_of_hex(epoch, k)?);
        }
        if !row.digest.is_empty() {
            m.acceptance.seen.insert(epoch, row.digest.clone());
            m.acceptance.current = Some(m.acceptance.current.map_or(epoch, |c| c.max(epoch)));
            if let Some(g) = row.gen.and_then(|g| u32::try_from(g).ok()) {
                m.acceptance.newest_gen = Some(m.acceptance.newest_gen.map_or(g, |n| n.max(g)));
            }
        }
    }
    let out = f(&m);
    let mut mirrors = db.mirrors();
    if mirrors.writes == writes {
        mirrors.by_scope.insert(at, m);
    }
    Ok(out)
}

/// The epochs of a scope whose event is here and whose key is not, by
/// number (`epochs_unopened` without the rows).
pub async fn unopened(db: &Db, community_id: &str, scope_id: &str) -> Result<Vec<u32>> {
    with_mirror(db, community_id, scope_id, |m| m.acceptance.seen.keys().filter(|e| !m.keys.contains_key(e)).copied().collect()).await
}

/// Whether the key of `epoch` is held.
pub async fn holds_key(db: &Db, community_id: &str, scope_id: &str, epoch: u32) -> Result<bool> {
    with_mirror(db, community_id, scope_id, |m| m.keys.contains_key(&epoch)).await
}

/// The newest `n` keys held of a scope, oldest first.
pub async fn newest_keys(db: &Db, community_id: &str, scope_id: &str, n: usize) -> Result<Vec<EpochKey>> {
    let mut keys = with_mirror(db, community_id, scope_id, |m| m.keys.values().rev().take(n).cloned().collect::<Vec<_>>()).await?;
    keys.reverse();
    Ok(keys)
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
    let mirror = with_mirror(db, community_id, scope_id, Mirror::clone).await?;
    state.keys = mirror.keys;
    let acceptance = mirror.acceptance;
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

    /// What `load_member` reads once and keeps (`Mirrors`) follows every
    /// write after it: as the rows read again by a database with nothing
    /// kept give it, the epochs to open and the newest keys too.
    #[tokio::test]
    async fn the_keys_kept_beside_the_rows_follow_every_write() {
        let db = Db::open_in_memory().await.unwrap();
        let fresh = || Db { pool: db.pool.clone(), mirrors: Default::default(), retired: Default::default() };
        let scope = || crate::wire::scope_ref(crate::state::tests::CHANNEL, "everyone").unwrap();
        put_epoch(&db, &row(1, "d1")).await.unwrap();
        load_member(&db, "c1", "everyone", scope()).await.unwrap();
        let k2 = EpochKey { epoch: 2, key: Key32([2u8; 32]) };
        set_key(&db, "c1", "everyone", &k2).await.unwrap();
        put_epoch(&db, &row(2, "d2")).await.unwrap();
        put_epoch(&db, &EpochRow { gen: Some(3), ..row(4, "d4") }).await.unwrap();
        put_epoch(&db, &EpochRow { key_hex: Some(hex::encode([5u8; 32])), ..row(5, "d5") }).await.unwrap();
        assert!(!put_epoch(&db, &row(4, "other")).await.unwrap());
        let k1 = EpochKey { epoch: 1, key: Key32([1u8; 32]) };
        set_key(&db, "c1", "everyone", &k1).await.unwrap();
        let (kept, kept_acc) = load_member(&db, "c1", "everyone", scope()).await.unwrap();
        let (read, read_acc) = load_member(&fresh(), "c1", "everyone", scope()).await.unwrap();
        assert_eq!(kept.keys, read.keys);
        assert_eq!(kept_acc, read_acc);
        assert_eq!(read_acc.newest_gen, Some(3));
        assert_eq!(unopened(&db, "c1", "everyone").await.unwrap(), vec![4]);
        assert_eq!(epochs_unopened(&db, "c1", "everyone").await.unwrap().iter().map(|r| r.epoch).collect::<Vec<_>>(), vec![4]);
        assert_eq!(newest_keys(&db, "c1", "everyone", 2).await.unwrap().iter().map(|k| k.epoch).collect::<Vec<_>>(), vec![2, 5]);
        assert!(holds_key(&db, "c1", "everyone", 1).await.unwrap() && !holds_key(&db, "c1", "everyone", 4).await.unwrap());
        // A channel forgotten forgets what was kept of it.
        crate::db::channels::delete(&db, "c1").await.unwrap();
        assert!(load_member(&db, "c1", "everyone", scope()).await.unwrap().0.keys.is_empty());
        assert!(unopened(&db, "c1", "everyone").await.unwrap().is_empty());
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
