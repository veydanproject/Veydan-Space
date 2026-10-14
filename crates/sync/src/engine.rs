// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Vault engine: create/open, push own ops, pull peers' ops, blobs, compaction.

use crate::envelope::{self, Kind};
use crate::hlc::Hlc;
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
use zeroize::Zeroizing;

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
    /// `error` is of the log's content — a file that does not open or does
    /// not continue the chain ([`is_broken`]), not of the storage. What the
    /// pull read before it is taken and the head stands just before it; no
    /// later read gets past it while the file stays as it is.
    pub broken: bool,
}

/// An error of what a log holds rather than of the storage that serves it:
/// an envelope that does not open (`Format`, `Crypto`) or content that does
/// not fit (`Integrity`). A copy still on its way (a cloud client) shows the
/// same way until it lands.
pub fn is_broken(e: &SyncError) -> bool {
    matches!(
        e,
        SyncError::Integrity(_) | SyncError::Format(_) | SyncError::Crypto(_)
    )
}

/// Ops from peers plus per-peer failures that did not stop the others.
#[derive(Debug, Default)]
pub struct PullResult {
    pub ops: Vec<Op>,
    pub errors: Vec<(String, String)>,
    pub peers: Vec<PullPeer>,
}

/// This device's log as the storage holds it ([`Engine::own_log_status`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnLogStatus {
    /// The highest seq reachable without a gap or a chain break; 0 for none.
    pub seq: u64,
    /// The hash of the chunk at `seq`, or the snapshot's head when it ends there.
    pub head_hash: String,
    /// `prev_hash` of the chunk at `seq`; none when `seq` is the snapshot's
    /// (or 0) and no chunk past it continues the chain.
    pub prev_hash: Option<String>,
    /// The newest HLC of the ops up to `seq`: what this device stamped last
    /// that the storage still holds.
    pub newest: Option<Hlc>,
}

/// What [`Engine::read_every_log`] read.
#[derive(Debug, Default)]
pub struct EveryLog {
    /// The latest op of each entity over the logs, a broken one up to the
    /// file it stops at.
    pub ops: Vec<Op>,
    /// The logs that stop at a file that does not open or does not continue
    /// the chain, and why.
    pub broken: Vec<(String, String)>,
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

/// The key of a passphrase is Argon2id over 64 MiB: seconds of one core on
/// a phone. It runs on the blocking pool, as the lock's does
/// (`crates/lock/src/vault.rs`): on a worker of the async runtime every
/// other task of that worker — a command of the UI among them — would wait
/// for it.
async fn off_runtime<T: Send + 'static>(
    job: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    match tokio::task::spawn_blocking(job).await {
        Ok(done) => done,
        Err(e) => Err(SyncError::Crypto(format!(
            "the key of the passphrase was not derived: {e}"
        ))),
    }
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
        let pass = Zeroizing::new(passphrase.to_owned());
        let (manifest, vmk) = off_runtime(move || Manifest::create(&pass)).await?;
        // Two devices creating in one empty folder at once: the second must
        // not replace the first one's manifest (its key, its vault id).
        let json = manifest.to_json()?;
        if !storage.put_if_absent(MANIFEST_KEY, &json).await? {
            return Err(SyncError::VaultExists);
        }
        Self::read_back_manifest(storage.as_ref(), &json).await?;
        let engine = Self::with_key(storage, &vmk, &manifest.vault_id, device_id);
        Ok((engine, vmk))
    }

    /// The manifest as the storage gives it back is the one written: a write
    /// the storage took but did not keep, or another writer's manifest in its
    /// place, leaves this device on a key nobody else has.
    async fn read_back_manifest(storage: &dyn Storage, written: &[u8]) -> Result<()> {
        match storage.get(MANIFEST_KEY).await? {
            Some(bytes) if bytes == written => Ok(()),
            Some(bytes) => match (Manifest::from_json(&bytes), Manifest::from_json(written)) {
                (Ok(theirs), Ok(ours)) if theirs.vault_id != ours.vault_id => {
                    Err(SyncError::VaultExists)
                }
                _ => Err(SyncError::Storage(
                    "the manifest read back is not the one written".into(),
                )),
            },
            None => Err(SyncError::Storage(
                "the manifest written is not in the storage".into(),
            )),
        }
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
        let pass = Zeroizing::new(passphrase.to_owned());
        let (manifest, vmk) = off_runtime(move || {
            let vmk = manifest.unlock(&pass)?;
            Ok((manifest, vmk))
        })
        .await?;
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
        let (old, new, vmk) = (
            Zeroizing::new(old.to_owned()),
            Zeroizing::new(new.to_owned()),
            vmk.clone(),
        );
        let manifest = off_runtime(move || {
            manifest.unlock(&old)?;
            manifest.rewrap(&vmk, &new)?;
            Ok(manifest)
        })
        .await?;
        let json = manifest.to_json()?;
        self.storage.put(MANIFEST_KEY, &json).await?;
        Self::read_back_manifest(self.storage.as_ref(), &json).await
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

    /// Where this device's log stands in the storage: the highest seq
    /// reachable from the snapshot without a gap or a chain break, with what
    /// the host needs to tell its own saved state from another writer's
    /// (spec 9.3, "the own log: pending and reconciliation").
    ///
    /// `own_seq` and `own_head` are what the device saved. When the listing
    /// ends at that chunk or the one after it, without a gap, the status is
    /// read from those one or two chunks; the snapshot and the whole chain
    /// are read only when it does not (an idle cycle costs a listing and a
    /// GET or two, not the log).
    pub async fn own_log_status(&self, own_seq: u64, own_head: &str) -> Result<OwnLogStatus> {
        let prefix = format!("devices/{}/log/", self.device_id);
        let keys = self.storage.list(&prefix).await?;
        let mut seqs: Vec<u64> = keys.iter().filter_map(|k| seq_from_key(k)).collect();
        seqs.sort_unstable();
        seqs.dedup();
        if let Some(status) = self.own_log_at(&seqs, own_seq, own_head).await? {
            return Ok(status);
        }
        self.own_log_walk(&seqs).await
    }

    /// The status from the saved position alone, when the listing allows it.
    async fn own_log_at(
        &self,
        seqs: &[u64],
        own_seq: u64,
        own_head: &str,
    ) -> Result<Option<OwnLogStatus>> {
        let top = seqs.last().copied().unwrap_or(0);
        let gap = seqs.windows(2).any(|w| w[1] != w[0] + 1);
        if own_seq == 0 || gap || !seqs.contains(&own_seq) || top > own_seq + 1 {
            return Ok(None);
        }
        let Some((chunk, hash)) = self.own_chunk(own_seq).await? else {
            return Ok(None);
        };
        if hash != own_head {
            return Ok(None);
        }
        let mut status = OwnLogStatus {
            seq: own_seq,
            newest: chunk.ops.iter().map(|op| &op.hlc).max().cloned(),
            prev_hash: Some(chunk.prev_hash),
            head_hash: hash,
        };
        if top == own_seq + 1 {
            let Some((next, hash)) = self.own_chunk(top).await? else {
                return Ok(None);
            };
            if next.prev_hash != status.head_hash {
                return Ok(None);
            }
            let newest = next.ops.iter().map(|op| &op.hlc).max().cloned();
            status = OwnLogStatus {
                seq: top,
                head_hash: hash,
                prev_hash: Some(next.prev_hash),
                newest: status.newest.max(newest),
            };
        }
        Ok(Some(status))
    }

    /// Chunk `seq` of this device's log and the hash of its file, when the
    /// storage gives one that opens and carries that seq.
    async fn own_chunk(&self, seq: u64) -> Result<Option<(Chunk, String)>> {
        let Some(bytes) = self.storage.get(&chunk_key(&self.device_id, seq)).await? else {
            return Ok(None);
        };
        Ok(self
            .open_chunk(&self.device_id, seq, &bytes)
            .ok()
            .filter(|chunk| chunk.seq == seq)
            .map(|chunk| (chunk, sha256_hex(&bytes))))
    }

    /// The hash of chunk `seq` of this device's log as the storage gives it
    /// by its key, whatever a listing said; none for 0 or a chunk it lacks.
    pub async fn own_chunk_hash(&self, seq: u64) -> Result<Option<String>> {
        if seq == 0 {
            return Ok(None);
        }
        Ok(self.own_chunk(seq).await?.map(|(_, hash)| hash))
    }

    /// The whole chain from the snapshot.
    async fn own_log_walk(&self, listed: &[u64]) -> Result<OwnLogStatus> {
        let mut status = OwnLogStatus::default();
        if let Some(snap) = self.read_snapshot(&self.device_id).await? {
            status.seq = snap.up_to_seq;
            status.head_hash = snap.head_hash;
            status.newest = snap.ops.iter().map(|op| &op.hlc).max().cloned();
        }
        let from = status.seq;
        for &s in listed.iter().filter(|s| **s > from) {
            if s != status.seq + 1 {
                break;
            }
            let Some((chunk, hash)) = self.own_chunk(s).await? else {
                break;
            };
            if chunk.prev_hash != status.head_hash {
                break;
            }
            let newest = chunk.ops.iter().map(|op| &op.hlc).max().cloned();
            status.newest = status.newest.max(newest);
            status.prev_hash = Some(chunk.prev_hash);
            status.head_hash = hash;
            status.seq = s;
        }
        Ok(status)
    }

    pub async fn own_chunk_count(&self) -> Result<usize> {
        Ok(self
            .storage
            .list(&format!("devices/{}/log/", self.device_id))
            .await?
            .len())
    }

    /// Fold own log into a snapshot and drop the chunks it covers.
    /// A tombstone is kept for ever: a device that was offline longer than any
    /// limit would bring the row back from its old log (audit A2-01). A chunk
    /// past the snapshot that the listing lacks or that does not read fails
    /// it before anything is written or deleted.
    pub async fn compact(&self, state: &LocalState) -> Result<()> {
        if state.own_seq == 0 {
            return Ok(());
        }
        let mut ops: Vec<Op> = Vec::new();
        let mut base = 0;
        if let Some(snap) = self.read_snapshot(&self.device_id).await? {
            base = snap.up_to_seq;
            ops.extend(snap.ops);
        }
        let keys = self
            .storage
            .list(&format!("devices/{}/log/", self.device_id))
            .await?;
        let mut seqs: Vec<u64> = keys.iter().filter_map(|k| seq_from_key(k)).collect();
        seqs.sort_unstable();
        seqs.dedup();
        // Every chunk from the snapshot on to the position is folded, or
        // none is: a snapshot without the ops of a chunk the listing lacks
        // or that does not read would stand for them, and a device that
        // has not read that chunk would never get them (audit A2-22).
        let past: Vec<u64> = seqs
            .iter()
            .copied()
            .filter(|s| *s > base && *s <= state.own_seq)
            .collect();
        if let Some(missing) = (base + 1..=state.own_seq).find(|s| past.binary_search(s).is_err()) {
            return Err(SyncError::Storage(format!(
                "chunk {} is not in the listing: the log is not compacted",
                chunk_key(&self.device_id, missing)
            )));
        }
        for seq in &seqs {
            if *seq > state.own_seq {
                continue;
            }
            let key = chunk_key(&self.device_id, *seq);
            let Some(bytes) = self.storage.get(&key).await? else {
                return Err(SyncError::Storage(format!(
                    "chunk {key} is listed but does not read: the log is not compacted"
                )));
            };
            let chunk = self.open_chunk(&self.device_id, *seq, &bytes)?;
            ops.extend(chunk.ops);
        }
        let latest = fold_latest(ops);

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
    /// a peer whose storage fails is reported in `errors` and its head is left
    /// as is; one whose log stops at a broken file (`PullPeer::broken`) too,
    /// with what it read before that file taken and its head moved there.
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
                // A broken file stops the log there, with an error: what
                // came before it is taken (audit A2-17).
                Ok((ops, broken)) => {
                    let n = ops.len();
                    let error = broken.map(|e| e.to_string());
                    result.peers.push(PullPeer {
                        device_id: peer.clone(),
                        head_seq,
                        files: current - read_from,
                        ops: n,
                        error: error.clone(),
                        broken: error.is_some(),
                    });
                    result.ops.extend(ops);
                    state.peers.insert(peer.clone(), head);
                    if let Some(message) = error {
                        result.errors.push((peer, message));
                    }
                }
                Err(e) => {
                    let message = e.to_string();
                    result.peers.push(PullPeer {
                        device_id: peer.clone(),
                        head_seq,
                        files: current - read_from,
                        ops: 0,
                        error: Some(message.clone()),
                        broken: false,
                    });
                    result.errors.push((peer, message));
                }
            }
        }
        result.ops.sort_by(|a, b| a.hlc.cmp(&b.hlc));
        Ok(result)
    }

    /// The ops of one peer past `head`, and the error of a broken file
    /// ([`is_broken`]) the log stops at: what came before it is kept and
    /// `head` stands just before it. An error of the storage gives nothing.
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
    ) -> Result<(Vec<Op>, Option<SyncError>)> {
        let start = *current;
        let mut ops = Vec::new();
        let read = self
            .read_peer(peer, head, all_keys, &mut ops, current, total, on_file)
            .await;
        *current = (*current).max(start + planned.len() as u32);
        match read {
            Ok(()) => Ok((ops, None)),
            Err(e) if is_broken(&e) => Ok((ops, Some(e))),
            Err(e) => Err(e),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn read_peer(
        &self,
        peer: &str,
        head: &mut PeerHead,
        all_keys: &[String],
        ops: &mut Vec<Op>,
        current: &mut u32,
        total: u32,
        on_file: &mut impl FnMut(u32, u32, &str),
    ) -> Result<()> {
        let log_prefix = format!("devices/{peer}/log/");
        let mut seqs: Vec<u64> = all_keys
            .iter()
            .filter(|k| k.starts_with(&log_prefix))
            .filter_map(|k| seq_from_key(k))
            .collect();
        seqs.sort_unstable();

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
            // A file that is not an envelope (empty, cut short, another
            // file under its name) stops the log with an error, not in
            // silence: until it is mended nothing past it is read (audit
            // A2-17).
            let chunk = self.open_chunk(peer, next, &bytes).map_err(|e| match e {
                SyncError::Format(why) => SyncError::Format(format!("chunk {key}: {why}")),
                e => e,
            })?;
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
        Ok(())
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

    /// Every log read from its start, as a pull of a device that has read
    /// nothing reads it: the snapshot, then the chunks in a chain from it.
    /// For a join, which takes the vault as the truth. A log that is not all
    /// there yet (a listed chunk that does not read, a chunk missing before
    /// a listed one) or a storage that fails is an error: a cloud client may
    /// still be delivering it. A log that stops at a file that does not open
    /// or does not continue its chain is `broken`: what came before that file
    /// is taken, as a pull takes it (`PullPeer::broken`), and nothing past it.
    pub async fn read_every_log(&self) -> Result<EveryLog> {
        let all = self.storage.list("devices/").await?;
        let devices: BTreeSet<String> = all
            .iter()
            .filter_map(|k| device_from_key(k))
            .map(str::to_string)
            .collect();
        let mut out = EveryLog::default();
        let mut ops: Vec<Op> = Vec::new();
        for device in devices {
            match self.read_log(&device, &all, &mut ops).await {
                Ok(()) => {}
                Err(e) if is_broken(&e) => out.broken.push((device, e.to_string())),
                Err(e) => return Err(e),
            }
        }
        out.ops = fold_latest(ops);
        Ok(out)
    }

    /// The ops of one log from its start into `ops`, for
    /// [`Engine::read_every_log`]; on an error, those before it.
    async fn read_log(&self, device: &str, all: &[String], ops: &mut Vec<Op>) -> Result<()> {
        let log_prefix = format!("devices/{device}/log/");
        let mut seqs: Vec<u64> = all
            .iter()
            .filter(|k| k.starts_with(&log_prefix))
            .filter_map(|k| seq_from_key(k))
            .collect();
        seqs.sort_unstable();
        seqs.dedup();
        let mut head = PeerHead::default();
        if let Some(snap) = self.read_snapshot(device).await? {
            ops.extend(snap.ops);
            head = PeerHead {
                seq: snap.up_to_seq,
                hash: snap.head_hash,
            };
        }
        let from = head.seq;
        for seq in seqs.into_iter().filter(|s| *s > from) {
            if seq != head.seq + 1 {
                return Err(SyncError::Storage(format!(
                    "chunk {} of {device} is not there yet",
                    head.seq + 1
                )));
            }
            let key = chunk_key(device, seq);
            let bytes = self.storage.get(&key).await?.ok_or_else(|| {
                SyncError::Storage(format!("chunk {key} listed but not readable"))
            })?;
            let chunk = self.open_chunk(device, seq, &bytes)?;
            if chunk.seq != seq {
                return Err(SyncError::Integrity(format!(
                    "chunk {seq} carries seq {}",
                    chunk.seq
                )));
            }
            if chunk.prev_hash != head.hash {
                return Err(SyncError::Integrity(format!(
                    "chain break at {device}/{seq}"
                )));
            }
            ops.extend(chunk.ops);
            head = PeerHead {
                seq,
                hash: sha256_hex(&bytes),
            };
        }
        Ok(())
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
