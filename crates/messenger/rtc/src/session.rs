// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A session: one PeerConnection, from the ICE servers to the statistics.
//!
//! The order of a call, as the signaling of `messenger-calls` drives it:
//!
//! ```text
//! caller                                        callee
//! Engine::session(config)                       Engine::session(config)
//! create_offer(false) ──── sdp offer ─────────▶ accept_offer(sdp) ─▶ sdp answer
//! accept_answer(sdp) ◀──── sdp answer ─────────┘
//! events: LocalCandidate ─── candidates ──────▶ add_remote_candidate
//! add_remote_candidate ◀──── candidates ─────── events: LocalCandidate
//! events: ConnectionState(Connected)            events: ConnectionState(Connected)
//! ```
//!
//! A restart of ICE (the network changed, the relay went away) is
//! [`Session::create_offer`] with `ice_restart`, by the same way; the
//! candidates come again. [`Session::path`] says whether the sound goes
//! directly or through a relay, for the label on the screen.
//!
//! The signaling keeps no order between the answer and the candidates
//! (they travel as separate events, over different relays), so a candidate
//! of the far end may arrive before its description. libwebrtc refuses
//! such a candidate instead of keeping it, so the session keeps it itself
//! and hands it over once the description is set.
//!
//! Video is in every session from the start: a video track of this side,
//! disabled, on a sender of its own, so that the first offer carries a
//! video m-line both ways and the far end's does too. Turning the camera
//! on is enabling the track and pushing frames into its source; off is
//! the reverse. No renegotiation either way, which matters because the
//! core lets only the caller make offers (internal/messenger-wire.md
//! §10). The frames of the far end come out of
//! [`Session::remote_video_frames`], this side's own out of
//! [`Session::local_video_frames`], both as broadcasts a late reader
//! skips along.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use libwebrtc::audio_source::native::NativeAudioSource;
use libwebrtc::audio_source::AudioSourceOptions;
use libwebrtc::audio_stream::native::{NativeAudioStream, NativeAudioStreamOptions};
use libwebrtc::audio_track::RtcAudioTrack;
use libwebrtc::ice_candidate::IceCandidate;
use libwebrtc::media_stream_track::MediaStreamTrack;
use libwebrtc::native::frame_cryptor::{EncryptionAlgorithm, FrameCryptor};
use libwebrtc::peer_connection::{
    AnswerOptions, IceConnectionState, IceGatheringState, OfferOptions, PeerConnection, PeerConnectionState,
};
use libwebrtc::peer_connection_factory::native::PeerConnectionFactoryExt;
use libwebrtc::peer_connection_factory::{
    ContinualGatheringPolicy, IceServer as LkIceServer, IceTransportsType, PeerConnectionFactory, RtcConfiguration,
};
use libwebrtc::rtp_sender::RtpSender;
use libwebrtc::session_description::{SdpType, SessionDescription};
use libwebrtc::stats::{IceCandidateType, RtcStats};
use libwebrtc::video_stream::native::{NativeVideoStream, NativeVideoStreamOptions};
use libwebrtc::video_track::RtcVideoTrack;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::{broadcast, oneshot};

use crate::audio::{self, Apm, AudioInput, AudioOutput, SAMPLE_RATE};
use crate::crypto::{Encryption, EncryptionState};
use crate::engine::{AudioMode, Inner as EngineInner};
use crate::video::{self, VideoFrame, VideoOutput, VideoSource};
use crate::{Error, Result};

/// The size the video source of a new session is made for; a camera or
/// a screen replaces it with one of its own size.
const DEFAULT_VIDEO: (u32, u32) = (640, 360);

/// A STUN or TURN server as the node's credentials name it: `stun:`,
/// `turn:` and `turns:` URIs (RFC 7064, 7065) with the user and the
/// password that go with them (empty for STUN).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IceServer {
    pub urls: Vec<String>,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
}

/// Which pairs of candidates ICE may take.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IcePolicy {
    /// Directly when it works, through a relay when it does not: the
    /// order ICE keeps by itself.
    #[default]
    Auto,
    /// Through a relay only: "always through a relay" of the settings,
    /// and the tests of the relay.
    RelayOnly,
}

#[derive(Debug, Clone, Default)]
pub struct SessionConfig {
    pub ice_servers: Vec<IceServer>,
    pub policy: IcePolicy,
    /// Encrypt the frames of every track with these keys (groups).
    pub encryption: Option<Encryption>,
}

/// One ICE candidate as it travels in the signaling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub sdp_mid: String,
    pub sdp_mline_index: i32,
    pub candidate: String,
}

/// The state of the whole connection (DTLS included).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    New,
    Connecting,
    Connected,
    Disconnected,
    Failed,
    Closed,
}

/// The state of ICE alone: `Disconnected` is where a restart is due,
/// `Failed` where it is overdue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IceState {
    New,
    Checking,
    Connected,
    Completed,
    Failed,
    Disconnected,
    Closed,
}

/// What a session reports, in the order it happened.
#[derive(Debug, Clone)]
pub enum SessionEvent {
    /// A candidate of this side, to send to the other.
    LocalCandidate(Candidate),
    /// No more candidates for the current description.
    GatheringComplete,
    ConnectionState(ConnectionState),
    IceState(IceState),
    /// The far end's audio track arrived; on the pushed path
    /// [`Session::take_audio_output`] has frames from now on.
    RemoteAudio,
    /// The far end's video track arrived (with its description: every
    /// call has one); [`Session::remote_video_frames`] gives frames once
    /// the far end sends any.
    RemoteVideo,
    /// The engine wants a new offer (a track was added after the first
    /// negotiation).
    NegotiationNeeded,
    /// The cryptor of one direction changed state; `participant` is this
    /// side's name for a sender, the other side's for a receiver.
    Encryption { participant: String, state: EncryptionState },
}

/// The kind of a candidate in the pair in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    /// An address of the machine itself.
    Host,
    /// The address the STUN server saw: the outside of a NAT.
    ServerReflexive,
    /// An address learnt from the other side's checks.
    PeerReflexive,
    /// An address on a TURN relay.
    Relay,
}

/// The pair of candidates the sound goes by.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Path {
    pub local: CandidateKind,
    pub remote: CandidateKind,
    /// `address:port` of each end.
    pub local_addr: String,
    pub remote_addr: String,
    /// `udp` or `tcp`, as the local candidate has it.
    pub protocol: String,
    pub rtt_ms: Option<f64>,
}

impl Path {
    /// Does the sound go through a relay on either end? The label
    /// "through a relay" of the screen.
    pub fn is_relayed(&self) -> bool {
        self.local == CandidateKind::Relay || self.remote == CandidateKind::Relay
    }
}

/// A reading of the statistics of a session. Counters are totals: a
/// rate is the difference of two readings over the time between them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    pub path: Option<Path>,
    /// From the pair in use, milliseconds.
    pub rtt_ms: Option<f64>,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub packets_sent: u64,
    pub packets_received: u64,
    /// Packets of the far end that never came, as its RTP counts them.
    pub packets_lost: i64,
    /// The jitter of what comes in, milliseconds.
    pub jitter_ms: f64,
    /// What the far end reports lost of what this side sent, 0.0–1.0.
    pub fraction_lost: f64,
    /// Level of the audio sent and received, 0.0–1.0.
    pub audio_level_out: f64,
    pub audio_level_in: f64,
    /// What the bandwidth estimate allows this side to send, kbit/s.
    pub available_outgoing_kbps: f64,
}

/// What the callbacks of libwebrtc reach: the events channel, the
/// outputs that appear with the far end's tracks, the cryptors.
struct Shared {
    events: UnboundedSender<SessionEvent>,
    audio_output: Mutex<Option<AudioOutput>>,
    /// Where the far end's video goes once its track arrives: to the task
    /// that pumps it into `remote_frames`.
    video_output: Mutex<Option<oneshot::Sender<VideoOutput>>>,
    /// This side's own frames, as pushed into the source in use.
    local_frames: broadcast::Sender<Arc<VideoFrame>>,
    /// Frames of the far end are handed out (the pushed path) rather than
    /// played by the device.
    pushed: bool,
    apm: Option<Arc<Apm>>,
    encryption: Option<EncryptionShared>,
}

struct EncryptionShared {
    keys: crate::FrameKeys,
    participant: String,
    enabled: AtomicBool,
    key_index: Mutex<u8>,
    senders: Mutex<Vec<FrameCryptor>>,
    receivers: Mutex<Vec<FrameCryptor>>,
}

/// The far end's candidates that came before its description, and whether
/// the description is there. libwebrtc answers a candidate added before
/// the remote description with an error (`AddIceCandidate` fails with
/// "no remote description", `api/uma_metrics.h`) and forgets it, so the
/// session keeps such candidates until a description is set and adds them
/// then.
#[derive(Default)]
struct Remote {
    described: bool,
    pending: Vec<Candidate>,
}

/// One PeerConnection with its audio track and its video track. Dropping
/// it closes the connection.
pub struct Session {
    pc: PeerConnection,
    factory: PeerConnectionFactory,
    audio_track: RtcAudioTrack,
    shared: Arc<Shared>,
    events: Mutex<Option<UnboundedReceiver<SessionEvent>>>,
    audio_input: Mutex<Option<AudioInput>>,
    /// The sender of this side's video, there from the start; its track
    /// is replaced when the source changes (camera to screen).
    video_sender: RtpSender,
    video_track: Mutex<RtcVideoTrack>,
    video_source: Mutex<VideoSource>,
    remote: Mutex<Remote>,
    /// The far end's frames, decoded: the sender the readers subscribe
    /// to, let go when the session closes so that they learn it is over
    /// (`Closed`). Not in `Shared`: the callbacks of libwebrtc hold that
    /// for as long as the native connection lives, past the close.
    remote_frames: Mutex<Option<broadcast::Sender<Arc<VideoFrame>>>>,
    /// The task that pumps the far end's track into `remote_frames`. The
    /// stream it reads ends only when it is dropped (libwebrtc's
    /// `NativeVideoStream` closes its queue on nothing else, not on the
    /// end of the track nor the close of the connection), so the task is
    /// aborted with the session, which drops the stream, the sink on the
    /// track and the task's copy of the sender.
    remote_pump: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Session {
    pub(crate) fn new(engine: Arc<EngineInner>, config: SessionConfig) -> Result<Session> {
        let factory = engine.factory.clone();
        // Non-exhaustive in the crate: the defaults, then what matters here.
        let mut rtc = RtcConfiguration::default();
        rtc.ice_servers = config
            .ice_servers
            .iter()
            .map(|s| LkIceServer { urls: s.urls.clone(), username: s.username.clone(), password: s.password.clone() })
            .collect();
        // Candidates keep coming after the first gathering: a new interface
        // (Wi-Fi to mobile) shows up as candidates without a restart.
        rtc.continual_gathering_policy = ContinualGatheringPolicy::GatherContinually;
        rtc.ice_transport_type = match config.policy {
            IcePolicy::Auto => IceTransportsType::All,
            IcePolicy::RelayOnly => IceTransportsType::Relay,
        };
        let pc = factory.create_peer_connection(rtc)?;

        let (events, events_rx) = mpsc::unbounded_channel();
        let (pushed, apm) = match engine.mode {
            AudioMode::Device => (false, None),
            AudioMode::Pushed(processing) => (true, Apm::new(processing)),
        };
        let encryption = config.encryption.as_ref().map(|e| EncryptionShared {
            keys: e.keys.clone(),
            participant: e.participant.clone(),
            enabled: AtomicBool::new(true),
            key_index: Mutex::new(0),
            senders: Mutex::new(Vec::new()),
            receivers: Mutex::new(Vec::new()),
        });
        let (local_frames, _) = broadcast::channel(video::FAN_OUT_FRAMES);
        let (remote_frames, _) = broadcast::channel(video::FAN_OUT_FRAMES);
        let (video_output_tx, video_output_rx) = oneshot::channel();
        let shared = Arc::new(Shared {
            events,
            audio_output: Mutex::new(None),
            video_output: Mutex::new(Some(video_output_tx)),
            local_frames,
            pushed,
            apm,
            encryption,
        });
        // The far end's video, once its track is there, into its
        // broadcast; the task is aborted with the session (see
        // `remote_pump`), which is the only way its stream ends.
        let pump_frames = remote_frames.clone();
        let remote_pump = tokio::spawn(async move {
            let Ok(mut output) = video_output_rx.await else { return };
            while let Some(frame) = output.next().await {
                let _ = pump_frames.send(Arc::new(frame));
            }
        });

        // The audio track of this side: the microphone through the device
        // module, or a source the caller pushes into.
        let (audio_track, audio_input) = match engine.mode {
            AudioMode::Device => (factory.create_device_audio_track("audio"), None),
            AudioMode::Pushed(_) => {
                // Options are kept by the source and act on nothing for
                // pushed frames (REPORT.md, section 3); the processing is
                // this crate's, in AudioInput.
                let source = NativeAudioSource::new(AudioSourceOptions::default(), SAMPLE_RATE, 1, 0);
                let track = factory.create_audio_track("audio", source.clone());
                (track, Some(AudioInput::new(source, shared.apm.clone())))
            }
        };
        let sender = pc.add_track(MediaStreamTrack::Audio(audio_track.clone()), &["veydan"])?;
        shared.encrypt_sender(&factory, sender);

        // The video track of this side, there from the start and disabled:
        // the first offer carries a video m-line (sendrecv, as the far end
        // answers it), and the camera later means frames, not an offer.
        let video_source = VideoSource::new(DEFAULT_VIDEO.0, DEFAULT_VIDEO.1, false, shared.local_frames.clone());
        let video_track = factory.create_video_track("video", video_source.inner.clone());
        video_track.set_enabled(false);
        let video_sender = pc.add_track(MediaStreamTrack::Video(video_track.clone()), &["veydan"])?;
        shared.encrypt_sender(&factory, video_sender.clone());

        let session = Session {
            pc,
            factory,
            audio_track,
            shared,
            events: Mutex::new(Some(events_rx)),
            audio_input: Mutex::new(audio_input),
            video_sender,
            video_track: Mutex::new(video_track),
            video_source: Mutex::new(video_source),
            remote: Mutex::new(Remote::default()),
            remote_frames: Mutex::new(Some(remote_frames)),
            remote_pump: Mutex::new(Some(remote_pump)),
        };
        session.wire_callbacks();
        Ok(session)
    }

    fn wire_callbacks(&self) {
        let pc = &self.pc;
        let shared = self.shared.clone();
        pc.on_ice_candidate(Some(Box::new(move |c| {
            tracing::debug!(candidate = %c.candidate(), "ice: local candidate");
            shared.send(SessionEvent::LocalCandidate(Candidate {
                sdp_mid: c.sdp_mid(),
                sdp_mline_index: c.sdp_mline_index(),
                candidate: c.candidate(),
            }));
        })));
        let shared = self.shared.clone();
        pc.on_ice_gathering_state_change(Some(Box::new(move |s| {
            if s == IceGatheringState::Complete {
                shared.send(SessionEvent::GatheringComplete);
            }
        })));
        let shared = self.shared.clone();
        pc.on_connection_state_change(Some(Box::new(move |s| {
            tracing::debug!(state = ?s, "connection state");
            shared.send(SessionEvent::ConnectionState(connection_state(s)));
        })));
        let shared = self.shared.clone();
        pc.on_ice_connection_state_change(Some(Box::new(move |s| {
            tracing::debug!(state = ?s, "ice state");
            if let Some(s) = ice_state(s) {
                shared.send(SessionEvent::IceState(s));
            }
        })));
        let shared = self.shared.clone();
        pc.on_negotiation_needed(Some(Box::new(move |_| {
            shared.send(SessionEvent::NegotiationNeeded);
        })));
        let shared = self.shared.clone();
        let factory = self.factory.clone();
        pc.on_track(Some(Box::new(move |ev| {
            shared.decrypt_receiver(&factory, ev.receiver.clone());
            match ev.track {
                MediaStreamTrack::Audio(track) => {
                    if shared.pushed {
                        let opts = NativeAudioStreamOptions { queue_size_frames: Some(audio::OUTPUT_QUEUE_FRAMES) };
                        let stream = NativeAudioStream::with_options(track, SAMPLE_RATE as i32, 1, opts);
                        *shared.audio_output.lock().unwrap_or_else(|e| e.into_inner()) =
                            Some(AudioOutput::new(stream, shared.apm.clone()));
                    }
                    shared.send(SessionEvent::RemoteAudio);
                }
                MediaStreamTrack::Video(track) => {
                    let stream = NativeVideoStream::with_options(track, NativeVideoStreamOptions { queue_size_frames: Some(video::OUTPUT_QUEUE_FRAMES) });
                    // The first video track is the far end's camera; a
                    // second one (none in a 1:1 call) would be left unread.
                    if let Some(tx) = shared.video_output.lock().unwrap_or_else(|e| e.into_inner()).take() {
                        let _ = tx.send(VideoOutput::new(stream));
                    }
                    shared.send(SessionEvent::RemoteVideo);
                }
            }
        })));
    }

    /// The events of this session, once: `None` the second time.
    pub fn events(&self) -> Option<UnboundedReceiver<SessionEvent>> {
        self.events.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    /// An offer for the other side, set as the local description. With
    /// `ice_restart` the candidates are gathered anew: the way to move to
    /// another network or relay in the middle of a call.
    pub async fn create_offer(&self, ice_restart: bool) -> Result<String> {
        // The default of the crate maps to the legacy offer_to_receive_audio
        // = 0, which makes the m-line sendonly and the far end's track never
        // arrives (REPORT.md, section 3). Video the same: both ways always.
        let options = OfferOptions { ice_restart, offer_to_receive_audio: true, offer_to_receive_video: true };
        let offer = self.pc.create_offer(options).await?;
        let sdp = offer.to_string();
        self.pc.set_local_description(offer).await?;
        Ok(sdp)
    }

    /// The other side's offer, as the remote description; an answer
    /// follows ([`Session::create_answer`]).
    pub async fn set_remote_offer(&self, sdp: &str) -> Result<()> {
        let offer = SessionDescription::parse(sdp, SdpType::Offer)?;
        self.pc.set_remote_description(offer).await?;
        self.flush_remote_candidates().await;
        Ok(())
    }

    /// The answer to the offer set with [`Session::set_remote_offer`],
    /// set as the local description: what goes back to the other side.
    pub async fn create_answer(&self) -> Result<String> {
        let answer = self.pc.create_answer(AnswerOptions::default()).await?;
        let sdp = answer.to_string();
        self.pc.set_local_description(answer).await?;
        Ok(sdp)
    }

    /// The other side's offer, answered in one go: the answer to send
    /// back.
    pub async fn accept_offer(&self, sdp: &str) -> Result<String> {
        self.set_remote_offer(sdp).await?;
        self.create_answer().await
    }

    /// The other side's answer to this side's offer.
    pub async fn set_remote_answer(&self, sdp: &str) -> Result<()> {
        let answer = SessionDescription::parse(sdp, SdpType::Answer)?;
        self.pc.set_remote_description(answer).await?;
        self.flush_remote_candidates().await;
        Ok(())
    }

    /// The same as [`Session::set_remote_answer`], by the name of the
    /// other half of [`Session::accept_offer`].
    pub async fn accept_answer(&self, sdp: &str) -> Result<()> {
        self.set_remote_answer(sdp).await
    }

    /// A candidate of the other side. Fine before its description arrived
    /// (the signaling may deliver the candidates first): the session keeps
    /// it and adds it once [`Session::set_remote_offer`] or
    /// [`Session::set_remote_answer`] went through. A candidate that does
    /// not parse is refused at once, kept or not.
    pub async fn add_remote_candidate(&self, candidate: &Candidate) -> Result<()> {
        let c = IceCandidate::parse(&candidate.sdp_mid, candidate.sdp_mline_index, &candidate.candidate)?;
        {
            let mut remote = self.remote.lock().unwrap_or_else(|e| e.into_inner());
            if !remote.described {
                remote.pending.push(candidate.clone());
                return Ok(());
            }
        }
        self.pc.add_ice_candidate(c).await?;
        Ok(())
    }

    /// How many candidates of the other side wait for its description.
    pub fn pending_remote_candidates(&self) -> usize {
        self.remote.lock().unwrap_or_else(|e| e.into_inner()).pending.len()
    }

    /// After a remote description was set: the candidates that waited for
    /// it go in, in the order they came. One libwebrtc refuses is logged
    /// and skipped, not an error of the description, which is in place.
    async fn flush_remote_candidates(&self) {
        let pending = {
            let mut remote = self.remote.lock().unwrap_or_else(|e| e.into_inner());
            remote.described = true;
            std::mem::take(&mut remote.pending)
        };
        for candidate in pending {
            let parsed = IceCandidate::parse(&candidate.sdp_mid, candidate.sdp_mline_index, &candidate.candidate);
            let added = match parsed {
                Ok(c) => self.pc.add_ice_candidate(c).await.map_err(Error::from),
                Err(e) => Err(e.into()),
            };
            if let Err(e) = added {
                tracing::warn!(candidate = %candidate.candidate, error = %e, "ice: a kept remote candidate was refused");
            }
        }
    }

    /// Tells ICE to gather anew on the next offer (`create_offer(true)`
    /// does it too).
    pub fn restart_ice(&self) {
        self.pc.restart_ice();
    }

    pub fn connection_state(&self) -> ConnectionState {
        connection_state(self.pc.connection_state())
    }

    pub fn ice_state(&self) -> Option<IceState> {
        ice_state(self.pc.ice_connection_state())
    }

    /// Ends the session. Dropping it does the same. The far end's frames
    /// end with it: a reader of [`Session::remote_video_frames`] gets
    /// `Closed` once the pump task let go of its sender (at once, or when
    /// the task next yields).
    pub fn close(&self) {
        self.pc.close();
        if let Some(pump) = self.remote_pump.lock().unwrap_or_else(|e| e.into_inner()).take() {
            pump.abort();
        }
        drop(self.remote_frames.lock().unwrap_or_else(|e| e.into_inner()).take());
    }

    /// Silence from this side, without a renegotiation.
    pub fn set_muted(&self, muted: bool) {
        self.audio_track.set_enabled(!muted);
    }

    pub fn muted(&self) -> bool {
        !self.audio_track.enabled()
    }

    /// The input of the pushed path, once; `None` on the device path or
    /// the second time.
    pub fn take_audio_input(&self) -> Option<AudioInput> {
        self.audio_input.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    /// The output of the pushed path, once the far end's audio track
    /// arrived ([`SessionEvent::RemoteAudio`]); `None` before, on the
    /// device path, or the second time.
    pub fn take_audio_output(&self) -> Option<AudioOutput> {
        self.shared.audio_output.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    /// This side's video goes (frames pushed into the source are encoded
    /// and sent) or not (nothing is sent, whatever is pushed). Without a
    /// renegotiation.
    pub fn set_video_enabled(&self, on: bool) {
        self.video_track.lock().unwrap_or_else(|e| e.into_inner()).set_enabled(on);
    }

    pub fn video_enabled(&self) -> bool {
        self.video_track.lock().unwrap_or_else(|e| e.into_inner()).enabled()
    }

    /// The source this side's video is pushed into now.
    pub fn video_source(&self) -> VideoSource {
        self.video_source.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// A fresh source for this side's video, of this size and kind (a
    /// screen asks the encoder for sharp text), on the same sender: the
    /// track is replaced, nothing renegotiated. What was pushed into the
    /// old source is no longer sent.
    pub fn replace_video_source(&self, width: u32, height: u32, screencast: bool) -> Result<VideoSource> {
        let source = VideoSource::new(width, height, screencast, self.shared.local_frames.clone());
        let track = self.factory.create_video_track("video", source.inner.clone());
        let mut current = self.video_track.lock().unwrap_or_else(|e| e.into_inner());
        track.set_enabled(current.enabled());
        self.video_sender.set_track(Some(MediaStreamTrack::Video(track.clone())))?;
        *current = track;
        *self.video_source.lock().unwrap_or_else(|e| e.into_inner()) = source.clone();
        Ok(source)
    }

    /// The most this side's video may spend, kbit/s; `None` leaves it to
    /// the engine's estimate of the way.
    pub fn set_video_max_bitrate(&self, max_kbps: Option<u32>) -> Result<()> {
        let mut params = self.video_sender.parameters();
        if params.encodings.is_empty() {
            // Before the first negotiation there is no encoding to cap
            // yet; the engine makes one with the description.
            return Ok(());
        }
        for e in &mut params.encodings {
            e.max_bitrate = max_kbps.map(|k| k as u64 * 1000);
        }
        self.video_sender.set_parameters(params)?;
        Ok(())
    }

    /// This side's own frames, as pushed into the source in use: for the
    /// small picture of oneself. A late reader skips to the newest.
    pub fn local_video_frames(&self) -> broadcast::Receiver<Arc<VideoFrame>> {
        self.shared.local_frames.subscribe()
    }

    /// The far end's frames, decoded, once they come. A late reader skips
    /// to the newest; every reader gets `Closed` once the session closed
    /// (a receiver that is closed already, after it).
    pub fn remote_video_frames(&self) -> broadcast::Receiver<Arc<VideoFrame>> {
        match self.remote_frames.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            Some(sender) => sender.subscribe(),
            None => broadcast::channel(1).1,
        }
    }

    /// Frame encryption on or off, both directions. On by default when
    /// the session was made with keys; nothing without them.
    pub fn set_encryption_enabled(&self, enabled: bool) {
        if let Some(e) = &self.shared.encryption {
            e.enabled.store(enabled, Ordering::SeqCst);
            for c in e.senders.lock().unwrap_or_else(|x| x.into_inner()).iter() {
                c.set_enabled(enabled);
            }
            for c in e.receivers.lock().unwrap_or_else(|x| x.into_inner()).iter() {
                c.set_enabled(enabled);
            }
        }
    }

    pub fn encryption_enabled(&self) -> bool {
        self.shared.encryption.as_ref().is_some_and(|e| e.enabled.load(Ordering::SeqCst))
    }

    /// The slot of [`crate::FrameKeys`] this side encrypts with from now
    /// on (the epoch). Receiving reads the slot from each frame.
    pub fn set_key_index(&self, index: u8) {
        if let Some(e) = &self.shared.encryption {
            *e.key_index.lock().unwrap_or_else(|x| x.into_inner()) = index;
            for c in e.senders.lock().unwrap_or_else(|x| x.into_inner()).iter() {
                c.set_key_index(index as i32);
            }
        }
    }

    /// A reading of the statistics.
    pub async fn stats(&self) -> Result<Stats> {
        let all = self.pc.get_stats().await?;
        Ok(stats_of(&all))
    }

    /// The pair of candidates in use, once ICE chose one.
    pub async fn path(&self) -> Result<Option<Path>> {
        Ok(self.stats().await?.path)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.close();
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").field("state", &self.connection_state()).finish()
    }
}

impl Shared {
    fn send(&self, event: SessionEvent) {
        // Nobody listening is fine: the session goes on.
        let _ = self.events.send(event);
    }

    /// A cryptor on a sender of this side, when the session encrypts.
    fn encrypt_sender(&self, factory: &PeerConnectionFactory, sender: RtpSender) {
        let Some(e) = &self.encryption else { return };
        let cryptor = FrameCryptor::new_for_rtp_sender(
            factory,
            e.participant.clone(),
            EncryptionAlgorithm::AesGcm,
            e.keys.provider.clone(),
            sender,
        );
        self.watch(&cryptor);
        cryptor.set_key_index(*e.key_index.lock().unwrap_or_else(|x| x.into_inner()) as i32);
        cryptor.set_enabled(e.enabled.load(Ordering::SeqCst));
        e.senders.lock().unwrap_or_else(|x| x.into_inner()).push(cryptor);
    }

    /// A cryptor on a receiver of the far end's, when the session
    /// encrypts. The far end is named "peer": with a shared key the name
    /// takes part in nothing but the state events.
    fn decrypt_receiver(&self, factory: &PeerConnectionFactory, receiver: libwebrtc::rtp_receiver::RtpReceiver) {
        let Some(e) = &self.encryption else { return };
        let cryptor = FrameCryptor::new_for_rtp_receiver(
            factory,
            "peer".to_string(),
            EncryptionAlgorithm::AesGcm,
            e.keys.provider.clone(),
            receiver,
        );
        self.watch(&cryptor);
        cryptor.set_enabled(e.enabled.load(Ordering::SeqCst));
        e.receivers.lock().unwrap_or_else(|x| x.into_inner()).push(cryptor);
    }

    fn watch(&self, cryptor: &FrameCryptor) {
        let events = self.events.clone();
        cryptor.on_state_change(Some(Box::new(move |participant, state| {
            tracing::debug!(%participant, ?state, "frame cryptor");
            let _ = events.send(SessionEvent::Encryption { participant, state: state.into() });
        })));
    }
}

fn connection_state(s: PeerConnectionState) -> ConnectionState {
    match s {
        PeerConnectionState::New => ConnectionState::New,
        PeerConnectionState::Connecting => ConnectionState::Connecting,
        PeerConnectionState::Connected => ConnectionState::Connected,
        PeerConnectionState::Disconnected => ConnectionState::Disconnected,
        PeerConnectionState::Failed => ConnectionState::Failed,
        PeerConnectionState::Closed => ConnectionState::Closed,
    }
}

fn ice_state(s: IceConnectionState) -> Option<IceState> {
    Some(match s {
        IceConnectionState::New => IceState::New,
        IceConnectionState::Checking => IceState::Checking,
        IceConnectionState::Connected => IceState::Connected,
        IceConnectionState::Completed => IceState::Completed,
        IceConnectionState::Failed => IceState::Failed,
        IceConnectionState::Disconnected => IceState::Disconnected,
        IceConnectionState::Closed => IceState::Closed,
        IceConnectionState::Max => return None,
    })
}

fn candidate_kind(t: Option<IceCandidateType>) -> CandidateKind {
    match t {
        Some(IceCandidateType::Srflx) => CandidateKind::ServerReflexive,
        Some(IceCandidateType::Prflx) => CandidateKind::PeerReflexive,
        Some(IceCandidateType::Relay) => CandidateKind::Relay,
        Some(IceCandidateType::Host) | None => CandidateKind::Host,
    }
}

/// The reading out of everything libwebrtc reports.
fn stats_of(all: &[RtcStats]) -> Stats {
    let mut out = Stats::default();
    // The transport knows the pair in use and counts across every pair it
    // had; a pair's own counters start again when ICE moves to another.
    let transport = all.iter().find_map(|s| match s {
        RtcStats::Transport(t) => Some(&t.transport),
        _ => None,
    });
    let pair = all.iter().find_map(|s| match s {
        RtcStats::CandidatePair(p)
            if transport.is_some_and(|t| t.selected_candidate_pair_id == p.rtc.id)
                || (transport.is_none() && p.candidate_pair.nominated) =>
        {
            Some(p)
        }
        _ => None,
    });
    if let Some(t) = transport {
        out.bytes_sent = t.bytes_sent;
        out.bytes_received = t.bytes_received;
        out.packets_sent = t.packets_sent;
        out.packets_received = t.packets_received;
    }
    if let Some(p) = pair {
        let pair = &p.candidate_pair;
        let find = |id: &str| {
            all.iter().find_map(|s| match s {
                RtcStats::LocalCandidate(c) if c.rtc.id == id => Some(&c.local_candidate),
                RtcStats::RemoteCandidate(c) if c.rtc.id == id => Some(&c.remote_candidate),
                _ => None,
            })
        };
        // A round trip of exactly zero is "not measured yet".
        let rtt_ms = (pair.current_round_trip_time > 0.0).then_some(pair.current_round_trip_time * 1000.0);
        out.rtt_ms = rtt_ms;
        if transport.is_none() {
            out.bytes_sent = pair.bytes_sent;
            out.bytes_received = pair.bytes_received;
            out.packets_sent = pair.packets_sent;
            out.packets_received = pair.packets_received;
        }
        out.available_outgoing_kbps = pair.available_outgoing_bitrate / 1000.0;
        if let (Some(local), Some(remote)) = (find(&pair.local_candidate_id), find(&pair.remote_candidate_id)) {
            out.path = Some(Path {
                local: candidate_kind(local.candidate_type),
                remote: candidate_kind(remote.candidate_type),
                local_addr: format!("{}:{}", local.address, local.port),
                remote_addr: format!("{}:{}", remote.address, remote.port),
                protocol: local.protocol.clone(),
                rtt_ms,
            });
        }
    }
    for s in all {
        match s {
            RtcStats::InboundRtp(r) => {
                out.packets_lost += r.received.packets_lost;
                out.jitter_ms = out.jitter_ms.max(r.received.jitter * 1000.0);
                out.audio_level_in = out.audio_level_in.max(r.inbound.audio_level);
            }
            RtcStats::RemoteInboundRtp(r) => {
                out.fraction_lost = out.fraction_lost.max(r.remote_inbound.fraction_lost);
            }
            RtcStats::MediaSource(m) => {
                out.audio_level_out = out.audio_level_out.max(m.audio.audio_level);
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_relayed_when_either_end_is() {
        let direct = Path {
            local: CandidateKind::Host,
            remote: CandidateKind::ServerReflexive,
            local_addr: "10.0.0.2:1".into(),
            remote_addr: "203.0.113.7:2".into(),
            protocol: "udp".into(),
            rtt_ms: Some(12.0),
        };
        assert!(!direct.is_relayed());
        assert!(Path { remote: CandidateKind::Relay, ..direct.clone() }.is_relayed());
        assert!(Path { local: CandidateKind::Relay, ..direct }.is_relayed());
    }

    #[test]
    fn the_wire_form_of_a_candidate_and_a_policy() {
        let c = Candidate { sdp_mid: "0".into(), sdp_mline_index: 0, candidate: "candidate:1 1 udp 2 10.0.0.2 1 typ host".into() };
        let text = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<Candidate>(&text).unwrap(), c);
        assert_eq!(serde_json::to_string(&IcePolicy::RelayOnly).unwrap(), "\"relay_only\"");
        let server: IceServer = serde_json::from_str(r#"{"urls":["stun:203.0.113.7:3478"]}"#).unwrap();
        assert_eq!(server.username, "");
    }
}
