// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Presence: who of my contacts is online, and when they were last seen.
//!
//! A beat (kind 30315, `d = veydan`, NIP-38) is an open event: anybody who
//! knows the key it is signed with can watch it. Signed with the user's own
//! key, it would tell everybody who knows the user's npub when the user is
//! online, whatever the user said about "contacts only". So the beat is
//! signed by a presence key of its own ([`key::derive`]), made from the
//! user's secret and an epoch, which every device of the user derives
//! alike. The key is told only to approved contacts, by a note inside a
//! direct message (`presence.key`); a contact that is removed or blocked
//! makes the user move to the next epoch, and the new key is told to the
//! contacts that remain. The removed one keeps watching a key that no
//! longer beats.
//!
//! What it does not hide: the operator of a relay sees the beat come over
//! the same connection as the user's own subscriptions, and can link the
//! presence key to the user by that. Anybody else sees a random key that
//! beats twice a minute.
//!
//! The crate holds the key, the beat, the handler of beats that come in and
//! the view. When to beat, whom to tell and when to rotate are the
//! runtime's: nothing here sends anything. The notes go through the DM
//! module and land in `messenger_store::presence`, which is all the two
//! share. See internal/messenger-wire.md, "Присутствие".
//!
//! Beside it, the focus ([`focus`]): the same kind under `d = veydan-focus`,
//! signed by a key of the same making that my devices alone know and no
//! contact is told, saying, sealed, which chat I am in now, so that my
//! other device shows no notification for it. Its own keys, its own
//! epoch, nothing to do with the presence switch; the kind is shared so
//! that a relay keeps and forgets it as a beat, and ingress skips it as
//! one (`TRANSIENT_KINDS`). What another device of mine said of it last,
//! and whether it reads a chat now, is [`elsewhere`]'s.

pub mod elsewhere;
pub mod focus;
pub mod handler;
pub mod heartbeat;
pub mod key;
pub mod sight;
pub mod view;

pub use elsewhere::{FocusHandler, FocusSlot};
pub use focus::{FocusKeys, FOCUS_D, FOCUS_EPOCH, FOCUS_TTL_SECS};
pub use handler::{PresenceHandler, UI_EVENT_PRESENCE_UPDATED};
pub use heartbeat::{BEAT_JITTER_SECS, BEAT_SECS, KIND_PRESENCE, ONLINE_TTL_SECS, PRESENCE_D};
pub use view::PresenceView;
