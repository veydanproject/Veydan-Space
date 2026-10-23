// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Fixed vectors of the formats of `cs-v1`, `sd-v1` and `flat-v1`: a
//! change to a derivation or a layout breaks this test.
//! `KEYDIST_WRITE_VECTORS=1` writes `tests/vectors.json` anew — only when
//! a format change is meant, and with the filter
//! `formats_match_the_fixed_vectors` (the other test reads the file).
//!
//! The JSON is written by the code it checks; the literals of
//! `derivations_match_openssl` are not: they were computed with
//! `openssl kdf … HKDF` and `openssl dgst -sha256` alone (the commands are
//! beside each constant), so a derivation that drifts from the
//! specification cannot pass by rewriting the JSON.

use keydist::epoch::{
    open_block, open_chain, open_sd_block, seal_block, seal_chain, seal_sd_block, EpochEvent, Nonces,
};
use keydist::flat::secret_bytes;
use keydist::kdf::{chain_key, NodeKeys};
use keydist::sd::{self, SdLabels};
use keydist::{
    channel_key, Bundle, EpochKey, GenerationMaster, History, Key32, RootSecret, SchemeId, ScopeRef, Tree,
    Verified,
};
use serde_json::{json, Value};

fn root() -> RootSecret {
    RootSecret::from_bytes([0x11; 32])
}

fn scope() -> ScopeRef {
    ScopeRef { community: [0x22; 32], scope: [0x33; 32] }
}

/// The vectors of `sd-v1`, under the same root, scope, generation and
/// epoch key as those of `cs-v1` (the master is the one of `sd-v1`):
/// labels, the three outputs of the generator, a walk, a subset key,
/// `K_all`, a block of `S_{3,14}` and the bundle of leaf 9 at depth 4 (a
/// member of `S_{3,14}`).
fn compute_sd(scope: &ScopeRef, gen: u32, k: &EpochKey) -> Value {
    let master = GenerationMaster::derive(&root(), scope, SchemeId::SdV1, gen);
    let labels = SdLabels::new(&Key32(*master.as_bytes()), scope, gen);
    let l3 = labels.label(3);
    let key_3_14 = labels.subset_key(3, 14);
    let nonce = Nonces::new(&Key32([0x99; 32])).subset(3, 14);
    let block = seal_sd_block(&key_3_14, &nonce, scope, gen, k, 3, 14);
    let bundle = Bundle::issue_sd(&labels, *scope, gen, Tree::new(4).unwrap(), 9);
    json!({
        "inputs": { "nonce_seed": hex::encode([0x99u8; 32]), "subset": [3, 14], "bundle_depth": 4, "bundle_leaf": 9 },
        "generation_key": hex::encode(master.as_bytes()),
        "label_1": hex::encode(labels.label(1).0),
        "label_3": hex::encode(l3.0),
        "label_depth24_leaf_7": hex::encode(labels.label((1u64 << 24) + 7).0),
        "prg_l_of_label_3": hex::encode(sd::g_left(&l3).0),
        "prg_m_of_label_3": hex::encode(sd::g_middle(&l3).0),
        "prg_r_of_label_3": hex::encode(sd::g_right(&l3).0),
        "label_3_14": hex::encode(labels.label_of(3, 14).0),
        "subset_key_3_14": hex::encode(key_3_14.0),
        "k_all": hex::encode(labels.all_key().0),
        "nonce_3_14": hex::encode(nonce),
        "block_3_14": hex::encode(block),
        "bundle": hex::encode(&*bundle.to_bytes()),
    })
}

fn compute() -> Value {
    let scope = scope();
    let gen = 1u32;
    let master = GenerationMaster::derive(&root(), &scope, SchemeId::CsV1, gen);
    let nodes = NodeKeys::new(&Key32(*master.as_bytes()), &scope, gen);
    let tree = Tree::new(4).unwrap();
    let leaf_node = tree.leaf_node(9);
    let k = EpochKey { epoch: 3, key: Key32([0x44; 32]) };
    let prev = EpochKey { epoch: 2, key: Key32([0x45; 32]) };
    let nonce = Nonces::new(&Key32([0x99; 32])).node(5);
    let block = seal_block(&nodes.node(5), &nonce, &scope, gen, &k, 5);
    let chain = seal_chain(&k, &prev, &[0x66; 12], &scope);
    let bundle = Bundle::issue(&nodes, scope, gen, tree, 9);
    let mut v = json!({
        "inputs": {
            "root": hex::encode([0x11u8; 32]),
            "community": hex::encode(scope.community),
            "scope": hex::encode(scope.scope),
            "gen": gen,
            "epoch_key": hex::encode(k.key.0),
            "epoch": k.epoch,
            "prev_key": hex::encode(prev.key.0),
            "prev_epoch": prev.epoch,
            "nonce_seed": hex::encode([0x99u8; 32]),
            "chain_nonce": hex::encode([0x66u8; 12]),
            "channel_id": "general",
            "bundle_depth": 4,
            "bundle_leaf": 9
        },
        "generation_key": hex::encode(master.as_bytes()),
        "node_key_1": hex::encode(nodes.node(1).0),
        "node_key_5": hex::encode(nodes.node(5).0),
        "node_key_leaf_9": hex::encode(nodes.node(leaf_node).0),
        "node_key_depth24_leaf_7": hex::encode(nodes.node((1u64 << 24) + 7).0),
        "channel_key": hex::encode(channel_key(&k, &scope, b"general").0),
        "chain_key": hex::encode(chain_key(&k, &scope).0),
        "nonce_node_5": hex::encode(nonce),
        "block_node_5": hex::encode(block),
        "chain": hex::encode(&chain),
        "bundle": hex::encode(&*bundle.to_bytes()),
        "flat_secret": hex::encode(&*secret_bytes(&scope, &k)),
    });
    v.as_object_mut().expect("an object").insert("sd".into(), compute_sd(&scope, gen, &k));
    v
}

#[test]
fn formats_match_the_fixed_vectors() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/vectors.json");
    let now = compute();
    if std::env::var_os("KEYDIST_WRITE_VECTORS").is_some() {
        std::fs::write(path, serde_json::to_string_pretty(&now).unwrap() + "\n").unwrap();
    }
    let fixed: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(now, fixed, "a format of keydist changed; the vectors are frozen");

    // The fixed bytes open, and parse, with the fixed inputs.
    let h = |k: &str| hex::decode(fixed[k].as_str().unwrap()).unwrap();
    let scope = scope();
    let nk = Key32(h("node_key_5").try_into().unwrap());
    let block = h("block_node_5");
    assert_eq!(block[8..20], h("nonce_node_5")[..]);
    let k = open_block(&nk, &block, &scope, 1, 3).unwrap();
    assert_eq!(k.key.0, [0x44; 32]);
    let ev = Verified::after_signature_check(EpochEvent {
        v: 2,
        scheme: SchemeId::CsV1,
        community_id: scope.community,
        scope_id: scope.scope,
        epoch: 3,
        prev_epoch: Some(2),
        at: 0,
        history: History::Open,
        trees: vec![],
        chain: Some(h("chain")),
    });
    assert_eq!(open_chain(&ev, &k).unwrap().key.0, [0x45; 32]);
    let b = Bundle::from_bytes(&h("bundle")).unwrap();
    assert_eq!((b.gen, b.leaf, b.tree.depth(), b.keys.len()), (1, 9, 4, 5));
    assert_eq!(b.keys[4].0.to_vec(), h("node_key_leaf_9"));

    // sd-v1: the block opens with the fixed subset key, and the fixed
    // bundle of leaf 9 derives that key and holds K_all first.
    let s = |k: &str| hex::decode(fixed["sd"][k].as_str().unwrap()).unwrap();
    let sk = Key32(s("subset_key_3_14").try_into().unwrap());
    let block = s("block_3_14");
    assert_eq!(block.len(), keydist::SD_BLOCK_LEN);
    assert_eq!(block[16..28], s("nonce_3_14")[..]);
    assert_eq!(open_sd_block(&sk, &block, &scope, 1, 3).unwrap().key.0, [0x44; 32]);
    let b = Bundle::from_bytes(&s("bundle")).unwrap();
    assert_eq!((b.scheme, b.gen, b.leaf, b.tree.depth(), b.keys.len()), (SchemeId::SdV1, 1, 9, 4, 11));
    assert_eq!(b.subset_key(3, 14), Some(sk));
    assert_eq!(b.keys[0].0.to_vec(), s("k_all"));
    let l3 = Key32(s("label_3").try_into().unwrap());
    assert_eq!(sd::g_middle(&sd::descend(&l3, 3, 14)).0.to_vec(), s("subset_key_3_14"));
    assert_eq!(sd::g_left(&sd::g_right(&l3)).0.to_vec(), s("label_3_14"));
}

/// Constants computed without this crate. With `H(s)` the hex of an ASCII
/// string, `X×32` 32 bytes of `X`:
///
/// ```text
/// openssl kdf -keylen 32 -kdfopt digest:SHA256 -kdfopt hexkey:<11×32> \
///   -kdfopt hexinfo:<H(veydan.keydist.gen.v1)><22×32><33×32>02 01000000 HKDF   # cs-v1 master
///   … the same with scheme 03                                                    # sd-v1 master
/// openssl kdf … -kdfopt hexkey:<cs-v1 master> \
///   -kdfopt hexinfo:<H(veydan.keydist.cs.v1)><22×32><33×32>01000000 0500000000000000 HKDF  # node 5
/// openssl kdf … -kdfopt hexkey:<44×32> -kdfopt hexinfo:<H(veydan.keydist.chain.v1)><22×32><33×32> HKDF
/// printf <H(veydan.keydist.nonce.v1)><99×32>0500000000000000 (as bytes) | openssl dgst -sha256  # first 12 bytes
/// ```
#[test]
fn derivations_match_openssl() {
    const GEN_CS: &str = "da60b883d80fdfb85f39cf4b4e4e3a9980ee92b41ccd84ec3ff2909b2c609191";
    const GEN_SD: &str = "08ed4cef9b1d030470ea3a8ea1deca5b2953065f06bd62dd4742786f06dbd692";
    const NODE_5: &str = "e8c84d5c128787d47e9eff65c96383a6d0bbb4400f0060eb1654bec6ab7397d2";
    const CHAIN_KEY: &str = "d84ecc0eee20752ad49ff3e1b28ba7f373395387c080eba7c1dc4d44c27375cb";
    const NONCE_5: &str = "22bf16c89f19e2a5d9c049af";
    let scope = scope();
    let cs = GenerationMaster::derive(&root(), &scope, SchemeId::CsV1, 1);
    let sd = GenerationMaster::derive(&root(), &scope, SchemeId::SdV1, 1);
    assert_eq!(hex::encode(cs.as_bytes()), GEN_CS);
    assert_eq!(hex::encode(sd.as_bytes()), GEN_SD);
    let node5 = NodeKeys::new(&Key32(*cs.as_bytes()), &scope, 1).node(5);
    assert_eq!(hex::encode(node5.0), NODE_5);
    let k = EpochKey { epoch: 3, key: Key32([0x44; 32]) };
    assert_eq!(hex::encode(chain_key(&k, &scope).0), CHAIN_KEY);
    assert_eq!(hex::encode(Nonces::new(&Key32([0x99; 32])).node(5)), NONCE_5);
    // And the JSON agrees with them.
    let fixed: Value =
        serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/vectors.json")).unwrap())
            .unwrap();
    assert_eq!(fixed["generation_key"], GEN_CS);
    assert_eq!(fixed["sd"]["generation_key"], GEN_SD);
    assert_eq!(fixed["node_key_5"], NODE_5);
    assert_eq!(fixed["chain_key"], CHAIN_KEY);
    assert_eq!(fixed["nonce_node_5"], NONCE_5);
}
