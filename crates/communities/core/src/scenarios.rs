// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Scenarios of sub-stages 1.2 and 1.3 (internal/communities-stage1.md
//! §7): the keeper and the members over the fakes of `testkit`, a relay,
//! blob hosts with failures and a clock moved by hand; the snapshot of
//! the readers, the transport, media and moderation.

use crate::db;
use crate::keeper::Joined;
use crate::link::{ChannelLink, LinkSecret};
use crate::op::tests::Settings;
use crate::op::{History, JoinMode, OpBody, Premod};
use crate::transport::FeedPage;
use crate::wire::{Links, Place, KIND_SNAPSHOT};
use messenger_core::{Envelope, TextFormat};
use crate::roles::Role;
use crate::service::{NewChannel, MEMBERSHIP_JOINED, MEMBERSHIP_LEFT, MEMBERSHIP_NONE, MEMBERSHIP_REJECTED};
use crate::state::SCOPE_EVERYONE;
use crate::testkit::{Node, World, H1, H2, H3};
use crate::wire::{self, KIND_CONTENT, KIND_EPOCH};
use keydist::{EpochKey, KeyDistribution};
use messenger_core::PubKey;
use std::sync::atomic::Ordering;

fn new_channel(join: JoinMode, history: History, storage: &[&str]) -> NewChannel {
    NewChannel {
        name: "News".into(),
        about: String::new(),
        picture: String::new(),
        join,
        history,
        relays: vec!["wss://r1.example".into(), "wss://r2.example".into()],
        storage: storage.iter().map(|s| s.to_string()).collect(),
    }
}

async fn create(w: &World, owner: &Node, join: JoinMode, history: History, storage: &[&str]) -> String {
    let (cid, _) = owner.svc.create(&owner.keys, new_channel(join, history, storage)).await.unwrap_or_else(|e| panic!("create: {e}"));
    w.pump(&[owner]).await;
    cid
}

async fn newest(n: &Node, cid: &str) -> Option<EpochKey> {
    n.svc.member(&n.keys).newest_key(cid, SCOPE_EVERYONE).await.unwrap_or_else(|e| panic!("keys: {e}"))
}

async fn has_key(n: &Node, cid: &str, epoch: u32) -> bool {
    db::keys::epochs(n.svc.db(), cid, SCOPE_EVERYONE).await.unwrap_or_default().iter().any(|r| r.epoch == i64::from(epoch) && r.key_hex.is_some())
}

async fn membership(n: &Node, cid: &str) -> String {
    db::channels::get(n.svc.db(), cid).await.ok().flatten().map(|r| r.membership).unwrap_or_default()
}

/// A reader let in by the link the owner hands on.
async fn join_by_link(w: &World, owner: &Node, reader: &Node, cid: &str, all: &[&Node]) {
    let link = owner.svc.share_link(cid).await.unwrap_or_else(|e| panic!("link: {e}"));
    w.pump(&[reader]).await;
    reader.svc.open_link(&reader.keys, &link, "").await.unwrap_or_else(|e| panic!("open link: {e}"));
    w.pump(all).await;
}

async fn tick(n: &Node) {
    n.svc.tick().await.unwrap_or_else(|e| panic!("tick: {e}"));
}

/// What the device left in its outbox, sent (the module's outbox task).
async fn flush(n: &Node) -> crate::outbox::Flushed {
    n.svc.flush_outbox().await.unwrap_or_else(|e| panic!("flush: {e}"))
}

async fn keeper_state(n: &Node, cid: &str) -> keydist::KeeperState {
    let k = n.svc.keeper(cid).await.ok().flatten().unwrap_or_else(|| panic!("not kept here"));
    let k = k.lock().await;
    k.scope(SCOPE_EVERYONE).map(|s| s.state().clone()).unwrap_or_else(|| panic!("no scope"))
}

async fn post_texts(n: &Node, cid: &str) -> Vec<String> {
    db::content::posts(n.svc.db(), cid, None, 100).await.unwrap_or_default().into_iter().filter_map(|p| p.text).collect()
}

fn pk(n: &Node) -> PubKey {
    n.me.clone()
}

#[tokio::test]
async fn a_new_channel_publishes_its_first_epoch_once_two_hosts_hold_every_blob() {
    let w = World::new();
    let owner = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2, H3]).await;
    assert_eq!(w.relay.epochs(), vec![1], "the first epoch, at once");
    let ev = &w.relay.of_kind(KIND_EPOCH)[0];
    let content: serde_json::Value = serde_json::from_str(ev["content"].as_str().unwrap_or("")).unwrap_or_default();
    assert_eq!(content["blobs"], serde_json::json!([H1, H2, H3]));
    assert!(w.hosts.count(H1) > 0 && w.hosts.count(H1) == w.hosts.count(H2));
    // The owner is the first reader: the issue came back to the owner's
    // inbox and the key of epoch 1 opened.
    assert_eq!(newest(&owner, &cid).await.map(|k| k.epoch), Some(1));
    assert_eq!(membership(&owner, &cid).await, MEMBERSHIP_JOINED);
    // The secret of the link went into the feed under the epoch.
    assert_eq!(w.relay.of_kind(KIND_CONTENT).len(), 1);
    assert!(owner.errors().is_empty(), "{:?}", owner.errors());
}

#[tokio::test]
async fn one_host_up_is_enough_for_the_event_and_the_others_catch_up() {
    // Every host down: the event waits for one of them.
    let w = World::new();
    let owner = w.node().await;
    w.hosts.set_down(H1, true);
    w.hosts.set_down(H2, true);
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    assert!(w.relay.epochs().is_empty(), "no host holds the blobs: the event waits");
    tick(&owner).await;
    assert!(w.relay.epochs().is_empty());
    w.hosts.set_down(H1, false);
    tick(&owner).await;
    assert_eq!(w.relay.epochs(), vec![1], "one host holding everything is enough (owner, 2026-10-23)");
    w.pump(&[&owner]).await;
    assert_eq!(newest(&owner, &cid).await.map(|k| k.epoch), Some(1));
    assert_eq!(w.hosts.count(H2), 0, "the host still down holds nothing yet");
    w.hosts.set_down(H2, false);
    tick(&owner).await;
    assert_eq!(w.hosts.count(H2), w.hosts.count(H1), "the host that was down caught up");

    // Three hosts, one down: the event goes out, the third catches up later.
    let w = World::new();
    let owner = w.node().await;
    w.hosts.set_down(H2, true);
    create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2, H3]).await;
    assert_eq!(w.relay.epochs(), vec![1]);
    let ev = &w.relay.of_kind(KIND_EPOCH)[0];
    let content: serde_json::Value = serde_json::from_str(ev["content"].as_str().unwrap_or("")).unwrap_or_default();
    assert_eq!(content["blobs"], serde_json::json!([H1, H3]), "the event names the hosts that hold everything");
    assert_eq!(w.hosts.count(H2), 0);
    w.hosts.set_down(H2, false);
    tick(&owner).await;
    assert_eq!(w.hosts.count(H2), w.hosts.count(H1), "the host that lagged caught up");
}

#[tokio::test]
async fn a_crash_between_the_blobs_and_the_event_publishes_the_same_epoch_after_a_restart() {
    let w = World::new();
    let owner = w.node().await;
    w.relay.refuse(KIND_EPOCH, true);
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    assert!(w.relay.epochs().is_empty(), "the relays did not take the event");
    let puts = w.hosts.puts.load(Ordering::SeqCst);
    assert!(puts > 0);
    let before = keeper_state(&owner, &cid).await;
    assert_eq!(before.pending.as_ref().map(|p| p.key.epoch), Some(1));
    // The app goes down and comes back: the keeper is put together from
    // its rows, and the pending epoch is the same one.
    owner.svc.unload_keepers().await;
    assert_eq!(keeper_state(&owner, &cid).await, before);
    w.relay.refuse(KIND_EPOCH, false);
    // The event waits in the outbox for its time (the relay refused it once).
    w.clock.advance(crate::keeper::OWED_FIRST_WAIT_SECS);
    tick(&owner).await;
    assert_eq!(w.relay.epochs(), vec![1], "the same number");
    // The snapshot of the readers follows the epoch: one shard on each host.
    let shards = w.relay.of_kind(crate::wire::KIND_SNAPSHOT).len() * 2;
    assert_eq!(w.hosts.puts.load(Ordering::SeqCst), puts + shards, "no blob of the epoch put twice");
    w.pump(&[&owner]).await;
    assert_eq!(newest(&owner, &cid).await.map(|k| k.epoch), Some(1));
    assert_eq!(newest(&owner, &cid).await.map(|k| k.key), before.head.map(|h| h.key));

    // A crash in the middle of the upload: the next turn goes on from the
    // blobs the hosts hold.
    let w = World::new();
    let owner = w.node().await;
    *w.hosts.budget.lock().unwrap_or_else(|e| e.into_inner()) = Some(1);
    create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    assert!(w.relay.epochs().is_empty());
    owner.svc.unload_keepers().await;
    *w.hosts.budget.lock().unwrap_or_else(|e| e.into_inner()) = None;
    tick(&owner).await;
    assert_eq!(w.relay.epochs(), vec![1]);
}

#[tokio::test]
async fn the_keeper_comes_back_from_its_rows_as_it_was() {
    let w = World::new();
    let owner = w.node().await;
    let (a, b) = (w.node().await, w.node().await);
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &a, &cid, &[&owner, &a]).await;
    join_by_link(&w, &owner, &b, &cid, &[&owner, &a, &b]).await;
    owner.svc.act(&owner.keys, &cid, OpBody::Ban { who: pk(&a), reason: None }).await.unwrap_or_else(|e| panic!("ban: {e}"));
    let before = keeper_state(&owner, &cid).await;
    assert_eq!(before.members.len(), 2, "the owner and b");
    owner.svc.unload_keepers().await;
    assert_eq!(keeper_state(&owner, &cid).await, before);
}

#[tokio::test]
async fn joining_by_the_link_with_a_proof_a_wrong_proof_and_the_previous_link_epoch() {
    let w = World::new();
    let owner = w.node().await;
    let admin = w.node().await;
    let (bob, carol, dave, eve) = (w.node().await, w.node().await, w.node().await, w.node().await);
    let all = [&owner, &admin, &bob, &carol, &dave, &eve];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &bob, &cid, &all).await;
    assert_eq!(membership(&bob, &cid).await, MEMBERSHIP_JOINED);
    assert_eq!(newest(&bob, &cid).await.map(|k| k.epoch), Some(1), "the current key came with the issue");

    // A proof under a secret that is not the link's.
    let forged = ChannelLink::parse(&owner.svc.share_link(&cid).await.unwrap_or_default()).map(|mut l| {
        l.secret = Some(LinkSecret::from_bytes(&[9u8; 32]).unwrap_or_else(|e| panic!("{e}")));
        l.encode()
    });
    w.pump(&[&carol]).await;
    carol.svc.open_link(&carol.keys, &forged.unwrap_or_default(), "").await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&all).await;
    assert_eq!(membership(&carol, &cid).await, MEMBERSHIP_REJECTED);
    assert!(newest(&carol, &cid).await.is_none());

    // An admin turns the link over; its secret does not reach the keeper
    // (the feed is not heard there for now). One without a key of the
    // channel could not hand the secret on: refused before it is out.
    owner.svc.act(&owner.keys, &cid, OpBody::SetRole { who: pk(&admin), role: Role::Admin }).await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&all).await;
    assert!(admin.svc.act(&admin.keys, &cid, OpBody::RotateLink { link_epoch: 1 }).await.is_err());
    join_by_link(&w, &owner, &admin, &cid, &all).await;
    let old_link = owner.svc.share_link(&cid).await.unwrap_or_default();
    owner.hold(KIND_CONTENT, true);
    admin.svc.act(&admin.keys, &cid, OpBody::RotateLink { link_epoch: 1 }).await.unwrap_or_else(|e| panic!("rotate link: {e}"));
    w.pump(&all).await;
    // Within five minutes the previous link still lets in.
    w.clock.advance(120);
    w.pump(&[&dave]).await;
    dave.svc.open_link(&dave.keys, &old_link, "").await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&all).await;
    assert_eq!(membership(&dave, &cid).await, MEMBERSHIP_JOINED);
    // After them, no more.
    w.clock.advance(300);
    w.pump(&[&eve]).await;
    eve.svc.open_link(&eve.keys, &old_link, "").await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&all).await;
    assert_eq!(membership(&eve, &cid).await, MEMBERSHIP_REJECTED);
    // The secret comes: the new link lets in.
    owner.hold(KIND_CONTENT, false);
    w.pump(&all).await;
    let new_link = admin.svc.share_link(&cid).await.unwrap_or_default();
    assert!(ChannelLink::parse(&new_link).is_ok_and(|l| l.link_epoch == 1 && l.secret.is_some()));
    let frank = w.node().await;
    w.pump(&[&frank]).await;
    frank.svc.open_link(&frank.keys, &new_link, "").await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&[&owner, &frank]).await;
    assert_eq!(membership(&frank, &cid).await, MEMBERSHIP_JOINED);
}

#[tokio::test]
async fn an_admission_counts_from_a_moderator_and_an_invitation_once() {
    let w = World::new();
    let owner = w.node().await;
    let (moder, reader, joiner, other) = (w.node().await, w.node().await, w.node().await, w.node().await);
    let all = [&owner, &moder, &reader, &joiner, &other];
    let cid = create(&w, &owner, JoinMode::Request, History::Open, &[H1, H2]).await;
    owner.svc.act(&owner.keys, &cid, OpBody::SetRole { who: pk(&moder), role: Role::Moderator }).await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&all).await;

    // A request, answered by the moderator: an admission, then the join.
    let link = owner.svc.share_link(&cid).await.unwrap_or_default();
    joiner.svc.open_link(&joiner.keys, &link, "let me in").await.unwrap_or_else(|e| panic!("request: {e}"));
    w.pump(&all).await;
    assert_eq!(db::channels::requests(moder.svc.db(), &cid, "in", "pending").await.unwrap_or_default().len(), 1);
    moder.svc.approve_request(&moder.keys, &cid, &pk(&joiner)).await.unwrap_or_else(|e| panic!("approve: {e}"));
    w.pump(&all).await;
    assert_eq!(membership(&joiner, &cid).await, MEMBERSHIP_JOINED);
    assert!(newest(&joiner, &cid).await.is_some());

    // An admission of somebody who is no moderator: refused by the
    // keeper, and a join with it waits for an admission that never comes:
    // no key, and no refusal either (wire §7.1).
    let state = owner.svc.state(&cid).await.unwrap_or_else(|e| panic!("{e}"));
    let admit = wire::Admit { community_id: cid.clone(), who: pk(&other), admit_id: "1".repeat(32), levels: 0, invite_id: None, expires_at: w.clock.now() + 3600, ticket_id: None };
    let keeper = state.keeper().map(|k| k.pubkey.clone()).unwrap_or_else(|| panic!("keeper"));
    reader.svc.send_note(Some(&state), &cid, &keeper, wire::T_ADMIT, &admit, None).await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&all).await;
    let join = wire::Join { community_id: cid.clone(), proof: None, admit_id: Some(admit.admit_id.clone()), created_at: w.clock.now() };
    other.svc.send_note(Some(&state), &cid, &keeper, wire::T_JOIN, &join, None).await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&all).await;
    assert!(newest(&other, &cid).await.is_none());
    assert_ne!(membership(&other, &cid).await, MEMBERSHIP_REJECTED);

    // An invitation used by one admission is not used by another.
    let k = owner.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("keeper"));
    let log = owner.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("log"));
    let mut k = k.lock().await;
    let first = wire::Admit { invite_id: Some("2".repeat(32)), admit_id: "3".repeat(32), ..admit.clone() };
    assert!(k.on_admit(&log, &pk(&moder), &first, w.clock.now()).await.unwrap_or(false));
    assert!(k.on_admit(&log, &pk(&moder), &first, w.clock.now()).await.unwrap_or(false), "the same admission again");
    let second = wire::Admit { admit_id: "4".repeat(32), ..first.clone() };
    assert!(!k.on_admit(&log, &pk(&moder), &second, w.clock.now()).await.unwrap_or(true), "the invitation was used");
    let by_reader = wire::Admit { admit_id: "5".repeat(32), invite_id: None, ..first.clone() };
    assert!(!k.on_admit(&log, &pk(&reader), &by_reader, w.clock.now()).await.unwrap_or(true));
    // A join with the admission that was taken goes through once.
    let join = wire::Join { community_id: cid.clone(), proof: None, admit_id: Some(first.admit_id.clone()), created_at: w.clock.now() };
    assert_eq!(k.on_join(&log, &pk(&other), &join, w.clock.now()).await.ok(), Some(Joined::Issued));
    assert_eq!(k.on_join(&log, &pk(&other), &join, w.clock.now()).await.ok(), Some(Joined::Issued), "sent again by a member: issued again");
    assert!(k.on_leave(&pk(&other), w.clock.now()).await.unwrap_or(false));
    assert!(matches!(k.on_join(&log, &pk(&other), &join, w.clock.now()).await.ok(), Some(Joined::Refused(_))), "used");
}

#[tokio::test]
async fn an_invitation_goes_to_an_admission_and_a_join() {
    let w = World::new();
    let owner = w.node().await;
    let guest = w.node().await;
    let all = [&owner, &guest];
    let cid = create(&w, &owner, JoinMode::Invite, History::Open, &[H1, H2]).await;
    w.pump(&all).await;
    let (invite_id, _) = owner.svc.invite(&owner.keys, &cid, &pk(&guest)).await.unwrap_or_else(|e| panic!("invite: {e}"));
    w.pump(&all).await;
    guest.svc.answer_invite(&invite_id, true).await.unwrap_or_else(|e| panic!("answer: {e}"));
    w.pump(&all).await;
    assert_eq!(membership(&guest, &cid).await, MEMBERSHIP_JOINED);
    assert!(newest(&guest, &cid).await.is_some());
}

/// The link of an invite-only channel is nothing to a stranger, and the
/// invitation taken to the one who holds it (a bot writes «here is the
/// link» to the one it invited).
#[tokio::test]
async fn the_link_of_an_invite_channel_takes_the_invitation_held() {
    let w = World::new();
    let owner = w.node().await;
    let guest = w.node().await;
    let stranger = w.node().await;
    let all = [&owner, &guest, &stranger];
    let cid = create(&w, &owner, JoinMode::Invite, History::Open, &[H1, H2]).await;
    w.pump(&all).await;
    let link = owner.svc.share_link(&cid).await.unwrap_or_else(|e| panic!("link: {e}"));
    stranger.svc.ask_journal(&cid).await.unwrap_or_else(|e| panic!("journal: {e}"));
    w.pump(&all).await;
    let refused = stranger.svc.open_link(&stranger.keys, &link, "").await.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(refused.contains("invitations only"), "a stranger: {refused}");
    owner.svc.invite(&owner.keys, &cid, &pk(&guest)).await.unwrap_or_else(|e| panic!("invite: {e}"));
    w.pump(&all).await;
    guest.svc.open_link(&guest.keys, &link, "").await.unwrap_or_else(|e| panic!("the link with an invitation: {e}"));
    w.pump(&all).await;
    assert_eq!(membership(&guest, &cid).await, MEMBERSHIP_JOINED);
    assert_eq!(membership(&stranger, &cid).await, MEMBERSHIP_NONE, "the stranger stays out");
}

/// A subscription made anew brings the wraps of two days back (a relay
/// connected again, a catch-up): the copy of the admission comes once
/// more to a guest who is in already, and no second join goes out (живой
/// запуск 2026-10-24: a join every five seconds).
#[tokio::test]
async fn an_admission_heard_again_sends_no_second_join() {
    let w = World::new();
    let owner = w.node().await;
    let guest = w.node().await;
    let all = [&owner, &guest];
    let cid = create(&w, &owner, JoinMode::Invite, History::Open, &[H1, H2]).await;
    w.pump(&all).await;
    let (invite_id, _) = owner.svc.invite(&owner.keys, &cid, &pk(&guest)).await.unwrap_or_else(|e| panic!("invite: {e}"));
    w.pump(&all).await;
    guest.svc.answer_invite(&invite_id, true).await.unwrap_or_else(|e| panic!("answer: {e}"));
    w.pump(&all).await;
    assert_eq!(membership(&guest, &cid).await, MEMBERSHIP_JOINED);
    let sent = guest.net.wraps.load(Ordering::SeqCst);
    let again: Vec<serde_json::Value> = w.relay.inbox.lock().unwrap_or_else(|p| p.into_inner()).get(guest.me.as_hex()).cloned().unwrap_or_default();
    assert!(!again.is_empty(), "the admission and the issue came to the guest");
    for wrap in &again {
        guest.svc.on_wrap(&guest.keys, wrap).await.unwrap_or_else(|e| panic!("heard again: {e}"));
    }
    w.pump(&all).await;
    assert_eq!(guest.net.wraps.load(Ordering::SeqCst), sent, "no join went out again");
    assert_eq!(membership(&guest, &cid).await, MEMBERSHIP_JOINED);
}

#[tokio::test]
async fn a_ban_puts_the_reader_out_at_the_window_and_the_others_read_on() {
    let w = World::new();
    let owner = w.node().await;
    let (a, b) = (w.node().await, w.node().await);
    let all = [&owner, &a, &b];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &a, &cid, &all).await;
    join_by_link(&w, &owner, &b, &cid, &all).await;
    w.clock.advance(10);
    owner.svc.act(&owner.keys, &cid, OpBody::Ban { who: pk(&a), reason: None }).await.unwrap_or_else(|e| panic!("ban: {e}"));
    w.pump(&all).await;
    // The ban is in force at once by the journal; the epoch waits for its
    // window.
    assert_eq!(w.relay.epochs(), vec![1]);
    w.clock.advance(150);
    tick(&owner).await;
    assert_eq!(w.relay.epochs(), vec![1], "not before the window");
    w.clock.advance(200);
    tick(&owner).await;
    assert_eq!(w.relay.epochs(), vec![1, 2]);
    w.pump(&all).await;
    assert!(has_key(&b, &cid, 2).await, "the others read the new epoch");
    assert!(!has_key(&a, &cid, 2).await, "the banned reader does not");
    assert_eq!(membership(&a, &cid).await, MEMBERSHIP_LEFT);
    assert_eq!(membership(&b, &cid).await, MEMBERSHIP_JOINED);
    // A post under epoch 2 is read by b and not by a.
    owner.svc.send_post(&owner.keys, &cid, "after the ban", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    assert_eq!(post_texts(&b, &cid).await, vec!["after the ban".to_string()]);
    assert!(post_texts(&a, &cid).await.is_empty());
}

/// A tree past keydist's threshold of a planned migration (more than half
/// of the leaves handed out revoked): keydist's `next_wake` says "now"
/// until a migration starts, and the engine starts none yet. The turn
/// must not ask to be woken in the past: the loop of the module would
/// turn at full speed (communities-bench, 2026-10-23).
#[tokio::test]
async fn a_tree_past_its_migration_threshold_does_not_wake_the_keeper_at_once() {
    let w = World::new();
    let owner = w.node().await;
    let (a, b) = (w.node().await, w.node().await);
    let all = [&owner, &a, &b];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &a, &cid, &all).await;
    join_by_link(&w, &owner, &b, &cid, &all).await;
    w.clock.advance(10);
    for r in [&a, &b] {
        owner.svc.act(&owner.keys, &cid, OpBody::Ban { who: pk(r), reason: None }).await.unwrap_or_else(|e| panic!("ban: {e}"));
    }
    w.clock.advance(400);
    tick(&owner).await;
    assert_eq!(w.relay.epochs(), vec![1, 2]);
    {
        let k = owner.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("keeper"));
        let k = k.lock().await;
        let sd = k.scope(SCOPE_EVERYONE).unwrap_or_else(|| panic!("scope"));
        assert!(sd.migration_due(&sd.state().policy.thresholds).is_some(), "2 of 3 leaves revoked");
    }
    for _ in 0..3 {
        w.clock.advance(10);
        let wake = owner.svc.tick().await.unwrap_or_else(|e| panic!("tick: {e}"));
        let now = u64::try_from(w.clock.now()).unwrap_or(0);
        assert!(wake.is_none_or(|t| t > now), "woken at {wake:?}, now {now}");
    }
}

#[tokio::test]
async fn rotate_now_builds_an_epoch_at_once_and_supersedes_one_that_waits() {
    let w = World::new();
    let owner = w.node().await;
    let moder = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &moder, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    owner.svc.act(&owner.keys, &cid, OpBody::SetRole { who: pk(&moder), role: Role::Moderator }).await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&all).await;
    // Urgent: no window, no least interval.
    moder.svc.act(&moder.keys, &cid, OpBody::RotateNow { scope_id: None }).await.unwrap_or_else(|e| panic!("rotate now: {e}"));
    w.pump(&all).await;
    assert_eq!(w.relay.epochs(), vec![1, 2]);
    assert!(has_key(&reader, &cid, 2).await);

    // An epoch that cannot go out (every host down) is replaced when an
    // urgent one is asked for: its number burns, the next names the
    // published one.
    w.hosts.set_down(H1, true);
    w.hosts.set_down(H2, true);
    moder.svc.act(&moder.keys, &cid, OpBody::RotateNow { scope_id: None }).await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&all).await;
    assert_eq!(keeper_state(&owner, &cid).await.pending.map(|p| p.key.epoch), Some(3));
    w.clock.advance(5);
    moder.svc.act(&moder.keys, &cid, OpBody::RotateNow { scope_id: None }).await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&all).await;
    let pending = keeper_state(&owner, &cid).await.pending.unwrap_or_else(|| panic!("pending"));
    assert_eq!((pending.key.epoch, pending.prev_epoch), (4, Some(2)), "3 burnt");
    w.hosts.set_down(H1, false);
    w.hosts.set_down(H2, false);
    tick(&owner).await;
    assert_eq!(w.relay.epochs(), vec![1, 2, 4]);
    w.pump(&all).await;
    // The gap of two is taken by the members (wire §5).
    assert!(has_key(&reader, &cid, 4).await);
    assert!(!has_key(&reader, &cid, 3).await);
    // The blobs of the burnt epoch and of epoch 2 went (open history).
    assert!(owner.svc.keeper(&cid).await.ok().flatten().is_some());
    assert!(reader.errors().is_empty(), "{:?}", reader.errors());
}

#[tokio::test]
async fn a_member_catches_up_three_epochs_by_the_chain_of_an_open_history() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &[&owner, &reader]).await;
    for _ in 0..3 {
        owner.svc.act(&owner.keys, &cid, OpBody::RotateNow { scope_id: None }).await.unwrap_or_else(|e| panic!("{e}"));
        w.pump(&[&owner]).await;
    }
    assert_eq!(w.relay.epochs(), vec![1, 2, 3, 4]);
    // The blobs of epochs 2 and 3 are gone: only the chain gives them.
    assert!(w.hosts.deletes.load(Ordering::SeqCst) > 0);
    w.pump(&[&owner, &reader]).await;
    for e in 1..=4 {
        assert!(has_key(&reader, &cid, e).await, "epoch {e}");
    }
}

/// A member back after a while under an open history, the catch-up
/// handing the newest epoch first: one index and one shard, of the
/// newest; every older key comes by the chain. The blobs of the older
/// epochs are gone from the hosts at once, a GET of them is a 404 per
/// host (communities-bench, 2026-10-23: two per epoch, 576 a day).
#[tokio::test]
async fn a_member_back_under_an_open_history_fetches_the_newest_shard_only() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &[&owner, &reader]).await;
    reader.skip_all(&w.relay);
    let start = w.relay.of_kind(KIND_EPOCH).len();
    for _ in 0..5 {
        w.clock.advance(300);
        owner.svc.act(&owner.keys, &cid, OpBody::RotateNow { scope_id: None }).await.unwrap_or_else(|e| panic!("{e}"));
        w.pump(&[&owner]).await;
    }
    let missed: Vec<serde_json::Value> = w.relay.of_kind(KIND_EPOCH)[start..].to_vec();
    assert_eq!(missed.len(), 5);
    let before = w.hosts.gets.load(Ordering::SeqCst);
    for ev in missed.iter().rev() {
        reader.svc.on_event(&reader.keys, ev, false).await.unwrap_or_else(|e| panic!("{e}"));
    }
    // The shard is fetched off the events (`run_fetch`), as the module runs it.
    reader.svc.run_fetches(&reader.keys).await;
    for e in 2..=6 {
        assert!(has_key(&reader, &cid, e).await, "epoch {e}");
    }
    assert_eq!(w.hosts.gets.load(Ordering::SeqCst) - before, 2, "the index and the shard of the newest epoch");
}

#[tokio::test]
async fn a_member_catches_up_by_the_shards_of_from_join() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::FromJoin, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &[&owner, &reader]).await;
    // From the join on: the first epoch after it.
    owner.svc.act(&owner.keys, &cid, OpBody::RotateNow { scope_id: None }).await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&[&owner, &reader]).await;
    let first = newest(&reader, &cid).await.map(|k| k.epoch).unwrap_or(0);
    assert!(first >= 2);
    reader.skip_all(&w.relay);
    let mut published = Vec::new();
    for _ in 0..3 {
        w.clock.advance(10);
        owner.svc.act(&owner.keys, &cid, OpBody::RotateNow { scope_id: None }).await.unwrap_or_else(|e| panic!("{e}"));
        w.pump(&[&owner]).await;
        published.push(*w.relay.epochs().last().unwrap_or(&0));
    }
    // The reader comes back: every missed epoch from its shard (kept for
    // the retention under `from_join`).
    let held = w.relay.of_kind(KIND_EPOCH);
    for ev in held.iter().rev().take(3) {
        reader.svc.on_event(&reader.keys, ev, false).await.unwrap_or_else(|e| panic!("{e}"));
    }
    reader.svc.run_fetches(&reader.keys).await;
    for e in published {
        assert!(has_key(&reader, &cid, e).await, "epoch {e}");
    }
    let mut k = owner.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("keeper")).lock_owned().await;
    assert!(k.garbage().await.unwrap_or(0) > 0, "the old blobs wait for the retention");
}

#[tokio::test]
async fn content_that_came_before_its_key_is_read_when_the_key_comes() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    reader.hold(KIND_EPOCH, true);
    owner.svc.act(&owner.keys, &cid, OpBody::RotateNow { scope_id: None }).await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&[&owner]).await;
    owner.svc.send_post(&owner.keys, &cid, "under epoch 2", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    assert!(post_texts(&reader, &cid).await.is_empty());
    assert_eq!(db::channels::count_pending(reader.svc.db(), &cid).await.unwrap_or(0), 1);
    reader.hold(KIND_EPOCH, false);
    w.pump(&all).await;
    assert_eq!(post_texts(&reader, &cid).await, vec!["under epoch 2".to_string()]);
    assert_eq!(db::channels::count_pending(reader.svc.db(), &cid).await.unwrap_or(9), 0);
}

#[tokio::test]
async fn another_event_for_an_epoch_taken_is_ignored_and_the_bundle_asked_for() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &[&owner, &reader]).await;
    let key_before = newest(&reader, &cid).await;
    // The keeper's key signs another version of epoch 1.
    let kkeys = owner.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("keeper")).lock().await.keys().clone();
    let ev = &w.relay.of_kind(KIND_EPOCH)[0];
    let mut content: crate::epoch::EpochContent = serde_json::from_str(ev["content"].as_str().unwrap_or("")).unwrap_or_else(|e| panic!("{e}"));
    content.blobs = vec![H2.into()];
    let other = crate::epoch::sign_epoch(&kkeys, &cid, SCOPE_EVERYONE, &content).unwrap_or_else(|e| panic!("{e}"));
    let inbox_before = w.relay.inbox.lock().map(|i| i.values().map(Vec::len).sum::<usize>()).unwrap_or(0);
    reader.svc.on_event(&reader.keys, &serde_json::to_value(&other).unwrap_or_default(), true).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(newest(&reader, &cid).await, key_before, "the key held stays");
    flush(&reader).await;
    let inbox_after = w.relay.inbox.lock().map(|i| i.values().map(Vec::len).sum::<usize>()).unwrap_or(0);
    assert_eq!(inbox_after, inbox_before + 1, "a rebundle went to the keeper");
    w.pump(&[&owner, &reader]).await;
    assert_eq!(newest(&reader, &cid).await, key_before);
}

#[tokio::test]
async fn two_devices_of_one_member_share_the_keys_by_a_note_of_their_own() {
    let w = World::new();
    let owner = w.node().await;
    let phone = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &phone, &cid, &[&owner, &phone]).await;
    owner.svc.send_post(&owner.keys, &cid, "hello", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    let desktop = w.device(phone.keys.clone()).await;
    w.pump(&[&owner, &phone, &desktop]).await;
    assert_eq!(membership(&desktop, &cid).await, MEMBERSHIP_JOINED);
    assert_eq!(newest(&desktop, &cid).await, newest(&phone, &cid).await);
    assert_eq!(post_texts(&desktop, &cid).await, vec!["hello".to_string()]);
    // A device without the note asks the keeper.
    let laptop = w.device(phone.keys.clone()).await;
    laptop.skip_all(&w.relay);
    w.pump(&[&laptop]).await;
    assert!(laptop.svc.ensure_keys(&laptop.keys, &cid).await.is_ok() || laptop.svc.log(&cid).await.ok().flatten().is_none());
}

#[tokio::test]
async fn a_device_without_a_note_asks_for_its_bundle() {
    let w = World::new();
    let owner = w.node().await;
    let phone = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &phone, &cid, &[&owner, &phone]).await;
    // A new device that heard the journal but none of my notes.
    let tablet = w.device(phone.keys.clone()).await;
    {
        let relay = &w.relay;
        let events = relay.events.lock().map(|e| e.clone()).unwrap_or_default();
        for ev in &events {
            let _ = tablet.svc.on_event(&tablet.keys, ev, false).await;
        }
    }
    assert!(newest(&tablet, &cid).await.is_none());
    assert!(tablet.svc.ensure_keys(&tablet.keys, &cid).await.unwrap_or(false));
    w.pump(&[&owner]).await;
    // The issue goes to my inbox, which every device of mine reads.
    let me = tablet.me.as_hex().to_string();
    let wraps = w.relay.inbox.lock().map(|i| i.get(&me).cloned().unwrap_or_default()).unwrap_or_default();
    for wr in wraps {
        let _ = tablet.svc.on_wrap(&tablet.keys, &wr).await;
    }
    assert!(newest(&tablet, &cid).await.is_some());
}

#[tokio::test]
async fn an_offline_keeper_bans_by_the_journal_and_the_newcomer_waits() {
    let w = World::new();
    let owner = w.node().await;
    let admin = w.node().await;
    let (spammer, newcomer) = (w.node().await, w.node().await);
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    owner.svc.act(&owner.keys, &cid, OpBody::SetRole { who: pk(&admin), role: Role::Admin }).await.unwrap_or_else(|e| panic!("{e}"));
    join_by_link(&w, &owner, &spammer, &cid, &[&owner, &admin, &spammer]).await;
    let link = owner.svc.share_link(&cid).await.unwrap_or_default();
    // The keeper's device is off from here on.
    admin.svc.act(&admin.keys, &cid, OpBody::Ban { who: pk(&spammer), reason: None }).await.unwrap_or_else(|e| panic!("ban: {e}"));
    w.pump(&[&admin, &spammer]).await;
    assert!(spammer.svc.send_post(&spammer.keys, &cid, "spam", TextFormat::Plain).await.is_err(), "the journal holds the ban at once");
    w.pump(&[&newcomer]).await;
    newcomer.svc.open_link(&newcomer.keys, &link, "").await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&[&admin, &newcomer]).await;
    assert!(newest(&newcomer, &cid).await.is_none(), "only the key waits");
    // The keeper comes back within the window of the note.
    w.clock.advance(60);
    w.pump(&[&owner, &admin, &newcomer, &spammer]).await;
    assert_eq!(membership(&newcomer, &cid).await, MEMBERSHIP_JOINED);
    assert!(!keeper_state(&owner, &cid).await.members.contains_key(&crate::keeper::member_id(&pk(&spammer)).unwrap_or_else(|e| panic!("{e}"))));
}

#[tokio::test]
async fn keydist_and_the_rows_agree_on_the_members() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &[&owner, &reader]).await;
    reader.svc.leave(&reader.keys, &cid).await.unwrap_or_else(|e| panic!("leave: {e}"));
    w.pump(&[&owner, &reader]).await;
    let k = owner.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("keeper"));
    let k = k.lock().await;
    let sd = k.scope(SCOPE_EVERYONE).unwrap_or_else(|| panic!("scope"));
    assert!(!sd.is_member(&crate::keeper::member_id(&pk(&reader)).unwrap_or_else(|e| panic!("{e}"))));
    assert!(sd.is_member(&crate::keeper::member_id(&pk(&owner)).unwrap_or_else(|e| panic!("{e}"))));
}

#[tokio::test]
async fn rows_that_do_not_add_up_are_recovered_as_an_emergency_generation() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &[&owner, &reader]).await;
    // A row that breaks an invariant (a revoked leaf never handed out).
    sqlx::query("INSERT INTO cm_keeper_revoked (community_id, scope_id, gen, leaf) VALUES (?, 'everyone', 1, 999999)")
        .bind(&cid)
        .execute(owner.svc.db().pool())
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    owner.svc.unload_keepers().await;
    let state = keeper_state(&owner, &cid).await;
    assert_eq!(state.gens.iter().map(|g| g.gen).collect::<Vec<_>>(), vec![2], "a new generation above the old");
    assert!(matches!(state.gens[0].kind, keydist::GenerationKind::Emergency));
    assert_eq!(state.members.len(), 2, "every member stays");
    tick(&owner).await;
    assert_eq!(w.relay.epochs(), vec![1, 2], "the next number, never one used");
    // The members ask for the bundle of the new generation and read on.
    w.pump(&[&owner, &reader]).await;
    assert!(has_key(&reader, &cid, 2).await);
    assert!(has_key(&owner, &cid, 2).await);
    // The keys of epochs opened before the emergency stay (wire §6.5).
    assert!(has_key(&reader, &cid, 1).await, "the history stays readable");
}

// ─── Sub-stage 1.3 ──────────────────────────────────────────────────────────

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// An event put on the relay by hand (somebody who does not use this
/// service as it is).
fn inject(w: &World, event: &nostr::prelude::Event) {
    lock(&w.relay.events).push(serde_json::to_value(event).unwrap_or_default());
}

/// A message sealed by hand: `author` signs it, under the newest key `holder` has.
async fn forged_message(holder: &Node, author: &nostr::key::Keys, cid: &str, links: Links, text: &str, at: i64) -> nostr::prelude::Event {
    let key = newest(holder, cid).await.unwrap_or_else(|| panic!("no key"));
    let place = Place::feed(cid);
    let inner = wire::sign_inner(author, &place, &links, &Envelope::text(text).encode(), at).unwrap_or_else(|e| panic!("{e}"));
    let payload = wire::Payload::event(wire::T_MSG, &inner).unwrap_or_else(|e| panic!("{e}"));
    wire::seal_content(&place, &key, links.outer_target(), &payload, at).unwrap_or_else(|e| panic!("{e}"))
}

async fn history(n: &Node, cid: &str, limit: i64, before: Option<i64>) -> FeedPage {
    n.svc.history(&n.keys, cid, limit, before).await.unwrap_or_else(|e| panic!("history: {e}"))
}

async fn thread_texts(n: &Node, cid: &str, post: &str) -> Vec<String> {
    let page = n.svc.thread(&n.keys, cid, post, 100, None).await.unwrap_or_else(|e| panic!("thread: {e}"));
    page.items.into_iter().filter_map(|i| i.text).collect()
}

fn snapshots(w: &World) -> Vec<crate::snapshot::Snapshot> {
    w.relay.of_kind(KIND_SNAPSHOT).iter().map(|e| crate::snapshot::content_of(&e.to_string()).unwrap_or_else(|e| panic!("{e}"))).collect()
}

/// The key the journal names as the keeper's inbox (wire §3).
async fn inbox_of_keeper(n: &Node, cid: &str) -> PubKey {
    let log = n.svc.log(cid).await.ok().flatten().unwrap_or_else(|| panic!("journal"));
    log.state().keeper().map(|k| k.inbox().clone()).unwrap_or_else(|| panic!("keeper"))
}

/// The notes to the keeper on its inbox: wraps there that open, with the
/// owner's key, to a note of the keeper's.
async fn keeper_inbox(w: &World, owner: &Node, cid: &str) -> usize {
    let to = inbox_of_keeper(owner, cid).await;
    let wraps = lock(&w.relay.inbox).get(to.as_hex()).cloned().unwrap_or_default();
    let keeper_notes = [wire::T_JOIN, wire::T_ADMIT, wire::T_REBUNDLE, wire::T_MISSED, wire::T_LEAVE, wire::T_KEEPER_MASTER];
    wraps
        .iter()
        .filter_map(|wr| crate::notes::open(&owner.keys, wr).ok())
        .filter(|o| Envelope::parse(&o.content).is_ok_and(|e| keeper_notes.contains(&e.t.as_str())))
        .count()
}

#[tokio::test]
async fn a_snapshot_goes_out_once_after_joins_and_not_again_without_a_change_nor_within_five_minutes() {
    let w = World::new();
    let owner = w.node().await;
    let (a, b) = (w.node().await, w.node().await);
    let all = [&owner, &a, &b];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    assert_eq!(snapshots(&w).iter().map(|s| s.members).collect::<Vec<_>>(), vec![1], "the owner, with the first epoch");
    join_by_link(&w, &owner, &a, &cid, &all).await;
    join_by_link(&w, &owner, &b, &cid, &all).await;
    assert_eq!(snapshots(&w).len(), 1, "not within five minutes of the last");
    w.clock.advance(300);
    tick(&owner).await;
    let s = snapshots(&w);
    assert_eq!(s.iter().map(|s| s.members).collect::<Vec<_>>(), vec![1, 3]);
    assert_eq!(s[1].epoch_ref.get(SCOPE_EVERYONE), Some(&1));
    tick(&owner).await;
    w.clock.advance(900);
    tick(&owner).await;
    assert_eq!(snapshots(&w).len(), 2, "nothing changed: no snapshot");
    w.pump(&all).await;
    let row = db::channels::get(a.svc.db(), &cid).await.ok().flatten().unwrap_or_else(|| panic!("row"));
    assert_eq!(row.members, 3);
    assert_eq!(a.svc.member(&a.keys).latest_snapshot(&cid).await.ok().flatten().map(|s| s.at), Some(s[1].at));
    assert!(a.errors().is_empty(), "{:?}", a.errors());
}

/// Every turn of the keeper (each note, each operation, each epoch) asks
/// for the snapshot; the rows of the readers are read only when keydist's
/// membership, the staff's levels or the tickets moved since they were
/// last read (communities-bench, 2026-10-23: 0.6 s a turn at 100 K
/// readers, 6 s at a million). Rows changed behind the keeper's back are
/// therefore not seen by a turn where nothing moved; a reader who comes
/// is.
#[tokio::test]
async fn a_turn_without_a_change_of_the_readers_does_not_read_them_again() {
    let w = World::new();
    let owner = w.node().await;
    let (a, b) = (w.node().await, w.node().await);
    let all = [&owner, &a, &b];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    // Let in a minute after the channel was made: its date is its own.
    w.clock.advance(60);
    join_by_link(&w, &owner, &a, &cid, &all).await;
    w.clock.advance(300);
    tick(&owner).await;
    assert_eq!(snapshots(&w).iter().map(|s| s.members).collect::<Vec<_>>(), vec![1, 2]);
    // The dates of the readers, gone from the rows: read again, they would
    // make other entries and a new snapshot.
    sqlx::query("DELETE FROM cm_keeper_readers WHERE community_id = ?").bind(&cid).execute(owner.svc.db().pool()).await.unwrap_or_else(|e| panic!("{e}"));
    w.clock.advance(900);
    tick(&owner).await;
    assert_eq!(snapshots(&w).len(), 2, "nothing moved: the rows were not read");
    join_by_link(&w, &owner, &b, &cid, &all).await;
    w.clock.advance(300);
    tick(&owner).await;
    assert_eq!(snapshots(&w).iter().map(|s| s.members).collect::<Vec<_>>(), vec![1, 2, 3], "a reader came: read again");
}

#[tokio::test]
async fn commenters_are_judged_by_the_shard_and_the_journal() {
    let w = World::new();
    let owner = w.node().await;
    let (a, b, c) = (w.node().await, w.node().await, w.node().await);
    let all = [&owner, &a, &b, &c];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &a, &cid, &all).await;
    join_by_link(&w, &owner, &b, &cid, &all).await;
    w.clock.advance(300);
    tick(&owner).await;
    w.pump(&all).await;
    let (post, _) = owner.svc.send_post(&owner.keys, &cid, "the post", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;

    // A reader of the snapshot: taken.
    w.clock.advance(1);
    a.svc.send_comment(&a.keys, &cid, &post, None, "by a", TextFormat::Plain).await.unwrap_or_else(|e| panic!("comment: {e}"));
    w.pump(&all).await;
    assert_eq!(thread_texts(&b, &cid, &post).await, vec!["by a".to_string()]);
    assert!(db::snapshots::shard(b.svc.db(), &cid, &snapshots(&w)[1].shards[0].from, &snapshots(&w)[1].shards[0].sha256).await.ok().flatten().is_some(), "the shard is cached");

    // Somebody no snapshot names, with the key of the epoch: waits for the
    // next snapshot, and is refused when that one does not name them.
    let stranger = nostr::key::Keys::generate();
    w.clock.advance(1);
    let links = Links { thread: Some(post.clone()), ..Links::default() };
    inject(&w, &forged_message(&a, &stranger, &cid, links.clone(), "spam", w.clock.now()).await);
    w.pump(&all).await;
    assert_eq!(thread_texts(&b, &cid, &post).await, vec!["by a".to_string()]);
    assert_eq!(db::channels::count_pending(b.svc.db(), &cid).await.unwrap_or(0), 1, "waits for a snapshot");
    w.clock.advance(300);
    join_by_link(&w, &owner, &c, &cid, &all).await;
    tick(&owner).await;
    w.pump(&all).await;
    assert_eq!(snapshots(&w).len(), 3);
    assert_eq!(thread_texts(&b, &cid, &post).await, vec!["by a".to_string()], "refused after the snapshot");
    assert_eq!(db::channels::count_pending(b.svc.db(), &cid).await.unwrap_or(9), 0);

    // Banned by the journal: refused although the shard names them.
    owner.svc.act(&owner.keys, &cid, OpBody::Ban { who: pk(&a), reason: None }).await.unwrap_or_else(|e| panic!("ban: {e}"));
    w.pump(&all).await;
    w.clock.advance(1);
    inject(&w, &forged_message(&b, &a.keys, &cid, links, "after the ban", w.clock.now()).await);
    w.pump(&all).await;
    assert_eq!(thread_texts(&b, &cid, &post).await, vec!["by a".to_string()]);
    assert_eq!(thread_texts(&c, &cid, &post).await, vec!["by a".to_string()]);
}

#[tokio::test]
async fn before_the_first_snapshot_any_author_counts_for_five_minutes() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    // The first epoch cannot go out (every host is down): no snapshot.
    w.hosts.set_down(H1, true);
    w.hosts.set_down(H2, true);
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    assert!(snapshots(&w).is_empty());
    let log = owner.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("log"));
    let member = reader.svc.member(&reader.keys);
    w.pump(&[&reader]).await;
    let stranger = PubKey::parse(&nostr::key::Keys::generate().public_key().to_hex()).unwrap_or_else(|| panic!("key"));
    let at = w.clock.now();
    assert_eq!(member.reader_of(&log, &stranger, at + 10).await.ok(), Some(crate::member::readers::Standing::Known(Some(crate::state::Reader { since: at }))));
    assert_eq!(member.reader_of(&log, &stranger, at + 301).await.ok(), Some(crate::member::readers::Standing::Wait), "after the window: the next snapshot says");
}

#[tokio::test]
async fn a_snapshot_of_a_keeper_of_a_past_series_is_refused() {
    let w = World::new();
    let owner = w.node().await;
    let a = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &a, &cid, &[&owner, &a]).await;
    let kkeys = owner.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("keeper")).lock().await.keys().clone();
    let latest = || async { a.svc.member(&a.keys).latest_snapshot(&cid).await.ok().flatten().map(|s| s.at) };
    let mut s = snapshots(&w).remove(0);
    s.at += 10;
    let newer = crate::snapshot::sign_snapshot(&kkeys, &s).unwrap_or_else(|e| panic!("{e}"));
    a.svc.on_event(&a.keys, &serde_json::to_value(&newer).unwrap_or_default(), true).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(latest().await, Some(s.at), "the current keeper's newer one is taken");
    // The owner hands the keeping to another key: serial 1.
    let next = PubKey::parse(&nostr::key::Keys::generate().public_key().to_hex()).unwrap_or_else(|| panic!("key"));
    owner.svc.act(&owner.keys, &cid, OpBody::Keeper { pubkey: next, serial: 1, from_epoch: 5, inbox: None }).await.unwrap_or_else(|e| panic!("keeper op: {e}"));
    flush(&owner).await;
    w.pump(&[&a]).await;
    s.at += 10;
    let stale = crate::snapshot::sign_snapshot(&kkeys, &s).unwrap_or_else(|e| panic!("{e}"));
    a.svc.on_event(&a.keys, &serde_json::to_value(&stale).unwrap_or_default(), true).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(latest().await, Some(s.at - 10), "the keeper of serial 0 is no longer the keeper");
}

#[tokio::test]
async fn a_link_secret_from_somebody_not_its_author_is_refused() {
    let w = World::new();
    let owner = w.node().await;
    let (a, b) = (w.node().await, w.node().await);
    let all = [&owner, &a, &b];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &a, &cid, &all).await;
    join_by_link(&w, &owner, &b, &cid, &all).await;
    let forge = |epoch: u32| {
        let link = wire::LinkPayload { link_epoch: epoch, secret: LinkSecret::from_bytes(&[7u8; 32]).unwrap_or_else(|e| panic!("{e}")).encode() };
        wire::sign_link(&a.keys, &cid, &link, w.clock.now()).unwrap_or_else(|e| panic!("{e}"))
    };
    let seal = |inner: &nostr::prelude::Event, key: &EpochKey| {
        let payload = wire::Payload::event(wire::T_LINK, inner).unwrap_or_else(|e| panic!("{e}"));
        wire::seal_content(&Place::feed(&cid), key, None, &payload, w.clock.now()).unwrap_or_else(|e| panic!("{e}"))
    };
    let key = newest(&a, &cid).await.unwrap_or_else(|| panic!("key"));
    let secret_of = |n: &Node, epoch: u32| {
        let svc = n.svc.clone();
        let cid = cid.clone();
        async move { crate::secrets::links(svc.env.secrets.as_ref(), &cid).await.ok().and_then(|l| l.get(&epoch).map(|s| s.encode())) }
    };
    let before = secret_of(&owner, 0).await;
    // A reader's secret for the current link epoch: the author of `create` is the owner.
    inject(&w, &seal(&forge(0), &key));
    w.pump(&all).await;
    assert_eq!(secret_of(&owner, 0).await, before);
    // One for a link epoch whose `rotate_link` has not come: it waits for
    // the operation and is refused when the author turns out another.
    inject(&w, &seal(&forge(1), &key));
    w.pump(&[&b]).await;
    assert_eq!(db::channels::count_pending(b.svc.db(), &cid).await.unwrap_or(0), 1);
    owner.svc.act(&owner.keys, &cid, OpBody::RotateLink { link_epoch: 1 }).await.unwrap_or_else(|e| panic!("rotate link: {e}"));
    w.pump(&all).await;
    let mine = secret_of(&owner, 1).await;
    assert!(mine.is_some());
    assert_eq!(secret_of(&b, 1).await, mine, "the owner's secret, not the reader's");
    assert_eq!(db::channels::count_pending(b.svc.db(), &cid).await.unwrap_or(9), 0);
}

#[tokio::test]
async fn a_join_without_an_issue_is_sent_again_after_ten_minutes() {
    let w = World::new();
    let owner = w.node().await;
    let newcomer = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    let link = owner.svc.share_link(&cid).await.unwrap_or_default();
    // The keeper's device is off.
    w.pump(&[&newcomer]).await;
    newcomer.svc.open_link(&newcomer.keys, &link, "").await.unwrap_or_else(|e| panic!("{e}"));
    flush(&newcomer).await;
    assert_eq!(keeper_inbox(&w, &owner, &cid).await, 1);
    w.clock.advance(599);
    tick(&newcomer).await;
    assert_eq!(keeper_inbox(&w, &owner, &cid).await, 1, "not before ten minutes");
    w.clock.advance(2);
    tick(&newcomer).await;
    assert_eq!(keeper_inbox(&w, &owner, &cid).await, 2, "sent again");
    w.clock.advance(601);
    tick(&newcomer).await;
    assert_eq!(keeper_inbox(&w, &owner, &cid).await, 2, "then twice as late");
    // The keeper comes back: both joins answered, the newcomer in once.
    w.pump(&[&owner, &newcomer]).await;
    assert_eq!(membership(&newcomer, &cid).await, MEMBERSHIP_JOINED);
    assert!(newest(&newcomer, &cid).await.is_some());
    w.clock.advance(3 * 3600);
    tick(&newcomer).await;
    assert_eq!(keeper_inbox(&w, &owner, &cid).await, 2, "nothing more once in");
    assert!(owner.errors().is_empty(), "{:?}", owner.errors());
}

#[tokio::test]
async fn a_note_of_six_days_is_taken_one_of_more_than_seven_is_not() {
    let w = World::new();
    let owner = w.node().await;
    let (early, late) = (w.node().await, w.node().await);
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    let link = owner.svc.share_link(&cid).await.unwrap_or_default();
    w.pump(&[&early, &late]).await;
    early.svc.open_link(&early.keys, &link, "").await.unwrap_or_else(|e| panic!("{e}"));
    w.clock.advance(6 * 24 * 3600);
    w.pump(&[&owner, &early]).await;
    assert_eq!(membership(&early, &cid).await, MEMBERSHIP_JOINED, "the keeper slept six days");
    late.svc.open_link(&late.keys, &link, "").await.unwrap_or_else(|e| panic!("{e}"));
    w.clock.advance(7 * 24 * 3600 + 1);
    w.pump(&[&owner, &late]).await;
    assert!(newest(&late, &cid).await.is_none(), "older than a week");
    assert!(!keeper_state(&owner, &cid).await.members.contains_key(&crate::keeper::member_id(&pk(&late)).unwrap_or_else(|e| panic!("{e}"))));
}

#[tokio::test]
async fn the_keeper_takes_a_change_of_join_from_the_journal() {
    let w = World::new();
    let owner = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    assert!(keeper_state(&owner, &cid).await.policy.open_scope);
    owner.svc.act(&owner.keys, &cid, Settings { join: Some(JoinMode::Invite), ..Settings::default() }.body()).await.unwrap_or_else(|e| panic!("{e}"));
    assert!(!keeper_state(&owner, &cid).await.policy.open_scope, "closed by the operation");
    owner.svc.unload_keepers().await;
    assert!(!keeper_state(&owner, &cid).await.policy.open_scope, "and so from its rows");
    owner.svc.act(&owner.keys, &cid, Settings { history: Some(History::FromJoin), ..Settings::default() }.body()).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(keeper_state(&owner, &cid).await.policy.history, History::FromJoin.keydist());
}

#[tokio::test]
async fn the_feed_and_a_thread_come_in_pages() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &[&owner, &reader]).await;
    let mut first = String::new();
    for i in 0..30 {
        w.clock.advance(1);
        let (id, _) = owner.svc.send_post(&owner.keys, &cid, &format!("post {i}"), TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
        if i == 0 {
            first = id;
        }
    }
    for i in 0..25 {
        w.clock.advance(1);
        owner.svc.send_comment(&owner.keys, &cid, &first, None, &format!("comment {i}"), TextFormat::Plain).await.unwrap_or_else(|e| panic!("comment: {e}"));
    }
    w.pump(&[&owner, &reader]).await;
    let mut seen = Vec::new();
    let mut before = None;
    for _ in 0..3 {
        let page = history(&reader, &cid, 10, before).await;
        assert_eq!(page.items.len(), 10);
        assert!(!page.asked, "the database holds the page");
        let texts: Vec<String> = page.items.iter().filter_map(|i| i.text.clone()).collect();
        // Each page oldest first, as a chat reads; the next one goes above it.
        seen.splice(0..0, texts);
        before = page.next_before;
    }
    assert_eq!(seen, (0..30).map(|i| format!("post {i}")).collect::<Vec<_>>(), "oldest at the top, the newest at the bottom");
    let last = history(&reader, &cid, 10, before).await;
    assert!(last.items.is_empty() && last.asked, "past the start: the relays are asked");
    let asked = lock(&reader.net.syncs).last().cloned().unwrap_or_default();
    assert_eq!((asked["#h"][0].as_str(), asked["#c"][0].as_str(), asked["limit"].as_i64()), (Some(cid.as_str()), Some("feed"), Some(10)));

    let mut comments = Vec::new();
    let mut before = None;
    for want in [10, 10, 5] {
        let page = reader.svc.thread(&reader.keys, &cid, &first, 10, before).await.unwrap_or_else(|e| panic!("thread: {e}"));
        assert_eq!(page.items.len(), want);
        let texts: Vec<String> = page.items.iter().filter_map(|i| i.text.clone()).collect();
        comments.splice(0..0, texts);
        before = page.next_before;
    }
    assert_eq!(comments, (0..25).map(|i| format!("comment {i}")).collect::<Vec<_>>(), "oldest first, every one once");

    // A device that heard the journal and not the feed asks the relays
    // for the page and reads what they send.
    let other = w.device(reader.keys.clone()).await;
    other.hold(KIND_CONTENT, true);
    w.pump(&[&owner, &other]).await;
    let page = history(&other, &cid, 10, None).await;
    assert!(page.items.is_empty() && page.asked);
    // The newest of the stream are comments: the asks walk back to posts.
    let mut got = 0;
    for _ in 0..6 {
        w.pump(&[&other]).await;
        let page = history(&other, &cid, 10, None).await;
        got = page.items.len();
        if !page.asked {
            break;
        }
    }
    assert_eq!(got, 10);
}

#[tokio::test]
async fn a_file_goes_up_with_a_host_down_and_comes_back_whole() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2, H3]).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("photo.jpg");
    let bytes: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &bytes).unwrap_or_else(|e| panic!("{e}"));
    // A reader writes in words only.
    assert!(reader.svc.media_placeholder(&reader.keys, &cid, None, None, &path, None).await.is_err());
    w.hosts.set_down(H2, true);
    let (row, _) = owner.svc.media_placeholder(&owner.keys, &cid, None, None, &path, Some("a photo")).await.unwrap_or_else(|e| panic!("placeholder: {e}"));
    assert_eq!(owner.svc.uploads(&cid).await.unwrap_or_default().len(), 1);
    let seen = std::sync::Mutex::new((0u64, String::new()));
    let (id, _) = owner
        .svc
        .media_send(
            &owner.keys,
            &row.id,
            None,
            &crate::media::Sink(|p: crate::media::Progress| {
                assert!(p.done_bytes <= p.total_bytes);
                assert_eq!((p.direction.as_str(), p.chat_id.as_deref()), ("up", Some(cid.as_str())));
                *lock(&seen) = (p.done_bytes, p.status);
            }),
        )
        .await
        .unwrap_or_else(|e| panic!("send: {e}"));
    assert_eq!(*lock(&seen), (bytes.len() as u64, "done".to_string()), "the last word of the transfer, as a chat's");
    assert!(owner.svc.uploads(&cid).await.unwrap_or_default().is_empty(), "the placeholder gave way");
    w.pump(&all).await;
    let page = history(&reader, &cid, 10, None).await;
    let item = page.items.iter().find(|i| i.id == id).unwrap_or_else(|| panic!("the post"));
    assert_eq!(item.text.as_deref(), Some("a photo"));
    let d = messenger_media::MediaDescriptor::from_envelope(&Envelope::parse(item.media_json.as_deref().unwrap_or("")).unwrap_or_else(|e| panic!("{e}"))).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(d.servers, vec![H1.to_string(), H3.to_string()], "the hosts that hold every chunk");
    // Another host down when it is fetched: the third has it.
    w.hosts.set_down(H1, true);
    let out = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(reader.svc.media_data_url(&cid, &id, out.path()).await.unwrap_or_else(|e| panic!("{e}")), None, "nothing to show before the fetch");
    let got = reader.svc.media_fetch(&cid, &id, out.path(), &crate::media::Sink(|_| {})).await.unwrap_or_else(|e| panic!("fetch: {e}"));
    assert_eq!(std::fs::read(got).unwrap_or_default(), bytes);
    // Fetched, the picture is handed to the page whole, as a chat's is.
    let shown = reader.svc.media_data_url(&cid, &id, out.path()).await.unwrap_or_else(|e| panic!("{e}")).unwrap_or_default();
    assert!(shown.starts_with("data:image/jpeg;base64,"), "{}", shown.get(..40).unwrap_or(""));
    assert!(reader.svc.media_data_url(&cid, &"0".repeat(64), out.path()).await.is_err(), "no such message");
}

/// Files picked together go as an album: each post carries the same
/// `batch` in its descriptor, kept with the row across a failed send; the
/// caption goes with the last; a batch that is not one is refused.
#[tokio::test]
async fn files_picked_together_carry_one_batch() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let batch = "0b5d7c1e-4a2f-4c3b-9d8e-123456789abc";
    assert!(owner.svc.media_placeholder_in(&owner.keys, &cid, None, None, None, &dir.path().join("x.jpg"), None, Some("no")).await.is_err(), "too short a batch");
    let mut ids = Vec::new();
    for (i, caption) in [(0u8, None), (1, Some("two pictures"))] {
        let path = dir.path().join(format!("p{i}.jpg"));
        std::fs::write(&path, vec![i; 10_000]).unwrap_or_else(|e| panic!("{e}"));
        let (row, _) = owner.svc.media_placeholder_in(&owner.keys, &cid, None, None, None, &path, caption, Some(batch)).await.unwrap_or_else(|e| panic!("placeholder: {e}"));
        assert_eq!(owner.svc.uploads(&cid).await.unwrap_or_default().iter().find(|u| u.id == row.id).and_then(|u| u.batch.clone()).as_deref(), Some(batch), "kept with the row");
        let (id, _) = owner.svc.media_send(&owner.keys, &row.id, None, &crate::media::Sink(|_| {})).await.unwrap_or_else(|e| panic!("send: {e}"));
        ids.push(id);
    }
    w.pump(&all).await;
    let page = history(&reader, &cid, 10, None).await;
    for (i, id) in ids.iter().enumerate() {
        let item = page.items.iter().find(|x| &x.id == id).unwrap_or_else(|| panic!("post {i}"));
        let d = messenger_media::MediaDescriptor::from_envelope(&Envelope::parse(item.media_json.as_deref().unwrap_or("")).unwrap_or_else(|e| panic!("{e}"))).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(d.batch.as_deref(), Some(batch), "post {i}");
        assert_eq!(d.caption.as_deref(), if i == 1 { Some("two pictures") } else { None });
    }
}

#[tokio::test]
async fn a_page_marks_my_own_reactions() {
    let w = World::new();
    let owner = w.node().await;
    let a = w.node().await;
    let all = [&owner, &a];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &a, &cid, &all).await;
    w.clock.advance(300);
    tick(&owner).await;
    w.pump(&all).await;
    let (post, _) = owner.svc.send_post(&owner.keys, &cid, "the post", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    for (n, emoji) in [(&a, "👍"), (&owner, "👍"), (&owner, "🔥")] {
        w.clock.advance(1);
        n.svc.send_reaction(&n.keys, &cid, &post, emoji, true).await.unwrap_or_else(|e| panic!("react: {e}"));
        w.pump(&all).await;
    }
    let seen = |page: FeedPage| page.items.into_iter().find(|i| i.id == post).map(|i| i.reactions).unwrap_or_default();
    let fire_and_thumb = |thumb: bool, fire: bool| vec![("👍".to_string(), 2, thumb), ("🔥".to_string(), 1, fire)];
    assert_eq!(seen(history(&a, &cid, 10, None).await), fire_and_thumb(true, false));
    assert_eq!(seen(history(&owner, &cid, 10, None).await), fire_and_thumb(true, true));
    w.clock.advance(1);
    a.svc.send_reaction(&a.keys, &cid, &post, "👍", false).await.unwrap_or_else(|e| panic!("unreact: {e}"));
    w.pump(&all).await;
    assert_eq!(seen(history(&a, &cid, 10, None).await), vec![("👍".to_string(), 1, false), ("🔥".to_string(), 1, false)]);
}

/// A reaction set and taken back within a second (a bot's 👀 after its
/// answer), and the second relay's copy of the setting coming after the
/// first relay's retraction: the post stays without it (живой запуск
/// 2026-10-24).
#[tokio::test]
async fn a_reaction_taken_back_is_not_put_back_by_a_copy_heard_later() {
    let w = World::new();
    let owner = w.node().await;
    let a = w.node().await;
    let all = [&owner, &a];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &a, &cid, &all).await;
    w.clock.advance(300);
    tick(&owner).await;
    w.pump(&all).await;
    let (post, _) = owner.svc.send_post(&owner.keys, &cid, "the post", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    let before = w.relay.of_kind(KIND_CONTENT).len();
    a.svc.send_reaction(&a.keys, &cid, &post, "👀", true).await.unwrap_or_else(|e| panic!("react: {e}"));
    w.pump(&all).await;
    let seen = |page: FeedPage| page.items.into_iter().find(|i| i.id == post).map(|i| i.reactions).unwrap_or_default();
    assert_eq!(seen(history(&owner, &cid, 10, None).await), vec![("👀".to_string(), 1, false)]);
    let setting = w.relay.of_kind(KIND_CONTENT).get(before).cloned().unwrap_or_else(|| panic!("the reaction on the relay"));
    a.svc.send_reaction(&a.keys, &cid, &post, "👀", false).await.unwrap_or_else(|e| panic!("unreact: {e}"));
    w.pump(&all).await;
    assert_eq!(seen(history(&owner, &cid, 10, None).await), vec![], "taken back");
    owner.svc.on_event(&owner.keys, &setting, true).await.unwrap_or_else(|e| panic!("the copy heard again: {e}"));
    assert_eq!(seen(history(&owner, &cid, 10, None).await), vec![], "the copy of the setting puts nothing back");
}

#[tokio::test]
async fn a_post_in_markdown_keeps_its_format_through_an_edit() {
    let w = World::new();
    let owner = w.node().await;
    let a = w.node().await;
    let all = [&owner, &a];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &a, &cid, &all).await;
    w.clock.advance(300);
    tick(&owner).await;
    w.pump(&all).await;
    let (md, _) = owner.svc.send_post(&owner.keys, &cid, "**bold** words", TextFormat::Markdown).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.clock.advance(1);
    let (plain, _) = owner.svc.send_post(&owner.keys, &cid, "**as typed**", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    let item = |page: &FeedPage, id: &str| page.items.iter().find(|i| i.id == id).cloned().unwrap_or_else(|| panic!("the post"));
    let page = history(&a, &cid, 10, None).await;
    assert_eq!(item(&page, &md).fmt.as_deref(), Some("md"));
    assert!(item(&page, &md).rich.is_some_and(|b| !b.is_empty()), "the blocks of a chat's Markdown");
    assert_eq!((item(&page, &plain).fmt, item(&page, &plain).rich), (None, None));
    // An edit says its own format: the plain post becomes Markdown, the other plain.
    w.clock.advance(1);
    owner.svc.send_edit(&owner.keys, &cid, &plain, "*now* Markdown", TextFormat::Markdown).await.unwrap_or_else(|e| panic!("edit: {e}"));
    owner.svc.send_edit(&owner.keys, &cid, &md, "plain now", TextFormat::Plain).await.unwrap_or_else(|e| panic!("edit: {e}"));
    w.pump(&all).await;
    let page = history(&a, &cid, 10, None).await;
    assert_eq!(item(&page, &plain).fmt.as_deref(), Some("md"));
    assert_eq!((item(&page, &md).fmt, item(&page, &md).text.as_deref().map(str::to_string)), (None, Some("plain now".to_string())));
}

#[tokio::test]
async fn premoderation_holds_until_a_moderator_approves_and_a_rejection_hides() {
    let w = World::new();
    let owner = w.node().await;
    let (a, b) = (w.node().await, w.node().await);
    let all = [&owner, &a, &b];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &a, &cid, &all).await;
    join_by_link(&w, &owner, &b, &cid, &all).await;
    w.clock.advance(300);
    tick(&owner).await;
    // Posts are by authors (stage 1): `a` writes as one, held as everyone below a moderator.
    owner.svc.act(&owner.keys, &cid, OpBody::SetRole { who: pk(&a), role: Role::Author }).await.unwrap_or_else(|e| panic!("{e}"));
    owner.svc.act(&owner.keys, &cid, Settings { premod: Some(Premod::All), ..Settings::default() }.body()).await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&all).await;
    w.clock.advance(1);
    let (held, _) = a.svc.send_post(&a.keys, &cid, "held", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.clock.advance(1);
    let (bad, _) = a.svc.send_post(&a.keys, &cid, "bad", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    let texts = |p: FeedPage| p.items.into_iter().filter_map(|i| i.text).collect::<Vec<_>>();
    assert!(texts(history(&b, &cid, 10, None).await).is_empty(), "held from the readers");
    assert_eq!(texts(history(&a, &cid, 10, None).await), vec!["held".to_string(), "bad".to_string()], "the author sees their own, oldest first");
    let (queue, total) = owner.svc.queue(&owner.keys, &cid, None, None, 10).await.unwrap_or_else(|e| panic!("queue: {e}"));
    assert_eq!((queue.len(), total, queue[0].more_from_author), (1, 2, 1), "one line per author");
    let author = a.keys.public_key().to_hex();
    let (theirs, total) = owner.svc.queue(&owner.keys, &cid, Some(&author), None, 10).await.unwrap_or_else(|e| panic!("queue: {e}"));
    assert_eq!(theirs.iter().map(|l| (l.id.clone(), l.more_from_author)).collect::<Vec<_>>(), vec![(bad.clone(), 0), (held.clone(), 1)], "one line per message, newest first");
    assert_eq!(total, 2);
    let (none, total) = owner.svc.queue(&owner.keys, &cid, Some(&b.keys.public_key().to_hex()), None, 10).await.unwrap_or_else(|e| panic!("queue: {e}"));
    assert!(none.is_empty() && total == 0, "nothing of another author");
    assert!(b.svc.queue(&b.keys, &cid, None, None, 10).await.is_err(), "for moderators");
    owner.svc.moderate(&owner.keys, &cid, crate::ModAction::Approve { target: held.clone(), version: String::new() }).await.unwrap_or_else(|e| panic!("approve: {e}"));
    owner.svc.moderate(&owner.keys, &cid, crate::ModAction::Reject { target: bad.clone() }).await.unwrap_or_else(|e| panic!("reject: {e}"));
    w.pump(&all).await;
    assert_eq!(texts(history(&b, &cid, 10, None).await), vec!["held".to_string()], "approved: shown; rejected: not");
    assert_eq!(owner.svc.queue(&owner.keys, &cid, None, None, 10).await.map(|q| q.1).unwrap_or(9), 0);
    let log = owner.svc.mod_log(&owner.keys, &cid, None, 50).await.unwrap_or_else(|e| panic!("log: {e}"));
    assert!(log.iter().any(|l| l.action == "approve" && l.target == held));
    assert!(log.iter().any(|l| l.action == "reject" && l.target == bad));
    assert!(log.iter().any(|l| l.action == "settings"));
}

#[tokio::test]
async fn invitations_stop_at_four_open_in_a_channel() {
    let w = World::new();
    let owner = w.node().await;
    let cid = create(&w, &owner, JoinMode::Invite, History::Open, &[H1, H2]).await;
    for _ in 0..4 {
        let guest = PubKey::parse(&nostr::key::Keys::generate().public_key().to_hex()).unwrap_or_else(|| panic!("key"));
        owner.svc.invite_create(&owner.keys, &cid, &guest).await.unwrap_or_else(|e| panic!("invite: {e}"));
    }
    let fifth = PubKey::parse(&nostr::key::Keys::generate().public_key().to_hex()).unwrap_or_else(|| panic!("key"));
    assert!(owner.svc.invite_create(&owner.keys, &cid, &fifth).await.is_err(), "the fifth open one");
    let status = owner.svc.invite_status(&cid).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!((status.of_channel, status.max_per_channel), (4, 4));
    let list = owner.svc.invite_list(&cid).await.unwrap_or_default();
    owner.svc.invite_revoke(&list[0].invite_id).await.unwrap_or_else(|e| panic!("{e}"));
    owner.svc.invite_create(&owner.keys, &cid, &fifth).await.unwrap_or_else(|e| panic!("one revoked, one more: {e}"));
}

// ─── Review 2026-10-22 ──────────────────────────────────────────────────────

/// The `create` of the channel, signed again by `forger` under its id:
/// dated back until its op id sorts before the real one, so that the
/// order of the rows (`received_at, op_id`) puts it first.
fn forged_create(cid: &str, real: &crate::op::Op, forger: &nostr::key::Keys) -> serde_json::Value {
    let me = PubKey::parse(&forger.public_key().to_hex()).unwrap_or_else(|| panic!("key"));
    for back in 0..512 {
        let op = crate::op::Op::new(cid, &me, vec![], real.created_at - back, real.body.clone());
        if op.id() < real.id() {
            return serde_json::to_value(wire::sign_op(forger, &op).unwrap_or_else(|e| panic!("{e}"))).unwrap_or_default();
        }
    }
    panic!("no op id below the real one")
}

#[tokio::test]
async fn a_create_under_the_id_of_a_channel_by_somebody_else_is_never_its_genesis() {
    let w = World::new();
    let owner = w.node().await;
    let (late, warned) = (w.node().await, w.node().await);
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    let real = owner.svc.log(&cid).await.ok().flatten().and_then(|l| l.ordered().next().cloned()).unwrap_or_else(|| panic!("create"));
    let forger = nostr::key::Keys::generate();
    // The forged create goes on the relay before everything of the channel.
    lock(&w.relay.events).insert(0, forged_create(&cid, &real, &forger));
    let link = owner.svc.share_link(&cid).await.unwrap_or_else(|e| panic!("link: {e}"));

    // Told the owner by the link before the journal comes: the forged
    // create is never kept.
    warned.svc.expect_owner(&cid, &owner.me).await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&[&warned]).await;
    assert_eq!(warned.svc.log(&cid).await.ok().flatten().and_then(|l| l.creator().cloned()), Some(owner.me.clone()));
    warned.svc.open_link(&warned.keys, &link, "").await.unwrap_or_else(|e| panic!("open link: {e}"));
    w.pump(&[&owner, &warned]).await;
    assert_eq!(membership(&warned, &cid).await, MEMBERSHIP_JOINED);

    // Not told: the forged one came first and is the genesis here, the
    // real one is never a second create. The link drops that journal and
    // asks for it again; the next try goes through.
    w.pump(&[&late]).await;
    assert_eq!(late.svc.log(&cid).await.ok().flatten().map(|l| l.len()), Some(1), "one create, the forged one; the real one not kept beside it");
    assert!(late.svc.open_link(&late.keys, &link, "").await.is_err(), "the journal is asked for again");
    w.pump(&[&late]).await;
    assert_eq!(late.svc.log(&cid).await.ok().flatten().and_then(|l| l.creator().cloned()), Some(owner.me.clone()));
    late.svc.open_link(&late.keys, &link, "").await.unwrap_or_else(|e| panic!("open link again: {e}"));
    w.pump(&[&owner, &late]).await;
    assert_eq!(membership(&late, &cid).await, MEMBERSHIP_JOINED);
}

#[tokio::test]
async fn a_reply_to_an_invitation_counts_for_its_own_channel_only() {
    let w = World::new();
    let owner = w.node().await;
    let guest = w.node().await;
    let all = [&owner, &guest];
    let a = create(&w, &owner, JoinMode::Invite, History::Open, &[H1, H2]).await;
    let b = create(&w, &owner, JoinMode::Invite, History::Open, &[H1, H2]).await;
    w.pump(&all).await;
    let (invite_id, _) = owner.svc.invite(&owner.keys, &a, &pk(&guest)).await.unwrap_or_else(|e| panic!("invite: {e}"));
    w.pump(&all).await;
    let before = keeper_inbox(&w, &owner, &b).await;
    // A yes to the invitation of `a`, sent as if it were of `b`.
    let reply = wire::InviteReply { invite_id: invite_id.clone(), community_id: b.clone(), accept: true };
    guest.svc.send_note(None, &b, &owner.me, wire::T_INVITE_REPLY, &reply, None).await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&all).await;
    assert_eq!(keeper_inbox(&w, &owner, &b).await, before, "no admission to the other channel");
    assert_eq!(membership(&guest, &b).await, crate::service::MEMBERSHIP_NONE);
    let row = db::channels::invite(owner.svc.db(), &invite_id).await.ok().flatten().unwrap_or_else(|| panic!("invite"));
    assert_eq!(row.status, "sent", "the invitation is still open");
}

#[tokio::test]
async fn operations_of_somebody_never_staff_are_not_kept_and_name_no_parent() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    let ops_before = db::channels::count_ops(reader.svc.db(), &cid).await.unwrap_or(0);
    // An outsider signs operations on top of the journal, dated far ahead.
    let outsider = nostr::key::Keys::generate();
    let them = PubKey::parse(&outsider.public_key().to_hex()).unwrap_or_else(|| panic!("key"));
    let log = owner.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("log"));
    for i in 0..20 {
        let op = log.next(&them, w.clock.now() + 1_000_000 + i, OpBody::SetMuted { who: pk(&reader), muted: true, until: None });
        inject(&w, &wire::sign_op(&outsider, &op).unwrap_or_else(|e| panic!("{e}")));
    }
    w.pump(&all).await;
    assert_eq!(db::channels::count_ops(reader.svc.db(), &cid).await.unwrap_or(0), ops_before, "nothing of theirs kept");
    let op = owner.svc.act(&owner.keys, &cid, OpBody::SetTrusted { who: pk(&reader), trusted: true }).await;
    assert!(op.is_ok());
    let log = owner.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("log"));
    let last = log.ordered().last().cloned().unwrap_or_else(|| panic!("op"));
    assert!(matches!(last.body, OpBody::SetTrusted { .. }));
    assert!(last.parents.iter().all(|p| log.get(p).is_some_and(|o| o.author == owner.me)), "the parents are the owner's own");
}

#[tokio::test]
async fn the_journal_in_memory_is_the_one_the_rows_make() {
    let w = World::new();
    let owner = w.node().await;
    let (moder, reader) = (w.node().await, w.node().await);
    let all = [&owner, &moder, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    owner.svc.act(&owner.keys, &cid, OpBody::SetRole { who: pk(&moder), role: Role::Moderator }).await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&all).await;
    moder.svc.act(&moder.keys, &cid, OpBody::SetMuted { who: pk(&reader), muted: true, until: None }).await.unwrap_or_else(|e| panic!("{e}"));
    owner.svc.act(&owner.keys, &cid, OpBody::Remove { who: pk(&moder) }).await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&all).await;
    for n in all {
        let held = n.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("log"));
        let rebuilt = n.svc.build_log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("rows"));
        let ids = |l: &crate::log::OpLog| l.ordered().map(crate::op::Op::id).collect::<Vec<_>>();
        assert_eq!(ids(&held), ids(&rebuilt));
        assert_eq!(held.state(), rebuilt.state());
        assert_eq!(held.heads(), rebuilt.heads());
    }
}

#[tokio::test]
async fn an_epoch_event_far_ahead_does_not_shut_out_live_content() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    // The keeper's key, stolen: an event of a far epoch on top of a far one.
    let kkeys = owner.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("keeper")).lock().await.keys().clone();
    let real = &w.relay.of_kind(KIND_EPOCH)[0];
    let mut content: crate::epoch::EpochContent = serde_json::from_str(real["content"].as_str().unwrap_or("")).unwrap_or_else(|e| panic!("{e}"));
    content.event.epoch = 4_000_000_000;
    content.event.prev_epoch = Some(3_999_999_999);
    inject(&w, &crate::epoch::sign_epoch(&kkeys, &cid, SCOPE_EVERYONE, &content).unwrap_or_else(|e| panic!("{e}")));
    w.pump(&all).await;
    w.clock.advance(1);
    owner.svc.send_post(&owner.keys, &cid, "still read", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    assert_eq!(post_texts(&reader, &cid).await, vec!["still read".to_string()]);
}

#[tokio::test]
async fn epochs_that_come_newest_first_are_all_taken() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    reader.hold(KIND_EPOCH, true);
    for _ in 0..3 {
        owner.svc.act(&owner.keys, &cid, OpBody::RotateNow { scope_id: None }).await.unwrap_or_else(|e| panic!("{e}"));
        w.pump(&all).await;
    }
    assert_eq!(w.relay.epochs(), vec![1, 2, 3, 4]);
    let mut events = w.relay.of_kind(KIND_EPOCH);
    events.reverse();
    for ev in events {
        reader.svc.on_event(&reader.keys, &ev, false).await.unwrap_or_else(|e| panic!("{e}"));
    }
    reader.svc.run_fetches(&reader.keys).await;
    for e in 2..=4 {
        assert!(has_key(&reader, &cid, e).await, "epoch {e}");
    }
    assert_eq!(db::channels::count_pending(reader.svc.db(), &cid).await.unwrap_or(9), 0, "nothing left aside");
}

#[tokio::test]
async fn an_operation_dated_years_ahead_is_refused_and_moves_no_head() {
    let w = World::new();
    let owner = w.node().await;
    let (moder, reader) = (w.node().await, w.node().await);
    let all = [&owner, &moder, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    owner.svc.act(&owner.keys, &cid, OpBody::SetRole { who: pk(&moder), role: Role::Moderator }).await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&all).await;
    // A moderator's operation dated in ten years.
    let log = moder.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("log"));
    let ahead = log.next(&moder.me, w.clock.now() + 10 * 365 * 24 * 3600, OpBody::SetTrusted { who: pk(&reader), trusted: true });
    inject(&w, &wire::sign_op(&moder.keys, &ahead).unwrap_or_else(|e| panic!("{e}")));
    w.pump(&all).await;
    for n in [&owner, &reader] {
        assert!(!n.svc.log(&cid).await.ok().flatten().is_some_and(|l| l.contains(&ahead.id())), "not taken");
    }
    // A timed mute set now is dated now and in force.
    let until = w.clock.now() + 3600;
    owner.svc.act(&owner.keys, &cid, OpBody::SetMuted { who: pk(&reader), muted: true, until: Some(until) }).await.unwrap_or_else(|e| panic!("mute: {e}"));
    let log = owner.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("log"));
    let mute = log.ordered().last().cloned().unwrap_or_else(|| panic!("op"));
    assert!(mute.created_at <= w.clock.now() + wire::SKEW_SECS);
    assert!(log.state().can_write_at(&pk(&reader), Some(&crate::state::Reader { since: 0 }), crate::state::Content::Comment, w.clock.now()).is_err(), "muted now");
}

#[tokio::test]
async fn a_comment_written_before_the_rules_tightened_stands_when_caught_up_later() {
    let w = World::new();
    let owner = w.node().await;
    let (a, late) = (w.node().await, w.node().await);
    let all = [&owner, &a, &late];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &a, &cid, &all).await;
    join_by_link(&w, &owner, &late, &cid, &all).await;
    w.clock.advance(300);
    tick(&owner).await;
    w.pump(&all).await;
    w.clock.advance(1);
    let (post, _) = owner.svc.send_post(&owner.keys, &cid, "the post", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    // `late` hears the feed only after the journal moved on.
    late.hold(KIND_CONTENT, true);
    w.clock.advance(1);
    a.svc.send_comment(&a.keys, &cid, &post, None, "written while open", TextFormat::Plain).await.unwrap_or_else(|e| panic!("comment: {e}"));
    w.pump(&all).await;
    w.clock.advance(10);
    owner.svc.act(&owner.keys, &cid, Settings { comments: Some(crate::op::Comments::Off), ..Settings::default() }.body()).await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&all).await;
    late.hold(KIND_CONTENT, false);
    w.pump(&all).await;
    assert_eq!(thread_texts(&late, &cid, &post).await, vec!["written while open".to_string()]);
    // Written after the change: refused.
    w.clock.advance(1);
    let under = Links { thread: Some(post.clone()), parent: None, target: None };
    inject(&w, &forged_message(&a, &a.keys, &cid, under, "written after", w.clock.now()).await);
    w.pump(&all).await;
    assert_eq!(thread_texts(&late, &cid, &post).await, vec!["written while open".to_string()]);
}

/// Posts are by authors and the team, whatever `post_mode` an operation
/// names (stage 1, wire §3, 2026-10-22): a reader's post is refused at
/// home and on every other device; the reader comments.
#[tokio::test]
async fn a_reader_never_posts_whatever_post_mode_says() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    w.clock.advance(300);
    tick(&owner).await;
    owner.svc.act(&owner.keys, &cid, Settings { post_mode: Some(crate::op::PostMode::All), ..Settings::default() }.body()).await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&all).await;
    w.clock.advance(1);
    assert!(reader.svc.send_post(&reader.keys, &cid, "mine", TextFormat::Plain).await.is_err(), "refused at home");
    inject(&w, &forged_message(&reader, &reader.keys, &cid, Links::default(), "forged", w.clock.now()).await);
    w.pump(&all).await;
    assert!(post_texts(&owner, &cid).await.is_empty(), "and by everybody else");
    let (post, _) = owner.svc.send_post(&owner.keys, &cid, "the post", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    reader.svc.send_comment(&reader.keys, &cid, &post, None, "a comment", TextFormat::Plain).await.unwrap_or_else(|e| panic!("comment: {e}"));
    w.pump(&all).await;
    assert_eq!(thread_texts(&owner, &cid, &post).await, vec!["a comment".to_string()]);
}

/// An edit of `target` by `author`, sealed under the newest key `holder` has.
async fn forged_edit(holder: &Node, author: &nostr::key::Keys, cid: &str, target: &str, text: &str, at: i64) -> (nostr::prelude::Event, nostr::prelude::Event) {
    let key = newest(holder, cid).await.unwrap_or_else(|| panic!("no key"));
    let place = Place::feed(cid);
    let links = Links { target: Some(target.to_string()), ..Links::default() };
    let inner = wire::sign_inner(author, &place, &links, &Envelope::edit(target, text).encode(), at).unwrap_or_else(|e| panic!("{e}"));
    let payload = wire::Payload::event(wire::T_MSG, &inner).unwrap_or_else(|e| panic!("{e}"));
    let outer = wire::seal_content(&place, &key, links.outer_target(), &payload, at).unwrap_or_else(|e| panic!("{e}"));
    (inner, outer)
}

#[tokio::test]
async fn an_older_edit_sealed_again_does_not_bring_the_old_text_back() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    let (post, _) = owner.svc.send_post(&owner.keys, &cid, "v0", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    flush(&owner).await;
    w.clock.advance(1);
    let (inner1, first) = forged_edit(&owner, &owner.keys, &cid, &post, "v1", w.clock.now()).await;
    inject(&w, &first);
    w.clock.advance(1);
    let (_, second) = forged_edit(&owner, &owner.keys, &cid, &post, "v2", w.clock.now()).await;
    inject(&w, &second);
    w.pump(&all).await;
    assert_eq!(post_texts(&reader, &cid).await, vec!["v2".to_string()]);
    // A reader seals the first signed edit again: the text stays.
    let key = newest(&reader, &cid).await.unwrap_or_else(|| panic!("key"));
    let payload = wire::Payload::event(wire::T_MSG, &inner1).unwrap_or_else(|e| panic!("{e}"));
    let replay = wire::seal_content(&Place::feed(&cid), &key, Some(&post), &payload, w.clock.now()).unwrap_or_else(|e| panic!("{e}"));
    inject(&w, &replay);
    w.pump(&all).await;
    assert_eq!(post_texts(&reader, &cid).await, vec!["v2".to_string()]);
    assert_eq!(post_texts(&owner, &cid).await, vec!["v2".to_string()]);
    // A deleted post takes no edit.
    owner.svc.send_delete(&owner.keys, &cid, &post).await.unwrap_or_else(|e| panic!("delete: {e}"));
    w.clock.advance(1);
    let (_, late) = forged_edit(&owner, &owner.keys, &cid, &post, "back", w.clock.now()).await;
    inject(&w, &late);
    w.pump(&all).await;
    assert!(post_texts(&reader, &cid).await.is_empty(), "deleted stays deleted");
}

#[tokio::test]
async fn a_join_that_comes_before_its_admission_waits_for_it() {
    let w = World::new();
    let owner = w.node().await;
    let joiner = w.node().await;
    let all = [&owner, &joiner];
    let cid = create(&w, &owner, JoinMode::Request, History::Open, &[H1, H2]).await;
    w.pump(&all).await;
    let link = owner.svc.share_link(&cid).await.unwrap_or_default();
    joiner.svc.open_link(&joiner.keys, &link, "let me in").await.unwrap_or_else(|e| panic!("request: {e}"));
    w.pump(&all).await;
    // The keeper is asleep: the admission waits on the relay; the joiner
    // hears its copy and sends the join, which the keeper reads first.
    let keeper = inbox_of_keeper(&owner, &cid).await;
    let at = lock(&w.relay.inbox).get(keeper.as_hex()).map_or(0, Vec::len);
    owner.svc.approve_request(&owner.keys, &cid, &pk(&joiner)).await.unwrap_or_else(|e| panic!("approve: {e}"));
    flush(&owner).await;
    let admit = lock(&w.relay.inbox).get_mut(keeper.as_hex()).map(|l| l.remove(at)).unwrap_or_else(|| panic!("the admission"));
    w.pump(&all).await;
    assert_ne!(membership(&joiner, &cid).await, MEMBERSHIP_REJECTED, "not refused");
    assert!(newest(&joiner, &cid).await.is_none());
    // The admission comes: the join that waited goes through.
    lock(&w.relay.inbox).entry(keeper.as_hex().to_string()).or_default().push(admit);
    w.pump(&all).await;
    assert_eq!(membership(&joiner, &cid).await, MEMBERSHIP_JOINED);
    assert!(newest(&joiner, &cid).await.is_some());
}

#[tokio::test]
async fn a_reader_removed_just_before_a_new_link_never_reads_its_secret() {
    let w = World::new();
    let owner = w.node().await;
    let (gone, stays) = (w.node().await, w.node().await);
    let all = [&owner, &gone, &stays];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &gone, &cid, &all).await;
    join_by_link(&w, &owner, &stays, &cid, &all).await;
    w.clock.advance(10);
    owner.svc.act(&owner.keys, &cid, OpBody::Remove { who: pk(&gone) }).await.unwrap_or_else(|e| panic!("remove: {e}"));
    owner.svc.act(&owner.keys, &cid, OpBody::RotateLink { link_epoch: 1 }).await.unwrap_or_else(|e| panic!("rotate: {e}"));
    w.pump(&all).await;
    let secret = |n: &Node| {
        let (svc, cid) = (n.svc.clone(), cid.clone());
        async move { crate::secrets::links(svc.env.secrets.as_ref(), &cid).await.ok().and_then(|l| l.get(&1).map(|s| s.encode())) }
    };
    let made = secret(&owner).await;
    assert!(made.is_some());
    assert_eq!(secret(&stays).await, made, "a member reads the new link");
    assert_eq!(secret(&gone).await, None, "the one put out does not");
}

#[tokio::test]
async fn notes_to_people_go_to_their_own_inboxes_notes_to_the_keeper_to_the_channel() {
    let w = World::new();
    let owner = w.node().await;
    let (guest, asker, refused) = (w.node().await, w.node().await, w.node().await);
    let all = [&owner, &guest, &asker, &refused];
    let cid = create(&w, &owner, JoinMode::Request, History::Open, &[H1, H2]).await;
    w.pump(&all).await;
    let keeper = inbox_of_keeper(&owner, &cid).await;
    let link = owner.svc.share_link(&cid).await.unwrap_or_default();
    for n in [&asker, &refused] {
        n.svc.open_link(&n.keys, &link, "").await.unwrap_or_else(|e| panic!("request: {e}"));
    }
    w.pump(&all).await;
    owner.svc.invite(&owner.keys, &cid, &pk(&guest)).await.unwrap_or_else(|e| panic!("invite: {e}"));
    owner.svc.approve_request(&owner.keys, &cid, &pk(&asker)).await.unwrap_or_else(|e| panic!("approve: {e}"));
    owner.svc.reject_request(&owner.keys, &cid, &pk(&refused)).await.unwrap_or_else(|e| panic!("reject: {e}"));
    flush(&owner).await;
    let hints = lock(&owner.net.hints).clone();
    let to = |who: &PubKey| hints.iter().filter(|(r, _)| r == who.as_hex()).map(|(_, n)| *n).collect::<Vec<_>>();
    assert_eq!(to(&pk(&guest)), vec![0], "the invitation");
    assert_eq!(to(&pk(&asker)), vec![0], "the copy of the admission");
    assert_eq!(to(&pk(&refused)), vec![0], "the refusal");
    assert!(to(&keeper).iter().all(|n| *n > 0), "the keeper listens on the channel's relays");
    // The guest answers: to the moderator's own inbox too.
    w.pump(&all).await;
    let invite_id = db::channels::invites(guest.svc.db(), "in", &["received"]).await.unwrap_or_default().pop().map(|i| i.invite_id).unwrap_or_default();
    guest.svc.answer_invite(&invite_id, true).await.unwrap_or_else(|e| panic!("answer: {e}"));
    flush(&guest).await;
    assert_eq!(lock(&guest.net.hints).iter().filter(|(r, _)| r == owner.me.as_hex()).map(|(_, n)| *n).collect::<Vec<_>>(), vec![0]);
}

#[tokio::test]
async fn a_full_channel_refuses_newcomers_and_a_flood_of_joins_waits() {
    let w = World::new();
    let owner = w.node().await;
    let (a, b, c) = (w.node().await, w.node().await, w.node().await);
    let all = [&owner, &a, &b, &c];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    let k = owner.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("keeper"));
    k.lock().await.limits = crate::keeper::Limits { max_readers: 100, joins_per_minute: 1, ..Default::default() };
    join_by_link(&w, &owner, &a, &cid, &all).await;
    assert_eq!(membership(&a, &cid).await, MEMBERSHIP_JOINED);
    // A second join within the minute: not refused, sent again later.
    join_by_link(&w, &owner, &b, &cid, &all).await;
    assert_ne!(membership(&b, &cid).await, MEMBERSHIP_REJECTED);
    assert!(newest(&b, &cid).await.is_none());
    w.clock.advance(crate::service::JOIN_RETRY_SECS);
    tick(&b).await;
    w.pump(&all).await;
    assert_eq!(membership(&b, &cid).await, MEMBERSHIP_JOINED);
    // Full: refused at the door, and the cycle goes on.
    k.lock().await.limits = crate::keeper::Limits { max_readers: 3, joins_per_minute: 100, ..Default::default() };
    join_by_link(&w, &owner, &c, &cid, &all).await;
    assert_eq!(membership(&c, &cid).await, MEMBERSHIP_REJECTED);
    w.clock.advance(300);
    owner.svc.tick().await.unwrap_or_else(|e| panic!("tick: {e}"));
}

#[tokio::test]
async fn sealed_events_wait_only_for_channels_followed_and_epochs_that_can_come() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    let stranger = nostr::key::Keys::generate();
    let sealed = |cid: &str, epoch: u32| {
        let key = EpochKey { epoch, key: keydist::Key32([7u8; 32]) };
        let place = Place::feed(cid);
        let inner = wire::sign_inner(&stranger, &place, &Links::default(), &Envelope::text("junk").encode(), w.clock.now()).unwrap_or_else(|e| panic!("{e}"));
        let payload = wire::Payload::event(wire::T_MSG, &inner).unwrap_or_else(|e| panic!("{e}"));
        wire::seal_content(&place, &key, None, &payload, w.clock.now()).unwrap_or_else(|e| panic!("{e}"))
    };
    let other = "cd".repeat(32);
    inject(&w, &sealed(&other, 1));
    inject(&w, &sealed(&cid, 1_000));
    inject(&w, &sealed(&cid, 2));
    w.pump(&all).await;
    assert_eq!(db::channels::count_pending(reader.svc.db(), &other).await.unwrap_or(9), 0, "a channel not followed");
    assert_eq!(db::channels::count_pending(reader.svc.db(), &cid).await.unwrap_or(9), 1, "only the epoch that can still come");
}

#[tokio::test]
async fn invitations_before_the_journal_are_unverified_capped_and_dropped_when_it_disagrees() {
    let w = World::new();
    let owner = w.node().await;
    let (guest, spoofer) = (w.node().await, w.node().await);
    let cid = create(&w, &owner, JoinMode::Invite, History::Open, &[H1, H2]).await;
    let invite = |by: &Node, n: u8| wire::Invite {
        invite_id: format!("{n:02x}").repeat(16),
        community_id: cid.clone(),
        name: "News".into(),
        about: String::new(),
        picture: String::new(),
        relay: String::new(),
        members: 99,
        levels: 0,
        created_at: w.clock.now(),
        expires_at: w.clock.now() + 3600,
        owner: Some(by.me.clone()),
    };
    // Before the journal reaches the guest: kept, unverified, at most ten of one sender.
    for n in 0..12u8 {
        spoofer.svc.send_note(None, "", &guest.me, wire::T_INVITE, &invite(&spoofer, n), None).await.unwrap_or_else(|e| panic!("{e}"));
    }
    flush(&spoofer).await;
    guest.hold(wire::KIND_JOURNAL, true);
    w.pump(&[&guest]).await;
    let kept = db::channels::invites(guest.svc.db(), "in", &["received"]).await.unwrap_or_default();
    assert_eq!(kept.len(), crate::service::MAX_INVITES_PER_PERSON);
    assert!(kept.iter().all(|i| !i.verified));
    // The journal comes: the spoofer is no moderator, the invitations go.
    guest.hold(wire::KIND_JOURNAL, false);
    w.pump(&[&owner, &guest]).await;
    assert!(db::channels::invites(guest.svc.db(), "in", &[]).await.unwrap_or_default().is_empty());
    // One from the owner, with the journal here: verified.
    owner.svc.invite(&owner.keys, &cid, &pk(&guest)).await.unwrap_or_else(|e| panic!("invite: {e}"));
    w.pump(&[&owner, &guest]).await;
    let kept = db::channels::invites(guest.svc.db(), "in", &["received"]).await.unwrap_or_default();
    assert_eq!(kept.iter().map(|i| i.verified).collect::<Vec<_>>(), vec![true]);
}

#[tokio::test]
async fn a_snapshot_dated_ahead_is_refused_and_blocks_no_later_one() {
    let w = World::new();
    let owner = w.node().await;
    let a = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &a, &cid, &[&owner, &a]).await;
    let kkeys = owner.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("keeper")).lock().await.keys().clone();
    let latest = || async { a.svc.member(&a.keys).latest_snapshot(&cid).await.ok().flatten().map(|s| s.at) };
    let mut s = snapshots(&w).remove(0);
    let base = s.at;
    s.at = w.clock.now() + 1_000_000;
    let ahead = crate::snapshot::sign_snapshot(&kkeys, &s).unwrap_or_else(|e| panic!("{e}"));
    a.svc.on_event(&a.keys, &serde_json::to_value(&ahead).unwrap_or_default(), true).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(latest().await, Some(base), "not taken");
    s.at = w.clock.now() + 10;
    let next = crate::snapshot::sign_snapshot(&kkeys, &s).unwrap_or_else(|e| panic!("{e}"));
    a.svc.on_event(&a.keys, &serde_json::to_value(&next).unwrap_or_default(), true).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(latest().await, Some(s.at), "the next one is newer");
}

#[tokio::test]
async fn the_secret_of_a_forged_link_gives_way_to_the_keepers() {
    let w = World::new();
    let owner = w.node().await;
    let guest = w.node().await;
    let all = [&owner, &guest];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    w.pump(&all).await;
    // A link with a wrong secret: kept from the URL, the join refused.
    let real = owner.svc.share_link(&cid).await.unwrap_or_default();
    let forged = ChannelLink::parse(&real).map(|mut l| {
        l.secret = Some(LinkSecret::from_bytes(&[9u8; 32]).unwrap_or_else(|e| panic!("{e}")));
        l.encode()
    });
    guest.svc.open_link(&guest.keys, &forged.unwrap_or_default(), "").await.unwrap_or_else(|e| panic!("open: {e}"));
    w.pump(&all).await;
    assert_eq!(membership(&guest, &cid).await, MEMBERSHIP_REJECTED);
    // Let in another way: the keeper's issue brings the real secret.
    let (invite_id, _) = owner.svc.invite(&owner.keys, &cid, &pk(&guest)).await.unwrap_or_else(|e| panic!("invite: {e}"));
    w.pump(&all).await;
    guest.svc.answer_invite(&invite_id, true).await.unwrap_or_else(|e| panic!("answer: {e}"));
    w.pump(&all).await;
    assert_eq!(membership(&guest, &cid).await, MEMBERSHIP_JOINED);
    assert_eq!(guest.svc.share_link(&cid).await.ok(), Some(real), "the link handed on is the real one");
}

#[tokio::test]
async fn a_replayed_join_is_not_heard_twice_by_the_keeper() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    let keeper = inbox_of_keeper(&owner, &cid).await;
    let at = lock(&w.relay.inbox).get(keeper.as_hex()).map_or(0, Vec::len);
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    assert_eq!(membership(&reader, &cid).await, MEMBERSHIP_JOINED);
    let join = lock(&w.relay.inbox).get(keeper.as_hex()).and_then(|l| l.get(at).cloned()).unwrap_or_else(|| panic!("the join"));
    reader.svc.leave(&reader.keys, &cid).await.unwrap_or_else(|e| panic!("leave: {e}"));
    w.pump(&all).await;
    let to_reader = || lock(&w.relay.inbox).get(reader.me.as_hex()).map_or(0, Vec::len);
    let before = to_reader();
    // A relay hands the same join to the keeper again.
    lock(&w.relay.inbox).entry(keeper.as_hex().to_string()).or_default().push(join);
    w.pump(&all).await;
    assert_eq!(to_reader(), before, "no issue for a replayed join");
}

#[tokio::test]
async fn a_join_that_does_not_read_stops_no_keeper() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    w.clock.advance(10);
    owner.svc.act(&owner.keys, &cid, OpBody::Ban { who: pk(&reader), reason: None }).await.unwrap_or_else(|e| panic!("ban: {e}"));
    // A row of a join that does not read.
    db::channels::put_joining(owner.svc.db(), &cid, "not json", w.clock.now() - 1000, w.clock.now() - 1).await.unwrap_or_else(|e| panic!("{e}"));
    w.clock.advance(400);
    assert!(owner.svc.tick().await.is_ok());
    assert_eq!(w.relay.epochs(), vec![1, 2], "the ban's epoch went out");
}

/// A channel made while no relay takes its journal is here at once, kept
/// and pending (its page says so); its epoch waits for the journal to be
/// out, and once the relay takes it the channel is whole: the first epoch
/// on the relay, the owner's key here.
#[tokio::test]
async fn a_channel_whose_journal_no_relay_takes_is_pending_here_and_whole_once_it_does() {
    let w = World::new();
    let owner = w.node().await;
    w.relay.refuse(wire::KIND_JOURNAL, true);
    let (cid, out) = owner.svc.create(&owner.keys, new_channel(JoinMode::Open, History::Open, &[H1, H2])).await.unwrap_or_else(|e| panic!("create: {e}"));
    assert_eq!(out.pending.len(), 1, "the create is on its way");
    assert_eq!(membership(&owner, &cid).await, MEMBERSHIP_JOINED, "listed at once");
    assert_eq!(owner.svc.kept().await.unwrap_or_default(), vec![cid.clone()], "kept at once");
    let summary = owner.svc.outbox_summary(&cid).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!((summary.queued, summary.failed), (4, 0), "the journal, the card, the keeper's issue to me and the copy of my keys to my other devices wait");
    w.pump(&[&owner]).await;
    assert!(w.relay.of_kind(wire::KIND_JOURNAL).is_empty(), "refused");
    assert_eq!(w.relay.of_kind(wire::KIND_CARD).len(), 1, "the card went on its own");
    assert!(w.relay.epochs().is_empty(), "no epoch before the journal is on the relay");
    // The copy of my own keys to my other devices goes on its own.
    let issues_to_me = || lock(&w.relay.inbox).get(owner.me.as_hex()).map_or(0, |l| l.iter().filter_map(|x| crate::notes::open(&owner.keys, x).ok()).filter(|o| o.content.contains("\"t\":\"channel.issue\"")).count());
    assert_eq!(issues_to_me(), 0, "the issue waits behind the journal");
    let summary = owner.svc.outbox_summary(&cid).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(summary.queued, 2, "the journal and the issue behind it");
    assert!(summary.oldest_error.as_deref().is_some_and(|e| e.contains("refused")), "{summary:?}");
    // Once the relay takes it, the channel is made whole: tried again at
    // its time.
    w.relay.refuse(wire::KIND_JOURNAL, false);
    w.clock.advance(crate::keeper::OWED_FIRST_WAIT_SECS);
    w.pump(&[&owner]).await;
    assert_eq!(w.relay.of_kind(wire::KIND_JOURNAL).len(), 1);
    assert_eq!(w.relay.epochs(), vec![1], "the first epoch after the journal");
    assert_eq!(newest(&owner, &cid).await.map(|k| k.epoch), Some(1), "the owner holds the key");
    assert_eq!(owner.svc.outbox_summary(&cid).await.unwrap_or_else(|e| panic!("{e}")).queued, 0);
    assert!(owner.errors().is_empty(), "{:?}", owner.errors());
}

/// A post comes back before any relay has it and goes out exactly once
/// later: refused, it is here (pending for the page), tried again at its
/// time, on the relay once, and the echo of the relay is not a second copy.
#[tokio::test]
async fn a_post_returns_before_the_relay_and_goes_out_exactly_once() {
    let w = World::new();
    let owner = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    let posts_on_relay = || w.relay.of_kind(KIND_CONTENT).len();
    let before = posts_on_relay();
    w.relay.refuse(KIND_CONTENT, true);
    let (id, out) = owner.svc.send_post(&owner.keys, &cid, "hello", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    assert_eq!(out.pending, vec![id.clone()]);
    assert_eq!(post_texts(&owner, &cid).await, vec!["hello".to_string()], "here at once");
    let summary = || async { owner.svc.outbox_summary(&cid).await.unwrap_or_else(|e| panic!("{e}")) };
    assert_eq!(summary().await.queued, 1);
    assert!(owner.svc.outbox_pending(&cid).await.unwrap_or_default().contains(&id), "pending for the page");
    w.pump(&[&owner]).await;
    assert_eq!(posts_on_relay(), before, "refused: not on the relay");
    let s = summary().await;
    assert_eq!((s.queued, s.failed), (1, 0));
    assert!(s.oldest_error.is_some());
    w.relay.refuse(KIND_CONTENT, false);
    w.pump(&[&owner]).await;
    assert_eq!(posts_on_relay(), before, "not before its time");
    w.clock.advance(crate::keeper::OWED_FIRST_WAIT_SECS);
    let f = flush(&owner).await;
    assert_eq!(f.sent, 1);
    assert_eq!(f.ui.len(), 1, "the page hears it went: {:?}", f.ui);
    assert_eq!(f.ui[0].payload["state"], "sent");
    assert_eq!(posts_on_relay(), before + 1, "exactly one on the relay");
    assert!(owner.svc.outbox_pending(&cid).await.unwrap_or_default().is_empty());
    w.pump(&[&owner]).await;
    assert_eq!(posts_on_relay(), before + 1, "nothing more goes out");
    assert_eq!(post_texts(&owner, &cid).await, vec!["hello".to_string()], "the echo of the relay is kept once");
    assert_eq!(db::content::posts(owner.svc.db(), &cid, None, 100).await.unwrap_or_default().len(), 1);
}

/// A note of the keeper waits for the operation it follows: the issue of
/// a section goes to the reader only after `scope_add` and `set_levels`
/// are on the relay (wire §14.5: an issue of an unknown scope is not
/// taken, and never sent again).
#[tokio::test]
async fn the_keepers_notes_wait_for_the_operation_they_follow() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    let wraps_to_reader = || lock(&w.relay.inbox).get(reader.me.as_hex()).map_or(0, Vec::len);
    let before = wraps_to_reader();
    w.relay.refuse(wire::KIND_JOURNAL, true);
    let scope_id = owner.svc.scope_add(&owner.keys, &cid, "Gold", keydist::SchemeId::SdV1, History::Open).await.unwrap_or_else(|e| panic!("scope: {e}")).0;
    owner.svc.level_set(&owner.keys, &cid, None, "gold", vec![scope_id.clone()], crate::LevelCan { comment: true, discuss: false, post: false }).await.unwrap_or_else(|e| panic!("level: {e}"));
    owner.svc.set_levels(&owner.keys, &cid, &pk(&reader), &[1]).await.unwrap_or_else(|e| panic!("levels: {e}"));
    w.pump(&all).await;
    assert_eq!(wraps_to_reader(), before, "no issue to the reader before its operations are on the relay");
    assert!(w.relay.of_kind(wire::KIND_JOURNAL).len() < 4, "the operations wait");
    w.relay.refuse(wire::KIND_JOURNAL, false);
    w.clock.advance(crate::keeper::OWED_FIRST_WAIT_SECS);
    w.pump(&all).await;
    let on_relay = w.relay.of_kind(wire::KIND_JOURNAL).len();
    assert_eq!(on_relay, 4, "create, scope_add, level_set, set_levels");
    // The issue is wrapped after the last operation reached the relay.
    assert!(wraps_to_reader() > before, "the issue of the section came");
    let relay_order: Vec<usize> = lock(&w.relay.events).iter().enumerate().filter(|(_, e)| e["kind"] == u64::from(wire::KIND_JOURNAL)).map(|(i, _)| i).collect();
    let first_wrap_after = lock(&w.relay.inbox).get(reader.me.as_hex()).map_or(0, Vec::len);
    assert!(first_wrap_after > before && relay_order.len() == 4);
    assert!(reader.svc.member(&reader.keys).newest_key(&cid, &scope_id).await.unwrap_or_else(|e| panic!("{e}")).is_some(), "the reader holds the key of the section");
    for n in all {
        assert!(n.errors().is_empty(), "{:?}", n.errors());
    }
}

/// The outbox resumes after a restart: what a run left queued, the next
/// run on the same database sends, once; the old run sends nothing more.
#[tokio::test]
async fn the_outbox_resumes_after_a_restart() {
    let w = World::new();
    let owner = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    let content = || w.relay.of_kind(KIND_CONTENT).len();
    let before = content();
    w.relay.refuse(KIND_CONTENT, true);
    owner.svc.send_post(&owner.keys, &cid, "kept", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&[&owner]).await;
    assert_eq!(content(), before);
    w.relay.refuse(KIND_CONTENT, false);
    // The app comes back on the same database.
    let again = w.device_on(owner.keys.clone(), owner.svc.db().clone());
    again.skip_all(&w.relay);
    w.clock.advance(crate::keeper::OWED_FIRST_WAIT_SECS);
    w.pump(&[&again]).await;
    assert_eq!(content(), before + 1, "delivered once by the new run");
    assert_eq!(again.svc.outbox_summary(&cid).await.unwrap_or_else(|e| panic!("{e}")).queued, 0);
    assert_eq!(flush(&owner).await.sent, 0, "the old run has nothing more to send");
    assert_eq!(content(), before + 1);
}

/// A live note to a channel kept here (a join) is queued and the turn
/// asked for: `on_wrap` makes no round trip to the hosts (живой запуск
/// 2026-10-23: every wrap held the loop 40–96 s in the keeper's turn), the
/// turn after it lets the reader in; the same note again asks for nothing.
#[tokio::test]
async fn a_live_note_to_a_kept_channel_makes_no_round_trip_to_the_hosts() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    let link = owner.svc.share_link(&cid).await.unwrap_or_else(|e| panic!("link: {e}"));
    w.pump(&[&reader]).await;
    reader.svc.open_link(&reader.keys, &link, "").await.unwrap_or_else(|e| panic!("open link: {e}"));
    flush(&reader).await;
    let to = inbox_of_keeper(&owner, &cid).await;
    let join = lock(&w.relay.inbox).get(to.as_hex()).and_then(|l| l.last().cloned()).unwrap_or_else(|| panic!("the join"));
    let round_trips = || w.hosts.puts.load(Ordering::SeqCst) + w.hosts.gets.load(Ordering::SeqCst) + w.hosts.deletes.load(Ordering::SeqCst);
    let (trips, sent) = (round_trips(), owner.net.sent.load(Ordering::SeqCst));
    let out = owner.svc.on_wrap(&owner.keys, &join).await.unwrap_or_else(|e| panic!("{e}")).unwrap_or_else(|| panic!("a note of the channel"));
    assert_eq!(round_trips(), trips, "no host was asked");
    assert_eq!(owner.net.sent.load(Ordering::SeqCst), sent, "nothing went out");
    assert_eq!(out.wake, Some(u64::try_from(w.clock.now()).unwrap_or(0)), "the turn is asked for now");
    assert!(owner.svc.take_turn_asked());
    assert_eq!(db::keeper::queued_count(owner.svc.db(), &cid).await.unwrap_or(0), 1, "queued for the turn");
    let out = owner.svc.on_wrap(&owner.keys, &join).await.unwrap_or_else(|e| panic!("{e}")).unwrap_or_default();
    assert!(out.wake.is_none() && !owner.svc.take_turn_asked(), "heard already: no turn asked");
    owner.skip_inboxes(&w.relay);
    tick(&owner).await;
    w.pump(&[&owner, &reader]).await;
    assert_eq!(membership(&reader, &cid).await, MEMBERSHIP_JOINED);
    assert!(newest(&reader, &cid).await.is_some());
}

#[tokio::test]
async fn a_file_given_up_while_it_uploads_sends_nothing_and_a_failed_one_is_retried() {
    let w = World::new();
    let owner = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("photo.jpg");
    std::fs::write(&path, vec![7u8; 300_000]).unwrap_or_else(|e| panic!("{e}"));
    let posts = || w.relay.of_kind(KIND_CONTENT).len();
    let before = posts();
    // Given up as it starts: the cancel is heard by the run at once.
    let (row, _) = owner.svc.media_placeholder(&owner.keys, &cid, None, None, &path, None).await.unwrap_or_else(|e| panic!("placeholder: {e}"));
    let svc = owner.svc.clone();
    let id = row.id.clone();
    let cancel = crate::media::Sink(move |p: crate::media::Progress| {
        if p.status == "running" {
            if let Some(c) = svc.transfers.lock().ok().and_then(|t| t.get(&id).cloned()) {
                c.ask(messenger_media::control::CANCEL);
            }
        }
    });
    let sent = owner.svc.media_send(&owner.keys, &row.id, None, &cancel).await;
    assert!(sent.is_err_and(|e| e.to_string().contains(crate::media::UPLOAD_CANCELLED)));
    assert_eq!(posts(), before, "no message went out");
    assert!(owner.svc.uploads(&cid).await.unwrap_or_default().is_empty(), "and no row came back");
    // Paused before a chunk went: the row waits, and goes on when asked.
    let (row, _) = owner.svc.media_placeholder(&owner.keys, &cid, None, None, &path, None).await.unwrap_or_else(|e| panic!("placeholder: {e}"));
    let svc = owner.svc.clone();
    let id = row.id.clone();
    let pause = crate::media::Sink(move |p: crate::media::Progress| {
        if p.status == "running" {
            svc.media_pause(&id);
        }
    });
    let sent = owner.svc.media_send(&owner.keys, &row.id, None, &pause).await;
    assert!(sent.is_err_and(|e| e.to_string().contains(crate::media::UPLOAD_PAUSED)));
    assert_eq!(owner.svc.uploads(&cid).await.unwrap_or_default()[0].status, db::content::UPLOAD_PAUSED);
    owner.svc.media_retry(&row.id).await.unwrap_or_else(|e| panic!("resume: {e}"));
    // A run that ended mid-upload is found at the next start, to go on.
    let left: Vec<String> = owner.svc.interrupted_uploads().await.unwrap_or_default().into_iter().map(|u| u.id).collect();
    assert_eq!(left, vec![row.id.clone()]);
    owner.svc.media_send(&owner.keys, &row.id, None, &crate::media::Sink(|_| {})).await.unwrap_or_else(|e| panic!("send: {e}"));
    w.pump(&[&owner]).await;
    assert_eq!(posts(), before + 1);
}

/// A photo goes made smaller unless it goes as it is, a picture carries
/// its size and its preview, a video the frame the page took: as in a chat.
#[tokio::test]
async fn a_photo_is_made_smaller_and_files_carry_their_previews() {
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1]).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let outgoing = dir.path().join("outgoing");
    let (pw, ph) = (2000u32, 1500u32);
    let rgba: Vec<u8> = (0..pw * ph).flat_map(|i| [(i % 251) as u8, (i * 7 % 253) as u8, (i % 13) as u8 * 19, 255]).collect();
    // A camera's photo: a large JPEG (a PNG with alpha goes as it is).
    let png = messenger_avatar::png_of_rgba(pw, ph, &rgba).unwrap_or_else(|e| panic!("png: {e:?}"));
    let decoded = messenger_avatar::decode(&png).unwrap_or_else(|e| panic!("{e:?}"));
    let photo = messenger_avatar::photo_jpeg(&decoded, pw, 95);
    let picked = dir.path().join("photo.jpeg");
    std::fs::write(&picked, &photo).unwrap_or_else(|e| panic!("{e}"));
    let video = dir.path().join("clip.mp4");
    std::fs::write(&video, vec![9u8; 50_000]).unwrap_or_else(|e| panic!("{e}"));
    let frame = messenger_avatar::photo_jpeg(&decoded, 480, 80);
    let poster = crate::media::Poster { jpeg: B64.encode(&frame), width: 1920, height: 1080 };
    let mut sent = Vec::new();
    for (path, original, poster) in [(&picked, false, None), (&picked, true, None), (&video, false, Some(poster))] {
        let file = crate::media::NewFile { original, poster, ..crate::media::NewFile::at(path) };
        let (row, _) = owner.svc.media_placeholder_file(&owner.keys, &cid, file).await.unwrap_or_else(|e| panic!("placeholder: {e}"));
        let stages = std::sync::Mutex::new(Vec::new());
        let (id, _) = owner
            .svc
            .media_send(&owner.keys, &row.id, Some(&outgoing), &crate::media::Sink(|p: crate::media::Progress| lock(&stages).push(p.stage)))
            .await
            .unwrap_or_else(|e| panic!("send: {e}"));
        let prepared = lock(&stages).contains(&messenger_media::TransferStage::Preparing);
        assert_eq!(prepared, path == &picked && !original, "only a photo made smaller is prepared");
        sent.push(id);
    }
    assert_eq!(std::fs::read_dir(&outgoing).map(|d| d.count()).unwrap_or(0), 1, "the smaller copy stays: my post shows it");
    assert!(picked.exists(), "the user's own file stays");
    w.pump(&all).await;
    let page = history(&reader, &cid, 10, None).await;
    let d = |id: &str| {
        let item = page.items.iter().find(|i| i.id == id).unwrap_or_else(|| panic!("the post"));
        messenger_media::MediaDescriptor::from_envelope(&Envelope::parse(item.media_json.as_deref().unwrap_or("")).unwrap_or_else(|e| panic!("{e}"))).unwrap_or_else(|e| panic!("{e}"))
    };
    let smaller = d(&sent[0]);
    assert_eq!((smaller.name.as_str(), smaller.mime.as_str(), smaller.dim), ("photo.jpg", "image/jpeg", Some((1280, 960))));
    assert!(smaller.size < photo.len() as u64);
    assert!(smaller.thumb.as_deref().is_some_and(messenger_media::descriptor::valid_thumb), "a preview of the picture");
    let original = d(&sent[1]);
    assert_eq!((original.name.as_str(), original.size, original.dim), ("photo.jpeg", photo.len() as u64, Some((pw, ph))));
    assert!(original.thumb.is_some());
    let clip = d(&sent[2]);
    assert_eq!((clip.kind, clip.dim), (messenger_media::MediaKind::Video, Some((1920, 1080))));
    assert!(clip.thumb.as_deref().is_some_and(messenger_media::descriptor::valid_thumb), "the frame as a preview");
}

/// A voice message and a video circle recorded in the app go as a chat's
/// do (2026-10-23): a post of the owner and a comment of a reader, each a
/// message of the kind `voice` or `circle` with its length (and a voice's
/// outline), which the other side reads and fetches whole; a reader's
/// recording of a post, a picked file of a reader's comment and a type a
/// chat refuses are refused.
#[tokio::test]
async fn a_voice_message_and_a_circle_go_as_a_post_and_as_a_readers_comment() {
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    use crate::media::{NewRecording, RecordingPlace};
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1]).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let outgoing = dir.path().join("outgoing");
    let voice: Vec<u8> = (0..40_000u32).map(|i| (i % 199) as u8).collect();
    let circle: Vec<u8> = (0..90_000u32).map(|i| (i % 211) as u8).collect();
    let rec = |kind: &str, mime: &str, bytes: &[u8], ms: u64, wave: Option<Vec<u8>>| NewRecording {
        kind: kind.into(),
        mime: mime.into(),
        duration_ms: Some(ms),
        waveform: wave,
        data_base64: B64.encode(bytes),
    };
    /// Recorded, placed and sent: the id of its message.
    async fn record(node: &Node, cid: &str, outgoing: &std::path::Path, r: NewRecording, place: RecordingPlace<'_>) -> messenger_core::Result<String> {
        let (row, _) = node.svc.media_placeholder_recording(&node.keys, cid, r, place, outgoing).await?;
        assert!(crate::media::UploadMeta::recorded_kind(&row.meta_json).is_some(), "the row knows it is a recording");
        Ok(node.svc.media_send(&node.keys, &row.id, Some(outgoing), &crate::media::Sink(|_| {})).await?.0)
    }
    let send = |node, r, place| record(node, &cid, &outgoing, r, place);
    // 70 values of outline: a chat keeps 64.
    let wave: Vec<u8> = (0..70u8).map(|i| i.wrapping_mul(3)).collect();
    let voice_post = send(&owner, rec("voice", "audio/webm;codecs=opus", &voice, 3_400, Some(wave)), RecordingPlace::default())
        .await
        .unwrap_or_else(|e| panic!("voice post: {e}"));
    w.pump(&all).await;
    let post = voice_post.as_str();
    let circle_comment = send(&reader, rec("circle", "video/webm", &circle, 7_250, None), RecordingPlace { post_id: Some(post), ..RecordingPlace::default() })
        .await
        .unwrap_or_else(|e| panic!("circle comment: {e}"));
    w.pump(&all).await;

    let d = |json: Option<&str>| {
        messenger_media::MediaDescriptor::from_envelope(&Envelope::parse(json.unwrap_or("")).unwrap_or_else(|e| panic!("{e}"))).unwrap_or_else(|e| panic!("{e}"))
    };
    let page = history(&reader, &cid, 10, None).await;
    let item = page.items.iter().find(|i| i.id == voice_post).unwrap_or_else(|| panic!("the voice post"));
    let v = d(item.media_json.as_deref());
    assert_eq!((v.kind, v.name.as_str(), v.mime.as_str(), v.duration_ms), (messenger_media::MediaKind::Voice, "voice.weba", "audio/webm", Some(3_400)));
    assert_eq!(v.waveform.as_ref().map(Vec::len), Some(64), "the outline as a chat keeps it");
    assert!(item.text.is_none(), "no words, no title");
    let thread = owner.svc.thread(&owner.keys, &cid, &voice_post, 10, None).await.unwrap_or_else(|e| panic!("thread: {e}"));
    let c = thread.items.iter().find(|i| i.id == circle_comment).unwrap_or_else(|| panic!("the circle comment"));
    let cd = d(c.media_json.as_deref());
    assert_eq!((cd.kind, cd.name.as_str(), cd.mime.as_str(), cd.duration_ms, cd.waveform.as_ref()), (messenger_media::MediaKind::Circle, "circle.webm", "video/webm", Some(7_250), None));
    // Each side fetches the other's whole, and plays it as a sound or a video.
    let out = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let got = reader.svc.media_fetch(&cid, &voice_post, out.path(), &crate::media::Sink(|_| {})).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(std::fs::read(got).unwrap_or_default(), voice);
    let got = owner.svc.media_fetch(&cid, &circle_comment, out.path(), &crate::media::Sink(|_| {})).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(std::fs::read(got).unwrap_or_default(), circle);
    let playable = owner.svc.media_playable(&cid, &circle_comment, out.path()).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(playable.map(|(_, m)| m), Some("video/webm".to_string()));

    // A reader records a comment, not a post; a reader's picked file is refused; so is what a chat refuses.
    assert!(send(&reader, rec("voice", "audio/webm", &voice, 1_000, None), RecordingPlace::default()).await.is_err(), "a reader's post");
    let picked = dir.path().join("note.pdf");
    std::fs::write(&picked, b"%PDF-1.4").unwrap_or_else(|e| panic!("{e}"));
    let file = crate::media::NewFile { post_id: Some(post), ..crate::media::NewFile::at(&picked) };
    assert!(reader.svc.media_placeholder_file(&reader.keys, &cid, file).await.is_err(), "a reader's file");
    for (kind, mime) in [("voice", "video/webm"), ("circle", "audio/webm"), ("file", "audio/webm"), ("voice", "audio/wav")] {
        assert!(send(&owner, rec(kind, mime, &voice, 1_000, None), RecordingPlace::default()).await.is_err(), "{kind} {mime}");
    }
    assert!(send(&owner, rec("voice", "audio/webm", &[], 1_000, None), RecordingPlace::default()).await.is_err(), "an empty recording");
    w.pump(&all).await;
    assert_eq!(history(&reader, &cid, 10, None).await.items.iter().filter(|i| i.media_json.is_some()).count(), 1, "nothing else went");
}

/// My own file is shown from where it was sent, never fetched back, and
/// the same file sent again goes as a new message of the chunks the hosts
/// hold: nothing goes up twice.
#[tokio::test]
async fn my_file_is_shown_from_here_and_the_same_file_again_uploads_nothing() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let video = dir.path().join("film.mp4");
    let bytes: Vec<u8> = (0..9 * 1024 * 1024u32).map(|i| (i % 241) as u8).collect();
    std::fs::write(&video, &bytes).unwrap_or_else(|e| panic!("{e}"));
    let send = |caption: &'static str| {
        let (svc, keys, cid, video) = (owner.svc.clone(), owner.keys.clone(), cid.clone(), video.clone());
        async move {
            let file = crate::media::NewFile { caption: Some(caption), ..crate::media::NewFile::at(&video) };
            let (row, _) = svc.media_placeholder_file(&keys, &cid, file).await.unwrap_or_else(|e| panic!("placeholder: {e}"));
            svc.media_send(&keys, &row.id, None, &crate::media::Sink(|_| {})).await.unwrap_or_else(|e| panic!("send: {e}")).0
        }
    };
    let first = send("once").await;
    // Shown and played from where it went: the hosts are not asked.
    let gets = w.hosts.gets.load(Ordering::SeqCst);
    let cache = dir.path().join("media");
    let here = owner.svc.media_fetch(&cid, &first, &cache, &crate::media::Sink(|_| {})).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(here, video);
    let playable = owner.svc.media_playable(&cid, &first, &cache).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(playable, Some((video.clone(), "video/mp4".to_string())));
    assert_eq!(w.hosts.gets.load(Ordering::SeqCst), gets, "nothing fetched back");
    // The same file again: no chunk goes up, the message names the same chunks.
    let puts = w.hosts.puts.load(Ordering::SeqCst);
    let again = send("twice").await;
    assert_eq!(w.hosts.puts.load(Ordering::SeqCst), puts, "nothing uploaded twice");
    w.pump(&all).await;
    let page = history(&reader, &cid, 10, None).await;
    let d = |id: &str| {
        let item = page.items.iter().find(|i| i.id == id).unwrap_or_else(|| panic!("the post"));
        messenger_media::MediaDescriptor::from_envelope(&Envelope::parse(item.media_json.as_deref().unwrap_or("")).unwrap_or_else(|e| panic!("{e}"))).unwrap_or_else(|e| panic!("{e}"))
    };
    let (a, b) = (d(&first), d(&again));
    assert_eq!((a.chunks.clone(), a.servers.clone()), (b.chunks.clone(), b.servers.clone()));
    assert_eq!((a.caption.as_deref(), b.caption.as_deref()), (Some("once"), Some("twice")));
    let out = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let got = reader.svc.media_fetch(&cid, &again, out.path(), &crate::media::Sink(|_| {})).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(std::fs::read(got).unwrap_or_default(), bytes);
    // A host lost a chunk: the file goes up anew.
    let chunk = a.chunks[0].sha256.clone();
    w.hosts.blobs.lock().unwrap_or_else(|e| e.into_inner()).get_mut(H2).map(|b| b.remove(&chunk));
    let puts = w.hosts.puts.load(Ordering::SeqCst);
    send("thrice").await;
    assert!(w.hosts.puts.load(Ordering::SeqCst) > puts, "uploaded again");
}

/// A video picked on a phone goes as the copy made of it, with the frame
/// the page took: the post and a comment carry its preview and size to the
/// reader, and so does the same file sent again (no chunk goes up, the
/// frame still goes) — живой запуск 2026-10-23: a channel's videos had no
/// frame on either side.
#[tokio::test]
async fn a_video_of_a_post_and_of_a_comment_carries_its_frame_also_sent_again() {
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1]).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let outgoing = dir.path().join("outgoing");
    std::fs::create_dir_all(&outgoing).unwrap_or_else(|e| panic!("{e}"));
    // The copy of a phone's file, under the name the user picked.
    let copy = outgoing.join("1792736850-207.mp4");
    std::fs::write(&copy, (0..300_000u32).map(|i| (i % 239) as u8).collect::<Vec<u8>>()).unwrap_or_else(|e| panic!("{e}"));
    let rgba: Vec<u8> = (0..320 * 180u32).flat_map(|i| [(i % 251) as u8, 90, 160, 255]).collect();
    let png = messenger_avatar::png_of_rgba(320, 180, &rgba).unwrap_or_else(|e| panic!("png: {e:?}"));
    let frame = messenger_avatar::photo_jpeg(&messenger_avatar::decode(&png).unwrap_or_else(|e| panic!("{e:?}")), 320, 80);
    let poster = || Some(crate::media::Poster { jpeg: B64.encode(&frame), width: 960, height: 2142 });
    let send = |caption: Option<&'static str>, post: Option<String>| {
        let (svc, keys, cid, copy, outgoing, poster) = (owner.svc.clone(), owner.keys.clone(), cid.clone(), copy.clone(), outgoing.clone(), poster());
        async move {
            let file = crate::media::NewFile { name: Some("207.mp4"), caption, post_id: post.as_deref(), poster, ..crate::media::NewFile::at(&copy) };
            let (row, _) = svc.media_placeholder_file(&keys, &cid, file).await.unwrap_or_else(|e| panic!("placeholder: {e}"));
            svc.media_send(&keys, &row.id, Some(&outgoing), &crate::media::Sink(|_| {})).await.unwrap_or_else(|e| panic!("send: {e}")).0
        }
    };
    let first = send(Some("Просто видео в тему"), None).await;
    let puts = w.hosts.puts.load(Ordering::SeqCst);
    let again = send(Some("ещё раз"), None).await;
    assert_eq!(w.hosts.puts.load(Ordering::SeqCst), puts, "the same file again: nothing goes up");
    let comment = send(None, Some(first.clone())).await;
    w.pump(&all).await;
    let page = history(&reader, &cid, 10, None).await;
    for id in [&first, &again] {
        let item = page.items.iter().find(|i| &i.id == id).unwrap_or_else(|| panic!("the post"));
        let d = messenger_media::MediaDescriptor::from_envelope(&Envelope::parse(item.media_json.as_deref().unwrap_or("")).unwrap_or_else(|e| panic!("{e}"))).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((d.kind, d.name.as_str(), d.mime.as_str(), d.dim), (messenger_media::MediaKind::Video, "207.mp4", "video/mp4", Some((960, 2142))));
        assert!(d.thumb.as_deref().is_some_and(messenger_media::descriptor::valid_thumb), "the frame as its preview");
    }
    let thread = reader.svc.thread(&reader.keys, &cid, &first, 10, None).await.unwrap_or_else(|e| panic!("thread: {e}"));
    let c = thread.items.iter().find(|i| i.id == comment).unwrap_or_else(|| panic!("the comment"));
    let d = messenger_media::MediaDescriptor::from_envelope(&Envelope::parse(c.media_json.as_deref().unwrap_or("")).unwrap_or_else(|e| panic!("{e}"))).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(d.dim, Some((960, 2142)));
    assert!(d.thumb.is_some(), "a comment's video carries its frame too");
}

/// A file given up after it failed leaves nothing on the hosts: the chunks
/// its state names are removed.
#[tokio::test]
async fn a_failed_file_given_up_leaves_nothing_on_the_hosts() {
    let w = World::new();
    let owner = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1]).await;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("film.mp4");
    std::fs::write(&path, vec![3u8; 9 * 1024 * 1024]).unwrap_or_else(|e| panic!("{e}"));
    let held = w.hosts.count(H1);
    let (row, _) = owner.svc.media_placeholder(&owner.keys, &cid, None, None, &path, None).await.unwrap_or_else(|e| panic!("placeholder: {e}"));
    *w.hosts.refuse_after.lock().unwrap_or_else(|e| e.into_inner()) = Some(1);
    assert!(owner.svc.media_send(&owner.keys, &row.id, None, &crate::media::Sink(|_| {})).await.is_err());
    assert_eq!(w.hosts.count(H1), held + 1, "one chunk stored before the refusal");
    owner.svc.media_cancel(&row.id).await.unwrap_or_else(|e| panic!("cancel: {e}"));
    for _ in 0..100 {
        if w.hosts.count(H1) == held {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(w.hosts.count(H1), held, "removed in the background");
    assert!(owner.svc.uploads(&cid).await.unwrap_or_default().is_empty());
}

/// A file goes up as a chat's does: several chunks, its state kept after
/// each, and a send that stopped goes on from the chunks the hosts hold.
#[tokio::test]
async fn a_failed_file_goes_on_from_the_chunks_the_hosts_hold() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1]).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("film.mp4");
    // Three chunks of the messenger's size (4 MiB, 4 MiB, 1 MiB).
    let bytes: Vec<u8> = (0..9 * 1024 * 1024u32).map(|i| (i % 253) as u8).collect();
    std::fs::write(&path, &bytes).unwrap_or_else(|e| panic!("{e}"));
    let (row, _) = owner.svc.media_placeholder(&owner.keys, &cid, None, None, &path, Some("a film")).await.unwrap_or_else(|e| panic!("placeholder: {e}"));
    // The host takes one chunk, then refuses (its plan): no retry helps.
    *w.hosts.refuse_after.lock().unwrap_or_else(|e| e.into_inner()) = Some(1);
    let sent = owner.svc.media_send(&owner.keys, &row.id, None, &crate::media::Sink(|_| {})).await;
    let e = sent.err().unwrap_or_else(|| panic!("refused"));
    assert!(e.to_string().contains(crate::media::UPLOAD_REFUSED), "{e}");
    let failed = owner.svc.uploads(&cid).await.unwrap_or_default().remove(0);
    assert_eq!(failed.status, db::content::UPLOAD_FAILED);
    let state: messenger_media::upload::UploadState = serde_json::from_str(&failed.state_json).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!((state.chunks_done(), state.chunks_total()), (1, 3), "the chunk the host holds is kept");
    assert_eq!(failed.done, 4 * 1024 * 1024, "and counted");
    // The plan allows it again: only the two missing chunks go.
    *w.hosts.refuse_after.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let puts = w.hosts.puts.load(Ordering::SeqCst);
    owner.svc.media_retry(&row.id).await.unwrap_or_else(|e| panic!("retry: {e}"));
    let last = std::sync::Mutex::new(String::new());
    let (id, _) = owner
        .svc
        .media_send(&owner.keys, &row.id, None, &crate::media::Sink(|p: crate::media::Progress| *lock(&last) = format!("{:?} {}", p.stage, p.status)))
        .await
        .unwrap_or_else(|e| panic!("send: {e}"));
    assert_eq!(w.hosts.puts.load(Ordering::SeqCst) - puts, 2);
    assert_eq!(*lock(&last), "Publishing done");
    w.pump(&all).await;
    let out = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let rates = std::sync::Mutex::new(Vec::new());
    let got = reader
        .svc
        .media_fetch(&cid, &id, out.path(), &crate::media::Sink(|p: crate::media::Progress| lock(&rates).push((p.direction.clone(), p.status.clone()))))
        .await
        .unwrap_or_else(|e| panic!("fetch: {e}"));
    assert_eq!(std::fs::read(&got).unwrap_or_default(), bytes);
    assert_eq!(lock(&rates).last().cloned(), Some(("down".to_string(), "done".to_string())));
    assert!(reader.svc.media_data_url(&cid, &id, out.path()).await.unwrap_or_else(|e| panic!("{e}")).is_some_and(|u| u.starts_with("data:video/mp4;base64,")));
}

#[tokio::test]
async fn rotate_now_and_the_status_of_a_keeper_never_wait_for_its_turn() {
    let w = World::new();
    let owner = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    let status = owner.svc.keeper_status(&cid).unwrap_or_else(|| panic!("a status after the first turn"));
    assert_eq!((status.epoch, status.readers), (Some(1), 1));
    // A turn in progress holds the keeper (uploads): neither call waits.
    let k = owner.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("keeper"));
    let held = k.lock().await;
    let asked = tokio::time::timeout(std::time::Duration::from_secs(1), owner.svc.rotate_now(&cid)).await;
    assert!(matches!(asked, Ok(Ok(true))));
    assert!(owner.svc.keeper_status(&cid).is_some_and(|s| s.rotate_asked));
    drop(held);
    tick(&owner).await;
    assert_eq!(w.relay.epochs(), vec![1, 2], "the next turn built it");
    let status = owner.svc.keeper_status(&cid).unwrap_or_else(|| panic!("status"));
    assert_eq!((status.epoch, status.rotate_asked), (Some(2), false));
}

#[tokio::test]
async fn posts_of_one_second_at_the_edge_of_a_page_are_not_skipped() {
    let w = World::new();
    let owner = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    for i in 0..5 {
        owner.svc.send_post(&owner.keys, &cid, &format!("p{i}"), TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    }
    let mut seen = Vec::new();
    let mut cursor: Option<crate::transport::FeedCursor> = None;
    for _ in 0..5 {
        let page = owner.svc.history_page(&owner.keys, &cid, 2, cursor.as_ref()).await.unwrap_or_else(|e| panic!("{e}"));
        seen.extend(page.items.iter().map(|i| i.id.clone()));
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 5, "every post, once");
}

#[tokio::test]
async fn a_catch_up_cut_by_the_relay_asks_for_the_pages_before() {
    crate::transport::CATCH_UP_LIMIT_TEST.with(|c| c.set(Some(5)));
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    *lock(&w.relay.cap) = Some(5);
    // The reader is away while twelve posts go out.
    for i in 0..12 {
        w.clock.advance(1);
        owner.svc.send_post(&owner.keys, &cid, &format!("p{i}"), TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    }
    reader.skip_all(&w.relay);
    reader.svc.listen(&reader.keys).await.unwrap_or_else(|e| panic!("listen: {e}"));
    w.pump(&all).await;
    assert_eq!(post_texts(&reader, &cid).await.len(), 12, "every page asked for");
}

/// The page before a full page of a catch-up is asked for off the way of
/// the event (живой запуск 2026-10-23: the page of the notes to the owner,
/// asked of every channel followed, held the loop 56 s after a channel was
/// made, each `Sync` awaited by the pool): the event that fills the page
/// sends nothing, it notes the ask (`take_asks`); whoever runs the service
/// sends it, and every page still comes.
#[tokio::test]
async fn a_full_page_of_a_catch_up_asks_for_the_one_before_off_the_event() {
    crate::transport::CATCH_UP_LIMIT_TEST.with(|c| c.set(Some(5)));
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    *lock(&w.relay.cap) = Some(5);
    for i in 0..12 {
        w.clock.advance(1);
        owner.svc.send_post(&owner.keys, &cid, &format!("p{i}"), TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    }
    // On the relay: the owner's outbox sent them.
    w.pump(&[&owner]).await;
    reader.skip_all(&w.relay);
    reader.svc.listen(&reader.keys).await.unwrap_or_else(|e| panic!("listen: {e}"));
    let page = reader.net.take_synced();
    let syncs = lock(&reader.net.syncs).len();
    for ev in &page {
        reader.svc.on_event(&reader.keys, ev, false).await.unwrap_or_else(|e| panic!("{e}"));
    }
    assert_eq!(lock(&reader.net.syncs).len(), syncs, "no request to the relays on the way of the events");
    let asks = reader.svc.take_asks();
    assert!(
        asks.iter().any(|a| matches!(a, messenger_core::Outbound::Sync { filter, .. } if filter.0["kinds"].as_array().is_some_and(|k| k.contains(&serde_json::json!(wire::KIND_CONTENT))) && filter.0.get("until").is_some())),
        "the page before the full one of the feed is wanted: {asks:?}"
    );
    reader.svc.send_asks(asks).await;
    assert!(lock(&reader.net.syncs).len() > syncs, "sent by whoever runs the service");
    w.pump(&all).await;
    assert_eq!(post_texts(&reader, &cid).await.len(), 12, "every page asked for");
}

/// The notes to me come as the same page from every channel on the same
/// relays, and from each relay of them (2026-10-23: the dev app asked for
/// the page before of every channel, every `limit` of the copies asked
/// again, and pulled 95 GB of the same wraps from one relay in a night;
/// strfry was killed for its memory). A full page asks for the page before
/// once for the set of relays; the copies of a page fill no page of their
/// own, but the oldest second of the page asked for, which it brings
/// again, counts there once; a short page ends the round.
#[tokio::test]
async fn the_copies_of_a_page_of_the_notes_ask_for_the_page_before_once() {
    crate::transport::CATCH_UP_LIMIT_TEST.with(|c| c.set(Some(5)));
    let w = World::new();
    let owner = w.node().await;
    for _ in 0..3 {
        create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    }
    owner.svc.catch_up(&owner.keys).await.unwrap_or_else(|e| panic!("catch-up: {e}"));
    owner.svc.take_asks();
    let me = pk(&owner).as_hex().to_string();
    let wrap = |i: i64| serde_json::json!({ "id": format!("{i:064x}"), "kind": crate::notes::KIND_WRAP, "created_at": 1_000 - i, "tags": [["p", me]] });
    let untils = |asks: Vec<messenger_core::Outbound>| -> Vec<i64> {
        asks.iter()
            .filter_map(|a| match a {
                messenger_core::Outbound::Sync { filter, .. } if filter.0["kinds"] == serde_json::json!([crate::notes::KIND_WRAP]) => filter.0["until"].as_i64(),
                _ => None,
            })
            .collect()
    };
    let page = |ids: std::ops::Range<i64>| {
        let (owner, wrap) = (&owner, &wrap);
        async move {
            // As each of the three channels brings it.
            for _ in 0..3 {
                for i in ids.clone() {
                    owner.svc.heard(&wrap(i), false).await.unwrap_or_else(|e| panic!("{e}"));
                }
            }
        }
    };
    page(0..5).await;
    assert_eq!(untils(owner.svc.take_asks()), vec![996], "the page before the full one, once");
    // Its oldest second again, and four older wraps.
    page(4..9).await;
    assert_eq!(untils(owner.svc.take_asks()), vec![992], "the next page, once");
    page(8..11).await;
    assert_eq!(untils(owner.svc.take_asks()), Vec::<i64>::new(), "a short page asks for none");
}

/// The heads of the journal a snapshot names that are not here are asked
/// for off the way of the event too (`take_asks`): the event sends
/// nothing, and the operation comes once the ask is sent.
#[tokio::test]
async fn the_heads_a_snapshot_names_are_asked_for_off_the_event() {
    let w = World::new();
    let (owner, reader, cid) = channel_with_reader(&w).await;
    let newcomer = w.node().await;
    let all = [&owner, &reader, &newcomer];
    // The reader hears no operation and no snapshot for now.
    reader.hold(wire::KIND_JOURNAL, true);
    reader.hold(KIND_SNAPSHOT, true);
    owner.svc.act(&owner.keys, &cid, OpBody::SetRole { who: pk(&newcomer), role: Role::Moderator }).await.unwrap_or_else(|e| panic!("role: {e}"));
    join_by_link(&w, &owner, &newcomer, &cid, &all).await;
    w.clock.advance(600);
    tick(&owner).await;
    let snap = w.relay.of_kind(KIND_SNAPSHOT).last().cloned().unwrap_or_else(|| panic!("a snapshot"));
    let s = crate::snapshot::content_of(&serde_json::to_string(&snap).unwrap_or_default()).unwrap_or_else(|e| panic!("snapshot: {e}"));
    let log = reader.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("journal"));
    let missing: Vec<String> = s.heads.iter().filter(|h| !log.contains(h)).map(|h| h.0.clone()).collect();
    assert!(!missing.is_empty(), "the snapshot names an operation the reader has not heard");
    let syncs = lock(&reader.net.syncs).len();
    reader.svc.on_event(&reader.keys, &snap, true).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(lock(&reader.net.syncs).len(), syncs, "no request to the relays on the way of the event");
    let asks = reader.svc.take_asks();
    assert!(
        asks.iter().any(|a| matches!(a, messenger_core::Outbound::Sync { filter, .. } if filter.0["ids"].as_array().is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(missing[0].as_str()))))),
        "the heads are wanted: {asks:?}"
    );
    reader.svc.send_asks(asks).await;
    let asked = lock(&reader.net.syncs)[syncs..].iter().any(|f| f["ids"].as_array().is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(missing[0].as_str()))));
    assert!(asked, "sent by whoever runs the service");
}

#[tokio::test]
async fn one_file_asked_for_twice_at_once_is_fetched_once() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("photo.jpg");
    std::fs::write(&path, vec![5u8; 300_000]).unwrap_or_else(|e| panic!("{e}"));
    let (row, _) = owner.svc.media_placeholder(&owner.keys, &cid, None, None, &path, None).await.unwrap_or_else(|e| panic!("{e}"));
    let (id, _) = owner.svc.media_send(&owner.keys, &row.id, None, &crate::media::Sink(|_| {})).await.unwrap_or_else(|e| panic!("send: {e}"));
    w.pump(&all).await;
    let into = dir.path().join("in");
    let before = w.hosts.gets.load(Ordering::SeqCst);
    let (a, b) = tokio::join!(reader.svc.media_fetch(&cid, &id, &into, &crate::media::Sink(|_| {})), reader.svc.media_fetch(&cid, &id, &into, &crate::media::Sink(|_| {})));
    let (a, b) = (a.unwrap_or_else(|e| panic!("{e}")), b.unwrap_or_else(|e| panic!("{e}")));
    assert_eq!(a, b);
    let once = w.hosts.gets.load(Ordering::SeqCst) - before;
    assert_eq!(std::fs::read(&a).unwrap_or_default(), vec![5u8; 300_000]);
    reader.svc.media_fetch(&cid, &id, &into, &crate::media::Sink(|_| {})).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(w.hosts.gets.load(Ordering::SeqCst) - before, once, "not again when the file is here");
}

#[tokio::test]
async fn the_row_says_why_i_am_out() {
    let w = World::new();
    let owner = w.node().await;
    let (leaver, removed, banned) = (w.node().await, w.node().await, w.node().await);
    let all = [&owner, &leaver, &removed, &banned];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    for n in [&leaver, &removed, &banned] {
        join_by_link(&w, &owner, n, &cid, &all).await;
    }
    let why = |n: &Node| {
        let (db, cid) = (n.svc.db().clone(), cid.clone());
        async move { db::channels::get(&db, &cid).await.ok().flatten().and_then(|r| r.out_reason) }
    };
    assert_eq!(why(&leaver).await, None, "in");
    leaver.svc.leave(&leaver.keys, &cid).await.unwrap_or_else(|e| panic!("leave: {e}"));
    owner.svc.act(&owner.keys, &cid, OpBody::Remove { who: pk(&removed) }).await.unwrap_or_else(|e| panic!("{e}"));
    owner.svc.act(&owner.keys, &cid, OpBody::Ban { who: pk(&banned), reason: None }).await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&all).await;
    assert_eq!(why(&leaver).await.as_deref(), Some(crate::service::OUT_LEFT));
    assert_eq!(why(&removed).await.as_deref(), Some(crate::service::OUT_REMOVED));
    assert_eq!(why(&banned).await.as_deref(), Some(crate::service::OUT_BANNED));
    assert_eq!(membership(&banned, &cid).await, MEMBERSHIP_LEFT);
    owner.svc.act(&owner.keys, &cid, OpBody::Disband).await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&all).await;
    assert_eq!(why(&leaver).await.as_deref(), Some(crate::service::OUT_DISBANDED));
}

#[tokio::test]
async fn every_member_sees_when_the_keeper_was_last_seen() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    let seen = || async { db::channels::get(reader.svc.db(), &cid).await.ok().flatten().and_then(|r| r.keeper_last_seen) };
    let first = seen().await.unwrap_or_else(|| panic!("an epoch was taken"));
    w.clock.advance(100);
    owner.svc.act(&owner.keys, &cid, OpBody::RotateNow { scope_id: None }).await.unwrap_or_else(|e| panic!("{e}"));
    w.pump(&all).await;
    assert_eq!(seen().await, Some(first + 100), "the time of the newest epoch taken");
}

/// A quiet channel makes no epoch for hours: «был в сети» of a live keeper
/// stood still and the joiner saw it away (живой запуск 2026-10-23, «был в
/// сети: 16:20» while the keeper was online), and the moderators would hand
/// tickets almost always (wire §17). The keeper says it runs (wire §18) at
/// most every five minutes; a member takes the newest sign of the keeper of
/// the current series only, and the last seen and the rule of the tickets
/// read it.
#[tokio::test]
async fn a_live_keeper_of_a_quiet_channel_is_seen_by_its_sign_of_life() {
    use crate::heartbeat::{sign_heartbeat, Heartbeat, HEARTBEAT_VERSION, INTERVAL_SECS};
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    let signs = || w.relay.of_kind(wire::KIND_HEARTBEAT).len();
    let seen = |n: &Node| {
        let (db, cid) = (n.svc.db().clone(), cid.clone());
        async move { db::channels::get(&db, &cid).await.ok().flatten().and_then(|r| r.keeper_last_seen) }
    };
    let first = signs();
    assert_eq!(first, 1, "said once as the keeper began, not on every turn since");

    // A turn within five minutes says nothing more.
    w.clock.advance(INTERVAL_SECS - 10);
    owner.svc.tick().await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(signs(), first);

    // Hours of nothing: no epoch, yet the keeper is seen as it runs.
    let epoch_seen = seen(&reader).await.unwrap_or_else(|| panic!("an epoch was taken"));
    w.clock.advance(3 * 3600);
    let next = owner.svc.tick().await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(signs(), first + 1, "said again once due");
    assert!(next.is_some_and(|at| i64::try_from(at).unwrap_or(i64::MAX) <= w.clock.now() + INTERVAL_SECS), "the cycle comes back for the next one: {next:?}");
    w.pump(&all).await;
    let now = w.clock.now();
    assert_eq!(seen(&reader).await, Some(now), "the reader sees the keeper now, not at its epoch {epoch_seen}");
    assert_eq!(seen(&owner).await, Some(now), "and so does the owner's page");
    let heard = reader.svc.keeper_heard_at(&cid).await.unwrap_or_else(|e| panic!("{e}"));
    assert!(heard.is_some_and(|at| now - at < crate::keeper::KEEPER_AWAY_SECS), "the tickets see the keeper about: {heard:?}");

    // Signed by another key, or naming another series: refused, the last
    // seen stays.
    let k = owner.svc.keeper(&cid).await.unwrap_or_else(|e| panic!("{e}")).unwrap_or_else(|| panic!("kept"));
    let keeper_keys = k.lock().await.keys().clone();
    w.clock.advance(60);
    let later = w.clock.now();
    let forged = |keys: &nostr::key::Keys, serial: u32| {
        let hb = Heartbeat { v: HEARTBEAT_VERSION, community_id: cid.clone(), at: later, serial };
        serde_json::to_value(sign_heartbeat(keys, &hb).unwrap_or_else(|e| panic!("{e}"))).unwrap_or_else(|e| panic!("{e}"))
    };
    for (bad, why) in [(forged(&reader.keys, 0), "not the current keeper"), (forged(&keeper_keys, 7), "not the current series")] {
        let out = reader.svc.on_event(&reader.keys, &bad, true).await.unwrap_or_else(|e| panic!("{e}"));
        assert!(out.notes.iter().any(|n| n.starts_with("heartbeat refused") && n.contains(why)), "{:?}", out.notes);
    }
    assert_eq!(seen(&reader).await, Some(now));
    // The keeper's own, of its series: taken.
    let good = forged(&keeper_keys, 0);
    reader.svc.on_event(&reader.keys, &good, true).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(seen(&reader).await, Some(later));
}

/// One round of listening as the host makes it: the catch-up asked for,
/// and what the relays send back taken as history.
async fn round(n: &Node) -> crate::service::Outcome {
    use messenger_core::Transport;
    for out in n.svc.catch_up(&n.keys).await.unwrap_or_else(|e| panic!("catch-up: {e}")) {
        n.net.send(out).await.unwrap_or_else(|e| panic!("send: {e}"));
    }
    let mut all = crate::service::Outcome::default();
    for ev in n.net.take_synced() {
        all.merge(n.svc.on_event(&n.keys, &ev, false).await.unwrap_or_else(|e| panic!("event: {e}")));
    }
    all
}

/// Every round of listening hears again what the rounds before heard (the
/// cursor of a part moves only after a whole round): the epochs, the
/// snapshot, the notes of the keeper. Heard again, they are silent — no
/// line in the log, no event for the page, and no ask for one more round,
/// which would ask for the next one in its turn, for ever (живой запуск
/// 2026-10-22, «epoch: Duplicate» и «snapshot refused: not newer» по кругу).
#[tokio::test]
async fn a_round_that_hears_again_what_was_heard_is_silent_and_asks_for_no_next_one() {
    let w = World::new();
    let owner = w.node().await;
    let (a, b) = (w.node().await, w.node().await);
    let all = [&owner, &a, &b];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &a, &cid, &all).await;
    join_by_link(&w, &owner, &b, &cid, &all).await;
    w.clock.advance(300);
    tick(&owner).await;
    w.pump(&all).await;
    assert_eq!(snapshots(&w).len(), 2);
    w.clock.advance(1);
    let (post, _) = owner.svc.send_post(&owner.keys, &cid, "post", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.clock.advance(1);
    a.svc.send_comment(&a.keys, &cid, &post, None, "comment", TextFormat::Plain).await.unwrap_or_else(|e| panic!("comment: {e}"));
    w.pump(&all).await;
    for n in all {
        for i in 0..3 {
            let out = round(n).await;
            assert!(!out.resubscribe, "round {i}: no next round asked for");
            assert!(out.events.is_empty(), "round {i}: no event for the page: {:?}", out.events);
            assert!(out.notes.is_empty(), "round {i}: nothing for the log: {:?}", out.notes);
            w.clock.advance(61);
        }
    }
}

/// A picture of noise, which no PNG makes small: more than the 1 MB of a
/// cover on the wire.
fn noise_png(w: u32, h: u32) -> Vec<u8> {
    let mut x: u32 = 0x1234_5678;
    let rgba: Vec<u8> = (0..w * h * 4)
        .map(|i| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            if i % 4 == 3 { 255 } else { (x >> 24) as u8 }
        })
        .collect();
    messenger_avatar::png_of_rgba(w, h, &rgba).unwrap_or_else(|e| panic!("png: {e:?}"))
}

/// The blobs of the keeper go up with the key of the user at work (the
/// owner, who pays for the shards and the indexes, communities-spec §13):
/// a host bound to accounts knows no keeper's key (живой запуск
/// 2026-10-22).
#[tokio::test]
async fn the_blobs_of_the_keeper_go_up_with_the_owners_key() {
    let w = World::new();
    let owner = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    let signers = lock(&w.hosts.signers).clone();
    assert!(!signers.is_empty(), "the first epoch put its blobs");
    let me = owner.keys.public_key().to_hex();
    assert!(signers.iter().all(|(_, by)| *by == me), "every blob by the owner's key: {signers:?}");
    assert!(newest(&owner, &cid).await.is_some());
}

/// A cover is made from any picture as the picture of a group is — a
/// square JPEG of 512, within the 1 MB of the wire — and put with the
/// user's key; a host that refuses it is named with its answer
/// (`channel_upload_refused`), not «something went wrong».
#[tokio::test]
async fn a_cover_is_made_square_and_a_refusal_of_the_hosts_says_their_answer() {
    let w = World::new();
    let owner = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    let picked = noise_png(1600, 1200);
    assert!(picked.len() > crate::media::MAX_COVER_BYTES, "a picture larger than a cover");
    owner.svc.set_cover(&owner.keys, &cid, &picked).await.unwrap_or_else(|e| panic!("cover: {e}"));
    let row = db::avatar::get(owner.svc.db(), &cid).await.ok().flatten().unwrap_or_else(|| panic!("the cover is kept"));
    let stored = lock(&w.hosts.blobs).get(H1).and_then(|b| b.get(&row.sha256).cloned()).unwrap_or_else(|| panic!("on the host"));
    assert!(stored.starts_with(&[0xff, 0xd8, 0xff]) && stored.len() <= crate::media::MAX_COVER_BYTES, "a JPEG within the wire's bound");
    let img = messenger_avatar::decode(&stored).unwrap_or_else(|e| panic!("decode: {e:?}"));
    assert_eq!((img.width(), img.height()), (messenger_avatar::OWN_SIDE, messenger_avatar::OWN_SIDE));
    let me = owner.keys.public_key().to_hex();
    assert!(lock(&w.hosts.signers).iter().filter(|(_, by)| *by != me).count() == 0);

    assert!(owner.svc.set_cover(&owner.keys, &cid, b"not a picture").await.is_err_and(|e| e.to_string().contains("a cover is not a picture")));
    for h in [H1, H2] {
        lock(&w.hosts.refusing).insert(h.to_string());
    }
    let err = owner.svc.set_cover(&owner.keys, &cid, &noise_png(64, 48)).await.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains(crate::media::UPLOAD_REFUSED) && err.contains("no_account"), "{err}");
}

/// One host of two down sets the cover all the same, its address on the
/// host that took it (живой запуск 2026-10-23: eu-1 down, the cover of a
/// new channel refused twice); with every host down it is refused.
#[tokio::test]
async fn a_host_down_holds_no_cover_back() {
    let w = World::new();
    let owner = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    w.hosts.set_down(H1, true);
    owner.svc.set_cover(&owner.keys, &cid, &noise_png(300, 200)).await.unwrap_or_else(|e| panic!("cover: {e}"));
    let row = db::avatar::get(owner.svc.db(), &cid).await.ok().flatten().unwrap_or_else(|| panic!("the cover is kept"));
    assert!(row.url.starts_with(H2), "the address names the host that took it: {}", row.url);
    assert_eq!(row.copies_json, serde_json::json!([H2]).to_string());
    w.hosts.set_down(H2, true);
    assert!(owner.svc.set_cover(&owner.keys, &cid, &noise_png(200, 300)).await.is_err(), "no host: refused");
}

/// The cover is seen by a reader, not only by its owner (живой запуск
/// 2026-10-23: «нельзя обновить фото канала» — a reader saw initials for
/// ever): it is read from the hosts of the channel, checked by the hash of
/// its address, kept by that hash, and a later ask fetches nothing; a
/// channel without a cover, or a cover no host holds, says `None`.
#[tokio::test]
async fn a_reader_sees_the_cover_from_the_hosts_of_the_channel_and_keeps_it() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &reader, &cid, &[&owner, &reader]).await;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(reader.svc.cover_data_url(&cid, dir.path()).await.unwrap_or_else(|e| panic!("{e}")), None, "no cover yet");

    owner.svc.set_cover(&owner.keys, &cid, &noise_png(300, 200)).await.unwrap_or_else(|e| panic!("cover: {e}"));
    w.pump(&[&owner, &reader]).await;
    let row = db::avatar::get(owner.svc.db(), &cid).await.ok().flatten().unwrap_or_else(|| panic!("the cover is kept"));
    let first = reader.svc.cover_data_url(&cid, dir.path()).await.unwrap_or_else(|e| panic!("{e}")).unwrap_or_else(|| panic!("the reader sees the cover"));
    assert!(first.starts_with("data:image/jpeg;base64,"));
    assert!(dir.path().join(format!("cover-{}.jpg", row.sha256)).exists(), "kept by the hash");

    // The hosts lose it: the kept copy is shown, nothing is fetched.
    lock(&w.hosts.blobs).clear();
    assert_eq!(reader.svc.cover_data_url(&cid, dir.path()).await.unwrap_or_else(|e| panic!("{e}")), Some(first));
    // Another folder (a fresh device) with no host that holds it: none, not an error.
    let other = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(reader.svc.cover_data_url(&cid, other.path()).await.unwrap_or_else(|e| panic!("{e}")), None);
}

/// The lines a node's service says, kept for the test.
fn said(n: &Node) -> std::sync::Arc<std::sync::Mutex<Vec<String>>> {
    let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let to = lines.clone();
    n.svc.set_say(Some(std::sync::Arc::new(move |l: &str| lock(&to).push(l.to_string()))));
    lines
}

/// Хосты хранилища отказывают (живой запуск 2026-10-23: 403 и молчащий
/// хранитель): вступивший всё равно получает ключ — записки хранителя идут
/// отдельно от выгрузок; отказ хоста виден строкой терминала один раз и в
/// состоянии хранителя (`storage_error`) с ответом хоста, а когда хосты
/// снова берут — одна строка об этом и состояние чистое.
#[tokio::test]
async fn a_keeper_whose_hosts_refuse_still_lets_readers_in_and_says_why() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let lines = said(&owner);
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    assert_eq!(w.relay.epochs(), vec![1]);
    for h in [H1, H2] {
        lock(&w.hosts.refusing).insert(h.to_string());
    }
    // An epoch asked for now cannot go up: the hosts say no.
    owner.svc.rotate_now(&cid).await.unwrap_or_else(|e| panic!("rotate: {e}"));
    tick(&owner).await;
    tick(&owner).await;
    assert_eq!(w.relay.epochs(), vec![1], "no epoch event without its blobs");
    let st = owner.svc.keeper_status(&cid).unwrap_or_else(|| panic!("a status"));
    let err = st.storage_error.unwrap_or_default();
    assert!(err.contains(H1) && err.contains("no_account"), "the host and its answer: {err}");
    let refusals: Vec<String> = lock(&lines).iter().filter(|l| l.contains("the blob hosts")).cloned().collect();
    assert_eq!(refusals.len(), 1, "said once, not every turn: {refusals:?}");
    assert!(refusals[0].starts_with(&format!("keeper {}: the blob hosts: ", &cid[..8])) && refusals[0].contains("no_account"), "{refusals:?}");

    // A reader comes by the link meanwhile: the keeper answers and the key
    // of the epoch out opens.
    join_by_link(&w, &owner, &reader, &cid, &[&owner, &reader]).await;
    tick(&owner).await;
    w.pump(&[&owner, &reader]).await;
    assert!(newest(&reader, &cid).await.is_some(), "the reader holds a key of the channel");
    assert_eq!(membership(&reader, &cid).await, MEMBERSHIP_JOINED);
    assert!(lock(&lines).iter().any(|l| l.contains("channel.join from") && l.contains("taken, a reader of the channel")), "{:?}", lock(&lines));

    // The hosts take the blobs again: the epoch goes out, the trouble ends.
    lock(&w.hosts.refusing).clear();
    tick(&owner).await;
    assert_eq!(w.relay.epochs(), vec![1, 2]);
    assert_eq!(owner.svc.keeper_status(&cid).and_then(|s| s.storage_error), None);
    assert!(lock(&lines).iter().any(|l| l == &format!("keeper {}: the blob hosts: every host takes the blobs again", &cid[..8])), "{:?}", lock(&lines));
    assert!(owner.errors().is_empty() && reader.errors().is_empty(), "{:?} {:?}", owner.errors(), reader.errors());
}

/// A gift wrap on the `#p` of my key that neither my key nor a keeper's
/// opens is said on the terminal, not dropped unseen; an epoch is said in
/// one short line, never the event itself.
#[tokio::test]
async fn a_wrap_that_does_not_open_and_an_epoch_are_said_in_short_lines() {
    let w = World::new();
    let owner = w.node().await;
    let lines = said(&owner);
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    let stranger = nostr::key::Keys::generate();
    let other = nostr::key::Keys::generate();
    // A note to somebody else, put on my `#p` by a relay that lies.
    let mut wrap = crate::notes::wrap_note(&stranger, &pk_of(&other), "{\"t\":\"channel.join\"}", w.clock.now()).unwrap_or_else(|e| panic!("wrap: {e}")).json;
    wrap["tags"] = serde_json::json!([["p", owner.me.as_hex()]]);
    assert!(owner.svc.on_wrap(&owner.keys, &wrap).await.unwrap_or_else(|e| panic!("wrap: {e}")).is_none());
    let line = lock(&lines).iter().find(|l| l.starts_with("a gift wrap ")).cloned().unwrap_or_default();
    assert!(line.contains(&owner.me.as_hex()[..8]) && line.contains("not opened by my key or a keeper's"), "{:?}", lock(&lines));

    // The epoch the owner hears back from the relay: one short line.
    let reader = w.node().await;
    reader.hold(KIND_EPOCH, true);
    let link = owner.svc.share_link(&cid).await.unwrap_or_else(|e| panic!("link: {e}"));
    w.pump(&[&reader]).await;
    reader.svc.open_link(&reader.keys, &link, "").await.unwrap_or_else(|e| panic!("open link: {e}"));
    let ev = w.relay.of_kind(KIND_EPOCH)[0].clone();
    let out = reader.svc.on_event(&reader.keys, &ev, true).await.unwrap_or_else(|e| panic!("epoch: {e}"));
    let notes: Vec<&String> = out.notes.iter().filter(|n| n.starts_with("epoch ")).collect();
    assert_eq!(notes, [&format!("epoch 1 of {}: accepted", &cid[..8])], "one short line, never the event");
}

fn pk_of(k: &nostr::key::Keys) -> PubKey {
    PubKey::parse(&k.public_key().to_hex()).unwrap_or_else(|| unreachable!())
}

// ─── Sub-stage 1.6: what the trial on the test servers found ────────────────

/// The trial (tools/trial): the loop of the app runs the keepers' cycle
/// while `create` is still uploading the first epoch; the keeper written
/// to the rows but not yet in memory was loaded a second time, and two
/// events of epoch 1 went out (the members took one and called the other
/// a conflict).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_cycle_running_during_create_keeps_one_keeper() {
    let w = World::new();
    let owner = w.node().await;
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (svc, flag) = (owner.svc.clone(), done.clone());
    let cycle = tokio::spawn(async move {
        while !flag.load(Ordering::SeqCst) {
            let _ = svc.tick().await;
            tokio::task::yield_now().await;
        }
    });
    let (cid, _) = owner.svc.create(&owner.keys, new_channel(JoinMode::Open, History::Open, &[H1, H2])).await.unwrap_or_else(|e| panic!("create: {e}"));
    done.store(true, Ordering::SeqCst);
    cycle.await.unwrap_or_else(|e| panic!("{e}"));
    tick(&owner).await;
    w.pump(&[&owner]).await;
    assert_eq!(w.relay.of_kind(KIND_EPOCH).len(), 1, "one event of epoch 1");
    assert_eq!(newest(&owner, &cid).await.map(|k| k.epoch), Some(1));
    assert_eq!(keeper_state(&owner, &cid).await.gens.len(), 1, "no emergency generation");
}

/// The trial: a relay answers a REQ newest first, so a device new to a
/// channel hears its later operations before its `create`. They wait in
/// the database (no error) and count once the `create` comes.
#[tokio::test]
async fn operations_before_their_create_wait_for_it() {
    let w = World::new();
    let owner = w.node().await;
    let (moder, newcomer) = (w.node().await, w.node().await);
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    owner.svc.act(&owner.keys, &cid, OpBody::SetRole { who: pk(&moder), role: Role::Moderator }).await.unwrap_or_else(|e| panic!("{e}"));
    flush(&owner).await;
    let journal = w.relay.of_kind(wire::KIND_JOURNAL);
    assert_eq!(journal.len(), 2);
    for ev in journal.iter().rev() {
        newcomer.svc.on_event(&newcomer.keys, ev, false).await.unwrap_or_else(|e| panic!("an operation before its create: {e}"));
    }
    let state = newcomer.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("journal")).state().clone();
    assert_eq!(state.role_of(&pk(&moder)), Some(Role::Moderator));
}

/// The trial: a reader new to a busy channel catches up the newest 500
/// events of the feed (the relay's limit of a REQ), all of them comments
/// that wait for a snapshot naming their authors; the page of the feed
/// asks the relay for what is older than those, not for the newest again.
#[tokio::test]
async fn a_page_of_the_feed_walks_back_past_what_waits() {
    let w = World::new();
    let owner = w.node().await;
    let (a, c) = (w.node().await, w.node().await);
    let all = [&owner, &a, &c];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &a, &cid, &all).await;
    join_by_link(&w, &owner, &c, &cid, &all).await;
    w.clock.advance(400);
    tick(&owner).await;
    w.pump(&all).await;
    let (post, _) = owner.svc.send_post(&owner.keys, &cid, "the post", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    c.skip_all(&w.relay);
    w.clock.advance(10);
    let at = w.clock.now();
    let stranger = nostr::key::Keys::generate();
    let parked = forged_message(&a, &stranger, &cid, Links { thread: Some(post), ..Links::default() }, "waits", at).await;
    c.svc.on_event(&c.keys, &serde_json::to_value(&parked).unwrap_or_default(), false).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(db::channels::count_pending(c.svc.db(), &cid).await.unwrap_or(0), 1, "waits for a snapshot");
    w.clock.advance(50);
    let page = history(&c, &cid, 10, None).await;
    assert!(page.asked);
    let until = lock(&c.net.syncs).last().map(|f| f["until"].clone()).unwrap_or_default();
    assert_eq!(until, serde_json::json!(at), "older than what waits");
}

/// The trial: a banned reader's device that reads its inbox again (a
/// start, a catch-up of two days of notes) takes its old `channel.issue`
/// and `own.channel_keys` once more; they must not make it a member again.
#[tokio::test]
async fn old_notes_heard_again_do_not_undo_a_ban() {
    let w = World::new();
    let owner = w.node().await;
    let a = w.node().await;
    let all = [&owner, &a];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &a, &cid, &all).await;
    owner.svc.act(&owner.keys, &cid, OpBody::Ban { who: pk(&a), reason: None }).await.unwrap_or_else(|e| panic!("ban: {e}"));
    w.pump(&all).await;
    assert_eq!(membership(&a, &cid).await, MEMBERSHIP_LEFT);
    let notes = lock(&w.relay.inbox).get(a.me.as_hex()).cloned().unwrap_or_default();
    assert!(notes.len() >= 2, "the issue and the note of my own keys");
    for wrap in &notes {
        let _ = a.svc.on_wrap(&a.keys, wrap).await;
    }
    assert_eq!(membership(&a, &cid).await, MEMBERSHIP_LEFT, "still out");
}

// ─── Stage 2 of the trial's fixes: the keeper's inbox ───────────────────────

/// Wire §3 `inbox` (2026-10-22): the keeper of the app names the owner's
/// key; a reader's join goes there, the owner's own subscription hears it
/// and the keeper kept here answers. Nothing goes to the keeper's key, no
/// filter names two keys (a relay with vguard serves a `#p` only to the
/// connection signed in as it), and the owner's other device, which keeps
/// nothing, hears the join and lets it be.
#[tokio::test]
async fn a_join_to_the_owners_key_is_answered_by_the_keeper_of_the_app() {
    let w = World::new();
    let owner = w.node().await;
    let phone = w.device(owner.keys.clone()).await;
    let reader = w.node().await;
    let all = [&owner, &phone, &reader];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    let keeper = owner.svc.keeper_pubkey(&cid).await.ok().flatten().unwrap_or_else(|| panic!("keeper"));
    assert_eq!(inbox_of_keeper(&owner, &cid).await, pk(&owner), "create names the owner as the inbox");
    let journal = w.relay.of_kind(wire::KIND_JOURNAL);
    assert!(journal[0]["content"].as_str().unwrap_or("").contains(&format!("\"inbox\":\"{}\"", owner.me.as_hex())));
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    assert_eq!(membership(&reader, &cid).await, MEMBERSHIP_JOINED);
    assert!(newest(&reader, &cid).await.is_some());
    assert_eq!(keeper_inbox(&w, &owner, &cid).await, 1, "the join went to the owner's key");
    assert!(lock(&w.relay.inbox).get(keeper.as_hex()).is_none_or(Vec::is_empty), "nothing to the keeper's key");
    for n in all {
        let subs = n.svc.subscriptions(&n.keys).await.unwrap_or_default();
        for s in subs {
            if let messenger_core::Outbound::Subscribe { filter, .. } = s {
                let p = filter.0["#p"].as_array().map_or(0, Vec::len);
                assert!(p <= 1, "one key a filter: {}", filter.0);
                assert!(!filter.0.to_string().contains(keeper.as_hex()), "the keeper's key is asked for nowhere");
            }
        }
        assert!(n.errors().is_empty(), "{:?}", n.errors());
    }
    assert!(phone.svc.kept().await.unwrap_or_default().is_empty(), "the phone keeps nothing");
}

/// A channel made before `inbox` (живой запуск 2026-10-23: its `create`
/// named no inbox, every join went to the keeper's own key, and the keeper
/// of the app, on the owner's connection, heard none of them). The owner's
/// next turn names the owner the inbox, once, by a `keeper` of the same
/// key in the next series; a reader who joins then is answered on the
/// owner's key, nothing more goes to the keeper's, and an epoch after the
/// new series opens for the reader.
#[tokio::test]
async fn a_channel_made_before_inbox_names_the_owner_as_the_inbox_of_its_keeper() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let new = new_channel(JoinMode::Open, History::Open, &[H1, H2]);
    let (cid, _) = owner.svc.create_naming(&owner.keys, new, false).await.unwrap_or_else(|e| panic!("create: {e}"));
    // The journal out, no turn of the keeper yet (a pump would run the one
    // the create asked for, and that turn names the inbox).
    flush(&owner).await;
    let keeper = owner.svc.keeper_pubkey(&cid).await.ok().flatten().unwrap_or_else(|| panic!("keeper"));
    assert_eq!(inbox_of_keeper(&owner, &cid).await, keeper, "a journal before inbox: the keeper's own key");

    let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let said = lines.clone();
    owner.svc.set_say(Some(std::sync::Arc::new(move |l: &str| lock(&said).push(l.to_string()))));
    tick(&owner).await;
    w.pump(&all).await;
    let log = owner.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("log"));
    let last = log.state().keeper().cloned().unwrap_or_else(|| panic!("keeper"));
    assert_eq!((&last.pubkey, last.serial, last.inbox.as_ref()), (&keeper, 1, Some(&pk(&owner))), "the same key, the next series, the owner's inbox");
    assert!(lock(&lines).iter().any(|l| l.starts_with(&format!("keeper {}: the journal named no inbox", &cid[..8]))), "{:?}", lock(&lines));
    tick(&owner).await;
    let named = |e: &&serde_json::Value| e["content"].as_str().unwrap_or("").contains("\"op\":\"keeper\"");
    assert_eq!(w.relay.of_kind(wire::KIND_JOURNAL).iter().filter(named).count(), 1, "named once");

    let to_keeper = || lock(&w.relay.inbox).get(keeper.as_hex()).map_or(0, Vec::len);
    let before = to_keeper();
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    assert_eq!(membership(&reader, &cid).await, MEMBERSHIP_JOINED);
    let had = newest(&reader, &cid).await.map(|k| k.epoch).unwrap_or_else(|| panic!("the reader holds a key"));
    assert_eq!(to_keeper(), before, "nothing more to the keeper's own key");
    assert!(keeper_inbox(&w, &owner, &cid).await >= 1, "the join went to the owner's key");

    owner.svc.rotate_now(&cid).await.unwrap_or_else(|e| panic!("rotate: {e}"));
    tick(&owner).await;
    w.pump(&all).await;
    let now = newest(&reader, &cid).await.map_or(0, |k| k.epoch);
    assert!(now > had, "an epoch after the new series opens for the reader: {had} → {now}");
    for n in all {
        assert!(n.errors().is_empty(), "{:?}", n.errors());
    }
}

/// The reader who asked before the inbox was named (the phone of живой
/// запуск 2026-10-23: «Ждём ключ хранителя» for good): its join went to
/// the keeper's own key, which the keeper of the app never hears. Once the
/// journal names the owner, the reader sends the join there at once, not
/// at its next time hours away, and is let in.
#[tokio::test]
async fn a_join_that_waits_goes_to_the_inbox_the_journal_names_at_once() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &reader];
    let new = new_channel(JoinMode::Open, History::Open, &[H1, H2]);
    let (cid, _) = owner.svc.create_naming(&owner.keys, new, false).await.unwrap_or_else(|e| panic!("create: {e}"));
    // The journal out, no turn of the keeper yet (a pump would run the one
    // the create asked for, and that turn names the inbox).
    flush(&owner).await;
    let keeper = owner.svc.keeper_pubkey(&cid).await.ok().flatten().unwrap_or_else(|| panic!("keeper"));
    let link = owner.svc.share_link(&cid).await.unwrap_or_else(|e| panic!("link: {e}"));
    w.pump(&[&reader]).await;
    reader.svc.open_link(&reader.keys, &link, "").await.unwrap_or_else(|e| panic!("open link: {e}"));
    flush(&reader).await;
    assert_eq!(lock(&w.relay.inbox).get(keeper.as_hex()).map_or(0, Vec::len), 1, "the join went to the keeper's own key");
    // The keeper of the app never hears that key.
    owner.skip_inboxes(&w.relay);

    let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let said = lines.clone();
    reader.svc.set_say(Some(std::sync::Arc::new(move |l: &str| lock(&said).push(l.to_string()))));
    tick(&owner).await;
    w.pump(&all).await;
    assert!(lock(&lines).iter().any(|l| l == &format!("channel.join for {}: the keeper is heard at another inbox now, sent there at once", &cid[..8])), "{:?}", lock(&lines));
    // The reader's turn right after, not ten minutes later.
    tick(&reader).await;
    w.pump(&all).await;
    assert_eq!(membership(&reader, &cid).await, MEMBERSHIP_JOINED, "{:?}", lock(&lines));
    assert!(newest(&reader, &cid).await.is_some());
    assert!(keeper_inbox(&w, &owner, &cid).await >= 1, "the join went to the owner's key");
    for n in all {
        assert!(n.errors().is_empty(), "{:?}", n.errors());
    }
}

/// A keeper held elsewhere (a turn uploading its blobs for minutes) holds
/// back nothing that comes to me (живой запуск 2026-10-23: every gift
/// wrap waited for the lock of every keeper, «an event of kind 1059 held
/// the loop 65 s», and the joins queued meanwhile were never taken): the
/// join is heard at once and waits in the queue, the cycle does not wait
/// for the keeper either, and the turn after the keeper is free lets the
/// reader in.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_keeper_held_elsewhere_holds_back_neither_the_notes_nor_the_cycle() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let said = lines.clone();
    owner.svc.set_say(Some(std::sync::Arc::new(move |l: &str| lock(&said).push(l.to_string()))));
    let link = owner.svc.share_link(&cid).await.unwrap_or_else(|e| panic!("link: {e}"));
    w.pump(&[&reader]).await;
    reader.svc.open_link(&reader.keys, &link, "").await.unwrap_or_else(|e| panic!("open link: {e}"));
    flush(&reader).await;

    let k = owner.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("keeper"));
    let held = k.lock().await;
    let heard = tokio::time::timeout(std::time::Duration::from_secs(5), w.pump(&[&owner])).await;
    assert!(heard.is_ok(), "a wrap waited for the keeper's lock");
    let ticked = tokio::time::timeout(std::time::Duration::from_secs(5), owner.svc.tick()).await;
    assert!(matches!(ticked, Ok(Ok(Some(_)))), "the cycle waited for the keeper, or set no time to come back: {ticked:?}");
    let joiner = &reader.me.as_hex()[..8];
    assert!(lock(&lines).iter().any(|l| l.starts_with(&format!("channel.join from {joiner} for {}: to the keeper", &cid[..8]))), "{:?}", lock(&lines));
    assert!(lock(&lines).iter().any(|l| l == &format!("keeper {}: held elsewhere: its turn waits, again every 5 s", &cid[..8])), "{:?}", lock(&lines));
    assert_ne!(membership(&reader, &cid).await, MEMBERSHIP_JOINED);

    drop(held);
    tick(&owner).await;
    w.pump(&[&owner, &reader]).await;
    assert_eq!(membership(&reader, &cid).await, MEMBERSHIP_JOINED, "{:?}", lock(&lines));
    assert!(newest(&reader, &cid).await.is_some());
    assert!(owner.errors().is_empty() && reader.errors().is_empty(), "{:?} {:?}", owner.errors(), reader.errors());
}

/// The messenger's own gift wraps on my `#p` (a direct message, a receipt)
/// are opened once: every round of listening hands two days of them again
/// on every relay, and opening each anew held the module's loop for
/// minutes while joins waited (живой запуск 2026-10-23). A note of the
/// channels on the same `#p` is still taken.
#[tokio::test]
async fn a_wrap_of_the_messenger_is_opened_once_and_a_join_after_it_is_taken() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    let dm = wire::note_envelope("text", &serde_json::json!({ "text": "hi" })).unwrap_or_else(|e| panic!("{e}"));
    let wrap = crate::notes::wrap_note(&reader.keys, &owner.me, &dm, w.clock.now()).unwrap_or_else(|e| panic!("{e}")).json;
    let id = wrap["id"].as_str().unwrap_or_default().to_string();
    assert!(owner.svc.on_wrap(&owner.keys, &wrap).await.is_ok());
    assert!(lock(&owner.svc.foreign_wraps).contains(&id), "known as the messenger's");
    assert!(matches!(owner.svc.on_wrap(&owner.keys, &wrap).await, Ok(None)), "not opened again");
    join_by_link(&w, &owner, &reader, &cid, &[&owner, &reader]).await;
    assert_eq!(membership(&reader, &cid).await, MEMBERSHIP_JOINED);
    assert_eq!(lock(&owner.svc.foreign_wraps).ids_len(), 1, "a note of the channels is not taken for the messenger's");
}

/// The notes to my key are asked for once for the relays that bring
/// them, whatever the number of channels on them, and live only new ones
/// (`limit` 0): each channel asking for two days of them again had every
/// round hand the module thousands of wraps (живой запуск 2026-10-23).
#[tokio::test]
async fn the_notes_to_a_key_are_asked_for_once_for_all_channels_on_the_same_relays() {
    let w = World::new();
    let owner = w.node().await;
    for _ in 0..3 {
        create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    }
    let me = owner.me.as_hex().to_string();
    let to_me = |f: &serde_json::Value| f["kinds"] == serde_json::json!([crate::notes::KIND_WRAP]) && f["#p"] == serde_json::json!([me.clone()]);
    let subs: Vec<serde_json::Value> = owner
        .svc
        .subscriptions(&owner.keys)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .into_iter()
        .filter_map(|o| if let messenger_core::Outbound::Subscribe { filter, .. } = o { Some(filter.0) } else { None })
        .filter(|f| to_me(f))
        .collect();
    assert_eq!(subs.len(), 1, "{subs:?}");
    assert_eq!(subs[0]["limit"], serde_json::json!(0), "live: new notes only");
    let syncs = owner
        .svc
        .catch_up(&owner.keys)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .into_iter()
        .filter(|o| matches!(o, messenger_core::Outbound::Sync { filter, .. } if to_me(&filter.0)))
        .count();
    assert_eq!(syncs, 1);
}

/// Every subscription of the channels has an id a relay takes: 64 bytes at
/// most (NIP-01, strfry's `MAX_SUBID_SIZE`). With the channel's id in it
/// every one was 77 to 142 bytes, strfry closed them all, and nothing of a
/// channel came live (живой запуск 2026-10-23: a reader's comment never
/// reached the owner's phone). The ids stay apart for every channel and
/// part, the notes of a keeper's own key among them, and the pool still
/// hands them to the module (`channel:`).
#[tokio::test]
async fn every_subscription_of_the_channels_has_an_id_a_relay_takes() {
    let w = World::new();
    let owner = w.node().await;
    create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    // A journal before `inbox`: the notes to the keeper's own key too.
    let new = new_channel(JoinMode::Open, History::Open, &[H1, H2]);
    owner.svc.create_naming(&owner.keys, new, false).await.unwrap_or_else(|e| panic!("create: {e}"));
    let ids: Vec<String> = owner
        .svc
        .subscriptions(&owner.keys)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .into_iter()
        .filter_map(|o| if let messenger_core::Outbound::Subscribe { id, .. } = o { Some(id.0) } else { None })
        .collect();
    assert!(ids.iter().any(|id| id.contains(&format!(":{}:", crate::transport::PART_NOTES))), "a keeper's own key: {ids:?}");
    assert!(ids.iter().all(|id| id.len() <= crate::transport::MAX_SUB_ID && id.starts_with("channel:")), "{ids:?}");
    let unique: std::collections::HashSet<&String> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len(), "{ids:?}");
    assert_eq!(crate::transport::sub_id("", crate::transport::PART_NOTES).0, "channel::notes", "the notes to me outside every channel");
}

/// Joins forged for `n` new readers by the link of the channel, as they
/// would send them to the keeper's inbox.
async fn forged_joins(w: &World, owner: &Node, cid: &str, n: usize) -> Vec<(nostr::key::Keys, serde_json::Value)> {
    let secret = crate::secrets::links(owner.svc.env.secrets.as_ref(), cid).await.ok().and_then(|l| l.get(&0).cloned()).unwrap_or_else(|| panic!("link secret"));
    let to = inbox_of_keeper(owner, cid).await;
    let now = w.clock.now();
    (0..n)
        .map(|_| {
            let k = nostr::key::Keys::generate();
            let who = PubKey::parse(&k.public_key().to_hex()).unwrap_or_else(|| panic!("key"));
            let join = wire::Join { community_id: cid.to_string(), proof: Some(wire::JoinProof { link_epoch: 0, mac: secret.join_mac(cid, &who, 0) }), admit_id: None, created_at: now };
            let content = wire::note_envelope(wire::T_JOIN, &join).unwrap_or_else(|e| panic!("{e}"));
            let wrap = crate::notes::wrap_note(&k, &to, &content, now).unwrap_or_else(|e| panic!("{e}")).json;
            (k, wrap)
        })
        .collect()
}

/// The trial of 1.6, problem 6: wraps are dated up to two days back and
/// strfry hands 500 events a REQ, so a keeper back after a day lost the
/// joins past the newest 500. The catch-up of its inbox now asks page
/// after page until a short one, and the keeper takes the queue oldest
/// first, 120 new readers a minute: 1 200 queued joins, all answered.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_keeper_back_after_a_day_answers_every_join_that_waited() {
    let w = World::new();
    // Gift wraps are dated by the system clock (NIP-59, up to two days
    // back): the world runs on it here.
    let real = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    w.clock.advance(real - w.clock.now());
    let owner = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    *lock(&w.relay.cap) = Some(500);
    let joins = forged_joins(&w, &owner, &cid, 1200).await;
    let to = inbox_of_keeper(&owner, &cid).await;
    lock(&w.relay.inbox).entry(to.as_hex().to_string()).or_default().extend(joins.iter().map(|(_, j)| j.clone()));
    // The keeper's device was off: only its catch-up brings them.
    owner.skip_inboxes(&w.relay);
    w.clock.advance(3600);
    let syncs_before = lock(&owner.net.syncs).len();
    owner.svc.listen(&owner.keys).await.unwrap_or_else(|e| panic!("listen: {e}"));
    w.pump(&[&owner]).await;
    let pages = lock(&owner.net.syncs)[syncs_before..].iter().filter(|f| f["#p"].is_array() && f["until"].is_number()).count();
    assert!(pages >= 5, "page after page of the notes: {pages}");
    // The notes asked for a turn, and the pump ran one: the minute's room
    // of joins taken, the rest wait in the queue.
    let taken = keeper_state(&owner, &cid).await.members.len() - 1;
    assert!(taken <= crate::keeper::JOINS_PER_MINUTE, "{taken}");
    assert_eq!(db::keeper::queued_count(owner.svc.db(), &cid).await.unwrap_or(0) + i64::try_from(taken).unwrap_or(0), 1200, "every join waits for the keeper's turn, or one turn took it");
    for _ in 0..12 {
        tick(&owner).await;
        w.clock.advance(61);
    }
    assert_eq!(db::keeper::queued_count(owner.svc.db(), &cid).await.unwrap_or(-1), 0, "the queue is taken");
    assert_eq!(keeper_state(&owner, &cid).await.members.len(), 1201, "the owner and every joiner");
    let inbox = lock(&w.relay.inbox).clone();
    let answered = joins.iter().filter(|(k, _)| inbox.get(&k.public_key().to_hex()).is_some_and(|l| l.len() == 1)).count();
    assert_eq!(answered, 1200, "one issue to each");
    assert!(owner.errors().is_empty(), "{:?}", owner.errors());
}

/// The notes the keeper sent: to the joiners, and its issues to the owner.
fn keeper_notes_sent(w: &World, owner: &Node, joiners: &[(nostr::key::Keys, serde_json::Value)]) -> usize {
    let inbox = lock(&w.relay.inbox).clone();
    let to_joiners: usize = joiners.iter().map(|(k, _)| inbox.get(&k.public_key().to_hex()).map_or(0, Vec::len)).sum();
    let to_owner = inbox.get(owner.me.as_hex()).into_iter().flatten().filter_map(|x| crate::notes::open(&owner.keys, x).ok()).filter(|o| o.content.contains("\"t\":\"channel.issue\"")).count();
    to_joiners + to_owner
}

/// The trial of 1.6, problem 7: a restarted keeper read two days of notes
/// again and sent every reader its bundle once more (~500 wraps, 14 MB).
/// Now a note is taken once by its rumor (kept in the database), and a
/// join written before its reader was let in is answered already: 500
/// joins replayed after a restart send nothing, nor do they once the
/// rumors taken are forgotten (rows past their week, a database before
/// them).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restarted_keeper_sends_nothing_for_joins_it_answered() {
    let w = World::new();
    let real = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    w.clock.advance(real - w.clock.now());
    let owner = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    let joins = forged_joins(&w, &owner, &cid, 500).await;
    let to = inbox_of_keeper(&owner, &cid).await.as_hex().to_string();
    let k = owner.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("keeper"));
    k.lock().await.limits = crate::keeper::Limits { max_readers: crate::keeper::MAX_READERS, joins_per_minute: 1000, ..Default::default() };
    lock(&w.relay.inbox).entry(to.clone()).or_default().extend(joins.iter().map(|(_, j)| j.clone()));
    w.pump(&[&owner]).await;
    assert_eq!(keeper_state(&owner, &cid).await.members.len(), 501);
    w.clock.advance(60);
    tick(&owner).await;
    // The restart: the keeper comes back from its rows and reads its inbox
    // again, by the catch-up and live.
    owner.svc.unload_keepers().await;
    let before = keeper_notes_sent(&w, &owner, &joins);
    owner.svc.listen(&owner.keys).await.unwrap_or_else(|e| panic!("listen: {e}"));
    w.pump(&[&owner]).await;
    lock(&w.relay.inbox).entry(to.clone()).or_default().extend(joins.iter().map(|(_, j)| j.clone()));
    w.pump(&[&owner]).await;
    tick(&owner).await;
    assert_eq!(keeper_notes_sent(&w, &owner, &joins) - before, 0, "no note sent for 500 joins heard again");
    // The rumors taken forgotten: the dates of the readers still answer.
    sqlx::query("DELETE FROM cm_keeper_wraps").execute(owner.svc.db().pool()).await.unwrap_or_else(|e| panic!("{e}"));
    owner.svc.unload_keepers().await;
    let before = keeper_notes_sent(&w, &owner, &joins);
    lock(&w.relay.inbox).entry(to).or_default().extend(joins.iter().map(|(_, j)| j.clone()));
    w.pump(&[&owner]).await;
    tick(&owner).await;
    assert_eq!(keeper_notes_sent(&w, &owner, &joins) - before, 0, "still none");
    assert_eq!(keeper_state(&owner, &cid).await.members.len(), 501);
    assert!(owner.errors().is_empty(), "{:?}", owner.errors());
}

/// The trial of 1.6, problem 9: the relay answered `timeout` to a few
/// publications of the keeper, and a lost `channel.issue` waited for the
/// reader's next join, ten minutes later. Now the keeper owes it and
/// sends it again on its next turns, each wait twice the one before.
#[tokio::test]
async fn an_issue_the_relay_did_not_take_is_sent_again_by_the_keeper() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    let link = owner.svc.share_link(&cid).await.unwrap_or_else(|e| panic!("link: {e}"));
    w.pump(&[&reader]).await;
    reader.svc.open_link(&reader.keys, &link, "").await.unwrap_or_else(|e| panic!("open link: {e}"));
    flush(&reader).await;
    // The join is on the relay; the keeper's issue is not taken.
    w.relay.refuse(crate::notes::KIND_WRAP, true);
    w.pump(&[&owner]).await;
    assert_eq!(keeper_state(&owner, &cid).await.members.len(), 2, "let in");
    let to_reader = || {
        let wraps = lock(&w.relay.inbox).get(reader.me.as_hex()).cloned().unwrap_or_default();
        wraps.iter().filter_map(|x| crate::notes::open(&reader.keys, x).ok()).filter(|o| o.content.contains("\"t\":\"channel.issue\"")).count()
    };
    assert_eq!(to_reader(), 0, "the issue did not go out");
    // The first wait: nothing yet; still refused: nothing, and a longer wait.
    w.clock.advance(10);
    tick(&owner).await;
    w.clock.advance(10);
    tick(&owner).await;
    assert_eq!(to_reader(), 0);
    w.relay.refuse(crate::notes::KIND_WRAP, false);
    w.clock.advance(20);
    tick(&owner).await;
    assert_eq!(to_reader(), 0, "the second wait is longer");
    w.clock.advance(15);
    tick(&owner).await;
    assert_eq!(to_reader(), 1, "sent again by the keeper, long before the reader's next join");
    w.pump(&[&owner, &reader]).await;
    assert_eq!(membership(&reader, &cid).await, MEMBERSHIP_JOINED);
    assert!(newest(&reader, &cid).await.is_some());
    tick(&owner).await;
    assert_eq!(to_reader(), 1, "and only once");
}

/// A message sealed by `holder` under the key of `epoch`, signed by
/// `author`, dated `at`.
async fn forged_under(holder: &Node, epoch: u32, author: &nostr::key::Keys, cid: &str, links: Links, text: &str, at: i64) -> nostr::prelude::Event {
    forged_envelope_under(holder, epoch, author, cid, links, &Envelope::text(text), at).await
}

/// `forged_under` of any envelope (a deletion, an edit).
async fn forged_envelope_under(holder: &Node, epoch: u32, author: &nostr::key::Keys, cid: &str, links: Links, envelope: &Envelope, at: i64) -> nostr::prelude::Event {
    let rows = db::keys::epochs(holder.svc.db(), cid, SCOPE_EVERYONE).await.unwrap_or_default();
    let hex_key = rows.iter().find(|r| r.epoch == i64::from(epoch)).and_then(|r| r.key_hex.clone()).unwrap_or_else(|| panic!("no key of epoch {epoch}"));
    let raw: [u8; 32] = hex::decode(hex_key).ok().and_then(|b| b.try_into().ok()).unwrap_or_else(|| panic!("key"));
    let key = EpochKey { epoch, key: keydist::Key32(raw) };
    let place = Place::feed(cid);
    let inner = wire::sign_inner(author, &place, &links, &envelope.encode(), at).unwrap_or_else(|e| panic!("{e}"));
    let payload = wire::Payload::event(wire::T_MSG, &inner).unwrap_or_else(|e| panic!("{e}"));
    wire::seal_content(&place, &key, links.outer_target(), &payload, at).unwrap_or_else(|e| panic!("{e}"))
}

/// The trial of 1.6, problem 8: a reader's comments written before their
/// ban stood on the device that heard them then (500) and were refused by
/// one that came after the ban (489: the journal judged at its head). Now
/// a ban is judged at the date of what was written, on every device, and
/// that its author was in shows the key of its epoch: the same comments
/// on both. What the banned author dates after the ban, or seals under an
/// epoch two behind the one current at its date, stands nowhere.
#[tokio::test]
async fn comments_written_before_a_ban_stand_on_a_device_that_came_after_it() {
    let w = World::new();
    let owner = w.node().await;
    let (a, b) = (w.node().await, w.node().await);
    let all = [&owner, &a, &b];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &a, &cid, &all).await;
    join_by_link(&w, &owner, &b, &cid, &all).await;
    w.clock.advance(300);
    tick(&owner).await;
    w.pump(&all).await;
    let (post, _) = owner.svc.send_post(&owner.keys, &cid, "the post", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    // Two epochs more, then the ban: `a` holds the keys of 1, 2 and 3, and
    // comments under 2, one of the last two before the ban.
    w.clock.advance(5);
    owner.svc.act(&owner.keys, &cid, OpBody::RotateNow { scope_id: None }).await.unwrap_or_else(|e| panic!("rotate: {e}"));
    w.pump(&all).await;
    w.clock.advance(5);
    a.svc.send_comment(&a.keys, &cid, &post, None, "before the ban", TextFormat::Plain).await.unwrap_or_else(|e| panic!("comment: {e}"));
    w.pump(&all).await;
    w.clock.advance(5);
    owner.svc.act(&owner.keys, &cid, OpBody::RotateNow { scope_id: None }).await.unwrap_or_else(|e| panic!("rotate: {e}"));
    w.pump(&all).await;
    assert!(has_key(&a, &cid, 3).await);
    w.clock.advance(5);
    let before_ban = w.clock.now();
    w.clock.advance(5);
    owner.svc.act(&owner.keys, &cid, OpBody::Ban { who: pk(&a), reason: None }).await.unwrap_or_else(|e| panic!("ban: {e}"));
    w.pump(&all).await;
    w.clock.advance(600);
    tick(&owner).await;
    w.pump(&all).await;
    let thread = Links { thread: Some(post.clone()), ..Links::default() };
    // After the ban, dated after it: nowhere. Dated before it, under the
    // key of epoch 1 while 3 was current: nowhere. Under 3: as written.
    let late = forged_under(&a, 3, &a.keys, &cid, thread.clone(), "after the ban", w.clock.now()).await;
    let old = forged_under(&a, 1, &a.keys, &cid, thread.clone(), "under an old key", before_ban).await;
    for ev in [&late, &old] {
        w.relay.events.lock().unwrap_or_else(|e| e.into_inner()).push(serde_json::to_value(ev).unwrap_or_default());
    }
    // A device that comes after the ban, and the one that was there.
    let c = w.node().await;
    let all = [&owner, &a, &b, &c];
    join_by_link(&w, &owner, &c, &cid, &all).await;
    w.clock.advance(300);
    tick(&owner).await;
    w.pump(&all).await;
    c.svc.listen(&c.keys).await.unwrap_or_else(|e| panic!("listen: {e}"));
    w.pump(&all).await;
    for n in [&b, &c] {
        let texts = thread_texts(n, &cid, &post).await;
        assert_eq!(texts, vec!["before the ban".to_string()], "the same comments on every device");
    }
}


/// The live run of 2026-10-30 (STA-56): the bots of a host restored on
/// empty folders refused every old post and comment of an author removed
/// after writing them (`message refused: NotMember`), which the devices
/// there at the time keep — a person's new device the same. A removal is
/// judged at the date of what was written, as a ban is: an author put out
/// later, staff then or a reader then, stands on a device that came after,
/// under one of the last two epochs before the removal. What they date
/// before it but seal under an older key they still hold, what they date
/// after it, and a deletion of their own words dated before it, stand
/// nowhere (the review of 2026-10-30).
#[tokio::test]
async fn posts_and_comments_of_an_author_removed_later_stand_on_a_device_that_came_after() {
    let w = World::new();
    let owner = w.node().await;
    let (a, s, b) = (w.node().await, w.node().await, w.node().await);
    let all = [&owner, &a, &s, &b];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    for n in [&a, &s, &b] {
        join_by_link(&w, &owner, n, &cid, &all).await;
    }
    w.clock.advance(300);
    tick(&owner).await;
    w.pump(&all).await;
    let old_epoch = a.svc.member(&a.keys).newest_key(&cid, SCOPE_EVERYONE).await.ok().flatten().map_or(0, |k| k.epoch);
    let in_old_epoch = w.clock.now();
    // Two epochs on; `s` an author: a post of theirs; `a` a reader: a comment under it.
    for _ in 0..2 {
        w.clock.advance(5);
        owner.svc.act(&owner.keys, &cid, OpBody::RotateNow { scope_id: None }).await.unwrap_or_else(|e| panic!("rotate: {e}"));
        w.pump(&all).await;
    }
    w.clock.advance(5);
    owner.svc.act(&owner.keys, &cid, OpBody::SetRole { who: pk(&s), role: Role::Author }).await.unwrap_or_else(|e| panic!("role: {e}"));
    w.pump(&all).await;
    w.clock.advance(5);
    let (post, _) = s.svc.send_post(&s.keys, &cid, "by the author", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    w.clock.advance(5);
    let (comment, _) = a.svc.send_comment(&a.keys, &cid, &post, None, "by the reader", TextFormat::Plain).await.unwrap_or_else(|e| panic!("comment: {e}"));
    w.pump(&all).await;
    assert_eq!(thread_texts(&b, &cid, &post).await, vec!["by the reader".to_string()]);
    let feed_b: Vec<String> = history(&b, &cid, 50, None).await.items.into_iter().filter_map(|i| i.text).collect();
    assert!(feed_b.contains(&"by the author".to_string()), "the post before the removal: {feed_b:?}");
    let current = a.svc.member(&a.keys).newest_key(&cid, SCOPE_EVERYONE).await.ok().flatten().map_or(0, |k| k.epoch);
    assert!(current >= old_epoch + 2, "{old_epoch} → {current}");
    // Both removed (no ban), an epoch past it.
    w.clock.advance(5);
    let before_removal = w.clock.now();
    w.clock.advance(5);
    for n in [&a, &s] {
        owner.svc.act(&owner.keys, &cid, OpBody::Remove { who: pk(n) }).await.unwrap_or_else(|e| panic!("remove: {e}"));
        w.pump(&all).await;
    }
    w.clock.advance(5);
    owner.svc.act(&owner.keys, &cid, OpBody::RotateNow { scope_id: None }).await.unwrap_or_else(|e| panic!("rotate: {e}"));
    w.pump(&all).await;
    w.clock.advance(600);
    tick(&owner).await;
    w.pump(&all).await;
    // Written after the removal: dated after it under the key `a` still
    // holds; dated before it under the old key of three epochs back; a
    // deletion of their own comment dated before it. Nowhere.
    let thread = Links { thread: Some(post.clone()), ..Links::default() };
    let late = forged_under(&a, current, &a.keys, &cid, thread.clone(), "after the removal", w.clock.now()).await;
    let back = forged_under(&a, old_epoch, &a.keys, &cid, thread, "back-dated under an old key", in_old_epoch + 1).await;
    let undo = forged_envelope_under(&a, current, &a.keys, &cid, Links { target: Some(comment.clone()), ..Links::default() }, &Envelope::delete(&comment), before_removal).await;
    for ev in [&late, &back, &undo] {
        inject(&w, ev);
    }
    w.pump(&all).await;
    // A device that comes after the removal.
    let c = w.node().await;
    let all = [&owner, &b, &c];
    join_by_link(&w, &owner, &c, &cid, &all).await;
    w.clock.advance(300);
    tick(&owner).await;
    w.pump(&all).await;
    c.svc.listen(&c.keys).await.unwrap_or_else(|e| panic!("listen: {e}"));
    w.pump(&all).await;
    for n in [&b, &c] {
        let feed: Vec<String> = history(n, &cid, 50, None).await.items.into_iter().filter_map(|i| i.text).collect();
        assert!(feed.contains(&"by the author".to_string()), "the removed author's post on every device: {feed:?}");
        assert_eq!(thread_texts(n, &cid, &post).await, vec!["by the reader".to_string()], "the same comments on every device");
    }
}

/// The review of 2026-10-30: one removed, in again, and removed again
/// held the key of the epoch before the first removal; dated in the time
/// they were out, under that key, nothing of theirs stands (the time out
/// is no time in).
#[tokio::test]
async fn what_one_removed_twice_dates_into_the_time_out_stands_nowhere() {
    let w = World::new();
    let owner = w.node().await;
    let (a, b) = (w.node().await, w.node().await);
    let all = [&owner, &a, &b];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    for n in [&a, &b] {
        join_by_link(&w, &owner, n, &cid, &all).await;
    }
    w.clock.advance(300);
    tick(&owner).await;
    w.pump(&all).await;
    let (post, _) = owner.svc.send_post(&owner.keys, &cid, "the post", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    let first = a.svc.member(&a.keys).newest_key(&cid, SCOPE_EVERYONE).await.ok().flatten().map_or(0, |k| k.epoch);
    // Out, an epoch on, then in again by the link, and out again.
    w.clock.advance(5);
    owner.svc.act(&owner.keys, &cid, OpBody::Remove { who: pk(&a) }).await.unwrap_or_else(|e| panic!("remove: {e}"));
    w.pump(&all).await;
    w.clock.advance(5);
    let while_out = w.clock.now();
    owner.svc.act(&owner.keys, &cid, OpBody::RotateNow { scope_id: None }).await.unwrap_or_else(|e| panic!("rotate: {e}"));
    w.pump(&all).await;
    w.clock.advance(400);
    tick(&owner).await;
    w.pump(&all).await;
    join_by_link(&w, &owner, &a, &cid, &all).await;
    w.clock.advance(300);
    tick(&owner).await;
    w.pump(&all).await;
    w.clock.advance(5);
    owner.svc.act(&owner.keys, &cid, OpBody::Remove { who: pk(&a) }).await.unwrap_or_else(|e| panic!("remove again: {e}"));
    w.pump(&all).await;
    w.clock.advance(5);
    owner.svc.act(&owner.keys, &cid, OpBody::RotateNow { scope_id: None }).await.unwrap_or_else(|e| panic!("rotate: {e}"));
    w.pump(&all).await;
    w.clock.advance(600);
    tick(&owner).await;
    w.pump(&all).await;
    let thread = Links { thread: Some(post.clone()), ..Links::default() };
    inject(&w, &forged_under(&a, first, &a.keys, &cid, thread, "in the time out", while_out).await);
    w.pump(&all).await;
    let c = w.node().await;
    let all = [&owner, &b, &c];
    join_by_link(&w, &owner, &c, &cid, &all).await;
    w.clock.advance(300);
    tick(&owner).await;
    w.pump(&all).await;
    c.svc.listen(&c.keys).await.unwrap_or_else(|e| panic!("listen: {e}"));
    w.pump(&all).await;
    for n in [&b, &c] {
        assert!(thread_texts(n, &cid, &post).await.is_empty(), "nothing dated into the time out");
    }
}

/// The review of 2026-10-30: a removed author's old post that comes to a
/// device after the removal goes by the premoderation in force now, as
/// their new words would; it is not shown before a moderator lets it.
#[tokio::test]
async fn an_old_post_of_one_removed_waits_for_the_premoderation_in_force_now() {
    let w = World::new();
    let owner = w.node().await;
    let s = w.node().await;
    let all = [&owner, &s];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &s, &cid, &all).await;
    w.clock.advance(300);
    tick(&owner).await;
    w.pump(&all).await;
    owner.svc.act(&owner.keys, &cid, OpBody::SetRole { who: pk(&s), role: Role::Author }).await.unwrap_or_else(|e| panic!("role: {e}"));
    w.pump(&all).await;
    w.clock.advance(5);
    let (post, _) = s.svc.send_post(&s.keys, &cid, "by the author", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    w.clock.advance(5);
    owner.svc.act(&owner.keys, &cid, OpBody::Remove { who: pk(&s) }).await.unwrap_or_else(|e| panic!("remove: {e}"));
    w.pump(&all).await;
    w.clock.advance(5);
    owner.svc.act(&owner.keys, &cid, Settings { premod: Some(Premod::All), ..Settings::default() }.body()).await.unwrap_or_else(|e| panic!("premod: {e}"));
    w.pump(&all).await;
    let c = w.node().await;
    let all = [&owner, &c];
    join_by_link(&w, &owner, &c, &cid, &all).await;
    w.clock.advance(300);
    tick(&owner).await;
    w.pump(&all).await;
    c.svc.listen(&c.keys).await.unwrap_or_else(|e| panic!("listen: {e}"));
    w.pump(&all).await;
    let row = db::content::get(c.svc.db(), &post).await.ok().flatten().unwrap_or_else(|| panic!("the post came"));
    assert_eq!(row.review, crate::marks::REVIEW_PENDING, "held by the premoderation in force now");
}
/// The trial of 1.6, problem 10: a reader new to a busy channel caught
/// up the newest 500 events of each kind (strfry's cap of a REQ) and no
/// more. The journal asked for by a link, then the journal, the epochs
/// and the snapshots of the catch-up come page after page until a short
/// one: the new reader holds every operation and every epoch.
#[tokio::test]
async fn a_new_reader_of_a_busy_channel_gets_every_operation_and_epoch() {
    crate::transport::CATCH_UP_LIMIT_TEST.with(|c| c.set(Some(5)));
    let w = World::new();
    let owner = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    for _ in 0..12 {
        w.clock.advance(5);
        let who = PubKey::parse(&nostr::key::Keys::generate().public_key().to_hex()).unwrap_or_else(|| panic!("key"));
        owner.svc.act(&owner.keys, &cid, OpBody::SetTrusted { who, trusted: true }).await.unwrap_or_else(|e| panic!("op: {e}"));
        owner.svc.act(&owner.keys, &cid, OpBody::RotateNow { scope_id: None }).await.unwrap_or_else(|e| panic!("rotate: {e}"));
        w.pump(&[&owner]).await;
    }
    let ops = w.relay.of_kind(wire::KIND_JOURNAL).len();
    let epochs = w.relay.epochs().len();
    assert!(ops > 20 && epochs > 10, "{ops} operations, {epochs} epochs");
    *lock(&w.relay.cap) = Some(5);
    // A new reader: nothing heard live, all of it by asking.
    let reader = w.node().await;
    reader.skip_all(&w.relay);
    let link = owner.svc.share_link(&cid).await.unwrap_or_else(|e| panic!("link: {e}"));
    reader.svc.ask_journal(&cid).await.unwrap_or_else(|e| panic!("ask: {e}"));
    w.pump(&[&reader]).await;
    assert!(reader.svc.log(&cid).await.ok().flatten().is_some(), "the journal down to its create");
    reader.svc.open_link(&reader.keys, &link, "").await.unwrap_or_else(|e| panic!("open link: {e}"));
    flush(&reader).await;
    w.pump(&[&owner]).await;
    reader.skip_all(&w.relay);
    w.pump(&[&reader]).await;
    reader.svc.listen(&reader.keys).await.unwrap_or_else(|e| panic!("listen: {e}"));
    w.pump(&[&reader]).await;
    assert_eq!(membership(&reader, &cid).await, MEMBERSHIP_JOINED);
    assert_eq!(db::channels::ops(reader.svc.db(), &cid).await.unwrap_or_default().len(), w.relay.of_kind(wire::KIND_JOURNAL).len(), "every operation");
    let held = db::keys::epochs(reader.svc.db(), &cid, SCOPE_EVERYONE).await.unwrap_or_default();
    assert_eq!(held.len(), w.relay.epochs().len(), "every epoch");
    assert!(held.iter().all(|r| r.key_hex.is_some()), "every key of the open history");
}

/// The trial of 1.6, run h: under `from_join` a late reader holds the keys
/// from the first epoch after its join on; an admin whose device had not
/// heard that epoch yet (a process that posted right after the rotation)
/// sealed the new post under the epoch before, which the late reader
/// never holds, and the post stayed unread by it. The readers who hold
/// both read it live (`current − 1`). The admin's device, once it hears
/// the epoch, seals again under it what it wrote after the epoch began:
/// the late reader reads the post, the others hold it once.
#[tokio::test]
async fn a_post_sealed_under_the_epoch_before_reaches_a_late_reader_of_from_join() {
    let w = World::new();
    let owner = w.node().await;
    let (admin, early) = (w.node().await, w.node().await);
    let all = [&owner, &admin, &early];
    let cid = create(&w, &owner, JoinMode::Open, History::FromJoin, &[H1, H2]).await;
    join_by_link(&w, &owner, &admin, &cid, &all).await;
    join_by_link(&w, &owner, &early, &cid, &all).await;
    owner.svc.act(&owner.keys, &cid, OpBody::SetRole { who: pk(&admin), role: Role::Admin }).await.unwrap_or_else(|e| panic!("{e}"));
    for _ in 0..2 {
        w.clock.advance(10);
        owner.svc.act(&owner.keys, &cid, OpBody::RotateNow { scope_id: None }).await.unwrap_or_else(|e| panic!("{e}"));
        w.pump(&all).await;
    }
    let before = *w.relay.epochs().last().unwrap_or(&0);
    // The late reader joins; the first epoch after its join is its first key.
    let late = w.node().await;
    let all = [&owner, &admin, &early, &late];
    join_by_link(&w, &owner, &late, &cid, &all).await;
    // The admin's device does not hear the epoch of the rotation.
    admin.hold(KIND_EPOCH, true);
    w.clock.advance(600);
    tick(&owner).await;
    w.pump(&all).await;
    let next = *w.relay.epochs().last().unwrap_or(&0);
    assert!(next > before, "an epoch after the join");
    assert!(has_key(&late, &cid, next).await && !has_key(&late, &cid, before).await, "from its join on");
    assert_eq!(newest(&admin, &cid).await.map(|k| k.epoch), Some(before), "the admin is an epoch behind");
    w.clock.advance(5);
    let (post, _) = admin.svc.send_post(&admin.keys, &cid, "right after the rotation", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    assert!(post_texts(&early, &cid).await.contains(&"right after the rotation".to_string()), "live under current − 1");
    // The admin hears the epoch: what it wrote after it began goes out again under it.
    admin.hold(KIND_EPOCH, false);
    w.pump(&all).await;
    assert!(post_texts(&late, &cid).await.contains(&"right after the rotation".to_string()), "the late reader reads it");
    let copies = w.relay.of_kind(KIND_CONTENT).iter().filter(|e| e["tags"].as_array().is_some_and(|t| t.iter().any(|t| t[0] == "n" && t[1].as_str().and_then(|s| s.parse::<u32>().ok()) == Some(next)))).count();
    assert!(copies >= 1);
    assert_eq!(db::content::posts(early.svc.db(), &cid, None, 100).await.unwrap_or_default().iter().filter(|p| p.id == post).count(), 1, "held once");
}

// ─── Levels and sections (wire §14) ─────────────────────────────────────────

use crate::op::LevelCan;
use keydist::SchemeId;

/// A section of the channel, made by `by` (admin+).
async fn section(w: &World, by: &Node, cid: &str, title: &str, scheme: SchemeId, all: &[&Node]) -> String {
    let (scope_id, _) = by.svc.scope_add(&by.keys, cid, title, scheme, History::Open).await.unwrap_or_else(|e| panic!("scope_add: {e}"));
    w.pump(all).await;
    scope_id
}

/// A level giving `scopes`, made by `by` (admin+): its id.
async fn level(w: &World, by: &Node, cid: &str, name: &str, scopes: &[&str], all: &[&Node]) -> u8 {
    let scopes = scopes.iter().map(|s| s.to_string()).collect();
    let (id, _) = by.svc.level_set(&by.keys, cid, None, name, scopes, LevelCan::BASE).await.unwrap_or_else(|e| panic!("level_set: {e}"));
    w.pump(all).await;
    id
}

async fn set_levels(w: &World, by: &Node, cid: &str, who: &Node, levels: &[u8], all: &[&Node]) {
    by.svc.set_levels(&by.keys, cid, &pk(who), levels).await.unwrap_or_else(|e| panic!("set_levels: {e}"));
    w.pump(all).await;
}

async fn scope_key(n: &Node, cid: &str, scope_id: &str) -> Option<EpochKey> {
    n.svc.member(&n.keys).newest_key(cid, scope_id).await.unwrap_or_else(|e| panic!("keys: {e}"))
}

/// The members of a scope at the keeper of `owner`.
async fn scope_members(owner: &Node, cid: &str, scope_id: &str) -> usize {
    let k = owner.svc.keeper(cid).await.ok().flatten().unwrap_or_else(|| panic!("not kept here"));
    let k = k.lock().await;
    k.scope_keeper(scope_id).map_or(0, |s| s.members_len())
}

/// A channel of the owner with one reader by the link.
async fn channel_with_reader(w: &World) -> (Node, Node, String) {
    let owner = w.node().await;
    let reader = w.node().await;
    let cid = create(w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(w, &owner, &reader, &cid, &[&owner, &reader]).await;
    assert!(newest(&reader, &cid).await.is_some(), "the reader holds a key of everyone");
    (owner, reader, cid)
}

#[tokio::test]
async fn a_grant_of_levels_issues_the_new_section_only() {
    let w = World::new();
    let (owner, reader, cid) = channel_with_reader(&w).await;
    let all = [&owner, &reader];
    let gold = section(&w, &owner, &cid, "Premium", SchemeId::SdV1, &all).await;
    assert_eq!(w.relay.epochs_of(&gold), vec![1], "the owner (staff) is its first member: its first epoch");
    assert!(scope_key(&owner, &cid, &gold).await.is_some(), "staff hold every section");
    assert!(scope_key(&reader, &cid, &gold).await.is_none());
    let lvl = level(&w, &owner, &cid, "Gold", &[&gold], &all).await;
    let everyone = w.relay.epochs_of(SCOPE_EVERYONE);
    let sent = notes_to(&owner, &reader);
    set_levels(&w, &owner, &cid, &reader, &[lvl], &all).await;
    assert_eq!(notes_to(&owner, &reader) - sent, 1, "one issue: of the new section");
    tick(&owner).await;
    w.pump(&all).await;
    assert!(scope_key(&reader, &cid, &gold).await.is_some(), "the key of the section came with its issue (open history)");
    assert_eq!(w.relay.epochs_of(SCOPE_EVERYONE), everyone, "everyone did not move");
    assert_eq!(membership(&reader, &cid).await, MEMBERSHIP_JOINED);
    assert!(reader.errors().is_empty(), "{:?}", reader.errors());
}

#[tokio::test]
async fn a_level_taken_away_rotates_its_section_only_and_the_reader_keeps_everyone() {
    let w = World::new();
    let (owner, reader, cid) = channel_with_reader(&w).await;
    let all = [&owner, &reader];
    let gold = section(&w, &owner, &cid, "Premium", SchemeId::SdV1, &all).await;
    let lvl = level(&w, &owner, &cid, "Gold", &[&gold], &all).await;
    set_levels(&w, &owner, &cid, &reader, &[lvl], &all).await;
    tick(&owner).await;
    w.pump(&all).await;
    let before = scope_key(&reader, &cid, &gold).await.map(|k| k.epoch).unwrap_or_else(|| panic!("in the section"));
    let everyone = w.relay.epochs_of(SCOPE_EVERYONE);
    set_levels(&w, &owner, &cid, &reader, &[], &all).await;
    assert_eq!(scope_members(&owner, &cid, &gold).await, 1, "only the owner left in the section");
    w.clock.advance(400);
    tick(&owner).await;
    w.pump(&all).await;
    let gold_epochs = w.relay.epochs_of(&gold);
    assert!(gold_epochs.iter().any(|e| *e > before), "the section moved: {gold_epochs:?}");
    assert_eq!(scope_key(&reader, &cid, &gold).await.map(|k| k.epoch), Some(before), "the reader holds no newer key of it");
    assert_eq!(w.relay.epochs_of(SCOPE_EVERYONE), everyone, "everyone did not move");
    assert_eq!(membership(&reader, &cid).await, MEMBERSHIP_JOINED, "a closed section is not out of the channel");
    assert!(scope_key(&reader, &cid, SCOPE_EVERYONE).await.is_some());
}

#[tokio::test]
async fn two_levels_giving_one_section_rekey_nothing_on_a_rename_or_one_of_them_gone() {
    let w = World::new();
    let (owner, reader, cid) = channel_with_reader(&w).await;
    let all = [&owner, &reader];
    let gold = section(&w, &owner, &cid, "Premium", SchemeId::SdV1, &all).await;
    let a = level(&w, &owner, &cid, "Gold", &[&gold], &all).await;
    let b = level(&w, &owner, &cid, "Patron", &[&gold], &all).await;
    set_levels(&w, &owner, &cid, &reader, &[a, b], &all).await;
    tick(&owner).await;
    w.pump(&all).await;
    let epochs = w.relay.epochs_of(&gold);
    // A rename: the same scopes, a new name.
    owner.svc.level_set(&owner.keys, &cid, Some(a), "Golden", vec![gold.clone()], LevelCan::BASE).await.unwrap_or_else(|e| panic!("rename: {e}"));
    w.pump(&all).await;
    let log = reader.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("journal"));
    assert_eq!(reader.svc.names(&log).await.unwrap_or_default().level(a), "Golden", "the reader reads the new name");
    // One of the two goes; the other still gives the section.
    owner.svc.level_del(&owner.keys, &cid, a).await.unwrap_or_else(|e| panic!("level_del: {e}"));
    w.pump(&all).await;
    w.clock.advance(400);
    tick(&owner).await;
    w.pump(&all).await;
    assert_eq!(w.relay.epochs_of(&gold), epochs, "no epoch of the section");
    assert_eq!(scope_members(&owner, &cid, &gold).await, 2);
}

#[tokio::test]
async fn a_newcomer_with_two_levels_gets_an_issue_for_each_section() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    let all = [&owner, &reader];
    let gold = section(&w, &owner, &cid, "Premium", SchemeId::SdV1, &all).await;
    let silver = section(&w, &owner, &cid, "Club", SchemeId::SdV1, &all).await;
    let a = level(&w, &owner, &cid, "Gold", &[&gold], &all).await;
    let b = level(&w, &owner, &cid, "Silver", &[&silver], &all).await;
    // Levels given before the reader came (a payment bot ahead of the join).
    set_levels(&w, &owner, &cid, &reader, &[a, b], &all).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    assert_eq!(notes_to(&owner, &reader), 3, "everyone, gold, silver");
    tick(&owner).await;
    w.pump(&all).await;
    assert!(scope_key(&reader, &cid, &gold).await.is_some() && scope_key(&reader, &cid, &silver).await.is_some());
    // The snapshot names the levels of the reader.
    w.clock.advance(600);
    tick(&owner).await;
    let snap = w.relay.of_kind(KIND_SNAPSHOT).last().cloned().unwrap_or_else(|| panic!("a snapshot"));
    let s = crate::snapshot::content_of(&serde_json::to_string(&snap).unwrap_or_default()).unwrap_or_else(|e| panic!("snapshot: {e}"));
    let key = newest(&owner, &cid).await.unwrap_or_else(|| panic!("key"));
    let blob = crate::blobs::BlobHosts::get(w.hosts.as_ref(), H1, &s.shards[0].sha256).await.ok().flatten().unwrap_or_else(|| panic!("shard"));
    let entries = crate::snapshot::open_shard(&cid, &key, &blob, &s.shards[0].sha256).unwrap_or_else(|e| panic!("open: {e}"));
    let mine = entries.iter().find(|e| hex::encode(e.pubkey) == pk(&reader).as_hex()).unwrap_or_else(|| panic!("the reader"));
    assert_eq!(mine.levels, 1 | 1 << a | 1 << b);
}

#[tokio::test]
async fn a_flat_section_of_the_staff_hands_each_epoch_by_issue() {
    let w = World::new();
    let (owner, reader, cid) = channel_with_reader(&w).await;
    let moderator = w.node().await;
    let all = [&owner, &reader, &moderator];
    join_by_link(&w, &owner, &moderator, &cid, &all).await;
    owner.svc.act(&owner.keys, &cid, OpBody::SetRole { who: pk(&moderator), role: Role::Moderator }).await.unwrap_or_else(|e| panic!("role: {e}"));
    w.pump(&all).await;
    let staff = section(&w, &owner, &cid, "Staff room", SchemeId::FlatV1, &all).await;
    tick(&owner).await;
    w.pump(&all).await;
    assert_eq!(scope_members(&owner, &cid, &staff).await, 2, "owner and moderator");
    let k1 = scope_key(&moderator, &cid, &staff).await.unwrap_or_else(|| panic!("the moderator holds the key"));
    assert_eq!(scope_key(&owner, &cid, &staff).await.map(|k| k.epoch), Some(k1.epoch));
    assert!(scope_key(&reader, &cid, &staff).await.is_none(), "a reader is not staff");
    // The moderator steps down: the next flat epoch is not theirs.
    owner.svc.act(&owner.keys, &cid, OpBody::SetRole { who: pk(&moderator), role: Role::Member }).await.unwrap_or_else(|e| panic!("role: {e}"));
    w.pump(&all).await;
    w.clock.advance(400);
    tick(&owner).await;
    w.pump(&all).await;
    let k2 = scope_key(&owner, &cid, &staff).await.unwrap_or_else(|| panic!("owner key"));
    assert!(k2.epoch > k1.epoch);
    assert_eq!(scope_key(&moderator, &cid, &staff).await.map(|k| k.epoch), Some(k1.epoch));
    assert_eq!(membership(&moderator, &cid).await, MEMBERSHIP_JOINED);
}

/// How many notes the device of `from` sent to the inbox of `to`.
fn notes_to(from: &Node, to: &Node) -> usize {
    from.net.hints.lock().map(|h| h.iter().filter(|(r, _)| r == pk(to).as_hex()).count()).unwrap_or(0)
}

#[tokio::test]
async fn a_section_opens_only_to_the_readers_of_a_level_that_gives_it() {
    let w = World::new();
    let (owner, gold_reader, cid) = channel_with_reader(&w).await;
    let plain = w.node().await;
    let all = [&owner, &gold_reader, &plain];
    join_by_link(&w, &owner, &plain, &cid, &all).await;
    let gold = section(&w, &owner, &cid, "Premium", SchemeId::SdV1, &all).await;
    let lvl = level(&w, &owner, &cid, "Gold", &[&gold], &all).await;
    set_levels(&w, &owner, &cid, &gold_reader, &[lvl], &all).await;
    tick(&owner).await;
    w.pump(&all).await;
    // A snapshot that names the readers, for their comments.
    w.clock.advance(600);
    tick(&owner).await;
    w.pump(&all).await;
    let (post, _) = owner.svc.send_post_in(&owner.keys, &cid, Some(&gold), "premium", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    owner.svc.send_post(&owner.keys, &cid, "for all", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    let mut texts = post_texts(&gold_reader, &cid).await;
    texts.sort();
    assert_eq!(texts, ["for all", "premium"]);
    assert_eq!(post_texts(&plain, &cid).await, ["for all"], "the section is not even kept");
    let page = gold_reader.svc.history_page_in(&gold_reader.keys, &cid, Some(&gold), 20, None).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(page.items.iter().map(|i| (i.text.clone().unwrap_or_default(), i.scope_id.clone())).collect::<Vec<_>>(), [("premium".to_string(), gold.clone())]);
    let page = plain.svc.history_page_in(&plain.keys, &cid, Some(&gold), 20, None).await.unwrap_or_else(|e| panic!("{e}"));
    assert!(page.items.is_empty());
    let err = plain.svc.send_post_in(&plain.keys, &cid, Some(&gold), "in", TextFormat::Plain).await.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("not member of scope"), "{err}");
    // A comment goes into the scope of its post.
    gold_reader.svc.send_comment(&gold_reader.keys, &cid, &post, None, "thanks", TextFormat::Plain).await.unwrap_or_else(|e| panic!("comment: {e}"));
    w.pump(&all).await;
    let thread = owner.svc.thread_page(&owner.keys, &cid, &post, 20, None).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(thread.items.iter().map(|i| i.scope_id.as_str()).collect::<Vec<_>>(), [gold.as_str()]);
    assert!(plain.errors().is_empty() && gold_reader.errors().is_empty(), "{:?} {:?}", plain.errors(), gold_reader.errors());
}

#[tokio::test]
async fn a_request_for_a_level_granted_brings_the_keys_and_refused_brings_the_reason() {
    let w = World::new();
    let (owner, reader, cid) = channel_with_reader(&w).await;
    let all = [&owner, &reader];
    let gold = section(&w, &owner, &cid, "Premium", SchemeId::SdV1, &all).await;
    let lvl = level(&w, &owner, &cid, "Gold", &[&gold], &all).await;
    let other = level(&w, &owner, &cid, "Silver", &[], &all).await;
    reader.svc.level_request(&reader.keys, &cid, lvl, "please").await.unwrap_or_else(|e| panic!("request: {e}"));
    reader.svc.level_request(&reader.keys, &cid, other, "").await.unwrap_or_else(|e| panic!("request: {e}"));
    assert!(reader.svc.level_request(&reader.keys, &cid, 0, "").await.is_err(), "nobody asks for the base level");
    w.pump(&all).await;
    let waiting = owner.svc.level_requests(&cid).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(waiting.iter().map(|r| (r.level, r.note.as_str())).collect::<Vec<_>>(), [(i64::from(lvl), "please"), (i64::from(other), "")]);
    assert!(reader.svc.level_requests(&cid).await.unwrap_or_default().is_empty(), "a reader is no moderator");
    // Granted: the journal gives the level, the keeper the section.
    owner.svc.level_request_act(&owner.keys, &cid, &pk(&reader), lvl, true, None).await.unwrap_or_else(|e| panic!("grant: {e}"));
    w.pump(&all).await;
    tick(&owner).await;
    w.pump(&all).await;
    assert!(scope_key(&reader, &cid, &gold).await.is_some(), "the keys of the section came");
    let mine = reader.svc.my_level_requests(&cid).await.unwrap_or_default();
    assert!(mine.iter().any(|r| r.level == i64::from(lvl) && r.status == db::levels::REQ_GRANTED));
    // Refused: a note with the reason, the journal untouched.
    let ops = owner.svc.log(&cid).await.ok().flatten().map(|l| l.ordered().count()).unwrap_or(0);
    owner.svc.level_request_act(&owner.keys, &cid, &pk(&reader), other, false, Some("not yet")).await.unwrap_or_else(|e| panic!("refuse: {e}"));
    w.pump(&all).await;
    let mine = reader.svc.my_level_requests(&cid).await.unwrap_or_default();
    let refused = mine.iter().find(|r| r.level == i64::from(other)).unwrap_or_else(|| panic!("my request"));
    assert_eq!((refused.status.as_str(), refused.reason.as_deref()), (db::levels::REQ_REFUSED, Some("not yet")));
    assert_eq!(owner.svc.log(&cid).await.ok().flatten().map(|l| l.ordered().count()).unwrap_or(0), ops);
    assert!(owner.svc.level_requests(&cid).await.unwrap_or_default().is_empty());
    assert!(reader.errors().is_empty(), "{:?}", reader.errors());
}

#[tokio::test]
async fn the_base_level_stays_and_a_channel_holds_sixteen_scopes() {
    let w = World::new();
    let owner = w.node().await;
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    assert!(err_text(owner.svc.level_del(&owner.keys, &cid, 0).await).contains("level last"));
    owner.svc.level_set(&owner.keys, &cid, Some(0), "Free", vec![], LevelCan::BASE).await.unwrap_or_else(|e| panic!("rename: {e}"));
    w.pump(&[&owner]).await;
    let log = owner.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("journal"));
    assert_eq!(owner.svc.names(&log).await.unwrap_or_default().level(0), "Free");
    assert!(err_text(owner.svc.level_set(&owner.keys, &cid, None, "FREE", vec![], LevelCan::BASE).await.map(|_| ())).contains("level name"), "a name of another level");
    assert!(err_text(owner.svc.level_set(&owner.keys, &cid, None, " ", vec![], LevelCan::BASE).await.map(|_| ())).contains("level name"));
    for i in 0..15 {
        section(&w, &owner, &cid, &format!("S{i}"), SchemeId::SdV1, &[&owner]).await;
    }
    assert!(err_text(owner.svc.scope_add(&owner.keys, &cid, "one more", SchemeId::SdV1, History::Open).await.map(|_| ())).contains("scope limit"));
    assert!(err_text(owner.svc.scope_del(&owner.keys, &cid, SCOPE_EVERYONE).await).contains("scope everyone"));
    let log = owner.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("journal"));
    assert_eq!(log.state().scopes.len(), 16);
    let names = owner.svc.names(&log).await.unwrap_or_default();
    assert_eq!(names.scopes.len(), 15, "every title, read back");
}

fn err_text<T>(r: messenger_core::Result<T>) -> String {
    r.err().map(|e| e.to_string()).unwrap_or_default()
}

/// The owner's model of 2026-10-23: `can.post` of a level lets its readers
/// post into the scopes of that level only; `everyone` only when a posting
/// level names it. A post sealed elsewhere by a reader is dropped by every
/// device, whatever key the reader holds.
#[tokio::test]
async fn a_posting_level_lets_post_only_into_its_own_sections() {
    let w = World::new();
    let (owner, writer, cid) = channel_with_reader(&w).await;
    let all = [&owner, &writer];
    let gold = section(&w, &owner, &cid, "Premium", SchemeId::SdV1, &all).await;
    let posting = LevelCan { comment: true, discuss: true, post: true };
    let (lvl, _) = owner.svc.level_set(&owner.keys, &cid, None, "Writers", vec![gold.clone()], posting).await.unwrap_or_else(|e| panic!("level_set: {e}"));
    w.pump(&all).await;
    set_levels(&w, &owner, &cid, &writer, &[lvl], &all).await;
    tick(&owner).await;
    w.pump(&all).await;
    w.clock.advance(600);
    tick(&owner).await;
    w.pump(&all).await;
    let state = writer.svc.log(&cid).await.ok().flatten().map(|l| l.state().clone()).unwrap_or_else(|| panic!("journal"));
    assert_eq!(state.post_scopes_of(&pk(&writer)).into_iter().collect::<Vec<_>>(), std::slice::from_ref(&gold));
    assert_eq!(state.post_scopes_of(&pk(&owner)).len(), 2, "the owner posts everywhere");
    writer.svc.send_post_in(&writer.keys, &cid, Some(&gold), "in gold", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    let err = writer.svc.send_post(&writer.keys, &cid, "in everyone", TextFormat::Plain).await.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("no post in this scope"), "{err}");
    // Past the check of the sender: sealed by hand into everyone's feed.
    let envelope = Envelope::text("forged into everyone");
    writer.svc.seal_and_send_in(&writer.keys, &cid, SCOPE_EVERYONE, Links::default(), envelope, wire::T_MSG).await.unwrap_or_else(|e| panic!("seal: {e}"));
    w.pump(&all).await;
    assert_eq!(post_texts(&owner, &cid).await, ["in gold"], "the post out of its sections is dropped");
    // A level that names everyone too lets post there.
    owner.svc.level_set(&owner.keys, &cid, Some(lvl), "Writers", vec![SCOPE_EVERYONE.to_string(), gold.clone()], posting).await.unwrap_or_else(|e| panic!("level_set: {e}"));
    w.pump(&all).await;
    writer.svc.send_post(&writer.keys, &cid, "now in everyone", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    let mut texts = post_texts(&owner, &cid).await;
    texts.sort();
    assert_eq!(texts, ["in gold", "now in everyone"]);
    // The base level lets nobody post: a reader without the level posts nowhere.
    owner.svc.set_levels(&owner.keys, &cid, &pk(&writer), &[]).await.unwrap_or_else(|e| panic!("set_levels: {e}"));
    w.pump(&all).await;
    assert!(writer.svc.send_post_in(&writer.keys, &cid, Some(SCOPE_EVERYONE), "x", TextFormat::Plain).await.is_err());
}

// ─── One device, its database written from several tasks ────────────────────

/// What waits for a key is taken while another task of the device writes
/// (the night trial of 2026-10-23: a journal operation of one reader of
/// a thousand failed with `database is locked`, code 517, and was lost).
/// `take_pending` read the rows and then deleted them in one deferred
/// transaction: a write of another connection between the two made the
/// delete fail on its stale snapshot (SQLITE_BUSY_SNAPSHOT), which no
/// busy timeout waits out.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_waits_is_taken_while_the_device_writes_elsewhere() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let db = db::Db::open(&dir.path().join(db::FILE_NAME)).await.unwrap_or_else(|e| panic!("db: {e}"));
    let row = |i: u64, key: &str| db::channels::PendingRow {
        event_id: format!("{i:064x}"),
        community_id: "c".into(),
        scope_id: SCOPE_EVERYONE.into(),
        epoch: 1,
        key_id: key.into(),
        event_json: "{}".into(),
        created_at: i as i64,
        received_at: 0,
    };
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // Other tasks of the device write all the time.
    let mut writers = Vec::new();
    for w in 0..3u64 {
        let (db, stop) = (db.clone(), stop.clone());
        writers.push(tokio::spawn(async move {
            let mut i = 1_000_000 * (w + 1);
            while !stop.load(Ordering::Relaxed) {
                db::channels::add_pending(&db, &row(i, "other")).await.unwrap_or_else(|e| panic!("write: {e}"));
                i += 1;
            }
        }));
    }
    let mut taken = 0;
    for round in 0..300u64 {
        for j in 0..3 {
            db::channels::add_pending(&db, &row(round * 10 + j, "k")).await.unwrap_or_else(|e| panic!("add: {e}"));
        }
        let rows = db::channels::take_pending(&db, "c", "k").await.unwrap_or_else(|e| panic!("round {round}: take: {e}"));
        assert!(rows.windows(2).all(|w| (w[0].created_at, &w[0].event_id) <= (w[1].created_at, &w[1].event_id)), "oldest first");
        taken += rows.len();
    }
    stop.store(true, Ordering::Relaxed);
    for w in writers {
        w.await.unwrap_or_else(|e| panic!("{e}"));
    }
    assert_eq!(taken, 900, "every row taken once");
}

/// An epoch whose hosts are down holds neither the keeper's other work
/// nor the events (живой запуск 2026-10-23: a turn held the channel's
/// lock and the loop across its uploads): a reader who joins meanwhile is
/// let in by the same pump, the hosts back bring the epoch out, and the
/// rows of the uploads name what the hosts hold and nothing else.
#[tokio::test]
async fn an_epoch_held_by_its_hosts_holds_neither_the_keeper_nor_the_events() {
    let w = World::new();
    let owner = w.node().await;
    let reader = w.node().await;
    w.hosts.set_down(H1, true);
    w.hosts.set_down(H2, true);
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    assert!(w.relay.epochs().is_empty(), "the first epoch waits for a host");
    assert_eq!(owner.svc.keeper_status(&cid).and_then(|s| s.pending_epoch), Some(1));
    join_by_link(&w, &owner, &reader, &cid, &[&owner, &reader]).await;
    assert_eq!(membership(&reader, &cid).await, MEMBERSHIP_JOINED, "let in while the epoch waits");
    assert!(w.relay.epochs().is_empty());
    assert!(db::keeper::uploads(owner.svc.db(), &cid, SCOPE_EVERYONE, 1).await.unwrap_or_default().is_empty(), "no host took anything");
    // One host back: the epoch goes out; the rows name that host alone.
    w.hosts.set_down(H1, false);
    tick(&owner).await;
    assert_eq!(w.relay.epochs(), vec![1]);
    let rows = db::keeper::uploads(owner.svc.db(), &cid, SCOPE_EVERYONE, 1).await.unwrap_or_default();
    assert!(!rows.is_empty() && rows.iter().all(|(h, _)| h == H1), "only the host that took them: {rows:?}");
    // The other back: it catches up.
    w.hosts.set_down(H2, false);
    tick(&owner).await;
    let rows = db::keeper::uploads(owner.svc.db(), &cid, SCOPE_EVERYONE, 1).await.unwrap_or_default();
    let blobs: std::collections::BTreeSet<&String> = rows.iter().map(|(_, s)| s).collect();
    for sha in &blobs {
        assert!(rows.contains(&(H1.to_string(), (*sha).clone())) && rows.contains(&(H2.to_string(), (*sha).clone())), "both hosts hold {sha}");
    }
    assert_eq!(rows.len(), blobs.len() * 2);
    w.pump(&[&owner, &reader]).await;
    assert!(has_key(&reader, &cid, 1).await);
    assert!(owner.errors().is_empty() && reader.errors().is_empty(), "{:?} {:?}", owner.errors(), reader.errors());
}

/// A pending epoch superseded while its blobs still go up (a host slow to
/// answer) cancels its upload: the job stops before its next put, and
/// the epoch that took its place goes out instead.
#[tokio::test]
async fn a_superseded_pending_epoch_cancels_its_upload() {
    let w = World::new();
    let owner = w.node().await;
    let lines = said(&owner);
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    assert_eq!(w.relay.epochs(), vec![1]);
    let gate = std::sync::Arc::new(tokio::sync::Notify::new());
    *lock(&w.hosts.hang) = Some(gate.clone());
    let puts = w.hosts.puts.load(Ordering::SeqCst);
    owner.svc.rotate_now(&cid).await.unwrap_or_else(|e| panic!("rotate: {e}"));
    let turned = owner.svc.turn().await.unwrap_or_else(|e| panic!("turn: {e}"));
    let key = crate::keeper::upload::put_key(&cid, SCOPE_EVERYONE, 2);
    assert!(turned.jobs.iter().any(|j| j.key() == key), "the blobs of epoch 2 are a job: {:?}", turned.jobs.iter().map(|j| j.key()).collect::<Vec<_>>());
    assert!(owner.svc.jobs_running().contains(&key));
    // The job runs beside the turn and hangs on its first put.
    let svc = owner.svc.clone();
    let running = tokio::spawn(async move { svc.run_jobs(turned.jobs).await });
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert_eq!(w.hosts.puts.load(Ordering::SeqCst), puts, "no put went through");
    // An urgent epoch meanwhile: the pending one is replaced, its job told to stop.
    w.clock.advance(5);
    owner.svc.rotate_now(&cid).await.unwrap_or_else(|e| panic!("rotate: {e}"));
    let next = owner.svc.turn().await.unwrap_or_else(|e| panic!("turn: {e}"));
    assert_eq!(keeper_state(&owner, &cid).await.pending.map(|p| p.key.epoch), Some(3), "3 took the place of 2");
    assert!(lock(&lines).iter().any(|l| l.contains("the blobs of epoch 2 are no longer wanted")), "{:?}", lock(&lines));
    *lock(&w.hosts.hang) = None;
    gate.notify_waiters();
    running.await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(w.hosts.puts.load(Ordering::SeqCst), puts + 2, "the puts in flight (one a host, the hosts at once) ended, no other followed");
    assert!(!owner.svc.jobs_running().contains(&key), "the job is gone");
    assert_eq!(w.relay.epochs(), vec![1], "no event of a burnt epoch");
    // The epoch that took its place goes out.
    owner.svc.run_jobs(next.jobs).await;
    tick(&owner).await;
    assert_eq!(w.relay.epochs(), vec![1, 3]);
    assert!(owner.errors().is_empty(), "{:?}", owner.errors());
}

/// «Typing» under a post (2026-10-24): the ephemeral twin of content,
/// opened under the key of the section, sent to the relays at once (not
/// through the outbox), a word for the screen alone — no message of it,
/// nothing of my own heard back.
#[tokio::test]
async fn a_typing_under_a_post_is_a_word_for_the_screen_alone() {
    let w = World::new();
    let owner = w.node().await;
    let a = w.node().await;
    let all = [&owner, &a];
    let cid = create(&w, &owner, JoinMode::Open, History::Open, &[H1, H2]).await;
    join_by_link(&w, &owner, &a, &cid, &all).await;
    w.clock.advance(300);
    tick(&owner).await;
    w.pump(&all).await;
    let (post, _) = owner.svc.send_post(&owner.keys, &cid, "the post", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    let before = w.relay.of_kind(wire::KIND_TYPING).len();
    a.svc.send_typing(&a.keys, &cid, &post).await.unwrap_or_else(|e| panic!("typing: {e}"));
    let typed = w.relay.of_kind(wire::KIND_TYPING);
    assert_eq!(typed.len(), before + 1, "sent to the relays at once");
    let ev = typed.last().cloned().unwrap_or_else(|| panic!("the typing on the relay"));
    let out = owner.svc.on_event(&owner.keys, &ev, true).await.unwrap_or_else(|e| panic!("heard: {e}"));
    let typing: Vec<_> = out.events.iter().filter(|e| e.name == crate::service::UI_EVENT_CHANNEL_TYPING).collect();
    assert_eq!(typing.len(), 1, "{:?}", out.events);
    assert_eq!(typing[0].payload["post_id"], post);
    assert_eq!(typing[0].payload["author"], a.me.as_hex());
    let out = a.svc.on_event(&a.keys, &ev, true).await.unwrap_or_else(|e| panic!("{e}"));
    assert!(out.events.iter().all(|e| e.name != crate::service::UI_EVENT_CHANNEL_TYPING), "my own says nothing");
    w.pump(&all).await;
    assert_eq!(history(&owner, &cid, 10, None).await.items.len(), 1, "no message of it");
}
