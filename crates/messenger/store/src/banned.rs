// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_banned_keys` and `msg_banned_groups`: the keys the network refuses
//! and the groups it closes, as the manifest in use names them (migrations
//! 034 and 035). What a ban does lives in `messenger-dm` and
//! `messenger-groups`; the copy is kept by `messenger-runtime`.

use crate::{storage, Store};
use messenger_core::Result;
use std::collections::BTreeSet;

/// One of the two lists: its table and the column of its ids. Fixed words
/// of this file, never anything that came from outside, so they may be put
/// into the text of a query.
#[derive(Clone, Copy)]
struct Table {
    name: &'static str,
    column: &'static str,
}

const KEYS: Table = Table { name: "msg_banned_keys", column: "pubkey" };
const GROUPS: Table = Table { name: "msg_banned_groups", column: "group_id" };

/// Whether the network bans `pubkey` (64 lowercase hex).
pub async fn contains(store: &Store, pubkey: &str) -> Result<bool> {
    contains_in(store, KEYS, pubkey).await
}

pub async fn list(store: &Store) -> Result<Vec<String>> {
    list_in(store, KEYS).await
}

/// Make the table hold `keys` and nothing else, in one transaction, so
/// that no reader sees half a list. Returns the keys that came or went,
/// for the screens of their chats to be told; empty when nothing changed.
pub async fn replace(store: &Store, keys: &[String]) -> Result<Vec<String>> {
    replace_in(store, KEYS, keys).await
}

/// Whether the network closes the group `group_id` (the `h` of its events).
pub async fn group_contains(store: &Store, group_id: &str) -> Result<bool> {
    contains_in(store, GROUPS, group_id).await
}

pub async fn group_list(store: &Store) -> Result<Vec<String>> {
    list_in(store, GROUPS).await
}

/// [`replace`] for the groups: the ids that came or went.
pub async fn group_replace(store: &Store, groups: &[String]) -> Result<Vec<String>> {
    replace_in(store, GROUPS, groups).await
}

async fn contains_in(store: &Store, t: Table, id: &str) -> Result<bool> {
    let sql = format!("SELECT 1 FROM {} WHERE {} = ?", t.name, t.column);
    Ok(sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)?
        .is_some())
}

async fn list_in(store: &Store, t: Table) -> Result<Vec<String>> {
    let sql = format!("SELECT {c} FROM {} ORDER BY {c}", t.name, c = t.column);
    sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(sql))
        .fetch_all(store.pool())
        .await
        .map_err(storage)
}

async fn replace_in(store: &Store, t: Table, ids: &[String]) -> Result<Vec<String>> {
    let want: BTreeSet<&str> = ids.iter().map(String::as_str).collect();
    let have = list_in(store, t).await?;
    let have: BTreeSet<&str> = have.iter().map(String::as_str).collect();
    let came: Vec<&str> = want.difference(&have).copied().collect();
    let went: Vec<&str> = have.difference(&want).copied().collect();
    if came.is_empty() && went.is_empty() {
        return Ok(vec![]);
    }
    let delete = format!("DELETE FROM {} WHERE {} = ?", t.name, t.column);
    let insert = format!("INSERT OR IGNORE INTO {} ({}) VALUES (?)", t.name, t.column);
    let mut tx = store.pool().begin().await.map_err(storage)?;
    for id in &went {
        sqlx::query(sqlx::AssertSqlSafe(delete.as_str())).bind(id).execute(&mut *tx).await.map_err(storage)?;
    }
    for id in &came {
        sqlx::query(sqlx::AssertSqlSafe(insert.as_str())).bind(id).execute(&mut *tx).await.map_err(storage)?;
    }
    tx.commit().await.map_err(storage)?;
    Ok(came.into_iter().chain(went).map(String::from).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_table_follows_the_list_and_tells_what_changed() {
        let s = Store::open_in_memory().await.unwrap();
        let (a, b, c) = ("aa".repeat(32), "bb".repeat(32), "cc".repeat(32));
        assert!(!contains(&s, &a).await.unwrap());
        assert_eq!(replace(&s, &[a.clone(), b.clone()]).await.unwrap(), vec![a.clone(), b.clone()]);
        assert!(contains(&s, &a).await.unwrap() && contains(&s, &b).await.unwrap());
        assert!(replace(&s, &[b.clone(), a.clone()]).await.unwrap().is_empty(), "the same list changes nothing");
        assert_eq!(replace(&s, &[b.clone(), c.clone()]).await.unwrap(), vec![c.clone(), a.clone()], "c came, a went");
        assert_eq!(list(&s).await.unwrap(), vec![b.clone(), c.clone()]);
        assert_eq!(replace(&s, &[]).await.unwrap(), vec![b, c]);
        assert!(list(&s).await.unwrap().is_empty());
    }

    /// The groups are a list of their own: a group id named there bans no
    /// key of the same spelling, and the other way round.
    #[tokio::test]
    async fn the_groups_are_a_list_of_their_own() {
        let s = Store::open_in_memory().await.unwrap();
        let (g, h) = ("aa".repeat(32), "club-7".to_string());
        assert_eq!(group_replace(&s, &[g.clone(), h.clone()]).await.unwrap(), vec![g.clone(), h.clone()]);
        assert!(group_contains(&s, &g).await.unwrap() && group_contains(&s, &h).await.unwrap());
        assert!(!contains(&s, &g).await.unwrap(), "not a banned key");
        assert!(group_replace(&s, &[h.clone(), g.clone()]).await.unwrap().is_empty());
        replace(&s, std::slice::from_ref(&g)).await.unwrap();
        assert_eq!(group_replace(&s, std::slice::from_ref(&h)).await.unwrap(), vec![g.clone()], "g went");
        assert_eq!(group_list(&s).await.unwrap(), vec![h.clone()]);
        assert!(contains(&s, &g).await.unwrap(), "the keys are not touched");
        assert_eq!(group_replace(&s, &[]).await.unwrap(), vec![h]);
        assert!(group_list(&s).await.unwrap().is_empty());
    }
}
