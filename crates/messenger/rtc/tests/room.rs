// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A room of a call node (services/call/spec/protocol.md, "Комнаты
// (SFU)") with sessions of this engine as its participants: each joins
// over the node's control channel with its one sendonly offer, keeps the
// room's protocol on the data channel `ctl` (answers the node's offers,
// learns whose stream an m-line carries), and hears and sees the others
// through the node, every frame encrypted under its sender's key and
// decrypted by the mid it comes on. Then simulcast: three layers to a
// node that takes them, one at the full size to a far end that does not.
//
// The node is the binary `VCALL_BIN` names; without one the tests that
// need it are skipped (CI has no node).

mod common;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use messenger_rtc::{
    test_pattern, ConnectionState, Encryption, EncryptionState, Engine, FrameKeys, RoomConfig, Session, SessionConfig,
    SessionEvent, Stats, TrackKind, TrackStats, VideoLayer,
};
use tokio::sync::{mpsc, watch};

const TONE_A: f64 = 440.0;
const TONE_B: f64 = 660.0;
const TONE_C: f64 = 880.0;
const TALK: Duration = Duration::from_secs(4);
const WAIT: Duration = Duration::from_secs(20);

/// What a participant's task tells the test.
#[derive(Debug, Clone, PartialEq, Eq)]
enum MemberEvent {
    Connected,
    Hello { you: u32, participants: Vec<u32> },
    Joined(u32),
    Left(u32),
    /// A stream of `from` arrived on `mid`.
    Track { mid: String, from: u32, kind: TrackKind },
    TrackGone { mid: String },
    Bytes { from: u32, data: Vec<u8> },
    Encryption { mid: String, state: EncryptionState },
}

/// One participant of a room in this process: a session of the engine
/// joined over the node's control channel, and the task that keeps the
/// room's protocol on `ctl` for it, as the core would: it answers the
/// node's offers, maps every m-line of them to its participant, and sets
/// the key of that participant on the m-line as soon as its track is
/// there.
struct Member {
    id: u32,
    session: Arc<Session>,
    events: mpsc::UnboundedReceiver<MemberEvent>,
    /// Whether the node's answer took the three layers of the video.
    took_simulcast: bool,
}

impl Member {
    /// Joins `room` on `node` as the seat `expect_id` (the node numbers
    /// seats from 1 in the order of joining), encrypting with
    /// `keys[expect_id]` and decrypting every other participant's stream
    /// with its key of `keys`. Back once connected and greeted.
    async fn join(engine: &Engine, node: &Node, room: &RoomCreated, expect_id: u32, keys: &HashMap<u32, Vec<u8>>, simulcast: bool) -> Member {
        let ring = FrameKeys::per_sender(b"veydan-room-test");
        assert!(ring.set_sender_key(0, &keys[&expect_id]));
        let config = SessionConfig {
            room: Some(RoomConfig { data_label: "ctl".into(), simulcast }),
            encryption: Some(Encryption { keys: ring, participant: format!("p{expect_id}") }),
            ..Default::default()
        };
        let session = Arc::new(engine.session(config).expect("room session"));
        let mut session_events = session.events().expect("events once");
        let offer = session.create_offer(false).await.expect("offer");
        assert!(offer.contains("a=sendonly"), "this side sends only:\n{offer}");
        assert!(!offer.contains("a=sendrecv") && !offer.contains("a=recvonly"), "nothing to receive yet:\n{offer}");
        assert!(offer.contains("m=application") && offer.contains("webrtc-datachannel"), "the data channel is in the offer:\n{offer}");
        assert_eq!(offer.matches("m=audio ").count(), 1);
        assert_eq!(offer.matches("m=video ").count(), 1);
        assert_eq!(offer.contains("a=simulcast:send q;h;f"), simulcast, "{offer}");

        let mut control = Control::connect(&node.reference(), None).await;
        let joined = control.join(&room.room_id, &room.join_token, &offer).await;
        assert_eq!(joined.participant_id, expect_id, "the seats are numbered in the order of joining");
        assert!(joined.sdp_answer.contains("a=ice-lite"), "the node is ICE-lite:\n{}", joined.sdp_answer);
        assert!(joined.sdp_answer.contains("a=candidate:"), "the node's candidates come in the answer");
        let took_simulcast = joined.sdp_answer.contains("a=simulcast:recv");
        session.set_remote_answer(&joined.sdp_answer).await.expect("the node's answer");

        let (tx, events) = mpsc::unbounded_channel();
        // Every m-line of the node's offers: whose stream, of what kind.
        let task_tracks: Arc<Mutex<HashMap<String, (u32, TrackKind)>>> = Arc::new(Mutex::new(HashMap::new()));
        let keys = keys.clone();
        let task_session = session.clone();
        tokio::spawn(async move {
            while let Some(ev) = session_events.recv().await {
                let out = match ev {
                    SessionEvent::ConnectionState(ConnectionState::Connected) => Some(MemberEvent::Connected),
                    SessionEvent::ConnectionState(s @ (ConnectionState::Failed | ConnectionState::Closed)) => {
                        eprintln!("p{expect_id}: {s:?}");
                        None
                    }
                    SessionEvent::Data { binary: false, data, .. } => {
                        let message: CtlMessage = serde_json::from_slice(&data).expect("a text frame of ctl is JSON");
                        match message {
                            CtlMessage::Hello { you, participants } => Some(MemberEvent::Hello { you, participants }),
                            CtlMessage::Joined { id } => Some(MemberEvent::Joined(id)),
                            CtlMessage::Left { id } => Some(MemberEvent::Left(id)),
                            CtlMessage::Offer { seq, sdp, tracks } => {
                                // Whose m-lines these are, before the tracks
                                // appear (they do while the offer is set).
                                {
                                    let mut map = task_tracks.lock().unwrap();
                                    for t in &tracks {
                                        map.insert(t.mid.clone(), (t.id, TrackKind::parse(&t.kind).expect("audio or video")));
                                    }
                                }
                                eprintln!("p{expect_id}: offer {seq} with {} tracks: {tracks:?}", tracks.len());
                                task_session.set_remote_offer(&sdp).await.expect("the node's offer");
                                let answer = task_session.create_answer().await.expect("answer");
                                assert!(!answer.contains("a=sendrecv"), "answered as the node offered:\n{answer}");
                                let json = serde_json::to_vec(&CtlMessage::Answer { seq, sdp: answer }).unwrap();
                                task_session.send_data(false, &json).expect("the answer goes on ctl");
                                None
                            }
                            CtlMessage::Answer { .. } => panic!("the node sends no answers"),
                            CtlMessage::Speaking { .. } => None,
                        }
                    }
                    SessionEvent::Data { binary: true, data, .. } => {
                        let (from, bytes) = relayed_from(&data).expect("a sender in front");
                        Some(MemberEvent::Bytes { from, data: bytes.to_vec() })
                    }
                    SessionEvent::RemoteTrack { mid, kind } => {
                        let (from, said) = task_tracks.lock().unwrap().get(&mid).copied().expect("every m-line was named in an offer");
                        assert_eq!(kind, said, "the kind of {mid} as the node said");
                        task_session.set_receiver_key(&mid, 0, &keys[&from]).expect("the key of the sender on its mid");
                        Some(MemberEvent::Track { mid, from, kind })
                    }
                    SessionEvent::RemoteTrackGone { mid } => Some(MemberEvent::TrackGone { mid }),
                    SessionEvent::RemoteEncryption { mid, state } => Some(MemberEvent::Encryption { mid, state }),
                    _ => None,
                };
                if let Some(out) = out {
                    if tx.send(out).is_err() {
                        break;
                    }
                }
            }
        });
        let mut member = Member { id: joined.participant_id, session, events, took_simulcast };
        member.wait(WAIT, |e| matches!(e, MemberEvent::Connected)).await;
        let hello = member.wait(WAIT, |e| matches!(e, MemberEvent::Hello { .. })).await;
        let MemberEvent::Hello { you, participants } = hello else { unreachable!() };
        assert_eq!(you, expect_id);
        assert_eq!(participants, joined.participants, "the hello and the join agree on who is here");
        member
    }

    /// The next event `pick` takes, within `timeout`; the others are
    /// kept aside in order.
    async fn wait(&mut self, timeout: Duration, pick: impl Fn(&MemberEvent) -> bool) -> MemberEvent {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut kept = Vec::new();
        let found = loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(left, self.events.recv()).await {
                Ok(Some(ev)) if pick(&ev) => break ev,
                Ok(Some(ev)) => kept.push(ev),
                Ok(None) => panic!("p{}: the events ended; kept {kept:?}", self.id),
                Err(_) => panic!("p{}: nothing within {timeout:?}; kept {kept:?}", self.id),
            }
        };
        // What was skipped is not lost: a fresh channel ahead of the rest.
        if !kept.is_empty() {
            let (tx, rx) = mpsc::unbounded_channel();
            for ev in kept {
                let _ = tx.send(ev);
            }
            let mut old = std::mem::replace(&mut self.events, rx);
            tokio::spawn(async move {
                while let Some(ev) = old.recv().await {
                    if tx.send(ev).is_err() {
                        break;
                    }
                }
            });
        }
        found
    }

    /// `n` tracks of `from`, as they arrive: mid by kind.
    async fn tracks_of(&mut self, from: u32, n: usize) -> HashMap<TrackKind, String> {
        let mut out = HashMap::new();
        while out.len() < n {
            let ev = self.wait(WAIT, |e| matches!(e, MemberEvent::Track { from: f, .. } if *f == from)).await;
            let MemberEvent::Track { mid, kind, .. } = ev else { unreachable!() };
            out.insert(kind, mid);
        }
        out
    }
}

/// Two participants in a room of a node on this machine hear and see
/// each other through it, encrypted; `ctl` carries their bytes; a wrong
/// key means silence, the epoch moves without a gap; a third comes and
/// goes, and the m-lines of its streams with it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn participants_meet_in_a_room_of_a_node_encrypted_end_to_end() {
    let Some(node) = Node::start().await else { return };
    let mut control = Control::connect(&node.reference(), None).await;
    let welcome = control.hello().await;
    assert!(welcome.capabilities.iter().any(|c| c == "sfu"), "{welcome:?}");
    let room = control.create_room().await;
    eprintln!("node v{} {:?}; room {} on {} / {}, {} seats, {} kbps each", welcome.version, welcome.codecs, room.room_id, room.sfu_udp, room.sfu_tcp, room.max_participants, room.kbps_per_participant);
    let engine = engine();
    // A sender key per participant, as the core would derive them from
    // the secret of the epoch and the participant's id.
    let keys: HashMap<u32, Vec<u8>> = [(1, vec![0x11; 32]), (2, vec![0x22; 32]), (3, vec![0x33; 32])].into_iter().collect();

    let mut a = Member::join(&engine, &node, &room, 1, &keys, false).await;
    let mut b = Member::join(&engine, &node, &room, 2, &keys, false).await;
    a.wait(WAIT, |e| *e == MemberEvent::Joined(2)).await;

    // The node offers each the other's streams: an audio and a video
    // m-line, named by their participant.
    let of_b_at_a = a.tracks_of(2, 2).await;
    let of_a_at_b = b.tracks_of(1, 2).await;
    eprintln!("b's streams at a: {of_b_at_a:?}; a's at b: {of_a_at_b:?}");
    let a_audio_at_b = of_a_at_b[&TrackKind::Audio].clone();
    let a_video_at_b = of_a_at_b[&TrackKind::Video].clone();
    let b_audio_at_a = of_b_at_a[&TrackKind::Audio].clone();
    assert_eq!(a.session.remote_tracks().len(), 2);
    assert_eq!(b.session.remote_tracks().len(), 2);

    // 1. A tone at each, read at the other, by the mid of its stream:
    // through the node, under each sender's own key.
    let (stop_tx, stop_rx) = watch::channel(false);
    let pump_a = pump_tone(a.session.take_audio_input().expect("input a"), TONE_A, stop_rx.clone());
    let pump_b = pump_tone(b.session.take_audio_input().expect("input b"), TONE_B, stop_rx.clone());
    let mut out_b = b.session.take_audio_output_of(&a_audio_at_b).expect("a's sound at b");
    let mut out_a = a.session.take_audio_output_of(&b_audio_at_a).expect("b's sound at a");
    assert!(b.session.take_audio_output_of(&a_audio_at_b).is_none(), "taken once");
    let (got_b, got_a) = tokio::join!(collect(&mut out_b, TALK), collect(&mut out_a, TALK));
    assert_tone(tail_tone(&got_b, TONE_A), TONE_A, "b, through the node");
    assert_tone(tail_tone(&got_a, TONE_B), TONE_B, "a, through the node");

    // 2. A's camera: the pattern reaches b on a's video m-line, at its
    // size, with its square (decrypted, so not noise).
    a.session.set_video_enabled(true);
    let (stop_video_tx, stop_video_rx) = watch::channel(false);
    let pump_video = pump_pattern(a.session.video_source(), 640, 360, stop_video_rx);
    let mut at_b = b.session.remote_video_frames_of(&a_video_at_b);
    let sizes = wait_for_size(&mut at_b, 640, 360, WAIT).await;
    eprintln!("b saw a's 640×360 through the node; sizes on the way: {sizes:?}");
    let stats = b.session.stats().await.expect("stats");
    let video_in = stats.inbound.iter().find(|t| t.mid == a_video_at_b).expect("a's video in b's stats");
    assert_eq!(video_in.kind, Some(TrackKind::Video));
    assert!(video_in.bytes > 0 && (video_in.width, video_in.height) == (640, 360), "{video_in:?}");
    assert!(stats.inbound.iter().any(|t| t.mid == a_audio_at_b && t.kind == Some(TrackKind::Audio) && t.packets > 100), "{:?}", stats.inbound);

    // 3. Bytes on ctl: the node puts the sender's id in front.
    a.session.send_data(true, b"hello from a").expect("bytes on ctl");
    let got = b.wait(WAIT, |e| matches!(e, MemberEvent::Bytes { .. })).await;
    assert_eq!(got, MemberEvent::Bytes { from: 1, data: b"hello from a".to_vec() });

    // 4. The wrong key for a's audio at b: silence, not noise; the right
    // one again: the tone, with nothing renegotiated.
    b.session.set_receiver_key(&a_audio_at_b, 0, &[0x99; 32]).expect("a key");
    let got = collect(&mut out_b, Duration::from_secs(3)).await;
    let (ratio, rms) = tail_tone(&got, TONE_A);
    assert!(rms < 100.0 && ratio < 0.5, "b with the wrong key hears {ratio:.3} / rms {rms:.0}");
    b.session.set_receiver_key(&a_audio_at_b, 0, &keys[&1]).expect("the key back");
    let got = collect(&mut out_b, Duration::from_secs(3)).await;
    assert_tone(tail_tone(&got, TONE_A), TONE_A, "b, the key back");

    // 5. The epoch moves: b has a's key of slot 1 first, then a encrypts
    // with it; the tone goes on without a gap (the slot travels in the
    // frames, the old key stays for the ones in flight).
    let next = [0xA1; 32];
    b.session.set_receiver_key(&a_audio_at_b, 1, &next).expect("the next key at b");
    a.session.set_sender_key(1, &next).expect("the next key at a");
    let got = collect(&mut out_b, Duration::from_secs(3)).await;
    assert_tone(tail_tone(&got, TONE_A), TONE_A, "b, slot 1");
    let silent = silent_frames(&got);
    eprintln!("across the epoch switch: {} frames, {silent} silent", got.len() / 480);
    assert!(silent <= 10, "{silent} silent frames of 10 ms across the switch of the epoch");

    // 6. A third participant: a and b get its streams on new m-lines of
    // the node's next offer, and hear it; it hears a once it has a's key
    // of the epoch a moved to (its task set slot 0, the epoch before).
    let mut c = Member::join(&engine, &node, &room, 3, &keys, false).await;
    a.wait(WAIT, |e| *e == MemberEvent::Joined(3)).await;
    let of_c_at_a = a.tracks_of(3, 2).await;
    let of_c_at_b = b.tracks_of(3, 2).await;
    eprintln!("c's streams at a: {of_c_at_a:?}; at b: {of_c_at_b:?}");
    assert_eq!(a.session.remote_tracks().len(), 4, "{:?}", a.session.remote_tracks());
    // A reader of c's video, as the page would subscribe once the track
    // is there; it learns the end of the track below.
    let mut c_video_at_a = a.session.remote_video_frames_of(&of_c_at_a[&TrackKind::Video]);
    let pump_c = pump_tone(c.session.take_audio_input().expect("input c"), TONE_C, stop_rx);
    let mut c_at_a = a.session.take_audio_output_of(&of_c_at_a[&TrackKind::Audio]).expect("c's sound at a");
    let (got_c, got_b_again) = tokio::join!(collect(&mut c_at_a, TALK), collect(&mut out_b, TALK));
    assert_tone(tail_tone(&got_c, TONE_C), TONE_C, "a hears c");
    assert_tone(tail_tone(&got_b_again, TONE_A), TONE_A, "b still hears a");
    let of_a_at_c = c.tracks_of(1, 2).await;
    let a_audio_at_c = of_a_at_c[&TrackKind::Audio].clone();
    let mut a_at_c = c.session.take_audio_output_of(&a_audio_at_c).expect("a's sound at c");
    let got = collect(&mut a_at_c, Duration::from_secs(2)).await;
    let (ratio, rms) = tail_tone(&got, TONE_A);
    assert!(rms < 100.0 && ratio < 0.5, "c without the key of a's epoch hears {ratio:.3} / rms {rms:.0}");
    c.session.set_receiver_key(&a_audio_at_c, 1, &next).expect("a's key of the epoch at c");
    let got = collect(&mut a_at_c, TALK).await;
    assert_tone(tail_tone(&got, TONE_A), TONE_A, "c hears a");

    // 7. c leaves by closing its connection: the node closes the m-lines
    // of its streams in its next offers, and a and b learn they are gone.
    c.session.close();
    drop(c);
    pump_c.abort();
    a.wait(WAIT, |e| *e == MemberEvent::Left(3)).await;
    for _ in 0..2 {
        let gone = a.wait(WAIT, |e| matches!(e, MemberEvent::TrackGone { .. })).await;
        let MemberEvent::TrackGone { mid } = gone else { unreachable!() };
        assert!(of_c_at_a.values().any(|m| *m == mid), "{mid} is one of c's: {of_c_at_a:?}");
    }
    assert_eq!(a.session.remote_tracks().len(), 2, "{:?}", a.session.remote_tracks());
    // The frames of c's video end for their reader.
    let ended = tokio::time::timeout(Duration::from_secs(5), async {
        while let Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) = c_video_at_a.recv().await {}
    })
    .await;
    assert!(ended.is_ok(), "the frames of c's video did not end with its m-line");
    // And so does its sound for the reader that took it: a reader that
    // mixes the room by waiting on every output per tick would otherwise
    // hang on the one who left (libwebrtc's stream ends only when
    // dropped, and the reader holds it).
    let ended = tokio::time::timeout(Duration::from_secs(5), async {
        while c_at_a.next().await.is_some() {}
    })
    .await;
    assert!(ended.is_ok(), "the sound of c did not end with its m-line");
    assert!(c_at_a.next().await.is_none(), "and stays ended");

    // What the cryptors said at b about a's streams: decrypting under
    // the right key reported Ok.
    let _ = stop_tx.send(true);
    let _ = stop_video_tx.send(true);
    let _ = tokio::join!(pump_a, pump_b, pump_video);
    let mut states = Vec::new();
    while let Ok(ev) = b.events.try_recv() {
        if let MemberEvent::Encryption { mid, state } = ev {
            states.push((mid, state));
        }
    }
    eprintln!("cryptor states at b: {states:?}");
    assert!(states.iter().any(|(m, s)| *m == a_audio_at_b && *s == EncryptionState::Ok), "{states:?}");
    assert!(states.iter().any(|(m, s)| *m == a_video_at_b && *s == EncryptionState::Ok), "{states:?}");

    a.session.close();
    b.session.close();
    node.stop().await;
}

/// The video layers of `stats` by RID (empty for a single stream).
fn video_layers(stats: Stats) -> Vec<TrackStats> {
    stats.outbound.into_iter().filter(|t| t.kind == Some(TrackKind::Video)).collect()
}

/// Reads the statistics of `session` until `done` is content with its
/// video layers, within `timeout`; the layers then.
async fn wait_layers(session: &Session, timeout: Duration, done: impl Fn(&[TrackStats]) -> bool) -> Vec<TrackStats> {
    let started = std::time::Instant::now();
    loop {
        let stats = session.stats().await.expect("stats");
        let estimate = stats.available_outgoing_kbps;
        let layers = video_layers(stats);
        if done(&layers) {
            eprintln!("layers after {:?} (estimate {estimate:.0} kbps): {layers:?}", started.elapsed());
            return layers;
        }
        assert!(started.elapsed() < timeout, "not within {timeout:?} (estimate {estimate:.0} kbps): {layers:?}");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Bytes of the layer `rid` in `layers`.
fn bytes_of(layers: &[TrackStats], rid: &str) -> u64 {
    layers.iter().find(|t| t.rid == rid).map_or(0, |t| t.bytes)
}

/// Three layers to a node: the offer names them, the node's answer takes
/// them (str0m mirrors the simulcast of an offer), three outbound-rtp
/// with their RIDs carry bytes at a quarter, a half and the full size,
/// and the top one can be turned off. A node that answered without
/// simulcast would get one stream at the full size instead.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_layers_of_video_go_to_a_node() {
    let Some(node) = Node::start().await else { return };
    let mut control = Control::connect(&node.reference(), None).await;
    let welcome = control.hello().await;
    let room = control.create_room().await;
    let engine = engine();
    let keys: HashMap<u32, Vec<u8>> = [(1, vec![0x11; 32])].into_iter().collect();
    let a = Member::join(&engine, &node, &room, 1, &keys, true).await;
    eprintln!("node v{} {:?} took the layers: {}", welcome.version, welcome.capabilities, a.took_simulcast);
    a.session.set_video_enabled(true);
    let (stop_tx, stop_rx) = watch::channel(false);
    let pump = pump_noisy_pattern(a.session.video_source(), 640, 360, stop_rx);

    if a.took_simulcast {
        // A 640×360 source is big enough for two layers in libwebrtc's
        // table (session.rs, `layers_for_width`): q at the half, h at the
        // full size, f held back; three outbound-rtp all the same.
        let encodings = a.session.video_encodings();
        assert_eq!(encodings.iter().map(|e| (e.rid.as_str(), e.active)).collect::<Vec<_>>(), vec![("q", true), ("h", true), ("f", false)], "{encodings:?}");
        let layers = wait_layers(&a.session, Duration::from_secs(40), |l| l.len() == 3 && ["q", "h"].iter().all(|rid| l.iter().any(|t| t.rid == *rid && t.bytes > 0 && t.width > 0))).await;
        let size = |rid: &str| layers.iter().find(|t| t.rid == rid).map(|t| (t.width, t.height)).unwrap();
        assert_eq!((size("q"), size("h")), ((320, 180), (640, 360)));
        assert_eq!(bytes_of(&layers, "f"), 0, "{layers:?}");
        // The upper layer off: h stops, q goes on.
        a.session.set_video_layers(VideoLayer::Low).expect("layers");
        tokio::time::sleep(Duration::from_secs(1)).await;
        let before = video_layers(a.session.stats().await.expect("stats"));
        tokio::time::sleep(Duration::from_secs(2)).await;
        let after = video_layers(a.session.stats().await.expect("stats"));
        assert_eq!(bytes_of(&after, "h"), bytes_of(&before, "h"), "h stopped");
        assert!(bytes_of(&after, "q") > bytes_of(&before, "q"), "q goes on");
        assert!(after.iter().find(|t| t.rid == "h").is_some_and(|t| !t.active));
        // Back to all: h goes again.
        a.session.set_video_layers(VideoLayer::High).expect("layers");
        let again = wait_layers(&a.session, Duration::from_secs(20), |l| bytes_of(l, "h") > bytes_of(&after, "h")).await;
        assert!(again.iter().find(|t| t.rid == "h").is_some_and(|t| t.active));
    } else {
        assert_eq!(a.session.video_encodings().len(), 1);
        let layers = wait_layers(&a.session, Duration::from_secs(40), |l| l.len() == 1 && l[0].bytes > 0 && l[0].width > 0).await;
        assert_eq!((layers[0].width, layers[0].height), (640, 360), "the one layer at the full size");
    }
    let _ = stop_tx.send(true);
    let _ = pump.await;
    a.session.close();
    node.stop().await;
}

/// `answer` without any simulcast: a far end that takes none.
fn without_simulcast(answer: &str) -> String {
    answer.split_inclusive('\n').filter(|l| !l.starts_with("a=rid:") && !l.starts_with("a=simulcast:")).collect()
}

/// `answer` with the video m-section taking the three layers (`a=rid:…
/// recv`, `a=simulcast:recv`), as an SFU answers; left as it is when it
/// takes them already.
fn with_simulcast_recv(answer: &str) -> String {
    if answer.contains("a=simulcast:") {
        return answer.to_string();
    }
    let mut out = String::new();
    let mut in_video = false;
    for line in answer.split_inclusive('\n') {
        let bare = line.trim_end();
        if bare.starts_with("m=") {
            in_video = bare.starts_with("m=video");
        }
        out.push_str(line);
        if in_video && bare.starts_with("a=mid:") {
            out.push_str("a=rid:q recv\r\na=rid:h recv\r\na=rid:f recv\r\na=simulcast:recv q;h;f\r\n");
        }
    }
    out
}

/// Without a node: the offer names the three layers; a far end whose
/// answer takes them (a peer of this process stands in for an SFU; it
/// decodes whichever layer comes) gets three outbound-rtp, two of them
/// going for a 640×360 source, and the upper one can be turned off; a
/// far end whose answer takes none gets one stream at the full size,
/// not the smallest layer libwebrtc keeps by itself.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_offer_names_three_layers_and_a_far_end_without_simulcast_gets_one_at_full_size() {
    let engine = engine();
    let room = SessionConfig { room: Some(RoomConfig { data_label: "ctl".into(), simulcast: true }), ..Default::default() };

    let a = Arc::new(engine.session(room.clone()).expect("a"));
    let b = Arc::new(engine.session(SessionConfig::default()).expect("b"));
    let mut a = wire(a.clone(), b.clone());
    let mut b = wire(b, a.session.clone());
    let offer = a.session.create_offer(false).await.expect("offer");
    for rid in ["q", "h", "f"] {
        assert!(offer.contains(&format!("a=rid:{rid} send")), "the layer {rid} is in the offer:\n{offer}");
    }
    assert!(offer.contains("a=simulcast:send q;h;f"), "{offer}");
    assert!(offer.contains("a=sendonly") && offer.contains("m=application"));
    let answer = with_simulcast_recv(&b.session.accept_offer(&offer).await.expect("answer"));
    a.session.accept_answer(&answer).await.expect("accept");
    wait_connected(&mut a, Duration::from_secs(15)).await;
    wait_connected(&mut b, Duration::from_secs(15)).await;
    // A 640×360 source: two layers go (session.rs, `layers_for_width`).
    let encodings = a.session.video_encodings();
    assert_eq!(encodings.iter().map(|e| (e.rid.as_str(), e.active)).collect::<Vec<_>>(), vec![("q", true), ("h", true), ("f", false)], "{encodings:?}");

    a.session.set_video_enabled(true);
    let (stop_tx, stop_rx) = watch::channel(false);
    let pump = pump_noisy_pattern(a.session.video_source(), 640, 360, stop_rx);
    let mut at_b = b.session.remote_video_frames();
    let (n, sizes) = watch_frames(&mut at_b, Duration::from_secs(3)).await;
    eprintln!("b got {n} frames at {sizes:?}");
    assert!(n > 30, "b got {n} frames");
    let layers = wait_layers(&a.session, Duration::from_secs(20), |l| l.len() == 3 && ["q", "h"].iter().all(|rid| l.iter().any(|t| t.rid == *rid && t.bytes > 0 && t.width > 0))).await;
    assert_eq!(layers.iter().map(|t| t.rid.as_str()).collect::<Vec<_>>(), vec!["f", "h", "q"], "three outbound-rtp, one per layer");
    assert!(layers.iter().all(|t| t.active == (t.rid != "f")), "{layers:?}");
    let size = |rid: &str| layers.iter().find(|t| t.rid == rid).map(|t| (t.width, t.height)).unwrap();
    assert_eq!((size("q"), size("h")), ((320, 180), (640, 360)));

    // The upper layers off: h stops, q goes on.
    a.session.set_video_layers(VideoLayer::Low).expect("layers");
    assert_eq!(a.session.video_layers(), VideoLayer::Low);
    let encodings = a.session.video_encodings();
    assert_eq!(encodings.iter().map(|e| (e.rid.as_str(), e.active)).collect::<Vec<_>>(), vec![("q", true), ("h", false), ("f", false)]);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let before = video_layers(a.session.stats().await.expect("stats"));
    tokio::time::sleep(Duration::from_secs(2)).await;
    let after = video_layers(a.session.stats().await.expect("stats"));
    assert!(bytes_of(&after, "q") > bytes_of(&before, "q"), "q goes on: {} -> {}", bytes_of(&before, "q"), bytes_of(&after, "q"));
    assert_eq!(bytes_of(&after, "h"), bytes_of(&before, "h"), "h stopped");
    assert_eq!(bytes_of(&after, "f"), bytes_of(&before, "f"), "f stopped");
    let _ = stop_tx.send(true);
    let _ = pump.await;
    a.session.close();
    b.session.close();

    // A far end that takes no simulcast: libwebrtc keeps the first layer,
    // the quarter; the session makes it the full size, and the far end
    // gets 640×360.
    let a2 = Arc::new(engine.session(room).expect("a2"));
    let b2 = Arc::new(engine.session(SessionConfig::default()).expect("b2"));
    let mut a2 = wire(a2.clone(), b2.clone());
    let mut b2 = wire(b2, a2.session.clone());
    let offer = a2.session.create_offer(false).await.expect("offer");
    let answer = without_simulcast(&b2.session.accept_offer(&offer).await.expect("answer"));
    assert!(!answer.contains("a=rid:"));
    a2.session.accept_answer(&answer).await.expect("accept");
    wait_connected(&mut a2, Duration::from_secs(15)).await;
    wait_connected(&mut b2, Duration::from_secs(15)).await;
    let encodings = a2.session.video_encodings();
    eprintln!("without simulcast the sender has {encodings:?}");
    assert_eq!(encodings.len(), 1, "{encodings:?}");
    a2.session.set_video_enabled(true);
    let (stop_tx, stop_rx) = watch::channel(false);
    let pump = pump_pattern(a2.session.video_source(), 640, 360, stop_rx);
    let mut at_b2 = b2.session.remote_video_frames();
    let sizes = wait_for_size(&mut at_b2, 640, 360, WAIT).await;
    eprintln!("without simulcast b2 saw 640×360; sizes on the way: {sizes:?}");
    // libwebrtc keeps an outbound-rtp per layer it was configured with;
    // one is on and carries bytes, at the full size.
    let layers = video_layers(a2.session.stats().await.expect("stats"));
    let going: Vec<_> = layers.iter().filter(|t| t.active && t.bytes > 0).collect();
    assert_eq!(going.len(), 1, "one layer goes: {layers:?}");
    assert_eq!((going[0].width, going[0].height), (640, 360), "{layers:?}");
    let _ = stop_tx.send(true);
    let _ = pump.await;
}

/// The layers of a simulcast are sized for the frames pushed, not for
/// the size the source was made for: a phone pushes whatever CameraX
/// delivers into the default 640×360 source of the session (nothing
/// replaces it on the pushed path), and a camera of a computer gives the
/// nearest size it has. Frames of 1280×720 into the default source are
/// three layers at a quarter, a half and the full size; frames of
/// 320×180 then are one layer at the full size. And the output of a
/// far end's sound taken on the pushed path ends with the session.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_layers_follow_the_frames_pushed_not_the_size_of_the_source() {
    let engine = engine();
    let room = SessionConfig { room: Some(RoomConfig { data_label: "ctl".into(), simulcast: true }), ..Default::default() };
    let a = Arc::new(engine.session(room).expect("a"));
    let b = Arc::new(engine.session(SessionConfig::default()).expect("b"));
    let mut a = wire(a.clone(), b.clone());
    let mut b = wire(b, a.session.clone());
    let offer = a.session.create_offer(false).await.expect("offer");
    let answer = with_simulcast_recv(&b.session.accept_offer(&offer).await.expect("answer"));
    a.session.accept_answer(&answer).await.expect("accept");
    wait_connected(&mut a, Duration::from_secs(15)).await;
    wait_connected(&mut b, Duration::from_secs(15)).await;
    let source = a.session.video_source();
    assert_eq!(source.resolution(), (640, 360), "the default source of a session");
    assert_eq!(source.frame_size(), None, "nothing pushed yet");
    let layout = |s: &Session| s.video_encodings().into_iter().map(|e| (e.rid, e.active, e.scale_down)).collect::<Vec<_>>();
    // Before the first frame the layers are made for the size of the
    // source: two of them.
    assert_eq!(layout(&a.session), vec![("q".into(), true, Some(2.0)), ("h".into(), true, Some(1.0)), ("f".into(), false, Some(1.0))]);

    // 1280×720 pushed into that source: three layers, from the first
    // frame on (the source tells the session before the frame goes).
    a.session.set_video_enabled(true);
    let (stop_tx, stop_rx) = watch::channel(false);
    let pump = pump_noisy_pattern(source.clone(), 1280, 720, stop_rx);
    let mut at_b = b.session.remote_video_frames();
    let (n, sizes) = watch_frames(&mut at_b, Duration::from_secs(2)).await;
    eprintln!("b got {n} frames at {sizes:?}");
    assert!(n > 0, "b got no frame");
    assert_eq!(source.frame_size(), Some((1280, 720)));
    assert_eq!(layout(&a.session), vec![("q".into(), true, Some(4.0)), ("h".into(), true, Some(2.0)), ("f".into(), true, Some(1.0))]);
    let layers = wait_layers(&a.session, Duration::from_secs(40), |l| {
        l.len() == 3 && ["q", "h", "f"].iter().all(|rid| l.iter().any(|t| t.rid == *rid && t.bytes > 0 && t.width > 0))
    })
    .await;
    let size = |l: &[TrackStats], rid: &str| l.iter().find(|t| t.rid == rid).map(|t| (t.width, t.height)).unwrap();
    assert_eq!((size(&layers, "q"), size(&layers, "h"), size(&layers, "f")), ((320, 180), (640, 360), (1280, 720)), "{layers:?}");
    let _ = stop_tx.send(true);
    let _ = pump.await;

    // 320×180 into the same source: one layer, at the full size of those
    // frames.
    let (stop_tx, stop_rx) = watch::channel(false);
    let pump = pump_noisy_pattern(source.clone(), 320, 180, stop_rx);
    let (n, sizes) = watch_frames(&mut at_b, Duration::from_secs(2)).await;
    eprintln!("b got {n} frames at {sizes:?}");
    assert_eq!(source.frame_size(), Some((320, 180)));
    assert_eq!(layout(&a.session), vec![("q".into(), true, Some(1.0)), ("h".into(), false, Some(2.0)), ("f".into(), false, Some(1.0))]);
    let before = video_layers(a.session.stats().await.expect("stats"));
    let layers = wait_layers(&a.session, Duration::from_secs(20), |l| {
        l.iter().any(|t| t.rid == "q" && (t.width, t.height) == (320, 180) && t.bytes > bytes_of(&before, "q"))
    })
    .await;
    assert!(layers.iter().all(|t| t.active == (t.rid == "q")), "{layers:?}");
    let _ = stop_tx.send(true);
    let _ = pump.await;

    // The sound of a at b, taken on the pushed path, ends when b's
    // session closes.
    let mut out_b = audio_output(&mut b).await;
    b.session.close();
    let ended = tokio::time::timeout(Duration::from_secs(5), async { while out_b.next().await.is_some() {} }).await;
    assert!(ended.is_ok(), "the output of the far end's sound did not end with the session");
    a.session.close();
}

/// The layers are set from two threads at once: the size hook on the
/// pusher's thread (a camera that opens, a phone that turns: here frames
/// of two sizes in turn, every push a change of size) and the core
/// setting the cap and the topmost layer on its own task. libwebrtc
/// takes a `SetParameters` only with the transaction id of the last
/// `GetParameters`; the session serializes the pairs, so no call of the
/// core is refused, and the sender ends with the layers for the last
/// frame pushed and the last cap and top asked for.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_size_hook_and_the_core_set_the_layers_at_once_and_none_is_refused() {
    let engine = engine();
    let room = SessionConfig { room: Some(RoomConfig { data_label: "ctl".into(), simulcast: true }), ..Default::default() };
    let a = Arc::new(engine.session(room).expect("a"));
    let b = Arc::new(engine.session(SessionConfig::default()).expect("b"));
    let mut a = wire(a.clone(), b.clone());
    let mut b = wire(b, a.session.clone());
    let offer = a.session.create_offer(false).await.expect("offer");
    let answer = with_simulcast_recv(&b.session.accept_offer(&offer).await.expect("answer"));
    a.session.accept_answer(&answer).await.expect("accept");
    wait_connected(&mut a, Duration::from_secs(15)).await;
    wait_connected(&mut b, Duration::from_secs(15)).await;
    assert_eq!(a.session.video_encodings().len(), 3, "three layers negotiated");
    a.session.set_video_enabled(true);

    // The capture's thread (a plain thread, as a camera's is, not a task
    // of the runtime): 640×360 and 320×180 in turn until told to stop.
    let source = a.session.video_source();
    let stop = Arc::new(AtomicBool::new(false));
    let pushed = Arc::new(AtomicU32::new(0));
    let pusher = std::thread::spawn({
        let (stop, pushed) = (stop.clone(), pushed.clone());
        move || {
            let mut seq = 0u32;
            while !stop.load(Ordering::Relaxed) {
                let (w, h) = if seq.is_multiple_of(2) { (640, 360) } else { (320, 180) };
                source.push(Arc::new(test_pattern(w, h, seq)));
                seq += 1;
                pushed.store(seq, Ordering::Relaxed);
            }
            seq
        }
    });
    // The core, meanwhile: the cap and the top layer, over and over,
    // with a breath between two settings (a tight loop would hold the
    // turn and starve the pusher of its hook; the core sets these a few
    // times a call, not a thousand times a second): at least 200 calls,
    // and on until the pusher got 50 frames in alongside.
    let started = std::time::Instant::now();
    let mut refused = Vec::new();
    let mut calls = 0u32;
    while calls < 200 || (pushed.load(Ordering::Relaxed) < 50 && started.elapsed() < Duration::from_secs(10)) {
        let result = if calls.is_multiple_of(2) {
            a.session.set_video_max_bitrate(Some(300 + calls))
        } else {
            a.session.set_video_layers(if calls % 4 == 1 { VideoLayer::Low } else { VideoLayer::High })
        };
        if let Err(e) = result {
            refused.push(format!("call {calls}: {e}"));
        }
        calls += 1;
        std::thread::sleep(Duration::from_micros(200));
    }
    // The last word of the core, with the pusher still at it: whichever
    // of the two goes in last reads the final state of all three.
    if let Err(e) = a.session.set_video_layers(VideoLayer::High) {
        refused.push(format!("final top: {e}"));
    }
    if let Err(e) = a.session.set_video_max_bitrate(Some(498)) {
        refused.push(format!("final cap: {e}"));
    }
    stop.store(true, Ordering::Relaxed);
    let pushed = pusher.join().expect("the pusher");
    eprintln!("{pushed} frames pushed against {calls} calls of the core in {:?}; refused: {}", started.elapsed(), refused.len());
    assert!(pushed >= 50, "the pusher ran alongside: {pushed} frames");
    assert!(refused.is_empty(), "no call of the core is refused:\n{}", refused.join("\n"));

    // The sender has the last cap (498), the top layer `High` and the
    // layers for the size of the last frame pushed.
    let (w, h) = a.session.video_source().frame_size().expect("frames were pushed");
    let going = if w * h >= 480 * 270 { 2 } else { 1 };
    let encodings = a.session.video_encodings();
    let active: Vec<_> = encodings.iter().filter(|e| e.active).map(|e| e.rid.as_str()).collect();
    assert_eq!(active, ["q", "h"][..going], "the layers for {w}×{h}: {encodings:?}");
    let ceiling = |rid: &str| match rid {
        "q" => 150,
        "h" => 500,
        _ => 1500,
    };
    assert!(
        encodings.iter().all(|e| e.max_kbps == Some(498.min(ceiling(&e.rid)))),
        "the last cap (498) on every layer, under its ceiling: {encodings:?}"
    );
    a.session.close();
    b.session.close();
}

/// What one side of a pair or a room sent and the other heard: the level
/// of the sound at the sender (its media source, as libwebrtc measures
/// the frames the device gave it), at the receiver (the inbound stream),
/// and the samples that came out on the pushed path.
struct Heard {
    level_out: f64,
    level_in: f64,
    packets_out: u64,
    packets_in: u64,
    rms: f64,
    tone: f64,
}

impl Heard {
    /// `got` came out at the receiver; `from` and `at` are the statistics
    /// of the two sides, the audio stream of the sender known at the
    /// receiver as `mid_at_receiver`.
    fn of(got: &[i16], from: &Stats, at: &Stats, mid_at_receiver: &str) -> Heard {
        let out = from.outbound.iter().find(|t| t.kind == Some(TrackKind::Audio)).cloned().unwrap_or_default();
        let inbound = at.inbound.iter().find(|t| t.mid == mid_at_receiver && t.kind == Some(TrackKind::Audio)).cloned().unwrap_or_default();
        let (tone, rms) = tail_tone(got, TONE_A);
        Heard { level_out: from.audio_level_out, level_in: inbound.audio_level, packets_out: out.packets, packets_in: inbound.packets, rms, tone }
    }

    fn describe(&self, what: &str) {
        eprintln!(
            "{what}: {} packets out, {} in; level out {:.4}, in {:.4}; rms {:.0}, {TONE_A} Hz ratio {:.3}",
            self.packets_out, self.packets_in, self.level_out, self.level_in, self.rms, self.tone
        );
    }
}

/// A participant on the device path (the microphone through the
/// platform's audio device module: how a computer and a phone join) is
/// heard by the others through the node: what the device gives in a call
/// between two it gives in a room, encrypted under its key and decrypted
/// by the mid it comes on, and it hears them through the speaker. And its
/// own picture: the frames pushed into the session's source (a camera
/// thread, a phone's plugin) come out of `local_video_frames` in a room
/// as in a call between two, for the tile of oneself.
///
/// Skipped without a device or a node. The device's recording is whatever
/// the platform gives: a microphone, or silence on a machine without one.
/// With silence the way is checked (packets both ways, the cryptor of the
/// receiver content, the level sent and the level heard the same); a
/// sound on the default recording device makes the check of the sound
/// itself bite (a tone on the far end's pushed path through the speaker
/// is not it: a machine without a loopback records nothing of its own
/// playout). On Linux a tone goes onto the default source with the Pulse
/// server: `pactl load-module module-null-sink sink_name=t`, `pactl
/// set-default-source t.monitor`, `paplay --device=t tone.wav` (what the
/// WSLg machine of the owner did through a small libpulse client, 2026-10-08:
/// the pushed member heard a 440 Hz tone of the device path through the
/// node at a ratio of 1.000).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_participant_on_the_device_path_is_heard_in_the_room_and_sees_itself() {
    let device = match Engine::new(messenger_rtc::AudioMode::Device) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("no audio device on this machine ({e}); the test is skipped");
            return;
        }
    };
    let Some(node) = Node::start().await else { return };
    eprintln!("devices: {:?}", device.audio_devices());
    let pushed = engine();

    // The reference: a call between two, the device on one side and the
    // pushed path on the other, which hears what the device gave.
    let reference = {
        let a = Arc::new(device.session(SessionConfig::default()).expect("a"));
        let b = Arc::new(pushed.session(SessionConfig::default()).expect("b"));
        let mut a = wire(a.clone(), b.clone());
        let mut b = wire(b, a.session.clone());
        negotiate(&a, &b, false).await;
        wait_connected(&mut a, Duration::from_secs(15)).await;
        wait_connected(&mut b, Duration::from_secs(15)).await;
        let mut out = audio_output(&mut b).await;
        let got = collect(&mut out, TALK).await;
        let (from, at) = (a.session.stats().await.expect("stats"), b.session.stats().await.expect("stats"));
        let heard = Heard::of(&got, &from, &at, "0");
        heard.describe("a call between two, the device at a");
        assert!(heard.packets_out > 100 && heard.packets_in > 100, "the sound of the device goes in a call between two");
        a.session.close();
        b.session.close();
        tokio::time::sleep(Duration::from_millis(500)).await;
        heard
    };

    // The room: the device at a, the pushed path at b.
    let mut control = Control::connect(&node.reference(), None).await;
    let _ = control.hello().await;
    let room = control.create_room().await;
    let keys: HashMap<u32, Vec<u8>> = [(1, vec![0x11; 32]), (2, vec![0x22; 32])].into_iter().collect();
    let mut a = Member::join(&device, &node, &room, 1, &keys, false).await;
    let mut b = Member::join(&pushed, &node, &room, 2, &keys, false).await;
    a.wait(WAIT, |e| *e == MemberEvent::Joined(2)).await;
    let of_a_at_b = b.tracks_of(1, 2).await;
    let of_b_at_a = a.tracks_of(2, 2).await;
    let a_audio_at_b = of_a_at_b[&TrackKind::Audio].clone();
    assert!(a.session.take_audio_input().is_none(), "the device path has no input to push into");
    assert!(a.session.take_audio_output_of(&of_b_at_a[&TrackKind::Audio]).is_none(), "the device path plays every remote track through the speaker: no output to take");
    assert!(!a.session.muted());
    let (stop_tx, stop_rx) = watch::channel(false);
    let pump_b = pump_tone(b.session.take_audio_input().expect("input b"), TONE_B, stop_rx);
    let mut out_b = b.session.take_audio_output_of(&a_audio_at_b).expect("a's sound at b");
    let got = collect(&mut out_b, TALK).await;
    let (from, at) = (a.session.stats().await.expect("stats"), b.session.stats().await.expect("stats"));
    let heard = Heard::of(&got, &from, &at, &a_audio_at_b);
    heard.describe("the room, the device at a");
    let states: Vec<(String, EncryptionState)> = std::iter::from_fn(|| b.events.try_recv().ok())
        .filter_map(|e| match e {
            MemberEvent::Encryption { mid, state } => Some((mid, state)),
            _ => None,
        })
        .collect();
    eprintln!("cryptor states at b: {states:?}");

    // The way: the device's audio goes out encrypted, b's cryptor reads it
    // under a's key, and b hears the level a sent.
    assert!(heard.packets_out > 100, "a sends its audio into the room: {heard:?}", heard = heard.packets_out);
    assert!(heard.packets_in > 100, "b gets a's audio through the node: {}", heard.packets_in);
    assert!(states.iter().any(|(m, s)| *m == a_audio_at_b && *s == EncryptionState::Ok), "b decrypts a's audio: {states:?}");
    assert!(!states.iter().any(|(m, s)| *m == a_audio_at_b && matches!(s, EncryptionState::DecryptionFailed | EncryptionState::MissingKey)), "{states:?}");
    assert!((heard.level_out - heard.level_in).abs() <= 0.01 + heard.level_out * 0.5, "b hears the level a sent: {:.4} sent, {:.4} heard", heard.level_out, heard.level_in);
    // And b's tone reaches a, which plays it through its speaker (the
    // level of the inbound stream is libwebrtc's, before the device).
    let b_at_a = from.inbound.iter().find(|t| t.mid == of_b_at_a[&TrackKind::Audio]).expect("b's audio in a's stats");
    assert!(b_at_a.packets > 100 && b_at_a.audio_level > 0.05, "a decodes b's tone for its speaker: {b_at_a:?}");
    // The sound: what the device gave in a call between two it gives in
    // the room. Silence from the device is silence both times, and says
    // nothing more.
    if reference.level_out > 0.001 {
        assert!(heard.level_out > 0.001 && heard.rms > 50.0, "the device's sound reaches the room as it reached a call between two: {:.4} / rms {:.0}", heard.level_out, heard.rms);
        if reference.tone > 0.9 {
            assert!(heard.tone > 0.9, "the tone of the device, {TONE_A} Hz: ratio {:.3} in the room, {:.3} between two", heard.tone, reference.tone);
        }
    } else {
        eprintln!("the recording device gives silence here (level {:.4} between two): the way was checked, the sound itself was not", reference.level_out);
    }

    // My own picture in the room: the frames pushed into the source of
    // the session (what a camera thread or a phone's plugin does) come
    // out of local_video_frames, and reach b through the node.
    let mut mine = a.session.local_video_frames();
    a.session.set_video_enabled(true);
    let (stop_video_tx, stop_video_rx) = watch::channel(false);
    let pump_video = pump_pattern(a.session.video_source(), 640, 360, stop_video_rx);
    let sizes = wait_for_size(&mut mine, 640, 360, Duration::from_secs(5)).await;
    assert!(sizes.len() <= 1, "my own frames come as pushed: {sizes:?}");
    let mut at_b = b.session.remote_video_frames_of(&of_a_at_b[&TrackKind::Video]);
    wait_for_size(&mut at_b, 640, 360, WAIT).await;

    let _ = stop_tx.send(true);
    let _ = stop_video_tx.send(true);
    let _ = tokio::join!(pump_b, pump_video);
    a.session.close();
    b.session.close();
    node.stop().await;
}
