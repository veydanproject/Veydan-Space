// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Who may use a private server, and how this device gets in
//! (internal/messenger-wire.md, «Режимы доступа сервера»).
//!
//! A server of the kit says its mode over HTTP under the host of its relay
//! (`wss://host[:port]` → `https://host[:port]`; `http` for this machine
//! alone, tests): `GET /.well-known/veydan-access/v1/info` →
//! `{"v":1,"mode":"open|team|channel","invite":bool,"join":"…"}`. A team
//! server lets in its members alone; one becomes one by an invite of its
//! admin, `POST …/v1/join` `{"invite":"vi1.…"}` under NIP-98 with my key.
//! `GET …/v1/me` (NIP-98) says what I am there.
//!
//! The invite is never stored and never logged: it is handed in, sent
//! once, and forgotten.

use crate::MessengerRuntime;
use messenger_core::traits::{Clock, SystemClock, UiEvent};
use messenger_core::{MessengerError, Result};
use messenger_store::servers;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// The paths of the access API of a server.
pub const INFO_PATH: &str = "/.well-known/veydan-access/v1/info";
pub const JOIN_PATH: &str = "/.well-known/veydan-access/v1/join";
pub const ME_PATH: &str = "/.well-known/veydan-access/v1/me";

/// The words the UI reads when a join fails.
pub const ERR_INVITE_INVALID: &str = "server_invite_invalid";
pub const ERR_INVITE_USED: &str = "server_invite_used";
pub const ERR_INVITE_EXPIRED: &str = "server_invite_expired";
pub const ERR_JOIN_FAILED: &str = "server_join_failed";
/// The access API of the server did not answer (`server_access_info`).
pub const ERR_ACCESS_UNREACHABLE: &str = "server_access_unreachable";

/// How long what a server said of its mode is believed.
const INFO_SECS: u64 = 600;

/// What a server says of its access mode.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessInfo {
    /// `open` | `team` | `channel`.
    pub mode: String,
    /// The server lets people in by invites.
    pub invite: bool,
}

/// What I am on a server, as it says (`GET …/v1/me`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessMe {
    pub member: bool,
    #[serde(default)]
    pub admin: bool,
    pub mode: String,
}

#[derive(Deserialize)]
struct InfoAnswer {
    mode: String,
    #[serde(default)]
    invite: bool,
}

#[derive(Deserialize)]
struct ErrorAnswer {
    #[serde(default)]
    error: String,
}

/// What a server said of its mode, by the base of its address, with when.
/// One for the process: a process holds one messenger.
static INFOS: LazyLock<Mutex<HashMap<String, (Instant, AccessInfo)>>> = LazyLock::new(Default::default);

fn cached(base: &str) -> Option<AccessInfo> {
    let infos = INFOS.lock().unwrap_or_else(|p| p.into_inner());
    infos
        .get(base)
        .filter(|(at, _)| at.elapsed() < Duration::from_secs(INFO_SECS))
        .map(|(_, info)| info.clone())
}

fn remember(base: &str, info: &AccessInfo) {
    INFOS.lock().unwrap_or_else(|p| p.into_inner()).insert(base.to_string(), (Instant::now(), info.clone()));
}

fn forget(base: &str) {
    INFOS.lock().unwrap_or_else(|p| p.into_inner()).remove(base);
}

/// The base of the access API of the server of `relay_url`: its host (and
/// port) over `https`, or over `http` for a relay of this machine
/// (`ws://localhost`, `ws://127.0.0.1`, `ws://[::1]`: tests and a kit tried
/// at home). No path, no query: the app key on the relay's address stays
/// there.
pub fn access_base(relay_url: &str) -> Result<String> {
    let bad = || MessengerError::Invalid(format!("{ERR_JOIN_FAILED}: the relay of the server is no ws(s):// address"));
    let url = reqwest::Url::parse(relay_url.trim()).map_err(|_| bad())?;
    let host = url.host_str().filter(|h| !h.is_empty()).ok_or_else(bad)?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(bad());
    }
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    let scheme = match url.scheme() {
        "wss" => "https",
        "ws" if loopback => "http",
        _ => return Err(bad()),
    };
    Ok(match url.port() {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    })
}

fn http() -> Result<reqwest::Client> {
    messenger_http::client(Duration::from_secs(6), Duration::from_secs(15))
}

fn unreachable(e: impl std::fmt::Display) -> MessengerError {
    MessengerError::Transport(format!("{ERR_ACCESS_UNREACHABLE}: {e}"))
}

/// The word of the UI for what a server answered a join with.
fn join_refusal(status: u16, error: &str) -> &'static str {
    match (status, error) {
        (401, "invite_invalid") => ERR_INVITE_INVALID,
        (401, "invite_expired") => ERR_INVITE_EXPIRED,
        (401, "invite_used") => ERR_INVITE_USED,
        _ => ERR_JOIN_FAILED,
    }
}

impl MessengerRuntime {
    /// Private server `id` of mine; `server_unknown` for any other.
    async fn private_row(&self, id: &str) -> Result<servers::ServerRow> {
        servers::get(self.store(), id.trim())
            .await?
            .filter(|s| s.is_private())
            .ok_or_else(|| MessengerError::Invalid(servers::ERR_SERVER_UNKNOWN.into()))
    }

    /// What private server `id` says of its access mode, asked at most
    /// every ten minutes. What it says is kept as the server's mode here
    /// (a card may be older than the server's word), and the UI is told
    /// when that changed. A server whose kit has no access API is open, as
    /// every server before the modes: it answers 404, or 401 (kits of 0.1.8
    /// and before gate everything but the relay in nginx's `location /`).
    pub async fn server_access_info(&self, id: &str) -> Result<AccessInfo> {
        let row = self.private_row(id).await?;
        let base = access_base(&row.relay_url)?;
        let info = match cached(&base) {
            Some(info) => info,
            None => {
                let answer = http()?.get(format!("{base}{INFO_PATH}")).send().await.map_err(unreachable)?;
                let info = match answer.status().as_u16() {
                    401 | 404 => AccessInfo { mode: servers::ACCESS_OPEN.into(), invite: false },
                    200 => {
                        let a: InfoAnswer = answer.json().await.map_err(unreachable)?;
                        AccessInfo { mode: a.mode, invite: a.invite }
                    }
                    other => return Err(unreachable(format!("HTTP {other}"))),
                };
                remember(&base, &info);
                info
            }
        };
        if servers::is_access(&info.mode) && servers::set_access(self.store(), &row.id, &info.mode).await? {
            self.servers_told();
        }
        Ok(info)
    }

    /// Join private server `id` with the invite `code` of its admin, under
    /// NIP-98 with my key: from then on its relay lets this device in. The
    /// refusal kept for it is forgotten, the UI is told, and what the
    /// relay refused before is asked and sent again. The words of a
    /// failure: `server_invite_invalid`, `server_invite_used`,
    /// `server_invite_expired` (the server said so of the invite),
    /// `server_join_failed` (anything else: the server unreachable, my
    /// signature refused, too many tries). An open server needs no invite:
    /// the join is done. The code is never stored nor logged.
    pub async fn server_join(&self, id: &str, code: &str) -> Result<()> {
        let row = self.private_row(id).await?;
        let code = messenger_links::server::invite_code(Some(code.trim().to_string()))
            .ok()
            .flatten()
            .ok_or_else(|| MessengerError::Invalid(ERR_INVITE_INVALID.into()))?;
        let keys = self.session_keys().await?;
        let base = access_base(&row.relay_url)?;
        let url = format!("{base}{JOIN_PATH}");
        let body = serde_json::to_vec(&serde_json::json!({ "invite": code }))?;
        let at = SystemClock.now().secs().max(0) as u64;
        let auth = messenger_http::nip98::auth_header(&keys, "POST", &url, &body, at)?;
        let failed = |why: String| MessengerError::Invalid(format!("{ERR_JOIN_FAILED}: {why}"));
        let answer = http()?
            .post(&url)
            .header(reqwest::header::AUTHORIZATION, auth)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| failed(e.to_string()))?;
        let status = answer.status().as_u16();
        let text = answer.text().await.unwrap_or_default();
        let error = serde_json::from_str::<ErrorAnswer>(&text).map(|e| e.error).unwrap_or_default();
        match status {
            200 => {
                let member = serde_json::from_str::<serde_json::Value>(&text)
                    .ok()
                    .and_then(|v| v.get("member").and_then(|m| m.as_bool()))
                    .unwrap_or(false);
                if !member {
                    return Err(failed("the server did not make me a member".into()));
                }
            }
            // The server is open: there is nothing to join.
            403 if error == "mode_open" => {
                servers::set_access(self.store(), &row.id, servers::ACCESS_OPEN).await?;
            }
            401 => {
                let word = join_refusal(status, &error);
                return Err(if word == ERR_JOIN_FAILED {
                    failed(format!("HTTP 401 {error}"))
                } else {
                    MessengerError::Invalid(word.into())
                });
            }
            other => return Err(failed(format!("HTTP {other} {error}"))),
        }
        forget(&base);
        servers::clear_denied(self.store(), &row.id).await?;
        self.server_let_in(&row.id).await;
        Ok(())
    }

    /// What I am on private server `id`, as it says (`GET …/v1/me` under
    /// NIP-98 with my key).
    pub async fn server_access_me(&self, id: &str) -> Result<AccessMe> {
        let row = self.private_row(id).await?;
        let keys = self.session_keys().await?;
        let url = format!("{}{ME_PATH}", access_base(&row.relay_url)?);
        let at = SystemClock.now().secs().max(0) as u64;
        let auth = messenger_http::nip98::auth_header(&keys, "GET", &url, b"", at)?;
        let answer = http()?
            .get(&url)
            .header(reqwest::header::AUTHORIZATION, auth)
            .send()
            .await
            .map_err(unreachable)?;
        match answer.status().as_u16() {
            200 => answer.json().await.map_err(unreachable),
            other => Err(unreachable(format!("HTTP {other}"))),
        }
    }

    /// A server let this device in (a join). Its relay is connected anew,
    /// so the connection signs in as a member's; the UI is told, so «Нет
    /// доступа» goes, and so is the session (`joined`): it asks again what
    /// the relay refused before (the inbox of the server and its groups,
    /// with their history), which `nostr-sdk` dropped for good on the
    /// `CLOSED restricted:`. What the outbox holds for it is tried now.
    pub(crate) async fn server_let_in(&self, id: &str) {
        self.relays.pool().await.reopen_server(id);
        let _ = self.ui.send(UiEvent {
            name: super::UI_EVENT_SERVERS_UPDATED.into(),
            payload: serde_json::json!({ JOINED: id }),
        });
        self.group_driver.server_joined(id).await;
        if let Err(e) = messenger_store::outbox::due_now(self.store(), SystemClock.now().secs()).await {
            eprintln!("messenger servers: the outbox was not made due: {e}");
        }
        self.outbox.kick();
    }
}

/// The field of `UI_EVENT_SERVERS_UPDATED` that names a server that let
/// this device in (a join): the session listens to it anew.
pub const JOINED: &str = "joined";

/// What the relays of private servers say of this device's access, kept
/// for as long as the session runs: a refusal (`restricted:`,
/// `auth-required:`) is kept as `servers.denied@<id>` and the UI is told
/// («Нет доступа»); an event taken there forgets it; a request answered
/// there forgets a refusal of the sign-in (`auth-required:`), which the
/// sign-in that follows it answers. Only for private servers of mine.
pub(crate) async fn denied_loop(
    mut signals: tokio::sync::broadcast::Receiver<messenger_transport::RelayAccess>,
    store: messenger_store::Store,
    ui: tokio::sync::broadcast::Sender<UiEvent>,
) {
    use messenger_transport::AccessVerdict;
    use tokio::sync::broadcast::error::RecvError;
    loop {
        let signal = match signals.recv().await {
            Ok(s) => s,
            Err(RecvError::Lagged(_)) => continue,
            Err(RecvError::Closed) => return,
        };
        let id = signal.server_id.as_str();
        let changed = match &signal.verdict {
            AccessVerdict::Denied(text) => match servers::get(&store, id).await {
                Ok(Some(row)) if row.is_private() => servers::set_denied(&store, id, text).await,
                Ok(_) => Ok(false),
                Err(e) => Err(e),
            },
            AccessVerdict::Written => servers::clear_denied(&store, id).await,
            AccessVerdict::Read => match servers::denied(&store, id).await {
                Ok(Some(text)) if text.starts_with("auth-required:") => servers::clear_denied(&store, id).await,
                Ok(_) => Ok(false),
                Err(e) => Err(e),
            },
        };
        match changed {
            Ok(true) => {
                let _ = ui.send(UiEvent {
                    name: super::UI_EVENT_SERVERS_UPDATED.into(),
                    payload: serde_json::json!({ "denied": id }),
                });
            }
            Ok(false) => {}
            Err(e) => eprintln!("messenger servers: the access of a server was not kept: {e}"),
        }
    }
}

#[cfg(test)]
mod tests;
