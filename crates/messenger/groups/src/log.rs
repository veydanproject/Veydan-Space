// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The log of a group: the operations a device knows, their one agreed
//! order, and the state that order produces.
//!
//! Operations form a graph: each names the operations its author had
//! seen. Two admins acting at the same moment produce two operations with
//! the same parents; nobody waits for anybody. The order is then decided
//! by a rule every device applies to the same graph:
//!
//! 1. an operation comes after all of its parents;
//! 2. an operation that can change someone's rights (removes, bans, gives
//!    a role to them, hands them the group, or is their own leave or
//!    handover) comes before whatever was made beside it, neither knowing
//!    of the other, by that person or acting on them: an author chooses
//!    the parents, and naming an old part of the log must not put an
//!    operation ahead of the one that took its ground. Where such
//!    operations wait for each other in a circle, the one held back by the
//!    lowest role (its author's own operations not counted) goes first,
//!    then one taken by whom it concerns (see `Graph::confirmed`);
//! 3. among operations that are ready, the one whose author has the
//!    higher role (in the state reached so far) goes first, an owner's
//!    handover of the group above the owner's other operations;
//! 4. with equal roles, the one with the smaller id.
//!
//! An operation that lost its ground on the way (its author was removed
//! by one that went first) is rejected and changes nothing, but stays in
//! the graph so that later operations can still refer to it.
//!
//! The order is recomputed from the first operation whenever the graph
//! changes. Groups change rarely; correctness is worth more here than the
//! microseconds.

use crate::op::{GroupKind, KeyId, Op, OpBody, OpId};
use crate::roles::Role;
use crate::state::{GroupState, Rejection};
use messenger_core::PubKey;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// What happened to an operation handed to `insert`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Insert {
    /// In the log; the state reflects it.
    Applied,
    /// In the log, but it does not apply (and why).
    Rejected(Rejection),
    /// Already known.
    Duplicate,
    /// Some parents are unknown; kept aside until they arrive.
    Waiting { missing: Vec<OpId> },
    /// Not an operation of this group, or a second `Create`.
    Refused(Rejection),
}

/// Can the current key be trusted, and does everyone have it?
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyStatus {
    Good,
    /// Someone who is no longer a member knows the current key: a manager
    /// has to bring a new one.
    Rotate,
    /// These members joined without the knowledge of whoever made the
    /// current key: a manager has to hand it to them.
    Deliver(Vec<PubKey>),
}

#[derive(Clone, Debug)]
pub struct OpLog {
    group_id: String,
    ops: HashMap<OpId, Op>,
    waiting: HashMap<OpId, Op>,
    /// The agreed order, rejected operations included.
    order: Vec<OpId>,
    rejected: HashMap<OpId, Rejection>,
    state: GroupState,
    key_status: KeyStatus,
}

impl OpLog {
    /// Start a log from the `Create` operation.
    pub fn create(op: Op) -> Result<Self, Rejection> {
        let state = GroupState::genesis(&op)?;
        let id = op.id();
        let mut ops = HashMap::new();
        let group_id = op.group_id.clone();
        ops.insert(id.clone(), op);
        Ok(Self {
            group_id,
            ops,
            waiting: HashMap::new(),
            order: vec![id],
            rejected: HashMap::new(),
            state,
            key_status: KeyStatus::Good,
        })
    }

    /// Rebuild a log from stored operations, in any order.
    pub fn from_ops(ops: impl IntoIterator<Item = Op>) -> Result<Self, Rejection> {
        let mut all: Vec<Op> = ops.into_iter().collect();
        let at = all.iter().position(|o| matches!(o.body, OpBody::Create { .. })).ok_or(Rejection::NotCreated)?;
        let mut log = Self::create(all.swap_remove(at))?;
        // All of them first, the order once: not once per operation.
        let mut aside: HashMap<OpId, Op> = HashMap::new();
        for op in all {
            let id = op.id();
            let refused = op.group_id != log.group_id || matches!(op.body, OpBody::Create { .. }) || op.parents.is_empty();
            if !refused && !log.ops.contains_key(&id) {
                aside.insert(id, op);
            }
        }
        // Taken in once every parent is in, the way `insert` would.
        let mut children: HashMap<OpId, Vec<OpId>> = HashMap::new();
        let mut missing: HashMap<OpId, usize> = HashMap::new();
        let mut ready: Vec<OpId> = Vec::new();
        for (id, op) in &aside {
            let n = op.parents.iter().filter(|p| !log.ops.contains_key(*p)).count();
            for p in op.parents.iter().filter(|p| !log.ops.contains_key(*p)) {
                children.entry(p.clone()).or_default().push(id.clone());
            }
            missing.insert(id.clone(), n);
            if n == 0 {
                ready.push(id.clone());
            }
        }
        while let Some(id) = ready.pop() {
            let Some(op) = aside.remove(&id) else { continue };
            log.ops.insert(id.clone(), op);
            for child in children.remove(&id).unwrap_or_default() {
                if let Some(n) = missing.get_mut(&child) {
                    *n = n.saturating_sub(1);
                    if *n == 0 {
                        ready.push(child);
                    }
                }
            }
        }
        log.waiting = aside;
        log.rebuild();
        Ok(log)
    }

    pub fn group_id(&self) -> &str {
        &self.group_id
    }

    pub fn state(&self) -> &GroupState {
        &self.state
    }

    pub fn key_status(&self) -> &KeyStatus {
        &self.key_status
    }

    /// Operations in the agreed order.
    pub fn ordered(&self) -> impl Iterator<Item = &Op> {
        self.order.iter().filter_map(|id| self.ops.get(id))
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn contains(&self, id: &OpId) -> bool {
        self.ops.contains_key(id)
    }

    pub fn rejection(&self, id: &OpId) -> Option<&Rejection> {
        self.rejected.get(id)
    }

    /// Ids of parents that operations kept aside are waiting for.
    pub fn missing(&self) -> Vec<OpId> {
        let mut out: BTreeSet<OpId> = BTreeSet::new();
        for op in self.waiting.values() {
            for p in &op.parents {
                if !self.ops.contains_key(p) && !self.waiting.contains_key(p) {
                    out.insert(p.clone());
                }
            }
        }
        out.into_iter().collect()
    }

    /// Operations nothing refers to yet: what a new operation names as
    /// its parents. Sorted.
    pub fn heads(&self) -> Vec<OpId> {
        let referenced: HashSet<&OpId> = self.ops.values().flat_map(|o| o.parents.iter()).collect();
        let mut heads: Vec<OpId> = self.ops.keys().filter(|id| !referenced.contains(id)).cloned().collect();
        heads.sort();
        heads
    }

    /// A new operation by `author` on top of everything known here.
    pub fn next(&self, author: &PubKey, created_at: i64, body: OpBody) -> Op {
        Op::new(&self.group_id, author, self.heads(), created_at, body)
    }

    /// Would this operation apply right now? For commands: refuse before
    /// anything is published.
    pub fn check(&self, op: &Op) -> Result<(), Rejection> {
        self.state.clone().apply(op)
    }

    /// Add an operation that arrived (or was just made here).
    pub fn insert(&mut self, op: Op) -> Insert {
        if op.group_id != self.group_id {
            return Insert::Refused(Rejection::WrongGroup);
        }
        if matches!(op.body, OpBody::Create { .. }) {
            return if self.ops.contains_key(&op.id()) { Insert::Duplicate } else { Insert::Refused(Rejection::AlreadyCreated) };
        }
        if op.parents.is_empty() {
            return Insert::Refused(Rejection::Invalid { what: "no parents".into() });
        }
        let id = op.id();
        if self.ops.contains_key(&id) || self.waiting.contains_key(&id) {
            return Insert::Duplicate;
        }
        let missing: Vec<OpId> = op.parents.iter().filter(|p| !self.ops.contains_key(*p)).cloned().collect();
        if !missing.is_empty() {
            self.waiting.insert(id, op);
            return Insert::Waiting { missing };
        }
        self.ops.insert(id.clone(), op);
        self.adopt_waiting();
        self.rebuild();
        match self.rejected.get(&id) {
            Some(r) => Insert::Rejected(r.clone()),
            None => Insert::Applied,
        }
    }

    /// Operations kept aside whose parents are all here now.
    fn adopt_waiting(&mut self) {
        loop {
            let ready: Vec<OpId> = self
                .waiting
                .iter()
                .filter(|(_, op)| op.parents.iter().all(|p| self.ops.contains_key(p)))
                .map(|(id, _)| id.clone())
                .collect();
            if ready.is_empty() {
                return;
            }
            for id in ready {
                if let Some(op) = self.waiting.remove(&id) {
                    self.ops.insert(id, op);
                }
            }
        }
    }

    fn rebuild(&mut self) {
        let Some((order, rejected, mut state)) = agree(&self.ops) else { return };
        self.key_status = key_status(&self.ops, &order, &rejected, &mut state);
        self.order = order;
        self.rejected = rejected;
        self.state = state;
    }
}

/// The order and the state everyone arrives at.
///
/// An operation names the part of the log its author knew, and nothing
/// stops an author from naming less than they know. For a join by link
/// that matters: named early enough, it would come before the change
/// of the link it has no key for. So a join counts only if every later
/// change of the link knew of it; one that happened beside or after a
/// change of the link does not.
fn agree(ops: &HashMap<OpId, Op>) -> Option<(Vec<OpId>, HashMap<OpId, Rejection>, GroupState)> {
    let mut stale: HashSet<OpId> = HashSet::new();
    loop {
        let (order, rejected, state) = linearize(ops, &stale)?;
        let applied = || order.iter().filter(|id| !rejected.contains_key(*id)).filter_map(|id| ops.get(id).map(|o| (id, o)));
        let changes: Vec<(u32, HashSet<OpId>)> = applied()
            .filter_map(|(id, o)| match o.body {
                OpBody::RotateLink { link_epoch } => Some((link_epoch, knowledge(ops, id))),
                _ => None,
            })
            .collect();
        let found: Vec<OpId> = applied()
            .filter(|(_, o)| o.body == OpBody::Join)
            .filter(|(id, o)| {
                let epoch = o.proof.as_ref().map(|p| p.epoch).unwrap_or(0);
                changes.iter().any(|(e, known)| *e > epoch && !known.contains(*id))
            })
            .map(|(id, _)| id.clone())
            .collect();
        if found.is_empty() {
            return Some((order, rejected, state));
        }
        stale.extend(found);
    }
}

/// One pass: the order of a set of operations and the state it produces.
/// `stale` are joins already known not to count.
///
/// Operations are taken by index here, the indices in the order of their
/// ids: the smaller index is the smaller id.
fn linearize(ops: &HashMap<OpId, Op>, stale: &HashSet<OpId>) -> Option<(Vec<OpId>, HashMap<OpId, Rejection>, GroupState)> {
    let (root_id, root) = ops.iter().find(|(_, o)| matches!(o.body, OpBody::Create { .. }))?;
    let mut state = GroupState::genesis(root).ok()?;

    let mut ids: Vec<&OpId> = ops.keys().collect();
    ids.sort();
    let at: HashMap<&OpId, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let graph = Graph::new(ids.iter().map(|id| &ops[*id]).collect(), &at);
    let n = graph.node.len();
    let root = at[root_id];
    let stale: Vec<bool> = ids.iter().map(|id| stale.contains(*id)).collect();

    let mut blocked: Vec<usize> = graph.node.iter().map(|o| o.parents.len()).collect();
    // Of a ready operation: those it waits for. Of any: the ready ones
    // waiting for it. How many of those it waits for are still to come.
    let mut holders: Vec<Vec<usize>> = vec![Vec::new(); n];
    // Of those, the ones by somebody else than its author.
    let mut others: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut held: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut waits: Vec<usize> = vec![0; n];
    let mut done: Vec<bool> = vec![false; n];
    let mut confirmed_once: Vec<Option<bool>> = vec![None; n];
    let mut walk = Walk::new(n);

    let mut order = vec![root];
    done[root] = true;
    let mut rejected = HashMap::new();
    let mut ready: Vec<usize> = Vec::new();
    let mut last = root;
    loop {
        for &child in &graph.children[last] {
            blocked[child] = blocked[child].saturating_sub(1);
            if blocked[child] > 0 {
                continue;
            }
            // Ready: all it knew is behind. It waits for each operation still
            // to come, made beside it (that one did not know of it), that can
            // change the rights it rests on: its author's, or those of the
            // one it acts on.
            walk.next_round();
            for &p in &graph.rests[child] {
                for &s in &graph.changers[p] {
                    if s == child || done[s] || !walk.first_time(s) {
                        continue;
                    }
                    if !graph.knew(s, child, &done, &mut walk) {
                        holders[child].push(s);
                        if graph.node[s].author != graph.node[child].author {
                            others[child].push(s);
                        }
                        held[s].push(child);
                        waits[child] += 1;
                    }
                }
            }
            ready.push(child);
        }
        let rank = |i: usize| rank_of(&state, graph.node[i]);
        // Highest rank, then the smallest id.
        let by_rank = |a: &usize, b: &usize| rank(*a).cmp(&rank(*b)).then_with(|| b.cmp(a));
        let mut pick = ready.iter().copied().filter(|m| waits[*m] == 0).max_by(by_rank);
        if pick.is_none() && !ready.is_empty() {
            // Each waits for another: the one held back by the weakest goes
            // first. A removed admin's own operations, held by the owner's
            // removal, so never get ahead of those the admin holds back. What
            // an author holds back of their own (a leave, a handover beside
            // their other operations) weighs nothing here: it protects nobody
            // from anybody.
            let strength: Vec<u8> = ready.iter().map(|m| others[*m].iter().filter(|s| !done[**s]).map(|s| rank(*s)).max().unwrap_or(0)).collect();
            let weakest = strength.iter().copied().min().unwrap_or(0);
            let tied: Vec<usize> = ready.iter().zip(&strength).filter(|(_, s)| **s == weakest).map(|(m, _)| *m).collect();
            pick = if tied.len() == 1 {
                tied.first().copied()
            } else {
                // Among those, one taken by whom it concerns, not knowing
                // what it waits for, before one that was not: an owner who
                // handed the group over cannot hand it once more beside it,
                // after the new owner acted. Looked at once a pass for each
                // operation: a circle of many costs no more than that.
                let confirmed: Vec<bool> = tied
                    .iter()
                    .map(|m| {
                        *confirmed_once[*m].get_or_insert_with(|| {
                            graph.confirmed(*m, holders[*m].iter().copied().filter(|s| !done[*s]), &state, &done, &mut walk)
                        })
                    })
                    .collect();
                (0..tied.len()).max_by(|a, b| confirmed[*a].cmp(&confirmed[*b]).then_with(|| by_rank(&tied[*a], &tied[*b]))).map(|i| tied[i])
            };
        }
        let Some(pick) = pick else { break };
        ready.retain(|m| *m != pick);
        done[pick] = true;
        for &m in &held[pick] {
            waits[m] = waits[m].saturating_sub(1);
        }
        let op = graph.node[pick];
        if stale[pick] {
            rejected.insert(ids[pick].clone(), Rejection::StaleLink);
        } else if let Err(r) = state.apply(op) {
            rejected.insert(ids[pick].clone(), r);
        }
        order.push(pick);
        last = pick;
    }
    Some((order.into_iter().map(|i| ids[i].clone()).collect(), rejected, state))
}

/// The rank an operation is ordered by: its author's role in the state
/// reached so far, an owner's handover of the group above the owner's
/// other operations.
fn rank_of(state: &GroupState, op: &Op) -> u8 {
    match state.role_of(&op.author) {
        Some(Role::Owner) if matches!(op.body, OpBody::TransferOwnership { .. }) => Role::Owner.rank() + 1,
        Some(r) => r.rank(),
        None => 0,
    }
}

/// The operations of one pass by index, and who they concern.
struct Graph<'a> {
    node: Vec<&'a Op>,
    children: Vec<Vec<usize>>,
    parents: Vec<Vec<usize>>,
    /// Whose rights the permission of each operation rests on (people by index).
    rests: Vec<Vec<usize>>,
    /// Of each person, the operations that can change their rights.
    changers: Vec<Vec<usize>>,
}

impl<'a> Graph<'a> {
    fn new(node: Vec<&'a Op>, at: &HashMap<&OpId, usize>) -> Self {
        let n = node.len();
        let mut children = vec![Vec::new(); n];
        let mut parents = vec![Vec::new(); n];
        let mut people: HashMap<&str, usize> = HashMap::new();
        let mut rests = Vec::with_capacity(n);
        let mut changes = Vec::with_capacity(n);
        for (i, &op) in node.iter().enumerate() {
            for p in &op.parents {
                if let Some(&j) = at.get(p) {
                    children[j].push(i);
                    parents[i].push(j);
                }
            }
            let mut person = |k: &'a PubKey| {
                let next = people.len();
                *people.entry(k.as_hex()).or_insert(next)
            };
            rests.push(rests_on(op).into_iter().map(&mut person).collect::<Vec<_>>());
            changes.push(changes_rights_of(op).into_iter().map(&mut person).collect::<Vec<_>>());
        }
        let mut changers = vec![Vec::new(); people.len()];
        for (i, c) in changes.iter().enumerate() {
            for &p in c {
                if !changers[p].contains(&i) {
                    changers[p].push(i);
                }
            }
        }
        Self { node, children, parents, rests, changers }
    }

    /// Did the author of `of` know of `what`, an operation not yet ordered?
    /// Operations already ordered are passed by: everything they knew is
    /// ordered too, so `what` is not among it.
    fn knew(&self, of: usize, what: usize, done: &[bool], walk: &mut Walk) -> bool {
        walk.begin();
        walk.stack.extend(self.parents[of].iter().copied());
        while let Some(i) = walk.stack.pop() {
            if i == what {
                walk.stack.clear();
                return true;
            }
            if done[i] || !walk.visit(i) {
                continue;
            }
            walk.stack.extend(self.parents[i].iter().copied());
        }
        false
    }

    /// Did somebody go on from `of` without knowing any of `rivals`
    /// (operations not yet ordered), by an operation that holds? Who counts:
    /// the receiver of a handover for a handover, any member but its author
    /// otherwise; a member in `state`, with an operation that applies after
    /// `of` there. Whoever knew a rival is passed by, and so are those after
    /// them: they knew it too. Anyone can name an operation as a parent at
    /// any time; only a member it concerns, acting as the operation lets
    /// them, says it was taken.
    fn confirmed(&self, of: usize, rivals: impl Iterator<Item = usize>, state: &GroupState, done: &[bool], walk: &mut Walk) -> bool {
        if self.children[of].is_empty() {
            return false;
        }
        let op = self.node[of];
        let mut after = state.clone();
        if after.apply(op).is_err() {
            return false;
        }
        let counts = |who: &PubKey| match &op.body {
            OpBody::TransferOwnership { to } => who == to,
            _ => who != &op.author,
        };
        let rivals: Vec<usize> = rivals.collect();
        let mut seen: HashSet<usize> = HashSet::new();
        let mut queue: Vec<usize> = self.children[of].clone();
        while let Some(d) = queue.pop() {
            if !seen.insert(d) || rivals.iter().any(|r| *r == d || self.knew(d, *r, done, walk)) {
                continue;
            }
            let by = &self.node[d].author;
            if counts(by) && state.is_member(by) && after.clone().apply(self.node[d]).is_ok() {
                return true;
            }
            queue.extend(self.children[d].iter().copied());
        }
        false
    }
}

/// Marks for the walks of one pass, reused: a round number per node
/// instead of a fresh set each time.
struct Walk {
    seen: Vec<u32>,
    round: u32,
    first: Vec<u32>,
    pass: u32,
    stack: Vec<usize>,
}

impl Walk {
    fn new(n: usize) -> Self {
        Self { seen: vec![0; n], round: 0, first: vec![0; n], pass: 0, stack: Vec::new() }
    }
    fn begin(&mut self) {
        self.round += 1;
        self.stack.clear();
    }
    fn visit(&mut self, i: usize) -> bool {
        if self.seen[i] == self.round {
            return false;
        }
        self.seen[i] = self.round;
        true
    }
    /// For the holders of one ready operation: each counted once.
    fn next_round(&mut self) {
        self.pass += 1;
    }
    fn first_time(&mut self, i: usize) -> bool {
        if self.first[i] == self.pass {
            return false;
        }
        self.first[i] = self.pass;
        true
    }
}

/// Everything an operation's author had seen: the operation itself and
/// all its ancestors.
fn knowledge(ops: &HashMap<OpId, Op>, of: &OpId) -> HashSet<OpId> {
    let mut seen = HashSet::new();
    let mut stack = vec![of.clone()];
    while let Some(id) = stack.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if let Some(op) = ops.get(&id) {
            stack.extend(op.parents.iter().cloned());
        }
    }
    seen
}

/// Whose rights an operation can change: the one it removes, bans or gives
/// a role to, the author of a leave, the author and the receiver of a
/// handover.
fn changes_rights_of(op: &Op) -> Vec<&PubKey> {
    match &op.body {
        OpBody::Remove { who } | OpBody::Ban { who } | OpBody::SetRole { who, .. } => vec![who],
        OpBody::Leave => vec![&op.author],
        OpBody::TransferOwnership { to } => vec![&op.author, to],
        _ => vec![],
    }
}

/// Whose rights the permission of an operation rests on: its author's,
/// and those of the one it acts on (a role that grows meanwhile can put
/// the target out of the author's reach).
fn rests_on(op: &Op) -> Vec<&PubKey> {
    match &op.body {
        OpBody::Remove { who } | OpBody::Ban { who } | OpBody::SetRole { who, .. } | OpBody::SetMuted { who, .. } | OpBody::SetTrusted { who, .. } => vec![&op.author, who],
        OpBody::TransferOwnership { to } => vec![&op.author, to],
        _ => vec![&op.author],
    }
}

/// The current key is the one brought by the last applied operation that
/// brings a key (a key on a `Leave` is ignored). Whoever made it knew a
/// certain part of the log. Anyone who stopped being a member outside that
/// part still has the key; anyone who joined outside that part does not
/// have it yet.
fn key_status(ops: &HashMap<OpId, Op>, order: &[OpId], rejected: &HashMap<OpId, Rejection>, state: &mut GroupState) -> KeyStatus {
    if state.kind == GroupKind::Public || state.disbanded {
        state.key_stale = false;
        return KeyStatus::Good;
    }
    let applied: Vec<&Op> = order.iter().filter(|id| !rejected.contains_key(*id)).filter_map(|id| ops.get(id)).collect();
    let Some(key_op) = applied.iter().rev().find(|o| o.key.is_some() && !matches!(o.body, OpBody::Leave)) else {
        return KeyStatus::Good;
    };
    let key_id = key_op.id();
    let known = knowledge(ops, &key_id);
    let current: Option<&KeyId> = key_op.key.as_ref();
    debug_assert_eq!(current, state.current_key.as_ref());

    // Replay membership to learn who left and who joined outside `known`.
    let mut replay: Option<GroupState> = None;
    let mut departed: BTreeMap<String, bool> = BTreeMap::new();
    let mut arrived: BTreeSet<String> = BTreeSet::new();
    let mut managers: HashSet<String> = HashSet::new();
    for op in &applied {
        let before: BTreeSet<String> = replay.as_ref().map(|s| s.members.keys().cloned().collect()).unwrap_or_default();
        match replay.as_mut() {
            None => replay = GroupState::genesis(op).ok(),
            Some(s) => {
                let _ = s.apply(op);
            }
        }
        let after: BTreeSet<String> = replay.as_ref().map(|s| s.members.keys().cloned().collect()).unwrap_or_default();
        // Everyone who has been a manager.
        match &op.body {
            OpBody::Create { .. } => {
                managers.insert(op.author.as_hex().to_string());
            }
            OpBody::SetRole { who, role } if role.is_manager() => {
                managers.insert(who.as_hex().to_string());
            }
            OpBody::TransferOwnership { to } => {
                managers.insert(to.as_hex().to_string());
            }
            _ => {}
        }
        // Later or at the same time: whoever made the key did not know of it.
        let unknown_to_key = !known.contains(&op.id());
        // At the same time: and whoever made this did not know the key.
        let concurrent = unknown_to_key && !knowledge(ops, &op.id()).contains(&key_id);
        for gone in before.difference(&after) {
            if unknown_to_key {
                departed.insert(gone.clone(), true);
            }
            arrived.remove(gone);
        }
        for new in after.difference(&before) {
            if concurrent {
                arrived.insert(new.clone());
            }
            departed.remove(new);
        }
    }

    // An admission the order turned down because its author lost the
    // rights beside it (it does not know the operation, ordered before it,
    // that took them): a device of a version before that rule took it and
    // may have handed the key to the newcomer. A key chosen without knowing
    // of it is replaced, once for each author: when the key knew of one such
    // admission of theirs, more of them change nothing (the older devices
    // hand that author's newcomers whatever key there is anyway). A fresh
    // admission by someone who already lost the rights is turned down by
    // every version and counts for nothing.
    let pos: HashMap<&OpId, usize> = order.iter().enumerate().map(|(i, id)| (id, i)).collect();
    let mut late: BTreeMap<&str, bool> = BTreeMap::new();
    for (id, r) in rejected {
        if !matches!(r, Rejection::AuthorNotMember | Rejection::NotPermitted) {
            continue;
        }
        let Some(o) = ops.get(id) else { continue };
        if !matches!(&o.body, OpBody::Admit { who } if !state.is_member(who)) || !managers.contains(o.author.as_hex()) {
            continue;
        }
        let at = pos.get(id).copied().unwrap_or(0);
        let knew = knowledge(ops, id);
        let backdated = order[..at].iter().filter(|s| !rejected.contains_key(*s) && !knew.contains(*s)).filter_map(|s| ops.get(s)).any(|s| changes_rights_of(s).contains(&&o.author));
        if backdated {
            *late.entry(o.author.as_hex()).or_insert(false) |= known.contains(id);
        }
    }
    let admitted_elsewhere = late.values().any(|seen| !seen);
    // Whoever chose the key and is no longer a member still knows it.
    if !departed.is_empty() || admitted_elsewhere || state.key_stale || !state.is_member(&key_op.author) {
        state.key_stale = true;
        return KeyStatus::Rotate;
    }
    let lacking: Vec<PubKey> = arrived.iter().filter(|k| state.members.contains_key(*k)).filter_map(|k| PubKey::parse(k)).collect();
    if lacking.is_empty() {
        KeyStatus::Good
    } else {
        KeyStatus::Deliver(lacking)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roles::Role;
    use crate::op::tests::Settings;
    use crate::op::{PostMode, Premod};
    use crate::state::tests::{create, key, pk, GROUP};

    /// A device with its own copy of the log.
    struct Device {
        me: PubKey,
        log: OpLog,
        clock: i64,
    }

    impl Device {
        fn make(&mut self, body: OpBody, with_key: Option<u8>) -> Op {
            self.clock += 1;
            let mut op = self.log.next(&self.me, self.clock, body);
            if op.body == OpBody::Join {
                op = op.with_proof(crate::op::JoinProof { epoch: self.log.state().link_epoch, mac: "m".into() });
            }
            if let Some(k) = with_key {
                op = op.with_key(key(k));
            }
            assert_eq!(self.log.insert(op.clone()), Insert::Applied, "{:?}", op.body);
            op
        }

        fn fork(&self, me: &PubKey) -> Device {
            Device { me: me.clone(), log: self.log.clone(), clock: self.clock }
        }
    }

    /// Owner `o` with admins `a`, `b` and members `u`, `w`; history shown.
    fn team(kind: GroupKind, history: bool) -> (Device, [PubKey; 5]) {
        let who = [pk("0"), pk("a"), pk("b"), pk("c"), pk("d")];
        let [o, a, b, u, w] = who.clone();
        let mut d = Device { me: o.clone(), log: OpLog::create(create(kind, &o, history)).unwrap(), clock: 100 };
        let mut k = 40;
        for m in [&a, &b, &u, &w] {
            let needs = d.log.state().requires_key(&OpBody::Admit { who: m.clone() });
            k += 1;
            d.make(OpBody::Admit { who: m.clone() }, needs.then_some(k));
        }
        d.make(OpBody::SetRole { who: a, role: Role::Admin }, None);
        d.make(OpBody::SetRole { who: b, role: Role::Admin }, None);
        (d, who)
    }

    fn deliver(to: &mut Device, ops: &[&Op]) {
        for op in ops {
            to.log.insert((*op).clone());
        }
    }

    /// Every ordering of up to five operations.
    fn permutations<T: Clone>(items: &[T]) -> Vec<Vec<T>> {
        if items.len() <= 1 {
            return vec![items.to_vec()];
        }
        let mut out = Vec::new();
        for i in 0..items.len() {
            let mut rest = items.to_vec();
            let first = rest.remove(i);
            for mut p in permutations(&rest) {
                p.insert(0, first.clone());
                out.push(p);
            }
        }
        out
    }

    #[test]
    fn a_line_of_operations() {
        let (d, [o, a, ..]) = team(GroupKind::Private, true);
        assert_eq!(d.log.len(), 7);
        assert_eq!(d.log.heads().len(), 1);
        assert_eq!(d.log.state().members.len(), 5);
        assert_eq!(d.log.state().role_of(&a), Some(Role::Admin));
        assert_eq!(d.log.state().owner, o);
        assert_eq!(d.log.key_status(), &KeyStatus::Good);
        assert_eq!(d.log.ordered().count(), 7);

        // Stored operations in any order give the same log.
        let mut stored: Vec<Op> = d.log.ordered().cloned().collect();
        stored.reverse();
        let rebuilt = OpLog::from_ops(stored).unwrap();
        assert_eq!(rebuilt.state(), d.log.state());
        assert!(OpLog::from_ops(vec![]).is_err());
    }

    #[test]
    fn two_admins_at_once_converge_in_any_delivery_order() {
        let (base, [o, a, b, u, w]) = team(GroupKind::Private, true);
        let x = pk("e");
        // A removes u, B admits x, the owner renames; none saw the others.
        let mut da = base.fork(&a);
        let mut db = base.fork(&b);
        let mut d0 = base.fork(&o);
        let op_a = da.make(OpBody::Remove { who: u.clone() }, Some(50));
        let op_b = db.make(OpBody::Admit { who: x.clone() }, None);
        let op_o = d0.make(Settings { name: Some("Renamed".into()), ..Settings::default() }.body(), None);
        // Then B, having seen only its own, mutes w.
        let op_b2 = db.make(OpBody::SetMuted { who: w.clone(), muted: true, until: None }, None);

        let all = [op_a.clone(), op_b.clone(), op_o.clone(), op_b2.clone()];
        let mut states = Vec::new();
        for order in permutations(&all) {
            let mut d = base.fork(&w);
            for op in &order {
                d.log.insert(op.clone());
            }
            assert!(d.log.missing().is_empty());
            assert_eq!(d.log.len(), base.log.len() + 4);
            states.push((d.log.state().clone(), d.log.ordered().map(Op::id).collect::<Vec<_>>(), d.log.key_status().clone()));
        }
        let first = states[0].clone();
        assert!(states.iter().all(|s| s == &first), "24 delivery orders, one result");

        let s = &first.0;
        assert!(!s.is_member(&u) && s.is_member(&x), "both admins' actions took effect");
        assert_eq!(s.name, "Renamed");
        assert!(s.member(&w).unwrap().muted);
        // The owner's operation is ordered before the admins'.
        let pos = |op: &Op| first.1.iter().position(|id| id == &op.id()).unwrap();
        assert!(pos(&op_o) < pos(&op_a) && pos(&op_o) < pos(&op_b));
        assert!(pos(&op_b) < pos(&op_b2), "an operation never precedes its parent");

        // x was admitted without A's knowledge and A made the current key:
        // x does not have it yet.
        assert_eq!(first.2, KeyStatus::Deliver(vec![x.clone()]));

        // Three heads merge into one with the next operation.
        let mut d = base.fork(&o);
        deliver(&mut d, &[&op_a, &op_b, &op_o, &op_b2]);
        assert_eq!(d.log.heads().len(), 3);
        let merge = d.make(OpBody::RotateKey, Some(60));
        assert_eq!(merge.parents.len(), 3);
        assert_eq!(d.log.heads(), vec![merge.id()]);
        assert_eq!(d.log.key_status(), &KeyStatus::Good, "the new key was made knowing everything");
    }

    #[test]
    fn concurrent_removals_leave_a_stale_key_until_someone_rotates() {
        let (base, [o, a, b, u, w]) = team(GroupKind::Private, true);
        let mut da = base.fork(&a);
        let mut db = base.fork(&b);
        let op_a = da.make(OpBody::Remove { who: u.clone() }, Some(51));
        let op_b = db.make(OpBody::Remove { who: w.clone() }, Some(52));

        for order in [[&op_a, &op_b], [&op_b, &op_a]] {
            let mut d = base.fork(&o);
            deliver(&mut d, &order);
            let s = d.log.state();
            assert!(!s.is_member(&u) && !s.is_member(&w));
            // Whichever key ended up current, one removed member has it.
            assert_eq!(d.log.key_status(), &KeyStatus::Rotate);
            assert!(s.key_stale);
            assert_eq!(s.keys.len(), 3, "both keys are kept: messages under either can be read");

            let fix = d.make(OpBody::RotateKey, Some(53));
            assert_eq!(d.log.key_status(), &KeyStatus::Good);
            assert_eq!(d.log.state().current_key, Some(key(53)));
            assert!(!d.log.state().key_stale);

            // The devices of A and B agree once they have everything.
            for dev in [&mut da, &mut db] {
                let mut copy = dev.fork(&dev.me.clone());
                deliver(&mut copy, &[&op_a, &op_b, &fix]);
                assert_eq!(copy.log.state(), d.log.state());
            }
        }
    }

    #[test]
    fn an_operation_of_someone_removed_meanwhile_is_rejected() {
        let (base, [o, a, b, u, _]) = team(GroupKind::Private, true);
        let mut d0 = base.fork(&o);
        let mut db = base.fork(&b);
        // The owner removes admin B while B, unaware, removes u.
        let op_o = d0.make(OpBody::Remove { who: b.clone() }, Some(54));
        let op_b = db.make(OpBody::Remove { who: u.clone() }, Some(55));

        for order in [[&op_o, &op_b], [&op_b, &op_o]] {
            let mut d = base.fork(&a);
            deliver(&mut d, &order);
            assert!(!d.log.state().is_member(&b));
            assert!(d.log.state().is_member(&u), "B's removal did not happen");
            assert_eq!(d.log.rejection(&op_b.id()), Some(&Rejection::AuthorNotMember));
            assert_eq!(d.log.state().current_key, Some(key(54)), "a rejected operation brings no key");
            assert!(d.log.contains(&op_b.id()), "it stays in the graph");
        }
        // On B's own device the same: once the owner's operation arrives,
        // B's is undone.
        deliver(&mut db, &[&op_o]);
        assert!(db.log.state().is_member(&u));
        assert!(!db.log.state().is_member(&b));
        assert_eq!(db.log.insert(op_b.clone()), Insert::Duplicate);

        // Later operations may still name the rejected one as a parent.
        let mut d = base.fork(&a);
        deliver(&mut d, &[&op_o, &op_b]);
        let next = d.make(OpBody::SetMuted { who: u.clone(), muted: true, until: None }, None);
        assert!(next.parents.contains(&op_b.id()));
    }

    #[test]
    fn operations_wait_for_their_parents() {
        let (base, [_, a, _, u, w]) = team(GroupKind::Private, true);
        let mut da = base.fork(&a);
        let first = da.make(OpBody::SetMuted { who: u.clone(), muted: true, until: None }, None);
        let second = da.make(OpBody::SetMuted { who: w.clone(), muted: true, until: None }, None);
        let third = da.make(OpBody::SetMuted { who: u.clone(), muted: false, until: None }, None);

        let mut d = base.fork(&w);
        assert_eq!(d.log.insert(third.clone()), Insert::Waiting { missing: vec![second.id()] });
        assert_eq!(d.log.insert(second.clone()), Insert::Waiting { missing: vec![first.id()] });
        assert_eq!(d.log.missing(), vec![first.id()]);
        assert_eq!(d.log.state(), base.log.state(), "nothing applies before its parents");
        assert_eq!(d.log.insert(third.clone()), Insert::Duplicate);

        assert_eq!(d.log.insert(first), Insert::Applied);
        assert!(d.log.missing().is_empty());
        assert_eq!(d.log.state(), da.log.state());
        assert!(!d.log.state().member(&u).unwrap().muted && d.log.state().member(&w).unwrap().muted);
    }

    #[test]
    fn refusals() {
        let (mut d, [o, a, ..]) = team(GroupKind::Private, true);
        let mut foreign = d.log.next(&a, 1, OpBody::RotateKey).with_key(key(1));
        foreign.group_id = "ff".repeat(32);
        assert_eq!(d.log.insert(foreign), Insert::Refused(Rejection::WrongGroup));
        let mut second = create(GroupKind::Public, &a, false);
        second.created_at = 5;
        assert_eq!(d.log.insert(second), Insert::Refused(Rejection::AlreadyCreated));
        assert_eq!(d.log.insert(create(GroupKind::Private, &o, true)), Insert::Duplicate);
        let orphan = Op::new(GROUP, &a, vec![], 1, OpBody::RotateKey).with_key(key(1));
        assert!(matches!(d.log.insert(orphan), Insert::Refused(Rejection::Invalid { .. })));

        // `check` tells a command what would happen, without changing anything.
        let before = d.log.state().clone();
        let bad = d.log.next(&a, 1, OpBody::Disband);
        assert_eq!(d.log.check(&bad), Err(Rejection::NotPermitted));
        let good = d.log.next(&o, 1, OpBody::Disband);
        assert_eq!(d.log.check(&good), Ok(()));
        assert_eq!(d.log.state(), &before);
        assert_eq!(d.log.group_id(), GROUP);
    }

    #[test]
    fn leaving_makes_the_key_stale() {
        let (mut d, [_, a, _, u, _]) = team(GroupKind::Private, true);
        let mut du = d.fork(&u);
        let leave = du.make(OpBody::Leave, None);
        deliver(&mut d, &[&leave]);
        assert_eq!(d.log.key_status(), &KeyStatus::Rotate);
        let mut da = d.fork(&a);
        da.make(OpBody::RotateKey, Some(70));
        assert_eq!(da.log.key_status(), &KeyStatus::Good);
    }

    #[test]
    fn a_key_brought_on_a_leave_is_ignored() {
        let (mut d, [_, a, ..]) = team(GroupKind::Private, true);
        let before = d.log.state().current_key.clone();
        let mut da = d.fork(&a);
        // An admin leaves and brings a key of their own choosing with it.
        let leave = da.make(OpBody::Leave, Some(71));
        deliver(&mut d, &[&leave]);
        for log in [&d.log, &da.log] {
            assert!(!log.state().is_member(&a));
            assert_eq!(log.key_status(), &KeyStatus::Rotate);
            assert!(log.state().key_stale);
            assert_eq!(log.state().current_key, before, "the key of the one who left is not current");
            assert!(!log.state().keys.contains(&key(71)));
        }
    }

    #[test]
    fn hidden_history_gives_each_newcomer_a_fresh_key() {
        let (base, [_, a, b, ..]) = team(GroupKind::Private, false);
        let (x, y) = (pk("e"), pk("f"));
        let mut da = base.fork(&a);
        let mut db = base.fork(&b);
        assert_eq!(da.log.check(&da.log.next(&a, 1, OpBody::Admit { who: x.clone() })), Err(Rejection::KeyRequired));
        let op_a = da.make(OpBody::Admit { who: x.clone() }, Some(80));
        let op_b = db.make(OpBody::Admit { who: y.clone() }, Some(81));
        let mut d = base.fork(&a);
        deliver(&mut d, &[&op_a, &op_b]);
        assert!(d.log.state().is_member(&x) && d.log.state().is_member(&y));
        // One of the two keys is current; the newcomer admitted by the
        // other admin does not have it.
        match d.log.key_status() {
            KeyStatus::Deliver(list) => assert_eq!(list.len(), 1),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn public_groups_need_no_key_care() {
        let (base, [_, a, b, u, _]) = team(GroupKind::Public, false);
        let (x, y) = (pk("e"), pk("f"));
        let mut dx = base.fork(&x);
        let mut dy = base.fork(&y);
        let mut da = base.fork(&a);
        let mut db = base.fork(&b);
        let join_x = dx.make(OpBody::Join, None);
        let join_y = dy.make(OpBody::Join, None);
        let ban_y = da.make(OpBody::Ban { who: y.clone() }, None);
        let remove_u = db.make(OpBody::Remove { who: u.clone() }, None);

        let all = [join_x, join_y.clone(), ban_y, remove_u];
        let mut results = Vec::new();
        for order in permutations(&all) {
            let mut d = base.fork(&a);
            for op in &order {
                d.log.insert(op.clone());
            }
            results.push((d.log.state().clone(), d.log.key_status().clone(), d.log.rejection(&join_y.id()).cloned()));
        }
        assert!(results.iter().all(|r| r == &results[0]));
        let (s, status, y_join) = &results[0];
        assert!(s.is_member(&x) && !s.is_member(&u));
        // The admin's ban goes before the newcomer's join: the join fails.
        assert!(s.is_banned(&y) && !s.is_member(&y));
        assert_eq!(y_join, &Some(Rejection::AuthorBanned));
        assert_eq!(status, &KeyStatus::Good);
        assert_eq!(s.keys.len(), 1);
    }

    #[test]
    fn ownership_transfer_races_with_the_new_owner() {
        let (base, [o, a, b, ..]) = team(GroupKind::Private, true);
        let mut d0 = base.fork(&o);
        let mut da = base.fork(&a);
        // The owner hands the group to A while A, still an admin, tries to
        // make B... an owner's move A cannot make yet.
        let transfer = d0.make(OpBody::TransferOwnership { to: a.clone() }, None);
        assert_eq!(
            da.log.check(&da.log.next(&a, 1, OpBody::Remove { who: b.clone() }).with_key(key(90))),
            Err(Rejection::NotPermitted),
            "an admin cannot remove an admin"
        );
        deliver(&mut da, &[&transfer]);
        assert_eq!(da.log.state().role_of(&a), Some(Role::Owner));
        let removal = da.make(OpBody::Remove { who: b.clone() }, Some(90));
        deliver(&mut d0, &[&removal]);
        assert_eq!(d0.log.state(), da.log.state());
        assert_eq!(d0.log.state().role_of(&o), Some(Role::Admin));
        // The former owner can no longer disband.
        assert_eq!(d0.log.check(&d0.log.next(&o, 1, OpBody::Disband)), Err(Rejection::NotPermitted));
        let end = da.make(OpBody::Disband, None);
        deliver(&mut d0, &[&end]);
        assert!(d0.log.state().disbanded);
        assert_eq!(d0.log.key_status(), &KeyStatus::Good);
    }

    /// An operation of `author` on `parents` the author chose, dated so
    /// that its id is smaller than `below`: the tie-break of equal ranks
    /// goes its way.
    fn crafted(author: &PubKey, parents: Vec<OpId>, body: OpBody, with_key: Option<u8>, below: &OpId) -> Op {
        (1..10_000)
            .map(|at| {
                let op = Op::new(GROUP, author, parents.clone(), at, body.clone());
                match with_key {
                    Some(k) => op.with_key(key(k)),
                    None => op,
                }
            })
            .find(|op| &op.id() < below)
            .expect("an id below")
    }

    #[test]
    fn a_removed_admin_cannot_go_back_in_the_log() {
        let (base, [o, a, _, u, w]) = team(GroupKind::Private, true);
        let x = pk("e");
        let p = base.log.heads()[0].clone();
        // u leaves; the owner, having seen it, removes admin A.
        let mut du = base.fork(&u);
        let leave = du.make(OpBody::Leave, None);
        let mut d0 = base.fork(&o);
        deliver(&mut d0, &[&leave]);
        let removal = d0.make(OpBody::Remove { who: a.clone() }, Some(60));
        // A, removed, names an old part of the log as all it knew and
        // admits someone of their own.
        let admit = crafted(&a, vec![p.clone()], OpBody::Admit { who: x.clone() }, None, &leave.id());
        assert_eq!(d0.log.insert(admit.clone()), Insert::Rejected(Rejection::AuthorNotMember));
        assert!(!d0.log.state().is_member(&x));
        assert_eq!(d0.log.key_status(), &KeyStatus::Rotate, "a device of an older version may have handed x the key");
        let mut fix = d0.fork(&o);
        fix.make(OpBody::RotateKey, Some(66));
        assert_eq!(fix.log.key_status(), &KeyStatus::Good, "the new key knew of the admission");

        // To hold back the leave the removal waits for, A also changes
        // the role of the one leaving, as if beside it: a circle, in which
        // the one held back by the weaker goes first, and that is not A.
        let hold = crafted(&a, vec![p], OpBody::SetRole { who: u.clone(), role: Role::Moderator }, None, &leave.id());

        let all = [leave, removal, admit.clone(), hold];
        let mut results = Vec::new();
        for order in permutations(&all) {
            let mut d = base.fork(&w);
            for op in &order {
                d.log.insert(op.clone());
            }
            results.push((d.log.state().clone(), d.log.ordered().map(Op::id).collect::<Vec<_>>(), d.log.key_status().clone()));
        }
        assert!(results.iter().all(|r| r == &results[0]));
        assert!(!results[0].0.is_member(&x) && !results[0].0.is_member(&a));
        assert_ne!(results[0].2, KeyStatus::Deliver(vec![x]));
    }

    #[test]
    fn a_former_owner_cannot_undo_the_transfer() {
        let (base, [o, a, b, u, w]) = team(GroupKind::Private, true);
        let mut d0 = base.fork(&o);
        let transfer = d0.make(OpBody::TransferOwnership { to: b.clone() }, None);
        // The new owner acts on it.
        let mut db = base.fork(&b);
        deliver(&mut db, &[&transfer]);
        let acted = db.make(OpBody::SetRole { who: a.clone(), role: Role::Member }, None);

        // The former owner removes the new one, as if beside the transfer.
        let undo = crafted(&o, transfer.parents.clone(), OpBody::Remove { who: b.clone() }, Some(61), &transfer.id());
        // With an operation of their own on B beside both, the three wait
        // for each other in a circle where the owner's own holds weigh
        // nothing; the handover still goes first.
        let circle = crafted(&o, transfer.parents.clone(), OpBody::SetRole { who: b.clone(), role: Role::Moderator }, None, &transfer.id());
        for order in permutations(&[transfer.clone(), undo.clone(), acted.clone(), circle.clone()]) {
            let mut d = base.fork(&w);
            for op in &order {
                d.log.insert(op.clone());
            }
            let s = d.log.state();
            assert_eq!(s.owner, b);
            assert!(s.is_member(&b));
            assert_eq!(s.role_of(&o), Some(Role::Admin));
            assert_eq!(d.log.rejection(&undo.id()), Some(&Rejection::NotPermitted));
            assert_eq!(d.log.rejection(&transfer.id()), None);
            assert_eq!(s.role_of(&a), Some(Role::Member), "what the new owner did stands");
        }

        // Or hands the group to somebody else beside it, a smaller id
        // winning a tie of the old rule: the handover the new owner went on
        // from comes first, and the second is the move of an admin.
        let again = crafted(&o, transfer.parents.clone(), OpBody::TransferOwnership { to: w.clone() }, None, &transfer.id());
        // Anybody can name the second as a parent, a key nobody knows, a
        // member it does not concern, or the receiver with an operation
        // that does not hold: none of it says the second was taken.
        let stranger = Op::new(GROUP, &pk("f"), vec![again.id()], 1, OpBody::Leave);
        let bystander = Op::new(GROUP, &u, vec![again.id()], 1, OpBody::SetMuted { who: w.clone(), muted: true, until: None });
        let receiver_leaving = Op::new(GROUP, &w, vec![again.id()], 1, OpBody::Leave);
        let named = [stranger, bystander, receiver_leaving];
        for (order, first) in permutations(&[transfer.clone(), again.clone(), acted.clone()]).into_iter().flat_map(|o| [(o.clone(), true), (o, false)]) {
            let mut d = base.fork(&u);
            let all: Vec<&Op> = if first { named.iter().chain(&order).collect() } else { order.iter().chain(&named).collect() };
            for op in all {
                d.log.insert(op.clone());
            }
            let s = d.log.state();
            assert_eq!(s.owner, b);
            assert_eq!(d.log.rejection(&again.id()), Some(&Rejection::NotPermitted));
            assert_eq!(s.role_of(&a), Some(Role::Member));
        }
        // What is left: the receiver of the second, a member, acting as an
        // owner on it beside the first. Each handover is taken by its own
        // receiver; the old rule decides, the same everywhere.
        let taken = Op::new(GROUP, &w, vec![again.id()], 1, OpBody::SetRole { who: a.clone(), role: Role::Moderator });
        let mut results = Vec::new();
        for order in permutations(&[transfer.clone(), again.clone(), acted.clone(), taken.clone()]) {
            let mut d = base.fork(&u);
            for op in &order {
                d.log.insert(op.clone());
            }
            results.push(d.log.state().clone());
        }
        assert!(results.iter().all(|s| s == &results[0]));
        assert_eq!(results[0].owner, w, "the smaller id");

        // Two handovers nobody went on from yet: the old rule, the same
        // everywhere.
        let mut d = base.fork(&u);
        deliver(&mut d, &[&transfer, &again]);
        assert_eq!(d.log.state().owner, w, "the smaller id");
        let mut d = base.fork(&u);
        deliver(&mut d, &[&again, &transfer]);
        assert_eq!(d.log.state().owner, w);
    }

    #[test]
    fn a_demoted_admin_forging_admissions_costs_one_new_key() {
        let (base, [o, a, ..]) = team(GroupKind::Private, true);
        let old = base.log.heads();
        let mut d0 = base.fork(&o);
        d0.make(OpBody::SetRole { who: a.clone(), role: Role::Member }, None);
        // Admissions made knowing the demotion: every version turns them down.
        for i in 0..5u8 {
            let fresh = Op::new(GROUP, &a, d0.log.heads(), 300 + i64::from(i), OpBody::Admit { who: pk(&format!("{}", i + 1)) });
            assert_eq!(d0.log.insert(fresh), Insert::Rejected(Rejection::NotPermitted));
        }
        assert_eq!(d0.log.key_status(), &KeyStatus::Good, "nothing an older device took");
        // One named on the part of the log before the demotion: an older
        // device took it. One new key, and no more for the same author.
        let back = Op::new(GROUP, &a, old.clone(), 400, OpBody::Admit { who: pk("e") });
        assert_eq!(d0.log.insert(back), Insert::Rejected(Rejection::NotPermitted));
        assert_eq!(d0.log.key_status(), &KeyStatus::Rotate);
        d0.make(OpBody::RotateKey, Some(67));
        assert_eq!(d0.log.key_status(), &KeyStatus::Good);
        let again = Op::new(GROUP, &a, old, 401, OpBody::Admit { who: pk("f") });
        assert_eq!(d0.log.insert(again), Insert::Rejected(Rejection::NotPermitted));
        assert_eq!(d0.log.key_status(), &KeyStatus::Good, "once for each author");
    }

    #[test]
    fn a_target_raised_beside_an_action_is_out_of_its_reach() {
        let (base, [o, a, b, u, w]) = team(GroupKind::Private, true);
        let x = w.clone();
        let p = base.log.heads()[0].clone();
        // u leaves; the owner, having seen it, makes x an admin. Admin A,
        // naming the part of the log before the leave, acts on x.
        let mut du = base.fork(&u);
        let leave = du.make(OpBody::Leave, None);
        let mut d0 = base.fork(&o);
        deliver(&mut d0, &[&leave]);
        let raise = d0.make(OpBody::SetRole { who: x.clone(), role: Role::Admin }, None);
        for body in [
            OpBody::Remove { who: x.clone() },
            OpBody::Ban { who: x.clone() },
            OpBody::SetMuted { who: x.clone(), muted: true, until: None },
            OpBody::SetRole { who: x.clone(), role: Role::Member },
        ] {
            let with_key = matches!(body, OpBody::Remove { .. } | OpBody::Ban { .. }).then_some(64);
            let act = crafted(&a, vec![p.clone()], body.clone(), with_key, &leave.id());
            for order in permutations(&[leave.clone(), raise.clone(), act.clone()]) {
                let mut d = base.fork(&b);
                for op in &order {
                    d.log.insert(op.clone());
                }
                let s = d.log.state();
                assert_eq!(s.role_of(&x), Some(Role::Admin), "{body:?}");
                assert!(!s.member(&x).unwrap().muted);
                assert_eq!(d.log.rejection(&act.id()), Some(&Rejection::NotPermitted), "{body:?}");
                assert_eq!(d.log.rejection(&raise.id()), None);
            }
        }

        // An admin removes a member the owner hands the group to, beside it.
        let mut d0 = base.fork(&o);
        deliver(&mut d0, &[&leave]);
        let transfer = d0.make(OpBody::TransferOwnership { to: x.clone() }, None);
        let removal = crafted(&a, vec![p.clone()], OpBody::Remove { who: x.clone() }, Some(65), &leave.id());
        for order in permutations(&[leave.clone(), transfer.clone(), removal.clone()]) {
            let mut d = base.fork(&b);
            for op in &order {
                d.log.insert(op.clone());
            }
            assert_eq!(d.log.state().owner, x);
            assert_eq!(d.log.rejection(&removal.id()), Some(&Rejection::NotPermitted));
        }
    }

    #[test]
    fn two_admins_removing_each_other_converge() {
        let (base, [o, a, b, u, _]) = team(GroupKind::Private, true);
        let heads = base.log.heads();
        // Each operation waits for the other: neither may go first by the
        // rule; the one held back by the weaker goes first.
        let by_a = Op::new(GROUP, &a, heads.clone(), 300, OpBody::Remove { who: b.clone() }).with_key(key(62));
        let by_b = Op::new(GROUP, &b, heads.clone(), 300, OpBody::Remove { who: a.clone() }).with_key(key(63));
        let mut du = base.fork(&u);
        let leave = du.make(OpBody::Leave, None);
        let mut d0 = base.fork(&o);
        let demote = d0.make(OpBody::SetRole { who: a.clone(), role: Role::Member }, None);

        let all = [by_a, by_b, leave, demote];
        let mut results = Vec::new();
        for order in permutations(&all) {
            let mut d = base.fork(&o);
            for op in &order {
                d.log.insert(op.clone());
            }
            results.push((d.log.state().clone(), d.log.ordered().map(Op::id).collect::<Vec<_>>(), d.log.key_status().clone()));
        }
        assert!(results.iter().all(|r| r == &results[0]), "24 delivery orders, one result");
        let s = &results[0].0;
        // The owner's demotion goes first; A, a member by then, is removed
        // by B, and A's own removal of B comes from someone no longer there.
        assert!(!s.is_member(&a) && s.is_member(&b));
        assert_eq!(s.role_of(&b), Some(Role::Admin));
        assert!(!s.is_member(&u));
        let mut d = base.fork(&o);
        deliver(&mut d, &[&all[0], &all[1]]);
        assert!(d.log.state().is_member(&a) && d.log.state().is_member(&b), "without it, an admin removes no admin");
        assert_eq!(d.log.rejection(&all[0].id()), Some(&Rejection::NotPermitted));
        assert_eq!(d.log.rejection(&all[1].id()), Some(&Rejection::NotPermitted));
    }

    /// Small deterministic generator: the test must fail the same way twice.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self, below: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % below as u64) as usize
        }
    }

    #[test]
    fn random_histories_converge() {
        for seed in 1..=40u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let (base, [o, a, b, u, w]) = team(GroupKind::Private, seed % 2 == 0);
            let people = [o.clone(), a.clone(), b.clone(), u.clone(), w.clone(), pk("e"), pk("f")];
            let mut devices: Vec<Device> = [&o, &a, &b].into_iter().map(|p| base.fork(p)).collect();
            let mut made: Vec<Op> = Vec::new();
            let mut key_no = 100u8;

            for _ in 0..18 {
                let d = rng.next(devices.len());
                // Sometimes a device hears about something first.
                if !made.is_empty() && rng.next(3) == 0 {
                    let heard = made[rng.next(made.len())].clone();
                    devices[d].log.insert(heard);
                }
                let target = people[rng.next(people.len())].clone();
                let body = match rng.next(10) {
                    0 => OpBody::Admit { who: target },
                    1 => OpBody::Remove { who: target },
                    2 => OpBody::SetMuted { who: target, muted: rng.next(2) == 0, until: None },
                    3 => OpBody::SetRole { who: target, role: [Role::Member, Role::Moderator, Role::Admin][rng.next(3)] },
                    4 => OpBody::Ban { who: target },
                    5 => Settings { name: Some(format!("n{}", rng.next(9))), ..Settings::default() }.body(),
                    // The bodies of version 2: a mute with an end, trust,
                    // the rules of posting.
                    6 => OpBody::SetMuted { who: target, muted: true, until: Some(1_000 + rng.next(500) as i64) },
                    7 => OpBody::SetTrusted { who: target, trusted: rng.next(2) == 0 },
                    8 => Settings {
                        post_mode: Some([PostMode::All, PostMode::Managers][rng.next(2)]),
                        premod: Some([Premod::Off, Premod::Newcomers, Premod::All][rng.next(3)]),
                        ..Settings::default()
                    }
                    .body(),
                    _ => OpBody::RotateKey,
                };
                let dev = &mut devices[d];
                dev.clock += 1;
                let mut op = dev.log.next(&dev.me, dev.clock, body);
                if dev.log.state().requires_key(&op.body) {
                    key_no = key_no.wrapping_add(1);
                    op = op.with_key(key(key_no));
                }
                // Devices publish only what is allowed where they stand.
                if dev.log.check(&op).is_ok() {
                    dev.log.insert(op.clone());
                    made.push(op);
                }
            }

            // Everyone gets everything, each in an order of their own.
            let mut results = Vec::new();
            for round in 0..4 {
                let mut order = made.clone();
                for i in (1..order.len()).rev() {
                    order.swap(i, rng.next(i + 1));
                }
                let mut d = base.fork(&w);
                if round == 0 {
                    d = devices.remove(0);
                }
                for op in order {
                    d.log.insert(op);
                }
                assert!(d.log.missing().is_empty(), "seed {seed}");
                assert_eq!(d.log.len(), base.log.len() + made.len(), "seed {seed}");
                results.push((d.log.state().clone(), d.log.ordered().map(Op::id).collect::<Vec<_>>(), d.log.key_status().clone()));
            }
            assert!(results.iter().all(|r| r == &results[0]), "seed {seed}: devices disagree");
            // Whatever happened, there is exactly one owner and they are a member.
            let s = &results[0].0;
            assert_eq!(s.members.values().filter(|m| m.role == Role::Owner).count(), 1, "seed {seed}");
            assert_eq!(s.role_of(&s.owner), Some(Role::Owner), "seed {seed}");
            assert!(s.banned.iter().all(|k| !s.members.contains_key(k)), "seed {seed}: a banned member");
        }
    }

    #[test]
    fn random_histories_with_old_parents_converge() {
        for seed in 1..=60u64 {
            let mut rng = Rng(seed.wrapping_mul(0xD1B5_4A32_D192_ED03));
            let (base, [o, a, b, u, w]) = team(GroupKind::Private, seed % 2 == 0);
            let people = [o.clone(), a.clone(), b.clone(), u.clone(), w.clone(), pk("e")];
            let mut devices: Vec<Device> = [&o, &a, &b, &u].into_iter().map(|p| base.fork(p)).collect();
            let mut made: Vec<Op> = Vec::new();
            let mut key_no = 150u8;

            for _ in 0..14 {
                let d = rng.next(devices.len());
                if !made.is_empty() && rng.next(3) == 0 {
                    let heard = made[rng.next(made.len())].clone();
                    devices[d].log.insert(heard);
                }
                let target = people[rng.next(people.len())].clone();
                let body = match rng.next(9) {
                    0 => OpBody::Admit { who: target },
                    1 => OpBody::Remove { who: target },
                    2 => OpBody::SetRole { who: target, role: [Role::Member, Role::Moderator, Role::Admin][rng.next(3)] },
                    3 => OpBody::Ban { who: target },
                    4 => OpBody::Leave,
                    5 => OpBody::TransferOwnership { to: target },
                    6 => OpBody::SetTrusted { who: target, trusted: true },
                    7 => OpBody::SetMuted { who: target, muted: true, until: Some(5_000) },
                    _ => OpBody::SetMuted { who: target, muted: true, until: None },
                };
                let dev = &mut devices[d];
                dev.clock += 1;
                // Half of the time the author names an old part of the log
                // as all they knew, and nothing is checked before.
                let parents = if rng.next(2) == 0 {
                    let known: Vec<OpId> = base.log.ordered().map(Op::id).chain(made.iter().map(Op::id)).collect();
                    vec![known[rng.next(known.len())].clone()]
                } else {
                    dev.log.heads()
                };
                let mut op = Op::new(GROUP, &dev.me, parents, dev.clock, body);
                if dev.log.state().requires_key(&op.body) || rng.next(4) == 0 {
                    key_no = key_no.wrapping_add(1);
                    op = op.with_key(key(key_no));
                }
                if made.iter().any(|m| m.id() == op.id()) {
                    continue;
                }
                dev.log.insert(op.clone());
                made.push(op);
                // Now and then a key nobody knows names something as its
                // parent, as if it went on from it.
                if rng.next(4) == 0 {
                    let named = made[rng.next(made.len())].id();
                    let stranger = pk(&format!("{}", 1 + rng.next(5)));
                    let op = Op::new(GROUP, &stranger, vec![named], dev.clock, OpBody::Leave);
                    if !made.iter().any(|m| m.id() == op.id()) {
                        made.push(op);
                    }
                }
            }

            let mut results = Vec::new();
            let mut last = None;
            for _ in 0..4 {
                let mut order = made.clone();
                for i in (1..order.len()).rev() {
                    order.swap(i, rng.next(i + 1));
                }
                let mut d = base.fork(&w);
                for op in order {
                    d.log.insert(op);
                }
                assert!(d.log.missing().is_empty(), "seed {seed}");
                results.push((d.log.state().clone(), d.log.ordered().map(Op::id).collect::<Vec<_>>(), d.log.key_status().clone()));
                last = Some(d.log);
            }
            assert!(results.iter().all(|r| r == &results[0]), "seed {seed}: devices disagree");
            let (s, order, _) = &results[0];
            assert_eq!(s.members.values().filter(|m| m.role == Role::Owner).count(), 1, "seed {seed}");
            assert_eq!(s.role_of(&s.owner), Some(Role::Owner), "seed {seed}");

            // What the owner changes of someone's rights, and a handover,
            // go before whatever was made beside them resting on those
            // rights. (Between those of equal standing a circle of such
            // operations can still be broken either way; it is broken the
            // same way everywhere.)
            let log = last.unwrap();
            let pos = |id: &OpId| order.iter().position(|o| o == id).unwrap();
            let applied: Vec<&Op> = log.ordered().filter(|op| log.rejection(&op.id()).is_none()).collect();
            let beside_of = |x: &OpId, y: &OpId| !knowledge(&log.ops, x).contains(y) && !knowledge(&log.ops, y).contains(x);
            // The owner is still the owner where `t` stands: no handover of
            // theirs went before it.
            let still_owner = |t: &Op| !applied.iter().any(|h| h.author == o && matches!(h.body, OpBody::TransferOwnership { .. }) && pos(&h.id()) < pos(&t.id()));
            for m in &applied {
                for t in &applied {
                    let (mid, tid) = (m.id(), t.id());
                    // Between the operations of one author the rule of circles
                    // decides: their order among themselves gives the author
                    // nothing they could not do anyway, but around a handover,
                    // which ranks above the rest and is taken by its receiver
                    // (a_former_owner_cannot_undo_the_transfer).
                    if mid == tid || m.author == t.author || !changes_rights_of(t).iter().any(|p| rests_on(m).contains(p)) {
                        continue;
                    }
                    let strong = match t.body {
                        // Several handovers of one owner beside each other are
                        // the owner's own disorder (several devices, or one
                        // lying): one of them goes first by the rule of circles,
                        // and the owner's other operations around them may go
                        // either way. A second handover after the new owner
                        // acted is a_former_owner_cannot_undo_the_transfer. A
                        // handover by someone not the owner yet (it counts
                        // once a handover to them went first) is held as
                        // what its author is when it waits, not as an owner.
                        OpBody::TransferOwnership { .. } => {
                            t.author == o
                                && still_owner(t)
                                && !log.ops.values().any(|h| h.author == o && h.id() != tid && matches!(h.body, OpBody::TransferOwnership { .. }) && beside_of(&h.id(), &tid))
                        }
                        OpBody::Leave => false,
                        _ => t.author == o && still_owner(t),
                    };
                    // Each resting on what the other changes: a circle, broken
                    // by the rule of circles (the tests above take the cases).
                    let circle = changes_rights_of(m).iter().any(|p| rests_on(t).contains(p));
                    let beside = !circle && beside_of(&mid, &tid);
                    assert!(!strong || !beside || pos(&tid) < pos(&mid), "seed {seed}: {:?} went before {:?}", m.body, t.body);
                }
            }
        }
    }
}
