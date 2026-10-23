// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The member's side of `cs-v1` and `sd-v1`: open an epoch from the event
//! and the one shard of the member's leaf, follow the chain of an open
//! history, derive channel keys.
//!
//! A member acts on an event only after the host checked the keeper's
//! signature of it (`Verified<EpochEvent>`). It asks `MemberState::status`
//! what to do: read shard `leaf / leaves_per_shard` of a generation it
//! holds a bundle of, ask the keeper for a new bundle, or nothing. It
//! fetches the witness the event names for that generation (the index
//! blob, or the shard's proof in Merkle mode) and the shard, checks the
//! shard against the event through the witness, then looks for its block.
//! In `cs-v1`: the nodes of its path among the blocks, at most `depth + 1`
//! lookups, and the block it finds opens with the key its bundle holds
//! for that node. In `sd-v1`: each ancestor `i` among the blocks, at most
//! `depth` lookups, the block whose `j` is off its path, and the key of
//! `S_{i,j}` walked down from the label its bundle holds. The event, the
//! bundle and the shard name their scheme and scope and must agree.
//!
//! A member never takes a second, different key for an epoch it holds
//! (`Error::Conflict`): a replayed or forged event, or a keeper gone back
//! in time, cannot swap a key. Once it takes the bundle of an emergency
//! generation it drops the bundles before it and refuses events that
//! carry only older generations (their master may be in other hands).

use std::collections::BTreeMap;

use crate::bundle::Bundle;
use crate::epoch::{block_node, open_block, open_chain, open_sd_block, sd_block_subset, EpochEvent};
use crate::generation::GenerationKind;
use crate::kdf;
use crate::shard;
use crate::{ChannelKey, EpochKey, Error, Issued, SchemeId, ScopeRef, Verified};

/// The number of the shard a bundle reads in an event, if the event
/// carries the bundle's generation and the leaf lies within its shards.
pub fn shard_of(bundle: &Bundle, ev: &EpochEvent) -> Option<u32> {
    let ti = ev.tree(bundle.gen)?;
    let no = bundle.leaf / ti.leaves.max(1);
    (no < ti.count).then_some(no)
}

/// Opens an epoch: the key, and the number of path nodes looked up.
/// `witness` is the blob the event names for the bundle's generation:
/// its index blob, or the shard's proof when the event carries a Merkle
/// root.
pub fn open_epoch_counted(
    bundle: &Bundle,
    ev: &Verified<EpochEvent>,
    shard_bytes: &[u8],
    witness: &[u8],
) -> Result<(EpochKey, usize), Error> {
    let tree_scheme = matches!(ev.scheme, SchemeId::CsV1 | SchemeId::SdV1);
    if !tree_scheme || ev.scheme != bundle.scheme || bundle.scope != ev.scope() {
        return Err(Error::Invalid("event of another scope or scheme"));
    }
    let ti = ev.tree(bundle.gen).ok_or(Error::NotCovered)?;
    if ti.depth != bundle.tree.depth() {
        return Err(Error::Invalid("depth"));
    }
    let no = shard_of(bundle, ev).ok_or(Error::NotCovered)?;
    shard::verify(ev, bundle.gen, no, shard_bytes, witness)?;
    if ev.scheme == SchemeId::SdV1 {
        let (block, checks) = shard::find_sd(shard_bytes, bundle.tree, bundle.leaf);
        let block = block.ok_or(Error::NotCovered)?;
        let (i, j) = sd_block_subset(block);
        let key = bundle.subset_key(i, j).ok_or(Error::NotCovered)?;
        let k = open_sd_block(&key, block, &ev.scope(), bundle.gen, ev.epoch)?;
        return Ok((k, checks));
    }
    let (block, checks) = shard::find(shard_bytes, bundle.tree.path(bundle.leaf));
    let block = block.ok_or(Error::NotCovered)?;
    let key = bundle.key_for(block_node(block)).ok_or(Error::NotCovered)?;
    let k = open_block(key, block, &ev.scope(), bundle.gen, ev.epoch)?;
    Ok((k, checks))
}

/// Opens an epoch with a bundle, the verified event, the member's shard
/// and the witness of the shard (`open_epoch_counted`).
pub fn open_epoch(
    bundle: &Bundle,
    ev: &Verified<EpochEvent>,
    shard_bytes: &[u8],
    witness: &[u8],
) -> Result<EpochKey, Error> {
    open_epoch_counted(bundle, ev, shard_bytes, witness).map(|(k, _)| k)
}

/// What a member does with an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Fetch the witness and shard `shard` of generation `gen`, then
    /// `open`.
    Read { gen: u32, shard: u32 },
    /// The event carries a generation newer than every bundle held: ask
    /// the keeper for the bundle of `gen` (a rebundle).
    NeedBundle { gen: u32 },
    /// Nothing of this event is for this member: it was not in the epoch
    /// (it joined after the epoch was built), the event is of another
    /// scope or scheme, or of a generation before an emergency.
    Out,
}

/// What a member keeps of a scope: its bundles (at most two
/// generations), the epoch keys it holds, and the newest emergency
/// generation it took a bundle of.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MemberState {
    pub scope: ScopeRef,
    pub bundles: Vec<Bundle>,
    pub keys: BTreeMap<u32, EpochKey>,
    /// Events that carry only generations older than this are refused.
    #[serde(default)]
    pub emergency_gen: Option<u32>,
}

impl MemberState {
    /// Nothing yet.
    pub fn new(scope: ScopeRef) -> Self {
        MemberState { scope, bundles: Vec::new(), keys: BTreeMap::new(), emergency_gen: None }
    }

    /// Takes what a keeper issued: the bundle (dropping all but the two
    /// newest generations, and every generation before an emergency one)
    /// and the keys. Nothing is taken when a key conflicts with one held
    /// or the bundle is older than the newest emergency.
    pub fn accept(&mut self, issued: &Issued) -> Result<(), Error> {
        if issued.scope != self.scope {
            return Err(Error::Invalid("issued for another scope"));
        }
        if self.emergency_gen.is_some_and(|e| issued.gen < e) {
            return Err(Error::Invalid("issued before an emergency"));
        }
        let bundle = if issued.bundle.is_empty() {
            None
        } else {
            let b = Bundle::from_bytes(&issued.bundle)?;
            if b.scope != self.scope || b.gen != issued.gen || b.leaf != issued.leaf || b.scheme != issued.scheme {
                return Err(Error::Invalid("bundle of another scope"));
            }
            Some(b)
        };
        for k in issued.current.iter().chain(issued.missed.iter()) {
            if self.keys.get(&k.epoch).is_some_and(|held| held != k) {
                return Err(Error::Conflict);
            }
        }
        if matches!(issued.kind, GenerationKind::Emergency) {
            self.emergency_gen = Some(self.emergency_gen.map_or(issued.gen, |e| e.max(issued.gen)));
            self.bundles.retain(|x| x.gen >= issued.gen);
        }
        if let Some(b) = bundle {
            self.bundles.retain(|x| x.gen != b.gen);
            self.bundles.push(b);
            self.bundles.sort_by_key(|x| x.gen);
            let excess = self.bundles.len().saturating_sub(2);
            self.bundles.drain(..excess);
        }
        for k in issued.current.iter().chain(issued.missed.iter()) {
            self.keys.entry(k.epoch).or_insert_with(|| k.clone());
        }
        Ok(())
    }

    /// Keeps a key (from a shard, a chain, a flat secret): a second,
    /// different key for an epoch held is refused.
    pub fn keep(&mut self, k: EpochKey) -> Result<(), Error> {
        match self.keys.get(&k.epoch) {
            Some(held) if *held != k => Err(Error::Conflict),
            Some(_) => Ok(()),
            None => {
                self.keys.insert(k.epoch, k);
                Ok(())
            }
        }
    }

    fn check_event(&self, ev: &EpochEvent) -> Result<(), Error> {
        if ev.scope() != self.scope {
            return Err(Error::Invalid("event of another scope"));
        }
        if let (Some(e), Some(n)) = (self.emergency_gen, ev.newest_gen()) {
            if n < e {
                return Err(Error::Invalid("event of a generation before an emergency"));
            }
        }
        Ok(())
    }

    /// What to do with an event (19.3, 15).
    pub fn status(&self, ev: &Verified<EpochEvent>) -> Status {
        if self.check_event(ev).is_err() {
            return Status::Out;
        }
        let Some(newest) = ev.newest_gen() else { return Status::Out };
        if let Some((gen, shard)) = self.bundles.iter().rev().find_map(|b| {
            (b.scheme == ev.scheme).then(|| shard_of(b, ev).map(|no| (b.gen, no))).flatten()
        }) {
            return Status::Read { gen, shard };
        }
        if self.bundles.iter().all(|b| b.gen < newest) {
            return Status::NeedBundle { gen: newest };
        }
        Status::Out
    }

    /// Opens an epoch with the bundle of the shard's generation and keeps
    /// the key.
    pub fn open(&mut self, ev: &Verified<EpochEvent>, shard_bytes: &[u8], witness: &[u8]) -> Result<EpochKey, Error> {
        self.check_event(ev)?;
        let gen = shard::parse_header(shard_bytes)?.gen;
        let b = self.bundles.iter().find(|b| b.gen == gen).ok_or(Error::NotCovered)?;
        let k = open_epoch(b, ev, shard_bytes, witness)?;
        self.keep(k.clone())?;
        Ok(k)
    }

    /// Under an open history, takes the previous key from an event whose
    /// key this member holds.
    pub fn follow_chain(&mut self, ev: &Verified<EpochEvent>) -> Result<EpochKey, Error> {
        self.check_event(ev)?;
        let k = self.keys.get(&ev.epoch).ok_or(Error::NotCovered)?;
        let prev = open_chain(ev, k)?;
        self.keep(prev.clone())?;
        Ok(prev)
    }

    /// Drops the keys of epochs before `keep_from`.
    pub fn prune(&mut self, keep_from: u32) {
        self.keys.retain(|&e, _| e >= keep_from);
    }

    /// A key this member holds.
    pub fn key(&self, epoch: u32) -> Option<&EpochKey> {
        self.keys.get(&epoch)
    }

    /// The key of a channel in an epoch this member holds.
    pub fn channel_key(&self, epoch: u32, channel_id: &[u8]) -> Option<ChannelKey> {
        self.keys.get(&epoch).map(|k| kdf::channel_key(k, &self.scope, channel_id))
    }
}
