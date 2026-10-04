// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use crate::browser::BrowserState;
use crate::error::{AppError, CmdResult};
use crate::models::{AntidetectCookie, CookieImportResult, ExportCookie, Profile};
use veydan_core::Core;

#[tauri::command]
pub async fn profile_import_cookies(
    id: String,
    cookies_json: String,
    core: tauri::State<'_, Core>,
    browser: tauri::State<'_, BrowserState>,
) -> CmdResult<CookieImportResult> {
    if browser.running.is_running(&id).await {
        return Err(AppError::other("Stop the browser before importing cookies"));
    }

    let profile = sqlx::query_as::<_, Profile>("SELECT * FROM profiles WHERE id = ?")
        .bind(&id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Profile not found"))?;

    let profile_path = std::path::PathBuf::from(&profile.profile_path);

    tokio::task::spawn_blocking(move || import_cookies_into_sqlite(&profile_path, &cookies_json))
        .await
        .map_err(|e| AppError::other(format!("Task error: {e}")))?
}

#[tauri::command]
pub async fn profile_export_cookies(id: String, core: tauri::State<'_, Core>) -> CmdResult<String> {
    let profile = sqlx::query_as::<_, Profile>("SELECT * FROM profiles WHERE id = ?")
        .bind(&id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Profile not found"))?;

    let profile_path = std::path::PathBuf::from(&profile.profile_path);

    tokio::task::spawn_blocking(move || export_cookies_from_sqlite(&profile_path))
        .await
        .map_err(|e| AppError::other(format!("Task error: {e}")))?
}

/// Export cookies directly to a file path chosen via dialog on the frontend.
#[tauri::command]
pub async fn profile_export_cookies_to_file(
    id: String,
    output_path: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<()> {
    let json = profile_export_cookies(id, core).await?;
    std::fs::write(&output_path, json).map_err(AppError::io)
}

fn parse_antidetect_cookies(json: &str) -> Result<Vec<AntidetectCookie>, AppError> {
    serde_json::from_str::<Vec<AntidetectCookie>>(json)
        .or_else(|_| {
            #[derive(serde::Deserialize)]
            struct Wrapped {
                cookies: Vec<AntidetectCookie>,
            }
            serde_json::from_str::<Wrapped>(json).map(|w| w.cookies)
        })
        .map_err(|e| AppError::other(format!("Invalid cookies JSON: {e}")))
}

fn export_cookies_from_sqlite(profile_path: &std::path::Path) -> CmdResult<String> {
    let db_path = profile_path.join("firefox-profile").join("cookies.sqlite");

    if !db_path.exists() {
        return Ok("[]".to_string());
    }

    let conn = rusqlite::Connection::open(&db_path).map_err(AppError::io)?;

    // Detect available columns to build a stable SELECT
    let mut col_stmt = conn
        .prepare("PRAGMA table_info(moz_cookies)")
        .map_err(AppError::other)?;
    let cols: std::collections::HashSet<String> = col_stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(AppError::other)?
        .filter_map(|r| r.ok())
        .collect();

    // Substitute literal fallbacks for optional columns so result always has 9 fields
    let same_site_expr = if cols.contains("sameSite") {
        "sameSite"
    } else {
        "0"
    };
    let is_session_expr = if cols.contains("isSession") {
        "isSession"
    } else {
        "0"
    };

    let query = format!(
        "SELECT host, name, value, path, expiry, isSecure, isHttpOnly, \
         {same_site_expr} AS sameSite, {is_session_expr} AS isSession \
         FROM moz_cookies ORDER BY host, name LIMIT 5000"
    );

    let mut stmt = conn.prepare(&query).map_err(AppError::other)?;

    let cookies: Vec<ExportCookie> = stmt
        .query_map([], |row| {
            let host: String = row.get(0)?;
            let name: String = row.get(1)?;
            let value: String = row.get(2)?;
            let path: String = row.get(3)?;
            let expiry: i64 = row.get(4).unwrap_or(0);
            let is_secure: i32 = row.get(5).unwrap_or(0);
            let is_http_only: i32 = row.get(6).unwrap_or(0);
            let same_site_int: i32 = row.get(7).unwrap_or(0);
            let is_session: i32 = row.get(8).unwrap_or(0);

            let session = is_session != 0 || expiry == 0;
            let expiration_date = if session { None } else { Some(expiry as f64) };
            let host_only = !host.starts_with('.');
            let same_site = match same_site_int {
                1 => "lax",
                2 => "strict",
                _ => "no_restriction",
            }
            .to_string();

            Ok(ExportCookie {
                name,
                value,
                domain: host,
                path,
                expiration_date,
                host_only,
                session,
                http_only: is_http_only != 0,
                secure: is_secure != 0,
                same_site,
            })
        })
        .map_err(AppError::other)?
        .filter_map(|r| r.ok())
        .collect();

    serde_json::to_string_pretty(&cookies).map_err(AppError::other)
}

fn import_cookies_into_sqlite(
    profile_path: &std::path::Path,
    cookies_json: &str,
) -> CmdResult<CookieImportResult> {
    let cookies = parse_antidetect_cookies(cookies_json)?;

    let firefox_dir = profile_path.join("firefox-profile");
    std::fs::create_dir_all(&firefox_dir).map_err(AppError::io)?;

    let db_path = firefox_dir.join("cookies.sqlite");
    let conn = rusqlite::Connection::open(&db_path).map_err(AppError::io)?;

    // Detect or create moz_cookies table
    let table_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='moz_cookies'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;

    if !table_exists {
        conn.execute_batch(
            "CREATE TABLE moz_cookies (
                id                      INTEGER PRIMARY KEY,
                originAttributes        TEXT NOT NULL DEFAULT '',
                name                    TEXT,
                value                   TEXT,
                host                    TEXT,
                path                    TEXT,
                expiry                  INTEGER,
                lastAccessed            INTEGER,
                creationTime            INTEGER,
                isSecure                INTEGER,
                isHttpOnly              INTEGER,
                appId                   INTEGER DEFAULT 0,
                inBrowserElement        INTEGER DEFAULT 0,
                sameSite                INTEGER DEFAULT 0,
                rawSameSite             INTEGER DEFAULT 0,
                schemeMap               INTEGER DEFAULT 0,
                isSession               INTEGER DEFAULT 0,
                isPartitionedAttributeSet INTEGER DEFAULT 0,
                baseDomain              TEXT
            );
            CREATE INDEX IF NOT EXISTS moz_basedomain
                ON moz_cookies (baseDomain, originAttributes);",
        )
        .map_err(AppError::other)?;
    }

    // Detect existing columns so we insert only what the table has
    let mut stmt = conn
        .prepare("PRAGMA table_info(moz_cookies)")
        .map_err(AppError::other)?;
    let existing_cols: std::collections::HashSet<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(AppError::other)?
        .filter_map(|r| r.ok())
        .collect();

    let now_micros = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64;

    let mut count = 0usize;
    let mut domain_set: std::collections::HashSet<String> = std::collections::HashSet::new();

    for cookie in &cookies {
        let is_session = cookie.session.unwrap_or(false) || cookie.expiration_date.is_none();
        let expiry: i64 = if is_session {
            0
        } else {
            cookie.expiration_date.unwrap_or(0.0) as i64
        };
        let is_secure = cookie.secure.unwrap_or(false) as i64;
        let is_http_only = cookie.http_only.unwrap_or(false) as i64;
        let same_site: i64 = match cookie.same_site.as_deref() {
            Some("lax") => 1,
            Some("strict") => 2,
            _ => 0,
        };
        // baseDomain: strip leading dot (simplified, not real eTLD+1)
        let base_domain = cookie.domain.trim_start_matches('.');

        // Delete existing cookie with the same identity before inserting
        conn.execute(
            "DELETE FROM moz_cookies WHERE host=? AND name=? AND path=? AND originAttributes=''",
            rusqlite::params![&cookie.domain, &cookie.name, &cookie.path],
        )
        .map_err(AppError::other)?;

        // Build INSERT only with columns that exist in this DB
        let mut cols = vec![
            "originAttributes",
            "name",
            "value",
            "host",
            "path",
            "expiry",
            "lastAccessed",
            "creationTime",
            "isSecure",
            "isHttpOnly",
            "sameSite",
            "rawSameSite",
        ];
        let mut placeholders = vec!["''", "?", "?", "?", "?", "?", "?", "?", "?", "?", "?", "?"];

        if existing_cols.contains("isSession") {
            cols.push("isSession");
            placeholders.push("?");
        }
        if existing_cols.contains("baseDomain") {
            cols.push("baseDomain");
            placeholders.push("?");
        }

        let sql = format!(
            "INSERT INTO moz_cookies ({}) VALUES ({})",
            cols.join(", "),
            placeholders.join(", ")
        );

        let result = if existing_cols.contains("isSession") && existing_cols.contains("baseDomain")
        {
            conn.execute(
                &sql,
                rusqlite::params![
                    &cookie.name,
                    &cookie.value,
                    &cookie.domain,
                    &cookie.path,
                    expiry,
                    now_micros,
                    now_micros,
                    is_secure,
                    is_http_only,
                    same_site,
                    same_site,
                    is_session as i64,
                    base_domain
                ],
            )
        } else if existing_cols.contains("isSession") {
            conn.execute(
                &sql,
                rusqlite::params![
                    &cookie.name,
                    &cookie.value,
                    &cookie.domain,
                    &cookie.path,
                    expiry,
                    now_micros,
                    now_micros,
                    is_secure,
                    is_http_only,
                    same_site,
                    same_site,
                    is_session as i64
                ],
            )
        } else if existing_cols.contains("baseDomain") {
            conn.execute(
                &sql,
                rusqlite::params![
                    &cookie.name,
                    &cookie.value,
                    &cookie.domain,
                    &cookie.path,
                    expiry,
                    now_micros,
                    now_micros,
                    is_secure,
                    is_http_only,
                    same_site,
                    same_site,
                    base_domain
                ],
            )
        } else {
            conn.execute(
                &sql,
                rusqlite::params![
                    &cookie.name,
                    &cookie.value,
                    &cookie.domain,
                    &cookie.path,
                    expiry,
                    now_micros,
                    now_micros,
                    is_secure,
                    is_http_only,
                    same_site,
                    same_site
                ],
            )
        };

        if result.map_err(AppError::other)? > 0 {
            count += 1;
            domain_set.insert(base_domain.to_string());
        }
    }

    let mut domains: Vec<String> = domain_set.into_iter().collect();
    domains.sort();
    domains.truncate(20);

    Ok(CookieImportResult { count, domains })
}
