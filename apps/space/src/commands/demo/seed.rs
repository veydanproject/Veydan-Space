// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use super::content::{pack, DemoPack};
use crate::error::{AppError, CmdResult};
use chrono::Utc;
use uuid::Uuid;
use veydan_core::Core;

/// Workspaces with their columns, proxies and profiles of the browser.
pub async fn seed_browser(core: &Core, locale: &str) -> CmdResult<()> {
    let pack = pack(locale);
    let now = Utc::now().to_rfc3339();
    seed_workspaces(core, &pack, &now).await?;
    seed_proxies(core, &pack, &now).await?;
    seed_profiles(core, &pack, &now).await?;
    // Extra volume for a fuller video demo (~5× catalog size)
    super::bulk::seed_browser(core, locale, &now).await
}

/// SSH keys and connections. A connection with a second factor names a TOTP
/// entry of pass by its id.
#[cfg(desktop)]
pub async fn seed_ssh(core: &Core, locale: &str) -> CmdResult<()> {
    let pack = pack(locale);
    let now = Utc::now().to_rfc3339();
    seed_ssh_pack(core, &pack, &now).await?;
    super::bulk::seed_ssh(core, &now).await
}

/// The rules the web clipper files captured pages by.
#[cfg(desktop)]
pub async fn seed_capture(core: &Core, locale: &str) -> CmdResult<()> {
    seed_capture_rules(core, &pack(locale)).await
}

async fn seed_workspaces(core: &Core, pack: &DemoPack, now: &str) -> CmdResult<()> {
    sqlx::query(
        "UPDATE workspaces SET name = ?, description = ?, color = ?, icon = ?, updated_at = ?
         WHERE id = 'default'",
    )
    .bind(pack.default_workspace_name)
    .bind(Some(if pack.default_workspace_name == "Личное" {
        "Личные аккаунты"
    } else {
        "Personal accounts"
    }))
    .bind("#6b7280")
    .bind("home")
    .bind(now)
    .execute(&core.db)
    .await
    .map_err(AppError::db)?;

    for (i, col) in pack.default_columns.iter().enumerate() {
        insert_column(
            core,
            "default",
            col.name,
            col.tag_name,
            col.color,
            i as i64,
            now,
        )
        .await?;
    }

    for ws in &pack.workspaces {
        sqlx::query(
            "INSERT INTO workspaces (id, name, description, color, icon, is_default, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 0, ?, ?)",
        )
        .bind(ws.id)
        .bind(ws.name)
        .bind(ws.description)
        .bind(ws.color)
        .bind(ws.icon)
        .bind(now)
        .bind(now)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;

        for (i, col) in ws.columns.iter().enumerate() {
            insert_column(
                core,
                ws.id,
                col.name,
                col.tag_name,
                col.color,
                i as i64,
                now,
            )
            .await?;
        }
    }
    Ok(())
}

async fn insert_column(
    core: &Core,
    workspace_id: &str,
    name: &str,
    tag_name: &str,
    color: &str,
    position: i64,
    now: &str,
) -> CmdResult<()> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO workspace_columns (id, workspace_id, name, tag_name, color, position, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(workspace_id)
    .bind(name)
    .bind(tag_name)
    .bind(color)
    .bind(position)
    .bind(now)
    .execute(&core.db)
    .await
    .map_err(AppError::db)?;
    Ok(())
}

async fn seed_proxies(core: &Core, pack: &DemoPack, now: &str) -> CmdResult<()> {
    for p in &pack.proxies {
        let tags = serde_json::to_string(p.tags).map_err(AppError::other)?;
        sqlx::query(
            "INSERT INTO proxies
             (id, name, proxy_type, host, port, username, password, country, city, status, tags, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'unknown', ?, ?)",
        )
        .bind(p.id)
        .bind(p.name)
        .bind(p.proxy_type)
        .bind(p.host)
        .bind(p.port)
        .bind(p.username)
        .bind(p.password)
        .bind(p.country)
        .bind(p.city)
        .bind(&tags)
        .bind(now)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;
    }
    Ok(())
}

async fn seed_profiles(core: &Core, pack: &DemoPack, now: &str) -> CmdResult<()> {
    let profiles_root = core.app_data_dir.join("profiles");
    std::fs::create_dir_all(&profiles_root).map_err(AppError::io)?;

    for p in &pack.profiles {
        let profile_path = profiles_root.join(p.id);
        std::fs::create_dir_all(&profile_path).map_err(AppError::io)?;
        let tags = serde_json::to_string(p.tags).map_err(AppError::other)?;

        sqlx::query(
            "INSERT INTO profiles
             (id, name, status, profile_path, browser_type, proxy_id, fingerprint_preset,
              user_agent, platform, timezone, locale, languages, screen_width, screen_height,
              webrtc_mode, geolocation_enabled, latitude, longitude, webgl_vendor, webgl_renderer,
              notes, workspace_id, kanban_status, kanban_order, tags, default_search_engine,
              history_enabled, created_at, updated_at)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(p.id)
        .bind(p.name)
        .bind("stopped")
        .bind(profile_path.to_string_lossy().as_ref())
        .bind("camoufox")
        .bind(p.proxy_id)
        .bind(p.fingerprint_preset)
        .bind(None::<String>)
        .bind(None::<String>)
        .bind(p.timezone)
        .bind(p.locale)
        .bind(p.languages)
        .bind(1920_i64)
        .bind(1080_i64)
        .bind("disable")
        .bind(0_i64)
        .bind(None::<f64>)
        .bind(None::<f64>)
        .bind(None::<String>)
        .bind(None::<String>)
        .bind(p.notes)
        .bind(p.workspace_id)
        .bind(p.kanban_status)
        .bind(p.kanban_order)
        .bind(&tags)
        .bind("ddg")
        .bind(1_i64)
        .bind(now)
        .bind(now)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;
    }
    Ok(())
}

#[cfg(desktop)]
async fn seed_ssh_pack(core: &Core, pack: &DemoPack, now: &str) -> CmdResult<()> {
    use crate::commands::ssh_keys::generate_key_material;

    for k in &pack.ssh_keys {
        let bits = if k.algorithm == "rsa" {
            Some(2048)
        } else {
            None
        };
        let material = tokio::task::spawn_blocking({
            let alg = k.algorithm.to_string();
            let comment = format!("demo@{}", k.name);
            move || generate_key_material(&alg, bits, comment, None)
        })
        .await
        .map_err(|e| AppError::other(e.to_string()))??;

        sqlx::query(
            "INSERT INTO ssh_keys (
                id, name, algorithm, bits, comment,
                private_key, public_key, passphrase, fingerprint, source,
                created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, NULL, ?, 'generated', ?, ?)",
        )
        .bind(k.id)
        .bind(k.name)
        .bind(&material.algorithm)
        .bind(material.bits)
        .bind(&material.comment)
        .bind(&material.private_pem)
        .bind(&material.public_openssh)
        .bind(&material.fingerprint)
        .bind(now)
        .bind(now)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;
    }

    for c in &pack.ssh_connections {
        sqlx::query(
            "INSERT INTO ssh_connections (
                id, name, host, port, username, auth_type,
                password, private_key, key_passphrase, ssh_key_id,
                requires_2fa, totp_entry_id, proxy_id,
                connect_timeout_sec, keepalive_sec, terminal_theme,
                default_cols, default_rows, created_at, updated_at
            ) VALUES (
                ?, ?, ?, ?, ?, ?,
                ?, NULL, NULL, ?,
                ?, ?, ?,
                15, 30, NULL,
                120, 32, ?, ?
            )",
        )
        .bind(c.id)
        .bind(c.name)
        .bind(c.host)
        .bind(c.port)
        .bind(c.username)
        .bind(c.auth_type)
        .bind(c.password)
        .bind(c.ssh_key_id)
        .bind(c.requires_2fa as i64)
        .bind(c.totp_entry_id)
        .bind(c.proxy_id)
        .bind(now)
        .bind(now)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;

        for wid in c.workspace_ids {
            sqlx::query(
                "INSERT OR IGNORE INTO ssh_connection_workspaces (connection_id, workspace_id) VALUES (?, ?)",
            )
            .bind(c.id)
            .bind(wid)
            .execute(&core.db)
            .await
            .map_err(AppError::db)?;
        }
        for pid in c.profile_ids {
            sqlx::query(
                "INSERT OR IGNORE INTO ssh_connection_profiles (connection_id, profile_id) VALUES (?, ?)",
            )
            .bind(c.id)
            .bind(pid)
            .execute(&core.db)
            .await
            .map_err(AppError::db)?;
        }
    }
    Ok(())
}

#[cfg(desktop)]
async fn seed_capture_rules(core: &Core, pack: &DemoPack) -> CmdResult<()> {
    use veydan_notes::capture::CaptureRule;

    let rules: Vec<CaptureRule> = pack
        .capture_rules
        .iter()
        .map(|r| CaptureRule {
            domain: r.domain.to_string(),
            folder_id: Some(r.folder_id.to_string()),
            tags: r.tags.iter().map(|s| s.to_string()).collect(),
            template_id: None,
        })
        .collect();
    veydan_core::settings::set_json(&core.db, crate::capture::rules::KEY, &rules).await
}
