// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Storage trait and the local-directory adapter.

use crate::{Result, SyncError};
use async_trait::async_trait;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// HTTP client for S3 / WebDAV: fail instead of hanging forever.
pub fn http_client() -> Result<reqwest::Client> {
    // HTTP/1.1 + keep-alive: new TLS per file times out on Seafile after a long list.
    let builder = reqwest::Client::builder()
        .user_agent("VeydanSync/1")
        .http1_only()
        .pool_max_idle_per_host(4)
        .pool_idle_timeout(Duration::from_secs(60))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(90))
        .dns_resolver(CachedIpv4Resolver);
    #[cfg(target_os = "android")]
    let builder = builder.use_preconfigured_tls(android_tls_config());
    builder
        .build()
        .map_err(|e| SyncError::Storage(e.to_string()))
}

fn dns_cache() -> &'static Mutex<HashMap<String, Vec<SocketAddr>>> {
    static C: OnceLock<Mutex<HashMap<String, Vec<SocketAddr>>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// IPv4-first lookup with a process cache. Android getaddrinfo often returns
/// NODATA on AAAA and then fails the next call for the same host.
struct CachedIpv4Resolver;

impl Resolve for CachedIpv4Resolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move { resolve_cached(host).await })
    }
}

async fn resolve_cached(
    host: String,
) -> std::result::Result<Addrs, Box<dyn std::error::Error + Send + Sync>> {
    match resolve_once(&host).await {
        Ok(addrs) if !addrs.is_empty() => {
            if let Ok(mut c) = dns_cache().lock() {
                c.insert(host, addrs.clone());
            }
            Ok(Box::new(addrs.into_iter()) as Addrs)
        }
        other => {
            if let Ok(c) = dns_cache().lock() {
                if let Some(cached) = c.get(&host) {
                    if !cached.is_empty() {
                        return Ok(Box::new(cached.clone().into_iter()) as Addrs);
                    }
                }
            }
            other.map(|a| Box::new(a.into_iter()) as Addrs)
        }
    }
}

async fn resolve_once(
    host: &str,
) -> std::result::Result<Vec<SocketAddr>, Box<dyn std::error::Error + Send + Sync>> {
    let host = host.to_string();
    tokio::task::spawn_blocking(move || lookup(&host))
        .await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?
}

fn lookup(
    host: &str,
) -> std::result::Result<Vec<SocketAddr>, Box<dyn std::error::Error + Send + Sync>> {
    match (host, 0u16).to_socket_addrs() {
        Ok(iter) => {
            let addrs: Vec<SocketAddr> = iter.collect();
            let v4: Vec<_> = addrs.iter().copied().filter(|a| a.is_ipv4()).collect();
            if !v4.is_empty() {
                return Ok(v4);
            }
            let extra = lookup_ipv4_only(host);
            if extra.is_empty() {
                Ok(addrs)
            } else {
                Ok(extra)
            }
        }
        Err(e) => {
            let v4 = lookup_ipv4_only(host);
            if v4.is_empty() {
                Err(Box::new(e))
            } else {
                Ok(v4)
            }
        }
    }
}

fn lookup_ipv4_only(host: &str) -> Vec<SocketAddr> {
    #[cfg(unix)]
    {
        gai_af_inet(host).unwrap_or_default()
    }
    #[cfg(not(unix))]
    {
        let _ = host;
        Vec::new()
    }
}

#[cfg(unix)]
fn gai_af_inet(
    host: &str,
) -> std::result::Result<Vec<SocketAddr>, Box<dyn std::error::Error + Send + Sync>> {
    use std::ffi::CString;
    use std::net::{IpAddr, Ipv4Addr};
    use std::ptr;

    let c_host = CString::new(host)?;
    let mut hints = unsafe { std::mem::zeroed::<libc::addrinfo>() };
    hints.ai_family = libc::AF_INET;
    hints.ai_socktype = libc::SOCK_STREAM;
    let mut res = ptr::null_mut();
    let rc = unsafe { libc::getaddrinfo(c_host.as_ptr(), ptr::null(), &hints, &mut res) };
    if rc != 0 {
        return Err(format!("getaddrinfo({host}): {rc}").into());
    }
    let mut out = Vec::new();
    let mut cur = res;
    while !cur.is_null() {
        let ai = unsafe { &*cur };
        if ai.ai_family == libc::AF_INET
            && !ai.ai_addr.is_null()
            && (ai.ai_addrlen as usize) >= std::mem::size_of::<libc::sockaddr_in>()
        {
            let sin = unsafe { &*(ai.ai_addr as *const libc::sockaddr_in) };
            let ip = Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
            out.push(SocketAddr::new(IpAddr::V4(ip), 0));
        }
        cur = ai.ai_next;
    }
    unsafe { libc::freeaddrinfo(res) };
    Ok(out)
}

/// Retry a rebuilt request on transport errors (reset / timeout / handshake).
pub async fn send_retry(build: impl Fn() -> reqwest::RequestBuilder) -> Result<reqwest::Response> {
    const BACKOFF: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(3)];
    let mut last = None;
    for attempt in 0..=BACKOFF.len() {
        match build().send().await {
            Ok(resp) => return Ok(resp),
            Err(e) => {
                last = Some(e);
                if let Some(pause) = BACKOFF.get(attempt) {
                    tokio::time::sleep(*pause).await;
                }
            }
        }
    }
    Err(last.unwrap().into())
}

/// Android has no usable system root store for rustls without a Java bridge.
#[cfg(target_os = "android")]
fn android_tls_config() -> rustls::ClientConfig {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let mut cfg = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    cfg
}

/// Minimal storage contract. Keys are `/`-separated, relative to the vault root.
///
/// Content-addressed objects (blobs, large-file chunks) are immutable: a key
/// fully determines the plaintext, so `put_if_absent` only saves traffic and a
/// race between two writers is harmless.
#[async_trait]
pub trait Storage: Send + Sync {
    /// All keys under `prefix`, recursively.
    async fn list(&self, prefix: &str) -> Result<Vec<String>>;
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;
    /// Atomic on the storage side: readers never observe a partial object.
    async fn put(&self, key: &str, data: &[u8]) -> Result<()>;
    async fn delete(&self, key: &str) -> Result<()>;
    /// Presence without transferring the body.
    async fn exists(&self, key: &str) -> Result<bool> {
        Ok(self.get(key).await?.is_some())
    }
    /// Write only when `key` is absent; returns whether bytes were written.
    async fn put_if_absent(&self, key: &str, data: &[u8]) -> Result<bool> {
        if self.exists(key).await? {
            return Ok(false);
        }
        self.put(key, data).await?;
        Ok(true)
    }
}

/// 401/403 are never retried and surface as `Auth`.
pub fn status_error(status: reqwest::StatusCode, what: &str, body: &str) -> SyncError {
    let msg = format!(
        "{what}: {status} {}",
        body.chars().take(300).collect::<String>()
    );
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        SyncError::Auth(msg)
    } else {
        SyncError::Storage(msg)
    }
}

/// Suffix of in-progress writes; skipped by every adapter's `list`.
pub const TMP_SUFFIX: &str = ".tmp";

/// Keys the vault never produces; ignored so a cloud client's metadata files
/// don't make a folder look "foreign".
pub fn is_noise_key(key: &str) -> bool {
    let name = key.rsplit('/').next().unwrap_or(key);
    name.starts_with('.')
        || name.ends_with(TMP_SUFFIX)
        || name.eq_ignore_ascii_case("desktop.ini")
        || name.eq_ignore_ascii_case("thumbs.db")
}

/// A plain directory. Works with any folder a cloud client mirrors.
pub struct LocalDir {
    root: PathBuf,
}

impl LocalDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn path_for(&self, key: &str) -> Result<PathBuf> {
        if key.split('/').any(|seg| seg == ".." || seg.is_empty()) {
            return Err(SyncError::Storage(format!("bad key {key}")));
        }
        Ok(self.root.join(key))
    }

    fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            let key = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            if is_noise_key(&key) {
                continue;
            }
            let ty = entry.file_type()?;
            if ty.is_dir() {
                Self::walk(&entry.path(), &key, out)?;
            } else {
                out.push(key);
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Storage for LocalDir {
    async fn list(&self, prefix: &str) -> Result<Vec<String>> {
        if !self.root.exists() {
            return Ok(vec![]);
        }
        let mut all = Vec::new();
        Self::walk(&self.root, "", &mut all)?;
        all.retain(|k| k.starts_with(prefix));
        Ok(all)
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        match std::fs::read(self.path_for(key)?) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn put(&self, key: &str, data: &[u8]) -> Result<()> {
        use std::io::Write;
        let path = self.path_for(key)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = PathBuf::from(format!("{}{}", path.display(), TMP_SUFFIX));
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<()> {
        match std::fs::remove_file(self.path_for(key)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        Ok(std::fs::metadata(self.path_for(key)?)
            .map(|m| m.is_file())
            .unwrap_or(false))
    }
}
