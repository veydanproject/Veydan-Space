// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The media engine of calls: libwebrtc behind a small door.
//!
//! The call itself (who rings whom, the signaling over the relays, which
//! node to take) is `messenger-calls`; this crate is the part that moves
//! sound. One [`Engine`] per process holds the factory of libwebrtc and
//! the way audio enters and leaves it ([`AudioMode`]); one [`Session`] per
//! call is a PeerConnection: ICE servers and the relay policy, offer and
//! answer as SDP text, candidates both ways, a restart of ICE, the state,
//! the pair of candidates in use (directly or through a relay), the
//! statistics, the audio track with its mute, and, for groups, the
//! encryption of frames ([`FrameKeys`]) and the leg to the SFU of a call
//! node ([`RoomConfig`]: sendonly tracks, the data channel, the tracks
//! of the others by their mid, three layers of video). Video: a track of this side in
//! every session from the start, fed by the camera of a computer
//! (`camera.rs`), its screen (`screen.rs`) or frames pushed from outside
//! (a phone's plugin, the CLI); the far end's frames and this side's own
//! come out as broadcasts of [`VideoFrame`]s for the page to draw.
//!
//! ```text
//! messenger-calls ── MediaEngine ──▶ adapter ──▶ Engine ──▶ Session ──▶ libwebrtc
//!                                                 │            │
//!                                   AudioMode::Device         AudioInput / AudioOutput
//!                                   (microphone, speaker,     (10 ms frames, the CLI and the tests)
//!                                    camera, screen)          VideoSource / local & remote frames
//! ```
//!
//! Two ways for audio, decided once per [`Engine`]:
//!
//! - [`AudioMode::Device`]: the platform's audio device module (ALSA or
//!   PulseAudio, WASAPI, CoreAudio, the JNI module of Android). The
//!   microphone goes through the echo canceller of the engine, the far end
//!   plays through the speaker. A computer and a phone take this.
//! - [`AudioMode::Pushed`]: the caller pushes 10 ms frames of 48 kHz mono
//!   PCM into the session ([`AudioInput`]) and pulls what the far end sent
//!   ([`AudioOutput`]). Frames pushed this way go past the engine's audio
//!   processing (tmp/calls-spike/REPORT.md, section 3), so this crate runs
//!   an `AudioProcessingModule` of its own on them: the far end's frames
//!   are its render stream, the pushed frames its capture stream.
//!   `messenger-cli` and the tests take this.
//!
//! Everything a session reports arrives as [`SessionEvent`]s on one
//! channel ([`Session::events`]); nothing here calls back into the
//! caller's code. What a session is asked is `async` where libwebrtc
//! answers on another thread (SDP, candidates, statistics).
//!
//! Build: the prebuilt libwebrtc and the clang that compiles its bridge are
//! set by `scripts/webrtc-toolchain.sh` through `build-env.sh` and
//! `android-env.sh` (`VEYDAN_WEBRTC_DIR`, `VEYDAN_WEBRTC_CLANGXX`); the
//! bridge is the vendored `vendor/webrtc-sys` (its `PATCH.md` says what was
//! changed and why).

pub mod adapter;
pub mod audio;
pub mod camera;
pub mod crypto;
pub mod engine;
pub mod session;
pub mod video;

#[cfg(target_os = "android")]
pub mod android;
#[cfg(not(target_os = "android"))]
pub mod screen;

pub use adapter::{AudioTap, LazyRtcEngine, RtcEngine, VideoTap};
pub use audio::{mix, AudioDevice, AudioDevices, AudioInput, AudioOutput, AudioProcessing, FRAME_SAMPLES, SAMPLE_RATE};
pub use camera::CameraCapture;
pub use crypto::{Encryption, EncryptionState, FrameKeys};
pub use engine::{AudioMode, Engine};
#[cfg(not(target_os = "android"))]
pub use screen::{ScreenCapture, SCREEN_FPS};
pub use session::{
    Candidate, CandidateKind, ConnectionState, IcePolicy, IceServer, IceState, Path, RoomConfig, Session, SessionConfig,
    SessionEvent, Stats, TrackKind, TrackStats, VideoEncoding, VideoLayer,
};
pub use video::{has_test_square, test_pattern, to_i420, PixelFormat, PushedFrame, VideoFrame, VideoOutput, VideoSource};

/// What goes wrong in the engine. The text is libwebrtc's own where it has
/// one; a caller shows it in a log, not to a person.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The engine could not be made or a session could not be created.
    #[error("engine: {0}")]
    Engine(String),
    /// A description or a candidate that libwebrtc does not take.
    #[error("sdp: {0}")]
    Sdp(String),
    /// Something asked in the wrong state, or of a session that is gone.
    #[error("state: {0}")]
    State(String),
    /// A frame of the wrong size or rate on the pushed audio path.
    #[error("audio: {0}")]
    Audio(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<libwebrtc::RtcError> for Error {
    fn from(e: libwebrtc::RtcError) -> Self {
        use libwebrtc::RtcErrorType;
        match e.error_type {
            RtcErrorType::InvalidSdp => Error::Sdp(e.message),
            RtcErrorType::InvalidState => Error::State(e.message),
            RtcErrorType::Internal => Error::Engine(e.message),
        }
    }
}

impl From<libwebrtc::session_description::SdpParseError> for Error {
    fn from(e: libwebrtc::session_description::SdpParseError) -> Self {
        Error::Sdp(e.to_string())
    }
}
