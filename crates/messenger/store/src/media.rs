// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_media_servers` and `msg_transfers`.

use crate::{storage, Store};
use messenger_core::Result;

pub const KIND_S3: &str = "s3";
pub const KIND_BLOSSOM: &str = "blossom";

pub const DIR_UP: &str = "up";
pub const DIR_DOWN: &str = "down";

pub const ST_QUEUED: &str = "queued";
pub const ST_RUNNING: &str = "running";
pub const ST_PAUSED: &str = "paused";
pub const ST_DONE: &str = "done";
pub const ST_FAILED: &str = "failed";
pub const ST_CANCELLED: &str = "cancelled";

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct ServerRow {
    pub id: String,
    pub kind: String,
    pub url: String,
    pub bucket: Option<String>,
    pub region: Option<String>,
    pub access_key: Option<String>,
    pub priority: i64,
    pub enabled: bool,
    pub source: String,
    pub created_at: i64,
    pub updated_at: i64,
}

pub async fn servers(store: &Store) -> Result<Vec<ServerRow>> {
    sqlx::query_as::<_, ServerRow>(
        "SELECT id, kind, url, bucket, region, access_key, priority, enabled, source, created_at, updated_at
         FROM msg_media_servers ORDER BY priority ASC, created_at ASC",
    )
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

pub async fn server(store: &Store, id: &str) -> Result<Option<ServerRow>> {
    Ok(servers(store).await?.into_iter().find(|s| s.id == id))
}

/// Insert or update by id. `enabled` of an existing row is kept.
pub async fn upsert_server(store: &Store, r: &ServerRow) -> Result<()> {
    let now = crate::now();
    sqlx::query(
        "INSERT INTO msg_media_servers (id, kind, url, bucket, region, access_key, priority, enabled, source, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET kind = excluded.kind, url = excluded.url, bucket = excluded.bucket,
           region = excluded.region, access_key = COALESCE(excluded.access_key, msg_media_servers.access_key),
           priority = excluded.priority, source = excluded.source, updated_at = excluded.updated_at",
    )
    .bind(&r.id)
    .bind(&r.kind)
    .bind(&r.url)
    .bind(&r.bucket)
    .bind(&r.region)
    .bind(&r.access_key)
    .bind(r.priority)
    .bind(r.enabled)
    .bind(&r.source)
    .bind(now)
    .bind(now)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

pub async fn set_server_enabled(store: &Store, id: &str, enabled: bool) -> Result<()> {
    sqlx::query("UPDATE msg_media_servers SET enabled = ?, updated_at = ? WHERE id = ?")
        .bind(enabled)
        .bind(crate::now())
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn delete_server(store: &Store, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM msg_media_servers WHERE id = ?").bind(id).execute(store.pool()).await.map_err(storage)?;
    Ok(())
}

// ─── Transfers ──────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct TransferRow {
    pub id: String,
    pub direction: String,
    pub message_id: Option<String>,
    pub chat_id: Option<String>,
    pub local_path: Option<String>,
    pub file_name: String,
    pub mime: String,
    pub size: i64,
    pub sha256: Option<String>,
    pub status: String,
    pub done_bytes: i64,
    pub attempts: i64,
    pub failure_reason: Option<String>,
    pub state_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

const T_COLS: &str = "id, direction, message_id, chat_id, local_path, file_name, mime, size, sha256, status, done_bytes, attempts, failure_reason, state_json, created_at, updated_at";

pub async fn insert_transfer(store: &Store, r: &TransferRow) -> Result<()> {
    let now = crate::now();
    sqlx::query(
        "INSERT INTO msg_transfers (id, direction, message_id, chat_id, local_path, file_name, mime, size, sha256, status, done_bytes, attempts, failure_reason, state_json, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&r.id)
    .bind(&r.direction)
    .bind(&r.message_id)
    .bind(&r.chat_id)
    .bind(&r.local_path)
    .bind(&r.file_name)
    .bind(&r.mime)
    .bind(r.size)
    .bind(&r.sha256)
    .bind(&r.status)
    .bind(r.done_bytes)
    .bind(r.attempts)
    .bind(&r.failure_reason)
    .bind(&r.state_json)
    .bind(now)
    .bind(now)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

pub async fn transfer(store: &Store, id: &str) -> Result<Option<TransferRow>> {
    sqlx::query_as::<_, TransferRow>(sqlx::AssertSqlSafe(format!("SELECT {T_COLS} FROM msg_transfers WHERE id = ?")))
        .bind(id)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)
}

/// Newest transfer of a message in the given direction.
pub async fn transfer_for_message(store: &Store, message_id: &str, direction: &str) -> Result<Option<TransferRow>> {
    sqlx::query_as::<_, TransferRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {T_COLS} FROM msg_transfers WHERE message_id = ? AND direction = ? ORDER BY created_at DESC LIMIT 1"
    )))
    .bind(message_id)
    .bind(direction)
    .fetch_optional(store.pool())
    .await
    .map_err(storage)
}

pub async fn transfers_with_status(store: &Store, statuses: &[&str]) -> Result<Vec<TransferRow>> {
    let mut out = Vec::new();
    for s in statuses {
        let mut rows = sqlx::query_as::<_, TransferRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {T_COLS} FROM msg_transfers WHERE status = ? ORDER BY created_at ASC"
        )))
        .bind(*s)
        .fetch_all(store.pool())
        .await
        .map_err(storage)?;
        out.append(&mut rows);
    }
    Ok(out)
}

pub async fn set_progress(store: &Store, id: &str, done_bytes: i64, state_json: Option<&str>) -> Result<()> {
    sqlx::query("UPDATE msg_transfers SET done_bytes = ?, state_json = COALESCE(?, state_json), updated_at = ? WHERE id = ?")
        .bind(done_bytes)
        .bind(state_json)
        .bind(crate::now())
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn set_status(store: &Store, id: &str, status: &str, failure_reason: Option<&str>) -> Result<()> {
    sqlx::query("UPDATE msg_transfers SET status = ?, failure_reason = ?, updated_at = ? WHERE id = ?")
        .bind(status)
        .bind(failure_reason)
        .bind(crate::now())
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn set_attempts(store: &Store, id: &str, attempts: i64) -> Result<()> {
    sqlx::query("UPDATE msg_transfers SET attempts = ?, updated_at = ? WHERE id = ?")
        .bind(attempts)
        .bind(crate::now())
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn set_result(store: &Store, id: &str, local_path: Option<&str>, sha256: Option<&str>, message_id: Option<&str>) -> Result<()> {
    sqlx::query(
        "UPDATE msg_transfers SET local_path = COALESCE(?, local_path), sha256 = COALESCE(?, sha256),
           message_id = COALESCE(?, message_id), updated_at = ? WHERE id = ?",
    )
    .bind(local_path)
    .bind(sha256)
    .bind(message_id)
    .bind(crate::now())
    .bind(id)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// After a restart nothing is running: what was in flight becomes paused
/// (the user or the scheduler resumes it; nothing is lost).
pub async fn pause_interrupted(store: &Store) -> Result<u64> {
    let res = sqlx::query(
        "UPDATE msg_transfers SET status = 'paused', failure_reason = 'err.interrupted', updated_at = ?
         WHERE status IN ('queued', 'running')",
    )
    .bind(crate::now())
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(res.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(id: &str, dir: &str, msg: Option<&str>) -> TransferRow {
        TransferRow {
            id: id.into(),
            direction: dir.into(),
            message_id: msg.map(String::from),
            chat_id: Some("c".into()),
            local_path: Some("/tmp/x".into()),
            file_name: "x".into(),
            mime: "application/octet-stream".into(),
            size: 10,
            sha256: None,
            status: ST_QUEUED.into(),
            done_bytes: 0,
            attempts: 0,
            failure_reason: None,
            state_json: "{}".into(),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[tokio::test]
    async fn servers_and_transfers() {
        let s = Store::open_in_memory().await.unwrap();
        let mut srv = ServerRow {
            id: "a".into(),
            kind: KIND_S3.into(),
            url: "https://s3.example".into(),
            bucket: Some("b".into()),
            region: Some("us-east-1".into()),
            access_key: Some("k".into()),
            priority: 10,
            enabled: true,
            source: "user".into(),
            created_at: 0,
            updated_at: 0,
        };
        upsert_server(&s, &srv).await.unwrap();
        set_server_enabled(&s, "a", false).await.unwrap();
        srv.url = "https://new.example".into();
        srv.access_key = None;
        upsert_server(&s, &srv).await.unwrap();
        let got = server(&s, "a").await.unwrap().unwrap();
        assert_eq!(got.url, "https://new.example");
        assert!(!got.enabled, "enabled survives an update");
        assert_eq!(got.access_key.as_deref(), Some("k"), "missing access key keeps the stored one");
        delete_server(&s, "a").await.unwrap();
        assert!(servers(&s).await.unwrap().is_empty());

        insert_transfer(&s, &t("1", DIR_UP, None)).await.unwrap();
        insert_transfer(&s, &t("2", DIR_DOWN, Some("m"))).await.unwrap();
        set_status(&s, "1", ST_RUNNING, None).await.unwrap();
        set_progress(&s, "1", 5, Some(r#"{"k":1}"#)).await.unwrap();
        set_progress(&s, "1", 7, None).await.unwrap();
        let r = transfer(&s, "1").await.unwrap().unwrap();
        assert_eq!((r.done_bytes, r.state_json.as_str()), (7, r#"{"k":1}"#));
        assert_eq!(transfer_for_message(&s, "m", DIR_DOWN).await.unwrap().unwrap().id, "2");
        assert!(transfer_for_message(&s, "m", DIR_UP).await.unwrap().is_none());
        set_result(&s, "1", None, Some("sha"), Some("msg")).await.unwrap();
        assert_eq!(transfer(&s, "1").await.unwrap().unwrap().message_id.as_deref(), Some("msg"));

        assert_eq!(pause_interrupted(&s).await.unwrap(), 2);
        assert_eq!(transfers_with_status(&s, &[ST_PAUSED]).await.unwrap().len(), 2);
        set_attempts(&s, "2", -1).await.unwrap();
        assert_eq!(transfer(&s, "2").await.unwrap().unwrap().attempts, -1);
    }
}
