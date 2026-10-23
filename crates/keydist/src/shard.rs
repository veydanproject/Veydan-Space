// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Shards: the static blobs an epoch is cut into, one GET per member, and
//! what proves them: an index blob or a Merkle proof.
//!
//! Shard `s` of a generation holds every block of the cover whose subtree
//! meets the leaves `[w·s, w·(s+1))`, `w` leaves per shard (a power of
//! two); a node wider than a shard is copied into each shard it spans.
//! The shards span the assigned leaves, `ceil(next_leaf / w)` of them.
//!
//! Layout of a shard (version 1), integers little-endian:
//!
//! | bytes | field |
//! |-------|-------|
//! | 4     | magic `VKD1` |
//! | 1     | version (1) |
//! | 32    | community |
//! | 32    | scope |
//! | 4     | generation |
//! | 4     | epoch |
//! | 4     | shard number |
//! | 4     | leaves per shard |
//! | 4     | count of blocks |
//! | 68 × count | blocks, by ascending node id |
//!
//! A shard of `sd-v1` has the same header under the magic `VKS1` and
//! blocks of 76 bytes (`epoch::seal_sd_block`) by ascending `(i, j)`. The
//! block of `S_{i,j}` goes into every shard that meets `leaves(i) \
//! leaves(j)`; a member at leaf `u` looks up each ancestor `i` of `u`
//! (`depth` binary searches) and takes the block whose `j` is not on its
//! path (`find_sd`).
//!
//! A proof is never inside the bytes it proves, and the event never
//! carries a list of hashes (19.3, 14). Up to `INDEX_MAX` shards a tree,
//! the event names the SHA-256 of an index blob (version 1):
//!
//! | bytes | field |
//! |-------|-------|
//! | 4     | magic `VKI1` |
//! | 1     | version (1) |
//! | 32    | community |
//! | 32    | scope |
//! | 4     | generation |
//! | 4     | epoch |
//! | 4     | leaves per shard |
//! | 4     | count of shards |
//! | 32 × count | SHA-256 of each shard, by number |
//!
//! at most 16 KiB of hashes. Above `INDEX_MAX` the event names the root
//! of a Merkle tree over the shard hashes (leaf `H(0 ‖ h)`, inner `H(1 ‖ l
//! ‖ r)`, an odd last node promoted unchanged), and each shard gets a
//! proof blob `VKP1`. Either way a member reads event → index or proof →
//! its shard.

use sha2::{Digest, Sha256};

use crate::epoch::{block_node, sd_block_subset, Blob, EpochEvent, TreeIndex, TreeRecipe, BLOCK_LEN, SD_BLOCK_LEN};
use crate::tree::Tree;
use crate::{BlobKind, Error, SchemeId, ScopeRef};

/// The magic of a shard.
pub const MAGIC: &[u8; 4] = b"VKD1";
/// The magic of a shard of `sd-v1`.
pub const SD_MAGIC: &[u8; 4] = b"VKS1";
/// The magic of a proof.
pub const PROOF_MAGIC: &[u8; 4] = b"VKP1";
/// The magic of an index.
pub const INDEX_MAGIC: &[u8; 4] = b"VKI1";
/// The version of the shard layout.
pub const SHARD_VERSION: u8 = 1;
/// The version of the index layout.
pub const INDEX_VERSION: u8 = 1;
/// The bytes before the blocks.
pub const SHARD_HEADER_LEN: usize = 89;
/// The bytes before the hashes of an index.
pub const INDEX_HEADER_LEN: usize = 85;
/// Up to this many shards a tree the event names an index blob (16 KiB of
/// hashes); above, a Merkle root with a proof blob per shard.
pub const INDEX_MAX: u32 = 512;

/// SHA-256 of bytes.
pub fn sha256(b: &[u8]) -> [u8; 32] {
    Sha256::digest(b).into()
}

/// The header of a shard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShardHeader {
    pub scope: ScopeRef,
    pub gen: u32,
    pub epoch: u32,
    pub shard_no: u32,
    pub leaves_per_shard: u32,
    pub count: u32,
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}

fn scope_at(b: &[u8]) -> ScopeRef {
    ScopeRef {
        community: b[5..37].try_into().expect("32 bytes"),
        scope: b[37..69].try_into().expect("32 bytes"),
    }
}

/// The scheme a shard's magic names and the length of its blocks.
pub fn shard_scheme(b: &[u8]) -> Option<(SchemeId, usize)> {
    match b.get(..4)? {
        m if m == MAGIC => Some((SchemeId::CsV1, BLOCK_LEN)),
        m if m == SD_MAGIC => Some((SchemeId::SdV1, SD_BLOCK_LEN)),
        _ => None,
    }
}

/// Parses the header of a shard of either scheme and checks that its
/// length matches its count of blocks.
pub fn parse_header(b: &[u8]) -> Result<ShardHeader, Error> {
    parse_header_of(b).map(|(h, _)| h)
}

/// Parses the header of a shard with the scheme its magic names.
pub fn parse_header_of(b: &[u8]) -> Result<(ShardHeader, SchemeId), Error> {
    let Some((scheme, block_len)) = shard_scheme(b) else { return Err(Error::Invalid("shard")) };
    if b.len() < SHARD_HEADER_LEN || b[4] != SHARD_VERSION {
        return Err(Error::Invalid("shard"));
    }
    let h = ShardHeader {
        scope: scope_at(b),
        gen: u32_at(b, 69),
        epoch: u32_at(b, 73),
        shard_no: u32_at(b, 77),
        leaves_per_shard: u32_at(b, 81),
        count: u32_at(b, 85),
    };
    if (b.len() - SHARD_HEADER_LEN) as u64 != u64::from(h.count) * block_len as u64 {
        return Err(Error::Invalid("shard length"));
    }
    Ok((h, scheme))
}

/// Cuts the blocks of one generation's cover (in the order of the
/// recipe) into shards.
pub fn layout(scope: &ScopeRef, tr: &TreeRecipe, epoch: u32, blocks: &[[u8; BLOCK_LEN]]) -> Result<Vec<Blob>, Error> {
    let tree = tr.tree;
    lay_out(
        scope,
        tr,
        epoch,
        MAGIC,
        blocks,
        &move |b| {
            let (s, e) = tree.leaf_range(block_node(b));
            [(s, e), (e, e)]
        },
        &|b| (block_node(b), 0),
    )
}

/// Cuts the blocks of one generation's `sd-v1` cover into shards: the
/// block of `S_{i,j}` into every shard that meets `leaves(i) \
/// leaves(j)`.
pub fn layout_sd(
    scope: &ScopeRef,
    tr: &TreeRecipe,
    epoch: u32,
    blocks: &[[u8; SD_BLOCK_LEN]],
) -> Result<Vec<Blob>, Error> {
    let tree = tr.tree;
    lay_out(
        scope,
        tr,
        epoch,
        SD_MAGIC,
        blocks,
        &move |b| {
            let (i, j) = sd_block_subset(b);
            crate::sd::subset_ranges(tree, i, j)
        },
        &|b| sd_block_subset(b),
    )
}

/// The leaf ranges a block reaches (at most two, either may be empty).
type BlockRanges<const N: usize> = dyn Fn(&[u8; N]) -> [(u64, u64); 2];
/// The key blocks are sorted by within a shard.
type BlockOrder<const N: usize> = dyn Fn(&[u8; N]) -> (u64, u64);

/// The shards of blocks of `N` bytes: each block into every shard that
/// meets one of its (at most two) leaf ranges, once; the blocks of a
/// shard by ascending `order`.
fn lay_out<const N: usize>(
    scope: &ScopeRef,
    tr: &TreeRecipe,
    epoch: u32,
    magic: &[u8; 4],
    blocks: &[[u8; N]],
    ranges: &BlockRanges<N>,
    order: &BlockOrder<N>,
) -> Result<Vec<Blob>, Error> {
    let w = u64::from(tr.leaves_per_shard);
    if !w.is_power_of_two() || w > tr.tree.capacity() {
        return Err(Error::Params("leaves per shard"));
    }
    let count = tr.next_leaf.div_ceil(w);
    u32::try_from(count).map_err(|_| Error::Params("too many shards"))?;
    let mut members: Vec<Vec<u32>> = vec![Vec::new(); count as usize];
    for (i, b) in blocks.iter().enumerate() {
        for (s, e) in ranges(b) {
            if s >= e || s / w >= count {
                continue;
            }
            let last = ((e - 1) / w).min(count - 1);
            for shard in (s / w)..=last {
                let list = &mut members[shard as usize];
                if list.last() != Some(&(i as u32)) {
                    list.push(i as u32);
                }
            }
        }
    }
    let mut out = Vec::with_capacity(count as usize);
    for (no, mut idx) in members.into_iter().enumerate() {
        idx.sort_unstable_by_key(|&i| order(&blocks[i as usize]));
        let mut bytes = Vec::with_capacity(SHARD_HEADER_LEN + N * idx.len());
        bytes.extend_from_slice(magic);
        bytes.push(SHARD_VERSION);
        bytes.extend_from_slice(&scope.community);
        bytes.extend_from_slice(&scope.scope);
        bytes.extend_from_slice(&tr.gen.to_le_bytes());
        bytes.extend_from_slice(&epoch.to_le_bytes());
        bytes.extend_from_slice(&(no as u32).to_le_bytes());
        bytes.extend_from_slice(&tr.leaves_per_shard.to_le_bytes());
        bytes.extend_from_slice(&(idx.len() as u32).to_le_bytes());
        for i in idx {
            bytes.extend_from_slice(&blocks[i as usize]);
        }
        let sha256 = sha256(&bytes);
        out.push(Blob { kind: BlobKind::Shard, gen: tr.gen, epoch, no: no as u32, sha256, bytes });
    }
    Ok(out)
}

/// The index of one generation's shards in one epoch: the hashes of the
/// shards by number, with what binds them to their place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShardIndex {
    pub scope: ScopeRef,
    pub gen: u32,
    pub epoch: u32,
    pub leaves_per_shard: u32,
    pub hashes: Vec<[u8; 32]>,
}

impl ShardIndex {
    /// The bytes of the layout `VKI1` above.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(INDEX_HEADER_LEN + 32 * self.hashes.len());
        out.extend_from_slice(INDEX_MAGIC);
        out.push(INDEX_VERSION);
        out.extend_from_slice(&self.scope.community);
        out.extend_from_slice(&self.scope.scope);
        out.extend_from_slice(&self.gen.to_le_bytes());
        out.extend_from_slice(&self.epoch.to_le_bytes());
        out.extend_from_slice(&self.leaves_per_shard.to_le_bytes());
        out.extend_from_slice(&(self.hashes.len() as u32).to_le_bytes());
        for h in &self.hashes {
            out.extend_from_slice(h);
        }
        out
    }

    /// Parses an index blob.
    pub fn from_bytes(b: &[u8]) -> Result<Self, Error> {
        let (scope, gen, epoch, leaves_per_shard, count) = index_header(b)?;
        let hashes = b[INDEX_HEADER_LEN..].as_chunks::<32>().0.to_vec();
        debug_assert_eq!(hashes.len(), count as usize);
        Ok(ShardIndex { scope, gen, epoch, leaves_per_shard, hashes })
    }
}

/// The header of an index blob, its length checked against its count.
fn index_header(b: &[u8]) -> Result<(ScopeRef, u32, u32, u32, u32), Error> {
    if b.len() < INDEX_HEADER_LEN || &b[..4] != INDEX_MAGIC || b[4] != INDEX_VERSION {
        return Err(Error::Invalid("index"));
    }
    let count = u32_at(b, 81);
    if count > INDEX_MAX || (b.len() - INDEX_HEADER_LEN) != 32 * count as usize {
        return Err(Error::Invalid("index length"));
    }
    Ok((scope_at(b), u32_at(b, 69), u32_at(b, 73), u32_at(b, 77), count))
}

/// The proof of one shard against a Merkle root: the sibling hashes from
/// the shard's leaf up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proof {
    pub gen: u32,
    pub epoch: u32,
    pub shard_no: u32,
    pub path: Vec<[u8; 32]>,
}

impl Proof {
    /// The bytes of the proof blob: magic `VKP1`, generation, epoch,
    /// shard number (u32 LE), the number of hashes (u8), the hashes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(17 + 32 * self.path.len());
        out.extend_from_slice(PROOF_MAGIC);
        out.extend_from_slice(&self.gen.to_le_bytes());
        out.extend_from_slice(&self.epoch.to_le_bytes());
        out.extend_from_slice(&self.shard_no.to_le_bytes());
        out.push(self.path.len() as u8);
        for h in &self.path {
            out.extend_from_slice(h);
        }
        out
    }

    /// Parses a proof blob.
    pub fn from_bytes(b: &[u8]) -> Result<Self, Error> {
        if b.len() < 17 || &b[..4] != PROOF_MAGIC {
            return Err(Error::Invalid("proof"));
        }
        let n = usize::from(b[16]);
        if b.len() != 17 + 32 * n {
            return Err(Error::Invalid("proof"));
        }
        Ok(Proof {
            gen: u32_at(b, 4),
            epoch: u32_at(b, 8),
            shard_no: u32_at(b, 12),
            path: b[17..].as_chunks::<32>().0.to_vec(),
        })
    }
}

fn leaf_hash(h: &[u8; 32]) -> [u8; 32] {
    let mut d = Sha256::new();
    d.update([0u8]);
    d.update(h);
    d.finalize().into()
}

fn node_hash(l: &[u8; 32], r: &[u8; 32]) -> [u8; 32] {
    let mut d = Sha256::new();
    d.update([1u8]);
    d.update(l);
    d.update(r);
    d.finalize().into()
}

/// The levels of the Merkle tree over shard hashes, leaves first.
fn merkle_levels(hashes: &[[u8; 32]]) -> Vec<Vec<[u8; 32]>> {
    let mut levels = vec![hashes.iter().map(leaf_hash).collect::<Vec<_>>()];
    while levels.last().expect("one level").len() > 1 {
        let up = levels
            .last()
            .expect("one level")
            .chunks(2)
            .map(|p| if p.len() == 2 { node_hash(&p[0], &p[1]) } else { p[0] })
            .collect();
        levels.push(up);
    }
    levels
}

/// The Merkle root over shard hashes (`None` for no shards).
pub fn merkle_root(hashes: &[[u8; 32]]) -> Option<[u8; 32]> {
    if hashes.is_empty() {
        return None;
    }
    Some(merkle_levels(hashes).last().expect("one level")[0])
}

/// Checks the hash of shard `index` of `count` against a root.
pub fn merkle_verify(root: &[u8; 32], count: u32, index: u32, hash: &[u8; 32], path: &[[u8; 32]]) -> bool {
    if index >= count {
        return false;
    }
    let (mut h, mut i, mut n, mut p) = (leaf_hash(hash), index, count, 0usize);
    while n > 1 {
        if (i ^ 1) < n {
            let Some(sib) = path.get(p) else { return false };
            p += 1;
            h = if i & 1 == 0 { node_hash(&h, sib) } else { node_hash(sib, &h) };
        }
        i >>= 1;
        n = n.div_ceil(2);
    }
    p == path.len() && &h == root
}

/// The entry of a generation's shards for the event, with what proves
/// them: one index blob, or a proof blob per shard above `INDEX_MAX`.
pub fn tree_index(scope: &ScopeRef, tr: &TreeRecipe, epoch: u32, shards: &[Blob]) -> (TreeIndex, Vec<Blob>) {
    let hashes: Vec<[u8; 32]> = shards.iter().map(|s| s.sha256).collect();
    let count = hashes.len() as u32;
    let mut ti = TreeIndex {
        gen: tr.gen,
        depth: tr.tree.depth(),
        leaves: tr.leaves_per_shard,
        count,
        index: None,
        root: None,
    };
    if count <= INDEX_MAX {
        let bytes =
            ShardIndex { scope: *scope, gen: tr.gen, epoch, leaves_per_shard: tr.leaves_per_shard, hashes }.to_bytes();
        let h = sha256(&bytes);
        ti.index = Some(h);
        return (ti, vec![Blob { kind: BlobKind::Index, gen: tr.gen, epoch, no: 0, sha256: h, bytes }]);
    }
    let levels = merkle_levels(&hashes);
    ti.root = Some(levels.last().expect("one level")[0]);
    let proofs = (0..count)
        .map(|no| {
            let mut path = Vec::new();
            let mut i = no as usize;
            for level in &levels[..levels.len() - 1] {
                if let Some(sib) = level.get(i ^ 1) {
                    path.push(*sib);
                }
                i >>= 1;
            }
            let bytes = Proof { gen: tr.gen, epoch, shard_no: no, path }.to_bytes();
            Blob { kind: BlobKind::Proof, gen: tr.gen, epoch, no, sha256: sha256(&bytes), bytes }
        })
        .collect();
    (ti, proofs)
}

/// Checks shard `shard_no` of generation `gen` against the event: its
/// hash through the witness (the index blob the event names, or the
/// shard's proof against the event's root) and its header.
pub fn verify(ev: &EpochEvent, gen: u32, shard_no: u32, bytes: &[u8], witness: &[u8]) -> Result<ShardHeader, Error> {
    let ti = ev.tree(gen).ok_or(Error::Invalid("generation of the shard"))?;
    if shard_no >= ti.count {
        return Err(Error::Invalid("shard number"));
    }
    let h = sha256(bytes);
    match (&ti.index, &ti.root) {
        (Some(index), _) => {
            if &sha256(witness) != index {
                return Err(Error::Invalid("index hash"));
            }
            let (scope, igen, iepoch, leaves, count) = index_header(witness)?;
            if scope != ev.scope() || igen != gen || iepoch != ev.epoch || leaves != ti.leaves || count != ti.count {
                return Err(Error::Invalid("index header"));
            }
            let at = INDEX_HEADER_LEN + 32 * shard_no as usize;
            if witness[at..at + 32] != h {
                return Err(Error::Invalid("shard hash"));
            }
        }
        (None, Some(root)) => {
            let p = Proof::from_bytes(witness)?;
            if p.gen != gen || p.epoch != ev.epoch || p.shard_no != shard_no {
                return Err(Error::Invalid("proof"));
            }
            if !merkle_verify(root, ti.count, shard_no, &h, &p.path) {
                return Err(Error::Invalid("shard proof"));
            }
        }
        (None, None) => return Err(Error::Invalid("event without shard index")),
    }
    let (hdr, scheme) = parse_header_of(bytes)?;
    if scheme != ev.scheme {
        return Err(Error::Invalid("shard of another scheme"));
    }
    if hdr.scope != ev.scope()
        || hdr.gen != gen
        || hdr.epoch != ev.epoch
        || hdr.shard_no != shard_no
        || hdr.leaves_per_shard != ti.leaves
    {
        return Err(Error::Invalid("shard header"));
    }
    Ok(hdr)
}

/// Finds the first of `path` (the nodes of a member's path) among the
/// blocks of a parsed shard: the block and the number of nodes checked
/// (at most `depth + 1`).
pub fn find(bytes: &[u8], path: impl Iterator<Item = u64>) -> (Option<&[u8]>, usize) {
    let blocks = &bytes[SHARD_HEADER_LEN.min(bytes.len())..];
    let n = blocks.len() / BLOCK_LEN;
    let at = |i: usize| &blocks[i * BLOCK_LEN..(i + 1) * BLOCK_LEN];
    let mut checks = 0;
    for node in path {
        checks += 1;
        let (mut lo, mut hi) = (0usize, n);
        while lo < hi {
            let mid = (lo + hi) / 2;
            match block_node(at(mid)).cmp(&node) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return (Some(at(mid)), checks),
            }
        }
    }
    (None, checks)
}

/// Finds the block of an `sd-v1` shard whose subset holds `leaf`: for
/// each ancestor `i` of the leaf, root first, a binary search for the
/// blocks of `i`, then a look at their `j`. Returns the block and the
/// number of checks (one per ancestor looked up, one more per further
/// block of the same `i`; a cover names each `i` once, so at most
/// `depth`).
pub fn find_sd(bytes: &[u8], tree: Tree, leaf: u32) -> (Option<&[u8]>, usize) {
    let blocks = &bytes[SHARD_HEADER_LEN.min(bytes.len())..];
    let n = blocks.len() / SD_BLOCK_LEN;
    let at = |i: usize| &blocks[i * SD_BLOCK_LEN..(i + 1) * SD_BLOCK_LEN];
    let d = u32::from(tree.depth());
    let u = tree.leaf_node(leaf);
    let mut checks = 0;
    for li in 0..d {
        let i = u >> (d - li);
        checks += 1;
        let (mut lo, mut hi) = (0usize, n);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if sd_block_subset(at(mid)).0 < i {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let mut first = true;
        while lo < n {
            let (bi, bj) = sd_block_subset(at(lo));
            if bi != i {
                break;
            }
            if !first {
                checks += 1;
            }
            first = false;
            if crate::sd::is_subset(tree, bi, bj) && crate::sd::subset_contains(tree, bi, bj, u64::from(leaf)) {
                return (Some(at(lo)), checks);
            }
            lo += 1;
        }
    }
    (None, checks)
}

/// The blocks of a shard of either scheme, after its header, with the
/// scheme its magic names.
pub fn blocks(bytes: &[u8]) -> Result<(SchemeId, std::slice::ChunksExact<'_, u8>), Error> {
    let (_, scheme) = parse_header_of(bytes)?;
    let len = if scheme == SchemeId::SdV1 { SD_BLOCK_LEN } else { BLOCK_LEN };
    Ok((scheme, bytes[SHARD_HEADER_LEN..].chunks_exact(len)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sd_shards_hold_exactly_the_subsets_that_meet_them() {
        use crate::epoch::seal_sd_block;
        use crate::rng::XorShift64;
        use crate::{EpochKey, Key32};
        let scope = ScopeRef { community: [1; 32], scope: [2; 32] };
        let tree = Tree::new(6).unwrap();
        let mut rng = XorShift64::new(3);
        for (next, w) in [(64u64, 8u32), (60, 8), (37, 4), (5, 16), (64, 1)] {
            let revoked: std::collections::BTreeSet<u32> = (0..6).map(|_| rng.below(64) as u32).collect();
            let cover = crate::cover::sd_cover(tree, next, &revoked);
            let tr = TreeRecipe::new_sd(1, tree, w, next, &cover);
            let k = EpochKey { epoch: 1, key: Key32([9; 32]) };
            let blocks: Vec<[u8; SD_BLOCK_LEN]> =
                cover.iter().map(|&(i, j)| seal_sd_block(&Key32([0; 32]), &[0; 12], &scope, 1, &k, i, j)).collect();
            let shards = layout_sd(&scope, &tr, 1, &blocks).unwrap();
            assert_eq!(shards.len() as u64, next.div_ceil(u64::from(w)));
            for s in &shards {
                let (lo, hi) = (u64::from(s.no) * u64::from(w), u64::from(s.no + 1) * u64::from(w));
                let (scheme, held) = super::blocks(&s.bytes).unwrap();
                assert_eq!(scheme, SchemeId::SdV1);
                let held: Vec<(u64, u64)> = held.map(sd_block_subset).collect();
                let want: Vec<(u64, u64)> = {
                    let mut v: Vec<(u64, u64)> = cover
                        .iter()
                        .copied()
                        .filter(|&(i, j)| (lo..hi).any(|l| crate::sd::subset_contains(tree, i, j, l)))
                        .collect();
                    v.sort_unstable();
                    v
                };
                assert_eq!(held, want, "next {next} w {w} shard {}", s.no);
                for l in lo..hi.min(next) {
                    let (found, checks) = find_sd(&s.bytes, tree, l as u32);
                    assert!(checks <= 6);
                    assert_eq!(found.is_some(), !revoked.contains(&(l as u32)), "leaf {l}");
                }
            }
        }
    }

    #[test]
    fn merkle_proves_every_index_of_every_count() {
        for count in 1..40u32 {
            let hashes: Vec<[u8; 32]> = (0..count).map(|i| sha256(&i.to_le_bytes())).collect();
            let root = merkle_root(&hashes).unwrap();
            let levels = merkle_levels(&hashes);
            for i in 0..count {
                let mut path = Vec::new();
                let mut j = i as usize;
                for level in &levels[..levels.len() - 1] {
                    if let Some(s) = level.get(j ^ 1) {
                        path.push(*s);
                    }
                    j >>= 1;
                }
                assert!(merkle_verify(&root, count, i, &hashes[i as usize], &path));
                let other = (i + 1) % count;
                if other != i {
                    assert!(!merkle_verify(&root, count, other, &hashes[i as usize], &path));
                }
            }
        }
    }

    #[test]
    fn proof_and_index_bytes_round_trip() {
        let p = Proof { gen: 1, epoch: 2, shard_no: 3, path: vec![[4; 32], [5; 32]] };
        assert_eq!(Proof::from_bytes(&p.to_bytes()).unwrap(), p);
        assert!(Proof::from_bytes(&p.to_bytes()[..40]).is_err());
        let scope = ScopeRef { community: [1; 32], scope: [2; 32] };
        let ix = ShardIndex { scope, gen: 3, epoch: 9, leaves_per_shard: 4096, hashes: vec![[7; 32]; 512] };
        let b = ix.to_bytes();
        assert_eq!(b.len(), INDEX_HEADER_LEN + 512 * 32);
        assert_eq!(&b[..4], INDEX_MAGIC);
        assert_eq!(ShardIndex::from_bytes(&b).unwrap(), ix);
        assert!(ShardIndex::from_bytes(&b[..b.len() - 1]).is_err());
        let mut long = ix.clone();
        long.hashes.push([8; 32]);
        assert!(ShardIndex::from_bytes(&long.to_bytes()).is_err(), "more than INDEX_MAX hashes");
        let empty = ShardIndex { hashes: vec![], ..ix };
        assert_eq!(ShardIndex::from_bytes(&empty.to_bytes()).unwrap(), empty);
    }
}
