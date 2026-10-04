// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The way to the project's servers through a bridge.
//!
//! Where the direct way to the servers is throttled, the messenger goes
//! through a *bridge*: a machine inside the country that anybody may run.
//! The bridge passes each connection to a *hub* outside, and the hub to
//! the server. What travels is the messenger's own TLS to that server: a
//! bridge and a hub move bytes they cannot read.
//!
//! ```text
//! reqwest ── proxy rule ──▶ the SOCKS door on 127.0.0.1 ─┐
//! nostr-sdk ── websocket transport ──────────────────────┤
//!                                                        ▼
//!                                      [`Net`]: by the name of the host
//!                                      a server of the project, bridges on → a bridge
//!                                      anything else                       → as before
//! ```
//!
//! [`Net`] is the one place that decides. Nothing here touches the system:
//! no resolver, no certificate store. It builds the same for a computer
//! and for a phone.
//!
//! # Where the code comes from
//!
//! `net.rs` and this file are written here. The client itself is not: it
//! is VLink, the bridge system, which lives in this repository in a
//! workspace of its own (`services/link/`). This crate depends on its crates
//! `vlink-proto` and `vlink-client` by path and gives them out under the
//! names below, so that the rest of the messenger knows one crate only.

pub mod net;
#[cfg(feature = "testing")]
pub mod testing;

pub use net::{Net, NetConfig, Route};
/// Opening streams through a bridge, and what is built into every client.
pub use vlink_client::{self as client, probe, socks, trust, Client, Error};
/// What a client shares with the bridges and the hubs: who a bridge is,
/// how it is called, how a list of bridges is checked.
pub use vlink_proto::{self as proto, BridgeId, BridgeRef, H2Stream, Target};
