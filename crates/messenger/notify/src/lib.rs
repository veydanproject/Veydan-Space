// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a push is about, worked out on the phone.
//!
//! A push server sees only the outside of an event. The phone has the keys
//! and the messenger's database, so it is the phone that says who wrote,
//! what, and in which chat, and whether that chat is to be quiet. This
//! crate is that: an event in, a notice out, nothing written anywhere.
//!
//! It runs where the app does not: in a process a push started, with no
//! window and no runtime. So it opens the database read-only, takes the
//! keys as a bundle the app exported, and uses no network but one short
//! question to a relay when the push could not carry the event.

pub mod bundle;
pub mod describe;
pub mod fetch;
pub mod live;
pub mod net;
pub mod notice;
pub mod push;
pub mod settings;

pub use bundle::{GroupKeyEntry, KeyBundle};
pub use describe::describe;
pub use live::{live, Face};
pub use notice::{Body, ChatKind, LinkKind, Notice, Outcome, Plain, Reason};
pub use push::{PushData, PushKind};
pub use settings::{Content, DesktopSettings, Settings};
