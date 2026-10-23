// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The change log of a tree keeper (19.2, 7): the saved state as rows, so
//! that a host does not write a state of a hundred megabytes for every
//! join. `TreeKeeper::take_changes` gives what changed since its last
//! call; the host writes it (one row per member, per revoked leaf, per
//! missed key, one header, one pending recipe) before it sends anything
//! the calls returned, and `KeeperState::apply` puts the rows back
//! together.
//!
//! Like the whole state, the rows are secret (the header holds the
//! masters) and never go back to an earlier version.

use std::collections::BTreeSet;

use crate::epoch::EpochRecipe;
use crate::generation::{GenHeader, GenState};
use crate::hexser;
use crate::keeper::{KeeperState, MemberRec, Policy, Schedule, TreeParams};
use crate::{EpochKey, Error, MemberId, SchemeId, ScopeRef};

/// Everything of a keeper's state but its members, its revoked leaves,
/// its missed keys, the published blobs and the pending recipe: small,
/// rewritten whenever a counter moves.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct KeeperHeader {
    pub v: u8,
    pub scheme: SchemeId,
    pub scope: ScopeRef,
    pub policy: Policy,
    pub params: TreeParams,
    /// The live generations without their revoked leaves.
    pub gens: Vec<GenHeader>,
    #[serde(with = "hexser::list")]
    pub retired_masters: Vec<[u8; 32]>,
    pub schedule: Schedule,
    pub next_epoch: u32,
    pub head: Option<EpochKey>,
    pub published_head: Option<EpochKey>,
    pub last_epoch_at: Option<u64>,
    pub unmigrated: u64,
    pub last_p95: Option<usize>,
    pub last_upload: u64,
}

/// One row of the change log.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Change {
    /// The header, whole. Revoked leaves of a generation it no longer
    /// names are dropped with that generation.
    Header(KeeperHeader),
    /// A member as it is now.
    MemberPut { member: MemberId, rec: MemberRec },
    /// A member put out.
    MemberRemove { member: MemberId },
    /// A leaf revoked in a live generation.
    Revoked { gen: u32, leaf: u32 },
    /// The recipe of the pending epoch (once per epoch built).
    Pending(EpochRecipe),
    /// No epoch is pending any more.
    PendingCleared,
    /// A key added to the missed keys.
    Missed(EpochKey),
    /// The missed keys dropped.
    MissedCleared,
    /// The hashes of the blobs of the newest published epoch.
    PublishedBlobs {
        #[serde(with = "hexser::list")]
        hashes: Vec<[u8; 32]>,
    },
}

impl KeeperState {
    /// The header of the state.
    pub fn header(&self) -> KeeperHeader {
        KeeperHeader {
            v: self.v,
            scheme: self.scheme,
            scope: self.scope,
            policy: self.policy.clone(),
            params: self.params,
            gens: self.gens.iter().map(GenState::header).collect(),
            retired_masters: self.retired_masters.clone(),
            schedule: self.schedule.clone(),
            next_epoch: self.next_epoch,
            head: self.head.clone(),
            published_head: self.published_head.clone(),
            last_epoch_at: self.last_epoch_at,
            unmigrated: self.unmigrated,
            last_p95: self.last_p95,
            last_upload: self.last_upload,
        }
    }

    /// A state with a header and nothing else.
    pub fn from_header(h: KeeperHeader) -> Self {
        let mut s = KeeperState {
            v: h.v,
            scheme: h.scheme,
            scope: h.scope,
            policy: h.policy.clone(),
            params: h.params,
            members: Default::default(),
            gens: Vec::new(),
            retired_masters: Vec::new(),
            schedule: Default::default(),
            next_epoch: 0,
            head: None,
            published_head: None,
            last_epoch_at: None,
            pending: None,
            published_blobs: Vec::new(),
            missed: Vec::new(),
            unmigrated: 0,
            last_p95: None,
            last_upload: 0,
        };
        s.apply(Change::Header(h));
        s
    }

    /// Applies one row of the change log.
    pub fn apply(&mut self, ch: Change) {
        match ch {
            Change::Header(h) => {
                let mut old: Vec<GenState> = std::mem::take(&mut self.gens);
                self.gens = h
                    .gens
                    .into_iter()
                    .map(|gh| {
                        let revoked = old
                            .iter_mut()
                            .find(|g| g.gen == gh.gen)
                            .map(|g| std::mem::take(&mut g.revoked))
                            .unwrap_or_else(BTreeSet::new);
                        GenState::from_header(gh, revoked)
                    })
                    .collect();
                self.v = h.v;
                self.scheme = h.scheme;
                self.scope = h.scope;
                self.policy = h.policy;
                self.params = h.params;
                self.retired_masters = h.retired_masters;
                self.schedule = h.schedule;
                self.next_epoch = h.next_epoch;
                self.head = h.head;
                self.published_head = h.published_head;
                self.last_epoch_at = h.last_epoch_at;
                self.unmigrated = h.unmigrated;
                self.last_p95 = h.last_p95;
                self.last_upload = h.last_upload;
            }
            Change::MemberPut { member, rec } => {
                self.members.insert(member, rec);
            }
            Change::MemberRemove { member } => {
                self.members.remove(&member);
            }
            Change::Revoked { gen, leaf } => {
                if let Some(g) = self.gens.iter_mut().find(|g| g.gen == gen) {
                    g.revoked.insert(leaf);
                }
            }
            Change::Pending(r) => self.pending = Some(r),
            Change::PendingCleared => self.pending = None,
            Change::Missed(k) => self.missed.push(k),
            Change::MissedCleared => self.missed.clear(),
            Change::PublishedBlobs { hashes } => self.published_blobs = hashes,
        }
    }

    /// A state from its rows, the first a header, checked as a state of
    /// its scheme.
    pub fn replay(changes: impl IntoIterator<Item = Change>) -> Result<Self, Error> {
        let mut it = changes.into_iter();
        let Some(Change::Header(h)) = it.next() else { return Err(Error::Invalid("rows without a header first")) };
        let mut s = KeeperState::from_header(h);
        for ch in it {
            s.apply(ch);
        }
        s.check(s.scheme)?;
        Ok(s)
    }
}
