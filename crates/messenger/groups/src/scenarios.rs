// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Whole scenarios: several people, several devices, one relay that
//! delivers late, twice, and in the wrong order.

use crate::keys::GroupLink;
use crate::op::{GroupKind, KeyId, OpBody};
use crate::roles::Role;
use crate::service::*;
use async_trait::async_trait;
use messenger_contacts::{ContactService, ProfileService};
use messenger_core::inbound::Envelope as WireEnvelope;
use messenger_core::outbound::WireEvent;
use messenger_core::{
    Clock, Context, DmInbound, EventId, EventSource, GroupInbound, Outbound, PubKey, RelayUrl, Result, SecretStore, Timestamp,
};
use messenger_dm::{pushtags, DmService, MessageView};
use messenger_store::Store;
use nostr::key::Keys;
use nostr::nips::nip59::UnwrappedGift;
use nostr::prelude::Event;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use zeroize::Zeroizing;

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.0.load(Ordering::SeqCst))
    }
}

#[derive(Default)]
struct Secrets(Mutex<HashMap<String, Vec<u8>>>);
#[async_trait]
impl SecretStore for Secrets {
    async fn get(&self, key: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        Ok(self.0.lock().unwrap().get(key).cloned().map(Zeroizing::new))
    }
    async fn put(&self, key: &str, value: &[u8]) -> Result<()> {
        self.0.lock().unwrap().insert(key.into(), value.to_vec());
        Ok(())
    }
    async fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
    async fn is_unlocked(&self) -> bool {
        true
    }
}

struct Device {
    keys: Keys,
    svc: GroupService,
    dm: DmService,
    online: bool,
    /// What the relay holds for this device while it is away.
    missed: Vec<Wire>,
    clock: Arc<TestClock>,
}

#[derive(Clone)]
enum Wire {
    Group(WireEvent),
    Dm(WireEvent),
}

impl Device {
    fn pk(&self) -> PubKey {
        me_of(&self.keys)
    }
    fn ctx(&self) -> Context {
        Context { my_pubkey: self.pk(), session_started_at: Timestamp(1_000), clock: self.clock.clone() }
    }
    async fn group(&self, id: &str) -> Option<GroupView> {
        self.svc.get(id, &self.pk()).await.unwrap()
    }
    async fn texts(&self, id: &str) -> Vec<String> {
        self.visible(id).await.into_iter().filter(|m| m.content_type == "text" && !m.deleted).filter_map(|m| m.text).collect()
    }
    async fn visible(&self, id: &str) -> Vec<MessageView> {
        let mut v = self.dm.messages(&format!("group:{id}"), None, 500).await.unwrap();
        v.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        v
    }
    async fn lines(&self, id: &str) -> Vec<String> {
        self.visible(id).await.into_iter().filter(|m| m.content_type == "system").filter_map(|m| m.text).collect()
    }
}

struct World {
    clock: Arc<TestClock>,
    devices: Vec<Device>,
    /// Every group event ever published, as the relay keeps them.
    relay: Vec<WireEvent>,
    notes: Vec<String>,
}

const RELAY: &str = "wss://relay.example";

impl World {
    fn new() -> Self {
        Self { clock: Arc::new(TestClock(AtomicI64::new(2_000))), devices: vec![], relay: vec![], notes: vec![] }
    }

    async fn device(&mut self, keys: Keys) -> usize {
        let store = Store::open_in_memory().await.unwrap();
        let profiles = ProfileService::new(store.clone());
        let contacts = ContactService::new(store.clone(), profiles.clone());
        let dm = DmService::new(store.clone(), contacts, profiles, self.clock.clone());
        dm.set_gate(false);
        // Knows whose it is, so that its own reactions show as mine.
        dm.set_signer(Some(keys.clone()));
        let svc = GroupService::new(store, Arc::new(Secrets::default()), self.clock.clone(), dm.clone());
        self.devices.push(Device { keys, svc, dm, online: true, missed: vec![], clock: self.clock.clone() });
        self.devices.len() - 1
    }

    async fn person(&mut self) -> usize {
        self.device(Keys::generate()).await
    }

    fn tick(&self) {
        self.clock.0.fetch_add(10, Ordering::SeqCst);
    }

    fn pk(&self, i: usize) -> PubKey {
        self.devices[i].pk()
    }

    /// Carry out everything an action led to, and everything that leads to.
    async fn run(&mut self, from: usize, outcome: Outcome) {
        let mut queue: VecDeque<(usize, Outcome)> = VecDeque::from([(from, outcome)]);
        let mut guard = 0;
        while let Some((origin, mut outcome)) = queue.pop_front() {
            guard += 1;
            assert!(guard < 2_000, "the world does not settle");
            self.notes.append(&mut outcome.notes);
            for g in std::mem::take(&mut outcome.maintain) {
                self.tick();
                let d = &self.devices[origin];
                let o = d.svc.maintain(&d.keys, &g).await.unwrap();
                queue.push_back((origin, o));
            }
            for out in outcome.publish {
                let (wire, to): (Wire, Vec<usize>) = match out {
                    Outbound::PublishScoped { event, .. } => {
                        self.relay.push(event.clone());
                        (Wire::Group(event), (0..self.devices.len()).collect())
                    }
                    Outbound::PublishToInbox { recipient, event, .. } => {
                        let to = (0..self.devices.len()).filter(|i| self.pk(*i) == recipient).collect();
                        (Wire::Dm(event), to)
                    }
                    Outbound::PublishOwn { event } => {
                        let me = self.pk(origin);
                        (Wire::Dm(event), (0..self.devices.len()).filter(|i| self.pk(*i) == me).collect())
                    }
                    other => panic!("unexpected {other:?}"),
                };
                for i in to {
                    if !self.devices[i].online {
                        self.devices[i].missed.push(wire.clone());
                        continue;
                    }
                    let o = self.deliver(i, &wire, false).await;
                    queue.push_back((i, o));
                }
            }
        }
    }

    async fn deliver(&self, i: usize, wire: &Wire, via_sync: bool) -> Outcome {
        let d = &self.devices[i];
        let url = RelayUrl::parse(RELAY).unwrap();
        let source = if via_sync { EventSource::Sync { url } } else { EventSource::Relay { url } };
        match wire {
            Wire::Group(event) => {
                let ev: Event = serde_json::from_value(event.json.clone()).unwrap();
                let tag = |name: &str| ev.tags.iter().filter(|t| t.kind() == name).filter_map(|t| t.as_slice().get(1)).next().cloned();
                let msg = GroupInbound {
                    envelope: WireEnvelope { wire_id: event.id.clone(), source, wire_created_at: Timestamp(ev.created_at.as_secs() as i64) },
                    group_id: tag("h").unwrap(),
                    sender: PubKey::parse(&ev.pubkey.to_hex()).unwrap(),
                    created_at: Timestamp(ev.created_at.as_secs() as i64),
                    kind: 9,
                    key_id: tag("k"),
                    ciphertext: ev.content.clone(),
                    reply_to: None,
                };
                d.svc.on_event(&d.keys, msg, &d.ctx()).await.unwrap()
            }
            Wire::Dm(event) => {
                let ev: Event = serde_json::from_value(event.json.clone()).unwrap();
                let Ok(u) = UnwrappedGift::from_gift_wrap(&d.keys, &ev) else { return Outcome::default() };
                let mut rumor = u.rumor.clone();
                rumor.ensure_id();
                let tags = |name: &str| -> Vec<String> {
                    rumor.tags.iter().filter(|t| t.kind() == name).filter_map(|t| t.as_slice().get(1)).cloned().collect()
                };
                let msg = DmInbound {
                    envelope: WireEnvelope { wire_id: event.id.clone(), source, wire_created_at: Timestamp(ev.created_at.as_secs() as i64) },
                    rumor_id: EventId::parse(&rumor.id.unwrap().to_hex()).unwrap(),
                    sender: PubKey::parse(&u.sender.to_hex()).unwrap(),
                    recipients: tags("p").iter().filter_map(|s| PubKey::parse(s)).collect(),
                    created_at: Timestamp(rumor.created_at.as_secs() as i64),
                    content: rumor.content.clone(),
                    reply_to: None,
                    rumor_kind: rumor.kind.as_u16(),
                };
                d.svc.on_dm(&d.keys, &msg, &d.ctx()).await.unwrap().expect("a group message")
            }
        }
    }

    fn offline(&mut self, i: usize) {
        self.devices[i].online = false;
    }

    /// Back online: what was missed arrives as history, newest first.
    async fn online(&mut self, i: usize) {
        self.devices[i].online = true;
        let mut missed = std::mem::take(&mut self.devices[i].missed);
        missed.reverse();
        for w in missed {
            let o = self.deliver(i, &w, true).await;
            self.run(i, o).await;
        }
    }

    /// Ask the relay for the whole history of the groups, newest first.
    async fn catch_up(&mut self, i: usize) {
        let mut all = self.relay.clone();
        all.reverse();
        for e in all {
            let o = self.deliver(i, &Wire::Group(e), true).await;
            self.run(i, o).await;
        }
    }

    // ─── What people do ─────────────────────────────────────────────────────

    async fn create(&mut self, who: usize, kind: GroupKind, name: &str, history: bool) -> String {
        self.tick();
        let d = &self.devices[who];
        let (view, o) = d.svc.create(&d.keys, kind, name, "", history, &RelayUrl::parse(RELAY).unwrap()).await.unwrap();
        self.run(who, o).await;
        view.id
    }

    async fn say(&mut self, who: usize, group: &str, text: &str) -> String {
        self.try_say(who, group, text).await.unwrap()
    }

    async fn try_say(&mut self, who: usize, group: &str, text: &str) -> Result<String> {
        self.tick();
        let d = &self.devices[who];
        let (m, out) = d.svc.prepare_text(&d.keys, group, text, None).await?;
        self.run(who, Outcome { publish: vec![out], ..Default::default() }).await;
        Ok(m.id)
    }

    async fn act(&mut self, who: usize, group: &str, body: OpBody) -> Result<()> {
        self.tick();
        let d = &self.devices[who];
        let o = d.svc.act(&d.keys, group, body).await?;
        self.run(who, o).await;
        Ok(())
    }

    /// Invite and have the invitation accepted.
    async fn bring(&mut self, manager: usize, group: &str, guest: usize) {
        self.tick();
        let who = self.pk(guest);
        let d = &self.devices[manager];
        let (invite, o) = d.svc.invite(&d.keys, group, &who).await.unwrap();
        self.run(manager, o).await;
        self.tick();
        let g = &self.devices[guest];
        assert_eq!(g.svc.invites("in").await.unwrap().len(), 1, "the invitation arrived");
        let o = g.svc.answer_invite(&g.keys, &invite.invite_id, true).await.unwrap();
        self.run(guest, o).await;
    }

    /// The history of a group has arrived: what the runtime does then.
    async fn synced(&mut self, who: usize, group: &str) {
        let d = &self.devices[who];
        let o = d.svc.maintain(&d.keys, group).await.unwrap();
        self.run(who, o).await;
    }

    async fn open_link(&mut self, who: usize, link: &str) -> GroupView {
        self.tick();
        let d = &self.devices[who];
        let (view, o) = d.svc.open_link(&d.keys, link, "let me in").await.unwrap();
        self.run(who, o).await;
        view
    }
}

fn code<T: std::fmt::Debug>(r: Result<T>) -> String {
    match r {
        Err(messenger_core::MessengerError::Invalid(c)) => c,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn private_group_invitation_conversation_and_second_device() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let carol = w.person().await;
    let alice2 = w.device(w.devices[alice].keys.clone()).await;

    let g = w.create(alice, GroupKind::Private, "Family", true).await;
    w.say(alice, &g, "first").await;
    // The creator's other device got the group by itself.
    let v = w.devices[alice2].group(&g).await.expect("second device has the group");
    assert_eq!((v.membership.as_str(), v.my_role.as_deref()), ("joined", Some("owner")));
    assert_eq!(w.devices[alice2].texts(&g).await, vec!["first"]);

    w.bring(alice, &g, bob).await;
    let v = w.devices[bob].group(&g).await.expect("bob is in");
    assert_eq!((v.membership.as_str(), v.my_role.as_deref(), v.members.len()), ("joined", Some("member"), 2));
    assert!(v.can_post && v.link.is_none(), "a member of a private group has no link to share");
    // History is on: what was said before the invitation is for bob too.
    w.catch_up(bob).await;
    assert_eq!(w.devices[bob].texts(&g).await, vec!["first"]);

    w.say(bob, &g, "hello").await;
    w.say(alice2, &g, "from the other device").await;
    for d in [alice, alice2, bob] {
        assert_eq!(w.devices[d].texts(&g).await, vec!["first", "hello", "from the other device"], "device {d}");
        assert_eq!(w.devices[d].group(&g).await.unwrap().undecrypted, 0);
    }
    assert!(w.devices[alice].lines(&g).await.contains(&"group_admitted".to_string()));
    // Carol hears the relay but is no part of it.
    assert!(w.devices[carol].group(&g).await.is_none());
    // A member does not invite.
    let d = &w.devices[bob];
    assert_eq!(code(d.svc.invite(&d.keys, &g, &w.pk(carol)).await), "group_not_permitted");
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

#[tokio::test]
async fn history_is_closed_when_the_group_says_so() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Closed", false).await;
    w.say(alice, &g, "before bob").await;
    w.bring(alice, &g, bob).await;
    w.catch_up(bob).await;
    w.say(alice, &g, "after bob").await;
    assert_eq!(w.devices[bob].texts(&g).await, vec!["after bob"]);
    assert_eq!(w.devices[alice].texts(&g).await, vec!["before bob", "after bob"]);
}

#[tokio::test]
async fn removal_changes_the_key_and_leaves_the_removed_outside() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let carol = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Team", true).await;
    w.bring(alice, &g, bob).await;
    w.bring(alice, &g, carol).await;
    w.say(bob, &g, "still here").await;
    let old = w.devices[alice].svc.need_log(&g).await.unwrap().state().current_key.clone().unwrap();

    w.act(alice, &g, OpBody::Remove { who: w.pk(bob) }).await.unwrap();
    let new = w.devices[alice].svc.need_log(&g).await.unwrap().state().current_key.clone().unwrap();
    assert_ne!(old, new);
    assert_eq!(w.devices[bob].group(&g).await.unwrap().membership, "removed");
    assert!(w.devices[bob].svc.key(&g, &new).await.unwrap().is_none(), "the new key never reached bob");
    assert!(w.devices[carol].svc.key(&g, &new).await.unwrap().is_some());
    assert_eq!(code(w.try_say(bob, &g, "let me speak").await), "group_not_member");

    w.say(alice, &g, "without bob").await;
    assert_eq!(w.devices[carol].texts(&g).await, vec!["still here", "without bob"]);
    assert_eq!(w.devices[bob].texts(&g).await, vec!["still here"]);
    // What bob said while a member stays.
    assert_eq!(w.devices[alice].texts(&g).await, vec!["still here", "without bob"]);
}

/// The group's marks on the newest event the relay holds.
fn marks_of_the_last(w: &World) -> (String, Vec<String>) {
    let event = &w.relay.last().unwrap().json;
    let marks = event["tags"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t[0] == pushtags::GROUP)
        .map(|t| t[1].as_str().unwrap().to_string())
        .collect();
    (event["pubkey"].as_str().unwrap().to_string(), marks)
}

#[tokio::test]
async fn for_a_week_after_a_change_of_the_key_the_key_before_marks_messages_too() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let carol = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Team", true).await;
    w.bring(alice, &g, bob).await;
    w.bring(alice, &g, carol).await;
    let push_key = |w: &World, who: usize, id: KeyId| {
        let svc = w.devices[who].svc.clone();
        let g = g.clone();
        async move { pushtags::group_push_key(svc.key(&g, &id).await.unwrap().unwrap().as_bytes()) }
    };
    let current = |w: &World| {
        let svc = w.devices[alice].svc.clone();
        let g = g.clone();
        async move { svc.need_log(&g).await.unwrap().state().current_key.clone().unwrap() }
    };

    // One key so far: one mark, and one key for the push server.
    let old = push_key(&w, alice, current(&w).await).await;
    w.say(bob, &g, "hello").await;
    let (signer, marks) = marks_of_the_last(&w);
    assert_eq!(marks, vec![pushtags::group_mark(&old, &signer).unwrap()]);
    assert_eq!(w.devices[carol].svc.push_keys(&g).await.unwrap(), vec![old.clone()]);

    // Bob is removed and the key changes. Carol's phone may have slept
    // through it, with only the old key told to its server: what is said
    // now carries both marks, and those who know the change take both.
    w.act(alice, &g, OpBody::Remove { who: w.pk(bob) }).await.unwrap();
    let new = push_key(&w, alice, current(&w).await).await;
    assert_ne!(new, old);
    w.say(alice, &g, "without bob").await;
    let (signer, marks) = marks_of_the_last(&w);
    assert_eq!(marks, vec![pushtags::group_mark(&new, &signer).unwrap(), pushtags::group_mark(&old, &signer).unwrap()]);
    assert_eq!(w.devices[carol].svc.push_keys(&g).await.unwrap(), vec![new.clone(), old.clone()]);

    // A week later the old key serves nobody but those who left with it.
    w.clock.0.fetch_add(pushtags::GROUP_GRACE_SECS, Ordering::SeqCst);
    w.say(carol, &g, "a week later").await;
    let (signer, marks) = marks_of_the_last(&w);
    assert_eq!(marks, vec![pushtags::group_mark(&new, &signer).unwrap()]);
    assert_eq!(w.devices[alice].svc.push_keys(&g).await.unwrap(), vec![new.clone()]);
    assert_eq!(w.devices[alice].texts(&g).await, vec!["hello", "without bob", "a week later"]);
}

#[tokio::test]
async fn a_member_leaves_and_a_manager_brings_a_new_key() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Team", true).await;
    w.bring(alice, &g, bob).await;
    let old = w.devices[alice].svc.need_log(&g).await.unwrap().state().current_key.clone().unwrap();
    w.act(bob, &g, OpBody::Leave).await.unwrap();
    assert_eq!(w.devices[bob].group(&g).await.unwrap().membership, "left");
    let log = w.devices[alice].svc.need_log(&g).await.unwrap();
    assert_eq!(log.state().members.len(), 1);
    assert_ne!(log.state().current_key.clone().unwrap(), old, "alice's device rotated by itself");
    assert!(!log.state().key_stale);
}

#[tokio::test]
async fn request_through_a_private_link() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let carol = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Club", true).await;
    w.say(alice, &g, "welcome text").await;
    let link = w.devices[alice].group(&g).await.unwrap().link.expect("managers have the link");
    assert!(GroupLink::parse(&link).unwrap().secret.is_none(), "a private link opens nothing by itself");

    let v = w.open_link(bob, &link).await;
    assert_eq!(v.membership, "requested");
    assert_eq!(w.devices[alice].group(&g).await.unwrap().requests, vec![w.pk(bob).as_hex().to_string()]);
    w.tick();
    let a = &w.devices[alice];
    let o = a.svc.approve_request(&a.keys, &g, &w.pk(bob)).await.unwrap();
    w.run(alice, o).await;
    assert_eq!(w.devices[bob].group(&g).await.unwrap().membership, "joined");
    assert!(w.devices[alice].group(&g).await.unwrap().requests.is_empty());
    w.catch_up(bob).await;
    assert_eq!(w.devices[bob].texts(&g).await, vec!["welcome text"]);

    w.open_link(carol, &link).await;
    w.tick();
    let a = &w.devices[alice];
    let o = a.svc.reject_request(&a.keys, &g, &w.pk(carol)).await.unwrap();
    w.run(alice, o).await;
    assert_eq!(w.devices[carol].group(&g).await.unwrap().membership, "rejected");
    assert_eq!(w.devices[alice].group(&g).await.unwrap().members.len(), 2);
}

#[tokio::test]
async fn nobody_is_put_into_a_group_unasked() {
    let mut w = World::new();
    let mallory = w.person().await;
    let bob = w.person().await;
    let g = w.create(mallory, GroupKind::Private, "Spam", true).await;
    w.tick();
    let m = &w.devices[mallory];
    let o = m.svc.admit(&m.keys, &g, &w.pk(bob)).await.unwrap();
    w.run(mallory, o).await;
    assert!(w.devices[bob].group(&g).await.is_none(), "a welcome nobody asked for is put aside");
}

#[tokio::test]
async fn public_group_by_link_with_history_in_the_wrong_order() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let dave = w.person().await;
    let g = w.create(alice, GroupKind::Public, "Town square", true).await;
    w.say(alice, &g, "one").await;
    let link = w.devices[alice].group(&g).await.unwrap().link.unwrap();

    // Bob opens the link while the relay has the history.
    let v = w.open_link(bob, &link).await;
    assert_eq!(v.membership, "joining");
    w.catch_up(bob).await;
    let v = w.devices[bob].group(&g).await.unwrap();
    assert_eq!((v.membership.as_str(), v.members.len()), ("joined", 2));
    assert_eq!(v.link.as_deref(), Some(link.as_str()), "members share the same link");
    assert_eq!(w.devices[alice].group(&g).await.unwrap().members.len(), 2);
    assert_eq!(w.devices[bob].texts(&g).await, vec!["one"]);

    w.say(bob, &g, "two").await;
    w.say(alice, &g, "three").await;
    // Dave comes much later and reads everything, newest first.
    w.open_link(dave, &link).await;
    w.catch_up(dave).await;
    assert_eq!(w.devices[dave].texts(&g).await, vec!["one", "two", "three"]);
    assert_eq!(w.devices[dave].group(&g).await.unwrap().members.len(), 3);
    assert_eq!(w.devices[dave].group(&g).await.unwrap().undecrypted, 0);
    for d in [alice, bob, dave] {
        let lines = w.devices[d].lines(&g).await;
        assert_eq!(lines.iter().filter(|l| *l == "group_joined").count(), 2, "device {d}: {lines:?}");
    }
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

#[tokio::test]
async fn ban_hides_and_a_new_link_shuts_the_old_one() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let troll = w.person().await;
    let late = w.person().await;
    let g = w.create(alice, GroupKind::Public, "Square", true).await;
    let link = w.devices[alice].group(&g).await.unwrap().link.unwrap();
    for p in [bob, troll] {
        w.open_link(p, &link).await;
        w.catch_up(p).await;
    }
    w.say(troll, &g, "fine so far").await;
    w.act(alice, &g, OpBody::Ban { who: w.pk(troll) }).await.unwrap();
    assert_eq!(w.devices[troll].group(&g).await.unwrap().membership, "banned");
    assert_eq!(w.devices[alice].group(&g).await.unwrap().banned, vec![w.pk(troll).as_hex().to_string()]);
    assert_eq!(code(w.try_say(troll, &g, "again").await), "group_not_member");
    // The link still opens the group for everyone else…
    w.open_link(troll, &link).await;
    w.catch_up(troll).await;
    w.synced(troll, &g).await;
    assert_eq!(w.devices[troll].group(&g).await.unwrap().membership, "banned");
    assert_eq!(w.devices[alice].group(&g).await.unwrap().members.len(), 2, "troll is not back");

    // …until the owner presses the button.
    w.act(alice, &g, OpBody::RotateLink { link_epoch: 1 }).await.unwrap();
    let new_link = w.devices[bob].group(&g).await.unwrap().link.unwrap();
    assert_ne!(new_link, link);
    assert_eq!(new_link, w.devices[alice].group(&g).await.unwrap().link.unwrap(), "members got the new link");
    w.say(alice, &g, "behind the new link").await;
    assert_eq!(w.devices[bob].texts(&g).await, vec!["fine so far", "behind the new link"]);

    // The old link shows the old part only and lets nobody in.
    w.open_link(late, &link).await;
    w.catch_up(late).await;
    let v = w.devices[late].group(&g).await.unwrap();
    assert_eq!(v.membership, "stale_link");
    assert!(!w.devices[late].texts(&g).await.contains(&"behind the new link".to_string()));
    assert_eq!(w.devices[alice].group(&g).await.unwrap().members.len(), 2);
}

#[tokio::test]
async fn roles_mute_and_moderation_of_messages() {
    let mut w = World::new();
    let alice = w.person().await;
    let mod_ = w.person().await;
    let user = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Moderated", true).await;
    w.bring(alice, &g, mod_).await;
    w.bring(alice, &g, user).await;
    w.act(alice, &g, OpBody::SetRole { who: w.pk(mod_), role: Role::Moderator }).await.unwrap();
    assert_eq!(w.devices[mod_].group(&g).await.unwrap().my_role.as_deref(), Some("moderator"));

    let bad = w.say(user, &g, "something rude").await;
    let good = w.say(alice, &g, "owner speaks").await;
    // A member removes nothing of others; a moderator nothing of the owner.
    let d = &w.devices[user];
    assert_eq!(code(d.svc.prepare_delete(&d.keys, &good).await.map(|_| ())), "group_not_permitted");
    let d = &w.devices[mod_];
    assert_eq!(code(d.svc.prepare_delete(&d.keys, &good).await.map(|_| ())), "group_not_permitted");
    w.tick();
    let d = &w.devices[mod_];
    let (_, _, out) = d.svc.prepare_delete(&d.keys, &bad).await.unwrap();
    w.run(mod_, Outcome { publish: vec![out], ..Default::default() }).await;
    for d in [alice, mod_, user] {
        assert_eq!(w.devices[d].texts(&g).await, vec!["owner speaks"], "device {d}");
    }

    // Own message, edited.
    w.tick();
    let d = &w.devices[alice];
    let (_, _, out) = d.svc.prepare_edit(&d.keys, &good, "owner spoke").await.unwrap();
    w.run(alice, Outcome { publish: vec![out], ..Default::default() }).await;
    assert_eq!(w.devices[user].texts(&g).await, vec!["owner spoke"]);

    w.act(mod_, &g, OpBody::SetMuted { who: w.pk(user), muted: true }).await.unwrap();
    assert_eq!(code(w.try_say(user, &g, "mmm").await), "group_muted");
    assert!(!w.devices[user].group(&g).await.unwrap().can_post);
    w.act(mod_, &g, OpBody::SetMuted { who: w.pk(user), muted: false }).await.unwrap();
    w.say(user, &g, "sorry").await;
    assert_eq!(w.devices[alice].texts(&g).await, vec!["owner spoke", "sorry"]);
    // A moderator does not remove people.
    assert_eq!(code(w.act(mod_, &g, OpBody::Remove { who: w.pk(user) }).await), "group_not_permitted");
}

#[tokio::test]
async fn a_device_that_was_away_catches_up() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let carol = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Team", true).await;
    w.bring(alice, &g, bob).await;
    w.offline(bob);
    w.say(alice, &g, "one").await;
    w.bring(alice, &g, carol).await;
    w.say(carol, &g, "two").await;
    w.act(alice, &g, OpBody::Remove { who: w.pk(carol) }).await.unwrap();
    w.say(alice, &g, "three").await;
    w.act(alice, &g, OpBody::EditSettings { name: Some("Team 2".into()), about: None, picture: None, history_for_new: None, call_node: None, call_node_key: None }).await.unwrap();
    w.online(bob).await;
    let v = w.devices[bob].group(&g).await.unwrap();
    assert_eq!((v.name.as_str(), v.members.len(), v.undecrypted), ("Team 2", 2, 0));
    assert_eq!(w.devices[bob].texts(&g).await, vec!["one", "two", "three"]);
    w.say(bob, &g, "back").await;
    assert_eq!(w.devices[alice].texts(&g).await, vec!["one", "two", "three", "back"]);
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

#[tokio::test]
async fn ownership_moves_and_the_group_ends() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Short", true).await;
    w.bring(alice, &g, bob).await;
    assert_eq!(code(w.act(alice, &g, OpBody::Leave).await), "group_not_permitted");
    w.act(alice, &g, OpBody::TransferOwnership { to: w.pk(bob) }).await.unwrap();
    assert_eq!(w.devices[bob].group(&g).await.unwrap().my_role.as_deref(), Some("owner"));
    w.act(alice, &g, OpBody::Leave).await.unwrap();
    w.act(bob, &g, OpBody::Disband).await.unwrap();
    assert_eq!(w.devices[bob].group(&g).await.unwrap().membership, "disbanded");
    assert_eq!(code(w.try_say(bob, &g, "anyone?").await), "group_not_member");
    let _ = KeyId("".into());
}

#[tokio::test]
async fn the_new_link_opens_everything_and_the_old_one_cannot_be_forced() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let newcomer = w.person().await;
    let cheat = w.person().await;
    let g = w.create(alice, GroupKind::Public, "Square", true).await;
    let old = w.devices[alice].group(&g).await.unwrap().link.unwrap();
    w.open_link(bob, &old).await;
    w.catch_up(bob).await;
    w.say(bob, &g, "before").await;
    w.act(alice, &g, OpBody::RotateLink { link_epoch: 1 }).await.unwrap();
    w.say(alice, &g, "after").await;
    let new = w.devices[alice].group(&g).await.unwrap().link.unwrap();

    w.open_link(newcomer, &new).await;
    w.catch_up(newcomer).await;
    let v = w.devices[newcomer].group(&g).await.unwrap();
    assert_eq!((v.membership.as_str(), v.members.len(), v.undecrypted), ("joined", 3, 0));
    assert_eq!(w.devices[newcomer].texts(&g).await, vec!["before", "after"]);
    w.say(newcomer, &g, "hello").await;
    assert_eq!(w.devices[bob].texts(&g).await, vec!["before", "after", "hello"]);

    // Someone with the old link who does not play by the rules: a join
    // that pretends not to know about the new link, and one that claims it.
    w.open_link(cheat, &old).await;
    let cheat_keys = w.devices[cheat].keys.clone();
    let link = GroupLink::parse(&old).unwrap();
    let old_key = link.secret.unwrap().group_key(&g, 0);
    let me = me_of(&cheat_keys);
    let create = w.devices[alice].svc.need_log(&g).await.unwrap().ordered().next().unwrap().clone();
    let heads = w.devices[alice].svc.need_log(&g).await.unwrap().heads();
    let forged = [
        crate::op::Op::new(&g, &me, vec![create.id()], 9_000, OpBody::Join)
            .with_proof(crate::op::JoinProof { epoch: 0, mac: old_key.join_mac(&g, &me, 0) }),
        crate::op::Op::new(&g, &me, heads, 9_001, OpBody::Join)
            .with_proof(crate::op::JoinProof { epoch: 1, mac: old_key.join_mac(&g, &me, 1) }),
    ];
    for op in forged {
        let signed = crate::wire::sign_op(&cheat_keys, &op).unwrap();
        let sealed = crate::wire::seal_op(&g, &old_key, &signed, vec![], &cheat_keys).unwrap();
        w.run(cheat, Outcome { publish: vec![GroupService::scoped(&g, sealed)], ..Default::default() }).await;
    }
    for d in [alice, bob, newcomer] {
        let v = w.devices[d].group(&g).await.unwrap();
        assert_eq!(v.members.len(), 3, "device {d}");
        assert!(!v.members.iter().any(|m| m.pubkey == me.as_hex()));
    }
    w.catch_up(cheat).await;
    assert_eq!(w.devices[cheat].group(&g).await.unwrap().membership, "stale_link");
}

#[tokio::test]
async fn joining_too_early_by_an_old_link_ends_as_a_stale_link() {
    let mut w = World::new();
    let alice = w.person().await;
    let carol = w.person().await;
    let g = w.create(alice, GroupKind::Public, "Square", true).await;
    let old = w.devices[alice].group(&g).await.unwrap().link.unwrap();
    w.offline(carol);
    w.act(alice, &g, OpBody::RotateLink { link_epoch: 1 }).await.unwrap();
    w.online(carol).await;
    // The relay answers oldest first this time: carol sees the group as
    // it was, joins, and only then learns that the link was replaced.
    w.open_link(carol, &old).await;
    let history = w.relay.clone();
    for e in history {
        let o = w.deliver(carol, &Wire::Group(e), true).await;
        w.run(carol, o).await;
    }
    assert_eq!(w.devices[carol].group(&g).await.unwrap().membership, "stale_link");
    assert_eq!(w.devices[alice].group(&g).await.unwrap().members.len(), 1);
    // The banned stay banned however they date their join.
    let troll = w.person().await;
    let new = w.devices[alice].group(&g).await.unwrap().link.unwrap();
    w.open_link(troll, &new).await;
    w.catch_up(troll).await;
    w.act(alice, &g, OpBody::Ban { who: w.pk(troll) }).await.unwrap();
    let keys = w.devices[troll].keys.clone();
    let me = me_of(&keys);
    let link = GroupLink::parse(&new).unwrap();
    let key = link.secret.unwrap().group_key(&g, 1);
    let log = w.devices[alice].svc.need_log(&g).await.unwrap();
    let rotate = log.ordered().find(|o| matches!(o.body, OpBody::RotateLink { .. })).unwrap().id();
    let op = crate::op::Op::new(&g, &me, vec![rotate], 9_000, OpBody::Join)
        .with_proof(crate::op::JoinProof { epoch: 1, mac: key.join_mac(&g, &me, 1) });
    let signed = crate::wire::sign_op(&keys, &op).unwrap();
    let sealed = crate::wire::seal_op(&g, &key, &signed, vec![], &keys).unwrap();
    w.run(troll, Outcome { publish: vec![GroupService::scoped(&g, sealed)], ..Default::default() }).await;
    let v = w.devices[alice].group(&g).await.unwrap();
    assert_eq!((v.members.len(), v.banned.len()), (1, 1));
}

#[tokio::test]
async fn a_lifted_ban_lets_one_be_invited_again() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Family", true).await;
    w.bring(alice, &g, bob).await;
    w.say(alice, &g, "before").await;

    w.act(alice, &g, OpBody::Ban { who: w.pk(bob) }).await.unwrap();
    assert_eq!(w.devices[bob].group(&g).await.unwrap().membership, "banned");
    w.say(alice, &g, "while banned").await;

    // Unbanning does not bring anyone back: it lets them be brought.
    w.act(alice, &g, OpBody::Unban { who: w.pk(bob) }).await.unwrap();
    let v = w.devices[alice].group(&g).await.unwrap();
    assert_eq!((v.members.len(), v.banned.len()), (1, 0));
    assert_eq!(w.devices[bob].group(&g).await.unwrap().membership, "banned", "bob hears nothing: he holds no key");

    w.bring(alice, &g, bob).await;
    let v = w.devices[bob].group(&g).await.unwrap();
    assert_eq!((v.membership.as_str(), v.can_post), ("joined", true));
    assert_eq!(w.devices[alice].group(&g).await.unwrap().members.len(), 2);
    w.catch_up(bob).await;
    w.say(bob, &g, "back").await;
    assert_eq!(w.devices[alice].texts(&g).await, vec!["before", "while banned", "back"]);
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

#[tokio::test]
async fn a_lifted_ban_opens_the_link_again() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let g = w.create(alice, GroupKind::Public, "Square", true).await;
    let link = w.devices[alice].group(&g).await.unwrap().link.unwrap();
    w.open_link(bob, &link).await;
    w.catch_up(bob).await;

    w.act(alice, &g, OpBody::Ban { who: w.pk(bob) }).await.unwrap();
    assert_eq!(w.devices[bob].group(&g).await.unwrap().membership, "banned");
    w.act(alice, &g, OpBody::Unban { who: w.pk(bob) }).await.unwrap();
    assert_eq!(w.devices[bob].group(&g).await.unwrap().membership, "banned", "bob no longer listens");

    // What bob knows says banned; the history he gets now says otherwise.
    let v = w.open_link(bob, &link).await;
    assert_eq!(v.membership, "joining");
    w.catch_up(bob).await;
    w.synced(bob, &g).await;
    let v = w.devices[bob].group(&g).await.unwrap();
    assert_eq!((v.membership.as_str(), v.can_post), ("joined", true));
    assert_eq!(w.devices[alice].group(&g).await.unwrap().members.len(), 2);
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

#[tokio::test]
async fn members_learn_who_read() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let carol = w.person().await;
    let dave = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Readers", true).await;
    w.bring(alice, &g, bob).await;
    w.bring(alice, &g, carol).await;
    let asked = w.say(alice, &g, "did you read it?").await;
    let also = w.say(carol, &g, "me too").await;
    let chat = format!("group:{g}");

    // Bob reads; his device owes the group a receipt.
    w.tick();
    w.devices[bob].dm.mark_read(&chat).await.unwrap();
    let due = w.devices[bob].dm.take_due_read().await.unwrap();
    let at = w.devices[bob].visible(&g).await.iter().filter(|m| m.direction == "in" && m.content_type != "system").map(|m| m.created_at).max().unwrap();
    assert_eq!(due, vec![(chat.clone(), at)]);
    let d = &w.devices[bob];
    let out = d.svc.prepare_read_receipt(&d.keys, &g, at).await.unwrap();
    let Outbound::PublishScoped { event, .. } = &out else { panic!("{out:?}") };
    let tags = event.json["tags"].as_array().unwrap();
    assert!(tags.iter().any(|t| t[0] == pushtags::SILENT), "a receipt wakes nobody");
    let before = w.devices[bob].visible(&g).await.len();
    w.run(bob, Outcome { publish: vec![out], ..Default::default() }).await;
    assert_eq!(w.devices[bob].visible(&g).await.len(), before, "a receipt is no message");
    assert_eq!(w.devices[alice].visible(&g).await.len(), before);

    // The authors see who read; nobody else's view of it changes.
    let bob_hex = w.pk(bob).as_hex().to_string();
    let of = |v: &[MessageView], id: &str| v.iter().find(|m| m.id == id).unwrap().clone();
    let a = of(&w.devices[alice].visible(&g).await, &asked);
    assert_eq!((a.read_at, a.seen_by.clone()), (Some(at), vec![bob_hex.clone()]));
    assert_eq!(a.delivered_at, Some(at), "a read says delivered");
    let c = of(&w.devices[carol].visible(&g).await, &also);
    assert_eq!(c.seen_by, vec![bob_hex.clone()]);
    assert!(of(&w.devices[carol].visible(&g).await, &asked).seen_by.is_empty(), "only the author is shown the readers");

    // What comes after the mark is not read.
    let next = w.say(alice, &g, "and this?").await;
    let n = of(&w.devices[alice].visible(&g).await, &next);
    assert_eq!((n.read_at, n.seen_by.len()), (None, 0));

    // Someone outside the group, holding its key somehow, is not believed.
    let svc = w.devices[alice].svc.clone();
    let key_id = svc.need_log(&g).await.unwrap().state().current_key.clone().unwrap();
    let key = svc.key(&g, &key_id).await.unwrap().unwrap();
    let outsider = &w.devices[dave].keys;
    let signed = crate::wire::sign_message(outsider, &g, &messenger_core::Envelope::receipt_read(i64::MAX / 2).encode(), w.clock.now().secs(), None).unwrap();
    let sealed = crate::wire::seal_note(&g, &key, None, &signed, outsider).unwrap();
    let o = w.deliver(alice, &Wire::Group(sealed), false).await;
    w.run(alice, o).await;
    let reads = messenger_store::receipts::peer_reads(w.devices[alice].dm.store(), &chat).await.unwrap();
    assert_eq!(reads, vec![(bob_hex, at)]);
}

// ─── Reactions ──────────────────────────────────────────────────────────────

impl World {
    async fn react(&mut self, who: usize, message_id: &str, emoji: &str) -> Result<()> {
        self.tick();
        let d = &self.devices[who];
        let (chat, out) = d.svc.prepare_reaction(&d.keys, message_id, emoji).await?;
        assert!(chat.starts_with("group:"));
        let Outbound::PublishScoped { event, .. } = &out else { panic!("{out:?}") };
        assert!(event.json["tags"].as_array().unwrap().iter().any(|t| t[0] == pushtags::SILENT), "a reaction wakes nobody");
        self.run(who, Outcome { publish: vec![out], ..Default::default() }).await;
        Ok(())
    }

    /// What `who` shows under a message: emoji, count, mine.
    async fn reactions(&self, who: usize, group: &str, message_id: &str) -> Vec<(String, i64, bool)> {
        let v = self.devices[who].visible(group).await;
        let m = v.iter().find(|m| m.id == message_id).expect("the message is here");
        m.reactions.iter().map(|r| (r.emoji.clone(), r.count, r.mine)).collect()
    }

    /// A reaction sealed with the group's key by `who`, whether the group
    /// lets them speak or not.
    async fn forged_reaction(&self, who: usize, group: &str, message_id: &str, emoji: &str) -> WireEvent {
        let svc = self.devices[0].svc.clone();
        let key_id = svc.need_log(group).await.unwrap().state().current_key.clone().unwrap();
        let key = svc.key(group, &key_id).await.unwrap().unwrap();
        let author = &self.devices[who].keys;
        let content = messenger_core::Envelope::reaction(message_id, emoji, true).encode();
        let signed = crate::wire::sign_message(author, group, &content, self.clock.now().secs(), None).unwrap();
        crate::wire::seal_note(group, &key, None, &signed, author).unwrap()
    }
}

fn r(emoji: &str, count: i64, mine: bool) -> (String, i64, bool) {
    (emoji.to_string(), count, mine)
}

#[tokio::test]
async fn members_react_and_outsiders_cannot() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let carol = w.person().await;
    let dave = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Reactions", true).await;
    w.bring(alice, &g, bob).await;
    w.bring(alice, &g, carol).await;
    let m = w.say(alice, &g, "pizza tonight?").await;
    let before = w.devices[bob].visible(&g).await.len();

    w.react(bob, &m, "👍").await.unwrap();
    w.react(carol, &m, "👍").await.unwrap();
    w.react(carol, &m, "🍕").await.unwrap();
    assert_eq!(w.reactions(alice, &g, &m).await, vec![r("👍", 2, false), r("🍕", 1, false)]);
    assert_eq!(w.reactions(bob, &g, &m).await, vec![r("👍", 2, true), r("🍕", 1, false)]);
    assert_eq!(w.reactions(carol, &g, &m).await, vec![r("👍", 2, true), r("🍕", 1, true)]);
    assert_eq!(w.devices[bob].visible(&g).await.len(), before, "a reaction is no message");
    assert_eq!(w.devices[alice].group(&g).await.unwrap().undecrypted, 0);

    // A second tap takes it back for everybody.
    w.react(bob, &m, "👍").await.unwrap();
    for d in [alice, bob, carol] {
        assert_eq!(w.reactions(d, &g, &m).await, vec![r("👍", 1, d == carol), r("🍕", 1, d == carol)], "device {d}");
    }
    // The limits hold in a group too.
    w.react(alice, &m, "🔥").await.unwrap();
    assert_eq!(code(w.react(bob, &m, "😮").await), "reaction_limit");
    assert_eq!(code(w.react(bob, &m, "no").await), "reaction_invalid");

    // Muted: refused here, and dropped by the others if sent anyway.
    w.act(alice, &g, OpBody::SetMuted { who: w.pk(carol), muted: true }).await.unwrap();
    assert_eq!(code(w.react(carol, &m, "🔥").await), "group_muted");
    w.tick();
    let sealed = w.forged_reaction(carol, &g, &m, "🔥").await;
    let o = w.deliver(alice, &Wire::Group(sealed), false).await;
    assert!(o.events.is_empty());
    w.run(alice, o).await;
    assert_eq!(w.reactions(alice, &g, &m).await, vec![r("👍", 1, false), r("🍕", 1, false), r("🔥", 1, true)]);

    // Someone outside, holding the key somehow: held like a message, since
    // the op that let them in may be on its way; shown nowhere meanwhile.
    let key_id = w.devices[bob].svc.need_log(&g).await.unwrap().state().current_key.clone().unwrap();
    let bucket = format!("held:{}", key_id.0);
    let on_bob = vec![r("👍", 1, false), r("🍕", 1, false), r("🔥", 1, false)];
    let sealed = w.forged_reaction(dave, &g, &m, "👍").await;
    let o = w.deliver(bob, &Wire::Group(sealed), false).await;
    w.run(bob, o).await;
    assert_eq!(w.reactions(bob, &g, &m).await, on_bob);
    let store = w.devices[bob].dm.store().clone();
    let held = messenger_store::groups::take_pending(&store, &g, &bucket).await.unwrap();
    assert_eq!(held.len(), 1, "it waits for the outsider to join");
    for p in &held {
        messenger_store::groups::add_pending(&store, p).await.unwrap();
    }

    // A day on, nobody let them in: when the log next grows, it is given
    // up, and the outsider who joins then does not bring it back (it was
    // said before the join anyway).
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    w.clock.0.store(now + crate::inbound::HELD_TTL_SECS + 60, Ordering::SeqCst);
    w.bring(alice, &g, dave).await;
    assert!(messenger_store::groups::take_pending(&store, &g, &bucket).await.unwrap().is_empty(), "given up, not held again");
    assert_eq!(w.reactions(bob, &g, &m).await, on_bob);
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

/// Carol joins and reacts at once; a device of Alice's that was away hears
/// the reaction before the op that let Carol in. It waits, and shows once
/// the op is there.
#[tokio::test]
async fn a_reaction_before_its_authors_join_waits_for_it() {
    let mut w = World::new();
    let alice = w.person().await;
    let carol = w.person().await;
    let alice2 = w.device(w.devices[alice].keys.clone()).await;
    let g = w.create(alice, GroupKind::Private, "Late", true).await;
    let m = w.say(alice, &g, "who is in?").await;
    assert_eq!(w.devices[alice2].texts(&g).await, vec!["who is in?"]);

    w.offline(alice2);
    w.bring(alice, &g, carol).await;
    w.catch_up(carol).await;
    w.react(carol, &m, "🙋").await.unwrap();
    // What it missed comes newest first: the reaction before the join.
    w.online(alice2).await;
    assert_eq!(w.reactions(alice2, &g, &m).await, vec![r("🙋", 1, false)]);
    assert_eq!(w.reactions(alice, &g, &m).await, vec![r("🙋", 1, false)]);
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

#[tokio::test]
async fn reactions_survive_history_replay() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let carol = w.person().await;
    let bob2 = w.device(w.devices[bob].keys.clone()).await;
    w.offline(bob2);
    let g = w.create(alice, GroupKind::Private, "Replay", true).await;
    w.bring(alice, &g, bob).await;
    let m = w.say(alice, &g, "vote").await;
    w.react(bob, &m, "👍").await.unwrap();
    w.react(alice, &m, "❤️").await.unwrap();
    w.react(alice, &m, "😂").await.unwrap();
    w.react(alice, &m, "❤️").await.unwrap();
    let want_bob = vec![r("👍", 1, true), r("😂", 1, false)];
    assert_eq!(w.reactions(bob, &g, &m).await, want_bob);

    // Bob's other device was away: it learns of the group, then fetches
    // its history, newest first: the take-back before the put it takes back.
    w.online(bob2).await;
    w.catch_up(bob2).await;
    assert_eq!(w.reactions(bob2, &g, &m).await, want_bob);

    // Carol joins later and reads the whole history, newest first.
    w.bring(alice, &g, carol).await;
    w.catch_up(carol).await;
    assert_eq!(w.reactions(carol, &g, &m).await, vec![r("👍", 1, false), r("😂", 1, false)]);

    // Everything again, on a device that has it all: nothing changes.
    let o_before = w.reactions(alice, &g, &m).await;
    w.catch_up(alice).await;
    assert_eq!(w.reactions(alice, &g, &m).await, o_before);
    assert_eq!(o_before, vec![r("👍", 1, false), r("😂", 1, true)]);
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

// ─── Contact cards ──────────────────────────────────────────────────────────

use messenger_core::Envelope;
use messenger_store::messages as msgs;

impl World {
    async fn send_card(&mut self, who: usize, group: &str, card: serde_json::Value) -> String {
        self.tick();
        let card = messenger_contacts::card::validate(&card).unwrap();
        let d = &self.devices[who];
        let (m, out) = d.svc.prepare_card(&d.keys, group, &card).await.unwrap();
        assert_eq!(m.content_type, "contact");
        self.run(who, Outcome { publish: vec![out], ..Default::default() }).await;
        m.id
    }
}

#[tokio::test]
async fn cards_in_a_group_keep_a_phone_only_from_its_owner() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Cards", true).await;
    w.bring(alice, &g, bob).await;
    let (alice_hex, carol_hex) = (w.pk(alice).as_hex().to_string(), Keys::generate().public_key().to_hex());

    let own = w.send_card(alice, &g, serde_json::json!({ "pubkey": alice_hex, "name": "alice", "phone": "+1 555 000 1111" })).await;
    let other = w.send_card(alice, &g, serde_json::json!({ "pubkey": carol_hex, "display_name": "Carol", "phone": "+15550002222" })).await;
    let list = w.devices[bob].visible(&g).await;
    let card = |id: &str| list.iter().find(|m| m.id == id).and_then(|m| m.card.clone()).expect("a card");
    assert_eq!((card(&own).pubkey, card(&own).phone.as_deref()), (alice_hex.clone(), Some("+15550001111")));
    assert_eq!((card(&other).label.as_str(), card(&other).phone.as_deref()), ("Carol", None));
    let row = msgs::get(w.devices[bob].dm.store(), &other).await.unwrap().unwrap();
    assert!(!row.envelope_json.contains("+1555") && !row.media_json.unwrap().contains("+1555"), "kept nowhere");
    for d in [alice, bob] {
        let chat = w.devices[d].dm.chat(&format!("group:{g}")).await.unwrap().unwrap();
        assert_eq!(chat.last_preview.as_deref(), Some("👤 Carol"), "device {d}");
    }

    // A card that is no card is not shown.
    let before = w.devices[bob].visible(&g).await.len();
    w.tick();
    let d = &w.devices[alice];
    let junk = Envelope::contact(serde_json::json!({ "pubkey": "nope" }));
    let (_, out) = d.svc.prepare_message(&d.keys, &g, junk, msgs::CT_CONTACT, None, None, None).await.unwrap();
    w.run(alice, Outcome { publish: vec![out], ..Default::default() }).await;
    assert_eq!(w.devices[bob].visible(&g).await.len(), before);
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}
