// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What the phone shows, in the words of the messenger. The words of the
//! user's language are the phone's: it has the strings, this side has the
//! facts.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatKind {
    Dm,
    /// A direct message from somebody who is not my contact yet.
    Request,
    Group,
}

/// What the message was, without a word of any language: the same facts the
/// running app tells of a message it takes in itself.
pub use messenger_core::{Body, LinkKind};

/// A message the phone may show in full.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    pub kind: ChatKind,
    /// `dm:<pubkey>` or `group:<id>`; what a tap opens. None: the list of chats.
    pub chat: Option<String>,
    /// The chat as the list shows it: the peer's name, or the group's.
    pub title: String,
    /// Who wrote, as this phone calls them.
    pub sender: String,
    pub sender_key: String,
    /// Address of the sender's picture, https only; the phone may fetch it.
    pub picture: Option<String>,
    /// None when the settings say the text stays in the app.
    pub body: Option<Body>,
    pub muted: bool,
    pub hide_on_lockscreen: bool,
    pub count: u32,
}

/// Something came, and that is all the phone may or can say.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plain {
    pub kind: ChatKind,
    pub chat: Option<String>,
    /// The group's name, when the push named a group this phone knows.
    pub title: Option<String>,
    pub muted: bool,
    pub count: u32,
}

/// Why nothing is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// Written by me, from another device.
    Own,
    /// From somebody I blocked, or who blocked me.
    Blocked,
    /// An edit, a deletion, a signal: nothing to read.
    NotAMessage,
    /// A group I am not in, or a chat that would not take the message.
    NotForMe,
    /// The event is not what the push said, or not valid.
    Invalid,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
    Show(Notice),
    Plain(Plain),
    Quiet { reason: Reason },
}
