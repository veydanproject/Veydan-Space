// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! How a request proves who is asking (NIP-98).
//!
//! The request carries an event of kind 27235, signed by the user's key,
//! that names the address, the method and the body it was signed for.
//!
//! Built by hand: the helper of the `nostr` crate knows no DELETE, and a
//! request is accepted once, so every event gets a `nonce` of its own. Two
//! requests of one second would be one event without it.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use messenger_core::{MessengerError, Result};
use nostr::key::Keys;
use nostr::prelude::*;
use sha2::{Digest, Sha256};

/// Kind of the event, NIP-98.
const HTTP_AUTH: u16 = 27235;

fn crypto(e: impl std::fmt::Display) -> MessengerError {
    MessengerError::Crypto(e.to_string())
}

fn nonce() -> Result<String> {
    let mut bytes = [0u8; 12];
    getrandom::fill(&mut bytes).map_err(crypto)?;
    Ok(hex::encode(bytes))
}

/// The value of the `Authorization` header.
///
/// `url` is the address of the request, whole and exactly as it is asked.
/// `at` is unix seconds: the clock of this device, corrected by what is
/// known about the server's.
pub fn auth_header(keys: &Keys, method: &str, url: &str, body: &[u8], at: u64) -> Result<String> {
    let mut tags = vec![
        Tag::parse(["u", url]).map_err(crypto)?,
        Tag::parse(["method", method]).map_err(crypto)?,
        Tag::parse(["nonce", nonce()?.as_str()]).map_err(crypto)?,
    ];
    if !body.is_empty() {
        let hash = hex::encode(Sha256::digest(body));
        tags.push(Tag::parse(["payload", hash.as_str()]).map_err(crypto)?);
    }
    let event = EventBuilder::new(Kind::from(HTTP_AUTH), "")
        .tags(tags)
        .custom_created_at(Timestamp::from_secs(at))
        .finalize(keys)
        .map_err(crypto)?;
    let json = serde_json::to_string(&event)?;
    Ok(format!("Nostr {}", STANDARD.encode(json)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event_of(header: &str) -> Event {
        let json = STANDARD.decode(header.strip_prefix("Nostr ").unwrap()).unwrap();
        serde_json::from_slice(&json).unwrap()
    }

    fn tag<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
        event.tags.iter().find_map(|t| match t.as_slice() {
            [kind, value, ..] if kind == name => Some(value.as_str()),
            _ => None,
        })
    }

    #[test]
    fn the_event_says_what_the_server_checks() {
        let keys = Keys::generate();
        let body = br#"{"app_id":"net.veydan.mobile"}"#;
        let url = "https://vpush.veydan.net/v1/devices/abcdef0123456789";
        let event = event_of(&auth_header(&keys, "PUT", url, body, 1_790_000_000).unwrap());

        event.verify().unwrap();
        assert_eq!(event.pubkey, keys.public_key());
        assert_eq!(event.kind.as_u16(), 27235);
        assert_eq!(event.created_at.as_secs(), 1_790_000_000);
        assert_eq!(event.content, "");
        assert_eq!(tag(&event, "u"), Some(url));
        assert_eq!(tag(&event, "method"), Some("PUT"));
        assert_eq!(
            tag(&event, "payload"),
            Some(hex::encode(Sha256::digest(body)).as_str())
        );
    }

    #[test]
    fn a_request_without_a_body_names_none() {
        let keys = Keys::generate();
        let event = event_of(&auth_header(&keys, "DELETE", "https://p.example.org/x", b"", 1).unwrap());
        event.verify().unwrap();
        assert_eq!(tag(&event, "method"), Some("DELETE"));
        assert_eq!(tag(&event, "payload"), None);
    }

    #[test]
    fn two_requests_of_one_second_are_two_events() {
        let keys = Keys::generate();
        let a = event_of(&auth_header(&keys, "GET", "https://p.example.org/x", b"", 7).unwrap());
        let b = event_of(&auth_header(&keys, "GET", "https://p.example.org/x", b"", 7).unwrap());
        assert_ne!(a.id, b.id);
        assert_ne!(tag(&a, "nonce"), tag(&b, "nonce"));
    }
}
