// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A media engine with no media: sessions of one process that "connect"
//! to each other by their SDP. Deterministic, no timers, no sockets, for
//! the tests of `messenger-calls` and for a demo.
//!
//! An offer is `fake-offer:<session>:<epoch>`, an answer
//! `fake-answer:<session>`. `set_remote` of an answer links the two
//! sessions: both are told `Connecting`, `Connected` and the pair they
//! settled on, which is `Relay` when either side was made with the
//! relay-only policy and `Direct` otherwise. `close` tells the linked
//! peer `Disconnected`. Whatever else the tests need to happen to a
//! session (a network change, a loss) they inject with [`FakeHandle`].
//!
//! Video is recorded, not carried: every `set_video` and every cap of the
//! bitrate goes into the [`Record`]; a camera "fails to open" when the
//! engine is told so ([`FakeEngine::set_camera_fails`]). The frames of a
//! session are broadcasts a test may push into ([`FakeHandle::show`]),
//! as the far end's decoded frames would come.

use async_trait::async_trait;
use messenger_calls::engine::{
    ConnectionState, IceCandidate, IceServer, Media, MediaEngine, PairKind, PushedFrame, RelayPolicy, SdpKind, Session,
    SessionEvent, VideoFrame, VideoInput, VideoSettings, VideoTrack,
};
use messenger_core::{MessengerError, Result};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, mpsc};

#[derive(Default)]
struct Fabric {
    next: u32,
    sessions: HashMap<u32, Peer>,
}

struct Peer {
    tx: mpsc::Sender<SessionEvent>,
    relay_only: bool,
    linked: Option<u32>,
    closed: bool,
}

/// What one session did, for the tests to look at.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Record {
    pub offers: u32,
    pub answers: u32,
    pub restarts: u32,
    pub remote_sdps: Vec<String>,
    pub remote_candidates: Vec<IceCandidate>,
    pub muted: bool,
    pub closed: bool,
    pub ice_servers: Vec<IceServer>,
    pub policy: RelayPolicy,
    pub media: Option<Media>,
    /// Every `set_video`, in order (the failed ones too).
    pub video: Vec<VideoInput>,
    /// The settings of the last `set_video` that was not `Off`.
    pub video_settings: Option<VideoSettings>,
    /// Every cap of the video bitrate, in order (`set_video_bitrate`).
    pub video_bitrate: Vec<Option<u32>>,
    /// Frames pushed into this side's video.
    pub pushed_frames: u32,
}

#[derive(Clone)]
pub struct FakeEngine {
    fabric: Arc<Mutex<Fabric>>,
    records: Arc<Mutex<HashMap<u32, Record>>>,
    /// Off: an answer links nothing, as when ICE finds no way.
    connects: Arc<AtomicBool>,
    /// On: an answer between two sessions linked already says nothing (no
    /// `Connecting`, `Connected`, pair), as libwebrtc does on a restart of
    /// ICE while the old way still works: the state never changed.
    quiet_relink: Arc<AtomicBool>,
    /// On: a camera or a screen fails to open (`set_video` errs).
    camera_fails: Arc<AtomicBool>,
    /// The frames of every session, by its id: this side's own and the far
    /// end's, for the tests to push into and the core to read.
    frames: Arc<Mutex<HashMap<u32, Frames>>>,
}

struct Frames {
    local: broadcast::Sender<Arc<VideoFrame>>,
    remote: broadcast::Sender<Arc<VideoFrame>>,
}

impl Default for FakeEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeEngine {
    pub fn new() -> Self {
        Self {
            fabric: Arc::default(),
            records: Arc::default(),
            connects: Arc::new(AtomicBool::new(true)),
            quiet_relink: Arc::new(AtomicBool::new(false)),
            camera_fails: Arc::new(AtomicBool::new(false)),
            frames: Arc::default(),
        }
    }

    /// Whether an answer connects the two sessions (it does by default).
    pub fn set_connects(&self, on: bool) {
        self.connects.store(on, Ordering::SeqCst);
    }

    /// Whether an answer between sessions linked already (a restart of
    /// ICE) passes without a word of the state, as libwebrtc's does when
    /// the old way still works (off by default: the state is told again,
    /// as after a loss).
    pub fn set_quiet_relink(&self, on: bool) {
        self.quiet_relink.store(on, Ordering::SeqCst);
    }

    /// Whether a camera or a screen fails to open from now on (`set_video`
    /// of anything but `Off` errs, as an engine whose camera is taken).
    pub fn set_camera_fails(&self, on: bool) {
        self.camera_fails.store(on, Ordering::SeqCst);
    }

    /// The sessions made so far, in order.
    pub fn sessions(&self) -> Vec<FakeHandle> {
        let fabric = self.fabric.lock().unwrap();
        let mut ids: Vec<u32> = fabric.sessions.keys().copied().collect();
        ids.sort_unstable();
        ids.into_iter().map(|id| FakeHandle { id, engine: self.clone() }).collect()
    }

    pub fn session_count(&self) -> usize {
        self.fabric.lock().unwrap().sessions.len()
    }

    fn tell(fabric: &Fabric, id: u32, ev: SessionEvent) {
        if let Some(p) = fabric.sessions.get(&id) {
            let _ = p.tx.try_send(ev);
        }
    }

    fn connect(&self, a: u32, b: u32) {
        let mut fabric = self.fabric.lock().unwrap();
        let relay = fabric.sessions.get(&a).is_some_and(|p| p.relay_only) || fabric.sessions.get(&b).is_some_and(|p| p.relay_only);
        let linked_already = fabric.sessions.get(&a).is_some_and(|p| p.linked == Some(b)) && fabric.sessions.get(&b).is_some_and(|p| p.linked == Some(a));
        if linked_already && self.quiet_relink.load(Ordering::SeqCst) {
            return;
        }
        for (me, other) in [(a, b), (b, a)] {
            if let Some(p) = fabric.sessions.get_mut(&me) {
                p.linked = Some(other);
            }
        }
        for id in [a, b] {
            Self::tell(&fabric, id, SessionEvent::ConnectionState(ConnectionState::Connecting));
            Self::tell(&fabric, id, SessionEvent::ConnectionState(ConnectionState::Connected));
            Self::tell(&fabric, id, SessionEvent::SelectedPair(if relay { PairKind::Relay } else { PairKind::Direct }));
        }
    }
}

/// A test's hold on one session.
#[derive(Clone)]
pub struct FakeHandle {
    id: u32,
    engine: FakeEngine,
}

impl FakeHandle {
    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn record(&self) -> Record {
        self.engine.records.lock().unwrap().get(&self.id).cloned().unwrap_or_default()
    }

    /// Tell the session's owner something, as the engine would.
    pub fn inject(&self, ev: SessionEvent) {
        FakeEngine::tell(&self.engine.fabric.lock().unwrap(), self.id, ev);
    }

    /// The session this one is connected to, if any.
    pub fn linked(&self) -> Option<u32> {
        self.engine.fabric.lock().unwrap().sessions.get(&self.id).and_then(|p| p.linked)
    }

    /// A frame of `track` of this session, as the engine would hand it
    /// out (the far end's decoded, or this side's own as captured).
    pub fn show(&self, track: VideoTrack, frame: VideoFrame) {
        if let Some(f) = self.engine.frames.lock().unwrap().get(&self.id) {
            let _ = match track {
                VideoTrack::Local => f.local.send(Arc::new(frame)),
                VideoTrack::Remote => f.remote.send(Arc::new(frame)),
            };
        }
    }
}

#[async_trait]
impl MediaEngine for FakeEngine {
    async fn create_session(&self, ice_servers: Vec<IceServer>, policy: RelayPolicy, media: Media) -> Result<Box<dyn Session>> {
        let (tx, rx) = mpsc::channel(1024);
        let id = {
            let mut fabric = self.fabric.lock().unwrap();
            fabric.next += 1;
            let id = fabric.next;
            fabric.sessions.insert(id, Peer { tx, relay_only: policy == RelayPolicy::RelayOnly, linked: None, closed: false });
            id
        };
        self.records.lock().unwrap().insert(
            id,
            Record { ice_servers, policy, media: Some(media), ..Record::default() },
        );
        self.frames.lock().unwrap().insert(id, Frames { local: broadcast::channel(4).0, remote: broadcast::channel(4).0 });
        Ok(Box::new(FakeSession { id, engine: self.clone(), rx: Mutex::new(Some(rx)), epoch: Mutex::new(0) }))
    }
}

pub struct FakeSession {
    id: u32,
    engine: FakeEngine,
    rx: Mutex<Option<mpsc::Receiver<SessionEvent>>>,
    epoch: Mutex<u32>,
}

impl FakeSession {
    fn tell(&self, ev: SessionEvent) {
        FakeEngine::tell(&self.engine.fabric.lock().unwrap(), self.id, ev);
    }

    fn with_record(&self, f: impl FnOnce(&mut Record)) {
        if let Some(r) = self.engine.records.lock().unwrap().get_mut(&self.id) {
            f(r);
        }
    }

    /// A host candidate and, with a TURN server, a relay one; then the end
    /// of gathering.
    fn gather(&self, epoch: u32) {
        self.tell(SessionEvent::LocalCandidate(IceCandidate {
            candidate: format!("candidate:{}{} 1 udp 2130706431 10.0.0.{} 5000 typ host", self.id, epoch, self.id),
            mid: Some("0".into()),
            index: Some(0),
        }));
        let has_turn = self.record_has_turn();
        if has_turn {
            self.tell(SessionEvent::LocalCandidate(IceCandidate {
                candidate: format!("candidate:{}{}r 1 udp 16777215 203.0.113.{} 49152 typ relay", self.id, epoch, self.id),
                mid: Some("0".into()),
                index: Some(0),
            }));
        }
        self.tell(SessionEvent::GatheringComplete);
    }

    fn record_has_turn(&self) -> bool {
        self.engine
            .records
            .lock()
            .unwrap()
            .get(&self.id)
            .is_some_and(|r| r.ice_servers.iter().any(|s| s.urls.iter().any(|u| u.starts_with("turn"))))
    }
}

fn remote_session(sdp: &str, prefix: &str) -> Result<u32> {
    sdp.strip_prefix(prefix)
        .and_then(|rest| rest.split(':').next())
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| MessengerError::Invalid(format!("not a fake sdp: {sdp}")))
}

#[async_trait]
impl Session for FakeSession {
    async fn create_offer(&self) -> Result<String> {
        let epoch = *self.epoch.lock().unwrap();
        self.with_record(|r| r.offers += 1);
        self.gather(epoch);
        Ok(format!("fake-offer:{}:{epoch}", self.id))
    }

    async fn create_answer(&self) -> Result<String> {
        self.with_record(|r| r.answers += 1);
        let epoch = *self.epoch.lock().unwrap();
        self.gather(epoch);
        Ok(format!("fake-answer:{}", self.id))
    }

    async fn set_remote(&self, sdp: &str, kind: SdpKind) -> Result<()> {
        self.with_record(|r| r.remote_sdps.push(sdp.to_string()));
        match kind {
            SdpKind::Offer => {
                let remote = remote_session(sdp, "fake-offer:")?;
                // The answerer links when its answer is taken; it only
                // remembers whom it answers.
                let mut fabric = self.engine.fabric.lock().unwrap();
                if let Some(p) = fabric.sessions.get_mut(&self.id) {
                    if p.linked.is_none() {
                        p.linked = Some(remote);
                    }
                }
                Ok(())
            }
            SdpKind::Answer => {
                let remote = remote_session(sdp, "fake-answer:")?;
                if self.engine.connects.load(Ordering::SeqCst) {
                    self.engine.connect(self.id, remote);
                }
                Ok(())
            }
        }
    }

    async fn add_ice(&self, candidate: &IceCandidate) -> Result<()> {
        self.with_record(|r| r.remote_candidates.push(candidate.clone()));
        Ok(())
    }

    async fn restart_ice(&self) -> Result<String> {
        let epoch = {
            let mut e = self.epoch.lock().unwrap();
            *e += 1;
            *e
        };
        self.with_record(|r| r.restarts += 1);
        self.gather(epoch);
        Ok(format!("fake-offer:{}:{epoch}", self.id))
    }

    async fn set_mute(&self, muted: bool) -> Result<()> {
        self.with_record(|r| r.muted = muted);
        Ok(())
    }

    async fn close(&self) {
        self.with_record(|r| r.closed = true);
        let fabric = self.engine.fabric.lock().unwrap();
        // Only a peer linked back is told: an answerer whose answer was
        // not taken knows its offerer, but the offerer talks to another.
        let linked = fabric.sessions.get(&self.id).and_then(|p| p.linked);
        if let Some(other) = linked {
            if fabric.sessions.get(&other).is_some_and(|p| !p.closed && p.linked == Some(self.id)) {
                FakeEngine::tell(&fabric, other, SessionEvent::ConnectionState(ConnectionState::Disconnected));
            }
        }
        drop(fabric);
        if let Some(p) = self.engine.fabric.lock().unwrap().sessions.get_mut(&self.id) {
            p.closed = true;
        }
        self.tell(SessionEvent::ConnectionState(ConnectionState::Closed));
    }

    fn events(&self) -> mpsc::Receiver<SessionEvent> {
        self.rx.lock().unwrap().take().expect("events() may be called once")
    }

    async fn set_video(&self, input: VideoInput, settings: VideoSettings) -> Result<()> {
        self.with_record(|r| r.video.push(input.clone()));
        if input == VideoInput::Off {
            return Ok(());
        }
        if self.engine.camera_fails.load(Ordering::SeqCst) {
            return Err(MessengerError::Transport("fake: the camera cannot be opened".into()));
        }
        self.with_record(|r| r.video_settings = Some(settings));
        Ok(())
    }

    async fn set_video_bitrate(&self, max_kbps: Option<u32>) -> Result<()> {
        self.with_record(|r| r.video_bitrate.push(max_kbps));
        Ok(())
    }

    fn video_frames(&self, track: VideoTrack) -> Option<broadcast::Receiver<Arc<VideoFrame>>> {
        let frames = self.engine.frames.lock().unwrap();
        let f = frames.get(&self.id)?;
        Some(match track {
            VideoTrack::Local => f.local.subscribe(),
            VideoTrack::Remote => f.remote.subscribe(),
        })
    }

    /// Counted, and shown as this side's own picture at its size (the
    /// bytes are not converted: a black frame stands in).
    fn push_video_frame(&self, frame: PushedFrame) -> Result<()> {
        self.with_record(|r| r.pushed_frames += 1);
        let mut shown = VideoFrame::black(frame.width, frame.height);
        shown.rotation = frame.rotation;
        shown.timestamp_us = frame.timestamp_us;
        if let Some(f) = self.engine.frames.lock().unwrap().get(&self.id) {
            let _ = f.local.send(Arc::new(shown));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn two_sessions_connect_through_their_sdp() {
        let engine = FakeEngine::new();
        let turn = IceServer { urls: vec!["turn:203.0.113.7:3478?transport=udp".into()], username: Some("u".into()), credential: Some("p".into()) };
        let a = engine.create_session(vec![turn], RelayPolicy::Auto, Media::Audio).await.unwrap();
        let b = engine.create_session(vec![], RelayPolicy::RelayOnly, Media::Audio).await.unwrap();
        let (mut ea, mut eb) = (a.events(), b.events());

        let offer = a.create_offer().await.unwrap();
        assert_eq!(offer, "fake-offer:1:0");
        assert!(matches!(ea.recv().await, Some(SessionEvent::LocalCandidate(c)) if c.candidate.contains("typ host")));
        assert!(matches!(ea.recv().await, Some(SessionEvent::LocalCandidate(c)) if c.candidate.contains("typ relay")));
        assert_eq!(ea.recv().await, Some(SessionEvent::GatheringComplete));

        b.set_remote(&offer, SdpKind::Offer).await.unwrap();
        let answer = b.create_answer().await.unwrap();
        assert!(matches!(eb.recv().await, Some(SessionEvent::LocalCandidate(_))));
        assert_eq!(eb.recv().await, Some(SessionEvent::GatheringComplete));
        a.set_remote(&answer, SdpKind::Answer).await.unwrap();
        for rx in [&mut ea, &mut eb] {
            assert_eq!(rx.recv().await, Some(SessionEvent::ConnectionState(ConnectionState::Connecting)));
            assert_eq!(rx.recv().await, Some(SessionEvent::ConnectionState(ConnectionState::Connected)));
            assert_eq!(rx.recv().await, Some(SessionEvent::SelectedPair(PairKind::Relay)), "one side is relay-only");
        }
        let handles = engine.sessions();
        assert_eq!(handles.len(), 2);
        assert_eq!(handles[0].linked(), Some(2));

        assert_eq!(a.restart_ice().await.unwrap(), "fake-offer:1:1");
        a.set_mute(true).await.unwrap();
        a.close().await;
        assert_eq!(eb.recv().await, Some(SessionEvent::ConnectionState(ConnectionState::Disconnected)));
        let r = handles[0].record();
        assert_eq!((r.offers, r.restarts, r.muted, r.closed), (1, 1, true, true));
        assert!(a.set_remote("v=0", SdpKind::Answer).await.is_err(), "not a fake sdp");
    }

    #[tokio::test]
    async fn video_is_recorded_and_frames_are_shown() {
        let engine = FakeEngine::new();
        let a = engine.create_session(vec![], RelayPolicy::Auto, Media::Video).await.unwrap();
        let settings = VideoSettings { width: 640, height: 360, fps: 30, max_kbps: Some(800) };
        a.set_video(VideoInput::Camera { id: None }, settings).await.unwrap();
        a.set_video_bitrate(Some(500)).await.unwrap();
        engine.set_camera_fails(true);
        assert!(a.set_video(VideoInput::Screen { id: None }, settings).await.is_err());
        a.set_video(VideoInput::Off, settings).await.unwrap();
        let handle = engine.sessions()[0].clone();
        let r = handle.record();
        assert_eq!(r.video, vec![VideoInput::Camera { id: None }, VideoInput::Screen { id: None }, VideoInput::Off]);
        assert_eq!(r.video_settings, Some(settings));
        assert_eq!(r.video_bitrate, vec![Some(500)]);

        let mut remote = a.video_frames(VideoTrack::Remote).unwrap();
        let mut local = a.video_frames(VideoTrack::Local).unwrap();
        handle.show(VideoTrack::Remote, VideoFrame::black(320, 180));
        assert_eq!((remote.try_recv().unwrap().width, local.try_recv().is_err()), (320, true));
        let pushed = PushedFrame { format: messenger_calls::engine::PixelFormat::Nv21, width: 4, height: 2, rotation: 90, timestamp_us: 5, data: vec![0; 12] };
        a.push_video_frame(pushed).unwrap();
        let shown = local.try_recv().unwrap();
        assert_eq!((shown.width, shown.height, shown.rotation), (4, 2, 90));
        assert_eq!(handle.record().pushed_frames, 1);
    }
}
