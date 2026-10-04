// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, Serializer};

fn serialize_tags_as_vec<S>(tags: &str, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let vec: Vec<String> = serde_json::from_str(tags).unwrap_or_default();
    vec.serialize(s)
}

/// Serialize a secret as `true`/`false` (present or not) instead of its value.
fn serialize_presence<S>(secret: &Option<String>, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    secret
        .as_deref()
        .is_some_and(|v| !v.is_empty())
        .serialize(s)
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WorkspaceColumn {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
    pub tag_name: String,
    pub color: String,
    pub position: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateWorkspaceColumnRequest {
    pub name: String,
    pub tag_name: String,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateWorkspaceColumnRequest {
    pub name: Option<String>,
    pub color: Option<String>,
    pub position: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub color: String,
    pub icon: String,
    pub notes: Option<String>,
    pub is_default: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateWorkspaceRequest {
    pub name: String,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateWorkspaceRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceStats {
    pub id: String,
    pub profile_count: i64,
    pub proxy_count: i64,
    pub active_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub status: String,
    pub profile_path: String,
    pub browser_type: String,
    pub proxy_id: Option<String>,
    pub fingerprint_preset: String,
    pub user_agent: Option<String>,
    pub platform: Option<String>,
    pub timezone: Option<String>,
    pub locale: String,
    pub languages: String,
    pub screen_width: i64,
    pub screen_height: i64,
    pub webrtc_mode: String,
    pub geolocation_enabled: bool,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub webgl_vendor: Option<String>,
    pub webgl_renderer: Option<String>,
    pub notes: Option<String>,
    pub workspace_id: Option<String>,
    pub kanban_status: String,
    pub kanban_order: i64,
    #[serde(serialize_with = "serialize_tags_as_vec")]
    pub tags: String,
    pub default_search_engine: String,
    pub history_enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_launch_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProfileRequest {
    pub name: String,
    pub workspace_id: Option<String>,
    pub browser_type: Option<String>,
    pub proxy_id: Option<String>,
    pub fingerprint_preset: Option<String>,
    pub user_agent: Option<String>,
    pub platform: Option<String>,
    pub timezone: Option<String>,
    pub locale: Option<String>,
    pub languages: Option<String>,
    pub screen_width: Option<i64>,
    pub screen_height: Option<i64>,
    pub webrtc_mode: Option<String>,
    pub geolocation_enabled: Option<bool>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub webgl_vendor: Option<String>,
    pub webgl_renderer: Option<String>,
    pub notes: Option<String>,
    pub default_search_engine: Option<String>,
    pub history_enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateProfileRequest {
    pub name: Option<String>,
    pub browser_type: Option<String>,
    pub proxy_id: Option<String>,
    pub fingerprint_preset: Option<String>,
    pub user_agent: Option<String>,
    pub platform: Option<String>,
    pub timezone: Option<String>,
    pub locale: Option<String>,
    pub languages: Option<String>,
    pub screen_width: Option<i64>,
    pub screen_height: Option<i64>,
    pub webrtc_mode: Option<String>,
    pub geolocation_enabled: Option<bool>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub webgl_vendor: Option<String>,
    pub webgl_renderer: Option<String>,
    pub notes: Option<String>,
    pub default_search_engine: Option<String>,
    pub history_enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Proxy {
    pub id: String,
    pub name: String,
    pub proxy_type: String,
    pub host: String,
    pub port: i64,
    pub username: Option<String>,
    // Secrets reach the UI only as presence flags.
    #[serde(rename = "has_password", serialize_with = "serialize_presence")]
    pub password: Option<String>,
    pub country: Option<String>,
    pub city: Option<String>,
    pub status: String,
    pub last_ip: Option<String>,
    pub last_check_at: Option<DateTime<Utc>>,
    #[serde(rename = "has_private_key", serialize_with = "serialize_presence")]
    pub private_key: Option<String>,
    /// SHA256 fingerprint of the SSH server's host key, saved on first successful connection (TOFU).
    /// None = never connected. On mismatch — connection is blocked.
    pub server_fingerprint: Option<String>,
    #[serde(serialize_with = "serialize_tags_as_vec")]
    pub tags: String,
    pub created_at: DateTime<Utc>,
}

/// A single directory entry, unified across the local filesystem (`commands::fs`)
/// and remote SFTP listings (`commands::sftp`). Remote paths are POSIX strings.
#[derive(Debug, Clone, Serialize)]
pub struct FileEntry {
    pub name: String,
    /// Absolute path of the entry (POSIX string for remote entries).
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    /// Modification time, epoch milliseconds.
    pub mtime: Option<i64>,
    /// Raw unix mode bits (file type + permissions).
    pub mode: u32,
    /// Symbolic permission string, e.g. "rwxr-xr-x".
    pub permissions: String,
    /// Octal permission string, e.g. "0644".
    pub octal: String,
    pub owner: Option<String>,
    pub group: Option<String>,
}

/// "rwxr-xr-x"-style string from raw mode bits (setuid/setgid/sticky included).
pub fn format_permissions(mode: u32) -> String {
    let mut s = String::with_capacity(9);
    let flags = [
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o040, 'r'),
        (0o020, 'w'),
        (0o010, 'x'),
        (0o004, 'r'),
        (0o002, 'w'),
        (0o001, 'x'),
    ];
    for (bit, ch) in flags {
        s.push(if mode & bit != 0 { ch } else { '-' });
    }
    // setuid / setgid / sticky replace the corresponding execute slot
    let mut b: Vec<char> = s.chars().collect();
    if mode & 0o4000 != 0 {
        b[2] = if mode & 0o100 != 0 { 's' } else { 'S' };
    }
    if mode & 0o2000 != 0 {
        b[5] = if mode & 0o010 != 0 { 's' } else { 'S' };
    }
    if mode & 0o1000 != 0 {
        b[8] = if mode & 0o001 != 0 { 't' } else { 'T' };
    }
    b.into_iter().collect()
}

/// "0644"-style octal string from raw mode bits (permission bits only).
pub fn format_octal(mode: u32) -> String {
    format!("{:04o}", mode & 0o7777)
}

#[cfg(test)]
impl Profile {
    /// Baseline profile for unit tests — clone and override fields as needed.
    pub fn test_default() -> Self {
        Self {
            id: "test-profile-id".into(),
            name: "Test".into(),
            status: "stopped".into(),
            profile_path: "/tmp/test-profile".into(),
            browser_type: "camoufox".into(),
            proxy_id: None,
            fingerprint_preset: "linux".into(),
            user_agent: None,
            platform: None,
            timezone: None,
            locale: "en-US".into(),
            languages: "en-US,en".into(),
            screen_width: 1920,
            screen_height: 1080,
            webrtc_mode: "disable".into(),
            geolocation_enabled: false,
            latitude: None,
            longitude: None,
            webgl_vendor: None,
            webgl_renderer: None,
            notes: None,
            workspace_id: None,
            kanban_status: "new".into(),
            kanban_order: 0,
            tags: "[]".into(),
            default_search_engine: "ddg".into(),
            history_enabled: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            last_launch_at: None,
        }
    }
}

#[cfg(test)]
impl Proxy {
    /// Baseline proxy for unit tests — clone and override fields as needed.
    pub fn test_default() -> Self {
        Self {
            id: "test-proxy-id".into(),
            name: "Test proxy".into(),
            proxy_type: "socks5".into(),
            host: "127.0.0.1".into(),
            port: 1080,
            username: None,
            password: None,
            country: None,
            city: None,
            status: "unknown".into(),
            last_ip: None,
            last_check_at: None,
            private_key: None,
            server_fingerprint: None,
            tags: "[]".into(),
            created_at: chrono::Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProxyRequest {
    pub name: String,
    pub proxy_type: String,
    pub host: String,
    pub port: i64,
    pub tags: Option<Vec<String>>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub country: Option<String>,
    pub city: Option<String>,
    pub private_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BulkProxyItem {
    pub line_number: i64,
    pub proxy_type: String,
    pub host: String,
    pub port: i64,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BulkImportRowResult {
    pub line_number: i64,
    /// "imported" | "duplicate" | "error"
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BulkImportResult {
    pub rows: Vec<BulkImportRowResult>,
    pub imported: Vec<Proxy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieEntry {
    pub host: String,
    pub name: String,
    pub value: String,
    pub path: String,
    pub expiry: Option<i64>,
    pub secure: bool,
    pub http_only: bool,
}

#[derive(Debug, Serialize)]
pub struct ProfileRawData {
    pub user_agent: String,
    pub platform: String,
    pub locale: String,
    pub languages: String,
    pub timezone: Option<String>,
    pub screen_width: i64,
    pub screen_height: i64,
    pub webrtc_mode: String,
    pub webgl_vendor: Option<String>,
    pub webgl_renderer: Option<String>,
    pub canvas_seed: u32,
    pub audio_seed: u32,
    pub fonts_seed: u32,
    pub geolocation_enabled: bool,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub camoufox_config: String,
    pub user_js: String,
    pub cookies: Vec<CookieEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyCheckResult {
    pub ip: String,
    pub country: Option<String>,
    pub city: Option<String>,
    pub ok: bool,
    /// For SSH proxies: fingerprint received from server. None for non-SSH.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_fingerprint: Option<String>,
    /// For SSH proxies: true if this was the first connection (fingerprint not yet saved).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_fingerprint_is_new: Option<bool>,
}

// ── Cookie Import ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct CookieImportResult {
    pub count: usize,
    /// Unique domains, up to 20, sorted alphabetically
    pub domains: Vec<String>,
}

/// Cookie in EditThisCookie / antidetect browser export format
#[derive(Debug, Serialize)]
pub struct ExportCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    #[serde(rename = "expirationDate", skip_serializing_if = "Option::is_none")]
    pub expiration_date: Option<f64>,
    #[serde(rename = "hostOnly")]
    pub host_only: bool,
    pub session: bool,
    #[serde(rename = "httpOnly")]
    pub http_only: bool,
    pub secure: bool,
    #[serde(rename = "sameSite")]
    pub same_site: String,
}

/// Cookie format exported by antidetect browsers (EditThisCookie compatible).
/// Supports both `[{...}]` and `{"cookies":[...]}` JSON shapes.
#[derive(Debug, Deserialize)]
pub struct AntidetectCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    #[serde(rename = "expirationDate")]
    pub expiration_date: Option<f64>,
    pub session: Option<bool>,
    #[serde(rename = "httpOnly")]
    pub http_only: Option<bool>,
    pub secure: Option<bool>,
    #[serde(rename = "sameSite")]
    pub same_site: Option<String>,
}

// ── Export / Import ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportOptions {
    pub include_proxy: bool,
    pub include_proxy_password: bool,
    pub include_files: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileExportData {
    pub name: String,
    pub browser_type: String,
    pub fingerprint_preset: String,
    pub user_agent: Option<String>,
    pub platform: Option<String>,
    pub timezone: Option<String>,
    pub locale: String,
    pub languages: String,
    pub screen_width: i64,
    pub screen_height: i64,
    pub webrtc_mode: String,
    pub geolocation_enabled: bool,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub webgl_vendor: Option<String>,
    pub webgl_renderer: Option<String>,
    pub notes: Option<String>,
    pub kanban_status: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyExportData {
    pub name: String,
    pub proxy_type: String,
    pub host: String,
    pub port: i64,
    pub username: Option<String>,
    pub password: Option<String>,
    pub country: Option<String>,
    pub city: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileExport {
    pub version: String,
    pub exported_at: String,
    pub profile: ProfileExportData,
    pub proxy: Option<ProxyExportData>,
}

#[cfg(test)]
mod file_entry_tests {
    use super::{format_octal, format_permissions};

    #[test]
    fn permissions_basic() {
        assert_eq!(format_permissions(0o644), "rw-r--r--");
        assert_eq!(format_permissions(0o755), "rwxr-xr-x");
        assert_eq!(format_permissions(0o000), "---------");
        // file-type bits must not affect the permission string
        assert_eq!(format_permissions(0o100644), "rw-r--r--");
        assert_eq!(format_permissions(0o040755), "rwxr-xr-x");
    }

    #[test]
    fn permissions_special_bits() {
        assert_eq!(format_permissions(0o4755), "rwsr-xr-x");
        assert_eq!(format_permissions(0o4644), "rwSr--r--");
        assert_eq!(format_permissions(0o2755), "rwxr-sr-x");
        assert_eq!(format_permissions(0o1777), "rwxrwxrwt");
        assert_eq!(format_permissions(0o1666), "rw-rw-rwT");
    }

    #[test]
    fn octal_strips_type_bits() {
        assert_eq!(format_octal(0o100644), "0644");
        assert_eq!(format_octal(0o40755), "0755");
        assert_eq!(format_octal(0o4755), "4755");
        assert_eq!(format_octal(0), "0000");
    }
}
