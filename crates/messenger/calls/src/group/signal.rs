// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The signalling of a group call as it travels in the group
//! (internal/messenger-wire.md §10, "Групповые звонки"): quiet notes of
//! the group, sealed with the group key like a message, read into one
//! typed [`GroupSignal`] and written back. The room, the node and the
//! first secret go in `call.start`; who sits where in `call.join` and
//! `call.leave`; the next secret in `call.epoch` (with the new token of
//! the room when the creator changed it); `call.end` closes.

use crate::engine::Media;
use crate::servers::NodeRef;
use crate::signal::{is_call_id, reason};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use messenger_core::envelope::{T_CALL_END, T_CALL_EPOCH, T_CALL_JOIN, T_CALL_LEAVE, T_CALL_START};
use messenger_core::Envelope;
use serde_json::Value;

/// The secret of an epoch: 32 random bytes, made by whoever opens the
/// epoch (the creator for the first, the oldest seat left for the next).
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
    Join { call_id: String, participant: u32 },
    Leave { call_id: String, participant: u32 },
    Epoch {
        call_id: String,
        epoch: u32,
        secret: Secret,
        /// The new token of the room, when the creator changed it (a
        /// member was removed): taken from the creator alone.
        join_token: Option<String>,
    },
    End { call_id: String, reason: String },
}

impl GroupSignal {
    pub fn call_id(&self) -> &str {
        match self {
            Self::Start { call_id, .. }
            | Self::Join { call_id, .. }
            | Self::Leave { call_id, .. }
            | Self::Epoch { call_id, .. }
            | Self::End { call_id, .. } => call_id,
        }
    }

    /// Read a `call.*` envelope of a group. `None` for anything else, or
    /// malformed: an id of another shape, a secret that is not 32 bytes,
    /// a node reference that does not parse.
    pub fn parse(e: &Envelope) -> Option<Self> {
        let call_id = e.str_field("call_id").filter(|id| is_call_id(id))?.to_string();
        let number = |key: &str| e.fields.get(key).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
        Some(match e.t.as_str() {
            T_CALL_START => {
                if number("epoch")? != 1 {
                    return None;
                }
                GroupSignal::Start {
                    call_id,
                    room_id: e.str_field("room_id").filter(|r| is_hex_token(r, 32))?.to_string(),
                    node: e.str_field("node")?.parse().ok()?,
                    key: e.str_field("key").filter(|k| !k.is_empty() && k.len() <= 256).map(String::from),
                    join_token: e.str_field("join_token").filter(|t| !t.is_empty() && t.len() <= 128)?.to_string(),
                    secret: secret(e.str_field("secret")?)?,
                    media: Media::parse(e.str_field("media")?)?,
                    expires_at: e.fields.get("expires_at").and_then(Value::as_i64).unwrap_or(0),
                }
            }
            T_CALL_JOIN => GroupSignal::Join { call_id, participant: number("participant")? },
            T_CALL_LEAVE => GroupSignal::Leave { call_id, participant: number("participant")? },
            T_CALL_EPOCH => {
                let epoch = number("epoch")?;
                if epoch < 2 {
                    return None;
                }
                GroupSignal::Epoch {
                    call_id,
                    epoch,
                    secret: secret(e.str_field("secret")?)?,
                    join_token: e.str_field("join_token").filter(|t| !t.is_empty() && t.len() <= 128).map(String::from),
                }
            }
            T_CALL_END => GroupSignal::End { call_id, reason: word(e.str_field("reason")) },
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
            Self::Join { call_id, participant } => Envelope::call_join(call_id, *participant),
            Self::Leave { call_id, participant } => Envelope::call_leave(call_id, *participant),
            Self::Epoch { call_id, epoch, secret, join_token } => {
                Envelope::call_epoch(call_id, *epoch, &B64.encode(secret), join_token.as_deref())
            }
            Self::End { call_id, reason } => Envelope::call_end(call_id, reason, None),
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
            GroupSignal::Join { call_id: id.clone(), participant: 3 },
            GroupSignal::Leave { call_id: id.clone(), participant: 3 },
            GroupSignal::Epoch { call_id: id.clone(), epoch: 2, secret: [9u8; 32], join_token: None },
            GroupSignal::Epoch { call_id: id.clone(), epoch: 3, secret: [9u8; 32], join_token: Some("34".repeat(24)) },
            GroupSignal::End { call_id: id.clone(), reason: reason::ENDED.into() },
        ];
        for s in all {
            let wire = s.to_envelope().encode();
            let back = GroupSignal::parse(&Envelope::parse(&wire).unwrap()).unwrap();
            assert_eq!(back, s, "{wire}");
            assert_eq!(back.call_id(), id);
        }
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
        assert_eq!(GroupSignal::parse(&Envelope::call_end(&id, "Hacked", None)).unwrap(), GroupSignal::End { call_id: id, reason: "ended".into() });
    }
}
