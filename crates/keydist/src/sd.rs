// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Subset Difference (`sd-v1`, Naor–Naor–Lotspiech 2001): the labels, the
//! generator that walks them down the tree, and the subsets they key.
//!
//! A subset `S_{i,j}` is the leaves under node `i` minus those under its
//! strict descendant `j`. The keeper derives one label per node,
//! `LABEL_i = HKDF-SHA256(K_generation, info = "veydan.keydist.sd.v1" ‖
//! community ‖ scope ‖ gen (u32 LE) ‖ i (u64 LE))`, on demand, never
//! stored. From `LABEL_{i,i} = LABEL_i` a generator `G` walks down:
//! `LABEL_{i,2x} = G_L(LABEL_{i,x})`, `LABEL_{i,2x+1} = G_R(LABEL_{i,x})`,
//! and the key of a subset is `K_{i,j} = G_M(LABEL_{i,j})`.
//!
//! `G` is SHA-256 with a domain tag: `G_X(label) = SHA-256("veydan.keydist.sd.g"
//! ‖ X ‖ label)` with `X` the byte `'L'`, `'M'` or `'R'`. The input is 52
//! bytes, one block of the compression function, a quarter of what an
//! HMAC of HKDF-Expand costs per step; a keeper walks a few hundred
//! thousand subsets an epoch, a member at most `depth` steps.
//!
//! The subset of every leaf (`S_{root,∅}`: nobody is revoked and the tree
//! is full) has its own key, `K_all = G_M(LABEL_root)`. `LABEL_{root,root}`
//! is never the label of any other subset (those are reached by at least
//! one `G_L` or `G_R` step), so `K_all` collides with no subset key; no
//! member can derive it from its labels, so the bundle carries it. On the
//! wire this subset is `(i, j) = (1, 0)` ([`ALL`]).
//!
//! A full subtree `t` other than the root needs no key of its own: its
//! leaves are `S_{parent(t), sibling(t)}`, a subset every leaf under `t`
//! already holds the label of.
//!
//! A member at leaf `u` holds, for every ancestor `i` of `u` from the root
//! to the leaf's parent, the label `LABEL_{i,x}` of every node `x` that
//! hangs off the path from `i` down to `u` (the sibling of each path node
//! below `i`): `depth·(depth + 1)/2` labels, plus `K_all`. Nothing on the
//! path itself: `LABEL_{i,v}` for `v` an ancestor of `u` would give the
//! key of a subset that excludes `u`.

use hkdf::Hkdf;
use sha2::{Digest, Sha256};

use crate::tree::Tree;
use crate::{Key32, ScopeRef};

/// The info prefix of the labels.
pub const SD_INFO: &[u8] = b"veydan.keydist.sd.v1";
/// The domain tag of the generator.
pub const PRG_TAG: &[u8] = b"veydan.keydist.sd.g";
/// The `j` of the subset of every leaf, with `i` the root.
pub const ALL: u64 = 0;

/// One output of the generator: `which` is `b'L'`, `b'M'` or `b'R'`.
pub fn prg(label: &Key32, which: u8) -> Key32 {
    let mut d = Sha256::new();
    d.update(PRG_TAG);
    d.update([which]);
    d.update(label.0);
    Key32(d.finalize().into())
}

/// The label of the left child.
pub fn g_left(label: &Key32) -> Key32 {
    prg(label, b'L')
}

/// The key of the subset a label names.
pub fn g_middle(label: &Key32) -> Key32 {
    prg(label, b'M')
}

/// The label of the right child.
pub fn g_right(label: &Key32) -> Key32 {
    prg(label, b'R')
}

/// Walks a label from node `from` down to `to`, a descendant of `from`
/// or `from` itself: `LABEL_{i,from}` becomes `LABEL_{i,to}`.
pub fn descend(label: &Key32, from: u64, to: u64) -> Key32 {
    let (lf, lt) = (Tree::level(from), Tree::level(to));
    debug_assert!(lt >= lf && to >> (lt - lf) == from, "{to} is not under {from}");
    let mut cur = label.clone();
    for m in lf + 1..=lt {
        cur = if (to >> (lt - m)) & 1 == 0 { g_left(&cur) } else { g_right(&cur) };
    }
    cur
}

/// Whether `(i, j)` names a subset of the tree: `j` a strict descendant
/// of `i`, or the subset of every leaf.
pub fn is_subset(tree: Tree, i: u64, j: u64) -> bool {
    if (i, j) == (1, ALL) {
        return true;
    }
    if !tree.contains(i) || !tree.contains(j) {
        return false;
    }
    let (li, lj) = (Tree::level(i), Tree::level(j));
    lj > li && j >> (lj - li) == i
}

/// The leaves of a subset as two ranges `[start, end)` (either may be
/// empty): those of `i` left of `j`, and right of it.
pub fn subset_ranges(tree: Tree, i: u64, j: u64) -> [(u64, u64); 2] {
    let (s, e) = tree.leaf_range(i);
    if j == ALL {
        return [(s, e), (e, e)];
    }
    let (js, je) = tree.leaf_range(j);
    [(s, js), (je, e)]
}

/// Whether a leaf lies in a subset.
pub fn subset_contains(tree: Tree, i: u64, j: u64, leaf: u64) -> bool {
    subset_ranges(tree, i, j).iter().any(|&(s, e)| s <= leaf && leaf < e)
}

/// The number of keys of a bundle at a depth: `depth·(depth + 1)/2`
/// labels and `K_all`.
pub fn bundle_key_count(depth: u8) -> usize {
    let d = usize::from(depth);
    d * (d + 1) / 2 + 1
}

/// Where a bundle keeps `LABEL_{i,x}`, with `i` at level `li` and `x` at
/// level `lx` (`li < lx ≤ depth`): after `K_all`, root first, and for
/// each `i` from the top down.
pub fn label_index(depth: u8, li: u32, lx: u32) -> usize {
    let (d, l, m) = (usize::from(depth), li as usize, lx as usize);
    debug_assert!(l < m && m <= d);
    1 + l * d - l * l.saturating_sub(1) / 2 + (m - l - 1)
}

/// The labels of one generation of one scope. The extract step runs
/// once; each label is one expand, and the walk down is the generator.
#[derive(Clone)]
pub struct SdLabels {
    hk: Hkdf<Sha256>,
    prefix: Vec<u8>,
}

impl SdLabels {
    /// The labels under `master` for `scope` and generation `gen`.
    pub fn new(master: &Key32, scope: &ScopeRef, gen: u32) -> Self {
        let mut tail = Vec::with_capacity(68);
        tail.extend_from_slice(&scope.community);
        tail.extend_from_slice(&scope.scope);
        tail.extend_from_slice(&gen.to_le_bytes());
        Self::from_hkdf(Hkdf::<Sha256>::new(None, &master.0), &tail)
    }

    /// The labels from an extracted master (the one the node keys of
    /// `cs-v1` use; the info prefixes keep the two apart) and the
    /// `community ‖ scope ‖ gen` part of the info.
    pub(crate) fn from_hkdf(hk: Hkdf<Sha256>, tail: &[u8]) -> Self {
        let mut prefix = Vec::with_capacity(SD_INFO.len() + tail.len());
        prefix.extend_from_slice(SD_INFO);
        prefix.extend_from_slice(tail);
        SdLabels { hk, prefix }
    }

    /// `LABEL_i`, which is `LABEL_{i,i}`.
    pub fn label(&self, i: u64) -> Key32 {
        let mut out = [0u8; 32];
        self.hk
            .expand_multi_info(&[&self.prefix, &i.to_le_bytes()], &mut out)
            .expect("32 bytes is a valid length of HKDF-SHA256");
        Key32(out)
    }

    /// `LABEL_{i,j}` for `j` under `i` (or `i` itself).
    pub fn label_of(&self, i: u64, j: u64) -> Key32 {
        descend(&self.label(i), i, j)
    }

    /// The key of the subset of every leaf.
    pub fn all_key(&self) -> Key32 {
        g_middle(&self.label(1))
    }

    /// The key `K_{i,j}` of a subset (`(1, ALL)`: `K_all`).
    pub fn subset_key(&self, i: u64, j: u64) -> Key32 {
        if j == ALL {
            debug_assert_eq!(i, 1);
            return self.all_key();
        }
        g_middle(&self.label_of(i, j))
    }

    /// The keys of the bundle of a leaf, in the order of
    /// [`label_index`]: `K_all`, then for every ancestor `i` from the
    /// root down, the labels of the nodes hanging off the path below it.
    pub fn member_keys(&self, tree: Tree, leaf: u32) -> Vec<Key32> {
        let d = u32::from(tree.depth());
        let u = tree.leaf_node(leaf);
        let mut out = Vec::with_capacity(bundle_key_count(tree.depth()));
        out.push(self.all_key());
        for li in 0..d {
            let i = u >> (d - li);
            let mut cur = self.label(i);
            for m in li + 1..=d {
                let on_path = u >> (d - m);
                if on_path & 1 == 0 {
                    out.push(g_right(&cur));
                    if m < d {
                        cur = g_left(&cur);
                    }
                } else {
                    out.push(g_left(&cur));
                    if m < d {
                        cur = g_right(&cur);
                    }
                }
            }
        }
        debug_assert_eq!(out.len(), bundle_key_count(tree.depth()));
        out
    }
}

/// Where a member at `leaf` finds the key of subset `(i, j)` in its
/// bundle: the index of its label `LABEL_{i,x}` and the node `x` to walk
/// down from, or `None` when the leaf is not in the subset. `(1, ALL)`
/// gives index 0 and no walk (`x = 0`).
pub fn member_label_for(tree: Tree, leaf: u32, i: u64, j: u64) -> Option<(usize, u64)> {
    if !is_subset(tree, i, j) {
        return None;
    }
    if j == ALL {
        return Some((0, 0));
    }
    let d = u32::from(tree.depth());
    let u = tree.leaf_node(leaf);
    let (li, lj) = (Tree::level(i), Tree::level(j));
    if u >> (d - li) != i {
        return None;
    }
    let diff = j ^ (u >> (d - lj));
    if diff == 0 {
        return None;
    }
    // The first level below i where j's path leaves u's.
    let m = lj - (63 - diff.leading_zeros());
    debug_assert!(m > li);
    let x = j >> (lj - m);
    Some((label_index(tree.depth(), li, m), x))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope() -> ScopeRef {
        ScopeRef { community: [1; 32], scope: [2; 32] }
    }

    #[test]
    fn generator_outputs_differ_and_walks_compose() {
        let l = Key32([7; 32]);
        let (a, b, c) = (g_left(&l), g_middle(&l), g_right(&l));
        assert!(a != b && b != c && a != c && a != l);
        // 5 → 11 → 22 is right then left.
        assert_eq!(descend(&l, 5, 22), g_left(&g_right(&l)));
        assert_eq!(descend(&l, 5, 5), l);
        assert_eq!(descend(&descend(&l, 2, 9), 9, 37), descend(&l, 2, 37));
    }

    #[test]
    fn labels_separate_by_every_input_and_from_cs() {
        let m = Key32([9; 32]);
        let a = SdLabels::new(&m, &scope(), 1);
        assert_eq!(a.label(5), a.label(5));
        assert_ne!(a.label(5), a.label(6));
        assert_ne!(a.label(5), SdLabels::new(&m, &scope(), 2).label(5));
        assert_ne!(a.label(5), SdLabels::new(&Key32([8; 32]), &scope(), 1).label(5));
        let cs = crate::kdf::NodeKeys::new(&m, &scope(), 1);
        assert_ne!(a.label(5), cs.node(5));
        assert_eq!(a.subset_key(1, ALL), g_middle(&a.label(1)));
        assert_ne!(a.subset_key(1, 2), a.subset_key(1, 3));
    }

    #[test]
    fn a_member_derives_exactly_the_subsets_it_is_in() {
        let labels = SdLabels::new(&Key32([3; 32]), &scope(), 4);
        for depth in 1..=5u8 {
            let tree = Tree::new(depth).unwrap();
            for leaf in 0..tree.capacity() as u32 {
                let keys = labels.member_keys(tree, leaf);
                assert_eq!(keys.len(), bundle_key_count(depth));
                for i in 1..tree.capacity() {
                    for j in (2..2 * tree.capacity()).chain([ALL]) {
                        if !is_subset(tree, i, j) {
                            assert!(member_label_for(tree, leaf, i, j).is_none());
                            continue;
                        }
                        let inside = subset_contains(tree, i, j, u64::from(leaf));
                        match member_label_for(tree, leaf, i, j) {
                            Some((at, x)) => {
                                assert!(inside, "leaf {leaf} not in ({i}, {j}) but found a label");
                                let k = if j == ALL { keys[0].clone() } else { g_middle(&descend(&keys[at], x, j)) };
                                assert_eq!(k, labels.subset_key(i, j), "depth {depth} leaf {leaf} ({i}, {j})");
                            }
                            None => assert!(!inside, "leaf {leaf} in ({i}, {j}) without a label"),
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn bundle_counts() {
        assert_eq!(bundle_key_count(24), 301);
        assert_eq!(bundle_key_count(1), 2);
        assert_eq!(label_index(4, 0, 1), 1);
        assert_eq!(label_index(4, 1, 2), 5);
        assert_eq!(label_index(4, 3, 4), 10);
    }
}
