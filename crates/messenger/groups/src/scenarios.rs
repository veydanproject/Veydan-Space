// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Whole scenarios: several people, several devices, one relay that
//! delivers late, twice, and in the wrong order.

use crate::keys::GroupLink;
use crate::op::tests::Settings;
use crate::op::{GroupKind, KeyId, OpBody, PostMode, Schedule};
use crate::roles::Role;
use crate::service::*;
use async_trait::async_trait;
use messenger_contacts::{ContactService, ProfileService};
use messenger_core::inbound::Envelope as WireEnvelope;
use messenger_core::outbound::WireEvent;
use messenger_core::{
    Clock, Context, DmInbound, EventId, EventSource, GroupInbound, Outbound, PubKey, RelayUrl, Result, Scope, SecretStore, Timestamp,
};
use messenger_dm::{pushtags, DmService, MessageView};
use messenger_store::{servers, Store};
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
    /// Everything published, in order, as it left, with whose device sent it.
    sent: Vec<(usize, Outbound)>,
    /// The relay everything comes by: `RELAY`, of every device's home set.
    by: String,
}

const RELAY: &str = "wss://relay.example";

fn server_row(url: &str, kind: &str) -> servers::ServerRow {
    servers::ServerRow {
        id: String::new(),
        kind: kind.into(),
        name: None,
        relay_url: url.into(),
        relay_key: None,
        media_url: None,
        call_node: None,
        push_mode: servers::PUSH_OFF.into(),
        source: None,
        added_at: 0,
    }
}

impl World {
    fn new() -> Self {
        Self { clock: Arc::new(TestClock(AtomicI64::new(2_000))), devices: vec![], relay: vec![], notes: vec![], sent: vec![], by: RELAY.into() }
    }

    async fn device(&mut self, keys: Keys) -> usize {
        let store = Store::open_in_memory().await.unwrap();
        // The relay of the world is a server of the home set of everyone.
        servers::put(&store, &server_row(RELAY, servers::KIND_HOME)).await.unwrap();
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
                self.sent.push((origin, out.clone()));
                let (wire, to): (Wire, Vec<usize>) = match out {
                    // A wrap scoped to a server (a word of a group of a
                    // private server): to whom its `p` names.
                    Outbound::PublishScoped { event, .. } if event.json["kind"] == 1059 => {
                        let p = event.json["tags"].as_array().and_then(|t| t.iter().find(|t| t[0] == "p")).and_then(|t| t[1].as_str()).and_then(PubKey::parse);
                        (Wire::Dm(event), (0..self.devices.len()).filter(|i| Some(self.pk(*i)) == p).collect())
                    }
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
        let url = RelayUrl::parse(&self.by).unwrap();
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
                    srv: tags("srv").into_iter().next(),
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

/// The id of the key a group event is sealed with (its `k` tag).
fn sealed_with(out: &Outbound) -> String {
    let Outbound::PublishScoped { event, .. } = out else { panic!("not a group event: {out:?}") };
    let ev: Event = serde_json::from_value(event.json.clone()).unwrap();
    ev.tags.iter().filter(|t| t.kind() == "k").filter_map(|t| t.as_slice().get(1)).next().cloned().unwrap()
}

#[tokio::test]
async fn nothing_is_sealed_with_a_key_someone_who_left_still_knows() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let carol = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Team", true).await;
    w.bring(alice, &g, bob).await;
    w.bring(alice, &g, carol).await;
    w.say(carol, &g, "before").await;
    let old = w.devices[alice].svc.need_log(&g).await.unwrap().state().current_key.clone().unwrap();

    // Bob leaves while nobody who manages the group is online.
    w.offline(alice);
    w.act(bob, &g, OpBody::Leave).await.unwrap();
    let c = &w.devices[carol];
    assert_eq!(c.svc.need_log(&g).await.unwrap().key_status(), &crate::log::KeyStatus::Rotate);
    assert_eq!(code(w.try_say(carol, &g, "bob reads this").await), "group_key_stale");
    let c = &w.devices[carol];
    assert_eq!(code(c.svc.prepare_read_receipt(&c.keys, &g, w.clock.now().secs()).await), "group_key_stale");
    let first = c.visible(&g).await.into_iter().find(|m| m.text.as_deref() == Some("before")).unwrap();
    assert_eq!(code(c.svc.prepare_reaction(&c.keys, &first.id, "👍").await), "group_key_stale");
    let renewed = c.svc.renew_stale_key(&c.keys, &g).await.unwrap();
    assert!(renewed.publish.is_empty(), "a member does not choose keys");
    assert_eq!(c.svc.need_log(&g).await.unwrap().state().current_key, Some(old.clone()));
    assert_eq!(w.devices[carol].texts(&g).await, vec!["before"], "nothing was kept to go out later");

    // Alice comes back; the leave is in her log before her device looked
    // after the key. What she writes goes under a new key.
    w.devices[alice].online = true;
    for wire in std::mem::take(&mut w.devices[alice].missed) {
        w.deliver(alice, &wire, true).await;
    }
    w.tick();
    let a = &w.devices[alice];
    assert_eq!(a.svc.need_log(&g).await.unwrap().key_status(), &crate::log::KeyStatus::Rotate);
    assert_eq!(code(a.svc.prepare_text(&a.keys, &g, "too early", None).await), "group_key_stale");
    let renewed = a.svc.renew_stale_key(&a.keys, &g).await.unwrap();
    assert_eq!(renewed.publish.len(), 1, "the rotation");
    let (_, out) = a.svc.prepare_text(&a.keys, &g, "after bob", None).await.unwrap();
    let new = a.svc.need_log(&g).await.unwrap().state().current_key.clone().unwrap();
    assert_ne!(new, old);
    assert_eq!(sealed_with(&out), new.0);
    w.run(alice, renewed).await;
    w.run(alice, Outcome { publish: vec![out], ..Default::default() }).await;

    assert_eq!(w.devices[carol].texts(&g).await, vec!["before", "after bob"]);
    assert!(w.devices[bob].svc.key(&g, &new).await.unwrap().is_none(), "the new key never reached bob");
    assert!(!w.devices[bob].texts(&g).await.contains(&"after bob".to_string()));
    w.say(carol, &g, "carol again").await;
    assert_eq!(w.devices[alice].texts(&g).await, vec!["before", "after bob", "carol again"]);
}

#[tokio::test]
async fn an_admin_who_leaves_with_a_key_of_their_own_does_not_keep_reading() {
    let mut w = World::new();
    let alice = w.person().await;
    let dave = w.person().await;
    let bob = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Team", true).await;
    w.bring(alice, &g, dave).await;
    w.bring(alice, &g, bob).await;
    w.act(alice, &g, OpBody::SetRole { who: w.pk(dave), role: Role::Admin }).await.unwrap();
    let old = w.devices[alice].svc.need_log(&g).await.unwrap().state().current_key.clone().unwrap();

    // Dave's client leaves and hands everyone a key dave chose.
    w.tick();
    let d = &w.devices[dave];
    let log = d.svc.need_log(&g).await.unwrap();
    let sealing = d.svc.key(&g, &old).await.unwrap().unwrap();
    let mine = crate::keys::GroupKey::generate().unwrap();
    let op = log.next(&d.pk(), w.clock.now().secs(), OpBody::Leave).with_key(mine.id());
    let envelopes = [alice, dave, bob].iter().map(|i| crate::wire::envelope_for(&d.keys, &w.pk(*i), mine.as_bytes()).unwrap()).collect();
    let signed = crate::wire::sign_op(&d.keys, &op).unwrap();
    let sealed = crate::wire::seal_op(&g, &sealing, &signed, envelopes, &d.keys).unwrap();
    w.run(dave, Outcome { publish: vec![GroupService::scoped(&g, sealed)], ..Default::default() }).await;

    for who in [alice, bob] {
        let log = w.devices[who].svc.need_log(&g).await.unwrap();
        assert!(!log.state().is_member(&w.pk(dave)));
        let now = log.state().current_key.clone().unwrap();
        assert_ne!(now, mine.id(), "the key of the one who left is not taken");
        assert_ne!(now, old, "a manager brought a new key");
        assert!(!log.state().key_stale);
        assert!(w.devices[dave].svc.key(&g, &now).await.unwrap().is_none(), "the new key never reached dave");
    }
    w.say(bob, &g, "without dave").await;
    assert_eq!(w.devices[alice].texts(&g).await, vec!["without dave"]);
    assert!(w.devices[dave].texts(&g).await.is_empty());
}

#[tokio::test]
async fn the_note_of_a_request_reaches_the_managers() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Club", true).await;
    let link = w.devices[alice].group(&g).await.unwrap().link.expect("managers have the link");

    // The store stamps the row by the wall clock, not by the world's.
    let wall = messenger_core::traits::SystemClock.now().secs();
    let v = w.open_link(bob, &link).await;
    assert_eq!(v.request_note, "let me in", "the sender sees what they wrote");
    let asked = w.devices[alice].group(&g).await.unwrap();
    assert_eq!(asked.requests.len(), 1);
    assert_eq!(asked.requests[0].note, "let me in");
    assert_eq!(asked.requests[0].requester, w.pk(bob).as_hex());
    assert_eq!(Some(asked.requests[0].npub.clone()), messenger_links::contact::npub_of(&w.pk(bob)));
    assert!(asked.requests[0].npub.starts_with("npub1"));
    assert!(asked.requests[0].received_at >= wall, "when it came is by my clock, whatever date the requester put on it");
    assert!(asked.request_note.is_empty(), "the manager has no request of their own");

    // Asked again while the first request waits: the new words and the new
    // date replace the old ones, and nobody is called a second time.
    let first_date = asked.requests[0].created_at;
    let later = first_date + 100;
    let again = crate::wire::JoinRequest { group_id: g.clone(), note: "second try".into(), created_at: later };
    let bob_keys = w.devices[bob].keys.clone();
    let o = dm_from(&w, &bob_keys, alice, crate::wire::T_JOIN_REQUEST, &again, later).await;
    assert!(o.notify.is_empty(), "a request that waits does not ring again");
    let asked = w.devices[alice].group(&g).await.unwrap();
    assert_eq!(asked.requests.len(), 1);
    assert_eq!(asked.requests[0].note, "second try");
    assert_eq!(asked.requests[0].created_at, later);
    assert!(asked.requests[0].received_at >= wall, "the new words were taken in now");

    // The same date again, or an older one, changes nothing.
    let stale = crate::wire::JoinRequest { group_id: g.clone(), note: "stale".into(), created_at: later };
    dm_from(&w, &bob_keys, alice, crate::wire::T_JOIN_REQUEST, &stale, later).await;
    assert_eq!(w.devices[alice].group(&g).await.unwrap().requests[0].note, "second try");

    // The note is a stranger's text: it goes through the same cleaning as
    // any other.
    let rude = crate::wire::JoinRequest { group_id: g.clone(), note: "hi\u{202e}there".into(), created_at: later + 10 };
    dm_from(&w, &bob_keys, alice, crate::wire::T_JOIN_REQUEST, &rude, later + 10).await;
    assert_eq!(w.devices[alice].group(&g).await.unwrap().requests[0].note, messenger_richtext::normalize("hi\u{202e}there"));
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
    assert_eq!(w.devices[alice].group(&g).await.unwrap().requests.iter().map(|r| r.requester.clone()).collect::<Vec<_>>(), vec![w.pk(bob).as_hex().to_string()]);
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
async fn a_request_answered_by_one_manager_leaves_the_others() {
    let mut w = World::new();
    let alice = w.person().await;
    let dave = w.person().await;
    let bob = w.person().await;
    let carol = w.person().await;
    let dave2 = w.device(w.devices[dave].keys.clone()).await;
    let g = w.create(alice, GroupKind::Private, "Club", true).await;
    w.bring(alice, &g, dave).await;
    w.act(alice, &g, OpBody::SetRole { who: w.pk(dave), role: Role::Admin }).await.unwrap();
    w.catch_up(dave2).await;
    assert_eq!(w.devices[dave2].group(&g).await.unwrap().my_role.as_deref(), Some("admin"));
    let link = w.devices[alice].group(&g).await.unwrap().link.expect("managers have the link");

    w.open_link(bob, &link).await;
    let bob_hex = w.pk(bob).as_hex().to_string();
    for d in [alice, dave, dave2] {
        assert_eq!(w.devices[d].group(&g).await.unwrap().requests.iter().map(|r| r.requester.clone()).collect::<Vec<_>>(), vec![bob_hex.clone()], "device {d}");
    }
    w.tick();
    let d = &w.devices[dave];
    let o = d.svc.approve_request(&d.keys, &g, &w.pk(bob)).await.unwrap();
    w.run(dave, o).await;
    assert_eq!(w.devices[bob].group(&g).await.unwrap().membership, "joined");
    for d in [alice, dave, dave2] {
        assert!(w.devices[d].group(&g).await.unwrap().requests.is_empty(), "device {d}");
    }
    let row = messenger_store::groups::request(&w.devices[alice].svc.store, &g, &bob_hex, "in").await.unwrap().unwrap();
    assert_eq!(row.status, "approved", "the owner's device closed it by itself");

    // A late click on the owner's device changes nothing and sends nothing.
    let a = &w.devices[alice];
    let o = a.svc.approve_request(&a.keys, &g, &w.pk(bob)).await.unwrap();
    assert!(o.publish.is_empty());
    let o = a.svc.reject_request(&a.keys, &g, &w.pk(bob)).await.unwrap();
    assert!(o.publish.is_empty(), "no \"rejected\" to a member");
    assert_eq!(w.devices[bob].group(&g).await.unwrap().membership, "joined");
    assert_eq!(w.devices[alice].group(&g).await.unwrap().members.len(), 3);

    // A ban by one manager closes the request on the other too.
    w.open_link(carol, &link).await;
    assert_eq!(w.devices[alice].group(&g).await.unwrap().requests.iter().map(|r| r.requester.clone()).collect::<Vec<_>>(), vec![w.pk(carol).as_hex().to_string()]);
    w.act(dave, &g, OpBody::Ban { who: w.pk(carol) }).await.unwrap();
    assert!(w.devices[alice].group(&g).await.unwrap().requests.is_empty());
    let row = messenger_store::groups::request(&w.devices[alice].svc.store, &g, w.pk(carol).as_hex(), "in").await.unwrap().unwrap();
    assert_eq!(row.status, "rejected");
    assert!(w.notes.is_empty(), "{:?}", w.notes);
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

    w.act(mod_, &g, OpBody::SetMuted { who: w.pk(user), muted: true, until: None }).await.unwrap();
    assert_eq!(code(w.try_say(user, &g, "mmm").await), "group_muted");
    assert!(!w.devices[user].group(&g).await.unwrap().can_post);
    w.act(mod_, &g, OpBody::SetMuted { who: w.pk(user), muted: false, until: None }).await.unwrap();
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
    w.act(alice, &g, Settings { name: Some("Team 2".into()), ..Settings::default() }.body()).await.unwrap();
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
    w.act(alice, &g, OpBody::SetMuted { who: w.pk(carol), muted: true, until: None }).await.unwrap();
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

// ─── Quotes of media ────────────────────────────────────────────────────────

#[tokio::test]
async fn a_quote_of_a_captionless_photo_in_a_group_says_photo_and_a_taken_back_one_says_deleted() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Photos", true).await;
    w.bring(alice, &g, bob).await;

    w.tick();
    let fields = serde_json::json!({ "name": "cat.png", "mime": "image/png", "size": 10, "kind": "image" });
    let d = &w.devices[alice];
    let ph = d.svc.media_placeholder(&d.keys, &g, fields.clone(), None, messenger_core::TextFormat::Plain).await.unwrap();
    let envelope = Envelope::new("media").with("kind", "image").with("name", "cat.png").with("key", "k");
    let (photo, out) = d.svc.media_finish(&d.keys, &ph.id, envelope, fields).await.unwrap();
    w.run(alice, Outcome { publish: vec![out], ..Default::default() }).await;
    assert!(photo.text.is_none());

    w.tick();
    let d = &w.devices[bob];
    let (reply, out) = d.svc.prepare_text(&d.keys, &g, "nice", Some(&photo.id)).await.unwrap();
    w.run(bob, Outcome { publish: vec![out], ..Default::default() }).await;
    for (who, quote) in [(bob, reply.reply_to.clone().unwrap()), (alice, {
        let list = w.devices[alice].visible(&g).await;
        list.iter().find(|m| m.id == reply.id).and_then(|m| m.reply_to.clone()).expect("the quote")
    })] {
        assert_eq!(quote.id, photo.id, "device {who}");
        assert!(quote.text.is_none() && !quote.deleted, "device {who}");
        assert_eq!(quote.content_type, "media");
        assert_eq!(quote.media.unwrap()["kind"], "image");
    }

    // Taken back for everyone: the quote says so and carries no file.
    w.tick();
    let d = &w.devices[alice];
    let (_, _, out) = d.svc.prepare_delete(&d.keys, &photo.id).await.unwrap();
    w.run(alice, Outcome { publish: vec![out], ..Default::default() }).await;
    let list = w.devices[bob].visible(&g).await;
    let quote = list.iter().find(|m| m.id == reply.id).and_then(|m| m.reply_to.clone()).expect("the quote");
    assert!(quote.deleted && quote.media.is_none());
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

// ─── Captions ───────────────────────────────────────────────────────────────

impl World {
    async fn send_photo(&mut self, who: usize, group: &str, caption: &str) -> String {
        self.tick();
        let fields = serde_json::json!({ "name": "cat.png", "mime": "image/png", "size": 10, "kind": "image" });
        let d = &self.devices[who];
        let ph = d.svc.media_placeholder(&d.keys, group, fields.clone(), Some(caption), messenger_core::TextFormat::Plain).await.unwrap();
        // The placeholder's id is replaced once the file is up: nobody would see an edit of it.
        assert!(d.svc.prepare_edit(&d.keys, &ph.id, "too early").await.is_err());
        let envelope = Envelope::new("media").with("kind", "image").with("name", "cat.png").with("caption", caption).with("key", "k");
        let (photo, out) = d.svc.media_finish(&d.keys, &ph.id, envelope, fields).await.unwrap();
        self.run(who, Outcome { publish: vec![out], ..Default::default() }).await;
        photo.id
    }

    async fn edit(&mut self, who: usize, message_id: &str, text: &str) {
        self.tick();
        let d = &self.devices[who];
        let (_, _, out) = d.svc.prepare_edit(&d.keys, message_id, text).await.unwrap();
        self.run(who, Outcome { publish: vec![out], ..Default::default() }).await;
    }

    async fn caption(&self, who: usize, group: &str, message_id: &str) -> (Option<String>, bool) {
        let list = self.devices[who].visible(group).await;
        let m = list.iter().find(|m| m.id == message_id).expect("the photo");
        assert_eq!(m.content_type, "media");
        (m.text.clone(), m.edited_at.is_some())
    }
}

#[tokio::test]
async fn the_caption_of_my_photo_is_edited_for_members_and_my_other_device() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let alice2 = w.device(w.devices[alice].keys.clone()).await;
    let g = w.create(alice, GroupKind::Private, "Photos", true).await;
    w.bring(alice, &g, bob).await;

    let photo = w.send_photo(alice, &g, "look").await;
    w.edit(alice, &photo, "a cat").await;
    for d in [alice, alice2, bob] {
        assert_eq!(w.caption(d, &g, &photo).await, (Some("a cat".into()), true), "device {d}");
    }
    let chat = w.devices[bob].dm.chat(&format!("group:{g}")).await.unwrap().unwrap();
    assert_eq!(chat.last_preview.as_deref(), Some("📎 a cat"));
    // Not mine: no edit.
    let d = &w.devices[bob];
    assert!(d.svc.prepare_edit(&d.keys, &photo, "mine now").await.is_err());

    // The edit comes before its photo: it waits and is applied on arrival.
    w.offline(bob);
    let late = w.send_photo(alice, &g, "one").await;
    w.edit(alice, &late, "two").await;
    w.online(bob).await;
    assert_eq!(w.caption(bob, &g, &late).await, (Some("two".into()), true));

    // A card has no text to edit.
    let card = w.send_card(alice, &g, serde_json::json!({ "pubkey": w.pk(alice).as_hex(), "name": "alice" })).await;
    let d = &w.devices[alice];
    assert!(d.svc.prepare_edit(&d.keys, &card, "text").await.is_err());
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

#[tokio::test]
async fn markdown_reaches_every_device_and_an_edit_overtaking_it_keeps_its_format() {
    use messenger_core::TextFormat;
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let alice2 = w.device(w.devices[alice].keys.clone()).await;
    let g = w.create(alice, GroupKind::Private, "Notes", true).await;
    w.bring(alice, &g, bob).await;

    w.tick();
    let d = &w.devices[alice];
    let (sent, out) = d.svc.prepare_text_as(&d.keys, &g, "# Plan\n\n`cargo test`", TextFormat::Markdown, None).await.unwrap();
    w.run(alice, Outcome { publish: vec![out], ..Default::default() }).await;
    for who in [alice, alice2, bob] {
        let list = w.devices[who].visible(&g).await;
        let m = list.iter().find(|m| m.id == sent.id).expect("the message");
        assert_eq!((m.fmt.as_deref(), m.rich.is_some()), (Some("md"), true), "device {who}");
    }
    let chat = w.devices[bob].dm.chat(&format!("group:{g}")).await.unwrap().unwrap();
    assert_eq!(chat.last_preview.as_deref(), Some("Plan\ncargo test"), "the words, not the marks");

    // An edit into plain text that comes before its message.
    w.offline(bob);
    w.tick();
    let d = &w.devices[alice];
    let (late, out) = d.svc.prepare_text_as(&d.keys, &g, "**v1**", TextFormat::Markdown, None).await.unwrap();
    w.run(alice, Outcome { publish: vec![out], ..Default::default() }).await;
    w.edit(alice, &late.id, "**v2**").await;
    w.online(bob).await;
    let list = w.devices[bob].visible(&g).await;
    let m = list.iter().find(|m| m.id == late.id).expect("the message");
    assert_eq!((m.text.as_deref(), m.fmt.as_deref()), (Some("**v2**"), None));
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

// ─── Messages dated back ────────────────────────────────────────────────────

/// A message signed by `who` at `inner`, sealed with `key` into an event
/// dated `outer`: what any holder of the key can make.
fn backdated(w: &World, who: usize, group: &str, key: &crate::keys::GroupKey, text: &str, inner: i64, outer: i64) -> WireEvent {
    let author = &w.devices[who].keys;
    let content = messenger_core::Envelope::text(text).encode();
    let signed = crate::wire::sign_message(author, group, &content, inner, None).unwrap();
    let payload = crate::wire::Payload {
        v: crate::wire::PAYLOAD_VERSION,
        t: "msg".into(),
        event: serde_json::to_value(&signed).unwrap(),
        envelopes: vec![],
        keys: vec![],
    };
    crate::wire::seal(group, key, None, &payload, outer, author, false).unwrap()
}

/// When the op of the log that matches `pick` was dated.
async fn op_time(w: &World, who: usize, group: &str, pick: impl Fn(&OpBody) -> bool) -> i64 {
    let log = w.devices[who].svc.need_log(group).await.unwrap();
    log.ordered().filter(|o| pick(&o.body)).map(|o| o.created_at).max().unwrap()
}

#[tokio::test]
async fn the_removed_does_not_speak_by_dating_a_message_back() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let carol = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Team", true).await;
    w.bring(alice, &g, bob).await;
    w.bring(alice, &g, carol).await;
    w.say(bob, &g, "still here").await;
    let old_id = w.devices[alice].svc.need_log(&g).await.unwrap().state().current_key.clone().unwrap();
    w.act(alice, &g, OpBody::Remove { who: w.pk(bob) }).await.unwrap();
    let removed_at = op_time(&w, carol, &g, |b| matches!(b, OpBody::Remove { .. })).await;
    // Bob kept the key he had; the time inside says he was still a member.
    let old = w.devices[bob].svc.key(&g, &old_id).await.unwrap().expect("bob kept the old key");
    w.tick();
    let now = w.clock.now().secs();
    assert!(now - (removed_at - 1) <= 300, "within the slack of the clocks");
    for (inner, text) in [(removed_at - 1, "a second before"), (removed_at, "the same second")] {
        let forged = backdated(&w, bob, &g, &old, text, inner, now);
        let o = w.deliver(carol, &Wire::Group(forged), false).await;
        assert!(o.notify.is_empty(), "nobody is woken for {text:?}");
        w.run(carol, o).await;
    }
    // Nothing waits to be let in later as history.
    w.act(alice, &g, Settings { name: Some("Team 2".into()), ..Settings::default() }.body()).await.unwrap();
    w.say(alice, &g, "without bob").await;
    assert_eq!(w.devices[carol].texts(&g).await, vec!["still here", "without bob"]);
}

#[tokio::test]
async fn a_muted_member_does_not_date_a_message_before_the_mute() {
    let mut w = World::new();
    let alice = w.person().await;
    let user = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Moderated", true).await;
    w.bring(alice, &g, user).await;
    w.say(user, &g, "fine").await;
    w.act(alice, &g, OpBody::SetMuted { who: w.pk(user), muted: true, until: None }).await.unwrap();
    let muted_at = op_time(&w, alice, &g, |b| matches!(b, OpBody::SetMuted { .. })).await;
    let id = w.devices[user].svc.need_log(&g).await.unwrap().state().current_key.clone().unwrap();
    let key = w.devices[user].svc.key(&g, &id).await.unwrap().unwrap();
    w.tick();
    // Sealed as an honest client seals: the event dated as the message.
    let forged = backdated(&w, user, &g, &key, "before the mute", muted_at - 1, muted_at - 1);
    let o = w.deliver(alice, &Wire::Group(forged), false).await;
    assert!(o.notify.is_empty());
    w.run(alice, o).await;
    assert_eq!(w.devices[alice].texts(&g).await, vec!["fine"]);
}

#[tokio::test]
async fn what_was_said_before_a_removal_comes_later_as_history() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let carol = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Team", true).await;
    w.bring(alice, &g, bob).await;
    w.bring(alice, &g, carol).await;
    w.offline(carol);
    w.say(bob, &g, "last word").await;
    w.act(alice, &g, OpBody::Remove { who: w.pk(bob) }).await.unwrap();
    w.say(alice, &g, "without bob").await;
    w.online(carol).await;
    assert_eq!(w.devices[carol].texts(&g).await, vec!["last word", "without bob"]);
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

#[tokio::test]
async fn an_event_dated_far_from_its_message_is_dropped() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Team", true).await;
    w.bring(alice, &g, bob).await;
    let id = w.devices[bob].svc.need_log(&g).await.unwrap().state().current_key.clone().unwrap();
    let key = w.devices[bob].svc.key(&g, &id).await.unwrap().unwrap();
    w.tick();
    let now = w.clock.now().secs();
    for (inner, outer, text) in [(now, now - 301, "an old event"), (now - 301, now, "an old message"), (now - 300, now, "within the slack")] {
        let sealed = backdated(&w, bob, &g, &key, text, inner, outer);
        let o = w.deliver(alice, &Wire::Group(sealed), false).await;
        w.run(alice, o).await;
    }
    assert_eq!(w.devices[alice].texts(&g).await, vec!["within the slack"]);
}

/// A direct message of the group protocol from `from` to device `to`, as
/// the DM service hands it over, dated `created_at`.
async fn dm_from<T: serde::Serialize>(w: &World, from: &Keys, to: usize, t: &str, body: &T, created_at: i64) -> Outcome {
    let d = &w.devices[to];
    let id = || EventId::parse(&Keys::generate().public_key().to_hex()).unwrap();
    let msg = DmInbound {
        envelope: WireEnvelope {
            wire_id: id(),
            source: EventSource::Relay { url: RelayUrl::parse(RELAY).unwrap() },
            wire_created_at: Timestamp(created_at),
        },
        rumor_id: id(),
        sender: me_of(from),
        recipients: vec![d.pk()],
        created_at: Timestamp(created_at),
        content: crate::wire::dm_envelope(t, body).unwrap(),
        reply_to: None,
        rumor_kind: 14,
        srv: None,
    };
    d.svc.on_dm(&d.keys, &msg, &d.ctx()).await.unwrap().expect("a group message")
}

fn an_invite(group_id: &str, name: &str, now: i64) -> crate::wire::Invite {
    let mut id = [0u8; 16];
    getrandom::fill(&mut id).unwrap();
    crate::wire::Invite {
        invite_id: hex::encode(id),
        group_id: group_id.to_string(),
        name: name.to_string(),
        about: String::new(),
        picture: String::new(),
        relay: RELAY.into(),
        members: 3,
        created_at: now,
        expires_at: now + crate::wire::INVITE_TTL_SECS,
    }
}

fn some_group_id() -> String {
    Keys::generate().public_key().to_hex()
}

#[tokio::test]
async fn an_invitation_is_dated_by_my_clock_and_its_fields_are_checked() {
    let mut w = World::new();
    let bob = w.person().await;
    let eve = Keys::generate();
    let now = w.clock.now().secs();
    let year = 365 * 24 * 3600;
    let bad = [
        // Dated ten years ahead, good for a week after that.
        crate::wire::Invite { created_at: now + 10 * year, expires_at: now + 10 * year + 3600, ..an_invite(&some_group_id(), "Later", now) },
        crate::wire::Invite { created_at: now + 301, ..an_invite(&some_group_id(), "Ahead", now) },
        crate::wire::Invite { expires_at: now, ..an_invite(&some_group_id(), "Over", now) },
        crate::wire::Invite { invite_id: "x".repeat(5000), ..an_invite(&some_group_id(), "Long id", now) },
        crate::wire::Invite { group_id: "AB".repeat(32), ..an_invite("", "Upper", now) },
        an_invite(&some_group_id(), "Bell\u{0007}", now),
        an_invite(&some_group_id(), "\u{202E}\u{202C}", now),
        an_invite(&some_group_id(), &"n".repeat(101), now),
        an_invite(&some_group_id(), "   ", now),
        crate::wire::Invite { about: "a".repeat(1001), ..an_invite(&some_group_id(), "About", now) },
        crate::wire::Invite { picture: "http://x/p.png".into(), ..an_invite(&some_group_id(), "Picture", now) },
        crate::wire::Invite { relay: "https://not.a.relay".into(), ..an_invite(&some_group_id(), "Relay", now) },
    ];
    for invite in &bad {
        let o = dm_from(&w, &eve, bob, crate::wire::T_INVITE, invite, now).await;
        assert!(o.notify.is_empty() && o.events.is_empty(), "{:?}", invite.name);
    }
    assert!(w.devices[bob].svc.invites("in").await.unwrap().is_empty(), "none of them is kept");

    // Within the skew and well-formed: kept with its name cleaned, and not
    // told of, since eve is nobody to bob.
    let fine = crate::wire::Invite {
        created_at: now + 300,
        about: "\u{202E}tuoba".into(),
        ..an_invite(&some_group_id(), " \u{202E}maeT ", now)
    };
    let o = dm_from(&w, &eve, bob, crate::wire::T_INVITE, &fine, now).await;
    assert!(o.notify.is_empty(), "a stranger's invitation is not told of");
    let kept = w.devices[bob].svc.invites("in").await.unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!((kept[0].name.as_str(), kept[0].about.as_str()), ("maeT", "tuoba"));

    // From a contact it is told of.
    let d = &w.devices[bob];
    d.dm.act(&d.keys, &me_of(&eve), messenger_dm::Action::Request).await.unwrap();
    let o = dm_from(&w, &eve, bob, crate::wire::T_INVITE, &an_invite(&some_group_id(), "Friends", now), now).await;
    assert_eq!(o.notify.len(), 1);
    assert_eq!(o.notify[0].title, "Friends");
}

#[tokio::test]
async fn in_a_group_i_was_in_a_manager_is_told_of_and_the_rest_waits_unheard() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let carol = w.person().await;
    let dave = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Team", true).await;
    w.bring(alice, &g, bob).await;
    w.bring(alice, &g, carol).await;
    w.act(alice, &g, OpBody::Ban { who: w.pk(dave) }).await.unwrap();
    w.act(bob, &g, OpBody::Leave).await.unwrap();
    assert_eq!(w.devices[bob].group(&g).await.unwrap().membership, "left");
    // Carol is made an admin after bob left: his log stopped before that.
    w.act(alice, &g, OpBody::SetRole { who: w.pk(carol), role: Role::Admin }).await.unwrap();
    let before = w.devices[bob].svc.invites("in").await.unwrap().len();
    let standing = |who: usize| {
        let (store, pk) = (w.devices[bob].svc.store.clone(), w.pk(who));
        let g = g.clone();
        async move { invite_standing_in(&store, &g, &pk, false).await.unwrap() }
    };
    assert_eq!(standing(dave).await, InviteStanding::Drop, "banned by the log bob holds");
    assert_eq!(standing(carol).await, InviteStanding::Unvouched, "no manager by that log");
    assert_eq!(standing(alice).await, InviteStanding::Vouched);

    // Dave is banned there: dropped.
    let now = w.clock.now().secs();
    let dave_keys = w.devices[dave].keys.clone();
    let o = dm_from(&w, &dave_keys, bob, crate::wire::T_INVITE, &an_invite(&g, "Team", now), now).await;
    assert!(o.notify.is_empty() && o.events.is_empty());
    assert_eq!(w.devices[bob].svc.invites("in").await.unwrap().len(), before);
    // Carol's word alone: kept to be answered, not told of.
    let carol_keys = w.devices[carol].keys.clone();
    let o = dm_from(&w, &carol_keys, bob, crate::wire::T_INVITE, &an_invite(&g, "Team", now), now).await;
    assert!(o.notify.is_empty());
    assert_eq!(w.devices[bob].svc.invites("in").await.unwrap().len(), before + 1);

    // Alice is the owner: kept and told of, though she is no contact of bob.
    w.tick();
    let a = &w.devices[alice];
    let (_, o) = a.svc.invite(&a.keys, &g, &w.pk(bob)).await.unwrap();
    let to_bob = o.publish.iter().find_map(|p| match p {
        Outbound::PublishToInbox { recipient, event, .. } if recipient == &w.pk(bob) => Some(event.clone()),
        _ => None,
    });
    let o = w.deliver(bob, &Wire::Dm(to_bob.expect("a DM to bob")), false).await;
    assert_eq!(o.notify.len(), 1, "the log vouches for the inviter");
    assert_eq!(w.devices[bob].svc.invites("in").await.unwrap().len(), before + 2);
}

#[tokio::test]
async fn invitations_of_strangers_and_requests_that_wait_are_few() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let now = w.clock.now().secs();
    let names = |w: &World| {
        let svc = w.devices[bob].svc.clone();
        async move { svc.invites("in").await.unwrap().into_iter().map(|i| i.name).collect::<Vec<_>>() }
    };

    // One inviter, many groups: the first ones make room for the later.
    let eve = Keys::generate();
    for i in 0..12 {
        let invite = an_invite(&some_group_id(), &format!("G{i}"), now - 100 + i);
        dm_from(&w, &eve, bob, crate::wire::T_INVITE, &invite, now).await;
    }
    let kept = names(&w).await;
    assert_eq!(kept.len(), 10);
    assert!(!kept.contains(&"G0".to_string()) && !kept.contains(&"G1".to_string()) && kept.contains(&"G11".to_string()), "{kept:?}");
    // Many strangers, one group; then my contact to the same group is not
    // held back by them, and pushes none of them out.
    let g = some_group_id();
    for _ in 0..6 {
        dm_from(&w, &Keys::generate(), bob, crate::wire::T_INVITE, &an_invite(&g, "One", now - 50), now).await;
    }
    let of_g = |w: &World| {
        let (svc, g) = (w.devices[bob].svc.clone(), g.clone());
        async move { svc.invites("in").await.unwrap().into_iter().filter(|i| i.group_id == g).count() }
    };
    assert_eq!(of_g(&w).await, 4);
    let friend = Keys::generate();
    let d = &w.devices[bob];
    d.dm.act(&d.keys, &me_of(&friend), messenger_dm::Action::Request).await.unwrap();
    let o = dm_from(&w, &friend, bob, crate::wire::T_INVITE, &an_invite(&g, "One", now), now).await;
    assert_eq!(o.notify.len(), 1);
    assert_eq!(of_g(&w).await, 5);
    // In all, from strangers each new: fifty of theirs, and the contact's.
    for i in 0..60 {
        dm_from(&w, &Keys::generate(), bob, crate::wire::T_INVITE, &an_invite(&some_group_id(), &format!("S{i}"), now), now).await;
    }
    assert_eq!(w.devices[bob].svc.invites("in").await.unwrap().len(), 51);
    assert_eq!(of_g(&w).await, 1, "the strangers' ones went first, the contact's stayed");

    // Requests to join a private group: twenty wait, the rest is dropped,
    // one dated ahead of the manager's clock is not taken, and a date eight
    // days back (the sender's word) does not take a request out of the count.
    let team = w.create(alice, GroupKind::Private, "Team", true).await;
    let now = w.clock.now().secs();
    let ahead = crate::wire::JoinRequest { group_id: team.clone(), note: String::new(), created_at: now + 301 };
    let o = dm_from(&w, &Keys::generate(), alice, crate::wire::T_JOIN_REQUEST, &ahead, now).await;
    assert!(o.notify.is_empty());
    let old = now - crate::wire::INVITE_TTL_SECS - 10;
    for _ in 0..25 {
        let r = crate::wire::JoinRequest { group_id: team.clone(), note: "old".into(), created_at: old };
        dm_from(&w, &Keys::generate(), alice, crate::wire::T_JOIN_REQUEST, &r, old).await;
    }
    for _ in 0..25 {
        let r = crate::wire::JoinRequest { group_id: team.clone(), note: "hi".into(), created_at: now };
        dm_from(&w, &Keys::generate(), alice, crate::wire::T_JOIN_REQUEST, &r, now).await;
    }
    assert_eq!(w.devices[alice].group(&team).await.unwrap().requests.len(), 20, "old dates count, by the time they came");
}

#[tokio::test]
async fn the_description_is_shown_without_controls_though_the_log_keeps_it() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Team", true).await;
    w.bring(alice, &g, bob).await;
    let about = "\u{202E}gnirts\u{0007} of the team";
    let edit = Settings { about: Some(about.into()), ..Settings::default() }.body();
    w.act(alice, &g, edit).await.unwrap();
    for d in [alice, bob] {
        assert_eq!(w.devices[d].svc.need_log(&g).await.unwrap().state().about, about, "the log is as it was written");
        assert_eq!(w.devices[d].group(&g).await.unwrap().about, "gnirts of the team", "device {d}");
    }
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

#[tokio::test]
async fn a_group_keeps_no_more_keys_than_its_limit_and_never_the_ones_its_log_names() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let carol = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Team", true).await;
    w.bring(alice, &g, bob).await;
    w.say(alice, &g, "before").await;
    let named = w.devices[bob].svc.need_log(&g).await.unwrap().state().keys.clone();

    // A manager hands bob a heap of keys nobody uses.
    let junk: Vec<_> = (0..600).map(|_| crate::keys::GroupKey::generate().unwrap()).collect();
    let alice_keys = w.devices[alice].keys.clone();
    let now = w.clock.now().secs();
    for part in junk.chunks(200) {
        let body = crate::wire::KeyDelivery { group_id: g.clone(), keys: crate::wire::encode_keys(part) };
        dm_from(&w, &alice_keys, bob, crate::wire::T_KEYS, &body, now).await;
    }
    let ids = messenger_store::groups::key_ids(&w.devices[bob].svc.store, &g).await.unwrap();
    assert_eq!(ids.len(), crate::inbound::MAX_KEYS);
    for k in &named {
        assert!(ids.contains(&k.0), "a key of the log stays");
        assert!(w.devices[bob].svc.key(&g, k).await.unwrap().is_some());
    }
    let (stay, gone): (Vec<_>, Vec<_>) = junk.iter().partition(|k| ids.contains(&k.id().0));
    assert_eq!(stay.len(), crate::inbound::MAX_KEYS - named.len(), "only the junk went");
    assert!(stay.iter().any(|k| k.id() == junk[599].id()), "the one kept last stays");
    for k in gone {
        assert!(w.devices[bob].svc.key(&g, &k.id()).await.unwrap().is_none(), "a key that went takes its secret with it");
    }

    // The group goes on: bob reads and writes, a newcomer gets in.
    w.say(alice, &g, "after").await;
    w.say(bob, &g, "still here").await;
    w.bring(alice, &g, carol).await;
    w.say(carol, &g, "hi").await;
    for d in [alice, bob] {
        assert_eq!(w.devices[d].texts(&g).await, vec!["before", "after", "still here", "hi"], "device {d}");
    }
}

#[tokio::test]
async fn a_stranger_s_invitation_pushed_out_leaves_no_row_and_its_word_is_replaced() {
    let mut w = World::new();
    let bob = w.person().await;
    let store = w.devices[bob].svc.store.clone();
    let now = w.clock.now().secs();
    // A row kept before the mark of vouching existed: counted as vouched.
    let old = an_invite(&some_group_id(), "From before", now);
    messenger_store::groups::insert_invite(
        &store,
        &messenger_store::groups::InviteRow {
            invite_id: old.invite_id.clone(),
            group_id: old.group_id.clone(),
            direction: "in".into(),
            peer: Keys::generate().public_key().to_hex(),
            status: "received".into(),
            payload_json: serde_json::to_string(&old).unwrap(),
            created_at: old.created_at,
            expires_at: old.expires_at,
            updated_at: 0,
        },
    )
    .await
    .unwrap();
    // One stranger asks twice about one group: the later word stands alone.
    let eve = Keys::generate();
    let g = some_group_id();
    dm_from(&w, &eve, bob, crate::wire::T_INVITE, &an_invite(&g, "First", now), now).await;
    dm_from(&w, &eve, bob, crate::wire::T_INVITE, &an_invite(&g, "Second", now), now).await;
    let of_g: Vec<_> = w.devices[bob].svc.invites("in").await.unwrap().into_iter().filter(|i| i.group_id == g).collect();
    assert_eq!(of_g.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), vec!["Second"]);
    assert!(!of_g[0].vouched);
    for i in 0..80 {
        dm_from(&w, &Keys::generate(), bob, crate::wire::T_INVITE, &an_invite(&some_group_id(), &format!("S{i}"), now), now).await;
    }
    let shown = w.devices[bob].svc.invites("in").await.unwrap();
    assert_eq!(shown.len(), 51);
    assert!(shown.iter().any(|i| i.invite_id == old.invite_id && i.vouched), "the old row stays");
    // What was pushed out is gone, not kept as a row.
    let rows = messenger_store::groups::invites(&store, "in", &["received", "expired"]).await.unwrap();
    assert_eq!(rows.len(), 51);
}

#[tokio::test]
async fn an_invitation_lapsed_here_and_accepted_on_my_other_device_brings_me_in() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let bob2 = w.device(w.devices[bob].keys.clone()).await;
    let g = w.create(alice, GroupKind::Private, "Team", true).await;
    w.tick();
    let a = &w.devices[alice];
    let (invite, o) = a.svc.invite(&a.keys, &g, &w.pk(bob)).await.unwrap();
    w.run(alice, o).await;
    // Here it lapsed (or was pushed out); there it is answered in time.
    let store = w.devices[bob].svc.store.clone();
    messenger_store::groups::set_invite_status(&store, &invite.invite_id, "expired").await.unwrap();
    w.tick();
    let d = &w.devices[bob2];
    let o = d.svc.answer_invite(&d.keys, &invite.invite_id, true).await.unwrap();
    w.run(bob2, o).await;
    for d in [bob, bob2] {
        assert_eq!(w.devices[d].group(&g).await.expect("in").membership, "joined", "device {d}");
    }
}

/// A text of `who` sealed with the group key as their client would, with
/// nothing asked of the service: what a client that ignores the rules
/// would put on the relay.
async fn forged_text(w: &World, who: usize, group: &str, text: &str) -> Outbound {
    let d = &w.devices[who];
    let log = d.svc.need_log(group).await.unwrap();
    let key = d.svc.key(group, log.state().current_key.as_ref().unwrap()).await.unwrap().unwrap();
    let signed = crate::wire::sign_message(&d.keys, group, &Envelope::text(text).encode(), w.clock.now().secs(), None).unwrap();
    let sealed = crate::wire::seal_message(group, &key, None, &signed, &d.keys).unwrap();
    GroupService::scoped(group, sealed)
}

#[tokio::test]
async fn a_timed_mute_lifts_by_itself() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Team", true).await;
    w.bring(alice, &g, bob).await;
    let until = w.clock.now().secs() + 1_000;
    w.act(alice, &g, OpBody::SetMuted { who: w.pk(bob), muted: true, until: Some(until) }).await.unwrap();
    assert_eq!(code(w.try_say(bob, &g, "now").await), "group_muted_until");
    let v = w.devices[bob].group(&g).await.unwrap();
    assert!(v.muted && !v.can_post);
    assert_eq!(v.muted_until, Some(until));
    let seen_by_alice = w.devices[alice].group(&g).await.unwrap();
    let member = seen_by_alice.members.iter().find(|m| m.pubkey == w.pk(bob).as_hex()).unwrap();
    assert_eq!((member.muted, member.muted_until), (true, Some(until)));
    assert!(w.devices[alice].lines(&g).await.contains(&"group_muted_until".to_string()));
    // A client of bob's that ignores the mute: dropped by everyone else.
    let forged = forged_text(&w, bob, &g, "under the mute").await;
    w.run(bob, Outcome { publish: vec![forged], ..Default::default() }).await;
    assert!(w.devices[alice].texts(&g).await.is_empty());

    // The end passes: no operation, and bob writes again.
    w.clock.0.fetch_add(1_000, Ordering::SeqCst);
    let v = w.devices[bob].group(&g).await.unwrap();
    assert!(!v.muted && v.can_post && v.muted_until.is_none());
    w.say(bob, &g, "free again").await;
    assert_eq!(w.devices[alice].texts(&g).await, vec!["free again"]);
    let member = w.devices[alice].group(&g).await.unwrap().members.into_iter().find(|m| m.pubkey == w.pk(bob).as_hex()).unwrap();
    assert!(!member.muted && member.muted_until.is_none());
    // Muted for good: an end of none, and the old refusal.
    w.act(alice, &g, OpBody::SetMuted { who: w.pk(bob), muted: true, until: None }).await.unwrap();
    assert_eq!(code(w.try_say(bob, &g, "still").await), "group_muted");
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

#[tokio::test]
async fn only_moderators_write_when_the_group_says_so() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let carol = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Team", true).await;
    w.bring(alice, &g, bob).await;
    w.bring(alice, &g, carol).await;
    w.act(alice, &g, Settings { post_mode: Some(PostMode::Managers), ..Settings::default() }.body()).await.unwrap();
    for d in [alice, bob, carol] {
        assert_eq!(w.devices[d].group(&g).await.unwrap().post_mode, "managers", "device {d}");
        assert!(w.devices[d].lines(&g).await.contains(&"group_post_mode".to_string()));
    }
    let v = w.devices[bob].group(&g).await.unwrap();
    assert!(!v.can_post && !v.muted && v.closed_until.is_none());
    assert_eq!(code(w.try_say(bob, &g, "members too?").await), "group_closed");
    let forged = forged_text(&w, bob, &g, "anyway").await;
    w.run(bob, Outcome { publish: vec![forged], ..Default::default() }).await;
    w.say(alice, &g, "managers only").await;
    assert_eq!(w.devices[carol].texts(&g).await, vec!["managers only"]);
    // Made a moderator, carol writes; bob reads.
    w.act(alice, &g, OpBody::SetRole { who: w.pk(carol), role: Role::Moderator }).await.unwrap();
    assert!(w.devices[carol].group(&g).await.unwrap().can_post);
    w.say(carol, &g, "as a moderator").await;
    assert_eq!(w.devices[bob].texts(&g).await, vec!["managers only", "as a moderator"]);
    // Open again.
    w.act(alice, &g, Settings { post_mode: Some(PostMode::All), ..Settings::default() }.body()).await.unwrap();
    assert!(w.devices[bob].group(&g).await.unwrap().can_post);
    w.say(bob, &g, "everyone again").await;
    assert_eq!(w.devices[alice].texts(&g).await, vec!["managers only", "as a moderator", "everyone again"]);
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

#[tokio::test]
async fn the_schedule_closes_the_group_outside_its_hours() {
    let mut w = World::new();
    // Friday 2026-10-16, 10:00 in Berlin (08:00 UTC).
    let berlin = |h: i8, mi: i8| jiff::civil::date(2026, 10, 16).at(h, mi, 0, 0).in_tz("Europe/Berlin").unwrap().timestamp().as_second();
    w.clock.0.store(berlin(10, 0), Ordering::SeqCst);
    let alice = w.person().await;
    let bob = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Team", true).await;
    w.bring(alice, &g, bob).await;
    let hours = Schedule { tz: "Europe/Berlin".into(), days: vec![5], from: "09:00".into(), to: "18:00".into() };
    w.act(alice, &g, Settings { post_schedule: Some(hours.clone()), ..Settings::default() }.body()).await.unwrap();
    let v = w.devices[bob].group(&g).await.unwrap();
    assert_eq!(v.schedule.as_ref(), Some(&hours));
    assert!(v.can_post && v.closed_until.is_none() && !v.schedule_tz_unknown);
    assert!(w.devices[bob].lines(&g).await.contains(&"group_schedule".to_string()));
    w.say(bob, &g, "in hours").await;

    // Six in the evening in Berlin: closed until next Friday, nine.
    w.clock.0.store(berlin(18, 0), Ordering::SeqCst);
    let v = w.devices[bob].group(&g).await.unwrap();
    assert!(!v.can_post);
    let next_friday = jiff::civil::date(2026, 10, 23).at(9, 0, 0, 0).in_tz("Europe/Berlin").unwrap().timestamp().as_second();
    assert_eq!(v.closed_until, Some(next_friday));
    assert_eq!(code(w.try_say(bob, &g, "after hours").await), "group_closed");
    let forged = forged_text(&w, bob, &g, "after hours anyway").await;
    w.run(bob, Outcome { publish: vec![forged], ..Default::default() }).await;
    assert_eq!(w.devices[alice].texts(&g).await, vec!["in hours"]);
    // The owner is never held by the hours.
    assert!(w.devices[alice].group(&g).await.unwrap().can_post);
    w.say(alice, &g, "owner after hours").await;
    assert_eq!(w.devices[bob].texts(&g).await, vec!["in hours", "owner after hours"]);

    // A zone this build does not know: read in UTC (16:00, still open), and said so.
    w.act(alice, &g, Settings { post_schedule: Some(Schedule { tz: "Mars/Olympus".into(), ..hours.clone() }), ..Settings::default() }.body()).await.unwrap();
    let v = w.devices[bob].group(&g).await.unwrap();
    assert!(v.schedule_tz_unknown && v.can_post);
    // Taken away: open at any hour.
    w.act(alice, &g, Settings { post_schedule: Some(Schedule { days: vec![], ..hours }), ..Settings::default() }.body()).await.unwrap();
    let v = w.devices[bob].group(&g).await.unwrap();
    assert!(v.schedule.is_none() && v.can_post && !v.schedule_tz_unknown);
    w.say(bob, &g, "open again").await;
    assert_eq!(w.devices[alice].texts(&g).await, vec!["in hours", "owner after hours", "open again"]);
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

#[tokio::test]
async fn an_operation_newer_than_the_build_waits_for_an_update() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let carol = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Team", true).await;
    w.bring(alice, &g, bob).await;
    w.bring(alice, &g, carol).await;
    w.say(bob, &g, "before").await;

    // Alice mutes carol with an end: an operation of version 2. Bob's
    // device reads version 1 only, as one not updated yet.
    w.tick();
    let until = w.clock.now().secs() + 1_000;
    let d = &w.devices[alice];
    let o = d.svc.act(&d.keys, &g, OpBody::SetMuted { who: w.pk(carol), muted: true, until: Some(until) }).await.unwrap();
    let wires: Vec<Wire> = o
        .publish
        .iter()
        .map(|out| match out {
            Outbound::PublishScoped { event, .. } => Wire::Group(event.clone()),
            other => panic!("{other:?}"),
        })
        .collect();
    for wire in &wires {
        if let Wire::Group(e) = wire {
            w.relay.push(e.clone());
        }
        for i in [alice, carol] {
            let o = w.deliver(i, wire, false).await;
            w.run(i, o).await;
        }
        crate::wire::READS_UP_TO.with(|c| c.set(Some(1)));
        let o = w.deliver(bob, wire, false).await;
        crate::wire::READS_UP_TO.with(|c| c.set(None));
        w.run(bob, o).await;
    }
    let muted_for = |w: &World, who: usize| {
        let svc = w.devices[who].svc.clone();
        let (g, carol) = (g.clone(), w.pk(carol));
        async move { svc.need_log(&g).await.unwrap().state().member(&carol).unwrap().muted }
    };
    assert!(muted_for(&w, alice).await && muted_for(&w, carol).await);
    assert!(!muted_for(&w, bob).await, "bob's device cannot read the operation");
    let v = w.devices[bob].group(&g).await.unwrap();
    assert!(v.needs_update, "and says it waits for an update");
    assert!(!w.devices[alice].group(&g).await.unwrap().needs_update);
    assert!(w.notes.is_empty(), "parked, not dropped: {:?}", w.notes);

    // Meanwhile the group goes on: messages reach bob, and an operation
    // after the one parked waits for it on bob's device.
    w.say(alice, &g, "meanwhile").await;
    assert_eq!(w.devices[bob].texts(&g).await, vec!["before", "meanwhile"]);
    w.act(alice, &g, Settings { name: Some("Renamed".into()), ..Settings::default() }.body()).await.unwrap();
    assert_eq!(w.devices[carol].group(&g).await.unwrap().name, "Renamed");
    assert_eq!(w.devices[bob].group(&g).await.unwrap().name, "Team");
    assert!(!w.devices[bob].lines(&g).await.contains(&"group_muted_until".to_string()));

    // Bob updates the app; at the start of the next session the runtime
    // looks after the group: what was parked is read now.
    w.synced(bob, &g).await;
    assert!(muted_for(&w, bob).await);
    let v = w.devices[bob].group(&g).await.unwrap();
    assert!(!v.needs_update, "nothing waits any more");
    assert_eq!(v.name, "Renamed", "and what waited for it came through");
    assert!(w.devices[bob].lines(&g).await.contains(&"group_muted_until".to_string()));
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

#[tokio::test]
async fn a_welcome_with_an_operation_newer_than_the_build() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let carol = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Team", true).await;
    w.bring(alice, &g, bob).await;
    w.act(alice, &g, OpBody::SetTrusted { who: w.pk(bob), trusted: true }).await.unwrap();
    assert!(w.devices[bob].group(&g).await.unwrap().members.iter().any(|m| m.is_me && m.trusted));

    // Carol's device reads version 1: the welcome's log has an operation
    // it cannot read, and everything after it (her own admission) waits
    // for that one. Not a member by what it reads, it keeps the welcome
    // whole and lists the group as one it is getting into.
    crate::wire::READS_UP_TO.with(|c| c.set(Some(1)));
    w.bring(alice, &g, carol).await;
    let v = w.devices[carol].group(&g).await.expect("listed");
    assert_eq!((v.membership.as_str(), v.needs_update, v.name.as_str()), ("joining", true, "Team"));
    assert!(w.devices[carol].texts(&g).await.is_empty());
    assert!(w.notes.is_empty(), "parked, not dropped: {:?}", w.notes);
    // What the group says meanwhile is kept under the keys the welcome brought.
    w.say(alice, &g, "while carol waits").await;
    // Another start of a session, not updated yet: still waiting.
    w.synced(carol, &g).await;
    let v = w.devices[carol].group(&g).await.unwrap();
    assert_eq!((v.membership.as_str(), v.needs_update), ("joining", true));
    crate::wire::READS_UP_TO.with(|c| c.set(None));
    // What the thread reads is every device's word in a test: a log read
    // while it read less (alice's, after the admission) is read again,
    // as a device does at the start of a session after an update.
    for d in &w.devices {
        d.svc.forget_log(&g).await;
    }

    // Updated: at the start of the next session the welcome is read whole,
    // and what was said meanwhile is there.
    w.synced(carol, &g).await;
    let v = w.devices[carol].group(&g).await.unwrap();
    assert_eq!((v.membership.as_str(), v.needs_update, v.my_role.as_deref()), ("joined", false, Some("member")));
    assert!(v.members.iter().any(|m| m.pubkey == w.pk(bob).as_hex() && m.trusted));
    assert_eq!(w.devices[carol].texts(&g).await, vec!["while carol waits"]);
    w.say(carol, &g, "here at last").await;
    assert_eq!(w.devices[alice].texts(&g).await, vec!["while carol waits", "here at last"]);
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

// ─── Moderation: marks, pre-moderation, the block of direct messages ────────

use crate::marks::ModNote;
use crate::op::Premod;
use messenger_dm::Action as DmAction;

impl Device {
    /// As a moderator sees the chat: what waits for one too.
    async fn moderated(&self, id: &str) -> Vec<MessageView> {
        let mut v = self.dm.messages_as(&format!("group:{id}"), None, 500, true).await.unwrap();
        v.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        v
    }

    async fn texts_as_moderator(&self, id: &str) -> Vec<String> {
        self.moderated(id).await.into_iter().filter(|m| m.content_type == "text" && !m.deleted).filter_map(|m| m.text).collect()
    }

    async fn unread(&self, id: &str) -> i64 {
        self.dm.chat(&format!("group:{id}")).await.unwrap().map(|c| c.unread).unwrap_or(0)
    }

    /// The line of the chat in the list.
    async fn preview(&self, id: &str) -> Option<String> {
        self.dm.chat(&format!("group:{id}")).await.unwrap().and_then(|c| c.last_preview)
    }
}

impl World {
    /// A note of moderation by `who`; `true` when something went out.
    async fn moderate(&mut self, who: usize, group: &str, note: ModNote) -> Result<bool> {
        self.tick();
        let d = &self.devices[who];
        let o = d.svc.moderate(&d.keys, group, note).await?;
        let sent = !o.publish.is_empty();
        self.run(who, o).await;
        Ok(sent)
    }

    /// A note sealed by `who` themselves, whatever their role, dated `at`:
    /// what a client of their own making could send.
    async fn forged_note(&self, who: usize, group: &str, note: &ModNote, at: i64) -> Outbound {
        self.forged_envelope(who, group, &note.envelope(), at).await
    }

    /// Any envelope sealed quietly by `who`, dated `at`.
    async fn forged_envelope(&self, who: usize, group: &str, envelope: &Envelope, at: i64) -> Outbound {
        let d = &self.devices[who];
        let log = d.svc.need_log(group).await.unwrap();
        let key = d.svc.key(group, log.state().current_key.as_ref().unwrap()).await.unwrap().unwrap();
        let signed = crate::wire::sign_message(&d.keys, group, &envelope.encode(), at, None).unwrap();
        let sealed = crate::wire::seal_note(group, &key, None, &signed, &d.keys).unwrap();
        GroupService::scoped(group, sealed)
    }

    /// `who` changes the text of their message; the id of the edit (the
    /// new version).
    async fn edit_versioned(&mut self, who: usize, id: &str, text: &str) -> String {
        self.tick();
        let d = &self.devices[who];
        let (_, version, out) = d.svc.prepare_edit(&d.keys, id, text).await.unwrap();
        self.run(who, Outcome { publish: vec![out], ..Default::default() }).await;
        version
    }

    /// Back online with what was missed delivered in the order given
    /// (indices into the missed list), as history.
    async fn online_in_order(&mut self, i: usize, order: &[usize]) {
        self.devices[i].online = true;
        let missed = std::mem::take(&mut self.devices[i].missed);
        for &k in order {
            let o = self.deliver(i, &missed[k], true).await;
            self.run(i, o).await;
        }
    }

    async fn log_of(&self, who: usize, group: &str) -> Vec<crate::moderation::LogLine> {
        let d = &self.devices[who];
        d.svc.mod_log(group, &d.pk()).await.unwrap()
    }
}

fn permutations<T: Clone>(items: &[T]) -> Vec<Vec<T>> {
    if items.len() <= 1 {
        return vec![items.to_vec()];
    }
    let mut out = Vec::new();
    for i in 0..items.len() {
        let mut rest = items.to_vec();
        let x = rest.remove(i);
        for mut p in permutations(&rest) {
            p.insert(0, x.clone());
            out.push(p);
        }
    }
    out
}

/// A group of an owner, a moderator and two members, everybody seeing
/// the history.
async fn moderated_group(w: &mut World) -> (String, usize, usize, usize, usize) {
    let owner = w.person().await;
    let moderator = w.person().await;
    let u = w.person().await;
    let v = w.person().await;
    let g = w.create(owner, GroupKind::Private, "Town", true).await;
    for p in [moderator, u, v] {
        w.bring(owner, &g, p).await;
    }
    w.act(owner, &g, OpBody::SetRole { who: w.pk(moderator), role: Role::Moderator }).await.unwrap();
    (g, owner, moderator, u, v)
}

#[tokio::test]
async fn a_member_s_mark_has_no_force_and_a_moderator_s_hides_everywhere() {
    let mut w = World::new();
    let (g, owner, moderator, u, v) = moderated_group(&mut w).await;
    let spam = w.say(u, &g, "buy now").await;
    let fine = w.say(owner, &g, "welcome").await;
    let everyone = [owner, moderator, u, v];

    // A member who seals a hide themselves: kept, in force for nobody.
    let forged = w.forged_note(v, &g, &ModNote::Hide { targets: vec![spam.clone()], reason: String::new() }, w.clock.now().secs()).await;
    w.run(v, Outcome { publish: vec![forged], ..Default::default() }).await;
    for d in everyone {
        assert_eq!(w.devices[d].texts(&g).await, vec!["buy now", "welcome"], "device {d}");
    }
    let log = w.log_of(owner, &g).await;
    assert_eq!(log.len(), 1);
    assert!(log[0].action == "hide" && log[0].by == w.pk(v).as_hex() && log[0].detail.contains("\"active\":false"));
    // A member may not even ask.
    assert_eq!(code(w.moderate(v, &g, ModNote::Hide { targets: vec![spam.clone()], reason: String::new() }).await), "group_not_permitted");
    // A moderator may not hide the owner's word, nor their own.
    assert_eq!(code(w.moderate(moderator, &g, ModNote::Hide { targets: vec![fine.clone()], reason: String::new() }).await), "group_not_permitted");
    assert!(w.moderate(moderator, &g, ModNote::Hide { targets: vec!["ab".repeat(32)], reason: String::new() }).await.is_err(), "unknown message");

    // The moderator hides: gone for everyone, the author too.
    assert!(w.moderate(moderator, &g, ModNote::Hide { targets: vec![spam.clone()], reason: "spam".into() }).await.unwrap());
    for d in everyone {
        assert_eq!(w.devices[d].texts(&g).await, vec!["welcome"], "device {d}");
        assert_eq!(w.devices[d].texts_as_moderator(&g).await, vec!["welcome"], "hidden is hidden for a moderator too");
    }
    // Hidden already: nothing goes out a second time.
    assert!(!w.moderate(moderator, &g, ModNote::Hide { targets: vec![spam.clone()], reason: String::new() }).await.unwrap());
    // Shown again by an unhide; the chat list follows.
    assert!(w.moderate(moderator, &g, ModNote::Unhide { targets: vec![spam.clone()] }).await.unwrap());
    for d in everyone {
        assert_eq!(w.devices[d].texts(&g).await, vec!["buy now", "welcome"], "device {d}");
    }
    assert!(!w.moderate(moderator, &g, ModNote::Unhide { targets: vec![spam.clone()] }).await.unwrap(), "shown already");

    // The member whose forged hide lies in the store is raised: the mark
    // comes into force, but the moderator's unhide is newer and stands.
    w.act(owner, &g, OpBody::SetRole { who: w.pk(v), role: Role::Moderator }).await.unwrap();
    for d in everyone {
        assert_eq!(w.devices[d].texts(&g).await, vec!["buy now", "welcome"], "device {d}");
    }
    // The first moderator demoted: their marks lose their force, and the
    // raised member's old hide is all that is left.
    w.act(owner, &g, OpBody::SetRole { who: w.pk(moderator), role: Role::Member }).await.unwrap();
    for d in everyone {
        assert_eq!(w.devices[d].texts(&g).await, vec!["welcome"], "device {d}");
    }
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

#[tokio::test]
async fn a_demoted_moderator_s_marks_lose_their_force_until_the_admin_inherits_them() {
    let mut w = World::new();
    let (g, owner, moderator, u, v) = moderated_group(&mut w).await;
    let a = w.say(u, &g, "a").await;
    let b = w.say(u, &g, "b").await;
    w.say(v, &g, "c").await;
    assert!(w.moderate(moderator, &g, ModNote::Hide { targets: vec![a.clone(), b.clone()], reason: String::new() }).await.unwrap());
    assert!(w.moderate(moderator, &g, ModNote::HideAuthor { author: w.pk(v), from: 0, to: w.clock.now().secs(), reason: String::new() }).await.unwrap());
    let everyone = [owner, moderator, u, v];
    for d in everyone {
        assert!(w.devices[d].texts(&g).await.is_empty(), "device {d}");
    }
    // Demoted: every mark of theirs is out of force, on every device.
    w.act(owner, &g, OpBody::SetRole { who: w.pk(moderator), role: Role::Member }).await.unwrap();
    for d in everyone {
        assert_eq!(w.devices[d].texts(&g).await, vec!["a", "b", "c"], "device {d}");
    }
    assert!(w.log_of(owner, &g).await.iter().filter(|l| l.action == "hide").all(|l| l.detail.contains("\"active\":false")));
    // The admin says the same as their own: hidden again everywhere.
    w.tick();
    let d = &w.devices[owner];
    let (n, o) = d.svc.inherit_marks(&d.keys, &g, &w.pk(moderator)).await.unwrap();
    assert_eq!(n, 3, "two messages and a span");
    w.run(owner, o).await;
    for d in everyone {
        assert!(w.devices[d].texts(&g).await.is_empty(), "device {d}");
    }
    // Said already: nothing a second time.
    let d = &w.devices[owner];
    let (n, _) = d.svc.inherit_marks(&d.keys, &g, &w.pk(moderator)).await.unwrap();
    assert_eq!(n, 0);
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

#[tokio::test]
async fn marks_and_messages_meet_in_any_order_and_end_alike() {
    let mut w = World::new();
    let owner = w.person().await;
    let moderator = w.person().await;
    // Devices of the moderator that will hear everything late, each in
    // another order: made now, so that the welcome reaches them.
    let late: Vec<usize> = {
        let keys = w.devices[moderator].keys.clone();
        let mut v = Vec::new();
        for _ in 0..(120 + 120) {
            v.push(w.device(keys.clone()).await);
        }
        v
    };
    let u = w.person().await;
    let g = w.create(owner, GroupKind::Private, "Town", true).await;
    w.bring(owner, &g, moderator).await;
    w.act(owner, &g, OpBody::SetRole { who: w.pk(moderator), role: Role::Moderator }).await.unwrap();
    w.act(owner, &g, Settings { premod: Some(Premod::Newcomers), ..Settings::default() }.body()).await.unwrap();
    w.bring(owner, &g, u).await;
    for &d in &late {
        assert_eq!(w.devices[d].group(&g).await.unwrap().my_role.as_deref(), Some("moderator"));
        w.offline(d);
    }
    // What happens, in this order, for those who are there: a newcomer's
    // message (held), their edit, the approval of the edit, a hide and an
    // unhide of it, and the demotion of the moderator.
    let m = w.say(u, &g, "hello").await;
    let version = w.edit_versioned(u, &m, "hello there").await;
    assert!(w.moderate(moderator, &g, ModNote::Approve { target: m.clone(), version }).await.unwrap());
    assert!(w.moderate(moderator, &g, ModNote::Hide { targets: vec![m.clone()], reason: String::new() }).await.unwrap());
    assert!(w.moderate(moderator, &g, ModNote::Unhide { targets: vec![m.clone()] }).await.unwrap());
    assert_eq!(w.devices[owner].texts(&g).await, vec!["hello there"], "approved for its new version, shown again");
    let demotion_at = w.devices[late[0]].missed.len();
    w.act(owner, &g, OpBody::SetRole { who: w.pk(moderator), role: Role::Member }).await.unwrap();
    assert_eq!(w.devices[late[0]].missed.len(), demotion_at + 1, "the demotion is one event");
    assert!(w.devices[owner].texts(&g).await.is_empty(), "the approval lost its force: waiting again");
    assert_eq!(w.devices[owner].texts_as_moderator(&g).await, vec!["hello there"]);

    // Without the demotion, in every order: approved and shown.
    let five: Vec<usize> = (0..demotion_at).collect();
    assert_eq!(five.len(), 5, "message, edit, approve, hide, unhide");
    let orders = permutations(&five);
    assert_eq!(orders.len(), 120);
    for (k, order) in orders.iter().enumerate() {
        let d = late[k];
        let mut order = order.clone();
        // The demotion is left where it lies: not delivered.
        order.retain(|&i| i != demotion_at);
        w.online_in_order(d, &order).await;
        assert_eq!(w.devices[d].texts(&g).await, vec!["hello there"], "order {order:?}");
        let row = &w.devices[d].moderated(&g).await.into_iter().find(|x| x.id == m).unwrap();
        assert_eq!(row.review, "ok", "order {order:?}");
    }
    // With the demotion somewhere in the order: nothing of the moderator's
    // holds, the message waits, and a moderator's view still has it.
    let mut six: Vec<Vec<usize>> = Vec::new();
    for order in orders.iter().take(20) {
        for pos in 0..=5 {
            let mut o = order.clone();
            o.insert(pos, demotion_at);
            six.push(o);
        }
    }
    for (k, order) in six.iter().enumerate() {
        let d = late[120 + k];
        w.online_in_order(d, order).await;
        assert!(w.devices[d].texts(&g).await.is_empty(), "order {order:?}");
        let row = &w.devices[d].moderated(&g).await.into_iter().find(|x| x.id == m).unwrap();
        assert_eq!((row.review.as_str(), row.text.as_deref()), ("pending", Some("hello there")), "order {order:?}");
    }
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

#[tokio::test]
async fn a_ban_with_a_hidden_span_outlives_the_unban() {
    let mut w = World::new();
    let (g, owner, moderator, u, v) = moderated_group(&mut w).await;
    let x = w.say(u, &g, "x").await;
    w.say(u, &g, "y").await;
    w.say(v, &g, "z").await;
    let now = w.clock.now().secs();
    w.act(owner, &g, OpBody::Ban { who: w.pk(u) }).await.unwrap();
    assert!(w.moderate(owner, &g, ModNote::HideAuthor { author: w.pk(u), from: 0, to: now, reason: "spam".into() }).await.unwrap());
    for d in [owner, moderator, v] {
        assert_eq!(w.devices[d].texts(&g).await, vec!["z"], "device {d}");
    }
    // The same span again: nothing goes out.
    assert!(!w.moderate(owner, &g, ModNote::HideAuthor { author: w.pk(u), from: 0, to: now, reason: "spam".into() }).await.unwrap());
    // Lifted and let in again: what was hidden stays hidden, the new word shows.
    w.act(owner, &g, OpBody::Unban { who: w.pk(u) }).await.unwrap();
    // Let in again by an invitation, as anyone who is not a member.
    w.bring(owner, &g, u).await;
    assert_eq!(w.devices[u].group(&g).await.unwrap().membership, "joined");
    // What the group said while they were out comes as history, the
    // span among it: hidden for them as for everyone.
    w.catch_up(u).await;
    w.say(u, &g, "back").await;
    for d in [owner, moderator, u, v] {
        assert_eq!(w.devices[d].texts(&g).await, vec!["z", "back"], "device {d}");
    }
    // One of the span shown again by name.
    assert!(w.moderate(moderator, &g, ModNote::Unhide { targets: vec![x] }).await.unwrap());
    for d in [owner, moderator, u, v] {
        assert_eq!(w.devices[d].texts(&g).await, vec!["x", "z", "back"], "device {d}");
    }
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

#[tokio::test]
async fn a_mark_dated_ahead_of_the_clock_or_before_its_message_is_dropped() {
    let mut w = World::new();
    let (g, owner, moderator, u, _) = moderated_group(&mut w).await;
    let m = w.say(u, &g, "m").await;
    let created = w.devices[owner].moderated(&g).await.into_iter().find(|x| x.id == m).unwrap().created_at;
    let note = ModNote::Hide { targets: vec![m.clone()], reason: String::new() };
    let ahead = w.forged_note(moderator, &g, &note, w.clock.now().secs() + crate::marks::SKEW_SECS + 1).await;
    w.run(moderator, Outcome { publish: vec![ahead], ..Default::default() }).await;
    assert_eq!(w.devices[owner].texts(&g).await, vec!["m"]);
    assert!(w.log_of(owner, &g).await.is_empty(), "dated ahead: not kept");
    assert!(w.notes.iter().all(|n| n.contains("dated ahead")), "{:?}", w.notes);
    w.notes.clear();
    let before = w.forged_note(moderator, &g, &note, created - 1).await;
    w.run(moderator, Outcome { publish: vec![before], ..Default::default() }).await;
    assert_eq!(w.devices[owner].texts(&g).await, vec!["m"]);
    assert!(w.log_of(owner, &g).await.is_empty(), "before its message: not kept");
    // Inside the window: in force.
    let inside = w.forged_note(moderator, &g, &note, w.clock.now().secs() + crate::marks::SKEW_SECS).await;
    w.run(moderator, Outcome { publish: vec![inside], ..Default::default() }).await;
    assert!(w.devices[owner].texts(&g).await.is_empty());
    // A note of a kind this build does not know: quietly nothing.
    let unknown = w.forged_envelope(moderator, &g, &Envelope::new("mod.later").with("targets", vec![m.clone()]), w.clock.now().secs()).await;
    w.run(moderator, Outcome { publish: vec![unknown], ..Default::default() }).await;
    assert!(w.notes.is_empty(), "{:?}", w.notes);
    assert_eq!(w.log_of(owner, &g).await.len(), 1);
}

#[tokio::test]
async fn premod_holds_a_newcomer_until_a_moderator_lets_them_through() {
    let mut w = World::new();
    let (g, owner, moderator, u, _) = moderated_group(&mut w).await;
    w.act(owner, &g, Settings { premod: Some(Premod::Newcomers), ..Settings::default() }.body()).await.unwrap();
    let n = w.person().await;
    w.bring(owner, &g, n).await;
    // The newcomer speaks; the message is handed to each device by hand,
    // so that what each one does with it is seen: nobody is woken, a
    // member or a moderator alike.
    w.tick();
    let d = &w.devices[n];
    let (m, out) = d.svc.prepare_text(&d.keys, &g, "hi all", None).await.unwrap();
    let hi = m.id;
    let Outbound::PublishScoped { event, .. } = &out else { panic!("{out:?}") };
    w.relay.push(event.clone());
    let wire = Wire::Group(event.clone());
    for i in [u, moderator, owner, n] {
        let o = w.deliver(i, &wire, false).await;
        assert!(o.notify.is_empty(), "device {i} is not woken for what waits");
        w.run(i, o).await;
    }
    // Members do not see it and are not told of it; moderators see it,
    // the author sees their own with the mark of waiting.
    assert!(w.devices[u].texts(&g).await.is_empty());
    assert_eq!(w.devices[u].unread(&g).await, 0, "nothing to read for a member");
    assert_eq!(w.devices[owner].texts(&g).await, vec![] as Vec<String>, "a member's view, even of the owner's device");
    assert_eq!(w.devices[owner].texts_as_moderator(&g).await, vec!["hi all"]);
    assert_eq!(w.devices[moderator].unread(&g).await, 0, "waiting counts for nobody");
    let own = w.devices[n].visible(&g).await.into_iter().find(|m| m.id == hi).unwrap();
    assert_eq!(own.review, "pending");
    assert_eq!(w.devices[moderator].group(&g).await.unwrap().queue, 1);
    assert_eq!(w.devices[u].group(&g).await.unwrap().queue, 0);
    // The old member is not held.
    w.say(u, &g, "old hand").await;
    assert_eq!(w.devices[owner].texts(&g).await, vec!["old hand"]);
    // An approval of the wrong version is refused, of the right one shows it.
    assert_eq!(code(w.moderate(moderator, &g, ModNote::Approve { target: hi.clone(), version: "ab".repeat(32) }).await), "group_version_stale");
    assert!(w.moderate(moderator, &g, ModNote::Approve { target: hi.clone(), version: hi.clone() }).await.unwrap());
    for d in [owner, moderator, u, n] {
        assert_eq!(w.devices[d].texts(&g).await, vec!["hi all", "old hand"], "device {d}");
        let row = w.devices[d].visible(&g).await.into_iter().find(|m| m.id == hi).unwrap();
        assert_eq!(row.review, "ok");
    }
    assert_eq!(w.devices[moderator].group(&g).await.unwrap().queue, 0);
    assert!(!w.moderate(moderator, &g, ModNote::Approve { target: hi.clone(), version: hi.clone() }).await.unwrap(), "approved already");
    // Edited: a new version waits again; the old approval is of the old one.
    let v2 = w.edit_versioned(n, &hi, "hi all, sorry").await;
    assert_eq!(w.devices[u].texts(&g).await, vec!["old hand"]);
    assert_eq!(w.devices[moderator].texts_as_moderator(&g).await, vec!["hi all, sorry", "old hand"]);
    assert_eq!(code(w.moderate(moderator, &g, ModNote::Approve { target: hi.clone(), version: hi.clone() }).await), "group_version_stale");
    assert!(w.moderate(moderator, &g, ModNote::Approve { target: hi.clone(), version: v2 }).await.unwrap());
    assert_eq!(w.devices[u].texts(&g).await, vec!["hi all, sorry", "old hand"]);
    // Rejected: hidden for everyone but its author, who is shown it with
    // the word that it was turned down.
    assert!(w.moderate(moderator, &g, ModNote::Reject { target: hi.clone() }).await.unwrap());
    for d in [owner, moderator, u] {
        assert_eq!(w.devices[d].texts(&g).await, vec!["old hand"], "device {d}");
        assert_eq!(w.devices[d].texts_as_moderator(&g).await, vec!["old hand"], "device {d}");
    }
    assert_eq!(w.devices[n].texts(&g).await, vec!["hi all, sorry", "old hand"]);
    let own = w.devices[n].visible(&g).await.into_iter().find(|m| m.id == hi).unwrap();
    assert_eq!(own.review, "rejected");
    assert!(!w.moderate(moderator, &g, ModNote::Reject { target: hi.clone() }).await.unwrap(), "rejected already");
    // What was never held takes no approval.
    let old = w.devices[u].visible(&g).await.into_iter().find(|m| m.text.as_deref() == Some("old hand")).unwrap().id;
    assert_eq!(code(w.moderate(moderator, &g, ModNote::Approve { target: old.clone(), version: old }).await), "group_not_pending");
    // Trusted: not held any more.
    w.act(moderator, &g, OpBody::SetTrusted { who: w.pk(n), trusted: true }).await.unwrap();
    w.say(n, &g, "trusted now").await;
    assert_eq!(w.devices[u].texts(&g).await, vec!["old hand", "trusted now"]);
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

#[tokio::test]
async fn a_block_in_direct_messages_hides_the_author_for_me_alone() {
    let mut w = World::new();
    let (g, owner, _, u, v) = moderated_group(&mut w).await;
    w.say(u, &g, "x").await;
    w.tick();
    w.say(owner, &g, "o").await;
    assert_eq!((w.devices[v].unread(&g).await, w.devices[v].preview(&g).await.as_deref()), (2, Some("o")));
    let d = &w.devices[v];
    d.dm.act(&d.keys, &w.pk(u), DmAction::Block).await.unwrap();
    assert_eq!(w.devices[v].texts(&g).await, vec!["o"], "blocked: gone for me");
    assert_eq!(w.devices[owner].texts(&g).await, vec!["x", "o"], "for nobody else");
    assert_eq!(w.devices[v].unread(&g).await, 1, "what they wrote stops counting at once");
    w.say(u, &g, "y").await;
    assert_eq!(w.devices[v].texts(&g).await, vec!["o"]);
    assert_eq!((w.devices[v].unread(&g).await, w.devices[v].preview(&g).await.as_deref()), (1, Some("o")), "not counted, not the line of the list");
    assert_eq!(w.devices[owner].texts(&g).await, vec!["x", "o", "y"]);
    assert_eq!(w.devices[owner].preview(&g).await.as_deref(), Some("y"));
    let d = &w.devices[v];
    d.dm.act(&d.keys, &w.pk(u), DmAction::Unblock).await.unwrap();
    assert_eq!(w.devices[v].texts(&g).await, vec!["x", "o", "y"], "the unblock shows them again");
    assert_eq!((w.devices[v].unread(&g).await, w.devices[v].preview(&g).await.as_deref()), (1, Some("y")), "in the list too; nothing rings for it");
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

#[tokio::test]
async fn what_waited_is_shown_while_the_journal_does_not_hold_its_author_and_waits_again_when_it_does() {
    let mut w = World::new();
    let owner = w.person().await;
    let moderator = w.person().await;
    let u = w.person().await;
    // A device of u that hears everything late, the journal before the message.
    let u_keys = w.devices[u].keys.clone();
    let u2 = w.device(u_keys).await;
    let n = w.person().await;
    let g = w.create(owner, GroupKind::Private, "Town", true).await;
    for p in [moderator, u] {
        w.bring(owner, &g, p).await;
    }
    w.act(owner, &g, OpBody::SetRole { who: w.pk(moderator), role: Role::Moderator }).await.unwrap();
    w.act(owner, &g, Settings { premod: Some(Premod::Newcomers), ..Settings::default() }.body()).await.unwrap();
    w.bring(owner, &g, n).await;
    w.say(u, &g, "old").await;
    w.offline(u2);
    let hi = w.say(n, &g, "hi").await;
    assert_eq!(w.devices[u].texts(&g).await, vec!["old"]);
    assert_eq!(w.devices[moderator].group(&g).await.unwrap().queue, 1);

    // Pre-moderation off: what waited is shown, on every device, and on
    // one that hears of the rule before the message.
    w.act(owner, &g, Settings { premod: Some(Premod::Off), ..Settings::default() }.body()).await.unwrap();
    for d in [owner, moderator, u, n] {
        assert_eq!(w.devices[d].texts(&g).await, vec!["old", "hi"], "device {d}");
        let row = w.devices[d].visible(&g).await.into_iter().find(|m| m.id == hi).unwrap();
        assert_eq!(row.review, "ok", "device {d}");
    }
    assert_eq!(w.devices[moderator].group(&g).await.unwrap().queue, 0);
    assert_eq!(w.devices[u].preview(&g).await.as_deref(), Some("hi"), "in the list now");
    let missed = w.devices[u2].missed.len();
    assert!(missed >= 2, "the message and the rule off");
    let order: Vec<usize> = (0..missed).rev().collect();
    w.online_in_order(u2, &order).await;
    assert_eq!(w.devices[u2].texts(&g).await, vec!["old", "hi"], "the late device ends the same");

    // On again, for everybody: what was held once waits again; what was
    // never held does not start to.
    w.act(owner, &g, Settings { premod: Some(Premod::All), ..Settings::default() }.body()).await.unwrap();
    for d in [u, u2] {
        assert_eq!(w.devices[d].texts(&g).await, vec!["old"], "device {d}");
    }
    assert_eq!(w.devices[moderator].group(&g).await.unwrap().queue, 1);
    assert_eq!(w.devices[moderator].texts_as_moderator(&g).await, vec!["old", "hi"]);
    // Trusted: released. Not trusted any more: waits again.
    w.act(moderator, &g, OpBody::SetTrusted { who: w.pk(n), trusted: true }).await.unwrap();
    assert_eq!(w.devices[u].texts(&g).await, vec!["old", "hi"]);
    w.act(moderator, &g, OpBody::SetTrusted { who: w.pk(n), trusted: false }).await.unwrap();
    assert_eq!(w.devices[u].texts(&g).await, vec!["old"]);
    // Raised: released. A member again: waits again.
    w.act(owner, &g, OpBody::SetRole { who: w.pk(n), role: Role::Moderator }).await.unwrap();
    assert_eq!(w.devices[u].texts(&g).await, vec!["old", "hi"]);
    w.act(owner, &g, OpBody::SetRole { who: w.pk(n), role: Role::Member }).await.unwrap();
    assert_eq!(w.devices[u].texts(&g).await, vec!["old"]);
    // An edit while released makes it wait only while the author is held.
    w.act(owner, &g, Settings { premod: Some(Premod::Off), ..Settings::default() }.body()).await.unwrap();
    w.edit_versioned(n, &hi, "hi there").await;
    assert_eq!(w.devices[u].texts(&g).await, vec!["old", "hi there"]);
    // A decision stands whatever the rule says after it.
    w.act(owner, &g, Settings { premod: Some(Premod::All), ..Settings::default() }.body()).await.unwrap();
    assert_eq!(w.devices[u].texts(&g).await, vec!["old"]);
    let version = w.devices[moderator].svc.queue(&g, &w.pk(moderator), &msgs::QueueFilter::default(), None, 10).await.unwrap().0[0].version.clone();
    assert!(w.moderate(moderator, &g, ModNote::Approve { target: hi.clone(), version }).await.unwrap());
    assert_eq!(w.devices[u].texts(&g).await, vec!["old", "hi there"]);
    w.act(owner, &g, Settings { premod: Some(Premod::Off), ..Settings::default() }.body()).await.unwrap();
    w.act(owner, &g, Settings { premod: Some(Premod::All), ..Settings::default() }.body()).await.unwrap();
    assert_eq!(w.devices[u].texts(&g).await, vec!["old", "hi there"], "approved: not waiting again");
    assert_eq!(w.devices[moderator].group(&g).await.unwrap().queue, 0);
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

impl World {
    /// An operation sealed by `who` themselves, as their own client could
    /// make it: of any version, in anybody's name.
    async fn forged_op(&self, who: usize, group: &str, op: &crate::op::Op, signer: Option<&Keys>) -> Outbound {
        let d = &self.devices[who];
        let log = d.svc.need_log(group).await.unwrap();
        let key = d.svc.key(group, log.state().current_key.as_ref().unwrap()).await.unwrap().unwrap();
        use nostr::prelude::*;
        let signer = signer.unwrap_or(&d.keys);
        let content = op.canonical();
        let signed = EventBuilder::new(Kind::from(crate::wire::KIND_INNER_OP), content)
            .tag(Tag::parse(["h", group]).unwrap())
            .custom_created_at(Timestamp::from_secs(op.created_at.max(0) as u64))
            .finalize(signer)
            .unwrap();
        let sealed = crate::wire::seal_op(group, &key, &signed, vec![], &d.keys).unwrap();
        GroupService::scoped(group, sealed)
    }

    async fn parked_ops(&self, who: usize, group: &str) -> i64 {
        messenger_store::groups::pending_prefix_size(&self.devices[who].svc.store, group, "future:").await.unwrap().0
    }
}

#[tokio::test]
async fn an_operation_of_an_absurd_version_or_in_another_s_name_is_junk_not_the_future() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Team", true).await;
    w.bring(alice, &g, bob).await;
    let reads = crate::op::OP_VERSION;
    // Bob holds the key: he can seal anything. A version far past what any
    // build reads is dropped, and nobody waits for an update over it.
    let log = w.devices[bob].svc.need_log(&g).await.unwrap();
    let mut absurd = log.next(&w.pk(bob), w.clock.now().secs(), OpBody::RotateKey);
    absurd.v = reads + crate::wire::AHEAD_WINDOW + 1;
    let out = w.forged_op(bob, &g, &absurd, None).await;
    w.run(bob, Outcome { publish: vec![out], ..Default::default() }).await;
    assert!(!w.devices[alice].group(&g).await.unwrap().needs_update, "junk pins nothing");
    assert_eq!(w.parked_ops(alice, &g).await, 0);
    assert_eq!(w.notes.len(), 2, "dropped on both devices: {:?}", w.notes);
    w.notes.clear();
    // The next version, in alice's name, signed by bob: a forgery is not
    // parked either, however new it says it is.
    let mut forged = log.next(&w.pk(alice), w.clock.now().secs(), OpBody::RotateKey);
    forged.v = reads + 1;
    let bob_keys = w.devices[bob].keys.clone();
    let out = w.forged_op(bob, &g, &forged, Some(&bob_keys)).await;
    w.run(bob, Outcome { publish: vec![out], ..Default::default() }).await;
    assert!(!w.devices[alice].group(&g).await.unwrap().needs_update);
    assert_eq!(w.parked_ops(alice, &g).await, 0);
    assert_eq!(w.notes.len(), 2, "{:?}", w.notes);
    w.notes.clear();
    // The next version in bob's own name is the future: parked, waited for.
    let mut ahead = log.next(&w.pk(bob), w.clock.now().secs(), OpBody::RotateKey);
    ahead.v = reads + 1;
    let out = w.forged_op(bob, &g, &ahead, None).await;
    w.run(bob, Outcome { publish: vec![out], ..Default::default() }).await;
    assert!(w.devices[alice].group(&g).await.unwrap().needs_update);
    assert_eq!(w.parked_ops(alice, &g).await, 1);
    assert!(w.notes.is_empty(), "parked, not dropped: {:?}", w.notes);
}

#[tokio::test]
async fn a_flood_of_what_the_build_cannot_read_evicts_its_own_oldest_and_never_the_welcome() {
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let carol = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Team", true).await;
    w.bring(alice, &g, bob).await;
    w.act(alice, &g, OpBody::SetTrusted { who: w.pk(bob), trusted: true }).await.unwrap();
    // Carol's device reads version 1 and parks at most three events.
    crate::wire::READS_UP_TO.with(|c| c.set(Some(1)));
    crate::inbound::FUTURE_ROWS_CAP.with(|c| c.set(Some(3)));
    w.bring(alice, &g, carol).await;
    assert_eq!(w.devices[carol].group(&g).await.unwrap().membership, "joining", "the welcome waits for an update");
    // Bob floods the group with operations of a version past every
    // build here, each dated earlier than the one before: carol keeps the
    // last three by their receipt, and the welcome beside them.
    let log = w.devices[bob].svc.need_log(&g).await.unwrap();
    for i in 0..5 {
        let mut op = log.next(&w.pk(bob), w.clock.now().secs() - i, OpBody::RotateKey);
        op.v = crate::op::OP_VERSION + 1;
        let out = w.forged_op(bob, &g, &op, None).await;
        w.run(bob, Outcome { publish: vec![out], ..Default::default() }).await;
    }
    assert_eq!(w.parked_ops(carol, &g).await, 3);
    let kept: Vec<String> = messenger_store::groups::pending_in(&w.devices[carol].svc.store, &g, &format!("future:{}", log.state().current_key.as_ref().unwrap().0))
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.event_id)
        .collect();
    assert_eq!(kept.len(), 3);
    let v = w.devices[carol].group(&g).await.unwrap();
    assert_eq!((v.membership.as_str(), v.needs_update), ("joining", true), "the welcome is still there");
    crate::inbound::FUTURE_ROWS_CAP.with(|c| c.set(None));
    crate::wire::READS_UP_TO.with(|c| c.set(None));
    // Updated: the welcome is read, carol is in.
    w.synced(carol, &g).await;
    let v = w.devices[carol].group(&g).await.unwrap();
    assert_eq!((v.membership.as_str(), v.my_role.as_deref()), ("joined", Some("member")));
    // The three kept were the last three to come: the first two by receipt
    // went, though they were dated latest.
    let relay_ids: Vec<String> = w.relay.iter().map(|e| e.id.as_hex().to_string()).collect();
    let last_three: Vec<&String> = relay_ids.iter().rev().take(3).collect();
    for id in &kept {
        assert!(last_three.contains(&id), "kept by receipt, not by date");
    }
    assert!(w.notes.is_empty(), "parked, not dropped: {:?}", w.notes);
}

#[tokio::test]
async fn the_queue_is_one_line_per_author_and_pages() {
    let mut w = World::new();
    let (g, owner, moderator, u, v) = moderated_group(&mut w).await;
    w.act(owner, &g, Settings { premod: Some(Premod::All), ..Settings::default() }.body()).await.unwrap();
    let u1 = w.say(u, &g, "u1").await;
    w.say(u, &g, "u2").await;
    w.say(u, &g, "u3").await;
    let v1 = w.say(v, &g, "v1").await;
    w.say(v, &g, "v2").await;
    let d = &w.devices[moderator];
    let me = d.pk();
    let all = msgs::QueueFilter::default();
    let (page, total) = d.svc.queue(&g, &me, &all, None, 1).await.unwrap();
    assert_eq!(total, 5, "every waiting row counts");
    assert_eq!(page.len(), 1);
    assert_eq!((page[0].row.id.as_str(), page[0].more_from_author, page[0].version.as_str()), (v1.as_str(), 1, v1.as_str()), "the newest author first, their oldest");
    let (next, _) = d.svc.queue(&g, &me, &all, Some(page[0].row.created_at), 1).await.unwrap();
    assert_eq!((next[0].row.id.as_str(), next[0].more_from_author), (u1.as_str(), 2));
    let (end, _) = d.svc.queue(&g, &me, &all, Some(next[0].row.created_at), 1).await.unwrap();
    assert!(end.is_empty());
    // By author, by text, by time.
    let of_u = msgs::QueueFilter { author: Some(w.pk(u).as_hex().to_string()), ..Default::default() };
    let (page, total) = d.svc.queue(&g, &me, &of_u, None, 10).await.unwrap();
    assert_eq!((page.len(), total, page[0].more_from_author), (1, 3, 2));
    let by_text = msgs::QueueFilter { text: Some("2".into()), ..Default::default() };
    let (page, total) = d.svc.queue(&g, &me, &by_text, None, 10).await.unwrap();
    assert_eq!(total, 2);
    assert!(page.iter().all(|l| l.more_from_author == 0), "the count behind a line is under the filter too");
    let since = page[0].row.created_at;
    let late = msgs::QueueFilter { since: Some(since), ..Default::default() };
    assert!(d.svc.queue(&g, &me, &late, None, 10).await.unwrap().1 < 5);
    // A member may not look.
    let member = &w.devices[u];
    assert_eq!(code(member.svc.queue(&g, &member.pk(), &all, None, 10).await), "group_not_permitted");
    assert_eq!(code(member.svc.mod_log(&g, &member.pk()).await), "group_not_permitted");
    // Decided: out of the queue; the count on the group follows.
    assert_eq!(w.devices[moderator].group(&g).await.unwrap().queue, 5);
    assert!(w.moderate(moderator, &g, ModNote::Approve { target: u1.clone(), version: u1.clone() }).await.unwrap());
    assert!(w.moderate(moderator, &g, ModNote::Reject { target: v1.clone() }).await.unwrap());
    assert_eq!(w.devices[moderator].group(&g).await.unwrap().queue, 3);
    assert_eq!(w.devices[owner].svc.queue(&g, &w.pk(owner), &all, None, 10).await.unwrap().1, 3);
    // The log has it all, newest first.
    let log = w.log_of(owner, &g).await;
    assert_eq!(log.iter().map(|l| l.action.as_str()).collect::<Vec<_>>(), vec!["reject", "approve", "settings"]);
    assert!(log[2].detail.contains("\"premod\":\"all\""));
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}


#[tokio::test]
async fn a_message_that_outran_the_rule_is_held_when_the_rule_comes() {
    let mut w = World::new();
    let owner = w.person().await;
    let moderator = w.person().await;
    let u = w.person().await;
    let u_keys = w.devices[u].keys.clone();
    let u2 = w.device(u_keys).await;
    let n = w.person().await;
    let n_keys = w.devices[n].keys.clone();
    let n2 = w.device(n_keys).await;
    let g = w.create(owner, GroupKind::Private, "Town", true).await;
    for p in [moderator, u, n] {
        w.bring(owner, &g, p).await;
    }
    w.act(owner, &g, OpBody::SetRole { who: w.pk(moderator), role: Role::Moderator }).await.unwrap();
    w.say(n, &g, "old").await;
    w.offline(u2);
    w.offline(n2);
    // The rule on, then the newcomer speaks: everybody online holds it.
    w.act(owner, &g, Settings { premod: Some(Premod::All), ..Settings::default() }.body()).await.unwrap();
    let hi = w.say(n, &g, "hi").await;
    assert_eq!(w.devices[u].texts(&g).await, vec!["old"]);
    assert_eq!(w.devices[moderator].group(&g).await.unwrap().queue, 1);
    // The late devices hear the message before the rule (a relay hands
    // the newest first): a member's, and the author's own.
    assert_eq!(w.devices[u2].missed.len(), 2, "the rule and the message");
    w.online_in_order(u2, &[1, 0]).await;
    assert_eq!(w.devices[u2].texts(&g).await, vec!["old"], "the late device ends the same: held");
    let row = w.devices[u2].moderated(&g).await.into_iter().find(|m| m.id == hi).unwrap();
    assert_eq!(row.review, "pending");
    w.online_in_order(n2, &[1, 0]).await;
    let own = w.devices[n2].visible(&g).await.into_iter().find(|m| m.id == hi).unwrap();
    assert_eq!(own.review, "pending", "the author's other device shows it as waiting too");
    // What is dated before the rule is nobody's to hold, however late the rule comes.
    let old = w.devices[u2].visible(&g).await.into_iter().find(|m| m.text.as_deref() == Some("old")).unwrap();
    assert_eq!(old.review, "ok");
    // The decision reaches the late device like any other.
    assert!(w.moderate(moderator, &g, ModNote::Approve { target: hi.clone(), version: hi.clone() }).await.unwrap());
    for d in [owner, moderator, u, u2, n, n2] {
        assert_eq!(w.devices[d].texts(&g).await, vec!["old", "hi"], "device {d}");
    }
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

#[tokio::test]
async fn the_admin_inherits_approvals_rejections_and_unhidings_and_passes_a_stale_approval_by() {
    let mut w = World::new();
    let (g, owner, moderator, u, v) = moderated_group(&mut w).await;
    w.act(owner, &g, Settings { premod: Some(Premod::All), ..Settings::default() }.body()).await.unwrap();
    let p1 = w.say(u, &g, "p1").await;
    let p2 = w.say(u, &g, "p2").await;
    let p3 = w.say(u, &g, "p3").await;
    let q1 = w.say(v, &g, "q1").await;
    assert!(w.moderate(moderator, &g, ModNote::Approve { target: p1.clone(), version: p1.clone() }).await.unwrap());
    assert!(w.moderate(moderator, &g, ModNote::Approve { target: p2.clone(), version: p2.clone() }).await.unwrap());
    assert!(w.moderate(moderator, &g, ModNote::Reject { target: p3.clone() }).await.unwrap());
    assert!(w.moderate(moderator, &g, ModNote::Hide { targets: vec![q1.clone()], reason: String::new() }).await.unwrap());
    assert!(w.moderate(moderator, &g, ModNote::Unhide { targets: vec![q1.clone()] }).await.unwrap());
    // Edited since: the approval of p2 is of a version that is gone.
    w.edit_versioned(u, &p2, "p2 again").await;
    // What the store says of a row (a rejected one is listed to nobody
    // but its author).
    let review = |w: &World, d: usize, id: &str| {
        let (store, id) = (w.devices[d].svc.store.clone(), id.to_string());
        async move { msgs::get(&store, &id).await.unwrap().map(|r| r.review) }
    };
    assert_eq!(w.devices[owner].texts(&g).await, vec!["p1"]);
    assert_eq!(review(&w, owner, &p3).await, Some("rejected".into()));

    // Demoted: every decision of theirs is out of force everywhere.
    w.act(owner, &g, OpBody::SetRole { who: w.pk(moderator), role: Role::Member }).await.unwrap();
    let everyone = [owner, moderator, u, v];
    for d in [owner, moderator] {
        assert!(w.devices[d].texts(&g).await.is_empty(), "device {d}: nothing approved any more");
    }
    for d in everyone {
        assert_eq!(review(&w, d, &p3).await, Some("pending".into()), "device {d}: the rejection is gone");
    }
    // The admin says them again: the approval of p1, the rejection of
    // p3, the unhiding of q1 (the newest about it; the hiding before it
    // is not said). The approval of p2 cannot be: its version is stale.
    w.tick();
    let d = &w.devices[owner];
    let (n, o) = d.svc.inherit_marks(&d.keys, &g, &w.pk(moderator)).await.unwrap();
    assert_eq!(n, 3, "an approval, a rejection, an unhiding");
    w.run(owner, o).await;
    assert_eq!(w.notes.len(), 1, "{:?}", w.notes);
    assert!(w.notes[0].contains("group_version_stale"), "{:?}", w.notes);
    w.notes.clear();
    for d in [owner, moderator] {
        assert_eq!(w.devices[d].texts(&g).await, vec!["p1"], "device {d}");
    }
    for d in everyone {
        assert_eq!(review(&w, d, &p1).await, Some("approved".into()), "device {d}");
        assert_eq!(review(&w, d, &p2).await, Some("pending".into()), "device {d}: edited, waits for an approval of the new version");
        assert_eq!(review(&w, d, &p3).await, Some("rejected".into()), "device {d}");
        assert_eq!(review(&w, d, &q1).await, Some("pending".into()), "device {d}: shown to moderators, not hidden");
    }
    let log = w.log_of(owner, &g).await;
    let mut mine: Vec<&str> = log.iter().filter(|l| l.by == w.pk(owner).as_hex()).map(|l| l.action.as_str()).collect();
    mine.sort_unstable();
    assert_eq!(mine, vec!["approve", "reject", "settings", "unhide"]);
    // Said already: nothing a second time.
    let d = &w.devices[owner];
    let (n, o) = d.svc.inherit_marks(&d.keys, &g, &w.pk(moderator)).await.unwrap();
    assert_eq!((n, o.publish.len()), (0, 0));
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

#[tokio::test]
async fn two_moderators_dated_alike_end_alike_in_either_order() {
    let mut w = World::new();
    let (g, owner, m1, u, m2) = moderated_group(&mut w).await;
    w.act(owner, &g, OpBody::SetRole { who: w.pk(m2), role: Role::Moderator }).await.unwrap();
    w.act(owner, &g, Settings { premod: Some(Premod::Newcomers), ..Settings::default() }.body()).await.unwrap();
    let n = w.person().await;
    w.bring(owner, &g, n).await;
    let x = w.say(u, &g, "x").await;
    let y = w.say(n, &g, "y").await;
    assert_eq!(w.devices[owner].texts(&g).await, vec!["x"]);
    // Two devices away while the two moderators decide in the same second,
    // the one against the other.
    w.offline(owner);
    w.offline(u);
    w.tick();
    let at = w.clock.now().secs();
    let notes = [
        (m1, ModNote::Hide { targets: vec![x.clone()], reason: String::new() }),
        (m2, ModNote::Unhide { targets: vec![x.clone()] }),
        (m1, ModNote::Approve { target: y.clone(), version: y.clone() }),
        (m2, ModNote::Reject { target: y.clone() }),
    ];
    for (who, note) in &notes {
        let out = w.forged_note(*who, &g, note, at).await;
        w.run(*who, Outcome { publish: vec![out], ..Default::default() }).await;
    }
    // Back, each in another order: the stricter stands on both, as on
    // the devices that heard them live.
    w.online_in_order(owner, &[0, 1, 2, 3]).await;
    w.online_in_order(u, &[3, 2, 1, 0]).await;
    for d in [owner, u, m1, m2] {
        assert!(w.devices[d].texts_as_moderator(&g).await.is_empty(), "device {d}: the hiding over the showing, the rejection over the approval");
    }
    assert_eq!(w.devices[n].texts(&g).await, vec!["y"], "the author alone is shown what was turned down");
    for d in [owner, u, m1, m2, n] {
        let row = msgs::get(&w.devices[d].svc.store, &y).await.unwrap().unwrap();
        assert_eq!(row.review, "rejected", "device {d}");
    }
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

#[tokio::test]
async fn an_unblock_opens_only_what_the_block_closed() {
    let mut w = World::new();
    let (g, owner, moderator, u, v) = moderated_group(&mut w).await;
    let x = w.say(u, &g, "x").await;
    w.say(u, &g, "y").await;
    let d = &w.devices[v];
    d.dm.act(&d.keys, &w.pk(u), DmAction::Block).await.unwrap();
    assert!(w.devices[v].texts(&g).await.is_empty());
    // While the block stands, a moderator hides one of them for everyone.
    assert!(w.moderate(moderator, &g, ModNote::Hide { targets: vec![x], reason: String::new() }).await.unwrap());
    assert_eq!(w.devices[owner].texts(&g).await, vec!["y"]);
    let d = &w.devices[v];
    d.dm.act(&d.keys, &w.pk(u), DmAction::Unblock).await.unwrap();
    assert_eq!(w.devices[v].texts(&g).await, vec!["y"], "the unblock shows what the block alone hid");
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

/// What the runtime does when the manifest names `banned` (and nobody
/// else): the table, then the groups they wrote in counted again.
async fn ban_on(w: &World, device: usize, banned: &[usize]) {
    let store = w.devices[device].dm.store();
    let keys: Vec<String> = banned.iter().map(|b| w.pk(*b).as_hex().to_string()).collect();
    for hex in messenger_store::banned::replace(store, &keys).await.unwrap() {
        messenger_store::chats::group_chats_follow_block(store, &hex).await.unwrap();
    }
}

#[tokio::test]
async fn a_key_the_network_bans_is_hidden_in_groups_without_a_note_and_brings_nobody_in() {
    let mut w = World::new();
    let (g, owner, _, u, v) = moderated_group(&mut w).await;
    // Two groups of u, one invitation to v that came before the ban.
    let mine = w.create(u, GroupKind::Private, "Mine", true).await;
    let other = w.create(u, GroupKind::Private, "Other", true).await;
    let d = &w.devices[u];
    let (early, o) = d.svc.invite(&d.keys, &mine, &w.pk(v)).await.unwrap();
    w.run(u, o).await;
    assert_eq!(w.devices[v].svc.invites("in").await.unwrap().len(), 1);
    w.say(u, &g, "x").await;
    w.tick();
    w.say(owner, &g, "o").await;
    assert_eq!(w.devices[v].unread(&g).await, 2);

    // Banned on v's device: all u wrote is hidden there, from the first
    // message, without a note of anybody's; for the others nothing changed.
    ban_on(&w, v, &[u]).await;
    assert_eq!(w.devices[v].texts(&g).await, vec!["o"]);
    assert_eq!(w.devices[owner].texts(&g).await, vec!["x", "o"]);
    assert_eq!((w.devices[v].unread(&g).await, w.devices[v].preview(&g).await.as_deref()), (1, Some("o")));
    w.say(u, &g, "y").await;
    assert_eq!(w.devices[v].texts(&g).await, vec!["o"], "a new one neither");
    assert_eq!(w.devices[v].unread(&g).await, 1, "and it does not count");

    // The invitation that waited is not offered nor taken; a new one does
    // not come; v invites u to nothing.
    assert!(w.devices[v].svc.invites("in").await.unwrap().is_empty());
    let d = &w.devices[v];
    assert_eq!(code(d.svc.answer_invite(&d.keys, &early.invite_id, true).await), "peer_banned");
    let d = &w.devices[u];
    let (_, o) = d.svc.invite(&d.keys, &other, &w.pk(v)).await.unwrap();
    w.run(u, o).await;
    assert!(w.devices[v].svc.invites("in").await.unwrap().is_empty(), "the second one was not taken");
    let ours = w.create(v, GroupKind::Private, "Ours", true).await;
    let d = &w.devices[v];
    assert_eq!(code(d.svc.invite(&d.keys, &ours, &w.pk(u)).await), "peer_banned");

    // The ban lifts: what was kept is shown again; what was refused while
    // it stood was never kept. The invitation that waited is offered again.
    ban_on(&w, v, &[]).await;
    assert_eq!(w.devices[v].texts(&g).await, vec!["x", "o", "y"]);
    assert_eq!(w.devices[v].svc.invites("in").await.unwrap().len(), 1);
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

/// What the runtime does when the manifest closes `groups` (and no other)
/// on a device: the table, as `banned_groups_follow` writes it.
async fn close_on(w: &World, device: usize, groups: &[&str]) {
    let mut ids: Vec<String> = groups.iter().map(|g| g.to_string()).collect();
    ids.sort();
    messenger_store::banned::group_replace(w.devices[device].dm.store(), &ids).await.unwrap();
}

#[tokio::test]
async fn a_group_the_network_closes_takes_nothing_in_sends_nothing_and_opens_again() {
    let mut w = World::new();
    let owner = w.person().await;
    let u = w.person().await;
    let v = w.person().await;
    let g = w.create(owner, GroupKind::Private, "Den", true).await;
    w.bring(owner, &g, u).await;
    let before = w.say(owner, &g, "before").await;
    // An invitation to v that came before the group was closed there.
    let d = &w.devices[owner];
    let (early, o) = d.svc.invite(&d.keys, &g, &w.pk(v)).await.unwrap();
    w.run(owner, o).await;
    let link = w.devices[owner].group(&g).await.unwrap().link.expect("a manager shares the link");

    // Closed on u's and v's devices: what the group says is not taken in,
    // and the chat stays as it was.
    close_on(&w, u, &[&g]).await;
    close_on(&w, v, &[&g]).await;
    w.say(owner, &g, "during").await;
    assert_eq!(w.devices[u].texts(&g).await, vec!["before"]);
    assert!(!w.devices[u].svc.watched().await.unwrap().contains(&g), "not listened to");
    let view = w.devices[u].group(&g).await.unwrap();
    assert_eq!(view.membership, MEMBERSHIP_JOINED, "still a member, by the journal here");
    assert!(!view.can_post);

    // Nothing goes to it, whatever is done.
    assert_eq!(code(w.try_say(u, &g, "hello?").await), "group_banned");
    let d = &w.devices[u];
    assert_eq!(code(d.svc.prepare_read_receipt(&d.keys, &g, 5_000).await), "group_banned");
    assert_eq!(code(d.svc.prepare_reaction(&d.keys, &before, "👍").await), "group_banned");
    assert_eq!(code(d.svc.prepare_call_note(&d.keys, &g, &Envelope::call_join(&"ab".repeat(16), 3)).await), "group_banned");
    assert_eq!(code(w.act(u, &g, OpBody::Leave).await), "group_banned");

    // The invitation that waited is not offered nor taken, a new one is not
    // kept, and the link leads nowhere.
    assert!(w.devices[v].svc.invites("in").await.unwrap().is_empty());
    let d = &w.devices[v];
    assert_eq!(code(d.svc.answer_invite(&d.keys, &early.invite_id, true).await), "group_banned");
    let d = &w.devices[owner];
    let (_, o) = d.svc.invite(&d.keys, &g, &w.pk(v)).await.unwrap();
    w.run(owner, o).await;
    let d = &w.devices[v];
    assert_eq!(code(d.svc.open_link(&d.keys, &link, "").await), "group_banned");
    assert!(d.svc.get(&g, &d.pk()).await.unwrap().is_none(), "no row for a closed group");

    // A manager's device closed too: no invitation, no key care.
    close_on(&w, owner, &[&g]).await;
    let d = &w.devices[owner];
    assert_eq!(code(d.svc.invite(&d.keys, &g, &w.pk(v)).await), "group_banned");
    let o = d.svc.maintain(&d.keys, &g).await.unwrap();
    assert!(o.publish.is_empty());
    close_on(&w, owner, &[]).await;

    // The ban lifts: the history brings what was refused, and u writes again.
    close_on(&w, u, &[]).await;
    assert!(w.devices[u].svc.watched().await.unwrap().contains(&g));
    w.catch_up(u).await;
    assert_eq!(w.devices[u].texts(&g).await, vec!["before", "during"]);
    w.say(u, &g, "back").await;
    assert_eq!(w.devices[owner].texts(&g).await, vec!["before", "during", "back"]);
    close_on(&w, v, &[]).await;
    let invites = w.devices[v].svc.invites("in").await.unwrap();
    assert_eq!(invites.len(), 1, "the one that waited, the one of the ban was never kept");
    assert_eq!(invites[0].invite_id, early.invite_id);
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

#[tokio::test]
async fn a_full_page_of_the_queue_keeps_the_lines_of_one_second_together() {
    let mut w = World::new();
    let (g, owner, moderator, u, v) = moderated_group(&mut w).await;
    w.act(owner, &g, Settings { premod: Some(Premod::All), ..Settings::default() }.body()).await.unwrap();
    // Two members speak in the same second, a third a little later.
    w.tick();
    let at = w.clock.now().secs();
    for (who, text) in [(u, "u1"), (v, "v1")] {
        let out = w.forged_envelope(who, &g, &Envelope::text(text), at).await;
        w.run(who, Outcome { publish: vec![out], ..Default::default() }).await;
    }
    let n = w.person().await;
    w.bring(owner, &g, n).await;
    w.say(n, &g, "n1").await;
    let d = &w.devices[moderator];
    let me = d.pk();
    let all = msgs::QueueFilter::default();
    let (page, total) = d.svc.queue(&g, &me, &all, None, 2).await.unwrap();
    assert_eq!(total, 3);
    assert_eq!(page.len(), 3, "the page is full at the second line, and the third shares its second: it comes along");
    assert_eq!(page[0].row.text.as_deref(), Some("n1"));
    assert_eq!((page[1].row.created_at, page[2].row.created_at), (at, at));
    let (next, _) = d.svc.queue(&g, &me, &all, Some(page[2].row.created_at), 2).await.unwrap();
    assert!(next.is_empty(), "nothing was passed by");
    let (one, _) = d.svc.queue(&g, &me, &all, Some(at + 1), 1).await.unwrap();
    assert_eq!(one.len(), 2, "a page of one takes both lines of the second");
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}

/// An operation kept for an update (`keep_op_ahead`) stands in the
/// store as a signed event of a version this build does not read; its
/// shape may be one the build happens to parse. The log passes it by
/// before parsing, so what names it as a parent waits, as on the path of
/// the bucket; the count of what waits for an update still has it.
#[tokio::test]
async fn an_operation_kept_for_an_update_holds_its_children_back_in_the_log() {
    let mut w = World::new();
    let alice = w.person().await;
    let g = w.create(alice, GroupKind::Private, "Team", true).await;
    let d = &w.devices[alice];
    let log = d.svc.need_log(&g).await.unwrap();
    let mut ahead = log.next(&w.pk(alice), w.clock.now().secs(), Settings { name: Some("Future".into()), ..Settings::default() }.body());
    ahead.v = crate::op::OP_VERSION + 1;
    let mut child = log.next(&w.pk(alice), w.clock.now().secs() + 1, Settings { name: Some("Child".into()), ..Settings::default() }.body());
    child.parents = vec![ahead.id()];
    for op in [&ahead, &child] {
        use nostr::prelude::*;
        let signed = EventBuilder::new(Kind::from(crate::wire::KIND_INNER_OP), op.canonical())
            .tag(Tag::parse(["h", g.as_str()]).unwrap())
            .custom_created_at(Timestamp::from_secs(op.created_at as u64))
            .finalize(&d.keys)
            .unwrap();
        assert!(messenger_store::groups::insert_op(&d.svc.store, &g, &op.id().0, &serde_json::to_value(&signed).unwrap().to_string()).await.unwrap());
    }
    d.svc.forget_log(&g).await;
    let log = d.svc.need_log(&g).await.unwrap();
    assert_eq!(log.state().name, "Team", "the child waits for what the build cannot read");
    assert!(!log.contains(&ahead.id()) && !log.contains(&child.id()));
    assert_eq!(log.missing(), vec![ahead.id()]);
    assert_eq!(crate::service::ops_ahead(&d.svc.store, &g).await.unwrap(), 1, "and it counts as waiting for an update");
}

/// A group of a private server lives there, and only there
/// (internal/messenger-wire.md, «Группы на сервере»). Alice has the home
/// set and the server S1 as a private one, on two devices; Bob's home set
/// is S1 itself; Carol has the home set alone. Alice makes the group on
/// S1: her other device gets it by a welcome to herself on S1, Bob asks
/// in by the link and is let in, Carol is told the server is unknown and
/// hears nothing of an invitation. Every word Alice's devices send of the
/// group (operations, messages, a receipt, a reaction, an edit, a note of
/// a call, the invitation, the welcomes) is scoped to the group or to S1,
/// never to an inbox or a relay of the home set; an event of the group
/// that comes by a relay of the home set is dropped, and taken when it
/// comes by S1.
#[tokio::test]
async fn a_group_of_a_private_server_lives_there_alone() {
    const S1: &str = "wss://s1.example";
    let mut w = World::new();
    let alice = w.person().await;
    let alice2 = w.device(w.devices[alice].keys.clone()).await;
    let bob = w.person().await;
    let carol = w.person().await;
    for (d, kind) in [(alice, servers::KIND_PRIVATE), (alice2, servers::KIND_PRIVATE), (bob, servers::KIND_HOME)] {
        servers::put(&w.devices[d].svc.store, &server_row(S1, kind)).await.unwrap();
    }
    let s1 = servers::id_of(S1);
    w.by = S1.into();

    w.tick();
    let (view, o) = {
        let d = &w.devices[alice];
        d.svc.create(&d.keys, GroupKind::Private, "On S1", "", true, &RelayUrl::parse(S1).unwrap()).await.unwrap()
    };
    w.run(alice, o).await;
    let g = view.id.clone();
    assert_eq!((view.relay.as_str(), view.server_id.as_deref()), (S1, Some(s1.as_str())));
    assert_eq!(w.devices[alice].svc.server_of(&g).await.unwrap().as_deref(), Some(s1.as_str()));
    let other = w.devices[alice2].group(&g).await.expect("my other device got the group by the welcome on S1");
    assert_eq!((other.membership.as_str(), other.server_id.as_deref()), ("joined", Some(s1.as_str())));

    // Bob, at home on S1, asks in by the link and is let in; for him the
    // group is of his home set.
    let link = view.link.clone().expect("the owner may share the link");
    w.open_link(bob, &link).await;
    let a = &w.devices[alice];
    let o = a.svc.approve_request(&a.keys, &g, &w.pk(bob)).await.unwrap();
    w.run(alice, o).await;
    let seen = w.devices[bob].group(&g).await.unwrap();
    assert_eq!((seen.membership.as_str(), seen.server_id), ("joined", None), "S1 is Bob's home");
    assert_eq!(w.devices[bob].svc.server_of(&g).await.unwrap(), None);

    // Carol has no S1: the link leads nowhere, an invitation never comes.
    let c = &w.devices[carol];
    assert_eq!(code(c.svc.open_link(&c.keys, &link, "").await), ERR_SERVER_UNKNOWN);
    assert!(c.svc.get(&g, &w.pk(carol)).await.unwrap().is_none(), "nothing written for it");
    w.tick();
    let (_, o) = {
        let a = &w.devices[alice];
        a.svc.invite(&a.keys, &g, &w.pk(carol)).await.unwrap()
    };
    w.run(alice, o).await;
    assert!(w.devices[carol].svc.invites("in").await.unwrap().is_empty(), "a word of a server Carol does not have");

    // Everything of the group, both ways.
    let first = w.say(alice, &g, "hello on s1").await;
    w.say(bob, &g, "back").await;
    assert_eq!(w.devices[alice2].texts(&g).await, vec!["hello on s1", "back"]);
    w.tick();
    let (a, b) = (&w.devices[alice], &w.devices[bob]);
    let mut outs = vec![a.svc.prepare_read_receipt(&a.keys, &g, w.clock.now().secs()).await.unwrap()];
    outs.push(a.svc.prepare_reaction(&a.keys, &first, "👍").await.unwrap().1);
    outs.push(a.svc.prepare_edit(&a.keys, &first, "hello on s1, edited").await.unwrap().2);
    outs.push(a.svc.prepare_call_note(&a.keys, &g, &messenger_core::Envelope::call_join(&"ab".repeat(16), 3)).await.unwrap());
    let bobs = b.svc.prepare_text(&b.keys, &g, "late", None).await.unwrap().1;
    w.run(alice, Outcome { publish: outs, ..Default::default() }).await;
    w.act(alice, &g, OpBody::RotateKey).await.unwrap();
    assert_eq!(w.devices[alice2].texts(&g).await, vec!["hello on s1, edited", "back"]);

    // Bob's message by a relay of the home set: dropped, not the group's
    // word; the same by S1: taken.
    let Outbound::PublishScoped { event, .. } = &bobs else { panic!("{bobs:?}") };
    w.by = RELAY.into();
    let o = w.deliver(alice, &Wire::Group(event.clone()), false).await;
    assert!(o.notes.iter().any(|n| n.contains("not of the group's server")), "{:?}", o.notes);
    assert!(!w.devices[alice].texts(&g).await.contains(&"late".to_string()));
    w.by = S1.into();
    let o = w.deliver(alice, &Wire::Group(event.clone()), false).await;
    w.run(alice, o).await;
    assert!(w.devices[alice].texts(&g).await.contains(&"late".to_string()));

    // Not one word of Alice's devices to an inbox or a relay of the home set.
    let mine: Vec<&Outbound> = w.sent.iter().filter(|(d, _)| *d == alice || *d == alice2).map(|(_, o)| o).collect();
    assert!(mine.len() > 10, "{}", mine.len());
    let mut to_server = 0;
    for out in mine {
        match out {
            Outbound::PublishScoped { scope: Scope::Group { id }, .. } => assert_eq!(id, &g),
            Outbound::PublishScoped { scope: Scope::Server { id }, event } => {
                assert_eq!(id, &s1);
                assert_eq!(event.json["kind"], 1059, "a word of the protocol, as in a private chat");
                to_server += 1;
            }
            other => panic!("left for the home set: {other:?}"),
        }
    }
    assert!(to_server >= 3, "the welcomes and the invitation: {to_server}");
}

/// A group whose relay is no server of mine never becomes a group of the
/// home set by an invitation or a welcome (internal/messenger-wire.md,
/// «Группы на сервере»). R2 is of the home set of Alice's first device;
/// her second device and Carol have it not. Carol's invitation waits,
/// listed with its relay, and an accept is `server_unknown` until she adds
/// R2; then it is accepted, and the group is of her private server R2.
/// The welcome of Alice's own to her second device is held, no group
/// made of it, and read when R2 comes there.
#[tokio::test]
async fn a_group_on_a_server_i_have_not_waits_for_it() {
    const R2: &str = "wss://r2.example";
    let mut w = World::new();
    let alice = w.person().await;
    let alice2 = w.device(w.devices[alice].keys.clone()).await;
    let carol = w.person().await;
    servers::put(&w.devices[alice].svc.store, &server_row(R2, servers::KIND_HOME)).await.unwrap();
    let r2 = servers::id_of(R2);
    w.by = R2.into();

    w.tick();
    let (view, o) = {
        let d = &w.devices[alice];
        d.svc.create(&d.keys, GroupKind::Private, "On R2", "", true, &RelayUrl::parse(R2).unwrap()).await.unwrap()
    };
    w.run(alice, o).await;
    let g = view.id.clone();
    assert_eq!(view.server_id, None, "R2 is Alice's home");
    assert!(w.devices[alice2].group(&g).await.is_none(), "no group of the home set made of my own welcome");
    assert!(w.notes.iter().any(|n| n.contains("held until it is")), "{:?}", w.notes);

    w.tick();
    let (_, o) = {
        let a = &w.devices[alice];
        a.svc.invite(&a.keys, &g, &w.pk(carol)).await.unwrap()
    };
    w.run(alice, o).await;
    let c = &w.devices[carol];
    let invites = c.svc.invites("in").await.unwrap();
    assert_eq!(invites.len(), 1);
    let inv = invites[0].clone();
    assert_eq!((inv.relay.as_str(), inv.server_known), (R2, false));
    assert_eq!(code(c.svc.answer_invite(&c.keys, &inv.invite_id, true).await), ERR_SERVER_UNKNOWN);
    assert!(c.svc.get(&g, &w.pk(carol)).await.unwrap().is_none(), "nothing written for it");
    let waiting = c.svc.invites("in").await.unwrap();
    assert_eq!((waiting.len(), waiting[0].status.as_str()), (1, "received"), "the invitation waits");

    // Carol adds R2: the same invitation is accepted, the group is of R2.
    servers::put(&c.svc.store, &server_row(R2, servers::KIND_PRIVATE)).await.unwrap();
    assert!(c.svc.invites("in").await.unwrap()[0].server_known);
    w.tick();
    let o = {
        let c = &w.devices[carol];
        c.svc.answer_invite(&c.keys, &inv.invite_id, true).await.unwrap()
    };
    w.run(carol, o).await;
    let joined = w.devices[carol].group(&g).await.expect("let in");
    assert_eq!((joined.membership.as_str(), joined.server_id.as_deref()), ("joined", Some(r2.as_str())));

    // R2 comes to Alice's second device: the welcome held is read.
    servers::put(&w.devices[alice2].svc.store, &server_row(R2, servers::KIND_PRIVATE)).await.unwrap();
    let o = {
        let d = &w.devices[alice2];
        d.svc.welcomes_of_servers_that_came(&d.keys).await.unwrap()
    };
    w.run(alice2, o).await;
    let mine = w.devices[alice2].group(&g).await.expect("the held welcome made the group");
    assert_eq!((mine.membership.as_str(), mine.server_id.as_deref()), ("joined", Some(r2.as_str())));
}

/// A public group joined by its link on one device comes to my other
/// devices by the welcome to myself, as one I make does: the link was
/// opened on one device only. On a private server the welcome goes there
/// (the owner's report of 2026-10-19: the group of S1 joined on the phone
/// never came to the desktop, a message in it neither).
#[tokio::test]
async fn a_group_joined_by_link_comes_to_my_other_device() {
    const S1: &str = "wss://s1.example";
    let s1 = servers::id_of(S1);
    for (relay, server) in [(None, None), (Some(S1), Some(s1.as_str()))] {
        let mut w = World::new();
        let alice = w.person().await;
        let bob = w.person().await;
        let bob2 = w.device(w.devices[bob].keys.clone()).await;
        for d in [alice, bob, bob2] {
            servers::put(&w.devices[d].svc.store, &server_row(S1, servers::KIND_PRIVATE)).await.unwrap();
        }
        w.by = relay.unwrap_or(RELAY).into();
        w.tick();
        let g = match relay {
            None => w.create(alice, GroupKind::Public, "Town square", true).await,
            Some(r) => {
                let (view, o) = {
                    let d = &w.devices[alice];
                    d.svc.create(&d.keys, GroupKind::Public, "On S1", "", true, &RelayUrl::parse(r).unwrap()).await.unwrap()
                };
                w.run(alice, o).await;
                view.id
            }
        };
        w.say(alice, &g, "one").await;
        let link = w.devices[alice].group(&g).await.unwrap().link.unwrap();
        w.open_link(bob, &link).await;
        w.catch_up(bob).await;
        assert_eq!(w.devices[bob].group(&g).await.unwrap().membership, "joined");

        let v = w.devices[bob2].group(&g).await.expect("my other device has the group I joined by link");
        assert_eq!((v.membership.as_str(), v.server_id.as_deref()), ("joined", server), "{relay:?}");
        assert_eq!(v.link.as_deref(), Some(link.as_str()), "the link comes with it");
        w.catch_up(bob2).await;
        w.say(bob, &g, "two").await;
        w.say(bob2, &g, "three").await;
        for d in [alice, bob, bob2] {
            assert_eq!(w.devices[d].texts(&g).await, vec!["one", "two", "three"], "device {d}, {relay:?}");
        }
        assert!(w.notes.is_empty(), "{relay:?}: {:?}", w.notes);
    }
}

/// The owner signed in on a new phone: its history reached back only so
/// far, and the groups he was in did not come. Every group I am in names
/// a welcome to myself now — made at its creation, or sent once for one
/// I was brought into — which my list carries by its id: a new device that
/// fetches those wraps by id, and nothing else of the past, is in every
/// group, and sends nothing anew for them.
#[tokio::test]
async fn a_new_device_gets_my_groups_by_the_welcomes_to_myself() {
    use messenger_store::groups::self_welcome;
    let mut w = World::new();
    let alice = w.person().await;
    let bob = w.person().await;
    let mine = w.create(alice, GroupKind::Private, "Mine", true).await;
    let theirs = w.create(bob, GroupKind::Private, "Theirs", true).await;
    w.say(bob, &theirs, "before alice").await;
    w.bring(bob, &theirs, alice).await;
    let store = w.devices[alice].dm.store().clone();
    let first = self_welcome(&store, &mine).await.unwrap().expect("made at the creation");
    assert!(self_welcome(&store, &theirs).await.unwrap().is_none(), "brought in by bob: none of mine yet");
    let d = &w.devices[alice];
    let owed = d.svc.welcomes_owed(&d.keys).await.unwrap();
    assert_eq!(owed.len(), 1, "one for the group bob brought me into");
    w.run(alice, Outcome { publish: owed, ..Default::default() }).await;
    let second = self_welcome(&store, &theirs).await.unwrap().expect("sent");
    let d = &w.devices[alice];
    assert!(d.svc.welcomes_owed(&d.keys).await.unwrap().is_empty(), "once");

    // A new device of alice's: of the past, only the two wraps by their ids.
    let keys = w.devices[alice].keys.clone();
    let phone = w.device(keys).await;
    for id in [&first, &second] {
        let wire = w
            .sent
            .iter()
            .find_map(|(_, o)| match o {
                Outbound::PublishToInbox { event, .. } | Outbound::PublishScoped { event, .. } | Outbound::PublishOwn { event }
                    if event.id.as_hex() == id.as_str() =>
                {
                    Some(Wire::Dm(event.clone()))
                }
                _ => None,
            })
            .expect("the wrap was sent");
        let o = w.deliver(phone, &wire, true).await;
        w.run(phone, o).await;
    }
    w.catch_up(phone).await;
    for g in [&mine, &theirs] {
        let v = w.devices[phone].group(g).await.expect("the new device has the group");
        assert_eq!(v.membership, "joined", "{g}");
    }
    assert_eq!(w.devices[phone].texts(&theirs).await, vec!["before alice"]);
    let phone_store = w.devices[phone].dm.store().clone();
    assert_eq!(self_welcome(&phone_store, &mine).await.unwrap(), Some(first), "its own welcome names it");
    assert_eq!(self_welcome(&phone_store, &theirs).await.unwrap(), Some(second));
    let d = &w.devices[phone];
    assert!(d.svc.welcomes_owed(&d.keys).await.unwrap().is_empty(), "nothing anew");
    assert!(w.notes.is_empty(), "{:?}", w.notes);
}
