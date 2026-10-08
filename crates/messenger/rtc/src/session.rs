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
//!
//! # A room (the SFU of a group call)
//!
//! A session made with [`SessionConfig::room`] is one participant's leg
//! to a call node (services/call/spec/protocol.md, "Комнаты (SFU)"):
//!
//! ```text
//! create_offer(false)  ─▶  sendonly audio, sendonly video (3 layers q/h/f
//!                          when `simulcast`), m=application with the data
//!                          channel `data_label` this side opens
//! set_remote_answer    ◀─  the node's answer (JOIN over its control
//!                          channel): recvonly m-lines, the node's own
//!                          candidates (ICE-lite: ours are not needed)
//! events: DataOpen     ─▶  the node's `hello` follows as Data text
//! events: Data(offer)  ─▶  set_remote_offer: new recvonly m-lines, one
//!                          per stream of each other participant;
//!                          events: RemoteTrack{mid, kind} as they appear,
//!                          RemoteTrackGone{mid} for an m-line closed
//!                          (port 0); create_answer ─▶ send_data(answer)
//! ```
//!
//! The node says whose stream an m-line carries (`tracks` of its offer);
//! the session knows the m-lines by their mid: the frames of a video
//! come out of [`Session::remote_video_frames_of`], the sound of the
//! pushed path out of [`Session::take_audio_output_of`] (on the device
//! path the engine mixes every remote audio track into the speaker), and
//! the key that decrypts a stream goes with its mid
//! ([`Session::set_receiver_key`]). After the join this side never
//! offers again: the node does, and this side answers.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use libwebrtc::audio_source::native::NativeAudioSource;
use libwebrtc::audio_source::AudioSourceOptions;
use libwebrtc::audio_stream::native::{NativeAudioStream, NativeAudioStreamOptions};
use libwebrtc::audio_track::RtcAudioTrack;
use libwebrtc::data_channel::{DataChannel, DataChannelInit, DataChannelState};
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
use libwebrtc::rtp_parameters::{Priority, RtpEncodingParameters};
use libwebrtc::rtp_sender::RtpSender;
use libwebrtc::rtp_transceiver::{RtpTransceiverDirection, RtpTransceiverInit};
use libwebrtc::session_description::{SdpType, SessionDescription};
use libwebrtc::stats::{IceCandidateType, RtcStats};
use libwebrtc::video_stream::native::{NativeVideoStream, NativeVideoStreamOptions};
use libwebrtc::video_track::RtcVideoTrack;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::{broadcast, oneshot};
use tokio::task::JoinHandle;

use crate::audio::{self, Apm, AudioEnd, AudioInput, AudioOutput, SAMPLE_RATE};
use crate::crypto::{self, Encryption, EncryptionState};
use crate::engine::{AudioMode, Inner as EngineInner};
use crate::video::{self, SizeHook, VideoFrame, VideoOutput, VideoSource};
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

/// What a session of a room is made with, beyond what every session has
/// (the module's "A room").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomConfig {
    /// The label of the data channel this side opens before its offer
    /// (`ctl` of a call node).
    pub data_label: String,
    /// Three layers of this side's video, RIDs `q`, `h` and `f` (a
    /// quarter, a half and the full size of the source;
    /// [`Session::set_video_layers`] turns the upper ones off). Only with
    /// a node whose welcome lists `simulcast`: one without it answers
    /// the three layers as one stream and forwards them mixed.
    pub simulcast: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SessionConfig {
    pub ice_servers: Vec<IceServer>,
    pub policy: IcePolicy,
    /// Encrypt the frames of every track with these keys (groups).
    pub encryption: Option<Encryption>,
    /// A leg to the SFU of a call node instead of a call between two.
    pub room: Option<RoomConfig>,
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

/// The kind of a track of a room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Audio,
    Video,
}

impl TrackKind {
    /// `audio` or `video`, as the node and the statistics name them.
    pub fn parse(s: &str) -> Option<TrackKind> {
        match s {
            "audio" => Some(TrackKind::Audio),
            "video" => Some(TrackKind::Video),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TrackKind::Audio => "audio",
            TrackKind::Video => "video",
        }
    }
}

/// The topmost layer of this side's simulcast video that is sent: `Low`
/// is the smallest layer alone (`q`), `Medium` adds the next (`h`),
/// `High` sends every layer the source is big enough for (`f` too). The
/// node or the bandwidth decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoLayer {
    Low,
    Medium,
    High,
}

impl VideoLayer {
    fn rank(self) -> usize {
        match self {
            VideoLayer::Low => 0,
            VideoLayer::Medium => 1,
            VideoLayer::High => 2,
        }
    }
}

/// The three layers: RID, the most each may spend when the caller caps
/// the video (kbit/s; without a cap libwebrtc's own table applies). The
/// lowest first: libwebrtc wants its simulcast streams in ascending
/// order, and keeps the first alone when the answer takes no simulcast.
const SIMULCAST_LAYERS: [(&str, u32, VideoLayer); 3] =
    [("q", 150, VideoLayer::Low), ("h", 500, VideoLayer::Medium), ("f", 1500, VideoLayer::High)];

/// The layer of a RID, if it is one of ours.
fn layer_of_rid(rid: &str) -> Option<(u32, VideoLayer)> {
    SIMULCAST_LAYERS.iter().find(|(r, ..)| *r == rid).map(|(_, kbps, layer)| (*kbps, *layer))
}

/// How many layers libwebrtc encodes for frames of this size: its table
/// of simulcast formats goes by the count of pixels (`kSimulcastFormats`,
/// `FindSimulcastFormatIndex`, `LimitSimulcastLayerCount`) and gives
/// three from 960×540, two from 480×270 and one below, whichever way
/// the frame is turned (720×1280 of a phone held upright is three). A
/// layer past that count is never encoded, whatever it is told. So the
/// layers that go are scaled from the top: the topmost of them at the
/// full size of the frames, each below at half of the one above
/// (640×360: `q` 320×180 and `h` 640×360, `f` off; 1280×720: 320×180,
/// 640×360, 1280×720).
///
/// libwebrtc may allow one layer more within a tenth below a step of
/// its table (`WebRTC-SimulcastLayerLimitRoundUp`); this counts the
/// plain step, the safe side: a layer counted here that libwebrtc does
/// not encode would be the one at the full size.
fn layers_for_size(width: u32, height: u32) -> usize {
    let pixels = u64::from(width) * u64::from(height);
    if pixels >= 960 * 540 {
        3
    } else if pixels >= 480 * 270 {
        2
    } else {
        1
    }
}

/// One layer of this side's video as the sender has it
/// ([`Session::video_encodings`]).
#[derive(Debug, Clone, PartialEq)]
pub struct VideoEncoding {
    /// Empty for a single stream.
    pub rid: String,
    pub active: bool,
    /// The most it may spend, kbit/s, when capped.
    pub max_kbps: Option<u32>,
    /// How many times smaller than the frames it is sent, when set.
    pub scale_down: Option<f64>,
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
    /// The far end's audio track arrived (a call between two); on the
    /// pushed path [`Session::take_audio_output`] has frames from now on.
    RemoteAudio,
    /// The far end's video track arrived (with its description: every
    /// call has one); [`Session::remote_video_frames`] gives frames once
    /// the far end sends any.
    RemoteVideo,
    /// The engine wants a new offer (a track was added after the first
    /// negotiation).
    NegotiationNeeded,
    /// The cryptor of one direction changed state; `participant` is this
    /// side's name for a sender, the other side's (`peer`) for the
    /// receiver of a call between two.
    Encryption { participant: String, state: EncryptionState },

    // ─── A room ──────────────────────────────────────────────────────
    /// The data channel of the room is open both ways.
    DataOpen { label: String },
    DataClosed { label: String },
    /// A message came on the data channel: text (the node's JSON) or
    /// bytes (relayed from another participant, its id in front).
    Data { label: String, binary: bool, data: Vec<u8> },
    /// A stream of another participant appeared on the m-line `mid` of
    /// the node's offer (the offer's `tracks` say whose). On the pushed
    /// path [`Session::take_audio_output_of`] has its sound from now
    /// on; [`Session::remote_video_frames_of`] its frames.
    RemoteTrack { mid: String, kind: TrackKind },
    /// The m-line `mid` was closed by a later offer (its participant
    /// left): its outputs end.
    RemoteTrackGone { mid: String },
    /// The frames of the video on `mid` come at this size now.
    RemoteVideoSize { mid: String, width: u32, height: u32 },
    /// The cryptor of the stream on `mid` changed state.
    RemoteEncryption { mid: String, state: EncryptionState },
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

/// One RTP stream of the statistics: an m-line of a room (and, for this
/// side's simulcast video, one of its layers by `rid`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackStats {
    pub mid: String,
    pub kind: Option<TrackKind>,
    /// The layer of a simulcast sender; empty otherwise.
    pub rid: String,
    pub bytes: u64,
    pub packets: u64,
    /// What never came (inbound) as RTP counts it.
    pub packets_lost: i64,
    pub jitter_ms: f64,
    /// Level of the sound, 0.0–1.0 (inbound audio).
    pub audio_level: f64,
    /// The size of the frames sent or decoded, once there are any.
    pub width: u32,
    pub height: u32,
    /// Whether a sender's layer is on.
    pub active: bool,
    /// What holds a sender's quality back now: `none`, `cpu`,
    /// `bandwidth` or `other` (empty for what comes in).
    pub quality_limitation: String,
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
    /// Every stream that comes in, by mid (a room has one per stream of
    /// each other participant).
    pub inbound: Vec<TrackStats>,
    /// Every stream that goes out: one per track, or one per layer of a
    /// simulcast video.
    pub outbound: Vec<TrackStats>,
}

/// What the callbacks of libwebrtc reach: the events channel, the
/// outputs that appear with the far end's tracks, the cryptors.
struct Shared {
    events: UnboundedSender<SessionEvent>,
    /// A session of a room: the far end's tracks are many and known by
    /// their mid.
    room: bool,
    /// The runtime the session was made in: the pumps of the remote
    /// videos of a room are spawned on it from libwebrtc's threads.
    runtime: tokio::runtime::Handle,
    audio_output: Mutex<Option<AudioOutput>>,
    /// Where the far end's video goes once its track arrives: to the task
    /// that pumps it into `remote_frames`.
    video_output: Mutex<Option<oneshot::Sender<VideoOutput>>>,
    /// The sound of every remote track of a room, by mid, until taken
    /// (the pushed path).
    audio_outputs: Mutex<HashMap<String, AudioOutput>>,
    /// The ends of the outputs of the pushed path, taken or not: by mid
    /// in a room, `peer` in a call between two. Pulled when the track's
    /// m-line is closed, dropped when the session is: either way the
    /// output's reader gets `None`.
    audio_ends: Mutex<HashMap<String, AudioEnd>>,
    /// The frames of every remote video of a room, by mid.
    remote_videos: Mutex<HashMap<String, RemoteVideo>>,
    /// The remote tracks of a room that are there now, by mid.
    mids: Mutex<HashMap<String, TrackKind>>,
    /// This side's own frames, as pushed into the source in use.
    local_frames: broadcast::Sender<Arc<VideoFrame>>,
    /// Frames of the far end are handed out (the pushed path) rather than
    /// played by the device.
    pushed: bool,
    apm: Option<Arc<Apm>>,
    encryption: Option<EncryptionShared>,
}

/// The decoded frames of one remote video of a room: the broadcast its
/// readers subscribe to, and the task that fills it (see
/// `Session::remote_pump` for why a pump is aborted, not awaited).
struct RemoteVideo {
    frames: broadcast::Sender<Arc<VideoFrame>>,
    pump: Option<JoinHandle<()>>,
}

impl RemoteVideo {
    fn new() -> RemoteVideo {
        RemoteVideo { frames: broadcast::channel(video::FAN_OUT_FRAMES).0, pump: None }
    }
}

impl Drop for RemoteVideo {
    fn drop(&mut self) {
        if let Some(pump) = self.pump.take() {
            pump.abort();
        }
    }
}

/// This side's video as it is sent: the source the frames are pushed
/// into, the sender they go out on, and the cap and the topmost layer
/// asked for. Shared with the source, which tells it the size of the
/// frames pushed: the layers of a simulcast follow that size, not the
/// one the source was made for (a camera gives the nearest size it has;
/// a phone pushes whatever CameraX delivers into the default source).
struct VideoSending {
    /// The sender, there from the start (set as the track goes on the
    /// connection, right after this is made); its track is replaced
    /// when the source changes (camera to screen).
    sender: OnceLock<RtpSender>,
    source: Mutex<VideoSource>,
    cap_kbps: Mutex<Option<u32>>,
    top: Mutex<VideoLayer>,
    /// Held over every `parameters()` + `set_parameters()` pair on the
    /// sender. libwebrtc takes a `SetParameters` only with the
    /// transaction id of the last `GetParameters` (`RtpSenderBase::
    /// CheckSetParameters`, INVALID_MODIFICATION otherwise), and in a
    /// room the pair runs on two threads at once: the size hook on the
    /// pusher's thread ([`VideoSending::frames_came_at`]) and the core's
    /// task setting the cap or the layers. Interleaved, the second of
    /// the two is refused; serialized, both go in, the later one on top.
    parameters: Mutex<()>,
    /// Whether the room was joined with the three layers.
    simulcast: bool,
    /// This side's own frames, as pushed into the source in use (the
    /// fan-out every source of this session is made with).
    local_frames: broadcast::Sender<Arc<VideoFrame>>,
}

impl VideoSending {
    fn new(simulcast: bool, local_frames: broadcast::Sender<Arc<VideoFrame>>) -> Arc<VideoSending> {
        Arc::new_cyclic(|weak| VideoSending {
            sender: OnceLock::new(),
            source: Mutex::new(VideoSending::make_source(weak, &local_frames, DEFAULT_VIDEO.0, DEFAULT_VIDEO.1, false)),
            cap_kbps: Mutex::new(None),
            top: Mutex::new(VideoLayer::High),
            parameters: Mutex::new(()),
            simulcast,
            local_frames,
        })
    }

    fn sender(&self) -> &RtpSender {
        self.sender.get().expect("the video sender is set as the session is made")
    }

    fn source(&self) -> VideoSource {
        self.source.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// A source that tells this of the size of its frames.
    fn make_source(weak: &Weak<VideoSending>, local_frames: &broadcast::Sender<Arc<VideoFrame>>, width: u32, height: u32, screencast: bool) -> VideoSource {
        let weak = weak.clone();
        let hook: SizeHook = Arc::new(move |source, width, height| {
            if let Some(video) = weak.upgrade() {
                video.frames_came_at(source, width, height);
            }
        });
        VideoSource::new(width, height, screencast, local_frames.clone(), Some(hook))
    }

    /// A fresh source in place of the current one (`Session::replace_video_source`).
    fn replace_source(self: &Arc<Self>, width: u32, height: u32, screencast: bool) -> VideoSource {
        let source = VideoSending::make_source(&Arc::downgrade(self), &self.local_frames, width, height, screencast);
        *self.source.lock().unwrap_or_else(|e| e.into_inner()) = source.clone();
        source
    }

    /// The frames pushed into `source` changed size: the layers of a
    /// simulcast are set for the new one, when `source` is still the one
    /// in use (a capture that pushes on into a replaced source changes
    /// nothing). On the pusher's thread; a change of size is rare (the
    /// camera went on, the phone was turned), the setting of the
    /// parameters is a hop to the signaling thread.
    fn frames_came_at(&self, source: &VideoSource, width: u32, height: u32) {
        if !self.simulcast || !self.source.lock().unwrap_or_else(|e| e.into_inner()).is_same(source) {
            return;
        }
        tracing::debug!(width, height, layers = layers_for_size(width, height), "video: the frames pushed changed size");
        if let Err(e) = self.apply_parameters() {
            tracing::warn!(error = %e, "the layers of the video could not follow the size of its frames");
        }
    }

    /// The size the layers are made for: that of the frames pushed into
    /// the source in use, or the size it was made for before the first.
    fn frame_size(&self) -> (u32, u32) {
        let source = self.source.lock().unwrap_or_else(|e| e.into_inner());
        source.frame_size().unwrap_or_else(|| source.resolution())
    }

    /// The cap, the layers the frames are big enough for
    /// ([`layers_for_size`]) and the topmost one asked for, onto the
    /// sender's encodings. Again whenever any of the three changes; two
    /// at once go in one after the other (`parameters`), each with the
    /// three as they are when its turn comes.
    fn apply_parameters(&self) -> Result<()> {
        let _turn = self.parameters.lock().unwrap_or_else(|e| e.into_inner());
        let cap = *self.cap_kbps.lock().unwrap_or_else(|e| e.into_inner());
        let top = *self.top.lock().unwrap_or_else(|e| e.into_inner());
        let (width, height) = self.frame_size();
        let going = layers_for_size(width, height);
        let sender = self.sender();
        let mut params = sender.parameters();
        if params.encodings.is_empty() {
            // Before the first negotiation there is no encoding to cap
            // yet; the engine makes one with the description.
            return Ok(());
        }
        let layers = params.encodings.iter().filter(|e| layer_of_rid(&e.rid).is_some()).count();
        for e in &mut params.encodings {
            match layer_of_rid(&e.rid) {
                Some((ceiling, layer)) if layers > 1 => {
                    let rank = layer.rank();
                    let fits = rank < going;
                    e.active = fits && layer <= top;
                    if fits {
                        e.scale_resolution_down_by = Some(f64::from(1u32 << (going - 1 - rank)));
                    }
                    e.max_bitrate = cap.map(|c| u64::from(c.min(ceiling)) * 1000);
                }
                // One stream, with or without a RID (the answer took no
                // simulcast): the cap alone.
                _ => e.max_bitrate = cap.map(|k| u64::from(k) * 1000),
            }
        }
        sender.set_parameters(params)?;
        Ok(())
    }
}

struct EncryptionShared {
    keys: crate::FrameKeys,
    participant: String,
    enabled: AtomicBool,
    key_index: Mutex<u8>,
    senders: Mutex<Vec<FrameCryptor>>,
    /// The cryptor of every receiver, by the mid of its m-line (`peer`
    /// in a call between two).
    receivers: Mutex<Vec<(String, FrameCryptor)>>,
}

/// The far end's candidates that came before the description they belong
/// to, and what is known of its descriptions. libwebrtc answers a
/// candidate added before the remote description with an error
/// (`AddIceCandidate` fails with "no remote description",
/// `api/uma_metrics.h`) and forgets it; a candidate of an ICE generation
/// it does not know yet (its `ufrag` is of a restart offer or answer
/// still on its way: the signaling keeps no order) it drops without a
/// word (`P2PTransportChannel::AddRemoteCandidate`, "unknown ufrag"). So
/// the session keeps such candidates until the description with that
/// ufrag is set, and adds them then.
#[derive(Default)]
struct Remote {
    described: bool,
    /// The `a=ice-ufrag` of every remote description set so far (one per
    /// ICE generation; libwebrtc keeps the older ones too).
    ufrags: HashSet<String>,
    pending: Vec<Candidate>,
}

impl Remote {
    /// Whether libwebrtc would take this candidate now: a description is
    /// there, and the generation it names (if it names one) is known.
    fn takes(&self, candidate: &Candidate) -> bool {
        self.described && candidate_ufrag(&candidate.candidate).is_none_or(|u| self.ufrags.contains(u))
    }
}

/// The `ufrag` a candidate line names (libwebrtc writes `... generation 0
/// ufrag abcd network-id 1`), if any.
fn candidate_ufrag(line: &str) -> Option<&str> {
    let mut words = line.split_ascii_whitespace();
    while let Some(w) = words.next() {
        if w == "ufrag" {
            return words.next();
        }
    }
    None
}

/// Every `a=ice-ufrag:` of an SDP (the same for every bundled m-line).
fn sdp_ufrags(sdp: &str) -> impl Iterator<Item = String> + '_ {
    sdp.lines().filter_map(|l| l.trim_end().strip_prefix("a=ice-ufrag:")).map(|u| u.trim().to_string())
}

/// The mids of the m-sections `sdp` closes (port 0: the node closes the
/// m-line of a participant who left this way).
fn closed_mids(sdp: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut closed = false;
    for line in sdp.lines().map(str::trim_end) {
        if line.starts_with("m=") {
            closed = line.split(' ').nth(1) == Some("0");
        } else if closed {
            if let Some(mid) = line.strip_prefix("a=mid:") {
                out.push(mid.trim().to_string());
            }
        }
    }
    out
}

/// Whether `sdp` takes (or offers) simulcast on any m-line.
fn sdp_has_simulcast(sdp: &str) -> bool {
    sdp.lines().any(|l| l.starts_with("a=simulcast:"))
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
    video_track: Mutex<RtcVideoTrack>,
    /// This side's video as it is sent: the source, the sender, the cap
    /// and the layers.
    video: Arc<VideoSending>,
    /// The room this session is a leg of, if any, and its data channel.
    room: Option<RoomConfig>,
    data: Option<DataChannel>,
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
    remote_pump: Mutex<Option<JoinHandle<()>>>,
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
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| Error::State("a session is made inside a tokio runtime".into()))?;

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
            room: config.room.is_some(),
            runtime,
            audio_output: Mutex::new(None),
            video_output: Mutex::new(Some(video_output_tx)),
            audio_outputs: Mutex::new(HashMap::new()),
            audio_ends: Mutex::new(HashMap::new()),
            remote_videos: Mutex::new(HashMap::new()),
            mids: Mutex::new(HashMap::new()),
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
        // The video track of this side, there from the start and disabled:
        // the first offer carries a video m-line (sendrecv, as the far end
        // answers it), and the camera later means frames, not an offer.
        let video = VideoSending::new(config.room.as_ref().is_some_and(|r| r.simulcast), shared.local_frames.clone());
        let video_track = factory.create_video_track("video", video.source().inner.clone());
        video_track.set_enabled(false);

        let (audio_sender, video_sender, data) = match &config.room {
            None => {
                let audio_sender = pc.add_track(MediaStreamTrack::Audio(audio_track.clone()), &["veydan"])?;
                let video_sender = pc.add_track(MediaStreamTrack::Video(video_track.clone()), &["veydan"])?;
                (audio_sender, video_sender, None)
            }
            Some(room) => {
                // A room: this side sends only, on transceivers of its own
                // (the node answers recvonly); what the others send comes
                // on m-lines of the node's offers. The data channel is
                // made before the offer, so that the offer carries it.
                let init = |send_encodings| RtpTransceiverInit {
                    direction: RtpTransceiverDirection::SendOnly,
                    stream_ids: vec!["veydan".into()],
                    send_encodings,
                };
                let audio = pc.add_transceiver(MediaStreamTrack::Audio(audio_track.clone()), init(vec![]))?;
                let encodings = if room.simulcast {
                    SIMULCAST_LAYERS
                        .iter()
                        .enumerate()
                        .map(|(i, (rid, _, _))| RtpEncodingParameters {
                            active: true,
                            max_bitrate: None,
                            max_framerate: None,
                            priority: Priority::Low,
                            rid: (*rid).into(),
                            // A quarter, a half and the full size for now;
                            // the answer brings the size of the source in
                            // (`apply_video_parameters`).
                            scale_resolution_down_by: Some(f64::from(1u32 << (2 - i as u32))),
                            scalability_mode: None,
                            has_ssrc: false,
                            ssrc: 0,
                        })
                        .collect()
                } else {
                    vec![]
                };
                let video = pc.add_transceiver(MediaStreamTrack::Video(video_track.clone()), init(encodings))?;
                let data = pc.create_data_channel(&room.data_label, DataChannelInit::default())?;
                (audio.sender(), video.sender(), Some(data))
            }
        };
        shared.encrypt_sender(&factory, audio_sender);
        shared.encrypt_sender(&factory, video_sender.clone());
        assert!(video.sender.set(video_sender).is_ok(), "the one sender of a new session");

        let session = Session {
            pc,
            factory,
            audio_track,
            shared,
            events: Mutex::new(Some(events_rx)),
            audio_input: Mutex::new(audio_input),
            video_track: Mutex::new(video_track),
            video,
            room: config.room,
            data,
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
            if shared.room {
                let mid = ev.transceiver.mid().unwrap_or_default();
                shared.decrypt_receiver(&factory, ev.receiver.clone(), &mid);
                shared.remote_track(mid, ev.track);
                return;
            }
            shared.decrypt_receiver(&factory, ev.receiver.clone(), "peer");
            match ev.track {
                MediaStreamTrack::Audio(track) => {
                    if shared.pushed {
                        let opts = NativeAudioStreamOptions { queue_size_frames: Some(audio::OUTPUT_QUEUE_FRAMES) };
                        let stream = NativeAudioStream::with_options(track, SAMPLE_RATE as i32, 1, opts);
                        let (end, output) = AudioOutput::new(stream, shared.apm.clone());
                        shared.audio_ends.lock().unwrap_or_else(|e| e.into_inner()).insert("peer".into(), end);
                        *shared.audio_output.lock().unwrap_or_else(|e| e.into_inner()) = Some(output);
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
        if let (Some(data), Some(room)) = (&self.data, &self.room) {
            let shared = self.shared.clone();
            let label = room.data_label.clone();
            data.on_state_change(Some(Box::new(move |state| {
                tracing::debug!(%label, ?state, "data channel");
                match state {
                    DataChannelState::Open => shared.send(SessionEvent::DataOpen { label: label.clone() }),
                    DataChannelState::Closed => shared.send(SessionEvent::DataClosed { label: label.clone() }),
                    DataChannelState::Connecting | DataChannelState::Closing => {}
                }
            })));
            let shared = self.shared.clone();
            let label = room.data_label.clone();
            data.on_message(Some(Box::new(move |buffer| {
                shared.send(SessionEvent::Data { label: label.clone(), binary: buffer.binary, data: buffer.data.to_vec() });
            })));
        }
    }

    /// The events of this session, once: `None` the second time.
    pub fn events(&self) -> Option<UnboundedReceiver<SessionEvent>> {
        self.events.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    /// An offer for the other side, set as the local description. With
    /// `ice_restart` the candidates are gathered anew: the way to move to
    /// another network or relay in the middle of a call.
    ///
    /// In a room it is the one offer of the join: sendonly m-lines and
    /// the data channel, nothing to receive (that comes on the node's
    /// offers).
    pub async fn create_offer(&self, ice_restart: bool) -> Result<String> {
        // The default of the crate maps to the legacy offer_to_receive_audio
        // = 0, which makes the m-line sendonly and the far end's track never
        // arrives (REPORT.md, section 3). Video the same: both ways always.
        // A room wants exactly that: this side sends only.
        let receive = self.room.is_none();
        let options = OfferOptions { ice_restart, offer_to_receive_audio: receive, offer_to_receive_video: receive };
        let offer = self.pc.create_offer(options).await?;
        let sdp = offer.to_string();
        self.pc.set_local_description(offer).await?;
        Ok(sdp)
    }

    /// The other side's offer, as the remote description; an answer
    /// follows ([`Session::create_answer`]). In a room: an offer of the
    /// node, whose new m-lines come as [`SessionEvent::RemoteTrack`]
    /// while it is set, and whose closed ones (port 0) end as
    /// [`SessionEvent::RemoteTrackGone`].
    pub async fn set_remote_offer(&self, sdp: &str) -> Result<()> {
        let offer = SessionDescription::parse(sdp, SdpType::Offer)?;
        let closed = if self.room.is_some() { closed_mids(sdp) } else { Vec::new() };
        self.pc.set_remote_description(offer).await?;
        self.flush_remote_candidates(sdp).await;
        for mid in closed {
            self.shared.forget_track(&mid);
        }
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

    /// The other side's answer to this side's offer. In a room: the
    /// node's answer to the join; a node that took no simulcast gets
    /// this side's one layer at the full size, not the quarter.
    pub async fn set_remote_answer(&self, sdp: &str) -> Result<()> {
        let answer = SessionDescription::parse(sdp, SdpType::Answer)?;
        self.pc.set_remote_description(answer).await?;
        self.flush_remote_candidates(sdp).await;
        if self.room.as_ref().is_some_and(|r| r.simulcast) {
            if sdp_has_simulcast(sdp) {
                self.apply_video_parameters()?;
            } else {
                self.single_layer_at_full_size();
            }
        }
        Ok(())
    }

    /// The same as [`Session::set_remote_answer`], by the name of the
    /// other half of [`Session::accept_offer`].
    pub async fn accept_answer(&self, sdp: &str) -> Result<()> {
        self.set_remote_answer(sdp).await
    }

    /// A candidate of the other side. Fine before its description arrived
    /// (the signaling may deliver the candidates first), and fine before
    /// the restart offer or answer of its ICE generation did (the `ufrag`
    /// it names is not known yet): the session keeps it and adds it once
    /// [`Session::set_remote_offer`] or [`Session::set_remote_answer`]
    /// brought that description. A candidate that does not parse is
    /// refused at once, kept or not.
    pub async fn add_remote_candidate(&self, candidate: &Candidate) -> Result<()> {
        let c = IceCandidate::parse(&candidate.sdp_mid, candidate.sdp_mline_index, &candidate.candidate)?;
        {
            let mut remote = self.remote.lock().unwrap_or_else(|e| e.into_inner());
            if !remote.takes(candidate) {
                tracing::debug!(candidate = %candidate.candidate, "ice: a remote candidate waits for its description");
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
    /// it go in, in the order they came; those of a generation still
    /// unknown keep waiting. One libwebrtc refuses is logged and skipped,
    /// not an error of the description, which is in place.
    async fn flush_remote_candidates(&self, sdp: &str) {
        let pending = {
            let mut remote = self.remote.lock().unwrap_or_else(|e| e.into_inner());
            remote.described = true;
            remote.ufrags.extend(sdp_ufrags(sdp));
            let (now, later): (Vec<Candidate>, Vec<Candidate>) = std::mem::take(&mut remote.pending).into_iter().partition(|c| remote.takes(c));
            remote.pending = later;
            now
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
    /// end with it: a reader of [`Session::remote_video_frames`] (or of
    /// a room's [`Session::remote_video_frames_of`]) gets `Closed` once
    /// the pump task let go of its sender (at once, or when the task
    /// next yields), and every [`AudioOutput`] taken gives `None`.
    pub fn close(&self) {
        if let Some(data) = &self.data {
            data.close();
        }
        self.pc.close();
        if let Some(pump) = self.remote_pump.lock().unwrap_or_else(|e| e.into_inner()).take() {
            pump.abort();
        }
        drop(self.remote_frames.lock().unwrap_or_else(|e| e.into_inner()).take());
        self.shared.remote_videos.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.shared.audio_outputs.lock().unwrap_or_else(|e| e.into_inner()).clear();
        // The ends dropped: every output taken (the far end's of a call
        // between two, each track's of a room) gives `None` from now on.
        self.shared.audio_ends.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.shared.mids.lock().unwrap_or_else(|e| e.into_inner()).clear();
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
    /// device path, the second time, or in a room (where every track has
    /// one: [`Session::take_audio_output_of`]).
    pub fn take_audio_output(&self) -> Option<AudioOutput> {
        self.shared.audio_output.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    // ─── A room ──────────────────────────────────────────────────────

    /// The room this session is a leg of, if any.
    pub fn room(&self) -> Option<&RoomConfig> {
        self.room.as_ref()
    }

    /// A message on the data channel of the room: text (an answer to the
    /// node) or bytes (for the other participants; the node puts this
    /// side's id in front for them). An error before the channel is open
    /// ([`SessionEvent::DataOpen`]) or without a room.
    pub fn send_data(&self, binary: bool, data: &[u8]) -> Result<()> {
        let channel = self.data.as_ref().ok_or_else(|| Error::State("this session has no data channel".into()))?;
        if channel.state() != DataChannelState::Open {
            return Err(Error::State(format!("the data channel is {:?}", channel.state())));
        }
        channel.send(data, binary).map_err(|e| Error::State(e.to_string()))
    }

    /// The remote tracks of the room that are there now: mid and kind,
    /// by mid. Whose they are the node said in its offers.
    pub fn remote_tracks(&self) -> Vec<(String, TrackKind)> {
        let mut out: Vec<(String, TrackKind)> =
            self.shared.mids.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|(m, k)| (m.clone(), *k)).collect();
        out.sort();
        out
    }

    /// The sound of the remote track on `mid` (the pushed path), once
    /// its [`SessionEvent::RemoteTrack`] came; taken once. `None` on the
    /// device path, where the engine mixes every remote track into the
    /// speaker by itself. A reader of several outputs sums their frames
    /// ([`crate::audio::mix`]); the output ends (`None`) once the
    /// m-line is closed ([`SessionEvent::RemoteTrackGone`]) or the
    /// session is, so such a reader is never left waiting on a
    /// participant who went.
    pub fn take_audio_output_of(&self, mid: &str) -> Option<AudioOutput> {
        self.shared.audio_outputs.lock().unwrap_or_else(|e| e.into_inner()).remove(mid)
    }

    /// The frames of the remote video on `mid`, decoded, as they come. A
    /// late reader skips to the newest; every reader gets `Closed` once
    /// the m-line was closed ([`SessionEvent::RemoteTrackGone`]) or the
    /// session did. Fine before the track is there.
    pub fn remote_video_frames_of(&self, mid: &str) -> broadcast::Receiver<Arc<VideoFrame>> {
        self.shared.remote_videos.lock().unwrap_or_else(|e| e.into_inner()).entry(mid.to_string()).or_insert_with(RemoteVideo::new).frames.subscribe()
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
        self.video.source()
    }

    /// A fresh source for this side's video, of this size and kind (a
    /// screen asks the encoder for sharp text), on the same sender: the
    /// track is replaced, nothing renegotiated. What was pushed into the
    /// old source is no longer sent.
    pub fn replace_video_source(&self, width: u32, height: u32, screencast: bool) -> Result<VideoSource> {
        let source = self.video.replace_source(width, height, screencast);
        let track = self.factory.create_video_track("video", source.inner.clone());
        let mut current = self.video_track.lock().unwrap_or_else(|e| e.into_inner());
        track.set_enabled(current.enabled());
        self.video.sender().set_track(Some(MediaStreamTrack::Video(track.clone())))?;
        *current = track;
        drop(current);
        // The layers of a simulcast follow the size of the source: the
        // one it was made for until its first frame, that frame's size
        // from then on (`VideoSending::frames_came_at`).
        if self.video.simulcast {
            self.apply_video_parameters()?;
        }
        Ok(source)
    }

    /// The most this side's video may spend, kbit/s; `None` leaves it to
    /// the engine's estimate of the way. With simulcast every layer is
    /// capped at the lesser of its own ceiling and this.
    pub fn set_video_max_bitrate(&self, max_kbps: Option<u32>) -> Result<()> {
        *self.video.cap_kbps.lock().unwrap_or_else(|e| e.into_inner()) = max_kbps;
        self.apply_video_parameters()
    }

    /// The topmost layer of this side's simulcast video that is sent;
    /// the lower ones always are. Nothing to do without simulcast (the
    /// one layer stays).
    pub fn set_video_layers(&self, top: VideoLayer) -> Result<()> {
        *self.video.top.lock().unwrap_or_else(|e| e.into_inner()) = top;
        self.apply_video_parameters()
    }

    pub fn video_layers(&self) -> VideoLayer {
        *self.video.top.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The layers of this side's video as the sender has them now; one
    /// entry without a RID for a single stream, none before the first
    /// negotiation.
    pub fn video_encodings(&self) -> Vec<VideoEncoding> {
        self.video
            .sender()
            .parameters()
            .encodings
            .iter()
            .map(|e| VideoEncoding {
                rid: e.rid.clone(),
                active: e.active,
                max_kbps: e.max_bitrate.map(|b| (b / 1000) as u32),
                scale_down: e.scale_resolution_down_by,
            })
            .collect()
    }

    /// The cap, the layers the frames are big enough for and the topmost
    /// one asked for, onto the sender's encodings
    /// ([`VideoSending::apply_parameters`]).
    fn apply_video_parameters(&self) -> Result<()> {
        self.video.apply_parameters()
    }

    /// The answer took no simulcast: libwebrtc keeps the first layer
    /// alone, which is the smallest one. Make it the full size with the
    /// ceiling of the top layer, so that a node without simulcast gets
    /// the picture a node with it would get at its best.
    fn single_layer_at_full_size(&self) {
        let sender = self.video.sender();
        // The same pair as `VideoSending::apply_parameters`, against
        // the same hook (a capture may push its first frame as the
        // answer comes).
        let _turn = self.video.parameters.lock().unwrap_or_else(|e| e.into_inner());
        let mut params = sender.parameters();
        match params.encodings.as_mut_slice() {
            [one] => {
                tracing::debug!(rid = %one.rid, "simulcast was not taken: one layer at the full size");
                let cap = *self.video.cap_kbps.lock().unwrap_or_else(|e| e.into_inner());
                let ceiling = SIMULCAST_LAYERS[SIMULCAST_LAYERS.len() - 1].1;
                one.scale_resolution_down_by = Some(1.0);
                one.max_bitrate = cap.map(|c| u64::from(c.min(ceiling)) * 1000);
                if let Err(e) = sender.set_parameters(params) {
                    tracing::warn!(error = %e, "the single layer could not be set to the full size");
                }
            }
            others => tracing::warn!(layers = others.len(), "simulcast was not taken, and the sender kept these layers"),
        }
    }

    /// This side's own frames, as pushed into the source in use: for the
    /// small picture of oneself. A late reader skips to the newest.
    pub fn local_video_frames(&self) -> broadcast::Receiver<Arc<VideoFrame>> {
        self.shared.local_frames.subscribe()
    }

    /// The far end's frames, decoded, once they come (a call between
    /// two). A late reader skips to the newest; every reader gets
    /// `Closed` once the session closed (a receiver that is closed
    /// already, after it).
    pub fn remote_video_frames(&self) -> broadcast::Receiver<Arc<VideoFrame>> {
        match self.remote_frames.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            Some(sender) => sender.subscribe(),
            None => broadcast::channel(1).1,
        }
    }

    // ─── Frame encryption ────────────────────────────────────────────

    /// Frame encryption on or off, both directions. On by default when
    /// the session was made with keys; nothing without them.
    pub fn set_encryption_enabled(&self, enabled: bool) {
        if let Some(e) = &self.shared.encryption {
            e.enabled.store(enabled, Ordering::SeqCst);
            for c in e.senders.lock().unwrap_or_else(|x| x.into_inner()).iter() {
                c.set_enabled(enabled);
            }
            for (_, c) in e.receivers.lock().unwrap_or_else(|x| x.into_inner()).iter() {
                c.set_enabled(enabled);
            }
        }
    }

    pub fn encryption_enabled(&self) -> bool {
        self.shared.encryption.as_ref().is_some_and(|e| e.enabled.load(Ordering::SeqCst))
    }

    /// The ring of keys this session encrypts with, if it does.
    pub fn frame_keys(&self) -> Option<crate::FrameKeys> {
        self.shared.encryption.as_ref().map(|e| e.keys.clone())
    }

    fn encryption(&self) -> Result<&EncryptionShared> {
        self.shared.encryption.as_ref().ok_or_else(|| Error::State("this session encrypts no frames".into()))
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

    /// The key material of slot `index` for everything this side sends,
    /// and the switch to that slot: the epoch moved. Frames in flight
    /// carry their own slot, so a receiver that still holds the old key
    /// reads them; nothing is lost on the way over. Replaces the slot.
    pub fn set_sender_key(&self, index: u8, key: &[u8]) -> Result<()> {
        let e = self.encryption()?;
        if !e.keys.set_sender_key(index, key) {
            return Err(Error::State(format!("the sender key of slot {index} was refused")));
        }
        self.set_key_index(index);
        Ok(())
    }

    /// The key material of slot `index` for the frames that come on the
    /// m-line `mid` (a room: whose stream it is the node said). Fine
    /// before the track is there. A track without the key of the slot
    /// its frames name stays silent and dark.
    pub fn set_receiver_key(&self, mid: &str, index: u8, key: &[u8]) -> Result<()> {
        let e = self.encryption()?;
        if !e.keys.set_receiver_key(mid, index, key) {
            return Err(Error::State(format!("the receiver key of slot {index} on {mid} was refused")));
        }
        Ok(())
    }

    /// This side's sender key of slot `index` ratcheted one step forward
    /// (HKDF); the new material, for whoever must follow.
    pub fn ratchet_sender_key(&self, index: u8) -> Result<Vec<u8>> {
        self.encryption()?
            .keys
            .ratchet_sender_key(index)
            .ok_or_else(|| Error::State(format!("no sender key in slot {index} to ratchet")))
    }

    /// The key of slot `index` of the m-line `mid` ratcheted one step
    /// forward; the new material.
    pub fn ratchet_receiver_key(&self, mid: &str, index: u8) -> Result<Vec<u8>> {
        self.encryption()?
            .keys
            .ratchet_receiver_key(mid, index)
            .ok_or_else(|| Error::State(format!("no key in slot {index} on {mid} to ratchet")))
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
        f.debug_struct("Session").field("state", &self.connection_state()).field("room", &self.room).finish()
    }
}

impl Shared {
    fn send(&self, event: SessionEvent) {
        // Nobody listening is fine: the session goes on.
        let _ = self.events.send(event);
    }

    /// A track of another participant of the room arrived on `mid`: its
    /// output is made and the track announced.
    fn remote_track(&self, mid: String, track: MediaStreamTrack) {
        let kind = match track {
            MediaStreamTrack::Audio(track) => {
                if self.pushed {
                    let opts = NativeAudioStreamOptions { queue_size_frames: Some(audio::OUTPUT_QUEUE_FRAMES) };
                    let stream = NativeAudioStream::with_options(track, SAMPLE_RATE as i32, 1, opts);
                    let (end, output) = AudioOutput::new(stream, self.apm.clone());
                    // A mid used again after its m-line was closed: the
                    // old end goes, and with it any reader still on it.
                    self.audio_ends.lock().unwrap_or_else(|e| e.into_inner()).insert(mid.clone(), end);
                    self.audio_outputs.lock().unwrap_or_else(|e| e.into_inner()).insert(mid.clone(), output);
                }
                TrackKind::Audio
            }
            MediaStreamTrack::Video(track) => {
                let stream = NativeVideoStream::with_options(track, NativeVideoStreamOptions { queue_size_frames: Some(video::OUTPUT_QUEUE_FRAMES) });
                let mut output = VideoOutput::new(stream);
                let mut videos = self.remote_videos.lock().unwrap_or_else(|e| e.into_inner());
                let video = videos.entry(mid.clone()).or_insert_with(RemoteVideo::new);
                let frames = video.frames.clone();
                let events = self.events.clone();
                let of = mid.clone();
                let pump = self.runtime.spawn(async move {
                    let mut size = (0, 0);
                    while let Some(frame) = output.next().await {
                        if (frame.width, frame.height) != size {
                            size = (frame.width, frame.height);
                            let _ = events.send(SessionEvent::RemoteVideoSize { mid: of.clone(), width: frame.width, height: frame.height });
                        }
                        let _ = frames.send(Arc::new(frame));
                    }
                });
                if let Some(old) = video.pump.replace(pump) {
                    old.abort();
                }
                TrackKind::Video
            }
        };
        tracing::debug!(%mid, ?kind, "room: a remote track");
        self.mids.lock().unwrap_or_else(|e| e.into_inner()).insert(mid.clone(), kind);
        self.send(SessionEvent::RemoteTrack { mid, kind });
    }

    /// The m-line `mid` is closed: its outputs end, its cryptor goes, the
    /// readers of its frames get `Closed`.
    fn forget_track(&self, mid: &str) {
        let known = self.mids.lock().unwrap_or_else(|e| e.into_inner()).remove(mid).is_some();
        self.audio_outputs.lock().unwrap_or_else(|e| e.into_inner()).remove(mid);
        // The output taken already ends for its reader.
        if let Some(end) = self.audio_ends.lock().unwrap_or_else(|e| e.into_inner()).remove(mid) {
            let _ = end.send(true);
        }
        self.remote_videos.lock().unwrap_or_else(|e| e.into_inner()).remove(mid);
        if let Some(e) = &self.encryption {
            e.receivers.lock().unwrap_or_else(|x| x.into_inner()).retain(|(m, _)| m != mid);
        }
        if known {
            tracing::debug!(%mid, "room: a remote track is gone");
            self.send(SessionEvent::RemoteTrackGone { mid: mid.to_string() });
        }
    }

    /// A cryptor on a sender of this side, when the session encrypts.
    /// Inside a per-sender ring the sender is `crypto::SENDER`; the
    /// events name it as the configuration does.
    fn encrypt_sender(&self, factory: &PeerConnectionFactory, sender: RtpSender) {
        let Some(e) = &self.encryption else { return };
        let id = if e.keys.is_shared() { e.participant.clone() } else { crypto::SENDER.to_string() };
        let cryptor = FrameCryptor::new_for_rtp_sender(factory, id, EncryptionAlgorithm::AesGcm, e.keys.provider.clone(), sender);
        let participant = e.participant.clone();
        self.watch(&cryptor, move |state| SessionEvent::Encryption { participant: participant.clone(), state });
        cryptor.set_key_index(*e.key_index.lock().unwrap_or_else(|x| x.into_inner()) as i32);
        cryptor.set_enabled(e.enabled.load(Ordering::SeqCst));
        e.senders.lock().unwrap_or_else(|x| x.into_inner()).push(cryptor);
    }

    /// A cryptor on a receiver of the far end's, when the session
    /// encrypts: the one of `peer` in a call between two, the one of the
    /// m-line `mid` in a room (its key is set by that mid).
    fn decrypt_receiver(&self, factory: &PeerConnectionFactory, receiver: libwebrtc::rtp_receiver::RtpReceiver, mid: &str) {
        let Some(e) = &self.encryption else { return };
        let id = if e.keys.is_shared() { "peer".to_string() } else { crypto::receiver_id(mid) };
        let cryptor = FrameCryptor::new_for_rtp_receiver(factory, id, EncryptionAlgorithm::AesGcm, e.keys.provider.clone(), receiver);
        let name = mid.to_string();
        let room = self.room;
        self.watch(&cryptor, move |state| {
            if room {
                SessionEvent::RemoteEncryption { mid: name.clone(), state }
            } else {
                SessionEvent::Encryption { participant: "peer".to_string(), state }
            }
        });
        cryptor.set_enabled(e.enabled.load(Ordering::SeqCst));
        e.receivers.lock().unwrap_or_else(|x| x.into_inner()).push((mid.to_string(), cryptor));
    }

    fn watch(&self, cryptor: &FrameCryptor, event: impl Fn(EncryptionState) -> SessionEvent + Send + Sync + 'static) {
        let events = self.events.clone();
        cryptor.on_state_change(Some(Box::new(move |participant, state| {
            tracing::debug!(%participant, ?state, "frame cryptor");
            let _ = events.send(event(state.into()));
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
                out.inbound.push(TrackStats {
                    mid: r.inbound.mid.clone(),
                    kind: TrackKind::parse(&r.stream.kind),
                    rid: String::new(),
                    bytes: r.inbound.bytes_received,
                    packets: r.received.packets_received,
                    packets_lost: r.received.packets_lost,
                    jitter_ms: r.received.jitter * 1000.0,
                    audio_level: r.inbound.audio_level,
                    width: r.inbound.frame_width,
                    height: r.inbound.frame_height,
                    active: true,
                    quality_limitation: String::new(),
                });
            }
            RtcStats::OutboundRtp(o) => {
                out.outbound.push(TrackStats {
                    mid: o.outbound.mid.clone(),
                    kind: TrackKind::parse(&o.stream.kind),
                    rid: o.outbound.rid.clone(),
                    bytes: o.sent.bytes_sent,
                    packets: o.sent.packets_sent,
                    packets_lost: 0,
                    jitter_ms: 0.0,
                    audio_level: 0.0,
                    width: o.outbound.frame_width,
                    height: o.outbound.frame_height,
                    active: o.outbound.active,
                    quality_limitation: format!("{:?}", o.outbound.quality_limitation_reason).to_lowercase(),
                });
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
    out.inbound.sort_by(|a, b| a.mid.cmp(&b.mid));
    out.outbound.sort_by(|a, b| (&a.mid, &a.rid).cmp(&(&b.mid, &b.rid)));
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

    /// A candidate of a generation not described yet waits; one without
    /// a ufrag, or of a known generation, goes in; a description brings
    /// its generation and lets its candidates through, not the others'.
    #[test]
    fn candidates_wait_for_the_description_of_their_generation() {
        let cand = |ufrag: Option<&str>| Candidate {
            sdp_mid: "0".into(),
            sdp_mline_index: 0,
            candidate: match ufrag {
                Some(u) => format!("candidate:1 1 udp 2 10.0.0.2 1 typ host generation 0 ufrag {u} network-id 1"),
                None => "candidate:1 1 udp 2 10.0.0.2 1 typ host".into(),
            },
        };
        assert_eq!(candidate_ufrag(&cand(Some("abcd")).candidate), Some("abcd"));
        assert_eq!(candidate_ufrag(&cand(None).candidate), None);
        let mut remote = Remote::default();
        assert!(!remote.takes(&cand(None)), "nothing before a description");
        remote.described = true;
        remote.ufrags.extend(sdp_ufrags("v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=ice-ufrag:abcd\r\na=ice-pwd:x\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\na=ice-ufrag:abcd\r\n"));
        assert_eq!(remote.ufrags.len(), 1);
        assert!(remote.takes(&cand(None)));
        assert!(remote.takes(&cand(Some("abcd"))));
        assert!(!remote.takes(&cand(Some("wxyz"))), "the restart's generation is not here yet");
        remote.ufrags.extend(sdp_ufrags("a=ice-ufrag:wxyz\n"));
        assert!(remote.takes(&cand(Some("wxyz"))));
        assert!(remote.takes(&cand(Some("abcd"))), "the old generation is still known");
    }

    #[test]
    fn the_wire_form_of_a_candidate_and_a_policy() {
        let c = Candidate { sdp_mid: "0".into(), sdp_mline_index: 0, candidate: "candidate:1 1 udp 2 10.0.0.2 1 typ host".into() };
        let text = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<Candidate>(&text).unwrap(), c);
        assert_eq!(serde_json::to_string(&IcePolicy::RelayOnly).unwrap(), "\"relay_only\"");
        let server: IceServer = serde_json::from_str(r#"{"urls":["stun:203.0.113.7:3478"]}"#).unwrap();
        assert_eq!(server.username, "");
        assert_eq!(serde_json::to_string(&VideoLayer::Medium).unwrap(), "\"medium\"");
        assert!(VideoLayer::Low < VideoLayer::Medium && VideoLayer::Medium < VideoLayer::High);
        assert_eq!(TrackKind::parse("video"), Some(TrackKind::Video));
        assert_eq!(TrackKind::parse("data"), None);
    }

    /// The node closes the m-line of a participant who left with port 0;
    /// the mids of such sections are read, the others left alone.
    #[test]
    fn closed_m_sections_are_known_by_their_mid() {
        let sdp = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:0\r\na=sendonly\r\nm=video 0 UDP/TLS/RTP/SAVPF 96\r\na=mid:1\r\nm=audio 0 UDP/TLS/RTP/SAVPF 111\r\nc=IN IP4 0.0.0.0\r\na=mid:7\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\na=mid:2\r\n";
        assert_eq!(closed_mids(sdp), vec!["1".to_string(), "7".to_string()]);
        assert!(closed_mids("v=0\r\n").is_empty());
        assert!(!sdp_has_simulcast(sdp));
        assert!(sdp_has_simulcast("m=video 9 UDP/TLS/RTP/SAVPF 96\r\na=rid:q recv\r\na=simulcast:recv q;h;f\r\n"));
        assert_eq!(layer_of_rid("h").map(|l| l.1), Some(VideoLayer::Medium));
        assert_eq!(layer_of_rid(""), None);
    }

    /// The count goes by pixels, as libwebrtc's table does, so a frame
    /// turned on its side (a phone held upright) counts as its landscape
    /// twin; a width alone would under-provision it.
    #[test]
    fn the_layers_go_by_the_pixels_of_the_frames_either_way_up() {
        assert_eq!((layers_for_size(320, 180), layers_for_size(640, 360), layers_for_size(1280, 720)), (1, 2, 3));
        assert_eq!((layers_for_size(180, 320), layers_for_size(360, 640), layers_for_size(720, 1280)), (1, 2, 3));
        assert_eq!((layers_for_size(960, 540), layers_for_size(540, 960), layers_for_size(480, 270)), (3, 3, 2));
        assert_eq!((layers_for_size(640, 480), layers_for_size(800, 600), layers_for_size(1920, 1080)), (2, 2, 3));
        assert_eq!(layers_for_size(0, 0), 1);
    }
}
