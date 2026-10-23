// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The test vectors of internal/communities-wire.md §1 (`vectors.json`,
//! computed by independent tools): what another implementation of the
//! wire checks itself against.

use communities_core::link::LinkSecret;
use communities_core::op::Op;
use communities_core::snapshot::merkle_root;
use communities_core::wire;
use keydist::{EpochKey, Key32};
use messenger_core::PubKey;
use serde_json::Value;
use sha2::{Digest, Sha256};

fn vectors() -> Value {
    serde_json::from_str(include_str!("vectors.json")).expect("vectors.json is JSON")
}

fn s<'a>(v: &'a Value, path: &[&str]) -> &'a str {
    path.iter().fold(v, |v, k| &v[*k]).as_str().unwrap_or_else(|| panic!("{path:?}"))
}

fn bytes32(hex_str: &str) -> [u8; 32] {
    hex::decode(hex_str).unwrap().try_into().unwrap()
}

#[test]
fn scope_bytes() {
    let v = vectors();
    assert_eq!(hex::encode(wire::scope_bytes(s(&v, &["scope", "scope_id"]))), s(&v, &["scope", "bytes"]));
    let scope = wire::scope_ref(s(&v, &["community_id"]), "everyone").unwrap();
    assert_eq!(hex::encode(scope.scope), s(&v, &["scope", "bytes"]));
}

#[test]
fn channel_key_and_key_id() {
    let v = vectors();
    let k = EpochKey { epoch: v["channel_key"]["epoch"].as_u64().unwrap() as u32, key: Key32(bytes32(s(&v, &["channel_key", "epoch_key"]))) };
    let scope = wire::scope_ref(s(&v, &["community_id"]), "everyone").unwrap();
    let key = wire::stream_key(&k, &scope, s(&v, &["channel_key", "channel_id"]));
    assert_eq!(hex::encode(key.0), s(&v, &["channel_key", "key"]));
    assert_eq!(wire::key_id(&k).0, s(&v, &["channel_key", "key_id"]));
}

#[test]
fn canonical_op_and_its_id() {
    let v = vectors();
    let canonical = s(&v, &["op", "canonical"]);
    let op: Op = serde_json::from_str(canonical).unwrap();
    assert_eq!(op.canonical(), canonical, "read and written back byte for byte");
    assert_eq!(op.id().0, s(&v, &["op", "id"]));
    assert_eq!(hex::encode(Sha256::digest(canonical.as_bytes())), s(&v, &["op", "id"]));
}

#[test]
fn join_mac() {
    let v = vectors();
    let secret = LinkSecret::from_bytes(&bytes32(s(&v, &["join_mac", "secret"]))).unwrap();
    let who = PubKey::parse(s(&v, &["join_mac", "pubkey"])).unwrap();
    let epoch = v["join_mac"]["link_epoch"].as_u64().unwrap() as u32;
    assert_eq!(secret.join_mac(s(&v, &["community_id"]), &who, epoch), s(&v, &["join_mac", "mac"]));
}

#[test]
fn merkle_root_of_three() {
    let v = vectors();
    let leaves: Vec<[u8; 32]> = [1u8, 2, 3].iter().map(|b| Sha256::digest([*b]).into()).collect();
    assert_eq!(hex::encode(merkle_root(&leaves)), s(&v, &["merkle", "root"]));
}
