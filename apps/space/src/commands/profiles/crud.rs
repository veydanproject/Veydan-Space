// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use crate::browser::launch as browser_launch;
use crate::browser::BrowserState;
use crate::error::{AppError, CmdResult};
use crate::models::{CookieEntry, CreateProfileRequest, Profile, UpdateProfileRequest};
use crate::modules::directory;
use chrono::Utc;
use std::path::PathBuf;
use uuid::Uuid;
use veydan_core::Core;

#[tauri::command]
pub async fn profiles_list(core: tauri::State<'_, Core>) -> CmdResult<Vec<Profile>> {
    sqlx::query_as::<_, Profile>("SELECT * FROM profiles ORDER BY created_at DESC")
        .fetch_all(&core.db)
        .await
        .map_err(AppError::db)
}

#[tauri::command]
pub async fn profile_get(id: String, core: tauri::State<'_, Core>) -> CmdResult<Option<Profile>> {
    sqlx::query_as::<_, Profile>("SELECT * FROM profiles WHERE id = ?")
        .bind(&id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)
}

#[tauri::command]
pub async fn profile_create(
    req: CreateProfileRequest,
    core: tauri::State<'_, Core>,
) -> CmdResult<Profile> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let workspace_id = req.workspace_id.as_deref().unwrap_or("default").to_string();
    let profile_path = core.app_data_dir.join("profiles").join(&id);

    std::fs::create_dir_all(&profile_path).map_err(AppError::io)?;

    let preset = req.fingerprint_preset.as_deref().unwrap_or("linux");
    let (screen_w, screen_h) = if let Some(p) = crate::fingerprint::get_preset(preset) {
        (p.screen_width, p.screen_height)
    } else {
        (1920, 1080)
    };

    // Get next kanban_order for 'new' column in this workspace
    let (max_order,): (Option<i64>,) = sqlx::query_as(
        "SELECT MAX(kanban_order) FROM profiles WHERE workspace_id = ? AND kanban_status = 'new'",
    )
    .bind(&workspace_id)
    .fetch_one(&core.db)
    .await
    .map_err(AppError::db)?;
    let kanban_order = max_order.unwrap_or(-1) + 1;

    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    sqlx::query(
        "INSERT INTO profiles
        (id, name, status, profile_path, browser_type, proxy_id, fingerprint_preset,
         user_agent, platform, timezone, locale, languages, screen_width, screen_height,
         webrtc_mode, geolocation_enabled, latitude, longitude, webgl_vendor, webgl_renderer,
         notes, workspace_id, kanban_status, kanban_order, tags, default_search_engine,
         history_enabled, created_at, updated_at)
        VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&id)
    .bind(&req.name)
    .bind("stopped")
    .bind(profile_path.to_string_lossy().as_ref())
    .bind(req.browser_type.as_deref().unwrap_or("camoufox"))
    .bind(&req.proxy_id)
    .bind(preset)
    .bind(&req.user_agent)
    .bind(&req.platform)
    .bind(&req.timezone)
    .bind(req.locale.as_deref().unwrap_or("en-US"))
    .bind(req.languages.as_deref().unwrap_or("en-US,en"))
    .bind(req.screen_width.unwrap_or(screen_w))
    .bind(req.screen_height.unwrap_or(screen_h))
    .bind(req.webrtc_mode.as_deref().unwrap_or("disable"))
    .bind(req.geolocation_enabled.unwrap_or(false))
    .bind(req.latitude)
    .bind(req.longitude)
    .bind(&req.webgl_vendor)
    .bind(&req.webgl_renderer)
    .bind(&req.notes)
    .bind(&workspace_id)
    .bind("new")
    .bind(kanban_order)
    .bind("[]")
    .bind(req.default_search_engine.as_deref().unwrap_or("ddg"))
    .bind(req.history_enabled.unwrap_or(true))
    .bind(now.to_rfc3339())
    .bind(now.to_rfc3339())
    .execute(&mut *tx)
    .await
    .map_err(AppError::db)?;
    directory::relabel(&core.directory, &mut tx, &directory::PROFILE, &id).await?;
    tx.commit().await.map_err(AppError::db)?;

    profile_get(id, core)
        .await
        .and_then(|p| p.ok_or_else(|| AppError::not_found("Profile not found after create")))
}

/// Update a profile. Field contract (matches EditProfilePanel.svelte, which
/// always submits the complete form state):
///
/// * NOT-NULL columns (`name`, `browser_type`, `fingerprint_preset`, `locale`,
///   `languages`, `screen_width/height`, `webrtc_mode`, `geolocation_enabled`,
///   `default_search_engine`, `history_enabled`) use COALESCE — omitting the
///   field (null) keeps the stored value, so a partial request can never wipe
///   them.
/// * Nullable columns (`proxy_id`, `user_agent`, `platform`, `timezone`,
///   `latitude`, `longitude`, `webgl_vendor`, `webgl_renderer`, `notes`) are
///   bound raw on purpose: null *clears* the column. The UI relies on this —
///   e.g. selecting "None" detaches the proxy (proxy_id: null), and every save
///   resets `user_agent` to null so the effective UA follows the (possibly
///   changed) fingerprint preset. Callers must therefore send the full state
///   for these fields, not a partial diff.
#[tauri::command]
pub async fn profile_update(
    id: String,
    req: UpdateProfileRequest,
    core: tauri::State<'_, Core>,
    browser: tauri::State<'_, BrowserState>,
) -> CmdResult<Profile> {
    if browser.running.is_running(&id).await {
        return Err(AppError::other(
            "Cannot update a running profile. Stop it first.",
        ));
    }

    let now = Utc::now();

    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    sqlx::query(
        "UPDATE profiles SET
            name = COALESCE(?, name),
            browser_type = COALESCE(?, browser_type),
            proxy_id = ?,
            fingerprint_preset = COALESCE(?, fingerprint_preset),
            user_agent = ?,
            platform = ?,
            timezone = ?,
            locale = COALESCE(?, locale),
            languages = COALESCE(?, languages),
            screen_width = COALESCE(?, screen_width),
            screen_height = COALESCE(?, screen_height),
            webrtc_mode = COALESCE(?, webrtc_mode),
            geolocation_enabled = COALESCE(?, geolocation_enabled),
            latitude = ?,
            longitude = ?,
            webgl_vendor = ?,
            webgl_renderer = ?,
            notes = ?,
            default_search_engine = COALESCE(?, default_search_engine),
            history_enabled = COALESCE(?, history_enabled),
            updated_at = ?
        WHERE id = ?",
    )
    .bind(&req.name)
    .bind(&req.browser_type)
    .bind(&req.proxy_id)
    .bind(&req.fingerprint_preset)
    .bind(&req.user_agent)
    .bind(&req.platform)
    .bind(&req.timezone)
    .bind(&req.locale)
    .bind(&req.languages)
    .bind(req.screen_width)
    .bind(req.screen_height)
    .bind(&req.webrtc_mode)
    .bind(req.geolocation_enabled)
    .bind(req.latitude)
    .bind(req.longitude)
    .bind(&req.webgl_vendor)
    .bind(&req.webgl_renderer)
    .bind(&req.notes)
    .bind(&req.default_search_engine)
    .bind(req.history_enabled)
    .bind(now.to_rfc3339())
    .bind(&id)
    .execute(&mut *tx)
    .await
    .map_err(AppError::db)?;
    directory::relabel(&core.directory, &mut tx, &directory::PROFILE, &id).await?;
    tx.commit().await.map_err(AppError::db)?;

    profile_get(id, core)
        .await
        .and_then(|p| p.ok_or_else(|| AppError::not_found("Profile not found after update")))
}

#[tauri::command]
pub async fn profile_delete(
    id: String,
    core: tauri::State<'_, Core>,
    browser: tauri::State<'_, BrowserState>,
    app_handle: tauri::AppHandle,
) -> CmdResult<()> {
    // Stop browser process if running before deleting files
    if browser.running.is_running(&id).await {
        browser_launch::stop(&id, &browser.running).await.ok();
        let ids = browser.running.running_ids().await;
        browser_launch::emit_running_changed(&app_handle, ids);
    }

    remove_profile(&id, &core).await
}

/// Remove a profile that is not running: its directory, then its row and
/// its label.
pub(crate) async fn remove_profile(id: &str, core: &Core) -> Result<(), AppError> {
    let profile = sqlx::query_as::<_, Profile>("SELECT * FROM profiles WHERE id = ?")
        .bind(id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?;

    if let Some(p) = profile {
        if std::path::Path::new(&p.profile_path).exists() {
            std::fs::remove_dir_all(&p.profile_path).map_err(AppError::io)?;
        }
    }

    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    sqlx::query("DELETE FROM profiles WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;
    directory::relabel(&core.directory, &mut tx, &directory::PROFILE, id).await?;
    tx.commit().await.map_err(AppError::db)?;

    Ok(())
}

#[tauri::command]
pub async fn profile_clone(
    id: String,
    core: tauri::State<'_, Core>,
    browser: tauri::State<'_, BrowserState>,
) -> CmdResult<Profile> {
    if browser.running.is_running(&id).await {
        return Err(AppError::other(
            "Cannot clone a running profile. Stop it first.",
        ));
    }

    let original = sqlx::query_as::<_, Profile>("SELECT * FROM profiles WHERE id = ?")
        .bind(&id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Profile not found"))?;

    let new_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let new_path = core.app_data_dir.join("profiles").join(&new_id);

    std::fs::create_dir_all(&new_path).map_err(AppError::io)?;

    let original_profile_dir = PathBuf::from(&original.profile_path).join("firefox-profile");
    let new_profile_dir = new_path.join("firefox-profile");

    if original_profile_dir.exists() {
        copy_dir_all(&original_profile_dir, &new_profile_dir).map_err(AppError::io)?;
    }

    let cloned_name = format!("{} (copy)", original.name);
    let workspace_id = original.workspace_id.as_deref().unwrap_or("default");

    let (max_order,): (Option<i64>,) = sqlx::query_as(
        "SELECT MAX(kanban_order) FROM profiles WHERE workspace_id = ? AND kanban_status = ?",
    )
    .bind(workspace_id)
    .bind(&original.kanban_status)
    .fetch_one(&core.db)
    .await
    .map_err(AppError::db)?;
    let kanban_order = max_order.unwrap_or(-1) + 1;

    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    sqlx::query(
        "INSERT INTO profiles
        (id, name, status, profile_path, browser_type, proxy_id, fingerprint_preset,
         user_agent, platform, timezone, locale, languages, screen_width, screen_height,
         webrtc_mode, geolocation_enabled, latitude, longitude, webgl_vendor, webgl_renderer,
         notes, workspace_id, kanban_status, kanban_order, tags, default_search_engine,
         history_enabled, created_at, updated_at)
        VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&new_id)
    .bind(&cloned_name)
    .bind("stopped")
    .bind(new_path.to_string_lossy().as_ref())
    .bind(&original.browser_type)
    .bind(&original.proxy_id)
    .bind(&original.fingerprint_preset)
    .bind(&original.user_agent)
    .bind(&original.platform)
    .bind(&original.timezone)
    .bind(&original.locale)
    .bind(&original.languages)
    .bind(original.screen_width)
    .bind(original.screen_height)
    .bind(&original.webrtc_mode)
    .bind(original.geolocation_enabled)
    .bind(original.latitude)
    .bind(original.longitude)
    .bind(&original.webgl_vendor)
    .bind(&original.webgl_renderer)
    .bind(&original.notes)
    .bind(workspace_id)
    .bind(&original.kanban_status)
    .bind(kanban_order)
    .bind(&original.tags)
    .bind(&original.default_search_engine)
    .bind(original.history_enabled)
    .bind(now.to_rfc3339())
    .bind(now.to_rfc3339())
    .execute(&mut *tx)
    .await
    .map_err(AppError::db)?;
    directory::relabel(&core.directory, &mut tx, &directory::PROFILE, &new_id).await?;
    tx.commit().await.map_err(AppError::db)?;

    profile_get(new_id, core)
        .await
        .and_then(|p| p.ok_or_else(|| AppError::not_found("Profile not found after clone")))
}

#[tauri::command]
pub async fn profile_raw_data(
    id: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<crate::models::ProfileRawData> {
    let profile =
        sqlx::query_as::<_, crate::models::Profile>("SELECT * FROM profiles WHERE id = ?")
            .bind(&id)
            .fetch_optional(&core.db)
            .await
            .map_err(AppError::db)?
            .ok_or_else(|| AppError::not_found("Profile not found"))?;

    let proxy = if let Some(proxy_id) = &profile.proxy_id {
        sqlx::query_as::<_, crate::models::Proxy>("SELECT * FROM proxies WHERE id = ?")
            .bind(proxy_id)
            .fetch_optional(&core.db)
            .await
            .map_err(AppError::db)?
    } else {
        None
    };

    let cfg = super::build_camoufox_config(&profile);
    let camoufox_config = serde_json::to_string_pretty(&cfg).unwrap_or_default();
    let user_js = crate::browser::userjs::generate(&profile, proxy.as_ref());

    let seed = profile
        .id
        .bytes()
        .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));

    let effective_ua = profile
        .user_agent
        .clone()
        .or_else(|| {
            crate::fingerprint::get_preset(&profile.fingerprint_preset)
                .map(|p| p.user_agent.to_owned())
        })
        .unwrap_or_else(|| {
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:127.0) Gecko/20100101 Firefox/127.0"
                .to_owned()
        });

    let effective_platform = profile
        .platform
        .clone()
        .or_else(|| {
            crate::fingerprint::get_preset(&profile.fingerprint_preset)
                .map(|p| p.platform.to_owned())
        })
        .unwrap_or_else(|| "Win32".to_owned());

    let cookies_db_path = PathBuf::from(&profile.profile_path)
        .join("firefox-profile")
        .join("cookies.sqlite");

    let cookies = if cookies_db_path.exists() {
        tokio::task::spawn_blocking(move || read_firefox_cookies(&cookies_db_path))
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    Ok(crate::models::ProfileRawData {
        user_agent: effective_ua,
        platform: effective_platform,
        locale: profile.locale.clone(),
        languages: profile.languages.clone(),
        timezone: profile.timezone.clone(),
        screen_width: profile.screen_width,
        screen_height: profile.screen_height,
        webrtc_mode: profile.webrtc_mode.clone(),
        webgl_vendor: profile.webgl_vendor.clone(),
        webgl_renderer: profile.webgl_renderer.clone(),
        canvas_seed: seed,
        audio_seed: seed.wrapping_add(1),
        fonts_seed: seed.wrapping_add(2),
        geolocation_enabled: profile.geolocation_enabled,
        latitude: profile.latitude,
        longitude: profile.longitude,
        camoufox_config,
        user_js,
        cookies,
    })
}

fn read_firefox_cookies(path: &std::path::Path) -> Vec<CookieEntry> {
    let conn = match rusqlite::Connection::open(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut stmt = match conn.prepare(
        "SELECT host, name, value, path, expiry, isSecure, isHttpOnly FROM moz_cookies ORDER BY host, name LIMIT 2000"
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = match stmt.query_map([], |row| {
        Ok(CookieEntry {
            host: row.get(0)?,
            name: row.get(1)?,
            value: row.get(2)?,
            path: row.get(3)?,
            expiry: row.get(4)?,
            secure: row.get::<_, i32>(5).unwrap_or(0) != 0,
            http_only: row.get::<_, i32>(6).unwrap_or(0) != 0,
        })
    }) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    rows.filter_map(|r| r.ok()).collect()
}

pub fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_symlink() {
            // skip symlinks (e.g. Firefox 'lock' file points to a non-existent path)
        } else if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}
