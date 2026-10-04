// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_sync_cursors`: per scope and relay, the newest event time known to
//! be fully synced. Advance-only: out-of-order arrivals never move it back.

use crate::{storage, Store};
use messenger_core::Result;

/// Relay url used for cursors that are not relay-specific.
pub const ANY_RELAY: &str = "*";

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
}
