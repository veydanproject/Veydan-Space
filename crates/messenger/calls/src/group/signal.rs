// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The signalling of a group call as it travels in the group
//! (internal/messenger-wire.md §10, "Групповые звонки"): quiet notes of
//! the group, sealed with the group key like a message, read into one
//! typed [`GroupSignal`] and written back. The room, the node and the
//! first secret go in `call.start`; who sits where in `call.join` and
//! `call.leave`; the next secret in `call.epoch` (with the new token of
//! the room when the creator changed it); `call.end` closes; `call.move`
//! takes the call to a new room on another node when the node of the
//! room is gone ("Каскад и переезд").
//!
//! Every note but the start and the move names its room in `room_id`;
//! one without it (a client before the moves) is of the room of the
//! start. The core relates a note to a room by it: a note of a room left
//! behind is stale, a note of a room not known yet waits for the move
//! that brings it.

use crate::engine::Media;
use crate::servers::NodeRef;
use crate::signal::{is_call_id, reason};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use messenger_core::envelope::{T_CALL_END, T_CALL_EPOCH, T_CALL_JOIN, T_CALL_LEAVE, T_CALL_MOVE, T_CALL_START};
use messenger_core::Envelope;
use serde_json::Value;

/// The secret of an epoch: 32 random bytes, made by whoever opens the
/// epoch (the creator for the first, the oldest seat left for the next,
/// the mover for the epoch of a new room).
pub type Secret = [u8; 32];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupSignal {
    Start {
        call_id: String,
        room_id: String,
        node: NodeRef,
        /// The access key of a private node, for the members to use it too.
        key: Option<String>,
        join_token: String,
        secret: Secret,
        media: Media,
        /// When the room ends on the node whatever goes on (its clock).
        expires_at: i64,
    },
    Join {
        call_id: String,
        participant: u32,
        /// The room; `None` is the room of the start.
        room_id: Option<String>,
    },
    Leave {
        call_id: String,
        participant: u32,
        room_id: Option<String>,
    },
    Epoch {
        call_id: String,
        epoch: u32,
        secret: Secret,
        /// The new token of the room, when its owner changed it (a
        /// member was removed): taken from the owner of the room alone.
        join_token: Option<String>,
        room_id: Option<String>,
    },
    End {
        call_id: String,
        reason: String,
        room_id: Option<String>,
    },
    /// The node of `from_room_id` is gone: the author made `room_id` on
    /// `node` for the same call and turned the keys to `epoch`; everybody
    /// comes over.
    Move {
        call_id: String,
        from_room_id: String,
        room_id: String,
        node: NodeRef,
        key: Option<String>,
        join_token: String,
        expires_at: i64,
        /// The author's seat in `from_room_id`: its right to move is
        /// judged by it.
        seat: u32,
        epoch: u32,
        secret: Secret,
    },
}

impl GroupSignal {
    pub fn call_id(&self) -> &str {
        match self {
            Self::Start { call_id, .. }
            | Self::Join { call_id, .. }
            | Self::Leave { call_id, .. }
            | Self::Epoch { call_id, .. }
            | Self::End { call_id, .. }
            | Self::Move { call_id, .. } => call_id,
        }
    }

    /// The room the note is about, as written: `None` is the room of the
    /// start (a start names its own room; a move the room it comes from,
    /// which is the one it must be judged against).
    pub fn room_id(&self) -> Option<&str> {
        match self {
            Self::Start { room_id, .. } => Some(room_id),
            Self::Move { from_room_id, .. } => Some(from_room_id),
            Self::Join { room_id, .. } | Self::Leave { room_id, .. } | Self::Epoch { room_id, .. } | Self::End { room_id, .. } => {
                room_id.as_deref()
            }
        }
    }

    /// Read a `call.*` envelope of a group. `None` for anything else, or
    /// malformed: an id of another shape, a secret that is not 32 bytes,
    /// a node reference that does not parse.
    pub fn parse(e: &Envelope) -> Option<Self> {
        let call_id = e.str_field("call_id").filter(|id| is_call_id(id))?.to_string();
        let number = |key: &str| e.fields.get(key).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
        let room = |key: &str| e.str_field(key).filter(|r| is_hex_token(r, 32)).map(String::from);
        let token = |key: &str| e.str_field(key).filter(|t| !t.is_empty() && t.len() <= 128).map(String::from);
        let key = e.str_field("key").filter(|k| !k.is_empty() && k.len() <= 256).map(String::from);
        Some(match e.t.as_str() {
            T_CALL_START => {
                if number("epoch")? != 1 {
                    return None;
                }
                GroupSignal::Start {
                    call_id,
                    room_id: room("room_id")?,
                    node: e.str_field("node")?.parse().ok()?,
                    key,
                    join_token: token("join_token")?,
                    secret: secret(e.str_field("secret")?)?,
                    media: Media::parse(e.str_field("media")?)?,
                    expires_at: e.fields.get("expires_at").and_then(Value::as_i64).unwrap_or(0),
                }
            }
            T_CALL_JOIN => GroupSignal::Join { call_id, participant: number("participant")?, room_id: room("room_id") },
            T_CALL_LEAVE => GroupSignal::Leave { call_id, participant: number("participant")?, room_id: room("room_id") },
            T_CALL_EPOCH => {
                let epoch = number("epoch")?;
                if epoch < 2 {
                    return None;
                }
                GroupSignal::Epoch { call_id, epoch, secret: secret(e.str_field("secret")?)?, join_token: token("join_token"), room_id: room("room_id") }
            }
            T_CALL_END => GroupSignal::End { call_id, reason: word(e.str_field("reason")), room_id: room("room_id") },
            T_CALL_MOVE => {
                let epoch = number("epoch")?;
                if epoch < 2 {
                    return None;
                }
                let (from_room_id, room_id) = (room("from_room_id")?, room("room_id")?);
                if from_room_id == room_id {
                    return None;
                }
                GroupSignal::Move {
                    call_id,
                    from_room_id,
                    room_id,
                    node: e.str_field("node")?.parse().ok()?,
                    key,
                    join_token: token("join_token")?,
                    expires_at: e.fields.get("expires_at").and_then(Value::as_i64).unwrap_or(0),
                    seat: number("seat")?,
                    epoch,
                    secret: secret(e.str_field("secret")?)?,
                }
            }
            _ => return None,
        })
    }

    pub fn to_envelope(&self) -> Envelope {
        match self {
            Self::Start { call_id, room_id, node, key, join_token, secret, media, expires_at } => Envelope::call_start(
                call_id,
                room_id,
                &node.to_string(),
                key.as_deref(),
                join_token,
                &B64.encode(secret),
                media.as_str(),
                *expires_at,
            ),
            Self::Join { call_id, participant, room_id } => Envelope::call_join(call_id, *participant).in_room(room_id.as_deref()),
            Self::Leave { call_id, participant, room_id } => Envelope::call_leave(call_id, *participant).in_room(room_id.as_deref()),
            Self::Epoch { call_id, epoch, secret, join_token, room_id } => {
                Envelope::call_epoch(call_id, *epoch, &B64.encode(secret), join_token.as_deref()).in_room(room_id.as_deref())
            }
            Self::End { call_id, reason, room_id } => Envelope::call_end(call_id, reason, None).in_room(room_id.as_deref()),
            Self::Move { call_id, from_room_id, room_id, node, key, join_token, expires_at, seat, epoch, secret } => Envelope::call_move(
                call_id,
                from_room_id,
                room_id,
                &node.to_string(),
                key.as_deref(),
                join_token,
                *expires_at,
                *seat,
                *epoch,
                &B64.encode(secret),
            ),
        }
    }
}

fn secret(b64: &str) -> Option<Secret> {
    B64.decode(b64).ok()?.try_into().ok()
}

fn is_hex_token(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn word(s: Option<&str>) -> String {
    match s {
        Some(w) if w.len() <= 32 && w.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') => w.to_string(),
        _ => reason::ENDED.to_string(),
    }
}

/// A new secret of an epoch.
pub fn new_secret() -> Secret {
    let mut s = [0u8; 32];
    getrandom::fill(&mut s).expect("the system has random bytes");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::new_call_id;

    fn node() -> NodeRef {
        format!("203.0.113.7:8443#{}", "ef".repeat(32)).parse().unwrap()
    }

    #[test]
    fn every_group_signal_survives_the_wire() {
        let id = new_call_id();
        let room = Some("ab".repeat(16));
        let all = vec![
            GroupSignal::Start {
                call_id: id.clone(),
                room_id: "cd".repeat(16),
                node: node(),
                key: Some("k".into()),
                join_token: "12".repeat(24),
                secret: [7u8; 32],
                media: Media::Video,
                expires_at: 1_760_043_200,
            },
            GroupSignal::Start {
                call_id: id.clone(),
                room_id: "cd".repeat(16),
                node: node(),
                key: None,
                join_token: "t".into(),
                secret: new_secret(),
                media: Media::Audio,
                expires_at: 0,
            },
            GroupSignal::Join { call_id: id.clone(), participant: 3, room_id: None },
            GroupSignal::Join { call_id: id.clone(), participant: 3, room_id: room.clone() },
            GroupSignal::Leave { call_id: id.clone(), participant: 3, room_id: room.clone() },
            GroupSignal::Epoch { call_id: id.clone(), epoch: 2, secret: [9u8; 32], join_token: None, room_id: None },
            GroupSignal::Epoch { call_id: id.clone(), epoch: 3, secret: [9u8; 32], join_token: Some("34".repeat(24)), room_id: room.clone() },
            GroupSignal::End { call_id: id.clone(), reason: reason::ENDED.into(), room_id: room.clone() },
            GroupSignal::Move {
                call_id: id.clone(),
                from_room_id: "cd".repeat(16),
                room_id: "ab".repeat(16),
                node: node(),
                key: Some("k".into()),
                join_token: "56".repeat(24),
                expires_at: 1_760_050_000,
                seat: 2,
                epoch: 4,
                secret: [3u8; 32],
            },
        ];
        for s in all {
            let wire = s.to_envelope().encode();
            let back = GroupSignal::parse(&Envelope::parse(&wire).unwrap()).unwrap();
            assert_eq!(back, s, "{wire}");
            assert_eq!(back.call_id(), id);
        }
        let moved = GroupSignal::parse(&Envelope::call_move(&id, &"cd".repeat(16), &"ab".repeat(16), &node().to_string(), None, "t", 1, 1, 2, &B64.encode([1u8; 32]))).unwrap();
        assert_eq!(moved.room_id(), Some("cd".repeat(16).as_str()), "a move is judged against the room it comes from");
        assert_eq!(GroupSignal::Join { call_id: id.clone(), participant: 1, room_id: None }.room_id(), None);
    }

    #[test]
    fn what_is_malformed_is_not_read() {
        let id = new_call_id();
        let good = GroupSignal::Start {
            call_id: id.clone(),
            room_id: "cd".repeat(16),
            node: node(),
            key: None,
            join_token: "t".into(),
            secret: [1u8; 32],
            media: Media::Audio,
            expires_at: 1,
        }
        .to_envelope();
        assert!(GroupSignal::parse(&good).is_some());
        assert!(GroupSignal::parse(&good.clone().with("secret", "c2hvcnQ=")).is_none(), "a short secret");
        assert!(GroupSignal::parse(&good.clone().with("node", "not a node")).is_none());
        assert!(GroupSignal::parse(&good.clone().with("epoch", 2)).is_none(), "a start is epoch 1");
        assert!(GroupSignal::parse(&good.clone().with("room_id", "xyz")).is_none());
        assert!(GroupSignal::parse(&good.clone().with("media", "hologram")).is_none());
        assert!(GroupSignal::parse(&Envelope::call_epoch(&id, 1, &B64.encode([1u8; 32]), None)).is_none(), "epoch 1 is the start's");
        assert!(GroupSignal::parse(&Envelope::call_join(&id, 1).with("participant", -1)).is_none());
        assert!(GroupSignal::parse(&Envelope::call_join("short", 1)).is_none());
        assert!(GroupSignal::parse(&Envelope::call_video(&id, true)).is_none(), "a word of a call between two");
        assert_eq!(
            GroupSignal::parse(&Envelope::call_end(&id, "Hacked", None)).unwrap(),
            GroupSignal::End { call_id: id.clone(), reason: "ended".into(), room_id: None }
        );
        // A room of another shape is no room: the note is of the start's.
        assert_eq!(GroupSignal::parse(&Envelope::call_join(&id, 1).with("room_id", "xyz")).unwrap().room_id(), None);
        let moved = Envelope::call_move(&id, &"cd".repeat(16), &"ab".repeat(16), &node().to_string(), None, "t", 1, 1, 2, &B64.encode([1u8; 32]));
        assert!(GroupSignal::parse(&moved).is_some());
        assert!(GroupSignal::parse(&moved.clone().with("epoch", 1)).is_none(), "a move turns the keys: epoch 2 at least");
        assert!(GroupSignal::parse(&moved.clone().with("room_id", "cd".repeat(16))).is_none(), "a move goes somewhere else");
        assert!(GroupSignal::parse(&moved.clone().with("seat", -1)).is_none());
        assert!(GroupSignal::parse(&moved.clone().with("node", "nowhere")).is_none());
    }
}
