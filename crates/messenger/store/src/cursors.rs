// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_sync_cursors`: per scope and server, the newest event time known to
//! be fully synced. Advance-only: out-of-order arrivals never move it back.
//! The `relay_url` column names the server (migration 039): `*` for the
//! home set, as every cursor before servers did, the server id otherwise.

use crate::{storage, Store};
use messenger_core::Result;

/// The key of a cursor of the home set (and of one that is not
/// relay-specific, which every cursor before servers was).
pub const ANY_RELAY: &str = "*";

/// The key of a cursor of `server`: its id, or `ANY_RELAY` for the home set.
pub fn of_server(server: Option<&str>) -> &str {
    server.unwrap_or(ANY_RELAY)
}

pub async fn get(store: &Store, scope: &str, relay_url: &str) -> Result<Option<i64>> {
    sqlx::query_scalar::<_, i64>("SELECT last_event_at FROM msg_sync_cursors WHERE scope = ? AND relay_url = ?")
        .bind(scope)
        .bind(relay_url)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)
}

/// Move the cursor forward to `ts` if it is newer than the stored value.
pub async fn advance(store: &Store, scope: &str, relay_url: &str, ts: i64) -> Result<()> {
    sqlx::query(
        "INSERT INTO msg_sync_cursors (scope, relay_url, last_event_at) VALUES (?, ?, ?)
         ON CONFLICT(scope, relay_url) DO UPDATE SET last_event_at = MAX(last_event_at, excluded.last_event_at)",
    )
    .bind(scope)
    .bind(relay_url)
    .bind(ts)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

pub async fn reset(store: &Store, scope: &str) -> Result<()> {
    sqlx::query("DELETE FROM msg_sync_cursors WHERE scope = ?")
        .bind(scope)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn advance_is_monotonic() {
        let s = Store::open_in_memory().await.unwrap();
        assert_eq!(get(&s, "dm", ANY_RELAY).await.unwrap(), None);
        advance(&s, "dm", ANY_RELAY, 100).await.unwrap();
        advance(&s, "dm", ANY_RELAY, 50).await.unwrap();
        assert_eq!(get(&s, "dm", ANY_RELAY).await.unwrap(), Some(100));
        advance(&s, "dm", ANY_RELAY, 150).await.unwrap();
        assert_eq!(get(&s, "dm", ANY_RELAY).await.unwrap(), Some(150));
        reset(&s, "dm").await.unwrap();
        assert_eq!(get(&s, "dm", ANY_RELAY).await.unwrap(), None);
    }

    #[tokio::test]
    async fn each_server_keeps_its_own_cursor_and_home_is_the_old_key() {
        let s = Store::open_in_memory().await.unwrap();
        assert_eq!(of_server(None), ANY_RELAY);
        advance(&s, "dm", of_server(None), 100).await.unwrap();
        advance(&s, "dm", of_server(Some("0123456789abcdef")), 7).await.unwrap();
        assert_eq!(get(&s, "dm", ANY_RELAY).await.unwrap(), Some(100));
        assert_eq!(get(&s, "dm", "0123456789abcdef").await.unwrap(), Some(7));
        assert_eq!(get(&s, "dm", "fedcba9876543210").await.unwrap(), None);
    }
}
