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
//! - `social`: links to the user's profiles elsewhere, checked per platform.
//! - `phone`: phone numbers in one form; private, never in kind 0.
//! - `card`: a contact card sent as a message, and the checks of a received one.
//!
//! No relay access here: the runtime subscribes and publishes; this crate
//! only interprets events and keeps state.

pub mod book;
pub mod card;
pub mod handler;
pub mod nip05;
pub mod phone;
pub mod profile;
pub mod social;

pub use book::{ContactPatch, ContactService, ContactView};
pub use card::{CardView, ContactCard};
pub use handler::{MetaHandler, UI_EVENT_CONTACTS_UPDATED, UI_EVENT_FOLLOWS_UPDATED, UI_EVENT_PROFILE_UPDATED};
pub use nip05::{Nip05Fetcher, Nip05Service, ReqwestFetcher};
pub use phone::normalize_phone;
pub use profile::{Picture, ProfileInput, ProfileService, ProfileView};
pub use social::{SocialLink, SocialPlatform, SocialView, MAX_SOCIALS};
