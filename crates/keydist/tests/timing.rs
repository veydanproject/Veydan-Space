// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A sanity check of the cost at scale: depth 24, a million leaves handed
//! out, 200 000 of them revoked at random. The cover alone runs in every
//! test run (a loose bound that holds in a debug build); the whole epoch
//! (node keys, nonces, blocks, shards, hashes, the index or the proofs)
//! and the keeper's state at a million members are `#[ignore]`d:
//! `cargo test --release -p keydist --test timing -- --ignored --nocapture`.
//! The same for `sd-v1` (labels, the walk down, blocks of 76 bytes,
//! subsets copied into every shard they meet), on the same generation.

use std::time::Instant;

use keydist::epoch::{self, EpochRecipe, TreeRecipe};
use keydist::generation::{GenState, GenerationKind};
use keydist::{
    Bundle, CsKeeper, EpochKey, GenerationMaster, History, Key32, KeeperState, KeyDistribution, MemberId, Policy,
    Revoke, SchemeId, ScopeRef, SecretBytes, Tree, TreeParams,
};

/// A seeded xorshift64* (the crate's own exists only under `tools`).
struct TestRng(u64);

impl TestRng {
    fn below(&mut self, n: u64) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d) % n
    }
}

fn big_generation() -> GenState {
    let mut g =
        GenState::new(1, Tree::new(24).unwrap(), 4096, GenerationMaster::from_bytes([0x4d; 32]), GenerationKind::Initial, 0);
    g.next_leaf = 1_000_000;
    let mut rng = TestRng(24);
    while g.revoked.len() < 200_000 {
        g.revoked.insert(rng.below(1_000_000) as u32);
    }
    g
}

fn recipe(scheme: SchemeId, tr: TreeRecipe) -> EpochRecipe {
    EpochRecipe {
        scheme,
        key: EpochKey { epoch: 1, key: Key32([9; 32]) },
        seed: Key32([7; 32]),
        prev_epoch: None,
        at: 0,
        history: History::Open,
        chain: None,
        trees: vec![tr],
        recipients: Vec::new(),
        retires: Vec::new(),
    }
}

#[test]
fn cover_of_200k_revoked_at_depth_24_is_fast() {
    let g = big_generation();
    let t = Instant::now();
    let cover = g.cover();
    let took = t.elapsed();
    eprintln!("cover: {} nodes in {took:?}", cover.len());
    // r · log2(N / r) is about 465 000 here, plus the tail.
    assert!(cover.len() > 300_000 && cover.len() < 700_000, "{}", cover.len());
    assert!(took.as_secs_f64() < 10.0, "cover took {took:?}");
}

#[test]
#[ignore]
fn whole_epoch_of_200k_revoked_at_depth_24() {
    let scope = ScopeRef { community: [1; 32], scope: [2; 32] };
    let g = big_generation();
    let t = Instant::now();
    let cover = g.cover();
    let t_cover = t.elapsed();
    let t = Instant::now();
    let tr = TreeRecipe::new(1, g.tree, 4096, u64::from(g.next_leaf), &cover);
    let recipe = recipe(SchemeId::CsV1, tr);
    let nodes = g.node_keys(&scope);
    let out = epoch::build(&scope, &recipe, &|_| Some(nodes.clone()), &|_| SecretBytes::default()).unwrap();
    let t_build = t.elapsed();
    let mut sizes = out.shard_sizes();
    sizes.sort_unstable();
    let p = |q: usize| sizes[(sizes.len() * q).div_ceil(100) - 1];
    let event = serde_json::to_vec(&out.event).unwrap().len();
    let recipe_json = serde_json::to_vec(&recipe).unwrap().len();
    eprintln!(
        "cover {} nodes in {t_cover:?}; build (keys, nonces, AES-GCM, {} shards, hashes, index) in {t_build:?}; upload {} bytes; shard p50 {} p95 {} max {}; event {event} B; recipe {recipe_json} B of JSON",
        cover.len(),
        sizes.len(),
        out.upload_bytes(),
        p(50),
        p(95),
        sizes[sizes.len() - 1],
    );
    assert!(event < 1024);
}

#[test]
fn sd_cover_of_200k_revoked_at_depth_24_is_fast() {
    let g = big_generation();
    let t = Instant::now();
    let cover = g.sd_cover();
    let took = t.elapsed();
    eprintln!("sd cover: {} subsets in {took:?}", cover.len());
    // At most 2r − 1 per subtree of the assigned range (and one per
    // subtree without a revoked leaf).
    assert!(cover.len() < 2 * 200_000 + 2 * 24, "{}", cover.len());
    assert!(cover.len() > 100_000, "{}", cover.len());
    assert!(took.as_secs_f64() < 10.0, "sd cover took {took:?}");
}

#[test]
#[ignore]
fn sd_whole_epoch_of_200k_revoked_at_depth_24() {
    let scope = ScopeRef { community: [1; 32], scope: [2; 32] };
    let g = big_generation();
    let t = Instant::now();
    let cover = g.sd_cover();
    let t_cover = t.elapsed();
    let t = Instant::now();
    let tr = TreeRecipe::new_sd(1, g.tree, 4096, u64::from(g.next_leaf), &cover);
    let recipe = recipe(SchemeId::SdV1, tr);
    let nodes = g.node_keys(&scope);
    let out = epoch::build(&scope, &recipe, &|_| Some(nodes.clone()), &|_| SecretBytes::default()).unwrap();
    let t_build = t.elapsed();
    let mut sizes = out.shard_sizes();
    sizes.sort_unstable();
    let p = |q: usize| sizes[(sizes.len() * q).div_ceil(100) - 1];
    // A bundle, and members opening the epoch from their shard.
    let labels = g.sd_labels(&scope);
    let t = Instant::now();
    let bundles: Vec<Bundle> = (0..200u32).map(|i| Bundle::issue_sd(&labels, scope, 1, g.tree, i * 4999)).collect();
    let t_bundle = t.elapsed() / 200;
    let bundle_len = bundles[0].to_bytes().len();
    let (mut opened, mut max_checks) = (0, 0);
    let t = Instant::now();
    for b in &bundles {
        let no = keydist::member::shard_of(b, &out.event).unwrap();
        let (shard, witness) = (&out.shard(1, no).unwrap().bytes, &out.witness(1, no).unwrap().bytes);
        if let Ok((k, checks)) = keydist::member::open_epoch_counted(b, &out.event, shard, witness) {
            assert_eq!(k, out.key);
            opened += 1;
            max_checks = max_checks.max(checks);
        } else {
            assert!(g.revoked.contains(&b.leaf));
        }
    }
    let t_open = t.elapsed() / 200;
    eprintln!(
        "sd cover {} subsets in {t_cover:?}; build (labels, walks, nonces, AES-GCM, {} shards, hashes, index) in {t_build:?}; upload {} bytes; shard p50 {} p95 {} max {}; bundle {bundle_len} bytes in {t_bundle:?}; open {t_open:?} per member ({opened}/200 in, at most {max_checks} checks)",
        cover.len(),
        sizes.len(),
        out.upload_bytes(),
        p(50),
        p(95),
        sizes[sizes.len() - 1],
    );
}

fn member(i: u64) -> MemberId {
    let mut b = [0u8; 32];
    b[..8].copy_from_slice(&i.to_le_bytes());
    MemberId(b)
}

/// The keeper of a million members: what one join and one epoch write
/// as rows, what the whole state weighs, and how long a restore (with its
/// invariant checks) takes.
#[test]
#[ignore]
fn keeper_state_of_a_million_members() {
    let scope = ScopeRef { community: [1; 32], scope: [2; 32] };
    let params = TreeParams::default();
    let mut k = CsKeeper::new(scope, Policy::closed(), params, GenerationMaster::from_bytes([3; 32]), 0).unwrap();
    k.set_change_log(false);
    let t = Instant::now();
    for i in 0..1_000_000 {
        k.join(member(i), 0).unwrap();
    }
    let t_join = t.elapsed();
    let mut rng = TestRng(5);
    let banned: Vec<MemberId> = (0..20_000).map(|_| member(rng.below(1_000_000))).collect();
    k.revoke(&banned, Revoke::Ban, 0);
    let e = k.rotate(0, true).built().unwrap();
    k.published(e.event.epoch);
    k.set_change_log(true);
    let rows = |c: &[keydist::Change]| serde_json::to_vec(c).unwrap().len();
    k.join(member(2_000_000), 10).unwrap();
    let one_join = rows(&k.take_changes());
    k.revoke(&[member(7)], Revoke::Ban, 10);
    let one_ban = rows(&k.take_changes());
    let e = k.rotate(10, true).built().unwrap();
    let one_epoch = rows(&k.take_changes());
    k.published(e.event.epoch);
    let one_publish = rows(&k.take_changes());
    let t = Instant::now();
    let whole = serde_json::to_vec(k.state()).unwrap();
    let t_save = t.elapsed();
    let t = Instant::now();
    let state: KeeperState = serde_json::from_slice(&whole).unwrap();
    let t_parse = t.elapsed();
    let t = Instant::now();
    let back = CsKeeper::from_state(state).unwrap();
    let t_restore = t.elapsed();
    assert_eq!(back.stats().members, k.stats().members);
    eprintln!(
        "1 M joins in {t_join:?}; rows: a join {one_join} B, a ban {one_ban} B, an epoch {one_epoch} B, its publication {one_publish} B; whole state {} B of JSON written in {t_save:?}, parsed in {t_parse:?}, checked and restored in {t_restore:?}",
        whole.len()
    );
}
