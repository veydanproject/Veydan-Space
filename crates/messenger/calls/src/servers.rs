// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Where the nodes of calls come from: sets of servers with priorities
//! (the plan of calls, «Учёт Veydan Cloud»). A call takes its
//! candidates from the sets in order — my own and the group's, the cloud
//! of a subscription, the project's manifest, the registry of
//! volunteers — and never asks where a set came from.
//!
//! The sets of [`SettingsServerSets`], in order:
//!
//! 1. **own** (class [`NodeClass::Own`]): the private nodes this device
//!    was invited to (`veydan://call-node/…`, the credentials of the
//!    device in the `SecretStore`, the index in `call.devices`), then the
//!    developer setting `call.nodes`;
//! 2. **cloud** ([`NodeClass::Cloud`]): the nodes of a subscription, when
//!    there is one — empty today, the door is `set_cloud`;
//! 3. **manifest** ([`NodeClass::Project`], or `Volunteer` when the
//!    manifest says so): what the host puts in with `set_manifest`;
//! 4. **registry** ([`NodeClass::Volunteer`]): the signed list of the
//!    registry ([`crate::registry::Registry`]), least loaded first.
//!
//! The **trust level** (`call.trust`, [`TrustLevel`]) cuts the sets: own
//! only → 1; the project's and own → 1–3 without volunteers; any → all.
//! The node pinned to a group (class `Group`) is the group's choice and
//! passes every level; the group service puts it in itself.
//!
//! The order of the list is the priority, by **tiers** ([`NodeClass::tier`]):
//! own, the group's, the cloud's each a tier of its own, asked one after
//! another; the project's and the volunteers' **one tier** (the owner's
//! decision of 2026-10-09, «берём ближайший быстрый»): asked together,
//! the nearest by the round trip taken, the load breaking a tie. So
//! under `any` a volunteer near me carries the call before a far node
//! of the project, and under `project_and_own` the tier has no
//! volunteer in it. How the tier is asked is with `NodeClient::pick`.

use crate::registry::Registry;
use async_trait::async_trait;
use messenger_core::{MessengerError, Result, SecretStore};
use messenger_store::{settings, Store};
use messenger_vlink::BridgeRef;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};

/// `address:port#id` of a call node, the reference a bridge has.
pub type NodeRef = BridgeRef;

/// Whose node it is. The order is the priority: a set earlier in the
/// list is tried before a set later in it — by tiers ([`Self::tier`]),
/// where the project's and the volunteers' are one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeClass {
    /// Mine: a node I run, was given the key to, or was invited to.
    Own,
    /// Pinned to the group of the call.
    Group,
    /// The cloud of a subscription (not there yet).
    Cloud,
    /// The project's nodes, from the manifest.
    Project,
    /// A volunteer's, from the registry (or named so by the manifest).
    Volunteer,
}

impl NodeClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Own => "own",
            Self::Group => "group",
            Self::Cloud => "cloud",
            Self::Project => "project",
            Self::Volunteer => "volunteer",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "own" => Some(Self::Own),
            "group" => Some(Self::Group),
            "cloud" => Some(Self::Cloud),
            "project" => Some(Self::Project),
            "volunteer" => Some(Self::Volunteer),
            _ => None,
        }
    }

    /// The tier the class is asked in: the nodes of one tier are asked
    /// together and the nearest of them taken; a lower tier is asked
    /// only when no node of a higher one answers (whoever runs their
    /// own node does not want the others to see the call). The
    /// project's and the volunteers' are one tier: among them the
    /// nearest wins, whoever runs it.
    pub fn tier(self) -> u8 {
        match self {
            Self::Own => 0,
            Self::Group => 1,
            Self::Cloud => 2,
            Self::Project | Self::Volunteer => 3,
        }
    }
}

/// Which nodes a call may use: the setting `call.trust`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    /// Any: own, the project's, volunteers'.
    #[default]
    Any,
    /// The project's and own: no volunteer, from the registry or the
    /// manifest alike.
    ProjectAndOwn,
    /// Own only: neither the registry nor the manifest is read.
    OwnOnly,
}

impl TrustLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::ProjectAndOwn => "project_and_own",
            Self::OwnOnly => "own_only",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "any" => Some(Self::Any),
            "project_and_own" => Some(Self::ProjectAndOwn),
            "own_only" => Some(Self::OwnOnly),
            _ => None,
        }
    }

    /// Whether a node of `class` may be used under this level.
    pub fn allows(self, class: NodeClass) -> bool {
        match self {
            Self::Any => true,
            Self::ProjectAndOwn => class != NodeClass::Volunteer,
            Self::OwnOnly => matches!(class, NodeClass::Own | NodeClass::Group),
        }
    }
}

/// `any` | `project_and_own` | `own_only`, see [`TrustLevel`].
pub const KEY_CALL_TRUST: &str = "call.trust";

/// One node a call may use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallNode {
    pub node: NodeRef,
    pub class: NodeClass,
    /// The key of a private node, when I have one: a shared access key
    /// as written, or the credentials of this device in the form
    /// [`device_key`] (`dev⏎<device id>⏎<secret>`, which no shared key
    /// can be).
    pub access_key: Option<String>,
}

impl CallNode {
    pub fn new(node: NodeRef, class: NodeClass) -> Self {
        Self { node, class, access_key: None }
    }

    pub fn with_key(node: NodeRef, class: NodeClass, access_key: Option<String>) -> Self {
        Self { node, class, access_key }
    }

    /// A node I was invited to: the credentials of this device as its key.
    pub fn with_device(class: NodeClass, creds: &DeviceCredentials) -> Self {
        Self { node: creds.node.clone(), class, access_key: Some(creds.key()) }
    }

    /// The credentials of this device, when the key is those: the
    /// device id and its secret.
    pub fn device(&self) -> Option<(&str, &str)> {
        self.access_key.as_deref().and_then(split_device_key)
    }

    /// Whether the key is the credentials of this device.
    pub fn is_device(&self) -> bool {
        self.device().is_some()
    }

    /// The key that may be given to others (the members of a group, in
    /// `call.start`): a shared access key, never the credentials of this
    /// device.
    pub fn shared_key(&self) -> Option<String> {
        if self.is_device() {
            None
        } else {
            self.access_key.clone()
        }
    }
}

// ─── The credentials of a device on a private node ─────────────────────────

/// How the credentials of this device are carried in `access_key`:
/// `dev⏎<device id>⏎<secret>` (a newline after `dev` and between the
/// two), so that a node is one value everywhere and the control channel
/// tells a device key from a shared key. The newline is what makes the
/// form unmistakable: a node reads its shared keys one per line of
/// `VCALL_ACCESS_KEYS` (trimmed), so no shared key a node could take
/// holds one, whatever a user types into `call.nodes` or the settings
/// of a group, or a `call.start` carries. The id and the secret a node
/// issues are hex (`services/call/spec/protocol.md`, "Приглашение и
/// устройства") and hold none either.
pub const DEVICE_KEY_PREFIX: &str = "dev\n";

/// The key form of a device's credentials.
pub fn device_key(device_id: &str, secret: &str) -> String {
    format!("{DEVICE_KEY_PREFIX}{device_id}\n{secret}")
}

/// The device id and the secret of a key in the device form; `None` for
/// a shared key (any text without the form's newlines, `dev:…` included).
pub fn split_device_key(key: &str) -> Option<(&str, &str)> {
    let rest = key.strip_prefix(DEVICE_KEY_PREFIX)?;
    let (id, secret) = rest.split_once('\n')?;
    (!id.is_empty() && !secret.is_empty() && !secret.contains('\n')).then_some((id, secret))
}

/// What a private node gave this device for an invitation
/// (`redeem_invite`): kept in the `SecretStore`, never shown whole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceCredentials {
    pub node: NodeRef,
    /// The node's name for this device: what its operator revokes.
    pub device_id: String,
    pub secret: String,
    /// The label the operator gave the invitation, when any: what the
    /// node may be shown as.
    pub label: String,
    /// What this device called itself when it was invited (a product
    /// and a platform).
    pub name: String,
    /// Unix seconds, by this device's clock.
    pub added_at: i64,
}

impl DeviceCredentials {
    /// As `access_key` carries it.
    pub fn key(&self) -> String {
        device_key(&self.device_id, &self.secret)
    }
}

/// The public half of a device's credentials: what `call.devices` lists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceNodeEntry {
    #[serde(rename = "ref")]
    pub reference: String,
    pub device_id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub added_at: i64,
}

impl DeviceNodeEntry {
    pub fn node(&self) -> Option<NodeRef> {
        self.reference.parse().ok()
    }
}

/// The index of the private nodes this device was invited to: a JSON
/// list of [`DeviceNodeEntry`]. The secrets are in the `SecretStore`
/// under [`secret_key_of`].
pub const KEY_CALL_DEVICES: &str = "call.devices";
/// The `SecretStore` key of the secret of a device on the node `id`.
pub const SECRET_DEVICE_PREFIX: &str = "call.device.";

pub fn secret_key_of(node: &NodeRef) -> String {
    format!("{SECRET_DEVICE_PREFIX}{}", node.id)
}

// ─── The sets ──────────────────────────────────────────────────────────────

/// The sets of servers, in order of priority. Implemented over the
/// settings here; a test or the CLI gives a fixed list.
#[async_trait]
pub trait ServerSets: Send + Sync {
    /// Every node a call may use, the most preferred first.
    async fn call_nodes(&self) -> Result<Vec<CallNode>>;
}

/// A fixed list.
pub struct StaticServerSets(pub Vec<CallNode>);

#[async_trait]
impl ServerSets for StaticServerSets {
    async fn call_nodes(&self) -> Result<Vec<CallNode>> {
        Ok(self.0.clone())
    }
}

/// The developer setting: a JSON list of nodes, each a string
/// `address:port#id` or `{"ref": "address:port#id", "key": "<access key>"}`.
pub const KEY_CALL_NODES: &str = "call.nodes";
/// `auto` | `relay_only`, see `RelayPolicy`.
pub const KEY_RELAY_POLICY: &str = "call.relay_policy";

/// Where a node of the sets came from, for the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeSource {
    /// Invited by a link: the credentials of this device.
    Device,
    /// The developer setting `call.nodes`.
    Setting,
    Cloud,
    Manifest,
    Registry,
}

/// One node of the sets as the screen lists it: its class and source,
/// what the registry said of it, and whether the trust level lets it be
/// used. The round trip, the load the node itself reports and whether
/// it answered are with the node client (`NodeClient::known`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeDescription {
    #[serde(rename = "ref")]
    pub reference: String,
    pub id: String,
    pub class: NodeClass,
    pub source: NodeSource,
    pub has_key: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caps: Vec<String>,
    /// How full the registry listed it, percent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load: Option<u8>,
    /// What the registry says of it: `active` | `degraded`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Passes the trust level: a call may use it.
    pub trusted: bool,
}

/// The sets as the app has them.
pub struct SettingsServerSets {
    store: Store,
    secrets: RwLock<Option<Arc<dyn SecretStore>>>,
    manifest: RwLock<Vec<CallNode>>,
    cloud: RwLock<Vec<CallNode>>,
    registry: RwLock<Option<Arc<Registry>>>,
}

impl SettingsServerSets {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            secrets: RwLock::new(None),
            manifest: RwLock::new(Vec::new()),
            cloud: RwLock::new(Vec::new()),
            registry: RwLock::new(None),
        }
    }

    /// Where the credentials of this device on private nodes are kept.
    /// Without it the nodes of `call.devices` are not listed.
    pub fn set_secrets(&self, secrets: Arc<dyn SecretStore>) {
        *self.secrets.write().unwrap() = Some(secrets);
    }

    /// The registry of volunteers: its list, cached, is the last set.
    pub fn set_registry(&self, registry: Arc<Registry>) {
        *self.registry.write().unwrap() = Some(registry);
    }

    pub fn registry(&self) -> Option<Arc<Registry>> {
        self.registry.read().unwrap().clone()
    }

    /// The project's nodes, from the manifest; replaces the last list.
    pub fn set_manifest(&self, nodes: Vec<NodeRef>) {
        self.set_manifest_classed(nodes.into_iter().map(|n| (n, NodeClass::Project)).collect());
    }

    /// The nodes of the manifest with the class it gives each: `project`
    /// or `volunteer` (`cloud` goes through `set_cloud`, when it comes).
    /// Another class is the manifest's mistake and is left out.
    pub fn set_manifest_classed(&self, nodes: Vec<(NodeRef, NodeClass)>) {
        *self.manifest.write().unwrap() = nodes
            .into_iter()
            .filter(|(_, c)| matches!(c, NodeClass::Project | NodeClass::Volunteer))
            .map(|(n, c)| CallNode::new(n, c))
            .collect();
    }

    /// The nodes of a subscription (class `cloud`): the door for Veydan
    /// Cloud. Nobody opens it today.
    pub fn set_cloud(&self, nodes: Vec<NodeRef>) {
        *self.cloud.write().unwrap() = nodes.into_iter().map(|n| CallNode::new(n, NodeClass::Cloud)).collect();
    }

    /// Write the developer setting.
    pub async fn set_own(&self, nodes: &[CallNode]) -> Result<()> {
        let list: Vec<NodeEntry> = nodes
            .iter()
            .map(|n| NodeEntry::Keyed { reference: n.node.to_string(), key: n.access_key.clone() })
            .collect();
        settings::set(&self.store, KEY_CALL_NODES, &serde_json::to_string(&list)?).await
    }

    pub async fn trust(&self) -> Result<TrustLevel> {
        Ok(settings::get(&self.store, KEY_CALL_TRUST).await?.as_deref().and_then(TrustLevel::parse).unwrap_or_default())
    }

    pub async fn set_trust(&self, level: TrustLevel) -> Result<()> {
        settings::set(&self.store, KEY_CALL_TRUST, level.as_str()).await
    }

    // ─── The private nodes this device was invited to ──────────────────

    /// The index of `call.devices`.
    pub async fn devices(&self) -> Result<Vec<DeviceNodeEntry>> {
        let Some(json) = settings::get(&self.store, KEY_CALL_DEVICES).await? else { return Ok(Vec::new()) };
        Ok(serde_json::from_str::<Vec<DeviceNodeEntry>>(&json).unwrap_or_default())
    }

    async fn set_devices(&self, entries: &[DeviceNodeEntry]) -> Result<()> {
        settings::set(&self.store, KEY_CALL_DEVICES, &serde_json::to_string(entries)?).await
    }

    fn secrets(&self) -> Result<Arc<dyn SecretStore>> {
        self.secrets.read().unwrap().clone().ok_or_else(|| MessengerError::Invalid("no secret store for the credentials of this device".into()))
    }

    /// Keep what a node gave this device: the secret in the `SecretStore`,
    /// the rest in the index. A second invitation to the same node
    /// replaces the first.
    pub async fn add_device(&self, creds: &DeviceCredentials) -> Result<()> {
        let secrets = self.secrets()?;
        secrets.put(&secret_key_of(&creds.node), creds.secret.as_bytes()).await?;
        let mut entries = self.devices().await?;
        entries.retain(|e| e.node().is_none_or(|n| n.id != creds.node.id));
        entries.push(DeviceNodeEntry {
            reference: creds.node.to_string(),
            device_id: creds.device_id.clone(),
            label: creds.label.clone(),
            name: creds.name.clone(),
            added_at: creds.added_at,
        });
        self.set_devices(&entries).await
    }

    /// Forget the node `id` (64 hex): the secret and the entry. `false`
    /// when there was none. The node is not told: its operator revokes
    /// the device on the node (`vcall ctl revoke`).
    pub async fn remove_device(&self, id: &str) -> Result<bool> {
        let id = id.to_ascii_lowercase();
        let mut entries = self.devices().await?;
        let before = entries.len();
        let gone: Vec<NodeRef> = entries.iter().filter(|e| e.node().is_some_and(|n| n.id.to_string() == id)).filter_map(|e| e.node()).collect();
        entries.retain(|e| e.node().is_none_or(|n| n.id.to_string() != id));
        if entries.len() == before {
            return Ok(false);
        }
        self.set_devices(&entries).await?;
        if let Ok(secrets) = self.secrets() {
            for node in gone {
                if let Err(e) = secrets.delete(&secret_key_of(&node)).await {
                    tracing::warn!(error = %e, node = %node.id.short(), "call nodes: the secret of the device was not deleted");
                }
            }
        }
        Ok(true)
    }

    /// The credentials of this device on `node`, when it was invited there.
    pub async fn device_for(&self, node: &NodeRef) -> Result<Option<DeviceCredentials>> {
        let Some(entry) = self.devices().await?.into_iter().find(|e| e.node().is_some_and(|n| n.id == node.id)) else { return Ok(None) };
        let Ok(secrets) = self.secrets() else { return Ok(None) };
        let Some(secret) = secrets.get(&secret_key_of(node)).await? else { return Ok(None) };
        let secret = String::from_utf8(secret.to_vec()).map_err(|_| MessengerError::Crypto("the secret of the device is not text".into()))?;
        Ok(Some(DeviceCredentials {
            node: entry.node().unwrap_or_else(|| node.clone()),
            device_id: entry.device_id,
            secret,
            label: entry.label,
            name: entry.name,
            added_at: entry.added_at,
        }))
    }

    /// The nodes this device was invited to, with their credentials
    /// (class `own`). Entries whose secret cannot be read (the store is
    /// locked, the secret is gone) are left out with a line in the log.
    async fn device_nodes(&self) -> Result<Vec<CallNode>> {
        let entries = self.devices().await?;
        if entries.is_empty() {
            return Ok(Vec::new());
        }
        let Ok(secrets) = self.secrets() else {
            tracing::debug!("call nodes: {} invited node(s) and no secret store: not listed", entries.len());
            return Ok(Vec::new());
        };
        let mut out = Vec::with_capacity(entries.len());
        for e in entries {
            let Some(node) = e.node() else { continue };
            match secrets.get(&secret_key_of(&node)).await {
                Ok(Some(secret)) => match std::str::from_utf8(&secret) {
                    Ok(s) => out.push(CallNode::with_key(node, NodeClass::Own, Some(device_key(&e.device_id, s)))),
                    Err(_) => tracing::warn!(node = %node.id.short(), "call nodes: the secret of the device is not text"),
                },
                Ok(None) => tracing::warn!(node = %node.id.short(), "call nodes: the secret of the device is gone"),
                Err(err) => tracing::debug!(error = %err, node = %node.id.short(), "call nodes: the secret of the device cannot be read now"),
            }
        }
        Ok(out)
    }

    /// Every node of the sets, by source, as the screen lists them: the
    /// trust level is told, not applied. The registry is read as
    /// `call_nodes` reads it: asked (in the background, when due) under
    /// `any` only; under the other levels what is cached is listed and
    /// nothing is asked, so that a screen opened under «только свои»
    /// speaks to no registry.
    pub async fn describe(&self) -> Result<Vec<NodeDescription>> {
        let trust = self.trust().await?;
        let mut out: Vec<NodeDescription> = Vec::new();
        let mut push = |d: NodeDescription| {
            if !out.iter().any(|o| o.id == d.id) {
                out.push(d);
            }
        };
        let describe = |n: &CallNode, source: NodeSource| NodeDescription {
            reference: n.node.to_string(),
            id: n.node.id.to_string(),
            class: n.class,
            source,
            has_key: n.access_key.is_some(),
            device_id: None,
            label: None,
            region: None,
            caps: vec![],
            load: None,
            state: None,
            trusted: trust.allows(n.class),
        };
        for e in self.devices().await? {
            let Some(node) = e.node() else { continue };
            push(NodeDescription {
                reference: node.to_string(),
                id: node.id.to_string(),
                class: NodeClass::Own,
                source: NodeSource::Device,
                has_key: true,
                device_id: Some(e.device_id),
                label: Some(if e.label.is_empty() { e.name } else { e.label }),
                region: None,
                caps: vec![],
                load: None,
                state: None,
                trusted: true,
            });
        }
        for n in self.own_setting().await? {
            push(describe(&n, NodeSource::Setting));
        }
        for n in self.cloud.read().unwrap().iter() {
            push(describe(n, NodeSource::Cloud));
        }
        for n in self.manifest.read().unwrap().iter() {
            push(describe(n, NodeSource::Manifest));
        }
        if let Some(registry) = self.registry() {
            let listed = if trust == TrustLevel::Any { registry.nodes().await } else { registry.cached().await };
            for r in listed.unwrap_or_default() {
                let mut d = describe(&CallNode::new(r.node.clone(), NodeClass::Volunteer), NodeSource::Registry);
                d.region = Some(r.region).filter(|s| !s.is_empty());
                d.caps = r.caps;
                d.load = Some(r.load);
                d.state = Some(r.state);
                push(d);
            }
        }
        Ok(out)
    }

    async fn own_setting(&self) -> Result<Vec<CallNode>> {
        Ok(match settings::get(&self.store, KEY_CALL_NODES).await? {
            Some(json) => parse_own_nodes(&json),
            None => Vec::new(),
        })
    }
}

/// One entry of `call.nodes`.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum NodeEntry {
    Plain(String),
    Keyed {
        #[serde(rename = "ref")]
        reference: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
    },
}

/// Read the developer setting; entries that are not a node reference are
/// skipped, so one typo does not take the rest with it.
pub fn parse_own_nodes(json: &str) -> Vec<CallNode> {
    let Ok(entries) = serde_json::from_str::<Vec<serde_json::Value>>(json) else { return Vec::new() };
    entries
        .into_iter()
        .filter_map(|e| {
            let (reference, key) = match serde_json::from_value::<NodeEntry>(e).ok()? {
                NodeEntry::Plain(r) => (r, None),
                NodeEntry::Keyed { reference, key } => (reference, key.filter(|k| !k.is_empty())),
            };
            let node: NodeRef = reference.parse().ok()?;
            Some(CallNode { node, class: NodeClass::Own, access_key: key })
        })
        .collect()
}

#[async_trait]
impl ServerSets for SettingsServerSets {
    /// Own (invited, then the setting) → cloud → manifest → registry,
    /// cut by the trust level, one entry per node (the first wins). The
    /// registry is not even read under `own_only`. The manifest's and
    /// the registry's nodes are one tier for the choice: the order here
    /// (the project's first, the registry's least loaded first) only
    /// breaks the ties of the round trip and the load.
    async fn call_nodes(&self) -> Result<Vec<CallNode>> {
        let trust = self.trust().await?;
        let mut out: Vec<CallNode> = self.device_nodes().await?;
        let mut add = |n: CallNode| {
            if trust.allows(n.class) && !out.iter().any(|o| o.node.id == n.node.id) {
                out.push(n);
            }
        };
        for n in self.own_setting().await? {
            add(n);
        }
        if trust == TrustLevel::OwnOnly {
            return Ok(out);
        }
        for n in self.cloud.read().unwrap().clone() {
            add(n);
        }
        for n in self.manifest.read().unwrap().clone() {
            add(n);
        }
        if trust == TrustLevel::Any {
            if let Some(registry) = self.registry() {
                // The registry's order (least loaded first); a degraded
                // node only when it lists no active one.
                let listed = registry.nodes().await.unwrap_or_default();
                let any_active = listed.iter().any(|r| r.is_active());
                for r in listed.into_iter().filter(|r| r.is_active() || !any_active) {
                    add(CallNode::new(r.node, NodeClass::Volunteer));
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_testkit::MemorySecretStore;

    const ID: &str = "fda09da75199c4e04601a9df309710fab15cd7b2b806ca3b2bbab202582a6dca";

    fn node(seed: u8, addr: &str) -> NodeRef {
        format!("{addr}#{}", format!("{seed:02x}").repeat(32)).parse().unwrap()
    }

    // The trust level and the registry are tried in tests/nodes.rs: the
    // fake registry of the testkit speaks to the library as built, which
    // the unit tests of the library are not.

    #[tokio::test]
    async fn own_nodes_come_first_and_bad_entries_are_skipped() {
        let store = Store::open_in_memory().await.unwrap();
        let sets = SettingsServerSets::new(store.clone());
        assert!(sets.call_nodes().await.unwrap().is_empty());

        let json = format!(r#"["108.61.171.68:8443#{ID}", "garbage", {{"ref": "149.28.37.154:443#{}", "key": "k1"}}, 7]"#, "ab".repeat(32));
        settings::set(&store, KEY_CALL_NODES, &json).await.unwrap();
        let project: NodeRef = format!("45.93.201.244:443#{}", "cd".repeat(32)).parse().unwrap();
        sets.set_manifest(vec![project.clone(), format!("108.61.171.68:8443#{ID}").parse().unwrap()]);

        let nodes = sets.call_nodes().await.unwrap();
        assert_eq!(nodes.len(), 3, "two of mine, one of the project; the project's copy of mine is not listed twice");
        assert_eq!(nodes[0].class, NodeClass::Own);
        assert_eq!(nodes[0].node.id.to_string(), ID);
        assert_eq!(nodes[0].access_key, None);
        assert_eq!(nodes[1].access_key.as_deref(), Some("k1"));
        assert_eq!(nodes[2].class, NodeClass::Project);
        assert_eq!(nodes[2].node, project);

        // Written back and read again the same.
        sets.set_own(&nodes[..2]).await.unwrap();
        assert_eq!(&sets.call_nodes().await.unwrap()[..2], &nodes[..2]);
        assert!(NodeClass::Own < NodeClass::Project, "the order of the classes is the priority");
    }

    #[tokio::test]
    async fn an_invited_node_is_mine_with_the_credentials_of_this_device_and_can_be_forgotten() {
        let store = Store::open_in_memory().await.unwrap();
        let sets = SettingsServerSets::new(store.clone());
        let private = node(8, "203.0.113.8:8443");
        let creds = DeviceCredentials {
            node: private.clone(),
            device_id: "d-1".into(),
            secret: "s3cr3t".into(),
            label: "my phone".into(),
            name: "Veydan Chat, Android".into(),
            added_at: 5,
        };
        assert!(sets.add_device(&creds).await.is_err(), "no secret store: nothing is kept");
        let secrets = Arc::new(MemorySecretStore::unlocked());
        sets.set_secrets(secrets.clone());
        sets.add_device(&creds).await.unwrap();
        sets.set_manifest(vec![node(2, "203.0.113.2:8443")]);

        let nodes = sets.call_nodes().await.unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].node, private);
        assert_eq!(nodes[0].class, NodeClass::Own);
        assert_eq!(nodes[0].device(), Some(("d-1", "s3cr3t")));
        assert_eq!(nodes[0].shared_key(), None, "the credentials of this device are given to nobody");
        assert_eq!(nodes[0].access_key.as_deref(), Some("dev\nd-1\ns3cr3t"));
        assert_eq!(sets.device_for(&private).await.unwrap(), Some(creds.clone()));
        assert_eq!(secrets.get(&secret_key_of(&private)).await.unwrap().unwrap().as_slice(), b"s3cr3t");
        let d = &sets.describe().await.unwrap()[0];
        assert_eq!((d.source, d.device_id.as_deref(), d.label.as_deref(), d.has_key), (NodeSource::Device, Some("d-1"), Some("my phone"), true));
        assert!(!serde_json::to_string(&sets.devices().await.unwrap()).unwrap().contains("s3cr3t"), "the index holds no secret");

        // The vault is locked: the node is not listed, and listed again
        // when it opens.
        secrets.set_unlocked(false);
        assert_eq!(sets.call_nodes().await.unwrap().len(), 1);
        secrets.set_unlocked(true);
        assert_eq!(sets.call_nodes().await.unwrap().len(), 2);

        // A shared key stays a shared key.
        let shared = CallNode::with_key(node(2, "203.0.113.2:8443"), NodeClass::Own, Some("k".into()));
        assert_eq!(shared.shared_key().as_deref(), Some("k"));
        assert_eq!(split_device_key("dev\n\nx"), None);
        assert_eq!(split_device_key("dev\na\n"), None);
        assert_eq!(split_device_key("dev\na\nb\nc"), None, "a secret holds no newline");
        assert_eq!(split_device_key(&device_key("a", "b:c")), Some(("a", "b:c")));

        assert!(sets.remove_device(&private.id.to_string().to_uppercase()).await.unwrap());
        assert!(!sets.remove_device(&private.id.to_string()).await.unwrap());
        assert_eq!(sets.call_nodes().await.unwrap().len(), 1);
        assert_eq!(secrets.get(&secret_key_of(&private)).await.unwrap(), None, "the secret went with the entry");
    }

    /// A shared key of any shape is a shared key: an operator may hand
    /// out `dev:team:2026`, and it goes to the node as `access.key`,
    /// whether typed into `call.nodes` or carried by a `call.start`.
    /// Only what this device was issued (the form with newlines, which
    /// no line of a node's key file can be) is the credentials of a
    /// device.
    #[tokio::test]
    async fn a_shared_key_that_looks_like_a_device_key_stays_a_shared_key() {
        use crate::node_client::Access;
        let store = Store::open_in_memory().await.unwrap();
        let sets = SettingsServerSets::new(store.clone());
        let json = format!(r#"[{{"ref": "203.0.113.1:8443#{}", "key": "dev:team:2026"}}]"#, "01".repeat(32));
        settings::set(&store, KEY_CALL_NODES, &json).await.unwrap();
        let nodes = sets.call_nodes().await.unwrap();
        assert_eq!(nodes[0].device(), None);
        assert!(!nodes[0].is_device());
        assert_eq!(nodes[0].shared_key().as_deref(), Some("dev:team:2026"), "told to the group as it is");
        let access = Access::of(&nodes[0]);
        assert_eq!((access.key.as_deref(), access.device.is_none()), (Some("dev:team:2026"), true), "sent as a shared key");

        // The same shape as a group's key, as a `call.start` carries it.
        let carried = CallNode::with_key(node(2, "203.0.113.2:8443"), NodeClass::Group, Some("dev:a:b".into()));
        assert_eq!(Access::of(&carried).key.as_deref(), Some("dev:a:b"));
        assert_eq!(carried.shared_key().as_deref(), Some("dev:a:b"));

        // The credentials of this device are told apart by the form.
        let creds = DeviceCredentials { node: node(3, "203.0.113.3:8443"), device_id: "d".into(), secret: "s".into(), label: String::new(), name: String::new(), added_at: 0 };
        let mine = CallNode::with_device(NodeClass::Own, &creds);
        assert!(mine.is_device() && mine.shared_key().is_none());
        let access = Access::of(&mine);
        assert_eq!(access.key, None);
        assert_eq!(access.device.map(|d| (d.id, d.secret)), Some(("d".into(), "s".into())));
        assert!(!mine.access_key.as_deref().unwrap().contains("dev:"), "the readable prefix of before is no part of the form");
    }
}
