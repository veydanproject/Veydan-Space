// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The member's fetches off the way of an event (живой запуск 2026-10-23:
//! «an event of kind 1059 holds the loop for 34 s», every minute, while an
//! epoch or a shard of a snapshot came from slow hosts inline): an epoch
//! whose key opens by a shard is kept without it and wanted of the hosts
//! (`member::fetch`), the job runs with no lock held, and what it opens is
//! read then; a key that comes another way meanwhile ends the job.

use crate::db;
use crate::op::{History, JoinMode, OpBody};
use crate::service::{NewChannel, UI_EVENT_CHANNEL_MESSAGE};
use crate::state::SCOPE_EVERYONE;
use crate::testkit::{Node, World, H1, H2};
use crate::wire::{self, KIND_CONTENT, KIND_EPOCH};
use messenger_core::TextFormat;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

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

async fn has_key(n: &Node, cid: &str, epoch: u32) -> Option<bool> {
    let rows = db::keys::epochs(n.svc.db(), cid, SCOPE_EVERYONE).await.unwrap_or_default();
    rows.iter().find(|r| r.epoch == i64::from(epoch)).map(|r| r.key_hex.is_some())
}

async fn pending(n: &Node, cid: &str) -> i64 {
    db::channels::count_pending(n.svc.db(), cid).await.unwrap_or(-1)
}

fn gets(w: &World) -> usize {
    w.hosts.gets.load(Ordering::SeqCst)
}

/// Every get of the hosts waits from now on, until the gate is closed.
fn hang_gets(w: &World) -> Arc<tokio::sync::Semaphore> {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *w.hosts.get_gate.lock().unwrap_or_else(|e| e.into_inner()) = Some(gate.clone());
    gate
}

fn answer_gets(w: &World, gate: &tokio::sync::Semaphore) {
    *w.hosts.get_gate.lock().unwrap_or_else(|e| e.into_inner()) = None;
    gate.close();
}

/// A channel with a reader, and a new epoch the reader has not heard of
/// yet, out on the relay while every get of the hosts hangs: the epoch,
/// and its event.
async fn new_epoch_while_the_hosts_hang(w: &World, owner: &Node, reader: &Node) -> (String, u32, serde_json::Value, Arc<tokio::sync::Semaphore>) {
    let (cid, _) = owner.svc.create(&owner.keys, new_channel()).await.unwrap_or_else(|e| panic!("create: {e}"));
    w.pump(&[owner]).await;
    let link = owner.svc.share_link(&cid).await.unwrap_or_else(|e| panic!("link: {e}"));
    w.pump(&[reader]).await;
    reader.svc.open_link(&reader.keys, &link, "").await.unwrap_or_else(|e| panic!("open link: {e}"));
    w.pump(&[owner, reader]).await;
    let gate = hang_gets(w);
    owner.svc.act(&owner.keys, &cid, OpBody::RotateNow { scope_id: None }).await.unwrap_or_else(|e| panic!("rotate: {e}"));
    // The keeper puts (the gate holds gets only) and publishes.
    timeout(Duration::from_secs(10), w.pump(&[owner])).await.unwrap_or_else(|_| panic!("the keeper fetched from the hosts"));
    let epoch = *w.relay.epochs().last().unwrap_or(&0);
    assert!(epoch >= 2, "a new epoch: {:?}", w.relay.epochs());
    let ev = w.relay.of_kind(KIND_EPOCH).last().cloned().unwrap_or_else(|| panic!("no epoch event"));
    (cid, epoch, ev, gate)
}

/// An epoch event while the hosts hang: taken at once, its row kept
/// without the key and the post under it parked; the job that fetches its
/// shard holds no lock while it waits, and once the hosts answer the
/// epoch opens and the post is read, for the page.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_epoch_whose_hosts_hang_holds_no_event_and_opens_once_they_answer() {
    let w = World::new();
    let (owner, reader) = (w.node().await, w.node().await);
    let (cid, epoch, ev, gate) = new_epoch_while_the_hosts_hang(&w, &owner, &reader).await;
    let before = gets(&w);
    timeout(Duration::from_secs(1), reader.svc.on_event(&reader.keys, &ev, true))
        .await
        .unwrap_or_else(|_| panic!("the epoch event waited for the hosts"))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(has_key(&reader, &cid, epoch).await, Some(false), "the row is kept, without its key");
    assert_eq!(gets(&w), before, "nothing fetched on the way of the event");
    assert_eq!(reader.svc.fetches_wanted(), [format!("{cid}:fetch:epoch:{SCOPE_EVERYONE}:{epoch}")]);
    // A post under the new epoch: parked until its key opens.
    w.clock.advance(1);
    owner.svc.send_post(&owner.keys, &cid, "under the new epoch", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    owner.svc.flush_outbox().await.unwrap_or_else(|e| panic!("flush: {e}"));
    let post = w.relay.of_kind(KIND_CONTENT).last().cloned().unwrap_or_else(|| panic!("no post"));
    timeout(Duration::from_secs(1), reader.svc.on_event(&reader.keys, &post, true)).await.unwrap_or_else(|_| panic!("held")).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(pending(&reader, &cid).await, 1);
    // The job: it waits for the hosts with no lock held.
    let jobs = reader.svc.take_fetches();
    assert_eq!(jobs.len(), 1);
    let (svc, keys, job) = (reader.svc.clone(), reader.keys.clone(), jobs[0].clone());
    let running = tokio::spawn(async move { svc.run_fetch(&keys, job).await });
    timeout(Duration::from_secs(5), async {
        while gets(&w) == before {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the job never asked the hosts"));
    assert!(!running.is_finished(), "the hosts hang");
    assert!(reader.svc.lock_of(&cid).await.try_lock().is_ok(), "the channel's lock is free while the hosts hang");
    timeout(Duration::from_secs(1), reader.svc.on_event(&reader.keys, &ev, true)).await.unwrap_or_else(|_| panic!("held")).unwrap_or_else(|e| panic!("{e}"));
    assert!(reader.svc.take_fetches().is_empty(), "one job of a kind at a time");
    // The hosts answer: the epoch opens, the post is read for the page.
    answer_gets(&w, &gate);
    let out = timeout(Duration::from_secs(5), running).await.unwrap_or_else(|_| panic!("the job never ended")).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(has_key(&reader, &cid, epoch).await, Some(true));
    assert_eq!(pending(&reader, &cid).await, 0);
    assert!(out.events.iter().any(|e| e.name == UI_EVENT_CHANNEL_MESSAGE), "the post to the page: {:?}", out.events);
    let posts: Vec<String> = db::content::posts(reader.svc.db(), &cid, None, 10).await.unwrap_or_default().into_iter().filter_map(|p| p.text).collect();
    assert_eq!(posts, ["under the new epoch"]);
    assert!(reader.svc.fetches_wanted().is_empty());
    assert!(reader.errors().is_empty(), "{:?}", reader.errors());
}

/// The key of an epoch whose fetch waits for the hosts comes by an issue
/// of the keeper: the fetch ends with nothing to open (no second open, no
/// error), and nothing is wanted of the hosts any more.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_key_that_comes_by_an_issue_ends_the_fetch_of_its_epoch() {
    let w = World::new();
    let (owner, reader) = (w.node().await, w.node().await);
    let (cid, epoch, ev, gate) = new_epoch_while_the_hosts_hang(&w, &owner, &reader).await;
    reader.svc.on_event(&reader.keys, &ev, true).await.unwrap_or_else(|e| panic!("{e}"));
    let before = gets(&w);
    let job = reader.svc.take_fetches().pop().unwrap_or_else(|| panic!("no fetch"));
    let (svc, keys) = (reader.svc.clone(), reader.keys.clone());
    let running = tokio::spawn(async move { svc.run_fetch(&keys, job).await });
    timeout(Duration::from_secs(5), async {
        while gets(&w) == before {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the job never asked the hosts"));
    // The issue: the key of the new epoch, under the reader's bundle.
    let key = owner.svc.member(&owner.keys).newest_key(&cid, SCOPE_EVERYONE).await.ok().flatten().unwrap_or_else(|| panic!("the owner's key"));
    assert_eq!(key.epoch, epoch);
    let (_, bytes, _) = db::keys::bundles(reader.svc.db(), &cid, SCOPE_EVERYONE).await.unwrap_or_default().pop().unwrap_or_else(|| panic!("no bundle"));
    let bundle = keydist::Bundle::from_bytes(&bytes).unwrap_or_else(|e| panic!("{e}"));
    let issued = keydist::Issued {
        scheme: bundle.scheme,
        scope: bundle.scope,
        gen: bundle.gen,
        leaf: bundle.leaf,
        kind: keydist::GenerationKind::Initial,
        bundle: keydist::SecretBytes::default(),
        current: None,
        missed: vec![key.clone()],
        missed_more: false,
    };
    let log = reader.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("log"));
    let state = log.state();
    let issue = wire::Issue {
        community_id: cid.clone(),
        relays: state.relays.clone(),
        storage: state.storage.clone(),
        heads: log.heads(),
        link: None,
        issued: wire::Issue::encode_issued(&issued),
    };
    let keeper = state.keeper().map(|k| k.pubkey.clone()).unwrap_or_else(|| panic!("no keeper"));
    let m = reader.svc.member(&reader.keys).on_issue(&log, &keeper, &issue, reader.svc.now()).await.unwrap_or_else(|e| panic!("issue: {e}"));
    assert_eq!(m.keys.iter().map(|k| k.epoch).collect::<Vec<_>>(), [epoch], "the key by the issue");
    assert!(reader.svc.fetches_wanted().is_empty(), "the fetch of its epoch is forgotten");
    // The hosts answer: the job finds the key held and opens nothing.
    answer_gets(&w, &gate);
    let out = timeout(Duration::from_secs(5), running).await.unwrap_or_else(|_| panic!("the job never ended")).unwrap_or_else(|e| panic!("{e}"));
    assert!(out.notes.iter().all(|n| !n.contains("opened by its shard")), "{:?}", out.notes);
    assert!(out.events.is_empty(), "{:?}", out.events);
    assert!(reader.svc.fetches_wanted().is_empty(), "nothing wanted again");
    assert_eq!(reader.svc.member(&reader.keys).newest_key(&cid, SCOPE_EVERYONE).await.ok().flatten(), Some(key));
    // Nothing more of the hosts at the next turns.
    let after = gets(&w);
    w.clock.advance(crate::member::fetch::RETRY_MAX_SECS);
    w.pump(&[&reader]).await;
    assert_eq!(gets(&w), after);
    assert!(reader.errors().is_empty(), "{:?}", reader.errors());
}

// ─── An epoch in Merkle mode (ROO-395) ──────────────────────────────────────

/// The keeper of a channel past `INDEX_LEAVES` leaves handed out: the
/// leaves of readers who came and went, never handed out again.
async fn past_the_wall(owner: &Node, cid: &str, n: u32) {
    let k = owner.svc.keeper(cid).await.ok().flatten().unwrap_or_else(|| panic!("not kept here"));
    k.lock().await.hand_out_leaves(SCOPE_EVERYONE, n).await.unwrap_or_else(|e| panic!("leaves: {e}"));
}

async fn join(w: &World, owner: &Node, reader: &Node, cid: &str) {
    let link = owner.svc.share_link(cid).await.unwrap_or_else(|e| panic!("link: {e}"));
    w.pump(&[reader]).await;
    reader.svc.open_link(&reader.keys, &link, "").await.unwrap_or_else(|e| panic!("open link: {e}"));
    w.pump(&[owner, reader]).await;
}

async fn rotate(w: &World, owner: &Node, cid: &str) -> u32 {
    let before = w.relay.epochs().last().copied().unwrap_or(0);
    w.clock.advance(1);
    owner.svc.act(&owner.keys, cid, OpBody::RotateNow { scope_id: None }).await.unwrap_or_else(|e| panic!("rotate: {e}"));
    w.pump(&[owner]).await;
    let epoch = w.relay.epochs().last().copied().unwrap_or(0);
    assert!(epoch > before, "a new epoch: {:?}", w.relay.epochs());
    epoch
}

fn path(cid: &str, kind: keydist::BlobKind, epoch: u32, no: u32) -> String {
    crate::blobs::key_path(cid, SCOPE_EVERYONE, kind, 1, epoch, no).unwrap_or_else(|| panic!("a path"))
}

/// A channel whose generation handed out `INDEX_MAX + 1` shards of leaves
/// (ROO-396, 1.1: a channel of 65 536 readers that come and go gets there
/// too): its epoch names a Merkle root and no index blob, the keeper puts
/// every proof and shard under its path, and a reader of the first shard
/// and one of the last (an odd node, promoted to the root unchanged) each
/// open it by the proof and the shard of their leaf. On 25ebae4c7 no reader
/// opened it: `plan_epoch` wanted an index blob.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reader_opens_an_epoch_of_more_shards_than_an_index_holds() {
    let w = World::new();
    let (owner, first, last) = (w.node().await, w.node().await, w.node().await);
    let (cid, _) = owner.svc.create(&owner.keys, new_channel()).await.unwrap_or_else(|e| panic!("create: {e}"));
    w.pump(&[&owner]).await;
    join(&w, &owner, &first, &cid).await;
    past_the_wall(&owner, &cid, u32::try_from(crate::keeper::INDEX_LEAVES).unwrap_or(u32::MAX)).await;
    join(&w, &owner, &last, &cid).await;
    assert!(last.svc.member(&last.keys).has_bundle(&cid, SCOPE_EVERYONE).await.unwrap_or(false), "let in: the hosts serve paths");
    let epoch = rotate(&w, &owner, &cid).await;
    let shards = keydist::shard::INDEX_MAX + 1;
    for no in [0, shards - 1] {
        for kind in [keydist::BlobKind::Proof, keydist::BlobKind::Shard] {
            assert!(w.hosts.at_path(H1, &path(&cid, kind, epoch, no)).is_some(), "{kind:?} {no} of epoch {epoch} under its path");
        }
    }
    assert!(w.hosts.at_path(H1, &path(&cid, keydist::BlobKind::Shard, epoch, shards)).is_none(), "{shards} shards, no more");
    w.pump(&[&first, &last]).await;
    assert_eq!(has_key(&first, &cid, epoch).await, Some(true), "the reader of shard 0");
    assert_eq!(has_key(&last, &cid, epoch).await, Some(true), "the reader of shard {}", shards - 1);
    // A post under it is read.
    owner.svc.send_post(&owner.keys, &cid, "past the wall", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    owner.svc.flush_outbox().await.unwrap_or_else(|e| panic!("flush: {e}"));
    w.pump(&[&last]).await;
    let posts: Vec<String> = db::content::posts(last.svc.db(), &cid, None, 10).await.unwrap_or_default().into_iter().filter_map(|p| p.text).collect();
    assert_eq!(posts, ["past the wall"]);
    for n in [&owner, &first, &last] {
        assert!(n.errors().is_empty(), "{:?}", n.errors());
    }
}

/// A path is no proof: a host that gives another proof for it is passed
/// over like one that is down, and when every host does the reader opens
/// nothing, `Invalid("shard proof")`, as a blob of a wrong hash.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_proof_a_host_changed_opens_nothing() {
    let w = World::new();
    let (owner, reader) = (w.node().await, w.node().await);
    let (cid, _) = owner.svc.create(&owner.keys, new_channel()).await.unwrap_or_else(|e| panic!("create: {e}"));
    w.pump(&[&owner]).await;
    join(&w, &owner, &reader, &cid).await;
    past_the_wall(&owner, &cid, u32::try_from(crate::keeper::INDEX_LEAVES).unwrap_or(u32::MAX)).await;
    let epoch = rotate(&w, &owner, &cid).await;
    let ev = w.relay.of_kind(KIND_EPOCH).last().cloned().unwrap_or_else(|| panic!("no epoch event"));
    reader.svc.on_event(&reader.keys, &ev, true).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(has_key(&reader, &cid, epoch).await, Some(false), "wanted of the hosts");
    // The proof of shard 0 with one hash of its path changed, on H1.
    let proof_path = path(&cid, keydist::BlobKind::Proof, epoch, 0);
    let forge = |host: &str| {
        let sha = w.hosts.at_path(host, &proof_path).unwrap_or_else(|| panic!("a proof on {host}"));
        let mut bytes = w.hosts.blobs.lock().unwrap_or_else(|e| e.into_inner())[host][&sha].clone();
        let at = bytes.len() - 1;
        bytes[at] ^= 1;
        w.hosts.tamper(host, &sha, bytes);
    };
    forge(H1);
    let log = reader.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("log"));
    let member = reader.svc.member(&reader.keys);
    let plan = member.plan_epoch(log.state(), SCOPE_EVERYONE, epoch).await.ok().flatten().unwrap_or_else(|| panic!("a plan"));
    assert!(matches!(plan, crate::member::fetch::Plan::Proved { .. }), "{plan:?}");
    assert!(crate::member::fetch::download(w.hosts.as_ref(), &plan).await.is_ok(), "H2 gives what the root proves");
    // Both hosts lie: nothing opens.
    forge(H2);
    let err = crate::member::fetch::download(w.hosts.as_ref(), &plan).await.err().unwrap_or_else(|| panic!("a forged proof taken"));
    assert!(matches!(&err, messenger_core::MessengerError::Invalid(m) if m.contains("shard proof")), "{err}");
    w.pump(&[&reader]).await;
    assert_eq!(has_key(&reader, &cid, epoch).await, Some(false), "no key by a forged proof");
    assert!(reader.svc.fetches_wanted().contains(&format!("{cid}:fetch:epoch:{SCOPE_EVERYONE}:{epoch}")), "tried again later");
}

/// The leaves the current generation of `everyone` handed out.
async fn handed(owner: &Node, cid: &str) -> u32 {
    let k = owner.svc.keeper(cid).await.ok().flatten().unwrap_or_else(|| panic!("not kept here"));
    let k = k.lock().await;
    k.scope(SCOPE_EVERYONE).map_or(0, |s| s.generation().next_leaf)
}

fn index_leaves() -> u32 {
    u32::try_from(crate::keeper::INDEX_LEAVES).unwrap_or(u32::MAX)
}

/// Hosts that serve no paths (a plain Blossom host), and a tree past the
/// wall all the same (a leaf handed out another way, the hosts of the
/// channel changed): its epoch would name a root no host serves, so the
/// next turn makes an emergency generation of the owner's root, whose
/// leaves are handed out again to the members alone. A ban goes out with
/// it, the one banned opens nothing after it, the readers read on, and a
/// newcomer is let in. On a26b92633 the epoch past the wall was taken by
/// no host and stayed pending for good, the ban behind it, while the posts
/// went on under the key the one banned held (the review of ROO-412).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hosts_without_paths_get_a_new_generation_at_the_wall() {
    let w = World::new();
    let (owner, reader, banned, late) = (w.node().await, w.node().await, w.node().await, w.node().await);
    w.hosts.set_paths(H1, false);
    w.hosts.set_paths(H2, false);
    let (cid, _) = owner.svc.create(&owner.keys, new_channel()).await.unwrap_or_else(|e| panic!("create: {e}"));
    w.pump(&[&owner]).await;
    join(&w, &owner, &reader, &cid).await;
    join(&w, &owner, &banned, &cid).await;
    let before = rotate(&w, &owner, &cid).await;
    w.pump(&[&reader, &banned]).await;
    assert_eq!(has_key(&banned, &cid, before).await, Some(true), "a reader before the ban");
    let to_wall = index_leaves() - handed(&owner, &cid).await;
    past_the_wall(&owner, &cid, to_wall + 1).await;
    w.clock.advance(1);
    owner.svc.act(&owner.keys, &cid, OpBody::Ban { who: banned.me.clone(), reason: None }).await.unwrap_or_else(|e| panic!("ban: {e}"));
    w.pump(&[&owner]).await;
    let epoch = rotate(&w, &owner, &cid).await;
    {
        let k = owner.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("kept"));
        let k = k.lock().await;
        let g = k.scope(SCOPE_EVERYONE).map(|s| s.generation().clone()).unwrap_or_else(|| panic!("a tree"));
        assert!(g.gen > 1 && g.next_leaf < 8, "a generation of the members alone: gen {}, {} leaves", g.gen, g.next_leaf);
    }
    w.pump(&[&reader, &banned]).await;
    assert_eq!(has_key(&reader, &cid, epoch).await, Some(true), "the reader reads on");
    assert_ne!(has_key(&banned, &cid, epoch).await, Some(true), "the one banned opens nothing after the ban");
    assert!(handed(&owner, &cid).await < index_leaves(), "off the wall");
    join(&w, &owner, &late, &cid).await;
    assert!(late.svc.member(&late.keys).has_bundle(&cid, SCOPE_EVERYONE).await.unwrap_or(false), "a newcomer let in");
    assert!(reader.errors().is_empty(), "{:?}", reader.errors());
}

/// A tree past the wall whose emergency generation waits behind a pending
/// epoch with no number left to burn (the hosts down through seventeen
/// windows, then hosts without paths, which never take a Merkle epoch): a
/// join is refused at once and says why in the keeper's block. On
/// 58240e3d9 it waited in the queue (`Busy`) at every turn, and every join
/// of the channel behind it, for good (the review of ROO-410).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_join_at_the_wall_behind_an_epoch_that_never_goes_is_refused() {
    let w = World::new();
    let (owner, reader, late) = (w.node().await, w.node().await, w.node().await);
    let (cid, _) = owner.svc.create(&owner.keys, new_channel()).await.unwrap_or_else(|e| panic!("create: {e}"));
    w.pump(&[&owner]).await;
    join(&w, &owner, &reader, &cid).await;
    let to_wall = index_leaves() - handed(&owner, &cid).await;
    past_the_wall(&owner, &cid, to_wall + 1).await;
    w.hosts.set_down(H1, true);
    w.hosts.set_down(H2, true);
    for _ in 0..17 {
        w.clock.advance(400);
        owner.svc.act(&owner.keys, &cid, OpBody::RotateNow { scope_id: None }).await.unwrap_or_else(|e| panic!("rotate now: {e}"));
        w.pump(&[&owner]).await;
    }
    w.hosts.set_paths(H1, false);
    w.hosts.set_paths(H2, false);
    w.hosts.set_down(H1, false);
    w.hosts.set_down(H2, false);
    w.clock.advance(400);
    w.pump(&[&owner]).await;
    let published = w.relay.epochs();
    {
        let k = owner.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("kept"));
        let mut k = k.lock().await;
        assert!(k.scope_keeper(SCOPE_EVERYONE).and_then(|s| s.pending_epoch()).is_some(), "a Merkle epoch no host takes: {published:?}");
        assert!(k.regeneration_held(SCOPE_EVERYONE).await.unwrap_or(false), "no number left to burn");
    }
    join(&w, &owner, &late, &cid).await;
    assert!(!late.svc.member(&late.keys).has_bundle(&cid, SCOPE_EVERYONE).await.unwrap_or(true), "nobody let in past the wall");
    assert_eq!(db::keeper::queued_count(owner.svc.db(), &cid).await.unwrap_or(-1), 0, "the join does not wait in the queue");
    let block = owner.svc.keeper_status(&cid).and_then(|s| s.storage_error).unwrap_or_default();
    assert!(block.contains("joins are refused"), "{block}");
    assert_eq!(w.relay.epochs(), published, "nothing went out");
}

/// Up to the wall the index holds, on hosts without paths: the readers
/// read the epoch of `INDEX_MAX` shards as before.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hosts_without_paths_read_up_to_the_wall() {
    let w = World::new();
    let (owner, reader) = (w.node().await, w.node().await);
    w.hosts.set_paths(H1, false);
    w.hosts.set_paths(H2, false);
    let (cid, _) = owner.svc.create(&owner.keys, new_channel()).await.unwrap_or_else(|e| panic!("create: {e}"));
    w.pump(&[&owner]).await;
    join(&w, &owner, &reader, &cid).await;
    let to_wall = index_leaves() - handed(&owner, &cid).await;
    past_the_wall(&owner, &cid, to_wall - 1).await;
    let epoch = rotate(&w, &owner, &cid).await;
    w.pump(&[&reader]).await;
    assert_eq!(has_key(&reader, &cid, epoch).await, Some(true), "{} shards, an index blob", keydist::shard::INDEX_MAX);
    assert_eq!(handed(&owner, &cid).await, index_leaves() - 1, "the same generation");
}

/// Every leaf handed out stops at the wall, not the first of a batch: two
/// tickets asked one leaf before it give one (the review of ROO-412).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tickets_stop_at_the_wall() {
    let w = World::new();
    let owner = w.node().await;
    w.hosts.set_paths(H1, false);
    w.hosts.set_paths(H2, false);
    let (cid, _) = owner.svc.create(&owner.keys, NewChannel { join: JoinMode::Request, ..new_channel() }).await.unwrap_or_else(|e| panic!("create: {e}"));
    w.pump(&[&owner]).await;
    let to_wall = index_leaves() - handed(&owner, &cid).await;
    past_the_wall(&owner, &cid, to_wall - 1).await;
    let log = owner.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("journal"));
    let k = owner.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("kept"));
    let req = wire::TicketRequest { community_id: cid.clone(), n: 2 };
    let n = k.lock().await.on_ticket_request(&log, &owner.me, &req, w.clock.now()).await.unwrap_or_else(|e| panic!("tickets: {e}"));
    assert_eq!(n, 1, "one leaf before the wall");
    assert_eq!(handed(&owner, &cid).await, index_leaves());
    let n = k.lock().await.on_ticket_request(&log, &owner.me, &req, w.clock.now()).await.unwrap_or_else(|e| panic!("tickets: {e}"));
    assert_eq!(n, 0, "none past it");
    assert_eq!(handed(&owner, &cid).await, index_leaves());
}

/// A bundle asked again at the wall: a member out of the tree (the owner,
/// a keeper recovered without them) gets no new leaf, a member in it the
/// same leaf again (the review of ROO-412).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rebundle_at_the_wall_hands_out_no_new_leaf() {
    let w = World::new();
    let (owner, reader) = (w.node().await, w.node().await);
    w.hosts.set_paths(H1, false);
    w.hosts.set_paths(H2, false);
    let (cid, _) = owner.svc.create(&owner.keys, new_channel()).await.unwrap_or_else(|e| panic!("create: {e}"));
    w.pump(&[&owner]).await;
    join(&w, &owner, &reader, &cid).await;
    let k = owner.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("kept"));
    k.lock().await.drop_leaf(&owner.me, w.clock.now()).await.unwrap_or_else(|e| panic!("drop: {e}"));
    let to_wall = index_leaves() - handed(&owner, &cid).await;
    past_the_wall(&owner, &cid, to_wall).await;
    let log = owner.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("journal"));
    let r = wire::Rebundle { community_id: cid.clone(), scope_id: SCOPE_EVERYONE.into(), gen: 1 };
    let sent = k.lock().await.on_rebundle(&log, &owner.me, &r, true, w.clock.now()).await.unwrap_or_else(|e| panic!("rebundle: {e}"));
    assert!(!sent, "no new leaf past the wall");
    assert!(!k.lock().await.is_member(&owner.me));
    assert_eq!(handed(&owner, &cid).await, index_leaves());
    let sent = k.lock().await.on_rebundle(&log, &reader.me, &r, true, w.clock.now()).await.unwrap_or_else(|e| panic!("rebundle: {e}"));
    assert!(sent, "the same leaf again");
    assert_eq!(handed(&owner, &cid).await, index_leaves());
}
