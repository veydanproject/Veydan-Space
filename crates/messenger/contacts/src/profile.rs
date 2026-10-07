// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Kind-0 profile cache and our own profile.
//!
//! A kind 0 is the standard NIP-01 JSON with two keys of ours:
//! `veydan_about`, the bio with its marks (messenger-richtext), and
//! `veydan_socials`, links elsewhere as `[{"p":"…","h":"…"}]`. `about` is
//! the bio without the marks, so other clients show the same text.
//!
//! Reading: the bio is the marked one only when its text without the marks
//! is the `about` beside it; a client that edited `about` and kept our key
//! wins, and the bio is read from `about` as plain text. Links are kept
//! only when they pass the checks of `social`. The content is untrusted:
//! every key is optional, a key of the wrong type is as good as absent, and
//! a content larger than `MAX_CONTENT_BYTES` is not read at all.
//!
//! Writing: our kind 0 starts from the last one cached for us (from this
//! device or another), changes only the keys it manages and keeps every
//! other key, `banner` and the keys of other clients. `picture` is never
//! typed by the user: it comes from the own avatar (`Picture`).

use crate::social::{self, SocialLink, SocialView};
use messenger_core::outbound::WireEvent;
use messenger_core::{EventId, MessengerError, PubKey, Result, Timestamp};
use messenger_richtext::{self as richtext, Span};
use messenger_store::{profiles as repo, Store};
use nostr::key::Keys;
use nostr::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use ts_rs::TS;

/// A kind 0 with a larger content is not read; ours is refused before it
/// is signed.
pub const MAX_CONTENT_BYTES: usize = 64 * 1024;

/// The keys our kind 0 writes; every other key is kept as it was.
/// `displayName` is the old spelling of `display_name`, dropped so it does
/// not show a stale name elsewhere.
const MANAGED: [&str; 10] = [
    "name",
    "display_name",
    "displayName",
    "about",
    "website",
    "nip05",
    "lud16",
    "picture",
    "veydan_about",
    "veydan_socials",
];

/// Profile as the UI sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ProfileView {
    pub pubkey: String,
    pub npub: String,
    pub name: Option<String>,
    pub display_name: Option<String>,
    /// As other clients see it: the bio without its marks.
    pub about: Option<String>,
    pub picture: Option<String>,
    pub banner: Option<String>,
    pub website: Option<String>,
    pub nip05: Option<String>,
    pub lud16: Option<String>,
    pub nip05_verified: bool,
    #[ts(type = "number")]
    pub event_created_at: i64,
    #[ts(type = "number")]
    pub fetched_at: i64,
    /// The bio to show.
    pub bio: Vec<Span>,
    /// The bio to edit, always markup: ours when it agrees with `about`,
    /// else `about` with its marks escaped, so that saving it unchanged
    /// gives other clients the same `about` again.
    pub bio_source: Option<String>,
    pub socials: Vec<SocialView>,
    /// `socials` in their stored form, for a contact card; not for the UI.
    #[serde(skip)]
    pub links: Vec<SocialLink>,
}

impl ProfileView {
    /// Best human label: display_name → name → nip05 → short npub.
    pub fn label(&self) -> String {
        self.display_name
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| self.name.clone().filter(|s| !s.trim().is_empty()))
            .or_else(|| self.nip05.clone())
            .unwrap_or_else(|| short_npub(&self.npub))
    }
}

/// `npub1abcdefgh…wxyz`; a key too short for it as it is.
pub(crate) fn short_npub(npub: &str) -> String {
    let chars: Vec<char> = npub.chars().collect();
    if chars.len() <= 16 {
        return npub.to_string();
    }
    let head: String = chars[..12].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

/// Fields the user edits for their own profile. There is no picture: the
/// avatar is set by its own commands.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ProfileInput {
    pub name: Option<String>,
    pub display_name: Option<String>,
    /// The bio, with its marks.
    pub about: Option<String>,
    pub website: Option<String>,
    pub nip05: Option<String>,
    pub lud16: Option<String>,
    /// As typed: a platform and a handle, `@handle` or a profile address.
    #[serde(default)]
    pub socials: Vec<SocialLink>,
}

/// What our kind 0 says as `picture`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picture<'a> {
    /// What the last kind 0 said: a profile edited on a device that does
    /// not hold the avatar keeps it.
    Keep,
    /// The address of the avatar just uploaded.
    Set(&'a str),
    /// No picture.
    Remove,
}

#[derive(Clone)]
pub struct ProfileService {
    store: Store,
    /// How many kind 0s of mine were read from relays (`note_own_heard`).
    own_heard: Arc<AtomicU64>,
}

impl ProfileService {
    pub fn new(store: Store) -> Self {
        Self { store, own_heard: Arc::default() }
    }

    /// A kind 0 of mine came from a relay and was applied, newer than the
    /// cache or not: the cache now knows what that relay knows of me.
    pub fn note_own_heard(&self) {
        self.own_heard.fetch_add(1, Ordering::SeqCst);
    }

    /// Grows with every `note_own_heard`; compare two readings.
    pub fn own_heard(&self) -> u64 {
        self.own_heard.load(Ordering::SeqCst)
    }

    /// Apply a kind-0 event. Returns `true` when the cache changed.
    pub async fn apply_event(&self, author: &PubKey, created_at: Timestamp, content: &str) -> Result<bool> {
        if content.len() > MAX_CONTENT_BYTES {
            return Ok(false);
        }
        repo::upsert_if_newer(&self.store, &read_content(author.as_hex(), created_at.secs(), content)).await
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

    /// Build and sign our kind 0 from `input` over the last one cached for
    /// us, and store it locally with the event's timestamp so the cache
    /// reflects what we published. The event is never older than the one
    /// it replaces, whatever this device's clock says. Errors (as
    /// `Invalid`): `bio_too_long`, `social_unknown_platform`,
    /// `social_bad_handle`, `profile_too_large`, and a website or picture
    /// that is no address.
    pub async fn build_own(&self, keys: &Keys, input: &ProfileInput, picture: Picture<'_>) -> Result<WireEvent> {
        let me = PubKey::parse(&keys.public_key().to_hex()).expect("valid pubkey");
        let last = repo::get(&self.store, me.as_hex()).await?;
        let base = last.as_ref().map(|r| r.raw_json.as_str()).unwrap_or("{}");
        let content = own_content(base, input, picture)?;
        self.sign_own(keys, &me, last, content).await
    }

    /// Our kind 0 as the last one cached for us with only `picture`
    /// changed: what setting, removing or moving the avatar publishes.
    /// Every other key stays as it is, ours or not.
    pub async fn build_picture(&self, keys: &Keys, picture: Picture<'_>) -> Result<WireEvent> {
        let me = PubKey::parse(&keys.public_key().to_hex()).expect("valid pubkey");
        let last = repo::get(&self.store, me.as_hex()).await?;
        let mut obj: Map<String, Value> = match last.as_ref().map(|r| serde_json::from_str::<Value>(&r.raw_json)) {
            Some(Ok(Value::Object(m))) => m,
            _ => Map::new(),
        };
        match picture {
            Picture::Keep => {}
            Picture::Set(url) => {
                let url = checked_url(Some(url.to_string()), "picture must be a URL")?
                    .ok_or_else(|| MessengerError::Invalid("picture must be a URL".into()))?;
                obj.insert("picture".into(), Value::String(url));
            }
            Picture::Remove => {
                obj.remove("picture");
            }
        }
        let content = serde_json::to_string(&Value::Object(obj))?;
        if content.len() > MAX_CONTENT_BYTES {
            return Err(MessengerError::Invalid("profile_too_large".into()));
        }
        self.sign_own(keys, &me, last, content).await
    }

    /// Sign `content` as our kind 0, never older than `last`, and cache it.
    async fn sign_own(&self, keys: &Keys, me: &PubKey, last: Option<repo::ProfileRow>, content: String) -> Result<WireEvent> {
        let not_before = last.map_or(0, |r| r.event_created_at.saturating_add(1));
        let created_at = nostr::types::Timestamp::now().as_secs().max(u64::try_from(not_before).unwrap_or(0));
        let event = EventBuilder::new(Kind::Metadata, content)
            .custom_created_at(nostr::types::Timestamp::from_secs(created_at))
            .finalize(keys)
            .map_err(|e| MessengerError::Crypto(e.to_string()))?;
        self.apply_event(me, Timestamp(event.created_at.as_secs() as i64), &event.content).await?;
        Ok(WireEvent { id: EventId::parse(&event.id.to_hex()).expect("hex id"), json: serde_json::to_value(&event)? })
    }

    pub async fn set_nip05_verified(&self, pubkey: &PubKey, verified_at: Option<i64>) -> Result<()> {
        repo::set_nip05_verified(&self.store, pubkey.as_hex(), verified_at).await
    }
}

/// The row a kind-0 content makes. Anything but a JSON object clears every
/// field; it is still the newer event.
fn read_content(pubkey: &str, created_at: i64, content: &str) -> repo::ProfileRow {
    let parsed = serde_json::from_str::<Value>(content).ok();
    let Some(obj) = parsed.as_ref().and_then(Value::as_object) else {
        return repo::ProfileRow {
            pubkey: pubkey.into(),
            event_created_at: created_at,
            raw_json: "{}".into(),
            ..Default::default()
        };
    };
    let text = |key: &str| clean(obj.get(key).and_then(Value::as_str).map(String::from));
    let about = text("about");
    let about_rich = obj
        .get("veydan_about")
        .and_then(Value::as_str)
        .map(richtext::normalize)
        .filter(|m| !m.is_empty() && richtext::strip(m) == richtext::normalize(about.as_deref().unwrap_or_default()));
    let socials = obj.get("veydan_socials").map(social::list_from_json).unwrap_or_default();
    repo::ProfileRow {
        pubkey: pubkey.into(),
        name: text("name"),
        display_name: text("display_name"),
        about,
        picture: clean_url(text("picture")),
        banner: clean_url(text("banner")),
        website: clean_url(text("website")),
        nip05: text("nip05").map(|s| s.to_lowercase()),
        lud16: text("lud16"),
        nip05_verified_at: None,
        event_created_at: created_at,
        fetched_at: 0,
        raw_json: content.to_string(),
        about_rich,
        socials_json: (!socials.is_empty()).then(|| serde_json::to_string(&socials).expect("links serialize")),
    }
}

/// The content of our kind 0: `base` (the last one, a JSON object or
/// anything else, then taken as empty) with the managed keys replaced. An
/// empty value removes its key.
fn own_content(base: &str, input: &ProfileInput, picture: Picture<'_>) -> Result<String> {
    let mut obj: Map<String, Value> = match serde_json::from_str::<Value>(base) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    };
    let kept_picture = obj.get("picture").cloned();
    for key in MANAGED {
        obj.remove(key);
    }
    let mut put = |key: &str, value: Option<String>| {
        if let Some(v) = value {
            obj.insert(key.into(), Value::String(v));
        }
    };
    put("name", clean(input.name.clone()));
    put("display_name", clean(input.display_name.clone()));
    if let Some(markup) = clean(input.about.clone()) {
        let markup = richtext::check(&markup).map_err(|e| MessengerError::Invalid(e.into()))?;
        let about = richtext::strip(&markup);
        if !about.is_empty() {
            put("about", Some(about));
            put("veydan_about", Some(markup));
        }
    }
    put("website", checked_url(input.website.clone(), "website must be a URL")?);
    put("nip05", clean(input.nip05.clone()).map(|s| s.to_lowercase()));
    put("lud16", clean(input.lud16.clone()));
    match picture {
        Picture::Keep => {
            if let Some(v) = kept_picture {
                obj.insert("picture".into(), v);
            }
        }
        Picture::Set(url) => put("picture", checked_url(Some(url.to_string()), "picture must be a URL")?),
        Picture::Remove => {}
    }
    let mut links = Vec::with_capacity(input.socials.len());
    for link in &input.socials {
        links.push(social::normalize_link(&link.p, &link.h).map_err(|e| MessengerError::Invalid(e.into()))?);
    }
    let links = social::clean_list(&links);
    if !links.is_empty() {
        obj.insert("veydan_socials".into(), serde_json::to_value(&links)?);
    }
    let content = serde_json::to_string(&Value::Object(obj))?;
    if content.len() > MAX_CONTENT_BYTES {
        return Err(MessengerError::Invalid("profile_too_large".into()));
    }
    Ok(content)
}

fn clean(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn clean_url(v: Option<String>) -> Option<String> {
    clean(v).filter(|s| s.starts_with("https://") || s.starts_with("http://"))
}

/// An address the user gave: none, or one that parses.
fn checked_url(v: Option<String>, err: &str) -> Result<Option<String>> {
    let Some(v) = clean(v) else { return Ok(None) };
    match clean_url(Some(v)) {
        Some(v) if Url::parse(&v).is_ok() => Ok(Some(v)),
        _ => Err(MessengerError::Invalid(err.into())),
    }
}

fn to_view(r: repo::ProfileRow) -> ProfileView {
    let npub = PublicKey::from_hex(&r.pubkey)
        .ok()
        .and_then(|p| p.to_bech32().ok())
        .unwrap_or_else(|| r.pubkey.clone());
    let bio = match (&r.about_rich, &r.about) {
        (Some(markup), _) => richtext::parse(markup),
        (None, Some(about)) => richtext::plain(about),
        (None, None) => Vec::new(),
    };
    let links = r
        .socials_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .map(|v| social::list_from_json(&v))
        .unwrap_or_default();
    let socials = links.iter().filter_map(social::view).collect();
    ProfileView {
        pubkey: r.pubkey,
        npub,
        name: r.name,
        display_name: r.display_name,
        bio_source: r.about_rich.or_else(|| r.about.as_deref().map(escape_plain)),
        about: r.about,
        picture: r.picture,
        banner: r.banner,
        website: r.website,
        nip05: r.nip05,
        lud16: r.lud16,
        nip05_verified: r.nip05_verified_at.is_some(),
        event_created_at: r.event_created_at,
        fetched_at: r.fetched_at,
        bio,
        socials,
        links,
    }
}

/// A plain `about` as markup that shows the same and strips back to it:
/// the marks escaped, except inside addresses, where no mark is read and
/// a `\` would change the address.
pub(crate) fn escape_plain(s: &str) -> String {
    const SCHEME: &str = "https://";
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + 8);
    let mut i = 0;
    while i < chars.len() {
        let at_word = i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');
        if at_word && chars[i..].iter().take(SCHEME.len()).copied().eq(SCHEME.chars()) {
            while i < chars.len() && !is_link_stop(chars[i]) {
                out.push(chars[i]);
                i += 1;
            }
            continue;
        }
        if matches!(chars[i], '\\' | '*' | '~' | '`' | '{' | '}') {
            out.push('\\');
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Ends an address (messenger-richtext's `link::is_stop`).
fn is_link_stop(c: char) -> bool {
    c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '`' | '{' | '}' | '\\')
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_richtext::{Color, Style};

    fn pk(keys: &Keys) -> PubKey {
        PubKey::parse(&keys.public_key().to_hex()).unwrap()
    }

    fn content_of(ev: &WireEvent) -> Value {
        let parsed: Event = serde_json::from_value(ev.json.clone()).unwrap();
        serde_json::from_str(&parsed.content).unwrap()
    }

    fn link(p: &str, h: &str) -> SocialLink {
        SocialLink { p: p.into(), h: h.into() }
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
    async fn hostile_content_is_read_safely() {
        let store = Store::open_in_memory().await.unwrap();
        let svc = ProfileService::new(store);
        let k = Keys::generate();
        // Wrong types are as good as absent.
        let weird = r#"{"name":42,"about":["x"],"picture":{"a":1},"veydan_about":7,"veydan_socials":"github:x","website":null}"#;
        assert!(svc.apply_event(&pk(&k), Timestamp(1), weird).await.unwrap());
        let v = svc.get(&pk(&k)).await.unwrap().unwrap();
        assert_eq!((v.name, v.about, v.picture, v.website), (None, None, None, None));
        assert!(v.bio.is_empty() && v.socials.is_empty() && v.bio_source.is_none());
        // A JSON array, a string, deep nesting: no panic, all cleared.
        for c in ["[1,2]", "\"x\"", &"[".repeat(10_000)] {
            svc.apply_event(&pk(&k), Timestamp(2), c).await.unwrap();
        }
        // Too large to read: ignored.
        let huge = format!(r#"{{"name":"big","about":"{}"}}"#, "a".repeat(MAX_CONTENT_BYTES));
        assert!(!svc.apply_event(&pk(&k), Timestamp(100), &huge).await.unwrap());
        assert_eq!(svc.get(&pk(&k)).await.unwrap().unwrap().event_created_at, 2);
    }

    #[tokio::test]
    async fn build_own_signs_and_caches() {
        let store = Store::open_in_memory().await.unwrap();
        let svc = ProfileService::new(store);
        let k = Keys::generate();
        let input = ProfileInput { name: Some("me".into()), ..Default::default() };
        let ev = svc.build_own(&k, &input, Picture::Set("https://x.example/a.png")).await.unwrap();
        let parsed: Event = serde_json::from_value(ev.json.clone()).unwrap();
        assert_eq!(parsed.kind.as_u16(), 0);
        assert!(parsed.verify().is_ok());
        let cached = svc.get(&pk(&k)).await.unwrap().unwrap();
        assert_eq!(cached.name.as_deref(), Some("me"));
        assert_eq!(cached.picture.as_deref(), Some("https://x.example/a.png"));
        assert!(svc.build_own(&k, &input, Picture::Set("https://bad url")).await.is_err());
        assert!(svc.build_own(&k, &input, Picture::Set("javascript:alert(1)")).await.is_err());
        let bad_site = ProfileInput { website: Some("ftp://x.example".into()), ..input.clone() };
        assert!(svc.build_own(&k, &bad_site, Picture::Keep).await.is_err());
        assert!(svc.search("me", 5).await.unwrap().len() == 1);

        // Keep: the picture stays; Remove: it goes.
        svc.build_own(&k, &ProfileInput { name: Some("me2".into()), ..Default::default() }, Picture::Keep).await.unwrap();
        let cached = svc.get(&pk(&k)).await.unwrap().unwrap();
        assert_eq!((cached.name.as_deref(), cached.picture.as_deref()), (Some("me2"), Some("https://x.example/a.png")));
        let ev = svc.build_own(&k, &input, Picture::Remove).await.unwrap();
        assert!(content_of(&ev).get("picture").is_none());
        assert_eq!(svc.get(&pk(&k)).await.unwrap().unwrap().picture, None);
    }

    #[tokio::test]
    async fn build_own_merges_over_the_last_kind_0() {
        let store = Store::open_in_memory().await.unwrap();
        let svc = ProfileService::new(store);
        let k = Keys::generate();
        // Written by another client, a minute "in the future".
        let other = r#"{"name":"old","displayName":"Old","banner":"https://b.example/b.png","picture":"https://p.example/p.png","bot":false,"x_client":{"theme":"dark"},"lud16":"me@ln.example"}"#;
        let future = nostr::types::Timestamp::now().as_secs() as i64 + 60;
        svc.apply_event(&pk(&k), Timestamp(future), other).await.unwrap();

        let input = ProfileInput { name: Some("new".into()), display_name: Some("New".into()), ..Default::default() };
        let ev = svc.build_own(&k, &input, Picture::Keep).await.unwrap();
        let c = content_of(&ev);
        assert_eq!(c["name"], "new");
        assert_eq!(c["display_name"], "New");
        assert!(c.get("displayName").is_none(), "the old spelling goes");
        assert_eq!(c["banner"], "https://b.example/b.png");
        assert_eq!(c["picture"], "https://p.example/p.png");
        assert_eq!(c["bot"], false);
        assert_eq!(c["x_client"]["theme"], "dark");
        assert!(c.get("lud16").is_none(), "a managed key left empty is removed");
        for key in ["about", "veydan_about", "veydan_socials", "website", "nip05"] {
            assert!(c.get(key).is_none(), "{key}");
        }
        let parsed: Event = serde_json::from_value(ev.json).unwrap();
        assert!(parsed.created_at.as_secs() as i64 > future, "never older than the one it replaces");
        let v = svc.get(&pk(&k)).await.unwrap().unwrap();
        assert_eq!(v.banner.as_deref(), Some("https://b.example/b.png"));
        assert_eq!(v.name.as_deref(), Some("new"));
    }

    #[tokio::test]
    async fn build_picture_changes_the_picture_alone() {
        let store = Store::open_in_memory().await.unwrap();
        let svc = ProfileService::new(store);
        let k = Keys::generate();
        // Nothing cached yet: a kind 0 with the picture only.
        let ev = svc.build_picture(&k, Picture::Set("https://m.example/aa")).await.unwrap();
        assert_eq!(content_of(&ev), serde_json::json!({ "picture": "https://m.example/aa" }));

        let other = r#"{"name":"me","about":"plain *text*","veydan_about":"x","displayName":"Old","banner":"https://b.example/b.png","x":{"y":1}}"#;
        let future = nostr::types::Timestamp::now().as_secs() as i64 + 60;
        svc.apply_event(&pk(&k), Timestamp(future), other).await.unwrap();
        let ev = svc.build_picture(&k, Picture::Set("https://m.example/bb")).await.unwrap();
        let mut want: Value = serde_json::from_str(other).unwrap();
        want["picture"] = "https://m.example/bb".into();
        assert_eq!(content_of(&ev), want, "every other key as it was, even those build_own manages");
        let parsed: Event = serde_json::from_value(ev.json).unwrap();
        assert!(parsed.verify().is_ok());
        assert!(parsed.created_at.as_secs() as i64 > future, "never older than the one it replaces");
        assert_eq!(svc.get(&pk(&k)).await.unwrap().unwrap().picture.as_deref(), Some("https://m.example/bb"));

        let ev = svc.build_picture(&k, Picture::Remove).await.unwrap();
        assert!(content_of(&ev).get("picture").is_none());
        assert_eq!(content_of(&ev)["banner"], "https://b.example/b.png");
        assert_eq!(svc.get(&pk(&k)).await.unwrap().unwrap().picture, None);
        let ev = svc.build_picture(&k, Picture::Keep).await.unwrap();
        assert!(content_of(&ev).get("picture").is_none());
        for bad in ["", "ftp://x.example/a", "javascript:alert(1)"] {
            assert!(svc.build_picture(&k, Picture::Set(bad)).await.is_err(), "{bad:?}");
        }
    }

    #[tokio::test]
    async fn bio_and_socials_round_trip() {
        let store = Store::open_in_memory().await.unwrap();
        let svc = ProfileService::new(store);
        let k = Keys::generate();
        let input = ProfileInput {
            about: Some("  **Hi**, I am {red}red{/}\r\nhttps://me.example  ".into()),
            socials: vec![
                link("github", "https://github.com/octocat"),
                link("telegram", "@durov_x"),
                link("github", "OctoCat"),
            ],
            ..Default::default()
        };
        let ev = svc.build_own(&k, &input, Picture::Keep).await.unwrap();
        let c = content_of(&ev);
        assert_eq!(c["about"], "Hi, I am red\nhttps://me.example");
        assert_eq!(c["veydan_about"], "**Hi**, I am {red}red{/}\nhttps://me.example");
        assert_eq!(c["veydan_socials"], serde_json::json!([{"p":"github","h":"octocat"},{"p":"telegram","h":"durov_x"}]));

        let v = svc.get(&pk(&k)).await.unwrap().unwrap();
        assert_eq!(v.about.as_deref(), Some("Hi, I am red\nhttps://me.example"));
        assert_eq!(v.bio_source.as_deref(), Some("**Hi**, I am {red}red{/}\nhttps://me.example"));
        assert_eq!(v.bio[0], Span::Text { text: "Hi".into(), style: Style { bold: true, ..Default::default() } });
        assert!(v.bio.contains(&Span::Text { text: "red".into(), style: Style { color: Some(Color::Red), ..Default::default() } }));
        assert!(v.bio.iter().any(|s| matches!(s, Span::Link { url, .. } if url == "https://me.example")));
        assert_eq!(v.socials.len(), 2);
        assert_eq!(v.socials[0].url, "https://github.com/octocat");
        assert_eq!(v.socials[1].platform, "telegram");

        // What the editor gets back saves to the same kind 0.
        let again = ProfileInput {
            about: v.bio_source.clone(),
            socials: c["veydan_socials"].as_array().unwrap().iter().map(|l| serde_json::from_value(l.clone()).unwrap()).collect(),
            ..Default::default()
        };
        let c2 = content_of(&svc.build_own(&k, &again, Picture::Keep).await.unwrap());
        assert_eq!((&c2["about"], &c2["veydan_about"], &c2["veydan_socials"]), (&c["about"], &c["veydan_about"], &c["veydan_socials"]));

        // Clearing the bio removes both keys.
        let c3 = content_of(&svc.build_own(&k, &ProfileInput::default(), Picture::Keep).await.unwrap());
        assert!(c3.get("about").is_none() && c3.get("veydan_about").is_none() && c3.get("veydan_socials").is_none());
        let v = svc.get(&pk(&k)).await.unwrap().unwrap();
        assert!(v.bio.is_empty() && v.socials.is_empty() && v.bio_source.is_none());
    }

    #[tokio::test]
    async fn build_own_refuses_what_it_cannot_save() {
        let store = Store::open_in_memory().await.unwrap();
        let svc = ProfileService::new(store);
        let k = Keys::generate();
        let err = |e: MessengerError| match e {
            MessengerError::Invalid(code) => code,
            other => panic!("{other:?}"),
        };
        let long = ProfileInput { about: Some("x".repeat(richtext::MAX_CHARS + 1)), ..Default::default() };
        assert_eq!(err(svc.build_own(&k, &long, Picture::Keep).await.unwrap_err()), "bio_too_long");
        let bad = ProfileInput { socials: vec![link("github", "-bad-")], ..Default::default() };
        assert_eq!(err(svc.build_own(&k, &bad, Picture::Keep).await.unwrap_err()), "social_bad_handle");
        let unknown = ProfileInput { socials: vec![link("myspace", "tom")], ..Default::default() };
        assert_eq!(err(svc.build_own(&k, &unknown, Picture::Keep).await.unwrap_err()), "social_unknown_platform");
        assert!(svc.get(&pk(&k)).await.unwrap().is_none(), "nothing was cached");
    }

    #[tokio::test]
    async fn an_inconsistent_bio_falls_back_to_the_plain_about() {
        let store = Store::open_in_memory().await.unwrap();
        let svc = ProfileService::new(store);
        let k = Keys::generate();
        // Another client edited `about` and kept our key.
        let c = r#"{"about":"edited **elsewhere**","veydan_about":"**old** bio"}"#;
        svc.apply_event(&pk(&k), Timestamp(1), c).await.unwrap();
        let v = svc.get(&pk(&k)).await.unwrap().unwrap();
        assert_eq!(v.bio_source.as_deref(), Some("edited \\*\\*elsewhere\\*\\*"), "escaped: it shows as it reads");
        assert_eq!(v.bio, vec![Span::Text { text: "edited **elsewhere**".into(), style: Style::default() }], "plain: marks are text");
        assert_eq!(richtext::parse(v.bio_source.as_deref().unwrap()), v.bio);

        // Consistent up to the one form (spaces, line ends): the marks count.
        let c = "{\"about\":\"a  \\r\\nb \",\"veydan_about\":\"*a*\\nb\"}";
        svc.apply_event(&pk(&k), Timestamp(2), c).await.unwrap();
        let v = svc.get(&pk(&k)).await.unwrap().unwrap();
        assert_eq!(v.bio_source.as_deref(), Some("*a*\nb"));
        assert_eq!(v.bio[0], Span::Text { text: "a".into(), style: Style { italic: true, ..Default::default() } });

        // Our key without an `about`: the marks are not trusted.
        svc.apply_event(&pk(&k), Timestamp(3), r#"{"veydan_about":"**x**"}"#).await.unwrap();
        let v = svc.get(&pk(&k)).await.unwrap().unwrap();
        assert!(v.bio.is_empty() && v.bio_source.is_none());
    }

    /// A bio another client wrote, with what reads as marks to us, saved
    /// back from the editor unchanged: other clients see the same `about`.
    #[tokio::test]
    async fn a_plain_about_saved_unchanged_stays_the_same() {
        let store = Store::open_in_memory().await.unwrap();
        let svc = ProfileService::new(store);
        let k = Keys::generate();
        let about = "I use *nix and love C:\\{red}x and ~~old~~ `code`, see https://a.example/*x*_{y} **bold**";
        svc.apply_event(&pk(&k), Timestamp(1), &serde_json::json!({ "name": "old", "about": about }).to_string()).await.unwrap();
        let v = svc.get(&pk(&k)).await.unwrap().unwrap();
        assert_eq!(richtext::parse(v.bio_source.as_deref().unwrap()), v.bio, "the editor shows what the profile shows");

        let input = ProfileInput { name: Some("new".into()), about: v.bio_source.clone(), ..Default::default() };
        let c = content_of(&svc.build_own(&k, &input, Picture::Keep).await.unwrap());
        assert_eq!(c["about"], about, "other clients see the same text");
        assert_eq!(c["name"], "new");
        let again = svc.get(&pk(&k)).await.unwrap().unwrap();
        assert_eq!(again.bio, v.bio);
        assert_eq!(again.bio_source, v.bio_source, "and it edits the same next time");
    }

    #[test]
    fn escape_plain_quotes_marks_but_not_addresses() {
        assert_eq!(escape_plain("a*b* `c` {red}d{/} \\"), "a\\*b\\* \\`c\\` \\{red\\}d\\{/\\} \\\\");
        assert_eq!(escape_plain("see https://a.example/*x* *y*"), "see https://a.example/*x* \\*y\\*");
        assert_eq!(escape_plain("xhttps://a.example/*"), "xhttps://a.example/\\*");
    }

    #[tokio::test]
    async fn invalid_received_socials_are_dropped() {
        let store = Store::open_in_memory().await.unwrap();
        let svc = ProfileService::new(store);
        let k = Keys::generate();
        let c = serde_json::json!({
            "name": "x",
            "veydan_socials": [
                {"p": "github", "h": "octocat"},
                {"p": "github", "h": "evil.com/../x"},
                {"p": "nowhere", "h": "x"},
                {"p": "other", "h": "javascript:alert(1)"},
                {"p": "telegram"},
                "github:x",
                {"p": "github", "h": "OctoCat"},
                {"p": "other", "h": "https://my.site.example/about"}
            ]
        })
        .to_string();
        svc.apply_event(&pk(&k), Timestamp(1), &c).await.unwrap();
        let v = svc.get(&pk(&k)).await.unwrap().unwrap();
        let got: Vec<(&str, &str)> = v.socials.iter().map(|s| (s.platform.as_str(), s.url.as_str())).collect();
        assert_eq!(got, vec![("github", "https://github.com/octocat"), ("other", "https://my.site.example/about")]);

        // Too many: at most MAX_SOCIALS are kept.
        let many: Vec<Value> = (0..40).map(|i| serde_json::json!({"p": "github", "h": format!("user{i}")})).collect();
        let c = serde_json::json!({ "veydan_socials": many }).to_string();
        svc.apply_event(&pk(&k), Timestamp(2), &c).await.unwrap();
        assert_eq!(svc.get(&pk(&k)).await.unwrap().unwrap().socials.len(), social::MAX_SOCIALS);
    }

    #[test]
    fn the_ui_types_leave_out_what_is_not_for_the_ui() {
        let cfg = ts_rs::Config::new();
        let view = ProfileView::decl(&cfg);
        assert!(view.contains("bio: Array<Span>") && view.contains("socials: Array<SocialView>"), "{view}");
        assert!(view.contains("event_created_at: number"), "{view}");
        assert!(!view.contains("links"), "{view}");
        let input = ProfileInput::decl(&cfg);
        assert!(input.contains("socials: Array<SocialLink>") && !input.contains("picture"), "{input}");
    }

    #[test]
    fn short_npub_never_panics() {
        assert_eq!(short_npub("npub1"), "npub1");
        assert_eq!(short_npub(&"é".repeat(20)), format!("{}…{}", "é".repeat(12), "é".repeat(4)));
    }
}
