// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The sealed push of the server's own examples opens here
//! (`crates/messenger/push/tests/golden`, the copy of `services/push/golden`
//! that `scripts/vpush-contract.sh` keeps): with the keys its README names,
//! `push_sealed.json` is the push of `push_group.json`, word for word.

use base64::Engine;
use messenger_notify::sealed::{open, Seal, Unsealed};
use messenger_notify::PushData;
use nostr::key::{Keys, PublicKey};
use nostr::nips::nip44;
use std::collections::BTreeMap;

fn golden(name: &str) -> serde_json::Value {
    let path = format!("{}/../push/tests/golden/{name}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap()
}

/// A push as FCM hands it over: every value a string.
fn data_map(v: &serde_json::Value) -> BTreeMap<String, String> {
    v.as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().map(String::from).unwrap_or_else(|| v.to_string())))
        .collect()
}

/// The push key of the device of the examples, and the seal of the
/// private push server: the secrets `1111…11` and `2222…22`.
fn keys() -> (Keys, Keys) {
    (Keys::parse(&"11".repeat(32)).unwrap(), Keys::parse(&"22".repeat(32)).unwrap())
}

#[test]
fn the_sealed_example_opens_to_the_push_of_a_group() {
    let (device, server) = keys();
    let sealed = data_map(&golden("push_sealed.json"));
    assert_eq!(sealed["sender"], server.public_key().to_hex(), "the README's keys are the example's");
    let inner = data_map(&golden("push_group.json"));

    // Byte for byte: the JSON the private server sealed is the data map of
    // the push of the group, the keys in their order.
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&sealed["blob"]).unwrap();
    let sender = PublicKey::from_hex(&sealed["sender"]).unwrap();
    let key = nip44::v2::ConversationKey::derive(device.secret_key(), &sender).unwrap();
    let plain = nip44::v2::decrypt_to_bytes(&key, &bytes).unwrap();
    assert_eq!(String::from_utf8(plain).unwrap(), serde_json::to_string(&inner).unwrap());

    // And the way the handler opens it, from a server of mine whose relay
    // is the one the push names.
    let senders = vec![Seal { sender: sealed["sender"].clone(), relay: inner["relay"].clone() }];
    let Unsealed::Open { data } = open(&sealed, &device, &senders) else {
        panic!("the example did not open");
    };
    assert_eq!(data, inner);
    let push = PushData::parse(&data).unwrap();
    assert_eq!(push.group_id.as_deref(), Some("11".repeat(32).as_str()));
    assert_eq!(push.count, 3);
    assert_eq!(push.trace, "9583cb58");

    // The same blob is no push of a server this device did not register at.
    assert!(matches!(open(&sealed, &device, &[]), Unsealed::Dropped { .. }));
    // Nor of another device.
    assert!(matches!(open(&sealed, &server, &senders), Unsealed::Dropped { .. }));

    // Of a server whose relay is another: the relay it names is not asked,
    // nor the event it names; the push wakes for that server alone.
    let elsewhere = vec![Seal { sender: sealed["sender"].clone(), relay: "wss://relay.s1.example".into() }];
    let Unsealed::Open { data } = open(&sealed, &device, &elsewhere) else {
        panic!("the example did not open");
    };
    let mut bare = inner.clone();
    bare.remove("relay");
    bare.remove("event_id");
    assert_eq!(data, bare);
    let push = PushData::parse(&data).unwrap();
    assert_eq!((push.relay, push.event_id), (None, None));
}

/// The keys of the examples seal a broadcast, a word of the Veydan network
/// the phone shows under the network's name: a private server has no such
/// word, and the handler drops it.
#[test]
fn a_broadcast_in_a_seal_is_dropped() {
    let (device, server) = keys();
    let broadcast: BTreeMap<String, String> = [
        ("v", "2"),
        ("type", "broadcast"),
        ("data", "{\"title\":\"Veydan\",\"text\":\"Your account is locked: sign in at https://veydan.example\"}"),
        ("trace", "9583cb58"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let text = nip44::encrypt(server.secret_key(), &device.public_key(), serde_json::to_string(&broadcast).unwrap(), nip44::Version::V2).unwrap();
    let bytes = base64::engine::general_purpose::STANDARD.decode(text).unwrap();
    let mut sealed = data_map(&golden("push_sealed.json"));
    sealed.insert("blob".into(), base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes));
    let senders = vec![Seal { sender: server.public_key().to_hex(), relay: "wss://node-1.veydan.net".into() }];
    assert_eq!(
        open(&sealed, &device, &senders),
        Unsealed::Dropped { reason: "a type a private server has no word in".into() }
    );
}

#[test]
fn the_wake_carries_the_blob_the_push_does() {
    assert_eq!(golden("wake.json")["blob"], golden("push_sealed.json")["blob"]);
}
