// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Sync settings in `app_settings` and the storage adapter they describe.

use crate::Registry;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use veydan_core::{settings, AppError, CmdResult};
use veydan_sync::{
    LargeFileConfig, LocalDir, S3Config, S3Storage, Storage, WebDavConfig, WebDavStorage,
};

pub const DEFAULT_INTERVAL_SEC: u64 = 60;
const MIN_INTERVAL_SEC: u64 = 1;
const MAX_INTERVAL_SEC: u64 = 86400;
const MIB: u64 = 1024 * 1024;

/// Large-file transfer settings, device-local. Says nothing about when to use v2.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LargeFileSettings {
    /// Chunk size for new files, 1..=64.
    pub chunk_mib: u32,
    /// Chunks in flight, 1..=8.
    pub parallelism: u32,
    /// Continue interrupted downloads from the staged prefix.
    pub resume: bool,
}

impl Default for LargeFileSettings {
    fn default() -> Self {
        let d = LargeFileConfig::default();
        Self {
            chunk_mib: (d.chunk_size / MIB) as u32,
            parallelism: d.parallelism as u32,
            resume: d.resume,
        }
    }
}

impl LargeFileSettings {
    /// Rust is the source of truth: reject out-of-range values instead of clamping silently.
    pub fn to_config(&self) -> CmdResult<LargeFileConfig> {
        let cfg = LargeFileConfig {
            chunk_size: self.chunk_mib as u64 * MIB,
            parallelism: self.parallelism as usize,
            resume: self.resume,
        };
        cfg.validate().map_err(AppError::other)?;
        Ok(cfg)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct S3Settings {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub access_key: String,
    /// Never sent to the UI. On save: `None` keeps the stored key, `Some("")` clears it.
    #[serde(skip_serializing, default)]
    pub secret_key: Option<String>,
    #[serde(default)]
    pub has_secret_key: bool,
    pub path_style: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebDavSettings {
    pub url: String,
    pub username: String,
    /// Same three-state rule as `S3Settings::secret_key`.
    #[serde(skip_serializing, default)]
    pub password: Option<String>,
    #[serde(default)]
    pub has_password: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    pub enabled: bool,
    /// "folder" | "s3" | "webdav"
    pub backend: String,
    pub folder_path: String,
    pub s3: S3Settings,
    pub webdav: WebDavSettings,
    pub interval_sec: u64,
    /// Replicate `firefox-profile/` directories (cookies, sessions, history).
    #[serde(default = "default_true")]
    pub profile_files: bool,
    /// How this device is shown to others (lease badge).
    #[serde(default)]
    pub device_name: String,
    #[serde(default)]
    pub large_files: LargeFileSettings,
}

fn default_true() -> bool {
    true
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            // Mobile has no shared folder and no Firefox profiles to replicate.
            backend: if cfg!(mobile) { "s3" } else { "folder" }.into(),
            folder_path: String::new(),
            s3: S3Settings::default(),
            webdav: WebDavSettings::default(),
            interval_sec: DEFAULT_INTERVAL_SEC,
            profile_files: cfg!(desktop),
            device_name: String::new(),
            large_files: LargeFileSettings::default(),
        }
    }
}

/// The name the OS gives the device, or nothing usable (the caller falls
/// back to the device id). A phone has no hostname of its own — Android
/// answers `localhost` on every device —, so there the model is the name.
fn default_device_name() -> String {
    let host = usable_hostname(&gethostname::gethostname().to_string_lossy());
    #[cfg(target_os = "android")]
    let host = if host.is_empty() {
        android_model()
    } else {
        host
    };
    host
}

/// A hostname that tells devices apart: `localhost` does not.
fn usable_hostname(raw: &str) -> String {
    let name = raw.trim();
    if name.eq_ignore_ascii_case("localhost") || name.eq_ignore_ascii_case("localhost.localdomain")
    {
        return String::new();
    }
    name.to_string()
}

/// `Build.MODEL` as the system property an app may read, e.g. `Pixel 8`.
#[cfg(target_os = "android")]
fn android_model() -> String {
    std::process::Command::new("getprop")
        .arg("ro.product.model")
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|model| !model.is_empty())
        .unwrap_or_else(|| "Android".to_string())
}

/// Identity of the vault this device joined. Absent until create/join.
#[derive(Debug, Clone)]
pub struct VaultBinding {
    pub vault_id: String,
    pub vmk_b64: String,
}

async fn get_or_empty(db: &Pool<Sqlite>, key: &str) -> String {
    settings::get(db, key).await.unwrap_or_default()
}

pub async fn load_config(db: &Pool<Sqlite>) -> SyncConfig {
    let d = SyncConfig::default();
    let s3_secret = get_or_empty(db, "sync_s3_secret_key").await;
    let webdav_password = get_or_empty(db, "sync_webdav_password").await;
    SyncConfig {
        enabled: settings::get(db, "sync_enabled").await.as_deref() == Some("1"),
        backend: settings::get(db, "sync_backend").await.unwrap_or(d.backend),
        folder_path: get_or_empty(db, "sync_folder_path").await,
        s3: S3Settings {
            endpoint: get_or_empty(db, "sync_s3_endpoint").await,
            region: get_or_empty(db, "sync_s3_region").await,
            bucket: get_or_empty(db, "sync_s3_bucket").await,
            prefix: get_or_empty(db, "sync_s3_prefix").await,
            access_key: get_or_empty(db, "sync_s3_access_key").await,
            has_secret_key: !s3_secret.is_empty(),
            secret_key: Some(s3_secret),
            path_style: settings::get(db, "sync_s3_path_style").await.as_deref() == Some("1"),
        },
        webdav: WebDavSettings {
            url: get_or_empty(db, "sync_webdav_url").await,
            username: get_or_empty(db, "sync_webdav_username").await,
            has_password: !webdav_password.is_empty(),
            password: Some(webdav_password),
        },
        interval_sec: settings::get(db, "sync_interval_sec")
            .await
            .and_then(|v| v.parse().ok())
            .unwrap_or(d.interval_sec),
        profile_files: cfg!(desktop)
            && settings::get(db, "sync_profile_files").await.as_deref() != Some("0"),
        device_name: device_name(db).await,
        large_files: LargeFileSettings {
            chunk_mib: settings::get(db, "sync_lf_chunk_mib")
                .await
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.large_files.chunk_mib),
            parallelism: settings::get(db, "sync_lf_parallelism")
                .await
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.large_files.parallelism),
            resume: settings::get(db, "sync_lf_resume").await.as_deref() != Some("0"),
        },
    }
}

/// Name shown to other devices; falls back to the hostname, then the device id.
pub async fn device_name(db: &Pool<Sqlite>) -> String {
    if let Some(n) = settings::get(db, "sync_device_name")
        .await
        .filter(|s| !s.trim().is_empty())
    {
        return n;
    }
    let host = default_device_name();
    if !host.is_empty() {
        return host;
    }
    device_id(db).await.unwrap_or_default()
}

pub async fn save_config(db: &Pool<Sqlite>, cfg: &SyncConfig) -> CmdResult<()> {
    cfg.large_files.to_config()?;
    settings::set(db, "sync_enabled", if cfg.enabled { "1" } else { "0" }).await?;
    settings::set(db, "sync_backend", &cfg.backend).await?;
    settings::set(db, "sync_folder_path", &cfg.folder_path).await?;
    settings::set(db, "sync_s3_endpoint", &cfg.s3.endpoint).await?;
    settings::set(db, "sync_s3_region", &cfg.s3.region).await?;
    settings::set(db, "sync_s3_bucket", &cfg.s3.bucket).await?;
    settings::set(db, "sync_s3_prefix", &cfg.s3.prefix).await?;
    settings::set(db, "sync_s3_access_key", &cfg.s3.access_key).await?;
    if let Some(v) = cfg.s3.secret_key.as_deref() {
        settings::set(db, "sync_s3_secret_key", v).await?;
    }
    settings::set(
        db,
        "sync_s3_path_style",
        if cfg.s3.path_style { "1" } else { "0" },
    )
    .await?;
    settings::set(db, "sync_webdav_url", &cfg.webdav.url).await?;
    settings::set(db, "sync_webdav_username", &cfg.webdav.username).await?;
    if let Some(v) = cfg.webdav.password.as_deref() {
        settings::set(db, "sync_webdav_password", v).await?;
    }
    let interval = cfg.interval_sec.clamp(MIN_INTERVAL_SEC, MAX_INTERVAL_SEC);
    settings::set(db, "sync_interval_sec", &interval.to_string()).await?;
    settings::set(
        db,
        "sync_profile_files",
        if cfg.profile_files { "1" } else { "0" },
    )
    .await?;
    settings::set(db, "sync_device_name", cfg.device_name.trim()).await?;
    settings::set(
        db,
        "sync_lf_chunk_mib",
        &cfg.large_files.chunk_mib.to_string(),
    )
    .await?;
    settings::set(
        db,
        "sync_lf_parallelism",
        &cfg.large_files.parallelism.to_string(),
    )
    .await?;
    settings::set(
        db,
        "sync_lf_resume",
        if cfg.large_files.resume { "1" } else { "0" },
    )
    .await?;
    Ok(())
}

/// Stable per-install device id, created on first use.
pub async fn device_id(db: &Pool<Sqlite>) -> CmdResult<String> {
    if let Some(id) = settings::get(db, "sync_device_id")
        .await
        .filter(|s| !s.is_empty())
    {
        return Ok(id);
    }
    let id = veydan_sync::random_hex(8);
    settings::set(db, "sync_device_id", &id).await?;
    Ok(id)
}

pub async fn load_binding(db: &Pool<Sqlite>) -> Option<VaultBinding> {
    let vault_id = settings::get(db, "sync_vault_id")
        .await
        .filter(|s| !s.is_empty())?;
    let vmk_b64 = settings::get(db, "sync_vault_key")
        .await
        .filter(|s| !s.is_empty())?;
    Some(VaultBinding { vault_id, vmk_b64 })
}

pub async fn save_binding(db: &Pool<Sqlite>, b: &VaultBinding) -> CmdResult<()> {
    settings::set(db, "sync_vault_id", &b.vault_id).await?;
    settings::set(db, "sync_vault_key", &b.vmk_b64).await
}

/// Start a fresh device log: new id, empty own chain. Peer heads, entity
/// states and the HLC stay so remote ops are still applied idempotently.
pub async fn rotate_device_id(db: &Pool<Sqlite>) -> CmdResult<()> {
    for key in ["sync_device_id", "sync_own_seq", "sync_own_head"] {
        settings::delete(db, key).await?;
    }
    Ok(())
}

/// Detect a database that was restored or copied from another install and
/// rotate the device id so two installs never write to the same log.
/// `data_dir/install.id` is outside the backup set; `sync_install_id` travels with the DB.
pub async fn check_install_marker(db: &Pool<Sqlite>, data_dir: &std::path::Path) -> CmdResult<()> {
    let marker_path = data_dir.join("install.id");
    let marker = match std::fs::read_to_string(&marker_path) {
        Ok(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => {
            let id = veydan_sync::random_hex(16);
            std::fs::write(&marker_path, &id).map_err(AppError::io)?;
            id
        }
    };
    match settings::get(db, "sync_install_id").await {
        Some(stored) if stored == marker => Ok(()),
        Some(_) => {
            rotate_device_id(db).await?;
            settings::set(db, "sync_install_id", &marker).await
        }
        None => settings::set(db, "sync_install_id", &marker).await,
    }
}

/// Set by a join that did not read the vault, which is one of a device with
/// no password vault key row. While it is set the device publishes no key row
/// it has made since: the vault may hold one, and the newer row would take its
/// place on every device. `join::settle_join_key` ends the wait.
pub const JOIN_PENDING: &str = "sync_join_pending";

/// Forget the binding and what was learnt under it, the ops the tables of
/// `registry` hold for rows that are not here among it.
pub async fn clear_binding(db: &Pool<Sqlite>, registry: &Registry) -> CmdResult<()> {
    let keys = [
        "sync_vault_id",
        "sync_vault_key",
        "sync_device_id",
        "sync_own_seq",
        "sync_own_head",
        // `sync_hlc` is kept: a clock reset would break LWW against existing ops.
        "sync_last_run",
        "sync_last_started",
        "sync_last_error",
        "sync_last_warning",
        "sync_last_applied",
        "sync_devices",
        "sync_gc_last",
        "sync_gc_blobs_total",
        "sync_gc_removed",
        "sync_gc_lf_total",
        "sync_gc_lf_removed",
        JOIN_PENDING,
        crate::state::REREAD_KEY,
        crate::state::REREAD_PEERS_KEY,
    ];
    for key in keys.into_iter().chain(registry.hold_keys()) {
        settings::delete(db, key).await?;
    }
    Ok(())
}

/// Storage adapter for the configured backend.
pub fn build_storage(cfg: &SyncConfig) -> CmdResult<Box<dyn Storage>> {
    match cfg.backend.as_str() {
        "folder" => {
            if cfg.folder_path.trim().is_empty() {
                return Err(AppError::other("sync folder is not set"));
            }
            Ok(Box::new(LocalDir::new(cfg.folder_path.trim())))
        }
        "s3" => {
            let s = &cfg.s3;
            let secret_key = s.secret_key.clone().unwrap_or_default();
            if s.endpoint.is_empty()
                || s.bucket.is_empty()
                || s.access_key.is_empty()
                || secret_key.is_empty()
            {
                return Err(AppError::other("S3 endpoint, bucket and keys are required"));
            }
            let storage = S3Storage::new(S3Config {
                endpoint: s.endpoint.clone(),
                region: if s.region.is_empty() {
                    "us-east-1".into()
                } else {
                    s.region.clone()
                },
                bucket: s.bucket.clone(),
                prefix: s.prefix.clone(),
                access_key: s.access_key.clone(),
                secret_key,
                path_style: s.path_style,
            })
            .map_err(AppError::other)?;
            Ok(Box::new(storage))
        }
        "webdav" => {
            let w = &cfg.webdav;
            if w.url.is_empty() {
                return Err(AppError::other("WebDAV URL is required"));
            }
            let storage = WebDavStorage::new(WebDavConfig {
                url: w.url.clone(),
                username: w.username.clone(),
                password: w.password.clone().unwrap_or_default(),
            })
            .map_err(AppError::other)?;
            Ok(Box::new(storage))
        }
        other => Err(AppError::other(format!("unknown sync backend {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::usable_hostname;

    #[test]
    fn localhost_is_not_a_device_name() {
        assert_eq!(usable_hostname(" workstation \n"), "workstation");
        assert_eq!(usable_hostname("localhost"), "");
        assert_eq!(usable_hostname("Localhost.localdomain"), "");
        assert_eq!(usable_hostname(""), "");
    }
}
