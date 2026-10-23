// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `flat-v1`: one secret per member per epoch, for the small closed scopes
//! inside a community. No tree, no bundle, no shard: an epoch is the event
//! plus one secret for every member, which the host seals to that
//! member's public key (NIP-44). It costs O(N) per epoch and proves that
//! the interface fits the small scopes too. It does not replace the key
//! rotation of today's groups (19.4).
//!
//! A secret (version 1): magic `VKF1` ‖ version ‖ community (32) ‖ scope
//! (32) ‖ epoch (u32 LE) ‖ key (32), 105 bytes. Its confidentiality and
//! its author are the envelope's; the crate only checks that it names the
//! event's scope and epoch.
//!
//! The state is small and saved whole (`FlatState`, version 2); it is a
//! secret (it holds the newest epoch keys) and never rolled back.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::epoch::{self, EpochEvent, EpochOut, EpochRecipe};
use crate::generation::{GenerationKind, GenerationOut, MigrationReason, Thresholds};
use crate::kdf::GenerationMaster;
use crate::keeper::{KeeperStats, Policy, Schedule, TreeParams};
use crate::rng::{OsRng, Rng};
use crate::{
    EpochKey, Error, History, Issued, Joined, Key32, KeyDistribution, MemberId, Revoke, Rotate, SchemeId,
    ScopeRef, SecretBytes, Verified,
};

/// The magic of a secret.
pub const FLAT_MAGIC: &[u8; 4] = b"VKF1";
/// The bytes of a secret.
pub const FLAT_SECRET_LEN: usize = 105;
/// The version of the saved state.
pub const FLAT_STATE_VERSION: u8 = 2;

/// The secret of one member for one epoch.
pub fn secret_bytes(scope: &ScopeRef, k: &EpochKey) -> SecretBytes {
    let mut out = SecretBytes::with_capacity(FLAT_SECRET_LEN);
    out.extend(FLAT_MAGIC);
    out.push(1);
    out.extend(&scope.community);
    out.extend(&scope.scope);
    out.extend(&k.epoch.to_le_bytes());
    out.extend(&k.key.0);
    out
}

/// Opens the secret of an epoch (after the host opened its envelope).
pub fn open_secret(ev: &Verified<EpochEvent>, b: &[u8]) -> Result<EpochKey, Error> {
    if b.len() != FLAT_SECRET_LEN || &b[..4] != FLAT_MAGIC || b[4] != 1 {
        return Err(Error::Invalid("flat secret"));
    }
    if ev.scheme != SchemeId::FlatV1 || b[5..37] != ev.community_id || b[37..69] != ev.scope_id {
        return Err(Error::Invalid("flat secret of another scope"));
    }
    let epoch = u32::from_le_bytes(b[69..73].try_into().expect("4 bytes"));
    if epoch != ev.epoch {
        return Err(Error::Invalid("flat secret of another epoch"));
    }
    Ok(EpochKey { epoch, key: Key32(b[73..].try_into().expect("32 bytes")) })
}

/// What a flat keeper knows of a member.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FlatRec {
    pub since_epoch: u32,
    pub ordinal: u32,
}

/// The saved state of a `flat-v1` keeper. A secret, never rolled back.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FlatState {
    pub v: u8,
    pub scope: ScopeRef,
    pub policy: Policy,
    pub members: BTreeMap<MemberId, FlatRec>,
    pub next_ordinal: u32,
    pub schedule: Schedule,
    pub next_epoch: u32,
    pub head: Option<EpochKey>,
    pub published_head: Option<EpochKey>,
    pub last_epoch_at: Option<u64>,
    pub pending: Option<EpochRecipe>,
    pub regenerations: u32,
}

/// A `flat-v1` keeper.
pub struct FlatKeeper {
    state: FlatState,
    rng: Box<dyn Rng>,
    built: Option<Arc<EpochOut>>,
}

impl FlatKeeper {
    /// A new scope on the system random source.
    pub fn new(scope: ScopeRef, policy: Policy) -> Self {
        let state = FlatState {
            v: FLAT_STATE_VERSION,
            scope,
            policy,
            members: BTreeMap::new(),
            next_ordinal: 0,
            schedule: Schedule::default(),
            next_epoch: 1,
            head: None,
            published_head: None,
            last_epoch_at: None,
            pending: None,
            regenerations: 0,
        };
        FlatKeeper { state, rng: Box::new(OsRng), built: None }
    }

    /// The same keeper on another random source (the crate's tests, the
    /// simulator, the benchmark; not in a production build).
    #[cfg(any(test, feature = "tools"))]
    pub fn with_rng(mut self, rng: Box<dyn Rng>) -> Self {
        self.rng = rng;
        self
    }

    /// A keeper from a saved state, on the system random source; the
    /// pending epoch is built again.
    pub fn from_state(state: FlatState) -> Result<Self, Error> {
        if state.v != FLAT_STATE_VERSION {
            return Err(Error::Invalid("keeper state version"));
        }
        let epochs_ok = match (&state.head, &state.published_head) {
            (None, Some(_)) => false,
            (Some(h), p) => state.next_epoch > h.epoch && p.as_ref().is_none_or(|p| p.epoch <= h.epoch),
            (None, None) => true,
        };
        let pending_ok = match &state.pending {
            Some(r) => r.scheme == SchemeId::FlatV1 && state.head.as_ref() == Some(&r.key),
            None => state.head == state.published_head,
        };
        if !epochs_ok || !pending_ok || state.members.values().any(|r| r.ordinal >= state.next_ordinal) {
            return Err(Error::Invalid("keeper state"));
        }
        let built = match &state.pending {
            Some(r) => Some(Arc::new(Self::build(&state.scope, r)?)),
            None => None,
        };
        Ok(FlatKeeper { state, rng: Box::new(OsRng), built })
    }

    /// The state to save.
    pub fn state(&self) -> &FlatState {
        &self.state
    }

    fn build(scope: &ScopeRef, r: &EpochRecipe) -> Result<EpochOut, Error> {
        epoch::build(scope, r, &|_| None, &|_| secret_bytes(scope, &r.key))
    }

    fn kind(&self) -> GenerationKind {
        if self.state.regenerations == 0 {
            GenerationKind::Initial
        } else {
            GenerationKind::Emergency
        }
    }

    fn issue(&self, member: MemberId) -> Result<Issued, Error> {
        let rec = self.state.members.get(&member).ok_or(Error::UnknownMember)?;
        let current = self.state.head.clone().filter(|h| h.epoch >= rec.since_epoch);
        Ok(Issued {
            scheme: SchemeId::FlatV1,
            scope: self.state.scope,
            gen: self.state.regenerations,
            leaf: rec.ordinal,
            kind: self.kind(),
            bundle: SecretBytes::default(),
            current,
            missed: Vec::new(),
            missed_more: false,
        })
    }

    fn build_next(&mut self, now: u64) -> Arc<EpochOut> {
        let epoch = self.state.next_epoch;
        self.state.next_epoch = epoch.checked_add(1).expect("epoch numbers run out after 4e9 epochs");
        let mut k = [0u8; 32];
        self.rng.fill(&mut k);
        let key = EpochKey { epoch, key: Key32(k) };
        let prev = self.state.published_head.clone();
        let chain = match (&prev, self.state.policy.history) {
            (Some(p), History::Open) => {
                let mut nonce = [0u8; 12];
                self.rng.fill(&mut nonce);
                Some(epoch::seal_chain(&key, p, &nonce, &self.state.scope))
            }
            _ => None,
        };
        let recipe = EpochRecipe {
            scheme: SchemeId::FlatV1,
            key: key.clone(),
            seed: Key32([0; 32]),
            prev_epoch: prev.map(|p| p.epoch),
            at: now,
            history: self.state.policy.history,
            chain,
            trees: Vec::new(),
            recipients: self.state.members.keys().copied().collect(),
            retires: Vec::new(),
        };
        let out = Arc::new(Self::build(&self.state.scope, &recipe).expect("a flat recipe builds"));
        self.state.head = Some(key);
        self.state.last_epoch_at = Some(now);
        self.state.pending = Some(recipe);
        self.state.schedule = Schedule::default();
        self.built = Some(out.clone());
        out
    }

    fn drop_pending(&mut self) {
        self.state.pending = None;
        self.built = None;
        self.state.head = self.state.published_head.clone();
    }

    fn due_at(&self) -> Option<u64> {
        self.state.schedule.due(self.state.last_epoch_at, self.state.policy.min_epoch_interval_secs)
    }
}

impl KeyDistribution for FlatKeeper {
    fn scheme(&self) -> SchemeId {
        SchemeId::FlatV1
    }

    fn scope(&self) -> &ScopeRef {
        &self.state.scope
    }

    fn join(&mut self, member: MemberId, now: u64) -> Result<Joined, Error> {
        if !self.state.members.contains_key(&member) {
            let ordinal = self.state.next_ordinal;
            let next = ordinal.checked_add(1).ok_or(Error::GenerationFull)?;
            let since = match self.state.policy.history {
                History::Open => 0,
                History::FromJoin => {
                    self.state.schedule.joined_waiting(&self.state.policy, now);
                    self.state.next_epoch
                }
            };
            self.state.next_ordinal = next;
            self.state.members.insert(member, FlatRec { since_epoch: since, ordinal });
        }
        self.issue(member)
    }

    fn revoke(&mut self, members: &[MemberId], why: Revoke, now: u64) {
        for m in members {
            if self.state.members.remove(m).is_some() {
                self.state.schedule.revoked(why, &self.state.policy, now);
            }
        }
    }

    fn rotate(&mut self, now: u64, urgent: bool) -> Rotate {
        if let Some(r) = &self.state.pending {
            return Rotate::Blocked { pending_epoch: r.key.epoch };
        }
        let first = self.state.head.is_none() && !self.state.members.is_empty();
        let due = self.due_at();
        if urgent || first || due.is_some_and(|d| d <= now) {
            return Rotate::Built(self.build_next(now));
        }
        Rotate::NotDue { due_at: due }
    }

    fn pending(&self) -> Option<&EpochOut> {
        self.built.as_deref()
    }

    fn published(&mut self, epoch: u32) -> bool {
        if self.state.pending.as_ref().is_none_or(|r| r.key.epoch != epoch) {
            return false;
        }
        self.state.pending = None;
        self.state.published_head = self.state.head.clone();
        self.built = None;
        true
    }

    fn supersede_pending(&mut self, now: u64) -> Option<Arc<EpochOut>> {
        self.state.pending.as_ref()?;
        self.drop_pending();
        Some(self.build_next(now))
    }

    /// A flat scope has no tree to outgrow.
    fn migration_due(&self, _t: &Thresholds) -> Option<MigrationReason> {
        None
    }

    fn migrate(&mut self, _next: GenerationMaster, _p: Option<TreeParams>, _now: u64) -> Result<GenerationOut, Error> {
        Err(Error::State("flat-v1 has no generations to migrate"))
    }

    /// A flat scope has no master: an emergency drops the unpublished
    /// epoch and sends a new key to every member at once.
    fn emergency_regenerate(&mut self, _new_master: GenerationMaster, now: u64) -> Result<GenerationOut, Error> {
        self.drop_pending();
        self.state.regenerations += 1;
        let epoch = self.build_next(now);
        Ok(GenerationOut {
            gen: self.state.regenerations,
            previous: Some(self.state.regenerations - 1),
            kind: GenerationKind::Emergency,
            epoch: Some(epoch),
        })
    }

    fn bundle_for(&mut self, member: MemberId, _now: u64) -> Result<Issued, Error> {
        self.issue(member)
    }

    fn missed_page(&self, member: &MemberId, _after_epoch: u32, _limit: usize) -> Result<Issued, Error> {
        let mut i = self.issue(*member)?;
        i.current = None;
        Ok(i)
    }

    fn is_member(&self, member: &MemberId) -> bool {
        self.state.members.contains_key(member)
    }

    fn next_wake(&self) -> Option<u64> {
        if self.state.head.is_none() && !self.state.members.is_empty() {
            // The first epoch: at once.
            return Some(0);
        }
        self.due_at()
    }

    fn stats(&self) -> KeeperStats {
        let members = self.state.members.len() as u64;
        KeeperStats {
            scheme: SchemeId::FlatV1,
            members,
            revoked: 0,
            next_leaf: u64::from(self.state.next_ordinal),
            capacity: u64::from(u32::MAX),
            generation: self.state.regenerations,
            live_generations: 1,
            epoch: self.state.head.as_ref().map_or(0, |h| h.epoch),
            unmigrated: 0,
            migrated: members,
            schedule: self.state.schedule.clone(),
            pending_epoch: self.state.pending.as_ref().map(|r| r.key.epoch),
            pending_blobs: 0,
            published_epoch: self.state.published_head.as_ref().map(|h| h.epoch),
            grace_until: None,
            last_p95: None,
            missed_len: 0,
            last_upload_bytes: self.built.as_ref().map_or(0, |b| b.secrets.iter().map(|(_, s)| s.len() as u64).sum()),
            next_wake: self.next_wake(),
        }
    }
}
