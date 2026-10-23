// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A small world for the scenario tests: one keeper of either scheme,
//! the bundles and keys every identity ever got, the published epochs.
//!
//! The keepers run on the system random source (a production build has
//! no other); the tests draw their own choices from `TestRng`.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use keydist::{
    open_epoch, open_secret, CsKeeper, EpochKey, EpochOut, Error, FlatKeeper, GenerationMaster, GenerationOut,
    Issued, KeeperState, KeeperStats, KeyDistribution, MemberId, MemberState, MigrationReason, Policy, Revoke,
    Rotate, SchemeId, ScopeRef, SdKeeper, Thresholds, TreeParams,
};

/// The tree schemes, for the scenarios that run on both.
pub const TREES: [SchemeId; 2] = [SchemeId::CsV1, SchemeId::SdV1];

/// A seeded xorshift64* for the tests' own choices (the crate's
/// `XorShift64` exists only under its feature `tools`).
pub struct TestRng(u64);

impl TestRng {
    pub fn new(seed: u64) -> Self {
        TestRng(if seed == 0 { 0x9e37_79b9_7f4a_7c15 } else { seed })
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

pub fn scope() -> ScopeRef {
    ScopeRef { community: [0xc0; 32], scope: [0x5c; 32] }
}

pub fn mid(i: u64) -> MemberId {
    let mut b = [0u8; 32];
    b[..8].copy_from_slice(&i.to_le_bytes());
    b[31] = 0xee;
    MemberId(b)
}

/// A master from one byte.
pub fn master(b: u8) -> GenerationMaster {
    GenerationMaster::from_bytes([b; 32])
}

/// The policy of most scenarios: the windows given, no least interval
/// between epochs.
pub fn policy(ban: u64, leave: u64, open_scope: bool, history: keydist::History) -> Policy {
    Policy::new(ban, leave, open_scope, history).with_min_epoch_interval(0)
}

/// A closed scope with open history, bans in 120 s, leaves in 600 s, no
/// least interval.
pub fn closed() -> Policy {
    policy(120, 600, false, keydist::History::Open)
}

pub fn cs(policy: Policy, depth: u8, lps: u32) -> CsKeeper {
    CsKeeper::new(scope(), policy, TreeParams { depth, leaves_per_shard: lps }, master(0x4d), 0).unwrap()
}

pub fn sd(policy: Policy, depth: u8, lps: u32) -> SdKeeper {
    SdKeeper::new(scope(), policy, TreeParams { depth, leaves_per_shard: lps }, master(0x4d), 0).unwrap()
}

pub fn flat(policy: Policy) -> FlatKeeper {
    FlatKeeper::new(scope(), policy)
}

pub fn any_cs(policy: Policy, depth: u8, lps: u32) -> AnyKeeper {
    AnyKeeper::Cs(cs(policy, depth, lps))
}

pub fn any_sd(policy: Policy, depth: u8, lps: u32) -> AnyKeeper {
    AnyKeeper::Sd(sd(policy, depth, lps))
}

pub fn any_flat(policy: Policy) -> AnyKeeper {
    AnyKeeper::Flat(flat(policy))
}

/// A keeper of a tree scheme.
pub fn any_tree(scheme: SchemeId, policy: Policy, depth: u8, lps: u32) -> AnyKeeper {
    match scheme {
        SchemeId::CsV1 => any_cs(policy, depth, lps),
        SchemeId::SdV1 => any_sd(policy, depth, lps),
        SchemeId::FlatV1 => panic!("flat-v1 has no tree"),
    }
}

/// A keeper of any scheme that can crash: its state goes through JSON
/// and it comes back on the system random source.
pub enum AnyKeeper {
    Cs(CsKeeper),
    Sd(SdKeeper),
    Flat(FlatKeeper),
}

macro_rules! each {
    ($self:expr, $k:ident => $e:expr) => {
        match $self {
            AnyKeeper::Cs($k) => $e,
            AnyKeeper::Sd($k) => $e,
            AnyKeeper::Flat($k) => $e,
        }
    };
}

impl AnyKeeper {
    /// The state of a tree keeper.
    pub fn tree_state(&self) -> &KeeperState {
        match self {
            AnyKeeper::Cs(k) => k.state(),
            AnyKeeper::Sd(k) => k.state(),
            AnyKeeper::Flat(_) => panic!("flat-v1 has no tree"),
        }
    }

    /// The change log of a tree keeper.
    pub fn take_changes(&mut self) -> Vec<keydist::Change> {
        match self {
            AnyKeeper::Cs(k) => k.take_changes(),
            AnyKeeper::Sd(k) => k.take_changes(),
            AnyKeeper::Flat(_) => panic!("flat-v1 has no change log"),
        }
    }

    /// A tree keeper restored from a state.
    pub fn restore(scheme: SchemeId, state: KeeperState) -> Result<AnyKeeper, Error> {
        Ok(match scheme {
            SchemeId::CsV1 => AnyKeeper::Cs(CsKeeper::from_state(state)?),
            SchemeId::SdV1 => AnyKeeper::Sd(SdKeeper::from_state(state)?),
            SchemeId::FlatV1 => panic!("flat-v1 has no tree"),
        })
    }

    pub fn crash(&mut self) {
        *self = match self {
            AnyKeeper::Cs(k) => {
                let saved = serde_json::to_string(k.state()).unwrap();
                AnyKeeper::Cs(CsKeeper::from_state(serde_json::from_str(&saved).unwrap()).unwrap())
            }
            AnyKeeper::Sd(k) => {
                let saved = serde_json::to_string(k.state()).unwrap();
                AnyKeeper::Sd(SdKeeper::from_state(serde_json::from_str(&saved).unwrap()).unwrap())
            }
            AnyKeeper::Flat(k) => {
                let saved = serde_json::to_string(k.state()).unwrap();
                AnyKeeper::Flat(FlatKeeper::from_state(serde_json::from_str(&saved).unwrap()).unwrap())
            }
        };
    }

    /// Rotates and expects an epoch.
    pub fn built(&mut self, now: u64, urgent: bool) -> Arc<EpochOut> {
        match self.rotate(now, urgent) {
            Rotate::Built(out) => out,
            other => panic!("no epoch built: {other:?}"),
        }
    }
}

impl KeyDistribution for AnyKeeper {
    fn scheme(&self) -> SchemeId {
        each!(self, k => k.scheme())
    }
    fn scope(&self) -> &ScopeRef {
        each!(self, k => k.scope())
    }
    fn join(&mut self, m: MemberId, now: u64) -> Result<Issued, Error> {
        each!(self, k => k.join(m, now))
    }
    fn revoke(&mut self, m: &[MemberId], why: Revoke, now: u64) {
        each!(self, k => k.revoke(m, why, now))
    }
    fn rotate(&mut self, now: u64, urgent: bool) -> Rotate {
        each!(self, k => k.rotate(now, urgent))
    }
    fn pending(&self) -> Option<&EpochOut> {
        each!(self, k => k.pending())
    }
    fn published(&mut self, epoch: u32) -> bool {
        each!(self, k => k.published(epoch))
    }
    fn supersede_pending(&mut self, now: u64) -> Option<Arc<EpochOut>> {
        each!(self, k => k.supersede_pending(now))
    }
    fn migration_due(&self, t: &Thresholds) -> Option<MigrationReason> {
        each!(self, k => k.migration_due(t))
    }
    fn migrate(&mut self, m: GenerationMaster, p: Option<TreeParams>, now: u64) -> Result<GenerationOut, Error> {
        each!(self, k => k.migrate(m, p, now))
    }
    fn migrate_if_needed(&mut self, t: &Thresholds, m: &GenerationMaster, now: u64) -> Result<Option<GenerationOut>, Error> {
        each!(self, k => k.migrate_if_needed(t, m, now))
    }
    fn emergency_regenerate(&mut self, m: GenerationMaster, now: u64) -> Result<GenerationOut, Error> {
        each!(self, k => k.emergency_regenerate(m, now))
    }
    fn bundle_for(&mut self, m: MemberId, now: u64) -> Result<Issued, Error> {
        each!(self, k => k.bundle_for(m, now))
    }
    fn missed_page(&self, m: &MemberId, after: u32, limit: usize) -> Result<Issued, Error> {
        each!(self, k => k.missed_page(m, after, limit))
    }
    fn is_member(&self, m: &MemberId) -> bool {
        each!(self, k => k.is_member(m))
    }
    fn next_wake(&self) -> Option<u64> {
        each!(self, k => k.next_wake())
    }
    fn stats(&self) -> KeeperStats {
        each!(self, k => k.stats())
    }
}

/// Tries to open an epoch as `who` with what `st` holds: the shard of
/// each bundle generation the event carries (`cs-v1`, `sd-v1`) or its
/// secret (`flat-v1`).
pub fn try_open(st: &MemberState, out: &EpochOut, who: &MemberId) -> Result<EpochKey, Error> {
    if out.event.scheme == SchemeId::FlatV1 {
        let (_, bytes) = out.secrets.iter().find(|(m, _)| m == who).ok_or(Error::NotCovered)?;
        return open_secret(&out.event, bytes);
    }
    let mut last = Err(Error::NotCovered);
    for b in st.bundles.iter().rev() {
        let Some(no) = keydist::member::shard_of(b, &out.event) else { continue };
        let shard = out.shard(b.gen, no).expect("the keeper built every shard");
        let witness = out.witness(b.gen, no).expect("the keeper built every witness");
        last = open_epoch(b, &out.event, &shard.bytes, &witness.bytes);
        if last.is_ok() {
            return last;
        }
    }
    last
}

/// Takes every page of missed keys after an `Issued` that has more.
pub fn all_pages(k: &AnyKeeper, m: &MemberId, first: &Issued) -> Vec<Issued> {
    let mut pages = Vec::new();
    let mut last = first.clone();
    while last.missed_more {
        let after = last.missed.last().expect("a page with more has keys").epoch;
        last = k.missed_page(m, after, keydist::issued::MISSED_PAGE).unwrap();
        pages.push(last.clone());
    }
    pages
}

pub struct World {
    pub k: AnyKeeper,
    pub held: BTreeMap<MemberId, MemberState>,
    pub published: Vec<Arc<EpochOut>>,
    pub now: u64,
}

impl World {
    pub fn new(k: AnyKeeper) -> Self {
        World { k, held: BTreeMap::new(), published: Vec::new(), now: 1000 }
    }

    /// What a member takes; the issued goes through its byte form, and
    /// every further page of missed keys is fetched.
    pub fn take(&mut self, m: MemberId, issued: &Issued) {
        let wire = Issued::from_bytes(&issued.to_bytes()).unwrap();
        assert_eq!(&wire, issued);
        let pages = all_pages(&self.k, &m, issued);
        let st = self.held.entry(m).or_insert_with(|| MemberState::new(scope()));
        st.accept(&wire).unwrap();
        for p in pages {
            st.accept(&p).unwrap();
        }
    }

    pub fn join(&mut self, m: MemberId) -> Issued {
        let i = self.k.join(m, self.now).unwrap();
        self.take(m, &i);
        i
    }

    pub fn bundle_for(&mut self, m: MemberId) -> Issued {
        let i = self.k.bundle_for(m, self.now).unwrap();
        self.take(m, &i);
        i
    }

    /// Publishes an epoch: the host acknowledges it, and every holder
    /// that can open it keeps its key.
    pub fn publish(&mut self, out: Arc<EpochOut>) -> Arc<EpochOut> {
        assert!(self.k.published(out.event.epoch));
        let keys: Vec<(MemberId, EpochKey)> = self
            .held
            .iter()
            .filter_map(|(m, st)| try_open(st, &out, m).ok().map(|k| (*m, k)))
            .collect();
        for (m, k) in keys {
            self.held.get_mut(&m).unwrap().keep(k).unwrap();
        }
        self.published.push(out.clone());
        out
    }

    /// Rotates and publishes at once.
    pub fn rotate(&mut self, urgent: bool) -> Option<Arc<EpochOut>> {
        let out = self.k.rotate(self.now, urgent).built()?;
        Some(self.publish(out))
    }

    pub fn opens(&self, m: &MemberId, out: &EpochOut) -> bool {
        match self.held.get(m) {
            Some(st) => match try_open(st, out, m) {
                Ok(k) => {
                    assert_eq!(k, out.key, "a block opened to a wrong key");
                    true
                }
                Err(_) => false,
            },
            None => false,
        }
    }
}
