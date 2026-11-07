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
//!
//! An envelope opens to at most a bound the caller names ([`open_limited`];
//! [`MAX_PLAINTEXT`] for [`open`]): a frame that says it is bigger is refused
//! before anything is allocated for it, one that does not say is read up to
//! the bound and refused past it. A sealed zstd bomb from a device that holds
//! the key costs no more memory than the bound.

use crate::{random_bytes, Result, SyncError};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use std::io::Read;

const MAGIC: &[u8; 4] = b"VSY1";
const NONCE_LEN: usize = 24;
const ZSTD_LEVEL: i32 = 3;
/// What [`open`] gives at most: a log chunk, a snapshot, a blob.
pub const MAX_PLAINTEXT: usize = 256 * 1024 * 1024;

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
    seal_compressed(key, vault_id, kind, ident, &zstd::encode_all(plaintext, ZSTD_LEVEL)?)
}

fn seal_compressed(
    key: &[u8; 32],
    vault_id: &str,
    kind: Kind,
    ident: &str,
    compressed: &[u8],
) -> Result<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce = random_bytes(NONCE_LEN);
    let ct = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: compressed,
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
    open_limited(key, vault_id, kind, ident, data, MAX_PLAINTEXT)
}

/// [`open`], refusing a plaintext of more than `max` bytes.
pub fn open_limited(
    key: &[u8; 32],
    vault_id: &str,
    kind: Kind,
    ident: &str,
    data: &[u8],
    max: usize,
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
    decompress(&compressed, max).map_err(|e| match e {
        SyncError::Integrity(m) => SyncError::Integrity(format!("{ident}: {m}")),
        e => e,
    })
}

fn too_big(max: usize) -> SyncError {
    SyncError::Integrity(format!("the plaintext is over {max} bytes"))
}

fn decompress(compressed: &[u8], max: usize) -> Result<Vec<u8>> {
    if let Ok(Some(size)) = zstd::zstd_safe::get_frame_content_size(compressed) {
        if size > max as u64 {
            return Err(too_big(max));
        }
    }
    let mut out = Vec::new();
    zstd::stream::read::Decoder::with_buffer(compressed)?
        .take(max as u64 + 1)
        .read_to_end(&mut out)?;
    if out.len() > max {
        return Err(too_big(max));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7; 32];

    #[test]
    fn a_frame_that_says_it_is_too_big_is_refused_before_it_is_read() {
        // A frame header that claims a terabyte (descriptor: an 8-byte
        // content size, a window byte), and a body never decoded.
        let mut frame = vec![0x28, 0xB5, 0x2F, 0xFD, 0xC0, 0x58];
        frame.extend_from_slice(&(1u64 << 40).to_le_bytes());
        frame.extend_from_slice(&[0u8; 16]);
        assert_eq!(zstd::zstd_safe::get_frame_content_size(&frame).unwrap(), Some(1 << 40));
        let sealed = seal_compressed(&KEY, "v", Kind::Chunk, "a/1", &frame).unwrap();
        let e = open(&KEY, "v", Kind::Chunk, "a/1", &sealed).unwrap_err();
        assert!(matches!(e, SyncError::Integrity(ref m) if m.contains("over")), "{e:?}");
    }

    #[test]
    fn a_frame_that_does_not_say_is_read_up_to_the_bound() {
        // The streaming encoder leaves the size out of the header.
        let bomb = zstd::encode_all(&vec![0u8; 4 << 20][..], ZSTD_LEVEL).unwrap();
        assert!(bomb.len() < 4096, "{} bytes", bomb.len());
        assert_eq!(zstd::zstd_safe::get_frame_content_size(&bomb).unwrap(), None);
        let sealed = seal_compressed(&KEY, "v", Kind::Blob, "b", &bomb).unwrap();
        let e = open_limited(&KEY, "v", Kind::Blob, "b", &sealed, 1 << 20).unwrap_err();
        assert!(matches!(e, SyncError::Integrity(ref m) if m.contains("over")), "{e:?}");
        assert_eq!(open_limited(&KEY, "v", Kind::Blob, "b", &sealed, 4 << 20).unwrap().len(), 4 << 20);
    }
}
