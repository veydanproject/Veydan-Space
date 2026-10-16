// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The NIP-44 v2 of this crate against the official vectors
//! (github.com/paulmillr/nip44, `nip44.vectors.json`, kept here as it was
//! fetched). The derivation of the message keys and the opening of a
//! payload are our own code; the conversation key and the encryption are
//! nostr's, checked here because a proof is only as right as both halves.

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use messenger_evidence::nip44::{calc_padded_len, message_keys, Payload};
use nostr::key::{PublicKey, SecretKey};
use nostr::nips::nip44::v2::{encrypt_to_bytes_with_nonce, ConversationKey};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn vectors() -> Value {
    serde_json::from_str(include_str!("nip44.vectors.json")).expect("the vectors are JSON")
}

fn bytes32(v: &Value) -> [u8; 32] {
    let mut out = [0u8; 32];
    hex::decode_to_slice(v.as_str().expect("hex string"), &mut out).expect("32 bytes of hex");
    out
}

fn list<'a>(v: &'a Value, path: &[&str]) -> &'a Vec<Value> {
    let mut at = v;
    for p in path {
        at = &at[*p];
    }
    at.as_array()
        .unwrap_or_else(|| panic!("{path:?} is a list"))
}

#[test]
fn get_message_keys() {
    let v = vectors();
    let section = &v["v2"]["valid"]["get_message_keys"];
    let conversation = bytes32(&section["conversation_key"]);
    let cases = list(section, &["keys"]);
    assert!(!cases.is_empty());
    for case in cases {
        let keys = message_keys(&conversation, &bytes32(&case["nonce"]));
        assert_eq!(
            hex::encode(keys.chacha_key),
            case["chacha_key"].as_str().unwrap()
        );
        assert_eq!(
            hex::encode(keys.chacha_nonce),
            case["chacha_nonce"].as_str().unwrap()
        );
        assert_eq!(
            hex::encode(keys.hmac_key),
            case["hmac_key"].as_str().unwrap()
        );
    }
}

#[test]
fn calc_padded_len_vectors() {
    let v = vectors();
    for pair in list(&v, &["v2", "valid", "calc_padded_len"]) {
        let len = pair[0].as_u64().unwrap() as usize;
        let padded = pair[1].as_u64().unwrap() as usize;
        assert_eq!(calc_padded_len(len), padded, "padded length of {len}");
    }
}

#[test]
fn get_conversation_key() {
    let v = vectors();
    for case in list(&v, &["v2", "valid", "get_conversation_key"]) {
        let secret = SecretKey::from_hex(case["sec1"].as_str().unwrap()).unwrap();
        let public = PublicKey::from_hex(case["pub2"].as_str().unwrap()).unwrap();
        let key = ConversationKey::derive(&secret, &public).unwrap();
        assert_eq!(
            hex::encode(key.as_bytes()),
            case["conversation_key"].as_str().unwrap()
        );
    }
    for case in list(&v, &["v2", "invalid", "get_conversation_key"]) {
        let derived = SecretKey::from_hex(case["sec1"].as_str().unwrap())
            .ok()
            .zip(PublicKey::from_hex(case["pub2"].as_str().unwrap()).ok())
            .and_then(|(s, p)| ConversationKey::derive(&s, &p).ok());
        assert!(derived.is_none(), "{}", case["note"]);
    }
}

/// Our opening reads what nostr's encryption writes, byte for byte as the
/// NIP's vectors have it.
#[test]
fn encrypt_decrypt() {
    let v = vectors();
    for case in list(&v, &["v2", "valid", "encrypt_decrypt"]) {
        let conversation = bytes32(&case["conversation_key"]);
        let nonce = bytes32(&case["nonce"]);
        let plaintext = case["plaintext"].as_str().unwrap();
        let encoded = case["payload"].as_str().unwrap();

        let written = encrypt_to_bytes_with_nonce(
            &ConversationKey::new(conversation),
            plaintext.as_bytes(),
            nonce,
        )
        .unwrap();
        assert_eq!(B64.encode(&written), encoded, "nostr writes the vector");

        let payload = Payload::parse(encoded).unwrap();
        assert_eq!(payload.nonce, nonce);
        let opened = payload
            .open(&message_keys(&conversation, &payload.nonce))
            .unwrap();
        assert_eq!(opened, plaintext.as_bytes());
    }
}

#[test]
fn encrypt_decrypt_long_msg() {
    let v = vectors();
    for case in list(&v, &["v2", "valid", "encrypt_decrypt_long_msg"]) {
        let conversation = bytes32(&case["conversation_key"]);
        let nonce = bytes32(&case["nonce"]);
        let plaintext = case["pattern"]
            .as_str()
            .unwrap()
            .repeat(case["repeat"].as_u64().unwrap() as usize);
        assert_eq!(
            hex::encode(Sha256::digest(plaintext.as_bytes())),
            case["plaintext_sha256"].as_str().unwrap()
        );

        let written = encrypt_to_bytes_with_nonce(
            &ConversationKey::new(conversation),
            plaintext.as_bytes(),
            nonce,
        )
        .unwrap();
        let encoded = B64.encode(&written);
        assert_eq!(
            hex::encode(Sha256::digest(encoded.as_bytes())),
            case["payload_sha256"].as_str().unwrap()
        );

        let payload = Payload::parse(&encoded).unwrap();
        assert_eq!(
            payload.open(&message_keys(&conversation, &nonce)).unwrap(),
            plaintext.as_bytes()
        );
    }
}

#[test]
fn invalid_payloads_do_not_open() {
    let v = vectors();
    let cases = list(&v, &["v2", "invalid", "decrypt"]);
    assert!(!cases.is_empty());
    for case in cases {
        let conversation = bytes32(&case["conversation_key"]);
        let opened = Payload::parse(case["payload"].as_str().unwrap())
            .and_then(|p| p.open(&message_keys(&conversation, &p.nonce)));
        assert!(opened.is_err(), "{} opened", case["note"]);
    }
}

/// A plaintext past the NIP's 65 535 bytes, in the wide prefix nostr 0.45
/// writes for it: a seal of a long rumor from our own clients.
#[test]
fn wide_prefix_opens() {
    let conversation = [7u8; 32];
    let nonce = [9u8; 32];
    for len in [65_535, 65_536, 70_000] {
        let plaintext = vec![b'y'; len];
        let written =
            encrypt_to_bytes_with_nonce(&ConversationKey::new(conversation), &plaintext, nonce)
                .unwrap();
        let payload = Payload::from_bytes(&written).unwrap();
        assert_eq!(
            payload.open(&message_keys(&conversation, &nonce)).unwrap(),
            plaintext,
            "length {len}"
        );
    }
}
