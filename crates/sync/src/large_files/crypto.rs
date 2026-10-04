// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Ids and envelopes for large files.
//!
//! - `chunk_id    = HMAC-SHA256(lf_id, plaintext_chunk)`
//! - `file_id     = HMAC-SHA256(lf_id, chunk_id_1 || chunk_id_2 || ...)` over the hex ids
//! - `manifest_id = HMAC-SHA256(lf_id, canonical manifest bytes)`
//!
//! `lf_id` is one key per vault, so equal chunks share one object. Chunk and
//! manifest ciphertexts use separate keys and envelope kinds; the AAD binds
//! vault id, kind and object id.

use super::manifest::ManifestV2;
use crate::envelope::{self, Kind};
use crate::keys::Keys;
use crate::{Result, SyncError};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

pub const CHUNKS_PREFIX: &str = "large-files/v2/chunks/";
pub const MANIFESTS_PREFIX: &str = "large-files/v2/manifests/";

pub fn chunk_key(id: &str) -> String {
    format!("{CHUNKS_PREFIX}{id}")
}

pub fn manifest_key(id: &str) -> String {
    format!("{MANIFESTS_PREFIX}{id}")
}

fn hmac_hex(key: &[u8; 32], parts: &[&[u8]]) -> String {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("any key length is valid");
    for p in parts {
        mac.update(p);
    }
    hex::encode(&mac.finalize().into_bytes()[..])
}

pub fn chunk_id(keys: &Keys, plain: &[u8]) -> String {
    hmac_hex(&keys.lf_id, &[plain])
}

pub fn file_id(keys: &Keys, chunk_ids: &[String]) -> String {
    let parts: Vec<&[u8]> = chunk_ids.iter().map(|s| s.as_bytes()).collect();
    hmac_hex(&keys.lf_id, &parts)
}

pub fn manifest_id(keys: &Keys, canonical: &[u8]) -> String {
    hmac_hex(&keys.lf_id, &[canonical])
}

pub fn seal_chunk(keys: &Keys, vault_id: &str, id: &str, plain: &[u8]) -> Result<Vec<u8>> {
    envelope::seal(&keys.lf_chunk, vault_id, Kind::LfChunk, id, plain)
}

/// Decrypt and verify size and content id; a moved or altered object fails here.
pub fn open_chunk(
    keys: &Keys,
    vault_id: &str,
    id: &str,
    expected_size: u64,
    data: &[u8],
) -> Result<Vec<u8>> {
    let plain = envelope::open(&keys.lf_chunk, vault_id, Kind::LfChunk, id, data)?;
    if plain.len() as u64 != expected_size {
        return Err(SyncError::Integrity(format!(
            "chunk {id}: size {} != {expected_size}",
            plain.len()
        )));
    }
    if chunk_id(keys, &plain) != id {
        return Err(SyncError::Integrity(format!(
            "chunk {id}: content id mismatch"
        )));
    }
    Ok(plain)
}

/// Canonical bytes, their id and the ciphertext ready to store.
pub fn seal_manifest(
    keys: &Keys,
    vault_id: &str,
    manifest: &ManifestV2,
) -> Result<(String, Vec<u8>)> {
    let canonical = manifest.canonical_bytes()?;
    let id = manifest_id(keys, &canonical);
    let sealed = envelope::seal(
        &keys.lf_manifest,
        vault_id,
        Kind::LfManifest,
        &id,
        &canonical,
    )?;
    Ok((id, sealed))
}

/// Decrypt, re-derive the manifest id and check the chunk list against `file_id`.
pub fn open_manifest(keys: &Keys, vault_id: &str, id: &str, data: &[u8]) -> Result<ManifestV2> {
    let canonical = envelope::open(&keys.lf_manifest, vault_id, Kind::LfManifest, id, data)?;
    if manifest_id(keys, &canonical) != id {
        return Err(SyncError::Integrity(format!("manifest {id}: id mismatch")));
    }
    let manifest = ManifestV2::parse(&canonical)?;
    let ids: Vec<String> = manifest.chunks.iter().map(|c| c.id.clone()).collect();
    if file_id(keys, &ids) != manifest.file_id {
        return Err(SyncError::Integrity(format!(
            "manifest {id}: file id mismatch"
        )));
    }
    Ok(manifest)
}
