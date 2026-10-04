// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `FileSecretStore`: secrets as a JSON map on disk. **Plaintext, for the
//! CLI and standalone experiments only.** A real host wraps secrets with
//! its own key (Veydan Space uses the vault).

use async_trait::async_trait;
use messenger_core::{MessengerError, Result, SecretStore};
use std::collections::BTreeMap;
use std::path::PathBuf;
use tokio::sync::Mutex;
use zeroize::Zeroizing;

pub struct FileSecretStore {
    path: PathBuf,
    map: Mutex<BTreeMap<String, String>>,
}

impl FileSecretStore {
    pub async fn open(path: PathBuf) -> Result<Self> {
        let map = match tokio::fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| MessengerError::Storage(format!("secrets file: {e}")))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self { path, map: Mutex::new(map) })
    }

    async fn save(&self, map: &BTreeMap<String, String>) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let bytes = serde_json::to_vec_pretty(map)?;
        tokio::fs::write(&self.path, bytes).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}

#[async_trait]
impl SecretStore for FileSecretStore {
    async fn get(&self, key: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        let map = self.map.lock().await;
        let Some(hex) = map.get(key) else { return Ok(None) };
        let bytes = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
            .collect::<std::result::Result<Vec<u8>, _>>()
            .map_err(|_| MessengerError::Storage("secrets file is corrupt".into()))?;
        Ok(Some(Zeroizing::new(bytes)))
    }

    async fn put(&self, key: &str, value: &[u8]) -> Result<()> {
        let mut map = self.map.lock().await;
        let hex: String = value.iter().map(|b| format!("{b:02x}")).collect();
        map.insert(key.to_string(), hex);
        self.save(&map).await
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let mut map = self.map.lock().await;
        map.remove(key);
        self.save(&map).await
    }

    async fn is_unlocked(&self) -> bool {
        true
    }
}
