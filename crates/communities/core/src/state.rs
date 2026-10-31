// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from crates/messenger/groups/src/state.rs at 1f2ef31d, then changed for channels.

//! The state of a channel by its journal and the effect of one operation
//! on it (internal/communities-wire.md §3).
//!
//! `apply` either changes the state or says why not. It never looks at
//! anything but the state and the operation, so two devices that apply
//! the same operations in the same order hold the same state.
//!
//! The journal knows the staff (owner, admins, moderators, authors), the
//! people it says something about (muted, trusted, levels), the bans, the
//! settings, the keepers and the pins. It never knows the readers: the
//! keeper lets them in and out and its snapshot says who they are. Where
//! a reader's rights are asked, the caller passes what the snapshot says
//! of them (`Reader`).

use crate::log::OpLog;
use crate::op::{Comments, History, JoinMode, KeeperRef, LevelCan, Op, OpBody, PostMode, Premod, Schedule, MAX_PARENTS, OP_VERSION};
use crate::roles::{permits, Action, Role};
use crate::schedule::{self, Status};
use keydist::SchemeId;
use messenger_core::{PubKey, RelayUrl};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_NAME_CHARS: usize = 100;
pub const MAX_ABOUT_CHARS: usize = 1000;
pub const MAX_PICTURE_CHARS: usize = 500;
/// Relays and storage hosts a channel names: at least one, at most this.
pub const MAX_HOSTS: usize = 8;
pub const MAX_HOST_CHARS: usize = 256;
/// The scope of everybody who entered, made with the channel (wire §1).
pub const SCOPE_EVERYONE: &str = "everyone";
/// The most scopes a channel has, `everyone` among them (wire §14.1).
pub const MAX_SCOPES: usize = 16;
/// Level ids run 0…63: a bit each of the `u64` of `set_levels`.
pub const MAX_LEVEL_ID: u8 = 63;
/// The base level: everybody who entered holds it; it gives `everyone`,
/// may be renamed and given other rights, never deleted.
pub const BASE_LEVEL: u8 = 0;
/// What a name of a level and a title of a scope may be at most (they
/// travel sealed, `wire::Meta`).
pub const MAX_LEVEL_NAME_CHARS: usize = 40;
pub const MAX_SCOPE_TITLE_CHARS: usize = 80;
/// The most members a `flat-v1` scope takes (spec §5: one secret per
/// member per epoch).
pub const FLAT_MAX_MEMBERS: usize = 300;
/// How far before its join a reader's message may be dated: clocks differ.
pub const JOIN_SLACK_SECS: i64 = 300;

/// What the journal says of one person: their role, a mute, trust,
/// levels. Readers the journal says nothing about are not here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Staff {
    /// `Member` for a reader the journal only mutes, trusts or levels.
    pub role: Role,
    /// May read, may not write.
    pub muted: bool,
    /// When the mute lifts by itself; none for one that stays until
    /// lifted. Means nothing while `muted` is false.
    pub muted_until: Option<i64>,
    /// Their messages are never held for moderation.
    pub trusted: bool,
    /// The levels `set_levels` gave (a bit per level id; the base level,
    /// bit 0, is everybody's whether set or not).
    pub levels: u64,
    /// The time of the operation that first named them.
    pub since: i64,
    /// Who named them; themselves for the owner.
    pub added_by: PubKey,
    /// Where in the applied order the operation that made them staff
    /// stands (the `Create` is 0): who came before whom, by the journal
    /// and not by anybody's clock.
    pub joined_pos: u64,
    /// Uploads the files of posts into their own quota (`set_quota`, wire
    /// §9.2); `false`, as everyone starts, into the owner's.
    #[serde(default, skip_serializing_if = "is_false")]
    pub quota_own: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Staff {
    /// Muted at `now`: a mute with an end lifts by itself.
    pub fn muted_at(&self, now: i64) -> bool {
        self.muted && self.muted_until.is_none_or(|until| now < until)
    }

    /// Nothing left to say of a reader: the entry can go.
    fn is_plain_reader(&self) -> bool {
        self.role == Role::Member && !self.muted && !self.trusted && self.levels & !1 == 0
    }
}

/// What the snapshot of the keeper says of a reader (wire §8): since when
/// they are in. The journal does not know it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reader {
    pub since: i64,
}

/// A scope of the channel by the journal (wire §14.1): its scheme and
/// history, fixed when it was made (`everyone` follows the history of the
/// channel). Its title is sealed and is not here. `hidden` (2026-10-23):
/// its title and look are sealed in its own feed, and those outside it are
/// not shown it at all; `scope_add` and `scope_set` say it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeDef {
    pub scheme: SchemeId,
    pub history: History,
    #[serde(default)]
    pub hidden: bool,
}

/// A level by the journal: the scopes it gives and what it lets do. Its
/// name is sealed and is not here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelDef {
    pub scopes: Vec<String>,
    pub can: LevelCan,
}

/// What is written: a post in the feed or a comment under a post.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Content {
    Post,
    Comment,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelState {
    pub community_id: String,
    // The card.
    pub name: String,
    pub about: String,
    pub picture: String,
    // The settings.
    pub join: JoinMode,
    pub history: History,
    pub relays: Vec<String>,
    pub storage: Vec<String>,
    pub scheme: SchemeId,
    /// Who may post: authors and above, always in stage 1 (wire §3,
    /// 2026-10-22); the `post_mode` of an operation stays on the wire and
    /// changes nothing.
    pub post_mode: PostMode,
    /// Whose messages wait for a moderator.
    pub premod: Premod,
    /// Where in the applied order the operation that turned `premod` on
    /// stands: staff made after it are newcomers. None while it is off.
    pub premod_since: Option<u64>,
    /// The time of that operation: readers who joined after it (by the
    /// snapshot) are newcomers.
    pub premod_since_at: Option<i64>,
    /// When readers may post; none for always.
    pub schedule: Option<Schedule>,
    pub comments: Comments,
    /// Which link secret admits newcomers (wire §10).
    pub link_epoch: u32,
    pub owner: PubKey,
    /// The author of the `create`: the owner who made the channel, not
    /// moved by a handover; the only author of a genesis a device without
    /// the journal takes (the `o` of a link, `owner` of my note of keys).
    pub creator: PubKey,
    /// Hex public key → what the journal says of them.
    pub staff: BTreeMap<String, Staff>,
    pub banned: BTreeSet<String>,
    /// The keeper series by `serial`, oldest first.
    pub keepers: Vec<KeeperRef>,
    /// Ids of pinned posts.
    pub pinned: BTreeSet<String>,
    /// The scopes by id, `everyone` always among them.
    #[serde(default)]
    pub scopes: BTreeMap<String, ScopeDef>,
    /// The levels by id, the base level always among them.
    #[serde(default)]
    pub levels: BTreeMap<u8, LevelDef>,
    /// A handover the owner made that waits for the `keeper` of the new
    /// owner (wire §3: in force from stage 3, taken and kept now).
    pub pending_owner: Option<PubKey>,
    pub disbanded: bool,
    /// Operations applied so far.
    pub version: u64,
}

/// Why someone may not write right now (`ChannelState::can_post_at`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Closed {
    /// Not in the channel (neither staff nor a reader), or it is disbanded.
    NotMember,
    Banned,
    /// Muted; `until` says when it lifts, none for until somebody lifts it.
    Muted { until: Option<i64> },
    /// Only authors and above post now.
    AuthorsOnly,
    /// Comments are off.
    CommentsOff,
    /// None of their levels lets them write this (wire §14.1).
    Level,
    /// Outside the hours of the schedule; the moment it opens.
    Schedule { opens_at: i64 },
}

/// The rules of writing that are not about one person: who may post or
/// comment, and when. Moderators and above are never held by them. Posts
/// are by authors and above, whatever `post_mode` an operation names: the
/// stage 1 fixes it to `authors` (wire §3, 2026-10-22); readers comment.
/// `can`: what the levels of the person let do (wire §14.1): a level may
/// let its readers post, or take comments from them; staff write by role.
pub(crate) fn gate(role: Role, can: LevelCan, content: Content, comments: Comments, schedule: Option<&Schedule>, now: i64) -> Result<(), Closed> {
    if role.is_moderator() {
        return Ok(());
    }
    match content {
        Content::Post if !permits(role, Action::Post) && !can.post => return Err(Closed::AuthorsOnly),
        Content::Comment if comments == Comments::Off => return Err(Closed::CommentsOff),
        Content::Comment if role < Role::Author && !can.comment => return Err(Closed::Level),
        _ => {}
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
    WrongChannel,
    NotCreated,
    AlreadyCreated,
    Disbanded,
    /// The author is not in the journal (a reader writes no operations).
    AuthorNotMember,
    AuthorBanned,
    NotPermitted,
    TargetBanned,
    NotBanned,
    Invalid { what: String },
}

fn invalid(what: &str) -> Rejection {
    Rejection::Invalid { what: what.into() }
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

pub(crate) fn check_text(name: Option<&str>, about: Option<&str>, picture: Option<&str>) -> Result<(), Rejection> {
    if let Some(n) = name {
        let n = n.trim();
        if n.is_empty() || n.chars().count() > MAX_NAME_CHARS || n.chars().any(char::is_control) || messenger_richtext::normalize(n).is_empty() {
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

/// Relays: 1…8, each in the normal form of `RelayUrl` (so that the
/// canonical form names each one way), no repeats.
pub(crate) fn check_relays(relays: &[String]) -> Result<(), Rejection> {
    let ok = (1..=MAX_HOSTS).contains(&relays.len())
        && relays.iter().all(|r| r.len() <= MAX_HOST_CHARS && RelayUrl::parse(r).is_some_and(|p| p.as_str() == r))
        && relays.iter().collect::<BTreeSet<_>>().len() == relays.len();
    if ok {
        Ok(())
    } else {
        Err(invalid("relays"))
    }
}

/// Storage hosts: 1…8 `https://` addresses (a path is allowed: blobs are
/// `<host>/<sha256>`), without a query, a fragment or a trailing slash.
pub(crate) fn check_storage(hosts: &[String]) -> Result<(), Rejection> {
    let one = |h: &String| {
        h.len() <= MAX_HOST_CHARS
            && h.strip_prefix("https://").is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'))
            && !h.ends_with('/')
            && !h.chars().any(|c| c.is_control() || c.is_whitespace() || c == '?' || c == '#')
    };
    if (1..=MAX_HOSTS).contains(&hosts.len()) && hosts.iter().all(one) && hosts.iter().collect::<BTreeSet<_>>().len() == hosts.len() {
        Ok(())
    } else {
        Err(invalid("storage"))
    }
}

/// The scopes the levels of a bit set give, with `everyone`: a level
/// gone or a scope gone gives nothing.
pub(crate) fn scopes_by_levels(levels: &BTreeMap<u8, LevelDef>, scopes: &BTreeMap<String, ScopeDef>, bits: u64) -> BTreeSet<String> {
    let mut out = BTreeSet::from([SCOPE_EVERYONE.to_string()]);
    for (id, l) in levels {
        if bits & (1u64 << id) != 0 {
            out.extend(l.scopes.iter().filter(|s| scopes.contains_key(*s)).cloned());
        }
    }
    out
}

/// What the levels of a bit set let do, all together.
pub(crate) fn can_by_levels(levels: &BTreeMap<u8, LevelDef>, bits: u64) -> LevelCan {
    levels.iter().filter(|(id, _)| bits & (1u64 << *id) != 0).fold(LevelCan::NONE, |acc, (_, l)| acc.union(l.can))
}

/// The scopes a reader may post into by the levels of a bit set
/// (2026-10-23, the owner's model): those of the levels that let post,
/// and no other. `everyone` only when such a level gives it (the base
/// level always does): a level «Золото» that posts in its own section
/// does not make its readers authors of the whole channel.
pub(crate) fn post_scopes_by_levels(levels: &BTreeMap<u8, LevelDef>, scopes: &BTreeMap<String, ScopeDef>, bits: u64) -> BTreeSet<String> {
    levels
        .iter()
        .filter(|(id, l)| bits & (1u64 << *id) != 0 && l.can.post)
        .flat_map(|(_, l)| l.scopes.iter().filter(|s| scopes.contains_key(*s)).cloned())
        .collect()
}

/// A scope id of stage 2: 16 random bytes, hex (wire §1).
pub(crate) fn is_scope_id(s: &str) -> bool {
    is_hex(s, 32)
}

impl ChannelState {
    /// The state right after `Create`.
    pub fn genesis(op: &Op) -> Result<Self, Rejection> {
        if op.v == 0 || op.v > OP_VERSION {
            return Err(Rejection::UnsupportedVersion);
        }
        if op.v < op.body.version() {
            return Err(invalid("version"));
        }
        let OpBody::Create { join, name, about, picture, history, relays, storage, scheme, keeper, link_epoch } = &op.body else {
            return Err(Rejection::NotCreated);
        };
        if !op.parents.is_empty() {
            return Err(invalid("create has parents"));
        }
        if !is_hex(&op.community_id, 64) {
            return Err(invalid("community id"));
        }
        check_text(Some(name), Some(about), Some(picture))?;
        check_relays(relays)?;
        check_storage(storage)?;
        // Stage 1 builds every scope on Subset Difference (wire §3).
        if *scheme != SchemeId::SdV1 {
            return Err(invalid("scheme"));
        }
        if keeper.serial != 0 || keeper.from_epoch != 1 {
            return Err(invalid("keeper"));
        }
        if *link_epoch != 0 {
            return Err(invalid("link epoch"));
        }
        let mut staff = BTreeMap::new();
        staff.insert(
            op.author.as_hex().to_string(),
            Staff { role: Role::Owner, muted: false, muted_until: None, trusted: false, levels: 0, since: op.created_at, added_by: op.author.clone(), joined_pos: 0, quota_own: false },
        );
        Ok(Self {
            community_id: op.community_id.clone(),
            name: name.trim().to_string(),
            about: about.clone(),
            picture: picture.clone(),
            join: *join,
            history: *history,
            relays: relays.clone(),
            storage: storage.clone(),
            scheme: *scheme,
            post_mode: PostMode::Authors,
            premod: Premod::Off,
            premod_since: None,
            premod_since_at: None,
            schedule: None,
            comments: Comments::On,
            link_epoch: 0,
            owner: op.author.clone(),
            creator: op.author.clone(),
            staff,
            banned: BTreeSet::new(),
            keepers: vec![keeper.clone()],
            pinned: BTreeSet::new(),
            scopes: BTreeMap::from([(SCOPE_EVERYONE.to_string(), ScopeDef { scheme: SchemeId::SdV1, history: *history, hidden: false })]),
            levels: BTreeMap::from([(BASE_LEVEL, LevelDef { scopes: vec![SCOPE_EVERYONE.to_string()], can: LevelCan::BASE })]),
            pending_owner: None,
            disbanded: false,
            version: 1,
        })
    }

    pub fn staff_of(&self, who: &PubKey) -> Option<&Staff> {
        self.staff.get(who.as_hex())
    }

    /// The role the journal gives: `None` for somebody it says nothing of.
    pub fn role_of(&self, who: &PubKey) -> Option<Role> {
        self.staff_of(who).map(|m| m.role)
    }

    /// Named by the journal (staff, or a reader it mutes, trusts, levels).
    pub fn is_member(&self, who: &PubKey) -> bool {
        self.staff.contains_key(who.as_hex())
    }

    pub fn is_banned(&self, who: &PubKey) -> bool {
        self.banned.contains(who.as_hex())
    }

    /// The role someone writes with: their staff role, `Member` for a
    /// reader by the snapshot, none for somebody outside.
    fn role_with(&self, who: &PubKey, reader: Option<&Reader>) -> Option<Role> {
        match self.role_of(who) {
            Some(r) if r >= Role::Author => Some(r),
            _ => reader.map(|_| Role::Member),
        }
    }

    /// The levels of a person: those `set_levels` gave, and the base level,
    /// which everybody who entered holds.
    pub fn levels_of(&self, who: &PubKey) -> u64 {
        self.staff_of(who).map_or(0, |m| m.levels) | 1
    }

    /// The scopes a person belongs to (wire §14.1): `everyone` and those
    /// their levels give; moderators and above belong to every scope.
    pub fn scopes_of(&self, who: &PubKey) -> BTreeSet<String> {
        if self.role_of(who).is_some_and(Role::is_moderator) {
            return self.scopes.keys().cloned().collect();
        }
        scopes_by_levels(&self.levels, &self.scopes, self.levels_of(who))
    }

    /// Whether `who` belongs to `scope_id`.
    pub fn in_scope(&self, who: &PubKey, scope_id: &str) -> bool {
        self.scopes_of(who).contains(scope_id)
    }

    /// What the levels of a person let them do, all together.
    pub fn can_of(&self, who: &PubKey) -> LevelCan {
        can_by_levels(&self.levels, self.levels_of(who))
    }

    /// The scopes a person may post into now: an author and above by role,
    /// in every scope they belong to; a reader only where a level of theirs
    /// that lets post gives the scope (`post_scopes_by_levels`).
    pub fn post_scopes_of(&self, who: &PubKey) -> BTreeSet<String> {
        if self.role_of(who).is_some_and(|r| r >= Role::Author) {
            return self.scopes_of(who);
        }
        post_scopes_by_levels(&self.levels, &self.scopes, self.levels_of(who))
    }

    /// The levels that give `scope_id`.
    pub fn levels_giving(&self, scope_id: &str) -> Vec<u8> {
        self.levels.iter().filter(|(_, l)| scope_id == SCOPE_EVERYONE || l.scopes.iter().any(|s| s == scope_id)).map(|(id, _)| *id).collect()
    }

    /// The keeper of the newest series.
    pub fn keeper(&self) -> Option<&KeeperRef> {
        self.keepers.last()
    }

    /// The series whose range holds `epoch` (wire §5): the newest one that
    /// began at or before it.
    pub fn keeper_for_epoch(&self, epoch: u32) -> Option<&KeeperRef> {
        self.keepers.iter().rev().find(|k| k.from_epoch <= epoch)
    }

    /// The first epoch past the range of a series: where the next begins.
    pub fn range_end(&self, serial: u32) -> Option<u32> {
        self.keepers.iter().find(|k| k.serial == serial.saturating_add(1) && serial != u32::MAX).map(|k| k.from_epoch)
    }

    /// May this person write `content` into the channel at `now`, and if
    /// not, why not? Staff by the journal, or a reader by the snapshot;
    /// not banned, not muted (a mute with an end lifts by itself), and let
    /// through by the rules of the channel (`gate`).
    pub fn can_write_at(&self, who: &PubKey, reader: Option<&Reader>, content: Content, now: i64) -> Result<(), Closed> {
        if self.disbanded {
            return Err(Closed::NotMember);
        }
        if self.is_banned(who) {
            return Err(Closed::Banned);
        }
        let role = self.role_with(who, reader).ok_or(Closed::NotMember)?;
        if let Some(m) = self.staff_of(who).filter(|m| m.muted_at(now)) {
            return Err(Closed::Muted { until: m.muted_until });
        }
        gate(role, self.can_of(who), content, self.comments, self.schedule.as_ref(), now)
    }

    /// `can_write_at` for a post.
    pub fn can_post_at(&self, who: &PubKey, reader: Option<&Reader>, now: i64) -> Result<(), Closed> {
        self.can_write_at(who, reader, Content::Post, now)
    }

    /// `can_write_at` for a comment.
    pub fn can_comment_at(&self, who: &PubKey, reader: Option<&Reader>, now: i64) -> Result<(), Closed> {
        self.can_write_at(who, reader, Content::Comment, now)
    }

    /// Do this person's messages wait for a moderator? Never a moderator's
    /// or a trusted person's; under `Premod::Newcomers` only those who
    /// came in after the rule was set: staff by the order of the journal,
    /// readers by the time of their join in the snapshot.
    pub fn subject_to_premod(&self, who: &PubKey, reader: Option<&Reader>) -> bool {
        let entry = self.staff_of(who);
        let Some(role) = self.role_with(who, reader) else { return false };
        if role.is_moderator() || entry.is_some_and(|m| m.trusted) {
            return false;
        }
        match self.premod {
            Premod::Off => false,
            Premod::All => true,
            Premod::Newcomers => match entry.filter(|m| m.role >= Role::Author) {
                Some(m) => self.premod_since.is_some_and(|since| m.joined_pos > since),
                None => matches!((self.premod_since_at, reader), (Some(since), Some(r)) if r.since > since),
            },
        }
    }

    /// The schedule names a zone this build does not know: it is read in
    /// UTC until the app is updated.
    pub fn schedule_tz_unknown(&self) -> bool {
        self.schedule.as_ref().is_some_and(|s| schedule::status(s, 0).is_err())
    }

    /// May `actor` remove a message written by `author`? Authors remove
    /// their own; moderators and above those of lower roles. A message of
    /// someone the journal does not name counts as a reader's.
    pub fn can_delete_message(&self, actor: &PubKey, actor_reader: Option<&Reader>, author: &PubKey) -> bool {
        if self.disbanded {
            return false;
        }
        let Some(role) = self.role_with(actor, actor_reader) else { return false };
        if actor == author {
            return true;
        }
        permits(role, Action::DeleteMessageOf(self.role_of(author).unwrap_or(Role::Member)))
    }

    /// Whether messages of this author are shown: banned authors are not.
    pub fn shows_messages_of(&self, author: &PubKey) -> bool {
        !self.is_banned(author)
    }

    pub fn managers(&self) -> Vec<PubKey> {
        self.with_role(Role::Admin)
    }

    /// Moderators and above: whom requests and reports go to.
    pub fn moderators(&self) -> Vec<PubKey> {
        self.with_role(Role::Moderator)
    }

    /// Authors and above: who may put blobs on the storage (the roster of
    /// stage 1, communities-stage1.md §10.1).
    pub fn uploaders(&self) -> Vec<PubKey> {
        self.with_role(Role::Author)
    }

    /// Does `who` upload the files of posts into their own quota (wire §9.2)?
    /// `false` for the staff in the owner's, as everyone starts.
    pub fn quota_own(&self, who: &PubKey) -> bool {
        self.staff_of(who).is_some_and(|m| m.quota_own)
    }

    fn with_role(&self, at_least: Role) -> Vec<PubKey> {
        self.staff.iter().filter(|(_, m)| m.role >= at_least).filter_map(|(k, _)| PubKey::parse(k)).collect()
    }

    /// The entry of `who`, made as a reader's when the journal did not
    /// name them yet.
    fn entry(&mut self, who: &PubKey, op: &Op) -> &mut Staff {
        let pos = self.version;
        self.staff.entry(who.as_hex().to_string()).or_insert_with(|| Staff {
            role: Role::Member,
            muted: false,
            muted_until: None,
            trusted: false,
            levels: 0,
            since: op.created_at,
            added_by: op.author.clone(),
            joined_pos: pos,
            quota_own: false,
        })
    }

    /// A reader's entry with nothing left to say goes.
    fn tidy(&mut self, who: &PubKey) {
        if self.staff_of(who).is_some_and(Staff::is_plain_reader) {
            self.staff.remove(who.as_hex());
        }
    }

    /// The next series of the keeper: one more than the last, beginning
    /// after it began. The range of the last can only be checked against
    /// what it published by the members (`epoch::Accept`).
    fn check_keeper(&self, serial: u32, from_epoch: u32) -> Result<(), Rejection> {
        let last = self.keeper().ok_or_else(|| invalid("keeper"))?;
        if Some(serial) != last.serial.checked_add(1) || from_epoch <= last.from_epoch {
            return Err(invalid("keeper"));
        }
        Ok(())
    }

    /// Apply one operation. On `Err` the state is unchanged.
    pub fn apply(&mut self, op: &Op) -> Result<(), Rejection> {
        if op.v == 0 || op.v > OP_VERSION {
            return Err(Rejection::UnsupportedVersion);
        }
        if op.v < op.body.version() {
            return Err(invalid("version"));
        }
        if op.community_id != self.community_id {
            return Err(Rejection::WrongChannel);
        }
        if matches!(op.body, OpBody::Create { .. }) {
            return Err(Rejection::AlreadyCreated);
        }
        if op.parents.is_empty() {
            return Err(invalid("no parents"));
        }
        if op.parents.len() > MAX_PARENTS {
            return Err(invalid("parents"));
        }
        // After a disband the owner still names a keeper: whoever reads the
        // past needs to know whose epochs to take.
        if self.disbanded && !matches!(op.body, OpBody::Keeper { .. }) {
            return Err(Rejection::Disbanded);
        }
        if self.is_banned(&op.author) {
            return Err(Rejection::AuthorBanned);
        }

        // The one operation of somebody who is not the owner yet: the new
        // owner's keeper, which puts a waiting handover in force.
        if let OpBody::Keeper { pubkey, serial, from_epoch, inbox } = &op.body {
            if !self.disbanded && self.pending_owner.as_ref() == Some(&op.author) {
                self.check_keeper(*serial, *from_epoch)?;
                self.keepers.push(KeeperRef { pubkey: pubkey.clone(), serial: *serial, from_epoch: *from_epoch, inbox: inbox.clone() });
                let old = std::mem::replace(&mut self.owner, op.author.clone());
                if let Some(m) = self.staff.get_mut(old.as_hex()) {
                    m.role = Role::Admin;
                }
                let new = self.entry(&op.author, op);
                new.role = Role::Owner;
                new.muted = false;
                new.muted_until = None;
                self.pending_owner = None;
                self.version += 1;
                return Ok(());
            }
        }

        let actor = self.role_of(&op.author).ok_or(Rejection::AuthorNotMember)?;
        let allow = |action: Action| if permits(actor, action) { Ok(()) } else { Err(Rejection::NotPermitted) };
        // The role of the one acted on: a reader the journal does not name
        // is a member.
        let target = |s: &Self, who: &PubKey| s.role_of(who).unwrap_or(Role::Member);

        match &op.body {
            OpBody::Create { .. } => return Err(Rejection::AlreadyCreated),
            OpBody::Settings { name, about, picture, history, relays, storage, post_mode: _, premod, schedule, comments, join } => {
                allow(Action::EditSettings)?;
                check_text(name.as_deref(), about.as_deref(), picture.as_deref())?;
                if let Some(r) = relays {
                    check_relays(r)?;
                }
                if let Some(s) = storage {
                    check_storage(s)?;
                }
                if let Some(s) = schedule {
                    s.check().map_err(|_| invalid("schedule"))?;
                }
                if let Some(p) = premod {
                    // Who counts as a newcomer is fixed when the rule is
                    // turned on and stays while it is on, whichever of its
                    // two shapes it takes; turned off, the line is forgotten.
                    match (self.premod, *p) {
                        (_, Premod::Off) => {
                            self.premod_since = None;
                            self.premod_since_at = None;
                        }
                        (Premod::Off, _) => {
                            self.premod_since = Some(self.version);
                            self.premod_since_at = Some(op.created_at);
                        }
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
                if let Some(h) = history {
                    self.history = *h;
                    // `everyone` reads as the channel does.
                    if let Some(s) = self.scopes.get_mut(SCOPE_EVERYONE) {
                        s.history = *h;
                    }
                }
                if let Some(r) = relays {
                    self.relays = r.clone();
                }
                if let Some(s) = storage {
                    self.storage = s.clone();
                }
                if let Some(s) = schedule {
                    self.schedule = if s.is_empty() { None } else { Some(s.clone()) };
                }
                if let Some(c) = comments {
                    self.comments = *c;
                }
                if let Some(j) = join {
                    self.join = *j;
                }
            }
            OpBody::SetRole { who, role } => {
                if self.is_banned(who) {
                    return Err(Rejection::TargetBanned);
                }
                let from = target(self, who);
                let action = match (from, *role) {
                    (Role::Member, Role::Author) | (Role::Author, Role::Member) => Action::SetAuthor(from),
                    _ => Action::SetRole { from, to: *role },
                };
                allow(action)?;
                // Raised to the staff: in the order of the journal from here.
                let raised = from < Role::Author && *role >= Role::Author;
                let pos = self.version;
                let m = self.entry(who, op);
                m.role = *role;
                if raised {
                    m.joined_pos = pos;
                }
                // Out of the staff, the choice of the quota goes: a new
                // appointment starts in the owner's, as every first one.
                if *role < Role::Author {
                    m.quota_own = false;
                }
                self.tidy(who);
            }
            OpBody::Ban { who, reason } => {
                if who == &op.author {
                    return Err(invalid("banning oneself"));
                }
                if reason.as_deref().is_some_and(|r| r != crate::marks::REASON_SPAM) {
                    return Err(invalid("reason"));
                }
                if self.is_banned(who) {
                    return Err(Rejection::TargetBanned);
                }
                allow(Action::Ban(target(self, who)))?;
                self.staff.remove(who.as_hex());
                self.banned.insert(who.as_hex().to_string());
                if self.pending_owner.as_ref() == Some(who) {
                    self.pending_owner = None;
                }
            }
            OpBody::Unban { who, reason } => {
                allow(Action::Unban)?;
                if reason.as_deref().is_some_and(|r| r != crate::marks::REASON_SPAM) {
                    return Err(invalid("reason"));
                }
                if !self.banned.remove(who.as_hex()) {
                    return Err(Rejection::NotBanned);
                }
            }
            OpBody::Remove { who } => {
                if who == &op.author {
                    return Err(invalid("removing oneself"));
                }
                if self.is_banned(who) {
                    return Err(Rejection::TargetBanned);
                }
                allow(Action::Remove(target(self, who)))?;
                self.staff.remove(who.as_hex());
                if self.pending_owner.as_ref() == Some(who) {
                    self.pending_owner = None;
                }
            }
            OpBody::SetMuted { who, muted, until } => {
                if self.is_banned(who) {
                    return Err(Rejection::TargetBanned);
                }
                allow(Action::Mute(target(self, who)))?;
                // An end belongs to a mute, and lies ahead of it: one that
                // ended before it was made would mean nothing anywhere.
                if until.is_some_and(|u| !*muted || u <= op.created_at) {
                    return Err(invalid("until"));
                }
                let m = self.entry(who, op);
                m.muted = *muted;
                m.muted_until = *until;
                self.tidy(who);
            }
            OpBody::SetTrusted { who, trusted } => {
                if self.is_banned(who) {
                    return Err(Rejection::TargetBanned);
                }
                allow(Action::Trust(target(self, who)))?;
                self.entry(who, op).trusted = *trusted;
                self.tidy(who);
            }
            OpBody::SetQuota { who, own } => {
                allow(Action::SetQuota)?;
                if self.is_banned(who) {
                    return Err(Rejection::TargetBanned);
                }
                // A choice for the staff alone, the owner's own uploads are
                // his anyway.
                if who == &self.owner || target(self, who) < Role::Author {
                    return Err(invalid("not of the staff"));
                }
                self.entry(who, op).quota_own = *own;
            }
            OpBody::SetLevels { who, levels } => {
                allow(Action::SetLevels)?;
                if self.is_banned(who) {
                    return Err(Rejection::TargetBanned);
                }
                // The base level is always held, and every level named
                // exists: a bit of a level that is not would give later
                // whatever a new level of that id gives.
                let known = self.levels.keys().fold(0u64, |acc, id| acc | (1u64 << id));
                if levels & 1 == 0 || levels & !known != 0 {
                    return Err(invalid("levels"));
                }
                self.entry(who, op).levels = *levels;
                self.tidy(who);
            }
            OpBody::ScopeAdd { scope_id, scheme, history, hidden } => {
                allow(Action::EditScopes)?;
                // Hiding a section is the owner's, as its look is (`scope_set`).
                if *hidden {
                    allow(Action::SetScope)?;
                }
                if !is_scope_id(scope_id) {
                    return Err(invalid("scope_id"));
                }
                if self.scopes.contains_key(scope_id) {
                    return Err(invalid("scope exists"));
                }
                if !matches!(scheme, SchemeId::SdV1 | SchemeId::FlatV1) {
                    return Err(invalid("scheme"));
                }
                if self.scopes.len() >= MAX_SCOPES {
                    return Err(invalid("scope limit"));
                }
                self.scopes.insert(scope_id.clone(), ScopeDef { scheme: *scheme, history: *history, hidden: *hidden });
            }
            OpBody::ScopeSet { scope_id, hidden } => {
                allow(Action::SetScope)?;
                let Some(def) = self.scopes.get_mut(scope_id) else { return Err(invalid("unknown scope")) };
                // `everyone` is everybody's: it is never hidden.
                if scope_id == SCOPE_EVERYONE && *hidden == Some(true) {
                    return Err(invalid("scope everyone"));
                }
                if let Some(h) = hidden {
                    def.hidden = *h;
                }
            }
            OpBody::ScopeDel { scope_id } => {
                allow(Action::DeleteScope)?;
                if scope_id == SCOPE_EVERYONE {
                    return Err(invalid("scope everyone"));
                }
                if self.scopes.remove(scope_id).is_none() {
                    return Err(invalid("unknown scope"));
                }
                for l in self.levels.values_mut() {
                    l.scopes.retain(|s| s != scope_id);
                }
            }
            OpBody::LevelSet { id, scopes, can } => {
                allow(Action::EditLevels)?;
                if *id > MAX_LEVEL_ID {
                    return Err(invalid("level id"));
                }
                let distinct: BTreeSet<&String> = scopes.iter().collect();
                if distinct.len() != scopes.len() || scopes.iter().any(|s| !self.scopes.contains_key(s)) {
                    return Err(invalid("level scopes"));
                }
                // The base level gives `everyone` whatever it names.
                let mut scopes = scopes.clone();
                if *id == BASE_LEVEL && !scopes.iter().any(|s| s == SCOPE_EVERYONE) {
                    scopes.insert(0, SCOPE_EVERYONE.to_string());
                }
                self.levels.insert(*id, LevelDef { scopes, can: *can });
            }
            OpBody::LevelDel { id } => {
                allow(Action::EditLevels)?;
                if *id == BASE_LEVEL {
                    return Err(invalid("base level"));
                }
                if self.levels.remove(id).is_none() {
                    return Err(invalid("unknown level"));
                }
                let bit = 1u64 << id;
                let holders: Vec<String> = self.staff.iter().filter(|(_, m)| m.levels & bit != 0).map(|(k, _)| k.clone()).collect();
                for k in holders {
                    if let Some(m) = self.staff.get_mut(&k) {
                        m.levels &= !bit;
                    }
                    if let Some(who) = PubKey::parse(&k) {
                        self.tidy(&who);
                    }
                }
            }
            OpBody::Keeper { pubkey, serial, from_epoch, inbox } => {
                allow(Action::Keeper)?;
                self.check_keeper(*serial, *from_epoch)?;
                self.keepers.push(KeeperRef { pubkey: pubkey.clone(), serial: *serial, from_epoch: *from_epoch, inbox: inbox.clone() });
            }
            OpBody::RotateNow { scope_id } => {
                allow(Action::RotateNow)?;
                if scope_id.as_deref().is_some_and(|s| s != SCOPE_EVERYONE && !is_hex(s, 32)) {
                    return Err(invalid("scope_id"));
                }
            }
            OpBody::RotateLink { link_epoch } => {
                allow(Action::RotateLink)?;
                if Some(*link_epoch) != self.link_epoch.checked_add(1) {
                    return Err(invalid("link epoch must grow by one"));
                }
                self.link_epoch = *link_epoch;
            }
            OpBody::Pin { post_id, pinned } => {
                allow(Action::Pin)?;
                if !is_hex(post_id, 64) {
                    return Err(invalid("post_id"));
                }
                if *pinned {
                    self.pinned.insert(post_id.clone());
                } else {
                    self.pinned.remove(post_id);
                }
            }
            OpBody::TransferOwnership { to } => {
                allow(Action::TransferOwnership)?;
                if to == &op.author {
                    return Err(invalid("transfer to oneself"));
                }
                if self.is_banned(to) {
                    return Err(Rejection::TargetBanned);
                }
                // Kept until the new owner's keeper; a later handover
                // replaces one that waits.
                self.pending_owner = Some(to.clone());
            }
            OpBody::Disband => {
                allow(Action::Disband)?;
                self.disbanded = true;
                self.pending_owner = None;
            }
        }
        self.version += 1;
        Ok(())
    }
}

// ─── The rules at a moment ──────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    Member,
    Muted,
    Outsider,
}

/// What the journal (and the snapshot) say of one person at one moment
/// (`rules_at`): whether they were in and could speak, what they were,
/// and the rules of writing in force then.
#[derive(Debug, PartialEq, Eq)]
pub struct Rules {
    pub standing: Standing,
    /// Of a mute in force: when it lifts by itself.
    pub muted_until: Option<i64>,
    pub role: Role,
    pub trusted: bool,
    /// What the levels held then let do (wire §14.1).
    pub can: LevelCan,
    /// The scopes the levels held then let post into (`post_scopes_by_levels`).
    pub post_scopes: BTreeSet<String>,
    comments: Comments,
    schedule: Option<Schedule>,
    premod: Premod,
    premod_since: Option<u64>,
    premod_since_at: Option<i64>,
    /// Of staff: where they were made staff.
    joined_pos: Option<u64>,
    /// Of a reader: their join by the snapshot.
    reader_since: Option<i64>,
}

impl Rules {
    /// Why this person may not write `content` at `at`, if they may not:
    /// not in, muted, or held by the rules of the channel (`gate`).
    pub fn closed(&self, content: Content, at: i64) -> Option<Closed> {
        match self.standing {
            Standing::Outsider => Some(Closed::NotMember),
            Standing::Muted => Some(Closed::Muted { until: self.muted_until }),
            Standing::Member => gate(self.role, self.can, content, self.comments, self.schedule.as_ref(), at).err(),
        }
    }

    /// Why a message written at `written` does not stand, judged on
    /// arrival: `self` holds at the head of the journal (who is in, banned,
    /// muted), `then` at the time it was written (the rules of the channel:
    /// comments, the schedule; the role then or now, whichever
    /// lets more). Judging the rules of the channel at the head would drop
    /// for good what was written in an open window, or before the rules
    /// were tightened, whenever it is caught up after a later operation.
    pub fn closed_written(&self, then: &Rules, content: Content, written: i64) -> Option<Closed> {
        match self.standing {
            Standing::Outsider => Some(Closed::NotMember),
            Standing::Muted => Some(Closed::Muted { until: self.muted_until }),
            Standing::Member => gate(self.role.max(then.role), self.can.union(then.can), content, then.comments, then.schedule.as_ref(), written).err(),
        }
    }

    /// Whether a post sealed in `scope_id` stands by the levels (2026-10-23):
    /// an author and above (then or now) posts wherever they belong; a
    /// reader only into a scope a posting level gave them, then or now,
    /// whichever lets more, as `closed_written` judges the role.
    pub fn post_scope_ok(&self, then: &Rules, scope_id: &str) -> bool {
        self.role.max(then.role) >= Role::Author || self.post_scopes.contains(scope_id) || then.post_scopes.contains(scope_id)
    }

    /// Do this person's messages wait for a moderator: the rule of
    /// `ChannelState::subject_to_premod`, as it stood then.
    pub fn subject_to_premod(&self) -> bool {
        if self.standing != Standing::Member || self.role.is_moderator() || self.trusted {
            return false;
        }
        match self.premod {
            Premod::Off => false,
            Premod::All => true,
            Premod::Newcomers => match (self.role >= Role::Author, self.joined_pos, self.reader_since) {
                (true, Some(pos), _) => self.premod_since.is_some_and(|since| pos > since),
                (false, _, Some(joined)) => self.premod_since_at.is_some_and(|since| joined > since),
                _ => false,
            },
        }
    }
}

/// What held for `who` at the moment `at` (wire §4: the time of a
/// message is max(its `created_at`, the head of the journal here)). The
/// roles and the rules of writing are those set by the operations dated
/// up to `at`; a ban or a removal dated up to `at` puts a reader out
/// unless the snapshot says they came in again after it; a mute with an
/// end lifts at that end, with no operation to say so. The places in the
/// journal (`joined_pos`, `premod_since`) are those of the applied order,
/// the same on every device. `reader`: what the snapshot says of them.
pub fn rules_at(log: &OpLog, who: &PubKey, reader: Option<&Reader>, at: i64) -> Rules {
    let mut rules = Rules {
        standing: Standing::Outsider,
        muted_until: None,
        role: Role::Member,
        trusted: false,
        can: LevelCan::BASE,
        post_scopes: BTreeSet::new(),
        comments: Comments::On,
        schedule: None,
        premod: Premod::Off,
        premod_since: None,
        premod_since_at: None,
        joined_pos: None,
        reader_since: reader.map(|r| r.since),
    };
    let mut staff = false;
    let mut banned = false;
    let mut out_at: Option<i64> = None;
    // A mute: since when, until when.
    let mut mute: Option<(i64, Option<i64>)> = None;
    // `at` found under a mute that ended: that mute's end.
    let mut under: Option<Option<i64>> = None;
    let mut owner: Option<PubKey> = None;
    let mut pending: Option<PubKey> = None;
    // The rights of each level then, and the levels of `who` then.
    let mut level_can: BTreeMap<u8, LevelCan> = BTreeMap::from([(BASE_LEVEL, LevelCan::BASE)]);
    // The scopes each level gave then: a reader posts only into those of a
    // level that let post (`post_scopes_by_levels`).
    let mut level_scopes: BTreeMap<u8, Vec<String>> = BTreeMap::from([(BASE_LEVEL, vec![SCOPE_EVERYONE.to_string()])]);
    let mut bits: u64 = 0;
    let end_mute = |mute: &mut Option<(i64, Option<i64>)>, under: &mut Option<Option<i64>>, end: i64| {
        if let Some((m, until)) = mute.take() {
            let stop = until.map_or(end, |u| u.min(end));
            if at >= m && at < stop && under.is_none() {
                *under = Some(until);
            }
        }
    };
    let applied = log.ordered().filter(|o| log.rejection(&o.id()).is_none());
    for (pos, op) in applied.enumerate() {
        let pos = pos as u64;
        let counts = op.created_at <= at;
        match &op.body {
            OpBody::Create { .. } => {
                owner = Some(op.author.clone());
                if &op.author == who && counts {
                    rules.role = Role::Owner;
                    staff = true;
                    rules.joined_pos = Some(pos);
                }
            }
            OpBody::SetRole { who: w, role } if w == who && counts => {
                if *role >= Role::Author && !staff {
                    rules.joined_pos = Some(pos);
                }
                staff = *role >= Role::Author;
                rules.role = *role;
            }
            OpBody::Ban { who: w, .. } | OpBody::Remove { who: w } if w == who => {
                end_mute(&mut mute, &mut under, op.created_at);
                if counts {
                    bits = 0;
                    staff = false;
                    rules.role = Role::Member;
                    rules.trusted = false;
                    out_at = Some(out_at.map_or(op.created_at, |t| t.max(op.created_at)));
                    banned |= matches!(op.body, OpBody::Ban { .. });
                }
            }
            OpBody::Unban { who: w, .. } if w == who && counts => banned = false,
            OpBody::SetMuted { who: w, muted, until } if w == who => {
                if *muted {
                    mute = match mute {
                        // A mute that had lifted by itself: a new one, and
                        // what was said under the old one stays hidden.
                        Some((m, Some(u))) if u <= op.created_at => {
                            if at >= m && at < u && under.is_none() {
                                under = Some(Some(u));
                            }
                            Some((op.created_at, *until))
                        }
                        // One in force keeps its start and takes the new end.
                        Some((m, _)) => Some((m, *until)),
                        None => Some((op.created_at, *until)),
                    };
                } else {
                    // A muted stretch that ended still hides what was said in it.
                    end_mute(&mut mute, &mut under, op.created_at);
                }
            }
            OpBody::SetTrusted { who: w, trusted } if w == who && counts => rules.trusted = *trusted,
            OpBody::SetLevels { who: w, levels } if w == who && counts => bits = *levels,
            OpBody::LevelSet { id, can, scopes } if counts => {
                level_can.insert(*id, *can);
                let mut scopes = scopes.clone();
                if *id == BASE_LEVEL && !scopes.iter().any(|s| s == SCOPE_EVERYONE) {
                    scopes.insert(0, SCOPE_EVERYONE.to_string());
                }
                level_scopes.insert(*id, scopes);
            }
            OpBody::LevelDel { id } if counts => {
                level_can.remove(id);
                level_scopes.remove(id);
                bits &= !(1u64 << id);
            }
            OpBody::ScopeDel { scope_id } if counts => {
                for s in level_scopes.values_mut() {
                    s.retain(|x| x != scope_id);
                }
            }
            OpBody::TransferOwnership { to } if counts => pending = Some(to.clone()),
            // The handover in force with the new owner's keeper (`apply`).
            OpBody::Keeper { .. } if counts && pending.as_ref() == Some(&op.author) => {
                if &op.author == who {
                    rules.role = Role::Owner;
                    staff = true;
                    rules.joined_pos = rules.joined_pos.or(Some(pos));
                } else if owner.as_ref() == Some(who) {
                    rules.role = Role::Admin;
                }
                owner = pending.take();
            }
            OpBody::Settings { premod, schedule, comments, .. } if counts => {
                if let Some(s) = schedule {
                    rules.schedule = if s.is_empty() { None } else { Some(s.clone()) };
                }
                if let Some(c) = comments {
                    rules.comments = *c;
                }
                if let Some(p) = premod {
                    match (rules.premod, *p) {
                        (_, Premod::Off) => {
                            rules.premod_since = None;
                            rules.premod_since_at = None;
                        }
                        (Premod::Off, _) => {
                            rules.premod_since = Some(pos);
                            rules.premod_since_at = Some(op.created_at);
                        }
                        _ => {}
                    }
                    rules.premod = *p;
                }
            }
            _ => {}
        }
    }
    // A mute still on at the end: `at` is under it while it lasts.
    if let Some((m, until)) = mute {
        if at >= m && until.is_none_or(|u| at < u) && under.is_none() {
            under = Some(until);
        }
    }
    rules.can = level_can.iter().filter(|(id, _)| (bits | 1) & (1u64 << *id) != 0).fold(LevelCan::NONE, |acc, (_, c)| acc.union(*c));
    rules.post_scopes = level_can
        .iter()
        .filter(|(id, c)| (bits | 1) & (1u64 << **id) != 0 && c.post)
        .flat_map(|(id, _)| level_scopes.get(id).cloned().unwrap_or_default())
        .collect();
    let as_reader = reader.is_some_and(|r| at >= r.since - JOIN_SLACK_SECS && out_at.is_none_or(|t| r.since > t));
    let inside = !banned && (staff || as_reader);
    if !staff && rules.role > Role::Member {
        rules.role = Role::Member;
    }
    if inside {
        match under {
            Some(until) => {
                rules.standing = Standing::Muted;
                rules.muted_until = until;
            }
            None => rules.standing = Standing::Member,
        }
    }
    rules
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::op::tests::{create_body, Settings, CID};
    use crate::op::OpId;

    pub(crate) const CHANNEL: &str = CID;

    pub(crate) fn pk(c: &str) -> PubKey {
        PubKey::parse(&c.repeat(64)[..64]).unwrap()
    }

    pub(crate) fn create(owner: &PubKey) -> Op {
        let mut body = create_body();
        if let OpBody::Create { name, .. } = &mut body {
            *name = " News ".into();
        }
        Op::new(CHANNEL, owner, vec![], 100, body)
    }

    /// An operation on top of "something": parents do not matter to `apply`.
    pub(crate) fn op(author: &PubKey, body: OpBody) -> Op {
        Op::new(CHANNEL, author, vec![OpId("p".into())], 200, body)
    }

    pub(crate) const READER: Reader = Reader { since: 150 };

    /// Owner `o`, admin `a`, moderator `m`, author `w`; `u` a reader the
    /// journal does not name.
    pub(crate) fn team() -> ChannelState {
        let (o, a, m, w) = (pk("0"), pk("a"), pk("b"), pk("e"));
        let mut s = ChannelState::genesis(&create(&o)).unwrap();
        s.apply(&op(&o, OpBody::SetRole { who: a, role: Role::Admin })).unwrap();
        s.apply(&op(&o, OpBody::SetRole { who: m, role: Role::Moderator })).unwrap();
        s.apply(&op(&o, OpBody::SetRole { who: w, role: Role::Author })).unwrap();
        s
    }

    #[test]
    fn the_owner_alone_says_whose_quota_the_staff_upload_into() {
        let mut s = team();
        let (o, a, m, w, u) = (pk("0"), pk("a"), pk("b"), pk("e"), pk("f"));
        // Everyone of the staff starts in the owner's.
        assert!(!s.quota_own(&w) && !s.quota_own(&m));
        // An admin may not; the owner may, for the staff alone.
        assert_eq!(s.apply(&op(&a, OpBody::SetQuota { who: w.clone(), own: true })), Err(Rejection::NotPermitted));
        s.apply(&op(&o, OpBody::SetQuota { who: w.clone(), own: true })).unwrap();
        assert!(s.quota_own(&w));
        assert!(s.apply(&op(&o, OpBody::SetQuota { who: u.clone(), own: true })).is_err(), "a reader");
        assert!(s.apply(&op(&o, OpBody::SetQuota { who: o.clone(), own: true })).is_err(), "the owner himself");
        // Out of the staff and back: the owner's again.
        s.apply(&op(&o, OpBody::SetRole { who: w.clone(), role: Role::Member })).unwrap();
        s.apply(&op(&o, OpBody::SetRole { who: w.clone(), role: Role::Author })).unwrap();
        assert!(!s.quota_own(&w));
        // The field travels only when set: a state of before reads.
        let json = serde_json::to_string(&s.staff[m.as_hex()]).unwrap();
        assert!(!json.contains("quota_own"), "{json}");
        // An operation of version 4: a build of 3 would keep it for an update.
        assert_eq!(OpBody::SetQuota { who: w, own: false }.version(), 4);
    }

    #[test]
    fn genesis() {
        let o = pk("0");
        let s = ChannelState::genesis(&create(&o)).unwrap();
        assert_eq!(s.name, "News");
        assert_eq!(s.role_of(&o), Some(Role::Owner));
        assert_eq!(s.keeper().unwrap().serial, 0);
        assert_eq!((s.join, s.history, s.link_epoch, s.version), (JoinMode::Open, History::Open, 0, 1));
        assert_eq!(s.relays.len(), 2);

        let bad = |f: &dyn Fn(&mut OpBody)| {
            let mut c = create(&o);
            f(&mut c.body);
            ChannelState::genesis(&c)
        };
        let set = |field: &'static str| move |b: &mut OpBody| {
            if let OpBody::Create { relays, storage, keeper, link_epoch, name, picture, scheme, .. } = b {
                match field {
                    "relays0" => relays.clear(),
                    "relays9" => *relays = (0..9).map(|i| format!("wss://r{i}.example")).collect(),
                    "relaydup" => *relays = vec!["wss://r.example".into(), "wss://r.example".into()],
                    "relayhttp" => *relays = vec!["https://r.example".into()],
                    "relayslash" => *relays = vec!["wss://r.example/".into()],
                    "storage0" => storage.clear(),
                    "storagehttp" => *storage = vec!["http://s.example".into()],
                    "storageslash" => *storage = vec!["https://s.example/".into()],
                    "serial" => keeper.serial = 1,
                    "from" => keeper.from_epoch = 0,
                    "link" => *link_epoch = 1,
                    "name" => *name = " ".into(),
                    "longname" => *name = "x".repeat(101),
                    "picture" => *picture = "http://p.example/a.jpg".into(),
                    "scheme" => *scheme = SchemeId::FlatV1,
                    _ => unreachable!(),
                }
            }
        };
        for field in [
            "relays0", "relays9", "relaydup", "relayhttp", "relayslash", "storage0", "storagehttp", "storageslash", "serial", "from", "link", "name", "longname", "picture",
            "scheme",
        ] {
            assert!(matches!(bad(&set(field)), Err(Rejection::Invalid { .. })), "{field}");
        }
        let mut short = create(&o);
        short.community_id = "short".into();
        assert!(matches!(ChannelState::genesis(&short), Err(Rejection::Invalid { .. })));
        assert_eq!(ChannelState::genesis(&op(&o, OpBody::Disband)), Err(Rejection::NotCreated));
    }

    #[test]
    fn rejected_operations_change_nothing() {
        let (o, a, m, w, u, x) = (pk("0"), pk("a"), pk("b"), pk("e"), pk("c"), pk("d"));
        let mut s = team();
        let before = s.clone();
        let hex64 = "cd".repeat(32);
        let cases: Vec<(Op, Rejection)> = vec![
            // A reader writes no operations, whatever they are.
            (op(&u, OpBody::SetMuted { who: x.clone(), muted: true, until: None }), Rejection::AuthorNotMember),
            (op(&u, OpBody::Pin { post_id: hex64.clone(), pinned: true }), Rejection::AuthorNotMember),
            // An author moderates nobody.
            (op(&w, OpBody::SetMuted { who: u.clone(), muted: true, until: None }), Rejection::NotPermitted),
            (op(&w, OpBody::Ban { who: u.clone(), reason: None }), Rejection::NotPermitted),
            (op(&w, OpBody::Pin { post_id: hex64.clone(), pinned: true }), Rejection::NotPermitted),
            (op(&w, OpBody::RotateNow { scope_id: None }), Rejection::NotPermitted),
            // Below oneself.
            (op(&m, OpBody::SetMuted { who: m.clone(), muted: true, until: None }), Rejection::NotPermitted),
            (op(&m, OpBody::Ban { who: a.clone(), reason: None }), Rejection::NotPermitted),
            (op(&m, OpBody::Remove { who: a.clone() }), Rejection::NotPermitted),
            (op(&a, OpBody::Remove { who: o.clone() }), Rejection::NotPermitted),
            (op(&m, OpBody::SetTrusted { who: a.clone(), trusted: true }), Rejection::NotPermitted),
            // Roles: admin+, an admin is the owner's to make.
            (op(&m, OpBody::SetRole { who: u.clone(), role: Role::Author }), Rejection::NotPermitted),
            (op(&m, OpBody::SetRole { who: u.clone(), role: Role::Admin }), Rejection::NotPermitted),
            (op(&a, OpBody::SetRole { who: a.clone(), role: Role::Member }), Rejection::NotPermitted),
            (op(&o, OpBody::SetRole { who: a.clone(), role: Role::Owner }), Rejection::NotPermitted),
            (op(&o, OpBody::SetRole { who: w.clone(), role: Role::Author }), Rejection::NotPermitted),
            // Settings: admin+.
            (op(&m, Settings { name: Some("x".into()), ..Settings::default() }.body()), Rejection::NotPermitted),
            // The owner's own.
            (op(&a, OpBody::Keeper { pubkey: x.clone(), serial: 1, from_epoch: 5, inbox: None }), Rejection::NotPermitted),
            (op(&a, OpBody::TransferOwnership { to: u.clone() }), Rejection::NotPermitted),
            (op(&a, OpBody::Disband), Rejection::NotPermitted),
            (op(&a, OpBody::Unban { who: x.clone(), reason: None }), Rejection::NotBanned),
            (op(&o, create_body()), Rejection::AlreadyCreated),
        ];
        for (operation, want) in cases {
            assert_eq!(s.apply(&operation), Err(want), "{:?} by {:?}", operation.body, operation.author);
            assert_eq!(s, before, "state must not change on rejection");
        }
        let invalid_cases: Vec<Op> = vec![
            op(&o, OpBody::Ban { who: o.clone(), reason: None }),
            op(&m, OpBody::Ban { who: u.clone(), reason: Some("rude".into()) }),
            op(&m, OpBody::Remove { who: m.clone() }),
            op(&m, OpBody::SetMuted { who: u.clone(), muted: true, until: Some(200) }),
            op(&m, OpBody::SetMuted { who: u.clone(), muted: false, until: Some(900) }),
            op(&o, OpBody::Keeper { pubkey: x.clone(), serial: 2, from_epoch: 5, inbox: None }),
            op(&o, OpBody::Keeper { pubkey: x.clone(), serial: 1, from_epoch: 1, inbox: None }),
            op(&m, OpBody::RotateLink { link_epoch: 2 }),
            op(&m, OpBody::RotateLink { link_epoch: 0 }),
            op(&m, OpBody::RotateNow { scope_id: Some("nobody".into()) }),
            op(&m, OpBody::Pin { post_id: "short".into(), pinned: true }),
            op(&o, OpBody::TransferOwnership { to: o.clone() }),
            op(&a, Settings { name: Some("  ".into()), ..Settings::default() }.body()),
            op(&a, Settings { picture: Some("http://x".into()), ..Settings::default() }.body()),
            op(&a, Settings { relays: Some(vec![]), ..Settings::default() }.body()),
            op(&a, Settings { storage: Some(vec!["ftp://s".into()]), ..Settings::default() }.body()),
            op(&a, Settings { schedule: Some(Schedule { tz: "UTC".into(), days: vec![8], from: "09:00".into(), to: "18:00".into() }), ..Settings::default() }.body()),
        ];
        for operation in invalid_cases {
            assert!(matches!(s.apply(&operation), Err(Rejection::Invalid { .. })), "{:?}", operation.body);
            assert_eq!(s, before, "state must not change on rejection");
        }
        let mut other = op(&o, OpBody::RotateNow { scope_id: None });
        other.community_id = "ff".repeat(32);
        assert_eq!(s.apply(&other), Err(Rejection::WrongChannel));
        let mut future = op(&o, OpBody::RotateNow { scope_id: None });
        future.v = OP_VERSION + 1;
        assert_eq!(s.apply(&future), Err(Rejection::UnsupportedVersion));
        let mut zero = future.clone();
        zero.v = 0;
        assert_eq!(s.apply(&zero), Err(Rejection::UnsupportedVersion));
        let orphan = Op::new(CHANNEL, &o, vec![], 1, OpBody::RotateNow { scope_id: None });
        assert!(matches!(s.apply(&orphan), Err(Rejection::Invalid { .. })));
        let wide = Op::new(CHANNEL, &o, (0..17).map(|i| OpId(format!("{i:02}"))).collect(), 1, OpBody::RotateNow { scope_id: None });
        assert!(matches!(s.apply(&wide), Err(Rejection::Invalid { .. })), "more than 16 parents");
        assert_eq!(s, before);
    }

    /// Every operation of the table of wire §3 by the lowest role that may
    /// make it, and what it does.
    #[test]
    fn every_operation_applies_by_its_rule() {
        let (o, a, m, w, u, x) = (pk("0"), pk("a"), pk("b"), pk("e"), pk("c"), pk("d"));
        let mut s = team();
        // settings: admin.
        s.apply(&op(
            &a,
            Settings {
                name: Some("Renamed".into()),
                about: Some("about".into()),
                picture: Some("https://p.example/c.jpg".into()),
                history: Some(History::FromJoin),
                relays: Some(vec!["wss://r3.example".into()]),
                storage: Some(vec!["https://s3.example/media".into()]),
                post_mode: Some(PostMode::Authors),
                premod: Some(Premod::Newcomers),
                schedule: None,
                comments: Some(Comments::Off),
                join: Some(JoinMode::Request),
            }
            .body(),
        ))
        .unwrap();
        assert_eq!((s.name.as_str(), s.about.as_str(), s.history, s.join), ("Renamed", "about", History::FromJoin, JoinMode::Request));
        assert_eq!((s.post_mode, s.premod, s.comments), (PostMode::Authors, Premod::Newcomers, Comments::Off));
        assert_eq!((s.relays.len(), s.storage[0].as_str()), (1, "https://s3.example/media"));
        // set_role: admin makes an author; `member` takes the role, the entry goes.
        s.apply(&op(&a, OpBody::SetRole { who: u.clone(), role: Role::Author })).unwrap();
        assert_eq!(s.role_of(&u), Some(Role::Author));
        assert_eq!(s.staff_of(&u).unwrap().added_by, a);
        s.apply(&op(&a, OpBody::SetRole { who: u.clone(), role: Role::Member })).unwrap();
        assert!(!s.is_member(&u), "a plain reader is not in the journal");
        // set_muted, set_trusted, set_levels: moderator, on a reader the
        // journal did not name.
        s.apply(&op(&m, OpBody::SetMuted { who: u.clone(), muted: true, until: Some(900) })).unwrap();
        assert_eq!(s.staff_of(&u).unwrap().role, Role::Member);
        assert!(s.staff_of(&u).unwrap().muted_at(800));
        s.apply(&op(&m, OpBody::SetTrusted { who: w.clone(), trusted: true })).unwrap();
        s.apply(&op(&a, OpBody::LevelSet { id: 2, scopes: vec![], can: LevelCan::BASE })).unwrap();
        s.apply(&op(&m, OpBody::SetLevels { who: x.clone(), levels: 5 })).unwrap();
        assert_eq!(s.staff_of(&x).unwrap().levels, 5);
        s.apply(&op(&m, OpBody::SetMuted { who: u.clone(), muted: false, until: None })).unwrap();
        assert!(!s.is_member(&u), "nothing left to say of u");
        // ban / unban: moderator, below oneself; a ban takes the role.
        s.apply(&op(&m, OpBody::Ban { who: w.clone(), reason: Some("spam".into()) })).unwrap();
        assert!(s.is_banned(&w) && !s.is_member(&w));
        assert_eq!(s.apply(&op(&w, OpBody::Pin { post_id: "cd".repeat(32), pinned: true })), Err(Rejection::AuthorBanned), "a banned author is refused");
        assert_eq!(s.apply(&op(&m, OpBody::SetMuted { who: w.clone(), muted: true, until: None })), Err(Rejection::TargetBanned));
        s.apply(&op(&m, OpBody::Unban { who: w.clone(), reason: None })).unwrap();
        assert!(!s.is_banned(&w) && s.role_of(&w).is_none(), "unbanned, the role is not back");
        // remove: moderator, a reader too (the keeper revokes the leaf).
        s.apply(&op(&m, OpBody::Remove { who: u.clone() })).unwrap();
        // keeper: owner, serial +1, a later start.
        s.apply(&op(&o, OpBody::Keeper { pubkey: x.clone(), serial: 1, from_epoch: 40, inbox: None })).unwrap();
        assert_eq!(s.keeper().unwrap().from_epoch, 40);
        assert_eq!(s.keeper_for_epoch(39).unwrap().serial, 0);
        assert_eq!(s.keeper_for_epoch(40).unwrap().serial, 1);
        assert_eq!(s.range_end(0), Some(40));
        assert_eq!(s.range_end(1), None);
        // rotate_now, rotate_link, pin: moderator.
        s.apply(&op(&m, OpBody::RotateNow { scope_id: Some(SCOPE_EVERYONE.into()) })).unwrap();
        s.apply(&op(&m, OpBody::RotateLink { link_epoch: 1 })).unwrap();
        assert_eq!(s.link_epoch, 1);
        let post = "cd".repeat(32);
        s.apply(&op(&m, OpBody::Pin { post_id: post.clone(), pinned: true })).unwrap();
        assert!(s.pinned.contains(&post));
        s.apply(&op(&m, OpBody::Pin { post_id: post.clone(), pinned: false })).unwrap();
        assert!(s.pinned.is_empty());
        // disband: owner; after it only a keeper.
        let version = s.version;
        s.apply(&op(&o, OpBody::Disband)).unwrap();
        assert!(s.disbanded);
        assert_eq!(s.version, version + 1);
        assert_eq!(s.apply(&op(&m, OpBody::RotateNow { scope_id: None })), Err(Rejection::Disbanded));
        assert_eq!(s.apply(&op(&o, Settings { name: Some("x".into()), ..Settings::default() }.body())), Err(Rejection::Disbanded));
        assert_eq!(s.apply(&op(&a, OpBody::Keeper { pubkey: x.clone(), serial: 2, from_epoch: 50, inbox: None })), Err(Rejection::NotPermitted));
        s.apply(&op(&o, OpBody::Keeper { pubkey: x.clone(), serial: 2, from_epoch: 50, inbox: None })).unwrap();
        assert_eq!(s.can_post_at(&o, None, 300), Err(Closed::NotMember));
    }

    /// The scopes and the levels of wire §14.1: who makes them, what a
    /// reader belongs to by them, and the rules that hold them.
    #[test]
    fn scopes_and_levels_by_their_rules() {
        let (o, a, m, w, u) = (pk("0"), pk("a"), pk("b"), pk("e"), pk("c"));
        let mut s = team();
        let (gold, staff) = ("ab".repeat(16), "cd".repeat(16));
        assert_eq!(s.scopes.keys().collect::<Vec<_>>(), [SCOPE_EVERYONE], "everyone, made with the channel");
        assert_eq!(s.levels[&BASE_LEVEL].can, LevelCan::BASE);
        // scope_add: admin and above; the scheme of a scope is sd-v1 or flat-v1.
        let add = |id: &str, scheme| OpBody::ScopeAdd { scope_id: id.to_string(), scheme, history: History::FromJoin, hidden: false };
        assert_eq!(s.apply(&op(&m, add(&gold, SchemeId::SdV1))), Err(Rejection::NotPermitted));
        assert_eq!(s.apply(&op(&a, add(&gold, SchemeId::CsV1))), Err(invalid("scheme")));
        assert_eq!(s.apply(&op(&a, add("everyone", SchemeId::SdV1))), Err(invalid("scope_id")));
        s.apply(&op(&a, add(&gold, SchemeId::SdV1))).unwrap();
        s.apply(&op(&a, add(&staff, SchemeId::FlatV1))).unwrap();
        assert_eq!(s.apply(&op(&a, add(&gold, SchemeId::SdV1))), Err(invalid("scope exists")));
        // level_set: admin; scopes that exist, once each.
        let level = |id: u8, scopes: Vec<String>, post: bool| OpBody::LevelSet { id, scopes, can: LevelCan { comment: true, discuss: true, post } };
        assert_eq!(s.apply(&op(&m, level(1, vec![gold.clone()], false))), Err(Rejection::NotPermitted));
        assert_eq!(s.apply(&op(&a, level(1, vec!["ef".repeat(16)], false))), Err(invalid("level scopes")));
        assert_eq!(s.apply(&op(&a, level(1, vec![gold.clone(), gold.clone()], false))), Err(invalid("level scopes")));
        assert_eq!(s.apply(&op(&a, level(64, vec![], false))), Err(invalid("level id")));
        s.apply(&op(&a, level(1, vec![gold.clone()], true))).unwrap();
        s.apply(&op(&a, level(2, vec![gold.clone()], false))).unwrap();
        // set_levels: moderator; the base level always, only levels that exist.
        assert_eq!(s.apply(&op(&m, OpBody::SetLevels { who: u.clone(), levels: 2 })), Err(invalid("levels")));
        assert_eq!(s.apply(&op(&m, OpBody::SetLevels { who: u.clone(), levels: 1 | 1 << 5 })), Err(invalid("levels")));
        assert_eq!(s.scopes_of(&u), BTreeSet::from([SCOPE_EVERYONE.to_string()]));
        s.apply(&op(&m, OpBody::SetLevels { who: u.clone(), levels: 0b111 })).unwrap();
        assert_eq!(s.scopes_of(&u), BTreeSet::from([SCOPE_EVERYONE.to_string(), gold.clone()]));
        assert!(s.can_of(&u).post, "level 1 lets post");
        assert_eq!(s.can_post_at(&u, Some(&READER), 300), Ok(()));
        // Staff from moderator up belong to every scope; an author by levels.
        assert_eq!(s.scopes_of(&m).len(), 3);
        assert_eq!(s.scopes_of(&w), BTreeSet::from([SCOPE_EVERYONE.to_string()]));
        // level_del: admin; the base level stays; holders lose the bit.
        assert_eq!(s.apply(&op(&a, OpBody::LevelDel { id: BASE_LEVEL })), Err(invalid("base level")));
        s.apply(&op(&a, OpBody::LevelDel { id: 1 })).unwrap();
        assert_eq!(s.levels_of(&u), 0b101);
        assert!(s.in_scope(&u, &gold), "level 2 still gives gold");
        assert!(!s.can_of(&u).post);
        assert_eq!(s.can_post_at(&u, Some(&READER), 300), Err(Closed::AuthorsOnly));
        s.apply(&op(&a, OpBody::LevelDel { id: 2 })).unwrap();
        assert!(!s.is_member(&u), "the base level alone is nothing to say");
        // The base level may take comments away from readers.
        s.apply(&op(&a, OpBody::LevelSet { id: BASE_LEVEL, scopes: vec![], can: LevelCan::NONE })).unwrap();
        assert_eq!(s.levels[&BASE_LEVEL].scopes, [SCOPE_EVERYONE], "it gives everyone whatever it names");
        assert_eq!(s.can_comment_at(&u, Some(&READER), 300), Err(Closed::Level));
        assert_eq!(s.can_comment_at(&w, None, 300), Ok(()), "an author comments by role");
        // scope_del: owner and admins (2026-10-25), never everyone; the levels let it go.
        s.apply(&op(&a, level(3, vec![staff.clone()], false))).unwrap();
        assert_eq!(s.apply(&op(&m, OpBody::ScopeDel { scope_id: staff.clone() })), Err(Rejection::NotPermitted), "a moderator");
        assert_eq!(s.apply(&op(&o, OpBody::ScopeDel { scope_id: SCOPE_EVERYONE.into() })), Err(invalid("scope everyone")));
        s.apply(&op(&a, OpBody::ScopeDel { scope_id: staff.clone() })).unwrap();
        assert!(s.levels[&3].scopes.is_empty());
        // At most 16 scopes, everyone among them.
        for i in 0..14u8 {
            s.apply(&op(&a, add(&format!("{i:02x}").repeat(16), SchemeId::SdV1))).unwrap();
        }
        assert_eq!(s.scopes.len(), MAX_SCOPES);
        assert_eq!(s.apply(&op(&a, add(&"ee".repeat(16), SchemeId::SdV1))), Err(invalid("scope limit")));
    }

    /// `scope_set` and a hidden `scope_add`: the owner's and the admins'
    /// (2026-10-25; the owner's alone from 2026-10-23), not a moderator's;
    /// `everyone` is never hidden; `hidden` none leaves it as it is.
    #[test]
    fn the_look_of_a_section_is_the_managers() {
        let (o, a, m) = (pk("0"), pk("a"), pk("b"));
        let mut s = team();
        let (gold, secret) = ("ab".repeat(16), "cd".repeat(16));
        let add = |id: &str, hidden| OpBody::ScopeAdd { scope_id: id.to_string(), scheme: SchemeId::SdV1, history: History::Open, hidden };
        let set = |id: &str, hidden| OpBody::ScopeSet { scope_id: id.to_string(), hidden };
        s.apply(&op(&a, add(&gold, false))).unwrap();
        assert_eq!(s.apply(&op(&m, add(&secret, true))), Err(Rejection::NotPermitted), "a moderator makes no sections");
        s.apply(&op(&a, add(&secret, true))).unwrap();
        assert!(s.scopes[&secret].hidden && !s.scopes[&gold].hidden);
        assert_eq!(s.apply(&op(&m, set(&gold, None))), Err(Rejection::NotPermitted), "a rename is a manager's");
        s.apply(&op(&a, set(&gold, None))).unwrap();
        assert_eq!(s.apply(&op(&o, set(&"ef".repeat(16), None))), Err(invalid("unknown scope")));
        assert_eq!(s.apply(&op(&o, set(SCOPE_EVERYONE, Some(true)))), Err(invalid("scope everyone")));
        s.apply(&op(&o, set(SCOPE_EVERYONE, None))).unwrap();
        s.apply(&op(&o, set(&gold, Some(true)))).unwrap();
        s.apply(&op(&o, set(&secret, None))).unwrap();
        assert!(s.scopes[&gold].hidden && s.scopes[&secret].hidden, "none leaves it as it is");
        s.apply(&op(&o, set(&secret, Some(false)))).unwrap();
        assert!(!s.scopes[&secret].hidden);
    }

    /// The rights of the levels in the replay of the journal: what the
    /// levels let at the moment of a message.
    #[test]
    fn the_rights_of_levels_at_the_moment() {
        let (mut log, [o, _, m, _]) = journal();
        let u = pk("c");
        let gold = "ab".repeat(16);
        at(&mut log, &o, 300, OpBody::ScopeAdd { scope_id: gold.clone(), scheme: SchemeId::SdV1, history: History::Open, hidden: false });
        at(&mut log, &o, 310, OpBody::LevelSet { id: 1, scopes: vec![gold], can: LevelCan { comment: true, discuss: true, post: true } });
        at(&mut log, &m, 320, OpBody::SetLevels { who: u.clone(), levels: 0b11 });
        at(&mut log, &o, 400, OpBody::LevelDel { id: 1 });
        let r = Reader { since: 150 };
        assert_eq!(rules_at(&log, &u, Some(&r), 315).closed(Content::Post, 315), Some(Closed::AuthorsOnly));
        assert_eq!(rules_at(&log, &u, Some(&r), 350).closed(Content::Post, 350), None, "level 1 let post then");
        assert_eq!(rules_at(&log, &u, Some(&r), 450).closed(Content::Post, 450), Some(Closed::AuthorsOnly));
        let now = rules_at(&log, &u, Some(&r), 450);
        let then = rules_at(&log, &u, Some(&r), 350);
        assert_eq!(now.closed_written(&then, Content::Post, 350), None, "written while the level let it, it stands");
    }

    #[test]
    fn a_handover_waits_for_the_keeper_of_the_new_owner() {
        let (o, a, m, x) = (pk("0"), pk("a"), pk("b"), pk("d"));
        let mut s = team();
        s.apply(&op(&o, OpBody::TransferOwnership { to: m.clone() })).unwrap();
        assert_eq!(s.pending_owner, Some(m.clone()));
        // Taken and kept: nothing changes yet.
        assert_eq!((s.owner.clone(), s.role_of(&m)), (o.clone(), Some(Role::Moderator)));
        assert_eq!(s.apply(&op(&m, OpBody::Disband)), Err(Rejection::NotPermitted), "not the owner yet");
        assert_eq!(s.apply(&op(&a, OpBody::Keeper { pubkey: x.clone(), serial: 1, from_epoch: 9, inbox: None })), Err(Rejection::NotPermitted));
        // The new owner's keeper must be the next series too.
        assert!(matches!(s.apply(&op(&m, OpBody::Keeper { pubkey: x.clone(), serial: 5, from_epoch: 9, inbox: None })), Err(Rejection::Invalid { .. })));
        assert_eq!(s.pending_owner, Some(m.clone()));
        s.apply(&op(&m, OpBody::Keeper { pubkey: x.clone(), serial: 1, from_epoch: 9, inbox: None })).unwrap();
        assert_eq!((s.owner.clone(), s.role_of(&m), s.role_of(&o)), (m.clone(), Some(Role::Owner), Some(Role::Admin)));
        assert_eq!(s.pending_owner, None);
        assert_eq!(s.keeper().unwrap().pubkey, x);
        assert_eq!(s.apply(&op(&o, OpBody::Disband)), Err(Rejection::NotPermitted), "the former owner is an admin");

        // A handover to a reader the journal does not name, then banned
        // before it took effect: the handover is gone.
        let mut s = team();
        let r = pk("c");
        s.apply(&op(&o, OpBody::TransferOwnership { to: r.clone() })).unwrap();
        s.apply(&op(&a, OpBody::Ban { who: r.clone(), reason: None })).unwrap();
        assert_eq!(s.pending_owner, None);
        assert_eq!(s.apply(&op(&r, OpBody::Keeper { pubkey: x.clone(), serial: 1, from_epoch: 9, inbox: None })), Err(Rejection::AuthorBanned));
        // A reader who takes a handover becomes the owner by its keeper.
        let mut s = team();
        s.apply(&op(&o, OpBody::TransferOwnership { to: r.clone() })).unwrap();
        s.apply(&op(&r, OpBody::Keeper { pubkey: x.clone(), serial: 1, from_epoch: 9, inbox: None })).unwrap();
        assert_eq!(s.role_of(&r), Some(Role::Owner));
        // A disband drops a handover that waits.
        let mut s = team();
        s.apply(&op(&o, OpBody::TransferOwnership { to: m.clone() })).unwrap();
        s.apply(&op(&o, OpBody::Disband)).unwrap();
        assert_eq!(s.apply(&op(&m, OpBody::Keeper { pubkey: x, serial: 1, from_epoch: 9, inbox: None })), Err(Rejection::NotPermitted), "a keeper only of the owner now");
    }

    #[test]
    fn deleting_messages() {
        let (o, a, m, w, u, x) = (pk("0"), pk("a"), pk("b"), pk("e"), pk("c"), pk("d"));
        let s = team();
        let r = Some(&READER);
        assert!(s.can_delete_message(&u, r, &u), "own message");
        assert!(!s.can_delete_message(&u, r, &m));
        assert!(s.can_delete_message(&m, None, &u));
        assert!(s.can_delete_message(&m, None, &w), "a moderator over an author");
        assert!(!s.can_delete_message(&w, None, &u), "an author over nobody");
        assert!(!s.can_delete_message(&m, None, &a));
        assert!(s.can_delete_message(&a, None, &m));
        assert!(!s.can_delete_message(&a, None, &o));
        assert!(s.can_delete_message(&o, None, &a));
        assert!(!s.can_delete_message(&x, None, &x), "outsiders delete nothing");
        assert_eq!(s.managers().len(), 2);
        assert_eq!(s.moderators().len(), 3);
        assert_eq!(s.uploaders().len(), 4);
    }

    /// Unix seconds of a wall-clock moment in UTC.
    pub(crate) fn utc(y: i16, mo: i8, d: i8, h: i8, mi: i8) -> i64 {
        jiff::civil::date(y, mo, d).at(h, mi, 0, 0).to_zoned(jiff::tz::TimeZone::UTC).unwrap().timestamp().as_second()
    }

    #[test]
    fn a_timed_mute_lifts_by_itself() {
        let (o, m, u) = (pk("0"), pk("b"), pk("c"));
        let r = Some(&READER);
        let mut s = team();
        // Every operation of `op` is dated 200.
        s.apply(&op(&m, OpBody::SetMuted { who: u.clone(), muted: true, until: Some(500) })).unwrap();
        assert_eq!(s.staff_of(&u).unwrap().muted_until, Some(500));
        assert_eq!(s.can_comment_at(&u, r, 300), Err(Closed::Muted { until: Some(500) }));
        assert_eq!(s.can_comment_at(&u, r, 499), Err(Closed::Muted { until: Some(500) }));
        assert_eq!(s.can_comment_at(&u, r, 500), Ok(()), "lifted by the clock, without an operation");
        assert!(s.staff_of(&u).unwrap().muted && !s.staff_of(&u).unwrap().muted_at(500));
        // Muted again, with no end.
        s.apply(&op(&m, OpBody::SetMuted { who: u.clone(), muted: true, until: None })).unwrap();
        assert_eq!(s.can_comment_at(&u, r, 10_000), Err(Closed::Muted { until: None }));
        s.apply(&op(&m, OpBody::SetMuted { who: u.clone(), muted: false, until: None })).unwrap();
        assert_eq!(s.can_comment_at(&u, r, 10_000), Ok(()));
        // The channel handed to a muted author lifts the mute, its end too.
        s.apply(&op(&o, OpBody::SetRole { who: u.clone(), role: Role::Author })).unwrap();
        s.apply(&op(&m, OpBody::SetMuted { who: u.clone(), muted: true, until: Some(900) })).unwrap();
        s.apply(&op(&o, OpBody::TransferOwnership { to: u.clone() })).unwrap();
        s.apply(&op(&u, OpBody::Keeper { pubkey: pk("f"), serial: 1, from_epoch: 2, inbox: None })).unwrap();
        assert_eq!((s.staff_of(&u).unwrap().muted, s.staff_of(&u).unwrap().muted_until), (false, None));
    }

    #[test]
    fn readers_post_and_comment_by_the_snapshot() {
        let (m, w, u, x) = (pk("b"), pk("e"), pk("c"), pk("d"));
        let mut s = team();
        let r = Some(&READER);
        assert_eq!(s.can_post_at(&u, r, 300), Err(Closed::AuthorsOnly), "posts are by authors; a reader comments");
        assert_eq!(s.can_comment_at(&u, r, 300), Ok(()));
        assert_eq!(s.can_post_at(&x, None, 300), Err(Closed::NotMember), "not in the snapshot, not in the journal");
        assert_eq!(s.can_post_at(&w, None, 300), Ok(()), "an author is in by the journal");
        s.apply(&op(&pk("a"), Settings { post_mode: Some(PostMode::All), ..Settings::default() }.body())).unwrap();
        assert_eq!(s.can_post_at(&u, r, 300), Err(Closed::AuthorsOnly), "`post_mode: all` of an older build changes nothing");
        assert_eq!(s.post_mode, PostMode::Authors);
        assert_eq!(s.can_comment_at(&u, r, 300), Ok(()), "comments stay open");
        assert_eq!(s.can_post_at(&w, None, 300), Ok(()));
        assert_eq!(s.can_post_at(&m, None, 300), Ok(()));
        s.apply(&op(&pk("a"), Settings { comments: Some(Comments::Off), ..Settings::default() }.body())).unwrap();
        assert_eq!(s.can_comment_at(&u, r, 300), Err(Closed::CommentsOff));
        assert_eq!(s.can_comment_at(&w, None, 300), Err(Closed::CommentsOff));
        assert_eq!(s.can_comment_at(&m, None, 300), Ok(()), "moderators are never held");
        s.apply(&op(&m, OpBody::Ban { who: u.clone(), reason: None })).unwrap();
        assert_eq!(s.can_comment_at(&u, r, 300), Err(Closed::Banned), "the snapshot is not above the journal");
    }

    #[test]
    fn posting_rules_and_the_schedule() {
        let (m, w, u) = (pk("b"), pk("e"), pk("c"));
        let a = pk("a");
        let r = Some(&READER);
        let mut s = team();
        // Fridays, 09:00 to 18:00 UTC; 2026-10-16 is a Friday.
        let sched = Schedule { tz: "UTC".into(), days: vec![5], from: "09:00".into(), to: "18:00".into() };
        s.apply(&op(&a, Settings { schedule: Some(sched.clone()), ..Settings::default() }.body())).unwrap();
        assert_eq!(s.schedule.as_ref(), Some(&sched));
        assert!(!s.schedule_tz_unknown());
        assert_eq!(s.can_comment_at(&u, r, utc(2026, 10, 16, 12, 0)), Ok(()));
        assert_eq!(s.can_comment_at(&u, r, utc(2026, 10, 16, 18, 0)), Err(Closed::Schedule { opens_at: utc(2026, 10, 23, 9, 0) }));
        assert_eq!(s.can_comment_at(&w, None, utc(2026, 10, 16, 18, 0)), Err(Closed::Schedule { opens_at: utc(2026, 10, 23, 9, 0) }));
        assert_eq!(s.can_post_at(&m, None, utc(2026, 10, 16, 18, 0)), Ok(()), "a moderator is never held by the hours");
        // A mute is said before the hours.
        s.apply(&op(&m, OpBody::SetMuted { who: u.clone(), muted: true, until: None })).unwrap();
        assert_eq!(s.can_comment_at(&u, r, utc(2026, 10, 16, 18, 0)), Err(Closed::Muted { until: None }));
        s.apply(&op(&m, OpBody::SetMuted { who: u.clone(), muted: false, until: None })).unwrap();
        // A zone this build does not know: read in UTC, and the state says so.
        let far = Schedule { tz: "Mars/Olympus".into(), ..sched.clone() };
        s.apply(&op(&a, Settings { schedule: Some(far.clone()), ..Settings::default() }.body())).unwrap();
        assert!(s.schedule_tz_unknown());
        assert_eq!(s.can_comment_at(&u, r, utc(2026, 10, 16, 8, 0)), Err(Closed::Schedule { opens_at: utc(2026, 10, 16, 9, 0) }));
        // No days: the schedule is gone.
        s.apply(&op(&a, Settings { schedule: Some(Schedule { days: vec![], ..sched }), ..Settings::default() }.body())).unwrap();
        assert_eq!(s.schedule, None);
        assert_eq!(s.can_comment_at(&u, r, utc(2026, 10, 16, 18, 0)), Ok(()));
    }

    #[test]
    fn premod_counts_newcomers_by_the_journal_and_the_snapshot() {
        let (o, a, m, w, u, x) = (pk("0"), pk("a"), pk("b"), pk("e"), pk("c"), pk("d"));
        let mut s = team();
        assert_eq!(s.staff_of(&o).unwrap().joined_pos, 0);
        assert_eq!(s.staff_of(&a).unwrap().joined_pos, 1);
        let old_reader = Reader { since: 150 };
        let new_reader = Reader { since: 250 };
        for who in [&o, &a, &m, &w] {
            assert!(!s.subject_to_premod(who, None));
        }
        // Newcomers: staff made after the rule, readers who joined after it.
        s.apply(&op(&a, Settings { premod: Some(Premod::Newcomers), ..Settings::default() }.body())).unwrap();
        let since = s.premod_since.unwrap();
        assert_eq!(since, s.version - 1, "the place of the operation that set the rule");
        assert_eq!(s.premod_since_at, Some(200));
        assert!(!s.subject_to_premod(&w, None), "an author from before");
        assert!(!s.subject_to_premod(&u, Some(&old_reader)), "a reader from before");
        assert!(s.subject_to_premod(&x, Some(&new_reader)), "a reader who came after");
        // x is made an author by an operation dated before the rule: the
        // order of the journal counts, not anybody's clock.
        let mut early = op(&a, OpBody::SetRole { who: x.clone(), role: Role::Author });
        early.created_at = 150;
        s.apply(&early).unwrap();
        assert!(s.staff_of(&x).unwrap().joined_pos > since);
        assert!(s.subject_to_premod(&x, None));
        // Trusted, or raised: no longer held.
        s.apply(&op(&m, OpBody::SetTrusted { who: x.clone(), trusted: true })).unwrap();
        assert!(!s.subject_to_premod(&x, None));
        s.apply(&op(&m, OpBody::SetTrusted { who: x.clone(), trusted: false })).unwrap();
        s.apply(&op(&o, OpBody::SetRole { who: x.clone(), role: Role::Moderator })).unwrap();
        assert!(!s.subject_to_premod(&x, None));
        // Everyone: the old ones too, never a moderator.
        s.apply(&op(&a, Settings { premod: Some(Premod::All), ..Settings::default() }.body())).unwrap();
        assert_eq!(s.premod_since, Some(since), "the line stays while the rule is on");
        assert!(s.subject_to_premod(&u, Some(&old_reader)) && s.subject_to_premod(&w, None) && !s.subject_to_premod(&m, None));
        // Off: nobody.
        s.apply(&op(&a, Settings { premod: Some(Premod::Off), ..Settings::default() }.body())).unwrap();
        assert_eq!((s.premod_since, s.premod_since_at), (None, None));
        assert!(!s.subject_to_premod(&u, Some(&new_reader)));
        assert!(!s.subject_to_premod(&pk("f"), None), "an outsider is nobody's to hold");
    }

    // ─── rules_at ───────────────────────────────────────────────────────────

    use crate::log::Insert;

    /// A journal with the team of `team()`, every operation dated as asked.
    fn journal() -> (OpLog, [PubKey; 4]) {
        let who = [pk("0"), pk("a"), pk("b"), pk("e")];
        let [o, a, m, w] = who.clone();
        let mut log = OpLog::create(create(&o)).unwrap();
        at(&mut log, &o, 120, OpBody::SetRole { who: a, role: Role::Admin });
        at(&mut log, &o, 121, OpBody::SetRole { who: m, role: Role::Moderator });
        at(&mut log, &o, 122, OpBody::SetRole { who: w, role: Role::Author });
        (log, who)
    }

    fn at(log: &mut OpLog, author: &PubKey, when: i64, body: OpBody) {
        let op = log.next(author, when, body);
        assert_eq!(log.insert(op.clone()), Insert::Applied, "{:?}", op.body);
    }

    #[test]
    fn a_timed_mute_lifts_by_itself_in_the_replay() {
        let (mut log, [_, _, m, _]) = journal();
        let u = pk("c");
        let r = Some(&READER);
        at(&mut log, &m, 300, OpBody::SetMuted { who: u.clone(), muted: true, until: Some(600) });
        assert_eq!(rules_at(&log, &u, r, 299).standing, Standing::Member);
        assert_eq!(rules_at(&log, &u, r, 300).closed(Content::Comment, 300), Some(Closed::Muted { until: Some(600) }));
        assert_eq!(rules_at(&log, &u, r, 599).standing, Standing::Muted);
        assert_eq!(rules_at(&log, &u, r, 600).closed(Content::Comment, 600), None, "lifted with no operation to say so");
        // Lifted early by a moderator: what was said under the mute stays hidden.
        at(&mut log, &m, 450, OpBody::SetMuted { who: u.clone(), muted: false, until: None });
        assert_eq!(rules_at(&log, &u, r, 400).standing, Standing::Muted);
        assert_eq!(rules_at(&log, &u, r, 500).standing, Standing::Member);
        // Muted again with an end, then for good after that end had passed.
        at(&mut log, &m, 700, OpBody::SetMuted { who: u.clone(), muted: true, until: Some(800) });
        at(&mut log, &m, 900, OpBody::SetMuted { who: u.clone(), muted: true, until: None });
        assert_eq!(rules_at(&log, &u, r, 750).closed(Content::Post, 750), Some(Closed::Muted { until: Some(800) }));
        assert_eq!(rules_at(&log, &u, r, 850).standing, Standing::Member);
        assert_eq!(rules_at(&log, &u, r, 950).closed(Content::Post, 950), Some(Closed::Muted { until: None }));
        // The moderator was never muted, and an outsider is outside.
        assert_eq!(rules_at(&log, &m, None, 950).closed(Content::Post, 950), None);
        assert_eq!(rules_at(&log, &pk("f"), None, 950).closed(Content::Comment, 950), Some(Closed::NotMember));
    }

    #[test]
    fn the_rules_are_those_in_force_at_the_moment() {
        let (mut log, [o, a, m, w]) = journal();
        let u = pk("c");
        let r = Some(&READER);
        at(&mut log, &a, 400, Settings { post_mode: Some(PostMode::All), ..Settings::default() }.body());
        assert_eq!(rules_at(&log, &u, r, 399).closed(Content::Post, 399), Some(Closed::AuthorsOnly), "posts are by authors");
        assert_eq!(rules_at(&log, &u, r, 400).closed(Content::Post, 400), Some(Closed::AuthorsOnly));
        assert_eq!(rules_at(&log, &w, None, 400).closed(Content::Post, 400), None);
        // A ban ends the stay at its second; a reader back after it (a
        // later join in the snapshot) is in again once unbanned.
        at(&mut log, &m, 500, OpBody::Ban { who: u.clone(), reason: None });
        assert_eq!(rules_at(&log, &u, r, 499).standing, Standing::Member);
        assert_eq!(rules_at(&log, &u, r, 500).standing, Standing::Outsider);
        at(&mut log, &m, 600, OpBody::Unban { who: u.clone(), reason: None });
        assert_eq!(rules_at(&log, &u, r, 700).standing, Standing::Outsider, "the old join is before the ban");
        assert_eq!(rules_at(&log, &u, Some(&Reader { since: 650 }), 700).standing, Standing::Member);
        // An author removed is out by the journal; a reader by the snapshot.
        at(&mut log, &a, 800, OpBody::Remove { who: w.clone() });
        assert_eq!(rules_at(&log, &w, None, 799).role, Role::Author);
        assert_eq!(rules_at(&log, &w, None, 800).standing, Standing::Outsider);
        // A handover is in force from the new owner's keeper.
        at(&mut log, &o, 900, OpBody::TransferOwnership { to: m.clone() });
        assert_eq!(rules_at(&log, &m, None, 950).role, Role::Moderator);
        at(&mut log, &m, 1000, OpBody::Keeper { pubkey: pk("f"), serial: 1, from_epoch: 3, inbox: None });
        assert_eq!(rules_at(&log, &m, None, 1000).role, Role::Owner);
        assert_eq!(rules_at(&log, &o, None, 1000).role, Role::Admin);
        assert_eq!(rules_at(&log, &o, None, 999).role, Role::Owner);
    }

    #[test]
    fn premod_in_the_replay() {
        let (mut log, [_, a, _, w]) = journal();
        at(&mut log, &a, 400, Settings { premod: Some(Premod::Newcomers), ..Settings::default() }.body());
        let x = pk("d");
        assert!(!rules_at(&log, &pk("c"), Some(&Reader { since: 300 }), 500).subject_to_premod());
        assert!(rules_at(&log, &x, Some(&Reader { since: 450 }), 500).subject_to_premod());
        assert!(!rules_at(&log, &w, None, 500).subject_to_premod(), "an author made before");
        at(&mut log, &a, 410, OpBody::SetRole { who: x.clone(), role: Role::Author });
        assert!(rules_at(&log, &x, None, 500).subject_to_premod(), "an author made after");
    }
}
