// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! NIP-17 wrapping. One rumor, two gift wraps: one the peer can open, one
//! we can open (self-copy: history on our other devices and after a
//! relogin). Both carry the same rumor, so the rumor id identifies the
//! message everywhere.
//!
//! A wrap says on its outside whether it is worth a push: see
//! [`crate::pushtags`]. The copy for ourselves never is.

use crate::pushtags;
use messenger_core::outbound::WireEvent;
use messenger_core::{EventId, MessengerError, PubKey, Result};
use nostr::key::Keys;
use nostr::nips::nip59::GiftWrapBuilder;
use nostr::prelude::*;

pub struct Wrapped {
    pub rumor_id: EventId,
    pub to_peer: WireEvent,
    /// `None` when the peer is ourselves (notes to self need one copy).
    pub to_self: Option<WireEvent>,
}

/// Is the peer's copy worth waking the peer's phone for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wake {
    /// A message: something for a person to read.
    Peer,
    /// A signal of the protocol: for the peer's app, not for the peer.
    Nobody,
}

fn crypto(e: impl std::fmt::Display) -> MessengerError {
    MessengerError::Crypto(e.to_string())
}

/// A message to `peer`. The same as [`wrap_as`] with [`Wake::Peer`].
pub fn wrap(keys: &Keys, peer: &PubKey, content: &str, created_at: i64, reply_to: Option<&str>) -> Result<Wrapped> {
    wrap_as(keys, peer, content, created_at, reply_to, Wake::Peer)
}

pub fn wrap_as(
    keys: &Keys,
    peer: &PubKey,
    content: &str,
    created_at: i64,
    reply_to: Option<&str>,
    wake: Wake,
) -> Result<Wrapped> {
    let receiver = PublicKey::from_hex(peer.as_hex()).map_err(crypto)?;
    let me = keys.public_key();
    let mut builder = EventBuilder::new(Kind::PrivateDirectMessage, content)
        .tag(Tag::public_key(receiver))
        .custom_created_at(nostr::types::Timestamp::from_secs(created_at.max(0) as u64));
    if let Some(id) = reply_to {
        builder = builder.tag(Tag::parse(["e", id]).map_err(crypto)?);
    }
    let mut rumor = builder.finalize_unsigned(me);
    rumor.ensure_id();
    let rumor_id = rumor
        .id
        .and_then(|id| EventId::parse(&id.to_hex()))
        .ok_or_else(|| MessengerError::Crypto("rumor has no id".into()))?;

    let seal = |to: PublicKey, quiet: bool| -> Result<WireEvent> {
        let mut gift = GiftWrapBuilder::new(to, rumor.clone());
        if quiet {
            gift = gift.extra_tags([pushtags::silent_tag()?]);
        }
        wire(gift.finalize(keys).map_err(crypto)?)
    };
    // A note to oneself is one's own copy, and nothing else.
    let to_peer = seal(receiver, wake == Wake::Nobody || receiver == me)?;
    let to_self = if receiver == me { None } else { Some(seal(me, true)?) };
    Ok(Wrapped { rumor_id, to_peer, to_self })
}

fn wire(event: Event) -> Result<WireEvent> {
    Ok(WireEvent {
        id: EventId::parse(&event.id.to_hex()).ok_or_else(|| MessengerError::Crypto("bad event id".into()))?,
        json: serde_json::to_value(&event)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::nips::nip59::UnwrappedGift;

    fn pk(k: &Keys) -> PubKey {
        PubKey::parse(&k.public_key().to_hex()).unwrap()
    }

    #[test]
    fn both_copies_carry_the_same_rumor() {
        let alice = Keys::generate();
        let bob = Keys::generate();
        let w = wrap(&alice, &pk(&bob), "hi", 1_700_000_000, Some(&"ab".repeat(32))).unwrap();
        let peer_ev: Event = serde_json::from_value(w.to_peer.json.clone()).unwrap();
        let self_ev: Event = serde_json::from_value(w.to_self.as_ref().unwrap().json.clone()).unwrap();
        assert_ne!(peer_ev.id, self_ev.id, "two different wraps");

        let by_bob = UnwrappedGift::from_gift_wrap(&bob, &peer_ev).unwrap();
        let by_alice = UnwrappedGift::from_gift_wrap(&alice, &self_ev).unwrap();
        assert!(UnwrappedGift::from_gift_wrap(&bob, &self_ev).is_err(), "bob cannot open alice's copy");
        for u in [&by_bob, &by_alice] {
            let mut r = u.rumor.clone();
            r.ensure_id();
            assert_eq!(r.id.unwrap().to_hex(), w.rumor_id.as_hex());
            assert_eq!(r.created_at.as_secs(), 1_700_000_000);
            assert_eq!(r.content, "hi");
            assert_eq!(u.sender, alice.public_key());
            assert!(r.tags.iter().any(|t| t.kind() == "e"));
        }
    }

    #[test]
    fn note_to_self_has_one_copy() {
        let me = Keys::generate();
        let w = wrap(&me, &pk(&me), "memo", 1, None).unwrap();
        assert!(w.to_self.is_none());
        assert!(silent(&w.to_peer), "one's own note wakes nobody");
    }

    /// Is the wrap marked, on its outside, as not worth a push.
    fn silent(w: &WireEvent) -> bool {
        let event: Event = serde_json::from_value(w.json.clone()).unwrap();
        event.tags.iter().any(|t| t.as_slice() == ["silent", "1"])
    }

    fn recipient(w: &WireEvent) -> String {
        let event: Event = serde_json::from_value(w.json.clone()).unwrap();
        let p: Vec<_> = event.tags.iter().filter(|t| t.kind() == "p").collect();
        assert_eq!(p.len(), 1, "a wrap is addressed to one");
        p[0].as_slice()[1].clone()
    }

    #[test]
    fn a_message_wakes_the_peer_and_never_the_author() {
        let (alice, bob) = (Keys::generate(), Keys::generate());
        let w = wrap(&alice, &pk(&bob), "hi", 1_700_000_000, None).unwrap();
        assert!(!silent(&w.to_peer));
        assert!(silent(w.to_self.as_ref().unwrap()));
        assert_eq!(recipient(&w.to_peer), bob.public_key().to_hex());
        assert_eq!(recipient(w.to_self.as_ref().unwrap()), alice.public_key().to_hex());
    }

    #[test]
    fn a_signal_wakes_nobody_and_is_opened_like_a_message() {
        let (alice, bob) = (Keys::generate(), Keys::generate());
        let w = wrap_as(&alice, &pk(&bob), "signal", 1_700_000_000, None, Wake::Nobody).unwrap();
        assert!(silent(&w.to_peer));
        assert!(silent(w.to_self.as_ref().unwrap()));

        // The tag is on the outside; what is inside is as it was.
        let event: Event = serde_json::from_value(w.to_peer.json.clone()).unwrap();
        event.verify().unwrap();
        let opened = UnwrappedGift::from_gift_wrap(&bob, &event).unwrap();
        assert_eq!(opened.rumor.content, "signal");
        assert_eq!(opened.sender, alice.public_key());
        assert!(!opened.rumor.tags.iter().any(|t| t.kind() == "silent"));
    }
}
