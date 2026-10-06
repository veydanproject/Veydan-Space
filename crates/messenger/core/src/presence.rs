// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a presence beat looks like on the wire. Here, not in the presence
//! crate, because ingress classifies the beat and must not depend on the
//! crate that handles it. See docs/messenger-wire.md, "Присутствие".

/// Kind of a presence beat (NIP-38 user status). Addressable, so a relay
/// keeps one per key and `d`.
pub const KIND_PRESENCE: u16 = 30315;

/// The `d` tag of our beats. A status of another client (`general`, `music`)
/// under the same kind is not ours and is ignored.
pub const PRESENCE_D: &str = "veydan";

/// The setting of the presence switch: whether my contacts see when I am
/// online, and I see them. Here because the handler of beats, the notes of
/// my devices and the runtime all read it; my devices carry it between
/// them (`own.presence`).
pub const KEY_PRESENCE: &str = "privacy.presence";
