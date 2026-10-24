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
