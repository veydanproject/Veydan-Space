// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A fake of `/cloud/v1/` and `GET /vault/v1/usage` on loopback (axum), the
//! seams around the client (`Http` over reqwest, a `Signer` of one fixed
//! identity, a box of the lock in memory, a lock to confirm with) and a
//! mock app that has them all: what the tests of the module run against.
//! It answers as the contract says (cloud-api.md 3-5, 9) and checks NIP-98
//! the way the server does: `u` character by character, `method`,
//! `payload`, `created_at` within 60 s of its clock (`auth_expired` with
//! `server_time` else), and the Schnorr signature of every event, the
//! identity's and T's alike.

// A refusal of the fake is the answer itself, returned as it is.
#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::Manager;
use veydan_recovery::pow;
use veydan_shell::seams::{
    code, CloudError, CloudReach, Http, HttpRequest, HttpResponse, LockBox, LockConfirm,
    SeamFuture, Signer, TempKey, WireEvent,
};
use zeroize::Zeroizing;

use crate::activate::Activation;
use crate::api::{CloudApi, HttpCloud};
use crate::delete::LockGate;
use crate::nip98::hex;

/// The key the fake signs its challenges with.
const SERVER_KEY: &[u8] = b"the key of the fake server";

/// The one key of activation the fake knows.
pub const KEY: &str = "7K2M-ABCD-EFGH-JKMN-PQRS";

/// The receipt of a deletion.
pub const RECEIPT: &str = "7K2M-ABCD-EFGH";

/// The secret of the fixed identity: a key of secp256k1 like any other.
fn identity_key() -> TempKey {
    TempKey::new(Zeroizing::new([0x3f; 32]))
}

fn npub() -> String {
    crate::nip98::temp_pub(&identity_key()).unwrap_or_default()
}

#[derive(Default)]
struct Server {
    base: String,
    d: u8,
    /// What `POST …/recovery/request` requires beyond the challenge's `d`.
    required: u8,
    /// The first request of recovery under a raised `required` was refused,
    /// whatever its solution: the client always meets the 428 once.
    raised: bool,
    account: Option<Value>,
    deleted: bool,
    spent: Vec<Vec<u8>>,
    solved: u64,
    refuse: Option<(String, u32)>,
    requests: u64,
    last_path: Option<String>,
    open: Option<Value>,
    /// The recovery config of the identity, `{k, n, generation}`.
    config: Option<Value>,
    /// A request needs a config (`404 not_configured` else), as the server
    /// says; off by default so that the tests of the client need none.
    strict_config: bool,
    nonce_r: u8,
    /// The fake's clock minus the real one, in seconds.
    skew: i64,
    /// `METHOD path?query` of every request, in order.
    log: Vec<String>,
}

impl Server {
    /// Unix seconds by the fake's clock.
    fn clock(&self) -> u64 {
        u64::try_from(now() as i64 + self.skew).unwrap_or(0)
    }
}

/// The fake server, with the counters a test reads.
pub struct Fake {
    server: Arc<Mutex<Server>>,
    down: Arc<AtomicBool>,
}

fn refuse(status: u16, word: &str, extra: Value) -> Response {
    let mut error = json!({ "code": word, "message": format!("the fake says {word}") });
    if let (Some(error), Some(extra)) = (error.as_object_mut(), extra.as_object()) {
        for (k, v) in extra {
            error.insert(k.clone(), v.clone());
        }
    }
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (
        status,
        [("x-request-id", "9c")],
        Json(json!({ "error": error, "request_id": "9c" })),
    )
        .into_response()
}

fn ok(status: u16, body: Value) -> Response {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
    (status, Json(body)).into_response()
}

fn now() -> u64 {
    crate::api::now().unwrap_or(0)
}

/// The event of `Authorization`, checked against the request as the server
/// sees it; the signer when it holds.
fn signer_of(
    server: &Server,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<WireEvent, Response> {
    let header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| refuse(401, "auth_missing", json!({})))?;
    let encoded = header
        .strip_prefix("Nostr ")
        .ok_or_else(|| refuse(401, "auth_invalid", json!({})))?;
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| refuse(401, "auth_invalid", json!({})))?;
    let event: WireEvent =
        serde_json::from_slice(&bytes).map_err(|_| refuse(401, "auth_invalid", json!({})))?;
    let tag = |name: &str| {
        event
            .tags
            .iter()
            .find(|t| t.first().map(String::as_str) == Some(name))
            .and_then(|t| t.get(1).cloned())
    };
    let url = format!("{}{}", server.base, uri);
    if event.kind != 27235 {
        return Err(refuse(401, "auth_invalid", json!({})));
    }
    if event.created_at.abs_diff(server.clock()) > 60 {
        return Err(refuse(
            401,
            "auth_expired",
            json!({ "server_time": server.clock() }),
        ));
    }
    if tag("u").as_deref() != Some(url.as_str()) {
        return Err(refuse(401, "auth_url_mismatch", json!({})));
    }
    let method_ok = tag("method").is_some_and(|m| m.eq_ignore_ascii_case(method.as_str()));
    let payload_ok = if body.is_empty() {
        tag("payload").is_none()
    } else {
        tag("payload").as_deref() == Some(hex(&Sha256::digest(body)).as_str())
    };
    if !method_ok || !payload_ok {
        return Err(refuse(401, "auth_invalid", json!({})));
    }
    {
        // The identity's or T's: the signature is checked as the server does.
        let id: [u8; 32] = Sha256::digest(
            json!([
                0,
                event.pubkey,
                event.created_at,
                event.kind,
                event.tags,
                event.content
            ])
            .to_string()
            .as_bytes(),
        )
        .into();
        let unhex = |s: &str| -> Vec<u8> {
            (0..s.len() / 2)
                .filter_map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok())
                .collect()
        };
        let secp = secp256k1::Secp256k1::verification_only();
        let key = secp256k1::XOnlyPublicKey::from_slice(&unhex(&event.pubkey));
        let sig = secp256k1::schnorr::Signature::from_slice(&unhex(&event.sig));
        let verified = match (key, sig) {
            (Ok(key), Ok(sig)) => secp.verify_schnorr(&sig, &id, &key).is_ok(),
            _ => false,
        };
        if hex(&id) != event.id || !verified {
            return Err(refuse(401, "auth_invalid", json!({})));
        }
    }
    Ok(event)
}

/// Checks a solution as 3.2 does; the leading zeros of `h` when it holds.
fn check_pow(
    server: &mut Server,
    pow_value: &Value,
    purpose: pow::Purpose,
) -> Result<u32, Response> {
    let text = pow_value["challenge"].as_str().unwrap_or_default();
    let nonce = pow_value["nonce"]
        .as_u64()
        .ok_or_else(|| refuse(400, "bad_request", json!({})))?;
    let bytes = pow::from_base64url(text).map_err(|_| refuse(400, "pow_invalid", json!({})))?;
    let challenge = pow::Challenge::decode(&bytes, &[SERVER_KEY])
        .map_err(|_| refuse(400, "pow_invalid", json!({})))?;
    if challenge.purpose != purpose {
        return Err(refuse(400, "pow_invalid", json!({})));
    }
    if !challenge.is_fresh(now()) {
        return Err(refuse(400, "pow_expired", json!({})));
    }
    let h = pow::hash(&bytes, nonce).map_err(|_| refuse(400, "pow_invalid", json!({})))?;
    let zeros = pow::leading_zero_bits(&h);
    if zeros < u32::from(challenge.d) {
        return Err(refuse(400, "pow_wrong", json!({})));
    }
    if server.spent.contains(&bytes) {
        return Err(refuse(409, "pow_replayed", json!({})));
    }
    server.spent.push(bytes);
    server.solved += 1;
    Ok(zeros)
}

/// The effective limits of a plan, the same in an account (4.3) and in the
/// catalog (4.5): those of `free`, and a paid plan's own over them.
fn limits_of(plan: &str) -> Value {
    let mut limits = json!({
        "vault.files_bytes": 2147483648u64, "vault.text_bytes": 536870912u64,
        "vault.max_object_bytes": 67108864u64, "media.max_file_bytes": 20971520u64,
        "media.upload_per_day": 2147483648u64, "media.retention_days": 14,
    });
    if plan != "free" {
        limits["vault.files_bytes"] = json!(53687091200u64);
        limits["media.max_file_bytes"] = json!(1073741824u64);
    }
    limits
}

fn account_of(plan: &str) -> Value {
    let paid = plan != "free";
    json!({
        "npub": npub(), "state": if paid { "paid" } else { "free" }, "plan": plan,
        "until": if paid { json!(1793604345u64) } else { Value::Null }, "grace_until": null,
        "limits": limits_of(plan),
        "usage": { "as_of": 1791010000u64, "vault": { "files_bytes": 1234567, "text_bytes": 45678 },
                   "media": { "upload_today_bytes": 0, "storage_bytes": 7340032 } },
        "recovery": { "configured": false, "k": null, "n": null, "generation": null },
        "last_seen_at": now(), "server_time": now(),
    })
}

/// An account as the fake answers it now: `server_time` by its clock.
fn timed(server: &Server, account: &Value) -> Value {
    let mut account = account.clone();
    account["server_time"] = json!(server.clock());
    account
}

async fn answer(
    axum::extract::State(server): axum::extract::State<Arc<Mutex<Server>>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let mut s = server.lock().unwrap_or_else(|p| p.into_inner());
    s.requests += 1;
    s.last_path = Some(uri.to_string());
    s.log.push(format!("{method} {uri}"));
    let path = uri.path().to_string();
    let query = uri.query().unwrap_or_default().to_string();
    let json_body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let signed = |s: &Server| signer_of(s, &method, &uri, &headers, &body);
    match (method.as_str(), path.as_str()) {
        ("GET", "/cloud/v1/challenge") => {
            let purpose = match query.as_str() {
                "purpose=activate" => pow::Purpose::Activate,
                "purpose=recovery.request" => pow::Purpose::RecoveryRequest,
                _ => return refuse(400, "bad_request", json!({})),
            };
            s.nonce_r = s.nonce_r.wrapping_add(1);
            let challenge = pow::Challenge {
                ts: now(),
                d: s.d,
                purpose,
                r: [s.nonce_r; 8],
            };
            let text = pow::to_base64url(&challenge.encode(SERVER_KEY));
            ok(
                200,
                json!({ "challenge": text, "d": s.d, "purpose": purpose.as_str(), "expires_at": now() + 120 }),
            )
        }
        ("POST", "/cloud/v1/activate") => {
            if let Err(r) = signed(&s) {
                return r;
            }
            if let Some(account) = &s.account {
                return ok(200, timed(&s, account));
            }
            if json_body["pow"].is_null() {
                return refuse(400, "pow_invalid", json!({}));
            }
            if let Err(r) = check_pow(&mut s, &json_body["pow"], pow::Purpose::Activate) {
                return r;
            }
            if let Some((word, left)) = s.refuse.take() {
                if left > 1 {
                    s.refuse = Some((word.clone(), left - 1));
                }
                let status = if word == "pow_replayed" { 409 } else { 400 };
                return refuse(status, &word, json!({}));
            }
            let account = account_of("free");
            s.account = Some(account.clone());
            s.deleted = false;
            ok(201, timed(&s, &account))
        }
        ("GET", "/cloud/v1/account") => match signed(&s) {
            Err(r) => r,
            Ok(_) => match &s.account {
                Some(account) => ok(200, timed(&s, account)),
                None => refuse(404, "not_activated", json!({})),
            },
        },
        ("POST", "/cloud/v1/redeem") => {
            if let Err(r) = signed(&s) {
                return r;
            }
            if json_body["key"].as_str() != Some(KEY) {
                return refuse(404, "key_unknown", json!({}));
            }
            if s.account.is_none() {
                return refuse(404, "not_activated", json!({}));
            }
            let mut account = account_of("plus");
            account["subscription"] =
                json!({ "plan": "plus", "since": 1, "until": 1793604345u64, "source": "key:1" });
            s.account = Some(account.clone());
            ok(200, timed(&s, &account))
        }
        ("GET", "/cloud/v1/plans") => {
            // Stricter than the server, which does not read the header: the
            // client must not sign what nobody asks to be signed.
            if headers.contains_key("authorization") {
                return refuse(400, "bad_request", json!({}));
            }
            ok(
                200,
                json!({
                    "plans": [
                        { "id": "free", "name": "Free", "free": true, "period_days": 0, "sort": 0,
                          "limits": limits_of("free"), "prices": [] },
                        { "id": "plus", "name": "Plus", "free": false, "period_days": 30, "sort": 10,
                          "limits": limits_of("plus"),
                          "prices": [ { "currency": "USD", "amount_minor": 399, "period_days": 30 },
                                      { "currency": "USD", "amount_minor": 3990, "period_days": 365 } ] },
                    ],
                    "server_time": s.clock(),
                }),
            )
        }
        ("DELETE", "/cloud/v1/account") => {
            if let Err(r) = signed(&s) {
                return r;
            }
            if query != "confirm=DELETE" {
                return refuse(400, "confirm_required", json!({}));
            }
            if s.account.take().is_some() || s.deleted {
                s.deleted = true;
                return ok(
                    200,
                    json!({ "deleted_at": 1791012345u64, "receipt": RECEIPT }),
                );
            }
            refuse(404, "not_activated", json!({}))
        }
        ("PUT", "/cloud/v1/recovery/config") => match signed(&s) {
            Err(r) => r,
            Ok(_) => {
                let (k, n) = (json_body["k"].as_u64(), json_body["n"].as_u64());
                if !matches!((k, n), (Some(k), Some(n)) if 2 <= k && k <= n && n <= 7) {
                    return refuse(400, "bad_request", json!({}));
                }
                let generation = json_body["generation"].as_u64().unwrap_or(0);
                if let Some(known) = s.config.as_ref().and_then(|c| c["generation"].as_u64()) {
                    if generation < known {
                        return refuse(409, "generation_stale", json!({ "generation": known }));
                    }
                }
                let config = json!({ "k": k, "n": n, "generation": generation, "updated_at": now() });
                s.config = Some(config.clone());
                ok(200, config)
            }
        },
        ("DELETE", "/cloud/v1/recovery/config") => match signed(&s) {
            Err(r) => r,
            Ok(_) => {
                s.config = None;
                if let Some(open) = s.open.as_mut() {
                    if open["state"] == "open" {
                        open["state"] = json!("expired");
                        open["closed_at"] = json!(now());
                    }
                }
                ok(200, json!({}))
            }
        },
        ("POST", "/cloud/v1/recovery/request") => {
            let event = match signed(&s) {
                Err(r) => return r,
                Ok(event) => event,
            };
            if json_body["temp_pub"].as_str() != Some(event.pubkey.as_str()) {
                return refuse(400, "temp_pub_mismatch", json!({}));
            }
            let zeros = match check_pow(&mut s, &json_body["pow"], pow::Purpose::RecoveryRequest) {
                Err(r) => return r,
                Ok(zeros) => zeros,
            };
            let raise = s.required > s.d && !s.raised;
            if raise || zeros < u32::from(s.required) {
                s.raised = true;
                let d = s.required;
                return refuse(428, "pow_insufficient", json!({ "d": d }));
            }
            if s.strict_config && s.config.is_none() {
                return refuse(404, "not_configured", json!({}));
            }
            if let Some(open) = s.open.as_ref().filter(|o| o["state"] == "open") {
                return refuse(
                    409,
                    "request_open",
                    json!({ "request_id": open["request_id"], "expires_at": open["expires_at"] }),
                );
            }
            let (k, n) = match &s.config {
                Some(c) => (c["k"].clone(), c["n"].clone()),
                None => (json!(3), json!(5)),
            };
            let request = json!({ "request_id": "9c".repeat(16), "npub": json_body["npub"], "temp_pub": event.pubkey, "state": "open",
                                  "created_at": now(), "expires_at": now() + 1800 });
            s.open = Some(request.clone());
            ok(
                201,
                json!({ "request_id": "9c".repeat(16), "k": k, "n": n, "created_at": now(), "expires_at": now() + 1800 }),
            )
        }
        ("GET", "/cloud/v1/recovery/request") => {
            if headers.contains_key("authorization") {
                return refuse(400, "bad_request", json!({}));
            }
            match &s.open {
                Some(open) if query == format!("npub={}", open["npub"].as_str().unwrap_or_default()) => {
                    let mut shown = open.clone();
                    shown.as_object_mut().map(|o| o.remove("npub"));
                    ok(200, shown)
                }
                _ => refuse(404, "not_found", json!({})),
            }
        }
        ("POST", "/cloud/v1/recovery/cancel") => match signed(&s) {
            Err(r) => r,
            Ok(_) => {
                // Lenient: a closed request answers `cancelled` too, where the
                // server would say `request_closed`; the tests of the client
                // cancel after a close.
                if let Some(open) = s.open.as_mut().filter(|o| o["state"] == "open") {
                    open["state"] = json!("cancelled");
                    open["closed_at"] = json!(now());
                }
                ok(
                    200,
                    json!({ "state": "cancelled", "frozen_until": now() + 7 * 86400 }),
                )
            }
        },
        ("POST", "/cloud/v1/recovery/close") => {
            let event = match signed(&s) {
                Err(r) => return r,
                Ok(event) => event,
            };
            let mine =
                s.open.as_ref().and_then(|o| o["temp_pub"].as_str()) == Some(event.pubkey.as_str());
            if !mine {
                return refuse(404, "not_found", json!({}));
            }
            if let Some(open) = s.open.as_mut() {
                match open["state"].as_str() {
                    Some("open") => {
                        open["state"] = json!("closed");
                        open["closed_at"] = json!(now());
                    }
                    Some("closed") => {}
                    _ => return refuse(409, "request_closed", json!({ "state": open["state"] })),
                }
            }
            ok(200, json!({ "state": "closed" }))
        }
        ("GET", "/vault/v1/usage") => match signed(&s) {
            Err(r) => r,
            Ok(_) => ok(
                200,
                json!({
                    "as_of": now(), "host": "eu-1", "plan": "free", "state": "free", "until": null,
                    "table_serial": 1842, "max_object_bytes": 67108864,
                    "pools": { "text": { "used": 1843200, "limit": 536870912, "objects": 412 },
                               "files": { "used": 1932735283u64, "limit": 2147483648u64, "objects": 1803 } },
                    "breakdown": [{ "prefix": "blobs/", "pool": "files", "bytes": 1500000000u64, "objects": 1700 }],
                    "reconciled_at": null,
                }),
            ),
        },
        _ => refuse(404, "not_found", json!({})),
    }
}

impl Fake {
    /// A fake whose challenges ask for `d` leading zero bits.
    pub async fn start(d: u8) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let server = Arc::new(Mutex::new(Server {
            base,
            d,
            required: d,
            ..Server::default()
        }));
        let app = axum::Router::new()
            .fallback(answer)
            .with_state(server.clone());
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self {
            server,
            down: Arc::new(AtomicBool::new(false)),
        }
    }

    fn with<T>(&self, f: impl FnOnce(&mut Server) -> T) -> T {
        f(&mut self.server.lock().unwrap_or_else(|p| p.into_inner()))
    }

    /// The identity the fake signer signs as.
    pub fn npub(&self) -> String {
        npub()
    }

    /// Solutions checked and accepted, of any purpose.
    pub fn solved(&self) -> u64 {
        self.with(|s| s.solved)
    }

    /// Requests that reached the server.
    pub fn requests(&self) -> u64 {
        self.with(|s| s.requests)
    }

    /// The path and query of the last request.
    pub fn last_path(&self) -> Option<String> {
        self.with(|s| s.last_path.clone())
    }

    /// `METHOD path?query` of every request so far, in order.
    pub fn log(&self) -> Vec<String> {
        self.with(|s| s.log.clone())
    }

    /// The fake's clock runs `secs` ahead of the device's (behind when negative).
    pub fn skew_clock(&self, secs: i64) {
        self.with(|s| s.skew = secs);
    }

    /// The identity has a free account from now on.
    pub fn add_account(&self) {
        self.with(|s| s.account = Some(account_of("free")));
    }

    /// The next `times` activations with a good solution are refused with `word`.
    pub fn refuse_next_activations(&self, word: &str, times: u32) {
        self.with(|s| s.refuse = Some((word.to_string(), times)));
    }

    /// A request of recovery needs `d` leading zero bits.
    pub fn require(&self, d: u8) {
        self.with(|s| s.required = d);
    }

    /// A request of recovery needs a config, as on the server (5.2).
    pub fn require_config(&self) {
        self.with(|s| s.strict_config = true);
    }

    /// The recovery config the identity wrote, `{k, n, generation}`.
    pub fn config(&self) -> Option<Value> {
        self.with(|s| s.config.clone())
    }

    /// The state of the one request the fake knows: `open`, `cancelled`,
    /// `closed`, `expired`.
    pub fn request_state(&self) -> Option<String> {
        self.with(|s| s.open.as_ref().and_then(|o| o["state"].as_str().map(str::to_string)))
    }

    /// The request expires now, as the task of the server does once a minute.
    pub fn expire_request(&self) {
        self.with(|s| {
            if let Some(open) = s.open.as_mut().filter(|o| o["state"] == "open") {
                open["state"] = json!("expired");
                open["closed_at"] = json!(now());
            }
        });
    }

    /// The network to the fake fails from now on.
    pub fn go_down(&self) {
        self.down.store(true, Ordering::SeqCst);
    }

    /// The reach of the project's servers with the fake as its one host.
    pub fn reach(&self) -> CloudReach {
        CloudReach::Veydan {
            hosts: vec![self.with(|s| s.base.clone())],
        }
    }

    /// The seam `Http` to the fake.
    pub fn http(&self) -> Arc<dyn Http> {
        Arc::new(LoopHttp {
            reach: self.reach(),
            down: self.down.clone(),
            sent: Arc::new(AtomicU64::new(0)),
        })
    }

    /// The client over the fake.
    pub fn client(&self) -> HttpCloud {
        HttpCloud::new(self.http(), Arc::new(FixedSigner))
    }

    /// A mock app with the seams of the messenger, the client, a box in
    /// memory, no lock set and the state of an activation.
    pub fn app(&self) -> tauri::App<tauri::test::MockRuntime> {
        self.app_locked(None)
    }

    /// [`Fake::app`] whose lock is set with `secret` (none: no lock).
    pub fn app_locked(&self, secret: Option<&str>) -> tauri::App<tauri::test::MockRuntime> {
        let app = tauri::test::mock_app();
        veydan_shell::provide::<dyn Http, _, _>(app.handle(), self.http());
        veydan_shell::provide::<dyn Signer, _, _>(app.handle(), Arc::new(FixedSigner));
        let boxed: Arc<dyn LockBox> = Arc::new(MemoryBox::default());
        veydan_shell::provide::<dyn LockBox, _, _>(app.handle(), boxed);
        let gate: Arc<dyn LockGate> = Arc::new(FakeLock(secret.map(str::to_string)));
        veydan_shell::provide::<dyn LockGate, _, _>(app.handle(), gate);
        crate::offer(app.handle());
        app.manage(Activation::default());
        app
    }
}

/// A lock that confirms its one secret, or no lock at all.
pub struct FakeLock(pub Option<String>);

impl LockGate for FakeLock {
    fn confirm<'a>(&'a self, secret: Option<&'a str>) -> SeamFuture<'a, LockConfirm> {
        Box::pin(async move {
            Ok(match (&self.0, secret) {
                (None, _) => LockConfirm::NoLock,
                (Some(mine), Some(given)) if mine == given => LockConfirm::Confirmed,
                _ => LockConfirm::Wrong,
            })
        })
    }
}

/// `Http` over reqwest to loopback.
struct LoopHttp {
    reach: CloudReach,
    down: Arc<AtomicBool>,
    sent: Arc<AtomicU64>,
}

impl Http for LoopHttp {
    fn reach(&self) -> SeamFuture<'_, CloudReach> {
        let reach = self.reach.clone();
        Box::pin(async move { Ok(reach) })
    }

    fn send(&self, request: HttpRequest) -> SeamFuture<'_, HttpResponse> {
        Box::pin(async move {
            if self.down.load(Ordering::SeqCst) {
                return Err(CloudError::new(code::TRANSPORT, "the network is down"));
            }
            self.sent.fetch_add(1, Ordering::SeqCst);
            let client = reqwest::Client::new();
            let method = reqwest::Method::from_bytes(request.method.as_bytes())
                .map_err(|e| CloudError::new(code::BAD_REQUEST, e.to_string()))?;
            let mut builder = client.request(method, &request.url).body(request.body);
            for (name, value) in &request.headers {
                builder = builder.header(name.as_str(), value.as_str());
            }
            let response = builder
                .send()
                .await
                .map_err(|e| CloudError::new(code::TRANSPORT, e.to_string()))?;
            let status = response.status().as_u16();
            let headers = response
                .headers()
                .iter()
                .filter_map(|(n, v)| Some((n.as_str().to_string(), v.to_str().ok()?.to_string())))
                .collect();
            let body = response
                .bytes()
                .await
                .map_err(|e| CloudError::new(code::TRANSPORT, e.to_string()))?
                .to_vec();
            Ok(HttpResponse {
                status,
                headers,
                body,
            })
        })
    }
}

/// A `Signer` of the fixed identity: events of NIP-98 signed with its key
/// (Schnorr, as the messenger's signer signs), now or at a given time.
pub struct FixedSigner;

impl Signer for FixedSigner {
    fn pubkey(&self) -> SeamFuture<'_, String> {
        Box::pin(async { Ok(npub()) })
    }
    fn nip98<'a>(
        &'a self,
        method: &'a str,
        url: &'a str,
        payload: Option<[u8; 32]>,
    ) -> SeamFuture<'a, WireEvent> {
        Box::pin(
            async move { crate::nip98::event_by(&identity_key(), method, url, payload, now()) },
        )
    }
    fn nip98_at<'a>(
        &'a self,
        method: &'a str,
        url: &'a str,
        payload: Option<[u8; 32]>,
        created_at: u64,
    ) -> SeamFuture<'a, WireEvent> {
        Box::pin(async move {
            crate::nip98::event_by(&identity_key(), method, url, payload, created_at)
        })
    }
    fn nip44_encrypt<'a>(&'a self, _: &'a str, _: &'a [u8]) -> SeamFuture<'a, Vec<u8>> {
        Box::pin(async { Err(CloudError::new(code::NOT_SUPPORTED, "fixed")) })
    }
    fn nip44_decrypt<'a>(&'a self, _: &'a str, _: &'a [u8]) -> SeamFuture<'a, Zeroizing<Vec<u8>>> {
        Box::pin(async { Err(CloudError::new(code::NOT_SUPPORTED, "fixed")) })
    }
}

/// A box of the lock in memory, which a test may close.
#[derive(Default)]
pub struct MemoryBox {
    values: Mutex<HashMap<String, Vec<u8>>>,
    closed: AtomicBool,
}

impl MemoryBox {
    /// The lock closes: reads and writes say `locked`.
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    /// Everything kept goes, as a device that lost its data.
    pub fn clear(&self) {
        self.values.lock().unwrap_or_else(|p| p.into_inner()).clear();
    }

    fn open(&self) -> Result<(), CloudError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(CloudError::new(code::LOCKED, "closed"));
        }
        Ok(())
    }
}

impl LockBox for MemoryBox {
    fn get<'a>(&'a self, key: &'a str) -> SeamFuture<'a, Option<Zeroizing<Vec<u8>>>> {
        Box::pin(async move {
            self.open()?;
            let values = self.values.lock().unwrap_or_else(|p| p.into_inner());
            Ok(values.get(key).cloned().map(Zeroizing::new))
        })
    }
    fn put<'a>(&'a self, key: &'a str, value: &'a [u8]) -> SeamFuture<'a, ()> {
        Box::pin(async move {
            self.open()?;
            self.values
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(key.to_string(), value.to_vec());
            Ok(())
        })
    }
    fn delete<'a>(&'a self, key: &'a str) -> SeamFuture<'a, ()> {
        Box::pin(async move {
            self.values
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(key);
            Ok(())
        })
    }
}

/// The client asks the account through the fake: the plumbing holds.
#[tokio::test]
async fn the_fake_answers_the_client() {
    let fake = Fake::start(2).await;
    let api = fake.client();
    assert_eq!(
        api.account().await.map_err(|e| e.code),
        Err("not_activated".to_string())
    );
    fake.add_account();
    assert_eq!(api.account().await.map(|a| a.plan), Ok("free".to_string()));
    assert_eq!(fake.requests(), 2);
}
