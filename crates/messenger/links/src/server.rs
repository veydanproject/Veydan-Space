// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The card of a server (internal/messenger-wire.md, «Ссылка»):
//! `veydan://server/<id>?u=<relay>&k=<app key>[&m=<media>][&c=<call node>][&pu=<push server>]&p=veydan|off&n=<name>`.
//!
//! What the kit of a server prints (`veydan-server card`) and what the app
//! takes to add it as a private server. A server is one relay with what
//! goes with it: the app key its relay and blob server want, its blob
//! server (`https://…`), its call node (`address:port#id`, as the node's
//! tools write it), its push server (`https://…`, where a phone registers
//! through the gateway), how phones are woken (`veydan` through the push of the
//! Veydan network, `off`) and a name. The id is the first 16 hex of the
//! SHA-256 of the relay url in its normal form (trimmed, lowercase, no
//! trailing slash), the id every device derives for itself
//! (`messenger_store::servers::id_of`): a link whose id is not that of its
//! relay names no server and is refused, so a card can never put one
//! server's name on another's relay.
//!
//! A link of a person or of a group may carry the card of the server it
//! lives on as `srv=`: the server link as base64url without padding
//! (`srv_param`, `from_srv`), so a person on another server opens the
//! chat there after adding it.

use crate::error::LinkError;
use crate::kind::LinkType;
use crate::uri::{Uri, UriBuilder, MAX_LINK_LEN};
use base64::Engine;
use messenger_core::RelayUrl;
use sha2::{Digest, Sha256};
use std::net::SocketAddr;

const B64URL: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// Hex characters of a server id.
pub const SERVER_ID_LEN: usize = 16;
const MAX_NAME_CHARS: usize = 100;
/// The app key: a word the kit made; anything longer is not one.
const MAX_KEY_LEN: usize = 256;
const MAX_URL_LEN: usize = 512;

pub const PUSH_OFF: &str = "off";
pub const PUSH_VEYDAN: &str = "veydan";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerLink {
    /// 16 hex, lower case: `server_id(relay)`.
    pub id: String,
    pub relay: RelayUrl,
    /// The app key; `None` for a server that wants none.
    pub key: Option<String>,
    /// The blob server, `https://…`.
    pub media: Option<String>,
    /// The call node, `address:port#id`.
    pub call_node: Option<String>,
    /// The push server of the server, `https://…` (`http` only for this
    /// machine, for tests): where a phone registers through the gateway.
    pub push_url: Option<String>,
    /// `veydan` | `off`.
    pub push: String,
    /// Empty when the card gives none.
    pub name: String,
}

/// The form a relay url is compared and hashed in: trimmed, lowercase,
/// without a trailing slash (the one of `messenger_store::servers`).
pub fn normalize_relay(relay_url: &str) -> String {
    relay_url.trim().trim_end_matches('/').to_ascii_lowercase()
}

/// The id of the server of `relay_url`: the first 16 hex of the SHA-256 of
/// its normal form, as the kit and `messenger_store::servers::id_of` make it.
pub fn server_id(relay_url: &str) -> String {
    let digest = Sha256::digest(normalize_relay(relay_url).as_bytes());
    digest
        .iter()
        .take(SERVER_ID_LEN / 2)
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn clean_name(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

/// A word without spaces or controls, of at most `max` bytes; empty is none.
fn word(value: Option<String>, max: usize) -> Result<Option<String>, LinkError> {
    match value {
        None => Ok(None),
        Some(v) if v.is_empty() => Ok(None),
        Some(v) if v.len() > max || v.chars().any(|c| c.is_control() || c.is_whitespace()) => {
            Err(LinkError::Param)
        }
        Some(v) => Ok(Some(v)),
    }
}

/// An `https` address with a host: the blob server.
fn media_url(value: Option<String>) -> Result<Option<String>, LinkError> {
    let Some(url) = word(value, MAX_URL_LEN)? else {
        return Ok(None);
    };
    let rest = url.strip_prefix("https://").ok_or(LinkError::Param)?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() || host.contains('@') {
        return Err(LinkError::Param);
    }
    Ok(Some(url.trim_end_matches('/').to_string()))
}

/// The push server: `https://host[/path]`, or `http://` for localhost and
/// 127.0.0.1 (tests). No credentials, query or fragment.
fn push_server_url(value: Option<String>) -> Result<Option<String>, LinkError> {
    let Some(url) = word(value, MAX_URL_LEN)? else {
        return Ok(None);
    };
    let rest = if let Some(rest) = url.strip_prefix("https://") {
        rest
    } else if let Some(rest) = url.strip_prefix("http://") {
        let host = rest.split(['/', ':']).next().unwrap_or("");
        if !matches!(host, "localhost" | "127.0.0.1") {
            return Err(LinkError::Param);
        }
        rest
    } else {
        return Err(LinkError::Param);
    };
    if rest.is_empty() || rest.starts_with('/') || rest.contains(['?', '#', '@']) {
        return Err(LinkError::Param);
    }
    Ok(Some(url.trim_end_matches('/').to_string()))
}

/// The relay of a card: `wss://`, as the push server is `https://`. The app
/// key goes on the relay's address and the login (AUTH) over it, both in
/// clear on `ws://`; that is taken for this machine alone (`localhost`,
/// `127.0.0.1`, tests and a kit tried at home).
fn secure_relay(relay: &str) -> bool {
    if relay.starts_with("wss://") {
        return true;
    }
    let Some(rest) = relay.strip_prefix("ws://") else { return false };
    let host = rest.split(['/', ':', '?', '#']).next().unwrap_or("");
    matches!(host, "localhost" | "127.0.0.1")
}

/// `address:port#id` as a call node's tools write it: a socket address
/// (a node is called by address, never by a name to look up) and the 64
/// hex of its key.
fn call_ref(value: Option<String>) -> Result<Option<String>, LinkError> {
    let Some(r) = word(value, MAX_URL_LEN)? else {
        return Ok(None);
    };
    let (addr, id) = r.split_once('#').ok_or(LinkError::Param)?;
    let addr: SocketAddr = addr.parse().map_err(|_| LinkError::Param)?;
    if addr.port() == 0 || id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(LinkError::Param);
    }
    Ok(Some(format!("{addr}#{}", id.to_ascii_lowercase())))
}

impl ServerLink {
    /// The card of the server of `relay`, its id derived from it.
    pub fn new(relay: RelayUrl) -> Self {
        Self {
            id: server_id(relay.as_str()),
            relay,
            key: None,
            media: None,
            call_node: None,
            push_url: None,
            push: PUSH_OFF.into(),
            name: String::new(),
        }
    }

    /// The card of a server this device holds, made of its parts as a
    /// store keeps them; `None` when they do not make a card the parser
    /// would take (a relay that is no `ws(s)://` address, a blob server
    /// that is not `https`, a call node not written `address:port#id`).
    pub fn of_parts(
        relay_url: &str,
        key: Option<&str>,
        media: Option<&str>,
        call_node: Option<&str>,
        push_url: Option<&str>,
        push: &str,
        name: Option<&str>,
    ) -> Option<Self> {
        let card = Self {
            key: key.map(String::from),
            media: media.map(String::from),
            call_node: call_node.map(String::from),
            push_url: push_url.map(String::from),
            push: push.to_string(),
            name: name.unwrap_or_default().to_string(),
            ..Self::new(RelayUrl::parse(&normalize_relay(relay_url))?)
        };
        Self::parse(&card.encode()).ok()
    }

    /// As the kit writes it: `u`, `k` (empty when there is no key), `m`
    /// `c` and `pu` when there are, `p`, `n`.
    pub fn encode(&self) -> String {
        let relay = normalize_relay(self.relay.as_str());
        let mut b = UriBuilder::new(&LinkType::Server, &server_id(&relay))
            .param("u", &relay)
            .param("k", self.key.as_deref().unwrap_or(""));
        if let Some(m) = &self.media {
            b = b.param("m", m);
        }
        if let Some(c) = &self.call_node {
            b = b.param("c", c);
        }
        if let Some(pu) = &self.push_url {
            b = b.param("pu", pu);
        }
        b.param("p", &self.push)
            .param("n", &clean_name(&self.name))
            .build()
    }

    pub fn parse(link: &str) -> Result<Self, LinkError> {
        Self::from_uri(&Uri::parse(link)?)
    }

    pub fn from_uri(uri: &Uri) -> Result<Self, LinkError> {
        if uri.link_type() != &LinkType::Server {
            return Err(LinkError::Type);
        }
        let id = uri.id().to_ascii_lowercase();
        if id.len() != SERVER_ID_LEN || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(LinkError::Id);
        }
        let relay = uri.text("u")?.ok_or(LinkError::Missing)?;
        let relay = RelayUrl::parse(&normalize_relay(&relay)).ok_or(LinkError::Param)?;
        if !secure_relay(relay.as_str()) {
            return Err(LinkError::Param);
        }
        // The id is the relay's: a card cannot name another server.
        if server_id(relay.as_str()) != id {
            return Err(LinkError::Id);
        }
        let push = match uri.text("p")?.as_deref() {
            None | Some("") | Some(PUSH_OFF) => PUSH_OFF.to_string(),
            Some(PUSH_VEYDAN) => PUSH_VEYDAN.to_string(),
            Some(_) => return Err(LinkError::Param),
        };
        Ok(Self {
            id,
            relay,
            key: word(uri.text("k")?, MAX_KEY_LEN)?,
            media: media_url(uri.text("m")?)?,
            call_node: call_ref(uri.text("c")?)?,
            push_url: push_server_url(uri.text("pu")?)?,
            push,
            name: clean_name(&uri.text("n")?.unwrap_or_default()),
        })
    }

    /// The card as the `srv=` of a link of a person or a group: base64url
    /// of the server link, without padding.
    pub fn srv_param(&self) -> String {
        B64URL.encode(self.encode())
    }

    /// The card a `srv=` carries. Padding is forgiven; anything that is not
    /// base64url of a server link is refused.
    pub fn from_srv(raw: &str) -> Result<Self, LinkError> {
        let raw = raw.trim_end_matches('=');
        if raw.is_empty() || raw.len() > MAX_LINK_LEN {
            return Err(LinkError::Param);
        }
        let bytes = B64URL.decode(raw).map_err(|_| LinkError::Param)?;
        let text = String::from_utf8(bytes).map_err(|_| LinkError::Param)?;
        Self::parse(&text).map_err(|_| LinkError::Param)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELAY: &str = "wss://relay.s1.example";
    const NODE: &str =
        "203.0.113.7:8443#fda09da75199c4e04601a9df309710fab15cd7b2b806ca3b2bbab202582a6dca";

    /// What `veydan-server card` of the kit prints
    /// (services/server/templates/veydan-server, `server_link`): every value
    /// through jq's `@uri`, which leaves `A-Z a-z 0-9 - _ . ~` and writes
    /// the rest as `%XX`, as `uri::encode` does.
    fn kit_card(id: &str, key: &str, media: &str, call: &str, push: &str, network: &str) -> String {
        let uri = crate::uri::encode;
        let mut q = format!("u={}&k={}", uri(RELAY), uri(key));
        if !media.is_empty() {
            q += &format!("&m={}", uri(media));
        }
        if !call.is_empty() {
            q += &format!("&c={}", uri(call));
        }
        q += &format!("&p={}&n={}", uri(push), uri(network));
        format!("veydan://server/{id}?{q}")
    }

    #[test]
    fn the_id_is_the_one_the_kit_and_the_store_derive() {
        // printf '%s' wss://relay.s1.example | sha256sum | cut -c1-16
        let id = server_id(RELAY);
        assert_eq!(id, "e06676e9354ae2e2");
        assert_eq!(
            server_id("  wss://Relay.S1.example/ "),
            id,
            "one server in any spelling"
        );
        assert_ne!(server_id("wss://relay.s2.example"), id);
    }

    #[test]
    fn the_card_of_the_kit_is_read() {
        let id = server_id(RELAY);
        let text = kit_card(
            &id,
            "k3y_of-the.app~1",
            "https://media.s1.example",
            NODE,
            "veydan",
            "Сеть S1",
        );
        let card = ServerLink::parse(&text).unwrap();
        assert_eq!(card.id, id);
        assert_eq!(card.relay.as_str(), RELAY);
        assert_eq!(card.key.as_deref(), Some("k3y_of-the.app~1"));
        assert_eq!(card.media.as_deref(), Some("https://media.s1.example"));
        assert_eq!(card.call_node.as_deref(), Some(NODE));
        assert_eq!(card.push, PUSH_VEYDAN);
        assert_eq!(card.name, "Сеть S1");
        assert_eq!(card.encode(), text, "written back as the kit writes it");

        // A server of the relay alone: no media, no node, no key, no name.
        let bare = kit_card(&id, "", "", "", "off", "");
        let card = ServerLink::parse(&bare).unwrap();
        assert_eq!(
            (
                card.key,
                card.media,
                card.call_node,
                card.push.as_str(),
                card.name.as_str()
            ),
            (None, None, None, PUSH_OFF, "")
        );
        assert_eq!(ServerLink::parse(&bare).unwrap().encode(), bare);
        assert_eq!(
            ServerLink::parse(&format!(" {bare}&future=1 ")).unwrap(),
            ServerLink::parse(&bare).unwrap(),
            "unknown parameters are skipped"
        );
    }

    #[test]
    fn the_push_server_of_the_kit_card_is_read() {
        let id = server_id(RELAY);
        let uri = crate::uri::encode;
        let bare = kit_card(&id, "k", "https://media.s1.example", "", "veydan", "S1");
        // The kit puts pu= after m= and c=, before p=.
        let with = bare.replace(
            "&p=veydan",
            &format!("&pu={}&p=veydan", uri("https://push.s1.example")),
        );
        let card = ServerLink::parse(&with).unwrap();
        assert_eq!(card.push_url.as_deref(), Some("https://push.s1.example"));
        assert_eq!(card.encode(), with, "written back as the kit writes it");
        assert_eq!(ServerLink::parse(&bare).unwrap().push_url, None);
        assert!(!ServerLink::parse(&bare).unwrap().encode().contains("pu="));
        // A trailing slash is dropped; localhost may use http (tests).
        let slash = bare.replace("&p=veydan", &format!("&pu={}&p=veydan", uri("https://push.s1.example/")));
        assert_eq!(ServerLink::parse(&slash).unwrap().push_url.as_deref(), Some("https://push.s1.example"));
        let local = bare.replace("&p=veydan", &format!("&pu={}&p=veydan", uri("http://127.0.0.1:8090")));
        assert_eq!(ServerLink::parse(&local).unwrap().push_url.as_deref(), Some("http://127.0.0.1:8090"));
        for bad in [
            "http://push.s1.example",
            "ftp://push.s1.example",
            "push.s1.example",
            "https://",
            "https:///x",
            "https://u@push.s1.example",
            "https://push.s1.example/?x=1",
            "https://push.s1.example/#x",
            "https://push s1.example",
        ] {
            let text = bare.replace("&p=veydan", &format!("&pu={}&p=veydan", uri(bad)));
            assert_eq!(ServerLink::parse(&text), Err(LinkError::Param), "{bad}");
        }
    }

    #[test]
    fn what_is_written_is_read() {
        let mut card = ServerLink::new(RelayUrl::parse("wss://Relay.S1.example:7777/").unwrap());
        card.key = Some("secret".into());
        card.name = "S1 & co / 2026".into();
        let text = card.encode();
        assert!(text.starts_with(&format!(
            "veydan://server/{}?u=wss%3A%2F%2Frelay.s1.example%3A7777&k=secret&p=off&n=",
            card.id
        )));
        let back = ServerLink::parse(&text).unwrap();
        assert_eq!(back.relay.as_str(), "wss://relay.s1.example:7777");
        assert_eq!(
            (back.id, back.name, back.key),
            (card.id.clone(), card.name.clone(), card.key.clone())
        );
    }

    #[test]
    fn a_card_naming_another_relay_is_refused() {
        let id = server_id(RELAY);
        let other = server_id("wss://relay.s2.example");
        let ok = kit_card(&id, "k", "", "", "off", "S1");
        assert!(ServerLink::parse(&ok).is_ok());
        for (text, why) in [
            (kit_card(&other, "k", "", "", "off", "S1"), LinkError::Id),
            (ok.replace(&id, &id[..15]), LinkError::Id),
            (ok.replace(&id, &format!("{}zz", &id[..14])), LinkError::Id),
            (format!("veydan://server/{id}?k=x"), LinkError::Missing),
            (ok.replace("u=wss", "u=https"), LinkError::Param),
            (ok.replace("p=off", "p=fcm"), LinkError::Param),
            (
                format!("{ok}&m=http%3A%2F%2Fmedia.example"),
                LinkError::Param,
            ),
            (
                format!("{ok}&m=https%3A%2F%2Fu%40media.example"),
                LinkError::Param,
            ),
            (
                format!("{ok}&c=node.example%3A443%23{}", "a".repeat(64)),
                LinkError::Param,
            ),
            (format!("{ok}&c=1.2.3.4%3A443%23abc"), LinkError::Param),
            (ok.replace("k=k", "k=a%20b"), LinkError::Param),
            (format!("veydan://group/{id}?u=x"), LinkError::Type),
        ] {
            assert_eq!(ServerLink::parse(&text), Err(why), "{text}");
        }
        // Upper-case hex is the same id.
        assert_eq!(
            ServerLink::parse(&ok.replace(&id, &id.to_uppercase()))
                .unwrap()
                .id,
            id
        );
    }

    /// The app key rides on the relay's address and the login goes over
    /// it: a card of a `ws://` relay would send both in clear, and is
    /// refused, but on this machine.
    #[test]
    fn a_relay_in_clear_is_refused_but_on_this_machine() {
        let card = |relay: &str| {
            let id = server_id(relay);
            format!("veydan://server/{id}?u={}&k=key&p=off&n=S1", crate::uri::encode(relay))
        };
        assert!(ServerLink::parse(&card("wss://relay.s1.example")).is_ok());
        for local in ["ws://127.0.0.1:7777", "ws://localhost:7777"] {
            assert!(ServerLink::parse(&card(local)).is_ok(), "{local}");
        }
        for clear in [
            "ws://relay.s1.example",
            "ws://203.0.113.7:7777",
            "ws://127.0.0.1.relay.example",
            "ws://localhost.relay.example",
            "ws://localhost@relay.example",
        ] {
            assert_eq!(ServerLink::parse(&card(clear)), Err(LinkError::Param), "{clear}");
            let srv = B64URL.encode(card(clear));
            assert_eq!(ServerLink::from_srv(&srv), Err(LinkError::Param), "{clear} as srv=");
        }
        // A server this device holds with such a relay makes no card either.
        assert!(ServerLink::of_parts("ws://relay.s1.example", Some("k"), None, None, None, "off", None).is_none());
    }

    #[test]
    fn a_card_rides_on_another_link_as_srv() {
        let card = ServerLink::parse(&kit_card(
            &server_id(RELAY),
            "k",
            "https://media.s1.example",
            NODE,
            "veydan",
            "S1",
        ))
        .unwrap();
        let srv = card.srv_param();
        assert!(
            srv.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "unreserved: written as it is"
        );
        assert_eq!(ServerLink::from_srv(&srv).unwrap(), card);
        assert_eq!(
            ServerLink::from_srv(&format!("{srv}==")).unwrap(),
            card,
            "padding is forgiven"
        );
        // What the kit writes for --contact: base64 -w0 | tr '+/' '-_' | tr -d '='.
        let kit = base64::engine::general_purpose::STANDARD
            .encode(card.encode())
            .replace('+', "-")
            .replace('/', "_")
            .replace('=', "");
        assert_eq!(kit, srv);
        for bad in [
            "",
            "!!!",
            "dmV5ZGFuOi8vZ3JvdXAvYQ",
            &B64URL.encode("veydan://server/0123456789abcdef?u=wss%3A%2F%2Fr.example"),
        ] {
            assert_eq!(ServerLink::from_srv(bad), Err(LinkError::Param), "{bad}");
        }
    }
}
