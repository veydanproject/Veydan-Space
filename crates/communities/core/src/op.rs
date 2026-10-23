// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from crates/messenger/groups/src/op.rs at 1f2ef31d, then changed for channels.

//! Operations: the only way the journal of a channel changes
//! (internal/communities-wire.md §3).
//!
//! An operation names the channel, its author, the operations its author
//! had seen (`parents`), and what to do. Its id is the SHA-256 of its
//! canonical JSON, so the same operation has the same id everywhere and
//! cannot be altered without becoming a different one. The layout is the
//! one of groups word for word, without their `key` and `proof`: keys are
//! the keeper's (keydist), and readers join through the keeper, not the
//! journal.

use crate::roles::Role;
pub use crate::schedule::Schedule;
use keydist::SchemeId;
use messenger_core::PubKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The newest format of operations this build reads; part of the hashed
/// content. A newer one is kept for an update within
/// `wire::AHEAD_WINDOW` (`wire::verify_op`). Version 2 brings the scopes
/// and the levels (wire §14.1): a build of version 1 keeps them for an
/// update instead of refusing what it cannot read. Version 3 (2026-10-23)
/// brings `scope_set` and the hidden section (`scope_add` with `hidden`);
/// version 4 (2026-10-23) `set_quota`, whose quota the staff upload into.
pub const OP_VERSION: u32 = 4;

/// The parents an operation names at most: the newest heads its author
/// knows (`OpLog::next`). More would let one operation grow without bound.
pub const MAX_PARENTS: usize = 16;

/// The heads of a journal a keeper names (`channel.issue`, a snapshot):
/// more than `MAX_PARENTS` when many operations were made beside each
/// other, never without bound.
pub const MAX_HEADS: usize = 256;

/// Who may post, as an operation names it: every reader, or authors and
/// above only. Stage 1 reads either as `Authors` (wire §3, 2026-10-22):
/// the field stays on the wire, a post is by an author or above.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostMode {
    #[default]
    All,
    Authors,
}

/// Whose messages wait for a moderator before the channel sees them:
/// nobody's, the newcomers' (those who joined after the rule was set),
/// or everyone's below a moderator. Trusted people are never held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Premod {
    #[default]
    Off,
    Newcomers,
    All,
}

/// How a reader gets in: by the link alone, by a request a moderator
/// answers, or only by an invitation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JoinMode {
    Open,
    Request,
    Invite,
}

/// How much of the past a newcomer reads: everything, or from the first
/// epoch after the join (keydist `History`, the words of the wire).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum History {
    #[default]
    Open,
    FromJoin,
}

impl History {
    /// The policy of the keeper this history means.
    pub fn keydist(self) -> keydist::History {
        match self {
            History::Open => keydist::History::Open,
            History::FromJoin => keydist::History::FromJoin,
        }
    }
}

/// Whether readers comment under posts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Comments {
    #[default]
    On,
    Off,
}

/// What a level lets the readers who hold it do (wire §14.1): the owner
/// gives each level its rights, none are built in. Staff write by their
/// role whatever their levels say.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelCan {
    /// Comment under the posts of the scopes the level gives.
    pub comment: bool,
    /// Take part in the discussions of a community (a later stage; kept).
    pub discuss: bool,
    /// Write posts into the scopes the level gives, as an author does.
    pub post: bool,
}

impl LevelCan {
    /// The rights of the base level until the owner sets others: what a
    /// reader of stage 1 could do (comment, not post).
    pub const BASE: Self = Self { comment: true, discuss: true, post: false };
    pub const NONE: Self = Self { comment: false, discuss: false, post: false };

    /// What either of two sets of rights lets do.
    pub fn union(self, other: Self) -> Self {
        Self { comment: self.comment || other.comment, discuss: self.discuss || other.discuss, post: self.post || other.post }
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OpId(pub String);

impl std::fmt::Debug for OpId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OpId({}…)", &self.0[..8.min(self.0.len())])
    }
}

/// Identifier of an epoch key: the hex SHA-256 of the key bytes (first 16
/// bytes), the `k` tag of sealed content (wire §1, as groups' `KeyId`).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KeyId(pub String);

impl KeyId {
    pub fn of(key: &[u8]) -> Self {
        Self(hex::encode(&Sha256::digest(key)[..16]))
    }
}

impl std::fmt::Debug for KeyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "KeyId({})", &self.0[..8.min(self.0.len())])
    }
}

/// The key of the keeper of one series (wire §3 `keeper`): the series
/// `serial` publishes epochs from `from_epoch` until the next series
/// begins.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeeperRef {
    pub pubkey: PubKey,
    pub serial: u32,
    pub from_epoch: u32,
    /// Whose key the notes to this keeper go to (wire §3, 2026-10-22):
    /// `channel.join`, `rebundle`, `missed`, `leave`, the keeper's copy of
    /// `channel.admit`, `keeper.master`. Absent (journals before it): the
    /// keeper's own key. The keeper in the app of stage 1 names the
    /// owner, whose connection the relays serve the notes to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inbox: Option<PubKey>,
}

impl KeeperRef {
    /// The key notes to this keeper are wrapped to.
    pub fn inbox(&self) -> &PubKey {
        self.inbox.as_ref().unwrap_or(&self.pubkey)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum OpBody {
    /// First operation of a channel; its author is the owner.
    Create {
        join: JoinMode,
        name: String,
        #[serde(default)]
        about: String,
        #[serde(default)]
        picture: String,
        history: History,
        relays: Vec<String>,
        storage: Vec<String>,
        scheme: SchemeId,
        /// Always `serial: 0, from_epoch: 1`.
        keeper: KeeperRef,
        /// Always 0.
        link_epoch: u32,
    },
    /// `None` leaves a field as it is.
    Settings {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        about: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        picture: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        history: Option<History>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        relays: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        storage: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        post_mode: Option<PostMode>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        premod: Option<Premod>,
        /// A schedule without days takes the schedule away.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schedule: Option<Schedule>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        comments: Option<Comments>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        join: Option<JoinMode>,
    },
    /// `member` takes the role away; the person stays a reader at the
    /// keeper.
    SetRole { who: PubKey, role: Role },
    Ban {
        who: PubKey,
        /// `spam` or nothing.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Unban {
        who: PubKey,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// Out without a ban: the keeper revokes the leaf.
    Remove { who: PubKey },
    /// `until`: the moment the mute lifts by itself (unix seconds, after
    /// the operation's own time); none for a mute that stays until lifted.
    SetMuted {
        who: PubKey,
        muted: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        until: Option<i64>,
    },
    /// Someone whose messages are never held for moderation.
    SetTrusted { who: PubKey, trusted: bool },
    /// Whose quota a member of the staff uploads the files of posts into
    /// (version 4, wire §9.2, the owner): the owner's plan (`own: false`, as
    /// every member of the staff starts) or the member's own.
    SetQuota { who: PubKey, own: bool },
    /// The levels of a person (wire §14.1): a bit per level id, bit 0 (the
    /// base level) always set. The keeper lets them into the scopes the
    /// levels give and out of those no level gives any more.
    SetLevels { who: PubKey, levels: u64 },
    /// A new scope (a section of the channel) of its own keys (wire
    /// §14.1). Its title is sealed (`t:"meta"` in the feed of `everyone`),
    /// not here: only members read it. `hidden` (version 3, written only
    /// when set): a section whose title and look are sealed in its own
    /// feed, so that only its members know what it is.
    ScopeAdd {
        scope_id: String,
        scheme: SchemeId,
        history: History,
        #[serde(default, skip_serializing_if = "is_false")]
        hidden: bool,
    },
    /// The look of a section changes (version 3, the owner): a fresh sealed
    /// meta binds to this operation, so that the title, the icon, the
    /// photo, the colour, the order and the note change by it; `hidden`
    /// (none: as it is) moves the meta into the section's own feed or back
    /// into that of `everyone`.
    ScopeSet {
        scope_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hidden: Option<bool>,
    },
    /// A scope goes; the levels that gave it no longer do.
    ScopeDel { scope_id: String },
    /// A level made or changed: the scopes it gives and what it lets do.
    /// Its name is sealed like the title of a scope.
    LevelSet { id: u8, scopes: Vec<String>, can: LevelCan },
    /// A level goes, and with it from everybody who held it.
    LevelDel { id: u8 },
    /// The owner delegates the keeper to a key of a new series; `inbox`:
    /// whose key its notes go to (`KeeperRef::inbox`).
    Keeper {
        pubkey: PubKey,
        serial: u32,
        from_epoch: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        inbox: Option<PubKey>,
    },
    /// A request to the keeper for an epoch now.
    RotateNow {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope_id: Option<String>,
    },
    /// A new link secret (wire §10); the old link stops admitting.
    RotateLink { link_epoch: u32 },
    Pin { post_id: String, pinned: bool },
    /// Kept from stage 1, in force from stage 3: only after a `keeper` of
    /// the new owner (`ChannelState::apply`).
    TransferOwnership { to: PubKey },
    Disband,
}

impl OpBody {
    /// The oldest format version that holds this body. Every body of the
    /// first version is 1; a later shape names its own.
    pub fn version(&self) -> u32 {
        match self {
            Self::SetQuota { .. } => 4,
            Self::ScopeAdd { hidden: true, .. } | Self::ScopeSet { .. } => 3,
            Self::ScopeAdd { .. } | Self::ScopeDel { .. } | Self::LevelSet { .. } | Self::LevelDel { .. } => 2,
            _ => 1,
        }
    }

    /// The person the operation is about, for the `p` tag of its event.
    pub fn subject(&self) -> Option<&PubKey> {
        match self {
            Self::SetRole { who, .. }
            | Self::Ban { who, .. }
            | Self::Unban { who, .. }
            | Self::Remove { who }
            | Self::SetMuted { who, .. }
            | Self::SetTrusted { who, .. }
            | Self::SetQuota { who, .. }
            | Self::SetLevels { who, .. } => Some(who),
            Self::TransferOwnership { to } => Some(to),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Op {
    pub v: u32,
    pub community_id: String,
    pub author: PubKey,
    /// Heads of the journal the author knew, sorted, at most
    /// `MAX_PARENTS`. Empty only for `Create`.
    pub parents: Vec<OpId>,
    /// Author's clock; shown to people, never used for ordering.
    pub created_at: i64,
    #[serde(flatten)]
    pub body: OpBody,
}

impl Op {
    pub fn new(community_id: &str, author: &PubKey, mut parents: Vec<OpId>, created_at: i64, body: OpBody) -> Self {
        parents.sort();
        parents.dedup();
        Self { v: body.version(), community_id: community_id.to_string(), author: author.clone(), parents, created_at, body }
    }

    /// Canonical form: compact JSON of the struct, fields in declaration
    /// order. No maps, so there is nothing whose order could vary.
    pub fn canonical(&self) -> String {
        // Strings, numbers, sequences and enums only: serializing cannot fail.
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn id(&self) -> OpId {
        OpId(hex::encode(Sha256::digest(self.canonical().as_bytes())))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const CID: &str = "00000000000000000000000000000000000000000000000000000000000000aa";

    fn pk(c: &str) -> PubKey {
        PubKey::parse(&c.repeat(64)[..64]).unwrap()
    }

    /// The canonical form of wire §3, and its id (also in
    /// tests/vectors.json).
    #[test]
    fn canonical_form_golden_vector() {
        let op = Op::new(
            CID,
            &pk("a"),
            vec![OpId("22".into()), OpId("11".into()), OpId("22".into())],
            1_760_000_000,
            OpBody::Ban { who: pk("b"), reason: Some("spam".into()) },
        );
        assert_eq!(op.parents, vec![OpId("11".into()), OpId("22".into())], "sorted, no duplicates");
        assert_eq!(
            op.canonical(),
            format!(
                r#"{{"v":1,"community_id":"{CID}","author":"{}","parents":["11","22"],"created_at":1760000000,"op":"ban","who":"{}","reason":"spam"}}"#,
                "a".repeat(64),
                "b".repeat(64)
            )
        );
        let back: Op = serde_json::from_str(&op.canonical()).unwrap();
        assert_eq!(back, op);
        assert_eq!(back.id(), op.id());
        assert_eq!(op.id().0, GOLDEN_BAN_ID);
    }

    /// SHA-256 of the canonical form above, by an independent tool.
    pub(crate) const GOLDEN_BAN_ID: &str = "f0be3cdfaddf2596615dceb0c03c3648c58395153ebd9104606c7ff2d6a867ab";

    #[test]
    fn create_has_the_layout_of_the_wire() {
        let op = Op::new(CID, &pk("a"), vec![], 1, create_body());
        assert_eq!(
            op.canonical(),
            format!(
                r#"{{"v":1,"community_id":"{CID}","author":"{a}","parents":[],"created_at":1,"op":"create","join":"open","name":"News","about":"","picture":"","history":"open","relays":["wss://r1.example","wss://r2.example"],"storage":["https://s1.example","https://s2.example"],"scheme":"sd-v1","keeper":{{"pubkey":"{k}","serial":0,"from_epoch":1}},"link_epoch":0}}"#,
                a = "a".repeat(64),
                k = "f".repeat(64)
            )
        );
    }

    /// The operations of the scopes and the levels (wire §14.1, 2026-10-22):
    /// ids only, no name or title (those are sealed), version 2.
    #[test]
    fn scopes_and_levels_have_the_layout_of_the_wire() {
        let s = "ab".repeat(16);
        let op = Op::new(CID, &pk("a"), vec![OpId("11".into())], 1, OpBody::ScopeAdd { scope_id: s.clone(), scheme: SchemeId::SdV1, history: History::Open, hidden: false });
        assert_eq!(
            op.canonical(),
            format!(r#"{{"v":2,"community_id":"{CID}","author":"{a}","parents":["11"],"created_at":1,"op":"scope_add","scope_id":"{s}","scheme":"sd-v1","history":"open"}}"#, a = "a".repeat(64))
        );
        let op = Op::new(CID, &pk("a"), vec![OpId("11".into())], 1, OpBody::LevelSet { id: 2, scopes: vec![s.clone()], can: LevelCan::BASE });
        assert_eq!(
            op.canonical(),
            format!(
                r#"{{"v":2,"community_id":"{CID}","author":"{a}","parents":["11"],"created_at":1,"op":"level_set","id":2,"scopes":["{s}"],"can":{{"comment":true,"discuss":true,"post":false}}}}"#,
                a = "a".repeat(64)
            )
        );
        assert_eq!(Op::new(CID, &pk("a"), vec![], 1, OpBody::SetLevels { who: pk("b"), levels: 1 }).v, 1, "set_levels was in the first version");
    }

    /// The look of a section (wire §14.1, 2026-10-23): `scope_set` and a
    /// hidden `scope_add` are version 3; a plain `scope_add` keeps the
    /// layout and the version of before.
    #[test]
    fn scope_set_and_a_hidden_section_are_version_three() {
        let s = "ab".repeat(16);
        let a = "a".repeat(64);
        let op = Op::new(CID, &pk("a"), vec![OpId("11".into())], 1, OpBody::ScopeAdd { scope_id: s.clone(), scheme: SchemeId::SdV1, history: History::Open, hidden: true });
        assert_eq!(
            op.canonical(),
            format!(r#"{{"v":3,"community_id":"{CID}","author":"{a}","parents":["11"],"created_at":1,"op":"scope_add","scope_id":"{s}","scheme":"sd-v1","history":"open","hidden":true}}"#)
        );
        let op = Op::new(CID, &pk("a"), vec![OpId("11".into())], 1, OpBody::ScopeSet { scope_id: s.clone(), hidden: None });
        assert_eq!(op.canonical(), format!(r#"{{"v":3,"community_id":"{CID}","author":"{a}","parents":["11"],"created_at":1,"op":"scope_set","scope_id":"{s}"}}"#));
        let op = Op::new(CID, &pk("a"), vec![], 1, OpBody::ScopeSet { scope_id: s, hidden: Some(false) });
        assert!(op.canonical().ends_with(r#""hidden":false}"#), "{}", op.canonical());
    }

    pub(crate) fn create_body() -> OpBody {
        OpBody::Create {
            join: JoinMode::Open,
            name: "News".into(),
            about: String::new(),
            picture: String::new(),
            history: History::Open,
            relays: vec!["wss://r1.example".into(), "wss://r2.example".into()],
            storage: vec!["https://s1.example".into(), "https://s2.example".into()],
            scheme: SchemeId::SdV1,
            keeper: KeeperRef { pubkey: pk("f"), serial: 0, from_epoch: 1, inbox: None },
            link_epoch: 0,
        }
    }

    #[test]
    fn any_change_is_a_different_operation() {
        let base = Op::new(CID, &pk("a"), vec![OpId("11".into())], 1, OpBody::Disband);
        let mut other = base.clone();
        other.created_at = 2;
        assert_ne!(base.id(), other.id());
        let mut other = base.clone();
        other.author = pk("b");
        assert_ne!(base.id(), other.id());
        assert_ne!(base.id(), Op::new(&"bb".repeat(32), &pk("a"), vec![OpId("11".into())], 1, OpBody::Disband).id());
    }

    #[test]
    fn every_body_of_the_table_reads_back() {
        let bodies = vec![
            create_body(),
            Settings { name: Some("n".into()), post_mode: Some(PostMode::Authors), comments: Some(Comments::Off), ..Settings::default() }.body(),
            OpBody::SetRole { who: pk("b"), role: Role::Author },
            OpBody::Ban { who: pk("b"), reason: None },
            OpBody::Unban { who: pk("b"), reason: None },
            OpBody::Remove { who: pk("b") },
            OpBody::SetMuted { who: pk("b"), muted: true, until: Some(9) },
            OpBody::SetTrusted { who: pk("b"), trusted: true },
            OpBody::SetLevels { who: pk("b"), levels: 3 },
            OpBody::Keeper { pubkey: pk("e"), serial: 1, from_epoch: 10, inbox: None },
            OpBody::RotateNow { scope_id: Some("everyone".into()) },
            OpBody::RotateLink { link_epoch: 1 },
            OpBody::Pin { post_id: "cd".repeat(32), pinned: true },
            OpBody::TransferOwnership { to: pk("b") },
            OpBody::Disband,
            OpBody::ScopeAdd { scope_id: "ab".repeat(16), scheme: SchemeId::FlatV1, history: History::FromJoin, hidden: false },
            OpBody::ScopeSet { scope_id: "ab".repeat(16), hidden: Some(true) },
            OpBody::ScopeDel { scope_id: "ab".repeat(16) },
            OpBody::LevelSet { id: 3, scopes: vec!["everyone".into(), "ab".repeat(16)], can: LevelCan { comment: true, discuss: false, post: true } },
            OpBody::LevelDel { id: 3 },
            OpBody::SetQuota { who: pk("b"), own: true },
        ];
        let names = [
            "create", "settings", "set_role", "ban", "unban", "remove", "set_muted", "set_trusted", "set_levels", "keeper", "rotate_now",
            "rotate_link", "pin", "transfer_ownership", "disband", "scope_add", "scope_set", "scope_del", "level_set", "level_del", "set_quota",
        ];
        for (body, name) in bodies.into_iter().zip(names) {
            let op = Op::new(CID, &pk("a"), vec![OpId("11".into())], 1, body);
            let json = op.canonical();
            assert!(json.contains(&format!(r#""op":"{name}""#)), "{json}");
            let back: Op = serde_json::from_str(&json).unwrap();
            assert_eq!(back, op, "{name}");
        }
        // What groups had and channels do not: refused as unknown.
        for gone in ["join", "admit", "leave", "rotate_key", "edit_settings"] {
            let json = format!(r#"{{"v":1,"community_id":"{CID}","author":"{}","parents":["11"],"created_at":1,"op":"{gone}"}}"#, "a".repeat(64));
            assert!(serde_json::from_str::<Op>(&json).is_err(), "{gone}");
        }
    }

    #[test]
    fn absent_fields_are_not_written() {
        let op = Op::new(CID, &pk("a"), vec![], 1, Settings { name: Some("n".into()), ..Settings::default() }.body());
        let json = op.canonical();
        for word in ["about", "post_mode", "schedule", "premod", "comments", "relays", "join\""] {
            assert!(!json.contains(word), "{word} in {json}");
        }
        let json = Op::new(CID, &pk("a"), vec![], 1, OpBody::SetMuted { who: pk("b"), muted: false, until: None }).canonical();
        assert!(!json.contains("until"), "{json}");
        let json = Op::new(CID, &pk("a"), vec![], 1, OpBody::Ban { who: pk("b"), reason: None }).canonical();
        assert!(!json.contains("reason"), "{json}");
    }

    /// The fields of `Settings` as a struct, so that a test names the
    /// ones it sets and leaves the rest (a variant has no `..default`).
    #[derive(Default)]
    pub(crate) struct Settings {
        pub(crate) name: Option<String>,
        pub(crate) about: Option<String>,
        pub(crate) picture: Option<String>,
        pub(crate) history: Option<History>,
        pub(crate) relays: Option<Vec<String>>,
        pub(crate) storage: Option<Vec<String>>,
        pub(crate) post_mode: Option<PostMode>,
        pub(crate) premod: Option<Premod>,
        pub(crate) schedule: Option<Schedule>,
        pub(crate) comments: Option<Comments>,
        pub(crate) join: Option<JoinMode>,
    }

    impl Settings {
        pub(crate) fn body(self) -> OpBody {
            OpBody::Settings {
                name: self.name,
                about: self.about,
                picture: self.picture,
                history: self.history,
                relays: self.relays,
                storage: self.storage,
                post_mode: self.post_mode,
                premod: self.premod,
                schedule: self.schedule,
                comments: self.comments,
                join: self.join,
            }
        }
    }

    #[test]
    fn key_id_is_derived_from_the_key() {
        let a = KeyId::of(&[1u8; 32]);
        assert_eq!(a, KeyId::of(&[1u8; 32]));
        assert_ne!(a, KeyId::of(&[2u8; 32]));
        assert_eq!(a.0.len(), 32);
    }
}
