// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `cm_snapshots`, `cm_readers`: the snapshots of the readers (§8) as
//! they came, the last two kept, and the shards opened from them, so that
//! a comment is judged without fetching its shard again.

use super::{now, storage, Db};
use crate::snapshot::{decode_shard, encode_shard, Entry};
use messenger_core::Result;

/// Keep a snapshot event; only the two newest of a channel stay.
pub async fn put_snapshot(db: &Db, community_id: &str, at: i64, event_json: &str) -> Result<()> {
    let mut tx = db.write_tx().await?;
    sqlx::query("INSERT OR IGNORE INTO cm_snapshots (community_id, at, event_json) VALUES (?, ?, ?)")
        .bind(community_id)
        .bind(at)
        .bind(event_json)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    sqlx::query("DELETE FROM cm_snapshots WHERE community_id = ? AND at NOT IN (SELECT at FROM cm_snapshots WHERE community_id = ? ORDER BY at DESC LIMIT 2)")
        .bind(community_id)
        .bind(community_id)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    tx.commit().await.map_err(storage)
}

/// The newest snapshot: its time and the event.
pub async fn latest(db: &Db, community_id: &str) -> Result<Option<(i64, String)>> {
    sqlx::query_as::<_, (i64, String)>("SELECT at, event_json FROM cm_snapshots WHERE community_id = ? ORDER BY at DESC LIMIT 1")
        .bind(community_id)
        .fetch_optional(db.pool())
        .await
        .map_err(storage)
}

/// Keep the opened shard of a range, in place of an older one.
pub async fn put_shard(db: &Db, community_id: &str, from: &str, sha256: &str, epoch: u32, entries: &[Entry]) -> Result<()> {
    sqlx::query(
        "INSERT INTO cm_readers (community_id, shard_from, sha256, epoch, entries, fetched_at) VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(community_id, shard_from) DO UPDATE SET sha256 = excluded.sha256, epoch = excluded.epoch,
           entries = excluded.entries, fetched_at = excluded.fetched_at",
    )
    .bind(community_id)
    .bind(from)
    .bind(sha256)
    .bind(i64::from(epoch))
    .bind(encode_shard(entries)?)
    .bind(now())
    .execute(db.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// The cached shard of a range, if it is the one of hash `sha256` (a
/// newer snapshot names another hash: the cache is stale then).
pub async fn shard(db: &Db, community_id: &str, from: &str, sha256: &str) -> Result<Option<Vec<Entry>>> {
    let bytes = sqlx::query_scalar::<_, Vec<u8>>("SELECT entries FROM cm_readers WHERE community_id = ? AND shard_from = ? AND sha256 = ?")
        .bind(community_id)
        .bind(from)
        .bind(sha256)
        .fetch_optional(db.pool())
        .await
        .map_err(storage)?;
    bytes.map(|b| decode_shard(&b)).transpose()
}

/// Forget the cached shards no snapshot names any more.
pub async fn retain_shards(db: &Db, community_id: &str, froms: &[String]) -> Result<u64> {
    let mut gone = 0;
    let all = sqlx::query_scalar::<_, String>("SELECT shard_from FROM cm_readers WHERE community_id = ?").bind(community_id).fetch_all(db.pool()).await.map_err(storage)?;
    for from in all.into_iter().filter(|f| !froms.contains(f)) {
        gone += sqlx::query("DELETE FROM cm_readers WHERE community_id = ? AND shard_from = ?")
            .bind(community_id)
            .bind(&from)
            .execute(db.pool())
            .await
            .map_err(storage)?
            .rows_affected();
    }
    Ok(gone)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::FLAG_READER;

    #[tokio::test]
    async fn the_two_newest_snapshots_and_the_shards_of_the_current_one() {
        let db = Db::open_in_memory().await.unwrap();
        for at in [10, 30, 20] {
            put_snapshot(&db, "c1", at, &format!("{{\"at\":{at}}}")).await.unwrap();
        }
        assert_eq!(latest(&db, "c1").await.unwrap().unwrap().0, 30);
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cm_snapshots").fetch_one(db.pool()).await.unwrap();
        assert_eq!(n, 2);

        let entries = vec![Entry { pubkey: [1u8; 32], flags: FLAG_READER, levels: 0, since: 5 }];
        put_shard(&db, "c1", "aa", "h1", 3, &entries).await.unwrap();
        assert_eq!(shard(&db, "c1", "aa", "h1").await.unwrap(), Some(entries.clone()));
        assert_eq!(shard(&db, "c1", "aa", "h2").await.unwrap(), None, "another hash: stale");
        put_shard(&db, "c1", "bb", "h3", 3, &entries).await.unwrap();
        assert_eq!(retain_shards(&db, "c1", &["bb".to_string()]).await.unwrap(), 1);
        assert_eq!(shard(&db, "c1", "aa", "h1").await.unwrap(), None);
    }
}
