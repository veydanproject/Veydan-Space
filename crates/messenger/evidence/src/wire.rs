// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The report as it travels from the client to the server of the project.
//! Sums are tagged by `t`, as every sum of the messenger's wire is.

use serde::{Deserialize, Serialize};

/// The version of the report this crate writes and reads.
pub const REPORT_VERSION: u8 = 1;

/// A report on a person or on a whole group, with the messages it rests on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    /// Whom the report is on: a person (the default, and all a report of
    /// an older client is), or the group of [`Context::Group`].
    #[serde(default, skip_serializing_if = "Target::is_person")]
    pub target: Target,
    /// [`REPORT_VERSION`]. A server refuses any other: a newer report may
    /// mean something this one does not check.
    pub v: u8,
    /// The key of whoever reports, 64 lowercase hex characters. The server
    /// learns it from the session too; here it is what the proofs of the
    /// reporter's own messages are checked against.
    pub reporter: String,
    /// The key of the person reported, 64 lowercase hex characters; of a
    /// group, its owner as the reporter's app knows it (a claim: the
    /// journal of the group is not attached).
    pub accused: String,
    /// What the report is about.
    pub reason: Reason,
    /// A finer word under the reason, from the list of the client
    /// (at most [`crate::MAX_SUB_REASON_CHARS`] characters).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_reason: Option<String>,
    /// What the reporter writes in their own words (at most
    /// [`crate::MAX_COMMENT_CHARS`] characters). Nobody signed it but the
    /// reporter: it is a claim, not evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    /// Where it happened.
    pub context: Context,
    /// The messages attached, at most [`crate::MAX_MESSAGES`].
    #[serde(default)]
    pub messages: Vec<Attached>,
    /// When the report was made, unix seconds.
    pub created_at: i64,
    /// The name of the group as the reporter's app shows it (at most
    /// [`crate::MAX_GROUP_NAME_CHARS`] characters), in a report from a
    /// group, on a person or on the group. Nobody's signature covers it:
    /// it tells the moderator which group, it proves nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_name: Option<String>,
    /// The owner and the admins of the group as the reporter's app knows
    /// them (at most [`crate::MAX_GROUP_ADMINS`] keys, 64 lowercase hex), in
    /// a report on a group: whom the measures may reach. A claim, as the
    /// name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub group_admins: Vec<String>,
    /// The message the report was opened on (its id, 64 lowercase hex), when
    /// it was opened on one: the moderator sees which of the attached it
    /// was. It counts only when it is the id of an attached message proved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
}

/// Whom a report is on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    /// The person of `accused`: the messages attached are theirs and the
    /// reporter's.
    #[default]
    Person,
    /// The group of [`Context::Group`]: the messages attached may be any
    /// member's, and the measures reach the group and its admins.
    Group,
}

impl Target {
    /// For `skip_serializing_if`: a report on a person reads as it did
    /// before groups could be reported.
    pub fn is_person(&self) -> bool {
        matches!(self, Target::Person)
    }
}

/// What a report is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    ChildHarm,
    Spam,
    Harassment,
    Illegal,
    Other,
}

/// Where the reported conduct happened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Context {
    /// A chat between the two.
    Dm,
    /// A group; `group_id` is the `h` of its events, and a group proof
    /// counts only for this group.
    Group { group_id: String },
    /// A request: a stranger wrote first and was not taken in.
    Request,
    /// A channel of a community (crates/communities): `community_id` is its
    /// id (64 lowercase hex), `post_id` the post or comment the report was
    /// opened on, when it was. The messages of a channel carry no proof a
    /// report can attach (they are sealed under the channel's keys): the
    /// moderator reads the post on the channel's relays. A DM attached
    /// still counts, as in any context.
    Channel {
        community_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        post_id: Option<String>,
    },
}

/// One attached message: its proof, or why there is none.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attached {
    /// The proof that its author wrote it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<Proof>,
    /// Why there is no proof, a short word of the client, e.g. `wrap_gone`
    /// when the device no longer keeps the event the message came in
    /// (at most [`crate::MAX_NOTE_CHARS`] characters).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// What shows that a key wrote a message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Proof {
    /// A NIP-17 message: the seal (kind 13, signed by the author) and the
    /// keys of this one message only.
    Dm {
        /// The seal as the gift wrap held it: its signature covers the
        /// encrypted rumor, and so the message.
        seal: nostr::event::Event,
        /// The keys that open the seal's payload and nothing else.
        keys: MessageKeys,
    },
    /// A group message: the inner event (kind 9, signed by the author), as
    /// it was inside the sealed event of the group.
    Group {
        /// The signed message; its content is the envelope.
        inner: nostr::event::Event,
    },
}

/// The per-message keys of NIP-44 v2 for the seal's payload: derived from
/// the conversation key with the payload's nonce by HKDF-expand, so handing
/// them over reveals this one message and nothing else (HKDF-expand is
/// one-way; the conversation key stays with its owners). Hex on the wire.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageKeys {
    /// The ChaCha20 key, 32 bytes.
    #[serde(with = "hex_array")]
    pub chacha_key: [u8; 32],
    /// The ChaCha20 nonce, 12 bytes.
    #[serde(with = "hex_array")]
    pub chacha_nonce: [u8; 12],
    /// The HMAC-SHA256 key of the payload's MAC, 32 bytes.
    #[serde(with = "hex_array")]
    pub hmac_key: [u8; 32],
}

/// The keys open one message only, but that message is somebody's private
/// word: a log line should not carry it.
impl std::fmt::Debug for MessageKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MessageKeys(<one message>)")
    }
}

/// A byte array as lowercase hex of exactly its length.
mod hex_array {
    use serde::{de::Error as _, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer, const N: usize>(
        bytes: &[u8; N],
        s: S,
    ) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>, const N: usize>(
        d: D,
    ) -> Result<[u8; N], D::Error> {
        let text = String::deserialize(d)?;
        let mut out = [0u8; N];
        hex::decode_to_slice(&text, &mut out)
            .map_err(|_| D::Error::custom(format!("expected {} hex characters", N * 2)))?;
        Ok(out)
    }
}
