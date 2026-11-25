// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The diagnosis of the network, for a report of a bug (`crate::bug`) and
//! for the network panel: what this device reaches of the project's
//! servers right now, each way measured on its own.
//!
//! - `direct`: every host of the manifest in use, by one HTTPS request on a
//!   client bound to a rule of its own that sends nothing to a bridge (as
//!   [`NetService`]'s check does); any HTTP answer counts as reached;
//! - `canary`: the 256 KB push to a registry, the same direct way;
//! - `bridges`: every bridge known (at most [`MAX_BRIDGES`]), each on a
//!   client of its own that knows only that bridge, by the probe of 256 KB;
//! - `calls`: the call nodes a call may use now that are the project's or
//!   a volunteer's (never one of mine; at most [`MAX_CALL_NODES`]), by
//!   HELLO and one STUN binding.
//!
//! Everything goes at once, under one deadline ([`DEADLINE`]): what did
//! not finish by then is `ok: false`, `error: "timeout"`. With the user's
//! own servers, or none chosen yet, nothing is asked at all: the answer
//! carries the choice and empty lists.

use super::{host_of, NetMode, NetService, BRIDGE_PATIENCE, CHECK_BYTES, DIRECT_PATIENCE};
use crate::call_nodes::CallNodeClass;
use crate::relays::ServersMode;
use crate::MessengerRuntime;
use messenger_calls::servers::{CallNode, NodeClass};
use messenger_calls::{NodeClient, TrustLevel};
use messenger_core::Result;
use messenger_transport::manifest::ManifestSource;
use messenger_transport::Manifest;
use messenger_vlink::{probe, BridgeRef, Client, Net};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::task::JoinSet;
use tokio::time::Instant;
use ts_rs::TS;

/// The version of the diagnosis, its `v`.
pub const DIAGNOSIS_VERSION: u8 = 1;
/// The whole diagnosis waits no longer than this.
pub const DEADLINE: Duration = Duration::from_secs(45);
/// Bridges tried at most: the first of the known, in the order a client
/// tries them.
pub const MAX_BRIDGES: usize = 8;
/// Call nodes asked at most.
pub const MAX_CALL_NODES: usize = 8;
/// An error word is cut to this many characters.
pub const MAX_ERROR_CHARS: usize = 80;
/// How long one host of the direct way is given.
const HOST_PATIENCE: Duration = Duration::from_secs(10);
const CONNECT_PATIENCE: Duration = Duration::from_secs(5);
/// How long one call node is given: its connection, HELLO, the
/// credentials and the STUN binding.
const NODE_PATIENCE: Duration = Duration::from_secs(15);

/// Which servers the user chose, as the diagnosis tells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosisServers {
    /// The project's: the diagnosis is made.
    Veydan,
    /// The user's own: nothing is asked.
    Own,
    /// None chosen yet: nothing is asked.
    Unchosen,
}

/// What a host of the project is to the manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum HostKind {
    Relay,
    Media,
    Push,
    Registry,
    /// Where the manifest itself is fetched.
    Source,
}

/// One host of the project, reached the direct way or not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct DirectCheck {
    /// The host name alone, as the manifest names it.
    pub host: String,
    pub kind: HostKind,
    /// Any HTTP answer came.
    pub ok: bool,
    /// How long the answer took, milliseconds.
    pub ms: Option<u32>,
    /// Why nothing came: `timeout`, `dns`, `tls`, `refused`, `reset` or
    /// `other`.
    pub error: Option<String>,
}

/// The push of [`CHECK_BYTES`] to a registry, the direct way.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CanaryCheck {
    /// A registry took the bytes and answered with success.
    pub ok: bool,
    pub ms: Option<u32>,
    /// What the test pushes.
    pub bytes: u32,
}

/// One bridge, tried on its own: a connection and the probe of
/// [`CHECK_BYTES`] each way.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct BridgeCheck {
    /// The address of the bridge (`address:port`) when a public list
    /// names it, [`PRIVATE_BRIDGE`] for one the user added; never its id.
    pub addr: String,
    pub ok: bool,
    pub ms: Option<u32>,
    /// What the probe moves each way.
    pub bytes: u32,
    /// Why it failed, as [`DirectCheck::error`].
    pub error: Option<String>,
}

/// One call node of the project or of a volunteer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CallNodeCheck {
    /// The address of the node (`address:port`), from the manifest or the
    /// registry's list.
    pub addr: String,
    /// The region the registry names for it, when it does.
    pub region: Option<String>,
    pub class: CallNodeClass,
    pub hello_ok: bool,
    pub hello_ms: Option<u32>,
    pub stun_ok: bool,
    pub stun_ms: Option<u32>,
}

/// The diagnosis of the network (`messenger_net_diagnose`).
///
/// It carries nothing of the user: no key and no npub, no address of this
/// device, no relay or server of the user's own, nothing of a chat. Only
/// the hosts of the project's manifest, the addresses of bridges and of
/// call nodes (public lists, or a bridge the user added for the project's
/// servers), the choices of the network panel and timings; an error is
/// one word of a fixed few, never the text of the system.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NetDiagnosis {
    pub v: u8,
    /// When it was made, unix seconds.
    #[ts(type = "number")]
    pub at: i64,
    pub servers_mode: DiagnosisServers,
    pub mode: NetMode,
    /// The project's servers are reached through a bridge now.
    pub active: bool,
    /// The address of the bridge in use, while a connection to one is up.
    pub bridge_in_use: Option<String>,
    pub direct: Vec<DirectCheck>,
    /// None when the manifest names no registry.
    pub canary: Option<CanaryCheck>,
    pub bridges: Vec<BridgeCheck>,
    pub calls: Vec<CallNodeCheck>,
}

/// A host of the project and the address a request for it goes to: its
/// scheme made HTTPS (a relay's `wss`), its port kept, the path `/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Endpoint {
    pub host: String,
    pub kind: HostKind,
    pub url: String,
}

/// What a diagnosis asks.
#[derive(Clone, Debug, Default)]
pub(crate) struct Plan {
    pub endpoints: Vec<Endpoint>,
    pub registries: Vec<String>,
    pub bridges: Vec<BridgeRef>,
}

/// The hosts of `manifest` (those of [`super::project_hosts`]), each once,
/// with what it first is to the manifest: a relay, a media server, a push
/// server, a source of the manifest, a registry.
pub(crate) fn endpoints(manifest: &Manifest) -> Vec<Endpoint> {
    let sources = manifest.sources.iter().filter_map(|s| match s {
        ManifestSource::Http { url, .. } => Some(url.as_str()),
        ManifestSource::Nostr { .. } => None,
    });
    let all = manifest
        .relays
        .iter()
        .map(|r| (r.url.as_str(), HostKind::Relay))
        .chain(
            manifest
                .media
                .iter()
                .map(|m| (m.url.as_str(), HostKind::Media)),
        )
        .chain(
            manifest
                .push
                .iter()
                .map(|p| (p.url.as_str(), HostKind::Push)),
        )
        .chain(sources.map(|u| (u, HostKind::Source)))
        .chain(
            manifest
                .registries
                .iter()
                .map(|r| (r.as_str(), HostKind::Registry)),
        );
    let mut out: Vec<Endpoint> = Vec::new();
    for (url, kind) in all {
        let (Some(host), Some(url)) = (host_of(url), root_of(url)) else {
            continue;
        };
        if !out.iter().any(|e| e.host == host) {
            out.push(Endpoint { host, kind, url });
        }
    }
    out
}

/// `wss://host:port/path?q` → `https://host:port/`: what a request of the
/// diagnosis asks of a host; nothing of the address beyond the host and
/// its port.
pub(super) fn root_of(url: &str) -> Option<String> {
    let mut url = reqwest::Url::parse(url.trim()).ok()?;
    let scheme = match url.scheme() {
        "wss" | "https" => "https",
        "ws" | "http" => "http",
        _ => return None,
    };
    url.set_scheme(scheme).ok()?;
    url.set_username("").ok()?;
    url.set_password(None).ok()?;
    url.set_path("/");
    url.set_query(None);
    url.set_fragment(None);
    Some(url.to_string())
}

fn ms(d: Duration) -> Option<u32> {
    Some(d.as_millis().min(u32::MAX as u128) as u32)
}

/// The one word of an error, out of its text and the texts of its causes:
/// never the text itself, which may carry an address.
pub(crate) fn word_of(error: &(dyn std::error::Error + 'static)) -> String {
    let mut text = error.to_string();
    let mut cause = error.source();
    while let Some(e) = cause {
        text.push(' ');
        text += &e.to_string();
        cause = e.source();
    }
    let text = text.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| text.contains(w));
    let word = if has(&["timed out", "timeout", "deadline"]) {
        "timeout"
    } else if has(&[
        "dns",
        "resolve",
        "lookup",
        "name or service",
        "no such host",
        "nodename",
    ]) {
        "dns"
    } else if has(&["certificate", "tls", "ssl", "handshake", "alert"]) {
        "tls"
    } else if has(&["refused"]) {
        "refused"
    } else if has(&["reset", "broken pipe", "aborted"]) {
        "reset"
    } else {
        "other"
    };
    cap(word)
}

/// An error word, at most [`MAX_ERROR_CHARS`] characters.
fn cap(word: &str) -> String {
    word.chars().take(MAX_ERROR_CHARS).collect()
}

const TIMEOUT: &str = "timeout";

/// The answers of `set`, in the order of their indices; a task that did not
/// give one (it panicked, or was cut) is `missing`.
async fn gather<T: 'static>(
    mut set: JoinSet<(usize, T)>,
    n: usize,
    missing: impl Fn(usize) -> T,
) -> Vec<T> {
    let mut out: Vec<Option<T>> = (0..n).map(|_| None).collect();
    while let Some(joined) = set.join_next().await {
        if let Ok((i, answer)) = joined {
            out[i] = Some(answer);
        }
    }
    out.into_iter()
        .enumerate()
        .map(|(i, a)| a.unwrap_or_else(|| missing(i)))
        .collect()
}

/// A client of the direct way: a rule of its own, which sends nothing to a
/// bridge; no redirect is followed, as any answer counts.
fn direct_client(total: Duration) -> Option<reqwest::Client> {
    messenger_http::builder_on(Net::new(), CONNECT_PATIENCE, total)
        .ok()?
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()
}

impl NetService {
    /// The ways of `plan`, all at once, under `deadline`: the hosts the
    /// direct way, the canary, each bridge on its own.
    pub(crate) async fn diagnose(
        &self,
        plan: &Plan,
        deadline: Instant,
    ) -> (Vec<DirectCheck>, Option<CanaryCheck>, Vec<BridgeCheck>) {
        let direct = self.diagnose_direct(&plan.endpoints, deadline);
        let canary = self.diagnose_canary(&plan.registries, deadline);
        let bridges = self.diagnose_bridges(&plan.bridges, deadline);
        tokio::join!(direct, canary, bridges)
    }

    async fn diagnose_direct(&self, endpoints: &[Endpoint], deadline: Instant) -> Vec<DirectCheck> {
        let client = direct_client(HOST_PATIENCE);
        let mut set = JoinSet::new();
        for (i, e) in endpoints.iter().enumerate() {
            self.count_asked();
            let (client, url) = (client.clone(), e.url.clone());
            set.spawn(async move {
                let Some(client) = client else {
                    return (i, (false, None, Some(cap("other"))));
                };
                let started = Instant::now();
                let asked = tokio::time::timeout_at(deadline, client.get(url).send()).await;
                (
                    i,
                    match asked {
                        Ok(Ok(_)) => (true, ms(started.elapsed()), None),
                        Ok(Err(e)) if e.is_timeout() => (false, None, Some(cap(TIMEOUT))),
                        Ok(Err(e)) => (false, None, Some(word_of(&e))),
                        Err(_) => (false, None, Some(cap(TIMEOUT))),
                    },
                )
            });
        }
        let answers = gather(set, endpoints.len(), |_| (false, None, Some(cap(TIMEOUT)))).await;
        endpoints
            .iter()
            .zip(answers)
            .map(|(e, (ok, ms, error))| DirectCheck {
                host: e.host.clone(),
                kind: e.kind,
                ok,
                ms,
                error,
            })
            .collect()
    }

    async fn diagnose_canary(
        &self,
        registries: &[String],
        deadline: Instant,
    ) -> Option<CanaryCheck> {
        if registries.is_empty() {
            return None;
        }
        self.count_asked();
        let bytes = CHECK_BYTES as u32;
        let Some(client) = direct_client(DIRECT_PATIENCE) else {
            return Some(CanaryCheck {
                ok: false,
                ms: None,
                bytes,
            });
        };
        let tried = async {
            for registry in registries {
                let started = Instant::now();
                let sent = client
                    .post(format!("{registry}/v1/canary"))
                    .body(vec![0x5au8; CHECK_BYTES])
                    .send()
                    .await;
                if sent.is_ok_and(|r| r.status().is_success()) {
                    return ms(started.elapsed());
                }
            }
            None
        };
        let took = tokio::time::timeout_at(deadline, tried)
            .await
            .ok()
            .flatten();
        Some(CanaryCheck {
            ok: took.is_some(),
            ms: took,
            bytes,
        })
    }

    async fn diagnose_bridges(&self, bridges: &[BridgeRef], deadline: Instant) -> Vec<BridgeCheck> {
        let bridges: Vec<BridgeRef> = bridges.iter().take(MAX_BRIDGES).cloned().collect();
        let mut set = JoinSet::new();
        for (i, bridge) in bridges.iter().enumerate() {
            self.count_asked();
            let bridge = bridge.clone();
            set.spawn(async move {
                // A client that knows this bridge alone: no other one stands
                // in for it.
                let client = Client::new(vec![bridge]);
                let started = Instant::now();
                let carried = tokio::time::timeout_at(
                    deadline,
                    tokio::time::timeout(BRIDGE_PATIENCE, probe::run(&client, CHECK_BYTES as u64)),
                )
                .await;
                (
                    i,
                    match carried {
                        Ok(Ok(Ok(_))) => (true, ms(started.elapsed()), None),
                        Ok(Ok(Err(e))) => (false, None, Some(word_of(&e))),
                        Ok(Err(_)) | Err(_) => (false, None, Some(cap(TIMEOUT))),
                    },
                )
            });
        }
        let answers = gather(set, bridges.len(), |_| (false, None, Some(cap(TIMEOUT)))).await;
        bridges
            .iter()
            .zip(answers)
            .map(|(b, (ok, ms, error))| BridgeCheck {
                addr: b.addr.to_string(),
                ok,
                ms,
                bytes: CHECK_BYTES as u32,
                error,
            })
            .collect()
    }
}

/// The call nodes `nodes` (with their regions), each asked on its own by
/// `client`, all at once, under `deadline`.
async fn diagnose_calls(
    client: NodeClient,
    nodes: Vec<(CallNode, Option<String>)>,
    deadline: Instant,
) -> Vec<CallNodeCheck> {
    let mut set = JoinSet::new();
    for (i, (node, _)) in nodes.iter().enumerate() {
        let (client, node) = (client.clone(), node.clone());
        set.spawn(async move {
            let checked = tokio::time::timeout_at(
                deadline,
                tokio::time::timeout(NODE_PATIENCE, client.check(&node)),
            )
            .await;
            (i, checked.ok().and_then(|c| c.ok()).unwrap_or_default())
        });
    }
    let answers = gather(set, nodes.len(), |_| messenger_calls::NodeCheck::default()).await;
    nodes
        .into_iter()
        .zip(answers)
        .map(|((node, region), checked)| CallNodeCheck {
            addr: node.node.addr.to_string(),
            region,
            class: node.class.into(),
            hello_ok: checked.hello.is_some(),
            hello_ms: checked.hello.and_then(ms),
            stun_ok: checked.stun.is_some(),
            stun_ms: checked.stun.and_then(ms),
        })
        .collect()
}

/// What stands for the address of a bridge of the user's own.
pub const PRIVATE_BRIDGE: &str = "private";

/// Puts [`PRIVATE_BRIDGE`] in place of every address of `hidden`, among
/// the bridges tried and for the bridge in use.
fn hide_private(out: &mut NetDiagnosis, hidden: &[String]) {
    for bridge in &mut out.bridges {
        if hidden.contains(&bridge.addr) {
            bridge.addr = PRIVATE_BRIDGE.into();
        }
    }
    if out.bridge_in_use.as_ref().is_some_and(|addr| hidden.contains(addr)) {
        out.bridge_in_use = Some(PRIVATE_BRIDGE.into());
    }
}

impl MessengerRuntime {
    /// The diagnosis of the network (see the module): with the project's
    /// servers chosen, every way measured now, for at most [`DEADLINE`];
    /// otherwise the choices alone, and nothing is asked.
    pub async fn net_diagnose(&self) -> Result<NetDiagnosis> {
        let servers_mode = match self.relays.servers_mode().await? {
            Some(ServersMode::Veydan) => DiagnosisServers::Veydan,
            Some(ServersMode::Own) => DiagnosisServers::Own,
            None => DiagnosisServers::Unchosen,
        };
        let mut out = NetDiagnosis {
            v: DIAGNOSIS_VERSION,
            at: super::now(),
            servers_mode,
            mode: self.net.mode().await?,
            active: self.net.net.is_active(),
            bridge_in_use: self.net.net.current().await.map(|b| b.addr.to_string()),
            direct: Vec::new(),
            canary: None,
            bridges: Vec::new(),
            calls: Vec::new(),
        };
        if servers_mode != DiagnosisServers::Veydan {
            return Ok(out);
        }
        let (manifest, _) = self.relays.current_manifest().await?;
        let plan = Plan {
            endpoints: endpoints(&manifest),
            registries: manifest.registries.clone(),
            bridges: self.net.bridges(&manifest).await?,
        };
        let nodes = self.diagnosis_call_nodes().await?;
        let deadline = Instant::now() + DEADLINE;
        let ((direct, canary, bridges), calls) = tokio::join!(
            self.net.diagnose(&plan, deadline),
            diagnose_calls(self.calls.nodes.clone(), nodes, deadline)
        );
        out.direct = direct;
        out.canary = canary;
        out.bridges = bridges;
        out.calls = calls;
        // A bridge the user added that no public list names is somebody's
        // own: whether it carries is told, its address is not.
        let mut public = self.net.listed().await?;
        public.extend(manifest.bridge_refs());
        let hidden: Vec<String> = self
            .net
            .private()
            .await?
            .into_iter()
            .filter(|b| !public.contains(b))
            .map(|b| b.addr.to_string())
            .collect();
        hide_private(&mut out, &hidden);
        Ok(out)
    }

    /// The call nodes a diagnosis asks: those of the manifest and of the
    /// registry's list kept here that the trust level lets a call use, the
    /// project's and the volunteers' only (a node of mine is nobody else's
    /// business), at most [`MAX_CALL_NODES`]. The registry is not asked for
    /// a list: the one kept is read, with the region it names.
    async fn diagnosis_call_nodes(&self) -> Result<Vec<(CallNode, Option<String>)>> {
        let sets = self.calls.servers();
        let trust = sets.trust().await?;
        if trust == TrustLevel::OwnOnly {
            return Ok(Vec::new());
        }
        let listed = match sets.registry() {
            Some(registry) if trust == TrustLevel::Any => {
                registry.cached().await.unwrap_or_default()
            }
            _ => Vec::new(),
        };
        let region_of = |id: &messenger_vlink::BridgeId| {
            listed
                .iter()
                .find(|r| r.node.id == *id)
                .map(|r| r.region.clone())
                .filter(|r| !r.is_empty())
        };
        let mut out: Vec<(CallNode, Option<String>)> = Vec::new();
        let manifest = self
            .calls
            .manifest
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let any_active = listed.iter().any(|r| r.is_active());
        let candidates = manifest.into_iter().chain(
            listed
                .iter()
                .filter(|r| r.is_active() || !any_active)
                .map(|r| (r.node.clone(), NodeClass::Volunteer)),
        );
        for (node, class) in candidates {
            if out.len() >= MAX_CALL_NODES {
                break;
            }
            if !matches!(class, NodeClass::Project | NodeClass::Volunteer) || !trust.allows(class) {
                continue;
            }
            if out.iter().any(|(n, _)| n.node.id == node.id) {
                continue;
            }
            let region = region_of(&node.id);
            out.push((CallNode::new(node, class), region));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_store::Store;
    use messenger_transport::EMBEDDED_MANIFEST_JSON;
    use messenger_vlink::testing::{bridge, Behaviour};
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    #[test]
    fn a_bridge_of_the_users_own_is_told_without_its_address() {
        let check = |addr: &str| BridgeCheck {
            addr: addr.into(),
            ok: true,
            ms: Some(40),
            bytes: 262_144,
            error: None,
        };
        let mut out = NetDiagnosis {
            v: DIAGNOSIS_VERSION,
            at: 1,
            servers_mode: DiagnosisServers::Veydan,
            mode: NetMode::On,
            active: true,
            bridge_in_use: Some("203.0.113.7:443".into()),
            direct: Vec::new(),
            canary: None,
            bridges: vec![check("203.0.113.7:443"), check("198.51.100.9:8443")],
            calls: Vec::new(),
        };
        hide_private(&mut out, &["203.0.113.7:443".to_string()]);
        assert_eq!(out.bridges[0].addr, PRIVATE_BRIDGE);
        assert!(out.bridges[0].ok, "whether it carries is still told");
        assert_eq!(out.bridges[1].addr, "198.51.100.9:8443", "a listed one keeps its address");
        assert_eq!(out.bridge_in_use.as_deref(), Some(PRIVATE_BRIDGE));
        assert!(!serde_json::to_string(&out).unwrap().contains("203.0.113.7"));
    }

    async fn service() -> NetService {
        NetService {
            store: Store::open_in_memory().await.unwrap(),
            net: Net::new(),
            relays_down_since: std::sync::Mutex::new(None),
            last_beat: std::sync::Mutex::new(None),
            asked: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// The probe target of a hub: `GET n` gives n bytes, `PUT n` takes n
    /// and answers `OK n`.
    async fn probe_target() -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let mut stream = BufReader::new(stream);
                    loop {
                        let mut line = String::new();
                        if stream.read_line(&mut line).await.unwrap_or(0) == 0 {
                            return;
                        }
                        let mut words = line.split_whitespace();
                        let (command, n) = (
                            words.next().unwrap_or(""),
                            words
                                .next()
                                .and_then(|n| n.parse::<usize>().ok())
                                .unwrap_or(0),
                        );
                        match command {
                            "GET" => stream.get_mut().write_all(&vec![0u8; n]).await.unwrap(),
                            "PUT" => {
                                let mut got = vec![0u8; n];
                                stream.read_exact(&mut got).await.unwrap();
                                stream
                                    .get_mut()
                                    .write_all(format!("OK {n}\n").as_bytes())
                                    .await
                                    .unwrap();
                            }
                            _ => return,
                        }
                    }
                });
            }
        });
        addr
    }

    /// A port nothing listens on.
    async fn closed_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        port
    }

    #[test]
    fn the_hosts_are_those_of_the_manifest_each_once_with_its_kind() {
        let mut manifest = Manifest::parse_content(EMBEDDED_MANIFEST_JSON).unwrap();
        manifest.registries = vec!["https://registry.example.org/vlink".into()];
        let got = endpoints(&manifest);
        let hosts: std::collections::BTreeSet<String> =
            got.iter().map(|e| e.host.clone()).collect();
        assert_eq!(
            hosts,
            super::super::project_hosts(&manifest),
            "the hosts of the project, no other"
        );
        assert_eq!(hosts.len(), got.len(), "each once");
        let registry = got
            .iter()
            .find(|e| e.host == "registry.example.org")
            .unwrap();
        assert_eq!(
            (registry.kind, registry.url.as_str()),
            (HostKind::Registry, "https://registry.example.org/")
        );
        let relay = &manifest.relays[0];
        let first = got
            .iter()
            .find(|e| Some(&e.host) == host_of(&relay.url).as_ref())
            .unwrap();
        assert_eq!(first.kind, HostKind::Relay);
        assert!(
            first.url.starts_with("https://") && first.url.ends_with('/'),
            "{}",
            first.url
        );

        assert_eq!(
            root_of("wss://u:p@r.example.org:8443/a?b=1#c").as_deref(),
            Some("https://r.example.org:8443/")
        );
        assert_eq!(root_of("ftp://x.example.org"), None);
    }

    #[test]
    fn an_error_is_a_word_never_its_text() {
        let e = std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "connect 192.0.2.7:443: Connection refused",
        );
        assert_eq!(word_of(&e), "refused");
        let e = std::io::Error::other(
            "failed to lookup address information: Name or service not known",
        );
        assert_eq!(word_of(&e), "dns");
        let e = std::io::Error::other("invalid peer certificate: UnknownIssuer");
        assert_eq!(word_of(&e), "tls");
        let e = std::io::Error::other("Connection reset by peer (os error 104)");
        assert_eq!(word_of(&e), "reset");
        let e = std::io::Error::other("operation timed out");
        assert_eq!(word_of(&e), "timeout");
        let e = std::io::Error::other("something at 198.51.100.4 with a long story");
        assert_eq!(word_of(&e), "other");
        assert_eq!(cap(&"x".repeat(200)).chars().count(), MAX_ERROR_CHARS);
    }

    // ─── The check that decides the way (`NetService::check`) ───────────────
    // Here for the fakes of this module: a registry, a relay, a bridge.

    use super::super::{
        Verdict, AUTO_HOLD_LONG_SECS, AUTO_HOLD_SECS, KEY_AUTO_RELEASED, KEY_AUTO_UNTIL,
        KEY_CHECK_PAUSE,
    };
    use messenger_store::settings;
    use messenger_vlink::NetConfig;

    /// A registry that takes the canary, and answers anything else 404.
    async fn registry() -> wiremock::MockServer {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/canary"))
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&server)
            .await;
        server
    }

    /// The embedded manifest with one relay at `relay` and the registry.
    fn manifest_of(relay: &str, registry: &wiremock::MockServer) -> Manifest {
        let mut manifest = Manifest::parse_content(EMBEDDED_MANIFEST_JSON).unwrap();
        manifest.relays.truncate(1);
        manifest.relays[0].url = relay.into();
        manifest.registries = vec![registry.uri()];
        manifest
    }

    async fn with_bridge(net: &NetService, bridge: &BridgeRef) {
        let config = NetConfig {
            active: false,
            hosts: Default::default(),
            bridges: vec![bridge.clone()],
        };
        net.net.configure(config).await.unwrap();
    }

    async fn canaries(registry: &wiremock::MockServer) -> usize {
        let asked = registry.received_requests().await.unwrap();
        asked.iter().filter(|r| r.url.path() == "/v1/canary").count()
    }

    /// The registry takes bytes the direct way and the relay does not
    /// answer on it: that is no working way, and the bridge that reaches
    /// the relay is taken.
    #[tokio::test]
    async fn a_way_that_carries_to_the_registry_and_not_to_a_relay_is_restricted() {
        let registry = registry().await;
        let manifest = manifest_of(&format!("ws://127.0.0.1:{}", closed_port().await), &registry);
        let carrying = bridge(Behaviour::CarryTo(probe_target().await)).await;
        let net = service().await;
        with_bridge(&net, &carrying.bridge).await;

        let check = net.check(&manifest).await.unwrap();
        assert_eq!((check.direct, check.bridge, check.verdict), (false, true, Verdict::Restricted));
        assert_eq!(canaries(&registry).await, 0, "no bytes are pushed with no relay answering");
        let until = net.stamp(KEY_AUTO_UNTIL).await.unwrap().expect("auto holds the bridge");
        let day = until - super::super::now();
        assert!((AUTO_HOLD_SECS - 60..=AUTO_HOLD_SECS).contains(&day), "{day}");
        assert!(net.stamp(KEY_CHECK_PAUSE).await.unwrap().is_some_and(|p| p == 600));
    }

    /// The relay answers neither way: it is down for everybody. Nobody is
    /// sent to a bridge for that, and the check is made again in a minute.
    #[tokio::test]
    async fn relays_down_for_everybody_send_nobody_to_a_bridge() {
        let registry = registry().await;
        let dead = closed_port().await;
        let manifest = manifest_of(&format!("ws://127.0.0.1:{dead}"), &registry);
        let to_nowhere: std::net::SocketAddr = format!("127.0.0.1:{dead}").parse().unwrap();
        let reaching = bridge(Behaviour::CarryTo(to_nowhere)).await;
        let net = service().await;
        with_bridge(&net, &reaching.bridge).await;

        let check = net.check(&manifest).await.unwrap();
        assert_eq!((check.direct, check.bridge, check.verdict), (false, false, Verdict::Offline));
        assert!(net.stamp(KEY_AUTO_UNTIL).await.unwrap().is_none(), "no hold");
        assert_eq!(reaching.streams.load(std::sync::atomic::Ordering::Relaxed), 0, "no probe");
        assert_eq!(canaries(&registry).await, 0);
        assert_eq!(net.stamp(KEY_CHECK_PAUSE).await.unwrap(), Some(60));
        let again = net.check(&manifest).await.unwrap();
        assert_eq!(again.verdict, Verdict::Offline);
        assert_eq!(net.stamp(KEY_CHECK_PAUSE).await.unwrap(), Some(120));
    }

    /// A bridge is left when the direct way worked twice, and a direct way
    /// that fails again soon after is not gone back to for a week.
    #[tokio::test]
    async fn a_bridge_is_left_after_two_tries_and_held_for_long_when_the_direct_way_fails_again() {
        let registry = registry().await;
        // The registry stands for the relay too: any HTTP answer counts.
        let working = manifest_of(&registry.uri().replace("http://", "ws://"), &registry);
        let carrying = bridge(Behaviour::CarryTo(probe_target().await)).await;
        let net = service().await;
        with_bridge(&net, &carrying.bridge).await;
        let now = super::super::now();
        settings::set(&net.store, KEY_AUTO_UNTIL, &(now + 3600).to_string()).await.unwrap();

        let check = net.check(&working).await.unwrap();
        assert_eq!(check.verdict, Verdict::Direct);
        assert_eq!(canaries(&registry).await, 2, "the direct way carried twice");
        assert!(net.stamp(KEY_AUTO_UNTIL).await.unwrap().is_none(), "the bridge is left");
        assert!(net.stamp(KEY_AUTO_RELEASED).await.unwrap().is_some_and(|at| at >= now));

        // Without a hold nothing is left: one try is enough.
        net.check(&working).await.unwrap();
        assert_eq!(canaries(&registry).await, 3);

        let failing = manifest_of(&format!("ws://127.0.0.1:{}", closed_port().await), &registry);
        let check = net.check(&failing).await.unwrap();
        assert_eq!(check.verdict, Verdict::Restricted);
        let week = net.stamp(KEY_AUTO_UNTIL).await.unwrap().unwrap() - super::super::now();
        assert!((AUTO_HOLD_LONG_SECS - 60..=AUTO_HOLD_LONG_SECS).contains(&week), "{week}");

        // A later check of a day's hold does not make the week shorter.
        settings::delete(&net.store, KEY_AUTO_RELEASED).await.unwrap();
        net.check(&failing).await.unwrap();
        let still = net.stamp(KEY_AUTO_UNTIL).await.unwrap().unwrap() - super::super::now();
        assert!(still > AUTO_HOLD_SECS, "{still}");
    }

    /// Each way on its own: a host that answers and one that does not, the
    /// canary of a registry, a bridge that carries and one that is not
    /// there, each tried with a client of its own.
    #[tokio::test]
    async fn every_way_is_measured_on_its_own() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/canary"))
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&server)
            .await;
        // Anything else is answered 404: an answer still.
        let dead = closed_port().await;
        let carrying = bridge(Behaviour::CarryTo(probe_target().await)).await;
        let gone: BridgeRef = format!("127.0.0.1:{}#{}", closed_port().await, carrying.bridge.id)
            .parse()
            .unwrap();
        let plan = Plan {
            endpoints: vec![
                Endpoint {
                    host: "up.example".into(),
                    kind: HostKind::Relay,
                    url: format!("{}/", server.uri()),
                },
                Endpoint {
                    host: "down.example".into(),
                    kind: HostKind::Media,
                    url: format!("http://127.0.0.1:{dead}/"),
                },
            ],
            registries: vec![format!("http://127.0.0.1:{dead}"), server.uri()],
            bridges: vec![carrying.bridge.clone(), gone.clone()],
        };
        let net = service().await;
        let (direct, canary, bridges) = net.diagnose(&plan, Instant::now() + DEADLINE).await;
        assert_eq!(direct.len(), 2);
        assert!(
            direct[0].ok && direct[0].ms.is_some() && direct[0].error.is_none(),
            "{:?}",
            direct[0]
        );
        assert_eq!(
            (direct[0].host.as_str(), direct[0].kind),
            ("up.example", HostKind::Relay)
        );
        assert!(!direct[1].ok && direct[1].ms.is_none());
        assert_eq!(direct[1].error.as_deref(), Some("refused"));
        let canary = canary.expect("a registry is named");
        assert!(
            canary.ok && canary.ms.is_some(),
            "the second registry took it"
        );
        assert_eq!(canary.bytes, 256 * 1024);
        assert_eq!(bridges.len(), 2);
        assert!(
            bridges[0].ok && bridges[0].error.is_none(),
            "{:?}",
            bridges[0]
        );
        assert_eq!(
            (bridges[0].addr.clone(), bridges[0].bytes),
            (carrying.bridge.addr.to_string(), 256 * 1024)
        );
        assert!(
            carrying.streams.load(std::sync::atomic::Ordering::Relaxed) >= 1,
            "the probe went through it"
        );
        assert!(!bridges[1].ok && bridges[1].ms.is_none());
        assert!(bridges[1]
            .error
            .as_deref()
            .is_some_and(|e| e.len() <= MAX_ERROR_CHARS));
        assert_eq!(bridges[1].addr, gone.addr.to_string());
        assert_eq!(net.asked(), 5, "two hosts, the canary, two bridges");

        // Nothing named: nothing asked, no canary.
        let none = service().await;
        let (direct, canary, bridges) = none
            .diagnose(&Plan::default(), Instant::now() + DEADLINE)
            .await;
        assert!(direct.is_empty() && canary.is_none() && bridges.is_empty());
        assert_eq!(none.asked(), 0);
    }

    /// What did not finish by the deadline is a timeout, and the deadline
    /// holds whatever a way would wait.
    #[tokio::test]
    async fn what_does_not_finish_by_the_deadline_is_a_timeout() {
        // A host that takes the connection and never answers.
        let silent = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = silent.local_addr().unwrap();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((s, _)) = silent.accept().await {
                held.push(s);
            }
        });
        let plan = Plan {
            endpoints: vec![Endpoint {
                host: "slow.example".into(),
                kind: HostKind::Push,
                url: format!("http://{addr}/"),
            }],
            ..Plan::default()
        };
        let net = service().await;
        let started = std::time::Instant::now();
        let (direct, _, _) = net
            .diagnose(&plan, Instant::now() + Duration::from_millis(300))
            .await;
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the deadline holds"
        );
        assert_eq!(
            (direct[0].ok, direct[0].error.as_deref()),
            (false, Some("timeout"))
        );
    }

    /// More than eight bridges: the first eight are tried.
    #[tokio::test]
    async fn at_most_eight_bridges_are_tried() {
        let id = "9c5f434def40ebc2879201642bab6c69063578c4a302aa92de5918b3663ffbfc";
        let port = closed_port().await;
        let many: Vec<BridgeRef> = (0..12)
            .map(|i| format!("127.0.0.{}:{port}#{id}", i + 1).parse().unwrap())
            .collect();
        let net = service().await;
        let plan = Plan {
            bridges: many,
            ..Plan::default()
        };
        let (_, _, bridges) = net.diagnose(&plan, Instant::now() + DEADLINE).await;
        assert_eq!(bridges.len(), MAX_BRIDGES);
        assert!(bridges.iter().all(|b| !b.ok));
        assert_eq!(net.asked(), MAX_BRIDGES);
    }
}
