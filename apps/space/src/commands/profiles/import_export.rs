// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use crate::error::{AppError, CmdResult};
use crate::models::{
    ExportOptions, Profile, ProfileExport, ProfileExportData, Proxy, ProxyExportData,
};
use crate::modules::directory;
use std::path::PathBuf;
use veydan_core::Core;

/// Runtime-only entries of a firefox-profile: locks and caches. Shared by
/// export, backup and vault sync.
pub(crate) const PROFILE_FILE_BLACKLIST: &[&str] = &[
    "lock",
    ".parentlock",
    "parent.lock",
    "compatibility.ini",
    "cache2",
    "startupCache",
    "shader-cache",
    "crashes",
    "minidumps",
];

pub(crate) fn is_blacklisted(name: &str) -> bool {
    PROFILE_FILE_BLACKLIST.contains(&name)
}

fn build_profile_export(
    profile: &Profile,
    proxy: Option<&Proxy>,
    options: &ExportOptions,
) -> ProfileExport {
    let tags: Vec<String> = serde_json::from_str(&profile.tags).unwrap_or_default();

    let proxy_data = if options.include_proxy {
        proxy.map(|p| ProxyExportData {
            name: p.name.clone(),
            proxy_type: p.proxy_type.clone(),
            host: p.host.clone(),
            port: p.port,
            username: p.username.clone(),
            password: if options.include_proxy_password {
                p.password.clone()
            } else {
                None
            },
            country: p.country.clone(),
            city: p.city.clone(),
        })
    } else {
        None
    };

    ProfileExport {
        version: "1".to_string(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        profile: ProfileExportData {
            name: profile.name.clone(),
            browser_type: profile.browser_type.clone(),
            fingerprint_preset: profile.fingerprint_preset.clone(),
            user_agent: profile.user_agent.clone(),
            platform: profile.platform.clone(),
            timezone: profile.timezone.clone(),
            locale: profile.locale.clone(),
            languages: profile.languages.clone(),
            screen_width: profile.screen_width,
            screen_height: profile.screen_height,
            webrtc_mode: profile.webrtc_mode.clone(),
            geolocation_enabled: profile.geolocation_enabled,
            latitude: profile.latitude,
            longitude: profile.longitude,
            webgl_vendor: profile.webgl_vendor.clone(),
            webgl_renderer: profile.webgl_renderer.clone(),
            notes: profile.notes.clone(),
            kanban_status: profile.kanban_status.clone(),
            tags,
        },
        proxy: proxy_data,
    }
}

async fn load_profile_and_proxy(
    id: &str,
    core: &tauri::State<'_, Core>,
) -> CmdResult<(Profile, Option<Proxy>)> {
    let profile = sqlx::query_as::<_, Profile>("SELECT * FROM profiles WHERE id = ?")
        .bind(id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Profile not found"))?;

    let proxy = if let Some(proxy_id) = &profile.proxy_id {
        sqlx::query_as::<_, Proxy>("SELECT * FROM proxies WHERE id = ?")
            .bind(proxy_id)
            .fetch_optional(&core.db)
            .await
            .map_err(AppError::db)?
    } else {
        None
    };

    Ok((profile, proxy))
}

fn validate_export(export: &ProfileExport) -> Result<(), AppError> {
    if export.version != "1" {
        return Err(AppError::other(format!(
            "Unsupported export version: {}",
            export.version
        )));
    }

    let p = &export.profile;

    if !["camoufox", "firefox"].contains(&p.browser_type.as_str()) {
        return Err(AppError::other(format!(
            "Invalid browser_type: {}",
            p.browser_type
        )));
    }
    if !["win10", "win11", "macos", "linux"].contains(&p.fingerprint_preset.as_str()) {
        return Err(AppError::other(format!(
            "Invalid fingerprint_preset: {}",
            p.fingerprint_preset
        )));
    }
    if !["disable", "proxy_only", "real_ip"].contains(&p.webrtc_mode.as_str()) {
        return Err(AppError::other(format!(
            "Invalid webrtc_mode: {}",
            p.webrtc_mode
        )));
    }
    if p.screen_width <= 0 || p.screen_height <= 0 {
        return Err(AppError::other("Invalid screen dimensions"));
    }
    if p.locale.is_empty() {
        return Err(AppError::other("locale cannot be empty"));
    }

    if let Some(proxy) = &export.proxy {
        if !["http", "https", "socks5", "ssh"].contains(&proxy.proxy_type.as_str()) {
            return Err(AppError::other(format!(
                "Invalid proxy_type: {}",
                proxy.proxy_type
            )));
        }
        if proxy.port < 1 || proxy.port > 65535 {
            return Err(AppError::other(format!(
                "Invalid proxy port: {}",
                proxy.port
            )));
        }
    }

    Ok(())
}

async fn create_profile_from_export(
    export: ProfileExport,
    workspace_id: Option<String>,
    core: &tauri::State<'_, Core>,
) -> CmdResult<Profile> {
    let workspace_id = workspace_id.unwrap_or_else(|| "default".to_string());

    let p = &export.profile;

    // Resolve name: append " (imported)" if collision
    let base_name = p.name.clone();
    let existing: Option<(String,)> =
        sqlx::query_as("SELECT id FROM profiles WHERE name = ? AND workspace_id = ?")
            .bind(&base_name)
            .bind(&workspace_id)
            .fetch_optional(&core.db)
            .await
            .map_err(AppError::db)?;

    let name = if existing.is_some() {
        format!("{} (imported)", base_name)
    } else {
        base_name
    };

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
    let profile_path = core.app_data_dir.join("profiles").join(&id);
    std::fs::create_dir_all(&profile_path).map_err(AppError::io)?;

    let (max_order,): (Option<i64>,) = sqlx::query_as(
        "SELECT MAX(kanban_order) FROM profiles WHERE workspace_id = ? AND kanban_status = ?",
    )
    .bind(&workspace_id)
    .bind(&p.kanban_status)
    .fetch_one(&core.db)
    .await
    .map_err(AppError::db)?;
    let kanban_order = max_order.unwrap_or(-1) + 1;

    // The proxy, the profile and their labels together: a profile that fails
    // to go in leaves no proxy behind.
    let inserted = insert_export(
        core,
        &export,
        &id,
        &name,
        &workspace_id,
        kanban_order,
        &profile_path,
        now,
    )
    .await;
    if let Err(e) = inserted {
        let _ = std::fs::remove_dir_all(&profile_path);
        return Err(e);
    }

    super::profile_get(id, core.clone())
        .await
        .and_then(|p| p.ok_or_else(|| AppError::not_found("Profile not found after import")))
}

/// Insert the proxy of `export`, if it has one, and the profile `id` in one
/// transaction with their labels.
#[allow(clippy::too_many_arguments)]
async fn insert_export(
    core: &Core,
    export: &ProfileExport,
    id: &str,
    name: &str,
    workspace_id: &str,
    kanban_order: i64,
    profile_path: &std::path::Path,
    now: chrono::DateTime<chrono::Utc>,
) -> CmdResult<()> {
    let p = &export.profile;
    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    let proxy_id: Option<String> = if let Some(proxy_data) = &export.proxy {
        let pid = uuid::Uuid::new_v4().to_string();
        let tags_json = format!("[\"workspace:{}\"]", workspace_id);
        sqlx::query(
            "INSERT INTO proxies (id, name, proxy_type, host, port, username, password, country, city, status, tags, created_at)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&pid)
        .bind(&proxy_data.name)
        .bind(&proxy_data.proxy_type)
        .bind(&proxy_data.host)
        .bind(proxy_data.port)
        .bind(&proxy_data.username)
        .bind(&proxy_data.password)
        .bind(&proxy_data.country)
        .bind(&proxy_data.city)
        .bind("unknown")
        .bind(&tags_json)
        .bind(now.to_rfc3339())
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;
        directory::relabel(&core.directory, &mut tx, &directory::PROXY, &pid).await?;
        Some(pid)
    } else {
        None
    };

    let tags_json = serde_json::to_string(&p.tags).unwrap_or_else(|_| "[]".to_string());
    sqlx::query(
        "INSERT INTO profiles
        (id, name, status, profile_path, browser_type, proxy_id, fingerprint_preset,
         user_agent, platform, timezone, locale, languages, screen_width, screen_height,
         webrtc_mode, geolocation_enabled, latitude, longitude, webgl_vendor, webgl_renderer,
         notes, workspace_id, kanban_status, kanban_order, tags, created_at, updated_at)
        VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(id)
    .bind(name)
    .bind("stopped")
    .bind(profile_path.to_string_lossy().as_ref())
    .bind(&p.browser_type)
    .bind(&proxy_id)
    .bind(&p.fingerprint_preset)
    .bind(&p.user_agent)
    .bind(&p.platform)
    .bind(&p.timezone)
    .bind(&p.locale)
    .bind(&p.languages)
    .bind(p.screen_width)
    .bind(p.screen_height)
    .bind(&p.webrtc_mode)
    .bind(p.geolocation_enabled)
    .bind(p.latitude)
    .bind(p.longitude)
    .bind(&p.webgl_vendor)
    .bind(&p.webgl_renderer)
    .bind(&p.notes)
    .bind(workspace_id)
    .bind(&p.kanban_status)
    .bind(kanban_order)
    .bind(&tags_json)
    .bind(now.to_rfc3339())
    .bind(now.to_rfc3339())
    .execute(&mut *tx)
    .await
    .map_err(AppError::db)?;
    directory::relabel(&core.directory, &mut tx, &directory::PROFILE, id).await?;
    tx.commit().await.map_err(AppError::db)
}

/// Take back an imported profile and the proxy it brought, with their labels.
async fn remove_import(core: &Core, profile: &Profile) {
    let removed = async {
        let mut tx = core.db.begin().await.map_err(AppError::db)?;
        sqlx::query("DELETE FROM profiles WHERE id = ?")
            .bind(&profile.id)
            .execute(&mut *tx)
            .await
            .map_err(AppError::db)?;
        directory::relabel(&core.directory, &mut tx, &directory::PROFILE, &profile.id).await?;
        if let Some(proxy_id) = &profile.proxy_id {
            sqlx::query("DELETE FROM proxies WHERE id = ?")
                .bind(proxy_id)
                .execute(&mut *tx)
                .await
                .map_err(AppError::db)?;
            directory::relabel(&core.directory, &mut tx, &directory::PROXY, proxy_id).await?;
        }
        tx.commit().await.map_err(AppError::db)
    };
    let _: CmdResult<()> = removed.await;
}

#[tauri::command]
pub async fn profile_export_json(
    id: String,
    options: ExportOptions,
    core: tauri::State<'_, Core>,
) -> CmdResult<String> {
    let (profile, proxy) = load_profile_and_proxy(&id, &core).await?;
    let export = build_profile_export(&profile, proxy.as_ref(), &options);
    serde_json::to_string_pretty(&export).map_err(AppError::other)
}

#[tauri::command]
pub async fn profile_export_zip(
    id: String,
    options: ExportOptions,
    output_path: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<()> {
    use std::io::Write;
    use zip::write::FileOptions;

    let (profile, proxy) = load_profile_and_proxy(&id, &core).await?;
    let export = build_profile_export(&profile, proxy.as_ref(), &options);
    let json = serde_json::to_string_pretty(&export).map_err(AppError::other)?;

    let file = std::fs::File::create(&output_path).map_err(AppError::io)?;
    let mut zip = zip::ZipWriter::new(file);

    let opts: FileOptions<()> =
        FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("profile.json", opts)
        .map_err(AppError::other)?;
    zip.write_all(json.as_bytes()).map_err(AppError::io)?;

    if options.include_files {
        let firefox_dir = PathBuf::from(&profile.profile_path).join("firefox-profile");
        if firefox_dir.exists() {
            add_dir_to_zip(&mut zip, &firefox_dir, "firefox-profile", opts)
                .map_err(AppError::io)?;
        }
    }

    zip.finish().map_err(AppError::other)?;
    Ok(())
}

fn add_dir_to_zip(
    zip: &mut zip::ZipWriter<std::fs::File>,
    dir: &std::path::Path,
    prefix: &str,
    opts: zip::write::FileOptions<()>,
) -> std::io::Result<()> {
    use std::io::Write;

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if is_blacklisted(&name_str) {
            continue;
        }

        let zip_path = format!("{prefix}/{name_str}");
        let ty = entry.file_type()?;

        if ty.is_dir() {
            add_dir_to_zip(zip, &entry.path(), &zip_path, opts)?;
        } else {
            zip.start_file(&zip_path, opts)
                .map_err(std::io::Error::other)?;
            let data = std::fs::read(entry.path())?;
            zip.write_all(&data)?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn profile_import_json(
    json: String,
    workspace_id: Option<String>,
    core: tauri::State<'_, Core>,
) -> CmdResult<Profile> {
    let export: ProfileExport =
        serde_json::from_str(&json).map_err(|e| AppError::other(format!("Invalid JSON: {e}")))?;
    validate_export(&export)?;
    create_profile_from_export(export, workspace_id, &core).await
}

#[tauri::command]
pub async fn profile_import_zip(
    file_path: String,
    workspace_id: Option<String>,
    core: tauri::State<'_, Core>,
) -> CmdResult<Profile> {
    let tmp_dir = core
        .app_data_dir
        .join("import_tmp")
        .join(uuid::Uuid::new_v4().to_string());

    std::fs::create_dir_all(&tmp_dir).map_err(AppError::io)?;

    let result = import_zip_inner(&file_path, workspace_id, &core, &tmp_dir).await;

    std::fs::remove_dir_all(&tmp_dir).ok();

    result
}

async fn import_zip_inner(
    file_path: &str,
    workspace_id: Option<String>,
    core: &tauri::State<'_, Core>,
    tmp_dir: &std::path::Path,
) -> CmdResult<Profile> {
    let zip_file = std::fs::File::open(file_path).map_err(AppError::io)?;
    let mut archive = zip::ZipArchive::new(zip_file).map_err(AppError::other)?;

    // Security: validate all paths before extracting
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(AppError::other)?;
        let entry_path = entry
            .enclosed_name()
            .ok_or_else(|| AppError::other("Invalid path in ZIP"))?;

        if entry_path.is_absolute() {
            return Err(AppError::other("Invalid path in ZIP: absolute path"));
        }
        for component in entry_path.components() {
            if let std::path::Component::ParentDir = component {
                return Err(AppError::other("Invalid path in ZIP: path traversal"));
            }
        }
    }

    // Extract all entries
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(AppError::other)?;
        let entry_path = entry
            .enclosed_name()
            .ok_or_else(|| AppError::other("Invalid path in ZIP"))?
            .to_owned();
        let dest = tmp_dir.join(&entry_path);

        if entry.is_dir() {
            std::fs::create_dir_all(&dest).map_err(AppError::io)?;
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(AppError::io)?;
            }
            let mut out = std::fs::File::create(&dest).map_err(AppError::io)?;
            std::io::copy(&mut entry, &mut out).map_err(AppError::io)?;
        }
    }

    // Read and parse profile.json
    let json_path = tmp_dir.join("profile.json");
    let json = std::fs::read_to_string(&json_path)
        .map_err(|_| AppError::other("profile.json not found in ZIP"))?;

    let export: ProfileExport = serde_json::from_str(&json)
        .map_err(|e| AppError::other(format!("Invalid profile.json: {e}")))?;
    validate_export(&export)?;

    let profile = create_profile_from_export(export, workspace_id, core).await?;

    // Copy firefox-profile if present; rollback both FS and DB on failure
    let tmp_ff = tmp_dir.join("firefox-profile");
    if tmp_ff.exists() {
        let dest_ff = PathBuf::from(&profile.profile_path).join("firefox-profile");
        if let Err(e) = super::copy_dir_all(&tmp_ff, &dest_ff) {
            let _ = std::fs::remove_dir_all(&profile.profile_path);
            remove_import(core, &profile).await;
            return Err(AppError::io(format!("Failed to copy firefox-profile: {e}")));
        }
    }

    Ok(profile)
}

#[tauri::command]
pub async fn profile_import_zip_data(
    data_b64: String,
    workspace_id: Option<String>,
    core: tauri::State<'_, Core>,
) -> CmdResult<Profile> {
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data_b64.trim())
        .map_err(|e| AppError::other(format!("Invalid base64: {e}")))?;

    let tmp_dir = core
        .app_data_dir
        .join("import_tmp")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&tmp_dir).map_err(AppError::io)?;

    let extracted_dir = tmp_dir.join("extracted");
    let tmp_zip = tmp_dir.join("upload.zip");
    std::fs::write(&tmp_zip, &bytes).map_err(AppError::io)?;

    let result = import_zip_inner(
        tmp_zip
            .to_str()
            .ok_or_else(|| AppError::other("Invalid tmp path"))?,
        workspace_id,
        &core,
        &extracted_dir,
    )
    .await;

    std::fs::remove_dir_all(&tmp_dir).ok();
    result
}

/// Export profile JSON directly to a file path chosen via dialog on the frontend.
#[tauri::command]
pub async fn profile_export_json_to_file(
    id: String,
    options: ExportOptions,
    output_path: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<()> {
    let json = profile_export_json(id, options, core).await?;
    std::fs::write(&output_path, json).map_err(AppError::io)
}
