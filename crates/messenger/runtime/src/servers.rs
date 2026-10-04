// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Whose servers the messenger uses, and keeping the project's manifest fresh.
//!
//! Until the user chooses, the messenger connects nowhere and no session
//! runs. "Veydan" fetches the project's signed manifest (falling back to the
//! one built into the app) and applies it; "own" drops what a manifest
//! brought and never asks the project for anything.

use crate::relays::{ManifestCheck, ServersMode};
use crate::MessengerRuntime;
use messenger_core::traits::SystemClock;
use messenger_core::{Clock, MessengerError, Result};
use messenger_transport::remote::{candidate_urls, fetch_signed};
use messenger_transport::{ManifestFetcher, MANIFEST_URLS, PROJECT_MANIFEST_PUBKEY};
use std::sync::Arc;

/// The project's manifest is asked for at most this often on its own.
pub const REFRESH_EVERY_SECS: i64 = 24 * 60 * 60;

/// Where the project's manifest is fetched from and whose signature it needs.
#[derive(Clone)]
pub struct ManifestRemote {
    pub fetcher: Arc<dyn ManifestFetcher>,
    pub urls: Vec<String>,
    pub pinned: String,
}

impl ManifestRemote {
    pub fn project(fetcher: Arc<dyn ManifestFetcher>) -> Self {
        Self {
            fetcher,
            urls: MANIFEST_URLS.iter().map(|u| u.to_string()).collect(),
            pinned: PROJECT_MANIFEST_PUBKEY.to_string(),
        }
    }
}

impl MessengerRuntime {
    pub async fn servers_mode(&self) -> Result<Option<ServersMode>> {
        self.relays.servers_mode().await
    }

    /// Replace where manifests come from (tests, the CLI).
    pub fn set_manifest_remote(&self, remote: ManifestRemote) {
        *self.manifest_remote.write().unwrap() = remote;
    }

    /// Use the project's servers: fetch its manifest now and apply it, or the
    /// built-in one when it cannot be had. Servers the user added stay.
    pub async fn servers_use_veydan(&self) -> Result<ManifestCheck> {
        self.relays.set_servers_mode(ServersMode::Veydan).await?;
        let check = self.manifest_check(true).await?;
        self.servers_changed().await?;
        Ok(check)
    }

    /// Use only the user's servers. Needs at least one enabled relay.
    pub async fn servers_use_own(&self) -> Result<()> {
        let has_own = self.relays.list().await?.iter().any(|r| r.source == "user" && r.enabled);
        if !has_own {
            return Err(MessengerError::Invalid("servers_own_needs_relay".into()));
        }
        if let Err(e) = self.push_leave_manifest_server().await {
            eprintln!("messenger push: the project's server was not told to forget: {e}");
        }
        self.relays.set_servers_mode(ServersMode::Own).await?;
        self.relays.drop_manifest_relays().await?;
        self.servers_changed().await
    }

    /// Ask the project for a newer manifest. Without `force` it happens at
    /// most once a day; with the own servers chosen, never. `None` when
    /// nothing was asked.
    pub async fn manifest_refresh(&self, force: bool) -> Result<Option<ManifestCheck>> {
        if self.relays.servers_mode().await? != Some(ServersMode::Veydan) {
            return Ok(None);
        }
        if !force {
            if let Some(at) = self.relays.manifest_checked_at().await? {
                if SystemClock.now().secs() - at < REFRESH_EVERY_SECS {
                    return Ok(None);
                }
            }
        }
        let check = self.manifest_check(false).await?;
        if check.updated {
            self.servers_changed().await?;
        }
        Ok(Some(check))
    }

    /// Fetch and apply. `reapply` applies the manifest in use even when
    /// nothing newer came (choosing the Veydan servers again after own ones).
    async fn manifest_check(&self, reapply: bool) -> Result<ManifestCheck> {
        let remote = self.manifest_remote.read().unwrap().clone();
        let (current, origin) = self.relays.current_manifest().await?;
        let urls = candidate_urls(&remote.urls.iter().map(String::as_str).collect::<Vec<_>>(), Some(&current));
        // Nowhere to ask (no project URLs, no sources in the manifest): the
        // one in use stands, and that is not an error.
        let fetched = if urls.is_empty() {
            Ok((current.clone(), origin.clone()))
        } else {
            fetch_signed(remote.fetcher.as_ref(), &urls, &remote.pinned).await
        };
        self.relays.set_manifest_checked_at(SystemClock.now().secs()).await?;
        let check = match fetched {
            Ok((m, url)) if m.serial > current.serial => {
                self.relays.adopt_manifest(&m, &url, true).await?;
                ManifestCheck { origin: url, serial: m.serial, updated: true, error: None }
            }
            Ok(_) => {
                if reapply {
                    self.relays.adopt_manifest(&current, &origin, true).await?;
                }
                ManifestCheck { origin, serial: current.serial, updated: false, error: None }
            }
            Err(e) => {
                if reapply {
                    self.relays.adopt_manifest(&current, &origin, true).await?;
                }
                ManifestCheck { origin, serial: current.serial, updated: false, error: Some(e.to_string()) }
            }
        };
        Ok(check)
    }

    /// The relay or media set changed: media follows the manifest, the
    /// session starts if it could not before, and peers learn our inbox.
    async fn servers_changed(&self) -> Result<()> {
        // Which servers are the project's decides what goes through a bridge.
        if let Err(e) = self.net_apply().await {
            eprintln!("messenger net: the way to the servers was not set: {e}");
        }
        if let Err(e) = self.seed_media_servers().await {
            eprintln!("messenger: media servers from the manifest not applied: {e}");
        }
        if !self.refresh_signer().await? && self.session.lock().await.is_some() {
            if let Err(e) = self.publish_dm_relays(false).await {
                eprintln!("messenger: inbox relay list not published: {e}");
            }
        }
        Ok(())
    }
}

/// For tests elsewhere in the crate: the Veydan servers, chosen without a
/// network (the project's manifest "cannot be fetched", the built-in one
/// is used).
#[cfg(test)]
pub(crate) async fn use_veydan_offline(rt: &MessengerRuntime) {
    struct Offline;
    #[async_trait::async_trait]
    impl ManifestFetcher for Offline {
        async fn fetch(&self, _: &str) -> Result<String> {
            Err(MessengerError::Transport("offline".into()))
        }
    }
    rt.set_manifest_remote(ManifestRemote { fetcher: Arc::new(Offline), urls: vec![], pinned: String::new() });
    rt.servers_use_veydan().await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use messenger_core::MessengerConfig;
    use messenger_testkit::MemorySecretStore;
    use messenger_transport::{Manifest, EMBEDDED_MANIFEST_JSON};
    use nostr::key::Keys;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Fake {
        bodies: std::sync::Mutex<HashMap<String, String>>,
        calls: AtomicUsize,
    }

    #[async_trait]
    impl ManifestFetcher for Fake {
        async fn fetch(&self, url: &str) -> Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.bodies
                .lock()
                .unwrap()
                .get(url)
                .cloned()
                .ok_or_else(|| MessengerError::Transport("http 404".into()))
        }
    }

    const URL: &str = "https://example.test/m.json";

    struct Setup {
        rt: MessengerRuntime,
        fake: Arc<Fake>,
        project: Keys,
        _dir: tempfile::TempDir,
    }

    async fn setup() -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let rt = MessengerRuntime::start(cfg, Arc::new(MemorySecretStore::unlocked())).await.unwrap();
        // Off the network: the pool keeps its rows but connects nowhere.
        rt.relays().set_silent(true).await.unwrap();
        let fake = Arc::new(Fake::default());
        let project = Keys::generate();
        rt.set_manifest_remote(ManifestRemote {
            fetcher: fake.clone(),
            urls: vec![URL.into()],
            pinned: project.public_key().to_hex(),
        });
        Setup { rt, fake, project, _dir: dir }
    }

    fn publish(s: &Setup, serial: u64, relay: &str) {
        let mut m = Manifest::parse_content(EMBEDDED_MANIFEST_JSON).unwrap();
        m.serial = serial;
        m.relays[0].url = relay.into();
        s.fake.bodies.lock().unwrap().insert(URL.into(), m.sign(&s.project).unwrap());
    }

    fn embedded_serial() -> u64 {
        Manifest::parse_content(EMBEDDED_MANIFEST_JSON).unwrap().serial
    }

    /// The relays in use, sorted.
    async fn urls(rt: &MessengerRuntime) -> Vec<String> {
        let mut v: Vec<String> = rt.relays().list().await.unwrap().into_iter().map(|r| r.url).collect();
        v.sort();
        v
    }

    /// The relays of the built-in manifest, sorted; with the first one
    /// replaced when `first` is given (what `publish` puts out).
    fn embedded_urls(first: Option<&str>) -> Vec<String> {
        let m = Manifest::parse_content(EMBEDDED_MANIFEST_JSON).unwrap();
        let mut v: Vec<String> = m.relays.iter().map(|r| r.url.clone()).collect();
        if let Some(f) = first {
            v[0] = f.into();
        }
        v.sort();
        v
    }

    #[tokio::test]
    async fn nothing_runs_until_servers_are_chosen() {
        let s = setup().await;
        assert_eq!(s.rt.servers_mode().await.unwrap(), None);
        assert!(s.rt.relays().list().await.unwrap().is_empty());
        assert!(s.rt.relays().pool().await.configured().is_empty());
        assert!(s.rt.media_servers().await.unwrap().is_empty());
        s.rt.identity().create("pw").await.unwrap();
        assert!(!s.rt.refresh_signer().await.unwrap(), "no session before a choice");
        assert!(!s.rt.status().await.unwrap().session_active);
        assert_eq!(s.rt.manifest_refresh(true).await.unwrap(), None, "nothing is asked");
        assert_eq!(s.fake.calls.load(Ordering::SeqCst), 0);

        s.rt.servers_use_veydan().await.unwrap();
        assert!(s.rt.status().await.unwrap().session_active, "the choice starts the session");
        s.rt.shutdown().await;
    }

    #[tokio::test]
    async fn veydan_takes_the_signed_manifest_or_the_built_in_one() {
        let s = setup().await;
        // Nothing published yet: the built-in manifest, and why.
        let check = s.rt.servers_use_veydan().await.unwrap();
        assert_eq!(check.origin, "embedded");
        assert!(check.error.as_deref().unwrap().contains("404"));
        assert_eq!(urls(&s.rt).await, embedded_urls(None));
        assert!(s.rt.media_servers().await.unwrap().iter().any(|m| m.source == "manifest"));

        // Published: a daily check waits, a forced one applies it.
        publish(&s, embedded_serial() + 1, "wss://node-2.veydan.net");
        assert_eq!(s.rt.manifest_refresh(false).await.unwrap(), None, "checked a moment ago");
        let check = s.rt.manifest_refresh(true).await.unwrap().unwrap();
        assert!(check.updated);
        assert_eq!(check.origin, URL);
        assert_eq!(urls(&s.rt).await, embedded_urls(Some("wss://node-2.veydan.net")));
        let info = s.rt.relays().manifest_info().await.unwrap();
        assert_eq!((info.origin.as_str(), info.serial), (URL, Some(embedded_serial() + 1)));

        // An older or foreign manifest changes nothing.
        publish(&s, embedded_serial(), "wss://old.veydan.net");
        assert!(!s.rt.manifest_refresh(true).await.unwrap().unwrap().updated);
        let mut m = Manifest::parse_content(EMBEDDED_MANIFEST_JSON).unwrap();
        m.serial = 100;
        s.fake.bodies.lock().unwrap().insert(URL.into(), m.sign(&Keys::generate()).unwrap());
        let check = s.rt.manifest_refresh(true).await.unwrap().unwrap();
        assert!(!check.updated && check.error.unwrap().contains("unexpected key"));
        assert_eq!(urls(&s.rt).await, embedded_urls(Some("wss://node-2.veydan.net")));
        s.rt.shutdown().await;
    }

    #[tokio::test]
    async fn nowhere_to_ask_is_the_built_in_manifest_without_an_error() {
        let s = setup().await;
        s.rt.set_manifest_remote(ManifestRemote { fetcher: s.fake.clone(), urls: vec![], pinned: "00".into() });
        let check = s.rt.servers_use_veydan().await.unwrap();
        assert_eq!((check.origin.as_str(), check.updated, check.error), ("embedded", false, None));
        assert_eq!(urls(&s.rt).await, embedded_urls(None));
        let check = s.rt.manifest_refresh(true).await.unwrap().unwrap();
        assert_eq!((check.updated, check.error), (false, None));
        assert_eq!(s.fake.calls.load(Ordering::SeqCst), 0, "nothing was asked");
        s.rt.shutdown().await;
    }

    #[tokio::test]
    async fn own_servers_drop_the_manifest_and_never_ask() {
        let s = setup().await;
        assert!(s.rt.servers_use_own().await.is_err(), "a relay of one's own is needed");
        s.rt.relays().add_user("wss://mine.example", None).await.unwrap();
        assert!(s.rt.relays().pool().await.configured().is_empty(), "not used before the choice");
        s.rt.servers_use_own().await.unwrap();
        assert_eq!(urls(&s.rt).await, vec!["wss://mine.example".to_string()]);
        assert_eq!(s.rt.relays().pool().await.configured().len(), 1);
        assert!(s.rt.media_servers().await.unwrap().is_empty());
        assert_eq!(s.rt.push_status().await.unwrap().server, None, "no push server of the project");
        assert_eq!(s.rt.manifest_refresh(true).await.unwrap(), None);
        assert_eq!(s.fake.calls.load(Ordering::SeqCst), 0, "the project was never asked");

        // Back to Veydan: its servers return, the user's stay.
        s.rt.servers_use_veydan().await.unwrap();
        let now = urls(&s.rt).await;
        assert!(embedded_urls(None).iter().all(|u| now.contains(u)) && now.contains(&"wss://mine.example".to_string()));
        s.rt.servers_use_own().await.unwrap();
        assert_eq!(urls(&s.rt).await, vec!["wss://mine.example".to_string()]);
        s.rt.shutdown().await;
    }

    #[tokio::test]
    async fn an_install_with_a_key_keeps_the_veydan_servers() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let secrets = Arc::new(MemorySecretStore::unlocked());
        let rt = MessengerRuntime::start(cfg.clone(), secrets.clone()).await.unwrap();
        rt.identity().create("pw").await.unwrap();
        rt.shutdown().await;

        let rt = MessengerRuntime::start(cfg, secrets).await.unwrap();
        assert_eq!(rt.servers_mode().await.unwrap(), Some(ServersMode::Veydan));
        assert!(rt.status().await.unwrap().session_active);
        assert_eq!(urls(&rt).await, embedded_urls(None));
        rt.shutdown().await;
    }
}
