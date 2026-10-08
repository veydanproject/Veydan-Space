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
//! - `servers`: [`ServerSets`], where the nodes come from, by priority;
//!   the trust level, the private nodes this device was invited to.
//! - `registry`: the signed list of volunteers' nodes, cached and
//!   refreshed in the background.
//! - `node_client`: the control channel of a call node, credentials, the
//!   exchange of an invitation, the choice among the nodes by distance
//!   and load.
//! - `feed`: the record of a call and its line in the chat.
//! - `call`: what the screen sees, and the names of its events.
//! - `group`: the calls of a group on an SFU room ([`GroupCallService`]).
//!
//! Wire format: internal/messenger-wire.md §10.

pub mod call;
pub mod engine;
pub mod feed;
pub mod group;
pub mod handler;
pub mod node_client;
pub mod registry;
pub mod servers;
pub mod service;
pub mod signal;

pub use call::{
    CallView, Direction, Outcome, Phase, ReconnectReason, VideoSize, UI_EVENT_CALL_ENDED, UI_EVENT_CALL_INCOMING,
    UI_EVENT_CALL_LEVEL, UI_EVENT_CALL_STATE, UI_EVENT_CALL_STATS,
};
pub use engine::{
    CameraInfo, ConnectionState, DataPayload, IceCandidate, IceServer, Media, MediaEngine, PairKind, PixelFormat, PushedFrame,
    RelayPolicy, RoomConfig, ScreenInfo, SdpKind, Session, SessionEvent, SessionStats, VideoFrame, VideoInput, VideoSettings,
    VideoTrack, CTL_LABEL,
};
pub use group::{
    AnnouncedCall, GroupAccess, GroupCallService, GroupCallView, GroupPhase, GroupSignal, ParticipantView, MY_VIDEO_MID,
    UI_EVENT_GROUP_CALL_ENDED, UI_EVENT_GROUP_CALL_LEVEL, UI_EVENT_GROUP_CALL_STARTED, UI_EVENT_GROUP_CALL_STATE,
};
pub use handler::CallDmHandler;
pub use node_client::{
    Access, Delegated, DeviceAccess, DeviceIssued, HttpRooms, InviteRequest, Joined, KnownNode, MediaLimits, NodeClient, NodeError, NodeLoad,
    Picked, RoomApi, RoomCreated, CAP_CASCADE, CAP_SFU,
};
pub use registry::{ListFetch, Registry, RegistryNode};
pub use servers::{
    CallNode, DeviceCredentials, DeviceNodeEntry, NodeClass, NodeDescription, NodeRef, NodeSource, ServerSets, SettingsServerSets,
    StaticServerSets, TrustLevel, KEY_CALL_DEVICES, KEY_CALL_NODES, KEY_CALL_TRUST, KEY_RELAY_POLICY,
};
pub use service::{
    CallService, VideoQuality, ANSWERED_ELSEWHERE, CONNECT_TIMEOUT, GATHER_WAIT, ICE_DEBOUNCE, KEY_INCOMING_ENABLED,
    KEY_VIDEO_QUALITY, LOSS_CONFIRM, RESTART_SETTLE, RING_TIMEOUT, VIDEO_FPS,
};
pub use signal::{Signal, INVITE_TTL_SECS};
