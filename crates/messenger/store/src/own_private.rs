// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_own_private`: what of my profile never goes into kind 0, my phone
//! and whether my own card carries it by default. One row. My devices tell
//! each other by own notes; the later `updated_at` wins, and at the same
//! time the larger value, so every device ends with the same.

use crate::{storage, Store};
use messenger_core::Result;

#[derive(Clone, Debug, Default, PartialEq, Eq, sqlx::FromRow)]
pub struct OwnPrivate {
    pub phone: Option<String>,
    pub share_phone: bool,
    /// 0 while nothing was ever set.
    pub updated_at: i64,
}

/// What is stored, or the default when nothing is.
pub async fn get(store: &Store) -> Result<OwnPrivate> {
    Ok(sqlx::query_as::<_, OwnPrivate>("SELECT phone, share_phone, updated_at FROM msg_own_private WHERE id = 1")
        .fetch_optional(store.pool())
        .await
        .map_err(storage)?
        .unwrap_or_default())
}

/// Keeps `value` when it is newer than what is stored (see the module).
/// `true` when the row changed.
pub async fn put_if_newer(store: &Store, value: &OwnPrivate) -> Result<bool> {
    let mut tx = store.pool().begin_with("BEGIN IMMEDIATE").await.map_err(storage)?;
    let old = sqlx::query_as::<_, OwnPrivate>("SELECT phone, share_phone, updated_at FROM msg_own_private WHERE id = 1")
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage)?;
    if !old.is_none_or(|o| wins(value, &o)) {
        tx.rollback().await.map_err(storage)?;
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO msg_own_private (id, phone, share_phone, updated_at) VALUES (1, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET phone = excluded.phone, share_phone = excluded.share_phone, updated_at = excluded.updated_at",
    )
    .bind(&value.phone)
    .bind(value.share_phone)
    .bind(value.updated_at)
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    tx.commit().await.map_err(storage)?;
    Ok(true)
}

/// A change made on this device: stamped `now`, or just after what is
/// stored when that is not earlier (a clock behind another device's), so
/// it always wins here. Returns what was stored, to be told to my other
/// devices.
pub async fn set(store: &Store, phone: Option<&str>, share_phone: bool, now: i64) -> Result<OwnPrivate> {
    let old = get(store).await?;
    let value = OwnPrivate { phone: phone.map(String::from), share_phone, updated_at: now.max(old.updated_at + 1) };
    put_if_newer(store, &value).await?;
    Ok(value)
}

fn wins(new: &OwnPrivate, old: &OwnPrivate) -> bool {
    (new.updated_at, &new.phone, new.share_phone) > (old.updated_at, &old.phone, old.share_phone)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(phone: Option<&str>, share: bool, at: i64) -> OwnPrivate {
        OwnPrivate { phone: phone.map(String::from), share_phone: share, updated_at: at }
    }

    #[tokio::test]
    async fn empty_until_set_then_later_wins() {
        let s = Store::open_in_memory().await.unwrap();
        assert_eq!(get(&s).await.unwrap(), OwnPrivate::default());
        assert!(put_if_newer(&s, &v(Some("+79991234567"), true, 10)).await.unwrap());
        assert!(!put_if_newer(&s, &v(None, false, 9)).await.unwrap(), "older note ignored");
        assert!(!put_if_newer(&s, &v(Some("+79991234567"), true, 10)).await.unwrap(), "the same again is no change");
        assert_eq!(get(&s).await.unwrap(), v(Some("+79991234567"), true, 10));
        assert!(put_if_newer(&s, &v(None, false, 11)).await.unwrap());
        assert_eq!(get(&s).await.unwrap(), v(None, false, 11));
    }

    #[tokio::test]
    async fn the_same_time_ends_the_same_in_any_order() {
        let a = v(Some("+15550001111"), false, 20);
        let b = v(Some("+15550002222"), false, 20);
        let s1 = Store::open_in_memory().await.unwrap();
        put_if_newer(&s1, &a).await.unwrap();
        put_if_newer(&s1, &b).await.unwrap();
        let s2 = Store::open_in_memory().await.unwrap();
        put_if_newer(&s2, &b).await.unwrap();
        put_if_newer(&s2, &a).await.unwrap();
        assert_eq!(get(&s1).await.unwrap(), get(&s2).await.unwrap());
    }

    #[tokio::test]
    async fn a_local_change_wins_over_a_clock_ahead() {
        let s = Store::open_in_memory().await.unwrap();
        put_if_newer(&s, &v(Some("+15550001111"), true, 1_000)).await.unwrap();
        let set_now = set(&s, None, false, 500).await.unwrap();
        assert_eq!(set_now, v(None, false, 1_001));
        assert_eq!(get(&s).await.unwrap(), set_now);
        assert_eq!(set(&s, Some("+15550003333"), true, 2_000).await.unwrap().updated_at, 2_000);
    }
}
