// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_profiles`: cache of kind-0 metadata, ours included. Rows are
//! replaced only by a newer `event_created_at` (last-writer-wins).

use crate::{storage, Store};
use messenger_core::Result;

#[derive(Clone, Debug, Default, PartialEq, Eq, sqlx::FromRow)]
pub struct ProfileRow {
    pub pubkey: String,
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub about: Option<String>,
    pub picture: Option<String>,
    pub banner: Option<String>,
    pub website: Option<String>,
    pub nip05: Option<String>,
    pub lud16: Option<String>,
    pub nip05_verified_at: Option<i64>,
    pub event_created_at: i64,
    pub fetched_at: i64,
    pub raw_json: String,
}

const COLS: &str = "pubkey, name, display_name, about, picture, banner, website, nip05, lud16, nip05_verified_at, event_created_at, fetched_at, raw_json";

pub async fn get(store: &Store, pubkey: &str) -> Result<Option<ProfileRow>> {
    sqlx::query_as::<_, ProfileRow>(
        "SELECT pubkey, name, display_name, about, picture, banner, website, nip05, lud16, nip05_verified_at, event_created_at, fetched_at, raw_json
         FROM msg_profiles WHERE pubkey = ?",
    )
    .bind(pubkey)
    .fetch_optional(store.pool())
    .await
    .map_err(storage)
}

pub async fn get_many(store: &Store, pubkeys: &[String]) -> Result<Vec<ProfileRow>> {
    let mut out = Vec::with_capacity(pubkeys.len());
    for pk in pubkeys {
        if let Some(r) = get(store, pk).await? {
            out.push(r);
        }
    }
    Ok(out)
}

/// Insert or replace when `row.event_created_at` is newer than the stored
/// one. Returns `true` when the row changed. `nip05_verified_at` is
/// preserved unless the nip05 value changed.
pub async fn upsert_if_newer(store: &Store, row: &ProfileRow) -> Result<bool> {
    let res = sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO msg_profiles ({COLS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(pubkey) DO UPDATE SET
               name = excluded.name, display_name = excluded.display_name, about = excluded.about,
               picture = excluded.picture, banner = excluded.banner, website = excluded.website,
               nip05 = excluded.nip05, lud16 = excluded.lud16,
               nip05_verified_at = CASE WHEN msg_profiles.nip05 IS excluded.nip05 THEN msg_profiles.nip05_verified_at ELSE NULL END,
               event_created_at = excluded.event_created_at, fetched_at = excluded.fetched_at, raw_json = excluded.raw_json
             WHERE excluded.event_created_at > msg_profiles.event_created_at"
    )))
    .bind(&row.pubkey)
    .bind(&row.name)
    .bind(&row.display_name)
    .bind(&row.about)
    .bind(&row.picture)
    .bind(&row.banner)
    .bind(&row.website)
    .bind(&row.nip05)
    .bind(&row.lud16)
    .bind(row.nip05_verified_at)
    .bind(row.event_created_at)
    .bind(crate::now())
    .bind(&row.raw_json)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(res.rows_affected() == 1)
}

pub async fn set_nip05_verified(store: &Store, pubkey: &str, verified_at: Option<i64>) -> Result<()> {
    sqlx::query("UPDATE msg_profiles SET nip05_verified_at = ? WHERE pubkey = ?")
        .bind(verified_at)
        .bind(pubkey)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// Case-insensitive search over name/display_name/nip05/pubkey prefix.
pub async fn search(store: &Store, query: &str, limit: i64) -> Result<Vec<ProfileRow>> {
    let like = format!("%{}%", query.trim().to_lowercase());
    sqlx::query_as::<_, ProfileRow>(
        "SELECT pubkey, name, display_name, about, picture, banner, website, nip05, lud16, nip05_verified_at, event_created_at, fetched_at, raw_json
         FROM msg_profiles
         WHERE lower(COALESCE(name,'')) LIKE ? OR lower(COALESCE(display_name,'')) LIKE ? OR lower(COALESCE(nip05,'')) LIKE ? OR pubkey LIKE ?
         ORDER BY fetched_at DESC LIMIT ?",
    )
    .bind(&like)
    .bind(&like)
    .bind(&like)
    .bind(&like)
    .bind(limit)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pk: &str, name: &str, created: i64) -> ProfileRow {
        ProfileRow { pubkey: pk.into(), name: Some(name.into()), event_created_at: created, ..Default::default() }
    }

    #[tokio::test]
    async fn last_writer_wins_by_event_time() {
        let s = Store::open_in_memory().await.unwrap();
        assert!(upsert_if_newer(&s, &row("a", "one", 10)).await.unwrap());
        assert!(!upsert_if_newer(&s, &row("a", "older", 5)).await.unwrap(), "older event ignored");
        assert_eq!(get(&s, "a").await.unwrap().unwrap().name.as_deref(), Some("one"));
        assert!(upsert_if_newer(&s, &row("a", "two", 20)).await.unwrap());
        assert_eq!(get(&s, "a").await.unwrap().unwrap().name.as_deref(), Some("two"));
    }

    #[tokio::test]
    async fn nip05_verification_survives_unless_nip05_changes() {
        let s = Store::open_in_memory().await.unwrap();
        let mut r = row("a", "x", 1);
        r.nip05 = Some("x@e.com".into());
        upsert_if_newer(&s, &r).await.unwrap();
        set_nip05_verified(&s, "a", Some(99)).await.unwrap();
        r.event_created_at = 2;
        r.name = Some("y".into());
        upsert_if_newer(&s, &r).await.unwrap();
        assert_eq!(get(&s, "a").await.unwrap().unwrap().nip05_verified_at, Some(99));
        r.event_created_at = 3;
        r.nip05 = Some("other@e.com".into());
        upsert_if_newer(&s, &r).await.unwrap();
        assert_eq!(get(&s, "a").await.unwrap().unwrap().nip05_verified_at, None);
    }

    #[tokio::test]
    async fn search_matches_name_and_prefix() {
        let s = Store::open_in_memory().await.unwrap();
        upsert_if_newer(&s, &row("abcdef", "Alice", 1)).await.unwrap();
        upsert_if_newer(&s, &row("ffffff", "Bob", 1)).await.unwrap();
        assert_eq!(search(&s, "ali", 10).await.unwrap().len(), 1);
        assert_eq!(search(&s, "abc", 10).await.unwrap().len(), 1);
        assert_eq!(search(&s, "zzz", 10).await.unwrap().len(), 0);
        assert_eq!(get_many(&s, &["abcdef".into(), "nope".into()]).await.unwrap().len(), 1);
    }
}
