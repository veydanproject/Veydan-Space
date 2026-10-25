// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Where the comment of a new reader is placed, and how soon (живой запуск
//! 2026-10-23: a reader joined and commented within a minute, and the
//! owner saw the comment two to six minutes later): the keeper's own
//! device by its rows, at once; the other members by a snapshot the
//! keeper of a small channel makes `SETTLE_SECS` after the change; a
//! comment whose shard the hosts did not give is read again once they
//! give it, with no new snapshot.

use crate::db;
use crate::member::readers::Standing;
use crate::op::{History, JoinMode, OpBody};
use crate::service::NewChannel;
use crate::state::Reader;
use crate::testkit::{Node, World, H1, H2};
use crate::wire::KIND_SNAPSHOT;
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

async fn join_by_link(w: &World, owner: &Node, reader: &Node, cid: &str, all: &[&Node]) {
    let link = owner.svc.share_link(cid).await.unwrap_or_else(|e| panic!("link: {e}"));
    w.pump(&[reader]).await;
    reader.svc.open_link(&reader.keys, &link, "").await.unwrap_or_else(|e| panic!("open link: {e}"));
    w.pump(all).await;
}

fn snapshots(w: &World) -> Vec<crate::snapshot::Snapshot> {
    w.relay.of_kind(KIND_SNAPSHOT).iter().map(|e| crate::snapshot::content_of(&e.to_string()).unwrap_or_else(|e| panic!("{e}"))).collect()
}

fn gets(w: &World) -> usize {
    w.hosts.gets.load(Ordering::SeqCst)
}

async fn pending(n: &Node, cid: &str) -> i64 {
    db::channels::count_pending(n.svc.db(), cid).await.unwrap_or(-1)
}

async fn thread_texts(n: &Node, cid: &str, post: &str) -> Vec<String> {
    let page = n.svc.thread(&n.keys, cid, post, 100, None).await.unwrap_or_else(|e| panic!("thread: {e}"));
    page.items.into_iter().filter_map(|i| i.text).collect()
}

/// What `n` says of `who`; a shard not in its cache fetched first, as the
/// way of a comment would have it fetched (`reader_of` fetches nothing).
async fn standing(n: &Node, cid: &str, who: &Node, at: i64) -> Option<Standing> {
    let log = n.svc.log(cid).await.ok().flatten().unwrap_or_else(|| panic!("log"));
    let member = n.svc.member(&n.keys);
    let first = member.reader_of(&log, &who.me, at).await.ok()?;
    let Standing::Unfetched(index) = first else { return Some(first) };
    let snapshot = member.latest_snapshot(cid).await.ok().flatten().unwrap_or_else(|| panic!("no snapshot"));
    n.svc.env.fetches.want(cid, crate::member::fetch::Want::Shard { at: snapshot.at, index });
    n.svc.run_fetches(&n.keys).await;
    member.reader_of(&log, &who.me, at).await.ok()
}

/// A channel past the window of its first snapshot, with `a` a reader
/// the newest snapshot names and a post of the owner; nothing left to
/// make (the clock stands right after that snapshot).
async fn channel_with_a_reader(w: &World, owner: &Node, a: &Node, all: &[&Node]) -> (String, String) {
    let (cid, _) = owner.svc.create(&owner.keys, new_channel()).await.unwrap_or_else(|e| panic!("create: {e}"));
    w.pump(&[owner]).await;
    w.clock.advance(400);
    join_by_link(w, owner, a, &cid, all).await;
    let names_a = snapshots(w).last().is_some_and(|s| s.members == 2);
    assert!(names_a, "the snapshot after a's join: {:?}", snapshots(w).iter().map(|s| s.members).collect::<Vec<_>>());
    let (post, _) = owner.svc.send_post(&owner.keys, &cid, "the post", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(all).await;
    (cid, post)
}

/// A reader who came after the newest snapshot comments at once: the
/// keeper's device places the comment by its rows, with no snapshot and
/// nothing fetched; a member places it by the snapshot the keeper makes
/// `SETTLE_SECS` after the join, not five minutes later.
#[tokio::test]
async fn a_new_readers_comment_is_placed_by_the_keeper_at_once_and_by_the_others_after_the_settle() {
    let w = World::new();
    let owner = w.node().await;
    let (a, r) = (w.node().await, w.node().await);
    let all = [&owner, &a, &r];
    let (cid, post) = channel_with_a_reader(&w, &owner, &a, &all).await;
    let made = snapshots(&w).len();
    let joined = w.clock.now();
    join_by_link(&w, &owner, &r, &cid, &all).await;
    assert_eq!(snapshots(&w).len(), made, "the join settles first");
    w.clock.advance(1);
    r.svc.send_comment(&r.keys, &cid, &post, None, "by the newcomer", TextFormat::Plain).await.unwrap_or_else(|e| panic!("comment: {e}"));
    r.svc.flush_outbox().await.unwrap_or_else(|e| panic!("flush: {e}"));
    let before = gets(&w);
    w.pump(&[&owner]).await;
    assert_eq!(thread_texts(&owner, &cid, &post).await, ["by the newcomer"], "placed by the keeper's rows");
    assert_eq!(pending(&owner, &cid).await, 0);
    assert_eq!(gets(&w), before, "nothing fetched on the keeper's device");
    assert_eq!(snapshots(&w).len(), made, "no snapshot was needed");
    assert!(matches!(standing(&owner, &cid, &r, joined + 1).await, Some(Standing::Known(Some(Reader { since }))) if since == joined));
    // A member waits for the snapshot that names the newcomer.
    w.pump(&[&a]).await;
    assert!(thread_texts(&a, &cid, &post).await.is_empty());
    assert_eq!(pending(&a, &cid).await, 1);
    // It comes `SETTLE_SECS` after the join: the keeper's turn wakes for it.
    w.clock.advance(crate::keeper::SETTLE_SECS - 2);
    w.pump(&all).await;
    assert_eq!(snapshots(&w).len(), made, "not before the settle is over");
    w.clock.advance(1);
    w.pump(&all).await;
    let last = snapshots(&w).last().cloned().unwrap_or_else(|| panic!("no snapshot"));
    assert_eq!((snapshots(&w).len(), last.members), (made + 1, 3), "the snapshot names the newcomer");
    assert_eq!(last.at, joined + crate::keeper::SETTLE_SECS);
    assert_eq!(thread_texts(&a, &cid, &post).await, ["by the newcomer"], "the member places the comment by it");
    assert_eq!(pending(&a, &cid).await, 0);
    for n in all {
        assert!(n.errors().is_empty(), "{:?}", n.errors());
    }
}

/// A burst of joins makes one snapshot, `SETTLE_SECS` after the first.
#[tokio::test]
async fn a_burst_of_joins_makes_one_snapshot() {
    let w = World::new();
    let owner = w.node().await;
    let (a, b, c) = (w.node().await, w.node().await, w.node().await);
    let all = [&owner, &a, &b, &c];
    let (cid, _) = channel_with_a_reader(&w, &owner, &a, &all).await;
    let made = snapshots(&w).len();
    let first = w.clock.now();
    join_by_link(&w, &owner, &b, &cid, &all).await;
    w.clock.advance(10);
    join_by_link(&w, &owner, &c, &cid, &all).await;
    assert_eq!(snapshots(&w).len(), made);
    w.clock.advance(crate::keeper::SETTLE_SECS - 10);
    w.pump(&all).await;
    let s = snapshots(&w);
    assert_eq!(s.len(), made + 1, "one snapshot for both");
    assert_eq!(s.last().map(|s| (s.members, s.at)), Some((4, first + crate::keeper::SETTLE_SECS)));
}

/// A big channel keeps the floor of `MIN_INTERVAL_SECS`.
#[tokio::test]
async fn a_big_channel_keeps_five_minutes_between_snapshots() {
    let w = World::new();
    let owner = w.node().await;
    let (a, r) = (w.node().await, w.node().await);
    let all = [&owner, &a, &r];
    let (cid, post) = channel_with_a_reader(&w, &owner, &a, &all).await;
    let last_at = snapshots(&w).last().map_or(0, |s| s.at);
    let made = snapshots(&w).len();
    // As if the channel had a thousand readers and more.
    let k = owner.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("no keeper"));
    k.lock().await.limits.small_channel = 0;
    join_by_link(&w, &owner, &r, &cid, &all).await;
    w.clock.advance(1);
    r.svc.send_comment(&r.keys, &cid, &post, None, "by the newcomer", TextFormat::Plain).await.unwrap_or_else(|e| panic!("comment: {e}"));
    w.pump(&all).await;
    assert_eq!(thread_texts(&owner, &cid, &post).await, ["by the newcomer"], "the keeper's device needs no snapshot");
    w.clock.advance(2 * crate::keeper::SETTLE_SECS);
    w.pump(&all).await;
    assert_eq!(snapshots(&w).len(), made, "no snapshot before the floor");
    assert!(thread_texts(&a, &cid, &post).await.is_empty());
    let now = w.clock.now();
    w.clock.advance(last_at + crate::snapshot::MIN_INTERVAL_SECS - now);
    w.pump(&all).await;
    assert_eq!(snapshots(&w).last().map(|s| (s.members, s.at)), Some((3, last_at + crate::snapshot::MIN_INTERVAL_SECS)));
    assert_eq!(thread_texts(&a, &cid, &post).await, ["by the newcomer"]);
}

/// A reader the keeper put out (a removal, a leave) is no reader on its
/// device at once, though the newest snapshot still names them; a member
/// goes by that snapshot until the next.
#[tokio::test]
async fn a_reader_put_out_is_no_reader_on_the_keepers_device_before_any_snapshot() {
    let w = World::new();
    let owner = w.node().await;
    let (a, b, c) = (w.node().await, w.node().await, w.node().await);
    let all = [&owner, &a, &b, &c];
    let (cid, post) = channel_with_a_reader(&w, &owner, &a, &all).await;
    join_by_link(&w, &owner, &b, &cid, &all).await;
    join_by_link(&w, &owner, &c, &cid, &all).await;
    w.clock.advance(crate::keeper::SETTLE_SECS);
    w.pump(&all).await;
    let made = snapshots(&w).len();
    assert_eq!(snapshots(&w).last().map(|s| s.members), Some(4));
    // b leaves: only the keeper knows (no operation of the journal).
    b.svc.leave(&b.keys, &cid).await.unwrap_or_else(|e| panic!("leave: {e}"));
    w.pump(&[&b, &owner]).await;
    let now = w.clock.now();
    assert_eq!(standing(&owner, &cid, &b, now + 1).await, Some(Standing::Known(None)), "out by the keeper's rows");
    assert!(matches!(standing(&c, &cid, &b, now + 1).await, Some(Standing::Known(Some(_)))), "the member goes by its snapshot");
    // a is removed and writes before hearing it: refused on the keeper's device.
    owner.svc.act(&owner.keys, &cid, OpBody::Remove { who: a.me.clone() }).await.unwrap_or_else(|e| panic!("remove: {e}"));
    w.pump(&[&owner]).await;
    assert_eq!(standing(&owner, &cid, &a, now + 2).await, Some(Standing::Known(None)));
    w.clock.advance(1);
    a.svc.send_comment(&a.keys, &cid, &post, None, "after the removal", TextFormat::Plain).await.unwrap_or_else(|e| panic!("comment: {e}"));
    a.svc.flush_outbox().await.unwrap_or_else(|e| panic!("flush: {e}"));
    w.pump(&[&owner]).await;
    assert!(thread_texts(&owner, &cid, &post).await.is_empty());
    assert_eq!(pending(&owner, &cid).await, 0, "refused, not parked");
    assert_eq!(snapshots(&w).len(), made, "no snapshot was needed");
}

/// The hosts are down when a member needs the shard that names a new
/// reader: the comment waits, and is read again once the hosts give the
/// shard, with no new snapshot.
#[tokio::test]
async fn a_comment_that_waited_for_down_hosts_is_read_again_when_they_are_back() {
    let w = World::new();
    let owner = w.node().await;
    let (a, r) = (w.node().await, w.node().await);
    let all = [&owner, &a, &r];
    let (cid, post) = channel_with_a_reader(&w, &owner, &a, &all).await;
    join_by_link(&w, &owner, &r, &cid, &all).await;
    w.clock.advance(crate::keeper::SETTLE_SECS);
    w.pump(&all).await;
    let made = snapshots(&w).len();
    assert_eq!(snapshots(&w).last().map(|s| s.members), Some(3));
    w.hosts.set_down(H1, true);
    w.hosts.set_down(H2, true);
    w.clock.advance(1);
    r.svc.send_comment(&r.keys, &cid, &post, None, "by the newcomer", TextFormat::Plain).await.unwrap_or_else(|e| panic!("comment: {e}"));
    r.svc.flush_outbox().await.unwrap_or_else(|e| panic!("flush: {e}"));
    w.pump(&[&a]).await;
    assert!(thread_texts(&a, &cid, &post).await.is_empty(), "the shard is not to be had");
    assert_eq!(pending(&a, &cid).await, 1);
    // Down still at the next try: it waits again, nothing lost.
    w.clock.advance(crate::member::fetch::RETRY_FIRST_SECS);
    w.pump(&[&a]).await;
    assert_eq!(pending(&a, &cid).await, 1);
    let tried = gets(&w);
    // Back: the next try (the wait doubled) opens the shard and places
    // the comment.
    w.hosts.set_down(H1, false);
    w.hosts.set_down(H2, false);
    w.clock.advance(2 * crate::member::fetch::RETRY_FIRST_SECS - 1);
    w.pump(&[&a]).await;
    assert_eq!(gets(&w), tried, "not before its time");
    assert_eq!(pending(&a, &cid).await, 1);
    w.clock.advance(1);
    w.pump(&[&a]).await;
    assert_eq!(thread_texts(&a, &cid, &post).await, ["by the newcomer"]);
    assert_eq!(pending(&a, &cid).await, 0);
    assert_eq!(snapshots(&w).len(), made, "no new snapshot");
    assert!(a.errors().is_empty(), "{:?}", a.errors());
}

/// The comment of a reader whose shard of the snapshot is not here waits
/// with no round trip on the way of its event (живой запуск 2026-10-23: a
/// shard fetched inline held the loop for tens of seconds): a job fetches
/// the shard off it, tried again at its time while the hosts lack it, and
/// places the comment once they give it, with no new snapshot.
#[tokio::test]
async fn a_comment_whose_shard_is_not_here_waits_with_no_get_and_the_fetch_places_it() {
    let w = World::new();
    let owner = w.node().await;
    let (a, r) = (w.node().await, w.node().await);
    let all = [&owner, &a, &r];
    let (cid, post) = channel_with_a_reader(&w, &owner, &a, &all).await;
    join_by_link(&w, &owner, &r, &cid, &all).await;
    w.clock.advance(crate::keeper::SETTLE_SECS);
    w.pump(&all).await;
    let made = snapshots(&w).len();
    assert_eq!(snapshots(&w).last().map(|s| s.members), Some(3));
    w.hosts.set_down(H1, true);
    w.hosts.set_down(H2, true);
    w.clock.advance(1);
    r.svc.send_comment(&r.keys, &cid, &post, None, "by the newcomer", TextFormat::Plain).await.unwrap_or_else(|e| panic!("comment: {e}"));
    r.svc.flush_outbox().await.unwrap_or_else(|e| panic!("flush: {e}"));
    let comment = w.relay.of_kind(crate::wire::KIND_CONTENT).last().cloned().unwrap_or_else(|| panic!("no comment"));
    let before = gets(&w);
    a.svc.on_event(&a.keys, &comment, true).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(gets(&w), before, "no round trip on the way of the event");
    assert_eq!(pending(&a, &cid).await, 1, "parked");
    assert!(a.svc.fetches_wanted().iter().any(|k| k.contains(":fetch:shard:")), "{:?}", a.svc.fetches_wanted());
    // The job: the hosts lack it, it waits for its time.
    assert_eq!(a.svc.run_fetches(&a.keys).await, 1);
    assert!(gets(&w) > before);
    assert_eq!(pending(&a, &cid).await, 1);
    assert_eq!(a.svc.run_fetches(&a.keys).await, 0, "not before its time");
    // The hosts give it: the next run places the comment.
    w.hosts.set_down(H1, false);
    w.hosts.set_down(H2, false);
    w.clock.advance(crate::member::fetch::RETRY_FIRST_SECS);
    assert_eq!(a.svc.run_fetches(&a.keys).await, 1);
    assert_eq!(thread_texts(&a, &cid, &post).await, ["by the newcomer"]);
    assert_eq!(pending(&a, &cid).await, 0);
    assert!(a.svc.fetches_wanted().is_empty());
    assert_eq!(snapshots(&w).len(), made, "no new snapshot");
    assert!(a.errors().is_empty(), "{:?}", a.errors());
}

/// The team reads the list of the readers (the owner's decision,
/// 2026-10-25): the keeper's device by its rows, an admin on another
/// device by the shards of the snapshot, a shard not here fetched by a job
/// the ask wants (the list says it is partial till then); the staff and I
/// are not in it, and a reader may not read it.
#[tokio::test]
async fn the_team_lists_the_readers_by_the_keepers_rows_and_by_the_snapshot() {
    let w = World::new();
    let owner = w.node().await;
    let (a, admin, b) = (w.node().await, w.node().await, w.node().await);
    let all = [&owner, &a, &admin, &b];
    let (cid, _) = channel_with_a_reader(&w, &owner, &a, &all).await;
    join_by_link(&w, &owner, &admin, &cid, &all).await;
    owner.svc.act(&owner.keys, &cid, OpBody::SetRole { who: admin.me.clone(), role: crate::roles::Role::Admin }).await.unwrap_or_else(|e| panic!("role: {e}"));
    w.pump(&all).await;
    w.clock.advance(5);
    join_by_link(&w, &owner, &b, &cid, &all).await;
    let keys = |p: &crate::reader_list::ReadersPage| p.readers.iter().map(|r| r.pubkey.clone()).collect::<Vec<_>>();
    let want = vec![b.me.as_hex().to_string(), a.me.as_hex().to_string()];
    // The keeper's device: at once, by its rows.
    let page = owner.svc.readers(&owner.keys, &cid, None, 0, 100).await.unwrap_or_else(|e| panic!("readers: {e}"));
    assert_eq!((keys(&page), page.total, page.partial), (want.clone(), 2, false));
    // The admin's device: by the snapshot that names b.
    w.clock.advance(crate::keeper::SETTLE_SECS);
    w.pump(&all).await;
    assert_eq!(snapshots(&w).last().map(|s| s.members), Some(4));
    let before = gets(&w);
    let first = admin.svc.readers(&admin.keys, &cid, None, 0, 100).await.unwrap_or_else(|e| panic!("readers: {e}"));
    assert_eq!(gets(&w), before, "no round trip on the ask");
    assert!(first.partial && first.readers.is_empty(), "the shard is not here yet: {first:?}");
    assert!(admin.svc.fetches_wanted().iter().any(|k| k.contains(":fetch:shard:")), "{:?}", admin.svc.fetches_wanted());
    admin.svc.run_fetches(&admin.keys).await;
    let page = admin.svc.readers(&admin.keys, &cid, None, 0, 100).await.unwrap_or_else(|e| panic!("readers: {e}"));
    assert_eq!((keys(&page), page.total, page.partial), (want, 2, false));
    let one = admin.svc.readers(&admin.keys, &cid, Some(&a.me.as_hex()[..8]), 0, 100).await.unwrap_or_else(|e| panic!("readers: {e}"));
    assert_eq!(keys(&one), [a.me.as_hex().to_string()]);
    // A reader may not.
    assert!(a.svc.readers(&a.keys, &cid, None, 0, 100).await.is_err());
    for n in all {
        assert!(n.errors().is_empty(), "{:?}", n.errors());
    }
}
