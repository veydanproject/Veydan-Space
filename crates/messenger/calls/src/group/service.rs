// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The core of a group call: one room at a time, the calls announced in
//! the groups, what the group's notes, the node's control channel and
//! the engine tell, what to send and show. Everything it wants done
//! outside goes out as an `Effect` on the outlet the host drains, as the
//! call between two does ([`crate::service::CallService`]).
//!
//! ```text
//!   Idle ──start──▶ Starting (a room on the node) ──▶ Joining ──connected──▶ InRoom ──leave/end──▶ Left
//!        ──join───▶ Joining (the room of the group's call) ──┘                 │
//!                                                        way lost: Reconnecting ─┘
//! ```
//!
//! The rules (internal/messenger-wire.md §10, "Групповые звонки"):
//!
//! - Whoever starts makes the room on a node (the one pinned to the group,
//!   else the nearest with an SFU from the sets of servers), keeps the
//!   admin token, joins, then tells the group `call.start` with the room,
//!   the node, the join token and the secret of epoch 1, and `call.join`
//!   with its seat. Everybody else joins by the start and says
//!   `call.join`; `call.leave` on the way out. The last one out says
//!   `call.end`; the room ends on the node by itself when empty. Two
//!   starts of one moment (neither creator knew of the other) are one
//!   call: the newer holds, the older never was, and whoever sat in its
//!   room goes over.
//! - A seat is a person once its word of identity came on the control
//!   channel ([`crate::group::keys`]): signed by a member's key, naming
//!   the call, the room, the seat the node put in front of the frame and
//!   the DTLS fingerprint of its description. Until then the seat is
//!   shown as nobody and not listened to: no key is set for its m-lines.
//!   The word is sent when the channel opens, again whenever somebody
//!   joins, so that a newcomer gets everybody's, and again whenever the
//!   epoch changes. A word under a secret not here yet (the note of its
//!   epoch is on its way) is kept and opened when the note comes. A seat
//!   that has said nothing for [`VERIFY_DEADLINE`] is nobody for good: the
//!   creator puts it out of the room, or the keys turn without it.
//! - The seats of my room are the node's word alone (`hello`, `joined`,
//!   `left`, the tracks of its offers): a note of the group names a seat
//!   the node spoke of, never makes one.
//! - Keys: slot = epoch; sender key = HKDF(secret of the epoch, call,
//!   seat, epoch). When a seat leaves, the oldest verified seat left (the
//!   smallest number) makes the next epoch and tells the group
//!   `call.epoch`; every participant keys every verified seat's m-lines
//!   for it at once and moves its own sending to it [`SEND_SWITCH_DELAY`]
//!   later, so that the others have read the note by then. Two rotations
//!   that met on one number (the rotator left before its note arrived,
//!   the next oldest rotated too) are settled alike everywhere: the
//!   smaller secret holds. Nobody rotates on a join: whoever joins is a
//!   member and reads the secrets of the group anyway. When the group
//!   loses a member, its seat is nobody from then on, the creator puts it
//!   out and changes the token of the room (told in its `call.epoch`),
//!   and the keys turn: what it kept of the secrets opens nothing new.
//! - What the banner shows is built from the notes: a start without an
//!   end, with the seats of the joins and leaves (a seat claimed once is
//!   not claimed over; a verified word of identity is the last word on
//!   it), dropped once the room would have expired on the node
//!   (`expires_at` of the start) or the node answers `room_not_found` to
//!   a join. No heartbeat: the node is the judge of a room, and a join
//!   asks it. A call that expired without an end is closed on record
//!   when I last left it.
//! - The node's refusals have their words: `room_not_found` (404) ends
//!   the announced call here; `room_full` (409) and `bad_token` (403) are
//!   told as errors and the call stays announced.
//!
//! The lock on the state is held across calls into the engine and the
//! groups: neither calls back (events come on channels).

use crate::call::Outcome;
use crate::engine::{
    DataPayload, IceServer, Media, MediaEngine, PushedFrame, RelayPolicy, RoomConfig, SdpKind, Session, SessionEvent, VideoFrame,
    VideoInput, VideoSettings, VideoTrack, CTL_LABEL,
};
use crate::group::access::GroupAccess;
use crate::group::ctl::{self, Message, Track};
use crate::group::feed::GroupFeed;
use crate::group::keys::{self, Hello, HelloError};
use crate::group::signal::{new_secret, GroupSignal, Secret};
use crate::group::view::{
    AnnouncedCall, GroupCallView, GroupPhase, ParticipantView, UI_EVENT_GROUP_CALL_ENDED, UI_EVENT_GROUP_CALL_LEVEL,
    UI_EVENT_GROUP_CALL_STARTED, UI_EVENT_GROUP_CALL_STATE,
};
use crate::node_client::{ice_servers_of, MediaLimits, NodeAccess, NodeClient, NodeError, RoomApi, RoomCreated, CAP_SFU};
use crate::servers::{CallNode, ServerSets};
use crate::service::{VideoQuality, CONNECT_TIMEOUT, KEY_VIDEO_QUALITY, VIDEO_FPS};
use crate::signal::{new_call_id, reason};
use messenger_core::traits::UiEvent;
use messenger_core::{Clock, Effect, Envelope, MessengerError, PubKey, Result};
use messenger_dm::DmService;
use messenger_store::{settings, Store};
use nostr::key::Keys;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, Mutex};

/// Calls announced lately that are over, kept so that a late note of
/// one changes nothing.
const ANNOUNCED_KEPT: usize = 128;
/// Notes of a call that came before its start (the relays keep no order).
const EARLY_KEPT: usize = 64;
/// The word a client asks a node for, in a layer request; the node says
/// it in `capabilities` when it has it.
const CAP_SIMULCAST: &str = "simulcast";
/// A seat that has not said who it is this long after the node spoke of
/// it is nobody for good: the creator puts it out of the room; without
/// the creator, the oldest verified seat turns the keys.
pub const VERIFY_DEADLINE: Duration = Duration::from_secs(15);
/// A new epoch is sent with this much after it is learned: the others
/// have read the note and keyed my m-lines for it by then, and hear me
/// throughout.
pub const SEND_SWITCH_DELAY: Duration = Duration::from_millis(1500);
/// Two starts of one group written within this many seconds of each other
/// are one call: neither creator knew of the other.
pub const GLARE_WINDOW_SECS: i64 = 60;
/// How many of the latest epochs every verified seat's m-lines are keyed
/// for: a frame of an older epoch is of a sender long gone.
const EPOCHS_KEYED: usize = 8;
/// Seats named by `call.leave` kept per call, at most.
const LEFT_KEPT: usize = 1024;
/// The cameras of a phone, where the engine lists none (its plugin holds
/// the camera and pushes its frames): the same words as in a call between
/// two.
const PHONE_CAMERAS: [&str; 2] = ["front", "back"];

/// The m-line `video_frames` gives my own video for: the frames of what
/// I send (the camera, the screen, the frames a phone pushed), for the
/// tile of "me" on the screen. Not a mid of the node: `@` is no token
/// character of an SDP mid, so no seat ever has it.
pub const MY_VIDEO_MID: &str = "@me";

/// The delays of a room; the tests make them short.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    pub verify_deadline: Duration,
    pub send_switch_delay: Duration,
    /// How long the way to the node may take to come at all
    /// ([`CONNECT_TIMEOUT`] of a call between two).
    pub connect_timeout: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self { verify_deadline: VERIFY_DEADLINE, send_switch_delay: SEND_SWITCH_DELAY, connect_timeout: CONNECT_TIMEOUT }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Timer {
    Connect,
    /// The seat has had its time to say who it is.
    Verify { seat: u32 },
    /// Move my sending to the epoch.
    Switch { epoch: u32 },
}

/// Whom to tell on the way out of a room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tell {
    /// The node (my seat freed) and the group (`call.leave`, and
    /// `call.end` when I was the last).
    All,
    /// The node alone: the group is not mine to tell any more, or the
    /// call was never its.
    Node,
    /// Nobody: the node closed my seat itself, or the session is gone.
    Nobody,
}

/// A call as the group announced it.
struct Announced {
    call_id: String,
    group_id: String,
    room_id: String,
    node: CallNode,
    join_token: String,
    media: Media,
    started_by: PubKey,
    started_at: i64,
    /// When its start was written, by its author's clock: what two starts
    /// of one moment are ordered by, the same on every device.
    said_at: i64,
    /// When the room ends on the node (0: unknown; the lifetime of a room
    /// of the protocol is taken then).
    expires_at: i64,
    /// The secrets of the epochs, by epoch.
    epochs: BTreeMap<u32, Secret>,
    /// Who said they sit where (`call.join`), or proved it in the room.
    seats: BTreeMap<u32, PubKey>,
    /// The seats whose `call.leave` came, with who left them (the node
    /// gives a seat once): a `call.join` of one that comes later is stale.
    left: BTreeSet<(u32, String)>,
    /// The seats this device itself took, by its own start or join: my
    /// `call.join` on one of them, come back when I sit there no more, is
    /// this device's own echo, not a word of another device of mine.
    my_seats: BTreeSet<u32>,
    /// When I last left the room, if I was in it: the end of the call for
    /// the record when no `call.end` comes.
    left_at: Option<i64>,
    ended: bool,
}

/// A room of the protocol lives this long at most (`room_lifetime_secs`
/// of the node by default), for a start that does not say.
const ROOM_LIFETIME_SECS: i64 = 12 * 3600;

impl Announced {
    fn current_epoch(&self) -> u32 {
        self.epochs.keys().next_back().copied().unwrap_or(1)
    }

    fn expires(&self) -> i64 {
        if self.expires_at > 0 {
            self.expires_at
        } else {
            self.started_at + ROOM_LIFETIME_SECS
        }
    }

    fn live(&self, now: i64) -> bool {
        !self.ended && now < self.expires()
    }

    fn view(&self, joined: bool) -> AnnouncedCall {
        AnnouncedCall {
            call_id: self.call_id.clone(),
            group_id: self.group_id.clone(),
            chat_id: messenger_store::groups::group_chat_id(&self.group_id),
            media: self.media,
            started_by: self.started_by.as_hex().to_string(),
            started_at: self.started_at,
            participants: self.seats.values().map(|p| p.as_hex().to_string()).collect(),
            joined,
        }
    }
}

/// A seat of the room I am in, other than mine.
#[derive(Default)]
struct Peer {
    npub: Option<PubKey>,
    verified: bool,
    speaking: bool,
    /// The m-lines of its streams, as the node's offers named them.
    mids: Vec<(String, Media)>,
    /// Its word of identity under a secret I do not have yet (the note of
    /// its epoch is on its way): opened when the note comes.
    pending_hello: Option<Vec<u8>>,
    /// Was a member and is one no more: its keys are spoiled, it is
    /// nobody until it proves itself a member again.
    expelled: bool,
}

struct Room {
    gen: u64,
    view: GroupCallView,
    node: CallNode,
    room_id: String,
    session: Option<Box<dyn Session>>,
    seat: u32,
    participant_token: String,
    /// The creator's: changes the token, removes a seat.
    admin_token: Option<String>,
    dtls_fp: String,
    peers: BTreeMap<u32, Peer>,
    /// The node said the way is there.
    connected: bool,
    /// The way was there at least once: a loss after that is the engine's
    /// to mend, not the connect timer's to judge.
    ever_connected: bool,
    /// `leave` is under way: a `Closed` of the engine is no news.
    leaving: bool,
    /// The node has simulcast: a layer may be asked for.
    simulcast: bool,
    /// The epoch and the secret my frames go out under.
    sending: Option<(u32, Secret)>,
    /// The members of the group as last seen: who is gone when they change.
    members: Vec<PubKey>,
}

impl Room {
    fn new(gen: u64, view: GroupCallView, node: CallNode, room_id: String, members: Vec<PubKey>) -> Self {
        Self {
            gen,
            view,
            node,
            room_id,
            session: None,
            seat: 0,
            participant_token: String::new(),
            admin_token: None,
            dtls_fp: String::new(),
            peers: BTreeMap::new(),
            connected: false,
            ever_connected: false,
            leaving: false,
            simulcast: false,
            sending: None,
            members,
        }
    }
}

#[derive(Default)]
struct State {
    announced: HashMap<String, Announced>,
    /// Calls over, newest last.
    over: VecDeque<String>,
    early: VecDeque<(String, Vec<(PubKey, GroupSignal)>)>,
    room: Option<Room>,
    gen: u64,
}

impl State {
    fn next_gen(&mut self) -> u64 {
        self.gen += 1;
        self.gen
    }

    fn room_of(&mut self, call_id: &str) -> Option<&mut Room> {
        self.room.as_mut().filter(|r| r.view.call_id == call_id)
    }

    fn forget(&mut self, call_id: &str) {
        if self.over.len() == ANNOUNCED_KEPT {
            if let Some(old) = self.over.pop_front() {
                self.announced.remove(&old);
            }
        }
        self.over.push_back(call_id.to_string());
    }

    fn hold_early(&mut self, call_id: &str, author: PubKey, sig: GroupSignal) {
        if let Some((_, list)) = self.early.iter_mut().find(|(id, _)| id == call_id) {
            if list.len() < EARLY_KEPT {
                list.push((author, sig));
            }
            return;
        }
        if self.early.len() == EARLY_KEPT {
            self.early.pop_front();
        }
        self.early.push_back((call_id.to_string(), vec![(author, sig)]));
    }

    fn take_early(&mut self, call_id: &str) -> Vec<(PubKey, GroupSignal)> {
        match self.early.iter().position(|(id, _)| id == call_id) {
            Some(i) => self.early.remove(i).map(|(_, l)| l).unwrap_or_default(),
            None => vec![],
        }
    }

    /// The call of the group that is on now: the newest of the live ones
    /// (two of one moment are one call, the newer; see `GLARE_WINDOW_SECS`).
    fn live_call(&self, group_id: &str, now: i64) -> Option<&Announced> {
        self.announced.values().filter(|a| a.group_id == group_id && a.live(now)).max_by_key(|a| (a.said_at, a.call_id.clone()))
    }
}

struct Inner {
    store: Store,
    feed: GroupFeed,
    groups: Arc<dyn GroupAccess>,
    engine: Arc<dyn MediaEngine>,
    servers: Arc<dyn ServerSets>,
    nodes: NodeClient,
    rooms: Arc<dyn RoomApi>,
    clock: Arc<dyn Clock>,
    signer: RwLock<Option<Keys>>,
    timing: RwLock<Timing>,
    state: Mutex<State>,
    outlet: mpsc::UnboundedSender<Effect>,
}

#[derive(Clone)]
pub struct GroupCallService {
    inner: Arc<Inner>,
}

impl GroupCallService {
    /// The service and the outlet of its effects: `Send` to publish (a
    /// scoped event of the group), `Emit` to show.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: Store,
        dm: DmService,
        groups: Arc<dyn GroupAccess>,
        engine: Arc<dyn MediaEngine>,
        servers: Arc<dyn ServerSets>,
        nodes: NodeClient,
        rooms: Arc<dyn RoomApi>,
        clock: Arc<dyn Clock>,
    ) -> (Self, mpsc::UnboundedReceiver<Effect>) {
        let (outlet, rx) = mpsc::unbounded_channel();
        let feed = GroupFeed::new(store.clone(), dm);
        let inner = Inner {
            store,
            feed,
            groups,
            engine,
            servers,
            nodes,
            rooms,
            clock,
            signer: RwLock::new(None),
            timing: RwLock::new(Timing::default()),
            state: Mutex::new(State::default()),
            outlet,
        };
        (Self { inner: Arc::new(inner) }, rx)
    }

    /// The delays of a room (the tests make them short).
    pub fn set_timing(&self, timing: Timing) {
        *self.inner.timing.write().unwrap() = timing;
    }

    /// The keys of the session; `None` on logout: the room is left. With
    /// keys, the records of calls whose end never came (this device was
    /// off) are closed as of when their rooms expired.
    pub fn set_signer(&self, keys: Option<Keys>) {
        let gone = keys.is_none();
        *self.inner.signer.write().unwrap() = keys;
        let inner = self.inner.clone();
        if gone {
            tokio::spawn(async move {
                let mut st = inner.state.lock().await;
                inner.leave_room(&mut st, Outcome::Failed, Tell::Nobody).await;
            });
        } else {
            tokio::spawn(async move {
                let now = inner.clock.now().secs();
                match inner.feed.close_stale(now, ROOM_LIFETIME_SECS).await {
                    Ok(fx) => inner.emit(fx),
                    Err(e) => inner.emit(vec![error_event(&e)]),
                }
            });
        }
    }

    /// The room I am in, as last shown.
    pub async fn current(&self) -> Option<GroupCallView> {
        self.inner.state.lock().await.room.as_ref().map(|r| r.view.clone())
    }

    /// The call announced in `group_id` that is on now, for the banner.
    pub async fn announced(&self, group_id: &str) -> Option<AnnouncedCall> {
        let now = self.inner.clock.now().secs();
        let st = self.inner.state.lock().await;
        let joined = |a: &Announced| st.room.as_ref().is_some_and(|r| r.view.call_id == a.call_id && r.view.phase != GroupPhase::Left);
        st.live_call(group_id, now).map(|a| a.view(joined(a)))
    }

    /// Start a call in the group: a room on a node, me in it, the group told.
    pub async fn start(&self, group_id: &str, media: Media) -> Result<GroupCallView> {
        let inner = &self.inner;
        let keys = inner.signer()?;
        let me = PubKey::parse(&keys.public_key().to_hex()).expect("a key is hex");
        let members = inner.groups.members(group_id).await?;
        if !members.contains(&me) {
            return Err(MessengerError::Invalid("not a member of the group".into()));
        }
        let now = inner.clock.now().secs();
        let call_id = new_call_id();
        let gen = {
            let mut st = inner.state.lock().await;
            if st.room.is_some() {
                return Err(MessengerError::Invalid("a group call is under way".into()));
            }
            if st.live_call(group_id, now).is_some() {
                return Err(MessengerError::Invalid("a call is on in this group already: join it".into()));
            }
            let gen = st.next_gen();
            let view = GroupCallView {
                call_id: call_id.clone(),
                group_id: group_id.to_string(),
                chat_id: messenger_store::groups::group_chat_id(group_id),
                phase: GroupPhase::Starting,
                media,
                muted: false,
                video_local: false,
                camera: None,
                started_by: me.as_hex().to_string(),
                started_at: now,
                joined_at: None,
                node: String::new(),
                participant: None,
                epoch: 1,
                participants: vec![],
                limits: None,
                kbps_per_participant: 0,
                max_participants: 0,
            };
            let placeholder =
                CallNode::new(format!("0.0.0.0:1#{}", "00".repeat(32)).parse().expect("a placeholder"), crate::servers::NodeClass::Own);
            st.room = Some(Room::new(gen, view.clone(), placeholder, String::new(), members));
            inner.emit(vec![state_event(&view)]);
            gen
        };
        // The node and the room are asked without the lock.
        let outcome: Result<(NodeAccess, RoomCreated)> = async {
            let access = inner.sfu_node(group_id, now).await?;
            let created = inner.rooms.create(&access.node, MediaLimits::default()).await?;
            Ok((access, created))
        }
        .await;
        let mut st = inner.state.lock().await;
        if st.room.as_ref().is_none_or(|r| r.gen != gen) {
            return Err(MessengerError::Invalid("the call ended".into()));
        }
        let (access, created) = match outcome {
            Ok(x) => x,
            Err(e) => {
                inner.leave_room(&mut st, Outcome::Failed, Tell::Nobody).await;
                return Err(e);
            }
        };
        let secret = new_secret();
        let node = access.node.clone();
        st.announced.insert(
            call_id.clone(),
            Announced {
                call_id: call_id.clone(),
                group_id: group_id.to_string(),
                room_id: created.room_id.clone(),
                node: node.clone(),
                join_token: created.join_token.clone(),
                media,
                started_by: me.clone(),
                started_at: now,
                said_at: now,
                expires_at: created.expires_at as i64,
                epochs: BTreeMap::from([(1, secret)]),
                seats: BTreeMap::new(),
                left: BTreeSet::new(),
                my_seats: BTreeSet::new(),
                left_at: None,
                ended: false,
            },
        );
        inner.schedule_expiry(call_id.clone(), created.expires_at as i64);
        {
            let room = st.room.as_mut().expect("checked above");
            room.node = node.clone();
            room.room_id = created.room_id.clone();
            room.admin_token = Some(created.admin_token.clone());
            room.view.node = node.node.to_string();
            room.view.limits = Some(access.welcome.limits.clone());
            room.view.kbps_per_participant = created.kbps_per_participant;
            room.view.max_participants = created.max_participants;
            room.simulcast = access.welcome.capabilities.iter().any(|c| c == CAP_SIMULCAST);
        }
        match inner.feed.begin(&call_id, group_id, me.as_hex(), true, media, now).await {
            Ok(fx) => inner.emit(fx),
            Err(e) => inner.emit(vec![error_event(&e)]),
        }
        if let Err(e) = inner.enter_room(&mut st, &keys, &call_id, &access, &created.join_token, &secret, 1).await {
            // Nobody was told of it: a call that failed, on record, off
            // the banner, and the group free for another.
            inner.leave_room(&mut st, Outcome::Failed, Tell::Nobody).await;
            inner.end_announced(&mut st, &call_id, Outcome::Failed, inner.clock.now().secs()).await;
            return Err(e);
        }
        // A start of the group that came while the node was asked, within
        // moments of mine: one call, settled here as on every other
        // device, by when each start was said (mine is said now, as its
        // note will say) and the call id. The newer holds. When it is not
        // mine, my room never was: out of it (the node told; the group
        // never heard of it), and into the room that holds.
        let said = inner.clock.now().secs();
        if let Some(a) = st.announced.get_mut(&call_id) {
            a.said_at = said;
        }
        let rival = st
            .announced
            .values()
            .filter(|a| a.call_id != call_id && a.group_id == group_id && a.live(said) && (a.said_at - said).abs() <= GLARE_WINDOW_SECS)
            .map(|a| (a.said_at, a.call_id.clone()))
            .max();
        if let Some((rival_at, rival_id)) = rival {
            if (rival_at, rival_id.as_str()) > (said, call_id.as_str()) {
                inner.drop_loser(&mut st, &call_id).await;
                drop(st);
                return self.join(group_id).await;
            }
            inner.supersede(&mut st, &rival_id).await;
        }
        let start = GroupSignal::Start {
            call_id: call_id.clone(),
            room_id: created.room_id,
            node: node.node.clone(),
            key: node.access_key.clone(),
            join_token: created.join_token,
            secret,
            media,
            expires_at: created.expires_at as i64,
        };
        inner.tell_group(group_id, &start).await;
        let seat = st.room.as_ref().map(|r| r.seat).unwrap_or(0);
        inner.tell_group(group_id, &GroupSignal::Join { call_id: call_id.clone(), participant: seat }).await;
        if let Some(a) = st.announced.get_mut(&call_id) {
            a.seats.insert(seat, me);
            a.my_seats.insert(seat);
        }
        if let Err(e) = inner.feed.took_seat(&call_id, seat).await {
            inner.emit(vec![error_event(&e)]);
        }
        inner.emit_announced(&st, &call_id);
        Ok(st.room.as_ref().expect("in the room").view.clone())
    }

    /// Join the call that is on in the group.
    pub async fn join(&self, group_id: &str) -> Result<GroupCallView> {
        let inner = &self.inner;
        let keys = inner.signer()?;
        let me = PubKey::parse(&keys.public_key().to_hex()).expect("a key is hex");
        let members = inner.groups.members(group_id).await?;
        if !members.contains(&me) {
            return Err(MessengerError::Invalid("not a member of the group".into()));
        }
        let now = inner.clock.now().secs();
        let (gen, call_id, node, token, secret, epoch) = {
            let mut st = inner.state.lock().await;
            if st.room.is_some() {
                return Err(MessengerError::Invalid("a group call is under way".into()));
            }
            let Some(a) = st.live_call(group_id, now) else {
                return Err(MessengerError::Invalid("no call is on in this group".into()));
            };
            let epoch = a.current_epoch();
            let secret = *a.epochs.get(&epoch).expect("the current epoch has its secret");
            let (call_id, node, token, media, started_by, started_at, room_id) =
                (a.call_id.clone(), a.node.clone(), a.join_token.clone(), a.media, a.started_by.clone(), a.started_at, a.room_id.clone());
            let gen = st.next_gen();
            let view = GroupCallView {
                call_id: call_id.clone(),
                group_id: group_id.to_string(),
                chat_id: messenger_store::groups::group_chat_id(group_id),
                phase: GroupPhase::Joining,
                media,
                muted: false,
                video_local: false,
                camera: None,
                started_by: started_by.as_hex().to_string(),
                started_at,
                joined_at: None,
                node: node.node.to_string(),
                participant: None,
                epoch,
                participants: vec![],
                limits: None,
                kbps_per_participant: 0,
                max_participants: 0,
            };
            st.room = Some(Room::new(gen, view.clone(), node.clone(), room_id, members));
            inner.emit(vec![state_event(&view)]);
            (gen, call_id, node, token, secret, epoch)
        };
        // The credentials of the node (its TURN, for a way to its SFU
        // through a hard NAT) are asked without the lock; a node that
        // gives none still has its SFU.
        let access = inner.nodes.access(&node, now).await;
        let mut st = inner.state.lock().await;
        if st.room.as_ref().is_none_or(|r| r.gen != gen) {
            return Err(MessengerError::Invalid("the call ended".into()));
        }
        let access = match access {
            Ok(a) => a,
            Err(e) => {
                inner.leave_room(&mut st, Outcome::Failed, Tell::Nobody).await;
                return Err(e);
            }
        };
        if let Some(room) = st.room.as_mut() {
            room.view.limits = Some(access.welcome.limits.clone());
            room.simulcast = access.welcome.capabilities.iter().any(|c| c == CAP_SIMULCAST);
        }
        if let Err(e) = inner.enter_room(&mut st, &keys, &call_id, &access, &token, &secret, epoch).await {
            let not_found = matches!(&e, MessengerError::Transport(t) if t.contains("room_not_found"));
            inner.leave_room(&mut st, Outcome::Failed, Tell::Nobody).await;
            if not_found {
                // The node has no such room: the call is over, whatever
                // the notes say.
                let now = inner.clock.now().secs();
                inner.end_announced(&mut st, &call_id, Outcome::Ended, now).await;
            }
            return Err(e);
        }
        let seat = st.room.as_ref().map(|r| r.seat).unwrap_or(0);
        inner.tell_group(group_id, &GroupSignal::Join { call_id: call_id.clone(), participant: seat }).await;
        if let Some(a) = st.announced.get_mut(&call_id) {
            a.seats.insert(seat, me);
            a.my_seats.insert(seat);
        }
        if let Err(e) = inner.feed.took_seat(&call_id, seat).await {
            inner.emit(vec![error_event(&e)]);
        }
        inner.emit_announced(&st, &call_id);
        Ok(st.room.as_ref().expect("in the room").view.clone())
    }

    /// Leave the room. The last one out ends the call for the group.
    pub async fn leave(&self) -> Result<()> {
        let inner = &self.inner;
        inner.signer()?;
        let mut st = inner.state.lock().await;
        if st.room.is_none() {
            return Err(MessengerError::Invalid("no group call".into()));
        }
        inner.leave_room(&mut st, Outcome::Ended, Tell::All).await;
        Ok(())
    }

    pub async fn set_mute(&self, muted: bool) -> Result<GroupCallView> {
        let inner = &self.inner;
        let mut st = inner.state.lock().await;
        let Some(room) = st.room.as_mut() else {
            return Err(MessengerError::Invalid("no group call".into()));
        };
        if let Some(s) = room.session.as_ref() {
            s.set_mute(muted).await?;
        }
        room.view.muted = muted;
        let view = room.view.clone();
        inner.emit(vec![state_event(&view)]);
        Ok(view)
    }

    /// My video in the room: the camera, the screen, or off.
    pub async fn set_video(&self, input: VideoInput) -> Result<GroupCallView> {
        let inner = &self.inner;
        let quality: VideoQuality =
            settings::get(&inner.store, KEY_VIDEO_QUALITY).await?.as_deref().and_then(VideoQuality::parse).unwrap_or_default();
        let mut st = inner.state.lock().await;
        let Some(room) = st.room.as_mut() else {
            return Err(MessengerError::Invalid("no group call".into()));
        };
        let Some(session) = room.session.as_ref() else {
            return Err(MessengerError::Invalid("the call has no media yet".into()));
        };
        let (width, height, kbps) = quality.profile();
        let cap = match room.view.kbps_per_participant {
            0 => kbps,
            node => kbps.min(node),
        };
        let settings = VideoSettings { width, height, fps: VIDEO_FPS, max_kbps: Some(cap) };
        let outcome = session.set_video(input.clone(), settings).await;
        room.view.video_local = outcome.is_ok() && input != VideoInput::Off;
        if let (Ok(()), VideoInput::Camera { id: Some(id) }) = (&outcome, &input) {
            room.view.camera = Some(id.clone());
        }
        let view = room.view.clone();
        inner.emit(vec![state_event(&view)]);
        outcome.map(|()| view)
    }

    /// The next camera of the engine's list (or the one `id` names):
    /// switched at once when my camera is on, kept for when it goes on
    /// otherwise. On a phone the list is `front`, `back` (the engine
    /// lists none: the plugin holds the camera and pushes its frames).
    pub async fn switch_camera(&self, id: Option<String>) -> Result<GroupCallView> {
        let inner = &self.inner;
        let mut ids: Vec<String> = inner.engine.cameras().await.into_iter().map(|c| c.id).collect();
        if ids.is_empty() {
            ids = PHONE_CAMERAS.iter().map(|s| s.to_string()).collect();
        }
        let (on, current) = {
            let st = inner.state.lock().await;
            let Some(room) = st.room.as_ref() else {
                return Err(MessengerError::Invalid("no group call".into()));
            };
            (room.view.video_local, room.view.camera.clone())
        };
        let next = match id {
            Some(id) => id,
            None => {
                // The one after the current in the list, around the end;
                // the second when none was chosen yet (the first is in use).
                let at = current.as_ref().and_then(|c| ids.iter().position(|i| i == c)).unwrap_or(0);
                ids[(at + 1) % ids.len()].clone()
            }
        };
        if on {
            return self.set_video(VideoInput::Camera { id: Some(next) }).await;
        }
        let mut st = inner.state.lock().await;
        let Some(room) = st.room.as_mut() else {
            return Err(MessengerError::Invalid("no group call".into()));
        };
        room.view.camera = Some(next);
        let view = room.view.clone();
        inner.emit(vec![state_event(&view)]);
        Ok(view)
    }

    /// A frame the platform captured (the camera of a phone, through the
    /// plugin), as my video in the room. Refused without a room or
    /// before its media is there; the engine converts and sends it when
    /// my video is on, and drops it otherwise.
    pub async fn push_video_frame(&self, frame: PushedFrame) -> Result<()> {
        let st = self.inner.state.lock().await;
        let session = st.room.as_ref().and_then(|r| r.session.as_ref()).ok_or_else(|| MessengerError::Invalid("no group call".into()))?;
        session.push_video_frame(frame)
    }

    /// Ask the node for a layer of a seat's video (`rid` of its simulcast):
    /// only where the node has simulcast; a node without it takes no
    /// such word, and this is an error.
    pub async fn set_layer(&self, participant: u32, rid: &str) -> Result<()> {
        let inner = &self.inner;
        let st = inner.state.lock().await;
        let Some(room) = st.room.as_ref() else {
            return Err(MessengerError::Invalid("no group call".into()));
        };
        if !room.simulcast {
            return Err(MessengerError::Invalid("the node has no simulcast".into()));
        }
        let Some(session) = room.session.as_ref() else {
            return Err(MessengerError::Invalid("the call has no media yet".into()));
        };
        let text = serde_json::json!({ "t": "layer", "participant": participant, "rid": rid }).to_string();
        session.send_data(CTL_LABEL, DataPayload::Text(text)).await
    }

    /// The frames of the video of the seat whose m-line is `mid`; for
    /// [`MY_VIDEO_MID`], the frames of what I send (the camera, the
    /// screen, the frames a phone pushed), for the tile of "me".
    pub async fn video_frames(&self, mid: &str) -> Option<broadcast::Receiver<Arc<VideoFrame>>> {
        let st = self.inner.state.lock().await;
        let session = st.room.as_ref()?.session.as_ref()?;
        if mid == MY_VIDEO_MID {
            session.video_frames(VideoTrack::Local)
        } else {
            session.video_frames_of(mid)
        }
    }

    /// A `call.*` note of a group (the groups' sink calls this): from
    /// `author`, a member at the time it was written, as the groups
    /// checked. My own copies come back too and change nothing. The
    /// members of the group are looked at on the way: whoever is gone
    /// from them is gone from the room.
    pub async fn on_group_note(&self, group_id: &str, author: &PubKey, envelope: &Envelope, at: i64) -> Result<()> {
        let inner = &self.inner;
        let Ok(keys) = inner.signer() else { return Ok(()) };
        let me = PubKey::parse(&keys.public_key().to_hex()).expect("a key is hex");
        let Some(sig) = GroupSignal::parse(envelope) else { return Ok(()) };
        // The groups let through what a member wrote; asked again here, so
        // that a word of somebody who is not one changes nothing whatever
        // door it came by.
        let members = match inner.groups.members(group_id).await {
            Ok(m) => m,
            Err(e) => {
                let mut st = inner.state.lock().await;
                inner.out_of_group(&mut st, group_id).await;
                return Err(e);
            }
        };
        let mut st = inner.state.lock().await;
        inner.check_members(&mut st, group_id, &members).await;
        if !members.contains(author) {
            return Ok(());
        }
        inner.on_signal(&mut st, group_id, author, &me, sig, at).await
    }

    /// The members of the group changed (the groups say so): whoever of
    /// my room is a member no more is nobody from here on, the creator
    /// puts it out and changes the token of the room, the keys turn; out
    /// of the room myself when I am the one gone.
    pub async fn on_members_changed(&self, group_id: &str) {
        let inner = &self.inner;
        if inner.signer().is_err() {
            return;
        }
        match inner.groups.members(group_id).await {
            Ok(members) => {
                let mut st = inner.state.lock().await;
                inner.check_members(&mut st, group_id, &members).await;
            }
            Err(_) => {
                let mut st = inner.state.lock().await;
                inner.out_of_group(&mut st, group_id).await;
            }
        }
    }
}

fn state_event(view: &GroupCallView) -> Effect {
    Effect::Emit(UiEvent { name: UI_EVENT_GROUP_CALL_STATE.into(), payload: serde_json::json!({ "call": view }) })
}

fn error_event(e: &MessengerError) -> Effect {
    Effect::Emit(UiEvent { name: "error".into(), payload: serde_json::json!({ "scope": "group_calls", "error": e }) })
}

impl Room {
    fn participants(&self, me: &str) -> Vec<ParticipantView> {
        let mut out = vec![ParticipantView {
            id: self.seat,
            npub: Some(me.to_string()),
            verified: true,
            speaking: false,
            audio: true,
            audio_mid: None,
            video_mid: None,
            me: true,
        }];
        for (id, p) in &self.peers {
            out.push(ParticipantView {
                id: *id,
                npub: p.npub.as_ref().map(|n| n.as_hex().to_string()).filter(|_| p.verified),
                verified: p.verified,
                speaking: p.speaking,
                audio: p.mids.iter().any(|(_, k)| *k == Media::Audio),
                audio_mid: p.mids.iter().find(|(_, k)| *k == Media::Audio).map(|(m, _)| m.clone()),
                video_mid: p.mids.iter().find(|(_, k)| *k == Media::Video).map(|(m, _)| m.clone()),
                me: false,
            });
        }
        out
    }

    fn refresh(&mut self, me: &str) {
        self.view.participants = self.participants(me);
        self.view.participant = Some(self.seat);
    }

    /// The smallest verified seat of the room is mine: I keep the epochs.
    /// A seat that has not said who it is does not count: it may be
    /// nobody's, and nobody's seat never rotates.
    fn oldest(&self) -> bool {
        self.peers.iter().find(|(_, p)| p.verified).is_none_or(|(first, _)| self.seat < *first)
    }
}

impl Inner {
    fn signer(&self) -> Result<Keys> {
        self.signer.read().unwrap().clone().ok_or(MessengerError::NotLoggedIn)
    }

    fn timing(&self) -> Timing {
        *self.timing.read().unwrap()
    }

    fn emit(&self, effects: Vec<Effect>) {
        for e in effects {
            let _ = self.outlet.send(e);
        }
    }

    fn me(&self) -> Option<String> {
        self.signer().ok().map(|k| k.public_key().to_hex())
    }

    fn schedule(self: &Arc<Self>, after: Duration, timer: Timer, gen: u64) {
        let inner = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(after).await;
            inner.on_timer(timer, gen).await;
        });
    }

    /// When the room of the announced call has expired on the node, the
    /// call is over here too, whatever notes did not come: on record as
    /// of when I last left it, or when the room ended.
    fn schedule_expiry(self: &Arc<Self>, call_id: String, expires: i64) {
        let now = self.clock.now().secs();
        let after = Duration::from_secs(u64::try_from(expires - now).unwrap_or(0)) + Duration::from_millis(200);
        let inner = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(after).await;
            let mut st = inner.state.lock().await;
            let now = inner.clock.now().secs();
            let Some(a) = st.announced.get(&call_id) else { return };
            if a.ended || a.live(now) {
                return;
            }
            let ended_at = a.left_at.unwrap_or_else(|| a.expires());
            inner.end_announced(&mut st, &call_id, Outcome::Ended, ended_at).await;
        });
    }

    fn pump(self: &Arc<Self>, mut events: mpsc::Receiver<SessionEvent>, gen: u64) {
        let inner = self.clone();
        tokio::spawn(async move {
            while let Some(ev) = events.recv().await {
                inner.on_engine_event(gen, ev).await;
            }
        });
    }

    /// The node for a room of `group_id`: the one pinned to the group,
    /// else the first with an SFU from the sets of servers.
    async fn sfu_node(&self, group_id: &str, now: i64) -> Result<NodeAccess> {
        if let Some(pinned) = self.groups.pinned_node(group_id).await? {
            let access = self.nodes.access(&pinned, now).await?;
            if !access.welcome.capabilities.iter().any(|c| c == CAP_SFU) {
                return Err(MessengerError::Transport("the group's node has no SFU".into()));
            }
            return Ok(access);
        }
        let nodes = self.servers.call_nodes().await.unwrap_or_default();
        self.nodes.pick_sfu(&nodes, now).await.ok_or_else(|| MessengerError::Transport("no call node with an SFU answered".into()))
    }

    /// Into the room: the session, my one offer to the node, its answer,
    /// my seat, my sending key. The room is in `st` already.
    #[allow(clippy::too_many_arguments)]
    async fn enter_room(
        self: &Arc<Self>,
        st: &mut State,
        keys: &Keys,
        call_id: &str,
        access: &NodeAccess,
        token: &str,
        secret: &Secret,
        epoch: u32,
    ) -> Result<()> {
        let (gen, node, room_id, media) = {
            let room = st.room.as_ref().ok_or_else(|| MessengerError::Invalid("no room".into()))?;
            (room.gen, room.node.clone(), room.room_id.clone(), room.view.media)
        };
        let servers: Vec<IceServer> = ice_servers_of(&access.credentials);
        let simulcast = st.room.as_ref().is_some_and(|r| r.simulcast);
        let config = RoomConfig { call_id: call_id.to_string(), data_label: CTL_LABEL.into(), key_salt: call_id.as_bytes().to_vec(), simulcast };
        let session = self.engine.create_room_session(servers, RelayPolicy::Auto, media, config).await?;
        self.pump(session.events(), gen);
        let offer = match session.create_offer().await {
            Ok(o) => o,
            Err(e) => {
                session.close().await;
                return Err(e);
            }
        };
        let joined = match self.rooms.join(&node, &room_id, token, &offer).await {
            Ok(j) => j,
            Err(e) => {
                session.close().await;
                return Err(e.into());
            }
        };
        if let Err(e) = session.set_remote(&joined.sdp_answer, SdpKind::Answer).await {
            session.close().await;
            return Err(e);
        }
        let key = keys::sender_key(secret, call_id, joined.participant_id, epoch);
        if let Err(e) = session.set_sender_key(keys::slot(epoch), &key).await {
            session.close().await;
            return Err(e);
        }
        let now = self.clock.now().secs();
        let room = st.room.as_mut().ok_or_else(|| MessengerError::Invalid("no room".into()))?;
        room.session = Some(session);
        room.seat = joined.participant_id;
        room.participant_token = joined.participant_token;
        room.dtls_fp = keys::dtls_fingerprint(&offer);
        room.sending = Some((epoch, *secret));
        for id in joined.participants {
            self.seat_appeared(room, id);
        }
        room.view.phase = GroupPhase::Joining;
        room.view.joined_at = Some(now);
        room.view.epoch = epoch;
        room.refresh(&keys.public_key().to_hex());
        let view = room.view.clone();
        self.schedule(self.timing().connect_timeout, Timer::Connect, gen);
        if let Err(e) = self.feed.joined(call_id, now).await {
            self.emit(vec![error_event(&e)]);
        }
        self.count_people(st, call_id).await;
        self.emit(vec![state_event(&view)]);
        if media == Media::Video {
            let quality = settings::get(&self.store, KEY_VIDEO_QUALITY).await.ok().flatten().as_deref().and_then(VideoQuality::parse).unwrap_or_default();
            let (width, height, kbps) = quality.profile();
            if let Some(room) = st.room.as_mut() {
                let cap = if room.view.kbps_per_participant > 0 { kbps.min(room.view.kbps_per_participant) } else { kbps };
                let settings = VideoSettings { width, height, fps: VIDEO_FPS, max_kbps: Some(cap) };
                if let Some(s) = room.session.as_ref() {
                    match s.set_video(VideoInput::Camera { id: room.view.camera.clone() }, settings).await {
                        Ok(()) => room.view.video_local = true,
                        Err(e) => self.emit(vec![error_event(&e)]),
                    }
                }
            }
        }
        Ok(())
    }

    /// A seat the node spoke of is in my room: nobody until its word of
    /// identity comes, and its time to say it runs from now.
    fn seat_appeared(self: &Arc<Self>, room: &mut Room, seat: u32) {
        if seat == room.seat || room.peers.contains_key(&seat) {
            return;
        }
        room.peers.insert(seat, Peer::default());
        self.schedule(self.timing().verify_deadline, Timer::Verify { seat }, room.gen);
    }

    /// How many people the record of the call has seen: the seats that
    /// said they are in, and me.
    async fn count_people(&self, st: &State, call_id: &str) {
        let Some(a) = st.announced.get(call_id) else { return };
        let mut people: Vec<&str> = a.seats.values().map(|p| p.as_hex()).collect();
        people.sort_unstable();
        people.dedup();
        let mut n = people.len();
        if let Some(me) = self.me() {
            if !people.iter().any(|p| *p == me) && st.room.as_ref().is_some_and(|r| r.view.call_id == call_id) {
                n += 1;
            }
        }
        match self.feed.participants(call_id, n).await {
            Ok(fx) => self.emit(fx),
            Err(e) => self.emit(vec![error_event(&e)]),
        }
    }

    /// A note to the group, sealed by the groups.
    async fn tell_group(&self, group_id: &str, signal: &GroupSignal) {
        match self.groups.seal_note(group_id, &signal.to_envelope()).await {
            Ok(out) => self.emit(vec![Effect::Send(out)]),
            Err(e) => self.emit(vec![error_event(&e)]),
        }
    }

    fn emit_announced(&self, st: &State, call_id: &str) {
        let Some(a) = st.announced.get(call_id) else { return };
        let joined = st.room.as_ref().is_some_and(|r| r.view.call_id == call_id && r.view.phase != GroupPhase::Left);
        self.emit(vec![Effect::Emit(UiEvent {
            name: UI_EVENT_GROUP_CALL_STARTED.into(),
            payload: serde_json::json!({ "call": a.view(joined) }),
        })]);
    }

    /// My word of identity on the control channel, under the newest epoch
    /// known (whoever lacks its secret keeps the word until the note of
    /// the epoch comes).
    async fn say_hello(&self, st: &mut State) {
        let Ok(keys) = self.signer() else { return };
        let Some(room) = st.room.as_ref() else { return };
        let Some(session) = room.session.as_ref() else { return };
        let Some(a) = st.announced.get(&room.view.call_id) else { return };
        let epoch = a.current_epoch();
        let Some(secret) = a.epochs.get(&epoch) else { return };
        let hello = Hello {
            npub: keys::npub_of(&keys),
            participant: room.seat,
            call_id: room.view.call_id.clone(),
            room_id: room.room_id.clone(),
            epoch,
            dtls_fp: room.dtls_fp.clone(),
        };
        let frame = keys::seal_hello(&keys, &hello, secret);
        if let Err(e) = session.send_data(CTL_LABEL, DataPayload::Binary(frame)).await {
            self.emit(vec![error_event(&e)]);
        }
    }

    /// The keys of every verified seat's m-lines for the latest epochs
    /// known, and mine again when the secret of the epoch I send with was
    /// replaced (two rotations met on one number). Called when a seat is
    /// verified, an m-line of it appears, or an epoch comes. Moving my
    /// sending to a new epoch is [`Self::switch_sending`]'s, on its timer.
    async fn key_everything(&self, st: &mut State) {
        let Some(room) = st.room.as_mut() else { return };
        let Some(session) = room.session.as_ref() else { return };
        let Some(a) = st.announced.get(&room.view.call_id) else { return };
        let call_id = room.view.call_id.clone();
        if let Some((epoch, secret)) = room.sending {
            if let Some(held) = a.epochs.get(&epoch) {
                if *held != secret {
                    let key = keys::sender_key(held, &call_id, room.seat, epoch);
                    if session.set_sender_key(keys::slot(epoch), &key).await.is_ok() {
                        room.sending = Some((epoch, *held));
                    }
                }
            }
        }
        let epochs: Vec<(u32, Secret)> = a.epochs.iter().rev().take(EPOCHS_KEYED).map(|(e, s)| (*e, *s)).collect();
        for (seat, peer) in room.peers.iter().filter(|(_, p)| p.verified) {
            for (epoch, secret) in epochs.iter().rev() {
                let key = keys::sender_key(secret, &call_id, *seat, *epoch);
                for (mid, _) in &peer.mids {
                    let _ = session.set_receiver_key(mid, keys::slot(*epoch), &key).await;
                }
            }
        }
    }

    /// My sending moves to `epoch`, when it is still the newest: the
    /// others have had [`SEND_SWITCH_DELAY`] to key my m-lines for it.
    async fn switch_sending(&self, st: &mut State, epoch: u32) {
        let Some(room) = st.room.as_mut() else { return };
        let Some(session) = room.session.as_ref() else { return };
        let Some(a) = st.announced.get(&room.view.call_id) else { return };
        if a.current_epoch() != epoch {
            return;
        }
        let Some(secret) = a.epochs.get(&epoch) else { return };
        if room.sending == Some((epoch, *secret)) {
            return;
        }
        let key = keys::sender_key(secret, &room.view.call_id, room.seat, epoch);
        if session.set_sender_key(keys::slot(epoch), &key).await.is_ok() {
            room.sending = Some((epoch, *secret));
            room.view.epoch = epoch;
            let view = room.view.clone();
            self.emit(vec![state_event(&view)]);
        }
    }

    /// A new epoch, told to the group: when a seat left or never said who
    /// it is, by the oldest verified seat; when the group lost a member,
    /// by the creator too (`force`), with the room's new token.
    async fn rotate(self: &Arc<Self>, st: &mut State, join_token: Option<String>, force: bool) {
        let Some(room) = st.room.as_ref() else { return };
        if room.view.phase == GroupPhase::Left || (!force && !room.oldest()) {
            return;
        }
        let (gen, call_id, group_id) = (room.gen, room.view.call_id.clone(), room.view.group_id.clone());
        let Some(a) = st.announced.get_mut(&call_id) else { return };
        let epoch = a.current_epoch() + 1;
        let secret = new_secret();
        a.epochs.insert(epoch, secret);
        self.tell_group(&group_id, &GroupSignal::Epoch { call_id, epoch, secret, join_token }).await;
        self.key_everything(st).await;
        self.schedule(self.timing().send_switch_delay, Timer::Switch { epoch }, gen);
        self.say_hello(st).await;
    }

    async fn on_signal(self: &Arc<Self>, st: &mut State, group_id: &str, author: &PubKey, me: &PubKey, sig: GroupSignal, at: i64) -> Result<()> {
        let call_id = sig.call_id().to_string();
        let now = self.clock.now().secs();
        if let GroupSignal::Start { room_id, node, key, join_token, secret, media, expires_at, .. } = sig {
            if st.announced.contains_key(&call_id) || st.over.contains(&call_id) {
                return Ok(());
            }
            // Two starts of one moment: the newer holds (by the authors'
            // own clocks, the same everywhere); the older never was. A
            // start of the moment that is over by now counts too: the
            // history comes in any order, and the older start, told of
            // after the newer has ended, would be a call nobody is in
            // (the losing room of a glare has no `call.end`).
            let rival = st
                .announced
                .values()
                .filter(|a| a.group_id == group_id && (a.said_at - at).abs() <= GLARE_WINDOW_SECS)
                .map(|a| (a.said_at, a.call_id.clone()))
                .max();
            if let Some((rival_at, rival_id)) = rival {
                if (rival_at, rival_id.as_str()) > (at, call_id.as_str()) {
                    st.forget(&call_id);
                    st.take_early(&call_id);
                    return Ok(());
                }
                self.supersede(st, &rival_id).await;
            }
            // Over on record already (I saw its end before: my own last
            // leave, the node's `room_not_found`, its expiry), and told
            // of again from the history after a restart: no live call,
            // whatever notes of its end did not come back. Kept with its
            // time all the same, so that an older start of its moment,
            // told of later, meets it.
            let over_on_record = self.feed.row(&call_id).await?.is_some_and(|r| r.outcome.is_some());
            // The seats this device took before (the app was restarted
            // in the room, or after it): its own `call.join` of them
            // is told from a word of another device of mine.
            let my_seats = if over_on_record { BTreeSet::new() } else { self.feed.my_seats(&call_id).await? };
            let node = CallNode { node, class: crate::servers::NodeClass::Group, access_key: key };
            let started_at = at.min(now);
            let a = Announced {
                call_id: call_id.clone(),
                group_id: group_id.to_string(),
                room_id,
                node,
                join_token,
                media,
                started_by: author.clone(),
                started_at,
                said_at: at,
                expires_at,
                epochs: BTreeMap::from([(1, secret)]),
                seats: BTreeMap::new(),
                left: BTreeSet::new(),
                my_seats,
                left_at: None,
                ended: over_on_record,
            };
            let (live, expires) = (a.live(now), a.expires());
            st.announced.insert(call_id.clone(), a);
            if over_on_record {
                st.forget(&call_id);
                st.take_early(&call_id);
                return Ok(());
            }
            match self.feed.begin(&call_id, group_id, author.as_hex(), author == me, media, started_at).await {
                Ok(fx) => self.emit(fx),
                Err(e) => self.emit(vec![error_event(&e)]),
            }
            for (early_author, early) in st.take_early(&call_id) {
                Box::pin(self.on_signal(st, group_id, &early_author, me, early, at)).await?;
            }
            if !live {
                // Over before it reached me (its room has expired): on
                // record, not on the banner.
                self.end_announced(st, &call_id, Outcome::Ended, expires).await;
                return Ok(());
            }
            self.schedule_expiry(call_id.clone(), expires);
            if !st.announced.get(&call_id).is_some_and(|a| a.ended) {
                self.emit_announced(st, &call_id);
            }
            return Ok(());
        }
        if !st.announced.contains_key(&call_id) {
            if st.over.contains(&call_id) {
                return Ok(());
            }
            st.hold_early(&call_id, author.clone(), sig);
            return Ok(());
        }
        if st.announced.get(&call_id).is_some_and(|a| a.group_id != group_id) {
            return Ok(());
        }
        match sig {
            GroupSignal::Start { .. } => unreachable!("handled above"),
            GroupSignal::Join { participant, .. } => {
                // My own word of a seat this device took counts while I
                // sit there: the seat is in the record of the call from my
                // own start or join already. Come back when I sit there no
                // more (my leave not yet, or never told: the node closed
                // my seat itself), it would seat a ghost of me. My word
                // of another seat is another device of mine, in the room
                // as anybody.
                let my_seat = st.room_of(&call_id).map(|r| r.seat);
                let Some(a) = st.announced.get_mut(&call_id) else { return Ok(()) };
                if a.ended {
                    return Ok(());
                }
                // A seat is claimed once: the first word holds, and a leave
                // of it that came before (the relays keep no order) makes
                // the claim stale.
                let echo = author == me && a.my_seats.contains(&participant) && my_seat != Some(participant);
                let stale = a.left.contains(&(participant, author.as_hex().to_string())) || echo;
                if !stale && a.seats.get(&participant).is_none_or(|who| who == author) {
                    a.seats.insert(participant, author.clone());
                }
                if let Some(room) = st.room_of(&call_id) {
                    // The note names a seat the node spoke of; it makes none.
                    if participant != room.seat {
                        if let Some(peer) = room.peers.get_mut(&participant) {
                            if !peer.verified && peer.npub.is_none() {
                                peer.npub = Some(author.clone());
                            }
                        }
                    }
                    let me_hex = me.as_hex().to_string();
                    room.refresh(&me_hex);
                    let view = room.view.clone();
                    self.emit(vec![state_event(&view)]);
                }
                self.count_people(st, &call_id).await;
                self.emit_announced(st, &call_id);
            }
            GroupSignal::Leave { participant, .. } => {
                let Some(a) = st.announced.get_mut(&call_id) else { return Ok(()) };
                if a.ended {
                    return Ok(());
                }
                if a.left.len() < LEFT_KEPT {
                    a.left.insert((participant, author.as_hex().to_string()));
                }
                if a.seats.get(&participant) == Some(author) {
                    a.seats.remove(&participant);
                }
                self.emit_announced(st, &call_id);
            }
            GroupSignal::Epoch { epoch, secret, join_token, .. } => {
                let Some(a) = st.announced.get_mut(&call_id) else { return Ok(()) };
                if a.ended {
                    return Ok(());
                }
                // Two rotations that met on one number (the rotator left
                // before its note arrived; the next oldest rotated too):
                // the smaller secret holds, on every device alike.
                let changed = match a.epochs.get(&epoch) {
                    Some(held) if secret >= *held => false,
                    _ => {
                        a.epochs.insert(epoch, secret);
                        true
                    }
                };
                if let Some(token) = join_token {
                    // The creator changed the token of the room: late
                    // joiners use the new one.
                    if author == &a.started_by {
                        a.join_token = token;
                    }
                }
                let newest = a.current_epoch() == epoch;
                if changed && st.room_of(&call_id).is_some() {
                    self.key_everything(st).await;
                    if newest {
                        let gen = st.room.as_ref().map(|r| r.gen).unwrap_or(0);
                        self.schedule(self.timing().send_switch_delay, Timer::Switch { epoch }, gen);
                    }
                    let me_hex = me.as_hex().to_string();
                    self.retry_hellos(st, &me_hex).await;
                    self.say_hello(st).await;
                    if let Some(room) = st.room.as_ref() {
                        self.emit(vec![state_event(&room.view)]);
                    }
                }
            }
            GroupSignal::End { .. } => {
                let outcome = Outcome::Ended;
                if st.room_of(&call_id).is_some() {
                    self.leave_room(st, outcome, Tell::Nobody).await;
                }
                self.end_announced(st, &call_id, outcome, at.min(now)).await;
            }
        }
        Ok(())
    }

    /// The call lost to a newer start of the same moment: it never was
    /// for the group. No record, no line, no banner; out of its room when
    /// I sat in it, and into the room of the call that holds.
    async fn supersede(self: &Arc<Self>, st: &mut State, loser: &str) {
        let Some(group_id) = self.drop_loser(st, loser).await else { return };
        let service = GroupCallService { inner: self.clone() };
        tokio::spawn(async move {
            if let Err(e) = service.join(&group_id).await {
                service.inner.emit(vec![error_event(&e)]);
            }
        });
    }

    /// The losing half of a glare is dropped: over, forgotten, off the
    /// record and the banner, and I am out of its room (the node told,
    /// the group not: the call was never its) when I sat in it. The group
    /// of the call then, for the room of the call that holds to be
    /// joined; `None` when there is nothing to join (I was not in it, or
    /// it was over already).
    async fn drop_loser(&self, st: &mut State, loser: &str) -> Option<String> {
        let a = st.announced.get_mut(loser)?;
        if a.ended {
            return None;
        }
        a.ended = true;
        let (view, group_id) = (a.view(false), a.group_id.clone());
        st.forget(loser);
        let rejoin = st.room_of(loser).is_some();
        if rejoin {
            self.leave_room(st, Outcome::Ended, Tell::Node).await;
        }
        match self.feed.forget(loser).await {
            Ok(fx) => self.emit(fx),
            Err(e) => self.emit(vec![error_event(&e)]),
        }
        self.emit(vec![Effect::Emit(UiEvent {
            name: UI_EVENT_GROUP_CALL_ENDED.into(),
            payload: serde_json::json!({ "call": view, "outcome": Outcome::Ended, "duration_secs": null }),
        })]);
        rejoin.then_some(group_id)
    }

    /// The announced call is over: the record closed as of `ended_at`,
    /// the banner told.
    async fn end_announced(&self, st: &mut State, call_id: &str, outcome: Outcome, ended_at: i64) {
        let Some(a) = st.announced.get_mut(call_id) else { return };
        if a.ended {
            return;
        }
        a.ended = true;
        let view = a.view(false);
        st.forget(call_id);
        match self.feed.finish(call_id, outcome, ended_at).await {
            Ok(fx) => self.emit(fx),
            Err(e) => self.emit(vec![error_event(&e)]),
        }
        let duration = self.feed.row(call_id).await.ok().flatten().and_then(|r| r.ended_at.map(|e| e - r.started_at));
        self.emit(vec![Effect::Emit(UiEvent {
            name: UI_EVENT_GROUP_CALL_ENDED.into(),
            payload: serde_json::json!({ "call": view, "outcome": outcome, "duration_secs": duration }),
        })]);
    }

    /// Out of the room here: the node told (unless nobody is), the session
    /// closed, the group told `call.leave` (and `call.end` when I was the
    /// last; only the node's word on who is left counts) when it is told
    /// at all, the screen told.
    async fn leave_room(&self, st: &mut State, outcome: Outcome, tell: Tell) {
        let Some(mut room) = st.room.take() else { return };
        st.next_gen();
        room.leaving = true;
        let (call_id, group_id, seat) = (room.view.call_id.clone(), room.view.group_id.clone(), room.seat);
        if let Some(s) = room.session.take() {
            if tell != Tell::Nobody && !room.participant_token.is_empty() {
                let _ = self.rooms.leave(&room.node, &room.room_id, seat, &room.participant_token).await;
            }
            s.close().await;
        }
        room.view.phase = GroupPhase::Left;
        room.view.participants.clear();
        self.emit(vec![state_event(&room.view)]);
        let now = self.clock.now().secs();
        let announced = st.announced.get(&call_id).is_some_and(|a| !a.ended);
        if !announced || room.view.joined_at.is_none() {
            return;
        }
        if tell != Tell::All {
            if let Some(a) = st.announced.get_mut(&call_id) {
                a.left_at = Some(now);
            }
            return;
        }
        self.tell_group(&group_id, &GroupSignal::Leave { call_id: call_id.clone(), participant: seat }).await;
        if let Some(a) = st.announced.get_mut(&call_id) {
            a.seats.remove(&seat);
        }
        if room.peers.is_empty() {
            self.tell_group(&group_id, &GroupSignal::End { call_id: call_id.clone(), reason: reason::ENDED.into() }).await;
            self.end_announced(st, &call_id, outcome, now).await;
        } else {
            if let Some(a) = st.announced.get_mut(&call_id) {
                a.left_at = Some(now);
            }
            self.emit_announced(st, &call_id);
        }
    }

    /// The members of the group as they are now. Whoever of my room is a
    /// member no more is nobody from here on (not shown, not listened
    /// to: its keys are spoiled), the creator puts it out and changes
    /// the token of the room, and the keys turn, so that what it kept of
    /// the secrets opens nothing new. The banner loses it too.
    async fn check_members(self: &Arc<Self>, st: &mut State, group_id: &str, members: &[PubKey]) {
        let mut banners = vec![];
        for a in st.announced.values_mut().filter(|a| a.group_id == group_id && !a.ended) {
            let before = a.seats.len();
            a.seats.retain(|_, who| members.contains(who));
            if a.seats.len() != before {
                banners.push(a.call_id.clone());
            }
        }
        let Some(room) = st.room.as_mut().filter(|r| r.view.group_id == group_id) else {
            for id in banners {
                self.emit_announced(st, &id);
            }
            return;
        };
        let gone: Vec<PubKey> = room.members.iter().filter(|m| !members.contains(m)).cloned().collect();
        room.members = members.to_vec();
        if gone.is_empty() {
            for id in banners {
                self.emit_announced(st, &id);
            }
            return;
        }
        let call_id = room.view.call_id.clone();
        let mut expelled: Vec<u32> = vec![];
        for (seat, p) in room.peers.iter_mut() {
            if p.verified && p.npub.as_ref().is_some_and(|n| gone.contains(n)) {
                p.verified = false;
                p.expelled = true;
                p.speaking = false;
                expelled.push(*seat);
            }
        }
        if let (Some(session), Some(a)) = (room.session.as_ref(), st.announced.get(&call_id)) {
            // Keys nobody has on its m-lines: its frames open no more.
            for seat in &expelled {
                let Some(p) = room.peers.get(seat) else { continue };
                for (mid, _) in &p.mids {
                    for (epoch, _) in a.epochs.iter().rev().take(EPOCHS_KEYED) {
                        let _ = session.set_receiver_key(mid, keys::slot(*epoch), &new_secret()).await;
                    }
                }
            }
        }
        let me_hex = self.me().unwrap_or_default();
        room.refresh(&me_hex);
        let view = room.view.clone();
        self.emit(vec![state_event(&view)]);
        let (node, room_id, admin) = (room.node.clone(), room.room_id.clone(), room.admin_token.clone());
        let mut token = None;
        if let Some(admin) = admin {
            for seat in &expelled {
                if let Err(e) = self.rooms.leave(&node, &room_id, *seat, &admin).await {
                    self.emit(vec![error_event(&e.into())]);
                }
            }
            match self.rooms.change_token(&node, &room_id, &admin).await {
                Ok(t) => {
                    if let Some(a) = st.announced.get_mut(&call_id) {
                        a.join_token = t.clone();
                    }
                    token = Some(t);
                }
                Err(e) => self.emit(vec![error_event(&e.into())]),
            }
        }
        let force = token.is_some();
        self.rotate(st, token, force).await;
        self.emit_announced(st, &call_id);
    }

    /// I am no member of the group (any more): out of its room, its call
    /// off my banner and closed on record.
    async fn out_of_group(&self, st: &mut State, group_id: &str) {
        if st.room.as_ref().is_some_and(|r| r.view.group_id == group_id) {
            self.leave_room(st, Outcome::Ended, Tell::Node).await;
        }
        let now = self.clock.now().secs();
        let ids: Vec<String> = st.announced.values().filter(|a| a.group_id == group_id && !a.ended).map(|a| a.call_id.clone()).collect();
        for id in ids {
            self.end_announced(st, &id, Outcome::Ended, now).await;
        }
    }

    async fn on_timer(self: &Arc<Self>, timer: Timer, gen: u64) {
        let mut st = self.state.lock().await;
        let Some(room) = st.room.as_ref().filter(|r| r.gen == gen) else { return };
        match timer {
            Timer::Connect => {
                if !room.ever_connected {
                    self.emit(vec![error_event(&MessengerError::Transport("no way to the node in time".into()))]);
                    self.leave_room(&mut st, Outcome::Failed, Tell::All).await;
                }
            }
            Timer::Verify { seat } => {
                if !room.peers.get(&seat).is_some_and(|p| !p.verified) {
                    return;
                }
                // Nobody, after its time. The creator puts it out (the node
                // says `left`, the keys turn then); without the creator,
                // the keys turn around it and it stays deaf.
                if let Some(admin) = room.admin_token.clone() {
                    let (node, room_id) = (room.node.clone(), room.room_id.clone());
                    if self.rooms.leave(&node, &room_id, seat, &admin).await.is_ok() {
                        return;
                    }
                }
                self.rotate(&mut st, None, false).await;
            }
            Timer::Switch { epoch } => self.switch_sending(&mut st, epoch).await,
        }
    }

    async fn on_engine_event(self: &Arc<Self>, gen: u64, ev: SessionEvent) {
        let mut st = self.state.lock().await;
        let Some(room) = st.room.as_mut().filter(|r| r.gen == gen) else { return };
        let me_hex = self.me().unwrap_or_default();
        match ev {
            SessionEvent::ConnectionState(state) => {
                use crate::engine::ConnectionState as C;
                match state {
                    C::Connected => {
                        room.connected = true;
                        room.ever_connected = true;
                        room.view.phase = GroupPhase::InRoom;
                        let view = room.view.clone();
                        self.emit(vec![state_event(&view)]);
                    }
                    C::Disconnected => {
                        room.connected = false;
                        if room.view.phase == GroupPhase::InRoom {
                            room.view.phase = GroupPhase::Reconnecting;
                            let view = room.view.clone();
                            self.emit(vec![state_event(&view)]);
                        }
                    }
                    C::Failed => {
                        self.emit(vec![error_event(&MessengerError::Transport("the way to the node is lost".into()))]);
                        self.leave_room(&mut st, Outcome::Failed, Tell::All).await;
                    }
                    C::Closed => {
                        if !room.leaving {
                            // The node closed my seat: the room ended, or
                            // the admin removed me.
                            self.leave_room(&mut st, Outcome::Ended, Tell::Nobody).await;
                        }
                    }
                    C::New | C::Connecting => {}
                }
            }
            SessionEvent::DataOpen { label } if label == CTL_LABEL => self.say_hello(&mut st).await,
            SessionEvent::Data { label, payload } if label == CTL_LABEL => match payload {
                DataPayload::Text(text) => self.on_ctl(&mut st, &me_hex, text).await,
                DataPayload::Binary(bytes) => self.on_ctl_frame(&mut st, &me_hex, &bytes).await,
            },
            SessionEvent::RemoteTrackGone { mid } => {
                for p in room.peers.values_mut() {
                    p.mids.retain(|(m, _)| *m != mid);
                }
                room.refresh(&me_hex);
                let view = room.view.clone();
                self.emit(vec![state_event(&view)]);
            }
            SessionEvent::RemoteLevel { mid, level } => {
                let call_id = room.view.call_id.clone();
                if let Some((seat, _)) = room.peers.iter().find(|(_, p)| p.verified && p.mids.iter().any(|(m, _)| *m == mid)) {
                    self.emit(vec![Effect::Emit(UiEvent {
                        name: UI_EVENT_GROUP_CALL_LEVEL.into(),
                        payload: serde_json::json!({ "call_id": call_id, "participant": seat, "level": level }),
                    })]);
                }
            }
            SessionEvent::VideoLost { reason } => {
                if room.view.video_local {
                    room.view.video_local = false;
                    let view = room.view.clone();
                    self.emit(vec![error_event(&MessengerError::Transport(format!("the video stopped: {reason}"))), state_event(&view)]);
                }
            }
            SessionEvent::RemoteTrack { .. }
            | SessionEvent::RemoteVideoSize { .. }
            | SessionEvent::DataOpen { .. }
            | SessionEvent::DataClosed { .. }
            | SessionEvent::Data { .. }
            | SessionEvent::LocalCandidate(_)
            | SessionEvent::GatheringComplete
            | SessionEvent::SelectedPair(_)
            | SessionEvent::Stats(_)
            | SessionEvent::AudioLevel(_)
            | SessionEvent::NetworkChanged
            | SessionEvent::VideoSize { .. } => {}
        }
    }

    /// A text of the node on the control channel.
    async fn on_ctl(self: &Arc<Self>, st: &mut State, me_hex: &str, text: String) {
        let Some(msg) = Message::parse(&text) else { return };
        let Some(room) = st.room.as_mut() else { return };
        match msg {
            Message::Hello { you, participants } => {
                if you != room.seat {
                    self.emit(vec![error_event(&MessengerError::Transport("the node names another seat for me".into()))]);
                    self.leave_room(st, Outcome::Failed, Tell::All).await;
                    return;
                }
                for id in participants {
                    self.seat_appeared(room, id);
                }
                room.refresh(me_hex);
                let view = room.view.clone();
                self.emit(vec![state_event(&view)]);
            }
            Message::Joined { id } => {
                if id != room.seat {
                    let seat_name = st.announced.get(&room.view.call_id).and_then(|a| a.seats.get(&id).cloned());
                    self.seat_appeared(room, id);
                    if let Some(peer) = room.peers.get_mut(&id) {
                        if peer.npub.is_none() {
                            peer.npub = seat_name;
                        }
                    }
                    room.refresh(me_hex);
                    let view = room.view.clone();
                    self.emit(vec![state_event(&view)]);
                    // The newcomer gets my word of identity.
                    self.say_hello(st).await;
                }
            }
            Message::Left { id } => {
                if room.peers.remove(&id).is_some() {
                    room.refresh(me_hex);
                    let view = room.view.clone();
                    self.emit(vec![state_event(&view)]);
                    self.rotate(st, None, false).await;
                }
            }
            Message::Offer { seq, sdp, tracks } => {
                let Some(session) = room.session.as_ref() else { return };
                let answer = match session.set_remote(&sdp, SdpKind::Offer).await {
                    Ok(()) => session.create_answer().await,
                    Err(e) => Err(e),
                };
                match answer {
                    Ok(answer) => {
                        let text = Message::Answer { seq, sdp: answer }.encode();
                        if let Err(e) = session.send_data(CTL_LABEL, DataPayload::Text(text)).await {
                            self.emit(vec![error_event(&e)]);
                        }
                    }
                    Err(e) => {
                        self.emit(vec![error_event(&e)]);
                        return;
                    }
                }
                for Track { id, kind, mid } in tracks {
                    if id == room.seat {
                        continue;
                    }
                    let Some(kind) = Media::parse(&kind) else { continue };
                    self.seat_appeared(room, id);
                    let Some(peer) = room.peers.get_mut(&id) else { continue };
                    peer.mids.retain(|(m, _)| *m != mid);
                    peer.mids.push((mid, kind));
                }
                room.refresh(me_hex);
                let view = room.view.clone();
                self.key_everything(st).await;
                self.emit(vec![state_event(&view)]);
            }
            Message::Speaking { participants } => {
                for (id, p) in room.peers.iter_mut() {
                    p.speaking = p.verified && participants.contains(id);
                }
                room.refresh(me_hex);
                let view = room.view.clone();
                self.emit(vec![state_event(&view)]);
            }
            Message::Answer { .. } => {}
        }
    }

    /// Bytes of another seat: a word of identity, or nothing this version
    /// reads.
    async fn on_ctl_frame(self: &Arc<Self>, st: &mut State, me_hex: &str, frame: &[u8]) {
        let Some((from, bytes)) = ctl::from_relayed(frame) else { return };
        if keys::hello_epoch(bytes).is_none() {
            return;
        }
        self.take_hello(st, me_hex, from, bytes).await;
    }

    /// A word of identity of the seat `from`. One under a secret not here
    /// yet (the note of its epoch is on its way through the relays), or
    /// not under the secret held for that epoch (two rotations met on
    /// one number and the sender has the other), is kept and opened when
    /// a note comes.
    async fn take_hello(self: &Arc<Self>, st: &mut State, me_hex: &str, from: u32, bytes: &[u8]) {
        let Some(epoch) = keys::hello_epoch(bytes) else { return };
        let Some(room) = st.room.as_mut() else { return };
        if from == room.seat {
            return;
        }
        let (call_id, room_id, group_id) = (room.view.call_id.clone(), room.room_id.clone(), room.view.group_id.clone());
        // The node put the seat in front of the frame: it is in the room.
        self.seat_appeared(room, from);
        let secret = st.announced.get(&call_id).and_then(|a| a.epochs.get(&epoch).copied());
        let hello = match secret.map(|s| keys::open_hello(bytes, from, &call_id, &room_id, &s)) {
            Some(Ok(h)) => h,
            None | Some(Err(HelloError::Unreadable)) => {
                if let Some(p) = room.peers.get_mut(&from) {
                    p.pending_hello = Some(bytes.to_vec());
                }
                return;
            }
            Some(Err(_)) => return,
        };
        if let Some(p) = room.peers.get_mut(&from) {
            p.pending_hello = None;
        }
        let Some(npub) = keys::pubkey_of(&hello) else { return };
        let members = self.groups.members(&group_id).await.unwrap_or_default();
        if !members.contains(&npub) {
            // A seat that is not a member of the group: never shown, never
            // listened to.
            return;
        }
        let Some(room) = st.room.as_mut() else { return };
        let Some(peer) = room.peers.get_mut(&from) else { return };
        let first = !peer.verified;
        peer.npub = Some(npub.clone());
        peer.verified = true;
        peer.expelled = false;
        if let Some(a) = st.announced.get_mut(&call_id) {
            // The word of identity is the last word on who sits there.
            a.seats.insert(from, npub);
        }
        room.refresh(me_hex);
        let view = room.view.clone();
        self.key_everything(st).await;
        self.count_people(st, &call_id).await;
        self.emit(vec![state_event(&view)]);
        if first {
            // The first word of a seat is answered with mine: the node says
            // `joined` before the newcomer's channel is open and relays
            // nothing to a channel that is not, so the word sent on
            // `joined` never reaches it; its own word proves its channel
            // is open now. A seat verified already is not answered: two
            // words a pair at most.
            self.say_hello(st).await;
        }
    }

    /// The words of identity kept for a secret that was not here: tried
    /// again, now that an epoch came.
    async fn retry_hellos(self: &Arc<Self>, st: &mut State, me_hex: &str) {
        let pending: Vec<(u32, Vec<u8>)> = st
            .room
            .as_ref()
            .map(|r| r.peers.iter().filter_map(|(s, p)| p.pending_hello.clone().map(|b| (*s, b))).collect())
            .unwrap_or_default();
        for (seat, bytes) in pending {
            self.take_hello(st, me_hex, seat, &bytes).await;
        }
    }
}

impl From<NodeError> for Outcome {
    fn from(_: NodeError) -> Self {
        Outcome::Failed
    }
}
