// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Operations: the only way a group changes.
//!
//! An operation names the group, its author, the operations its author
//! had seen (`parents`), and what to do. Its id is the SHA-256 of its
//! canonical JSON, so the same operation has the same id everywhere and
//! cannot be altered without becoming a different one.

use crate::roles::Role;
pub use crate::schedule::Schedule;
use messenger_core::PubKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The newest format of operations this build reads; part of the hashed
/// content. An operation carries the oldest version that holds it
/// (`OpBody::needs_v2`): what a device of version 1 can read it reads as
/// before, byte for byte, and what it cannot it keeps for after its
/// update (`wire::verify_op`).
///
/// 2: a mute with an end (`SetMuted::until`), `SetTrusted`, and the
/// posting rules of `EditSettings` (`post_mode`, `post_schedule`, `premod`).
pub const OP_VERSION: u32 = 2;

/// Who may post: everyone, or moderators and above only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostMode {
    #[default]
    All,
    Managers,
}

/// Whose messages wait for a moderator before the group sees them:
/// nobody's, the newcomers' (those who joined after the rule was set),
/// or everyone's below a moderator. Trusted members are never held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Premod {
    #[default]
    Off,
    Newcomers,
    All,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OpId(pub String);

impl std::fmt::Debug for OpId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OpId({}…)", &self.0[..8.min(self.0.len())])
    }
}

/// Identifier of a group key: the hex SHA-256 of the key bytes (first 16
/// bytes). Keys are told apart by id, never by a counter: two keys made at
/// the same moment by two admins are simply two keys.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupKind {
    /// Anyone with the link joins by themselves; history is always there.
    Public,
    /// A manager admits every member and hands them the key.
    Private,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum OpBody {
    /// First operation of a group; its author is the owner.
    Create {
        kind: GroupKind,
        name: String,
        #[serde(default)]
        about: String,
        #[serde(default)]
        picture: String,
        /// Private groups: do newcomers get the old keys. Public: ignored.
        #[serde(default)]
        history_for_new: bool,
    },
    /// A manager admits someone (request approved, invitation accepted).
    Admit { who: PubKey },
    /// Public groups: the author adds themselves.
    Join,
    Leave,
    Remove { who: PubKey },
    Ban { who: PubKey },
    Unban { who: PubKey },
    SetRole { who: PubKey, role: Role },
    /// `until`: the moment the mute lifts by itself (unix seconds, after
    /// the operation's own time); none for a mute that stays until lifted.
    SetMuted {
        who: PubKey,
        muted: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        until: Option<i64>,
    },
    /// A member whose messages are never held for moderation.
    SetTrusted { who: PubKey, trusted: bool },
    /// `None` leaves a field as it is.
    EditSettings {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        about: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        picture: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        history_for_new: Option<bool>,
        /// The call node pinned to the group (`address:port#id`), for its
        /// group calls; an empty string takes it away.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        call_node: Option<String>,
        /// The access key of that node when it is private; an empty
        /// string takes it away.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        call_node_key: Option<String>,
        /// Who may post (version 2).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        post_mode: Option<PostMode>,
        /// When members may post (version 2); a schedule without days
        /// takes the schedule away.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        post_schedule: Option<Schedule>,
        /// Whose messages wait for a moderator (version 2).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        premod: Option<Premod>,
    },
    /// The owner becomes an admin, `to` becomes the owner.
    TransferOwnership { to: PubKey },
    /// A new key for no other reason (the previous one went stale).
    RotateKey,
    /// Public groups: a new link secret; the old link stops admitting.
    RotateLink { link_epoch: u32 },
    Disband,
}

impl OpBody {
    /// Does the body use something a device of version 1 cannot read? An
    /// operation that does not is written as version 1, so that it stays
    /// what it always was for every device.
    pub fn needs_v2(&self) -> bool {
        match self {
            Self::SetMuted { until, .. } => until.is_some(),
            Self::SetTrusted { .. } => true,
            Self::EditSettings { post_mode, post_schedule, premod, .. } => post_mode.is_some() || post_schedule.is_some() || premod.is_some(),
            _ => false,
        }
    }

    /// The oldest format version that holds this body.
    pub fn version(&self) -> u32 {
        if self.needs_v2() {
            2
        } else {
            1
        }
    }
}

/// Joining a public group by its link: the author shows that they hold
/// the key of the link that is current.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinProof {
    pub epoch: u32,
    /// See `GroupKey::join_mac`.
    pub mac: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Op {
    pub v: u32,
    pub group_id: String,
    pub author: PubKey,
    /// Every head of the log the author knew, sorted. Empty only for `Create`.
    pub parents: Vec<OpId>,
    /// Author's clock; shown to people, never used for ordering.
    pub created_at: i64,
    /// The key that is current after this operation, when it brings one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<KeyId>,
    /// `Join` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<JoinProof>,
    #[serde(flatten)]
    pub body: OpBody,
}

impl Op {
    pub fn new(group_id: &str, author: &PubKey, mut parents: Vec<OpId>, created_at: i64, body: OpBody) -> Self {
        parents.sort();
        parents.dedup();
        Self { v: body.version(), group_id: group_id.to_string(), author: author.clone(), parents, created_at, key: None, proof: None, body }
    }

    pub fn with_proof(mut self, proof: JoinProof) -> Self {
        self.proof = Some(proof);
        self
    }

    pub fn with_key(mut self, key: KeyId) -> Self {
        self.key = Some(key);
        self
    }

    /// Canonical form: compact JSON of the struct, fields in declaration
    /// order. No maps, so there is nothing whose order could vary.
    pub fn canonical(&self) -> String {
        serde_json::to_string(self).expect("an operation is always serializable")
    }

    pub fn id(&self) -> OpId {
        OpId(hex::encode(Sha256::digest(self.canonical().as_bytes())))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn pk(c: &str) -> PubKey {
        PubKey::parse(&c.repeat(64)[..64]).unwrap()
    }

    #[test]
    fn canonical_form_golden_vector() {
        let op = Op::new(
            "g1",
            &pk("a"),
            vec![OpId("22".into()), OpId("11".into()), OpId("22".into())],
            1_700_000_000,
            OpBody::SetRole { who: pk("b"), role: Role::Moderator },
        )
        .with_key(KeyId("k".into()));
        assert_eq!(op.parents, vec![OpId("11".into()), OpId("22".into())], "sorted, no duplicates");
        assert_eq!(
            op.canonical(),
            format!(
                r#"{{"v":1,"group_id":"g1","author":"{}","parents":["11","22"],"created_at":1700000000,"key":"k","op":"set_role","who":"{}","role":"moderator"}}"#,
                "a".repeat(64),
                "b".repeat(64)
            )
        );
        let back: Op = serde_json::from_str(&op.canonical()).unwrap();
        assert_eq!(back, op);
        assert_eq!(back.id(), op.id());
        assert_eq!(op.id().0.len(), 64);
    }

    #[test]
    fn any_change_is_a_different_operation() {
        let base = Op::new("g", &pk("a"), vec![], 1, OpBody::Join);
        let mut other = base.clone();
        other.created_at = 2;
        assert_ne!(base.id(), other.id());
        let mut other = base.clone();
        other.author = pk("b");
        assert_ne!(base.id(), other.id());
        assert_ne!(base.id(), base.clone().with_key(KeyId("x".into())).id());
        assert_ne!(base.id(), Op::new("g2", &pk("a"), vec![], 1, OpBody::Join).id());
    }

    /// What version 1 wrote, version 2 writes the same: a mute without an
    /// end, settings without posting rules, and every other body.
    #[test]
    fn operations_of_the_old_shape_stay_version_one_byte_for_byte() {
        let mute = Op::new("g1", &pk("a"), vec![OpId("11".into())], 1_700_000_000, OpBody::SetMuted { who: pk("b"), muted: true, until: None });
        assert_eq!(
            mute.canonical(),
            format!(r#"{{"v":1,"group_id":"g1","author":"{}","parents":["11"],"created_at":1700000000,"op":"set_muted","who":"{}","muted":true}}"#, "a".repeat(64), "b".repeat(64))
        );
        let edit = Op::new("g1", &pk("a"), vec![OpId("11".into())], 1_700_000_000, Settings { name: Some("n".into()), ..Settings::default() }.body());
        assert_eq!(
            edit.canonical(),
            format!(r#"{{"v":1,"group_id":"g1","author":"{}","parents":["11"],"created_at":1700000000,"op":"edit_settings","name":"n"}}"#, "a".repeat(64))
        );
        for body in [OpBody::Join, OpBody::Leave, OpBody::RotateKey, OpBody::Disband, OpBody::Ban { who: pk("b") }, OpBody::SetRole { who: pk("b"), role: Role::Admin }] {
            assert_eq!(Op::new("g", &pk("a"), vec![], 1, body).v, 1);
        }
        // Read back as it was written, whichever version wrote it.
        let old: Op = serde_json::from_str(&mute.canonical()).unwrap();
        assert_eq!(old, mute);
        assert_eq!(old.body, OpBody::SetMuted { who: pk("b"), muted: true, until: None });
        // A hand-made body of the new shape under the old label parses (the
        // reader takes every field), and says so of itself: the state
        // refuses it by that (`GroupState::apply`, `invalid("version")`).
        let mislabelled: Op = serde_json::from_str(&mute.canonical().replace(r#""muted":true}"#, r#""muted":true,"until":1700003600}"#)).unwrap();
        assert_eq!((mislabelled.v, mislabelled.body.version()), (1, 2));
    }

    #[test]
    fn canonical_form_golden_vector_version_two() {
        let until = Op::new("g1", &pk("a"), vec![OpId("11".into())], 1_700_000_000, OpBody::SetMuted { who: pk("b"), muted: true, until: Some(1_700_003_600) });
        assert_eq!(
            until.canonical(),
            format!(
                r#"{{"v":2,"group_id":"g1","author":"{}","parents":["11"],"created_at":1700000000,"op":"set_muted","who":"{}","muted":true,"until":1700003600}}"#,
                "a".repeat(64),
                "b".repeat(64)
            )
        );
        let trust = Op::new("g1", &pk("a"), vec![OpId("11".into())], 1_700_000_000, OpBody::SetTrusted { who: pk("b"), trusted: true });
        assert_eq!(
            trust.canonical(),
            format!(r#"{{"v":2,"group_id":"g1","author":"{}","parents":["11"],"created_at":1700000000,"op":"set_trusted","who":"{}","trusted":true}}"#, "a".repeat(64), "b".repeat(64))
        );
        let rules = Op::new(
            "g1",
            &pk("a"),
            vec![OpId("11".into())],
            1_700_000_000,
            Settings {
                post_mode: Some(PostMode::Managers),
                post_schedule: Some(Schedule { tz: "Europe/Berlin".into(), days: vec![1, 5], from: "09:00".into(), to: "18:00".into() }),
                premod: Some(Premod::Newcomers),
                ..Settings::default()
            }
            .body(),
        );
        assert_eq!(
            rules.canonical(),
            format!(
                r#"{{"v":2,"group_id":"g1","author":"{}","parents":["11"],"created_at":1700000000,"op":"edit_settings","post_mode":"managers","post_schedule":{{"tz":"Europe/Berlin","days":[1,5],"from":"09:00","to":"18:00"}},"premod":"newcomers"}}"#,
                "a".repeat(64)
            )
        );
        for op in [&until, &trust, &rules] {
            let back: Op = serde_json::from_str(&op.canonical()).unwrap();
            assert_eq!(&back, op);
            assert_eq!(back.id(), op.id());
        }
    }

    #[test]
    fn absent_fields_are_not_written() {
        let op = Op::new("g", &pk("a"), vec![], 1, Settings { name: Some("n".into()), ..Settings::default() }.body());
        let json = op.canonical();
        assert!(!json.contains("about") && !json.contains("\"key\""), "{json}");
        for word in ["post_mode", "post_schedule", "premod"] {
            assert!(!json.contains(word), "{json}");
        }
        let json = Op::new("g", &pk("a"), vec![], 1, OpBody::SetMuted { who: pk("b"), muted: false, until: None }).canonical();
        assert!(!json.contains("until"), "{json}");
        let json = Op::new("g", &pk("a"), vec![], 1, Settings { premod: Some(Premod::Off), ..Settings::default() }.body()).canonical();
        assert!(json.contains(r#""premod":"off""#) && !json.contains("post_mode"), "{json}");
    }

    /// The fields of `EditSettings` as a struct, so that a test names the
    /// ones it sets and leaves the rest (a variant has no `..default`).
    #[derive(Default)]
    pub(crate) struct Settings {
        pub(crate) name: Option<String>,
        pub(crate) about: Option<String>,
        pub(crate) picture: Option<String>,
        pub(crate) history_for_new: Option<bool>,
        pub(crate) call_node: Option<String>,
        pub(crate) call_node_key: Option<String>,
        pub(crate) post_mode: Option<PostMode>,
        pub(crate) post_schedule: Option<Schedule>,
        pub(crate) premod: Option<Premod>,
    }

    impl Settings {
        pub(crate) fn body(self) -> OpBody {
            OpBody::EditSettings {
                name: self.name,
                about: self.about,
                picture: self.picture,
                history_for_new: self.history_for_new,
                call_node: self.call_node,
                call_node_key: self.call_node_key,
                post_mode: self.post_mode,
                post_schedule: self.post_schedule,
                premod: self.premod,
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
