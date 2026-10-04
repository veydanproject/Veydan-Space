// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The network. Three places can lead a request inside, and each is
//! closed: the address itself (`Target`), every redirect, and what a name
//! resolves to.

use crate::target::{is_public, Target};
use crate::refuse;
use async_trait::async_trait;
use messenger_core::{MessengerError, Result};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

const MAX_REDIRECTS: usize = 3;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(8);
/// Says what asks, and nothing about who.
const USER_AGENT: &str = "Mozilla/5.0 (compatible; link-preview)";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fetched {
    /// Where the answer came from, after redirects.
    pub url: String,
    /// Lowercase, without parameters: `text/html`.
    pub content_type: String,
    /// At most as many bytes as were asked for.
    pub body: Vec<u8>,
    /// There was more than was asked for.
    pub truncated: bool,
}

#[async_trait]
pub trait Fetcher: Send + Sync {
    async fn get(&self, target: &Target, accept: &str, max_bytes: usize) -> Result<Fetched>;
}

/// Resolves a name and drops every address that is not on the open internet.
struct PublicOnly;

impl Resolve for PublicOnly {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let found = tokio::net::lookup_host((name.as_str(), 0)).await?;
            let open: Vec<SocketAddr> = found.filter(|a| is_public(a.ip())).collect();
            if open.is_empty() {
                return Err("preview_private_host".into());
            }
            let addrs: Addrs = Box::new(open.into_iter());
            Ok(addrs)
        })
    }
}

pub struct ReqwestFetcher {
    client: reqwest::Client,
}

impl ReqwestFetcher {
    pub fn new() -> Result<Self> {
        let redirects = reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.error("preview_too_many_redirects");
            }
            match Target::parse(attempt.url().as_str()) {
                Ok(_) => attempt.follow(),
                Err(_) => attempt.error("preview_private_host"),
            }
        });
        let client = messenger_http::builder(CONNECT_TIMEOUT, TOTAL_TIMEOUT)?
            .user_agent(USER_AGENT)
            .redirect(redirects)
            .referer(false)
            .dns_resolver(Arc::new(PublicOnly))
            .build()
            .map_err(|e| MessengerError::Transport(e.to_string()))?;
        Ok(Self { client })
    }
}

/// A refusal made on the way keeps its code; anything else is "failed".
fn failure(e: reqwest::Error) -> MessengerError {
    let mut source: Option<&dyn std::error::Error> = Some(&e);
    while let Some(s) = source {
        let text = s.to_string();
        for code in ["preview_private_host", "preview_too_many_redirects"] {
            if text.contains(code) {
                return refuse(code);
            }
        }
        source = s.source();
    }
    if e.is_timeout() {
        return refuse("preview_timeout");
    }
    refuse("preview_failed")
}

#[async_trait]
impl Fetcher for ReqwestFetcher {
    async fn get(&self, target: &Target, accept: &str, max_bytes: usize) -> Result<Fetched> {
        let mut resp = self
            .client
            .get(target.url())
            .header(reqwest::header::ACCEPT, accept)
            .send()
            .await
            .map_err(failure)?;
        if !resp.status().is_success() {
            return Err(refuse("preview_failed"));
        }
        let url = resp.url().to_string();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(';').next().unwrap_or_default().trim().to_ascii_lowercase())
            .unwrap_or_default();
        let mut body = Vec::new();
        let mut truncated = false;
        while let Some(chunk) = resp.chunk().await.map_err(failure)? {
            let room = max_bytes - body.len();
            if chunk.len() > room {
                body.extend_from_slice(&chunk[..room]);
                truncated = true;
                break;
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Fetched { url, content_type, body, truncated })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_client_builds() {
        assert!(ReqwestFetcher::new().is_ok());
    }

    #[tokio::test]
    async fn a_name_that_leads_inside_is_not_resolved() {
        let name: Name = "localhost".parse().unwrap();
        let err = PublicOnly.resolve(name).await.err().expect("localhost is this machine");
        assert!(err.to_string().contains("preview_private_host"), "{err}");
    }
}
