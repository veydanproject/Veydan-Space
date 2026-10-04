// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Wire event → `Inbound`. Pure with respect to storage; the only input
//! besides the event is the optional signer needed to open gift wraps.

use messenger_core::inbound::Envelope as WireEnvelope;
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
    if rumor.kind.as_u16() != KIND_DM_RUMOR {
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
