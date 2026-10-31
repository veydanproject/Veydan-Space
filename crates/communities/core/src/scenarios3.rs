// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Scenarios of stage 3 (internal/communities-wire.md §15–§17): the keeper
//! moving between the owner's devices and recovered when one is lost, the
//! channel handed to a new owner, the tickets of the moderators.

use crate::db;
use crate::op::{History, JoinMode};
use crate::secrets;
use crate::service::{NewChannel, MEMBERSHIP_JOINED};
use crate::state::SCOPE_EVERYONE;
use crate::testkit::{Node, World, H1, H2};
use crate::wire::KIND_EPOCH;
use keydist::EpochKey;
use messenger_core::{PubKey, TextFormat};

fn new_channel(join: JoinMode) -> NewChannel {
    NewChannel {
        name: "News".into(),
        about: String::new(),
        picture: String::new(),
        join,
        history: History::Open,
        relays: vec!["wss://r1.example".into(), "wss://r2.example".into()],
        storage: vec![H1.into(), H2.into()],
    }
}

async fn create(w: &World, owner: &Node, join: JoinMode) -> String {
    let (cid, _) = owner.svc.create(&owner.keys, new_channel(join)).await.unwrap_or_else(|e| panic!("create: {e}"));
    w.pump(&[owner]).await;
    cid
}

async fn join_by_link(w: &World, owner: &Node, reader: &Node, cid: &str, all: &[&Node]) {
    let link = owner.svc.share_link(cid).await.unwrap_or_else(|e| panic!("link: {e}"));
    w.pump(&[reader]).await;
    reader.svc.open_link(&reader.keys, &link, "").await.unwrap_or_else(|e| panic!("open link: {e}"));
    w.pump(all).await;
}

async fn newest(n: &Node, cid: &str) -> Option<EpochKey> {
    n.svc.member(&n.keys).newest_key(cid, SCOPE_EVERYONE).await.unwrap_or_else(|e| panic!("keys: {e}"))
}

async fn tick(n: &Node) {
    n.svc.tick().await.unwrap_or_else(|e| panic!("tick: {e}"));
}

async fn post_texts(n: &Node, cid: &str) -> Vec<String> {
    let mut t: Vec<String> = db::content::posts(n.svc.db(), cid, None, 100).await.unwrap_or_default().into_iter().filter_map(|p| p.text).collect();
    t.sort();
    t
}

async fn membership(n: &Node, cid: &str) -> String {
    db::channels::get(n.svc.db(), cid).await.ok().flatten().map(|r| r.membership).unwrap_or_default()
}

fn pk(n: &Node) -> PubKey {
    n.me.clone()
}

/// Whether this device runs the keeper of the channel now.
async fn keeps(n: &Node, cid: &str) -> bool {
    n.svc.keeper(cid).await.ok().flatten().is_some()
}

/// The signers of the epoch events on the relay, by epoch.
fn epoch_signers(w: &World) -> Vec<(u32, String)> {
    w.relay
        .of_kind(KIND_EPOCH)
        .iter()
        .filter_map(|e| {
            let n = e["tags"].as_array()?.iter().find(|t| t[0] == "n")?[1].as_str()?.parse().ok()?;
            Some((n, e["pubkey"].as_str()?.to_string()))
        })
        .collect()
}

/// The owner on device A, a reader by the link, and the owner's device B
/// that heard everything (and A's notes of its keys, with the root).
async fn owner_two_devices(w: &World) -> (Node, Node, Node, String) {
    let a = w.node().await;
    let cid = create(w, &a, JoinMode::Open).await;
    let r = w.node().await;
    join_by_link(w, &a, &r, &cid, &[&a, &r]).await;
    let b = w.device(a.keys.clone()).await;
    w.pump(&[&a, &b, &r]).await;
    assert_eq!(membership(&b, &cid).await, MEMBERSHIP_JOINED, "B reads the channel");
    (a, b, r, cid)
}

// ─── The keeper between the owner's devices (wire §15) ──────────────────────

#[tokio::test]
async fn the_root_reaches_the_owners_other_device_by_its_note_of_keys() {
    let w = World::new();
    let (a, b, _r, cid) = owner_two_devices(&w).await;
    let root_a = secrets::root(a.svc.env.secrets.as_ref(), &cid).await.ok().flatten().map(|r| *r.as_bytes());
    let root_b = secrets::root(b.svc.env.secrets.as_ref(), &cid).await.ok().flatten().map(|r| *r.as_bytes());
    assert!(root_a.is_some());
    assert_eq!(root_a, root_b, "the owner's other device holds the root");
}

#[tokio::test]
async fn the_keeper_moves_to_another_device_of_the_owner_and_the_readers_read_on() {
    let w = World::new();
    let (a, b, r, cid) = owner_two_devices(&w).await;
    let all = [&a, &b, &r];
    let before = newest(&r, &cid).await.map(|k| k.epoch).unwrap_or(0);
    let bundles_before = db::keys::bundles(r.svc.db(), &cid, SCOPE_EVERYONE).await.unwrap_or_default().len();
    b.svc.keeper_take(&b.keys, &cid, false).await.unwrap_or_else(|e| panic!("take: {e}"));
    assert!(b.svc.keeper_take_status(&cid).await.ok().flatten().is_some(), "B waits for the state");
    w.pump(&all).await;
    // A handed its state over and keeps nothing; B runs the keeper.
    assert!(keeps(&b, &cid).await, "B keeps the channel");
    assert!(!keeps(&a, &cid).await);
    assert!(db::keeper::keeper(a.svc.db(), &cid).await.ok().flatten().is_none(), "A's rows are gone");
    assert!(secrets::keeper_keys(a.svc.env.secrets.as_ref(), &cid).await.ok().flatten().is_none(), "A's keeper key is gone");
    assert!(b.svc.keeper_take_status(&cid).await.ok().flatten().is_none());
    // B published the next epoch under its own key, from the epoch its
    // operation named; the reader opened it with the bundle it held.
    let log = b.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("journal"));
    let series = log.state().keeper().cloned().unwrap_or_else(|| panic!("keeper"));
    assert_eq!(series.serial, 1);
    let after = newest(&r, &cid).await.map(|k| k.epoch).unwrap_or(0);
    assert!(after >= series.from_epoch && after > before, "{before} → {after}, from {}", series.from_epoch);
    assert!(epoch_signers(&w).iter().any(|(n, s)| *n == after && *s == series.pubkey.as_hex()));
    assert_eq!(db::keys::bundles(r.svc.db(), &cid, SCOPE_EVERYONE).await.unwrap_or_default().len(), bundles_before, "no new bundle was needed");
    // A newcomer is let in by B, and what the owner posts the readers read.
    let n = w.node().await;
    join_by_link(&w, &b, &n, &cid, &[&a, &b, &r, &n]).await;
    assert_eq!(membership(&n, &cid).await, MEMBERSHIP_JOINED);
    b.svc.send_post(&b.keys, &cid, "from B", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&[&a, &b, &r, &n]).await;
    assert_eq!(post_texts(&r, &cid).await, ["from B"]);
    assert_eq!(post_texts(&n, &cid).await, ["from B"]);
    // A publishes nothing any more, whatever its clock says.
    let signed_by_a = |w: &World| epoch_signers(w).iter().filter(|(e, _)| *e >= series.from_epoch).filter(|(_, s)| *s != series.pubkey.as_hex()).count();
    w.clock.advance(3600);
    tick(&a).await;
    assert_eq!(signed_by_a(&w), 0);
    assert!(a.errors().is_empty() && b.errors().is_empty() && r.errors().is_empty(), "{:?} {:?} {:?}", a.errors(), b.errors(), r.errors());
}

#[tokio::test]
async fn a_lost_keeper_device_is_replaced_by_an_emergency_generation() {
    let w = World::new();
    let (a, b, r, cid) = owner_two_devices(&w).await;
    // A snapshot that names the reader: what B rebuilds the readers from.
    w.clock.advance(600);
    tick(&a).await;
    w.pump(&[&a, &b, &r]).await;
    let old_root = secrets::root(b.svc.env.secrets.as_ref(), &cid).await.ok().flatten().map(|r| *r.as_bytes());
    // A is lost: from here on it hears nothing.
    b.svc.keeper_take(&b.keys, &cid, true).await.unwrap_or_else(|e| panic!("take: {e}"));
    w.pump(&[&b, &r]).await;
    assert!(keeps(&b, &cid).await);
    let new_root = secrets::root(b.svc.env.secrets.as_ref(), &cid).await.ok().flatten().map(|r| *r.as_bytes());
    assert_ne!(new_root, old_root, "a new root: the lost device held the old one");
    // The reader got a bundle of the emergency generation and reads on.
    let bundles = db::keys::bundles(r.svc.db(), &cid, SCOPE_EVERYONE).await.unwrap_or_default();
    assert!(bundles.iter().any(|(_, _, emergency)| *emergency), "{bundles:?}");
    let log = r.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("journal"));
    let from = log.state().keeper().map(|k| k.from_epoch).unwrap_or(0);
    assert_eq!(newest(&r, &cid).await.map(|k| k.epoch), Some(from));
    b.svc.send_post(&b.keys, &cid, "after the loss", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&[&b, &r]).await;
    assert_eq!(post_texts(&r, &cid).await, ["after the loss"]);
    // A comes back: it sees the later series, hands over into nothing and
    // keeps no keeper; B stays the keeper.
    w.pump(&[&a, &b, &r]).await;
    assert!(!keeps(&a, &cid).await && db::keeper::keeper(a.svc.db(), &cid).await.ok().flatten().is_none());
    assert!(keeps(&b, &cid).await);
    assert!(r.errors().is_empty(), "{:?}", r.errors());
}

#[tokio::test]
async fn an_emergency_that_failed_half_way_is_resumed_when_asked_again() {
    let w = World::new();
    let (a, b, r, cid) = owner_two_devices(&w).await;
    w.clock.advance(600);
    tick(&a).await;
    w.pump(&[&a, &b, &r]).await;
    // The hosts are down: the shards of the snapshot cannot be had, and the
    // emergency is refused before it writes anything.
    w.hosts.set_down(H1, true);
    w.hosts.set_down(H2, true);
    let err = b.svc.keeper_take(&b.keys, &cid, true).await.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains(crate::handover::NO_READERS), "{err}");
    assert!(b.svc.keeper_take_status(&cid).await.ok().flatten().is_none(), "no take row");
    let serial = |n: &Node| {
        let (svc, cid) = (n.svc.clone(), cid.clone());
        async move { svc.log(&cid).await.ok().flatten().and_then(|l| l.state().keeper().map(|k| k.serial)) }
    };
    assert_eq!(serial(&b).await, Some(0), "no series announced");
    w.hosts.set_down(H1, false);
    w.hosts.set_down(H2, false);
    w.pump(&[&b, &r]).await;
    // An emergency that failed after its `keeper` operation went out (a
    // failure past the readers): the row says emergency, no keeper here.
    // Asked again, the same take goes on to its end.
    b.svc.keeper_take(&b.keys, &cid, false).await.unwrap_or_else(|e| panic!("take: {e}"));
    let mut take = b.svc.keeper_take_status(&cid).await.ok().flatten().unwrap_or_else(|| panic!("the take row"));
    take.mode = db::handover::MODE_EMERGENCY.into();
    db::handover::put_take(b.svc.db(), &take).await.unwrap_or_else(|e| panic!("{e}"));
    assert!(!keeps(&b, &cid).await);
    b.svc.keeper_take(&b.keys, &cid, true).await.unwrap_or_else(|e| panic!("take again: {e}"));
    w.pump(&[&b, &r]).await;
    assert!(keeps(&b, &cid).await, "the emergency was resumed");
    assert!(b.svc.keeper_take_status(&cid).await.ok().flatten().is_none());
    let bundles = db::keys::bundles(r.svc.db(), &cid, SCOPE_EVERYONE).await.unwrap_or_default();
    assert!(bundles.iter().any(|(_, _, emergency)| *emergency), "the reader got the emergency bundle: {bundles:?}");
    b.svc.send_post(&b.keys, &cid, "resumed", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&[&b, &r]).await;
    assert_eq!(post_texts(&r, &cid).await, ["resumed"]);
}

/// The owner, a reader who stays and two who are put out (each removal an
/// epoch of its own), and a snapshot that names the readers.
async fn owner_and_readers(w: &World) -> (Node, Node, Node, Node, String) {
    let a = w.node().await;
    let cid = create(w, &a, JoinMode::Open).await;
    let (r, g1, g2) = (w.node().await, w.node().await, w.node().await);
    join_by_link(w, &a, &r, &cid, &[&a, &r]).await;
    join_by_link(w, &a, &g1, &cid, &[&a, &r, &g1]).await;
    join_by_link(w, &a, &g2, &cid, &[&a, &r, &g1, &g2]).await;
    w.clock.advance(600);
    tick(&a).await;
    w.pump(&[&a, &r, &g1, &g2]).await;
    (a, r, g1, g2, cid)
}

async fn put_out(w: &World, a: &Node, who: &Node, cid: &str, all: &[&Node]) {
    w.clock.advance(10);
    a.svc.act(&a.keys, cid, OpBody::Remove { who: pk(who) }).await.unwrap_or_else(|e| panic!("remove: {e}"));
    w.pump(all).await;
    // The epoch that leaves them out, now rather than at the keeper's hour.
    a.svc.rotate_now(cid).await.unwrap_or_else(|e| panic!("rotate: {e}"));
    w.clock.advance(10);
    tick(a).await;
    w.pump(all).await;
}

#[tokio::test]
async fn a_takeover_by_a_device_that_heard_no_epoch_is_refused_and_begins_past_them_once_heard() {
    let w = World::new();
    let (a, r, g1, _g2, cid) = owner_and_readers(&w).await;
    put_out(&w, &a, &g1, &cid, &[&a, &r, &g1]).await;
    let newest_a = db::keys::newest_epoch(r.svc.db(), &cid).await.unwrap_or(0);
    assert!(newest_a >= 2, "the keeper rotated past the removal: {newest_a}");
    // A is lost. The owner's fresh device has the journal and the keys, no epoch event.
    let c = w.device(a.keys.clone()).await;
    c.hold(KIND_EPOCH, true);
    w.pump(&[&c]).await;
    assert!(c.svc.log(&cid).await.ok().flatten().is_some(), "the journal is here");
    let err = c.svc.keeper_take(&c.keys, &cid, true).await.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("epochs of the channel are not here"), "{err}");
    assert!(c.svc.keeper_take_status(&cid).await.ok().flatten().is_none(), "nothing of the take was written");
    // Heard: the series begins past the newest epoch, and the reader takes it.
    c.hold(KIND_EPOCH, false);
    w.pump(&[&c]).await;
    // No snapshot after the removal: the one put out is still in the newest.
    let snap_after = c.svc.member(&c.keys).latest_snapshot(&cid).await.ok().flatten().map(|s| s.at).unwrap_or(0);
    let removed_at = c.svc.log(&cid).await.ok().flatten().and_then(|l| l.ordered().filter(|o| matches!(o.body, OpBody::Remove { .. })).map(|o| o.created_at).max()).unwrap_or(0);
    assert!(snap_after < removed_at, "the newest snapshot predates the removal ({snap_after} < {removed_at})");
    c.svc.keeper_take(&c.keys, &cid, true).await.unwrap_or_else(|e| panic!("take: {e}"));
    w.pump(&[&c, &r, &g1]).await;
    let from = c.svc.log(&cid).await.ok().flatten().and_then(|l| l.state().keeper().map(|k| k.from_epoch)).unwrap_or(0);
    assert!(from > newest_a, "the series begins past {newest_a}: {from}");
    assert_eq!(db::keys::newest_epoch(r.svc.db(), &cid).await.unwrap_or(0), from, "the reader took the first epoch of the series");
    c.svc.send_post(&c.keys, &cid, "from the new keeper", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&[&c, &r]).await;
    assert_eq!(post_texts(&r, &cid).await, ["from the new keeper"]);
    w.pump(&[&g1]).await;
    let emergency_bundles = db::keys::bundles(g1.svc.db(), &cid, SCOPE_EVERYONE).await.unwrap_or_default().into_iter().filter(|(_, _, e)| *e).count();
    assert_eq!(emergency_bundles, 0, "the one put out after the snapshot gets no bundle of the new generation");
    assert!(!post_texts(&g1, &cid).await.contains(&"from the new keeper".to_string()));
    assert!(r.errors().is_empty(), "{:?}", r.errors());
}

#[tokio::test]
async fn a_partly_behind_device_takes_the_keeper_past_the_epochs_on_the_relays() {
    let w = World::new();
    let (a, r, g1, g2, cid) = owner_and_readers(&w).await;
    // A device of the owner that heard epoch 1, then no epoch live.
    let c = w.device(a.keys.clone()).await;
    w.pump(&[&c]).await;
    c.hold(KIND_EPOCH, true);
    put_out(&w, &a, &g1, &cid, &[&a, &r, &g1, &c]).await;
    put_out(&w, &a, &g2, &cid, &[&a, &r, &g2, &c]).await;
    let newest_a = db::keys::newest_epoch(r.svc.db(), &cid).await.unwrap_or(0);
    assert!(newest_a >= 3, "{newest_a}");
    assert!(c.svc.newest_epoch_known(&cid).await.unwrap_or(0) < newest_a, "behind: the epochs since were not heard");
    // A is lost. The take brings the channel to its head from the relays
    // first (the epochs come by a catch-up, not live), then numbers past it.
    let pump = async {
        for _ in 0..60 {
            c.sync().await;
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    };
    let (taken, ()) = tokio::join!(c.svc.keeper_take_at_head(&c.keys, &cid, true, false, std::time::Duration::from_secs(20)), pump);
    taken.unwrap_or_else(|e| panic!("take: {e}"));
    let from = c.svc.log(&cid).await.ok().flatten().and_then(|l| l.state().keeper().map(|k| k.from_epoch)).unwrap_or(0);
    assert!(from > newest_a, "the series begins past {newest_a}: {from}");
    w.pump(&[&c, &r]).await;
    c.svc.send_post(&c.keys, &cid, "past the head", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&[&c, &r]).await;
    assert_eq!(post_texts(&r, &cid).await, ["past the head"]);
}

#[tokio::test]
async fn a_removal_the_journal_rejected_leaves_the_reader_in_the_emergency_generation() {
    let w = World::new();
    let (a, b, x, cid) = owner_two_devices(&w).await;
    let m = w.node().await;
    join_by_link(&w, &a, &m, &cid, &[&a, &b, &x, &m]).await;
    a.svc.act(&a.keys, &cid, OpBody::SetRole { who: pk(&m), role: Role::Moderator }).await.unwrap_or_else(|e| panic!("role: {e}"));
    w.pump(&[&a, &b, &x, &m]).await;
    w.clock.advance(600);
    tick(&a).await;
    w.pump(&[&a, &b, &x, &m]).await;
    // A demotes M, who does not hear it and removes X: kept, rejected.
    m.hold(crate::wire::KIND_JOURNAL, true);
    w.clock.advance(10);
    a.svc.act(&a.keys, &cid, OpBody::SetRole { who: pk(&m), role: Role::Member }).await.unwrap_or_else(|e| panic!("demote: {e}"));
    w.pump(&[&a, &b, &x]).await;
    w.clock.advance(10);
    m.svc.act(&m.keys, &cid, OpBody::Remove { who: pk(&x) }).await.unwrap_or_else(|e| panic!("remove: {e}"));
    w.pump(&[&a, &b, &x, &m]).await;
    let log = b.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("journal"));
    let removal = log.ordered().find(|o| matches!(&o.body, OpBody::Remove { who } if *who == pk(&x))).map(|o| o.id()).unwrap_or_else(|| panic!("the removal is kept"));
    assert!(log.rejection(&removal).is_some(), "the journal rejected the demoted moderator's removal");
    // A is lost; B takes it over in an emergency: X is a reader still.
    b.svc.keeper_take(&b.keys, &cid, true).await.unwrap_or_else(|e| panic!("take: {e}"));
    w.pump(&[&b, &x]).await;
    let bundles = db::keys::bundles(x.svc.db(), &cid, SCOPE_EVERYONE).await.unwrap_or_default();
    assert!(bundles.iter().any(|(_, _, emergency)| *emergency), "X got the emergency bundle: {bundles:?}");
    b.svc.send_post(&b.keys, &cid, "X reads on", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&[&b, &x]).await;
    assert!(post_texts(&x, &cid).await.contains(&"X reads on".to_string()), "{:?}", post_texts(&x, &cid).await);
}

#[tokio::test]
async fn an_epoch_of_a_stolen_keeper_key_far_ahead_does_not_move_where_a_series_begins() {
    let w = World::new();
    let (a, b, r, cid) = owner_two_devices(&w).await;
    // A snapshot that names the reader, as an emergency needs.
    w.clock.advance(600);
    tick(&a).await;
    w.pump(&[&a, &b, &r]).await;
    let kkeys = a.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("keeper")).lock().await.keys().clone();
    let real = &w.relay.of_kind(KIND_EPOCH)[0];
    for (epoch, prev) in [(u32::MAX - 1, Some(u32::MAX - 2)), (u32::MAX, None), (4_000_000_000, Some(3_999_999_999))] {
        let mut content: crate::epoch::EpochContent = serde_json::from_str(real["content"].as_str().unwrap_or("")).unwrap_or_else(|e| panic!("{e}"));
        content.event.epoch = epoch;
        content.event.prev_epoch = prev;
        let ev = crate::epoch::sign_epoch(&kkeys, &cid, SCOPE_EVERYONE, &content).unwrap_or_else(|e| panic!("{e}"));
        w.relay.events.lock().unwrap_or_else(|p| p.into_inner()).push(serde_json::to_value(&ev).unwrap_or_default());
    }
    w.pump(&[&a, &b, &r]).await;
    let known = b.svc.newest_epoch_known(&cid).await.unwrap_or(0);
    assert!(known < 100, "the forged epochs counted: {known}");
    b.svc.keeper_take(&b.keys, &cid, true).await.unwrap_or_else(|e| panic!("take: {e}"));
    let from = b.svc.log(&cid).await.ok().flatten().and_then(|l| l.state().keeper().map(|k| k.from_epoch)).unwrap_or(0);
    assert!(from < 100, "{from}");
    w.clock.advance(400);
    tick(&b).await;
    w.pump(&[&b, &r]).await;
    b.svc.send_post(&b.keys, &cid, "numbers sane", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&[&b, &r]).await;
    assert_eq!(post_texts(&r, &cid).await, ["numbers sane"]);
}

#[tokio::test]
async fn a_take_row_of_a_series_the_journal_went_past_is_dropped_not_resumed() {
    let w = World::new();
    let (a, b, r, cid) = owner_two_devices(&w).await;
    // A hears nothing of the journal any more (it never hands over). B
    // announces a take; then a third device of the owner takes the keeper
    // (a later series), and B is asked for an emergency.
    a.hold(crate::wire::KIND_JOURNAL, true);
    b.svc.keeper_take(&b.keys, &cid, false).await.unwrap_or_else(|e| panic!("take: {e}"));
    w.pump(&[&a, &b, &r]).await;
    let c = w.device(a.keys.clone()).await;
    w.pump(&[&c]).await;
    // B's series never published an epoch: the owner says so (`force`).
    c.svc.keeper_take_with(&c.keys, &cid, false, true).await.unwrap_or_else(|e| panic!("take: {e}"));
    w.pump(&[&a, &c, &r]).await;
    w.pump(&[&b]).await;
    let mut take = b.svc.keeper_take_status(&cid).await.ok().flatten().unwrap_or_else(|| panic!("B still waits"));
    take.mode = db::handover::MODE_EMERGENCY.into();
    db::handover::put_take(b.svc.db(), &take).await.unwrap_or_else(|e| panic!("{e}"));
    // B's row names a series the journal went past: dropped, and B takes a new one.
    b.svc.keeper_take_with(&b.keys, &cid, true, true).await.unwrap_or_else(|e| panic!("take again: {e}"));
    let series = b.svc.log(&cid).await.ok().flatten().and_then(|l| l.state().keeper().map(|k| k.serial)).unwrap_or(0);
    assert!(series as i64 > take.serial, "a series of its own past the stale one: {series} > {}", take.serial);
    assert!(keeps(&b, &cid).await);
}

#[tokio::test]
async fn a_take_is_the_owners_alone_and_not_on_the_keepers_device() {
    let w = World::new();
    let (a, _b, r, cid) = owner_two_devices(&w).await;
    let err = r.svc.keeper_take(&r.keys, &cid, false).await.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("not permitted"), "{err}");
    let err = a.svc.keeper_take(&a.keys, &cid, false).await.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("this device keeps the channel"), "{err}");
    let _ = pk(&r);
}

/// A section whose epochs lag behind those of everyone's jumps to the first
/// epoch of the new series (wire §15: one number for every scope), and its
/// readers take it with the bundle they hold.
#[tokio::test]
async fn a_section_follows_the_keeper_to_the_new_series() {
    let w = World::new();
    let (a, b, r, cid) = owner_two_devices(&w).await;
    let all = [&a, &b, &r];
    let (gold, _) = a.svc.scope_add(&a.keys, &cid, "Gold", keydist::SchemeId::SdV1, History::Open).await.unwrap_or_else(|e| panic!("scope: {e}"));
    w.pump(&all).await;
    let (lvl, _) = a.svc.level_set(&a.keys, &cid, None, "Gold", vec![gold.clone()], crate::op::LevelCan::BASE).await.unwrap_or_else(|e| panic!("level: {e}"));
    w.pump(&all).await;
    a.svc.set_levels(&a.keys, &cid, &pk(&r), &[lvl]).await.unwrap_or_else(|e| panic!("set_levels: {e}"));
    w.pump(&all).await;
    tick(&a).await;
    w.pump(&all).await;
    // Everyone's epochs run ahead of the section's.
    for _ in 0..3 {
        a.svc.rotate_now(&cid).await.unwrap_or_else(|e| panic!("{e}"));
        w.clock.advance(60);
        tick(&a).await;
        w.pump(&all).await;
    }
    let section = |n: &Node| {
        let (svc, keys, cid, gold) = (n.svc.clone(), n.keys.clone(), cid.clone(), gold.clone());
        async move { svc.member(&keys).newest_key(&cid, &gold).await.ok().flatten().map(|k| k.epoch) }
    };
    let lagging = section(&r).await.unwrap_or(0);
    assert!(lagging < newest(&r, &cid).await.map(|k| k.epoch).unwrap_or(0), "the section lags");
    b.svc.keeper_take(&b.keys, &cid, false).await.unwrap_or_else(|e| panic!("take: {e}"));
    w.pump(&all).await;
    let from = b.svc.log(&cid).await.ok().flatten().and_then(|l| l.state().keeper().map(|k| k.from_epoch)).unwrap_or(0);
    let now_at = section(&r).await.unwrap_or(0);
    assert!(now_at >= from, "the section jumped to the series: {lagging} → {now_at}, from {from}");
    assert!(r.errors().is_empty(), "{:?}", r.errors());
}

// ─── A new owner (wire §16) ─────────────────────────────────────────────────

use crate::op::OpBody;
use crate::roles::Role;

/// The owner, a moderator who will take the channel, a reader.
async fn channel_with_team(w: &World) -> (Node, Node, Node, String) {
    let o = w.node().await;
    let cid = create(w, &o, JoinMode::Open).await;
    let m = w.node().await;
    let r = w.node().await;
    join_by_link(w, &o, &m, &cid, &[&o, &m]).await;
    join_by_link(w, &o, &r, &cid, &[&o, &m, &r]).await;
    o.svc.act(&o.keys, &cid, OpBody::SetRole { who: pk(&m), role: Role::Moderator }).await.unwrap_or_else(|e| panic!("role: {e}"));
    w.pump(&[&o, &m, &r]).await;
    (o, m, r, cid)
}

/// A card of the channel signed by `by`, as the relay would hand it to `to`.
async fn card_heard(by: &Node, to: &Node, cid: &str) -> Vec<String> {
    let state = by.svc.log(cid).await.ok().flatten().map(|l| l.state().clone()).unwrap_or_else(|| panic!("journal"));
    let card = by.svc.card_of(&state, by.svc.now(), 0);
    let signed = serde_json::to_value(crate::wire::sign_card(&by.keys, &card).unwrap_or_else(|e| panic!("{e}"))).unwrap_or_default();
    to.svc.on_event(&to.keys, &signed, true).await.map(|o| o.notes).unwrap_or_default()
}

/// The count of readers a member shows is the keeper's snapshot's; the
/// owner's card heard again by a catch-up (an old one says 0) takes
/// nothing away from it (живой запуск 2026-10-24: «читателей: 0» after a
/// restart of the app).
#[tokio::test]
async fn a_card_heard_again_does_not_zero_the_readers_a_member_counts() {
    let w = World::new();
    let (o, m, r, cid) = channel_with_team(&w).await;
    let all = [&o, &m, &r];
    w.clock.advance(600);
    tick(&o).await;
    w.pump(&all).await;
    let counted = db::channels::get(r.svc.db(), &cid).await.ok().flatten().map_or(0, |row| row.members);
    assert!(counted >= 2, "the snapshot counted the readers: {counted}");
    assert!(!card_heard(&o, &r, &cid).await.iter().any(|n| n.contains("card refused")));
    assert_eq!(db::channels::get(r.svc.db(), &cid).await.ok().flatten().map_or(0, |row| row.members), counted, "the card takes nothing away");
    // A row an old card wrote 0 into (the apps before this change) is put
    // right by the next card heard, and by the next refresh of the row.
    let mut row = db::channels::get(r.svc.db(), &cid).await.ok().flatten().unwrap_or_else(|| panic!("row"));
    row.members = 0;
    db::channels::upsert(r.svc.db(), &row).await.unwrap_or_else(|e| panic!("{e}"));
    assert!(!card_heard(&o, &r, &cid).await.iter().any(|n| n.contains("card refused")));
    assert_eq!(db::channels::get(r.svc.db(), &cid).await.ok().flatten().map_or(0, |row| row.members), counted, "put right by the card");
    row.members = 0;
    db::channels::upsert(r.svc.db(), &row).await.unwrap_or_else(|e| panic!("{e}"));
    o.svc.act(&o.keys, &cid, OpBody::SetRole { who: pk(&r), role: Role::Author }).await.unwrap_or_else(|e| panic!("role: {e}"));
    w.pump(&all).await;
    assert_eq!(db::channels::get(r.svc.db(), &cid).await.ok().flatten().map_or(0, |row| row.members), counted, "put right by the refresh of the row");
}

#[tokio::test]
async fn a_channel_handed_over_is_the_new_owners_from_his_keeper_on() {
    let w = World::new();
    let (o, m, r, cid) = channel_with_team(&w).await;
    let all = [&o, &m, &r];
    o.svc.transfer(&o.keys, &cid, &pk(&m)).await.unwrap_or_else(|e| panic!("transfer: {e}"));
    w.pump(&all).await;
    // Offered, and nothing changed yet: the owner is the owner.
    let offer = m.svc.transfer_of(&cid).await.ok().flatten().unwrap_or_else(|| panic!("the offer came"));
    assert_eq!((offer.direction.as_str(), offer.peer.as_str(), offer.status.as_str()), ("in", o.me.as_hex(), "offered"));
    assert!(m.svc.act(&m.keys, &cid, OpBody::Disband).await.is_err(), "not the owner yet");
    assert!(!card_heard(&o, &r, &cid).await.iter().any(|n| n.contains("card refused")), "the owner's card stands");
    let gens_before = db::keys::newest_gen(r.svc.db(), &cid, SCOPE_EVERYONE).await.unwrap_or(0);
    m.svc.transfer_answer(&m.keys, &cid, true).await.unwrap_or_else(|e| panic!("accept: {e}"));
    w.pump(&all).await;
    for n in all {
        let s = n.svc.log(&cid).await.ok().flatten().map(|l| l.state().clone()).unwrap_or_else(|| panic!("journal"));
        assert_eq!((s.owner.clone(), s.role_of(&pk(&o))), (pk(&m), Some(Role::Admin)), "the new owner on every device");
    }
    // The keeper went to the new owner; the old owner keeps neither it nor the root.
    assert!(keeps(&m, &cid).await && !keeps(&o, &cid).await);
    assert!(db::keeper::keeper(o.svc.db(), &cid).await.ok().flatten().is_none());
    assert!(secrets::root(o.svc.env.secrets.as_ref(), &cid).await.ok().flatten().is_none(), "the old root is gone from the old owner");
    // A generation of the new root, both alive for the grace; the reader got a bundle of it.
    let k = m.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("kept"));
    assert_eq!(k.lock().await.scope_keeper(SCOPE_EVERYONE).map(|s| s.generations()), Some(2));
    assert!(db::keys::newest_gen(r.svc.db(), &cid, SCOPE_EVERYONE).await.unwrap_or(0) > gens_before, "a bundle of the new generation");
    m.svc.send_post(&m.keys, &cid, "the new owner", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    assert_eq!(post_texts(&r, &cid).await, ["the new owner"]);
    // The card: the new owner's stands, the old owner's is refused.
    assert!(!card_heard(&m, &r, &cid).await.iter().any(|n| n.contains("card refused")));
    assert!(card_heard(&o, &r, &cid).await.iter().any(|n| n.contains("card refused")), "the former owner's card");
    assert!(m.svc.transfer_of(&cid).await.ok().flatten().is_none() && o.svc.transfer_of(&cid).await.ok().flatten().is_none(), "nothing left to show");
    assert!(r.errors().is_empty() && m.errors().is_empty(), "{:?} {:?}", r.errors(), m.errors());
}

#[tokio::test]
async fn a_declined_handover_changes_nothing() {
    let w = World::new();
    let (o, m, r, cid) = channel_with_team(&w).await;
    let all = [&o, &m, &r];
    o.svc.transfer(&o.keys, &cid, &pk(&m)).await.unwrap_or_else(|e| panic!("transfer: {e}"));
    w.pump(&all).await;
    m.svc.transfer_answer(&m.keys, &cid, false).await.unwrap_or_else(|e| panic!("decline: {e}"));
    w.pump(&all).await;
    let mine = o.svc.transfer_of(&cid).await.ok().flatten().unwrap_or_else(|| panic!("my offer"));
    assert_eq!((mine.direction.as_str(), mine.status.as_str()), ("out", "declined"));
    assert!(m.svc.transfer_of(&cid).await.ok().flatten().is_none(), "answered");
    assert!(m.svc.transfer_answer(&m.keys, &cid, true).await.is_err(), "no offer to accept any more");
    let s = r.svc.log(&cid).await.ok().flatten().map(|l| l.state().clone()).unwrap_or_else(|| panic!("journal"));
    assert_eq!(s.owner, pk(&o));
    assert!(keeps(&o, &cid).await && !keeps(&m, &cid).await);
    // Only the owner offers a channel, and not to himself.
    assert!(m.svc.transfer(&m.keys, &cid, &pk(&r)).await.is_err());
    assert!(o.svc.transfer(&o.keys, &cid, &pk(&o)).await.is_err());
}

/// A channel handed over: my note of the keys names the owner who made it
/// (the author of its `create`), not the owner now, and a device of mine
/// that never heard the journal joins it; a `create` of somebody else
/// under its id (here the owner now) is not its genesis there (ROO-271,
/// ROO-323).
#[tokio::test]
async fn a_channel_handed_over_shows_on_a_device_without_its_journal_by_the_author_of_its_create() {
    let w = World::new();
    let (o, m, r, cid) = channel_with_team(&w).await;
    let all = [&o, &m, &r];
    o.svc.transfer(&o.keys, &cid, &pk(&m)).await.unwrap_or_else(|e| panic!("transfer: {e}"));
    w.pump(&all).await;
    m.svc.transfer_answer(&m.keys, &cid, true).await.unwrap_or_else(|e| panic!("accept: {e}"));
    w.pump(&all).await;
    let log = r.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("journal"));
    assert_eq!((log.state().owner.clone(), log.creator().cloned()), (pk(&m), Some(pk(&o))));
    let body = r.svc.member(&r.keys).own_keys_for_test(log.state()).await;
    assert_eq!(body.owner.as_deref(), Some(o.me.as_hex()), "who made it, not the owner now");
    // A create of the owner now under the id, before everything on the
    // relay; a device of the reader that heard nothing of it all.
    let real = log.ordered().next().cloned().unwrap_or_else(|| panic!("create"));
    w.relay.events.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(0, crate::scenarios::forged_create(&cid, &real, &m.keys));
    let tablet = w.device(r.keys.clone()).await;
    tablet.skip_all(&w.relay);
    tablet.skip_inboxes(&w.relay);
    w.clock.advance(crate::member::OWN_KEYS_EVERY_SECS + 1);
    r.svc.turn().await.unwrap_or_else(|e| panic!("turn: {e}"));
    let all = [&o, &m, &r, &tablet];
    w.pump(&all).await;
    assert_eq!(tablet.svc.log(&cid).await.ok().flatten().and_then(|l| l.creator().cloned()), Some(pk(&o)), "the real create, not the other");
    assert_eq!(membership(&tablet, &cid).await, MEMBERSHIP_JOINED, "joined there by itself");
    assert_eq!(newest(&tablet, &cid).await, newest(&r, &cid).await);
    tablet.svc.listen(&tablet.keys).await.unwrap_or_else(|e| panic!("listen: {e}"));
    w.pump(&all).await;
    m.svc.send_post(&m.keys, &cid, "the new owner", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    assert_eq!(post_texts(&tablet, &cid).await, ["the new owner"]);
}

// ─── The tickets of the moderators (wire §17) ───────────────────────────────

/// A `request` channel with a moderator (not the owner) and a post by the owner.
async fn request_channel(w: &World) -> (Node, Node, String) {
    let o = w.node().await;
    let m = w.node().await;
    let cid = create(w, &o, JoinMode::Request).await;
    o.svc.act(&o.keys, &cid, OpBody::SetRole { who: pk(&m), role: Role::Moderator }).await.unwrap_or_else(|e| panic!("role: {e}"));
    o.svc.send_post(&o.keys, &cid, "welcome", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&[&o, &m]).await;
    (o, m, cid)
}

async fn open_tickets(o: &Node, cid: &str) -> Vec<db::tickets::MintedRow> {
    db::tickets::open(o.svc.db(), cid, None).await.unwrap_or_default()
}

#[tokio::test]
async fn a_moderator_lets_a_newcomer_read_by_a_ticket_while_the_keeper_is_away() {
    let w = World::new();
    let (o, m, cid) = request_channel(&w).await;
    m.svc.tickets_request(&m.keys, &cid, 5).await.unwrap_or_else(|e| panic!("tickets: {e}"));
    w.pump(&[&o, &m]).await;
    assert_eq!(m.svc.tickets_held(&cid).await.map(|t| t.held).unwrap_or(0), 5, "the moderator holds five");
    assert_eq!(open_tickets(&o, &cid).await.len(), 5);
    // The tickets are nobody: no snapshot names them.
    w.clock.advance(600);
    tick(&o).await;
    w.pump(&[&o, &m]).await;
    let snap = m.svc.member(&m.keys).latest_snapshot(&cid).await.ok().flatten().map(|s| s.members).unwrap_or(0);
    assert_eq!(snap, 1, "the owner alone");
    // The keeper goes away; a newcomer asks, the moderator says yes.
    w.clock.advance(600);
    let n = w.node().await;
    let link = o.svc.share_link(&cid).await.unwrap_or_default();
    w.pump(&[&m, &n]).await;
    n.svc.open_link(&n.keys, &link, "hello").await.unwrap_or_else(|e| panic!("request: {e}"));
    w.pump(&[&m, &n]).await;
    m.svc.approve_request(&m.keys, &cid, &pk(&n)).await.unwrap_or_else(|e| panic!("approve: {e}"));
    w.pump(&[&m, &n]).await;
    assert_eq!(membership(&n, &cid).await, MEMBERSHIP_JOINED, "in at once");
    assert!(newest(&n, &cid).await.is_some());
    assert_eq!(post_texts(&n, &cid).await, ["welcome"], "reads without the keeper");
    assert_eq!(m.svc.tickets_held(&cid).await.map(|t| t.held).unwrap_or(0), 4);
    // The keeper is back: the ticket used, the newcomer on a leaf of his own.
    w.pump(&[&o, &m, &n]).await;
    let used: Option<String> = sqlx::query_scalar("SELECT status FROM cm_keeper_tickets WHERE community_id = ? AND used_by = ?")
        .bind(&cid)
        .bind(n.me.as_hex())
        .fetch_optional(o.svc.db().pool())
        .await
        .unwrap_or_default();
    assert_eq!(used.as_deref(), Some(db::tickets::USED), "the keeper counts the ticket used by the newcomer");
    assert_eq!(open_tickets(&o, &cid).await.len(), 4, "one used");
    let k = o.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("kept"));
    assert!(k.lock().await.is_member(&pk(&n)), "a reader of his own");
    // The ticket's leaf goes at the next epoch; the newcomer reads on.
    w.clock.advance(3 * 24 * 3600);
    tick(&o).await;
    w.pump(&[&o, &m, &n]).await;
    let last = *w.relay.epochs().last().unwrap_or(&0);
    assert_eq!(newest(&n, &cid).await.map(|k| k.epoch), Some(last));
    // A day on, the tickets nobody used are put out too.
    assert!(open_tickets(&o, &cid).await.is_empty(), "expired");
    assert!(n.errors().is_empty() && m.errors().is_empty(), "{:?} {:?}", n.errors(), m.errors());
}

#[tokio::test]
async fn tickets_stop_at_fifty_and_an_open_channel_has_none() {
    let w = World::new();
    let (o, m, cid) = request_channel(&w).await;
    for n in [50, 10] {
        m.svc.tickets_request(&m.keys, &cid, n).await.unwrap_or_else(|e| panic!("tickets: {e}"));
        w.pump(&[&o, &m]).await;
    }
    assert_eq!(open_tickets(&o, &cid).await.len(), 50, "fifty open at once");
    assert_eq!(m.svc.tickets_held(&cid).await.map(|t| t.held).unwrap_or(0), 50);
    // A moderator no longer: his tickets are put out.
    o.svc.act(&o.keys, &cid, OpBody::SetRole { who: pk(&m), role: Role::Member }).await.unwrap_or_else(|e| panic!("role: {e}"));
    w.pump(&[&o, &m]).await;
    tick(&o).await;
    assert!(open_tickets(&o, &cid).await.is_empty(), "revoked with the role");
    // A reader asks nothing; an open channel has no tickets.
    assert!(m.svc.tickets_request(&m.keys, &cid, 1).await.is_err());
    let o2 = w.node().await;
    let open = create(&w, &o2, JoinMode::Open).await;
    let err = o2.svc.tickets_request(&o2.keys, &open, 1).await.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("open channel"), "{err}");
}

#[tokio::test]
async fn a_join_written_before_its_writer_was_put_out_is_dropped_without_a_word() {
    let w = World::new();
    let (a, _b, r, cid) = owner_two_devices(&w).await;
    let written = w.clock.now();
    w.clock.advance(600);
    a.svc.act(&a.keys, &cid, OpBody::Remove { who: pk(&r) }).await.unwrap_or_else(|e| panic!("remove: {e}"));
    w.pump(&[&a, &r]).await;
    let k = a.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("keeper"));
    let log = a.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("log"));
    let replay = crate::wire::Join { community_id: cid.clone(), proof: None, admit_id: Some("6".repeat(32)), created_at: written };
    let got = k.lock().await.on_join(&log, &pk(&r), &replay, w.clock.now()).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(got, crate::keeper::Joined::Refused("put out after this join"));
    w.pump(&[&a, &r]).await;
    assert_ne!(membership(&r, &cid).await, crate::service::MEMBERSHIP_REJECTED, "nothing said to the one put out");
    // A join written after the removal (a new invitation) is no replay.
    w.clock.advance(1);
    let fresh = crate::wire::Join { community_id: cid.clone(), proof: None, admit_id: Some("7".repeat(32)), created_at: w.clock.now() };
    let got = k.lock().await.on_join(&log, &pk(&r), &fresh, w.clock.now()).await.unwrap_or_else(|e| panic!("{e}"));
    assert_ne!(got, crate::keeper::Joined::Refused("put out after this join"));
}

#[tokio::test]
async fn a_take_row_left_after_the_keeper_was_written_is_dropped_not_kept_for_ever() {
    let w = World::new();
    let (a, b, r, cid) = owner_two_devices(&w).await;
    w.clock.advance(600);
    tick(&a).await;
    w.pump(&[&a, &b, &r]).await;
    b.svc.keeper_take(&b.keys, &cid, true).await.unwrap_or_else(|e| panic!("take: {e}"));
    assert!(keeps(&b, &cid).await);
    // A run that failed after the keeper was written left its row.
    let series = b.svc.log(&cid).await.ok().flatten().and_then(|l| l.state().keeper().cloned()).unwrap_or_else(|| panic!("keeper"));
    let row = db::handover::TakeRow {
        community_id: cid.clone(),
        keeper_pubkey: series.pubkey.as_hex().to_string(),
        serial: i64::from(series.serial),
        from_epoch: i64::from(series.from_epoch),
        mode: db::handover::MODE_EMERGENCY.into(),
        started_at: w.clock.now(),
    };
    db::handover::put_take(b.svc.db(), &row).await.unwrap_or_else(|e| panic!("{e}"));
    b.svc.keeper_take(&b.keys, &cid, true).await.unwrap_or_else(|e| panic!("asked again: {e}"));
    assert!(b.svc.keeper_take_status(&cid).await.ok().flatten().is_none(), "the row went");
    assert!(keeps(&b, &cid).await);
    let err = b.svc.keeper_take(&b.keys, &cid, true).await.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("this device keeps the channel"), "{err}");
}

#[tokio::test]
async fn an_epoch_heard_before_its_journal_moves_no_cursor_and_is_asked_again() {
    let w = World::new();
    let (a, _b, _r, cid) = owner_two_devices(&w).await;
    // A fresh device of the owner hears an epoch before the journal (a page
    // of epochs before the page of the journal): dropped, and the next round
    // must ask for it again.
    let c = w.device(a.keys.clone()).await;
    let ev = w.relay.of_kind(KIND_EPOCH)[0].clone();
    c.svc.on_event(&c.keys, &ev, false).await.unwrap_or_else(|e| panic!("{e}"));
    let at = db::channels::heard_at(c.svc.db(), &cid, crate::transport::PART_EPOCHS).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(at, None, "no cursor moved by an epoch that could not be taken");
}

#[tokio::test]
async fn one_in_the_channel_is_not_invited_again_and_a_done_invitation_holds_no_place() {
    let w = World::new();
    let a = w.node().await;
    let cid = create(&w, &a, JoinMode::Invite).await;
    let r = w.node().await;
    let all = [&a, &r];
    let invite = |a: &Node, r: &Node| {
        let (svc, keys, cid, who) = (a.svc.clone(), a.keys.clone(), cid.clone(), pk(r));
        async move { svc.invite(&keys, &cid, &who).await.map(|(id, _)| id) }
    };
    invite(&a, &r).await.unwrap_or_else(|e| panic!("invite: {e}"));
    w.pump(&all).await;
    let got = db::channels::invites(r.svc.db(), "in", &["received"]).await.unwrap_or_default();
    let id = got.first().map(|i| i.invite_id.clone()).unwrap_or_else(|| panic!("no invitation came"));
    let out = r.svc.answer_invite(&id, true).await.unwrap_or_else(|e| panic!("accept: {e}"));
    let _ = out;
    w.pump(&all).await;
    tick(&a).await;
    w.pump(&all).await;
    assert_eq!(membership(&r, &cid).await, MEMBERSHIP_JOINED);
    let err = invite(&a, &r).await.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains(crate::service::ChannelService::ALREADY_MEMBER), "{err}");
    // Three more let in by invitations: four accepted, as many as a channel
    // holds open, and none of them open any more.
    for _ in 0..3 {
        let p = w.node().await;
        invite(&a, &p).await.unwrap_or_else(|e| panic!("invite: {e}"));
        w.pump(&[&a, &p]).await;
        let id = db::channels::invites(p.svc.db(), "in", &["received"]).await.unwrap_or_default().first().map(|i| i.invite_id.clone()).unwrap_or_else(|| panic!("no invitation"));
        p.svc.answer_invite(&id, true).await.unwrap_or_else(|e| panic!("accept: {e}"));
        w.pump(&[&a, &p]).await;
        tick(&a).await;
        w.pump(&[&a, &p]).await;
        assert_eq!(membership(&p, &cid).await, MEMBERSHIP_JOINED);
    }
    let fifth = w.node().await;
    invite(&a, &fifth).await.unwrap_or_else(|e| panic!("a fifth invitation past four done: {e}"));
    // Put out: invited again, the old accepted invitation holds no place.
    w.clock.advance(10);
    a.svc.act(&a.keys, &cid, OpBody::Remove { who: pk(&r) }).await.unwrap_or_else(|e| panic!("remove: {e}"));
    w.pump(&all).await;
    tick(&a).await;
    w.pump(&all).await;
    invite(&a, &r).await.unwrap_or_else(|e| panic!("invited again: {e}"));
}

/// Review of 2026-10-30, finding 1: the owner took the keeper on a phone
/// D while his other devices were offline, D published no epoch (its
/// hosts down) and was lost. Every other device was refused for good
/// (`EPOCHS_NOT_HERE`: none of the current series' epochs here), the app
/// having no `force`. Brought to the head and read to its end with every
/// page answered, a series that published nothing is known so, and the
/// take numbers past it by itself.
#[tokio::test]
async fn a_keeper_series_that_published_nothing_is_taken_past_without_the_owners_word() {
    let w = World::new();
    let (a, b, r, cid) = owner_two_devices(&w).await;
    let d = w.device(a.keys.clone()).await;
    w.pump(&[&a, &b, &r, &d]).await;
    w.clock.advance(600);
    tick(&a).await;
    w.pump(&[&a, &b, &r, &d]).await;
    // D takes the keeper in an emergency; then its hosts are down: no
    // epoch goes out.
    d.svc.keeper_take(&d.keys, &cid, true).await.unwrap_or_else(|e| panic!("take on D: {e}"));
    w.hosts.set_down(H1, true);
    w.hosts.set_down(H2, true);
    for _ in 0..3 {
        w.clock.advance(400);
        tick(&d).await;
        w.pump(&[&d]).await;
    }
    let series = b.svc.log(&cid).await.ok().flatten();
    w.pump(&[&b, &r]).await;
    let d_from = b.svc.log(&cid).await.ok().flatten().and_then(|l| l.state().keeper().map(|k| k.from_epoch)).unwrap_or(0);
    assert!(series.is_some() && d_from > 0);
    assert!(b.svc.newest_epoch_known(&cid).await.unwrap_or(0) < d_from, "D published no epoch");
    // D is lost; the hosts are back. B takes the keeper.
    w.hosts.set_down(H1, false);
    w.hosts.set_down(H2, false);
    let pump = async {
        for _ in 0..60 {
            b.sync().await;
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    };
    let (taken, ()) = tokio::join!(b.svc.keeper_take_at_head(&b.keys, &cid, true, false, std::time::Duration::from_secs(20)), pump);
    taken.unwrap_or_else(|e| panic!("take on B: {e}"));
    let from = b.svc.log(&cid).await.ok().flatten().and_then(|l| l.state().keeper().map(|k| k.from_epoch)).unwrap_or(0);
    assert!(from > d_from, "past D's series: {from} > {d_from}");
    for _ in 0..3 {
        w.clock.advance(400);
        tick(&b).await;
        w.pump(&[&b, &r]).await;
    }
    b.svc.send_post(&b.keys, &cid, "after D", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&[&b, &r]).await;
    assert_eq!(post_texts(&r, &cid).await, ["after D"]);
}

/// The review of 2026-10-30, finding 3: a reader the journal names only to
/// trust (staff entries hold such ones) who left was refused for good as
/// «already a member»; a moderator's device, which knows the readers by a
/// snapshot, refused one removed after it until the next snapshot. In now
/// is an author and above by the journal, or a reader the journal did not
/// put out since, refused where known: the keeper's rows (a leave takes
/// them out at once) and the journal.
#[tokio::test]
async fn one_who_left_or_was_removed_since_is_invited_again() {
    let w = World::new();
    let a = w.node().await;
    let cid = create(&w, &a, JoinMode::Open).await;
    let (r, m, x) = (w.node().await, w.node().await, w.node().await);
    let all = [&a, &r, &m, &x];
    for n in [&r, &m, &x] {
        join_by_link(&w, &a, n, &cid, &all).await;
    }
    w.clock.advance(600);
    tick(&a).await;
    w.pump(&all).await;
    a.svc.act(&a.keys, &cid, OpBody::SetTrusted { who: pk(&r), trusted: true }).await.unwrap_or_else(|e| panic!("trust: {e}"));
    a.svc.act(&a.keys, &cid, OpBody::SetRole { who: pk(&m), role: Role::Moderator }).await.unwrap_or_else(|e| panic!("role: {e}"));
    w.pump(&all).await;
    // In: refused on the keeper.
    let err = a.svc.invite(&a.keys, &cid, &pk(&r)).await.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains(crate::service::ChannelService::ALREADY_MEMBER), "{err}");
    // Left: the trust in the journal is no place in the channel.
    let out = r.svc.leave(&r.keys, &cid).await.unwrap_or_else(|e| panic!("leave: {e}"));
    let _ = out;
    w.pump(&all).await;
    tick(&a).await;
    w.pump(&all).await;
    a.svc.invite(&a.keys, &cid, &pk(&r)).await.unwrap_or_else(|e| panic!("one who left, invited again: {e}"));
    // Removed after the snapshot the moderator's device holds: invited again
    // there at once, not after the next snapshot.
    w.clock.advance(5);
    a.svc.act(&a.keys, &cid, OpBody::Remove { who: pk(&x) }).await.unwrap_or_else(|e| panic!("remove: {e}"));
    w.pump(&[&a, &m, &x]).await;
    m.svc.invite(&m.keys, &cid, &pk(&x)).await.unwrap_or_else(|e| panic!("one removed, invited by the moderator: {e}"));
}

/// Production, 2026-10-30: agent bots made authors by the journal before
/// they were invited (a migration named them staff at once) were refused
/// as «already a member», so the keeper never let them in: no reader row,
/// no key, a follow that failed every 90 s. In the channel, for an
/// invitation, is what the keeper let in; staff it never let in are
/// invited, admitted and given the keys like anyone, their role kept; and
/// before that, the keeper answers their ask for a bundle with a refusal
/// they can read («never let in: invite first») instead of silence.
#[tokio::test]
async fn staff_never_let_in_are_invited_admitted_and_told_why_no_bundle_comes() {
    let w = World::new();
    let a = w.node().await;
    let cid = create(&w, &a, JoinMode::Invite).await;
    let s = w.node().await;
    let all = [&a, &s];
    a.svc.act(&a.keys, &cid, OpBody::SetRole { who: pk(&s), role: Role::Author }).await.unwrap_or_else(|e| panic!("role: {e}"));
    w.pump(&all).await;
    // Not let in: an ask for a bundle is refused, in words.
    s.svc.ask_journal(&cid).await.unwrap_or_else(|e| panic!("ask: {e}"));
    w.pump(&all).await;
    if let Some(log) = s.svc.log(&cid).await.ok().flatten() {
        s.svc.member(&s.keys).ask_rebundle(log.state(), SCOPE_EVERYONE, 0, w.clock.now()).await.unwrap_or_else(|e| panic!("rebundle: {e}"));
    }
    w.pump(&all).await;
    tick(&a).await;
    w.pump(&all).await;
    assert_eq!(s.svc.refused_by_keeper(&cid).await.unwrap_or_default().as_deref(), Some(crate::service::NEVER_LET_IN), "the keeper says why");
    // Invited although staff, admitted on acceptance, the role kept.
    a.svc.invite(&a.keys, &cid, &pk(&s)).await.unwrap_or_else(|e| panic!("staff never let in, invited: {e}"));
    w.pump(&all).await;
    let id = db::channels::invites(s.svc.db(), "in", &["received"]).await.unwrap_or_default().first().map(|i| i.invite_id.clone()).unwrap_or_else(|| panic!("no invitation came"));
    s.svc.answer_invite(&id, true).await.unwrap_or_else(|e| panic!("accept: {e}"));
    w.pump(&all).await;
    tick(&a).await;
    w.pump(&all).await;
    assert!(newest(&s, &cid).await.is_some(), "a key of the channel came");
    assert_eq!(s.svc.refused_by_keeper(&cid).await.unwrap_or_default(), None, "the refusal is gone once let in");
    let role = s.svc.log(&cid).await.ok().flatten().and_then(|l| l.state().role_of(&pk(&s)));
    assert_eq!(role, Some(Role::Author));
    let err = a.svc.invite(&a.keys, &cid, &pk(&s)).await.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains(crate::service::ChannelService::ALREADY_MEMBER), "let in now: {err}");
    s.svc.send_post(&s.keys, &cid, "by the agent", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    assert_eq!(post_texts(&a, &cid).await, ["by the agent"]);
}

// ─── A channel catches up by itself (ROO-270) ───────────────────────────────

/// The gift wraps on the relay so far, to anyone.
fn wraps_on_relay(w: &World) -> usize {
    w.relay.inbox.lock().map(|i| i.values().map(Vec::len).sum::<usize>()).unwrap_or(0)
}

/// Production, RC 5.2.0: «19 more posts will open when the keeper is
/// online», for good. A reader asked for the bundle of a new generation
/// once, when its epoch came; the answer lost, the epoch heard again is a
/// `Duplicate` and asks nothing, and the posts waited for the next epoch.
/// Asked again by the turn once `REBUNDLE_EVERY_SECS` passed, not sooner.
#[tokio::test]
async fn a_rebundle_whose_answer_was_lost_is_asked_again_and_the_key_comes_at_a_later_turn() {
    let w = World::new();
    let (a, b, r, cid) = owner_two_devices(&w).await;
    // A snapshot that names the reader, as an emergency needs.
    w.clock.advance(600);
    tick(&a).await;
    w.pump(&[&a, &b, &r]).await;
    let before = newest(&r, &cid).await.map(|k| k.epoch);
    // A is lost; B takes over by a new generation, whose bundle to the
    // reader is lost on the way.
    b.svc.keeper_take(&b.keys, &cid, true).await.unwrap_or_else(|e| panic!("take: {e}"));
    w.pump(&[&b]).await;
    r.skip_inboxes(&w.relay);
    // The reader hears the epoch of the new generation and asks once; that
    // answer is lost too.
    w.pump(&[&r]).await;
    w.pump(&[&b]).await;
    r.skip_inboxes(&w.relay);
    w.pump(&[&b, &r]).await;
    assert_eq!(newest(&r, &cid).await.map(|k| k.epoch), before, "no key: both answers were lost");
    // What is posted now waits for the keeper, and is said to.
    b.svc.send_post(&b.keys, &cid, "after the loss", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&[&b, &r]).await;
    assert_eq!(waiting(&r, &cid).await, crate::member::Waiting { keeper: 1, ..Default::default() });
    // Within the pause the turn asks nothing.
    let sent = wraps_on_relay(&w);
    tick(&r).await;
    assert_eq!(wraps_on_relay(&w), sent, "asked again within the pause");
    // After it, the next turn asks again, and the key comes.
    w.clock.advance(crate::service::REBUNDLE_EVERY_SECS);
    tick(&r).await;
    w.pump(&[&b, &r]).await;
    let after = newest(&r, &cid).await.map(|k| k.epoch);
    assert!(after > before, "{before:?} → {after:?}");
    assert_eq!(post_texts(&r, &cid).await, ["after the loss"]);
    assert_eq!(waiting(&r, &cid).await.total(), 0);
}

async fn waiting(n: &Node, cid: &str) -> crate::member::Waiting {
    let log = n.svc.log(cid).await.ok().flatten().unwrap_or_else(|| panic!("journal"));
    n.svc.member(&n.keys).waiting(log.state()).await.unwrap_or_else(|e| panic!("waiting: {e}"))
}

/// Posts under an epoch whose event has not come yet wait for the relay,
/// not for the keeper: the page says «catching up», not «when the keeper is
/// online» (RC 5.2.0: the keeper's text for whatever waited).
#[tokio::test]
async fn posts_under_an_epoch_not_heard_yet_wait_for_the_relay_not_for_the_keeper() {
    let w = World::new();
    let a = w.node().await;
    let cid = create(&w, &a, JoinMode::Open).await;
    let r = w.node().await;
    let all = [&a, &r];
    join_by_link(&w, &a, &r, &cid, &all).await;
    r.hold(KIND_EPOCH, true);
    w.clock.advance(400);
    a.svc.rotate_now(&cid).await.unwrap_or_else(|e| panic!("rotate: {e}"));
    tick(&a).await;
    a.svc.send_post(&a.keys, &cid, "under the new epoch", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    assert!(post_texts(&r, &cid).await.is_empty());
    assert_eq!(waiting(&r, &cid).await, crate::member::Waiting { ahead: 1, ..Default::default() });
    r.hold(KIND_EPOCH, false);
    w.pump(&all).await;
    assert_eq!(post_texts(&r, &cid).await, ["under the new epoch"]);
    assert_eq!(waiting(&r, &cid).await.total(), 0);
}

/// `asker` asks the keeper on `keeper` for a bundle, and it answers.
async fn ask_bundle(w: &World, asker: &Node, keeper: &Node, all: &[&Node], cid: &str) {
    let log = asker.svc.log(cid).await.ok().flatten().unwrap_or_else(|| panic!("journal"));
    asker.svc.member(&asker.keys).ask_rebundle(log.state(), SCOPE_EVERYONE, 1, w.clock.now()).await.unwrap_or_else(|e| panic!("rebundle: {e}"));
    w.pump(all).await;
    tick(keeper).await;
    w.pump(all).await;
}

/// Staff whom the keeper let in before, gone from its tree (a keeper
/// recovered without them), are let in again by their ask for a bundle,
/// not told «never let in»: by the keeper's rows of its readers, or by the
/// newest snapshot. One removed after the snapshot is not.
#[tokio::test]
async fn staff_out_of_the_tree_but_known_as_readers_are_let_in_again() {
    let w = World::new();
    let a = w.node().await;
    let cid = create(&w, &a, JoinMode::Open).await;
    let s = w.node().await;
    let all = [&a, &s];
    join_by_link(&w, &a, &s, &cid, &all).await;
    a.svc.act(&a.keys, &cid, OpBody::SetRole { who: pk(&s), role: Role::Author }).await.unwrap_or_else(|e| panic!("role: {e}"));
    w.pump(&all).await;
    let keeper = a.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("keeper"));
    let log = a.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("journal"));
    assert!(!a.svc.member(&a.keys).in_snapshot(&log, &pk(&s)).await.unwrap_or(true), "no snapshot names the author yet");
    // Gone from the tree, its reader row kept: let in again.
    keeper.lock().await.drop_leaf(&pk(&s), w.clock.now()).await.unwrap_or_else(|e| panic!("drop: {e}"));
    assert!(!keeper.lock().await.is_member(&pk(&s)));
    ask_bundle(&w, &s, &a, &all, &cid).await;
    assert!(keeper.lock().await.is_member(&pk(&s)), "let in again by the keeper's rows");
    assert_eq!(s.svc.refused_by_keeper(&cid).await.unwrap_or_default(), None, "not told «never let in»");
    // Gone from the tree and from the rows: the snapshot names them.
    w.clock.advance(600);
    tick(&a).await;
    w.pump(&all).await;
    let log = a.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("journal"));
    assert!(a.svc.member(&a.keys).in_snapshot(&log, &pk(&s)).await.unwrap_or(false), "the snapshot names the author");
    w.clock.advance(5);
    keeper.lock().await.drop_leaf(&pk(&s), w.clock.now()).await.unwrap_or_else(|e| panic!("drop: {e}"));
    db::keeper::reader_out(a.svc.db(), &cid, pk(&s).as_hex()).await.unwrap_or_else(|e| panic!("{e}"));
    ask_bundle(&w, &s, &a, &all, &cid).await;
    assert!(keeper.lock().await.is_member(&pk(&s)), "let in again by the snapshot");
    assert_eq!(s.svc.refused_by_keeper(&cid).await.unwrap_or_default(), None);
    s.svc.send_post(&s.keys, &cid, "by the author", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    assert_eq!(post_texts(&a, &cid).await, ["by the author"]);
    // Removed after the snapshot, the role given again: the snapshot lets
    // nobody the journal put out since back in.
    w.clock.advance(5);
    a.svc.act(&a.keys, &cid, OpBody::Remove { who: pk(&s) }).await.unwrap_or_else(|e| panic!("remove: {e}"));
    w.pump(&all).await;
    tick(&a).await;
    w.pump(&all).await;
    if a.svc.log(&cid).await.ok().flatten().is_some_and(|l| l.state().role_of(&pk(&s)).is_none()) {
        a.svc.act(&a.keys, &cid, OpBody::SetRole { who: pk(&s), role: Role::Author }).await.unwrap_or_else(|e| panic!("role: {e}"));
        w.pump(&all).await;
    }
    let log = a.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("journal"));
    assert!(!a.svc.member(&a.keys).in_snapshot(&log, &pk(&s)).await.unwrap_or(true), "put out after the snapshot");
    ask_bundle(&w, &s, &a, &all, &cid).await;
    assert!(!keeper.lock().await.is_member(&pk(&s)), "the one removed is not let in by the snapshot");
}

/// One the tree lost, removed while out of it, the role given again: its
/// reader row went with the removal and does not let it in. Nor does a row
/// an older run left from before the removal.
#[tokio::test]
async fn one_removed_after_the_tree_lost_them_is_not_let_in_by_the_rows() {
    let w = World::new();
    let a = w.node().await;
    let cid = create(&w, &a, JoinMode::Open).await;
    let s = w.node().await;
    let all = [&a, &s];
    join_by_link(&w, &a, &s, &cid, &all).await;
    a.svc.act(&a.keys, &cid, OpBody::SetRole { who: pk(&s), role: Role::Author }).await.unwrap_or_else(|e| panic!("role: {e}"));
    w.pump(&all).await;
    let keeper = a.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("keeper"));
    keeper.lock().await.drop_leaf(&pk(&s), w.clock.now()).await.unwrap_or_else(|e| panic!("drop: {e}"));
    let since = db::keeper::reader_since(a.svc.db(), &cid, pk(&s).as_hex()).await.ok().flatten().unwrap_or_else(|| panic!("the row is kept"));
    w.clock.advance(5);
    a.svc.act(&a.keys, &cid, OpBody::Remove { who: pk(&s) }).await.unwrap_or_else(|e| panic!("remove: {e}"));
    w.pump(&all).await;
    tick(&a).await;
    w.pump(&all).await;
    assert_eq!(db::keeper::reader_since(a.svc.db(), &cid, pk(&s).as_hex()).await.unwrap_or_default(), None, "the removal drops the row");
    w.clock.advance(5);
    a.svc.act(&a.keys, &cid, OpBody::SetRole { who: pk(&s), role: Role::Author }).await.unwrap_or_else(|e| panic!("role: {e}"));
    w.pump(&all).await;
    ask_bundle(&w, &s, &a, &all, &cid).await;
    assert!(!keeper.lock().await.is_member(&pk(&s)), "no row: not let in");
    // A row from before the removal (an older run kept it): weighed
    // against the time of the removal, it lets nobody in.
    db::keeper::reader_in(a.svc.db(), &cid, pk(&s).as_hex(), since).await.unwrap_or_else(|e| panic!("{e}"));
    w.clock.advance(5);
    ask_bundle(&w, &s, &a, &all, &cid).await;
    assert!(!keeper.lock().await.is_member(&pk(&s)), "a row older than the removal: not let in");
}

/// Staff made a plain member again, its mute (a setting the journal keeps)
/// still on: named by the journal, but no staff, it is not let in again by
/// its row when the tree lost it.
#[tokio::test]
async fn staff_made_a_member_with_settings_kept_is_not_let_in_by_the_rows() {
    let w = World::new();
    let a = w.node().await;
    let cid = create(&w, &a, JoinMode::Open).await;
    let s = w.node().await;
    let all = [&a, &s];
    join_by_link(&w, &a, &s, &cid, &all).await;
    a.svc.act(&a.keys, &cid, OpBody::SetRole { who: pk(&s), role: Role::Author }).await.unwrap_or_else(|e| panic!("role: {e}"));
    a.svc.act(&a.keys, &cid, OpBody::SetMuted { who: pk(&s), muted: true, until: None }).await.unwrap_or_else(|e| panic!("mute: {e}"));
    a.svc.act(&a.keys, &cid, OpBody::SetRole { who: pk(&s), role: Role::Member }).await.unwrap_or_else(|e| panic!("role: {e}"));
    w.pump(&all).await;
    let log = a.svc.log(&cid).await.ok().flatten().unwrap_or_else(|| panic!("journal"));
    assert_eq!(log.state().role_of(&pk(&s)), Some(Role::Member), "the journal still names them");
    let keeper = a.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("keeper"));
    keeper.lock().await.drop_leaf(&pk(&s), w.clock.now()).await.unwrap_or_else(|e| panic!("drop: {e}"));
    assert!(db::keeper::reader_since(a.svc.db(), &cid, pk(&s).as_hex()).await.ok().flatten().is_some(), "the row is kept");
    ask_bundle(&w, &s, &a, &all, &cid).await;
    assert!(!keeper.lock().await.is_member(&pk(&s)), "a member is not let in again by its row");
}

/// The owner is never told «never let in»: a device of the owner's that
/// asks for a bundle while the keeper's tree lacks the owner (rows lost
/// with it) is let in again.
#[tokio::test]
async fn the_owner_is_never_told_not_let_in_and_is_let_in_again() {
    let w = World::new();
    let (a, b, r, cid) = owner_two_devices(&w).await;
    let all = [&a, &b, &r];
    let keeper = a.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("keeper"));
    keeper.lock().await.drop_leaf(&pk(&b), w.clock.now()).await.unwrap_or_else(|e| panic!("drop: {e}"));
    db::keeper::reader_out(a.svc.db(), &cid, pk(&b).as_hex()).await.unwrap_or_else(|e| panic!("{e}"));
    assert!(!keeper.lock().await.is_member(&pk(&b)));
    ask_bundle(&w, &b, &a, &all, &cid).await;
    assert_eq!(b.svc.refused_by_keeper(&cid).await.unwrap_or_default(), None, "the owner is never refused");
    assert!(keeper.lock().await.is_member(&pk(&b)), "the owner is let in again");
    let not_let_in = b.errors().iter().chain(a.errors().iter()).filter(|e| e.contains("not_let_in")).count();
    assert_eq!(not_let_in, 0);
}
