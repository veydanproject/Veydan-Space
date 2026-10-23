// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Epochs: the blocks that carry an epoch key under node keys, the chain
//! of keys of an open history, the epoch event, and the saved recipe of
//! an epoch from which the same bytes are built again.
//!
//! A block (specification 5) is `node_id (u64 LE) ‖ nonce (12 bytes) ‖
//! AES-256-GCM(K_node, nonce, aad, K_epoch)`, 68 bytes, with `aad =
//! community ‖ scope ‖ gen (u32 LE) ‖ epoch (u32 LE) ‖ node_id (u64 LE)`.
//! The nonce travels in the block; the keeper derives it (19.1, 3) from
//! one random seed of 32 bytes drawn for every build of an epoch:
//! `nonce = SHA-256("veydan.keydist.nonce.v1" ‖ seed ‖ node_id (u64
//! LE))[..12]`. A keeper that built an epoch, failed and built it again
//! draws a new seed (and a new key), so no nonce repeats under a node key;
//! the recipe it saves holds the key, the seed and the bare node ids.
//!
//! A block of `sd-v1` names a subset instead of a node: `i (u64 LE) ‖ j
//! (u64 LE) ‖ nonce (12) ‖ AES-256-GCM(K_{i,j}, nonce, aad, K_epoch)`, 76
//! bytes, with `aad = community ‖ scope ‖ gen ‖ epoch ‖ i ‖ j` (88 bytes)
//! and `nonce = SHA-256("veydan.keydist.nonce.v1" ‖ seed ‖ i ‖ j)[..12]`.
//!
//! The chain of an open history is `nonce (12) ‖ AES-256-GCM(K_chain,
//! nonce, aad = community ‖ scope ‖ epoch ‖ prev_epoch, K_prev)`, 60 bytes,
//! in the event, with `K_chain = kdf::chain_key(K_epoch)` (19.1, 4).
//!
//! The event (version 2) names, per generation, either the SHA-256 of an
//! index blob that lists the hashes of the shards (`index`, up to
//! `shard::INDEX_MAX` shards) or the root of a Merkle tree over them
//! (`root`, each shard then has a proof blob): the event stays a few
//! hundred bytes however many shards there are (19.3, 14).
//!
//! An epoch, once built, is kept as an `EpochRecipe` until the host says
//! it is published: built again from the recipe it gives the very same
//! bytes, and its number is never given to another epoch.

use aes_gcm::aead::{Aead, KeyInit, Nonce, Payload};
use aes_gcm::Aes256Gcm;
use sha2::{Digest, Sha256};

use crate::hexser;
use crate::kdf::{chain_key, NodeKeys};
use crate::shard;
use crate::tree::Tree;
use crate::{
    BlobKind, EpochKey, Error, History, Key32, MemberId, Outbound, SchemeId, ScopeRef, SecretBytes,
    Verified,
};

/// The bytes of one block.
pub const BLOCK_LEN: usize = 68;
/// The bytes of one block of `sd-v1`.
pub const SD_BLOCK_LEN: usize = 76;
/// The bytes of the chain.
pub const CHAIN_LEN: usize = 60;
/// The version of the event.
pub const EVENT_VERSION: u8 = 2;
/// The domain of the nonces of blocks.
pub const NONCE_INFO: &[u8] = b"veydan.keydist.nonce.v1";
/// The bytes of one entry of a saved cover: the node id.
const COVER_ENTRY: usize = 8;
/// The bytes of one entry of a saved `sd-v1` cover: `i` and `j`.
const SD_COVER_ENTRY: usize = 16;

fn cipher(key: &Key32) -> Aes256Gcm {
    Aes256Gcm::new_from_slice(&key.0).expect("a key of 32 bytes")
}

/// The nonces of one build of an epoch: one hash state over the domain
/// and the seed, cloned for every block.
#[derive(Clone)]
pub struct Nonces(Sha256);

impl Nonces {
    /// The nonces from the seed of a recipe.
    pub fn new(seed: &Key32) -> Self {
        let mut d = Sha256::new();
        d.update(NONCE_INFO);
        d.update(seed.0);
        Nonces(d)
    }

    /// The nonce of the block of a `cs-v1` node.
    pub fn node(&self, node: u64) -> [u8; 12] {
        let mut d = self.0.clone();
        d.update(node.to_le_bytes());
        d.finalize()[..12].try_into().expect("12 bytes")
    }

    /// The nonce of the block of an `sd-v1` subset.
    pub fn subset(&self, i: u64, j: u64) -> [u8; 12] {
        let mut d = self.0.clone();
        d.update(i.to_le_bytes());
        d.update(j.to_le_bytes());
        d.finalize()[..12].try_into().expect("12 bytes")
    }
}

fn block_aad(scope: &ScopeRef, gen: u32, epoch: u32, node: u64) -> [u8; 80] {
    let mut aad = [0u8; 80];
    aad[..32].copy_from_slice(&scope.community);
    aad[32..64].copy_from_slice(&scope.scope);
    aad[64..68].copy_from_slice(&gen.to_le_bytes());
    aad[68..72].copy_from_slice(&epoch.to_le_bytes());
    aad[72..80].copy_from_slice(&node.to_le_bytes());
    aad
}

/// Seals an epoch key under a node key into a block.
pub fn seal_block(
    node_key: &Key32,
    nonce: &[u8; 12],
    scope: &ScopeRef,
    gen: u32,
    k: &EpochKey,
    node: u64,
) -> [u8; BLOCK_LEN] {
    let aad = block_aad(scope, gen, k.epoch, node);
    let ct = cipher(node_key)
        .encrypt(&Nonce::<Aes256Gcm>::from(*nonce), Payload { msg: &k.key.0, aad: &aad })
        .expect("AES-GCM seals 32 bytes");
    let mut block = [0u8; BLOCK_LEN];
    block[..8].copy_from_slice(&node.to_le_bytes());
    block[8..20].copy_from_slice(nonce);
    block[20..].copy_from_slice(&ct);
    block
}

/// The node id a block names.
pub fn block_node(block: &[u8]) -> u64 {
    u64::from_le_bytes(block[..8].try_into().expect("a block holds 8 bytes of node id"))
}

/// Opens a block with the key of its node.
pub fn open_block(
    node_key: &Key32,
    block: &[u8],
    scope: &ScopeRef,
    gen: u32,
    epoch: u32,
) -> Result<EpochKey, Error> {
    if block.len() != BLOCK_LEN {
        return Err(Error::Invalid("block"));
    }
    let node = block_node(block);
    let aad = block_aad(scope, gen, epoch, node);
    let nonce: [u8; 12] = block[8..20].try_into().expect("12 bytes");
    let pt = cipher(node_key)
        .decrypt(&Nonce::<Aes256Gcm>::from(nonce), Payload { msg: &block[20..], aad: &aad })
        .map_err(|_| Error::Invalid("block"))?;
    let key: [u8; 32] = pt.as_slice().try_into().map_err(|_| Error::Invalid("block"))?;
    Ok(EpochKey { epoch, key: Key32(key) })
}

fn sd_block_aad(scope: &ScopeRef, gen: u32, epoch: u32, i: u64, j: u64) -> [u8; 88] {
    let mut aad = [0u8; 88];
    aad[..80].copy_from_slice(&block_aad(scope, gen, epoch, i));
    aad[80..].copy_from_slice(&j.to_le_bytes());
    aad
}

/// Seals an epoch key under the key of subset `(i, j)` into an `sd-v1`
/// block.
pub fn seal_sd_block(
    subset_key: &Key32,
    nonce: &[u8; 12],
    scope: &ScopeRef,
    gen: u32,
    k: &EpochKey,
    i: u64,
    j: u64,
) -> [u8; SD_BLOCK_LEN] {
    let aad = sd_block_aad(scope, gen, k.epoch, i, j);
    let ct = cipher(subset_key)
        .encrypt(&Nonce::<Aes256Gcm>::from(*nonce), Payload { msg: &k.key.0, aad: &aad })
        .expect("AES-GCM seals 32 bytes");
    let mut block = [0u8; SD_BLOCK_LEN];
    block[..8].copy_from_slice(&i.to_le_bytes());
    block[8..16].copy_from_slice(&j.to_le_bytes());
    block[16..28].copy_from_slice(nonce);
    block[28..].copy_from_slice(&ct);
    block
}

/// The subset `(i, j)` an `sd-v1` block names.
pub fn sd_block_subset(block: &[u8]) -> (u64, u64) {
    (
        u64::from_le_bytes(block[..8].try_into().expect("a block holds 8 bytes of i")),
        u64::from_le_bytes(block[8..16].try_into().expect("a block holds 8 bytes of j")),
    )
}

/// Opens an `sd-v1` block with the key of its subset.
pub fn open_sd_block(
    subset_key: &Key32,
    block: &[u8],
    scope: &ScopeRef,
    gen: u32,
    epoch: u32,
) -> Result<EpochKey, Error> {
    if block.len() != SD_BLOCK_LEN {
        return Err(Error::Invalid("block"));
    }
    let (i, j) = sd_block_subset(block);
    let aad = sd_block_aad(scope, gen, epoch, i, j);
    let nonce: [u8; 12] = block[16..28].try_into().expect("12 bytes");
    let pt = cipher(subset_key)
        .decrypt(&Nonce::<Aes256Gcm>::from(nonce), Payload { msg: &block[28..], aad: &aad })
        .map_err(|_| Error::Invalid("block"))?;
    let key: [u8; 32] = pt.as_slice().try_into().map_err(|_| Error::Invalid("block"))?;
    Ok(EpochKey { epoch, key: Key32(key) })
}

fn chain_aad(scope: &ScopeRef, epoch: u32, prev: u32) -> [u8; 72] {
    let mut aad = [0u8; 72];
    aad[..32].copy_from_slice(&scope.community);
    aad[32..64].copy_from_slice(&scope.scope);
    aad[64..68].copy_from_slice(&epoch.to_le_bytes());
    aad[68..72].copy_from_slice(&prev.to_le_bytes());
    aad
}

/// Seals the previous key under the chain key of the new one.
pub fn seal_chain(k: &EpochKey, prev: &EpochKey, nonce: &[u8; 12], scope: &ScopeRef) -> Vec<u8> {
    let aad = chain_aad(scope, k.epoch, prev.epoch);
    let ct = cipher(&chain_key(k, scope))
        .encrypt(&Nonce::<Aes256Gcm>::from(*nonce), Payload { msg: &prev.key.0, aad: &aad })
        .expect("AES-GCM seals 32 bytes");
    let mut out = Vec::with_capacity(CHAIN_LEN);
    out.extend_from_slice(nonce);
    out.extend_from_slice(&ct);
    out
}

/// The previous key of an open history, from the chain of an event and
/// the key of that event's epoch.
pub fn open_chain(ev: &EpochEvent, k: &EpochKey) -> Result<EpochKey, Error> {
    let (Some(chain), Some(prev)) = (ev.chain.as_ref(), ev.prev_epoch) else {
        return Err(Error::Invalid("no chain"));
    };
    if k.epoch != ev.epoch || chain.len() != CHAIN_LEN {
        return Err(Error::Invalid("chain"));
    }
    let scope = ev.scope();
    let aad = chain_aad(&scope, ev.epoch, prev);
    let nonce: [u8; 12] = chain[..12].try_into().expect("12 bytes");
    let pt = cipher(&chain_key(k, &scope))
        .decrypt(&Nonce::<Aes256Gcm>::from(nonce), Payload { msg: &chain[12..], aad: &aad })
        .map_err(|_| Error::Invalid("chain"))?;
    let key: [u8; 32] = pt.as_slice().try_into().map_err(|_| Error::Invalid("chain"))?;
    Ok(EpochKey { epoch: prev, key: Key32(key) })
}

/// Where the shards of one generation of an epoch are and how to check
/// them: the SHA-256 of the index blob that lists the hashes of all
/// shards (`index`, at most `shard::INDEX_MAX` shards), else the Merkle
/// root over those hashes (`root`; each shard then has a proof blob).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TreeIndex {
    pub gen: u32,
    pub depth: u8,
    /// Leaves per shard.
    pub leaves: u32,
    /// The number of shards: they span the assigned leaves.
    pub count: u32,
    #[serde(with = "hexser::opt", default, skip_serializing_if = "Option::is_none")]
    pub index: Option<[u8; 32]>,
    #[serde(with = "hexser::opt", default, skip_serializing_if = "Option::is_none")]
    pub root: Option<[u8; 32]>,
}

/// The epoch event: what the keeper signs and puts on the relays. The
/// signature is the host's; a member acts on an event only as
/// `Verified<EpochEvent>`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EpochEvent {
    pub v: u8,
    pub scheme: SchemeId,
    #[serde(with = "hexser")]
    pub community_id: [u8; 32],
    #[serde(with = "hexser")]
    pub scope_id: [u8; 32],
    pub epoch: u32,
    pub prev_epoch: Option<u32>,
    /// The keeper's clock when the epoch was built, seconds.
    pub at: u64,
    pub history: History,
    /// One entry per live generation: two during the grace of a planned
    /// migration, else one; none for `flat-v1`.
    pub trees: Vec<TreeIndex>,
    /// The previous key under this one, under `history = open`.
    #[serde(with = "hexser::opt", default, skip_serializing_if = "Option::is_none")]
    pub chain: Option<Vec<u8>>,
}

impl EpochEvent {
    /// The scope the event is of.
    pub fn scope(&self) -> ScopeRef {
        ScopeRef { community: self.community_id, scope: self.scope_id }
    }

    /// The index of a generation's shards, if the event carries it.
    pub fn tree(&self, gen: u32) -> Option<&TreeIndex> {
        self.trees.iter().find(|t| t.gen == gen)
    }

    /// The newest generation the event carries.
    pub fn newest_gen(&self) -> Option<u32> {
        self.trees.iter().map(|t| t.gen).max()
    }
}

/// The cover of one generation as it was built: the tree, the shard
/// width, the assigned leaves, and the ids of the blocks.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TreeRecipe {
    pub gen: u32,
    pub tree: Tree,
    pub leaves_per_shard: u32,
    pub next_leaf: u64,
    /// `cs-v1`: packed node ids (u64 LE), 8 bytes each, in the order of
    /// the leaves. `sd-v1`: `i ‖ j` (u64 LE each), 16 bytes, in the order
    /// of the cover. The scheme is the one of the `EpochRecipe` that holds
    /// it; the nonces come from its seed.
    #[serde(with = "hexser")]
    pub cover: Vec<u8>,
}

impl TreeRecipe {
    /// The number of nodes of a `cs-v1` cover.
    pub fn nodes(&self) -> usize {
        self.cover.len() / COVER_ENTRY
    }

    /// The number of subsets of an `sd-v1` cover.
    pub fn subsets(&self) -> usize {
        self.cover.len() / SD_COVER_ENTRY
    }

    /// The number of blocks of the cover under a scheme.
    pub fn blocks(&self, scheme: SchemeId) -> usize {
        match scheme {
            SchemeId::SdV1 => self.subsets(),
            _ => self.nodes(),
        }
    }

    /// Lays out an `sd-v1` cover (`cover::sd_cover`).
    pub fn new_sd(gen: u32, tree: Tree, leaves_per_shard: u32, next_leaf: u64, subsets: &[(u64, u64)]) -> Self {
        let mut cover = Vec::with_capacity(SD_COVER_ENTRY * subsets.len());
        for (i, j) in subsets {
            cover.extend_from_slice(&i.to_le_bytes());
            cover.extend_from_slice(&j.to_le_bytes());
        }
        TreeRecipe { gen, tree, leaves_per_shard, next_leaf, cover }
    }

    /// Lays out a `cs-v1` cover (`cover::cs_cover`).
    pub fn new(gen: u32, tree: Tree, leaves_per_shard: u32, next_leaf: u64, nodes: &[u64]) -> Self {
        let mut cover = Vec::with_capacity(COVER_ENTRY * nodes.len());
        for n in nodes {
            cover.extend_from_slice(&n.to_le_bytes());
        }
        TreeRecipe { gen, tree, leaves_per_shard, next_leaf, cover }
    }
}

/// Everything an epoch is built from; the keeper saves it until the
/// epoch is published.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EpochRecipe {
    pub scheme: SchemeId,
    pub key: EpochKey,
    /// The seed of the nonces of the blocks, random for every build.
    pub seed: Key32,
    pub prev_epoch: Option<u32>,
    pub at: u64,
    pub history: History,
    #[serde(with = "hexser::opt", default, skip_serializing_if = "Option::is_none")]
    pub chain: Option<Vec<u8>>,
    pub trees: Vec<TreeRecipe>,
    /// `flat-v1`: the members the key goes to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recipients: Vec<MemberId>,
    /// The hashes of the blobs this epoch makes obsolete: those of the
    /// epoch published before it and of any epoch it superseded.
    #[serde(with = "hexser::list", default, skip_serializing_if = "Vec::is_empty")]
    pub retires: Vec<[u8; 32]>,
}

/// One blob of an epoch: a shard, the proof of a shard, or the index of
/// a generation's shards.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blob {
    pub kind: BlobKind,
    pub gen: u32,
    pub epoch: u32,
    /// The shard number (`Index`: 0).
    pub no: u32,
    pub sha256: [u8; 32],
    pub bytes: Vec<u8>,
}

/// An epoch, built: the event, the blobs and the secrets of single
/// members, ready to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpochOut {
    /// The event; the keeper built it, so it is its own verified event.
    pub event: Verified<EpochEvent>,
    /// The shards of every generation of the event, then the index or
    /// the proofs of each generation.
    pub blobs: Vec<Blob>,
    /// Secrets of single members (`flat-v1`: every member; the tree
    /// schemes: none, the shards reach everybody).
    pub secrets: Vec<(MemberId, SecretBytes)>,
    /// The epoch key itself, for the keeper's own use.
    pub key: EpochKey,
    /// The size of the cover per generation, `(gen, blocks)`: nodes for
    /// `cs-v1`, subsets for `sd-v1`.
    pub cover: Vec<(u32, usize)>,
    /// The hashes of the blobs this epoch makes obsolete (those of the
    /// previous epoch and of a superseded one), for the garbage
    /// collection of the store. Under `from_join` the host keeps them as
    /// long as offline members may still catch up with them.
    pub retires: Vec<[u8; 32]>,
}

impl EpochOut {
    /// What the host sends, borrowed: the blobs first, then the secrets,
    /// the event last.
    pub fn outbound(&self) -> Vec<Outbound<'_>> {
        let mut out = Vec::with_capacity(self.blobs.len() + self.secrets.len() + 1);
        for b in &self.blobs {
            out.push(Outbound::Blob {
                kind: b.kind,
                gen: b.gen,
                epoch: b.epoch,
                no: b.no,
                sha256: b.sha256,
                bytes: &b.bytes,
            });
        }
        for (to, bytes) in &self.secrets {
            out.push(Outbound::Secret { to: *to, bytes });
        }
        out.push(Outbound::Event(&self.event));
        out
    }

    fn blob(&self, kind: BlobKind, gen: u32, no: u32) -> Option<&Blob> {
        self.blobs.iter().find(|b| b.kind == kind && b.gen == gen && b.no == no)
    }

    /// A shard by generation and number.
    pub fn shard(&self, gen: u32, no: u32) -> Option<&Blob> {
        self.blob(BlobKind::Shard, gen, no)
    }

    /// The proof of a shard, in Merkle mode.
    pub fn proof(&self, gen: u32, no: u32) -> Option<&Blob> {
        self.blob(BlobKind::Proof, gen, no)
    }

    /// The index blob of a generation, in index mode.
    pub fn index(&self, gen: u32) -> Option<&Blob> {
        self.blob(BlobKind::Index, gen, 0)
    }

    /// What a member checks shard `no` of `gen` with: the index blob or
    /// the shard's proof, as the event says.
    pub fn witness(&self, gen: u32, no: u32) -> Option<&Blob> {
        self.index(gen).or_else(|| self.proof(gen, no))
    }

    /// The shards.
    pub fn shards(&self) -> impl Iterator<Item = &Blob> {
        self.blobs.iter().filter(|b| b.kind == BlobKind::Shard)
    }

    /// The size of every shard, bytes.
    pub fn shard_sizes(&self) -> Vec<usize> {
        self.shards().map(|s| s.bytes.len()).collect()
    }

    /// The bytes of all blobs together: the keeper's upload.
    pub fn upload_bytes(&self) -> usize {
        self.blobs.iter().map(|b| b.bytes.len()).sum()
    }

    /// The hashes of all blobs.
    pub fn blob_hashes(&self) -> Vec<[u8; 32]> {
        self.blobs.iter().map(|b| b.sha256).collect()
    }
}

/// Builds an epoch from its recipe. `node_keys` gives the node keys of a
/// generation of the recipe (an `sd-v1` recipe takes its labels from the
/// same master, `NodeKeys::sd_labels`); `secret` makes the secret of one
/// member (`flat-v1`).
pub fn build(
    scope: &ScopeRef,
    recipe: &EpochRecipe,
    node_keys: &dyn Fn(u32) -> Option<NodeKeys>,
    secret: &dyn Fn(&MemberId) -> SecretBytes,
) -> Result<EpochOut, Error> {
    let k = &recipe.key;
    let nonces = Nonces::new(&recipe.seed);
    let mut trees = Vec::with_capacity(recipe.trees.len());
    let mut blobs = Vec::new();
    let mut witnesses = Vec::new();
    let mut cover = Vec::with_capacity(recipe.trees.len());
    for tr in &recipe.trees {
        let nodes = node_keys(tr.gen).ok_or(Error::Invalid("recipe of a dropped generation"))?;
        let laid = match recipe.scheme {
            SchemeId::CsV1 => {
                if tr.cover.len() % COVER_ENTRY != 0 {
                    return Err(Error::Invalid("cover of the recipe"));
                }
                let blocks: Vec<[u8; BLOCK_LEN]> = tr
                    .cover
                    .as_chunks::<COVER_ENTRY>()
                    .0
                    .iter()
                    .map(|e| {
                        let node = u64::from_le_bytes(*e);
                        seal_block(&nodes.node(node), &nonces.node(node), scope, tr.gen, k, node)
                    })
                    .collect();
                shard::layout(scope, tr, k.epoch, &blocks)?
            }
            SchemeId::SdV1 => {
                if tr.cover.len() % SD_COVER_ENTRY != 0 {
                    return Err(Error::Invalid("cover of the recipe"));
                }
                let labels = nodes.sd_labels();
                let mut blocks: Vec<[u8; SD_BLOCK_LEN]> = Vec::with_capacity(tr.subsets());
                for e in tr.cover.as_chunks::<SD_COVER_ENTRY>().0 {
                    let i = u64::from_le_bytes(e[..8].try_into().expect("8 bytes"));
                    let j = u64::from_le_bytes(e[8..].try_into().expect("8 bytes"));
                    if !crate::sd::is_subset(tr.tree, i, j) {
                        return Err(Error::Invalid("subset of the recipe"));
                    }
                    blocks.push(seal_sd_block(&labels.subset_key(i, j), &nonces.subset(i, j), scope, tr.gen, k, i, j));
                }
                shard::layout_sd(scope, tr, k.epoch, &blocks)?
            }
            SchemeId::FlatV1 => return Err(Error::Invalid("a flat recipe has no tree")),
        };
        let (index, mut w) = shard::tree_index(scope, tr, k.epoch, &laid);
        trees.push(index);
        blobs.extend(laid);
        witnesses.append(&mut w);
        cover.push((tr.gen, tr.blocks(recipe.scheme)));
    }
    blobs.append(&mut witnesses);
    let event = EpochEvent {
        v: EVENT_VERSION,
        scheme: recipe.scheme,
        community_id: scope.community,
        scope_id: scope.scope,
        epoch: k.epoch,
        prev_epoch: recipe.prev_epoch,
        at: recipe.at,
        history: recipe.history,
        trees,
        chain: recipe.chain.clone(),
    };
    let secrets = recipe.recipients.iter().map(|m| (*m, secret(m))).collect();
    Ok(EpochOut {
        event: Verified::after_signature_check(event),
        blobs,
        secrets,
        key: k.clone(),
        cover,
        retires: recipe.retires.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_and_chain_open_only_with_their_context() {
        let scope = ScopeRef { community: [1; 32], scope: [2; 32] };
        let nk = Key32([5; 32]);
        let k = EpochKey { epoch: 9, key: Key32([6; 32]) };
        let b = seal_block(&nk, &[7; 12], &scope, 3, &k, 42);
        assert_eq!(b.len(), BLOCK_LEN);
        assert_eq!(block_node(&b), 42);
        assert_eq!(open_block(&nk, &b, &scope, 3, 9).unwrap(), k);
        assert!(open_block(&nk, &b, &scope, 4, 9).is_err());
        assert!(open_block(&nk, &b, &scope, 3, 10).is_err());
        assert!(open_block(&Key32([0; 32]), &b, &scope, 3, 9).is_err());
        let mut moved = b;
        moved[..8].copy_from_slice(&43u64.to_le_bytes());
        assert!(open_block(&nk, &moved, &scope, 3, 9).is_err());

        let prev = EpochKey { epoch: 8, key: Key32([4; 32]) };
        let chain = seal_chain(&k, &prev, &[1; 12], &scope);
        assert_eq!(chain.len(), CHAIN_LEN);
        let ev = EpochEvent {
            v: EVENT_VERSION,
            scheme: SchemeId::CsV1,
            community_id: scope.community,
            scope_id: scope.scope,
            epoch: 9,
            prev_epoch: Some(8),
            at: 0,
            history: History::Open,
            trees: vec![],
            chain: Some(chain.clone()),
        };
        assert_eq!(open_chain(&ev, &k).unwrap(), prev);
        let mut wrong = ev.clone();
        wrong.prev_epoch = Some(7);
        assert!(open_chain(&wrong, &k).is_err());
        // The chain is not sealed under the epoch key itself.
        let raw = cipher(&k.key).decrypt(
            &Nonce::<Aes256Gcm>::from([1u8; 12]),
            Payload { msg: &chain[12..], aad: &chain_aad(&scope, 9, 8) },
        );
        assert!(raw.is_err());
        let json = serde_json::to_string(&ev).unwrap();
        assert_eq!(serde_json::from_str::<EpochEvent>(&json).unwrap(), ev);
    }

    #[test]
    fn nonces_come_from_the_seed_and_the_id() {
        let a = Nonces::new(&Key32([1; 32]));
        let b = Nonces::new(&Key32([2; 32]));
        assert_eq!(a.node(5), a.node(5));
        assert_ne!(a.node(5), a.node(6));
        assert_ne!(a.node(5), b.node(5));
        assert_ne!(a.subset(5, 10), a.subset(5, 11));
        assert_ne!(a.subset(5, 10), b.subset(5, 10));
        let mut d = Sha256::new();
        d.update(NONCE_INFO);
        d.update([1u8; 32]);
        d.update(5u64.to_le_bytes());
        assert_eq!(a.node(5)[..], d.finalize()[..12]);
    }

    #[test]
    fn sd_block_opens_only_with_its_context() {
        let scope = ScopeRef { community: [1; 32], scope: [2; 32] };
        let sk = Key32([5; 32]);
        let k = EpochKey { epoch: 9, key: Key32([6; 32]) };
        let b = seal_sd_block(&sk, &[7; 12], &scope, 3, &k, 5, 22);
        assert_eq!(b.len(), SD_BLOCK_LEN);
        assert_eq!(sd_block_subset(&b), (5, 22));
        assert_eq!(open_sd_block(&sk, &b, &scope, 3, 9).unwrap(), k);
        assert!(open_sd_block(&sk, &b, &scope, 4, 9).is_err());
        assert!(open_sd_block(&sk, &b, &scope, 3, 10).is_err());
        assert!(open_sd_block(&Key32([0; 32]), &b, &scope, 3, 9).is_err());
        assert!(open_sd_block(&sk, &b[..SD_BLOCK_LEN - 1], &scope, 3, 9).is_err());
        // Another i or another j: refused by the aad.
        for at in [0, 8] {
            let mut moved = b;
            moved[at] ^= 1;
            assert!(open_sd_block(&sk, &moved, &scope, 3, 9).is_err());
        }
    }
}
