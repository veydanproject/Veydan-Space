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

use crate::fake_engine::{RoomHook, NODE_ANSWER, NODE_OFFER};
use crate::FakeEngine;
use async_trait::async_trait;
use messenger_calls::engine::{DataPayload, Media, SessionEvent, CTL_LABEL};
use messenger_calls::group::ctl::{self, Message, Track};
use messenger_calls::node_client::{Joined, Limits, MediaLimits, NodeClient, NodeError, RoomApi, RoomCreated, TurnCredentials, Welcome, CAP_SFU};
use messenger_calls::servers::CallNode;
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
}

struct FakeRoom {
    join_token: String,
    admin_token: String,
    next_seat: u32,
    seats: BTreeMap<u32, Seat>,
    expires_at: u64,
}

#[derive(Default)]
struct NodeState {
    next: u32,
    rooms: HashMap<String, FakeRoom>,
    /// The next request is refused so.
    refuse_next: Option<NodeError>,
    /// The next join is refused so (the room is made, the door is shut).
    refuse_next_join: Option<NodeError>,
    /// The next room is made only when this gate is opened.
    hold_create: Option<Arc<tokio::sync::Notify>>,
    /// Answers the sessions gave to offers: session, `seq`.
    answers: Vec<(u32, u32)>,
    /// Texts of the sessions that are not answers (a layer request).
    texts: Vec<(u32, String)>,
    /// The node references the rooms were made on.
    created_on: Vec<String>,
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

/// A fake call node: the rooms, and the `ctl` of their participants.
#[derive(Clone)]
pub struct FakeNode {
    state: Arc<Mutex<NodeState>>,
    engine: FakeEngine,
}

fn hex_id(n: u32, len: usize) -> String {
    format!("{n:02x}").repeat(len / 2)
}

impl FakeNode {
    /// A node whose rooms are joined by the sessions of `engine`.
    pub fn new(engine: FakeEngine) -> Self {
        let node = Self { state: Arc::new(Mutex::new(NodeState { max_participants: 12, now: 1_760_000_000, ..NodeState::default() })), engine: engine.clone() };
        engine.set_room_hook(Arc::new(node.clone()));
        node
    }

    /// The reference of this node, as the sets of servers list it.
    pub fn reference(&self) -> messenger_calls::servers::NodeRef {
        format!("203.0.113.7:8443#{}", "fa".repeat(32)).parse().expect("a node reference")
    }

    /// This node in the sets of servers, as the project's.
    pub fn as_call_node(&self) -> CallNode {
        CallNode::new(self.reference(), messenger_calls::servers::NodeClass::Project)
    }

    /// A node client whose HELLO every node answers with an SFU and
    /// credentials of a TURN that is not there: nothing of the network
    /// is touched.
    pub fn client(&self) -> NodeClient {
        NodeClient::with_fetch(
            "test",
            Arc::new(|node, _| {
                Box::pin(async move {
                    let welcome = Welcome {
                        protocol_min: 1,
                        protocol_max: 1,
                        node_id: node.node.id.to_string(),
                        version: "fake".into(),
                        capabilities: vec!["stun".into(), "turn".into(), CAP_SFU.into()],
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
                    Ok((welcome, credentials, std::time::Duration::from_millis(20)))
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
        self.text(session, &Message::Hello { you: seat, participants });
        self.offer(&mut st, room_id, seat, tracks);
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

    /// The seats of a room, in order, with their sessions.
    pub fn seats(&self, room_id: &str) -> Vec<(u32, u32)> {
        self.state.lock().unwrap().rooms.get(room_id).map(|r| r.seats.iter().map(|(s, p)| (*s, p.session)).collect()).unwrap_or_default()
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

    /// The node ends the room: every seat's connection is closed.
    pub fn end_room(&self, room_id: &str) {
        let sessions: Vec<u32> = {
            let mut st = self.state.lock().unwrap();
            st.rooms.remove(room_id).map(|r| r.seats.values().map(|s| s.session).collect()).unwrap_or_default()
        };
        for s in sessions {
            self.engine.inject_into(s, SessionEvent::ConnectionState(messenger_calls::engine::ConnectionState::Closed));
        }
    }

    /// The node says who speaks in the room of `session`.
    pub fn speaking(&self, room_id: &str, ids: Vec<u32>) {
        let sessions: Vec<u32> =
            self.state.lock().unwrap().rooms.get(room_id).map(|r| r.seats.values().map(|s| s.session).collect()).unwrap_or_default();
        let text = Message::Speaking { participants: ids }.encode();
        for s in sessions {
            self.engine.inject_into(s, SessionEvent::Data { label: CTL_LABEL.into(), payload: DataPayload::Text(text.clone()) });
        }
    }

    fn text(&self, session: u32, msg: &Message) {
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
        self.text(session, &Message::Offer { seq, sdp: format!("{NODE_OFFER}{seq}"), tracks: tracks.clone() });
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
            self.text(s, &Message::Left { id: seat });
            self.engine.inject_into(s, SessionEvent::RemoteTrackGone { mid: format!("a{seat}") });
            if gone.video {
                self.engine.inject_into(s, SessionEvent::RemoteTrackGone { mid: format!("v{seat}") });
            }
        }
    }

    fn seat_of(st: &NodeState, session: u32) -> Option<(String, u32)> {
        st.rooms.iter().find_map(|(id, r)| r.seats.iter().find(|(_, s)| s.session == session).map(|(seat, _)| (id.clone(), *seat)))
    }
}

fn refused(status: u16, error: &str) -> NodeError {
    NodeError::Refused { status, error: error.into(), message: format!("fake node: {error}") }
}

#[async_trait]
impl RoomApi for FakeNode {
    async fn create(&self, node: &CallNode, _limits: MediaLimits) -> std::result::Result<RoomCreated, NodeError> {
        let gate = self.state.lock().unwrap().hold_create.take();
        if let Some(gate) = gate {
            gate.notified().await;
        }
        let mut st = self.state.lock().unwrap();
        Self::take_refusal(&mut st)?;
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
            sfu_udp: "203.0.113.7:3479".into(),
            sfu_tcp: "203.0.113.7:3479".into(),
        };
        st.rooms.insert(
            room_id,
            FakeRoom {
                join_token: created.join_token.clone(),
                admin_token: created.admin_token.clone(),
                next_seat: 0,
                seats: BTreeMap::new(),
                expires_at: created.expires_at,
            },
        );
        st.created_on.push(node.node.to_string());
        Ok(created)
    }

    async fn join(&self, _node: &CallNode, room_id: &str, token: &str, sdp_offer: &str) -> std::result::Result<Joined, NodeError> {
        let mut st = self.state.lock().unwrap();
        Self::take_refusal(&mut st)?;
        if let Some(e) = st.refuse_next_join.take() {
            return Err(e);
        }
        let max = st.max_participants;
        let session: u32 = sdp_offer
            .strip_prefix("fake-offer:")
            .and_then(|r| r.split(':').next())
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| refused(400, "bad_sdp"))?;
        let video = self.engine.media_of(session) == Some(Media::Video);
        let room = st.rooms.get_mut(room_id).ok_or_else(|| refused(404, "room_not_found"))?;
        if room.join_token != token {
            return Err(refused(403, "bad_token"));
        }
        if room.seats.len() as u32 >= max {
            return Err(refused(409, "room_full"));
        }
        room.next_seat += 1;
        let seat = room.next_seat;
        let others: Vec<(u32, bool)> = room.seats.iter().map(|(s, p)| (*s, p.video)).collect();
        let token = format!("seat-{seat}");
        let _ = room.expires_at;
        let real_order = st.real_ctl_order;
        let room = st.rooms.get_mut(room_id).expect("the room is there");
        room.seats.insert(seat, Seat { session, token: token.clone(), video, offers: 0, ctl_open: !real_order, pending_offer: vec![] });
        let joined = Joined {
            sdp_answer: format!("{NODE_ANSWER}{seat}"),
            participant_id: seat,
            participant_token: token,
            participants: others.iter().map(|(s, _)| *s).collect(),
        };
        // The channel opens, the node says hello, the others hear of the
        // newcomer, and the offers go both ways. In the real order the
        // others hear `joined` now and the newcomer's channel opens later
        // (`open_ctl`); a seat whose channel is closed hears nothing.
        if !real_order {
            self.engine.inject_into(session, SessionEvent::DataOpen { label: CTL_LABEL.into() });
            self.text(session, &Message::Hello { you: seat, participants: joined.participants.clone() });
        }
        let other_sessions: Vec<(u32, u32)> =
            st.rooms[room_id].seats.iter().filter(|(s, p)| **s != seat && p.ctl_open).map(|(s, p)| (*s, p.session)).collect();
        for (_, s) in &other_sessions {
            self.text(*s, &Message::Joined { id: seat });
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
            self.offer(&mut st, room_id, seat, tracks);
        }
        for (other, _) in other_sessions {
            self.offer(&mut st, room_id, other, Self::tracks_of(seat, video));
        }
        Ok(joined)
    }

    async fn leave(&self, _node: &CallNode, room_id: &str, participant_id: u32, token: &str) -> std::result::Result<(), NodeError> {
        let mut st = self.state.lock().unwrap();
        Self::take_refusal(&mut st)?;
        let room = st.rooms.get(room_id).ok_or_else(|| refused(404, "room_not_found"))?;
        let seat = room.seats.get(&participant_id).ok_or_else(|| refused(404, "room_not_found"))?;
        let by_admin = token == room.admin_token;
        if !by_admin && token != seat.token {
            return Err(refused(403, "bad_token"));
        }
        let session = seat.session;
        self.remove_seat(&mut st, room_id, participant_id);
        if by_admin {
            self.engine.inject_into(session, SessionEvent::ConnectionState(messenger_calls::engine::ConnectionState::Closed));
        }
        Ok(())
    }

    async fn change_token(&self, _node: &CallNode, room_id: &str, admin_token: &str) -> std::result::Result<String, NodeError> {
        let mut st = self.state.lock().unwrap();
        Self::take_refusal(&mut st)?;
        let room = st.rooms.get_mut(room_id).ok_or_else(|| refused(404, "room_not_found"))?;
        if room.admin_token != admin_token {
            return Err(refused(403, "bad_token"));
        }
        room.join_token = format!("{}-next", room.join_token);
        Ok(room.join_token.clone())
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
                let others: Vec<u32> = st.rooms[&room_id].seats.iter().filter(|(s, p)| **s != seat && p.ctl_open).map(|(_, p)| p.session).collect();
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
