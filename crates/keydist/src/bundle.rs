// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The bundle of a member: the keys it holds for one generation, handed
//! out once per generation.
//!
//! Binary layout (version 1), integers little-endian:
//!
//! | bytes | field                                     |
//! |-------|-------------------------------------------|
//! | 1     | version (1)                               |
//! | 1     | scheme (`SchemeId::code`)                 |
//! | 32    | community                                 |
//! | 32    | scope                                     |
//! | 4     | generation                                |
//! | 1     | depth                                     |
//! | 4     | leaf                                      |
//! | 32 × count | keys                                 |
//!
//! The scheme byte decides what the keys are:
//!
//! - `cs-v1` (2): the `depth + 1` node keys of the leaf's path, root
//!   first. At depth 24: 75 + 800 = 875 bytes before the host seals it.
//! - `sd-v1` (3): `depth·(depth + 1)/2 + 1` keys (`sd::bundle_key_count`):
//!   `K_all`, then for every ancestor `i` of the leaf from the root to the
//!   leaf's parent, the labels `LABEL_{i,x}` of the nodes `x` hanging off
//!   the path below `i`, top down (`sd::label_index`). At depth 24: 75 +
//!   301 × 32 = 9707 bytes.

use crate::kdf::NodeKeys;
use crate::sd::{self, SdLabels};
use crate::tree::Tree;
use crate::{Error, Key32, ScopeRef, SchemeId, SecretBytes};

/// The version byte of the layout.
pub const BUNDLE_VERSION: u8 = 1;
/// The bytes before the keys.
pub const BUNDLE_HEADER_LEN: usize = 75;

/// The keys of one member in one generation.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Bundle {
    pub scheme: SchemeId,
    pub scope: ScopeRef,
    pub gen: u32,
    pub tree: Tree,
    pub leaf: u32,
    /// `cs-v1`: `depth + 1` node keys, the root's first, the leaf's last.
    /// `sd-v1`: `K_all` and the labels, in the order of the layout.
    pub keys: Vec<Key32>,
}

impl Bundle {
    /// The bundle of `leaf`, derived from the node keys of its generation.
    pub fn issue(nodes: &NodeKeys, scope: ScopeRef, gen: u32, tree: Tree, leaf: u32) -> Self {
        Bundle {
            scheme: SchemeId::CsV1,
            scope,
            gen,
            tree,
            leaf,
            keys: tree.path(leaf).map(|n| nodes.node(n)).collect(),
        }
    }

    /// The `sd-v1` bundle of `leaf`, derived from the labels of its
    /// generation.
    pub fn issue_sd(labels: &SdLabels, scope: ScopeRef, gen: u32, tree: Tree, leaf: u32) -> Self {
        Bundle { scheme: SchemeId::SdV1, scope, gen, tree, leaf, keys: labels.member_keys(tree, leaf) }
    }

    /// The number of keys a bundle of a scheme holds at a depth.
    pub fn key_count(scheme: SchemeId, depth: u8) -> Option<usize> {
        match scheme {
            SchemeId::CsV1 => Some(usize::from(depth) + 1),
            SchemeId::SdV1 => Some(sd::bundle_key_count(depth)),
            SchemeId::FlatV1 => None,
        }
    }

    /// The key of a node of the cover, if the node is on this bundle's
    /// path (`cs-v1`; `None` for a bundle of another scheme).
    pub fn key_for(&self, node: u64) -> Option<&Key32> {
        if self.scheme != SchemeId::CsV1 || !self.tree.contains(node) {
            return None;
        }
        let level = Tree::level(node);
        (self.tree.ancestor_at(self.leaf, level) == node).then(|| &self.keys[level as usize])
    }

    /// The nodes of the path, root first, each with its key (`cs-v1`;
    /// nothing for a bundle of another scheme).
    pub fn path(&self) -> impl Iterator<Item = (u64, &Key32)> {
        let n = if self.scheme == SchemeId::CsV1 { self.keys.len() } else { 0 };
        self.tree.path(self.leaf).zip(self.keys.iter()).take(n)
    }

    /// The key of subset `(i, j)` of an `sd-v1` cover (`(1, sd::ALL)`:
    /// every leaf), derived from the label this bundle holds for it; `None`
    /// when the leaf is not in the subset or the bundle is not `sd-v1`.
    pub fn subset_key(&self, i: u64, j: u64) -> Option<Key32> {
        if self.scheme != SchemeId::SdV1 {
            return None;
        }
        let (at, x) = sd::member_label_for(self.tree, self.leaf, i, j)?;
        if j == sd::ALL {
            return Some(self.keys[0].clone());
        }
        Some(sd::g_middle(&sd::descend(&self.keys[at], x, j)))
    }

    /// The `K_all` of an `sd-v1` bundle.
    pub fn all_key(&self) -> Option<&Key32> {
        (self.scheme == SchemeId::SdV1).then(|| &self.keys[0])
    }

    /// The labels of an `sd-v1` bundle as `(i, x, LABEL_{i,x})`, in the
    /// order of the layout (`K_all` left out); nothing for another scheme.
    pub fn sd_labels(&self) -> Vec<(u64, u64, &Key32)> {
        if self.scheme != SchemeId::SdV1 {
            return Vec::new();
        }
        let d = u32::from(self.tree.depth());
        let u = self.tree.leaf_node(self.leaf);
        let mut out = Vec::with_capacity(self.keys.len().saturating_sub(1));
        for li in 0..d {
            for m in li + 1..=d {
                let x = (u >> (d - m)) ^ 1;
                out.push((u >> (d - li), x, &self.keys[sd::label_index(self.tree.depth(), li, m)]));
            }
        }
        out
    }

    /// The bytes of the layout above. They hold secrets: the buffer is
    /// wiped when dropped.
    pub fn to_bytes(&self) -> SecretBytes {
        let mut out = SecretBytes::with_capacity(BUNDLE_HEADER_LEN + 32 * self.keys.len());
        out.push(BUNDLE_VERSION);
        out.push(self.scheme.code());
        out.extend(&self.scope.community);
        out.extend(&self.scope.scope);
        out.extend(&self.gen.to_le_bytes());
        out.push(self.tree.depth());
        out.extend(&self.leaf.to_le_bytes());
        for k in &self.keys {
            out.extend(&k.0);
        }
        out
    }

    /// Parses the layout above, refusing every length, depth, leaf or
    /// scheme that does not fit.
    pub fn from_bytes(b: &[u8]) -> Result<Self, Error> {
        if b.len() < BUNDLE_HEADER_LEN || b[0] != BUNDLE_VERSION {
            return Err(Error::Invalid("bundle"));
        }
        let scheme = match SchemeId::from_code(b[1]) {
            Some(s @ (SchemeId::CsV1 | SchemeId::SdV1)) => s,
            _ => return Err(Error::Invalid("bundle scheme")),
        };
        let tree = Tree::new(b[70]).map_err(|_| Error::Invalid("bundle depth"))?;
        let leaf = u32::from_le_bytes(b[71..75].try_into().expect("4 bytes"));
        let count = Self::key_count(scheme, tree.depth()).expect("a scheme with a bundle");
        if b.len() != BUNDLE_HEADER_LEN + 32 * count || u64::from(leaf) >= tree.capacity() {
            return Err(Error::Invalid("bundle"));
        }
        let keys = b[BUNDLE_HEADER_LEN..]
            .as_chunks::<32>()
            .0
            .iter()
            .map(|c| Key32(*c))
            .collect();
        Ok(Bundle {
            scheme,
            scope: ScopeRef {
                community: b[2..34].try_into().expect("32 bytes"),
                scope: b[34..66].try_into().expect("32 bytes"),
            },
            gen: u32::from_le_bytes(b[66..70].try_into().expect("4 bytes")),
            tree,
            leaf,
            keys,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_round_trips_and_refuses_garbage() {
        let scope = ScopeRef { community: [1; 32], scope: [2; 32] };
        let tree = Tree::new(24).unwrap();
        let nodes = NodeKeys::new(&Key32([3; 32]), &scope, 7);
        let b = Bundle::issue(&nodes, scope, 7, tree, 123_456);
        let bytes = b.to_bytes();
        assert_eq!(bytes.len(), 875);
        assert_eq!(Bundle::from_bytes(&bytes).unwrap(), b);
        let leaf_node = tree.leaf_node(123_456);
        assert_eq!(b.key_for(leaf_node), Some(&nodes.node(leaf_node)));
        assert_eq!(b.key_for(1), Some(&nodes.node(1)));
        assert_eq!(b.key_for(leaf_node ^ 1), None);
        assert!(Bundle::from_bytes(&bytes[..bytes.len() - 1]).is_err());
        let mut bad = bytes.to_vec();
        bad[1] = SchemeId::SdV1.code();
        assert!(Bundle::from_bytes(&bad).is_err());
        let mut bad = bytes.to_vec();
        bad[71..75].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Bundle::from_bytes(&bad).is_err());
    }

    #[test]
    fn sd_layout_round_trips_and_refuses_garbage() {
        let scope = ScopeRef { community: [1; 32], scope: [2; 32] };
        let tree = Tree::new(24).unwrap();
        let labels = SdLabels::new(&Key32([3; 32]), &scope, 7);
        let leaf = 123_456;
        let b = Bundle::issue_sd(&labels, scope, 7, tree, leaf);
        let bytes = b.to_bytes();
        assert_eq!(bytes.len(), BUNDLE_HEADER_LEN + 32 * 301);
        assert_eq!(bytes.len(), 9707);
        assert_eq!(bytes[1], SchemeId::SdV1.code());
        assert_eq!(Bundle::from_bytes(&bytes).unwrap(), b);
        assert_eq!(b.all_key(), Some(&labels.all_key()));
        assert_eq!(b.key_for(1), None);
        assert_eq!(b.path().count(), 0);
        // Each label is the one of its (i, x), and x hangs off the path.
        let u = tree.leaf_node(leaf);
        let held = b.sd_labels();
        assert_eq!(held.len(), 300);
        for (i, x, k) in held.iter().step_by(17) {
            assert_eq!(*k, &labels.label_of(*i, *x));
            assert_eq!(*x >> (Tree::level(*x) - Tree::level(*i)), *i);
            assert_eq!(u >> (24 - Tree::level(*x)), *x ^ 1);
        }
        // A subset with the leaf: derived; one without it: none.
        let (i, j) = (u >> 20, (u >> 3) ^ 1);
        assert_eq!(b.subset_key(i, j), Some(labels.subset_key(i, j)));
        assert_eq!(b.subset_key(i, u >> 3), None);
        assert_eq!(b.subset_key(1, sd::ALL), Some(labels.all_key()));
        assert_eq!(b.subset_key(u, u), None);
        // A CS bundle has no subset key, an SD one no node key.
        let cs = Bundle::issue(&NodeKeys::new(&Key32([3; 32]), &scope, 7), scope, 7, tree, leaf);
        assert_eq!(cs.subset_key(1, sd::ALL), None);
        assert!(cs.sd_labels().is_empty());
        // Garbage.
        assert!(Bundle::from_bytes(&bytes[..bytes.len() - 32]).is_err());
        let mut bad = bytes.to_vec();
        bad[1] = SchemeId::CsV1.code();
        assert!(Bundle::from_bytes(&bad).is_err());
        let mut bad = bytes.to_vec();
        bad[70] = 23;
        assert!(Bundle::from_bytes(&bad).is_err());
    }
}
