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

/// The `d` tag of a focus event: the same kind, signed by a focus key only
/// my devices know, saying which chat I am in now (`messenger_presence::focus`,
/// internal/messenger-wire.md §8 «Фокус»). Not a beat: ingress built before
/// it ignores it as a status of another client.
pub const FOCUS_D: &str = "veydan-focus";

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

/// The newest focus event another device of mine made, as the runtime
/// heard it: `"<chat> <seen_at> <until>"`, `-` for the chat when it said
/// it is in none (`messenger_presence::elsewhere`).
pub const KEY_FOCUS_ELSEWHERE: &str = "focus.elsewhere";

/// The ids of the focus events this device sent last, space-separated,
/// written before each goes: the copy a relay brings back is this
/// device's, whatever its time.
pub const KEY_FOCUS_SENT: &str = "focus.sent";

/// The ids of the focus events of my other devices heard last,
/// space-separated: a copy heard again (another relay, a relay that
/// sends it twice) moves nothing.
pub const KEY_FOCUS_HEARD: &str = "focus.heard";
