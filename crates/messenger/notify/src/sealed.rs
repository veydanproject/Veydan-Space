// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A push of a private server, sealed to the push key of the device and
//! carried by the gateway of the Veydan network (`type: sealed`;
//! services/push/spec/protocol.md, «The gateway»; internal/messenger-wire.md,
//! «Push через шлюз»).
//!
//! The private push server builds the push as it would for FCM, the data
//! map with every value a string, seals its JSON with NIP-44 v2 from its own
//! key (`sender`) to the push key of the device, and hands the bytes of the
//! payload, base64url without padding, to the gateway as `blob`. Opened,
//! the map is a push like any other and goes the same way.
//!
//! What is dropped: a `sender` that is not the seal of a private server
//! this device registered at (checked first, keys or no keys), a blob that
//! does not open, an inside that is no push of version 2, is sealed again,
//! or is of a type a private server has no word in (`INNER_TYPES`: a
//! message, a group's, a burst, a test; never a word of the Veydan network
//! such as `broadcast`, which the phone would show as the network's). The
//! question of a registration (a `test` that carries `check`) shows
//! nothing. A `relay` inside that is not the relay of the server whose seal
//! opened it is not followed: the push is a bare wake of that server.

use crate::bundle::KeyBundle;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use messenger_core::{MessengerConfig, Result};
use messenger_store::{servers, Store};
use nostr::key::{Keys, PublicKey};
use nostr::nips::nip44;
use nostr::nips::nip44::v2::ConversationKey;
use serde::Serialize;
use std::collections::BTreeMap;

pub const TYPE_SEALED: &str = "sealed";
/// What a gateway carries at most is 3600 characters of `blob`; a little
/// more is read, nothing near what would cost the handler.
const BLOB_MAX: usize = 4096;
/// The types a private server may put inside a seal: what its push server
/// says of its own relay. Anything else (`broadcast`, `manifest_update`, a
/// word of a newer server) would be shown or handled as the Veydan
/// network's, and is dropped.
pub const INNER_TYPES: &[&str] = &["dm", "group", "sync", "test"];

/// A private server whose seal this device knows: the key its push server
/// seals with (`push_sender`) and its relay, the one relay a push of it may
/// send the handler to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seal {
    pub sender: String,
    pub relay: String,
}

/// Whether the data map of a push is a sealed one.
pub fn is_sealed(data: &BTreeMap<String, String>) -> bool {
    data.get("type").map(String::as_str) == Some(TYPE_SEALED)
}

/// What a sealed push turned out to be.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Unsealed {
    /// The push of the private server: its data map, as FCM would have
    /// carried it, for the same way as every other push.
    Open { data: BTreeMap<String, String> },
    /// The question of a registration (`test` with `check`): nothing is
    /// shown.
    Check,
    /// Sealed by a server of mine, and no push key here to open it: a PIN
    /// guards the app and the handler holds no keys, or the bundle came
    /// before the device had a push key.
    NoKeys,
    /// Not of a server of mine, or not a push: dropped, nothing shown.
    Dropped { reason: String },
}

fn dropped(reason: &str) -> Unsealed {
    Unsealed::Dropped { reason: reason.to_string() }
}

fn hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// The outside of a sealed push, checked without any key: its version, a
/// blob of a size a gateway carries, and a `sender` that is the seal of a
/// server of mine. The server, and the blob.
fn outside<'a>(data: &'a BTreeMap<String, String>, seals: &'a [Seal]) -> std::result::Result<(&'a Seal, &'a str), Unsealed> {
    if !is_sealed(data) {
        return Err(dropped("not a sealed push"));
    }
    if data.get("v").map(String::as_str) != Some("2") {
        return Err(dropped("a version this code does not know"));
    }
    let (Some(blob), Some(sender)) = (data.get("blob"), data.get("sender")) else {
        return Err(dropped("no blob or no sender"));
    };
    if !hex64(sender) {
        return Err(dropped("the sender is not a key"));
    }
    let Some(seal) = seals.iter().find(|s| s.sender == *sender) else {
        return Err(dropped("not of a private server of mine"));
    };
    if blob.is_empty() || blob.len() > BLOB_MAX {
        return Err(dropped("the blob is of no size a gateway carries"));
    }
    Ok((seal, blob))
}

/// Opens a sealed push with the push key of the device, when its `sender`
/// is the seal of one of `seals` (the private servers this device
/// registered at).
pub fn open(data: &BTreeMap<String, String>, push_key: &Keys, seals: &[Seal]) -> Unsealed {
    let (seal, blob) = match outside(data, seals) {
        Ok(found) => found,
        Err(out) => return out,
    };
    let Ok(bytes) = URL_SAFE_NO_PAD.decode(blob) else {
        return dropped("the blob is not base64url");
    };
    let Ok(from) = PublicKey::from_hex(&seal.sender) else {
        return dropped("the sender is not a key");
    };
    let Ok(key) = ConversationKey::derive(push_key.secret_key(), &from) else {
        return dropped("no conversation with the sender");
    };
    let Ok(plain) = nip44::v2::decrypt_to_bytes(&key, &bytes) else {
        return dropped("the blob does not open");
    };
    let Ok(mut inner) = serde_json::from_slice::<BTreeMap<String, String>>(&plain) else {
        return dropped("the inside is no data map of a push");
    };
    if inner.get("v").map(String::as_str) != Some("2") {
        return dropped("the inside is of a version this code does not know");
    }
    match inner.get("type").map(String::as_str) {
        None => return dropped("the inside has no type"),
        Some(TYPE_SEALED) => return dropped("a seal in a seal"),
        Some("test") if inner.get("check").map(String::as_str) == Some("1") => return Unsealed::Check,
        Some(t) if !INNER_TYPES.contains(&t) => return dropped("a type a private server has no word in"),
        Some(_) => {}
    }
    // The handler fetches what the push names from the relay it names: of
    // this server alone. Another relay is not asked, and the event the
    // push says came from there is not taken either.
    let foreign = inner
        .get("relay")
        .is_some_and(|relay| servers::normalize(relay) != servers::normalize(&seal.relay));
    if foreign {
        for field in ["relay", "event_id", "event"] {
            inner.remove(field);
        }
    }
    Unsealed::Open { data: inner }
}

/// The seals this device knows: every private server whose push is on and
/// whose push server named its key, with its relay. Readable without keys.
pub async fn seals(store: &Store) -> Result<Vec<Seal>> {
    let mut out = Vec::new();
    for row in servers::private(store).await? {
        let Some(push) = servers::push_of(store, &row.id).await? else { continue };
        if push.push_mode != servers::PUSH_VEYDAN {
            continue;
        }
        if let Some(sender) = push.push_sender {
            out.push(Seal { sender, relay: row.relay_url });
        }
    }
    Ok(out)
}

/// [`open`] with the push key of the bundle and the seals the database of
/// `config` knows, read-only, as the push handler has them. The sender is
/// checked before the keys: a push that is no server's of mine is dropped
/// on a phone without keys too, and is not shown as «something came».
pub async fn unseal_as(
    config: &MessengerConfig,
    bundle: Option<&KeyBundle>,
    data: &BTreeMap<String, String>,
) -> Result<Unsealed> {
    let store = Store::open_read_only(config).await?;
    let known = seals(&store).await;
    store.close().await;
    let known = known?;
    if let Err(out) = outside(data, &known) {
        return Ok(out);
    }
    let Some(push_key) = bundle.and_then(KeyBundle::push_keys) else {
        return Ok(Unsealed::NoKeys);
    };
    Ok(open(data, &push_key, &known))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELAY: &str = "wss://relay.s1.example";

    fn seal(from: &Keys, to: &Keys, inner: &BTreeMap<String, String>) -> BTreeMap<String, String> {
        let json = serde_json::to_string(inner).unwrap();
        // The payload in its usual base64 text, then its bytes, as a
        // private push server hands them to the gateway.
        let text = nip44::encrypt(from.secret_key(), &to.public_key(), json, nip44::Version::V2).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD.decode(text).unwrap();
        map(&[
            ("v", "2"),
            ("type", "sealed"),
            ("blob", &URL_SAFE_NO_PAD.encode(bytes)),
            ("sender", &from.public_key().to_hex()),
            ("trace", "b21c07d4"),
        ])
    }

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn mine(server: &Keys) -> Vec<Seal> {
        vec![Seal { sender: server.public_key().to_hex(), relay: RELAY.into() }]
    }

    #[test]
    fn a_sealed_push_of_my_server_opens_to_its_map() {
        let (server, device) = (Keys::generate(), Keys::generate());
        let inner = map(&[("v", "2"), ("type", "dm"), ("event_id", &"ab".repeat(32)), ("relay", "wss://Relay.s1.example/"), ("trace", "x")]);
        let data = seal(&server, &device, &inner);
        assert_eq!(open(&data, &device, &mine(&server)), Unsealed::Open { data: inner });
    }

    #[test]
    fn the_question_of_a_registration_shows_nothing() {
        let (server, device) = (Keys::generate(), Keys::generate());
        let check = seal(&server, &device, &map(&[("v", "2"), ("type", "test"), ("check", "1"), ("trace", "x")]));
        assert_eq!(open(&check, &device, &mine(&server)), Unsealed::Check);
        // A test the owner asked for is shown, as at home.
        let test = map(&[("v", "2"), ("type", "test"), ("trace", "x")]);
        assert_eq!(open(&seal(&server, &device, &test), &device, &mine(&server)), Unsealed::Open { data: test });
    }

    /// A private server speaks of its relay alone: a word of the network
    /// (a broadcast, a manifest update) in its seal would be shown or
    /// handled as the network's own.
    #[test]
    fn only_the_types_of_a_relay_open() {
        let (server, device) = (Keys::generate(), Keys::generate());
        for kind in INNER_TYPES {
            let inner = map(&[("v", "2"), ("type", kind), ("group_id", &"11".repeat(32)), ("trace", "x")]);
            assert!(matches!(open(&seal(&server, &device, &inner), &device, &mine(&server)), Unsealed::Open { .. }), "{kind}");
        }
        for kind in ["broadcast", "manifest_update", "update", "call_x"] {
            let inner = map(&[("v", "2"), ("type", kind), ("data", "{\"text\":\"Veydan: confirm your account\"}"), ("trace", "x")]);
            assert_eq!(
                open(&seal(&server, &device, &inner), &device, &mine(&server)),
                dropped("a type a private server has no word in"),
                "{kind}"
            );
        }
    }

    /// The relay a push sends the handler to is its server's: another one
    /// (anybody's, or one that would see the phone ask) is not asked, and
    /// the push wakes for its server without naming an event.
    #[test]
    fn a_relay_that_is_not_its_servers_is_not_followed() {
        let (server, device) = (Keys::generate(), Keys::generate());
        let g = "11".repeat(32);
        let inner = map(&[
            ("v", "2"),
            ("type", "group"),
            ("group_id", &g),
            ("event_id", &"44".repeat(32)),
            ("event", "{}"),
            ("relay", "wss://watcher.example"),
            ("trace", "x"),
        ]);
        let Unsealed::Open { data } = open(&seal(&server, &device, &inner), &device, &mine(&server)) else { panic!("not opened") };
        assert_eq!(data, map(&[("v", "2"), ("type", "group"), ("group_id", &g), ("trace", "x")]));
        // Without a relay, nothing is cut.
        let bare = map(&[("v", "2"), ("type", "dm"), ("event", "{}"), ("trace", "x")]);
        assert_eq!(open(&seal(&server, &device, &bare), &device, &mine(&server)), Unsealed::Open { data: bare });
    }

    #[test]
    fn what_is_not_of_my_server_or_does_not_open_is_dropped() {
        let (server, device, stranger) = (Keys::generate(), Keys::generate(), Keys::generate());
        let inner = map(&[("v", "2"), ("type", "dm"), ("trace", "x")]);
        let senders = mine(&server);
        let is_dropped = |u: Unsealed| matches!(u, Unsealed::Dropped { .. });

        // Sealed by a key that is no seal of mine.
        assert!(is_dropped(open(&seal(&stranger, &device, &inner), &device, &senders)));
        // Sealed to another device.
        assert!(is_dropped(open(&seal(&server, &stranger, &inner), &device, &senders)));
        // A sender that says it is my server, over a blob that is not its.
        let mut forged = seal(&stranger, &device, &inner);
        forged.insert("sender".into(), server.public_key().to_hex());
        assert!(is_dropped(open(&forged, &device, &senders)));
        // A blob cut short, a blob in padded base64.
        let mut cut = seal(&server, &device, &inner);
        let blob = cut["blob"].clone();
        cut.insert("blob".into(), blob[..blob.len() - 4].to_string());
        assert!(is_dropped(open(&cut, &device, &senders)));
        // A seal in a seal, an inside of another version, no inside type.
        let again = seal(&server, &device, &map(&[("v", "2"), ("type", "sealed"), ("blob", "x"), ("sender", "y")]));
        assert!(is_dropped(open(&again, &device, &senders)));
        assert!(is_dropped(open(&seal(&server, &device, &map(&[("v", "3"), ("type", "dm")])), &device, &senders)));
        assert!(is_dropped(open(&seal(&server, &device, &map(&[("v", "2")])), &device, &senders)));
        // Not sealed at all.
        assert!(is_dropped(open(&inner, &device, &senders)));
    }

    /// Without keys (a PIN on the app) a push is shown as «something came»
    /// only when a server of mine sealed it: a wake by a stranger, or by a
    /// server this device does not register at, is dropped before.
    #[tokio::test]
    async fn without_keys_the_sender_is_checked_first() {
        let dir = tempfile::tempdir().unwrap();
        let config = MessengerConfig::new(dir.path().join("messenger"));
        let store = Store::open(&config).await.unwrap();
        let (server, device, stranger) = (Keys::generate(), Keys::generate(), Keys::generate());
        let row = servers::ServerRow {
            id: String::new(),
            kind: String::new(),
            name: Some("S1".into()),
            relay_url: RELAY.into(),
            relay_key: None,
            media_url: None,
            call_node: None,
            push_mode: servers::PUSH_VEYDAN.into(),
            source: None,
            added_at: 1_760_000_000,
        };
        let id = servers::put_private(&store, &row).await.unwrap();
        servers::set_push(&store, &id, Some("https://push.s1.example")).await.unwrap();
        servers::set_push_seal(&store, &id, Some("https://vpush.example"), Some(&server.public_key().to_hex())).await.unwrap();
        store.close().await;

        let inner = map(&[("v", "2"), ("type", "dm"), ("trace", "x")]);
        let from_mine = seal(&server, &device, &inner);
        let from_stranger = seal(&stranger, &device, &inner);
        assert_eq!(unseal_as(&config, None, &from_mine).await.unwrap(), Unsealed::NoKeys);
        assert_eq!(unseal_as(&config, None, &from_stranger).await.unwrap(), dropped("not of a private server of mine"));
        assert_eq!(unseal_as(&config, None, &inner).await.unwrap(), dropped("not a sealed push"));
    }
}
