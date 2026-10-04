// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Contacts and profiles.
//!
//! - `profile`: kind-0 metadata cache with last-writer-wins, our own
//!   profile, event building for publishing.
//! - `nip05`: `user@domain` resolution and verification over a pluggable
//!   HTTP fetcher (tests inject a fake).
//! - `book`: the private address book (`msg_private_contacts`) and the
//!   public follow list (`msg_follows`).
//! - `handler`: `MetaHandler`, the `Inbound::Meta` consumer.
//!
//! No relay access here: the runtime subscribes and publishes; this crate
//! only interprets events and keeps state.

pub mod book;
pub mod handler;
pub mod nip05;
pub mod profile;

pub use book::{ContactPatch, ContactService, ContactView};
pub use handler::{MetaHandler, UI_EVENT_FOLLOWS_UPDATED, UI_EVENT_PROFILE_UPDATED};
pub use nip05::{Nip05Fetcher, Nip05Service, ReqwestFetcher};
pub use profile::{ProfileInput, ProfileService, ProfileView};
