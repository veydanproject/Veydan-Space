// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Kind-0 profile cache and our own profile.

use messenger_core::outbound::WireEvent;
use messenger_core::{EventId, MessengerError, PubKey, Result, Timestamp};
use messenger_store::{profiles as repo, Store};
use nostr::key::Keys;
use nostr::nips::nip01::Metadata;
use nostr::prelude::*;
use serde::{Deserialize, Serialize};

/// Profile as the UI sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileView {
    pub pubkey: String,
    pub npub: String,
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub about: Option<String>,
    pub picture: Option<String>,
    pub banner: Option<String>,
    pub website: Option<String>,
    pub nip05: Option<String>,
    pub lud16: Option<String>,
    pub nip05_verified: bool,
    pub event_created_at: i64,
    pub fetched_at: i64,
}

impl ProfileView {
    /// Best human label: display_name → name → nip05 → short npub.
    pub fn label(&self) -> String {
        self.display_name
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| self.name.clone().filter(|s| !s.trim().is_empty()))
            .or_else(|| self.nip05.clone())
            .unwrap_or_else(|| format!("{}…{}", &self.npub[..12], &self.npub[self.npub.len() - 4..]))
    }
}

/// Fields the user edits for their own profile.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileInput {
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub about: Option<String>,
    pub picture: Option<String>,
    pub banner: Option<String>,
    pub website: Option<String>,
    pub nip05: Option<String>,
    pub lud16: Option<String>,
}

#[derive(Clone)]
pub struct ProfileService {
    store: Store,
}

impl ProfileService {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    /// Apply a kind-0 event. Returns `true` when the cache changed.
    pub async fn apply_event(&self, author: &PubKey, created_at: Timestamp, content: &str) -> Result<bool> {
        let meta: Metadata = serde_json::from_str(content).unwrap_or_default();
        let row = repo::ProfileRow {
            pubkey: author.as_hex().to_string(),
            name: clean(meta.name),
            display_name: clean(meta.display_name),
            about: clean(meta.about),
            picture: clean_url(meta.picture),
            banner: clean_url(meta.banner),
            website: clean_url(meta.website),
            nip05: clean(meta.nip05).map(|s| s.to_lowercase()),
            lud16: clean(meta.lud16),
            nip05_verified_at: None,
            event_created_at: created_at.secs(),
            fetched_at: 0,
            raw_json: content.chars().take(16 * 1024).collect(),
        };
        repo::upsert_if_newer(&self.store, &row).await
    }

    pub async fn get(&self, pubkey: &PubKey) -> Result<Option<ProfileView>> {
        Ok(repo::get(&self.store, pubkey.as_hex()).await?.map(to_view))
    }

    pub async fn get_many(&self, pubkeys: &[PubKey]) -> Result<Vec<ProfileView>> {
        let keys: Vec<String> = pubkeys.iter().map(|p| p.as_hex().to_string()).collect();
        Ok(repo::get_many(&self.store, &keys).await?.into_iter().map(to_view).collect())
    }

    pub async fn search(&self, query: &str, limit: i64) -> Result<Vec<ProfileView>> {
        if query.trim().is_empty() {
            return Ok(vec![]);
        }
        Ok(repo::search(&self.store, query, limit).await?.into_iter().map(to_view).collect())
    }

    /// Build and sign our kind-0 from `input`, and store it locally with the
    /// event's timestamp so the cache reflects what we published.
    pub async fn build_own(&self, keys: &Keys, input: &ProfileInput) -> Result<WireEvent> {
        let mut meta = Metadata::new();
        if let Some(v) = clean(input.name.clone()) {
            meta = meta.name(v);
        }
        if let Some(v) = clean(input.display_name.clone()) {
            meta = meta.display_name(v);
        }
        if let Some(v) = clean(input.about.clone()) {
            meta = meta.about(v);
        }
        if let Some(v) = clean_url(input.picture.clone()) {
            meta = meta.picture(Url::parse(&v).map_err(|_| MessengerError::Invalid("picture must be a URL".into()))?);
        }
        if let Some(v) = clean_url(input.banner.clone()) {
            meta = meta.banner(Url::parse(&v).map_err(|_| MessengerError::Invalid("banner must be a URL".into()))?);
        }
        if let Some(v) = clean_url(input.website.clone()) {
            meta = meta.website(Url::parse(&v).map_err(|_| MessengerError::Invalid("website must be a URL".into()))?);
        }
        if let Some(v) = clean(input.nip05.clone()) {
            meta = meta.nip05(v.to_lowercase());
        }
        if let Some(v) = clean(input.lud16.clone()) {
            meta = meta.lud16(v);
        }
        let event = meta.finalize(keys).map_err(|e| MessengerError::Crypto(e.to_string()))?;
        let me = PubKey::parse(&keys.public_key().to_hex()).expect("valid pubkey");
        self.apply_event(&me, Timestamp(event.created_at.as_secs() as i64), &event.content).await?;
        Ok(WireEvent { id: EventId::parse(&event.id.to_hex()).expect("hex id"), json: serde_json::to_value(&event)? })
    }

    pub async fn set_nip05_verified(&self, pubkey: &PubKey, verified_at: Option<i64>) -> Result<()> {
        repo::set_nip05_verified(&self.store, pubkey.as_hex(), verified_at).await
    }
}

fn clean(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn clean_url(v: Option<String>) -> Option<String> {
    clean(v).filter(|s| s.starts_with("https://") || s.starts_with("http://"))
}

fn to_view(r: repo::ProfileRow) -> ProfileView {
    let npub = PublicKey::from_hex(&r.pubkey)
        .ok()
        .and_then(|p| p.to_bech32().ok())
        .unwrap_or_else(|| r.pubkey.clone());
    ProfileView {
        pubkey: r.pubkey,
        npub,
        name: r.name,
        display_name: r.display_name,
        about: r.about,
        picture: r.picture,
        banner: r.banner,
        website: r.website,
        nip05: r.nip05,
        lud16: r.lud16,
        nip05_verified: r.nip05_verified_at.is_some(),
        event_created_at: r.event_created_at,
        fetched_at: r.fetched_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pk(keys: &Keys) -> PubKey {
        PubKey::parse(&keys.public_key().to_hex()).unwrap()
    }

    #[tokio::test]
    async fn apply_event_parses_and_lww() {
        let store = Store::open_in_memory().await.unwrap();
        let svc = ProfileService::new(store);
        let k = Keys::generate();
        assert!(svc
            .apply_event(&pk(&k), Timestamp(10), r#"{"name":" alice ","display_name":"Alice","picture":"javascript:x","nip05":"Alice@Example.com"}"#)
            .await
            .unwrap());
        let v = svc.get(&pk(&k)).await.unwrap().unwrap();
        assert_eq!(v.name.as_deref(), Some("alice"));
        assert_eq!(v.picture, None, "non-http picture dropped");
        assert_eq!(v.nip05.as_deref(), Some("alice@example.com"));
        assert_eq!(v.label(), "Alice");
        assert!(v.npub.starts_with("npub1"));
        assert!(!svc.apply_event(&pk(&k), Timestamp(5), r#"{"name":"old"}"#).await.unwrap());
        assert!(svc.apply_event(&pk(&k), Timestamp(11), "not json").await.unwrap(), "garbage content clears fields but is a newer event");
        let v = svc.get(&pk(&k)).await.unwrap().unwrap();
        assert_eq!(v.name, None);
        assert!(v.label().starts_with("npub1"));
    }

    #[tokio::test]
    async fn build_own_signs_and_caches() {
        let store = Store::open_in_memory().await.unwrap();
        let svc = ProfileService::new(store);
        let k = Keys::generate();
        let input = ProfileInput { name: Some("me".into()), picture: Some("https://x.example/a.png".into()), ..Default::default() };
        let ev = svc.build_own(&k, &input).await.unwrap();
        let parsed: Event = serde_json::from_value(ev.json).unwrap();
        assert_eq!(parsed.kind.as_u16(), 0);
        assert!(parsed.verify().is_ok());
        let cached = svc.get(&pk(&k)).await.unwrap().unwrap();
        assert_eq!(cached.name.as_deref(), Some("me"));
        assert_eq!(cached.picture.as_deref(), Some("https://x.example/a.png"));
        assert!(svc.build_own(&k, &ProfileInput { picture: Some("https://bad url".into()), ..Default::default() }).await.is_err());
        assert!(svc.search("me", 5).await.unwrap().len() == 1);
    }
}
