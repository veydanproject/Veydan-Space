// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from crates/messenger/groups/src/keys.rs at 1f2ef31d, then changed for channels.

//! The link of a channel and its secret (internal/communities-wire.md §10).
//!
//! `veydan://channel/<community_id>?r=<relay>&o=<owner>&s=<secret>&e=<link_epoch>[&i=<invite_id>]`
//!
//! The secret is known to the keeper and every member: whoever joins by
//! the link shows the keeper a MAC of it bound to who they are
//! (`join_mac`), and the keeper checks it against the link epoch of the
//! journal. Without `s` the link opens the card and a request (or nothing,
//! for a channel by invitation); `i` names an invitation. The keys of the
//! content are the keeper's (keydist): a link gives no key.
//!
//! The type `channel` is not yet one `messenger-links` names (stage 1.4
//! adds it); until then it is parsed as the unknown word it is.

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use base64::Engine as _;
use messenger_core::{MessengerError, PubKey, RelayUrl, Result};
use messenger_links::{LinkError, LinkType, Uri, UriBuilder};
use sha2::{Digest, Sha256};

/// The word of the link type.
pub const LINK_TYPE: &str = "channel";
/// The label of the MAC of a join: that of groups, for channels.
pub const JOIN_LABEL: &[u8] = b"veydan.channel.join.v1\0";

fn crypto(e: impl std::fmt::Display) -> MessengerError {
    MessengerError::Crypto(e.to_string())
}

fn link_type() -> LinkType {
    LinkType::Unknown(LINK_TYPE.into())
}

/// The secret inside the link of a channel.
#[derive(Clone, PartialEq, Eq)]
pub struct LinkSecret([u8; 32]);

impl std::fmt::Debug for LinkSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LinkSecret(…)")
    }
}

impl LinkSecret {
    pub fn generate() -> Result<Self> {
        let mut b = [0u8; 32];
        getrandom::fill(&mut b).map_err(crypto)?;
        Ok(Self(b))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Ok(Self(bytes.try_into().map_err(|_| MessengerError::Crypto("a link secret is 32 bytes".into()))?))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// As it is written: in the link (`s`), in `Payload t:"link"`, in
    /// `channel.issue.link`.
    pub fn encode(&self) -> String {
        B64URL.encode(self.0)
    }

    pub fn decode(s: &str) -> Result<Self> {
        Self::from_bytes(&B64URL.decode(s).map_err(|_| MessengerError::Crypto("a link secret is base64url".into()))?)
    }

    /// What someone who joins by the link shows the keeper: a value only a
    /// holder of the secret of this link epoch can compute, bound to who
    /// they are. The MAC of groups word for word, with the label of
    /// channels: SHA-256(label ‖ secret ‖ id ‖ hex(pubkey) ‖ epoch LE).
    pub fn join_mac(&self, community_id: &str, author: &PubKey, link_epoch: u32) -> String {
        let mut h = Sha256::new();
        h.update(JOIN_LABEL);
        h.update(self.0);
        h.update(community_id.as_bytes());
        h.update(author.as_hex().as_bytes());
        h.update(link_epoch.to_le_bytes());
        hex::encode(h.finalize())
    }

    /// Whether `mac` is the MAC of `author` for this link epoch (compared
    /// in constant time: a keeper answers whoever asks).
    pub fn check_mac(&self, community_id: &str, author: &PubKey, link_epoch: u32, mac: &str) -> bool {
        let want = self.join_mac(community_id, author, link_epoch);
        want.len() == mac.len() && want.bytes().zip(mac.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
    }
}

/// What a link or a QR code says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelLink {
    pub community_id: String,
    /// Where a newcomer reads the journal (`#h`).
    pub relay: RelayUrl,
    /// The owner: a `channel.issue` is taken from the keeper of the
    /// journal whose `create` is theirs (§11.6).
    pub owner: PubKey,
    /// Of a channel by link; none opens the card only.
    pub secret: Option<LinkSecret>,
    pub link_epoch: u32,
    /// An invitation (16 bytes hex).
    pub invite_id: Option<String>,
}

fn bad(what: &str) -> MessengerError {
    MessengerError::Invalid(format!("channel link: {what}"))
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl ChannelLink {
    pub fn encode(&self) -> String {
        let mut b = UriBuilder::new(&link_type(), &self.community_id).param("r", self.relay.as_str()).param("o", self.owner.as_hex());
        if let Some(secret) = &self.secret {
            b = b.param("s", &secret.encode()).param("e", &self.link_epoch.to_string());
        }
        if let Some(i) = &self.invite_id {
            b = b.param("i", i);
        }
        b.build()
    }

    pub fn parse(link: &str) -> Result<Self> {
        let uri = Uri::parse(link).map_err(|e| match e {
            LinkError::Scheme | LinkError::Type => bad("not a channel link"),
            LinkError::Id => bad("community id"),
            other => bad(other.code()),
        })?;
        Self::from_uri(&uri)
    }

    /// A link already taken apart (by whoever looked at its type first).
    pub fn from_uri(uri: &Uri) -> Result<Self> {
        if uri.link_type().as_str() != LINK_TYPE {
            return Err(bad("not a channel link"));
        }
        let community_id = uri.id();
        if !is_hex(community_id, 64) {
            return Err(bad("community id"));
        }
        let relay = uri.text("r").ok().flatten().and_then(|u| RelayUrl::parse(&u)).ok_or_else(|| bad("relay"))?;
        let owner = uri.raw("o").and_then(PubKey::parse).ok_or_else(|| bad("owner"))?;
        let secret = match uri.raw("s") {
            Some(v) => Some(LinkSecret::decode(v).map_err(|_| bad("secret"))?),
            None => None,
        };
        let link_epoch = match uri.raw("e") {
            Some(v) => v.parse().map_err(|_| bad("epoch"))?,
            None => 0,
        };
        if secret.is_none() && uri.raw("e").is_some() {
            return Err(bad("an epoch without a secret"));
        }
        let invite_id = match uri.raw("i") {
            Some(i) if is_hex(i, 32) => Some(i.to_string()),
            Some(_) => return Err(bad("invite")),
            None => None,
        };
        Ok(Self { community_id: community_id.to_string(), relay, owner, secret, link_epoch, invite_id })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CID: &str = "00000000000000000000000000000000000000000000000000000000000000aa";

    fn pk(c: &str) -> PubKey {
        PubKey::parse(&c.repeat(64)[..64]).unwrap()
    }

    /// The MAC of a join for fixed inputs (also in tests/vectors.json).
    #[test]
    fn join_mac_vector() {
        let s = LinkSecret::from_bytes(&[7u8; 32]).unwrap();
        let mac = s.join_mac(CID, &pk("a"), 3);
        assert_eq!(mac, JOIN_MAC_VECTOR);
        assert_ne!(mac, s.join_mac(CID, &pk("a"), 4), "another link epoch");
        assert_ne!(mac, s.join_mac(CID, &pk("b"), 3), "somebody else");
        assert_ne!(mac, s.join_mac(&"bb".repeat(32), &pk("a"), 3), "another channel");
        assert_ne!(mac, LinkSecret::from_bytes(&[8u8; 32]).unwrap().join_mac(CID, &pk("a"), 3));
        assert!(s.check_mac(CID, &pk("a"), 3, &mac));
        assert!(!s.check_mac(CID, &pk("a"), 3, &mac[..63]));
        assert!(!s.check_mac(CID, &pk("b"), 3, &mac));
        assert!(!format!("{s:?}").contains("07"));
    }

    /// SHA-256 of the label, the secret, the id, the pubkey and the epoch,
    /// by an independent tool.
    pub(crate) const JOIN_MAC_VECTOR: &str = "9bef180acd74caeb0418efa932610da03124596844dae068d5eba15e80a30033";

    #[test]
    fn links_round_trip() {
        let open = ChannelLink {
            community_id: CID.into(),
            relay: RelayUrl::parse("wss://node-1.veydan.net").unwrap(),
            owner: pk("a"),
            secret: Some(LinkSecret::from_bytes(&[9u8; 32]).unwrap()),
            link_epoch: 3,
            invite_id: None,
        };
        let text = open.encode();
        assert!(text.starts_with("veydan://channel/"), "{text}");
        assert_eq!(ChannelLink::parse(&text).unwrap(), open);
        assert_eq!(ChannelLink::parse(&format!("  {text}&future=1 ")).unwrap(), open, "unknown parameters are skipped");

        let card_only = ChannelLink { secret: None, link_epoch: 0, ..open.clone() };
        let back = ChannelLink::parse(&card_only.encode()).unwrap();
        assert_eq!(back, card_only);
        assert!(!card_only.encode().contains("&e="));

        let invited = ChannelLink { invite_id: Some("cd".repeat(16)), ..card_only };
        assert_eq!(ChannelLink::parse(&invited.encode()).unwrap(), invited);
    }

    #[test]
    fn bad_links() {
        let ok = format!("veydan://channel/{CID}?r=wss%3A%2F%2Fr.example&o={}", "a".repeat(64));
        assert!(ChannelLink::parse(&ok).is_ok());
        for bad in [
            "https://example.com".to_string(),
            format!("veydan://channel/{CID}"),
            format!("veydan://group/{CID}?r=wss%3A%2F%2Fr.example&o={}", "a".repeat(64)),
            format!("veydan://channel/short?r=wss%3A%2F%2Fr.example&o={}", "a".repeat(64)),
            ok.replace("wss%3A%2F%2Fr.example", "https%3A%2F%2Fr.example"),
            ok.replace(&format!("o={}", "a".repeat(64)), "o=zz"),
            format!("{ok}&s=AAAA"),
            format!("{ok}&e=1"),
            format!("{ok}&i=xyz"),
            format!("{ok}&s={}&e=x", LinkSecret::from_bytes(&[1u8; 32]).unwrap().encode()),
        ] {
            assert!(ChannelLink::parse(&bad).is_err(), "{bad}");
        }
    }
}
