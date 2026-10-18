// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The private chat (internal/messenger-wire.md, «Частный чат»): one person,
//! two chats — the home one and one on a private server — kept apart by the
//! `srv` tag of the rumor, or by the relay a wrap without one came by.

use super::*;
use crate::relationship::Action;
use messenger_core::Scope;
use messenger_store::servers::{self, ServerRow};

const S1_RELAY: &str = "wss://s1.example";

/// `p` has the private server of `relay` (as a card would add it); its id.
async fn add_server(p: &Party, relay: &str) -> String {
    let row = ServerRow {
        id: String::new(),
        kind: servers::KIND_PRIVATE.into(),
        name: Some("S1".into()),
        relay_url: relay.into(),
        relay_key: Some("k".into()),
        media_url: None,
        call_node: None,
        push_mode: servers::PUSH_OFF.into(),
        source: None,
        added_at: 0,
    };
    servers::put(p.dm.store(), &row).await.unwrap()
}

fn on(p: &Party, server: &str) -> DmPeer {
    DmPeer::on(p.pk(), Some(server))
}

/// The wrap of an outbound of the private chat, which must go to `server`
/// and to nothing else.
fn scoped(out: &Outbound, server: &str) -> WireEvent {
    match out {
        Outbound::PublishScoped {
            scope: Scope::Server { id },
            event,
        } if id == server => event.clone(),
        other => panic!("not scoped to the server: {other:?}"),
    }
}

fn addressed_to(w: &WireEvent, p: &Party) -> bool {
    let event: Event = serde_json::from_value(w.json.clone()).unwrap();
    event.tags.iter().any(|t| {
        t.kind() == "p" && t.as_slice().get(1).map(String::as_str) == Some(p.pk().as_hex())
    })
}

async fn texts(p: &Party, chat_id: &str) -> Vec<String> {
    p.dm.messages(chat_id, None, 100)
        .await
        .unwrap()
        .into_iter()
        .filter(|m| m.content_type != "system")
        .map(|m| m.text.unwrap_or_default())
        .collect()
}

#[tokio::test]
async fn two_chats_with_one_peer_are_kept_apart_by_srv() {
    let (alice, bob) = (Party::new().await, Party::new().await);
    let s1 = add_server(&alice, S1_RELAY).await;
    assert_eq!(
        add_server(&bob, S1_RELAY).await,
        s1,
        "the same id on every device"
    );

    let home = alice
        .dm
        .prepare_text(&alice.keys, &bob.pk(), "at home", None)
        .await
        .unwrap();
    let private = alice
        .dm
        .prepare_text(&alice.keys, on(&bob, &s1), "on s1", None)
        .await
        .unwrap();
    assert!(matches!(home.to_peer, Outbound::PublishToInbox { .. }));
    assert_eq!(
        private.message.chat_id,
        format!("dm:{}@{s1}", bob.pk().as_hex())
    );
    let to_bob = scoped(&private.to_peer, &s1);
    scoped(private.to_self.as_ref().unwrap(), &s1);

    // Both by the same relay: the tag alone tells the chats apart.
    bob.receive(&peer_event(&home)).await;
    bob.receive(&to_bob).await;
    let home_chat = chats::dm_chat_id(alice.pk().as_hex(), None);
    let private_chat = chats::dm_chat_id(alice.pk().as_hex(), Some(&s1));
    assert_eq!(texts(&bob, &home_chat).await, vec!["at home"]);
    assert_eq!(texts(&bob, &private_chat).await, vec!["on s1"]);
    let listed = bob.dm.list_chats(false).await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(
        listed
            .iter()
            .find(|c| c.id == private_chat)
            .unwrap()
            .server_id
            .as_deref(),
        Some(s1.as_str())
    );
    assert_eq!(
        listed.iter().find(|c| c.id == home_chat).unwrap().server_id,
        None
    );

    // The answer, an edit and a deletion stay in the private chat and on
    // its server.
    let reply = bob
        .dm
        .prepare_text(
            &bob.keys,
            on(&alice, &s1),
            "back on s1",
            Some(&private.message.id),
        )
        .await
        .unwrap();
    alice.receive(&scoped(&reply.to_peer, &s1)).await;
    assert_eq!(
        texts(&alice, &chats::dm_chat_id(bob.pk().as_hex(), Some(&s1))).await,
        vec!["on s1", "back on s1"]
    );
    assert_eq!(
        texts(&alice, &chats::dm_chat_id(bob.pk().as_hex(), None)).await,
        vec!["at home"]
    );

    alice.clock.0.store(1_000_100, Ordering::SeqCst);
    let edit = alice
        .dm
        .prepare_edit(&alice.keys, &private.message.id, "on s1, edited")
        .await
        .unwrap();
    bob.receive(&scoped(&edit.to_peer, &s1)).await;
    scoped(edit.to_self.as_ref().unwrap(), &s1);
    assert_eq!(
        texts(&bob, &private_chat).await,
        vec!["on s1, edited", "back on s1"]
    );
    let delete = alice
        .dm
        .prepare_delete(&alice.keys, &private.message.id)
        .await
        .unwrap();
    bob.receive(&scoped(&delete.to_peer, &s1)).await;
    assert!(
        bob.dm
            .message(&private.message.id)
            .await
            .unwrap()
            .unwrap()
            .deleted
    );
    assert_eq!(
        texts(&bob, &home_chat).await,
        vec!["at home"],
        "the home chat is untouched"
    );
}

#[tokio::test]
async fn a_wrap_without_srv_on_a_private_servers_relay_lands_in_the_private_chat() {
    let (alice, bob) = (Party::new().await, Party::new().await);
    let s1 = add_server(&bob, "wss://S1.example/").await;
    // A client that knows no `srv` writes to the server's relay.
    let p = alice
        .dm
        .prepare_text(&alice.keys, &bob.pk(), "from an old client", None)
        .await
        .unwrap();
    bob.receive_by(&peer_event(&p), S1_RELAY, false).await;
    assert_eq!(
        texts(&bob, &chats::dm_chat_id(alice.pk().as_hex(), Some(&s1))).await,
        vec!["from an old client"]
    );
    assert!(bob
        .dm
        .chat(&chats::dm_chat_id(alice.pk().as_hex(), None))
        .await
        .unwrap()
        .is_none());
    // History caught up from the same relay goes the same way.
    let older = alice
        .dm
        .prepare_text(&alice.keys, &bob.pk(), "synced", None)
        .await
        .unwrap();
    bob.receive_by(&peer_event(&older), S1_RELAY, true).await;
    assert_eq!(
        texts(&bob, &chats::dm_chat_id(alice.pk().as_hex(), Some(&s1)))
            .await
            .len(),
        2
    );
}

#[tokio::test]
async fn a_wrap_naming_a_server_this_device_does_not_have_is_dropped() {
    let (alice, bob) = (Party::new().await, Party::new().await);
    let s1 = add_server(&alice, S1_RELAY).await;
    let p = alice
        .dm
        .prepare_text(&alice.keys, on(&bob, &s1), "on s1", None)
        .await
        .unwrap();
    let fx = bob.receive(&scoped(&p.to_peer, &s1)).await;
    assert!(fx.is_empty(), "{fx:?}");
    assert!(
        bob.dm.list_chats(true).await.unwrap().is_empty(),
        "no chat, home or private"
    );
}

#[tokio::test]
async fn a_relation_is_kept_per_server() {
    let (alice, bob) = mutual().await;
    let s1 = add_server(&alice, S1_RELAY).await;
    add_server(&bob, S1_RELAY).await;
    bob.dm
        .act(&bob.keys, &alice.pk(), Action::Block)
        .await
        .unwrap();
    assert_eq!(mode(&bob, &alice).await, "blocked");

    // Alice writes on S1: a first message of a new chat, a request there.
    let p = alice
        .dm
        .prepare_text(&alice.keys, on(&bob, &s1), "it is me, on s1", None)
        .await
        .unwrap();
    assert!(p.followups.iter().all(|o| matches!(
        o,
        Outbound::PublishScoped {
            scope: Scope::Server { .. },
            ..
        }
    )));
    assert!(!p.became_contact, "the address book is of the home set");
    bob.receive(&scoped(&p.to_peer, &s1)).await;
    // The followups hold my copies too, on the same server: the peer's
    // are the ones addressed to them.
    for f in &p.followups {
        let w = scoped(f, &s1);
        if addressed_to(&w, &bob) {
            bob.receive(&w).await;
        }
    }
    assert_eq!(
        bob.dm.relation(on(&alice, &s1)).await.unwrap().mode,
        "request_received",
        "open on the server"
    );
    assert_eq!(
        bob.dm
            .relation(on(&alice, &s1))
            .await
            .unwrap()
            .server_id
            .as_deref(),
        Some(s1.as_str())
    );
    assert_eq!(mode(&bob, &alice).await, "blocked", "blocked at home");
    let q = bob.dm.requests(None, 10).await.unwrap();
    assert_eq!(
        q.iter().map(|r| r.chat_id.clone()).collect::<Vec<_>>(),
        vec![chats::dm_chat_id(alice.pk().as_hex(), Some(&s1))]
    );

    // Accepted on the server: the signals go there, the home block stands.
    let acc = bob
        .dm
        .act(&bob.keys, on(&alice, &s1), Action::Accept)
        .await
        .unwrap();
    assert!(!acc.outbounds.is_empty());
    for o in &acc.outbounds {
        scoped(o, &s1);
    }
    assert_eq!(
        bob.dm.relation(on(&alice, &s1)).await.unwrap().mode,
        "full_chat"
    );
    assert!(bob.dm.is_blocked(&alice.pk()).await.unwrap());
    assert!(!bob.dm.is_blocked(on(&alice, &s1)).await.unwrap());
    assert!(
        bob.dm.calls_allowed(on(&alice, &s1)).await.unwrap(),
        "calls in the private chat"
    );
    assert!(
        !bob.dm.calls_allowed(&alice.pk()).await.unwrap(),
        "no calls at home"
    );
}

#[tokio::test]
async fn a_reaction_of_a_private_chat_goes_to_its_server() {
    let (alice, bob) = (Party::new().await, Party::new().await);
    let s1 = add_server(&alice, S1_RELAY).await;
    add_server(&bob, S1_RELAY).await;
    let p = alice
        .dm
        .prepare_text(&alice.keys, on(&bob, &s1), "react to me", None)
        .await
        .unwrap();
    bob.receive(&scoped(&p.to_peer, &s1)).await;
    let r = bob
        .dm
        .prepare_reaction(&bob.keys, &p.message.id, "👍")
        .await
        .unwrap();
    let to_alice = scoped(&r.to_peer, &s1);
    scoped(r.to_self.as_ref().unwrap(), &s1);
    alice.receive(&to_alice).await;
    let shown = alice
        .dm
        .message(&p.message.id)
        .await
        .unwrap()
        .unwrap()
        .reactions;
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].emoji, "👍");
}

/// A system line of the same kind and second in both chats is two rows,
/// one in each.
#[tokio::test]
async fn system_lines_of_the_two_chats_do_not_share_a_row() {
    let (alice, bob) = (gated().await, gated().await);
    let s1 = add_server(&alice, S1_RELAY).await;
    add_server(&bob, S1_RELAY).await;
    let home = alice
        .dm
        .prepare_text(&alice.keys, &bob.pk(), "hi", None)
        .await
        .unwrap();
    let private = alice
        .dm
        .prepare_text(&alice.keys, on(&bob, &s1), "hi", None)
        .await
        .unwrap();
    bob.receive(&peer_event(&home)).await;
    bob.receive(&scoped(&private.to_peer, &s1)).await;
    let lines = |chat: String| {
        let bob = &bob;
        async move {
            bob.dm
                .messages(&chat, None, 100)
                .await
                .unwrap()
                .into_iter()
                .filter(|m| m.content_type == "system")
                .count()
        }
    };
    assert_eq!(lines(chats::dm_chat_id(alice.pk().as_hex(), None)).await, 1);
    assert_eq!(
        lines(chats::dm_chat_id(alice.pk().as_hex(), Some(&s1))).await,
        1
    );
}
