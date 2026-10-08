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

    /// A quiet note of mine to the group, sealed with the group key as a
    /// message is: what to publish.
    async fn seal_note(&self, group_id: &str, envelope: &Envelope) -> Result<Outbound>;
}
