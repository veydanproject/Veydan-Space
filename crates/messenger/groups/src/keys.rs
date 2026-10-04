// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Group keys and links.
//!
//! A private group has random keys, handed to each member personally. A
//! public group has a link secret; its key is derived from the secret, so
//! whoever holds the link holds the key and nobody has to be online to
//! let them in.

use crate::op::{GroupKind, KeyId};
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::{STANDARD as B64, URL_SAFE_NO_PAD as B64URL};
use base64::Engine as _;
use messenger_core::{MessengerError, PubKey, RelayUrl, Result};
use messenger_links::{LinkError, LinkType, Uri, UriBuilder};
use sha2::{Digest, Sha256};

const NONCE_LEN: usize = 12;

fn crypto(e: impl std::fmt::Display) -> MessengerError {
    MessengerError::Crypto(e.to_string())
}

fn random<const N: usize>() -> Result<[u8; N]> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(crypto)?;
    Ok(b)
}

#[derive(Clone, PartialEq, Eq)]
pub struct GroupKey([u8; 32]);

impl std::fmt::Debug for GroupKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GroupKey({:?})", self.id())
    }
}

impl GroupKey {
    pub fn generate() -> Result<Self> {
        Ok(Self(random()?))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Ok(Self(bytes.try_into().map_err(|_| MessengerError::Crypto("a group key is 32 bytes".into()))?))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// What someone who joins by a link shows: a value only a holder of
    /// the key of that link can compute, bound to who they are.
    pub fn join_mac(&self, group_id: &str, author: &messenger_core::PubKey, link_epoch: u32) -> String {
        let mut h = Sha256::new();
        h.update(b"veydan.group.join.v1\0");
        h.update(self.0);
        h.update(group_id.as_bytes());
        h.update(author.as_hex().as_bytes());
        h.update(link_epoch.to_le_bytes());
        hex::encode(h.finalize())
    }

    pub fn id(&self) -> KeyId {
        KeyId::of(&self.0)
    }

    /// AES-256-GCM with a random nonce: base64 of `nonce ‖ ciphertext ‖ tag`.
    pub fn seal(&self, plaintext: &[u8]) -> Result<String> {
        let nonce: [u8; NONCE_LEN] = random()?;
        let cipher = Aes256Gcm::new_from_slice(&self.0).map_err(crypto)?;
        let ct = cipher.encrypt(&Nonce::from(nonce), plaintext).map_err(|_| MessengerError::Crypto("sealing failed".into()))?;
        let mut out = nonce.to_vec();
        out.extend(ct);
        Ok(B64.encode(out))
    }

    pub fn open(&self, sealed: &str) -> Result<Vec<u8>> {
        let raw = B64.decode(sealed.trim()).map_err(|_| MessengerError::Crypto("not base64".into()))?;
        if raw.len() < NONCE_LEN + 16 {
            return Err(MessengerError::Crypto("too short".into()));
        }
        let (nonce, ct) = raw.split_at(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = nonce.try_into().expect("split at nonce length");
        Aes256Gcm::new_from_slice(&self.0)
            .map_err(crypto)?
            .decrypt(&Nonce::from(nonce), ct)
            .map_err(|_| MessengerError::Crypto("does not open with this key".into()))
    }
}

/// The secret inside the link of a public group.
#[derive(Clone, PartialEq, Eq)]
pub struct LinkSecret([u8; 32]);

impl std::fmt::Debug for LinkSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LinkSecret(…)")
    }
}

impl LinkSecret {
    pub fn generate() -> Result<Self> {
        Ok(Self(random()?))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Ok(Self(bytes.try_into().map_err(|_| MessengerError::Crypto("a link secret is 32 bytes".into()))?))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The group key this link gives.
    pub fn group_key(&self, group_id: &str, link_epoch: u32) -> GroupKey {
        let mut h = Sha256::new();
        h.update(b"veydan.group.key.v1\0");
        h.update(self.0);
        h.update(group_id.as_bytes());
        h.update(link_epoch.to_le_bytes());
        GroupKey(h.finalize().into())
    }
}

/// What a link or a QR code says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupLink {
    pub group_id: String,
    pub kind: GroupKind,
    pub relay: RelayUrl,
    pub name: String,
    pub owner: PubKey,
    /// Private: whom to ask (owner first). Public: empty.
    pub managers: Vec<PubKey>,
    /// Public only.
    pub secret: Option<LinkSecret>,
    pub link_epoch: u32,
}

const MAX_LINK_MANAGERS: usize = 3;

fn bad(what: &str) -> MessengerError {
    MessengerError::Invalid(format!("group link: {what}"))
}

impl GroupLink {
    pub fn encode(&self) -> String {
        let mut b = UriBuilder::new(&LinkType::Group, &self.group_id)
            .param("t", if self.kind == GroupKind::Public { "public" } else { "private" })
            .param("r", self.relay.as_str())
            .param("o", self.owner.as_hex())
            .param("n", &self.name)
            .list("m", self.managers.iter().take(MAX_LINK_MANAGERS).map(|p| p.as_hex()));
        if let Some(secret) = &self.secret {
            b = b.param("s", &B64URL.encode(secret.0)).param("e", &self.link_epoch.to_string());
        }
        b.build()
    }

    pub fn parse(link: &str) -> Result<Self> {
        let uri = Uri::parse(link).map_err(|e| match e {
            LinkError::Scheme | LinkError::Type => bad("not a group link"),
            LinkError::Id => bad("group id"),
            other => bad(other.code()),
        })?;
        Self::from_uri(&uri)
    }

    /// A link already taken apart (by whoever looked at its type first).
    pub fn from_uri(uri: &Uri) -> Result<Self> {
        if uri.link_type() != &LinkType::Group {
            return Err(bad("not a group link"));
        }
        let group_id = uri.id();
        if group_id.len() != 64 || !group_id.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
            return Err(bad("group id"));
        }
        let kind = match uri.raw("t") {
            Some("public") => GroupKind::Public,
            Some("private") => GroupKind::Private,
            _ => return Err(bad("type")),
        };
        let relay = uri.text("r").ok().flatten().and_then(|u| RelayUrl::parse(&u)).ok_or_else(|| bad("relay"))?;
        let owner = uri.raw("o").and_then(PubKey::parse).ok_or_else(|| bad("owner"))?;
        let name: String =
            uri.text("n").ok().flatten().unwrap_or_default().chars().filter(|c| !c.is_control()).take(100).collect();
        let managers =
            uri.list("m").unwrap_or_default().iter().filter_map(|m| PubKey::parse(m)).take(MAX_LINK_MANAGERS).collect();
        let secret = match uri.raw("s") {
            Some(v) => Some(LinkSecret::from_bytes(&B64URL.decode(v).map_err(|_| bad("secret"))?)?),
            None => None,
        };
        let link_epoch = match uri.raw("e") {
            Some(v) => v.parse().map_err(|_| bad("epoch"))?,
            None => 0,
        };
        if (kind == GroupKind::Public) != secret.is_some() {
            return Err(bad("a public link carries a secret, a private one does not"));
        }
        Ok(Self { group_id: group_id.to_string(), kind, relay, name, owner, managers, secret, link_epoch })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GID: &str = "00000000000000000000000000000000000000000000000000000000000000aa";

    fn pk(c: &str) -> PubKey {
        PubKey::parse(&c.repeat(64)[..64]).unwrap()
    }

    #[test]
    fn seal_and_open() {
        let k = GroupKey::generate().unwrap();
        let sealed = k.seal("привет".as_bytes()).unwrap();
        assert_ne!(sealed, k.seal("привет".as_bytes()).unwrap(), "a fresh nonce every time");
        assert_eq!(k.open(&sealed).unwrap(), "привет".as_bytes());
        assert!(GroupKey::generate().unwrap().open(&sealed).is_err());
        let mut raw = B64.decode(&sealed).unwrap();
        raw[15] ^= 1;
        assert!(k.open(&B64.encode(raw)).is_err());
        assert!(k.open("AAAA").is_err());
        assert!(k.open("not base64 !").is_err());
        assert!(GroupKey::from_bytes(&[0; 31]).is_err());
        assert_eq!(k.id(), GroupKey::from_bytes(k.as_bytes()).unwrap().id());
        assert!(!format!("{k:?}").contains(&hex::encode(k.as_bytes())));
    }

    #[test]
    fn link_key_derivation_vector() {
        let s = LinkSecret::from_bytes(&[7u8; 32]).unwrap();
        let k = s.group_key(GID, 0);
        assert_eq!(k, s.group_key(GID, 0));
        assert_ne!(k, s.group_key(GID, 1), "a new epoch is a new key");
        assert_ne!(k, s.group_key(&"bb".repeat(32), 0));
        assert_ne!(k, LinkSecret::from_bytes(&[8u8; 32]).unwrap().group_key(GID, 0));
        assert_eq!(hex::encode(k.as_bytes()), DERIVED);
    }

    /// Cross-checked with an independent SHA-256.
    const DERIVED: &str = "2fb892400cf121a2ea6e8c5731dbb5fb233e5b5de3c22e658a8bfb6a9f63f324";

    #[test]
    fn links_round_trip() {
        let public = GroupLink {
            group_id: GID.into(),
            kind: GroupKind::Public,
            relay: RelayUrl::parse("wss://node-1.veydan.net").unwrap(),
            name: "Клуб & друзья / 2026".into(),
            owner: pk("a"),
            managers: vec![],
            secret: Some(LinkSecret::from_bytes(&[9u8; 32]).unwrap()),
            link_epoch: 3,
        };
        let text = public.encode();
        assert!(text.starts_with("veydan://group/"));
        assert!(!text.contains(' ') && !text.contains('&') || text.matches('&').count() >= 4);
        assert_eq!(GroupLink::parse(&text).unwrap(), public);
        assert_eq!(GroupLink::parse(&format!("  {text}&future=1 ")).unwrap(), public, "unknown parameters are skipped");

        let private = GroupLink {
            kind: GroupKind::Private,
            managers: vec![pk("a"), pk("b"), pk("c"), pk("d")],
            secret: None,
            link_epoch: 0,
            ..public.clone()
        };
        let back = GroupLink::parse(&private.encode()).unwrap();
        assert_eq!(back.managers.len(), 3, "at most three managers in a link");
        assert_eq!(back.kind, GroupKind::Private);
        assert!(back.secret.is_none());
    }

    #[test]
    fn bad_links() {
        let ok = format!("veydan://group/{GID}?t=private&r=wss%3A%2F%2Fr.example&o={}&n=x", "a".repeat(64));
        assert!(GroupLink::parse(&ok).is_ok());
        for bad in [
            "https://example.com".to_string(),
            format!("veydan://group/{GID}"),
            format!("veydan://group/short?t=private&r=wss%3A%2F%2Fr.example&o={}", "a".repeat(64)),
            ok.replace("t=private", "t=secret"),
            ok.replace("t=private", "t=public"),
            ok.replace("wss%3A%2F%2Fr.example", "https%3A%2F%2Fr.example"),
            ok.replace(&format!("o={}", "a".repeat(64)), "o=zz"),
            format!("{ok}&s=AAAA"),
        ] {
            assert!(GroupLink::parse(&bad).is_err(), "{bad}");
        }
    }
}
