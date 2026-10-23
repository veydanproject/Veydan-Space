// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The Complete Subtree cover: the fewest subtrees whose leaves are
//! exactly the active ones.
//!
//! Leaves are handed out in order, so the assigned ones are `[0,
//! next_leaf)`; the active ones are those minus the revoked. The tail
//! `[next_leaf, capacity)` counts as revoked, one range, so it costs
//! nothing per leaf. The cover is the maximal aligned decomposition of
//! each gap between two consecutive revoked leaves: the parent of every
//! node it gives spans a revoked leaf (it reaches past the gap, so over
//! one of its ends), which is the CS cover of Naor–Naor–Lotspiech. Time
//! and size are O((r + 1) · depth) for r revoked leaves.

use std::collections::BTreeSet;

use crate::tree::Tree;

/// The cover of the active leaves of a tree, left to right.
pub fn cs_cover(tree: Tree, next_leaf: u64, revoked: &BTreeSet<u32>) -> Vec<u64> {
    let next_leaf = next_leaf.min(tree.capacity());
    let mut out = Vec::new();
    let mut from = 0u64;
    for &r in revoked {
        let r = u64::from(r);
        if r >= next_leaf {
            break;
        }
        tree.decompose(from, r, &mut out);
        from = r + 1;
    }
    tree.decompose(from, next_leaf, &mut out);
    out
}

/// The Subset Difference cover of the active leaves: subsets `(i, j)`
/// (`sd::ALL` for `j`: every leaf) that are pairwise disjoint and whose
/// union is exactly the assigned leaves `[0, next_leaf)` minus the
/// revoked.
///
/// The tail `[next_leaf, capacity)` is not a set of revoked leaves: the
/// assigned range is cut into its maximal aligned subtrees
/// (`Tree::decompose`, at most `2 · depth`), and the cover algorithm of
/// Naor–Naor–Lotspiech runs inside each on the revoked leaves it holds,
/// giving at most `2 r_t − 1` subsets for `r_t ≥ 1` revoked leaves. A
/// subtree `t` without a revoked leaf is one subset: `S_{parent(t),
/// sibling(t)}`, the very leaves of `t` (the decomposition is maximal, so
/// `parent(t)` reaches past the assigned range), or `(1, ALL)` when `t`
/// is the whole tree. Time O(r · depth).
///
/// Inside a subtree the algorithm is the one of the paper, by recursion
/// over the Steiner tree of the revoked leaves: a node where both
/// children hold revoked leaves is a branch point `v`; each child `c` of
/// `v` whose chain ends at a node `e ≠ c` (the next branch point down, or
/// the revoked leaf) gives `S_{c,e}`; at the top, `S_{t,e}` when the chain
/// from `t` ends at `e ≠ t`. A node is the `i` of at most one subset of
/// the cover.
pub fn sd_cover(tree: Tree, next_leaf: u64, revoked: &BTreeSet<u32>) -> Vec<(u64, u64)> {
    let next_leaf = next_leaf.min(tree.capacity());
    let mut tops = Vec::new();
    tree.decompose(0, next_leaf, &mut tops);
    let mut out = Vec::new();
    let mut leaves: Vec<u64> = Vec::new();
    for t in tops {
        let (s, e) = tree.leaf_range(t);
        leaves.clear();
        leaves.extend(revoked.range(s as u32..=(e - 1) as u32).map(|&l| tree.leaf_node(l)));
        if leaves.is_empty() {
            out.push(if t == 1 { (1, crate::sd::ALL) } else { (t >> 1, t ^ 1) });
            continue;
        }
        let end = chain_end(&leaves, &mut out);
        if end != t {
            out.push((t, end));
        }
    }
    out
}

/// The end of the chain that leads down to `nodes` (revoked leaf nodes,
/// sorted, at least one): the branch point of the Steiner tree they span,
/// or the leaf itself; emits the subsets below it.
fn chain_end(nodes: &[u64], out: &mut Vec<(u64, u64)>) -> u64 {
    let (first, last) = (nodes[0], nodes[nodes.len() - 1]);
    if first == last {
        return first;
    }
    // The lowest common ancestor of the two ends is the branch point.
    let shift = 64 - (first ^ last).leading_zeros();
    let v = first >> shift;
    let (a, b) = (2 * v, 2 * v + 1);
    let split = nodes.partition_point(|&n| n >> (shift - 1) == a);
    let ea = chain_end(&nodes[..split], out);
    if ea != a {
        out.push((a, ea));
    }
    let eb = chain_end(&nodes[split..], out);
    if eb != b {
        out.push((b, eb));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::XorShift64;

    /// The cover by definition: nodes with no revoked leaf whose parent
    /// has one, the tail counting as revoked.
    fn brute(tree: Tree, next_leaf: u64, revoked: &BTreeSet<u32>) -> Vec<u64> {
        let out_leaf = |l: u64| l >= next_leaf || revoked.contains(&(l as u32));
        let has_out = |n: u64| {
            let (s, e) = tree.leaf_range(n);
            (s..e).any(out_leaf)
        };
        let mut nodes: Vec<u64> = (1..2 * tree.capacity())
            .filter(|&n| !has_out(n) && (n == 1 || has_out(n / 2)))
            .collect();
        nodes.sort_by_key(|&n| tree.leaf_range(n).0);
        nodes
    }

    #[test]
    fn matches_the_definition() {
        let mut rng = XorShift64::new(11);
        for depth in 1..=7u8 {
            let tree = Tree::new(depth).unwrap();
            for _ in 0..60 {
                let next = rng.below(tree.capacity() + 1);
                let mut revoked = BTreeSet::new();
                for _ in 0..rng.below(6) {
                    revoked.insert(rng.below(tree.capacity()) as u32);
                }
                assert_eq!(cs_cover(tree, next, &revoked), brute(tree, next, &revoked));
            }
        }
    }

    fn under(n: u64, v: u64) -> bool {
        let (ln, lv) = (Tree::level(n), Tree::level(v));
        ln >= lv && n >> (ln - lv) == v
    }

    fn lca(a: u64, b: u64) -> u64 {
        let (la, lb) = (Tree::level(a), Tree::level(b));
        let (mut a, mut b) = (a >> la.saturating_sub(lb), b >> lb.saturating_sub(la));
        while a != b {
            a >>= 1;
            b >>= 1;
        }
        a
    }

    /// The cover of the paper inside subtree `t`, by the book: an explicit
    /// Steiner tree, two of its leaves whose common ancestor holds no other
    /// leaf of it taken at a time.
    fn textbook(t: u64, revoked: &[u64]) -> Vec<(u64, u64)> {
        let mut st = BTreeSet::new();
        for &n in revoked {
            let mut x = n;
            while x != t {
                st.insert(x);
                x >>= 1;
            }
            st.insert(t);
        }
        let mut out = Vec::new();
        loop {
            let leaves: Vec<u64> =
                st.iter().copied().filter(|&n| !st.contains(&(2 * n)) && !st.contains(&(2 * n + 1))).collect();
            if leaves.len() == 1 {
                if leaves[0] != t {
                    out.push((t, leaves[0]));
                }
                return out;
            }
            let mut pick = None;
            'find: for (x, &vi) in leaves.iter().enumerate() {
                for &vj in &leaves[x + 1..] {
                    let v = lca(vi, vj);
                    if leaves.iter().filter(|&&l| under(l, v)).count() == 2 {
                        pick = Some((vi, vj, v));
                        break 'find;
                    }
                }
            }
            let (vi, vj, v) = pick.expect("two leaves meet somewhere");
            for leaf in [vi, vj] {
                let child = leaf >> (Tree::level(leaf) - Tree::level(v) - 1);
                if child != leaf {
                    out.push((child, leaf));
                }
            }
            st.retain(|&n| n == v || !under(n, v));
        }
    }

    #[test]
    fn sd_cover_is_an_exact_partition_and_the_algorithm_of_the_paper() {
        use crate::sd::{is_subset, subset_contains, ALL};
        let mut rng = XorShift64::new(12);
        for depth in 1..=7u8 {
            let tree = Tree::new(depth).unwrap();
            for round in 0..80 {
                let next = if round % 4 == 0 { tree.capacity() } else { rng.below(tree.capacity() + 1) };
                let mut revoked = BTreeSet::new();
                for _ in 0..rng.below(1 + tree.capacity().min(12)) {
                    revoked.insert(rng.below(tree.capacity()) as u32);
                }
                let cover = sd_cover(tree, next, &revoked);
                // Every subset is one, they are disjoint, and they hold
                // exactly the active leaves.
                let active = |l: u64| l < next && !revoked.contains(&(l as u32));
                for l in 0..tree.capacity() {
                    let n = cover.iter().filter(|&&(i, j)| subset_contains(tree, i, j, l)).count();
                    assert_eq!(n, usize::from(active(l)), "depth {depth} leaf {l} next {next} {revoked:?}: {cover:?}");
                }
                let mut is: Vec<u64> = cover.iter().map(|&(i, j)| {
                    assert!(is_subset(tree, i, j), "({i}, {j})");
                    i
                }).collect();
                is.sort_unstable();
                is.dedup();
                assert_eq!(is.len(), cover.len(), "a node is the i of two subsets");
                // Per maximal subtree: the subsets of the paper, at most
                // 2r − 1 of them, or the one subset of a whole subtree.
                let mut tops = Vec::new();
                tree.decompose(0, next, &mut tops);
                let mut expected = Vec::new();
                for t in tops {
                    let (s, e) = tree.leaf_range(t);
                    let r: Vec<u64> =
                        (s..e).filter(|&l| revoked.contains(&(l as u32))).map(|l| tree.leaf_node(l as u32)).collect();
                    let mine: Vec<(u64, u64)> =
                        cover.iter().copied().filter(|&(i, j)| j != ALL && under(i, t)).collect();
                    if r.is_empty() {
                        let whole = if t == 1 { (1, ALL) } else { (t / 2, t ^ 1) };
                        assert!(cover.contains(&whole));
                        expected.push(whole);
                        continue;
                    }
                    assert!(mine.len() < 2 * r.len(), "{} subsets for {} revoked", mine.len(), r.len());
                    let mut book = textbook(t, &r);
                    let mut mine = mine;
                    book.sort_unstable();
                    mine.sort_unstable();
                    assert_eq!(mine, book, "depth {depth} subtree {t} revoked {r:?}");
                    expected.extend(book);
                }
                assert_eq!(expected.len(), cover.len());
            }
        }
    }

    #[test]
    fn sd_full_and_empty() {
        let t = Tree::new(4).unwrap();
        assert_eq!(sd_cover(t, 16, &BTreeSet::new()), vec![(1, crate::sd::ALL)]);
        assert!(sd_cover(t, 0, &BTreeSet::new()).is_empty());
        // One revoked leaf: one subset, the whole tree minus it.
        assert_eq!(sd_cover(t, 16, &[5].into()), vec![(1, 21)]);
        // The tail: [0, 12) is subtrees 2 and 6; 6 is S_{3,7}.
        assert_eq!(sd_cover(t, 12, &BTreeSet::new()), vec![(1, 3), (3, 7)]);
        assert_eq!(sd_cover(t, 12, &[0, 15].into()), vec![(2, 16), (3, 7)]);
    }

    #[test]
    fn full_and_empty() {
        let t = Tree::new(4).unwrap();
        assert_eq!(cs_cover(t, 16, &BTreeSet::new()), vec![1]);
        assert!(cs_cover(t, 0, &BTreeSet::new()).is_empty());
        assert_eq!(cs_cover(t, 16, &[0].into()), vec![17, 9, 5, 3]);
    }
}
