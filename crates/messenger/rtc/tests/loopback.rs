// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Two sessions in one process, joined through SDP text and trickled
// candidates as a call over the relays would join them: a tone pushed in
// at one end is read out at the other. Directly, after a restart of ICE,
// with the frames encrypted, and through a call node on this machine
// (relay only, over UDP and over TLS). A test pattern pushed as video
// at one end comes out at the other at its size, and stops when the
// video goes off, with no offer in between.

mod common;

use std::time::Duration;

use common::*;
use messenger_rtc::{
    has_test_square, CandidateKind, ConnectionState, Encryption, EncryptionState, FrameKeys, IcePolicy, SessionConfig,
    SessionEvent,
};
use tokio::sync::{broadcast, watch};

const TONE_A: f64 = 440.0;
const TONE_B: f64 = 660.0;
const TALK: Duration = Duration::from_secs(4);

/// A tone at each end, read at the other: the ratio of the tone (1.0 is
/// pure) and the RMS, as pushed (8000 of amplitude is 5657 of RMS).
async fn exchange(a: &mut Side, b: &mut Side) -> ((f64, f64), (f64, f64)) {
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let in_a = a.session.take_audio_input().expect("input a");
    let in_b = b.session.take_audio_input().expect("input b");
    let pump_a = pump_tone(in_a, TONE_A, stop_rx.clone());
    let pump_b = pump_tone(in_b, TONE_B, stop_rx);
    let mut out_b = audio_output(b).await;
    let mut out_a = audio_output(a).await;
    let (got_b, got_a) = tokio::join!(collect(&mut out_b, TALK), collect(&mut out_a, TALK));
    let _ = stop_tx.send(true);
    let _ = tokio::join!(pump_a, pump_b);
    (tail_tone(&got_b, TONE_A), tail_tone(&got_a, TONE_B))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tone_crosses_a_direct_pair_both_ways() {
    let engine = engine();
    let (mut a, mut b) = pair(&engine, SessionConfig::default());
    negotiate(&a, &b, false).await;
    let took = wait_connected(&mut a, Duration::from_secs(15)).await;
    wait_connected(&mut b, Duration::from_secs(15)).await;
    eprintln!("connected after {took:?}");

    let (at_b, at_a) = exchange(&mut a, &mut b).await;
    assert_tone(at_b, TONE_A, "b");
    assert_tone(at_a, TONE_B, "a");

    let stats = a.session.stats().await.expect("stats");
    let path = stats.path.clone().expect("a path once connected");
    eprintln!("path {path:?}, stats {stats:?}");
    assert!(!path.is_relayed());
    assert_eq!(path.local, CandidateKind::Host);
    assert!(stats.bytes_sent > 0 && stats.bytes_received > 0);
    assert!(stats.packets_received > 100, "{}", stats.packets_received);
    assert!(stats.audio_level_in > 0.0 && stats.audio_level_out > 0.0, "{stats:?}");

    // Mute: the track is disabled, nothing renegotiated.
    a.session.set_muted(true);
    assert!(a.session.muted());
    a.session.set_muted(false);
    assert!(!a.session.muted());
}

/// The callee's `call.ice` may be delivered before its `call.answer`
/// (different relays, a push wake-up): the caller's session takes the
/// candidates, keeps them, and adds them with the answer. libwebrtc alone
/// would refuse them ("no remote description") and the caller would never
/// check the callee's addresses.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn candidates_that_come_before_the_answer_are_kept_and_go_in_with_it() {
    let engine = engine();
    let a = std::sync::Arc::new(engine.session(SessionConfig::default()).expect("a"));
    let b = std::sync::Arc::new(engine.session(SessionConfig::default()).expect("b"));
    let mut events_a = a.events().expect("events of a");
    let mut events_b = b.events().expect("events of b");

    let offer = a.create_offer(false).await.expect("offer");
    let answer = b.accept_offer(&offer).await.expect("answer");

    // The candidates of b, before a has seen the answer: the first within
    // 10 s, then whatever follows within a moment (the session gathers
    // continually, so no end of gathering is announced).
    let mut of_b = Vec::new();
    let first = async {
        while let Some(ev) = events_b.recv().await {
            if let SessionEvent::LocalCandidate(c) = ev {
                of_b.push(c);
                break;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(10), first).await.expect("b has a candidate within 10 s");
    while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(500), events_b.recv()).await {
        if let SessionEvent::LocalCandidate(c) = ev {
            of_b.push(c);
        }
    }
    assert!(!of_b.is_empty(), "b has candidates");
    for c in &of_b {
        a.add_remote_candidate(c).await.expect("a candidate before the answer is kept, not refused");
    }
    assert_eq!(a.pending_remote_candidates(), of_b.len(), "all of them wait for the answer");

    a.accept_answer(&answer).await.expect("accept answer");
    assert_eq!(a.pending_remote_candidates(), 0, "the answer took them in");

    // From here on as over the wire: a's candidates straight into b (b has
    // the offer already), and both sides connect.
    let (mut state_a, mut state_b) = (ConnectionState::New, ConnectionState::New);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while state_a != ConnectionState::Connected || state_b != ConnectionState::Connected {
        tokio::select! {
            ev = events_a.recv() => match ev.expect("events of a go on") {
                SessionEvent::LocalCandidate(c) => {
                    b.add_remote_candidate(&c).await.expect("b has the offer: straight in");
                    assert_eq!(b.pending_remote_candidates(), 0);
                }
                SessionEvent::ConnectionState(s) => state_a = s,
                _ => {}
            },
            ev = events_b.recv() => {
                if let SessionEvent::ConnectionState(s) = ev.expect("events of b go on") {
                    state_b = s;
                }
            }
            _ = tokio::time::sleep_until(deadline) => panic!("not connected: a {state_a:?}, b {state_b:?}"),
        }
    }
    let path = a.path().await.expect("stats").expect("a path once connected");
    eprintln!("connected over {path:?}");
    assert!(!path.is_relayed());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ice_restarts_with_new_credentials_and_the_sound_goes_on() {
    let engine = engine();
    let (mut a, mut b) = pair(&engine, SessionConfig::default());
    let offer = a.session.create_offer(false).await.expect("offer");
    let answer = b.session.accept_offer(&offer).await.expect("answer");
    a.session.accept_answer(&answer).await.expect("accept");
    wait_connected(&mut a, Duration::from_secs(15)).await;
    wait_connected(&mut b, Duration::from_secs(15)).await;
    // The candidates of the first gathering, out of the way.
    let first = candidate_lines(&mut a);
    assert!(!first.is_empty());

    let ufrag = |sdp: &str| sdp.lines().find(|l| l.starts_with("a=ice-ufrag:")).map(str::to_owned);
    let restart = a.session.create_offer(true).await.expect("offer with restart");
    assert_ne!(ufrag(&offer), ufrag(&restart), "a restart gives a new ufrag");
    let answer = b.session.accept_offer(&restart).await.expect("answer");
    a.session.accept_answer(&answer).await.expect("accept");

    // New candidates come for the new credentials, and the pair is
    // connected again (it may never have shown as anything else: a
    // restart keeps the media flowing).
    let gathered = async {
        loop {
            match a.events.recv().await.expect("events") {
                SessionEvent::LocalCandidate(_) => return,
                _ => continue,
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(10), gathered).await.expect("candidates after the restart");
    wait_connected(&mut a, Duration::from_secs(15)).await;
    wait_connected(&mut b, Duration::from_secs(15)).await;

    let (at_b, at_a) = exchange(&mut a, &mut b).await;
    assert_tone(at_b, TONE_A, "b after the restart");
    assert_tone(at_a, TONE_B, "a after the restart");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn frames_are_encrypted_end_to_end_under_a_shared_key() {
    const K1: [u8; 32] = [0x11; 32];
    const K2: [u8; 32] = [0x22; 32];
    let engine = engine();
    // A ring on each side, as two devices would hold them.
    let keys_a = FrameKeys::new(b"veydan-test-call");
    let keys_b = FrameKeys::new(b"veydan-test-call");
    assert!(keys_a.set_key(0, &K1));
    assert!(keys_b.set_key(0, &K1));
    let with = |keys: &FrameKeys, who: &str| SessionConfig {
        encryption: Some(Encryption { keys: keys.clone(), participant: who.into() }),
        ..Default::default()
    };
    let a = std::sync::Arc::new(engine.session(with(&keys_a, "a")).expect("a"));
    let b = std::sync::Arc::new(engine.session(with(&keys_b, "b")).expect("b"));
    assert!(a.encryption_enabled());
    let mut a = wire(a.clone(), b.clone());
    let mut b = wire(b, a.session.clone());
    negotiate(&a, &b, false).await;
    wait_connected(&mut a, Duration::from_secs(15)).await;
    wait_connected(&mut b, Duration::from_secs(15)).await;

    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let pump_a = pump_tone(a.session.take_audio_input().expect("input"), TONE_A, stop_rx.clone());
    let pump_b = pump_tone(b.session.take_audio_input().expect("input"), TONE_B, stop_rx);
    let mut out_b = audio_output(&mut b).await;
    let mut out_a = audio_output(&mut a).await;

    // 1. The same key: the tones cross.
    let (got_b, got_a) = tokio::join!(collect(&mut out_b, TALK), collect(&mut out_a, TALK));
    assert_tone(tail_tone(&got_b, TONE_A), TONE_A, "b, same key");
    assert_tone(tail_tone(&got_a, TONE_B), TONE_B, "a, same key");

    // 2. Another key at b: b decrypts nothing of a's, and a nothing of b's
    // (b encrypts with the new key too): silence both ways, not noise.
    assert!(keys_b.set_key(0, &K2));
    let (got_b, got_a) = tokio::join!(collect(&mut out_b, Duration::from_secs(3)), collect(&mut out_a, Duration::from_secs(3)));
    let (ratio_b, rms_b) = tail_tone(&got_b, TONE_A);
    let (ratio_a, rms_a) = tail_tone(&got_a, TONE_B);
    assert!(rms_b < 100.0 && ratio_b < 0.5, "b with the wrong key hears {ratio_b:.3} / rms {rms_b:.0}");
    assert!(rms_a < 100.0 && ratio_a < 0.5, "a with b on the wrong key hears {ratio_a:.3} / rms {rms_a:.0}");

    // 3. The key back: the tones again, without a renegotiation.
    assert!(keys_b.set_key(0, &K1));
    let (got_b, got_a) = tokio::join!(collect(&mut out_b, Duration::from_secs(3)), collect(&mut out_a, Duration::from_secs(3)));
    assert_tone(tail_tone(&got_b, TONE_A), TONE_A, "b, key back");
    assert_tone(tail_tone(&got_a, TONE_B), TONE_B, "a, key back");

    // 4. A second slot on both, and a moves to it: the index travels in
    // the frame, b reads it.
    const K3: [u8; 32] = [0x33; 32];
    assert!(keys_a.set_key(1, &K3));
    assert!(keys_b.set_key(1, &K3));
    a.session.set_key_index(1);
    let got_b = collect(&mut out_b, Duration::from_secs(3)).await;
    assert_tone(tail_tone(&got_b, TONE_A), TONE_A, "b, slot 1");

    let _ = stop_tx.send(true);
    let _ = tokio::join!(pump_a, pump_b);

    // What the cryptors reported.
    let mut states_a = Vec::new();
    while let Ok(ev) = a.events.try_recv() {
        if let SessionEvent::Encryption { participant, state } = ev {
            states_a.push((participant, state));
        }
    }
    let mut states_b = Vec::new();
    while let Ok(ev) = b.events.try_recv() {
        if let SessionEvent::Encryption { participant, state } = ev {
            states_b.push((participant, state));
        }
    }
    eprintln!("cryptor states at a: {states_a:?}\ncryptor states at b: {states_b:?}");
    // Both the sender and the receiver of each side ran: a failure to
    // decrypt under the wrong key is not reported by the cryptor (the
    // silence above is the evidence of it), the recovery is `Ok` again.
    assert!(states_a.iter().any(|(p, s)| p == "a" && *s == EncryptionState::Ok));
    assert!(states_a.iter().any(|(p, s)| p == "peer" && *s == EncryptionState::Ok));
    assert!(states_b.iter().any(|(p, s)| p == "b" && *s == EncryptionState::Ok));
    assert!(states_b.iter().any(|(p, s)| p == "peer" && *s == EncryptionState::Ok));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relay_only_goes_through_a_node_over_udp_and_over_tls() {
    let Some(node) = Node::start().await else { return };
    let (welcome, creds) = credentials(&node.reference(), None).await;
    eprintln!("node {} v{} {:?}; credentials {} in {} for {} s: {:?}", welcome.node_id, welcome.version, welcome.capabilities, creds.username, creds.realm, creds.ttl_secs, creds.urls);
    let engine = engine();

    for (name, server) in [("udp", creds.ice_server("turn", Some("udp"))), ("tls", creds.ice_server("turns", Some("tcp")))] {
        let config = SessionConfig { ice_servers: vec![server], policy: IcePolicy::RelayOnly, ..Default::default() };
        let (mut a, mut b) = pair(&engine, config);
        negotiate(&a, &b, false).await;
        let took = wait_connected(&mut a, Duration::from_secs(30)).await;
        wait_connected(&mut b, Duration::from_secs(30)).await;
        let path = a.session.path().await.expect("stats").expect("a path");
        eprintln!("{name}: connected after {took:?}, path {path:?}");
        assert!(path.is_relayed(), "{name}: {path:?}");
        assert_eq!(path.local, CandidateKind::Relay, "{name}: relay only means a relay at this end");
        assert_eq!(path.remote, CandidateKind::Relay, "{name}: and at the other");
        // Every candidate of a relay-only session is a relay candidate.
        for c in candidate_lines(&mut a) {
            assert!(c.candidate.contains(" typ relay "), "{name}: {}", c.candidate);
        }
        let (at_b, at_a) = exchange(&mut a, &mut b).await;
        assert_tone(at_b, TONE_A, &format!("b over {name}"));
        assert_tone(at_a, TONE_B, &format!("a over {name}"));
        a.session.close();
        b.session.close();
    }
    node.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_test_pattern_crosses_at_its_size_and_stops_with_the_video() {
    let engine = engine();
    let (mut a, mut b) = pair(&engine, SessionConfig::default());
    negotiate(&a, &b, false).await;
    wait_connected(&mut a, Duration::from_secs(15)).await;
    wait_connected(&mut b, Duration::from_secs(15)).await;
    // The video m-line is in the first offer: the far end's track is there
    // before any frame, on both sides.
    let saw_remote_video = |side: &mut Side| {
        let mut saw = false;
        while let Ok(ev) = side.events.try_recv() {
            saw |= matches!(ev, SessionEvent::RemoteVideo);
        }
        saw
    };
    assert!(saw_remote_video(&mut b), "b has a's video track from the offer");
    assert!(saw_remote_video(&mut a), "a has b's from the answer");
    assert!(!a.session.video_enabled() && !b.session.video_enabled());

    let mut at_b = b.session.remote_video_frames();
    let mut own_a = a.session.local_video_frames();
    let (n, _) = watch_frames(&mut at_b, Duration::from_millis(500)).await;
    assert_eq!(n, 0, "nothing comes while the video is off");

    // The camera on at a: the pattern at 640×360 into the source of the
    // session, the track enabled, the cap of 360p.
    a.session.set_video_enabled(true);
    a.session.set_video_max_bitrate(Some(800)).expect("cap");
    let (stop_tx, stop_rx) = watch::channel(false);
    let pump = pump_pattern(a.session.video_source(), 640, 360, stop_rx.clone());
    // What a pushes it sees itself at once, as pushed.
    let own = tokio::time::timeout(Duration::from_secs(5), own_a.recv()).await.expect("own frame in time").expect("own frame");
    assert_eq!((own.width, own.height), (640, 360));
    assert!(has_test_square(&own));
    // The far end decodes it: the encoder may start smaller while the
    // estimate of the way climbs, and reaches the full size soon.
    let started = std::time::Instant::now();
    let sizes = wait_for_size(&mut at_b, 640, 360, Duration::from_secs(20)).await;
    eprintln!("b saw 640×360 with the square after {:?}; sizes on the way: {sizes:?}", started.elapsed());
    // At the full size the rate is the source's.
    let (n, sizes) = watch_frames(&mut at_b, Duration::from_secs(2)).await;
    eprintln!("b: {n} frames in 2 s at {sizes:?}");
    assert!(n >= 40, "b got {n} frames in 2 s (30 a second pushed)");
    assert_eq!(sizes, vec![(640, 360)]);
    let stats = a.session.stats().await.expect("stats");
    assert!(stats.bytes_sent > 100_000, "video was sent: {stats:?}");

    // Off, as the adapter does it: the camera stops pushing and the track
    // is disabled; frames stop at b within a moment, and no offer was
    // asked for (checked at the end).
    let _ = stop_tx.send(true);
    let pushed = pump.await.expect("pump");
    assert!(pushed > 60, "pushed {pushed}");
    a.session.set_video_enabled(false);
    assert!(!a.session.video_enabled());
    let _ = watch_frames(&mut at_b, Duration::from_millis(700)).await;
    let (n, _) = watch_frames(&mut at_b, Duration::from_secs(1)).await;
    assert_eq!(n, 0, "frames after the video went off");

    // A screen instead of the camera: a new source of another size on
    // the same sender, no renegotiation; the frames come at that size.
    let screen = a.session.replace_video_source(320, 180, true).expect("new source");
    assert!(screen.is_screencast());
    a.session.set_video_enabled(true);
    let (stop_tx, stop_rx) = watch::channel(false);
    let pump = pump_pattern(screen, 320, 180, stop_rx);
    let sizes = wait_for_size(&mut at_b, 320, 180, Duration::from_secs(20)).await;
    eprintln!("b saw 320×180 after the switch; sizes on the way: {sizes:?}");
    let _ = stop_tx.send(true);
    let _ = pump.await;
    while let Ok(ev) = a.events.try_recv() {
        assert!(!matches!(ev, SessionEvent::NegotiationNeeded), "no offer was asked for");
    }

    // The call ends at b: the far end's frames end with its session, so
    // that whoever reads them (the page's subscription) learns it is
    // over, instead of waiting on a track that is never closed.
    b.session.close();
    let ended = tokio::time::timeout(Duration::from_secs(5), async {
        while let Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) = at_b.recv().await {}
    })
    .await;
    assert!(ended.is_ok(), "the far end's frames did not end with the session");
}

/// Two calls in a row through one engine, as a phone or a computer makes
/// them. libwebrtc runs its media engine from the first PeerConnection to
/// the last: the sessions of the first pair are closed and gone, so the
/// voice engine is terminated, with the audio device under it; the second
/// pair must bring it back and carry a tone both ways again (on this path
/// libwebrtc pulls the sound by itself while no device plays, so the pair
/// spoke even before patch 4 of vendor/webrtc-sys; the test below sees the
/// device). What the sessions do twice is the same as on a phone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_pair_of_the_same_engine_carries_the_sound_again() {
    let engine = engine();
    for round in 1..=2 {
        let (mut a, mut b) = pair(&engine, SessionConfig::default());
        negotiate(&a, &b, false).await;
        wait_connected(&mut a, Duration::from_secs(15)).await;
        wait_connected(&mut b, Duration::from_secs(15)).await;
        let (at_b, at_a) = exchange(&mut a, &mut b).await;
        assert_tone(at_b, TONE_A, &format!("b, round {round}"));
        assert_tone(at_a, TONE_B, &format!("a, round {round}"));
        // Closed as a call ends: the last PeerConnection of the engine
        // goes, and the media engine with it, before the next pair.
        a.session.close();
        b.session.close();
        drop(a);
        drop(b);
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// The same on the device path, where the bug lived: a machine with a
/// sound device (a sound card, or the Pulse server of WSLg); skipped where
/// the platform's device cannot be taken. The device answers its lists
/// only while it is initialized, and libwebrtc terminates it with the
/// media engine after the last PeerConnection; the first PeerConnection
/// of the next pair initializes the media engine again, and the device
/// with it (patch 4 of vendor/webrtc-sys): the lists are back. Without
/// the patch the device stayed terminated: empty lists here, and on
/// Android a dead process.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_device_of_the_engine_is_back_for_a_second_pair() {
    let engine = match messenger_rtc::Engine::new(messenger_rtc::AudioMode::Device) {
        Ok(engine) => engine,
        Err(e) => {
            eprintln!("no audio device on this machine ({e}); skipped");
            return;
        }
    };
    let before = engine.audio_devices();
    eprintln!("devices: {before:?}");
    assert!(!before.playout.is_empty() || !before.recording.is_empty(), "a device to speak of");
    for round in 1..=2 {
        let (mut a, mut b) = pair(&engine, SessionConfig::default());
        let now = engine.audio_devices();
        assert_eq!(
            (now.playout.len(), now.recording.len()),
            (before.playout.len(), before.recording.len()),
            "round {round}: the device is initialized with the first session: {now:?}"
        );
        negotiate(&a, &b, false).await;
        wait_connected(&mut a, Duration::from_secs(15)).await;
        wait_connected(&mut b, Duration::from_secs(15)).await;
        a.session.close();
        b.session.close();
        drop(a);
        drop(b);
        tokio::time::sleep(Duration::from_millis(500)).await;
        // The last PeerConnection went: libwebrtc terminated its media
        // engine and the device with it, which answers no list now. This
        // is what the next round must come back from.
        let gone = engine.audio_devices();
        assert!(
            gone.playout.is_empty() && gone.recording.is_empty(),
            "round {round}: the device is terminated after the last session: {gone:?}"
        );
    }
}
