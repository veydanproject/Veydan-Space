// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The tests of the fix wave after the review of 2026-10-17
//! (internal/communities-keydist.md, 19.5), for both tree schemes.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use common::*;
use keydist::issued::MISSED_PAGE;
use keydist::{
    open_epoch, Bundle, Change, EpochOut, Error, GenerationKind, GenerationMaster, History, Issued, KeeperState,
    KeyDistribution, MemberId, MemberState, Policy, Revoke, RootSecret, Rotate, SchemeId, ScopeRef, Status,
    Thresholds, TreeParams, Verified,
};

/// The rows of a keeper's change log, applied to a replica.
struct Replica(Option<KeeperState>);

impl Replica {
    fn follow(&mut self, k: &mut AnyKeeper) {
        let changes = k.take_changes();
        // The rows survive JSON.
        let json = serde_json::to_string(&changes).unwrap();
        let changes: Vec<Change> = serde_json::from_str(&json).unwrap();
        for ch in changes {
            match (&mut self.0, ch) {
                (None, Change::Header(h)) => self.0 = Some(KeeperState::from_header(h)),
                (None, other) => panic!("a row before the header: {other:?}"),
                (Some(s), ch) => s.apply(ch),
            }
        }
        assert_eq!(self.0.as_ref(), Some(k.tree_state()), "the rows give the state");
    }
}

/// Whether a member takes an event at all: after an emergency it refuses
/// the events of older generations only.
fn acceptable(st: &MemberState, out: &EpochOut) -> bool {
    st.emergency_gen.is_none_or(|e| out.event.newest_gen().is_some_and(|n| n >= e))
}

/// Walks the chains of the published events down from every key a member
/// holds (open history), over the events it takes.
fn walk_chains(st: &mut MemberState, published: &[Arc<EpochOut>]) {
    for out in published.iter().rev() {
        if !acceptable(st, out) {
            assert!(st.follow_chain(&out.event).is_err());
            continue;
        }
        if st.key(out.event.epoch).is_some() && out.event.prev_epoch.is_some_and(|p| st.key(p).is_none()) {
            st.follow_chain(&out.event).unwrap();
        }
    }
}

/// A random run: joins, bans and leaves, crashes while an epoch waits
/// for its publication, superseded epochs, planned migrations (some to a
/// deeper tree) and emergencies, members that rebundle when the event
/// says so, and former members that try their old bundles. The change log
/// replays the state at every step.
fn randomized(scheme: SchemeId, history: History, open_scope: bool, seed: u64) {
    let mut rng = TestRng::new(seed);
    let t = Thresholds { grace_secs: 300, ..Thresholds::default() };
    let mut w = World::new(any_tree(scheme, policy(60, 120, open_scope, history).with_thresholds(t), 10, 16));
    let mut replica = Replica(None);
    replica.follow(&mut w.k);
    let mut leaves: BTreeMap<(u32, u32), MemberId> = BTreeMap::new();
    // The first epoch a member is entitled to, and the newest epoch built
    // when a former member was put out.
    let mut first: BTreeMap<MemberId, u32> = BTreeMap::new();
    let mut cut: BTreeMap<MemberId, u32> = BTreeMap::new();
    let mut next_master = 0x10u8;
    let mut next_id = 0u64;
    let (mut published, mut migrations, mut emergencies, mut supersedes, mut pending_crashes) = (0, 0, 0, 0, 0);
    for step in 0..700 {
        w.now += rng.below(40);
        match rng.below(20) {
            0..=8 => {
                let m = if next_id > 0 && rng.below(5) == 0 {
                    mid(rng.below(next_id))
                } else {
                    next_id += 1;
                    mid(next_id - 1)
                };
                let was = w.k.is_member(&m);
                match w.k.join(m, w.now) {
                    Ok(iss) => {
                        if let Some(prev) = leaves.insert((iss.gen, iss.leaf), m) {
                            assert_eq!(prev, m, "leaf ({}, {}) handed out twice", iss.gen, iss.leaf);
                        }
                        if !was {
                            let entitled = match history {
                                History::Open => 0,
                                History::FromJoin => w.k.stats().epoch + 1,
                            };
                            first.insert(m, entitled);
                            cut.remove(&m);
                        }
                        w.take(m, &iss);
                    }
                    Err(Error::GenerationFull) => {}
                    Err(e) => panic!("join: {e}"),
                }
            }
            9..=11 => {
                let members: Vec<_> = w.held.keys().copied().filter(|m| w.k.is_member(m)).collect();
                if !members.is_empty() {
                    let m = members[rng.below(members.len() as u64) as usize];
                    let why = if rng.below(2) == 0 { Revoke::Ban } else { Revoke::Leave };
                    w.k.revoke(&[m], why, w.now);
                    cut.insert(m, w.k.stats().epoch);
                }
            }
            12 => w.k.crash(),
            13 => {
                // Built, then a crash before the publication: the same bytes.
                if let Rotate::Built(out) = w.k.rotate(w.now, true) {
                    replica.follow(&mut w.k);
                    w.k.crash();
                    assert_eq!(w.k.pending(), Some(&*out));
                    pending_crashes += 1;
                    if rng.below(2) == 0 {
                        // An urgent ban while it waits: superseded.
                        let members: Vec<_> = w.held.keys().copied().filter(|m| w.k.is_member(m)).collect();
                        if let Some(&m) = members.first() {
                            w.k.revoke(&[m], Revoke::Ban, w.now);
                            cut.insert(m, w.k.stats().epoch);
                        }
                        let fresh = w.k.supersede_pending(w.now).unwrap();
                        assert_eq!(fresh.event.epoch, out.event.epoch + 1);
                        assert!(out.blob_hashes().iter().all(|h| fresh.retires.contains(h)));
                        assert!(!w.k.published(out.event.epoch));
                        supersedes += 1;
                    }
                }
            }
            14 => {
                let t = Thresholds { capacity_ratio: 0.0, grace_secs: rng.below(600), ..Thresholds::default() };
                let depth = 10 + rng.below(2) as u8;
                let params = (rng.below(3) == 0).then_some(TreeParams { depth, leaves_per_shard: 16 });
                let m = master(next_master);
                let r = if params.is_some() {
                    w.k.migrate(m, params, w.now).map(Some)
                } else {
                    w.k.migrate_if_needed(&t, &m, w.now)
                };
                match r {
                    Ok(Some(_)) => {
                        migrations += 1;
                        next_master += 1;
                    }
                    Ok(None) | Err(Error::State(_)) => {}
                    Err(e) => panic!("migrate: {e}"),
                }
            }
            15 if rng.below(4) == 0 => {
                let g = w.k.emergency_regenerate(master(next_master), w.now).unwrap();
                next_master += 1;
                emergencies += 1;
                assert_eq!(g.kind, GenerationKind::Emergency);
            }
            16 => {
                // A member's first activity after a migration.
                let members: Vec<_> = w.held.keys().copied().filter(|m| w.k.is_member(m)).collect();
                if !members.is_empty() {
                    let m = members[rng.below(members.len() as u64) as usize];
                    match w.k.bundle_for(m, w.now) {
                        Ok(iss) => {
                            if let Some(prev) = leaves.insert((iss.gen, iss.leaf), m) {
                                assert_eq!(prev, m, "leaf ({}, {}) handed out twice", iss.gen, iss.leaf);
                            }
                            w.take(m, &iss);
                        }
                        Err(Error::GenerationFull) => {}
                        Err(e) => panic!("bundle_for: {e}"),
                    }
                }
            }
            _ => {}
        }
        // An emergency leaves its first epoch pending; anything else may
        // build one now.
        let out = match w.k.pending() {
            Some(p) => Some(Arc::new(p.clone())),
            None => w.k.rotate(w.now, rng.below(6) == 0).built(),
        };
        if let Some(out) = out {
            replica.follow(&mut w.k);
            w.publish(out.clone());
            published += 1;
            // Members the event asks to rebundle do so (and walk the chain
            // under open history); former members open nothing after
            // their cut.
            let ids: Vec<MemberId> = w.held.keys().copied().collect();
            for m in ids {
                if !w.k.is_member(&m) {
                    if cut.get(&m).is_some_and(|&c| out.event.epoch > c) {
                        assert!(!w.opens(&m, &out), "{scheme} step {step}: former {m:?} opens {}", out.event.epoch);
                    }
                    continue;
                }
                if let Status::NeedBundle { gen } = w.held[&m].status(&out.event) {
                    let iss = w.bundle_for(m);
                    assert_eq!(iss.gen, gen);
                    if let Some(prev) = leaves.insert((iss.gen, iss.leaf), m) {
                        assert_eq!(prev, m, "leaf ({}, {}) handed out twice", iss.gen, iss.leaf);
                    }
                }
                if history == History::Open {
                    walk_chains(w.held.get_mut(&m).unwrap(), &w.published);
                }
                let st = &w.held[&m];
                for e in &w.published {
                    // Under open history the keys before an emergency the
                    // member took come only from events it now refuses.
                    let reachable = history == History::FromJoin || acceptable(st, e);
                    if e.event.epoch >= first[&m] && reachable {
                        assert!(
                            st.key(e.event.epoch).is_some(),
                            "{scheme} {history:?} step {step}: member {m:?} lacks epoch {} (entitled from {})",
                            e.event.epoch,
                            first[&m],
                        );
                    }
                }
            }
        }
        replica.follow(&mut w.k);
    }
    assert!(published > 30 && migrations > 3 && pending_crashes > 3, "{published} {migrations} {pending_crashes}");
    eprintln!(
        "{scheme} {history:?}: {published} epochs, {migrations} migrations, {emergencies} emergencies, {supersedes} superseded"
    );
}

#[test]
fn randomized_cs_open_with_crashes_migrations_and_emergencies() {
    randomized(SchemeId::CsV1, History::Open, false, 11);
}

#[test]
fn randomized_cs_from_join_with_crashes_migrations_and_emergencies() {
    randomized(SchemeId::CsV1, History::FromJoin, true, 12);
}

#[test]
fn randomized_sd_open_with_crashes_migrations_and_emergencies() {
    randomized(SchemeId::SdV1, History::Open, true, 13);
}

#[test]
fn randomized_sd_from_join_with_crashes_migrations_and_emergencies() {
    randomized(SchemeId::SdV1, History::FromJoin, false, 14);
}

#[test]
fn a_member_that_skips_two_planned_migrations_reads_every_epoch() {
    for history in [History::Open, History::FromJoin] {
        for scheme in TREES {
            let mut w = World::new(any_tree(scheme, policy(120, 600, false, history), 6, 8));
            for i in 0..10 {
                w.join(mid(i));
            }
            w.rotate(false).unwrap();
            let t = Thresholds { capacity_ratio: 0.0, grace_secs: 100, ..Thresholds::default() };
            let mut epochs = Vec::new();
            for round in 0..2u8 {
                w.k.migrate_if_needed(&t, &master(7 + round), w.now).unwrap().unwrap();
                // Everybody but member 9 migrates.
                for i in 0..9 {
                    w.bundle_for(mid(i));
                }
                epochs.push(w.rotate(true).unwrap()); // in the grace
                w.now += 100;
                epochs.push(w.rotate(true).unwrap()); // after it
            }
            // Member 9 still has only its bundle of generation 1.
            assert!(w.held[&mid(9)].bundles.iter().all(|b| b.gen == 1));
            assert_eq!(w.held[&mid(9)].status(&epochs[3].event), Status::NeedBundle { gen: 3 });
            let late = w.bundle_for(mid(9));
            assert_eq!(late.gen, 3);
            if history == History::FromJoin {
                // Every epoch since generation 2 began, the graces too.
                let got: Vec<u32> = late.missed.iter().map(|k| k.epoch).collect();
                let want: Vec<u32> = epochs.iter().map(|e| e.event.epoch).collect();
                assert_eq!(got, want, "{scheme}");
            } else {
                assert!(late.missed.is_empty());
                walk_chains(w.held.get_mut(&mid(9)).unwrap(), &w.published);
            }
            for e in &epochs {
                assert_eq!(w.held[&mid(9)].key(e.event.epoch), Some(&e.key), "{scheme} {history:?}");
            }
        }
    }
}

#[test]
fn an_emergency_during_the_grace_drops_both_generations() {
    for scheme in TREES {
        let mut w = World::new(any_tree(scheme, closed(), 6, 8));
        for i in 0..20 {
            w.join(mid(i));
        }
        w.rotate(false).unwrap();
        let t = Thresholds { capacity_ratio: 0.0, grace_secs: 10_000, ..Thresholds::default() };
        w.k.migrate_if_needed(&t, &master(7), w.now).unwrap().unwrap();
        for i in 0..10 {
            w.bundle_for(mid(i));
        }
        let grace = w.rotate(true).unwrap();
        assert_eq!(grace.event.trees.len(), 2);
        let g = w.k.emergency_regenerate(master(8), w.now).unwrap();
        assert_eq!((g.gen, g.previous), (3, Some(2)));
        let first = w.publish(g.epoch.unwrap());
        assert_eq!(first.event.trees.iter().map(|t| t.gen).collect::<Vec<_>>(), vec![3]);
        assert_eq!(w.k.stats().live_generations, 1);
        assert_eq!(w.k.stats().grace_until, None);
        for i in 0..20 {
            assert!(!w.opens(&mid(i), &first), "{scheme}: an old bundle of either generation opens nothing");
            assert_eq!(w.held[&mid(i)].status(&first.event), Status::NeedBundle { gen: 3 });
        }
        // Neither earlier master comes back.
        assert_eq!(w.k.emergency_regenerate(master(7), w.now).unwrap_err(), Error::ReusedMaster);
        for i in 0..20 {
            w.bundle_for(mid(i));
        }
        let next = w.rotate(true).unwrap();
        for i in 0..20 {
            assert!(w.opens(&mid(i), &next));
        }
    }
}

#[test]
fn a_master_used_before_is_refused() {
    for scheme in TREES {
        let mut k = any_tree(scheme, closed(), 6, 8);
        k.join(mid(1), 0).unwrap();
        let e = k.built(0, false);
        k.published(e.event.epoch);
        // The master of the current generation (0x4d).
        assert_eq!(k.migrate(master(0x4d), None, 0).unwrap_err(), Error::ReusedMaster);
        assert_eq!(k.emergency_regenerate(master(0x4d), 0).unwrap_err(), Error::ReusedMaster);
        k.migrate(master(7), None, 0).unwrap();
        assert_eq!(k.stats().generation, 2);
        // The grace ends at the next rotation; then 0x4d is retired.
        let e = k.built(30 * 24 * 3600, true);
        k.published(e.event.epoch);
        assert_eq!(k.stats().live_generations, 1);
        assert_eq!(k.tree_state().retired_masters.len(), 1);
        assert_eq!(k.migrate(master(0x4d), None, 0).unwrap_err(), Error::ReusedMaster);
        assert_eq!(k.migrate(master(7), None, 0).unwrap_err(), Error::ReusedMaster);
        assert_eq!(k.emergency_regenerate(master(7), 0).unwrap_err(), Error::ReusedMaster);
        let g = k.emergency_regenerate(master(8), 0).unwrap();
        assert_eq!(g.gen, 3);
        assert_eq!(k.migrate(master(7), None, 0).unwrap_err(), Error::ReusedMaster);
        // A refusal changes nothing; the state still checks.
        AnyKeeper::restore(scheme, k.tree_state().clone()).unwrap();
        // Masters derived for each generation are all different.
        let root = RootSecret::from_bytes([1; 32]);
        let masters: BTreeSet<[u8; 32]> =
            (1..50).map(|g| *GenerationMaster::derive(&root, &scope(), scheme, g).as_bytes()).collect();
        assert_eq!(masters.len(), 49);
    }
}

#[test]
fn an_event_of_only_an_older_generation_after_an_emergency_is_refused() {
    for scheme in TREES {
        let mut w = World::new(any_tree(scheme, closed(), 6, 8));
        for i in 0..10 {
            w.join(mid(i));
        }
        w.rotate(false).unwrap();
        // An epoch built under the old master and never published: what a
        // thief of the old master and the old keeper key could still sign.
        let stolen = w.k.built(w.now, true);
        let g = w.k.emergency_regenerate(master(9), w.now).unwrap();
        w.publish(g.epoch.unwrap());
        let iss = w.bundle_for(mid(1));
        assert_eq!(iss.kind, GenerationKind::Emergency);
        let st = w.held.get_mut(&mid(1)).unwrap();
        assert_eq!(st.emergency_gen, Some(2));
        assert_eq!(st.status(&stolen.event), Status::Out);
        let no = keydist::member::shard_of(&Bundle::from_bytes(&iss.bundle).unwrap(), &stolen.event);
        let (s, wit) = (stolen.shard(1, no.unwrap_or(0)).unwrap(), stolen.witness(1, no.unwrap_or(0)).unwrap());
        assert_eq!(
            st.open(&stolen.event, &s.bytes, &wit.bytes),
            Err(Error::Invalid("event of a generation before an emergency"))
        );
        // Nor a bundle of the old generation.
        let mut old = iss.clone();
        old.gen = 1;
        old.bundle = Default::default();
        old.kind = GenerationKind::Initial;
        assert!(st.accept(&old).is_err());
    }
}

#[test]
fn another_key_for_a_held_epoch_is_a_conflict() {
    for scheme in TREES {
        let mut w = World::new(any_tree(scheme, closed(), 6, 8));
        for i in 0..10 {
            w.join(mid(i));
        }
        w.rotate(false).unwrap();
        let saved = w.k.tree_state().clone();
        let honest = w.rotate(true).unwrap();
        // The keeper is put back to the earlier state (what the contract
        // forbids) and builds the same epoch number with another key.
        let mut rolled = AnyKeeper::restore(scheme, saved).unwrap();
        let replay = rolled.built(w.now, true);
        assert_eq!(replay.event.epoch, honest.event.epoch);
        assert_ne!(replay.key, honest.key);
        let st = w.held.get_mut(&mid(3)).unwrap();
        assert_eq!(st.key(honest.event.epoch), Some(&honest.key));
        let b = st.bundles[0].clone();
        let no = keydist::member::shard_of(&b, &replay.event).unwrap();
        let (s, wit) = (&replay.shard(1, no).unwrap().bytes, &replay.witness(1, no).unwrap().bytes);
        assert_eq!(open_epoch(&b, &replay.event, s, wit).unwrap(), replay.key);
        assert_eq!(st.open(&replay.event, s, wit), Err(Error::Conflict));
        assert_eq!(st.key(honest.event.epoch), Some(&honest.key), "the held key stays");
        // The same through an issued key, and the same key is fine.
        let forged = rolled.bundle_for(mid(3), w.now).unwrap();
        assert_eq!(forged.current.as_ref(), Some(&replay.key));
        assert_eq!(st.accept(&forged), Err(Error::Conflict));
        assert_eq!(st.keep(honest.key.clone()), Ok(()));
        // Pruning drops the old keys only.
        st.prune(honest.event.epoch);
        assert!(st.key(honest.event.epoch - 1).is_none());
        assert!(st.key(honest.event.epoch).is_some());
    }
}

#[test]
fn a_state_of_another_scheme_is_refused_without_a_pending_epoch() {
    for scheme in TREES {
        let mut k = any_tree(scheme, closed(), 6, 8);
        k.join(mid(1), 0).unwrap();
        let e = k.built(0, false);
        assert!(k.published(e.event.epoch));
        assert!(k.pending().is_none());
        let other = if scheme == SchemeId::CsV1 { SchemeId::SdV1 } else { SchemeId::CsV1 };
        let state = k.tree_state().clone();
        assert!(matches!(AnyKeeper::restore(other, state.clone()), Err(Error::Invalid(_))));
        assert!(AnyKeeper::restore(scheme, state).is_ok());
    }
}

#[test]
fn bundles_and_events_of_another_scope_are_refused() {
    for scheme in TREES {
        let other = ScopeRef { community: [0xc0; 32], scope: [0x77; 32] };
        let params = TreeParams { depth: 6, leaves_per_shard: 8 };
        let mut a = any_tree(scheme, closed(), 6, 8);
        let mut b = match scheme {
            SchemeId::CsV1 => AnyKeeper::Cs(keydist::CsKeeper::new(other, closed(), params, master(0x4d), 0).unwrap()),
            _ => AnyKeeper::Sd(keydist::SdKeeper::new(other, closed(), params, master(0x4d), 0).unwrap()),
        };
        let ia = a.join(mid(1), 0).unwrap();
        let ib = b.join(mid(1), 0).unwrap();
        let (ea, eb) = (a.built(0, false), b.built(0, false));
        let ba = Bundle::from_bytes(&ia.bundle).unwrap();
        let bb = Bundle::from_bytes(&ib.bundle).unwrap();
        let (sa, wa) = (&ea.shard(1, 0).unwrap().bytes, &ea.witness(1, 0).unwrap().bytes);
        let (sb, wb) = (&eb.shard(1, 0).unwrap().bytes, &eb.witness(1, 0).unwrap().bytes);
        assert!(open_epoch(&ba, &ea.event, sa, wa).is_ok());
        assert!(matches!(open_epoch(&ba, &eb.event, sb, wb), Err(Error::Invalid(_))));
        assert!(matches!(open_epoch(&bb, &ea.event, sa, wa), Err(Error::Invalid(_))));
        // The shard and witness of the other scope under this scope's event.
        assert!(matches!(open_epoch(&ba, &ea.event, sb, wb), Err(Error::Invalid(_))));
        let mut st = MemberState::new(scope());
        st.accept(&ia).unwrap();
        assert!(matches!(st.accept(&ib), Err(Error::Invalid(_))));
        assert_eq!(st.status(&eb.event), Status::Out);
        assert!(matches!(st.open(&eb.event, sb, wb), Err(Error::Invalid(_))));
        assert!(st.open(&ea.event, sa, wa).is_ok());
    }
}

#[test]
fn recover_repeats_no_leaf_and_no_epoch_number() {
    for scheme in TREES {
        let mut w = World::new(any_tree(scheme, closed(), 6, 8));
        for i in 0..30 {
            w.join(mid(i));
        }
        w.rotate(false).unwrap();
        let t = Thresholds { capacity_ratio: 0.0, grace_secs: 0, ..Thresholds::default() };
        w.k.migrate_if_needed(&t, &master(7), w.now).unwrap().unwrap();
        for i in 0..30 {
            w.bundle_for(mid(i));
        }
        w.k.revoke(&[mid(4)], Revoke::Ban, w.now);
        let last = w.rotate(true).unwrap();
        // The keeper is lost. The relays show `last`; the journal the
        // members.
        let min_gen = last.event.newest_gen().unwrap();
        let members: Vec<MemberId> = (0..30).filter(|&i| i != 4).map(mid).collect();
        let root = RootSecret::from_bytes([0x42; 32]);
        let m = GenerationMaster::derive(&root, &scope(), scheme, min_gen + 1);
        let policy = closed();
        let params = TreeParams { depth: 6, leaves_per_shard: 8 };
        let (k, g) = match scheme {
            SchemeId::CsV1 => {
                let (k, g) = keydist::CsKeeper::recover(scope(), policy, params, &members, min_gen, last.event.epoch, m, w.now).unwrap();
                (AnyKeeper::Cs(k), g)
            }
            _ => {
                let (k, g) = keydist::SdKeeper::recover(scope(), policy, params, &members, min_gen, last.event.epoch, m, w.now).unwrap();
                (AnyKeeper::Sd(k), g)
            }
        };
        assert_eq!((g.gen, g.kind.clone()), (min_gen + 1, GenerationKind::Emergency));
        let old_leaves: BTreeSet<(u32, u32)> =
            w.held.values().flat_map(|st| st.bundles.iter().map(|b| (b.gen, b.leaf))).collect();
        w.k = k;
        let first = w.publish(g.epoch.unwrap());
        assert!(first.event.epoch > last.event.epoch);
        assert_eq!(first.event.prev_epoch, None, "nothing of the lost history is chained");
        assert!(w.published.iter().filter(|e| e.event.epoch == first.event.epoch).count() == 1);
        // Members rebundle: new leaves of a new generation.
        for &m in &members {
            assert_eq!(w.held[&m].status(&first.event), Status::NeedBundle { gen: min_gen + 1 });
            let iss = w.bundle_for(m);
            assert!(!old_leaves.contains(&(iss.gen, iss.leaf)));
            assert_eq!(iss.current.as_ref(), Some(&first.key));
        }
        assert_eq!(w.k.bundle_for(mid(4), w.now), Err(Error::UnknownMember));
        let next = w.rotate(true).unwrap();
        for &m in &members {
            assert!(w.opens(&m, &next));
        }
        assert!(!w.opens(&mid(4), &next));
        let numbers: Vec<u32> = w.published.iter().map(|e| e.event.epoch).collect();
        assert!(numbers.windows(2).all(|p| p[0] < p[1]), "{numbers:?}");
        // The change log of a recovered keeper starts with everything.
        let changes = w.k.take_changes();
        let replayed = KeeperState::replay(changes.clone());
        assert!(replayed.is_ok());
    }
}

#[test]
fn the_least_interval_holds_a_stream_of_bans() {
    for scheme in TREES {
        let p = Policy::new(60, 600, false, History::Open)
            .with_thresholds(Thresholds { revoked_ratio: 0.9, ..Thresholds::default() });
        assert_eq!(p.min_epoch_interval_secs, 300);
        let mut w = World::new(any_tree(scheme, p, 9, 32));
        for i in 0..400 {
            w.join(mid(i));
        }
        let t0 = w.now;
        w.rotate(false).unwrap();
        let mut built_at = vec![t0];
        let mut banned = Vec::new();
        for s in 1..=300u64 {
            w.now = t0 + 10 * s;
            let m = mid(s);
            w.k.revoke(&[m], Revoke::Ban, w.now);
            banned.push(m);
            if let Some(out) = w.rotate(false) {
                built_at.push(w.now);
                for b in &banned {
                    assert!(!w.opens(b, &out));
                }
            }
        }
        // 3000 s of bans every 10 s: an epoch every 300 s, not every 60.
        assert!(built_at.windows(2).all(|p| p[1] - p[0] >= 300), "{built_at:?}");
        assert!(built_at.len() <= 11, "{} epochs", built_at.len());
        // What waits is due at once after the interval; urgent goes first.
        let last = *built_at.last().unwrap();
        w.k.revoke(&[mid(399)], Revoke::Ban, w.now);
        if let Rotate::NotDue { due_at } = w.k.rotate(w.now, false) {
            assert!(due_at.unwrap() >= last + 300);
            assert_eq!(w.k.next_wake(), due_at);
        }
        assert!(w.rotate(true).is_some(), "urgent does not wait for the interval");
    }
}

#[test]
fn supersede_pending_burns_the_number_and_bans_at_once() {
    for scheme in TREES {
        let mut w = World::new(any_tree(scheme, closed(), 8, 16));
        for i in 0..100 {
            w.join(mid(i));
        }
        let e1 = w.rotate(false).unwrap();
        assert!(w.k.supersede_pending(w.now).is_none(), "nothing pending");
        w.k.revoke(&[mid(1)], Revoke::Ban, w.now);
        w.now += 120;
        let slow = w.k.built(w.now, false);
        assert_eq!(w.k.stats().pending_epoch, Some(slow.event.epoch));
        assert_eq!(w.k.stats().pending_blobs as usize, slow.blobs.len());
        // The upload of `slow` drags; an urgent ban comes.
        w.k.revoke(&[mid(2)], Revoke::Ban, w.now);
        assert!(matches!(w.k.rotate(w.now, true), Rotate::Blocked { pending_epoch } if pending_epoch == slow.event.epoch));
        let fast = w.k.supersede_pending(w.now).unwrap();
        assert_eq!(fast.event.epoch, slow.event.epoch + 1);
        assert_eq!(fast.event.prev_epoch, Some(e1.event.epoch), "the burnt epoch is not chained");
        for h in e1.blob_hashes().iter().chain(slow.blob_hashes().iter()) {
            assert!(fast.retires.contains(h));
        }
        assert!(!w.k.published(slow.event.epoch));
        assert_eq!(w.k.pending(), Some(&*fast));
        w.publish(fast.clone());
        assert!(!w.opens(&mid(1), &fast) && !w.opens(&mid(2), &fast));
        assert!(w.opens(&mid(3), &fast));
        let st = w.held.get_mut(&mid(3)).unwrap();
        assert_eq!(st.follow_chain(&fast.event).unwrap(), e1.key);
        assert!(st.key(slow.event.epoch).is_none());
        // Flat too.
        let mut f = flat(closed());
        f.join(mid(1), 0).unwrap();
        let a = f.rotate(0, false).built().unwrap();
        let b = f.supersede_pending(0).unwrap();
        assert_eq!(b.event.epoch, a.event.epoch + 1);
        assert!(!f.published(a.event.epoch) && f.published(b.event.epoch));
    }
}

#[test]
fn missed_keys_come_in_pages_and_go_once_everyone_migrated() {
    for scheme in TREES {
        let mut w = World::new(any_tree(scheme, policy(120, 600, false, History::FromJoin), 5, 8));
        for i in 0..10 {
            w.join(mid(i));
        }
        w.rotate(false).unwrap();
        let t = Thresholds { capacity_ratio: 0.0, grace_secs: 0, ..Thresholds::default() };
        w.k.migrate_if_needed(&t, &master(7), w.now).unwrap().unwrap();
        for i in 0..9 {
            w.bundle_for(mid(i));
        }
        let n = MISSED_PAGE + 100;
        let mut epochs = Vec::new();
        for _ in 0..n {
            epochs.push(w.rotate(true).unwrap());
        }
        assert_eq!(w.k.stats().missed_len as usize, n);
        assert_eq!(w.k.stats().migrated, 9);
        let late = w.k.bundle_for(mid(9), w.now).unwrap();
        assert_eq!(late.missed.len(), MISSED_PAGE);
        assert!(late.missed_more);
        let rest = w.k.missed_page(&mid(9), late.missed.last().unwrap().epoch, MISSED_PAGE).unwrap();
        assert_eq!(rest.missed.len(), 100);
        assert!(!rest.missed_more && rest.bundle.is_empty() && rest.current.is_none());
        // A full page with the largest bundle (sd-v1, depth 24) fits the
        // 64 KiB plaintext of a NIP-44 envelope.
        assert!(late.to_bytes().len() - late.bundle.len() + 9707 < 65_535);
        w.take(mid(9), &late);
        for e in &epochs {
            assert_eq!(w.held[&mid(9)].key(e.event.epoch), Some(&e.key));
        }
        // Everybody migrated: the next publication drops the keys.
        assert_eq!(w.k.stats().unmigrated, 0);
        w.rotate(true).unwrap();
        assert_eq!(w.k.stats().missed_len, 0);
        // A member without a bundle of the current generation gets no page.
        assert!(matches!(w.k.missed_page(&mid(100), 0, 10), Err(Error::UnknownMember)));
    }
}

#[test]
fn status_says_read_rebundle_or_out() {
    for scheme in TREES {
        let mut w = World::new(any_tree(scheme, closed(), 6, 4));
        for i in 0..8 {
            w.join(mid(i));
        }
        let e1 = w.rotate(false).unwrap();
        assert_eq!(w.held[&mid(5)].status(&e1.event), Status::Read { gen: 1, shard: 1 });
        // A newcomer past the shards of the epoch: nothing to read in it.
        w.join(mid(8));
        assert_eq!(w.held[&mid(8)].status(&e1.event), Status::Out);
        let t = Thresholds { capacity_ratio: 0.0, grace_secs: 0, ..Thresholds::default() };
        w.k.migrate_if_needed(&t, &master(7), w.now).unwrap().unwrap();
        let e2 = w.rotate(true).unwrap();
        assert_eq!(e2.event.trees.len(), 1);
        assert_eq!(w.held[&mid(5)].status(&e2.event), Status::NeedBundle { gen: 2 });
        w.bundle_for(mid(5));
        // Its new leaf came after e2 was built: nothing for it there.
        assert_eq!(w.held[&mid(5)].status(&e2.event), Status::Out);
        let e3 = w.rotate(true).unwrap();
        assert_eq!(w.held[&mid(5)].status(&e3.event), Status::Read { gen: 2, shard: 0 });
        assert!(w.opens(&mid(5), &e3));
        // A member's state survives JSON.
        let st = &w.held[&mid(5)];
        let back: MemberState = serde_json::from_str(&serde_json::to_string(st).unwrap()).unwrap();
        assert_eq!(&back, st);
    }
}

#[test]
fn a_migration_may_deepen_the_tree() {
    for scheme in TREES {
        let mut w = World::new(any_tree(scheme, closed(), 5, 8));
        for i in 0..30 {
            w.join(mid(i));
        }
        w.rotate(false).unwrap();
        assert_eq!(w.k.migration_due(&Thresholds::default()), Some(keydist::MigrationReason::Capacity));
        assert_eq!(w.k.next_wake(), Some(w.now), "a reached threshold wakes the host");
        let deeper = TreeParams { depth: 9, leaves_per_shard: 16 };
        let g = w.k.migrate(master(7), Some(deeper), w.now).unwrap();
        assert!(matches!(g.kind, GenerationKind::Planned { reason: keydist::MigrationReason::Capacity, .. }));
        for i in 0..15 {
            assert_eq!(Bundle::from_bytes(&w.bundle_for(mid(i)).bundle).unwrap().tree.depth(), 9);
        }
        let grace = w.rotate(true).unwrap();
        assert_eq!(grace.event.trees.iter().map(|t| (t.depth, t.leaves)).collect::<Vec<_>>(), vec![(5, 8), (9, 16)]);
        for i in 0..30 {
            assert!(w.opens(&mid(i), &grace));
        }
        assert_eq!(w.k.stats().capacity, 512);
        let bad = TreeParams { depth: 9, leaves_per_shard: 3 };
        assert!(matches!(w.k.migrate(master(8), Some(bad), w.now), Err(Error::State(_))));
    }
}

#[test]
fn stats_tell_a_line_of_ui() {
    let mut w = World::new(any_tree(SchemeId::CsV1, closed(), 6, 8));
    for i in 0..10 {
        w.join(mid(i));
    }
    let out = w.k.built(w.now, false);
    let s = w.k.stats();
    assert_eq!((s.pending_epoch, s.published_epoch), (Some(out.event.epoch), None));
    assert_eq!(s.last_upload_bytes as usize, out.upload_bytes());
    assert!(s.last_p95.is_some());
    w.publish(out.clone());
    let t = Thresholds { capacity_ratio: 0.0, grace_secs: 500, ..Thresholds::default() };
    w.k.migrate_if_needed(&t, &master(7), w.now).unwrap().unwrap();
    w.bundle_for(mid(0));
    let s = w.k.stats();
    assert_eq!((s.pending_epoch, s.published_epoch), (None, Some(out.event.epoch)));
    assert_eq!(s.grace_until, Some(w.now + 500));
    assert_eq!(s.next_wake, Some(w.now + 500));
    assert_eq!((s.migrated, s.unmigrated), (1, 9));
    assert!((s.migrated_share() - 0.1).abs() < 1e-9);
    let json = serde_json::to_string(&s).unwrap();
    assert!(json.contains("\"next_wake\""));
}

#[test]
fn issued_goes_over_the_wire() {
    let mut k = any_tree(SchemeId::SdV1, policy(60, 60, false, History::FromJoin), 6, 8);
    let iss = k.join(mid(1), 0).unwrap();
    let bytes = iss.to_bytes();
    assert_eq!(Issued::from_bytes(&bytes).unwrap(), iss);
    assert!(format!("{iss:?}").contains("SecretBytes("));
    let mut st = MemberState::new(scope());
    st.accept(&Issued::from_bytes(&bytes).unwrap()).unwrap();
    assert_eq!(st.bundles.len(), 1);
    // Outbound borrows: the event is last, every blob is named.
    let out = k.built(0, false);
    let ob = out.outbound();
    assert!(matches!(ob.last(), Some(keydist::Outbound::Event(_))));
    let kinds: BTreeSet<_> = ob
        .iter()
        .filter_map(|o| match o {
            keydist::Outbound::Blob { kind, .. } => Some(*kind),
            _ => None,
        })
        .collect();
    assert_eq!(kinds, [keydist::BlobKind::Shard, keydist::BlobKind::Index].into());
    // A verified event serializes as the event itself.
    let v = serde_json::to_string(&out.event).unwrap();
    let plain = serde_json::to_string(&*out.event).unwrap();
    assert_eq!(v, plain);
    let _: Verified<keydist::EpochEvent> = Verified::after_signature_check(serde_json::from_str(&v).unwrap());
}
