// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a presence beat looks like on the wire. Here, not in the presence
//! crate, because ingress classifies the beat and must not depend on the
//! crate that handles it. See internal/messenger-wire.md, "Присутствие".

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

/// When this device last beat (unix seconds, the `created_at` of the beat),
/// written by the runtime before the beat goes, so a copy of it is never
/// taken for my other device's (`messenger_presence::sight`).
pub const KEY_PRESENCE_LAST_BEAT: &str = "presence.last_beat";

/// My presence key now (hex), watched by the runtime with my contacts'.
pub const KEY_PRESENCE_MINE: &str = "presence.mine";

/// The newest beat of my presence key another device of mine made:
/// `"<seen_at> <online_until>"`, read by a phone woken by a push.
pub const KEY_PRESENCE_ELSEWHERE: &str = "presence.elsewhere";
