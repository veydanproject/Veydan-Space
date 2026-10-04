// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Where encrypted chunks are stored. Every backend is content-addressed:
//! a blob is written under the SHA-256 of its bytes and read back from
//! `<public_base>/<sha256>` without credentials (the content is encrypted
//! and the name unguessable). Writing needs credentials.

use crate::sigv4::{self, Credentials, Request};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use messenger_core::{MessengerError, Result};
use nostr::key::Keys;
use nostr::prelude::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Failure of one request, with what a retry policy needs to know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendError {
    pub status: Option<u16>,
    pub message: String,
}

impl BackendError {
    /// Network errors, timeouts, 429 and 5xx are worth another attempt;
    /// 4xx means the request itself is wrong.
    pub fn is_retryable(&self) -> bool {
        match self.status {
            None => true,
            Some(s) => s == 408 || s == 429 || s >= 500,
        }
    }

    pub fn code(&self) -> &'static str {
        match self.status {
            None => "err.network",
            Some(401) | Some(403) => "err.auth_failed",
            Some(413) => "err.file_too_large",
            Some(429) => "err.rate_limited",
            Some(s) if s >= 500 => "err.server",
            Some(_) => "err.rejected",
        }
    }
}

impl From<BackendError> for MessengerError {
    fn from(e: BackendError) -> Self {
        MessengerError::Transport(format!("{}: {}", e.code(), e.message))
    }
}

pub type BackendResult<T> = std::result::Result<T, BackendError>;

#[async_trait]
pub trait BlobBackend: Send + Sync {
    /// Base under which blobs are readable: `<public_base>/<sha256>`.
    fn public_base(&self) -> String;
    /// Is the blob already there? (resume and dedup)
    async fn exists(&self, sha256: &str) -> BackendResult<bool>;
    async fn put(&self, sha256: &str, bytes: Vec<u8>) -> BackendResult<()>;
    /// Make sure the store is usable (bucket exists, readable by peers).
    async fn prepare(&self) -> BackendResult<()> {
        Ok(())
    }
}

pub(crate) fn http_client() -> Result<reqwest::Client> {
    messenger_http::client(Duration::from_secs(10), Duration::from_secs(600))
}

/// The host of a URL, for [`messenger_http::following_route`].
pub(crate) fn host_of(url: &str) -> String {
    reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default()
}

/// The way to the server changed under a request three times running.
fn route_changed(_: messenger_http::RouteChanged) -> BackendError {
    BackendError { status: None, message: "the way to the server kept changing".into() }
}

fn from_reqwest(e: reqwest::Error) -> BackendError {
    BackendError { status: e.status().map(|s| s.as_u16()), message: e.to_string() }
}

async fn fail(resp: reqwest::Response) -> BackendError {
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    let message: String = body.chars().take(200).collect();
    BackendError { status: Some(status), message: if message.is_empty() { format!("http {status}") } else { message } }
}

// ─── S3 ─────────────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq, Eq)]
pub struct S3Config {
    /// `https://host[:port]`, path-style addressing.
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
}

impl std::fmt::Debug for S3Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Config")
            .field("endpoint", &self.endpoint)
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .finish_non_exhaustive()
    }
}

pub struct S3Backend {
    cfg: S3Config,
    http: reqwest::Client,
}

impl S3Backend {
    pub fn new(cfg: S3Config) -> Result<Self> {
        let endpoint = cfg.endpoint.trim_end_matches('/').to_string();
        if !(endpoint.starts_with("https://") || endpoint.starts_with("http://")) {
            return Err(MessengerError::Invalid("s3 endpoint must be http(s)".into()));
        }
        let ok_bucket = (3..=63).contains(&cfg.bucket.len())
            && cfg.bucket.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.');
        if !ok_bucket {
            return Err(MessengerError::Invalid("s3 bucket name must be 3-63 chars of a-z, 0-9, '-', '.'".into()));
        }
        Ok(Self { cfg: S3Config { endpoint, ..cfg }, http: http_client()? })
    }

    fn host(&self) -> String {
        self.cfg.endpoint.split("://").nth(1).unwrap_or("").to_string()
    }

    fn signed(
        &self,
        method: &str,
        path: &str,
        query: &[(&str, &str)],
        payload_sha256: &str,
        extra: Vec<(String, String)>,
    ) -> Vec<(String, String)> {
        let mut headers = vec![("host".to_string(), self.host())];
        headers.extend(extra);
        sigv4::sign(
            &Credentials { access_key: &self.cfg.access_key, secret_key: &self.cfg.secret_key, region: &self.cfg.region },
            Request { method, path, query, headers, payload_sha256 },
            now(),
        )
    }

    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, &str)],
        body: Option<Vec<u8>>,
        extra: Vec<(String, String)>,
    ) -> BackendResult<reqwest::Response> {
        let payload = match &body {
            Some(b) => crate::crypto::sha256_hex(b),
            None => sigv4::EMPTY_SHA256.to_string(),
        };
        let headers = self.signed(method.as_str(), path, query, &payload, extra);
        let qs = if query.is_empty() {
            String::new()
        } else {
            let mut q: Vec<String> =
                query.iter().map(|(k, v)| format!("{}={}", sigv4::uri_encode(k, false), sigv4::uri_encode(v, false))).collect();
            q.sort();
            format!("?{}", q.join("&"))
        };
        let url = format!("{}{}{}", self.cfg.endpoint, sigv4::uri_encode(path, true), qs);
        let host = host_of(&url);
        messenger_http::following_route(&host, || {
            let mut req = self.http.request(method.clone(), &url);
            for (k, v) in &headers {
                if k != "host" {
                    req = req.header(k, v);
                }
            }
            if let Some(b) = &body {
                req = req.body(b.clone());
            }
            req.send()
        })
        .await
        .map_err(route_changed)?
        .map_err(from_reqwest)
    }

    fn object_path(&self, sha256: &str) -> String {
        format!("/{}/{}", self.cfg.bucket, sha256)
    }

    /// Anonymous read of objects, nothing else: peers fetch chunks by hash.
    fn public_read_policy(&self) -> String {
        serde_json::json!({
            "Version": "2012-10-17",
            "Statement": [{
                "Effect": "Allow",
                "Principal": { "AWS": ["*"] },
                "Action": ["s3:GetObject"],
                "Resource": [format!("arn:aws:s3:::{}/*", self.cfg.bucket)],
            }]
        })
        .to_string()
    }
}

#[async_trait]
impl BlobBackend for S3Backend {
    fn public_base(&self) -> String {
        format!("{}/{}", self.cfg.endpoint, self.cfg.bucket)
    }

    async fn exists(&self, sha256: &str) -> BackendResult<bool> {
        let resp = self.send(reqwest::Method::HEAD, &self.object_path(sha256), &[], None, vec![]).await?;
        match resp.status().as_u16() {
            200 => Ok(true),
            404 => Ok(false),
            _ => Err(fail(resp).await),
        }
    }

    async fn put(&self, sha256: &str, bytes: Vec<u8>) -> BackendResult<()> {
        let extra = vec![("content-type".to_string(), "application/octet-stream".to_string())];
        let resp = self.send(reqwest::Method::PUT, &self.object_path(sha256), &[], Some(bytes), extra).await?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(fail(resp).await)
        }
    }

    /// Create the bucket when it is missing and allow anonymous reads.
    async fn prepare(&self) -> BackendResult<()> {
        let bucket_path = format!("/{}", self.cfg.bucket);
        let head = self.send(reqwest::Method::HEAD, &bucket_path, &[], None, vec![]).await?;
        match head.status().as_u16() {
            200 => {}
            404 => {
                let created = self.send(reqwest::Method::PUT, &bucket_path, &[], Some(Vec::new()), vec![]).await?;
                if !created.status().is_success() && created.status().as_u16() != 409 {
                    return Err(fail(created).await);
                }
            }
            _ => return Err(fail(head).await),
        }
        let policy = self.public_read_policy().into_bytes();
        let resp = self.send(reqwest::Method::PUT, &bucket_path, &[("policy", "")], Some(policy), vec![]).await?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(fail(resp).await)
        }
    }
}

// ─── Blossom ────────────────────────────────────────────────────────────────

/// Blossom (BUD-01/02): `PUT <base>/upload` authorised by a signed
/// kind-24242 event, blobs readable at `<base>/<sha256>`.
pub struct BlossomBackend {
    base: String,
    keys: Keys,
    http: reqwest::Client,
}

impl BlossomBackend {
    pub fn new(base: &str, keys: Keys) -> Result<Self> {
        let base = base.trim_end_matches('/').to_string();
        if !(base.starts_with("https://") || base.starts_with("http://")) {
            return Err(MessengerError::Invalid("blossom url must be http(s)".into()));
        }
        Ok(Self { base, keys, http: http_client()? })
    }

    /// `Authorization: Nostr <base64(event)>`. The event is dated slightly
    /// in the past so a server with a slow clock accepts it.
    pub fn auth_header(&self, verb: &str, sha256: &str) -> BackendResult<String> {
        let err = |e: String| BackendError { status: None, message: e };
        let t = now();
        let event = EventBuilder::new(Kind::from(24242u16), format!("{verb} {sha256}"))
            .tag(Tag::parse(["t", verb]).map_err(|e| err(e.to_string()))?)
            .tag(Tag::parse(["x", sha256]).map_err(|e| err(e.to_string()))?)
            .tag(Tag::parse(["expiration", &(t + 300).to_string()]).map_err(|e| err(e.to_string()))?)
            .custom_created_at(nostr::types::Timestamp::from_secs((t - 30).max(0) as u64))
            .finalize(&self.keys)
            .map_err(|e| err(e.to_string()))?;
        let json = serde_json::to_string(&event).map_err(|e| err(e.to_string()))?;
        Ok(format!("Nostr {}", B64.encode(json)))
    }
}

#[async_trait]
impl BlobBackend for BlossomBackend {
    fn public_base(&self) -> String {
        self.base.clone()
    }

    async fn exists(&self, sha256: &str) -> BackendResult<bool> {
        let url = format!("{}/{}", self.base, sha256);
        let resp = messenger_http::following_route(&host_of(&url), || self.http.head(&url).send())
            .await
            .map_err(route_changed)?
            .map_err(from_reqwest)?;
        match resp.status().as_u16() {
            200 => Ok(true),
            404 => Ok(false),
            _ => Err(fail(resp).await),
        }
    }

    async fn put(&self, sha256: &str, bytes: Vec<u8>) -> BackendResult<()> {
        let url = format!("{}/upload", self.base);
        let auth = self.auth_header("upload", sha256)?;
        let resp = messenger_http::following_route(&host_of(&url), || {
            self.http
                .put(&url)
                .header("authorization", &auth)
                .header("content-type", "application/octet-stream")
                .header("x-sha-256", sha256)
                .body(bytes.clone())
                .send()
        })
        .await
        .map_err(route_changed)?
        .map_err(from_reqwest)?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(fail(resp).await)
        }
    }
}

// ─── In memory (tests, offline development) ─────────────────────────────────

/// Blobs in a map, with switches to simulate a flaky or dying server.
#[derive(Clone, Default)]
pub struct MemoryBackend {
    pub blobs: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    /// Fail this many `put` calls with a retryable error, then recover.
    pub flaky_puts: Arc<Mutex<u32>>,
    /// After this many successful puts every call fails for good
    /// (`None` = never): the connection dropped mid-transfer.
    pub die_after_puts: Arc<Mutex<Option<u32>>>,
    pub put_calls: Arc<Mutex<u32>>,
    pub base: String,
}

impl MemoryBackend {
    pub fn new(base: &str) -> Self {
        Self { base: base.to_string(), ..Default::default() }
    }

    pub fn get(&self, sha256: &str) -> Option<Vec<u8>> {
        self.blobs.lock().unwrap().get(sha256).cloned()
    }

    pub fn len(&self) -> usize {
        self.blobs.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait]
impl BlobBackend for MemoryBackend {
    fn public_base(&self) -> String {
        self.base.clone()
    }

    async fn exists(&self, sha256: &str) -> BackendResult<bool> {
        Ok(self.blobs.lock().unwrap().contains_key(sha256))
    }

    async fn put(&self, sha256: &str, bytes: Vec<u8>) -> BackendResult<()> {
        *self.put_calls.lock().unwrap() += 1;
        {
            let mut flaky = self.flaky_puts.lock().unwrap();
            if *flaky > 0 {
                *flaky -= 1;
                return Err(BackendError { status: Some(503), message: "try again".into() });
            }
        }
        {
            let mut die = self.die_after_puts.lock().unwrap();
            if let Some(n) = die.as_mut() {
                if *n == 0 {
                    return Err(BackendError { status: None, message: "connection lost".into() });
                }
                *n -= 1;
            }
        }
        if crate::crypto::sha256_hex(&bytes) != sha256 {
            return Err(BackendError { status: Some(400), message: "hash mismatch".into() });
        }
        self.blobs.lock().unwrap().insert(sha256.to_string(), bytes);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s3_config_validation_and_urls() {
        let cfg = S3Config {
            endpoint: "https://s3.example:9000/".into(),
            bucket: "veydan-media".into(),
            region: "us-east-1".into(),
            access_key: "a".into(),
            secret_key: "s".into(),
        };
        let b = S3Backend::new(cfg.clone()).unwrap();
        assert_eq!(b.public_base(), "https://s3.example:9000/veydan-media");
        assert_eq!(b.host(), "s3.example:9000");
        assert_eq!(b.object_path("abc"), "/veydan-media/abc");
        assert!(b.public_read_policy().contains("arn:aws:s3:::veydan-media/*"));
        assert!(!format!("{cfg:?}").contains("secret"), "debug output never shows keys");
        assert!(S3Backend::new(S3Config { bucket: "Bad_Name".into(), ..cfg.clone() }).is_err());
        assert!(S3Backend::new(S3Config { endpoint: "ftp://x".into(), ..cfg }).is_err());
    }

    #[test]
    fn blossom_auth_event_is_signed_and_scoped() {
        let keys = Keys::generate();
        let b = BlossomBackend::new("https://blossom.example/", keys.clone()).unwrap();
        assert_eq!(b.public_base(), "https://blossom.example");
        let h = b.auth_header("upload", &"ab".repeat(32)).unwrap();
        let json = String::from_utf8(B64.decode(h.strip_prefix("Nostr ").unwrap()).unwrap()).unwrap();
        let ev: Event = serde_json::from_str(&json).unwrap();
        ev.verify().unwrap();
        assert_eq!(ev.kind.as_u16(), 24242);
        assert_eq!(ev.pubkey, keys.public_key());
        let tag = |k: &str| ev.tags.iter().find(|t| t.kind() == k).and_then(|t| t.as_slice().get(1).cloned());
        assert_eq!(tag("t").as_deref(), Some("upload"));
        assert_eq!(tag("x"), Some("ab".repeat(32)));
        let exp: i64 = tag("expiration").unwrap().parse().unwrap();
        assert!(exp > ev.created_at.as_secs() as i64 + 300);
    }

    #[test]
    fn retry_classification() {
        let e = |s| BackendError { status: s, message: String::new() };
        assert!(e(None).is_retryable());
        assert!(e(Some(503)).is_retryable());
        assert!(e(Some(429)).is_retryable());
        assert!(!e(Some(403)).is_retryable());
        assert!(!e(Some(413)).is_retryable());
        assert_eq!(e(Some(403)).code(), "err.auth_failed");
        assert_eq!(e(Some(413)).code(), "err.file_too_large");
        assert_eq!(e(None).code(), "err.network");
    }

    #[tokio::test]
    async fn memory_backend_checks_hashes() {
        let m = MemoryBackend::new("mem://x");
        let sha = crate::crypto::sha256_hex(b"data");
        assert!(!m.exists(&sha).await.unwrap());
        assert!(m.put("00", b"data".to_vec()).await.is_err());
        m.put(&sha, b"data".to_vec()).await.unwrap();
        assert!(m.exists(&sha).await.unwrap());
        assert_eq!(m.get(&sha).unwrap(), b"data");
    }
}
