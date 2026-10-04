// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Manifest v2: plaintext schema and canonical bytes.
//!
//! Serialized with serde_json in declared field order; `manifest_id` is the
//! HMAC of exactly these bytes, so the order below is part of the protocol.

use crate::{Result, SyncError};
use serde::{Deserialize, Serialize};

pub const FORMAT_VERSION: u32 = 2;
/// Chunk payload codec; the envelope applies it per chunk.
pub const CODEC_ZSTD: &str = "zstd";
const ID_HEX_LEN: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkEntry {
    pub id: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestV2 {
    pub version: u32,
    pub size: u64,
    pub file_id: String,
    pub chunk_size: u64,
    pub codec: String,
    pub chunks: Vec<ChunkEntry>,
}

fn is_id(s: &str) -> bool {
    s.len() == ID_HEX_LEN && s.bytes().all(|b| b.is_ascii_hexdigit())
}

impl ManifestV2 {
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(self)?)
    }

    /// Parse and check structural invariants; ids are verified by the caller.
    pub fn parse(bytes: &[u8]) -> Result<ManifestV2> {
        let m: ManifestV2 = serde_json::from_slice(bytes)?;
        m.validate()?;
        Ok(m)
    }

    pub fn validate(&self) -> Result<()> {
        let bad = |msg: &str| Err(SyncError::Format(format!("manifest v2: {msg}")));
        if self.version != FORMAT_VERSION {
            return bad(&format!("unsupported version {}", self.version));
        }
        if self.codec != CODEC_ZSTD {
            return bad(&format!("unsupported codec {}", self.codec));
        }
        if self.chunk_size == 0 || !is_id(&self.file_id) {
            return bad("bad chunk size or file id");
        }
        let mut total = 0u64;
        for (i, c) in self.chunks.iter().enumerate() {
            if !is_id(&c.id) {
                return bad("bad chunk id");
            }
            let last = i + 1 == self.chunks.len();
            if c.size == 0 || c.size > self.chunk_size || (!last && c.size != self.chunk_size) {
                return bad("chunk size does not match layout");
            }
            total = total
                .checked_add(c.size)
                .ok_or_else(|| SyncError::Format("manifest v2: size overflow".into()))?;
        }
        if total != self.size {
            return bad("chunk sizes do not sum to file size");
        }
        Ok(())
    }
}
