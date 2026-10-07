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
use crate::servers::{CallNode, NodeClass};
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
/// A request or an answer is a few hundred bytes; more is not one.
const MAX_BODY: usize = 64 * 1024;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Access {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
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
    pub credentials: TurnCredentials,
    /// The round trip to the node, as measured when the credentials were
    /// fetched.
    pub rtt: Duration,
    /// Until when the credentials are used, by this device's clock: their
    /// `ttl_secs` from the moment they came, less a margin.
    pub good_until: Instant,
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

/// What `fetch` gives: WELCOME, the credentials, the round trip.
type Fetched = (Welcome, TurnCredentials, Duration);
/// The way to a node; the tests put a fake one in.
type Fetch = Arc<dyn Fn(CallNode, String) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Fetched>> + Send>> + Send + Sync>;

struct Inner {
    client_name: String,
    cache: Mutex<HashMap<BridgeId, NodeAccess>>,
    fetch: Fetch,
}

#[derive(Clone)]
pub struct NodeClient {
    inner: Arc<Inner>,
}

impl NodeClient {
    /// `client_name` goes into HELLO for the node's log: `veydan-chat/5.1.0`.
    pub fn new(client_name: &str) -> Self {
        Self::with_fetch(client_name, Arc::new(|node, name| Box::pin(async move { fetch(&node, &name).await })))
    }

    fn with_fetch(client_name: &str, fetch: Fetch) -> Self {
        Self { inner: Arc::new(Inner { client_name: client_name.to_string(), cache: Mutex::new(HashMap::new()), fetch }) }
    }

    /// Forget every credential (a logout, a test).
    pub fn clear(&self) {
        self.inner.cache.lock().unwrap().clear();
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
        let good_until = good_until(fetched, &credentials, now);
        let access = NodeAccess { node: node.clone(), welcome, credentials, rtt, good_until };
        self.inner.cache.lock().unwrap().insert(node.node.id, access.clone());
        Ok(access)
    }

    /// The ICE servers of a call: of the first class of `nodes` (which
    /// come in order of priority) that has a node answering, the one or
    /// two nearest. The classes are asked one after another, and a class
    /// is not spoken to while a higher one answers. Without a node a call
    /// under `Auto` goes with host candidates alone; under `RelayOnly`
    /// there is nothing to relay through, and that is an error.
    pub async fn pick(&self, nodes: &[CallNode], policy: RelayPolicy, now: i64) -> Result<Picked> {
        let mut answered: Vec<NodeAccess> = Vec::new();
        let mut classes: Vec<NodeClass> = nodes.iter().map(|n| n.class).collect();
        classes.sort_unstable();
        classes.dedup();
        let deadline = tokio::time::Instant::now() + PICK_TIMEOUT;
        for class in classes {
            let mut tasks = tokio::task::JoinSet::new();
            for node in nodes.iter().filter(|n| n.class == class) {
                let (client, node) = (self.clone(), node.clone());
                tasks.spawn(async move { client.access(&node, now).await });
            }
            let class_deadline = deadline.min(tokio::time::Instant::now() + CLASS_TIMEOUT);
            while let Ok(Some(joined)) = tokio::time::timeout_at(class_deadline, tasks.join_next()).await {
                if let Ok(Ok(access)) = joined {
                    answered.push(access);
                }
            }
            if !answered.is_empty() {
                break;
            }
        }
        answered.sort_by_key(|a| (a.node.class, a.rtt));
        let Some(best) = answered.first().map(|a| a.node.class) else {
            return if policy == RelayPolicy::RelayOnly {
                Err(MessengerError::Transport("no call node answered: nothing to relay through".into()))
            } else {
                Ok(Picked::default())
            };
        };
        let chosen: Vec<&NodeAccess> = answered.iter().filter(|a| a.node.class == best).take(NODES_PER_CALL).collect();
        Ok(Picked {
            servers: chosen.iter().flat_map(|a| ice_servers_of(&a.credentials)).collect(),
            nodes: chosen.iter().map(|a| a.node.node.id.to_string()).collect(),
            limits: chosen.first().map(|a| a.welcome.limits.clone()),
        })
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
async fn fetch(node: &CallNode, client_name: &str) -> Result<Fetched> {
    let short = node.node.id.short();
    let short = short.as_str();
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(node.node.addr))
        .await
        .map_err(|_| MessengerError::Transport(format!("node {short}: no connection in time")))?
        .map_err(|e| MessengerError::Transport(format!("node {short}: {e}")))?;
    let _ = tcp.set_nodelay(true);
    let config = client_config(node.node.id)?;
    let name = pin::server_name(&node.node).map_err(|e| MessengerError::Crypto(e.to_string()))?;
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

    let access = Access { key: node.access_key.clone() };
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
    let credentials: TurnCredentials = post(&send, PATH_TURN, &TurnRequest { access }, short).await?;
    let rtt = match stun_addr(&credentials) {
        Some(addr) => stun_rtt(addr).await.unwrap_or(hello_rtt),
        None => hello_rtt,
    };
    Ok((welcome, credentials, rtt))
}

/// One request; a status other than 200 is the node's refusal.
async fn post<T: Serialize, R: DeserializeOwned>(
    send: &h2::client::SendRequest<Bytes>,
    path: &str,
    body: &T,
    short: &str,
) -> Result<R> {
    let mut send = send.clone().ready().await.map_err(transport)?;
    let request = http::Request::builder()
        .method(http::Method::POST)
        .uri(path)
        .header("content-type", "application/json")
        .body(())
        .map_err(transport)?;
    let (response, mut stream) = send.send_request(request, false).map_err(transport)?;
    stream.send_data(Bytes::from(serde_json::to_vec(body)?), true).map_err(transport)?;
    let response = tokio::time::timeout(REQUEST_TIMEOUT, response)
        .await
        .map_err(|_| MessengerError::Transport(format!("node {short}: no answer in time")))?
        .map_err(transport)?;
    let status = response.status().as_u16();
    let body = tokio::time::timeout(REQUEST_TIMEOUT, read_body(response.into_body()))
        .await
        .map_err(|_| MessengerError::Transport(format!("node {short}: no answer in time")))??;
    if status == 200 {
        return serde_json::from_slice(&body).map_err(|e| MessengerError::Transport(format!("node {short}: {e}")));
    }
    let refusal: Refusal = serde_json::from_slice(&body).unwrap_or(Refusal { error: "http".into(), message: format!("status {status}") });
    Err(MessengerError::Transport(format!("node {short}: {status} {}: {}", refusal.error, refusal.message)))
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
        let h = hello(Access { key: Some("k".into()) }, "veydan-chat/5.1.0");
        let json = serde_json::to_value(&h).unwrap();
        assert_eq!(json["protocol_max"], 1);
        assert_eq!(json["access"]["key"], "k");
        assert_eq!(serde_json::to_value(hello(Access::default(), "x")).unwrap()["access"], serde_json::json!({}));
        let w: Welcome = serde_json::from_str(r#"{"protocol_min":1,"protocol_max":1,"node_id":"ab","limits":{"turn_lifetime_secs":600}}"#).unwrap();
        assert_eq!(w.limits.turn_lifetime_secs, 600);
        assert!(!w.private);
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

    /// A client whose nodes answer or refuse by their port (1 refuses),
    /// counting who was asked.
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
                    let welcome = Welcome {
                        protocol_min: 1,
                        protocol_max: 1,
                        node_id: node.node.id.to_string(),
                        version: String::new(),
                        capabilities: vec![],
                        codecs: vec![],
                        private: false,
                        limits: Limits { turn_lifetime_secs: 600, ..Limits::default() },
                    };
                    let c = creds(&[&format!("turn:127.0.0.1:{port}?transport=udp")]);
                    Ok((welcome, c, Duration::from_millis(port as u64)))
                })
            }),
        )
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
