// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Key distribution for large encrypted scopes.
//!
//! A scope (a community, a channel, a group) encrypts its messages under
//! an epoch key. This crate hands that key out and takes it back in a way
//! whose cost does not grow with the number of members: stateless
//! broadcast revocation (Naor–Naor–Lotspiech) over a binary tree. Every
//! member holds a small bundle of node keys once per generation; a new
//! epoch is a cover of the tree that reaches every member still in and no
//! member put out, cut into static shards of which a member fetches one.
//!
//! The crate is the scheme and its state, nothing else: no relay, no blob
//! store, no database, no signature. The host persists the keeper's state
//! (whole, `KeeperState`, or as rows from `TreeKeeper::take_changes`) and
//! `MemberState`, and carries the bytes this crate hands it (an epoch
//! event, blobs, a secret for one member). The same code runs in the
//! application of a community owner (the keeper on a phone or a desktop),
//! in a keeper service the owner hosts, and in every member's client.
//!
//! Schemes carry a name so that a scope created on one keeps it for life
//! while a new scope may start on another:
//!
//! | scheme    | what                                                     |
//! |-----------|----------------------------------------------------------|
//! | `flat-v1` | one secret per member, the way the groups do it today    |
//! | `cs-v1`   | Complete Subtree: `depth + 1` keys per member            |
//! | `sd-v1`   | Subset Difference: more keys per member, smaller covers  |
//!
//! The specification and the plan of phase 1 (prototype, simulator,
//! measurements) are in `internal/communities-keydist.md`; the decisions
//! of the review of 2026-10-17 are its section 19 and win over 5–7. The
//! wire formats are frozen there once the report of phase 1 picks the
//! scheme.
//!
//! Keys: the owner's device holds a `RootSecret` and derives the
//! `GenerationMaster` of one generation of one scope and scheme
//! (`GenerationMaster::derive`); the keeper holds only masters and refuses
//! one it held before. Every epoch key is random; the key of a channel and
//! the key of the chain of an open history are derived from it by HKDF.
//!
//! The contract with the host (specification 6, 19.2): save the state (or
//! the changes) first, then send what an operation returned; never put an
//! older saved state back. A keeper is `CsKeeper`, `SdKeeper` (one logic,
//! `TreeKeeper`) or `FlatKeeper` behind the trait `KeyDistribution`; a
//! member checks the keeper's signature of an event, wraps it in
//! `Verified`, and opens the epoch with `open_epoch` (or `MemberState`).
//!
//! # Changes of stage 4a (fix wave after the review, 19)
//!
//! Format changes against the vectors of stages 1 and 4: the master of a
//! generation now takes `scope ‖ scheme code` into its info (19.1, 1); the
//! chain of an open history is sealed under `HKDF(K_epoch, "veydan.keydist
//! .chain.v1" ‖ community ‖ scope)` instead of `K_epoch` (the 60 bytes of
//! the chain are laid out as before); the 12-byte nonce of a block is
//! `SHA-256("veydan.keydist.nonce.v1" ‖ seed ‖ node or (i, j))[..12]` from
//! one random seed per build (the block layout of 68 and 76 bytes is
//! unchanged). The bundle layout is unchanged (version 1). The epoch event
//! is version 2: a tree names `index` (the SHA-256 of an index blob `VKI1`
//! that lists the shard hashes) or `root` (Merkle, above 512 shards a
//! tree), never the hash list itself. The saved keeper state is version 2
//! and names its scheme; a recipe holds the seed and bare node ids (8
//! bytes, `sd-v1` 16) instead of a nonce per node. New: the delivery of
//! `Issued` as bytes (`VKA1`).

#![forbid(unsafe_code)]

use std::fmt;
use std::sync::Arc;

use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

pub mod bundle;
pub mod changes;
pub mod cover;
pub mod epoch;
pub mod flat;
pub mod generation;
pub mod issued;
pub mod kdf;
pub mod keeper;
pub mod member;
pub mod rng;
pub mod sd;
pub mod shard;
pub mod tree;

mod hexser;

pub use bundle::Bundle;
pub use changes::{Change, KeeperHeader};
pub use epoch::{Blob, EpochEvent, EpochOut, TreeIndex, BLOCK_LEN, SD_BLOCK_LEN};
pub use flat::{open_secret, FlatKeeper, FlatState};
pub use generation::{GenerationKind, GenerationOut, MigrationReason, Thresholds};
pub use kdf::{channel_key, generation_key, GenerationMaster, RootSecret};
pub use keeper::{
    CsKeeper, CsScheme, KeeperState, KeeperStats, Policy, Scheme, SdKeeper, SdScheme, TreeKeeper,
    TreeParams,
};
pub use member::{open_epoch, MemberState, Status};
pub use rng::{OsRng, Rng};
#[cfg(any(test, feature = "tools"))]
pub use rng::XorShift64;
pub use shard::{Proof, ShardIndex};
pub use tree::Tree;

/// The name of a key distribution scheme, fixed when a scope is created.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SchemeId {
    /// One secret per member (the groups of the messenger today).
    FlatV1,
    /// Complete Subtree over a binary tree.
    CsV1,
    /// Subset Difference over a binary tree.
    SdV1,
}

impl SchemeId {
    /// The word on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            SchemeId::FlatV1 => "flat-v1",
            SchemeId::CsV1 => "cs-v1",
            SchemeId::SdV1 => "sd-v1",
        }
    }

    /// The scheme a word names, if any.
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "flat-v1" => Some(SchemeId::FlatV1),
            "cs-v1" => Some(SchemeId::CsV1),
            "sd-v1" => Some(SchemeId::SdV1),
            _ => None,
        }
    }

    /// The byte that names the scheme inside binary formats (a bundle, a
    /// flat secret, the info of a generation master).
    pub fn code(self) -> u8 {
        match self {
            SchemeId::FlatV1 => 1,
            SchemeId::CsV1 => 2,
            SchemeId::SdV1 => 3,
        }
    }

    /// The scheme a byte names, if any.
    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(SchemeId::FlatV1),
            2 => Some(SchemeId::CsV1),
            3 => Some(SchemeId::SdV1),
            _ => None,
        }
    }
}

impl fmt::Display for SchemeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What can go wrong inside the scheme. The host maps it to its own
/// errors; nothing here names a transport or a store.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    /// A member this keeper does not know.
    #[error("unknown member")]
    UnknownMember,
    /// The tree of this generation has no leaf left.
    #[error("the generation is full")]
    GenerationFull,
    /// Bytes that are not what they claim to be: a shard of another
    /// epoch, a block that does not open, a hash that does not match.
    #[error("invalid {0}")]
    Invalid(&'static str),
    /// The bundle holds no key of the cover: the member is out, or the
    /// shard is not the member's.
    #[error("no key of the cover")]
    NotCovered,
    /// Parameters a keeper cannot be built with (a depth out of range, a
    /// shard width that is not a power of two).
    #[error("bad parameter: {0}")]
    Params(&'static str),
    /// A generation master equal to the master of a generation this
    /// keeper holds or held: refused, a new generation needs a new master.
    #[error("a generation master that was used before")]
    ReusedMaster,
    /// A key for an epoch the member already holds, other than the one it
    /// holds: a replay, a forgery or a keeper that went back in time.
    #[error("another key for an epoch already held")]
    Conflict,
    /// The operation does not fit the keeper now (a migration while one
    /// runs, a generation of a scheme without generations).
    #[error("not now: {0}")]
    State(&'static str),
}

/// A member: the 32 bytes of its public key (a Nostr npub in the
/// messenger). The crate never interprets them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MemberId(pub [u8; 32]);

impl fmt::Debug for MemberId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MemberId({})", hex::encode(&self.0[..6]))
    }
}

impl serde::Serialize for MemberId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        hexser::serialize(&self.0, s)
    }
}

impl<'de> serde::Deserialize<'de> for MemberId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        hexser::deserialize(d).map(MemberId)
    }
}

/// The scope a key belongs to: the community and the scope inside it
/// (the whole community, or a closed group of it). Both are raw 32 bytes
/// that go into every key derivation and every associated data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ScopeRef {
    #[serde(with = "hexser")]
    pub community: [u8; 32],
    #[serde(with = "hexser")]
    pub scope: [u8; 32],
}

/// A secret of 32 bytes: a node key, an epoch key, a label. Wiped when
/// dropped; its `Debug` never prints it.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct Key32(pub [u8; 32]);

impl Key32 {
    /// The bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for Key32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Key32(..)")
    }
}

impl serde::Serialize for Key32 {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        hexser::serialize(&self.0, s)
    }
}

impl<'de> serde::Deserialize<'de> for Key32 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        hexser::deserialize(d).map(Key32)
    }
}

/// Bytes that hold secrets: a bundle, a flat secret, an `Issued` in its
/// byte form. Wiped when dropped; its `Debug` prints only the length.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct SecretBytes(Zeroizing<Vec<u8>>);

impl SecretBytes {
    /// Takes the bytes (the vector is wiped with the wrapper).
    pub fn new(bytes: Vec<u8>) -> Self {
        SecretBytes(Zeroizing::new(bytes))
    }

    /// The bytes.
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    /// Appends bytes (used while a layout is written).
    pub(crate) fn extend(&mut self, b: &[u8]) {
        self.0.extend_from_slice(b);
    }

    pub(crate) fn push(&mut self, b: u8) {
        self.0.push(b);
    }

    pub(crate) fn with_capacity(n: usize) -> Self {
        SecretBytes(Zeroizing::new(Vec::with_capacity(n)))
    }
}

impl std::ops::Deref for SecretBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8]> for SecretBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for SecretBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretBytes({} bytes)", self.0.len())
    }
}

/// The key of one epoch of a scope, with its number. Epoch numbers run
/// over the whole life of a scope, across generations, and are never
/// reused.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EpochKey {
    pub epoch: u32,
    pub key: Key32,
}

/// The key of one channel of a scope in one epoch. Only a name space:
/// whoever knows the epoch key knows the key of every channel.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct ChannelKey(pub [u8; 32]);

impl fmt::Debug for ChannelKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ChannelKey(..)")
    }
}

/// Why a member is put out. A ban rotates after the ban window (or at
/// once when urgent); a leave rotates after the leave window in a closed
/// scope and never by itself in an open one (the next epoch for any
/// reason leaves the member out).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Revoke {
    Ban,
    Leave,
}

/// How much of the past a member reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum History {
    /// Every epoch event carries the previous key under the new one: who
    /// holds a key holds every key before it. A newcomer gets the current
    /// key at once.
    Open,
    /// Strict: a newcomer reads from the first epoch that begins after
    /// its join, not the one during which it joined.
    FromJoin,
}

/// What a keeper hands one member: its bundle (to be sealed to the
/// member's public key by the host, NIP-44 in the messenger) and the epoch
/// keys it is entitled to but cannot open from a shard. The host sends it
/// as `to_bytes` (module `issued`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issued {
    pub scheme: SchemeId,
    pub scope: ScopeRef,
    /// The generation and the leaf of the bundle (`flat-v1`: the number
    /// of emergencies and the member's ordinal).
    pub gen: u32,
    pub leaf: u32,
    /// How the generation began: a member that takes the bundle of an
    /// emergency generation drops the bundles before it and refuses the
    /// events of older generations from then on.
    pub kind: GenerationKind,
    /// The bundle bytes (`Bundle::to_bytes`); empty for `flat-v1` and for
    /// a further page of missed keys (`KeyDistribution::missed_page`).
    pub bundle: SecretBytes,
    /// The newest epoch key, when the member may read it now. `None`
    /// under strict `from_join` until the member's first epoch, and
    /// before the first epoch of a scope.
    pub current: Option<EpochKey>,
    /// `from_join` only: keys of epochs the member was entitled to but no
    /// cover of a bundle it holds reached (a late migration, the epochs
    /// after an emergency). Oldest first, at most one page
    /// (`issued::MISSED_PAGE` keys). Under `open` it is always empty: the
    /// current key and the chain of the events give every earlier key.
    pub missed: Vec<EpochKey>,
    /// Whether more missed keys follow: the host asks
    /// `missed_page(member, last epoch of this page, …)`.
    pub missed_more: bool,
}

/// What `join` returns: the same as a reissue.
pub type Joined = Issued;

/// What a blob is, so that the host can address it by a path and not
/// only by its hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlobKind {
    /// A shard of one generation (`no` its number).
    Shard,
    /// The Merkle proof of shard `no` (trees above
    /// `shard::INDEX_MAX` shards).
    Proof,
    /// The index of one generation's shard hashes (`no` is 0).
    Index,
}

/// One thing the host sends after it saved the state, borrowed from the
/// `EpochOut` it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outbound<'a> {
    /// The epoch event, to be signed by the keeper's key and put on the
    /// relays (the host serializes it, JSON in the messenger). Always the
    /// last item.
    Event(&'a EpochEvent),
    /// A static blob, addressed by its SHA-256 or by `(kind, gen, epoch,
    /// no)`. Sending one twice is harmless: the bytes never change.
    Blob { kind: BlobKind, gen: u32, epoch: u32, no: u32, sha256: [u8; 32], bytes: &'a [u8] },
    /// A secret for one member, to be sealed to its public key.
    Secret { to: MemberId, bytes: &'a SecretBytes },
}

/// What `rotate` did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rotate {
    /// A new epoch, now pending until `published(epoch)`.
    Built(Arc<EpochOut>),
    /// Nothing is due; `due_at` is when the queued changes want an epoch
    /// (`None`: nothing is queued).
    NotDue { due_at: Option<u64> },
    /// An epoch built earlier waits for `published`; nothing new is built
    /// (`supersede_pending` replaces it while its event is not out).
    Blocked { pending_epoch: u32 },
}

impl Rotate {
    /// The epoch built, if any.
    pub fn built(self) -> Option<Arc<EpochOut>> {
        match self {
            Rotate::Built(out) => Some(out),
            _ => None,
        }
    }
}

/// A value whose signature the host checked: an epoch event signed by the
/// keeper of its scope. Only the host builds it, after the check; the
/// functions that act on an event take nothing else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verified<T>(T);

impl<T> Verified<T> {
    /// Wraps a value the host has checked the signature of. The crate
    /// cannot check it (the signature is the host's); calling this before
    /// the check is the host's bug.
    pub fn after_signature_check(value: T) -> Self {
        Verified(value)
    }

    /// The value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> std::ops::Deref for Verified<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: serde::Serialize> serde::Serialize for Verified<T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
}

/// A keeper of a scope, whatever its scheme. All operations are pure
/// state changes; time is the host's clock in seconds, passed in.
pub trait KeyDistribution {
    /// The scheme of the scope.
    fn scheme(&self) -> SchemeId;
    /// The scope.
    fn scope(&self) -> &ScopeRef;
    /// Adds a member, or hands an existing one the same leaf and bundle
    /// again (idempotent). A member put out and let in again gets a new
    /// leaf: a leaf is never handed out twice.
    fn join(&mut self, member: MemberId, now: u64) -> Result<Joined, Error>;
    /// Puts members out. Their leaves are excluded from the next epoch at
    /// once; when that epoch is built depends on `why` and the policy.
    fn revoke(&mut self, members: &[MemberId], why: Revoke, now: u64);
    /// Builds the next epoch when one is due (or at once when `urgent`,
    /// or for the first epoch of a scope with members). A due epoch waits
    /// for `Policy::min_epoch_interval_secs` after the previous one unless
    /// it is urgent.
    fn rotate(&mut self, now: u64, urgent: bool) -> Rotate;
    /// The epoch built and not yet acknowledged, the same bytes every
    /// time, also after the state went through serialization.
    fn pending(&self) -> Option<&EpochOut>;
    /// The host acknowledges that the event of `epoch` is out. `false`
    /// (and nothing changes) when `epoch` is not the pending one.
    fn published(&mut self, epoch: u32) -> bool;
    /// While the event of the pending epoch is not out: burns its number
    /// and builds a fresh epoch at once (an urgent ban does not wait
    /// behind a large upload). `None` when nothing is pending.
    fn supersede_pending(&mut self, now: u64) -> Option<Arc<EpochOut>>;
    /// Whether a planned migration is due by `t`, and why.
    fn migration_due(&self, t: &Thresholds) -> Option<MigrationReason>;
    /// Starts a planned migration to a new generation under
    /// `next_master` (derived offline by the owner), deeper or wider when
    /// `new_params` says so; the grace is the policy's.
    fn migrate(
        &mut self,
        next_master: GenerationMaster,
        new_params: Option<TreeParams>,
        now: u64,
    ) -> Result<GenerationOut, Error>;
    /// `migrate` when `migration_due(t)`, with the grace of `t`.
    fn migrate_if_needed(
        &mut self,
        t: &Thresholds,
        next_master: &GenerationMaster,
        now: u64,
    ) -> Result<Option<GenerationOut>, Error> {
        if self.migration_due(t).is_none() {
            return Ok(None);
        }
        self.migrate(next_master.clone(), None, now).map(Some)
    }
    /// Drops every generation at once and starts a new one under
    /// `new_master`, with a new epoch that only the new generation opens.
    fn emergency_regenerate(&mut self, new_master: GenerationMaster, now: u64) -> Result<GenerationOut, Error>;
    /// The bundle of a member in the current generation (assigning a leaf
    /// in it on a lazy migration), with the first page of the keys it
    /// missed.
    fn bundle_for(&mut self, member: MemberId, now: u64) -> Result<Issued, Error>;
    /// A further page of missed keys (`from_join`): those after
    /// `after_epoch`, at most `limit`, in an `Issued` without a bundle.
    fn missed_page(&self, member: &MemberId, after_epoch: u32, limit: usize) -> Result<Issued, Error>;
    /// Whether the member is in (joined and not put out).
    fn is_member(&self, member: &MemberId) -> bool;
    /// When the host should call `rotate` next: the earliest of the due
    /// epoch, the end of a grace and a reached migration threshold of the
    /// policy (a moment in the past means at once). `None`: nothing waits.
    fn next_wake(&self) -> Option<u64>;
    /// Counters for the host, the simulator and the thresholds.
    fn stats(&self) -> KeeperStats;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_words_round_trip() {
        for s in [SchemeId::FlatV1, SchemeId::CsV1, SchemeId::SdV1] {
            assert_eq!(SchemeId::parse(s.as_str()), Some(s));
            assert_eq!(SchemeId::from_code(s.code()), Some(s));
            let json = serde_json::to_string(&s).unwrap();
            assert_eq!(json, format!("\"{}\"", s.as_str()));
        }
        assert_eq!(SchemeId::parse("mls"), None);
        assert_eq!(SchemeId::from_code(0), None);
    }

    #[test]
    fn ids_serialize_as_hex() {
        let m = MemberId([0xab; 32]);
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(json, format!("\"{}\"", "ab".repeat(32)));
        assert_eq!(serde_json::from_str::<MemberId>(&json).unwrap(), m);
        assert!(serde_json::from_str::<MemberId>("\"abcd\"").is_err());
        let k = Key32([7; 32]);
        assert_eq!(format!("{k:?}"), "Key32(..)");
    }

    #[test]
    fn secrets_never_print() {
        let s = SecretBytes::new(vec![0x42; 5]);
        assert_eq!(format!("{s:?}"), "SecretBytes(5 bytes)");
        let k = EpochKey { epoch: 3, key: Key32([0x42; 32]) };
        assert!(!format!("{k:?}").contains("42, 42"));
        let m = GenerationMaster::from_bytes([0x42; 32]);
        assert_eq!(format!("{m:?}"), "GenerationMaster(..)");
        let r = RootSecret::from_bytes([0x42; 32]);
        assert_eq!(format!("{r:?}"), "RootSecret(..)");
    }
}
