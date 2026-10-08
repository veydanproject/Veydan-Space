// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a call is, as the host and the screen see it: its phase, its
//! outcome, the view of it that goes with every `call.*` UI event.

use crate::engine::{Media, PairKind};
use serde::{Deserialize, Serialize};

/// A call comes in: `{ "call": CallView }`. Once per call.
pub const UI_EVENT_CALL_INCOMING: &str = "call.incoming";
/// The call changed (phase, the way of the media, mute): `{ "call": CallView }`.
pub const UI_EVENT_CALL_STATE: &str = "call.state";
/// The call is over: `{ "call": CallView, "outcome": "...", "duration_secs": n|null }`.
/// Also for a call that was missed while I was away, with no `call.incoming`
/// before it.
pub const UI_EVENT_CALL_ENDED: &str = "call.ended";
/// `{ "call_id": "...", "stats": SessionStats }`, as the engine tells them.
pub const UI_EVENT_CALL_STATS: &str = "call.stats";
/// `{ "call_id": "...", "level": 0.0..1.0 }`: the sound from the peer.
pub const UI_EVENT_CALL_LEVEL: &str = "call.level";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    In,
    Out,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::In => messenger_store::calls::DIR_IN,
            Self::Out => messenger_store::calls::DIR_OUT,
        }
    }
}

/// Where a call is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// I called; the peer's phone rings (or the offer is still being made).
    Outgoing,
    /// The peer calls; my phone rings.
    Incoming,
    /// Answered on both sides; ICE is looking for a way.
    Connecting,
    /// Talking.
    Active,
    /// Talking, and the way was lost (or the network changed under one
    /// side): a restart of ICE is under way. `reconnect_reason` says why.
    Reconnecting,
    Ended,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Outgoing => "outgoing",
            Self::Incoming => "incoming",
            Self::Connecting => "connecting",
            Self::Active => "active",
            Self::Reconnecting => "reconnecting",
            Self::Ended => "ended",
        }
    }
}

/// Why a call is `Reconnecting`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconnectReason {
    /// The engine saw the way go (no answer from the peer for a while).
    ConnectionLost,
    /// My network changed under the call (a new interface came up).
    NetworkChanged,
    /// The peer lost the way or changed its network: it asked for a new
    /// offer, or made one.
    PeerLost,
}

/// How a call ended, as the record keeps it (`msg_calls.outcome`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Missed,
    Declined,
    Busy,
    Ended,
    Failed,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        use messenger_store::calls as repo;
        match self {
            Self::Missed => repo::OUTCOME_MISSED,
            Self::Declined => repo::OUTCOME_DECLINED,
            Self::Busy => repo::OUTCOME_BUSY,
            Self::Ended => repo::OUTCOME_ENDED,
            Self::Failed => repo::OUTCOME_FAILED,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        use messenger_store::calls as repo;
        Some(match s {
            repo::OUTCOME_MISSED => Self::Missed,
            repo::OUTCOME_DECLINED => Self::Declined,
            repo::OUTCOME_BUSY => Self::Busy,
            repo::OUTCOME_ENDED => Self::Ended,
            repo::OUTCOME_FAILED => Self::Failed,
            _ => return None,
        })
    }
}

/// The size of the frames of one video, as they come.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoSize {
    pub width: u32,
    pub height: u32,
}

/// The call as the screen shows it. Plain data, no secrets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CallView {
    pub call_id: String,
    /// The peer, hex.
    pub peer: String,
    pub chat_id: String,
    pub direction: Direction,
    pub media: Media,
    pub phase: Phase,
    /// How the media goes, once ICE settled: `direct` | `relay`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<PairKind>,
    /// Why the call is `Reconnecting`; `None` in every other phase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconnect_reason: Option<ReconnectReason>,
    pub muted: bool,
    /// When the invitation was made (the time inside the rumor).
    pub started_at: i64,
    /// When it was taken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_at: Option<i64>,
    /// The ids of the nodes this side uses, nearest first; empty when the
    /// call goes with host candidates alone.
    #[serde(default)]
    pub nodes: Vec<String>,
    /// What the nearest node allows, as it said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<crate::node_client::Limits>,
    /// My video goes: the camera, or the screen (`video_screen`).
    #[serde(default)]
    pub video_local: bool,
    /// What I send is my screen, not my camera.
    #[serde(default)]
    pub video_screen: bool,
    /// The camera in use, by the id the engine lists; `None` for its
    /// default (or on a phone, where the plugin holds the camera).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<String>,
    /// The peer's video goes, by its word (`call.video`), or by the
    /// invitation of a video call until it says otherwise.
    #[serde(default)]
    pub video_remote: bool,
    /// The size of the frames each way, once some came; `None` while the
    /// video is off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_local_size: Option<VideoSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_remote_size: Option<VideoSize>,
}

impl CallView {
    /// A call just begun: nothing settled yet beyond what is given.
    pub fn new(call_id: String, peer: String, direction: Direction, media: Media, phase: Phase, started_at: i64) -> Self {
        Self {
            chat_id: messenger_store::chats::dm_chat_id(&peer),
            call_id,
            peer,
            direction,
            media,
            phase,
            via: None,
            reconnect_reason: None,
            muted: false,
            started_at,
            answered_at: None,
            nodes: vec![],
            limits: None,
            video_local: false,
            video_screen: false,
            camera: None,
            // The invitation says what the caller sends from the start;
            // the called side's camera goes on as it takes a video call.
            video_remote: media == Media::Video,
            video_local_size: None,
            video_remote_size: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_are_the_words_of_the_record() {
        for o in [Outcome::Missed, Outcome::Declined, Outcome::Busy, Outcome::Ended, Outcome::Failed] {
            assert_eq!(Outcome::parse(o.as_str()), Some(o));
            assert_eq!(serde_json::to_string(&o).unwrap(), format!("\"{}\"", o.as_str()));
        }
        assert_eq!(Outcome::parse("lost"), None);
        assert_eq!(serde_json::to_string(&Phase::Connecting).unwrap(), "\"connecting\"");
        assert_eq!(serde_json::to_string(&Phase::Reconnecting).unwrap(), "\"reconnecting\"");
        assert_eq!(serde_json::to_string(&ReconnectReason::PeerLost).unwrap(), "\"peer_lost\"");
        assert_eq!(Direction::Out.as_str(), "out");
    }
}
