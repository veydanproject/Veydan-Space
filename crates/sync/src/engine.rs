// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Vault engine: create/open, push own ops, pull peers' ops, blobs, compaction.

use crate::envelope::{self, Kind};
use crate::keys::{Keys, Manifest, Vmk, MANIFEST_KEY};
use crate::large_files::{LargeFileConfig, LargeFileStore};
use crate::log::{
    blob_key, chunk_ident, chunk_key, device_from_key, fold_latest, seq_from_key, snapshot_ident,
    snapshot_key, Chunk, LocalState, Op, PeerHead, Snapshot,
};
use crate::storage::{is_noise_key, Storage};
use crate::{sha256_hex, Result, SyncError};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use std::collections::BTreeSet;

/// What a storage location contains before we touch it.
#[derive(Debug)]
pub enum Probe {
    Empty,
    Vault(Manifest),
    Foreign,
}

/// One peer seen under `devices/` during a pull.
#[derive(Debug, Clone, Default)]
pub struct PullPeer {
    pub device_id: String,
    pub head_seq: u64,
    /// Storage reads attempted for this peer.
    pub files: u32,
    pub ops: usize,
    pub error: Option<String>,
}

/// Ops from peers plus per-peer failures that did not stop the others.
#[derive(Debug, Default)]
pub struct PullResult {
    pub ops: Vec<Op>,
    pub errors: Vec<(String, String)>,
    pub peers: Vec<PullPeer>,
}

pub struct Engine {
    storage: Box<dyn Storage>,
    vault_id: String,
    device_id: String,
    keys: Keys,
}

fn name_key(device_id: &str) -> String {
    format!("devices/{device_id}/name.bin")
}

fn name_ident(device_id: &str) -> String {
    format!("device-name/{device_id}")
}

/// Snapshot (if the chain has a gap) plus every chunk past `head`.
fn planned_gets(peer: &str, head: &PeerHead, all: &[String]) -> Vec<String> {
    let log_prefix = format!("devices/{peer}/log/");
    let mut seqs: Vec<u64> = all
        .iter()
        .filter(|k| k.starts_with(&log_prefix))
        .filter_map(|k| seq_from_key(k))
        .collect();
    seqs.sort_unstable();
    let mut keys = Vec::new();
    if seqs.iter().copied().find(|s| *s > head.seq) != Some(head.seq + 1) {
        keys.push(snapshot_key(peer));
    }
    for seq in seqs {
        if seq > head.seq {
            keys.push(chunk_key(peer, seq));
        }
    }
    keys
}

impl Engine {
    pub async fn probe(storage: &dyn Storage) -> Result<Probe> {
        if let Some(bytes) = storage.get(MANIFEST_KEY).await? {
            return Ok(Probe::Vault(Manifest::from_json(&bytes)?));
        }
        let keys = storage.list("").await?;
        if keys.iter().any(|k| !is_noise_key(k)) {
            return Ok(Probe::Foreign);
        }
        Ok(Probe::Empty)
    }

    /// New vault in an empty storage.
    pub async fn create(
        storage: Box<dyn Storage>,
        passphrase: &str,
        device_id: &str,
    ) -> Result<(Engine, Vmk)> {
        match Self::probe(storage.as_ref()).await? {
            Probe::Empty => {}
            Probe::Vault(_) => return Err(SyncError::VaultExists),
            Probe::Foreign => return Err(SyncError::ForeignStorage),
        }
        let (manifest, vmk) = Manifest::create(passphrase)?;
        storage.put(MANIFEST_KEY, &manifest.to_json()?).await?;
        let engine = Self::with_key(storage, &vmk, &manifest.vault_id, device_id);
        Ok((engine, vmk))
    }

    /// Join an existing vault with its passphrase.
    pub async fn open(
        storage: Box<dyn Storage>,
        passphrase: &str,
        device_id: &str,
    ) -> Result<(Engine, Vmk)> {
        let manifest = match Self::probe(storage.as_ref()).await? {
            Probe::Vault(m) => m,
            Probe::Empty => return Err(SyncError::NoVault),
            Probe::Foreign => return Err(SyncError::ForeignStorage),
        };
        let vmk = manifest.unlock(passphrase)?;
        let engine = Self::with_key(storage, &vmk, &manifest.vault_id, device_id);
        Ok((engine, vmk))
    }

    /// Reopen with a stored master key; `vault_id` is what the device joined.
    pub fn with_key(
        storage: Box<dyn Storage>,
        vmk: &Vmk,
        vault_id: &str,
        device_id: &str,
    ) -> Engine {
        Engine {
            storage,
            vault_id: vault_id.to_string(),
            device_id: device_id.to_string(),
            keys: Keys::derive(vmk, vault_id),
        }
    }

    /// Guard against pointing a joined device at a different vault.
    pub async fn verify_manifest(&self) -> Result<()> {
        match Self::probe(self.storage.as_ref()).await? {
            Probe::Vault(m) if m.vault_id == self.vault_id => Ok(()),
            Probe::Vault(_) => Err(SyncError::VaultMismatch),
            Probe::Empty => Err(SyncError::NoVault),
            Probe::Foreign => Err(SyncError::ForeignStorage),
        }
    }

    pub async fn change_passphrase(&self, vmk: &Vmk, old: &str, new: &str) -> Result<()> {
        let bytes = self
            .storage
            .get(MANIFEST_KEY)
            .await?
            .ok_or(SyncError::NoVault)?;
        let mut manifest = Manifest::from_json(&bytes)?;
        if manifest.vault_id != self.vault_id {
            return Err(SyncError::VaultMismatch);
        }
        manifest.unlock(old)?;
        manifest.rewrap(vmk, new)?;
        self.storage.put(MANIFEST_KEY, &manifest.to_json()?).await
    }

    pub fn vault_id(&self) -> &str {
        &self.vault_id
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// Write this device's display name if it changed. Ciphertext only.
    pub async fn publish_device_name(&self, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Ok(());
        }
        if self.read_device_name(&self.device_id).await?.as_deref() == Some(name) {
            return Ok(());
        }
        let bytes = envelope::seal(
            &self.keys.blob,
            &self.vault_id,
            Kind::Blob,
            &name_ident(&self.device_id),
            name.as_bytes(),
        )?;
        self.storage.put(&name_key(&self.device_id), &bytes).await
    }

    pub async fn read_device_name(&self, device_id: &str) -> Result<Option<String>> {
        let Some(bytes) = self.storage.get(&name_key(device_id)).await? else {
            return Ok(None);
        };
        let pt = envelope::open(
            &self.keys.blob,
            &self.vault_id,
            Kind::Blob,
            &name_ident(device_id),
            &bytes,
        )?;
        String::from_utf8(pt)
            .map(Some)
            .map_err(|e| SyncError::Format(e.to_string()))
    }

    /// Device ids under `devices/`, with the published name when the file is readable.
    pub async fn list_device_cards(&self) -> Result<Vec<(String, String)>> {
        let all = self.storage.list("devices/").await?;
        let mut ids: Vec<String> = all
            .iter()
            .filter_map(|k| device_from_key(k))
            .map(str::to_string)
            .collect();
        ids.sort();
        ids.dedup();
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            let name = self
                .read_device_name(&id)
                .await
                .unwrap_or(None)
                .unwrap_or_default();
            out.push((id, name));
        }
        Ok(out)
    }

    /// Large files v2 over the same storage and key hierarchy.
    pub fn large_files(&self, config: LargeFileConfig) -> Result<LargeFileStore<'_>> {
        LargeFileStore::new(self.storage.as_ref(), &self.vault_id, &self.keys, config)
    }

    // ── Own log ──────────────────────────────────────────────────────────────

    /// Append one chunk with `ops` to this device's log. Never overwrites: if the
    /// slot is taken by an identical chunk (a previous push whose result was lost)
    /// its hash is adopted, otherwise `OwnLogCollision` is returned.
    pub async fn push(&self, state: &mut LocalState, ops: Vec<Op>) -> Result<()> {
        if ops.is_empty() {
            return Ok(());
        }
        let seq = state.own_seq + 1;
        let chunk = Chunk {
            prev_hash: state.own_head_hash.clone(),
            seq,
            ops,
        };
        let bytes = envelope::seal(
            &self.keys.log,
            &self.vault_id,
            Kind::Chunk,
            &chunk_ident(&self.device_id, seq),
            &serde_json::to_vec(&chunk)?,
        )?;
        let key = chunk_key(&self.device_id, seq);
        let head_hash = if self.storage.put_if_absent(&key, &bytes).await? {
            sha256_hex(&bytes)
        } else {
            let existing = self
                .storage
                .get(&key)
                .await?
                .ok_or(SyncError::OwnLogCollision(seq))?;
            let theirs = self
                .open_chunk(&self.device_id, seq, &existing)
                .map_err(|_| SyncError::OwnLogCollision(seq))?;
            if theirs.prev_hash != chunk.prev_hash
                || serde_json::to_vec(&theirs.ops)? != serde_json::to_vec(&chunk.ops)?
            {
                return Err(SyncError::OwnLogCollision(seq));
            }
            sha256_hex(&existing)
        };
        state.own_seq = seq;
        state.own_head_hash = head_hash;
        Ok(())
    }

    /// Highest seq reachable from the snapshot without a gap or a chain break.
    async fn own_log_tip(&self) -> Result<(u64, String)> {
        let mut seq = 0u64;
        let mut hash = String::new();
        if let Some(snap) = self.read_snapshot(&self.device_id).await? {
            seq = snap.up_to_seq;
            hash = snap.head_hash;
        }
        let prefix = format!("devices/{}/log/", self.device_id);
        let keys = self.storage.list(&prefix).await?;
        let mut seqs: Vec<u64> = keys
            .iter()
            .filter_map(|k| seq_from_key(k))
            .filter(|s| *s > seq)
            .collect();
        seqs.sort_unstable();
        seqs.dedup();
        for s in seqs {
            if s != seq + 1 {
                break;
            }
            let Some(bytes) = self.storage.get(&chunk_key(&self.device_id, s)).await? else {
                break;
            };
            let chunk = match self.open_chunk(&self.device_id, s, &bytes) {
                Ok(c) => c,
                Err(_) => break,
            };
            if chunk.seq != s || chunk.prev_hash != hash {
                break;
            }
            hash = sha256_hex(&bytes);
            seq = s;
        }
        Ok((seq, hash))
    }

    /// Step the local cursor back when it is ahead of the stored chain.
    /// The next push then continues from a head peers can follow.
    pub async fn rewind_own_log(&self, state: &mut LocalState) -> Result<Option<String>> {
        let (seq, hash) = self.own_log_tip().await?;
        if state.own_seq <= seq {
            return Ok(None);
        }
        let from = state.own_seq;
        state.own_seq = seq;
        state.own_head_hash = hash;
        Ok(Some(format!("own log rewound from {from} to {seq}")))
    }

    pub async fn own_chunk_count(&self) -> Result<usize> {
        Ok(self
            .storage
            .list(&format!("devices/{}/log/", self.device_id))
            .await?
            .len())
    }

    /// Fold own log into a snapshot and drop the chunks it covers.
    /// Tombstones older than `tombstone_ttl_ms` (by HLC wall time) are dropped.
    pub async fn compact(
        &self,
        state: &LocalState,
        now_ms: u64,
        tombstone_ttl_ms: u64,
    ) -> Result<()> {
        if state.own_seq == 0 {
            return Ok(());
        }
        let mut ops: Vec<Op> = Vec::new();
        if let Some(snap) = self.read_snapshot(&self.device_id).await? {
            ops.extend(snap.ops);
        }
        let keys = self
            .storage
            .list(&format!("devices/{}/log/", self.device_id))
            .await?;
        let mut seqs: Vec<u64> = keys.iter().filter_map(|k| seq_from_key(k)).collect();
        seqs.sort_unstable();
        for seq in &seqs {
            if *seq > state.own_seq {
                continue;
            }
            let Some(bytes) = self.storage.get(&chunk_key(&self.device_id, *seq)).await? else {
                continue;
            };
            let chunk = self.open_chunk(&self.device_id, *seq, &bytes)?;
            ops.extend(chunk.ops);
        }
        let mut latest = fold_latest(ops);
        latest
            .retain(|op| !(op.deleted && op.hlc.wall_ms.saturating_add(tombstone_ttl_ms) < now_ms));

        let snap = Snapshot {
            up_to_seq: state.own_seq,
            head_hash: state.own_head_hash.clone(),
            ops: latest,
        };
        let bytes = envelope::seal(
            &self.keys.log,
            &self.vault_id,
            Kind::Snapshot,
            &snapshot_ident(&self.device_id),
            &serde_json::to_vec(&snap)?,
        )?;
        self.storage
            .put(&snapshot_key(&self.device_id), &bytes)
            .await?;
        for seq in seqs {
            if seq <= state.own_seq {
                self.storage
                    .delete(&chunk_key(&self.device_id, seq))
                    .await?;
            }
        }
        Ok(())
    }

    // ── Peers ────────────────────────────────────────────────────────────────

    /// Read every peer's log past the remembered head. Verifies the hash chain;
    /// a peer whose chain breaks is reported in `errors` and its head is left as is.
    /// `on_file(current, total, key)` is called before each GET.
    pub async fn pull(
        &self,
        state: &mut LocalState,
        mut on_file: impl FnMut(u32, u32, &str),
    ) -> Result<PullResult> {
        on_file(0, 0, "devices/");
        let mut result = PullResult::default();
        let all = self.storage.list("devices/").await?;
        let peers: BTreeSet<String> = all
            .iter()
            .filter_map(|k| device_from_key(k))
            .filter(|d| *d != self.device_id)
            .map(str::to_string)
            .collect();

        let mut planned: Vec<(String, Vec<String>)> = Vec::new();
        for peer in &peers {
            let head = state.peers.get(peer).cloned().unwrap_or_default();
            planned.push((peer.clone(), planned_gets(peer, &head, &all)));
        }
        let total = planned
            .iter()
            .map(|(_, keys)| keys.len() as u32)
            .sum::<u32>();
        let mut current = 0u32;
        on_file(0, total, "devices/");

        for (peer, keys) in planned {
            let head_seq = state.peers.get(&peer).map(|h| h.seq).unwrap_or(0);
            let read_from = current;
            let mut head = state.peers.get(&peer).cloned().unwrap_or_default();
            match self
                .pull_peer(
                    &peer,
                    &mut head,
                    &all,
                    &keys,
                    &mut current,
                    total,
                    &mut on_file,
                )
                .await
            {
                Ok(ops) => {
                    let n = ops.len();
                    result.peers.push(PullPeer {
                        device_id: peer.clone(),
                        head_seq,
                        files: current - read_from,
                        ops: n,
                        error: None,
                    });
                    result.ops.extend(ops);
                    state.peers.insert(peer, head);
                }
                Err(e) => {
                    let message = e.to_string();
                    result.peers.push(PullPeer {
                        device_id: peer.clone(),
                        head_seq,
                        files: current - read_from,
                        ops: 0,
                        error: Some(message.clone()),
                    });
                    result.errors.push((peer, message));
                }
            }
        }
        result.ops.sort_by(|a, b| a.hlc.cmp(&b.hlc));
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    async fn pull_peer(
        &self,
        peer: &str,
        head: &mut PeerHead,
        all_keys: &[String],
        planned: &[String],
        current: &mut u32,
        total: u32,
        on_file: &mut impl FnMut(u32, u32, &str),
    ) -> Result<Vec<Op>> {
        let log_prefix = format!("devices/{peer}/log/");
        let mut seqs: Vec<u64> = all_keys
            .iter()
            .filter(|k| k.starts_with(&log_prefix))
            .filter_map(|k| seq_from_key(k))
            .collect();
        seqs.sort_unstable();
        let mut ops = Vec::new();
        let start = *current;

        // No chunk continues our head: either nothing new, or the peer compacted
        // and the snapshot now carries what we miss.
        let first_available = seqs.iter().copied().find(|s| *s > head.seq);
        if first_available != Some(head.seq + 1) {
            let snap_key = snapshot_key(peer);
            on_file(*current, total, &snap_key);
            *current += 1;
            if let Some(snap) = self.read_snapshot(peer).await? {
                // Snapshot predating our head carries nothing new; chunks decide.
                if snap.up_to_seq == head.seq
                    && !head.hash.is_empty()
                    && snap.head_hash != head.hash
                {
                    return Err(SyncError::Integrity(
                        "snapshot head does not match known head".into(),
                    ));
                }
                if snap.up_to_seq > head.seq {
                    ops.extend(snap.ops);
                    *head = PeerHead {
                        seq: snap.up_to_seq,
                        hash: snap.head_hash,
                    };
                }
            }
        }

        let mut next = head.seq + 1;
        while seqs.binary_search(&next).is_ok() {
            let key = chunk_key(peer, next);
            on_file(*current, total, &key);
            *current += 1;
            // Listed but not readable yet: the cloud client is still delivering it.
            let Some(bytes) = self.storage.get(&key).await? else {
                break;
            };
            let chunk = match self.open_chunk(peer, next, &bytes) {
                Ok(c) => c,
                Err(SyncError::Format(_)) => break,
                Err(e) => return Err(e),
            };
            if chunk.seq != next {
                return Err(SyncError::Integrity(format!(
                    "chunk {next} carries seq {}",
                    chunk.seq
                )));
            }
            if chunk.prev_hash != head.hash {
                return Err(SyncError::Integrity(format!(
                    "chain break at {peer}/{next}"
                )));
            }
            ops.extend(chunk.ops);
            *head = PeerHead {
                seq: next,
                hash: sha256_hex(&bytes),
            };
            next += 1;
        }
        *current = (*current).max(start + planned.len() as u32);
        Ok(ops)
    }

    /// Authenticated but undecodable content is a real fault, not a partial
    /// download, so it surfaces as `Integrity` instead of a silent `Format` skip.
    fn open_chunk(&self, device: &str, seq: u64, bytes: &[u8]) -> Result<Chunk> {
        let pt = envelope::open(
            &self.keys.log,
            &self.vault_id,
            Kind::Chunk,
            &chunk_ident(device, seq),
            bytes,
        )?;
        serde_json::from_slice(&pt)
            .map_err(|e| SyncError::Integrity(format!("chunk {device}/{seq}: {e}")))
    }

    async fn read_snapshot(&self, device: &str) -> Result<Option<Snapshot>> {
        let Some(bytes) = self.storage.get(&snapshot_key(device)).await? else {
            return Ok(None);
        };
        let pt = envelope::open(
            &self.keys.log,
            &self.vault_id,
            Kind::Snapshot,
            &snapshot_ident(device),
            &bytes,
        )?;
        Ok(Some(serde_json::from_slice(&pt)?))
    }

    /// Latest op per entity across every device's log (snapshot + all chunks).
    /// Fails when a listed chunk cannot be read yet, since its references are unknown.
    pub async fn all_latest_ops(&self) -> Result<Vec<Op>> {
        let all = self.storage.list("devices/").await?;
        let devices: BTreeSet<String> = all
            .iter()
            .filter_map(|k| device_from_key(k))
            .map(str::to_string)
            .collect();
        let mut ops: Vec<Op> = Vec::new();
        for device in devices {
            if let Some(snap) = self.read_snapshot(&device).await? {
                ops.extend(snap.ops);
            }
            let log_prefix = format!("devices/{device}/log/");
            for key in all.iter().filter(|k| k.starts_with(&log_prefix)) {
                let Some(seq) = seq_from_key(key) else {
                    continue;
                };
                let bytes = self.storage.get(key).await?.ok_or_else(|| {
                    SyncError::Storage(format!("chunk {key} listed but not readable"))
                })?;
                ops.extend(self.open_chunk(&device, seq, &bytes)?.ops);
            }
        }
        Ok(fold_latest(ops))
    }

    // ── Blobs ────────────────────────────────────────────────────────────────

    /// Names of every blob in the storage.
    pub async fn list_blobs(&self) -> Result<BTreeSet<String>> {
        let keys = self.storage.list("blobs/").await?;
        Ok(keys
            .iter()
            .filter_map(|k| k.strip_prefix("blobs/"))
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .collect())
    }

    pub async fn delete_blob(&self, name: &str) -> Result<()> {
        self.storage.delete(&blob_key(name)).await
    }

    /// Content-addressed name: keyed HMAC so the storage cannot correlate plaintext hashes.
    pub fn blob_name(&self, data: &[u8]) -> String {
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&self.keys.id)
            .expect("any key length is valid");
        mac.update(data);
        hex::encode(&mac.finalize().into_bytes()[..])
    }

    pub async fn put_blob(&self, data: &[u8]) -> Result<String> {
        let name = self.blob_name(data);
        let bytes = envelope::seal(&self.keys.blob, &self.vault_id, Kind::Blob, &name, data)?;
        self.storage.put(&blob_key(&name), &bytes).await?;
        Ok(name)
    }

    pub async fn get_blob(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let Some(bytes) = self.storage.get(&blob_key(name)).await? else {
            return Ok(None);
        };
        Ok(Some(envelope::open(
            &self.keys.blob,
            &self.vault_id,
            Kind::Blob,
            name,
            &bytes,
        )?))
    }

    /// Presence check without downloading the body.
    pub async fn blob_exists(&self, name: &str) -> Result<bool> {
        self.storage.exists(&blob_key(name)).await
    }
}
