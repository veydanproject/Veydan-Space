// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Group calls between cores on the fake engine and the fake node: the
//! notes of the group carried by hand to every member (the fake groups
//! of these tests), the control channel spoken by the fake node. A party
//! may be made deaf for a while: the notes to it are held and handed
//! over later, in order or reversed, as the relays would.

use messenger_calls::engine::ConnectionState;
use messenger_calls::group::ctl::Message;
use messenger_calls::group::keys::{sender_key, slot};
use messenger_calls::group::service::Timing;
use messenger_calls::{
    CallNode, DataPayload, GroupAccess, GroupCallService, GroupCallView, GroupPhase, Media, MediaLimits, NodeClass, NodeError, RoomApi,
    SessionEvent, StaticServerSets, CTL_LABEL, UI_EVENT_GROUP_CALL_ENDED, UI_EVENT_GROUP_CALL_STARTED, UI_EVENT_GROUP_CALL_STATE,
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
        let servers = vec![node.as_call_node()];
        Self::over(name, keys, store, clock, engine, node, servers).await
    }

    /// The party over `store` with `keys`: a fresh core, as after a
    /// restart of the app (nothing of the old one is in memory; the
    /// record is). `servers` is its sets of servers (the cascade tests
    /// give it an own node of its own).
    async fn over(
        name: &'static str,
        keys: Keys,
        store: Store,
        clock: Arc<TestClock>,
        engine: FakeEngine,
        node: FakeNode,
        servers: Vec<messenger_calls::CallNode>,
    ) -> Self {
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
            Arc::new(StaticServerSets(servers)),
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
        let servers = vec![self.node.as_call_node()];
        let fresh = Party::over(old.name, old.keys.clone(), old.store.clone(), self.clock.clone(), self.engine.clone(), self.node.clone(), servers).await;
        fresh.groups.set_members(GROUP, members);
        self.parties[i] = Arc::new(fresh);
    }

    /// Another device of the party: the same keys over a record of its
    /// own, a member wherever the party is. Its index.
    async fn twin(&mut self, i: usize, name: &'static str) -> usize {
        let old = self.parties[i].clone();
        let members: Vec<PubKey> = old.groups.members(GROUP).await.unwrap_or_default();
        let store = Store::open_in_memory().await.unwrap();
        let servers = vec![self.node.as_call_node()];
        let fresh = Party::over(name, old.keys.clone(), store, self.clock.clone(), self.engine.clone(), self.node.clone(), servers).await;
        fresh.groups.set_members(GROUP, members);
        self.parties.push(Arc::new(fresh));
        self.parties.len() - 1
    }

    /// Rebuild a party with the sets of servers given (for the cascade and
    /// move tests: an own node of its own, more than one node to move to),
    /// keeping its record, keys and group membership. The timing passed is
    /// kept too.
    async fn with_servers(&mut self, i: usize, servers: Vec<messenger_calls::CallNode>, timing: Timing) {
        let old = self.parties[i].clone();
        let members: Vec<PubKey> = old.groups.members(GROUP).await.unwrap_or_default();
        let pinned = old.groups.pinned_node(GROUP).await.ok().flatten();
        let fresh =
            Party::over(old.name, old.keys.clone(), old.store.clone(), self.clock.clone(), self.engine.clone(), self.node.clone(), servers).await;
        fresh.groups.set_members(GROUP, members);
        if let Some(node) = pinned {
            fresh.groups.pin(GROUP, node);
        }
        fresh.calls.set_timing(timing);
        self.parties[i] = Arc::new(fresh);
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

    /// Carry everything until nothing moves for a few passes. An event
    /// shown again unchanged (the same name, the same payload as the last
    /// of that name) is no movement: verified seats answer each other's
    /// words of identity a quarter of `hello_retry` apart, and under the
    /// load of a whole suite that gap always elapses, so the states they
    /// show again would never let the world settle.
    async fn settle(&self) {
        let mut quiet = 0;
        // What kept moving, for the panic below: the last effects seen.
        let mut last: std::collections::VecDeque<String> = std::collections::VecDeque::new();
        let mut shown: HashMap<(&'static str, String), serde_json::Value> = HashMap::new();
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
                    let again = match &e {
                        Effect::Emit(ev) => shown.insert((party.name, ev.name.clone()), ev.payload.clone()).is_some_and(|before| before == ev.payload),
                        _ => false,
                    };
                    if !again {
                        moved = true;
                    }
                    if last.len() == 12 {
                        last.pop_front();
                    }
                    last.push_back(match &e {
                        Effect::Emit(ev) => format!("{}: {} {}", party.name, ev.name, ev.payload.to_string().chars().take(160).collect::<String>()),
                        Effect::Notify(_) => format!("{}: notify", party.name),
                        Effect::Send(out) => format!("{}: send {:?}", party.name, FakeGroups::open_note(out).map(|(_, _, e)| e.t)),
                    });
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
        panic!("the world does not settle; the last effects:\n{}", last.iter().map(|s| format!("  {s}")).collect::<Vec<_>>().join("\n"));
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
    // `Failed` after the way was once there is a sign the node may be
    // lost, not the end of the call (wave 5): the home is asked, answers,
    // and Alice joins it again rather than ending alone.
    s.inject(SessionEvent::ConnectionState(ConnectionState::Failed));
    w.settle().await;
    assert_eq!(alice.last_state().unwrap().phase, GroupPhase::InRoom, "the home answered: joined again");
    assert!(!alice.notes().contains(&"call.end".to_string()), "the node was there: the call did not end");
    assert!(alice.calls.announced(GROUP).await.is_some());
    assert!(alice.events(UI_EVENT_GROUP_CALL_ENDED).is_empty());
    assert!(alice.errors().is_empty(), "{:?}", alice.errors());
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

// ─── The cascade and the move (wave 5) ───────────────────────────────────────

/// The delays of the cascade and the move, in these tests: short enough
/// that one `settle` (its own small sleeps let the timers fire) carries a
/// whole move through.
fn cascade_timing() -> Timing {
    Timing {
        send_switch_delay: SWITCH,
        // A short retry of the word of identity, as in life: a word lost
        // in the churn of a move is said again before the room settles.
        hello_retry: Duration::from_millis(20),
        lost_after: Duration::from_millis(10),
        hello_check: Duration::from_millis(20),
        rejoin_connect: Duration::from_millis(80),
        rejoin_retry: Duration::from_millis(15),
        move_backup: Duration::from_millis(250),
        move_wait: Duration::from_millis(600),
        ..Timing::default()
    }
}

/// A joiner sits on its own nearest node in a cascade: the room stays on
/// the home, its own node seats it there through a proxy seat (a pass
/// from the home), and the two still confirm each other through it.
#[tokio::test]
async fn a_joiner_sits_through_its_own_node_in_a_cascade() {
    let mut w = World::new(&["alice", "bob"]).await;
    let n1 = w.node.add_node();
    w.node.set_rtt(0, 80);
    w.node.set_rtt(1, 10);
    w.group(&[0, 1]);
    // The room is pinned to node 0 (the home for everyone).
    for i in [0, 1] {
        w.p(i).groups.pin(GROUP, w.node.as_call_node());
    }
    // Bob's own nearest node is node 1, with the cascade.
    w.with_servers(1, vec![w.node.node(n1, NodeClass::Own)], cascade_timing()).await;
    let (alice, bob) = (w.p(0).clone(), w.p(1).clone());

    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    let home_room = w.node.rooms_on(0)[0].clone();
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;

    let bv = bob.last_state().unwrap();
    assert_eq!(bv.phase, GroupPhase::InRoom);
    assert_eq!(bv.node, w.node.node(n1, NodeClass::Own).node.to_string(), "bob is on his own node");
    assert_eq!(bv.home, w.node.as_call_node().node.to_string(), "the room is on the home");
    assert!(w.node.seats_via(&home_room).iter().any(|(_, via)| *via == Some(n1)), "a proxy seat of node 1: {:?}", w.node.seats_via(&home_room));
    assert!(w.node.delegated().iter().any(|(_, r)| *r == home_room), "a pass was asked of the home: {:?}", w.node.delegated());
    assert_eq!(w.node.rooms_on(1), Vec::<String>::new(), "no room is made on bob's own node");
    for p in [&alice, &bob] {
        let v = p.last_state().unwrap();
        assert!(!verified_seats(&v).is_empty() && verified_seats(&v).iter().all(|(_, ok)| *ok), "{}: {:?}", p.name, v.participants);
        assert!(p.errors().is_empty(), "{}: {:?}", p.name, p.errors());
    }
    assert_eq!(alice.last_state().unwrap().home, alice.last_state().unwrap().node, "alice is home directly");
}

/// My own node refusing the cascade (503) is no error to me: I fall back
/// to the home and sit there directly.
#[tokio::test]
async fn a_cascade_refused_falls_back_to_the_home() {
    let mut w = World::new(&["alice", "bob"]).await;
    let n1 = w.node.add_node();
    w.group(&[0, 1]);
    for i in [0, 1] {
        w.p(i).groups.pin(GROUP, w.node.as_call_node());
    }
    w.with_servers(1, vec![w.node.node(n1, NodeClass::Own)], cascade_timing()).await;
    let (alice, bob) = (w.p(0).clone(), w.p(1).clone());
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    let home_room = w.node.rooms_on(0)[0].clone();

    w.node.refuse_next_via(NodeError::Refused { status: 503, error: "cascade_refused".into(), message: "off".into() });
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;

    let bv = bob.last_state().unwrap();
    assert_eq!(bv.phase, GroupPhase::InRoom);
    assert_eq!(bv.node, w.node.as_call_node().node.to_string(), "bob fell back to the home");
    assert_eq!(bv.home, bv.node);
    assert!(w.node.seats_via(&home_room).iter().all(|(_, via)| via.is_none()), "no proxy seat: {:?}", w.node.seats_via(&home_room));
    assert!(bob.errors().is_empty(), "a 503 is no error to me: {:?}", bob.errors());
}

/// The node of the room dies: the creator (the first of the room) makes a
/// room on another node, tells `call.move` with a new epoch, and everybody
/// joins it under the same call — one record, the banner unbroken.
#[tokio::test]
async fn the_room_moves_when_its_node_dies() {
    let mut w = World::new(&["alice", "bob", "carol"]).await;
    let n1 = w.node.add_node();
    w.node.set_rtt(0, 20);
    w.node.set_rtt(1, 50);
    w.group(&[0, 1, 2]);
    let servers = vec![w.node.as_call_node(), w.node.node(n1, NodeClass::Project)];
    for i in [0, 1, 2] {
        w.with_servers(i, servers.clone(), cascade_timing()).await;
    }
    let (alice, bob, carol) = (w.p(0).clone(), w.p(1).clone(), w.p(2).clone());

    let view = alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    carol.calls.join(GROUP).await.unwrap();
    w.settle().await;
    let home_room = w.node.rooms_on(0)[0].clone();
    assert_eq!(w.node.seats(&home_room).len(), 3);
    assert!(w.node.seats_via(&home_room).iter().all(|(_, via)| via.is_none()), "all directly on the home");
    let epoch_before = alice.last_state().unwrap().epoch;

    // The home dies: every sitter is cut off.
    w.node.kill(0);
    w.settle().await;

    assert_eq!(w.node.rooms_on(0), Vec::<String>::new(), "the home's rooms are gone");
    let moved = w.node.rooms_on(1);
    assert_eq!(moved.len(), 1, "one new room on the other node");
    let moved = moved[0].clone();
    assert_eq!(w.node.seats(&moved).len(), 3, "all three came over");
    assert_eq!(alice.count("call.move"), 1, "the creator moved the room");
    assert_eq!(bob.count("call.move"), 0);
    assert_eq!(carol.count("call.move"), 0);
    for p in [&alice, &bob, &carol] {
        let v = p.last_state().unwrap();
        assert_eq!(v.phase, GroupPhase::InRoom, "{}: {:?}", p.name, v);
        assert_eq!(v.call_id, view.call_id, "{}: the same call", p.name);
        assert_eq!(v.home, w.node.node(n1, NodeClass::Project).node.to_string(), "{}: the new home", p.name);
        assert!(v.epoch > epoch_before, "{}: the epoch turned ({} > {})", p.name, v.epoch, epoch_before);
        assert!(!verified_seats(&v).is_empty() && verified_seats(&v).iter().all(|(_, ok)| *ok), "{}: all verified again: {:?}", p.name, v.participants);
        assert_eq!(repo::get(&p.store, &view.call_id).await.unwrap().unwrap().outcome, None, "{}: the call is live, one record", p.name);
    }
    for p in [&alice, &bob, &carol] {
        assert!(p.errors().iter().all(|e| !e.contains("nobody moved")), "{}: {:?}", p.name, p.errors());
    }
}

/// Two moves from one room at once: the first (the creator) and the
/// second both move while none has heard the other (all deaf through the
/// death of the home), each to a node of its own. Then everybody hears
/// both: the newer holds on every device (by `created_at`, then room),
/// the loser's author moves on into it, and all end in one room under one
/// call and one epoch.
#[tokio::test]
async fn two_moves_from_one_room_settle_on_one() {
    let mut w = World::new(&["alice", "bob", "carol"]).await;
    let n1 = w.node.add_node();
    let n2 = w.node.add_node();
    w.node.set_rtt(0, 10);
    w.node.set_rtt(1, 20);
    w.node.set_rtt(2, 30);
    w.group(&[0, 1, 2]);
    // A short backup so the second moves; a long wait so the third does
    // not move within the test (it holds, deaf, for the moves to arrive).
    let timing = Timing { move_backup: Duration::from_millis(15), move_wait: Duration::from_secs(50), ..cascade_timing() };
    // The creator's spare node is node 1, the second's is node 2: the two
    // moves go to plainly distinct rooms.
    w.with_servers(0, vec![w.node.as_call_node(), w.node.node(n1, NodeClass::Project)], timing).await;
    w.with_servers(1, vec![w.node.as_call_node(), w.node.node(n2, NodeClass::Project)], timing).await;
    w.with_servers(2, vec![w.node.as_call_node(), w.node.node(n1, NodeClass::Project)], timing).await;
    let (alice, bob, carol) = (w.p(0).clone(), w.p(1).clone(), w.p(2).clone());
    let view = alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    carol.calls.join(GROUP).await.unwrap();
    w.settle().await;

    // Nobody hears anybody while the home dies: the creator (first) and
    // the second each move the room, to a node of their own.
    for i in [0, 1, 2] {
        w.deafen(i);
    }
    w.node.kill(0);
    w.settle().await;
    assert_eq!(alice.count("call.move"), 1, "the creator moved");
    assert_eq!(bob.count("call.move"), 1, "the second moved too (deaf to the first)");
    assert_eq!(carol.count("call.move"), 0, "the third waits");

    // Now everybody hears everything: the moves meet and the newer holds.
    for i in [0, 1, 2] {
        w.hear(i).await;
    }
    w.settle().await;

    let (a, b, c) = (alice.last_state().unwrap(), bob.last_state().unwrap(), carol.last_state().unwrap());
    assert_eq!(a.phase, GroupPhase::InRoom, "{:?}", a);
    assert_eq!((b.phase, c.phase), (GroupPhase::InRoom, GroupPhase::InRoom));
    assert_eq!((a.call_id.as_str(), b.call_id.as_str(), c.call_id.as_str()), (view.call_id.as_str(), view.call_id.as_str(), view.call_id.as_str()));
    // The home is the winning room's node everywhere (a follower may sit
    // on its own node through a cascade, so `node` can differ; `home` is
    // the room).
    assert_eq!(a.home, b.home, "the same winning room everywhere");
    assert_eq!(b.home, c.home);
    assert_eq!(a.epoch, b.epoch, "one epoch everywhere");
    assert_eq!(b.epoch, c.epoch);
    // The winner holds everywhere: each confirms the other two, which can
    // happen only in one shared room (a split would leave fewer peers).
    for p in [&alice, &bob, &carol] {
        let v = p.last_state().unwrap();
        let seen = verified_seats(&v);
        assert_eq!(seen.len(), 2, "{}: two peers, so one room: {:?}", p.name, v.participants);
        assert!(seen.iter().all(|(_, ok)| *ok), "{}: both verified: {:?}", p.name, v.participants);
    }
    assert_eq!(repo::get(&alice.store, &view.call_id).await.unwrap().unwrap().outcome, None, "one live call");
}

/// A client that does not understand `call.move` (a 5.1.6 one) is modelled
/// by the rule it relies on: after a move, a note of the old room — and a
/// `call.end` without any `room_id`, which it writes — is of a room left
/// behind and does not touch the call.
#[tokio::test]
async fn a_note_of_the_room_left_behind_after_a_move_is_stale() {
    let mut w = World::new(&["alice", "bob", "carol"]).await;
    let n1 = w.node.add_node();
    w.node.set_rtt(0, 20);
    w.node.set_rtt(1, 50);
    w.group(&[0, 1, 2]);
    let servers = vec![w.node.as_call_node(), w.node.node(n1, NodeClass::Project)];
    for i in [0, 1, 2] {
        w.with_servers(i, servers.clone(), cascade_timing()).await;
    }
    let (alice, bob, carol) = (w.p(0).clone(), w.p(1).clone(), w.p(2).clone());
    let view = alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    carol.calls.join(GROUP).await.unwrap();
    w.settle().await;
    let start_room = w.node.rooms_on(0)[0].clone();

    w.node.kill(0);
    w.settle().await;
    assert_eq!(carol.last_state().unwrap().phase, GroupPhase::InRoom, "carol came over");

    // A 5.1.6 client, lost in the old room, writes `call.end` with no
    // `room_id` (the room of the start). It must not end the moved call.
    let stale_end = Envelope::call_end(&view.call_id, "ended", None);
    carol.calls.on_group_note(GROUP, &alice.pk(), &stale_end, w.now()).await.unwrap();
    // And a `call.epoch` of the room left behind, likewise.
    let stale_epoch = GroupSignalRaw::epoch(&view.call_id, 9, &start_room);
    carol.calls.on_group_note(GROUP, &bob.pk(), &stale_epoch, w.now()).await.unwrap();
    w.settle().await;

    let v = carol.last_state().unwrap();
    assert_eq!(v.phase, GroupPhase::InRoom, "the stale call.end did not end the moved call");
    assert!(v.epoch < 9, "the stale call.epoch of the old room did not apply: {}", v.epoch);
    assert!(carol.calls.announced(GROUP).await.is_some(), "the banner stands");
    assert_eq!(repo::get(&carol.store, &view.call_id).await.unwrap().unwrap().outcome, None);
}

/// A tail of wave 4: the banner of "a call is on" outlived `call.end`.
/// When the node's `left` for a peer never reached me (my channel stalled)
/// but that peer's own `call.leave` did, I know the room is empty on my
/// way out and send `call.end`; the banner goes.
#[tokio::test]
async fn call_end_goes_out_when_a_peer_left_by_its_own_note_though_the_node_was_silent() {
    let w = World::new(&["alice", "bob"]).await;
    w.group(&[0, 1]);
    let (alice, bob) = (w.p(0), w.p(1));
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    let alice_session = w.engine.sessions()[0].id();
    assert_eq!(verified_seats(&alice.last_state().unwrap()), vec![(2, true)], "alice sees bob");

    // Alice's channel stalls: the node's words (bob's `left`) no longer
    // reach her. Bob leaves — his `call.leave` note still reaches her.
    w.node.freeze_ctl(alice_session);
    bob.calls.leave().await.unwrap();
    w.settle().await;
    assert!(bob.notes().contains(&"call.leave".to_string()));
    assert!(!bob.notes().contains(&"call.end".to_string()), "bob was not the last");

    // Alice leaves last: she knows bob is gone by his note, so `call.end`
    // goes and the banner is cleared for everybody.
    alice.calls.leave().await.unwrap();
    w.settle().await;
    assert!(alice.notes().contains(&"call.end".to_string()), "call.end goes: the call is over: {:?}", alice.notes());
    assert!(alice.calls.announced(GROUP).await.is_none(), "alice's banner is gone");
    assert!(bob.calls.announced(GROUP).await.is_none(), "bob's banner is gone too");
    assert_eq!(bob.events(UI_EVENT_GROUP_CALL_ENDED).len(), 1);
}

/// A helper to write a `call.epoch` naming a room, for the stale-note test
/// (the core's own signal writer is not public to the tests).
struct GroupSignalRaw;
impl GroupSignalRaw {
    fn epoch(call_id: &str, epoch: u32, room_id: &str) -> Envelope {
        Envelope::call_epoch(call_id, epoch, "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==", None).in_room(Some(room_id))
    }
}

/// The home is a node of the wave before the cascade (no `delegate`): the
/// core understands it and joins directly, even with an own node that
/// could cascade.
#[tokio::test]
async fn a_home_of_the_wave_before_the_cascade_is_joined_directly() {
    let mut w = World::new(&["alice", "bob"]).await;
    let n1 = w.node.add_node();
    w.node.set_wave4(0, true); // the home has no cascade and no delegate
    w.group(&[0, 1]);
    for i in [0, 1] {
        w.p(i).groups.pin(GROUP, w.node.as_call_node());
    }
    w.with_servers(1, vec![w.node.node(n1, NodeClass::Own)], cascade_timing()).await;
    let (alice, bob) = (w.p(0).clone(), w.p(1).clone());
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    let home_room = w.node.rooms_on(0)[0].clone();
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;

    let bv = bob.last_state().unwrap();
    assert_eq!(bv.phase, GroupPhase::InRoom);
    assert_eq!(bv.node, w.node.as_call_node().node.to_string(), "bob is on the home directly");
    assert_eq!(bv.home, bv.node);
    assert!(w.node.seats_via(&home_room).iter().all(|(_, via)| via.is_none()), "no proxy seat on a wave-4 home");
    assert_eq!(w.node.rooms_on(1), Vec::<String>::new(), "no room on bob's own node");
    assert!(bob.errors().is_empty(), "{:?}", bob.errors());
}

// ─── The findings of the review of the move ──────────────────────────────────

/// A secret of all zeros: the smallest there is, so that the rule of
/// `call.epoch` (the smaller secret holds) would surely take it.
const ZERO_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

/// The session of `seat` in `room_id` on the fake node.
fn session_of_seat(w: &World, room_id: &str, seat: u32) -> u32 {
    w.node.seats(room_id).into_iter().find(|(s, _)| *s == seat).map(|(_, session)| session).unwrap_or_else(|| panic!("no seat {seat} in {room_id}"))
}

/// The last key the session `id` sends with in the slot of `epoch`.
fn sending_key_of(w: &World, id: u32, epoch: u32) -> Option<Vec<u8>> {
    w.engine.sessions().into_iter().find(|h| h.id() == id).and_then(|h| sender_key_in(&h.record(), slot(epoch)))
}

/// The secret a party's `call.move` carried.
fn moved_secret_of(p: &Party) -> [u8; 32] {
    use base64::Engine as _;
    let e = p.envelopes().into_iter().find(|e| e.t == "call.move").expect("a call.move");
    base64::engine::general_purpose::STANDARD.decode(e.str_field("secret").unwrap()).unwrap().try_into().unwrap()
}

/// A `call.move` written by hand: from `from` to a room the test made on
/// `node`, by the author's `seat` in the room left, to `epoch` with the
/// zero secret.
async fn crafted_move(w: &World, call_id: &str, from: &str, node: &CallNode, seat: u32, epoch: u32) -> (Envelope, String) {
    let created = RoomApi::create(&w.node, node, MediaLimits::default()).await.unwrap();
    let e = Envelope::call_move(
        call_id,
        from,
        &created.room_id,
        &node.node.to_string(),
        None,
        &created.join_token,
        created.expires_at as i64,
        seat,
        epoch,
        ZERO_SECRET,
    );
    (e, created.room_id)
}

/// Three in a room of the first node, each with the sets of servers given
/// (the home first in each) and the timing given.
async fn three_in_a_room(w: &mut World, servers: [Vec<CallNode>; 3], timing: Timing) -> (GroupCallView, String) {
    w.group(&[0, 1, 2]);
    for (i, s) in servers.into_iter().enumerate() {
        w.with_servers(i, s, timing).await;
    }
    let view = w.p(0).calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    w.p(1).calls.join(GROUP).await.unwrap();
    w.settle().await;
    w.p(2).calls.join(GROUP).await.unwrap();
    w.settle().await;
    let room = w.node.rooms_on(0)[0].clone();
    assert_eq!(w.node.seats(&room).len(), 3);
    (view, room)
}

/// Finding: the move that holds sets the epoch. Two moves from one room
/// carry one epoch number with secrets of their own; the loser's is the
/// smallest there is, so the rule of `call.epoch` would leave every
/// follower on it while the winner (who never takes the loser's note)
/// kept its own, and nobody could read anybody. Now the winner's secret
/// holds everywhere, whatever the loser said and in whatever order the
/// two notes came.
#[tokio::test]
async fn the_move_that_holds_sets_the_epoch_whatever_the_loser_said() {
    let mut w = World::new(&["alice", "bob", "carol"]).await;
    let n1 = w.node.add_node();
    let n2 = w.node.add_node();
    w.node.set_rtt(0, 10);
    w.node.set_rtt(1, 20);
    w.node.set_rtt(2, 30);
    let timing = Timing { move_backup: Duration::from_secs(50), move_wait: Duration::from_secs(50), ..cascade_timing() };
    let (home, p1, p2) = (w.node.as_call_node(), w.node.node(n1, NodeClass::Project), w.node.node(n2, NodeClass::Project));
    let (view, r0) = three_in_a_room(&mut w, [vec![home.clone(), p1.clone()], vec![home.clone(), p2.clone()], vec![home.clone(), p1.clone()]], timing).await;
    let (alice, bob, carol) = (w.p(0).clone(), w.p(1).clone(), w.p(2).clone());

    // Everybody deaf, the home dies: the creator moves to node 1.
    for i in [0, 1, 2] {
        w.deafen(i);
    }
    w.node.kill(0);
    w.settle().await;
    assert_eq!(alice.count("call.move"), 1, "the creator moved");
    assert_eq!(bob.count("call.move"), 0, "the second holds (a long backup in this test)");
    let r_a = w.node.rooms_on(1)[0].clone();

    // Bob's move, said a moment before Alice's (it loses), to a room on
    // node 2 with the zero secret for the same epoch number. Everybody
    // gets it first — the followers apply it and go there.
    let (loser, r_b) = crafted_move(&w, &view.call_id, &r0, &p2, 2, 2).await;
    for i in [0, 1, 2] {
        w.p(i).calls.on_group_note(GROUP, &bob.pk(), &loser, START - 1).await.unwrap();
    }
    w.settle().await;
    assert_eq!(w.node.seats(&r_b).len(), 2, "bob and carol went to the losing room first: {:?}", w.node.seats(&r_b));
    assert_eq!(alice.last_state().unwrap().home, p1.node.to_string(), "alice keeps her own newer room");

    // Then Alice's: the newer holds, over the loser, everywhere.
    for i in [0, 1, 2] {
        w.hear(i).await;
    }
    w.wait(Duration::from_millis(100)).await;

    let winner = moved_secret_of(&alice);
    for p in [&alice, &bob, &carol] {
        let v = p.last_state().unwrap();
        assert_eq!(v.phase, GroupPhase::InRoom, "{}: {:?}", p.name, v);
        assert_eq!(v.home, p1.node.to_string(), "{}: in the winner's room", p.name);
        assert_eq!(v.epoch, 2, "{}: the epoch of the move", p.name);
        let seat = v.participant.unwrap();
        let session = session_of_seat(&w, &r_a, seat);
        assert_eq!(
            sending_key_of(&w, session, 2),
            Some(sender_key(&winner, &view.call_id, seat, 2)),
            "{}: sends under the winner's secret, not the smaller one of the loser",
            p.name
        );
        let seen = verified_seats(&v);
        assert_eq!(seen.len(), 2, "{}: two peers: {:?}", p.name, v.participants);
        assert!(seen.iter().all(|(_, ok)| *ok), "{}: both read: {:?}", p.name, v.participants);
    }
    assert_eq!(w.node.seats(&r_b), vec![], "the losing room is empty");
}

/// Finding: a move held for the HELLO check (the receiver sitting well) is
/// judged against its rival again when it is applied. Carol sits well in
/// the room (its node silent to requests, her channel open) and gets the
/// newer move first and the older one a moment later; both HELLOs time
/// out, the older one's last. The newer holds all the same: the order of
/// the checks does not decide.
#[tokio::test]
async fn a_move_held_for_the_hello_check_still_loses_to_the_newer_one() {
    let mut w = World::new(&["alice", "bob", "carol"]).await;
    let n1 = w.node.add_node();
    let n2 = w.node.add_node();
    w.node.set_rtt(0, 10);
    w.node.set_rtt(1, 20);
    w.node.set_rtt(2, 30);
    let timing = Timing {
        hello_check: Duration::from_millis(60),
        move_backup: Duration::from_millis(300),
        move_wait: Duration::from_secs(50),
        ..cascade_timing()
    };
    let (home, p1, p2) = (w.node.as_call_node(), w.node.node(n1, NodeClass::Project), w.node.node(n2, NodeClass::Project));
    // Carol sits through node 2 (her own), the others on the home directly.
    let own2 = w.node.node(n2, NodeClass::Own);
    let (_view, r0) = three_in_a_room(&mut w, [vec![home.clone(), p1.clone()], vec![home.clone(), p2.clone()], vec![own2.clone()]], timing).await;
    let (alice, bob, carol) = (w.p(0).clone(), w.p(1).clone(), w.p(2).clone());
    let (sa, sb, sc) = (session_of_seat(&w, &r0, 1), session_of_seat(&w, &r0, 2), session_of_seat(&w, &r0, 3));
    assert_eq!(carol.last_state().unwrap().node, own2.node.to_string(), "carol sits through node 2");

    // The home answers nothing any more, but Carol's way stays: she is
    // well. Its words stall too (a dead home says no `left`), so that the
    // composition everybody judges the right and the order to move by
    // stays whole.
    w.node.set_silent(0, true);
    for s in [sa, sb, sc] {
        w.node.freeze_ctl(s);
    }
    for i in [0, 1, 2] {
        w.deafen(i);
    }
    // Alice and Bob lose their way: the first moves at once, the second
    // on its backup, each to a node of its own; Bob's note is the newer.
    w.engine.inject_into(sa, SessionEvent::ConnectionState(ConnectionState::Disconnected));
    w.engine.inject_into(sb, SessionEvent::ConnectionState(ConnectionState::Disconnected));
    w.wait(Duration::from_millis(150)).await;
    assert_eq!(alice.count("call.move"), 1, "alice moved first");
    assert_eq!(bob.count("call.move"), 0, "bob not yet");
    w.clock.0.store(START + 5, Ordering::SeqCst);
    w.wait(Duration::from_millis(450)).await;
    assert_eq!(bob.count("call.move"), 1, "bob moved too, deaf to alice");
    let alice_move = alice.envelopes().into_iter().find(|e| e.t == "call.move").unwrap();
    let bob_move = bob.envelopes().into_iter().find(|e| e.t == "call.move").unwrap();

    // Carol, sitting well: Bob's newer move first, Alice's older one a
    // moment later; each is held for a HELLO that times out, the older
    // one's last.
    carol.calls.on_group_note(GROUP, &bob.pk(), &bob_move, START + 5).await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    carol.calls.on_group_note(GROUP, &alice.pk(), &alice_move, START).await.unwrap();
    // Carol hears nothing else: each move once, as in life (a repeat of
    // the newer one later would mend a wrong choice by itself).
    for i in [0, 1] {
        w.hear(i).await;
    }
    w.wait(Duration::from_millis(250)).await;

    for p in [&alice, &bob, &carol] {
        let v = p.last_state().unwrap();
        assert_eq!(v.phase, GroupPhase::InRoom, "{}: {:?}", p.name, v);
        assert_eq!(v.home, p2.node.to_string(), "{}: the newer move holds, whichever check ended last", p.name);
    }
}

/// Finding: no time window on the glare of moves. A move from a room the
/// group left two minutes ago, said after the one that holds, is a
/// straggler's: it does not uproot the room everybody sits in.
#[tokio::test]
async fn a_straggler_move_long_after_the_one_that_holds_changes_nothing() {
    let mut w = World::new(&["alice", "bob", "carol"]).await;
    let n1 = w.node.add_node();
    let n2 = w.node.add_node();
    w.node.set_rtt(0, 20);
    w.node.set_rtt(1, 50);
    w.node.set_rtt(2, 50);
    let (home, p1, p2) = (w.node.as_call_node(), w.node.node(n1, NodeClass::Project), w.node.node(n2, NodeClass::Project));
    let servers = vec![home.clone(), p1.clone()];
    let (view, r0) = three_in_a_room(&mut w, [servers.clone(), servers.clone(), servers], cascade_timing()).await;
    let (alice, bob, carol) = (w.p(0).clone(), w.p(1).clone(), w.p(2).clone());
    w.node.kill(0);
    w.settle().await;
    for p in [&alice, &bob, &carol] {
        assert_eq!(p.last_state().unwrap().home, p1.node.to_string(), "{}: moved to node 1", p.name);
    }
    let epoch = carol.last_state().unwrap().epoch;

    // Two minutes later: a move of the dead room by its second (Bob's seat
    // 2, a right he had), to a room on node 2. Newer than the move that
    // holds, but long after it.
    w.clock.0.store(START + 120, Ordering::SeqCst);
    let (straggler, r_x) = crafted_move(&w, &view.call_id, &r0, &p2, 2, 3).await;
    for i in [0, 1, 2] {
        w.p(i).calls.on_group_note(GROUP, &bob.pk(), &straggler, w.now()).await.unwrap();
    }
    w.wait(Duration::from_millis(100)).await;

    for p in [&alice, &bob, &carol] {
        let v = p.last_state().unwrap();
        assert_eq!(v.phase, GroupPhase::InRoom, "{}: {:?}", p.name, v);
        assert_eq!(v.home, p1.node.to_string(), "{}: still in the room that holds", p.name);
        assert_eq!(v.epoch, epoch, "{}: the epoch did not turn", p.name);
    }
    assert_eq!(w.node.seats(&r_x), vec![], "nobody went to the straggler's room");
}

/// Finding: the mover ordered two moves by the time its task began, not by
/// the time its note was said (`created_at`, stamped after the room was
/// made and joined). Alice's room is slow to make; Bob's move is said in
/// between. Everybody else orders Alice's note (said later) over Bob's —
/// and so does Alice now, instead of abandoning her own room.
#[tokio::test]
async fn the_mover_orders_two_moves_by_the_time_its_note_is_said() {
    let mut w = World::new(&["alice", "bob", "carol"]).await;
    let n1 = w.node.add_node();
    let n2 = w.node.add_node();
    w.node.set_rtt(0, 20);
    w.node.set_rtt(1, 50);
    w.node.set_rtt(2, 50);
    let timing = Timing { move_backup: Duration::from_millis(100), move_wait: Duration::from_secs(50), ..cascade_timing() };
    let (home, p1, p2) = (w.node.as_call_node(), w.node.node(n1, NodeClass::Project), w.node.node(n2, NodeClass::Project));
    let (_view, _r0) = three_in_a_room(&mut w, [vec![home.clone(), p1.clone()], vec![home.clone(), p2.clone()], vec![home.clone(), p2.clone()]], timing).await;
    let (alice, bob, carol) = (w.p(0).clone(), w.p(1).clone(), w.p(2).clone());

    for i in [0, 1, 2] {
        w.deafen(i);
    }
    // Alice's room (the next one made) waits at the gate; the home dies.
    let gate = w.node.hold_next_create();
    w.node.kill(0);
    // Alice is at the gate, her task begun at START; Bob's backup is not
    // due yet.
    tokio::time::sleep(Duration::from_millis(40)).await;
    w.clock.0.store(START + 5, Ordering::SeqCst);
    w.wait(Duration::from_millis(200)).await;
    assert_eq!(bob.count("call.move"), 1, "bob moved on his backup, his note said at START+5");
    assert_eq!(alice.count("call.move"), 0, "alice still waits for her room");
    // Alice's room is made now, her note said at START+10: the newer.
    w.clock.0.store(START + 10, Ordering::SeqCst);
    gate.notify_one();
    w.wait(Duration::from_millis(200)).await;
    assert_eq!(alice.count("call.move"), 1, "alice moved");

    for i in [0, 1, 2] {
        w.hear(i).await;
    }
    w.wait(Duration::from_millis(200)).await;
    for p in [&alice, &bob, &carol] {
        let v = p.last_state().unwrap();
        assert_eq!(v.phase, GroupPhase::InRoom, "{}: {:?}", p.name, v);
        assert_eq!(v.home, p1.node.to_string(), "{}: alice's room, said later, holds — for alice too", p.name);
    }
}

/// Finding: a join after a move that never connects was never timed out.
/// Bob follows the move and his way to the new node never comes: in
/// `rejoin_connect` the loss is judged (the home answers), he joins again,
/// and the way then comes.
#[tokio::test]
async fn a_join_after_a_move_that_never_connects_is_judged_in_its_time() {
    let mut w = World::new(&["alice", "bob"]).await;
    let n1 = w.node.add_node();
    w.node.set_rtt(0, 20);
    w.node.set_rtt(1, 50);
    w.group(&[0, 1]);
    let servers = vec![w.node.as_call_node(), w.node.node(n1, NodeClass::Project)];
    let timing = Timing { move_backup: Duration::from_secs(2), ..cascade_timing() };
    for i in [0, 1] {
        w.with_servers(i, servers.clone(), timing).await;
    }
    let (alice, bob) = (w.p(0).clone(), w.p(1).clone());
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;

    w.deafen(1);
    w.node.kill(0);
    w.settle().await;
    assert_eq!(alice.count("call.move"), 1);
    let hellos_before = w.node.hellos(1);

    // Bob hears the move; his way to the new node never comes.
    w.engine.set_connects(false);
    w.hear(1).await;
    w.settle().await;
    assert_eq!(bob.last_state().unwrap().phase, GroupPhase::Joining, "{:?}", bob.last_state());
    assert_eq!(w.node.hellos(1), hellos_before, "nothing judged yet");
    // Before his time is up the way opens again: his join again connects.
    tokio::time::sleep(Duration::from_millis(20)).await;
    w.engine.set_connects(true);
    w.wait(Duration::from_millis(250)).await;

    let v = bob.last_state().unwrap();
    assert_eq!(v.phase, GroupPhase::InRoom, "judged in its time and joined again: {v:?}");
    assert!(w.node.hellos(1) > hellos_before, "the home was asked from his point");
    assert_eq!(v.home, w.node.node(n1, NodeClass::Project).node.to_string());
    assert!(!bob.notes().contains(&"call.end".to_string()));
    assert!(alice.last_state().unwrap().phase == GroupPhase::InRoom);
}

/// Finding: the second point of the judgement (a join through my own node,
/// after a pass from the home) was not bounded like the HELLO: a home
/// whose packets are black-holed holds the pass request until the connect
/// times out, seconds later. Now the judgement ends in `hello_check` from
/// both points, and the creator moves within the budget.
#[tokio::test]
async fn the_second_point_of_the_judgement_is_bounded_like_the_hello() {
    let mut w = World::new(&["alice", "bob"]).await;
    let n1 = w.node.add_node();
    w.node.set_rtt(0, 80);
    w.node.set_rtt(1, 10);
    w.group(&[0, 1]);
    for i in [0, 1] {
        w.p(i).groups.pin(GROUP, w.node.as_call_node());
    }
    let timing = Timing { move_backup: Duration::from_millis(300), ..cascade_timing() };
    // Alice's own node is node 1 (nearer, with the cascade): the second
    // point of her judgement.
    w.with_servers(0, vec![w.node.node(n1, NodeClass::Own)], timing).await;
    w.with_servers(1, vec![w.node.as_call_node()], timing).await;
    let (alice, bob) = (w.p(0).clone(), w.p(1).clone());
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    assert_eq!(alice.last_state().unwrap().node, w.node.as_call_node().node.to_string(), "the creator sits on the home");

    // The home's VM is gone with its packets: nothing answers, nothing
    // refuses.
    w.node.set_silent(0, true);
    w.node.kill(0);
    w.wait(Duration::from_millis(150)).await;

    assert_eq!(alice.count("call.move"), 1, "the creator moved within the budget: {:?}", alice.notes());
    assert_eq!(bob.count("call.move"), 0, "the second had no need to");
    for p in [&alice, &bob] {
        let v = p.last_state().unwrap();
        assert_eq!(v.phase, GroupPhase::InRoom, "{}: {:?}", p.name, v);
        assert_eq!(v.home, w.node.node(n1, NodeClass::Own).node.to_string(), "{}: on node 1", p.name);
    }
}

/// Finding: the time of a loss ran from the first `Disconnected`, not the
/// latest. A way that comes back and goes again (a handover) is judged
/// from its latest outage: no loss sign while each outage is shorter than
/// `lost_after`.
#[tokio::test]
async fn a_way_that_came_back_and_went_again_is_judged_from_its_latest_outage() {
    let w = World::new(&["alice", "bob"]).await;
    w.group(&[0, 1]);
    w.timing(Timing { lost_after: Duration::from_millis(200), hello_check: Duration::from_millis(20), send_switch_delay: SWITCH, ..Timing::default() });
    let (alice, bob) = (w.p(0).clone(), w.p(1).clone());
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    let room = w.node.rooms()[0].clone();
    let sb = session_of_seat(&w, &room, 2);
    let hellos_before = w.node.hellos(0);

    // Out at 0, back at 100, out at 150, back at 250: no outage lasts 200.
    w.engine.inject_into(sb, SessionEvent::ConnectionState(ConnectionState::Disconnected));
    tokio::time::sleep(Duration::from_millis(100)).await;
    w.engine.inject_into(sb, SessionEvent::ConnectionState(ConnectionState::Connected));
    tokio::time::sleep(Duration::from_millis(50)).await;
    w.engine.inject_into(sb, SessionEvent::ConnectionState(ConnectionState::Disconnected));
    tokio::time::sleep(Duration::from_millis(100)).await;
    w.engine.inject_into(sb, SessionEvent::ConnectionState(ConnectionState::Connected));
    w.wait(Duration::from_millis(300)).await;

    let v = bob.last_state().unwrap();
    assert_eq!(v.phase, GroupPhase::InRoom, "{v:?}");
    assert_eq!(v.participant, Some(2), "the same seat: no join again");
    assert_eq!(w.node.hellos(0), hellos_before, "no judgement began");
    assert_eq!(w.node.seats(&room).len(), 2);
    assert_eq!(bob.count("call.leave"), 0);
}

/// Finding: after a join again, my own node's earlier `home_lost` was
/// still remembered, and the next loss was judged from one point only.
/// Bob sits through his own node; it says the home is lost though it is
/// not; the home answers, Bob joins it again directly. Later his direct
/// way breaks: judged from both points again, his own node seats him.
#[tokio::test]
async fn after_a_join_again_the_next_loss_is_judged_from_both_points() {
    let mut w = World::new(&["alice", "bob"]).await;
    let n1 = w.node.add_node();
    w.node.set_rtt(0, 80);
    w.node.set_rtt(1, 10);
    w.group(&[0, 1]);
    for i in [0, 1] {
        w.p(i).groups.pin(GROUP, w.node.as_call_node());
    }
    w.with_servers(1, vec![w.node.node(n1, NodeClass::Own)], cascade_timing()).await;
    let (alice, bob) = (w.p(0).clone(), w.p(1).clone());
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    let room = w.node.rooms_on(0)[0].clone();
    let own = w.node.node(n1, NodeClass::Own).node.to_string();
    let home = w.node.as_call_node().node.to_string();
    assert_eq!(bob.last_state().unwrap().node, own, "bob sits through his own node");

    // His own node says the home is lost; it is not: the home answers
    // and Bob joins it again directly.
    let sb = session_of_seat(&w, &room, bob.last_state().unwrap().participant.unwrap());
    w.engine.inject_into(sb, SessionEvent::Data { label: CTL_LABEL.into(), payload: DataPayload::Text(Message::HomeLost.encode()) });
    w.wait(Duration::from_millis(100)).await;
    let v = bob.last_state().unwrap();
    assert_eq!((v.phase, v.node.as_str()), (GroupPhase::InRoom, home.as_str()), "joined the home again directly: {v:?}");
    let passes_before = w.node.delegated().len();

    // His direct way breaks while the home is fine: both points judge,
    // and the join through his own node is the join again.
    let sb = session_of_seat(&w, &room, v.participant.unwrap());
    w.engine.inject_into(sb, SessionEvent::ConnectionState(ConnectionState::Disconnected));
    w.wait(Duration::from_millis(150)).await;
    let v = bob.last_state().unwrap();
    assert_eq!(v.phase, GroupPhase::InRoom, "{v:?}");
    assert_eq!(v.node, own, "judged from his own node too: seated through it again");
    assert!(w.node.delegated().len() > passes_before, "a pass was asked of the home for the second point");
    assert_eq!(v.home, home);
}

/// Finding: a move that did not work out took the mover out of the call
/// with `call.leave`. The creator's room is refused by the node; she tells
/// the group nothing and waits; the second moves on its backup, and she
/// follows.
#[tokio::test]
async fn a_move_that_did_not_work_out_waits_for_the_seconds_move() {
    let mut w = World::new(&["alice", "bob", "carol"]).await;
    let n1 = w.node.add_node();
    let n2 = w.node.add_node();
    w.node.set_rtt(0, 20);
    w.node.set_rtt(1, 50);
    w.node.set_rtt(2, 50);
    let timing = Timing { move_backup: Duration::from_millis(60), ..cascade_timing() };
    let (home, p1, p2) = (w.node.as_call_node(), w.node.node(n1, NodeClass::Project), w.node.node(n2, NodeClass::Project));
    let (view, _r0) = three_in_a_room(&mut w, [vec![home.clone(), p1.clone()], vec![home.clone(), p2.clone()], vec![home.clone(), p2.clone()]], timing).await;
    let (alice, bob, carol) = (w.p(0).clone(), w.p(1).clone(), w.p(2).clone());

    // The next room asked for (the creator's) is refused.
    w.node.refuse_next(NodeError::Refused { status: 503, error: "overloaded".into(), message: "fake node: overloaded".into() });
    w.node.kill(0);
    w.wait(Duration::from_millis(250)).await;

    assert_eq!(alice.count("call.move"), 0, "the creator's room was refused");
    assert_eq!(alice.count("call.leave"), 0, "she told the group nothing and did not leave: {:?}", alice.notes());
    assert_eq!(bob.count("call.move"), 1, "the second moved on his backup");
    for p in [&alice, &bob, &carol] {
        let v = p.last_state().unwrap();
        assert_eq!(v.phase, GroupPhase::InRoom, "{}: {:?}", p.name, v);
        assert_eq!(v.call_id, view.call_id);
        assert_eq!(v.home, p2.node.to_string(), "{}: in the second's room", p.name);
    }
    assert_eq!(repo::get(&alice.store, &view.call_id).await.unwrap().unwrap().outcome, None, "the call is live for the creator too");
}

/// Finding, the other end: a move that did not work out leaves the mover
/// waiting with its seat in the dead room as it was claimed. Nobody else
/// moves in time, so it leaves `failed` — with the `call.leave` of the
/// dead room, which its claim still owes, and never `call.end`.
#[tokio::test]
async fn a_move_that_did_not_work_out_still_owes_the_leave_of_the_dead_room() {
    let mut w = World::new(&["alice", "bob"]).await;
    let n1 = w.node.add_node();
    w.node.set_rtt(0, 20);
    w.node.set_rtt(1, 50);
    w.group(&[0, 1]);
    let servers = vec![w.node.as_call_node(), w.node.node(n1, NodeClass::Project)];
    // The second moves long after the creator gives up waiting.
    w.with_servers(0, servers.clone(), Timing { move_wait: Duration::from_millis(200), ..cascade_timing() }).await;
    w.with_servers(1, servers, Timing { move_backup: Duration::from_secs(50), ..cascade_timing() }).await;
    let (alice, bob) = (w.p(0).clone(), w.p(1).clone());
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    let r0 = w.node.rooms_on(0)[0].clone();
    assert_eq!(alice.count("call.join"), 1, "her seat in the room is claimed");

    w.node.refuse_next(NodeError::Refused { status: 503, error: "overloaded".into(), message: "fake node: overloaded".into() });
    w.node.kill(0);
    w.wait(Duration::from_millis(150)).await;
    assert_eq!(alice.count("call.move"), 0, "the creator's room was refused");
    assert_eq!(alice.count("call.leave"), 0, "she waits for the second's move: {:?}", alice.notes());
    assert_eq!(alice.last_state().unwrap().phase, GroupPhase::Reconnecting);

    w.wait(Duration::from_millis(300)).await;
    assert_eq!(alice.last_state().unwrap().phase, GroupPhase::Left, "nobody moved the room in time: out");
    let leaves: Vec<Envelope> = alice.envelopes().into_iter().filter(|e| e.t == "call.leave").collect();
    assert_eq!(leaves.len(), 1, "the leave of her claimed seat: {:?}", alice.notes());
    assert_eq!(leaves[0].str_field("room_id"), Some(r0.as_str()), "of the dead room");
    assert_eq!(leaves[0].fields.get("participant").and_then(serde_json::Value::as_u64), Some(1), "her seat there");
    assert_eq!(alice.count("call.end"), 0, "an empty dead room is no knowledge of the end");
}

// ─── Private nodes by invitation (wave 6) ──────────────────────────────────

/// A private node I was invited to is mine (class `own`): my rooms go
/// on it (it being my only node) with the credentials of my device,
/// which nobody else is told — a member joins my room by its token, the
/// node not letting them use its TURN. Revoked on the node, the
/// credentials open nothing: no room of mine goes there, and with the
/// project's node in my sets the next room goes on it.
#[tokio::test]
async fn an_invited_private_node_is_mine_and_its_credentials_stay_with_me() {
    let mut w = World::new(&["alice", "bob"]).await;
    let n1 = w.node.add_node();
    w.node.set_private(n1, true, &[]);
    w.group(&[0, 1]);
    // The operator invites Alice's phone (`vcall ctl invite`); the app
    // exchanges the token of the link for the credentials of the device.
    let token = w.node.invite(n1, 1, 3600);
    let private = w.node.node(n1, NodeClass::Own).node;
    let creds = w.node.client().redeem_invite(&private, &token, "Veydan Chat, test", START).await.unwrap();
    assert_eq!(w.node.devices(n1).len(), 1);
    assert!(w.node.client().redeem_invite(&private, &token, "again", START).await.unwrap_err().to_string().contains("bad_invite"), "one use");
    let mine = messenger_calls::CallNode::with_device(NodeClass::Own, &creds);
    assert!(mine.is_device() && mine.shared_key().is_none());
    w.with_servers(0, vec![mine.clone()], cascade_timing()).await;
    let (alice, bob) = (w.p(0).clone(), w.p(1).clone());

    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    assert_eq!(w.node.rooms_on(n1).len(), 1, "the room is on my private node, my only one");
    assert_eq!(alice.last_state().unwrap().phase, GroupPhase::InRoom);
    let start = alice.envelopes().into_iter().find_map(|e| match messenger_calls::GroupSignal::parse(&e) {
        Some(s @ messenger_calls::GroupSignal::Start { .. }) => Some(s),
        _ => None,
    });
    let messenger_calls::GroupSignal::Start { key, node, .. } = start.unwrap() else { unreachable!() };
    assert_eq!(node, private, "the group is told the node");
    assert_eq!(key, None, "and nothing of my credentials");

    // Bob has nothing for the node: he still gets in by the token.
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    assert_eq!(bob.last_state().unwrap().phase, GroupPhase::InRoom);
    assert!(bob.errors().is_empty(), "{:?}", bob.errors());
    alice.calls.leave().await.unwrap();
    bob.calls.leave().await.unwrap();
    w.settle().await;

    // Revoked (`vcall ctl revoke`): a fresh core of Alice's finds the
    // node there but closed to her: no room on it, and the next room
    // elsewhere when there is an elsewhere.
    assert!(w.node.revoke(n1, &creds.device_id));
    assert!(w.node.devices(n1).is_empty());
    w.with_servers(0, vec![mine.clone()], cascade_timing()).await;
    let made_before = w.node.created_on().len();
    let err = w.p(0).calls.start(GROUP, Media::Audio).await.unwrap_err();
    assert!(err.to_string().contains("no call node with an SFU answered"), "{err}");
    assert_eq!(w.node.created_on().len(), made_before, "nothing was made on the node that shut me out");
    w.with_servers(0, vec![mine, w.node.as_call_node()], cascade_timing()).await;
    let alice = w.p(0).clone();
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    let made = w.node.created_on();
    assert_eq!(made.len(), made_before + 1);
    assert_eq!(made.last().unwrap(), &w.node.reference().to_string(), "the room went on the project's node, not the private one");
    assert_eq!(alice.last_state().unwrap().phase, GroupPhase::InRoom);
}

/// The node pinned to a group with the group's key, on which I was also
/// invited: my own requests go with my credentials, the group is still
/// told the group's key, and a member without an invitation uses it.
#[tokio::test]
async fn the_node_pinned_to_a_group_takes_the_group_key_and_my_device_alike() {
    let mut w = World::new(&["alice", "bob"]).await;
    let n1 = w.node.add_node();
    w.node.set_private(n1, true, &["gk"]);
    w.group(&[0, 1]);
    let private = w.node.node(n1, NodeClass::Own).node;
    for i in [0, 1] {
        w.p(i).groups.pin(GROUP, messenger_calls::CallNode::with_key(private.clone(), NodeClass::Group, Some("gk".into())));
    }
    let token = w.node.invite(n1, 1, 3600);
    let creds = w.node.client().redeem_invite(&private, &token, "Veydan Chat, test", START).await.unwrap();
    w.with_servers(0, vec![messenger_calls::CallNode::with_device(NodeClass::Own, &creds)], cascade_timing()).await;
    let (alice, bob) = (w.p(0).clone(), w.p(1).clone());

    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    assert_eq!(w.node.rooms_on(n1).len(), 1);
    let start = alice.envelopes().into_iter().find_map(|e| match messenger_calls::GroupSignal::parse(&e) {
        Some(s @ messenger_calls::GroupSignal::Start { .. }) => Some(s),
        _ => None,
    });
    let messenger_calls::GroupSignal::Start { key, .. } = start.unwrap() else { unreachable!() };
    assert_eq!(key.as_deref(), Some("gk"), "the group's key, not my credentials");
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    assert_eq!(bob.last_state().unwrap().phase, GroupPhase::InRoom);
    for p in [&alice, &bob] {
        assert!(p.errors().is_empty(), "{}: {:?}", p.name, p.errors());
    }

    // My device revoked on the node (`vcall ctl revoke`), the group's key
    // still good: the pinned node takes the group's key after my
    // credentials, and my calls go on there as everybody's.
    alice.calls.leave().await.unwrap();
    bob.calls.leave().await.unwrap();
    w.settle().await;
    assert!(w.node.revoke(n1, &creds.device_id));
    w.with_servers(0, vec![messenger_calls::CallNode::with_device(NodeClass::Own, &creds)], cascade_timing()).await;
    let alice = w.p(0).clone();
    let made_before = w.node.created_on().len();
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    assert_eq!(w.node.created_on().len(), made_before + 1);
    assert_eq!(w.node.created_on().last().unwrap(), &private.to_string(), "on the pinned node, by the group's key");
    assert_eq!(alice.last_state().unwrap().phase, GroupPhase::InRoom);
    assert!(alice.errors().is_empty(), "{:?}", alice.errors());
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    assert_eq!(bob.last_state().unwrap().phase, GroupPhase::InRoom);
    alice.calls.leave().await.unwrap();
    bob.calls.leave().await.unwrap();
    w.settle().await;

    // The group's key withdrawn on the node too: the pinned node lets me
    // in no more, and a start says so.
    w.node.set_private(n1, true, &[]);
    w.with_servers(0, vec![messenger_calls::CallNode::with_device(NodeClass::Own, &creds)], cascade_timing()).await;
    let err = w.p(0).calls.start(GROUP, Media::Audio).await.unwrap_err();
    assert!(err.to_string().contains("does not let me in"), "{err}");
}

/// Whether the session `id` of the fake engine was given a TURN server.
fn session_has_turn(w: &World, id: u32) -> bool {
    w.engine.sessions().into_iter().find(|h| h.id() == id).is_some_and(|h| h.record().ice_servers.iter().any(|s| s.urls.iter().any(|u| u.starts_with("turn"))))
}

/// The session of a party's own seat in the room `room_id`.
fn my_session(w: &World, p: &Party, room_id: &str) -> u32 {
    let seat = p.last_state().unwrap().participant.expect("a seat");
    session_of_seat(w, room_id, seat)
}

/// A family's private node invited all of us. The room is on it with no
/// key told (my credentials go to nobody); a member enters it with their
/// own credentials and has its TURN, as on a pinned node. A member whose
/// device the operator revoked since still enters by the token, as
/// anybody, without TURN.
#[tokio::test]
async fn a_member_invited_to_the_node_of_the_room_enters_with_their_own_credentials() {
    let mut w = World::new(&["alice", "bob", "carol"]).await;
    let n1 = w.node.add_node();
    w.node.set_private(n1, true, &[]);
    w.group(&[0, 1, 2]);
    let private = w.node.node(n1, NodeClass::Own).node;
    let mut creds = vec![];
    for i in [0, 1, 2] {
        let token = w.node.invite(n1, 1, 3600);
        let c = w.node.client().redeem_invite(&private, &token, "Veydan Chat, test", START).await.unwrap();
        w.with_servers(i, vec![messenger_calls::CallNode::with_device(NodeClass::Own, &c)], cascade_timing()).await;
        creds.push(c);
    }
    // Carol's device was revoked before the call (`vcall ctl revoke`).
    assert!(w.node.revoke(n1, &creds[2].device_id));
    let (alice, bob, carol) = (w.p(0).clone(), w.p(1).clone(), w.p(2).clone());

    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    assert_eq!(w.node.rooms_on(n1).len(), 1, "the invited node is the only one: the room is on it");
    let room = w.node.rooms_on(n1)[0].clone();
    let start = alice.envelopes().into_iter().find(|e| e.t == "call.start").unwrap();
    assert_eq!(start.str_field("key"), None, "nothing of my credentials is told");
    assert!(session_has_turn(&w, my_session(&w, &alice, &room)), "the starter has the TURN of her node");

    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    let bv = bob.last_state().unwrap();
    assert_eq!(bv.phase, GroupPhase::InRoom);
    assert_eq!((bv.node.as_str(), bv.home.as_str()), (private.to_string().as_str(), private.to_string().as_str()), "directly on the home");
    assert!(session_has_turn(&w, my_session(&w, &bob, &room)), "bob's own credentials opened the node's TURN for him");
    assert!(bob.errors().is_empty(), "{:?}", bob.errors());
    assert_eq!(w.node.delegated(), Vec::<(String, String)>::new(), "no pass was asked: his own node is the home");

    carol.calls.join(GROUP).await.unwrap();
    w.settle().await;
    let cv = carol.last_state().unwrap();
    assert_eq!(cv.phase, GroupPhase::InRoom);
    assert_eq!(cv.node, private.to_string());
    assert!(!session_has_turn(&w, my_session(&w, &carol, &room)), "no credentials of hers hold there: in by the token, no TURN");
    assert!(carol.errors().is_empty(), "{:?}", carol.errors());
    assert_eq!(alice.last_state().unwrap().participants.len(), 3, "alice sees both");
}

/// A room goes where the group can follow: on a node whose access the
/// members can be told (the project's), before a private node that
/// invited only this device — there the members would have no TURN,
/// and a member of 5.1.7 could not enter at all. The invited node hosts
/// a room when nothing else with an SFU answers.
#[tokio::test]
async fn a_room_goes_where_the_group_can_follow_before_my_invited_node() {
    let mut w = World::new(&["alice", "bob"]).await;
    let n1 = w.node.add_node();
    w.node.set_private(n1, true, &[]);
    // The invited node is the nearer: nearness does not make it the host.
    w.node.set_rtt(0, 80);
    w.node.set_rtt(n1, 10);
    w.group(&[0, 1]);
    let private = w.node.node(n1, NodeClass::Own).node;
    let token = w.node.invite(n1, 1, 3600);
    let creds = w.node.client().redeem_invite(&private, &token, "Veydan Chat, test", START).await.unwrap();
    let mine = messenger_calls::CallNode::with_device(NodeClass::Own, &creds);
    w.with_servers(0, vec![mine.clone(), w.node.as_call_node()], cascade_timing()).await;
    let (alice, bob) = (w.p(0).clone(), w.p(1).clone());

    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    assert_eq!(w.node.rooms_on(n1), Vec::<String>::new(), "no room on the node only I can use");
    assert_eq!(w.node.rooms_on(0).len(), 1, "the room is on the project's node");
    let room = w.node.rooms_on(0)[0].clone();
    bob.calls.join(GROUP).await.unwrap();
    w.settle().await;
    assert_eq!(bob.last_state().unwrap().phase, GroupPhase::InRoom);
    assert!(session_has_turn(&w, my_session(&w, &bob, &room)), "a member has the TURN of the project's node");
    assert!(bob.errors().is_empty(), "{:?}", bob.errors());
    bob.calls.leave().await.unwrap();
    w.wait(SWITCH * 3).await;
    alice.calls.leave().await.unwrap();
    w.settle().await;
    assert!(alice.calls.announced(GROUP).await.is_none(), "the last one out ended it");

    // The project's node gone (a fresh core of Alice's, so that nothing
    // of it is in the cache of credentials): the invited node is the
    // last resort.
    w.node.kill(0);
    w.with_servers(0, vec![mine, w.node.as_call_node()], cascade_timing()).await;
    let alice = w.p(0).clone();
    let made_before = w.node.created_on().len();
    alice.calls.start(GROUP, Media::Audio).await.unwrap();
    w.settle().await;
    assert_eq!(w.node.created_on().len(), made_before + 1);
    assert_eq!(w.node.created_on().last().unwrap(), &private.to_string(), "the room went on my invited node");
    assert_eq!(alice.last_state().unwrap().phase, GroupPhase::InRoom);
}
