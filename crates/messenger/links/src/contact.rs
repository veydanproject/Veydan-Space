// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The link of a person: `veydan://contact/<npub>?n=<name>&r=<relay>,<relay>&i=<invite>&srv=<server card>`.
//!
//! The key is the person. The name is only what the sender called them,
//! shown until their own profile is known; the relays say where to look.
//! The invitation (`i`) is a secret of theirs that lets whoever opens the
//! link write to them without a request: 16 bytes as base64url without
//! padding, 22 characters, and nothing else is one (a code is told by
//! mouth, never written into a link).
//!
//! The server card (`srv`) says the person is reached on a server
//! (`server::ServerLink`, as base64url): whoever opens the link adds that
//! server, after asking, and talks to them there, in the private chat of
//! that server.

use crate::error::LinkError;
use crate::kind::LinkType;
use crate::server::ServerLink;
use crate::uri::{Uri, UriBuilder};
use messenger_core::{PubKey, RelayUrl};
use nostr::nips::nip19::{FromBech32, ToBech32};
use nostr::key::PublicKey;

const MAX_NAME_CHARS: usize = 100;
const MAX_LINK_RELAYS: usize = 3;
/// 16 bytes in base64url without padding.
pub const INVITE_LINK_CHARS: usize = 22;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactLink {
    pub pubkey: PubKey,
    /// Empty when the link gives none.
    pub name: String,
    pub relays: Vec<RelayUrl>,
    /// The secret of an invitation, when the link carries one.
    pub invite: Option<String>,
    /// The card of the server the person is reached on, when the link
    /// carries one: the chat is the private one there, not the home one.
    pub srv: Option<ServerLink>,
}

/// Whether `s` is the secret of a link invitation as the grammar writes
/// it: 22 characters of the base64url alphabet (16 bytes, no padding).
pub fn is_invite_secret(s: &str) -> bool {
    s.len() == INVITE_LINK_CHARS && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The link of a person put together from its parts: what the facade of
/// invitations writes.
pub fn contact_link(pubkey: &PubKey, name: &str, relays: &[RelayUrl], invite: Option<&str>) -> Result<String, LinkError> {
    ContactLink { pubkey: pubkey.clone(), name: name.into(), relays: relays.to_vec(), invite: invite.map(String::from), srv: None }.encode()
}

/// `npub1…` → key. Nothing else is accepted: a secret key never parses.
pub fn pubkey_of_npub(npub: &str) -> Option<PubKey> {
    if !npub.starts_with("npub1") {
        return None;
    }
    PublicKey::from_bech32(npub).ok().and_then(|p| PubKey::parse(&p.to_hex()))
}

pub fn npub_of(pubkey: &PubKey) -> Option<String> {
    PublicKey::from_hex(pubkey.as_hex()).ok().and_then(|p| p.to_bech32().ok())
}

fn clean_name(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).take(MAX_NAME_CHARS).collect::<String>().trim().to_string()
}

impl ContactLink {
    pub fn encode(&self) -> Result<String, LinkError> {
        let npub = npub_of(&self.pubkey).ok_or(LinkError::Id)?;
        let mut b = UriBuilder::new(&LinkType::Contact, &npub);
        let name = clean_name(&self.name);
        if !name.is_empty() {
            b = b.param("n", &name);
        }
        b = b.list("r", self.relays.iter().take(MAX_LINK_RELAYS).map(|r| r.as_str()));
        if let Some(i) = &self.invite {
            if !is_invite_secret(i) {
                return Err(LinkError::Param);
            }
            b = b.param("i", i);
        }
        if let Some(srv) = &self.srv {
            b = b.param("srv", &srv.srv_param());
        }
        Ok(b.build())
    }

    pub fn parse(link: &str) -> Result<Self, LinkError> {
        Self::from_uri(&Uri::parse(link)?)
    }

    pub fn from_uri(uri: &Uri) -> Result<Self, LinkError> {
        if uri.link_type() != &LinkType::Contact {
            return Err(LinkError::Type);
        }
        let pubkey = pubkey_of_npub(uri.id()).ok_or(LinkError::Id)?;
        let name = clean_name(&uri.text("n")?.unwrap_or_default());
        let mut relays = Vec::new();
        for r in uri.list("r")?.into_iter().take(MAX_LINK_RELAYS) {
            relays.push(RelayUrl::parse(&r).ok_or(LinkError::Param)?);
        }
        // A secret of another shape is no secret of ours: refused, not
        // carried, so nothing is ever sent as an invitation that is not one.
        let invite = uri.text("i")?;
        if invite.as_deref().is_some_and(|i| !is_invite_secret(i)) {
            return Err(LinkError::Param);
        }
        let srv = match uri.raw("srv") {
            Some(raw) => Some(ServerLink::from_srv(raw)?),
            None => None,
        };
        Ok(Self { pubkey, name, relays, invite, srv })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    const HEX: &str = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";
    /// NIP-19 test vector for `HEX`.
    const NPUB: &str = "npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6";

    fn relay(u: &str) -> RelayUrl {
        RelayUrl::parse(u).unwrap()
    }

    #[test]
    fn vector() {
        let link = ContactLink {
            pubkey: PubKey::parse(HEX).unwrap(),
            name: "Алиса М.".into(),
            relays: vec![relay("wss://node-1.veydan.net"), relay("wss://r.example:7777")],
            invite: None,
            srv: None,
        };
        let text = link.encode().unwrap();
        assert_eq!(
            text,
            format!("veydan://contact/{NPUB}?n=%D0%90%D0%BB%D0%B8%D1%81%D0%B0%20%D0%9C.&r=wss%3A%2F%2Fnode-1.veydan.net,wss%3A%2F%2Fr.example%3A7777")
        );
        assert_eq!(ContactLink::parse(&text).unwrap(), link);
        assert_eq!(ContactLink::parse(&format!(" {text}&future=1 ")).unwrap(), link, "unknown parameters are skipped");
    }

    #[test]
    fn the_key_alone_is_enough() {
        let bare = ContactLink { pubkey: PubKey::parse(HEX).unwrap(), name: " \u{7} ".into(), relays: vec![], invite: None, srv: None };
        let text = bare.encode().unwrap();
        assert_eq!(text, format!("veydan://contact/{NPUB}"));
        let back = ContactLink::parse(&text).unwrap();
        assert_eq!(back.pubkey.as_hex(), HEX);
        assert!(back.name.is_empty() && back.relays.is_empty());
    }

    #[test]
    fn limits() {
        let many = ContactLink {
            pubkey: PubKey::parse(HEX).unwrap(),
            name: "x".repeat(300),
            relays: (1..=5).map(|i| relay(&format!("wss://r{i}.example"))).collect(),
            invite: None,
            srv: None,
        };
        let back = ContactLink::parse(&many.encode().unwrap()).unwrap();
        assert_eq!(back.name.chars().count(), MAX_NAME_CHARS);
        assert_eq!(back.relays.len(), MAX_LINK_RELAYS);
    }

    #[test]
    fn refusals() {
        let ok = format!("veydan://contact/{NPUB}");
        assert!(ContactLink::parse(&ok).is_ok());
        let mut wrong_sum = NPUB.to_string();
        wrong_sum.pop();
        wrong_sum.push('7');
        for (text, why) in [
            (format!("veydan://group/{NPUB}"), LinkError::Type),
            (format!("veydan://contact/{HEX}"), LinkError::Id),
            (format!("veydan://contact/{wrong_sum}"), LinkError::Id),
            ("veydan://contact/nsec1vl029mgpspedva04g90vltkh6fvh240zqtv9k0t9af8935ke9laqsnlfe5".to_string(), LinkError::Id),
            (format!("{ok}?r=https%3A%2F%2Fr.example"), LinkError::Param),
            (format!("{ok}?n=%ZZ"), LinkError::Param),
            (format!("{ok}?n=a&n=b"), LinkError::Param),
            (format!("https://veydan.net/contact/{NPUB}"), LinkError::Scheme),
            (format!("{ok}?i=123456"), LinkError::Param),
            (format!("{ok}?i={}", "A".repeat(23)), LinkError::Param),
            (format!("{ok}?i={}%3D", "A".repeat(21)), LinkError::Param),
            (format!("{ok}?i="), LinkError::Param),
            (format!("{ok}?i=AAECAwQFBgcICQoLDA0O_w&i=AAECAwQFBgcICQoLDA0O_w"), LinkError::Param),
        ] {
            assert_eq!(ContactLink::parse(&text), Err(why), "{text}");
        }
    }

    /// What `veydan-server card --contact <npub>` of the kit prints: the
    /// person, and the card of the server as `srv`, base64url without
    /// padding.
    #[test]
    fn a_person_on_a_server_of_their_own() {
        let card = crate::server::ServerLink::parse(&format!(
            "veydan://server/{}?u=wss%3A%2F%2Frelay.s1.example&k=app-key&m=https%3A%2F%2Fmedia.s1.example&p=off&n=S1",
            crate::server_id("wss://relay.s1.example")
        ))
        .unwrap();
        let kit = format!(
            "veydan://contact/{NPUB}?srv={}",
            base64::engine::general_purpose::STANDARD.encode(card.encode()).replace('+', "-").replace('/', "_").replace('=', "")
        );
        let link = ContactLink::parse(&kit).unwrap();
        assert_eq!(link.pubkey.as_hex(), HEX);
        assert_eq!(link.srv.as_ref(), Some(&card));
        assert_eq!(link.encode().unwrap(), kit, "written back as the kit writes it");

        let mine = ContactLink { name: "Bob".into(), relays: vec![relay("wss://relay.s1.example")], ..link.clone() };
        assert_eq!(ContactLink::parse(&mine.encode().unwrap()).unwrap(), mine);
        assert_eq!(ContactLink::parse(&format!("veydan://contact/{NPUB}?srv=AAAA")), Err(LinkError::Param), "a card that is none");
    }

    /// The vector of the invitation: 16 bytes, base64url, no padding.
    #[test]
    fn invite_vector() {
        const SECRET: &str = "dmV5ZGFuLWludml0ZS0wMQ"; // base64url of `veydan-invite-01`
        let pubkey = PubKey::parse(HEX).unwrap();
        let text = contact_link(&pubkey, "Alice", &[relay("wss://r.example")], Some(SECRET)).unwrap();
        assert_eq!(text, format!("veydan://contact/{NPUB}?n=Alice&r=wss%3A%2F%2Fr.example&i={SECRET}"));
        let back = ContactLink::parse(&text).unwrap();
        assert_eq!(back.invite.as_deref(), Some(SECRET));
        assert_eq!(back.name, "Alice");
        let bare = contact_link(&pubkey, "", &[], Some(SECRET)).unwrap();
        assert_eq!(bare, format!("veydan://contact/{NPUB}?i={SECRET}"));
        assert!(is_invite_secret(SECRET));
        assert!(!is_invite_secret("123456"), "a code is said, never linked");
        assert!(!is_invite_secret(&format!("{}+", "A".repeat(21))), "not the alphabet of base64url");
        assert_eq!(contact_link(&pubkey, "", &[], Some("short")), Err(LinkError::Param), "a secret of another shape is not written");
    }
}
