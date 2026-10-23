// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Generations: one tree and its master. Leaves are handed out in order
//! and never again within a generation (a leaf's keys stay with whoever
//! held them). A generation ends in one of two ways (specification 5):
//!
//! - a **planned migration**, when the tree fills up, too many of its
//!   leaves are revoked or its shards grow too big: the new generation's
//!   bundles go out lazily, on each member's first activity; until the
//!   grace ends every epoch carries a cover of both generations; a member
//!   that migrates late reads the epochs it missed from the chain (`open`
//!   history) or gets their keys with its bundle (`from_join`). The new
//!   generation may have a deeper tree (`migrate` with new parameters);
//! - an **emergency regeneration**, when the keeper is compromised or
//!   lost: the old generation is dead at once, no epoch carries its cover
//!   again (its master may be in other hands), and a member reads again
//!   once it has a bundle of the new generation. A lost keeper is
//!   recovered the same way (`TreeKeeper::recover`).
//!
//! The master of every generation is new: a keeper refuses a master whose
//! fingerprint it holds (19.1, 1).

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::cover::{cs_cover, sd_cover};
use crate::epoch::EpochOut;
use crate::kdf::{GenerationMaster, NodeKeys};
use crate::sd::SdLabels;
use crate::tree::Tree;
use crate::{Error, ScopeRef};

/// When a planned migration starts, and how long its grace lasts. The
/// numbers are placeholders until the report of phase 1 sets them.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Thresholds {
    /// Share of the leaves handed out (0.9 = 90 %).
    pub capacity_ratio: f64,
    /// Share of the handed-out leaves that are revoked.
    pub revoked_ratio: f64,
    /// The 95th percentile of the shard size of the last epoch, bytes.
    pub shard_p95_bytes: usize,
    /// How long epochs carry the cover of the old generation, seconds.
    pub grace_secs: u64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            capacity_ratio: 0.9,
            revoked_ratio: 0.5,
            shard_p95_bytes: 128 * 1024,
            grace_secs: 30 * 24 * 3600,
        }
    }
}

/// Why a planned migration started.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MigrationReason {
    Capacity,
    Revoked,
    ShardSize,
    /// The owner asked for it (`migrate` with no threshold reached, say
    /// to deepen the tree).
    Requested,
}

impl MigrationReason {
    /// The byte of the reason in binary formats.
    pub fn code(self) -> u8 {
        match self {
            MigrationReason::Capacity => 0,
            MigrationReason::Revoked => 1,
            MigrationReason::ShardSize => 2,
            MigrationReason::Requested => 3,
        }
    }

    /// The reason a byte names.
    pub fn from_code(c: u8) -> Option<Self> {
        match c {
            0 => Some(MigrationReason::Capacity),
            1 => Some(MigrationReason::Revoked),
            2 => Some(MigrationReason::ShardSize),
            3 => Some(MigrationReason::Requested),
            _ => None,
        }
    }
}

impl Thresholds {
    /// The first threshold a generation reached, if any.
    pub fn reached(&self, g: &GenState, last_p95: Option<usize>) -> Option<MigrationReason> {
        let cap = g.tree.capacity() as f64;
        let handed = f64::from(g.next_leaf);
        if handed >= self.capacity_ratio * cap {
            return Some(MigrationReason::Capacity);
        }
        if g.next_leaf > 0 && g.revoked.len() as f64 > self.revoked_ratio * handed {
            return Some(MigrationReason::Revoked);
        }
        if last_p95.is_some_and(|p| p > self.shard_p95_bytes) {
            return Some(MigrationReason::ShardSize);
        }
        None
    }
}

/// How a generation began.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GenerationKind {
    /// The first generation of a scope.
    Initial,
    /// A planned migration; the previous generation is covered until
    /// `grace_until`.
    Planned { reason: MigrationReason, grace_until: u64 },
    /// An emergency regeneration (or the recovery of a lost keeper); the
    /// previous generations are dead.
    Emergency,
}

/// What starting a generation hands the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationOut {
    pub gen: u32,
    pub previous: Option<u32>,
    pub kind: GenerationKind,
    /// The first epoch under the new generation, already pending
    /// (emergency and recovery: a planned migration waits for the next
    /// rotation).
    pub epoch: Option<Arc<EpochOut>>,
}

/// One generation without its revoked leaves: what the header of the
/// saved state holds of it (`changes::KeeperHeader`).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GenHeader {
    pub gen: u32,
    pub tree: Tree,
    pub leaves_per_shard: u32,
    pub master: GenerationMaster,
    pub next_leaf: u32,
    pub kind: GenerationKind,
    pub started_at: u64,
}

/// One generation as the keeper holds it: sparse, a counter and a set.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GenState {
    pub gen: u32,
    pub tree: Tree,
    /// The shard width of this generation's epochs.
    pub leaves_per_shard: u32,
    pub master: GenerationMaster,
    /// The next leaf to hand out; `[0, next_leaf)` are handed out.
    pub next_leaf: u32,
    /// Revoked leaves, sorted.
    pub revoked: BTreeSet<u32>,
    pub kind: GenerationKind,
    pub started_at: u64,
}

impl GenState {
    /// A new, empty generation.
    pub fn new(
        gen: u32,
        tree: Tree,
        leaves_per_shard: u32,
        master: GenerationMaster,
        kind: GenerationKind,
        now: u64,
    ) -> Self {
        GenState {
            gen,
            tree,
            leaves_per_shard,
            master,
            next_leaf: 0,
            revoked: BTreeSet::new(),
            kind,
            started_at: now,
        }
    }

    /// The header of this generation.
    pub fn header(&self) -> GenHeader {
        GenHeader {
            gen: self.gen,
            tree: self.tree,
            leaves_per_shard: self.leaves_per_shard,
            master: self.master.clone(),
            next_leaf: self.next_leaf,
            kind: self.kind.clone(),
            started_at: self.started_at,
        }
    }

    /// A generation from its header and its revoked leaves.
    pub fn from_header(h: GenHeader, revoked: BTreeSet<u32>) -> Self {
        GenState {
            gen: h.gen,
            tree: h.tree,
            leaves_per_shard: h.leaves_per_shard,
            master: h.master,
            next_leaf: h.next_leaf,
            revoked,
            kind: h.kind,
            started_at: h.started_at,
        }
    }

    /// The node keys of this generation.
    pub fn node_keys(&self, scope: &ScopeRef) -> NodeKeys {
        NodeKeys::new(self.master.key(), scope, self.gen)
    }

    /// Hands out the next leaf.
    pub fn assign(&mut self) -> Result<u32, Error> {
        if u64::from(self.next_leaf) >= self.tree.capacity() {
            return Err(Error::GenerationFull);
        }
        let leaf = self.next_leaf;
        self.next_leaf += 1;
        Ok(leaf)
    }

    /// The cover of the active leaves.
    pub fn cover(&self) -> Vec<u64> {
        cs_cover(self.tree, u64::from(self.next_leaf), &self.revoked)
    }

    /// The `sd-v1` cover of the active leaves: subsets `(i, j)`.
    pub fn sd_cover(&self) -> Vec<(u64, u64)> {
        sd_cover(self.tree, u64::from(self.next_leaf), &self.revoked)
    }

    /// The `sd-v1` labels of this generation.
    pub fn sd_labels(&self, scope: &ScopeRef) -> SdLabels {
        SdLabels::new(self.master.key(), scope, self.gen)
    }

    /// The end of the grace, for the old side of a planned migration
    /// (read from the generation that follows it).
    pub fn grace_until(&self) -> Option<u64> {
        match self.kind {
            GenerationKind::Planned { grace_until, .. } => Some(grace_until),
            _ => None,
        }
    }
}
