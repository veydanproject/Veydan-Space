// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_contact_private`: what I know of a contact privately, from the card
//! it sent me about itself: its phone. My devices tell each other by own
//! notes; per contact the later `updated_at` wins, and at the same time the
//! larger value, so every device ends with the same. A phone taken back
//! stays as a row without a phone, so an older note does not bring it back.

use crate::{storage, Store};
use messenger_core::Result;

#[derive(Clone, Debug, Default, PartialEq, Eq, sqlx::FromRow)]
pub struct ContactPrivate {
    pub pubkey: String,
    pub phone: Option<String>,
    pub updated_at: i64,
}

pub async fn get(store: &Store, pubkey: &str) -> Result<Option<ContactPrivate>> {
    sqlx::query_as::<_, ContactPrivate>("SELECT pubkey, phone, updated_at FROM msg_contact_private WHERE pubkey = ?")
        .bind(pubkey)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)
}

/// Keeps `value` when it is newer than what is stored for its contact (see
/// the module). `true` when the row changed.
pub async fn put_if_newer(store: &Store, value: &ContactPrivate) -> Result<bool> {
    let mut tx = store.pool().begin_with("BEGIN IMMEDIATE").await.map_err(storage)?;
    let old = sqlx::query_as::<_, ContactPrivate>("SELECT pubkey, phone, updated_at FROM msg_contact_private WHERE pubkey = ?")
        .bind(&value.pubkey)
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage)?;
    if !old.is_none_or(|o| (value.updated_at, &value.phone) > (o.updated_at, &o.phone)) {
        tx.rollback().await.map_err(storage)?;
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO msg_contact_private (pubkey, phone, updated_at) VALUES (?, ?, ?)
         ON CONFLICT(pubkey) DO UPDATE SET phone = excluded.phone, updated_at = excluded.updated_at",
    )
    .bind(&value.pubkey)
    .bind(&value.phone)
    .bind(value.updated_at)
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    tx.commit().await.map_err(storage)?;
    Ok(true)
}

/// A change made on this device: stamped `now`, or just after what is
/// stored when that is not earlier, so it always wins here. Returns what
/// was stored, to be told to my other devices.
pub async fn set(store: &Store, pubkey: &str, phone: Option<&str>, now: i64) -> Result<ContactPrivate> {
    let old_at = get(store, pubkey).await?.map_or(0, |o| o.updated_at);
    let value = ContactPrivate { pubkey: pubkey.into(), phone: phone.map(String::from), updated_at: now.max(old_at + 1) };
    put_if_newer(store, &value).await?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(pk: &str, phone: Option<&str>, at: i64) -> ContactPrivate {
        ContactPrivate { pubkey: pk.into(), phone: phone.map(String::from), updated_at: at }
    }

    #[tokio::test]
    async fn per_contact_later_wins() {
        let s = Store::open_in_memory().await.unwrap();
        assert_eq!(get(&s, "a").await.unwrap(), None);
        assert!(put_if_newer(&s, &v("a", Some("+15550001111"), 10)).await.unwrap());
        assert!(put_if_newer(&s, &v("b", Some("+15550002222"), 5)).await.unwrap(), "another contact is its own");
        assert!(!put_if_newer(&s, &v("a", None, 9)).await.unwrap());
        assert!(put_if_newer(&s, &v("a", None, 11)).await.unwrap(), "taken back");
        assert!(!put_if_newer(&s, &v("a", Some("+15550001111"), 10)).await.unwrap(), "an older note does not bring it back");
        assert_eq!(get(&s, "a").await.unwrap(), Some(v("a", None, 11)));
        assert_eq!(get(&s, "b").await.unwrap(), Some(v("b", Some("+15550002222"), 5)));
    }

    #[tokio::test]
    async fn the_same_time_ends_the_same_and_local_changes_win() {
        let s = Store::open_in_memory().await.unwrap();
        put_if_newer(&s, &v("a", Some("+15550002222"), 20)).await.unwrap();
        assert!(!put_if_newer(&s, &v("a", Some("+15550001111"), 20)).await.unwrap(), "the larger stays");
        assert!(put_if_newer(&s, &v("a", Some("+15550003333"), 20)).await.unwrap());
        assert_eq!(set(&s, "a", None, 7).await.unwrap(), v("a", None, 21));
        assert_eq!(set(&s, "c", Some("+15550004444"), 7).await.unwrap(), v("c", Some("+15550004444"), 7));
    }
}
