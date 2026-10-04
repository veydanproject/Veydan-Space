// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Which way to the servers the app was using when it last ran.
//!
//! The app decides whether the project's servers are reached through a
//! bridge, and writes the decision down. The process a push starts has no
//! app to ask: it reads what was written and goes the same way.

use messenger_core::Result;
use messenger_store::{settings, Store};
use messenger_vlink::{BridgeRef, Net, NetConfig};
use serde::{Deserialize, Serialize};

/// `1` while the project's servers are reached through a bridge.
pub const KEY_ACTIVE: &str = "net.active";
/// The hosts and the bridges the decision was made for.
pub const KEY_SNAPSHOT: &str = "net.snapshot";

#[derive(Default, Serialize, Deserialize)]
struct Snapshot {
    hosts: Vec<String>,
    bridges: Vec<String>,
}

/// Writes down what is in force.
pub async fn save(store: &Store, config: &NetConfig) -> Result<()> {
    let snapshot = Snapshot {
        hosts: config.hosts.iter().cloned().collect(),
        bridges: config.bridges.iter().map(BridgeRef::to_string).collect(),
    };
    settings::set(store, KEY_SNAPSHOT, &serde_json::to_string(&snapshot)?).await?;
    settings::set_bool(store, KEY_ACTIVE, config.active).await
}

/// What was written down. Nothing written is "directly".
pub async fn saved(store: &Store) -> Result<NetConfig> {
    let snapshot: Snapshot = settings::get(store, KEY_SNAPSHOT)
        .await?
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    Ok(NetConfig {
        active: settings::get_bool(store, KEY_ACTIVE, false).await?,
        hosts: snapshot.hosts.into_iter().collect(),
        // A reference this build cannot read is passed over, not an error.
        bridges: snapshot.bridges.iter().filter_map(|b| b.parse().ok()).collect(),
    })
}

/// Puts what was written down in force for this process. Says whether
/// that means going through a bridge.
pub async fn apply_saved(store: &Store) -> Result<bool> {
    let config = saved(store).await?;
    Net::global().configure(config).await?;
    Ok(Net::global().is_active())
}
