// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Evidence of a report. A user may report a person to the moderators of
//! the project and attach up to ten messages. The server that takes the
//! report reads nobody's conversation, yet it must know that the accused
//! really wrote each message attached, or any modified client could put
//! words in anybody's mouth.
//!
//! Every message already carries its author's signature: a DM in its seal
//! (NIP-59), a group message as its inner event. A proof hands over that
//! signed object and, for a DM, the keys of NIP-44 that open this one
//! payload and no other ([`MessageKeys`]). The server checks the signature,
//! opens the payload and reads who wrote what to whom ([`verify`]); the
//! client builds the proofs from what it holds ([`dm_proof`],
//! [`group_proof`]).
//!
//! What a proof shows: this key wrote this message, to that key (a DM) or
//! in that group. What it does not show: that it is the whole
//! conversation, or what came before and after; the reporter picks the
//! messages. See README.md.
//!
//! Built by the server of the project as well: nothing here touches the
//! store, the network or an application framework.

mod build;
pub mod nip44;
mod verify;
mod wire;

pub use build::{dm_proof, group_proof};
pub use verify::{
    verify, Message, Outcome, Verified, DM_KINDS, KIND_GIFT_WRAP, KIND_GROUP_MESSAGE, KIND_SEAL,
    MAX_COMMENT_CHARS, MAX_CONTENT_BYTES, MAX_GROUP_ADMINS, MAX_GROUP_ID_CHARS,
    MAX_GROUP_NAME_CHARS, MAX_MESSAGES, MAX_NOTE_CHARS, MAX_SUB_REASON_CHARS,
    REPORT_FUTURE_WINDOW_SECS, REPORT_PAST_WINDOW_SECS, SEAL_TIME_SLACK_SECS, SEAL_TWEAK_SECS,
};
pub use wire::{Attached, Context, MessageKeys, Proof, Reason, Report, Target, REPORT_VERSION};

/// Why the client could not make a proof.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Not a gift wrap, or one whose signature does not hold.
    #[error("not a gift wrap")]
    NotAWrap,
    /// The wrap is not addressed to these keys.
    #[error("the wrap does not open with these keys")]
    NotMine,
    /// What the wrap holds is not a valid seal.
    #[error("bad seal: {0}")]
    Seal(&'static str),
    /// The seal's payload does not open.
    #[error("bad seal payload: {0}")]
    Payload(nip44::PayloadError),
    /// The seal holds no message of its author.
    #[error("bad rumor: {0}")]
    Rumor(&'static str),
    /// Not a signed group message.
    #[error("not a group message: {0}")]
    Group(&'static str),
}
