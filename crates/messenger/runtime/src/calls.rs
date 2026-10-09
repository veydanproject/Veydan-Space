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
//! Where the nodes come from: the core's server sets — the private nodes
//! this device was invited to (their credentials in the secret store of
//! the runtime) and the developer setting `call.nodes`, the `calls` of the
//! project's manifest for the region in use with their classes (put in
//! whenever the manifest or the region changes,
//! [`MessengerRuntime::seed_call_nodes`]), and the signed list of the
//! registry of volunteers ([`RegistrySource`]), cut by the trust level.
//! The list of the settings, adding by a link, the trust level:
//! `crate::call_nodes`.
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
//!
//! Group calls ([`GroupCallsDriver`]): the `GroupCallService` of the core
//! over the same engine, the same server sets and node client, with two
//! doors to the groups of the runtime: what the core asks of them
//! (`GroupAccess`: the members, the node pinned in the settings, a quiet
//! note sealed with the group key) and where the groups hand the
//! `call.*` notes they receive and say that their members changed
//! (`GroupCallSink`). Its effects are drained like the ones of a call
//! between two. One call at a time, of either kind: a group call is
//! refused while a call between two is under way, and the other way
//! round ([`MessengerRuntime::group_call_start`],
//! [`MessengerRuntime::call_start`]); a call that rings in while I sit in
//! a room says so (`busy_with_group` of `call.incoming`), and its
//! `accept` is refused. The events: `group_call.state`,
//! `group_call.started`, `group_call.ended`, `group_call.level`. The
//! camera of a phone pushes its frames into a room through
//! [`MessengerRuntime::group_call_push_video_frame`], as into a call
//! between two.

use crate::MessengerRuntime;
use messenger_calls::engine::{Media, PairKind, RelayPolicy as CorePolicy, VideoInput as CoreVideoInput, VideoTrack as CoreTrack};
use messenger_calls::servers::{parse_own_nodes, CallNode, NodeClass, NodeRef};
use messenger_calls::{
    AnnouncedCall as CoreAnnounced, CallDmHandler, CallService, CallView as CoreView, GroupAccess, GroupCallService,
    GroupCallView as CoreGroupView, GroupPhase as CoreGroupPhase, HttpRooms, NodeClient, Phase, RoomApi, ServerSets,
    ListFetch, Registry, SettingsServerSets, VideoQuality as CoreQuality, KEY_CALL_NODES,
};
use messenger_core::traits::UiEvent;
use messenger_core::{Effect, Envelope, MessengerError, Outbound, PubKey, Result, SecretStore};
use messenger_transport::manifest::CallNodeClass;
use messenger_dm::DmService;
use messenger_groups::{GroupCallSink, GroupService};
use messenger_ingress::Outbox;
use messenger_store::{settings, Store};
use serde::{Deserialize, Serialize};
use crate::relays::ServersMode;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, oneshot};
use ts_rs::TS;

pub use messenger_calls::engine::{MediaEngine, PixelFormat, PushedFrame, VideoFrame};
pub use messenger_calls::{
    UI_EVENT_CALL_ENDED, UI_EVENT_CALL_INCOMING, UI_EVENT_CALL_LEVEL, UI_EVENT_CALL_STATE, UI_EVENT_CALL_STATS,
    UI_EVENT_GROUP_CALL_ENDED, UI_EVENT_GROUP_CALL_LEVEL, UI_EVENT_GROUP_CALL_STARTED, UI_EVENT_GROUP_CALL_STATE,
};

/// What this client says of itself to a call node.
fn client_name() -> String {
    format!("veydan-messenger/{}", messenger_core::VERSION)
}

/// What the calls of a runtime are made with: the media engine, the
/// client of the control channel of the nodes and the client of their
/// rooms. The build's own by default ([`CallBackends::of_build`]); a
/// host or a test gives others (the CLI its engine on the pushed path,
/// the tests the fakes of the testkit: `FakeEngine`, `FakeNode`).
pub struct CallBackends {
    /// `None`: this build has no engine, and the screen sees `available: false`.
    pub engine: Option<Arc<dyn MediaEngine>>,
    pub nodes: NodeClient,
    pub rooms: Arc<dyn RoomApi>,
    /// The registry of volunteers' nodes, the last set of servers.
    pub registry: RegistrySource,
}

/// Where the registry of volunteers' call nodes is asked
/// (`messenger_calls::Registry`).
pub enum RegistrySource {
    /// The registries built into the client (`trust::REGISTRIES`), over
    /// HTTPS with the roots of the web, through a bridge when bridges
    /// are on ([`registry_fetch`]); asked only with the project's servers
    /// and not in the silent mode ([`gated_registry_fetch`]).
    Build,
    /// A registry the host makes over the store: the fake of the tests.
    Given(Box<dyn FnOnce(Store) -> Registry + Send>),
    /// No registry: the sets end with the manifest (the tests that touch
    /// no network).
    Off,
}

impl CallBackends {
    /// The engine of this build and the HTTP clients of the nodes.
    pub fn of_build() -> Self {
        Self::with_engine(default_engine())
    }

    /// `engine` with the HTTP clients of the nodes and of the registry.
    pub fn with_engine(engine: Option<Arc<dyn MediaEngine>>) -> Self {
        Self {
            engine,
            nodes: NodeClient::new(&client_name()),
            rooms: Arc::new(HttpRooms::new(&client_name())),
            registry: RegistrySource::Build,
        }
    }
}

/// The `GET` of the registry's list on the HTTP client of the messenger
/// (`messenger_http`: through a bridge when bridges are on, as the list
/// of bridges). Every ask is a line in the log: what the device told the
/// network is to be seen there.
pub fn registry_fetch() -> ListFetch {
    Arc::new(|url: String| {
        Box::pin(async move {
            eprintln!("messenger calls: asking the registry for call nodes: {url}");
            let client = messenger_http::client(Duration::from_secs(6), Duration::from_secs(12))?;
            let transport = |e: reqwest::Error| MessengerError::Transport(e.to_string());
            let response = client.get(&url).send().await.map_err(transport)?;
            if !response.status().is_success() {
                return Err(MessengerError::Transport(format!("the registry answered {}", response.status())));
            }
            response.text().await.map_err(transport)
        })
    })
}

/// Whether the registry of volunteers may be asked now: the rule of
/// everything else of the project's infrastructure (the manifest, the
/// list of bridges), only with the project's servers chosen, and never
/// in the silent mode. Before the onboarding, with own servers or in the
/// silent mode the device tells the registry nothing, whatever the trust
/// level of calls.
pub(crate) async fn registry_may_ask(store: &Store) -> bool {
    let mode = settings::get(store, crate::relays::KEY_MODE).await.ok().flatten();
    let veydan = mode.as_deref().and_then(ServersMode::parse) == Some(ServersMode::Veydan);
    let silent = settings::get_bool(store, crate::relays::KEY_SILENT, false).await.unwrap_or(true);
    veydan && !silent
}

/// `fetch` behind [`registry_may_ask`]: an ask it refuses touches no
/// network and fails (the core's own asks, `Registry::nodes` of every
/// call, go through it too). The watch of the app asks at once when the
/// gate opens, not after the pause that follows a failure
/// (`MessengerRuntime::call_registry_tick`).
pub fn gated_registry_fetch(store: Store, fetch: ListFetch) -> ListFetch {
    Arc::new(move |url: String| {
        let (store, fetch) = (store.clone(), fetch.clone());
        Box::pin(async move {
            if !registry_may_ask(&store).await {
                return Err(MessengerError::Transport("the registry is not asked: the project's servers are not in use, or the silent mode is on".into()));
            }
            fetch(url).await
        })
    })
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
    /// The client of the nodes the service uses: what it knows of them
    /// (`known`), the probes and the invitations of the settings.
    pub(crate) nodes: NodeClient,
    /// The nodes of the manifest in use, with their classes, as put into
    /// the sets (`seed_call_nodes`): what the list of the settings names.
    pub(crate) manifest: std::sync::RwLock<Vec<(NodeRef, NodeClass)>>,
    /// The nodes the last probe of the settings did not reach, by id, and
    /// why (`call_nodes_list` with `probe`).
    pub(crate) unreachable: std::sync::Mutex<std::collections::HashMap<String, String>>,
    /// The last tick of the registry found it paused ([`registry_may_ask`]
    /// said no; so it is before the first tick): the next tick that may
    /// ask, asks at once if the list is due, whatever a refused ask left
    /// behind as a failure.
    pub(crate) registry_paused: AtomicBool,
    /// A media engine is here (of the build, or given by the host): a
    /// call can be made.
    available: bool,
    /// Drains the effects of the service; aborted at shutdown.
    effects: tokio::task::JoinHandle<()>,
    /// Asks the drain to answer once everything queued before is done
    /// (`flush`).
    flush: mpsc::UnboundedSender<oneshot::Sender<()>>,
    /// The group calls, once they are made: a call that rings in while I
    /// sit in a room is told so (`busy_with_group` of `call.incoming`).
    group_calls: Arc<std::sync::OnceLock<GroupCallService>>,
}

/// The key of `call.incoming` that says I sit in the room of a group
/// call as it rings: its `accept` is refused (one call at a time), so the
/// screen shows "busy" instead of "answer", and a phone rings nothing.
pub const INCOMING_BUSY_WITH_GROUP: &str = "busy_with_group";

/// Drains the effects of a service of calls for as long as the runtime
/// lives: wraps and scoped events go to the outbox with a kick, events
/// to the UI channel. The sender answers a flush once everything queued
/// before it is done. With `busy` (the drain of the calls between two),
/// `call.incoming` carries whether a group call is under way.
fn drain_effects(
    mut rx: mpsc::UnboundedReceiver<Effect>,
    outbox: Outbox,
    ui: broadcast::Sender<UiEvent>,
    busy: Option<Arc<std::sync::OnceLock<GroupCallService>>>,
) -> (tokio::task::JoinHandle<()>, mpsc::UnboundedSender<oneshot::Sender<()>>) {
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
                        Effect::Emit(mut ev) => {
                            if let Some(busy) = busy.as_ref().filter(|_| ev.name == UI_EVENT_CALL_INCOMING) {
                                let in_room = match busy.get() {
                                    Some(group) => group.current().await.is_some(),
                                    None => false,
                                };
                                if let Some(payload) = ev.payload.as_object_mut() {
                                    payload.insert(INCOMING_BUSY_WITH_GROUP.into(), serde_json::Value::Bool(in_room));
                                }
                            }
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
    (effects, flush)
}

impl CallsDriver {
    /// With `engine` `None` every call fails at its start, and the
    /// state says `available: false`.
    ///
    /// The sets of servers keep the credentials of this device on private
    /// nodes in `secrets` and end with the registry `registry` names.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: Store,
        dm: DmService,
        engine: Option<Arc<dyn MediaEngine>>,
        nodes: NodeClient,
        outbox: Outbox,
        ui: broadcast::Sender<UiEvent>,
        secrets: Arc<dyn SecretStore>,
        registry: RegistrySource,
    ) -> Self {
        let servers = Arc::new(SettingsServerSets::new(store.clone()));
        servers.set_secrets(secrets);
        match registry {
            RegistrySource::Build => {
                let fetch = gated_registry_fetch(store.clone(), registry_fetch());
                servers.set_registry(Arc::new(Registry::new(store.clone(), fetch)))
            }
            RegistrySource::Given(make) => servers.set_registry(Arc::new(make(store.clone()))),
            RegistrySource::Off => {}
        }
        let available = engine.is_some();
        let (service, rx) = CallService::new(
            store,
            dm,
            engine.unwrap_or_else(|| Arc::new(NoEngine)),
            servers.clone(),
            nodes.clone(),
            Arc::new(messenger_core::traits::SystemClock),
        );
        let group_calls = Arc::new(std::sync::OnceLock::new());
        let (effects, flush) = drain_effects(rx, outbox, ui, Some(group_calls.clone()));
        Self {
            service,
            servers,
            nodes,
            manifest: std::sync::RwLock::new(Vec::new()),
            unreachable: std::sync::Mutex::new(std::collections::HashMap::new()),
            registry_paused: AtomicBool::new(true),
            available,
            effects,
            flush,
            group_calls,
        }
    }

    /// The sets of servers the calls take their nodes from: the group
    /// calls share them.
    pub fn servers(&self) -> Arc<SettingsServerSets> {
        self.servers.clone()
    }

    /// The group calls, made after this driver: from now on a call that
    /// rings in while a room is under way says `busy_with_group`.
    pub fn link_group_calls(&self, group_calls: GroupCallService) {
        let _ = self.group_calls.set(group_calls);
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

// ─── Group calls ─────────────────────────────────────────────────────────────

/// The groups as the core of group calls asks for them, over the group
/// service: who is in a group, which node its settings pin, a quiet note
/// sealed with its key. Everything is of the running session: without
/// keys there are no members and no notes.
struct GroupsDoor {
    groups: GroupService,
}

impl GroupsDoor {
    fn keys(&self) -> Result<nostr::key::Keys> {
        self.groups.signer().ok_or(MessengerError::NotLoggedIn)
    }
}

#[async_trait::async_trait]
impl GroupAccess for GroupsDoor {
    async fn members(&self, group_id: &str) -> Result<Vec<PubKey>> {
        let keys = self.keys()?;
        let me = PubKey::parse(&keys.public_key().to_hex()).expect("a key is hex");
        self.groups.members_of(group_id, &me).await
    }

    /// The node of the settings (`EditSettings::call_node`), class
    /// `group`, with its key. A reference the settings hold but this
    /// version cannot read is no node.
    async fn pinned_node(&self, group_id: &str) -> Result<Option<CallNode>> {
        let Some((reference, key)) = self.groups.call_node_of(group_id).await? else { return Ok(None) };
        let Ok(node) = reference.parse::<NodeRef>() else {
            eprintln!("messenger calls: the node pinned to the group {group_id} is not a node reference: {reference}");
            return Ok(None);
        };
        Ok(Some(CallNode { node, class: NodeClass::Group, access_key: key }))
    }

    async fn seal_note(&self, group_id: &str, envelope: &Envelope) -> Result<Outbound> {
        let keys = self.keys()?;
        self.groups.prepare_call_note(&keys, group_id, envelope).await
    }
}

/// Where the groups hand the `call.*` notes of their members: to the
/// core of group calls. A note read from the history (`historical`) goes
/// too: a call that is still on when the app starts is announced from
/// its notes, and one that is over is kept on record; the core drops
/// what has expired.
struct GroupNotes {
    service: GroupCallService,
}

#[async_trait::async_trait]
impl GroupCallSink for GroupNotes {
    async fn on_group_call(&self, group_id: &str, author: &PubKey, envelope: &Envelope, created_at: i64, _historical: bool) {
        if let Err(e) = self.service.on_group_note(group_id, author, envelope, created_at).await {
            eprintln!("messenger calls: a note of the group {group_id} was not taken: {e}");
        }
    }

    /// Who is in the group changed: the core looks at its room again (a
    /// member removed is nobody there from now on, the keys turn; out of
    /// the room myself when I am the one gone).
    async fn on_members_changed(&self, group_id: &str) {
        self.service.on_members_changed(group_id).await;
    }
}

/// The group call service and what serves it, owned by the runtime.
pub struct GroupCallsDriver {
    pub service: GroupCallService,
    effects: tokio::task::JoinHandle<()>,
    flush: mpsc::UnboundedSender<oneshot::Sender<()>>,
}

impl GroupCallsDriver {
    /// Over the groups of `groups` (whose `call.*` notes come here from
    /// now on), the engine, the server sets `servers` the calls between
    /// two use too, and the clients of the nodes.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: Store,
        dm: DmService,
        groups: GroupService,
        engine: Option<Arc<dyn MediaEngine>>,
        servers: Arc<SettingsServerSets>,
        nodes: NodeClient,
        rooms: Arc<dyn RoomApi>,
        outbox: Outbox,
        ui: broadcast::Sender<UiEvent>,
    ) -> Self {
        let (service, rx) = GroupCallService::new(
            store,
            dm,
            Arc::new(GroupsDoor { groups: groups.clone() }),
            engine.unwrap_or_else(|| Arc::new(NoEngine)),
            servers,
            nodes,
            rooms,
            Arc::new(messenger_core::traits::SystemClock),
        );
        groups.set_call_sink(Some(Arc::new(GroupNotes { service: service.clone() })));
        let (effects, flush) = drain_effects(rx, outbox, ui, None);
        Self { service, effects, flush }
    }

    /// Resolves once every effect the service gave before this call is
    /// done (its notes are in the outbox, its events on the UI channel);
    /// at once when the drain is stopped.
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

/// The payload of `call.incoming`: the call that rings, and whether I sit
/// in the room of a group call as it does. `busy_with_group`: its
/// `accept` is refused here (one call at a time); the screen shows
/// "busy" instead of "answer", the ringing goes on for my other devices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct CallIncoming {
    pub call: CallView,
    #[serde(default)]
    pub busy_with_group: bool,
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
    /// Whose it is: `own` (invited, or the setting), `project` (the
    /// manifest), `volunteer` (the registry, or so named by the manifest).
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

/// Where I am with the room of a group call: making it on the node,
/// joining it (my offer is with the node, ICE on its way), in it, the
/// way to the node lost (the engine tries on), or out of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GroupCallPhase {
    Starting,
    Joining,
    InRoom,
    Reconnecting,
    Left,
}

/// One seat of the room of a group call, as the screen shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct GroupParticipant {
    /// The seat: the node's participant id.
    pub id: u32,
    /// Who sits there, hex, once its word of identity was checked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub npub: Option<String>,
    /// The word of identity checked: a member, signed by its key, on
    /// this seat. Only a verified seat is shown as a person and heard.
    pub verified: bool,
    pub speaking: bool,
    /// The seat sends sound.
    pub audio: bool,
    /// The m-line of the seat's sound when it sends one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub audio_mid: Option<String>,
    /// The m-line of the seat's video when it sends one: what
    /// `messenger_group_call_video_subscribe` takes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub video_mid: Option<String>,
    pub me: bool,
    /// The seat's camera is on, by its own word of state (mine: my own
    /// state). Absent: not known — the seat is not verified, its word has
    /// not come yet, or its client is of before the word (5.1.11 and
    /// older); the screen judges such a seat by its frames, as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub camera: Option<bool>,
    /// The seat's microphone is on (not muted), the same way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub mic: Option<bool>,
    /// The seat shares its screen (its video is the screen, not the
    /// camera), the same way. The seat's video is on when `camera` or
    /// `screen` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub screen: Option<bool>,
}

/// The room of a group call I am in, as the screen shows it: the `call`
/// of `group_call.state` and the answer of the group call commands.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct GroupCallView {
    pub call_id: String,
    pub group_id: String,
    pub chat_id: String,
    pub phase: GroupCallPhase,
    pub media: CallMedia,
    pub muted: bool,
    pub video_local: bool,
    /// The camera in use (or the one for the next time), by the id the
    /// engine lists; absent for its default. On a phone `front` or `back`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub camera: Option<String>,
    /// Who made the room, hex.
    pub started_by: String,
    /// When the room was made, unix seconds.
    #[ts(type = "number")]
    pub started_at: i64,
    /// When I got into the room.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub joined_at: Option<i64>,
    /// The node I am connected to, `address:port#id`: the node of the
    /// room, or my own nearest node when I sit in the room through it
    /// (the cascade, services/call/spec/cascade.md).
    pub node: String,
    /// The node the room is on (its home), `address:port#id`; equal to
    /// `node` when I sit there directly. When the home dies the room
    /// moves (`call.move`): both change, and the phase goes
    /// `reconnecting` → `in_room` again under the same `call_id`.
    #[serde(default)]
    pub home: String,
    /// My seat, once the node gave it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub participant: Option<u32>,
    /// The epoch of the keys I send with.
    pub epoch: u32,
    /// Every seat of the room, mine included, by seat.
    pub participants: Vec<GroupParticipant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub limits: Option<CallLimits>,
    /// The most the room takes from one participant, kbit/s (0: no limit).
    #[serde(default)]
    pub kbps_per_participant: u32,
    /// Seats the room has at most (0: the node did not say).
    #[serde(default)]
    pub max_participants: u32,
}

/// A call announced in a group, in its room or not: the `call` of
/// `group_call.started` and `group_call.ended`, what the banner of the
/// chat shows ("a call is on — join").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct GroupCallAnnounced {
    pub call_id: String,
    pub group_id: String,
    pub chat_id: String,
    pub media: CallMedia,
    /// Who made the room, hex.
    pub started_by: String,
    #[ts(type = "number")]
    pub started_at: i64,
    /// The members in the room by their own word, hex.
    pub participants: Vec<String>,
    /// I am in this room.
    pub joined: bool,
}

/// The payload of `group_call.ended`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct GroupCallEnded {
    pub call: GroupCallAnnounced,
    pub outcome: CallOutcome,
    /// From the start of the room to the end, seconds; `null` when unknown.
    #[ts(type = "number | null")]
    pub duration_secs: Option<i64>,
}

/// The payload of `group_call.level`: how loud one seat is, 0 to 1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct GroupCallLevel {
    pub call_id: String,
    pub participant: u32,
    pub level: f32,
}

/// Group calls as the screen sees them now: the room I am in, and the
/// call announced in the group asked about.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct GroupCallState {
    pub call: Option<GroupCallView>,
    pub announced: Option<GroupCallAnnounced>,
}

impl From<Media> for CallMedia {
    fn from(m: Media) -> Self {
        match m {
            Media::Audio => Self::Audio,
            Media::Video => Self::Video,
        }
    }
}

impl From<CoreGroupView> for GroupCallView {
    fn from(v: CoreGroupView) -> Self {
        Self {
            call_id: v.call_id,
            group_id: v.group_id,
            chat_id: v.chat_id,
            phase: match v.phase {
                CoreGroupPhase::Starting => GroupCallPhase::Starting,
                CoreGroupPhase::Joining => GroupCallPhase::Joining,
                CoreGroupPhase::InRoom => GroupCallPhase::InRoom,
                CoreGroupPhase::Reconnecting => GroupCallPhase::Reconnecting,
                CoreGroupPhase::Left => GroupCallPhase::Left,
            },
            media: v.media.into(),
            muted: v.muted,
            video_local: v.video_local,
            camera: v.camera,
            started_by: v.started_by,
            started_at: v.started_at,
            joined_at: v.joined_at,
            node: v.node,
            home: v.home,
            participant: v.participant,
            epoch: v.epoch,
            participants: v
                .participants
                .into_iter()
                .map(|p| GroupParticipant {
                    id: p.id,
                    npub: p.npub,
                    verified: p.verified,
                    speaking: p.speaking,
                    audio: p.audio,
                    audio_mid: p.audio_mid,
                    video_mid: p.video_mid,
                    me: p.me,
                    camera: p.camera,
                    mic: p.mic,
                    screen: p.screen,
                })
                .collect(),
            limits: v.limits.map(|l| CallLimits {
                turn_lifetime_secs: l.turn_lifetime_secs,
                turn_kbps_per_allocation: l.turn_kbps_per_allocation,
                credentials_ttl_secs: l.credentials_ttl_secs,
            }),
            kbps_per_participant: v.kbps_per_participant,
            max_participants: v.max_participants,
        }
    }
}

impl From<CoreAnnounced> for GroupCallAnnounced {
    fn from(a: CoreAnnounced) -> Self {
        Self {
            call_id: a.call_id,
            group_id: a.group_id,
            chat_id: a.chat_id,
            media: a.media.into(),
            started_by: a.started_by,
            started_at: a.started_at,
            participants: a.participants,
            joined: a.joined,
        }
    }
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
    /// mutual chat with, and while a call is under way (of either kind).
    pub async fn call_start(&self, peer: &str, media: CallMedia) -> Result<CallView> {
        let pk = messenger_contacts::book::parse_key(peer)?;
        self.no_group_call().await?;
        self.calls.service.start(&pk, media.into()).await.map(Into::into)
    }

    /// Take the ringing call. Refused while I am in the room of a group
    /// call: one call at a time, and the ringing one keeps ringing for my
    /// other devices.
    pub async fn call_accept(&self, call_id: &str) -> Result<CallView> {
        self.no_group_call().await?;
        self.calls.service.accept(call_id).await.map(Into::into)
    }

    /// A call between two is refused while a group call is under way.
    async fn no_group_call(&self) -> Result<()> {
        match self.group_calls.service.current().await {
            Some(_) => Err(MessengerError::Invalid("a group call is under way".into())),
            None => Ok(()),
        }
    }

    /// A group call is refused while a call between two is under way
    /// (ringing either way, or talking).
    async fn no_dm_call(&self) -> Result<()> {
        match self.calls.service.current().await {
            Some(_) => Err(MessengerError::Invalid("a call is under way".into())),
            None => Ok(()),
        }
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
        let nodes: Vec<(NodeRef, NodeClass)> = manifest
            .calls_for_region(&region)
            .iter()
            .filter_map(|c| {
                let class = match c.class {
                    CallNodeClass::Project => NodeClass::Project,
                    CallNodeClass::Volunteer => NodeClass::Volunteer,
                    // `cloud` comes through a door of its own when there
                    // is one; `own` and unknown classes are no node of a
                    // manifest (`ManifestCall::usable` left them out).
                    CallNodeClass::Own | CallNodeClass::Cloud | CallNodeClass::Unknown => return None,
                };
                Some((c.node.parse().ok()?, class))
            })
            .collect();
        *self.calls.manifest.write().unwrap() = nodes.clone();
        self.calls.servers.set_manifest_classed(nodes);
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

    // ─── Group calls ─────────────────────────────────────────────────────

    pub fn group_calls(&self) -> &GroupCallService {
        &self.group_calls.service
    }

    /// Start a call in the group `group_id`: a room on a node (the one
    /// pinned to the group, else the nearest with an SFU), me in it, the
    /// group told. Refused while a call of either kind is under way, and
    /// while a call is on in this group already (join it).
    pub async fn group_call_start(&self, group_id: &str, media: CallMedia) -> Result<GroupCallView> {
        self.no_dm_call().await?;
        self.group_calls.service.start(group_id, media.into()).await.map(Into::into)
    }

    /// Join the call announced in the group `group_id`. Refused while a
    /// call of either kind is under way.
    pub async fn group_call_join(&self, group_id: &str) -> Result<GroupCallView> {
        self.no_dm_call().await?;
        self.group_calls.service.join(group_id).await.map(Into::into)
    }

    /// Leave the room; the last one out ends the call for the group.
    pub async fn group_call_leave(&self) -> Result<()> {
        self.group_calls.service.leave().await
    }

    pub async fn group_call_set_mute(&self, muted: bool) -> Result<GroupCallView> {
        self.group_calls.service.set_mute(muted).await.map(Into::into)
    }

    /// My video in the room: a camera, a screen, or off.
    pub async fn group_call_set_video(&self, input: VideoInput) -> Result<GroupCallView> {
        self.group_calls.service.set_video(input.into()).await.map(Into::into)
    }

    /// The next camera of the engine's list (or the one `camera` names)
    /// in the room: switched at once when my camera is on, kept for when
    /// it goes on otherwise. On a phone the list is `front`, `back`.
    pub async fn group_call_switch_camera(&self, camera: Option<String>) -> Result<GroupCallView> {
        self.group_calls.service.switch_camera(camera).await.map(Into::into)
    }

    /// A frame the platform captured (the camera of a phone, through the
    /// plugin: NV21 with its rotation), as my video in the room, the way
    /// [`Self::call_push_video_frame`] takes one for a call between two.
    /// Refused without a room. The engine converts and sends it when my
    /// video is on (`video_local`), and drops it otherwise.
    pub async fn group_call_push_video_frame(&self, frame: PushedFrame) -> Result<()> {
        self.group_calls.service.push_video_frame(frame).await
    }

    /// The layer of a seat's video I want (`q`, `h` or `f`: a quarter, a
    /// half or the full size), for the size of its tile. An error with a
    /// node without simulcast.
    pub async fn group_call_set_layer(&self, participant: u32, rid: &str) -> Result<()> {
        self.group_calls.service.set_layer(participant, rid).await
    }

    /// The frames of the video of the seat whose m-line is `mid`
    /// (`GroupParticipant::video_mid`), as the engine hands them; `None`
    /// without a room or before its media is there.
    pub async fn group_call_video_frames(&self, mid: &str) -> Option<broadcast::Receiver<Arc<VideoFrame>>> {
        self.group_calls.service.video_frames(mid).await
    }

    /// The room I am in, and the call announced in `group_id` when one is.
    pub async fn group_call_state(&self, group_id: Option<&str>) -> GroupCallState {
        let announced = match group_id {
            Some(g) => self.group_calls.service.announced(g).await.map(Into::into),
            None => None,
        };
        GroupCallState { call: self.group_calls.service.current().await.map(Into::into), announced }
    }

    /// The session is about to stop under a group call: the room is
    /// left, so the others hear of it from the node and the group now.
    /// `true` says a note of the leave is in the outbox.
    pub(crate) async fn leave_group_call_before_leaving(&self) -> bool {
        if self.group_calls.service.current().await.is_none() {
            return false;
        }
        if let Err(e) = self.group_calls.service.leave().await {
            eprintln!("messenger calls: the group call was not left before leaving: {e}");
            return false;
        }
        self.group_calls.flush().await;
        true
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
        // No registry: what is listed is what the test put in.
        let backends = CallBackends { registry: RegistrySource::Off, ..CallBackends::with_engine(Some(Arc::new(FakeEngine::new()))) };
        let rt = MessengerRuntime::start_with_backends(cfg, secrets, backends).await.unwrap();
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
        let backends = CallBackends { registry: RegistrySource::Off, ..CallBackends::with_engine(Some(Arc::new(engine.clone()))) };
        let rt = MessengerRuntime::start_with_backends(cfg, secrets, backends).await.unwrap();
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
        let backends = CallBackends { registry: RegistrySource::Off, ..CallBackends::with_engine(Some(Arc::new(engine.clone()))) };
        let rt = MessengerRuntime::start_with_backends(cfg, secrets.clone(), backends).await.unwrap();
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

    /// The views of a group call are the core's, field for field, as the
    /// views of a call between two are.
    #[test]
    fn the_group_views_of_the_ui_are_the_views_of_the_core() {
        let core = CoreGroupView {
            call_id: "c1".into(),
            group_id: "g".repeat(64),
            chat_id: format!("group:{}", "g".repeat(64)),
            phase: CoreGroupPhase::InRoom,
            media: Media::Video,
            muted: true,
            video_local: true,
            camera: Some("back".into()),
            started_by: "ab".repeat(32),
            started_at: 100,
            joined_at: Some(103),
            node: NODE.into(),
            home: NODE.into(),
            participant: Some(2),
            epoch: 3,
            participants: vec![messenger_calls::ParticipantView {
                id: 1,
                npub: Some("cd".repeat(32)),
                verified: true,
                speaking: true,
                audio: true,
                audio_mid: Some("1".into()),
                video_mid: Some("2".into()),
                me: false,
                camera: Some(true),
                mic: Some(false),
                screen: None,
            }],
            limits: Some(messenger_calls::node_client::Limits { turn_lifetime_secs: 3600, turn_kbps_per_allocation: 2000, credentials_ttl_secs: 600 }),
            kbps_per_participant: 2500,
            max_participants: 12,
        };
        let json = serde_json::to_value(&core).unwrap();
        let view: GroupCallView = core.clone().into();
        assert_eq!(serde_json::to_value(&view).unwrap(), json);
        assert_eq!(serde_json::from_value::<GroupCallView>(json).unwrap(), view);
        let bare = CoreGroupView { joined_at: None, participant: None, limits: None, participants: vec![], camera: None, ..core };
        let json = serde_json::to_value(&bare).unwrap();
        assert_eq!(serde_json::to_value(GroupCallView::from(bare)).unwrap(), json);
        assert!(json.get("joined_at").is_none() && json.get("limits").is_none() && json.get("camera").is_none());
        let announced = CoreAnnounced {
            call_id: "c1".into(),
            group_id: "g".repeat(64),
            chat_id: format!("group:{}", "g".repeat(64)),
            media: Media::Audio,
            started_by: "ab".repeat(32),
            started_at: 100,
            participants: vec!["ab".repeat(32)],
            joined: true,
        };
        let json = serde_json::to_value(&announced).unwrap();
        assert_eq!(serde_json::to_value(GroupCallAnnounced::from(announced)).unwrap(), json);
        for (phase, core) in [
            (GroupCallPhase::Starting, CoreGroupPhase::Starting),
            (GroupCallPhase::InRoom, CoreGroupPhase::InRoom),
            (GroupCallPhase::Reconnecting, CoreGroupPhase::Reconnecting),
            (GroupCallPhase::Left, CoreGroupPhase::Left),
        ] {
            assert_eq!(serde_json::to_value(phase).unwrap(), serde_json::to_value(core).unwrap());
        }
    }

    /// A group call from the runtime on the fakes: the group I made, its
    /// room on the fake node through the fake engine, me in it (the
    /// notes of the group in the outbox, the events on the UI channel);
    /// a call between two is refused meanwhile, and a group call while
    /// one is under way; the node pinned to the group is the one asked
    /// for the room.
    #[tokio::test]
    async fn a_group_call_goes_through_the_fakes_and_one_call_at_a_time() {
        use messenger_testkit::FakeNode;
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let secrets = Arc::new(MemorySecretStore::unlocked());
        let engine = FakeEngine::new();
        let node = FakeNode::new(engine.clone());
        // The runtime keeps the system clock: the fake node's rooms expire
        // twelve hours from now, not from the clock of the core's tests.
        node.set_now(messenger_core::Clock::now(&messenger_core::traits::SystemClock).secs() as u64);
        let backends = CallBackends { engine: Some(Arc::new(engine.clone())), nodes: node.client(), rooms: Arc::new(node.clone()), registry: RegistrySource::Off };
        let rt = MessengerRuntime::start_with_backends(cfg, secrets, backends).await.unwrap();
        rt.relays().set_silent(true).await.unwrap();
        use_veydan_offline(&rt).await;
        rt.identity().create("pw").await.unwrap();
        rt.refresh_signer().await.unwrap();
        rt.dm().set_gate(false);
        // The fake node is my own: the sets of servers find its SFU first.
        rt.call_set_nodes(vec![CallNodeInput { reference: node.reference().to_string(), key: None }]).await.unwrap();
        let group = rt.group_create(messenger_groups::GroupKind::Private, "Team", "", true).await.unwrap();
        assert!(rt.group_call_state(Some(&group.id)).await.announced.is_none());
        assert!(matches!(rt.group_call_join(&group.id).await, Err(MessengerError::Invalid(_))), "nothing to join yet");
        let before = rt.outbox().pending().await.unwrap();
        let mut events = rt.ui_events();

        let view = rt.group_call_start(&group.id, CallMedia::Audio).await.unwrap();
        assert_eq!((view.group_id.as_str(), view.media, view.participant), (group.id.as_str(), CallMedia::Audio, Some(1)));
        assert_eq!(view.node, node.reference().to_string());
        assert_eq!(view.home, view.node, "the room is on the node I made it on: no cascade");
        assert_eq!(view.participants.len(), 1, "me: {:?}", view.participants);
        assert!(view.participants[0].me && view.participants[0].verified);
        // The fake node's answer connects at once; the core hears of it
        // on its pump.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let st = rt.group_call_state(Some(&group.id)).await;
        assert_eq!(st.call.as_ref().map(|c| c.phase), Some(GroupCallPhase::InRoom), "{st:?}");
        let announced = st.announced.expect("the call is announced in the group");
        assert!(announced.joined && announced.participants == vec![view.started_by.clone()]);
        assert_eq!(node.rooms().len(), 1);
        assert_eq!(node.seats(&node.rooms()[0]).len(), 1);
        let session = engine.sessions().pop().expect("the room session");
        assert!(session.record().room.is_some(), "a session of a room");
        assert_eq!(session.record().sender_keys.len(), 1, "my key of epoch 1");
        assert!(rt.outbox().pending().await.unwrap() >= before + 2, "call.start and call.join wait in the outbox");
        // One call at a time: a call between two is refused meanwhile.
        let peer = nostr::key::Keys::generate().public_key().to_hex();
        let refused = rt.call_start(&peer, CallMedia::Audio).await;
        assert!(matches!(&refused, Err(MessengerError::Invalid(e)) if e.contains("group call")), "{refused:?}");
        assert!(rt.call_state().await.unwrap().call.is_none());
        // The seat's own controls.
        assert!(rt.group_call_set_mute(true).await.unwrap().muted);
        assert!(matches!(rt.group_call_set_layer(2, "q").await, Err(MessengerError::Invalid(_))), "the fake node has no simulcast");
        assert!(rt.group_call_video_frames("v9").await.is_some(), "the frames of a seat's video can be asked for");

        rt.group_call_leave().await.unwrap();
        assert!(rt.group_call_state(Some(&group.id)).await.call.is_none());
        assert!(session.record().closed);
        assert!(node.seats(&node.rooms()[0]).is_empty(), "my seat is gone from the node");
        let mut seen = Vec::new();
        while let Ok(ev) = events.try_recv() {
            seen.push(ev.name);
        }
        for name in [UI_EVENT_GROUP_CALL_STATE, UI_EVENT_GROUP_CALL_STARTED, UI_EVENT_GROUP_CALL_ENDED] {
            assert!(seen.iter().any(|n| n == name), "{name} among {seen:?}");
        }
        assert!(rt.group_call_state(Some(&group.id)).await.announced.is_none(), "the last one out ended the call");

        // The other way round: a group call is refused while a call
        // between two is under way.
        let call = rt.call_start(&peer, CallMedia::Audio).await.unwrap();
        let refused = rt.group_call_start(&group.id, CallMedia::Audio).await;
        assert!(matches!(&refused, Err(MessengerError::Invalid(e)) if e.contains("a call is under way")), "{refused:?}");
        assert!(rt.group_call_state(None).await.call.is_none());
        rt.call_end(&call.call_id).await.unwrap();

        // The node pinned to the group in its settings is the one asked
        // for the room, whatever the sets of servers say.
        let pinned = format!("203.0.113.9:8443#{}", "fb".repeat(32));
        let op = messenger_groups::OpBody::EditSettings {
            name: None,
            about: None,
            picture: None,
            history_for_new: None,
            call_node: Some(pinned.clone()),
            call_node_key: Some("k".into()),
        };
        rt.group_act(&group.id, op).await.unwrap();
        assert_eq!(rt.groups().call_node_of(&group.id).await.unwrap(), Some((pinned.clone(), Some("k".into()))));
        let view = rt.group_call_start(&group.id, CallMedia::Video).await.unwrap();
        assert_eq!(view.node, pinned);
        assert_eq!(node.created_on().last().map(String::as_str), Some(pinned.as_str()));
        rt.group_call_leave().await.unwrap();
        rt.shutdown().await;
    }

    /// Through the runtime: the camera of a phone pushes its frames into
    /// the room as into a call between two; a call between two that
    /// rings in while I sit in the room says `busy_with_group` and its
    /// answer is refused; and the groups tell the room of a change of
    /// their members (here: the group is disbanded under the call), so
    /// the room is left and the banner gone.
    #[tokio::test]
    async fn the_camera_the_busy_word_and_the_members_of_a_group_call_through_the_runtime() {
        use messenger_testkit::FakeNode;
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let secrets = Arc::new(MemorySecretStore::unlocked());
        let engine = FakeEngine::new();
        let node = FakeNode::new(engine.clone());
        node.set_now(messenger_core::Clock::now(&messenger_core::traits::SystemClock).secs() as u64);
        let backends = CallBackends { engine: Some(Arc::new(engine.clone())), nodes: node.client(), rooms: Arc::new(node.clone()), registry: RegistrySource::Off };
        let rt = MessengerRuntime::start_with_backends(cfg, secrets, backends).await.unwrap();
        rt.relays().set_silent(true).await.unwrap();
        use_veydan_offline(&rt).await;
        rt.identity().create("pw").await.unwrap();
        rt.refresh_signer().await.unwrap();
        rt.dm().set_gate(false);
        rt.call_set_nodes(vec![CallNodeInput { reference: node.reference().to_string(), key: None }]).await.unwrap();
        let group = rt.group_create(messenger_groups::GroupKind::Private, "Team", "", true).await.unwrap();
        let frame = || PushedFrame { format: PixelFormat::Nv21, width: 4, height: 2, rotation: 90, timestamp_us: 0, data: vec![0; 12] };
        assert!(matches!(rt.group_call_push_video_frame(frame()).await, Err(MessengerError::Invalid(_))), "no room, no frames");
        assert!(matches!(rt.group_call_switch_camera(None).await, Err(MessengerError::Invalid(_))));
        let mut events = rt.ui_events();

        // A video call: the camera is on from the start, the plugin's
        // frames go in and show as my own picture, the other camera on a
        // switch.
        let view = rt.group_call_start(&group.id, CallMedia::Video).await.unwrap();
        assert!(view.video_local && view.camera.is_none());
        let session = engine.sessions().pop().expect("the room session");
        let mut local = session.frames(VideoTrack::Local.into()).expect("the frames of my own video");
        rt.group_call_push_video_frame(frame()).await.unwrap();
        assert_eq!(session.record().pushed_frames, 1);
        let shown = local.try_recv().expect("the pushed frame shows as my own picture");
        assert_eq!((shown.width, shown.height, shown.rotation), (4, 2, 90));
        let switched = rt.group_call_switch_camera(None).await.unwrap();
        assert_eq!(switched.camera.as_deref(), Some("back"), "a phone's cameras when the engine lists none");
        assert!(switched.video_local);

        // A call between two rings in: told as busy, its answer refused,
        // the room untouched.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let caller = nostr::key::Keys::generate();
        let peer = messenger_core::PubKey::parse(&caller.public_key().to_hex()).unwrap();
        let me = rt.session_pubkey().await.unwrap();
        let invite = messenger_calls::signal::Signal::Invite {
            call_id: "ab".repeat(16),
            media: Media::Audio,
            sdp: "v=0".into(),
            ice: vec![],
            restart: false,
            live_restart: true,
        }
        .to_envelope();
        let now = messenger_core::Clock::now(&messenger_core::traits::SystemClock);
        let msg = messenger_core::DmInbound {
            envelope: messenger_core::inbound::Envelope {
                wire_id: messenger_core::EventId::parse(&"ef".repeat(32)).unwrap(),
                source: messenger_core::EventSource::Server { id: "test".into() },
                wire_created_at: now,
            },
            rumor_id: messenger_core::EventId::parse(&"cd".repeat(32)).unwrap(),
            sender: peer.clone(),
            recipients: vec![me.clone()],
            created_at: now,
            content: invite.encode(),
            reply_to: None,
            rumor_kind: 14,
        };
        let ctx = messenger_core::Context { my_pubkey: me, session_started_at: now, clock: Arc::new(messenger_core::traits::SystemClock) };
        rt.calls().on_dm(&msg, &invite, &ctx).await.unwrap();
        rt.calls.flush().await;
        let mut incoming = None;
        while let Ok(ev) = events.try_recv() {
            if ev.name == UI_EVENT_CALL_INCOMING {
                incoming = Some(ev.payload);
            }
        }
        let incoming = incoming.expect("call.incoming was emitted");
        assert_eq!(incoming["call"]["call_id"], serde_json::json!("ab".repeat(16)));
        assert_eq!(incoming[INCOMING_BUSY_WITH_GROUP], serde_json::json!(true), "{incoming}");
        let refused = rt.call_accept(&"ab".repeat(16)).await;
        assert!(matches!(&refused, Err(MessengerError::Invalid(e)) if e.contains("group call")), "{refused:?}");
        assert!(rt.group_call_state(None).await.call.is_some(), "the room goes on");
        rt.call_decline(&"ab".repeat(16)).await.unwrap();

        // The group is disbanded under the call: the groups tell the core
        // of calls, which is out of the room, the banner gone, the record
        // closed.
        rt.group_act(&group.id, messenger_groups::OpBody::Disband).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let st = rt.group_call_state(Some(&group.id)).await;
        assert!(st.call.is_none(), "out of the room: {st:?}");
        assert!(st.announced.is_none(), "no banner: {st:?}");
        assert!(session.record().closed);
        let row = messenger_store::calls::get(rt.store(), &view.call_id).await.unwrap().unwrap();
        assert_eq!(row.outcome.as_deref(), Some("ended"));
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
