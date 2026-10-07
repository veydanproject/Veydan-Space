// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Where the nodes of calls come from: sets of servers with priorities
//! (the plan of calls, «Учёт Veydan Cloud»). A call takes its
//! candidates from the sets in order — my own and the group's, the cloud
//! of a subscription, the project's manifest, the registry of
//! volunteers — and never asks where a set came from. Today two sets are
//! filled: the developer setting `call.nodes` (class [`NodeClass::Own`])
//! and whatever the host puts in with [`SettingsServerSets::set_manifest`]
//! (the `calls` field of the manifest, class [`NodeClass::Project`]).
//! The cloud and the registry come later through the same door.

use async_trait::async_trait;
use messenger_core::Result;
use messenger_store::{settings, Store};
use messenger_vlink::BridgeRef;
use serde::{Deserialize, Serialize};
use std::sync::RwLock;

/// `address:port#id` of a call node, the reference a bridge has.
pub type NodeRef = BridgeRef;

/// Whose node it is. The order is the priority: a set earlier in the
/// list is tried before a set later in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeClass {
    /// Mine: a node I run, or was given the key to.
    Own,
    /// Pinned to the group of the call.
    Group,
    /// The cloud of a subscription (not there yet).
    Cloud,
    /// The project's nodes, from the manifest.
    Project,
    /// A volunteer's, from the registry (not there yet).
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
}

/// One node a call may use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallNode {
    pub node: NodeRef,
    pub class: NodeClass,
    /// The key of a private node, when I have one.
    pub access_key: Option<String>,
}

impl CallNode {
    pub fn new(node: NodeRef, class: NodeClass) -> Self {
        Self { node, class, access_key: None }
    }
}

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

/// The sets as the app has them: my own nodes from `call.nodes`, then the
/// project's from the manifest, as the host last told them.
pub struct SettingsServerSets {
    store: Store,
    manifest: RwLock<Vec<CallNode>>,
}

impl SettingsServerSets {
    pub fn new(store: Store) -> Self {
        Self { store, manifest: RwLock::new(Vec::new()) }
    }

    /// The project's nodes, from the manifest; replaces the last list.
    pub fn set_manifest(&self, nodes: Vec<NodeRef>) {
        *self.manifest.write().unwrap() = nodes.into_iter().map(|n| CallNode::new(n, NodeClass::Project)).collect();
    }

    /// Write the developer setting.
    pub async fn set_own(&self, nodes: &[CallNode]) -> Result<()> {
        let list: Vec<NodeEntry> = nodes
            .iter()
            .map(|n| NodeEntry::Keyed { reference: n.node.to_string(), key: n.access_key.clone() })
            .collect();
        settings::set(&self.store, KEY_CALL_NODES, &serde_json::to_string(&list)?).await
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
    async fn call_nodes(&self) -> Result<Vec<CallNode>> {
        let mut out = match settings::get(&self.store, KEY_CALL_NODES).await? {
            Some(json) => parse_own_nodes(&json),
            None => Vec::new(),
        };
        let manifest = self.manifest.read().unwrap().clone();
        for n in manifest {
            if !out.iter().any(|o| o.node.id == n.node.id) {
                out.push(n);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "fda09da75199c4e04601a9df309710fab15cd7b2b806ca3b2bbab202582a6dca";

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
}
