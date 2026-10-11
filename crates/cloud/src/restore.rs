// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Scenario 3 of internal/cloud-recovery.md §7: a clean install and the 12
//! words. The escrow is read from vstore with the key `auth` of the words
//! and opened in memory; then the device is put together in the one order
//! that does not leave the messenger in `SecretsLocked`:
//!
//! 1. the escrow is open, every field in memory, nothing written yet;
//! 2. a signer in memory over the nsec of the escrow — the backend of the
//!    vault needs NIP-98 before the nsec may go into the lock;
//! 3. `Engine::with_key` — the vault open with the key of the escrow;
//! 4. `adopt_vault_key` — the key row of the vault takes the place of the
//!    row of this device, and the device is bound;
//! 5. `Lock::recover_keep` — the row came with the forgotten password: the
//!    words open its second wrap, the new password wraps the key;
//! 6. only now the nsec goes into the box of the lock as the identity.
//!
//! Why: an identity imported first makes a key row of its own under the
//! built-in secret; the row of the vault replaces it, its password is
//! unknown, the box never opens, and `HostSecretStore::get` answers
//! `SecretsLocked`. The integration test of the crate runs both orders.
//!
//! The scenario is a clean install (§7): a device that has an identity, a
//! binding to a vault or a lock is refused before any request
//! ([`ensure_clean`]) — steps 3 to 5 would otherwise replace its key row
//! and leave its boxes unreadable on a live account.
//!
//! The vault is the one the escrow names, where it lives (§4): Veydan
//! Cloud, or the user's own S3, WebDAV or folder, set up on this device as
//! it was on the one that wrote the escrow. A storage that does not answer
//! or is not here (a folder of another computer, any folder on a phone) is
//! [`words::VAULT_UNREACHABLE`]; the person may then restore without it
//! ([`Vault::Skip`]): the identity and the lock come back, the vault is
//! joined from the settings of sync.
//!
//! A linked device (§6) enters the same way with the secret of the lock in
//! place of the words: [`enter`].

use std::sync::Arc;

use serde::Serialize;
use sqlx::{Pool, Sqlite};
use veydan_core::{settings, AppError, BoxFuture, Core};
use veydan_lock::{Lock, SecretKey};
use veydan_recovery::{LockKey, RecoveryKey, RecoveryParams, SyncKeys};
use veydan_shell::seams::{code, CloudError, LockBox, SeamFuture, Signer, TempKey, WireEvent};
use veydan_sync::{Engine, Vmk};
use veydan_sync_host::join::{self, JoinKey};
use veydan_sync_host::{config, state, Host, Registry};
use zeroize::Zeroizing;

use crate::api::now;
use crate::escrow::{self, words, EscrowState, EscrowStore};
use crate::nip98;

/// A [`Signer`] over an nsec in memory (step 2): NIP-98 and nothing else.
/// Dropped with the restore; the nsec is wiped with it.
pub struct MemorySigner {
    key: TempKey,
    pubkey: String,
}

impl MemorySigner {
    /// Over the 32 bytes of an nsec; refused when they are not a key.
    pub fn new(nsec: Zeroizing<[u8; 32]>) -> Result<Self, CloudError> {
        let key = TempKey::new(nsec);
        let pubkey = nip98::temp_pub(&key)?;
        Ok(Self { key, pubkey })
    }
}

fn not_supported<'a, T: Send + 'a>(what: &'static str) -> SeamFuture<'a, T> {
    Box::pin(async move {
        Err(CloudError::new(
            code::NOT_SUPPORTED,
            format!("a signer in memory does not do {what}"),
        ))
    })
}

impl Signer for MemorySigner {
    fn pubkey(&self) -> SeamFuture<'_, String> {
        Box::pin(async move { Ok(self.pubkey.clone()) })
    }

    fn nip98<'a>(
        &'a self,
        method: &'a str,
        url: &'a str,
        payload_sha256: Option<[u8; 32]>,
    ) -> SeamFuture<'a, WireEvent> {
        Box::pin(async move { nip98::event_by(&self.key, method, url, payload_sha256, now()?) })
    }

    fn nip44_encrypt<'a>(&'a self, _to: &'a str, _plaintext: &'a [u8]) -> SeamFuture<'a, Vec<u8>> {
        not_supported("nip44")
    }

    fn nip44_decrypt<'a>(
        &'a self,
        _from: &'a str,
        _ciphertext: &'a [u8],
    ) -> SeamFuture<'a, Zeroizing<Vec<u8>>> {
        not_supported("nip44")
    }

    fn nip98_at<'a>(
        &'a self,
        method: &'a str,
        url: &'a str,
        payload_sha256: Option<[u8; 32]>,
        created_at: u64,
    ) -> SeamFuture<'a, WireEvent> {
        Box::pin(async move { nip98::event_by(&self.key, method, url, payload_sha256, created_at) })
    }

    fn export_for_escrow(&self) -> SeamFuture<'_, Zeroizing<[u8; 32]>> {
        Box::pin(async move { Ok(Zeroizing::new(*self.key.secret_bytes())) })
    }
}

/// Where a restore stands; the screen shows it (`cloud://restore`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// The escrow is asked for with the key of the words.
    Reading,
    /// The escrow is opened (Argon2id, about a second).
    Opening,
    /// The vault is opened with the key of the escrow.
    Vault,
    /// The key row of the vault takes the place of the row of this device.
    KeyRow,
    /// The lock takes the new password.
    Lock,
    /// The identity goes into the box of the lock.
    Identity,
    Done,
}

/// What became of the lock at step 5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LockOutcome {
    /// The row of the vault opened with the words and took the new password.
    Kept,
    /// The row of the vault was another one (a `reset` after the escrow was
    /// written): the lock was set up anew with the new password, its second
    /// wrap under the same words; the boxes of this device were empty.
    Reinstalled,
    /// No lock on the account: the vault opens by itself.
    NoLock,
}

/// What a restore does about the vault of sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vault {
    /// Steps 3 and 4: the vault the escrow names is opened and joined.
    Join,
    /// The person chose to go on without a storage that is not reachable:
    /// the identity and the lock alone.
    Skip,
}

impl Vault {
    /// From the flag of a command: `without_vault`.
    pub fn of(without: Option<bool>) -> Self {
        if without == Some(true) {
            Vault::Skip
        } else {
            Vault::Join
        }
    }
}

/// What a restore ends with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Restored {
    /// The owner, 64 hex.
    pub npub: String,
    pub lock: LockOutcome,
    /// Whether the vault of sync was joined; false after [`Vault::Skip`].
    pub vault_joined: bool,
    /// The trusted people of the account, as the escrow knew them.
    pub k: u8,
    pub n: u8,
    pub generation: u32,
    /// After [`LockOutcome::Reinstalled`]: whether the escrow was written
    /// again so that it carries the key in use (§4); `false` tells the
    /// screen to say so. None when there was nothing to write again.
    pub escrow_refreshed: Option<bool>,
}

/// The new secret of the lock a restore sets (step 5).
#[derive(Debug, Clone)]
pub struct NewSecret {
    pub password: String,
    /// `pin` | `password`; `None` is `password`.
    pub kind: Option<String>,
    pub hint: Option<String>,
}

/// What a restore needs of the device: the lock, the data file, the vault
/// and the identity — the app's, or a test's over a folder.
pub trait RestoreEnv: Send + Sync {
    fn lock(&self) -> &Lock;
    fn db(&self) -> &Pool<Sqlite>;
    /// Whether the messenger has an identity on this device: a restore is
    /// refused then (`identity_exists`).
    fn has_identity(&self) -> BoxFuture<'_, Result<bool, CloudError>>;
    /// Step 3: the vault `sync` opened with its key where it lives,
    /// requests to Veydan Cloud signed by `signer` (the nsec of the escrow,
    /// in memory); [`words::VAULT_UNREACHABLE`] when a storage of the
    /// user's own does not answer or does not hold this vault.
    fn open_vault<'a>(
        &'a self,
        signer: Arc<dyn Signer>,
        sync: &'a SyncKeys,
    ) -> BoxFuture<'a, Result<Engine, CloudError>>;
    /// Step 4: the settings of sync take the storage of `sync`, then
    /// [`adopt_and_bind`] with the host of the app or of the test. Whether
    /// the row of the vault took the place of the local one.
    fn adopt_vault_key<'a>(
        &'a self,
        engine: &'a Engine,
        sync: &'a SyncKeys,
    ) -> BoxFuture<'a, Result<bool, CloudError>>;
    /// Step 6: the nsec becomes the identity of the messenger.
    fn import_identity<'a>(&'a self, nsec: &'a [u8; 32]) -> BoxFuture<'a, Result<(), CloudError>>;
}

/// The lock's error in the words of the seams.
pub fn lock_error(e: veydan_lock::Error) -> CloudError {
    match e {
        veydan_lock::Error::VaultLocked | veydan_lock::Error::VaultMismatch => {
            CloudError::new(code::LOCKED, "the lock is closed")
        }
        veydan_lock::Error::RecoveryInvalid => {
            CloudError::new(words::ESCROW_INVALID, "the words do not open the lock")
        }
        veydan_lock::Error::Db(m) => CloudError::new(code::STORAGE, m),
        wrong if wrong.is_wrong_secret() => {
            CloudError::new(code::WRONG_PASSWORD, wrong.to_string())
        }
        other => CloudError::new(code::BAD_REQUEST, other.to_string()),
    }
}

/// An error of sync-host in the words of the seams.
pub fn app_error(e: AppError) -> CloudError {
    CloudError::new(code::STORAGE, e.to_string())
}

/// §7, «чистая установка»: nothing of another account may be on the
/// device. An identity is `identity_exists`; a binding to a vault or a
/// lock in place is [`words::NOT_CLEAN`]. A key row without a lock (the
/// built-in secret) is what a clean install has and passes.
pub async fn ensure_clean(env: &dyn RestoreEnv) -> Result<(), CloudError> {
    if env.has_identity().await? {
        return Err(CloudError::new(
            code::IDENTITY_EXISTS,
            "another identity is on this device; a restore needs a clean install",
        ));
    }
    if config::load_binding(env.db()).await.is_some() {
        return Err(CloudError::new(
            words::NOT_CLEAN,
            "this device is bound to a sync vault; a restore needs a clean install",
        ));
    }
    let lock = env.lock();
    if veydan_lock::key_id(env.db()).await.map_err(lock_error)?.is_some() && lock.enabled().await {
        return Err(CloudError::new(
            words::NOT_CLEAN,
            "this device has a lock; a restore needs a clean install",
        ));
    }
    Ok(())
}

/// Step 4 as `sync_join_vault` does it, with the vault already open: the
/// row of the vault takes the place of the local one (or the local one
/// stands, see `join::adopt_vault_key`), the device is bound, the state of
/// the row is saved and the lock hears of the change. A device without a
/// key row makes one first, so the vault is read now and not at a later
/// cycle: the restore needs the row at step 5.
pub async fn adopt_and_bind(
    host: &Host<'_>,
    engine: &Engine,
    core: &Core,
    registry: &Registry,
    vmk: &Vmk,
) -> Result<bool, AppError> {
    if let Some(lock) = host.state::<Lock>() {
        if veydan_lock::key_id(&core.db).await?.is_none() {
            lock.ensure_key().await?;
        }
    }
    let key_row = join::adopt_vault_key(host, engine, core, registry, true).await?;
    join::bind(&core.db, registry, engine.vault_id(), vmk).await?;
    match &key_row {
        JoinKey::Vaults(st) => state::save_row_state(&core.db, st).await?,
        JoinKey::Unread => settings::set(&core.db, config::JOIN_PENDING, "1").await?,
        JoinKey::Own => {}
    }
    let replaced = matches!(key_row, JoinKey::Vaults(_));
    if replaced {
        join::key_row_changed(host).await;
    }
    Ok(replaced)
}

/// Step 5 (§7, «строка не совпала»): the row the vault brought opens with
/// the words and takes the new password; a row that does not open (another
/// `vault_id` after a `reset`, a reissue the escrow did not follow, a wrap
/// of version 1) gives way to a lock set up anew with its second wrap under
/// the same words — around the key the escrow carries (`escrow.lock`, the
/// insurance of §4: what the vault holds encrypted under it still reads),
/// or around a fresh key when the escrow carries none; no lock stays no
/// lock.
async fn settle_lock(
    lock: &Lock,
    rk: &RecoveryKey,
    rk_words: &str,
    escrow_lock: Option<&LockKey>,
    secret: &NewSecret,
) -> Result<LockOutcome, CloudError> {
    if !lock.enabled().await {
        lock.open_default().await.map_err(lock_error)?;
        return Ok(LockOutcome::NoLock);
    }
    match lock
        .recover_keep(
            rk_words,
            secret.password.clone(),
            secret.kind.clone(),
            secret.hint.clone(),
        )
        .await
    {
        Ok(()) => Ok(LockOutcome::Kept),
        Err(veydan_lock::Error::RecoveryInvalid) => {
            // The boxes of a clean install are empty: nothing is lost with the row.
            match escrow_lock {
                Some(key) => {
                    lock.install_key(
                        SecretKey::from_bytes(&key.vault_key),
                        &key.vault_id,
                        rk.as_bytes(),
                        secret.password.clone(),
                        secret.kind.clone(),
                        secret.hint.clone(),
                    )
                    .await
                    .map_err(lock_error)?;
                }
                None => {
                    lock.wipe().await.map_err(lock_error)?;
                    lock.set(
                        Some(secret.password.clone()),
                        None,
                        secret.kind.clone(),
                        secret.hint.clone(),
                    )
                    .await
                    .map_err(lock_error)?;
                    let prepared = lock
                        .recovery_prepare_v2(&secret.password, rk.as_bytes())
                        .await
                        .map_err(lock_error)?;
                    lock.recovery_store_v2(&prepared).await.map_err(lock_error)?;
                }
            }
            Ok(LockOutcome::Reinstalled)
        }
        Err(e) => Err(lock_error(e)),
    }
}

/// Step 5 without the vault ([`Vault::Skip`]): no key row came, so the
/// lock of the account is set up around the key the escrow carries, under
/// the new password and the words — the row the vault holds is the same
/// key, and joining the vault later replaces nothing; an account without
/// a lock stays without.
async fn lock_without_vault(
    lock: &Lock,
    rk: &RecoveryKey,
    escrow_lock: Option<&LockKey>,
    secret: &NewSecret,
) -> Result<LockOutcome, CloudError> {
    let Some(key) = escrow_lock else {
        lock.open_default().await.map_err(lock_error)?;
        return Ok(LockOutcome::NoLock);
    };
    lock.install_key(
        SecretKey::from_bytes(&key.vault_key),
        &key.vault_id,
        rk.as_bytes(),
        secret.password.clone(),
        secret.kind.clone(),
        secret.hint.clone(),
    )
    .await
    .map_err(lock_error)?;
    Ok(LockOutcome::Reinstalled)
}

/// Scenario 2 on the new device (§6, step 4): the vault of the account
/// entered in the order of §7 before the identity is imported — opened
/// with its key where it lives, its key row adopted, the device bound and
/// its settings of sync set as on the old device — and the lock opened
/// with `password`, the one of the old device (the row is the same). A
/// wrong secret is `wrong_password` and leaves the vault joined: the next
/// call goes straight to the lock.
pub async fn enter(
    env: &dyn RestoreEnv,
    nsec: &Zeroizing<[u8; 32]>,
    sync: &SyncKeys,
    password: Option<&str>,
) -> Result<(), CloudError> {
    match config::load_binding(env.db()).await {
        // An earlier try got as far as the lock.
        Some(bound) if bound.vault_id == sync.vault_id => {}
        Some(_) => {
            return Err(CloudError::new(
                words::NOT_CLEAN,
                "this device is bound to another sync vault",
            ))
        }
        None => {
            ensure_clean(env).await?;
            let signer: Arc<dyn Signer> = Arc::new(MemorySigner::new(nsec.clone())?);
            let engine = env.open_vault(signer, sync).await?;
            env.adopt_vault_key(&engine, sync).await?;
        }
    }
    let lock = env.lock();
    if !lock.enabled().await {
        return lock.open_default().await.map_err(lock_error);
    }
    if lock.is_locked().await {
        let secret = password.filter(|p| !p.is_empty()).ok_or_else(|| {
            CloudError::new(code::WRONG_PASSWORD, "the secret of the lock is needed")
        })?;
        if !lock.unlock(secret).await.map_err(lock_error)? {
            return Err(CloudError::new(
                code::WRONG_PASSWORD,
                "the secret is not the lock's",
            ));
        }
    }
    Ok(())
}

/// The restore, in the order of §7. `boxed` is the box `cloud` of the lock,
/// written once the lock is open: the words and the generation, and the
/// state of the escrow as read, so that a changed field is written again.
pub async fn restore(
    env: &dyn RestoreEnv,
    store: &dyn EscrowStore,
    boxed: &dyn LockBox,
    typed_words: &str,
    secret: NewSecret,
    vault: Vault,
    progress: &(dyn Fn(Phase) + Send + Sync),
) -> Result<Restored, CloudError> {
    progress(Phase::Reading);
    let rk = RecoveryKey::from_words(typed_words)
        .map_err(|e| CloudError::new(words::BAD_WORDS, e.to_string()))?;
    ensure_clean(env).await?;
    let rk_words = rk.words();
    let auth = rk
        .escrow_auth()
        .map_err(|e| CloudError::new(code::CRYPTO, e.to_string()))?;
    let bytes = store.get(&auth).await?;
    let sealed_sha256 = escrow::sha256_hex(&bytes);

    progress(Phase::Opening);
    // Step 1: every field in memory, wiped when this function returns.
    let escrow = escrow::open(&rk, bytes).await?;

    let lock = match vault {
        Vault::Join => {
            progress(Phase::Vault);
            // Step 2 and 3.
            let signer: Arc<dyn Signer> =
                Arc::new(MemorySigner::new(Zeroizing::new(escrow.nsec))?);
            let engine = env.open_vault(signer, &escrow.sync).await?;

            progress(Phase::KeyRow);
            // Step 4.
            env.adopt_vault_key(&engine, &escrow.sync).await?;

            progress(Phase::Lock);
            // Step 5.
            settle_lock(env.lock(), &rk, &rk_words, escrow.lock.as_ref(), &secret).await?
        }
        Vault::Skip => {
            progress(Phase::Lock);
            lock_without_vault(env.lock(), &rk, escrow.lock.as_ref(), &secret).await?
        }
    };

    progress(Phase::Identity);
    // Step 6.
    env.import_identity(&escrow.nsec).await?;

    escrow::save_rk(boxed, &rk, escrow.generation).await?;
    escrow::save_state(
        boxed,
        &EscrowState {
            address: auth.escrow_address(),
            fingerprint: escrow::fingerprint(&escrow),
            generation: escrow.generation,
            written_at: escrow.created_at,
            sealed_sha256: Some(sealed_sha256),
        },
    )
    .await?;
    progress(Phase::Done);
    Ok(Restored {
        npub: nip98::hex(&escrow.npub),
        lock,
        vault_joined: vault == Vault::Join,
        k: escrow.recovery.k,
        n: escrow.recovery.n,
        generation: escrow.generation,
        escrow_refreshed: None,
    })
}

/// The numbers of the trusted people a restore reports, for the screen.
pub fn recovery_of(restored: &Restored) -> RecoveryParams {
    RecoveryParams {
        k: restored.k,
        n: restored.n,
        generation: restored.generation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_signer_in_memory_signs_nip98_and_nothing_else() {
        let signer = MemorySigner::new(Zeroizing::new([0x42; 32])).expect("key");
        let pubkey = signer.pubkey().await.expect("pubkey");
        assert_eq!(pubkey.len(), 64);
        let event = signer
            .nip98("GET", "https://media-eu.example.com/vault/v1/escrow/ab", None)
            .await
            .expect("event");
        assert_eq!((event.kind, event.pubkey.as_str()), (27235, pubkey.as_str()));
        let at = signer
            .nip98_at("GET", "https://media-eu.example.com/vault/v1/escrow/ab", None, 7)
            .await
            .expect("event");
        assert_eq!(at.created_at, 7);
        let e = signer.nip44_encrypt(&pubkey, b"x").await.err().map(|e| e.code);
        assert_eq!(e.as_deref(), Some(code::NOT_SUPPORTED));
        let e = signer.import_for_restore(&[1; 32]).await.err().map(|e| e.code);
        assert_eq!(e.as_deref(), Some(code::NOT_SUPPORTED));
        assert_eq!(*signer.export_for_escrow().await.expect("nsec"), [0x42; 32]);
        assert!(MemorySigner::new(Zeroizing::new([0; 32])).is_err());
    }

    #[test]
    fn the_errors_of_the_lock_take_the_words_of_the_seams() {
        assert!(lock_error(veydan_lock::Error::VaultLocked).is(code::LOCKED));
        assert!(lock_error(veydan_lock::Error::RecoveryInvalid).is(words::ESCROW_INVALID));
        assert!(lock_error(veydan_lock::Error::Db("x".into())).is(code::STORAGE));
        assert!(lock_error(veydan_lock::Error::other("short")).is(code::BAD_REQUEST));
        assert!(lock_error(veydan_lock::Error::other(veydan_lock::ERR_WRONG)).is(code::WRONG_PASSWORD));
        assert!(lock_error(veydan_lock::Error::other(veydan_lock::ERR_CURRENT)).is(code::WRONG_PASSWORD));
        let phase = serde_json::to_value(Phase::KeyRow).expect("json");
        assert_eq!(phase, "key_row");
    }
}
