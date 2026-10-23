// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The snapshot of the readers (internal/communities-wire.md §8): a cache
//! of who reads the channel, signed by the keeper, for judging comments.
//! The staff, the bans, the mutes and trust are the journal's alone.
//!
//! A shard holds up to 1024 readers in ascending order of their keys,
//! each `pubkey(32) ‖ flags(1) ‖ levels(8) ‖ since(4)` (integers little
//! endian, as the epoch of a join MAC). It is sealed with the key of the
//! `everyone` stream of the epoch the snapshot names (`t:"snapshot"`) and
//! stored as `nonce ‖ ciphertext ‖ tag` under its SHA-256. The event names
//! the shards by the first key of each and their hashes, and the root of
//! a binary Merkle tree over those hashes in that order (an odd last one
//! is paired with itself).

use crate::op::{OpId, MAX_HEADS};
use crate::state::{ChannelState, Reader, SCOPE_EVERYONE};
use crate::wire::{self, CHANNEL_FEED, KIND_SNAPSHOT};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use keydist::{EpochKey, ScopeRef};
use messenger_core::{MessengerError, PubKey, Result};
use nostr::key::Keys;
use nostr::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const SNAPSHOT_VERSION: u32 = 1;
/// Readers one shard holds at most.
pub const SHARD_ENTRIES: usize = 1024;
/// Shards one event names at most (65 536 readers); more is an index
/// blob of stage 4.
pub const MAX_SHARDS: usize = 64;
/// The bytes of one reader in a shard.
pub const ENTRY_LEN: usize = 32 + 1 + 8 + 4;
/// `flags` bit 0: a reader (always set); bits 1–7 are reserved and read
/// as nothing.
pub const FLAG_READER: u8 = 1;
/// The keeper publishes a snapshot no more often than this, but in a small
/// channel (`keeper::SMALL_CHANNEL_READERS`), whose snapshot follows a change
/// of the readers within `keeper::SETTLE_SECS`.
pub const MIN_INTERVAL_SECS: i64 = 300;

fn invalid(what: &str) -> MessengerError {
    MessengerError::Invalid(format!("channel snapshot: {what}"))
}

/// One reader in a shard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub pubkey: [u8; 32],
    pub flags: u8,
    pub levels: u64,
    /// The time of `join()` at the keeper.
    pub since: u32,
}

impl Entry {
    pub fn reader(&self) -> Reader {
        Reader { since: i64::from(self.since) }
    }
}

/// The bytes of a shard: the entries, ascending by key, no key twice.
pub fn encode_shard(entries: &[Entry]) -> Result<Vec<u8>> {
    if entries.is_empty() || entries.len() > SHARD_ENTRIES {
        return Err(invalid("shard size"));
    }
    if entries.windows(2).any(|w| w[0].pubkey >= w[1].pubkey) {
        return Err(invalid("shard order"));
    }
    let mut out = Vec::with_capacity(entries.len() * ENTRY_LEN);
    for e in entries {
        if e.flags & FLAG_READER == 0 {
            return Err(invalid("flags"));
        }
        out.extend_from_slice(&e.pubkey);
        out.push(e.flags);
        out.extend_from_slice(&e.levels.to_le_bytes());
        out.extend_from_slice(&e.since.to_le_bytes());
    }
    Ok(out)
}

pub fn decode_shard(bytes: &[u8]) -> Result<Vec<Entry>> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(ENTRY_LEN) || bytes.len() / ENTRY_LEN > SHARD_ENTRIES {
        return Err(invalid("shard size"));
    }
    let mut out: Vec<Entry> = Vec::with_capacity(bytes.len() / ENTRY_LEN);
    for chunk in bytes.as_chunks::<ENTRY_LEN>().0 {
        let (pubkey, rest) = chunk.split_at(32);
        let (flags, rest) = rest.split_at(1);
        let (levels, since) = rest.split_at(8);
        let e = Entry {
            pubkey: pubkey.try_into().map_err(|_| invalid("entry"))?,
            flags: flags[0],
            levels: u64::from_le_bytes(levels.try_into().map_err(|_| invalid("entry"))?),
            since: u32::from_le_bytes(since.try_into().map_err(|_| invalid("entry"))?),
        };
        if e.flags & FLAG_READER == 0 {
            return Err(invalid("flags"));
        }
        if out.last().is_some_and(|l| l.pubkey >= e.pubkey) {
            return Err(invalid("shard order"));
        }
        out.push(e);
    }
    Ok(out)
}

/// The scope of the shards: the `everyone` scope of the channel.
fn everyone(community_id: &str) -> Result<ScopeRef> {
    wire::scope_ref(community_id, SCOPE_EVERYONE)
}

/// A shard sealed as it is stored: the payload of `t:"snapshot"` under the
/// key of the feed of `epoch_key`'s epoch, `nonce ‖ ciphertext ‖ tag`.
pub fn seal_shard(community_id: &str, epoch_key: &EpochKey, entries: &[Entry]) -> Result<Vec<u8>> {
    let key = wire::stream_key(epoch_key, &everyone(community_id)?, CHANNEL_FEED);
    let sealed = wire::seal(&key, &wire::Payload::shard(&encode_shard(entries)?))?;
    B64.decode(sealed).map_err(|_| invalid("sealed shard"))
}

/// A stored shard opened: the hash checked first (§9), then the seal.
pub fn open_shard(community_id: &str, epoch_key: &EpochKey, blob: &[u8], sha256: &str) -> Result<Vec<Entry>> {
    if hex::encode(Sha256::digest(blob)) != sha256 {
        return Err(invalid("hash"));
    }
    let key = wire::stream_key(epoch_key, &everyone(community_id)?, CHANNEL_FEED);
    let payload = wire::open(&key, &B64.encode(blob))?;
    if payload.t != wire::T_SNAPSHOT {
        return Err(invalid("payload"));
    }
    let shard = payload.shard.ok_or_else(|| invalid("payload"))?;
    decode_shard(&B64.decode(shard).map_err(|_| invalid("shard"))?)
}

/// The root of the binary Merkle tree over the hashes of the shards, in
/// their order: a parent is SHA-256(left ‖ right), an odd last node is
/// paired with itself. No shards: SHA-256 of nothing.
pub fn merkle_root(leaves: &[[u8; 32]]) -> [u8; 32] {
    if leaves.is_empty() {
        return Sha256::digest([]).into();
    }
    let mut level: Vec<[u8; 32]> = leaves.to_vec();
    while level.len() > 1 {
        level = level
            .chunks(2)
            .map(|pair| {
                let mut h = Sha256::new();
                h.update(pair[0]);
                h.update(pair.get(1).unwrap_or(&pair[0]));
                h.finalize().into()
            })
            .collect();
    }
    level[0]
}

/// One shard as the event names it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardRef {
    /// The first key of the shard, hex.
    pub from: String,
    /// Of the stored (sealed) shard, hex.
    pub sha256: String,
}

/// The content of a snapshot event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub v: u32,
    pub community_id: String,
    /// The heads of the journal the keeper knew: a client behind them
    /// fetches what it misses; the snapshot never overrides the journal.
    pub heads: Vec<OpId>,
    /// The epoch of each scope whose key seals the shards.
    pub epoch_ref: BTreeMap<String, u32>,
    pub members: u64,
    pub at: i64,
    pub root: String,
    pub shards: Vec<ShardRef>,
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl Snapshot {
    /// The shards' hashes as the leaves of the tree.
    fn leaves(&self) -> Result<Vec<[u8; 32]>> {
        self.shards
            .iter()
            .map(|s| hex::decode(&s.sha256).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| invalid("sha256")))
            .collect()
    }

    /// The shape of §8: the version, the channel, the heads, 1…64 shards
    /// in strictly ascending `from`, the root over their hashes.
    pub fn check(&self) -> Result<()> {
        if self.v == 0 || self.v > SNAPSHOT_VERSION {
            return Err(invalid("v"));
        }
        if !is_hex(&self.community_id, 64) {
            return Err(invalid("community_id"));
        }
        if self.heads.is_empty() || self.heads.len() > MAX_HEADS || self.heads.iter().any(|h| !is_hex(&h.0, 64)) {
            return Err(invalid("heads"));
        }
        if !self.epoch_ref.contains_key(SCOPE_EVERYONE) {
            return Err(invalid("epoch_ref"));
        }
        if self.shards.len() > MAX_SHARDS || self.shards.iter().any(|s| !is_hex(&s.from, 64) || !is_hex(&s.sha256, 64)) {
            return Err(invalid("shards"));
        }
        if self.shards.windows(2).any(|w| w[0].from >= w[1].from) {
            return Err(invalid("shard order"));
        }
        if hex::encode(merkle_root(&self.leaves()?)) != self.root {
            return Err(invalid("root"));
        }
        Ok(())
    }

    /// The shard whose range holds `who`: the last one that begins at or
    /// before the key (hex order is byte order).
    pub fn shard_for(&self, who: &PubKey) -> Option<usize> {
        self.shards.iter().rposition(|s| s.from.as_str() <= who.as_hex())
    }

    /// Whether the entries opened from shard `index` are the ones the
    /// range says: the first is its `from`, the last is before the next
    /// shard's.
    pub fn check_entries(&self, index: usize, entries: &[Entry]) -> Result<()> {
        let s = self.shards.get(index).ok_or_else(|| invalid("index"))?;
        let first = entries.first().ok_or_else(|| invalid("empty"))?;
        if hex::encode(first.pubkey) != s.from {
            return Err(invalid("from"));
        }
        if let (Some(next), Some(last)) = (self.shards.get(index + 1), entries.last()) {
            if hex::encode(last.pubkey) >= next.from {
                return Err(invalid("range"));
            }
        }
        Ok(())
    }
}

/// What the snapshot says of `who`, by the entries of its shard.
pub fn lookup(entries: &[Entry], who: &PubKey) -> Option<Reader> {
    let key: [u8; 32] = hex::decode(who.as_hex()).ok()?.try_into().ok()?;
    entries.binary_search_by(|e| e.pubkey.cmp(&key)).ok().map(|i| entries[i].reader())
}

/// The content of a snapshot event kept here (already verified when it
/// came).
pub fn content_of(event_json: &str) -> Result<Snapshot> {
    let event: serde_json::Value = serde_json::from_str(event_json)?;
    let content = event["content"].as_str().ok_or_else(|| invalid("content"))?;
    serde_json::from_str(content).map_err(|_| invalid("content"))
}

/// The event of a snapshot, signed by the keeper's key.
pub fn sign_snapshot(keeper: &Keys, snapshot: &Snapshot) -> Result<Event> {
    snapshot.check()?;
    EventBuilder::new(Kind::from(KIND_SNAPSHOT), serde_json::to_string(snapshot)?)
        .tag(Tag::parse(["h", snapshot.community_id.as_str()]).map_err(|e| invalid(&e.to_string()))?)
        .custom_created_at(Timestamp::from_secs(snapshot.at.max(0) as u64))
        .finalize(keeper)
        .map_err(|e| MessengerError::Crypto(e.to_string()))
}

/// Read a snapshot event (§8, §11.4): signed by the key of the keeper of
/// the current series by the journal, of this channel, of the shape of
/// §8, newer than the one known (`known_at`) and not dated past my clock
/// `now` and its skew: one dated ahead would make every later snapshot
/// "not newer" until that date.
pub fn verify_snapshot(signed: &serde_json::Value, state: &ChannelState, known_at: Option<i64>, now: i64) -> Result<Snapshot> {
    let event: Event = serde_json::from_value(signed.clone()).map_err(|_| invalid("event"))?;
    event.verify().map_err(|_| invalid("signature"))?;
    if event.kind.as_u16() != KIND_SNAPSHOT {
        return Err(invalid("kind"));
    }
    let keeper = state.keeper().ok_or_else(|| invalid("no keeper"))?;
    if event.pubkey.to_hex() != keeper.pubkey.as_hex() {
        return Err(invalid("not the current keeper"));
    }
    let h = event.tags.iter().find(|t| t.kind() == "h").and_then(|t| t.as_slice().get(1).cloned());
    if h.as_deref() != Some(state.community_id.as_str()) {
        return Err(invalid("h"));
    }
    let snapshot: Snapshot = serde_json::from_str(&event.content).map_err(|_| invalid("content"))?;
    if snapshot.community_id != state.community_id {
        return Err(invalid("community_id"));
    }
    snapshot.check()?;
    if snapshot.at > now + crate::wire::SKEW_SECS {
        return Err(invalid("dated ahead"));
    }
    if known_at.is_some_and(|k| snapshot.at <= k) {
        return Err(invalid("not newer"));
    }
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op::OpBody;
    use crate::state::tests::{op, pk, team, CHANNEL};
    use keydist::Key32;

    fn entry(first: u8, since: u32) -> Entry {
        let mut pubkey = [first; 32];
        pubkey[31] = 0;
        Entry { pubkey, flags: FLAG_READER, levels: 1, since }
    }

    #[test]
    fn shards_round_trip() {
        let entries = vec![entry(1, 10), entry(2, 20), entry(3, 30)];
        let bytes = encode_shard(&entries).unwrap();
        assert_eq!(bytes.len(), 3 * ENTRY_LEN);
        assert_eq!(&bytes[33..41], &1u64.to_le_bytes());
        assert_eq!(&bytes[41..45], &10u32.to_le_bytes());
        assert_eq!(decode_shard(&bytes).unwrap(), entries);
        // Reserved bits read as nothing; the reader bit is a must.
        let mut reserved = bytes.clone();
        reserved[32] = 0b1000_0001;
        assert_eq!(decode_shard(&reserved).unwrap()[0].flags, 0b1000_0001);
        let mut not_reader = bytes.clone();
        not_reader[32] = 0;
        assert!(decode_shard(&not_reader).is_err());
        // Order, size.
        assert!(encode_shard(&[entry(2, 0), entry(1, 0)]).is_err());
        assert!(encode_shard(&[entry(1, 0), entry(1, 0)]).is_err());
        assert!(encode_shard(&[]).is_err());
        assert!(decode_shard(&bytes[..ENTRY_LEN + 1]).is_err());
        let full: Vec<Entry> = (0..=SHARD_ENTRIES as u32)
            .map(|i| {
                let mut pubkey = [0u8; 32];
                pubkey[..4].copy_from_slice(&i.to_be_bytes());
                Entry { pubkey, flags: FLAG_READER, levels: 0, since: 0 }
            })
            .collect();
        assert!(encode_shard(&full[..SHARD_ENTRIES]).is_ok());
        assert!(encode_shard(&full).is_err(), "1025 readers");
    }

    #[test]
    fn sealed_shards_open_with_the_key_of_their_epoch() {
        let k = EpochKey { epoch: 17, key: Key32([4u8; 32]) };
        let entries = vec![entry(1, 10), entry(9, 20)];
        let blob = seal_shard(CHANNEL, &k, &entries).unwrap();
        let sha = hex::encode(Sha256::digest(&blob));
        assert_eq!(open_shard(CHANNEL, &k, &blob, &sha).unwrap(), entries);
        assert!(open_shard(CHANNEL, &k, &blob, &"00".repeat(32)).is_err(), "the hash first");
        let other = EpochKey { epoch: 18, key: Key32([5u8; 32]) };
        assert!(open_shard(CHANNEL, &other, &blob, &sha).is_err());
    }

    /// The root of a tree of three leaves, computed by hand (also in
    /// tests/vectors.json).
    #[test]
    fn merkle_root_vector() {
        let leaf = |b: u8| -> [u8; 32] { Sha256::digest([b]).into() };
        let pair = |a: &[u8; 32], b: &[u8; 32]| -> [u8; 32] {
            let mut h = Sha256::new();
            h.update(a);
            h.update(b);
            h.finalize().into()
        };
        let (a, b, c) = (leaf(1), leaf(2), leaf(3));
        assert_eq!(merkle_root(&[a]), a, "one leaf is the root");
        assert_eq!(merkle_root(&[a, b]), pair(&a, &b));
        let three = pair(&pair(&a, &b), &pair(&c, &c));
        assert_eq!(merkle_root(&[a, b, c]), three, "the odd last is paired with itself");
        assert_eq!(hex::encode(three), MERKLE_VECTOR);
        assert_eq!(merkle_root(&[]), <[u8; 32]>::from(Sha256::digest([])));
    }

    pub(crate) const MERKLE_VECTOR: &str = "9faa2a58b06fa09e3df6f260fcd26040b798fd90bfb33a759f85ef29e95ae648";

    fn snapshot(shards: Vec<ShardRef>) -> Snapshot {
        let leaves: Vec<[u8; 32]> = shards.iter().map(|s| hex::decode(&s.sha256).unwrap().try_into().unwrap()).collect();
        Snapshot {
            v: 1,
            community_id: CHANNEL.into(),
            heads: vec![OpId("ab".repeat(32))],
            epoch_ref: BTreeMap::from([(SCOPE_EVERYONE.to_string(), 17)]),
            members: 3,
            at: 1_000,
            root: hex::encode(merkle_root(&leaves)),
            shards,
        }
    }

    #[test]
    fn a_reader_is_found_by_the_range_of_its_shard() {
        let s = snapshot(vec![
            ShardRef { from: format!("{}{}", "01".repeat(31), "00"), sha256: "11".repeat(32) },
            ShardRef { from: format!("{}{}", "80".repeat(31), "00"), sha256: "22".repeat(32) },
        ]);
        s.check().unwrap();
        assert_eq!(s.shard_for(&PubKey::parse(&"00".repeat(32)).unwrap()), None, "before the first shard");
        assert_eq!(s.shard_for(&PubKey::parse(&"02".repeat(32)).unwrap()), Some(0));
        assert_eq!(s.shard_for(&PubKey::parse(&"ff".repeat(32)).unwrap()), Some(1));
        let first = vec![entry(1, 10), entry(2, 20)];
        s.check_entries(0, &first).unwrap();
        assert!(s.check_entries(0, &[entry(1, 10), entry(0x90, 1)]).is_err(), "into the next range");
        assert!(s.check_entries(1, &first).is_err(), "not its from");
        let who = PubKey::parse(&hex::encode(entry(2, 0).pubkey)).unwrap();
        assert_eq!(lookup(&first, &who), Some(Reader { since: 20 }));
        assert_eq!(lookup(&first, &PubKey::parse(&"03".repeat(32)).unwrap()), None);
        // Shapes refused.
        let mut bad = s.clone();
        bad.root = "00".repeat(32);
        assert!(bad.check().is_err());
        let mut bad = s.clone();
        bad.shards.reverse();
        assert!(bad.check().is_err());
        let mut bad = s.clone();
        bad.epoch_ref.clear();
        assert!(bad.check().is_err());
        let many = (0..=MAX_SHARDS).map(|i| ShardRef { from: format!("{i:064x}"), sha256: "11".repeat(32) }).collect();
        assert!(snapshot(many).check().is_err(), "65 shards");
    }

    #[test]
    fn a_snapshot_is_taken_from_the_current_keeper_only_and_newer_only() {
        let keeper = Keys::generate();
        let other = Keys::generate();
        let mut state = team();
        let as_pk = |k: &Keys| PubKey::parse(&k.public_key().to_hex()).unwrap();
        state.apply(&op(&pk("0"), OpBody::Keeper { pubkey: as_pk(&keeper), serial: 1, from_epoch: 5, inbox: None })).unwrap();
        let s = snapshot(vec![ShardRef { from: "01".repeat(32), sha256: "11".repeat(32) }]);
        let signed = serde_json::to_value(sign_snapshot(&keeper, &s).unwrap()).unwrap();
        assert_eq!(verify_snapshot(&signed, &state, None, 1_000).unwrap(), s);
        assert_eq!(verify_snapshot(&signed, &state, Some(999), 1_000).unwrap(), s);
        assert!(verify_snapshot(&signed, &state, None, 1_000 - crate::wire::SKEW_SECS - 1).is_err(), "dated ahead of my clock");
        assert!(verify_snapshot(&signed, &state, Some(1_000), 1_000).is_err(), "not newer");
        let forged = serde_json::to_value(sign_snapshot(&other, &s).unwrap()).unwrap();
        assert!(verify_snapshot(&forged, &state, None, 1_000).is_err(), "not the keeper");
        // The keeper of a past series no longer.
        state.apply(&op(&pk("0"), OpBody::Keeper { pubkey: as_pk(&other), serial: 2, from_epoch: 9, inbox: None })).unwrap();
        assert!(verify_snapshot(&signed, &state, None, 1_000).is_err());
    }
}
