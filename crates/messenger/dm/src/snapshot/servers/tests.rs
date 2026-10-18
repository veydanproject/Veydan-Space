// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use super::super::{Piece, KEY_REV};
use super::*;
use crate::relationship::{MyContact, Relationship};
use messenger_contacts::{ContactService, ProfileService};
use messenger_core::outbound::WireEvent;
use messenger_core::traits::SystemClock;
use messenger_core::Clock;
use messenger_store::settings;
use messenger_store::{media, relays, Store};
use nostr::nips::nip44;
use nostr::prelude::*;
use std::sync::Arc;

async fn device(keys: &Keys) -> DmService {
    let store = Store::open_in_memory().await.unwrap();
    let profiles = ProfileService::new(store.clone());
    let contacts = ContactService::new(store.clone(), profiles.clone());
    let dm = DmService::new(store, contacts, profiles, Arc::new(SystemClock));
    dm.set_signer(Some(keys.clone()));
    dm
}

fn peer(n: usize) -> PubKey {
    PubKey::parse(&format!("{n:064x}")).unwrap()
}

fn now() -> i64 {
    SystemClock.now().secs()
}

fn pieces(events: &[WireEvent], me: &PublicKey) -> Vec<Piece> {
    events
        .iter()
        .map(|e| Piece::of_json(&e.json, me).expect("a piece of mine"))
        .collect()
}

fn plain_header(keys: &Keys, events: &[WireEvent]) -> String {
    let ev: Event = serde_json::from_value(events[0].json.clone()).unwrap();
    nip44::decrypt(keys.secret_key(), &keys.public_key(), &ev.content).unwrap()
}

fn server(url: &str) -> ServerRow {
    ServerRow {
        id: String::new(),
        kind: servers::KIND_PRIVATE.into(),
        name: Some("S1".into()),
        relay_url: url.into(),
        relay_key: Some("app-key".into()),
        media_url: Some("https://media.s1.example".into()),
        call_node: Some("node.s1.example:3478#n1".into()),
        push_mode: servers::PUSH_OFF.into(),
        source: Some(servers::SOURCE_HAND.into()),
        added_at: 0,
    }
}

async fn decided(dm: &DmService, p: &DmPeer, m: MyContact, blocked: bool, decided_at: i64) {
    let r = Relationship {
        my_contact: m,
        blocked,
        decided_at,
        ..Relationship::default()
    };
    dm.save_relation(p.clone(), &r).await.unwrap();
}

async fn relation(dm: &DmService, p: &DmPeer) -> (MyContact, bool, i64) {
    let r = dm.load_relation(p.clone()).await.unwrap();
    (r.my_contact, r.blocked, r.decided_at)
}

/// A server put in on one device, with its key, blob server and call
/// node, and the relations of its chats, come to the other; the home
/// relation of the same person does not move. Its removal takes it there
/// with what is its, a device that still has it later cannot bring it
/// back, and a new put after the removal comes again.
#[tokio::test]
async fn a_private_server_and_its_removal_reach_my_other_device() {
    let keys = Keys::generate();
    let me = keys.public_key();
    let (a, b) = (device(&keys).await, device(&keys).await);
    let id = servers::put_private(a.store(), &server("wss://s1.example"))
        .await
        .unwrap();
    let (x, y) = (
        DmPeer::on(peer(1), Some(&id)),
        DmPeer::on(peer(2), Some(&id)),
    );
    decided(&a, &x, MyContact::None, true, 1_700_000_000).await;
    decided(&a, &y, MyContact::Approved, false, 1_700_000_001).await;
    // A relation on a server this device does not have (any more) stays home.
    decided(
        &a,
        &DmPeer::on(peer(3), Some("0123456789abcdef")),
        MyContact::Approved,
        false,
        1,
    )
    .await;

    let (_, events) = a.build_relations_snapshot(&keys, now()).await.unwrap();
    let plain = plain_header(&keys, &events);
    let h: Header = serde_json::from_str(&plain).unwrap();
    assert_eq!(h.servers.as_ref().map(Vec::len), Some(1));
    assert_eq!(
        h.private_peers.len(),
        2,
        "the blocked first: {:?}",
        h.private_peers
    );
    assert_eq!(
        h.private_peers[0],
        serde_json::json!([peer(1).as_hex(), id, "n", 1, 1_700_000_000])
    );
    assert!(
        h.peers.is_empty(),
        "nothing of a private chat in the home list an older build reads"
    );

    let (applied, _) = b
        .apply_relations_snapshot(&keys, &pieces(&events, &me))
        .await
        .unwrap();
    assert!(applied.servers_changed);
    assert!(!b.relations_owed().await.unwrap(), "a copy is no decision");
    let got = servers::get(b.store(), &id)
        .await
        .unwrap()
        .expect("the server came");
    let there = servers::get(a.store(), &id).await.unwrap().unwrap();
    assert_eq!(
        (
            got.kind.as_str(),
            got.relay_key.as_deref(),
            got.call_node.as_deref(),
            got.added_at,
            got.source.as_deref()
        ),
        (
            servers::KIND_PRIVATE,
            Some("app-key"),
            Some("node.s1.example:3478#n1"),
            there.added_at,
            Some(servers::SOURCE_SNAPSHOT)
        )
    );
    let relay = relays::get(b.store(), "wss://s1.example")
        .await
        .unwrap()
        .expect("its relay");
    assert_eq!(
        (relay.server_id.as_deref(), relay.auth_secret.as_deref()),
        (Some(id.as_str()), Some("app-key"))
    );
    let blobs = media::servers_of(b.store(), Some(&id)).await.unwrap();
    assert_eq!(blobs.len(), 1);
    assert_eq!(
        (blobs[0].url.as_str(), blobs[0].access_key.as_deref()),
        ("https://media.s1.example", Some("app-key"))
    );
    assert_eq!(
        relation(&b, &x).await,
        (MyContact::None, true, 1_700_000_000)
    );
    assert_eq!(
        relation(&b, &y).await,
        (MyContact::Approved, false, 1_700_000_001)
    );
    assert_eq!(
        relation(&b, &DmPeer::home(peer(1))).await,
        (MyContact::None, false, 0),
        "the home chat is another"
    );
    assert!(
        !b.contacts.is_contact(&peer(2)).await.unwrap(),
        "the book is of the home set"
    );
    // The same set again changes nothing.
    let (again, _) = b
        .apply_relations_snapshot(&keys, &pieces(&events, &me))
        .await
        .unwrap();
    assert!(!again.servers_changed && again.changed.is_empty());

    // A third device still holds the server when the removal comes.
    let c = device(&keys).await;
    c.apply_relations_snapshot(&keys, &pieces(&events, &me))
        .await
        .unwrap();
    let chat = messenger_store::chats::ensure_dm(c.store(), peer(2).as_hex(), Some(&id))
        .await
        .unwrap()
        .id;

    // Removed on b: the removal travels and takes it on a and c.
    assert!(servers::remove_by_me(b.store(), &id, now())
        .await
        .unwrap()
        .is_some());
    let (_, removal) = b.build_relations_snapshot(&keys, now()).await.unwrap();
    let h: Header = serde_json::from_str(&plain_header(&keys, &removal)).unwrap();
    assert_eq!(h.servers, Some(vec![]));
    assert_eq!(h.servers_removed.len(), 1);
    assert!(
        h.private_peers.is_empty(),
        "the relations went with the server"
    );
    for d in [&a, &c] {
        let (applied, _) = d
            .apply_relations_snapshot(&keys, &pieces(&removal, &me))
            .await
            .unwrap();
        assert!(applied.servers_changed);
        assert!(servers::get(d.store(), &id).await.unwrap().is_none());
        assert!(relays::get(d.store(), "wss://s1.example")
            .await
            .unwrap()
            .is_none());
        assert!(media::servers_of(d.store(), Some(&id))
            .await
            .unwrap()
            .is_empty());
        assert_eq!(relation(d, &x).await, (MyContact::None, false, 0));
        assert!(servers::removed_at(d.store(), &id).await.unwrap() > there.added_at);
    }
    assert!(
        messenger_store::chats::get(c.store(), &chat)
            .await
            .unwrap()
            .is_none(),
        "its chat went with it"
    );

    // A device that never heard of the removal publishes a newer revision
    // with the server as it was: the removal stands where it is known.
    let stale = device(&keys).await;
    servers::put(
        stale.store(),
        &ServerRow {
            added_at: there.added_at,
            ..server("wss://s1.example")
        },
    )
    .await
    .unwrap();
    settings::set(stale.store(), KEY_REV, "50").await.unwrap();
    let (_, old_word) = stale.build_relations_snapshot(&keys, now()).await.unwrap();
    let (applied, _) = a
        .apply_relations_snapshot(&keys, &pieces(&old_word, &me))
        .await
        .unwrap();
    assert!(!applied.servers_changed);
    assert!(
        servers::get(a.store(), &id).await.unwrap().is_none(),
        "not brought back"
    );

    // Put in again on b, after the removal: it comes to a again.
    servers::put_private(b.store(), &server("wss://s1.example"))
        .await
        .unwrap();
    settings::set(b.store(), KEY_REV, "60").await.unwrap();
    let (_, back) = b.build_relations_snapshot(&keys, now()).await.unwrap();
    let h: Header = serde_json::from_str(&plain_header(&keys, &back)).unwrap();
    assert!(
        h.servers_removed.is_empty(),
        "the new put forgot the removal"
    );
    let (applied, _) = a
        .apply_relations_snapshot(&keys, &pieces(&back, &me))
        .await
        .unwrap();
    assert!(applied.servers_changed);
    assert!(servers::get(a.store(), &id).await.unwrap().is_some());
    assert_eq!(servers::removed_at(a.store(), &id).await.unwrap(), 0);
}

/// The header as the build before servers read it: its fields and no
/// other (serde skips what it does not know).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct HeaderBefore {
    v: u32,
    rev: u64,
    at: i64,
    pages: usize,
    complete: bool,
    dm_from: String,
    keep_days: u32,
    #[serde(default)]
    settings_at: i64,
    #[serde(default)]
    invites: Vec<serde_json::Value>,
    #[serde(default)]
    chats: Vec<(String, i64, i64, i64)>,
    #[serde(default)]
    deleted: Vec<(String, i64, i64)>,
    peers: Vec<(String, String, u8, i64)>,
}

#[tokio::test]
async fn the_build_before_servers_and_this_one_read_each_other() {
    let keys = Keys::generate();
    let me = keys.public_key();
    let a = device(&keys).await;
    let id = servers::put_private(a.store(), &server("wss://s1.example"))
        .await
        .unwrap();
    decided(
        &a,
        &DmPeer::on(peer(1), Some(&id)),
        MyContact::None,
        true,
        1_700_000_000,
    )
    .await;
    decided(
        &a,
        &DmPeer::home(peer(2)),
        MyContact::Approved,
        false,
        1_700_000_001,
    )
    .await;
    let (_, events) = a.build_relations_snapshot(&keys, now()).await.unwrap();
    let before: HeaderBefore = serde_json::from_str(&plain_header(&keys, &events))
        .expect("the older build reads the header");
    assert_eq!(
        before.peers,
        vec![(
            peer(2).as_hex().to_string(),
            "a".to_string(),
            0,
            1_700_000_001
        )],
        "and sees no private relation as a home one"
    );

    // A header of the build before: no servers, which is no word on them.
    let old = serde_json::json!({
        "v": 1, "rev": 9, "at": now(), "pages": 1, "complete": true, "dm_from": "anyone", "keep_days": 7,
        "peers": [[peer(3).as_hex(), "a", 0, 1_700_000_002]],
    })
    .to_string();
    let h: Header = serde_json::from_str(&old).unwrap();
    assert!(h.servers.is_none() && h.servers_removed.is_empty() && h.private_peers.is_empty());
    let content = nip44::encrypt(keys.secret_key(), &me, old, nip44::Version::V2).unwrap();
    let piece = Piece {
        page: 0,
        created_at: now(),
        content,
    };
    a.relations_settled().await.unwrap();
    let (applied, _) = a.apply_relations_snapshot(&keys, &[piece]).await.unwrap();
    assert_eq!((applied.rev, applied.servers_changed), (Some(9), false));
    assert!(
        servers::get(a.store(), &id).await.unwrap().is_some(),
        "an older list removes nothing"
    );
    assert!(
        a.relations_owed().await.unwrap(),
        "the relays are told the servers again"
    );

    // A fresh device with no server is owed nothing by it.
    let fresh = device(&keys).await;
    let empty = serde_json::json!({
        "v": 1, "rev": 9, "at": now(), "pages": 1, "complete": true, "dm_from": "anyone", "keep_days": 7, "peers": [],
    });
    let content = nip44::encrypt(
        keys.secret_key(),
        &me,
        empty.to_string(),
        nip44::Version::V2,
    )
    .unwrap();
    fresh
        .apply_relations_snapshot(
            &keys,
            &[Piece {
                page: 0,
                created_at: now(),
                content,
            }],
        )
        .await
        .unwrap();
    assert!(!fresh.relations_owed().await.unwrap());
}

/// An entry this build cannot read, or one that names another server's
/// relay under its id, is skipped and costs nothing else; more servers
/// than `SERVERS_MAX` are cut to it.
#[tokio::test]
async fn entries_are_read_one_by_one_and_capped() {
    let keys = Keys::generate();
    let me = keys.public_key();
    let a = device(&keys).await;
    for n in 0..SERVERS_MAX + 3 {
        servers::put_private(
            a.store(),
            &ServerRow {
                added_at: 1_700_000_000 + n as i64,
                ..server(&format!("wss://s{n}.example"))
            },
        )
        .await
        .unwrap();
    }
    let (_, events) = a.build_relations_snapshot(&keys, now()).await.unwrap();
    let mut h: Header = serde_json::from_str(&plain_header(&keys, &events)).unwrap();
    let list = h.servers.as_mut().unwrap();
    assert_eq!(list.len(), SERVERS_MAX, "the first added");
    assert_eq!(list[0]["relay_url"], "wss://s0.example");
    // One that lies about its id, one this build cannot read, one good.
    let good = list[1].clone();
    let mut liar = list[0].clone();
    liar["relay_url"] = "wss://elsewhere.example".into();
    h.servers = Some(vec![liar, serde_json::json!({"id": 5}), good.clone()]);
    h.servers_removed = vec![serde_json::json!("junk"), serde_json::json!(["xyz", 5])];
    h.private_peers = vec![serde_json::json!([1, 2])];
    let content = nip44::encrypt(
        keys.secret_key(),
        &me,
        serde_json::to_string(&h).unwrap(),
        nip44::Version::V2,
    )
    .unwrap();
    let b = device(&keys).await;
    let (applied, _) = b
        .apply_relations_snapshot(
            &keys,
            &[Piece {
                page: 0,
                created_at: now(),
                content,
            }],
        )
        .await
        .unwrap();
    assert!(applied.servers_changed);
    let taken = servers::private(b.store()).await.unwrap();
    assert_eq!(
        taken
            .iter()
            .map(|s| s.relay_url.as_str())
            .collect::<Vec<_>>(),
        vec![good["relay_url"].as_str().unwrap()]
    );
    assert!(servers::removed(b.store()).await.unwrap().is_empty());
}

/// The push server of a server travels with it (internal/messenger-wire.md,
/// «Push через шлюз»); what it said of its gateway and its seal does not:
/// each device asks it itself.
#[tokio::test]
async fn the_push_server_of_a_server_reaches_my_other_device() {
    let keys = Keys::generate();
    let me = keys.public_key();
    let (a, b) = (device(&keys).await, device(&keys).await);
    let id = servers::put_private(a.store(), &server("wss://s1.example"))
        .await
        .unwrap();
    assert!(servers::set_push(a.store(), &id, Some("https://s1-push.example/"))
        .await
        .unwrap());
    servers::set_push_seal(
        a.store(),
        &id,
        Some("https://eu-1-vpush.veydan.net"),
        Some(&"ab".repeat(32)),
    )
    .await
    .unwrap();

    let (_, events) = a.build_relations_snapshot(&keys, now()).await.unwrap();
    let h: Header = serde_json::from_str(&plain_header(&keys, &events)).unwrap();
    let entry = &h.servers.as_ref().unwrap()[0];
    assert_eq!(entry["push_url"], "https://s1-push.example");
    assert_eq!(entry["push_mode"], "veydan");
    assert!(entry.get("push_sender").is_none() && entry.get("push_gateway").is_none());

    b.apply_relations_snapshot(&keys, &pieces(&events, &me))
        .await
        .unwrap();
    let push = servers::push_of(b.store(), &id).await.unwrap().unwrap();
    assert_eq!(
        (push.push_mode.as_str(), push.push_url.as_deref()),
        ("veydan", Some("https://s1-push.example"))
    );
    assert_eq!((push.push_gateway, push.push_sender), (None, None));
    assert!(servers::push_senders(b.store()).await.unwrap().is_empty());
}

/// A full list: 500 home decisions, the 300 relations of private chats it
/// carries, and many choices of chats and deletions. The relations of
/// private chats go on the pages like the home ones, the header holds
/// every choice and every deletion, and the other device takes all of it.
#[tokio::test]
async fn a_full_list_keeps_the_choices_and_the_deletions_in_the_header() {
    use messenger_core::relations::RELATIONS_PEERS_PER_PAGE;
    use messenger_store::chats::{self, Pref};
    let keys = Keys::generate();
    let me = keys.public_key();
    let a = device(&keys).await;
    let s1 = servers::put_private(a.store(), &server("wss://relay.s1.example"))
        .await
        .unwrap();
    for n in 1..=RELATIONS_PEERS_PER_PAGE {
        decided(&a, &DmPeer::home(peer(n)), MyContact::Approved, false, 1_700_000_000 + n as i64).await;
    }
    for n in 1..=PRIVATE_PEERS_MAX {
        let p = DmPeer::on(peer(10_000 + n), Some(&s1));
        decided(&a, &p, MyContact::Approved, n % 7 == 0, 1_700_000_000 + n as i64).await;
    }
    let at = now() - 100;
    for n in 1..=120 {
        chats::set_pref(a.store(), &format!("dm:{}", peer(20_000 + n).as_hex()), Pref::Pinned, true, at)
            .await
            .unwrap();
        chats::delete_copy(a.store(), &format!("dm:{}", peer(30_000 + n).as_hex()), at, at, at)
            .await
            .unwrap();
    }

    let (_, events) = a.build_relations_snapshot(&keys, now()).await.unwrap();
    let plain = plain_header(&keys, &events);
    assert!(plain.len() <= super::super::HEADER_MAX_BYTES, "{} bytes", plain.len());
    let h: Header = serde_json::from_str(&plain).unwrap();
    assert_eq!(h.chats.len(), 120, "every choice");
    assert_eq!(h.deleted.len(), 120, "every deletion");
    assert_eq!(h.servers.as_ref().map(Vec::len), Some(1));
    assert!(h.complete);
    // Home first, private after: 250 on the header, 500 a page.
    assert_eq!((h.peers.len(), h.private_peers.len()), (250, 0));
    assert_eq!(events.len(), 3);

    let b = device(&keys).await;
    servers::put_private(b.store(), &server("wss://relay.s1.example"))
        .await
        .unwrap();
    let (applied, _) = b
        .apply_relations_snapshot(&keys, &pieces(&events, &me))
        .await
        .unwrap();
    assert!(applied.complete);
    for n in [1, RELATIONS_PEERS_PER_PAGE] {
        assert_eq!(
            relation(&b, &DmPeer::home(peer(n))).await,
            (MyContact::Approved, false, 1_700_000_000 + n as i64)
        );
    }
    for n in [1, 7, PRIVATE_PEERS_MAX] {
        assert_eq!(
            relation(&b, &DmPeer::on(peer(10_000 + n), Some(&s1))).await,
            (MyContact::Approved, n % 7 == 0, 1_700_000_000 + n as i64),
            "the relation of a private chat from a page"
        );
    }
    // The blocks of private chats leave tombstones of their own here.
    let deleted = chats::tombstones(b.store()).await.unwrap().into_iter().filter(|(id, _, _)| !id.contains('@')).count();
    assert_eq!(deleted, 120, "every deletion taken");
    let pinned = chats::prefs(b.store()).await.unwrap().into_iter().filter(|r| r.get(Pref::Pinned).0).count();
    assert_eq!(pinned, 120, "every choice taken");
}
