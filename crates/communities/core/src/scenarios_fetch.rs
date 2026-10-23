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
