// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The keeper and the member side of one device: what the keeper makes
//! for its own user (the issue, the key of an epoch it published, its
//! snapshot) is taken there at once, with no echo of the relay and no
//! blob fetched from the hosts (живой запуск 2026-10-23: «Ждём ключ
//! хранителя» after making a channel, no post in a section just made).

use crate::db;
use crate::op::{History, JoinMode, OpBody};
use crate::service::{NewChannel, MEMBERSHIP_JOINED};
use crate::state::SCOPE_EVERYONE;
use crate::testkit::{Node, World, H1, H2};
use crate::wire::{KIND_EPOCH, KIND_SNAPSHOT};
use keydist::{EpochKey, SchemeId};
use messenger_core::TextFormat;
use std::sync::atomic::Ordering;

fn new_channel() -> NewChannel {
    NewChannel {
        name: "News".into(),
        about: String::new(),
        picture: String::new(),
        join: JoinMode::Open,
        history: History::Open,
        relays: vec!["wss://r1.example".into(), "wss://r2.example".into()],
        storage: vec![H1.into(), H2.into()],
    }
}

async fn key_of(n: &Node, cid: &str, scope_id: &str) -> Option<EpochKey> {
    n.svc.member(&n.keys).newest_key(cid, scope_id).await.unwrap_or_else(|e| panic!("keys: {e}"))
}

async fn tick(n: &Node) {
    n.svc.tick().await.unwrap_or_else(|e| panic!("tick: {e}"));
}

async fn join_by_link(w: &World, owner: &Node, reader: &Node, cid: &str, all: &[&Node]) {
    let link = owner.svc.share_link(cid).await.unwrap_or_else(|e| panic!("link: {e}"));
    w.pump(&[reader]).await;
    reader.svc.open_link(&reader.keys, &link, "").await.unwrap_or_else(|e| panic!("open link: {e}"));
    w.pump(all).await;
}

async fn post_texts(n: &Node, cid: &str) -> Vec<String> {
    let mut t: Vec<String> = db::content::posts(n.svc.db(), cid, None, 100).await.unwrap_or_default().into_iter().filter_map(|p| p.text).collect();
    t.sort();
    t
}

fn gets(w: &World) -> usize {
    w.hosts.gets.load(Ordering::SeqCst)
}

#[tokio::test]
async fn the_owner_holds_the_key_of_a_new_channel_with_no_echo_and_no_fetch() {
    let w = World::new();
    let owner = w.node().await;
    // Nothing pumped: the owner's device hears nothing of the relay. The
    // journal goes out and the first epoch is published by the keeper's
    // turn the create asked for (`tick`: the turn and the outbox, no
    // event of the relay read).
    let (cid, _) = owner.svc.create(&owner.keys, new_channel()).await.unwrap_or_else(|e| panic!("create: {e}"));
    tick(&owner).await;
    assert_eq!(w.relay.epochs(), vec![1]);
    assert_eq!(key_of(&owner, &cid, SCOPE_EVERYONE).await.map(|k| k.epoch), Some(1), "the key of the epoch the keeper published");
    let rows = db::keys::epochs(owner.svc.db(), &cid, SCOPE_EVERYONE).await.unwrap_or_default();
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].event_json.is_empty() && !rows[0].digest.is_empty(), "the row has its event, as one heard from a relay");
    assert_eq!(db::keys::current_epoch(owner.svc.db(), &cid, SCOPE_EVERYONE).await.ok().flatten(), Some(1));
    assert!(!db::keys::bundles(owner.svc.db(), &cid, SCOPE_EVERYONE).await.unwrap_or_default().is_empty(), "my bundle, from the issue taken here");
    // The issue still went, for my other devices.
    assert!(w.relay.inbox.lock().map(|i| i.get(owner.me.as_hex()).map_or(0, Vec::len)).unwrap_or(0) > 0);
    owner.svc.send_post(&owner.keys, &cid, "first", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    assert_eq!(gets(&w), 0, "no shard fetched by the owner's own device");

    // The echo of the relay: the same epoch and issue again change nothing.
    w.pump(&[&owner]).await;
    assert!(owner.errors().is_empty(), "{:?}", owner.errors());
    assert_eq!(gets(&w), 0, "the echo of the epoch fetched nothing");
    let again = db::keys::epochs(owner.svc.db(), &cid, SCOPE_EVERYONE).await.unwrap_or_default();
    assert_eq!(again, rows);
    assert_eq!(db::channels::get(owner.svc.db(), &cid).await.ok().flatten().map(|r| r.membership).as_deref(), Some(MEMBERSHIP_JOINED));
    // The secret of the link went out once, under the key taken here.
    assert_eq!(w.relay.of_kind(crate::wire::KIND_CONTENT).len(), 2, "the link and the post");

    // A reader reads the post.
    let reader = w.node().await;
    join_by_link(&w, &owner, &reader, &cid, &[&owner, &reader]).await;
    assert_eq!(post_texts(&reader, &cid).await, ["first"]);
}

#[tokio::test]
async fn the_owner_posts_in_a_section_just_made_with_no_echo() {
    for scheme in [SchemeId::SdV1, SchemeId::FlatV1] {
        let w = World::new();
        let owner = w.node().await;
        let reader = w.node().await;
        let (cid, _) = owner.svc.create(&owner.keys, new_channel()).await.unwrap_or_else(|e| panic!("create: {e}"));
        let (gold, _) = owner.svc.scope_add(&owner.keys, &cid, "Premium", scheme, History::Open).await.unwrap_or_else(|e| panic!("scope_add: {e}"));
        tick(&owner).await;
        assert!(!w.relay.epochs_of(&gold).is_empty(), "{scheme:?}: the first epoch of the section is out");
        let k = key_of(&owner, &cid, &gold).await.unwrap_or_else(|| panic!("{scheme:?}: the owner holds the key of the section"));
        assert_eq!(Some(k.epoch), w.relay.epochs_of(&gold).into_iter().max());
        owner.svc.send_post_in(&owner.keys, &cid, Some(&gold), "premium", TextFormat::Plain).await.unwrap_or_else(|e| panic!("{scheme:?}: post: {e}"));
        assert_eq!(gets(&w), 0, "{scheme:?}: nothing fetched");
        // The echo changes nothing; a reader of everyone does not see the section.
        join_by_link(&w, &owner, &reader, &cid, &[&owner, &reader]).await;
        assert!(owner.errors().is_empty(), "{scheme:?}: {:?}", owner.errors());
        assert_eq!(key_of(&owner, &cid, &gold).await, Some(k));
        assert!(post_texts(&reader, &cid).await.is_empty(), "{scheme:?}");
    }
}

/// Two rotations the owner's device never heard back from the relay (a
/// ban, then an urgent epoch): the owner's next post is sealed under the
/// newest epoch, and a reader takes it live.
#[tokio::test]
async fn the_owner_posts_under_the_newest_epoch_right_after_a_rotation() {
    let w = World::new();
    let owner = w.node().await;
    let (a, b) = (w.node().await, w.node().await);
    let all = [&owner, &a, &b];
    let (cid, _) = owner.svc.create(&owner.keys, new_channel()).await.unwrap_or_else(|e| panic!("create: {e}"));
    // The journal and the first epoch out, by the turn the create asked for.
    tick(&owner).await;
    join_by_link(&w, &owner, &a, &cid, &all).await;
    join_by_link(&w, &owner, &b, &cid, &all).await;
    owner.skip_all(&w.relay);
    owner.hold(KIND_EPOCH, true);
    owner.svc.act(&owner.keys, &cid, OpBody::Ban { who: a.me.clone(), reason: None }).await.unwrap_or_else(|e| panic!("ban: {e}"));
    owner.svc.rotate_now(&cid).await.unwrap_or_else(|e| panic!("rotate: {e}"));
    tick(&owner).await;
    owner.svc.rotate_now(&cid).await.unwrap_or_else(|e| panic!("rotate: {e}"));
    tick(&owner).await;
    let newest = w.relay.epochs().into_iter().max().unwrap_or(0);
    assert!(newest >= 3, "two epochs after the first: {:?}", w.relay.epochs());
    assert_eq!(key_of(&owner, &cid, SCOPE_EVERYONE).await.map(|k| k.epoch), Some(newest), "taken at publication, not by the echo");
    owner.svc.send_post(&owner.keys, &cid, "after the ban", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    // The post to the relay, the owner hearing nothing back.
    owner.svc.flush_outbox().await.unwrap_or_else(|e| panic!("flush: {e}"));
    w.pump(&[&b]).await;
    assert_eq!(post_texts(&b, &cid).await, ["after the ban"], "taken live, not «under an old epoch»");
    assert!(b.errors().is_empty(), "{:?}", b.errors());
}

async fn thread_texts(n: &Node, cid: &str, post: &str) -> Vec<String> {
    let page = n.svc.thread(&n.keys, cid, post, 100, None).await.unwrap_or_else(|e| panic!("thread: {e}"));
    page.items.into_iter().filter_map(|i| i.text).collect()
}

fn snapshots(w: &World) -> Vec<crate::snapshot::Snapshot> {
    w.relay.of_kind(KIND_SNAPSHOT).iter().map(|e| crate::snapshot::content_of(&e.to_string()).unwrap_or_else(|e| panic!("{e}"))).collect()
}

async fn pending(n: &Node, cid: &str) -> i64 {
    db::channels::count_pending(n.svc.db(), cid).await.unwrap_or(-1)
}

/// The owner's device never hears the snapshot that names a new reader
/// back from the relay: it took it from its keeper, shards and all, and
/// places the reader's comment at once, fetching nothing.
#[tokio::test]
async fn a_new_readers_comment_is_placed_on_the_owners_device_with_no_echo_of_the_snapshot() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let (cid, _) = owner.svc.create(&owner.keys, new_channel()).await.unwrap_or_else(|e| panic!("create: {e}"));
    w.pump(&[&owner]).await;
    owner.hold(KIND_SNAPSHOT, true);
    w.clock.advance(400);
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    tick(&owner).await;
    let s = snapshots(&w);
    assert_eq!(s.last().map(|s| s.members), Some(2), "a snapshot names the reader");
    let (post, _) = owner.svc.send_post(&owner.keys, &cid, "the post", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    w.clock.advance(1);
    reader.svc.send_comment(&reader.keys, &cid, &post, None, "by the reader", TextFormat::Plain).await.unwrap_or_else(|e| panic!("comment: {e}"));
    // The comment to the relay; the reader's device fetches nothing here.
    reader.svc.flush_outbox().await.unwrap_or_else(|e| panic!("flush: {e}"));
    let before = gets(&w);
    w.pump(&[&owner]).await;
    assert_eq!(thread_texts(&owner, &cid, &post).await, ["by the reader"], "placed, not waiting for a snapshot");
    assert_eq!(pending(&owner, &cid).await, 0);
    assert_eq!(gets(&w), before, "the shard came from the keeper, not from the hosts");
    assert!(owner.errors().is_empty(), "{:?}", owner.errors());
}

/// A member holds a snapshot whose `epoch_ref` key it has not opened yet
/// (the epoch event came late): a comment of a reader it names waits for
/// the shard, and is read again when that key comes, not at the next
/// snapshot.
#[tokio::test]
async fn a_comment_that_waited_for_the_shard_is_read_again_when_the_key_of_its_epoch_comes() {
    let w = World::new();
    let owner = w.node().await;
    let (b, c) = (w.node().await, w.node().await);
    let all = [&owner, &b, &c];
    let (cid, _) = owner.svc.create(&owner.keys, new_channel()).await.unwrap_or_else(|e| panic!("create: {e}"));
    w.pump(&[&owner]).await;
    w.clock.advance(400);
    join_by_link(&w, &owner, &b, &cid, &all).await;
    tick(&owner).await;
    w.pump(&all).await;
    let (post, _) = owner.svc.send_post(&owner.keys, &cid, "the post", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    join_by_link(&w, &owner, &c, &cid, &all).await;
    // b hears no epoch event for now; the next epoch and the snapshot that
    // names c, sealed under it, go out.
    b.hold(KIND_EPOCH, true);
    w.clock.advance(400);
    owner.svc.rotate_now(&cid).await.unwrap_or_else(|e| panic!("rotate: {e}"));
    tick(&owner).await;
    let newest = w.relay.epochs().into_iter().max().unwrap_or(0);
    let s = snapshots(&w);
    let last = s.last().unwrap_or_else(|| panic!("no snapshot"));
    assert_eq!((last.members, last.epoch_ref.get(SCOPE_EVERYONE).copied()), (3, Some(newest)), "the snapshot names c, under the new epoch");
    // c writes under the key it holds; b holds that key, not the snapshot's.
    w.clock.advance(1);
    c.svc.send_comment(&c.keys, &cid, &post, None, "by c", TextFormat::Plain).await.unwrap_or_else(|e| panic!("comment: {e}"));
    c.svc.flush_outbox().await.unwrap_or_else(|e| panic!("flush: {e}"));
    w.pump(&[&b]).await;
    assert!(thread_texts(&b, &cid, &post).await.is_empty(), "waits for the shard");
    assert_eq!(pending(&b, &cid).await, 1);
    // The epoch event comes: its key opens the shard, and the comment is read again.
    b.hold(KIND_EPOCH, false);
    w.pump(&[&b]).await;
    assert_eq!(thread_texts(&b, &cid, &post).await, ["by c"]);
    assert_eq!(pending(&b, &cid).await, 0);
    assert!(b.errors().is_empty(), "{:?}", b.errors());
}

/// An epoch the relays refuse waits; the snapshot that names a newcomer
/// does not wait behind it, sealed under the epoch published before, as
/// long as nobody was put out since that one was built. After a ban it
/// waits for the epoch that leaves the banned reader out.
#[tokio::test]
async fn a_snapshot_goes_under_the_published_epoch_while_the_next_waits_unless_somebody_was_put_out() {
    let w = World::new();
    let owner = w.node().await;
    let (a, c, d) = (w.node().await, w.node().await, w.node().await);
    let all = [&owner, &a, &c, &d];
    let (cid, _) = owner.svc.create(&owner.keys, new_channel()).await.unwrap_or_else(|e| panic!("create: {e}"));
    w.pump(&[&owner]).await;
    w.clock.advance(400);
    join_by_link(&w, &owner, &a, &cid, &all).await;
    tick(&owner).await;
    // The next epoch is built and its event refused: it waits.
    w.relay.refuse(KIND_EPOCH, true);
    owner.svc.rotate_now(&cid).await.unwrap_or_else(|e| panic!("rotate: {e}"));
    tick(&owner).await;
    let published = w.relay.epochs().into_iter().max().unwrap_or(0);
    let status = owner.svc.keeper_status(&cid).unwrap_or_else(|| panic!("no status"));
    assert_eq!((status.epoch, status.pending_epoch.is_some()), (Some(published), true), "an epoch waits for its relays");
    // A newcomer: the snapshot names them under the published epoch.
    join_by_link(&w, &owner, &c, &cid, &all).await;
    w.clock.advance(400);
    tick(&owner).await;
    let last = snapshots(&w).last().cloned().unwrap_or_else(|| panic!("no snapshot"));
    assert_eq!((last.members, last.epoch_ref.get(SCOPE_EVERYONE).copied()), (3, Some(published)), "not held back by the waiting epoch");
    // A ban: no snapshot under a key the banned reader holds.
    let made = snapshots(&w).len();
    owner.svc.act(&owner.keys, &cid, OpBody::Ban { who: a.me.clone(), reason: None }).await.unwrap_or_else(|e| panic!("ban: {e}"));
    join_by_link(&w, &owner, &d, &cid, &all).await;
    w.clock.advance(400);
    tick(&owner).await;
    assert_eq!(snapshots(&w).len(), made, "waits for the epoch that puts the banned reader out");
    // The relays take the epochs again: the one built over the ban goes out, then the snapshot.
    w.relay.refuse(KIND_EPOCH, false);
    for _ in 0..3 {
        w.clock.advance(400);
        tick(&owner).await;
    }
    let last = snapshots(&w).last().cloned().unwrap_or_else(|| panic!("no snapshot"));
    let newest = w.relay.epochs().into_iter().max().unwrap_or(0);
    assert_eq!((last.members, last.epoch_ref.get(SCOPE_EVERYONE).copied()), (3, Some(newest)), "the owner, c and d, under the epoch after the ban");
    assert!(newest > published + 1, "an epoch built over the ban: {:?}", w.relay.epochs());
}
