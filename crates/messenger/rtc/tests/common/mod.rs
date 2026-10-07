// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What the tests of the engine share: a tone and its detector, two
//! sessions joined in this process with their candidates trickled across,
//! a call node started from `services/call/dist/vcall`, and a client of
//! its control channel for the credentials (the same HELLO/TURN over
//! pinned TLS and h2 that `vcall probe` makes; copied, not depended on:
//! the node's crates stay in their own workspace).

#![allow(dead_code)]

use std::net::{IpAddr, SocketAddr};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use http::{Method, Request};
use messenger_rtc::{
    AudioInput, AudioMode, AudioOutput, AudioProcessing, Candidate, ConnectionState, Engine, IceServer, Session,
    SessionConfig, SessionEvent, FRAME_SAMPLES, SAMPLE_RATE,
};
use messenger_vlink::proto::{io as h2io, pin, BridgeRef};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{mpsc, watch};
use tokio_rustls::TlsConnector;

/// A 10 ms mono frame of a sine at `hz`; `phase` carries over.
pub fn tone_frame(hz: f64, amplitude: f64, phase: &mut f64) -> Vec<i16> {
    let step = 2.0 * std::f64::consts::PI * hz / SAMPLE_RATE as f64;
    (0..FRAME_SAMPLES)
        .map(|_| {
            let s = (phase.sin() * amplitude) as i16;
            *phase += step;
            if *phase > 2.0 * std::f64::consts::PI {
                *phase -= 2.0 * std::f64::consts::PI;
            }
            s
        })
        .collect()
}

/// Goertzel power of `hz` in `samples` against the total power: 1.0 a
/// pure tone, 0.0 silence or noise. Also the RMS in i16 units.
pub fn tone_ratio(samples: &[i16], hz: f64) -> (f64, f64) {
    if samples.is_empty() {
        return (0.0, 0.0);
    }
    let n = samples.len() as f64;
    let k = (0.5 + n * hz / SAMPLE_RATE as f64).floor();
    let w = 2.0 * std::f64::consts::PI * k / n;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    let mut total = 0.0f64;
    for &x in samples {
        let x = x as f64;
        total += x * x;
        let s0 = x + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    let power = s1 * s1 + s2 * s2 - coeff * s1 * s2;
    let ratio = if total > 0.0 { power / (total * n / 2.0) } else { 0.0 };
    (ratio.min(1.0), (total / n).sqrt())
}

/// The engine on the pushed path without processing: the tones must
/// arrive as pushed, so the detector measures the connection alone.
pub fn engine() -> Engine {
    Engine::new(AudioMode::Pushed(AudioProcessing::NONE)).expect("engine")
}

/// Pushes a tone into `input` every 10 ms until `stop` says so.
pub fn pump_tone(input: AudioInput, hz: f64, mut stop: watch::Receiver<bool>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut phase = 0.0;
        let mut tick = tokio::time::interval(Duration::from_millis(10));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    let mut frame = tone_frame(hz, 8000.0, &mut phase);
                    if let Err(e) = input.push(&mut frame).await {
                        eprintln!("push: {e}");
                    }
                }
                _ = stop.changed() => break,
            }
        }
    })
}

/// Everything `output` gives for `dur`.
pub async fn collect(output: &mut AudioOutput, dur: Duration) -> Vec<i16> {
    let mut out = Vec::new();
    let deadline = tokio::time::sleep(dur);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            frame = output.next() => match frame {
                Some(f) => out.extend_from_slice(&f),
                None => break,
            },
            _ = &mut deadline => break,
        }
    }
    out
}

/// The last two seconds of `got` (past the ramp of the jitter buffer):
/// the ratio of `hz` and the RMS.
pub fn tail_tone(got: &[i16], hz: f64) -> (f64, f64) {
    let tail = &got[got.len().saturating_sub(2 * SAMPLE_RATE as usize)..];
    tone_ratio(tail, hz)
}

/// One side of a pair: the session and what its events said.
pub struct Side {
    pub session: Arc<Session>,
    pub state: watch::Receiver<ConnectionState>,
    pub remote_audio: watch::Receiver<bool>,
    pub events: mpsc::UnboundedReceiver<SessionEvent>,
    /// The first candidate of this side the other refused, if any: a
    /// failure of the test, not a line in its output (a pair on a LAN
    /// connects without the candidates of one side, through the checks of
    /// the other, and would hide it).
    pub wire_error: watch::Receiver<Option<String>>,
}

impl Side {
    /// Fails the test when the other side refused a candidate of this one.
    pub fn assert_wired(&self) {
        if let Some(e) = self.wire_error.borrow().as_ref() {
            panic!("a candidate was refused by the other side: {e}");
        }
    }
}

/// Two sessions of `engine` with `config`, their candidates trickled to
/// each other as they come; nothing negotiated yet.
pub fn pair(engine: &Engine, config: SessionConfig) -> (Side, Side) {
    let a = Arc::new(engine.session(config.clone()).expect("session a"));
    let b = Arc::new(engine.session(config).expect("session b"));
    let side_a = wire(a.clone(), b.clone());
    let side_b = wire(b, a);
    (side_a, side_b)
}

/// Forwards the candidates of `me` to `other` and keeps the rest of the
/// events: the state in a watch, everything in a channel the test may
/// read. The task holds `other` weakly: the two tasks of a pair would
/// otherwise keep each other's session alive, and a session a test
/// dropped would live on (its PeerConnection with it, which libwebrtc's
/// media engine counts).
pub fn wire(me: Arc<Session>, other: Arc<Session>) -> Side {
    let mut events = me.events().expect("events once");
    let other = Arc::downgrade(&other);
    let (state_tx, state) = watch::channel(ConnectionState::New);
    let (audio_tx, remote_audio) = watch::channel(false);
    let (rest_tx, rest) = mpsc::unbounded_channel();
    let (error_tx, wire_error) = watch::channel(None);
    tokio::spawn(async move {
        while let Some(ev) = events.recv().await {
            match &ev {
                SessionEvent::LocalCandidate(c) => {
                    let Some(other) = other.upgrade() else { continue };
                    if let Err(e) = other.add_remote_candidate(c).await {
                        error_tx.send_if_modified(|slot| {
                            if slot.is_none() {
                                *slot = Some(format!("{e} ({})", c.candidate));
                                true
                            } else {
                                false
                            }
                        });
                    }
                }
                SessionEvent::ConnectionState(s) => {
                    let _ = state_tx.send(*s);
                }
                SessionEvent::RemoteAudio => {
                    let _ = audio_tx.send(true);
                }
                _ => {}
            }
            let _ = rest_tx.send(ev);
        }
    });
    Side { session: me, state, remote_audio, events: rest, wire_error }
}

/// Offer from `a`, answer from `b`, through SDP text as over the wire.
pub async fn negotiate(a: &Side, b: &Side, ice_restart: bool) {
    let offer = a.session.create_offer(ice_restart).await.expect("offer");
    assert!(offer.contains("a=sendrecv"), "the offer is both ways:\n{offer}");
    let answer = b.session.accept_offer(&offer).await.expect("answer");
    a.session.accept_answer(&answer).await.expect("accept answer");
}

/// Waits for `side` to be connected, within `timeout`; how long it took.
pub async fn wait_connected(side: &mut Side, timeout: Duration) -> Duration {
    let start = Instant::now();
    let name = "session";
    loop {
        side.assert_wired();
        let s = *side.state.borrow();
        match s {
            ConnectionState::Connected => return start.elapsed(),
            ConnectionState::Failed | ConnectionState::Closed => panic!("{name}: {s:?}"),
            _ => {}
        }
        let left = timeout.saturating_sub(start.elapsed());
        if left.is_zero() {
            panic!("{name}: not connected after {timeout:?}: {s:?}");
        }
        let _ = tokio::time::timeout(left, side.state.changed()).await;
    }
}

/// The far end's audio of `side`, once its track is there.
pub async fn audio_output(side: &mut Side) -> AudioOutput {
    let wait = async {
        while !*side.remote_audio.borrow() {
            side.remote_audio.changed().await.expect("remote audio");
        }
    };
    tokio::time::timeout(Duration::from_secs(10), wait).await.expect("the far end's track within 10 s");
    side.session.take_audio_output().expect("audio output once")
}

/// The address of the interface a peer on the LAN would see: libwebrtc
/// gathers no candidates on loopback, so a node for the tests listens
/// here.
pub fn lan_ip() -> IpAddr {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").expect("udp");
    s.connect("1.1.1.1:53").expect("route");
    s.local_addr().expect("local").ip()
}

/// A call node of `services/call/dist/vcall` on this machine, for the
/// relay tests. `None` when the binary is not there (`make -C
/// services/call release`): the test says so and passes nothing.
pub struct Node {
    child: tokio::process::Child,
    pub control: SocketAddr,
    pub turn: SocketAddr,
    pub id: String,
    _data: tempfile::TempDir,
}

impl Node {
    pub async fn start() -> Option<Node> {
        let bin = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../services/call/dist/vcall");
        if !bin.exists() {
            eprintln!("no {}: make -C services/call release; the relay test is skipped", bin.display());
            return None;
        }
        let ip = lan_ip();
        let data = tempfile::tempdir().expect("tempdir");
        // Relay ports away from libwebrtc's own (ephemeral) and from a node
        // of another test in the same run.
        let base = 50_000 + (std::process::id() % 100) * 100;
        let mut child = tokio::process::Command::new(bin)
            .env("VCALL_LISTEN", format!("{ip}:0"))
            .env("VCALL_TURN_LISTEN", format!("{ip}:0"))
            .env("VCALL_PUBLIC_IP", ip.to_string())
            .env("VCALL_DATA", data.path())
            .env("VCALL_CTL", data.path().join("ctl.sock"))
            .env("VCALL_RELAY_PORTS", format!("{base}-{}", base + 99))
            .env("VCALL_UNSAFE_RELAY_TO_PRIVATE", "true")
            .env("VCALL_LOG", "info")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn vcall");
        let stderr = child.stderr.take().expect("stderr");
        let mut lines = BufReader::new(stderr).lines();
        let up = async {
            while let Some(line) = lines.next_line().await.expect("read") {
                if line.contains("node is up") {
                    return line;
                }
            }
            panic!("vcall ended before it was up");
        };
        let line = tokio::time::timeout(Duration::from_secs(10), up).await.expect("vcall up within 10 s");
        // `... node is up control=1.2.3.4:5 turn=1.2.3.4:6 ... id=<hex>`, in
        // the colours tracing paints a terminal with, pipe or not.
        let line = strip_ansi(&line);
        let field = |name: &str| -> String {
            line.split_whitespace()
                .find_map(|w| w.strip_prefix(&format!("{name}=")))
                .unwrap_or_else(|| panic!("no {name} in: {line}"))
                .to_string()
        };
        let node = Node {
            child,
            control: field("control").parse().expect("control addr"),
            turn: field("turn").parse().expect("turn addr"),
            id: field("id"),
            _data: data,
        };
        // The rest of the log, so that the pipe never fills.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
        Some(node)
    }

    pub fn reference(&self) -> String {
        format!("{}#{}", self.control, self.id)
    }

    pub async fn stop(mut self) {
        let _ = self.child.kill().await;
    }
}

// The control protocol of a node (services/call/crates/vcall-proto):
// enough of it for credentials.
const ALPN: &[u8] = b"vcall/1";
const PATH_HELLO: &str = "/v1/hello";
const PATH_TURN: &str = "/v1/turn";

#[derive(Serialize)]
struct Hello {
    protocol_min: u16,
    protocol_max: u16,
    capabilities: Vec<String>,
    codecs: Vec<String>,
    access: Access,
    client: String,
}

#[derive(Serialize, Default)]
struct Access {
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
}

#[derive(Serialize)]
struct TurnRequest {
    access: Access,
}

#[derive(Deserialize, Debug)]
pub struct Welcome {
    pub node_id: String,
    pub version: String,
    pub capabilities: Vec<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct TurnCredentials {
    pub username: String,
    pub password: String,
    pub realm: String,
    pub ttl_secs: u64,
    pub urls: Vec<String>,
}

impl TurnCredentials {
    /// The ICE server of these credentials over the URLs with `scheme`
    /// (`stun`, `turn`, `turns`) and, for TURN, `transport`.
    pub fn ice_server(&self, scheme: &str, transport: Option<&str>) -> IceServer {
        let urls: Vec<String> = self
            .urls
            .iter()
            .filter(|u| {
                u.starts_with(&format!("{scheme}:"))
                    && transport.is_none_or(|t| u.ends_with(&format!("?transport={t}")))
            })
            .cloned()
            .collect();
        assert!(!urls.is_empty(), "no {scheme} url in {:?}", self.urls);
        IceServer { urls, username: self.username.clone(), password: self.password.clone() }
    }
}

/// HELLO and TURN on the control channel of the node `reference`
/// (`address:port#id`): its welcome and credentials.
pub async fn credentials(reference: &str, access_key: Option<String>) -> (Welcome, TurnCredentials) {
    let node: BridgeRef = reference.parse().expect("node reference");
    let tcp = tokio::time::timeout(Duration::from_secs(10), tokio::net::TcpStream::connect(node.addr))
        .await
        .expect("connect in time")
        .expect("connect");
    let _ = tcp.set_nodelay(true);
    let mut config = rustls::ClientConfig::builder_with_provider(pin::provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("tls13")
        .dangerous()
        .with_custom_certificate_verifier(pin::verifier(node.id))
        .with_no_client_auth();
    config.alpn_protocols = vec![ALPN.to_vec()];
    let name = pin::server_name(&node).expect("server name");
    let tls = tokio::time::timeout(
        Duration::from_secs(10),
        TlsConnector::from(Arc::new(config)).connect(name, tcp),
    )
    .await
    .expect("tls in time")
    .expect("tls: the pin");
    assert_eq!(tls.get_ref().1.alpn_protocol(), Some(ALPN), "the node took the control ALPN");
    let (mut send, connection) = h2io::client_builder().handshake::<_, Bytes>(tls).await.expect("h2");
    tokio::spawn(async move {
        let _ = connection.await;
    });

    async fn post<T: Serialize, R: for<'de> Deserialize<'de>>(
        send: &mut h2::client::SendRequest<Bytes>,
        path: &str,
        body: &T,
    ) -> R {
        let mut ready = send.clone().ready().await.expect("ready");
        let request = Request::builder()
            .method(Method::POST)
            .uri(path)
            .header("content-type", "application/json")
            .body(())
            .expect("request");
        let (response, mut stream) = ready.send_request(request, false).expect("send");
        stream.send_data(Bytes::from(serde_json::to_vec(body).expect("json")), true).expect("body");
        let response = tokio::time::timeout(Duration::from_secs(10), response).await.expect("in time").expect("response");
        let status = response.status().as_u16();
        let mut body = response.into_body();
        let mut out = Vec::new();
        while let Some(chunk) = body.data().await {
            let chunk = chunk.expect("chunk");
            let _ = body.flow_control().release_capacity(chunk.len());
            out.extend_from_slice(&chunk);
        }
        assert_eq!(status, 200, "{path}: {status} {}", String::from_utf8_lossy(&out));
        serde_json::from_slice(&out).expect("json body")
    }

    let hello = Hello {
        protocol_min: 1,
        protocol_max: 1,
        capabilities: ["stun", "turn", "turn-tcp", "turn-tls"].map(String::from).to_vec(),
        codecs: vec!["opus".into()],
        access: Access { key: access_key.clone() },
        client: "messenger-rtc-test/0".into(),
    };
    let welcome: Welcome = post(&mut send, PATH_HELLO, &hello).await;
    assert_eq!(welcome.node_id, node.id.to_string(), "the node is the one pinned");
    let creds: TurnCredentials = post(&mut send, PATH_TURN, &TurnRequest { access: Access { key: access_key } }).await;
    (welcome, creds)
}

/// The candidates trickled so far, for a look at what ICE had.
pub fn candidate_lines(side: &mut Side) -> Vec<Candidate> {
    let mut out = Vec::new();
    while let Ok(ev) = side.events.try_recv() {
        if let SessionEvent::LocalCandidate(c) = ev {
            out.push(c);
        }
    }
    out
}

/// `text` without the escape sequences of a coloured log line.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}
