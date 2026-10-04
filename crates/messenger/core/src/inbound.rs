// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Everything that arrived from the network, already validated and
//! classified. Produced by `messenger-ingress`, consumed by handlers.
//!
//! Adding a message family means adding a variant here and a handler for it;
//! nothing else changes. Variants that no handler claims yet are still
//! present so the dispatcher can log them as `Ignored` with a reason.

use crate::types::{EventId, EventSource, PubKey, Timestamp};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum Inbound {
    Dm(DmInbound),
    Group(GroupInbound),
    /// Reserved for public channels / feed. Not implemented in alpha.
    Channel(ChannelInbound),
    Meta(MetaInbound),
    Ignored { kind: u16, reason: String },
}

impl Inbound {
    pub fn ignored(kind: u16, reason: impl Into<String>) -> Self {
        Self::Ignored { kind, reason: reason.into() }
    }

    /// Family name for logs and metrics.
    pub fn family(&self) -> &'static str {
        match self {
            Self::Dm(_) => "dm",
            Self::Group(_) => "group",
            Self::Channel(_) => "channel",
            Self::Meta(_) => "meta",
            Self::Ignored { .. } => "ignored",
        }
    }
}

/// Common envelope facts shared by every family.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Envelope {
    /// Id of the outer (wire) event — what relays and dedup see.
    pub wire_id: EventId,
    pub source: EventSource,
    /// `created_at` of the outer event.
    pub wire_created_at: Timestamp,
}

/// A NIP-17 direct message after unwrapping. `content` is the decrypted
/// application envelope (`{"v","t",…}`); parsing it is the DM handler's job.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DmInbound {
    pub envelope: Envelope,
    /// Stable inner-rumor id — the same on every copy of the message.
    pub rumor_id: EventId,
    pub sender: PubKey,
    pub recipients: Vec<PubKey>,
    /// `created_at` of the rumor: the application-controlled message time.
    pub created_at: Timestamp,
    pub content: String,
    pub reply_to: Option<EventId>,
}

/// A group event (kind 9 or its companions). Content is still ciphertext:
/// the group handler owns the key lookup.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GroupInbound {
    pub envelope: Envelope,
    pub group_id: String,
    pub sender: PubKey,
    pub created_at: Timestamp,
    pub kind: u16,
    /// Which group key encrypts the content (tag `k`).
    pub key_id: Option<String>,
    pub ciphertext: String,
    pub reply_to: Option<EventId>,
}

/// Placeholder for channel/feed events. Carried through untouched.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelInbound {
    pub envelope: Envelope,
    pub kind: u16,
}

/// Public metadata events that need no decryption.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "meta", rename_all = "snake_case")]
pub enum MetaInbound {
    Profile { author: PubKey, created_at: Timestamp, content: String },
    Follows { author: PubKey, created_at: Timestamp, follows: Vec<PubKey> },
    RelayList { author: PubKey, created_at: Timestamp, relays: Vec<(String, Option<String>)> },
    /// NIP-17 inbox relay list (kind 10050): where to deliver DMs to `author`.
    DmRelays { author: PubKey, created_at: Timestamp, relays: Vec<String> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignored_carries_kind_and_reason() {
        let i = Inbound::ignored(1234, "no handler");
        assert_eq!(i.family(), "ignored");
        match i {
            Inbound::Ignored { kind, reason } => {
                assert_eq!(kind, 1234);
                assert_eq!(reason, "no handler");
            }
            _ => panic!("expected Ignored"),
        }
    }

    #[test]
    fn inbound_is_serializable_for_ui_and_logs() {
        let i = Inbound::ignored(1, "x");
        let s = serde_json::to_string(&i).unwrap();
        assert!(s.contains("\"family\":\"ignored\""));
        let back: Inbound = serde_json::from_str(&s).unwrap();
        assert_eq!(back.family(), "ignored");
    }
}
