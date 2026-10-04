// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Encrypted file format shared by chunks, snapshots and blobs.
//!
//! ```text
//! "VSY1" | kind u8 | nonce 24 B | XChaCha20-Poly1305( zstd(plaintext) )
//! AAD = "veydan-sync/v1|<vault_id>|<kind>|<ident>"
//! ```
//!
//! `ident` is the logical address (device/seq, device/snapshot, blob name), so
//! a ciphertext moved to another address fails to open.

use crate::{random_bytes, Result, SyncError};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

const MAGIC: &[u8; 4] = b"VSY1";
const NONCE_LEN: usize = 24;
const ZSTD_LEVEL: i32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Chunk = 1,
    Snapshot = 2,
    Blob = 3,
    /// Large files v2 chunk; ident = chunk id.
    LfChunk = 4,
    /// Large files v2 manifest; ident = manifest id.
    LfManifest = 5,
}

impl Kind {
    fn from_u8(v: u8) -> Option<Kind> {
        match v {
            1 => Some(Kind::Chunk),
            2 => Some(Kind::Snapshot),
            3 => Some(Kind::Blob),
            4 => Some(Kind::LfChunk),
            5 => Some(Kind::LfManifest),
            _ => None,
        }
    }
}

fn aad(vault_id: &str, kind: Kind, ident: &str) -> Vec<u8> {
    format!("veydan-sync/v1|{vault_id}|{}|{ident}", kind as u8).into_bytes()
}

pub fn seal(
    key: &[u8; 32],
    vault_id: &str,
    kind: Kind,
    ident: &str,
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    let compressed = zstd::encode_all(plaintext, ZSTD_LEVEL)?;
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce = random_bytes(NONCE_LEN);
    let ct = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &compressed,
                aad: &aad(vault_id, kind, ident),
            },
        )
        .map_err(|_| SyncError::Crypto("seal failed".into()))?;
    let mut out = Vec::with_capacity(4 + 1 + NONCE_LEN + ct.len());
    out.extend_from_slice(MAGIC);
    out.push(kind as u8);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn open(
    key: &[u8; 32],
    vault_id: &str,
    kind: Kind,
    ident: &str,
    data: &[u8],
) -> Result<Vec<u8>> {
    if data.len() < 4 + 1 + NONCE_LEN || &data[..4] != MAGIC {
        return Err(SyncError::Format("not a sync envelope".into()));
    }
    let file_kind =
        Kind::from_u8(data[4]).ok_or_else(|| SyncError::Format("unknown envelope kind".into()))?;
    if file_kind != kind {
        return Err(SyncError::Integrity(format!(
            "expected {:?}, found {:?}",
            kind, file_kind
        )));
    }
    let nonce = &data[5..5 + NONCE_LEN];
    let ct = &data[5 + NONCE_LEN..];
    let cipher = XChaCha20Poly1305::new(key.into());
    let compressed = cipher
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ct,
                aad: &aad(vault_id, kind, ident),
            },
        )
        .map_err(|_| SyncError::Integrity(format!("cannot open {ident}")))?;
    Ok(zstd::decode_all(&compressed[..])?)
}
