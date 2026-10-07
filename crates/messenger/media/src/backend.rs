// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Where encrypted chunks are stored. Every backend is content-addressed:
//! a blob is written under the SHA-256 of its bytes and read back from
//! `<public_base>/<sha256>` without credentials (the content is encrypted
//! and the name unguessable). Writing needs credentials.
//!
//! Public blobs (an avatar) take the same way, unencrypted and with their
//! own content type: `put_typed`, and `delete` when they are replaced.

use crate::sigv4::{self, Credentials, Request};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use messenger_core::{MessengerError, Result};
use nostr::key::Keys;
use nostr::prelude::*;
use bytes::Bytes;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
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
            Some(408) => "err.timeout",
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
    /// `put`, adding the bytes sent so far to `sent` while they leave
    /// (`sent` starts again from 0 when the request does). A backend that
    /// cannot tell adds them all once the server has them.
    async fn put_counted(&self, sha256: &str, bytes: Bytes, sent: Arc<AtomicU64>) -> BackendResult<()> {
        let len = bytes.len() as u64;
        self.put(sha256, bytes.to_vec()).await?;
        sent.fetch_add(len, Ordering::Relaxed);
        Ok(())
    }
    /// Store a blob that is served with `content_type` (a public blob).
    /// A backend that cannot keep a type stores it as `put` does.
    async fn put_typed(&self, sha256: &str, bytes: Vec<u8>, content_type: &str) -> BackendResult<()> {
        let _ = content_type;
        self.put(sha256, bytes).await
    }
    /// Remove a blob. A blob that is not there counts as removed.
    async fn delete(&self, sha256: &str) -> BackendResult<()> {
        let _ = sha256;
        Err(BackendError { status: Some(405), message: "this server cannot delete".into() })
    }
    /// Make sure the store is usable (bucket exists, readable by peers).
    async fn prepare(&self) -> BackendResult<()> {
        Ok(())
    }
}

/// A content type that may go into a header as it is: printable ASCII
/// with a `/` inside, at most 127 bytes. Spaces stand alone and never at
/// an end: SigV4 signs a header with runs of spaces collapsed and the
/// ends trimmed, so any other spacing would be signed unlike S3 checks it.
pub fn valid_content_type(ct: &str) -> bool {
    (3..=127).contains(&ct.len())
        && ct.bytes().all(|b| (0x21..0x7f).contains(&b) || b == b' ')
        && ct.find('/').is_some_and(|i| i > 0 && i + 1 < ct.len())
        && !ct.starts_with(' ')
        && !ct.ends_with(' ')
        && !ct.contains("  ")
}

fn check_content_type(ct: &str) -> BackendResult<()> {
    if valid_content_type(ct) {
        Ok(())
    } else {
        Err(BackendError { status: Some(400), message: "bad content type".into() })
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

/// Pieces a counted body is handed to the connection in: well under the
/// time the speed is averaged over, even on a slow line.
const PIECE: usize = 64 * 1024;

/// A body that counts its bytes into `sent` as the connection takes them.
/// Made anew for every attempt of a request; the count starts again.
fn counted_body(bytes: &Bytes, sent: &Arc<AtomicU64>) -> reqwest::Body {
    use futures_util::StreamExt as _;
    sent.store(0, Ordering::Relaxed);
    let pieces: Vec<Bytes> = (0..bytes.len()).step_by(PIECE).map(|at| bytes.slice(at..(at + PIECE).min(bytes.len()))).collect();
    let sent = sent.clone();
    let stream = futures_util::stream::iter(pieces).map(move |piece| {
        sent.fetch_add(piece.len() as u64, Ordering::Relaxed);
        Ok::<_, std::io::Error>(piece)
    });
    reqwest::Body::wrap_stream(stream)
}

/// What a request carries.
struct Payload {
    bytes: Bytes,
    /// Hex SHA-256 of `bytes`.
    sha256: String,
    /// Counts the bytes as they leave, when somebody watches.
    sent: Option<Arc<AtomicU64>>,
}

impl Payload {
    fn whole(bytes: Vec<u8>) -> Self {
        let sha256 = crate::crypto::sha256_hex(&bytes);
        Self { bytes: bytes.into(), sha256, sent: None }
    }

    /// The body of one attempt. A stream has no length of its own, so the
    /// length goes in a header: stores refuse a chunked upload.
    fn attach(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.sent {
            Some(sent) => req.header(reqwest::header::CONTENT_LENGTH, self.bytes.len()).body(counted_body(&self.bytes, sent)),
            None => req.body(self.bytes.clone()),
        }
    }
}

/// A delete is done when the server says so or has nothing to delete.
async fn deleted(resp: reqwest::Response) -> BackendResult<()> {
    if resp.status().is_success() || resp.status().as_u16() == 404 {
        Ok(())
    } else {
        Err(fail(resp).await)
    }
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
        body: Option<Payload>,
        extra: Vec<(String, String)>,
    ) -> BackendResult<reqwest::Response> {
        let payload = match &body {
            Some(b) => b.sha256.clone(),
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
                req = b.attach(req);
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

    async fn put_payload(&self, sha256: &str, body: Payload, content_type: &str) -> BackendResult<()> {
        check_content_type(content_type)?;
        let extra = vec![("content-type".to_string(), content_type.to_string())];
        let resp = self.send(reqwest::Method::PUT, &self.object_path(sha256), &[], Some(body), extra).await?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(fail(resp).await)
        }
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
        self.put_typed(sha256, bytes, "application/octet-stream").await
    }

    /// Streamed. The name of a blob is the SHA-256 of its bytes, so it is
    /// the payload hash SigV4 signs; a wrong one is refused by the store.
    async fn put_counted(&self, sha256: &str, bytes: Bytes, sent: Arc<AtomicU64>) -> BackendResult<()> {
        let body = Payload { bytes, sha256: sha256.to_string(), sent: Some(sent) };
        self.put_payload(sha256, body, "application/octet-stream").await
    }

    /// The content type is signed with the request and kept by the store,
    /// which serves the object with it.
    async fn put_typed(&self, sha256: &str, bytes: Vec<u8>, content_type: &str) -> BackendResult<()> {
        self.put_payload(sha256, Payload::whole(bytes), content_type).await
    }

    async fn delete(&self, sha256: &str) -> BackendResult<()> {
        deleted(self.send(reqwest::Method::DELETE, &self.object_path(sha256), &[], None, vec![]).await?).await
    }

    /// Create the bucket when it is missing and allow anonymous reads.
    async fn prepare(&self) -> BackendResult<()> {
        let bucket_path = format!("/{}", self.cfg.bucket);
        let head = self.send(reqwest::Method::HEAD, &bucket_path, &[], None, vec![]).await?;
        match head.status().as_u16() {
            200 => {}
            404 => {
                let created = self.send(reqwest::Method::PUT, &bucket_path, &[], Some(Payload::whole(Vec::new())), vec![]).await?;
                if !created.status().is_success() && created.status().as_u16() != 409 {
                    return Err(fail(created).await);
                }
            }
            _ => return Err(fail(head).await),
        }
        let policy = self.public_read_policy().into_bytes();
        let resp = self.send(reqwest::Method::PUT, &bucket_path, &[("policy", "")], Some(Payload::whole(policy)), vec![]).await?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(fail(resp).await)
        }
    }
}

// ─── Blossom ────────────────────────────────────────────────────────────────

/// How long a Blossom upload token is valid, in seconds.
const AUTH_SECS: i64 = 300;
/// How long a Blossom delete token is valid, in seconds.
const DELETE_AUTH_SECS: i64 = 60;

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
    ///
    /// A delete is bound to this server (BUD-01 `server` tag) and lives a
    /// minute: the same blob sits on other servers too, and the server
    /// that receives the token must not replay it there. Uploads stay
    /// unbound: a replayed upload only stores the same bytes again.
    pub fn auth_header(&self, verb: &str, sha256: &str) -> BackendResult<String> {
        let err = |e: String| BackendError { status: None, message: e };
        let t = now();
        let delete = verb == "delete";
        let lifetime = if delete { DELETE_AUTH_SECS } else { AUTH_SECS };
        let mut builder = EventBuilder::new(Kind::from(24242u16), format!("{verb} {sha256}"))
            .tag(Tag::parse(["t", verb]).map_err(|e| err(e.to_string()))?)
            .tag(Tag::parse(["x", sha256]).map_err(|e| err(e.to_string()))?)
            .tag(Tag::parse(["expiration", &(t + lifetime).to_string()]).map_err(|e| err(e.to_string()))?);
        if delete {
            let host = host_of(&self.base);
            if host.is_empty() {
                return Err(err("blossom url has no host".into()));
            }
            builder = builder.tag(Tag::parse(["server", &host]).map_err(|e| err(e.to_string()))?);
        }
        let event = builder
            .custom_created_at(nostr::types::Timestamp::from_secs((t - 30).max(0) as u64))
            .finalize(&self.keys)
            .map_err(|e| err(e.to_string()))?;
        let json = serde_json::to_string(&event).map_err(|e| err(e.to_string()))?;
        Ok(format!("Nostr {}", B64.encode(json)))
    }

    async fn put_payload(&self, body: Payload, content_type: &str) -> BackendResult<()> {
        check_content_type(content_type)?;
        let url = format!("{}/upload", self.base);
        let auth = self.auth_header("upload", &body.sha256)?;
        let resp = messenger_http::following_route(&host_of(&url), || {
            let req = self
                .http
                .put(&url)
                .header("authorization", &auth)
                .header("content-type", content_type)
                .header("x-sha-256", &body.sha256);
            body.attach(req).send()
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
        self.put_typed(sha256, bytes, "application/octet-stream").await
    }

    /// Streamed; `x-sha-256` names the blob the server must find.
    async fn put_counted(&self, sha256: &str, bytes: Bytes, sent: Arc<AtomicU64>) -> BackendResult<()> {
        let body = Payload { bytes, sha256: sha256.to_string(), sent: Some(sent) };
        self.put_payload(body, "application/octet-stream").await
    }

    async fn put_typed(&self, sha256: &str, bytes: Vec<u8>, content_type: &str) -> BackendResult<()> {
        let body = Payload { bytes: bytes.into(), sha256: sha256.to_string(), sent: None };
        self.put_payload(body, content_type).await
    }

    /// BUD-02: `DELETE <base>/<sha256>`, authorised for that blob only.
    async fn delete(&self, sha256: &str) -> BackendResult<()> {
        let url = format!("{}/{}", self.base, sha256);
        let auth = self.auth_header("delete", sha256)?;
        let resp =
            messenger_http::following_route(&host_of(&url), || self.http.delete(&url).header("authorization", &auth).send())
                .await
                .map_err(route_changed)?
                .map_err(from_reqwest)?;
        deleted(resp).await
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
    /// Content type of every blob stored by `put_typed`.
    pub content_types: Arc<Mutex<HashMap<String, String>>>,
    pub delete_calls: Arc<Mutex<u32>>,
    pub base: String,
    /// Every `put` takes this long: a slow line.
    pub put_delay: Arc<Mutex<Duration>>,
    /// While set, every `put` waits for a permit of it: a line that hangs,
    /// opened a request at a time by adding permits.
    pub put_gate: Arc<Mutex<Option<Arc<tokio::sync::Semaphore>>>>,
    /// Only puts of at least this many bytes wait at the gate; smaller
    /// ones go through (a photo beside a big file).
    pub gate_from: Arc<Mutex<usize>>,
    /// Refuse every `put` for good (403).
    pub reject_puts: Arc<Mutex<bool>>,
    /// Puts under way now, and the most there ever were at once.
    pub puts_in_flight: Arc<AtomicUsize>,
    pub max_puts_in_flight: Arc<AtomicUsize>,
    /// Every `exists` call (a HEAD), and how long each takes.
    pub exists_calls: Arc<AtomicUsize>,
    pub exists_delay: Arc<Mutex<Duration>>,
    /// Every `exists` fails as the network would (a HEAD that gets no
    /// answer).
    pub fail_exists: Arc<Mutex<bool>>,
    /// Every `exists` is refused with this status (a server that does not
    /// answer a HEAD as asked).
    pub exists_status: Arc<Mutex<Option<u16>>>,
    /// The put of this one blob hangs for as long as the request lives;
    /// the others go through.
    pub hang_sha: Arc<Mutex<Option<String>>>,
    /// The server keeps what it was sent even when the client gave up on
    /// the request: a put dropped while waiting at the gate is stored once
    /// the gate lets it through.
    pub keep_dropped_puts: Arc<Mutex<bool>>,
    /// `put_counted` counts half of the body as sent before the server has
    /// it, as a streamed request does.
    pub streamed: Arc<Mutex<bool>>,
}

/// Counts a put as under way for as long as it lives.
struct Underway(Arc<AtomicUsize>);

impl Drop for Underway {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
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

    pub fn content_type(&self, sha256: &str) -> Option<String> {
        self.content_types.lock().unwrap().get(sha256).cloned()
    }
}

#[async_trait]
impl BlobBackend for MemoryBackend {
    fn public_base(&self) -> String {
        self.base.clone()
    }

    async fn exists(&self, sha256: &str) -> BackendResult<bool> {
        self.exists_calls.fetch_add(1, Ordering::SeqCst);
        let delay = *self.exists_delay.lock().unwrap();
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        if *self.fail_exists.lock().unwrap() {
            return Err(BackendError { status: None, message: "connection reset".into() });
        }
        if let Some(status) = *self.exists_status.lock().unwrap() {
            return Err(BackendError { status: Some(status), message: "refused".into() });
        }
        Ok(self.blobs.lock().unwrap().contains_key(sha256))
    }

    async fn put(&self, sha256: &str, bytes: Vec<u8>) -> BackendResult<()> {
        *self.put_calls.lock().unwrap() += 1;
        if self.hang_sha.lock().unwrap().as_deref() == Some(sha256) {
            std::future::pending::<()>().await;
        }
        if *self.keep_dropped_puts.lock().unwrap() {
            // The request goes on without us when we drop it.
            let (this, sha256) = (self.clone(), sha256.to_string());
            return tokio::spawn(async move { this.store(&sha256, bytes).await })
                .await
                .unwrap_or_else(|e| Err(BackendError { status: None, message: e.to_string() }));
        }
        self.store(sha256, bytes).await
    }

    async fn put_counted(&self, sha256: &str, bytes: Bytes, sent: Arc<AtomicU64>) -> BackendResult<()> {
        let len = bytes.len() as u64;
        let early = if *self.streamed.lock().unwrap() { len / 2 } else { 0 };
        sent.fetch_add(early, Ordering::Relaxed);
        self.put(sha256, bytes.to_vec()).await?;
        sent.fetch_add(len - early, Ordering::Relaxed);
        Ok(())
    }

    async fn put_typed(&self, sha256: &str, bytes: Vec<u8>, content_type: &str) -> BackendResult<()> {
        check_content_type(content_type)?;
        self.put(sha256, bytes).await?;
        self.content_types.lock().unwrap().insert(sha256.to_string(), content_type.to_string());
        Ok(())
    }

    async fn delete(&self, sha256: &str) -> BackendResult<()> {
        *self.delete_calls.lock().unwrap() += 1;
        self.blobs.lock().unwrap().remove(sha256);
        self.content_types.lock().unwrap().remove(sha256);
        Ok(())
    }
}

impl MemoryBackend {
    /// A put as the server sees it: wait as the switches say, then check
    /// and keep the blob.
    async fn store(&self, sha256: &str, bytes: Vec<u8>) -> BackendResult<()> {
        let now = self.puts_in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        let _underway = Underway(self.puts_in_flight.clone());
        self.max_puts_in_flight.fetch_max(now, Ordering::SeqCst);
        let gate = self.put_gate.lock().unwrap().clone().filter(|_| bytes.len() >= *self.gate_from.lock().unwrap());
        if let Some(gate) = gate {
            if let Ok(permit) = gate.acquire().await {
                permit.forget();
            }
        }
        let delay = *self.put_delay.lock().unwrap();
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        if *self.reject_puts.lock().unwrap() {
            return Err(BackendError { status: Some(403), message: "not allowed".into() });
        }
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
        assert!(e(Some(408)).is_retryable());
        assert_eq!(e(Some(408)).code(), "err.timeout");
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

    #[tokio::test]
    async fn memory_backend_keeps_types_and_deletes() {
        let m = MemoryBackend::new("mem://x");
        let sha = crate::crypto::sha256_hex(b"jpeg");
        m.put_typed(&sha, b"jpeg".to_vec(), "image/jpeg").await.unwrap();
        assert_eq!(m.content_type(&sha).as_deref(), Some("image/jpeg"));
        assert_eq!(*m.put_calls.lock().unwrap(), 1);
        assert!(m.put_typed(&sha, b"jpeg".to_vec(), "jpeg").await.is_err(), "a type without a slash");
        m.delete(&sha).await.unwrap();
        assert!(!m.exists(&sha).await.unwrap());
        assert!(m.content_type(&sha).is_none());
        m.delete(&sha).await.unwrap();
        assert_eq!(*m.delete_calls.lock().unwrap(), 2, "a missing blob is deleted all the same");
    }

    /// Only what the trait requires: the defaults take over.
    struct Plain(MemoryBackend);
    #[async_trait]
    impl BlobBackend for Plain {
        fn public_base(&self) -> String {
            self.0.public_base()
        }
        async fn exists(&self, sha256: &str) -> BackendResult<bool> {
            self.0.exists(sha256).await
        }
        async fn put(&self, sha256: &str, bytes: Vec<u8>) -> BackendResult<()> {
            self.0.put(sha256, bytes).await
        }
    }

    #[tokio::test]
    async fn trait_defaults_store_untyped_and_refuse_to_delete() {
        let p = Plain(MemoryBackend::new("mem://p"));
        let sha = crate::crypto::sha256_hex(b"x");
        p.put_typed(&sha, b"x".to_vec(), "image/png").await.unwrap();
        assert!(p.0.get(&sha).is_some());
        assert!(p.0.content_type(&sha).is_none(), "the default goes through put");
        let err = p.delete(&sha).await.unwrap_err();
        assert_eq!(err.status, Some(405));
        assert!(!err.is_retryable());
        assert!(p.0.get(&sha).is_some());
    }

    #[test]
    fn content_types() {
        for ok in ["image/jpeg", "image/png", "application/octet-stream", "text/plain; charset=utf-8"] {
            assert!(valid_content_type(ok), "{ok}");
        }
        for bad in ["", "jpeg", "/jpeg", "image/", "image/jpeg\r\nx-evil: 1", "image/jpég", &format!("image/{}", "a".repeat(130))] {
            assert!(!valid_content_type(bad), "{bad:?}");
        }
    }

    /// SigV4 signs a header value with its spaces collapsed and its ends
    /// trimmed; a type spaced otherwise would be signed unlike S3 checks it.
    #[test]
    fn content_types_are_spaced_as_signed() {
        for bad in ["text/plain;  charset=utf-8", " image/jpeg", "image/jpeg ", "text/plain; charset=utf-8  "] {
            assert!(!valid_content_type(bad), "{bad:?}");
        }
        assert!(valid_content_type("text/plain; charset=utf-8"));
    }

    #[tokio::test]
    async fn s3_refuses_a_type_it_would_sign_wrongly() {
        let server = MockServer::start().await;
        let sha = crate::crypto::sha256_hex(b"t");
        let err = s3(&server).put_typed(&sha, b"t".to_vec(), "text/plain;  charset=utf-8").await.unwrap_err();
        assert_eq!(err.status, Some(400));
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[test]
    fn blossom_delete_auth_is_bound_to_the_server_and_short() {
        let b = BlossomBackend::new("https://Media.Example:8443/base/", Keys::generate()).unwrap();
        let decode = |h: String| -> Event {
            let json = B64.decode(h.strip_prefix("Nostr ").unwrap()).unwrap();
            serde_json::from_slice(&json).unwrap()
        };
        let del = decode(b.auth_header("delete", &"cd".repeat(32)).unwrap());
        del.verify().unwrap();
        let servers: Vec<String> =
            del.tags.iter().filter(|t| t.kind() == "server").filter_map(|t| t.as_slice().get(1).cloned()).collect();
        assert_eq!(servers, vec!["media.example".to_string()], "the host of the base, nothing else");
        let exp: i64 = tag(&del, "expiration").unwrap().parse().unwrap();
        assert!(exp <= now() + 60, "a delete token lives a minute");
        assert!(exp > now());

        let up = decode(b.auth_header("upload", &"cd".repeat(32)).unwrap());
        assert!(tag(&up, "server").is_none(), "uploads stay unbound");
        let exp: i64 = tag(&up, "expiration").unwrap().parse().unwrap();
        assert!(exp >= now() + 299);
    }

    // ─── Request shapes against a local server ──────────────────────────────

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request as Seen, ResponseTemplate};

    fn header(r: &Seen, name: &str) -> String {
        r.headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string()
    }

    /// The kind-24242 event of an `Authorization: Nostr …` header, verified.
    fn auth_event(r: &Seen) -> Event {
        let h = header(r, "authorization");
        let json = B64.decode(h.strip_prefix("Nostr ").expect("nostr auth")).unwrap();
        let ev: Event = serde_json::from_slice(&json).unwrap();
        ev.verify().unwrap();
        assert_eq!(ev.kind.as_u16(), 24242);
        ev
    }

    fn tag(ev: &Event, k: &str) -> Option<String> {
        ev.tags.iter().find(|t| t.kind() == k).and_then(|t| t.as_slice().get(1).cloned())
    }

    fn s3(server: &MockServer) -> S3Backend {
        S3Backend::new(S3Config {
            endpoint: server.uri(),
            bucket: "avatars".into(),
            region: "eu-1".into(),
            access_key: "AK".into(),
            secret_key: "SK".into(),
        })
        .unwrap()
    }

    #[tokio::test]
    async fn s3_put_typed_signs_the_content_type() {
        let server = MockServer::start().await;
        let body = b"\xff\xd8 a jpeg".to_vec();
        let sha = crate::crypto::sha256_hex(&body);
        Mock::given(method("PUT")).and(path(format!("/avatars/{sha}"))).respond_with(ResponseTemplate::new(200)).mount(&server).await;
        s3(&server).put_typed(&sha, body.clone(), "image/jpeg").await.unwrap();

        let seen = server.received_requests().await.unwrap();
        assert_eq!(seen.len(), 1);
        let r = &seen[0];
        assert_eq!(header(r, "content-type"), "image/jpeg");
        assert_eq!(header(r, "x-amz-content-sha256"), sha);
        assert_eq!(r.body, body);
        let auth = header(r, "authorization");
        assert!(auth.starts_with("AWS4-HMAC-SHA256 Credential=AK/"), "{auth}");
        assert!(auth.contains("/eu-1/s3/aws4_request"), "{auth}");
        let signed = auth.split("SignedHeaders=").nth(1).unwrap().split(',').next().unwrap();
        assert!(signed.split(';').any(|h| h == "content-type"), "the type is signed: {signed}");
    }

    #[tokio::test]
    async fn s3_plain_put_stays_octet_stream() {
        let server = MockServer::start().await;
        let sha = crate::crypto::sha256_hex(b"chunk");
        Mock::given(method("PUT")).respond_with(ResponseTemplate::new(200)).mount(&server).await;
        s3(&server).put(&sha, b"chunk".to_vec()).await.unwrap();
        let seen = server.received_requests().await.unwrap();
        assert_eq!(header(&seen[0], "content-type"), "application/octet-stream");
    }

    #[tokio::test]
    async fn s3_delete_accepts_gone_and_reports_refusal() {
        let server = MockServer::start().await;
        let (a, b, c) = ("a".repeat(64), "b".repeat(64), "c".repeat(64));
        Mock::given(method("DELETE")).and(path(format!("/avatars/{a}"))).respond_with(ResponseTemplate::new(204)).mount(&server).await;
        Mock::given(method("DELETE")).and(path(format!("/avatars/{b}"))).respond_with(ResponseTemplate::new(404)).mount(&server).await;
        Mock::given(method("DELETE"))
            .and(path(format!("/avatars/{c}")))
            .respond_with(ResponseTemplate::new(403).set_body_string("AccessDenied"))
            .mount(&server)
            .await;
        let backend = s3(&server);
        backend.delete(&a).await.unwrap();
        backend.delete(&b).await.unwrap();
        let err = backend.delete(&c).await.unwrap_err();
        assert_eq!(err.status, Some(403));
        assert_eq!(err.code(), "err.auth_failed");
        assert!(err.message.contains("AccessDenied"));

        let seen = server.received_requests().await.unwrap();
        assert_eq!(seen.len(), 3);
        for r in &seen {
            assert_eq!(r.method.as_str(), "DELETE");
            assert!(r.body.is_empty());
            assert_eq!(header(r, "x-amz-content-sha256"), sigv4::EMPTY_SHA256);
            assert!(header(r, "authorization").starts_with("AWS4-HMAC-SHA256 "));
        }
    }

    #[tokio::test]
    async fn a_bad_content_type_never_reaches_the_network() {
        let server = MockServer::start().await;
        let sha = crate::crypto::sha256_hex(b"x");
        let err = s3(&server).put_typed(&sha, b"x".to_vec(), "text/html\r\nx: y").await.unwrap_err();
        assert_eq!(err.status, Some(400));
        let blossom = BlossomBackend::new(&server.uri(), Keys::generate()).unwrap();
        assert!(blossom.put_typed(&sha, b"x".to_vec(), "html").await.is_err());
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn blossom_put_typed_sends_the_type_and_an_upload_auth() {
        let server = MockServer::start().await;
        let keys = Keys::generate();
        let body = b"\x89PNG an image".to_vec();
        let sha = crate::crypto::sha256_hex(&body);
        Mock::given(method("PUT")).and(path("/upload")).respond_with(ResponseTemplate::new(200)).mount(&server).await;
        BlossomBackend::new(&server.uri(), keys.clone()).unwrap().put_typed(&sha, body.clone(), "image/png").await.unwrap();

        let seen = server.received_requests().await.unwrap();
        assert_eq!(seen.len(), 1);
        let r = &seen[0];
        assert_eq!(header(r, "content-type"), "image/png");
        assert_eq!(header(r, "x-sha-256"), sha);
        assert_eq!(r.body, body);
        let ev = auth_event(r);
        assert_eq!(ev.pubkey, keys.public_key());
        assert_eq!(tag(&ev, "t").as_deref(), Some("upload"));
        assert_eq!(tag(&ev, "x"), Some(sha));
    }

    #[tokio::test]
    async fn blossom_delete_is_authorised_for_that_blob_only() {
        let server = MockServer::start().await;
        let keys = Keys::generate();
        let (sha, gone, locked) = ("d".repeat(64), "e".repeat(64), "f".repeat(64));
        Mock::given(method("DELETE")).and(path(format!("/{sha}"))).respond_with(ResponseTemplate::new(200)).mount(&server).await;
        Mock::given(method("DELETE")).and(path(format!("/{gone}"))).respond_with(ResponseTemplate::new(404)).mount(&server).await;
        Mock::given(method("DELETE")).and(path(format!("/{locked}"))).respond_with(ResponseTemplate::new(401)).mount(&server).await;
        let backend = BlossomBackend::new(&format!("{}/", server.uri()), keys.clone()).unwrap();
        backend.delete(&sha).await.unwrap();
        backend.delete(&gone).await.unwrap();
        assert_eq!(backend.delete(&locked).await.unwrap_err().code(), "err.auth_failed");

        let seen = server.received_requests().await.unwrap();
        assert_eq!(seen.len(), 3);
        let r = &seen[0];
        assert_eq!(r.method.as_str(), "DELETE");
        assert_eq!(r.url.path(), format!("/{sha}"));
        let ev = auth_event(r);
        assert_eq!(ev.pubkey, keys.public_key());
        assert_eq!(tag(&ev, "t").as_deref(), Some("delete"));
        assert_eq!(tag(&ev, "x"), Some(sha.clone()));
        assert_eq!(tag(&ev, "server").as_deref(), Some("127.0.0.1"), "bound to the server it is sent to");
        assert_eq!(ev.content, format!("delete {sha}"));
        assert_eq!(tag(&auth_event(&seen[1]), "x"), Some(gone));
    }

    /// A chunk of a few pieces, not a multiple of the piece size.
    fn chunk() -> Vec<u8> {
        (0..PIECE * 3 + 1234).map(|i| (i * 31 % 251) as u8).collect()
    }

    #[tokio::test]
    async fn s3_counted_put_streams_with_a_length_and_the_signed_hash() {
        let server = MockServer::start().await;
        let body = chunk();
        let sha = crate::crypto::sha256_hex(&body);
        Mock::given(method("PUT")).and(path(format!("/avatars/{sha}"))).respond_with(ResponseTemplate::new(200)).mount(&server).await;
        let sent = Arc::new(AtomicU64::new(0));
        s3(&server).put_counted(&sha, Bytes::from(body.clone()), sent.clone()).await.unwrap();
        assert_eq!(sent.load(Ordering::SeqCst), body.len() as u64, "every byte counted once");

        let seen = server.received_requests().await.unwrap();
        let r = &seen[0];
        assert_eq!(r.body, body);
        assert_eq!(header(r, "content-length"), body.len().to_string());
        assert!(header(r, "transfer-encoding").is_empty(), "not chunked: stores refuse that");
        assert_eq!(header(r, "x-amz-content-sha256"), sha);
        assert_eq!(header(r, "content-type"), "application/octet-stream");
        let signed = header(r, "authorization");
        let signed = signed.split("SignedHeaders=").nth(1).unwrap().split(',').next().unwrap();
        assert!(signed.split(';').any(|h| h == "x-amz-content-sha256"), "{signed}");
    }

    #[tokio::test]
    async fn blossom_counted_put_streams_with_a_length() {
        let server = MockServer::start().await;
        let body = chunk();
        let sha = crate::crypto::sha256_hex(&body);
        Mock::given(method("PUT")).and(path("/upload")).respond_with(ResponseTemplate::new(200)).mount(&server).await;
        let sent = Arc::new(AtomicU64::new(0));
        let b = BlossomBackend::new(&server.uri(), Keys::generate()).unwrap();
        b.put_counted(&sha, Bytes::from(body.clone()), sent.clone()).await.unwrap();
        assert_eq!(sent.load(Ordering::SeqCst), body.len() as u64);
        let seen = server.received_requests().await.unwrap();
        let r = &seen[0];
        assert_eq!(r.body, body);
        assert_eq!(header(r, "content-length"), body.len().to_string());
        assert!(header(r, "transfer-encoding").is_empty());
        assert_eq!(header(r, "x-sha-256"), sha);
        assert_eq!(tag(&auth_event(r), "x"), Some(sha));
    }

    #[tokio::test]
    async fn counted_put_of_a_plain_backend_counts_on_success() {
        let m = MemoryBackend::new("mem://c");
        let sha = crate::crypto::sha256_hex(b"abc");
        let sent = Arc::new(AtomicU64::new(0));
        m.put_counted(&sha, Bytes::from_static(b"abc"), sent.clone()).await.unwrap();
        assert_eq!(sent.load(Ordering::SeqCst), 3);
        *m.reject_puts.lock().unwrap() = true;
        let e = m.put_counted(&sha, Bytes::from_static(b"abc"), sent.clone()).await.unwrap_err();
        assert!(!e.is_retryable());
        assert_eq!(sent.load(Ordering::SeqCst), 3, "a refused put adds nothing");
    }
}
