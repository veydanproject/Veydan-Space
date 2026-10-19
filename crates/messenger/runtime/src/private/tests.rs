// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A private chat end to end: two runtimes on one home relay, both with the
//! same private server on a second relay, talk in the chat of the server,
//! and the home relay hears nothing of it.

use super::*;
use messenger_core::traits::RelayState;
use messenger_core::{MessengerConfig, PubKey, Transport};
use messenger_dm::Action;
use messenger_testkit::MemorySecretStore;
use messenger_transport::{RelayConfig, RelayPool};
use nostr_sdk::local_relay::LocalRelay;
use std::sync::Arc;
use std::time::Duration;

async fn connected(pool: &RelayPool) {
    for _ in 0..50 {
        if pool
            .status()
            .await
            .relays
            .iter()
            .all(|r| r.state == RelayState::Connected)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the local relays were not reached");
}

/// What one relay holds that matches `filter`, read by a pool of its own.
async fn on_relay(url: &str, filter: serde_json::Value) -> Vec<serde_json::Value> {
    let pool = RelayPool::new(None);
    let url = RelayUrl::parse(url).unwrap();
    pool.set_relays(vec![RelayConfig {
        url,
        read: true,
        write: true,
        api_key: None,
        server_id: None,
    }])
    .await
    .unwrap();
    connected(&pool).await;
    let found = pool
        .fetch(
            &messenger_core::outbound::Filter(filter),
            &messenger_core::Scope::Own,
            Duration::from_secs(3),
        )
        .await
        .unwrap();
    pool.shutdown().await;
    found
}

/// The gift wraps a relay holds for `who`.
async fn wraps_for(url: &str, who: &PubKey) -> usize {
    on_relay(
        url,
        serde_json::json!({ "kinds": [1059], "#p": [who.as_hex()] }),
    )
    .await
    .len()
}

/// A runtime on the home relay with the private server, logged in.
async fn device(
    home: &str,
    private: &str,
) -> (tempfile::TempDir, MessengerRuntime, PubKey, String) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = MessengerConfig::new(dir.path().join("messenger"));
    let rt = MessengerRuntime::start(cfg, Arc::new(MemorySecretStore::unlocked()))
        .await
        .unwrap();
    rt.relays().add_user(home, None).await.unwrap();
    rt.servers_use_own().await.unwrap();
    let id = rt
        .server_put_private(&PrivateServerInput {
            relay_url: private.into(),
            name: Some("S1".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(id, servers::id_of(private));
    rt.identity().create("pw").await.unwrap();
    assert!(rt.refresh_signer().await.unwrap());
    connected(&*rt.relays().pool().await).await;
    let me = rt.session_pubkey().await.unwrap();
    (dir, rt, me, id)
}

/// The texts of a chat once `want` of them are there (or what is, after 8 s).
async fn texts_when(rt: &MessengerRuntime, chat: &str, want: usize) -> Vec<String> {
    let mut texts = Vec::new();
    for _ in 0..80 {
        texts = rt
            .dm()
            .messages(chat, None, 100)
            .await
            .unwrap()
            .into_iter()
            .filter(|m| m.content_type == "text")
            .map(|m| m.text.unwrap_or_default())
            .collect();
        if texts.len() >= want {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    texts
}

async fn mode_when(rt: &MessengerRuntime, peer: &str, want: &str) -> String {
    let mut mode = String::new();
    for _ in 0..80 {
        mode = rt.dm_relation(peer).await.unwrap().mode;
        if mode == want {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    mode
}

#[tokio::test]
async fn a_private_chat_never_reaches_the_home_relay() {
    let home = LocalRelay::builder().build();
    home.run().await.unwrap();
    let home_url = home.url().await.as_str_without_trailing_slash().to_string();
    let private = LocalRelay::builder().build();
    private.run().await.unwrap();
    let private_url = private
        .url()
        .await
        .as_str_without_trailing_slash()
        .to_string();

    let (_a, alice, alice_pk, s1) = device(&home_url, &private_url).await;
    let (_b, bob, bob_pk, _) = device(&home_url, &private_url).await;
    let (alice_on_s1, bob_on_s1) = (
        format!("{}@{s1}", alice_pk.as_hex()),
        format!("{}@{s1}", bob_pk.as_hex()),
    );
    assert!(
        alice
            .dm_send_text(
                &format!("{}@{}", bob_pk.as_hex(), "f".repeat(16)),
                "nowhere",
                None
            )
            .await
            .is_err(),
        "a server I do not have"
    );

    // A first text on the server: a request in the private chat there.
    let sent = alice
        .dm_send_text(&bob_on_s1, "hello on s1", None)
        .await
        .unwrap();
    assert_eq!(sent.chat_id, format!("dm:{bob_on_s1}"));
    let bobs_chat = format!("dm:{alice_on_s1}");
    assert_eq!(texts_when(&bob, &bobs_chat, 1).await, vec!["hello on s1"]);
    assert_eq!(
        mode_when(&bob, &alice_on_s1, "request_received").await,
        "request_received"
    );
    assert!(
        bob.dm()
            .chat(&format!("dm:{}", alice_pk.as_hex()))
            .await
            .unwrap()
            .is_none(),
        "no home chat came of it"
    );
    assert!(
        !bob.contacts().is_contact(&alice_pk).await.unwrap(),
        "nor a contact of the book"
    );

    // Accepted there, answered, read: all of it on the server.
    bob.dm_act(&alice_on_s1, Action::Accept).await.unwrap();
    assert_eq!(
        mode_when(&alice, &bob_on_s1, "full_chat").await,
        "full_chat"
    );
    bob.dm_send_text(&alice_on_s1, "back on s1", None)
        .await
        .unwrap();
    assert_eq!(
        texts_when(&alice, &format!("dm:{bob_on_s1}"), 2).await,
        vec!["hello on s1", "back on s1"]
    );
    alice
        .chat_mark_read(&format!("dm:{bob_on_s1}"))
        .await
        .unwrap();
    bob.flush_receipts().await.unwrap();
    // The read mark of Alice comes back to Bob's message.
    let mut read = None;
    for _ in 0..80 {
        let mine = bob.dm().messages(&bobs_chat, None, 100).await.unwrap();
        read = mine
            .iter()
            .find(|m| m.text.as_deref() == Some("back on s1"))
            .and_then(|m| m.read_at);
        if read.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(read.is_some(), "the read receipt went by the server");

    // A reaction, an edit and a deletion go the same way.
    bob.dm_react(&sent.id, "👍").await.unwrap();
    alice
        .dm_edit(&sent.id, "hello on s1, edited")
        .await
        .unwrap();
    let extra = alice
        .dm_send_text(&bob_on_s1, "to be taken back", None)
        .await
        .unwrap();
    assert_eq!(texts_when(&bob, &bobs_chat, 3).await.len(), 3);
    alice.dm_delete(&extra.id, true).await.unwrap();
    let mut seen = (false, false, false);
    for _ in 0..80 {
        let reacted = alice
            .dm()
            .message(&sent.id)
            .await
            .unwrap()
            .is_some_and(|m| m.reactions.iter().any(|r| r.emoji == "👍"));
        let edited = bob
            .dm()
            .message(&sent.id)
            .await
            .unwrap()
            .is_some_and(|m| m.text.as_deref() == Some("hello on s1, edited"));
        let gone = bob
            .dm()
            .message(&extra.id)
            .await
            .unwrap()
            .is_some_and(|m| m.deleted);
        seen = (reacted, edited, gone);
        if reacted && edited && gone {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        seen,
        (true, true, true),
        "the reaction, the edit and the deletion came by the server"
    );

    // The server holds the wraps of the chat, both ways and my copies;
    // the home relay not one.
    assert!(wraps_for(&private_url, &bob_pk).await >= 2);
    assert!(wraps_for(&private_url, &alice_pk).await >= 2);
    assert_eq!(
        wraps_for(&home_url, &bob_pk).await,
        0,
        "nothing of the private chat at home"
    );
    assert_eq!(wraps_for(&home_url, &alice_pk).await, 0);

    // My kind 0 goes to the server too, of the same second as the home one.
    let profile = messenger_contacts::ProfileInput {
        name: Some("Alice".into()),
        ..Default::default()
    };
    alice.publish_own_profile(&profile).await.unwrap();
    let mine = serde_json::json!({ "kinds": [0], "authors": [alice_pk.as_hex()] });
    let (mut on_s1, mut at_home) = (Vec::new(), Vec::new());
    for _ in 0..50 {
        (on_s1, at_home) = (
            on_relay(&private_url, mine.clone()).await,
            on_relay(&home_url, mine.clone()).await,
        );
        if !on_s1.is_empty() && !at_home.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(on_s1.len(), 1, "my profile on the private server");
    let content: serde_json::Value =
        serde_json::from_str(on_s1[0]["content"].as_str().unwrap()).unwrap();
    assert_eq!(content["name"], "Alice");
    assert!(
        content.get("picture").is_none(),
        "no picture of the home set"
    );
    assert_eq!(
        on_s1[0]["created_at"], at_home[0]["created_at"],
        "never newer than the home one"
    );

    // The home chat with the same person is another chat, by the home relay.
    let at_home = alice
        .dm_send_text(bob_pk.as_hex(), "at home", None)
        .await
        .unwrap();
    assert_eq!(at_home.chat_id, format!("dm:{}", bob_pk.as_hex()));
    assert_eq!(
        texts_when(&bob, &format!("dm:{}", alice_pk.as_hex()), 1).await,
        vec!["at home"]
    );
    assert_eq!(
        mode_when(&bob, alice_pk.as_hex(), "request_received").await,
        "request_received",
        "its own relation"
    );
    assert_eq!(
        bob.dm_relation(&alice_on_s1).await.unwrap().mode,
        "full_chat"
    );
    assert!(wraps_for(&home_url, &bob_pk).await >= 1);
    assert_eq!(
        texts_when(&bob, &bobs_chat, 3).await,
        vec!["hello on s1, edited", "back on s1", ""],
        "the private chat is as it was, its deletion too"
    );
    alice.shutdown().await;
    bob.shutdown().await;
}

/// A runtime on the home relay alone, with a new identity or the one of
/// `nsec` (another device of the same person), logged in.
async fn plain_device(
    home: &str,
    nsec: Option<&str>,
) -> (tempfile::TempDir, MessengerRuntime, PubKey) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = MessengerConfig::new(dir.path().join("messenger"));
    let rt = MessengerRuntime::start(cfg, Arc::new(MemorySecretStore::unlocked()))
        .await
        .unwrap();
    rt.relays().add_user(home, None).await.unwrap();
    rt.servers_use_own().await.unwrap();
    match nsec {
        Some(nsec) => {
            rt.identity().import_nsec(nsec).await.unwrap();
        }
        None => {
            rt.identity().create("pw").await.unwrap();
        }
    }
    rt.refresh_signer().await.unwrap();
    connected(&*rt.relays().pool().await).await;
    let me = rt.session_pubkey().await.unwrap();
    (dir, rt, me)
}

/// A runtime whose home set is the one relay `home`, with a new identity.
async fn home_device(home: &str) -> (tempfile::TempDir, MessengerRuntime, PubKey) {
    plain_device(home, None).await
}

/// The private servers of `rt` once they are `want` (or what they are,
/// after 10 s).
async fn servers_when(rt: &MessengerRuntime, want: &[&str]) -> Vec<String> {
    let mut ids = Vec::new();
    for _ in 0..100 {
        ids = rt
            .servers_private()
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect::<Vec<_>>();
        if ids == want {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    ids
}

/// Waits until `check` (an expression that may await) holds, or panics
/// after 8 s with `what`.
macro_rules! until {
    ($what:expr, $check:expr) => {{
        let mut held = false;
        for _ in 0..80 {
            if $check {
                held = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(held, "{}", $what);
    }};
}

/// The texts of a chat of a group.
async fn group_texts(rt: &MessengerRuntime, chat: &str) -> Vec<String> {
    rt.dm()
        .messages(chat, None, 100)
        .await
        .unwrap()
        .into_iter()
        .filter(|m| m.content_type == "text")
        .filter_map(|m| m.text)
        .collect()
}

/// Stage Э5: a server one device of mine puts in reaches the other by the
/// snapshot of my relations, and both hear the server without a restart —
/// the first by hand, the second from the snapshot, with the history the
/// server holds and what comes live; the relation of the private chat
/// travels with it. A removal on the second takes the server, its relay and
/// its chat on the first.
#[tokio::test]
async fn a_private_server_reaches_my_other_device_and_so_does_its_removal() {
    let home = LocalRelay::builder().build();
    home.run().await.unwrap();
    let home_url = home.url().await.as_str_without_trailing_slash().to_string();
    let private = LocalRelay::builder().build();
    private.run().await.unwrap();
    let private_url = private
        .url()
        .await
        .as_str_without_trailing_slash()
        .to_string();

    let (_b, bob, bob_pk, s1) = device(&home_url, &private_url).await;
    let (_a1, alice1, alice_pk) = plain_device(&home_url, None).await;
    let nsec = alice1
        .identity()
        .load_keys()
        .await
        .unwrap()
        .secret_key()
        .to_secret_hex();
    let (_a2, alice2, same) = plain_device(&home_url, Some(&nsec)).await;
    assert_eq!(same, alice_pk, "two devices of one person");
    let (alice_on_s1, bob_on_s1) = (
        format!("{}@{s1}", alice_pk.as_hex()),
        format!("{}@{s1}", bob_pk.as_hex()),
    );
    let chat = format!("dm:{bob_on_s1}");

    // Put in on the first device while its session runs: heard at once.
    let id = alice1
        .server_put_private(&PrivateServerInput {
            relay_url: private_url.clone(),
            name: Some("S1".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(id, s1);
    bob.dm_send_text(&alice_on_s1, "first", None).await.unwrap();
    assert_eq!(texts_when(&alice1, &chat, 1).await, vec!["first"]);

    // The second device takes it from the snapshot, listens to it without
    // a restart and catches up what the server holds.
    assert_eq!(servers_when(&alice2, &[&s1]).await, vec![s1.clone()]);
    let taken = servers::get(alice2.store(), &s1).await.unwrap().unwrap();
    assert_eq!(
        (taken.name.as_deref(), taken.source.as_deref()),
        (Some("S1"), Some(servers::SOURCE_SNAPSHOT))
    );
    assert_eq!(texts_when(&alice2, &chat, 1).await, vec!["first"]);

    // Accepted on the first device: the second gates the chat alike, and
    // hears what comes live on the server.
    alice1.dm_act(&bob_on_s1, Action::Accept).await.unwrap();
    assert_eq!(
        mode_when(&alice2, &bob_on_s1, "full_chat").await,
        "full_chat"
    );
    assert_eq!(
        mode_when(&bob, &alice_on_s1, "full_chat").await,
        "full_chat"
    );
    bob.dm_send_text(&alice_on_s1, "second", None)
        .await
        .unwrap();
    assert_eq!(texts_when(&alice2, &chat, 2).await, vec!["first", "second"]);
    assert_eq!(
        wraps_for(&home_url, &alice_pk).await,
        0,
        "nothing of the private chat at home"
    );

    // Removed on the second device: gone on both, its relay and its chat.
    assert!(alice2.server_remove_private(&"f".repeat(16)).await.is_err());
    alice2.server_remove_private(&s1).await.unwrap();
    assert!(alice2.dm().chat(&chat).await.unwrap().is_none());
    assert_eq!(servers_when(&alice1, &[]).await, Vec::<String>::new());
    let mut gone = false;
    for _ in 0..50 {
        gone = alice1.dm().chat(&chat).await.unwrap().is_none()
            && alice1
                .relays()
                .pool()
                .await
                .configured()
                .iter()
                .all(|r| r.server_id.is_none());
        if gone {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(gone, "the chat and the relay of the server went with it");
    alice1.shutdown().await;
    alice2.shutdown().await;
    bob.shutdown().await;
}

/// A group of a private server, end to end (internal/messenger-wire.md,
/// «Группы на сервере»): A has the home relay H alone, B has H at home and
/// S1 as a private server, C has S1 as its home. B makes a group on S1, C
/// asks in by its link and is let in, A is told the server is unknown; the
/// messages of the group go both ways by S1, a file of the group goes to
/// S1's blob server, and H holds not one event of the group, not one wrap
/// of its protocol.
#[tokio::test]
async fn a_group_of_a_private_server_never_reaches_the_home_relay() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let home = LocalRelay::builder().build();
    home.run().await.unwrap();
    let home_url = home.url().await.as_str_without_trailing_slash().to_string();
    let private = LocalRelay::builder().build();
    private.run().await.unwrap();
    let private_url = private
        .url()
        .await
        .as_str_without_trailing_slash()
        .to_string();
    let blobs = MockServer::start().await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&blobs)
        .await;

    let (_a, a, _a_pk) = home_device(&home_url).await;
    let (_b, b, b_pk, s1) = device(&home_url, &private_url).await;
    b.server_put_private(&PrivateServerInput {
        relay_url: private_url.clone(),
        name: Some("S1".into()),
        media_url: Some(blobs.uri()),
        ..Default::default()
    })
    .await
    .unwrap();
    let (_c, c, c_pk) = home_device(&private_url).await;

    // B makes the group on S1; it is listed with its server.
    assert!(
        b.group_create(crate::GroupKind::Private, "Nowhere", "", true, Some(&"f".repeat(16)))
            .await
            .is_err(),
        "a server I do not have"
    );
    let g = b
        .group_create(crate::GroupKind::Private, "On S1", "", true, Some(&s1))
        .await
        .unwrap();
    assert_eq!(g.server_id.as_deref(), Some(s1.as_str()));
    let chat = format!("group:{}", g.id);
    assert_eq!(
        b.chat(&chat).await.unwrap().unwrap().server_id.as_deref(),
        Some(s1.as_str()),
        "the chat names the server, its id stays group:<id>"
    );
    let link = g.link.clone().expect("the owner may share the link");

    // A has no S1: the link leads nowhere, and nothing is written for it.
    let refused = a.group_open_link(&link, "").await.unwrap_err();
    assert!(refused.to_string().contains("server_unknown"), "{refused}");
    assert!(a.group_list().await.unwrap().is_empty());

    // C, at home on S1, asks; B lets C in.
    c.group_open_link(&link, "let me in").await.unwrap();
    until!("B hears the request", !b.group_get(&g.id).await.unwrap().requests.is_empty());
    b.group_answer_request(&g.id, c_pk.as_hex(), true)
        .await
        .unwrap();
    until!("C is in", c.group_get(&g.id).await.unwrap().membership == "joined");
    assert_eq!(
        c.group_get(&g.id).await.unwrap().server_id,
        None,
        "S1 is C's home"
    );

    // The messages of the group, both ways.
    b.group_send_text(&g.id, "hello on s1", None).await.unwrap();
    until!("C reads B", group_texts(&c, &chat).await.contains(&"hello on s1".to_string()));
    c.group_send_text(&g.id, "back", None).await.unwrap();
    until!("B reads C", group_texts(&b, &chat).await.contains(&"back".to_string()));
    b.chat_mark_read(&chat).await.unwrap();

    // A file of the group goes to S1's blob server.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes.txt");
    tokio::fs::write(&file, b"of the group").await.unwrap();
    b.dm_send_file(&chat, &file, None, None, true).await.unwrap();
    until!("the file is on S1's blob server", !blobs.received_requests().await.unwrap_or_default().is_empty());

    // S1 holds the group; H not one event of it, not one wrap for B or C.
    let of_group = serde_json::json!({ "kinds": [9], "#h": [g.id] });
    assert!(!on_relay(&private_url, of_group.clone()).await.is_empty());
    assert!(
        on_relay(&home_url, serde_json::json!({ "kinds": [9] }))
            .await
            .is_empty(),
        "no event of the group at home"
    );
    assert_eq!(
        wraps_for(&home_url, &b_pk).await,
        0,
        "nothing of the group's protocol at home"
    );
    assert_eq!(wraps_for(&home_url, &c_pk).await, 0);
    assert!(wraps_for(&private_url, &c_pk).await >= 1, "C's welcome on S1");
    a.shutdown().await;
    b.shutdown().await;
    c.shutdown().await;
}

/// The card a message of `chat` carries once one is there (or `None`
/// after 8 s): the server it names, as the card view shows it.
async fn server_of_card_when(rt: &MessengerRuntime, chat: &str) -> Option<String> {
    for _ in 0..80 {
        let found = rt
            .dm()
            .messages(chat, None, 100)
            .await
            .unwrap()
            .into_iter()
            .find_map(|m| m.card.and_then(|c| c.server));
        if found.is_some() {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    None
}

/// Stage Э4, the owner's second situation (internal/messenger-wire.md,
/// «Принятие сервера»): A (home H alone) and B (home H, private server S1)
/// talk at home; B sends A the card of S1 in that chat; A sees what S1 is,
/// accepts it and opens the private chat with B there, and both sides are
/// in a full chat on S1 at once — B approved A by giving the card, A by
/// opening the chat — without a request to answer. What they write there
/// goes by S1.
#[tokio::test]
async fn a_server_card_in_a_home_chat_opens_a_private_chat_on_both_sides() {
    let home = LocalRelay::builder().build();
    home.run().await.unwrap();
    let home_url = home.url().await.as_str_without_trailing_slash().to_string();
    let private = LocalRelay::builder().build();
    private.run().await.unwrap();
    let private_url = private
        .url()
        .await
        .as_str_without_trailing_slash()
        .to_string();

    let (_a, alice, alice_pk) = home_device(&home_url).await;
    let (_b, bob, bob_pk, s1) = device(&home_url, &private_url).await;

    // A full chat at home first.
    alice
        .dm_send_text(bob_pk.as_hex(), "hi at home", None)
        .await
        .unwrap();
    assert_eq!(
        mode_when(&bob, alice_pk.as_hex(), "request_received").await,
        "request_received"
    );
    bob.dm_act(alice_pk.as_hex(), Action::Accept).await.unwrap();
    assert_eq!(
        mode_when(&alice, bob_pk.as_hex(), "full_chat").await,
        "full_chat"
    );

    // B sends A the card of S1, in the home chat: only B's own card takes
    // one, and only of a server B has.
    assert!(bob
        .card_send_with_server(alice_pk.as_hex(), Some(alice_pk.as_hex()), false, Some(&s1))
        .await
        .is_err());
    assert!(bob
        .card_send_with_server(alice_pk.as_hex(), None, false, Some(&"f".repeat(16)))
        .await
        .is_err());
    let sent = bob
        .card_send_with_server(alice_pk.as_hex(), None, false, Some(&s1))
        .await
        .unwrap();
    assert!(sent.card.as_ref().and_then(|c| c.server.as_ref()).is_some());
    let alice_on_s1 = format!("{}@{s1}", alice_pk.as_hex());
    let bob_on_s1 = format!("{}@{s1}", bob_pk.as_hex());
    let approved = bob.dm_relation(&alice_on_s1).await.unwrap();
    assert_eq!(
        (approved.my_contact.as_str(), approved.peer_signal.as_str()),
        ("approved", "none"),
        "giving the card approved A on S1, silently"
    );

    // A gets it, sees what S1 is and accepts it.
    let card = server_of_card_when(&alice, &format!("dm:{}", bob_pk.as_hex()))
        .await
        .expect("the card with the server came");
    let preview = alice.server_preview(&card).await.unwrap();
    assert_eq!(preview.id, s1);
    assert_eq!(preview.name, "S1");
    assert!(!preview.known && !preview.is_veydan);
    assert_eq!(alice.server_accept(&card).await.unwrap(), s1);
    assert_eq!(servers_when(&alice, &[&s1]).await, vec![s1.clone()]);

    // The private chat, open on both sides at once.
    let chat = alice
        .contact_open_private(bob_pk.as_hex(), &s1, None)
        .await
        .unwrap();
    assert_eq!(chat.id, format!("dm:{bob_on_s1}"));
    assert_eq!(
        mode_when(&bob, &alice_on_s1, "full_chat").await,
        "full_chat",
        "B's side is open"
    );
    assert_eq!(
        mode_when(&alice, &bob_on_s1, "full_chat").await,
        "full_chat",
        "and A's, by B's answer"
    );
    alice
        .dm_send_text(&bob_on_s1, "on s1 now", None)
        .await
        .unwrap();
    assert_eq!(
        texts_when(&bob, &format!("dm:{alice_on_s1}"), 1).await,
        vec!["on s1 now"]
    );
    bob.dm_send_text(&alice_on_s1, "here too", None)
        .await
        .unwrap();
    assert_eq!(
        texts_when(&alice, &chat.id, 2).await,
        vec!["on s1 now", "here too"]
    );
    // Opened again: only opened.
    alice
        .contact_open_private(bob_pk.as_hex(), &s1, None)
        .await
        .unwrap();
    assert!(
        wraps_for(&private_url, &alice_pk).await >= 2,
        "the private chat went by S1"
    );
    alice.shutdown().await;
    bob.shutdown().await;
}

/// Stage Э4, the owner's first situation: B, on S1, gives A (home H alone)
/// the link of the private chat there, and the link of a group of S1. The
/// links carry the card of S1 (`srv=`); A sees it, adds S1 and writes to B
/// there (B's app puts an invitation of his on the link, so A is let in; the
/// link of the kit has none: a request, which B
/// answers), and asks into the group, which B hears on S1.
#[tokio::test]
async fn a_link_with_the_card_of_its_server_leads_there() {
    let home = LocalRelay::builder().build();
    home.run().await.unwrap();
    let home_url = home.url().await.as_str_without_trailing_slash().to_string();
    let private = LocalRelay::builder().build();
    private.run().await.unwrap();
    let private_url = private
        .url()
        .await
        .as_str_without_trailing_slash()
        .to_string();

    let (_a, alice, alice_pk) = home_device(&home_url).await;
    let (_b, bob, bob_pk, s1) = device(&home_url, &private_url).await;
    let alice_on_s1 = format!("{}@{s1}", alice_pk.as_hex());
    let bob_on_s1 = format!("{}@{s1}", bob_pk.as_hex());

    let link = bob.contact_link_on(&bob_pk, None, Some(&s1)).await.unwrap();
    let crate::LinkView::Contact {
        server: Some(preview),
        pubkey,
        invite,
        ..
    } = alice.link_inspect(&link).await.unwrap()
    else {
        panic!("a person with the card of a server")
    };
    assert_eq!((preview.id.as_str(), preview.known), (s1.as_str(), false));
    let id = alice.server_accept(&link).await.unwrap();
    assert_eq!(id, s1);
    let chat = alice
        .contact_open_private(&pubkey, &id, invite.as_deref())
        .await
        .unwrap();
    assert_eq!(chat.id, format!("dm:{bob_on_s1}"));
    assert!(
        invite.is_some(),
        "the link made in the app carries an invitation of B's"
    );
    alice
        .dm_send_text(&bob_on_s1, "found you on s1", None)
        .await
        .unwrap();
    assert_eq!(
        mode_when(&bob, &alice_on_s1, "full_chat").await,
        "full_chat",
        "an invitation: let straight in, no request"
    );
    assert_eq!(
        mode_when(&alice, &bob_on_s1, "full_chat").await,
        "full_chat"
    );

    // The link of the kit (veydan-server card --contact) has no invitation:
    // C's first message is a request B answers.
    let (_c, carol, carol_pk) = home_device(&home_url).await;
    let card = bob.server_card(&s1).await.unwrap();
    let kit_link = messenger_links::ContactLink {
        pubkey: bob_pk.clone(),
        name: String::new(),
        relays: vec![card.relay.clone()],
        invite: None,
        srv: Some(card),
    }
    .encode()
    .unwrap();
    let carol_on_s1 = format!("{}@{s1}", carol_pk.as_hex());
    carol.server_accept(&kit_link).await.unwrap();
    carol
        .contact_open_private(bob_pk.as_hex(), &s1, None)
        .await
        .unwrap();
    carol
        .dm_send_text(&bob_on_s1, "from the kit", None)
        .await
        .unwrap();
    assert_eq!(
        mode_when(&bob, &carol_on_s1, "request_received").await,
        "request_received",
        "a link without an invitation: a request"
    );
    carol.shutdown().await;

    // The link of a group of S1 carries the card too; A has S1 now, so the
    // link is opened at once, on S1.
    let g = bob
        .group_create(crate::GroupKind::Private, "On S1", "", true, Some(&s1))
        .await
        .unwrap();
    let group_link = g.link.clone().expect("the owner may share the link");
    match alice.link_inspect(&group_link).await.unwrap() {
        crate::LinkView::Group {
            server: Some(p),
            server_known,
            ..
        } => assert!(p.id == s1 && p.known && server_known),
        other => panic!("{other:?}"),
    }
    alice.group_open_link(&group_link, "me too").await.unwrap();
    until!(
        "B hears the request on S1",
        !bob.group_get(&g.id).await.unwrap().requests.is_empty()
    );
    assert_eq!(wraps_for(&home_url, &bob_pk).await, 0, "nothing of it at home");
    alice.shutdown().await;
    bob.shutdown().await;
}

/// Both devices of one person are past the list a start publishes, and a
/// second is gone since: a change made now is dated after both, and the
/// relay keeps it. Two revisions made in the same second on two devices
/// are one `d` the relay keeps once, and the device whose word was dropped
/// does not publish it again (a fix of the snapshot owed, not of this test).
async fn lists_settled(a: &MessengerRuntime, b: &MessengerRuntime) {
    async fn past_start(rt: &MessengerRuntime) -> bool {
        let rev = messenger_store::settings::get(rt.store(), messenger_dm::snapshot::KEY_REV).await.unwrap();
        rev.is_some_and(|r| r != "0") && !rt.dm().relations_owed().await.unwrap()
    }
    until!("both lists past the start", past_start(a).await && past_start(b).await);
    tokio::time::sleep(Duration::from_millis(1_100)).await;
}

/// The secrets of the keys of a group this device holds.
async fn group_secrets(rt: &MessengerRuntime, group: &str, keys: &[String]) -> usize {
    let mut n = 0;
    for k in keys {
        if rt.secrets().get(&format!("group.{group}.key.{k}")).await.unwrap().is_some() {
            n += 1;
        }
    }
    n
}

/// Stage Э3 of private servers, the end of it: a server removed takes its
/// groups. Two devices of Alice have S1 (the first by hand, the second by
/// the snapshot); the first makes a group on S1, which the second learns
/// by the welcome to herself there. The first removes S1: the group, its
/// chat and the keys of it go there at once, and on the second by the
/// snapshot, with nothing published about it. A row of the group put back
/// on the first (as a build before this one would have kept it) is closed:
/// not listened to, nothing of it can be written, what is queued for it
/// goes nowhere, and the home relay never holds an event of the group.
#[tokio::test]
async fn a_removed_server_takes_its_groups_on_every_device() {
    let home = LocalRelay::builder().build();
    home.run().await.unwrap();
    let home_url = home.url().await.as_str_without_trailing_slash().to_string();
    let private = LocalRelay::builder().build();
    private.run().await.unwrap();
    let private_url = private.url().await.as_str_without_trailing_slash().to_string();

    let (_a1, alice1, _alice_pk) = plain_device(&home_url, None).await;
    let nsec = alice1.identity().load_keys().await.unwrap().secret_key().to_secret_hex();
    let (_a2, alice2, _) = plain_device(&home_url, Some(&nsec)).await;
    lists_settled(&alice1, &alice2).await;
    let s1 = alice1
        .server_put_private(&PrivateServerInput { relay_url: private_url.clone(), name: Some("S1".into()), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(servers_when(&alice2, &[&s1]).await, vec![s1.clone()]);

    let g = alice1.group_create(crate::GroupKind::Private, "On S1", "", true, Some(&s1)).await.unwrap();
    let chat = format!("group:{}", g.id);
    alice1.group_send_text(&g.id, "hello on s1", None).await.unwrap();
    until!("the second device has the group", alice2.group_get(&g.id).await.is_ok_and(|v| v.membership == "joined"));
    let keys1 = messenger_store::groups::key_ids(alice1.store(), &g.id).await.unwrap();
    assert!(!keys1.is_empty() && group_secrets(&alice1, &g.id, &keys1).await == keys1.len());
    until!("the second device holds its key", {
        let keys2 = messenger_store::groups::key_ids(alice2.store(), &g.id).await.unwrap();
        !keys2.is_empty() && group_secrets(&alice2, &g.id, &keys2).await == keys2.len()
    });
    let keys2 = messenger_store::groups::key_ids(alice2.store(), &g.id).await.unwrap();
    assert_eq!(servers::counts(alice1.store(), &s1).await.unwrap(), (0, 1), "what the question of the UI counts");

    // Removed on the first device: its group goes with it, keys and all.
    alice1.server_remove_private(&s1).await.unwrap();
    assert!(alice1.group_list().await.unwrap().is_empty());
    assert!(alice1.chat(&chat).await.unwrap().is_none());
    assert_eq!(group_secrets(&alice1, &g.id, &keys1).await, 0, "its keys left the secret store");
    assert!(messenger_store::groups::secrets_owed(alice1.store()).await.unwrap().is_empty());
    assert!(alice1.groups().watched().await.unwrap().is_empty());
    assert!(alice1.group_send_text(&g.id, "after", None).await.is_err());

    // And on the second, by the snapshot.
    assert_eq!(servers_when(&alice2, &[]).await, Vec::<String>::new());
    until!(
        "the group went on the second device",
        alice2.group_list().await.unwrap().is_empty() && alice2.chat(&chat).await.unwrap().is_none()
    );
    until!("its keys too", group_secrets(&alice2, &g.id, &keys2).await == 0);

    // A row of the group here all the same, written under its lock as a
    // welcome read while the server went writes it: closed, never home,
    // for as long as the lock is held; then the sweep takes it.
    let lock = alice1.groups().lock_of(&g.id).await;
    let guard = lock.lock().await;
    let row = messenger_store::groups::GroupRow {
        id: g.id.clone(),
        kind: "private".into(),
        name: "On S1".into(),
        about: String::new(),
        picture: String::new(),
        relay_url: private_url.clone(),
        owner: g.owner.clone(),
        membership: "joined".into(),
        my_role: None,
        members: 1,
        link_epoch: 0,
        created_at: 0,
        updated_at: 0,
        needs_update: false,
    };
    messenger_store::groups::upsert(alice1.store(), &row).await.unwrap();
    assert!(servers::group_closed(alice1.store(), &g.id).await.unwrap());
    assert!(alice1.groups().watched().await.unwrap().is_empty(), "not listened to");
    let view = alice1.chat(&chat).await.unwrap().unwrap();
    assert_eq!((view.server_id, view.can_send), (None, false));
    assert!(!alice1.group_get(&g.id).await.unwrap().can_post);
    let event = messenger_core::outbound::WireEvent {
        id: messenger_core::EventId::parse(&"ab".repeat(32)).unwrap(),
        json: serde_json::json!({ "kind": 9, "tags": [["h", g.id]] }),
    };
    let queued = Outbound::PublishScoped { scope: messenger_core::Scope::Group { id: g.id.clone() }, event };
    assert!(scopes::resolve(alice1.store(), queued.clone()).await.is_err(), "no scope for it: not the home set");
    alice1.outbox.enqueue(queued).await.unwrap();
    alice1.outbox.kick();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        on_relay(&home_url, serde_json::json!({ "kinds": [9] })).await.is_empty(),
        "the home relay never held an event of the group"
    );
    let sweep = tokio::spawn({
        let groups = alice1.groups().clone();
        async move { groups.forget_gone().await }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(alice1.group_get(&g.id).await.is_ok(), "the sweep waits for the lock");
    drop(guard);
    sweep.await.unwrap().unwrap();
    assert!(alice1.group_list().await.unwrap().is_empty());
    assert!(alice1.chat(&chat).await.unwrap().is_none());
    alice1.shutdown().await;
    alice2.shutdown().await;
}

/// Stage Э3, the groups of a server that comes mid-session: Alice is in a
/// public group of Bob's, whose relay R2 was a relay of her home set on
/// both her devices. She drops R2 from her home set (the group's relay is
/// then no server of hers, and it is listened to at home, where it is
/// not); Bob writes. Her first device puts R2 in as a private server, the
/// second takes it from the snapshot: both hear the group on R2 at once,
/// with what was written meanwhile and what comes live, without a restart.
#[tokio::test]
async fn the_groups_of_a_server_that_comes_mid_session_are_heard_at_once() {
    let home = LocalRelay::builder().build();
    home.run().await.unwrap();
    let home_url = home.url().await.as_str_without_trailing_slash().to_string();
    let r2 = LocalRelay::builder().build();
    r2.run().await.unwrap();
    let r2_url = r2.url().await.as_str_without_trailing_slash().to_string();

    let (_b, bob, _bob_pk) = home_device(&r2_url).await;
    let g = bob.group_create(crate::GroupKind::Public, "Open", "", true, None).await.unwrap();
    let chat = format!("group:{}", g.id);
    let link = g.link.clone().expect("the owner may share the link");

    let (_a1, alice1, _alice_pk) = plain_device(&home_url, None).await;
    let nsec = alice1.identity().load_keys().await.unwrap().secret_key().to_secret_hex();
    let (_a2, alice2, _) = plain_device(&home_url, Some(&nsec)).await;
    for a in [&alice1, &alice2] {
        a.relays().add_user(&r2_url, None).await.unwrap();
        connected(&*a.relays().pool().await).await;
        a.group_open_link(&link, "").await.unwrap();
        until!("Alice is in", a.group_get(&g.id).await.is_ok_and(|v| v.membership == "joined"));
    }
    bob.group_send_text(&g.id, "one", None).await.unwrap();
    for a in [&alice1, &alice2] {
        until!("Alice reads one", group_texts(a, &chat).await.contains(&"one".to_string()));
    }

    // R2 leaves her home set: the group is listened to at home alone.
    for a in [&alice1, &alice2] {
        a.relays().remove_user(&r2_url).await.unwrap();
        a.group_driver.resubscribe().await;
        assert_eq!(servers::of_group(a.store(), &g.id).await.unwrap(), None);
    }
    bob.group_send_text(&g.id, "two", None).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!group_texts(&alice1, &chat).await.contains(&"two".to_string()), "not heard at home");

    // R2 comes as a private server: by hand on the first device, by the
    // snapshot on the second; the group is of it from then on.
    let id = alice1
        .server_put_private(&PrivateServerInput { relay_url: r2_url.clone(), name: Some("R2".into()), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(servers_when(&alice2, &[&id]).await, vec![id.clone()]);
    for a in [&alice1, &alice2] {
        assert_eq!(servers::of_group(a.store(), &g.id).await.unwrap(), Some(id.clone()));
        until!("Alice reads two, written meanwhile", group_texts(a, &chat).await.contains(&"two".to_string()));
    }
    bob.group_send_text(&g.id, "three", None).await.unwrap();
    for a in [&alice1, &alice2] {
        until!("Alice reads three, live", group_texts(a, &chat).await.contains(&"three".to_string()));
    }
    // What she writes now goes to R2 alone (her join went to her whole
    // home set, R2 among it, while R2 was of it).
    let at_home = on_relay(&home_url, serde_json::json!({ "kinds": [9] })).await.len();
    alice1.group_send_text(&g.id, "four", None).await.unwrap();
    until!("Bob reads four", group_texts(&bob, &chat).await.contains(&"four".to_string()));
    assert_eq!(
        on_relay(&home_url, serde_json::json!({ "kinds": [9] })).await.len(),
        at_home,
        "nothing more of the group at home"
    );
    alice1.shutdown().await;
    alice2.shutdown().await;
    bob.shutdown().await;
}

/// A server removed and put in again brings the history of its private
/// chats back, as the wire document says: its wraps were seen, and the
/// removal forgets them, so neither the ingress nor the reconciliation
/// skips them the second time; the pool hands the ingress the copy the
/// relay sends again (`nostr-sdk` tells an event once per client), and
/// the session listens to the server anew though both changes came
/// before it looked.
#[tokio::test]
async fn a_server_put_in_again_brings_its_history_back() {
    let home = LocalRelay::builder().build();
    home.run().await.unwrap();
    let home_url = home.url().await.as_str_without_trailing_slash().to_string();
    let private = LocalRelay::builder().build();
    private.run().await.unwrap();
    let private_url = private
        .url()
        .await
        .as_str_without_trailing_slash()
        .to_string();
    let (_a, alice, alice_pk, s1) = device(&home_url, &private_url).await;
    let (_b, bob, bob_pk, _) = device(&home_url, &private_url).await;
    alice
        .dm_send_text(&format!("{}@{s1}", bob_pk.as_hex()), "kept on s1", None)
        .await
        .unwrap();
    let bobs_chat = format!("dm:{}@{s1}", alice_pk.as_hex());
    assert_eq!(texts_when(&bob, &bobs_chat, 1).await, vec!["kept on s1"]);

    bob.server_remove_private(&s1).await.unwrap();
    assert!(bob.dm().chat(&bobs_chat).await.unwrap().is_none(), "the chat went with it");
    let again = bob
        .server_put_private(&PrivateServerInput {
            relay_url: private_url.clone(),
            name: Some("S1".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(again, s1);
    // The relay is connected anew before its history is asked: a loaded
    // machine takes its time.
    let mut texts = Vec::new();
    for _ in 0..4 {
        texts = texts_when(&bob, &bobs_chat, 1).await;
        if !texts.is_empty() {
            break;
        }
    }
    assert_eq!(texts, vec!["kept on s1"], "the history of the server came back");
    alice.shutdown().await;
    bob.shutdown().await;
}
