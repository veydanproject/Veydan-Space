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
//! 2. among operations that are ready, the one whose author has the
//!    higher role (in the state reached so far) goes first;
//! 3. with equal roles, the one with the smaller id.
//!
//! An operation that lost its ground on the way (its author was removed
//! by one that went first) is rejected and changes nothing, but stays in
//! the graph so that later operations can still refer to it.
//!
//! The order is recomputed from the first operation whenever the graph
//! changes. Groups change rarely; correctness is worth more here than the
//! microseconds.

use crate::op::{GroupKind, KeyId, Op, OpBody, OpId};
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
        for op in all {
            log.insert(op);
        }
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
fn linearize(ops: &HashMap<OpId, Op>, stale: &HashSet<OpId>) -> Option<(Vec<OpId>, HashMap<OpId, Rejection>, GroupState)> {
    let (root_id, root) = ops.iter().find(|(_, o)| matches!(o.body, OpBody::Create { .. }))?;
    let mut state = GroupState::genesis(root).ok()?;

    let mut children: HashMap<OpId, Vec<OpId>> = HashMap::new();
    let mut blocked: HashMap<OpId, usize> = HashMap::new();
    for (id, op) in ops {
        blocked.insert(id.clone(), op.parents.len());
        for p in &op.parents {
            children.entry(p.clone()).or_default().push(id.clone());
        }
    }

    let mut order = vec![root_id.clone()];
    let mut rejected = HashMap::new();
    let mut ready: BTreeSet<OpId> = BTreeSet::new();
    let mut done = root_id.clone();
    loop {
        for child in children.get(&done).into_iter().flatten() {
            if let Some(n) = blocked.get_mut(child) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    ready.insert(child.clone());
                }
            }
        }
        // Highest role of the author in the state reached so far, then
        // the smallest id.
        let rank = |id: &OpId| ops.get(id).and_then(|o| state.role_of(&o.author)).map(|r| r.rank()).unwrap_or(0);
        let Some(pick) = ready.iter().max_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| b.cmp(a))).cloned() else { break };
        ready.remove(&pick);
        if let Some(op) = ops.get(&pick) {
            if stale.contains(&pick) {
                rejected.insert(pick.clone(), Rejection::StaleLink);
            } else if let Err(r) = state.apply(op) {
                rejected.insert(pick.clone(), r);
            }
        }
        order.push(pick.clone());
        done = pick;
    }
    Some((order, rejected, state))
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

/// The current key is the one brought by the last applied operation that
/// brings a key. Whoever made it knew a certain part of the log. Anyone
/// who stopped being a member outside that part still has the key; anyone
/// who joined outside that part does not have it yet.
fn key_status(ops: &HashMap<OpId, Op>, order: &[OpId], rejected: &HashMap<OpId, Rejection>, state: &mut GroupState) -> KeyStatus {
    if state.kind == GroupKind::Public || state.disbanded {
        state.key_stale = false;
        return KeyStatus::Good;
    }
    let applied: Vec<&Op> = order.iter().filter(|id| !rejected.contains_key(*id)).filter_map(|id| ops.get(id)).collect();
    let Some(key_op) = applied.iter().rev().find(|o| o.key.is_some()) else { return KeyStatus::Good };
    let key_id = key_op.id();
    let known = knowledge(ops, &key_id);
    let current: Option<&KeyId> = key_op.key.as_ref();
    debug_assert_eq!(current, state.current_key.as_ref());

    // Replay membership to learn who left and who joined outside `known`.
    let mut replay: Option<GroupState> = None;
    let mut departed: BTreeMap<String, bool> = BTreeMap::new();
    let mut arrived: BTreeSet<String> = BTreeSet::new();
    for op in &applied {
        let before: BTreeSet<String> = replay.as_ref().map(|s| s.members.keys().cloned().collect()).unwrap_or_default();
        match replay.as_mut() {
            None => replay = GroupState::genesis(op).ok(),
            Some(s) => {
                let _ = s.apply(op);
            }
        }
        let after: BTreeSet<String> = replay.as_ref().map(|s| s.members.keys().cloned().collect()).unwrap_or_default();
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

    if !departed.is_empty() || state.key_stale {
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
        let op_o = d0.make(OpBody::EditSettings { name: Some("Renamed".into()), about: None, picture: None, history_for_new: None, call_node: None, call_node_key: None }, None);
        // Then B, having seen only its own, mutes w.
        let op_b2 = db.make(OpBody::SetMuted { who: w.clone(), muted: true }, None);

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
        let next = d.make(OpBody::SetMuted { who: u.clone(), muted: true }, None);
        assert!(next.parents.contains(&op_b.id()));
    }

    #[test]
    fn operations_wait_for_their_parents() {
        let (base, [_, a, _, u, w]) = team(GroupKind::Private, true);
        let mut da = base.fork(&a);
        let first = da.make(OpBody::SetMuted { who: u.clone(), muted: true }, None);
        let second = da.make(OpBody::SetMuted { who: w.clone(), muted: true }, None);
        let third = da.make(OpBody::SetMuted { who: u.clone(), muted: false }, None);

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
                let body = match rng.next(7) {
                    0 => OpBody::Admit { who: target },
                    1 => OpBody::Remove { who: target },
                    2 => OpBody::SetMuted { who: target, muted: rng.next(2) == 0 },
                    3 => OpBody::SetRole { who: target, role: [Role::Member, Role::Moderator, Role::Admin][rng.next(3)] },
                    4 => OpBody::Ban { who: target },
                    5 => OpBody::EditSettings { name: Some(format!("n{}", rng.next(9))), about: None, picture: None, history_for_new: None, call_node: None, call_node_key: None },
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
}
