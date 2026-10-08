// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A call node with no network: the rooms of an SFU as the protocol has
//! them (services/call/spec/protocol.md, "Комнаты"), kept in a map, and
//! the control channel `ctl` spoken into the room sessions of a
//! [`FakeEngine`]. Plus the door to the groups a group call needs
//! ([`FakeGroups`]): members in a map, notes carried in the open.
//!
//! `join` reads the session out of the fake offer (`fake-offer:<id>:…`),
//! gives it a seat and speaks to it as the node would: `hello` with who
//! is there, `joined` to the others, an offer with the m-lines of the
//! others to the newcomer and one with the newcomer's to each of them
//! (`a<seat>` for audio, `v<seat>` for a video session). A binary frame
//! a session sends on `ctl` goes to every other seat of its room with the
//! seat in front; a text answer is kept for the tests to look at.
//!
//! One `FakeNode` stands for **several nodes** (the cascade,
//! services/call/spec/cascade.md): the first is there from the start
//! (`reference`, `as_call_node`), more come with `add_node`. Every room
//! is on one node; a join through another node with `home` (`join_via`)
//! seats the session in the home's room as the real node would, by a
//! pass from `delegate`. A node may be killed (`kill`): its rooms are
//! gone, whoever sat there directly is `Disconnected`, whoever sat there
//! through another node hears `home_lost` and is closed; a node of the
//! wave before has no `delegate` and no `cascade`; a silent node
//! (`set_silent`) answers no request, ever: the client waits out its own
//! timeouts, as with a VM gone whose packets are black-holed.

use crate::fake_engine::{RoomHook, NODE_ANSWER, NODE_OFFER};
use crate::FakeEngine;
use async_trait::async_trait;
use messenger_calls::engine::{ConnectionState, DataPayload, Media, SessionEvent, CTL_LABEL};
use messenger_calls::group::ctl::{self, Message, Track};
use messenger_calls::node_client::{
    Delegated, Joined, Limits, MediaLimits, NodeClient, NodeError, RoomApi, RoomCreated, TurnCredentials, Welcome, CAP_CASCADE, CAP_SFU,
};
use messenger_calls::servers::{CallNode, NodeClass, NodeRef};
use messenger_calls::GroupAccess;
use messenger_core::outbound::{Scope, WireEvent};
use messenger_core::{Envelope, EventId, MessengerError, Outbound, PubKey, Result};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

struct Seat {
    session: u32,
    token: String,
    video: bool,
    /// Offers made to this seat so far (`seq` of the next is one more).
    offers: u32,
    /// The seat's `ctl` is open: the node writes to it. In the real order
    /// (`set_real_ctl_order`) a newcomer's channel opens after its join
    /// (`open_ctl`), and a frame to a closed channel is dropped.
    ctl_open: bool,
    /// The offer of the others' tracks, kept while the channel is closed.
    pending_offer: Vec<Track>,
    /// The seat sits through this other node (its id): a proxy seat of
    /// that node on the home.
    via: Option<String>,
    /// The token of the proxy seat on the home, given to the client.
    home_token: Option<String>,
    /// The node's words no longer reach the seat (a stalled channel).
    frozen: bool,
}

struct FakeRoom {
    /// The id of the node the room is on.
    node: String,
    join_token: String,
    admin_token: String,
    next_seat: u32,
    seats: BTreeMap<u32, Seat>,
    expires_at: u64,
    /// Live passes of seats (`delegate`), each good once.
    passes: Vec<String>,
}

/// One of the nodes this fake stands for.
struct NodeInfo {
    reference: NodeRef,
    /// Seats its participants in the rooms of other nodes.
    cascade: bool,
    /// Of the wave before the cascade: no `delegate`, no `cascade`.
    wave4: bool,
    dead: bool,
    /// Its requests never answer (a VM gone with its packets black-holed:
    /// nothing says RST, the client waits out its own timeouts).
    silent: bool,
    rtt_ms: u64,
    /// HELLOs answered (the checks of the core).
    hellos: u32,
}

#[derive(Default)]
struct NodeState {
    next: u32,
    nodes: Vec<NodeInfo>,
    rooms: HashMap<String, FakeRoom>,
    /// The next request is refused so.
    refuse_next: Option<NodeError>,
    /// The next join is refused so (the room is made, the door is shut).
    refuse_next_join: Option<NodeError>,
    /// The next join through a node (`join_via`) is refused so.
    refuse_next_via: Option<NodeError>,
    /// The next room is made only when this gate is opened.
    hold_create: Option<Arc<tokio::sync::Notify>>,
    /// Answers the sessions gave to offers: session, `seq`.
    answers: Vec<(u32, u32)>,
    /// Texts of the sessions that are not answers (a layer request).
    texts: Vec<(u32, String)>,
    /// The node references the rooms were made on.
    created_on: Vec<String>,
    /// Passes asked for: the home, the room.
    delegated: Vec<(String, String)>,
    max_participants: u32,
    /// The node's clock when a room is made: its `expires_at` is twelve
    /// hours from here (the lifetime of a room of the protocol).
    now: u64,
    /// The order of the real node: the others hear `joined` before the
    /// newcomer's `ctl` is open, and nothing is relayed to a closed
    /// channel; the test opens the channel with `open_ctl`.
    real_ctl_order: bool,
    /// Binary frames of one seat that are lost on the way
    /// (`drop_relayed`): of those with somebody to hear them, so many are
    /// passed first, then so many dropped.
    relay_from: u32,
    relay_pass: u32,
    relay_drop: u32,
}

/// A fake call node (or several): the rooms, and the `ctl` of their
/// participants.
#[derive(Clone)]
pub struct FakeNode {
    state: Arc<Mutex<NodeState>>,
    engine: FakeEngine,
}

fn hex_id(n: u32, len: usize) -> String {
    format!("{n:02x}").repeat(len / 2)
}

/// The reference of the node number `i` (from 0): its own address and id.
fn reference_of(i: usize) -> NodeRef {
    format!("203.0.113.{}:8443#{}", 7 + i, format!("{:02x}", 0xfa + i as u32).repeat(32)).parse().expect("a node reference")
}

impl FakeNode {
    /// A node whose rooms are joined by the sessions of `engine`.
    pub fn new(engine: FakeEngine) -> Self {
        let first = NodeInfo { reference: reference_of(0), cascade: true, wave4: false, dead: false, silent: false, rtt_ms: 20, hellos: 0 };
        let node = Self {
            state: Arc::new(Mutex::new(NodeState { max_participants: 12, now: 1_760_000_000, nodes: vec![first], ..NodeState::default() })),
            engine: engine.clone(),
        };
        engine.set_room_hook(Arc::new(node.clone()));
        node
    }

    /// The reference of the first node, as the sets of servers list it.
    pub fn reference(&self) -> NodeRef {
        reference_of(0)
    }

    /// The first node in the sets of servers, as the project's.
    pub fn as_call_node(&self) -> CallNode {
        CallNode::new(self.reference(), NodeClass::Project)
    }

    /// One more node, as the project's: its index (the first is 0).
    pub fn add_node(&self) -> usize {
        let mut st = self.state.lock().unwrap();
        let i = st.nodes.len();
        st.nodes.push(NodeInfo { reference: reference_of(i), cascade: true, wave4: false, dead: false, silent: false, rtt_ms: 20, hellos: 0 });
        i
    }

    /// The node `i` in the sets of servers, of `class`.
    pub fn node(&self, i: usize, class: NodeClass) -> CallNode {
        CallNode::new(reference_of(i), class)
    }

    /// Whether the node `i` seats its participants in the rooms of other
    /// nodes (`cascade` in its capabilities; on by default).
    pub fn set_cascade(&self, i: usize, on: bool) {
        self.state.lock().unwrap().nodes[i].cascade = on;
    }

    /// The node `i` is of the wave before the cascade: a 404 page to
    /// `delegate`, no `cascade` in its capabilities.
    pub fn set_wave4(&self, i: usize, on: bool) {
        self.state.lock().unwrap().nodes[i].wave4 = on;
    }

    /// The round trip to the node `i`, as the client measures it.
    pub fn set_rtt(&self, i: usize, ms: u64) {
        self.state.lock().unwrap().nodes[i].rtt_ms = ms;
    }

    /// HELLOs the node `i` answered.
    pub fn hellos(&self, i: usize) -> u32 {
        self.state.lock().unwrap().nodes[i].hellos
    }

    /// The node `i` answers no request from now on (HELLO, rooms, a pass,
    /// a join through it): the request hangs until the client's own
    /// timeout, as with a VM gone whose packets are black-holed. Nothing
    /// else changes: its rooms stay, nobody is `Disconnected` (with
    /// `kill` after it, the node is gone and silent at once). A join
    /// through another node into a room of a silent home is refused
    /// `cascade_unreachable` at once (that node's own HELLO to it fails).
    pub fn set_silent(&self, i: usize, on: bool) {
        self.state.lock().unwrap().nodes[i].silent = on;
    }

    /// Waits forever when `node` is silent; otherwise returns at once.
    async fn gate(&self, node: &NodeRef) {
        let silent = self.state.lock().unwrap().nodes.iter().any(|n| n.reference.id == node.id && n.silent);
        if silent {
            std::future::pending::<()>().await;
        }
    }

    fn is_silent(st: &NodeState, node: &NodeRef) -> bool {
        Self::info(st, node).is_some_and(|n| n.silent)
    }

    /// The node `i` dies: its rooms are gone; whoever sat in them
    /// directly is `Disconnected` (the engine would say so after its
    /// checks), whoever sat through another node hears `home_lost` and
    /// is closed; whoever sat in a room of another node through this one
    /// is `Disconnected` and its proxy seat on the home reaped. Every
    /// request to it fails from now on.
    pub fn kill(&self, i: usize) {
        let mut st = self.state.lock().unwrap();
        let id = reference_of(i).id.to_string();
        st.nodes[i].dead = true;
        let dead_rooms: Vec<String> = st.rooms.iter().filter(|(_, r)| r.node == id).map(|(k, _)| k.clone()).collect();
        for room_id in dead_rooms {
            let Some(room) = st.rooms.remove(&room_id) else { continue };
            for seat in room.seats.values() {
                if seat.via.is_some() {
                    self.text(&st, seat.session, &Message::HomeLost);
                    self.engine.inject_into(seat.session, SessionEvent::ConnectionState(ConnectionState::Closed));
                } else {
                    self.engine.inject_into(seat.session, SessionEvent::ConnectionState(ConnectionState::Disconnected));
                }
            }
        }
        let orphans: Vec<(String, u32, u32)> = st
            .rooms
            .iter()
            .flat_map(|(rid, r)| r.seats.iter().filter(|(_, s)| s.via.as_deref() == Some(&id)).map(move |(seat, s)| (rid.clone(), *seat, s.session)))
            .collect();
        for (room_id, seat, session) in orphans {
            self.engine.inject_into(session, SessionEvent::ConnectionState(ConnectionState::Disconnected));
            self.remove_seat(&mut st, &room_id, seat);
        }
    }

    /// A node client whose HELLO every node of this fake answers with its
    /// capabilities and round trip (an SFU, the cascade unless taken
    /// away), with credentials of a TURN that is not there: nothing of
    /// the network is touched. A node this fake does not know answers as
    /// the first does.
    pub fn client(&self) -> NodeClient {
        let state = self.state.clone();
        NodeClient::with_fetch(
            "test",
            Arc::new(move |node, _| {
                let state = state.clone();
                Box::pin(async move {
                    let (caps, rtt) = {
                        let st = state.lock().unwrap();
                        let info = st.nodes.iter().find(|n| n.reference.id == node.node.id);
                        // A silent node answers nothing the cache does not
                        // hold: told as dead here, so that a pick of nodes
                        // is not held up by it.
                        if info.is_some_and(|n| n.dead || n.silent) {
                            return Err(MessengerError::Transport("fake node: dead".into()));
                        }
                        let mut caps = vec!["stun".to_string(), "turn".to_string(), CAP_SFU.to_string()];
                        if info.is_none_or(|n| n.cascade && !n.wave4) {
                            caps.push(CAP_CASCADE.to_string());
                        }
                        (caps, info.map(|n| n.rtt_ms).unwrap_or(20))
                    };
                    let welcome = Welcome {
                        protocol_min: 1,
                        protocol_max: 1,
                        node_id: node.node.id.to_string(),
                        version: "fake".into(),
                        capabilities: caps,
                        codecs: vec!["opus".into(), "vp8".into()],
                        private: node.access_key.is_some(),
                        limits: Limits { turn_lifetime_secs: 600, turn_kbps_per_allocation: 2000, credentials_ttl_secs: 600 },
                    };
                    let credentials = TurnCredentials {
                        username: "1760000600:fake".into(),
                        password: "pw".into(),
                        realm: "veydan".into(),
                        ttl_secs: 600,
                        expires_at: 1_760_000_600,
                        urls: vec![format!("turn:{}?transport=udp", node.node.addr)],
                    };
                    Ok((welcome, credentials, std::time::Duration::from_millis(rtt)))
                })
            }),
        )
    }

    /// Refuse the next request with `error`.
    pub fn refuse_next(&self, error: NodeError) {
        self.state.lock().unwrap().refuse_next = Some(error);
    }

    /// Refuse the next join with `error`, whatever else is asked before.
    pub fn refuse_next_join(&self, error: NodeError) {
        self.state.lock().unwrap().refuse_next_join = Some(error);
    }

    /// Refuse the next join through a node (`join_via`) with `error`.
    pub fn refuse_next_via(&self, error: NodeError) {
        self.state.lock().unwrap().refuse_next_via = Some(error);
    }

    /// The next room is made only when the gate given is opened
    /// (`notify_one`): a slow node, and what the group says meanwhile
    /// reaches the creator while it waits.
    pub fn hold_next_create(&self) -> Arc<tokio::sync::Notify> {
        let gate = Arc::new(tokio::sync::Notify::new());
        self.state.lock().unwrap().hold_create = Some(gate.clone());
        gate
    }

    /// The token a room is joined with now (the creator may change it).
    pub fn join_token(&self, room_id: &str) -> Option<String> {
        self.state.lock().unwrap().rooms.get(room_id).map(|r| r.join_token.clone())
    }

    /// The newcomers' `ctl` opens only when the test says so (`open_ctl`),
    /// as on the real node, where `joined` reaches the others before the
    /// newcomer's channel is open and a frame to a closed channel is
    /// dropped (the word of identity the others send on `joined` never
    /// reaches the newcomer).
    pub fn set_real_ctl_order(&self, on: bool) {
        self.state.lock().unwrap().real_ctl_order = on;
    }

    /// The seat's `ctl` opens now: `DataOpen`, the node's `hello`, and the
    /// offer of the others' tracks that waited for the channel.
    pub fn open_ctl(&self, room_id: &str, seat: u32) {
        let mut st = self.state.lock().unwrap();
        let Some(room) = st.rooms.get_mut(room_id) else { return };
        let participants: Vec<u32> = room.seats.keys().filter(|k| **k != seat).copied().collect();
        let Some(s) = room.seats.get_mut(&seat) else { return };
        if s.ctl_open {
            return;
        }
        s.ctl_open = true;
        let session = s.session;
        let tracks = std::mem::take(&mut s.pending_offer);
        self.engine.inject_into(session, SessionEvent::DataOpen { label: CTL_LABEL.into() });
        self.text(&st, session, &Message::Hello { you: seat, participants });
        self.offer(&mut st, room_id, seat, tracks);
    }

    /// The node's words (`joined`, `left`, offers) no longer reach the
    /// seat of `session`: its channel stalled.
    pub fn freeze_ctl(&self, session: u32) {
        let mut st = self.state.lock().unwrap();
        for r in st.rooms.values_mut() {
            for s in r.seats.values_mut() {
                if s.session == session {
                    s.frozen = true;
                }
            }
        }
    }

    /// Of the binary frames the seat `from` sends from now on that have
    /// somebody to hear them (a channel open), the next `pass` are
    /// relayed and the `drop` after them are lost on the way (a stalled
    /// channel, the node's limit on frames): a word of identity that
    /// never arrives.
    pub fn drop_relayed(&self, from: u32, pass: u32, drop: u32) {
        let mut st = self.state.lock().unwrap();
        st.relay_from = from;
        st.relay_pass = pass;
        st.relay_drop = drop;
    }

    pub fn set_max_participants(&self, n: u32) {
        self.state.lock().unwrap().max_participants = n;
    }

    /// The node's clock, unix seconds: the rooms made from now on expire
    /// twelve hours after it (the tests of the core keep their own clock;
    /// a runtime on the system clock sets this to it).
    pub fn set_now(&self, unix: u64) {
        self.state.lock().unwrap().now = unix;
    }

    pub fn rooms(&self) -> Vec<String> {
        let mut out: Vec<String> = self.state.lock().unwrap().rooms.keys().cloned().collect();
        out.sort();
        out
    }

    /// The rooms on the node `i`, in order.
    pub fn rooms_on(&self, i: usize) -> Vec<String> {
        let id = reference_of(i).id.to_string();
        let mut out: Vec<String> = self.state.lock().unwrap().rooms.iter().filter(|(_, r)| r.node == id).map(|(k, _)| k.clone()).collect();
        out.sort();
        out
    }

    /// The seats of a room, in order, with their sessions.
    pub fn seats(&self, room_id: &str) -> Vec<(u32, u32)> {
        self.state.lock().unwrap().rooms.get(room_id).map(|r| r.seats.iter().map(|(s, p)| (*s, p.session)).collect()).unwrap_or_default()
    }

    /// The seats of a room with the index of the node each sits through,
    /// when it does not sit on the home directly.
    pub fn seats_via(&self, room_id: &str) -> Vec<(u32, Option<usize>)> {
        let st = self.state.lock().unwrap();
        let index = |id: &str| st.nodes.iter().position(|n| n.reference.id.to_string() == id);
        st.rooms.get(room_id).map(|r| r.seats.iter().map(|(s, p)| (*s, p.via.as_deref().and_then(index))).collect()).unwrap_or_default()
    }

    pub fn answers(&self) -> Vec<(u32, u32)> {
        self.state.lock().unwrap().answers.clone()
    }

    pub fn texts(&self) -> Vec<(u32, String)> {
        self.state.lock().unwrap().texts.clone()
    }

    pub fn created_on(&self) -> Vec<String> {
        self.state.lock().unwrap().created_on.clone()
    }

    /// Passes asked of the homes: (home reference, room).
    pub fn delegated(&self) -> Vec<(String, String)> {
        self.state.lock().unwrap().delegated.clone()
    }

    /// The node ends the room: every seat's connection is closed.
    pub fn end_room(&self, room_id: &str) {
        let sessions: Vec<u32> = {
            let mut st = self.state.lock().unwrap();
            st.rooms.remove(room_id).map(|r| r.seats.values().map(|s| s.session).collect()).unwrap_or_default()
        };
        for s in sessions {
            self.engine.inject_into(s, SessionEvent::ConnectionState(ConnectionState::Closed));
        }
    }

    /// The node says who speaks in the room of `session`.
    pub fn speaking(&self, room_id: &str, ids: Vec<u32>) {
        let st = self.state.lock().unwrap();
        let sessions: Vec<u32> = st.rooms.get(room_id).map(|r| r.seats.values().map(|s| s.session).collect()).unwrap_or_default();
        for s in sessions {
            self.text(&st, s, &Message::Speaking { participants: ids.clone() });
        }
    }

    fn text(&self, st: &NodeState, session: u32, msg: &Message) {
        if st.rooms.values().any(|r| r.seats.values().any(|s| s.session == session && s.frozen)) {
            return;
        }
        self.engine.inject_into(session, SessionEvent::Data { label: CTL_LABEL.into(), payload: DataPayload::Text(msg.encode()) });
    }

    fn tracks_of(seat: u32, video: bool) -> Vec<Track> {
        let mut out = vec![Track { id: seat, kind: "audio".into(), mid: format!("a{seat}") }];
        if video {
            out.push(Track { id: seat, kind: "video".into(), mid: format!("v{seat}") });
        }
        out
    }

    /// An offer to the seat `to` of `room` with `tracks`.
    fn offer(&self, st: &mut NodeState, room_id: &str, to: u32, tracks: Vec<Track>) {
        if tracks.is_empty() {
            return;
        }
        let Some(room) = st.rooms.get_mut(room_id) else { return };
        let Some(seat) = room.seats.get_mut(&to) else { return };
        seat.offers += 1;
        let seq = seat.offers;
        let session = seat.session;
        self.text(st, session, &Message::Offer { seq, sdp: format!("{NODE_OFFER}{seq}"), tracks: tracks.clone() });
        for t in tracks {
            let kind = if t.kind == "video" { Media::Video } else { Media::Audio };
            self.engine.inject_into(session, SessionEvent::RemoteTrack { mid: t.mid, kind });
        }
    }

    fn take_refusal(st: &mut NodeState) -> std::result::Result<(), NodeError> {
        match st.refuse_next.take() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// A seat is gone (by request, or its connection closed): the others
    /// are told, its m-lines closed with them.
    fn remove_seat(&self, st: &mut NodeState, room_id: &str, seat: u32) {
        let Some(room) = st.rooms.get_mut(room_id) else { return };
        let Some(gone) = room.seats.remove(&seat) else { return };
        let others: Vec<u32> = room.seats.values().map(|s| s.session).collect();
        for s in others {
            self.text(st, s, &Message::Left { id: seat });
            self.engine.inject_into(s, SessionEvent::RemoteTrackGone { mid: format!("a{seat}") });
            if gone.video {
                self.engine.inject_into(s, SessionEvent::RemoteTrackGone { mid: format!("v{seat}") });
            }
        }
    }

    fn seat_of(st: &NodeState, session: u32) -> Option<(String, u32)> {
        st.rooms.iter().find_map(|(id, r)| r.seats.iter().find(|(_, s)| s.session == session).map(|(seat, _)| (id.clone(), *seat)))
    }

    fn info<'a>(st: &'a NodeState, node: &NodeRef) -> Option<&'a NodeInfo> {
        st.nodes.iter().find(|n| n.reference.id == node.id)
    }

    fn alive(st: &NodeState, node: &NodeRef) -> std::result::Result<(), NodeError> {
        match Self::info(st, node) {
            Some(n) if n.dead => Err(NodeError::Unreachable("fake node: dead".into())),
            _ => Ok(()),
        }
    }

    /// Seat `session` in `room_id`: the node's words to it and the
    /// others, the offers both ways.
    fn seat(&self, st: &mut NodeState, room_id: &str, session: u32, via: Option<String>) -> std::result::Result<Joined, NodeError> {
        let max = st.max_participants;
        let video = self.engine.media_of(session) == Some(Media::Video);
        let room = st.rooms.get_mut(room_id).ok_or_else(|| refused(404, "room_not_found"))?;
        if room.seats.len() as u32 >= max {
            return Err(refused(409, "room_full"));
        }
        room.next_seat += 1;
        let seat = room.next_seat;
        let others: Vec<(u32, bool)> = room.seats.iter().map(|(s, p)| (*s, p.video)).collect();
        let token = format!("seat-{seat}");
        let home_token = via.as_ref().map(|_| format!("home-{seat}"));
        let real_order = st.real_ctl_order;
        let room = st.rooms.get_mut(room_id).expect("the room is there");
        room.seats.insert(
            seat,
            Seat { session, token: token.clone(), video, offers: 0, ctl_open: !real_order, pending_offer: vec![], via, home_token: home_token.clone(), frozen: false },
        );
        let joined = Joined {
            sdp_answer: format!("{NODE_ANSWER}{seat}"),
            participant_id: seat,
            participant_token: token,
            participants: others.iter().map(|(s, _)| *s).collect(),
            home_token,
        };
        // The channel opens, the node says hello, the others hear of the
        // newcomer, and the offers go both ways. In the real order the
        // others hear `joined` now and the newcomer's channel opens later
        // (`open_ctl`); a seat whose channel is closed hears nothing.
        if !real_order {
            self.engine.inject_into(session, SessionEvent::DataOpen { label: CTL_LABEL.into() });
            self.text(st, session, &Message::Hello { you: seat, participants: joined.participants.clone() });
        }
        let other_sessions: Vec<(u32, u32)> =
            st.rooms[room_id].seats.iter().filter(|(s, p)| **s != seat && p.ctl_open).map(|(s, p)| (*s, p.session)).collect();
        for (_, s) in &other_sessions {
            self.text(st, *s, &Message::Joined { id: seat });
        }
        let mut tracks = vec![];
        for (s, v) in &others {
            tracks.extend(Self::tracks_of(*s, *v));
        }
        if real_order {
            if let Some(s) = st.rooms.get_mut(room_id).and_then(|r| r.seats.get_mut(&seat)) {
                s.pending_offer = tracks;
            }
        } else {
            self.offer(st, room_id, seat, tracks);
        }
        for (other, _) in other_sessions {
            self.offer(st, room_id, other, Self::tracks_of(seat, video));
        }
        Ok(joined)
    }
}

fn refused(status: u16, error: &str) -> NodeError {
    NodeError::Refused { status, error: error.into(), message: format!("fake node: {error}") }
}

fn session_of(sdp_offer: &str) -> std::result::Result<u32, NodeError> {
    sdp_offer
        .strip_prefix("fake-offer:")
        .and_then(|r| r.split(':').next())
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| refused(400, "bad_sdp"))
}

#[async_trait]
impl RoomApi for FakeNode {
    async fn create(&self, node: &CallNode, _limits: MediaLimits) -> std::result::Result<RoomCreated, NodeError> {
        self.gate(&node.node).await;
        let gate = self.state.lock().unwrap().hold_create.take();
        if let Some(gate) = gate {
            gate.notified().await;
        }
        let mut st = self.state.lock().unwrap();
        Self::take_refusal(&mut st)?;
        Self::alive(&st, &node.node)?;
        st.next += 1;
        let n = st.next;
        let room_id = hex_id(n, 32);
        let created = RoomCreated {
            room_id: room_id.clone(),
            join_token: format!("join-{n}"),
            admin_token: format!("admin-{n}"),
            expires_at: st.now + 12 * 3600,
            idle_secs: 300,
            max_participants: st.max_participants,
            kbps_per_participant: 2500,
            sfu_udp: format!("{}:3479", node.node.addr.ip()),
            sfu_tcp: format!("{}:3479", node.node.addr.ip()),
        };
        st.rooms.insert(
            room_id,
            FakeRoom {
                node: node.node.id.to_string(),
                join_token: created.join_token.clone(),
                admin_token: created.admin_token.clone(),
                next_seat: 0,
                seats: BTreeMap::new(),
                expires_at: created.expires_at,
                passes: vec![],
            },
        );
        st.created_on.push(node.node.to_string());
        Ok(created)
    }

    async fn join(&self, node: &CallNode, room_id: &str, token: &str, sdp_offer: &str) -> std::result::Result<Joined, NodeError> {
        self.gate(&node.node).await;
        let mut st = self.state.lock().unwrap();
        Self::take_refusal(&mut st)?;
        if let Some(e) = st.refuse_next_join.take() {
            return Err(e);
        }
        Self::alive(&st, &node.node)?;
        let session = session_of(sdp_offer)?;
        let room = st.rooms.get(room_id).ok_or_else(|| refused(404, "room_not_found"))?;
        // A room of another node is no room of this one.
        if room.node != node.node.id.to_string() {
            return Err(refused(404, "room_not_found"));
        }
        if room.join_token != token {
            return Err(refused(403, "bad_token"));
        }
        let _ = room.expires_at;
        self.seat(&mut st, room_id, session, None)
    }

    async fn leave(&self, node: &CallNode, room_id: &str, participant_id: u32, token: &str) -> std::result::Result<(), NodeError> {
        self.gate(&node.node).await;
        let mut st = self.state.lock().unwrap();
        Self::take_refusal(&mut st)?;
        Self::alive(&st, &node.node)?;
        let room = st.rooms.get(room_id).ok_or_else(|| refused(404, "room_not_found"))?;
        let seat = room.seats.get(&participant_id).ok_or_else(|| refused(404, "room_not_found"))?;
        let by_admin = token == room.admin_token;
        let by_home_token = seat.home_token.as_deref() == Some(token) && room.node == node.node.id.to_string();
        if !by_admin && !by_home_token && token != seat.token {
            return Err(refused(403, "bad_token"));
        }
        let session = seat.session;
        self.remove_seat(&mut st, room_id, participant_id);
        if by_admin {
            self.engine.inject_into(session, SessionEvent::ConnectionState(ConnectionState::Closed));
        }
        Ok(())
    }

    async fn change_token(&self, node: &CallNode, room_id: &str, admin_token: &str) -> std::result::Result<String, NodeError> {
        self.gate(&node.node).await;
        let mut st = self.state.lock().unwrap();
        Self::take_refusal(&mut st)?;
        Self::alive(&st, &node.node)?;
        let room = st.rooms.get_mut(room_id).ok_or_else(|| refused(404, "room_not_found"))?;
        if room.admin_token != admin_token {
            return Err(refused(403, "bad_token"));
        }
        room.join_token = format!("{}-next", room.join_token);
        Ok(room.join_token.clone())
    }

    async fn delegate(&self, home: &CallNode, room_id: &str, join_token: &str) -> std::result::Result<Delegated, NodeError> {
        self.gate(&home.node).await;
        let mut st = self.state.lock().unwrap();
        Self::alive(&st, &home.node)?;
        st.delegated.push((home.node.to_string(), room_id.to_string()));
        if Self::info(&st, &home.node).is_some_and(|n| n.wave4) {
            // The page of a node that has no such path.
            return Err(NodeError::Refused { status: 404, error: "http".into(), message: "status 404".into() });
        }
        st.next += 1;
        let pass = format!("pass-{}", st.next);
        let expires_at = st.now + 60;
        let room = st.rooms.get_mut(room_id).ok_or_else(|| refused(404, "room_not_found"))?;
        if room.node != home.node.id.to_string() {
            return Err(refused(404, "room_not_found"));
        }
        if room.join_token != join_token {
            return Err(refused(403, "bad_token"));
        }
        room.passes.push(pass.clone());
        Ok(Delegated { proxy_token: pass, expires_at })
    }

    async fn join_via(&self, via: &CallNode, home: &NodeRef, room_id: &str, proxy_token: &str, sdp_offer: &str) -> std::result::Result<Joined, NodeError> {
        self.gate(&via.node).await;
        let mut st = self.state.lock().unwrap();
        if let Some(e) = st.refuse_next_via.take() {
            return Err(e);
        }
        Self::alive(&st, &via.node)?;
        let session = session_of(sdp_offer)?;
        if via.node.id == home.id {
            return Err(refused(400, "bad_home"));
        }
        if !Self::info(&st, &via.node).is_none_or(|n| n.cascade && !n.wave4) {
            return Err(refused(503, "cascade_refused"));
        }
        if Self::alive(&st, home).is_err() || Self::is_silent(&st, home) {
            return Err(refused(503, "cascade_unreachable"));
        }
        let room = st.rooms.get_mut(room_id).ok_or_else(|| refused(404, "room_not_found"))?;
        if room.node != home.id.to_string() {
            return Err(refused(404, "room_not_found"));
        }
        let Some(at) = room.passes.iter().position(|p| p == proxy_token) else {
            return Err(refused(403, "bad_token"));
        };
        room.passes.remove(at);
        self.seat(&mut st, room_id, session, Some(via.node.id.to_string()))
    }

    async fn hello(&self, node: &CallNode) -> std::result::Result<Welcome, NodeError> {
        self.gate(&node.node).await;
        let mut st = self.state.lock().unwrap();
        Self::alive(&st, &node.node)?;
        let mut caps = vec![CAP_SFU.to_string()];
        if let Some(info) = st.nodes.iter_mut().find(|n| n.reference.id == node.node.id) {
            info.hellos += 1;
            if info.cascade && !info.wave4 {
                caps.push(CAP_CASCADE.to_string());
            }
        }
        Ok(Welcome {
            protocol_min: 1,
            protocol_max: 1,
            node_id: node.node.id.to_string(),
            version: "fake".into(),
            capabilities: caps,
            codecs: vec![],
            private: false,
            limits: Limits::default(),
        })
    }
}

impl RoomHook for FakeNode {
    fn data(&self, session: u32, label: &str, payload: DataPayload) {
        if label != CTL_LABEL {
            return;
        }
        let mut st = self.state.lock().unwrap();
        let Some((room_id, seat)) = Self::seat_of(&st, session) else { return };
        match payload {
            DataPayload::Text(text) => match Message::parse(&text) {
                Some(Message::Answer { seq, .. }) => st.answers.push((session, seq)),
                _ => st.texts.push((session, text)),
            },
            DataPayload::Binary(bytes) => {
                let frame = ctl::relayed(seat, &bytes);
                // The real node relays nothing to a closed channel.
                let others: Vec<u32> =
                    st.rooms[&room_id].seats.iter().filter(|(s, p)| **s != seat && p.ctl_open && !p.frozen).map(|(_, p)| p.session).collect();
                if seat == st.relay_from && !others.is_empty() {
                    if st.relay_pass > 0 {
                        st.relay_pass -= 1;
                    } else if st.relay_drop > 0 {
                        st.relay_drop -= 1;
                        return;
                    }
                }
                for s in others {
                    self.engine.inject_into(s, SessionEvent::Data { label: CTL_LABEL.into(), payload: DataPayload::Binary(frame.clone()) });
                }
            }
        }
    }

    fn closed(&self, session: u32) {
        let mut st = self.state.lock().unwrap();
        if let Some((room_id, seat)) = Self::seat_of(&st, session) {
            self.remove_seat(&mut st, &room_id, seat);
        }
    }
}

// ─── The groups ──────────────────────────────────────────────────────────────

/// The groups as a group call needs them: members in a map, a node pinned
/// by the tests, notes carried in the open (the test world hands them
/// to every member).
pub struct FakeGroups {
    me: PubKey,
    members: Mutex<HashMap<String, Vec<PubKey>>>,
    pinned: Mutex<HashMap<String, CallNode>>,
}

impl FakeGroups {
    pub fn new(me: PubKey) -> Self {
        Self { me, members: Mutex::default(), pinned: Mutex::default() }
    }

    pub fn set_members(&self, group_id: &str, members: Vec<PubKey>) {
        self.members.lock().unwrap().insert(group_id.to_string(), members);
    }

    pub fn pin(&self, group_id: &str, node: CallNode) {
        self.pinned.lock().unwrap().insert(group_id.to_string(), node);
    }

    /// A note of a group as `seal_note` made it, taken apart: the group,
    /// the author and the envelope.
    pub fn open_note(out: &Outbound) -> Option<(String, PubKey, Envelope)> {
        let Outbound::PublishScoped { scope: Scope::Group { id }, event } = out else { return None };
        let author = PubKey::parse(event.json["author"].as_str()?)?;
        let envelope = Envelope::parse(event.json["content"].as_str()?).ok()?;
        Some((id.clone(), author, envelope))
    }
}

#[async_trait]
impl GroupAccess for FakeGroups {
    async fn members(&self, group_id: &str) -> Result<Vec<PubKey>> {
        let members = self.members.lock().unwrap().get(group_id).cloned();
        match members {
            Some(m) if m.contains(&self.me) => Ok(m),
            _ => Err(MessengerError::Invalid("group_unknown".into())),
        }
    }

    async fn pinned_node(&self, group_id: &str) -> Result<Option<CallNode>> {
        Ok(self.pinned.lock().unwrap().get(group_id).cloned())
    }

    async fn seal_note(&self, group_id: &str, envelope: &Envelope) -> Result<Outbound> {
        let id = EventId::parse(&nostr::key::Keys::generate().public_key().to_hex()).expect("64 hex");
        let json = serde_json::json!({ "group": group_id, "author": self.me.as_hex(), "content": envelope.encode() });
        Ok(Outbound::PublishScoped { scope: Scope::Group { id: group_id.to_string() }, event: WireEvent { id, json } })
    }
}
