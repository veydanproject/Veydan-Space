// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The link of a channel of a community:
//! `veydan://channel/<community_id>?r=<relay>&o=<owner>&s=<secret>&e=<link_epoch>[&i=<invite_id>]`
//! (internal/communities-wire.md §10).
//!
//! Grammar only: what the link means is the business of the communities
//! module. Without `s` the link opens the card alone; `s` (32 bytes,
//! base64url without padding) needs `e`.

use crate::error::LinkError;
use crate::kind::LinkType;
use crate::uri::{Uri, UriBuilder};
use base64::Engine;
use messenger_core::{PubKey, RelayUrl};

const B64URL: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[derive(Clone, PartialEq, Eq)]
pub struct ChannelLink {
    /// 64 hex.
    pub community_id: String,
    pub relay: RelayUrl,
    pub owner: PubKey,
    pub secret: Option<[u8; 32]>,
    /// Present exactly when `secret` is.
    pub link_epoch: Option<u32>,
    /// 32 hex.
    pub invite: Option<String>,
}

impl std::fmt::Debug for ChannelLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChannelLink")
            .field("community_id", &self.community_id)
            .field("relay", &self.relay)
            .field("owner", &self.owner)
            .field("secret", &self.secret.as_ref().map(|_| "…"))
            .field("link_epoch", &self.link_epoch)
            .field("invite", &self.invite)
            .finish()
    }
}

impl ChannelLink {
    pub fn encode(&self) -> Result<String, LinkError> {
        if !is_hex(&self.community_id, 64) {
            return Err(LinkError::Id);
        }
        if self.secret.is_some() != self.link_epoch.is_some() {
            return Err(LinkError::Param);
        }
        let mut b = UriBuilder::new(&LinkType::Channel, &self.community_id)
            .param("r", self.relay.as_str())
            .param("o", self.owner.as_hex());
        if let (Some(s), Some(e)) = (&self.secret, self.link_epoch) {
            b = b.param("s", &B64URL.encode(s)).param("e", &e.to_string());
        }
        if let Some(i) = &self.invite {
            if !is_hex(i, 32) {
                return Err(LinkError::Param);
            }
            b = b.param("i", i);
        }
        Ok(b.build())
    }

    pub fn parse(link: &str) -> Result<Self, LinkError> {
        Self::from_uri(&Uri::parse(link)?)
    }

    pub fn from_uri(uri: &Uri) -> Result<Self, LinkError> {
        if uri.link_type() != &LinkType::Channel {
            return Err(LinkError::Type);
        }
        let community_id = uri.id();
        if !is_hex(community_id, 64) {
            return Err(LinkError::Id);
        }
        let relay = uri.text("r")?.and_then(|u| RelayUrl::parse(&u)).ok_or(LinkError::Param)?;
        let owner = uri.raw("o").and_then(PubKey::parse).ok_or(LinkError::Param)?;
        let secret = match uri.raw("s") {
            Some(v) => {
                let bytes = B64URL.decode(v).map_err(|_| LinkError::Param)?;
                Some(<[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| LinkError::Param)?)
            }
            None => None,
        };
        let link_epoch = match uri.raw("e") {
            Some(v) => Some(v.parse::<u32>().map_err(|_| LinkError::Param)?),
            None => None,
        };
        // A secret without its epoch is refused; an epoch alone says nothing.
        if secret.is_some() && link_epoch.is_none() {
            return Err(LinkError::Missing);
        }
        if secret.is_none() && link_epoch.is_some() {
            return Err(LinkError::Param);
        }
        let invite = uri.text("i")?;
        if invite.as_deref().is_some_and(|i| !is_hex(i, 32)) {
            return Err(LinkError::Param);
        }
        Ok(Self { community_id: community_id.to_string(), relay, owner, secret, link_epoch, invite })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CID: &str = "00000000000000000000000000000000000000000000000000000000000000aa";
    const OWNER: &str = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";
    const INVITE: &str = "0123456789abcdef0123456789abcdef";

    fn link(secret: bool, invite: bool) -> ChannelLink {
        ChannelLink {
            community_id: CID.into(),
            relay: RelayUrl::parse("wss://node-1.veydan.net").unwrap(),
            owner: PubKey::parse(OWNER).unwrap(),
            secret: secret.then_some([7u8; 32]),
            link_epoch: secret.then_some(3),
            invite: invite.then(|| INVITE.to_string()),
        }
    }

    #[test]
    fn a_full_link_round_trips() {
        let l = link(true, false);
        let text = l.encode().unwrap();
        assert!(text.starts_with(&format!("veydan://channel/{CID}?")));
        assert_eq!(ChannelLink::parse(&text).unwrap(), l);
    }

    #[test]
    fn a_card_only_link_has_no_secret() {
        let l = link(false, false);
        let text = l.encode().unwrap();
        assert!(!text.contains("s=") && !text.contains("e="));
        let back = ChannelLink::parse(&text).unwrap();
        assert_eq!((back.secret, back.link_epoch), (None, None));
    }

    #[test]
    fn the_invite_form_round_trips() {
        let l = link(true, true);
        assert_eq!(ChannelLink::parse(&l.encode().unwrap()).unwrap(), l);
        let card = link(false, true);
        assert_eq!(ChannelLink::parse(&card.encode().unwrap()).unwrap(), card);
    }

    #[test]
    fn a_secret_without_an_epoch_is_refused() {
        let text = link(true, false).encode().unwrap();
        let cut = text.split("&e=").next().unwrap();
        assert_eq!(ChannelLink::parse(cut), Err(LinkError::Missing));
        let mut l = link(true, false);
        l.link_epoch = None;
        assert!(l.encode().is_err());
    }

    #[test]
    fn bad_values_are_refused() {
        let good = link(true, true).encode().unwrap();
        let bad_id = good.replace(CID, &CID.to_uppercase());
        assert_eq!(ChannelLink::parse(&bad_id), Err(LinkError::Id));
        assert_eq!(ChannelLink::parse(&good.replace(CID, "abcd")), Err(LinkError::Id));
        assert!(ChannelLink::parse(&good.replace(OWNER, "zz")).is_err());
        assert!(ChannelLink::parse(&good.replace(INVITE, "xyz")).is_err());
        assert!(ChannelLink::parse(&good.replace("&e=3", "&e=-1")).is_err());
        assert!(ChannelLink::parse(&good.replace("wss%3A%2F%2F", "ftp%3A%2F%2F")).is_err());
        let short = format!("veydan://channel/{CID}?r=wss%3A%2F%2Fnode-1.veydan.net&o={OWNER}&s=AAAA&e=1");
        assert!(ChannelLink::parse(&short).is_err());
        let gl = good.replace("veydan://channel/", "veydan://group/");
        assert_eq!(ChannelLink::parse(&gl), Err(LinkError::Type));
    }

    #[test]
    fn other_types_stay_unknown() {
        assert_eq!(LinkType::of("channel"), LinkType::Channel);
        assert_eq!(LinkType::of("channels"), LinkType::Unknown("channels".into()));
    }
}
