// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What the core of group calls asks of the groups. `messenger-calls`
//! takes nothing from `messenger-groups` (scripts/boundaries.sh), so the
//! groups are behind this door: the runtime implements it over
//! `GroupService` (`prepare_call_note`, the member list, the node pinned
//! in the settings), the tests over a map.

use crate::servers::CallNode;
use async_trait::async_trait;
use messenger_core::{Envelope, Outbound, PubKey, Result};

#[async_trait]
pub trait GroupAccess: Send + Sync {
    /// The members of the group, me among them. An error when the group
    /// is unknown here or I am not in it.
    async fn members(&self, group_id: &str) -> Result<Vec<PubKey>>;

    /// The node pinned to the group in its settings (class `Group`), with
    /// its access key, when there is one.
    async fn pinned_node(&self, group_id: &str) -> Result<Option<CallNode>>;

    /// The nodes a room of the group may be on besides the pinned one,
    /// when the group lives on a private server (internal/messenger-wire.md,
    /// «Группы на сервере»): that server's own node (class own, with the
    /// server's key), or none, never a node of the project's or a
    /// volunteer's. `None` for a group of the home set, whose rooms take
    /// the sets of servers as before.
    async fn server_nodes(&self, _group_id: &str) -> Result<Option<Vec<CallNode>>> {
        Ok(None)
    }

    /// A quiet note of mine to the group, sealed with the group key as a
    /// message is: what to publish.
    async fn seal_note(&self, group_id: &str, envelope: &Envelope) -> Result<Outbound>;

    /// Someone who is no longer a member still knows the group key: a
    /// note would be refused (`group_key_stale`) until a manager brings a
    /// new one.
    async fn key_stale(&self, _group_id: &str) -> Result<bool> {
        Ok(false)
    }
}
