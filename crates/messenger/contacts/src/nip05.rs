// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! NIP-05: `user@domain` → public key, via
//! `https://domain/.well-known/nostr.json?name=user`. HTTP is behind a
//! trait so tests never touch the network.
//!
//! The name comes from someone else's profile: the domain is a host name
//! on the open internet and nothing else (no port, path or query), the
//! user is the few characters NIP-05 allows, and the request takes the one
//! rule of the messenger for such addresses (`messenger_http::Target`,
//! `messenger_http::public_builder`).

use async_trait::async_trait;
use messenger_core::{MessengerError, PubKey, Result};
use messenger_http::Target;
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
        let client = messenger_http::public_builder(Duration::from_secs(5), Duration::from_secs(10), 0)
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
        // A literal address never goes to the resolver that checks names.
        let target = Target::parse(url)?;
        let resp = self.client.get(target.url()).send().await.map_err(|e| MessengerError::Transport(e.to_string()))?;
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
        let bad = || MessengerError::Invalid("not a valid NIP-05 identifier".into());
        let id = identifier.trim();
        let (name, domain) = id.split_once('@').unwrap_or(("_", id));
        // The characters NIP-05 allows: nothing that could add to the query.
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')) {
            return Err(bad());
        }
        // A host on the open internet, and only a host: a port, a path or a
        // query would make the address point elsewhere than it says.
        let target = Target::parse(&format!("https://{domain}/"))?;
        if target.host() != domain.trim_end_matches('.').to_ascii_lowercase() {
            return Err(bad());
        }
        Nip05Address::parse(&format!("{name}@{}", target.host())).map_err(|_| bad())
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

    /// Writes down every address it is asked for.
    #[derive(Default)]
    struct Asked(Mutex<Vec<String>>);
    #[async_trait]
    impl Nip05Fetcher for Asked {
        async fn fetch(&self, url: &str) -> Result<String> {
            self.0.lock().unwrap().push(url.to_string());
            Err(MessengerError::Transport("not here".into()))
        }
    }

    /// A name that points inside, or adds a port, a path or a query of its
    /// own, is refused before anything is asked.
    #[tokio::test]
    async fn a_name_that_leads_inside_or_elsewhere_is_not_asked() {
        let asked = Arc::new(Asked::default());
        let svc = Nip05Service::new(asked.clone());
        let pk = PubKey::parse(&Keys::generate().public_key().to_hex()).unwrap();
        for id in [
            "a@127.0.0.1",
            "a@localhost",
            "a@10.0.0.1",
            "a@[::1]",
            "a@169.254.169.254",
            "a@router.lan",
            "x@127.0.0.1:8443/admin?",
            "x@example.com:8443",
            "x@example.com/admin?",
            "a&b=c@example.com",
            "a?b@example.com",
            "@example.com",
            "a@b@example.com",
            "a@http://example.com",
        ] {
            assert!(svc.resolve(id).await.is_err(), "{id}");
            assert!(svc.verify(id, &pk).await.is_err(), "{id}");
        }
        assert!(asked.0.lock().unwrap().is_empty(), "{:?}", asked.0.lock().unwrap());
        // A good name is asked, as NIP-05 writes it.
        let _ = svc.resolve("Alice@Example.COM").await;
        assert_eq!(*asked.0.lock().unwrap(), ["https://example.com/.well-known/nostr.json?name=Alice"]);
    }

    /// The real fetcher refuses a literal address inside by itself.
    #[tokio::test]
    async fn the_fetcher_refuses_an_address_inside() {
        let e = ReqwestFetcher::new().fetch("https://127.0.0.1/.well-known/nostr.json?name=a").await.unwrap_err();
        assert!(e.to_string().contains("preview_private_host"), "{e}");
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
