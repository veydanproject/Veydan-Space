// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Notes to a key that is not a contact, and the events around them
//! (internal/messenger-wire.md §3, "Восстановление доступа и связывание
//! устройств"; internal/cloud-recovery.md §6):
//!
//! - [`wrap_note_to_key`]: a note (rumor kind 30079, quiet, no copy to
//!   myself) to a temporary key T, whose outer key is kept, so that the
//!   wrap can be deleted (NIP-09) once it served;
//! - [`wrap_own_now`]: a note to my own devices stamped with the real
//!   time, so that a device waiting for it asks the relays for minutes,
//!   not for the two days a tweaked wrap may lie in the past;
//! - [`link_code_event`]: the open event of a typed link code, kind 30078
//!   signed by T;
//! - [`deletion`]: a NIP-09 request signed by the key of the events;
//! - [`open_wrap`]: a wrap opened with a key that is not the identity.
//!
//! Nothing here talks to a relay.

use crate::pushtags;
use messenger_core::envelope::{Envelope, KIND_LINK_CODE, KIND_OWN_RUMOR, KIND_PEER_NOTE_RUMOR};
use messenger_core::outbound::WireEvent;
use messenger_core::{EventId, MessengerError, PubKey, Result};
use nostr::nips::nip44;
use nostr::nips::nip59::{GiftWrapSealBuilder, UnwrappedGift};
use nostr::prelude::*;

/// A wrap and the key that signed its outside: only that key may ask the
/// relays to delete it.
pub struct KeptWrap {
    /// The gift wrap as it goes to the relays.
    pub wrap: WireEvent,
    /// The key that signed its outside.
    pub wrapper: Keys,
}

fn crypto(e: impl std::fmt::Display) -> MessengerError {
    MessengerError::Crypto(e.to_string())
}

fn at(secs: i64) -> Timestamp {
    Timestamp::from_secs(secs.max(0) as u64)
}

fn wire(event: Event) -> Result<WireEvent> {
    Ok(WireEvent {
        id: EventId::parse(&event.id.to_hex()).ok_or_else(|| crypto("bad event id"))?,
        json: serde_json::to_value(&event)?,
    })
}

fn public(key: &PubKey) -> Result<PublicKey> {
    PublicKey::from_hex(key.as_hex()).map_err(crypto)
}

/// The gift wrap of `rumor` to `receiver`, sealed by `keys`, its outside
/// signed by `wrapper` at `created_at` with `outside` tags.
fn gift(
    keys: &Keys,
    receiver: PublicKey,
    rumor: UnsignedEvent,
    wrapper: &Keys,
    created_at: i64,
    outside: Vec<Tag>,
) -> Result<WireEvent> {
    let seal = GiftWrapSealBuilder::new(rumor, receiver)
        .finalize(keys)
        .map_err(crypto)?;
    let content = nip44::encrypt(
        wrapper.secret_key(),
        &receiver,
        seal.as_json(),
        nip44::Version::V2,
    )
    .map_err(crypto)?;
    let mut tags = outside;
    tags.push(Tag::public_key(receiver));
    wire(
        EventBuilder::new(Kind::GiftWrap, content)
            .tags(tags)
            .custom_created_at(at(created_at))
            .finalize(wrapper)
            .map_err(crypto)?,
    )
}

/// A note to the key `to` (rumor kind 30079, sealed by `keys`): quiet on the
/// outside (`silent`), no copy to myself, forgotten by the relays at
/// `expiration` (NIP-40). The outer key is returned with the wrap. The
/// wrap is stamped with the real time: the relay sees when it came anyway,
/// and T is a key of one procedure.
pub fn wrap_note_to_key(
    keys: &Keys,
    to: &PubKey,
    content: &str,
    created_at: i64,
    expiration: i64,
) -> Result<KeptWrap> {
    let receiver = public(to)?;
    let mut rumor = EventBuilder::new(Kind::from(KIND_PEER_NOTE_RUMOR), content)
        .tag(Tag::public_key(receiver))
        .custom_created_at(at(created_at))
        .finalize_unsigned(keys.public_key());
    rumor.ensure_id();
    let wrapper = Keys::generate();
    let outside = vec![pushtags::silent_tag()?, Tag::expiration(at(expiration))];
    let wrap = gift(keys, receiver, rumor, &wrapper, created_at, outside)?;
    Ok(KeptWrap { wrap, wrapper })
}

/// A note to my own devices (rumor kind 30078 sealed and wrapped to my key,
/// `silent`) whose wrap carries the real time: the device that waits for it
/// asks the relays from a few minutes back.
pub fn wrap_own_now(keys: &Keys, content: &str, created_at: i64) -> Result<WireEvent> {
    let me = keys.public_key();
    let mut rumor = EventBuilder::new(Kind::from(KIND_OWN_RUMOR), content)
        .tag(Tag::public_key(me))
        .custom_created_at(at(created_at))
        .finalize_unsigned(me);
    rumor.ensure_id();
    gift(
        keys,
        me,
        rumor,
        &Keys::generate(),
        created_at,
        vec![pushtags::silent_tag()?],
    )
}

/// The open event of a typed link code: kind 30078 signed by T, `d` the hex
/// sha256 of the canonical code, empty, forgotten at `expiration`.
pub fn link_code_event(t: &Keys, d: &str, created_at: i64, expiration: i64) -> Result<WireEvent> {
    let lower = d.len() == 64 && d.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if !lower {
        return Err(MessengerError::Invalid(
            "the d of a link code is 64 hex in lower case".into(),
        ));
    }
    wire(
        EventBuilder::new(Kind::from(KIND_LINK_CODE), "")
            .tags([Tag::identifier(d), Tag::expiration(at(expiration))])
            .custom_created_at(at(created_at))
            .finalize(t)
            .map_err(crypto)?,
    )
}

/// NIP-09: `signer` asks the relays to delete its events `ids` of `kind`.
pub fn deletion(signer: &Keys, ids: &[EventId], kind: u16, created_at: i64) -> Result<WireEvent> {
    if ids.is_empty() {
        return Err(MessengerError::Invalid(
            "a deletion names at least one event".into(),
        ));
    }
    let mut tags = Vec::with_capacity(ids.len() + 1);
    for id in ids {
        tags.push(Tag::event(
            nostr::event::EventId::from_hex(id.as_hex()).map_err(crypto)?,
        ));
    }
    tags.push(Tag::parse(["k", &kind.to_string()]).map_err(crypto)?);
    wire(
        EventBuilder::new(Kind::EventDeletion, "")
            .tags(tags)
            .custom_created_at(at(created_at))
            .finalize(signer)
            .map_err(crypto)?,
    )
}

/// What a wrap to a key held.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyNote {
    /// The id of the rumor, 64 hex.
    pub rumor_id: String,
    /// 30079 for a note from another key, 30078 for one of my own devices.
    pub rumor_kind: u16,
    /// The signer of the seal, which is the author of the rumor.
    pub from: PubKey,
    /// What the note says.
    pub envelope: Envelope,
    /// The `created_at` of the rumor, unix seconds.
    pub created_at: i64,
}

/// A gift wrap opened with `keys`: a note (rumor kind 30079, or 30078 sealed
/// by `keys` itself) whose author is the signer of its seal and whose
/// content is an envelope. Anything else is `None`.
pub fn open_wrap(keys: &Keys, event: &Event) -> Option<KeyNote> {
    let opened = UnwrappedGift::from_gift_wrap(keys, event).ok()?;
    let rumor = opened.rumor;
    if rumor.pubkey != opened.sender {
        return None;
    }
    let kind = rumor.kind.as_u16();
    let own = kind == KIND_OWN_RUMOR && opened.sender == keys.public_key();
    if kind != KIND_PEER_NOTE_RUMOR && !own {
        return None;
    }
    // The id of a rumor is the hash of its fields, never the id written
    // beside them (as for a direct message, ingress `classify`): a rumor
    // that claims another id is nobody's.
    rumor.verify_id().ok()?;
    let rumor_id = rumor.compute_id().to_hex();
    let envelope = Envelope::parse(&rumor.content).ok()?;
    Some(KeyNote {
        rumor_id,
        rumor_kind: kind,
        from: PubKey::parse(&opened.sender.to_hex())?,
        envelope,
        created_at: rumor.created_at.as_secs() as i64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::envelope::{T_DEVICE_LINK, T_DEVICE_LINK_DONE};

    fn pk(k: &Keys) -> PubKey {
        PubKey::parse(&k.public_key().to_hex()).unwrap()
    }

    fn event(w: &WireEvent) -> Event {
        serde_json::from_value(w.json.clone()).unwrap()
    }

    fn tag<'a>(e: &'a Event, name: &str) -> Option<&'a str> {
        e.tags
            .iter()
            .find(|t| t.kind() == name)
            .and_then(|t| t.as_slice().get(1))
            .map(String::as_str)
    }

    #[test]
    fn a_note_to_a_key_opens_with_that_key_alone_and_names_its_sealer() {
        let (old, t, other) = (Keys::generate(), Keys::generate(), Keys::generate());
        let content = Envelope::new(T_DEVICE_LINK).with("blob", "AQID").encode();
        let kept = wrap_note_to_key(&old, &pk(&t), &content, 1_791_000_000, 1_791_000_600).unwrap();
        let wrap = event(&kept.wrap);
        assert_eq!(wrap.kind, Kind::GiftWrap);
        assert!(wrap.verify().is_ok());
        assert_eq!(
            wrap.pubkey,
            kept.wrapper.public_key(),
            "the kept key signed the outside"
        );
        assert_eq!(tag(&wrap, "p"), Some(t.public_key().to_hex().as_str()));
        assert_eq!(tag(&wrap, "expiration"), Some("1791000600"));
        assert_eq!(tag(&wrap, "silent"), Some("1"));
        assert_eq!(wrap.created_at.as_secs(), 1_791_000_000);

        let note = open_wrap(&t, &wrap).expect("T opens it");
        assert_eq!(note.rumor_kind, KIND_PEER_NOTE_RUMOR);
        assert_eq!(note.from, pk(&old));
        assert_eq!(note.envelope.t, T_DEVICE_LINK);
        assert_eq!(note.envelope.str_field("blob"), Some("AQID"));
        assert!(open_wrap(&other, &wrap).is_none(), "nobody else opens it");
        assert!(open_wrap(&old, &wrap).is_none(), "and no copy is mine");
    }

    #[test]
    fn an_own_note_carries_the_real_time_and_opens_only_as_mine() {
        let me = Keys::generate();
        let content = Envelope::new(T_DEVICE_LINK_DONE)
            .with("t_pub", "ab".repeat(32))
            .encode();
        let w = event(&wrap_own_now(&me, &content, 1_791_000_123).unwrap());
        assert_eq!(w.created_at.as_secs(), 1_791_000_123);
        assert_eq!(tag(&w, "p"), Some(me.public_key().to_hex().as_str()));
        let note = open_wrap(&me, &w).expect("mine");
        assert_eq!(note.rumor_kind, KIND_OWN_RUMOR);
        assert_eq!(note.envelope.t, T_DEVICE_LINK_DONE);
        assert!(open_wrap(&Keys::generate(), &w).is_none());

        // A kind 30078 rumor sealed by somebody else is not my own note.
        let stranger = Keys::generate();
        let rumor = EventBuilder::new(Kind::from(KIND_OWN_RUMOR), content.as_str())
            .tag(Tag::public_key(me.public_key()))
            .finalize_unsigned(stranger.public_key());
        let forged = gift(
            &stranger,
            me.public_key(),
            rumor,
            &Keys::generate(),
            1,
            vec![],
        )
        .unwrap();
        assert!(open_wrap(&me, &event(&forged)).is_none());
    }

    #[test]
    fn a_note_whose_rumor_claims_an_id_not_its_hash_is_dropped() {
        let (a, t) = (Keys::generate(), Keys::generate());
        let content = Envelope::new(T_DEVICE_LINK).with("blob", "AQID").encode();
        let rumor = |id: Option<nostr::event::EventId>| {
            let mut r = EventBuilder::new(Kind::from(KIND_PEER_NOTE_RUMOR), content.as_str())
                .tag(Tag::public_key(t.public_key()))
                .custom_created_at(at(1_791_000_000))
                .finalize_unsigned(a.public_key());
            r.id = id;
            r
        };
        let honest = rumor(None).compute_id();
        let zeros = nostr::event::EventId::from_hex(&"0".repeat(64)).unwrap();
        let forged = gift(&a, t.public_key(), rumor(Some(zeros)), &Keys::generate(), 1, vec![]).unwrap();
        assert!(open_wrap(&t, &event(&forged)).is_none(), "an id of zeros is not the hash");
        // Without an id, or with its own hash, the note opens under its hash.
        for id in [None, Some(honest)] {
            let w = gift(&a, t.public_key(), rumor(id), &Keys::generate(), 1, vec![]).unwrap();
            assert_eq!(open_wrap(&t, &event(&w)).expect("opens").rumor_id, honest.to_hex());
        }
    }

    #[test]
    fn a_wrap_whose_rumor_is_not_an_envelope_or_a_note_is_dropped() {
        let (a, t) = (Keys::generate(), Keys::generate());
        let kept = wrap_note_to_key(&a, &pk(&t), "not json", 1, 601).unwrap();
        assert!(open_wrap(&t, &event(&kept.wrap)).is_none());
        // A message (kind 14) to T is not a note.
        let rumor = EventBuilder::new(Kind::PrivateDirectMessage, Envelope::text("hi").encode())
            .tag(Tag::public_key(t.public_key()))
            .finalize_unsigned(a.public_key());
        let dm = gift(&a, t.public_key(), rumor, &Keys::generate(), 1, vec![]).unwrap();
        assert!(open_wrap(&t, &event(&dm)).is_none());
    }

    #[test]
    fn the_link_code_event_and_a_deletion_are_signed_by_the_key_of_t() {
        let t = Keys::generate();
        let d = "0f".repeat(32);
        let code = event(&link_code_event(&t, &d, 1_791_000_000, 1_791_000_600).unwrap());
        assert_eq!(code.kind.as_u16(), 30078);
        assert_eq!(code.pubkey, t.public_key());
        assert!(code.content.is_empty());
        assert_eq!(tag(&code, "d"), Some(d.as_str()));
        assert_eq!(tag(&code, "expiration"), Some("1791000600"));
        assert!(code.verify().is_ok());
        assert!(link_code_event(&t, "0F", 1, 2).is_err());
        assert!(link_code_event(&t, &"0F".repeat(32), 1, 2).is_err());

        let id = EventId::parse(&code.id.to_hex()).unwrap();
        let del = event(&deletion(&t, &[id], KIND_LINK_CODE, 1_791_000_100).unwrap());
        assert_eq!(del.kind, Kind::EventDeletion);
        assert_eq!(del.pubkey, t.public_key());
        assert_eq!(tag(&del, "e"), Some(code.id.to_hex().as_str()));
        assert_eq!(tag(&del, "k"), Some("30078"));
        assert!(deletion(&t, &[], 1059, 1).is_err());
    }
}
