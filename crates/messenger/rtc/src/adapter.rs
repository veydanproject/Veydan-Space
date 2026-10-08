// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The engine behind the trait of the core: `messenger_calls::MediaEngine`
//! and `messenger_calls::Session` over [`Engine`] and [`crate::Session`].
//!
//! The adapter lives here and not in `messenger-calls` so that the core
//! and its tests stay free of libwebrtc: the core names the trait, this
//! crate fills it. What the two sides say differently is translated:
//!
//! - the core's `IceServer` has optional credentials, the engine's has
//!   strings; the core's `IceCandidate` has an optional mid and index, the
//!   engine's `Candidate` has both;
//! - the core expects `SelectedPair`, `Stats` and `AudioLevel` as events;
//!   the engine answers `stats()` when asked, so a task of the adapter
//!   asks it every second while the session lives and tells the core of
//!   what changed;
//! - a restart of ICE is an offer made with `ice_restart`;
//! - the core expects `NetworkChanged` when the network changes under
//!   the engine. libwebrtc tells nobody in so many words, but with
//!   continual gathering (session.rs) a new interface shows up as a new
//!   local candidate after gathering was complete: that is the word.
//!
//! The rule of the core holds: nothing here calls into it; every event
//! goes on the channel the core takes once.
//!
//! On the pushed path ([`AudioMode::Pushed`]) the sound has to come from
//! somewhere and go somewhere: for every session the engine makes, an
//! [`AudioTap`] goes to whoever took [`RtcEngine::audio_taps`] (the CLI,
//! which plays a file or a tone into it and records what comes out). The
//! video of that path is pushed the same way ([`VideoTap`]): the tap says
//! when frames are wanted, the CLI pushes its test pattern.
//!
//! Video on the device path: `set_video` opens the camera or the screen
//! on a thread of the engine (`camera.rs`, `screen.rs`) into the
//! session's source and enables the track; `Off` stops the thread and
//! disables the track. A phone has no camera thread here: its plugin
//! pushes frames (`push_video_frame`), the track is enabled all the same.
//! Either way nothing is renegotiated. Opening and stopping a device
//! take a moment and may wait on it (`camera.rs`), and the core calls in
//! with its state lock held: both run on tokio's blocking threads, not
//! on the workers of the runtime. A capture that ends by itself (the
//! camera unplugged, the shared window closed) is told to the core as
//! `VideoLost`, which turns the video off and tells the peer.
//!
//! The size of a video is told to the core on its first frame and on
//! every change, and again after a gap in the frames or every so many
//! frames: the core forgets a size when a video goes off (its own by
//! `set_video(Off)`, the peer's by `call.video {on: false}`), and the
//! same camera coming back at the same size would otherwise never be
//! told again. The core drops a size it already has.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use messenger_calls::engine::{
    CameraInfo, ConnectionState as CoreState, IceCandidate, IceServer as CoreIceServer, Media, MediaEngine, PairKind,
    PushedFrame, RelayPolicy, ScreenInfo, SdpKind, Session as CoreSession, SessionEvent as CoreEvent, SessionStats,
    VideoFrame, VideoInput, VideoSettings, VideoTrack,
};
use messenger_core::{MessengerError, Result};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::audio::{AudioInput, AudioOutput};
use crate::camera::CameraCapture;
use crate::engine::{AudioMode, Engine};
use crate::session::{Candidate, ConnectionState, IcePolicy, IceServer, Session, SessionConfig, SessionEvent};
use crate::video::VideoSource;

/// How often the adapter reads the statistics of a session.
const STATS_EVERY: Duration = Duration::from_secs(1);
/// Every so many readings the core is told the statistics; the pair in
/// use and the level of the sound are told on every reading.
const STATS_TOLD_EVERY: u32 = 2;
/// Events of a session the core has not read yet.
const EVENTS_QUEUE: usize = 256;
/// A gap in the frames of a video after which its size is told again
/// on the next frame: the video went off and came back.
const SIZE_AGAIN_AFTER: Duration = Duration::from_millis(250);
/// Frames after which the size is told again regardless (about a second
/// at 30 a second): a gap too short to see, with the peer's word of the
/// off and on arriving after the frames resumed (the relays are slow).
const SIZE_AGAIN_EVERY: u32 = 30;

/// The sound of one session on the pushed path, handed to whoever took
/// [`RtcEngine::audio_taps`]: frames go in through `input`, the far end's
/// frames come out of `output` once its track arrived (`None` when the
/// session ended before that). The video of the same session is `video`.
pub struct AudioTap {
    pub input: AudioInput,
    pub output: oneshot::Receiver<AudioOutput>,
    pub video: VideoTap,
}

/// The video of one session on the pushed path: `source` takes this
/// side's frames (a test pattern), `wanted` says whether any are wanted
/// now (my video is on), and the far end's frames come out of `remote`.
pub struct VideoTap {
    pub source: VideoSource,
    pub wanted: watch::Receiver<bool>,
    pub remote: broadcast::Receiver<Arc<VideoFrame>>,
}

/// What captures this side's video on the device path. Held for its
/// `Drop` alone: letting it go stops the thread that reads the camera or
/// the screen.
#[allow(dead_code)]
enum Capture {
    Camera(CameraCapture),
    #[cfg(not(target_os = "android"))]
    Screen(crate::screen::ScreenCapture),
}

/// [`Engine`] as the core sees it. One per process (see [`RtcEngine::shared`]).
pub struct RtcEngine {
    engine: Engine,
    taps: Mutex<Option<mpsc::UnboundedSender<AudioTap>>>,
    taps_rx: Mutex<Option<mpsc::UnboundedReceiver<AudioTap>>>,
}

static SHARED: OnceLock<Mutex<Option<Arc<RtcEngine>>>> = OnceLock::new();

impl RtcEngine {
    /// The engine in `mode`. Fails where the platform has no audio device
    /// to take ([`AudioMode::Device`] on a machine without one).
    pub fn new(mode: AudioMode) -> Result<Self> {
        let engine = Engine::new(mode).map_err(engine_error)?;
        let (tx, rx) = mpsc::unbounded_channel();
        Ok(Self { engine, taps: Mutex::new(Some(tx)), taps_rx: Mutex::new(Some(rx)) })
    }

    /// The one engine of this process, made on the first call. A second
    /// runtime in the same process (the module switched off and on) takes
    /// the same engine: libwebrtc's factory and the platform's audio
    /// device are made once. Asking for another mode than the one it was
    /// made in is an error.
    pub fn shared(mode: AudioMode) -> Result<Arc<RtcEngine>> {
        let slot = SHARED.get_or_init(|| Mutex::new(None));
        let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(engine) = guard.as_ref() {
            if engine.engine.mode() != mode {
                return Err(MessengerError::Transport(format!(
                    "the media engine of this process runs in {:?}, not {mode:?}",
                    engine.engine.mode()
                )));
            }
            return Ok(engine.clone());
        }
        let engine = Arc::new(Self::new(mode)?);
        *guard = Some(engine.clone());
        Ok(engine)
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// The taps of every session to come on the pushed path, once; `None`
    /// the second time. On the device path nothing ever comes.
    pub fn audio_taps(&self) -> Option<mpsc::UnboundedReceiver<AudioTap>> {
        self.taps_rx.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

#[async_trait]
impl MediaEngine for RtcEngine {
    async fn create_session(&self, ice_servers: Vec<CoreIceServer>, policy: RelayPolicy, _media: Media) -> Result<Box<dyn CoreSession>> {
        // The media of the call says what goes on at the start; the
        // session carries audio and video either way (session.rs).
        let config = SessionConfig {
            ice_servers: ice_servers.into_iter().map(ice_server).collect(),
            policy: match policy {
                RelayPolicy::Auto => IcePolicy::Auto,
                RelayPolicy::RelayOnly => IcePolicy::RelayOnly,
            },
            encryption: None,
        };
        let session = Arc::new(self.engine.session(config).map_err(engine_error)?);
        let events = session.events().expect("the events of a new session");
        let (wanted_tx, wanted_rx) = watch::channel(false);
        let (lost_tx, lost_rx) = mpsc::unbounded_channel();
        // The sound of the pushed path: told to whoever listens, or dropped
        // (then nothing is pushed, and the far end hears silence).
        let output_slot = match session.take_audio_input() {
            Some(input) => {
                let (tx, rx) = oneshot::channel();
                let taps = self.taps.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(taps) = taps.as_ref() {
                    let video = VideoTap { source: session.video_source(), wanted: wanted_rx, remote: session.remote_video_frames() };
                    let _ = taps.send(AudioTap { input, output: rx, video });
                }
                Some(tx)
            }
            None => None,
        };
        Ok(Box::new(RtcSession::new(session, events, output_slot, self.engine.captures_video(), wanted_tx, lost_tx, lost_rx)))
    }

    async fn cameras(&self) -> Vec<CameraInfo> {
        self.engine.cameras()
    }

    async fn screens(&self) -> Vec<ScreenInfo> {
        self.engine.screens()
    }
}

/// The engine made on the first call, not at the start of the runtime: a
/// machine without an audio device (a build server, WSL) runs the
/// messenger as before and learns of the engine when it calls. Takes the
/// engine of the process ([`RtcEngine::shared`]).
pub struct LazyRtcEngine {
    mode: AudioMode,
}

impl LazyRtcEngine {
    pub fn new(mode: AudioMode) -> Self {
        Self { mode }
    }
}

#[async_trait]
impl MediaEngine for LazyRtcEngine {
    async fn create_session(&self, ice_servers: Vec<CoreIceServer>, policy: RelayPolicy, media: Media) -> Result<Box<dyn CoreSession>> {
        RtcEngine::shared(self.mode)?.create_session(ice_servers, policy, media).await
    }

    /// Empty where the engine cannot be made (no audio device): the
    /// cameras are asked for with a call, or before one.
    async fn cameras(&self) -> Vec<CameraInfo> {
        match RtcEngine::shared(self.mode) {
            Ok(engine) => engine.cameras().await,
            Err(_) => vec![],
        }
    }

    async fn screens(&self) -> Vec<ScreenInfo> {
        match RtcEngine::shared(self.mode) {
            Ok(engine) => engine.screens().await,
            Err(_) => vec![],
        }
    }
}

/// One session as the core drives it.
struct RtcSession {
    inner: Arc<Session>,
    events: Mutex<Option<mpsc::Receiver<CoreEvent>>>,
    closed: Arc<AtomicBool>,
    /// The task that carries the engine's events to the core and reads the
    /// statistics; ends with the session.
    pump: tokio::task::JoinHandle<()>,
    /// The engine opens the camera and the screen itself (a computer);
    /// otherwise frames are pushed (the CLI, a phone).
    captures: bool,
    /// The thread capturing this side's video, while it is on.
    capture: Mutex<Option<Capture>>,
    /// Whether frames are wanted from whoever pushes them.
    wanted: watch::Sender<bool>,
    /// Where a capture that ended by itself says why; the pump carries
    /// it to the core.
    lost: mpsc::UnboundedSender<String>,
    /// Gathering for the current description is complete: a candidate
    /// that comes after that is of a network that just came up. Reset by
    /// every new description of this side (an offer, an answer, a restart).
    gathering_done: Arc<AtomicBool>,
}

impl RtcSession {
    fn new(
        inner: Arc<Session>,
        events: mpsc::UnboundedReceiver<SessionEvent>,
        output_slot: Option<oneshot::Sender<AudioOutput>>,
        captures: bool,
        wanted: watch::Sender<bool>,
        lost: mpsc::UnboundedSender<String>,
        lost_rx: mpsc::UnboundedReceiver<String>,
    ) -> Self {
        let (tx, rx) = mpsc::channel(EVENTS_QUEUE);
        let closed = Arc::new(AtomicBool::new(false));
        let gathering_done = Arc::new(AtomicBool::new(false));
        let pump = tokio::spawn(pump(Arc::downgrade(&inner), events, tx, output_slot, closed.clone(), lost_rx, gathering_done.clone()));
        Self { inner, events: Mutex::new(Some(rx)), closed, pump, captures, capture: Mutex::new(None), wanted, lost, gathering_done }
    }

    /// Whatever captured this side's video, taken out of the session (a
    /// camera is let go before another is opened: most machines lend it
    /// to one at a time). Dropping it stops the thread, which may take a
    /// moment ([`crate::camera::STOP_WAIT`]): the async paths drop it
    /// off the runtime's workers ([`RtcSession::stop_capture`]).
    fn take_capture(&self) -> Option<Capture> {
        self.capture.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    async fn stop_capture(&self) {
        if let Some(old) = self.take_capture() {
            let _ = tokio::task::spawn_blocking(move || drop(old)).await;
        }
    }

    /// What a capture calls when it ends by itself.
    fn on_lost(&self) -> crate::camera::OnLost {
        let lost = self.lost.clone();
        Box::new(move |why| {
            let _ = lost.send(why);
        })
    }
}

#[async_trait]
impl CoreSession for RtcSession {
    async fn create_offer(&self) -> Result<String> {
        self.gathering_done.store(false, Ordering::SeqCst);
        self.inner.create_offer(false).await.map_err(engine_error)
    }

    async fn create_answer(&self) -> Result<String> {
        self.gathering_done.store(false, Ordering::SeqCst);
        self.inner.create_answer().await.map_err(engine_error)
    }

    async fn set_remote(&self, sdp: &str, kind: SdpKind) -> Result<()> {
        match kind {
            SdpKind::Offer => self.inner.set_remote_offer(sdp).await,
            SdpKind::Answer => self.inner.set_remote_answer(sdp).await,
        }
        .map_err(engine_error)
    }

    async fn add_ice(&self, candidate: &IceCandidate) -> Result<()> {
        let c = Candidate {
            sdp_mid: candidate.mid.clone().unwrap_or_default(),
            sdp_mline_index: candidate.index.map(i32::from).unwrap_or(0),
            candidate: candidate.candidate.clone(),
        };
        self.inner.add_remote_candidate(&c).await.map_err(engine_error)
    }

    async fn restart_ice(&self) -> Result<String> {
        self.gathering_done.store(false, Ordering::SeqCst);
        self.inner.create_offer(true).await.map_err(engine_error)
    }

    async fn set_mute(&self, muted: bool) -> Result<()> {
        self.inner.set_muted(muted);
        Ok(())
    }

    async fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.pump.abort();
        self.stop_capture().await;
        let _ = self.wanted.send(false);
        self.inner.close();
    }

    async fn set_video(&self, input: VideoInput, settings: VideoSettings) -> Result<()> {
        self.stop_capture().await;
        let outcome = match &input {
            VideoInput::Off => {
                let _ = self.wanted.send(false);
                self.inner.set_video_enabled(false);
                return Ok(());
            }
            VideoInput::Camera { id } => {
                if self.captures {
                    // A source of the camera's size: the engine scales to
                    // the way, the camera is read as it is. Opened off
                    // the workers: it lists and opens devices.
                    let source = self.inner.replace_video_source(settings.width, settings.height, false).map_err(engine_error)?;
                    let (id, on_lost) = (id.clone(), self.on_lost());
                    tokio::task::spawn_blocking(move || CameraCapture::start(id.as_deref(), &settings, source, on_lost))
                        .await
                        .map_err(|e| MessengerError::Transport(format!("the camera could not be opened: {e}")))?
                        .map(|c| Some(Capture::Camera(c)))
                        .map_err(engine_error)
                } else {
                    // Pushed: the plugin of a phone or the CLI pushes when
                    // told the frames are wanted.
                    let _ = self.wanted.send(true);
                    Ok(None)
                }
            }
            VideoInput::Screen { id } => {
                #[cfg(not(target_os = "android"))]
                {
                    if self.captures {
                        let source = self.inner.replace_video_source(settings.width, settings.height, true).map_err(engine_error)?;
                        let (id, on_lost) = (id.clone(), self.on_lost());
                        tokio::task::spawn_blocking(move || crate::screen::ScreenCapture::start(id.as_deref(), source, on_lost))
                            .await
                            .map_err(|e| MessengerError::Transport(format!("the screen could not be opened: {e}")))?
                            .map(|c| Some(Capture::Screen(c)))
                            .map_err(engine_error)
                    } else {
                        Err(MessengerError::Transport("the screen is shared on a computer with its devices only".into()))
                    }
                }
                #[cfg(target_os = "android")]
                {
                    let _ = id;
                    Err(MessengerError::Transport("a phone shares no screen".into()))
                }
            }
        };
        match outcome {
            Ok(capture) => {
                *self.capture.lock().unwrap_or_else(|e| e.into_inner()) = capture;
                self.inner.set_video_enabled(true);
                self.inner.set_video_max_bitrate(settings.max_kbps).map_err(engine_error)?;
                Ok(())
            }
            Err(e) => {
                let _ = self.wanted.send(false);
                self.inner.set_video_enabled(false);
                Err(e)
            }
        }
    }

    async fn set_video_bitrate(&self, max_kbps: Option<u32>) -> Result<()> {
        self.inner.set_video_max_bitrate(max_kbps).map_err(engine_error)
    }

    fn video_frames(&self, track: VideoTrack) -> Option<broadcast::Receiver<Arc<VideoFrame>>> {
        Some(match track {
            VideoTrack::Local => self.inner.local_video_frames(),
            VideoTrack::Remote => self.inner.remote_video_frames(),
        })
    }

    /// A frame the engine drops (no encoder yet, or adapting its rate)
    /// is no error: the pusher goes on with the next.
    fn push_video_frame(&self, frame: PushedFrame) -> Result<()> {
        self.inner.video_source().push_raw(frame).map(|_| ()).map_err(engine_error)
    }

    fn events(&self) -> mpsc::Receiver<CoreEvent> {
        match self.events.lock().unwrap_or_else(|e| e.into_inner()).take() {
            Some(rx) => rx,
            // Taken already: a channel nobody writes to, closed at once.
            None => mpsc::channel(1).1,
        }
    }
}

impl Drop for RtcSession {
    fn drop(&mut self) {
        self.pump.abort();
        // Dropped here, with its bounded wait: nothing to await in a drop.
        drop(self.take_capture());
    }
}

/// The size a frame shows at: turned by 90 or 270 degrees, its width and
/// height change places.
fn shown_size(f: &VideoFrame) -> (u32, u32) {
    if matches!(f.rotation, 90 | 270) {
        (f.height, f.width)
    } else {
        (f.width, f.height)
    }
}

/// When the size of one video is told to the core (the module's note on
/// sizes): on the first frame, on a change, on the first frame after a
/// gap of [`SIZE_AGAIN_AFTER`], and every [`SIZE_AGAIN_EVERY`] frames.
#[derive(Default)]
struct SizeTold {
    last: Option<(u32, u32)>,
    at: Option<Instant>,
    frames_since: u32,
}

impl SizeTold {
    /// A frame of `size` came at `now`: whether to tell the size.
    fn note(&mut self, size: (u32, u32), now: Instant) -> bool {
        self.frames_since += 1;
        let again = self.last != Some(size)
            || self.at.is_some_and(|at| now.duration_since(at) >= SIZE_AGAIN_AFTER)
            || self.frames_since >= SIZE_AGAIN_EVERY;
        self.at = Some(now);
        if again {
            self.last = Some(size);
            self.frames_since = 0;
        }
        again
    }
}

/// Carries the engine's events to the core and asks the session for its
/// statistics every second: the pair in use when it changed (a new pair
/// of addresses as well as a new kind: after a restart of ICE the call
/// is told its way anew, whether the state ever changed or not), the
/// level of the far end's sound, the counters every other reading.
/// Watches the frames of both videos for their size ([`SizeTold`]), and
/// carries the word of a capture that ended by itself. A local candidate
/// after gathering was complete is a network that came up: told to the
/// core as `NetworkChanged`, once per gathering. Holds the session
/// weakly: it ends when the session is gone, and is aborted when it is
/// closed.
async fn pump(
    session: Weak<Session>,
    mut events: mpsc::UnboundedReceiver<SessionEvent>,
    out: mpsc::Sender<CoreEvent>,
    mut output_slot: Option<oneshot::Sender<AudioOutput>>,
    closed: Arc<AtomicBool>,
    mut lost: mpsc::UnboundedReceiver<String>,
    gathering_done: Arc<AtomicBool>,
) {
    let mut tick = tokio::time::interval(STATS_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The pair told last: its kind and its two addresses.
    let mut told_pair: Option<(PairKind, String, String)> = None;
    let mut readings: u32 = 0;
    let mut connected = false;
    let (mut local, mut remote) = match session.upgrade() {
        Some(s) => (Some(s.local_video_frames()), Some(s.remote_video_frames())),
        None => (None, None),
    };
    let (mut local_size, mut remote_size) = (SizeTold::default(), SizeTold::default());
    loop {
        tokio::select! {
            frame = async { local.as_mut().expect("checked").recv().await }, if local.is_some() => {
                match frame {
                    Ok(f) => {
                        let size = shown_size(&f);
                        if local_size.note(size, Instant::now())
                            && out.send(CoreEvent::VideoSize { track: VideoTrack::Local, width: size.0, height: size.1 }).await.is_err()
                        {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => local = None,
                }
            }
            frame = async { remote.as_mut().expect("checked").recv().await }, if remote.is_some() => {
                match frame {
                    Ok(f) => {
                        let size = shown_size(&f);
                        if remote_size.note(size, Instant::now())
                            && out.send(CoreEvent::VideoSize { track: VideoTrack::Remote, width: size.0, height: size.1 }).await.is_err()
                        {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => remote = None,
                }
            }
            why = lost.recv() => {
                let Some(reason) = why else { break };
                if out.send(CoreEvent::VideoLost { reason }).await.is_err() {
                    break;
                }
            }
            ev = events.recv() => {
                let Some(ev) = ev else { break };
                let translated = match ev {
                    SessionEvent::LocalCandidate(c) => {
                        if gathering_done.swap(false, Ordering::SeqCst) {
                            // Gathering was complete: this one is of a network
                            // that just came up. The candidate itself goes too
                            // (first), and the core restarts ICE on the word.
                            tracing::info!(candidate = %c.candidate, "ice: a candidate after gathering was complete: the network changed");
                            let candidate = CoreEvent::LocalCandidate(IceCandidate {
                                candidate: c.candidate,
                                mid: Some(c.sdp_mid),
                                index: u16::try_from(c.sdp_mline_index).ok(),
                            });
                            if out.send(candidate).await.is_err() {
                                break;
                            }
                            Some(CoreEvent::NetworkChanged)
                        } else {
                            Some(CoreEvent::LocalCandidate(IceCandidate {
                                candidate: c.candidate,
                                mid: Some(c.sdp_mid),
                                index: u16::try_from(c.sdp_mline_index).ok(),
                            }))
                        }
                    }
                    SessionEvent::GatheringComplete => {
                        gathering_done.store(true, Ordering::SeqCst);
                        Some(CoreEvent::GatheringComplete)
                    }
                    SessionEvent::ConnectionState(s) => {
                        let state = connection_state(s);
                        connected = state == CoreState::Connected;
                        if !connected {
                            told_pair = None;
                        }
                        Some(CoreEvent::ConnectionState(state))
                    }
                    SessionEvent::RemoteAudio => {
                        if let (Some(slot), Some(session)) = (output_slot.take(), session.upgrade()) {
                            if let Some(output) = session.take_audio_output() {
                                let _ = slot.send(output);
                            }
                        }
                        None
                    }
                    // ICE's own state is inside the connection state; the far
                    // end's video speaks through its frames (the size above);
                    // renegotiation never happens in a 1:1 call; the
                    // cryptors come with the groups.
                    SessionEvent::IceState(_)
                    | SessionEvent::RemoteVideo
                    | SessionEvent::NegotiationNeeded
                    | SessionEvent::Encryption { .. } => None,
                };
                if let Some(ev) = translated {
                    if out.send(ev).await.is_err() {
                        break;
                    }
                }
            }
            _ = tick.tick() => {
                if closed.load(Ordering::SeqCst) || !connected {
                    continue;
                }
                let Some(session) = session.upgrade() else { break };
                let Ok(stats) = session.stats().await else { continue };
                drop(session);
                readings = readings.wrapping_add(1);
                if let Some(path) = &stats.path {
                    let kind = if path.is_relayed() { PairKind::Relay } else { PairKind::Direct };
                    let pair = (kind, path.local_addr.clone(), path.remote_addr.clone());
                    if told_pair.as_ref() != Some(&pair) {
                        told_pair = Some(pair);
                        if out.send(CoreEvent::SelectedPair(kind)).await.is_err() {
                            break;
                        }
                    }
                }
                let level = stats.audio_level_in.clamp(0.0, 1.0) as f32;
                if out.send(CoreEvent::AudioLevel(level)).await.is_err() {
                    break;
                }
                if readings.is_multiple_of(STATS_TOLD_EVERY) {
                    let told = SessionStats {
                        rtt_ms: stats.rtt_ms.map(|ms| ms.round() as u32),
                        bytes_sent: Some(stats.bytes_sent),
                        bytes_received: Some(stats.bytes_received),
                        packets_lost: Some(stats.packets_lost.max(0) as u64),
                        jitter_ms: Some(stats.jitter_ms.round() as u32),
                    };
                    if out.send(CoreEvent::Stats(told)).await.is_err() {
                        break;
                    }
                }
            }
        }
    }
}

fn connection_state(s: ConnectionState) -> CoreState {
    match s {
        ConnectionState::New => CoreState::New,
        ConnectionState::Connecting => CoreState::Connecting,
        ConnectionState::Connected => CoreState::Connected,
        ConnectionState::Disconnected => CoreState::Disconnected,
        ConnectionState::Failed => CoreState::Failed,
        ConnectionState::Closed => CoreState::Closed,
    }
}

fn ice_server(s: CoreIceServer) -> IceServer {
    IceServer { urls: s.urls, username: s.username.unwrap_or_default(), password: s.credential.unwrap_or_default() }
}

/// What the engine said, as the core's error: a call that cannot be made
/// is a failure of the way, not of the input.
fn engine_error(e: crate::Error) -> MessengerError {
    MessengerError::Transport(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The engine of the process is made once and kept: a second taker
    /// gets the same one, it outlives every taker (the slot of the process
    /// holds it), and another mode is refused. So libwebrtc's factory and
    /// the platform's audio device live as long as the process, not as
    /// long as a call; a second call finds them where the first left them
    /// (and brings the device back from the terminated state libwebrtc
    /// leaves it in after the last session: patch 4 of vendor/webrtc-sys,
    /// tests/loopback.rs).
    #[test]
    fn the_shared_engine_is_one_per_process_and_outlives_its_takers() {
        let mode = AudioMode::Pushed(crate::AudioProcessing::NONE);
        let first = RtcEngine::shared(mode).expect("engine");
        let second = RtcEngine::shared(mode).expect("the same engine");
        assert!(Arc::ptr_eq(&first, &second), "one engine per process");
        let weak = Arc::downgrade(&first);
        drop(first);
        drop(second);
        assert!(weak.upgrade().is_some(), "the process holds the engine after its takers let go");
        let err = RtcEngine::shared(AudioMode::Device).err().expect("another mode is refused");
        assert!(err.to_string().contains("runs in Pushed"), "{err}");
    }

    #[test]
    fn the_words_of_the_two_sides_are_translated() {
        let s = ice_server(CoreIceServer { urls: vec!["stun:203.0.113.7:3478".into()], username: None, credential: None });
        assert_eq!(s, IceServer { urls: vec!["stun:203.0.113.7:3478".into()], username: String::new(), password: String::new() });
        let s = ice_server(CoreIceServer { urls: vec!["turn:203.0.113.7:3478".into()], username: Some("u".into()), credential: Some("p".into()) });
        assert_eq!((s.username.as_str(), s.password.as_str()), ("u", "p"));
        assert_eq!(connection_state(ConnectionState::Disconnected), CoreState::Disconnected);
        assert_eq!(connection_state(ConnectionState::Failed), CoreState::Failed);
        assert_eq!(engine_error(crate::Error::Sdp("x".into())).to_string(), "transport error: sdp: x");
    }

    /// The size is told on the first frame, on a change, again after a
    /// gap in the frames (the video went off and on at the same size),
    /// and every so many frames regardless; not on every frame.
    #[test]
    fn the_size_is_told_again_after_a_gap_and_every_so_often() {
        let t0 = Instant::now();
        let mut told = SizeTold::default();
        assert!(told.note((640, 360), t0), "the first frame");
        assert!(!told.note((640, 360), t0 + Duration::from_millis(33)), "the same size, no gap");
        assert!(told.note((360, 640), t0 + Duration::from_millis(66)), "turned");
        assert!(!told.note((360, 640), t0 + Duration::from_millis(99)));
        assert!(told.note((360, 640), t0 + Duration::from_millis(99) + SIZE_AGAIN_AFTER), "after a gap: off and on again");
        let mut t = t0 + Duration::from_millis(99) + SIZE_AGAIN_AFTER;
        let mut again = 0;
        for _ in 0..SIZE_AGAIN_EVERY * 3 {
            t += Duration::from_millis(33);
            if told.note((360, 640), t) {
                again += 1;
            }
        }
        assert_eq!(again, 3, "once every {SIZE_AGAIN_EVERY} frames");
    }

    /// A session of the engine on the pushed path reaches the core through
    /// the trait: an offer with candidates, a tap for its sound, and a
    /// close that ends its events.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_session_of_the_engine_speaks_the_core_trait() {
        let engine = RtcEngine::new(AudioMode::Pushed(crate::AudioProcessing::NONE)).expect("engine");
        let mut taps = engine.audio_taps().expect("taps once");
        assert!(engine.audio_taps().is_none());
        let session = engine.create_session(vec![], RelayPolicy::Auto, Media::Audio).await.expect("session");
        let mut tap = taps.try_recv().expect("a tap for the session");
        let mut events = session.events();
        let offer = session.create_offer().await.expect("offer");
        assert!(offer.contains("a=sendrecv"), "{offer}");
        // An audio call offers video too: turning the camera on later
        // needs no new offer.
        assert_eq!(offer.matches("m=video").count(), 1, "{offer}");
        assert_eq!(offer.matches("m=audio").count(), 1, "{offer}");

        // Video on the pushed path: the tap is told when frames are
        // wanted, a pushed frame reaches the watchers of this side's own
        // picture, the screen is not for this path.
        assert!(!*tap.video.wanted.borrow());
        let settings = VideoSettings { width: 640, height: 360, fps: 30, max_kbps: Some(800) };
        session.set_video(VideoInput::Camera { id: None }, settings).await.expect("camera on the pushed path");
        assert!(*tap.video.wanted.borrow());
        let mut local = session.video_frames(VideoTrack::Local).expect("local frames");
        let pattern = crate::video::test_pattern(320, 180, 1);
        let pushed = PushedFrame { format: crate::video::PixelFormat::I420, width: 320, height: 180, rotation: 90, timestamp_us: 0, data: pattern.data.clone() };
        session.push_video_frame(pushed).expect("pushed");
        let seen = local.try_recv().expect("the pushed frame fans out");
        assert_eq!((seen.width, seen.height, seen.rotation), (320, 180, 90));
        let turned = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(CoreEvent::VideoSize { track: VideoTrack::Local, width, height }) = events.recv().await {
                    return (width, height);
                }
            }
        })
        .await
        .expect("the size of this side's video is told");
        assert_eq!(turned, (180, 320), "shown turned by 90 degrees");
        assert!(session.set_video(VideoInput::Screen { id: None }, settings).await.is_err());
        assert!(!*tap.video.wanted.borrow(), "a failed switch leaves the video off");
        session.set_video(VideoInput::Off, settings).await.expect("off");
        assert!(tap.video.remote.try_recv().is_err(), "nothing came from a far end that is not there");
        // The candidates of this machine, then the end of gathering.
        let mut got_candidate = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            match tokio::time::timeout_at(deadline, events.recv()).await {
                Ok(Some(CoreEvent::LocalCandidate(c))) => {
                    assert!(c.candidate.starts_with("candidate:"), "{c:?}");
                    // The audio m-line or the video one: both are in the offer.
                    assert!(matches!(c.index, Some(0 | 1)), "{c:?}");
                    got_candidate = true;
                }
                Ok(Some(CoreEvent::GatheringComplete)) => break,
                Ok(Some(_)) => {}
                Ok(None) => panic!("the events ended before gathering was complete"),
                Err(_) => break,
            }
        }
        assert!(got_candidate, "a host candidate of this machine");
        let mut frame = vec![0i16; crate::FRAME_SAMPLES];
        tap.input.push(&mut frame).await.expect("a frame goes in");
        session.set_mute(true).await.unwrap();
        session.close().await;
        assert!(session.events().try_recv().is_err(), "the events are taken once");
        // The far end's frames end with the session, so that a reader of
        // them (the page's subscription) learns the call is over.
        let ended = tokio::time::timeout(Duration::from_secs(2), tap.video.remote.recv()).await.expect("the remote frames end in time");
        assert!(matches!(ended, Err(broadcast::error::RecvError::Closed)), "{ended:?}");
        let mut after = session.video_frames(VideoTrack::Remote).expect("a receiver still");
        assert!(matches!(after.try_recv(), Err(broadcast::error::TryRecvError::Closed)), "closed from the start after the session");
    }
}
