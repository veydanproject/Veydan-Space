// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The requests to a push server.

use std::time::Duration;

use messenger_core::MessengerError;
use nostr::key::Keys;
use serde::de::DeserializeOwned;

use crate::dto::{DeviceAnswer, DevicePut, ErrorBody, Info, TestAnswer};
use crate::nip98;

/// What went wrong with a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushError {
    /// The address is not one a push server can have.
    Address(String),
    /// No answer: no network, no such host, a timeout.
    Unreachable(String),
    /// The server answered with a refusal.
    Refused {
        status: u16,
        /// The server's word for the reason: `unknown_app`, `auth_expired`,
        /// `token_invalid`. Kept as the server wrote it, so a word of a
        /// newer server is told apart like the ones known today.
        code: String,
        message: String,
        /// Id of the request in the server's log.
        request_id: String,
    },
    /// The answer cannot be read: not a push server, or not this protocol.
    Unreadable(String),
    /// The request could not be signed.
    Signing(String),
}

impl std::fmt::Display for PushError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Address(why) => write!(f, "push_address: {why}"),
            Self::Unreachable(why) => write!(f, "push_unreachable: {why}"),
            Self::Refused { code, message, request_id, .. } => {
                write!(f, "push_refused_{code}: {message} (request {request_id})")
            }
            Self::Unreadable(why) => write!(f, "push_unreadable: {why}"),
            Self::Signing(why) => write!(f, "push_signing: {why}"),
        }
    }
}

impl std::error::Error for PushError {}

impl From<PushError> for MessengerError {
    fn from(e: PushError) -> Self {
        match e {
            PushError::Address(_) => MessengerError::Invalid(e.to_string()),
            PushError::Signing(_) => MessengerError::Crypto(e.to_string()),
            _ => MessengerError::Transport(e.to_string()),
        }
    }
}

pub type PushResult<T> = std::result::Result<T, PushError>;

/// The address of a server as it is used: `https`, no trailing slash, no
/// query. `http` is for this machine, for tests.
pub fn server_address(url: &str) -> PushResult<String> {
    let url = url.trim().trim_end_matches('/');
    let bad = |why: &str| Err(PushError::Address(why.to_string()));
    let rest = if let Some(rest) = url.strip_prefix("https://") {
        rest
    } else if let Some(rest) = url.strip_prefix("http://") {
        let host = rest.split(['/', ':']).next().unwrap_or_default();
        if !matches!(host, "localhost" | "127.0.0.1") {
            return bad("the address must begin with https://");
        }
        rest
    } else {
        return bad("the address must begin with https://");
    };
    if rest.is_empty() || rest.starts_with('/') {
        return bad("the address has no host");
    }
    if rest.contains(['?', '#', '@', ' ']) {
        return bad("the address must be a host and, at most, a path");
    }
    Ok(url.to_string())
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub struct VpushClient {
    http: reqwest::Client,
    base: String,
}

impl VpushClient {
    pub fn new(server: &str) -> PushResult<Self> {
        let http = messenger_http::client(Duration::from_secs(10), Duration::from_secs(30))
            .map_err(|e| PushError::Unreachable(e.to_string()))?;
        Ok(Self {
            http,
            base: server_address(server)?,
        })
    }

    pub fn server(&self) -> &str {
        &self.base
    }

    fn device_url(&self, device_id: &str) -> String {
        format!("{}/v1/devices/{device_id}", self.base)
    }

    async fn send(
        &self,
        keys: Option<&Keys>,
        method: reqwest::Method,
        url: &str,
        body: &[u8],
        at: u64,
    ) -> PushResult<reqwest::Response> {
        let mut request = self.http.request(method.clone(), url);
        if let Some(keys) = keys {
            let header = nip98::auth_header(keys, method.as_str(), url, body, at)
                .map_err(|e| PushError::Signing(e.to_string()))?;
            request = request.header(reqwest::header::AUTHORIZATION, header);
        }
        if !body.is_empty() {
            request = request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.to_vec());
        }
        request
            .send()
            .await
            .map_err(|e| PushError::Unreachable(e.without_url().to_string()))
    }

    /// One request; when the server says the clock is off, once more with
    /// the clock corrected by what the server said.
    async fn ask(
        &self,
        keys: Option<&Keys>,
        method: reqwest::Method,
        url: String,
        body: Vec<u8>,
    ) -> PushResult<(u16, String)> {
        let mut at = now();
        for attempt in 0..2 {
            let response = self.send(keys, method.clone(), &url, &body, at).await?;
            let status = response.status().as_u16();
            let request_id = response
                .headers()
                .get("x-request-id")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            let text = response
                .text()
                .await
                .map_err(|e| PushError::Unreachable(e.without_url().to_string()))?;
            if (200..300).contains(&status) {
                return Ok((status, text));
            }
            let refusal = match serde_json::from_str::<ErrorBody>(&text) {
                Ok(body) => body,
                Err(_) => {
                    return Err(PushError::Unreadable(format!(
                        "the server answered {status}, and not in the words of a push server"
                    )))
                }
            };
            if attempt == 0 && refusal.error.code == "auth_expired" {
                if let Some(server_time) = refusal.error.server_time {
                    at = server_time;
                    continue;
                }
            }
            return Err(PushError::Refused {
                status,
                code: refusal.error.code,
                message: refusal.error.message,
                request_id: if refusal.request_id.is_empty() { request_id } else { refusal.request_id },
            });
        }
        unreachable!("the loop returns on its second round")
    }

    fn read<T: DeserializeOwned>(text: &str) -> PushResult<T> {
        serde_json::from_str(text)
            .map_err(|e| PushError::Unreadable(format!("the answer cannot be read: {e}")))
    }

    /// What the server says about itself. Not signed: nothing about the
    /// user is said by asking.
    pub async fn info(&self) -> PushResult<Info> {
        let url = format!("{}/v1/info", self.base);
        let (_, text) = self.ask(None, reqwest::Method::GET, url, Vec::new()).await?;
        Self::read(&text)
    }

    pub async fn put_device(
        &self,
        keys: &Keys,
        device_id: &str,
        device: &DevicePut,
    ) -> PushResult<DeviceAnswer> {
        let body = serde_json::to_vec(device)
            .map_err(|e| PushError::Unreadable(e.to_string()))?;
        let (_, text) = self
            .ask(Some(keys), reqwest::Method::PUT, self.device_url(device_id), body)
            .await?;
        Self::read(&text)
    }

    /// Removes the device. Removing what is not there is not an error.
    pub async fn delete_device(&self, keys: &Keys, device_id: &str) -> PushResult<()> {
        self.ask(Some(keys), reqwest::Method::DELETE, self.device_url(device_id), Vec::new())
            .await
            .map(|_| ())
    }

    pub async fn test(&self, keys: &Keys, device_id: &str) -> PushResult<TestAnswer> {
        let url = format!("{}/test", self.device_url(device_id));
        let (_, text) = self.ask(Some(keys), reqwest::Method::POST, url, Vec::new()).await?;
        Self::read(&text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The app tells refusals apart by the server's word in the error.
    #[test]
    fn a_refusal_is_named_by_the_servers_word_for_it() {
        let refused = |code: &str| PushError::Refused {
            status: 422,
            code: code.into(),
            message: "why".into(),
            request_id: "5f3a9c1e".into(),
        };
        // The push service does not know the token the phone registered.
        assert_eq!(
            refused("token_invalid").to_string(),
            "push_refused_token_invalid: why (request 5f3a9c1e)"
        );
        assert_eq!(
            refused("limit_devices_total").to_string(),
            "push_refused_limit_devices_total: why (request 5f3a9c1e)"
        );
        assert!(matches!(
            MessengerError::from(refused("token_invalid")),
            MessengerError::Transport(text) if text.starts_with("push_refused_token_invalid")
        ));
    }

    #[test]
    fn addresses() {
        for (given, used) in [
            ("https://vpush.veydan.net", "https://vpush.veydan.net"),
            (" https://vpush.veydan.net/ ", "https://vpush.veydan.net"),
            ("https://example.org/push/", "https://example.org/push"),
            ("http://localhost:8090", "http://localhost:8090"),
            ("http://127.0.0.1:8090/", "http://127.0.0.1:8090"),
        ] {
            assert_eq!(server_address(given).unwrap(), used, "{given}");
        }
        for bad in [
            "",
            "vpush.veydan.net",
            "http://vpush.veydan.net",
            "wss://vpush.veydan.net",
            "https://",
            "https:///path",
            "https://vpush.veydan.net/?key=1",
            "https://user@vpush.veydan.net",
            "https://vpush.veydan.net/#x",
        ] {
            assert!(server_address(bad).is_err(), "`{bad}` was accepted");
        }
    }
}
