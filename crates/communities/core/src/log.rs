// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from crates/messenger/groups/src/log.rs at 1f2ef31d, then changed for channels.

//! The journal of a channel: the operations a device knows, their one
//! agreed order, and the state that order produces (wire §3: the order of
//! groups, rules 1–4, and nothing more).
//!
//! Operations form a graph: each names the operations its author had
//! seen. Two admins acting at the same moment produce two operations with
//! the same parents; nobody waits for anybody. The order is then decided
//! by a rule every device applies to the same graph:
//!
//! 1. an operation comes after all of its parents;
//! 2. an operation that can change someone's rights (removes, bans, gives
//!    a role to them, hands them the channel, or is their own handover)
//!    comes before whatever was made beside it, neither knowing of the
//!    other, by that person or acting on them: an author chooses the
//!    parents, and naming an old part of the journal must not put an
//!    operation ahead of the one that took its ground. Where such
//!    operations wait for each other in a circle, the one held back by the
//!    lowest role (its author's own operations not counted) goes first,
//!    then one taken by whom it concerns (see `Graph::confirmed`);
//! 3. among operations that are ready, the one whose author has the
//!    higher role (in the state reached so far) goes first, an owner's
//!    handover of the channel above the owner's other operations;
//! 4. with equal roles, the one with the smaller id.
//!
//! An operation that lost its ground on the way (its author was removed
//! by one that went first) is rejected and changes nothing, but stays in
//! the graph so that later operations can still refer to it.
//!
//! The order is recomputed from the first operation whenever the graph
//! changes. The journal holds the staff, not the readers: thousands of
//! operations, not millions.

use crate::op::{Op, OpBody, OpId, MAX_PARENTS};
use crate::roles::Role;
use crate::state::{ChannelState, Rejection};
use messenger_core::PubKey;
use std::collections::{BTreeSet, HashMap, HashSet};

/// What happened to an operation handed to `insert`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Insert {
    /// In the journal; the state reflects it.
    Applied,
    /// In the journal, but it does not apply (and why).
    Rejected(Rejection),
    /// Already known.
    Duplicate,
    /// Some parents are unknown; kept aside until they arrive.
    Waiting { missing: Vec<OpId> },
    /// Not an operation of this channel, a second `Create`, or one that
    /// names no parents or too many.
    Refused(Rejection),
}

#[derive(Clone, Debug)]
pub struct OpLog {
    community_id: String,
    ops: HashMap<OpId, Op>,
    waiting: HashMap<OpId, Op>,
    /// The agreed order, rejected operations included.
    order: Vec<OpId>,
    rejected: HashMap<OpId, Rejection>,
    state: ChannelState,
}

/// What every operation but the first names: one parent at least, at
/// most `MAX_PARENTS`.
fn parents_fit(op: &Op) -> bool {
    !op.parents.is_empty() && op.parents.len() <= MAX_PARENTS
}

impl OpLog {
    /// Start a journal from the `Create` operation.
    pub fn create(op: Op) -> Result<Self, Rejection> {
        let state = ChannelState::genesis(&op)?;
        let id = op.id();
        let mut ops = HashMap::new();
        let community_id = op.community_id.clone();
        ops.insert(id.clone(), op);
        Ok(Self { community_id, ops, waiting: HashMap::new(), order: vec![id], rejected: HashMap::new(), state })
    }

    /// Rebuild a journal from stored operations, in any order.
    pub fn from_ops(ops: impl IntoIterator<Item = Op>) -> Result<Self, Rejection> {
        let mut all: Vec<Op> = ops.into_iter().collect();
        let at = all.iter().position(|o| matches!(o.body, OpBody::Create { .. })).ok_or(Rejection::NotCreated)?;
        let mut log = Self::create(all.swap_remove(at))?;
        // All of them first, the order once: not once per operation.
        let mut aside: HashMap<OpId, Op> = HashMap::new();
        for op in all {
            let id = op.id();
            let refused = op.community_id != log.community_id || matches!(op.body, OpBody::Create { .. }) || !parents_fit(&op);
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

    pub fn community_id(&self) -> &str {
        &self.community_id
    }

    pub fn state(&self) -> &ChannelState {
        &self.state
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

    pub fn get(&self, id: &OpId) -> Option<&Op> {
        self.ops.get(id)
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

    /// Applied operations no applied operation refers to yet. Sorted.
    /// A rejected operation stays in the graph but is never a head: spam
    /// dated ahead would otherwise take every parent slot of the next
    /// operation (`parents_for_next`).
    pub fn heads(&self) -> Vec<OpId> {
        let applied = |id: &OpId| !self.rejected.contains_key(id);
        let referenced: HashSet<&OpId> = self.ops.iter().filter(|(id, _)| applied(id)).flat_map(|(_, o)| o.parents.iter()).collect();
        let mut heads: Vec<OpId> = self.ops.keys().filter(|id| applied(id) && !referenced.contains(id)).cloned().collect();
        heads.sort();
        heads
    }

    /// Whether `who` may have written operations of this journal: the
    /// owner, somebody an operation here named staff (whatever became of
    /// it: a removed moderator's operation made meanwhile is still named
    /// as a parent by others), or the one a handover names. Anybody
    /// else's operation is junk that would only be kept, rejected,
    /// forever.
    pub fn may_author(&self, who: &PubKey) -> bool {
        if self.state.owner == *who || self.state.role_of(who).is_some() {
            return true;
        }
        self.ops.values().any(|o| match &o.body {
            OpBody::Create { .. } => o.author == *who,
            OpBody::SetRole { who: w, .. } => w == who,
            OpBody::TransferOwnership { to } => to == who,
            _ => false,
        })
    }

    /// How many operations wait for their parents.
    pub fn waiting_len(&self) -> usize {
        self.waiting.len()
    }

    /// What a new operation names as its parents: the newest heads (by
    /// their authors' clocks, then the larger id), at most `MAX_PARENTS`,
    /// sorted. A head left out stays a head and is named next time.
    pub fn parents_for_next(&self) -> Vec<OpId> {
        let mut heads: Vec<(i64, OpId)> = self.heads().into_iter().map(|id| (self.ops.get(&id).map_or(0, |o| o.created_at), id)).collect();
        heads.sort_by(|a, b| b.cmp(a));
        let mut chosen: Vec<OpId> = heads.into_iter().take(MAX_PARENTS).map(|(_, id)| id).collect();
        chosen.sort();
        chosen
    }

    /// A new operation by `author` on top of what is known here.
    pub fn next(&self, author: &PubKey, created_at: i64, body: OpBody) -> Op {
        Op::new(&self.community_id, author, self.parents_for_next(), created_at, body)
    }

    /// Newest `created_at` of the operations the journal took (without the
    /// rejected ones): a live message is judged no earlier than this, by
    /// the journal as it stands now (wire §4, `rules_at`).
    pub fn head_time(&self) -> i64 {
        self.ordered().filter(|o| self.rejection(&o.id()).is_none()).map(|o| o.created_at).max().unwrap_or(0)
    }

    /// Who hands on the secret of `link_epoch` (wire §4, §10): the author
    /// of the `create` (link epoch 0) or of the applied `rotate_link`
    /// that made it. None while that operation is not here.
    pub fn link_author(&self, link_epoch: u32) -> Option<&PubKey> {
        self.ordered()
            .filter(|o| self.rejection(&o.id()).is_none())
            .find(|o| match &o.body {
                OpBody::Create { link_epoch: e, .. } | OpBody::RotateLink { link_epoch: e } => *e == link_epoch,
                _ => false,
            })
            .map(|o| &o.author)
    }

    /// The date of the `create`: when the channel was made.
    pub fn created_at(&self) -> Option<i64> {
        self.ordered().find(|o| matches!(o.body, OpBody::Create { .. })).map(|o| o.created_at)
    }

    /// The author of the `create`: the owner a link (`o`) names (wire §3).
    pub fn creator(&self) -> Option<&PubKey> {
        self.ordered().find(|o| matches!(o.body, OpBody::Create { .. })).map(|o| &o.author)
    }

    /// Would this operation apply right now? For commands: refuse before
    /// anything is published.
    pub fn check(&self, op: &Op) -> Result<(), Rejection> {
        self.state.clone().apply(op)
    }

    /// Add an operation that arrived (or was just made here).
    pub fn insert(&mut self, op: Op) -> Insert {
        if op.community_id != self.community_id {
            return Insert::Refused(Rejection::WrongChannel);
        }
        if matches!(op.body, OpBody::Create { .. }) {
            return if self.ops.contains_key(&op.id()) { Insert::Duplicate } else { Insert::Refused(Rejection::AlreadyCreated) };
        }
        if op.parents.is_empty() {
            return Insert::Refused(Rejection::Invalid { what: "no parents".into() });
        }
        if op.parents.len() > MAX_PARENTS {
            return Insert::Refused(Rejection::Invalid { what: "parents".into() });
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
        let Some((order, rejected, state)) = linearize(&self.ops) else { return };
        self.order = order;
        self.rejected = rejected;
        self.state = state;
    }
}

/// One pass: the order of a set of operations and the state it produces.
///
/// Operations are taken by index here, the indices in the order of their
/// ids: the smaller index is the smaller id.
fn linearize(ops: &HashMap<OpId, Op>) -> Option<(Vec<OpId>, HashMap<OpId, Rejection>, ChannelState)> {
    let (root_id, root) = ops.iter().find(|(_, o)| matches!(o.body, OpBody::Create { .. }))?;
    let mut state = ChannelState::genesis(root).ok()?;

    let mut ids: Vec<&OpId> = ops.keys().collect();
    ids.sort();
    let at: HashMap<&OpId, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let graph = Graph::new(ids.iter().map(|id| &ops[*id]).collect(), &at);
    let n = graph.node.len();
    let root = at[root_id];

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
                // handed the channel over cannot hand it once more beside it,
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
        if let Err(r) = state.apply(op) {
            rejected.insert(ids[pick].clone(), r);
        }
        order.push(pick);
        last = pick;
    }
    Some((order.into_iter().map(|i| ids[i].clone()).collect(), rejected, state))
}

/// The rank an operation is ordered by: its author's role in the state
/// reached so far, an owner's handover of the channel above the owner's
/// other operations. The keeper of the owner a handover waits for is the
/// other half of that handover (it puts it in force, `ChannelState::apply`)
/// and ranks with it: the old owner cannot get ahead of it beside it.
fn rank_of(state: &ChannelState, op: &Op) -> u8 {
    if matches!(op.body, OpBody::Keeper { .. }) && state.pending_owner.as_ref() == Some(&op.author) {
        return Role::Owner.rank() + 1;
    }
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
    fn confirmed(&self, of: usize, rivals: impl Iterator<Item = usize>, state: &ChannelState, done: &[bool], walk: &mut Walk) -> bool {
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
#[cfg(test)]
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
/// a role to, the author and the receiver of a handover, the author of a
/// keeper (a new owner's keeper puts the handover in force).
fn changes_rights_of(op: &Op) -> Vec<&PubKey> {
    match &op.body {
        OpBody::Remove { who } | OpBody::Ban { who, .. } | OpBody::SetRole { who, .. } => vec![who],
        OpBody::TransferOwnership { to } => vec![&op.author, to],
        OpBody::Keeper { .. } => vec![&op.author],
        _ => vec![],
    }
}

/// Whose rights the permission of an operation rests on: its author's,
/// and those of the one it acts on (a role that grows meanwhile can put
/// the target out of the author's reach).
fn rests_on(op: &Op) -> Vec<&PubKey> {
    match &op.body {
        OpBody::Remove { who }
        | OpBody::Ban { who, .. }
        | OpBody::SetRole { who, .. }
        | OpBody::SetMuted { who, .. }
        | OpBody::SetQuota { who, .. }
        | OpBody::SetTrusted { who, .. } => vec![&op.author, who],
        OpBody::TransferOwnership { to } => vec![&op.author, to],
        _ => vec![&op.author],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op::tests::Settings;
    use crate::op::{PostMode, Premod};
    use crate::state::tests::{create, pk, CHANNEL};

    /// A device with its own copy of the journal.
    struct Device {
        me: PubKey,
        log: OpLog,
        clock: i64,
    }

    impl Device {
        fn make(&mut self, body: OpBody) -> Op {
            self.clock += 1;
            let op = self.log.next(&self.me, self.clock, body);
            assert_eq!(self.log.insert(op.clone()), Insert::Applied, "{:?}", op.body);
            op
        }

        fn fork(&self, me: &PubKey) -> Device {
            Device { me: me.clone(), log: self.log.clone(), clock: self.clock }
        }
    }

    /// Owner `o` with admins `a`, `b`, author `w`; `u` a reader the
    /// journal mutes (so it names them).
    fn team() -> (Device, [PubKey; 5]) {
        let who = [pk("0"), pk("a"), pk("b"), pk("c"), pk("d")];
        let [o, a, b, u, w] = who.clone();
        let mut d = Device { me: o.clone(), log: OpLog::create(create(&o)).unwrap(), clock: 100 };
        d.make(OpBody::SetRole { who: a, role: Role::Admin });
        d.make(OpBody::SetRole { who: b, role: Role::Admin });
        d.make(OpBody::SetRole { who: w, role: Role::Author });
        d.make(OpBody::SetTrusted { who: u, trusted: true });
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
        let (d, [o, a, ..]) = team();
        assert_eq!(d.log.len(), 5);
        assert_eq!(d.log.heads().len(), 1);
        assert_eq!(d.log.state().staff.len(), 5);
        assert_eq!(d.log.state().role_of(&a), Some(Role::Admin));
        assert_eq!(d.log.state().owner, o);
        assert_eq!(d.log.ordered().count(), 5);
        assert_eq!(d.log.head_time(), 104);

        // Stored operations in any order give the same journal.
        let mut stored: Vec<Op> = d.log.ordered().cloned().collect();
        stored.reverse();
        let rebuilt = OpLog::from_ops(stored).unwrap();
        assert_eq!(rebuilt.state(), d.log.state());
        assert!(OpLog::from_ops(vec![]).is_err());
    }

    #[test]
    fn two_admins_at_once_converge_in_any_delivery_order() {
        let (base, [o, a, b, u, w]) = team();
        let x = pk("e");
        // A removes w, B mutes x, the owner renames; none saw the others.
        let mut da = base.fork(&a);
        let mut db = base.fork(&b);
        let mut d0 = base.fork(&o);
        let op_a = da.make(OpBody::Remove { who: w.clone() });
        let op_b = db.make(OpBody::SetMuted { who: x.clone(), muted: true, until: None });
        let op_o = d0.make(Settings { name: Some("Renamed".into()), ..Settings::default() }.body());
        // Then B, having seen only its own, mutes u.
        let op_b2 = db.make(OpBody::SetMuted { who: u.clone(), muted: true, until: None });

        let all = [op_a.clone(), op_b.clone(), op_o.clone(), op_b2.clone()];
        let mut states = Vec::new();
        for order in permutations(&all) {
            let mut d = base.fork(&u);
            for op in &order {
                d.log.insert(op.clone());
            }
            assert!(d.log.missing().is_empty());
            assert_eq!(d.log.len(), base.log.len() + 4);
            states.push((d.log.state().clone(), d.log.ordered().map(Op::id).collect::<Vec<_>>()));
        }
        let first = states[0].clone();
        assert!(states.iter().all(|s| s == &first), "24 delivery orders, one result");

        let s = &first.0;
        assert!(s.role_of(&w).is_none() && s.staff_of(&x).unwrap().muted, "both admins' actions took effect");
        assert_eq!(s.name, "Renamed");
        assert!(s.staff_of(&u).unwrap().muted);
        // The owner's operation is ordered before the admins'.
        let pos = |op: &Op| first.1.iter().position(|id| id == &op.id()).unwrap();
        assert!(pos(&op_o) < pos(&op_a) && pos(&op_o) < pos(&op_b));
        assert!(pos(&op_b) < pos(&op_b2), "an operation never precedes its parent");

        // Three heads merge into one with the next operation.
        let mut d = base.fork(&o);
        deliver(&mut d, &[&op_a, &op_b, &op_o, &op_b2]);
        assert_eq!(d.log.heads().len(), 3);
        let merge = d.make(OpBody::RotateNow { scope_id: None });
        assert_eq!(merge.parents.len(), 3);
        assert_eq!(d.log.heads(), vec![merge.id()]);
    }

    #[test]
    fn an_operation_of_someone_removed_meanwhile_is_rejected() {
        let (base, [o, a, b, u, _]) = team();
        let mut d0 = base.fork(&o);
        let mut db = base.fork(&b);
        // The owner removes admin B while B, unaware, bans u.
        let op_o = d0.make(OpBody::Remove { who: b.clone() });
        let op_b = db.make(OpBody::Ban { who: u.clone(), reason: None });

        for order in [[&op_o, &op_b], [&op_b, &op_o]] {
            let mut d = base.fork(&a);
            deliver(&mut d, &order);
            assert!(!d.log.state().is_member(&b));
            assert!(!d.log.state().is_banned(&u), "B's ban did not happen");
            assert_eq!(d.log.rejection(&op_b.id()), Some(&Rejection::AuthorNotMember));
            assert!(d.log.contains(&op_b.id()), "it stays in the graph");
        }
        // On B's own device the same: once the owner's operation arrives,
        // B's is undone.
        deliver(&mut db, &[&op_o]);
        assert!(!db.log.state().is_banned(&u));
        assert_eq!(db.log.insert(op_b.clone()), Insert::Duplicate);

        // A rejected operation is no head: the next one here does not name
        // it, though one made elsewhere that does is still taken.
        let mut d = base.fork(&a);
        deliver(&mut d, &[&op_o, &op_b]);
        assert!(!d.log.heads().contains(&op_b.id()));
        let next = d.make(OpBody::SetMuted { who: u.clone(), muted: true, until: None });
        assert!(!next.parents.contains(&op_b.id()));
        let naming = Op::new(CHANNEL, &a, vec![op_b.id()], 200, OpBody::SetTrusted { who: u.clone(), trusted: false });
        assert_eq!(d.log.insert(naming), Insert::Applied);
    }

    #[test]
    fn spam_dated_ahead_takes_no_parent_slot() {
        let (mut d, [_, _, _, u, _]) = team();
        let outsider = pk("e");
        let create_id = d.log.ordered().next().map(Op::id).unwrap();
        for i in 0..20 {
            let spam = Op::new(CHANNEL, &outsider, vec![create_id.clone()], 4_000_000_000 + i, OpBody::SetMuted { who: u.clone(), muted: true, until: None });
            assert!(matches!(d.log.insert(spam), Insert::Rejected(_)));
        }
        assert!(!d.log.may_author(&outsider));
        let next = d.make(OpBody::SetTrusted { who: u.clone(), trusted: false });
        assert!(next.parents.iter().all(|p| d.log.rejection(p).is_none()), "only applied operations are parents");
    }

    #[test]
    fn operations_wait_for_their_parents() {
        let (base, [_, a, _, u, w]) = team();
        let mut da = base.fork(&a);
        let first = da.make(OpBody::SetMuted { who: u.clone(), muted: true, until: None });
        let second = da.make(OpBody::SetMuted { who: w.clone(), muted: true, until: None });
        let third = da.make(OpBody::SetMuted { who: u.clone(), muted: false, until: None });

        let mut d = base.fork(&w);
        assert_eq!(d.log.insert(third.clone()), Insert::Waiting { missing: vec![second.id()] });
        assert_eq!(d.log.insert(second.clone()), Insert::Waiting { missing: vec![first.id()] });
        assert_eq!(d.log.missing(), vec![first.id()]);
        assert_eq!(d.log.state(), base.log.state(), "nothing applies before its parents");
        assert_eq!(d.log.insert(third.clone()), Insert::Duplicate);

        assert_eq!(d.log.insert(first), Insert::Applied);
        assert!(d.log.missing().is_empty());
        assert_eq!(d.log.state(), da.log.state());
        assert!(!d.log.state().staff_of(&u).unwrap().muted && d.log.state().staff_of(&w).unwrap().muted);
    }

    #[test]
    fn refusals() {
        let (d, [o, a, ..]) = team();
        let mut d = d;
        let mut foreign = d.log.next(&a, 1, OpBody::RotateNow { scope_id: None });
        foreign.community_id = "ff".repeat(32);
        assert_eq!(d.log.insert(foreign), Insert::Refused(Rejection::WrongChannel));
        let mut second = create(&a);
        second.created_at = 5;
        assert_eq!(d.log.insert(second), Insert::Refused(Rejection::AlreadyCreated));
        assert_eq!(d.log.insert(create(&o)), Insert::Duplicate);
        let orphan = Op::new(CHANNEL, &a, vec![], 1, OpBody::RotateNow { scope_id: None });
        assert!(matches!(d.log.insert(orphan), Insert::Refused(Rejection::Invalid { .. })));
        let wide = Op::new(CHANNEL, &a, (0..=MAX_PARENTS).map(|i| OpId(format!("{i:02}"))).collect(), 1, OpBody::RotateNow { scope_id: None });
        assert!(matches!(d.log.insert(wide.clone()), Insert::Refused(Rejection::Invalid { .. })), "17 parents");
        assert!(!d.log.contains(&wide.id()) && d.log.missing().is_empty(), "not even kept aside");

        // `check` tells a command what would happen, without changing anything.
        let before = d.log.state().clone();
        let bad = d.log.next(&a, 1, OpBody::Disband);
        assert_eq!(d.log.check(&bad), Err(Rejection::NotPermitted));
        let good = d.log.next(&o, 1, OpBody::Disband);
        assert_eq!(d.log.check(&good), Ok(()));
        assert_eq!(d.log.state(), &before);
        assert_eq!(d.log.community_id(), CHANNEL);
        let _ = d.make(OpBody::RotateNow { scope_id: None });
    }

    /// Twenty operations made beside each other: the next one names the
    /// sixteen newest heads, the rest stay heads and are named after.
    #[test]
    fn parents_are_capped_at_sixteen_newest_heads() {
        let (base, [o, a, ..]) = team();
        let mut d = base.fork(&o);
        let mut made = Vec::new();
        for i in 0..20u8 {
            let mut dev = base.fork(&a);
            dev.clock = 1_000 + i64::from(i);
            made.push(dev.make(OpBody::SetLevels { who: pk(&format!("{}", i % 10)), levels: 1 }));
        }
        for op in &made {
            d.log.insert(op.clone());
        }
        assert_eq!(d.log.heads().len(), 20);
        let next = d.make(OpBody::RotateNow { scope_id: None });
        assert_eq!(next.parents.len(), MAX_PARENTS);
        let mut newest: Vec<OpId> = made[4..].iter().map(Op::id).collect();
        newest.sort();
        assert_eq!(next.parents, newest, "the newest by their clocks, sorted");
        // The four oldest stay heads beside the new one, and the next
        // operation gathers them.
        assert_eq!(d.log.heads().len(), 5);
        let gather = d.make(OpBody::RotateNow { scope_id: None });
        assert_eq!(gather.parents.len(), 5);
        assert_eq!(d.log.heads(), vec![gather.id()]);
        // Everyone else arrives at the same journal.
        let mut other = base.fork(&a);
        for op in made.iter().rev().chain([&next, &gather]) {
            other.log.insert(op.clone());
        }
        assert_eq!(other.log.state(), d.log.state());
    }

    #[test]
    fn a_handover_is_taken_by_the_keeper_of_the_new_owner() {
        let (base, [o, a, b, ..]) = team();
        let mut d0 = base.fork(&o);
        let transfer = d0.make(OpBody::TransferOwnership { to: a.clone() });
        let mut da = base.fork(&a);
        deliver(&mut da, &[&transfer]);
        assert_eq!(da.log.state().role_of(&a), Some(Role::Admin), "not before the keeper");
        assert_eq!(da.log.check(&da.log.next(&a, 1, OpBody::Remove { who: b.clone() })), Err(Rejection::NotPermitted), "an admin cannot remove an admin");
        let keeper = da.make(OpBody::Keeper { pubkey: pk("f"), serial: 1, from_epoch: 7, inbox: None });
        assert_eq!(da.log.state().role_of(&a), Some(Role::Owner));
        let removal = da.make(OpBody::Remove { who: b.clone() });
        // The old owner, beside the keeper, tries to name a keeper too: the
        // handover went first (its receiver went on from it), so the old
        // owner is an admin there and the keeper is refused.
        let late = d0.make(OpBody::Keeper { pubkey: pk("e"), serial: 1, from_epoch: 8, inbox: None });
        for order in permutations(&[transfer.clone(), keeper.clone(), removal.clone(), late.clone()]) {
            let mut d = base.fork(&b);
            for op in &order {
                d.log.insert(op.clone());
            }
            let s = d.log.state();
            assert_eq!(s.owner, a);
            assert_eq!(s.role_of(&o), Some(Role::Admin));
            assert!(!s.is_member(&b));
            assert_eq!(s.keeper().unwrap().pubkey, pk("f"));
            assert_eq!(d.log.rejection(&late.id()), Some(&Rejection::NotPermitted));
        }
    }

    #[test]
    fn two_admins_removing_each_other_converge() {
        let (base, [o, a, b, ..]) = team();
        let heads = base.log.heads();
        // Each operation waits for the other: neither may go first by the
        // rule; the one held back by the weaker goes first.
        let by_a = Op::new(CHANNEL, &a, heads.clone(), 300, OpBody::Remove { who: b.clone() });
        let by_b = Op::new(CHANNEL, &b, heads.clone(), 300, OpBody::Remove { who: a.clone() });
        let mut d0 = base.fork(&o);
        let demote = d0.make(OpBody::SetRole { who: a.clone(), role: Role::Member });

        let all = [by_a, by_b, demote];
        let mut results = Vec::new();
        for order in permutations(&all) {
            let mut d = base.fork(&o);
            for op in &order {
                d.log.insert(op.clone());
            }
            results.push((d.log.state().clone(), d.log.ordered().map(Op::id).collect::<Vec<_>>()));
        }
        assert!(results.iter().all(|r| r == &results[0]), "six delivery orders, one result");
        let s = &results[0].0;
        // The owner's demotion goes first; A, a reader by then, is removed
        // by B, and A's own removal of B comes from someone without a role.
        assert!(s.role_of(&a).is_none() && s.role_of(&b) == Some(Role::Admin));
        let mut d = base.fork(&o);
        deliver(&mut d, &[&all[0], &all[1]]);
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
            let (base, [o, a, b, u, w]) = team();
            let people = [o.clone(), a.clone(), b.clone(), u.clone(), w.clone(), pk("e"), pk("f")];
            let mut devices: Vec<Device> = [&o, &a, &b].into_iter().map(|p| base.fork(p)).collect();
            let mut made: Vec<Op> = Vec::new();
            let mut keeper_serial = 0u32;

            for _ in 0..18 {
                let d = rng.next(devices.len());
                // Sometimes a device hears about something first.
                if !made.is_empty() && rng.next(3) == 0 {
                    let heard = made[rng.next(made.len())].clone();
                    devices[d].log.insert(heard);
                }
                let target = people[rng.next(people.len())].clone();
                let body = match rng.next(11) {
                    0 => OpBody::Remove { who: target },
                    1 => OpBody::SetMuted { who: target, muted: rng.next(2) == 0, until: None },
                    2 => OpBody::SetRole { who: target, role: [Role::Member, Role::Author, Role::Moderator, Role::Admin][rng.next(4)] },
                    3 => OpBody::Ban { who: target, reason: None },
                    4 => Settings { name: Some(format!("n{}", rng.next(9))), ..Settings::default() }.body(),
                    5 => OpBody::SetMuted { who: target, muted: true, until: Some(1_000 + rng.next(500) as i64) },
                    6 => OpBody::SetTrusted { who: target, trusted: rng.next(2) == 0 },
                    7 => Settings {
                        post_mode: Some([PostMode::All, PostMode::Authors][rng.next(2)]),
                        premod: Some([Premod::Off, Premod::Newcomers, Premod::All][rng.next(3)]),
                        ..Settings::default()
                    }
                    .body(),
                    8 => OpBody::Unban { who: target, reason: None },
                    9 => {
                        keeper_serial += 1;
                        OpBody::Keeper { pubkey: target, serial: keeper_serial, from_epoch: 10 * keeper_serial, inbox: None }
                    }
                    _ => OpBody::Pin { post_id: format!("{:064x}", rng.next(4)), pinned: rng.next(2) == 0 },
                };
                let dev = &mut devices[d];
                dev.clock += 1;
                let op = dev.log.next(&dev.me, dev.clock, body);
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
                results.push((d.log.state().clone(), d.log.ordered().map(Op::id).collect::<Vec<_>>()));
            }
            assert!(results.iter().all(|r| r == &results[0]), "seed {seed}: devices disagree");
            // Whatever happened, there is exactly one owner and nobody banned is staff.
            let s = &results[0].0;
            assert_eq!(s.staff.values().filter(|m| m.role == Role::Owner).count(), 1, "seed {seed}");
            assert_eq!(s.role_of(&s.owner), Some(Role::Owner), "seed {seed}");
            assert!(s.banned.iter().all(|k| !s.staff.contains_key(k)), "seed {seed}: a banned member");
            // The keeper series run one by one.
            assert!(s.keepers.iter().enumerate().all(|(i, k)| k.serial as usize == i), "seed {seed}");
        }
    }

    #[test]
    fn what_an_operation_knew() {
        let (base, [o, ..]) = team();
        let mut d = base.fork(&o);
        let first = d.make(OpBody::RotateNow { scope_id: None });
        let second = d.make(OpBody::RotateNow { scope_id: None });
        let known = knowledge(&d.log.ops, &second.id());
        assert!(known.contains(&first.id()) && known.contains(&second.id()));
        assert_eq!(known.len(), d.log.len());
    }
}
