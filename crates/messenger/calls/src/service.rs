// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The core of a call: one call at a time, its state, what the peer and
//! the engine tell, what to send and show. Everything it wants done
//! outside (a wrap published, an event shown) goes out as an `Effect` on
//! the outlet the host drains; nothing here touches a relay or a screen.
//!
//! ```text
//!   Idle ──start──▶ Outgoing ──answer──▶ Connecting ──connected──▶ Active ──end──▶ Ended
//!        ◀─invite── Incoming ──accept──▶ Connecting                  │
//!                     │ decline / busy / missed                  restart (network changed,
//!                     ▼                                          connection lost): Connecting
//!                   Ended
//! ```
//!
//! The rules (internal/messenger-wire.md §10):
//!
//! - An invitation rings for 45 s by the time inside its rumor; an older
//!   one is a missed call on record. Where it came from (a live
//!   subscription, a catch-up) and when this session began do not
//!   matter: a push wakes the phone after the invitation was made.
//! - Only a peer we both chose to talk with (`full_chat`) may call or be
//!   called; a stranger's invitation is dropped without a word.
//! - A call while I am on one is answered `call.busy`.
//! - Both called each other at once (glare): the smaller call id wins;
//!   its loser gives up its own call with `call.end {superseded}` (so
//!   that every device that heard of it forgets it) and takes the
//!   winner's as answered when it asked for the same media; a video call
//!   it did not ask for rings instead.
//! - The peer answered from two devices: the caller takes the first
//!   answer and tells the other device with `call.end` naming its answer.
//! - Timeouts: 45 s for an answer, 30 s for ICE to connect (also after a
//!   restart). One ICE restart per loss of connection: the engine's
//!   `Disconnected` (not `Failed`, which comes much later), confirmed by
//!   a short timer ([`LOSS_CONFIRM`]) in case the way comes back by
//!   itself; the network changing under the engine is a restart at once.
//!   Only the caller makes offers within a call; the called side asks
//!   with `call.restart`. The pieces of a restart leave without waiting
//!   for candidates (the first offer and answer wait [`GATHER_WAIT`]):
//!   the candidates trickle after them in `call.ice`. A restart whose
//!   pieces have not come within [`RESTART_SETTLE`] was lost on the
//!   relays: the call talks on when the way still works, or is
//!   restarted once more when it does not (the connect timer judges
//!   that one). A request of the called side that overtakes its first
//!   answer is kept and honoured once that answer is taken.
//! - A restart may run while the engine is still connected (the peer's
//!   request came late, a network changed but the old way still works):
//!   the engine then never says `Connected` again, so the call is
//!   `Active` as soon as the new description is in place, and
//!   `Reconnecting` only while the engine says the way is gone. Such a
//!   restart is made on one's own word (the network changed) only for a
//!   peer that said `live_restart` (5.1.3+): an older one would wait for
//!   a `Connected` that never comes and give up; with it the loss, when
//!   the old way goes, gets its restart as any.
//! - A device with `call.incoming_enabled` off ignores every invitation
//!   it is not itself calling about: it neither rings nor answers (not
//!   even `call.busy`, which would stop my other devices from ringing),
//!   and writes nothing down.
//! - The relays keep no order: what came of a call before its invitation
//!   did (an answer, an end) waits for the invitation and is written
//!   with it.
//!
//! Video (stage 6): every call is negotiated with a video m-line from the
//! first offer, whatever `media` says, so that a camera goes on and off
//! in the middle of a call without a new offer — the caller alone makes
//! offers, and the called side would otherwise have to ask for one. The
//! engine sends frames while my video is on and nothing while it is off;
//! `call.video {on}` tells the peer, so its screen shows a placeholder
//! instead of a frozen last frame. `media: video` in the invitation
//! means the caller's camera is on from the start and the called side's
//! goes on as it takes the call; a camera that fails to open leaves the
//! call as an audio one, and says so.
//!
//! The lock on the state is held across calls into the engine: the engine
//! never calls back (its events come on a channel), so that is safe, and
//! it keeps every transition whole.

use crate::call::{
    CallView, Direction, Outcome, Phase, ReconnectReason, VideoSize, UI_EVENT_CALL_INCOMING, UI_EVENT_CALL_LEVEL, UI_EVENT_CALL_STATE,
    UI_EVENT_CALL_STATS,
};
use crate::engine::{
    CameraInfo, ConnectionState, IceCandidate, Media, MediaEngine, PairKind, PushedFrame, RelayPolicy, ScreenInfo, SdpKind, Session,
    SessionEvent, VideoFrame, VideoInput, VideoSettings, VideoTrack,
};
use crate::feed::Feed;
use crate::node_client::NodeClient;
use crate::servers::{ServerSets, KEY_RELAY_POLICY};
use crate::signal::{self, new_call_id, Signal, INVITE_EXPIRATION_SECS, INVITE_TTL_SECS, NOTE_EXPIRATION_SECS};
use messenger_core::traits::UiEvent;
use messenger_core::{Clock, Context, DmInbound, Effect, Envelope, MessengerError, Outbound, PubKey, Result};
use messenger_dm::wrap::{wrap_expiring, wrap_note_as, Wake};
use messenger_dm::DmService;
use messenger_store::{calls as repo, settings, Store};
use nostr::key::Keys;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, Mutex};

/// The setting of the size of my video: `360p` (the default) or `720p`.
pub const KEY_VIDEO_QUALITY: &str = "call.video_quality";
/// The setting of whether this device takes calls at all: `true` (the
/// default) or `false`, when it ignores every invitation without a word,
/// leaving it to my other devices.
pub const KEY_INCOMING_ENABLED: &str = "call.incoming_enabled";

/// How big my video is sent. The numbers are what a camera is asked for
/// and the most the encoder may spend at that size; the engine scales
/// down by itself when the way is narrower.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoQuality {
    /// 640×360 at 30 frames a second, up to 800 kbit/s.
    #[default]
    #[serde(rename = "360p")]
    Sd,
    /// 1280×720 at 30 frames a second, up to 1800 kbit/s.
    #[serde(rename = "720p")]
    Hd,
}

impl VideoQuality {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sd => "360p",
            Self::Hd => "720p",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "360p" => Some(Self::Sd),
            "720p" => Some(Self::Hd),
            _ => None,
        }
    }

    /// Width, height, the largest bitrate in kbit/s.
    pub fn profile(self) -> (u32, u32, u32) {
        match self {
            Self::Sd => (640, 360, 800),
            Self::Hd => (1280, 720, 1800),
        }
    }
}

/// Frames a second asked of a camera.
pub const VIDEO_FPS: u32 = 30;

/// The cameras of a phone, as its plugin names them when the engine lists
/// none: switching goes between these two.
const PHONE_CAMERAS: [&str; 2] = ["front", "back"];

/// How long the first offer or answer of a call waits for candidates
/// before it leaves. Later ones follow in `call.ice`. The pieces of a
/// restart do not wait: every second counts then, and the candidates
/// trickle.
pub const GATHER_WAIT: Duration = Duration::from_millis(1500);
/// How long a lost connection (`Disconnected` from the engine) is given to
/// come back by itself before a restart is made. Short: the engine has
/// waited for the peer's answers for seconds already by the time it says
/// so, and a restart costs a round trip through the relays.
pub const LOSS_CONFIRM: Duration = Duration::from_secs(1);
/// Candidates found after the first piece left are sent in batches this
/// far apart.
pub const ICE_DEBOUNCE: Duration = Duration::from_millis(200);
/// A call nobody answered in this time is missed.
pub const RING_TIMEOUT: Duration = Duration::from_secs(INVITE_TTL_SECS as u64);
/// ICE that found no way in this time has failed.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a restart is given to have its pieces exchanged (the called
/// side's request, the caller's offer, the answer) before it is taken as
/// lost on the relays: a round trip through them takes a second or two.
/// With the way still there the call talks on and the next loss gets its
/// own restart; without it one more restart is made, and then the
/// connect timer judges.
pub const RESTART_SETTLE: Duration = Duration::from_secs(10);

/// Calls that ended lately, whose late signals are dropped unread.
const RECENT_KEPT: usize = 64;
/// Rumor ids of what I sent lately: a copy of my own coming back is not news.
const SENT_KEPT: usize = 256;
/// Calls whose end or answer came before their invitation, kept for it.
const EARLY_KEPT: usize = 64;

/// The outcome of a call that another device of mine took: not a word of
/// the record (that device keeps it), only of the screen. Also of a call
/// of the peer's that lost a glare while this device rang for it.
pub const ANSWERED_ELSEWHERE: &str = "answered_elsewhere";

/// The piece that waits for the candidates of the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FirstPiece {
    Invite { restart: bool },
    Answer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Timer {
    Gather,
    IceFlush,
    Ring,
    /// Armed at every attempt to connect; an older attempt's timer says
    /// nothing of a newer one.
    Connect { attempt: u32 },
    /// The way was lost this long ago ([`LOSS_CONFIRM`]); a restart is due
    /// unless it came back. `loss` counts the losses: an older loss's
    /// timer says nothing of a newer one.
    Lost { loss: u32 },
    /// The restart of this attempt had [`RESTART_SETTLE`] to settle; one
    /// still awaited then was lost on the relays.
    Settle { attempt: u32 },
}

struct Current {
    view: CallView,
    /// Timers and engine events of an older generation are not for this call.
    gen: u64,
    policy: RelayPolicy,
    session: Option<Box<dyn Session>>,
    /// The peer's offer, until I take the call.
    remote_offer: Option<String>,
    /// The peer's candidates that came before the engine had the peer's
    /// description to put them against.
    remote_ice: Vec<IceCandidate>,
    /// My offer or answer, waiting for candidates.
    first: Option<(FirstPiece, String)>,
    gathered: Vec<IceCandidate>,
    pending_ice: Vec<IceCandidate>,
    flush_scheduled: bool,
    /// The invitation left; an end has to be told.
    invite_sent: bool,
    /// Caller: the answer I took. Callee: the rumor id of my answer.
    taken_answer: Option<String>,
    my_answer: Option<String>,
    /// Caller: a restart offer left, the next answer is for it. Callee: a
    /// restart was asked for, the caller's offer is awaited.
    awaiting_restart: bool,
    /// One restart per loss of the connection: set when a restart is made
    /// for a loss, cleared when the engine connects (or a restart made
    /// while it was connected is complete).
    restart_tried: bool,
    /// The restart of this loss was made a second time, its first having
    /// settled nothing within [`RESTART_SETTLE`]; cleared when the engine
    /// connects. There is no third: the connect timer judges.
    retried: bool,
    /// The peer takes a restart while its way still works (it said
    /// `live_restart`, or counted its offers in `call.restart`). Without
    /// it no restart is made on my own word while my way is live.
    peer_live_restart: bool,
    /// Caller: the called side asked for a restart before its first answer
    /// came (its network changed as it answered): honoured with the answer.
    asked_early: bool,
    /// Caller: how many of my offers were answered. A request of the
    /// called side that saw fewer is older than that answer, and stale.
    answered_offers: u32,
    /// How many times this call set out to connect (see `Timer::Connect`).
    connect_attempt: u32,
    /// The engine's last word: the way is there. A restart made while it
    /// is (the peer asked late, the network changed but the old way still
    /// works) never gets a `Connected` of its own.
    connected: bool,
    /// How many times the way was lost (see `Timer::Lost`).
    loss: u32,
    /// Caller: offers made (the first, then every restart). Callee: offers
    /// taken. The called side says its count in `call.restart` (`seen`),
    /// and the caller drops a request older than its latest offer.
    offers: u32,
    /// Why the restart under way was made, for the screen when the way
    /// goes during it.
    restarting_for: Option<ReconnectReason>,
    /// What the peer believes of my video: the invitation's word at first
    /// (a video call has both cameras on), then every `call.video` sent.
    told_video: bool,
}

/// What came of a call before its invitation did: the relays keep no
/// order, and a catch-up gives the newest first. Kept until the
/// invitation makes the record, and written into it then.
#[derive(Clone, Debug, Default)]
struct Early {
    answered_at: Option<i64>,
    end: Option<(Outcome, i64)>,
    /// The peer's last word of its video (`call.video`), when it came
    /// before the invitation: a caller whose camera failed says so right
    /// after an invitation that still says `video`, and the note may
    /// overtake it.
    video: Option<bool>,
}

impl Early {
    /// Whether the call is over, or taken on another device of mine:
    /// the invitation then makes a record and rings for nothing.
    fn settled(&self) -> bool {
        self.answered_at.is_some() || self.end.is_some()
    }

    fn answered(&mut self, at: i64) {
        self.answered_at.get_or_insert(at);
        // An end that came first read the call as missed; it was taken.
        if let Some((outcome, _)) = self.end.as_mut() {
            if *outcome == Outcome::Missed {
                *outcome = Outcome::Ended;
            }
        }
    }

    /// The first end holds, as it does on the record.
    fn ended(&mut self, outcome: Outcome, at: i64) {
        self.end.get_or_insert((outcome, at));
    }
}

#[derive(Default)]
struct State {
    current: Option<Current>,
    recent: VecDeque<String>,
    sent: VecDeque<String>,
    early: VecDeque<(String, Early)>,
    gen: u64,
}

impl State {
    fn next_gen(&mut self) -> u64 {
        self.gen += 1;
        self.gen
    }

    fn remember(list: &mut VecDeque<String>, id: String, keep: usize) {
        if list.len() == keep {
            list.pop_front();
        }
        list.push_back(id);
    }

    fn current_of(&mut self, call_id: &str) -> Option<&mut Current> {
        self.current.as_mut().filter(|c| c.view.call_id == call_id)
    }

    fn early_of(&self, call_id: &str) -> Option<&Early> {
        self.early.iter().find(|(id, _)| id == call_id).map(|(_, e)| e)
    }

    fn early_mut(&mut self, call_id: &str) -> &mut Early {
        if let Some(i) = self.early.iter().position(|(id, _)| id == call_id) {
            return &mut self.early[i].1;
        }
        if self.early.len() == EARLY_KEPT {
            self.early.pop_front();
        }
        self.early.push_back((call_id.to_string(), Early::default()));
        &mut self.early.back_mut().expect("just pushed").1
    }

    fn take_early(&mut self, call_id: &str) -> Option<Early> {
        let i = self.early.iter().position(|(id, _)| id == call_id)?;
        self.early.remove(i).map(|(_, e)| e)
    }
}

/// What a signal asks to be done once the lock is released.
enum Followup {
    None,
    Accept(String),
}

struct Inner {
    store: Store,
    dm: DmService,
    feed: Feed,
    engine: Arc<dyn MediaEngine>,
    servers: Arc<dyn ServerSets>,
    nodes: NodeClient,
    clock: Arc<dyn Clock>,
    signer: RwLock<Option<Keys>>,
    state: Mutex<State>,
    outlet: mpsc::UnboundedSender<Effect>,
}

#[derive(Clone)]
pub struct CallService {
    inner: Arc<Inner>,
}

impl CallService {
    /// The service and the outlet of its effects: `Send` to publish (the
    /// outbox), `Emit` to show. The host drains it for as long as it runs.
    pub fn new(
        store: Store,
        dm: DmService,
        engine: Arc<dyn MediaEngine>,
        servers: Arc<dyn ServerSets>,
        nodes: NodeClient,
        clock: Arc<dyn Clock>,
    ) -> (Self, mpsc::UnboundedReceiver<Effect>) {
        let (outlet, rx) = mpsc::unbounded_channel();
        let feed = Feed::new(store.clone(), dm.clone());
        let inner = Inner {
            store,
            dm,
            feed,
            engine,
            servers,
            nodes,
            clock,
            signer: RwLock::new(None),
            state: Mutex::new(State::default()),
            outlet,
        };
        (Self { inner: Arc::new(inner) }, rx)
    }

    /// The keys of the session; `None` on logout: no signal is sent or
    /// read, a call under way is ended.
    pub fn set_signer(&self, keys: Option<Keys>) {
        let gone = keys.is_none();
        *self.inner.signer.write().unwrap() = keys;
        if gone {
            let inner = self.inner.clone();
            tokio::spawn(async move {
                let mut st = inner.state.lock().await;
                if st.current.is_some() {
                    inner.end_locally(&mut st, Outcome::Failed).await;
                }
            });
        }
    }

    pub async fn policy(&self) -> Result<RelayPolicy> {
        Ok(settings::get(&self.inner.store, KEY_RELAY_POLICY).await?.as_deref().and_then(RelayPolicy::parse).unwrap_or_default())
    }

    /// For the next call; the current one keeps its way.
    pub async fn set_policy(&self, policy: RelayPolicy) -> Result<()> {
        settings::set(&self.inner.store, KEY_RELAY_POLICY, policy.as_str()).await
    }

    pub async fn current(&self) -> Option<CallView> {
        self.inner.state.lock().await.current.as_ref().map(|c| c.view.clone())
    }

    /// Whether this device takes calls (`call.incoming_enabled`, on by
    /// default). Off: every invitation is ignored without a word, so
    /// that my other devices ring for it; nothing goes on record here.
    pub async fn incoming_enabled(&self) -> Result<bool> {
        self.inner.incoming_enabled().await
    }

    /// For the invitations to come; a call ringing now goes on ringing.
    pub async fn set_incoming_enabled(&self, enabled: bool) -> Result<()> {
        settings::set(&self.inner.store, KEY_INCOMING_ENABLED, if enabled { "true" } else { "false" }).await
    }

    /// The platform saw the network change (an interface came or went):
    /// the call under way restarts ICE at once, as it does when the
    /// engine says so itself. Nothing without a call that talks.
    pub async fn network_changed(&self) {
        let inner = &self.inner;
        let mut st = inner.state.lock().await;
        inner.on_network_changed(&mut st).await;
    }

    pub async fn video_quality(&self) -> Result<VideoQuality> {
        self.inner.video_quality().await
    }

    /// For the next time my video goes on; what is on keeps its size.
    pub async fn set_video_quality(&self, quality: VideoQuality) -> Result<()> {
        settings::set(&self.inner.store, KEY_VIDEO_QUALITY, quality.as_str()).await
    }

    /// The cameras the engine can open (none on a phone: its plugin holds
    /// the camera and pushes its frames).
    pub async fn cameras(&self) -> Vec<CameraInfo> {
        self.inner.engine.cameras().await
    }

    /// The screens and windows the engine can share (a computer).
    pub async fn screens(&self) -> Vec<ScreenInfo> {
        self.inner.engine.screens().await
    }

    /// My video in the call under way: the camera, the screen, or off.
    /// The peer is told; nothing is renegotiated. A camera that will not
    /// open is an error, and the video is off.
    pub async fn set_video(&self, input: VideoInput) -> Result<CallView> {
        let inner = &self.inner;
        let quality = inner.video_quality().await?;
        let mut st = inner.state.lock().await;
        inner.turn_video(&mut st, input, quality).await
    }

    /// The next camera (or the one `id` names): switched at once when my
    /// camera is on, kept for when it goes on otherwise.
    pub async fn switch_camera(&self, id: Option<String>) -> Result<CallView> {
        let inner = &self.inner;
        let mut ids: Vec<String> = inner.engine.cameras().await.into_iter().map(|c| c.id).collect();
        if ids.is_empty() {
            ids = PHONE_CAMERAS.iter().map(|s| s.to_string()).collect();
        }
        let quality = inner.video_quality().await?;
        let mut st = inner.state.lock().await;
        let Some(cur) = st.current.as_mut() else {
            return Err(MessengerError::Invalid("no call".into()));
        };
        let next = match id {
            Some(id) => id,
            None => {
                // The one after the current in the list, around the end;
                // the second when none was chosen yet (the first is in use).
                let at = cur.view.camera.as_ref().and_then(|c| ids.iter().position(|i| i == c)).unwrap_or(0);
                ids[(at + 1) % ids.len()].clone()
            }
        };
        if cur.view.video_local && !cur.view.video_screen {
            return inner.turn_video(&mut st, VideoInput::Camera { id: Some(next) }, quality).await;
        }
        cur.view.camera = Some(next);
        let view = cur.view.clone();
        inner.emit(vec![state_event(&view)]);
        Ok(view)
    }

    /// The frames of one video of the call under way, for the screen;
    /// `None` without a call, or before its media is there.
    pub async fn video_frames(&self, track: VideoTrack) -> Option<broadcast::Receiver<Arc<VideoFrame>>> {
        let st = self.inner.state.lock().await;
        st.current.as_ref()?.session.as_ref()?.video_frames(track)
    }

    /// A frame the platform captured, as my video (the camera of a phone).
    pub async fn push_video_frame(&self, frame: PushedFrame) -> Result<()> {
        let st = self.inner.state.lock().await;
        let session = st.current.as_ref().and_then(|c| c.session.as_ref()).ok_or_else(|| MessengerError::Invalid("no call".into()))?;
        session.push_video_frame(frame)
    }

    /// Call `peer`. Refused while a call is under way, and for anybody we
    /// are not in a mutual chat with.
    pub async fn start(&self, peer: &PubKey, media: Media) -> Result<CallView> {
        let inner = &self.inner;
        let keys = inner.signer()?;
        if peer.as_hex() == keys.public_key().to_hex() {
            return Err(MessengerError::Invalid("that is your own key".into()));
        }
        if !inner.dm.calls_allowed(peer).await? {
            return Err(MessengerError::Invalid("calls go to contacts only".into()));
        }
        let policy = self.policy().await?;
        let now = inner.clock.now().secs();
        let gen = {
            let mut st = inner.state.lock().await;
            if st.current.is_some() {
                return Err(MessengerError::Invalid("a call is under way".into()));
            }
            let gen = st.next_gen();
            let call_id = new_call_id();
            let view = CallView::new(call_id.clone(), peer.as_hex().to_string(), Direction::Out, media, Phase::Outgoing, now);
            st.current = Some(Current::new(view.clone(), gen, policy));
            let effects = inner.feed.begin(&call_id, peer.as_hex(), Direction::Out, media, now).await?;
            inner.emit(effects);
            inner.emit(vec![state_event(&view)]);
            inner.schedule(RING_TIMEOUT, Timer::Ring, gen);
            gen
        };
        // The nodes are asked without the lock: a signal may come meanwhile.
        let picked = inner.pick(policy, now).await;
        let mut st = inner.state.lock().await;
        let Some(cur) = st.current.as_mut().filter(|c| c.gen == gen) else {
            return Err(MessengerError::Invalid("the call ended".into()));
        };
        let picked = match picked {
            Ok(p) => p,
            Err(e) => {
                inner.end_locally(&mut st, Outcome::Failed).await;
                return Err(e);
            }
        };
        cur.view.nodes = picked.nodes;
        cur.view.limits = picked.limits;
        let session = match inner.engine.create_session(picked.servers, policy, media).await {
            Ok(s) => s,
            Err(e) => {
                inner.end_locally(&mut st, Outcome::Failed).await;
                return Err(e);
            }
        };
        inner.pump(session.events(), gen);
        let offer = match session.create_offer().await {
            Ok(o) => o,
            Err(e) => {
                session.close().await;
                inner.end_locally(&mut st, Outcome::Failed).await;
                return Err(e);
            }
        };
        let cur = st.current.as_mut().expect("checked above");
        cur.session = Some(session);
        cur.offers = 1;
        cur.first = Some((FirstPiece::Invite { restart: false }, offer));
        inner.schedule(GATHER_WAIT, Timer::Gather, gen);
        if media == Media::Video {
            // The camera, before the invitation leaves; a camera that
            // fails leaves an audio call, told so with the invitation.
            inner.auto_video(&mut st).await;
        }
        Ok(st.current.as_ref().expect("checked above").view.clone())
    }

    /// Take the ringing call.
    pub async fn accept(&self, call_id: &str) -> Result<CallView> {
        let inner = &self.inner;
        inner.signer()?;
        let (gen, policy, media) = {
            let mut st = inner.state.lock().await;
            let now = inner.clock.now().secs();
            let Some(cur) = st.current_of(call_id).filter(|c| c.view.phase == Phase::Incoming) else {
                return Err(MessengerError::Invalid("no such call is ringing".into()));
            };
            cur.view.phase = Phase::Connecting;
            cur.view.answered_at = Some(now);
            let (gen, policy, media, view) = (cur.gen, cur.policy, cur.view.media, cur.view.clone());
            inner.feed.answered(call_id, now).await?;
            inner.emit(vec![state_event(&view)]);
            (gen, policy, media)
        };
        let picked = inner.pick(policy, inner.clock.now().secs()).await;
        let mut st = inner.state.lock().await;
        let Some(cur) = st.current.as_mut().filter(|c| c.gen == gen) else {
            return Err(MessengerError::Invalid("the call ended".into()));
        };
        let result: Result<CallView> = async {
            let picked = picked?;
            cur.view.nodes = picked.nodes;
            cur.view.limits = picked.limits;
            let session = inner.engine.create_session(picked.servers, policy, media).await?;
            inner.pump(session.events(), gen);
            let offer = cur.remote_offer.take().ok_or_else(|| MessengerError::Invalid("no offer".into()))?;
            if let Err(e) = session.set_remote(&offer, SdpKind::Offer).await {
                session.close().await;
                return Err(e);
            }
            for c in cur.remote_ice.drain(..) {
                let _ = session.add_ice(&c).await;
            }
            let answer = match session.create_answer().await {
                Ok(a) => a,
                Err(e) => {
                    session.close().await;
                    return Err(e);
                }
            };
            cur.session = Some(session);
            cur.offers = 1;
            cur.first = Some((FirstPiece::Answer, answer));
            inner.schedule(GATHER_WAIT, Timer::Gather, gen);
            // The caller may be gone by now: the answer has its time too.
            inner.arm_connect(cur);
            Ok(cur.view.clone())
        }
        .await;
        match result {
            Ok(mut view) => {
                if media == Media::Video {
                    inner.auto_video(&mut st).await;
                    if let Some(cur) = st.current.as_ref() {
                        view = cur.view.clone();
                    }
                }
                Ok(view)
            }
            Err(e) => {
                let id = call_id.to_string();
                inner.send(&mut st, &Signal::End { call_id: id, reason: signal::reason::FAILED.into(), answer: None }, true).await;
                inner.end_locally(&mut st, Outcome::Failed).await;
                Err(e)
            }
        }
    }

    /// Refuse the ringing call; my other devices stop ringing too.
    pub async fn decline(&self, call_id: &str) -> Result<()> {
        let inner = &self.inner;
        inner.signer()?;
        let mut st = inner.state.lock().await;
        if st.current_of(call_id).filter(|c| c.view.phase == Phase::Incoming).is_none() {
            return Err(MessengerError::Invalid("no such call is ringing".into()));
        }
        inner.send(&mut st, &Signal::Decline { call_id: call_id.into(), reason: signal::reason::DECLINED.into() }, true).await;
        inner.end_locally(&mut st, Outcome::Declined).await;
        Ok(())
    }

    /// Hang up, or give up calling. A ringing incoming call is declined.
    pub async fn end(&self, call_id: &str) -> Result<()> {
        let inner = &self.inner;
        inner.signer()?;
        let mut st = inner.state.lock().await;
        let Some(cur) = st.current_of(call_id) else {
            return Err(MessengerError::Invalid("no such call".into()));
        };
        let (phase, answered, invite_sent) = (cur.view.phase, cur.view.answered_at.is_some(), cur.invite_sent);
        let id = call_id.to_string();
        match phase {
            Phase::Incoming => {
                inner.send(&mut st, &Signal::Decline { call_id: id, reason: signal::reason::DECLINED.into() }, true).await;
                inner.end_locally(&mut st, Outcome::Declined).await;
            }
            Phase::Outgoing if !answered => {
                if invite_sent {
                    inner.send(&mut st, &Signal::End { call_id: id, reason: signal::reason::ENDED.into(), answer: None }, true).await;
                }
                inner.end_locally(&mut st, Outcome::Missed).await;
            }
            _ => {
                inner.send(&mut st, &Signal::End { call_id: id, reason: signal::reason::ENDED.into(), answer: None }, true).await;
                inner.end_locally(&mut st, Outcome::Ended).await;
            }
        }
        Ok(())
    }

    pub async fn set_mute(&self, muted: bool) -> Result<CallView> {
        let inner = &self.inner;
        let mut st = inner.state.lock().await;
        let Some(cur) = st.current.as_mut() else {
            return Err(MessengerError::Invalid("no call".into()));
        };
        if let Some(s) = cur.session.as_ref() {
            s.set_mute(muted).await?;
        }
        cur.view.muted = muted;
        let view = cur.view.clone();
        inner.emit(vec![state_event(&view)]);
        Ok(view)
    }

    /// A `call.*` envelope that came as a DM (the handler calls this).
    /// Where it came from and when this session began do not matter: an
    /// invitation is judged by its own time alone, so that a phone woken
    /// by the push of it rings.
    pub async fn on_dm(&self, msg: &DmInbound, envelope: &Envelope, ctx: &Context) -> Result<()> {
        let inner = &self.inner;
        if inner.signer().is_err() {
            return Ok(());
        }
        let me = &ctx.my_pubkey;
        let from_me = &msg.sender == me;
        let peer = if from_me {
            match msg.recipients.iter().find(|p| *p != me) {
                Some(p) => p.clone(),
                None => return Ok(()),
            }
        } else {
            msg.sender.clone()
        };
        let Some(sig) = Signal::parse(envelope) else { return Ok(()) };
        let rumor = msg.rumor_id.as_hex().to_string();
        let followup = {
            let mut st = inner.state.lock().await;
            if from_me && st.sent.contains(&rumor) {
                return Ok(());
            }
            if !from_me && !inner.dm.calls_allowed(&peer).await? {
                return Ok(());
            }
            let at = msg.created_at.secs();
            if from_me {
                inner.on_copy(&mut st, &peer, sig, at).await?;
                Followup::None
            } else {
                inner.on_signal(&mut st, &peer, sig, &rumor, at).await?
            }
        };
        if let Followup::Accept(call_id) = followup {
            let _ = self.accept(&call_id).await;
        }
        Ok(())
    }
}

impl Current {
    fn new(view: CallView, gen: u64, policy: RelayPolicy) -> Self {
        Self {
            gen,
            policy,
            session: None,
            remote_offer: None,
            remote_ice: Vec::new(),
            first: None,
            gathered: Vec::new(),
            pending_ice: Vec::new(),
            flush_scheduled: false,
            invite_sent: false,
            taken_answer: None,
            my_answer: None,
            awaiting_restart: false,
            restart_tried: false,
            retried: false,
            peer_live_restart: false,
            asked_early: false,
            answered_offers: 0,
            connect_attempt: 0,
            connected: false,
            loss: 0,
            offers: 0,
            restarting_for: None,
            told_video: view.media == Media::Video,
            view,
        }
    }

    /// The call shows as restoring its way (the engine says it is gone).
    fn show_reconnecting(&mut self, reason: ReconnectReason) {
        self.view.phase = Phase::Reconnecting;
        self.view.reconnect_reason = Some(reason);
        self.view.via = None;
    }

    /// The call shows as talking again.
    fn show_active(&mut self) {
        self.view.phase = Phase::Active;
        self.view.reconnect_reason = None;
    }

    /// The most my video may spend here: the size's own, and no more
    /// than the node allows on a relayed way (`limits` of its welcome).
    fn video_cap(&self, quality: VideoQuality) -> Option<u32> {
        let (_, _, kbps) = quality.profile();
        let relayed = self.view.via == Some(PairKind::Relay);
        match self.view.limits.as_ref().map(|l| l.turn_kbps_per_allocation) {
            Some(node) if relayed && node > 0 => Some(kbps.min(node)),
            _ => Some(kbps),
        }
    }

    fn in_call(&self) -> bool {
        matches!(self.view.phase, Phase::Connecting | Phase::Active | Phase::Reconnecting) && self.session.is_some()
    }

    /// Whether the engine has the peer's description to put candidates
    /// against: the callee's once I took the call, the caller's with the
    /// answer (the first, and the one to a restart offer). A candidate
    /// given earlier is refused by the engine, so it waits.
    fn remote_ready(&self) -> bool {
        self.session.is_some() && !self.awaiting_restart && !(self.view.direction == Direction::Out && self.taken_answer.is_none())
    }
}

fn state_event(view: &CallView) -> Effect {
    Effect::Emit(UiEvent { name: UI_EVENT_CALL_STATE.into(), payload: serde_json::json!({ "call": view }) })
}

impl Inner {
    fn signer(&self) -> Result<Keys> {
        self.signer.read().unwrap().clone().ok_or(MessengerError::NotLoggedIn)
    }

    fn emit(&self, effects: Vec<Effect>) {
        for e in effects {
            let _ = self.outlet.send(e);
        }
    }

    async fn pick(&self, policy: RelayPolicy, now: i64) -> Result<crate::node_client::Picked> {
        let nodes = self.servers.call_nodes().await.unwrap_or_default();
        self.nodes.pick(&nodes, policy, now).await
    }

    async fn video_quality(&self) -> Result<VideoQuality> {
        Ok(settings::get(&self.store, KEY_VIDEO_QUALITY).await?.as_deref().and_then(VideoQuality::parse).unwrap_or_default())
    }

    async fn incoming_enabled(&self) -> Result<bool> {
        Ok(settings::get(&self.store, KEY_INCOMING_ENABLED).await?.as_deref() != Some("false"))
    }

    /// My video as `input`, in the call under way: the engine is asked,
    /// the view says what came of it, the peer is told of a change.
    async fn turn_video(&self, st: &mut State, input: VideoInput, quality: VideoQuality) -> Result<CallView> {
        let Some(cur) = st.current.as_mut() else {
            return Err(MessengerError::Invalid("no call".into()));
        };
        let Some(session) = cur.session.as_ref() else {
            return Err(MessengerError::Invalid("the call has no media yet".into()));
        };
        let (width, height, _) = quality.profile();
        let settings = VideoSettings { width, height, fps: VIDEO_FPS, max_kbps: cur.video_cap(quality) };
        let outcome = session.set_video(input.clone(), settings).await;
        match (&outcome, &input) {
            (Ok(()), VideoInput::Off) | (Err(_), _) => {
                // Off, or the engine could not: nothing goes.
                cur.view.video_local = false;
                cur.view.video_screen = false;
                cur.view.video_local_size = None;
            }
            (Ok(()), VideoInput::Camera { id }) => {
                cur.view.video_local = true;
                cur.view.video_screen = false;
                if id.is_some() {
                    cur.view.camera = id.clone();
                }
            }
            (Ok(()), VideoInput::Screen { .. }) => {
                cur.view.video_local = true;
                cur.view.video_screen = true;
            }
        }
        self.tell_video(st).await;
        let view = st.current.as_ref().expect("still here").view.clone();
        self.emit(vec![state_event(&view)]);
        outcome.map(|()| view)
    }

    /// The camera of a video call, as it begins: a failure is reported as
    /// an error event and leaves an audio call, which the peer is told of.
    async fn auto_video(&self, st: &mut State) {
        let quality = self.video_quality().await.unwrap_or_default();
        let camera = st.current.as_ref().and_then(|c| c.view.camera.clone());
        if let Err(e) = self.turn_video(st, VideoInput::Camera { id: camera }, quality).await {
            self.emit(vec![error_event(&e)]);
        }
    }

    /// `call.video` to the peer when what it believes of my video is no
    /// longer so. Before the invitation left nothing is sent: the
    /// invitation goes first, and the note follows it (`flush_first`).
    async fn tell_video(&self, st: &mut State) {
        let Some(cur) = st.current.as_mut() else { return };
        let on = cur.view.video_local;
        if cur.told_video == on || (cur.view.direction == Direction::Out && !cur.invite_sent) {
            return;
        }
        cur.told_video = on;
        let id = cur.view.call_id.clone();
        self.send(st, &Signal::Video { call_id: id, on }, false).await;
    }

    fn schedule(self: &Arc<Self>, after: Duration, timer: Timer, gen: u64) {
        let inner = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(after).await;
            inner.on_timer(timer, gen).await;
        });
    }

    /// The connect timeout of a new attempt; an earlier attempt's timer,
    /// still to come, is thereby void.
    fn arm_connect(self: &Arc<Self>, cur: &mut Current) {
        cur.connect_attempt += 1;
        self.schedule(CONNECT_TIMEOUT, Timer::Connect { attempt: cur.connect_attempt }, cur.gen);
    }

    /// The time the restart of the current attempt has to settle.
    fn arm_settle(self: &Arc<Self>, cur: &Current) {
        self.schedule(RESTART_SETTLE, Timer::Settle { attempt: cur.connect_attempt }, cur.gen);
    }

    fn pump(self: &Arc<Self>, mut events: mpsc::Receiver<SessionEvent>, gen: u64) {
        let inner = self.clone();
        tokio::spawn(async move {
            while let Some(ev) = events.recv().await {
                inner.on_engine_event(gen, ev).await;
            }
        });
    }

    /// Wrap and queue `signal` for the peer of the current call. The
    /// invitation goes as a message that wakes the peer and expires in
    /// a minute; the rest as notes that expire in five. A note that ends
    /// a ringing is marked so on the outside (`["call", "0"]`,
    /// [`Wake::CallEnd`]): the copy to my own devices of an answer, a
    /// refusal, a busy and an end (they ring for the same invitation),
    /// and the peer's copy of a refusal, a busy and an end (the peer's
    /// phone may hold the ringing from a push, with the app not up). The
    /// rest (`call.ice`, `call.restart`, `call.video`, a restart offer)
    /// is quiet. Errors are logged as effects of nothing: a call cannot
    /// do more than try.
    async fn send(&self, st: &mut State, signal: &Signal, self_copy: bool) -> Option<String> {
        let peer = PubKey::parse(&st.current.as_ref()?.view.peer)?;
        self.send_to(st, &peer, signal, self_copy).await
    }

    async fn send_to(&self, st: &mut State, peer: &PubKey, signal: &Signal, self_copy: bool) -> Option<String> {
        let keys = self.signer().ok()?;
        let now = self.clock.now().secs();
        let content = signal.to_envelope().encode();
        let wrapped = match signal {
            Signal::Invite { restart: false, .. } => wrap_expiring(&keys, peer, &content, now, now + INVITE_EXPIRATION_SECS),
            _ => {
                let ends_mine = matches!(signal, Signal::Answer { .. } | Signal::Decline { .. } | Signal::Busy { .. } | Signal::End { .. });
                let ends_theirs = matches!(signal, Signal::Decline { .. } | Signal::Busy { .. } | Signal::End { .. });
                let word = |ends: bool| if ends { Wake::CallEnd } else { Wake::Nobody };
                let mine = self_copy.then_some(word(ends_mine));
                wrap_note_as(&keys, peer, &content, now, Some(now + NOTE_EXPIRATION_SECS), word(ends_theirs), mine)
            }
        };
        let w = match wrapped {
            Ok(w) => w,
            Err(_) => return None,
        };
        let hint_relays = self.dm.hints(peer).await.unwrap_or_default();
        let rumor = w.rumor_id.as_hex().to_string();
        State::remember(&mut st.sent, rumor.clone(), SENT_KEPT);
        let mut effects = vec![Effect::Send(Outbound::PublishToInbox { recipient: peer.clone(), event: w.to_peer, hint_relays })];
        if let Some(event) = w.to_self {
            effects.push(Effect::Send(Outbound::PublishOwn { event }));
        }
        self.emit(effects);
        Some(rumor)
    }

    /// The current call is over here: the session closed, the record
    /// finished, the screen told. Nothing is sent; the caller did that.
    async fn end_locally(&self, st: &mut State, outcome: Outcome) {
        let Some(cur) = st.current.take() else { return };
        st.next_gen();
        if let Some(s) = cur.session {
            s.close().await;
        }
        State::remember(&mut st.recent, cur.view.call_id.clone(), RECENT_KEPT);
        let now = self.clock.now().secs();
        match self.feed.finish(&cur.view, outcome, now).await {
            Ok(effects) => self.emit(effects),
            Err(e) => self.emit(vec![error_event(&e)]),
        }
    }

    /// Another device of mine has the call (or the call I rang for never
    /// was): mine is dropped without a record of its own, and the screen
    /// told.
    async fn drop_for_elsewhere(&self, st: &mut State) {
        let Some(cur) = st.current.take() else { return };
        st.next_gen();
        if let Some(s) = cur.session {
            s.close().await;
        }
        State::remember(&mut st.recent, cur.view.call_id.clone(), RECENT_KEPT);
        let mut view = cur.view;
        view.phase = Phase::Ended;
        self.emit(vec![Effect::Emit(UiEvent {
            name: crate::call::UI_EVENT_CALL_ENDED.into(),
            payload: serde_json::json!({ "call": view, "outcome": ANSWERED_ELSEWHERE, "duration_secs": null }),
        })]);
    }

    /// The first piece (offer or answer) leaves with the candidates
    /// gathered so far.
    async fn flush_first(&self, st: &mut State) {
        let Some(cur) = st.current.as_mut() else { return };
        let Some((piece, sdp)) = cur.first.take() else { return };
        // What came while the piece waited goes with it.
        let mut ice = std::mem::take(&mut cur.gathered);
        ice.append(&mut cur.pending_ice);
        let call_id = cur.view.call_id.clone();
        match piece {
            FirstPiece::Invite { restart } => {
                let media = cur.view.media;
                // The first invitation says I take a restart while the way
                // still works; a restart offer says nothing of it.
                let sig = Signal::Invite { call_id, media, sdp, ice, restart, live_restart: !restart };
                self.send(st, &sig, !restart).await;
                if let Some(cur) = st.current.as_mut() {
                    cur.invite_sent = true;
                }
                // The camera of a video call failed before the invitation
                // left: the peer learns of it now, after the invitation.
                self.tell_video(st).await;
            }
            FirstPiece::Answer => {
                let sig = Signal::Answer { call_id, sdp, ice, live_restart: true };
                let rumor = self.send(st, &sig, true).await;
                if let Some(cur) = st.current.as_mut() {
                    if cur.my_answer.is_none() {
                        cur.my_answer = rumor;
                    }
                }
            }
        }
    }

    async fn flush_ice(&self, st: &mut State) {
        let Some(cur) = st.current.as_mut() else { return };
        cur.flush_scheduled = false;
        if cur.pending_ice.is_empty() || cur.first.is_some() {
            return;
        }
        let sig = Signal::Ice { call_id: cur.view.call_id.clone(), ice: std::mem::take(&mut cur.pending_ice) };
        self.send(st, &sig, false).await;
    }

    /// The connection was lost, or the network changed: the caller makes
    /// a new offer for the same call; the called side asks the caller for
    /// one. Two offers at once would need a rollback in the engine, so
    /// only one side ever makes them. The offer (and the request) leave
    /// at once, without waiting for candidates: they trickle after.
    ///
    /// The screen sees `Reconnecting` only when the engine says the way
    /// is gone; while it still works (a late request of the peer, a new
    /// interface beside the old one) the call stays `Active` and the
    /// restart goes on underneath.
    async fn restart(self: &Arc<Self>, st: &mut State, reason: ReconnectReason) {
        let Some(cur) = st.current.as_mut().filter(|c| c.in_call()) else { return };
        let Some(session) = cur.session.as_ref() else { return };
        if cur.connected && reason == ReconnectReason::NetworkChanged && !cur.peer_live_restart {
            // The old way still works, and the peer (5.1.2) would move to
            // `connecting` on a restart and give up when no `Connected`
            // comes to it: none is made on my word alone. When the old
            // way goes, the loss gets its restart, and the peer, whose
            // way went too, connects anew.
            return;
        }
        // One restart per loss: the next comes after the engine connected
        // (or this one settled on a way that never went).
        cur.restart_tried = true;
        cur.restarting_for = Some(reason);
        // Shown as restoring only when the way went: a restart while the
        // first way is still being made keeps `Connecting`.
        let show = !cur.connected && cur.view.phase != Phase::Connecting;
        if cur.view.direction == Direction::In {
            cur.awaiting_restart = true;
            if show {
                cur.show_reconnecting(reason);
            }
            let (id, seen, view) = (cur.view.call_id.clone(), cur.offers, cur.view.clone());
            self.arm_connect(cur);
            self.arm_settle(cur);
            self.send(st, &Signal::Restart { call_id: id, seen: Some(seen) }, false).await;
            self.emit(vec![state_event(&view)]);
            return;
        }
        match session.restart_ice().await {
            Ok(offer) => {
                cur.offers += 1;
                cur.first = Some((FirstPiece::Invite { restart: true }, offer));
                // Candidates of the old way, not yet sent, are of no use now.
                cur.gathered.clear();
                cur.pending_ice.clear();
                cur.awaiting_restart = true;
                if show {
                    cur.show_reconnecting(reason);
                }
                let view = cur.view.clone();
                self.arm_connect(cur);
                self.arm_settle(cur);
                self.flush_first(st).await;
                self.emit(vec![state_event(&view)]);
            }
            Err(_) => {
                let id = cur.view.call_id.clone();
                self.send(st, &Signal::End { call_id: id, reason: signal::reason::FAILED.into(), answer: None }, true).await;
                self.end_locally(st, Outcome::Failed).await;
            }
        }
    }

    /// The restart's description is in place (the caller took the answer,
    /// the called side sent its own): with the way still there, the call
    /// talks on as if nothing happened; without it, the engine's
    /// `Connected` (or the connect timer) is the judge.
    fn restart_settled(cur: &mut Current) {
        cur.awaiting_restart = false;
        cur.restarting_for = None;
        cur.answered_offers = cur.offers;
        if cur.connected {
            cur.restart_tried = false;
            cur.show_active();
        }
    }

    /// The network changed under the call (the engine or the platform
    /// says so): a restart at once, unless one is on its way already.
    async fn on_network_changed(self: &Arc<Self>, st: &mut State) {
        let Some(cur) = st.current.as_mut() else { return };
        if cur.in_call() && !cur.awaiting_restart {
            self.restart(st, ReconnectReason::NetworkChanged).await;
        }
    }

    async fn on_timer(self: &Arc<Self>, timer: Timer, gen: u64) {
        let mut st = self.state.lock().await;
        let Some(cur) = st.current.as_mut().filter(|c| c.gen == gen) else { return };
        match timer {
            Timer::Gather => self.flush_first(&mut st).await,
            Timer::IceFlush => self.flush_ice(&mut st).await,
            Timer::Ring => match cur.view.phase {
                Phase::Outgoing => {
                    let id = cur.view.call_id.clone();
                    if cur.invite_sent {
                        self.send(&mut st, &Signal::End { call_id: id, reason: signal::reason::TIMEOUT.into(), answer: None }, true).await;
                    }
                    self.end_locally(&mut st, Outcome::Missed).await;
                }
                Phase::Incoming => self.end_locally(&mut st, Outcome::Missed).await,
                _ => {}
            },
            Timer::Connect { attempt } => {
                if matches!(cur.view.phase, Phase::Connecting | Phase::Reconnecting) && cur.connect_attempt == attempt {
                    let id = cur.view.call_id.clone();
                    self.send(&mut st, &Signal::End { call_id: id, reason: signal::reason::FAILED.into(), answer: None }, true).await;
                    self.end_locally(&mut st, Outcome::Failed).await;
                }
            }
            Timer::Lost { loss } => {
                // Still gone, and no restart on its way: one is due.
                if cur.loss == loss && !cur.connected && !cur.awaiting_restart && !cur.restart_tried && cur.in_call() {
                    self.restart(&mut st, ReconnectReason::ConnectionLost).await;
                }
            }
            Timer::Settle { attempt } => {
                // The pieces of the restart had their time: one still
                // awaited was lost on the relays (or dropped by the peer),
                // and nothing is on its way any more.
                if cur.connect_attempt != attempt || !cur.awaiting_restart || !cur.in_call() {
                    return;
                }
                cur.awaiting_restart = false;
                cur.restarting_for = None;
                // The peer's candidates kept for the description that never
                // came go to the engine, which holds those of a generation
                // it does not know.
                if let Some(session) = cur.session.as_ref() {
                    for c in cur.remote_ice.drain(..) {
                        let _ = session.add_ice(&c).await;
                    }
                }
                if cur.connected {
                    // The old way still works: the call talks on, and the
                    // next loss gets its own restart.
                    cur.restart_tried = false;
                } else if !cur.retried {
                    // The way is gone and nothing is on its way: once more.
                    cur.retried = true;
                    self.restart(&mut st, ReconnectReason::ConnectionLost).await;
                }
                // Otherwise the connect timer of this attempt judges.
            }
        }
    }

    async fn on_engine_event(self: &Arc<Self>, gen: u64, ev: SessionEvent) {
        let mut st = self.state.lock().await;
        let Some(cur) = st.current.as_mut().filter(|c| c.gen == gen) else { return };
        match ev {
            SessionEvent::LocalCandidate(c) => {
                if cur.first.is_some() {
                    cur.gathered.push(c);
                } else {
                    cur.pending_ice.push(c);
                    if !cur.flush_scheduled {
                        cur.flush_scheduled = true;
                        self.schedule(ICE_DEBOUNCE, Timer::IceFlush, gen);
                    }
                }
            }
            SessionEvent::GatheringComplete => self.flush_first(&mut st).await,
            SessionEvent::ConnectionState(state) => match state {
                ConnectionState::Connected => {
                    let now = self.clock.now().secs();
                    cur.connected = true;
                    cur.restart_tried = false;
                    cur.retried = false;
                    if cur.view.direction == Direction::In && cur.awaiting_restart {
                        // The called side asked, and a way is there (the old
                        // one came back, or the caller's own offer made a
                        // new one): the offer, if it still comes, is taken
                        // all the same, and the caller's candidates no
                        // longer wait for it.
                        cur.awaiting_restart = false;
                        cur.restarting_for = None;
                        if let Some(session) = cur.session.as_ref() {
                            for c in cur.remote_ice.drain(..) {
                                let _ = session.add_ice(&c).await;
                            }
                        }
                    }
                    // The caller's restart offer on its way stays on its
                    // way: its answer is still to be taken, whatever the
                    // old way does meanwhile.
                    cur.show_active();
                    if cur.view.answered_at.is_none() {
                        cur.view.answered_at = Some(now);
                    }
                    let (id, view) = (cur.view.call_id.clone(), cur.view.clone());
                    let _ = self.feed.answered(&id, now).await;
                    self.emit(vec![state_event(&view)]);
                }
                ConnectionState::Disconnected | ConnectionState::Failed => {
                    cur.connected = false;
                    // Once talking, a loss gets one restart, after a moment
                    // to see whether the way comes back by itself; before
                    // that the connect timeout is the judge. While a restart
                    // is on its way the old way may well die: the timeout
                    // judges that too.
                    if !cur.in_call() {
                        // Ringing, or the offer still on its way: no way yet.
                    } else if cur.awaiting_restart {
                        // The restart on its way is the answer; shown now.
                        if cur.view.phase != Phase::Reconnecting {
                            let reason = cur.restarting_for.unwrap_or(ReconnectReason::ConnectionLost);
                            cur.show_reconnecting(reason);
                            let view = cur.view.clone();
                            self.emit(vec![state_event(&view)]);
                        }
                    } else if !cur.restart_tried {
                        if cur.view.phase != Phase::Reconnecting {
                            cur.show_reconnecting(ReconnectReason::ConnectionLost);
                            let view = cur.view.clone();
                            self.emit(vec![state_event(&view)]);
                        }
                        cur.loss += 1;
                        let (loss, gen) = (cur.loss, cur.gen);
                        self.schedule(LOSS_CONFIRM, Timer::Lost { loss }, gen);
                    } else if state == ConnectionState::Failed {
                        // The one restart of this loss did not help.
                        let id = cur.view.call_id.clone();
                        self.send(&mut st, &Signal::End { call_id: id, reason: signal::reason::FAILED.into(), answer: None }, true).await;
                        self.end_locally(&mut st, Outcome::Failed).await;
                    } else if cur.view.phase != Phase::Reconnecting {
                        // Restarted, not connected yet: the way went again
                        // before it came; the connect timer judges.
                        let reason = cur.restarting_for.unwrap_or(ReconnectReason::ConnectionLost);
                        cur.show_reconnecting(reason);
                        let view = cur.view.clone();
                        self.emit(vec![state_event(&view)]);
                    }
                }
                ConnectionState::New | ConnectionState::Connecting | ConnectionState::Closed => {}
            },
            SessionEvent::SelectedPair(kind) => {
                cur.view.via = Some(kind);
                let (id, view) = (cur.view.call_id.clone(), cur.view.clone());
                let _ = self.feed.via(&id, kind).await;
                // A relayed way is as wide as the node allows: my video
                // keeps within it from here on.
                if cur.view.video_local {
                    let quality = self.video_quality().await.unwrap_or_default();
                    let cap = cur.video_cap(quality);
                    if let Some(session) = cur.session.as_ref() {
                        let _ = session.set_video_bitrate(cap).await;
                    }
                }
                self.emit(vec![state_event(&view)]);
            }
            SessionEvent::VideoSize { track, width, height } => {
                let size = Some(VideoSize { width, height });
                let slot = match track {
                    VideoTrack::Local => &mut cur.view.video_local_size,
                    VideoTrack::Remote => &mut cur.view.video_remote_size,
                };
                if *slot != size {
                    *slot = size;
                    if track == VideoTrack::Remote {
                        // Frames say more than a word: they come, so it is on.
                        cur.view.video_remote = true;
                    }
                    let view = cur.view.clone();
                    self.emit(vec![state_event(&view)]);
                }
            }
            SessionEvent::Stats(stats) => {
                let id = cur.view.call_id.clone();
                self.emit(vec![Effect::Emit(UiEvent {
                    name: UI_EVENT_CALL_STATS.into(),
                    payload: serde_json::json!({ "call_id": id, "stats": stats }),
                })]);
            }
            SessionEvent::AudioLevel(level) => {
                let id = cur.view.call_id.clone();
                self.emit(vec![Effect::Emit(UiEvent {
                    name: UI_EVENT_CALL_LEVEL.into(),
                    payload: serde_json::json!({ "call_id": id, "level": level }),
                })]);
            }
            SessionEvent::NetworkChanged => self.on_network_changed(&mut st).await,
            SessionEvent::VideoLost { reason } => {
                // My camera or screen went away in the middle: the video
                // is off as if I had turned it off (the engine is told so
                // too, which costs it nothing), the peer learns of it by
                // `call.video`, and the screen hears why.
                if cur.view.video_local {
                    let quality = self.video_quality().await.unwrap_or_default();
                    let _ = self.turn_video(&mut st, VideoInput::Off, quality).await;
                    self.emit(vec![error_event(&MessengerError::Transport(format!("the video stopped: {reason}")))]);
                }
            }
            // The words of a room: a call between two has no data channel
            // and no m-lines of a node.
            SessionEvent::DataOpen { .. }
            | SessionEvent::DataClosed { .. }
            | SessionEvent::Data { .. }
            | SessionEvent::RemoteTrack { .. }
            | SessionEvent::RemoteTrackGone { .. }
            | SessionEvent::RemoteLevel { .. }
            | SessionEvent::RemoteVideoSize { .. } => {}
        }
    }

    /// A signal of the peer.
    async fn on_signal(self: &Arc<Self>, st: &mut State, peer: &PubKey, sig: Signal, rumor: &str, at: i64) -> Result<Followup> {
        let now = self.clock.now().secs();
        let peer_hex = peer.as_hex().to_string();
        match sig {
            Signal::Invite { call_id, media, sdp, ice, restart: false, live_restart } => {
                if st.recent.contains(&call_id) || st.current_of(&call_id).is_some() {
                    return Ok(Followup::None);
                }
                // This device takes no calls: the invitation is nobody's
                // business here, not even on record (my other devices ring
                // for it, and the one that takes it keeps the record); not
                // a word back, which would stop them ringing. Unless I am
                // calling the same peer: that is my own call coming back
                // (both called at once), settled below as any glare.
                let glare = st.current.as_ref().is_some_and(|c| c.view.peer == peer_hex && c.view.direction == Direction::Out);
                if !glare && !self.incoming_enabled().await? {
                    State::remember(&mut st.recent, call_id, RECENT_KEPT);
                    return Ok(Followup::None);
                }
                // The time inside the rumor is when the call began; a clock
                // ahead of mine makes no call of the future, nor a longer ring.
                let at = at.min(now);
                if st.early_of(&call_id).is_some_and(Early::settled) {
                    // Over, or taken on another device of mine, before I saw
                    // it: on record as that, and nothing rings.
                    let (effects, _) = self.begin_record(st, &call_id, &peer_hex, Direction::In, media, at).await?;
                    self.emit(effects);
                    return Ok(Followup::None);
                }
                if now - at > INVITE_TTL_SECS {
                    // A missed call, on record and in the chat; nothing rings.
                    let mut effects = self.feed.begin(&call_id, &peer_hex, Direction::In, media, at).await?;
                    if let Some(view) = self.view_of(&call_id).await? {
                        effects.extend(self.feed.finish(&view, Outcome::Missed, at).await?);
                    }
                    self.emit(effects);
                    return Ok(Followup::None);
                }
                let mut accept_now = false;
                if let Some(cur) = st.current.as_ref() {
                    // I am calling the same peer: both called at once.
                    if cur.view.peer == peer_hex && cur.view.direction == Direction::Out {
                        if call_id < cur.view.call_id && cur.view.phase == Phase::Outgoing {
                            // Both called at once: the smaller id wins, and it
                            // is theirs. Mine never was: everybody who heard of
                            // it (their other devices ring for it, mine have it
                            // on record) is told so.
                            let mine = cur.view.call_id.clone();
                            let same_media = cur.view.media == media;
                            let heard = cur.invite_sent;
                            let cur = st.current.take().expect("just seen");
                            st.next_gen();
                            if let Some(s) = cur.session {
                                s.close().await;
                            }
                            State::remember(&mut st.recent, mine.clone(), RECENT_KEPT);
                            if heard {
                                let sig = Signal::End { call_id: mine.clone(), reason: signal::reason::SUPERSEDED.into(), answer: None };
                                self.send_to(st, peer, &sig, true).await;
                            }
                            let effects = self.feed.forget(&mine).await?;
                            self.emit(effects);
                            // Theirs is my answer when it is what I asked for; a
                            // video call I did not ask for rings, as any other.
                            accept_now = same_media;
                        } else {
                            // Mine wins (the smaller id, or already taken);
                            // the peer takes it as the answer.
                            return Ok(Followup::None);
                        }
                    } else {
                        self.send_to(st, peer, &Signal::Busy { call_id: call_id.clone() }, true).await;
                        let mut effects = self.feed.begin(&call_id, &peer_hex, Direction::In, media, at).await?;
                        if let Some(view) = self.view_of(&call_id).await? {
                            effects.extend(self.feed.finish(&view, Outcome::Busy, now).await?);
                        }
                        self.emit(effects);
                        return Ok(Followup::None);
                    }
                }
                let gen = st.next_gen();
                let mut view = CallView::new(call_id.clone(), peer_hex.clone(), Direction::In, media, Phase::Incoming, at);
                // The peer's word of its video that overtook the invitation.
                if let Some(on) = st.take_early(&call_id).and_then(|e| e.video) {
                    view.video_remote = on;
                }
                let policy = settings::get(&self.store, KEY_RELAY_POLICY).await?.as_deref().and_then(RelayPolicy::parse).unwrap_or_default();
                let mut cur = Current::new(view.clone(), gen, policy);
                cur.remote_offer = Some(sdp);
                cur.remote_ice = ice;
                cur.peer_live_restart = live_restart;
                st.current = Some(cur);
                let mut effects = self.feed.begin(&call_id, &peer_hex, Direction::In, media, at).await?;
                effects.push(Effect::Emit(UiEvent { name: UI_EVENT_CALL_INCOMING.into(), payload: serde_json::json!({ "call": view }) }));
                self.emit(effects);
                // The rest of the 45 s the invitation is good for.
                let left = (at + INVITE_TTL_SECS - now).clamp(0, INVITE_TTL_SECS) as u64;
                self.schedule(Duration::from_secs(left), Timer::Ring, gen);
                Ok(if accept_now { Followup::Accept(call_id) } else { Followup::None })
            }
            Signal::Invite { call_id, sdp, ice, restart: true, .. } => {
                // The caller's new offer for the call; the caller takes none.
                let Some(cur) = st.current_of(&call_id).filter(|c| c.in_call() && c.view.direction == Direction::In) else {
                    return Ok(Followup::None);
                };
                cur.first = None;
                let session = cur.session.as_ref().expect("in_call");
                if session.set_remote(&sdp, SdpKind::Offer).await.is_err() {
                    return Ok(Followup::None);
                }
                cur.offers += 1;
                for c in ice.iter().chain(cur.remote_ice.iter()) {
                    let _ = session.add_ice(c).await;
                }
                cur.remote_ice.clear();
                if let Ok(answer) = session.create_answer().await {
                    cur.first = Some((FirstPiece::Answer, answer));
                    // Candidates of the old way, not yet sent, are of no use.
                    cur.gathered.clear();
                    cur.pending_ice.clear();
                    // One restart per loss, whoever made it: a loss this
                    // side saw too (its timer still to come) asks for no
                    // second one. With the way still here the restart is
                    // complete at once (`restart_settled`).
                    cur.restart_tried = true;
                    cur.loss += 1;
                    Self::restart_settled(cur);
                    if !cur.connected && cur.view.phase != Phase::Reconnecting {
                        // The caller's loss, and the way is gone here too
                        // (or not told as such by the engine yet): restoring.
                        cur.show_reconnecting(ReconnectReason::PeerLost);
                    }
                    let view = cur.view.clone();
                    self.arm_connect(cur);
                    // The answer leaves now; its candidates trickle after it.
                    self.flush_first(st).await;
                    self.emit(vec![state_event(&view)]);
                }
                Ok(Followup::None)
            }
            Signal::Restart { call_id, seen } => {
                // The called side lost the way: a new offer, unless one is
                // on its way already, or the request is older than my
                // latest offer (which answers it: the relays keep no order,
                // and a request made before that offer reached the called
                // side would otherwise restart a call just restored).
                let Some(cur) = st.current_of(&call_id).filter(|c| c.view.direction == Direction::Out) else {
                    return Ok(Followup::None);
                };
                if seen.is_some() {
                    // A called side that counts takes a restart while its
                    // way still works.
                    cur.peer_live_restart = true;
                }
                if !cur.in_call() {
                    // The called side's network changed as it answered: the
                    // request overtook its answer (which waits for
                    // candidates). Kept, and honoured with the answer.
                    if cur.view.phase == Phase::Outgoing && cur.invite_sent {
                        cur.asked_early = true;
                    }
                    return Ok(Followup::None);
                }
                // Older than my latest answered offer: that offer answered
                // it. An offer never answered (lost) settles nothing, and a
                // request made before it is honoured.
                let stale = seen.is_some_and(|n| n < cur.answered_offers);
                if !cur.awaiting_restart && !stale {
                    self.restart(st, ReconnectReason::PeerLost).await;
                }
                Ok(Followup::None)
            }
            Signal::Video { call_id, on } => {
                // The peer's camera went on or off: the screen shows a
                // placeholder or waits for frames. The size is the frames'.
                if let Some(cur) = st.current_of(&call_id) {
                    if cur.view.video_remote != on {
                        cur.view.video_remote = on;
                        if !on {
                            cur.view.video_remote_size = None;
                        }
                        let view = cur.view.clone();
                        self.emit(vec![state_event(&view)]);
                    }
                } else if !st.recent.contains(&call_id) {
                    // Before its invitation (the relays keep no order): kept
                    // for the view the invitation makes. A word of a call
                    // long over makes an entry that goes with the oldest.
                    st.early_mut(&call_id).video = Some(on);
                }
                Ok(Followup::None)
            }
            Signal::Answer { call_id, sdp, ice, live_restart } => {
                let Some(cur) = st.current_of(&call_id) else {
                    // Another device of mine called, and the peer took it:
                    // on record here too.
                    self.note_answered(st, &call_id, at).await?;
                    return Ok(Followup::None);
                };
                if cur.view.direction != Direction::Out {
                    return Ok(Followup::None);
                }
                if live_restart {
                    cur.peer_live_restart = true;
                }
                if cur.awaiting_restart {
                    if let Some(session) = cur.session.as_ref() {
                        let _ = session.set_remote(&sdp, SdpKind::Answer).await;
                        for c in ice.iter().chain(cur.remote_ice.iter()) {
                            let _ = session.add_ice(c).await;
                        }
                    }
                    cur.remote_ice.clear();
                    Self::restart_settled(cur);
                    let view = cur.view.clone();
                    self.emit(vec![state_event(&view)]);
                    return Ok(Followup::None);
                }
                if cur.taken_answer.is_some() {
                    // Another device of the peer answered too; it is told.
                    let sig = Signal::End { call_id, reason: signal::reason::ANSWERED_ELSEWHERE.into(), answer: Some(rumor.to_string()) };
                    self.send(st, &sig, false).await;
                    return Ok(Followup::None);
                }
                if cur.view.phase != Phase::Outgoing || !cur.invite_sent {
                    return Ok(Followup::None);
                }
                let Some(session) = cur.session.as_ref() else { return Ok(Followup::None) };
                if session.set_remote(&sdp, SdpKind::Answer).await.is_err() {
                    return Ok(Followup::None);
                }
                for c in ice.iter().chain(cur.remote_ice.iter()) {
                    let _ = session.add_ice(c).await;
                }
                cur.remote_ice.clear();
                cur.taken_answer = Some(rumor.to_string());
                cur.answered_offers = cur.offers;
                cur.view.phase = Phase::Connecting;
                cur.view.answered_at = Some(now);
                let (view, asked_early) = (cur.view.clone(), cur.asked_early);
                self.feed.answered(&call_id, now).await?;
                self.arm_connect(cur);
                self.emit(vec![state_event(&view)]);
                if asked_early {
                    // The called side's network changed as it answered: the
                    // offer it asked for, now that there is a call to make
                    // it in.
                    self.restart(st, ReconnectReason::PeerLost).await;
                }
                Ok(Followup::None)
            }
            Signal::Ice { call_id, ice } => {
                if let Some(cur) = st.current_of(&call_id) {
                    if cur.remote_ready() {
                        let session = cur.session.as_ref().expect("remote_ready");
                        for c in &ice {
                            let _ = session.add_ice(c).await;
                        }
                    } else {
                        cur.remote_ice.extend(ice);
                    }
                }
                Ok(Followup::None)
            }
            Signal::Decline { call_id, .. } => {
                match st.current_of(&call_id).map(|c| c.view.direction) {
                    Some(Direction::Out) => self.end_locally(st, Outcome::Declined).await,
                    Some(Direction::In) => {}
                    // Another device of mine called; the peer refused it.
                    None => self.note_end(st, &call_id, Outcome::Declined, at).await?,
                }
                Ok(Followup::None)
            }
            Signal::Busy { call_id } => {
                match st.current_of(&call_id).map(|c| c.view.direction) {
                    Some(Direction::Out) => self.end_locally(st, Outcome::Busy).await,
                    Some(Direction::In) => {}
                    None => self.note_end(st, &call_id, Outcome::Busy, at).await?,
                }
                Ok(Followup::None)
            }
            Signal::End { call_id, reason, answer } => {
                if reason == signal::reason::SUPERSEDED {
                    self.supersede(st, &call_id).await?;
                    return Ok(Followup::None);
                }
                if let Some(cur) = st.current_of(&call_id) {
                    if answer.is_some() && answer != cur.my_answer {
                        // For another device of mine.
                        return Ok(Followup::None);
                    }
                    if reason == signal::reason::ANSWERED_ELSEWHERE {
                        self.drop_for_elsewhere(st).await;
                        return Ok(Followup::None);
                    }
                    let outcome = outcome_of(&reason, cur.view.answered_at.is_some());
                    self.end_locally(st, outcome).await;
                } else if answer.is_none() {
                    // A call of my other device, or one I missed: the record.
                    self.note_end_by(st, &call_id, &reason, at).await?;
                }
                Ok(Followup::None)
            }
        }
    }

    /// A copy of a signal another device of mine sent.
    async fn on_copy(self: &Arc<Self>, st: &mut State, peer: &PubKey, sig: Signal, at: i64) -> Result<()> {
        match sig {
            Signal::Invite { call_id, media, restart: false, .. } => {
                // My other device called: the call is in my history too.
                let (effects, _) = self.begin_record(st, &call_id, peer.as_hex(), Direction::Out, media, at).await?;
                self.emit(effects);
            }
            Signal::Invite { .. } | Signal::Ice { .. } | Signal::Restart { .. } | Signal::Video { .. } => {}
            Signal::Answer { call_id, .. } => {
                match st.current_of(&call_id).map(|c| c.view.phase) {
                    // Another device of mine took it: this one stops ringing.
                    Some(Phase::Incoming) => {
                        self.drop_for_elsewhere(st).await;
                        self.feed.answered(&call_id, at).await?;
                    }
                    // I took it too; the caller says which answer it keeps.
                    Some(_) => self.feed.answered(&call_id, at).await?,
                    None => self.note_answered(st, &call_id, at).await?,
                }
            }
            // Another device of mine refused, or was busy: this one stops
            // ringing; the record says so either way.
            Signal::Decline { call_id, .. } => {
                if st.current_of(&call_id).filter(|c| c.view.phase == Phase::Incoming).is_some() {
                    self.end_locally(st, Outcome::Declined).await;
                } else {
                    self.note_end(st, &call_id, Outcome::Declined, at).await?;
                }
            }
            Signal::Busy { call_id } => {
                if st.current_of(&call_id).filter(|c| c.view.phase == Phase::Incoming).is_some() {
                    self.end_locally(st, Outcome::Busy).await;
                } else {
                    self.note_end(st, &call_id, Outcome::Busy, at).await?;
                }
            }
            Signal::End { call_id, reason, answer } => {
                if reason == signal::reason::SUPERSEDED {
                    // My other device's call lost a glare: it never was.
                    self.supersede(st, &call_id).await?;
                } else if let Some(cur) = st.current_of(&call_id) {
                    if answer.is_none() || answer == cur.my_answer {
                        let outcome = outcome_of(&reason, cur.view.answered_at.is_some());
                        self.end_locally(st, outcome).await;
                    }
                } else if answer.is_none() {
                    self.note_end_by(st, &call_id, &reason, at).await?;
                }
            }
        }
        Ok(())
    }

    /// The record of a call begins, and what came of the call before its
    /// invitation did is written into it at once. Returns the effects
    /// and that early word, if there was one.
    async fn begin_record(
        &self,
        st: &mut State,
        call_id: &str,
        peer: &str,
        direction: Direction,
        media: Media,
        at: i64,
    ) -> Result<(Vec<Effect>, Option<Early>)> {
        let mut effects = self.feed.begin(call_id, peer, direction, media, at).await?;
        let early = st.take_early(call_id);
        if let Some(early) = &early {
            if let Some(when) = early.answered_at {
                self.feed.answered(call_id, when).await?;
            }
            if let Some((outcome, when)) = early.end {
                if let Some(view) = self.view_of(call_id).await? {
                    effects.extend(self.feed.finish(&view, outcome, when).await?);
                }
            }
        }
        Ok((effects, early))
    }

    /// A call no device of mine holds was answered (the peer took my other
    /// device's call; my other device took the peer's): on record, or
    /// kept for the record when the invitation has not come yet.
    async fn note_answered(&self, st: &mut State, call_id: &str, at: i64) -> Result<()> {
        if st.recent.iter().any(|id| id == call_id) {
            // This device held the call and closed it; a late answer
            // changes nothing of its verdict.
            return Ok(());
        }
        match self.feed.get(call_id).await? {
            Some(row) => {
                if row.answered_at.is_none() {
                    self.feed.answered(call_id, at).await?;
                }
                // Its end came first and read it as missed; it was taken
                // before that end (by the times of the two).
                if row.outcome.as_deref() == Some(repo::OUTCOME_MISSED) && row.ended_at.is_none_or(|ended| at <= ended) {
                    let effects = self.feed.correct(call_id, Outcome::Ended).await?;
                    self.emit(effects);
                }
            }
            None => st.early_mut(call_id).answered(at),
        }
        Ok(())
    }

    /// A call no device of mine holds ended with `reason` (as `call.end`
    /// words it).
    async fn note_end_by(&self, st: &mut State, call_id: &str, reason: &str, at: i64) -> Result<()> {
        let answered = match self.feed.get(call_id).await? {
            Some(row) => row.answered_at.is_some(),
            None => st.early_of(call_id).is_some_and(|e| e.answered_at.is_some()),
        };
        self.note_end(st, call_id, outcome_of(reason, answered), at).await
    }

    /// A call no device of mine holds is over: the record closed, or the
    /// outcome kept for the record when the invitation has not come yet.
    async fn note_end(&self, st: &mut State, call_id: &str, outcome: Outcome, at: i64) -> Result<()> {
        if self.feed.get(call_id).await?.is_some() {
            self.close_record(call_id, outcome, at).await
        } else if st.recent.iter().any(|id| id == call_id) {
            // Held here and closed, or ignored on purpose (this device
            // takes no calls): nothing of it is kept.
            Ok(())
        } else {
            st.early_mut(call_id).ended(outcome, at);
            Ok(())
        }
    }

    /// A call that lost a glare never was: whoever rings for it here
    /// stops (the screen hears `answered_elsewhere`: the same peer is on
    /// the call that won), and the record of it goes.
    async fn supersede(&self, st: &mut State, call_id: &str) -> Result<()> {
        if st.current_of(call_id).is_some() {
            self.drop_for_elsewhere(st).await;
        } else {
            // Its invitation may still be on its way: it is not to ring.
            State::remember(&mut st.recent, call_id.to_string(), RECENT_KEPT);
        }
        st.take_early(call_id);
        let effects = self.feed.forget(call_id).await?;
        self.emit(effects);
        Ok(())
    }

    /// The record of a call no device of mine has any more: closed with
    /// `outcome`, which also overrides the "missed" of a device that
    /// learned of the call after another one took it.
    async fn close_record(&self, call_id: &str, outcome: Outcome, at: i64) -> Result<()> {
        let Some(row) = self.feed.get(call_id).await? else { return Ok(()) };
        if row.outcome.is_none() {
            if let Some(view) = self.view_of(call_id).await? {
                let effects = self.feed.finish(&view, outcome, at).await?;
                self.emit(effects);
            }
        } else if row.outcome.as_deref() == Some(repo::OUTCOME_MISSED) && outcome != Outcome::Missed {
            let effects = self.feed.correct(call_id, outcome).await?;
            self.emit(effects);
        }
        Ok(())
    }

    /// The view of a call on record, for the events of its end.
    async fn view_of(&self, call_id: &str) -> Result<Option<CallView>> {
        let Some(row) = self.feed.get(call_id).await? else { return Ok(None) };
        let direction = if row.direction == repo::DIR_OUT { Direction::Out } else { Direction::In };
        let media = Media::parse(&row.media).unwrap_or(Media::Audio);
        let mut view = CallView::new(row.call_id.clone(), row.peer.clone(), direction, media, Phase::Ended, row.started_at);
        view.chat_id = row.chat_id.clone();
        view.answered_at = row.answered_at;
        view.video_remote = false;
        Ok(Some(view))
    }
}

/// What the record says of an end told with `reason`.
fn outcome_of(reason: &str, answered: bool) -> Outcome {
    match reason {
        signal::reason::FAILED => Outcome::Failed,
        _ if answered => Outcome::Ended,
        _ => Outcome::Missed,
    }
}

fn error_event(e: &MessengerError) -> Effect {
    Effect::Emit(UiEvent { name: "error".into(), payload: serde_json::json!({ "scope": "calls", "error": e }) })
}
