// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! «Load earlier» in a direct chat against local relays: a device that
//! came back (its catch-up only reached a recent window) reads the older
//! wraps of the inbox page by page, the other chats' with them, up to the
//! start of the inbox; a private chat reads its own server's.
//!
//! The wraps are my own copies of what I wrote (a rumor of mine sealed and
//! wrapped to me), put on the relays with the times of weeks ago: what a
//! fresh device gets of my past in every chat, and nothing of the gates of
//! strangers comes into it.

use super::*;
use crate::private::PrivateServerInput;
use messenger_core::envelope::{Envelope, TAG_SERVER};
use messenger_core::outbound::WireEvent;
use messenger_core::traits::RelayState;
use messenger_core::{MessengerConfig, Outbound, PubKey, RelayUrl, Transport};
use messenger_store::{cursors, servers};
use messenger_testkit::MemorySecretStore;
use messenger_transport::{RelayConfig, RelayPool};
use nostr::nips::nip19::ToBech32;
use nostr::nips::nip44;
use nostr::nips::nip59::GiftWrapSealBuilder;
use nostr::prelude::*;
use nostr_sdk::local_relay::LocalRelay;

const DAY: i64 = 24 * 3600;

async fn connected(pool: &RelayPool) {
    for _ in 0..50 {
        if pool.status().await.relays.iter().all(|r| r.state == RelayState::Connected) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the local relays were not reached");
}

async fn local_relay() -> (LocalRelay, String) {
    let relay = LocalRelay::builder().build();
    relay.run().await.unwrap();
    let url = relay.url().await.as_str_without_trailing_slash().to_string();
    (relay, url)
}

/// A text of mine to `to` (on `server`, of a private chat) written at
/// `at`, as the copy for my own devices, its wrap made at `wrapped`.
fn my_copy(me: &Keys, to: &PubKey, server: Option<&str>, text: &str, at: i64, wrapped: i64) -> WireEvent {
    let mut builder = EventBuilder::new(Kind::PrivateDirectMessage, Envelope::text(text).encode())
        .tag(Tag::public_key(PublicKey::from_hex(to.as_hex()).unwrap()))
        .custom_created_at(nostr::types::Timestamp::from_secs(at as u64));
    if let Some(id) = server {
        builder = builder.tag(Tag::parse([TAG_SERVER, id]).unwrap());
    }
    let rumor = builder.finalize_unsigned(me.public_key());
    let seal = GiftWrapSealBuilder::new(rumor, me.public_key()).finalize(me).unwrap();
    let one_time = Keys::generate();
    let content = nip44::encrypt(one_time.secret_key(), &me.public_key(), seal.as_json(), nip44::Version::default()).unwrap();
    let wrap = EventBuilder::new(Kind::GiftWrap, content)
        .tag(Tag::public_key(me.public_key()))
        .custom_created_at(nostr::types::Timestamp::from_secs(wrapped as u64))
        .finalize(&one_time)
        .unwrap();
    WireEvent { id: messenger_core::EventId::parse(&wrap.id.to_hex()).unwrap(), json: serde_json::to_value(&wrap).unwrap() }
}

/// Puts `wraps` on the relay `url`.
async fn put(url: &str, wraps: &[WireEvent]) {
    let pool = RelayPool::new(None);
    pool.set_relays(vec![RelayConfig { url: RelayUrl::parse(url).unwrap(), read: true, write: true, api_key: None, server_id: None }])
        .await
        .unwrap();
    connected(&pool).await;
    for w in wraps {
        assert!(pool.send(Outbound::PublishOwn { event: w.clone() }).await.unwrap().is_delivered());
    }
    pool.shutdown().await;
}

/// The device of `me` that came back: the home relay and the private
/// server, and a catch-up that reaches back to the last days only, as the
/// first one of a device reaches its newest `DM_HISTORY_LIMIT` wraps only.
async fn device_back(me: &Keys, home: &str, private: &str) -> (tempfile::TempDir, MessengerRuntime, String) {
    let dir = tempfile::tempdir().unwrap();
    let rt = MessengerRuntime::start(MessengerConfig::new(dir.path().join("messenger")), Arc::new(MemorySecretStore::unlocked()))
        .await
        .unwrap();
    rt.relays().add_user(home, None).await.unwrap();
    rt.servers_use_own().await.unwrap();
    let s1 = rt
        .server_put_private(&PrivateServerInput { relay_url: private.into(), name: Some("S1".into()), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(s1, servers::id_of(private));
    let now = now_secs();
    for server in [None, Some(s1.as_str())] {
        cursors::advance(rt.store(), filters::CURSOR_DM_INBOX, cursors::of_server(server), now).await.unwrap();
    }
    rt.identity().import_nsec(&me.secret_key().to_bech32().unwrap()).await.unwrap();
    assert!(rt.refresh_signer().await.unwrap());
    connected(&*rt.relays().pool().await).await;
    // Pages of three: each asks its oldest second again, so two are new.
    rt.dm_history_state.set_page(3);
    (dir, rt, s1)
}

async fn texts(rt: &MessengerRuntime, chat: &str) -> Vec<String> {
    rt.dm()
        .messages(chat, None, 500)
        .await
        .unwrap()
        .into_iter()
        .filter(|m| m.content_type == "text")
        .map(|m| m.text.unwrap_or_default())
        .collect()
}

async fn texts_when(rt: &MessengerRuntime, chat: &str, want: usize) -> Vec<String> {
    let mut got = Vec::new();
    for _ in 0..100 {
        got = texts(rt, chat).await;
        if got.len() >= want {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    got
}

/// The time of the oldest message of `chat` (or `fallback` for none).
async fn oldest(rt: &MessengerRuntime, chat: &str, fallback: i64) -> i64 {
    rt.dm().messages(chat, None, 500).await.unwrap().first().map(|m| m.created_at).unwrap_or(fallback)
}

#[tokio::test]
async fn the_past_of_a_direct_chat_comes_page_by_page_to_the_start_of_the_inbox() {
    let (_home, home_url) = local_relay().await;
    let (_private, private_url) = local_relay().await;
    let (bob, alice, carol) = (Keys::generate(), Keys::generate(), Keys::generate());
    let pk = |k: &Keys| PubKey::parse(&k.public_key().to_hex()).unwrap();
    let (alice_pk, carol_pk) = (pk(&alice), pk(&carol));
    let s1 = servers::id_of(&private_url);
    let now = now_secs();

    // The home inbox, newest first: one to Alice of an hour ago (in the
    // window of the catch-up), then three to Carol, then seven to Alice,
    // all weeks old. The private server holds three to Alice there.
    let mut home = vec![my_copy(&bob, &alice_pk, None, "a recent", now - 3_500, now - 3_600)];
    for i in 1..=3 {
        let at = now - (10 + i) * DAY;
        home.push(my_copy(&bob, &carol_pk, None, &format!("c{i}"), at, at - 100));
    }
    for i in 1..=7 {
        let at = now - (20 + i) * DAY;
        home.push(my_copy(&bob, &alice_pk, None, &format!("a{i}"), at, at - 100));
    }
    put(&home_url, &home).await;
    let on_s1: Vec<WireEvent> = (1..=3)
        .map(|i| {
            let at = now - (30 + i) * DAY;
            my_copy(&bob, &alice_pk, Some(&s1), &format!("s{i}"), at, at - 100)
        })
        .collect();
    put(&private_url, &on_s1).await;

    let (_dir, rt, _) = device_back(&bob, &home_url, &private_url).await;
    let (alice_chat, carol_chat) = (chats::dm_chat_id(alice_pk.as_hex(), None), chats::dm_chat_id(carol_pk.as_hex(), None));
    let private_chat = chats::dm_chat_id(alice_pk.as_hex(), Some(&s1));
    assert_eq!(texts_when(&rt, &alice_chat, 1).await, vec!["a recent"], "the catch-up brought the window");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(texts(&rt, &alice_chat).await, vec!["a recent"], "and nothing older");
    assert!(texts(&rt, &carol_chat).await.is_empty());

    // The first ask: Carol's three are newer than any older one of Alice's,
    // so they come on the way — her chat filled — and it stops at Alice's.
    let first = rt.dm_history(&alice_chat, oldest(&rt, &alice_chat, now).await).await.unwrap();
    assert!(first.older >= 1 && !first.end, "{first:?}");
    let shown = texts(&rt, &alice_chat).await;
    assert_eq!((shown.len() as u32, shown.last().map(String::as_str)), (first.older + 1, Some("a recent")), "{shown:?}");
    assert!(shown.contains(&"a1".to_string()) && !shown.contains(&"a7".to_string()), "{shown:?}");
    assert_eq!(texts(&rt, &carol_chat).await, vec!["c3", "c2", "c1"], "the wraps of another chat came too");

    // Asked again from what is shown, until the start of the inbox.
    let mut asks = vec![first];
    loop {
        let page = rt.dm_history(&alice_chat, oldest(&rt, &alice_chat, now).await).await.unwrap();
        asks.push(page.clone());
        if page.end || asks.len() > 10 {
            break;
        }
        assert!(page.older > 0, "each ask brings something until the end: {asks:?}");
    }
    assert_eq!(
        texts(&rt, &alice_chat).await,
        vec!["a7", "a6", "a5", "a4", "a3", "a2", "a1", "a recent"],
        "the whole past, in order: {asks:?}"
    );
    assert!(asks.len() >= 3, "page by page: {asks:?}");
    assert_eq!(asks.last(), Some(&DmHistoryPage { older: 0, end: true }));
    // The end is kept: asked again, it answers at once.
    let at_once = tokio::time::Instant::now();
    assert_eq!(rt.dm_history(&carol_chat, oldest(&rt, &carol_chat, now).await).await.unwrap(), DmHistoryPage { older: 0, end: true });
    assert!(at_once.elapsed() < Duration::from_secs(1));
    assert!(texts(&rt, &private_chat).await.is_empty(), "nothing of the private server came by the home set");

    // The private chat, empty here, reads its own server's inbox.
    let mut asks = Vec::new();
    loop {
        let page = rt.dm_history(&private_chat, oldest(&rt, &private_chat, now).await).await.unwrap();
        asks.push(page.clone());
        if page.end || asks.len() > 10 {
            break;
        }
    }
    assert_eq!(texts(&rt, &private_chat).await, vec!["s3", "s2", "s1"], "{asks:?}");
    assert!(asks[0].older >= 1 && !asks[0].end, "a chat with nothing shown asks before now: {asks:?}");
    assert_eq!(asks.last(), Some(&DmHistoryPage { older: 0, end: true }));
    assert_eq!(texts(&rt, &alice_chat).await.len(), 8, "the home chat is as it was");

    // A server this device does not have is refused.
    let unknown = format!("dm:{}@{}", alice_pk.as_hex(), "f".repeat(16));
    assert!(rt.dm_history(&unknown, now).await.is_err());
    rt.shutdown().await;
}

/// Two asks at once read the pages once: the second waits for the first
/// and finds what it brought.
#[tokio::test]
async fn two_asks_at_once_read_the_pages_once() {
    let (_home, home_url) = local_relay().await;
    let (_private, private_url) = local_relay().await;
    let (bob, alice) = (Keys::generate(), Keys::generate());
    let alice_pk = PubKey::parse(&alice.public_key().to_hex()).unwrap();
    let now = now_secs();
    let old: Vec<WireEvent> = (1..=3)
        .map(|i| {
            let at = now - (20 + i) * DAY;
            my_copy(&bob, &alice_pk, None, &format!("a{i}"), at, at - 100)
        })
        .collect();
    put(&home_url, &old).await;
    let (_dir, rt, _) = device_back(&bob, &home_url, &private_url).await;
    let chat = chats::dm_chat_id(alice_pk.as_hex(), None);
    let (one, two) = tokio::join!(rt.dm_history(&chat, now), rt.dm_history(&chat, now));
    let (one, two) = (one.unwrap(), two.unwrap());
    assert!(one.older >= 1 && !one.end, "{one:?}");
    assert_eq!(two, one, "the second found what the first brought, and asked nothing");
    assert_eq!(texts(&rt, &chat).await.len() as u32, one.older);
    rt.shutdown().await;
}

/// A wrap far older than the window, fetched by its id (as the welcome to
/// myself of a group is, `relations::welcomes_wanted`), says nothing of
/// what lies between it and the window: the first ask starts at the top
/// and the gap comes.
#[tokio::test]
async fn an_old_wrap_held_alone_does_not_hide_the_gap_above_it() {
    let (_home, home_url) = local_relay().await;
    let (_private, private_url) = local_relay().await;
    let (bob, alice, carol) = (Keys::generate(), Keys::generate(), Keys::generate());
    let pk = |k: &Keys| PubKey::parse(&k.public_key().to_hex()).unwrap();
    let (alice_pk, carol_pk) = (pk(&alice), pk(&carol));
    let now = now_secs();
    let mut home: Vec<WireEvent> = (1..=3)
        .map(|i| {
            let at = now - (20 + i) * DAY;
            my_copy(&bob, &alice_pk, None, &format!("a{i}"), at, at - 100)
        })
        .collect();
    let lone = my_copy(&bob, &carol_pk, None, "long ago", now - 100 * DAY, now - 100 * DAY - 100);
    home.push(lone.clone());
    put(&home_url, &home).await;

    let (_dir, rt, _) = device_back(&bob, &home_url, &private_url).await;
    let pool = rt.relays().pool().await;
    let filter = messenger_core::outbound::Filter(serde_json::json!({ "kinds": [1059], "ids": [lone.id.as_hex()] }));
    pool.send(Outbound::Sync { scope: messenger_core::Scope::Own, filter, local: vec![] }).await.unwrap();
    let carol_chat = chats::dm_chat_id(carol_pk.as_hex(), None);
    assert_eq!(texts_when(&rt, &carol_chat, 1).await, vec!["long ago"], "held alone");

    let alice_chat = chats::dm_chat_id(alice_pk.as_hex(), None);
    let mut asks = Vec::new();
    loop {
        let page = rt.dm_history(&alice_chat, oldest(&rt, &alice_chat, now).await).await.unwrap();
        asks.push(page.clone());
        if page.end || asks.len() > 10 {
            break;
        }
    }
    assert_eq!(texts(&rt, &alice_chat).await, vec!["a3", "a2", "a1"], "the gap came: {asks:?}");
    rt.shutdown().await;
}
