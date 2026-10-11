// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The commands of the recovery of the account (internal/cloud-recovery.md
//! §3, §4, §7): the Recovery Key issued once and reissued, the restore of a
//! clean install from the 12 words, and the escrow kept current — written
//! again when the lock or the binding of sync changed ([`watch`]) and
//! compared by `HEAD` once a day ([`start_daily`]). The flows are `reissue`,
//! `restore` and `escrow`; this module binds them to the app — the lock,
//! the data file, the sync of the product and the seams of the messenger —
//! and to the screens. One lane ([`Lane`]) keeps a write of the escrow and
//! a reissue from running at once.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use veydan_core::{BoxFuture, Core};
use veydan_lock::{Event, Lock};
use veydan_recovery::{Storage, SyncKeys};
use veydan_shell::seams::{code, CloudError, LockBox, SeamFuture, Signer, VaultHosts};
use veydan_sync::{CloudStorage, Engine, RequestSigner, SignFuture, SyncError, Vmk};
use veydan_sync_host::{config, Host, SyncManager};

use crate::escrow::{self, HttpEscrowStore, Sources, BOX};
use crate::reissue::{self, Env, Issued, NoShares};
use crate::link::VaultEntry;
use crate::restore::{self, adopt_and_bind, NewSecret, Phase, RestoreEnv, Restored, Vault};

/// How often the daily check looks whether a day passed.
const LOOK_EVERY: Duration = Duration::from_secs(3600);
/// How often the escrow in vstore is compared with the last write (§4).
const CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);

/// The one lane of the escrow: a reissue, a write after a change and the
/// daily check take it in turn, so that a write under the old key never
/// lands after the reissue deleted the old escrow.
#[derive(Default)]
pub struct Lane(tokio::sync::Mutex<()>);

/// The lane of the app; a mock app without one is not guarded.
async fn take_lane<R: Runtime>(app: &AppHandle<R>) -> Option<tokio::sync::MutexGuard<'_, ()>> {
    let lane = app.try_state::<Lane>()?;
    Some(lane.inner().0.lock().await)
}

/// The event of the progress of a restore, with a [`RestoreProgress`].
pub const EVENT_RESTORE: &str = "cloud://restore";

/// What the screen of a restore hears.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RestoreProgress {
    pub phase: Phase,
}

/// The Recovery Key as a command hands it out, once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecoveryKeyIssued {
    /// The 12 words, one space between.
    pub words: String,
    pub generation: u32,
}

impl From<Issued> for RecoveryKeyIssued {
    fn from(issued: Issued) -> Self {
        Self {
            words: issued.words.to_string(),
            generation: issued.generation,
        }
    }
}

/// What the card «Восстановление» shows of the Recovery Key on this device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecoveryKeyStatus {
    /// A key of the cloud is issued on this device: the escrow is written.
    pub issued: bool,
    pub generation: u32,
    /// Unix seconds of the last write of the escrow by this device.
    pub escrow_written_at: Option<u64>,
    /// The second wrap of the lock: `1` a code, `2` the words, none without.
    pub lock_recovery_version: Option<u8>,
}

fn messenger_stopped(what: &str) -> CloudError {
    CloudError::new(
        code::MESSENGER_STOPPED,
        format!("the messenger gives no {what} in this product"),
    )
}

fn seam<S: ?Sized + Send + Sync + 'static, R: Runtime>(
    app: &AppHandle<R>,
    what: &str,
) -> Result<Arc<S>, CloudError> {
    veydan_shell::seam::<S, _, _>(app).ok_or_else(|| messenger_stopped(what))
}

fn lock_of<R: Runtime>(app: &AppHandle<R>) -> Result<tauri::State<'_, Lock>, CloudError> {
    app.try_state::<Lock>()
        .ok_or_else(|| CloudError::new(code::LOCKED, "the app has no lock in place"))
}

fn core_of<R: Runtime>(app: &AppHandle<R>) -> Result<tauri::State<'_, Core>, CloudError> {
    app.try_state::<Core>()
        .ok_or_else(|| CloudError::new(code::STORAGE, "the data file is not open"))
}

/// The store of the escrow over the seams of the messenger.
fn store_of<R: Runtime>(app: &AppHandle<R>) -> Result<HttpEscrowStore, CloudError> {
    Ok(HttpEscrowStore::new(
        seam::<dyn VaultHosts, _>(app, "hosts of the vault")?,
        seam::<dyn Signer, _>(app, "signer")?,
    ))
}

/// The box `cloud` of the lock.
fn box_of<R: Runtime>(app: &AppHandle<R>) -> Arc<dyn LockBox> {
    veydan_shell::seams::lock_box(app, BOX)
}

/// An issue of the Recovery Key: the first, or a new one. A reissue that
/// was interrupted is rolled back first (see `reissue`).
async fn issue<R: Runtime>(
    app: &AppHandle<R>,
    current: Option<String>,
    first: bool,
) -> Result<RecoveryKeyIssued, CloudError> {
    let lock = lock_of(app)?;
    let core = core_of(app)?;
    let signer = seam::<dyn Signer, _>(app, "signer")?;
    let store = store_of(app)?;
    let boxed = box_of(app);
    // Step 5 and the revoke of step 7: the trusted people of this device
    // (social.rs); a product without the seam of the notes has none.
    let shares: Box<dyn reissue::ShareDelivery> = match crate::social::social_of(app).await {
        Ok(social) => Box::new(social),
        Err(_) => Box::new(NoShares),
    };
    let env = Env {
        sources: Sources {
            lock: lock.inner(),
            db: &core.db,
            signer: signer.as_ref(),
        },
        store: &store,
        boxed: boxed.as_ref(),
        shares: shares.as_ref(),
    };
    let _lane = take_lane(app).await;
    reissue::resume(&env).await?;
    let current = current.unwrap_or_default();
    let issued = if first {
        reissue::first_setup(&env, &current).await?
    } else {
        reissue::reissue(&env, &current).await?
    };
    Ok(issued.into())
}

/// The first setup of the recovery of the cloud on this device: a Recovery
/// Key of 12 words, the second wrap of the lock under it, the escrow in
/// vstore; the words come back once. `current` is the secret of the lock.
#[tauri::command]
pub async fn cloud_recovery_key_show_once<R: Runtime>(
    current: Option<String>,
    app: AppHandle<R>,
) -> Result<RecoveryKeyIssued, CloudError> {
    issue(&app, current, true).await
}

/// A new Recovery Key in place of the one in force, all or nothing (§3).
#[tauri::command]
pub async fn cloud_recovery_key_reissue<R: Runtime>(
    current: Option<String>,
    app: AppHandle<R>,
) -> Result<RecoveryKeyIssued, CloudError> {
    issue(&app, current, false).await
}

/// What this device knows of the Recovery Key of the cloud.
#[tauri::command]
pub async fn cloud_recovery_status<R: Runtime>(
    app: AppHandle<R>,
) -> Result<RecoveryKeyStatus, CloudError> {
    let boxed = box_of(&app);
    let lock_recovery_version = match app.try_state::<Lock>() {
        Some(lock) => lock.status().await.recovery_version,
        None => None,
    };
    let state = escrow::load_state(boxed.as_ref()).await?;
    Ok(RecoveryKeyStatus {
        issued: escrow::load_rk(boxed.as_ref()).await?.is_some(),
        generation: escrow::load_generation(boxed.as_ref()).await?,
        escrow_written_at: state.map(|s| s.written_at),
        lock_recovery_version,
    })
}

/// [`RequestSigner`] of the backend of the vault over a [`Signer`]: the
/// one in memory during a restore.
struct SignerRequests(Arc<dyn Signer>);

impl RequestSigner for SignerRequests {
    fn authorization<'a>(
        &'a self,
        method: &'a str,
        url: &'a str,
        payload_sha256: Option<[u8; 32]>,
        created_at: u64,
    ) -> SignFuture<'a> {
        Box::pin(async move {
            let event = self
                .0
                .nip98_at(method, url, payload_sha256, created_at)
                .await
                .map_err(|e| SyncError::Auth(e.to_string()))?;
            crate::nip98::authorization(&event).map_err(|e| SyncError::Crypto(e.to_string()))
        })
    }
}

/// [`RestoreEnv`] of the running app.
struct AppRestoreEnv {
    app: AppHandle,
}

impl AppRestoreEnv {
    /// The hosts of `storage[]` of the manifest, for the backend `cloud`.
    async fn vault_hosts(&self) -> Result<Vec<String>, CloudError> {
        Ok(
            seam::<dyn VaultHosts, _>(&self.app, "hosts of the vault")?
                .storage_hosts()
                .await?
                .into_iter()
                .map(|h| h.url)
                .collect(),
        )
    }
}

/// A storage of the user's own that does not answer, is not on this device
/// or holds another vault (§7).
fn unreachable(e: impl std::fmt::Display) -> CloudError {
    CloudError::new(escrow::words::VAULT_UNREACHABLE, e.to_string())
}

/// [`VaultEntry`] of the running app: the vault of this device for a link
/// (§6) — its keys and storage on the old device, the entry of §7 on the
/// new one.
pub struct AppVault {
    app: AppHandle,
}

impl AppVault {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl VaultEntry for AppVault {
    fn keys(&self) -> SeamFuture<'_, Option<SyncKeys>> {
        Box::pin(async move { escrow::sync_keys(&core_of(&self.app)?.db).await })
    }

    fn enter<'a>(
        &'a self,
        nsec: &'a zeroize::Zeroizing<[u8; 32]>,
        sync: &'a SyncKeys,
        password: Option<&'a str>,
    ) -> SeamFuture<'a, ()> {
        Box::pin(async move {
            let env = AppRestoreEnv {
                app: self.app.clone(),
            };
            let _lane = take_lane(&self.app).await;
            restore::enter(&env, nsec, sync, password).await
        })
    }
}

impl RestoreEnv for AppRestoreEnv {
    fn lock(&self) -> &Lock {
        self.app.state::<Lock>().inner()
    }

    fn db(&self) -> &sqlx::Pool<sqlx::Sqlite> {
        &self.app.state::<Core>().inner().db
    }

    fn has_identity(&self) -> BoxFuture<'_, Result<bool, CloudError>> {
        Box::pin(async move {
            match seam::<dyn Signer, _>(&self.app, "signer")?.pubkey().await {
                Ok(_) => Ok(true),
                Err(e) if e.is(code::NO_IDENTITY) => Ok(false),
                Err(e) => Err(e),
            }
        })
    }

    fn open_vault<'a>(
        &'a self,
        signer: Arc<dyn Signer>,
        sync: &'a SyncKeys,
    ) -> BoxFuture<'a, Result<Engine, CloudError>> {
        Box::pin(async move {
            let device = config::device_id(self.db())
                .await
                .map_err(restore::app_error)?;
            let storage: Box<dyn veydan_sync::Storage> = match &sync.storage {
                Storage::Cloud => {
                    let hosts = self.vault_hosts().await?;
                    let storage = CloudStorage::new(&hosts, Arc::new(SignerRequests(signer)))
                        .map_err(|e| CloudError::new(code::NO_HOST, e.to_string()))?;
                    Box::new(storage)
                }
                own => {
                    let mut cfg = config::load_config(self.db()).await;
                    escrow::apply_storage(&mut cfg, own);
                    config::build_storage(&cfg).map_err(unreachable)?
                }
            };
            let engine = Engine::with_key(storage, &Vmk(sync.vmk), &sync.vault_id, &device);
            // A storage of the user's own is found by what the escrow or the
            // link says of it: it must answer and hold this very vault, or
            // the device would be bound to an empty place and call it joined.
            if sync.storage != Storage::Cloud {
                engine.verify_manifest().await.map_err(unreachable)?;
            }
            Ok(engine)
        })
    }

    fn adopt_vault_key<'a>(
        &'a self,
        engine: &'a Engine,
        sync: &'a SyncKeys,
    ) -> BoxFuture<'a, Result<bool, CloudError>> {
        Box::pin(async move {
            let core = core_of(&self.app)?;
            let manager = self.app.try_state::<SyncManager>().ok_or_else(|| {
                CloudError::new(code::STORAGE, "the sync of the product is not in place")
            })?;
            // The settings of sync take the storage of the vault before the
            // row is adopted, or the device would stand «joined» on a folder
            // that is not set (seen in E1).
            let mut cfg = config::load_config(&core.db).await;
            escrow::apply_storage(&mut cfg, &sync.storage);
            cfg.enabled = true;
            if sync.storage == Storage::Cloud {
                // The files of profiles stay off on Veydan Cloud until the
                // person says yes (plan 4.7).
                cfg.profile_files = false;
                let hosts = self.vault_hosts().await?;
                config::save_cloud_hosts(&core.db, &hosts)
                    .await
                    .map_err(restore::app_error)?;
            }
            config::save_config(&core.db, &cfg)
                .await
                .map_err(restore::app_error)?;
            let vmk = Vmk(sync.vmk);
            adopt_and_bind(&Host::App(&self.app), engine, &core, manager.registry(), &vmk)
                .await
                .map_err(restore::app_error)
        })
    }

    fn import_identity<'a>(&'a self, nsec: &'a [u8; 32]) -> BoxFuture<'a, Result<(), CloudError>> {
        Box::pin(async move {
            seam::<dyn Signer, _>(&self.app, "signer")?
                .import_for_restore(nsec)
                .await
        })
    }
}

/// Scenario 3 (§7): the 12 words of a clean install, the escrow, the six
/// steps in their order, a new secret of the lock. The progress goes to
/// the screen as [`EVENT_RESTORE`]. `without_vault` is the person's answer
/// to `vault_unreachable`: the identity and the lock without the storage.
/// Afterwards the escrow is written again when the lock was set up anew
/// around a vault that was joined, so it carries the key in use; whether
/// that worked goes back to the screen as `escrow_refreshed`.
#[tauri::command]
pub async fn cloud_restore_with_words(
    words: String,
    password: String,
    kind: Option<String>,
    hint: Option<String>,
    without_vault: Option<bool>,
    app: AppHandle,
) -> Result<Restored, CloudError> {
    let store = store_of(&app)?;
    let boxed = box_of(&app);
    let env = AppRestoreEnv { app: app.clone() };
    let progress = {
        let app = app.clone();
        move |phase: Phase| {
            let _ = app.emit(EVENT_RESTORE, RestoreProgress { phase });
        }
    };
    let lane = take_lane(&app).await;
    let mut restored = restore::restore(
        &env,
        &store,
        boxed.as_ref(),
        &words,
        NewSecret {
            password,
            kind,
            hint,
        },
        Vault::of(without_vault),
        &progress,
    )
    .await?;
    drop(lane);
    // Without the vault there is no binding to write of: the escrow in
    // vstore stands as it was read, and names the vault still.
    if restored.lock == restore::LockOutcome::Reinstalled && restored.vault_joined {
        restored.escrow_refreshed = Some(match refresh_escrow(&app).await {
            Ok(_) => true,
            Err(e) => {
                eprintln!("cloud: the escrow was not written after the restore: {e}");
                false
            }
        });
    }
    Ok(restored)
}

/// Runs `work` with the sources of the escrow of this device: the lock,
/// the data file, the signer of the messenger, the store and the box.
async fn with_sources<R, T, F>(app: &AppHandle<R>, work: F) -> Result<T, CloudError>
where
    R: Runtime,
    F: for<'a> FnOnce(
        &'a Sources<'a>,
        &'a HttpEscrowStore,
        &'a dyn LockBox,
    ) -> BoxFuture<'a, Result<T, CloudError>>,
{
    let boxed = box_of(app);
    let lock = lock_of(app)?;
    let core = core_of(app)?;
    let signer = seam::<dyn Signer, _>(app, "signer")?;
    let store = store_of(app)?;
    let sources = Sources {
        lock: lock.inner(),
        db: &core.db,
        signer: signer.as_ref(),
    };
    let _lane = take_lane(app).await;
    work(&sources, &store, boxed.as_ref()).await
}

/// Writes the escrow again when a field changed since the last write by
/// this device (§4); nothing when the recovery is not set up here.
pub async fn refresh_escrow<R: Runtime>(app: &AppHandle<R>) -> Result<bool, CloudError> {
    with_sources(app, |sources, store, boxed| {
        Box::pin(escrow::refresh(sources, store, boxed, false))
    })
    .await
}

/// The daily check of §4: the object in vstore compared by `HEAD` with the
/// last write of this device, written again when it fell behind.
pub async fn check_escrow<R: Runtime>(app: &AppHandle<R>) -> Result<bool, CloudError> {
    with_sources(app, |sources, store, boxed| {
        Box::pin(escrow::check(sources, store, boxed))
    })
    .await
}

/// Whether a refusal of a write of the escrow is the ordinary state of a
/// device — the lock closed, no identity, no vault, the messenger off —
/// and not worth a line.
fn routine(e: &CloudError) -> bool {
    [
        code::LOCKED,
        code::NO_IDENTITY,
        code::MESSENGER_STOPPED,
        code::NO_HOST,
        escrow::words::NO_SYNC_VAULT,
    ]
    .iter()
    .any(|word| e.is(word))
}

/// Writes the escrow again in the background, after a change.
fn refresh_later(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = refresh_escrow(&app).await {
            if !routine(&e) {
                eprintln!("cloud: the escrow was not written after a change: {e}");
            }
        }
    });
}

/// §4, «перезаписывается при любом изменении поля»: the module hears the
/// lock (`Unlocked` after `set`, `Reset` after `reset` and `wipe`,
/// `Changed` after a synced row or a stored wrap), the binding of sync
/// (saved or cleared) and its settings (where the vault lives), and writes
/// the escrow again when a field changed.
/// Called in the setup of the module; the lane is put in place here.
pub fn watch(app: AppHandle) {
    app.manage(Lane::default());
    let on_binding = app.clone();
    config::on_binding_changed(Arc::new(move || refresh_later(on_binding.clone())));
    let Some(lock) = app.try_state::<Lock>() else {
        return;
    };
    let mut events = lock.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(Event::Unlocked | Event::Changed | Event::Reset) => refresh_later(app.clone()),
                Ok(Event::Locked) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    refresh_later(app.clone())
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// Once a day (first an hour after the start): the escrow in vstore is
/// compared by `HEAD` with the last write of this device (§4).
pub fn start_daily(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut checked: Option<Instant> = None;
        loop {
            tokio::time::sleep(LOOK_EVERY).await;
            if checked.is_some_and(|at| at.elapsed() < CHECK_EVERY) {
                continue;
            }
            match check_escrow(&app).await {
                Ok(_) => checked = Some(Instant::now()),
                Err(e) => {
                    if !routine(&e) {
                        eprintln!("cloud: the daily check of the escrow: {e}");
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A product whose messenger offers no seams: the commands say so, and
    /// the status of a device without a lock is empty.
    #[tokio::test]
    async fn without_the_seams_of_the_messenger_the_commands_say_so() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let e = cloud_recovery_key_show_once(Some("4821".into()), handle.clone())
            .await
            .err()
            .map(|e| e.code);
        assert_eq!(e.as_deref(), Some(code::LOCKED), "no lock in the mock app");
        let e = cloud_recovery_key_reissue(None, handle.clone())
            .await
            .err()
            .map(|e| e.code);
        assert_eq!(e.as_deref(), Some(code::LOCKED));
        let e = cloud_recovery_status(handle.clone()).await.err().map(|e| e.code);
        assert_eq!(e.as_deref(), Some(code::LOCKED), "the box needs the lock");
        let e = refresh_escrow(&handle).await.err().map(|e| e.code);
        assert_eq!(e.as_deref(), Some(code::LOCKED));
        assert!(routine(&CloudError::new(code::LOCKED, "")));
        assert!(!routine(&CloudError::new(code::STORAGE, "")));
        let serialized = serde_json::to_value(RecoveryKeyIssued {
            words: "a b".into(),
            generation: 1,
        })
        .expect("json");
        assert_eq!(serialized["generation"], 1);
    }
}
