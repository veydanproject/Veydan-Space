// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_identity`: the public half of the local identity plus a reference to
//! where the host keeps the secret. Exactly one row (`id = 'primary'`) in
//! alpha; the column is there so multi-identity does not need a migration.

use crate::{storage, Store};
use messenger_core::Result;

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct IdentityRow {
    pub id: String,
    pub npub: String,
    pub pubkey_hex: String,
    pub nsec_ref: String,
    pub created_at: i64,
}

pub async fn get(store: &Store) -> Result<Option<IdentityRow>> {
    sqlx::query_as::<_, IdentityRow>(
        "SELECT id, npub, pubkey_hex, nsec_ref, created_at FROM msg_identity WHERE id = 'primary'",
    )
    .fetch_optional(store.pool())
    .await
    .map_err(storage)
}

pub async fn insert(store: &Store, row: &IdentityRow) -> Result<()> {
    sqlx::query(
        "INSERT INTO msg_identity (id, npub, pubkey_hex, nsec_ref, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&row.id)
    .bind(&row.npub)
    .bind(&row.pubkey_hex)
    .bind(&row.nsec_ref)
    .bind(row.created_at)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

pub async fn delete(store: &Store) -> Result<()> {
    sqlx::query("DELETE FROM msg_identity WHERE id = 'primary'")
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn insert_get_delete() {
        let store = Store::open_in_memory().await.unwrap();
        assert!(get(&store).await.unwrap().is_none());
        let row = IdentityRow {
            id: "primary".into(),
            npub: "npub1x".into(),
            pubkey_hex: "ab".repeat(32),
            nsec_ref: "identity.nsec".into(),
            created_at: 1,
        };
        insert(&store, &row).await.unwrap();
        let got = get(&store).await.unwrap().unwrap();
        assert_eq!(got.npub, "npub1x");
        assert!(insert(&store, &row).await.is_err(), "primary key is unique");
        delete(&store).await.unwrap();
        assert!(get(&store).await.unwrap().is_none());
    }
}
