// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Links of the messenger.
//!
//! Every internal link is `veydan://<type>/<id>?<parameters>`, and this
//! crate is the only place that knows how such a text is taken apart and
//! put together. What a link of a given type means is decided by whoever
//! owns the type: a contact needs nothing but its key and lives here, a
//! group carries a secret and lives with the groups.
//!
//! - `uri`: the grammar.
//! - `kind`: the types of links there are.
//! - `contact`: the link of a person.
//! - `bridge`: the link of a VLink bridge to the project's servers (`veydan://vlink/…`).
//! - `error`: refusals, as stable codes.
//!
//! Everything in this crate is pure: no network, no storage, no clock.

pub mod bridge;
pub mod contact;
pub mod error;
pub mod kind;
pub mod uri;

pub use bridge::BridgeLink;
pub use contact::ContactLink;
pub use error::LinkError;
pub use kind::LinkType;
pub use uri::{Uri, UriBuilder, MAX_LINK_LEN, SCHEME};
