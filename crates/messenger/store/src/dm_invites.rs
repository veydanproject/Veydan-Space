// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_dm_invites` and `msg_dm_invite_uses`: the invitations to write to
//! me and who used each (migration 030). Whether one admits somebody is
//! decided in `messenger-dm`; this is storage only. The uses are a set of
//! keys, so a use recorded twice (a replay, a copy of the same decision
//! from another device) stays one.

use crate::{storage, Store};
use messenger_core::Result;

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct InviteRow {
    /// Hex sha256 of the secret.
    pub id: String,
    pub secret: String,
    /// `link` | `code` | `qr`
    pub kind: String,
    pub label: String,
    pub created_at: i64,
    /// `None`: never runs out.
    pub expires_at: Option<i64>,
    /// `None`: any number.
    pub max_uses: Option<i64>,
    /// `None`: anybody who has the secret.
    pub for_pubkey: Option<String>,
    pub revoked_at: Option<i64>,
}


/// Insert an invitation; one with the same id is left as it is (the same
/// secret made twice is the same invitation). Returns whether a row was
/// written.
pub async fn create(store: &Store, r: &InviteRow) -> Result<bool> {
    let done = sqlx::query(
        "INSERT OR IGNORE INTO msg_dm_invites (id, secret, kind, label, created_at, expires_at, max_uses, for_pubkey, revoked_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&r.id)
    .bind(&r.secret)
    .bind(&r.kind)
    .bind(&r.label)
    .bind(r.created_at)
    .bind(r.expires_at)
    .bind(r.max_uses)
    .bind(&r.for_pubkey)
    .bind(r.revoked_at)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(done.rows_affected() > 0)
}

pub async fn get(store: &Store, id: &str) -> Result<Option<InviteRow>> {
    sqlx::query_as::<_, InviteRow>(
        "SELECT id, secret, kind, label, created_at, expires_at, max_uses, for_pubkey, revoked_at FROM msg_dm_invites WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(store.pool())
    .await
    .map_err(storage)
}

/// Every invitation, newest first; which of them still admit is the
/// caller's to tell (it needs the time and the uses).
pub async fn list(store: &Store) -> Result<Vec<InviteRow>> {
    sqlx::query_as::<_, InviteRow>(
        "SELECT id, secret, kind, label, created_at, expires_at, max_uses, for_pubkey, revoked_at FROM msg_dm_invites
         ORDER BY created_at DESC, id",
    )
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// Revoke at `at`; one revoked already keeps its time. Returns whether
/// the row changed.
pub async fn revoke(store: &Store, id: &str, at: i64) -> Result<bool> {
    let done = sqlx::query("UPDATE msg_dm_invites SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
        .bind(at)
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(done.rows_affected() > 0)
}

/// Record that `peer` came in by the invitation; a second record of the
/// same key is no record. Returns whether it was new.
pub async fn add_use(store: &Store, id: &str, peer: &str, at: i64) -> Result<bool> {
    let done = sqlx::query("INSERT OR IGNORE INTO msg_dm_invite_uses (invite_id, peer, at) VALUES (?, ?, ?)")
        .bind(id)
        .bind(peer)
        .bind(at)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(done.rows_affected() > 0)
}

/// Who used the invitation, in the order they came: `(peer, at)`.
pub async fn uses(store: &Store, id: &str) -> Result<Vec<(String, i64)>> {
    sqlx::query_as::<_, (String, i64)>("SELECT peer, at FROM msg_dm_invite_uses WHERE invite_id = ? ORDER BY at, peer")
        .bind(id)
        .fetch_all(store.pool())
        .await
        .map_err(storage)
}

/// An invitation as another device of mine holds it (the snapshot of my
/// relations): unknown here, it is taken whole; known, its revocation is
/// the later of the two and its uses are the union. Returns whether
/// anything changed here.
pub async fn merge(store: &Store, r: &InviteRow, uses: &[(String, i64)]) -> Result<bool> {
    let mut changed = create(store, r).await?;
    if let Some(at) = r.revoked_at {
        let done = sqlx::query("UPDATE msg_dm_invites SET revoked_at = ? WHERE id = ? AND (revoked_at IS NULL OR revoked_at < ?)")
            .bind(at)
            .bind(&r.id)
            .bind(at)
            .execute(store.pool())
            .await
            .map_err(storage)?;
        changed |= done.rows_affected() > 0;
    }
    for (peer, at) in uses {
        changed |= add_use(store, &r.id, peer, *at).await?;
    }
    Ok(changed)
}

/// Forget the invitations that ran out before `before`, with their uses:
/// a code shown every quarter of an hour would otherwise pile up.
pub async fn prune_expired(store: &Store, before: i64) -> Result<u64> {
    sqlx::query("DELETE FROM msg_dm_invite_uses WHERE invite_id IN (SELECT id FROM msg_dm_invites WHERE expires_at IS NOT NULL AND expires_at < ?)")
        .bind(before)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    let done = sqlx::query("DELETE FROM msg_dm_invites WHERE expires_at IS NOT NULL AND expires_at < ?")
        .bind(before)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(done.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str) -> InviteRow {
        InviteRow {
            id: id.into(),
            secret: format!("s-{id}"),
            kind: "link".into(),
            label: String::new(),
            created_at: 100,
            expires_at: Some(1_000),
            max_uses: Some(1),
            for_pubkey: None,
            revoked_at: None,
        }
    }

    #[tokio::test]
    async fn uses_are_a_set_and_a_merge_is_a_union_with_the_later_revocation() {
        let s = Store::open_in_memory().await.unwrap();
        assert!(create(&s, &row("a")).await.unwrap());
        assert!(!create(&s, &row("a")).await.unwrap(), "the same invitation twice is one");
        assert!(add_use(&s, "a", "p1", 10).await.unwrap());
        assert!(!add_use(&s, "a", "p1", 11).await.unwrap(), "a replay records nothing");
        assert_eq!(uses(&s, "a").await.unwrap(), vec![("p1".to_string(), 10)]);

        // Another device knew of p2 and revoked it later than nobody did here.
        let mut theirs = row("a");
        theirs.revoked_at = Some(500);
        assert!(merge(&s, &theirs, &[("p1".into(), 10), ("p2".into(), 20)]).await.unwrap());
        let got = get(&s, "a").await.unwrap().unwrap();
        assert_eq!(got.revoked_at, Some(500));
        assert_eq!(uses(&s, "a").await.unwrap().len(), 2);
        // An older revocation does not move it back; the same set changes nothing.
        theirs.revoked_at = Some(400);
        assert!(!merge(&s, &theirs, &[("p2".into(), 20)]).await.unwrap());
        assert_eq!(get(&s, "a").await.unwrap().unwrap().revoked_at, Some(500));

        assert!(!revoke(&s, "a", 600).await.unwrap(), "revoked already");
        create(&s, &row("b")).await.unwrap();
        assert!(revoke(&s, "b", 600).await.unwrap());
        assert_eq!(list(&s).await.unwrap().len(), 2);
        assert_eq!(prune_expired(&s, 2_000).await.unwrap(), 2);
        assert!(list(&s).await.unwrap().is_empty() && uses(&s, "a").await.unwrap().is_empty());
    }
}
