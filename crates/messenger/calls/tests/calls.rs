// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Calls between cores on the fake engine, the wraps carried between the
//! parties by hand (the fake transport of these tests): every `Send`
//! effect of one party is opened with the keys of whoever it is
//! addressed to and handed to that party's handler, as ingress would.
//! Several parties may share keys: the devices of one person.

use messenger_calls::{
    CallDmHandler, CallService, CallView, Direction, Media, NodeClient, Outcome, PairKind, Phase, RelayPolicy, SessionEvent,
    StaticServerSets, VideoInput, VideoQuality, VideoSettings, VideoSize, VideoTrack, ANSWERED_ELSEWHERE, CONNECT_TIMEOUT,
    LOSS_CONFIRM, RESTART_SETTLE, RING_TIMEOUT, UI_EVENT_CALL_ENDED, UI_EVENT_CALL_INCOMING, UI_EVENT_CALL_STATE, VIDEO_FPS,
};
use messenger_contacts::{ContactService, ProfileService};
use messenger_core::outbound::WireEvent;
use messenger_core::traits::UiEvent;
use messenger_core::{Clock, Context, Effect, Envelope, Handler, Outbound, PubKey, Timestamp};
use messenger_dm::{Action, DmHandler, DmService};
use messenger_store::{calls as repo, chats, Store};
use messenger_testkit::{open_dm, wrap_recipients, FakeEngine};
use nostr::key::Keys;
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

struct Party {
    name: &'static str,
    keys: Keys,
    store: Store,
    dm: DmService,
    calls: CallService,
    handler: Arc<dyn Handler<messenger_core::DmInbound>>,
    rx: Mutex<UnboundedReceiver<Effect>>,
    ui: Mutex<Vec<UiEvent>>,
    sent: Mutex<Vec<Outbound>>,
    /// What the fake transport is told to lose: wraps to this party are
    /// dropped while set.
    deaf: Mutex<bool>,
    /// When this party's session began (the context of its handler).
    started: AtomicI64,
}

impl Party {
    async fn new(name: &'static str, keys: Keys, clock: Arc<TestClock>, gate: bool, engine: FakeEngine) -> Self {
        let store = Store::open_in_memory().await.unwrap();
        let profiles = ProfileService::new(store.clone());
        let contacts = ContactService::new(store.clone(), profiles.clone());
        let dm = DmService::new(store.clone(), contacts, profiles, clock.clone());
        dm.set_gate(gate);
        dm.set_signer(Some(keys.clone()));
        let (calls, rx) =
            CallService::new(store.clone(), dm.clone(), Arc::new(engine), Arc::new(StaticServerSets(vec![])), NodeClient::new("test"), clock);
        calls.set_signer(Some(keys.clone()));
        let handler: Arc<dyn Handler<messenger_core::DmInbound>> =
            Arc::new(CallDmHandler::new(calls.clone(), Arc::new(DmHandler::new(dm.clone()))));
        Self {
            name,
            keys,
            store,
            dm,
            calls,
            handler,
            rx: Mutex::new(rx),
            ui: Mutex::new(vec![]),
            sent: Mutex::new(vec![]),
            deaf: Mutex::new(false),
            started: AtomicI64::new(START - 10),
        }
    }

    fn pk(&self) -> PubKey {
        PubKey::parse(&self.keys.public_key().to_hex()).unwrap()
    }

    fn ctx(&self, clock: &Arc<TestClock>) -> Context {
        Context { my_pubkey: self.pk(), session_started_at: Timestamp(self.started.load(Ordering::SeqCst)), clock: clock.clone() }
    }

    /// The wraps this party published that `to` would open: to its inbox,
    /// and the copies to itself when `to` shares its keys. In order.
    fn wraps_to(&self, to: &Party) -> Vec<WireEvent> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter_map(|o| match o {
                Outbound::PublishToInbox { recipient, event, .. } if *recipient == to.pk() => Some(event.clone()),
                Outbound::PublishOwn { event } if wrap_recipients(event).contains(&to.pk()) => Some(event.clone()),
                _ => None,
            })
            .collect()
    }

    /// Events of this name seen so far, newest last.
    fn events(&self, name: &str) -> Vec<serde_json::Value> {
        self.ui.lock().unwrap().iter().filter(|e| e.name == name).map(|e| e.payload.clone()).collect()
    }

    /// The call as last shown: by `call.state`, or by `call.incoming`.
    fn last_state(&self) -> Option<CallView> {
        self.ui
            .lock()
            .unwrap()
            .iter()
            .rfind(|e| e.name == UI_EVENT_CALL_STATE || e.name == UI_EVENT_CALL_INCOMING)
            .map(|e| serde_json::from_value(e.payload["call"].clone()).unwrap())
    }

    fn ended(&self) -> Vec<(String, String)> {
        self.events(UI_EVENT_CALL_ENDED)
            .iter()
            .map(|p| (p["call"]["call_id"].as_str().unwrap().to_string(), p["outcome"].as_str().unwrap().to_string()))
            .collect()
    }

    fn incoming(&self) -> Vec<CallView> {
        self.events(UI_EVENT_CALL_INCOMING).iter().map(|p| serde_json::from_value(p["call"].clone()).unwrap()).collect()
    }

    async fn records(&self) -> Vec<repo::CallRow> {
        let mut out = vec![];
        for chat in chats::list(&self.store, true).await.unwrap() {
            out.extend(repo::list(&self.store, &chat.id, 100).await.unwrap());
        }
        out
    }

    /// The call lines in the chat with `peer`, oldest first: the details.
    async fn feed(&self, peer: &Party) -> Vec<serde_json::Value> {
        let chat = chats::dm_chat_id(peer.pk().as_hex());
        self.dm
            .messages(&chat, None, 100)
            .await
            .unwrap()
            .into_iter()
            .filter(|m| m.content_type == "system" && m.text.as_deref() == Some("call"))
            .map(|m| m.media.unwrap())
            .collect()
    }
}

struct World {
    clock: Arc<TestClock>,
    parties: Vec<Arc<Party>>,
    /// One engine for all: its sessions connect to each other.
    engine: FakeEngine,
}

impl World {
    async fn new(names: &[(&'static str, Option<usize>)], gate: bool) -> Self {
        let clock = Arc::new(TestClock(AtomicI64::new(START)));
        let engine = FakeEngine::new();
        let mut parties: Vec<Arc<Party>> = vec![];
        for (name, same_as) in names {
            let keys = match same_as {
                Some(i) => parties[*i].keys.clone(),
                None => Keys::generate(),
            };
            parties.push(Arc::new(Party::new(name, keys, clock.clone(), gate, engine.clone()).await));
        }
        Self { clock, parties, engine }
    }

    fn p(&self, i: usize) -> &Arc<Party> {
        &self.parties[i]
    }

    /// The i-th session made in this world, in order of making.
    fn session(&self, i: usize) -> messenger_testkit::FakeHandle {
        self.engine.sessions()[i].clone()
    }

    fn now(&self) -> i64 {
        self.clock.0.load(Ordering::SeqCst)
    }

    /// Move the clocks on: the wall clock the cores read, and tokio's.
    /// Tokio's is paused for the jump alone: under a paused clock the
    /// store's pool times out (its wait auto-advances), so the rest of a
    /// test runs in real time, with the timers of the cores due at once.
    async fn advance(&self, secs: u64) {
        self.clock.0.fetch_add(secs as i64, Ordering::SeqCst);
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(secs)).await;
        tokio::time::resume();
        self.settle().await;
    }

    /// Carry everything until nothing moves for a few passes: a timer or
    /// an engine event is handled by a task of its own, whose writes to
    /// the store take a moment that one quiet pass may not cover.
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
                            self.carry(party, out).await;
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

    async fn carry(&self, from: &Party, out: Outbound) {
        let (event, recipients): (WireEvent, Vec<PubKey>) = match out {
            Outbound::PublishToInbox { recipient, event, .. } => (event, vec![recipient]),
            Outbound::PublishOwn { event } => {
                let r = wrap_recipients(&event);
                (event, r)
            }
            other => panic!("{}: unexpected {other:?}", from.name),
        };
        for party in &self.parties {
            if !recipients.contains(&party.pk()) || *party.deaf.lock().unwrap() {
                continue;
            }
            let Some(msg) = open_dm(&party.keys, &event, false) else { continue };
            // What the handler returns (the DM module's own answers) goes
            // the same way; the call service answers through its outlet.
            let effects = party.handler.handle(msg, &party.ctx(&self.clock)).await.unwrap();
            for e in effects {
                match e {
                    Effect::Emit(ev) => party.ui.lock().unwrap().push(ev),
                    Effect::Notify(_) => {}
                    Effect::Send(out) => Box::pin(self.carry(party, out)).await,
                }
            }
        }
    }

    /// Hand `event` to `to` as history: old, from a sync.
    async fn carry_as_history(&self, to: &Party, event: &WireEvent) {
        self.deliver(to, event, true).await;
    }

    /// Hand `event` to `to` alone, live or as history, whatever it
    /// answers carried on.
    async fn deliver(&self, to: &Party, event: &WireEvent, via_sync: bool) {
        let msg = open_dm(&to.keys, event, via_sync).unwrap();
        let effects = to.handler.handle(msg, &to.ctx(&self.clock)).await.unwrap();
        for e in effects {
            match e {
                Effect::Emit(ev) => to.ui.lock().unwrap().push(ev),
                Effect::Notify(_) => {}
                Effect::Send(out) => Box::pin(self.carry(to, out)).await,
            }
        }
    }

    /// A signal of `from` to `to`, made by hand (for what a real core
    /// would not send: a call id of one's choosing, a time of the future).
    async fn carry_made(&self, from: &Party, to: &Party, envelope: &Envelope, at: i64) {
        let content = envelope.encode();
        let w = if envelope.t == "call.invite" {
            messenger_dm::wrap::wrap_expiring(&from.keys, &to.pk(), &content, at, at + 60).unwrap()
        } else {
            messenger_dm::wrap::wrap_note(&from.keys, &to.pk(), &content, at, false, Some(at + 300)).unwrap()
        };
        self.carry(from, Outbound::PublishToInbox { recipient: to.pk(), event: w.to_peer, hint_relays: vec![] }).await;
    }

    /// Alice (0) and Bob (1) in a mutual chat, the gate on.
    async fn make_mutual(&self, a: usize, b: usize) {
        let (alice, bob) = (self.p(a), self.p(b));
        let p = alice.dm.prepare_text(&alice.keys, &bob.pk(), "hi", None).await.unwrap();
        let mut outs = vec![p.to_peer.clone()];
        outs.extend(p.followups.clone());
        for o in outs {
            self.carry(alice, o).await;
        }
        let acc = bob.dm.act(&bob.keys, &alice.pk(), Action::Accept).await.unwrap();
        for o in acc.outbounds {
            self.carry(bob, o).await;
        }
        self.settle().await;
        assert!(alice.dm.calls_allowed(&bob.pk()).await.unwrap());
        assert!(bob.dm.calls_allowed(&alice.pk()).await.unwrap());
    }
}

fn phase(p: &Party) -> Option<Phase> {
    p.last_state().map(|v| v.phase)
}

#[tokio::test]
async fn a_full_call_on_the_fake_engine() {
    let w = World::new(&[("alice", None), ("bob", None)], false).await;
    let (alice, bob) = (w.p(0), w.p(1));

    let view = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    assert_eq!(view.phase, Phase::Outgoing);
    assert_eq!(view.direction, Direction::Out);
    w.settle().await;

    // Bob's phone rings with the offer; the invitation went as a message
    // with a self-copy, so Alice's other devices would know.
    let ringing = bob.incoming();
    assert_eq!(ringing.len(), 1);
    assert_eq!(ringing[0].call_id, view.call_id);
    assert_eq!(ringing[0].phase, Phase::Incoming);
    assert_eq!(ringing[0].peer, alice.pk().as_hex());
    assert_eq!(alice.sent.lock().unwrap().len(), 2, "to the peer and to myself");
    assert!(bob.calls.current().await.is_some());
    assert_eq!(bob.records().await[0].outcome, None);

    w.advance(3).await;
    bob.calls.accept(&view.call_id).await.unwrap();
    w.settle().await;
    for p in [alice, bob] {
        let s = p.last_state().unwrap();
        assert_eq!(s.phase, Phase::Active, "{}", p.name);
        assert_eq!(s.via, Some(PairKind::Direct), "{}", p.name);
        assert!(s.answered_at.is_some());
    }
    // The engines saw each other's candidates and SDP.
    let a = w.session(0).record();
    let b = w.session(1).record();
    assert_eq!(a.remote_sdps, vec!["fake-answer:2"]);
    assert_eq!(b.remote_sdps, vec!["fake-offer:1:0"]);
    assert_eq!(a.remote_candidates.len(), 1);
    assert_eq!(b.remote_candidates.len(), 1);

    alice.calls.set_mute(true).await.unwrap();
    w.settle().await;
    assert!(alice.last_state().unwrap().muted);
    assert!(w.session(0).record().muted);

    w.advance(60).await;
    alice.calls.end(&view.call_id).await.unwrap();
    w.settle().await;
    for p in [alice, bob] {
        assert_eq!(p.ended(), vec![(view.call_id.clone(), "ended".to_string())], "{}", p.name);
        assert!(p.calls.current().await.is_none());
        let rows = p.records().await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].outcome.as_deref(), Some("ended"));
        assert_eq!(rows[0].duration_secs(), Some(60));
        assert!(!rows[0].via_relay);
    }
    assert_eq!(alice.records().await[0].direction, "out");
    assert_eq!(bob.records().await[0].direction, "in");
    assert!(w.session(0).record().closed);
    assert!(w.session(1).record().closed);

    // The line in the chat, with the facts of the call.
    let line = &bob.feed(alice).await;
    assert_eq!(line.len(), 1);
    assert_eq!(line[0]["call_id"], view.call_id);
    assert_eq!(line[0]["direction"], "in");
    assert_eq!(line[0]["media"], "audio");
    assert_eq!(line[0]["outcome"], "ended");
    assert_eq!(line[0]["duration_secs"], 60);
    assert_eq!(line[0]["via"], "direct");
    assert_eq!(alice.feed(bob).await[0]["direction"], "out");
    // A call is no message of the chat.
    let chat = chats::get(&bob.store, &chats::dm_chat_id(alice.pk().as_hex())).await.unwrap().unwrap();
    assert_eq!(chat.unread, 0);
    assert_eq!(chat.last_preview.as_deref(), Some("📞"));
}

#[tokio::test]
async fn both_call_at_once_and_the_smaller_id_wins() {
    let w = World::new(&[("alice", None), ("bob", None)], false).await;
    let (alice, bob) = (w.p(0), w.p(1));
    let a = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    let b = bob.calls.start(&alice.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    let winner = if a.call_id < b.call_id { &a.call_id } else { &b.call_id };
    let loser = if a.call_id < b.call_id { &b.call_id } else { &a.call_id };
    for p in [alice, bob] {
        let s = p.last_state().unwrap();
        assert_eq!(s.phase, Phase::Active, "{}", p.name);
        assert_eq!(&s.call_id, winner, "{}", p.name);
        let rows = p.records().await;
        assert_eq!(rows.len(), 1, "{}: the losing call left no record", p.name);
        assert_eq!(&rows[0].call_id, winner);
        assert!(p.ended().is_empty(), "{}", p.name);
        assert!(repo::get(&p.store, loser).await.unwrap().is_none());
    }
    assert_eq!(alice.feed(bob).await.len(), 1);
    assert_eq!(bob.feed(alice).await.len(), 1);
    // The loser of the glare is the one who answered.
    let (lw, _) = if winner == &a.call_id { (bob, alice) } else { (alice, bob) };
    assert!(lw.incoming().is_empty() || lw.incoming()[0].call_id == *winner);
    assert_eq!(w.engine.session_count(), 3, "two offers, and the answer of the loser");
}

#[tokio::test]
async fn the_peers_devices_both_ring_and_one_answer_is_taken() {
    let w = World::new(&[("alice", None), ("bob-1", None), ("bob-2", Some(1))], false).await;
    let (alice, bob1, bob2) = (w.p(0), w.p(1), w.p(2));
    let call = alice.calls.start(&bob1.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    assert_eq!(bob1.incoming().len(), 1);
    assert_eq!(bob2.incoming().len(), 1);

    // One device answers: the other stops ringing on the copy of the answer.
    bob1.calls.accept(&call.call_id).await.unwrap();
    w.settle().await;
    assert_eq!(phase(alice), Some(Phase::Active));
    assert_eq!(phase(bob1), Some(Phase::Active));
    assert_eq!(bob2.ended(), vec![(call.call_id.clone(), ANSWERED_ELSEWHERE.to_string())]);
    assert!(bob2.calls.current().await.is_none());
    assert_eq!(bob2.records().await[0].outcome, None, "the record is open until the call ends");
    assert!(bob2.records().await[0].answered_at.is_some());

    alice.calls.end(&call.call_id).await.unwrap();
    w.settle().await;
    assert_eq!(bob2.records().await[0].outcome.as_deref(), Some("ended"), "the other device's record closes too");
    assert_eq!(bob1.records().await[0].outcome.as_deref(), Some("ended"));

    // Both devices answer before either hears of the other: the caller
    // takes the first answer and tells the other device.
    w.advance(5).await;
    let call = alice.calls.start(&bob1.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    bob2.calls.accept(&call.call_id).await.unwrap();
    bob1.calls.accept(&call.call_id).await.unwrap();
    w.settle().await;
    assert_eq!(phase(alice), Some(Phase::Active));
    let (taken, other) = if phase(bob2) == Some(Phase::Active) { (bob2, bob1) } else { (bob1, bob2) };
    assert_eq!(phase(taken), Some(Phase::Active));
    assert_eq!(other.ended().last().unwrap(), &(call.call_id.clone(), ANSWERED_ELSEWHERE.to_string()));
    assert!(other.calls.current().await.is_none());
    assert_eq!(w.session(2).record().remote_sdps.len(), 1, "one answer taken");
}

#[tokio::test]
async fn an_old_invitation_is_a_missed_call() {
    let w = World::new(&[("alice", None), ("bob", None), ("carol", None)], false).await;
    let (alice, bob, carol) = (w.p(0), w.p(1), w.p(2));
    // Alice calls while Bob is away: nothing of it reaches him now.
    *bob.deaf.lock().unwrap() = true;
    let call = alice.calls.start(&bob.pk(), Media::Video).await.unwrap();
    w.settle().await;
    let invite = alice.sent.lock().unwrap().iter().find_map(|o| match o {
        Outbound::PublishToInbox { event, .. } => Some(event.clone()),
        _ => None,
    }).unwrap();
    *bob.deaf.lock().unwrap() = false;

    // A minute later the relay hands it over: a missed call, no ringing.
    w.advance(60).await;
    w.carry_as_history(bob, &invite).await;
    w.settle().await;
    assert!(bob.incoming().is_empty());
    assert!(bob.calls.current().await.is_none());
    assert_eq!(bob.ended(), vec![(call.call_id.clone(), "missed".to_string())]);
    let rows = bob.records().await;
    assert_eq!(rows[0].outcome.as_deref(), Some("missed"));
    assert_eq!(rows[0].media, "video");
    assert_eq!(bob.feed(alice).await[0]["outcome"], "missed");
    let chat = chats::get(&bob.store, &chats::dm_chat_id(alice.pk().as_hex())).await.unwrap().unwrap();
    assert_eq!(chat.unread, 1, "a missed call is something to see");

    // Alice gave up at the timeout: missed on her side too, told to Bob.
    assert_eq!(alice.ended(), vec![(call.call_id.clone(), "missed".to_string())]);
    assert!(alice.calls.current().await.is_none());
    let ends: Vec<String> = alice.sent.lock().unwrap().iter().filter_map(|o| match o {
        Outbound::PublishToInbox { event, .. } => open_dm(&bob.keys, event, false).map(|m| Envelope::parse(&m.content).unwrap().t),
        _ => None,
    }).collect();
    assert_eq!(ends, vec!["call.invite", "call.end"]);

    // Fresh, though it came as history (a catch-up after a short loss)
    // and was made before this session began (the push of it woke the
    // phone, the session started after): it rings all the same.
    let call2 = carol.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    *bob.deaf.lock().unwrap() = true;
    w.settle().await;
    let invite = carol.sent.lock().unwrap().iter().find_map(|o| match o {
        Outbound::PublishToInbox { event, .. } => Some(event.clone()),
        _ => None,
    }).unwrap();
    *bob.deaf.lock().unwrap() = false;
    w.advance(3).await;
    bob.started.store(w.now(), Ordering::SeqCst);
    w.carry_as_history(bob, &invite).await;
    w.settle().await;
    assert_eq!(bob.incoming().len(), 1, "a fresh invitation rings, whatever way it came");
    assert_eq!(bob.incoming()[0].call_id, call2.call_id);
    bob.calls.accept(&call2.call_id).await.unwrap();
    w.settle().await;
    assert_eq!(phase(bob), Some(Phase::Active));
    assert_eq!(phase(carol), Some(Phase::Active));
    assert_eq!(bob.records().await.iter().find(|r| r.call_id == call2.call_id).unwrap().outcome, None);
}

#[tokio::test]
async fn an_invitation_of_the_future_rings_now_and_no_longer_than_any() {
    let w = World::new(&[("alice", None), ("bob", None)], false).await;
    let (alice, bob) = (w.p(0), w.p(1));
    // Bob's clock is an hour ahead: the call is of now, by my clock, and
    // rings the 45 s any call does, not an hour.
    let id = "cd".repeat(16);
    let invite = Envelope::call_invite(&id, "audio", "fake-offer:9:0", vec![], false);
    w.carry_made(bob, alice, &invite, w.now() + 3600).await;
    w.settle().await;
    assert_eq!(alice.incoming().len(), 1);
    assert_eq!(alice.incoming()[0].started_at, w.now());
    assert_eq!(alice.records().await[0].started_at, w.now());
    w.advance(RING_TIMEOUT.as_secs() + 1).await;
    assert!(alice.calls.current().await.is_none(), "the ring timed out as any");
    assert_eq!(alice.ended(), vec![(id, "missed".to_string())]);
}

#[tokio::test]
async fn busy_decline_and_the_ring_timeout() {
    let w = World::new(&[("alice", None), ("bob", None), ("carol", None)], false).await;
    let (alice, bob, carol) = (w.p(0), w.p(1), w.p(2));

    // Declined.
    let call = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.decline(&call.call_id).await.unwrap();
    w.settle().await;
    assert_eq!(alice.ended(), vec![(call.call_id.clone(), "declined".to_string())]);
    assert_eq!(bob.ended(), vec![(call.call_id.clone(), "declined".to_string())]);
    assert_eq!(alice.records().await[0].outcome.as_deref(), Some("declined"));
    assert_eq!(bob.feed(alice).await[0]["outcome"], "declined");
    assert!(alice.calls.current().await.is_none() && bob.calls.current().await.is_none());

    // Busy: Carol calls Bob while Bob talks with Alice.
    w.advance(1).await;
    let call = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.accept(&call.call_id).await.unwrap();
    w.settle().await;
    assert_eq!(phase(bob), Some(Phase::Active));
    let other = carol.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    assert_eq!(carol.ended(), vec![(other.call_id.clone(), "busy".to_string())]);
    assert_eq!(bob.incoming().len(), 2, "only Alice's calls rang");
    assert_eq!(phase(bob), Some(Phase::Active), "Bob's call goes on");
    let busy = repo::get(&bob.store, &other.call_id).await.unwrap().unwrap();
    assert_eq!(busy.outcome.as_deref(), Some("busy"));
    assert_eq!(busy.direction, "in");
    assert_eq!(bob.feed(carol).await[0]["outcome"], "busy");
    bob.calls.end(&call.call_id).await.unwrap();
    w.settle().await;

    // Nobody answers: missed on both sides at the timeout.
    w.advance(1).await;
    let call = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    assert_eq!(phase(bob), Some(Phase::Incoming));
    w.advance(RING_TIMEOUT.as_secs() - 1).await;
    assert_eq!(phase(bob), Some(Phase::Incoming), "still ringing");
    w.advance(2).await;
    assert_eq!(alice.ended().last().unwrap(), &(call.call_id.clone(), "missed".to_string()));
    assert_eq!(bob.ended().last().unwrap(), &(call.call_id.clone(), "missed".to_string()));
    assert!(bob.calls.current().await.is_none());

    // Hanging up while it rings, on the caller's side: missed for both.
    w.advance(1).await;
    let call = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    alice.calls.end(&call.call_id).await.unwrap();
    w.settle().await;
    assert_eq!(bob.ended().last().unwrap(), &(call.call_id.clone(), "missed".to_string()));
    assert!(bob.calls.current().await.is_none());
}

#[tokio::test]
async fn the_connection_is_restarted_when_the_network_changes_and_fails_at_the_timeout() {
    let w = World::new(&[("alice", None), ("bob", None)], false).await;
    let (alice, bob) = (w.p(0), w.p(1));
    let call = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.accept(&call.call_id).await.unwrap();
    w.settle().await;
    assert_eq!(phase(alice), Some(Phase::Active));

    // The network changes under Alice while the old way still works (a
    // second interface came up): a new offer within the call at once, the
    // peer answers it, and nobody sees "reconnecting": the engine never
    // said the way was gone.
    let states_before = alice.events(UI_EVENT_CALL_STATE).len();
    w.session(0).inject(SessionEvent::NetworkChanged);
    w.settle().await;
    assert_eq!(w.session(0).record().restarts, 1);
    assert_eq!(w.session(1).record().remote_sdps, vec!["fake-offer:1:0", "fake-offer:1:1"]);
    assert_eq!(w.session(1).record().answers, 2);
    assert_eq!(phase(alice), Some(Phase::Active));
    assert_eq!(phase(bob), Some(Phase::Active));
    let seen: Vec<String> = alice.events(UI_EVENT_CALL_STATE)[states_before..].iter().map(|p| p["call"]["phase"].as_str().unwrap().to_string()).collect();
    assert!(seen.iter().all(|p| p == "active"), "the way never went: {seen:?}");
    assert_eq!(w.engine.session_count(), 2, "the same sessions, restarted");

    // The way is lost on both sides (as it is when one of them loses its
    // network): Bob asks, Alice makes the one restart offer (Bob, the
    // called side, never makes one); when that finds no way either, the
    // call fails at the timeout.
    w.engine.set_connects(false);
    w.session(0).inject(SessionEvent::ConnectionState(messenger_calls::ConnectionState::Disconnected));
    w.session(1).inject(SessionEvent::ConnectionState(messenger_calls::ConnectionState::Disconnected));
    w.settle().await;
    for p in [alice, bob] {
        let s = p.last_state().unwrap();
        assert_eq!((s.phase, s.reconnect_reason), (Phase::Reconnecting, Some(messenger_calls::ReconnectReason::ConnectionLost)), "{}", p.name);
    }
    assert_eq!(w.session(0).record().restarts, 1, "a moment is given for the way to come back");
    w.advance(LOSS_CONFIRM.as_secs() + 1).await;
    assert_eq!(w.session(1).record().restarts, 0, "the called side asks, it does not offer");
    assert_eq!(w.session(0).record().restarts, 2);
    assert_eq!(w.session(1).record().answers, 3);
    assert_eq!(phase(bob), Some(Phase::Reconnecting));
    assert_eq!(phase(alice), Some(Phase::Reconnecting));
    w.advance(CONNECT_TIMEOUT.as_secs() + 1).await;
    assert_eq!(bob.ended(), vec![(call.call_id.clone(), "failed".to_string())]);
    assert_eq!(alice.ended(), vec![(call.call_id.clone(), "failed".to_string())]);
    assert_eq!(alice.records().await[0].outcome.as_deref(), Some("failed"));
    assert!(alice.calls.current().await.is_none() && bob.calls.current().await.is_none());
}

#[tokio::test]
async fn the_called_side_recovers_from_its_own_network_change_through_the_callers_offer() {
    let w = World::new(&[("alice", None), ("bob", None)], false).await;
    let (alice, bob) = (w.p(0), w.p(1));
    let call = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.accept(&call.call_id).await.unwrap();
    w.settle().await;
    assert_eq!(phase(bob), Some(Phase::Active));

    // Bob's network changes (Wi-Fi to mobile) while the old way still
    // works: he asks at once, Alice offers again, Bob answers it; nobody
    // shows "reconnecting", the way never went.
    let states_before = bob.events(UI_EVENT_CALL_STATE).len();
    w.session(1).inject(SessionEvent::NetworkChanged);
    w.settle().await;
    assert_eq!(w.session(1).record().restarts, 0);
    assert_eq!(w.session(0).record().restarts, 1);
    assert_eq!(w.session(1).record().remote_sdps, vec!["fake-offer:1:0", "fake-offer:1:1"]);
    assert_eq!(w.session(0).record().remote_sdps, vec!["fake-answer:2", "fake-answer:2"]);
    assert_eq!(phase(alice), Some(Phase::Active));
    assert_eq!(phase(bob), Some(Phase::Active));
    let seen: Vec<String> = bob.events(UI_EVENT_CALL_STATE)[states_before..].iter().map(|p| p["call"]["phase"].as_str().unwrap().to_string()).collect();
    assert!(seen.iter().all(|p| p == "active"), "{seen:?}");
    let asked: Vec<String> = bob.wraps_to(alice).iter().filter_map(|e| open_dm(&alice.keys, e, false)).map(|m| Envelope::parse(&m.content).unwrap().t).collect();
    assert_eq!(asked, vec!["call.answer", "call.restart", "call.answer"]);
    assert_eq!(signals_of(bob, alice, "call.restart")[0]["seen"], 1, "the request says how many offers Bob has taken");

    // Both lose the way at once: both show it; after the moment given for
    // it to come back Alice offers and Bob asks, and his request (made
    // before her offer reached him) is dropped as stale: one offer, one
    // answer, and both talk again.
    w.session(0).inject(SessionEvent::ConnectionState(messenger_calls::ConnectionState::Disconnected));
    w.session(1).inject(SessionEvent::ConnectionState(messenger_calls::ConnectionState::Disconnected));
    w.settle().await;
    assert_eq!(phase(alice), Some(Phase::Reconnecting));
    assert_eq!(phase(bob), Some(Phase::Reconnecting));
    w.advance(LOSS_CONFIRM.as_secs() + 1).await;
    assert_eq!(phase(alice), Some(Phase::Active));
    assert_eq!(phase(bob), Some(Phase::Active));
    assert_eq!(w.session(0).record().restarts, 2);
    assert_eq!(w.session(1).record().answers, 3);
    assert_eq!(signals_of(bob, alice, "call.restart").len(), 2);
    assert!(alice.ended().is_empty() && bob.ended().is_empty());
    assert!(alice.last_state().unwrap().reconnect_reason.is_none());
}

#[tokio::test]
async fn the_called_side_gives_up_when_no_connection_comes_after_its_answer() {
    let w = World::new(&[("alice", None), ("bob", None)], false).await;
    let (alice, bob) = (w.p(0), w.p(1));
    let call = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    // Alice is gone by the time Bob answers: nothing ever connects.
    *alice.deaf.lock().unwrap() = true;
    bob.calls.accept(&call.call_id).await.unwrap();
    w.settle().await;
    assert_eq!(phase(bob), Some(Phase::Connecting));
    w.advance(CONNECT_TIMEOUT.as_secs() - 1).await;
    assert_eq!(phase(bob), Some(Phase::Connecting));
    w.advance(2).await;
    assert_eq!(bob.ended(), vec![(call.call_id.clone(), "failed".to_string())]);
    assert!(bob.calls.current().await.is_none());
}

#[tokio::test]
async fn an_old_connect_timer_does_not_fail_a_newer_attempt() {
    let w = World::new(&[("alice", None), ("bob", None)], false).await;
    let (alice, bob) = (w.p(0), w.p(1));
    let call = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.accept(&call.call_id).await.unwrap();
    w.settle().await;
    assert_eq!(phase(bob), Some(Phase::Active));

    // Just before the timers of the first attempt would be due, the way
    // is lost on both sides and the restart finds no way yet.
    w.advance(CONNECT_TIMEOUT.as_secs() - 4).await;
    w.engine.set_connects(false);
    w.session(0).inject(SessionEvent::ConnectionState(messenger_calls::ConnectionState::Disconnected));
    w.session(1).inject(SessionEvent::ConnectionState(messenger_calls::ConnectionState::Disconnected));
    w.settle().await;
    w.advance(LOSS_CONFIRM.as_secs() + 1).await;
    assert_eq!(w.session(0).record().restarts, 1);
    assert_eq!(phase(alice), Some(Phase::Reconnecting));
    assert_eq!(phase(bob), Some(Phase::Reconnecting));
    w.advance(4).await;
    assert_eq!(phase(alice), Some(Phase::Reconnecting), "the first attempt's timer says nothing of the second");
    assert_eq!(phase(bob), Some(Phase::Reconnecting));
    assert!(alice.ended().is_empty() && bob.ended().is_empty());
    w.advance(CONNECT_TIMEOUT.as_secs()).await;
    assert_eq!(alice.ended(), vec![(call.call_id.clone(), "failed".to_string())]);
    assert_eq!(bob.ended(), vec![(call.call_id.clone(), "failed".to_string())]);
}

/// A real loss (the engine says the way is gone on both sides, as it does
/// when one side's network goes): both show "reconnecting"; a moment
/// later the caller offers and the called side asks, the offer leaves at
/// once with its candidates trickling after it, the answer the same, and
/// both talk again. Then once more: the second loss is restored like the
/// first.
#[tokio::test]
async fn a_lost_way_is_restored_in_one_round_trip_and_again_after_the_next_loss() {
    let w = World::new(&[("alice", None), ("bob", None)], false).await;
    let (alice, bob) = (w.p(0), w.p(1));
    let call = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.accept(&call.call_id).await.unwrap();
    w.settle().await;
    w.advance(1).await;
    assert_eq!(phase(alice), Some(Phase::Active));
    let invites_before = signals_of(alice, bob, "call.invite").len();
    let ice_before = (signals_of(alice, bob, "call.ice").len(), signals_of(bob, alice, "call.ice").len());

    for round in 1..=2u32 {
        w.session(0).inject(SessionEvent::ConnectionState(messenger_calls::ConnectionState::Disconnected));
        w.session(1).inject(SessionEvent::ConnectionState(messenger_calls::ConnectionState::Disconnected));
        w.settle().await;
        for p in [alice, bob] {
            let s = p.last_state().unwrap();
            assert_eq!(s.phase, Phase::Reconnecting, "{} round {round}", p.name);
            assert_eq!(s.reconnect_reason, Some(messenger_calls::ReconnectReason::ConnectionLost));
            assert!(s.via.is_none(), "the way is unknown while it is restored");
        }
        assert_eq!(w.session(0).record().restarts, round - 1, "not before the moment is up");
        w.advance(LOSS_CONFIRM.as_secs() + 1).await;
        assert_eq!(w.session(0).record().restarts, round, "round {round}");
        assert_eq!(w.session(1).record().answers, 1 + round);
        for p in [alice, bob] {
            let s = p.last_state().unwrap();
            assert_eq!(s.phase, Phase::Active, "{} round {round}: {s:?}", p.name);
            assert_eq!(s.via, Some(PairKind::Direct), "the way is told again");
            assert!(s.reconnect_reason.is_none());
        }
        assert!(alice.ended().is_empty() && bob.ended().is_empty());
    }
    // The restart offers and answers left without waiting for candidates;
    // those followed in `call.ice` (the last batch after its debounce).
    w.advance(1).await;
    let invites = signals_of(alice, bob, "call.invite");
    assert_eq!(invites.len(), invites_before + 2);
    for inv in &invites[invites_before..] {
        assert_eq!(inv["restart"], true);
        assert_eq!(inv["ice"].as_array().map(Vec::len), Some(0), "at once, nothing gathered yet: {inv}");
    }
    let answers = signals_of(bob, alice, "call.answer");
    assert_eq!(answers.len(), 3);
    assert!(answers[1]["ice"].as_array().unwrap().is_empty() && answers[2]["ice"].as_array().unwrap().is_empty(), "{answers:?}");
    assert!(signals_of(alice, bob, "call.ice").len() >= ice_before.0 + 2, "the caller's candidates trickled");
    assert!(signals_of(bob, alice, "call.ice").len() >= ice_before.1 + 2, "the called side's candidates trickled");
    // Every trickled candidate reached the other engine.
    assert!(w.session(1).record().remote_candidates.iter().any(|c| c.candidate.contains("candidate:12 ")), "{:?}", w.session(1).record().remote_candidates);
    assert_eq!(w.session(0).record().remote_candidates.len(), 3, "one of Bob's per answer: {:?}", w.session(0).record().remote_candidates);
    // Each request of Bob's was stale by the time it came (Alice's offer
    // was on its way): one offer per loss.
    assert_eq!(signals_of(bob, alice, "call.restart").len(), 2);
}

/// The way is lost on the caller's side alone (its own network blinked):
/// the caller offers after the moment, the called side, whose engine
/// never said a word, answers without showing anything, and both talk.
/// A loss during the restart changes nothing: the one restart on its way
/// is the answer, and a flicker of the old way does not make the caller
/// forget it is waiting for that answer.
#[tokio::test]
async fn a_loss_on_the_callers_side_and_a_loss_during_the_restart() {
    let w = World::new(&[("alice", None), ("bob", None)], false).await;
    let (alice, bob) = (w.p(0), w.p(1));
    let call = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.accept(&call.call_id).await.unwrap();
    w.settle().await;
    let bob_states = bob.events(UI_EVENT_CALL_STATE).len();

    // Bob is deaf for a while: Alice's restart offer waits on the relays.
    w.session(0).inject(SessionEvent::ConnectionState(messenger_calls::ConnectionState::Disconnected));
    w.settle().await;
    assert_eq!(phase(alice), Some(Phase::Reconnecting));
    *bob.deaf.lock().unwrap() = true;
    w.advance(LOSS_CONFIRM.as_secs() + 1).await;
    assert_eq!(w.session(0).record().restarts, 1);
    let offer = alice.wraps_to(bob).into_iter().rfind(|e| open_dm(&bob.keys, e, false).is_some_and(|m| Envelope::parse(&m.content).unwrap().t == "call.invite")).unwrap();

    // The way goes again while the offer is out, and flickers back: no
    // second offer, and the answer is still awaited.
    w.session(0).inject(SessionEvent::ConnectionState(messenger_calls::ConnectionState::Disconnected));
    w.settle().await;
    w.advance(LOSS_CONFIRM.as_secs() + 1).await;
    assert_eq!(w.session(0).record().restarts, 1, "one restart per loss");
    w.session(0).inject(SessionEvent::ConnectionState(messenger_calls::ConnectionState::Connected));
    w.settle().await;
    assert_eq!(phase(alice), Some(Phase::Active), "the old way flickered back");
    w.session(0).inject(SessionEvent::ConnectionState(messenger_calls::ConnectionState::Disconnected));
    w.settle().await;
    assert_eq!(phase(alice), Some(Phase::Reconnecting));

    // The offer reaches Bob: he answers at once (his engine said nothing,
    // so his screen shows nothing), and Alice takes the answer as the
    // answer to her offer, not as a second device of Bob's.
    *bob.deaf.lock().unwrap() = false;
    w.deliver(bob, &offer, false).await;
    w.settle().await;
    assert_eq!(w.session(1).record().answers, 2);
    assert_eq!(w.session(0).record().remote_sdps, vec!["fake-answer:2", "fake-answer:2"]);
    assert_eq!(phase(alice), Some(Phase::Active));
    assert_eq!(phase(bob), Some(Phase::Active));
    let seen: Vec<String> = bob.events(UI_EVENT_CALL_STATE)[bob_states..].iter().map(|p| p["call"]["phase"].as_str().unwrap().to_string()).collect();
    assert!(seen.iter().all(|p| p == "active"), "Bob's way never went: {seen:?}");
    assert!(signals_of(alice, bob, "call.end").is_empty(), "the answer was not mistaken for another device's");
    assert!(alice.ended().is_empty() && bob.ended().is_empty());

    // A later real loss is restored again: the restart above did not use
    // up the one restart of the next loss.
    w.session(0).inject(SessionEvent::ConnectionState(messenger_calls::ConnectionState::Disconnected));
    w.session(1).inject(SessionEvent::ConnectionState(messenger_calls::ConnectionState::Disconnected));
    w.settle().await;
    w.advance(LOSS_CONFIRM.as_secs() + 1).await;
    assert_eq!(w.session(0).record().restarts, 2);
    assert_eq!(phase(alice), Some(Phase::Active));
    assert_eq!(phase(bob), Some(Phase::Active));
}

/// The relays keep no order: the called side's request for a new offer,
/// made at the loss, may reach the caller after the call is restored (the
/// answer came first). It is stale (it says fewer offers than the caller
/// made) and restarts nothing. A request without that count (an older
/// client) is honoured, but with the way still there the call stays
/// active through the restart, since the engine never says `Connected`
/// again, and the next real loss is still restored.
#[tokio::test]
async fn a_late_request_does_not_restart_a_restored_call_and_a_restart_while_connected_stays_active() {
    let w = World::new(&[("alice", None), ("bob", None)], false).await;
    let (alice, bob) = (w.p(0), w.p(1));
    let call = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.accept(&call.call_id).await.unwrap();
    w.settle().await;

    // Alice hears nothing from Bob for a while: his request and his
    // answer both wait, and come back in the wrong order.
    w.session(0).inject(SessionEvent::ConnectionState(messenger_calls::ConnectionState::Disconnected));
    w.session(1).inject(SessionEvent::ConnectionState(messenger_calls::ConnectionState::Disconnected));
    w.settle().await;
    *alice.deaf.lock().unwrap() = true;
    w.advance(LOSS_CONFIRM.as_secs() + 1).await;
    assert_eq!(w.session(0).record().restarts, 1);
    assert_eq!(w.session(1).record().answers, 2, "Bob took the offer");
    let kind = |e: &WireEvent| Envelope::parse(&open_dm(&alice.keys, e, false).unwrap().content).unwrap().t;
    let held: Vec<WireEvent> = bob.wraps_to(alice).into_iter().filter(|e| matches!(kind(e).as_str(), "call.restart" | "call.answer")).collect();
    let request = held.iter().find(|e| kind(e) == "call.restart").unwrap().clone();
    let answer = held.iter().rfind(|e| kind(e) == "call.answer").unwrap().clone();
    *alice.deaf.lock().unwrap() = false;
    w.deliver(alice, &answer, false).await;
    w.settle().await;
    assert_eq!(phase(alice), Some(Phase::Active));
    assert_eq!(phase(bob), Some(Phase::Active));

    // The stale request: nothing.
    let alice_states = alice.events(UI_EVENT_CALL_STATE).len();
    w.deliver(alice, &request, false).await;
    w.settle().await;
    assert_eq!(w.session(0).record().restarts, 1, "a request older than my latest offer restarts nothing");
    assert_eq!(alice.events(UI_EVENT_CALL_STATE).len(), alice_states);
    assert_eq!(phase(alice), Some(Phase::Active));

    // A request of an older client (no count), while the way still works
    // on both sides: honoured, the engines exchange descriptions without
    // a word of their state, and the call shows active throughout.
    w.engine.set_quiet_relink(true);
    let bob_states = bob.events(UI_EVENT_CALL_STATE).len();
    w.carry_made(bob, alice, &Envelope::call_restart(&call.call_id), w.now()).await;
    w.settle().await;
    assert_eq!(w.session(0).record().restarts, 2);
    assert_eq!(w.session(1).record().answers, 3);
    assert_eq!(w.session(0).record().remote_sdps.len(), 3, "the answer was taken");
    let seen: Vec<String> = alice.events(UI_EVENT_CALL_STATE)[alice_states..].iter().map(|p| p["call"]["phase"].as_str().unwrap().to_string()).collect();
    assert!(seen.iter().all(|p| p == "active"), "{seen:?}");
    let seen: Vec<String> = bob.events(UI_EVENT_CALL_STATE)[bob_states..].iter().map(|p| p["call"]["phase"].as_str().unwrap().to_string()).collect();
    assert!(seen.iter().all(|p| p == "active"), "{seen:?}");
    assert!(signals_of(alice, bob, "call.end").is_empty());
    w.advance(CONNECT_TIMEOUT.as_secs() + 1).await;
    assert!(alice.ended().is_empty() && bob.ended().is_empty(), "no timer judges a way that never went");
    assert_eq!(phase(alice), Some(Phase::Active));

    // The next real loss is restored as the first was.
    w.engine.set_quiet_relink(false);
    w.session(0).inject(SessionEvent::ConnectionState(messenger_calls::ConnectionState::Disconnected));
    w.session(1).inject(SessionEvent::ConnectionState(messenger_calls::ConnectionState::Disconnected));
    w.settle().await;
    assert_eq!(phase(alice), Some(Phase::Reconnecting));
    w.advance(LOSS_CONFIRM.as_secs() + 1).await;
    assert_eq!(w.session(0).record().restarts, 3);
    assert_eq!(phase(alice), Some(Phase::Active));
    assert_eq!(phase(bob), Some(Phase::Active));
}

/// The platform says the network changed (the phone's connectivity
/// callback): the call restarts at once, without the moment given to a
/// loss, on either side; nothing without a call.
#[tokio::test]
async fn the_platforms_word_of_a_network_change_restarts_at_once() {
    let w = World::new(&[("alice", None), ("bob", None)], false).await;
    let (alice, bob) = (w.p(0), w.p(1));
    alice.calls.network_changed().await;
    let call = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.accept(&call.call_id).await.unwrap();
    w.settle().await;
    // The way went on Bob's side with his network; the platform knows
    // before the engine does.
    w.session(1).inject(SessionEvent::ConnectionState(messenger_calls::ConnectionState::Disconnected));
    w.settle().await;
    bob.calls.network_changed().await;
    w.settle().await;
    assert_eq!(signals_of(bob, alice, "call.restart").len(), 1, "asked at once");
    assert_eq!(w.session(0).record().restarts, 1);
    assert_eq!(phase(bob), Some(Phase::Active));
    assert_eq!(phase(alice), Some(Phase::Active));
    let reasons: Vec<Option<messenger_calls::ReconnectReason>> = bob.events(UI_EVENT_CALL_STATE).iter().filter_map(|p| serde_json::from_value::<CallView>(p["call"].clone()).ok()).map(|v| v.reconnect_reason).collect();
    assert!(reasons.contains(&Some(messenger_calls::ReconnectReason::NetworkChanged)), "{reasons:?}");
    // And on the caller's side, with the way still there: no moment, no
    // "reconnecting", a new offer at once.
    alice.calls.network_changed().await;
    w.settle().await;
    assert_eq!(w.session(0).record().restarts, 2);
    assert_eq!(phase(alice), Some(Phase::Active));
    assert!(alice.ended().is_empty() && bob.ended().is_empty());
}

fn lost() -> SessionEvent {
    SessionEvent::ConnectionState(messenger_calls::ConnectionState::Disconnected)
}

fn phases_since(p: &Party, from: usize) -> Vec<String> {
    p.events(UI_EVENT_CALL_STATE)[from..].iter().map(|v| v["call"]["phase"].as_str().unwrap().to_string()).collect()
}

/// The called side's network changes as it answers (within the moment its
/// answer waits for candidates): its request for a restart may reach the
/// caller before the answer does. The caller, still calling, keeps the
/// request and makes the offer once the answer is taken; the called side,
/// whose first way was still being made, shows `connecting` throughout,
/// never `reconnecting`, and nothing of the call stays waiting.
#[tokio::test]
async fn a_request_that_overtakes_the_first_answer_is_honoured_with_it() {
    let w = World::new(&[("alice", None), ("bob", None)], false).await;
    let (alice, bob) = (w.p(0), w.p(1));
    let call = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    let (alice_states, bob_states) = (alice.events(UI_EVENT_CALL_STATE).len(), bob.events(UI_EVENT_CALL_STATE).len());
    *alice.deaf.lock().unwrap() = true;
    bob.calls.accept(&call.call_id).await.unwrap();
    bob.calls.network_changed().await;
    w.settle().await;
    assert_eq!(phase(bob), Some(Phase::Connecting), "the first way is still being made: not 'reconnecting'");
    let kind = |e: &WireEvent| Envelope::parse(&open_dm(&alice.keys, e, false).unwrap().content).unwrap().t;
    let held = bob.wraps_to(alice);
    let request = held.iter().find(|e| kind(e) == "call.restart").unwrap().clone();
    let answer = held.iter().find(|e| kind(e) == "call.answer").unwrap().clone();
    assert_eq!(signals_of(bob, alice, "call.restart")[0]["seen"], 1);
    *alice.deaf.lock().unwrap() = false;

    // The request first: no call to make an offer in yet, but it is kept.
    w.deliver(alice, &request, false).await;
    w.settle().await;
    assert_eq!(w.session(0).record().restarts, 0);
    assert_eq!(phase(alice), Some(Phase::Outgoing));

    // The answer: taken, and the offer asked for follows at once.
    w.deliver(alice, &answer, false).await;
    w.settle().await;
    assert_eq!(w.session(0).record().restarts, 1, "the request was honoured with the answer");
    assert_eq!(w.session(1).record().remote_sdps, vec!["fake-offer:1:0", "fake-offer:1:1"]);
    assert_eq!(w.session(1).record().answers, 2);
    assert_eq!(phase(alice), Some(Phase::Active));
    assert_eq!(phase(bob), Some(Phase::Active));
    assert!(alice.ended().is_empty() && bob.ended().is_empty());
    for (p, from) in [(alice, alice_states), (bob, bob_states)] {
        let seen = phases_since(p, from);
        assert!(!seen.iter().any(|s| s == "reconnecting"), "{}: the way never went: {seen:?}", p.name);
    }

    // Nothing waits: the caller's candidates reach the engine at once,
    // and a later loss on the called side is restored as any.
    let before = w.session(1).record().remote_candidates.len();
    let cand = serde_json::json!({ "candidate": "candidate:late 1 udp 1 203.0.113.9 5009 typ host", "mid": "0", "index": 0 });
    w.carry_made(alice, bob, &Envelope::call_ice(&call.call_id, vec![cand]), w.now()).await;
    w.settle().await;
    assert_eq!(w.session(1).record().remote_candidates.len(), before + 1);
    w.session(1).inject(lost());
    w.settle().await;
    w.advance(LOSS_CONFIRM.as_secs() + 1).await;
    assert_eq!(signals_of(bob, alice, "call.restart").len(), 2);
    assert_eq!(w.session(0).record().restarts, 2);
    assert_eq!(phase(bob), Some(Phase::Active));
    assert_eq!(phase(alice), Some(Phase::Active));
}

/// A piece of a restart may be lost on the relays (or dropped by the
/// peer). The restart is given its time ([`RESTART_SETTLE`]) and no more:
/// with the way still there the call talks on and takes the next loss as
/// any; with the way gone one more restart is made, well before the
/// connect timer would fail the call. An offer never answered makes no
/// later request stale.
#[tokio::test]
async fn a_restart_lost_on_the_relays_does_not_lock_the_call() {
    let w = World::new(&[("alice", None), ("bob", None)], false).await;
    let (alice, bob) = (w.p(0), w.p(1));
    let call = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.accept(&call.call_id).await.unwrap();
    w.settle().await;
    assert_eq!(phase(bob), Some(Phase::Active));

    // The called side's request is lost while the way still works. The
    // caller's candidates that come meanwhile wait for an offer that
    // never comes; at the deadline they go to the engine, and the next
    // loss is restored as any.
    *alice.deaf.lock().unwrap() = true;
    bob.calls.network_changed().await;
    w.settle().await;
    assert_eq!(signals_of(bob, alice, "call.restart").len(), 1);
    assert_eq!(phase(bob), Some(Phase::Active), "the way still works");
    *alice.deaf.lock().unwrap() = false;
    let before = w.session(1).record().remote_candidates.len();
    let cand = serde_json::json!({ "candidate": "candidate:late 1 udp 1 203.0.113.9 5009 typ host", "mid": "0", "index": 0 });
    w.carry_made(alice, bob, &Envelope::call_ice(&call.call_id, vec![cand]), w.now()).await;
    w.settle().await;
    assert_eq!(w.session(1).record().remote_candidates.len(), before, "kept for the offer");
    w.advance(RESTART_SETTLE.as_secs() + 1).await;
    assert_eq!(w.session(1).record().remote_candidates.len(), before + 1, "given to the engine at the deadline");
    assert_eq!(phase(bob), Some(Phase::Active));
    w.session(1).inject(lost());
    w.settle().await;
    assert_eq!(phase(bob), Some(Phase::Reconnecting));
    w.advance(LOSS_CONFIRM.as_secs() + 1).await;
    assert_eq!(signals_of(bob, alice, "call.restart").len(), 2, "asked again: the lost request holds nothing");
    assert_eq!(w.session(0).record().restarts, 1);
    assert_eq!(phase(bob), Some(Phase::Active));
    assert_eq!(phase(alice), Some(Phase::Active));

    // The caller's offer under a live way is lost: at the deadline the
    // call talks on. The called side's next request saw one offer fewer
    // than the caller made, and is honoured: the lost offer answered
    // nothing.
    *bob.deaf.lock().unwrap() = true;
    alice.calls.network_changed().await;
    w.settle().await;
    assert_eq!(w.session(0).record().restarts, 2);
    *bob.deaf.lock().unwrap() = false;
    w.advance(RESTART_SETTLE.as_secs() + 1).await;
    assert_eq!(phase(alice), Some(Phase::Active));
    w.session(1).inject(lost());
    w.settle().await;
    w.advance(LOSS_CONFIRM.as_secs() + 1).await;
    assert_eq!(signals_of(bob, alice, "call.restart").last().unwrap()["seen"], 2, "two offers taken of the three made");
    assert_eq!(w.session(0).record().restarts, 3, "not stale: the lost offer answered nothing");
    assert_eq!(phase(bob), Some(Phase::Active));
    assert_eq!(phase(alice), Some(Phase::Active));

    // The offer of a loss is lost (both sides down): one more at the
    // deadline, and the call is restored long before the connect timer.
    *bob.deaf.lock().unwrap() = true;
    w.session(0).inject(lost());
    w.session(1).inject(lost());
    w.settle().await;
    w.advance(LOSS_CONFIRM.as_secs() + 1).await;
    assert_eq!(w.session(0).record().restarts, 4);
    assert_eq!(phase(alice), Some(Phase::Reconnecting));
    *bob.deaf.lock().unwrap() = false;
    w.advance(RESTART_SETTLE.as_secs() + 1).await;
    assert_eq!(w.session(0).record().restarts, 5, "once more");
    assert_eq!(phase(alice), Some(Phase::Active));
    assert_eq!(phase(bob), Some(Phase::Active));
    assert!(alice.ended().is_empty() && bob.ended().is_empty());
    // No timer of the restarts fails the restored call later.
    w.advance(CONNECT_TIMEOUT.as_secs() + 1).await;
    assert!(alice.ended().is_empty() && bob.ended().is_empty());
    assert_eq!(phase(alice), Some(Phase::Active));
}

/// Both sides lose the way; the caller's engine says so first, and its
/// offer reaches the called side before the called side's own moment
/// ([`LOSS_CONFIRM`]) is up. That offer is the restart of this loss: the
/// called side's timer asks for no second one while the new way is still
/// being checked. The next loss is restored as any.
#[tokio::test]
async fn the_called_side_asks_for_no_second_restart_after_taking_the_offer_of_the_same_loss() {
    let w = World::new(&[("alice", None), ("bob", None)], false).await;
    let (alice, bob) = (w.p(0), w.p(1));
    let call = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.accept(&call.call_id).await.unwrap();
    w.settle().await;
    w.engine.set_quiet_relink(true);
    w.session(0).inject(lost());
    w.settle().await;
    w.session(1).inject(lost());
    w.settle().await;
    assert_eq!(phase(bob), Some(Phase::Reconnecting));
    // The caller restarts at once (the platform's word) before Bob's
    // moment is up: his engine still says nothing of the new way.
    alice.calls.network_changed().await;
    w.settle().await;
    assert_eq!(w.session(0).record().restarts, 1);
    assert_eq!(w.session(1).record().answers, 2, "the offer was taken");
    assert_eq!(phase(bob), Some(Phase::Reconnecting), "the new way is still being checked");
    w.advance(LOSS_CONFIRM.as_secs() + 1).await;
    assert!(signals_of(bob, alice, "call.restart").is_empty(), "no second restart for the same loss");
    assert_eq!(w.session(0).record().restarts, 1);
    for i in [0, 1] {
        w.session(i).inject(SessionEvent::ConnectionState(messenger_calls::ConnectionState::Connected));
    }
    w.settle().await;
    assert_eq!(phase(alice), Some(Phase::Active));
    assert_eq!(phase(bob), Some(Phase::Active));

    // The next loss on the called side gets its own restart.
    w.engine.set_quiet_relink(false);
    w.session(1).inject(lost());
    w.settle().await;
    w.advance(LOSS_CONFIRM.as_secs() + 1).await;
    assert_eq!(signals_of(bob, alice, "call.restart").len(), 1);
    assert_eq!(w.session(0).record().restarts, 2);
    assert_eq!(phase(bob), Some(Phase::Active));
    assert!(alice.ended().is_empty() && bob.ended().is_empty());
}

/// A peer of 5.1.2 says nothing of `live_restart`: on a restart while its
/// way still works it would wait for a `Connected` that never comes and
/// give up after 30 s. For such a peer no restart is made on the word of
/// a network change while the way is live; the loss, when the old way
/// goes, is restored as any. A peer of this version says the word in its
/// invitation and in its answer.
#[tokio::test]
async fn no_restart_is_made_on_a_live_way_for_a_peer_that_takes_none() {
    // An older called side: its answer carries no word.
    let w = World::new(&[("alice", None), ("bob", None)], false).await;
    let (alice, bob) = (w.p(0), w.p(1));
    let call = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    assert_eq!(signals_of(alice, bob, "call.invite")[0]["live_restart"], true);
    *alice.deaf.lock().unwrap() = true;
    bob.calls.accept(&call.call_id).await.unwrap();
    w.settle().await;
    assert_eq!(signals_of(bob, alice, "call.answer")[0]["live_restart"], true, "this version says so; what Alice takes below is an older client's");
    *alice.deaf.lock().unwrap() = false;
    w.carry_made(bob, alice, &Envelope::call_answer(&call.call_id, "fake-answer:2", vec![]), w.now()).await;
    w.settle().await;
    assert_eq!(phase(alice), Some(Phase::Active));
    assert_eq!(phase(bob), Some(Phase::Active));
    alice.calls.network_changed().await;
    w.settle().await;
    assert_eq!(w.session(0).record().restarts, 0, "the old way works, and the peer could not take a restart under it");
    assert_eq!(phase(alice), Some(Phase::Active));
    w.session(0).inject(lost());
    w.session(1).inject(lost());
    w.settle().await;
    w.advance(LOSS_CONFIRM.as_secs() + 1).await;
    assert_eq!(w.session(0).record().restarts, 1, "the loss gets its restart");
    assert_eq!(phase(alice), Some(Phase::Active));
    assert_eq!(phase(bob), Some(Phase::Active));

    // An older caller: its invitation carries no word.
    let w = World::new(&[("alice", None), ("bob", None)], false).await;
    let (alice, bob) = (w.p(0), w.p(1));
    let call = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    *bob.deaf.lock().unwrap() = true;
    w.settle().await;
    *bob.deaf.lock().unwrap() = false;
    w.carry_made(alice, bob, &Envelope::call_invite(&call.call_id, "audio", "fake-offer:1:0", vec![], false), w.now()).await;
    w.settle().await;
    assert_eq!(bob.incoming().len(), 1);
    bob.calls.accept(&call.call_id).await.unwrap();
    w.settle().await;
    assert_eq!(phase(alice), Some(Phase::Active));
    assert_eq!(phase(bob), Some(Phase::Active));
    bob.calls.network_changed().await;
    w.settle().await;
    assert!(signals_of(bob, alice, "call.restart").is_empty(), "nothing asked while the way works");
    assert_eq!(phase(bob), Some(Phase::Active));
    w.session(1).inject(lost());
    w.settle().await;
    w.advance(LOSS_CONFIRM.as_secs() + 1).await;
    assert_eq!(signals_of(bob, alice, "call.restart").len(), 1);
    assert_eq!(w.session(0).record().restarts, 1);
    assert_eq!(phase(bob), Some(Phase::Active));
    assert_eq!(phase(alice), Some(Phase::Active));
}

/// A device that takes no calls (`call.incoming_enabled` off) ignores an
/// invitation without a word: it does not ring, does not decline, is not
/// busy, keeps no record; my other devices ring and the one that takes
/// the call keeps its record. My own calls go out as before, and a glare
/// with the peer is settled as any.
#[tokio::test]
async fn a_device_that_takes_no_calls_ignores_the_invitation_without_a_word() {
    let w = World::new(&[("alice", None), ("bob-1", None), ("bob-2", Some(1))], false).await;
    let (alice, bob1, bob2) = (w.p(0), w.p(1), w.p(2));
    assert!(bob2.calls.incoming_enabled().await.unwrap());
    bob2.calls.set_incoming_enabled(false).await.unwrap();
    assert!(!bob2.calls.incoming_enabled().await.unwrap());

    let call = alice.calls.start(&bob1.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    assert_eq!(bob1.incoming().len(), 1);
    assert!(bob2.incoming().is_empty(), "no ring");
    assert!(bob2.calls.current().await.is_none());
    assert!(bob2.records().await.is_empty(), "no record");
    assert!(bob2.sent.lock().unwrap().is_empty(), "not a word");
    bob1.calls.accept(&call.call_id).await.unwrap();
    w.settle().await;
    assert_eq!(phase(bob1), Some(Phase::Active));
    assert_eq!(phase(alice), Some(Phase::Active));
    w.advance(5).await;
    alice.calls.end(&call.call_id).await.unwrap();
    w.settle().await;
    assert!(bob2.records().await.is_empty(), "the answer's and the end's copies make no record either");
    assert!(bob2.ended().is_empty());
    assert_eq!(bob1.records().await[0].outcome.as_deref(), Some("ended"));

    // Nobody else takes it: the caller's timer makes it a missed call,
    // with no decline from the device that ignores it.
    *bob1.deaf.lock().unwrap() = true;
    w.advance(1).await;
    let call = alice.calls.start(&bob1.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    assert!(bob2.sent.lock().unwrap().is_empty());
    w.advance(RING_TIMEOUT.as_secs() + 1).await;
    assert_eq!(alice.ended().last().unwrap(), &(call.call_id.clone(), "missed".to_string()));
    assert!(bob2.records().await.is_empty());
    *bob1.deaf.lock().unwrap() = false;

    // The device still calls out, and a glare with the peer is settled.
    w.advance(1).await;
    let mine = bob2.calls.start(&alice.pk(), Media::Audio).await.unwrap();
    let theirs = alice.calls.start(&bob1.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    let winner = if mine.call_id < theirs.call_id { &mine.call_id } else { &theirs.call_id };
    assert_eq!(phase(bob2), Some(Phase::Active), "{:?}", bob2.last_state());
    assert_eq!(bob2.last_state().unwrap().call_id, *winner);
    assert_eq!(phase(alice), Some(Phase::Active));
    bob2.calls.end(winner).await.unwrap();
    w.settle().await;

    // On again: it rings.
    bob2.calls.set_incoming_enabled(true).await.unwrap();
    w.advance(1).await;
    let call = alice.calls.start(&bob1.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    assert_eq!(bob2.incoming().last().map(|c| c.call_id.as_str()), Some(call.call_id.as_str()));
}

#[tokio::test]
async fn candidates_that_come_before_the_answer_wait_for_it() {
    let w = World::new(&[("alice", None), ("bob", None)], false).await;
    let (alice, bob) = (w.p(0), w.p(1));
    let call = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    // Bob's answer is held back by the relays; a candidate he found after
    // it overtakes it.
    *alice.deaf.lock().unwrap() = true;
    bob.calls.accept(&call.call_id).await.unwrap();
    w.settle().await;
    let answer = bob.wraps_to(alice).into_iter().find(|e| open_dm(&alice.keys, e, false).is_some_and(|m| Envelope::parse(&m.content).unwrap().t == "call.answer")).unwrap();
    *alice.deaf.lock().unwrap() = false;
    let late = messenger_calls::IceCandidate { candidate: "candidate:late 1 udp 1 203.0.113.9 49152 typ relay".into(), mid: Some("0".into()), index: Some(0) };
    w.session(1).inject(SessionEvent::LocalCandidate(late.clone()));
    // Late candidates leave in a batch, a moment later.
    w.settle().await;
    w.advance(1).await;
    let ice: Vec<String> = bob.wraps_to(alice).iter().filter_map(|e| open_dm(&alice.keys, e, false)).map(|m| Envelope::parse(&m.content).unwrap().t).collect();
    assert_eq!(ice, vec!["call.answer", "call.ice"]);
    assert!(!w.session(0).record().remote_candidates.contains(&late), "not given to the engine before the answer");
    assert_eq!(phase(alice), Some(Phase::Outgoing));

    // The answer comes: the engine gets it, then the candidate.
    w.deliver(alice, &answer, false).await;
    w.settle().await;
    assert_eq!(phase(alice), Some(Phase::Active));
    assert_eq!(phase(bob), Some(Phase::Active));
    assert!(w.session(0).record().remote_candidates.contains(&late));
}

#[tokio::test]
async fn the_callers_other_device_keeps_the_record_of_the_call() {
    let w = World::new(&[("alice-1", None), ("alice-2", Some(0)), ("bob", None), ("carol", None)], false).await;
    let (alice1, alice2, bob, carol) = (w.p(0), w.p(1), w.p(2), w.p(3));

    // Answered and ended: the other device's record says so, with the duration.
    let call = alice1.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    assert_eq!(alice2.records().await[0].direction, "out");
    assert_eq!(alice2.records().await[0].outcome, None);
    bob.calls.accept(&call.call_id).await.unwrap();
    w.settle().await;
    assert!(alice2.records().await[0].answered_at.is_some(), "the peer's answer is on record on every device of mine");
    assert!(alice2.calls.current().await.is_none());
    w.advance(10).await;
    alice1.calls.end(&call.call_id).await.unwrap();
    w.settle().await;
    let row = repo::get(&alice2.store, &call.call_id).await.unwrap().unwrap();
    assert_eq!(row.outcome.as_deref(), Some("ended"));
    assert_eq!(row.duration_secs(), Some(10));
    assert_eq!(alice2.feed(bob).await[0]["outcome"], "ended");

    // Declined by the peer.
    w.advance(1).await;
    let call = alice1.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.decline(&call.call_id).await.unwrap();
    w.settle().await;
    assert_eq!(repo::get(&alice2.store, &call.call_id).await.unwrap().unwrap().outcome.as_deref(), Some("declined"));
    assert_eq!(alice2.ended().last().unwrap(), &(call.call_id.clone(), "declined".to_string()));

    // The peer is busy.
    w.advance(1).await;
    let other = carol.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.accept(&other.call_id).await.unwrap();
    w.settle().await;
    let call = alice1.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    assert_eq!(alice1.ended().last().unwrap(), &(call.call_id.clone(), "busy".to_string()));
    assert_eq!(repo::get(&alice2.store, &call.call_id).await.unwrap().unwrap().outcome.as_deref(), Some("busy"));
    assert!(alice2.records().await.iter().all(|r| r.outcome.is_some()), "no record of mine is left open");
}

#[tokio::test]
async fn the_history_of_a_call_may_come_backwards() {
    let w = World::new(&[("alice", None), ("bob-1", None), ("bob-2", Some(1))], false).await;
    let (alice, bob1, bob2) = (w.p(0), w.p(1), w.p(2));
    // Bob's second device is away: Bob's first takes the call, and it ends.
    *bob2.deaf.lock().unwrap() = true;
    let call = alice.calls.start(&bob1.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    bob1.calls.accept(&call.call_id).await.unwrap();
    w.settle().await;
    w.advance(10).await;
    alice.calls.end(&call.call_id).await.unwrap();
    w.settle().await;
    *bob2.deaf.lock().unwrap() = false;

    // The catch-up hands it all over newest first: the end, the answer
    // of my other device, the invitation.
    let mut history = alice.wraps_to(bob2);
    history.extend(bob1.wraps_to(bob2));
    let mut kinds: Vec<(i64, WireEvent, String)> = history
        .into_iter()
        .filter_map(|e| open_dm(&bob2.keys, &e, true).map(|m| (m.created_at.secs(), e, Envelope::parse(&m.content).unwrap().t)))
        .collect();
    kinds.sort_by_key(|(at, _, t)| (-at, if t == "call.invite" { 1 } else { 0 }));
    let order: Vec<&str> = kinds.iter().map(|(_, _, t)| t.as_str()).collect();
    assert_eq!(order, vec!["call.end", "call.answer", "call.invite"]);
    w.advance(20).await;
    for (_, e, _) in &kinds {
        w.carry_as_history(bob2, e).await;
    }
    w.settle().await;
    assert!(bob2.incoming().is_empty());
    let row = repo::get(&bob2.store, &call.call_id).await.unwrap().unwrap();
    assert_eq!(row.outcome.as_deref(), Some("ended"), "taken on my other device, not missed");
    assert!(row.answered_at.is_some());
    assert_eq!(row.duration_secs(), Some(10));
    let chat = chats::get(&bob2.store, &chats::dm_chat_id(alice.pk().as_hex())).await.unwrap().unwrap();
    assert_eq!(chat.unread, 0);
    assert_eq!(bob2.feed(alice).await[0]["outcome"], "ended");
}

#[tokio::test]
async fn a_call_that_lost_a_glare_is_forgotten_on_every_device() {
    // Which call wins is the luck of the ids: both ways are seen.
    let (mut mine_won, mut theirs_won) = (false, false);
    for _ in 0..40 {
        let w = World::new(&[("alice-1", None), ("alice-2", Some(0)), ("bob", None)], false).await;
        let (alice1, alice2, bob) = (w.p(0), w.p(1), w.p(2));
        let a = alice1.calls.start(&bob.pk(), Media::Audio).await.unwrap();
        let b = bob.calls.start(&alice1.pk(), Media::Audio).await.unwrap();
        w.settle().await;
        let (winner, loser) = if a.call_id < b.call_id { (&a.call_id, &b.call_id) } else { (&b.call_id, &a.call_id) };
        assert_eq!(phase(alice1), Some(Phase::Active));
        assert_eq!(phase(bob), Some(Phase::Active));
        // The other device of Alice: it heard of both calls (the copy of
        // hers, the invitation of his), rang for none in the end, and has
        // the one call that was on record, answered.
        assert!(alice2.calls.current().await.is_none(), "nothing rings for a call that never was");
        let rows = alice2.records().await;
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(&rows[0].call_id, winner);
        assert!(rows[0].answered_at.is_some());
        assert_eq!(rows[0].outcome, None);
        assert!(repo::get(&alice2.store, loser).await.unwrap().is_none());
        assert!(repo::get(&bob.store, loser).await.unwrap().is_none());
        assert_eq!(alice2.feed(bob).await.len(), 1);
        if let Some((id, outcome)) = alice2.ended().last() {
            assert_eq!(outcome, ANSWERED_ELSEWHERE, "the screen alone: the call with him is on my other device");
            assert!(id == loser || id == winner);
        }
        w.advance(5).await;
        bob.calls.end(winner).await.unwrap();
        w.settle().await;
        let row = repo::get(&alice2.store, winner).await.unwrap().unwrap();
        assert_eq!(row.outcome.as_deref(), Some("ended"));
        assert_eq!(row.duration_secs(), Some(5));
        if winner == &a.call_id {
            mine_won = true;
        } else {
            theirs_won = true;
        }
        if mine_won && theirs_won {
            return;
        }
    }
    panic!("forty glares, and the ids fell the same way every time");
}

#[tokio::test]
async fn a_glare_with_other_media_rings_instead_of_answering() {
    let w = World::new(&[("alice", None), ("bob", None)], false).await;
    let (alice, bob) = (w.p(0), w.p(1));
    let mine = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    // Bob's invitation wins by its id, and asks for video I did not: my
    // call is given up, his rings as any other; nothing is answered for me.
    let theirs = "00".repeat(16);
    w.carry_made(bob, alice, &Envelope::call_invite(&theirs, "video", "fake-offer:9:0", vec![], false), w.now()).await;
    w.settle().await;
    let shown = alice.last_state().unwrap();
    assert_eq!(shown.call_id, theirs);
    assert_eq!(shown.phase, Phase::Incoming);
    assert_eq!(shown.media, Media::Video);
    assert_eq!(alice.incoming().len(), 1);
    assert!(repo::get(&alice.store, &mine.call_id).await.unwrap().is_none(), "mine never was");
    assert_eq!(w.engine.session_count(), 1, "no answer was made");
    let told: Vec<String> = alice.wraps_to(bob).iter().filter_map(|e| open_dm(&bob.keys, e, false)).map(|m| Envelope::parse(&m.content).unwrap().t).collect();
    assert_eq!(told, vec!["call.invite", "call.end"], "my other devices and his are told mine is gone");

    // With the same media the winner's call is my answer, as before.
    alice.calls.decline(&theirs).await.unwrap();
    w.settle().await;
    let mine = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    let theirs = "01".repeat(16);
    w.carry_made(bob, alice, &Envelope::call_invite(&theirs, "audio", "fake-offer:9:0", vec![], false), w.now()).await;
    w.settle().await;
    assert_eq!(alice.last_state().unwrap().call_id, theirs);
    assert_ne!(alice.last_state().unwrap().phase, Phase::Incoming);
    assert!(repo::get(&alice.store, &mine.call_id).await.unwrap().is_none());
}

#[tokio::test]
async fn a_stranger_cannot_call_and_a_contact_can() {
    let w = World::new(&[("alice", None), ("bob", None), ("carol", None)], true).await;
    let (alice, bob, carol) = (w.p(0), w.p(1), w.p(2));
    w.make_mutual(0, 1).await;

    // Carol is nobody to Bob: she may not even try, and if her client did,
    // Bob's would not ring, answer or write anything down.
    assert!(carol.calls.start(&bob.pk(), Media::Audio).await.is_err());
    assert!(carol.calls.current().await.is_none());
    let sent_before = bob.sent.lock().unwrap().len();
    let invite = Envelope::call_invite(&"ab".repeat(16), "audio", "fake-offer:9:0", vec![], false).encode();
    let wrapped = messenger_dm::wrap::wrap_expiring(&carol.keys, &bob.pk(), &invite, w.now(), w.now() + 60).unwrap();
    w.carry(carol, Outbound::PublishToInbox { recipient: bob.pk(), event: wrapped.to_peer, hint_relays: vec![] }).await;
    w.settle().await;
    assert!(bob.incoming().is_empty());
    assert!(bob.calls.current().await.is_none());
    assert!(bob.records().await.is_empty());
    assert_eq!(bob.sent.lock().unwrap().len(), sent_before, "not a word back");

    // Alice and Bob talk: the call goes through.
    let call = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    assert_eq!(bob.incoming().len(), 1);
    bob.calls.accept(&call.call_id).await.unwrap();
    w.settle().await;
    assert_eq!(phase(alice), Some(Phase::Active));
    assert_eq!(phase(bob), Some(Phase::Active));
}

#[tokio::test]
async fn relay_only_without_a_node_fails_and_the_policy_is_kept() {
    let w = World::new(&[("alice", None), ("bob", None)], false).await;
    let (alice, bob) = (w.p(0), w.p(1));
    assert_eq!(alice.calls.policy().await.unwrap(), RelayPolicy::Auto);
    alice.calls.set_policy(RelayPolicy::RelayOnly).await.unwrap();
    assert_eq!(alice.calls.policy().await.unwrap(), RelayPolicy::RelayOnly);
    let err = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap_err();
    assert!(err.to_string().contains("relay"), "{err}");
    w.settle().await;
    assert_eq!(alice.ended().last().map(|(_, o)| o.as_str()), Some("failed"));
    assert!(alice.calls.current().await.is_none());
    assert!(alice.sent.lock().unwrap().is_empty(), "no invitation left");
    assert!(bob.incoming().is_empty());
    let _ = Outcome::Failed;
}

/// The signals of this kind `from` sent `to` so far, in order.
fn signals_of(from: &Party, to: &Party, t: &str) -> Vec<serde_json::Value> {
    from.wraps_to(to)
        .iter()
        .filter_map(|e| open_dm(&to.keys, e, false))
        .filter_map(|m| Envelope::parse(&m.content).ok())
        .filter(|e| e.t == t)
        .map(|e| serde_json::to_value(e.fields).unwrap())
        .collect()
}

/// The word for the push server on the outside of every wrap `party` sent
/// so far that `peer` or `party` itself would open: `(t, restart, copy,
/// word)`, the copy `"peer"` or `"self"`, the word `"silent"` or
/// `"call:<value>"`. A wrap carries one word, never both.
fn outside_words(party: &Party, peer: &Party) -> Vec<(String, bool, &'static str, String)> {
    let word = |e: &WireEvent| -> String {
        let ev: nostr::event::Event = serde_json::from_value(e.json.clone()).unwrap();
        let tag = |name: &str| ev.tags.iter().find(|t| t.kind() == name).map(|t| t.as_slice()[1].clone());
        match (tag("silent"), tag("call")) {
            (Some(_), None) => "silent".into(),
            (None, Some(v)) => format!("call:{v}"),
            (s, c) => panic!("one word on a wrap: silent={s:?} call={c:?}"),
        }
    };
    party
        .sent
        .lock()
        .unwrap()
        .iter()
        .filter_map(|o| match o {
            Outbound::PublishToInbox { event, .. } => Some((open_dm(&peer.keys, event, false)?, "peer", word(event))),
            Outbound::PublishOwn { event } => Some((open_dm(&party.keys, event, false)?, "self", word(event))),
            _ => None,
        })
        .map(|(m, copy, w)| {
            let e = Envelope::parse(&m.content).unwrap();
            let restart = e.fields.get("restart").and_then(|v| v.as_bool()).unwrap_or(false);
            (e.t, restart, copy, w)
        })
        .collect()
}

/// What the push server reads off the outside of the signals (wire §6,
/// §10): the invitation rings (`["call", "1"]`), the signals that end a
/// ringing say so (`["call", "0"]`) on the copy to my own devices (answer,
/// refusal, busy, end) and on the peer's copy (refusal, busy, end: the
/// caller's phone may hold the ringing from a push); everything else,
/// the peer's copy of an answer included, is silent.
#[tokio::test]
async fn the_signals_that_end_a_ringing_are_marked_outside() {
    let w = World::new(&[("alice", None), ("bob", None), ("carol", None)], false).await;
    let (alice, bob, carol) = (w.p(0), w.p(1), w.p(2));

    // Taken and ended by the caller.
    let call = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.accept(&call.call_id).await.unwrap();
    w.settle().await;
    alice.calls.end(&call.call_id).await.unwrap();
    w.settle().await;
    // Declined.
    let call = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.decline(&call.call_id).await.unwrap();
    w.settle().await;
    // Busy: Bob talks to Carol when Alice calls.
    let other = carol.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.accept(&other.call_id).await.unwrap();
    w.settle().await;
    alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;

    let mut seen = std::collections::BTreeSet::new();
    for (t, restart, copy, word) in outside_words(alice, bob).into_iter().chain(outside_words(bob, alice)) {
        let expected = match (t.as_str(), copy) {
            ("call.invite", "peer") if !restart => "call:1",
            ("call.invite", "self") if !restart => "silent",
            ("call.answer", "peer") => "silent",
            ("call.answer", "self") => "call:0",
            ("call.decline" | "call.busy" | "call.end", _) => "call:0",
            _ => "silent",
        };
        assert_eq!(word, expected, "{t} (restart {restart}), the {copy}'s copy");
        seen.insert((t, copy));
    }
    for want in [
        ("call.invite", "peer"),
        ("call.invite", "self"),
        ("call.answer", "peer"),
        ("call.answer", "self"),
        ("call.end", "peer"),
        ("call.end", "self"),
        ("call.decline", "peer"),
        ("call.decline", "self"),
        ("call.busy", "peer"),
        ("call.busy", "self"),
    ] {
        assert!(seen.contains(&(want.0.to_string(), want.1)), "{want:?} was sent: {seen:?}");
    }
}

/// My camera goes on and off in the middle of an audio call: the engine is
/// asked without a new offer, the peer is told by `call.video`, the sizes
/// come from the frames; a phone switches between its two cameras; a
/// screen is a video too.
#[tokio::test]
async fn video_goes_on_and_off_in_the_middle_of_a_call() {
    let w = World::new(&[("alice", None), ("bob", None)], false).await;
    let (alice, bob) = (w.p(0), w.p(1));
    let view = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    assert!(!view.video_local && !view.video_remote);
    w.settle().await;
    bob.calls.accept(&view.call_id).await.unwrap();
    w.settle().await;
    for p in [alice, bob] {
        let s = p.last_state().unwrap();
        assert!(s.phase == Phase::Active && !s.video_local && !s.video_remote, "{}: {s:?}", p.name);
    }
    assert!(w.session(0).record().video.is_empty(), "an audio call asks the engine for no video");
    assert!(alice.calls.cameras().await.is_empty(), "the fake lists no camera: a phone");

    // Alice's camera, at the default size and cap.
    let v = alice.calls.set_video(VideoInput::Camera { id: None }).await.unwrap();
    assert!(v.video_local && !v.video_screen && v.camera.is_none());
    w.settle().await;
    let r = w.session(0).record();
    assert_eq!(r.video, vec![VideoInput::Camera { id: None }]);
    assert_eq!(r.video_settings, Some(VideoSettings { width: 640, height: 360, fps: VIDEO_FPS, max_kbps: Some(800) }));
    assert_eq!((r.offers, r.restarts), (1, 0), "nothing renegotiated");
    let s = bob.last_state().unwrap();
    assert!(s.video_remote && s.video_remote_size.is_none(), "told by call.video, no frames yet: {s:?}");
    assert!(!s.video_local);
    assert_eq!(signals_of(alice, bob, "call.video"), vec![serde_json::json!({ "call_id": view.call_id, "on": true })]);

    // The frames say their size (a phone held upright at Alice's).
    w.session(1).inject(SessionEvent::VideoSize { track: VideoTrack::Remote, width: 360, height: 640 });
    w.session(0).inject(SessionEvent::VideoSize { track: VideoTrack::Local, width: 360, height: 640 });
    w.settle().await;
    assert_eq!(bob.last_state().unwrap().video_remote_size, Some(VideoSize { width: 360, height: 640 }));
    assert_eq!(alice.last_state().unwrap().video_local_size, Some(VideoSize { width: 360, height: 640 }));
    let states_before = alice.events(UI_EVENT_CALL_STATE).len();
    w.session(0).inject(SessionEvent::VideoSize { track: VideoTrack::Local, width: 360, height: 640 });
    w.settle().await;
    assert_eq!(alice.events(UI_EVENT_CALL_STATE).len(), states_before, "the same size again says nothing");

    // The other camera of a phone (the engine lists none: front, back).
    let v = alice.calls.switch_camera(None).await.unwrap();
    assert_eq!(v.camera.as_deref(), Some("back"));
    assert!(v.video_local);
    assert_eq!(w.session(0).record().video.last(), Some(&VideoInput::Camera { id: Some("back".into()) }));
    let v = alice.calls.switch_camera(None).await.unwrap();
    assert_eq!(v.camera.as_deref(), Some("front"));
    w.settle().await;
    assert_eq!(signals_of(alice, bob, "call.video").len(), 1, "a switch is no news to the peer");

    // The cap follows the node once the way is relayed.
    w.session(0).inject(SessionEvent::SelectedPair(PairKind::Relay));
    w.settle().await;
    assert_eq!(w.session(0).record().video_bitrate.last(), Some(&Some(800)), "no node limits: the size's own cap");

    // Off: the peer is told, the sizes go.
    let v = alice.calls.set_video(VideoInput::Off).await.unwrap();
    assert!(!v.video_local && v.video_local_size.is_none());
    assert_eq!(v.camera.as_deref(), Some("front"), "the camera is kept for the next time");
    w.settle().await;
    let s = bob.last_state().unwrap();
    assert!(!s.video_remote && s.video_remote_size.is_none(), "{s:?}");
    assert_eq!(signals_of(alice, bob, "call.video").len(), 2);
    assert_eq!(w.session(0).record().video.last(), Some(&VideoInput::Off));

    // 720p for the next time; Bob shares his screen at it.
    bob.calls.set_video_quality(VideoQuality::Hd).await.unwrap();
    assert_eq!(bob.calls.video_quality().await.unwrap(), VideoQuality::Hd);
    let v = bob.calls.set_video(VideoInput::Screen { id: Some("screen:1".into()) }).await.unwrap();
    assert!(v.video_local && v.video_screen);
    w.settle().await;
    assert!(alice.last_state().unwrap().video_remote);
    let r = w.session(1).record();
    assert_eq!(r.video, vec![VideoInput::Screen { id: Some("screen:1".into()) }]);
    assert_eq!(r.video_settings.map(|s| (s.width, s.height, s.max_kbps)), Some((1280, 720, Some(1800))));
    // A switch of the camera while the screen goes keeps the screen and
    // remembers the camera.
    let v = bob.calls.switch_camera(Some("back".into())).await.unwrap();
    assert!(v.video_screen && v.camera.as_deref() == Some("back"));
    assert_eq!(w.session(1).record().video.len(), 1);

    // A camera that will not open: an error, and the video is off at both.
    w.engine.set_camera_fails(true);
    assert!(bob.calls.set_video(VideoInput::Camera { id: None }).await.is_err());
    w.settle().await;
    assert!(!bob.last_state().unwrap().video_local);
    assert!(!alice.last_state().unwrap().video_remote);

    // The frames of the call under way, for the screen; none without one.
    assert!(alice.calls.video_frames(VideoTrack::Remote).await.is_some());
    alice.calls.end(&view.call_id).await.unwrap();
    w.settle().await;
    assert!(alice.calls.video_frames(VideoTrack::Remote).await.is_none());
    assert!(matches!(alice.calls.set_video(VideoInput::Camera { id: None }).await, Err(messenger_core::MessengerError::Invalid(_))));
    assert!(w.session(0).record().closed && w.session(1).record().closed);
}

/// A video call: the caller's camera goes on with the invitation, the
/// called side's as it takes the call. A camera that fails at the start
/// leaves an audio call: the invitation still says video (nothing else
/// was known when it left), and `call.video {on: false}` follows it.
#[tokio::test]
async fn a_video_call_turns_the_cameras_on_and_a_failed_one_leaves_an_audio_call() {
    let w = World::new(&[("alice", None), ("bob", None)], false).await;
    let (alice, bob) = (w.p(0), w.p(1));
    let view = alice.calls.start(&bob.pk(), Media::Video).await.unwrap();
    assert!(view.video_local && view.video_remote, "my camera from the start, theirs expected: {view:?}");
    assert_eq!(w.session(0).record().video, vec![VideoInput::Camera { id: None }]);
    w.settle().await;
    let ringing = bob.incoming();
    assert!(ringing[0].video_remote && !ringing[0].video_local, "{:?}", ringing[0]);
    assert!(signals_of(alice, bob, "call.video").is_empty(), "the invitation said it");
    bob.calls.accept(&view.call_id).await.unwrap();
    w.settle().await;
    for p in [alice, bob] {
        let s = p.last_state().unwrap();
        assert!(s.phase == Phase::Active && s.video_local && s.video_remote, "{}: {s:?}", p.name);
    }
    assert_eq!(w.session(1).record().video, vec![VideoInput::Camera { id: None }]);
    alice.calls.end(&view.call_id).await.unwrap();
    w.settle().await;

    // The camera fails at the caller.
    w.engine.set_camera_fails(true);
    let view = alice.calls.start(&bob.pk(), Media::Video).await.unwrap();
    assert!(!view.video_local && view.media == Media::Video, "{view:?}");
    w.settle().await;
    assert!(alice.events("error").iter().any(|e| e["scope"] == "calls"), "the failure is reported");
    let ringing = bob.incoming();
    assert_eq!(ringing.len(), 2);
    assert_eq!(ringing[1].media, Media::Video, "the invitation left as made");
    let s = bob.last_state().unwrap();
    assert!(!s.video_remote, "call.video off followed the invitation: {s:?}");
    assert_eq!(signals_of(alice, bob, "call.video").last(), Some(&serde_json::json!({ "call_id": view.call_id, "on": false })));
    // Bob takes it: his camera fails too (the same fake), the call goes
    // on as an audio one between the two, said so both ways.
    bob.calls.accept(&view.call_id).await.unwrap();
    w.settle().await;
    for p in [alice, bob] {
        let s = p.last_state().unwrap();
        assert!(s.phase == Phase::Active && !s.video_local && !s.video_remote, "{}: {s:?}", p.name);
    }
    alice.calls.end(&view.call_id).await.unwrap();
    w.settle().await;
}

/// The relays keep no order: the caller's `call.video {on: false}` (its
/// camera failed) may reach the called side before the invitation that
/// says `video`. The word waits for the invitation, as an answer or an
/// end would, and the call rings as an audio one; a word of a call that
/// is over is dropped.
#[tokio::test]
async fn the_peers_word_of_its_video_that_overtakes_the_invitation_is_kept_for_it() {
    let w = World::new(&[("alice", None), ("bob", None)], false).await;
    let (alice, bob) = (w.p(0), w.p(1));
    w.engine.set_camera_fails(true);
    *bob.deaf.lock().unwrap() = true;
    let view = alice.calls.start(&bob.pk(), Media::Video).await.unwrap();
    w.settle().await;
    let wraps = alice.wraps_to(bob);
    let kind = |e: &WireEvent| Envelope::parse(&open_dm(&bob.keys, e, false).unwrap().content).unwrap().t;
    let kinds: Vec<String> = wraps.iter().map(kind).collect();
    assert_eq!(kinds, vec!["call.invite", "call.video"], "the invitation, then the word of the failed camera");
    *bob.deaf.lock().unwrap() = false;

    // The word first, then the invitation.
    w.deliver(bob, &wraps[1], false).await;
    assert!(bob.incoming().is_empty(), "a word alone rings nothing");
    w.deliver(bob, &wraps[0], false).await;
    w.settle().await;
    let ringing = bob.incoming();
    assert_eq!(ringing.len(), 1);
    assert_eq!(ringing[0].media, Media::Video, "the invitation as made");
    assert!(!ringing[0].video_remote, "but the caller's camera is known to be off: {:?}", ringing[0]);
    assert!(!bob.last_state().unwrap().video_remote);
    bob.calls.accept(&view.call_id).await.unwrap();
    w.settle().await;
    let s = bob.last_state().unwrap();
    assert!(s.phase == Phase::Active && !s.video_remote, "{s:?}");
    assert_eq!(signals_of(alice, bob, "call.video").len(), 1, "the caller has nothing more to say");

    // The same word for a call that is over: dropped, nothing kept.
    alice.calls.end(&view.call_id).await.unwrap();
    w.settle().await;
    w.deliver(bob, &wraps[1], false).await;
    w.settle().await;
    assert_eq!(bob.incoming().len(), 1);
    assert!(bob.calls.current().await.is_none());
    assert_eq!(bob.ended().len(), 1, "{:?}", bob.ended());
}

/// My camera goes away in the middle of the call (unplugged: the engine
/// says `VideoLost`): my video is off as if I had turned it off, the peer
/// is told by `call.video {on: false}`, the screen hears why. A loss when
/// my video is off already says nothing.
#[tokio::test]
async fn a_camera_that_goes_away_turns_my_video_off_and_tells_the_peer() {
    let w = World::new(&[("alice", None), ("bob", None)], false).await;
    let (alice, bob) = (w.p(0), w.p(1));
    let view = alice.calls.start(&bob.pk(), Media::Audio).await.unwrap();
    w.settle().await;
    bob.calls.accept(&view.call_id).await.unwrap();
    w.settle().await;
    alice.calls.set_video(VideoInput::Camera { id: None }).await.unwrap();
    w.settle().await;
    w.session(1).inject(SessionEvent::VideoSize { track: VideoTrack::Remote, width: 640, height: 360 });
    w.settle().await;
    let s = bob.last_state().unwrap();
    assert!(s.video_remote && s.video_remote_size.is_some(), "{s:?}");
    assert_eq!(signals_of(alice, bob, "call.video").len(), 1);
    let errors_before = alice.events("error").len();

    w.session(0).inject(SessionEvent::VideoLost { reason: "unplugged".into() });
    w.settle().await;
    let s = alice.last_state().unwrap();
    assert!(!s.video_local && s.video_local_size.is_none() && s.phase == Phase::Active, "{s:?}");
    assert_eq!(w.session(0).record().video.last(), Some(&VideoInput::Off), "the engine is told off");
    assert_eq!(signals_of(alice, bob, "call.video").last(), Some(&serde_json::json!({ "call_id": view.call_id, "on": false })));
    let s = bob.last_state().unwrap();
    assert!(!s.video_remote && s.video_remote_size.is_none(), "{s:?}");
    let errors = alice.events("error");
    assert_eq!(errors.len(), errors_before + 1);
    assert_eq!(errors.last().unwrap()["scope"], "calls");
    assert!(errors.last().unwrap()["error"].to_string().contains("unplugged"), "{:?}", errors.last());

    // Off already: a loss changes nothing and says nothing.
    let videos = w.session(0).record().video.len();
    w.session(0).inject(SessionEvent::VideoLost { reason: "again".into() });
    w.settle().await;
    assert_eq!(w.session(0).record().video.len(), videos);
    assert_eq!(alice.events("error").len(), errors_before + 1);
    assert_eq!(signals_of(alice, bob, "call.video").len(), 2);
    alice.calls.end(&view.call_id).await.unwrap();
    w.settle().await;
}
