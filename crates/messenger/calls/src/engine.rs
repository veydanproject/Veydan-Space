// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What the core asks of a media engine. `messenger-rtc` implements it on
//! libwebrtc; `messenger-testkit` fakes it. The core never sees a socket,
//! a codec or a frame: it hands the engine ICE servers and a policy, moves
//! SDP and candidates between the engine and the peer, and listens.
//!
//! One [`Session`] is one peer connection. Its events come on a channel
//! the core takes once (`events`); the engine never calls back into the
//! core, so the core may hold its own lock while it calls the engine.

use async_trait::async_trait;
use messenger_core::{MessengerError, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};

/// What a call carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Media {
    Audio,
    Video,
}

impl Media {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Video => "video",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "audio" => Some(Self::Audio),
            "video" => Some(Self::Video),
            _ => None,
        }
    }
}

/// Which ICE candidates the engine may use. `Auto` is ICE as it is: a
/// direct pair when one works, a relay when none does. `RelayOnly` hides
/// my address from the peer behind a relay, always.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayPolicy {
    #[default]
    Auto,
    RelayOnly,
}

impl RelayPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::RelayOnly => "relay_only",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(Self::Auto),
            "relay_only" => Some(Self::RelayOnly),
            _ => None,
        }
    }
}

/// One entry of the ICE server list, as libwebrtc takes it: `stun:` and
/// `turn:`/`turns:` URIs (RFC 7064, 7065) with the short-lived credentials
/// a node handed out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IceServer {
    pub urls: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

/// One ICE candidate, as it travels in the signalling: the candidate line
/// and where in the SDP it belongs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IceCandidate {
    pub candidate: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u16>,
}

/// Whether the SDP given to `set_remote` is the peer's offer or its answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SdpKind {
    Offer,
    Answer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    New,
    Connecting,
    Connected,
    /// The connection was lost; it may come back, or an ICE restart may
    /// bring it back.
    Disconnected,
    /// ICE gave up.
    Failed,
    Closed,
}

/// How the media goes: directly between the two, or through a relay.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairKind {
    Direct,
    Relay,
}

impl PairKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Relay => "relay",
        }
    }
}

/// What an engine tells of a running session, for the screen. All
/// optional: an engine tells what it has.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionStats {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_sent: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_received: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packets_lost: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jitter_ms: Option<u32>,
}

/// One frame of video as plain data: I420, the three planes packed one
/// after another with no padding — Y of `width`×`height` bytes, then U
/// and V of ⌈width/2⌉×⌈height/2⌉ each. What the engine decodes and what
/// a camera captures come in this shape, and the screen draws it as it is
/// (a YUV→RGB shader costs less than the three times more bytes of RGBA
/// through the IPC: internal/messenger-wire.md §10, "Видео").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    /// Degrees the picture is to be turned clockwise to stand upright: 0,
    /// 90, 180 or 270 (a phone held sideways). The planes are as captured.
    pub rotation: u16,
    /// When it was captured, microseconds of a monotonic clock; 0 unknown.
    pub timestamp_us: i64,
    pub data: Vec<u8>,
}

impl VideoFrame {
    /// The size of the chroma planes of a frame this big.
    pub fn chroma_size(width: u32, height: u32) -> (u32, u32) {
        (width.div_ceil(2), height.div_ceil(2))
    }

    /// How many bytes a frame of this size holds.
    pub fn len_for(width: u32, height: u32) -> usize {
        let (cw, ch) = Self::chroma_size(width, height);
        (width as usize * height as usize) + 2 * (cw as usize * ch as usize)
    }

    /// A black frame: what a placeholder or a test sends.
    pub fn black(width: u32, height: u32) -> VideoFrame {
        let y = width as usize * height as usize;
        let mut data = vec![16u8; Self::len_for(width, height)];
        data[y..].fill(128);
        VideoFrame { width, height, rotation: 0, timestamp_us: 0, data }
    }

    /// The planes, or `None` when the data is shorter than the size says.
    pub fn planes(&self) -> Option<(&[u8], &[u8], &[u8])> {
        let y = self.width as usize * self.height as usize;
        let (cw, ch) = Self::chroma_size(self.width, self.height);
        let c = cw as usize * ch as usize;
        if self.data.len() < y + 2 * c {
            return None;
        }
        Some((&self.data[..y], &self.data[y..y + c], &self.data[y + c..y + 2 * c]))
    }

    /// Whether the planes are all there and the rotation is one of the
    /// four, and the frame is not empty.
    pub fn is_well_formed(&self) -> bool {
        self.width > 0 && self.height > 0 && matches!(self.rotation, 0 | 90 | 180 | 270) && self.planes().is_some()
    }
}

/// Which video of a call: mine as the camera sees it (for the small
/// picture of myself), or the peer's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoTrack {
    Local,
    Remote,
}

/// What this side sends as video.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VideoInput {
    Off,
    /// The camera `id` names (`CameraInfo::id`), or the engine's default.
    Camera {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// The screen or window `id` names (`ScreenInfo::id`; a computer
    /// only), or the first screen.
    Screen {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
}

/// How my video is captured and sent: the size and rate asked of the
/// camera (a camera gives the nearest it has) and the most the encoder
/// may spend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoSettings {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub max_kbps: Option<u32>,
}

/// A camera the engine can open.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraInfo {
    /// What `VideoInput::Camera` takes: a device path, an index, whatever
    /// the platform names a camera by. Opaque to everybody but the engine.
    pub id: String,
    pub name: String,
}

/// A screen or a window the engine can share.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenInfo {
    pub id: String,
    pub title: String,
    /// A window rather than a whole screen.
    pub window: bool,
}

/// The layout of a frame the platform hands to the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PixelFormat {
    /// Three planes, as [`VideoFrame`] has them.
    I420,
    /// Y, then one plane of U and V interleaved (the camera of a phone).
    Nv12,
    /// Y, then V and U interleaved (Android's `ImageFormat.NV21`).
    Nv21,
    /// Y U Y V, two bytes per pixel (a USB camera).
    Yuyv,
}

/// A frame captured by the platform and pushed into the engine: the
/// camera of a phone, through the plugin, in whatever layout it gives.
/// Tightly packed: a row is `width` pixels wide, with no padding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushedFrame {
    pub format: PixelFormat,
    pub width: u32,
    pub height: u32,
    pub rotation: u16,
    pub timestamp_us: i64,
    pub data: Vec<u8>,
}

/// What a session tells the core.
#[derive(Clone, Debug, PartialEq)]
pub enum SessionEvent {
    /// A candidate of mine the peer should know.
    LocalCandidate(IceCandidate),
    /// No more candidates are coming for the current offer or answer.
    GatheringComplete,
    ConnectionState(ConnectionState),
    /// The pair ICE settled on: whether the media goes directly or through
    /// a relay. Told on every change.
    SelectedPair(PairKind),
    Stats(SessionStats),
    /// Level of the sound from the peer, 0.0 to 1.0, for a meter.
    AudioLevel(f32),
    /// The engine saw the network change under it (an interface came or
    /// went); the core answers with an ICE restart.
    NetworkChanged,
    /// The frames of one video changed size (the first frame, a camera
    /// switched, the far end turned its phone): for the screen to lay
    /// out. An engine may tell a size the core has again (after a gap in
    /// the frames); the core drops what it has already.
    VideoSize { track: VideoTrack, width: u32, height: u32 },
    /// My video ended by itself: the camera was unplugged, the shared
    /// window closed. The engine sends nothing from then on; the core
    /// turns the video off, tells the peer and shows the reason.
    VideoLost { reason: String },
}

/// One peer connection.
#[async_trait]
pub trait Session: Send + Sync {
    /// The offer of a new call: starts gathering candidates, which come as
    /// `LocalCandidate` events until `GatheringComplete`.
    async fn create_offer(&self) -> Result<String>;
    /// The answer to the offer given with `set_remote`; gathers likewise.
    async fn create_answer(&self) -> Result<String>;
    async fn set_remote(&self, sdp: &str, kind: SdpKind) -> Result<()>;
    async fn add_ice(&self, candidate: &IceCandidate) -> Result<()>;
    /// A new offer for the same call with fresh candidates (the network
    /// changed, the relay went away). The peer answers it as the first.
    async fn restart_ice(&self) -> Result<String>;
    async fn set_mute(&self, muted: bool) -> Result<()>;
    async fn close(&self);
    /// The events of this session. Taken once, by the core.
    fn events(&self) -> mpsc::Receiver<SessionEvent>;

    /// My video: a camera, a screen, or nothing. Without a renegotiation:
    /// the video of every call is in the first offer, and frames start or
    /// stop (internal/messenger-wire.md §10). An engine without video
    /// takes it silently (the fake of the tests); a camera that cannot be
    /// opened is an error, and the call goes on without the video.
    async fn set_video(&self, _input: VideoInput, _settings: VideoSettings) -> Result<()> {
        Ok(())
    }

    /// The most my video may spend from now on, kbit/s; `None` for the
    /// engine's own estimate alone. The core caps it by the node's limit
    /// once the media goes through a relay.
    async fn set_video_bitrate(&self, _max_kbps: Option<u32>) -> Result<()> {
        Ok(())
    }

    /// The frames of one video of the call as they come, for the screen.
    /// A reader that falls behind skips what it missed and goes on with
    /// the newest. `None` for an engine without video.
    fn video_frames(&self, _track: VideoTrack) -> Option<broadcast::Receiver<Arc<VideoFrame>>> {
        None
    }

    /// A frame the platform captured (the camera of a phone, through the
    /// plugin), as my video. The engine converts and sends it.
    fn push_video_frame(&self, _frame: PushedFrame) -> Result<()> {
        Err(MessengerError::Transport("this engine takes no pushed video".into()))
    }
}

/// The engine: a factory of sessions.
#[async_trait]
pub trait MediaEngine: Send + Sync {
    async fn create_session(&self, ice_servers: Vec<IceServer>, policy: RelayPolicy, media: Media) -> Result<Box<dyn Session>>;

    /// The cameras of this machine, the default first. Empty on a phone:
    /// its plugin lists them and pushes their frames.
    async fn cameras(&self) -> Vec<CameraInfo> {
        vec![]
    }

    /// The screens and windows that can be shared (a computer).
    async fn screens(&self) -> Vec<ScreenInfo> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_round_trip() {
        assert_eq!(Media::parse("audio"), Some(Media::Audio));
        assert_eq!(Media::parse("video").map(Media::as_str), Some("video"));
        assert_eq!(Media::parse("text"), None);
        assert_eq!(RelayPolicy::parse("relay_only"), Some(RelayPolicy::RelayOnly));
        assert_eq!(RelayPolicy::parse("auto"), Some(RelayPolicy::Auto));
        assert_eq!(RelayPolicy::parse("relay"), None);
        assert_eq!(RelayPolicy::default(), RelayPolicy::Auto);
        assert_eq!(serde_json::to_string(&PairKind::Relay).unwrap(), "\"relay\"");
    }

    #[test]
    fn a_candidate_and_a_server_leave_out_what_they_do_not_have() {
        let c = IceCandidate { candidate: "candidate:1 1 udp 1 203.0.113.7 5000 typ host".into(), mid: None, index: None };
        assert_eq!(serde_json::to_value(&c).unwrap(), serde_json::json!({ "candidate": c.candidate }));
        let s = IceServer { urls: vec!["stun:203.0.113.7:3478".into()], username: None, credential: None };
        assert_eq!(serde_json::to_value(&s).unwrap(), serde_json::json!({ "urls": ["stun:203.0.113.7:3478"] }));
        let back: IceCandidate = serde_json::from_str(r#"{"candidate":"x","mid":"0","index":0}"#).unwrap();
        assert_eq!(back.index, Some(0));
    }

    #[test]
    fn a_frame_knows_its_planes() {
        let f = VideoFrame::black(5, 3);
        assert_eq!(f.data.len(), 15 + 2 * 6, "odd sizes round the chroma up");
        let (y, u, v) = f.planes().unwrap();
        assert_eq!((y.len(), u.len(), v.len()), (15, 6, 6));
        assert!(y.iter().all(|&b| b == 16) && u.iter().all(|&b| b == 128) && v.iter().all(|&b| b == 128));
        assert!(f.is_well_formed());
        let short = VideoFrame { data: vec![0; 10], ..f.clone() };
        assert!(short.planes().is_none() && !short.is_well_formed());
        assert!(!VideoFrame { rotation: 45, ..f }.is_well_formed());
        assert_eq!(VideoFrame::len_for(640, 360), 640 * 360 * 3 / 2);
    }

    #[test]
    fn the_words_of_video_on_the_wire() {
        assert_eq!(serde_json::to_string(&VideoInput::Off).unwrap(), r#"{"kind":"off"}"#);
        assert_eq!(serde_json::to_string(&VideoInput::Camera { id: None }).unwrap(), r#"{"kind":"camera"}"#);
        assert_eq!(serde_json::to_string(&VideoInput::Screen { id: Some("s:1".into()) }).unwrap(), r#"{"kind":"screen","id":"s:1"}"#);
        assert_eq!(serde_json::from_str::<VideoInput>(r#"{"kind":"camera","id":"/dev/video0"}"#).unwrap(), VideoInput::Camera { id: Some("/dev/video0".into()) });
        assert_eq!(serde_json::to_string(&VideoTrack::Remote).unwrap(), "\"remote\"");
        assert_eq!(serde_json::to_string(&PixelFormat::Nv21).unwrap(), "\"nv21\"");
    }
}
