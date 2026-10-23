// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Key derivation (specification 5 and 19.1), all HKDF-SHA256 without a
//! salt, integers little-endian:
//!
//! - the master of a generation from the owner's root secret, so that the
//!   root stays offline: info `"veydan.keydist.gen.v1" ‖ community ‖ scope
//!   ‖ scheme code (u8) ‖ gen (u32)`. One master opens one generation of
//!   one scope of one scheme, nothing else: an emergency of one scope
//!   leaves the others untouched;
//! - the key of a node from the master: info `"veydan.keydist.cs.v1" ‖
//!   community ‖ scope ‖ gen ‖ node (u64)`. A node key is never derived
//!   from its parent's: who holds a node key learns nothing else;
//! - the key of a channel from an epoch key: info
//!   `"veydan.keydist.channel.v1" ‖ community ‖ channel_id`;
//! - the key of the chain of an open history from an epoch key: info
//!   `"veydan.keydist.chain.v1" ‖ community ‖ scope`. An epoch key is only
//!   ever the input of HKDF (channels, the chain) and the plaintext of a
//!   block, never an AES key itself.
//!
//! The labels of `sd-v1` come from the same master under their own info
//! (`"veydan.keydist.sd.v1"`, module `sd`).

use std::fmt;

use hkdf::Hkdf;
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::sd::SdLabels;
use crate::{ChannelKey, EpochKey, Key32, SchemeId, ScopeRef};

const GEN_INFO: &[u8] = b"veydan.keydist.gen.v1";
const NODE_INFO: &[u8] = b"veydan.keydist.cs.v1";
const CHANNEL_INFO: &[u8] = b"veydan.keydist.channel.v1";
const CHAIN_INFO: &[u8] = b"veydan.keydist.chain.v1";
/// The domain of the fingerprint of a master.
const MASTER_FP: &[u8] = b"veydan.keydist.master-fp.v1";

fn expand(hk: &Hkdf<Sha256>, info: &[&[u8]]) -> Key32 {
    let mut out = [0u8; 32];
    hk.expand_multi_info(info, &mut out)
        .expect("32 bytes is a valid length of HKDF-SHA256");
    Key32(out)
}

/// The owner's root secret. It stays on the owner's device (offline when
/// it can); a keeper never holds it. Wiped when dropped; `Debug` hides it.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct RootSecret([u8; 32]);

impl RootSecret {
    /// A root secret from its bytes (restored from the owner's store).
    pub fn from_bytes(b: [u8; 32]) -> Self {
        RootSecret(b)
    }

    /// A new root secret from the system random source.
    pub fn generate() -> Self {
        let mut b = [0u8; 32];
        crate::rng::Rng::fill(&mut crate::rng::OsRng, &mut b);
        RootSecret(b)
    }

    /// The bytes, for the owner's store.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for RootSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RootSecret(..)")
    }
}

/// The master of one generation of one scope and scheme: what a keeper
/// holds. Derived by the owner's device (`derive`) and handed to the
/// keeper (`from_bytes` on the keeper's side). Wiped when dropped; `Debug`
/// hides it.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct GenerationMaster(Key32);

impl GenerationMaster {
    /// The master of generation `gen` of `scope` under `scheme`, from the
    /// owner's root secret.
    pub fn derive(root: &RootSecret, scope: &ScopeRef, scheme: SchemeId, gen: u32) -> Self {
        generation_key(root, scope, scheme, gen)
    }

    /// A master from its bytes (the keeper's side of the hand-over).
    pub fn from_bytes(b: [u8; 32]) -> Self {
        GenerationMaster(Key32(b))
    }

    /// The bytes, for the hand-over to a keeper.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0 .0
    }

    pub(crate) fn key(&self) -> &Key32 {
        &self.0
    }

    /// `SHA-256("veydan.keydist.master-fp.v1" ‖ master)`: what a keeper
    /// keeps of the masters it held, to refuse one a second time.
    pub fn fingerprint(&self) -> [u8; 32] {
        let mut d = Sha256::new();
        d.update(MASTER_FP);
        d.update(self.0 .0);
        d.finalize().into()
    }
}

impl fmt::Debug for GenerationMaster {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GenerationMaster(..)")
    }
}

/// The master of generation `gen` of `scope` under `scheme`, from the
/// owner's root secret (`GenerationMaster::derive`).
pub fn generation_key(root: &RootSecret, scope: &ScopeRef, scheme: SchemeId, gen: u32) -> GenerationMaster {
    let hk = Hkdf::<Sha256>::new(None, &root.0);
    GenerationMaster(expand(
        &hk,
        &[GEN_INFO, &scope.community, &scope.scope, &[scheme.code()], &gen.to_le_bytes()],
    ))
}

/// The node keys of one generation of one scope. The extract step runs
/// once; each node key is one expand.
#[derive(Clone)]
pub struct NodeKeys {
    hk: Hkdf<Sha256>,
    prefix: Vec<u8>,
}

impl NodeKeys {
    /// The node keys under `master` for `scope` and generation `gen`.
    pub fn new(master: &Key32, scope: &ScopeRef, gen: u32) -> Self {
        let mut prefix = Vec::with_capacity(NODE_INFO.len() + 68);
        prefix.extend_from_slice(NODE_INFO);
        prefix.extend_from_slice(&scope.community);
        prefix.extend_from_slice(&scope.scope);
        prefix.extend_from_slice(&gen.to_le_bytes());
        NodeKeys { hk: Hkdf::<Sha256>::new(None, &master.0), prefix }
    }

    /// The key of a node.
    pub fn node(&self, node: u64) -> Key32 {
        expand(&self.hk, &[&self.prefix, &node.to_le_bytes()])
    }

    /// The `sd-v1` labels of the same master, scope and generation
    /// (`sd::SdLabels`), without a second extract step.
    pub fn sd_labels(&self) -> SdLabels {
        SdLabels::from_hkdf(self.hk.clone(), &self.prefix[NODE_INFO.len()..])
    }
}

/// The key of a channel in an epoch. `channel_id` is any bytes the host
/// names the channel with; the scope's `scope` part does not enter (the
/// epoch key is already the scope's own).
pub fn channel_key(k: &EpochKey, scope: &ScopeRef, channel_id: &[u8]) -> ChannelKey {
    let hk = Hkdf::<Sha256>::new(None, &k.key.0);
    let key = expand(&hk, &[CHANNEL_INFO, &scope.community, channel_id]);
    ChannelKey(key.0)
}

/// The key that seals the previous epoch key in the chain of an open
/// history.
pub fn chain_key(k: &EpochKey, scope: &ScopeRef) -> Key32 {
    let hk = Hkdf::<Sha256>::new(None, &k.key.0);
    expand(&hk, &[CHAIN_INFO, &scope.community, &scope.scope])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope() -> ScopeRef {
        ScopeRef { community: [1; 32], scope: [2; 32] }
    }

    #[test]
    fn keys_separate_by_every_input() {
        let m = Key32([9; 32]);
        let a = NodeKeys::new(&m, &scope(), 1);
        assert_eq!(a.node(5), a.node(5));
        assert_ne!(a.node(5), a.node(6));
        assert_ne!(a.node(5), NodeKeys::new(&m, &scope(), 2).node(5));
        let other = ScopeRef { scope: [3; 32], ..scope() };
        assert_ne!(a.node(5), NodeKeys::new(&m, &other, 1).node(5));
        assert_ne!(a.node(5), NodeKeys::new(&Key32([8; 32]), &scope(), 1).node(5));
        let root = RootSecret::from_bytes([1; 32]);
        let g = |s: &ScopeRef, scheme, gen| GenerationMaster::derive(&root, s, scheme, gen);
        let base = g(&scope(), SchemeId::CsV1, 1);
        assert_ne!(base, g(&scope(), SchemeId::CsV1, 2));
        assert_ne!(base, g(&other, SchemeId::CsV1, 1), "a master is per scope");
        assert_ne!(base, g(&scope(), SchemeId::SdV1, 1), "a master is per scheme");
        assert_ne!(base.fingerprint(), g(&scope(), SchemeId::CsV1, 2).fingerprint());
        assert_ne!(&base.fingerprint(), base.as_bytes());
        let k = EpochKey { epoch: 1, key: Key32([4; 32]) };
        assert_ne!(channel_key(&k, &scope(), b"a"), channel_key(&k, &scope(), b"b"));
        assert_ne!(chain_key(&k, &scope()), k.key);
        assert_ne!(chain_key(&k, &scope()), chain_key(&k, &other));
        assert_ne!(chain_key(&k, &scope()).0, channel_key(&k, &scope(), b"").0);
    }
}
