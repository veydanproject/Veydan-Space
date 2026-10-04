// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Remote manifests: where the project publishes them, who signs them, and
//! how one is fetched.
//!
//! A remote manifest is always the signed event form, signed by the project
//! key pinned here. The places are tried in order and the first one that
//! answers with a valid, correctly signed manifest wins. HTTP is behind a
//! trait so tests never touch the network.

use crate::manifest::Manifest;
use async_trait::async_trait;
use messenger_core::{MessengerError, Result};
use std::time::Duration;

/// Public key (hex) of the project key that signs remote manifests.
pub const PROJECT_MANIFEST_PUBKEY: &str = "9a3df86b487832646de015da85813b568fe3419e5d3d7a6c9a234592a66c6871";

/// Where the project publishes the signed manifest, in the order tried.
///
/// Empty for now: the manifest built into the app is the only one, and the
/// app asks nowhere. Where the signed manifest will live (one place to change
/// it, reachable from Russia, where connections to Cloudflare are cut after
/// ~16 KB) is still to be decided.
pub const MANIFEST_URLS: &[&str] = &[];

/// A manifest is a few kilobytes; anything much larger is not one.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;

#[async_trait]
pub trait ManifestFetcher: Send + Sync {
    /// Body at `url`, or an error.
    async fn fetch(&self, url: &str) -> Result<String>;
}

/// HTTPS fetcher: no redirects, short timeouts, bounded body.
pub struct HttpManifestFetcher {
    client: reqwest::Client,
}

impl HttpManifestFetcher {
    pub fn new() -> Result<Self> {
        let client = messenger_http::builder(Duration::from_secs(5), Duration::from_secs(10))?
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("veydan-messenger")
            .build()
            .map_err(|e| MessengerError::Transport(e.to_string()))?;
        Ok(Self { client })
    }
}

#[async_trait]
impl ManifestFetcher for HttpManifestFetcher {
    async fn fetch(&self, url: &str) -> Result<String> {
        let mut resp = self.client.get(url).send().await.map_err(|e| MessengerError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(MessengerError::Transport(format!("http {}", resp.status())));
        }
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| MessengerError::Transport(e.to_string()))? {
            body.extend_from_slice(&chunk);
            if body.len() > MAX_MANIFEST_BYTES {
                return Err(MessengerError::Invalid("manifest too large".into()));
            }
        }
        String::from_utf8(body).map_err(|_| MessengerError::Invalid("manifest is not utf-8".into()))
    }
}

/// The URLs to try: the built-in ones, then the `http` sources the current
/// manifest names. Only `https://`, no duplicates.
pub fn candidate_urls(builtin: &[&str], current: Option<&Manifest>) -> Vec<String> {
    let mut urls: Vec<String> = builtin.iter().map(|u| u.to_string()).collect();
    if let Some(m) = current {
        let mut sources: Vec<_> = m
            .sources
            .iter()
            .filter_map(|s| match s {
                crate::manifest::ManifestSource::Http { url, priority, .. } => Some((*priority, url.clone())),
                _ => None,
            })
            .collect();
        sources.sort_by_key(|a| std::cmp::Reverse(a.0));
        urls.extend(sources.into_iter().map(|(_, u)| u));
    }
    let mut seen = std::collections::BTreeSet::new();
    urls.retain(|u| u.starts_with("https://") && seen.insert(u.clone()));
    urls
}

/// Try `urls` in order; the first valid manifest signed by `pinned` wins.
/// On failure the error lists what went wrong at each place.
pub async fn fetch_signed(fetcher: &dyn ManifestFetcher, urls: &[String], pinned: &str) -> Result<(Manifest, String)> {
    let mut errors = Vec::new();
    for url in urls {
        let parsed = match fetcher.fetch(url).await {
            Ok(body) if body.len() > MAX_MANIFEST_BYTES => Err(MessengerError::Invalid("manifest too large".into())),
            Ok(body) => Manifest::parse_signed(&body, Some(pinned)),
            Err(e) => Err(e),
        };
        match parsed {
            Ok(m) => return Ok((m, url.clone())),
            Err(e) => errors.push(format!("{url}: {e}")),
        }
    }
    if errors.is_empty() {
        errors.push("no manifest address".into());
    }
    Err(MessengerError::Transport(errors.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{ManifestSource, EMBEDDED_MANIFEST_JSON};
    use nostr::key::Keys;
    use std::collections::HashMap;

    struct Fake(HashMap<String, Result<String>>);

    #[async_trait]
    impl ManifestFetcher for Fake {
        async fn fetch(&self, url: &str) -> Result<String> {
            match self.0.get(url) {
                Some(Ok(s)) => Ok(s.clone()),
                Some(Err(e)) => Err(MessengerError::Transport(e.to_string())),
                None => Err(MessengerError::Transport("http 404".into())),
            }
        }
    }

    fn signed(keys: &Keys, serial: u64) -> String {
        let mut m = Manifest::parse_content(EMBEDDED_MANIFEST_JSON).unwrap();
        m.serial = serial;
        m.sign(keys).unwrap()
    }

    fn urls(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn pinned_key_is_a_valid_public_key() {
        assert!(nostr::key::PublicKey::from_hex(PROJECT_MANIFEST_PUBKEY).is_ok());
        assert!(PROJECT_MANIFEST_PUBKEY.chars().any(|c| c != '0'), "a real key, not a placeholder");
    }

    #[tokio::test]
    async fn first_valid_signed_manifest_wins() {
        let project = Keys::generate();
        let pinned = project.public_key().to_hex();
        let fetcher = Fake(HashMap::from([
            ("https://a/m".to_string(), Ok(signed(&Keys::generate(), 9))),
            ("https://b/m".to_string(), Ok("not json".into())),
            ("https://c/m".to_string(), Ok(signed(&project, 7))),
            ("https://d/m".to_string(), Ok(signed(&project, 8))),
        ]));
        let (m, url) =
            fetch_signed(&fetcher, &urls(&["https://x/m", "https://a/m", "https://b/m", "https://c/m", "https://d/m"]), &pinned)
                .await
                .unwrap();
        assert_eq!((m.serial, url.as_str()), (7, "https://c/m"));
    }

    #[tokio::test]
    async fn every_failure_is_reported() {
        let project = Keys::generate();
        let fetcher = Fake(HashMap::from([
            ("https://a/m".to_string(), Ok(signed(&Keys::generate(), 9))),
            ("https://b/m".to_string(), Ok("x".repeat(MAX_MANIFEST_BYTES + 1))),
        ]));
        let err = fetch_signed(&fetcher, &urls(&["https://a/m", "https://b/m", "https://c/m"]), &project.public_key().to_hex())
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("unexpected key"), "{err}");
        assert!(err.contains("too large"), "{err}");
        assert!(err.contains("404"), "{err}");
        assert!(fetch_signed(&fetcher, &[], "00").await.is_err());
    }

    #[test]
    fn candidates_are_builtin_then_sources_https_only_once() {
        let mut m = Manifest::parse_content(EMBEDDED_MANIFEST_JSON).unwrap();
        m.sources = vec![
            ManifestSource::Http { url: "https://low/m".into(), regions: vec![], priority: 1 },
            ManifestSource::Http { url: "https://a/m".into(), regions: vec![], priority: 5 },
            ManifestSource::Http { url: "http://plain/m".into(), regions: vec![], priority: 9 },
            ManifestSource::Http { url: "https://high/m".into(), regions: vec![], priority: 9 },
        ];
        assert_eq!(
            candidate_urls(&["https://a/m", "https://b/m"], Some(&m)),
            urls(&["https://a/m", "https://b/m", "https://high/m", "https://low/m"])
        );
        assert_eq!(candidate_urls(MANIFEST_URLS, None).len(), MANIFEST_URLS.len());
    }
}
