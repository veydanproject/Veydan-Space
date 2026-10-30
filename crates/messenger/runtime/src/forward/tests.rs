// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A chat of Alice with Anna, forwarded to her contact Bob and to her
//! group; Bob is a whole runtime, kept off the network: what Alice queues
//! he hears through the same classify and dispatch as a relay's events.

use super::*;
use crate::session::RuntimeSink;
use messenger_core::traits::SystemClock;
use messenger_core::{Context, EventSource, MessengerConfig, Outbound, RawEvent, RelayUrl, Timestamp};
use messenger_groups::GroupKind;
use messenger_media::MemoryBackend;
use messenger_store::messages::NewMessage;
use messenger_testkit::MemorySecretStore;
use nostr::prelude::Event;
use std::sync::Arc;

const SERVER: &str = "https://mem.example/a";

/// Reads the blobs of a memory backend as a server would hand them out.
struct MemFetch(MemoryBackend);

#[async_trait::async_trait]
impl messenger_media::download::BlobFetcher for MemFetch {
    async fn fetch(&self, url: &str, _max: u64) -> Result<Option<Vec<u8>>> {
        Ok(url.rsplit('/').next().and_then(|sha| self.0.get(sha)))
    }
}

/// A runtime with a session, its files going to `backend`.
async fn started(backend: &MemoryBackend) -> (tempfile::TempDir, MessengerRuntime, Keys) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = MessengerConfig::new(dir.path().join("messenger"));
    let mut rt = MessengerRuntime::start(cfg, Arc::new(MemorySecretStore::unlocked())).await.unwrap();
    rt.relays().set_silent(true).await.unwrap();
    crate::servers::use_veydan_offline(&rt).await;
    rt.identity().create("pw").await.unwrap();
    assert!(rt.refresh_signer().await.unwrap());
    rt.dm().set_gate(false);
    rt.media = rt.media.clone().with_backend(Arc::new(backend.clone()), Arc::new(MemFetch(backend.clone())));
    let keys = rt.session_keys().await.unwrap();
    (dir, rt, keys)
}

/// The public profile of `who` as `rt` heard it.
async fn profile_of(rt: &MessengerRuntime, who: &Keys, content: &str) {
    let pk = PubKey::parse(&hex(who)).unwrap();
    assert!(rt.profiles().apply_event(&pk, Timestamp(1), content).await.unwrap());
}

fn hex(k: &Keys) -> String {
    k.public_key().to_hex()
}

/// Everything `from` has queued, as `to` would hear it from a relay.
async fn deliver(from: &MessengerRuntime, to: &MessengerRuntime) {
    let keys = to.session_keys().await.unwrap();
    let ctx = Context { my_pubkey: PubKey::parse(&hex(&keys)).unwrap(), session_started_at: Timestamp(0), clock: Arc::new(SystemClock) };
    let sink = RuntimeSink { pool: to.relays.pool().await, outbox: to.outbox.clone(), ui: to.ui.clone() };
    for row in messenger_store::outbox::due(from.store(), i64::MAX / 4, 0).await.unwrap() {
        let event = match row.outbound().unwrap() {
            Outbound::PublishToInbox { event, .. } | Outbound::PublishOwn { event } => event,
            _ => continue,
        };
        let ev: Event = serde_json::from_value(event.json.clone()).unwrap();
        let raw = RawEvent {
            id: event.id.clone(),
            kind: ev.kind.as_u16(),
            pubkey: PubKey::parse(&ev.pubkey.to_hex()).unwrap(),
            created_at: Timestamp(ev.created_at.as_secs() as i64),
            json: event.json.clone(),
            source: EventSource::Relay { url: RelayUrl::parse("wss://relay.example").unwrap() },
        };
        to.dispatcher.dispatch(messenger_ingress::classify(&raw, Some(&keys)), &ctx, &sink).await;
    }
}

fn code<T: std::fmt::Debug>(r: Result<T>) -> String {
    match r {
        Err(MessengerError::Invalid(c)) => c,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// The stored descriptor of a file of `size` bytes on `SERVER`, `seed`
/// making its hashes its own.
fn descriptor(kind: &str, name: &str, seed: char, batch: Option<(&str, u32)>) -> serde_json::Value {
    let size = 100u64;
    let mut d = serde_json::json!({
        "kind": kind,
        "name": name,
        "mime": messenger_media::descriptor::mime_for(name),
        "size": size,
        "sha256": seed.to_string().repeat(64),
        "chunk_size": 64 * 1024,
        "chunks": [{ "sha256": format!("{seed}{}", "0".repeat(63)), "size": size + messenger_media::crypto::TAG_LEN as u64 }],
        "algo": messenger_media::descriptor::ALGO,
        "key": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        "iv": "AAAAAAAAAAAAAAAA",
        "servers": [SERVER],
    });
    if kind == "voice" {
        d["duration_ms"] = 4200.into();
        d["waveform"] = serde_json::json!([1, 2, 3]);
    }
    if let Some((b, pos)) = batch {
        d["batch"] = b.into();
        d["pos"] = pos.into();
    }
    MediaDescriptor::from_fields(&d).expect("a valid descriptor");
    d
}

/// A row of `chat` as it was stored when it came (or went).
#[allow(clippy::too_many_arguments)]
async fn stored(
    rt: &MessengerRuntime,
    chat: &str,
    id: &str,
    from: &str,
    at: i64,
    content_type: &str,
    text: Option<&str>,
    envelope: &Envelope,
    media: Option<serde_json::Value>,
) {
    let mine = from == hex(&rt.session_keys().await.unwrap());
    repo::insert(
        rt.store(),
        &NewMessage {
            id: id.into(),
            chat_id: chat.into(),
            wire_id: Some(format!("w-{id}")),
            direction: if mine { repo::DIR_OUT.into() } else { repo::DIR_IN.into() },
            status: if mine { repo::STATUS_SENT.into() } else { repo::STATUS_RECEIVED.into() },
            content_type: content_type.into(),
            text: text.map(String::from),
            envelope_json: envelope.encode(),
            sender_pubkey: from.into(),
            reply_to_id: None,
            target_id: None,
            created_at: at,
            is_hidden: false,
            outbox_local_id: None,
            media_json: media.map(|m| m.to_string()),
        },
    )
    .await
    .unwrap();
}

async fn media_row(rt: &MessengerRuntime, chat: &str, id: &str, from: &str, at: i64, d: serde_json::Value, caption: Option<&str>) {
    let mut e = MediaDescriptor::from_fields(&d).unwrap().to_envelope();
    if let Some(c) = caption {
        e = e.with("caption", c);
    }
    stored(rt, chat, id, from, at, repo::CT_MEDIA, caption, &e, Some(d)).await;
}

/// The chat with Anna: a text, a photo, an album of three, a voice
/// message and a file, as they came; ids out of the order of the chat.
async fn annas_chat(rt: &MessengerRuntime, anna: &str) -> (String, Vec<String>) {
    let chat = messenger_store::chats::ensure_dm(rt.store(), anna, None).await.unwrap().id;
    let text = Envelope::text("**план** на завтра").with_format(TextFormat::Markdown);
    stored(rt, &chat, "m1", anna, 100, repo::CT_TEXT, Some("**план** на завтра"), &text, None).await;
    media_row(rt, &chat, "m2", anna, 101, descriptor("image", "cat.jpg", 'a', None), Some("кот")).await;
    for (i, id) in ["m3a", "m3b", "m3c"].iter().enumerate() {
        let seed = ['b', 'c', 'd'][i];
        media_row(rt, &chat, id, anna, 102 + i as i64, descriptor("image", &format!("p{i}.jpg"), seed, Some(("album001", i as u32))), None).await;
    }
    media_row(rt, &chat, "m4", anna, 105, descriptor("voice", "voice.weba", 'e', None), None).await;
    media_row(rt, &chat, "m5", anna, 106, descriptor("file", "plan.pdf", 'f', None), None).await;
    let ids = ["m5", "m3b", "m1", "m4", "m3a", "m2", "m3c"].iter().map(|s| s.to_string()).collect();
    (chat, ids)
}

fn envelope_of(m: &MessageView, rows: &HashMap<String, MessageRow>) -> Envelope {
    Envelope::parse(&rows[&m.id].envelope_json).unwrap()
}

async fn rows_of(rt: &MessengerRuntime, ms: &[MessageView]) -> HashMap<String, MessageRow> {
    let mut out = HashMap::new();
    for m in ms {
        out.insert(m.id.clone(), repo::get(rt.store(), &m.id).await.unwrap().unwrap());
    }
    out
}

#[tokio::test]
async fn messages_go_in_their_order_and_files_name_the_same_blobs() {
    let backend = MemoryBackend::new(SERVER);
    let (_a, alice, a_keys) = started(&backend).await;
    let (_b, bob, b_keys) = started(&MemoryBackend::new(SERVER)).await;
    let anna = Keys::generate();
    alice.contact_add(&hex(&b_keys), None).await.unwrap();
    alice.contact_add(&hex(&anna), Some("Аня")).await.unwrap();
    profile_of(&alice, &anna, r#"{"name":"anna","display_name":"Анна"}"#).await;
    let group = alice.group_create(GroupKind::Private, "Team", "", true, None).await.unwrap();
    let (chat, ids) = annas_chat(&alice, &hex(&anna)).await;
    let before = messenger_store::outbox::due(alice.store(), i64::MAX / 4, 0).await.unwrap().len();

    let to_bob = format!("dm:{}", hex(&b_keys));
    let to_group = format!("group:{}", group.id);
    let results = alice.forward(&chat, &ids, &[to_bob.clone(), to_group.clone()], false, Some("  смотри  ")).await.unwrap();

    assert_eq!(results.iter().map(|r| r.target.as_str()).collect::<Vec<_>>(), [to_bob.as_str(), to_group.as_str()]);
    let mut batches = Vec::new();
    for r in &results {
        assert_eq!((r.error.as_deref(), r.skipped.len()), (None, 0), "{r:?}");
        assert_eq!(r.sent.len(), 8, "the comment and seven messages");
        assert!(r.sent.iter().all(|m| m.chat_id == r.target && m.direction == "out"));
        let rows = rows_of(&alice, &r.sent).await;
        let comment = envelope_of(&r.sent[0], &rows);
        assert_eq!((comment.as_text(), comment.fwd()), (Some("смотри"), None), "the comment is mine, and first");

        let text = envelope_of(&r.sent[1], &rows);
        assert_eq!(text.as_text(), Some("**план** на завтра"));
        assert_eq!(text.format(), TextFormat::Markdown, "written as it was");
        assert_eq!(text.fwd(), Some(Forwarded::named(Some("Анна"))), "the name of her profile");
        for m in &r.sent {
            let json = &rows[&m.id].envelope_json;
            assert!(!json.contains(&hex(&anna)), "no key of the author: {json}");
            assert!(!json.contains("Аня"), "nor my name for her: {json}");
        }

        let names: Vec<String> = r.sent[2..].iter().map(|m| m.media.as_ref().unwrap()["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(names, ["cat.jpg", "p0.jpg", "p1.jpg", "p2.jpg", "voice.weba", "plan.pdf"], "the order of the chat");
        let source: Vec<char> = vec!['a', 'b', 'c', 'd', 'e', 'f'];
        for (m, seed) in r.sent[2..].iter().zip(source) {
            let e = envelope_of(m, &rows);
            let d = MediaDescriptor::from_envelope(&e).unwrap();
            assert_eq!(d.chunks[0].sha256, format!("{seed}{}", "0".repeat(63)), "the same blob");
            assert_eq!(d.servers, [SERVER]);
            assert_eq!(e.fwd(), Some(Forwarded::named(Some("Анна"))));
            assert!(!e.fields.contains_key("local_path") && !e.fields.contains_key("transfer_id"), "{}", e.encode());
        }
        let photo = envelope_of(&r.sent[2], &rows);
        assert_eq!((photo.str_field("caption"), r.sent[2].text.as_deref()), (Some("кот"), Some("кот")));
        let voice = MediaDescriptor::from_envelope(&envelope_of(&r.sent[6], &rows)).unwrap();
        assert_eq!((voice.kind, voice.duration_ms), (messenger_media::MediaKind::Voice, Some(4200)));
        let album: Vec<MediaDescriptor> = r.sent[3..6].iter().map(|m| MediaDescriptor::from_envelope(&envelope_of(m, &rows)).unwrap()).collect();
        let batch = album[0].batch.clone().unwrap();
        assert!(messenger_media::descriptor::valid_batch(&batch) && batch != "album001", "a new album: {batch}");
        assert!(album.iter().all(|d| d.batch.as_deref() == Some(batch.as_str())), "one album");
        assert_eq!(album.iter().map(|d| d.pos).collect::<Vec<_>>(), [Some(0), Some(1), Some(2)], "each in its place");
        batches.push(batch);
    }
    assert_ne!(batches[0], batches[1], "an album of its own in each chat");
    assert_eq!(backend.len(), 0, "nothing was uploaded");
    assert!(alice.media_transfers().await.unwrap().is_empty(), "nor fetched");
    let queued = messenger_store::outbox::due(alice.store(), i64::MAX / 4, 0).await.unwrap().len();
    assert!(queued > before, "the messages go the usual way");

    // Bob gets them as they were, «forwarded from Анна», in the same order.
    deliver(&alice, &bob).await;
    let bobs = bob.dm().open_chat(&PubKey::parse(&hex(&a_keys)).unwrap()).await.unwrap();
    let got = bob.dm().messages(&bobs.id, None, 50).await.unwrap();
    assert_eq!(got.len(), 8, "{got:?}");
    assert_eq!((got[0].text.as_deref(), got[0].fwd.as_ref()), (Some("смотри"), None));
    assert_eq!(got[1].fwd, Some(Forwarded::named(Some("Анна"))));
    assert_eq!(got[1].fmt.as_deref(), Some("md"));
    let names: Vec<&str> = got[2..].iter().map(|m| m.media.as_ref().unwrap()["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["cat.jpg", "p0.jpg", "p1.jpg", "p2.jpg", "voice.weba", "plan.pdf"]);
    assert!(got[2..].iter().all(|m| m.fwd == Some(Forwarded::named(Some("Анна")))));
    let line = bob.chats(false).await.unwrap().into_iter().find(|c| c.id == bobs.id).unwrap().last_body;
    assert!(matches!(line, Some(messenger_core::Body::Forwarded { .. })), "the list says it is forwarded: {line:?}");

    alice.shutdown().await;
    bob.shutdown().await;
}

#[tokio::test]
async fn a_hidden_sender_leaves_no_trace_and_a_forward_keeps_its_first_author() {
    let backend = MemoryBackend::new(SERVER);
    let (_a, alice, a_keys) = started(&backend).await;
    let bob = Keys::generate();
    let anna = Keys::generate();
    alice.contact_add(&hex(&bob), None).await.unwrap();
    alice.contact_add(&hex(&anna), Some("Аня")).await.unwrap();
    profile_of(&alice, &anna, r#"{"name":" ","display_name":""}"#).await;
    let chat = messenger_store::chats::ensure_dm(alice.store(), &hex(&anna), None).await.unwrap().id;
    let first = Envelope::text("от Вики").with_fwd(Some(&Forwarded::named(Some("Вика"))));
    stored(&alice, &chat, "f1", &hex(&anna), 100, repo::CT_TEXT, Some("от Вики"), &first, None).await;
    stored(&alice, &chat, "f2", &hex(&anna), 101, repo::CT_TEXT, Some("от Ани"), &Envelope::text("от Ани"), None).await;
    stored(&alice, &chat, "f3", &hex(&a_keys), 102, repo::CT_TEXT, Some("моё"), &Envelope::text("моё"), None).await;
    let photo = descriptor("image", "a.jpg", 'a', None);
    let mut photo_env = MediaDescriptor::from_fields(&photo).unwrap().to_envelope().with_fwd(Some(&Forwarded::named(Some("Вика"))));
    photo_env = photo_env.with("caption", "c");
    stored(&alice, &chat, "f4", &hex(&anna), 103, repo::CT_MEDIA, Some("c"), &photo_env, Some(photo)).await;
    let ids: Vec<String> = ["f1", "f2", "f3", "f4"].iter().map(|s| s.to_string()).collect();
    let to = [format!("dm:{}", hex(&bob))];

    let shown = alice.forward(&chat, &ids, &to, false, None).await.unwrap().remove(0);
    let rows = rows_of(&alice, &shown.sent).await;
    let fwd: Vec<Option<Forwarded>> = shown.sent.iter().map(|m| envelope_of(m, &rows).fwd()).collect();
    assert_eq!(
        fwd,
        [Some(Forwarded::named(Some("Вика"))), Some(Forwarded::default()), Some(Forwarded::default()), Some(Forwarded::named(Some("Вика")))],
        "the first author stays; one without a public name is forwarded from nobody named, not by my name for her"
    );
    assert_eq!(shown.sent[1].fwd, Some(Forwarded::default()), "the view says so too");
    for m in &shown.sent {
        let json = &rows[&m.id].envelope_json;
        assert!(!json.contains(&hex(&anna)) && !json.contains(&hex(&a_keys)), "no key: {json}");
        assert!(!json.contains("Аня"), "no name of my address book: {json}");
    }

    let hidden = alice.forward(&chat, &ids, &to, true, None).await.unwrap().remove(0);
    let rows = rows_of(&alice, &hidden.sent).await;
    for m in &hidden.sent {
        assert!(!rows[&m.id].envelope_json.contains(FIELD_FWD), "no fwd: {}", rows[&m.id].envelope_json);
        assert_eq!(m.fwd, None);
    }
    assert_eq!(hidden.sent.iter().map(|m| m.text.as_deref().unwrap()).collect::<Vec<_>>(), ["от Вики", "от Ани", "моё", "c"]);
    alice.shutdown().await;
}

#[tokio::test]
async fn what_may_not_go_and_where_it_may_not_go_is_refused() {
    let backend = MemoryBackend::new(SERVER);
    let (_a, alice, a_keys) = started(&backend).await;
    let bob = Keys::generate();
    let stranger = Keys::generate();
    let archived = Keys::generate();
    let anna = Keys::generate();
    alice.contact_add(&hex(&bob), None).await.unwrap();
    alice.contact_add(&hex(&archived), None).await.unwrap();
    let chat = messenger_store::chats::ensure_dm(alice.store(), &hex(&anna), None).await.unwrap().id;
    stored(&alice, &chat, "ok", &hex(&anna), 100, repo::CT_TEXT, Some("hi"), &Envelope::text("hi"), None).await;
    let to_bob = vec![format!("dm:{}", hex(&bob))];
    let one = vec!["ok".to_string()];

    // Messages that may not go.
    stored(&alice, &chat, "gone", &hex(&anna), 101, repo::CT_TEXT, Some("x"), &Envelope::text("x"), None).await;
    repo::mark_deleted(alice.store(), "gone", 102).await.unwrap();
    stored(&alice, &chat, "failed", &hex(&a_keys), 103, repo::CT_TEXT, Some("x"), &Envelope::text("x"), None).await;
    repo::set_status(alice.store(), "failed", repo::STATUS_FAILED, Some("err.network")).await.unwrap();
    stored(&alice, &chat, "queued", &hex(&a_keys), 104, repo::CT_TEXT, Some("x"), &Envelope::text("x"), None).await;
    repo::set_status(alice.store(), "queued", repo::STATUS_QUEUED, None).await.unwrap();
    stored(&alice, &chat, "system", &hex(&anna), 105, repo::CT_SYSTEM, Some("x"), &Envelope::new("control"), None).await;
    stored(&alice, &chat, "odd", &hex(&anna), 106, "poll", Some("x"), &Envelope::new("poll"), None).await;
    let uploading = serde_json::json!({ "name": "a.jpg", "kind": "image", "local_path": "/tmp/a.jpg" });
    stored(&alice, &chat, "local:up", &hex(&a_keys), 107, repo::CT_MEDIA, None, &Envelope::new("media"), Some(uploading)).await;
    let elsewhere = messenger_store::chats::ensure_dm(alice.store(), &hex(&bob), None).await.unwrap().id;
    stored(&alice, &elsewhere, "other", &hex(&bob), 108, repo::CT_TEXT, Some("x"), &Envelope::text("x"), None).await;
    for id in ["gone", "failed", "queued", "system", "odd", "local:up", "other", "nobody"] {
        let ids = vec!["ok".to_string(), id.to_string()];
        assert_eq!(code(alice.forward(&chat, &ids, &to_bob, false, None).await), "forward_message_unfit", "{id}");
    }

    // Chats it may not go to.
    messenger_store::chats::ensure_dm(alice.store(), &hex(&stranger), None).await.unwrap();
    messenger_store::chats::ensure_dm(alice.store(), &hex(&archived), None).await.unwrap();
    alice.dm().set_archived(&format!("dm:{}", hex(&archived)), true).await.unwrap();
    for target in [format!("dm:{}", hex(&stranger)), format!("dm:{}", hex(&archived)), "group:nope".into(), "channel:x".into(), "dm:".into()] {
        let targets = vec![to_bob[0].clone(), target.clone()];
        assert_eq!(code(alice.forward(&chat, &one, &targets, false, None).await), "forward_target_unfit", "{target}");
    }
    let sent_before = alice.dm().messages(&to_bob[0], None, 50).await.unwrap().len();

    // Limits, and nothing to do.
    let many: Vec<String> = (0..=MAX_FORWARD_MESSAGES).map(|i| format!("m{i}")).collect();
    assert_eq!(code(alice.forward(&chat, &many, &to_bob, false, None).await), "forward_too_many_messages");
    let chats: Vec<String> = (0..=MAX_FORWARD_TARGETS).map(|_| format!("dm:{}", hex(&Keys::generate()))).collect();
    assert_eq!(code(alice.forward(&chat, &one, &chats, false, None).await), "forward_too_many_targets");
    assert_eq!(code(alice.forward(&chat, &[], &to_bob, false, None).await), "forward_nothing");
    assert_eq!(code(alice.forward(&chat, &one, &[], false, None).await), "forward_nothing");
    let twice = vec![to_bob[0].clone(); MAX_FORWARD_TARGETS + 5];
    assert_eq!(alice.forward(&chat, &one, &twice, false, None).await.unwrap().len(), 1, "a chat named twice is one");
    assert_eq!(alice.dm().messages(&to_bob[0], None, 50).await.unwrap().len(), sent_before + 1, "nothing went before that");
    alice.shutdown().await;
}

/// A chat of Alice on a private server, its files sent from this device
/// and kept in `outgoing/` as the app keeps what it sent: forwarded to a
/// chat of the home set, they go up again.
async fn private_chat_with_files(rt: &MessengerRuntime, anna: &Keys, files: &[(&str, &str, &[u8])]) -> (String, Vec<PathBuf>) {
    let me = hex(&rt.session_keys().await.unwrap());
    let chat = messenger_store::chats::ensure_dm(rt.store(), &hex(anna), Some("srv.example")).await.unwrap().id;
    let mut paths = Vec::new();
    for (i, (id, name, bytes)) in files.iter().enumerate() {
        let dir = rt.config.data_dir().join("outgoing").join(format!("sent{i}"));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join(name);
        tokio::fs::write(&path, bytes).await.unwrap();
        let kind = if name.ends_with(".mp4") { "video" } else { "file" };
        let mut d = descriptor(kind, name, ['a', 'b', 'c'][i], None);
        if kind == "video" {
            d["thumb"] = THUMB.into();
            d["dim"] = serde_json::json!([1280, 720]);
        }
        d["local_path"] = path.to_string_lossy().into_owned().into();
        media_row(rt, &chat, id, &me, 100 + i as i64, d, None).await;
        paths.push(path);
    }
    (chat, paths)
}

/// A JPEG as far as its first bytes go.
const THUMB: &str = "/9j/4AAQSkZJRg==";

#[tokio::test]
async fn a_file_that_goes_up_again_keeps_its_place_and_its_preview() {
    let backend = MemoryBackend::new(SERVER);
    *backend.put_delay.lock().unwrap() = std::time::Duration::from_millis(300);
    let (_a, alice, _) = started(&backend).await;
    let (bob, anna) = (Keys::generate(), Keys::generate());
    alice.contact_add(&hex(&bob), None).await.unwrap();
    let group = alice.group_create(GroupKind::Private, "Team", "", true, None).await.unwrap();
    let (chat, paths) = private_chat_with_files(&alice, &anna, &[("v1", "clip.mp4", b"a video, slow to go up")]).await;
    let me = hex(&alice.session_keys().await.unwrap());
    stored(&alice, &chat, "t2", &me, 110, repo::CT_TEXT, Some("после видео"), &Envelope::text("после видео"), None).await;
    let ids = vec!["t2".to_string(), "v1".to_string()];
    let targets = [format!("dm:{}", hex(&bob)), format!("group:{}", group.id)];

    let results = alice.forward(&chat, &ids, &targets, false, None).await.unwrap();
    for r in &results {
        assert_eq!((r.error.as_deref(), r.sent.len()), (None, 2), "{r:?}");
        let (video, text) = (&r.sent[0], &r.sent[1]);
        assert!(!video.id.starts_with("local:"), "the message, out: {video:?}");
        assert!(video.created_at < text.created_at, "the file first, as in the chat: {} {}", video.created_at, text.created_at);
        assert_eq!(text.text.as_deref(), Some("после видео"));
        let rows = rows_of(&alice, &r.sent).await;
        let e = envelope_of(video, &rows);
        let d = MediaDescriptor::from_envelope(&e).unwrap();
        assert_ne!(d.chunks[0].sha256, format!("a{}", "0".repeat(63)), "a blob of its own");
        assert_eq!((d.kind, d.thumb.as_deref(), d.dim), (messenger_media::MediaKind::Video, Some(THUMB), Some((1280, 720))), "the preview stays");
        assert_eq!(d.name, "clip.mp4");
        assert!(e.fwd().is_some());
    }
    assert!(!backend.is_empty(), "uploaded");
    assert!(tokio::fs::metadata(&paths[0]).await.is_ok(), "the file of the message forwarded stays");
    alice.shutdown().await;
}

#[tokio::test]
async fn a_cancelled_upload_of_a_forward_leaves_the_file_of_the_message_forwarded() {
    for pause_first in [false, true] {
        let backend = MemoryBackend::new(SERVER);
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *backend.put_gate.lock().unwrap() = Some(gate.clone());
        let (_a, alice, _) = started(&backend).await;
        let (bob, carol, anna) = (Keys::generate(), Keys::generate(), Keys::generate());
        alice.contact_add(&hex(&bob), None).await.unwrap();
        alice.contact_add(&hex(&carol), None).await.unwrap();
        let (chat, paths) = private_chat_with_files(&alice, &anna, &[("f1", "plan.pdf", b"the plan")]).await;
        let source_before = repo::get(alice.store(), "f1").await.unwrap().unwrap().media_json;
        let targets = [format!("dm:{}", hex(&bob)), format!("dm:{}", hex(&carol))];
        let ids = vec!["f1".to_string()];

        let stop = async {
            let id = loop {
                let running = alice.media_transfers().await.unwrap().into_iter().find(|t| t.status == "running");
                if let Some(t) = running {
                    break t.id;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            };
            if pause_first {
                alice.media_pause(&id).await.unwrap();
            } else {
                alice.media_cancel(&id).await.unwrap();
            }
            gate.add_permits(1000);
            id
        };
        let (results, stopped) = tokio::join!(alice.forward(&chat, &ids, &targets, false, None), stop);
        let results = results.unwrap();
        assert!(results[0].error.is_some(), "the first chat did not get it: {:?}", results[0]);
        assert_eq!((results[1].error.as_deref(), results[1].sent.len()), (None, 1), "the other one did: {:?}", results[1]);
        if pause_first {
            assert_eq!(results[0].sent[0].status, "paused", "{:?}", results[0]);
            alice.media_cancel(&stopped).await.unwrap();
        }
        assert!(tokio::fs::metadata(&paths[0]).await.is_ok(), "the file of the message forwarded stays (pause first: {pause_first})");
        assert_eq!(repo::get(alice.store(), "f1").await.unwrap().unwrap().media_json, source_before);
        assert_eq!(alice.media_local_path("f1").await.unwrap(), Some(paths[0].clone()));
        alice.shutdown().await;
    }
}
