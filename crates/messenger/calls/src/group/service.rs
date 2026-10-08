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
//!                                     way lost: Reconnecting (judged, joined again, or moved) ─┘
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
//!   joins, so that a newcomer gets everybody's, again whenever the
//!   epoch changes, in answer to every word of another seat, and again
//!   every [`HELLO_RETRY`] for as long as a seat of the room is not
//!   confirmed: the node drops a frame to a channel that is not open yet
//!   (the word sent on `joined` never reaches the newcomer), a channel
//!   may stall, and a word under a secret the other does not hold opens
//!   nothing — one more word costs nothing, a word lost costs the seat.
//!   A word under a secret not here yet (the note of its epoch is on its
//!   way) is kept and opened when the note comes. A seat that has said
//!   nothing for [`VERIFY_DEADLINE`] after I could hear it (my own
//!   channel open; a seat seen before that has its time from the
//!   opening) is nobody for good: the creator puts it out of the room,
//!   or the keys turn without it. Nobody judges a seat while deaf: a
//!   participant whose channel never opened kicks nobody and turns no
//!   keys; the connect timer judges it.
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
//! The cascade and the move (services/call/spec/cascade.md; wire §10,
//! "Каскад и переезд"):
//!
//! - Everybody but the creator sits on its **own nearest node** when that
//!   is worth it: the home of the room (the node of `call.start`) is
//!   asked for a one-time pass of a seat (`delegate`), and my own node
//!   seats me in the home's room through a seat of its own there. The
//!   home answers 404 to `delegate` when it is of the wave before: then,
//!   and whenever my own node refuses or fails, I join the home directly.
//!   A private home (a key in the start) is joined directly: its key
//!   goes to no other node.
//! - A call holds a **chain of rooms**: the room of the start and every
//!   room a `call.move` took it to; the last is the current one. A note
//!   names its room (`room_id`; none: the room of the start). A note of
//!   the current room applies; of a room left behind, it is stale; of a
//!   room not known yet, it waits for the move that brings it.
//! - The way lost, after [`LOST_AFTER`] without the node, is **judged
//!   from two points**: HELLO to the home, and a join into the same room
//!   through my own node. Either answering means the trouble is on my
//!   way: I join again ([`REJOIN_CONNECT`] to connect, two tries, a full
//!   room tried again [`REJOIN_RETRY`] later). Neither: the home is gone.
//! - The home gone, the **first** of the room's last composition (the
//!   creator when its seat is there, else the smallest seat) moves the
//!   room: a new room on another node, `call.move` with a new epoch, then
//!   everybody joins it under the same call. The **second** moves if no
//!   move came within [`MOVE_BACKUP`]; the rest wait [`MOVE_WAIT`] and
//!   leave `failed` — with `call.leave` of the dead room, never
//!   `call.end`: an empty dead room is no knowledge of the end. Two moves
//!   from one room: the newer (`created_at`, then `room_id`) holds, and a
//!   move said more than [`GLARE_WINDOW_SECS`] after the one that holds
//!   is a straggler's and changes nothing. The move that holds **sets**
//!   the epoch: its number takes its secret whatever the number held
//!   (the loser's, a late rotation of the dead room), and the epochs
//!   above it are dropped — everybody applies the one move that holds,
//!   so everybody ends on one secret. A participant sitting well who
//!   gets a `call.move` asks the home first: when it answers, the mover
//!   had the trouble, and its note counts as its leave. A move that did
//!   not work out tells the group nothing: the mover waits for the
//!   second's move like the rest, and leaves `failed` only when there is
//!   no other node at all.
//!
//! The lock on the state is held across calls into the engine and the
//! groups (neither calls back; events come on channels) and across a
//! join of the node, as before; the judgement of a loss and the HELLO at
//! a `call.move` run without it.

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
use crate::node_client::{ice_servers_of, Joined, MediaLimits, NodeAccess, NodeClient, NodeError, RoomApi, RoomCreated, CAP_CASCADE, CAP_SFU};
use crate::servers::{CallNode, NodeClass, NodeRef, ServerSets};
use crate::service::{VideoQuality, CONNECT_TIMEOUT, KEY_VIDEO_QUALITY, VIDEO_FPS};
use crate::signal::{new_call_id, reason};
use messenger_core::traits::UiEvent;
use messenger_core::{Clock, Effect, Envelope, MessengerError, PubKey, Result};
use messenger_dm::DmService;
use messenger_store::{settings, Store};
use messenger_vlink::BridgeId;
use nostr::key::Keys;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, Mutex};

/// Calls announced lately that are over, kept so that a late note of
/// one changes nothing.
const ANNOUNCED_KEPT: usize = 128;
/// Notes of a call that came before its start, or of a room that came
/// before the move that brings it (the relays keep no order): per call
/// and room, and such lists at most.
const EARLY_KEPT: usize = 64;
/// The word a client asks a node for, in a layer request; the node says
/// it in `capabilities` when it has it.
const CAP_SIMULCAST: &str = "simulcast";
/// A seat that has not said who it is this long after I could hear it
/// (the node spoke of it with my channel open, or my channel opened
/// after) is nobody for good: the creator puts it out of the room;
/// without the creator, the oldest verified seat turns the keys. Longer
/// than the worst handshake of the channel (dcSCTP tries its INIT at 1,
/// 2, 4 and 8 seconds: a fifth try at 15 s, seen on a node), so that a
/// seat whose channel came late is not put out as it opens.
pub const VERIFY_DEADLINE: Duration = Duration::from_secs(20);
/// My word of identity is said again this often while a seat of the room
/// is not confirmed, from when my channel opens; and at most this often
/// in answer to the words of the others.
pub const HELLO_RETRY: Duration = Duration::from_secs(2);
/// A new epoch is sent with this much after it is learned: the others
/// have read the note and keyed my m-lines for it by then, and hear me
/// throughout.
pub const SEND_SWITCH_DELAY: Duration = Duration::from_millis(1500);
/// Two starts of one group written within this many seconds of each other
/// are one call: neither creator knew of the other. Two moves from one
/// room are rivals within the same window; a move said more than this
/// after the one that holds is a straggler's (the room was left behind
/// long ago) and changes nothing.
pub const GLARE_WINDOW_SECS: i64 = 60;
/// The way to the node gone (`Disconnected` of the engine not back to
/// `Connected`, my channel closed and not open again) this long is a
/// sign the node may be lost: the judgement from two points begins.
pub const LOST_AFTER: Duration = Duration::from_secs(5);
/// How long a join after a loss may take to connect: the home answered,
/// so the way is there within seconds or not at all.
pub const REJOIN_CONNECT: Duration = Duration::from_secs(10);
/// A full room on a join after a loss is tried again this much later
/// (the home still reaps the seats of the dead way), so many times.
pub const REJOIN_RETRY: Duration = Duration::from_secs(3);
const REJOIN_FULL_TRIES: u32 = 3;
/// Joins after a loss that did not connect before the home is judged
/// gone (or one `room_not_found`).
const REJOIN_TRIES: u32 = 2;
/// The second of the room moves it when no `call.move` came within this
/// of its own loss.
pub const MOVE_BACKUP: Duration = Duration::from_secs(8);
/// Everybody but the first and the second waits this long for a
/// `call.move`, then leaves `failed`.
pub const MOVE_WAIT: Duration = Duration::from_secs(25);
/// A node judged gone is not chosen for this long.
pub const BAD_NODE_HOLD: Duration = Duration::from_secs(300);
/// My own node is taken over the home of the room, when both are of the
/// same standing, only when it is nearer by this much: a node more on
/// the way is a point of failure more.
pub const CASCADE_GAIN: Duration = Duration::from_millis(30);
/// How many of the latest epochs every verified seat's m-lines are keyed
/// for: a frame of an older epoch is of a sender long gone.
const EPOCHS_KEYED: usize = 8;
/// Seats named by `call.leave` kept per room, at most.
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
    /// How often my word of identity is said again while a seat is not
    /// confirmed ([`HELLO_RETRY`]).
    pub hello_retry: Duration,
    /// [`LOST_AFTER`].
    pub lost_after: Duration,
    /// How long the judgement from a point waits for an answer
    /// (`HELLO_CHECK` of the cascade; the HELLO of the node client has
    /// its own bound).
    pub hello_check: Duration,
    /// [`REJOIN_CONNECT`].
    pub rejoin_connect: Duration,
    /// [`REJOIN_RETRY`].
    pub rejoin_retry: Duration,
    /// [`MOVE_BACKUP`].
    pub move_backup: Duration,
    /// [`MOVE_WAIT`].
    pub move_wait: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            verify_deadline: VERIFY_DEADLINE,
            send_switch_delay: SEND_SWITCH_DELAY,
            connect_timeout: CONNECT_TIMEOUT,
            hello_retry: HELLO_RETRY,
            lost_after: LOST_AFTER,
            hello_check: crate::node_client::HELLO_CHECK,
            rejoin_connect: REJOIN_CONNECT,
            rejoin_retry: REJOIN_RETRY,
            move_backup: MOVE_BACKUP,
            move_wait: MOVE_WAIT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Timer {
    /// The way has had its time to come (a first join), or a join after
    /// a loss its time to connect.
    Connect,
    /// The seat has had its time to say who it is.
    Verify { seat: u32 },
    /// Move my sending to the epoch.
    Switch { epoch: u32 },
    /// Say my word again when somebody is not confirmed yet; `opening`
    /// counts the openings of my channel: the chain of an earlier
    /// opening ends when the channel opens anew.
    Hello { opening: u32 },
    /// The way has been gone for [`LOST_AFTER`]: a sign of loss, when it
    /// is not back. `outage` counts the `Disconnected`s of the session:
    /// the time runs from the latest, and the timer of an earlier outage
    /// that ended meanwhile says nothing.
    Lost { outage: u32 },
    /// My channel has been closed for [`LOST_AFTER`]: the same, from its
    /// latest closing.
    CtlLost { closing: u32 },
    /// Try the join after a loss again (a full room, a failed join).
    Rejoin { full_tries: u32 },
    /// The second of the room moves it, no `call.move` having come.
    MoveBackup,
    /// Nobody moved the room: out, `failed`.
    MoveWait,
}

/// Whom to tell on the way out of a room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tell {
    /// The node (my seat freed) and the group (`call.leave`, and
    /// `call.end` when I was the last).
    All,
    /// The group alone, `call.leave` alone: the way out after a sign of
    /// loss (the node is gone or judged so; an empty dead room is no
    /// knowledge of the end).
    LeaveOnly,
    /// The node alone: the group is not mine to tell any more, or the
    /// call was never its.
    Node,
    /// Nobody: the node closed my seat itself, or the session is gone.
    Nobody,
}

/// One room of a call: the room of the start, or one a move took it to.
struct RoomLink {
    room_id: String,
    node: CallNode,
    join_token: String,
    /// When the room ends on the node (0: unknown; the lifetime of a room
    /// of the protocol is taken then).
    expires_at: i64,
    /// The room this one was moved from; `None` for the room of the start.
    from: Option<String>,
    /// Whose room it is: the creator for the room of the start, the
    /// author of the `call.move` for the next. A new join token in
    /// `call.epoch` is taken from the owner alone.
    owner: PubKey,
    /// When the note that brought the room was written, by its author's
    /// clock: what two moves from one room are ordered by.
    said_at: i64,
    /// Who said they sit where (`call.join`), or proved it in the room.
    seats: BTreeMap<u32, PubKey>,
    /// The seats whose `call.leave` came, with who left them (the node
    /// gives a seat once): a `call.join` of one that comes later is stale.
    left: BTreeSet<(u32, String)>,
}

/// A call as the group announced it.
struct Announced {
    call_id: String,
    group_id: String,
    /// The room of the start, then every room a move took the call to;
    /// the last is the current one.
    rooms: Vec<RoomLink>,
    media: Media,
    started_by: PubKey,
    started_at: i64,
    /// When its start was written, by its author's clock: what two starts
    /// of one moment are ordered by, the same on every device.
    said_at: i64,
    /// The secrets of the epochs, by epoch.
    epochs: BTreeMap<u32, Secret>,
    /// The seats this device itself took, by its own start or join, in
    /// any room of the call: my `call.join` on one of them, come back
    /// when I sit there no more, is this device's own echo, not a word of
    /// another device of mine.
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
    fn current(&self) -> &RoomLink {
        self.rooms.last().expect("a call has the room of its start")
    }

    fn current_mut(&mut self) -> &mut RoomLink {
        self.rooms.last_mut().expect("a call has the room of its start")
    }

    fn start_room(&self) -> &str {
        &self.rooms[0].room_id
    }

    fn link(&self, room_id: &str) -> Option<&RoomLink> {
        self.rooms.iter().find(|l| l.room_id == room_id)
    }

    fn link_mut(&mut self, room_id: &str) -> Option<&mut RoomLink> {
        self.rooms.iter_mut().find(|l| l.room_id == room_id)
    }

    /// The room a note is of: the one it names, or the room of the start.
    fn room_named(&self, room_id: Option<&str>) -> String {
        room_id.unwrap_or(self.start_room()).to_string()
    }

    fn current_epoch(&self) -> u32 {
        self.epochs.keys().next_back().copied().unwrap_or(1)
    }

    fn expires(&self) -> i64 {
        let current = self.current();
        if current.expires_at > 0 {
            current.expires_at
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
            participants: self.current().seats.values().map(|p| p.as_hex().to_string()).collect(),
            joined,
        }
    }

    /// Take `secret` for `epoch` by the rule of `call.epoch`: a number
    /// not held yet is taken; on one held, the smaller secret holds.
    /// Whether anything changed.
    fn take_epoch(&mut self, epoch: u32, secret: Secret) -> bool {
        match self.epochs.get(&epoch) {
            Some(held) if secret >= *held => false,
            _ => {
                self.epochs.insert(epoch, secret);
                true
            }
        }
    }

    /// The epoch a `call.move` that holds sets: `epoch` takes `secret`
    /// whatever the number held (the secret of a move that lost, a
    /// rotation of the dead room the mover never saw), and the epochs
    /// above it go — they are of rooms the move writes over. Everybody
    /// applies the one move that holds, so everybody ends on one secret;
    /// the rule of `call.epoch` (the smaller secret) would leave the
    /// winner, who never takes the loser's note, with another secret
    /// than the followers who took both. Whether anything changed.
    fn set_epoch(&mut self, epoch: u32, secret: Secret) -> bool {
        let same = self.epochs.get(&epoch) == Some(&secret) && self.current_epoch() == epoch;
        self.epochs.retain(|e, _| *e < epoch);
        self.epochs.insert(epoch, secret);
        !same
    }
}

/// How a `call.move` from `from_room_id` to `room_id`, said at `at`,
/// stands against the chain of the call: judged the same before the
/// HELLO of a participant sitting well and again when it is applied,
/// so that two moves held for that HELLO settle by their time and not
/// by the order their checks end in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MoveVerdict {
    /// No rival from that room, or this one is the newer: it holds.
    Holds,
    /// The same room again (my own copy, a repeat).
    Repeat,
    /// A rival from the same room holds: this one is older, or a
    /// straggler's, said more than [`GLARE_WINDOW_SECS`] after the one
    /// that holds.
    Lost(&'static str),
}

fn judge_move(a: &Announced, from_room_id: &str, room_id: &str, at: i64) -> MoveVerdict {
    let Some(known) = a.rooms.iter().find(|l| l.from.as_deref() == Some(from_room_id)) else {
        return MoveVerdict::Holds;
    };
    if known.room_id == room_id {
        return MoveVerdict::Repeat;
    }
    if at > known.said_at + GLARE_WINDOW_SECS {
        return MoveVerdict::Lost("a straggler's move, long after the one that holds");
    }
    if (known.said_at, known.room_id.as_str()) >= (at, room_id) {
        return MoveVerdict::Lost("a move that lost to a newer one");
    }
    MoveVerdict::Holds
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
    /// Its `call.leave` of this room came, though the node has not said
    /// `left` (my channel may be stalled): on my way out it does not
    /// keep me from ending the call.
    gone: bool,
}

/// Where a room is after a sign that its node may be lost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Loss {
    /// The judgement from two points, and the join again, are under way.
    Judging,
    /// The home is gone: the room moves, or I wait for its move.
    HomeLost,
}

struct Room {
    gen: u64,
    view: GroupCallView,
    /// The node I am connected to: the home, or my own node in a cascade.
    node: CallNode,
    /// The node the room is on.
    home: CallNode,
    room_id: String,
    session: Option<Box<dyn Session>>,
    seat: u32,
    participant_token: String,
    /// In a cascade: the token of my seat on the home, to give it up
    /// myself when my own node is gone.
    home_token: Option<String>,
    /// The owner's: changes the token, removes a seat.
    admin_token: Option<String>,
    dtls_fp: String,
    peers: BTreeMap<u32, Peer>,
    /// The node said the way is there.
    connected: bool,
    /// The way was there at least once in this session: a loss after
    /// that is the engine's to mend, not the connect timer's to judge.
    ever_connected: bool,
    /// The session is a join after a loss or a move: the room and the
    /// call go on, and a way that never comes in its time is a sign of
    /// loss (judged from two points), not a failed first join.
    again: bool,
    /// How many times the way went `Disconnected` in this session: the
    /// time of a loss runs from the latest outage.
    outages: u32,
    /// How many times my channel closed in this session: the same.
    closings: u32,
    /// `leave` is under way: a `Closed` of the engine is no news.
    leaving: bool,
    /// The node has simulcast: a layer may be asked for.
    simulcast: bool,
    /// The epoch and the secret my frames go out under.
    sending: Option<(u32, Secret)>,
    /// The members of the group as last seen: who is gone when they change.
    members: Vec<PubKey>,
    /// My control channel is open: I hear the words of the others and the
    /// node relays mine. The time of a seat to say who it is runs from
    /// here at the earliest.
    ctl_open: bool,
    /// How many times my channel opened: the chain of `Timer::Hello` of
    /// an earlier opening ends when it opens anew.
    openings: u32,
    /// When my word of identity last went out.
    last_hello: Option<Instant>,
    /// I told the group `call.join` of this room: `call.leave` is owed.
    claimed: bool,
    /// The seats of the room as the node last named them (me among
    /// them), kept through a loss: who is first to move the room.
    composition: BTreeSet<u32>,
    /// A sign of loss was taken.
    lost: Option<Loss>,
    /// My own node said the home is gone (`home_lost`): it has judged
    /// the home from its point already.
    home_lost_by_node: bool,
    /// Joins after the loss that did not connect.
    rejoins: u32,
    /// I am making the new room of the call myself.
    moving: bool,
}

impl Room {
    fn new(gen: u64, view: GroupCallView, home: CallNode, room_id: String, members: Vec<PubKey>) -> Self {
        Self {
            gen,
            view,
            node: home.clone(),
            home,
            room_id,
            session: None,
            seat: 0,
            participant_token: String::new(),
            home_token: None,
            admin_token: None,
            dtls_fp: String::new(),
            peers: BTreeMap::new(),
            connected: false,
            ever_connected: false,
            again: false,
            outages: 0,
            closings: 0,
            leaving: false,
            simulcast: false,
            sending: None,
            members,
            ctl_open: false,
            openings: 0,
            last_hello: None,
            claimed: false,
            composition: BTreeSet::new(),
            lost: None,
            home_lost_by_node: false,
            rejoins: 0,
            moving: false,
        }
    }

    /// A loss of the way is mine to judge (from two points), not the
    /// connect timer's: the way was there once in this session, or the
    /// session is a join after a loss or a move.
    fn judges_loss(&self) -> bool {
        self.ever_connected || self.again
    }

    /// A seat of the room is not confirmed and not spoiled: my word may
    /// still be wanted.
    fn somebody_unconfirmed(&self) -> bool {
        self.peers.values().any(|p| !p.verified && !p.expelled)
    }

    /// Nobody I know of is left in the room: everybody else has gone by
    /// the node's word, or said so itself.
    fn nobody_left(&self) -> bool {
        self.peers.values().all(|p| p.gone)
    }

    /// Who sits on `seat` as far as I know: me, a verified or claimed
    /// peer, or nobody.
    fn owner_of(&self, seat: u32, me: &PubKey) -> Option<PubKey> {
        if seat == self.seat {
            return Some(me.clone());
        }
        self.peers.get(&seat).and_then(|p| p.npub.clone())
    }
}

/// How I get into a room: directly on its home, or through my own node.
struct Plan {
    home: NodeAccess,
    /// My own node and the pass of a seat the home gave for it.
    via: Option<(NodeAccess, String)>,
}

/// What a join gave: the session on its way, the node's answer.
struct Entered {
    session: Box<dyn Session>,
    offer: String,
    joined: Joined,
    /// The node I am connected to.
    node: CallNode,
    access: NodeAccess,
}

/// What an early-held note is keyed by: the call, and the room it names
/// (`None` — before the start, any room).
type EarlyKey = (String, Option<String>);
/// An early-held note: its author, the note, when it was written.
type EarlyNote = (PubKey, GroupSignal, i64);

#[derive(Default)]
struct State {
    announced: HashMap<String, Announced>,
    /// Calls over, newest last.
    over: VecDeque<String>,
    /// Notes that came before what they belong to, by call and room:
    /// `None` is "before the start of the call" (any room), `Some` a
    /// room the chain does not know yet.
    early: VecDeque<(EarlyKey, Vec<EarlyNote>)>,
    room: Option<Room>,
    gen: u64,
    /// Nodes judged gone, and when: not chosen for [`BAD_NODE_HOLD`].
    bad_nodes: HashMap<BridgeId, Instant>,
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

    fn hold_early(&mut self, call_id: &str, room: Option<String>, author: PubKey, sig: GroupSignal, at: i64) {
        let key = (call_id.to_string(), room);
        if let Some((_, list)) = self.early.iter_mut().find(|(k, _)| *k == key) {
            if list.len() < EARLY_KEPT {
                list.push((author, sig, at));
            }
            return;
        }
        if self.early.len() == EARLY_KEPT {
            self.early.pop_front();
        }
        self.early.push_back((key, vec![(author, sig, at)]));
    }

    fn take_early(&mut self, call_id: &str, room: Option<&str>) -> Vec<(PubKey, GroupSignal, i64)> {
        let key = (call_id.to_string(), room.map(String::from));
        match self.early.iter().position(|(k, _)| *k == key) {
            Some(i) => self.early.remove(i).map(|(_, l)| l).unwrap_or_default(),
            None => vec![],
        }
    }

    /// Everything held for the call, in every room: dropped with it.
    fn drop_early(&mut self, call_id: &str) {
        self.early.retain(|((id, _), _)| id != call_id);
    }

    /// The call of the group that is on now: the newest of the live ones
    /// (two of one moment are one call, the newer; see `GLARE_WINDOW_SECS`).
    fn live_call(&self, group_id: &str, now: i64) -> Option<&Announced> {
        self.announced.values().filter(|a| a.group_id == group_id && a.live(now)).max_by_key(|a| (a.said_at, a.call_id.clone()))
    }

    fn mark_bad(&mut self, node: &NodeRef) {
        self.bad_nodes.insert(node.id, Instant::now());
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
        let (gen, bad) = {
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
                home: String::new(),
                participant: None,
                epoch: 1,
                participants: vec![],
                limits: None,
                kbps_per_participant: 0,
                max_participants: 0,
            };
            let placeholder = CallNode::new(format!("0.0.0.0:1#{}", "00".repeat(32)).parse().expect("a placeholder"), NodeClass::Own);
            st.room = Some(Room::new(gen, view.clone(), placeholder, String::new(), members));
            inner.emit(vec![state_event(&view)]);
            (gen, st.bad_nodes.clone())
        };
        // The node and the room are asked without the lock.
        let outcome: Result<(NodeAccess, RoomCreated)> = async {
            let access = inner.sfu_node(group_id, now, &bad).await?;
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
                rooms: vec![RoomLink {
                    room_id: created.room_id.clone(),
                    node: node.clone(),
                    join_token: created.join_token.clone(),
                    expires_at: created.expires_at as i64,
                    from: None,
                    owner: me.clone(),
                    said_at: now,
                    seats: BTreeMap::new(),
                    left: BTreeSet::new(),
                }],
                media,
                started_by: me.clone(),
                started_at: now,
                said_at: now,
                epochs: BTreeMap::from([(1, secret)]),
                my_seats: BTreeSet::new(),
                left_at: None,
                ended: false,
            },
        );
        inner.schedule_expiry(call_id.clone(), created.expires_at as i64);
        {
            let room = st.room.as_mut().expect("checked above");
            room.node = node.clone();
            room.home = node.clone();
            room.room_id = created.room_id.clone();
            room.admin_token = Some(created.admin_token.clone());
            room.view.node = node.node.to_string();
            room.view.home = node.node.to_string();
            room.view.limits = Some(access.welcome.limits.clone());
            room.view.kbps_per_participant = created.kbps_per_participant;
            room.view.max_participants = created.max_participants;
            room.simulcast = access.welcome.capabilities.iter().any(|c| c == CAP_SIMULCAST);
        }
        match inner.feed.begin(&call_id, group_id, me.as_hex(), true, media, now).await {
            Ok(fx) => inner.emit(fx),
            Err(e) => inner.emit(vec![error_event(&e)]),
        }
        let plan = Plan { home: access, via: None };
        if let Err(e) = inner.enter_room(&mut st, &keys, &call_id, plan, &created.join_token, &secret, 1, false).await {
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
            a.rooms[0].said_at = said;
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
            room_id: created.room_id.clone(),
            node: node.node.clone(),
            key: node.access_key.clone(),
            join_token: created.join_token,
            secret,
            media,
            expires_at: created.expires_at as i64,
        };
        inner.tell_group(group_id, &start).await;
        inner.claim_seat(&mut st, &me, &call_id).await;
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
        let (gen, call_id, home, room_id, token, secret, epoch, bad) = {
            let mut st = inner.state.lock().await;
            if st.room.is_some() {
                return Err(MessengerError::Invalid("a group call is under way".into()));
            }
            let Some(a) = st.live_call(group_id, now) else {
                return Err(MessengerError::Invalid("no call is on in this group".into()));
            };
            let epoch = a.current_epoch();
            let secret = *a.epochs.get(&epoch).expect("the current epoch has its secret");
            let current = a.current();
            let (call_id, home, token, media, started_by, started_at, room_id) = (
                a.call_id.clone(),
                current.node.clone(),
                current.join_token.clone(),
                a.media,
                a.started_by.clone(),
                a.started_at,
                current.room_id.clone(),
            );
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
                node: home.node.to_string(),
                home: home.node.to_string(),
                participant: None,
                epoch,
                participants: vec![],
                limits: None,
                kbps_per_participant: 0,
                max_participants: 0,
            };
            st.room = Some(Room::new(gen, view.clone(), home.clone(), room_id.clone(), members));
            inner.emit(vec![state_event(&view)]);
            (gen, call_id, home, room_id, token, secret, epoch, st.bad_nodes.clone())
        };
        // The credentials of the home and the choice of my own node, the
        // pass of a seat when I go through it: all without the lock.
        let plan = inner.plan_join(group_id, &home, &room_id, &token, now, &bad).await;
        let mut st = inner.state.lock().await;
        if st.room.as_ref().is_none_or(|r| r.gen != gen) {
            return Err(MessengerError::Invalid("the call ended".into()));
        }
        let plan = match plan {
            Ok(p) => p,
            Err(e) => {
                inner.leave_room(&mut st, Outcome::Failed, Tell::Nobody).await;
                return Err(e);
            }
        };
        if let Err(e) = inner.enter_room(&mut st, &keys, &call_id, plan, &token, &secret, epoch, false).await {
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
        inner.claim_seat(&mut st, &me, &call_id).await;
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

/// The order of the seats of a room for the move: the creator's seat
/// first when it is there, else the smallest; then by number. `owners`
/// says who sits on a seat as far as the judge knows.
fn move_order(composition: &BTreeSet<u32>, creator: &PubKey, owners: &BTreeMap<u32, PubKey>) -> Vec<u32> {
    let mut order: Vec<u32> = composition.iter().copied().collect();
    if let Some(at) = order.iter().position(|s| owners.get(s) == Some(creator)) {
        let first = order.remove(at);
        order.insert(0, first);
    }
    order
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

    fn my_key(&self) -> Option<PubKey> {
        self.me().and_then(|hex| PubKey::parse(&hex))
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
    /// of when I last left it, or when the room ended. A room the call
    /// moved to since has its own timer; this one finds the call live.
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

    /// The nodes a room of `group_id` may be made on, or sat on through:
    /// the one pinned to the group (class `Group`), then the sets of
    /// servers; nodes judged gone lately left out.
    async fn candidates(&self, group_id: &str, bad: &HashMap<BridgeId, Instant>) -> Vec<CallNode> {
        let mut out: Vec<CallNode> = Vec::new();
        if let Ok(Some(mut pinned)) = self.groups.pinned_node(group_id).await {
            pinned.class = NodeClass::Group;
            out.push(pinned);
        }
        for n in self.servers.call_nodes().await.unwrap_or_default() {
            if !out.iter().any(|o| o.node.id == n.node.id) {
                out.push(n);
            }
        }
        out.retain(|n| !bad.get(&n.node.id).is_some_and(|t| t.elapsed() < BAD_NODE_HOLD));
        out
    }

    /// The node for a room of `group_id`: the one pinned to the group,
    /// else the first with an SFU from the sets of servers.
    async fn sfu_node(&self, group_id: &str, now: i64, bad: &HashMap<BridgeId, Instant>) -> Result<NodeAccess> {
        if let Some(pinned) = self.groups.pinned_node(group_id).await? {
            let access = self.nodes.access(&pinned, now).await?;
            if !access.welcome.capabilities.iter().any(|c| c == CAP_SFU) {
                return Err(MessengerError::Transport("the group's node has no SFU".into()));
            }
            return Ok(access);
        }
        let nodes = self.candidates(group_id, bad).await;
        self.nodes.pick_sfu(&nodes, now).await.ok_or_else(|| MessengerError::Transport("no call node with an SFU answered".into()))
    }

    /// How to get into the room `room_id` on `home` (cascade.md,
    /// "Клиент → Выбор узла при входе"): the credentials of the home
    /// and, at the same time, my own nearest node from my sets. Through
    /// my own node when it is another node with the cascade and either
    /// stands higher in my sets than the home (a home not in them stands
    /// below all: my own node keeps my address from it) or is nearer by
    /// [`CASCADE_GAIN`] — and the home gives a pass for it. A private
    /// home is joined directly: its key goes to no other node. A home
    /// without `delegate` (404) is of the wave before: directly.
    async fn plan_join(&self, group_id: &str, home: &CallNode, room_id: &str, join_token: &str, now: i64, bad: &HashMap<BridgeId, Instant>) -> Result<Plan> {
        if home.access_key.is_some() {
            let access = self.nodes.access(home, now).await?;
            return Ok(Plan { home: access, via: None });
        }
        let candidates = self.candidates(group_id, bad).await;
        let home_class = candidates.iter().find(|n| n.node.id == home.node.id).map(|n| n.class);
        let (home_access, own) = tokio::join!(self.nodes.access(home, now), self.nodes.pick_sfu(&candidates, now));
        let home_access = home_access?;
        let Some(own) = own else {
            return Ok(Plan { home: home_access, via: None });
        };
        let has_cascade = own.welcome.capabilities.iter().any(|c| c == CAP_CASCADE);
        let higher = home_class.is_none_or(|hc| own.node.class < hc);
        let nearer = own.rtt + CASCADE_GAIN < home_access.rtt;
        if own.node.node.id == home.node.id || !has_cascade || !(higher || nearer) {
            tracing::debug!(own = %own.node.node.id.short(), has_cascade, higher, nearer, "group call: joining the home directly");
            return Ok(Plan { home: home_access, via: None });
        }
        match self.rooms.delegate(home, room_id, join_token).await {
            Ok(pass) => {
                tracing::info!(own = %own.node.node.id.short(), home = %home.node.id.short(), "group call: joining through my own node");
                Ok(Plan { home: home_access, via: Some((own, pass.proxy_token)) })
            }
            Err(e) if e.status() == Some(403) => Err(e.into()),
            Err(e) => {
                tracing::debug!(error = %e, "group call: no pass from the home: joining it directly");
                Ok(Plan { home: home_access, via: None })
            }
        }
    }

    /// A session with the one offer of a room: on the ICE servers of
    /// `access` (the node I connect to).
    async fn open_session(&self, call_id: &str, media: Media, simulcast: bool, access: &NodeAccess) -> Result<(Box<dyn Session>, String)> {
        let servers: Vec<IceServer> = ice_servers_of(&access.credentials);
        let config = RoomConfig { call_id: call_id.to_string(), data_label: CTL_LABEL.into(), key_salt: call_id.as_bytes().to_vec(), simulcast };
        let session = self.engine.create_room_session(servers, RelayPolicy::Auto, media, config).await?;
        match session.create_offer().await {
            Ok(offer) => Ok((session, offer)),
            Err(e) => {
                session.close().await;
                Err(e)
            }
        }
    }

    /// The join itself: through my own node when the plan says so, and
    /// directly on the home when it does not, or when my own node
    /// refuses or fails (once; the home's own 404 and 409 through it are
    /// its words, told as they are).
    async fn attempt_join(&self, call_id: &str, room_id: &str, media: Media, simulcast: bool, plan: &Plan, join_token: &str) -> Result<Entered> {
        let home = plan.home.node.clone();
        if let Some((own, pass)) = &plan.via {
            let (session, offer) = self.open_session(call_id, media, simulcast, own).await?;
            match self.rooms.join_via(&own.node, &home.node, room_id, pass, &offer).await {
                Ok(joined) => return Ok(Entered { session, offer, joined, node: own.node.clone(), access: own.clone() }),
                Err(e) => {
                    session.close().await;
                    if e.is_not_found() || e.is_room_full() {
                        return Err(e.into());
                    }
                    tracing::info!(error = %e, "group call: my own node did not seat me: joining the home directly");
                }
            }
        }
        let (session, offer) = self.open_session(call_id, media, simulcast, &plan.home).await?;
        match self.rooms.join(&home, room_id, join_token, &offer).await {
            Ok(joined) => Ok(Entered { session, offer, joined, node: home, access: plan.home.clone() }),
            Err(e) => {
                session.close().await;
                Err(e.into())
            }
        }
    }

    /// Into the room: the session, my one offer to the node, its answer,
    /// my seat, my sending key. The room is in `st` already. `again`
    /// after a loss or a move: the seats come fresh from the node, the
    /// record keeps its first join, the way has [`REJOIN_CONNECT`].
    #[allow(clippy::too_many_arguments)]
    async fn enter_room(
        self: &Arc<Self>,
        st: &mut State,
        keys: &Keys,
        call_id: &str,
        plan: Plan,
        join_token: &str,
        secret: &Secret,
        epoch: u32,
        again: bool,
    ) -> Result<()> {
        let (gen, room_id, media, simulcast) = {
            let room = st.room.as_ref().ok_or_else(|| MessengerError::Invalid("no room".into()))?;
            (room.gen, room.room_id.clone(), room.view.media, room.simulcast)
        };
        let entered = self.attempt_join(call_id, &room_id, media, simulcast, &plan, join_token).await?;
        let Entered { session, offer, joined, node, access } = entered;
        if let Err(e) = session.set_remote(&joined.sdp_answer, SdpKind::Answer).await {
            session.close().await;
            return Err(e);
        }
        let key = keys::sender_key(secret, call_id, joined.participant_id, epoch);
        if let Err(e) = session.set_sender_key(keys::slot(epoch), &key).await {
            session.close().await;
            return Err(e);
        }
        self.pump(session.events(), gen);
        let now = self.clock.now().secs();
        let room = st.room.as_mut().ok_or_else(|| MessengerError::Invalid("no room".into()))?;
        room.session = Some(session);
        room.seat = joined.participant_id;
        room.participant_token = joined.participant_token;
        room.home_token = joined.home_token;
        room.node = node.clone();
        room.view.node = node.node.to_string();
        room.view.home = room.home.node.to_string();
        room.view.limits = Some(access.welcome.limits.clone());
        room.simulcast = access.welcome.capabilities.iter().any(|c| c == CAP_SIMULCAST);
        room.dtls_fp = keys::dtls_fingerprint(&offer);
        room.sending = Some((epoch, *secret));
        room.peers.clear();
        room.composition = BTreeSet::from([room.seat]);
        room.connected = false;
        room.ever_connected = false;
        room.again = again;
        room.ctl_open = false;
        room.lost = None;
        room.home_lost_by_node = false;
        room.moving = false;
        for id in joined.participants {
            self.seat_appeared(room, id);
        }
        room.view.phase = GroupPhase::Joining;
        if room.view.joined_at.is_none() {
            room.view.joined_at = Some(now);
        }
        room.view.epoch = epoch;
        room.refresh(&keys.public_key().to_hex());
        let view = room.view.clone();
        let wait = if again { self.timing().rejoin_connect } else { self.timing().connect_timeout };
        self.schedule(wait, Timer::Connect, gen);
        tracing::info!(seat = room.seat, node = %node.node.id.short(), home = %room.home.node.id.short(), again, "group call: in the room");
        if !again {
            if let Err(e) = self.feed.joined(call_id, now).await {
                self.emit(vec![error_event(&e)]);
            }
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

    /// My seat told to the group (`call.join` of the room I am in) and
    /// put on the banner and the record.
    async fn claim_seat(&self, st: &mut State, me: &PubKey, call_id: &str) {
        let Some(room) = st.room_of(call_id) else { return };
        let (seat, group_id, room_id) = (room.seat, room.view.group_id.clone(), room.room_id.clone());
        room.claimed = true;
        self.tell_group(&group_id, &GroupSignal::Join { call_id: call_id.to_string(), participant: seat, room_id: Some(room_id.clone()) }).await;
        if let Some(a) = st.announced.get_mut(call_id) {
            if let Some(link) = a.link_mut(&room_id) {
                link.seats.insert(seat, me.clone());
            }
            a.my_seats.insert(seat);
        }
        if let Err(e) = self.feed.took_seat(call_id, seat).await {
            self.emit(vec![error_event(&e)]);
        }
        self.emit_announced(st, call_id);
    }

    /// A seat the node spoke of is in my room: nobody until its word of
    /// identity comes, and its time to say it runs from now when my
    /// channel is open — from the opening otherwise (`channel_opened`):
    /// deaf, I would judge a seat whose word I could not have heard.
    fn seat_appeared(self: &Arc<Self>, room: &mut Room, seat: u32) {
        room.composition.insert(seat);
        if seat == room.seat || room.peers.contains_key(&seat) {
            return;
        }
        room.peers.insert(seat, Peer::default());
        if room.ctl_open {
            self.schedule(self.timing().verify_deadline, Timer::Verify { seat }, room.gen);
        }
    }

    /// My channel is open: every seat not confirmed yet has its time
    /// from now, my word goes out, and goes again every `hello_retry`
    /// while somebody is not confirmed (`Timer::Hello`).
    async fn channel_opened(self: &Arc<Self>, st: &mut State) {
        let Some(room) = st.room.as_mut() else { return };
        room.ctl_open = true;
        room.openings += 1;
        let (gen, opening) = (room.gen, room.openings);
        let timing = self.timing();
        let unconfirmed: Vec<u32> = room.peers.iter().filter(|(_, p)| !p.verified).map(|(s, _)| *s).collect();
        for seat in unconfirmed {
            self.schedule(timing.verify_deadline, Timer::Verify { seat }, gen);
        }
        tracing::debug!(seat = room.seat, "group call: the control channel is open");
        self.say_hello(st).await;
        self.schedule(timing.hello_retry, Timer::Hello { opening }, gen);
    }

    /// My channel closed: nobody is judged, no word is said, until it
    /// opens again; and not open again in [`LOST_AFTER`], it is a sign
    /// the node may be lost.
    fn channel_closed(self: &Arc<Self>, st: &mut State) {
        if let Some(room) = st.room.as_mut() {
            room.ctl_open = false;
            room.closings += 1;
            tracing::debug!(seat = room.seat, "group call: the control channel closed");
            if room.judges_loss() && !room.leaving && room.lost.is_none() {
                self.schedule(self.timing().lost_after, Timer::CtlLost { closing: room.closings }, room.gen);
            }
        }
    }

    /// My word in answer to the word of another seat: it proves its
    /// channel is open now, and the word I sent it before may have been
    /// lost (sent on `joined`, before its channel was open; dropped by a
    /// stalled channel; under a secret it did not hold). Every word is
    /// answered, a quarter of `hello_retry` apart at least: two words a
    /// pair in the common case, a few more when the way is bad.
    async fn answer_hello(&self, st: &mut State) {
        let gap = self.timing().hello_retry / 4;
        let recent = st.room.as_ref().and_then(|r| r.last_hello).is_some_and(|at| at.elapsed() < gap);
        if !recent {
            self.say_hello(st).await;
        }
    }

    /// How many people the record of the call has seen: the seats that
    /// said they are in, and me.
    async fn count_people(&self, st: &State, call_id: &str) {
        let Some(a) = st.announced.get(call_id) else { return };
        let mut people: Vec<&str> = a.current().seats.values().map(|p| p.as_hex()).collect();
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
        tracing::debug!(seat = room.seat, epoch, "group call: my word of identity goes out");
        let sent = session.send_data(CTL_LABEL, DataPayload::Binary(frame)).await;
        match sent {
            Ok(()) => {
                if let Some(room) = st.room.as_mut() {
                    room.last_hello = Some(Instant::now());
                }
            }
            // A channel that is not open yet takes no word: the word
            // goes again when it opens. Anything else is told.
            Err(e) => {
                tracing::debug!(error = %e, "group call: my word of identity did not go out");
                if st.room.as_ref().is_some_and(|r| r.ctl_open) {
                    self.emit(vec![error_event(&e)]);
                }
            }
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
    /// by the owner of the room too (`force`), with the room's new token.
    async fn rotate(self: &Arc<Self>, st: &mut State, join_token: Option<String>, force: bool) {
        let Some(room) = st.room.as_ref() else { return };
        if room.view.phase == GroupPhase::Left || room.lost.is_some() || (!force && !room.oldest()) {
            return;
        }
        let (gen, call_id, group_id, room_id) = (room.gen, room.view.call_id.clone(), room.view.group_id.clone(), room.room_id.clone());
        let Some(a) = st.announced.get_mut(&call_id) else { return };
        let epoch = a.current_epoch() + 1;
        let secret = new_secret();
        a.epochs.insert(epoch, secret);
        self.tell_group(&group_id, &GroupSignal::Epoch { call_id, epoch, secret, join_token, room_id: Some(room_id) }).await;
        self.key_everything(st).await;
        self.schedule(self.timing().send_switch_delay, Timer::Switch { epoch }, gen);
        self.say_hello(st).await;
    }

    /// `on_signal` boxed, for the notes held early that are handed back
    /// through it: the recursion needs a named future, and one that is
    /// `Send` (the move applies from a task of its own).
    fn on_signal_boxed<'a>(
        self: &'a Arc<Self>,
        st: &'a mut State,
        group_id: &'a str,
        author: &'a PubKey,
        me: &'a PubKey,
        sig: GroupSignal,
        at: i64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(self.on_signal(st, group_id, author, me, sig, at))
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
                    st.drop_early(&call_id);
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
            let node = CallNode { node, class: NodeClass::Group, access_key: key };
            let started_at = at.min(now);
            let a = Announced {
                call_id: call_id.clone(),
                group_id: group_id.to_string(),
                rooms: vec![RoomLink {
                    room_id: room_id.clone(),
                    node,
                    join_token,
                    expires_at,
                    from: None,
                    owner: author.clone(),
                    said_at: at,
                    seats: BTreeMap::new(),
                    left: BTreeSet::new(),
                }],
                media,
                started_by: author.clone(),
                started_at,
                said_at: at,
                epochs: BTreeMap::from([(1, secret)]),
                my_seats,
                left_at: None,
                ended: over_on_record,
            };
            let (live, expires) = (a.live(now), a.expires());
            st.announced.insert(call_id.clone(), a);
            if over_on_record {
                st.forget(&call_id);
                st.drop_early(&call_id);
                return Ok(());
            }
            match self.feed.begin(&call_id, group_id, author.as_hex(), author == me, media, started_at).await {
                Ok(fx) => self.emit(fx),
                Err(e) => self.emit(vec![error_event(&e)]),
            }
            let mut early = st.take_early(&call_id, None);
            early.extend(st.take_early(&call_id, Some(&room_id)));
            for (early_author, early, early_at) in early {
                self.on_signal_boxed(st, group_id, &early_author, me, early, early_at).await?;
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
        let Some(a) = st.announced.get(&call_id) else {
            if st.over.contains(&call_id) {
                return Ok(());
            }
            st.hold_early(&call_id, None, author.clone(), sig, at);
            return Ok(());
        };
        if a.group_id != group_id {
            return Ok(());
        }
        // The room the note is of: the current one applies; one left
        // behind is stale; one not known yet waits for the move that
        // brings it (the relays keep no order: a `call.join` of the new
        // room often comes before the `call.move`).
        let room_id = a.room_named(sig.room_id());
        if a.link(&room_id).is_none() {
            if !a.ended {
                st.hold_early(&call_id, Some(room_id), author.clone(), sig, at);
            }
            return Ok(());
        }
        if let GroupSignal::Move { .. } = sig {
            return self.on_move(st, group_id, author, me, sig, at).await;
        }
        if a.current().room_id != room_id {
            tracing::debug!(t = ?sig, room = %room_id, "group call: a note of a room left behind");
            return Ok(());
        }
        match sig {
            GroupSignal::Start { .. } | GroupSignal::Move { .. } => unreachable!("handled above"),
            GroupSignal::Join { participant, .. } => {
                // My own word of a seat this device took counts while I
                // sit there: the seat is in the record of the call from my
                // own start or join already. Come back when I sit there no
                // more (my leave not yet, or never told: the node closed
                // my seat itself), it would seat a ghost of me. My word
                // of another seat is another device of mine, in the room
                // as anybody.
                let my_seat = st.room_of(&call_id).filter(|r| r.room_id == room_id).map(|r| r.seat);
                let Some(a) = st.announced.get_mut(&call_id) else { return Ok(()) };
                if a.ended {
                    return Ok(());
                }
                // A seat is claimed once: the first word holds, and a leave
                // of it that came before (the relays keep no order) makes
                // the claim stale.
                let echo = author == me && a.my_seats.contains(&participant) && my_seat != Some(participant);
                let link = a.current_mut();
                let stale = link.left.contains(&(participant, author.as_hex().to_string())) || echo;
                if !stale && link.seats.get(&participant).is_none_or(|who| who == author) {
                    link.seats.insert(participant, author.clone());
                }
                if let Some(room) = st.room_of(&call_id).filter(|r| r.room_id == room_id) {
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
                let link = a.current_mut();
                if link.left.len() < LEFT_KEPT {
                    link.left.insert((participant, author.as_hex().to_string()));
                }
                if link.seats.get(&participant) == Some(author) {
                    link.seats.remove(&participant);
                }
                // The seat said it left, whatever the node has said so far
                // (its `left` may be stuck in my channel): on my own way
                // out it does not keep me from ending the call.
                if let Some(room) = st.room_of(&call_id).filter(|r| r.room_id == room_id) {
                    if let Some(peer) = room.peers.get_mut(&participant) {
                        if peer.npub.as_ref() == Some(author) {
                            peer.gone = true;
                        }
                    }
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
                let changed = a.take_epoch(epoch, secret);
                if let Some(token) = join_token {
                    // The owner of the room changed its token: late
                    // joiners use the new one.
                    if author == &a.current().owner {
                        a.current_mut().join_token = token;
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

    /// A `call.move` of a room the chain knows (cascade.md, "Переезд →
    /// Приём"): judged by the author's right to move (its seat in the
    /// last composition of the room left, first or second in the order
    /// of the move), by what I know of the room (sitting well, I ask the
    /// home first), and against another move from the same room (the
    /// newer holds).
    async fn on_move(self: &Arc<Self>, st: &mut State, group_id: &str, author: &PubKey, me: &PubKey, sig: GroupSignal, at: i64) -> Result<()> {
        let GroupSignal::Move { call_id, from_room_id, room_id, seat, .. } = &sig else { return Ok(()) };
        let (call_id, from_room_id, room_id, seat) = (call_id.clone(), from_room_id.clone(), room_id.clone(), *seat);
        let Some(a) = st.announced.get(&call_id) else { return Ok(()) };
        if a.ended {
            return Ok(());
        }
        let Some(from) = a.link(&from_room_id) else { return Ok(()) };
        // Another move from the same room came before: the newer holds
        // (judged again when the move is applied, after the HELLO of a
        // participant sitting well). A duplicate (my own copy, a repeat)
        // changes nothing.
        match judge_move(a, &from_room_id, &room_id, at) {
            MoveVerdict::Holds => {}
            MoveVerdict::Repeat => return Ok(()),
            MoveVerdict::Lost(why) => {
                tracing::info!(room = %room_id, why, "group call: a move that does not hold");
                st.take_early(&call_id, Some(&room_id));
                return Ok(());
            }
        }
        // The right to move: the author's seat in the last composition
        // of the room left, as I know it (from the node when I sat
        // there, else from the claims), on its own seat or one nobody
        // claimed, and first or second in the order.
        let sitting_there = st.room.as_ref().filter(|r| r.view.call_id == call_id && r.room_id == from_room_id);
        let (composition, owners): (BTreeSet<u32>, BTreeMap<u32, PubKey>) = match sitting_there {
            Some(room) => (room.composition.clone(), room.composition.iter().filter_map(|s| room.owner_of(*s, me).map(|o| (*s, o))).collect()),
            None => (from.seats.keys().copied().collect(), from.seats.clone()),
        };
        if !composition.contains(&seat) || owners.get(&seat).is_some_and(|o| o != author) {
            tracing::warn!(seat, "group call: a move by somebody not on that seat");
            return Ok(());
        }
        let order = move_order(&composition, &a.started_by, &owners);
        if !order.iter().take(2).any(|s| *s == seat) {
            tracing::warn!(seat, ?order, "group call: a move by neither the first nor the second of the room");
            return Ok(());
        }
        // Sitting well in the room left: the home is asked first, without
        // the lock. Answering, it is the author that had the trouble, and
        // its note counts as its leave.
        let well = sitting_there.is_some_and(|r| r.connected && r.lost.is_none() && !r.leaving);
        if well {
            let (gen, home) = sitting_there.map(|r| (r.gen, r.home.clone())).expect("sitting");
            let inner = self.clone();
            let (group_id, author, me) = (group_id.to_string(), author.clone(), me.clone());
            tokio::spawn(async move {
                let answered = tokio::time::timeout(inner.timing().hello_check, inner.rooms.hello(&home)).await.is_ok_and(|r| r.is_ok());
                let mut st = inner.state.lock().await;
                let still_well = st.room.as_ref().is_some_and(|r| r.gen == gen && r.connected && r.lost.is_none() && !r.leaving && r.ctl_open);
                if answered && still_well {
                    tracing::info!(seat, "group call: the home answers: the mover lost its way, not the room");
                    if let Some(a) = st.announced.get_mut(&call_id) {
                        if let Some(link) = a.link_mut(&from_room_id) {
                            if link.seats.get(&seat) == Some(&author) {
                                link.seats.remove(&seat);
                            }
                        }
                    }
                    if let Some(room) = st.room_of(&call_id) {
                        if let Some(p) = room.peers.get_mut(&seat) {
                            p.gone = true;
                        }
                    }
                    inner.emit_announced(&st, &call_id);
                    return;
                }
                inner.apply_move(&mut st, &group_id, &author, &me, sig, at).await;
            });
            return Ok(());
        }
        self.apply_move(st, group_id, author, me, sig, at).await;
        Ok(())
    }

    /// The move holds: the chain gets the room (over a losing move from
    /// the same room), the epoch is set by it, and I follow when I sat
    /// in the room left or in the loser's. Judged against a rival from
    /// the same room once more here: a move held for the HELLO of a
    /// participant sitting well is applied after whatever came
    /// meanwhile, and the newer must hold whichever check ends last.
    async fn apply_move(self: &Arc<Self>, st: &mut State, group_id: &str, author: &PubKey, me: &PubKey, sig: GroupSignal, at: i64) {
        let GroupSignal::Move { call_id, from_room_id, room_id, node, key, join_token, expires_at, epoch, secret, .. } = sig else { return };
        let Some(a) = st.announced.get(&call_id) else { return };
        if a.ended {
            return;
        }
        match judge_move(a, &from_room_id, &room_id, at) {
            MoveVerdict::Holds => {}
            MoveVerdict::Repeat => return,
            MoveVerdict::Lost(why) => {
                tracing::info!(room = %room_id, why, "group call: a move that does not hold");
                st.take_early(&call_id, Some(&room_id));
                return;
            }
        }
        let Some(a) = st.announced.get_mut(&call_id) else { return };
        let Some(pos) = a.rooms.iter().position(|l| l.room_id == from_room_id) else { return };
        let losers: Vec<String> = a.rooms.drain(pos + 1..).map(|l| l.room_id).collect();
        a.rooms.push(RoomLink {
            room_id: room_id.clone(),
            node: CallNode { node, class: NodeClass::Group, access_key: key },
            join_token,
            expires_at,
            from: Some(from_room_id.clone()),
            owner: author.clone(),
            said_at: at,
            seats: BTreeMap::new(),
            left: BTreeSet::new(),
        });
        // The move sets the epoch: its secret for its number, whatever a
        // losing move or a late rotation of the dead room put there, and
        // nothing above it — the one move that holds is applied alike
        // everywhere, so everybody ends on one secret.
        let changed = a.set_epoch(epoch, secret);
        let expires = a.expires();
        tracing::info!(room = %room_id, from = %from_room_id, epoch, by = %author.as_hex(), "group call: the room moved");
        for loser in &losers {
            st.take_early(&call_id, Some(loser));
        }
        self.schedule_expiry(call_id.clone(), expires);
        let sitting = st.room.as_ref().filter(|r| r.view.call_id == call_id).map(|r| r.room_id.clone());
        if let Some(where_i_sit) = sitting {
            if where_i_sit == room_id {
                // My own move: the chain was written when I made the room.
            } else {
                // In the room left, or in the loser's: out of it without
                // a word to anybody (the node is gone, or the room never
                // was), and into the room that holds.
                let members = st.room.as_ref().map(|r| r.members.clone()).unwrap_or_default();
                self.drop_session(st, Tell::Nobody).await;
                let link = st.announced.get(&call_id).map(|a| a.current().clone_link()).expect("just pushed");
                let gen = st.next_gen();
                let Some(room) = st.room.as_mut() else { return };
                room.gen = gen;
                room.room_id = room_id.clone();
                room.home = link.node.clone();
                room.node = link.node.clone();
                room.view.node = link.node.node.to_string();
                room.view.home = link.node.node.to_string();
                room.view.phase = GroupPhase::Joining;
                room.view.participants.clear();
                room.admin_token = None;
                room.claimed = false;
                room.lost = None;
                room.moving = false;
                room.members = members;
                let view = room.view.clone();
                self.emit(vec![state_event(&view)]);
                self.follow(room.gen);
            }
        } else if changed {
            // Not in the room: the banner learns its new address.
        }
        let early = st.take_early(&call_id, Some(&room_id));
        for (early_author, early, early_at) in early {
            let _ = self.on_signal_boxed(st, group_id, &early_author, me, early, early_at).await;
        }
        self.emit_announced(st, &call_id);
    }

    /// Into the current room of my call after a move (the room is set
    /// in the state, `Joining`), by the rule of the choice of node.
    fn follow(self: &Arc<Self>, gen: u64) {
        let inner = self.clone();
        tokio::spawn(async move {
            let Ok(keys) = inner.signer() else { return };
            let me = PubKey::parse(&keys.public_key().to_hex()).expect("a key is hex");
            let now = inner.clock.now().secs();
            let (call_id, group_id, home, room_id, token, secret, epoch, bad) = {
                let st = inner.state.lock().await;
                let Some(room) = st.room.as_ref().filter(|r| r.gen == gen) else { return };
                let Some(a) = st.announced.get(&room.view.call_id) else { return };
                let epoch = a.current_epoch();
                let Some(secret) = a.epochs.get(&epoch).copied() else { return };
                let current = a.current();
                (
                    a.call_id.clone(),
                    a.group_id.clone(),
                    current.node.clone(),
                    current.room_id.clone(),
                    current.join_token.clone(),
                    secret,
                    epoch,
                    st.bad_nodes.clone(),
                )
            };
            let plan = inner.plan_join(&group_id, &home, &room_id, &token, now, &bad).await;
            let mut st = inner.state.lock().await;
            if st.room.as_ref().is_none_or(|r| r.gen != gen) {
                return;
            }
            let plan = match plan {
                Ok(p) => p,
                Err(e) => {
                    inner.emit(vec![error_event(&e)]);
                    inner.leave_room(&mut st, Outcome::Failed, Tell::LeaveOnly).await;
                    return;
                }
            };
            match inner.enter_room(&mut st, &keys, &call_id, plan, &token, &secret, epoch, true).await {
                Ok(()) => inner.claim_seat(&mut st, &me, &call_id).await,
                Err(e) => {
                    inner.emit(vec![error_event(&e)]);
                    inner.leave_room(&mut st, Outcome::Failed, Tell::LeaveOnly).await;
                }
            }
        });
    }

    /// A sign the node may be lost (cascade.md, "Пропажа дома"): the way
    /// gone for [`LOST_AFTER`], `Failed` of the engine, my channel closed
    /// for as long, `home_lost` from my own node. The session is dropped
    /// (its node is gone or deaf), and the judgement from two points
    /// begins without the lock.
    async fn on_loss_sign(self: &Arc<Self>, st: &mut State, what: &str) {
        let Some(room) = st.room.as_mut() else { return };
        if room.leaving || room.lost.is_some() || !room.judges_loss() {
            return;
        }
        tracing::warn!(seat = room.seat, what, "group call: a sign the node may be lost");
        room.lost = Some(Loss::Judging);
        room.rejoins = 0;
        self.drop_session(st, Tell::Nobody).await;
        let Some(room) = st.room.as_mut() else { return };
        room.view.phase = GroupPhase::Reconnecting;
        let view = room.view.clone();
        self.emit(vec![state_event(&view)]);
        self.mend(room.gen, 0);
    }

    /// The session closed and the room given a new generation, so that
    /// nothing of the old session (its `Closed`, its timers) is read as
    /// news. The node is told when `tell` says so.
    async fn drop_session(&self, st: &mut State, tell: Tell) {
        let Some(room) = st.room.as_mut() else { return };
        if let Some(s) = room.session.take() {
            if tell != Tell::Nobody && !room.participant_token.is_empty() {
                let _ = self.rooms.leave(&room.node, &room.room_id, room.seat, &room.participant_token).await;
            }
            s.close().await;
        }
        room.connected = false;
        room.ctl_open = false;
        let gen = st.next_gen();
        if let Some(room) = st.room.as_mut() {
            room.gen = gen;
        }
    }

    /// The judgement from two points, and the join again (cascade.md,
    /// "Пропажа дома и повторный вход"): HELLO to the home and, when I
    /// sat there directly and have an own node with the cascade, a join
    /// into the same room through it; both at once. The join through my
    /// own node succeeding is the join again; the HELLO answering alone
    /// means a direct join again (my seat on the home given up by its
    /// token first, when I sat through my own node). Neither: the home is
    /// gone. `full_tries` counts the full rooms met.
    fn mend(self: &Arc<Self>, gen: u64, full_tries: u32) {
        let inner = self.clone();
        tokio::spawn(async move {
            let Ok(keys) = inner.signer() else { return };
            let me = PubKey::parse(&keys.public_key().to_hex()).expect("a key is hex");
            let now = inner.clock.now().secs();
            let (call_id, group_id, home, room_id, token, secret, epoch, media, simulcast, by_node, home_seat, bad) = {
                let st = inner.state.lock().await;
                let Some(room) = st.room.as_ref().filter(|r| r.gen == gen && r.lost == Some(Loss::Judging)) else { return };
                let Some(a) = st.announced.get(&room.view.call_id) else { return };
                let epoch = a.current_epoch();
                let Some(secret) = a.epochs.get(&epoch).copied() else { return };
                let current = a.current();
                (
                    a.call_id.clone(),
                    a.group_id.clone(),
                    room.home.clone(),
                    room.room_id.clone(),
                    current.join_token.clone(),
                    secret,
                    epoch,
                    room.view.media,
                    room.simulcast,
                    room.home_lost_by_node,
                    room.home_token.clone().map(|t| (room.seat, t)),
                    st.bad_nodes.clone(),
                )
            };
            let timing = inner.timing();
            let hello = async { tokio::time::timeout(timing.hello_check, inner.rooms.hello(&home)).await.is_ok_and(|r| r.is_ok()) };
            // My own node as the second point, when it has not judged the
            // home from its point already (`home_lost`): a join into the
            // same room through it. Bounded by `hello_check` like the
            // HELLO: the pass is asked of the home (a dead one answers
            // nothing until the connect times out, seconds later), and a
            // dead own node takes as long; the judgement is not to wait
            // for either. A join that ends late is closed, unwanted.
            let via_task = {
                let (inner, group_id, home, room_id, token, bad, call_id) =
                    (inner.clone(), group_id.clone(), home.clone(), room_id.clone(), token.clone(), bad.clone(), call_id.clone());
                tokio::spawn(async move {
                    if by_node {
                        return None;
                    }
                    let plan = inner.plan_join(&group_id, &home, &room_id, &token, now, &bad).await.ok()?;
                    let (own, pass) = plan.via?;
                    // The direct join is the HELLO's to decide: only the way
                    // through my own node is tried here.
                    let (session, offer) = inner.open_session(&call_id, media, simulcast, &own).await.ok()?;
                    match inner.rooms.join_via(&own.node, &home.node, &room_id, &pass, &offer).await {
                        Ok(joined) => Some(Ok(Entered { session, offer, joined, node: own.node.clone(), access: own })),
                        Err(e) => {
                            session.close().await;
                            Some(Err(e))
                        }
                    }
                })
            };
            let via = async move {
                let mut via_task = via_task;
                match tokio::time::timeout(timing.hello_check, &mut via_task).await {
                    Ok(Ok(outcome)) => outcome,
                    Ok(Err(_)) => None,
                    Err(_) => {
                        tracing::debug!("group call: the join through my own node did not end in time: not waited for");
                        tokio::spawn(async move {
                            if let Ok(Some(Ok(entered))) = via_task.await {
                                entered.session.close().await;
                            }
                        });
                        None
                    }
                }
            };
            let (answered, via) = tokio::join!(hello, via);
            let mut st = inner.state.lock().await;
            if st.room.as_ref().is_none_or(|r| r.gen != gen || r.lost != Some(Loss::Judging)) {
                if let Some(Ok(entered)) = via {
                    entered.session.close().await;
                }
                return;
            }
            let via_full = matches!(&via, Some(Err(e)) if e.is_room_full());
            let via_gone = matches!(&via, Some(Err(e)) if e.is_not_found());
            if let Some(Ok(entered)) = via {
                tracing::info!(seat = entered.joined.participant_id, "group call: joined again through my own node");
                inner.settle_rejoin(&mut st, &keys, &me, &call_id, entered, &secret, epoch).await;
                return;
            }
            if via_gone {
                inner.on_home_lost(&mut st, "the home has no such room").await;
                return;
            }
            if !answered {
                inner.on_home_lost(&mut st, "the home answers from no point").await;
                return;
            }
            // The home answers: my way was the trouble. Directly, my
            // seat on the home through my own node given up first.
            if let Some((seat, home_token)) = home_seat {
                let _ = inner.rooms.leave(&home, &room_id, seat, &home_token).await;
            }
            let access = match inner.nodes.access(&home, now).await {
                Ok(a) => a,
                Err(e) => {
                    tracing::warn!(error = %e, "group call: the home answered hello but gave no credentials");
                    inner.on_home_lost(&mut st, "no credentials from the home").await;
                    return;
                }
            };
            let plan = Plan { home: access, via: None };
            match inner.attempt_join(&call_id, &room_id, media, simulcast, &plan, &token).await {
                Ok(entered) => inner.settle_rejoin(&mut st, &keys, &me, &call_id, entered, &secret, epoch).await,
                Err(e) => {
                    let full = via_full || matches!(&e, MessengerError::Transport(t) if t.contains("room_full"));
                    let gone = matches!(&e, MessengerError::Transport(t) if t.contains("room_not_found"));
                    if gone {
                        inner.on_home_lost(&mut st, "the home has no such room").await;
                        return;
                    }
                    let Some(room) = st.room.as_mut() else { return };
                    if full && full_tries + 1 < REJOIN_FULL_TRIES {
                        // The home still reaps the seats of the dead way.
                        tracing::info!(tries = full_tries + 1, "group call: the room is full: once more in a moment");
                        inner.schedule(timing.rejoin_retry, Timer::Rejoin { full_tries: full_tries + 1 }, room.gen);
                        return;
                    }
                    room.rejoins += 1;
                    if room.rejoins >= REJOIN_TRIES {
                        inner.on_home_lost(&mut st, "no join again went through").await;
                    } else {
                        tracing::info!(error = %e, "group call: the join again failed: once more in a moment");
                        inner.schedule(timing.rejoin_retry, Timer::Rejoin { full_tries }, room.gen);
                    }
                }
            }
        });
    }

    /// The join again went through: the session and the seat set as at
    /// a first join, the seat told to the group when it is another.
    #[allow(clippy::too_many_arguments)]
    async fn settle_rejoin(self: &Arc<Self>, st: &mut State, keys: &Keys, me: &PubKey, call_id: &str, entered: Entered, secret: &Secret, epoch: u32) {
        let old_seat = st.room.as_ref().map(|r| r.seat).unwrap_or(0);
        let Entered { session, offer, joined, node, access } = entered;
        if let Err(e) = session.set_remote(&joined.sdp_answer, SdpKind::Answer).await {
            session.close().await;
            self.emit(vec![error_event(&e)]);
            self.on_home_lost(st, "the answer of the home was not taken").await;
            return;
        }
        let key = keys::sender_key(secret, call_id, joined.participant_id, epoch);
        let _ = session.set_sender_key(keys::slot(epoch), &key).await;
        let Some(room) = st.room.as_mut() else {
            session.close().await;
            return;
        };
        let gen = room.gen;
        self.pump(session.events(), gen);
        room.session = Some(session);
        room.seat = joined.participant_id;
        room.participant_token = joined.participant_token;
        room.home_token = joined.home_token;
        room.node = node.clone();
        room.view.node = node.node.to_string();
        room.view.limits = Some(access.welcome.limits.clone());
        room.simulcast = access.welcome.capabilities.iter().any(|c| c == CAP_SIMULCAST);
        room.dtls_fp = keys::dtls_fingerprint(&offer);
        room.sending = Some((epoch, *secret));
        room.peers.clear();
        room.composition = BTreeSet::from([room.seat]);
        room.connected = false;
        room.ever_connected = false;
        room.again = true;
        room.ctl_open = false;
        room.openings = 0;
        // A new session, judged anew: my own node's `home_lost` was of
        // the session before, and the next loss is judged from both
        // points again.
        room.home_lost_by_node = false;
        for id in joined.participants {
            self.seat_appeared(room, id);
        }
        room.view.phase = GroupPhase::Joining;
        room.view.epoch = epoch;
        room.refresh(&keys.public_key().to_hex());
        let view = room.view.clone();
        self.schedule(self.timing().rejoin_connect, Timer::Connect, gen);
        self.emit(vec![state_event(&view)]);
        if room.seat != old_seat {
            // Another seat of the same room: the old one's leave, the new
            // one's claim.
            let (group_id, room_id) = (room.view.group_id.clone(), room.room_id.clone());
            if room.claimed {
                self.tell_group(&group_id, &GroupSignal::Leave { call_id: call_id.to_string(), participant: old_seat, room_id: Some(room_id.clone()) }).await;
                if let Some(a) = st.announced.get_mut(call_id) {
                    if let Some(link) = a.link_mut(&room_id) {
                        if link.seats.get(&old_seat) == Some(me) {
                            link.seats.remove(&old_seat);
                        }
                    }
                }
            }
            self.claim_seat(st, me, call_id).await;
        }
    }

    /// The home is gone (cascade.md, "Переезд"): judged so from every
    /// point. The first of the room's last composition moves it, the
    /// second after [`MOVE_BACKUP`] without a move, the rest wait
    /// [`MOVE_WAIT`] and leave. The home is not chosen again for a while.
    async fn on_home_lost(self: &Arc<Self>, st: &mut State, why: &str) {
        let Some(me) = self.my_key() else { return };
        let Some(room) = st.room.as_ref() else { return };
        let (home, gen) = (room.home.node.clone(), room.gen);
        st.mark_bad(&home);
        let Some(room) = st.room.as_mut() else { return };
        if let Some(s) = room.session.take() {
            s.close().await;
        }
        room.lost = Some(Loss::HomeLost);
        room.connected = false;
        room.ctl_open = false;
        room.view.phase = GroupPhase::Reconnecting;
        let view = room.view.clone();
        self.emit(vec![state_event(&view)]);
        let creator = st.announced.get(&room.view.call_id).map(|a| a.started_by.clone()).unwrap_or_else(|| me.clone());
        let composition = room.composition.clone();
        let owners: BTreeMap<u32, PubKey> = composition.iter().filter_map(|s| room.owner_of(*s, &me).map(|o| (*s, o))).collect();
        let order = move_order(&composition, &creator, &owners);
        let rank = order.iter().position(|s| *s == room.seat);
        tracing::warn!(seat = room.seat, why, ?order, home = %home.id.short(), "group call: the home is gone");
        let timing = self.timing();
        match rank {
            Some(0) => self.make_move(gen),
            Some(1) => self.schedule(timing.move_backup, Timer::MoveBackup, gen),
            _ => self.schedule(timing.move_wait, Timer::MoveWait, gen),
        }
    }

    /// Move the room (cascade.md, "Переезд → Как"): a room on the nearest
    /// node of my sets but the home gone, me in it, the group told
    /// `call.move` with a new epoch, then `call.join`. Nothing to the
    /// group when it did not work out: I wait for the second's move like
    /// the rest ([`Self::wait_for_move`]), and leave `failed` only when
    /// there is no other node at all.
    fn make_move(self: &Arc<Self>, gen: u64) {
        let inner = self.clone();
        tokio::spawn(async move {
            let Ok(keys) = inner.signer() else { return };
            let me = PubKey::parse(&keys.public_key().to_hex()).expect("a key is hex");
            let now = inner.clock.now().secs();
            let (call_id, group_id, from_room_id, from_seat, from_home, from_admin, from_claimed, bad) = {
                let mut st = inner.state.lock().await;
                let bad = st.bad_nodes.clone();
                let Some(room) = st.room.as_mut().filter(|r| r.gen == gen && r.lost == Some(Loss::HomeLost) && !r.moving) else { return };
                room.moving = true;
                (
                    room.view.call_id.clone(),
                    room.view.group_id.clone(),
                    room.room_id.clone(),
                    room.seat,
                    room.home.clone(),
                    room.admin_token.clone(),
                    room.claimed,
                    bad,
                )
            };
            let candidates = inner.candidates(&group_id, &bad).await;
            let nobody = candidates.is_empty();
            let outcome: Result<(NodeAccess, RoomCreated)> = async {
                let access = inner
                    .nodes
                    .pick_sfu(&candidates, now)
                    .await
                    .ok_or_else(|| MessengerError::Transport("no other call node with an SFU answered".into()))?;
                let created = inner.rooms.create(&access.node, MediaLimits::default()).await?;
                Ok((access, created))
            }
            .await;
            let mut st = inner.state.lock().await;
            if st.room.as_ref().is_none_or(|r| r.gen != gen || r.lost != Some(Loss::HomeLost)) {
                // A move of somebody else came meanwhile and I follow it;
                // or I left. A room made for nothing ends by itself.
                return;
            }
            let (access, created) = match outcome {
                Ok(x) => x,
                Err(e) => {
                    tracing::warn!(error = %e, nobody, "group call: no room to move to");
                    inner.emit(vec![error_event(&e)]);
                    if nobody {
                        // A world of one node, as before the cascade.
                        inner.leave_room(&mut st, Outcome::Failed, Tell::LeaveOnly).await;
                    } else {
                        inner.wait_for_move(&mut st).await;
                    }
                    return;
                }
            };
            let secret = new_secret();
            let node = access.node.clone();
            let Some(a) = st.announced.get_mut(&call_id) else { return };
            let epoch = a.current_epoch() + 1;
            a.epochs.insert(epoch, secret);
            if let Some(pos) = a.rooms.iter().position(|l| l.room_id == from_room_id) {
                a.rooms.truncate(pos + 1);
            }
            a.rooms.push(RoomLink {
                room_id: created.room_id.clone(),
                node: node.clone(),
                join_token: created.join_token.clone(),
                expires_at: created.expires_at as i64,
                from: Some(from_room_id.clone()),
                owner: me.clone(),
                said_at: now,
                seats: BTreeMap::new(),
                left: BTreeSet::new(),
            });
            inner.schedule_expiry(call_id.clone(), created.expires_at as i64);
            let gen = st.next_gen();
            let Some(room) = st.room.as_mut() else { return };
            room.gen = gen;
            room.room_id = created.room_id.clone();
            room.home = node.clone();
            room.node = node.clone();
            room.admin_token = Some(created.admin_token.clone());
            room.view.node = node.node.to_string();
            room.view.home = node.node.to_string();
            room.view.kbps_per_participant = created.kbps_per_participant;
            room.view.max_participants = created.max_participants;
            room.view.phase = GroupPhase::Joining;
            room.view.participants.clear();
            room.claimed = false;
            room.lost = None;
            room.moving = false;
            room.simulcast = access.welcome.capabilities.iter().any(|c| c == CAP_SIMULCAST);
            let view = room.view.clone();
            inner.emit(vec![state_event(&view)]);
            let plan = Plan { home: access, via: None };
            if let Err(e) = inner.enter_room(&mut st, &keys, &call_id, plan, &created.join_token, &secret, epoch, true).await {
                tracing::warn!(error = %e, "group call: the room moved to did not take me");
                inner.emit(vec![error_event(&e)]);
                // The chain is mine alone so far: back to the room gone,
                // so that another's move from it still applies; and I
                // wait for it like the rest.
                if let Some(a) = st.announced.get_mut(&call_id) {
                    a.rooms.pop();
                    a.epochs.remove(&epoch);
                }
                if let Some(room) = st.room.as_mut() {
                    room.room_id = from_room_id;
                    room.seat = from_seat;
                    room.home = from_home.clone();
                    room.node = from_home.clone();
                    room.view.node = from_home.node.to_string();
                    room.view.home = from_home.node.to_string();
                    room.admin_token = from_admin;
                    // My seat there was claimed as it was: its `call.leave`
                    // is still owed when the wait ends in `failed`.
                    room.claimed = from_claimed;
                }
                inner.wait_for_move(&mut st).await;
                return;
            }
            if st.room.as_ref().is_none_or(|r| r.gen != gen) {
                return;
            }
            // My move is said now, as its note will say (`created_at` is
            // stamped when the note is sealed, after the room was made
            // and joined): what everybody else orders two moves by, so
            // what I order them by too.
            let said = inner.clock.now().secs();
            if let Some(a) = st.announced.get_mut(&call_id) {
                if let Some(link) = a.link_mut(&created.room_id) {
                    link.said_at = said;
                }
            }
            tracing::info!(room = %created.room_id, from = %from_room_id, epoch, node = %node.node.id.short(), "group call: I moved the room");
            let moved = GroupSignal::Move {
                call_id: call_id.clone(),
                from_room_id,
                room_id: created.room_id,
                node: node.node.clone(),
                key: node.access_key.clone(),
                join_token: created.join_token,
                expires_at: created.expires_at as i64,
                seat: from_seat,
                epoch,
                secret,
            };
            inner.tell_group(&group_id, &moved).await;
            inner.claim_seat(&mut st, &me, &call_id).await;
        });
    }

    /// My move did not work out (no node answered, the room was not made,
    /// it did not take me) while other nodes exist: the group hears
    /// nothing of it, and I wait for another's `call.move` like the rest
    /// of the room — the second moves on its timer — and leave `failed`
    /// when none comes in [`MOVE_WAIT`].
    async fn wait_for_move(self: &Arc<Self>, st: &mut State) {
        let Some(room) = st.room.as_mut() else { return };
        room.lost = Some(Loss::HomeLost);
        room.moving = false;
        room.view.phase = GroupPhase::Reconnecting;
        let (gen, view) = (room.gen, room.view.clone());
        tracing::info!(seat = room.seat, "group call: my move did not work out: waiting for another's");
        self.emit(vec![state_event(&view)]);
        self.schedule(self.timing().move_wait, Timer::MoveWait, gen);
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
        st.drop_early(loser);
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
        st.drop_early(call_id);
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
    /// last; only the node's word on who is left counts, and the word of
    /// a seat that said it left itself) when it is told at all, the
    /// screen told. After a sign of loss the node is not asked and the
    /// group hears `call.leave` alone.
    async fn leave_room(&self, st: &mut State, outcome: Outcome, tell: Tell) {
        let Some(mut room) = st.room.take() else { return };
        st.next_gen();
        room.leaving = true;
        let tell = match (tell, room.lost) {
            (Tell::All, Some(_)) => Tell::LeaveOnly,
            (t, _) => t,
        };
        let (call_id, group_id, seat, room_id) = (room.view.call_id.clone(), room.view.group_id.clone(), room.seat, room.room_id.clone());
        if let Some(s) = room.session.take() {
            if matches!(tell, Tell::All | Tell::Node) && !room.participant_token.is_empty() {
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
        if !matches!(tell, Tell::All | Tell::LeaveOnly) {
            if let Some(a) = st.announced.get_mut(&call_id) {
                a.left_at = Some(now);
            }
            return;
        }
        if room.claimed {
            self.tell_group(&group_id, &GroupSignal::Leave { call_id: call_id.clone(), participant: seat, room_id: Some(room_id.clone()) }).await;
            if let Some(a) = st.announced.get_mut(&call_id) {
                if let Some(link) = a.link_mut(&room_id) {
                    link.seats.remove(&seat);
                }
            }
        }
        if tell == Tell::All && room.nobody_left() {
            self.tell_group(&group_id, &GroupSignal::End { call_id: call_id.clone(), reason: reason::ENDED.into(), room_id: Some(room_id) }).await;
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
    /// to: its keys are spoiled), the owner of the room puts it out and
    /// changes the token of the room, and the keys turn, so that what it
    /// kept of the secrets opens nothing new. The banner loses it too.
    async fn check_members(self: &Arc<Self>, st: &mut State, group_id: &str, members: &[PubKey]) {
        let mut banners = vec![];
        for a in st.announced.values_mut().filter(|a| a.group_id == group_id && !a.ended) {
            let link = a.current_mut();
            let before = link.seats.len();
            link.seats.retain(|_, who| members.contains(who));
            if link.seats.len() != before {
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
        let (home, room_id, admin) = (room.home.clone(), room.room_id.clone(), room.admin_token.clone());
        let mut token = None;
        if let Some(admin) = admin {
            for seat in &expelled {
                if let Err(e) = self.rooms.leave(&home, &room_id, *seat, &admin).await {
                    self.emit(vec![error_event(&e.into())]);
                }
            }
            match self.rooms.change_token(&home, &room_id, &admin).await {
                Ok(t) => {
                    if let Some(a) = st.announced.get_mut(&call_id) {
                        if let Some(link) = a.link_mut(&room_id) {
                            link.join_token = t.clone();
                        }
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
                if room.lost == Some(Loss::Judging) {
                    // A join after a loss that did not connect.
                    if room.connected {
                        return;
                    }
                    let Some(room) = st.room.as_mut() else { return };
                    room.rejoins += 1;
                    let tries = room.rejoins;
                    tracing::warn!(seat = room.seat, tries, "group call: the join again did not connect in time");
                    self.drop_session(&mut st, Tell::Nobody).await;
                    if tries >= REJOIN_TRIES {
                        self.on_home_lost(&mut st, "two joins again without a way").await;
                    } else if let Some(room) = st.room.as_ref() {
                        self.mend(room.gen, 0);
                    }
                } else if !room.ever_connected && room.again && !room.leaving {
                    // A join after a move that never connected: the room
                    // and the call go on, so this is a sign of loss, judged
                    // from two points (the home answering, I join again;
                    // silent, it is gone and the room moves on), not a
                    // failed first join.
                    tracing::warn!(seat = room.seat, "group call: the join after the move did not connect in time");
                    self.on_loss_sign(&mut st, "the join after the move gave no way in time").await;
                } else if !room.ever_connected {
                    tracing::warn!(seat = room.seat, "group call: no way to the node in time");
                    self.emit(vec![error_event(&MessengerError::Transport("no way to the node in time".into()))]);
                    self.leave_room(&mut st, Outcome::Failed, Tell::All).await;
                }
            }
            Timer::Verify { seat } => {
                // Deaf, I judge nobody: the time of the seat runs again
                // from the opening of my channel.
                if !room.ctl_open || !room.peers.get(&seat).is_some_and(|p| !p.verified) {
                    return;
                }
                // Nobody, after its time. The owner puts it out (the node
                // says `left`, the keys turn then); without the owner,
                // the keys turn around it and it stays deaf.
                tracing::info!(seat, "group call: the seat said nothing in its time: nobody");
                if let Some(admin) = room.admin_token.clone() {
                    let (home, room_id) = (room.home.clone(), room.room_id.clone());
                    match self.rooms.leave(&home, &room_id, seat, &admin).await {
                        Ok(()) => return,
                        Err(e) => tracing::warn!(seat, error = %e, "group call: the node did not put the seat out"),
                    }
                }
                self.rotate(&mut st, None, false).await;
            }
            Timer::Switch { epoch } => self.switch_sending(&mut st, epoch).await,
            Timer::Hello { opening } => {
                if !room.ctl_open || room.openings != opening {
                    return;
                }
                let (gen, again) = (room.gen, room.somebody_unconfirmed());
                if again {
                    self.say_hello(&mut st).await;
                }
                self.schedule(self.timing().hello_retry, Timer::Hello { opening }, gen);
            }
            Timer::Lost { outage } => {
                // Of the latest outage alone: one that ended meanwhile (the
                // way came back and went again) has its own timer.
                if !room.connected && room.outages == outage {
                    self.on_loss_sign(&mut st, "the way has been gone for a while").await;
                }
            }
            Timer::CtlLost { closing } => {
                if !room.ctl_open && room.closings == closing {
                    self.on_loss_sign(&mut st, "the control channel has been closed for a while").await;
                }
            }
            Timer::Rejoin { full_tries } => {
                if room.lost == Some(Loss::Judging) {
                    self.mend(gen, full_tries);
                }
            }
            Timer::MoveBackup => {
                if room.lost == Some(Loss::HomeLost) && !room.moving {
                    tracing::info!(seat = room.seat, "group call: no move came from the first: I move the room");
                    self.make_move(gen);
                }
            }
            Timer::MoveWait => {
                if room.lost == Some(Loss::HomeLost) {
                    tracing::warn!(seat = room.seat, "group call: no move came: out");
                    self.emit(vec![error_event(&MessengerError::Transport("the node of the room is gone and nobody moved it".into()))]);
                    self.leave_room(&mut st, Outcome::Failed, Tell::LeaveOnly).await;
                }
            }
        }
    }

    async fn on_engine_event(self: &Arc<Self>, gen: u64, ev: SessionEvent) {
        let mut st = self.state.lock().await;
        let Some(room) = st.room.as_mut().filter(|r| r.gen == gen) else { return };
        let me_hex = self.me().unwrap_or_default();
        match ev {
            SessionEvent::ConnectionState(state) => {
                use crate::engine::ConnectionState as C;
                tracing::debug!(seat = room.seat, ?state, "group call: the way to the node");
                match state {
                    C::Connected => {
                        room.connected = true;
                        room.ever_connected = true;
                        room.lost = None;
                        room.rejoins = 0;
                        room.view.phase = GroupPhase::InRoom;
                        let view = room.view.clone();
                        self.emit(vec![state_event(&view)]);
                    }
                    C::Disconnected => {
                        room.connected = false;
                        room.outages += 1;
                        if room.view.phase == GroupPhase::InRoom {
                            room.view.phase = GroupPhase::Reconnecting;
                            let view = room.view.clone();
                            self.emit(vec![state_event(&view)]);
                        }
                        if room.judges_loss() && !room.leaving && room.lost.is_none() {
                            self.schedule(self.timing().lost_after, Timer::Lost { outage: room.outages }, room.gen);
                        }
                    }
                    C::Failed => {
                        if room.judges_loss() && !room.leaving {
                            self.on_loss_sign(&mut st, "the engine gave the way up").await;
                        } else {
                            self.emit(vec![error_event(&MessengerError::Transport("the way to the node is lost".into()))]);
                            self.leave_room(&mut st, Outcome::Failed, Tell::All).await;
                        }
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
            SessionEvent::DataOpen { label } if label == CTL_LABEL => self.channel_opened(&mut st).await,
            SessionEvent::DataClosed { label } if label == CTL_LABEL => self.channel_closed(&mut st),
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
                room.composition = BTreeSet::from([room.seat]);
                for id in participants {
                    self.seat_appeared(room, id);
                }
                room.refresh(me_hex);
                let view = room.view.clone();
                self.emit(vec![state_event(&view)]);
            }
            Message::Joined { id } => {
                if id != room.seat {
                    let seat_name = st.announced.get(&room.view.call_id).and_then(|a| a.link(&room.room_id)).and_then(|l| l.seats.get(&id).cloned());
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
                room.composition.remove(&id);
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
            Message::HomeLost => {
                // My own node lost the home: it has judged from its point;
                // mine is the HELLO. A close of the channel follows and is
                // no news (the session is dropped here).
                room.home_lost_by_node = true;
                self.on_loss_sign(st, "my own node lost the home").await;
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
                tracing::debug!(from, epoch, held = secret.is_some(), "group call: a word of identity kept: not under the secret held for its epoch");
                if let Some(p) = room.peers.get_mut(&from) {
                    p.pending_hello = Some(bytes.to_vec());
                }
                return;
            }
            Some(Err(e)) => {
                tracing::warn!(from, epoch, ?e, "group call: a word of identity refused");
                return;
            }
        };
        if let Some(p) = room.peers.get_mut(&from) {
            p.pending_hello = None;
        }
        let Some(npub) = keys::pubkey_of(&hello) else { return };
        let members = self.groups.members(&group_id).await.unwrap_or_default();
        if !members.contains(&npub) {
            // A seat that is not a member of the group: never shown, never
            // listened to.
            tracing::warn!(from, npub = %npub.as_hex(), "group call: a word of identity of somebody who is no member here");
            return;
        }
        let Some(room) = st.room.as_mut() else { return };
        let Some(peer) = room.peers.get_mut(&from) else { return };
        let first = !peer.verified;
        peer.npub = Some(npub.clone());
        peer.verified = true;
        peer.expelled = false;
        peer.gone = false;
        tracing::info!(from, npub = %npub.as_hex(), first, "group call: the seat is confirmed");
        if let Some(a) = st.announced.get_mut(&call_id) {
            // The word of identity is the last word on who sits there.
            if let Some(link) = a.link_mut(&room_id) {
                link.seats.insert(from, npub);
            }
        }
        room.refresh(me_hex);
        let view = room.view.clone();
        self.key_everything(st).await;
        self.count_people(st, &call_id).await;
        self.emit(vec![state_event(&view)]);
        // Every word of a seat is answered with mine: the node says
        // `joined` before the newcomer's channel is open and relays
        // nothing to a channel that is not, so the word sent on `joined`
        // never reaches it; its own first word proves its channel is open
        // now, and is answered at once. A word said again is a seat that
        // has not confirmed me yet (my earlier word was lost on the way,
        // or sent under a secret it did not hold): answered too, a
        // quarter of `hello_retry` apart at least.
        if first {
            self.say_hello(st).await;
        } else {
            self.answer_hello(st).await;
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

impl RoomLink {
    /// The address of the room: what a follower needs of it.
    fn clone_link(&self) -> RoomLink {
        RoomLink {
            room_id: self.room_id.clone(),
            node: self.node.clone(),
            join_token: self.join_token.clone(),
            expires_at: self.expires_at,
            from: self.from.clone(),
            owner: self.owner.clone(),
            said_at: self.said_at,
            seats: BTreeMap::new(),
            left: BTreeSet::new(),
        }
    }
}

impl From<NodeError> for Outcome {
    fn from(_: NodeError) -> Self {
        Outcome::Failed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u8) -> PubKey {
        PubKey::parse(&format!("{n:02x}").repeat(32)).unwrap()
    }

    #[test]
    fn the_order_of_the_move_puts_the_creator_first() {
        let composition: BTreeSet<u32> = [2, 3, 5].into_iter().collect();
        let owners: BTreeMap<u32, PubKey> = [(2, key(2)), (3, key(1)), (5, key(5))].into_iter().collect();
        // The creator (key 1) sits on 3: first; then by number.
        assert_eq!(move_order(&composition, &key(1), &owners), vec![3, 2, 5]);
        // The creator is not in the room: the smallest seat first.
        assert_eq!(move_order(&composition, &key(9), &owners), vec![2, 3, 5]);
        assert_eq!(move_order(&BTreeSet::new(), &key(1), &owners), Vec::<u32>::new());
    }
}
