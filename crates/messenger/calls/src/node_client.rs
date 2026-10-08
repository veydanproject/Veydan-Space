// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The client side of a call node's control channel
//! (services/call/spec/protocol.md): TLS 1.3 to the node's address with
//! its id pinned (identity v2, as a bridge), ALPN `vcall/1`, h2, two
//! JSON requests: HELLO, then TURN credentials. The credentials are kept
//! until shortly before they expire, by the clock of this device from
//! the moment they came (the node's own `expires_at` is by the node's
//! clock, which may be anywhere); a call within that time asks the node
//! nothing.
//!
//! Which nodes a call uses is decided here too ([`NodeClient::pick`]):
//! the classes are asked in order of priority, one after another, and
//! the first class with a node answering is the one; in it the one or
//! two nearest by the round trip of a STUN binding (or of HELLO, when the
//! node's STUN does not answer over UDP). A class lower down is not even
//! spoken to while a higher one answers: whoever runs their own node
//! does not want the project's, or a volunteer's, to see their address
//! and the time of every call. The media never passes through here:
//! libwebrtc takes the ICE servers and speaks STUN and TURN itself.
//!
//! The types of the wire are spelled again here rather than taken from
//! `vcall-proto`: the messenger takes no crate from `services/` but the
//! client of VLink (scripts/boundaries.sh).

use crate::engine::{IceServer, RelayPolicy};
use crate::servers::{CallNode, DeviceCredentials, NodeClass, NodeRef};
use bytes::Bytes;
use messenger_core::{MessengerError, Result};
use messenger_vlink::proto::{io as h2io, pin, BridgeId};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::{TcpStream, UdpSocket};
use tokio_rustls::TlsConnector;

pub const PROTOCOL_MIN: u16 = 1;
pub const PROTOCOL_MAX: u16 = 1;
pub const ALPN: &[u8] = b"vcall/1";
const PATH_HELLO: &str = "/v1/hello";
const PATH_TURN: &str = "/v1/turn";
const PATH_ROOMS: &str = "/v1/rooms";
/// The exchange of an invitation for the credentials of this device.
const PATH_INVITE: &str = "/v1/invite";
/// What a node with an SFU says in `capabilities`.
pub const CAP_SFU: &str = "sfu";
/// What a node that seats its participants in the rooms of other nodes
/// says in `capabilities` (services/call/spec/cascade.md): a client may
/// join the room of a home node through it (`join` with `home`).
pub const CAP_CASCADE: &str = "cascade";
/// A request or an answer is a few hundred bytes; more is not one.
const MAX_BODY: usize = 64 * 1024;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// A join through my own node into the room of another takes longer:
/// its HELLO to the home (2 s), the join there (2 s), the ICE between
/// them (4 s), with a margin (`CASCADE_JOIN_TIMEOUT` of the cascade).
pub const CASCADE_JOIN_TIMEOUT: Duration = Duration::from_secs(10);
/// A HELLO that judges whether a node is there at all: a short one
/// (`HELLO_CHECK` of the cascade).
pub const HELLO_CHECK: Duration = Duration::from_secs(2);
/// A STUN binding that takes longer than this says nothing useful of the
/// distance; HELLO's time is used then.
const STUN_TIMEOUT: Duration = Duration::from_secs(1);
/// How long a pick waits for the nodes to answer, all classes together.
const PICK_TIMEOUT: Duration = Duration::from_secs(6);
/// How long one class gets before the next is asked: a node that neither
/// answers nor refuses should not keep the whole pick waiting.
const CLASS_TIMEOUT: Duration = Duration::from_secs(3);
/// Credentials this close to expiry are fetched again rather than used:
/// the allocation would outlive them, but a call started on them would
/// not get an allocation.
const CREDENTIALS_MARGIN_SECS: i64 = 60;
/// Nodes a call takes at most: one near one is enough, a second one is a
/// spare for when the first goes away mid-call.
const NODES_PER_CALL: usize = 2;

// ─── The wire of the control channel ───────────────────────────────────────

/// How a private node is asked to let me in: a shared access key, or
/// the credentials of this device (its id with the secret as `key`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Access {
    /// A shared access key (`VCALL_ACCESS_KEYS`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// The credentials of this device, as the invitation gave them
    /// (`redeem_invite`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<DeviceAccess>,
}

/// The credentials of a device on a private node: the node's name for
/// it and the secret (the node keeps the secret's hash only).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceAccess {
    pub id: String,
    pub secret: String,
}

impl Access {
    /// What `node` is asked with: its key as a shared key, or taken
    /// apart into the device's id and secret.
    pub fn of(node: &CallNode) -> Self {
        match node.device() {
            Some((id, secret)) => Self { key: None, device: Some(DeviceAccess { id: id.to_string(), secret: secret.to_string() }) },
            None => Self { key: node.access_key.clone(), device: None },
        }
    }
}

/// How busy a node says it is, in WELCOME: the figures of its report to
/// the registry (services/call/spec/protocol.md, "Регистрация"). A
/// `max_*` of 0 is "no limit said". Believed for the choice among nodes
/// and for the screen, nothing else.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeLoad {
    #[serde(default)]
    pub allocations: u32,
    #[serde(default)]
    pub max_allocations: u32,
    #[serde(default)]
    pub participants: u32,
    #[serde(default)]
    pub max_participants: u32,
    #[serde(default)]
    pub mbps: u32,
    #[serde(default)]
    pub max_mbps: u32,
    #[serde(default)]
    pub rooms: u32,
    #[serde(default)]
    pub max_rooms: u32,
    #[serde(default)]
    pub uplinks: u32,
    #[serde(default)]
    pub max_uplinks: u32,
}

impl NodeLoad {
    fn part(now: u32, max: u32) -> u8 {
        if max == 0 {
            0
        } else {
            (u64::from(now) * 100 / u64::from(max)).min(100) as u8
        }
    }

    /// The fullest of its parts, in percent (0–100), as the registry
    /// reckons it.
    pub fn percent(&self) -> u8 {
        Self::part(self.allocations, self.max_allocations)
            .max(Self::part(self.participants, self.max_participants))
            .max(Self::part(self.mbps, self.max_mbps))
            .max(Self::part(self.rooms, self.max_rooms))
            .max(Self::part(self.uplinks, self.max_uplinks))
    }

    /// At one of its own caps: a call there would be refused or starved.
    /// The caps are the node's; nothing of the client's is in this.
    pub fn full(&self) -> bool {
        self.percent() >= 100
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol_min: u16,
    pub protocol_max: u16,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub codecs: Vec<String>,
    #[serde(default)]
    pub access: Access,
    #[serde(default)]
    pub client: String,
}

/// What the node allows. Shown to the user; nothing of it is carried in
/// the client as a number of its own.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    #[serde(default)]
    pub turn_lifetime_secs: u32,
    #[serde(default)]
    pub turn_kbps_per_allocation: u32,
    #[serde(default)]
    pub credentials_ttl_secs: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Welcome {
    pub protocol_min: u16,
    pub protocol_max: u16,
    pub node_id: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub codecs: Vec<String>,
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub limits: Limits,
    /// How busy the node is now; a node of before the volunteers' wave
    /// says nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load: Option<NodeLoad>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnRequest {
    #[serde(default)]
    pub access: Access,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnCredentials {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub realm: String,
    #[serde(default)]
    pub ttl_secs: u64,
    pub expires_at: u64,
    /// `stun:`, `turn:` and `turns:` URIs, as an ICE server list takes them.
    pub urls: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    pub error: String,
    #[serde(default)]
    pub message: String,
}

/// `POST /v1/invite`: the invitation of a private node (`vcall ctl
/// invite`, the `t` of `veydan://call-node/…`) exchanged for the
/// credentials of this device.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteRequest {
    pub token: String,
    /// What this device calls itself, for the operator's list
    /// (`vcall ctl devices`): a product and a platform, no name of a person.
    #[serde(default)]
    pub name: String,
}

/// What the node gave for the invitation: the device's id (what the
/// operator revokes) and its secret, good until the operator revokes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceIssued {
    pub device_id: String,
    pub secret: String,
    /// The node as it names itself, `address:port#id`.
    #[serde(default)]
    pub node: String,
    /// The label the operator gave the invitation, if any.
    #[serde(default)]
    pub label: String,
}

/// What a node allows a room: the creator may lower the node's limits,
/// never raise them (`None` is the node's own).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_participants: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kbps_per_participant: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomRequest {
    #[serde(default)]
    pub access: Access,
    #[serde(default)]
    pub media_limits: MediaLimits,
}

/// The room as made: the label, the two tokens and the limits as set
/// (the node's, not the client's: shown, never carried as numbers of
/// the client's own).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomCreated {
    pub room_id: String,
    pub join_token: String,
    pub admin_token: String,
    /// Unix seconds by the node's clock: the room ends then whatever goes on.
    pub expires_at: u64,
    #[serde(default)]
    pub idle_secs: u32,
    #[serde(default)]
    pub max_participants: u32,
    #[serde(default)]
    pub kbps_per_participant: u32,
    #[serde(default)]
    pub sfu_udp: String,
    #[serde(default)]
    pub sfu_tcp: String,
}

/// The home of a room, in a join through another node: `node` is the
/// reference of the node the room is on (`address:port#id`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinHome {
    pub node: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinRequest {
    /// The join token of the room; in a join with `home`, the one-time
    /// pass of a seat the home gave (`delegate`).
    pub token: String,
    pub sdp_offer: String,
    #[serde(default)]
    pub caps: Vec<String>,
    /// The key of the node asked, when it is private and the join is
    /// through it into the room of another (read with `home` alone).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<Access>,
    /// The room is on another node: seat me there through this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<JoinHome>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Joined {
    pub sdp_answer: String,
    /// My seat: what the others see me as.
    pub participant_id: u32,
    /// For `leave` alone.
    pub participant_token: String,
    /// Who is in the room already.
    #[serde(default)]
    pub participants: Vec<u32>,
    /// In a join through another node: the token of my seat on the home
    /// (the proxy seat of my node there), to give it up myself when my
    /// node is gone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_token: Option<String>,
}

/// `POST /v1/rooms/{id}/delegate`: a one-time pass of a seat for a join
/// through another node, by the join token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DelegateRequest {
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delegated {
    pub proxy_token: String,
    #[serde(default)]
    pub expires_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaveRequest {
    pub participant_id: u32,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenRequest {
    pub admin_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenChanged {
    pub join_token: String,
}

/// Why a request to a node failed: the node refused it with a word
/// (`404 room_not_found`, `403 bad_token`, `409 room_full`, `429
/// rooms_full`…), or the way to it did not work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeError {
    Refused { status: u16, error: String, message: String },
    Unreachable(String),
}

impl NodeError {
    /// The node's word, when it refused.
    pub fn word(&self) -> Option<&str> {
        match self {
            Self::Refused { error, .. } => Some(error),
            Self::Unreachable(_) => None,
        }
    }

    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Refused { status, .. } => Some(*status),
            Self::Unreachable(_) => None,
        }
    }

    /// The node has no such room (`404 room_not_found`), or no such path
    /// at all (a 404 page of a node before the cascade).
    pub fn is_not_found(&self) -> bool {
        self.status() == Some(404)
    }

    /// The room is full (`409 room_full`).
    pub fn is_room_full(&self) -> bool {
        self.status() == Some(409)
    }

    /// The node would not let me in: no key, a wrong one, the device
    /// revoked (`401 access_key_required`, `403 bad_key`, `403
    /// device_revoked`). Not a refusal of the request itself.
    pub fn is_not_let_in(&self) -> bool {
        matches!(self.status(), Some(401) | Some(403))
    }
}

impl std::fmt::Display for NodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused { status, error, message } => write!(f, "{status} {error}: {message}"),
            Self::Unreachable(e) => f.write_str(e),
        }
    }
}

impl From<NodeError> for MessengerError {
    fn from(e: NodeError) -> Self {
        MessengerError::Transport(format!("call node: {e}"))
    }
}

/// The rooms of an SFU, as a client sees them (services/call/spec/protocol.md,
/// "Комнаты"). Implemented over the control channel here; the tests give
/// a fake node.
#[async_trait::async_trait]
pub trait RoomApi: Send + Sync {
    /// `POST /v1/rooms`: a room on `node` with the access it needs.
    async fn create(&self, node: &CallNode, limits: MediaLimits) -> std::result::Result<RoomCreated, NodeError>;
    /// `POST /v1/rooms/{id}/join` with my one offer; the node's answer
    /// carries its candidates.
    async fn join(&self, node: &CallNode, room_id: &str, token: &str, sdp_offer: &str) -> std::result::Result<Joined, NodeError>;
    /// `POST /v1/rooms/{id}/leave`: my own token gives my seat up, the
    /// admin's gives anybody's.
    async fn leave(&self, node: &CallNode, room_id: &str, participant_id: u32, token: &str) -> std::result::Result<(), NodeError>;
    /// `POST /v1/rooms/{id}/token`: a new join token; the old one lets
    /// nobody in from now on.
    async fn change_token(&self, node: &CallNode, room_id: &str, admin_token: &str) -> std::result::Result<String, NodeError>;
    /// `POST /v1/rooms/{id}/delegate` on the home of the room: a one-time
    /// pass of a seat by the join token, for a join through my own node
    /// (services/call/spec/cascade.md). A node before the cascade answers
    /// 404 (`NodeError::is_not_found`): the join goes to the home directly.
    async fn delegate(&self, home: &CallNode, room_id: &str, join_token: &str) -> std::result::Result<Delegated, NodeError>;
    /// `POST /v1/rooms/{id}/join` on `via`, my own node, with `home`: it
    /// seats me in the room of the home through a seat of its own there.
    /// `proxy_token` is the pass from `delegate`; the key of `via` goes
    /// in `access` when it has one. Waits [`CASCADE_JOIN_TIMEOUT`].
    async fn join_via(
        &self,
        via: &CallNode,
        home: &NodeRef,
        room_id: &str,
        proxy_token: &str,
        sdp_offer: &str,
    ) -> std::result::Result<Joined, NodeError>;
    /// HELLO alone, within [`HELLO_CHECK`]: is the node there.
    async fn hello(&self, node: &CallNode) -> std::result::Result<Welcome, NodeError>;
}

/// The rooms over the control channel of the node.
pub struct HttpRooms {
    client_name: String,
}

impl HttpRooms {
    pub fn new(client_name: &str) -> Self {
        Self { client_name: client_name.to_string() }
    }

    async fn session(&self, node: &CallNode) -> std::result::Result<(h2::client::SendRequest<Bytes>, Welcome), NodeError> {
        let send = connect(&node.node).await.map_err(|e| NodeError::Unreachable(e.to_string()))?;
        let short = node.node.id.short();
        let welcome: Welcome = post(&send, PATH_HELLO, &hello(Access::of(node), &self.client_name), short.as_str()).await?;
        if welcome.node_id != node.node.id.to_string() {
            return Err(NodeError::Unreachable(format!("node {short}: says it is another node")));
        }
        Ok((send, welcome))
    }
}

#[async_trait::async_trait]
impl RoomApi for HttpRooms {
    async fn create(&self, node: &CallNode, limits: MediaLimits) -> std::result::Result<RoomCreated, NodeError> {
        let (send, _) = self.session(node).await?;
        let request = RoomRequest { access: Access::of(node), media_limits: limits };
        post(&send, PATH_ROOMS, &request, node.node.id.short().as_str()).await
    }

    async fn join(&self, node: &CallNode, room_id: &str, token: &str, sdp_offer: &str) -> std::result::Result<Joined, NodeError> {
        let (send, _) = self.session(node).await?;
        let request = JoinRequest { token: token.to_string(), sdp_offer: sdp_offer.to_string(), caps: vec![], access: None, home: None };
        post(&send, &format!("{PATH_ROOMS}/{room_id}/join"), &request, node.node.id.short().as_str()).await
    }

    async fn leave(&self, node: &CallNode, room_id: &str, participant_id: u32, token: &str) -> std::result::Result<(), NodeError> {
        let (send, _) = self.session(node).await?;
        let request = LeaveRequest { participant_id, token: token.to_string() };
        let _: serde_json::Value = post(&send, &format!("{PATH_ROOMS}/{room_id}/leave"), &request, node.node.id.short().as_str()).await?;
        Ok(())
    }

    async fn change_token(&self, node: &CallNode, room_id: &str, admin_token: &str) -> std::result::Result<String, NodeError> {
        let (send, _) = self.session(node).await?;
        let request = TokenRequest { admin_token: admin_token.to_string() };
        let changed: TokenChanged = post(&send, &format!("{PATH_ROOMS}/{room_id}/token"), &request, node.node.id.short().as_str()).await?;
        Ok(changed.join_token)
    }

    async fn delegate(&self, home: &CallNode, room_id: &str, join_token: &str) -> std::result::Result<Delegated, NodeError> {
        let (send, _) = self.session(home).await?;
        let request = DelegateRequest { token: join_token.to_string() };
        post(&send, &format!("{PATH_ROOMS}/{room_id}/delegate"), &request, home.node.id.short().as_str()).await
    }

    async fn join_via(
        &self,
        via: &CallNode,
        home: &NodeRef,
        room_id: &str,
        proxy_token: &str,
        sdp_offer: &str,
    ) -> std::result::Result<Joined, NodeError> {
        let (send, _) = self.session(via).await?;
        let request = JoinRequest {
            token: proxy_token.to_string(),
            sdp_offer: sdp_offer.to_string(),
            caps: vec![],
            access: via.access_key.as_ref().map(|_| Access::of(via)),
            home: Some(JoinHome { node: home.to_string() }),
        };
        post_within(&send, &format!("{PATH_ROOMS}/{room_id}/join"), &request, via.node.id.short().as_str(), CASCADE_JOIN_TIMEOUT).await
    }

    async fn hello(&self, node: &CallNode) -> std::result::Result<Welcome, NodeError> {
        let short = node.node.id.short();
        match tokio::time::timeout(HELLO_CHECK, self.session(node)).await {
            Ok(Ok((_, welcome))) => Ok(welcome),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(NodeError::Unreachable(format!("node {short}: no answer to hello in time"))),
        }
    }
}

/// What this client says of itself in HELLO.
pub fn hello(access: Access, client: &str) -> Hello {
    Hello {
        protocol_min: PROTOCOL_MIN,
        protocol_max: PROTOCOL_MAX,
        capabilities: vec!["stun".into(), "turn".into(), "turn-tcp".into(), "turn-tls".into()],
        codecs: vec!["opus".into()],
        access,
        client: client.to_string(),
    }
}

/// The ICE servers the credentials of one node make: STUN without
/// credentials, TURN with them.
pub fn ice_servers_of(creds: &TurnCredentials) -> Vec<IceServer> {
    let (stun, turn): (Vec<String>, Vec<String>) = creds.urls.iter().cloned().partition(|u| u.starts_with("stun:"));
    let mut out = Vec::new();
    if !stun.is_empty() {
        out.push(IceServer { urls: stun, username: None, credential: None });
    }
    if !turn.is_empty() {
        out.push(IceServer { urls: turn, username: Some(creds.username.clone()), credential: Some(creds.password.clone()) });
    }
    out
}

/// The `stun:` address in a credentials' URI list, when it is a bare one.
pub fn stun_addr(creds: &TurnCredentials) -> Option<SocketAddr> {
    creds.urls.iter().find_map(|u| u.strip_prefix("stun:")?.split('?').next()?.parse().ok())
}

// ─── The client ────────────────────────────────────────────────────────────

/// What a node gave, kept while it is good.
#[derive(Clone, Debug)]
pub struct NodeAccess {
    pub node: CallNode,
    pub welcome: Welcome,
    /// The TURN credentials; none (no `urls`) when the node would not
    /// let me in (`authorized` false): a private node I have no key to,
    /// or whose key it no longer takes. HELLO and a join by token still
    /// work there; TURN and rooms of my own do not.
    pub credentials: TurnCredentials,
    /// The node took my access: it gave TURN credentials.
    pub authorized: bool,
    /// The round trip to the node, as measured when the credentials were
    /// fetched.
    pub rtt: Duration,
    /// Until when the credentials are used, by this device's clock: their
    /// `ttl_secs` from the moment they came, less a margin.
    pub good_until: Instant,
}

impl NodeAccess {
    /// How full the node said it was in WELCOME, percent.
    pub fn load(&self) -> Option<u8> {
        self.welcome.load.map(|l| l.percent())
    }

    /// At one of its caps by its own word: taken only when no other
    /// node of the class answered.
    pub fn full(&self) -> bool {
        self.welcome.load.is_some_and(|l| l.full())
    }
}

/// A node the client spoke to lately, for the screen: its round trip,
/// what it said of itself, whether it let me in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownNode {
    pub id: String,
    pub class: NodeClass,
    pub rtt_ms: u64,
    pub private: bool,
    pub authorized: bool,
    pub version: String,
    pub capabilities: Vec<String>,
    /// How full the node said it was, percent; `None` when it says nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load: Option<u8>,
    /// `active` | `degraded` (at one of its caps by its own word).
    pub state: String,
    /// Whether the credentials kept are still good.
    pub good: bool,
}

/// The round trip in steps of this: nodes within one step are as near
/// as each other, and the less loaded of them is preferred.
const RTT_STEP: Duration = Duration::from_millis(50);

/// The order of the nodes of one class that answered (the rule of the
/// choice, written once): not full before full; nearer before farther,
/// by steps of [`RTT_STEP`]; within a step the less loaded by the node's
/// own word first (a node that says nothing counts as empty); then the
/// order the sets gave (the registry's list is least loaded first); the
/// exact round trip last.
fn rank(a: &NodeAccess, index: usize) -> (bool, u128, u8, usize, Duration) {
    (a.full(), a.rtt.as_millis() / RTT_STEP.as_millis(), a.load().unwrap_or(0), index, a.rtt)
}

/// Until when credentials that came at `fetched` are used. The node
/// says how long they last (`ttl_secs`); when it does not, its
/// `expires_at` against `now` (both by the node's and this device's
/// clocks, the best there is then). Shorter than the margin: not at all.
fn good_until(fetched: Instant, creds: &TurnCredentials, now: i64) -> Instant {
    let ttl = if creds.ttl_secs > 0 { creds.ttl_secs as i64 } else { creds.expires_at as i64 - now };
    fetched + Duration::from_secs((ttl - CREDENTIALS_MARGIN_SECS).max(0) as u64)
}

/// What a call was given to connect with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Picked {
    pub servers: Vec<IceServer>,
    /// The ids of the nodes behind `servers`, nearest first.
    pub nodes: Vec<String>,
    /// The limits of the nearest node, for the screen.
    pub limits: Option<Limits>,
}

/// What `fetch` gives: WELCOME, the credentials (none when the node
/// would not let me in), the round trip.
pub type Fetched = (Welcome, TurnCredentials, Duration);
/// The way to a node; the tests put a fake one in (`NodeClient::with_fetch`).
pub type Fetch = Arc<dyn Fn(CallNode, String) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Fetched>> + Send>> + Send + Sync>;
/// The exchange of an invitation: the node, the token, the label, the
/// client's name → what the node issued. The tests put a fake one in.
pub type Invite = Arc<
    dyn Fn(NodeRef, String, String, String) -> std::pin::Pin<Box<dyn std::future::Future<Output = std::result::Result<DeviceIssued, NodeError>> + Send>>
        + Send
        + Sync,
>;

struct Inner {
    client_name: String,
    cache: Mutex<HashMap<BridgeId, NodeAccess>>,
    fetch: Fetch,
    invite: Invite,
}

#[derive(Clone)]
pub struct NodeClient {
    inner: Arc<Inner>,
}

impl NodeClient {
    /// `client_name` goes into HELLO for the node's log: `veydan-chat/5.1.0`.
    pub fn new(client_name: &str) -> Self {
        Self::with_fakes(
            client_name,
            Arc::new(|node, name| Box::pin(async move { fetch(&node, &name).await })),
            Arc::new(|node, token, label, name| Box::pin(async move { redeem(&node, &token, &label, &name).await })),
        )
    }

    /// A client whose way to every node is `fetch`: the fakes of the
    /// tests. Invitations are exchanged over the network still.
    pub fn with_fetch(client_name: &str, fetch: Fetch) -> Self {
        Self::with_fakes(client_name, fetch, Arc::new(|node, token, label, name| Box::pin(async move { redeem(&node, &token, &label, &name).await })))
    }

    /// A client whose way to every node is `fetch` and whose invitations
    /// are exchanged by `invite`: the fakes of the tests.
    pub fn with_fakes(client_name: &str, fetch: Fetch, invite: Invite) -> Self {
        Self { inner: Arc::new(Inner { client_name: client_name.to_string(), cache: Mutex::new(HashMap::new()), fetch, invite }) }
    }

    /// Forget every credential (a logout, a test).
    pub fn clear(&self) {
        self.inner.cache.lock().unwrap().clear();
    }

    /// Forget what one node gave (its key changed, it was removed).
    pub fn forget(&self, id: &BridgeId) {
        self.inner.cache.lock().unwrap().remove(id);
    }

    /// The nodes spoken to lately, as the screen lists them.
    pub fn known(&self) -> Vec<KnownNode> {
        let now = Instant::now();
        let mut out: Vec<KnownNode> = self
            .inner
            .cache
            .lock()
            .unwrap()
            .values()
            .map(|a| KnownNode {
                id: a.node.node.id.to_string(),
                class: a.node.class,
                rtt_ms: a.rtt.as_millis() as u64,
                private: a.welcome.private,
                authorized: a.authorized,
                version: a.welcome.version.clone(),
                capabilities: a.welcome.capabilities.clone(),
                load: a.load(),
                state: if a.full() { "degraded" } else { "active" }.to_string(),
                good: a.good_until > now,
            })
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// The invitation of a private node (the `t` of its link) exchanged
    /// for the credentials of this device, over TLS pinned to the id of
    /// the link. `name` is what this device calls itself for the
    /// operator's list. The node answers 404 when it is of a version
    /// before invitations, 403 `bad_invite` when the token is unknown,
    /// used up or ran out.
    pub async fn redeem_invite(&self, node: &NodeRef, token: &str, name: &str, now: i64) -> Result<DeviceCredentials> {
        let issued = (self.inner.invite)(node.clone(), token.to_string(), name.to_string(), self.inner.client_name.clone())
            .await
            .map_err(|e| match e.status() {
                Some(404) => MessengerError::Transport(format!("node {}: takes no invitations (a version before them)", node.id.short())),
                _ => MessengerError::from(e),
            })?;
        if !issued.node.is_empty() && issued.node.parse::<NodeRef>().ok().is_none_or(|n| n.id != node.id) {
            return Err(MessengerError::Transport(format!("node {}: says it is another node", node.id.short())));
        }
        if issued.device_id.is_empty() || issued.secret.is_empty() || issued.device_id.contains(':') {
            return Err(MessengerError::Transport(format!("node {}: issued no usable credentials", node.id.short())));
        }
        self.forget(&node.id);
        Ok(DeviceCredentials {
            node: node.clone(),
            device_id: issued.device_id,
            secret: issued.secret,
            label: issued.label,
            name: name.to_string(),
            added_at: now,
        })
    }

    /// The credentials of `node`, from the cache while they are good for
    /// another minute by this device's clock, otherwise from the node.
    pub async fn access(&self, node: &CallNode, now: i64) -> Result<NodeAccess> {
        if let Some(kept) = self.inner.cache.lock().unwrap().get(&node.node.id) {
            if kept.node.access_key == node.access_key && kept.good_until > Instant::now() {
                return Ok(kept.clone());
            }
        }
        let fetched = Instant::now();
        let (welcome, credentials, rtt) = (self.inner.fetch)(node.clone(), self.inner.client_name.clone()).await?;
        let authorized = !credentials.urls.is_empty();
        let good_until = if authorized { good_until(fetched, &credentials, now) } else { fetched };
        let access = NodeAccess { node: node.clone(), welcome, credentials, authorized, rtt, good_until };
        self.inner.cache.lock().unwrap().insert(node.node.id, access.clone());
        Ok(access)
    }

    /// Asks every node of `class` among `nodes` at once, within
    /// [`CLASS_TIMEOUT`] and `deadline`: the ones that answered and let
    /// me in (`keep` says which answers count), in the order of the rule
    /// ([`rank`]).
    async fn ask_class(&self, nodes: &[CallNode], class: NodeClass, deadline: tokio::time::Instant, now: i64, keep: fn(&NodeAccess) -> bool) -> Vec<NodeAccess> {
        let mut tasks = tokio::task::JoinSet::new();
        for (index, node) in nodes.iter().enumerate().filter(|(_, n)| n.class == class) {
            let (client, node) = (self.clone(), node.clone());
            tasks.spawn(async move { (index, client.access(&node, now).await) });
        }
        let class_deadline = deadline.min(tokio::time::Instant::now() + CLASS_TIMEOUT);
        let mut answered: Vec<(usize, NodeAccess)> = Vec::new();
        while let Ok(Some(joined)) = tokio::time::timeout_at(class_deadline, tasks.join_next()).await {
            if let Ok((index, Ok(access))) = joined {
                if access.authorized && keep(&access) {
                    answered.push((index, access));
                } else if !access.authorized {
                    tracing::debug!(node = %access.node.node.id.short(), "call node: would not let me in; not chosen");
                }
            }
        }
        answered.sort_by_key(|(index, a)| rank(a, *index));
        answered.into_iter().map(|(_, a)| a).collect()
    }

    /// The ICE servers of a call: of the first class of `nodes` (which
    /// come in order of priority) that has a node answering and letting
    /// me in, the one or two first by the rule of [`rank`]: nearest by
    /// steps of 50 ms, the less loaded within a step, a node at its caps
    /// only when no other answered. The classes are asked one after
    /// another, and a class is not spoken to while a higher one answers.
    /// Without a node a call under `Auto` goes with host candidates
    /// alone; under `RelayOnly` there is nothing to relay through, and
    /// that is an error.
    pub async fn pick(&self, nodes: &[CallNode], policy: RelayPolicy, now: i64) -> Result<Picked> {
        let mut classes: Vec<NodeClass> = nodes.iter().map(|n| n.class).collect();
        classes.sort_unstable();
        classes.dedup();
        let deadline = tokio::time::Instant::now() + PICK_TIMEOUT;
        for class in classes {
            let answered = self.ask_class(nodes, class, deadline, now, |_| true).await;
            if answered.is_empty() {
                continue;
            }
            let chosen: Vec<&NodeAccess> = answered.iter().take(NODES_PER_CALL).collect();
            return Ok(Picked {
                servers: chosen.iter().flat_map(|a| ice_servers_of(&a.credentials)).collect(),
                nodes: chosen.iter().map(|a| a.node.node.id.to_string()).collect(),
                limits: chosen.first().map(|a| a.welcome.limits.clone()),
            });
        }
        if policy == RelayPolicy::RelayOnly {
            Err(MessengerError::Transport("no call node answered: nothing to relay through".into()))
        } else {
            Ok(Picked::default())
        }
    }

    /// The node a group call makes its room on: of the first class of
    /// `nodes` (in order of priority) with a node that answers, lets me
    /// in and has an SFU, the first by the rule of [`rank`]. The classes
    /// are asked one after another, as in `pick`. `None` when no node
    /// with an SFU answers.
    pub async fn pick_sfu(&self, nodes: &[CallNode], now: i64) -> Option<NodeAccess> {
        let mut classes: Vec<NodeClass> = nodes.iter().map(|n| n.class).collect();
        classes.sort_unstable();
        classes.dedup();
        let deadline = tokio::time::Instant::now() + PICK_TIMEOUT;
        for class in classes {
            let answered = self.ask_class(nodes, class, deadline, now, |a| a.welcome.capabilities.iter().any(|c| c == CAP_SFU)).await;
            if let Some(best) = answered.into_iter().next() {
                return Some(best);
            }
        }
        None
    }
}

fn transport(e: impl std::fmt::Display) -> MessengerError {
    MessengerError::Transport(e.to_string())
}

/// How a caller reaches the node `id`: the one chain accepted, and the
/// ALPN of the control channel.
fn client_config(id: BridgeId) -> Result<rustls::ClientConfig> {
    let mut config = rustls::ClientConfig::builder_with_provider(pin::provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| MessengerError::Crypto(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(pin::verifier(id))
        .with_no_client_auth();
    config.alpn_protocols = vec![ALPN.to_vec()];
    Ok(config)
}

/// HELLO and TURN credentials from `node`, with the round trip measured.
/// A node that takes the HELLO but not my access (401, 403) answers with
/// no credentials: the caller sees it is there and that it would not let
/// me in (`NodeAccess::authorized`).
async fn fetch(node: &CallNode, client_name: &str) -> Result<Fetched> {
    let short = node.node.id.short();
    let short = short.as_str();
    let send = connect(&node.node).await?;
    let access = Access::of(node);
    let started = Instant::now();
    let welcome: Welcome = post(&send, PATH_HELLO, &hello(access.clone(), client_name), short).await?;
    let hello_rtt = started.elapsed();
    let low = PROTOCOL_MIN.max(welcome.protocol_min);
    let high = PROTOCOL_MAX.min(welcome.protocol_max);
    if low > high {
        return Err(MessengerError::Transport(format!("node {short}: no protocol version in common")));
    }
    if welcome.node_id != node.node.id.to_string() {
        return Err(MessengerError::Transport(format!("node {short}: says it is another node")));
    }
    let credentials: TurnCredentials = match post(&send, PATH_TURN, &TurnRequest { access }, short).await {
        Ok(c) => c,
        Err(e) if e.is_not_let_in() => {
            tracing::info!(node = %short, error = %e, "call node: would not let me in");
            return Ok((welcome, TurnCredentials::none(), hello_rtt));
        }
        Err(e) => return Err(e.into()),
    };
    let rtt = match stun_addr(&credentials) {
        Some(addr) => stun_rtt(addr).await.unwrap_or(hello_rtt),
        None => hello_rtt,
    };
    Ok((welcome, credentials, rtt))
}

/// The exchange of an invitation at `node`: HELLO (a node of another id
/// or version is found out here), then `POST /v1/invite`.
async fn redeem(node: &NodeRef, token: &str, name: &str, client_name: &str) -> std::result::Result<DeviceIssued, NodeError> {
    let short = node.id.short();
    let short = short.as_str();
    let send = connect(node).await.map_err(|e| NodeError::Unreachable(e.to_string()))?;
    let welcome: Welcome = post(&send, PATH_HELLO, &hello(Access::default(), client_name), short).await?;
    if welcome.node_id != node.id.to_string() {
        return Err(NodeError::Unreachable(format!("node {short}: says it is another node")));
    }
    let request = InviteRequest { token: token.to_string(), name: name.to_string() };
    post(&send, PATH_INVITE, &request, short).await
}

impl TurnCredentials {
    /// No credentials: the node would not let me in.
    pub fn none() -> Self {
        Self { username: String::new(), password: String::new(), realm: String::new(), ttl_secs: 0, expires_at: 0, urls: vec![] }
    }
}

/// The control channel of `node`: TLS with its id pinned, h2.
async fn connect(node: &NodeRef) -> Result<h2::client::SendRequest<Bytes>> {
    let short = node.id.short();
    let short = short.as_str();
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(node.addr))
        .await
        .map_err(|_| MessengerError::Transport(format!("node {short}: no connection in time")))?
        .map_err(|e| MessengerError::Transport(format!("node {short}: {e}")))?;
    let _ = tcp.set_nodelay(true);
    let config = client_config(node.id)?;
    let name = pin::server_name(node).map_err(|e| MessengerError::Crypto(e.to_string()))?;
    let tls = tokio::time::timeout(CONNECT_TIMEOUT, TlsConnector::from(Arc::new(config)).connect(name, tcp))
        .await
        .map_err(|_| MessengerError::Transport(format!("node {short}: no TLS in time")))?
        .map_err(|e| MessengerError::Transport(format!("node {short}: tls: {e}")))?;
    if tls.get_ref().1.alpn_protocol() != Some(ALPN) {
        return Err(MessengerError::Transport(format!("node {short}: not a call node (no control ALPN)")));
    }
    let (send, connection) = tokio::time::timeout(CONNECT_TIMEOUT, h2io::client_builder().handshake::<_, Bytes>(tls))
        .await
        .map_err(|_| MessengerError::Transport(format!("node {short}: no h2 in time")))?
        .map_err(transport)?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    Ok(send)
}

/// One request; a status other than 200 is the node's refusal.
async fn post<T: Serialize, R: DeserializeOwned>(
    send: &h2::client::SendRequest<Bytes>,
    path: &str,
    body: &T,
    short: &str,
) -> std::result::Result<R, NodeError> {
    post_within(send, path, body, short, REQUEST_TIMEOUT).await
}

/// One request that may take `wait` to be answered.
async fn post_within<T: Serialize, R: DeserializeOwned>(
    send: &h2::client::SendRequest<Bytes>,
    path: &str,
    body: &T,
    short: &str,
    wait: Duration,
) -> std::result::Result<R, NodeError> {
    let unreachable = |e: MessengerError| NodeError::Unreachable(e.to_string());
    let mut send = send.clone().ready().await.map_err(transport).map_err(unreachable)?;
    let request = http::Request::builder()
        .method(http::Method::POST)
        .uri(path)
        .header("content-type", "application/json")
        .body(())
        .map_err(transport)
        .map_err(unreachable)?;
    let (response, mut stream) = send.send_request(request, false).map_err(transport).map_err(unreachable)?;
    let json = serde_json::to_vec(body).map_err(|e| NodeError::Unreachable(e.to_string()))?;
    stream.send_data(Bytes::from(json), true).map_err(transport).map_err(unreachable)?;
    let response = tokio::time::timeout(wait, response)
        .await
        .map_err(|_| NodeError::Unreachable(format!("node {short}: no answer in time")))?
        .map_err(transport)
        .map_err(unreachable)?;
    let status = response.status().as_u16();
    let body = tokio::time::timeout(REQUEST_TIMEOUT, read_body(response.into_body()))
        .await
        .map_err(|_| NodeError::Unreachable(format!("node {short}: no answer in time")))?
        .map_err(unreachable)?;
    if status == 200 {
        return serde_json::from_slice(&body).map_err(|e| NodeError::Unreachable(format!("node {short}: {e}")));
    }
    let refusal: Refusal = serde_json::from_slice(&body).unwrap_or(Refusal { error: "http".into(), message: format!("status {status}") });
    Err(NodeError::Refused { status, error: refusal.error, message: format!("node {short}: {}", refusal.message) })
}

async fn read_body(mut body: h2::RecvStream) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    while let Some(chunk) = body.data().await {
        let chunk = chunk.map_err(transport)?;
        out.extend_from_slice(&chunk);
        if out.len() > MAX_BODY {
            return Err(MessengerError::Transport("the node's answer is too long".into()));
        }
        let _ = body.flow_control().release_capacity(chunk.len());
    }
    Ok(out)
}

/// The round trip of one STUN Binding (RFC 8489) over UDP to `server`:
/// a request of the header alone, the answer told by its transaction id.
/// `None` when nothing came in time.
pub async fn stun_rtt(server: SocketAddr) -> Option<Duration> {
    let any = if server.is_ipv6() { "[::]:0" } else { "0.0.0.0:0" };
    let socket = UdpSocket::bind(any).await.ok()?;
    let mut request = [0u8; 20];
    request[0..2].copy_from_slice(&0x0001u16.to_be_bytes());
    request[4..8].copy_from_slice(&0x2112_A442u32.to_be_bytes());
    getrandom::fill(&mut request[8..20]).ok()?;
    let started = Instant::now();
    socket.send_to(&request, server).await.ok()?;
    let mut buf = [0u8; 512];
    loop {
        let left = STUN_TIMEOUT.checked_sub(started.elapsed())?;
        let (n, from) = tokio::time::timeout(left, socket.recv_from(&mut buf)).await.ok()?.ok()?;
        if from == server && n >= 20 && buf[0..2] == [0x01, 0x01] && buf[8..20] == request[8..20] {
            return Some(started.elapsed());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds(urls: &[&str]) -> TurnCredentials {
        TurnCredentials {
            username: "1760000000:ab12".into(),
            password: "pw".into(),
            realm: "veydan".into(),
            ttl_secs: 600,
            expires_at: 1_760_000_000,
            urls: urls.iter().map(|u| u.to_string()).collect(),
        }
    }

    #[test]
    fn credentials_become_ice_servers() {
        let c = creds(&["stun:203.0.113.7:3478", "turn:203.0.113.7:3478?transport=udp", "turns:203.0.113.7:443?transport=tcp"]);
        let servers = ice_servers_of(&c);
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].urls, vec!["stun:203.0.113.7:3478"]);
        assert_eq!(servers[0].username, None);
        assert_eq!(servers[1].urls, vec!["turn:203.0.113.7:3478?transport=udp", "turns:203.0.113.7:443?transport=tcp"]);
        assert_eq!(servers[1].username.as_deref(), Some("1760000000:ab12"));
        assert_eq!(servers[1].credential.as_deref(), Some("pw"));
        assert_eq!(stun_addr(&c), Some("203.0.113.7:3478".parse().unwrap()));
        assert_eq!(stun_addr(&creds(&["turn:203.0.113.7:3478?transport=udp"])), None);
        assert!(ice_servers_of(&creds(&[])).is_empty());
    }

    #[test]
    fn hello_is_what_the_node_reads() {
        let h = hello(Access { key: Some("k".into()), device: None }, "veydan-chat/5.1.0");
        let json = serde_json::to_value(&h).unwrap();
        assert_eq!(json["protocol_max"], 1);
        assert_eq!(json["access"], serde_json::json!({ "key": "k" }), "a shared key goes as before");
        assert_eq!(serde_json::to_value(hello(Access::default(), "x")).unwrap()["access"], serde_json::json!({}));
        let w: Welcome = serde_json::from_str(r#"{"protocol_min":1,"protocol_max":1,"node_id":"ab","limits":{"turn_lifetime_secs":600}}"#).unwrap();
        assert_eq!(w.limits.turn_lifetime_secs, 600);
        assert!(!w.private);
        assert_eq!(w.load, None, "a node of before says nothing of its load");

        // The credentials of this device: the id and the secret apart.
        let mine = CallNode::with_key(format!("127.0.0.1:1#{}", "ab".repeat(32)).parse().unwrap(), NodeClass::Own, Some(crate::servers::device_key("d-7", "s3")));
        assert_eq!(serde_json::to_value(Access::of(&mine)).unwrap(), serde_json::json!({ "device": { "id": "d-7", "secret": "s3" } }));
        let w: Welcome = serde_json::from_str(
            r#"{"protocol_min":1,"protocol_max":1,"node_id":"ab","private":true,"load":{"allocations":500,"max_allocations":1000,"participants":40,"max_participants":40}}"#,
        )
        .unwrap();
        assert_eq!(w.load.unwrap().percent(), 100);
        assert!(w.load.unwrap().full());
        assert_eq!(NodeLoad { mbps: 30, max_mbps: 100, ..NodeLoad::default() }.percent(), 30);
        assert_eq!(NodeLoad { mbps: 30, ..NodeLoad::default() }.percent(), 0, "no limit said: empty");
    }

    #[test]
    fn credentials_are_good_by_my_clock_from_when_they_came() {
        let fetched = Instant::now();
        // The node's ttl, less the margin; its expires_at is not read.
        let mut c = creds(&["stun:203.0.113.7:3478"]);
        c.expires_at = 1;
        assert_eq!(good_until(fetched, &c, 1_760_000_000), fetched + Duration::from_secs(600 - CREDENTIALS_MARGIN_SECS as u64));
        // No ttl told: expires_at against now, the best there is.
        c.ttl_secs = 0;
        c.expires_at = 1_760_000_300;
        assert_eq!(good_until(fetched, &c, 1_760_000_000), fetched + Duration::from_secs(300 - CREDENTIALS_MARGIN_SECS as u64));
        // Shorter than the margin: not good at all.
        c.ttl_secs = 30;
        assert_eq!(good_until(fetched, &c, 0), fetched);
    }

    /// A node of `class` on `port`, its id (the key of the cache) its own.
    fn node(class: NodeClass, port: u16) -> CallNode {
        CallNode::new(format!("127.0.0.1:{port}#{}", format!("{:02x}", port % 256).repeat(32)).parse().unwrap(), class)
    }

    /// A client whose nodes answer or refuse by their port (1 refuses;
    /// 5 is private and lets nobody in; 6 lets the device `d-6` in and
    /// nobody else), counting who was asked. Ports 10–99 are nodes at
    /// 10 ms (one step of the rule) whose load is the port's second digit
    /// in tens of percent, 9 full. Other ports: the round trip in ms.
    fn fake_client(asked: Arc<Mutex<Vec<u16>>>) -> NodeClient {
        NodeClient::with_fetch(
            "test",
            Arc::new(move |node, _| {
                let asked = asked.clone();
                Box::pin(async move {
                    let port = node.node.addr.port();
                    asked.lock().unwrap().push(port);
                    if port == 1 {
                        return Err(MessengerError::Transport("refused".into()));
                    }
                    let private = matches!(port, 5 | 6);
                    let load = (10..100).contains(&port).then(|| {
                        let tenths = (port % 10) as u32;
                        NodeLoad { participants: tenths * 10, max_participants: 90, ..NodeLoad::default() }
                    });
                    let welcome = Welcome {
                        protocol_min: 1,
                        protocol_max: 1,
                        node_id: node.node.id.to_string(),
                        version: String::new(),
                        // Nodes on a port below 100 have an SFU.
                        capabilities: if port < 100 { vec![CAP_SFU.into()] } else { vec![] },
                        codecs: vec![],
                        private,
                        limits: Limits { turn_lifetime_secs: 600, ..Limits::default() },
                        load,
                    };
                    let let_in = match port {
                        5 => false,
                        6 => node.device().is_some_and(|(id, secret)| id == "d-6" && secret == "s6"),
                        _ => true,
                    };
                    let c = if let_in { creds(&[&format!("turn:127.0.0.1:{port}?transport=udp")]) } else { TurnCredentials::none() };
                    let rtt = if (10..100).contains(&port) { 10 } else { port as u64 };
                    Ok((welcome, c, Duration::from_millis(rtt)))
                })
            }),
        )
    }

    #[tokio::test]
    async fn among_nodes_as_near_the_less_loaded_is_taken_and_a_full_one_last() {
        // All at 10 ms: loads 70 %, 30 %, full (90 of 90), 50 %, and a
        // node of before that says nothing of its load (counts as empty).
        let asked = Arc::new(Mutex::new(vec![]));
        let client = fake_client(asked.clone());
        let nodes = [node(NodeClass::Project, 17), node(NodeClass::Project, 13), node(NodeClass::Project, 19), node(NodeClass::Project, 15)];
        let picked = client.pick(&nodes, RelayPolicy::Auto, 0).await.unwrap();
        assert_eq!(picked.servers[0].urls, vec!["turn:127.0.0.1:13?transport=udp"], "the least loaded of a step");
        assert_eq!(picked.servers[1].urls, vec!["turn:127.0.0.1:15?transport=udp"]);
        assert_eq!(client.pick_sfu(&nodes, 0).await.unwrap().node.node.addr.port(), 13);
        // A nearer step wins over a lighter load: 2 ms against 10 ms.
        let nodes = [node(NodeClass::Project, 17), node(NodeClass::Project, 2)];
        assert_eq!(client.pick_sfu(&nodes, 0).await.unwrap().node.node.addr.port(), 2);
        // Within a step, equal loads: the order of the sets (the
        // registry lists the least loaded first).
        let nodes = [node(NodeClass::Volunteer, 24), node(NodeClass::Volunteer, 14)];
        assert_eq!(client.pick_sfu(&nodes, 0).await.unwrap().node.node.addr.port(), 24);
        // A node that says nothing of its load counts as empty.
        let nodes = [node(NodeClass::Project, 13), node(NodeClass::Project, 7)];
        assert_eq!(client.pick_sfu(&nodes, 0).await.unwrap().node.node.addr.port(), 7);
        // Full (degraded) only when nobody else of the class answered.
        let nodes = [node(NodeClass::Project, 19), node(NodeClass::Project, 1)];
        assert_eq!(client.pick_sfu(&nodes, 0).await.unwrap().node.node.addr.port(), 19);
        let known = client.known();
        let k = |port: u16| known.iter().find(|k| k.id.starts_with(&format!("{:02x}", port % 256))).unwrap();
        assert_eq!((k(19).state.as_str(), k(19).load, k(19).rtt_ms), ("degraded", Some(100), 10));
        assert_eq!((k(13).state.as_str(), k(13).load), ("active", Some(33)), "30 of 90");
        assert_eq!(k(7).load, None);
    }

    #[tokio::test]
    async fn a_node_that_does_not_let_me_in_is_there_but_not_chosen() {
        let asked = Arc::new(Mutex::new(vec![]));
        let client = fake_client(asked.clone());
        // My private node without a key that holds: HELLO answers, TURN
        // does not; the class below is asked then.
        let mine = CallNode::with_key(node(NodeClass::Own, 5).node, NodeClass::Own, Some("old-key".into()));
        let access = client.access(&mine, 0).await.unwrap();
        assert!(access.welcome.private && !access.authorized && access.credentials.urls.is_empty());
        let picked = client.pick(&[mine.clone(), node(NodeClass::Project, 3000)], RelayPolicy::Auto, 0).await.unwrap();
        assert_eq!(picked.servers[0].urls, vec!["turn:127.0.0.1:3000?transport=udp"], "fallen back to the project's");
        assert!(client.pick_sfu(std::slice::from_ref(&mine), 0).await.is_none());
        assert!(!client.known()[0].authorized);
        // Nothing of it is kept as good: the next pick asks it again.
        let before = asked.lock().unwrap().len();
        client.pick(std::slice::from_ref(&mine), RelayPolicy::Auto, 0).await.unwrap();
        assert_eq!(asked.lock().unwrap().len(), before + 1);
        assert!(client.pick(&[mine], RelayPolicy::RelayOnly, 0).await.is_err());

        // The credentials of this device open the node 6; another
        // device's, or a revoked one's, do not.
        let invited = CallNode::with_key(node(NodeClass::Own, 6).node, NodeClass::Own, Some(crate::servers::device_key("d-6", "s6")));
        let picked = client.pick(&[invited, node(NodeClass::Project, 3000)], RelayPolicy::Auto, 0).await.unwrap();
        assert_eq!(picked.servers[0].urls, vec!["turn:127.0.0.1:6?transport=udp"]);
        let revoked = CallNode::with_key(node(NodeClass::Own, 6).node, NodeClass::Own, Some(crate::servers::device_key("d-6", "gone")));
        let picked = client.pick(&[revoked, node(NodeClass::Project, 3000)], RelayPolicy::Auto, 0).await.unwrap();
        assert_eq!(picked.servers[0].urls, vec!["turn:127.0.0.1:3000?transport=udp"]);
    }

    #[tokio::test]
    async fn an_invitation_is_exchanged_for_the_credentials_of_this_device() {
        let issued = Arc::new(Mutex::new(vec![]));
        let seen = issued.clone();
        let client = NodeClient::with_fakes(
            "veydan-chat/5.1.8",
            Arc::new(|_, _| Box::pin(async { Err(MessengerError::Transport("not here".into())) })),
            Arc::new(move |node, token, name, client| {
                let seen = seen.clone();
                Box::pin(async move {
                    seen.lock().unwrap().push((node.to_string(), token.clone(), name, client));
                    match token.as_str() {
                        "inv-1" => Ok(DeviceIssued { device_id: "d-1".into(), secret: "s-1".into(), label: "Anna's phone".into(), node: node.to_string() }),
                        "other" => Ok(DeviceIssued { device_id: "d-1".into(), secret: "s-1".into(), label: String::new(), node: format!("{}#{}", node.addr, "ff".repeat(32)) }),
                        "empty" => Ok(DeviceIssued { device_id: String::new(), secret: String::new(), label: String::new(), node: String::new() }),
                        "old" => Err(NodeError::Refused { status: 404, error: "http".into(), message: "status 404".into() }),
                        _ => Err(NodeError::Refused { status: 403, error: "bad_invite".into(), message: "used up".into() }),
                    }
                })
            }),
        );
        let n = node(NodeClass::Own, 2000).node;
        let creds = client.redeem_invite(&n, "inv-1", "Veydan Chat, Android", 77).await.unwrap();
        assert_eq!(
            creds,
            DeviceCredentials {
                node: n.clone(),
                device_id: "d-1".into(),
                secret: "s-1".into(),
                label: "Anna's phone".into(),
                name: "Veydan Chat, Android".into(),
                added_at: 77
            }
        );
        assert_eq!(creds.key(), crate::servers::device_key("d-1", "s-1"));
        assert_eq!(serde_json::to_value(InviteRequest { token: "t".into(), name: "n".into() }).unwrap(), serde_json::json!({ "token": "t", "name": "n" }));
        assert_eq!(issued.lock().unwrap()[0], (n.to_string(), "inv-1".to_string(), "Veydan Chat, Android".to_string(), "veydan-chat/5.1.8".to_string()));
        assert!(client.redeem_invite(&n, "other", "", 0).await.unwrap_err().to_string().contains("another node"));
        assert!(client.redeem_invite(&n, "empty", "", 0).await.is_err());
        assert!(client.redeem_invite(&n, "old", "", 0).await.unwrap_err().to_string().contains("takes no invitations"));
        assert!(client.redeem_invite(&n, "used", "", 0).await.unwrap_err().to_string().contains("bad_invite"));
        let r = NodeError::Refused { status: 403, error: "device_revoked".into(), message: String::new() };
        assert!(r.is_not_let_in() && !NodeError::Refused { status: 429, error: "rate_limited".into(), message: String::new() }.is_not_let_in());
    }

    #[tokio::test]
    async fn a_lower_class_is_not_asked_while_a_higher_one_answers() {
        let asked = Arc::new(Mutex::new(vec![]));
        let client = fake_client(asked.clone());
        let nodes = [node(NodeClass::Project, 3000), node(NodeClass::Own, 2000), node(NodeClass::Volunteer, 4000)];
        let picked = client.pick(&nodes, RelayPolicy::Auto, 0).await.unwrap();
        assert_eq!(picked.nodes.len(), 1);
        assert_eq!(picked.servers[0].urls, vec!["turn:127.0.0.1:2000?transport=udp"]);
        assert_eq!(*asked.lock().unwrap(), vec![2000], "my own node answered: nobody else saw the call");

        // My own node is down: the project's is asked then, and the
        // volunteer's still not.
        let asked = Arc::new(Mutex::new(vec![]));
        let client = fake_client(asked.clone());
        let nodes = [node(NodeClass::Own, 1), node(NodeClass::Project, 3000), node(NodeClass::Volunteer, 4000)];
        let picked = client.pick(&nodes, RelayPolicy::Auto, 0).await.unwrap();
        assert_eq!(picked.servers[0].urls, vec!["turn:127.0.0.1:3000?transport=udp"]);
        assert_eq!(*asked.lock().unwrap(), vec![1, 3000]);

        // Within a class every node is asked, and the nearest two are taken.
        let asked = Arc::new(Mutex::new(vec![]));
        let client = fake_client(asked.clone());
        let nodes = [node(NodeClass::Project, 3300), node(NodeClass::Project, 3100), node(NodeClass::Project, 3200)];
        let picked = client.pick(&nodes, RelayPolicy::Auto, 0).await.unwrap();
        assert_eq!(picked.servers.len(), 2);
        assert_eq!(picked.servers[0].urls, vec!["turn:127.0.0.1:3100?transport=udp"]);
        assert_eq!(picked.servers[1].urls, vec!["turn:127.0.0.1:3200?transport=udp"]);
        let mut all = asked.lock().unwrap().clone();
        all.sort_unstable();
        assert_eq!(all, vec![3100, 3200, 3300]);
    }

    #[tokio::test]
    async fn a_room_is_made_on_the_nearest_node_with_an_sfu() {
        // Port 2 answers with an SFU, 3000 without (see `fake_client`):
        // my own node has no SFU, so the project's is taken; nobody
        // answers with one among two classes: none.
        let asked = Arc::new(Mutex::new(vec![]));
        let client = fake_client(asked.clone());
        let nodes = [node(NodeClass::Own, 3000), node(NodeClass::Project, 2), node(NodeClass::Project, 4)];
        let picked = client.pick_sfu(&nodes, 0).await.unwrap();
        assert_eq!(picked.node.node.addr.port(), 2, "the nearest of the two with an SFU");
        assert!(client.pick_sfu(&[node(NodeClass::Own, 3000), node(NodeClass::Volunteer, 1)], 0).await.is_none());
        let e = NodeError::Refused { status: 404, error: "room_not_found".into(), message: "x".into() };
        assert_eq!((e.word(), e.status()), (Some("room_not_found"), Some(404)));
        assert!(MessengerError::from(e).to_string().contains("room_not_found"));
        assert!(NodeError::Unreachable("down".into()).word().is_none());
        assert_eq!(serde_json::to_value(RoomRequest::default()).unwrap(), serde_json::json!({ "access": {}, "media_limits": {} }));
        let joined: Joined = serde_json::from_str(r#"{"sdp_answer":"v=0","participant_id":2,"participant_token":"t"}"#).unwrap();
        assert!(joined.participants.is_empty());
    }

    #[tokio::test]
    async fn the_cache_serves_by_my_clock_and_the_node_is_asked_once() {
        let asked = Arc::new(Mutex::new(vec![]));
        let client = fake_client(asked.clone());
        let n = node(NodeClass::Own, 2000);
        // The node's expires_at is in the past by its clock (or mine is
        // ahead of it): the ttl it told is what counts.
        let first = client.access(&n, 1_760_000_000).await.unwrap();
        assert!(first.good_until > Instant::now());
        let again = client.access(&n, 1_760_000_000 + 5000).await.unwrap();
        assert_eq!(again.credentials.username, first.credentials.username);
        assert_eq!(asked.lock().unwrap().len(), 1, "the cache answered the second time");
        client.clear();
        client.access(&n, 0).await.unwrap();
        assert_eq!(asked.lock().unwrap().len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn without_nodes_auto_goes_alone_and_relay_only_cannot() {
        let client = NodeClient::new("test");
        assert_eq!(client.pick(&[], RelayPolicy::Auto, 0).await.unwrap(), Picked::default());
        assert!(client.pick(&[], RelayPolicy::RelayOnly, 0).await.is_err());
    }

    #[tokio::test]
    async fn a_node_that_is_not_there_is_left_out() {
        // Port 1 on loopback: refused at once, no waiting.
        let node = CallNode::new(format!("127.0.0.1:1#{}", "ab".repeat(32)).parse().unwrap(), NodeClass::Own);
        let client = NodeClient::new("test");
        assert!(client.access(&node, 0).await.is_err());
        assert_eq!(client.pick(std::slice::from_ref(&node), RelayPolicy::Auto, 0).await.unwrap(), Picked::default());
        assert!(client.pick(&[node], RelayPolicy::RelayOnly, 0).await.is_err());
    }

    /// Against a live node: `VEYDAN_CALL_NODE=address:port#id cargo test
    /// -p messenger-calls -- --ignored live_node`. The node's id is
    /// pinned; a wrong id fails the TLS handshake.
    #[tokio::test]
    #[ignore]
    async fn live_node_hands_out_credentials_once_and_the_cache_serves_the_rest() {
        let Ok(reference) = std::env::var("VEYDAN_CALL_NODE") else { return };
        let node = CallNode::new(reference.parse().unwrap(), NodeClass::Own);
        let client = NodeClient::new("messenger-calls/test");
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let access = client.access(&node, now).await.unwrap();
        assert_eq!(access.welcome.node_id, node.node.id.to_string());
        assert!(access.credentials.expires_at as i64 > now + CREDENTIALS_MARGIN_SECS);
        assert!(access.credentials.urls.iter().any(|u| u.starts_with("turn:")), "{:?}", access.credentials.urls);
        assert!(access.welcome.limits.turn_lifetime_secs > 0);
        eprintln!("node {} rtt {:?} urls {:?} limits {:?}", node.node.id.short(), access.rtt, access.credentials.urls, access.welcome.limits);
        // The same credentials again, without the node.
        let again = client.access(&node, now).await.unwrap();
        assert_eq!(again.credentials.username, access.credentials.username);
        let picked = client.pick(std::slice::from_ref(&node), RelayPolicy::RelayOnly, now).await.unwrap();
        assert_eq!(picked.nodes, vec![node.node.id.to_string()]);
        assert_eq!(picked.servers.len(), 2, "stun, then turn with credentials");
        assert_eq!(picked.servers[1].username.as_deref(), Some(access.credentials.username.as_str()));

        // Another id for the same address: the pin refuses the chain.
        let wrong = CallNode::new(format!("{}#{}", node.node.addr, "ab".repeat(32)).parse().unwrap(), NodeClass::Own);
        let err = client.access(&wrong, now).await.unwrap_err();
        assert!(err.to_string().contains("tls"), "{err}");
    }
}
