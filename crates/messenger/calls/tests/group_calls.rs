// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Group calls between cores on the fake engine and the fake node: the
//! notes of the group carried by hand to every member (the fake groups
//! of these tests), the control channel spoken by the fake node. A party
//! may be made deaf for a while: the notes to it are held and handed
//! over later, in order or reversed, as the relays would.

use messenger_calls::engine::ConnectionState;
use messenger_calls::group::keys::{sender_key, slot};
use messenger_calls::group::service::Timing;
use messenger_calls::{
    GroupAccess, GroupCallService, GroupCallView, GroupPhase, Media, NodeError, SessionEvent, StaticServerSets, UI_EVENT_GROUP_CALL_ENDED,
    UI_EVENT_GROUP_CALL_STARTED, UI_EVENT_GROUP_CALL_STATE,
};
use messenger_contacts::{ContactService, ProfileService};
use messenger_core::traits::UiEvent;
use messenger_core::{Clock, Effect, Envelope, Outbound, PubKey, Timestamp};
use messenger_dm::DmService;
use messenger_store::{calls as repo, Store};
use messenger_testkit::{FakeEngine, FakeGroups, FakeNode};
use nostr::key::Keys;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;

struct TestClock(AtomicI64);

impl Clock for TestClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.0.load(Ordering::SeqCst))
    }
}

const START: i64 = 1_760_000_000;
const GROUP: &str = "00000000000000000000000000000000000000000000000000000000000000aa";
/// The sending moves to a new epoch this much after it is learned, in
/// these tests (a second and a half in life).
const SWITCH: Duration = Duration::from_millis(30);

struct Party {
    name: &'static str,
    keys: Keys,
    store: Store,
    groups: Arc<FakeGroups>,
    calls: GroupCallService,
    rx: Mutex<UnboundedReceiver<Effect>>,
    ui: Mutex<Vec<UiEvent>>,
    sent: Mutex<Vec<Outbound>>,
    /// When each of `sent` was written (its `created_at` in life).
    sent_at: Mutex<Vec<i64>>,
}

impl Party {
    async fn new(name: &'static str, clock: Arc<TestClock>, engine: FakeEngine, node: FakeNode) -> Self {
        let keys = Keys::generate();
        let store = Store::open_in_memory().await.unwrap();
        Self::over(name, keys, store, clock, engine, node).await
    }

    /// The party over `store` with `keys`: a fresh core, as after a
    /// restart of the app (nothing of the old one is in memory; the
    /// record is).
    async fn over(name: &'static str, keys: Keys, store: Store, clock: Arc<TestClock>, engine: FakeEngine, node: FakeNode) -> Self {
        let profiles = ProfileService::new(store.clone());
        let contacts = ContactService::new(store.clone(), profiles.clone());
        let dm = DmService::new(store.clone(), contacts, profiles, clock.clone());
        dm.set_signer(Some(keys.clone()));
        let me = PubKey::parse(&keys.public_key().to_hex()).unwrap();
        let groups = Arc::new(FakeGroups::new(me));
        let (calls, rx) = GroupCallService::new(
            store.clone(),
            dm,
            groups.clone(),
            Arc::new(engine),
            Arc::new(StaticServerSets(vec![node.as_call_node()])),
            node.client(),
            Arc::new(node),
            clock,
        );
        calls.set_timing(Timing { send_switch_delay: SWITCH, ..Timing::default() });
        calls.set_signer(Some(keys.clone()));
        Self { name, keys, store, groups, calls, rx: Mutex::new(rx), ui: Mutex::new(vec![]), sent: Mutex::new(vec![]), sent_at: Mutex::new(vec![]) }
    }

    fn pk(&self) -> PubKey {
        PubKey::parse(&self.keys.public_key().to_hex()).unwrap()
    }

    fn events(&self, name: &str) -> Vec<serde_json::Value> {
        self.ui.lock().unwrap().iter().filter(|e| e.name == name).map(|e| e.payload.clone()).collect()
    }

    fn last_state(&self) -> Option<GroupCallView> {
        self.ui
            .lock()
            .unwrap()
            .iter()
            .rfind(|e| e.name == UI_EVENT_GROUP_CALL_STATE)
            .map(|e| serde_json::from_value(e.payload["call"].clone()).unwrap())
    }

    fn notes(&self) -> Vec<String> {
        self.sent.lock().unwrap().iter().filter_map(FakeGroups::open_note).map(|(_, _, e)| e.t).collect()
    }

    /// The notes sent, whole.
    fn envelopes(&self) -> Vec<Envelope> {
        self.sent.lock().unwrap().iter().filter_map(FakeGroups::open_note).map(|(_, _, e)| e).collect()
    }

    fn errors(&self) -> Vec<String> {
        self.events("error").iter().map(|p| p["error"].to_string()).collect()
    }

    fn count(&self, note: &str) -> usize {
        self.notes().iter().filter(|t| *t == note).count()
    }
}

/// A note held for a deaf party: the group, the author, the note, when it
/// was written.
type HeldNote = (String, PubKey, Envelope, i64);

struct World {
    clock: Arc<TestClock>,
    parties: Vec<Arc<Party>>,
    engine: FakeEngine,
    node: FakeNode,
    /// Parties that hear no note for now, and what they will hear.
    deaf: Mutex<HashSet<usize>>,
    /// With the time each note was written (its `created_at` in life).
    held: Mutex<HashMap<usize, Vec<HeldNote>>>,
}

impl World {
    async fn new(names: &[&'static str]) -> Self {
        let clock = Arc::new(TestClock(AtomicI64::new(START)));
        let engine = FakeEngine::new();
        let node = FakeNode::new(engine.clone());
        let mut parties = vec![];
        for name in names {
            parties.push(Arc::new(Party::new(name, clock.clone(), engine.clone(), node.clone()).await));
        }
        Self { clock, parties, engine, node, deaf: Mutex::default(), held: Mutex::default() }
    }

    fn p(&self, i: usize) -> &Arc<Party> {
        &self.parties[i]
    }

    /// The app of the party restarts: a fresh core over the same record
    /// and keys, nothing of the old one in memory, a member of the same
    /// group. The old core is dropped unheard.
    async fn restart(&mut self, i: usize) {
        let old = self.parties[i].clone();
        let members: Vec<PubKey> = old.groups.members(GROUP).await.unwrap_or_default();
        let fresh = Party::over(old.name, old.keys.clone(), old.store.clone(), self.clock.clone(), self.engine.clone(), self.node.clone()).await;
        fresh.groups.set_members(GROUP, members);
        self.parties[i] = Arc::new(fresh);
    }

    /// Another device of the party: the same keys over a record of its
    /// own, a member wherever the party is. Its index.
    async fn twin(&mut self, i: usize, name: &'static str) -> usize {
        let old = self.parties[i].clone();
        let members: Vec<PubKey> = old.groups.members(GROUP).await.unwrap_or_default();
        let store = Store::open_in_memory().await.unwrap();
        let fresh = Party::over(name, old.keys.clone(), store, self.clock.clone(), self.engine.clone(), self.node.clone()).await;
        fresh.groups.set_members(GROUP, members);
        self.parties.push(Arc::new(fresh));
        self.parties.len() - 1
    }

    /// Every note of the group sent so far, with its author and when it
    /// was written, oldest first: what the relays would give back after
    /// a restart.
    fn history(&self) -> Vec<(PubKey, Envelope, i64)> {
        let mut out = vec![];
        for p in &self.parties {
            let at = p.sent_at.lock().unwrap().clone();
            out.extend(p.envelopes().into_iter().zip(at).map(|(e, at)| (p.pk(), e, at)));
        }
        out.sort_by_key(|(_, _, at)| *at);
        out
    }

    /// The party hears `notes` as the history, newest first (the relays
    /// keep no order).
    async fn replay(&self, i: usize, notes: &[(PubKey, Envelope, i64)]) {
        for (author, envelope, at) in notes.iter().rev() {
            self.p(i).calls.on_group_note(GROUP, author, envelope, *at).await.unwrap();
        }
    }

    /// Everybody of `members` is a member of the group, in everybody's eyes.
    fn group(&self, members: &[usize]) {
        let mut keys: Vec<PubKey> = vec![];
        for i in members {
            let key = self.p(*i).pk();
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        for i in members {
            self.p(*i).groups.set_members(GROUP, keys.clone());
        }
    }

    fn now(&self) -> i64 {
        self.clock.0.load(Ordering::SeqCst)
    }

    fn timing(&self, timing: Timing) {
        for p in &self.parties {
            p.calls.set_timing(timing);
        }
    }

    /// The notes to the party are held from now on.
    fn deafen(&self, i: usize) {
        self.deaf.lock().unwrap().insert(i);
    }

    /// The party hears again, and gets what was held, in order.
    async fn hear(&self, i: usize) {
        self.deaf.lock().unwrap().remove(&i);
        let held = self.held.lock().unwrap().remove(&i).unwrap_or_default();
        for (g, author, envelope, at) in held {
            self.p(i).calls.on_group_note(&g, &author, &envelope, at).await.unwrap();
        }
    }

    /// The party hears again, and gets what was held newest first (the
    /// relays keep no order).
    async fn hear_reversed(&self, i: usize) {
        self.deaf.lock().unwrap().remove(&i);
        let mut held = self.held.lock().unwrap().remove(&i).unwrap_or_default();
        held.reverse();
        for (g, author, envelope, at) in held {
            self.p(i).calls.on_group_note(&g, &author, &envelope, at).await.unwrap();
        }
    }

    /// Carry everything until nothing moves for a few passes.
    async fn settle(&self) {
        let mut quiet = 0;
        for _ in 0..500 {
            let mut moved = false;
            for _ in 0..4 {
                tokio::task::yield_now().await;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
            for party in &self.parties {
                let effects: Vec<Effect> = {
                    let mut rx = party.rx.lock().unwrap();
                    let mut out = vec![];
                    while let Ok(e) = rx.try_recv() {
                        out.push(e);
                    }
                    out
                };
                for e in effects {
                    moved = true;
                    match e {
                        Effect::Emit(ev) => party.ui.lock().unwrap().push(ev),
                        Effect::Notify(_) => {}
                        Effect::Send(out) => {
                            party.sent.lock().unwrap().push(out.clone());
                            party.sent_at.lock().unwrap().push(self.now());
                            self.carry(out).await;
                        }
                    }
                }
            }
            if moved {
                quiet = 0;
            } else {
                quiet += 1;
                if quiet >= 5 {
                    return;
                }
            }
        }
        panic!("the world does not settle");
    }

    /// Let the timers of the rooms run (the move of the sending to a new
    /// epoch, a seat's time to say who it is), then settle.
    async fn wait(&self, d: Duration) {
        self.settle().await;
        tokio::time::sleep(d).await;
        self.settle().await;
    }

    /// A note of the group to every party that counts itself a member
    /// (the author's own copy too); held for a deaf one.
    async fn carry(&self, out: Outbound) {
        let Some((group_id, author, envelope)) = FakeGroups::open_note(&out) else { panic!("not a group note: {out:?}") };
        for (i, party) in self.parties.iter().enumerate() {
            if party.groups.members(&group_id).await.is_err() {
                continue;
            }
            if self.deaf.lock().unwrap().contains(&i) {
                self.held.lock().unwrap().entry(i).or_default().push((group_id.clone(), author.clone(), envelope.clone(), self.now()));
                continue;
            }
            party.calls.on_group_note(&group_id, &author, &envelope, self.now()).await.unwrap();
        }
    }
}

fn verified_seats(v: &GroupCallView) -> Vec<(u32, bool)> {
    v.participants.iter().filter(|p| !p.me).map(|p| (p.id, p.verified)).collect()
}

/// The last key a session set for `mid` in `slot`.
fn receiver_key(record: &messenger_testkit::fake_engine::Record, mid: &str, s: u8) -> Option<Vec<u8>> {
    record.receiver_keys.iter().filter(|(m, i, _)| m == mid && *i == s).map(|(_, _, k)| k.clone()).next_back()
}

/// The last key a session sends with in `slot`.
fn sender_key_in(record: &messenger_testkit::fake_engine::Record, s: u8) -> Option<Vec<u8>> {
    record.sender_keys.iter().filter(|(i, _)| *i == s).map(|(_, k)| k.clone()).next_back()
}

#[tokio::test]
async fn a_call_of_three_with_identities_and_a_rotation_on_leaving() {
    let w = World::new(&["alice", "bob", "carol"]).await;
    w.group(&[0, 1, 2]);
    let (alice, bob, carol) = (w.p(0), w.p(1), w.p(2));

    // Alice starts: a room on the node, her seat, the group told.
    let view = alice.calls.start(GROUP, Media::Audio).await.unwrap();
    assert_eq!(view.phase, GroupPhase::Joining);
    assert_eq!(view.participant, Some(1));
    w.settle().await;
    assert_eq!(alice.last_state().unwrap().phase, GroupPhase::InRoom);
    assert_eq!(alice.notes(), vec!["call.start", "call.join"]);
    assert_eq!(w.node.rooms().len(), 1);
    let room = w.node.rooms()[0].clone();
    // Everybody has the banner, with Alice in it.
    for p in [alice, bob, carol] {
        let banner = p.calls.announced(GROUP).await.unwrap_or_else(|| panic!("{}: no banner", p.name));
        assert_eq!(banner.started_by, alice.pk().as_hex());
        assert_eq!(banner.participants, vec![alice.pk().as_hex().to_string()], "{}", p.name);
        assert_eq!(banner.joined, p.name == "alice");
        assert!(!p.events(UI_EVENT_GROUP_CALL_STARTED).is_empty());
    }
    // The record: a line in the group's chat, out for Alice, in for Bob.
    let row = repo::get(&alice.store, &view.call_id).await.unwrap().unwrap();
    assert_eq!((row.direction.as_str(), row.chat_id.as_str()), ("out", "group:00000000000000000000000000000000000000000000000000000000000000aa"));
    assert_eq!(repo::get(&bob.store, &view.call_id).await.unwrap().unwrap().direction, "in");
    assert_eq!(repo::group_info(&bob.store, &view.call_id).await.unwrap().unwrap().started_by.as_deref(), Some(alice.pk().as_hex()));

    // Bob joins: both see each other, verified by their words of identity.
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    let (a, b) = (alice.last_state().unwrap(), bob.last_state().unwrap());
    assert_eq!(b.phase, GroupPhase::InRoom);
    assert_eq!(verified_seats(&a), vec![(2, true)], "alice sees bob");
    assert_eq!(verified_seats(&b), vec![(1, true)], "bob sees alice");
    assert_eq!(a.participants[1].npub.as_deref(), Some(bob.pk().as_hex()));
    assert!(a.participants[1].audio);
    assert_eq!(a.epoch, 1);
    // Bob's key for Alice's m-line is Alice's sending key of epoch 1.
    let sessions = w.engine.sessions();
    let (sa, sb) = (sessions[0].record(), sessions[1].record());
    assert_eq!(sa.sender_keys.len(), 1);
    assert_eq!(sa.sender_keys[0].0, slot(1));
    assert_eq!(sb.receiver_keys.iter().find(|(mid, _, _)| mid == "a1").map(|(_, i, k)| (*i, k.clone())), Some((slot(1), sa.sender_keys[0].1.clone())));
    assert_eq!(sa.receiver_keys.iter().find(|(mid, _, _)| mid == "a2").map(|(_, _, k)| k.clone()), Some(sb.sender_keys[0].1.clone()));
    assert_eq!(w.node.answers().len(), 2, "each answered the node's offer of the other");
    assert_eq!(carol.calls.announced(GROUP).await.unwrap().participants.len(), 2);
    assert_eq!(repo::group_info(&carol.store, &view.call_id).await.unwrap().unwrap().participants, 2);

    // Carol comes late: three in the room, everybody verified by everybody.
    carol.calls.join(GROUP).await.unwrap();
    w.settle().await;
    for p in [alice, bob, carol] {
        let v = p.last_state().unwrap();
        assert_eq!(v.participants.len(), 3, "{}: {:?}", p.name, v.participants);
        assert!(v.participants.iter().all(|s| s.verified), "{}: {:?}", p.name, v.participants);
    }

    // Bob leaves: Alice, the oldest seat, makes epoch 2; Carol moves to it.
    bob.calls.leave().await.unwrap();
    w.wait(SWITCH * 3).await;
    assert_eq!(bob.last_state().unwrap().phase, GroupPhase::Left);
    assert!(bob.notes().contains(&"call.leave".to_string()));
    assert!(!bob.notes().contains(&"call.end".to_string()), "not the last one out");
    assert!(alice.notes().contains(&"call.epoch".to_string()));
    assert!(!carol.notes().contains(&"call.epoch".to_string()), "only the oldest seat rotates");
    let (a, c) = (alice.last_state().unwrap(), carol.last_state().unwrap());
    assert_eq!((a.epoch, c.epoch), (2, 2));
    assert_eq!(verified_seats(&a), vec![(3, true)]);
    let sessions = w.engine.sessions();
    let (sa, sc) = (sessions[0].record(), sessions[2].record());
    assert_eq!(sa.sender_keys.last().unwrap().0, slot(2));
    assert_eq!(sc.sender_keys.last().unwrap().0, slot(2));
    let carols_key_for_alice: Vec<&(String, u8, Vec<u8>)> = sc.receiver_keys.iter().filter(|(mid, i, _)| mid == "a1" && *i == slot(2)).collect();
    assert_eq!(carols_key_for_alice.last().map(|(_, _, k)| k.clone()), Some(sa.sender_keys.last().unwrap().1.clone()));
    assert_eq!(bob.calls.announced(GROUP).await.unwrap().participants.len(), 2, "bob still sees the banner of the others");
    assert_eq!(w.node.seats(&room).len(), 2);

    // Carol leaves, then Alice: the last one out ends the call.
    carol.calls.leave().await.unwrap();
    w.wait(SWITCH * 3).await;
    assert!(alice.notes().iter().filter(|t| *t == "call.epoch").count() >= 2, "another epoch as carol left");
    alice.calls.leave().await.unwrap();
    w.settle().await;
    assert!(alice.notes().contains(&"call.end".to_string()));
    for p in [alice, bob, carol] {
        assert!(p.calls.announced(GROUP).await.is_none(), "{}: the banner is gone", p.name);
        assert_eq!(p.events(UI_EVENT_GROUP_CALL_ENDED).len(), 1, "{}", p.name);
        let row = repo::get(&p.store, &view.call_id).await.unwrap().unwrap();
        assert_eq!(row.outcome.as_deref(), Some("ended"), "{}", p.name);
        assert_eq!(repo::group_info(&p.store, &view.call_id).await.unwrap().unwrap().participants, 3, "{}", p.name);
    }
    assert!(alice.calls.current().await.is_none());
    assert!(w.node.seats(&room).is_empty());
    for p in [alice, bob, carol] {
        assert!(p.errors().is_empty(), "{}: {:?}", p.name, p.errors());
    }
}

#[tokio::test]
async fn a_stranger_cannot_join_and_an_unconfirmed_seat_is_not_listened_to() {
    let w = World::new(&["alice", "bob", "mallory"]).await;
    // Alice and Bob are the group; Mallory counts herself a member but
    // the others do not (she has the token and the secret, somehow).
    w.group(&[0, 1]);
    let (alice, bob, mallory) = (w.p(0), w.p(1), w.p(2));
    let err = mallory.calls.start(GROUP, Media::Audio).await.unwrap_err();
    assert!(err.to_string().contains("group_unknown"), "{err}");
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    assert!(mallory.calls.announced(GROUP).await.is_none(), "no note reaches a stranger");
    assert!(mallory.calls.join(GROUP).await.is_err());

    // Now she has the notes (a leaked key): she gets a seat on the node,
    // but her word of identity is not a member's.
    mallory.groups.set_members(GROUP, vec![alice.pk(), bob.pk(), mallory.pk()]);
    let out = alice.sent.lock().unwrap().clone();
    for o in out {
        let (g, author, envelope) = FakeGroups::open_note(&o).unwrap();
        mallory.calls.on_group_note(&g, &author, &envelope, w.now()).await.unwrap();
    }
    mallory.calls.join(GROUP).await.unwrap();
    w.settle().await;
    let a = alice.last_state().unwrap();
    assert_eq!(verified_seats(&a), vec![(2, false)], "alice shows the seat as nobody");
    assert_eq!(a.participants[1].npub, None);
    let sa = w.engine.sessions()[0].record();
    assert!(sa.receiver_keys.iter().all(|(mid, _, _)| mid != "a2"), "no key for her m-line: {:?}", sa.receiver_keys);
    // And her word does not count her among the people of the call.
    assert_eq!(alice.calls.announced(GROUP).await.unwrap().participants, vec![alice.pk().as_hex().to_string()]);

    // Bob, a member, is verified as before.
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    let a = alice.last_state().unwrap();
    assert_eq!(verified_seats(&a), vec![(2, false), (3, true)]);
}

#[tokio::test]
async fn the_node_has_the_last_word_on_a_room() {
    let w = World::new(&["alice", "bob"]).await;
    w.group(&[0, 1]);
    let (alice, bob) = (w.p(0), w.p(1));

    // No node with an SFU answers: the start fails and is shown so.
    w.node.refuse_next(NodeError::Refused { status: 429, error: "rooms_full".into(), message: "x".into() });
    let err = alice.calls.start(GROUP, Media::Video).await.unwrap_err();
    assert!(err.to_string().contains("rooms_full"), "{err}");
    w.settle().await;
    assert!(alice.calls.current().await.is_none());
    assert!(alice.notes().is_empty(), "nothing was told");

    // A video call: the camera goes on in the room.
    let view = alice.calls.start(GROUP, Media::Video).await.unwrap();
    w.settle().await;
    assert!(alice.last_state().unwrap().video_local);
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    let b = bob.last_state().unwrap();
    assert_eq!(b.participants[1].video_mid.as_deref(), Some("v1"), "alice's video is on an m-line of the node");
    assert!(bob.calls.video_frames("v1").await.is_some());
    let sb = w.engine.sessions()[1].record();
    assert!(sb.receiver_keys.iter().any(|(mid, _, _)| mid == "v1"), "keyed as her audio is");
    assert_eq!(sb.receiver_keys.iter().find(|(mid, _, _)| mid == "v1").unwrap().2, sender_key(&secret_of(alice, &view.call_id), &view.call_id, 1, 1));

    // The node says who speaks.
    let room = w.node.rooms()[0].clone();
    w.node.speaking(&room, vec![1]);
    w.settle().await;
    assert!(bob.last_state().unwrap().participants[1].speaking);
    assert!(!alice.last_state().unwrap().participants[0].speaking, "my own seat is not told");

    // The node closes the room (it expired, the admin ended it): both are
    // out, the call ends for the group.
    w.node.end_room(&room);
    w.settle().await;
    assert_eq!(alice.last_state().unwrap().phase, GroupPhase::Left);
    assert_eq!(bob.last_state().unwrap().phase, GroupPhase::Left);
    assert!(alice.calls.current().await.is_none());
    // Nobody told the group (the node closed them, not they themselves):
    // the banner stays until a join finds no room.
    assert!(alice.calls.announced(GROUP).await.is_some());
    let err = bob.calls.join(GROUP).await.unwrap_err();
    assert!(err.to_string().contains("room_not_found"), "{err}");
    w.settle().await;
    assert!(bob.calls.announced(GROUP).await.is_none(), "the node has no such room: over");
    assert_eq!(bob.events(UI_EVENT_GROUP_CALL_ENDED).len(), 1);
    assert_eq!(repo::get(&bob.store, &view.call_id).await.unwrap().unwrap().outcome.as_deref(), Some("ended"));
    assert!(alice.calls.announced(GROUP).await.is_some(), "alice has not asked the node");

    // A start that is old by the time it comes is on record and not on the banner.
    w.clock.0.fetch_add(13 * 3600, Ordering::SeqCst);
    assert!(alice.calls.announced(GROUP).await.is_none(), "the room would have expired by now");
}

/// The camera of a phone in a room, as in a call between two: the engine
/// lists no cameras, so they are `front` and `back`; a switch while my
/// video is on turns the camera at once, one while it is off is kept for
/// the next time; the frames the plugin pushes go into the room's session
/// and show as my own picture.
#[tokio::test]
async fn the_camera_of_a_phone_goes_into_the_room_as_into_a_call_between_two() {
    let w = World::new(&["alice"]).await;
    w.group(&[0]);
    let alice = w.p(0);
    let frame = || messenger_calls::PushedFrame { format: messenger_calls::PixelFormat::Nv21, width: 4, height: 2, rotation: 90, timestamp_us: 7, data: vec![0; 12] };
    assert!(matches!(alice.calls.push_video_frame(frame()).await, Err(messenger_core::MessengerError::Invalid(_))), "no room, no frames");
    assert!(alice.calls.switch_camera(None).await.is_err());

    let view = alice.calls.start(GROUP, Media::Video).await.unwrap();
    w.settle().await;
    let v = alice.last_state().unwrap();
    assert!(v.video_local && v.camera.is_none(), "the default camera at the start: {v:?}");
    let session = w.engine.sessions()[0].clone();
    assert_eq!(session.record().video.last(), Some(&messenger_calls::VideoInput::Camera { id: None }));
    let mut local = session.frames(messenger_calls::VideoTrack::Local).expect("the frames of my own video");
    alice.calls.push_video_frame(frame()).await.unwrap();
    assert_eq!(session.record().pushed_frames, 1);
    let shown = local.try_recv().expect("the pushed frame shows as my own picture");
    assert_eq!((shown.width, shown.height, shown.rotation), (4, 2, 90));

    // The other camera, at once.
    let v = alice.calls.switch_camera(None).await.unwrap();
    assert_eq!(v.camera.as_deref(), Some("back"), "the second of a phone's two");
    assert!(v.video_local);
    assert_eq!(session.record().video.last(), Some(&messenger_calls::VideoInput::Camera { id: Some("back".into()) }));
    // Off: the camera is remembered; a switch now is for the next time.
    let v = alice.calls.set_video(messenger_calls::VideoInput::Off).await.unwrap();
    assert!(!v.video_local);
    assert_eq!(v.camera.as_deref(), Some("back"));
    let turns = session.record().video.len();
    let v = alice.calls.switch_camera(None).await.unwrap();
    assert_eq!((v.camera.as_deref(), v.video_local), (Some("front"), false));
    assert_eq!(session.record().video.len(), turns, "nothing asked of the engine while my video is off");
    let v = alice.calls.set_video(messenger_calls::VideoInput::Camera { id: v.camera.clone() }).await.unwrap();
    assert!(v.video_local && v.camera.as_deref() == Some("front"));
    let v = alice.calls.switch_camera(Some("back".into())).await.unwrap();
    assert_eq!(v.camera.as_deref(), Some("back"));
    w.settle().await;
    assert_eq!(alice.last_state().unwrap().camera.as_deref(), Some("back"), "the screen hears of the camera");
    assert_eq!(alice.last_state().unwrap().call_id, view.call_id);
    alice.calls.leave().await.unwrap();
    w.settle().await;
    assert!(alice.errors().is_empty(), "{:?}", alice.errors());
}

/// After a restart the history of the group tells of the calls again. A
/// call that is over on this device's record (I saw its end: my own last
/// leave, the node's word, its expiry) is not announced again, whatever
/// notes of its end the relays did not give back.
#[tokio::test]
async fn a_call_over_on_record_is_not_announced_again_after_a_restart() {
    let mut w = World::new(&["alice", "bob"]).await;
    w.group(&[0, 1]);
    let (alice, bob) = (w.p(0), w.p(1));
    let view = alice.calls.start(GROUP, Media::Video).await.unwrap();
    w.settle().await;
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    bob.calls.leave().await.unwrap();
    w.wait(SWITCH * 3).await;
    alice.calls.leave().await.unwrap();
    w.settle().await;
    assert!(alice.notes().contains(&"call.end".to_string()), "the last one out ended it");
    for p in [alice, bob] {
        assert!(p.calls.announced(GROUP).await.is_none(), "{}", p.name);
        assert_eq!(repo::get(&p.store, &view.call_id).await.unwrap().unwrap().outcome.as_deref(), Some("ended"), "{}", p.name);
    }
    // The history without the end (the note expired on the relays): the
    // start, the joins, the leaves, the epoch of Bob's leave.
    let history: Vec<(PubKey, Envelope, i64)> = w.history().into_iter().filter(|(_, e, _)| e.t != "call.end").collect();
    let kinds: Vec<&str> = history.iter().map(|(_, e, _)| e.t.as_str()).collect();
    assert!(kinds.contains(&"call.start") && kinds.iter().filter(|t| **t == "call.leave").count() == 2, "{kinds:?}");
    let end = w.history().into_iter().find(|(_, e, _)| e.t == "call.end").unwrap();

    for i in [0, 1] {
        w.restart(i).await;
        w.replay(i, &history).await;
        w.settle().await;
        let p = w.p(i);
        assert!(p.calls.announced(GROUP).await.is_none(), "{}: no ghost of a call that is over", p.name);
        assert!(p.events(UI_EVENT_GROUP_CALL_STARTED).is_empty(), "{}: the banner was not raised", p.name);
        assert!(p.events(UI_EVENT_GROUP_CALL_ENDED).is_empty(), "{}: nothing to end", p.name);
        assert_eq!(repo::get(&p.store, &view.call_id).await.unwrap().unwrap().outcome.as_deref(), Some("ended"), "{}", p.name);
        assert!(p.errors().is_empty(), "{}: {:?}", p.name, p.errors());
    }
    // A late note of the call changes nothing either.
    w.replay(1, std::slice::from_ref(&end)).await;
    w.settle().await;
    assert!(w.p(1).events(UI_EVENT_GROUP_CALL_ENDED).is_empty());
}

/// The app was closed in the room (no leave, no end): after the restart
/// the history tells of my own join, but I sit in no room, so I am not
/// on the banner as a ghost; and the node, which has no such room any
/// more, has the last word when I ask it.
#[tokio::test]
async fn my_own_seat_of_before_the_restart_is_no_ghost_and_the_node_closes_the_call() {
    let mut w = World::new(&["alice", "bob"]).await;
    w.group(&[0, 1]);
    let (alice, bob) = (w.p(0), w.p(1));
    let view = alice.calls.start(GROUP, Media::Video).await.unwrap();
    w.settle().await;
    assert_eq!(bob.calls.announced(GROUP).await.unwrap().participants, vec![alice.pk().as_hex().to_string()]);
    let room = w.node.rooms()[0].clone();
    // The app dies in the room; the node ends the empty room later.
    w.node.end_room(&room);
    w.settle().await;
    assert_eq!(repo::get(&alice.store, &view.call_id).await.unwrap().unwrap().outcome, None, "nobody closed the record");
    let history = w.history();
    assert_eq!(history.iter().map(|(_, e, _)| e.t.as_str()).collect::<Vec<_>>(), ["call.start", "call.join"]);

    w.restart(0).await;
    w.replay(0, &history).await;
    w.settle().await;
    let alice = w.p(0);
    let banner = alice.calls.announced(GROUP).await.expect("the start without an end is announced: others may sit in it");
    assert!(banner.participants.is_empty(), "my seat of before the restart is nobody's: {:?}", banner.participants);
    assert!(!banner.joined);
    // The node is the judge: no such room, no such call.
    let err = alice.calls.join(GROUP).await.unwrap_err();
    assert!(err.to_string().contains("room_not_found"), "{err}");
    w.settle().await;
    assert!(alice.calls.announced(GROUP).await.is_none());
    assert_eq!(alice.events(UI_EVENT_GROUP_CALL_ENDED).len(), 1);
    assert_eq!(repo::get(&alice.store, &view.call_id).await.unwrap().unwrap().outcome.as_deref(), Some("ended"));
    // Told again (the relays give the history twice): over is over.
    w.replay(0, &history).await;
    w.settle().await;
    assert!(alice.calls.announced(GROUP).await.is_none());
    assert_eq!(alice.events(UI_EVENT_GROUP_CALL_ENDED).len(), 1);
}

/// The secret of epoch 1 of `call_id`, as the party's own start note carried it.
fn secret_of(p: &Party, call_id: &str) -> [u8; 32] {
    use base64::Engine as _;
    for o in p.sent.lock().unwrap().iter() {
        if let Some((_, _, e)) = FakeGroups::open_note(o) {
            if e.t == "call.start" && e.str_field("call_id") == Some(call_id) {
                return base64::engine::general_purpose::STANDARD.decode(e.str_field("secret").unwrap()).unwrap().try_into().unwrap();
            }
        }
    }
    panic!("no start of {call_id}");
}

#[tokio::test]
async fn the_way_to_the_node_is_judged_by_the_engine() {
    let w = World::new(&["alice"]).await;
    w.group(&[0]);
    let alice = w.p(0);
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    let s = w.engine.sessions()[0].clone();
    s.inject(SessionEvent::ConnectionState(ConnectionState::Disconnected));
    w.settle().await;
    assert_eq!(alice.last_state().unwrap().phase, GroupPhase::Reconnecting);
    s.inject(SessionEvent::ConnectionState(ConnectionState::Connected));
    w.settle().await;
    assert_eq!(alice.last_state().unwrap().phase, GroupPhase::InRoom);
    alice.calls.set_mute(true).await.unwrap();
    assert!(alice.last_state().is_some_and(|v| v.muted) || alice.calls.current().await.unwrap().muted);
    assert!(alice.calls.set_layer(2, "h").await.is_err(), "the fake node has no simulcast");
    s.inject(SessionEvent::ConnectionState(ConnectionState::Failed));
    w.settle().await;
    assert_eq!(alice.last_state().unwrap().phase, GroupPhase::Left);
    assert!(alice.notes().contains(&"call.end".to_string()), "alone in the room: the call is over");
    assert_eq!(repo::get(&alice.store, &alice.notes().len().to_string()).await.unwrap(), None);
    let ended = alice.events(UI_EVENT_GROUP_CALL_ENDED);
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0]["outcome"], "failed");
}

// ─── The findings of the review ──────────────────────────────────────────────

/// A start whose join the node refuses leaves nothing behind: no banner,
/// a failed call on record, and the group free for the next start.
#[tokio::test]
async fn a_start_the_node_refuses_to_join_is_a_failed_call_and_not_a_live_one() {
    let w = World::new(&["alice", "bob"]).await;
    w.group(&[0, 1]);
    let alice = w.p(0);
    w.node.refuse_next_join(NodeError::Refused { status: 409, error: "room_full".into(), message: "x".into() });
    let err = alice.calls.start(GROUP, Media::Audio).await.unwrap_err();
    assert!(err.to_string().contains("room_full"), "{err}");
    w.settle().await;
    assert!(alice.calls.current().await.is_none());
    assert!(alice.calls.announced(GROUP).await.is_none(), "nobody was told of it: no banner");
    assert!(alice.notes().is_empty());
    let ended = alice.events(UI_EVENT_GROUP_CALL_ENDED);
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0]["outcome"], "failed");
    let failed = ended[0]["call"]["call_id"].as_str().unwrap().to_string();
    assert_eq!(repo::get(&alice.store, &failed).await.unwrap().unwrap().outcome.as_deref(), Some("failed"));
    assert!(repo::open_group_calls(&alice.store, "group:00000000000000000000000000000000000000000000000000000000000000aa").await.unwrap().is_empty());

    // The next start goes through.
    let view = alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    assert_ne!(view.call_id, failed);
    assert_eq!(alice.last_state().unwrap().phase, GroupPhase::InRoom);
    assert_eq!(alice.calls.announced(GROUP).await.unwrap().call_id, view.call_id);
}

/// The connect timer judges the first way only: a loss after it is the
/// engine's to mend. A way that never comes is still a failure.
#[tokio::test]
async fn the_connect_timer_does_not_throw_out_a_participant_whose_way_was_there() {
    let w = World::new(&["alice"]).await;
    w.group(&[0]);
    w.timing(Timing { connect_timeout: Duration::from_millis(60), send_switch_delay: SWITCH, ..Timing::default() });
    let alice = w.p(0);
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    assert_eq!(alice.last_state().unwrap().phase, GroupPhase::InRoom);
    let s = w.engine.sessions()[0].clone();
    s.inject(SessionEvent::ConnectionState(ConnectionState::Disconnected));
    w.wait(Duration::from_millis(120)).await;
    assert_eq!(alice.last_state().unwrap().phase, GroupPhase::Reconnecting, "the timer was no judge of a way once there");
    assert!(alice.errors().is_empty(), "{:?}", alice.errors());
    alice.calls.leave().await.unwrap();
    w.settle().await;

    // No way at all: out after the time, as before.
    w.engine.set_connects(false);
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.wait(Duration::from_millis(120)).await;
    assert_eq!(alice.last_state().unwrap().phase, GroupPhase::Left);
    assert!(alice.errors().iter().any(|e| e.contains("no way to the node in time")), "{:?}", alice.errors());
}

/// A `call.join` names a seat the node spoke of; it makes none. One that
/// comes after the node's `left` (a short visit, the relays out of
/// order) raises no ghost: the oldest still rotates, the last still ends.
#[tokio::test]
async fn a_late_join_note_raises_no_ghost_seat() {
    let w = World::new(&["alice", "bob", "carol"]).await;
    w.group(&[0, 1, 2]);
    let (alice, bob, carol) = (w.p(0), w.p(1), w.p(2));
    let view = alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    // Alice and Carol hear nothing for a while: Bob's visit reaches them
    // only through the node (Alice) or not at all yet (Carol).
    w.deafen(0);
    w.deafen(2);
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    assert_eq!(verified_seats(&alice.last_state().unwrap()), vec![(2, true)]);
    bob.calls.leave().await.unwrap();
    w.wait(SWITCH * 3).await;
    assert_eq!(alice.count("call.epoch"), 1, "alice rotated on bob's leaving");
    assert_eq!(verified_seats(&alice.last_state().unwrap()), vec![]);
    // Now Bob's notes come: his join after his leave for Carol, in order
    // for Alice.
    w.hear(0).await;
    w.hear_reversed(2).await;
    w.settle().await;
    assert_eq!(verified_seats(&alice.last_state().unwrap()), vec![], "no seat is raised by a note");
    assert_eq!(
        carol.calls.announced(GROUP).await.unwrap().participants,
        vec![alice.pk().as_hex().to_string()],
        "a join that comes after its leave is stale"
    );
    assert_eq!(alice.calls.announced(GROUP).await.unwrap().participants, vec![alice.pk().as_hex().to_string()]);
    // Alice leaves last: the call ends for everybody.
    alice.calls.leave().await.unwrap();
    w.settle().await;
    assert!(alice.notes().contains(&"call.end".to_string()), "{:?}", alice.notes());
    for p in [alice, bob, carol] {
        assert!(p.calls.announced(GROUP).await.is_none(), "{}", p.name);
        assert_eq!(repo::get(&p.store, &view.call_id).await.unwrap().unwrap().outcome.as_deref(), Some("ended"), "{}", p.name);
    }
}

/// A seat is claimed once: a member's note for a seat another holds
/// changes nothing on the banner, and a word of identity is the last
/// word on who sits there.
#[tokio::test]
async fn a_claim_of_a_seat_another_holds_changes_nothing() {
    let w = World::new(&["alice", "bob", "carol"]).await;
    w.group(&[0, 1, 2]);
    let (alice, bob, carol) = (w.p(0), w.p(1), w.p(2));
    let view = alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    // Carol claims Alice's seat and leaves it again.
    for env in [Envelope::call_join(&view.call_id, 1), Envelope::call_leave(&view.call_id, 1)] {
        for p in [alice, bob] {
            p.calls.on_group_note(GROUP, &carol.pk(), &env, w.now()).await.unwrap();
        }
    }
    w.settle().await;
    for p in [alice, bob] {
        assert_eq!(p.calls.announced(GROUP).await.unwrap().participants, vec![alice.pk().as_hex().to_string()], "{}", p.name);
    }
    // Carol's claim came first for Bob's seat: the word of identity in the
    // room sets it right, and Alice's honest leave is honoured.
    w.deafen(0);
    alice.calls.on_group_note(GROUP, &carol.pk(), &Envelope::call_join(&view.call_id, 2), w.now()).await.unwrap();
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    assert_eq!(alice.calls.announced(GROUP).await.unwrap().participants, vec![alice.pk().as_hex().to_string(), bob.pk().as_hex().to_string()]);
    w.hear(0).await;
    w.settle().await;
    bob.calls.leave().await.unwrap();
    w.settle().await;
    assert_eq!(alice.calls.announced(GROUP).await.unwrap().participants, vec![alice.pk().as_hex().to_string()]);
}

/// Only verified seats count for who is the oldest: a seat of nobody
/// with a small number blocks no rotation.
#[tokio::test]
async fn a_seat_of_nobody_does_not_block_the_rotation() {
    let w = World::new(&["alice", "bob", "mallory"]).await;
    w.group(&[0, 1]);
    let (alice, bob, mallory) = (w.p(0), w.p(1), w.p(2));
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    // Mallory, removed from the group but holding the start, takes seat 2.
    mallory.groups.set_members(GROUP, vec![alice.pk(), bob.pk(), mallory.pk()]);
    let out = alice.sent.lock().unwrap().clone();
    for o in out {
        let (g, author, envelope) = FakeGroups::open_note(&o).unwrap();
        mallory.calls.on_group_note(&g, &author, &envelope, w.now()).await.unwrap();
    }
    mallory.calls.join(GROUP).await.unwrap();
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    assert_eq!(verified_seats(&bob.last_state().unwrap()), vec![(1, true), (2, false)]);
    // Alice leaves: Bob (seat 3) is the oldest verified seat and rotates.
    alice.calls.leave().await.unwrap();
    w.wait(SWITCH * 3).await;
    assert_eq!(bob.count("call.epoch"), 1, "{:?}", bob.notes());
    assert_eq!(bob.last_state().unwrap().epoch, 2);
    let sb = w.engine.sessions()[2].record();
    assert!(sb.receiver_keys.iter().all(|(mid, _, _)| mid != "a2"), "bob never keyed her m-line: {:?}", sb.receiver_keys);
}

/// A seat that says nothing within its time is nobody for good: the
/// creator puts it out and the keys turn; without the creator, the oldest
/// verified seat turns the keys around it.
#[tokio::test]
async fn a_seat_that_never_says_who_it_is_is_put_out_or_keyed_around() {
    let w = World::new(&["alice", "bob", "mallory"]).await;
    w.group(&[0, 1]);
    w.timing(Timing { verify_deadline: Duration::from_millis(80), send_switch_delay: SWITCH, ..Timing::default() });
    let (alice, bob, mallory) = (w.p(0), w.p(1), w.p(2));
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.join(GROUP).await.unwrap();
    w.wait(Duration::from_millis(120)).await;
    assert_eq!(alice.count("call.epoch"), 0, "members who said who they are cost no rotation");
    let room = w.node.rooms()[0].clone();
    mallory.groups.set_members(GROUP, vec![alice.pk(), bob.pk(), mallory.pk()]);
    let out = alice.sent.lock().unwrap().clone();
    for o in out {
        let (g, author, envelope) = FakeGroups::open_note(&o).unwrap();
        mallory.calls.on_group_note(&g, &author, &envelope, w.now()).await.unwrap();
    }
    mallory.calls.join(GROUP).await.unwrap();
    w.settle().await;
    assert_eq!(w.node.seats(&room).len(), 3);
    w.wait(Duration::from_millis(160)).await;
    assert_eq!(w.node.seats(&room).len(), 2, "the creator put the seat out");
    assert_eq!(mallory.last_state().unwrap().phase, GroupPhase::Left);
    assert_eq!(alice.count("call.epoch"), 1, "the keys turned as the seat left: {:?}", alice.notes());
    assert_eq!(verified_seats(&bob.last_state().unwrap()), vec![(1, true)]);

    // Without the creator: Alice leaves (Bob rotates), Mallory comes
    // again, and after her time Bob turns the keys around her.
    alice.calls.leave().await.unwrap();
    w.wait(SWITCH * 3).await;
    assert_eq!(bob.count("call.epoch"), 1);
    mallory.calls.join(GROUP).await.unwrap();
    w.wait(Duration::from_millis(160)).await;
    assert_eq!(w.node.seats(&room).len(), 2, "nobody can put her out");
    assert_eq!(bob.count("call.epoch"), 2, "{:?}", bob.notes());
    assert_eq!(verified_seats(&bob.last_state().unwrap()), vec![(4, false)]);
}

/// The group loses a member: its seat is nobody from then on (its keys
/// spoiled), the creator puts it out and changes the token, the keys
/// turn, and the banner loses it. One removed myself is out of the room.
#[tokio::test]
async fn a_member_removed_from_the_group_is_out_of_the_call() {
    let w = World::new(&["alice", "bob", "carol"]).await;
    w.group(&[0, 1, 2]);
    let (alice, bob, carol) = (w.p(0), w.p(1), w.p(2));
    let view = alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    carol.calls.join(GROUP).await.unwrap();
    w.settle().await;
    let room = w.node.rooms()[0].clone();
    let token_before = w.node.join_token(&room).unwrap();
    let carols_key = w.engine.sessions()[2].record().sender_keys[0].1.clone();
    assert_eq!(receiver_key(&w.engine.sessions()[1].record(), "a3", slot(1)), Some(carols_key.clone()));

    // Carol is removed: the groups tell Alice and Bob. The group's key
    // turned with her removal, so no note of theirs reaches her from now.
    w.deafen(2);
    for p in [alice, bob] {
        p.groups.set_members(GROUP, vec![alice.pk(), bob.pk()]);
        p.calls.on_members_changed(GROUP).await;
    }
    w.wait(SWITCH * 3).await;
    assert_eq!(w.node.seats(&room).len(), 2, "the creator put her out");
    assert_ne!(w.node.join_token(&room).unwrap(), token_before, "and changed the token");
    assert_eq!(carol.last_state().unwrap().phase, GroupPhase::Left);
    let epochs: Vec<Envelope> = alice.envelopes().into_iter().filter(|e| e.t == "call.epoch").collect();
    assert!(!epochs.is_empty(), "the keys turned: {:?}", alice.notes());
    assert_eq!(epochs[0].str_field("join_token"), w.node.join_token(&room).as_deref(), "the new token rides in the creator's note");
    let b = bob.last_state().unwrap();
    assert_eq!(b.epoch, alice.last_state().unwrap().epoch);
    assert!(b.epoch >= 2);
    assert!(b.participants.iter().all(|p| p.id != 3 || !p.verified), "{:?}", b.participants);
    let sb = w.engine.sessions()[1].record();
    assert_ne!(receiver_key(&sb, "a3", slot(1)), Some(carols_key), "her keys were spoiled before the node took her seat");
    for p in [alice, bob] {
        assert_eq!(p.calls.announced(GROUP).await.unwrap().participants, vec![alice.pk().as_hex().to_string(), bob.pk().as_hex().to_string()], "{}", p.name);
    }
    // Carol still holds the start note: the old token opens no door.
    let err = carol.calls.join(GROUP).await.unwrap_err();
    assert!(err.to_string().contains("bad_token"), "{err}");

    // Bob learns he is out himself: his room is left, the banner gone.
    bob.groups.set_members(GROUP, vec![alice.pk()]);
    bob.calls.on_members_changed(GROUP).await;
    w.settle().await;
    assert_eq!(bob.last_state().unwrap().phase, GroupPhase::Left);
    assert!(bob.calls.announced(GROUP).await.is_none());
    assert_eq!(repo::get(&bob.store, &view.call_id).await.unwrap().unwrap().outcome.as_deref(), Some("ended"));
    assert_eq!(w.node.seats(&room).len(), 1);
}

/// A word of identity under a secret not here yet is kept and opened
/// when the note of its epoch comes: the newcomer and the laggard both
/// end up verified by everybody.
#[tokio::test]
async fn a_word_of_identity_under_an_epoch_not_here_yet_waits_for_its_note() {
    let w = World::new(&["alice", "bob", "carol", "dave"]).await;
    w.group(&[0, 1, 2, 3]);
    let (alice, bob, carol, dave) = (w.p(0), w.p(1), w.p(2), w.p(3));
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    carol.calls.join(GROUP).await.unwrap();
    w.settle().await;
    // Dave hears nothing for a while; Bob leaves, Alice makes epoch 2.
    w.deafen(3);
    bob.calls.leave().await.unwrap();
    w.wait(SWITCH * 3).await;
    assert_eq!(alice.last_state().unwrap().epoch, 2);
    // Dave joins with epoch 1 alone: his word is under 1, theirs under 2.
    dave.calls.join(GROUP).await.unwrap();
    w.settle().await;
    assert_eq!(verified_seats(&alice.last_state().unwrap()), vec![(3, true), (4, true)], "dave's word under epoch 1 opens for them");
    assert_eq!(verified_seats(&dave.last_state().unwrap()), vec![(1, false), (3, false)], "their words wait for the note");
    w.hear(3).await;
    w.wait(SWITCH * 3).await;
    let d = dave.last_state().unwrap();
    assert_eq!(verified_seats(&d), vec![(1, true), (3, true)], "the note came: their words opened");
    assert_eq!(d.epoch, 2);
    let (sa, sd) = (w.engine.sessions()[0].record(), w.engine.sessions()[3].record());
    assert_eq!(receiver_key(&sd, "a1", slot(2)), sender_key_in(&sa, slot(2)), "dave hears alice on epoch 2");
    assert_eq!(receiver_key(&sa, "a4", slot(2)), sender_key_in(&sd, slot(2)), "and alice dave");

    // The mirror: Carol lags; a newcomer (Bob again, seat 5) comes with
    // the newest epoch.
    w.deafen(2);
    dave.calls.leave().await.unwrap();
    w.wait(SWITCH * 3).await;
    assert_eq!(alice.last_state().unwrap().epoch, 3);
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    assert_eq!(verified_seats(&carol.last_state().unwrap()), vec![(1, true), (5, false)], "a word under epoch 3 waits at carol");
    w.hear(2).await;
    w.wait(SWITCH * 3).await;
    assert_eq!(verified_seats(&carol.last_state().unwrap()), vec![(1, true), (5, true)]);
    assert_eq!(verified_seats(&bob.last_state().unwrap()), vec![(1, true), (3, true)]);
}

/// Two rotations that met on one number (the rotator left before its
/// note arrived; the next oldest rotated too) settle on one secret
/// everywhere: the smaller.
#[tokio::test]
async fn two_rotations_on_one_number_settle_on_the_smaller_secret() {
    let w = World::new(&["alice", "bob", "carol", "dave"]).await;
    w.group(&[0, 1, 2, 3]);
    let (alice, bob, carol, dave) = (w.p(0), w.p(1), w.p(2), w.p(3));
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    carol.calls.join(GROUP).await.unwrap();
    dave.calls.join(GROUP).await.unwrap();
    w.settle().await;
    // Carol and Dave hear no note: Alice leaves, Bob makes epoch 2; Bob
    // leaves before his note got through, Carol makes epoch 2 too.
    w.deafen(2);
    w.deafen(3);
    alice.calls.leave().await.unwrap();
    w.settle().await;
    assert_eq!(bob.count("call.epoch"), 1);
    bob.calls.leave().await.unwrap();
    w.settle().await;
    assert_eq!(carol.count("call.epoch"), 1, "{:?}", carol.notes());
    let secret = |p: &Party| -> Vec<u8> {
        use base64::Engine as _;
        let e = p.envelopes().into_iter().find(|e| e.t == "call.epoch").unwrap();
        assert_eq!(e.fields["epoch"], 2);
        base64::engine::general_purpose::STANDARD.decode(e.str_field("secret").unwrap()).unwrap()
    };
    assert_ne!(secret(bob), secret(carol), "two secrets for one number");
    // Dave hears Bob's first, Carol hers (her own copy) after Bob's.
    w.hear(3).await;
    w.hear(2).await;
    w.wait(SWITCH * 3).await;
    let (sc, sd) = (w.engine.sessions()[2].record(), w.engine.sessions()[3].record());
    assert_eq!(receiver_key(&sd, "a3", slot(2)), sender_key_in(&sc, slot(2)), "dave hears carol");
    assert_eq!(receiver_key(&sc, "a4", slot(2)), sender_key_in(&sd, slot(2)), "carol hears dave");
    assert_eq!(carol.last_state().unwrap().epoch, 2);
    assert_eq!(dave.last_state().unwrap().epoch, 2);
    let winner = secret(bob).min(secret(carol));
    let call_id = carol.last_state().unwrap().call_id;
    let expected: [u8; 32] = winner.try_into().unwrap();
    assert_eq!(sender_key_in(&sc, slot(2)), Some(sender_key(&expected, &call_id, 3, 2)), "the smaller secret holds");
    assert_eq!(verified_seats(&carol.last_state().unwrap()), vec![(4, true)]);
    assert_eq!(verified_seats(&dave.last_state().unwrap()), vec![(3, true)]);
}

/// The keys of a new epoch are set for receiving at once and the sending
/// moves to it later, so that nobody misses the rotator's frames while
/// the note travels.
#[tokio::test]
async fn the_sending_moves_to_a_new_epoch_after_the_others_had_time_to_read_it() {
    let w = World::new(&["alice", "bob", "carol"]).await;
    w.group(&[0, 1, 2]);
    w.timing(Timing { send_switch_delay: Duration::from_millis(250), ..Timing::default() });
    let (alice, bob, carol) = (w.p(0), w.p(1), w.p(2));
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    carol.calls.join(GROUP).await.unwrap();
    w.settle().await;
    bob.calls.leave().await.unwrap();
    w.settle().await;
    assert_eq!(alice.count("call.epoch"), 1);
    let (sa, sc) = (w.engine.sessions()[0].record(), w.engine.sessions()[2].record());
    assert_eq!(sa.sender_keys.last().unwrap().0, slot(1), "alice still sends on epoch 1");
    assert_eq!(alice.last_state().unwrap().epoch, 1);
    assert!(receiver_key(&sc, "a1", slot(2)).is_some(), "carol can already hear epoch 2");
    assert!(receiver_key(&sa, "a3", slot(2)).is_some());
    w.wait(Duration::from_millis(350)).await;
    let (sa, sc) = (w.engine.sessions()[0].record(), w.engine.sessions()[2].record());
    assert_eq!(sa.sender_keys.last().unwrap().0, slot(2));
    assert_eq!(sc.sender_keys.last().unwrap().0, slot(2));
    assert_eq!((alice.last_state().unwrap().epoch, carol.last_state().unwrap().epoch), (2, 2));
}

/// A call whose end never comes is closed on record when its room has
/// expired: as of when I last left it, or when the room ended. Stale
/// records of calls from before are closed on login.
#[tokio::test]
async fn a_call_without_an_end_is_closed_when_its_room_expires() {
    let w = World::new(&["alice", "bob", "carol"]).await;
    w.group(&[0, 1, 2]);
    let (alice, bob, carol) = (w.p(0), w.p(1), w.p(2));
    // The node's rooms end two seconds from now (the clock of the tests).
    w.node.set_now((START - 12 * 3600 + 2) as u64);
    let view = alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    let room = w.node.rooms()[0].clone();
    // The node closes everybody at once: nobody says `call.end`.
    w.clock.0.store(START + 1, Ordering::SeqCst);
    w.node.end_room(&room);
    w.settle().await;
    assert!(alice.notes().iter().all(|t| t != "call.end"));
    assert!(carol.calls.announced(GROUP).await.is_some());
    w.clock.0.store(START + 3, Ordering::SeqCst);
    w.wait(Duration::from_millis(2400)).await;
    for p in [alice, bob, carol] {
        assert!(p.calls.announced(GROUP).await.is_none(), "{}", p.name);
        let row = repo::get(&p.store, &view.call_id).await.unwrap().unwrap();
        assert_eq!(row.outcome.as_deref(), Some("ended"), "{}", p.name);
        let expected = if p.name == "carol" { START + 2 } else { START + 1 };
        assert_eq!(row.ended_at, Some(expected), "{}: when I last left it, or when the room ended", p.name);
        assert_eq!(p.events(UI_EVENT_GROUP_CALL_ENDED).len(), 1, "{}", p.name);
    }

    // A record from before, never closed: closed on login as of its expiry.
    let old = repo::NewCall {
        call_id: "ab".repeat(16),
        chat_id: "group:00000000000000000000000000000000000000000000000000000000000000aa".into(),
        peer: GROUP.into(),
        direction: "in".into(),
        media: "audio".into(),
        started_at: START - 13 * 3600,
    };
    repo::insert_group(&alice.store, &old, bob.pk().as_hex()).await.unwrap();
    alice.calls.set_signer(Some(alice.keys.clone()));
    w.settle().await;
    let row = repo::get(&alice.store, &old.call_id).await.unwrap().unwrap();
    assert_eq!((row.outcome.as_deref(), row.ended_at), (Some("ended"), Some(START - 3600)));
}

/// Two members start within moments of each other: the newer start holds,
/// the older never was, and its creator goes over to the room that holds.
#[tokio::test]
async fn two_starts_of_one_moment_are_one_call() {
    let w = World::new(&["alice", "bob", "carol"]).await;
    w.group(&[0, 1, 2]);
    let (alice, bob, carol) = (w.p(0), w.p(1), w.p(2));
    w.deafen(0);
    w.deafen(1);
    w.deafen(2);
    let older = alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    w.clock.0.fetch_add(1, Ordering::SeqCst);
    let newer = bob.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    assert_eq!(w.node.rooms().len(), 2);
    // The notes arrive, Alice's first for everybody.
    for i in [2, 1, 0] {
        w.hear(i).await;
    }
    w.wait(Duration::from_millis(50)).await;
    for p in [alice, bob, carol] {
        let banner = p.calls.announced(GROUP).await.unwrap();
        assert_eq!(banner.call_id, newer.call_id, "{}", p.name);
        assert!(repo::get(&p.store, &older.call_id).await.unwrap().is_none(), "{}: the older never was", p.name);
        let dropped = p.events(UI_EVENT_GROUP_CALL_ENDED).iter().filter(|e| e["call"]["call_id"] == older.call_id).count();
        assert_eq!(dropped, usize::from(p.name != "bob"), "{}: bob never had the older on his banner", p.name);
    }
    assert_eq!(alice.calls.current().await.unwrap().call_id, newer.call_id, "alice went over");
    let (a, b) = (alice.last_state().unwrap(), bob.last_state().unwrap());
    assert_eq!(a.phase, GroupPhase::InRoom);
    assert_eq!(verified_seats(&a), vec![(1, true)]);
    assert_eq!(verified_seats(&b), vec![(2, true)]);
    let rooms = w.node.rooms();
    let seats: Vec<usize> = rooms.iter().map(|r| w.node.seats(r).len()).collect();
    assert_eq!(seats.iter().sum::<usize>(), 2);
    assert!(seats.contains(&0), "the older room is empty");
    assert_eq!(carol.calls.announced(GROUP).await.unwrap().participants.len(), 2);
    for p in [alice, bob, carol] {
        assert!(p.errors().is_empty(), "{}: {:?}", p.name, p.errors());
    }
}

/// The losing half of a glare (two starts of one moment) is forgotten
/// without a `call.end` and without a record. After the winner has
/// ended, a restart hears the history in any order: the loser's start,
/// told of after the winner's (over by now) or before it, is no banner
/// for a room nobody sits in, and no record.
#[tokio::test]
async fn the_loser_of_a_glare_is_no_ghost_after_a_restart() {
    let mut w = World::new(&["alice", "bob", "carol"]).await;
    w.group(&[0, 1, 2]);
    let (alice, bob, carol) = (w.p(0), w.p(1), w.p(2));
    w.deafen(0);
    w.deafen(1);
    w.deafen(2);
    let older = alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    w.clock.0.fetch_add(1, Ordering::SeqCst);
    let newer = bob.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    for i in [2, 1, 0] {
        w.hear(i).await;
    }
    w.wait(Duration::from_millis(50)).await;
    assert_eq!(carol.calls.announced(GROUP).await.unwrap().call_id, newer.call_id);
    // The winner ends: everybody out.
    w.clock.0.fetch_add(30, Ordering::SeqCst);
    alice.calls.leave().await.unwrap();
    w.wait(SWITCH * 3).await;
    bob.calls.leave().await.unwrap();
    w.settle().await;
    for p in [alice, bob, carol] {
        assert!(p.calls.announced(GROUP).await.is_none(), "{}", p.name);
    }
    let history = w.history();
    let mut oldest_first = history.clone();
    oldest_first.reverse();
    w.clock.0.fetch_add(600, Ordering::SeqCst);
    for (order, notes) in [("newest first", &history), ("oldest first", &oldest_first)] {
        w.restart(2).await;
        w.replay(2, notes).await;
        w.settle().await;
        let carol = w.p(2);
        let banner = carol.calls.announced(GROUP).await;
        assert!(banner.is_none(), "carol, {order}: a ghost banner of the loser {}: {banner:?}", older.call_id);
        assert!(repo::get(&carol.store, &older.call_id).await.unwrap().is_none(), "carol, {order}: the loser got a record again");
        assert_eq!(repo::get(&carol.store, &newer.call_id).await.unwrap().unwrap().outcome.as_deref(), Some("ended"), "{order}");
        // Newest first, no banner was raised at all; oldest first, the
        // loser's was raised (its start, then its join) and lowered once
        // by the winner's start, as live.
        let (raised, lowered) = (carol.events(UI_EVENT_GROUP_CALL_STARTED).len(), carol.events(UI_EVENT_GROUP_CALL_ENDED).len());
        assert_eq!((raised > 0, lowered), (order == "oldest first", usize::from(order == "oldest first")), "carol, {order}: {raised} raised");
        assert!(carol.errors().is_empty(), "carol, {order}: {:?}", carol.errors());
    }
}

/// My other device is in the room as anybody: its `call.join` seats it
/// on my banner and counts it (one person with me on the record). Only
/// this device's own word of a seat it took and sits in no more is
/// stale: after the PC died in the room, the history tells of its own
/// join (no leave) and of the phone's; the PC's seat is nobody's, the
/// phone's is mine.
#[tokio::test]
async fn my_other_device_is_on_the_banner_and_my_own_old_seat_is_not() {
    let mut w = World::new(&["alice", "bob"]).await;
    let phone = w.twin(0, "alice-phone").await;
    w.group(&[0, 1, phone]);
    let (alice, bob, phone) = (w.p(0).clone(), w.p(1).clone(), w.p(phone).clone());
    assert_eq!(alice.pk(), phone.pk());
    let me = alice.pk().as_hex().to_string();
    let view = alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    phone.calls.join(GROUP).await.unwrap();
    w.settle().await;
    assert_eq!(phone.last_state().unwrap().phase, GroupPhase::InRoom);
    for p in [&alice, &bob, &phone] {
        let banner = p.calls.announced(GROUP).await.unwrap();
        assert_eq!(banner.participants.len(), 3, "{}: {:?}", p.name, banner.participants);
        assert_eq!(banner.participants.iter().filter(|p| **p == me).count(), 2, "{}: both of my devices", p.name);
        assert_eq!(repo::group_info(&p.store, &view.call_id).await.unwrap().unwrap().participants, 2, "{}: two people", p.name);
    }
    // The PC dies in the room (no leave of its seat 1) and comes back:
    // the history tells of its own join and of the phone's.
    let history = w.history();
    assert_eq!(history.iter().filter(|(a, e, _)| *a == alice.pk() && e.t == "call.join").count(), 2, "the PC's and the phone's joins");
    w.restart(0).await;
    w.replay(0, &history).await;
    w.settle().await;
    let alice = w.p(0).clone();
    let banner = alice.calls.announced(GROUP).await.unwrap();
    assert_eq!(banner.participants.len(), 2, "bob and my phone, not my seat of before: {:?}", banner.participants);
    assert_eq!(banner.participants.iter().filter(|p| **p == me).count(), 1, "the phone is on the banner");
    assert!(!banner.joined);
    assert_eq!(repo::group_info(&alice.store, &view.call_id).await.unwrap().unwrap().participants, 2, "two people still");
    // The phone leaves: its seat goes too.
    phone.calls.leave().await.unwrap();
    w.wait(SWITCH * 3).await;
    let banner = alice.calls.announced(GROUP).await.unwrap();
    assert_eq!(banner.participants, vec![bob.pk().as_hex().to_string()], "{:?}", banner.participants);
    for p in [&alice, &bob, &phone] {
        assert!(p.errors().is_empty(), "{}: {:?}", p.name, p.errors());
    }
}

/// A start of the group that came while my node was asked is settled
/// as on every other device: by when each start was said. Bob's is
/// newer (his clock runs ahead, so his start is said after mine will
/// be): my room never was, the group never hears of it, and I join his.
#[tokio::test]
async fn a_newer_start_heard_while_the_node_was_asked_wins_over_mine() {
    let w = World::new(&["alice", "bob", "carol"]).await;
    w.group(&[0, 1, 2]);
    let (alice, bob, carol) = (w.p(0), w.p(1), w.p(2));
    let gate = w.node.hold_next_create();
    let starting = {
        let alice = alice.clone();
        tokio::spawn(async move { alice.calls.start(GROUP, Media::Audio).await })
    };
    w.settle().await;
    assert_eq!(alice.last_state().unwrap().phase, GroupPhase::Starting);
    // Bob's clock is five seconds ahead: his start is said at START + 5.
    w.clock.0.store(START + 5, Ordering::SeqCst);
    let bobs = bob.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    assert_eq!(alice.calls.announced(GROUP).await.unwrap().call_id, bobs.call_id, "alice heard of bob's while waiting");
    // Alice's node answers; her clock says START + 1.
    w.clock.0.store(START + 1, Ordering::SeqCst);
    gate.notify_one();
    let view = starting.await.unwrap().unwrap();
    w.settle().await;
    assert_eq!(view.call_id, bobs.call_id, "start() ends in bob's room");
    assert_eq!(alice.notes(), vec!["call.join"], "the group never heard of alice's own call");
    let current = alice.calls.current().await.unwrap();
    assert_eq!((current.call_id.as_str(), current.phase), (bobs.call_id.as_str(), GroupPhase::InRoom));
    assert_eq!(verified_seats(&alice.last_state().unwrap()), vec![(1, true)], "alice sees bob");
    assert_eq!(verified_seats(&bob.last_state().unwrap()), vec![(2, true)], "bob sees alice");
    let dropped: Vec<serde_json::Value> = alice.events(UI_EVENT_GROUP_CALL_ENDED);
    assert_eq!(dropped.len(), 1);
    let mine = dropped[0]["call"]["call_id"].as_str().unwrap().to_string();
    assert_ne!(mine, bobs.call_id);
    assert!(repo::get(&alice.store, &mine).await.unwrap().is_none(), "alice's own call never was");
    let rooms = w.node.rooms();
    assert_eq!(rooms.len(), 2);
    let seats: Vec<usize> = rooms.iter().map(|r| w.node.seats(r).len()).collect();
    assert_eq!(seats.iter().sum::<usize>(), 2);
    assert!(seats.contains(&0), "alice's room is empty");
    for p in [alice, bob, carol] {
        let banner = p.calls.announced(GROUP).await.unwrap();
        assert_eq!(banner.call_id, bobs.call_id, "{}", p.name);
        assert_eq!(banner.participants.len(), 2, "{}: {:?}", p.name, banner.participants);
        assert!(p.errors().is_empty(), "{}: {:?}", p.name, p.errors());
    }
}

// ─── Probes of 2026-10-08 (Android ↔ Windows, both seats unconfirmed) ────────
//
// The fake node of before opened the newcomer's `ctl` inside its join and
// relayed every frame to every seat; the real node says `joined` to the
// others first, the newcomer's channel opens later, and a frame to a
// closed channel is dropped (`set_real_ctl_order`). Each probe is one
// hypothesis of track B.

/// Two devices of one npub (the owner's phone and PC) in the real order
/// of the node: the creator's word on `joined` is lost to the closed
/// channel of the newcomer; the newcomer's own word, once its channel is
/// open, is answered, and both are confirmed. The hello path itself holds
/// for twins.
#[tokio::test]
async fn probe_twins_confirm_each_other_in_the_real_order_of_the_node() {
    let mut w = World::new(&["phone"]).await;
    let pc = w.twin(0, "pc").await;
    w.group(&[0, pc]);
    let (phone, pc) = (w.p(0).clone(), w.p(pc).clone());
    phone.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    w.node.set_real_ctl_order(true);
    let view = pc.calls.join(GROUP).await.unwrap();
    w.settle().await;
    let pc_seat = view.participant.unwrap();
    // The way is there (the fake answer connects), the channel is not:
    // nobody is confirmed yet, on either side.
    assert_eq!(pc.last_state().unwrap().phase, GroupPhase::InRoom);
    assert_eq!(verified_seats(&pc.last_state().unwrap()), vec![(1, false)], "the creator's word on `joined` never reached the closed channel");
    assert_eq!(verified_seats(&phone.last_state().unwrap()), vec![(pc_seat, false)]);
    let room = w.node.rooms()[0].clone();
    w.node.open_ctl(&room, pc_seat);
    w.settle().await;
    assert_eq!(verified_seats(&pc.last_state().unwrap()), vec![(1, true)], "the PC's word was answered");
    assert_eq!(verified_seats(&phone.last_state().unwrap()), vec![(pc_seat, true)]);
    for p in [&phone, &pc] {
        assert!(p.errors().is_empty(), "{}: {:?}", p.name, p.errors());
    }
}

/// The Windows screen of the owner: the join over HTTP gives a seat and
/// the others' seats (`participants`), the way to the node never comes.
/// The phase stays `joining`, no word of identity goes either way, the
/// creator shows the seat unconfirmed, and nothing says why until the
/// connect timer gives up with `failed` and frees the seat.
#[tokio::test]
async fn probe_a_joiner_whose_way_never_comes_is_the_windows_screen() {
    let mut w = World::new(&["phone"]).await;
    let pc = w.twin(0, "pc").await;
    w.group(&[0, pc]);
    let (phone, pc) = (w.p(0).clone(), w.p(pc).clone());
    phone.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    w.timing(Timing { connect_timeout: Duration::from_millis(120), verify_deadline: Duration::from_secs(30), send_switch_delay: SWITCH, ..Timing::default() });
    w.node.set_real_ctl_order(true);
    w.engine.set_connects(false);
    let view = pc.calls.join(GROUP).await.unwrap();
    w.settle().await;
    assert_eq!(view.phase, GroupPhase::Joining);
    assert_eq!(view.participants.len(), 2, "me and the creator, from the HTTP join");
    assert_eq!(pc.last_state().unwrap().phase, GroupPhase::Joining);
    assert_eq!(verified_seats(&pc.last_state().unwrap()), vec![(1, false)]);
    assert_eq!(verified_seats(&phone.last_state().unwrap()), vec![(2, false)], "the creator sees a seat without a word");
    assert!(pc.errors().is_empty(), "nothing on the screen says why: {:?}", pc.errors());
    w.wait(Duration::from_millis(250)).await;
    assert_eq!(pc.last_state().unwrap().phase, GroupPhase::Left);
    assert!(pc.events(UI_EVENT_GROUP_CALL_ENDED).is_empty(), "a joiner's leave ends nothing: the call goes on for the creator");
    assert!(phone.calls.announced(GROUP).await.is_some_and(|a| a.joined), "the creator is still in");
    assert!(pc.notes().contains(&"call.leave".to_string()));
    assert!(verified_seats(&phone.last_state().unwrap()).is_empty(), "the node said `left`");
    assert!(pc.errors().iter().any(|e| e.contains("no way to the node in time")), "{:?}", pc.errors());
    w.engine.set_connects(true);
}

/// The third seat of the screenshot: a seat that never connected (an
/// earlier attempt) sits in the room when the PC joins. The creator puts
/// it out at the deadline and the epoch turns; the pair confirms each
/// other all the same, under the new epoch too.
#[tokio::test]
async fn probe_a_stale_seat_at_join_time_is_put_out_and_the_pair_still_confirms() {
    let mut w = World::new(&["phone", "ghost"]).await;
    let pc = w.twin(0, "pc").await;
    w.group(&[0, 1, pc]);
    let (phone, ghost, pc) = (w.p(0).clone(), w.p(1).clone(), w.p(pc).clone());
    phone.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    w.timing(Timing { connect_timeout: Duration::from_secs(30), verify_deadline: Duration::from_millis(150), send_switch_delay: SWITCH, ..Timing::default() });
    w.node.set_real_ctl_order(true);
    w.engine.set_connects(false);
    ghost.calls.join(GROUP).await.unwrap();
    w.settle().await;
    w.engine.set_connects(true);
    let view = pc.calls.join(GROUP).await.unwrap();
    w.settle().await;
    assert_eq!(view.participants.len(), 3, "me, the creator and the stale seat");
    let pc_seat = view.participant.unwrap();
    let room = w.node.rooms()[0].clone();
    w.node.open_ctl(&room, pc_seat);
    w.wait(Duration::from_millis(400)).await;
    // The creator's `call.epoch` is carried in the settle above; the PC
    // moves its sending (and its `epoch` on the screen) `SWITCH` later.
    w.wait(SWITCH * 3).await;
    let phone_view = phone.last_state().unwrap();
    let pc_view = pc.last_state().unwrap();
    assert_eq!(phone_view.phase, GroupPhase::InRoom);
    assert_eq!(pc_view.phase, GroupPhase::InRoom);
    assert_eq!(verified_seats(&phone_view), vec![(pc_seat, true)], "the ghost was put out, the PC confirmed: {phone_view:?}");
    assert_eq!(verified_seats(&pc_view), vec![(1, true)], "{pc_view:?}");
    assert_eq!(phone_view.epoch, 2, "the keys turned after the ghost left");
    assert_eq!(pc_view.epoch, 2);
    assert!(phone.count("call.epoch") >= 1);
    for p in [&phone, &pc] {
        assert!(p.errors().is_empty(), "{}: {:?}", p.name, p.errors());
    }
}

/// A word of identity from an npub that is not in my member list is
/// dropped without a word: the seat stays unconfirmed, no error, nothing
/// in the log. (Two devices of one npub never meet this: `me` is always
/// a member; two identities with diverging group logs do.)
#[tokio::test]
async fn probe_a_word_from_outside_my_member_list_is_dropped_in_silence() {
    let w = World::new(&["alice", "bob"]).await;
    w.group(&[0, 1]);
    let (alice, bob) = (w.p(0).clone(), w.p(1).clone());
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    // Bob's log of the group does not know Alice as a member.
    bob.groups.set_members(GROUP, vec![bob.pk()]);
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    assert_eq!(verified_seats(&alice.last_state().unwrap()), vec![(2, true)], "Alice knows Bob");
    assert_eq!(verified_seats(&bob.last_state().unwrap()), vec![(1, false)], "Bob does not know Alice: unconfirmed for good");
    assert!(bob.errors().is_empty(), "and nothing says so: {:?}", bob.errors());
}

/// A joiner whose way never came judges none of the seats it saw in the
/// HTTP join: deaf (its channel never opened), it would count itself the
/// oldest and turn the keys once per seat without a word (two
/// `call.epoch` from a seat that was never in the room, as it did before
/// 2026-10-09). The connect timer judges it instead.
#[tokio::test]
async fn a_joiner_without_a_way_turns_no_keys_at_the_deadline() {
    let mut w = World::new(&["phone", "ghost"]).await;
    let pc = w.twin(0, "pc").await;
    w.group(&[0, 1, pc]);
    let (phone, ghost, pc) = (w.p(0).clone(), w.p(1).clone(), w.p(pc).clone());
    phone.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    w.node.set_real_ctl_order(true);
    w.engine.set_connects(false);
    // The creator's deadline is long: it does not put anybody out here.
    phone.calls.set_timing(Timing { connect_timeout: Duration::from_secs(30), verify_deadline: Duration::from_secs(30), send_switch_delay: SWITCH, ..Timing::default() });
    ghost.calls.set_timing(Timing { connect_timeout: Duration::from_secs(30), verify_deadline: Duration::from_secs(30), send_switch_delay: SWITCH, ..Timing::default() });
    ghost.calls.join(GROUP).await.unwrap();
    w.settle().await;
    pc.calls.set_timing(Timing { connect_timeout: Duration::from_secs(30), verify_deadline: Duration::from_millis(100), send_switch_delay: SWITCH, ..Timing::default() });
    let view = pc.calls.join(GROUP).await.unwrap();
    assert_eq!(view.phase, GroupPhase::Joining);
    assert_eq!(view.participants.len(), 3);
    w.wait(Duration::from_millis(300)).await;
    assert_eq!(pc.last_state().unwrap().phase, GroupPhase::Joining, "still no way");
    assert_eq!(pc.count("call.epoch"), 0, "deaf, the PC judged nobody: {:?}", pc.notes());
    assert!(pc.errors().is_empty(), "and said nothing in vain: {:?}", pc.errors());
    w.engine.set_connects(true);
}

// ─── The word of identity, said again (2026-10-09) ───────────────────────────
//
// What the node of 2026-10-08 showed: a word of identity is lost on the
// way (the channel of the newcomer stalled for fifteen seconds; a frame
// to a channel that is not open is dropped), and a word lost was a seat
// unconfirmed for good on one side, then put out by the creator. The word
// is said again until everybody is confirmed, every word is answered, and
// a seat is judged from when I could hear it.

/// Bob's first word, on the opening of his channel, is lost on the way;
/// the one he says again confirms him, and Alice's answer confirms her.
#[tokio::test]
async fn a_word_lost_on_the_way_is_said_again_until_the_seat_is_confirmed() {
    let w = World::new(&["alice", "bob"]).await;
    w.group(&[0, 1]);
    w.timing(Timing { hello_retry: Duration::from_millis(40), verify_deadline: Duration::from_secs(30), send_switch_delay: SWITCH, ..Timing::default() });
    let (alice, bob) = (w.p(0), w.p(1));
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    w.node.set_real_ctl_order(true);
    // Alice's word on `joined` goes to Bob's closed channel; Bob's first
    // word, on the opening of his channel, is lost on the way.
    w.node.drop_relayed(2, 0, 1);
    let view = bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    let seat = view.participant.unwrap();
    let room = w.node.rooms()[0].clone();
    w.node.open_ctl(&room, seat);
    // Bob's first word is lost (the drop above); the one he says again
    // `hello_retry` later comes within the wait.
    w.wait(Duration::from_millis(120)).await;
    assert_eq!(verified_seats(&alice.last_state().unwrap()), vec![(seat, true)], "Bob said it again");
    assert_eq!(verified_seats(&bob.last_state().unwrap()), vec![(1, true)], "and Alice answered");
    assert_eq!(alice.count("call.epoch") + bob.count("call.epoch"), 0, "nobody turned the keys: {:?} {:?}", alice.notes(), bob.notes());
    for p in [alice, bob] {
        assert!(p.errors().is_empty(), "{}: {:?}", p.name, p.errors());
    }
}

/// Alice's answer to Bob's word is lost: Bob says his word again (he has
/// not confirmed her), and Alice, who has confirmed him already, answers
/// it all the same.
#[tokio::test]
async fn every_word_is_answered_so_that_a_lost_answer_is_given_again() {
    let w = World::new(&["alice", "bob"]).await;
    w.group(&[0, 1]);
    w.timing(Timing { hello_retry: Duration::from_millis(40), verify_deadline: Duration::from_secs(30), send_switch_delay: SWITCH, ..Timing::default() });
    let (alice, bob) = (w.p(0), w.p(1));
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    w.node.set_real_ctl_order(true);
    // Alice's word on `joined` goes to nobody; her first word with
    // somebody to hear it, the answer to Bob's word, is lost.
    w.node.drop_relayed(1, 0, 1);
    let view = bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    let seat = view.participant.unwrap();
    let room = w.node.rooms()[0].clone();
    w.node.open_ctl(&room, seat);
    w.settle().await;
    assert_eq!(verified_seats(&alice.last_state().unwrap()), vec![(seat, true)], "Alice took Bob's word");
    assert_eq!(verified_seats(&bob.last_state().unwrap()), vec![(1, false)], "her answer was lost");
    w.wait(Duration::from_millis(120)).await;
    assert_eq!(verified_seats(&bob.last_state().unwrap()), vec![(1, true)], "Bob said it again and was answered again");
    for p in [alice, bob] {
        assert!(p.errors().is_empty(), "{}: {:?}", p.name, p.errors());
    }
}

/// A seat seen in the join, before my channel opened, has its time from
/// the opening: deaf, I would judge a seat whose word I could not have
/// heard. Bob's channel opens long after his deadline would have passed;
/// he turns no keys, puts nobody out, and the pair confirms each other.
#[tokio::test]
async fn a_seat_seen_before_my_channel_opened_has_its_time_from_the_opening() {
    let w = World::new(&["alice", "bob"]).await;
    w.group(&[0, 1]);
    let (alice, bob) = (w.p(0), w.p(1));
    alice.calls.set_timing(Timing { verify_deadline: Duration::from_secs(30), send_switch_delay: SWITCH, ..Timing::default() });
    bob.calls.set_timing(Timing { verify_deadline: Duration::from_millis(60), hello_retry: Duration::from_millis(40), send_switch_delay: SWITCH, ..Timing::default() });
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    w.node.set_real_ctl_order(true);
    let view = bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    let seat = view.participant.unwrap();
    assert_eq!(verified_seats(&bob.last_state().unwrap()), vec![(1, false)], "Alice's seat, from the join");
    w.wait(Duration::from_millis(200)).await;
    assert_eq!(bob.count("call.epoch"), 0, "deaf, Bob judged nobody: {:?}", bob.notes());
    assert_eq!(bob.last_state().unwrap().epoch, 1);
    assert!(bob.errors().is_empty(), "nothing was said in vain: {:?}", bob.errors());
    let room = w.node.rooms()[0].clone();
    w.node.open_ctl(&room, seat);
    w.wait(Duration::from_millis(40)).await;
    assert_eq!(verified_seats(&bob.last_state().unwrap()), vec![(1, true)]);
    assert_eq!(verified_seats(&alice.last_state().unwrap()), vec![(seat, true)]);
    w.wait(Duration::from_millis(150)).await;
    assert_eq!(bob.count("call.epoch"), 0, "confirmed in time, from the opening: {:?}", bob.notes());
}

/// With the channel open, a seat that says nothing in its time is judged
/// as before: the creator puts it out.
#[tokio::test]
async fn a_seat_that_says_nothing_after_my_channel_opened_is_put_out_in_its_time() {
    let w = World::new(&["alice", "bob"]).await;
    w.group(&[0, 1]);
    w.timing(Timing { verify_deadline: Duration::from_millis(80), hello_retry: Duration::from_millis(30), send_switch_delay: SWITCH, ..Timing::default() });
    let (alice, bob) = (w.p(0), w.p(1));
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    w.node.set_real_ctl_order(true);
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    let room = w.node.rooms()[0].clone();
    assert_eq!(w.node.seats(&room).len(), 2);
    // Bob's channel never opens: his word never comes.
    w.wait(Duration::from_millis(160)).await;
    assert_eq!(w.node.seats(&room).len(), 1, "the creator put the seat out");
    assert_eq!(bob.last_state().unwrap().phase, GroupPhase::Left);
    assert_eq!(bob.count("call.epoch"), 0, "and the seat, deaf, turned no keys: {:?}", bob.notes());
}
