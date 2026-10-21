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
//! - `call_node`: the link of a private call node with an invitation to it (`veydan://call-node/…`).
//! - `recover`: the link to a trusted person of a request of recovery (`veydan://recover/…`).
//! - `server`: the card of a server (`veydan://server/…`), alone or riding on the link of a person or a group (`srv=`).
//! - `web`: a sign-in to the web cabinet of Veydan Cloud (`veydan://web/…`).
//! - `error`: refusals, as stable codes.
//!
//! Everything in this crate is pure: no network, no storage, no clock.

pub mod bridge;
pub mod call_node;
pub mod contact;
pub mod error;
pub mod kind;
pub mod recover;
pub mod server;
pub mod uri;
pub mod web;

pub use bridge::BridgeLink;
pub use call_node::CallNodeLink;
pub use contact::{contact_link, is_invite_secret, ContactLink, INVITE_LINK_CHARS};
pub use error::LinkError;
pub use kind::LinkType;
pub use recover::RecoverLink;
pub use server::{server_id, ServerLink};
pub use uri::{Uri, UriBuilder, MAX_LINK_LEN, SCHEME};
pub use web::{is_web_id, WebLink};
