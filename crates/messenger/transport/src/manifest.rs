// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Infrastructure manifest, version 1.
//!
//! The manifest tells a fresh install which relays and media servers to use,
//! per region, and where to look for newer manifests. It is our own format,
//! modelled on the region/stable-id design of the previous project but
//! simplified to flat lists tagged with regions.
//!
//! Two shapes are accepted:
//!
//! - **content**: the bare JSON document (`parse_content`). Only the
//!   embedded, compiled-in manifest is trusted in this shape.
//! - **signed**: a Nostr event of kind 30078 with `d = MANIFEST_D` whose
//!   `content` is the document (`parse_signed`). Signature and, when the
//!   caller pins one, the author key are verified. Every remote source must
//!   use this shape. Being a plain Nostr event, the same document can be
//!   served over HTTP or published to a relay without any extra format.
//!
//! Anti-rollback is by `serial`: a manifest is applied only when its serial
//! is greater than the last applied one.

use messenger_core::{MessengerError, RelayUrl, Result};
use nostr::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};

pub const FORMAT: &str = "veydan-messenger-manifest";
pub const VERSION: u32 = 1;
pub const MANIFEST_KIND: u16 = 30078;
pub const MANIFEST_D: &str = "net.veydan.messenger.manifest";
/// Region tag meaning "every region".
pub const REGION_ANY: &str = "*";
pub const REGION_DEFAULT: &str = "default";

/// The manifest compiled into the binary (trust tier: embedded).
pub const EMBEDDED_MANIFEST_JSON: &str = include_str!("../manifest/embedded.json");

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub v: u32,
    pub serial: u64,
    pub issued_at: i64,
    #[serde(default)]
    pub regions: Vec<String>,
    #[serde(default)]
    pub relays: Vec<ManifestRelay>,
    #[serde(default)]
    pub media: Vec<ManifestMedia>,
    #[serde(default)]
    pub sources: Vec<ManifestSource>,
    /// Push servers. Absent in manifests written before pushes existed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub push: Vec<ManifestPush>,
}

/// A server that watches relays for the user and wakes the phone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestPush {
    pub id: String,
    /// `https://…`, without a trailing slash.
    pub url: String,
    #[serde(default)]
    pub regions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestRelay {
    /// Stable logical id. Changing `url` under the same id means "same
    /// relay, new address" and migrates stored references.
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub regions: Vec<String>,
    #[serde(default = "yes")]
    pub read: bool,
    #[serde(default = "yes")]
    pub write: bool,
    #[serde(default)]
    pub auth: Option<ManifestAuth>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ManifestAuth {
    /// NIP-42 challenge/response with the user's key.
    Nip42,
    /// Static key gate in front of the relay: sent as `?key=` on the
    /// WebSocket URL. It is shipped with the manifest, so it protects the
    /// relay from the open internet, not from app users.
    ApiKey { key: String },
}

impl ManifestAuth {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Nip42 => "nip42",
            Self::ApiKey { .. } => "api_key",
        }
    }

    pub fn secret(&self) -> Option<&str> {
        match self {
            Self::ApiKey { key } => Some(key),
            Self::Nip42 => None,
        }
    }
}

fn default_media_kind() -> String {
    "blossom".into()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestMedia {
    pub id: String,
    pub url: String,
    /// `blossom` (default) or `s3`.
    #[serde(default = "default_media_kind", rename = "type")]
    pub kind: String,
    /// S3 only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket: Option<String>,
    /// S3 only; the signing region, not a manifest region.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3_region: Option<String>,
    #[serde(default)]
    pub regions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ManifestSource {
    Http {
        url: String,
        #[serde(default)]
        regions: Vec<String>,
        #[serde(default)]
        priority: i32,
    },
    Nostr {
        pubkey: String,
        #[serde(default)]
        relays: Vec<String>,
        #[serde(default)]
        regions: Vec<String>,
        #[serde(default)]
        priority: i32,
    },
}

fn yes() -> bool {
    true
}

impl Manifest {
    /// Parse a bare document. Use only for the embedded manifest.
    pub fn parse_content(json: &str) -> Result<Self> {
        let m: Manifest = serde_json::from_str(json)
            .map_err(|e| MessengerError::Invalid(format!("manifest json: {e}")))?;
        m.validate()?;
        Ok(m)
    }

    /// Parse a signed manifest event. `expected_pubkey` (hex) pins the
    /// author; pass `None` only in tests.
    pub fn parse_signed(event_json: &str, expected_pubkey: Option<&str>) -> Result<Self> {
        let event: Event = serde_json::from_str(event_json)
            .map_err(|e| MessengerError::Invalid(format!("manifest event json: {e}")))?;
        event
            .verify()
            .map_err(|_| MessengerError::Invalid("manifest signature is invalid".into()))?;
        if event.kind.as_u16() != MANIFEST_KIND {
            return Err(MessengerError::Invalid(format!(
                "manifest event kind {} != {MANIFEST_KIND}",
                event.kind.as_u16()
            )));
        }
        if event.tags.identifier().as_deref() != Some(MANIFEST_D) {
            return Err(MessengerError::Invalid("manifest event has wrong d tag".into()));
        }
        if let Some(expected) = expected_pubkey {
            if event.pubkey.to_hex() != expected.to_ascii_lowercase() {
                return Err(MessengerError::Invalid("manifest signed by an unexpected key".into()));
            }
        }
        Self::parse_content(&event.content)
    }

    /// Produce the signed event JSON for this manifest. Used by tests today
    /// and by the operator tooling later.
    pub fn sign(&self, keys: &Keys) -> Result<String> {
        self.validate()?;
        let content = serde_json::to_string(self)?;
        let event = EventBuilder::new(Kind::from(MANIFEST_KIND), content)
            .tag(Tag::identifier(MANIFEST_D))
            .custom_created_at(Timestamp::from_secs(self.issued_at.max(0) as u64))
            .finalize(keys)
            .map_err(|e| MessengerError::Crypto(e.to_string()))?;
        Ok(serde_json::to_string(&event)?)
    }

    pub fn validate(&self) -> Result<()> {
        if self.format != FORMAT {
            return Err(MessengerError::Invalid(format!("manifest format '{}' unsupported", self.format)));
        }
        if self.v != VERSION {
            return Err(MessengerError::Invalid(format!("manifest v{} unsupported", self.v)));
        }
        let mut ids = BTreeSet::new();
        for r in &self.relays {
            if r.id.trim().is_empty() {
                return Err(MessengerError::Invalid(format!("relay '{}' has an empty id", r.url)));
            }
            if !ids.insert(r.id.as_str()) {
                return Err(MessengerError::Invalid(format!("duplicate relay id '{}'", r.id)));
            }
            if RelayUrl::parse(&r.url).is_none() {
                return Err(MessengerError::Invalid(format!("relay '{}' has an invalid url '{}'", r.id, r.url)));
            }
            if let Some(ManifestAuth::ApiKey { key }) = &r.auth {
                if key.trim().is_empty() || key.chars().any(|c| c.is_whitespace() || c == '&' || c == '#') {
                    return Err(MessengerError::Invalid(format!("relay '{}' has an invalid api key", r.id)));
                }
            }
        }
        let mut media_ids = BTreeSet::new();
        for m in &self.media {
            if m.id.trim().is_empty() || !media_ids.insert(m.id.as_str()) {
                return Err(MessengerError::Invalid(format!("bad media server id '{}'", m.id)));
            }
            if !matches!(m.kind.as_str(), "blossom" | "s3") {
                return Err(MessengerError::Invalid(format!("media server '{}' has an unknown type", m.id)));
            }
            if m.kind == "s3" && m.bucket.as_deref().unwrap_or("").is_empty() {
                return Err(MessengerError::Invalid(format!("media server '{}' needs a bucket", m.id)));
            }
            if !(m.url.starts_with("https://") || m.url.starts_with("http://")) {
                return Err(MessengerError::Invalid(format!("media server '{}' url must be http(s)", m.id)));
            }
        }
        Ok(())
    }

    /// `true` when this manifest may replace one with `last_serial`.
    pub fn passes_anti_rollback(&self, last_serial: Option<u64>) -> bool {
        match last_serial {
            None => true,
            Some(last) => self.serial > last,
        }
    }

    /// Relays for `region`: entries tagged with the region or `*`. When the
    /// region has no dedicated relays, the `default` ones are used instead
    /// (plus `*`). Order is the manifest order.
    pub fn relays_for_region(&self, region: &str) -> Vec<&ManifestRelay> {
        let pick = |tag: &str| -> Vec<&ManifestRelay> {
            self.relays
                .iter()
                .filter(|r| r.regions.iter().any(|g| g == tag || g == REGION_ANY))
                .collect()
        };
        let dedicated = self.relays.iter().any(|r| r.regions.iter().any(|g| g == region));
        if dedicated || region == REGION_DEFAULT {
            pick(region)
        } else {
            pick(REGION_DEFAULT)
        }
    }

    /// Push servers for `region`, by the rule of the media servers: the
    /// region's own when it has any, the `default` ones otherwise.
    pub fn push_for_region(&self, region: &str) -> Vec<&ManifestPush> {
        let has = self.push.iter().any(|p| p.regions.iter().any(|g| g == region));
        let tag = if has || region == REGION_DEFAULT { region } else { REGION_DEFAULT };
        self.push
            .iter()
            .filter(|p| p.regions.iter().any(|g| g == tag || g == REGION_ANY))
            .collect()
    }

    pub fn media_for_region(&self, region: &str) -> Vec<&ManifestMedia> {
        let has = self.media.iter().any(|m| m.regions.iter().any(|g| g == region));
        let tag = if has || region == REGION_DEFAULT { region } else { REGION_DEFAULT };
        self.media
            .iter()
            .filter(|m| m.regions.iter().any(|g| g == tag || g == REGION_ANY))
            .collect()
    }
}

/// A manifest-managed relay as currently stored, for diffing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredRelay {
    pub id: String,
    pub url: RelayUrl,
}

/// What applying a manifest changes. `renamed` carries `(id, old, new)` so
/// stored references (scoped relays, routes) can be rewritten by id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RelayChanges {
    pub added: Vec<ManifestRelay>,
    pub removed: Vec<RelayUrl>,
    pub renamed: Vec<(String, RelayUrl, RelayUrl)>,
    /// Entries whose url is unchanged (flags may still differ).
    pub kept: Vec<ManifestRelay>,
}

impl RelayChanges {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.renamed.is_empty()
    }
}

/// Pure diff between what is stored and what the manifest says for a region.
pub fn diff_relays(stored: &[StoredRelay], target: &[&ManifestRelay]) -> RelayChanges {
    let by_id: HashMap<&str, &StoredRelay> = stored.iter().map(|s| (s.id.as_str(), s)).collect();
    let mut out = RelayChanges::default();
    let mut seen_ids = BTreeSet::new();
    for t in target {
        seen_ids.insert(t.id.as_str());
        let Some(url) = RelayUrl::parse(&t.url) else { continue };
        match by_id.get(t.id.as_str()) {
            Some(s) if s.url == url => out.kept.push((*t).clone()),
            Some(s) => out.renamed.push((t.id.clone(), s.url.clone(), url)),
            None => out.added.push((*t).clone()),
        }
    }
    for s in stored {
        if !seen_ids.contains(s.id.as_str()) {
            out.removed.push(s.url.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        Manifest {
            format: FORMAT.into(),
            v: VERSION,
            serial: 5,
            issued_at: 1_759_000_000,
            regions: vec!["default".into(), "ru".into()],
            relays: vec![
                ManifestRelay { id: "eu-1".into(), url: "wss://eu1.example".into(), regions: vec!["default".into()], read: true, write: true, auth: None },
                ManifestRelay { id: "ru-1".into(), url: "wss://ru1.example".into(), regions: vec!["ru".into()], read: true, write: true, auth: Some(ManifestAuth::Nip42) },
                ManifestRelay { id: "any".into(), url: "wss://any.example".into(), regions: vec!["*".into()], read: true, write: false, auth: None },
            ],
            media: vec![ManifestMedia { id: "m1".into(), url: "https://media.example".into(), regions: vec!["default".into()], kind: "blossom".into(), bucket: None, s3_region: None }],
            sources: vec![ManifestSource::Http { url: "https://cfg.example/m.json".into(), regions: vec![], priority: 10 }],
            push: vec![ManifestPush { id: "p1".into(), url: "https://push.example".into(), regions: vec!["default".into()] }],
        }
    }

    #[test]
    fn embedded_manifest_parses() {
        // What the built-in manifest must be, whatever servers it names: a
        // change of servers is a change of embedded.json alone, not of this
        // test.
        let m = Manifest::parse_content(EMBEDDED_MANIFEST_JSON).unwrap();
        assert!(m.serial > 0);
        assert!(
            m.relays.iter().map(|r| r.url.as_str()).chain(m.media.iter().map(|x| x.url.as_str())).chain(m.push.iter().map(|p| p.url.as_str()))
                .all(|u| u.split("://").nth(1).and_then(|h| h.split(['/', ':']).next()).is_some_and(|h| h.ends_with(".veydan.net"))),
            "the embedded manifest names only the project's own servers"
        );
        assert!(m.relays.iter().any(|r| matches!(r.auth, Some(ManifestAuth::ApiKey { .. }))), "project relay is gated");
        // Every region it names, the default one and an unknown one get a
        // relay, a media server and a push server.
        let regions = m.regions.iter().map(String::as_str).chain([REGION_DEFAULT, "nowhere"]);
        for region in regions {
            assert!(!m.relays_for_region(region).is_empty(), "no relay for {region}");
            assert!(!m.media_for_region(region).is_empty(), "no media server for {region}");
            assert!(!m.push_for_region(region).is_empty(), "no push server for {region}");
        }
    }

    #[test]
    fn region_resolution() {
        let m = sample();
        let d: Vec<_> = m.relays_for_region("default").iter().map(|r| r.id.as_str()).collect();
        assert_eq!(d, vec!["eu-1", "any"]);
        let ru: Vec<_> = m.relays_for_region("ru").iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ru, vec!["ru-1", "any"]);
        let unknown: Vec<_> = m.relays_for_region("mars").iter().map(|r| r.id.as_str()).collect();
        assert_eq!(unknown, vec!["eu-1", "any"], "unknown region falls back to default");
        assert_eq!(m.media_for_region("ru").len(), 1);
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let m = sample();
        let keys = Keys::generate();
        let signed = m.sign(&keys).unwrap();
        let back = Manifest::parse_signed(&signed, Some(&keys.public_key().to_hex())).unwrap();
        assert_eq!(back, m);

        let other = Keys::generate();
        assert!(Manifest::parse_signed(&signed, Some(&other.public_key().to_hex())).is_err(), "pinned key mismatch");

        // `content` is a JSON string inside the event, so its quotes are escaped.
        let tampered = signed.replace("\\\"serial\\\":5", "\\\"serial\\\":9");
        assert_ne!(tampered, signed, "the tamper pattern must match the serialized event");
        assert!(Manifest::parse_signed(&tampered, None).is_err(), "content change breaks signature");
    }

    #[test]
    fn rejects_wrong_kind_and_d_tag() {
        let m = sample();
        let keys = Keys::generate();
        let content = serde_json::to_string(&m).unwrap();
        let wrong_kind = EventBuilder::new(Kind::from(1u16), content.clone())
            .tag(Tag::identifier(MANIFEST_D))
            .finalize(&keys)
            .unwrap();
        assert!(Manifest::parse_signed(&serde_json::to_string(&wrong_kind).unwrap(), None).is_err());
        let wrong_d = EventBuilder::new(Kind::from(MANIFEST_KIND), content)
            .tag(Tag::identifier("other"))
            .finalize(&keys)
            .unwrap();
        assert!(Manifest::parse_signed(&serde_json::to_string(&wrong_d).unwrap(), None).is_err());
    }

    #[test]
    fn validation_catches_bad_ids_and_urls() {
        let mut m = sample();
        m.relays[1].id = "eu-1".into();
        assert!(m.validate().is_err(), "duplicate id");
        let mut m = sample();
        m.relays[0].url = "https://not-a-relay".into();
        assert!(m.validate().is_err(), "bad relay url");
        let mut m = sample();
        m.format = "other".into();
        assert!(m.validate().is_err());
        let mut m = sample();
        m.v = 2;
        assert!(m.validate().is_err());
    }

    #[test]
    fn anti_rollback() {
        let m = sample();
        assert!(m.passes_anti_rollback(None));
        assert!(m.passes_anti_rollback(Some(4)));
        assert!(!m.passes_anti_rollback(Some(5)));
        assert!(!m.passes_anti_rollback(Some(6)));
    }

    #[test]
    fn diff_detects_add_remove_rename_keep() {
        let stored = vec![
            StoredRelay { id: "eu-1".into(), url: RelayUrl::parse("wss://old-eu1.example").unwrap() },
            StoredRelay { id: "gone".into(), url: RelayUrl::parse("wss://gone.example").unwrap() },
            StoredRelay { id: "any".into(), url: RelayUrl::parse("wss://any.example").unwrap() },
        ];
        let m = sample();
        let target = m.relays_for_region("default");
        let d = diff_relays(&stored, &target);
        assert_eq!(d.added.len(), 0);
        assert_eq!(d.kept.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["any"]);
        assert_eq!(d.renamed.len(), 1);
        assert_eq!(d.renamed[0].0, "eu-1");
        assert_eq!(d.renamed[0].1.as_str(), "wss://old-eu1.example");
        assert_eq!(d.renamed[0].2.as_str(), "wss://eu1.example");
        assert_eq!(d.removed.len(), 1);
        assert_eq!(d.removed[0].as_str(), "wss://gone.example");
        assert!(!d.is_empty());

        let again = diff_relays(
            &target.iter().map(|r| StoredRelay { id: r.id.clone(), url: RelayUrl::parse(&r.url).unwrap() }).collect::<Vec<_>>(),
            &target,
        );
        assert!(again.is_empty());
    }
}
