// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Wire event → `Inbound`. Pure with respect to storage; the only input
//! besides the event is the optional signer needed to open gift wraps.

use messenger_core::envelope::{KIND_OWN_RUMOR, KIND_PEER_NOTE_RUMOR};
use messenger_core::inbound::Envelope as WireEnvelope;
/// Re-exported: ingress filters and skips beats by these, the presence
/// crate builds them; both take them from core.
pub use messenger_core::presence::{KIND_PRESENCE, PRESENCE_D};
use messenger_core::{ChannelInbound, DmInbound, EventId, GroupInbound, Inbound, MetaInbound, PubKey, RawEvent, Timestamp};
use nostr::key::Keys;
use nostr::nips::nip59::UnwrappedGift;
use nostr::prelude::*;

pub const KIND_PROFILE: u16 = 0;
pub const KIND_FOLLOWS: u16 = 3;
pub const KIND_GROUP_MESSAGE: u16 = 9;
pub const KIND_DM_RUMOR: u16 = 14;
pub const KIND_GIFT_WRAP: u16 = 1059;
pub const KIND_RELAY_LIST: u16 = 10002;
pub const KIND_DM_RELAYS: u16 = 10050;
pub const CHANNEL_KINDS: &[u16] = &[40, 41, 42, 43, 44, 1111];

/// Classify one event. `keys` is the local identity; without it gift wraps
/// are ignored (not an error: the host may be locked).
pub fn classify(raw: &RawEvent, keys: Option<&Keys>) -> Inbound {
    let event: Event = match serde_json::from_value(raw.json.clone()) {
        Ok(e) => e,
        Err(e) => return Inbound::ignored(raw.kind, format!("unparseable event: {e}")),
    };
    if event.verify().is_err() {
        return Inbound::ignored(raw.kind, "invalid signature");
    }
    let envelope = WireEnvelope { wire_id: raw.id.clone(), source: raw.source.clone(), wire_created_at: raw.created_at };

    match raw.kind {
        KIND_GIFT_WRAP => classify_gift_wrap(&event, envelope, keys),
        KIND_GROUP_MESSAGE => classify_group(&event, envelope),
        KIND_PROFILE => Inbound::Meta(MetaInbound::Profile {
            author: pk(&event.pubkey),
            created_at: ts(event.created_at),
            content: event.content.clone(),
        }),
        KIND_FOLLOWS => Inbound::Meta(MetaInbound::Follows {
            author: pk(&event.pubkey),
            created_at: ts(event.created_at),
            follows: event
                .tags
                .iter()
                .filter(|t| t.kind() == "p")
                .filter_map(|t| t.as_slice().get(1))
                .filter_map(|s| PubKey::parse(s))
                .collect(),
        }),
        KIND_RELAY_LIST => Inbound::Meta(MetaInbound::RelayList {
            author: pk(&event.pubkey),
            created_at: ts(event.created_at),
            relays: event
                .tags
                .iter()
                .filter(|t| t.kind() == "r")
                .filter_map(|t| {
                    let s = t.as_slice();
                    Some((s.get(1)?.clone(), s.get(2).cloned()))
                })
                .collect(),
        }),
        KIND_DM_RELAYS => Inbound::Meta(MetaInbound::DmRelays {
            author: pk(&event.pubkey),
            created_at: ts(event.created_at),
            relays: event
                .tags
                .iter()
                .filter(|t| t.kind() == "relay")
                .filter_map(|t| t.as_slice().get(1).cloned())
                .collect(),
        }),
        KIND_PRESENCE => classify_presence(&event),
        k if CHANNEL_KINDS.contains(&k) => Inbound::Channel(ChannelInbound { envelope, kind: k }),
        k => Inbound::ignored(k, "no handler for kind"),
    }
}

fn classify_gift_wrap(event: &Event, envelope: WireEnvelope, keys: Option<&Keys>) -> Inbound {
    let Some(keys) = keys else {
        return Inbound::ignored(KIND_GIFT_WRAP, "no signer to open gift wrap");
    };
    let me = keys.public_key();
    let addressed_to_me = event
        .tags
        .iter()
        .filter(|t| t.kind() == "p")
        .filter_map(|t| t.as_slice().get(1))
        .any(|p| p == &me.to_hex());
    if !addressed_to_me {
        return Inbound::ignored(KIND_GIFT_WRAP, "gift wrap not addressed to us");
    }
    let unwrapped = match UnwrappedGift::from_gift_wrap(keys, event) {
        Ok(u) => u,
        Err(_) => return Inbound::ignored(KIND_GIFT_WRAP, "gift wrap does not open with our key"),
    };
    let rumor = unwrapped.rumor;
    // A note from one of my devices to the others: only I can seal one.
    let own_note = rumor.kind.as_u16() == KIND_OWN_RUMOR && unwrapped.sender == me;
    // A note from a peer may come from anybody; who may send one is for the
    // DM service to decide, as for a message.
    let peer_note = rumor.kind.as_u16() == KIND_PEER_NOTE_RUMOR;
    if rumor.kind.as_u16() != KIND_DM_RUMOR && !own_note && !peer_note {
        return Inbound::ignored(KIND_GIFT_WRAP, format!("rumor kind {} is not a DM", rumor.kind.as_u16()));
    }
    // The seal is signed by the real sender; the rumor's pubkey must match it,
    // otherwise someone is forging the inner author.
    if rumor.pubkey != unwrapped.sender {
        return Inbound::ignored(KIND_GIFT_WRAP, "rumor author does not match seal signer");
    }
    let rumor_id = match rumor.id.map(|id| id.to_hex()).and_then(|h| EventId::parse(&h)) {
        Some(id) => id,
        None => match EventId::parse(&rumor_id_of(&rumor)) {
            Some(id) => id,
            None => return Inbound::ignored(KIND_GIFT_WRAP, "rumor has no id"),
        },
    };
    let recipients: Vec<PubKey> = rumor
        .tags
        .iter()
        .filter(|t| t.kind() == "p")
        .filter_map(|t| t.as_slice().get(1))
        .filter_map(|s| PubKey::parse(s))
        .collect();
    let reply_to = rumor
        .tags
        .iter()
        .filter(|t| t.kind() == "e")
        .filter_map(|t| t.as_slice().get(1))
        .filter_map(|s| EventId::parse(s))
        .next();
    Inbound::Dm(DmInbound {
        envelope,
        rumor_id,
        sender: pk(&unwrapped.sender),
        recipients,
        created_at: ts(rumor.created_at),
        content: rumor.content,
        reply_to,
        rumor_kind: rumor.kind.as_u16(),
    })
}

fn classify_group(event: &Event, envelope: WireEnvelope) -> Inbound {
    let group_id = event
        .tags
        .iter()
        .filter(|t| t.kind() == "h")
        .filter_map(|t| t.as_slice().get(1))
        .next()
        .cloned();
    let Some(group_id) = group_id else {
        return Inbound::ignored(KIND_GROUP_MESSAGE, "group message without h tag");
    };
    let key_id = event
        .tags
        .iter()
        .filter(|t| t.kind() == "k")
        .filter_map(|t| t.as_slice().get(1))
        .next()
        .cloned();
    let reply_to = event
        .tags
        .iter()
        .filter(|t| t.kind() == "e")
        .filter_map(|t| t.as_slice().get(1))
        .filter_map(|s| EventId::parse(s))
        .next();
    Inbound::Group(GroupInbound {
        envelope,
        group_id,
        sender: pk(&event.pubkey),
        created_at: ts(event.created_at),
        kind: KIND_GROUP_MESSAGE,
        key_id,
        ciphertext: event.content.clone(),
        reply_to,
    })
}

/// Id of an unsigned rumor per NIP-01 (sha256 of the canonical array).
/// A beat of our own `d` only: other clients use the same kind for a song
/// or a mood. The expiration is kept as given; the handler decides what a
/// missing one means.
fn classify_presence(event: &Event) -> Inbound {
    let d = event.tags.iter().find(|t| t.kind() == "d").and_then(|t| t.as_slice().get(1));
    if d.map(String::as_str) != Some(PRESENCE_D) {
        return Inbound::ignored(KIND_PRESENCE, "presence status of another client");
    }
    let expires_at = event
        .tags
        .iter()
        .find(|t| t.kind() == "expiration")
        .and_then(|t| t.as_slice().get(1))
        .and_then(|s| s.parse::<i64>().ok())
        .map(Timestamp);
    Inbound::Meta(MetaInbound::Presence { author: pk(&event.pubkey), created_at: ts(event.created_at), expires_at })
}

fn rumor_id_of(rumor: &UnsignedEvent) -> String {
    let mut r = rumor.clone();
    r.ensure_id();
    r.id.map(|id| id.to_hex()).unwrap_or_default()
}

fn pk(p: &PublicKey) -> PubKey {
    PubKey::parse(&p.to_hex()).expect("nostr public keys are 64 hex chars")
}

fn ts(t: nostr::types::Timestamp) -> Timestamp {
    Timestamp(t.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::{EventSource, RelayUrl};
    use nostr::nips::nip17::PrivateDirectMessageBuilder;

    fn raw_of(event: &Event) -> RawEvent {
        RawEvent {
            id: EventId::parse(&event.id.to_hex()).unwrap(),
            kind: event.kind.as_u16(),
            pubkey: PubKey::parse(&event.pubkey.to_hex()).unwrap(),
            created_at: Timestamp(event.created_at.as_secs() as i64),
            json: serde_json::to_value(event).unwrap(),
            source: EventSource::Relay { url: RelayUrl::parse("wss://r.example").unwrap() },
        }
    }

    #[test]
    fn dm_gift_wrap_opens_for_the_recipient_only() {
        let alice = Keys::generate();
        let bob = Keys::generate();
        let eve = Keys::generate();
        let wrap = PrivateDirectMessageBuilder::new(bob.public_key(), r#"{"v":1,"t":"text","text":"hi"}"#)
            .finalize(&alice)
            .unwrap();
        let raw = raw_of(&wrap);

        match classify(&raw, Some(&bob)) {
            Inbound::Dm(dm) => {
                assert_eq!(dm.sender.as_hex(), alice.public_key().to_hex());
                assert_eq!(dm.content, r#"{"v":1,"t":"text","text":"hi"}"#);
                assert!(dm.recipients.iter().any(|p| p.as_hex() == bob.public_key().to_hex()));
                assert_eq!(dm.envelope.wire_id, raw.id);
                assert!(dm.created_at.secs() > 0);
            }
            other => panic!("expected Dm, got {other:?}"),
        }
        assert!(matches!(classify(&raw, Some(&eve)), Inbound::Ignored { .. }), "not addressed to eve");
        assert!(matches!(classify(&raw, None), Inbound::Ignored { .. }), "no signer");
    }

    /// A rumor of `KIND_OWN_RUMOR` wrapped by `author` to `to`.
    fn own_note(author: &Keys, to: &Keys) -> Event {
        let rumor = EventBuilder::new(Kind::from(KIND_OWN_RUMOR), r#"{"v":1,"t":"own.read","chat":"dm:x","at":1}"#)
            .tag(Tag::public_key(to.public_key()))
            .finalize_unsigned(author.public_key());
        nostr::nips::nip59::GiftWrapBuilder::new(to.public_key(), rumor).finalize(author).unwrap()
    }

    #[test]
    fn notes_between_my_devices_come_from_me_only() {
        let me = Keys::generate();
        let eve = Keys::generate();
        match classify(&raw_of(&own_note(&me, &me)), Some(&me)) {
            Inbound::Dm(dm) => {
                assert_eq!(dm.rumor_kind, KIND_OWN_RUMOR);
                assert_eq!(dm.sender.as_hex(), me.public_key().to_hex());
            }
            other => panic!("expected Dm, got {other:?}"),
        }
        assert!(matches!(classify(&raw_of(&own_note(&eve, &me)), Some(&me)), Inbound::Ignored { .. }), "sealed by someone else");
    }

    /// A rumor of `kind` wrapped by `author` to `to`.
    fn rumor_of_kind(kind: u16, author: &Keys, to: &Keys) -> Event {
        let rumor = EventBuilder::new(Kind::from(kind), r#"{"v":1,"t":"receipt.read","at":1}"#)
            .tag(Tag::public_key(to.public_key()))
            .finalize_unsigned(author.public_key());
        nostr::nips::nip59::GiftWrapBuilder::new(to.public_key(), rumor).finalize(author).unwrap()
    }

    #[test]
    fn a_note_from_a_peer_comes_from_anybody_and_nothing_else_does() {
        let me = Keys::generate();
        let stranger = Keys::generate();
        match classify(&raw_of(&rumor_of_kind(KIND_PEER_NOTE_RUMOR, &stranger, &me)), Some(&me)) {
            Inbound::Dm(dm) => {
                assert_eq!(dm.rumor_kind, KIND_PEER_NOTE_RUMOR);
                assert_eq!(dm.sender.as_hex(), stranger.public_key().to_hex());
                assert_eq!(dm.content, r#"{"v":1,"t":"receipt.read","at":1}"#);
            }
            other => panic!("expected Dm, got {other:?}"),
        }
        assert!(
            matches!(classify(&raw_of(&rumor_of_kind(KIND_OWN_RUMOR, &stranger, &me)), Some(&me)), Inbound::Ignored { .. }),
            "a note between my devices is still mine only"
        );
        assert!(
            matches!(classify(&raw_of(&rumor_of_kind(15, &stranger, &me)), Some(&me)), Inbound::Ignored { .. }),
            "any other kind is not a DM"
        );
    }

    #[test]
    fn tampered_signature_and_unknown_kinds_are_ignored() {
        let k = Keys::generate();
        let ev = EventBuilder::new(Kind::from(1u16), "note").finalize(&k).unwrap();
        let mut raw = raw_of(&ev);
        assert!(matches!(classify(&raw, None), Inbound::Ignored { kind: 1, .. }));
        raw.json["content"] = serde_json::Value::String("changed".into());
        match classify(&raw, None) {
            Inbound::Ignored { reason, .. } => assert!(reason.contains("signature")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn meta_kinds_are_parsed() {
        let k = Keys::generate();
        let follow = Keys::generate();
        let profile = EventBuilder::new(Kind::from(0u16), r#"{"name":"a"}"#).finalize(&k).unwrap();
        assert!(matches!(classify(&raw_of(&profile), None), Inbound::Meta(MetaInbound::Profile { .. })));

        let follows = EventBuilder::new(Kind::from(3u16), "")
            .tag(Tag::public_key(follow.public_key()))
            .finalize(&k)
            .unwrap();
        match classify(&raw_of(&follows), None) {
            Inbound::Meta(MetaInbound::Follows { follows, .. }) => {
                assert_eq!(follows.len(), 1);
                assert_eq!(follows[0].as_hex(), follow.public_key().to_hex());
            }
            other => panic!("{other:?}"),
        }

        let relays = EventBuilder::new(Kind::from(10002u16), "")
            .tag(Tag::parse(["r", "wss://a.example", "read"]).unwrap())
            .tag(Tag::parse(["r", "wss://b.example"]).unwrap())
            .finalize(&k)
            .unwrap();
        match classify(&raw_of(&relays), None) {
            Inbound::Meta(MetaInbound::RelayList { relays, .. }) => {
                assert_eq!(relays.len(), 2);
                assert_eq!(relays[0], ("wss://a.example".to_string(), Some("read".to_string())));
                assert_eq!(relays[1].1, None);
            }
            other => panic!("{other:?}"),
        }
    }

    fn beat(k: &Keys, d: &str, expiration: Option<&str>) -> Event {
        let mut b = EventBuilder::new(Kind::from(KIND_PRESENCE), "")
            .tag(Tag::parse(["d", d]).unwrap())
            .custom_created_at(nostr::types::Timestamp::from(1_759_700_000u64));
        if let Some(e) = expiration {
            b = b.tag(Tag::parse(["expiration", e]).unwrap());
        }
        b.finalize(k).unwrap()
    }

    #[test]
    fn a_presence_beat_of_ours_is_meta_with_its_expiration() {
        let k = Keys::generate();
        match classify(&raw_of(&beat(&k, PRESENCE_D, Some("1759700080"))), None) {
            Inbound::Meta(MetaInbound::Presence { author, created_at, expires_at }) => {
                assert_eq!(author.as_hex(), k.public_key().to_hex());
                assert_eq!(created_at, Timestamp(1_759_700_000));
                assert_eq!(expires_at, Some(Timestamp(1_759_700_080)));
            }
            other => panic!("expected Meta::Presence, got {other:?}"),
        }
    }

    #[test]
    fn a_presence_beat_without_a_good_expiration_has_none() {
        let k = Keys::generate();
        for exp in [None, Some("soon"), Some("")] {
            match classify(&raw_of(&beat(&k, PRESENCE_D, exp)), None) {
                Inbound::Meta(MetaInbound::Presence { expires_at, .. }) => assert_eq!(expires_at, None, "{exp:?}"),
                other => panic!("expected Meta::Presence, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_status_of_another_client_is_ignored() {
        let k = Keys::generate();
        for d in ["general", "music", ""] {
            assert!(
                matches!(classify(&raw_of(&beat(&k, d, Some("1759700080"))), None), Inbound::Ignored { kind: KIND_PRESENCE, .. }),
                "d = {d:?}"
            );
        }
        let no_d = EventBuilder::new(Kind::from(KIND_PRESENCE), "").finalize(&k).unwrap();
        assert!(matches!(classify(&raw_of(&no_d), None), Inbound::Ignored { .. }), "no d at all");
    }

    #[test]
    fn group_message_needs_h_tag() {
        let k = Keys::generate();
        let no_h = EventBuilder::new(Kind::from(9u16), "cipher").finalize(&k).unwrap();
        assert!(matches!(classify(&raw_of(&no_h), None), Inbound::Ignored { kind: 9, .. }));
        let with_h = EventBuilder::new(Kind::from(9u16), "cipher")
            .tag(Tag::parse(["h", "group1"]).unwrap())
            .tag(Tag::parse(["k", "abc"]).unwrap())
            .finalize(&k)
            .unwrap();
        match classify(&raw_of(&with_h), None) {
            Inbound::Group(g) => {
                assert_eq!(g.group_id, "group1");
                assert_eq!(g.key_id.as_deref(), Some("abc"));
                assert_eq!(g.ciphertext, "cipher");
            }
            other => panic!("{other:?}"),
        }
    }
}
