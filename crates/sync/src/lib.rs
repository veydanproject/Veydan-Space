// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Sync core: encrypted per-device append-only logs on top of a storage
//! that only needs `list / get / put / delete`.
//!
//! Vault layout (all paths relative to the vault root):
//!
//! ```text
//! manifest.json                     plaintext: vault_id, KDF params, wrapped master key
//! devices/<device_id>/log/NNNNNNNN.bin   encrypted chunk of ops, chained by prev_hash
//! devices/<device_id>/snapshot.bin       encrypted compaction of the owner's log
//! blobs/<name>                           encrypted content-addressed payloads
//! large-files/v2/chunks/<chunk_id>       encrypted chunk of a large file (shared, immutable)
//! large-files/v2/manifests/<manifest_id> encrypted chunk list of one large file
//! ```
//!
//! Only the owning device writes under `devices/<device_id>/`, so two devices
//! never write the same file. Conflict resolution is the caller's job; this
//! crate delivers ops in HLC order and guarantees integrity of each log.

pub mod engine;
pub mod envelope;
pub mod hlc;
pub mod keys;
pub mod large_files;
pub mod log;
pub mod storage;
pub mod storage_s3;
pub mod storage_webdav;

pub use engine::{Engine, Probe, PullResult};
pub use hlc::{Hlc, HlcClock};
pub use keys::{Manifest, Vmk};
pub use large_files::{
    available_space, refs_from_payload, CancelFlag, FileSource, GcOutcome, LargeFileConfig,
    LargeFileMetadata, LargeFileRef, LargeFileSink, LargeFileSource, LargeFileStore, Opener,
    PathSink, PathSource, Phase, Progress, LF_REFS_FIELD,
};
pub use log::{device_from_key, LocalState, Op, PeerHead};
pub use storage::{LocalDir, Storage};
pub use storage_s3::{S3Config, S3Storage};
pub use storage_webdav::{WebDavConfig, WebDavStorage};

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("storage: {0}")]
    Storage(String),
    /// Server rejected the credentials; never retried.
    #[error("auth: {0}")]
    Auth(String),
    /// Local source could not be read (path, content URI, generator).
    #[error("source: {0}")]
    Source(String),
    /// Local destination has no room or cannot be written.
    #[error("disk: {0}")]
    Disk(String),
    #[error("cancelled")]
    Cancelled,
    #[error("crypto: {0}")]
    Crypto(String),
    #[error("wrong passphrase")]
    WrongPassphrase,
    #[error("no vault in this storage")]
    NoVault,
    #[error("vault already exists")]
    VaultExists,
    #[error("storage is not empty and has no vault")]
    ForeignStorage,
    #[error("vault id mismatch")]
    VaultMismatch,
    #[error("format: {0}")]
    Format(String),
    #[error("integrity: {0}")]
    Integrity(String),
    /// Another writer already holds the next position in our own log:
    /// the caller must start a fresh device log.
    #[error("own log collision at seq {0}")]
    OwnLogCollision(u64),
}

pub type Result<T> = std::result::Result<T, SyncError>;

impl From<serde_json::Error> for SyncError {
    fn from(e: serde_json::Error) -> Self {
        SyncError::Format(e.to_string())
    }
}

impl From<std::io::Error> for SyncError {
    fn from(e: std::io::Error) -> Self {
        SyncError::Storage(e.to_string())
    }
}

fn reqwest_msg(e: &reqwest::Error) -> String {
    let mut msg = e.to_string();
    let mut src = std::error::Error::source(e);
    while let Some(err) = src {
        let next = err.to_string();
        if !msg.contains(&next) {
            msg.push_str(": ");
            msg.push_str(&next);
        }
        src = err.source();
    }
    msg
}

impl From<reqwest::Error> for SyncError {
    fn from(e: reqwest::Error) -> Self {
        SyncError::Storage(reqwest_msg(&e))
    }
}

/// Hex SHA-256 of arbitrary bytes.
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let out = Sha256::digest(data);
    hex::encode(&out[..])
}

/// Hex SHA-256 of a stream; constant memory regardless of length.
pub fn sha256_reader_hex(reader: &mut dyn std::io::Read) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(&hasher.finalize()[..]))
}

/// Random bytes from the OS CSPRNG.
pub fn random_bytes(n: usize) -> Vec<u8> {
    use rand::Rng;
    let mut buf = vec![0u8; n];
    rand::rng().fill_bytes(&mut buf);
    buf
}

/// `n` random bytes as lowercase hex.
pub fn random_hex(n: usize) -> String {
    hex::encode(random_bytes(n))
}
