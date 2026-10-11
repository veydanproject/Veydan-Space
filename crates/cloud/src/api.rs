// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `CloudApi`: the client of Veydan Cloud the module offers through the
//! shell (services/cloud/spec/cloud-api.md §12) — to its own screens, to
//! the backend `cloud` of sync and to the modules that come later. One
//! place parses the errors and keeps the host and the server's clock, and
//! the screens are tested against a fake one.
//!
//! The types are the bodies of the contract: `/cloud/v1/` (sections 3-5)
//! and `GET /vault/v1/usage` (spec/vstore.md §6). Unknown fields are
//! ignored (2.1).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use veydan_shell::seams::{
    code, CloudError, CloudReach, Http, HttpRequest, HttpResponse, SeamFuture, Signer, TempKey,
    WireEvent,
};
use veydan_sync::storage::Storage;

use crate::nip98;

/// What a proof of work is for (3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Purpose {
    #[serde(rename = "activate")]
    Activate,
    #[serde(rename = "recovery.request")]
    RecoveryRequest,
}

/// `GET /cloud/v1/challenge` (3.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Challenge {
    pub challenge: String,
    pub d: u8,
    pub purpose: Purpose,
    pub expires_at: u64,
}

/// A solved challenge, as a request carries it (3.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pow {
    pub challenge: String,
    pub nonce: u64,
}

/// The state of an account in the answers (2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountState {
    Free,
    Paid,
    Grace,
    Suspended,
}

/// `GET /cloud/v1/account` (4.3), also the answer of 4.1 and 4.2.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub npub: String,
    pub state: AccountState,
    pub plan: String,
    pub until: Option<u64>,
    pub grace_until: Option<u64>,
    /// The effective limits of the plan, by their keys (`vault.files_bytes`, …).
    #[serde(default)]
    pub limits: BTreeMap<String, u64>,
    pub usage: Option<AccountUsage>,
    pub recovery: Option<RecoveryInfo>,
    pub last_seen_at: Option<u64>,
    pub server_time: Option<u64>,
    /// Only in the answer of `POST /cloud/v1/redeem` (4.2).
    pub subscription: Option<Subscription>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountUsage {
    pub as_of: Option<u64>,
    pub vault: Option<VaultUsage>,
    pub media: Option<MediaUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultUsage {
    pub files_bytes: u64,
    pub text_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaUsage {
    pub upload_today_bytes: u64,
    pub storage_bytes: u64,
}

/// The numbers of the recovery config (4.3, 5.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryInfo {
    pub configured: bool,
    pub k: Option<u8>,
    pub n: Option<u8>,
    pub generation: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscription {
    pub plan: String,
    pub since: u64,
    pub until: Option<u64>,
    pub source: String,
}

/// `DELETE /cloud/v1/account?confirm=DELETE` (4.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deleted {
    pub deleted_at: u64,
    pub receipt: String,
}

/// `GET /cloud/v1/plans` (4.5): the plans on offer, in the order the
/// server gives them (the owner's `sort`, then the id).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plans {
    pub plans: Vec<PlanOffer>,
    pub server_time: Option<u64>,
}

/// One plan of the catalog (4.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanOffer {
    /// The word [`Account::plan`] carries for an account on this plan.
    pub id: String,
    pub name: String,
    /// The plan an account starts on.
    pub free: bool,
    /// 0 for a plan without a term.
    pub period_days: u32,
    pub sort: i64,
    /// The effective limits, by the keys and in the numbers of
    /// [`Account::limits`] of an account on this plan.
    #[serde(default)]
    pub limits: BTreeMap<String, u64>,
    /// By currency, then by period; none for a plan that is not sold.
    #[serde(default)]
    pub prices: Vec<PlanPrice>,
}

/// One price of a plan (4.5), in the minor units of its currency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanPrice {
    pub currency: String,
    pub amount_minor: i64,
    pub period_days: u32,
}

/// `PUT /cloud/v1/recovery/config` (5.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryConfig {
    pub k: u8,
    pub n: u8,
    pub generation: u32,
    pub updated_at: u64,
}

/// A request of recovery: the answer of `POST …/recovery/request` (5.2,
/// with `k` and `n`) or of `GET …/recovery/request` (5.3, with `temp_pub`
/// and `state`, and `closed_at` once it is `closed`, `cancelled` or
/// `expired`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryRequest {
    pub request_id: String,
    pub temp_pub: Option<String>,
    pub state: Option<String>,
    pub k: Option<u8>,
    pub n: Option<u8>,
    pub created_at: u64,
    pub expires_at: u64,
    /// When the request stopped being `open` (5.3); the trusted person
    /// sees it for 24 hours after.
    pub closed_at: Option<u64>,
}

/// `POST /cloud/v1/recovery/cancel` (5.4): `cancelled`, or `expired` when
/// the request ran out before the owner said no.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cancelled {
    pub state: String,
    pub frozen_until: Option<u64>,
}

/// `POST /cloud/v1/recovery/close` (5.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Closed {
    pub state: String,
}

/// `GET /vault/v1/usage` of one host (spec/vstore.md §6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub as_of: u64,
    pub host: String,
    pub plan: String,
    pub state: AccountState,
    pub until: Option<u64>,
    pub table_serial: u64,
    pub max_object_bytes: u64,
    /// `text` and `files` (§9).
    pub pools: BTreeMap<String, Pool>,
    #[serde(default)]
    pub breakdown: Vec<UsageLine>,
    pub reconciled_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pool {
    pub used: u64,
    pub limit: u64,
    pub objects: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageLine {
    pub prefix: String,
    pub pool: String,
    pub bytes: u64,
    pub objects: u64,
}

/// The client of Veydan Cloud (cloud-api.md §12). Requests of the identity
/// are signed through the messenger's `Signer`, those of a procedure by its
/// key T; the hosts come from the manifest, one per session.
pub trait CloudApi: Send + Sync {
    /// `GET /cloud/v1/account` (4.3).
    fn account(&self) -> SeamFuture<'_, Account>;
    /// `POST /cloud/v1/activate` (4.1).
    fn activate(&self, pow: Option<Pow>) -> SeamFuture<'_, Account>;
    /// `POST /cloud/v1/redeem` (4.2).
    fn redeem<'a>(&'a self, key: &'a str) -> SeamFuture<'a, Account>;
    /// `DELETE /cloud/v1/account?confirm=DELETE` (4.4).
    fn delete(&self) -> SeamFuture<'_, Deleted>;
    /// `GET /cloud/v1/plans`, not signed (4.5): an identity without an
    /// account reads the catalog too.
    fn plans(&self) -> SeamFuture<'_, Plans>;
    /// `PUT /cloud/v1/recovery/config` (5.1).
    fn recovery_config_put(&self, k: u8, n: u8, generation: u32) -> SeamFuture<'_, RecoveryConfig>;
    /// `DELETE /cloud/v1/recovery/config` (5.1).
    fn recovery_config_delete(&self) -> SeamFuture<'_, ()>;
    /// `POST /cloud/v1/recovery/request`, signed by T (5.2); on
    /// `pow_insufficient` it takes a new challenge, works to `error.d` and
    /// tries once more (3.4).
    fn recovery_request<'a>(
        &'a self,
        npub: &'a str,
        t: &'a TempKey,
        pow: Pow,
    ) -> SeamFuture<'a, RecoveryRequest>;
    /// `GET /cloud/v1/recovery/request`, not signed by the identity (5.3).
    fn recovery_request_get<'a>(&'a self, npub: &'a str)
        -> SeamFuture<'a, Option<RecoveryRequest>>;
    /// `POST /cloud/v1/recovery/cancel` (5.4).
    fn recovery_cancel<'a>(&'a self, request_id: &'a str) -> SeamFuture<'a, Cancelled>;
    /// `POST /cloud/v1/recovery/close`, signed by T (5.5).
    fn recovery_close<'a>(&'a self, request_id: &'a str, t: &'a TempKey) -> SeamFuture<'a, Closed>;
    /// `GET /cloud/v1/challenge` (3.1); a new one for every `pow_insufficient`.
    fn challenge(&self, purpose: Purpose) -> SeamFuture<'_, Challenge>;
    /// The backend `CloudStorage` (spec/vstore.md §14) under `prefix`, on the
    /// host of `storage[]` chosen for the session.
    fn storage<'a>(&'a self, prefix: &'a str) -> SeamFuture<'a, Arc<dyn Storage>>;
    /// `GET /vault/v1/usage` of the same host (spec/vstore.md §6).
    fn usage(&self) -> SeamFuture<'_, Usage>;
}

/// The client of a product whose messenger offers no way to the hosts of
/// the project: every call fails with `no_host`.
pub struct Unconfigured;

fn no_host<'a, T: Send + 'a>() -> SeamFuture<'a, T> {
    Box::pin(async {
        Err(CloudError::new(
            code::NO_HOST,
            "no host of Veydan Cloud is known on this device",
        ))
    })
}

impl CloudApi for Unconfigured {
    fn account(&self) -> SeamFuture<'_, Account> {
        no_host()
    }
    fn activate(&self, _pow: Option<Pow>) -> SeamFuture<'_, Account> {
        no_host()
    }
    fn redeem<'a>(&'a self, _key: &'a str) -> SeamFuture<'a, Account> {
        no_host()
    }
    fn delete(&self) -> SeamFuture<'_, Deleted> {
        no_host()
    }
    fn plans(&self) -> SeamFuture<'_, Plans> {
        no_host()
    }
    fn recovery_config_put(
        &self,
        _k: u8,
        _n: u8,
        _generation: u32,
    ) -> SeamFuture<'_, RecoveryConfig> {
        no_host()
    }
    fn recovery_config_delete(&self) -> SeamFuture<'_, ()> {
        no_host()
    }
    fn recovery_request<'a>(
        &'a self,
        _npub: &'a str,
        _t: &'a TempKey,
        _pow: Pow,
    ) -> SeamFuture<'a, RecoveryRequest> {
        no_host()
    }
    fn recovery_request_get<'a>(
        &'a self,
        _npub: &'a str,
    ) -> SeamFuture<'a, Option<RecoveryRequest>> {
        no_host()
    }
    fn recovery_cancel<'a>(&'a self, _request_id: &'a str) -> SeamFuture<'a, Cancelled> {
        no_host()
    }
    fn recovery_close<'a>(
        &'a self,
        _request_id: &'a str,
        _t: &'a TempKey,
    ) -> SeamFuture<'a, Closed> {
        no_host()
    }
    fn challenge(&self, _purpose: Purpose) -> SeamFuture<'_, Challenge> {
        no_host()
    }
    fn storage<'a>(&'a self, _prefix: &'a str) -> SeamFuture<'a, Arc<dyn Storage>> {
        no_host()
    }
    fn usage(&self) -> SeamFuture<'_, Usage> {
        no_host()
    }
}

/// What a refusal of a server carries besides its word (section 9): the
/// difficulty of `pow_insufficient`, the clock of `auth_expired`, the wait
/// of `rate_limited`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub error: CloudError,
    /// `error.d` of `428 pow_insufficient`.
    pub d: Option<u8>,
    /// `error.server_time` of `401 auth_expired`.
    pub server_time: Option<u64>,
    /// The header `Retry-After` of `429` and `503`, in seconds.
    pub retry_after: Option<u64>,
}

impl From<CloudError> for Refusal {
    fn from(error: CloudError) -> Self {
        Self {
            error,
            d: None,
            server_time: None,
            retry_after: None,
        }
    }
}

#[derive(Deserialize)]
struct ErrorBody {
    error: ErrorFields,
    request_id: Option<String>,
}

#[derive(Deserialize)]
struct ErrorFields {
    code: String,
    #[serde(default)]
    message: String,
    d: Option<u8>,
    server_time: Option<u64>,
}

/// Unix seconds of this device.
pub(crate) fn now() -> Result<u64, CloudError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| CloudError::new(code::CRYPTO, "the clock of the device is before 1970"))
}

/// The refusal in an answer that is not a success (2.2): the body of the
/// contract when there is one, else a word from the status alone.
pub fn refusal_of(answer: &HttpResponse) -> Refusal {
    let retry_after = answer
        .header("retry-after")
        .and_then(|v| v.trim().parse().ok());
    let header_id = answer.header("x-request-id").map(str::to_string);
    let Ok(body) = serde_json::from_slice::<ErrorBody>(&answer.body) else {
        let word = match answer.status {
            401 => "auth_invalid",
            404 => "not_found",
            413 => "payload_too_large",
            429 => "rate_limited",
            _ => "internal",
        };
        let mut error = CloudError::new(word, format!("the server answered {}", answer.status));
        error.request_id = header_id;
        return Refusal {
            retry_after,
            ..Refusal::from(error)
        };
    };
    let mut message = body.error.message;
    if let (Some(server), Ok(here)) = (body.error.server_time, now()) {
        if body.error.code == "auth_expired" {
            let skew = here as i64 - server as i64;
            message = format!("the clock of this device differs from the server's by {skew} s");
        }
    }
    let mut error = CloudError::new(&body.error.code, message);
    error.request_id = body.request_id.or(header_id);
    Refusal {
        error,
        d: body.error.d,
        server_time: body.error.server_time,
        retry_after,
    }
}

/// A gateway that answered for a host whose service did not (nginx 502,
/// 503, 504 with a page of its own): the host failed, as a network does.
fn gateway_failed(answer: &HttpResponse) -> bool {
    matches!(answer.status, 502..=504) && serde_json::from_slice::<ErrorBody>(&answer.body).is_err()
}

/// Who signs a request (2.3).
#[derive(Clone, Copy)]
enum Auth<'a> {
    /// The identity, through the messenger's `Signer`.
    Identity,
    /// The temporary key of a procedure.
    Temp(&'a TempKey),
    /// Nobody: `challenge`, `plans` and `GET …/recovery/request`.
    Anonymous,
}

fn not_hex64(text: &str) -> bool {
    text.len() != 64 || !text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The server's clock in an answer of success (4.3 `server_time`).
#[derive(Deserialize)]
struct ServerTime {
    server_time: Option<u64>,
}

/// The client of Veydan Cloud over the seams of the messenger: its way to
/// the hosts of the project (`Http`) and its key (`Signer`). One host per
/// session, the first that answers; another only after a failure of the
/// network (cloud-api.md §12). It keeps the offset of the server's clock
/// (`server_time` of 4.3 and of `auth_expired`, 2.2) and signs at the
/// server's time, so a device whose clock is off still gets in.
pub struct HttpCloud {
    http: Arc<dyn Http>,
    signer: Arc<dyn Signer>,
    session: Mutex<Option<String>>,
    /// The server's clock minus this device's, in seconds.
    offset: Mutex<i64>,
}

impl HttpCloud {
    /// A client over the seams the messenger offers. Also the way for a
    /// test or an e2e run to point the module at any `/cloud/`: an `Http`
    /// whose `reach` names `http://127.0.0.1:<port>`, as the fake's does.
    pub fn new(http: Arc<dyn Http>, signer: Arc<dyn Signer>) -> Self {
        Self {
            http,
            signer,
            session: Mutex::new(None),
            offset: Mutex::new(0),
        }
    }

    /// The offset of the server's clock learnt so far, in seconds.
    pub fn clock_offset(&self) -> i64 {
        *self
            .offset
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Learns the server's clock from a `server_time` it sent.
    fn learn_clock(&self, server_time: u64) {
        let Ok(here) = now() else {
            return;
        };
        let offset = i64::try_from(server_time).unwrap_or(i64::MAX)
            - i64::try_from(here).unwrap_or(i64::MAX);
        *self
            .offset
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = offset;
    }

    /// Unix seconds by the server's clock, as far as it is known.
    fn server_now(&self) -> Result<u64, CloudError> {
        let here = i64::try_from(now()?).unwrap_or(i64::MAX);
        Ok(u64::try_from(here.saturating_add(self.clock_offset())).unwrap_or(0))
    }

    /// The event of the identity at the server's time: `nip98` while the
    /// clocks agree, `nip98_at` once an offset is known (a signer that
    /// cannot sign at a given time signs now).
    async fn identity_event(
        &self,
        method: &str,
        url: &str,
        payload: Option<[u8; 32]>,
    ) -> Result<WireEvent, CloudError> {
        if self.clock_offset() == 0 {
            return self.signer.nip98(method, url, payload).await;
        }
        let at = self.server_now()?;
        match self.signer.nip98_at(method, url, payload, at).await {
            Err(e) if e.is(code::NOT_SUPPORTED) => self.signer.nip98(method, url, payload).await,
            done => done,
        }
    }

    fn session(&self) -> Option<String> {
        self.session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn keep(&self, host: &str) {
        *self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(host.to_string());
    }

    /// The hosts to try, the one of the session first.
    async fn hosts(&self) -> Result<Vec<String>, CloudError> {
        let mut hosts = match self.http.reach().await? {
            CloudReach::Veydan { hosts } if !hosts.is_empty() => hosts,
            CloudReach::Veydan { .. } => {
                return Err(CloudError::new(
                    code::NO_HOST,
                    "the manifest names no host of Veydan Cloud",
                ))
            }
            CloudReach::Own => {
                return Err(CloudError::new(
                    code::NO_HOST,
                    "the servers in use are the user's own: Veydan Cloud is not asked",
                ))
            }
            // A clean install recovering (the rendezvous of the trusted people,
            // the close of a restore) reaches the project before the choice.
            CloudReach::Unchosen { hosts } if !hosts.is_empty() => hosts,
            CloudReach::Unchosen { .. } => {
                return Err(CloudError::new(code::NO_HOST, "no servers are chosen yet"))
            }
        };
        if let Some(kept) = self.session() {
            if let Some(i) = hosts.iter().position(|h| *h == kept) {
                let host = hosts.remove(i);
                hosts.insert(0, host);
            }
        }
        Ok(hosts)
    }

    /// One request of the contract; the body of a success, or the refusal.
    /// A signed request refused with `auth_expired` and the server's clock
    /// is signed again at that clock and sent once more (2.2: the client
    /// checks the clock before it complains).
    async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<serde_json::Value>,
        auth: Auth<'_>,
    ) -> Result<Vec<u8>, Refusal> {
        let body = match body {
            Some(value) => serde_json::to_vec(&value)
                .map_err(|e| CloudError::new(code::BAD_REQUEST, e.to_string()))?,
            None => Vec::new(),
        };
        match self.send_once(method, path, &body, auth).await {
            Err(Refusal {
                error,
                server_time: Some(server_time),
                ..
            }) if error.is("auth_expired") && !matches!(auth, Auth::Anonymous) => {
                self.learn_clock(server_time);
                self.send_once(method, path, &body, auth).await
            }
            done => done,
        }
    }

    /// [`HttpCloud::request`] once, over the hosts of the session.
    async fn send_once(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        auth: Auth<'_>,
    ) -> Result<Vec<u8>, Refusal> {
        let payload = nip98::payload_hash(body);
        let mut failed: Option<CloudError> = None;
        for host in self.hosts().await? {
            let url = format!("{host}{path}");
            let mut headers = vec![("accept".to_string(), "application/json".to_string())];
            if !body.is_empty() {
                headers.push(("content-type".into(), "application/json".into()));
            }
            let event = match auth {
                Auth::Identity => Some(self.identity_event(method, &url, payload).await?),
                Auth::Temp(t) => Some(nip98::event_by(
                    t,
                    method,
                    &url,
                    payload,
                    self.server_now()?,
                )?),
                Auth::Anonymous => None,
            };
            if let Some(event) = event {
                headers.push(("authorization".into(), nip98::authorization(&event)?));
            }
            let sent = self
                .http
                .send(HttpRequest {
                    method: method.to_string(),
                    url,
                    headers,
                    body: body.to_vec(),
                })
                .await;
            let answer = match sent {
                Err(e) if e.is(code::TRANSPORT) => {
                    failed = Some(e);
                    continue;
                }
                Err(e) => return Err(e.into()),
                Ok(answer) if gateway_failed(&answer) => {
                    failed = Some(CloudError::new(
                        code::TRANSPORT,
                        format!("the gateway of {host} answered {}", answer.status),
                    ));
                    continue;
                }
                Ok(answer) => answer,
            };
            self.keep(&host);
            if (200..300).contains(&answer.status) {
                if let Ok(ServerTime {
                    server_time: Some(server_time),
                }) = serde_json::from_slice(&answer.body)
                {
                    self.learn_clock(server_time);
                }
                return Ok(answer.body);
            }
            return Err(refusal_of(&answer));
        }
        let why = failed.map_or_else(|| "no host to ask".to_string(), |e| e.message);
        Err(CloudError::new(
            code::NO_HOST,
            format!("no host of Veydan Cloud answered: {why}"),
        )
        .into())
    }

    /// [`HttpCloud::request`] read as `T`.
    async fn ask<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        path: &str,
        body: Option<serde_json::Value>,
        auth: Auth<'_>,
    ) -> Result<T, Refusal> {
        let bytes = self.request(method, path, body, auth).await?;
        serde_json::from_slice(&bytes).map_err(|e| {
            Refusal::from(CloudError::new(
                "internal",
                format!("the answer of the server does not read: {e}"),
            ))
        })
    }

    /// `POST …/recovery/request` once, the refusal whole.
    async fn request_once(
        &self,
        npub: &str,
        t: &TempKey,
        pow: &Pow,
    ) -> Result<RecoveryRequest, Refusal> {
        let body = serde_json::json!({ "npub": npub, "temp_pub": nip98::temp_pub(t)?, "pow": pow });
        self.ask(
            "POST",
            "/cloud/v1/recovery/request",
            Some(body),
            Auth::Temp(t),
        )
        .await
    }
}

fn plain<T>(result: Result<T, Refusal>) -> Result<T, CloudError> {
    result.map_err(|r| r.error)
}

impl CloudApi for HttpCloud {
    fn account(&self) -> SeamFuture<'_, Account> {
        Box::pin(async move {
            plain(
                self.ask("GET", "/cloud/v1/account", None, Auth::Identity)
                    .await,
            )
        })
    }

    fn activate(&self, pow: Option<Pow>) -> SeamFuture<'_, Account> {
        Box::pin(async move {
            let body = match pow {
                Some(pow) => serde_json::json!({ "pow": pow }),
                None => serde_json::json!({}),
            };
            plain(
                self.ask("POST", "/cloud/v1/activate", Some(body), Auth::Identity)
                    .await,
            )
        })
    }

    fn redeem<'a>(&'a self, key: &'a str) -> SeamFuture<'a, Account> {
        Box::pin(async move {
            let body = serde_json::json!({ "key": key });
            plain(
                self.ask("POST", "/cloud/v1/redeem", Some(body), Auth::Identity)
                    .await,
            )
        })
    }

    fn delete(&self) -> SeamFuture<'_, Deleted> {
        Box::pin(async move {
            plain(
                self.ask(
                    "DELETE",
                    "/cloud/v1/account?confirm=DELETE",
                    None,
                    Auth::Identity,
                )
                .await,
            )
        })
    }

    fn plans(&self) -> SeamFuture<'_, Plans> {
        Box::pin(async move {
            plain(
                self.ask("GET", "/cloud/v1/plans", None, Auth::Anonymous)
                    .await,
            )
        })
    }

    fn recovery_config_put(&self, k: u8, n: u8, generation: u32) -> SeamFuture<'_, RecoveryConfig> {
        Box::pin(async move {
            if !(2 <= k && k <= n && n <= 7) {
                return Err(CloudError::new(code::BAD_REQUEST, "2 ≤ k ≤ n ≤ 7"));
            }
            let body = serde_json::json!({ "k": k, "n": n, "generation": generation });
            plain(
                self.ask(
                    "PUT",
                    "/cloud/v1/recovery/config",
                    Some(body),
                    Auth::Identity,
                )
                .await,
            )
        })
    }

    fn recovery_config_delete(&self) -> SeamFuture<'_, ()> {
        Box::pin(async move {
            let done = self
                .request("DELETE", "/cloud/v1/recovery/config", None, Auth::Identity)
                .await;
            plain(done).map(|_| ())
        })
    }

    fn recovery_request<'a>(
        &'a self,
        npub: &'a str,
        t: &'a TempKey,
        pow: Pow,
    ) -> SeamFuture<'a, RecoveryRequest> {
        Box::pin(async move {
            if not_hex64(npub) {
                return Err(CloudError::new(
                    code::BAD_REQUEST,
                    "npub is 64 hex in lower case",
                ));
            }
            match self.request_once(npub, t, &pow).await {
                Err(Refusal {
                    error, d: Some(d), ..
                }) if error.is("pow_insufficient") => {
                    // A new challenge, worked to the difficulty asked, once (3.4).
                    let challenge = self.challenge(Purpose::RecoveryRequest).await?;
                    let pow = crate::activate::solve(&challenge, d, |_| {}, None).await?;
                    plain(self.request_once(npub, t, &pow).await)
                }
                other => plain(other),
            }
        })
    }

    fn recovery_request_get<'a>(
        &'a self,
        npub: &'a str,
    ) -> SeamFuture<'a, Option<RecoveryRequest>> {
        Box::pin(async move {
            if not_hex64(npub) {
                return Err(CloudError::new(
                    code::BAD_REQUEST,
                    "npub is 64 hex in lower case",
                ));
            }
            let path = format!("/cloud/v1/recovery/request?npub={npub}");
            match self.ask("GET", &path, None, Auth::Anonymous).await {
                Ok(request) => Ok(Some(request)),
                Err(r) if r.error.is("not_found") => Ok(None),
                Err(r) => Err(r.error),
            }
        })
    }

    fn recovery_cancel<'a>(&'a self, request_id: &'a str) -> SeamFuture<'a, Cancelled> {
        Box::pin(async move {
            let body = serde_json::json!({ "request_id": request_id });
            plain(
                self.ask(
                    "POST",
                    "/cloud/v1/recovery/cancel",
                    Some(body),
                    Auth::Identity,
                )
                .await,
            )
        })
    }

    fn recovery_close<'a>(&'a self, request_id: &'a str, t: &'a TempKey) -> SeamFuture<'a, Closed> {
        Box::pin(async move {
            let body = serde_json::json!({ "request_id": request_id });
            plain(
                self.ask(
                    "POST",
                    "/cloud/v1/recovery/close",
                    Some(body),
                    Auth::Temp(t),
                )
                .await,
            )
        })
    }

    fn challenge(&self, purpose: Purpose) -> SeamFuture<'_, Challenge> {
        Box::pin(async move {
            let word = match purpose {
                Purpose::Activate => "activate",
                Purpose::RecoveryRequest => "recovery.request",
            };
            let path = format!("/cloud/v1/challenge?purpose={word}");
            plain(self.ask("GET", &path, None, Auth::Anonymous).await)
        })
    }

    fn storage<'a>(&'a self, _prefix: &'a str) -> SeamFuture<'a, Arc<dyn Storage>> {
        Box::pin(async {
            Err(CloudError::new(
                code::NOT_SUPPORTED,
                "the backend of the vault on Veydan Cloud is not in this version",
            ))
        })
    }

    fn usage(&self) -> SeamFuture<'_, Usage> {
        Box::pin(async move {
            plain(
                self.ask("GET", "/vault/v1/usage", None, Auth::Identity)
                    .await,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn every_call_of_the_unconfigured_client_fails_with_no_host() {
        let api = Unconfigured;
        let t = TempKey::generate().expect("key");
        let pow = || Pow {
            challenge: "AA".into(),
            nonce: 0,
        };
        let codes = [
            api.account().await.err(),
            api.activate(None).await.err(),
            api.redeem("7K2M-ABCD-EFGH-JKMN-PQRS").await.err(),
            api.delete().await.err(),
            api.plans().await.err(),
            api.recovery_config_put(3, 5, 1).await.err(),
            api.recovery_config_delete().await.err(),
            api.recovery_request(&"ab".repeat(32), &t, pow())
                .await
                .err(),
            api.recovery_request_get(&"ab".repeat(32)).await.err(),
            api.recovery_cancel("9c").await.err(),
            api.recovery_close("9c", &t).await.err(),
            api.challenge(Purpose::Activate).await.err(),
            api.storage("notes/").await.err(),
            api.usage().await.err(),
        ];
        for e in codes {
            assert_eq!(e.map(|e| e.code).as_deref(), Some(code::NO_HOST));
        }
    }

    /// The example of 4.3 reads, and the unknown fields of a later server
    /// do not break it (2.1).
    #[test]
    fn the_account_of_the_contract_reads() {
        let json = serde_json::json!({
            "npub": "3f".repeat(32), "state": "paid", "plan": "plus", "until": 1793604345u64,
            "grace_until": null,
            "limits": { "vault.files_bytes": 53687091200u64, "media.retention_days": 90 },
            "usage": { "as_of": 1791010000u64,
                       "vault": { "files_bytes": 1234567, "text_bytes": 45678 },
                       "media": { "upload_today_bytes": 0, "storage_bytes": 7340032 } },
            "recovery": { "configured": true, "k": 3, "n": 5, "generation": 2 },
            "last_seen_at": 1791012000u64, "server_time": 1791012345u64, "later": "ignored",
        });
        let account: Account = serde_json::from_value(json).expect("account");
        assert_eq!(account.state, AccountState::Paid);
        assert_eq!(account.limits["media.retention_days"], 90);
        assert_eq!(account.recovery.and_then(|r| r.k), Some(3));
        assert!(account.subscription.is_none());
    }

    /// The example of 4.5 reads: a free plan without prices and a paid one
    /// with two, the unknown fields of a later server ignored (2.1).
    #[test]
    fn the_plans_of_the_contract_read() {
        let json = serde_json::json!({
            "plans": [
                { "id": "free", "name": "Free", "free": true, "period_days": 0, "sort": 0,
                  "limits": { "vault.files_bytes": 2147483648u64, "media.retention_days": 14 },
                  "prices": [] },
                { "id": "plus", "name": "Plus", "free": false, "period_days": 30, "sort": 10,
                  "limits": { "vault.files_bytes": 53687091200u64, "media.retention_days": 14 },
                  "prices": [ { "currency": "USD", "amount_minor": 399, "period_days": 30 },
                              { "currency": "USDT", "amount_minor": 400, "period_days": 30,
                                "later": "ignored" } ],
                  "later": "ignored" },
            ],
            "server_time": 1790000000u64,
        });
        let catalog: Plans = serde_json::from_value(json).expect("plans");
        assert_eq!(catalog.server_time, Some(1_790_000_000));
        let [free, plus] = catalog.plans.as_slice() else {
            panic!("two plans: {:?}", catalog.plans);
        };
        assert!(free.free && free.prices.is_empty());
        assert_eq!((free.period_days, free.sort), (0, 0));
        assert_eq!(
            (plus.id.as_str(), plus.name.as_str(), plus.free),
            ("plus", "Plus", false)
        );
        assert_eq!(plus.limits["vault.files_bytes"], 53_687_091_200);
        assert_eq!(
            plus.prices[1],
            PlanPrice {
                currency: "USDT".into(),
                amount_minor: 400,
                period_days: 30
            }
        );
    }

    /// The answers of 5.3: an open request, and one closed less than 24
    /// hours ago with its `closed_at`; the answer of 5.2 has neither.
    #[test]
    fn a_request_of_recovery_reads_open_and_closed() {
        let open: RecoveryRequest = serde_json::from_value(serde_json::json!({
            "request_id": "9c".repeat(16), "temp_pub": "ab".repeat(32), "state": "open",
            "created_at": 1791012345u64, "expires_at": 1791617145u64,
        }))
        .expect("open");
        assert_eq!(open.state.as_deref(), Some("open"));
        assert_eq!(open.closed_at, None);
        let cancelled: RecoveryRequest = serde_json::from_value(serde_json::json!({
            "request_id": "9c".repeat(16), "temp_pub": "ab".repeat(32), "state": "cancelled",
            "created_at": 1791012345u64, "expires_at": 1791617145u64, "closed_at": 1791020000u64,
        }))
        .expect("cancelled");
        assert_eq!(cancelled.state.as_deref(), Some("cancelled"));
        assert_eq!(cancelled.closed_at, Some(1_791_020_000));
        let made: RecoveryRequest = serde_json::from_value(serde_json::json!({
            "request_id": "9c".repeat(16), "k": 3, "n": 5,
            "created_at": 1791012345u64, "expires_at": 1791617145u64,
        }))
        .expect("made");
        assert_eq!(
            (made.k, made.n, made.state, made.closed_at),
            (Some(3), Some(5), None, None)
        );
    }

    #[test]
    fn the_purposes_are_the_words_of_the_contract() {
        assert_eq!(
            serde_json::to_value(Purpose::Activate).expect("json"),
            "activate"
        );
        assert_eq!(
            serde_json::to_value(Purpose::RecoveryRequest).expect("json"),
            "recovery.request"
        );
        let usage: Usage = serde_json::from_value(serde_json::json!({
            "as_of": 1791901234u64, "host": "eu-1", "plan": "free", "state": "free", "until": null,
            "table_serial": 1842, "max_object_bytes": 67108864,
            "pools": { "text": { "used": 1, "limit": 2, "objects": 3 } },
            "breakdown": [{ "prefix": "blobs/", "pool": "files", "bytes": 5, "objects": 1 }],
            "reconciled_at": null,
        }))
        .expect("usage");
        assert_eq!(usage.pools["text"].limit, 2);
        assert_eq!(usage.breakdown[0].pool, "files");
    }

    fn answer(status: u16, headers: &[(&str, &str)], body: &str) -> HttpResponse {
        HttpResponse {
            status,
            headers: headers
                .iter()
                .map(|(n, v)| (n.to_string(), v.to_string()))
                .collect(),
            body: body.as_bytes().to_vec(),
        }
    }

    /// The body of 2.2 with the extra fields of section 9; a page that is
    /// not the contract's gets a word from its status.
    #[test]
    fn a_refusal_reads_its_word_and_its_extra_fields() {
        let insufficient = refusal_of(&answer(
            428,
            &[],
            r#"{"error":{"code":"pow_insufficient","message":"…","d":10},"request_id":"9c"}"#,
        ));
        assert_eq!(insufficient.error.code, "pow_insufficient");
        assert_eq!(insufficient.d, Some(10));
        assert_eq!(insufficient.error.request_id.as_deref(), Some("9c"));
        let limited = refusal_of(&answer(
            429,
            &[("retry-after", "30"), ("x-request-id", "7a")],
            r#"{"error":{"code":"rate_limited","message":"slow down"},"request_id":null}"#,
        ));
        assert_eq!(
            (limited.error.code.as_str(), limited.retry_after),
            ("rate_limited", Some(30))
        );
        assert_eq!(limited.error.request_id.as_deref(), Some("7a"));
        let expired = refusal_of(&answer(
            401,
            &[],
            r#"{"error":{"code":"auth_expired","message":"…","server_time":1}},"request_id":"1"}"#,
        ));
        assert_eq!(expired.error.code, "auth_invalid");
        let expired = refusal_of(&answer(
            401,
            &[],
            r#"{"error":{"code":"auth_expired","message":"…","server_time":1},"request_id":"1"}"#,
        ));
        assert_eq!(expired.server_time, Some(1));
        assert!(
            expired.error.message.contains("clock"),
            "{}",
            expired.error.message
        );
        let page = refusal_of(&answer(413, &[], "<html>too large</html>"));
        assert_eq!(page.error.code, "payload_too_large");
        assert!(gateway_failed(&answer(
            502,
            &[],
            "<html>bad gateway</html>"
        )));
        assert!(!gateway_failed(&answer(
            503,
            &[],
            r#"{"error":{"code":"busy","message":"…"},"request_id":"1"}"#
        )));
    }

    /// Every endpoint of /cloud/v1 the client calls, against the fake: the
    /// paths and bodies of the contract, NIP-98 by the identity or by T, no
    /// signature where the contract says none.
    #[tokio::test]
    async fn the_client_speaks_every_endpoint_of_the_contract() {
        let fake = crate::fake::Fake::start(2).await;
        let api = fake.client();
        // The catalog is read before there is an account, and nobody signs it
        // (the fake refuses a signed one).
        let catalog = api.plans().await.expect("plans");
        assert_eq!(fake.last_path().as_deref(), Some("/cloud/v1/plans"));
        let ids: Vec<&str> = catalog.plans.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["free", "plus"]);
        assert!(catalog.plans[0].free && catalog.plans[0].prices.is_empty());
        assert_eq!(catalog.plans[1].prices[0].amount_minor, 399);
        fake.add_account();
        // The account says the limits the catalog says for its plan.
        let account = api.account().await.expect("account");
        assert_eq!(account.limits, catalog.plans[0].limits);
        let config = api.recovery_config_put(3, 5, 2).await.expect("config");
        assert_eq!((config.k, config.n, config.generation), (3, 5, 2));
        assert_eq!(
            api.recovery_config_put(1, 5, 2).await.map_err(|e| e.code),
            Err(code::BAD_REQUEST.to_string())
        );
        api.recovery_config_delete().await.expect("deleted");

        let npub = fake.npub();
        assert_eq!(api.recovery_request_get(&npub).await, Ok(None));
        let t = TempKey::generate().expect("key");
        let challenge = api
            .challenge(Purpose::RecoveryRequest)
            .await
            .expect("challenge");
        assert_eq!(challenge.purpose, Purpose::RecoveryRequest);
        // The server asks more than the challenge's d: the client works the
        // difference on a new challenge and asks once more (3.4).
        fake.require(4);
        let pow = crate::activate::solve(&challenge, challenge.d, |_| {}, None)
            .await
            .expect("pow");
        let made = api.recovery_request(&npub, &t, pow).await.expect("request");
        assert_eq!((made.k, made.n), (Some(3), Some(5)));
        assert_eq!(fake.solved(), 2);
        let open = api
            .recovery_request_get(&npub)
            .await
            .expect("get")
            .expect("open");
        assert_eq!(open.state.as_deref(), Some("open"));
        assert_eq!(
            open.temp_pub,
            Some(crate::nip98::temp_pub(&t).expect("pub"))
        );
        let closed = api
            .recovery_close(&made.request_id, &t)
            .await
            .expect("close");
        assert_eq!(closed.state, "closed");
        let stranger = TempKey::generate().expect("key");
        assert_eq!(
            api.recovery_close(&made.request_id, &stranger)
                .await
                .map_err(|e| e.code),
            Err("not_found".to_string())
        );
        let cancelled = api.recovery_cancel(&made.request_id).await.expect("cancel");
        assert_eq!(cancelled.state, "cancelled");
        assert_eq!(
            api.recovery_request_get("not hex")
                .await
                .map_err(|e| e.code),
            Err(code::BAD_REQUEST.to_string())
        );

        let usage = api.usage().await.expect("usage");
        assert_eq!(usage.pools["files"].limit, 2_147_483_648);
        assert_eq!(
            api.storage("notes/").await.err().map(|e| e.code).as_deref(),
            Some(code::NOT_SUPPORTED)
        );
    }

    /// A device whose clock is an hour off: the first signed request is
    /// refused with `auth_expired` and the server's clock, the client signs
    /// again at that clock and gets in once; the requests after go at
    /// once, those of T too. A clock behind is the same.
    #[tokio::test]
    async fn a_wrong_clock_is_set_by_the_servers() {
        let fake = crate::fake::Fake::start(2).await;
        fake.add_account();
        fake.skew_clock(3600);
        let api = fake.client();
        let account = api.account().await.expect("account");
        assert_eq!(account.plan, "free");
        assert_eq!(fake.requests(), 2);
        assert!(
            (api.clock_offset() - 3600).abs() <= 2,
            "{}",
            api.clock_offset()
        );
        api.usage().await.expect("usage");
        assert_eq!(fake.requests(), 3);
        let t = TempKey::generate().expect("key");
        assert_eq!(
            api.recovery_close("9c", &t).await.map_err(|e| e.code),
            Err("not_found".to_string())
        );
        assert_eq!(fake.requests(), 4);

        fake.skew_clock(-3600);
        api.account().await.expect("account");
        assert_eq!(fake.requests(), 6);
        assert!(
            (api.clock_offset() + 3600).abs() <= 2,
            "{}",
            api.clock_offset()
        );

        // Unsigned requests are not made again: nothing to correct.
        let anonymous = api.recovery_request_get(&"ab".repeat(32)).await;
        assert_eq!(anonymous, Ok(None));
        assert_eq!(fake.requests(), 7);
    }

    /// The fake checks the Schnorr signature of the identity as the server
    /// does: an event of another key under the identity's name is refused.
    #[tokio::test]
    async fn a_forged_signature_of_the_identity_is_refused() {
        struct Forger;
        impl Signer for Forger {
            fn pubkey(&self) -> SeamFuture<'_, String> {
                Box::pin(async { crate::fake::FixedSigner.pubkey().await })
            }
            fn nip98<'a>(
                &'a self,
                method: &'a str,
                url: &'a str,
                payload: Option<[u8; 32]>,
            ) -> SeamFuture<'a, WireEvent> {
                Box::pin(async move {
                    let mut event = crate::fake::FixedSigner.nip98(method, url, payload).await?;
                    let other = TempKey::generate()?;
                    let forged = nip98::event_by(&other, method, url, payload, now()?)?;
                    event.sig = forged.sig;
                    Ok(event)
                })
            }
            fn nip44_encrypt<'a>(&'a self, _: &'a str, _: &'a [u8]) -> SeamFuture<'a, Vec<u8>> {
                Box::pin(async { Err(CloudError::new(code::NOT_SUPPORTED, "forger")) })
            }
            fn nip44_decrypt<'a>(
                &'a self,
                _: &'a str,
                _: &'a [u8],
            ) -> SeamFuture<'a, zeroize::Zeroizing<Vec<u8>>> {
                Box::pin(async { Err(CloudError::new(code::NOT_SUPPORTED, "forger")) })
            }
        }
        let fake = crate::fake::Fake::start(2).await;
        fake.add_account();
        assert!(fake.client().account().await.is_ok());
        let forged = HttpCloud::new(fake.http(), Arc::new(Forger));
        assert_eq!(
            forged.account().await.map_err(|e| e.code),
            Err("auth_invalid".to_string())
        );
    }

    /// The servers of the user ask nothing of the project; a host that
    /// does not answer is `no_host` once every host was tried.
    #[tokio::test]
    async fn off_the_project_nothing_goes_out_and_a_dead_host_is_no_host() {
        struct OwnServers;
        impl Http for OwnServers {
            fn reach(&self) -> SeamFuture<'_, CloudReach> {
                Box::pin(async { Ok(CloudReach::Own) })
            }
            fn send(&self, _: HttpRequest) -> SeamFuture<'_, HttpResponse> {
                Box::pin(async { Err(CloudError::new("sent", "a request went out")) })
            }
        }
        let own = HttpCloud::new(Arc::new(OwnServers), Arc::new(crate::fake::FixedSigner));
        let e = own.account().await.expect_err("refused");
        assert_eq!(e.code, code::NO_HOST);
        assert!(e.message.contains("own"), "{}", e.message);

        let fake = crate::fake::Fake::start(2).await;
        let api = fake.client();
        fake.add_account();
        assert!(api.account().await.is_ok());
        fake.go_down();
        assert_eq!(
            api.account().await.map_err(|e| e.code),
            Err(code::NO_HOST.to_string())
        );
    }
}
