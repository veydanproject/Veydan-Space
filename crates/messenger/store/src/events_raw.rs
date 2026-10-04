// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_events_raw`: every wire event seen once. Primary dedup for the
//! ingress loop and the local id set for reconciliation.

use crate::{storage, Store};
use messenger_core::{EventId, RawEvent, Result};

/// Insert the event; returns `false` when it was already known.
pub async fn insert_if_new(store: &Store, raw: &RawEvent) -> Result<bool> {
    let res = sqlx::query(
        "INSERT OR IGNORE INTO msg_events_raw (event_id, kind, pubkey, created_at, received_at, source_json, raw_json)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(raw.id.as_hex())
    .bind(raw.kind as i64)
    .bind(raw.pubkey.as_hex())
    .bind(raw.created_at.secs())
    .bind(crate::now())
    .bind(serde_json::to_string(&raw.source).unwrap_or_else(|_| "{}".into()))
    .bind(raw.json.to_string())
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(res.rows_affected() == 1)
}

pub async fn contains(store: &Store, id: &EventId) -> Result<bool> {
    let n = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM msg_events_raw WHERE event_id = ?")
        .bind(id.as_hex())
        .fetch_one(store.pool())
        .await
        .map_err(storage)?;
    Ok(n > 0)
}

/// Ids of events of `kind` with `created_at` in `[since, until]`, for
/// reconciliation (Negentropy) or gap detection.
pub async fn ids_in_window(store: &Store, kind: u16, since: i64, until: i64, limit: i64) -> Result<Vec<EventId>> {
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT event_id FROM msg_events_raw WHERE kind = ? AND created_at BETWEEN ? AND ?
         ORDER BY created_at DESC LIMIT ?",
    )
    .bind(kind as i64)
    .bind(since)
    .bind(until)
    .bind(limit)
    .fetch_all(store.pool())
    .await
    .map_err(storage)?;
    Ok(rows.iter().filter_map(|s| EventId::parse(s)).collect())
}


/// `(id, created_at)` of the newest known events of `kind` since `since`,
/// the local set for a history reconciliation.
pub async fn items_since(store: &Store, kind: u16, since: i64, limit: i64) -> Result<Vec<(EventId, i64)>> {
    let rows = sqlx::query_as::<_, (String, i64)>(
        "SELECT event_id, created_at FROM msg_events_raw WHERE kind = ? AND created_at >= ?
         ORDER BY created_at DESC LIMIT ?",
    )
    .bind(kind as i64)
    .bind(since)
    .bind(limit)
    .fetch_all(store.pool())
    .await
    .map_err(storage)?;
    Ok(rows.into_iter().filter_map(|(id, at)| Some((EventId::parse(&id)?, at))).collect())
}

pub async fn count(store: &Store) -> Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM msg_events_raw")
        .fetch_one(store.pool())
        .await
        .map_err(storage)
}

/// Forget the wire events that carry this tag value, so that they are
/// taken again when they arrive (a group that was removed and re-joined).
pub async fn forget_tagged(store: &Store, kind: u16, value: &str) -> Result<u64> {
    if value.len() < 32 || !value.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Ok(0);
    }
    let res = sqlx::query("DELETE FROM msg_events_raw WHERE kind = ? AND raw_json LIKE ?")
        .bind(kind as i64)
        .bind(format!("%\"{value}\"%"))
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(res.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::{EventSource, PubKey, RelayUrl, Timestamp};

    fn raw(id: &str, kind: u16, ts: i64) -> RawEvent {
        RawEvent {
            id: EventId::parse(&id.repeat(64)).unwrap(),
            kind,
            pubkey: PubKey::parse(&"ab".repeat(32)).unwrap(),
            created_at: Timestamp(ts),
            json: serde_json::json!({"id": id}),
            source: EventSource::Relay { url: RelayUrl::parse("wss://r.example").unwrap() },
        }
    }

    #[tokio::test]
    async fn dedup_and_window() {
        let s = Store::open_in_memory().await.unwrap();
        assert!(insert_if_new(&s, &raw("1", 1059, 100)).await.unwrap());
        assert!(!insert_if_new(&s, &raw("1", 1059, 100)).await.unwrap(), "second insert is a no-op");
        assert!(insert_if_new(&s, &raw("2", 1059, 200)).await.unwrap());
        assert!(insert_if_new(&s, &raw("3", 9, 150)).await.unwrap());
        assert!(contains(&s, &EventId::parse(&"1".repeat(64)).unwrap()).await.unwrap());
        assert_eq!(count(&s).await.unwrap(), 3);
        assert_eq!(items_since(&s, 1059, 150, 10).await.unwrap().len(), 1);
        let ids = ids_in_window(&s, 1059, 0, 150, 10).await.unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].as_hex(), "1".repeat(64));
    }
}

