// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// One live run through a call node of the project: both sessions in this
// process, relay only, so that the sound goes out to the node and back,
// over UDP and over TLS. Ignored by default: it needs the internet and
// the node. Run it by hand, once per change of the engine:
//
//   cargo test -p messenger-rtc --test live -- --ignored --nocapture
//
// VEYDAN_RTC_NODE names the node (`address:port#id`); the default is the
// node on eu-1 (the plan of calls). VEYDAN_RTC_ACCESS_KEY for a
// private one.

mod common;

use std::time::Duration;

use common::*;
use messenger_rtc::{CandidateKind, IcePolicy, SessionConfig};

const DEFAULT_NODE: &str = "108.61.171.68:8443#fda09da75199c4e04601a9df309710fab15cd7b2b806ca3b2bbab202582a6dca";
const TONE_A: f64 = 440.0;
const TONE_B: f64 = 660.0;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the internet and a call node; run by hand"]
async fn relay_only_through_the_project_node() {
    let reference = std::env::var("VEYDAN_RTC_NODE").unwrap_or_else(|_| DEFAULT_NODE.to_string());
    let access = std::env::var("VEYDAN_RTC_ACCESS_KEY").ok();
    let (welcome, creds) = credentials(&reference, access).await;
    eprintln!(
        "node {} v{} {:?}; credentials {} in {} for {} s: {:?}",
        welcome.node_id, welcome.version, welcome.capabilities, creds.username, creds.realm, creds.ttl_secs, creds.urls
    );
    let engine = engine();

    for (name, server) in [("udp", creds.ice_server("turn", Some("udp"))), ("tls", creds.ice_server("turns", Some("tcp")))] {
        let config = SessionConfig { ice_servers: vec![server], policy: IcePolicy::RelayOnly, ..Default::default() };
        let (mut a, mut b) = pair(&engine, config);
        negotiate(&a, &b, false).await;
        let took = wait_connected(&mut a, Duration::from_secs(40)).await;
        wait_connected(&mut b, Duration::from_secs(40)).await;

        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let pump_a = pump_tone(a.session.take_audio_input().expect("input"), TONE_A, stop_rx.clone());
        let pump_b = pump_tone(b.session.take_audio_input().expect("input"), TONE_B, stop_rx);
        let mut out_b = audio_output(&mut b).await;
        let mut out_a = audio_output(&mut a).await;
        let (got_b, got_a) = tokio::join!(collect(&mut out_b, Duration::from_secs(6)), collect(&mut out_a, Duration::from_secs(6)));
        let _ = stop_tx.send(true);
        let _ = tokio::join!(pump_a, pump_b);

        let stats = a.session.stats().await.expect("stats");
        let path = stats.path.clone().expect("a path");
        let (ratio_b, rms_b) = tail_tone(&got_b, TONE_A);
        let (ratio_a, rms_a) = tail_tone(&got_a, TONE_B);
        eprintln!(
            "{name}: connected after {took:?}; path {} {} <-> {} {} ({}), rtt {:?} ms; sent {} B, received {} B, lost {}, jitter {:.1} ms; tone at b {ratio_b:.3} rms {rms_b:.0}, at a {ratio_a:.3} rms {rms_a:.0}",
            match path.local { CandidateKind::Relay => "relay", _ => "other" },
            path.local_addr,
            match path.remote { CandidateKind::Relay => "relay", _ => "other" },
            path.remote_addr,
            path.protocol,
            stats.rtt_ms,
            stats.bytes_sent,
            stats.bytes_received,
            stats.packets_lost,
            stats.jitter_ms
        );
        assert!(path.is_relayed(), "{name}: {path:?}");
        assert_eq!(path.local, CandidateKind::Relay);
        assert_eq!(path.remote, CandidateKind::Relay);
        assert!(ratio_b > 0.9 && rms_b > 3000.0, "{name}: {TONE_A} Hz at b: {ratio_b:.3} / {rms_b:.0}");
        assert!(ratio_a > 0.9 && rms_a > 3000.0, "{name}: {TONE_B} Hz at a: {ratio_a:.3} / {rms_a:.0}");
        a.session.close();
        b.session.close();
    }
}
