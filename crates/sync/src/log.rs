// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Op log data types. What each device writes and what a reader remembers.

use crate::hlc::Hlc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One change to one entity. `payload` is entity-specific and opaque here.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Op {
    pub entity_type: String,
    pub entity_id: String,
    pub hlc: Hlc,
    pub deleted: bool,
    #[serde(default)]
    pub payload: serde_json::Value,
}

/// Plaintext of `devices/<id>/log/<seq>.bin`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Chunk {
    /// SHA-256 of the previous chunk file; empty for seq 1.
    pub prev_hash: String,
    pub seq: u64,
    pub ops: Vec<Op>,
}

/// Plaintext of `devices/<id>/snapshot.bin`: latest op per entity up to `up_to_seq`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub up_to_seq: u64,
    /// SHA-256 of the chunk file at `up_to_seq`; the next chunk chains to it.
    pub head_hash: String,
    pub ops: Vec<Op>,
}

/// Last verified position in a peer's log.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerHead {
    pub seq: u64,
    pub hash: String,
}

/// Everything a device must remember between runs. Persisted by the caller.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LocalState {
    pub own_seq: u64,
    pub own_head_hash: String,
    pub peers: HashMap<String, PeerHead>,
}

pub fn chunk_key(device_id: &str, seq: u64) -> String {
    format!("devices/{device_id}/log/{seq:08}.bin")
}

pub fn snapshot_key(device_id: &str) -> String {
    format!("devices/{device_id}/snapshot.bin")
}

pub fn blob_key(name: &str) -> String {
    format!("blobs/{name}")
}

pub fn chunk_ident(device_id: &str, seq: u64) -> String {
    format!("{device_id}/{seq}")
}

pub fn snapshot_ident(device_id: &str) -> String {
    format!("{device_id}/snapshot")
}

/// Sequence number from a chunk key like `devices/x/log/00000012.bin`.
pub fn seq_from_key(key: &str) -> Option<u64> {
    let name = key.rsplit('/').next()?;
    name.strip_suffix(".bin")?.parse().ok()
}

/// Device id from any key under `devices/<id>/...`.
pub fn device_from_key(key: &str) -> Option<&str> {
    key.strip_prefix("devices/")?.split('/').next()
}

/// Keep only the newest op per entity.
pub fn fold_latest(ops: impl IntoIterator<Item = Op>) -> Vec<Op> {
    let mut latest: HashMap<(String, String), Op> = HashMap::new();
    for op in ops {
        let k = (op.entity_type.clone(), op.entity_id.clone());
        match latest.get(&k) {
            Some(cur) if cur.hlc >= op.hlc => {}
            _ => {
                latest.insert(k, op);
            }
        }
    }
    let mut out: Vec<Op> = latest.into_values().collect();
    out.sort_by(|a, b| a.hlc.cmp(&b.hlc));
    out
}
