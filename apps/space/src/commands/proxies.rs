// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use crate::error::{AppError, CmdResult};
use crate::models::{
    BulkImportResult, BulkImportRowResult, BulkProxyItem, CreateProxyRequest, Proxy,
    ProxyCheckResult,
};
use crate::modules::directory;
use crate::proxy::check;
use chrono::Utc;
use serde::Serialize;
use std::collections::HashSet;
use uuid::Uuid;
use veydan_core::{Core, Directory};

#[tauri::command]
pub async fn proxies_list(core: tauri::State<'_, Core>) -> CmdResult<Vec<Proxy>> {
    sqlx::query_as::<_, Proxy>("SELECT * FROM proxies ORDER BY created_at DESC")
        .fetch_all(&core.db)
        .await
        .map_err(AppError::db)
}

#[tauri::command]
pub async fn proxy_get(id: String, core: tauri::State<'_, Core>) -> CmdResult<Option<Proxy>> {
    sqlx::query_as::<_, Proxy>("SELECT * FROM proxies WHERE id = ?")
        .bind(&id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)
}

#[derive(sqlx::FromRow)]
struct ProxyCredentials {
    proxy_type: String,
    host: String,
    port: i64,
    username: Option<String>,
    password: Option<String>,
}

/// Full connection string `type://user:pass@host:port` for the clipboard.
/// The only path that hands the password to the UI, on explicit user action.
#[tauri::command]
pub async fn proxy_export_url(id: String, core: tauri::State<'_, Core>) -> CmdResult<String> {
    let p = sqlx::query_as::<_, ProxyCredentials>(
        "SELECT proxy_type, host, port, username, password FROM proxies WHERE id = ?",
    )
    .bind(&id)
    .fetch_optional(&core.db)
    .await
    .map_err(AppError::db)?
    .ok_or_else(|| AppError::not_found(format!("Proxy {id}")))?;
    let auth = match (p.username.filter(|u| !u.is_empty()), p.password.filter(|s| !s.is_empty())) {
        (Some(u), Some(pw)) => format!("{u}:{pw}@"),
        (Some(u), None) => format!("{u}@"),
        _ => String::new(),
    };
    Ok(format!("{}://{auth}{}:{}", p.proxy_type, p.host, p.port))
}

#[tauri::command]
pub async fn proxy_create(
    mut req: CreateProxyRequest,
    core: tauri::State<'_, Core>,
) -> CmdResult<Proxy> {
    crate::proxy::tor::normalize(&mut req).map_err(AppError::proxy)?;
    if req.proxy_type == crate::proxy::tor::TYPE {
        // A new row has no stored secret to keep: no value, not an empty one.
        req.password = None;
        req.private_key = None;
    }
    let id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let tags_json =
        serde_json::to_string(&req.tags.unwrap_or_default()).map_err(AppError::other)?;

    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    sqlx::query(
        "INSERT INTO proxies
        (id, name, proxy_type, host, port, username, password, country, city, status, tags, private_key, created_at)
        VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&id)
    .bind(&req.name)
    .bind(&req.proxy_type)
    .bind(&req.host)
    .bind(req.port)
    .bind(&req.username)
    .bind(&req.password)
    .bind(&req.country)
    .bind(&req.city)
    .bind("unknown")
    .bind(&tags_json)
    .bind(&req.private_key)
    .bind(now.to_rfc3339())
    .execute(&mut *tx)
    .await
    .map_err(AppError::db)?;
    directory::relabel(&core.directory, &mut tx, &directory::PROXY, &id).await?;
    tx.commit().await.map_err(AppError::db)?;

    proxy_get(id, core)
        .await
        .and_then(|p| p.ok_or_else(|| AppError::not_found("Proxy not found after create")))
}

const BULK_ALLOWED_TYPES: &[&str] = &["http", "https", "socks5"];

fn validate_bulk_item(item: &BulkProxyItem) -> Result<(), String> {
    if !BULK_ALLOWED_TYPES.contains(&item.proxy_type.as_str()) {
        return Err(format!("Unsupported proxy type: {}", item.proxy_type));
    }
    let host = item.host.trim();
    if host.is_empty() || host.contains(char::is_whitespace) {
        return Err("Invalid host".into());
    }
    if !(1..=65535).contains(&item.port) {
        return Err(format!("Invalid port: {}", item.port));
    }
    Ok(())
}

fn dedup_key(
    proxy_type: &str,
    host: &str,
    port: i64,
    username: Option<&str>,
) -> (String, String, i64, String) {
    (
        proxy_type.to_string(),
        host.trim().to_lowercase(),
        port,
        username.unwrap_or("").to_string(),
    )
}

#[tauri::command]
pub async fn proxies_bulk_create(
    items: Vec<BulkProxyItem>,
    core: tauri::State<'_, Core>,
) -> CmdResult<BulkImportResult> {
    bulk_create_impl(&core.db, &core.directory, items).await
}

async fn bulk_create_impl(
    db: &sqlx::Pool<sqlx::Sqlite>,
    labels: &Directory,
    items: Vec<BulkProxyItem>,
) -> CmdResult<BulkImportResult> {
    let existing: Vec<(String, String, i64, Option<String>)> =
        sqlx::query_as("SELECT proxy_type, LOWER(host), port, username FROM proxies")
            .fetch_all(db)
            .await
            .map_err(AppError::db)?;

    let mut seen: HashSet<(String, String, i64, String)> = existing
        .into_iter()
        .map(|(t, h, p, u)| dedup_key(&t, &h, p, u.as_deref()))
        .collect();

    let mut tx = db.begin().await.map_err(AppError::db)?;
    let mut rows: Vec<BulkImportRowResult> = Vec::with_capacity(items.len());
    let mut imported_ids: Vec<String> = Vec::new();
    let now = Utc::now().to_rfc3339();

    for item in &items {
        if let Err(msg) = validate_bulk_item(item) {
            rows.push(BulkImportRowResult {
                line_number: item.line_number,
                status: "error".into(),
                message: Some(msg),
                id: None,
            });
            continue;
        }

        let key = dedup_key(
            &item.proxy_type,
            &item.host,
            item.port,
            item.username.as_deref(),
        );
        if seen.contains(&key) {
            rows.push(BulkImportRowResult {
                line_number: item.line_number,
                status: "duplicate".into(),
                message: None,
                id: None,
            });
            continue;
        }
        seen.insert(key);

        let id = Uuid::new_v4().to_string();
        let host = item.host.trim();
        sqlx::query(
            "INSERT INTO proxies
            (id, name, proxy_type, host, port, username, password, country, city, status, tags, private_key, created_at)
            VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&id)
        .bind(format!("{}:{}", host, item.port))
        .bind(&item.proxy_type)
        .bind(host)
        .bind(item.port)
        .bind(&item.username)
        .bind(&item.password)
        .bind(None::<String>)
        .bind(None::<String>)
        .bind("unknown")
        .bind("[]")
        .bind(None::<String>)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;
        directory::relabel(labels, &mut tx, &directory::PROXY, &id).await?;

        rows.push(BulkImportRowResult {
            line_number: item.line_number,
            status: "imported".into(),
            message: None,
            id: Some(id.clone()),
        });
        imported_ids.push(id);
    }

    tx.commit().await.map_err(AppError::db)?;

    let mut imported: Vec<Proxy> = Vec::with_capacity(imported_ids.len());
    for id in &imported_ids {
        let proxy = sqlx::query_as::<_, Proxy>("SELECT * FROM proxies WHERE id = ?")
            .bind(id)
            .fetch_optional(db)
            .await
            .map_err(AppError::db)?
            .ok_or_else(|| AppError::not_found("Proxy not found after import"))?;
        imported.push(proxy);
    }

    Ok(BulkImportResult { rows, imported })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(proxy_type: &str, host: &str, port: i64) -> BulkProxyItem {
        BulkProxyItem {
            line_number: 1,
            proxy_type: proxy_type.into(),
            host: host.into(),
            port,
            username: None,
            password: None,
        }
    }

    #[test]
    fn validate_accepts_supported_types() {
        for t in ["http", "https", "socks5"] {
            assert!(validate_bulk_item(&item(t, "1.2.3.4", 8080)).is_ok());
        }
    }

    #[test]
    fn validate_rejects_ssh_and_unknown_types() {
        assert!(validate_bulk_item(&item("ssh", "1.2.3.4", 22)).is_err());
        assert!(validate_bulk_item(&item("socks4", "1.2.3.4", 1080)).is_err());
    }

    #[test]
    fn validate_rejects_bad_port() {
        assert!(validate_bulk_item(&item("http", "1.2.3.4", 0)).is_err());
        assert!(validate_bulk_item(&item("http", "1.2.3.4", 65536)).is_err());
        assert!(validate_bulk_item(&item("http", "1.2.3.4", 65535)).is_ok());
    }

    #[test]
    fn validate_rejects_bad_host() {
        assert!(validate_bulk_item(&item("http", "", 8080)).is_err());
        assert!(validate_bulk_item(&item("http", "ho st", 8080)).is_err());
    }

    #[test]
    fn dedup_key_is_case_insensitive_on_host() {
        assert_eq!(
            dedup_key("http", "Example.COM", 8080, Some("u")),
            dedup_key("http", "example.com", 8080, Some("u"))
        );
        assert_ne!(
            dedup_key("http", "example.com", 8080, None),
            dedup_key("socks5", "example.com", 8080, None)
        );
    }

    async fn test_pool() -> sqlx::Pool<sqlx::Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE proxies (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                proxy_type TEXT NOT NULL DEFAULT 'socks5',
                host TEXT NOT NULL,
                port INTEGER NOT NULL,
                username TEXT,
                password TEXT,
                country TEXT,
                city TEXT,
                status TEXT NOT NULL DEFAULT 'unknown',
                last_ip TEXT,
                last_check_at TEXT,
                private_key TEXT,
                server_fingerprint TEXT,
                tags TEXT NOT NULL DEFAULT '[]',
                created_at TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    fn full_item(line: i64, host: &str, port: i64, user: Option<&str>) -> BulkProxyItem {
        BulkProxyItem {
            line_number: line,
            proxy_type: "http".into(),
            host: host.into(),
            port,
            username: user.map(String::from),
            password: user.map(|_| "pw".to_string()),
        }
    }

    #[tokio::test]
    async fn bulk_create_imports_dedups_and_reports_errors() {
        let pool = test_pool().await;

        let result = bulk_create_impl(
            &pool,
            &Directory::default(),
            vec![
                full_item(1, "1.2.3.4", 8080, None),
                full_item(2, "1.2.3.4", 8080, None), // dup within batch
                full_item(3, "1.2.3.4", 8080, Some("user")), // different username -> new
                full_item(4, "bad host", 8080, None), // error
                full_item(5, "5.6.7.8", 0, None),    // error: port
            ],
        )
        .await
        .unwrap();

        let statuses: Vec<&str> = result.rows.iter().map(|r| r.status.as_str()).collect();
        assert_eq!(
            statuses,
            ["imported", "duplicate", "imported", "error", "error"]
        );
        assert_eq!(result.imported.len(), 2);
        assert_eq!(result.imported[0].name, "1.2.3.4:8080");
        assert_eq!(result.imported[0].status, "unknown");

        // Re-import: everything already in DB → duplicates (case-insensitive host)
        let again = bulk_create_impl(
            &pool,
            &Directory::default(),
            vec![
                full_item(1, "1.2.3.4", 8080, None),
                full_item(2, "1.2.3.4", 8080, Some("user")),
            ],
        )
        .await
        .unwrap();
        assert!(again.rows.iter().all(|r| r.status == "duplicate"));
        assert!(again.imported.is_empty());

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM proxies")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 2);
    }

    async fn insert_proxy(pool: &sqlx::SqlitePool, id: &str, proxy_type: &str) {
        sqlx::query(
            "INSERT INTO proxies (id, name, proxy_type, host, port, created_at)
             VALUES (?, ?, ?, '1.2.3.4', 8080, '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(format!("proxy-{id}"))
        .bind(proxy_type)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn resolve_required_returns_none_only_when_no_proxy_configured() {
        let pool = test_pool().await;
        assert!(resolve_required(&pool, None).await.unwrap().is_none());
        // The UI sends "" for "no value" on nullable fields; treat it as unset.
        assert!(resolve_required(&pool, Some("")).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn resolve_required_resolves_every_supported_type() {
        let pool = test_pool().await;
        for t in SUPPORTED_PROXY_TYPES {
            insert_proxy(&pool, t, t).await;
            let proxy = resolve_required(&pool, Some(t)).await.unwrap();
            assert_eq!(proxy.unwrap().proxy_type, t);
        }
    }

    /// The regression this whole resolver exists for: a deleted proxy must abort
    /// the connect, never degrade it to a direct one that leaks the real IP.
    #[tokio::test]
    async fn resolve_required_errors_on_dangling_id() {
        let pool = test_pool().await;
        let err = resolve_required(&pool, Some("gone"))
            .await
            .expect_err("a dangling proxy_id must not resolve to a direct connection");
        assert!(matches!(err, AppError::Proxy(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn resolve_required_errors_on_unroutable_type() {
        let pool = test_pool().await;
        insert_proxy(&pool, "p1", "socks4").await;
        let err = resolve_required(&pool, Some("p1"))
            .await
            .expect_err("an unsupported proxy type must not fall back to direct");
        assert!(matches!(err, AppError::Proxy(_)), "got {err:?}");
    }
}

/// Update a proxy. Field contract (matches ProxyPanel.svelte, which pre-fills
/// the edit form from the stored proxy and submits the complete state):
///
/// * `name`, `proxy_type`, `host`, `port`, `tags` are required and replaced.
/// * `username`, `country`, `city` are bound raw: null clears the column
///   (the UI sends the full state each save, so null means "no value").
/// * Credentials (`password`, `private_key`) use COALESCE — null means "keep
///   the stored secret", so a caller omitting them can never wipe credentials
///   by accident. Clearing them intentionally still works: an empty string
///   from the UI is stored as NULL.
#[tauri::command]
pub async fn proxy_update(
    id: String,
    mut req: CreateProxyRequest,
    core: tauri::State<'_, Core>,
) -> CmdResult<Proxy> {
    crate::proxy::tor::normalize(&mut req).map_err(AppError::proxy)?;
    let tags_json =
        serde_json::to_string(&req.tags.unwrap_or_default()).map_err(AppError::other)?;

    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    sqlx::query(
        "UPDATE proxies SET
            name = ?, proxy_type = ?, host = ?, port = ?,
            username = ?, password = NULLIF(COALESCE(?, password), ''),
            country = ?, city = ?,
            private_key = NULLIF(COALESCE(?, private_key), ''), tags = ?
        WHERE id = ?",
    )
    .bind(&req.name)
    .bind(&req.proxy_type)
    .bind(&req.host)
    .bind(req.port)
    .bind(&req.username)
    .bind(&req.password)
    .bind(&req.country)
    .bind(&req.city)
    .bind(&req.private_key)
    .bind(&tags_json)
    .bind(&id)
    .execute(&mut *tx)
    .await
    .map_err(AppError::db)?;
    directory::relabel(&core.directory, &mut tx, &directory::PROXY, &id).await?;
    tx.commit().await.map_err(AppError::db)?;

    proxy_get(id, core)
        .await
        .and_then(|p| p.ok_or_else(|| AppError::not_found("Proxy not found after update")))
}

/// TOFU pin for an SSH proxy after a successful connect made outside `proxy_check`
/// (profile launch, jump host). Only fills an empty slot; never overwrites.
pub async fn pin_ssh_fingerprint(
    db: &sqlx::SqlitePool,
    proxy: &Proxy,
    result: &crate::proxy::ssh::SshConnectResult,
) {
    if !result.is_new || proxy.server_fingerprint.is_some() {
        return;
    }
    if let Err(e) = sqlx::query(
        "UPDATE proxies SET server_fingerprint = ? WHERE id = ? AND server_fingerprint IS NULL",
    )
    .bind(&result.fingerprint)
    .bind(&proxy.id)
    .execute(db)
    .await
    {
        eprintln!("[proxy] failed to pin host key for {}: {e}", proxy.id);
    }
}

/// Proxy types the app can actually route traffic through.
/// Every entry must be handled by *both* `commands::ssh::establish_transport`
/// (terminal + SFTP) and `browser::profile_launcher::setup_proxy` (browser).
/// Adding a type here without wiring both paths reintroduces the silent
/// direct-connection fallback this list exists to prevent.
pub const SUPPORTED_PROXY_TYPES: [&str; 5] = ["http", "https", "socks5", "ssh", "tor"];

pub fn is_supported_type(proxy_type: &str) -> bool {
    SUPPORTED_PROXY_TYPES.contains(&proxy_type)
}

/// Resolve a stored `proxy_id` into its row, failing closed.
///
/// A profile or SSH connection that names a proxy must never quietly fall back
/// to a direct connection: that exposes the real IP against the user's explicit
/// configuration. So a dangling id (the proxy row was deleted out from under
/// this one — `profiles.proxy_id` references nothing, see `db::schemas`) or a `proxy_type`
/// no transport implements is an error, never `Ok(None)`.
///
/// `Ok(None)` means one thing only: no proxy is configured. That is the single
/// path to a direct connection.
pub async fn resolve_required(
    db: &sqlx::SqlitePool,
    proxy_id: Option<&str>,
) -> Result<Option<Proxy>, AppError> {
    let Some(id) = proxy_id.filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let proxy = sqlx::query_as::<_, Proxy>("SELECT * FROM proxies WHERE id = ?")
        .bind(id)
        .fetch_optional(db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| {
            AppError::proxy(format!(
                "The configured proxy ({id}) no longer exists — refusing to connect \
                 directly. Assign another proxy, or clear the proxy setting to \
                 connect without one on purpose."
            ))
        })?;
    if !is_supported_type(&proxy.proxy_type) {
        return Err(AppError::proxy(format!(
            "Proxy '{}' has type '{}', which this build cannot route through — \
             refusing to connect directly. Supported types: {}.",
            proxy.name,
            proxy.proxy_type,
            SUPPORTED_PROXY_TYPES.join(", ")
        )));
    }
    Ok(Some(proxy))
}

/// What a proxy is still attached to. The delete confirmation shows this so the
/// user learns which rows stop connecting until they assign a new proxy.
#[derive(Debug, Serialize)]
pub struct ProxyUsage {
    pub profiles: Vec<String>,
    pub ssh_connections: Vec<String>,
}

#[tauri::command]
pub async fn proxy_usage(id: String, core: tauri::State<'_, Core>) -> CmdResult<ProxyUsage> {
    let profiles = sqlx::query_scalar::<_, String>(
        "SELECT name FROM profiles WHERE proxy_id = ? ORDER BY name",
    )
    .bind(&id)
    .fetch_all(&core.db)
    .await
    .map_err(AppError::db)?;
    let ssh_connections = sqlx::query_scalar::<_, String>(
        "SELECT name FROM ssh_connections WHERE proxy_id = ? ORDER BY name",
    )
    .bind(&id)
    .fetch_all(&core.db)
    .await
    .map_err(AppError::db)?;
    Ok(ProxyUsage {
        profiles,
        ssh_connections,
    })
}

#[tauri::command]
pub async fn proxy_delete(id: String, core: tauri::State<'_, Core>) -> CmdResult<()> {
    // Delete paths do not lean on the schema's `ON DELETE SET NULL` (see
    // `db::schemas`) — clear every reference explicitly. One transaction with the delete:
    // a partial failure would leave rows pointing at a proxy that is already
    // gone, and those then refuse to connect (`resolve_required`) until edited.
    let mut tx = core.db.begin().await.map_err(AppError::db)?;

    sqlx::query("UPDATE profiles SET proxy_id = NULL WHERE proxy_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;

    sqlx::query("UPDATE ssh_connections SET proxy_id = NULL, updated_at = ? WHERE proxy_id = ?")
        .bind(Utc::now().to_rfc3339())
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;

    sqlx::query("DELETE FROM proxies WHERE id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;
    directory::relabel(&core.directory, &mut tx, &directory::PROXY, &id).await?;

    tx.commit().await.map_err(AppError::db)?;
    Ok(())
}

#[tauri::command]
pub async fn proxy_check(
    id: String,
    core: tauri::State<'_, Core>,
    tor: tauri::State<'_, veydan_tor::TorManager>,
) -> CmdResult<ProxyCheckResult> {
    let proxy = sqlx::query_as::<_, Proxy>("SELECT * FROM proxies WHERE id = ?")
        .bind(&id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Proxy not found"))?;

    let result = check::check_proxy(&proxy, &tor)
        .await
        .map_err(AppError::proxy)?;

    // For SSH proxies with a new fingerprint — do NOT save status yet,
    // the frontend must prompt the user to confirm the fingerprint first.
    if proxy.proxy_type == "ssh" && result.ssh_fingerprint_is_new == Some(true) {
        return Ok(result);
    }

    let status = if result.ok { "active" } else { "error" };
    // The country of a Tor row is the exits the user asked for, and empty
    // means any: where this check happened to come out is not written there.
    let (country, city) = if crate::proxy::tor::is_tor(&proxy) {
        (None, None)
    } else {
        (result.country.clone(), result.city.clone())
    };
    sqlx::query(
        "UPDATE proxies SET
            status = ?,
            last_ip = ?,
            last_check_at = datetime('now'),
            country = CASE WHEN (country IS NULL OR country = '') THEN ? ELSE country END,
            city    = CASE WHEN (city IS NULL OR city = '')       THEN ? ELSE city    END
        WHERE id = ?",
    )
    .bind(status)
    .bind(&result.ip)
    .bind(&country)
    .bind(&city)
    .bind(&id)
    .execute(&core.db)
    .await
    .map_err(AppError::db)?;

    Ok(result)
}

/// Save a trusted SSH server fingerprint (TOFU: user confirmed the fingerprint in UI).
/// Also marks the proxy as active and stores IP/country/city from the check result.
#[tauri::command]
pub async fn proxy_trust_fingerprint(
    id: String,
    fingerprint: String,
    ip: String,
    country: Option<String>,
    city: Option<String>,
    core: tauri::State<'_, Core>,
) -> CmdResult<()> {
    sqlx::query(
        "UPDATE proxies SET
            server_fingerprint = ?,
            status = 'active',
            last_ip = ?,
            last_check_at = datetime('now'),
            country = CASE WHEN (country IS NULL OR country = '') THEN ? ELSE country END,
            city    = CASE WHEN (city IS NULL OR city = '')       THEN ? ELSE city    END
        WHERE id = ?",
    )
    .bind(&fingerprint)
    .bind(&ip)
    .bind(&country)
    .bind(&city)
    .bind(&id)
    .execute(&core.db)
    .await
    .map_err(AppError::db)?;
    Ok(())
}
