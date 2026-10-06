// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The beat: an empty kind 30315 with `d = veydan`, signed by the presence
//! key. Addressable, so a relay keeps only the newest one per key. "Gone"
//! is never sent: a beat expires by itself.

use messenger_core::outbound::WireEvent;
use messenger_core::{EventId, MessengerError, Result};
use nostr::key::Keys;
use nostr::prelude::*;

pub use messenger_core::presence::{KIND_PRESENCE, PRESENCE_D};

/// How long a beat says the user is online. More than two beats, so one
/// lost beat does not show the user gone.
pub const ONLINE_TTL_SECS: i64 = 80;
/// How often to beat while the app is in sight.
pub const BEAT_SECS: i64 = 30;
/// A beat comes up to this much earlier or later than [`BEAT_SECS`], so
/// beats of many users do not fall on one second.
pub const BEAT_JITTER_SECS: i64 = 5;

// Two beats, each late by all its jitter, fit in one TTL: one lost beat
// does not show the user gone.
const _: () = assert!(2 * (BEAT_SECS + BEAT_JITTER_SECS) <= ONLINE_TTL_SECS);

fn crypto(e: impl std::fmt::Display) -> MessengerError {
    MessengerError::Crypto(e.to_string())
}

/// A beat made at `now`: online until `now + ONLINE_TTL_SECS`, which is
/// also its NIP-40 expiration, so a relay may forget it.
pub fn build(presence_keys: &Keys, now: i64) -> Result<WireEvent> {
    let expiration = (now + ONLINE_TTL_SECS).to_string();
    let event = EventBuilder::new(Kind::from(KIND_PRESENCE), "")
        .tag(Tag::parse(["d", PRESENCE_D]).map_err(crypto)?)
        .tag(Tag::parse(["expiration", expiration.as_str()]).map_err(crypto)?)
        .custom_created_at(nostr::types::Timestamp::from_secs(now.max(0) as u64))
        .finalize(presence_keys)
        .map_err(crypto)?;
    Ok(WireEvent {
        id: EventId::parse(&event.id.to_hex()).ok_or_else(|| MessengerError::Crypto("bad event id".into()))?,
        json: serde_json::to_value(&event)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_beat_is_signed_by_the_presence_key_and_says_for_how_long() {
        let presence = crate::key::derive(&Keys::generate(), 0);
        let wire = build(&presence, 1_759_700_000).unwrap();
        let event: Event = serde_json::from_value(wire.json.clone()).unwrap();
        event.verify().unwrap();
        assert_eq!(event.id.to_hex(), wire.id.as_hex());
        assert_eq!(event.pubkey, presence.public_key());
        assert_eq!(event.kind.as_u16(), 30315);
        assert_eq!(event.created_at.as_secs(), 1_759_700_000);
        assert_eq!(event.content, "");
        let tags: Vec<Vec<String>> = event.tags.iter().map(|t| t.as_slice().to_vec()).collect();
        assert_eq!(tags, vec![vec!["d".to_string(), "veydan".into()], vec!["expiration".into(), "1759700080".into()]]);
    }

    #[test]
    fn the_wire_constants_are_fixed() {
        assert_eq!(KIND_PRESENCE, 30315);
        assert_eq!(PRESENCE_D, "veydan");
        assert_eq!((ONLINE_TTL_SECS, BEAT_SECS, BEAT_JITTER_SECS), (80, 30, 5));
    }
}
