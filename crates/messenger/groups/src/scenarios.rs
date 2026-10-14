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
        assert_eq!(w.devices[d].group(&g).await.unwrap().requests, vec![bob_hex.clone()], "device {d}");
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
    assert_eq!(w.devices[alice].group(&g).await.unwrap().requests, vec![w.pk(carol).as_hex().to_string()]);
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
    let ph = d.svc.media_placeholder(&d.keys, &g, fields.clone(), None).await.unwrap();
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
        let ph = d.svc.media_placeholder(&d.keys, group, fields.clone(), Some(caption)).await.unwrap();
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
    w.act(alice, &g, OpBody::EditSettings { name: Some("Team 2".into()), about: None, picture: None, history_for_new: None, call_node: None, call_node_key: None }).await.unwrap();
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
    w.act(alice, &g, OpBody::SetMuted { who: w.pk(user), muted: true }).await.unwrap();
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
    let edit = OpBody::EditSettings { name: None, about: Some(about.into()), picture: None, history_for_new: None, call_node: None, call_node_key: None };
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
