// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The call nodes as the settings show them: every node the sets of
//! servers know, whose it is and where it came from, how near and how
//! full, whether a call may use it; adding a private node by its link,
//! removing one of mine, the trust level (the plan of calls, «Обнаружение
//! узлов», and internal/messenger-wire.md §10).
//!
//! The sets themselves are the core's (`messenger_calls::servers`): mine
//! (the nodes this device was invited to, then the developer setting
//! `call.nodes`) → the cloud of a subscription (empty) → the manifest →
//! the registry of volunteers. The list here reads them the way the core
//! does and adds what the node client knows of each node; it decides
//! nothing a call does.
//!
//! What touches the network, and when:
//! - the registry is asked only with the project's servers chosen, out
//!   of the silent mode, and under the level `any`, and then in the
//!   background when its list is due (`Registry::nodes`, and
//!   [`MessengerRuntime::call_registry_tick`] from the watch of the
//!   app); otherwise only the list kept from before is shown, and nothing
//!   is asked (the gate of the fetch, `calls::gated_registry_fetch`, holds
//!   the core's own asks too);
//! - a probe (`call_nodes_list` with `probe`) asks the nodes a call may
//!   use now, and only those: under `own_only` no node of the manifest or
//!   the registry is spoken to;
//! - adding by a link with an invitation exchanges it at the node (TLS
//!   pinned to the id of the link) and asks that node at once whether it
//!   lets this device in.
//!
//! Every ask is a line in the log (`messenger calls: …`).

use crate::MessengerRuntime;
use messenger_calls::servers::{parse_own_nodes, CallNode, NodeClass, NodeRef};
use messenger_calls::{KnownNode, ServerSets, TrustLevel, KEY_CALL_NODES};
use messenger_core::{MessengerError, Result};
use messenger_links::{CallNodeLink, LinkType, Uri};
use messenger_store::settings;
use messenger_vlink::BridgeId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::Duration;
use ts_rs::TS;

/// How long a probe waits for one node.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(6);

/// Which nodes a call may use: the setting `call.trust`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CallTrust {
    /// Mine, the project's, volunteers'.
    #[default]
    Any,
    /// Mine and the project's: no volunteer's node.
    ProjectAndOwn,
    /// Mine only: neither the manifest nor the registry.
    OwnOnly,
}

impl From<TrustLevel> for CallTrust {
    fn from(t: TrustLevel) -> Self {
        match t {
            TrustLevel::Any => Self::Any,
            TrustLevel::ProjectAndOwn => Self::ProjectAndOwn,
            TrustLevel::OwnOnly => Self::OwnOnly,
        }
    }
}

impl From<CallTrust> for TrustLevel {
    fn from(t: CallTrust) -> Self {
        match t {
            CallTrust::Any => Self::Any,
            CallTrust::ProjectAndOwn => Self::ProjectAndOwn,
            CallTrust::OwnOnly => Self::OwnOnly,
        }
    }
}

impl CallTrust {
    pub fn parse(s: &str) -> Option<Self> {
        TrustLevel::parse(s).map(Into::into)
    }

    pub fn as_str(self) -> &'static str {
        TrustLevel::from(self).as_str()
    }
}

/// Whose a node is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CallNodeClass {
    /// Mine: invited by a link, or in my own list.
    Own,
    /// Pinned to a group (not listed here: the group's settings show it).
    Group,
    /// The cloud of a subscription (none yet).
    Cloud,
    /// The project's, from the manifest.
    Project,
    /// A volunteer's, from the registry (or so named by the manifest).
    Volunteer,
}

impl CallNodeClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Own => "own",
            Self::Group => "group",
            Self::Cloud => "cloud",
            Self::Project => "project",
            Self::Volunteer => "volunteer",
        }
    }
}

impl From<NodeClass> for CallNodeClass {
    fn from(c: NodeClass) -> Self {
        match c {
            NodeClass::Own => Self::Own,
            NodeClass::Group => Self::Group,
            NodeClass::Cloud => Self::Cloud,
            NodeClass::Project => Self::Project,
            NodeClass::Volunteer => Self::Volunteer,
        }
    }
}

/// Where a node of the list came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CallNodeSource {
    /// A link with an invitation: the credentials of this device.
    Device,
    /// My own list (`call.nodes`): a reference, maybe with a key.
    Setting,
    Cloud,
    Manifest,
    Registry,
}

impl CallNodeSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Device => "device",
            Self::Setting => "setting",
            Self::Cloud => "cloud",
            Self::Manifest => "manifest",
            Self::Registry => "registry",
        }
    }
}

/// How a node is, as far as this device knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CallNodeHealth {
    /// It answered and let me in (or the registry lists it as working).
    Active,
    /// At one of its limits by its own word, or the registry's last check
    /// of it failed: used only when nothing better answers.
    Degraded,
    /// It answered and would not let me in: a private node without my
    /// key, a device it revoked.
    Refused,
    /// The last probe did not reach it.
    Unreachable,
    /// Nothing is known yet: never asked.
    Unknown,
}

impl CallNodeHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Degraded => "degraded",
            Self::Refused => "refused",
            Self::Unreachable => "unreachable",
            Self::Unknown => "unknown",
        }
    }
}

/// One node of the list of the settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CallNodeInfo {
    /// `address:port#id`.
    pub reference: String,
    /// 64 hex: what TLS to its control channel is pinned to.
    pub id: String,
    /// `address:port` of its control channel.
    pub addr: String,
    pub class: CallNodeClass,
    pub source: CallNodeSource,
    /// Mine (invited, or in my own list): only mine can be removed.
    pub mine: bool,
    /// A key or the credentials of this device are kept for it.
    pub has_key: bool,
    /// The label of its invitation, else what this device called itself
    /// when invited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub label: Option<String>,
    /// The region the node names for itself in the registry (`eu`); the
    /// registry knows no country.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub region: Option<String>,
    /// What it can do: the node's word when it was spoken to, the
    /// registry's otherwise (`sfu`, `cascade`, `turn-tls`…).
    pub caps: Vec<String>,
    /// The round trip of the last answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub rtt_ms: Option<u32>,
    /// How full, percent: the node's own word when it was spoken to
    /// lately, the registry's otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub load: Option<u8>,
    pub health: CallNodeHealth,
    /// Private (only for those with a key or an invitation), by its word.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub private: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub version: Option<String>,
    /// A call may use it now: the trust level lets it, and it is in the
    /// sets (the credentials of an invited node are readable, a degraded
    /// node of the registry only while it lists no active one).
    pub used: bool,
    /// When this device was invited, unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub added_at: Option<i64>,
}

/// The call nodes and the trust level, as the settings show them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CallNodesView {
    pub trust: CallTrust,
    /// Mine first, then the cloud's, the manifest's, the registry's: the
    /// order in which a call tries them.
    pub nodes: Vec<CallNodeInfo>,
    /// This build asks a registry for volunteers' nodes.
    pub registry: bool,
    /// When the registry last answered, unix seconds.
    #[ts(type = "number | null")]
    pub registry_checked_at: Option<i64>,
    /// The registry is not asked now, whatever the trust level: the
    /// project's servers are not in use (own ones, or none chosen yet),
    /// or the silent mode is on. The list kept from before is still
    /// shown and used under `any`.
    pub registry_paused: bool,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn invalid(code: &str) -> MessengerError {
    MessengerError::Invalid(code.into())
}

/// The node a text names, and the invitation it carries: a link
/// `veydan://call-node/<id>?a=<address:port>&t=<token>` (the token may be
/// missing), or a reference `address:port#id` as the node's own tools
/// print it.
pub fn node_of_text(text: &str) -> Result<(NodeRef, Option<String>)> {
    let text = text.trim();
    if text.contains("://") {
        let uri = Uri::parse(text).map_err(|_| invalid("call_node_invalid"))?;
        if uri.link_type() != &LinkType::CallNode {
            return Err(invalid("call_node_link_type"));
        }
        let link = CallNodeLink::from_uri(&uri).map_err(|_| invalid("call_node_invalid"))?;
        let node: NodeRef = link.reference().parse().map_err(|_| invalid("call_node_invalid"))?;
        return Ok((node, link.token));
    }
    text.parse::<NodeRef>().map(|n| (n, None)).map_err(|_| invalid("call_node_invalid"))
}

/// What the exchange of an invitation answered, as a code the screen
/// words (`call_node_*`); the node's own wording follows a code that has
/// one.
fn invite_error(e: MessengerError) -> MessengerError {
    let text = e.to_string();
    let code = if text.contains("takes no invitations") {
        "call_node_no_invites"
    } else if text.contains("bad_invite") {
        "call_node_bad_invite"
    } else if text.contains("rate_limited") {
        "call_node_rate_limited"
    } else if text.contains("another node") {
        "call_node_other"
    } else {
        match e {
            MessengerError::Transport(_) => return MessengerError::Invalid(format!("call_node_unreachable: {text}")),
            other => return other,
        }
    };
    MessengerError::Invalid(code.into())
}

/// One row before what the node client knows is added to it.
struct Row {
    node: NodeRef,
    class: NodeClass,
    source: CallNodeSource,
    has_key: bool,
    label: Option<String>,
    added_at: Option<i64>,
    region: Option<String>,
    caps: Vec<String>,
    listed_load: Option<u8>,
    listed_state: Option<String>,
}

impl Row {
    fn new(node: NodeRef, class: NodeClass, source: CallNodeSource, has_key: bool) -> Self {
        Self { node, class, source, has_key, label: None, added_at: None, region: None, caps: vec![], listed_load: None, listed_state: None }
    }
}

impl MessengerRuntime {
    /// The call nodes and the trust level. With `probe` the nodes a call
    /// may use now are asked first (HELLO and credentials, all at once,
    /// [`PROBE_TIMEOUT`] each; what was asked lately comes from the cache
    /// of the node client): their round trip, load and state come back
    /// with the list.
    pub async fn call_nodes_list(&self, probe: bool) -> Result<CallNodesView> {
        if probe {
            let nodes = self.calls.servers().call_nodes().await?;
            self.probe_call_nodes(nodes).await;
        }
        self.call_nodes_view().await
    }

    /// Asks `nodes` at once and keeps which did not answer.
    async fn probe_call_nodes(&self, nodes: Vec<CallNode>) {
        if nodes.is_empty() {
            return;
        }
        let names = nodes.iter().map(|n| n.node.to_string()).collect::<Vec<_>>().join(", ");
        eprintln!("messenger calls: probing {} call node(s): {names}", nodes.len());
        let at = now();
        let mut tasks = tokio::task::JoinSet::new();
        for node in nodes {
            let client = self.calls.nodes.clone();
            tasks.spawn(async move {
                let answer = tokio::time::timeout(PROBE_TIMEOUT, client.access(&node, at)).await;
                (node.node.id, answer)
            });
        }
        while let Some(joined) = tasks.join_next().await {
            let Ok((bid, answer)) = joined else { continue };
            let id = bid.to_string();
            let why = match answer {
                Ok(Ok(_)) => {
                    self.calls.unreachable.lock().unwrap().remove(&id);
                    continue;
                }
                Ok(Err(e)) => e.to_string(),
                Err(_) => "no answer in time".to_string(),
            };
            // What the node client kept of it is older than this miss (a
            // good entry would have been the answer): forgotten, so that
            // an entry seen later is a later answer, of a call or a
            // probe, and the row shows it rather than this miss.
            let mut unreachable = self.calls.unreachable.lock().unwrap();
            self.calls.nodes.forget(&bid);
            unreachable.insert(id, why);
        }
    }

    async fn own_setting_nodes(&self) -> Result<Vec<CallNode>> {
        Ok(match settings::get(&self.store, KEY_CALL_NODES).await? {
            Some(json) => parse_own_nodes(&json),
            None => Vec::new(),
        })
    }

    /// The list as it is now; nothing is asked but the registry, under
    /// `any`, when its list is due (in the background).
    pub async fn call_nodes_view(&self) -> Result<CallNodesView> {
        let sets = self.calls.servers();
        let trust = sets.trust().await?;
        // What a call would take now: the core's own reading of the sets.
        let used = sets.call_nodes().await?;
        let registry = sets.registry();
        // The list kept from before, checked again; never asked for here.
        let listed = match &registry {
            Some(r) => r.cached().await.unwrap_or_default(),
            None => Vec::new(),
        };

        let mut rows: Vec<Row> = Vec::new();
        let mut push = |row: Row| {
            if !rows.iter().any(|r| r.node.id == row.node.id) {
                rows.push(row);
            }
        };
        for e in sets.devices().await? {
            let Some(node) = e.node() else { continue };
            let mut row = Row::new(node, NodeClass::Own, CallNodeSource::Device, true);
            row.label = Some(if e.label.is_empty() { e.name } else { e.label }).filter(|l| !l.trim().is_empty());
            row.added_at = Some(e.added_at).filter(|t| *t > 0);
            push(row);
        }
        for n in self.own_setting_nodes().await? {
            let has_key = n.access_key.is_some();
            push(Row::new(n.node, NodeClass::Own, CallNodeSource::Setting, has_key));
        }
        let manifest = self.calls.manifest.read().unwrap().clone();
        for (node, class) in manifest {
            push(Row::new(node, class, CallNodeSource::Manifest, false));
        }
        for r in listed {
            match rows.iter_mut().find(|row| row.node.id == r.node.id) {
                // A node of mine or of the manifest that the registry lists
                // too: what the registry says of it is told on its row.
                Some(row) => {
                    row.region = Some(r.region).filter(|s| !s.is_empty());
                    row.caps = r.caps;
                    row.listed_load = Some(r.load);
                    row.listed_state = Some(r.state);
                }
                None => {
                    let mut row = Row::new(r.node, NodeClass::Volunteer, CallNodeSource::Registry, false);
                    row.region = Some(r.region).filter(|s| !s.is_empty());
                    row.caps = r.caps;
                    row.listed_load = Some(r.load);
                    row.listed_state = Some(r.state);
                    rows.push(row);
                }
            }
        }

        let known: HashMap<String, KnownNode> = self.calls.nodes.known().into_iter().map(|k| (k.id.clone(), k)).collect();
        // A miss of a probe forgets what the node client kept of the node
        // (`probe_call_nodes`): what it knows of it now came after, and
        // is newer than the miss.
        let unreachable = {
            let mut misses = self.calls.unreachable.lock().unwrap();
            misses.retain(|id, _| !known.contains_key(id));
            misses.clone()
        };
        let nodes = rows
            .into_iter()
            .map(|row| {
                let id = row.node.id.to_string();
                let k = known.get(&id);
                let health = if unreachable.contains_key(&id) {
                    CallNodeHealth::Unreachable
                } else if let Some(k) = k {
                    match (k.authorized, k.state.as_str()) {
                        (false, _) => CallNodeHealth::Refused,
                        (true, "degraded") => CallNodeHealth::Degraded,
                        (true, _) => CallNodeHealth::Active,
                    }
                } else {
                    match row.listed_state.as_deref() {
                        Some("degraded") => CallNodeHealth::Degraded,
                        Some(_) => CallNodeHealth::Active,
                        None => CallNodeHealth::Unknown,
                    }
                };
                CallNodeInfo {
                    reference: row.node.to_string(),
                    addr: row.node.addr.to_string(),
                    class: row.class.into(),
                    source: row.source,
                    mine: row.class == NodeClass::Own,
                    has_key: row.has_key,
                    label: row.label,
                    region: row.region,
                    caps: k.map(|k| k.capabilities.clone()).filter(|c| !c.is_empty()).unwrap_or(row.caps),
                    rtt_ms: k.map(|k| k.rtt_ms.min(u32::MAX as u64) as u32),
                    load: k.and_then(|k| k.load).or(row.listed_load),
                    health,
                    private: k.map(|k| k.private),
                    version: k.map(|k| k.version.clone()).filter(|v| !v.is_empty()),
                    used: used.iter().any(|u| u.node.id == row.node.id),
                    added_at: row.added_at,
                    id,
                }
            })
            .collect();
        let registry_checked_at = match &registry {
            Some(r) => r.checked_at().await?,
            None => None,
        };
        let registry_paused = !crate::calls::registry_may_ask(&self.store).await;
        Ok(CallNodesView { trust: trust.into(), nodes, registry: registry.is_some(), registry_checked_at, registry_paused })
    }

    /// Adds a node of mine from `text`: a link with an invitation
    /// (`veydan://call-node/<id>?a=…&t=…`) is exchanged at the node for
    /// the credentials of this device, kept in the secret store; a link
    /// without one, or a reference `address:port#id`, goes into my own
    /// list, with `key` when the node is private and its key was given by
    /// hand. `name` is what this device calls itself in the node's list of
    /// devices (a product and a platform). The node is asked at once
    /// whether it lets me in; the answer is on its row.
    ///
    /// Refusals are codes the screen words: `call_node_invalid`,
    /// `call_node_link_type`, `call_node_no_invites` (a node of a version
    /// before invitations), `call_node_bad_invite` (the token is unknown,
    /// used up or ran out), `call_node_rate_limited`, `call_node_other`
    /// (the node is not the one of the link), `call_node_unreachable: …`;
    /// a locked secret store is `SecretsLocked`, before the token is spent.
    pub async fn call_node_add(&self, text: &str, key: Option<String>, name: &str) -> Result<CallNodesView> {
        let (node, token) = node_of_text(text)?;
        let sets = self.calls.servers();
        match token {
            Some(token) => {
                // The token is spent by the exchange: where to keep what it
                // gives must be open before it is asked.
                if !self.secrets.is_unlocked().await {
                    return Err(MessengerError::SecretsLocked);
                }
                eprintln!("messenger calls: exchanging an invitation at the call node {node}");
                let creds = self.calls.nodes.redeem_invite(&node, &token, name, now()).await.map_err(invite_error)?;
                sets.add_device(&creds).await?;
            }
            None => {
                let mut own = self.own_setting_nodes().await?;
                own.retain(|n| n.node.id != node.id);
                own.push(CallNode::with_key(node.clone(), NodeClass::Own, key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty())));
                sets.set_own(&own).await?;
            }
        }
        self.calls.nodes.forget(&node.id);
        self.calls.unreachable.lock().unwrap().remove(&node.id.to_string());
        let now_used: Vec<CallNode> = sets.call_nodes().await?.into_iter().filter(|n| n.node.id == node.id).collect();
        self.probe_call_nodes(now_used).await;
        self.call_nodes_view().await
    }

    /// Removes the node `id` (64 hex) of mine: the credentials of this
    /// device on it, or its entry in my own list. The node is not told:
    /// its operator revokes a device (`vcall ctl revoke`).
    /// `call_node_unknown` when no node of mine has that id.
    pub async fn call_node_remove(&self, id: &str) -> Result<CallNodesView> {
        let id = id.trim().to_ascii_lowercase();
        let sets = self.calls.servers();
        let mut gone = sets.remove_device(&id).await?;
        let own = self.own_setting_nodes().await?;
        let kept: Vec<CallNode> = own.iter().filter(|n| n.node.id.to_string() != id).cloned().collect();
        if kept.len() != own.len() {
            sets.set_own(&kept).await?;
            gone = true;
        }
        if !gone {
            return Err(invalid("call_node_unknown"));
        }
        if let Ok(bid) = id.parse::<BridgeId>() {
            self.calls.nodes.forget(&bid);
        }
        self.calls.unreachable.lock().unwrap().remove(&id);
        self.call_nodes_view().await
    }

    /// Which nodes calls may use from now on; the call under way keeps
    /// its own.
    pub async fn call_nodes_set_trust(&self, trust: CallTrust) -> Result<CallNodesView> {
        self.calls.servers().set_trust(trust.into()).await?;
        self.call_nodes_view().await
    }

    /// Asks the registry for its list now, waited for. Under another
    /// level than `any` nothing is asked: the registry is not used; nor
    /// while it is paused (`registry_paused`: own servers, none chosen
    /// yet, the silent mode).
    pub async fn call_nodes_refresh(&self) -> Result<CallNodesView> {
        let sets = self.calls.servers();
        if sets.trust().await? == TrustLevel::Any && crate::calls::registry_may_ask(&self.store).await {
            if let Some(registry) = sets.registry() {
                registry.refresh().await?;
            }
        }
        self.call_nodes_view().await
    }

    /// The housekeeping of the registry (the watch of the app calls it
    /// every few seconds): with the project's servers, out of the silent
    /// mode and under `any`, a list that is due is asked for in the
    /// background, one ask at a time, not again within a quarter of an
    /// hour of one that failed (`Registry::refresh_in_background`). An
    /// ask the gate refused while it was paused (the servers or the
    /// silent mode were otherwise then) is no failure: the first tick
    /// that may ask does so at once.
    pub async fn call_registry_tick(&self) {
        if !crate::calls::registry_may_ask(&self.store).await {
            self.calls.registry_paused.store(true, Ordering::SeqCst);
            return;
        }
        let sets = self.calls.servers();
        if !matches!(sets.trust().await, Ok(TrustLevel::Any)) {
            return;
        }
        let Some(registry) = sets.registry() else { return };
        let was_paused = self.calls.registry_paused.swap(false, Ordering::SeqCst);
        if !registry.is_stale().await.unwrap_or(false) {
            return;
        }
        if was_paused {
            tokio::spawn(async move {
                if let Err(e) = registry.refresh().await {
                    eprintln!("messenger calls: the registry's list was not refreshed: {e}");
                }
            });
        } else {
            registry.refresh_in_background();
        }
    }

    /// Whether a node of mine has the id `id`, for the card of a link: a
    /// link with an invitation is "added" only when this device holds its
    /// own credentials on that node (an entry of my own list, with a key
    /// or without, leaves the invitation to be exchanged); a link without
    /// one when the node is mine either way.
    pub(crate) async fn call_node_is_mine(&self, id: &str, invitation: bool) -> Result<bool> {
        let devices = self.calls.servers().devices().await?;
        if devices.iter().any(|e| e.node().is_some_and(|n| n.id.to_string() == id)) {
            return Ok(true);
        }
        if invitation {
            return Ok(false);
        }
        Ok(self.own_setting_nodes().await?.iter().any(|n| n.node.id.to_string() == id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calls::{CallBackends, RegistrySource};
    use crate::servers::use_veydan_offline;
    use messenger_core::MessengerConfig;
    use messenger_testkit::{FakeEngine, FakeNode, FakeRegistry, MemorySecretStore};
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    async fn runtime(dir: &tempfile::TempDir, node: &FakeNode, registry: &FakeRegistry, secrets: Arc<MemorySecretStore>) -> MessengerRuntime {
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let (fetch, root) = (registry.fetch(), registry.root_hex());
        let backends = CallBackends {
            engine: Some(Arc::new(FakeEngine::new())),
            nodes: node.client(),
            rooms: Arc::new(node.clone()),
            registry: RegistrySource::Given(Box::new(move |store| {
                messenger_calls::Registry::with_trust(store, fetch, vec!["https://registry.test/vlink".into()], root)
            })),
        };
        let rt = MessengerRuntime::start_with_backends(cfg, secrets, backends).await.unwrap();
        rt.relays().set_silent(true).await.unwrap();
        use_veydan_offline(&rt).await;
        rt
    }

    #[test]
    fn a_text_names_a_node_by_a_link_or_a_reference() {
        let id = "fa".repeat(32);
        let (node, token) = node_of_text(&format!(" veydan://call-node/{id}?a=203.0.113.7%3A8443&t=inv-1 ")).unwrap();
        assert_eq!((node.to_string(), token.as_deref()), (format!("203.0.113.7:8443#{id}"), Some("inv-1")));
        let (node, token) = node_of_text(&format!("203.0.113.7:8443#{id}")).unwrap();
        assert_eq!((node.addr.port(), token), (8443, None));
        assert_eq!(node_of_text(&format!("veydan://call-node/{id}?a=203.0.113.7%3A8443")).unwrap().1, None);
        let code = |t: &str| match node_of_text(t) {
            Err(MessengerError::Invalid(c)) => c,
            other => panic!("{other:?}"),
        };
        assert_eq!(code(&format!("veydan://vlink/{id}?a=203.0.113.7%3A443")), "call_node_link_type");
        assert_eq!(code("veydan://call-node/abc?a=1.2.3.4%3A443"), "call_node_invalid");
        assert_eq!(code("nonsense"), "call_node_invalid");
        assert_eq!(CallTrust::parse("own_only"), Some(CallTrust::OwnOnly));
        assert_eq!(serde_json::to_value(CallTrust::ProjectAndOwn).unwrap(), serde_json::json!(TrustLevel::ProjectAndOwn.as_str()));
        for class in [NodeClass::Own, NodeClass::Group, NodeClass::Cloud, NodeClass::Project, NodeClass::Volunteer] {
            let mine = CallNodeClass::from(class);
            assert_eq!(serde_json::to_value(mine).unwrap(), serde_json::json!(class.as_str()), "the words of the core");
            assert_eq!(mine.as_str(), class.as_str());
        }
        for (source, word) in [(CallNodeSource::Device, "device"), (CallNodeSource::Registry, "registry"), (CallNodeSource::Setting, "setting")] {
            assert_eq!((serde_json::to_value(source).unwrap(), source.as_str()), (serde_json::json!(word), word));
        }
        for health in [CallNodeHealth::Active, CallNodeHealth::Degraded, CallNodeHealth::Refused, CallNodeHealth::Unreachable, CallNodeHealth::Unknown] {
            assert_eq!(serde_json::to_value(health).unwrap(), serde_json::json!(health.as_str()));
        }
    }

    /// The list of the settings over the fakes: the manifest's nodes, the
    /// registry's, a private node added by its invitation and called
    /// through, the trust levels, a revoked device, the removal.
    #[tokio::test]
    async fn the_list_the_link_the_trust_and_the_removal() {
        let dir = tempfile::tempdir().unwrap();
        let node = FakeNode::new(FakeEngine::new());
        let registry = FakeRegistry::new();
        // A node of a volunteer in the registry, and a private node.
        let volunteer = node.add_node();
        let private = node.add_node();
        let volunteer_ref = node.node(volunteer, NodeClass::Volunteer).node;
        let private_ref = node.node(private, NodeClass::Own).node;
        node.set_private(private, true, &[]);
        registry.list(vec![(volunteer_ref.clone(), "eu", 30)]);
        let secrets = Arc::new(MemorySecretStore::unlocked());
        let rt = runtime(&dir, &node, &registry, secrets.clone()).await;

        // The first look: the manifest's nodes, never asked; the registry
        // is asked in the background (the list is due).
        let view = rt.call_nodes_list(false).await.unwrap();
        assert_eq!(view.trust, CallTrust::Any);
        assert!(view.registry);
        let manifest: Vec<&CallNodeInfo> = view.nodes.iter().filter(|n| n.source == CallNodeSource::Manifest).collect();
        assert!(!manifest.is_empty() && manifest.iter().all(|n| n.class == CallNodeClass::Project && !n.mine), "{view:?}");
        assert!(manifest.iter().all(|n| n.health == CallNodeHealth::Unknown && n.used && n.rtt_ms.is_none()));
        rt.calls.servers().registry().unwrap().settle().await;
        assert_eq!(registry.asked().load(Ordering::SeqCst), 1);
        let view = rt.call_nodes_list(false).await.unwrap();
        let listed = view.nodes.iter().find(|n| n.id == volunteer_ref.id.to_string()).expect("the registry's node is listed");
        assert_eq!((listed.class, listed.source, listed.region.as_deref(), listed.load, listed.health), (CallNodeClass::Volunteer, CallNodeSource::Registry, Some("eu"), Some(30), CallNodeHealth::Active));
        assert!(listed.used && view.registry_checked_at.is_some());

        // The probe asks the nodes a call may use: each comes back with a
        // round trip and its state.
        let view = rt.call_nodes_list(true).await.unwrap();
        assert!(view.nodes.iter().filter(|n| n.used).all(|n| n.rtt_ms.is_some() && n.health == CallNodeHealth::Active), "{view:?}");

        // A link of the private node with an invitation: exchanged, kept,
        // mine, and the node lets this device in.
        let token = node.invite(private, 1, 3600);
        let link = format!("veydan://call-node/{}?a={}&t={token}", private_ref.id, private_ref.addr.to_string().replace(':', "%3A"));
        match rt.link_inspect(&link).await.unwrap() {
            crate::LinkView::CallNode { id, has_token, added, .. } => assert_eq!((id, has_token, added), (private_ref.id.to_string(), true, false)),
            other => panic!("{other:?}"),
        }
        let view = rt.call_node_add(&link, None, "messenger tests, linux").await.unwrap();
        let mine = &view.nodes[0];
        assert_eq!((mine.id.as_str(), mine.class, mine.source, mine.mine, mine.has_key, mine.used), (private_ref.id.to_string().as_str(), CallNodeClass::Own, CallNodeSource::Device, true, true, true));
        assert_eq!((mine.health, mine.private), (CallNodeHealth::Active, Some(true)), "asked at once: it lets me in");
        assert_eq!(node.devices(private).len(), 1);
        assert!(matches!(rt.link_inspect(&link).await.unwrap(), crate::LinkView::CallNode { added: true, .. }));
        // The token is spent.
        match rt.call_node_add(&link, None, "again").await {
            Err(MessengerError::Invalid(code)) => assert_eq!(code, "call_node_bad_invite"),
            other => panic!("{other:?}"),
        }
        // The call nodes of a call come from the same sets: mine first,
        // with the credentials of this device.
        let st = rt.call_state().await.unwrap();
        assert_eq!((st.nodes[0].id.as_str(), st.nodes[0].class.as_str(), st.nodes[0].has_key), (private_ref.id.to_string().as_str(), "own", true));

        // Mine and the project's: the registry's node is listed and not
        // used; the registry is not asked.
        let view = rt.call_nodes_set_trust(CallTrust::ProjectAndOwn).await.unwrap();
        assert_eq!(view.trust, CallTrust::ProjectAndOwn);
        assert!(!view.nodes.iter().find(|n| n.source == CallNodeSource::Registry).unwrap().used);
        assert!(view.nodes.iter().filter(|n| n.source != CallNodeSource::Registry).all(|n| n.used));
        // Mine only: the manifest's nodes are not used and a probe speaks
        // to mine alone.
        let view = rt.call_nodes_set_trust(CallTrust::OwnOnly).await.unwrap();
        assert!(view.nodes.iter().all(|n| n.used == n.mine), "{view:?}");
        rt.calls.nodes.clear();
        rt.call_nodes_list(true).await.unwrap();
        let spoken: Vec<String> = rt.calls.nodes.known().into_iter().map(|k| k.id).collect();
        assert_eq!(spoken, vec![private_ref.id.to_string()], "mine is asked, neither the manifest's nor the registry's");
        let asked = registry.asked().load(Ordering::SeqCst);
        rt.call_nodes_refresh().await.unwrap();
        rt.call_registry_tick().await;
        assert_eq!(registry.asked().load(Ordering::SeqCst), asked, "nor the registry");

        // The operator revokes the device: the node answers and lets me
        // in no more.
        let device = node.devices(private)[0].device_id.clone();
        assert!(node.revoke(private, &device));
        rt.calls.nodes.clear();
        let view = rt.call_nodes_list(true).await.unwrap();
        assert_eq!(view.nodes[0].health, CallNodeHealth::Refused);

        // Removed: the secret goes with it; a node not mine is refused.
        let view = rt.call_node_remove(&private_ref.id.to_string()).await.unwrap();
        assert!(view.nodes.iter().all(|n| !n.mine));
        assert!(rt.calls.servers().devices().await.unwrap().is_empty());
        match rt.call_node_remove(&private_ref.id.to_string()).await {
            Err(MessengerError::Invalid(code)) => assert_eq!(code, "call_node_unknown"),
            other => panic!("{other:?}"),
        }
        // A reference with a key goes into my own list.
        let view = rt.call_node_add(&private_ref.to_string(), Some(" k1 ".into()), "x").await.unwrap();
        assert_eq!((view.nodes[0].source, view.nodes[0].has_key), (CallNodeSource::Setting, true));
        assert_eq!(rt.call_own_nodes().await.unwrap()[0].reference, private_ref.to_string());
        rt.call_node_remove(&private_ref.id.to_string()).await.unwrap();
        assert!(rt.call_own_nodes().await.unwrap().is_empty());

        // A locked store: the token is not spent.
        let token = node.invite(private, 1, 3600);
        secrets.set_unlocked(false);
        let link = format!("veydan://call-node/{}?a={}&t={token}", private_ref.id, private_ref.addr.to_string().replace(':', "%3A"));
        assert!(matches!(rt.call_node_add(&link, None, "x").await, Err(MessengerError::SecretsLocked)));
        secrets.set_unlocked(true);
        rt.call_nodes_set_trust(CallTrust::Any).await.unwrap();
        rt.call_node_add(&link, None, "x").await.unwrap();
        rt.shutdown().await;
    }

    /// The registry is the project's infrastructure, as the manifest and
    /// the list of bridges: under the default level `any` it is still not
    /// asked before the servers are chosen, with own servers, or in the
    /// silent mode, neither by the tick nor by the core's own reading of
    /// the sets nor by the button; once it may be, the next tick asks at
    /// once, not after the pause that the refused asks left behind.
    #[tokio::test]
    async fn the_registry_is_asked_only_with_the_project_servers() {
        let dir = tempfile::tempdir().unwrap();
        let node = FakeNode::new(FakeEngine::new());
        let registry = FakeRegistry::new();
        let volunteer = node.add_node();
        let volunteer_ref = node.node(volunteer, NodeClass::Volunteer).node;
        registry.list(vec![(volunteer_ref.clone(), "eu", 10)]);
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let (fetch, root) = (registry.fetch(), registry.root_hex());
        let backends = CallBackends {
            engine: Some(Arc::new(FakeEngine::new())),
            nodes: node.client(),
            rooms: Arc::new(node.clone()),
            // The fetch of the build, gated as the build gates it.
            registry: RegistrySource::Given(Box::new(move |store: messenger_store::Store| {
                let gated = crate::calls::gated_registry_fetch(store.clone(), fetch);
                messenger_calls::Registry::with_trust(store, gated, vec!["https://registry.test/vlink".into()], root)
            })),
        };
        // No servers chosen yet: the pool connects nowhere, silent or not.
        let rt = MessengerRuntime::start_with_backends(cfg, Arc::new(MemorySecretStore::unlocked()), backends).await.unwrap();
        let asked = || registry.asked().load(Ordering::SeqCst);
        let registry_of = rt.calls.servers().registry().unwrap();
        let (rt_ref, registry_ref) = (&rt, &registry_of);
        let every_way = move || async move {
            let (rt, registry_of) = (rt_ref, registry_ref);
            rt.call_registry_tick().await;
            let view = rt.call_nodes_list(false).await.unwrap();
            rt.call_nodes_refresh().await.unwrap();
            registry_of.settle().await;
            view
        };

        let view = every_way().await;
        assert_eq!((view.trust, view.registry_paused, asked()), (CallTrust::Any, true, 0), "before the onboarding");
        // Own servers (as the setting says; a relay of my own is not needed
        // for what the gate reads).
        settings::set(&rt.store, crate::relays::KEY_MODE, "own").await.unwrap();
        let view = every_way().await;
        assert_eq!((view.registry_paused, asked()), (true, 0), "own servers");
        // The project's servers, as the gate reads them, in the silent
        // mode (the pool keeps the own servers' rows: none).
        rt.relays().set_silent(true).await.unwrap();
        settings::set(&rt.store, crate::relays::KEY_MODE, "veydan").await.unwrap();
        let view = every_way().await;
        assert_eq!((view.registry_paused, asked()), (true, 0), "the silent mode");
        assert!(view.nodes.iter().all(|n| n.source != CallNodeSource::Registry));

        // The silent mode off: the next tick asks at once.
        rt.relays().set_silent(false).await.unwrap();
        rt.call_registry_tick().await;
        tokio::time::timeout(Duration::from_secs(5), async {
            while registry_of.checked_at().await.unwrap().is_none() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the registry is asked at once when it may be");
        assert_eq!(asked(), 1);
        let view = rt.call_nodes_list(false).await.unwrap();
        assert!(!view.registry_paused);
        assert!(view.nodes.iter().any(|n| n.id == volunteer_ref.id.to_string() && n.source == CallNodeSource::Registry && n.used), "{view:?}");
        rt.call_nodes_refresh().await.unwrap();
        assert_eq!(asked(), 2, "the button asks");
        // Silent again: the button asks nothing, the list kept is shown.
        rt.relays().set_silent(true).await.unwrap();
        let view = rt.call_nodes_refresh().await.unwrap();
        assert_eq!((view.registry_paused, asked()), (true, 2));
        assert!(view.nodes.iter().any(|n| n.source == CallNodeSource::Registry));
        rt.shutdown().await;
    }

    /// A probe that missed a node says "unreachable" only until the node
    /// is heard from again: a call that reached it later shows on its row.
    #[tokio::test]
    async fn a_miss_of_a_probe_gives_way_to_a_later_answer() {
        let dir = tempfile::tempdir().unwrap();
        let node = FakeNode::new(FakeEngine::new());
        let registry = FakeRegistry::new();
        let mine = node.add_node();
        let mine_ref = node.node(mine, NodeClass::Own).node;
        let rt = runtime(&dir, &node, &registry, Arc::new(MemorySecretStore::unlocked())).await;
        rt.call_node_add(&mine_ref.to_string(), None, "x").await.unwrap();
        let row = |view: &CallNodesView| view.nodes.iter().find(|n| n.id == mine_ref.id.to_string()).cloned().unwrap();

        node.set_silent(mine, true);
        rt.calls.nodes.clear();
        let view = rt.call_nodes_list(true).await.unwrap();
        assert_eq!((row(&view).health, row(&view).rtt_ms), (CallNodeHealth::Unreachable, None));
        // Still so on another look: nothing newer is known.
        assert_eq!(row(&rt.call_nodes_list(false).await.unwrap()).health, CallNodeHealth::Unreachable);

        // The network is back and a call reaches the node (the way a call
        // asks it); the panel is looked at again without a probe.
        node.set_silent(mine, false);
        rt.calls.nodes.access(&CallNode::new(mine_ref.clone(), NodeClass::Own), now()).await.unwrap();
        let view = rt.call_nodes_list(false).await.unwrap();
        assert_eq!(row(&view).health, CallNodeHealth::Active, "{view:?}");
        assert!(row(&view).rtt_ms.is_some());
        assert!(rt.calls.unreachable.lock().unwrap().is_empty());
        rt.shutdown().await;
    }

    /// The card of a link with an invitation offers to exchange it unless
    /// this device holds its own credentials on the node: an entry of my
    /// own list (a reference, a shared key) is not that. A link without an
    /// invitation is "added" either way.
    #[tokio::test]
    async fn an_invitation_to_a_node_of_my_list_is_still_offered() {
        let dir = tempfile::tempdir().unwrap();
        let node = FakeNode::new(FakeEngine::new());
        let registry = FakeRegistry::new();
        let private = node.add_node();
        let private_ref = node.node(private, NodeClass::Own).node;
        node.set_private(private, true, &["k1"]);
        let rt = runtime(&dir, &node, &registry, Arc::new(MemorySecretStore::unlocked())).await;
        let addr = private_ref.addr.to_string().replace(':', "%3A");
        let bare = format!("veydan://call-node/{}?a={addr}", private_ref.id);
        let rt_ref = &rt;
        let added = move |link: String| {
            let rt = rt_ref;
            async move {
                match rt.link_inspect(&link).await.unwrap() {
                    crate::LinkView::CallNode { added, has_token, .. } => (has_token, added),
                    other => panic!("{other:?}"),
                }
            }
        };

        // A link without an invitation and its key given by hand: in my
        // own list, and the node lets me in.
        let view = rt.call_node_add(&bare, Some("k1".into()), "x").await.unwrap();
        let row = &view.nodes[0];
        assert_eq!((row.source, row.has_key, row.health), (CallNodeSource::Setting, true, CallNodeHealth::Active), "{view:?}");
        let token = node.invite(private, 1, 3600);
        let invitation = format!("{bare}&t={token}");
        assert_eq!(added(invitation.clone()).await, (true, false), "the invitation is still to be exchanged");
        assert_eq!(added(bare.clone()).await, (false, true));
        // Exchanged: the device's own credentials, and now "added".
        let view = rt.call_node_add(&invitation, None, "x").await.unwrap();
        assert_eq!(view.nodes[0].source, CallNodeSource::Device);
        assert_eq!(added(format!("{bare}&t=another")).await, (true, true));
        rt.shutdown().await;
    }

    /// The gate of the build's fetch: nothing goes out unless the
    /// project's servers are chosen and the silent mode is off.
    #[tokio::test]
    async fn the_gate_of_the_fetch() {
        let store = messenger_store::Store::open_in_memory().await.unwrap();
        let registry = FakeRegistry::new();
        let gated = crate::calls::gated_registry_fetch(store.clone(), registry.fetch());
        let ask = || gated("https://registry.test/vlink/v1/calls".into());
        for (mode, silent) in [(None, false), (Some("own"), false), (Some("veydan"), true)] {
            if let Some(mode) = mode {
                settings::set(&store, crate::relays::KEY_MODE, mode).await.unwrap();
            }
            settings::set_bool(&store, crate::relays::KEY_SILENT, silent).await.unwrap();
            assert!(matches!(ask().await, Err(MessengerError::Transport(_))), "{mode:?} {silent}");
        }
        assert_eq!(registry.asked().load(Ordering::SeqCst), 0);
        settings::set_bool(&store, crate::relays::KEY_SILENT, false).await.unwrap();
        assert!(ask().await.is_ok());
        assert_eq!(registry.asked().load(Ordering::SeqCst), 1);
    }
}
