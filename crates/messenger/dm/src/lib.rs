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
//! - `view`: what the host shows; `body`: what a notification tells of a message.

pub mod body;
pub mod handler;
pub mod media;
pub mod relations;
pub mod relationship;
pub mod service;
pub mod view;
pub mod pushtags;
pub mod wrap;

pub use handler::{DmHandler, DmRoutesHandler, UI_EVENT_CHATS_UPDATED, UI_EVENT_DM_MESSAGE, UI_EVENT_DM_UPDATED};
pub use relations::{ActionResult, RelationView, UI_EVENT_DM_RELATIONSHIP};
pub use relationship::Action;
pub use service::{DmService, Prepared};
pub use view::{ChatView, MessageView};
