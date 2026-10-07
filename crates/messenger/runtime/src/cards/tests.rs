// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Three people, each a whole runtime kept off the network: what one
//! queues the next hears, through the same classify and dispatch as a
//! relay's events.

use super::*;
use crate::session::RuntimeSink;
use messenger_contacts::{Picture, ProfileInput};
use messenger_core::envelope::{KIND_OWN_RUMOR, T_OWN_CARD, T_OWN_PROFILE};
use messenger_core::{Context, EventSource, MessengerConfig, RawEvent, RelayUrl, Timestamp};
use messenger_testkit::MemorySecretStore;
use nostr::nips::nip59::UnwrappedGift;
use nostr::prelude::Event;
use std::sync::Arc;

async fn started() -> (tempfile::TempDir, MessengerRuntime, Keys) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = MessengerConfig::new(dir.path().join("messenger"));
    let rt = MessengerRuntime::start(cfg, Arc::new(MemorySecretStore::unlocked())).await.unwrap();
    rt.relays().set_silent(true).await.unwrap();
    crate::servers::use_veydan_offline(&rt).await;
    rt.identity().create("pw").await.unwrap();
    assert!(rt.refresh_signer().await.unwrap());
    rt.dm().set_gate(false);
    let keys = rt.session_keys().await.unwrap();
    (dir, rt, keys)
}

fn hex(k: &Keys) -> String {
    k.public_key().to_hex()
}

fn pk(k: &Keys) -> PubKey {
    PubKey::parse(&hex(k)).unwrap()
}

/// Everything `from` has queued, as `to` would hear it from a relay.
async fn deliver(from: &MessengerRuntime, to: &MessengerRuntime) {
    let keys = to.session_keys().await.unwrap();
    let ctx = Context { my_pubkey: pk(&keys), session_started_at: Timestamp(0), clock: Arc::new(SystemClock) };
    let sink = RuntimeSink { pool: to.relays.pool().await, outbox: to.outbox.clone(), ui: to.ui.clone() };
    for row in messenger_store::outbox::due(from.store(), i64::MAX / 4, 0).await.unwrap() {
        let event = match row.outbound().unwrap() {
            Outbound::PublishToInbox { event, .. } | Outbound::PublishOwn { event } => event,
            _ => continue,
        };
        let ev: Event = serde_json::from_value(event.json.clone()).unwrap();
        let raw = RawEvent {
            id: event.id.clone(),
            kind: ev.kind.as_u16(),
            pubkey: PubKey::parse(&ev.pubkey.to_hex()).unwrap(),
            created_at: Timestamp(ev.created_at.as_secs() as i64),
            json: event.json.clone(),
            source: EventSource::Relay { url: RelayUrl::parse("wss://relay.example").unwrap() },
        };
        to.dispatcher.dispatch(messenger_ingress::classify(&raw, Some(&keys)), &ctx, &sink).await;
    }
}

/// The notes `rt` queued for my other devices.
async fn own_notes(rt: &MessengerRuntime, me: &Keys) -> Vec<Envelope> {
    let mut out = Vec::new();
    for row in messenger_store::outbox::due(rt.store(), i64::MAX / 4, 0).await.unwrap() {
        let Ok(Outbound::PublishOwn { event }) = row.outbound() else { continue };
        let ev: Event = serde_json::from_value(event.json.clone()).unwrap();
        let Ok(u) = UnwrappedGift::from_gift_wrap(me, &ev) else { continue };
        if u.rumor.kind.as_u16() == KIND_OWN_RUMOR {
            out.push(Envelope::parse(&u.rumor.content).unwrap());
        }
    }
    out
}

/// The newest message of the direct chat with `peer`.
async fn last_message(rt: &MessengerRuntime, peer: &Keys) -> MessageView {
    let chat = rt.dm().open_chat(&pk(peer)).await.unwrap();
    rt.dm().messages(&chat.id, None, 50).await.unwrap().pop().expect("a message")
}

fn code<T: std::fmt::Debug>(r: Result<T>) -> String {
    match r {
        Err(MessengerError::Invalid(c)) => c,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// An uncompressed 24-bit BMP, `side` square, every pixel `rgb`.
fn bmp(side: u32, rgb: [u8; 3]) -> Vec<u8> {
    let row = (side * 3).div_ceil(4) * 4;
    let size = 54 + row * side;
    let mut out = Vec::with_capacity(size as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(side as i32).to_le_bytes());
    out.extend_from_slice(&(side as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(row * side).to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for _ in 0..side {
        let mut line: Vec<u8> = (0..side).flat_map(|_| [rgb[2], rgb[1], rgb[0]]).collect();
        line.resize(row as usize, 0);
        out.extend_from_slice(&line);
    }
    out
}

#[tokio::test]
async fn my_card_carries_my_phone_and_picture_and_a_passed_on_one_never_a_phone() {
    let (_a, alice, a_keys) = started().await;
    let (_b, bob, b_keys) = started().await;
    let (_c, carol, c_keys) = started().await;

    // Alice: a profile whose picture this device keeps, and a phone.
    let picture = messenger_avatar::square_thumb(&bmp(64, [200, 30, 30]), messenger_avatar::OWN_SIDE).unwrap();
    let sha = messenger_avatar::sha256_hex(&picture);
    let own = alice.config().avatars_dir().join("own");
    std::fs::create_dir_all(&own).unwrap();
    std::fs::write(own.join(format!("{sha}.jpg")), &picture).unwrap();
    let url = format!("https://media.example/{sha}");
    let input = ProfileInput {
        name: Some("alice".into()),
        display_name: Some("Alice".into()),
        about: Some("**Hi**, I am Alice".into()),
        ..Default::default()
    };
    let event = alice.profiles().build_own(&a_keys, &input, Picture::Set(&url)).await.unwrap();
    alice.outbox().enqueue(Outbound::PublishOwn { event }).await.unwrap();
    assert_eq!(code(alice.own_private_set(Some("call me"), true).await), "phone_invalid");
    let set = alice.own_private_set(Some("+1 (555) 000-1111"), true).await.unwrap();
    assert_eq!(set, OwnPrivateView { phone: Some("+15550001111".into()), share_phone: true });
    assert_eq!(alice.own_private_get().await.unwrap(), set);
    let notes = own_notes(&alice, &a_keys).await;
    assert!(notes.iter().any(|n| n.t == T_OWN_PROFILE && n.str_field("phone") == Some("+15550001111")), "my devices are told");
    let now = SystemClock.now().secs();
    assert!(!send_own_profile_if_due(alice.dm(), alice.outbox(), &a_keys, now).await.unwrap(), "just told");
    let week = now + messenger_dm::own::OWN_PROFILE_EVERY_SECS;
    assert!(send_own_profile_if_due(alice.dm(), alice.outbox(), &a_keys, week).await.unwrap(), "told again a week on");
    assert_eq!(own_notes(&alice, &a_keys).await.iter().filter(|n| n.t == T_OWN_PROFILE).count(), 2);

    // Unticked, my card has no phone; ticked, it has.
    let plain = alice.card_send(&hex(&b_keys), None, false).await.unwrap();
    assert_eq!(plain.card.as_ref().unwrap().phone, None);
    let sent = alice.card_send(&format!("dm:{}", hex(&b_keys)), None, true).await.unwrap();
    let card = sent.card.as_ref().expect("a card");
    assert_eq!((card.label.as_str(), card.phone.as_deref(), card.is_me), ("Alice", Some("+15550001111"), true));
    assert!(card.avatar.as_deref().is_some_and(|a| a.starts_with("data:image/jpeg;base64,")), "my picture, from my file");
    assert_eq!(code(alice.card_accept(&sent.id).await), "card_is_me");

    // Bob gets it: the phone and the picture came with it.
    deliver(&alice, &bob).await;
    let got = last_message(&bob, &a_keys).await;
    let card = got.card.clone().expect("a card");
    assert_eq!((card.pubkey.as_str(), card.phone.as_deref()), (hex(&a_keys).as_str(), Some("+15550001111")));
    assert!(card.avatar.is_some() && !card.is_contact && !card.is_me);
    assert!(!card.bio.is_empty());
    assert_eq!(bob.dm().open_chat(&pk(&a_keys)).await.unwrap().last_preview.as_deref(), Some("👤 Alice"));
    assert_eq!(bob.contact_private_get(&hex(&a_keys)).await.unwrap().phone, None, "not before Add");

    let accepted = bob.card_accept(&got.id).await.unwrap();
    assert!(accepted.card.unwrap().is_contact);
    assert!(bob.contacts().is_contact(&pk(&a_keys)).await.unwrap());
    let kept = bob.contact_private_get(&hex(&a_keys)).await.unwrap();
    assert_eq!(kept, ContactPrivateView { pubkey: hex(&a_keys), phone: Some("+15550001111".into()) });
    let told: Vec<_> = own_notes(&bob, &b_keys).await.into_iter().filter(|n| n.t == T_OWN_CARD).collect();
    assert_eq!(told.len(), 1, "my devices are told once");
    assert_eq!((told[0].str_field("pubkey"), told[0].str_field("phone")), (Some(hex(&a_keys).as_str()), Some("+15550001111")));
    bob.card_accept(&got.id).await.unwrap();
    assert_eq!(own_notes(&bob, &b_keys).await.into_iter().filter(|n| n.t == T_OWN_CARD).count(), 1, "nothing new to tell");

    // Bob passes Alice's card on to Carol, the picture from his cache, the
    // phone he keeps for her not at all.
    let thumb = messenger_avatar::square_thumb(&picture, messenger_avatar::CACHE_SIDE).unwrap();
    std::fs::create_dir_all(bob.config().avatars_dir()).unwrap();
    std::fs::write(bob.config().avatars_dir().join(format!("{sha}.jpg")), &thumb).unwrap();
    messenger_store::avatar_cache::put_fetched(bob.store(), &url, &sha, now).await.unwrap();
    let passed = bob.card_send(&hex(&c_keys), Some(&hex(&a_keys)), true).await.unwrap();
    let card = passed.card.as_ref().unwrap();
    assert_eq!((card.label.as_str(), card.phone.as_deref(), card.is_me), ("Alice", None, false));
    assert!(card.avatar.is_some());
    let row = messenger_store::messages::get(bob.store(), &passed.id).await.unwrap().unwrap();
    assert!(!row.envelope_json.contains("+1555"), "the phone never left");

    deliver(&bob, &carol).await;
    let got = last_message(&carol, &b_keys).await;
    let card = got.card.clone().unwrap();
    assert_eq!((card.pubkey.as_str(), card.phone.as_deref()), (hex(&a_keys).as_str(), None));
    assert!(card.avatar.is_some());
    carol.card_accept(&got.id).await.unwrap();
    assert!(carol.contacts().is_contact(&pk(&a_keys)).await.unwrap());
    assert_eq!(carol.contact_private_get(&hex(&a_keys)).await.unwrap().phone, None);
    assert!(own_notes(&carol, &c_keys).await.iter().all(|n| n.t != T_OWN_CARD));

    for rt in [alice, bob, carol] {
        rt.shutdown().await;
    }
}

#[tokio::test]
async fn a_card_of_somebody_unknown_is_the_key_alone_and_bad_targets_are_refused() {
    let (_a, alice, _) = started().await;
    let stranger = Keys::generate();
    let bob = Keys::generate();
    let m = alice.card_send(&hex(&bob), Some(&stranger.public_key().to_bech32().unwrap()), true).await.unwrap();
    let card = m.card.unwrap();
    assert_eq!((card.pubkey, card.phone, card.avatar, card.name), (hex(&stranger), None, None, None));
    assert!(card.label.starts_with("npub1"));
    assert_eq!(code(alice.card_send("group:nope", None, false).await), "group_unknown");
    assert!(alice.card_send(&hex(&bob), Some("not a key"), false).await.is_err());
    assert!(alice.card_send("nobody", None, false).await.is_err());
    assert_eq!(code(alice.card_accept("missing").await), "card_unknown");
    alice.shutdown().await;
}

/// A change of my phone made while no note can go is owed at once, not a
/// week after the last one that went.
#[tokio::test]
async fn a_phone_changed_without_a_session_is_told_at_the_next() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = MessengerConfig::new(dir.path().join("messenger"));
    let rt = MessengerRuntime::start(cfg, Arc::new(MemorySecretStore::unlocked())).await.unwrap();
    rt.relays().set_silent(true).await.unwrap();
    let now = SystemClock.now().secs();
    rt.dm().set_own_private(Some("+1 555 000 1111"), false).await.unwrap();
    rt.dm().own_profile_sent(now - 86_400).await.unwrap();
    assert_eq!(rt.dm().own_profile_due(now).await.unwrap(), None, "told yesterday");

    // No keys: nothing can be queued.
    rt.own_private_set(Some("+1 555 000 2222"), false).await.unwrap();
    let due = rt.dm().own_profile_due(now).await.unwrap().expect("owed now");
    assert_eq!(due.str_field("phone"), Some("+15550002222"));
    rt.shutdown().await;
}

/// My phone goes to a person or a private group, never to a public group
/// that anyone with the link reads, history and all.
#[tokio::test]
async fn my_phone_never_goes_to_a_public_group() {
    let (_a, alice, _) = started().await;
    alice.own_private_set(Some("+1 555 000 1111"), true).await.unwrap();
    let public = alice.group_create(messenger_groups::GroupKind::Public, "Open", "", true).await.unwrap();
    let private = alice.group_create(messenger_groups::GroupKind::Private, "Closed", "", true).await.unwrap();

    assert_eq!(code(alice.card_send(&format!("group:{}", public.id), None, true).await), "phone_public_group");
    let without = alice.card_send(&format!("group:{}", public.id), None, false).await.unwrap();
    assert_eq!(without.card.unwrap().phone, None);
    let with = alice.card_send(&format!("group:{}", private.id), None, true).await.unwrap();
    assert_eq!(with.card.unwrap().phone.as_deref(), Some("+15550001111"));
    // Without a phone to give, ticking the box changes nothing.
    alice.own_private_set(None, true).await.unwrap();
    assert_eq!(alice.card_send(&format!("group:{}", public.id), None, true).await.unwrap().card.unwrap().phone, None);
    alice.shutdown().await;
}

/// Accepting an older card of a person after a newer one keeps the phone
/// of the newer, here and on my other devices.
#[tokio::test]
async fn an_older_card_does_not_take_back_a_newer_phone() {
    let (_a, alice, a_keys) = started().await;
    let (_b, bob, b_keys) = started().await;
    alice.own_private_set(Some("+1 555 000 1111"), true).await.unwrap();
    let january = alice.card_send(&hex(&b_keys), None, true).await.unwrap();
    deliver(&alice, &bob).await;
    let old = last_message(&bob, &a_keys).await;
    assert_eq!(old.card.as_ref().unwrap().phone.as_deref(), Some("+15550001111"));

    // A newer card, a second on (what came before is delivered again and
    // kept once).
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    alice.own_private_set(Some("+1 555 000 2222"), true).await.unwrap();
    alice.card_send(&hex(&b_keys), None, true).await.unwrap();
    deliver(&alice, &bob).await;
    let new = last_message(&bob, &a_keys).await;
    assert_ne!(new.id, old.id);
    assert_ne!(new.id, january.id);

    bob.card_accept(&new.id).await.unwrap();
    assert_eq!(bob.contact_private_get(&hex(&a_keys)).await.unwrap().phone.as_deref(), Some("+15550002222"));
    bob.card_accept(&old.id).await.unwrap();
    assert_eq!(bob.contact_private_get(&hex(&a_keys)).await.unwrap().phone.as_deref(), Some("+15550002222"), "the newer stays");
    let told: Vec<_> = own_notes(&bob, &b_keys).await.into_iter().filter(|n| n.t == T_OWN_CARD).collect();
    assert_eq!(told.len(), 1, "nothing older is told");
    assert_eq!(told[0].str_field("phone"), Some("+15550002222"));
    for rt in [alice, bob] {
        rt.shutdown().await;
    }
}
