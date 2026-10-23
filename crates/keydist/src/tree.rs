// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The complete binary tree of a generation, by numbers only.
//!
//! Nodes are numbered heap-style: the root is 1, the children of `i` are
//! `2i` and `2i + 1`, and leaf `i` is node `2^depth + i`. A node at level
//! `l` (the root at level 0) spans `2^(depth − l)` consecutive leaves. No
//! tree is ever built: everything here is arithmetic on the numbers.

use crate::Error;

/// The smallest depth a tree may have.
pub const MIN_DEPTH: u8 = 1;
/// The largest depth a tree may have (a billion leaves; the specification
/// uses 14, 17, 20 and 24).
pub const MAX_DEPTH: u8 = 30;

/// The shape of a tree: its depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Tree {
    depth: u8,
}

impl Tree {
    /// A tree of `depth` levels below the root.
    pub fn new(depth: u8) -> Result<Self, Error> {
        if !(MIN_DEPTH..=MAX_DEPTH).contains(&depth) {
            return Err(Error::Params("depth out of range"));
        }
        Ok(Tree { depth })
    }

    /// The depth.
    pub fn depth(self) -> u8 {
        self.depth
    }

    /// The number of leaves.
    pub fn capacity(self) -> u64 {
        1u64 << self.depth
    }

    /// The node of a leaf. The leaf must be below the capacity.
    pub fn leaf_node(self, leaf: u32) -> u64 {
        debug_assert!(u64::from(leaf) < self.capacity());
        self.capacity() + u64::from(leaf)
    }

    /// The level of a node (the root is 0).
    pub fn level(node: u64) -> u32 {
        63 - node.leading_zeros()
    }

    /// Whether the node is a node of this tree.
    pub fn contains(self, node: u64) -> bool {
        node >= 1 && Self::level(node) <= u32::from(self.depth)
    }

    /// The leaves a node spans, `[start, end)`.
    pub fn leaf_range(self, node: u64) -> (u64, u64) {
        let level = Self::level(node);
        let height = u32::from(self.depth) - level;
        let start = (node - (1u64 << level)) << height;
        (start, start + (1u64 << height))
    }

    /// The ancestor of a leaf at a level (the leaf's own node at level
    /// `depth`).
    pub fn ancestor_at(self, leaf: u32, level: u32) -> u64 {
        self.leaf_node(leaf) >> (u32::from(self.depth) - level)
    }

    /// The path of a leaf, root first: `depth + 1` nodes.
    pub fn path(self, leaf: u32) -> impl Iterator<Item = u64> {
        let node = self.leaf_node(leaf);
        let depth = u32::from(self.depth);
        (0..=depth).map(move |l| node >> (depth - l))
    }

    /// Appends to `out` the maximal subtrees that together span exactly
    /// the leaves `[start, end)`, left to right: at most `2 · depth`
    /// nodes, each the largest aligned block that fits.
    pub fn decompose(self, mut start: u64, end: u64, out: &mut Vec<u64>) {
        let depth = u32::from(self.depth);
        let end = end.min(self.capacity());
        while start < end {
            let mut h = if start == 0 { depth } else { start.trailing_zeros().min(depth) };
            while (1u64 << h) > end - start {
                h -= 1;
            }
            out.push((1u64 << (depth - h)) + (start >> h));
            start += 1u64 << h;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbering() {
        let t = Tree::new(3).unwrap();
        assert_eq!(t.capacity(), 8);
        assert_eq!(t.leaf_node(0), 8);
        assert_eq!(t.leaf_node(7), 15);
        assert_eq!(t.leaf_range(1), (0, 8));
        assert_eq!(t.leaf_range(3), (4, 8));
        assert_eq!(t.leaf_range(5), (2, 4));
        assert_eq!(t.leaf_range(13), (5, 6));
        assert_eq!(t.path(5).collect::<Vec<_>>(), vec![1, 3, 6, 13]);
        assert_eq!(t.ancestor_at(5, 1), 3);
        assert!(t.contains(15) && !t.contains(16) && !t.contains(0));
        assert!(Tree::new(0).is_err() && Tree::new(31).is_err());
    }

    #[test]
    fn decomposition_is_exact_and_maximal() {
        let t = Tree::new(5).unwrap();
        for start in 0..=32u64 {
            for end in start..=32u64 {
                let mut out = Vec::new();
                t.decompose(start, end, &mut out);
                let mut at = start;
                for &n in &out {
                    let (s, e) = t.leaf_range(n);
                    assert_eq!(s, at);
                    at = e;
                    if n > 1 {
                        let (ps, pe) = t.leaf_range(n / 2);
                        assert!(ps < start || pe > end, "parent of {n} fits in [{start},{end})");
                    }
                }
                assert_eq!(at, end);
                assert!(out.len() <= 2 * 5);
            }
        }
    }
}
