// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Messenger contract crate.
//!
//! This is the only crate every other messenger crate may share. It holds:
//!
//! - `types` — identifiers and primitives (`PubKey`, `EventId`, `RelayUrl`, …)
//! - `inbound` / `outbound` — the single seam between transport and the
//!   per-type handlers (`enum Inbound`, `enum Outbound`)
//! - `traits` — `Transport`, `SecretStore`, `Clock`, `Handler`, `Effect`
//! - `config` — `MessengerConfig` (paths and switches passed in by the host)
//! - `error` — `MessengerError`
//! - `emoji` — what may be sent as a reaction
//! - `presence` — the kind and `d` tag of a presence beat
//!
//! Rules (docs/messenger-spec.md §4.2): no Tauri, no application code, no
//! protocol crates beyond plain types. Handlers never see relays; transport
//! never sees message types.

pub mod config;
pub mod emoji;
pub mod envelope;
pub mod error;
pub mod inbound;
pub mod outbound;
pub mod presence;
pub mod traits;
pub mod types;

pub use config::MessengerConfig;
pub use envelope::Envelope;
pub use error::{MessengerError, Result};
pub use inbound::{ChannelInbound, DmInbound, GroupInbound, Inbound, MetaInbound};
pub use outbound::{Outbound, Scope, SyncItem};
pub use traits::{Ack, Body, Clock, Context, Effect, Handler, LinkKind, Notice, SecretStore, Transport};
pub use types::{EventId, EventSource, PubKey, RawEvent, RelayUrl, SubId, Timestamp};

/// Messenger core version, reported by the runtime `status()`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
