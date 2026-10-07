// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Direct messages.
//!
//! - `wrap`: rumor (kind 14) → seal → gift wrap for the peer plus a
//!   self-copy, with an application-controlled `created_at`.
//! - `service`: chats and messages (send, edit, delete, read state,
//!   statuses). Returns `Outbound`s; it never talks to relays.
//! - `handler`: `DmHandler` (`Inbound::Dm`) and `DmRoutesHandler`
//!   (inbox relay lists from `Inbound::Meta`).
//! - `own`: notes between my devices (read chats, messages removed for me,
//!   the emoji I use most, the epoch of my presence key, my phone and the
//!   phones my contacts sent me).
//! - `notes`: notes between me and a peer (delivery and read receipts,
//!   reactions, presence keys).
//! - `reactions`: reactions on messages, the limits, and making one.
//! - `cards`: contact cards sent and received as messages.
//! - `view`: what the host shows; `body`: what a notification tells of a message.

pub mod body;
pub mod cards;
pub mod handler;
pub mod media;
pub mod notes;
pub mod own;
pub mod reactions;
pub mod relations;
pub mod relationship;
pub mod service;
pub mod view;
pub mod pushtags;
pub mod wrap;

pub use notes::{presence_proof, presence_proof_ok, KEY_READ_RECEIPTS, RECEIPT_WINDOW_SECS, UI_EVENT_PRESENCE_KEYS_CHANGED};
pub use own::{
    PresenceState, KEY_PRESENCE, KEY_PRESENCE_DEVICES_TOLD, KEY_PRESENCE_EPOCH, KEY_PRESENCE_SINCE, UI_EVENT_CHAT_READ, UI_EVENT_CHAT_RECEIPT,
    UI_EVENT_CONTACT_PRIVATE_UPDATED, UI_EVENT_EMOJI_UPDATED, UI_EVENT_OWN_PRIVATE_UPDATED, UI_EVENT_PRESENCE_EPOCH_CHANGED,
};
pub use reactions::{PreparedReaction, Refusal, MAX_DISTINCT_PER_MESSAGE, MAX_MINE_PER_MESSAGE};
pub use handler::{DmHandler, DmRoutesHandler, UI_EVENT_CHATS_UPDATED, UI_EVENT_DM_MESSAGE, UI_EVENT_DM_UPDATED};
pub use relations::{ActionResult, RelationView, UI_EVENT_DM_RELATIONSHIP};
pub use relationship::Action;
pub use service::{DmService, Prepared};
pub use cards::StoredCard;
pub use view::{CardView, ChatView, MessageView, ReactionView};
