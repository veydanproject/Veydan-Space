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
/// Failed for a reason that may pass; the next attempt starts by itself
/// after a pause, or at once when the user asks.
pub const ST_WAITING_RETRY: &str = "waiting_retry";
/// Why a transfer is paused that was under way when its process ended or
/// its service stopped (the app closed). A pause the user made has none.
pub const REASON_INTERRUPTED: &str = "err.interrupted";

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

/// Transfers in any of `statuses`, the newest first (in the order they
/// were made, also within one second).
pub async fn transfers_newest_first(store: &Store, statuses: &[&str]) -> Result<Vec<TransferRow>> {
    let marks = vec!["?"; statuses.len()].join(", ");
    let sql = format!("SELECT {T_COLS} FROM msg_transfers WHERE status IN ({marks}) ORDER BY created_at DESC, rowid DESC");
    let mut q = sqlx::query_as::<_, TransferRow>(sqlx::AssertSqlSafe(sql));
    for s in statuses {
        q = q.bind(*s);
    }
    q.fetch_all(store.pool()).await.map_err(storage)
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

/// Set `status` (and clear the failure) only while the row is in one of
/// `from`, in one step: two that race for a transfer never both win.
/// Whether it was set.
pub async fn claim(store: &Store, id: &str, status: &str, from: &[&str]) -> Result<bool> {
    claim_where(store, id, status, from, None, None).await
}

/// `claim`, only while the row still names `message_id` (a placeholder
/// whose message may have gone out meanwhile).
pub async fn claim_for_message(store: &Store, id: &str, status: &str, from: &[&str], message_id: &str) -> Result<bool> {
    claim_where(store, id, status, from, Some(message_id), None).await
}

/// `claim`, and the row names `message_id` from then on, in the same
/// step: both are written or neither is.
pub async fn claim_naming(store: &Store, id: &str, status: &str, from: &[&str], message_id: &str) -> Result<bool> {
    claim_where(store, id, status, from, None, Some(message_id)).await
}

/// `claim`, while the row names `message_id` when one is given, and
/// naming `names` from then on when one is given.
async fn claim_where(
    store: &Store,
    id: &str,
    status: &str,
    from: &[&str],
    message_id: Option<&str>,
    names: Option<&str>,
) -> Result<bool> {
    let marks = vec!["?"; from.len()].join(", ");
    let message = if message_id.is_some() { " AND message_id = ?" } else { "" };
    let sql = format!(
        "UPDATE msg_transfers SET status = ?, failure_reason = NULL, message_id = COALESCE(?, message_id), updated_at = ?
         WHERE id = ? AND status IN ({marks}){message}"
    );
    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(status).bind(names).bind(crate::now()).bind(id);
    for s in from {
        q = q.bind(*s);
    }
    if let Some(m) = message_id {
        q = q.bind(m);
    }
    Ok(q.execute(store.pool()).await.map_err(storage)?.rows_affected() == 1)
}

/// Replace the state of a transfer, its progress untouched.
pub async fn set_state(store: &Store, id: &str, state_json: &str) -> Result<()> {
    sqlx::query("UPDATE msg_transfers SET state_json = ?, updated_at = ? WHERE id = ?")
        .bind(state_json)
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

/// The file of an upload is another one (a photo made smaller): its path,
/// name, type and size. Only while the upload is queued and nothing of it
/// was sent yet. Whether it was set.
pub async fn set_file(store: &Store, id: &str, local_path: &str, file_name: &str, mime: &str, size: i64) -> Result<bool> {
    let res = sqlx::query(
        "UPDATE msg_transfers SET local_path = ?, file_name = ?, mime = ?, size = ?, updated_at = ?
         WHERE id = ? AND direction = 'up' AND status = 'queued' AND done_bytes = 0 AND state_json = '{}'",
    )
    .bind(local_path)
    .bind(file_name)
    .bind(mime)
    .bind(size)
    .bind(crate::now())
    .bind(id)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(res.rows_affected() == 1)
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

/// After a restart nothing is running: what was in flight or waiting for
/// its next attempt becomes paused (the user or the scheduler resumes it;
/// nothing is lost).
pub async fn pause_interrupted(store: &Store) -> Result<u64> {
    let res = sqlx::query(
        "UPDATE msg_transfers SET status = 'paused', failure_reason = 'err.interrupted', updated_at = ?
         WHERE status IN ('queued', 'running', 'waiting_retry')",
    )
    .bind(crate::now())
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(res.rows_affected())
}

/// `pause_interrupted` for transfer `id` alone, whose run is known to be
/// gone. Whether it was under way.
pub async fn pause_interrupted_one(store: &Store, id: &str) -> Result<bool> {
    let res = sqlx::query(
        "UPDATE msg_transfers SET status = 'paused', failure_reason = 'err.interrupted', updated_at = ?
         WHERE id = ? AND status IN ('queued', 'running', 'waiting_retry')",
    )
    .bind(crate::now())
    .bind(id)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(res.rows_affected() == 1)
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
        insert_transfer(&s, &t("3", DIR_DOWN, Some("w"))).await.unwrap();
        set_status(&s, "1", ST_RUNNING, None).await.unwrap();
        set_status(&s, "3", ST_WAITING_RETRY, Some("err.network")).await.unwrap();
        set_progress(&s, "1", 5, Some(r#"{"k":1}"#)).await.unwrap();
        set_progress(&s, "1", 7, None).await.unwrap();
        let r = transfer(&s, "1").await.unwrap().unwrap();
        assert_eq!((r.done_bytes, r.state_json.as_str()), (7, r#"{"k":1}"#));
        assert_eq!(transfer_for_message(&s, "m", DIR_DOWN).await.unwrap().unwrap().id, "2");
        assert!(transfer_for_message(&s, "m", DIR_UP).await.unwrap().is_none());
        set_result(&s, "1", None, Some("sha"), Some("msg")).await.unwrap();
        assert_eq!(transfer(&s, "1").await.unwrap().unwrap().message_id.as_deref(), Some("msg"));

        set_status(&s, "2", ST_WAITING_RETRY, Some("err.network")).await.unwrap();
        assert!(pause_interrupted_one(&s, "2").await.unwrap());
        let r = transfer(&s, "2").await.unwrap().unwrap();
        assert_eq!((r.status.as_str(), r.failure_reason.as_deref()), (ST_PAUSED, Some("err.interrupted")));
        assert!(!pause_interrupted_one(&s, "2").await.unwrap(), "only one under way");
        set_status(&s, "2", ST_QUEUED, None).await.unwrap();
        assert_eq!(pause_interrupted(&s).await.unwrap(), 3, "waiting for the next attempt is in flight too");
        assert_eq!(transfers_with_status(&s, &[ST_PAUSED]).await.unwrap().len(), 3);
        let newest: Vec<String> = transfers_newest_first(&s, &[ST_PAUSED, ST_DONE]).await.unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(newest, ["3", "2", "1"], "within one second too");
        assert!(transfers_newest_first(&s, &[]).await.unwrap().is_empty());

        // A claim wins once, and only from the states it names.
        assert!(claim(&s, "1", ST_QUEUED, &[ST_PAUSED, ST_FAILED]).await.unwrap());
        assert!(!claim(&s, "1", ST_QUEUED, &[ST_PAUSED, ST_FAILED]).await.unwrap());
        let r = transfer(&s, "1").await.unwrap().unwrap();
        assert_eq!((r.status.as_str(), r.failure_reason), (ST_QUEUED, None), "the failure goes with the claim");
        assert!(!claim(&s, "nope", ST_QUEUED, &[ST_PAUSED]).await.unwrap());
        // Only while the row still names the message (here "msg").
        assert!(!claim_for_message(&s, "1", ST_CANCELLED, &[ST_QUEUED], "local:x").await.unwrap());
        assert_eq!(transfer(&s, "1").await.unwrap().unwrap().status, ST_QUEUED);
        assert!(claim_for_message(&s, "1", ST_PAUSED, &[ST_QUEUED], "msg").await.unwrap());
        assert!(claim(&s, "1", ST_QUEUED, &[ST_PAUSED]).await.unwrap());
        // The message is named with the claim, never without it.
        assert!(!claim_naming(&s, "1", ST_DONE, &[ST_RUNNING], "out").await.unwrap());
        assert_eq!(transfer(&s, "1").await.unwrap().unwrap().message_id.as_deref(), Some("msg"));
        assert!(claim_naming(&s, "1", ST_QUEUED, &[ST_QUEUED], "out").await.unwrap());
        assert_eq!(transfer(&s, "1").await.unwrap().unwrap().message_id.as_deref(), Some("out"));
        set_state(&s, "1", r#"{"k":2}"#).await.unwrap();
        let r = transfer(&s, "1").await.unwrap().unwrap();
        assert_eq!((r.done_bytes, r.state_json.as_str()), (7, r#"{"k":2}"#));
        set_attempts(&s, "2", -1).await.unwrap();
        assert_eq!(transfer(&s, "2").await.unwrap().unwrap().attempts, -1);
    }

    /// The file of an upload changes only while nothing of it was sent.
    #[tokio::test]
    async fn the_file_of_an_upload_changes_before_it_is_sent() {
        let s = Store::open_in_memory().await.unwrap();
        insert_transfer(&s, &t("1", DIR_UP, Some("local:a"))).await.unwrap();
        assert!(set_file(&s, "1", "/tmp/x.jpg", "x.jpg", "image/jpeg", 4).await.unwrap());
        let r = transfer(&s, "1").await.unwrap().unwrap();
        assert_eq!(
            (r.local_path.as_deref(), r.file_name.as_str(), r.mime.as_str(), r.size),
            (Some("/tmp/x.jpg"), "x.jpg", "image/jpeg", 4)
        );

        set_progress(&s, "1", 0, Some(r#"{"key":"k"}"#)).await.unwrap();
        assert!(!set_file(&s, "1", "/tmp/y.jpg", "y.jpg", "image/jpeg", 3).await.unwrap(), "its key is chosen");
        insert_transfer(&s, &t("2", DIR_UP, Some("local:b"))).await.unwrap();
        set_status(&s, "2", ST_CANCELLED, None).await.unwrap();
        assert!(!set_file(&s, "2", "/tmp/y.jpg", "y.jpg", "image/jpeg", 3).await.unwrap(), "no longer queued");
        insert_transfer(&s, &t("3", DIR_DOWN, Some("m"))).await.unwrap();
        assert!(!set_file(&s, "3", "/tmp/y.jpg", "y.jpg", "image/jpeg", 3).await.unwrap(), "a download keeps its file");
        assert_eq!(transfer(&s, "1").await.unwrap().unwrap().file_name, "x.jpg");
    }
}
