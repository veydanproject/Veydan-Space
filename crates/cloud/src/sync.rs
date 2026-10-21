// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The backend `cloud` of sync from the side of the module
//! (internal/vcloud-plan.md 4.7, 4.8; spec/vstore.md §14): the module registers the
//! credentials sync-host builds `CloudStorage` with — the NIP-98 of the
//! identity through the messenger's `Signer`, the hosts of `storage[]`
//! through `VaultHosts`, the passphrase of the vault derived from the nsec
//! inside the messenger (`Signer::derive_secret`), the way of the requests
//! through the messenger's bridges (`VaultHosts::send`) — and answers
//! `cloud_usage`: the pools of the plan, how full they are and what takes
//! the room.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};
use veydan_core::{AppError, BoxFuture, CmdResult, Core};
use veydan_shell::seams::{
    code, CloudError, CloudReach, Http, HttpRequest, Signer, VaultHosts, WireEvent,
    SYNC_VAULT_PASSPHRASE,
};
use veydan_shell::sync_config::{self, CloudCredentials};
use veydan_sync::storage_cloud::{
    SendFailure, TransportFuture, VaultAnswer, VaultRequest, VaultTransport,
};
use veydan_sync::{RequestSigner, SignFuture, SyncError};
use zeroize::Zeroizing;

use crate::api::{CloudApi, Usage, UsageLine};

/// The share of a pool from which the settings of sync say so quietly.
pub const NOTICE_PERCENT: u8 = 70;
/// From here a banner warns.
pub const WARN_PERCENT: u8 = 90;
/// The pool is full: files no longer sync.
pub const FULL_PERCENT: u8 = 100;

/// Tries of the refresh of the hosts at start, while the messenger starts.
const START_TRIES: u32 = 3;
const START_PAUSE: Duration = Duration::from_secs(20);

/// The passphrase of the vault from the 32 bytes the messenger derives:
/// their base64, as `veydan_recovery::rk::sync_vault_passphrase` writes it.
pub fn passphrase_of(okm: &[u8; 32]) -> Zeroizing<String> {
    Zeroizing::new(B64.encode(okm))
}

/// `Authorization` of NIP-98 from the event the messenger signed.
fn authorization(event: &WireEvent) -> Result<String, SyncError> {
    let json = serde_json::to_vec(event)?;
    Ok(format!("Nostr {}", B64.encode(json)))
}

/// A refusal of a seam in the words of sync: a broken signature is
/// `Crypto`, the rest (no identity, the lock closed, the messenger
/// stopped) is `Auth`, never retried.
fn sync_error(e: CloudError) -> SyncError {
    if e.is(code::CRYPTO) {
        SyncError::Crypto(e.to_string())
    } else {
        SyncError::Auth(e.to_string())
    }
}

/// The module `cloud` follows the messenger (products.json `requires`), so
/// a seam of the messenger that is not there yet means it has not started.
const NOT_STARTED: &str = "the messenger has not started yet";

/// [`RequestSigner`] over the messenger's [`Signer`].
pub struct IdentityRequests {
    signer: Option<Arc<dyn Signer>>,
}

impl RequestSigner for IdentityRequests {
    fn authorization<'a>(
        &'a self,
        method: &'a str,
        url: &'a str,
        payload_sha256: Option<[u8; 32]>,
        created_at: u64,
    ) -> SignFuture<'a> {
        Box::pin(async move {
            let signer = self.signer.as_ref().ok_or_else(|| {
                SyncError::Auth(format!("{}: {NOT_STARTED}", code::MESSENGER_STOPPED))
            })?;
            let event = signer
                .nip98_at(method, url, payload_sha256, created_at)
                .await
                .map_err(sync_error)?;
            authorization(&event)
        })
    }
}

/// [`VaultTransport`] over the messenger's way to the hosts of `storage[]`
/// ([`VaultHosts::send`]): through a bridge wherever the media go through
/// one (spec/vstore.md §14).
pub struct BridgedVault {
    hosts: Option<Arc<dyn VaultHosts>>,
}

/// A refusal of the messenger's way in the words of the storage: the
/// network and a host gone from the manifest let the next host try.
fn send_failure(e: CloudError) -> SendFailure {
    if e.is(code::TRANSPORT) || e.is(code::NO_HOST) {
        SendFailure::Host(e.to_string())
    } else {
        SendFailure::Other(SyncError::Storage(e.to_string()))
    }
}

impl VaultTransport for BridgedVault {
    fn send<'a>(&'a self, request: VaultRequest<'a>) -> TransportFuture<'a> {
        Box::pin(async move {
            let hosts = self.hosts.as_ref().ok_or_else(|| {
                SendFailure::Other(SyncError::Storage(format!(
                    "{}: {NOT_STARTED}",
                    code::MESSENGER_STOPPED
                )))
            })?;
            let request = HttpRequest {
                method: request.method.to_string(),
                url: request.url.to_string(),
                headers: request
                    .headers
                    .into_iter()
                    .map(|(name, value)| (name.to_string(), value))
                    .collect(),
                body: request.body.map(<[u8]>::to_vec).unwrap_or_default(),
            };
            let answer = hosts.send(request).await.map_err(send_failure)?;
            Ok(VaultAnswer {
                status: answer.status,
                headers: answer.headers,
                body: answer.body,
            })
        })
    }
}

/// Why the last refresh found no host, for the error of the next cycle.
static WHY_NO_HOST: Mutex<Option<String>> = Mutex::new(None);

fn set_why_no_host(why: Option<String>) {
    *WHY_NO_HOST.lock().unwrap_or_else(|e| e.into_inner()) = why;
}

fn app_error(e: CloudError) -> AppError {
    AppError::other(e)
}

/// The credentials of the backend `cloud` for sync-host.
pub struct Credentials<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> Credentials<R> {
    fn signer_seam(&self) -> Option<Arc<dyn Signer>> {
        veydan_shell::seam::<dyn Signer, _, _>(&self.app)
    }

    /// The `public_url`s of `storage[]` for this device, in the order to try.
    async fn hosts(&self) -> Result<Vec<String>, CloudError> {
        let seam = veydan_shell::seam::<dyn VaultHosts, _, _>(&self.app)
            .ok_or_else(|| CloudError::new(code::MESSENGER_STOPPED, NOT_STARTED))?;
        Ok(seam
            .storage_hosts()
            .await?
            .into_iter()
            .map(|h| h.url)
            .collect())
    }

    /// Why `storage[]` gave no host, by whose servers are in use.
    async fn why_none(&self) -> CloudError {
        let reach = match veydan_shell::seam::<dyn Http, _, _>(&self.app) {
            Some(http) => http.reach().await.ok(),
            None => None,
        };
        let message = match reach {
            Some(CloudReach::Own) => "Veydan Cloud is not available with your own servers",
            Some(CloudReach::Unchosen { .. }) => "the servers of the messenger are not chosen yet",
            _ => "the servers in use name no host of Veydan Cloud for this region",
        };
        CloudError::new(code::NO_HOST, message)
    }
}

impl<R: Runtime> CloudCredentials for Credentials<R> {
    fn signer(&self) -> Arc<dyn RequestSigner> {
        Arc::new(IdentityRequests {
            signer: self.signer_seam(),
        })
    }

    fn refresh_hosts(&self) -> BoxFuture<'_, CmdResult<()>> {
        Box::pin(async move {
            let hosts = self.hosts().await.map_err(app_error)?;
            let core = self
                .app
                .try_state::<Core>()
                .ok_or_else(|| AppError::other("the data file is not open"))?;
            sync_config::save_cloud_hosts(&core.db, &hosts).await?;
            if hosts.is_empty() {
                let why = self.why_none().await;
                set_why_no_host(Some(why.to_string()));
                return Err(app_error(why));
            }
            set_why_no_host(None);
            Ok(())
        })
    }

    fn transport(&self) -> Option<Arc<dyn VaultTransport>> {
        Some(Arc::new(BridgedVault {
            hosts: veydan_shell::seam::<dyn VaultHosts, _, _>(&self.app),
        }))
    }

    fn why_no_host(&self) -> Option<String> {
        WHY_NO_HOST
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn passphrase(&self) -> BoxFuture<'_, CmdResult<Zeroizing<String>>> {
        Box::pin(async move {
            let signer = self.signer_seam().ok_or_else(|| {
                AppError::other(format!("{}: {NOT_STARTED}", code::MESSENGER_STOPPED))
            })?;
            let okm = signer
                .derive_secret(SYNC_VAULT_PASSPHRASE)
                .await
                .map_err(app_error)?;
            Ok(passphrase_of(&okm))
        })
    }
}

/// Puts the credentials in place for sync. Called by the setup of the module.
pub fn register<R: Runtime>(app: &AppHandle<R>) {
    sync_config::register_sync_cloud_credentials(Arc::new(Credentials { app: app.clone() }));
}

/// The hosts of the manifest in use, once the messenger answers: a few
/// tries while it starts, quietly; a create, a join and a probe refresh
/// them again.
pub fn start(app: AppHandle) -> BoxFuture<'static, Result<(), AppError>> {
    Box::pin(async move {
        tauri::async_runtime::spawn(async move {
            let credentials = Credentials { app };
            for attempt in 0..START_TRIES {
                if credentials.refresh_hosts().await.is_ok() {
                    return;
                }
                if attempt + 1 < START_TRIES {
                    tokio::time::sleep(START_PAUSE).await;
                }
            }
        });
        Ok(())
    })
}

// ── Usage ─────────────────────────────────────────────────────────────────

/// One pool of the plan and how full it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PoolLevel {
    /// `text` or `files` (spec/vstore.md §9); `media` and `total` when the
    /// plan sets `media.storage_bytes` or `storage.total_bytes` (§6).
    pub pool: String,
    /// Bytes in the pool.
    pub used: u64,
    /// The bytes of the plan; 0 is no limit.
    pub limit: u64,
    /// `used` in percent of `limit`, whole, at most 100.
    pub percent: u8,
    /// 0, [`NOTICE_PERCENT`], [`WARN_PERCENT`] or [`FULL_PERCENT`].
    pub level: u8,
}

/// What takes the room, by the paths of the vault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Category {
    /// `journal` (notes, passwords, settings: `manifest.json`,
    /// `devices/`), `files` (`blobs/`: attachments of notes under the
    /// threshold of large files and the files of browser profiles, not told
    /// apart), `large_files` (`large-files/`: attachments of notes from the
    /// threshold), `recovery` (`escrow`), `other`.
    pub id: &'static str,
    /// The pool it counts in, `text` or `files`.
    pub pool: String,
    /// Bytes under the paths of the category.
    pub bytes: u64,
    /// Objects under them.
    pub objects: u64,
}

/// The answer of `cloud_usage`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CloudUsage {
    /// `GET /vault/v1/usage` as the host wrote it.
    pub usage: Usage,
    /// Each pool of the plan with its level.
    pub pools: Vec<PoolLevel>,
    /// The highest level of the pools: what the settings and the banner show.
    pub level: u8,
    /// What takes the room, the largest first.
    pub categories: Vec<Category>,
    /// False: the categories come from the paths alone. The client does not
    /// know yet which blob is a note's and which a profile's (the owners of
    /// blobs are the handlers of the modules), so `files` holds both.
    pub by_owner: bool,
    /// What turning on the files of profiles would send at most, bytes.
    pub profile_files_estimate: u64,
    /// Sync of this device uses the backend `cloud`.
    pub on_cloud: bool,
}

/// The level of `used` of `limit`: a pool without a limit is never full.
pub fn level_of(used: u64, limit: u64) -> u8 {
    if limit == 0 {
        return 0;
    }
    let percent = u128::from(used) * 100 / u128::from(limit);
    match percent {
        p if p >= u128::from(FULL_PERCENT) => FULL_PERCENT,
        p if p >= u128::from(WARN_PERCENT) => WARN_PERCENT,
        p if p >= u128::from(NOTICE_PERCENT) => NOTICE_PERCENT,
        _ => 0,
    }
}

fn percent_of(used: u64, limit: u64) -> u8 {
    if limit == 0 {
        return 0;
    }
    (u128::from(used) * 100 / u128::from(limit)).min(100) as u8
}

fn category_of(line: &UsageLine) -> &'static str {
    let p = line.prefix.as_str();
    if p == "manifest.json" || p.starts_with("devices/") {
        "journal"
    } else if p.starts_with("blobs/") {
        "files"
    } else if p.starts_with("large-files/") {
        "large_files"
    } else if p.starts_with("escrow") {
        "recovery"
    } else {
        "other"
    }
}

fn pool_level(pool: &str, used: u64, limit: u64) -> PoolLevel {
    PoolLevel {
        pool: pool.to_owned(),
        used,
        limit,
        percent: percent_of(used, limit),
        level: level_of(used, limit),
    }
}

/// The pools with their levels and the breakdown by category, the largest
/// first.
pub fn summarize(usage: &Usage) -> (Vec<PoolLevel>, u8, Vec<Category>) {
    let pools: Vec<PoolLevel> = usage
        .pools
        .iter()
        .map(|(pool, p)| pool_level(pool, p.used, p.limit))
        .collect();
    // The media of chats and the shared storage count as pools when the
    // plan sets them (spec/vstore.md §6): a full total stops the vault too.
    let mut pools = pools;
    if let Some(media) = &usage.media {
        if let Some(limit) = media.limit {
            pools.push(pool_level("media", media.used, limit));
        }
    }
    if let Some(total) = &usage.total {
        if let Some(limit) = total.limit {
            pools.push(pool_level("total", total.used, limit));
        }
    }
    let level = pools.iter().map(|p| p.level).max().unwrap_or(0);
    let mut categories: Vec<Category> = Vec::new();
    for line in &usage.breakdown {
        let id = category_of(line);
        match categories.iter_mut().find(|c| c.id == id) {
            Some(c) => {
                c.bytes += line.bytes;
                c.objects += line.objects;
            }
            None => categories.push(Category {
                id,
                pool: line.pool.clone(),
                bytes: line.bytes,
                objects: line.objects,
            }),
        }
    }
    categories.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.id.cmp(b.id)));
    (pools, level, categories)
}

/// What the account's vault takes, for the settings of sync, the banner of
/// a full pool and the page of the module. The hosts are refreshed on the
/// way, quietly.
#[tauri::command]
pub async fn cloud_usage<R: Runtime>(app: AppHandle<R>) -> Result<CloudUsage, CloudError> {
    let _ = Credentials { app: app.clone() }.refresh_hosts().await;
    let api = veydan_shell::seam::<dyn CloudApi, _, _>(&app)
        .ok_or_else(|| CloudError::new(code::NO_HOST, "Veydan Cloud is not in this product"))?;
    let usage = api.usage().await?;
    let (pools, level, categories) = summarize(&usage);
    let (profile_files_estimate, on_cloud) = match app.try_state::<Core>() {
        Some(core) => {
            // Tens of thousands of files in a profile: off the runtime's workers.
            let dir = core.app_data_dir.clone();
            let estimate = tauri::async_runtime::spawn_blocking(move || {
                sync_config::profile_files_estimate(&dir)
            });
            (
                estimate.await.unwrap_or(0),
                sync_config::on_cloud(&core.db).await,
            )
        }
        None => (0, false),
    };
    Ok(CloudUsage {
        usage,
        pools,
        level,
        categories,
        by_owner: false,
        profile_files_estimate,
        on_cloud,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::AccountState;
    use veydan_shell::seams::{HttpResponse, SeamFuture, StorageHost, DERIVE_LABELS, DERIVE_SALT};
    use veydan_sync::Storage as _;

    /// The nsec of the vectors of crates/recovery (tests/recovery.rs).
    const NSEC: [u8; 32] = [0x42; 32];
    /// `sync_vault_passphrase` of crates/recovery/golden/recovery-v1.json.
    const GOLDEN: &str = "B2fw67HdFNJT2vAlkJ9Q19jFqc2F2V3iUZF4uCgnEnI=";

    /// A signer that derives as the messenger does, through the helper of
    /// crates/recovery, and signs nothing it is not asked.
    struct Fake;

    impl Signer for Fake {
        fn pubkey(&self) -> SeamFuture<'_, String> {
            Box::pin(async { Ok("ab".repeat(32)) })
        }
        fn nip98<'a>(
            &'a self,
            _: &'a str,
            _: &'a str,
            _: Option<[u8; 32]>,
        ) -> SeamFuture<'a, WireEvent> {
            Box::pin(async { Err(CloudError::new(code::NOT_SUPPORTED, "now")) })
        }
        fn nip44_encrypt<'a>(&'a self, _: &'a str, p: &'a [u8]) -> SeamFuture<'a, Vec<u8>> {
            Box::pin(async move { Ok(p.to_vec()) })
        }
        fn nip44_decrypt<'a>(
            &'a self,
            _: &'a str,
            c: &'a [u8],
        ) -> SeamFuture<'a, Zeroizing<Vec<u8>>> {
            Box::pin(async move { Ok(Zeroizing::new(c.to_vec())) })
        }
        fn nip98_at<'a>(
            &'a self,
            method: &'a str,
            url: &'a str,
            payload: Option<[u8; 32]>,
            created_at: u64,
        ) -> SeamFuture<'a, WireEvent> {
            Box::pin(async move {
                let mut tags = vec![
                    vec!["u".to_string(), url.to_string()],
                    vec!["method".into(), method.into()],
                ];
                if let Some(p) = payload {
                    tags.push(vec![
                        "payload".into(),
                        p.iter().map(|b| format!("{b:02x}")).collect(),
                    ]);
                }
                Ok(WireEvent {
                    id: "00".repeat(32),
                    pubkey: "ab".repeat(32),
                    created_at,
                    kind: 27235,
                    tags,
                    content: String::new(),
                    sig: "11".repeat(64),
                })
            })
        }
        fn derive_secret<'a>(&'a self, label: &'a str) -> SeamFuture<'a, Zeroizing<[u8; 32]>> {
            Box::pin(async move {
                if label != SYNC_VAULT_PASSPHRASE {
                    return Err(CloudError::new(code::BAD_REQUEST, "label"));
                }
                let text = veydan_recovery::rk::sync_vault_passphrase(&NSEC);
                let raw = B64
                    .decode(text.as_str())
                    .map_err(|e| CloudError::new(code::CRYPTO, e.to_string()))?;
                let mut out = Zeroizing::new([0u8; 32]);
                out.copy_from_slice(&raw);
                Ok(out)
            })
        }
    }

    /// `storage[]` of the manifest in use, and the messenger's way to it:
    /// it keeps what was sent and answers `404` as vstore would.
    #[derive(Default)]
    struct Hosts {
        urls: Mutex<Vec<&'static str>>,
        sent: Mutex<Vec<HttpRequest>>,
    }

    impl VaultHosts for Hosts {
        fn storage_hosts(&self) -> SeamFuture<'_, Vec<StorageHost>> {
            let hosts = self
                .urls
                .lock()
                .expect("hosts")
                .iter()
                .enumerate()
                .map(|(i, url)| StorageHost {
                    url: url.to_string(),
                    regions: vec!["*".into()],
                    priority: i as u32,
                })
                .collect();
            Box::pin(async move { Ok(hosts) })
        }

        fn send(&self, request: HttpRequest) -> SeamFuture<'_, HttpResponse> {
            self.sent.lock().expect("sent").push(request);
            Box::pin(async {
                Ok(HttpResponse {
                    status: 404,
                    headers: vec![("content-type".into(), "application/json".into())],
                    body: br#"{"error":{"code":"not_found","message":"no"}}"#.to_vec(),
                })
            })
        }
    }

    /// The messenger on the user's own servers.
    struct OwnServers;

    impl Http for OwnServers {
        fn reach(&self) -> SeamFuture<'_, CloudReach> {
            Box::pin(async { Ok(CloudReach::Own) })
        }
        fn send(&self, _: HttpRequest) -> SeamFuture<'_, HttpResponse> {
            Box::pin(async { Err(CloudError::new(code::NO_HOST, "own servers")) })
        }
    }

    /// A transport the messenger refuses, in the words of the storage.
    #[test]
    fn the_refusals_of_the_way_keep_their_meaning() {
        assert!(matches!(
            send_failure(CloudError::new(code::TRANSPORT, "reset")),
            SendFailure::Host(_)
        ));
        assert!(matches!(
            send_failure(CloudError::new(code::NO_HOST, "gone")),
            SendFailure::Host(_)
        ));
        assert!(matches!(
            send_failure(CloudError::new(code::MESSENGER_STOPPED, "stopped")),
            SendFailure::Other(SyncError::Storage(_))
        ));
    }

    #[test]
    fn the_seam_derives_what_recovery_derives() {
        assert_eq!(DERIVE_SALT.as_bytes(), veydan_recovery::rk::CLOUD_SALT);
        assert_eq!(
            SYNC_VAULT_PASSPHRASE.as_bytes(),
            veydan_recovery::rk::SYNC_VAULT_PASSPHRASE_INFO
        );
        assert!(DERIVE_LABELS.contains(&SYNC_VAULT_PASSPHRASE));
        assert_eq!(
            veydan_recovery::rk::sync_vault_passphrase(&NSEC).as_str(),
            GOLDEN
        );
    }

    #[tokio::test]
    async fn the_credentials_give_the_signature_the_hosts_and_the_passphrase() {
        let dir = tempfile::tempdir().expect("dir");
        let db = veydan_core::db::open(&dir.path().join("app.db"), &[veydan_core::SCHEMA])
            .await
            .expect("db");
        let app = tauri::test::mock_app();
        app.manage(Core::new(
            db.clone(),
            dir.path().to_owned(),
            Default::default(),
        ));
        let credentials = Credentials {
            app: app.handle().clone(),
        };

        // Without the messenger's seams nothing is signed, nothing derived.
        let refused = credentials
            .signer()
            .authorization("GET", "https://a.example.com/vault/v1/o?prefix=", None, 1)
            .await;
        match refused {
            Err(SyncError::Auth(m)) => assert!(m.contains("not started"), "{m}"),
            other => panic!("{other:?}"),
        }
        assert!(credentials.passphrase().await.is_err());
        assert!(credentials.refresh_hosts().await.is_err());
        let way = credentials.transport().expect("the messenger's way");
        let request = VaultRequest {
            method: "GET",
            url: "https://media-eu.example.com/vault/v1/o/manifest.json",
            headers: Vec::new(),
            body: None,
        };
        assert!(matches!(
            way.send(request).await,
            Err(SendFailure::Other(SyncError::Storage(_)))
        ));

        veydan_shell::provide::<dyn Signer, _, _>(app.handle(), Arc::new(Fake));
        let hosts = Arc::new(Hosts::default());
        veydan_shell::provide::<dyn VaultHosts, _, _>(app.handle(), hosts.clone());
        veydan_shell::provide::<dyn Http, _, _>(app.handle(), Arc::new(OwnServers));
        assert_eq!(
            credentials.passphrase().await.expect("passphrase").as_str(),
            GOLDEN
        );
        // The user's own servers: no host, and the error says why.
        let none = credentials.refresh_hosts().await.expect_err("no host");
        assert!(none.to_string().contains("own servers"), "{none}");
        assert!(credentials
            .why_no_host()
            .is_some_and(|w| w.starts_with("no_host") && w.contains("own servers")));
        *hosts.urls.lock().expect("hosts") = vec![
            "https://media-eu.example.com/vault",
            "https://media-us.example.com/vault",
        ];
        credentials.refresh_hosts().await.expect("hosts");
        assert_eq!(credentials.why_no_host(), None);
        assert_eq!(
            sync_config::load_cloud_hosts(&db).await,
            [
                "https://media-eu.example.com/vault",
                "https://media-us.example.com/vault"
            ]
        );

        // `Authorization: Nostr <base64 of the event>`, made at the time asked.
        let header = credentials
            .signer()
            .authorization(
                "PUT",
                "https://media-eu.example.com/vault/v1/o/blobs/aa",
                Some([7; 32]),
                1_791_000_000,
            )
            .await
            .expect("signed");
        let json = B64
            .decode(header.strip_prefix("Nostr ").expect("scheme"))
            .expect("base64");
        let event: WireEvent = serde_json::from_slice(&json).expect("event");
        assert_eq!(event.created_at, 1_791_000_000);
        assert_eq!(
            event.tags[0],
            ["u", "https://media-eu.example.com/vault/v1/o/blobs/aa"]
        );
        assert_eq!(event.tags[2][1], "07".repeat(32));

        // The storage sync builds goes by the messenger's way, signed.
        let storage = veydan_sync::CloudStorage::new(
            &sync_config::load_cloud_hosts(&db).await,
            credentials.signer(),
        )
        .expect("storage")
        .with_transport(credentials.transport().expect("way"));
        assert_eq!(storage.get("manifest.json").await.expect("404"), None);
        let sent = hosts.sent.lock().expect("sent");
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].method, "GET");
        assert_eq!(
            sent[0].url,
            "https://media-eu.example.com/vault/v1/o/manifest.json"
        );
        assert!(sent[0]
            .headers
            .iter()
            .any(|(n, v)| n == "authorization" && v.starts_with("Nostr ")));
    }

    fn usage(text: (u64, u64), files: (u64, u64)) -> Usage {
        serde_json::from_value(serde_json::json!({
            "as_of": 1791901234u64, "host": "eu-1", "plan": "free", "state": "free", "until": null,
            "table_serial": 1842, "max_object_bytes": 67108864,
            "pools": {
                "text":  { "used": text.0,  "limit": text.1,  "objects": 412 },
                "files": { "used": files.0, "limit": files.1, "objects": 1803 }
            },
            "breakdown": [
                { "prefix": "manifest.json", "pool": "text", "bytes": 612, "objects": 1 },
                { "prefix": "devices/0f3a2b1c/log/", "pool": "text", "bytes": 1200000, "objects": 380 },
                { "prefix": "devices/0f3a2b1c/snapshot.bin", "pool": "text", "bytes": 640000, "objects": 1 },
                { "prefix": "blobs/", "pool": "files", "bytes": 1500000000u64, "objects": 1700 },
                { "prefix": "large-files/v2/chunks/", "pool": "files", "bytes": 432000000u64, "objects": 100 },
                { "prefix": "large-files/v2/manifests/", "pool": "files", "bytes": 735283, "objects": 3 },
                { "prefix": "escrow", "pool": "text", "bytes": 2588, "objects": 1 },
                { "prefix": "calendar/", "pool": "files", "bytes": 10, "objects": 1 }
            ],
            "reconciled_at": 1791898800u64
        }))
        .expect("usage")
    }

    /// The example of spec/vstore.md §6, read and summed by category.
    #[test]
    fn usage_reads_and_sums_by_category() {
        let u = usage((1843200, 536870912), (1932735283, 2147483648));
        assert_eq!(u.state, AccountState::Free);
        let (pools, level, categories) = summarize(&u);
        assert_eq!(pools.len(), 2);
        let files = pools.iter().find(|p| p.pool == "files").expect("files");
        // 1932735283 of 2 GiB is 89.99 %: a notice, the warning starts at 90.
        assert_eq!((files.percent, files.level), (89, NOTICE_PERCENT));
        assert_eq!(level, NOTICE_PERCENT);
        let ids: Vec<_> = categories.iter().map(|c| (c.id, c.bytes)).collect();
        assert_eq!(
            ids,
            [
                ("files", 1_500_000_000),
                ("large_files", 432_735_283),
                ("journal", 1_840_612),
                ("recovery", 2588),
                ("other", 10),
            ]
        );
        assert_eq!(categories[1].objects, 103);
        // A host that sends the media and the total: they are pools too.
        let mut shared = u.clone();
        shared.media = Some(crate::api::UsageMedia {
            used: 500,
            limit: None,
            known: true,
            reconciled_at: None,
        });
        shared.total = Some(crate::api::UsageTotal {
            used: 1843200 + 1932735283 + 500,
            limit: Some(2147483648),
        });
        let (pools, level, _) = summarize(&shared);
        let names: Vec<_> = pools.iter().map(|p| p.pool.as_str()).collect();
        assert_eq!(
            names,
            ["files", "text", "total"],
            "media without a limit is no pool"
        );
        let total = pools.iter().find(|p| p.pool == "total").expect("total");
        assert_eq!((total.percent, total.level), (90, WARN_PERCENT));
        assert_eq!(level, WARN_PERCENT);
        // Without a limit the total is not a pool either.
        shared.total = Some(crate::api::UsageTotal {
            used: 5,
            limit: None,
        });
        assert_eq!(summarize(&shared).0.len(), 2);
    }

    #[test]
    fn the_levels_are_70_90_and_100() {
        assert_eq!(level_of(0, 100), 0);
        assert_eq!(level_of(69, 100), 0);
        assert_eq!(level_of(70, 100), NOTICE_PERCENT);
        assert_eq!(level_of(89, 100), NOTICE_PERCENT);
        assert_eq!(level_of(90, 100), WARN_PERCENT);
        assert_eq!(level_of(100, 100), FULL_PERCENT);
        assert_eq!(level_of(150, 100), FULL_PERCENT);
        assert_eq!(level_of(5, 0), 0, "no limit, never full");
        assert_eq!(level_of(u64::MAX, u64::MAX), FULL_PERCENT);
        let (pools, level, _) = summarize(&usage((10, 100), (2147483648, 2147483648)));
        assert_eq!(level, FULL_PERCENT);
        assert_eq!(
            pools.iter().find(|p| p.pool == "files").map(|p| p.percent),
            Some(100)
        );
        // The pools of the vault quiet, the media or the total full: the level is theirs.
        let mut quiet = usage((10, 100), (10, 100));
        quiet.media = Some(crate::api::UsageMedia {
            used: 95,
            limit: Some(100),
            known: true,
            reconciled_at: None,
        });
        assert_eq!(summarize(&quiet).1, WARN_PERCENT);
        quiet.total = Some(crate::api::UsageTotal {
            used: 115,
            limit: Some(100),
        });
        let (pools, level, _) = summarize(&quiet);
        assert_eq!(level, FULL_PERCENT);
        assert_eq!(
            pools.iter().find(|p| p.pool == "total").map(|p| p.percent),
            Some(100)
        );
    }
}
