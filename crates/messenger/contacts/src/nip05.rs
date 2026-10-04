// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! NIP-05: `user@domain` → public key, via
//! `https://domain/.well-known/nostr.json?name=user`. HTTP is behind a
//! trait so tests never touch the network.

use async_trait::async_trait;
use messenger_core::{MessengerError, PubKey, Result};
use nostr::nips::nip05::{verify_from_raw_json, Nip05Address, Nip05Profile};
use nostr::prelude::*;
use std::sync::Arc;
use std::time::Duration;

#[async_trait]
pub trait Nip05Fetcher: Send + Sync {
    /// Body of the well-known document for `address`, or an error.
    async fn fetch(&self, url: &str) -> Result<String>;
}

pub struct ReqwestFetcher {
    client: reqwest::Client,
}

impl ReqwestFetcher {
    pub fn new() -> Self {
        let client = messenger_http::builder(Duration::from_secs(5), Duration::from_secs(10))
            .expect("tls configuration")
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("veydan-messenger")
            .build()
            .expect("reqwest client");
        Self { client }
    }
}

impl Default for ReqwestFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Nip05Fetcher for ReqwestFetcher {
    async fn fetch(&self, url: &str) -> Result<String> {
        let resp = self.client.get(url).send().await.map_err(|e| MessengerError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(MessengerError::Transport(format!("nip05: http {}", resp.status())));
        }
        let bytes = resp.bytes().await.map_err(|e| MessengerError::Transport(e.to_string()))?;
        if bytes.len() > 64 * 1024 {
            return Err(MessengerError::Invalid("nip05: document too large".into()));
        }
        String::from_utf8(bytes.to_vec()).map_err(|_| MessengerError::Invalid("nip05: not utf-8".into()))
    }
}

pub struct Nip05Service {
    fetcher: Arc<dyn Nip05Fetcher>,
}

impl Nip05Service {
    pub fn new(fetcher: Arc<dyn Nip05Fetcher>) -> Self {
        Self { fetcher }
    }

    fn address(identifier: &str) -> Result<Nip05Address> {
        let id = identifier.trim();
        let id = if id.contains('@') { id.to_string() } else { format!("_@{id}") };
        Nip05Address::parse(&id).map_err(|_| MessengerError::Invalid("not a valid NIP-05 identifier".into()))
    }

    /// `user@domain` (or bare `domain` for `_@domain`) → public key.
    pub async fn resolve(&self, identifier: &str) -> Result<PubKey> {
        let addr = Self::address(identifier)?;
        let raw = self.fetcher.fetch(addr.url().as_str()).await?;
        let profile = Nip05Profile::from_raw_json(&addr, &raw)
            .map_err(|_| MessengerError::Invalid("name not found in the NIP-05 document".into()))?;
        Ok(PubKey::parse(&profile.public_key.to_hex()).expect("valid pubkey"))
    }

    /// Does `identifier` really point at `pubkey`?
    pub async fn verify(&self, identifier: &str, pubkey: &PubKey) -> Result<bool> {
        let addr = Self::address(identifier)?;
        let raw = self.fetcher.fetch(addr.url().as_str()).await?;
        let pk = PublicKey::from_hex(pubkey.as_hex()).map_err(|_| MessengerError::Invalid("bad pubkey".into()))?;
        Ok(verify_from_raw_json(&pk, &addr, &raw).unwrap_or(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct Fake(Mutex<HashMap<String, String>>);
    #[async_trait]
    impl Nip05Fetcher for Fake {
        async fn fetch(&self, url: &str) -> Result<String> {
            self.0.lock().unwrap().get(url).cloned().ok_or_else(|| MessengerError::Transport(format!("404 {url}")))
        }
    }

    #[tokio::test]
    async fn resolve_and_verify_with_fake_http() {
        let keys = Keys::generate();
        let hex = keys.public_key().to_hex();
        let doc = format!(r#"{{"names":{{"alice":"{hex}","_":"{hex}"}}}}"#);
        let fake = Fake(Mutex::new(HashMap::from([
            ("https://example.com/.well-known/nostr.json?name=alice".to_string(), doc.clone()),
            ("https://example.com/.well-known/nostr.json?name=_".to_string(), doc),
        ])));
        let svc = Nip05Service::new(Arc::new(fake));
        let pk = svc.resolve("alice@example.com").await.unwrap();
        assert_eq!(pk.as_hex(), hex);
        assert_eq!(svc.resolve("example.com").await.unwrap().as_hex(), hex, "bare domain means _@domain");
        assert!(svc.verify("alice@example.com", &pk).await.unwrap());
        let other = PubKey::parse(&Keys::generate().public_key().to_hex()).unwrap();
        assert!(!svc.verify("alice@example.com", &other).await.unwrap());
        assert!(svc.resolve("bob@example.com").await.is_err(), "name missing → error");
        assert!(svc.resolve("nobody@nowhere.example").await.is_err(), "http error surfaces");
        assert!(svc.resolve("not an identifier").await.is_err());
    }
}
