// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Relay configuration glue: manifest → `msg_relays` → live `RelayPool`.
//!
//! The transport crate knows how to diff a manifest and how to talk to
//! relays; the store knows how to persist rows. This service sequences the
//! two and owns the pool instance, rebuilding it when the signer changes.

use messenger_core::traits::RelayState;
use messenger_core::{MessengerError, RelayUrl, Result, Transport};
use messenger_store::{relays as repo, settings, Store};
use messenger_transport::manifest::{diff_relays, StoredRelay, REGION_DEFAULT};
use messenger_transport::{Manifest, RelayChanges, RelayConfig, RelayPool, EMBEDDED_MANIFEST_JSON};
use nostr::key::Keys;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

const KEY_SERIAL: &str = "manifest.serial";
const KEY_ISSUED_AT: &str = "manifest.issued_at";
const KEY_REGION: &str = "manifest.region";
const KEY_SILENT: &str = "relays.silent_mode";
pub(crate) const KEY_MODE: &str = "servers.mode";
const KEY_JSON: &str = "manifest.json";
const KEY_ORIGIN: &str = "manifest.origin";
const KEY_CHECKED_AT: &str = "manifest.checked_at";

/// Origin of a manifest that came with the app.
pub const ORIGIN_EMBEDDED: &str = "embedded";

/// Whose servers the messenger uses. Until the user picks one, the
/// messenger talks to nobody.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServersMode {
    /// The project's servers, from its signed manifest.
    Veydan,
    /// Only servers the user added; nothing goes to the project.
    Own,
}

impl ServersMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Veydan => "veydan",
            Self::Own => "own",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "veydan" => Some(Self::Veydan),
            "own" => Some(Self::Own),
            _ => None,
        }
    }
}

/// What a manifest check found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestCheck {
    /// The manifest in use after the check: its URL, or `embedded`.
    pub origin: String,
    pub serial: u64,
    /// A newer manifest was applied by this check.
    pub updated: bool,
    /// Why the project's manifest could not be fetched, if it could not.
    pub error: Option<String>,
}

/// One relay as the UI sees it: stored configuration plus live state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RelayView {
    pub url: String,
    pub relay_id: Option<String>,
    pub source: String,
    pub regions: Vec<String>,
    pub read: bool,
    pub write: bool,
    pub enabled: bool,
    /// `nip42` | `api_key` | none. The key itself is never exposed.
    pub auth_type: Option<String>,
    pub state: RelayState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManifestInfo {
    pub serial: Option<u64>,
    pub issued_at: Option<i64>,
    pub region: String,
    /// Regions the embedded manifest knows about (for the region picker).
    pub regions: Vec<String>,
    pub silent_mode: bool,
    pub mode: Option<ServersMode>,
    /// Where the manifest in use came from: a URL, or `embedded`.
    pub origin: String,
    /// When the project's manifest was last asked for (unix seconds).
    pub checked_at: Option<i64>,
}

pub struct RelayService {
    store: Store,
    pool: RwLock<Arc<RelayPool>>,
}

impl RelayService {
    /// Build the service and the first pool. With the Veydan servers chosen,
    /// a manifest newer than the applied one (an app update) is applied.
    /// Loads rows and connects (unless silent, or no servers chosen yet).
    pub async fn init(store: Store, signer: Option<Keys>) -> Result<Self> {
        let svc = Self { store, pool: RwLock::new(Arc::new(RelayPool::new(signer))) };
        if svc.servers_mode().await? == Some(ServersMode::Veydan) {
            let (current, origin) = svc.current_manifest().await?;
            if current.passes_anti_rollback(svc.manifest_serial().await?) {
                svc.adopt_manifest(&current, &origin, false).await?;
            }
        }
        if svc.is_silent().await? {
            svc.pool.read().await.set_silent(true).await;
        }
        svc.reload().await?;
        Ok(svc)
    }

    pub async fn pool(&self) -> Arc<RelayPool> {
        self.pool.read().await.clone()
    }

    /// Swap the signer: the pool is rebuilt because nostr-sdk binds the
    /// authenticator at construction. Subscriptions are the caller's to redo.
    pub async fn set_signer(&self, signer: Option<Keys>) -> Result<()> {
        let silent = self.is_silent().await?;
        let fresh = Arc::new(RelayPool::new(signer));
        if silent {
            fresh.set_silent(true).await;
        }
        let old = {
            let mut guard = self.pool.write().await;
            std::mem::replace(&mut *guard, fresh)
        };
        old.shutdown().await;
        self.reload().await
    }

    // ─── Manifest ───────────────────────────────────────────────────────────

    pub async fn manifest_serial(&self) -> Result<Option<u64>> {
        Ok(settings::get(&self.store, KEY_SERIAL).await?.and_then(|s| s.parse().ok()))
    }

    pub async fn region(&self) -> Result<String> {
        Ok(settings::get(&self.store, KEY_REGION).await?.unwrap_or_else(|| REGION_DEFAULT.to_string()))
    }

    pub async fn manifest_info(&self) -> Result<ManifestInfo> {
        let (current, origin) = self.current_manifest().await?;
        Ok(ManifestInfo {
            serial: self.manifest_serial().await?,
            issued_at: settings::get(&self.store, KEY_ISSUED_AT).await?.and_then(|s| s.parse().ok()),
            region: self.region().await?,
            regions: current.regions,
            silent_mode: self.is_silent().await?,
            mode: self.servers_mode().await?,
            origin,
            checked_at: self.manifest_checked_at().await?,
        })
    }

    pub async fn servers_mode(&self) -> Result<Option<ServersMode>> {
        Ok(settings::get(&self.store, KEY_MODE).await?.as_deref().and_then(ServersMode::parse))
    }

    /// Record the choice. Applying what it means (manifest rows in or out)
    /// is the caller's: see `MessengerRuntime::servers_use_*`.
    pub async fn set_servers_mode(&self, mode: ServersMode) -> Result<()> {
        settings::set(&self.store, KEY_MODE, mode.as_str()).await
    }

    /// The best manifest this install knows and where it came from: the
    /// stored one, unless the app brought a newer one.
    pub async fn current_manifest(&self) -> Result<(Manifest, String)> {
        let embedded = Manifest::parse_content(EMBEDDED_MANIFEST_JSON)?;
        // Stored only after its signature was checked, so the bare form is fine.
        let stored = settings::get(&self.store, KEY_JSON).await?.and_then(|json| Manifest::parse_content(&json).ok());
        match stored {
            Some(m) if m.serial >= embedded.serial => {
                let origin = settings::get(&self.store, KEY_ORIGIN).await?.unwrap_or_else(|| ORIGIN_EMBEDDED.into());
                Ok((m, origin))
            }
            _ => Ok((embedded, ORIGIN_EMBEDDED.into())),
        }
    }

    /// Apply `m` and remember it as the manifest in use.
    pub async fn adopt_manifest(&self, m: &Manifest, origin: &str, force: bool) -> Result<RelayChanges> {
        let changes = self.apply_manifest(m, force).await?;
        settings::set(&self.store, KEY_JSON, &serde_json::to_string(m)?).await?;
        settings::set(&self.store, KEY_ORIGIN, origin).await?;
        Ok(changes)
    }

    pub async fn manifest_checked_at(&self) -> Result<Option<i64>> {
        Ok(settings::get(&self.store, KEY_CHECKED_AT).await?.and_then(|s| s.parse().ok()))
    }

    pub async fn set_manifest_checked_at(&self, at: i64) -> Result<()> {
        settings::set(&self.store, KEY_CHECKED_AT, &at.to_string()).await
    }

    /// Own servers: the rows a manifest brought go away, the user's stay.
    pub async fn drop_manifest_relays(&self) -> Result<()> {
        for r in repo::list(&self.store).await? {
            if r.source == repo::SOURCE_MANIFEST {
                repo::delete(&self.store, &r.url).await?;
            }
        }
        self.reload().await
    }

    /// Apply `m` for the current region. Refused when the serial does not
    /// advance, unless `force` (region switch re-applies the same manifest).
    pub async fn apply_manifest(&self, m: &Manifest, force: bool) -> Result<RelayChanges> {
        if !force && !m.passes_anti_rollback(self.manifest_serial().await?) {
            return Err(MessengerError::Invalid(format!(
                "manifest serial {} does not advance past the applied one",
                m.serial
            )));
        }
        let region = self.region().await?;
        let target = m.relays_for_region(&region);
        let stored: Vec<StoredRelay> = repo::list(&self.store)
            .await?
            .into_iter()
            .filter(|r| r.source == repo::SOURCE_MANIFEST)
            .filter_map(|r| Some(StoredRelay { id: r.relay_id?, url: RelayUrl::parse(&r.url)? }))
            .collect();
        let changes = diff_relays(&stored, &target);

        for (_, old, new) in &changes.renamed {
            repo::migrate_url(&self.store, old.as_str(), new.as_str()).await?;
        }
        for url in &changes.removed {
            repo::delete(&self.store, url.as_str()).await?;
        }
        for r in changes.added.iter().chain(changes.kept.iter()).chain(target_renamed(&changes, &target)) {
            repo::upsert(
                &self.store,
                &repo::RelayUpsert {
                    url: RelayUrl::parse(&r.url).map(|u| u.as_str().to_string()).unwrap_or_else(|| r.url.clone()),
                    relay_id: Some(r.id.clone()),
                    source: repo::SOURCE_MANIFEST.into(),
                    regions: r.regions.clone(),
                    read: r.read,
                    write: r.write,
                    enabled: true,
                    auth_type: r.auth.as_ref().map(|a| a.type_name().to_string()),
                    auth_secret: r.auth.as_ref().and_then(|a| a.secret().map(String::from)),
                },
                false,
            )
            .await?;
        }
        settings::set(&self.store, KEY_SERIAL, &m.serial.to_string()).await?;
        settings::set(&self.store, KEY_ISSUED_AT, &m.issued_at.to_string()).await?;
        self.reload().await?;
        Ok(changes)
    }

    /// Change region and re-apply the manifest in use for it.
    pub async fn set_region(&self, region: &str) -> Result<()> {
        let region = region.trim();
        if region.is_empty() || !region.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '*') {
            return Err(MessengerError::Invalid("region must be alphanumeric".into()));
        }
        settings::set(&self.store, KEY_REGION, region).await?;
        if self.servers_mode().await? == Some(ServersMode::Veydan) {
            let (m, origin) = self.current_manifest().await?;
            self.adopt_manifest(&m, &origin, true).await?;
        }
        Ok(())
    }

    // ─── Relay rows ─────────────────────────────────────────────────────────

    pub async fn list(&self) -> Result<Vec<RelayView>> {
        let rows = repo::list(&self.store).await?;
        let status = self.pool.read().await.status().await;
        Ok(rows
            .into_iter()
            .map(|r| {
                let state = status
                    .relays
                    .iter()
                    .find(|s| s.url.as_str() == r.url)
                    .map(|s| s.state)
                    .unwrap_or(RelayState::Disconnected);
                RelayView {
                    regions: r.regions(),
                    url: r.url,
                    relay_id: r.relay_id,
                    source: r.source,
                    read: r.read,
                    write: r.write,
                    enabled: r.enabled,
                    auth_type: r.auth_type,
                    state,
                }
            })
            .collect())
    }

    pub async fn add_user(&self, url: &str, api_key: Option<String>) -> Result<RelayView> {
        let parsed = RelayUrl::parse(url).ok_or_else(|| MessengerError::Invalid("relay url must be ws:// or wss:// with a host".into()))?;
        if repo::get(&self.store, parsed.as_str()).await?.is_some() {
            return Err(MessengerError::Invalid("this relay is already configured".into()));
        }
        repo::upsert(
            &self.store,
            &repo::RelayUpsert {
                url: parsed.as_str().to_string(),
                relay_id: None,
                source: repo::SOURCE_USER.into(),
                regions: vec![],
                read: true,
                write: true,
                enabled: true,
                auth_type: api_key.as_ref().map(|_| "api_key".to_string()),
                auth_secret: api_key.filter(|k| !k.trim().is_empty()),
            },
            true,
        )
        .await?;
        self.reload().await?;
        self.list()
            .await?
            .into_iter()
            .find(|v| v.url == parsed.as_str())
            .ok_or_else(|| MessengerError::Storage("relay vanished after insert".into()))
    }

    /// Only user relays can be removed; manifest relays are disabled instead.
    pub async fn remove_user(&self, url: &str) -> Result<()> {
        let Some(row) = repo::get(&self.store, url).await? else { return Ok(()) };
        if row.source != repo::SOURCE_USER {
            return Err(MessengerError::Invalid("manifest relays cannot be removed; disable them".into()));
        }
        repo::delete(&self.store, url).await?;
        self.reload().await
    }

    pub async fn set_enabled(&self, url: &str, enabled: bool) -> Result<()> {
        if repo::get(&self.store, url).await?.is_none() {
            return Err(MessengerError::Invalid("unknown relay".into()));
        }
        repo::set_enabled(&self.store, url, enabled).await?;
        self.reload().await
    }

    // ─── Silent mode ────────────────────────────────────────────────────────

    pub async fn is_silent(&self) -> Result<bool> {
        settings::get_bool(&self.store, KEY_SILENT, false).await
    }

    pub async fn set_silent(&self, on: bool) -> Result<()> {
        settings::set_bool(&self.store, KEY_SILENT, on).await?;
        self.pool.read().await.set_silent(on).await;
        Ok(())
    }

    /// Push the enabled rows into the live pool. Until the user has chosen
    /// whose servers to use, the pool gets none and connects nowhere.
    pub async fn reload(&self) -> Result<()> {
        if self.servers_mode().await?.is_none() {
            return self.pool.read().await.set_relays(Vec::new()).await;
        }
        let relays: Vec<RelayConfig> = repo::list(&self.store)
            .await?
            .into_iter()
            .filter(|r| r.enabled)
            .filter_map(|r| Some(RelayConfig { url: RelayUrl::parse(&r.url)?, read: r.read, write: r.write, api_key: r.auth_secret }))
            .collect();
        self.pool.read().await.set_relays(relays).await
    }

    pub async fn shutdown(&self) {
        self.pool.read().await.shutdown().await;
    }
}

/// Renamed entries need their (new) manifest definition upserted too.
fn target_renamed<'a>(
    changes: &'a RelayChanges,
    target: &'a [&'a messenger_transport::ManifestRelay],
) -> impl Iterator<Item = &'a messenger_transport::ManifestRelay> + 'a {
    target
        .iter()
        .copied()
        .filter(move |t| changes.renamed.iter().any(|(id, _, _)| id == &t.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_transport::manifest::{ManifestRelay, FORMAT, VERSION};

    /// A store where the user chose the Veydan servers.
    async fn veydan_store() -> Store {
        let store = Store::open_in_memory().await.unwrap();
        settings::set(&store, KEY_MODE, "veydan").await.unwrap();
        store
    }

    fn manifest(serial: u64, relays: Vec<(&str, &str, &str)>) -> Manifest {
        Manifest {
            format: FORMAT.into(),
            v: VERSION,
            serial,
            issued_at: 1,
            regions: vec!["default".into(), "ru".into()],
            relays: relays
                .into_iter()
                .map(|(id, url, region)| ManifestRelay {
                    id: id.into(),
                    url: url.into(),
                    regions: vec![region.into()],
                    read: true,
                    write: true,
                    auth: None,
                })
                .collect(),
            media: vec![],
            sources: vec![],
            push: vec![],
        }
    }

    #[tokio::test]
    async fn init_applies_embedded_manifest_once_and_upgrades_older_installs() {
        let store = veydan_store().await;
        let svc = RelayService::init(store.clone(), None).await.unwrap();
        let embedded = Manifest::parse_content(EMBEDDED_MANIFEST_JSON).unwrap().serial;
        assert_eq!(svc.manifest_serial().await.unwrap(), Some(embedded));
        let n = svc.list().await.unwrap().len();
        assert!(n >= 1);
        let again = RelayService::init(store.clone(), None).await.unwrap();
        assert_eq!(again.list().await.unwrap().len(), n, "second init does not duplicate rows");
        svc.shutdown().await;
        again.shutdown().await;

        // An install that applied an older manifest gets the embedded one on start.
        settings::set(&store, KEY_SERIAL, "1").await.unwrap();
        let upgraded = RelayService::init(store.clone(), None).await.unwrap();
        assert_eq!(upgraded.manifest_serial().await.unwrap(), Some(embedded));
        upgraded.shutdown().await;

        // A newer (remotely applied) manifest is not rolled back by the embedded one.
        settings::set(&store, KEY_SERIAL, "99").await.unwrap();
        let kept = RelayService::init(store, None).await.unwrap();
        assert_eq!(kept.manifest_serial().await.unwrap(), Some(99));
        kept.shutdown().await;
    }

    #[tokio::test]
    async fn apply_manifest_migrates_renames_and_keeps_user_rows_and_enabled_flags() {
        let store = veydan_store().await;
        let svc = RelayService::init(store.clone(), None).await.unwrap();
        svc.apply_manifest(&manifest(10, vec![("a", "wss://a1.example", "default"), ("b", "wss://b.example", "default")]), false)
            .await
            .unwrap();
        svc.add_user("wss://mine.example", None).await.unwrap();
        svc.set_enabled("wss://a1.example", false).await.unwrap();

        // Rollback refused, same serial refused.
        assert!(svc.apply_manifest(&manifest(10, vec![]), false).await.is_err());
        assert!(svc.apply_manifest(&manifest(9, vec![]), false).await.is_err());

        let ch = svc
            .apply_manifest(&manifest(11, vec![("a", "wss://a2.example", "default"), ("c", "wss://c.example", "default")]), false)
            .await
            .unwrap();
        assert_eq!(ch.renamed.len(), 1);
        assert_eq!(ch.removed.len(), 1);
        assert_eq!(ch.added.len(), 1);

        let rows = svc.list().await.unwrap();
        let urls: Vec<_> = rows.iter().map(|r| r.url.as_str()).collect();
        assert!(urls.contains(&"wss://a2.example"), "renamed by id");
        assert!(!urls.contains(&"wss://a1.example"));
        assert!(!urls.contains(&"wss://b.example"), "removed");
        assert!(urls.contains(&"wss://c.example"), "added");
        assert!(urls.contains(&"wss://mine.example"), "user rows survive");
        let a2 = rows.iter().find(|r| r.url == "wss://a2.example").unwrap();
        assert!(!a2.enabled, "disabled flag survives the rename");
        assert_eq!(a2.relay_id.as_deref(), Some("a"));
        assert_eq!(svc.manifest_serial().await.unwrap(), Some(11));
        svc.shutdown().await;
    }

    #[tokio::test]
    async fn user_relays_add_remove_and_manifest_rows_are_protected() {
        let store = veydan_store().await;
        let svc = RelayService::init(store, None).await.unwrap();
        assert!(svc.add_user("https://nope", None).await.is_err());
        let v = svc.add_user("wss://mine.example/", Some("k1".into())).await.unwrap();
        assert_eq!(v.url, "wss://mine.example");
        assert_eq!(v.source, "user");
        assert!(svc.add_user("wss://mine.example", None).await.is_err(), "duplicate");
        let manifest_url = svc.list().await.unwrap().into_iter().find(|r| r.source == "manifest").unwrap().url;
        assert!(svc.remove_user(&manifest_url).await.is_err());
        svc.remove_user("wss://mine.example").await.unwrap();
        assert!(svc.list().await.unwrap().iter().all(|r| r.url != "wss://mine.example"));
        svc.shutdown().await;
    }

    #[tokio::test]
    async fn region_switch_and_silent_mode_persist() {
        let store = veydan_store().await;
        let svc = RelayService::init(store.clone(), None).await.unwrap();
        svc.set_region("ru").await.unwrap();
        assert_eq!(svc.region().await.unwrap(), "ru");
        assert!(svc.set_region("bad region!").await.is_err());
        svc.set_silent(true).await.unwrap();
        assert!(svc.is_silent().await.unwrap());
        assert!(svc.pool().await.is_silent());
        let info = svc.manifest_info().await.unwrap();
        assert!(info.silent_mode);
        assert_eq!(info.region, "ru");
        svc.shutdown().await;

        let again = RelayService::init(store, None).await.unwrap();
        assert!(again.pool().await.is_silent(), "silent mode survives restart");
        again.shutdown().await;
    }
}
