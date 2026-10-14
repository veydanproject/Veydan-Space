// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! File hashes with a per-process cache: a file is re-read only when its
//! mtime or size changed. Shared by attachments and profile files.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;
use veydan_sync::sha256_reader_hex;

/// Streaming SHA-256 so multi-gigabyte files never sit in memory.
fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut reader = std::io::BufReader::with_capacity(1 << 20, std::fs::File::open(path)?);
    sha256_reader_hex(&mut reader)
}

#[derive(Clone)]
struct FileStamp {
    mtime: Option<SystemTime>,
    size: u64,
    hash: String,
}

#[derive(Default)]
pub struct HashCache(Mutex<HashMap<PathBuf, FileStamp>>);

impl HashCache {
    /// SHA-256 of the file, or `None` when it cannot be read.
    pub fn file_hash(&self, path: &Path) -> Option<String> {
        self.try_file_hash(path).ok()
    }

    /// SHA-256 of the file, or why it cannot be read: a caller that must
    /// tell a file gone (`NotFound`) from one it may not read uses this.
    pub fn try_file_hash(&self, path: &Path) -> std::io::Result<String> {
        let meta = std::fs::metadata(path)?;
        let (mtime, size) = (meta.modified().ok(), meta.len());
        let mut cache = self
            .0
            .lock()
            .map_err(|_| std::io::Error::other("hash cache poisoned"))?;
        if let Some(s) = cache.get(path) {
            if s.mtime == mtime && s.size == size {
                return Ok(s.hash.clone());
            }
        }
        let hash = hash_file(path)?;
        cache.insert(
            path.to_path_buf(),
            FileStamp {
                mtime,
                size,
                hash: hash.clone(),
            },
        );
        Ok(hash)
    }
}
