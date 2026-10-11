// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Sync settings in `app_settings` and the storage adapter they describe.

use crate::Registry;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use std::sync::{Arc, RwLock};
use veydan_core::{settings, AppError, BoxFuture, CmdResult};
use veydan_sync::storage_cloud::VaultTransport;
use veydan_sync::{
    CloudStorage, LargeFileConfig, LocalDir, RequestSigner, S3Config, S3Storage, Storage,
    WebDavConfig, WebDavStorage,
};
use zeroize::Zeroizing;

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
    /// The hosts of the backend `cloud` (`sync_cloud_hosts`): written by the
    /// module `cloud` from `storage[]` of the manifest, never by the form.
    #[serde(skip)]
    pub cloud_hosts: Vec<String>,
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
            cloud_hosts: Vec::new(),
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
        cloud_hosts: load_cloud_hosts(db).await,
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
    // Where the vault lives may have changed: the escrow carries it.
    binding_changed();
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
    settings::set(db, "sync_vault_key", &b.vmk_b64).await?;
    binding_changed();
    Ok(())
}

/// Who hears that the binding of this device to a vault was saved or
/// cleared, or that the settings of sync — where the vault lives — were
/// saved: the module `cloud` writes its escrow again (cloud-recovery.md
/// §4). The listener returns at once; the work is its own.
pub type BindingListener = Arc<dyn Fn() + Send + Sync>;

fn binding_slot() -> &'static RwLock<Vec<BindingListener>> {
    static SLOT: RwLock<Vec<BindingListener>> = RwLock::new(Vec::new());
    &SLOT
}

/// Called in the setup of a module; every listener hears every change.
pub fn on_binding_changed(listener: BindingListener) {
    binding_slot()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .push(listener);
}

fn binding_changed() {
    let listeners = binding_slot()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    for listener in listeners {
        listener();
    }
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
    binding_changed();
    Ok(())
}

// ── The backend `cloud` (internal/vcloud-plan.md 4.8, spec/vstore.md §14) ──

/// The value of `sync_backend` for Veydan Cloud.
pub const CLOUD_BACKEND: &str = "cloud";
/// The `public_url`s of `storage[]`, a JSON array in the order to try them.
pub const CLOUD_HOSTS_KEY: &str = "sync_cloud_hosts";

/// What the backend `cloud` takes from the module `cloud`: the signature of
/// the identity and the passphrase of the vault derived from it. The module
/// registers it with [`register_sync_cloud_credentials`]; sync knows neither
/// the messenger nor the recovery crate.
pub trait CloudCredentials: Send + Sync {
    /// Signs the requests of the storage with the identity (NIP-98).
    fn signer(&self) -> Arc<dyn RequestSigner>;
    /// Writes the hosts of `storage[]` of the manifest in use to
    /// [`CLOUD_HOSTS_KEY`]; none when the servers are not the project's.
    fn refresh_hosts(&self) -> BoxFuture<'_, CmdResult<()>>;
    /// The passphrase of the vault: base64 of HKDF of the nsec (plan 4.8),
    /// the same on every device of the identity.
    fn passphrase(&self) -> BoxFuture<'_, CmdResult<Zeroizing<String>>>;
    /// The way of the requests to the hosts (the messenger's, with its
    /// bridges); none sends them directly.
    fn transport(&self) -> Option<Arc<dyn VaultTransport>> {
        None
    }
    /// Why no host is known, in the module's words, after a refresh that
    /// found none (own servers, no `storage[]` for the region).
    fn why_no_host(&self) -> Option<String> {
        None
    }
}

/// The credentials of the process: one app, one module `cloud`.
fn cloud_slot() -> &'static RwLock<Option<Arc<dyn CloudCredentials>>> {
    static SLOT: RwLock<Option<Arc<dyn CloudCredentials>>> = RwLock::new(None);
    &SLOT
}

/// Called by the setup of the module `cloud`; a later call replaces it.
pub fn register_sync_cloud_credentials(credentials: Arc<dyn CloudCredentials>) {
    *cloud_slot().write().unwrap_or_else(|e| e.into_inner()) = Some(credentials);
}

/// The credentials of the backend `cloud`, when the product has the module.
pub fn sync_cloud_credentials() -> Option<Arc<dyn CloudCredentials>> {
    cloud_slot()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

fn no_cloud() -> AppError {
    AppError::other("Veydan Cloud is not available in this app")
}

/// The hosts the module `cloud` wrote; a value that does not read is none.
pub async fn load_cloud_hosts(db: &Pool<Sqlite>) -> Vec<String> {
    settings::get(db, CLOUD_HOSTS_KEY)
        .await
        .and_then(|v| serde_json::from_str::<Vec<String>>(&v).ok())
        .unwrap_or_default()
}

/// Keeps the hosts of `storage[]`; returns whether they changed.
pub async fn save_cloud_hosts(db: &Pool<Sqlite>, hosts: &[String]) -> CmdResult<bool> {
    if load_cloud_hosts(db).await == hosts {
        return Ok(false);
    }
    let json = serde_json::to_string(hosts).map_err(AppError::other)?;
    settings::set(db, CLOUD_HOSTS_KEY, &json).await?;
    Ok(true)
}

/// The passphrase a create or a join uses: the one the user typed, or, on
/// the backend `cloud`, the one derived from the identity (whatever was
/// typed). The hosts are refreshed first, so the storage built after it
/// goes where the manifest says.
pub async fn vault_passphrase(db: &Pool<Sqlite>, typed: String) -> CmdResult<Zeroizing<String>> {
    if !on_cloud(db).await {
        return Ok(Zeroizing::new(typed));
    }
    let credentials = sync_cloud_credentials().ok_or_else(no_cloud)?;
    credentials.refresh_hosts().await?;
    credentials.passphrase().await
}

/// Whether the configured backend is `cloud`.
pub async fn on_cloud(db: &Pool<Sqlite>) -> bool {
    settings::get(db, "sync_backend").await.as_deref() == Some(CLOUD_BACKEND)
}

/// On the backend `cloud`, the hosts of the manifest in use before the
/// storage is built (a probe); nothing on another backend.
pub async fn refresh_cloud_hosts(db: &Pool<Sqlite>) -> CmdResult<()> {
    if !on_cloud(db).await {
        return Ok(());
    }
    sync_cloud_credentials()
        .ok_or_else(no_cloud)?
        .refresh_hosts()
        .await
}

/// A config as the form sends it, made whole. The form carries no secret it
/// did not change and never the hosts of the backend `cloud`: both are what
/// is stored, the hosts refreshed first as before a probe. For a storage
/// that is read before its config is saved (`join::change_storage`).
pub async fn completed(db: &Pool<Sqlite>, mut cfg: SyncConfig) -> CmdResult<SyncConfig> {
    let stored = load_config(db).await;
    if cfg.s3.secret_key.is_none() {
        cfg.s3.secret_key = stored.s3.secret_key;
    }
    if cfg.webdav.password.is_none() {
        cfg.webdav.password = stored.webdav.password;
    }
    if cfg.backend == CLOUD_BACKEND {
        sync_cloud_credentials()
            .ok_or_else(no_cloud)?
            .refresh_hosts()
            .await?;
    }
    cfg.cloud_hosts = load_cloud_hosts(db).await;
    Ok(cfg)
}

/// The bytes of the browser profiles' files on this device: what turning
/// on `profile_files` would send at most. An upper bound: the caches the
/// snapshot leaves out are counted too.
pub fn profile_files_estimate(data_dir: &std::path::Path) -> u64 {
    fn walk(dir: &std::path::Path) -> u64 {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return 0;
        };
        entries
            .flatten()
            .map(|e| match e.file_type() {
                Ok(t) if t.is_dir() => walk(&e.path()),
                Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
                _ => 0,
            })
            .sum()
    }
    let Ok(profiles) = std::fs::read_dir(data_dir.join("profiles")) else {
        return 0;
    };
    profiles
        .flatten()
        .map(|p| walk(&p.path().join("firefox-profile")))
        .sum()
}

/// Storage adapter for the configured backend.
pub fn build_storage(cfg: &SyncConfig) -> CmdResult<Box<dyn Storage>> {
    match cfg.backend.as_str() {
        CLOUD_BACKEND => {
            let credentials = sync_cloud_credentials().ok_or_else(no_cloud)?;
            if cfg.cloud_hosts.is_empty() {
                return Err(AppError::other(credentials.why_no_host().unwrap_or_else(
                    || "no host of Veydan Cloud is known on this device yet".into(),
                )));
            }
            let mut storage = CloudStorage::new(&cfg.cloud_hosts, credentials.signer())
                .map_err(AppError::other)?;
            if let Some(transport) = credentials.transport() {
                storage = storage.with_transport(transport);
            }
            Ok(Box::new(storage))
        }
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

#[cfg(test)]
mod cloud_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use veydan_sync::SignFuture;

    struct Signs;

    impl RequestSigner for Signs {
        fn authorization<'a>(
            &'a self,
            _: &'a str,
            _: &'a str,
            _: Option<[u8; 32]>,
            _: u64,
        ) -> SignFuture<'a> {
            Box::pin(async { Ok("Nostr e30=".to_string()) })
        }
    }

    /// The module `cloud` as the tests see it: hosts to write, a passphrase.
    struct Fake {
        db: Pool<Sqlite>,
        refreshed: AtomicUsize,
        why: RwLock<Option<String>>,
    }

    impl CloudCredentials for Fake {
        fn signer(&self) -> Arc<dyn RequestSigner> {
            Arc::new(Signs)
        }
        fn refresh_hosts(&self) -> BoxFuture<'_, CmdResult<()>> {
            Box::pin(async move {
                self.refreshed.fetch_add(1, Ordering::Relaxed);
                let hosts = ["https://media-eu.example.com/vault".to_string()];
                save_cloud_hosts(&self.db, &hosts).await.map(|_| ())
            })
        }
        fn passphrase(&self) -> BoxFuture<'_, CmdResult<Zeroizing<String>>> {
            Box::pin(async { Ok(Zeroizing::new("derived-from-the-identity".to_string())) })
        }
        fn why_no_host(&self) -> Option<String> {
            self.why.read().ok()?.clone()
        }
    }

    async fn db(dir: &tempfile::TempDir) -> Pool<Sqlite> {
        veydan_core::db::open(&dir.path().join("app.db"), &[veydan_core::SCHEMA])
            .await
            .expect("db")
    }

    /// One test owns the slot of the process: the credentials are global.
    #[tokio::test]
    async fn the_backend_cloud_takes_its_hosts_and_passphrase_from_the_module() {
        let dir = tempfile::tempdir().unwrap();
        let db = db(&dir).await;
        let mut cfg = SyncConfig {
            backend: CLOUD_BACKEND.into(),
            ..SyncConfig::default()
        };
        // Before the module registers: the product has no Veydan Cloud.
        assert!(sync_cloud_credentials().is_none());
        let err = build_storage(&cfg)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(err.contains("not available"), "{err}");
        settings::set(&db, "sync_backend", CLOUD_BACKEND)
            .await
            .unwrap();
        assert!(vault_passphrase(&db, "typed".into()).await.is_err());

        let fake = Arc::new(Fake {
            db: db.clone(),
            refreshed: AtomicUsize::new(0),
            why: RwLock::new(None),
        });
        register_sync_cloud_credentials(fake.clone());
        // Registered, but no host known yet.
        let err = build_storage(&cfg)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(err.contains("no host"), "{err}");
        // The module knows why: its words, not the generic ones.
        *fake.why.write().unwrap() = Some("no_host: not with your own servers".into());
        let err = build_storage(&cfg)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(err.contains("own servers"), "{err}");
        *fake.why.write().unwrap() = None;

        // A create or a join on the backend cloud: the hosts first, then the
        // derived passphrase, whatever was typed.
        let pass = vault_passphrase(&db, "typed".into()).await.unwrap();
        assert_eq!(pass.as_str(), "derived-from-the-identity");
        assert_eq!(fake.refreshed.load(Ordering::Relaxed), 1);
        cfg = load_config(&db).await;
        assert_eq!(cfg.backend, CLOUD_BACKEND);
        assert_eq!(cfg.cloud_hosts, ["https://media-eu.example.com/vault"]);
        assert!(build_storage(&cfg).is_ok());

        // The form never writes the hosts, and does not erase them.
        save_config(&db, &cfg).await.unwrap();
        let json = serde_json::to_value(&cfg).unwrap();
        assert!(json.get("cloud_hosts").is_none());
        assert_eq!(load_cloud_hosts(&db).await.len(), 1);
        assert!(!save_cloud_hosts(&db, &cfg.cloud_hosts).await.unwrap());
        assert!(save_cloud_hosts(&db, &[]).await.unwrap());
        assert!(load_config(&db).await.cloud_hosts.is_empty());

        // Another backend keeps the passphrase the user typed.
        settings::set(&db, "sync_backend", "folder").await.unwrap();
        let pass = vault_passphrase(&db, "typed by the user".into())
            .await
            .unwrap();
        assert_eq!(pass.as_str(), "typed by the user");
        assert_eq!(fake.refreshed.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn the_estimate_of_profile_files_counts_their_folders_only() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(profile_files_estimate(dir.path()), 0);
        let p1 = dir.path().join("profiles/p1/firefox-profile/storage");
        std::fs::create_dir_all(&p1).unwrap();
        std::fs::write(p1.join("a.sqlite"), vec![0u8; 1000]).unwrap();
        std::fs::write(
            dir.path().join("profiles/p1/firefox-profile/prefs.js"),
            b"12345",
        )
        .unwrap();
        std::fs::write(dir.path().join("profiles/p1/meta.json"), b"not the profile").unwrap();
        std::fs::create_dir_all(dir.path().join("profiles/p2/firefox-profile")).unwrap();
        std::fs::write(dir.path().join("profiles/p2/firefox-profile/x"), b"12").unwrap();
        std::fs::write(dir.path().join("app.db-other"), b"elsewhere").unwrap();
        assert_eq!(profile_files_estimate(dir.path()), 1007);
    }
}
