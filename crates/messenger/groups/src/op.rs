// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Operations: the only way a group changes.
//!
//! An operation names the group, its author, the operations its author
//! had seen (`parents`), and what to do. Its id is the SHA-256 of its
//! canonical JSON, so the same operation has the same id everywhere and
//! cannot be altered without becoming a different one.

use crate::roles::Role;
use messenger_core::PubKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Format version of operations; part of the hashed content.
pub const OP_VERSION: u32 = 1;

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
    SetMuted { who: PubKey, muted: bool },
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
    },
    /// The owner becomes an admin, `to` becomes the owner.
    TransferOwnership { to: PubKey },
    /// A new key for no other reason (the previous one went stale).
    RotateKey,
    /// Public groups: a new link secret; the old link stops admitting.
    RotateLink { link_epoch: u32 },
    Disband,
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
        Self { v: OP_VERSION, group_id: group_id.to_string(), author: author.clone(), parents, created_at, key: None, proof: None, body }
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
mod tests {
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

    #[test]
    fn absent_fields_are_not_written() {
        let op = Op::new("g", &pk("a"), vec![], 1, OpBody::EditSettings { name: Some("n".into()), about: None, picture: None, history_for_new: None });
        let json = op.canonical();
        assert!(!json.contains("about") && !json.contains("\"key\""), "{json}");
    }

    #[test]
    fn key_id_is_derived_from_the_key() {
        let a = KeyId::of(&[1u8; 32]);
        assert_eq!(a, KeyId::of(&[1u8; 32]));
        assert_ne!(a, KeyId::of(&[2u8; 32]));
        assert_eq!(a.0.len(), 32);
    }
}
