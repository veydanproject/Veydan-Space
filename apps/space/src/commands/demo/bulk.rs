// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Extra demo volume (~5×) generated on top of the handcrafted pack. The
//! bulk TOTP entries are pass's, the bulk notes are notes' (`veydan_notes`).

use crate::error::{AppError, CmdResult};
use uuid::Uuid;
use veydan_core::Core;

const WS: &[&str] = &[
    "default",
    "demo-ws-smm",
    "demo-ws-dev",
    "demo-ws-devops",
    "demo-ws-biz",
    "demo-ws-freelance",
    "demo-ws-research",
];

const KANBAN: &[&[&str]] = &[
    &["inbox", "active", "done"],
    &["ideas", "in_progress", "published"],
    &["backlog", "coding", "review", "done_dev"],
    &["plan", "deploy", "monitor"],
    &["lead", "negotiation", "closed"],
    &["inbox", "active", "done"],
    &["inbox", "active", "done"],
];

const PRESETS: &[&str] = &["win10", "win11", "macos", "linux"];
const PROXY_TYPES: &[&str] = &["http", "https", "socks5"];
const COUNTRIES: &[(&str, &str)] = &[
    ("US", "New York"),
    ("DE", "Berlin"),
    ("NL", "Amsterdam"),
    ("GB", "London"),
    ("SG", "Singapore"),
    ("JP", "Tokyo"),
    ("FR", "Paris"),
    ("CA", "Toronto"),
    ("AU", "Sydney"),
    ("PL", "Warsaw"),
    ("ES", "Madrid"),
    ("IT", "Milan"),
    ("SE", "Stockholm"),
    ("BR", "Sao Paulo"),
    ("IN", "Mumbai"),
    ("KR", "Seoul"),
];

/// The bulk profiles: `demo-pr-bulk-01` to `demo-pr-bulk-56`. TOTP entries
/// of pass and notes link them by these ids.
const BULK_PROFILES: u32 = 56;

fn bulk_profile_ids() -> Vec<String> {
    (1..=BULK_PROFILES)
        .map(|i| format!("demo-pr-bulk-{i:02}"))
        .collect()
}

/// Extra workspaces, bulk proxies and profiles.
pub async fn seed_browser(core: &Core, locale: &str, now: &str) -> CmdResult<()> {
    let ru = locale.eq_ignore_ascii_case("ru");
    seed_extra_workspaces(core, ru, now).await?;
    seed_bulk_proxies(core, now).await?;
    seed_bulk_profiles(core, ru, now).await
}

#[cfg(desktop)]
pub async fn seed_ssh(core: &Core, now: &str) -> CmdResult<()> {
    seed_bulk_ssh(core, now).await
}

async fn seed_extra_workspaces(core: &Core, ru: bool, now: &str) -> CmdResult<()> {
    let extras: &[(&str, &str, &str, &str, &str)] = if ru {
        &[
            (
                "demo-ws-freelance",
                "Фриланс",
                "Клиентские проекты",
                "#8b5cf6",
                "laptop",
            ),
            (
                "demo-ws-research",
                "Исследования",
                "Разведка и конкуренты",
                "#14b8a6",
                "search",
            ),
        ]
    } else {
        &[
            (
                "demo-ws-freelance",
                "Freelance",
                "Client projects",
                "#8b5cf6",
                "laptop",
            ),
            (
                "demo-ws-research",
                "Research",
                "Competitive intel",
                "#14b8a6",
                "search",
            ),
        ]
    };

    for (id, name, desc, color, icon) in extras {
        sqlx::query(
            "INSERT OR IGNORE INTO workspaces (id, name, description, color, icon, is_default, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 0, ?, ?)",
        )
        .bind(id)
        .bind(name)
        .bind(desc)
        .bind(color)
        .bind(icon)
        .bind(now)
        .bind(now)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;

        for (i, (col_name, tag)) in [
            ("Inbox", "inbox"),
            (if ru { "В работе" } else { "Active" }, "active"),
            (if ru { "Готово" } else { "Done" }, "done"),
        ]
        .into_iter()
        .enumerate()
        {
            let col_id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO workspace_columns (id, workspace_id, name, tag_name, color, position, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&col_id)
            .bind(id)
            .bind(col_name)
            .bind(tag)
            .bind(["#94a3b8", "#3b82f6", "#22c55e"][i])
            .bind(i as i64)
            .bind(now)
            .execute(&core.db)
            .await
            .map_err(AppError::db)?;
        }
    }
    Ok(())
}

async fn seed_bulk_proxies(core: &Core, now: &str) -> CmdResult<()> {
    // 8 handcrafted + 32 bulk = 40
    for i in 1..=32 {
        let (cc, city) = COUNTRIES[(i as usize - 1) % COUNTRIES.len()];
        let ptype = PROXY_TYPES[(i as usize - 1) % PROXY_TYPES.len()];
        let ws = WS[(i as usize - 1) % WS.len()];
        let id = format!("demo-px-bulk-{i:02}");
        let name = format!("{cc} {ptype} #{i:02}");
        let host = format!("proxy-{}.example", cc.to_lowercase());
        let port = 10000 + i as i64;
        let tags =
            serde_json::to_string(&vec![format!("workspace:{ws}")]).map_err(AppError::other)?;
        let user = format!("demo-u{i}");
        let pass = format!("demo-pass-{i}");

        sqlx::query(
            "INSERT INTO proxies
             (id, name, proxy_type, host, port, username, password, country, city, status, tags, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'unknown', ?, ?)",
        )
        .bind(&id)
        .bind(&name)
        .bind(ptype)
        .bind(&host)
        .bind(port)
        .bind(&user)
        .bind(&pass)
        .bind(cc)
        .bind(city)
        .bind(&tags)
        .bind(now)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;
    }
    Ok(())
}

async fn seed_bulk_profiles(core: &Core, ru: bool, now: &str) -> CmdResult<()> {
    let profiles_root = core.app_data_dir.join("profiles");
    std::fs::create_dir_all(&profiles_root).map_err(AppError::io)?;

    let labels_en = [
        "Shop",
        "Ads",
        "Support",
        "QA",
        "Staging",
        "Docs",
        "Blog",
        "Forum",
        "Store",
        "Portal",
        "Admin",
        "Partner",
        "Affiliate",
        "Review",
        "Monitor",
        "Backup",
        "Legacy",
        "Sandbox",
        "Client",
        "Vendor",
    ];
    let labels_ru = [
        "Магазин",
        "Реклама",
        "Поддержка",
        "QA",
        "Staging",
        "Доки",
        "Блог",
        "Форум",
        "Витрина",
        "Портал",
        "Админ",
        "Партнёр",
        "Аффилиат",
        "Обзор",
        "Монитор",
        "Бэкап",
        "Legacy",
        "Песочница",
        "Клиент",
        "Вендор",
    ];
    let labels = if ru { &labels_ru[..] } else { &labels_en[..] };

    // 14 handcrafted + 56 bulk = 70
    for (i, id) in (1..=BULK_PROFILES).zip(bulk_profile_ids()) {
        let ws_i = (i as usize - 1) % WS.len();
        let ws = WS[ws_i];
        let cols = KANBAN[ws_i];
        let status = cols[(i as usize - 1) % cols.len()];
        let label = labels[(i as usize - 1) % labels.len()];
        let name = format!("{label} #{i:02}");
        let profile_path = profiles_root.join(&id);
        std::fs::create_dir_all(&profile_path).map_err(AppError::io)?;
        let tags = serde_json::to_string(&vec![status.to_string()]).map_err(AppError::other)?;
        let proxy_id = if i % 3 == 0 {
            Some(format!("demo-px-bulk-{:02}", ((i - 1) % 32) + 1))
        } else {
            None
        };
        let preset = PRESETS[(i as usize - 1) % PRESETS.len()];
        let (cc, _) = COUNTRIES[(i as usize - 1) % COUNTRIES.len()];

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
        .bind(&name)
        .bind("stopped")
        .bind(profile_path.to_string_lossy().as_ref())
        .bind("camoufox")
        .bind(&proxy_id)
        .bind(preset)
        .bind(None::<String>)
        .bind(None::<String>)
        .bind(None::<String>)
        .bind(format!("en-{cc}"))
        .bind("en-US,en")
        .bind(1920_i64)
        .bind(1080_i64)
        .bind("disable")
        .bind(0_i64)
        .bind(None::<f64>)
        .bind(None::<f64>)
        .bind(None::<String>)
        .bind(None::<String>)
        .bind(None::<String>)
        .bind(ws)
        .bind(status)
        .bind(((i as i64 - 1) % 8) + 10)
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
async fn seed_bulk_ssh(core: &Core, now: &str) -> CmdResult<()> {
    use crate::commands::ssh_keys::generate_key_material;

    // 2 handcrafted + 4 bulk keys = 6 (ed25519 only — fast)
    let mut key_ids = vec!["demo-key-deploy".to_string(), "demo-key-laptop".to_string()];
    for i in 1..=4 {
        let id = format!("demo-key-bulk-{i}");
        let name = format!("bulk-ed25519-{i}");
        let material = tokio::task::spawn_blocking({
            let comment = format!("demo@{name}");
            move || generate_key_material("ed25519", None, comment, None)
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
        .bind(&id)
        .bind(&name)
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
        key_ids.push(id);
    }

    // 5 handcrafted + 20 bulk = 25
    let roles = [
        "web", "api", "db", "cache", "worker", "queue", "metrics", "log", "bastion", "ci",
        "staging", "canary", "edge", "cdn", "vpn", "mail", "dns", "backup", "mirror", "lab",
    ];
    for i in 1..=20 {
        let id = format!("demo-ssh-bulk-{i:02}");
        let role = roles[(i as usize - 1) % roles.len()];
        let name = format!("{role}-{i:02}");
        let host = format!("{role}{i}.example.com");
        let auth = if i % 3 == 0 { "password" } else { "key" };
        let key_id = if auth == "key" {
            Some(key_ids[(i as usize - 1) % key_ids.len()].clone())
        } else {
            None
        };
        let password = if auth == "password" {
            Some(format!("demo-ssh-pass-{i}"))
        } else {
            None
        };
        let ws = WS[(i as usize - 1) % WS.len()];
        let totp = if i % 5 == 0 {
            Some(format!("demo-totp-bulk-{:02}", ((i - 1) % 48) + 1))
        } else {
            None
        };

        sqlx::query(
            "INSERT INTO ssh_connections (
                id, name, host, port, username, auth_type,
                password, private_key, key_passphrase, ssh_key_id,
                requires_2fa, totp_entry_id, proxy_id,
                connect_timeout_sec, keepalive_sec, terminal_theme,
                default_cols, default_rows, created_at, updated_at
            ) VALUES (
                ?, ?, ?, 22, 'deploy', ?,
                ?, NULL, NULL, ?,
                ?, ?, NULL,
                15, 30, NULL,
                120, 32, ?, ?
            )",
        )
        .bind(&id)
        .bind(&name)
        .bind(&host)
        .bind(auth)
        .bind(&password)
        .bind(&key_id)
        .bind(if totp.is_some() { 1_i64 } else { 0 })
        .bind(&totp)
        .bind(now)
        .bind(now)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;

        sqlx::query(
            "INSERT OR IGNORE INTO ssh_connection_workspaces (connection_id, workspace_id) VALUES (?, ?)",
        )
        .bind(&id)
        .bind(ws)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;
    }
    Ok(())
}
