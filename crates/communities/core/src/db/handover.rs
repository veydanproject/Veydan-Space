// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A keeper moved between devices (internal/communities-wire.md §15,
//! 2026-10-23): the take this device waits for (`cm_keeper_take`), and the
//! rows of a keeper as they leave one device and enter another.
//!
//! The rows go table by table, each row as the JSON object SQLite makes of
//! its columns: the tables of the keeper change with the migrations, and a
//! row of an older build still enters a newer one (a column it lacks takes
//! its default). Only the tables named here move, and a column enters only
//! when the table here has it: the names come from the blob of another
//! device and are never put into SQL unchecked.

use super::{storage, Db};
use messenger_core::Result;
use sqlx::Row;
use std::collections::{BTreeMap, BTreeSet};

/// The keeper's tables that move with it. Not `cm_keeper_events`: an
/// event signed by the old keeper's key is no use in the new one's range.
pub const TABLES: [&str; 19] = [
    "cm_keeper",
    "cm_keeper_header",
    "cm_keeper_members",
    "cm_keeper_revoked",
    "cm_keeper_pending",
    "cm_keeper_missed",
    "cm_keeper_blobs",
    "cm_keeper_uploads",
    "cm_keeper_retired",
    "cm_keeper_invites",
    "cm_keeper_ops",
    "cm_keeper_readers",
    "cm_keeper_snapshot",
    "cm_keeper_held_joins",
    "cm_keeper_wraps",
    "cm_keeper_inbox",
    "cm_keeper_flat",
    "cm_keeper_tickets",
    "cm_keeper_upload_keys",
];

/// Tables of the channel the keeper reads besides its own: the open
/// admissions of the moderators. Taken where this device has none.
pub const SHARED_TABLES: [&str; 1] = ["cm_admits"];

/// Rows of one table, each a JSON object of its columns.
pub type TableRows = Vec<serde_json::Map<String, serde_json::Value>>;

/// A take this device waits for.
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct TakeRow {
    pub community_id: String,
    pub keeper_pubkey: String,
    pub serial: i64,
    pub from_epoch: i64,
    pub mode: String,
    pub started_at: i64,
}

pub const MODE_HANDOVER: &str = "handover";
pub const MODE_EMERGENCY: &str = "emergency";
/// A new owner takes the keeper of the old one (wire §16).
pub const MODE_TRANSFER: &str = "transfer";

pub async fn put_take(db: &Db, row: &TakeRow) -> Result<()> {
    sqlx::query("INSERT OR REPLACE INTO cm_keeper_take (community_id, keeper_pubkey, serial, from_epoch, mode, started_at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(&row.community_id)
        .bind(&row.keeper_pubkey)
        .bind(row.serial)
        .bind(row.from_epoch)
        .bind(&row.mode)
        .bind(row.started_at)
        .execute(db.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn take(db: &Db, community_id: &str) -> Result<Option<TakeRow>> {
    sqlx::query_as::<_, TakeRow>("SELECT community_id, keeper_pubkey, serial, from_epoch, mode, started_at FROM cm_keeper_take WHERE community_id = ?")
        .bind(community_id)
        .fetch_optional(db.pool())
        .await
        .map_err(storage)
}

pub async fn drop_take(db: &Db, community_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM cm_keeper_take WHERE community_id = ?").bind(community_id).execute(db.pool()).await.map_err(storage)?;
    Ok(())
}

/// The columns of a table of ours (the name is one of `TABLES`).
/// Why each `AssertSqlSafe` here holds: a table name is one of `TABLES` or
/// `SHARED_TABLES` (constants), a column name one `PRAGMA table_info` gave
/// for it; values are always bound.
async fn columns(db: &Db, table: &str) -> Result<BTreeSet<String>> {
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!("PRAGMA table_info({table})"))).fetch_all(db.pool()).await.map_err(storage)?;
    rows.iter().map(|r| r.try_get::<String, _>("name").map_err(storage)).collect()
}

/// Every row of the keeper of a channel, by table.
pub async fn export(db: &Db, community_id: &str) -> Result<BTreeMap<String, TableRows>> {
    let mut out = BTreeMap::new();
    for table in TABLES.iter().chain(SHARED_TABLES.iter()) {
        let cols = columns(db, table).await?;
        let pairs: Vec<String> = cols.iter().map(|c| format!("'{c}', \"{c}\"")).collect();
        let sql = format!("SELECT json_object({}) FROM {table} WHERE community_id = ?", pairs.join(", "));
        let rows: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql)).bind(community_id).fetch_all(db.pool()).await.map_err(storage)?;
        let rows = rows.iter().map(|r| serde_json::from_str(r)).collect::<std::result::Result<TableRows, _>>()?;
        out.insert(table.to_string(), rows);
    }
    Ok(out)
}

/// Puts the rows of another device's keeper here, in one transaction: the
/// keeper's own tables replace whatever this device held of the channel,
/// the shared ones add what is not here. A table or a column this build
/// does not know is left out; every row is of `community_id`, whatever it
/// says.
pub async fn import(db: &Db, community_id: &str, tables: &BTreeMap<String, TableRows>) -> Result<()> {
    let mut known: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for table in TABLES.iter().chain(SHARED_TABLES.iter()) {
        known.insert(table, columns(db, table).await?);
    }
    let mut tx = db.write_tx().await?;
    for table in TABLES {
        sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table} WHERE community_id = ?"))).bind(community_id).execute(&mut *tx).await.map_err(storage)?;
    }
    for (table, rows) in tables {
        let Some((name, cols)) = known.get_key_value(table.as_str()) else { continue };
        let verb = if TABLES.contains(name) { "INSERT OR REPLACE" } else { "INSERT OR IGNORE" };
        for row in rows {
            let names: Vec<&String> = row.keys().filter(|c| cols.contains(*c) && c.as_str() != "community_id").collect();
            let marks = vec!["?"; names.len() + 1].join(", ");
            let list = names.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(", ");
            let sql = format!("{verb} INTO {name} (community_id{}{list}) VALUES ({marks})", if names.is_empty() { "" } else { ", " });
            let mut q = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(community_id);
            for c in names {
                q = match &row[c] {
                    serde_json::Value::Null => q.bind(None::<String>),
                    serde_json::Value::Bool(b) => q.bind(i64::from(*b)),
                    serde_json::Value::Number(n) => match n.as_i64() {
                        Some(i) => q.bind(i),
                        None => q.bind(n.as_f64().unwrap_or(0.0)),
                    },
                    serde_json::Value::String(s) => q.bind(s.clone()),
                    other => q.bind(other.to_string()),
                };
            }
            q.execute(&mut *tx).await.map_err(storage)?;
        }
    }
    tx.commit().await.map_err(storage)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_rows_of_a_keeper_move_whole_and_only_into_known_columns() {
        let a = Db::open_in_memory().await.unwrap();
        let b = Db::open_in_memory().await.unwrap();
        crate::db::keeper::put_keeper(&a, "c1", "kk", 5).await.unwrap();
        crate::db::keeper::reader_in(&a, "c1", "r1", 7).await.unwrap();
        crate::db::keeper::reader_in(&a, "c2", "r2", 7).await.unwrap();
        crate::db::keeper::reader_in(&b, "c1", "stale", 1).await.unwrap();
        let mut dump = export(&a, "c1").await.unwrap();
        assert_eq!(dump["cm_keeper_readers"].len(), 1, "of this channel only");
        // A column and a table another build may have: left out.
        dump.get_mut("cm_keeper_readers").unwrap()[0].insert("x\"; DROP TABLE cm_keeper; --".into(), serde_json::json!(1));
        dump.insert("cm_unknown".into(), vec![serde_json::Map::new()]);
        import(&b, "c1", &dump).await.unwrap();
        let since = crate::db::keeper::readers_since(&b, "c1").await.unwrap();
        assert_eq!(since.into_iter().collect::<Vec<_>>(), [("r1".to_string(), 7)], "the rows of this device gave way");
        assert_eq!(crate::db::keeper::keeper(&b, "c1").await.unwrap().map(|k| k.keeper_pubkey), Some("kk".into()));
    }
}
