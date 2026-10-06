// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Ingress: the seam between transport and handlers.
//!
//! ```text
//! RawEvent ─▶ dedup (msg_events_raw) ─▶ classify ─▶ Inbound ─▶ Dispatcher ─▶ Handler ─▶ Effects
//! ```
//!
//! - `classify`: signature check, unwrap of gift wraps, mapping of kinds to
//!   `Inbound` families. Never panics, never errors: anything it cannot
//!   place becomes `Inbound::Ignored { reason }`.
//! - `dispatch`: routes `Inbound` to the registered handler with the
//!   ordering rules (one DM peer at a time, historical/live flag) and runs
//!   the returned effects through an `EffectSink`.
//! - `outbox`: persistent queue of `Outbound` requests with backoff; a retry
//!   republishes the same signed event.
//! - `filters`: the relay filters the runtime subscribes with.
//! - `r#loop`: `IngressLoop`, the task that ties the above to a transport.

pub mod classify;
pub mod dispatch;
pub mod filters;
pub mod outbox;
pub mod r#loop;

pub use classify::classify;
pub use dispatch::{Dispatcher, EffectSink, Fanout};
pub use outbox::Outbox;
pub use r#loop::{is_transient, IngressLoop, IngressStats, TRANSIENT_KINDS};
