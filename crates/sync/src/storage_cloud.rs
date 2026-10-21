// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The backend `cloud`: the vault on vstore, the storage of Veydan Cloud
//! (services/cloud/spec/vstore.md §5, §14). Every request is signed with
//! NIP-98 by a [`RequestSigner`] the caller passes in, so this crate knows
//! neither the messenger nor the account. The hosts are the `public_url`s
//! of `storage[]` in the manifest (`https://media-eu.example.com/vault`):
//! the first that answers is kept for the session and another is tried only
//! when the transport fails, since two hosts share neither the memory of
//! replays nor the counters of the quota (§2, §9); the host so chosen
//! outlives the instance, which sync-host builds anew for every cycle. The
//! requests go by a [`VaultTransport`]: the module `cloud` gives the
//! messenger's, with its bridges, as for the media (§14).

use crate::storage::{http_client, is_noise_key, Storage};
use crate::{sha256_hex, Result, SyncError};
use async_trait::async_trait;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, DATE, ETAG, RETRY_AFTER};
use reqwest::{Client, Method, StatusCode};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The set `S3_ENCODE` of `storage_s3.rs`, which vstore.md §4 makes the
/// canonical form of a key in a URL: all but A-Z a-z 0-9 - _ . ~
const KEY_ENCODE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// Transport retries on one host, as `send_retry` does them.
const TRANSPORT_PAUSES: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(3)];

/// The longest `Retry-After` the client waits once (`no_table` asks 30 s).
const MAX_RETRY_AFTER: Duration = Duration::from_secs(30);

/// The pause after a `429`/`503` that names no `Retry-After`.
const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(2);

/// A clock further off than this is corrected by the `Date` of an answer
/// without a body (a `HEAD`): the window of NIP-98 is 60 s.
const CLOCK_WINDOW_SECS: i64 = 60;

/// What a [`RequestSigner`] returns.
pub type SignFuture<'a> = Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>>;

/// Signs one request with the key of the identity, without giving the key
/// out: the module `cloud` implements it over the messenger's `Signer`.
pub trait RequestSigner: Send + Sync {
    /// The value of `Authorization`, `Nostr <base64 of the event>`, for an
    /// event of kind 27235 made at `created_at` with the tags `u` (exactly
    /// `url`), `method` and, for a body, `payload` (its sha256).
    fn authorization<'a>(
        &'a self,
        method: &'a str,
        url: &'a str,
        payload_sha256: Option<[u8; 32]>,
        created_at: u64,
    ) -> SignFuture<'a>;
}

/// One request of [`CloudStorage`] as it goes on the wire.
pub struct VaultRequest<'a> {
    /// `GET`, `HEAD`, `PUT` or `DELETE`.
    pub method: &'a str,
    /// The whole URL: the tag `u` of its signature.
    pub url: &'a str,
    /// The headers, `authorization` among them; names in lower case.
    pub headers: Vec<(&'static str, String)>,
    /// The body of a `PUT`.
    pub body: Option<&'a [u8]>,
}

/// The answer of a host read to its end; header names in lower case.
pub struct VaultAnswer {
    /// The HTTP status.
    pub status: u16,
    /// The headers of the answer.
    pub headers: Vec<(String, String)>,
    /// The body of the answer.
    pub body: Vec<u8>,
}

/// Why a request got no answer from one host.
#[derive(Debug)]
pub enum SendFailure {
    /// The network, or a gateway in front of vstore: another host may answer.
    Host(String),
    /// Anything else: the same on every host.
    Other(SyncError),
}

impl From<SyncError> for SendFailure {
    fn from(e: SyncError) -> Self {
        SendFailure::Other(e)
    }
}

/// What a [`VaultTransport`] returns.
pub type TransportFuture<'a> =
    Pin<Box<dyn Future<Output = std::result::Result<VaultAnswer, SendFailure>> + Send + 'a>>;

/// The way of the requests to the hosts. Direct by default; the module
/// `cloud` gives the messenger's, which goes through a bridge wherever the
/// media do (vstore.md §14, plan 4.8).
pub trait VaultTransport: Send + Sync {
    /// Sends one request; a failure of the network is [`SendFailure::Host`].
    fn send<'a>(&'a self, request: VaultRequest<'a>) -> TransportFuture<'a>;
}

/// [`VaultTransport`] straight to the host, by the client of the other
/// backends.
struct Direct {
    client: Client,
}

impl VaultTransport for Direct {
    fn send<'a>(&'a self, request: VaultRequest<'a>) -> TransportFuture<'a> {
        Box::pin(async move {
            let method = Method::from_bytes(request.method.as_bytes())
                .map_err(|e| SyncError::Storage(format!("method {}: {e}", request.method)))?;
            // A list is a GET of `/v1/o` with a prefix; a blob is `/v1/o/<key>`.
            let short = method == Method::HEAD || request.url.contains("/v1/o?");
            let mut builder = self.client.request(method, request.url);
            if short {
                builder = builder.timeout(crate::storage::METADATA_TIMEOUT);
            }
            for (name, value) in &request.headers {
                builder = builder.header(*name, value.as_str());
            }
            if let Some(body) = request.body {
                builder = builder.body(body.to_vec());
            }
            let resp = builder
                .send()
                .await
                .map_err(|e| SendFailure::Host(crate::reqwest_msg(&e)))?;
            let status = resp.status().as_u16();
            let headers = resp
                .headers()
                .iter()
                .filter_map(|(n, v)| Some((n.as_str().to_string(), v.to_str().ok()?.to_string())))
                .collect();
            let body = resp
                .bytes()
                .await
                .map_err(|e| SendFailure::Host(crate::reqwest_msg(&e)))?;
            Ok(VaultAnswer {
                status,
                headers,
                body: body.to_vec(),
            })
        })
    }
}

/// The hosts of the session across the instances of the process: sync-host
/// builds a storage per cycle, and §14 keeps the host until the transport
/// fails. A few lists of hosts, the last used last.
static SESSION_HOSTS: Mutex<Vec<(Vec<String>, usize)>> = Mutex::new(Vec::new());

/// Lists of hosts the process remembers its host for.
const SESSION_LISTS: usize = 8;

/// The host the last storage over `hosts` ended on; the first otherwise.
fn session_host(hosts: &[String]) -> usize {
    let lists = SESSION_HOSTS.lock().unwrap_or_else(|e| e.into_inner());
    lists
        .iter()
        .find(|(list, _)| list.as_slice() == hosts)
        .map(|(_, i)| *i)
        .unwrap_or(0)
}

/// Keeps `i` as the host of the session for `hosts`.
fn remember_host(hosts: &[String], i: usize) {
    let mut lists = SESSION_HOSTS.lock().unwrap_or_else(|e| e.into_inner());
    lists.retain(|(list, _)| list.as_slice() != hosts);
    if lists.len() >= SESSION_LISTS {
        lists.remove(0);
    }
    lists.push((hosts.to_vec(), i));
}

/// The body of an error of `/vault/v1/` (vstore.md §11).
#[derive(Debug, Default, Deserialize)]
struct ErrorBody {
    #[serde(default)]
    error: ErrorFields,
    #[serde(default)]
    request_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ErrorFields {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    pool: Option<String>,
    #[serde(default)]
    used: Option<u64>,
    #[serde(default)]
    limit: Option<u64>,
    #[serde(default)]
    server_time: Option<i64>,
}

/// One page of `GET /vault/v1/o` (§5.1).
#[derive(Debug, Deserialize)]
struct ListPage {
    keys: Vec<String>,
    next: Option<String>,
}

/// An answer read to its end.
struct Answer {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Answer {
    /// The answer of a transport in the types of reqwest; a header that
    /// does not parse is dropped.
    fn of(answer: VaultAnswer) -> std::result::Result<Self, SendFailure> {
        let status = StatusCode::from_u16(answer.status)
            .map_err(|_| SendFailure::Host(format!("the status {} is not HTTP", answer.status)))?;
        let mut headers = HeaderMap::new();
        for (name, value) in &answer.headers {
            if let (Ok(n), Ok(v)) = (
                HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                headers.append(n, v);
            }
        }
        Ok(Self {
            status,
            headers,
            body: answer.body,
        })
    }

    fn error(&self) -> ErrorBody {
        serde_json::from_slice(&self.body).unwrap_or_default()
    }

    fn retry_after(&self) -> Duration {
        self.headers
            .get(RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_RETRY_AFTER)
    }

    /// The time of the server: `server_time` of the body, else `Date`.
    fn server_time(&self) -> Option<i64> {
        self.error().error.server_time.or_else(|| {
            let date = self.headers.get(DATE)?.to_str().ok()?;
            chrono::DateTime::parse_from_rfc2822(date)
                .ok()
                .map(|t| t.timestamp())
        })
    }
}

/// One request: the method, the path after the `public_url` with its query,
/// the body and whether it is written only when absent.
struct Request<'a> {
    method: Method,
    path: String,
    body: Option<&'a [u8]>,
    if_none_match: bool,
    what: &'a str,
}

/// [`Storage`] over `/vault/v1/` of vstore (vstore.md §14).
pub struct CloudStorage {
    hosts: Vec<String>,
    signer: Arc<dyn RequestSigner>,
    transport: Arc<dyn VaultTransport>,
    /// The host of the session: an index into `hosts`.
    host: AtomicUsize,
    /// Seconds to add to the clock of the device, learnt from `auth_expired`.
    clock_offset: AtomicI64,
    transport_pauses: Vec<Duration>,
    max_retry_after: Duration,
}

/// Unix seconds of the device; 0 before 1970, which the server refuses.
fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A key or a prefix as it goes in the URL: each segment encoded, `/` kept.
fn encode_key(key: &str) -> String {
    key.split('/')
        .map(|seg| utf8_percent_encode(seg, KEY_ENCODE).to_string())
        .collect::<Vec<_>>()
        .join("/")
}

/// A host as the client keeps it: https without userinfo (plain http only
/// to the loopback, for tests), no trailing `/`.
fn normalize_host(host: &str) -> Result<String> {
    let host = host.trim().trim_end_matches('/');
    let parsed = url::Url::parse(host)
        .map_err(|e| SyncError::Storage(format!("Veydan Cloud host {host}: {e}")))?;
    let loopback = matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    let scheme_ok = match parsed.scheme() {
        "https" => true,
        "http" => loopback,
        _ => false,
    };
    if !scheme_ok || parsed.host_str().is_none() {
        return Err(SyncError::Storage(format!(
            "Veydan Cloud host {host} is not an https URL"
        )));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(SyncError::Storage(format!(
            "Veydan Cloud host {host} carries a user name"
        )));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(SyncError::Storage(format!(
            "Veydan Cloud host {host} has a query"
        )));
    }
    Ok(host.to_string())
}

impl CloudStorage {
    /// A client for `hosts` in the order of the manifest (priority first),
    /// signing with `signer`. No host is an error: the backend has nowhere
    /// to go.
    pub fn new(hosts: &[String], signer: Arc<dyn RequestSigner>) -> Result<Self> {
        let mut list = Vec::with_capacity(hosts.len());
        for host in hosts {
            let host = normalize_host(host)?;
            if !list.contains(&host) {
                list.push(host);
            }
        }
        if list.is_empty() {
            return Err(SyncError::Storage(
                "no host of Veydan Cloud is known on this device".into(),
            ));
        }
        let start = session_host(&list) % list.len();
        Ok(Self {
            hosts: list,
            signer,
            transport: Arc::new(Direct {
                client: http_client()?,
            }),
            host: AtomicUsize::new(start),
            clock_offset: AtomicI64::new(0),
            transport_pauses: TRANSPORT_PAUSES.to_vec(),
            max_retry_after: MAX_RETRY_AFTER,
        })
    }

    /// The same storage, its requests sent by `transport` instead of directly.
    pub fn with_transport(mut self, transport: Arc<dyn VaultTransport>) -> Self {
        self.transport = transport;
        self
    }

    /// The hosts, in the order they are tried.
    pub fn hosts(&self) -> &[String] {
        &self.hosts
    }

    /// The host of the session.
    pub fn current_host(&self) -> &str {
        let i = self.host.load(Ordering::Relaxed) % self.hosts.len();
        &self.hosts[i]
    }

    /// The request on the host of the session, then on the others in turn
    /// while the transport fails; the host that answered stays.
    async fn send(&self, req: &Request<'_>) -> Result<Answer> {
        let n = self.hosts.len();
        let start = self.host.load(Ordering::Relaxed) % n;
        let mut failures = Vec::new();
        for step in 0..n {
            let i = (start + step) % n;
            match self.on_host(&self.hosts[i], req).await {
                Ok(answer) => {
                    if step > 0 {
                        self.host.store(i, Ordering::Relaxed);
                        remember_host(&self.hosts, i);
                    }
                    return Ok(answer);
                }
                Err(SendFailure::Host(e)) => failures.push(format!("{}: {e}", self.hosts[i])),
                Err(SendFailure::Other(e)) => return Err(e),
            }
        }
        Err(SyncError::Storage(format!(
            "{}: no host of Veydan Cloud answered ({})",
            req.what,
            failures.join("; ")
        )))
    }

    /// One host: a clock off by more than the window is corrected and the
    /// request sent again once; `429`/`503` wait their `Retry-After` once.
    async fn on_host(
        &self,
        host: &str,
        req: &Request<'_>,
    ) -> std::result::Result<Answer, SendFailure> {
        let mut clock_retry = true;
        let mut busy_retry = true;
        loop {
            let answer = self.exchange(host, req).await?;
            let status = answer.status;
            if status == StatusCode::UNAUTHORIZED && clock_retry && self.correct_clock(&answer) {
                clock_retry = false;
                continue;
            }
            let code = answer.error().error.code;
            if matches!(
                status,
                StatusCode::BAD_GATEWAY
                    | StatusCode::SERVICE_UNAVAILABLE
                    | StatusCode::GATEWAY_TIMEOUT
            ) && code.is_empty()
            {
                // Not vstore's answer (§11 always has a code): its gateway.
                return Err(SendFailure::Host(format!("{status} from the gateway")));
            }
            if matches!(
                status,
                StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE
            ) && busy_retry
            {
                busy_retry = false;
                tokio::time::sleep(answer.retry_after().min(self.max_retry_after)).await;
                continue;
            }
            return Ok(answer);
        }
    }

    /// Learn the server's clock from an answer `auth_expired` (or, with no
    /// body, a `Date` beyond the window); true when it changed the offset.
    fn correct_clock(&self, answer: &Answer) -> bool {
        let code = answer.error().error.code;
        let Some(server) = answer.server_time() else {
            return false;
        };
        let offset = server - unix_now();
        let ours = self.clock_offset.load(Ordering::Relaxed);
        let expired = code == "auth_expired"
            || (code.is_empty() && (offset - ours).abs() > CLOCK_WINDOW_SECS);
        if !expired || offset == ours {
            return false;
        }
        self.clock_offset.store(offset, Ordering::Relaxed);
        true
    }

    /// Sign and send, again with a new signature after a transport error:
    /// a signature sent once is spent (`auth_replay`).
    async fn exchange(
        &self,
        host: &str,
        req: &Request<'_>,
    ) -> std::result::Result<Answer, SendFailure> {
        let url = format!("{host}{}", req.path);
        let payload = req
            .body
            .filter(|b| !b.is_empty())
            .map(|b| -> [u8; 32] { Sha256::digest(b).into() });
        let mut last = String::new();
        for attempt in 0..=self.transport_pauses.len() {
            let created_at = (unix_now() + self.clock_offset.load(Ordering::Relaxed)).max(0) as u64;
            let auth = self
                .signer
                .authorization(req.method.as_str(), &url, payload, created_at)
                .await?;
            let mut headers = vec![("authorization", auth)];
            if req.body.is_some() {
                headers.push(("content-type", "application/octet-stream".into()));
            }
            if req.if_none_match {
                headers.push(("if-none-match", "*".into()));
            }
            let request = VaultRequest {
                method: req.method.as_str(),
                url: &url,
                headers,
                body: req.body,
            };
            match self.transport.send(request).await {
                Ok(answer) => return Answer::of(answer),
                Err(SendFailure::Host(e)) => last = e,
                Err(other) => return Err(other),
            }
            if let Some(pause) = self.transport_pauses.get(attempt) {
                tokio::time::sleep(*pause).await;
            }
        }
        Err(SendFailure::Host(last))
    }

    async fn object(&self, method: Method, key: &str, what: &str) -> Result<Answer> {
        self.send(&Request {
            method,
            path: format!("/v1/o/{}", encode_key(key)),
            body: None,
            if_none_match: false,
            what,
        })
        .await
    }

    async fn write(&self, key: &str, data: &[u8], if_none_match: bool) -> Result<Answer> {
        self.send(&Request {
            method: Method::PUT,
            path: format!("/v1/o/{}", encode_key(key)),
            body: Some(data),
            if_none_match,
            what: "put",
        })
        .await
    }
}

/// An answer that is not the one the method wants, in the words of §11:
/// `quota` is [`SyncError::Quota`]; 401 and 403 are `Auth` but for
/// `read_only`, which is the wall of the plan and so `Storage`.
fn fail(answer: &Answer, what: &str) -> SyncError {
    let body = answer.error();
    let e = body.error;
    if answer.status == StatusCode::PAYLOAD_TOO_LARGE && e.code == "quota" {
        return SyncError::Quota {
            pool: e.pool.unwrap_or_default(),
            used: e.used.unwrap_or_default(),
            limit: e.limit.unwrap_or_default(),
        };
    }
    let mut msg = format!("{what}: {}", answer.status);
    if !e.code.is_empty() {
        msg.push_str(&format!(" {}", e.code));
    }
    if !e.message.is_empty() {
        msg.push_str(&format!(
            ": {}",
            e.message.chars().take(300).collect::<String>()
        ));
    }
    if let Some(id) = body.request_id {
        msg.push_str(&format!(" (request {id})"));
    }
    let auth = answer.status == StatusCode::UNAUTHORIZED
        || (answer.status == StatusCode::FORBIDDEN && e.code != "read_only");
    if auth {
        SyncError::Auth(msg)
    } else {
        SyncError::Storage(msg)
    }
}

/// The `ETag` of vstore is the sha256 of the body in quotes (§5): a body
/// cut short on the way does not pass.
fn check_etag(answer: &Answer, key: &str) -> Result<()> {
    let Some(etag) = answer.headers.get(ETAG).and_then(|v| v.to_str().ok()) else {
        return Ok(());
    };
    let etag = etag.trim().trim_start_matches("W/").trim_matches('"');
    if etag.eq_ignore_ascii_case(&sha256_hex(&answer.body)) {
        Ok(())
    } else {
        Err(SyncError::Storage(format!(
            "get {key}: the body does not match its ETag"
        )))
    }
}

#[async_trait]
impl Storage for CloudStorage {
    async fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let mut keys = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let mut path = format!("/v1/o?prefix={}", encode_key(prefix));
            if let Some(a) = &after {
                path.push_str(&format!("&after={}", encode_key(a)));
            }
            let answer = self
                .send(&Request {
                    method: Method::GET,
                    path,
                    body: None,
                    if_none_match: false,
                    what: "list",
                })
                .await?;
            if answer.status != StatusCode::OK {
                return Err(fail(&answer, "list"));
            }
            let page: ListPage = serde_json::from_slice(&answer.body)?;
            keys.extend(page.keys.into_iter().filter(|k| !is_noise_key(k)));
            match page.next {
                None => return Ok(keys),
                Some(next) if next.is_empty() || after.as_deref() >= Some(next.as_str()) => {
                    return Err(SyncError::Storage(format!(
                        "list {prefix}: the next page does not move on"
                    )))
                }
                Some(next) => after = Some(next),
            }
        }
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let answer = self.object(Method::GET, key, "get").await?;
        match answer.status {
            StatusCode::OK => {
                check_etag(&answer, key)?;
                Ok(Some(answer.body))
            }
            StatusCode::NOT_FOUND => Ok(None),
            _ => Err(fail(&answer, "get")),
        }
    }

    async fn put(&self, key: &str, data: &[u8]) -> Result<()> {
        let answer = self.write(key, data, false).await?;
        match answer.status {
            StatusCode::OK | StatusCode::CREATED => Ok(()),
            _ => Err(fail(&answer, "put")),
        }
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let answer = self.object(Method::DELETE, key, "delete").await?;
        match answer.status {
            StatusCode::OK | StatusCode::NO_CONTENT | StatusCode::NOT_FOUND => Ok(()),
            _ => Err(fail(&answer, "delete")),
        }
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        let answer = self.object(Method::HEAD, key, "head").await?;
        match answer.status {
            StatusCode::OK => Ok(true),
            StatusCode::NOT_FOUND => Ok(false),
            _ => Err(fail(&answer, "head")),
        }
    }

    async fn put_if_absent(&self, key: &str, data: &[u8]) -> Result<bool> {
        let answer = self.write(key, data, true).await?;
        match answer.status {
            StatusCode::OK | StatusCode::CREATED => Ok(true),
            StatusCode::PRECONDITION_FAILED => Ok(false),
            _ => Err(fail(&answer, "put")),
        }
    }
}

#[cfg(test)]
mod tests {
    //! `CloudStorage` against an in-process fake of `/vault/v1/` that checks
    //! what vstore checks: the tag `u` against the URL as it came, the
    //! method, the `payload`, the window of the clock and replays.

    use super::*;
    use axum::body::Bytes;
    use axum::extract::State;
    use axum::http::{HeaderMap as Headers, HeaderValue, Method as Verb, StatusCode as Code, Uri};
    use axum::response::{IntoResponse, Response};
    use axum::Router;
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    use percent_encoding::percent_decode_str;
    use serde_json::{json, Value};
    use std::collections::{BTreeMap, HashSet};
    use std::sync::Mutex;

    /// Signs with no key: the "event" is the JSON of its tags. The fake
    /// reads it back; the real signature is the messenger's, tested there.
    #[derive(Default)]
    struct TagSigner {
        n: AtomicUsize,
    }

    impl RequestSigner for TagSigner {
        fn authorization<'a>(
            &'a self,
            method: &'a str,
            url: &'a str,
            payload_sha256: Option<[u8; 32]>,
            created_at: u64,
        ) -> SignFuture<'a> {
            let id = self.n.fetch_add(1, Ordering::Relaxed);
            let event = json!({
                "id": id, "u": url, "method": method, "created_at": created_at,
                "payload": payload_sha256.map(hex::encode),
            });
            Box::pin(async move { Ok(format!("Nostr {}", B64.encode(event.to_string()))) })
        }
    }

    /// A signer the identity behind it refuses.
    struct Refusing;

    impl RequestSigner for Refusing {
        fn authorization<'a>(
            &'a self,
            _: &'a str,
            _: &'a str,
            _: Option<[u8; 32]>,
            _: u64,
        ) -> SignFuture<'a> {
            Box::pin(async { Err(SyncError::Auth("locked: the lock is closed".into())) })
        }
    }

    #[derive(Default)]
    struct Vault {
        /// The origin, `http://127.0.0.1:<port>`: `u` is it and the path as received.
        origin: String,
        objects: BTreeMap<String, Vec<u8>>,
        seen: HashSet<u64>,
        /// Keys per page of the list; 0 is 1000.
        page: usize,
        /// The pool `files`; 0 is no limit.
        files_limit: u64,
        /// The shared storage of the plan, every pool together
        /// (`storage.total_bytes`); 0 is none.
        total_limit: u64,
        /// The server's clock runs this far ahead of the device's.
        skew: i64,
        /// A gateway answers instead of vstore: `502` with no body.
        gateway_down: bool,
        /// The next request is `503 busy` with `Retry-After: 0`.
        busy_once: bool,
        /// A `GET` sends one byte less than its `ETag` says.
        cut_body: bool,
        /// `no_account`, `read_only`, or empty for an account in order.
        refuse: &'static str,
        /// Every request that reached the fake, `METHOD path?query`.
        log: Vec<String>,
    }

    type Shared = Arc<Mutex<Vault>>;

    fn error(status: Code, code: &str, extra: Value) -> Response {
        let mut fields = json!({ "code": code, "message": code });
        if let (Some(f), Some(e)) = (fields.as_object_mut(), extra.as_object()) {
            f.extend(e.clone());
        }
        let body = json!({ "error": fields, "request_id": "r1" });
        (
            status,
            [("content-type", "application/json")],
            body.to_string(),
        )
            .into_response()
    }

    fn decode(s: &str) -> String {
        percent_decode_str(s).decode_utf8_lossy().into_owned()
    }

    fn query(uri: &Uri, name: &str) -> Option<String> {
        uri.query()?.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            (k == name).then(|| decode(v))
        })
    }

    fn files_pool(key: &str) -> bool {
        key.starts_with("blobs/") || key.starts_with("large-files/")
    }

    /// What vstore would say about the `Authorization` of a request.
    fn check_auth(
        v: &mut Vault,
        method: &Verb,
        uri: &Uri,
        headers: &Headers,
        body: &[u8],
    ) -> Option<Response> {
        let now = unix_now() + v.skew;
        let expired = || {
            let mut r = error(
                Code::UNAUTHORIZED,
                "auth_expired",
                json!({ "server_time": now }),
            );
            if let Some(date) = chrono::DateTime::from_timestamp(now, 0) {
                let date = date.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
                if let Ok(value) = HeaderValue::from_str(&date) {
                    r.headers_mut().insert("date", value);
                }
            }
            r
        };
        let Some(auth) = headers.get("authorization").and_then(|h| h.to_str().ok()) else {
            return Some(error(Code::UNAUTHORIZED, "auth_missing", json!({})));
        };
        let event: Value = auth
            .strip_prefix("Nostr ")
            .and_then(|b| B64.decode(b).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let sent = format!(
            "{}{}",
            v.origin,
            uri.path_and_query().map(|p| p.as_str()).unwrap_or("")
        );
        if event["u"] != sent.as_str() {
            return Some(error(Code::UNAUTHORIZED, "auth_url_mismatch", json!({})));
        }
        if event["method"] != method.as_str() {
            return Some(error(Code::UNAUTHORIZED, "auth_invalid", json!({})));
        }
        let payload = (!body.is_empty()).then(|| sha256_hex(body));
        if event["payload"].as_str() != payload.as_deref() {
            return Some(error(Code::UNAUTHORIZED, "auth_invalid", json!({})));
        }
        let at = event["created_at"].as_i64().unwrap_or(0);
        if (at - now).abs() > 60 {
            return Some(expired());
        }
        let id = event["id"].as_u64().unwrap_or(u64::MAX);
        if !v.seen.insert(id) {
            return Some(error(Code::UNAUTHORIZED, "auth_replay", json!({})));
        }
        match v.refuse {
            "no_account" => Some(error(Code::FORBIDDEN, "no_account", json!({}))),
            "read_only" if *method == Verb::PUT => {
                Some(error(Code::FORBIDDEN, "read_only", json!({ "until": 1 })))
            }
            _ => None,
        }
    }

    async fn handle(
        State(shared): State<Shared>,
        method: Verb,
        uri: Uri,
        headers: Headers,
        body: Bytes,
    ) -> Response {
        let Ok(mut v) = shared.lock() else {
            return Code::INTERNAL_SERVER_ERROR.into_response();
        };
        v.log.push(format!(
            "{method} {}",
            uri.path_and_query().map(|p| p.as_str()).unwrap_or("")
        ));
        if v.gateway_down {
            return (Code::BAD_GATEWAY, "<html>502 Bad Gateway</html>").into_response();
        }
        if v.busy_once {
            v.busy_once = false;
            let mut r = error(Code::SERVICE_UNAVAILABLE, "busy", json!({}));
            r.headers_mut()
                .insert("retry-after", HeaderValue::from_static("0"));
            return r;
        }
        if let Some(refused) = check_auth(&mut v, &method, &uri, &headers, &body) {
            return refused;
        }
        let Some(path) = uri.path().strip_prefix("/vault/v1/") else {
            return error(Code::NOT_FOUND, "not_found", json!({}));
        };
        if path == "o" {
            let prefix = query(&uri, "prefix").unwrap_or_default();
            let after = query(&uri, "after");
            let page = if v.page == 0 { 1000 } else { v.page };
            let rest: Vec<&String> = v
                .objects
                .keys()
                .filter(|k| k.starts_with(&prefix) && after.as_ref().is_none_or(|a| *k > a))
                .collect();
            let keys: Vec<&String> = rest.iter().take(page).copied().collect();
            let next = (rest.len() > page)
                .then(|| keys.last().map(|k| k.to_string()))
                .flatten();
            let body = json!({ "keys": keys, "next": next });
            return (
                Code::OK,
                [("content-type", "application/json")],
                body.to_string(),
            )
                .into_response();
        }
        let Some(raw) = path.strip_prefix("o/") else {
            return error(Code::NOT_FOUND, "not_found", json!({}));
        };
        let key = raw.split('/').map(decode).collect::<Vec<_>>().join("/");
        match method {
            Verb::GET | Verb::HEAD => match v.objects.get(&key) {
                Some(data) => {
                    let etag = format!("\"{}\"", sha256_hex(data));
                    let mut data = data.clone();
                    if v.cut_body {
                        data.pop();
                    }
                    let data = if method == Verb::HEAD {
                        Vec::new()
                    } else {
                        data
                    };
                    (Code::OK, [("etag", etag)], data).into_response()
                }
                None => error(Code::NOT_FOUND, "not_found", json!({})),
            },
            Verb::PUT => {
                if body.is_empty() {
                    return error(Code::BAD_REQUEST, "empty_body", json!({}));
                }
                let old = v.objects.get(&key).map(|o| o.len() as u64);
                if old.is_some() && headers.get("if-none-match").is_some() {
                    return error(Code::PRECONDITION_FAILED, "precondition_failed", json!({}));
                }
                if files_pool(&key) && v.files_limit > 0 {
                    let used: u64 = v
                        .objects
                        .iter()
                        .filter(|(k, _)| files_pool(k))
                        .map(|(_, o)| o.len() as u64)
                        .sum();
                    if used - old.unwrap_or(0) + body.len() as u64 > v.files_limit {
                        let extra =
                            json!({ "pool": "files", "used": used, "limit": v.files_limit });
                        return error(Code::PAYLOAD_TOO_LARGE, "quota", extra);
                    }
                }
                if v.total_limit > 0 {
                    let used: u64 = v.objects.values().map(|o| o.len() as u64).sum();
                    if used - old.unwrap_or(0) + body.len() as u64 > v.total_limit {
                        let extra =
                            json!({ "pool": "total", "used": used, "limit": v.total_limit });
                        return error(Code::PAYLOAD_TOO_LARGE, "quota", extra);
                    }
                }
                v.objects.insert(key, body.to_vec());
                let status = if old.is_some() {
                    Code::OK
                } else {
                    Code::CREATED
                };
                (
                    status,
                    json!({ "etag": "x", "bytes": body.len() }).to_string(),
                )
                    .into_response()
            }
            Verb::DELETE => {
                v.objects.remove(&key);
                Code::NO_CONTENT.into_response()
            }
            _ => error(Code::BAD_REQUEST, "bad_key", json!({})),
        }
    }

    async fn serve(vault: Vault) -> (String, Shared) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let shared = Arc::new(Mutex::new(Vault {
            origin: format!("http://{addr}"),
            ..vault
        }));
        let app = Router::new().fallback(handle).with_state(shared.clone());
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{addr}/vault/"), shared)
    }

    fn storage(hosts: &[String]) -> CloudStorage {
        let mut s = CloudStorage::new(hosts, Arc::new(TagSigner::default())).expect("storage");
        s.transport_pauses = vec![Duration::ZERO; 2];
        s.max_retry_after = Duration::ZERO;
        s
    }

    fn requests(shared: &Shared) -> usize {
        shared.lock().expect("fake").log.len()
    }

    #[tokio::test]
    async fn objects_go_up_come_back_and_go_away() {
        let (host, shared) = serve(Vault::default()).await;
        let s = storage(&[host]);
        assert_eq!(
            s.current_host(),
            format!("{}/vault", shared.lock().unwrap().origin)
        );
        assert_eq!(s.get("manifest.json").await.unwrap(), None);
        assert!(!s.exists("manifest.json").await.unwrap());
        s.put("manifest.json", b"{\"v\":1}").await.unwrap();
        assert_eq!(
            s.get("manifest.json").await.unwrap().as_deref(),
            Some(&b"{\"v\":1}"[..])
        );
        assert!(s.exists("manifest.json").await.unwrap());
        // A key with what a URL must encode reaches the same object, and `u` matches.
        s.put("blobs/a b%+", b"odd").await.unwrap();
        assert!(shared.lock().unwrap().objects.contains_key("blobs/a b%+"));
        assert_eq!(
            s.get("blobs/a b%+").await.unwrap().as_deref(),
            Some(&b"odd"[..])
        );
        s.delete("manifest.json").await.unwrap();
        s.delete("manifest.json").await.unwrap();
        assert_eq!(s.get("manifest.json").await.unwrap(), None);
        assert!(shared
            .lock()
            .unwrap()
            .log
            .iter()
            .any(|l| l.starts_with("PUT /vault/v1/o/blobs/a%20b%25%2B")));
    }

    #[tokio::test]
    async fn the_list_follows_the_pages_and_drops_the_noise() {
        let mut objects = BTreeMap::new();
        for k in [
            "devices/0a/log/00000001.bin",
            "devices/0a/log/00000002.bin",
            "devices/0a/snapshot.bin",
            "devices/0b/log/00000001.bin",
            "devices/0b/.DS_Store",
            "devices/0b/log/00000002.bin.tmp",
            "blobs/ff",
        ] {
            objects.insert(k.to_string(), b"x".to_vec());
        }
        let (host, shared) = serve(Vault {
            objects,
            page: 2,
            ..Default::default()
        })
        .await;
        let s = storage(&[host]);
        let keys = s.list("devices/").await.unwrap();
        assert_eq!(
            keys,
            [
                "devices/0a/log/00000001.bin",
                "devices/0a/log/00000002.bin",
                "devices/0a/snapshot.bin",
                "devices/0b/log/00000001.bin",
            ]
        );
        // Six keys under the prefix, two a page: three requests.
        assert_eq!(requests(&shared), 3);
        assert!(shared.lock().unwrap().log[1].ends_with("&after=devices/0a/log/00000002.bin"));
        assert_eq!(s.list("").await.unwrap().len(), 5);
        assert!(s.list("nothing/").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_conditional_put_writes_once() {
        let (host, shared) = serve(Vault::default()).await;
        let s = storage(&[host]);
        assert!(s.put_if_absent("blobs/aa", b"first").await.unwrap());
        assert!(!s.put_if_absent("blobs/aa", b"second").await.unwrap());
        assert_eq!(shared.lock().unwrap().objects["blobs/aa"], b"first");
    }

    #[tokio::test]
    async fn a_full_pool_is_quota_and_the_text_still_goes() {
        let (host, _shared) = serve(Vault {
            files_limit: 10,
            ..Default::default()
        })
        .await;
        let s = storage(&[host]);
        s.put("blobs/small", b"12345678").await.unwrap();
        match s.put("blobs/big", b"12345").await {
            Err(SyncError::Quota { pool, used, limit }) => {
                assert_eq!((pool.as_str(), used, limit), ("files", 8, 10));
            }
            other => panic!("not quota: {other:?}"),
        }
        assert!(matches!(
            s.put_if_absent("large-files/v2/chunks/c1", b"12345").await,
            Err(SyncError::Quota { .. })
        ));
        // The journal is in the other pool.
        s.put("devices/0a/log/00000001.bin", b"1234567890abc")
            .await
            .unwrap();
        let e = SyncError::Quota {
            pool: "files".into(),
            used: 8,
            limit: 10,
        };
        assert_eq!(
            e.to_string(),
            "quota: the pool files is full (8 of 10 bytes)"
        );
    }

    /// A full shared storage (`413 quota`, pool `total`) holds the text as
    /// well: its word passes as the host wrote it.
    #[tokio::test]
    async fn a_full_total_is_quota_for_the_text_too() {
        let (host, _shared) = serve(Vault {
            total_limit: 12,
            ..Default::default()
        })
        .await;
        let s = storage(&[host]);
        s.put("blobs/small", b"12345678").await.unwrap();
        match s.put("devices/0a/log/00000001.bin", b"12345").await {
            Err(SyncError::Quota { pool, used, limit }) => {
                assert_eq!((pool.as_str(), used, limit), ("total", 8, 12));
            }
            other => panic!("not quota: {other:?}"),
        }
        assert!(matches!(
            s.put("blobs/other", b"12345").await,
            Err(SyncError::Quota { pool, .. }) if pool == "total"
        ));
        // What fits under the whole still goes.
        s.put("devices/0a/log/00000001.bin", b"1234").await.unwrap();
    }

    #[tokio::test]
    async fn refusals_are_auth_but_the_wall_of_the_plan() {
        let (host, _) = serve(Vault {
            refuse: "no_account",
            ..Default::default()
        })
        .await;
        match storage(&[host]).get("manifest.json").await {
            Err(SyncError::Auth(m)) => assert!(m.contains("no_account") && m.contains("r1"), "{m}"),
            other => panic!("{other:?}"),
        }
        let (host, _) = serve(Vault {
            refuse: "read_only",
            ..Default::default()
        })
        .await;
        let s = storage(&[host]);
        assert_eq!(s.get("manifest.json").await.unwrap(), None);
        match s.put("blobs/a", b"x").await {
            Err(SyncError::Storage(m)) => assert!(m.contains("read_only"), "{m}"),
            other => panic!("{other:?}"),
        }
        let (host, shared) = serve(Vault::default()).await;
        let refused = CloudStorage::new(&[host], Arc::new(Refusing)).unwrap();
        assert!(matches!(
            refused.get("manifest.json").await,
            Err(SyncError::Auth(_))
        ));
        assert_eq!(requests(&shared), 0, "nothing goes out unsigned");
    }

    #[tokio::test]
    async fn a_clock_off_is_corrected_once_and_kept() {
        let (host, shared) = serve(Vault {
            skew: 600,
            ..Default::default()
        })
        .await;
        let s = storage(&[host]);
        s.put("manifest.json", b"m").await.unwrap();
        assert_eq!(requests(&shared), 2, "refused, then sent again");
        let offset = s.clock_offset.load(Ordering::Relaxed);
        assert!((595..=605).contains(&offset), "{offset}");
        s.get("manifest.json").await.unwrap();
        assert!(s.exists("manifest.json").await.unwrap());
        assert_eq!(requests(&shared), 4, "the offset holds for the session");
        // A HEAD has no body: the `Date` of the answer corrects the clock.
        let (host, shared) = serve(Vault {
            skew: -900,
            ..Default::default()
        })
        .await;
        let s = storage(&[host]);
        assert!(!s.exists("manifest.json").await.unwrap());
        assert_eq!(requests(&shared), 2);
    }

    #[tokio::test]
    async fn the_next_host_answers_when_the_first_is_down_and_stays() {
        // A port nothing listens on.
        let dead = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}/vault", l.local_addr().unwrap())
        };
        let (gateway, gw) = serve(Vault {
            gateway_down: true,
            ..Default::default()
        })
        .await;
        let (live, shared) = serve(Vault::default()).await;
        let s = storage(&[dead, gateway, live.clone()]);
        s.put("manifest.json", b"m").await.unwrap();
        assert_eq!(s.current_host(), live.trim_end_matches('/'));
        assert_eq!(
            requests(&gw),
            1,
            "a gateway's 502 is not retried on its host"
        );
        assert_eq!(
            s.get("manifest.json").await.unwrap().as_deref(),
            Some(&b"m"[..])
        );
        s.delete("manifest.json").await.unwrap();
        assert_eq!(requests(&gw), 1, "the host of the session is kept");
        assert_eq!(requests(&shared), 3);
        // No host at all: the error names them.
        gw.lock().unwrap().log.clear();
        let s = storage(&[s.hosts()[1].clone()]);
        match s.get("manifest.json").await {
            Err(SyncError::Storage(m)) => {
                assert!(m.contains("no host of Veydan Cloud answered"), "{m}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn a_new_storage_starts_on_the_host_the_last_one_ended_on() {
        let dead = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}/vault", l.local_addr().unwrap())
        };
        let (live, shared) = serve(Vault::default()).await;
        let hosts = [dead, live.clone()];
        let first = storage(&hosts);
        first.put("manifest.json", b"m").await.unwrap();
        assert_eq!(first.current_host(), live.trim_end_matches('/'));
        // The next cycle builds the storage again: it starts where this ended.
        let second = storage(&hosts);
        assert_eq!(second.current_host(), live.trim_end_matches('/'));
        second.get("manifest.json").await.unwrap();
        assert_eq!(requests(&shared), 2);
        // Another list of hosts starts on its first.
        let other = storage(&[live.clone(), hosts[0].clone()]);
        assert_eq!(other.current_host(), live.trim_end_matches('/'));
    }

    /// A way of the requests that counts them and sends them directly.
    struct Counting {
        inner: Direct,
        sent: Mutex<Vec<String>>,
    }

    impl VaultTransport for Counting {
        fn send<'a>(&'a self, request: VaultRequest<'a>) -> TransportFuture<'a> {
            if let Ok(mut sent) = self.sent.lock() {
                sent.push(format!("{} {}", request.method, request.url));
            }
            self.inner.send(request)
        }
    }

    /// A way that reaches nothing, as a bridge that is down.
    struct Unreachable;

    impl VaultTransport for Unreachable {
        fn send<'a>(&'a self, _: VaultRequest<'a>) -> TransportFuture<'a> {
            Box::pin(async { Err(SendFailure::Host("the bridge is down".into())) })
        }
    }

    #[tokio::test]
    async fn every_request_goes_by_the_transport_given() {
        let (host, shared) = serve(Vault::default()).await;
        let way = Arc::new(Counting {
            inner: Direct {
                client: http_client().unwrap(),
            },
            sent: Mutex::new(Vec::new()),
        });
        let s = storage(std::slice::from_ref(&host)).with_transport(way.clone());
        s.put("blobs/aa", b"x").await.unwrap();
        assert!(s.exists("blobs/aa").await.unwrap());
        assert_eq!(s.list("blobs/").await.unwrap(), ["blobs/aa"]);
        let sent = way.sent.lock().unwrap().clone();
        assert_eq!(sent.len(), 3);
        assert_eq!(requests(&shared), 3);
        assert!(
            sent[0].starts_with(&format!("PUT {}v1/o/blobs/aa", host)),
            "{sent:?}"
        );
        // A way that fails is a failure of the transport, tried again.
        let s = storage(&[host]).with_transport(Arc::new(Unreachable));
        match s.get("blobs/aa").await {
            Err(SyncError::Storage(m)) => assert!(m.contains("the bridge is down"), "{m}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(requests(&shared), 3, "nothing went directly");
    }

    #[tokio::test]
    async fn busy_waits_once_and_a_cut_body_does_not_pass() {
        let mut objects = BTreeMap::new();
        objects.insert("blobs/aa".to_string(), b"hello".to_vec());
        let (host, shared) = serve(Vault {
            objects,
            busy_once: true,
            ..Default::default()
        })
        .await;
        let s = storage(&[host]);
        assert_eq!(
            s.get("blobs/aa").await.unwrap().as_deref(),
            Some(&b"hello"[..])
        );
        assert_eq!(requests(&shared), 2);
        shared.lock().unwrap().cut_body = true;
        match s.get("blobs/aa").await {
            Err(SyncError::Storage(m)) => assert!(m.contains("ETag"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn hosts_are_http_urls_without_a_trailing_slash() {
        let signer: Arc<dyn RequestSigner> = Arc::new(TagSigner::default());
        assert!(CloudStorage::new(&[], signer.clone()).is_err());
        assert!(
            CloudStorage::new(&["ftp://media.example.com/vault".into()], signer.clone()).is_err()
        );
        assert!(CloudStorage::new(
            &["https://media.example.com/vault?x=1".into()],
            signer.clone()
        )
        .is_err());
        // A signed request goes in clear text to the loopback only, never
        // to a URL with a user name.
        assert!(
            CloudStorage::new(&["http://media.example.com/vault".into()], signer.clone()).is_err()
        );
        assert!(CloudStorage::new(
            &["https://user@evil.example.com/vault".into()],
            signer.clone()
        )
        .is_err());
        assert!(CloudStorage::new(&["http://127.0.0.1:8080/vault".into()], signer.clone()).is_ok());
        let s = CloudStorage::new(
            &[
                "https://media-eu.example.com/vault/".into(),
                "https://media-eu.example.com/vault".into(),
                " https://media-us.example.com/vault ".into(),
            ],
            signer,
        )
        .unwrap();
        assert_eq!(
            s.hosts(),
            [
                "https://media-eu.example.com/vault",
                "https://media-us.example.com/vault"
            ]
        );
        assert_eq!(
            encode_key("devices/0a/log/00000001.bin"),
            "devices/0a/log/00000001.bin"
        );
        assert_eq!(encode_key("a b/c%d/"), "a%20b/c%25d/");
    }
}
