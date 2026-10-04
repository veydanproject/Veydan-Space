// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Client of a push server (VPush).
//!
//! A push server watches relays for the user and wakes the phone when
//! something arrives. This crate is how the messenger talks to one: what is
//! said ([`dto`]), how a request proves who is asking ([`nip98`]), and the
//! requests themselves ([`client`]).
//!
//! The server is somebody's server. Nothing is sent to it that it does not
//! need: no keys of relays, no contacts, no message. What it does learn is
//! which relays and groups to watch, and what the user calls the groups.

pub mod client;
pub mod dto;
pub mod nip98;

pub use client::{PushError, VpushClient};
pub use dto::*;
