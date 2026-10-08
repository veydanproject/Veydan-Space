// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Calls, the core (the plan of calls, «Клиент → calls/»): what a
//! call is and how it goes, without a media engine and without a host.
//!
//! - `engine`: the [`MediaEngine`] and [`Session`] an engine implements
//!   (`messenger-rtc` on libwebrtc, the fake of `messenger-testkit`).
//! - `signal`: the `call.*` envelopes inside NIP-17, read and written.
//! - `service`: [`CallService`], the state of the one call under way,
//!   its commands and what it does with what the peer and the engine say.
//! - `handler`: [`CallDmHandler`], the door in the chain of DM handlers.
//! - `servers`: [`ServerSets`], where the nodes come from, by priority.
//! - `node_client`: the control channel of a call node, credentials, the
//!   choice of the nearest nodes.
//! - `feed`: the record of a call and its line in the chat.
//! - `call`: what the screen sees, and the names of its events.
//!
//! Wire format: internal/messenger-wire.md §10.

pub mod call;
pub mod engine;
pub mod feed;
pub mod handler;
pub mod node_client;
pub mod servers;
pub mod service;
pub mod signal;

pub use call::{
    CallView, Direction, Outcome, Phase, ReconnectReason, VideoSize, UI_EVENT_CALL_ENDED, UI_EVENT_CALL_INCOMING,
    UI_EVENT_CALL_LEVEL, UI_EVENT_CALL_STATE, UI_EVENT_CALL_STATS,
};
pub use engine::{
    CameraInfo, ConnectionState, IceCandidate, IceServer, Media, MediaEngine, PairKind, PixelFormat, PushedFrame, RelayPolicy,
    ScreenInfo, SdpKind, Session, SessionEvent, SessionStats, VideoFrame, VideoInput, VideoSettings, VideoTrack,
};
pub use handler::CallDmHandler;
pub use node_client::{NodeClient, Picked};
pub use servers::{CallNode, NodeClass, NodeRef, ServerSets, SettingsServerSets, StaticServerSets, KEY_CALL_NODES, KEY_RELAY_POLICY};
pub use service::{
    CallService, VideoQuality, ANSWERED_ELSEWHERE, CONNECT_TIMEOUT, GATHER_WAIT, ICE_DEBOUNCE, KEY_INCOMING_ENABLED,
    KEY_VIDEO_QUALITY, LOSS_CONFIRM, RESTART_SETTLE, RING_TIMEOUT, VIDEO_FPS,
};
pub use signal::{Signal, INVITE_TTL_SECS};
