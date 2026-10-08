// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a group call is, as the host and the screen see it: the room I am
//! in with its seats, and the calls announced in the groups (the banner
//! "a call is on — join").

use crate::engine::Media;
use crate::node_client::Limits;
use serde::{Deserialize, Serialize};

/// The room I am in changed (phase, seats, who speaks, mute):
/// `{ "call": GroupCallView }`.
pub const UI_EVENT_GROUP_CALL_STATE: &str = "group_call.state";
/// A call is on in a group (its start came, or somebody joined or left
/// it): `{ "call": AnnouncedCall }`. For the banner of the chat.
pub const UI_EVENT_GROUP_CALL_STARTED: &str = "group_call.started";
/// The call of a group is over: `{ "call": AnnouncedCall, "outcome": "...", "duration_secs": n|null }`.
/// With `GroupCallView` in `"room"` when I was in it.
pub const UI_EVENT_GROUP_CALL_ENDED: &str = "group_call.ended";
/// `{ "call_id": "...", "participant": seat, "level": 0.0..1.0 }`.
pub const UI_EVENT_GROUP_CALL_LEVEL: &str = "group_call.level";

/// Where I am with a room.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupPhase {
    /// Making the room on the node.
    Starting,
    /// Joining the room: the offer is with the node, ICE is on its way.
    Joining,
    /// In the room.
    InRoom,
    /// The way to the node was lost: the engine tries on, the core judges
    /// the node and joins again, or the room moves to another node
    /// (`call.move`) and I follow. One phase for all of it.
    Reconnecting,
    /// Out of the room (the end of my part of the call).
    Left,
}

impl GroupPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Joining => "joining",
            Self::InRoom => "in_room",
            Self::Reconnecting => "reconnecting",
            Self::Left => "left",
        }
    }
}

/// One seat of the room, as the screen shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParticipantView {
    /// The seat (the node's participant id).
    pub id: u32,
    /// Who sits there, hex, once its word of identity was taken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npub: Option<String>,
    /// The word of identity checked: a member, signed by its key, on this
    /// seat. Only a verified seat is shown as a person and listened to.
    pub verified: bool,
    pub speaking: bool,
    /// The seat sends sound (an audio m-line of it is here).
    pub audio: bool,
    /// The m-line of the seat's sound, when it sends one: what the
    /// level of its sound comes by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_mid: Option<String>,
    /// The m-line of the seat's video, when it sends one: what the screen
    /// asks the frames of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_mid: Option<String>,
    pub me: bool,
}

/// The room I am in, as the screen shows it. Plain data, no secrets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroupCallView {
    pub call_id: String,
    pub group_id: String,
    pub chat_id: String,
    pub phase: GroupPhase,
    pub media: Media,
    pub muted: bool,
    pub video_local: bool,
    /// The camera in use (or the one for the next time), by the id the
    /// engine lists; absent for its default. On a phone `front` or `back`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<String>,
    /// Who made the room, hex.
    pub started_by: String,
    /// When the room was made (the time of `call.start`).
    pub started_at: i64,
    /// When I got into the room.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub joined_at: Option<i64>,
    /// The node I am connected to (`address:port#id`): the node of the
    /// room, or my own nearest node when I sit in the room through it
    /// (the cascade, services/call/spec/cascade.md).
    pub node: String,
    /// The node the room is on (its home); equal to `node` when I sit
    /// there directly. For the log and the CLI; no screen shows it yet.
    #[serde(default)]
    pub home: String,
    /// My seat, once the node gave it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participant: Option<u32>,
    /// The epoch of the keys I send with.
    pub epoch: u32,
    /// Every seat of the room, mine included, by seat.
    pub participants: Vec<ParticipantView>,
    /// What the node allows, as it said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<Limits>,
    /// The most the room takes from one participant, kbit/s (0: no limit).
    #[serde(default)]
    pub kbps_per_participant: u32,
    /// Seats the room has at most (0: the node did not say).
    #[serde(default)]
    pub max_participants: u32,
}

/// A call announced in a group, in the room or not: what the banner of
/// the chat shows. Built from `call.start` without `call.end`, with
/// the seats of `call.join` and `call.leave`, and dropped when the room
/// would have expired on the node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnouncedCall {
    pub call_id: String,
    pub group_id: String,
    pub chat_id: String,
    pub media: Media,
    pub started_by: String,
    pub started_at: i64,
    /// The members in the room by their own word (`call.join`), hex.
    pub participants: Vec<String>,
    /// I am in this room.
    pub joined: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_of_the_phases() {
        assert_eq!(serde_json::to_string(&GroupPhase::InRoom).unwrap(), "\"in_room\"");
        assert_eq!(GroupPhase::Reconnecting.as_str(), "reconnecting");
        let p = ParticipantView { id: 1, npub: None, verified: false, speaking: false, audio: true, audio_mid: None, video_mid: None, me: false };
        let json = serde_json::to_value(&p).unwrap();
        assert!(json.get("npub").is_none() && json.get("video_mid").is_none());
    }
}
