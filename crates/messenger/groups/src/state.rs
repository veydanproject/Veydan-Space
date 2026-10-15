// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The state of a group and the effect of one operation on it.
//!
//! `apply` either changes the state or says why not. It never looks at
//! anything but the state and the operation, so two devices that apply
//! the same operations in the same order hold the same state.

use crate::op::{GroupKind, KeyId, Op, OpBody, PostMode, Premod, Schedule, OP_VERSION};
use crate::schedule::{self, Status};
use crate::roles::{permits, Action, Role};
use messenger_core::PubKey;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_NAME_CHARS: usize = 100;
pub const MAX_ABOUT_CHARS: usize = 1000;
pub const MAX_PICTURE_CHARS: usize = 500;
pub const MAX_CALL_NODE_CHARS: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub role: Role,
    /// May read, may not write.
    pub muted: bool,
    /// When the mute lifts by itself; none for one that stays until
    /// lifted. Means nothing while `muted` is false.
    #[serde(default)]
    pub muted_until: Option<i64>,
    /// Their messages are never held for moderation.
    #[serde(default)]
    pub trusted: bool,
    pub joined_at: i64,
    /// Who admitted them; themselves for the owner and for public joins.
    pub added_by: PubKey,
    /// Where in the applied order the operation that let them in stands
    /// (the `Create` is 0): who came before whom, by the log and not by
    /// anybody's clock.
    #[serde(default)]
    pub joined_pos: u64,
}

impl Member {
    /// Muted at `now`: a mute with an end lifts by itself.
    pub fn muted_at(&self, now: i64) -> bool {
        self.muted && self.muted_until.is_none_or(|until| now < until)
    }
}

/// A key that is no longer the group's, and since when: the time of the
/// operation that brought the next one, by the clock of its author.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplacedKey {
    pub key: KeyId,
    pub at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupState {
    pub group_id: String,
    pub kind: GroupKind,
    pub name: String,
    pub about: String,
    pub picture: String,
    /// Private groups only; a public group always shows its history.
    pub history_for_new: bool,
    pub owner: PubKey,
    /// Hex public key → member.
    pub members: BTreeMap<String, Member>,
    pub banned: BTreeSet<String>,
    /// Public groups: which link secret admits newcomers.
    pub link_epoch: u32,
    pub disbanded: bool,
    /// Operations applied so far.
    pub version: u64,
    pub current_key: Option<KeyId>,
    /// The key that was current before this one.
    pub replaced_key: Option<ReplacedKey>,
    /// Every key the group had, oldest first.
    pub keys: Vec<KeyId>,
    /// Someone who knows the current key is no longer a member.
    pub key_stale: bool,
    /// The call node pinned to the group (`address:port#id`), empty for
    /// none: its group calls go through it (class `group`).
    #[serde(default)]
    pub call_node: String,
    /// The access key of that node, empty for a public one.
    #[serde(default)]
    pub call_node_key: String,
    /// Who may post.
    #[serde(default)]
    pub post_mode: PostMode,
    /// When members may post; none for always.
    #[serde(default)]
    pub schedule: Option<Schedule>,
    /// Whose messages wait for a moderator.
    #[serde(default)]
    pub premod: Premod,
    /// Where in the applied order the operation that turned `premod` on
    /// stands: whoever joined after it is a newcomer. None while it is off.
    #[serde(default)]
    pub premod_since: Option<u64>,
}

/// Why someone may not post right now (`GroupState::can_post_at`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Closed {
    /// Not a member, or the group is disbanded.
    NotMember,
    Banned,
    /// Muted; `until` says when it lifts, none for until somebody lifts it.
    Muted { until: Option<i64> },
    /// Only moderators and above write now.
    ManagersOnly,
    /// Outside the hours of the schedule; the moment it opens.
    Schedule { opens_at: i64 },
}

/// The rules of posting that are not about one person: who may post, and
/// when. Moderators and above are never held by them.
pub(crate) fn gate(role: Role, post_mode: PostMode, schedule: Option<&Schedule>, now: i64) -> Result<(), Closed> {
    if role >= Role::Moderator {
        return Ok(());
    }
    if post_mode == PostMode::Managers {
        return Err(Closed::ManagersOnly);
    }
    if let Some(s) = schedule {
        if let (Status::Closed { opens_at }, _) = schedule::status_or_utc(s, now) {
            return Err(Closed::Schedule { opens_at });
        }
    }
    Ok(())
}

/// Why an operation does not apply.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Rejection {
    UnsupportedVersion,
    WrongGroup,
    NotCreated,
    AlreadyCreated,
    Disbanded,
    AuthorNotMember,
    AuthorBanned,
    NotPermitted,
    TargetNotMember,
    AlreadyMember,
    TargetBanned,
    NotBanned,
    WrongKind,
    KeyRequired,
    KeyNotAllowed,
    /// A join without the proof, or by a link that is no longer current.
    StaleLink,
    Invalid { what: String },
}

fn invalid(what: &str) -> Rejection {
    Rejection::Invalid { what: what.into() }
}

/// A pinned call node is `address:port#id` (the id 64 hex), or nothing;
/// its key is short. The groups do not know the nodes: the reference is
/// checked for its shape here and parsed by whoever makes the call.
fn check_call_node(node: Option<&str>, key: Option<&str>) -> Result<(), Rejection> {
    if let Some(n) = node {
        let n = n.trim();
        if !n.is_empty() {
            let Some((addr, id)) = n.rsplit_once('#') else { return Err(invalid("call_node")) };
            let well_formed = addr.rsplit_once(':').is_some_and(|(host, port)| !host.is_empty() && port.parse::<u16>().is_ok())
                && id.len() == 64
                && id.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
            if !well_formed || n.chars().count() > MAX_CALL_NODE_CHARS {
                return Err(invalid("call_node"));
            }
        }
    }
    if key.is_some_and(|k| k.chars().count() > MAX_CALL_NODE_CHARS || k.chars().any(char::is_control)) {
        return Err(invalid("call_node_key"));
    }
    Ok(())
}

fn check_text(name: Option<&str>, about: Option<&str>, picture: Option<&str>) -> Result<(), Rejection> {
    if let Some(n) = name {
        let n = n.trim();
        if n.is_empty() || n.chars().count() > MAX_NAME_CHARS || n.chars().any(char::is_control) {
            return Err(invalid("name"));
        }
    }
    if about.is_some_and(|a| a.chars().count() > MAX_ABOUT_CHARS) {
        return Err(invalid("about"));
    }
    if let Some(p) = picture {
        if !p.is_empty() && (!p.starts_with("https://") || p.chars().count() > MAX_PICTURE_CHARS) {
            return Err(invalid("picture"));
        }
    }
    Ok(())
}

impl GroupState {
    /// The state right after `Create`.
    pub fn genesis(op: &Op) -> Result<Self, Rejection> {
        if op.v == 0 || op.v > OP_VERSION {
            return Err(Rejection::UnsupportedVersion);
        }
        if op.v < op.body.version() {
            return Err(invalid("version"));
        }
        let OpBody::Create { kind, name, about, picture, history_for_new } = &op.body else {
            return Err(Rejection::NotCreated);
        };
        if !op.parents.is_empty() {
            return Err(invalid("create has parents"));
        }
        if op.group_id.len() != 64 || !op.group_id.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
            return Err(invalid("group id"));
        }
        check_text(Some(name), Some(about), Some(picture))?;
        let key = op.key.clone().ok_or(Rejection::KeyRequired)?;
        let mut members = BTreeMap::new();
        members.insert(
            op.author.as_hex().to_string(),
            Member { role: Role::Owner, muted: false, muted_until: None, trusted: false, joined_at: op.created_at, added_by: op.author.clone(), joined_pos: 0 },
        );
        Ok(Self {
            group_id: op.group_id.clone(),
            kind: *kind,
            name: name.trim().to_string(),
            about: about.clone(),
            picture: picture.clone(),
            history_for_new: *kind == GroupKind::Public || *history_for_new,
            owner: op.author.clone(),
            members,
            banned: BTreeSet::new(),
            link_epoch: 0,
            disbanded: false,
            version: 1,
            current_key: Some(key.clone()),
            replaced_key: None,
            keys: vec![key],
            key_stale: false,
            call_node: String::new(),
            call_node_key: String::new(),
            post_mode: PostMode::All,
            schedule: None,
            premod: Premod::Off,
            premod_since: None,
        })
    }

    pub fn member(&self, who: &PubKey) -> Option<&Member> {
        self.members.get(who.as_hex())
    }

    pub fn role_of(&self, who: &PubKey) -> Option<Role> {
        self.member(who).map(|m| m.role)
    }

    pub fn is_member(&self, who: &PubKey) -> bool {
        self.members.contains_key(who.as_hex())
    }

    pub fn is_banned(&self, who: &PubKey) -> bool {
        self.banned.contains(who.as_hex())
    }

    /// May this person write into the group at `now`, and if not, why
    /// not? A member who is not banned, whose mute (if any) has lifted,
    /// and whom the posting rules of the group let through (`gate`).
    pub fn can_post_at(&self, who: &PubKey, now: i64) -> Result<(), Closed> {
        if self.disbanded {
            return Err(Closed::NotMember);
        }
        if self.is_banned(who) {
            return Err(Closed::Banned);
        }
        let Some(m) = self.member(who) else { return Err(Closed::NotMember) };
        if m.muted_at(now) {
            return Err(Closed::Muted { until: m.muted_until });
        }
        gate(m.role, self.post_mode, self.schedule.as_ref(), now)
    }

    /// Do this member's messages wait for a moderator? Never a moderator's
    /// or a trusted member's; under `Premod::Newcomers` only those who
    /// came in after the rule was set, by the order of the log.
    pub fn subject_to_premod(&self, who: &PubKey) -> bool {
        let Some(m) = self.member(who) else { return false };
        if m.role >= Role::Moderator || m.trusted {
            return false;
        }
        match self.premod {
            Premod::Off => false,
            Premod::All => true,
            Premod::Newcomers => self.premod_since.is_some_and(|since| m.joined_pos > since),
        }
    }

    /// The schedule names a zone this build does not know: it is read in
    /// UTC until the app is updated.
    pub fn schedule_tz_unknown(&self) -> bool {
        self.schedule.as_ref().is_some_and(|s| schedule::status(s, 0).is_err())
    }

    /// May `actor` remove a message written by `author`? Authors remove
    /// their own; moderators and above remove those of lower roles. A
    /// message of someone who is no longer a member counts as a member's.
    pub fn can_delete_message(&self, actor: &PubKey, author: &PubKey) -> bool {
        if self.disbanded {
            return false;
        }
        let Some(role) = self.role_of(actor) else { return false };
        if actor == author {
            return true;
        }
        permits(role, Action::DeleteMessageOf(self.role_of(author).unwrap_or(Role::Member)))
    }

    /// Whether messages of this author are shown. Banned authors are
    /// hidden; in a private group so are people who are not members.
    pub fn shows_messages_of(&self, author: &PubKey) -> bool {
        if self.is_banned(author) {
            return false;
        }
        self.kind == GroupKind::Public || self.is_member(author)
    }

    /// Everyone who must hold the current key.
    pub fn key_holders(&self) -> Vec<PubKey> {
        self.members.keys().filter_map(|k| PubKey::parse(k)).collect()
    }

    pub fn managers(&self) -> Vec<PubKey> {
        self.members.iter().filter(|(_, m)| m.role.is_manager()).filter_map(|(k, _)| PubKey::parse(k)).collect()
    }

    /// Must an operation with this body, made by a manager, bring a new
    /// key? In a private group: whenever someone stops being a member, and
    /// when someone joins a group that hides its history.
    pub fn requires_key(&self, body: &OpBody) -> bool {
        if self.kind == GroupKind::Public {
            return matches!(body, OpBody::RotateKey | OpBody::RotateLink { .. });
        }
        match body {
            OpBody::Remove { .. } | OpBody::RotateKey => true,
            OpBody::Ban { who } => self.is_member(who),
            OpBody::Admit { .. } => !self.history_for_new,
            _ => false,
        }
    }

    fn take_key(&mut self, op: &Op) {
        if let Some(k) = &op.key {
            if !self.keys.contains(k) {
                self.keys.push(k.clone());
            }
            if let Some(before) = self.current_key.replace(k.clone()) {
                if &before != k {
                    self.replaced_key = Some(ReplacedKey { key: before, at: op.created_at });
                }
            }
            self.key_stale = false;
        }
    }

    /// Apply one operation. On `Err` the state is unchanged.
    pub fn apply(&mut self, op: &Op) -> Result<(), Rejection> {
        if op.v == 0 || op.v > OP_VERSION {
            return Err(Rejection::UnsupportedVersion);
        }
        // A body of the new shape under the old label: a build before the
        // shape would read it as the old one (a mute with an end as one
        // without) and part ways with this one. The label is what keeps
        // every device at the same reading, so it has to hold what it says.
        if op.v < op.body.version() {
            return Err(invalid("version"));
        }
        if op.group_id != self.group_id {
            return Err(Rejection::WrongGroup);
        }
        if matches!(op.body, OpBody::Create { .. }) {
            return Err(Rejection::AlreadyCreated);
        }
        if self.disbanded {
            return Err(Rejection::Disbanded);
        }
        if op.parents.is_empty() {
            return Err(invalid("no parents"));
        }
        if self.is_banned(&op.author) {
            return Err(Rejection::AuthorBanned);
        }

        // The one thing a non-member may do: join a public group.
        if op.body == OpBody::Join {
            if self.kind != GroupKind::Public {
                return Err(Rejection::WrongKind);
            }
            if self.is_member(&op.author) {
                return Err(Rejection::AlreadyMember);
            }
            match &op.proof {
                Some(p) if p.epoch == self.link_epoch && !p.mac.is_empty() => {}
                _ => return Err(Rejection::StaleLink),
            }
            if op.key.is_some() {
                return Err(Rejection::KeyNotAllowed);
            }
            self.members.insert(
                op.author.as_hex().to_string(),
                Member { role: Role::Member, muted: false, muted_until: None, trusted: false, joined_at: op.created_at, added_by: op.author.clone(), joined_pos: self.version },
            );
            self.version += 1;
            return Ok(());
        }

        if op.proof.is_some() {
            return Err(invalid("proof"));
        }
        let actor = self.role_of(&op.author).ok_or(Rejection::AuthorNotMember)?;
        // Only managers choose keys: whoever brings a key knows it.
        if op.key.is_some() && !actor.is_manager() {
            return Err(Rejection::KeyNotAllowed);
        }
        if self.requires_key(&op.body) && op.key.is_none() {
            return Err(Rejection::KeyRequired);
        }
        let allow = |action: Action| if permits(actor, action) { Ok(()) } else { Err(Rejection::NotPermitted) };

        match &op.body {
            OpBody::Create { .. } | OpBody::Join => unreachable!("handled above"),
            OpBody::Admit { who } => {
                allow(Action::Admit)?;
                if self.is_banned(who) {
                    return Err(Rejection::TargetBanned);
                }
                if self.is_member(who) {
                    return Err(Rejection::AlreadyMember);
                }
                self.members.insert(
                    who.as_hex().to_string(),
                    Member { role: Role::Member, muted: false, muted_until: None, trusted: false, joined_at: op.created_at, added_by: op.author.clone(), joined_pos: self.version },
                );
            }
            OpBody::Leave => {
                allow(Action::Leave)?;
                self.members.remove(op.author.as_hex());
                // The one who left still knows the key and cannot be the
                // one to choose the next: a manager has to.
                if self.kind == GroupKind::Private {
                    self.key_stale = true;
                }
            }
            OpBody::Remove { who } => {
                let target = self.role_of(who).ok_or(Rejection::TargetNotMember)?;
                allow(Action::Remove(target))?;
                self.members.remove(who.as_hex());
            }
            OpBody::Ban { who } => {
                if who == &op.author {
                    return Err(invalid("banning oneself"));
                }
                if self.is_banned(who) {
                    return Err(Rejection::TargetBanned);
                }
                allow(Action::Ban(self.role_of(who)))?;
                self.members.remove(who.as_hex());
                self.banned.insert(who.as_hex().to_string());
            }
            OpBody::Unban { who } => {
                allow(Action::Unban)?;
                if !self.banned.remove(who.as_hex()) {
                    return Err(Rejection::NotBanned);
                }
            }
            OpBody::SetRole { who, role } => {
                let from = self.role_of(who).ok_or(Rejection::TargetNotMember)?;
                allow(Action::SetRole { from, to: *role })?;
                if let Some(m) = self.members.get_mut(who.as_hex()) {
                    m.role = *role;
                }
            }
            OpBody::SetMuted { who, muted, until } => {
                let target = self.role_of(who).ok_or(Rejection::TargetNotMember)?;
                allow(Action::Mute(target))?;
                // An end belongs to a mute, and lies ahead of it: one that
                // ended before it was made would mean nothing anywhere.
                if until.is_some_and(|u| !*muted || u <= op.created_at) {
                    return Err(invalid("until"));
                }
                if let Some(m) = self.members.get_mut(who.as_hex()) {
                    m.muted = *muted;
                    m.muted_until = *until;
                }
            }
            OpBody::SetTrusted { who, trusted } => {
                let target = self.role_of(who).ok_or(Rejection::TargetNotMember)?;
                allow(Action::Trust(target))?;
                if let Some(m) = self.members.get_mut(who.as_hex()) {
                    m.trusted = *trusted;
                }
            }
            OpBody::EditSettings { name, about, picture, history_for_new, call_node, call_node_key, post_mode, post_schedule, premod } => {
                allow(Action::EditSettings)?;
                check_text(name.as_deref(), about.as_deref(), picture.as_deref())?;
                check_call_node(call_node.as_deref(), call_node_key.as_deref())?;
                if let Some(s) = post_schedule {
                    s.check().map_err(|_| invalid("post_schedule"))?;
                }
                if history_for_new.is_some() && self.kind == GroupKind::Public {
                    return Err(Rejection::WrongKind);
                }
                if let Some(p) = post_mode {
                    self.post_mode = *p;
                }
                if let Some(s) = post_schedule {
                    self.schedule = if s.is_empty() { None } else { Some(s.clone()) };
                }
                if let Some(p) = premod {
                    // Who counts as a newcomer is fixed when the rule is
                    // turned on and stays while it is on, whichever of its
                    // two shapes it takes; turned off, the line is forgotten.
                    match (self.premod, *p) {
                        (_, Premod::Off) => self.premod_since = None,
                        (Premod::Off, _) => self.premod_since = Some(self.version),
                        _ => {}
                    }
                    self.premod = *p;
                }
                if let Some(n) = name {
                    self.name = n.trim().to_string();
                }
                if let Some(a) = about {
                    self.about = a.clone();
                }
                if let Some(p) = picture {
                    self.picture = p.clone();
                }
                if let Some(h) = history_for_new {
                    self.history_for_new = *h;
                }
                if let Some(n) = call_node {
                    self.call_node = n.trim().to_string();
                }
                if let Some(k) = call_node_key {
                    self.call_node_key = k.clone();
                }
            }
            OpBody::TransferOwnership { to } => {
                allow(Action::TransferOwnership)?;
                if to == &op.author {
                    return Err(invalid("transfer to oneself"));
                }
                if !self.is_member(to) {
                    return Err(Rejection::TargetNotMember);
                }
                if let Some(m) = self.members.get_mut(op.author.as_hex()) {
                    m.role = Role::Admin;
                }
                if let Some(m) = self.members.get_mut(to.as_hex()) {
                    m.role = Role::Owner;
                    m.muted = false;
                    m.muted_until = None;
                }
                self.owner = to.clone();
            }
            OpBody::RotateKey => allow(Action::RotateKey)?,
            OpBody::RotateLink { link_epoch } => {
                allow(Action::RotateLink)?;
                if self.kind != GroupKind::Public {
                    return Err(Rejection::WrongKind);
                }
                if *link_epoch <= self.link_epoch {
                    return Err(invalid("link epoch must grow"));
                }
                self.link_epoch = *link_epoch;
            }
            OpBody::Disband => {
                allow(Action::Disband)?;
                self.disbanded = true;
            }
        }
        // A key on a `Leave` is ignored, not refused: whoever leaves knows it
        // and cannot be the one to choose the next, and refusing the whole
        // `Leave` would split the members between versions of the client.
        if !matches!(op.body, OpBody::Leave) {
            self.take_key(op);
        }
        self.version += 1;
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::op::tests::Settings;
    use crate::op::OpId;

    pub(crate) const GROUP: &str = "00000000000000000000000000000000000000000000000000000000000000aa";

    pub(crate) fn pk(c: &str) -> PubKey {
        PubKey::parse(&c.repeat(64)[..64]).unwrap()
    }

    pub(crate) fn key(n: u8) -> KeyId {
        KeyId::of(&[n; 32])
    }

    pub(crate) fn create(kind: GroupKind, owner: &PubKey, history: bool) -> Op {
        Op::new(GROUP, owner, vec![], 100, OpBody::Create { kind, name: " Team ".into(), about: String::new(), picture: String::new(), history_for_new: history })
            .with_key(key(1))
    }

    /// An operation on top of "something": parents do not matter to `apply`.
    pub(crate) fn op(author: &PubKey, body: OpBody) -> Op {
        let join = body == OpBody::Join;
        let op = Op::new(GROUP, author, vec![OpId("p".into())], 200, body);
        if join {
            op.with_proof(crate::op::JoinProof { epoch: 0, mac: "m".into() })
        } else {
            op
        }
    }

    /// Owner `o`, admin `a`, moderator `m`, member `u`.
    fn team(kind: GroupKind, history: bool) -> GroupState {
        let (o, a, m, u) = (pk("0"), pk("a"), pk("b"), pk("c"));
        let mut s = GroupState::genesis(&create(kind, &o, history)).unwrap();
        for (i, who) in [&a, &m, &u].into_iter().enumerate() {
            let mut admit = op(&o, OpBody::Admit { who: who.clone() });
            if s.requires_key(&admit.body) {
                admit = admit.with_key(key(10 + i as u8));
            }
            s.apply(&admit).unwrap();
        }
        s.apply(&op(&o, OpBody::SetRole { who: a, role: Role::Admin })).unwrap();
        s.apply(&op(&o, OpBody::SetRole { who: m, role: Role::Moderator })).unwrap();
        s
    }

    #[test]
    fn genesis() {
        let o = pk("0");
        let s = GroupState::genesis(&create(GroupKind::Private, &o, false)).unwrap();
        assert_eq!(s.name, "Team");
        assert_eq!(s.role_of(&o), Some(Role::Owner));
        assert_eq!(s.current_key, Some(key(1)));
        assert_eq!(s.replaced_key, None, "the first key replaced none");
        assert!(!s.history_for_new);
        assert_eq!(s.version, 1);
        let public = GroupState::genesis(&create(GroupKind::Public, &o, false)).unwrap();
        assert!(public.history_for_new, "a public group always shows its history");

        let mut no_key = create(GroupKind::Private, &o, false);
        no_key.key = None;
        assert_eq!(GroupState::genesis(&no_key), Err(Rejection::KeyRequired));
        let mut bad = create(GroupKind::Private, &o, false);
        bad.group_id = "short".into();
        assert!(matches!(GroupState::genesis(&bad), Err(Rejection::Invalid { .. })));
        assert_eq!(GroupState::genesis(&op(&o, OpBody::Join)), Err(Rejection::NotCreated));
    }

    #[test]
    fn rejected_operations_change_nothing() {
        let (o, a, m, u, x) = (pk("0"), pk("a"), pk("b"), pk("c"), pk("d"));
        let mut s = team(GroupKind::Private, true);
        let before = s.clone();
        let cases: Vec<(Op, Rejection)> = vec![
            (op(&x, OpBody::Leave), Rejection::AuthorNotMember),
            (op(&x, OpBody::Join), Rejection::WrongKind),
            (op(&u, OpBody::Admit { who: x.clone() }), Rejection::NotPermitted),
            (op(&m, OpBody::Admit { who: x.clone() }), Rejection::NotPermitted),
            (op(&a, OpBody::Admit { who: u.clone() }), Rejection::AlreadyMember),
            (op(&a, OpBody::Remove { who: u.clone() }), Rejection::KeyRequired),
            (op(&a, OpBody::Remove { who: o.clone() }).with_key(key(9)), Rejection::NotPermitted),
            (op(&a, OpBody::Remove { who: x.clone() }).with_key(key(9)), Rejection::TargetNotMember),
            (op(&u, OpBody::SetMuted { who: m.clone(), muted: true, until: None }), Rejection::NotPermitted),
            (op(&a, OpBody::SetRole { who: u.clone(), role: Role::Admin }), Rejection::NotPermitted),
            (op(&a, OpBody::TransferOwnership { to: u.clone() }), Rejection::NotPermitted),
            (op(&o, OpBody::TransferOwnership { to: x.clone() }), Rejection::TargetNotMember),
            (op(&o, OpBody::Leave), Rejection::NotPermitted),
            (op(&a, OpBody::Disband), Rejection::NotPermitted),
            (op(&a, OpBody::Unban { who: x.clone() }), Rejection::NotBanned),
            (op(&a, OpBody::RotateLink { link_epoch: 1 }).with_key(key(9)), Rejection::WrongKind),
            (op(&a, OpBody::RotateKey), Rejection::KeyRequired),
            (op(&u, OpBody::Leave).with_key(key(9)), Rejection::KeyNotAllowed),
            (op(&m, OpBody::SetMuted { who: u.clone(), muted: true, until: None }).with_key(key(9)), Rejection::KeyNotAllowed),
            (create(GroupKind::Private, &o, true), Rejection::AlreadyCreated),
        ];
        for (operation, want) in cases {
            assert_eq!(s.apply(&operation), Err(want), "{:?} by {:?}", operation.body, operation.author);
            assert_eq!(s, before, "state must not change on rejection");
        }
        let mut other = op(&o, OpBody::RotateKey).with_key(key(9));
        other.group_id = "ff".repeat(32);
        assert_eq!(s.apply(&other), Err(Rejection::WrongGroup));
        let mut future = op(&o, OpBody::RotateKey).with_key(key(9));
        future.v = 3;
        assert_eq!(s.apply(&future), Err(Rejection::UnsupportedVersion));
        let mut zero = op(&o, OpBody::RotateKey).with_key(key(9));
        zero.v = 0;
        assert_eq!(s.apply(&zero), Err(Rejection::UnsupportedVersion));
        // A body that needs version 2 under the label of version 1: an old
        // build would read it as the old shape (a mute for good), so it is
        // no operation at all.
        let mut mislabelled = op(&m, OpBody::SetMuted { who: u.clone(), muted: true, until: Some(900) });
        mislabelled.v = 1;
        assert_eq!(s.apply(&mislabelled), Err(invalid("version")));
        assert_eq!(s, before, "state must not change on rejection");
        // A mute with an end behind it, or an end on a lifting.
        assert!(matches!(s.apply(&op(&m, OpBody::SetMuted { who: u.clone(), muted: true, until: Some(200) })), Err(Rejection::Invalid { .. })));
        assert!(matches!(s.apply(&op(&m, OpBody::SetMuted { who: u.clone(), muted: false, until: Some(900) })), Err(Rejection::Invalid { .. })));
        assert_eq!(s.apply(&op(&u, OpBody::SetTrusted { who: m.clone(), trusted: true })), Err(Rejection::NotPermitted));
        assert_eq!(s.apply(&op(&m, OpBody::SetTrusted { who: x.clone(), trusted: true })), Err(Rejection::TargetNotMember));
        let bad = Schedule { tz: "UTC".into(), days: vec![8], from: "09:00".into(), to: "18:00".into() };
        assert!(matches!(s.apply(&op(&a, Settings { post_schedule: Some(bad), ..Settings::default() }.body())), Err(Rejection::Invalid { .. })));
        assert_eq!(s.apply(&op(&m, Settings { post_mode: Some(PostMode::Managers), ..Settings::default() }.body())), Err(Rejection::NotPermitted));
        assert_eq!(s, before);
        assert!(matches!(
            s.apply(&op(&o, Settings { name: Some("  ".into()), ..Settings::default() }.body())),
            Err(Rejection::Invalid { .. })
        ));
        assert!(matches!(
            s.apply(&op(&o, Settings { picture: Some("http://x".into()), ..Settings::default() }.body())),
            Err(Rejection::Invalid { .. })
        ));
        assert_eq!(s, before);
    }

    #[test]
    fn private_group_life() {
        let (o, a, m, u, x) = (pk("0"), pk("a"), pk("b"), pk("c"), pk("d"));
        let mut s = team(GroupKind::Private, false);
        assert_eq!(s.members.len(), 4);
        assert_eq!(s.keys.len(), 4, "history hidden: every admission brought a key");

        // The admin admits, mutes through the moderator, removes.
        s.apply(&op(&a, OpBody::Admit { who: x.clone() }).with_key(key(20))).unwrap();
        assert_eq!(s.member(&x).unwrap().added_by, a);
        s.apply(&op(&m, OpBody::SetMuted { who: x.clone(), muted: true, until: None })).unwrap();
        assert!(s.can_post_at(&x, 300).is_err() && s.is_member(&x));
        s.apply(&op(&a, OpBody::Remove { who: x.clone() }).with_key(key(21))).unwrap();
        assert!(!s.is_member(&x));
        assert_eq!(s.current_key, Some(key(21)));
        assert_eq!(s.replaced_key, Some(ReplacedKey { key: key(20), at: 200 }), "the key before, and since when");
        assert!(!s.shows_messages_of(&x), "private: only members are shown");

        // A member leaves: the key is stale until a manager brings a new one.
        s.apply(&op(&u, OpBody::Leave)).unwrap();
        assert!(s.key_stale);
        s.apply(&op(&a, OpBody::RotateKey).with_key(key(22))).unwrap();
        assert!(!s.key_stale);

        // Ban removes and keeps out.
        s.apply(&op(&o, OpBody::Ban { who: m.clone() }).with_key(key(23))).unwrap();
        assert!(!s.is_member(&m) && s.is_banned(&m));
        assert_eq!(s.apply(&op(&a, OpBody::Admit { who: m.clone() }).with_key(key(24))), Err(Rejection::TargetBanned));
        // Banning an outsider needs no key: they never had one.
        s.apply(&op(&a, OpBody::Ban { who: x.clone() })).unwrap();
        s.apply(&op(&a, OpBody::Unban { who: x.clone() })).unwrap();
        assert!(!s.is_banned(&x));

        // Ownership moves; the old owner is an admin now.
        s.apply(&op(&o, OpBody::TransferOwnership { to: a.clone() })).unwrap();
        assert_eq!((s.role_of(&a), s.role_of(&o)), (Some(Role::Owner), Some(Role::Admin)));
        assert_eq!(s.owner, a);
        assert_eq!(s.apply(&op(&o, OpBody::Disband)), Err(Rejection::NotPermitted));
        s.apply(&op(&o, OpBody::Leave)).unwrap();

        s.apply(&op(&a, Settings { name: Some("New".into()), about: Some("x".into()), history_for_new: Some(true), ..Settings::default() }.body())).unwrap();
        assert_eq!((s.name.as_str(), s.history_for_new), ("New", true));
        assert!(!s.requires_key(&OpBody::Admit { who: x.clone() }), "history shown: newcomers get the old keys");
        // The call node pinned to the group: a reference of the right
        // shape with its key, nothing else; an empty string takes it away.
        let node = format!("203.0.113.7:8443#{}", "fa".repeat(32));
        let pin = |n: &str, k: Option<&str>| Settings { call_node: Some(n.into()), call_node_key: k.map(String::from), ..Settings::default() }.body();
        s.apply(&op(&a, pin(&node, Some("secret")))).unwrap();
        assert_eq!((s.call_node.as_str(), s.call_node_key.as_str()), (node.as_str(), "secret"));
        assert!(matches!(s.apply(&op(&a, pin("203.0.113.7:8443", None))), Err(Rejection::Invalid { .. })), "no id");
        assert!(matches!(s.apply(&op(&a, pin("203.0.113.7#abcd", None))), Err(Rejection::Invalid { .. })), "no port, short id");
        assert_eq!(s.call_node, node, "a refused edit changes nothing");
        s.apply(&op(&a, pin("", Some("")))).unwrap();
        assert!(s.call_node.is_empty() && s.call_node_key.is_empty());

        s.apply(&op(&a, OpBody::Disband)).unwrap();
        assert_eq!(s.apply(&op(&a, OpBody::RotateKey).with_key(key(30))), Err(Rejection::Disbanded));
        assert_eq!(s.can_post_at(&a, 300), Err(Closed::NotMember));
    }

    #[test]
    fn public_group_life() {
        let (o, a, x, y) = (pk("0"), pk("a"), pk("d"), pk("e"));
        let mut s = team(GroupKind::Public, false);
        assert_eq!(s.keys.len(), 1, "public: admissions bring no keys");

        s.apply(&op(&x, OpBody::Join)).unwrap();
        assert_eq!(s.role_of(&x), Some(Role::Member));
        assert_eq!(s.apply(&op(&x, OpBody::Join)), Err(Rejection::AlreadyMember));
        assert_eq!(s.apply(&op(&y, OpBody::Join).with_key(key(9))), Err(Rejection::KeyNotAllowed));

        // Leaving and removing need no new key: the link gives it to anyone.
        s.apply(&op(&x, OpBody::Leave)).unwrap();
        assert!(!s.key_stale);
        s.apply(&op(&x, OpBody::Join)).unwrap();
        s.apply(&op(&a, OpBody::Remove { who: x.clone() })).unwrap();

        // A ban hides the author and refuses their operations.
        s.apply(&op(&a, OpBody::Ban { who: y.clone() })).unwrap();
        assert_eq!(s.apply(&op(&y, OpBody::Join)), Err(Rejection::AuthorBanned));
        assert!(!s.shows_messages_of(&y));
        assert!(s.shows_messages_of(&x), "public: former members stay visible");

        assert_eq!(
            s.apply(&op(&a, Settings { history_for_new: Some(false), ..Settings::default() }.body())),
            Err(Rejection::WrongKind)
        );
        assert_eq!(s.apply(&op(&a, OpBody::RotateLink { link_epoch: 1 })), Err(Rejection::KeyRequired));
        s.apply(&op(&a, OpBody::RotateLink { link_epoch: 1 }).with_key(key(2))).unwrap();
        assert_eq!((s.link_epoch, s.current_key.clone()), (1, Some(key(2))));
        assert!(matches!(s.apply(&op(&o, OpBody::RotateLink { link_epoch: 1 }).with_key(key(3))), Err(Rejection::Invalid { .. })));
    }

    #[test]
    fn deleting_messages() {
        let (o, a, m, u, x) = (pk("0"), pk("a"), pk("b"), pk("c"), pk("d"));
        let s = team(GroupKind::Private, true);
        assert!(s.can_delete_message(&u, &u), "own message");
        assert!(!s.can_delete_message(&u, &m));
        assert!(s.can_delete_message(&m, &u));
        assert!(!s.can_delete_message(&m, &a));
        assert!(s.can_delete_message(&a, &m));
        assert!(!s.can_delete_message(&a, &o));
        assert!(s.can_delete_message(&o, &a));
        assert!(s.can_delete_message(&m, &x), "a former member's message counts as a member's");
        assert!(!s.can_delete_message(&x, &x), "outsiders delete nothing");
        assert_eq!(s.managers().len(), 2);
        assert_eq!(s.key_holders().len(), 4);
    }

    /// Unix seconds of a wall-clock moment in UTC.
    pub(crate) fn utc(y: i16, mo: i8, d: i8, h: i8, mi: i8) -> i64 {
        jiff::civil::date(y, mo, d).at(h, mi, 0, 0).to_zoned(jiff::tz::TimeZone::UTC).unwrap().timestamp().as_second()
    }

    #[test]
    fn a_timed_mute_lifts_by_itself() {
        let (o, m, u) = (pk("0"), pk("b"), pk("c"));
        let mut s = team(GroupKind::Private, true);
        // Every operation of `op` is dated 200.
        s.apply(&op(&m, OpBody::SetMuted { who: u.clone(), muted: true, until: Some(500) })).unwrap();
        assert_eq!(s.member(&u).unwrap().muted_until, Some(500));
        assert_eq!(s.can_post_at(&u, 300), Err(Closed::Muted { until: Some(500) }));
        assert_eq!(s.can_post_at(&u, 499), Err(Closed::Muted { until: Some(500) }));
        assert_eq!(s.can_post_at(&u, 500), Ok(()), "lifted by the clock, without an operation");
        assert!(s.member(&u).unwrap().muted && !s.member(&u).unwrap().muted_at(500), "the log still says muted; the time says no longer");
        // Muted again, with no end.
        s.apply(&op(&m, OpBody::SetMuted { who: u.clone(), muted: true, until: None })).unwrap();
        assert_eq!(s.can_post_at(&u, 10_000), Err(Closed::Muted { until: None }));
        s.apply(&op(&m, OpBody::SetMuted { who: u.clone(), muted: false, until: None })).unwrap();
        assert_eq!(s.can_post_at(&u, 10_000), Ok(()));
        // The group handed to a muted member lifts the mute, its end too.
        s.apply(&op(&m, OpBody::SetMuted { who: u.clone(), muted: true, until: Some(900) })).unwrap();
        s.apply(&op(&o, OpBody::TransferOwnership { to: u.clone() })).unwrap();
        assert_eq!((s.member(&u).unwrap().muted, s.member(&u).unwrap().muted_until), (false, None));
    }

    #[test]
    fn trust_follows_the_table_of_mutes() {
        let (o, a, m, u) = (pk("0"), pk("a"), pk("b"), pk("c"));
        let mut s = team(GroupKind::Private, true);
        // A moderator trusts a member; nobody trusts an equal or above.
        s.apply(&op(&m, OpBody::SetTrusted { who: u.clone(), trusted: true })).unwrap();
        assert!(s.member(&u).unwrap().trusted);
        assert_eq!(s.apply(&op(&m, OpBody::SetTrusted { who: a.clone(), trusted: true })), Err(Rejection::NotPermitted));
        assert_eq!(s.apply(&op(&m, OpBody::SetTrusted { who: m.clone(), trusted: true })), Err(Rejection::NotPermitted));
        assert_eq!(s.apply(&op(&a, OpBody::SetTrusted { who: m.clone(), trusted: true })), Ok(()));
        assert_eq!(s.apply(&op(&a, OpBody::SetTrusted { who: o.clone(), trusted: true })), Err(Rejection::NotPermitted));
        assert_eq!(s.apply(&op(&o, OpBody::SetTrusted { who: a.clone(), trusted: true })), Ok(()));
        s.apply(&op(&m, OpBody::SetTrusted { who: u.clone(), trusted: false })).unwrap();
        assert!(!s.member(&u).unwrap().trusted);
        // Trust is of one stay: gone with the member, not back with a re-admission.
        s.apply(&op(&m, OpBody::SetTrusted { who: u.clone(), trusted: true })).unwrap();
        s.apply(&op(&a, OpBody::Remove { who: u.clone() }).with_key(key(30))).unwrap();
        s.apply(&op(&a, OpBody::Admit { who: u.clone() })).unwrap();
        assert!(!s.member(&u).unwrap().trusted);
    }

    #[test]
    fn posting_rules_managers_only_and_the_schedule() {
        let (o, a, m, u) = (pk("0"), pk("a"), pk("b"), pk("c"));
        let mut s = team(GroupKind::Private, true);
        for who in [&o, &a, &m, &u] {
            assert_eq!(s.can_post_at(who, 300), Ok(()));
        }
        s.apply(&op(&a, Settings { post_mode: Some(PostMode::Managers), ..Settings::default() }.body())).unwrap();
        assert_eq!(s.post_mode, PostMode::Managers);
        assert_eq!(s.can_post_at(&u, 300), Err(Closed::ManagersOnly));
        for who in [&o, &a, &m] {
            assert_eq!(s.can_post_at(who, 300), Ok(()), "moderators and above write on");
        }
        s.apply(&op(&a, Settings { post_mode: Some(PostMode::All), ..Settings::default() }.body())).unwrap();
        assert_eq!(s.can_post_at(&u, 300), Ok(()));

        // Fridays, 09:00 to 18:00 UTC; 2026-10-16 is a Friday.
        let sched = Schedule { tz: "UTC".into(), days: vec![5], from: "09:00".into(), to: "18:00".into() };
        s.apply(&op(&a, Settings { post_schedule: Some(sched.clone()), ..Settings::default() }.body())).unwrap();
        assert_eq!(s.schedule.as_ref(), Some(&sched));
        assert!(!s.schedule_tz_unknown());
        assert_eq!(s.can_post_at(&u, utc(2026, 10, 16, 12, 0)), Ok(()));
        assert_eq!(s.can_post_at(&u, utc(2026, 10, 16, 18, 0)), Err(Closed::Schedule { opens_at: utc(2026, 10, 23, 9, 0) }));
        assert_eq!(s.can_post_at(&m, utc(2026, 10, 16, 18, 0)), Ok(()), "a moderator is never held by the hours");
        // A mute is said before the hours.
        s.apply(&op(&m, OpBody::SetMuted { who: u.clone(), muted: true, until: None })).unwrap();
        assert_eq!(s.can_post_at(&u, utc(2026, 10, 16, 18, 0)), Err(Closed::Muted { until: None }));
        s.apply(&op(&m, OpBody::SetMuted { who: u.clone(), muted: false, until: None })).unwrap();
        // A zone this build does not know: read in UTC, and the state says so.
        let far = Schedule { tz: "Mars/Olympus".into(), ..sched.clone() };
        s.apply(&op(&a, Settings { post_schedule: Some(far.clone()), ..Settings::default() }.body())).unwrap();
        assert_eq!(s.schedule.as_ref(), Some(&far), "taken: another device may know the zone");
        assert!(s.schedule_tz_unknown());
        assert_eq!(s.can_post_at(&u, utc(2026, 10, 16, 12, 0)), Ok(()));
        assert_eq!(s.can_post_at(&u, utc(2026, 10, 16, 8, 0)), Err(Closed::Schedule { opens_at: utc(2026, 10, 16, 9, 0) }));
        // No days: the schedule is gone.
        s.apply(&op(&a, Settings { post_schedule: Some(Schedule { days: vec![], ..sched }), ..Settings::default() }.body())).unwrap();
        assert_eq!(s.schedule, None);
        assert!(!s.schedule_tz_unknown());
        assert_eq!(s.can_post_at(&u, utc(2026, 10, 16, 18, 0)), Ok(()));
    }

    #[test]
    fn premod_counts_newcomers_by_their_place_in_the_log() {
        let (o, a, m, u, x, y) = (pk("0"), pk("a"), pk("b"), pk("c"), pk("d"), pk("e"));
        let mut s = team(GroupKind::Private, true);
        assert_eq!(s.member(&o).unwrap().joined_pos, 0);
        assert_eq!(s.member(&a).unwrap().joined_pos, 1);
        assert_eq!(s.member(&u).unwrap().joined_pos, 3);
        for who in [&o, &a, &m, &u] {
            assert!(!s.subject_to_premod(who));
        }
        // Newcomers: those let in after the rule.
        s.apply(&op(&a, Settings { premod: Some(Premod::Newcomers), ..Settings::default() }.body())).unwrap();
        let since = s.premod_since.unwrap();
        assert_eq!(since, s.version - 1, "the place of the operation that set the rule");
        assert!(!s.subject_to_premod(&u), "a member from before");
        // x is let in by an operation dated before the rule: the order of
        // the log counts, not anybody's clock.
        let mut early = op(&a, OpBody::Admit { who: x.clone() });
        early.created_at = 150;
        s.apply(&early).unwrap();
        assert!(s.member(&x).unwrap().joined_pos > since);
        assert!(s.subject_to_premod(&x));
        // Trusted, or raised: no longer held.
        s.apply(&op(&m, OpBody::SetTrusted { who: x.clone(), trusted: true })).unwrap();
        assert!(!s.subject_to_premod(&x));
        s.apply(&op(&m, OpBody::SetTrusted { who: x.clone(), trusted: false })).unwrap();
        assert!(s.subject_to_premod(&x));
        s.apply(&op(&o, OpBody::SetRole { who: x.clone(), role: Role::Moderator })).unwrap();
        assert!(!s.subject_to_premod(&x));
        // Everyone: the old member too, never a moderator.
        s.apply(&op(&a, Settings { premod: Some(Premod::All), ..Settings::default() }.body())).unwrap();
        assert_eq!(s.premod_since, Some(since), "the line stays while the rule is on");
        assert!(s.subject_to_premod(&u) && !s.subject_to_premod(&m) && !s.subject_to_premod(&x));
        // Off: nobody. On again: a new line, with everyone here before it.
        s.apply(&op(&a, Settings { premod: Some(Premod::Off), ..Settings::default() }.body())).unwrap();
        assert_eq!(s.premod_since, None);
        assert!(!s.subject_to_premod(&u));
        s.apply(&op(&a, OpBody::Admit { who: y.clone() })).unwrap();
        s.apply(&op(&a, Settings { premod: Some(Premod::Newcomers), ..Settings::default() }.body())).unwrap();
        assert!(s.premod_since.unwrap() > since);
        assert!(!s.subject_to_premod(&y) && !s.subject_to_premod(&u));
        // Out and in again: a newcomer.
        s.apply(&op(&a, OpBody::Remove { who: y.clone() }).with_key(key(31))).unwrap();
        s.apply(&op(&a, OpBody::Admit { who: y.clone() })).unwrap();
        assert!(s.subject_to_premod(&y));
        assert!(!s.subject_to_premod(&pk("f")), "an outsider is nobody's to hold");
    }
}
