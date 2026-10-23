// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The mandatory scenarios of stage 1 (internal/communities-keydist.md,
//! 11), each for both tree schemes (`cs-v1`, `sd-v1`) and, where it
//! applies, `flat-v1`; brought to the decisions of 19. The tests of the
//! fix wave (19.5) are in `fixwave.rs`.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::*;
use keydist::epoch::{block_node, sd_block_subset};
use keydist::kdf::NodeKeys;
use keydist::member::open_epoch_counted;
use keydist::sd::{self, SdLabels};
use keydist::shard::{self, SHARD_HEADER_LEN};
use keydist::{
    open_epoch, Bundle, Error, History, Key32, KeyDistribution, Policy, Revoke, Rotate, SchemeId, Thresholds,
    Verified, BLOCK_LEN,
};

fn keepers(policy: Policy) -> Vec<(&'static str, AnyKeeper)> {
    vec![
        ("cs-v1", any_cs(policy.clone(), 7, 16)),
        ("sd-v1", any_sd(policy.clone(), 7, 16)),
        ("flat-v1", any_flat(policy)),
    ]
}

/// A block of a shard: the subset it names (`cs-v1`: `(node, 0)`), its
/// nonce, its bytes and the generation of its shard.
type Block = ((u64, u64), Vec<u8>, Vec<u8>, u32);

/// Every block of every shard of an epoch.
fn all_blocks(out: &keydist::EpochOut) -> Vec<Block> {
    let mut v = Vec::new();
    for s in out.shards() {
        let (scheme, blocks) = shard::blocks(&s.bytes).unwrap();
        for b in blocks {
            let (id, nonce) = match scheme {
                SchemeId::SdV1 => (sd_block_subset(b), b[16..28].to_vec()),
                _ => ((block_node(b), 0), b[8..20].to_vec()),
            };
            v.push((id, nonce, b.to_vec(), s.gen));
        }
    }
    v
}

/// Opens a block of either scheme with a key.
fn open_any(key: &Key32, block: &[u8], gen: u32, epoch: u32) -> Result<keydist::EpochKey, Error> {
    if block.len() == BLOCK_LEN {
        keydist::epoch::open_block(key, block, &scope(), gen, epoch)
    } else {
        keydist::epoch::open_sd_block(key, block, &scope(), gen, epoch)
    }
}

#[test]
fn revoked_open_nothing_after_their_rotation_and_actives_open() {
    for (name, k) in keepers(closed()) {
        let mut w = World::new(k);
        for i in 0..60 {
            w.join(mid(i));
        }
        let first = w.rotate(false).expect("the first epoch is due at once");
        for i in 0..60 {
            assert!(w.opens(&mid(i), &first), "{name}: member {i}");
        }
        let banned: Vec<_> = [3, 17, 18, 40, 59].map(mid).to_vec();
        w.k.revoke(&banned, Revoke::Ban, w.now);
        let second = w.rotate(true).unwrap();
        w.now += 10;
        w.join(mid(100));
        let third = w.rotate(true).unwrap();
        for out in [&second, &third] {
            for i in (0..60).chain([100]) {
                let m = mid(i);
                if out.event.epoch == second.event.epoch && i == 100 {
                    continue;
                }
                assert_eq!(w.opens(&m, out), !banned.contains(&m), "{name}: member {i}, epoch {}", out.event.epoch);
            }
        }
    }
}

#[test]
fn collusion_of_every_revoked_bundle_reaches_no_block() {
    for scheme in TREES {
        let mut w = World::new(any_tree(scheme, closed(), 9, 32));
        for i in 0..400 {
            w.join(mid(i));
        }
        w.rotate(false).unwrap();
        let mut rng = TestRng::new(9);
        let revoked: BTreeSet<_> = (0..60).map(|_| mid(rng.below(400))).collect();
        let revoked: Vec<_> = revoked.into_iter().collect();
        w.k.revoke(&revoked, Revoke::Ban, w.now);
        let out = w.rotate(true).unwrap();
        let blocks = all_blocks(&out);
        assert!(!blocks.is_empty());
        let bundles: Vec<&Bundle> = revoked.iter().flat_map(|m| w.held[m].bundles.iter()).collect();
        match scheme {
            SchemeId::CsV1 => {
                // Every node key any revoked member holds.
                let mut stolen = BTreeMap::new();
                for b in &bundles {
                    for (node, key) in b.path() {
                        stolen.insert(node, key.clone());
                    }
                }
                for ((node, _), _, block, gen) in &blocks {
                    assert!(!stolen.contains_key(node), "a revoked member holds a cover node");
                    for key in stolen.values() {
                        assert!(open_any(key, block, *gen, out.event.epoch).is_err());
                    }
                }
            }
            _ => {
                // Every label and K_all any revoked member holds. A label
                // LABEL_{i,x} gives the subsets (i, j) with j under x and
                // nothing else; no cover subset is one of them.
                let mut stolen: BTreeMap<(u64, u64), Key32> = BTreeMap::new();
                let mut all = BTreeSet::new();
                for b in &bundles {
                    for (i, x, l) in b.sd_labels() {
                        stolen.insert((i, x), l.clone());
                    }
                    all.insert(b.all_key().unwrap().0);
                    assert!(b.subset_key(1, sd::ALL).is_some());
                }
                let mut tried = BTreeSet::new();
                for ((i, j), _, block, gen) in &blocks {
                    assert_ne!(*j, sd::ALL, "a cover with revoked leaves has no subset of every leaf");
                    for b in &bundles {
                        assert!(b.subset_key(*i, *j).is_none(), "a revoked bundle derives ({i}, {j})");
                    }
                    let depth_j = keydist::Tree::level(*j);
                    for ((si, x), label) in stolen.range((*i, 0)..=(*i, u64::MAX)) {
                        let lx = keydist::Tree::level(*x);
                        assert!(!(lx <= depth_j && j >> (depth_j - lx) == *x), "label ({si}, {x}) reaches ({i}, {j})");
                        if tried.insert((*i, *j, *x)) {
                            // The label's own subset key, and the label used as a key.
                            assert!(open_any(&sd::g_middle(label), block, *gen, out.event.epoch).is_err());
                            assert!(open_any(label, block, *gen, out.event.epoch).is_err());
                        }
                    }
                    for k in &all {
                        assert!(open_any(&Key32(*k), block, *gen, out.event.epoch).is_err());
                    }
                }
            }
        }
        for i in 0..400 {
            let m = mid(i);
            assert_eq!(w.opens(&m, &out), !revoked.contains(&m), "{scheme}: member {i}");
        }
    }
}

#[test]
fn strict_from_join_newcomer_opens_nothing_before_its_first_epoch() {
    for (name, k) in keepers(policy(120, 600, false, History::FromJoin)) {
        let mut w = World::new(k);
        for i in 0..20 {
            w.join(mid(i));
        }
        w.rotate(false).unwrap();
        w.now += 200;
        w.k.revoke(&[mid(1)], Revoke::Ban, w.now);
        w.rotate(true).unwrap();
        let tail = w.k.stats().next_leaf;
        let joined = w.join(mid(50));
        assert!(joined.current.is_none(), "{name}: a strict newcomer gets no current key");
        if name != "flat-v1" {
            assert_eq!(u64::from(joined.leaf), tail, "the newcomer takes the tail leaf");
        }
        // Idempotent join still gives no key.
        assert!(w.join(mid(50)).current.is_none());
        for out in &w.published {
            assert!(!w.opens(&mid(50), out), "{name}: epoch {} before the join", out.event.epoch);
            assert!(out.event.chain.is_none());
        }
        // The join asks for an epoch within the join window, not at once.
        assert!(w.rotate(false).is_none());
        w.now += 119;
        assert!(w.rotate(false).is_none());
        w.now += 1;
        let activation = w.rotate(false).expect("the join window ended");
        assert!(w.opens(&mid(50), &activation), "{name}");
        assert!(!w.opens(&mid(1), &activation));
        // A newcomer joining while an epoch is pending is not in it.
        let pending = w.k.built(w.now, true);
        let late = w.k.join(mid(51), w.now).unwrap();
        assert!(late.current.is_none());
        w.take(mid(51), &late);
        assert!(!w.opens(&mid(51), &pending));
        assert!(w.k.published(pending.event.epoch));
    }
}

#[test]
fn open_history_newcomer_gets_the_current_key_and_follows_the_chain() {
    for scheme in TREES {
        let mut w = World::new(any_tree(scheme, closed(), 6, 8));
        w.join(mid(0));
        let e1 = w.rotate(false).unwrap();
        w.k.revoke(&[mid(0)], Revoke::Ban, w.now);
        w.join(mid(1));
        let e2 = w.rotate(true).unwrap();
        let e3 = w.rotate(true).unwrap();
        let j = w.join(mid(2));
        assert_eq!(j.current.as_ref(), Some(&e3.key));
        assert!(j.missed.is_empty(), "open history keeps no missed keys");
        let st = w.held.get_mut(&mid(2)).unwrap();
        assert_eq!(st.follow_chain(&e3.event).unwrap(), e2.key);
        assert_eq!(st.follow_chain(&e2.event).unwrap(), e1.key);
        assert!(st.follow_chain(&e1.event).is_err());
        let ch = st.channel_key(e1.event.epoch, b"general").unwrap();
        assert_eq!(ch, keydist::channel_key(&e1.key, &scope(), b"general"));
    }
}

#[test]
fn planned_migration_late_member_reads_what_it_was_entitled_to() {
    for history in [History::Open, History::FromJoin] {
        for scheme in TREES {
            let mut w = World::new(any_tree(scheme, policy(120, 600, false, history), 6, 8));
            for i in 0..40 {
                w.join(mid(i));
            }
            let before = w.rotate(false).unwrap();
            let t = Thresholds { capacity_ratio: 0.5, grace_secs: 1000, ..Thresholds::default() };
            assert!(w.k.migrate_if_needed(&Thresholds::default(), &master(7), w.now).unwrap().is_none());
            let g = w.k.migrate_if_needed(&t, &master(7), w.now).unwrap().expect("40 of 64 leaves");
            assert_eq!(g.gen, 2);
            assert!(w.k.migrate_if_needed(&t, &master(8), w.now).unwrap().is_none(), "one migration at a time");
            assert_eq!(w.k.migrate(master(8), None, w.now).unwrap_err(), Error::State("a migration is running"));
            // Early members migrate on their first activity.
            for i in 0..20 {
                let iss = w.bundle_for(mid(i));
                assert_eq!(iss.gen, 2);
                assert!(iss.missed.is_empty());
            }
            // A newcomer during the grace joins the new generation.
            assert_eq!(w.join(mid(100)).gen, 2);
            // During the grace every epoch carries both covers.
            w.k.revoke(&[mid(5), mid(25)], Revoke::Ban, w.now);
            let grace = w.rotate(true).unwrap();
            assert_eq!(grace.event.trees.len(), 2);
            for i in (0..40).chain([100]) {
                let m = mid(i);
                assert_eq!(w.opens(&m, &grace), i != 5 && i != 25, "{scheme}: member {i} in the grace");
            }
            // After the grace the old generation is gone.
            w.now += 1000;
            let after1 = w.rotate(true).unwrap();
            let after2 = w.rotate(true).unwrap();
            for out in [&after1, &after2] {
                assert_eq!(out.event.trees.len(), 1);
                assert_eq!(out.event.trees[0].gen, 2);
                assert!(!w.opens(&mid(30), out), "an unmigrated member cannot read with the old bundle");
                assert!(w.opens(&mid(3), out));
                assert!(!w.opens(&mid(5), out));
            }
            // The late member gets its bundle and the current key; under
            // from_join also every key published while it was not
            // migrated (the grace too), under open the chain gives them.
            let late = w.bundle_for(mid(30));
            assert_eq!(late.gen, 2);
            assert_eq!(late.current.as_ref(), Some(&after2.key));
            let missed: Vec<u32> = late.missed.iter().map(|k| k.epoch).collect();
            match history {
                History::FromJoin => {
                    assert_eq!(missed, vec![grace.event.epoch, after1.event.epoch, after2.event.epoch]);
                    assert_eq!(late.missed[1], after1.key);
                }
                History::Open => {
                    assert!(missed.is_empty());
                    let st = w.held.get_mut(&mid(30)).unwrap();
                    assert_eq!(st.follow_chain(&after2.event).unwrap(), after1.key);
                }
            }
            let held = &w.held[&mid(30)];
            for e in [&before, &grace, &after1, &after2] {
                assert!(held.key(e.event.epoch).is_some(), "epoch {}", e.event.epoch);
            }
            let next = w.rotate(true).unwrap();
            assert!(w.opens(&mid(30), &next));
            // A banned member gets nothing.
            assert_eq!(w.k.bundle_for(mid(25), w.now), Err(Error::UnknownMember));
        }
    }
}

#[test]
fn emergency_regeneration_kills_the_old_master() {
    for history in [History::Open, History::FromJoin] {
        for scheme in TREES {
            let mut w = World::new(any_tree(scheme, policy(120, 600, false, history), 6, 8));
            for i in 0..30 {
                w.join(mid(i));
            }
            w.rotate(false).unwrap();
            // An epoch built under the stolen master is dropped, its number too.
            let dropped = w.k.built(w.now, true).event.epoch;
            let old_master = Key32([0x4d; 32]);
            let g = w.k.emergency_regenerate(master(0x99), w.now).unwrap();
            assert_eq!(g.gen, 2);
            let first = g.epoch.clone().unwrap();
            assert!(first.event.epoch > dropped);
            assert_eq!(w.k.pending().unwrap(), &*first);
            assert!(first.event.trees.iter().all(|t| t.gen == 2));
            assert!(!w.k.published(dropped), "the dropped epoch is not pending");
            w.publish(first.clone());
            // Half the members get the new bundle at once, the rest later.
            for i in 0..15 {
                let iss = w.bundle_for(mid(i));
                assert_eq!(iss.kind, keydist::GenerationKind::Emergency);
                assert_eq!(iss.current.as_ref(), Some(&first.key));
                if history == History::FromJoin {
                    assert_eq!(iss.missed.first().map(|k| k.epoch), Some(first.event.epoch));
                }
            }
            let next = w.rotate(true).unwrap();
            for i in 0..30 {
                assert_eq!(w.opens(&mid(i), &next), i < 15, "{scheme}: member {i}");
            }
            // The holder of the old master derives every key of every
            // generation number and opens no block.
            for out in [&first, &next] {
                for ((i, j), _, block, gen) in all_blocks(out) {
                    for g in [1, 2] {
                        let key = match scheme {
                            SchemeId::CsV1 => NodeKeys::new(&old_master, &scope(), g).node(i),
                            _ => SdLabels::new(&old_master, &scope(), g).subset_key(i, j),
                        };
                        assert!(open_any(&key, &block, gen, out.event.epoch).is_err());
                    }
                }
                // Nor with an old bundle.
                assert!(!w.opens(&mid(20), out));
            }
            let late = w.bundle_for(mid(20));
            let got: Vec<u32> = late.missed.iter().map(|k| k.epoch).collect();
            match history {
                History::FromJoin => assert_eq!(got, vec![first.event.epoch, next.event.epoch]),
                History::Open => {
                    assert!(got.is_empty());
                    assert_eq!(late.current.as_ref(), Some(&next.key));
                    let st = w.held.get_mut(&mid(20)).unwrap();
                    assert_eq!(st.follow_chain(&next.event).unwrap(), first.key);
                }
            }
            // The member dropped its bundle of generation 1.
            assert!(w.held[&mid(20)].bundles.iter().all(|b| b.gen == 2));
            let after = w.rotate(true).unwrap();
            assert!(w.opens(&mid(20), &after));
        }
    }
}

#[test]
fn repeat_join_gives_the_same_leaf_and_bundle() {
    for (name, k) in keepers(closed()) {
        let mut w = World::new(k);
        let a = w.join(mid(1));
        w.join(mid(2));
        let b = w.join(mid(1));
        assert_eq!(a.leaf, b.leaf, "{name}");
        assert_eq!(a.bundle, b.bundle, "{name}");
        assert_eq!(w.k.stats().next_leaf, 2, "{name}");
        // Put out and let in again: a new leaf, never the old one.
        w.k.revoke(&[mid(1)], Revoke::Ban, w.now);
        let c = w.join(mid(1));
        assert_ne!(c.leaf, a.leaf, "{name}");
    }
}

#[test]
fn crash_between_build_and_publish_gives_the_same_bytes() {
    for scheme in TREES {
        let mut k = any_tree(scheme, closed(), 8, 16);
        for i in 0..100 {
            k.join(mid(i), 0).unwrap();
        }
        k.revoke(&[mid(7), mid(70)], Revoke::Ban, 0);
        let out = k.built(10, true);
        // Saved, crashed, restored: the same epoch, byte for byte.
        let saved = serde_json::to_string(k.tree_state()).unwrap();
        drop(k);
        let mut k = AnyKeeper::restore(scheme, serde_json::from_str(&saved).unwrap()).unwrap();
        assert_eq!(k.pending(), Some(&*out));
        assert_eq!(k.pending().unwrap().outbound(), out.outbound());
        // The state of one scheme does not restore into the other's keeper.
        let other = if scheme == SchemeId::CsV1 { SchemeId::SdV1 } else { SchemeId::CsV1 };
        assert!(matches!(AnyKeeper::restore(other, serde_json::from_str(&saved).unwrap()), Err(Error::Invalid(_))));
        // Nothing new is built while it waits.
        assert_eq!(k.rotate(1_000_000, true), Rotate::Blocked { pending_epoch: out.event.epoch });
        assert_eq!(k.pending(), Some(&*out));
        assert!(!k.published(out.event.epoch + 1), "an acknowledgement names its epoch");
        assert!(k.published(out.event.epoch));
        assert!(k.pending().is_none());
        assert!(!k.published(out.event.epoch), "twice is a no-op");
        let next = k.built(20, true);
        assert_eq!(next.event.epoch, out.event.epoch + 1);
        assert_eq!(next.event.prev_epoch, Some(out.event.epoch));
        // The new epoch retires the blobs of the published one.
        assert_eq!(next.retires, out.blob_hashes());
        // A new rotation: a new key and new nonces for the very same cover.
        assert_ne!(next.key.key, out.key.key);
        let nonces = |o: &keydist::EpochOut| -> BTreeMap<(u64, u64), Vec<u8>> {
            all_blocks(o).into_iter().map(|(id, nonce, _, _)| (id, nonce)).collect()
        };
        let (a, b) = (nonces(&out), nonces(&next));
        assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>());
        assert!(a.iter().all(|(n, x)| b[n] != *x));
    }
}

#[test]
fn flat_crash_between_build_and_publish_gives_the_same_bytes() {
    let mut k = flat(closed());
    for i in 0..10 {
        k.join(mid(i), 0).unwrap();
    }
    let out = k.rotate(10, false).built().unwrap();
    let saved = serde_json::to_string(k.state()).unwrap();
    let mut k = keydist::FlatKeeper::from_state(serde_json::from_str(&saved).unwrap()).unwrap();
    assert_eq!(k.pending(), Some(&*out));
    assert!(matches!(k.rotate(20, true), Rotate::Blocked { .. }));
    assert!(k.published(out.event.epoch));
    let next = k.rotate(20, true).built().unwrap();
    assert_eq!(next.event.epoch, out.event.epoch + 1);
    assert_ne!(next.key, out.key);
    assert!(format!("{:?}", next.secrets[0].1).starts_with("SecretBytes("));
}

#[test]
fn tampered_foreign_and_stale_shards_are_refused() {
    // Index mode and Merkle mode (more than 512 shards).
    for scheme in TREES {
        for (depth, lps, members) in [(8u8, 16u32, 200u64), (14, 4, 9000)] {
            let mut k = any_tree(scheme, closed(), depth, lps);
            let mut bundles = Vec::new();
            for i in 0..members {
                let iss = k.join(mid(i), 0).unwrap();
                bundles.push(Bundle::from_bytes(&iss.bundle).unwrap());
            }
            let e1 = k.built(0, false);
            assert!(k.published(e1.event.epoch));
            k.revoke(&[mid(3)], Revoke::Ban, 0);
            let e2 = k.built(0, true);
            assert!(k.published(e2.event.epoch));
            let merkle = e2.event.trees[0].root.is_some();
            assert_eq!(merkle, depth == 14);
            assert_eq!(e2.event.trees[0].index.is_none(), merkle);
            assert_eq!(e2.proof(1, 0).is_none(), !merkle);
            assert_eq!(e2.index(1).is_none(), merkle);
            // The event stays small either way.
            let json = serde_json::to_vec(&e2.event).unwrap();
            assert!(json.len() < 1024, "{} bytes", json.len());
            let b = &bundles[100];
            let no = keydist::member::shard_of(b, &e2.event).unwrap();
            let mine = &e2.shard(1, no).unwrap().bytes;
            let witness = &e2.witness(1, no).unwrap().bytes;
            assert!(open_epoch(b, &e2.event, mine, witness).is_ok());
            // A flipped byte anywhere, in the shard or in its witness.
            for at in [0, 70, SHARD_HEADER_LEN + 3, mine.len() - 1] {
                let mut bad = mine.clone();
                bad[at] ^= 1;
                assert!(matches!(open_epoch(b, &e2.event, &bad, witness), Err(Error::Invalid(_))), "{scheme}: byte {at}");
            }
            for at in [0, 5, 40, witness.len() - 1] {
                let mut bad = witness.clone();
                bad[at] ^= 1;
                assert!(matches!(open_epoch(b, &e2.event, mine, &bad), Err(Error::Invalid(_))), "{scheme}: witness {at}");
            }
            // A foreign shard (another member's) with its own witness.
            let other = (no + 1) % e2.event.trees[0].count;
            let foreign = &e2.shard(1, other).unwrap().bytes;
            let foreign_w = &e2.witness(1, other).unwrap().bytes;
            assert!(matches!(open_epoch(b, &e2.event, foreign, foreign_w), Err(Error::Invalid(_))));
            // The same shard of another epoch, with that epoch's witness.
            let stale = &e1.shard(1, no).unwrap().bytes;
            let stale_w = &e1.witness(1, no).unwrap().bytes;
            assert!(matches!(open_epoch(b, &e2.event, stale, stale_w), Err(Error::Invalid(_))));
            assert!(matches!(open_epoch(b, &e2.event, mine, stale_w), Err(Error::Invalid(_))));
            // No witness.
            assert!(matches!(open_epoch(b, &e2.event, mine, &[]), Err(Error::Invalid(_))));
            if merkle {
                assert!(open_epoch(b, &e2.event, mine, foreign_w).is_err());
            }
            // The proof lies outside the hashed content.
            assert_eq!(shard::sha256(mine), e2.shard(1, no).unwrap().sha256);
            // Every member finds its block in at most depth + 1 lookups
            // (sd-v1: depth); the revoked one in none.
            let bound = usize::from(depth) + usize::from(scheme == SchemeId::CsV1);
            for (i, b) in bundles.iter().enumerate() {
                let no = keydist::member::shard_of(b, &e2.event).unwrap();
                let r = open_epoch_counted(b, &e2.event, &e2.shard(1, no).unwrap().bytes, &e2.witness(1, no).unwrap().bytes);
                if i == 3 {
                    assert_eq!(r, Err(Error::NotCovered));
                } else {
                    let (key, checks) = r.unwrap();
                    assert_eq!(key, e2.key);
                    assert!(checks <= bound, "{scheme}: {checks} checks");
                }
            }
        }
    }
}

#[test]
fn shards_bundles_and_events_of_another_scheme_are_refused() {
    let mut c = any_cs(closed(), 6, 8);
    let mut s = any_sd(closed(), 6, 8);
    let (mut bc, mut bs) = (Vec::new(), Vec::new());
    for i in 0..20 {
        bc.push(Bundle::from_bytes(&c.join(mid(i), 0).unwrap().bundle).unwrap());
        bs.push(Bundle::from_bytes(&s.join(mid(i), 0).unwrap().bundle).unwrap());
    }
    let (ec, es) = (c.built(0, false), s.built(0, false));
    assert_eq!(es.event.scheme, SchemeId::SdV1);
    let (sc, ss) = (&ec.shard(1, 0).unwrap().bytes, &es.shard(1, 0).unwrap().bytes);
    let (wc, ws) = (&ec.index(1).unwrap().bytes, &es.index(1).unwrap().bytes);
    assert_eq!(&ss[..4], shard::SD_MAGIC);
    assert!(open_epoch(&bc[2], &ec.event, sc, wc).is_ok());
    assert!(open_epoch(&bs[2], &es.event, ss, ws).is_ok());
    assert!(matches!(open_epoch(&bc[2], &es.event, ss, ws), Err(Error::Invalid(_))));
    assert!(matches!(open_epoch(&bs[2], &ec.event, sc, wc), Err(Error::Invalid(_))));
    assert!(matches!(open_epoch(&bs[2], &es.event, sc, ws), Err(Error::Invalid(_))));
    // An event that names the other scheme but carries the same index:
    // the shard's magic gives it away.
    let mut lying = (*es.event).clone();
    lying.scheme = SchemeId::CsV1;
    let lying = Verified::after_signature_check(lying);
    assert!(matches!(open_epoch(&bc[2], &lying, ss, ws), Err(Error::Invalid(_))));
}

#[test]
fn shard_of_another_generation_is_refused() {
    for scheme in TREES {
        let mut w = World::new(any_tree(scheme, closed(), 6, 8));
        for i in 0..40 {
            w.join(mid(i));
        }
        w.rotate(false).unwrap();
        let t = Thresholds { capacity_ratio: 0.5, ..Thresholds::default() };
        w.k.migrate_if_needed(&t, &master(7), w.now).unwrap().unwrap();
        w.bundle_for(mid(0));
        let out = w.rotate(true).unwrap();
        let st = &w.held[&mid(0)];
        let new = st.bundles.iter().find(|b| b.gen == 2).unwrap();
        let old = st.bundles.iter().find(|b| b.gen == 1).unwrap();
        let no_new = keydist::member::shard_of(new, &out.event).unwrap();
        let no_old = keydist::member::shard_of(old, &out.event).unwrap();
        let (w1, w2) = (&out.index(1).unwrap().bytes, &out.index(2).unwrap().bytes);
        assert!(open_epoch(new, &out.event, &out.shard(2, no_new).unwrap().bytes, w2).is_ok());
        assert!(open_epoch(old, &out.event, &out.shard(1, no_old).unwrap().bytes, w1).is_ok());
        assert!(matches!(open_epoch(new, &out.event, &out.shard(1, no_new).unwrap().bytes, w1), Err(Error::Invalid(_))));
        assert!(matches!(open_epoch(new, &out.event, &out.shard(1, no_new).unwrap().bytes, w2), Err(Error::Invalid(_))));
        assert!(matches!(open_epoch(old, &out.event, &out.shard(2, no_old).unwrap().bytes, w1), Err(Error::Invalid(_))));
    }
}

#[test]
fn batching_windows_urgency_and_lazy_leaves() {
    for open_scope in [false, true] {
        for (name, k) in keepers(policy(120, 600, open_scope, History::Open)) {
            let mut w = World::new(k);
            for i in 0..30 {
                w.join(mid(i));
            }
            w.rotate(false).unwrap();
            let t0 = w.now;
            // Three bans in one window: one epoch, when the window from
            // the first one ends.
            for (dt, i) in [(0, 1), (30, 2), (60, 3)] {
                w.now = t0 + dt;
                w.k.revoke(&[mid(i)], Revoke::Ban, w.now);
                assert!(w.rotate(false).is_none(), "{name}");
            }
            w.now = t0 + 119;
            assert_eq!(w.k.rotate(w.now, false), Rotate::NotDue { due_at: Some(t0 + 120) });
            w.now = t0 + 120;
            let e = w.rotate(false).expect("the window ended");
            assert!(w.rotate(false).is_none(), "one epoch per batch");
            for i in 1..=3 {
                assert!(!w.opens(&mid(i), &e));
            }
            // Urgent: at once.
            w.k.revoke(&[mid(4)], Revoke::Ban, w.now);
            let u = w.rotate(true).expect("urgent");
            assert!(!w.opens(&mid(4), &u));
            // A leave.
            let t1 = w.now;
            w.k.revoke(&[mid(5)], Revoke::Leave, w.now);
            assert!(!w.k.is_member(&mid(5)));
            w.now = t1 + 599;
            assert!(w.rotate(false).is_none(), "{name}");
            w.now = t1 + 600;
            if open_scope {
                assert!(w.rotate(false).is_none(), "{name}: a lazy leave makes no epoch");
                assert_eq!(w.k.next_wake(), None);
                w.now = t1 + 10_000_000;
                assert!(w.rotate(false).is_none());
                // It still holds the current key, as accepted.
                assert!(w.held[&mid(5)].key(u.event.epoch).is_some());
                // The next epoch for any reason leaves it out.
                w.k.revoke(&[mid(6)], Revoke::Ban, w.now);
                w.now += 120;
                let next = w.rotate(false).unwrap();
                assert!(!w.opens(&mid(5), &next) && !w.opens(&mid(6), &next));
                assert!(w.opens(&mid(7), &next));
            } else {
                let l = w.rotate(false).expect("a closed leave rotates after its window");
                assert!(!w.opens(&mid(5), &l));
                assert!(w.opens(&mid(7), &l));
            }
        }
    }
}

#[test]
fn rebuilt_epochs_are_identical_and_new_ones_fresh() {
    for scheme in TREES {
        let mut k = any_tree(scheme, closed(), 10, 64);
        for i in 0..500 {
            k.join(mid(i), 0).unwrap();
        }
        let out = k.built(0, false);
        let state = k.tree_state().clone();
        let a = AnyKeeper::restore(scheme, state.clone()).unwrap();
        let b = AnyKeeper::restore(scheme, state).unwrap();
        assert_eq!(a.pending(), Some(&*out));
        assert_eq!(b.pending(), Some(&*out));
    }
    // Two rotations never repeat a key or a seed.
    let params = keydist::TreeParams { depth: 4, leaves_per_shard: 4 };
    let mut x = keydist::CsKeeper::new(scope(), closed(), params, master(1), 0).unwrap();
    x.join(mid(1), 0).unwrap();
    let k1 = x.rotate(0, false).built().unwrap();
    assert!(x.published(k1.event.epoch));
    let k2 = x.rotate(0, true).built().unwrap();
    assert_ne!(k1.key.key, k2.key.key);
    let mut y = keydist::SdKeeper::new(scope(), closed(), params, master(1), 0).unwrap();
    y.join(mid(1), 0).unwrap();
    let s1 = y.rotate(0, false).built().unwrap();
    assert!(y.published(s1.event.epoch));
    let s2 = y.rotate(0, true).built().unwrap();
    assert_ne!(s1.key.key, s2.key.key);
    assert_ne!(s1.key.key, k1.key.key);
}

/// Random joins, bans and leaves with crashes (the state through JSON) on
/// depth 14: a leaf is never handed out twice; a member always holds the
/// current key (unless it waits for its first epoch); after every
/// rotation an identity opens the new epoch if and only if it is in.
fn randomized(history: History, open_scope: bool, scheme: SchemeId, seed: u64) {
    let policy = policy(60, 120, open_scope, history);
    let mut rng = TestRng::new(seed);
    let k = match scheme {
        SchemeId::FlatV1 => any_flat(policy),
        _ => any_tree(scheme, policy, 14, 64),
    };
    let mut w = World::new(k);
    let mut leaves: BTreeMap<(u32, u32), keydist::MemberId> = BTreeMap::new();
    let mut waiting: BTreeSet<keydist::MemberId> = BTreeSet::new();
    let mut next_id = 0u64;
    let mut rotations = 0;
    for step in 0..900 {
        w.now += rng.below(25);
        match rng.below(10) {
            0..=4 => {
                let m = if next_id > 0 && rng.below(5) == 0 {
                    mid(rng.below(next_id))
                } else {
                    next_id += 1;
                    mid(next_id - 1)
                };
                let was = w.k.is_member(&m);
                let iss = w.join(m);
                if let Some(prev) = leaves.insert((iss.gen, iss.leaf), m) {
                    assert_eq!(prev, m, "leaf ({}, {}) handed out twice", iss.gen, iss.leaf);
                }
                if !was && history == History::FromJoin {
                    waiting.insert(m);
                }
            }
            5 | 6 => {
                let members: Vec<_> = w.held.keys().copied().filter(|m| w.k.is_member(m)).collect();
                if !members.is_empty() {
                    let m = members[rng.below(members.len() as u64) as usize];
                    let why = if rng.below(2) == 0 { Revoke::Ban } else { Revoke::Leave };
                    w.k.revoke(&[m], why, w.now);
                    waiting.remove(&m);
                }
            }
            7 => {
                // A crash: the keeper comes back from its saved state.
                w.k.crash();
            }
            _ => {}
        }
        if let Some(out) = w.rotate(rng.below(8) == 0) {
            rotations += 1;
            waiting.clear();
            for m in w.held.keys() {
                assert_eq!(w.opens(m, &out), w.k.is_member(m), "{scheme} step {step}: {m:?} epoch {}", out.event.epoch);
            }
        }
        let head = w.published.last().map(|o| o.event.epoch);
        for (m, st) in &w.held {
            if w.k.is_member(m) && !waiting.contains(m) {
                if let Some(h) = head {
                    assert!(st.key(h).is_some(), "{scheme} step {step}: member {m:?} lacks epoch {h}");
                }
            }
        }
    }
    assert!(rotations > 10, "only {rotations} rotations");
}

#[test]
fn randomized_cs_open_history_closed_scope() {
    randomized(History::Open, false, SchemeId::CsV1, 101);
}

#[test]
fn randomized_cs_from_join_open_scope() {
    randomized(History::FromJoin, true, SchemeId::CsV1, 202);
}

#[test]
fn randomized_sd_open_history_closed_scope() {
    randomized(History::Open, false, SchemeId::SdV1, 505);
}

#[test]
fn randomized_sd_from_join_open_scope() {
    randomized(History::FromJoin, true, SchemeId::SdV1, 606);
}

#[test]
fn randomized_flat() {
    randomized(History::Open, true, SchemeId::FlatV1, 303);
    randomized(History::FromJoin, false, SchemeId::FlatV1, 404);
}
