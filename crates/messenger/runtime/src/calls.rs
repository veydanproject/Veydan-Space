// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Calls in the runtime: the `CallService` of `messenger-calls` wired to
//! the session, the outbox and the UI, and the commands the host gives it.
//!
//! The core decides everything about a call and asks for two things to be
//! done outside: a wrap published (`Effect::Send`), an event shown
//! (`Effect::Emit`). A task of this module drains them for as long as the
//! runtime lives: wraps go to the outbox with a kick, as every other
//! signal of the messenger does, and events go on the UI channel
//! (`call.incoming`, `call.state`, `call.ended`, `call.stats`,
//! `call.level`, and the `dm.*` of the call's line in the chat).
//!
//! The media engine comes with the runtime: libwebrtc through
//! `messenger-rtc` behind the feature `rtc`, made on the first call
//! ([`default_engine`]); a host or a test may give another one
//! (`MessengerRuntime::start_with_engine`).
//!
//! Where the nodes come from: the developer setting `call.nodes` (own
//! nodes, by reference) and the `calls` of the project's manifest for the
//! region in use, put into the core's server sets whenever the manifest
//! or the region changes ([`MessengerRuntime::seed_call_nodes`]).
//!
//! The types the UI reads (`CallView` and its words) are spelled here
//! again with their TypeScript, field for field as the core has them; a
//! test keeps the two the same.
//!
//! Video: the frames of a call come out of [`MessengerRuntime::call_video_frames`]
//! as the engine hands them (I420, `VideoFrame` of the core); the host
//! packs them for its page (the app: a binary `tauri::ipc::Channel`,
//! `messenger-app/src/commands/calls.rs`). A phone's camera pushes its
//! frames in through [`MessengerRuntime::call_push_video_frame`].

use crate::MessengerRuntime;
use messenger_calls::engine::{Media, PairKind, RelayPolicy as CorePolicy, VideoInput as CoreVideoInput, VideoTrack as CoreTrack};
use messenger_calls::servers::{parse_own_nodes, CallNode, NodeClass, NodeRef};
use messenger_calls::{
    CallDmHandler, CallService, CallView as CoreView, NodeClient, Phase, ServerSets, SettingsServerSets, VideoQuality as CoreQuality,
    KEY_CALL_NODES,
};
use messenger_core::traits::UiEvent;
use messenger_core::{Effect, MessengerError, Result};
use messenger_dm::DmService;
use messenger_ingress::Outbox;
use messenger_store::{settings, Store};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, oneshot};
use ts_rs::TS;

pub use messenger_calls::engine::{MediaEngine, PixelFormat, PushedFrame, VideoFrame};
pub use messenger_calls::{UI_EVENT_CALL_ENDED, UI_EVENT_CALL_INCOMING, UI_EVENT_CALL_LEVEL, UI_EVENT_CALL_STATE, UI_EVENT_CALL_STATS};

/// What this client says of itself to a call node.
fn client_name() -> String {
    format!("veydan-messenger/{}", messenger_core::VERSION)
}

/// The engine of this build: libwebrtc through the platform's audio
/// device, made on the first call (a machine without a sound card runs
/// the messenger as before and learns of it when it calls).
#[cfg(feature = "rtc")]
pub fn default_engine() -> Option<Arc<dyn MediaEngine>> {
    Some(Arc::new(messenger_rtc::LazyRtcEngine::new(messenger_rtc::AudioMode::Device)))
}

/// A build without the engine (`--no-default-features`) has none: the
/// screen sees `available: false`, a call fails at its start and says so.
#[cfg(not(feature = "rtc"))]
pub fn default_engine() -> Option<Arc<dyn MediaEngine>> {
    None
}

/// What stands in for the engine when there is none.
struct NoEngine;

#[async_trait::async_trait]
impl MediaEngine for NoEngine {
    async fn create_session(&self, _: Vec<messenger_calls::IceServer>, _: CorePolicy, _: Media) -> Result<Box<dyn messenger_calls::Session>> {
        Err(MessengerError::Transport("this build has no media engine (feature rtc)".into()))
    }
}

/// How long the end of a call is given to leave when the session stops
/// under it (the module off, a logout, the app closed): one pump of the
/// outbox, with the relays as they are. What did not leave waits for the
/// next session; the note expires on the relays in five minutes anyway.
pub const GOODBYE_WAIT: Duration = Duration::from_secs(3);

/// The call service and what serves it, owned by the runtime.
pub struct CallsDriver {
    pub service: CallService,
    servers: Arc<SettingsServerSets>,
    /// A media engine is here (of the build, or given by the host): a
    /// call can be made.
    available: bool,
    /// Drains the effects of the service; aborted at shutdown.
    effects: tokio::task::JoinHandle<()>,
    /// Asks the drain to answer once everything queued before is done
    /// (`flush`).
    flush: mpsc::UnboundedSender<oneshot::Sender<()>>,
}

impl CallsDriver {
    /// With `engine` `None` every call fails at its start, and the
    /// state says `available: false`.
    pub fn new(store: Store, dm: DmService, engine: Option<Arc<dyn MediaEngine>>, outbox: Outbox, ui: broadcast::Sender<UiEvent>) -> Self {
        let servers = Arc::new(SettingsServerSets::new(store.clone()));
        let available = engine.is_some();
        let (service, mut rx) = CallService::new(
            store,
            dm,
            engine.unwrap_or_else(|| Arc::new(NoEngine)),
            servers.clone(),
            NodeClient::new(&client_name()),
            Arc::new(messenger_core::traits::SystemClock),
        );
        let (flush, mut flush_rx) = mpsc::unbounded_channel::<oneshot::Sender<()>>();
        let effects = tokio::spawn(async move {
            loop {
                tokio::select! {
                    // The effects come first: a flush is answered only when
                    // none waits, so everything queued before it has been
                    // done (each one runs to its end before the next look).
                    biased;
                    effect = rx.recv() => {
                        let Some(effect) = effect else { break };
                        match effect {
                            // Stored first, published by the pump at once: a signal
                            // of a call is worth nothing late, but a wrap that could
                            // not leave is not the core's problem to hold.
                            Effect::Send(out) => {
                                if let Err(e) = outbox.enqueue(out).await {
                                    eprintln!("messenger calls: a signal was not queued: {e}");
                                }
                                outbox.kick();
                            }
                            Effect::Emit(ev) => {
                                let _ = ui.send(ev);
                            }
                            // The screen hears of a call by `call.incoming`.
                            Effect::Notify(_) => {}
                        }
                    }
                    ask = flush_rx.recv() => {
                        // The driver is gone with its sender: nothing asks any more.
                        let Some(done) = ask else { break };
                        let _ = done.send(());
                    }
                }
            }
        });
        Self { service, servers, available, effects, flush }
    }

    /// The door of the `call.*` envelopes in the chain of DM handlers.
    pub fn handler(&self, next: Arc<dyn messenger_core::Handler<messenger_core::DmInbound>>) -> Arc<CallDmHandler> {
        Arc::new(CallDmHandler::new(self.service.clone(), next))
    }

    /// Resolves once every effect the service gave before this call is
    /// done: its wraps are in the outbox, its events on the UI channel.
    /// At once when the drain is stopped.
    pub async fn flush(&self) {
        let (tx, rx) = oneshot::channel();
        if self.flush.send(tx).is_ok() {
            let _ = rx.await;
        }
    }

    pub fn stop(&self) {
        self.effects.abort();
    }
}

// ─── What the UI reads ───────────────────────────────────────────────────────

/// What a call carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CallMedia {
    Audio,
    Video,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CallDirection {
    In,
    Out,
}

/// Where a call is. `reconnecting` is `active` with the way lost: a
/// restart of ICE is under way, and `reconnect_reason` says why.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CallPhase {
    Outgoing,
    Incoming,
    Connecting,
    Active,
    Reconnecting,
    Ended,
}

/// Why a call is `reconnecting`: the engine saw the way go, my network
/// changed under the call, or the peer lost the way (it asked for a new
/// offer, or made one).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReconnectReason {
    ConnectionLost,
    NetworkChanged,
    PeerLost,
}

/// How the media goes: directly between the two, or through a relay.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CallVia {
    Direct,
    Relay,
}

/// How a call ended, in `call.ended` (`answered_elsewhere` is of the
/// screen only: another device of mine took the call).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CallOutcome {
    Missed,
    Declined,
    Busy,
    Ended,
    Failed,
    AnsweredElsewhere,
}

/// Which ICE candidates a call may use: `auto` is ICE as it is, a direct
/// pair when one works and a relay when none does; `relay_only` hides my
/// address behind a relay, always.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RelayPolicy {
    #[default]
    Auto,
    RelayOnly,
}

/// Which video of a call: mine as the camera sees it (the small picture
/// of myself), or the peer's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum VideoTrack {
    Local,
    Remote,
}

/// What I send as video: nothing, a camera (by the id
/// `messenger_call_list_cameras` gives, or the default; on a phone
/// `front` or `back`), or a screen or window (by the id of
/// `messenger_call_list_screens`, or the first screen; a computer only).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VideoInput {
    Off,
    Camera {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        id: Option<String>,
    },
    Screen {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        id: Option<String>,
    },
}

/// How big my video is sent: 640×360 (the default) or 1280×720, both at
/// 30 frames a second; the engine scales down by itself when the way is
/// narrower.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum VideoQuality {
    #[default]
    #[serde(rename = "360p")]
    Sd,
    #[serde(rename = "720p")]
    Hd,
}

/// A camera the engine can open.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CameraInfo {
    /// What `VideoInput::Camera` and `messenger_call_switch_camera` take:
    /// a device path, an index, whatever the platform names a camera by.
    pub id: String,
    pub name: String,
}

/// A screen or a window the engine can share.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ScreenInfo {
    /// What `VideoInput::Screen` takes.
    pub id: String,
    pub title: String,
    /// A window rather than a whole screen.
    pub window: bool,
}

/// The size of the frames of one video as they show (a phone held
/// upright gives a tall one).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct VideoSize {
    pub width: u32,
    pub height: u32,
}

/// What the nearest node allows, as it said. Nothing of it is a number of
/// the client's own.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CallLimits {
    pub turn_lifetime_secs: u32,
    pub turn_kbps_per_allocation: u32,
    pub credentials_ttl_secs: u32,
}

/// The call as the screen shows it: the payload `call` of every `call.*`
/// event and the answer of the call commands.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct CallView {
    pub call_id: String,
    /// The peer, hex.
    pub peer: String,
    pub chat_id: String,
    pub direction: CallDirection,
    pub media: CallMedia,
    pub phase: CallPhase,
    /// How the media goes, once ICE settled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub via: Option<CallVia>,
    /// Why the call is `reconnecting`; absent in every other phase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reconnect_reason: Option<ReconnectReason>,
    pub muted: bool,
    /// When the invitation was made, unix seconds.
    #[ts(type = "number")]
    pub started_at: i64,
    /// When it was taken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub answered_at: Option<i64>,
    /// The ids of the nodes this side uses, nearest first; empty when the
    /// call goes with host candidates alone.
    #[serde(default)]
    pub nodes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub limits: Option<CallLimits>,
    /// My video goes: the camera, or the screen (`video_screen`).
    #[serde(default)]
    pub video_local: bool,
    /// What I send is my screen, not my camera.
    #[serde(default)]
    pub video_screen: bool,
    /// The camera in use (or the one for the next time), by the id the
    /// engine lists; absent for its default. On a phone `front` or `back`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub camera: Option<String>,
    /// The peer's video goes, by its word (`call.video`) or by its
    /// invitation; the screen shows a placeholder until frames come.
    #[serde(default)]
    pub video_remote: bool,
    /// The size of the frames each way, once some came; absent while that
    /// video is off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub video_local_size: Option<VideoSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub video_remote_size: Option<VideoSize>,
}

/// The payload of `call.ended`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct CallEnded {
    pub call: CallView,
    pub outcome: CallOutcome,
    /// How long the call was answered, seconds; `null` for one that never was.
    #[ts(type = "number | null")]
    pub duration_secs: Option<i64>,
}

/// What the engine tells of a running call, the payload `stats` of
/// `call.stats`. All optional: an engine tells what it has.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct CallStats {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub rtt_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub bytes_sent: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub bytes_received: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub packets_lost: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub jitter_ms: Option<u32>,
}

/// A call node the client may use, as the settings list it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CallNodeView {
    /// `address:port#id`.
    pub reference: String,
    /// The node's id, hex: what the TLS of its control channel is pinned to.
    pub id: String,
    /// Whose it is: `own` (the setting), `project` (the manifest).
    pub class: String,
    /// A key of a private node is kept for it.
    pub has_key: bool,
}

/// One own node, as the settings take it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CallNodeInput {
    /// `address:port#id`.
    pub reference: String,
    /// The access key of a private node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub key: Option<String>,
}

/// Calls as the settings and the screen see them now.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct CallState {
    /// The call under way, if any.
    pub call: Option<CallView>,
    pub policy: RelayPolicy,
    /// Every node a call may use, the most preferred first.
    pub nodes: Vec<CallNodeView>,
    /// This build can make a call (it has a media engine).
    pub available: bool,
    /// How big my video is sent, for the next time it goes on.
    #[serde(default)]
    pub video_quality: VideoQuality,
    /// This device takes calls (on by default). Off: every invitation is
    /// ignored here without a word, so that my other devices ring for it;
    /// nothing goes on record on this device.
    #[serde(default = "yes")]
    pub incoming_enabled: bool,
}

fn yes() -> bool {
    true
}

impl From<CoreQuality> for VideoQuality {
    fn from(q: CoreQuality) -> Self {
        match q {
            CoreQuality::Sd => Self::Sd,
            CoreQuality::Hd => Self::Hd,
        }
    }
}

impl From<VideoQuality> for CoreQuality {
    fn from(q: VideoQuality) -> Self {
        match q {
            VideoQuality::Sd => Self::Sd,
            VideoQuality::Hd => Self::Hd,
        }
    }
}

impl From<VideoInput> for CoreVideoInput {
    fn from(v: VideoInput) -> Self {
        match v {
            VideoInput::Off => Self::Off,
            VideoInput::Camera { id } => Self::Camera { id },
            VideoInput::Screen { id } => Self::Screen { id },
        }
    }
}

impl From<VideoTrack> for CoreTrack {
    fn from(t: VideoTrack) -> Self {
        match t {
            VideoTrack::Local => Self::Local,
            VideoTrack::Remote => Self::Remote,
        }
    }
}

impl From<CorePolicy> for RelayPolicy {
    fn from(p: CorePolicy) -> Self {
        match p {
            CorePolicy::Auto => Self::Auto,
            CorePolicy::RelayOnly => Self::RelayOnly,
        }
    }
}

impl From<RelayPolicy> for CorePolicy {
    fn from(p: RelayPolicy) -> Self {
        match p {
            RelayPolicy::Auto => Self::Auto,
            RelayPolicy::RelayOnly => Self::RelayOnly,
        }
    }
}

impl From<CallMedia> for Media {
    fn from(m: CallMedia) -> Self {
        match m {
            CallMedia::Audio => Self::Audio,
            CallMedia::Video => Self::Video,
        }
    }
}

impl From<CoreView> for CallView {
    fn from(v: CoreView) -> Self {
        use messenger_calls::{Direction, Phase};
        Self {
            call_id: v.call_id,
            peer: v.peer,
            chat_id: v.chat_id,
            direction: match v.direction {
                Direction::In => CallDirection::In,
                Direction::Out => CallDirection::Out,
            },
            media: match v.media {
                Media::Audio => CallMedia::Audio,
                Media::Video => CallMedia::Video,
            },
            phase: match v.phase {
                Phase::Outgoing => CallPhase::Outgoing,
                Phase::Incoming => CallPhase::Incoming,
                Phase::Connecting => CallPhase::Connecting,
                Phase::Active => CallPhase::Active,
                Phase::Reconnecting => CallPhase::Reconnecting,
                Phase::Ended => CallPhase::Ended,
            },
            via: v.via.map(|p| match p {
                PairKind::Direct => CallVia::Direct,
                PairKind::Relay => CallVia::Relay,
            }),
            reconnect_reason: v.reconnect_reason.map(|r| match r {
                messenger_calls::ReconnectReason::ConnectionLost => ReconnectReason::ConnectionLost,
                messenger_calls::ReconnectReason::NetworkChanged => ReconnectReason::NetworkChanged,
                messenger_calls::ReconnectReason::PeerLost => ReconnectReason::PeerLost,
            }),
            muted: v.muted,
            started_at: v.started_at,
            answered_at: v.answered_at,
            nodes: v.nodes,
            limits: v.limits.map(|l| CallLimits {
                turn_lifetime_secs: l.turn_lifetime_secs,
                turn_kbps_per_allocation: l.turn_kbps_per_allocation,
                credentials_ttl_secs: l.credentials_ttl_secs,
            }),
            video_local: v.video_local,
            video_screen: v.video_screen,
            camera: v.camera,
            video_remote: v.video_remote,
            video_local_size: v.video_local_size.map(|s| VideoSize { width: s.width, height: s.height }),
            video_remote_size: v.video_remote_size.map(|s| VideoSize { width: s.width, height: s.height }),
        }
    }
}

fn node_view(n: &CallNode) -> CallNodeView {
    CallNodeView { reference: n.node.to_string(), id: n.node.id.to_string(), class: n.class.as_str().into(), has_key: n.access_key.is_some() }
}

// ─── The runtime's side ──────────────────────────────────────────────────────

impl MessengerRuntime {
    pub fn calls(&self) -> &CallService {
        &self.calls.service
    }

    /// Call `peer` (hex or npub). Refused for anybody we are not in a
    /// mutual chat with, and while a call is under way.
    pub async fn call_start(&self, peer: &str, media: CallMedia) -> Result<CallView> {
        let pk = messenger_contacts::book::parse_key(peer)?;
        self.calls.service.start(&pk, media.into()).await.map(Into::into)
    }

    /// Take the ringing call.
    pub async fn call_accept(&self, call_id: &str) -> Result<CallView> {
        self.calls.service.accept(call_id).await.map(Into::into)
    }

    /// Refuse the ringing call.
    pub async fn call_decline(&self, call_id: &str) -> Result<()> {
        self.calls.service.decline(call_id).await
    }

    /// Hang up, or give up calling.
    pub async fn call_end(&self, call_id: &str) -> Result<()> {
        self.calls.service.end(call_id).await
    }

    pub async fn call_set_mute(&self, muted: bool) -> Result<CallView> {
        self.calls.service.set_mute(muted).await.map(Into::into)
    }

    /// My video in the call under way: a camera, a screen, or off. The
    /// peer is told; nothing is renegotiated. A camera that will not open
    /// is an error, and the call goes on without the video.
    pub async fn call_set_video(&self, input: VideoInput) -> Result<CallView> {
        self.calls.service.set_video(input.into()).await.map(Into::into)
    }

    /// The next camera of the engine's list (or the one `camera` names):
    /// switched at once when my camera is on, kept for when it goes on
    /// otherwise. On a phone the list is `front`, `back`.
    pub async fn call_switch_camera(&self, camera: Option<String>) -> Result<CallView> {
        self.calls.service.switch_camera(camera).await.map(Into::into)
    }

    /// The cameras the engine can open, the default first; empty on a
    /// phone (its plugin holds the camera) and on a machine without one.
    pub async fn call_cameras(&self) -> Vec<CameraInfo> {
        self.calls.service.cameras().await.into_iter().map(|c| CameraInfo { id: c.id, name: c.name }).collect()
    }

    /// The screens and windows the engine can share (a computer with a
    /// display); empty elsewhere.
    pub async fn call_screens(&self) -> Vec<ScreenInfo> {
        self.calls.service.screens().await.into_iter().map(|s| ScreenInfo { id: s.id, title: s.title, window: s.window }).collect()
    }

    /// For the next time my video goes on; what is on keeps its size.
    pub async fn call_set_video_quality(&self, quality: VideoQuality) -> Result<CallState> {
        self.calls.service.set_video_quality(quality.into()).await?;
        self.call_state().await
    }

    /// The frames of one video of the call under way, as the engine hands
    /// them: `None` without a call or before its media is there. A reader
    /// that falls behind skips to the newest; the stream closes with the
    /// session.
    pub async fn call_video_frames(&self, track: VideoTrack) -> Option<broadcast::Receiver<Arc<VideoFrame>>> {
        self.calls.service.video_frames(track.into()).await
    }

    /// A frame the platform captured (the camera of a phone, through the
    /// plugin: NV21 with its rotation), as my video of the call under way.
    /// Refused without a call. The engine converts and sends it when my
    /// video is on (`video_local`), and drops it otherwise.
    pub async fn call_push_video_frame(&self, frame: PushedFrame) -> Result<()> {
        self.calls.service.push_video_frame(frame).await
    }

    /// The call under way, the policy and the nodes.
    pub async fn call_state(&self) -> Result<CallState> {
        let nodes = self.calls.servers.call_nodes().await?;
        Ok(CallState {
            call: self.calls.service.current().await.map(Into::into),
            policy: self.calls.service.policy().await?.into(),
            nodes: nodes.iter().map(node_view).collect(),
            available: self.calls.available,
            video_quality: self.calls.service.video_quality().await?.into(),
            incoming_enabled: self.calls.service.incoming_enabled().await?,
        })
    }

    /// Whether this device takes calls (`call.incoming_enabled`). Off:
    /// every invitation is ignored here without a word (no ring, no
    /// decline, no busy), my other devices ring for it, and this device
    /// keeps no record of it. A call ringing now goes on ringing.
    pub async fn call_set_incoming(&self, enabled: bool) -> Result<CallState> {
        self.calls.service.set_incoming_enabled(enabled).await?;
        self.call_state().await
    }

    /// The platform saw the network change (an interface came or went:
    /// the phone's connectivity callback, the page's `online` event): the
    /// call under way restarts ICE at once instead of waiting for the
    /// engine to notice the way is gone. Nothing without a call.
    pub async fn call_network_changed(&self) {
        self.calls.service.network_changed().await;
    }

    /// For the next call; the current one keeps its way.
    pub async fn call_set_policy(&self, policy: RelayPolicy) -> Result<CallState> {
        self.calls.service.set_policy(policy.into()).await?;
        self.call_state().await
    }

    /// My own nodes, replacing the list (`call.nodes`). A reference that
    /// is not `address:port#id` is refused, and the list stays.
    pub async fn call_set_nodes(&self, nodes: Vec<CallNodeInput>) -> Result<CallState> {
        let mut own = Vec::with_capacity(nodes.len());
        for n in nodes {
            let node: NodeRef = n
                .reference
                .parse()
                .map_err(|_| MessengerError::Invalid(format!("not a node reference (address:port#id): {}", n.reference)))?;
            own.push(CallNode { node, class: NodeClass::Own, access_key: n.key.filter(|k| !k.is_empty()) });
        }
        self.calls.servers.set_own(&own).await?;
        self.call_state().await
    }

    /// The project's call nodes for the region in use, from the manifest
    /// in use, into the core's server sets. Called at the start and
    /// whenever the servers changed.
    pub(crate) async fn seed_call_nodes(&self) -> Result<()> {
        let (manifest, _) = self.relays.current_manifest().await?;
        let region = self.relays.region().await?;
        let nodes: Vec<NodeRef> = manifest.calls_for_region(&region).iter().filter_map(|c| c.node.parse().ok()).collect();
        self.calls.servers.set_manifest(nodes);
        Ok(())
    }

    /// The session is about to stop under a call (the module off, a
    /// logout, the app closed): the call ends as a hang-up, so the peer
    /// hears of it now and not from a connection that goes quiet. The end
    /// is in the outbox when this returns; `true` says one was queued. A
    /// call still ringing in is left alone: a quit is no decline, the
    /// caller's own timer makes it a missed one.
    pub(crate) async fn end_call_before_leaving(&self) -> bool {
        let Some(call) = self.calls.service.current().await else { return false };
        if call.phase == Phase::Incoming {
            return false;
        }
        if let Err(e) = self.calls.service.end(&call.call_id).await {
            eprintln!("messenger calls: the call was not ended before leaving: {e}");
            return false;
        }
        self.calls.flush().await;
        true
    }

    /// The own nodes as the setting has them (for the CLI's `--node`:
    /// what was set before).
    pub async fn call_own_nodes(&self) -> Result<Vec<CallNodeView>> {
        let own = match settings::get(&self.store, KEY_CALL_NODES).await? {
            Some(json) => parse_own_nodes(&json),
            None => Vec::new(),
        };
        Ok(own.iter().map(node_view).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::servers::use_veydan_offline;
    use messenger_core::MessengerConfig;
    use messenger_testkit::{FakeEngine, MemorySecretStore};

    const NODE: &str = "108.61.171.68:8443#fda09da75199c4e04601a9df309710fab15cd7b2b806ca3b2bbab202582a6dca";

    /// The view of the runtime is the view of the core, field for field:
    /// the events carry the core's JSON, the commands answer the runtime's.
    #[test]
    fn the_view_of_the_ui_is_the_view_of_the_core() {
        let core = CoreView {
            call_id: "c1".into(),
            peer: "ab".repeat(32),
            chat_id: format!("dm:{}", "ab".repeat(32)),
            direction: messenger_calls::Direction::In,
            media: Media::Video,
            phase: messenger_calls::Phase::Reconnecting,
            via: Some(PairKind::Relay),
            reconnect_reason: Some(messenger_calls::ReconnectReason::NetworkChanged),
            muted: true,
            started_at: 100,
            answered_at: Some(105),
            nodes: vec!["n1".into()],
            limits: Some(messenger_calls::node_client::Limits { turn_lifetime_secs: 3600, turn_kbps_per_allocation: 2000, credentials_ttl_secs: 600 }),
            video_local: true,
            video_screen: true,
            camera: Some("/dev/video1".into()),
            video_remote: true,
            video_local_size: Some(messenger_calls::VideoSize { width: 1280, height: 720 }),
            video_remote_size: Some(messenger_calls::VideoSize { width: 360, height: 640 }),
        };
        let json = serde_json::to_value(&core).unwrap();
        let view: CallView = core.clone().into();
        assert_eq!(serde_json::to_value(&view).unwrap(), json);
        assert_eq!(serde_json::from_value::<CallView>(json).unwrap(), view);
        // Without the optional parts the JSON is the same too.
        let bare = CoreView {
            via: None,
            reconnect_reason: None,
            phase: messenger_calls::Phase::Active,
            answered_at: None,
            limits: None,
            nodes: vec![],
            camera: None,
            video_local_size: None,
            video_remote_size: None,
            ..core
        };
        let json = serde_json::to_value(&bare).unwrap();
        assert_eq!(serde_json::to_value(CallView::from(bare)).unwrap(), json);
        assert!(json.get("via").is_none());
        assert!(json.get("reconnect_reason").is_none());
        // The words of the outcomes and the policies are the core's.
        for (o, word) in [
            (CallOutcome::Missed, messenger_calls::Outcome::Missed.as_str()),
            (CallOutcome::Failed, messenger_calls::Outcome::Failed.as_str()),
            (CallOutcome::AnsweredElsewhere, messenger_calls::ANSWERED_ELSEWHERE),
        ] {
            assert_eq!(serde_json::to_value(o).unwrap(), serde_json::json!(word));
        }
        assert_eq!(serde_json::to_value(RelayPolicy::RelayOnly).unwrap(), serde_json::to_value(CorePolicy::RelayOnly).unwrap());
        assert_eq!(serde_json::to_value(CallStats::default()).unwrap(), serde_json::to_value(messenger_calls::SessionStats::default()).unwrap());
        // The words of the video are the core's too.
        for (mine, core) in [
            (VideoInput::Off, CoreVideoInput::Off),
            (VideoInput::Camera { id: None }, CoreVideoInput::Camera { id: None }),
            (VideoInput::Screen { id: Some("screen:1".into()) }, CoreVideoInput::Screen { id: Some("screen:1".into()) }),
        ] {
            assert_eq!(serde_json::to_value(&mine).unwrap(), serde_json::to_value(&core).unwrap());
            assert_eq!(CoreVideoInput::from(mine), core);
        }
        assert_eq!(serde_json::to_value(VideoInput::Camera { id: Some("front".into()) }).unwrap(), serde_json::json!({ "kind": "camera", "id": "front" }));
        assert_eq!(serde_json::to_value(VideoTrack::Remote).unwrap(), serde_json::to_value(CoreTrack::Remote).unwrap());
        assert_eq!(serde_json::to_value(VideoQuality::Hd).unwrap(), serde_json::to_value(CoreQuality::Hd).unwrap());
        assert_eq!(serde_json::to_value(VideoQuality::Sd).unwrap(), serde_json::json!("360p"));
    }

    #[tokio::test]
    async fn the_settings_of_calls_and_the_nodes_of_the_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let secrets = Arc::new(MemorySecretStore::unlocked());
        let rt = MessengerRuntime::start_with_engine(cfg, secrets, Arc::new(FakeEngine::new())).await.unwrap();
        rt.relays().set_silent(true).await.unwrap();
        use_veydan_offline(&rt).await;

        let st = rt.call_state().await.unwrap();
        assert!(st.call.is_none());
        assert_eq!(st.policy, RelayPolicy::Auto);
        // The embedded manifest names the nodes of the project; nothing of mine yet.
        let embedded = messenger_transport::manifest::Manifest::parse_content(messenger_transport::manifest::EMBEDDED_MANIFEST_JSON).unwrap().calls.len();
        assert!(embedded > 0, "the embedded manifest names the nodes of the project");
        assert_eq!(st.nodes.len(), embedded, "{:?}", st.nodes);
        assert!(st.nodes.iter().all(|n| n.class == "project"), "{:?}", st.nodes);
        assert!(st.available, "the engine was given");
        assert_eq!(st.video_quality, VideoQuality::Sd);
        assert!(st.incoming_enabled, "a device takes calls unless told otherwise");
        let st = rt.call_set_video_quality(VideoQuality::Hd).await.unwrap();
        assert_eq!(st.video_quality, VideoQuality::Hd);
        let st = rt.call_set_incoming(false).await.unwrap();
        assert!(!st.incoming_enabled);
        assert!(rt.call_set_incoming(true).await.unwrap().incoming_enabled);
        // Without a call a change of the network is nothing.
        rt.call_network_changed().await;
        // Video without a call: nothing to turn on, no frames, no cameras
        // on the fake (a phone's plugin lists them), no screens either.
        assert!(matches!(rt.call_set_video(VideoInput::Camera { id: None }).await, Err(MessengerError::Invalid(_))));
        assert!(matches!(rt.call_switch_camera(None).await, Err(MessengerError::Invalid(_))));
        assert!(rt.call_video_frames(VideoTrack::Remote).await.is_none());
        assert!(rt.call_cameras().await.is_empty() && rt.call_screens().await.is_empty());
        let frame = PushedFrame { format: PixelFormat::Nv21, width: 2, height: 2, rotation: 0, timestamp_us: 0, data: vec![0; 6] };
        assert!(matches!(rt.call_push_video_frame(frame).await, Err(MessengerError::Invalid(_))));

        let st = rt.call_set_policy(RelayPolicy::RelayOnly).await.unwrap();
        assert_eq!(st.policy, RelayPolicy::RelayOnly);
        assert!(rt.call_set_nodes(vec![CallNodeInput { reference: "not a node".into(), key: None }]).await.is_err());
        let st = rt.call_set_nodes(vec![CallNodeInput { reference: NODE.into(), key: Some("k".into()) }]).await.unwrap();
        // My node is the project's eu-1 too: mine takes its place in the list.
        assert_eq!(st.nodes.len(), embedded, "mine first, then the project's others: {:?}", st.nodes);
        assert_eq!((st.nodes[0].class.as_str(), st.nodes[0].has_key), ("own", true));
        assert_eq!(st.nodes[0].reference, NODE);
        assert_eq!(rt.call_own_nodes().await.unwrap(), st.nodes[..1]);

        // A manifest with a node of the project: listed after mine.
        let (mut manifest, origin) = rt.relays().current_manifest().await.unwrap();
        manifest.serial += 1;
        manifest.calls.push(messenger_transport::manifest::ManifestCall {
            node: format!("45.93.201.244:443#{}", "cd".repeat(32)),
            turn_port: 3478,
            sfu_port: 0,
            class: messenger_transport::manifest::CallNodeClass::Project,
            regions: vec!["*".into()],
        });
        rt.relays().adopt_manifest(&manifest, &origin, false).await.unwrap();
        rt.seed_call_nodes().await.unwrap();
        let st = rt.call_state().await.unwrap();
        let classes = st.nodes.iter().map(|n| n.class.as_str()).collect::<Vec<_>>();
        assert_eq!(classes[0], "own");
        assert_eq!(classes.len(), embedded + 1);
        assert!(classes[1..].iter().all(|c| *c == "project"), "{classes:?}");

        // No session: a call cannot be started, and says so.
        assert!(matches!(rt.call_start(&"ab".repeat(32), CallMedia::Audio).await, Err(MessengerError::NotLoggedIn)));
        assert!(rt.call_end("nothing").await.is_err());
        rt.shutdown().await;
    }

    /// With keys, a stranger cannot be called; a contact in a mutual chat
    /// can, and the invitation leaves through the outbox.
    #[tokio::test]
    async fn a_call_to_a_contact_queues_the_invitation() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let secrets = Arc::new(MemorySecretStore::unlocked());
        let engine = FakeEngine::new();
        let rt = MessengerRuntime::start_with_engine(cfg, secrets, Arc::new(engine.clone())).await.unwrap();
        rt.relays().set_silent(true).await.unwrap();
        use_veydan_offline(&rt).await;
        rt.identity().create("pw").await.unwrap();
        rt.refresh_signer().await.unwrap();
        let peer = nostr::key::Keys::generate().public_key().to_hex();
        let refused = rt.call_start(&peer, CallMedia::Audio).await;
        assert!(matches!(refused, Err(MessengerError::Invalid(_))), "{refused:?}");

        // With the gate of the relationships off every chat is a full one.
        rt.dm().set_gate(false);
        let pk = messenger_core::PubKey::parse(&peer).unwrap();
        assert!(rt.dm().calls_allowed(&pk).await.unwrap());
        let mut events = rt.ui_events();
        let view = rt.call_start(&peer, CallMedia::Audio).await.unwrap();
        assert_eq!((view.phase, view.direction), (CallPhase::Outgoing, CallDirection::Out));
        assert_eq!(rt.call_state().await.unwrap().call.as_ref().map(|c| c.call_id.as_str()), Some(view.call_id.as_str()));
        // My video in the call: on, with the engine's door for pushed
        // frames open and the frames of both videos readable; then off.
        assert!(!view.video_local);
        let on = rt.call_set_video(VideoInput::Camera { id: None }).await.unwrap();
        assert!(on.video_local && !on.video_screen);
        let mut local = rt.call_video_frames(VideoTrack::Local).await.expect("the frames of my video");
        assert!(rt.call_video_frames(VideoTrack::Remote).await.is_some());
        let frame = PushedFrame { format: PixelFormat::Nv21, width: 4, height: 2, rotation: 90, timestamp_us: 0, data: vec![0; 12] };
        rt.call_push_video_frame(frame).await.unwrap();
        let shown = local.try_recv().expect("the pushed frame shows as my own picture");
        assert_eq!((shown.width, shown.height, shown.rotation), (4, 2, 90));
        let switched = rt.call_switch_camera(None).await.unwrap();
        assert_eq!(switched.camera.as_deref(), Some("back"), "a phone's cameras when the engine lists none");
        let off = rt.call_set_video(VideoInput::Off).await.unwrap();
        assert!(!off.video_local);
        assert_eq!(engine.sessions()[0].record().video.len(), 3);
        // The fake gathers at once; the offer leaves after the gathering wait.
        tokio::time::sleep(messenger_calls::GATHER_WAIT + std::time::Duration::from_millis(300)).await;
        let mut seen = Vec::new();
        while let Ok(ev) = events.try_recv() {
            seen.push(ev.name);
        }
        assert!(seen.iter().any(|n| n == UI_EVENT_CALL_STATE), "{seen:?}");
        assert!(rt.outbox().pending().await.unwrap() >= 1, "the invitation (and its copy) wait in the outbox");
        rt.call_end(&view.call_id).await.unwrap();
        assert!(rt.call_state().await.unwrap().call.is_none());
        assert_eq!(engine.sessions().len(), 1);
        assert!(engine.sessions()[0].record().closed);
        rt.shutdown().await;
    }

    /// The session stops under a call (here: the secrets lock, as on a
    /// logout or the module going off): the call is hung up while the keys
    /// are still here, so the end for the peer waits in the outbox by the
    /// time the session is gone, and the screen hears of the end.
    #[tokio::test]
    async fn leaving_in_a_call_says_goodbye_to_the_peer() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let secrets = Arc::new(MemorySecretStore::unlocked());
        let engine = FakeEngine::new();
        let rt = MessengerRuntime::start_with_engine(cfg, secrets.clone(), Arc::new(engine.clone())).await.unwrap();
        rt.relays().set_silent(true).await.unwrap();
        use_veydan_offline(&rt).await;
        rt.identity().create("pw").await.unwrap();
        rt.refresh_signer().await.unwrap();
        rt.dm().set_gate(false);
        let peer = nostr::key::Keys::generate().public_key().to_hex();
        let view = rt.call_start(&peer, CallMedia::Audio).await.unwrap();
        // The invitation has left: the peer rings, and must be told to stop.
        tokio::time::sleep(messenger_calls::GATHER_WAIT + std::time::Duration::from_millis(300)).await;
        let before = rt.outbox().pending().await.unwrap();
        let mut events = rt.ui_events();

        secrets.set_unlocked(false);
        assert!(rt.refresh_signer().await.unwrap(), "locked secrets stop the session");
        assert!(!rt.status().await.unwrap().session_active);
        assert!(rt.call_state().await.unwrap().call.is_none());
        let after = rt.outbox().pending().await.unwrap();
        assert!(after > before, "the end for the peer (and its copy) wait in the outbox: {before} -> {after}");
        assert!(engine.sessions()[0].record().closed);
        let mut ended = None;
        while let Ok(ev) = events.try_recv() {
            if ev.name == UI_EVENT_CALL_ENDED {
                ended = Some(ev.payload);
            }
        }
        let ended = ended.expect("call.ended was emitted");
        assert_eq!(ended["call"]["call_id"], serde_json::json!(view.call_id));
        assert_eq!(ended["outcome"], serde_json::json!("missed"), "hung up before it was answered: {ended}");
        rt.shutdown().await;
    }

    /// The build without the engine (`--no-default-features`) says so:
    /// the screen sees `available: false`, and a call fails at its start
    /// with the reason, leaving nothing under way. (A given engine makes
    /// the same build `available`: the test of the settings above.)
    #[cfg(not(feature = "rtc"))]
    #[tokio::test]
    async fn without_the_engine_a_call_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let secrets = Arc::new(MemorySecretStore::unlocked());
        let rt = MessengerRuntime::start(cfg, secrets).await.unwrap();
        rt.relays().set_silent(true).await.unwrap();
        use_veydan_offline(&rt).await;
        assert!(!rt.call_state().await.unwrap().available);
        rt.identity().create("pw").await.unwrap();
        rt.refresh_signer().await.unwrap();
        rt.dm().set_gate(false);
        let peer = nostr::key::Keys::generate().public_key().to_hex();
        let err = rt.call_start(&peer, CallMedia::Audio).await.unwrap_err();
        assert!(err.to_string().contains("no media engine"), "{err}");
        assert!(rt.call_state().await.unwrap().call.is_none());
        rt.shutdown().await;
    }
}
