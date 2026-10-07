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
use messenger_core::envelope::KIND_PEER_NOTE_RUMOR;
use messenger_core::outbound::WireEvent;
use messenger_core::{EventId, MessengerError, PubKey, Result};
use nostr::key::Keys;
use nostr::nips::nip59::GiftWrapBuilder;
use nostr::prelude::*;

pub struct Wrapped {
    pub rumor_id: EventId,
    pub to_peer: WireEvent,
    /// `None` when the peer is ourselves (notes to self need one copy), or
    /// for a note to a peer that my devices need not see.
    pub to_self: Option<WireEvent>,
}

/// Is the peer's copy worth waking the peer's phone for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wake {
    /// A message: something for a person to read.
    Peer,
    /// A signal of the protocol: for the peer's app, not for the peer.
    Nobody,
    /// A call: the peer's phone rings now or never. The push server sends
    /// it at once, past the throttle of a chat, and keeps it for a minute,
    /// not a day (`["call", "1"]` on the outside; services/push/spec). The
    /// price: a relay sees that the wrap is a call, not what call.
    Call,
}

/// `["call", "1"]`: the tag of a call on the outside of a wrap. Spelled
/// here and in `vpush-server::pipeline::classify`; the two must agree.
pub const CALL_TAG: &str = "call";

fn call_tag() -> Result<Tag> {
    Tag::parse([CALL_TAG, "1"]).map_err(crypto)
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
    wrap_kind(keys, peer, Kind::PrivateDirectMessage.as_u16(), content, created_at, reply_to, wake, true, None)
}

/// A message to `peer` that wakes the peer as a call ([`Wake::Call`]) and
/// is forgotten by the relays at `expiration` (NIP-40, on the outside of
/// both wraps): the invitation to a call, which rings a phone now or not
/// at all. With a self-copy, so my other devices know I called.
pub fn wrap_expiring(keys: &Keys, peer: &PubKey, content: &str, created_at: i64, expiration: i64) -> Result<Wrapped> {
    wrap_kind(keys, peer, Kind::PrivateDirectMessage.as_u16(), content, created_at, None, Wake::Call, true, Some(expiration))
}

/// A note to `peer` that is not a message (a receipt, a reaction, a key): a
/// rumor of `KIND_PEER_NOTE_RUMOR` that wakes nobody, on either copy. A copy
/// for myself only when asked: my other devices learn a receipt on their
/// own, a reaction they need. `expiration` (unix seconds) asks the relays,
/// on the outside of both wraps (NIP-40), to forget the note then.
pub fn wrap_note(
    keys: &Keys,
    peer: &PubKey,
    content: &str,
    created_at: i64,
    self_copy: bool,
    expiration: Option<i64>,
) -> Result<Wrapped> {
    wrap_kind(keys, peer, KIND_PEER_NOTE_RUMOR, content, created_at, None, Wake::Nobody, self_copy, expiration)
}

#[allow(clippy::too_many_arguments)]
fn wrap_kind(
    keys: &Keys,
    peer: &PubKey,
    kind: u16,
    content: &str,
    created_at: i64,
    reply_to: Option<&str>,
    wake: Wake,
    self_copy: bool,
    expiration: Option<i64>,
) -> Result<Wrapped> {
    let receiver = PublicKey::from_hex(peer.as_hex()).map_err(crypto)?;
    let me = keys.public_key();
    let mut builder = EventBuilder::new(Kind::from(kind), content)
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

    let expiration = expiration.map(|at| Tag::expiration(nostr::types::Timestamp::from_secs(at.max(0) as u64)));
    let seal = |to: PublicKey, quiet: bool| -> Result<WireEvent> {
        let mut outside = Vec::new();
        if quiet {
            outside.push(pushtags::silent_tag()?);
        } else if wake == Wake::Call {
            // The peer's copy of a call, and only that: the copy for my
            // own devices is quiet like any other.
            outside.push(call_tag()?);
        }
        outside.extend(expiration.clone());
        let gift = GiftWrapBuilder::new(to, rumor.clone()).extra_tags(outside);
        wire(gift.finalize(keys).map_err(crypto)?)
    };
    // A note to oneself is one's own copy, and nothing else.
    let to_peer = seal(receiver, wake == Wake::Nobody || receiver == me)?;
    let to_self = if receiver == me || !self_copy { None } else { Some(seal(me, true)?) };
    Ok(Wrapped { rumor_id, to_peer, to_self })
}

/// A note for my other devices: a rumor of `KIND_OWN_RUMOR`, sealed and
/// wrapped to my own key, never worth a push.
pub fn wrap_own(keys: &Keys, content: &str, created_at: i64) -> Result<WireEvent> {
    let me = keys.public_key();
    let rumor = EventBuilder::new(Kind::from(messenger_core::envelope::KIND_OWN_RUMOR), content)
        .tag(Tag::public_key(me))
        .custom_created_at(nostr::types::Timestamp::from_secs(created_at.max(0) as u64))
        .finalize_unsigned(me);
    let gift = GiftWrapBuilder::new(me, rumor).extra_tags([pushtags::silent_tag()?]);
    wire(gift.finalize(keys).map_err(crypto)?)
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

    fn tag_value(w: &WireEvent, name: &str) -> Option<String> {
        let event: Event = serde_json::from_value(w.json.clone()).unwrap();
        event.tags.iter().find(|t| t.kind() == name).map(|t| t.as_slice()[1].clone())
    }

    #[test]
    fn a_note_wakes_nobody_and_has_no_copy_unless_asked() {
        let (alice, bob) = (Keys::generate(), Keys::generate());
        let w = wrap_note(&alice, &pk(&bob), r#"{"v":1,"t":"receipt.read","at":1}"#, 1_700_000_000, false, None).unwrap();
        assert!(silent(&w.to_peer));
        assert!(w.to_self.is_none(), "not asked for");
        assert_eq!(recipient(&w.to_peer), bob.public_key().to_hex());
        assert_eq!(tag_value(&w.to_peer, "expiration"), None);

        let event: Event = serde_json::from_value(w.to_peer.json.clone()).unwrap();
        let opened = UnwrappedGift::from_gift_wrap(&bob, &event).unwrap();
        assert_eq!(opened.rumor.kind.as_u16(), KIND_PEER_NOTE_RUMOR);
        assert_eq!(opened.sender, alice.public_key());
        assert!(!opened.rumor.tags.iter().any(|t| t.kind() == "e"), "a note replies to nothing");

        // A note to myself is one copy, whatever was asked.
        let mine = wrap_note(&alice, &pk(&alice), "x", 1, true, None).unwrap();
        assert!(mine.to_self.is_none());
        assert!(silent(&mine.to_peer));
    }

    #[test]
    fn a_note_expires_outside_only_and_both_copies_are_one_rumor() {
        let (alice, bob) = (Keys::generate(), Keys::generate());
        let w = wrap_note(&alice, &pk(&bob), "note", 1_700_000_000, true, Some(1_700_604_800)).unwrap();
        let copy = w.to_self.as_ref().expect("asked for");
        assert!(silent(&w.to_peer));
        assert!(silent(copy));
        assert_eq!(recipient(copy), alice.public_key().to_hex());
        for wrap in [&w.to_peer, copy] {
            assert_eq!(tag_value(wrap, "expiration").as_deref(), Some("1700604800"));
        }

        let by_bob = UnwrappedGift::from_gift_wrap(&bob, &serde_json::from_value(w.to_peer.json.clone()).unwrap()).unwrap();
        let by_alice = UnwrappedGift::from_gift_wrap(&alice, &serde_json::from_value(copy.json.clone()).unwrap()).unwrap();
        for u in [&by_bob, &by_alice] {
            assert!(!u.rumor.tags.iter().any(|t| t.kind() == "expiration"), "the rumor itself does not expire");
            let mut r = u.rumor.clone();
            r.ensure_id();
            assert_eq!(r.id.unwrap().to_hex(), w.rumor_id.as_hex());
            assert_eq!(r.kind.as_u16(), KIND_PEER_NOTE_RUMOR);
        }
    }

    #[test]
    fn an_invitation_wakes_the_peer_and_expires_outside() {
        let (alice, bob) = (Keys::generate(), Keys::generate());
        let w = wrap_expiring(&alice, &pk(&bob), r#"{"v":1,"t":"call.invite"}"#, 1_700_000_000, 1_700_000_060).unwrap();
        let copy = w.to_self.as_ref().expect("my devices learn I called");
        assert!(!silent(&w.to_peer), "a ring");
        assert!(silent(copy));
        for wrap in [&w.to_peer, copy] {
            assert_eq!(tag_value(wrap, "expiration").as_deref(), Some("1700000060"));
        }
        let opened = UnwrappedGift::from_gift_wrap(&bob, &serde_json::from_value(w.to_peer.json.clone()).unwrap()).unwrap();
        assert_eq!(opened.rumor.kind, Kind::PrivateDirectMessage);
        assert!(!opened.rumor.tags.iter().any(|t| t.kind() == "expiration"));
    }

    /// The push server reads `["call", "1"]` off the peer's copy and sends
    /// it past its throttle (the same value is in the tests of
    /// `vpush-server::pipeline::classify`); my own copy is quiet, and a
    /// plain message or a note is no call.
    #[test]
    fn an_invitation_is_marked_as_a_call_on_the_peers_copy_only() {
        let (alice, bob) = (Keys::generate(), Keys::generate());
        let w = wrap_expiring(&alice, &pk(&bob), r#"{"v":1,"t":"call.invite"}"#, 1_700_000_000, 1_700_000_060).unwrap();
        assert_eq!(tag_value(&w.to_peer, CALL_TAG).as_deref(), Some("1"));
        assert!(!silent(&w.to_peer));
        let copy = w.to_self.as_ref().unwrap();
        assert_eq!(tag_value(copy, CALL_TAG), None);
        assert!(silent(copy));
        // The tag is on the outside; the rumor says nothing of it.
        let opened = UnwrappedGift::from_gift_wrap(&bob, &serde_json::from_value(w.to_peer.json.clone()).unwrap()).unwrap();
        assert!(!opened.rumor.tags.iter().any(|t| t.kind() == CALL_TAG));

        let message = wrap(&alice, &pk(&bob), "hi", 1_700_000_000, None).unwrap();
        assert_eq!(tag_value(&message.to_peer, CALL_TAG), None);
        let note = wrap_note(&alice, &pk(&bob), "x", 1_700_000_000, false, None).unwrap();
        assert_eq!(tag_value(&note.to_peer, CALL_TAG), None);
        // A call to myself is my own copy: quiet, and no call.
        let mine = wrap_expiring(&alice, &pk(&alice), "x", 1, 60).unwrap();
        assert!(silent(&mine.to_peer));
        assert_eq!(tag_value(&mine.to_peer, CALL_TAG), None);
    }

    #[test]
    fn a_message_neither_expires_nor_is_a_note() {
        let (alice, bob) = (Keys::generate(), Keys::generate());
        let w = wrap(&alice, &pk(&bob), "hi", 1_700_000_000, None).unwrap();
        assert_eq!(tag_value(&w.to_peer, "expiration"), None);
        assert_eq!(tag_value(w.to_self.as_ref().unwrap(), "expiration"), None);
        let opened = UnwrappedGift::from_gift_wrap(&bob, &serde_json::from_value(w.to_peer.json.clone()).unwrap()).unwrap();
        assert_eq!(opened.rumor.kind, Kind::PrivateDirectMessage);
    }
}
