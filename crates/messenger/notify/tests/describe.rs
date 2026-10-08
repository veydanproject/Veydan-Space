// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The phone of `me`, with its database on disk, gets pushes about events
//! that others wrote. What it may show is what the app would have shown.

use messenger_contacts::{ContactService, ProfileService};
use messenger_core::traits::{Clock, SystemClock};
use messenger_core::{Context, Envelope, MessengerConfig, PubKey, RelayUrl, Timestamp};
use messenger_dm::relationship::Action;
use messenger_dm::wrap::{wrap, wrap_as, wrap_note, wrap_note_as, wrap_own, Wake};
use messenger_dm::DmService;
use messenger_groups::wire::{seal_message, sign_message};
use messenger_groups::{GroupKind, GroupService};
use messenger_media::{ChunkRef, MediaDescriptor, MediaKind};
use messenger_notify::{describe, Body, ChatKind, Content, GroupKeyEntry, KeyBundle, Outcome, PushData, Reason, Settings};
use messenger_store::{calls, Store};
use messenger_testkit::MemorySecretStore;
use nostr::key::Keys;
use std::collections::BTreeMap;
use std::sync::Arc;

struct Phone {
    dir: tempfile::TempDir,
    keys: Keys,
    store: Store,
    contacts: ContactService,
    dm: DmService,
    groups: GroupService,
}

impl Phone {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&MessengerConfig::new(dir.path().join("messenger"))).await.unwrap();
        let profiles = ProfileService::new(store.clone());
        let contacts = ContactService::new(store.clone(), profiles.clone());
        let dm = DmService::new(store.clone(), contacts.clone(), profiles, Arc::new(SystemClock));
        let groups = GroupService::new(store.clone(), Arc::new(MemorySecretStore::unlocked()), Arc::new(SystemClock), dm.clone());
        Self { dir, keys: Keys::generate(), store, contacts, dm, groups }
    }

    fn me(&self) -> PubKey {
        pk(&self.keys)
    }

    fn ctx(&self) -> Context {
        Context { my_pubkey: self.me(), session_started_at: Timestamp(0), clock: Arc::new(SystemClock) }
    }

    async fn bundle(&self) -> KeyBundle {
        let groups = self.groups.export_keys().await.unwrap().iter().map(|(g, k)| GroupKeyEntry::of(g, k)).collect();
        KeyBundle::new(&self.keys, groups)
    }

    /// The app stored this message itself (it was running when it came).
    async fn app_received(&self, event: &serde_json::Value) {
        let raw = raw(event);
        let messenger_core::Inbound::Dm(dm) = messenger_ingress::classify(&raw, Some(&self.keys)) else { panic!("a dm") };
        self.dm.apply_inbound(dm, &self.ctx()).await.unwrap();
    }

    async fn describe(&self, push: PushData) -> Outcome {
        let bundle = self.bundle().await;
        self.describe_with(&bundle, push).await
    }

    async fn describe_with(&self, bundle: &KeyBundle, push: PushData) -> Outcome {
        describe(&self.dir.path().join("messenger"), Some(bundle), &push).await.unwrap()
    }
}

fn now() -> i64 {
    SystemClock.now().secs()
}

fn pk(keys: &Keys) -> PubKey {
    PubKey::parse(&keys.public_key().to_hex()).unwrap()
}

fn raw(event: &serde_json::Value) -> messenger_core::RawEvent {
    messenger_core::RawEvent {
        id: messenger_core::EventId::parse(event["id"].as_str().unwrap()).unwrap(),
        kind: event["kind"].as_u64().unwrap() as u16,
        pubkey: PubKey::parse(event["pubkey"].as_str().unwrap()).unwrap(),
        created_at: Timestamp(event["created_at"].as_i64().unwrap()),
        json: event.clone(),
        source: messenger_core::EventSource::Relay { url: RelayUrl::parse("wss://r.example").unwrap() },
    }
}

fn push_with(pairs: &[(&str, &str)]) -> PushData {
    let mut map: BTreeMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    map.entry("v".into()).or_insert("2".into());
    PushData::parse(&map).unwrap()
}

fn dm_push(event: &serde_json::Value) -> PushData {
    push_with(&[("type", "dm"), ("event", &event.to_string())])
}

fn group_push(group_id: &str, event: &serde_json::Value) -> PushData {
    push_with(&[("type", "group"), ("group_id", group_id), ("event", &event.to_string())])
}

fn dm_from(sender: &Keys, to: &Phone, envelope: &Envelope, at: i64) -> serde_json::Value {
    wrap(sender, &to.me(), &envelope.encode(), at, None).unwrap().to_peer.json
}

fn shown(outcome: Outcome) -> messenger_notify::Notice {
    match outcome {
        Outcome::Show(n) => n,
        other => panic!("expected a notice, got {other:?}"),
    }
}

fn quiet(outcome: Outcome) -> Reason {
    match outcome {
        Outcome::Quiet { reason } => reason,
        other => panic!("expected quiet, got {other:?}"),
    }
}

fn plain(outcome: Outcome) -> messenger_notify::Plain {
    match outcome {
        Outcome::Plain(p) => p,
        other => panic!("expected plain, got {other:?}"),
    }
}

async fn befriend(phone: &Phone, peer: &Keys, nickname: &str) {
    phone.contacts.add(&phone.me(), &peer.public_key().to_hex(), Some(nickname)).await.unwrap();
    phone.dm.act(&phone.keys, &pk(peer), Action::Request).await.unwrap();
}

#[tokio::test]
async fn a_text_from_a_contact_names_them_and_says_what() {
    let phone = Phone::new().await;
    let alice = Keys::generate();
    befriend(&phone, &alice, "Al").await;

    let event = dm_from(&alice, &phone, &Envelope::text("  hello   there "), 1_000_000);
    let n = shown(phone.describe(dm_push(&event)).await);
    assert_eq!(n.kind, ChatKind::Dm);
    assert_eq!(n.chat.as_deref(), Some(format!("dm:{}", alice.public_key().to_hex()).as_str()));
    assert_eq!(n.title, "Al");
    assert_eq!(n.sender, "Al");
    assert_eq!(n.sender_key, alice.public_key().to_hex());
    assert_eq!(n.body, Some(Body::Text { text: "hello there".into() }));
    assert!(!n.muted);
    assert!(!n.hide_on_lockscreen);
    assert_eq!(n.count, 1);
}

#[tokio::test]
async fn a_stranger_is_a_request_once_and_nothing_the_second_time() {
    let phone = Phone::new().await;
    let stranger = Keys::generate();
    let first = dm_from(&stranger, &phone, &Envelope::text("hi"), 1_000_000);
    let n = shown(phone.describe(dm_push(&first)).await);
    assert_eq!(n.kind, ChatKind::Request);
    assert!(n.title.starts_with("npub1"), "{}", n.title);

    // The app took the first message in; a second before an answer is dropped.
    phone.app_received(&first).await;
    assert_eq!(shown(phone.describe(dm_push(&first)).await).kind, ChatKind::Dm, "a copy of what is stored is shown as stored");
    let second = dm_from(&stranger, &phone, &Envelope::text("hi again"), 1_000_001);
    assert_eq!(quiet(phone.describe(dm_push(&second)).await), Reason::NotForMe);
}

#[tokio::test]
async fn blocked_muted_and_my_own_messages() {
    let phone = Phone::new().await;
    let alice = Keys::generate();
    befriend(&phone, &alice, "Al").await;
    let chat = format!("dm:{}", alice.public_key().to_hex());

    phone.dm.set_muted(&chat, true).await.unwrap();
    let n = shown(phone.describe(dm_push(&dm_from(&alice, &phone, &Envelope::text("psst"), 1_000_000))).await);
    assert!(n.muted);

    phone.dm.act(&phone.keys, &pk(&alice), Action::Block).await.unwrap();
    assert_eq!(quiet(phone.describe(dm_push(&dm_from(&alice, &phone, &Envelope::text("?"), 1_000_001))).await), Reason::Blocked);

    // My copy of a message of mine, from another device.
    let bob = Keys::generate();
    let mine = wrap_as(&phone.keys, &pk(&bob), &Envelope::text("from my other phone").encode(), 1_000_002, None, Wake::Peer)
        .unwrap()
        .to_self
        .unwrap()
        .json;
    assert_eq!(quiet(phone.describe(dm_push(&mine)).await), Reason::Own);
}

#[tokio::test]
async fn edits_deletes_and_signals_are_not_messages() {
    let phone = Phone::new().await;
    let alice = Keys::generate();
    befriend(&phone, &alice, "Al").await;
    for envelope in [Envelope::edit("abc", "new"), Envelope::delete("abc"), Envelope::control("dm_accept")] {
        let event = dm_from(&alice, &phone, &envelope, 1_000_000);
        assert_eq!(quiet(phone.describe(dm_push(&event)).await), Reason::NotAMessage, "{}", envelope.t);
    }
}

#[tokio::test]
async fn notes_from_a_peer_and_between_my_devices_are_not_messages() {
    let phone = Phone::new().await;
    let alice = Keys::generate();
    befriend(&phone, &alice, "Al").await;

    // A receipt, and a note whose type nobody knows yet: neither is named.
    for envelope in [Envelope::receipt_read(1_000_000), Envelope::new("reaction.future")] {
        let note = wrap_note(&alice, &phone.me(), &envelope.encode(), 1_000_000, false, None).unwrap().to_peer.json;
        assert_eq!(quiet(phone.describe(dm_push(&note)).await), Reason::NotAMessage, "{}", envelope.t);
    }
    // A note from a stranger is not a request either.
    let stranger = Keys::generate();
    let note = wrap_note(&stranger, &phone.me(), &Envelope::text("hi").encode(), 1_000_000, false, None).unwrap().to_peer.json;
    assert_eq!(quiet(phone.describe(dm_push(&note)).await), Reason::NotAMessage);

    let own = wrap_own(&phone.keys, &Envelope::own_read("dm:x", 1).encode(), 1_000_000).unwrap().json;
    assert_eq!(quiet(phone.describe(dm_push(&own)).await), Reason::NotAMessage);
}

fn descriptor(kind: MediaKind, name: &str, caption: Option<&str>) -> MediaDescriptor {
    MediaDescriptor {
        kind,
        name: name.into(),
        mime: "application/octet-stream".into(),
        size: 100,
        sha256: "ab".repeat(32),
        chunk_size: 64 * 1024,
        chunks: vec![ChunkRef { sha256: "cd".repeat(32), size: 116 }],
        algo: "aes-256-gcm".into(),
        key: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into(),
        iv: "AAAAAAAAAAAAAAAA".into(),
        servers: vec!["https://blob.example".into()],
        caption: caption.map(String::from),
        batch: None,
        dim: None,
        duration_ms: None,
        waveform: None,
        thumb: None,
    }
}

#[tokio::test]
async fn media_says_its_kind_and_the_caption() {
    let phone = Phone::new().await;
    let alice = Keys::generate();
    befriend(&phone, &alice, "Al").await;

    let mut voice = descriptor(MediaKind::Voice, "voice.weba", None);
    voice.duration_ms = Some(12_400);
    let voice = voice.to_envelope();
    let n = shown(phone.describe(dm_push(&dm_from(&alice, &phone, &voice, 1_000_000))).await);
    assert_eq!(
        n.body,
        Some(Body::Media { kind: "voice".into(), name: "voice.weba".into(), caption: None, duration_ms: Some(12_400), batch: None })
    );

    let mut photo = descriptor(MediaKind::Image, "cat.jpg", Some("look "));
    photo.batch = Some("album-0001".into());
    let photo = photo.to_envelope();
    let n = shown(phone.describe(dm_push(&dm_from(&alice, &phone, &photo, 1_000_001))).await);
    assert_eq!(
        n.body,
        Some(Body::Media {
            kind: "image".into(),
            name: "cat.jpg".into(),
            caption: Some("look".into()),
            duration_ms: None,
            batch: Some("album-0001".into()),
        })
    );

    // What the phone reads has no `null` in it: its JSON reader would
    // show a missing caption as the word "null".
    let file = descriptor(MediaKind::File, "report.pdf", None).to_envelope();
    let outcome = phone.describe(dm_push(&dm_from(&alice, &phone, &file, 1_000_002))).await;
    let json = serde_json::to_value(&outcome).unwrap();
    assert_eq!(json["body"], serde_json::json!({ "t": "media", "kind": "file", "name": "report.pdf" }));
}

#[tokio::test]
async fn the_settings_decide_how_much_is_said() {
    let phone = Phone::new().await;
    let alice = Keys::generate();
    befriend(&phone, &alice, "Al").await;
    let event = dm_from(&alice, &phone, &Envelope::text("secret"), 1_000_000);

    Settings { content: Content::Sender, lockscreen_hidden: true }.save(&phone.store).await.unwrap();
    let n = shown(phone.describe(dm_push(&event)).await);
    assert_eq!(n.sender, "Al");
    assert!(n.body.is_none());
    assert!(n.hide_on_lockscreen);

    Settings { content: Content::None, lockscreen_hidden: false }.save(&phone.store).await.unwrap();
    let p = plain(phone.describe(dm_push(&event)).await);
    assert_eq!(p.kind, ChatKind::Dm);
    assert!(p.chat.is_none());
    assert!(p.title.is_none());
}

#[tokio::test]
async fn a_push_without_the_event_and_without_a_relay_is_plain() {
    let phone = Phone::new().await;
    let (gview, _) = phone
        .groups
        .create(&phone.keys, GroupKind::Private, "Тихая", "", true, &RelayUrl::parse("wss://r.example").unwrap())
        .await
        .unwrap();
    let p = plain(phone.describe(push_with(&[("type", "group"), ("group_id", &gview.id), ("event_id", &"ef".repeat(32)), ("count", "4")])).await);
    assert_eq!(p.kind, ChatKind::Group);
    assert_eq!(p.chat.as_deref(), Some(gview.chat_id.as_str()));
    assert_eq!(p.title.as_deref(), Some("Тихая"));
    assert_eq!(p.count, 4);
}

async fn group_with_bob(phone: &Phone) -> (String, Keys, messenger_groups::GroupKey) {
    let (view, _) = phone
        .groups
        .create(&phone.keys, GroupKind::Private, "Пуш-тест", "", true, &RelayUrl::parse("wss://r.example").unwrap())
        .await
        .unwrap();
    let key = phone.groups.export_keys().await.unwrap().into_iter().find(|(g, _)| g == &view.id).unwrap().1;
    (view.id, Keys::generate(), key)
}

fn group_message(group_id: &str, key: &messenger_groups::GroupKey, author: &Keys, envelope: &Envelope, at: i64) -> serde_json::Value {
    let signed = sign_message(author, group_id, &envelope.encode(), at, None).unwrap();
    seal_message(group_id, key, None, &signed, author).unwrap().json
}

#[tokio::test]
async fn a_group_message_names_the_group_and_the_author() {
    let phone = Phone::new().await;
    let (gid, bob, key) = group_with_bob(&phone).await;
    phone.contacts.add(&phone.me(), &bob.public_key().to_hex(), Some("Боб")).await.unwrap();

    let event = group_message(&gid, &key, &bob, &Envelope::text("всем привет"), 1_000_000);
    let n = shown(phone.describe(group_push(&gid, &event)).await);
    assert_eq!(n.kind, ChatKind::Group);
    assert_eq!(n.chat.as_deref(), Some(format!("group:{gid}").as_str()));
    assert_eq!(n.title, "Пуш-тест");
    assert_eq!(n.sender, "Боб");
    assert_eq!(n.body, Some(Body::Text { text: "всем привет".into() }));

    phone.dm.set_muted(&format!("group:{gid}"), true).await.unwrap();
    assert!(shown(phone.describe(group_push(&gid, &event)).await).muted);
}

#[tokio::test]
async fn my_own_group_message_edits_and_unknown_keys() {
    let phone = Phone::new().await;
    let (gid, bob, key) = group_with_bob(&phone).await;

    let mine = group_message(&gid, &key, &phone.keys, &Envelope::text("from my other phone"), 1_000_000);
    assert_eq!(quiet(phone.describe(group_push(&gid, &mine)).await), Reason::Own);

    let edit = group_message(&gid, &key, &bob, &Envelope::edit("abc", "fixed"), 1_000_001);
    assert_eq!(quiet(phone.describe(group_push(&gid, &edit)).await), Reason::NotAMessage);

    // A key this phone does not have: the app will get it, the notice stays plain.
    let other = messenger_groups::GroupKey::generate().unwrap();
    let sealed = group_message(&gid, &other, &bob, &Envelope::text("?"), 1_000_002);
    let p = plain(phone.describe(group_push(&gid, &sealed)).await);
    assert_eq!(p.title.as_deref(), Some("Пуш-тест"));
    assert_eq!(p.chat.as_deref(), Some(format!("group:{gid}").as_str()));
}

#[tokio::test]
async fn a_group_i_am_not_in_says_nothing() {
    let phone = Phone::new().await;
    let (gid, bob, key) = group_with_bob(&phone).await;
    let event = group_message(&gid, &key, &bob, &Envelope::text("x"), 1_000_000);
    let bundle = phone.bundle().await;

    let other = Phone::new().await;
    // The other phone has the key somehow, but no such group.
    let outcome = describe(&other.dir.path().join("messenger"), Some(&KeyBundle::new(&other.keys, bundle.groups.clone())), &group_push(&gid, &event)).await.unwrap();
    assert_eq!(quiet(outcome), Reason::NotForMe);
}

#[tokio::test]
async fn an_event_that_is_not_what_the_push_says_is_refused() {
    let phone = Phone::new().await;
    let alice = Keys::generate();
    let event = dm_from(&alice, &phone, &Envelope::text("hi"), 1_000_000);
    let push = push_with(&[("type", "dm"), ("event", &event.to_string()), ("event_id", &"00".repeat(32))]);
    let bundle = phone.bundle().await;
    assert!(describe(&phone.dir.path().join("messenger"), Some(&bundle), &push).await.is_err());

    let mut forged = event.clone();
    forged["content"] = serde_json::Value::String("tampered".into());
    assert_eq!(quiet(phone.describe(dm_push(&forged)).await), Reason::Invalid);
}

#[tokio::test]
async fn a_database_from_another_version_is_refused() {
    let phone = Phone::new().await;
    sqlx::query("UPDATE _sqlx_migrations SET version = version + 1000 WHERE version = (SELECT MAX(version) FROM _sqlx_migrations)")
        .execute(phone.store.pool())
        .await
        .unwrap();
    let bundle = phone.bundle().await;
    let push = push_with(&[("type", "dm"), ("event", "{}")]);
    assert!(describe(&phone.dir.path().join("messenger"), Some(&bundle), &push).await.is_err());
}

#[tokio::test]
async fn without_keys_the_group_is_still_named() {
    let phone = Phone::new().await;
    let (gid, bob, key) = group_with_bob(&phone).await;
    phone.dm.set_muted(&format!("group:{gid}"), true).await.unwrap();
    let event = group_message(&gid, &key, &bob, &Envelope::text("x"), 1_000_000);
    let outcome = describe(&phone.dir.path().join("messenger"), None, &group_push(&gid, &event)).await.unwrap();
    let p = plain(outcome);
    assert_eq!(p.title.as_deref(), Some("Пуш-тест"));
    assert_eq!(p.chat.as_deref(), Some(format!("group:{gid}").as_str()));
    assert!(p.muted);
    let dm = describe(&phone.dir.path().join("messenger"), None, &push_with(&[("type", "dm"), ("event", "{}")])).await.unwrap();
    assert!(plain(dm).chat.is_none());
}

#[tokio::test]
async fn what_does_not_open_with_a_key_the_phone_has_says_nothing() {
    let phone = Phone::new().await;
    let (gid, bob, key) = group_with_bob(&phone).await;

    // Anybody can publish an event with the group's id and the id of its
    // key on it; without the key, what is inside is noise.
    let mut forged = group_message(&gid, &key, &bob, &Envelope::text("x"), 1_000_000);
    forged["content"] = serde_json::Value::String("bm90IGEgbWVzc2FnZSBvZiB0aGUgZ3JvdXA=".into());
    let resigned = {
        use nostr::prelude::*;
        let once = Keys::generate();
        let tags: Vec<Tag> = forged["tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| Tag::parse(t.as_array().unwrap().iter().map(|s| s.as_str().unwrap())).unwrap())
            .collect();
        let event = EventBuilder::new(Kind::from(9u16), forged["content"].as_str().unwrap()).tags(tags).finalize(&once).unwrap();
        serde_json::to_value(&event).unwrap()
    };
    assert_eq!(quiet(phone.describe(group_push(&gid, &resigned)).await), Reason::Invalid);
}

/// A card says whose it is, in a direct chat and in a group; one the app
/// would drop says nothing, as the app keeps nothing of it.
#[tokio::test]
async fn a_contact_card_is_told_and_a_broken_one_is_not() {
    let phone = Phone::new().await;
    let alice = Keys::generate();
    befriend(&phone, &alice, "Al").await;
    let card = |pubkey: &str| Envelope::contact(serde_json::json!({ "pubkey": pubkey, "display_name": "Анна", "at": 1 }));
    let anna = Keys::generate().public_key().to_hex();
    let want = Some(Body::Link { link: messenger_notify::LinkKind::Contact, title: "Анна".into() });

    let n = shown(phone.describe(dm_push(&dm_from(&alice, &phone, &card(&anna), 1_000_000))).await);
    assert_eq!(n.body, want);
    let junk = dm_from(&alice, &phone, &card("zz"), 1_000_001);
    assert_eq!(quiet(phone.describe(dm_push(&junk)).await), Reason::Invalid);
    let no_card = dm_from(&alice, &phone, &Envelope::new(messenger_core::envelope::T_CONTACT), 1_000_002);
    assert_eq!(quiet(phone.describe(dm_push(&no_card)).await), Reason::Invalid);

    let (gid, bob, key) = group_with_bob(&phone).await;
    let n = shown(phone.describe(group_push(&gid, &group_message(&gid, &key, &bob, &card(&anna), 1_000_003))).await);
    assert_eq!((n.kind, n.body), (ChatKind::Group, want));
    let junk = group_message(&gid, &key, &bob, &card("zz"), 1_000_004);
    assert_eq!(quiet(phone.describe(group_push(&gid, &junk)).await), Reason::Invalid);
}

// ─── Calls ───────────────────────────────────────────────────────────────────

/// A call rings only from somebody we both chose to talk with (the gate
/// of the app, `calls_allowed`): the phone then rings with who calls, by
/// the chat's name, and the invitation's times. A new offer inside a call,
/// a stranger's call and a malformed one ring nobody.
#[tokio::test]
async fn an_invitation_to_a_call_rings_from_a_mutual_contact_only() {
    let phone = Phone::new().await;
    let alice = Keys::generate();
    befriend(&phone, &alice, "Al").await;
    let id = "0123456789abcdef0123456789abcdef";
    let invite = Envelope::call_invite(id, "video", "v=0", vec![], false);
    let made = now();

    // My request alone is no mutual chat: the peer has not accepted.
    let event = dm_from(&alice, &phone, &invite, made);
    assert_eq!(quiet(phone.describe(dm_push(&event)).await), Reason::NotForMe);

    phone.app_received(&dm_from(&alice, &phone, &Envelope::control("dm_accept"), made - 1)).await;
    let call = match phone.describe(dm_push(&event)).await {
        Outcome::Call(c) => c,
        other => panic!("expected a call, got {other:?}"),
    };
    assert_eq!(call.call_id, id);
    assert_eq!(call.media, "video");
    assert_eq!(call.name, "Al");
    assert_eq!(call.peer_key, alice.public_key().to_hex());
    assert_eq!(call.created_at, made);
    assert_eq!(call.expires_at, made + messenger_notify::CALL_INVITE_TTL_SECS);
    assert!(!call.hide_on_lockscreen);

    let restart = dm_from(&alice, &phone, &Envelope::call_invite(id, "audio", "v=0", vec![], true), made);
    assert_eq!(quiet(phone.describe(dm_push(&restart)).await), Reason::NotAMessage);
    let malformed = dm_from(&alice, &phone, &Envelope::call_invite("short", "audio", "v=0", vec![], false), made);
    assert_eq!(quiet(phone.describe(dm_push(&malformed)).await), Reason::Invalid);
    let hologram = dm_from(&alice, &phone, &Envelope::call_invite(id, "hologram", "v=0", vec![], false), made);
    assert_eq!(quiet(phone.describe(dm_push(&hologram)).await), Reason::Invalid);

    let stranger = Keys::generate();
    let event = dm_from(&stranger, &phone, &invite, made);
    assert_eq!(quiet(phone.describe(dm_push(&event)).await), Reason::NotForMe);

    // The JSON the phone reads: the outcome's word and the fields by name.
    let json = serde_json::to_value(Outcome::Call(call)).unwrap();
    assert_eq!(json["outcome"], "call");
    assert_eq!(json["call_id"], id);
    assert_eq!(json["expires_at"], made + 45);
}

/// A push comes late as often as not. An invitation older than its life
/// (45 s from the time inside it, as the app judges) is a missed call for
/// the app to show, not a ringing phone; the last second still rings, as
/// in the app. A clock behind mine makes no difference to that.
#[tokio::test]
async fn an_invitation_older_than_its_life_is_a_missed_call_and_rings_nothing() {
    let phone = Phone::new().await;
    let alice = Keys::generate();
    befriend(&phone, &alice, "Al").await;
    phone.app_received(&dm_from(&alice, &phone, &Envelope::control("dm_accept"), 999_999)).await;
    let id = "0123456789abcdef0123456789abcdef";
    let invite = Envelope::call_invite(id, "audio", "v=0", vec![], false);

    let late = dm_from(&alice, &phone, &invite, now() - messenger_notify::CALL_INVITE_TTL_SECS - 2);
    assert_eq!(quiet(phone.describe(dm_push(&late)).await), Reason::Expired);
    let old = dm_from(&alice, &phone, &invite, 1_000_000);
    assert_eq!(quiet(phone.describe(dm_push(&old)).await), Reason::Expired);
    // Two seconds short of its life: the describing takes less than that.
    let in_time = dm_from(&alice, &phone, &invite, now() - messenger_notify::CALL_INVITE_TTL_SECS + 2);
    assert!(matches!(phone.describe(dm_push(&in_time)).await, Outcome::Call(_)));
    // Under "no content" too: nothing of a missed call is shown.
    Settings { content: Content::None, lockscreen_hidden: false }.save(&phone.store).await.unwrap();
    let marked = push_with(&[("type", "dm"), ("event", &late.to_string()), ("call", "1")]);
    assert_eq!(quiet(phone.describe(marked).await), Reason::Expired);
}

/// The app may have been up when the call came, or caught up with the
/// relays since: a call it has on record as answered (here or on another
/// device) or over (declined, missed, ended) rings nothing from a push
/// that comes later. A call still open on record (the app rings for it,
/// or never heard its end) rings; the plugin keeps one ringing per call.
#[tokio::test]
async fn a_call_the_app_has_on_record_as_over_rings_nothing() {
    let phone = Phone::new().await;
    let alice = Keys::generate();
    befriend(&phone, &alice, "Al").await;
    phone.app_received(&dm_from(&alice, &phone, &Envelope::control("dm_accept"), 999_999)).await;
    let made = now();
    let chat_id = format!("dm:{}", alice.public_key().to_hex());
    let row = |id: &str| calls::NewCall {
        call_id: id.into(),
        chat_id: chat_id.clone(),
        peer: alice.public_key().to_hex(),
        direction: "in".into(),
        media: "audio".into(),
        started_at: made,
    };
    let invite = |id: &str| dm_from(&alice, &phone, &Envelope::call_invite(id, "audio", "v=0", vec![], false), made);

    let open = "0123456789abcdef0123456789abcdef";
    calls::insert(&phone.store, &row(open)).await.unwrap();
    assert!(matches!(phone.describe(dm_push(&invite(open))).await, Outcome::Call(_)), "open on record: rings");

    let answered = "1123456789abcdef0123456789abcdef";
    calls::insert(&phone.store, &row(answered)).await.unwrap();
    calls::set_answered(&phone.store, answered, made).await.unwrap();
    assert_eq!(quiet(phone.describe(dm_push(&invite(answered))).await), Reason::Over);

    let missed = "2123456789abcdef0123456789abcdef";
    calls::insert(&phone.store, &row(missed)).await.unwrap();
    calls::finish(&phone.store, missed, calls::OUTCOME_MISSED, made).await.unwrap();
    assert_eq!(quiet(phone.describe(dm_push(&invite(missed))).await), Reason::Over);
    let marked = push_with(&[("type", "dm"), ("event", &invite(missed).to_string()), ("call", "1")]);
    Settings { content: Content::None, lockscreen_hidden: false }.save(&phone.store).await.unwrap();
    assert_eq!(quiet(phone.describe(marked).await), Reason::Over);
}

/// An invitation carries an offer of some 20 KB and never fits a push:
/// the push names the event and the relay, and the phone asks the relay.
/// A relay that cannot be reached (or has the event no more) leaves a
/// call-marked push with nothing to ring for, and nothing is shown: "a
/// new message" would be the notification of nothing, and the app shows
/// the missed call when it runs. A push of a message that cannot be had
/// still says that something came.
#[tokio::test]
async fn a_call_push_whose_event_cannot_be_had_shows_nothing() {
    let phone = Phone::new().await;
    let named = [("type", "dm"), ("event_id", "ef".repeat(32).leak()), ("relay", "ws://127.0.0.1:1")];
    assert_eq!(quiet(phone.describe(push_with(&[&named[..], &[("call", "1")]].concat())).await), Reason::Unreachable);
    assert_eq!(plain(phone.describe(push_with(&named)).await).kind, ChatKind::Dm);
    Settings { content: Content::None, lockscreen_hidden: false }.save(&phone.store).await.unwrap();
    assert_eq!(quiet(phone.describe(push_with(&[&named[..], &[("call", "1")]].concat())).await), Reason::Unreachable);
    assert_eq!(plain(phone.describe(push_with(&named)).await).kind, ChatKind::Dm);
}

/// A phone ringing from a push, with the app not up, hears that the call
/// is over by push too: my other device answered (declined, was busy,
/// ended), or the caller gave up. The caller's answer or refusal is for
/// the app that calls, a stranger's end is nothing, and the other signals
/// of a call are the app's.
#[tokio::test]
async fn the_end_of_a_call_stops_the_ringing_by_push() {
    let phone = Phone::new().await;
    let alice = Keys::generate();
    befriend(&phone, &alice, "Al").await;
    phone.app_received(&dm_from(&alice, &phone, &Envelope::control("dm_accept"), 999_999)).await;
    let id = "0123456789abcdef0123456789abcdef";
    let at = now();
    let ended = |outcome: Outcome| match outcome {
        Outcome::CallEnd(end) => end.call_id,
        other => panic!("expected the end of a call, got {other:?}"),
    };
    let mine = |envelope: &Envelope| wrap_note_as(&phone.keys, &pk(&alice), &envelope.encode(), at, Some(at + 300), Wake::Nobody, Some(Wake::CallEnd)).unwrap().to_self.unwrap().json;
    let theirs = |from: &Keys, envelope: &Envelope| wrap_note_as(from, &phone.me(), &envelope.encode(), at, Some(at + 300), Wake::CallEnd, None).unwrap().to_peer.json;

    for envelope in [
        Envelope::call_answer(id, "v=0", vec![]),
        Envelope::call_decline(id, "declined"),
        Envelope::call_busy(id),
        Envelope::call_end(id, "ended", None),
    ] {
        assert_eq!(ended(phone.describe(dm_push(&mine(&envelope))).await), id, "{}", envelope.t);
    }
    assert_eq!(ended(phone.describe(dm_push(&theirs(&alice, &Envelope::call_end(id, "timeout", None)))).await), id);
    assert_eq!(quiet(phone.describe(dm_push(&theirs(&alice, &Envelope::call_answer(id, "v=0", vec![])))).await), Reason::NotAMessage);
    assert_eq!(quiet(phone.describe(dm_push(&theirs(&alice, &Envelope::call_ice(id, vec![])))).await), Reason::NotAMessage);
    assert_eq!(quiet(phone.describe(dm_push(&mine(&Envelope::call_ice(id, vec![])))).await), Reason::NotAMessage);
    assert_eq!(quiet(phone.describe(dm_push(&mine(&Envelope::call_end("short", "ended", None)))).await), Reason::NotAMessage);
    let stranger = Keys::generate();
    assert_eq!(quiet(phone.describe(dm_push(&theirs(&stranger, &Envelope::call_end(id, "ended", None)))).await), Reason::NotAMessage);

    // The JSON the phone reads.
    let json = serde_json::to_value(phone.describe(dm_push(&mine(&Envelope::call_answer(id, "v=0", vec![])))).await).unwrap();
    assert_eq!(json["outcome"], "call_end");
    assert_eq!(json["call_id"], id);

    // Under "no content" the push the server marked is opened for this too.
    Settings { content: Content::None, lockscreen_hidden: true }.save(&phone.store).await.unwrap();
    let end = mine(&Envelope::call_answer(id, "v=0", vec![]));
    assert_eq!(ended(phone.describe(push_with(&[("type", "dm"), ("event", &end.to_string()), ("call", "1")])).await), id);
    assert!(matches!(phone.describe(dm_push(&end)).await, Outcome::Plain(_)), "unmarked: not opened");
}

/// While its notifications show no content (and no PIN guards the app:
/// under a PIN the handler gets no keys at all), the app hands the phone
/// its key for calls alone: a message is not opened and says only that
/// something came, as without keys; a call rings, nameless, and the end
/// of a call stops the ringing. A push marked as a call that carries no
/// call says nothing: "something came" is no call.
#[tokio::test]
async fn keys_for_calls_alone_ring_and_say_nothing_of_messages() {
    let phone = Phone::new().await;
    let alice = Keys::generate();
    befriend(&phone, &alice, "Al").await;
    phone.app_received(&dm_from(&alice, &phone, &Envelope::control("dm_accept"), 999_999)).await;
    let bundle = KeyBundle::for_calls_only(&phone.keys);
    let id = "0123456789abcdef0123456789abcdef";
    let at = now();
    let marked = |event: &serde_json::Value| push_with(&[("type", "dm"), ("event", &event.to_string()), ("call", "1")]);

    let text = dm_from(&alice, &phone, &Envelope::text("secret"), at);
    assert!(plain(phone.describe_with(&bundle, dm_push(&text)).await).chat.is_none());
    assert_eq!(quiet(phone.describe_with(&bundle, marked(&text)).await), Reason::NotACall);

    let invite = dm_from(&alice, &phone, &Envelope::call_invite(id, "audio", "v=0", vec![], false), at);
    let Outcome::Call(call) = phone.describe_with(&bundle, marked(&invite)).await else { panic!("a call") };
    assert_eq!(call.call_id, id);
    assert_eq!(call.name, "", "nobody is named");
    assert!(call.picture.is_none());
    assert!(matches!(phone.describe_with(&bundle, dm_push(&invite)).await, Outcome::Plain(_)), "unmarked: not opened");

    let answer = wrap_note_as(&phone.keys, &pk(&alice), &Envelope::call_answer(id, "v=0", vec![]).encode(), at, None, Wake::Nobody, Some(Wake::CallEnd)).unwrap().to_self.unwrap().json;
    assert!(matches!(phone.describe_with(&bundle, marked(&answer)).await, Outcome::CallEnd(_)));
}

/// The life of an invitation is the core's word (`messenger_calls`), which
/// this crate spells again rather than depend on the core of calls.
#[test]
fn the_life_of_an_invitation_is_the_cores() {
    assert_eq!(messenger_notify::CALL_INVITE_TTL_SECS, messenger_calls::INVITE_TTL_SECS);
}

/// A contact whose clock is ahead sends an invitation from the future: it
/// is good for its life from now, as the app judges it, and not for as
/// long as the limit of a ringing would let a push ring.
#[tokio::test]
async fn an_invitation_from_a_clock_ahead_rings_for_its_life_from_now() {
    let phone = Phone::new().await;
    let alice = Keys::generate();
    befriend(&phone, &alice, "Al").await;
    phone.app_received(&dm_from(&alice, &phone, &Envelope::control("dm_accept"), 999_999)).await;
    let id = "0123456789abcdef0123456789abcdef";
    let before = SystemClock.now().secs();
    let event = dm_from(&alice, &phone, &Envelope::call_invite(id, "audio", "v=0", vec![], false), before + 600);
    let Outcome::Call(call) = phone.describe(dm_push(&event)).await else { panic!("a call") };
    let after = SystemClock.now().secs();
    assert!((before..=after).contains(&call.created_at), "{} is not now", call.created_at);
    assert_eq!(call.expires_at, call.created_at + messenger_notify::CALL_INVITE_TTL_SECS);
}

/// "Only that something came" says nothing of a message, and a message
/// is not even opened; a call must still ring, as it does on a computer
/// and in the running app, so the push the server marked as a call is
/// opened and rings with no name and no picture. The mark is anyone's to
/// put on a wrap: a stranger's "call", or a message so marked, says
/// nothing, since "something came" for a push that promised a ring is the
/// notification of nothing (the phone would show an empty "New message").
#[tokio::test]
async fn under_no_content_a_call_rings_nameless_and_a_message_says_nothing() {
    let phone = Phone::new().await;
    let alice = Keys::generate();
    befriend(&phone, &alice, "Al").await;
    phone.app_received(&dm_from(&alice, &phone, &Envelope::control("dm_accept"), 999_999)).await;
    Settings { content: Content::None, lockscreen_hidden: true }.save(&phone.store).await.unwrap();
    let id = "0123456789abcdef0123456789abcdef";
    let invite = Envelope::call_invite(id, "video", "v=0", vec![], false);
    let marked = |event: &serde_json::Value| push_with(&[("type", "dm"), ("event", &event.to_string()), ("call", "1")]);

    let event = dm_from(&alice, &phone, &invite, now());
    let Outcome::Call(call) = phone.describe(marked(&event)).await else { panic!("a call") };
    assert_eq!(call.call_id, id);
    assert_eq!(call.media, "video");
    assert_eq!(call.name, "", "no content: nobody is named");
    assert!(call.picture.is_none());
    assert!(call.hide_on_lockscreen);

    // The same invitation in a push the server did not mark: not opened.
    let p = plain(phone.describe(dm_push(&event)).await);
    assert_eq!(p.kind, ChatKind::Dm);
    // A message in a push marked as a call: opened, found not to be one,
    // and not a word of it (the app shows it when it runs).
    let text = dm_from(&alice, &phone, &Envelope::text("secret"), 1_000_001);
    assert_eq!(quiet(phone.describe(marked(&text)).await), Reason::NotACall);
    // A stranger's call: as in the app, nothing.
    let stranger = Keys::generate();
    let event = dm_from(&stranger, &phone, &invite, 1_000_002);
    assert_eq!(quiet(phone.describe(marked(&event)).await), Reason::NotForMe);
    // A push marked as a call whose event is not the one it names: an
    // error, as with the content shown, and never "something came".
    let other = dm_from(&alice, &phone, &invite, 1_000_003);
    let misnamed = push_with(&[("type", "dm"), ("event", &other.to_string()), ("event_id", &"ab".repeat(32)), ("call", "1")]);
    assert!(describe(&phone.dir.path().join("messenger"), Some(&phone.bundle().await), &misnamed).await.is_err());
}

/// The owner's phone under a PIN (the app hands the handler no keys),
/// or one whose keys are not handed over yet, gets the push of a call:
/// there is nothing to open it with, so nothing rings and nothing is
/// shown. A "New message" with no name and no text was what the phone
/// showed for it before, and the owner took it for an empty push. A push
/// of a message without keys still says that something came, as before.
#[tokio::test]
async fn a_call_push_without_keys_rings_nothing_and_says_nothing() {
    let phone = Phone::new().await;
    let alice = Keys::generate();
    befriend(&phone, &alice, "Al").await;
    phone.app_received(&dm_from(&alice, &phone, &Envelope::control("dm_accept"), 999_999)).await;
    let dir = phone.dir.path().join("messenger");
    let id = "0123456789abcdef0123456789abcdef";
    let at = now();
    let marked = |event: &serde_json::Value| push_with(&[("type", "dm"), ("event", &event.to_string()), ("call", "1")]);

    let invite = dm_from(&alice, &phone, &Envelope::call_invite(id, "audio", "v=0", vec![], false), at);
    assert_eq!(quiet(describe(&dir, None, &marked(&invite)).await.unwrap()), Reason::NoKeys);
    let end = wrap_note_as(&phone.keys, &pk(&alice), &Envelope::call_answer(id, "v=0", vec![]).encode(), at, None, Wake::Nobody, Some(Wake::CallEnd)).unwrap().to_self.unwrap().json;
    assert_eq!(quiet(describe(&dir, None, &marked(&end)).await.unwrap()), Reason::NoKeys);
    // A call named by id and relay, not carried: no keys would open it either.
    let named = push_with(&[("type", "dm"), ("event_id", &"ef".repeat(32)), ("relay", "ws://127.0.0.1:1"), ("call", "1")]);
    assert_eq!(quiet(describe(&dir, None, &named).await.unwrap()), Reason::NoKeys);

    // Not marked as a call: "something came", as ever without keys.
    let text = dm_from(&alice, &phone, &Envelope::text("secret"), at);
    assert_eq!(plain(describe(&dir, None, &dm_push(&text)).await.unwrap()).kind, ChatKind::Dm);
    assert_eq!(plain(describe(&dir, None, &dm_push(&invite)).await.unwrap()).kind, ChatKind::Dm);

    // With the keys the same pushes ring and stop the ringing.
    let Outcome::Call(call) = phone.describe(marked(&invite)).await else { panic!("a call") };
    assert_eq!(call.call_id, id);
    assert!(matches!(phone.describe(marked(&end)).await, Outcome::CallEnd(_)));
}
