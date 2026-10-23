// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The keeper of a tree scope (`cs-v1` or `sd-v1`): who is in, which leaf
//! each member has, which leaves are out, when the next epoch is due, and
//! the epoch built but not yet published.
//!
//! The state is sparse: a map member → leaf, the revoked leaves of each
//! live generation, counters and the recipe of the pending epoch. A tree
//! of 16.7 million leaves is never built; node keys and labels are derived
//! when a bundle or a block needs them.
//!
//! The logic is one for both tree schemes, `TreeKeeper<S>`; a `Scheme`
//! gives what differs, the cover of a generation and the bundle of a leaf.
//! The blocks are sealed by `epoch::build` and opened by
//! `member::open_epoch`, both by the scheme the recipe or the event names.
//! `CsKeeper` and `SdKeeper` are the two keepers.
//!
//! The contract with the host (specification 6, 19.2): after every call
//! that changes the state, save first — the rows of `take_changes()`, or
//! the whole `state()` for a small scope — then send what the call
//! returned. The saved state is a secret (it holds the masters of the live
//! generations) and is never put back to an earlier version: an older
//! state would hand out leaves and epoch numbers a second time. `join` is
//! idempotent; an epoch, once built, is handed out with the same bytes by
//! `pending()` until `published(epoch)`, also after the state went through
//! serialization (`from_state`); its number is never given to another
//! epoch. The state names its scheme, and `from_state` refuses a state of
//! another scheme or one that breaks an invariant (`KeeperState::check`).
//!
//! A lost keeper is not restored from nothing: `recover` starts an
//! emergency generation above the last generation and epoch the relays
//! show, so that no master, leaf or epoch number comes back.

use std::collections::{BTreeMap, BTreeSet};
use std::marker::PhantomData;
use std::sync::Arc;

use crate::bundle::Bundle;
use crate::changes::Change;
use crate::epoch::{self, EpochOut, EpochRecipe, TreeRecipe};
use crate::generation::{GenState, GenerationKind, GenerationOut, MigrationReason, Thresholds};
use crate::hexser;
use crate::issued::MISSED_PAGE;
use crate::kdf::GenerationMaster;
use crate::rng::{OsRng, Rng};
use crate::tree::Tree;
use crate::{
    EpochKey, Error, History, Issued, Joined, Key32, KeyDistribution, MemberId, Revoke, Rotate, SchemeId,
    ScopeRef, SecretBytes,
};

/// What a tree scheme gives the shared keeper: the cover of a generation
/// as a recipe and the bundle of a leaf.
pub trait Scheme: Send + Sync + 'static {
    /// The name of the scheme.
    const ID: SchemeId;
    /// The cover of the active leaves of a generation as a recipe.
    fn recipe(g: &GenState) -> TreeRecipe;
    /// The bundle of a leaf of a generation.
    fn bundle(g: &GenState, scope: &ScopeRef, leaf: u32) -> Bundle;
}

/// Complete Subtree (`cs-v1`).
#[derive(Clone, Copy, Debug, Default)]
pub struct CsScheme;

impl Scheme for CsScheme {
    const ID: SchemeId = SchemeId::CsV1;

    fn recipe(g: &GenState) -> TreeRecipe {
        TreeRecipe::new(g.gen, g.tree, g.leaves_per_shard, u64::from(g.next_leaf), &g.cover())
    }

    fn bundle(g: &GenState, scope: &ScopeRef, leaf: u32) -> Bundle {
        Bundle::issue(&g.node_keys(scope), *scope, g.gen, g.tree, leaf)
    }
}

/// Subset Difference (`sd-v1`).
#[derive(Clone, Copy, Debug, Default)]
pub struct SdScheme;

impl Scheme for SdScheme {
    const ID: SchemeId = SchemeId::SdV1;

    fn recipe(g: &GenState) -> TreeRecipe {
        TreeRecipe::new_sd(g.gen, g.tree, g.leaves_per_shard, u64::from(g.next_leaf), &g.sd_cover())
    }

    fn bundle(g: &GenState, scope: &ScopeRef, leaf: u32) -> Bundle {
        Bundle::issue_sd(&g.sd_labels(scope), *scope, g.gen, g.tree, leaf)
    }
}

/// The version of the saved state.
pub const STATE_VERSION: u8 = 2;

/// The least time between two epochs by default, seconds.
pub const MIN_EPOCH_INTERVAL_SECS: u64 = 300;

/// How a scope reacts to bans, leaves and joins.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Policy {
    /// Bans within this window from the first one go into one epoch.
    pub ban_window_secs: u64,
    /// Leaves of a closed scope within this window go into one epoch.
    pub leave_window_secs: u64,
    /// Under strict `from_join`, a newcomer is activated by an epoch at
    /// most this long after its join.
    pub join_window_secs: u64,
    /// No epoch is built sooner than this after the previous one, whatever
    /// waits in the queue, unless the host asks for it urgently (19.2,
    /// 12): it bounds the upload of a keeper under a stream of bans.
    pub min_epoch_interval_secs: u64,
    /// An open scope (anyone with the link may join): a leave makes no
    /// epoch of its own, the next epoch for any reason leaves it out.
    pub open_scope: bool,
    pub history: History,
    /// The thresholds of a planned migration (`next_wake`) and its grace
    /// (`migrate`).
    pub thresholds: Thresholds,
}

impl Policy {
    /// A policy; the join window is the ban window, the least interval
    /// between epochs `MIN_EPOCH_INTERVAL_SECS`, the thresholds the
    /// defaults.
    pub fn new(ban_window_secs: u64, leave_window_secs: u64, open_scope: bool, history: History) -> Self {
        Policy {
            ban_window_secs,
            leave_window_secs,
            join_window_secs: ban_window_secs,
            min_epoch_interval_secs: MIN_EPOCH_INTERVAL_SECS,
            open_scope,
            history,
            thresholds: Thresholds::default(),
        }
    }

    /// The defaults for an open scope (19.2, 12): bans in five minutes,
    /// leaves lazy, newcomers of `from_join` in five minutes, open history.
    pub fn open() -> Self {
        Policy::new(300, 3600, true, History::Open)
    }

    /// The defaults for a closed scope: bans in five minutes, leaves in an
    /// hour, open history.
    pub fn closed() -> Self {
        Policy::new(300, 3600, false, History::Open)
    }

    /// The same policy with another least interval between epochs.
    pub fn with_min_epoch_interval(mut self, secs: u64) -> Self {
        self.min_epoch_interval_secs = secs;
        self
    }

    /// The same policy with another join window.
    pub fn with_join_window(mut self, secs: u64) -> Self {
        self.join_window_secs = secs;
        self
    }

    /// The same policy with other thresholds.
    pub fn with_thresholds(mut self, t: Thresholds) -> Self {
        self.thresholds = t;
        self
    }
}

impl Default for Policy {
    fn default() -> Self {
        Policy::closed()
    }
}

/// The shape of the trees of a scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TreeParams {
    pub depth: u8,
    /// Leaves per shard, a power of two not above the capacity.
    pub leaves_per_shard: u32,
}

impl TreeParams {
    /// Checks the parameters.
    pub fn validate(&self) -> Result<Tree, Error> {
        let tree = Tree::new(self.depth)?;
        let w = u64::from(self.leaves_per_shard);
        if !w.is_power_of_two() || w > tree.capacity() {
            return Err(Error::Params("leaves per shard"));
        }
        Ok(tree)
    }
}

impl Default for TreeParams {
    /// Depth 24 (16.7 million leaves) and 4096 leaves a shard (19.2, 11).
    fn default() -> Self {
        TreeParams { depth: 24, leaves_per_shard: 4096 }
    }
}

/// When the next epoch is due. Shared by every scheme.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Schedule {
    /// The earliest moment a queued change wants an epoch.
    pub due_at: Option<u64>,
    /// Bans and closed leaves waiting for the next epoch.
    pub queued: u32,
    /// Leaves of an open scope waiting for the next epoch (no due time).
    pub lazy: u32,
    /// Newcomers of a strict `from_join` waiting for their first epoch.
    pub waiting: u32,
}

impl Schedule {
    fn want(&mut self, at: u64) {
        self.due_at = Some(self.due_at.map_or(at, |d| d.min(at)));
    }

    /// Notes a revocation.
    pub fn revoked(&mut self, why: Revoke, policy: &Policy, now: u64) {
        match why {
            Revoke::Ban => {
                self.queued += 1;
                self.want(now.saturating_add(policy.ban_window_secs));
            }
            Revoke::Leave if policy.open_scope => self.lazy += 1,
            Revoke::Leave => {
                self.queued += 1;
                self.want(now.saturating_add(policy.leave_window_secs));
            }
        }
    }

    /// Notes a newcomer that waits for its first epoch.
    pub fn joined_waiting(&mut self, policy: &Policy, now: u64) {
        self.waiting += 1;
        self.want(now.saturating_add(policy.join_window_secs));
    }

    /// When the queued changes may have their epoch: the due time, not
    /// sooner than `min_interval` after the previous epoch.
    pub fn due(&self, last_epoch_at: Option<u64>, min_interval: u64) -> Option<u64> {
        let floor = last_epoch_at.map_or(0, |t| t.saturating_add(min_interval));
        self.due_at.map(|d| d.max(floor))
    }
}

/// A member's leaf in a generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GenLeaf {
    pub gen: u32,
    pub leaf: u32,
}

/// What the keeper knows of a member.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MemberRec {
    /// The first epoch the member may read (0: every epoch, open
    /// history).
    pub since_epoch: u32,
    /// Its leaves in at most two generations (the two live during a
    /// migration). A slot may name a generation no longer live.
    pub slots: [Option<GenLeaf>; 2],
}

impl MemberRec {
    /// The leaf in a generation.
    pub fn leaf_in(&self, gen: u32) -> Option<u32> {
        self.slots.iter().flatten().find(|s| s.gen == gen).map(|s| s.leaf)
    }

    /// Records a leaf in a generation, over the slot of the oldest
    /// generation.
    pub fn put(&mut self, gen: u32, leaf: u32) {
        let at = match self.slots {
            [None, _] => 0,
            [_, None] => 1,
            [Some(a), Some(b)] => usize::from(b.gen < a.gen),
        };
        self.slots[at] = Some(GenLeaf { gen, leaf });
    }
}

/// The saved state of a tree keeper (`cs-v1` or `sd-v1`, named by
/// `scheme`).
///
/// It is a secret: it holds the masters of the live generations and the
/// keys of the newest epochs; the host keeps it where it keeps keys. It is
/// never rolled back: putting an earlier state back would hand out leaves
/// and epoch numbers a second time. A large scope saves it as rows
/// (`TreeKeeper::take_changes`, `KeeperState::apply`), a small one may
/// serialize it whole.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct KeeperState {
    pub v: u8,
    pub scheme: SchemeId,
    pub scope: ScopeRef,
    pub policy: Policy,
    /// The shape of the current generation's tree.
    pub params: TreeParams,
    pub members: BTreeMap<MemberId, MemberRec>,
    /// The live generations, oldest first; the last is the current one.
    /// Two only during the grace of a planned migration.
    pub gens: Vec<GenState>,
    /// The fingerprints of the masters of the generations dropped
    /// (`GenerationMaster::fingerprint`): a master is never taken twice.
    #[serde(with = "hexser::list")]
    pub retired_masters: Vec<[u8; 32]>,
    pub schedule: Schedule,
    /// The number the next epoch takes. Advanced when an epoch is built.
    pub next_epoch: u32,
    /// The newest epoch built (pending or published).
    pub head: Option<EpochKey>,
    /// The newest epoch published.
    pub published_head: Option<EpochKey>,
    /// When the newest epoch was built.
    pub last_epoch_at: Option<u64>,
    /// The epoch built and not yet published.
    pub pending: Option<EpochRecipe>,
    /// The hashes of the blobs of the newest published epoch: the next
    /// epoch retires them.
    #[serde(with = "hexser::list")]
    pub published_blobs: Vec<[u8; 32]>,
    /// `from_join` only: keys of the epochs published while some member
    /// had no leaf in the current generation, the first generation
    /// excepted; handed to a member with its new bundle, in pages. Dropped
    /// at the first publication after every member migrated.
    pub missed: Vec<EpochKey>,
    /// Members without a leaf in the current generation.
    pub unmigrated: u64,
    /// The 95th percentile of the shard size of the last epoch, current
    /// generation.
    pub last_p95: Option<usize>,
    /// The bytes of all blobs of the last epoch built.
    pub last_upload: u64,
}

impl KeeperState {
    /// The current generation.
    pub fn current(&self) -> Option<&GenState> {
        self.gens.last()
    }

    /// Checks the invariants of a state of `scheme` (19.2, 6): the
    /// version and the scheme; the epoch numbers (`next_epoch > head ≥
    /// published head`, the pending epoch is the head, without one the
    /// head is published); one or two generations, ascending, with sound
    /// trees and masters never seen twice; revoked leaves and members'
    /// leaves below `next_leaf`, members' leaves unique; the pending
    /// recipe of this scheme and of live generations; `unmigrated` the
    /// count of members without a leaf in the current generation.
    pub fn check(&self, scheme: SchemeId) -> Result<(), Error> {
        let bad = |what| Err(Error::Invalid(what));
        if self.v != STATE_VERSION {
            return bad("keeper state version");
        }
        if self.scheme != scheme {
            return bad("keeper state of another scheme");
        }
        self.params.validate()?;
        let Some(cur) = self.gens.last() else { return bad("keeper state without a generation") };
        if self.gens.len() > 2 {
            return bad("more than two live generations");
        }
        if self.gens.windows(2).any(|w| w[0].gen >= w[1].gen) {
            return bad("generations out of order");
        }
        if self.gens.len() == 2 && cur.grace_until().is_none() {
            return bad("two generations without a grace");
        }
        if cur.tree != self.params.validate()? || cur.leaves_per_shard != self.params.leaves_per_shard {
            return bad("parameters of the current generation");
        }
        let mut fps: BTreeSet<[u8; 32]> = self.retired_masters.iter().copied().collect();
        for g in &self.gens {
            TreeParams { depth: g.tree.depth(), leaves_per_shard: g.leaves_per_shard }.validate()?;
            if u64::from(g.next_leaf) > g.tree.capacity() {
                return bad("next leaf");
            }
            if g.revoked.last().is_some_and(|&l| l >= g.next_leaf) {
                return bad("a revoked leaf never handed out");
            }
            if !fps.insert(g.master.fingerprint()) {
                return bad("a master used twice");
            }
        }
        match (&self.head, &self.published_head) {
            (None, Some(_)) => return bad("published head without a head"),
            (Some(h), p) => {
                if self.next_epoch <= h.epoch || p.as_ref().is_some_and(|p| p.epoch > h.epoch) {
                    return bad("epoch numbers");
                }
            }
            (None, None) => {}
        }
        match &self.pending {
            Some(r) => {
                if r.scheme != scheme {
                    return bad("pending epoch of another scheme");
                }
                if self.head.as_ref() != Some(&r.key) {
                    return bad("pending epoch is not the head");
                }
                if r.trees.iter().any(|t| !self.gens.iter().any(|g| g.gen == t.gen)) {
                    return bad("pending epoch of a dropped generation");
                }
            }
            None => {
                if self.head != self.published_head {
                    return bad("an unpublished head without a pending epoch");
                }
            }
        }
        if self.missed.windows(2).any(|w| w[0].epoch >= w[1].epoch)
            || self.missed.last().is_some_and(|k| k.epoch >= self.next_epoch)
        {
            return bad("missed keys");
        }
        let mut seen: Vec<Vec<u64>> =
            self.gens.iter().map(|g| vec![0u64; (g.next_leaf as usize).div_ceil(64)]).collect();
        let mut unmigrated = 0u64;
        for rec in self.members.values() {
            if rec.leaf_in(cur.gen).is_none() {
                unmigrated += 1;
            }
            for (g, bits) in self.gens.iter().zip(seen.iter_mut()) {
                let Some(leaf) = rec.leaf_in(g.gen) else { continue };
                if leaf >= g.next_leaf || g.revoked.contains(&leaf) {
                    return bad("a member's leaf");
                }
                let (w, b) = (leaf as usize / 64, leaf % 64);
                if bits[w] >> b & 1 == 1 {
                    return bad("a leaf of two members");
                }
                bits[w] |= 1 << b;
            }
        }
        if unmigrated != self.unmigrated {
            return bad("count of unmigrated members");
        }
        Ok(())
    }
}

/// Counters for the host, the simulator, the thresholds and a line of UI.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct KeeperStats {
    pub scheme: SchemeId,
    pub members: u64,
    /// Revoked leaves of the current generation.
    pub revoked: u64,
    pub next_leaf: u64,
    pub capacity: u64,
    pub generation: u32,
    pub live_generations: u32,
    /// The newest epoch built, 0 before the first.
    pub epoch: u32,
    /// Members without a leaf in the current generation, and with one.
    pub unmigrated: u64,
    pub migrated: u64,
    pub schedule: Schedule,
    /// The epoch built and not yet published, with its number of blobs.
    pub pending_epoch: Option<u32>,
    pub pending_blobs: u32,
    /// The newest epoch published.
    pub published_epoch: Option<u32>,
    /// The end of the grace of a running migration.
    pub grace_until: Option<u64>,
    pub last_p95: Option<usize>,
    /// Keys kept for late members (`from_join`).
    pub missed_len: u64,
    pub last_upload_bytes: u64,
    pub next_wake: Option<u64>,
}

impl KeeperStats {
    /// The share of the members that hold a leaf of the current
    /// generation (1 when there are none).
    pub fn migrated_share(&self) -> f64 {
        if self.members == 0 {
            1.0
        } else {
            self.migrated as f64 / self.members as f64
        }
    }
}

/// What changed since the last `take_changes`, coalesced: a flag for the
/// header and the pending epoch, the members touched, the revoked leaves
/// and the missed keys in order.
#[derive(Default)]
struct Log {
    off: bool,
    header: bool,
    members: BTreeSet<MemberId>,
    revoked: Vec<(u32, u32)>,
    missed: Vec<Change>,
    pending: bool,
    blobs: bool,
}

/// A keeper of a tree scheme: the state and a random source.
pub struct TreeKeeper<S: Scheme> {
    state: KeeperState,
    rng: Box<dyn Rng>,
    built: Option<Arc<EpochOut>>,
    log: Log,
    scheme: PhantomData<S>,
}

/// A `cs-v1` keeper.
pub type CsKeeper = TreeKeeper<CsScheme>;

/// An `sd-v1` keeper.
pub type SdKeeper = TreeKeeper<SdScheme>;

impl<S: Scheme> TreeKeeper<S> {
    fn with_state(state: KeeperState) -> Self {
        TreeKeeper { state, rng: Box::new(OsRng), built: None, log: Log::default(), scheme: PhantomData }
    }

    /// A new scope with its first generation under `master` (generation
    /// 1), on the system random source.
    pub fn new(
        scope: ScopeRef,
        policy: Policy,
        params: TreeParams,
        master: GenerationMaster,
        now: u64,
    ) -> Result<Self, Error> {
        let tree = params.validate()?;
        let gen = GenState::new(1, tree, params.leaves_per_shard, master, GenerationKind::Initial, now);
        let mut k = Self::with_state(Self::empty_state(scope, policy, params, gen, 1));
        k.log.header = true;
        Ok(k)
    }

    fn empty_state(scope: ScopeRef, policy: Policy, params: TreeParams, gen: GenState, next_epoch: u32) -> KeeperState {
        KeeperState {
            v: STATE_VERSION,
            scheme: S::ID,
            scope,
            policy,
            params,
            members: BTreeMap::new(),
            gens: vec![gen],
            retired_masters: Vec::new(),
            schedule: Schedule::default(),
            next_epoch,
            head: None,
            published_head: None,
            last_epoch_at: None,
            pending: None,
            published_blobs: Vec::new(),
            missed: Vec::new(),
            unmigrated: 0,
            last_p95: None,
            last_upload: 0,
        }
    }

    /// Recovers the keeper of a scope whose keeper was lost (19.2, 9):
    /// the members from the journal of the community, `min_gen` and
    /// `min_epoch` from the newest epoch event on the relays (a host may
    /// add a margin). Always starts an emergency generation `min_gen + 1`
    /// under `master` with epochs from `min_epoch + 1`, and builds its
    /// first epoch (pending; members read it once they have the new
    /// bundle). Nothing of the lost keeper's history is chained.
    #[allow(clippy::too_many_arguments)]
    pub fn recover(
        scope: ScopeRef,
        policy: Policy,
        params: TreeParams,
        members: &[MemberId],
        min_gen: u32,
        min_epoch: u32,
        master: GenerationMaster,
        now: u64,
    ) -> Result<(Self, GenerationOut), Error> {
        let tree = params.validate()?;
        let gen = min_gen.checked_add(1).ok_or(Error::Params("generation number"))?;
        let next_epoch = min_epoch.checked_add(1).ok_or(Error::Params("epoch number"))?;
        let g = GenState::new(gen, tree, params.leaves_per_shard, master, GenerationKind::Emergency, now);
        let mut state = Self::empty_state(scope, policy, params, g, next_epoch);
        for m in members {
            state.members.insert(*m, MemberRec { since_epoch: 0, slots: [None, None] });
        }
        state.unmigrated = state.members.len() as u64;
        let mut k = Self::with_state(state);
        k.log.header = true;
        k.log.members = k.state.members.keys().copied().collect();
        let out = k.build_next(now, Vec::new());
        let previous = (min_gen > 0).then_some(min_gen);
        Ok((k, GenerationOut { gen, previous, kind: GenerationKind::Emergency, epoch: Some(out) }))
    }

    /// The same keeper on another random source (the crate's tests, the
    /// simulator, the benchmark; not in a production build).
    #[cfg(any(test, feature = "tools"))]
    pub fn with_rng(mut self, rng: Box<dyn Rng>) -> Self {
        self.rng = rng;
        self
    }

    /// A keeper from a saved state, on the system random source; the
    /// pending epoch, if any, is built again from its recipe, byte for
    /// byte. Refuses a state of another scheme or one that breaks an
    /// invariant (`KeeperState::check`).
    pub fn from_state(state: KeeperState) -> Result<Self, Error> {
        state.check(S::ID)?;
        let mut k = Self::with_state(state);
        if let Some(recipe) = &k.state.pending {
            k.built = Some(Arc::new(k.build(recipe)?));
        }
        Ok(k)
    }

    /// The state to save.
    pub fn state(&self) -> &KeeperState {
        &self.state
    }

    /// The state, giving up the keeper.
    pub fn into_state(self) -> KeeperState {
        self.state
    }

    /// What changed since the last call, to be written (as rows) before
    /// what the calls returned is sent (19.2, 7). Coalesced: one header,
    /// one entry per member touched, the revoked leaves and missed keys in
    /// order, the pending epoch as it is now. A new keeper's first call
    /// gives its header; `recover` gives every member too.
    /// `KeeperState::apply` replays them.
    pub fn take_changes(&mut self) -> Vec<Change> {
        let off = self.log.off;
        let log = std::mem::replace(&mut self.log, Log { off, ..Log::default() });
        let mut out = Vec::new();
        if log.header {
            out.push(Change::Header(self.state.header()));
        }
        for m in log.members {
            out.push(match self.state.members.get(&m) {
                Some(rec) => Change::MemberPut { member: m, rec: rec.clone() },
                None => Change::MemberRemove { member: m },
            });
        }
        out.extend(log.revoked.into_iter().map(|(gen, leaf)| Change::Revoked { gen, leaf }));
        out.extend(log.missed);
        if log.pending {
            out.push(match &self.state.pending {
                Some(r) => Change::Pending(r.clone()),
                None => Change::PendingCleared,
            });
        }
        if log.blobs {
            out.push(Change::PublishedBlobs { hashes: self.state.published_blobs.clone() });
        }
        out
    }

    /// Turns the change log off (a host that saves the whole state, the
    /// simulator) or on again. Off, nothing is recorded and what was is
    /// dropped; turned on again, the log starts from the state as it is,
    /// so the host saves that whole state first.
    pub fn set_change_log(&mut self, on: bool) {
        self.log = Log { off: !on, ..Log::default() };
    }

    fn touch_header(&mut self) {
        self.log.header = !self.log.off;
    }

    fn touch_member(&mut self, m: MemberId) {
        if !self.log.off {
            self.log.members.insert(m);
        }
    }

    /// The current generation.
    pub fn generation(&self) -> &GenState {
        self.state.gens.last().expect("a keeper has a generation")
    }

    fn current_mut(&mut self) -> &mut GenState {
        self.state.gens.last_mut().expect("a keeper has a generation")
    }

    /// A member's leaf in the current generation.
    pub fn leaf_of(&self, member: &MemberId) -> Option<GenLeaf> {
        let gen = self.generation().gen;
        let rec = self.state.members.get(member)?;
        rec.leaf_in(gen).map(|leaf| GenLeaf { gen, leaf })
    }

    fn build(&self, recipe: &EpochRecipe) -> Result<EpochOut, Error> {
        let s = &self.state;
        epoch::build(
            &s.scope,
            recipe,
            &|gen| s.gens.iter().find(|g| g.gen == gen).map(|g| g.node_keys(&s.scope)),
            &|_| SecretBytes::default(),
        )
    }

    fn random_key(&mut self) -> Key32 {
        let mut k = [0u8; 32];
        self.rng.fill(&mut k);
        Key32(k)
    }

    /// Refuses a master this keeper holds or held.
    fn check_master(&self, m: &GenerationMaster) -> Result<(), Error> {
        let fp = m.fingerprint();
        if self.state.retired_masters.contains(&fp) || self.state.gens.iter().any(|g| g.master.fingerprint() == fp) {
            return Err(Error::ReusedMaster);
        }
        Ok(())
    }

    /// Drops the generations before the current one, keeping the
    /// fingerprints of their masters.
    fn retire_old_gens(&mut self) {
        let n = self.state.gens.len() - 1;
        for g in self.state.gens.drain(..n) {
            self.state.retired_masters.push(g.master.fingerprint());
        }
    }

    /// Missed keys of epochs from `since` after `after`, at most `limit`,
    /// and whether more follow.
    fn missed_after(&self, since: u32, after: Option<u32>, limit: usize) -> (Vec<EpochKey>, bool) {
        let mut it = self.state.missed.iter().filter(|k| k.epoch >= since && after.is_none_or(|a| k.epoch > a));
        let page: Vec<EpochKey> = it.by_ref().take(limit).cloned().collect();
        (page, it.next().is_some())
    }

    /// Issues the bundle of an existing member in the current
    /// generation, assigning a leaf there if it has none.
    fn issue(&mut self, member: MemberId) -> Result<Issued, Error> {
        let gen = self.generation().gen;
        let rec = self.state.members.get(&member).ok_or(Error::UnknownMember)?;
        let since = rec.since_epoch;
        let leaf = match rec.leaf_in(gen) {
            Some(l) => l,
            None => {
                let leaf = self.current_mut().assign()?;
                self.state.members.get_mut(&member).expect("checked above").put(gen, leaf);
                self.state.unmigrated = self.state.unmigrated.saturating_sub(1);
                self.touch_member(member);
                self.touch_header();
                leaf
            }
        };
        let g = self.generation();
        let bundle = S::bundle(g, &self.state.scope, leaf).to_bytes();
        let kind = g.kind.clone();
        let current = self.state.head.clone().filter(|h| h.epoch >= since);
        let (missed, missed_more) = self.missed_after(since, None, MISSED_PAGE);
        Ok(Issued { scheme: S::ID, scope: self.state.scope, gen, leaf, kind, bundle, current, missed, missed_more })
    }

    fn end_grace(&mut self, now: u64) {
        if self.state.gens.len() > 1 && self.generation().grace_until().is_some_and(|until| now >= until) {
            self.retire_old_gens();
            self.touch_header();
        }
    }

    /// The moment the queued changes may have their epoch.
    fn due_at(&self) -> Option<u64> {
        self.state.schedule.due(self.state.last_epoch_at, self.state.policy.min_epoch_interval_secs)
    }

    /// Builds the next epoch over the live generations; it becomes the
    /// pending one. `extra_retires`: blobs of an epoch it replaces.
    fn build_next(&mut self, now: u64, extra_retires: Vec<[u8; 32]>) -> Arc<EpochOut> {
        let epoch = self.state.next_epoch;
        self.state.next_epoch = epoch.checked_add(1).expect("epoch numbers run out after 4e9 epochs");
        let key = EpochKey { epoch, key: self.random_key() };
        let seed = self.random_key();
        let prev = self.state.published_head.clone();
        let chain = match (&prev, self.state.policy.history) {
            (Some(p), History::Open) => {
                let mut nonce = [0u8; 12];
                self.rng.fill(&mut nonce);
                Some(epoch::seal_chain(&key, p, &nonce, &self.state.scope))
            }
            _ => None,
        };
        let trees = self.state.gens.iter().map(S::recipe).collect();
        let mut retires = self.state.published_blobs.clone();
        for h in extra_retires {
            if !retires.contains(&h) {
                retires.push(h);
            }
        }
        let recipe = EpochRecipe {
            scheme: S::ID,
            key: key.clone(),
            seed,
            prev_epoch: prev.as_ref().map(|p| p.epoch),
            at: now,
            history: self.state.policy.history,
            chain,
            trees,
            recipients: Vec::new(),
            retires,
        };
        let out = self.build(&recipe).expect("a recipe of live generations builds");
        let cur = self.generation().gen;
        let mut sizes: Vec<usize> = out.shards().filter(|s| s.gen == cur).map(|s| s.bytes.len()).collect();
        sizes.sort_unstable();
        self.state.last_p95 = (!sizes.is_empty()).then(|| sizes[(sizes.len() * 95).div_ceil(100) - 1]);
        self.state.last_upload = out.upload_bytes() as u64;
        self.state.head = Some(key);
        self.state.last_epoch_at = Some(now);
        self.state.pending = Some(recipe);
        self.state.schedule = Schedule::default();
        let out = Arc::new(out);
        self.built = Some(out.clone());
        self.touch_header();
        self.log.pending = !self.log.off;
        out
    }

    /// Drops the pending epoch (its number stays burnt), giving the
    /// hashes of its blobs to retire.
    fn drop_pending(&mut self) -> Vec<[u8; 32]> {
        let mut retire = Vec::new();
        if let Some(r) = self.state.pending.take() {
            retire.extend(r.retires);
        }
        if let Some(b) = self.built.take() {
            retire.extend(b.blob_hashes());
        }
        self.state.head = self.state.published_head.clone();
        self.log.pending = !self.log.off;
        self.touch_header();
        retire
    }

    fn start_planned(
        &mut self,
        master: GenerationMaster,
        new_params: Option<TreeParams>,
        reason: MigrationReason,
        grace_secs: u64,
        now: u64,
    ) -> Result<GenerationOut, Error> {
        if self.state.gens.len() > 1 {
            return Err(Error::State("a migration is running"));
        }
        self.check_master(&master)?;
        let params = new_params.unwrap_or(self.state.params);
        let tree = params.validate()?;
        let gen = self.generation().gen.checked_add(1).ok_or(Error::Params("generation number"))?;
        let kind = GenerationKind::Planned { reason, grace_until: now.saturating_add(grace_secs) };
        self.state.gens.push(GenState::new(gen, tree, params.leaves_per_shard, master, kind.clone(), now));
        self.state.params = params;
        self.state.unmigrated = self.state.members.len() as u64;
        self.touch_header();
        Ok(GenerationOut { gen, previous: Some(gen - 1), kind, epoch: None })
    }
}

impl<S: Scheme> KeyDistribution for TreeKeeper<S> {
    fn scheme(&self) -> SchemeId {
        S::ID
    }

    fn scope(&self) -> &ScopeRef {
        &self.state.scope
    }

    fn join(&mut self, member: MemberId, now: u64) -> Result<Joined, Error> {
        if self.state.members.contains_key(&member) {
            return self.issue(member);
        }
        let leaf = self.current_mut().assign()?;
        let gen = self.generation().gen;
        let since = match self.state.policy.history {
            History::Open => 0,
            History::FromJoin => {
                self.state.schedule.joined_waiting(&self.state.policy, now);
                self.state.next_epoch
            }
        };
        let mut rec = MemberRec { since_epoch: since, slots: [None, None] };
        rec.put(gen, leaf);
        self.state.members.insert(member, rec);
        self.touch_member(member);
        self.touch_header();
        self.issue(member)
    }

    fn revoke(&mut self, members: &[MemberId], why: Revoke, now: u64) {
        let cur = self.generation().gen;
        for m in members {
            let Some(rec) = self.state.members.remove(m) else { continue };
            for g in &mut self.state.gens {
                if let Some(leaf) = rec.leaf_in(g.gen) {
                    g.revoked.insert(leaf);
                    if !self.log.off {
                        self.log.revoked.push((g.gen, leaf));
                    }
                }
            }
            if rec.leaf_in(cur).is_none() {
                self.state.unmigrated = self.state.unmigrated.saturating_sub(1);
            }
            self.state.schedule.revoked(why, &self.state.policy, now);
            self.touch_member(*m);
            self.touch_header();
        }
    }

    fn rotate(&mut self, now: u64, urgent: bool) -> Rotate {
        if let Some(r) = &self.state.pending {
            return Rotate::Blocked { pending_epoch: r.key.epoch };
        }
        self.end_grace(now);
        let first = self.state.head.is_none() && !self.state.members.is_empty();
        let due = self.due_at();
        if urgent || first || due.is_some_and(|d| d <= now) {
            return Rotate::Built(self.build_next(now, Vec::new()));
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
        let built = self.built.take().expect("a pending epoch is built");
        self.state.published_blobs = built.blob_hashes();
        let initial = matches!(self.generation().kind, GenerationKind::Initial);
        if self.state.unmigrated == 0 {
            if !self.state.missed.is_empty() {
                self.state.missed.clear();
                if !self.log.off {
                    self.log.missed.push(Change::MissedCleared);
                }
            }
        } else if self.state.policy.history == History::FromJoin && !initial {
            let k = built.key.clone();
            self.state.missed.push(k.clone());
            if !self.log.off {
                self.log.missed.push(Change::Missed(k));
            }
        }
        self.touch_header();
        self.log.pending = !self.log.off;
        self.log.blobs = !self.log.off;
        true
    }

    fn supersede_pending(&mut self, now: u64) -> Option<Arc<EpochOut>> {
        self.state.pending.as_ref()?;
        let retire = self.drop_pending();
        self.end_grace(now);
        Some(self.build_next(now, retire))
    }

    fn migration_due(&self, t: &Thresholds) -> Option<MigrationReason> {
        if self.state.gens.len() > 1 {
            return None;
        }
        t.reached(self.generation(), self.state.last_p95)
    }

    fn migrate(
        &mut self,
        next_master: GenerationMaster,
        new_params: Option<TreeParams>,
        now: u64,
    ) -> Result<GenerationOut, Error> {
        let t = &self.state.policy.thresholds;
        let reason = self.migration_due(t).unwrap_or(MigrationReason::Requested);
        let grace = t.grace_secs;
        self.start_planned(next_master, new_params, reason, grace, now)
    }

    /// `migrate` when `migration_due(t)`, with the grace of `t`.
    fn migrate_if_needed(
        &mut self,
        t: &Thresholds,
        next_master: &GenerationMaster,
        now: u64,
    ) -> Result<Option<GenerationOut>, Error> {
        let Some(reason) = self.migration_due(t) else { return Ok(None) };
        self.start_planned(next_master.clone(), None, reason, t.grace_secs, now).map(Some)
    }

    fn emergency_regenerate(&mut self, new_master: GenerationMaster, now: u64) -> Result<GenerationOut, Error> {
        self.check_master(&new_master)?;
        // An epoch built under the compromised master is never published.
        let retire = self.drop_pending();
        let prev = self.generation().gen;
        let gen = prev.checked_add(1).ok_or(Error::Params("generation number"))?;
        let tree = self.state.params.validate()?;
        let lps = self.state.params.leaves_per_shard;
        self.retire_old_gens();
        let last = self.state.gens.pop().expect("a keeper has a generation");
        self.state.retired_masters.push(last.master.fingerprint());
        self.state.gens.push(GenState::new(gen, tree, lps, new_master, GenerationKind::Emergency, now));
        self.state.unmigrated = self.state.members.len() as u64;
        let epoch = self.build_next(now, retire);
        Ok(GenerationOut { gen, previous: Some(prev), kind: GenerationKind::Emergency, epoch: Some(epoch) })
    }

    fn bundle_for(&mut self, member: MemberId, _now: u64) -> Result<Issued, Error> {
        self.issue(member)
    }

    fn missed_page(&self, member: &MemberId, after_epoch: u32, limit: usize) -> Result<Issued, Error> {
        let rec = self.state.members.get(member).ok_or(Error::UnknownMember)?;
        let g = self.generation();
        let leaf = rec.leaf_in(g.gen).ok_or(Error::State("no bundle of the current generation yet"))?;
        let (missed, missed_more) = self.missed_after(rec.since_epoch, Some(after_epoch), limit);
        Ok(Issued {
            scheme: S::ID,
            scope: self.state.scope,
            gen: g.gen,
            leaf,
            kind: g.kind.clone(),
            bundle: SecretBytes::default(),
            current: None,
            missed,
            missed_more,
        })
    }

    fn is_member(&self, member: &MemberId) -> bool {
        self.state.members.contains_key(member)
    }

    fn next_wake(&self) -> Option<u64> {
        let g = self.generation();
        let at_once = self.state.last_epoch_at.unwrap_or(g.started_at);
        let mut wake = self.due_at();
        let mut at = |t: u64| wake = Some(wake.map_or(t, |w| w.min(t)));
        if self.state.head.is_none() && !self.state.members.is_empty() {
            at(at_once);
        }
        if self.state.gens.len() > 1 {
            if let Some(until) = g.grace_until() {
                at(until);
            }
        }
        if self.migration_due(&self.state.policy.thresholds).is_some() {
            at(at_once);
        }
        wake
    }

    fn stats(&self) -> KeeperStats {
        let g = self.generation();
        let members = self.state.members.len() as u64;
        KeeperStats {
            scheme: S::ID,
            members,
            revoked: g.revoked.len() as u64,
            next_leaf: u64::from(g.next_leaf),
            capacity: g.tree.capacity(),
            generation: g.gen,
            live_generations: self.state.gens.len() as u32,
            epoch: self.state.head.as_ref().map_or(0, |h| h.epoch),
            unmigrated: self.state.unmigrated,
            migrated: members.saturating_sub(self.state.unmigrated),
            schedule: self.state.schedule.clone(),
            pending_epoch: self.state.pending.as_ref().map(|r| r.key.epoch),
            pending_blobs: self.built.as_ref().map_or(0, |b| b.blobs.len() as u32),
            published_epoch: self.state.published_head.as_ref().map(|h| h.epoch),
            grace_until: if self.state.gens.len() > 1 { g.grace_until() } else { None },
            last_p95: self.state.last_p95,
            missed_len: self.state.missed.len() as u64,
            last_upload_bytes: self.state.last_upload,
            next_wake: self.next_wake(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::XorShift64;

    fn scope() -> ScopeRef {
        ScopeRef { community: [1; 32], scope: [2; 32] }
    }

    fn mid(i: u8) -> MemberId {
        MemberId([i; 32])
    }

    fn keeper() -> CsKeeper {
        let params = TreeParams { depth: 6, leaves_per_shard: 8 };
        CsKeeper::new(scope(), Policy::closed(), params, GenerationMaster::from_bytes([3; 32]), 0)
            .unwrap()
            .with_rng(Box::new(XorShift64::new(1)))
    }

    #[test]
    fn a_broken_state_is_refused() {
        let mut k = keeper();
        for i in 0..5 {
            k.join(mid(i), 0).unwrap();
        }
        k.rotate(0, false).built().unwrap();
        let good = k.state().clone();
        assert!(CsKeeper::from_state(good.clone()).is_ok());
        type Break = Box<dyn Fn(&mut KeeperState)>;
        let breaks: Vec<(&str, Break)> = vec![
            ("version", Box::new(|s| s.v = 1)),
            ("scheme", Box::new(|s| s.scheme = SchemeId::SdV1)),
            ("next epoch", Box::new(|s| s.next_epoch = 1)),
            ("published ahead", Box::new(|s| s.published_head = Some(EpochKey { epoch: 9, key: Key32([0; 32]) }))),
            ("pending not head", Box::new(|s| s.head = None)),
            ("two members, one leaf", Box::new(|s| {
                let rec = s.members[&mid(0)].clone();
                s.members.insert(mid(1), rec);
            })),
            ("leaf beyond next", Box::new(|s| s.gens[0].next_leaf = 2)),
            ("revoked beyond next", Box::new(|s| {
                s.gens[0].revoked.insert(40);
            })),
            ("unmigrated", Box::new(|s| s.unmigrated = 3)),
            ("dropped master", Box::new(|s| {
                let fp = s.gens[0].master.fingerprint();
                s.retired_masters.push(fp);
            })),
            ("pending of a dropped generation", Box::new(|s| s.pending.as_mut().unwrap().trees[0].gen = 7)),
            ("generations out of order", Box::new(|s| {
                let mut g = s.gens[0].clone();
                g.gen = 0;
                g.master = GenerationMaster::from_bytes([9; 32]);
                s.gens.insert(1, g);
            })),
        ];
        for (what, f) in breaks {
            let mut s = good.clone();
            f(&mut s);
            assert!(matches!(CsKeeper::from_state(s), Err(Error::Invalid(_))), "{what}");
        }
        // Another scheme, nothing pending: refused too.
        k.published(1);
        assert!(matches!(SdKeeper::from_state(k.state().clone()), Err(Error::Invalid(_))));
        assert!(CsKeeper::from_state(k.state().clone()).is_ok());
    }

    #[test]
    fn min_interval_and_next_wake() {
        let mut k = keeper();
        k.join(mid(0), 0).unwrap();
        assert_eq!(k.next_wake(), Some(0), "the first epoch at once");
        let e1 = k.rotate(10, false).built().unwrap();
        assert!(k.published(e1.event.epoch));
        assert_eq!(k.next_wake(), None);
        k.join(mid(1), 20).unwrap();
        k.revoke(&[mid(1)], Revoke::Ban, 20);
        // Due at 20 + 300 by the window, and not before 10 + 300.
        assert_eq!(k.rotate(100, false), Rotate::NotDue { due_at: Some(320) });
        assert_eq!(k.next_wake(), Some(320));
        assert_eq!(k.stats().next_wake, Some(320));
        assert!(k.rotate(320, false).built().is_some());
    }
}
