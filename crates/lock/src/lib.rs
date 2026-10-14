// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The lock of the app and the key of the vault it guards.
//!
//! One random key — the vault key — encrypts what is kept behind the lock:
//! the fields of passwords and the values of the secret boxes. The only row
//! of `password_vault` holds the key wrapped by the PIN or password of the
//! lock (by a built-in secret while there is none) and, optionally, by a
//! recovery code, and next to the wrap the parameters of the lock: hash, kind,
//! hint. The row is synced, so devices that share a sync vault share the
//! lock; nothing of the lock is kept anywhere else.
//!
//! The crate knows neither Tauri nor the core. It is handed the pool of the
//! data file, gives its schema steps to whoever opens the file, and tells
//! what happens through a stream of [`Event`]s.

mod crypto;
mod error;
mod legacy;
mod recovery;
mod secrets;
mod session;
mod state;
mod vault;

pub use crypto::SecretKey;
pub use error::{Error, Result};
pub use secrets::SecretBox;
pub use vault::{
    decrypt_field, encrypt_field, key_id, recovery_wrap, store_lock_meta, synced_lock, LockMeta,
    SyncedLock, DEFAULT_SECRET, ERR_JOIN_OWN_LOCK, ROW_ID,
};

use argon2::password_hash::{phc::PasswordHash, PasswordHasher, PasswordVerifier};
use argon2::Argon2;
use serde::Serialize;
use session::Session;
use sqlx::{Pool, Sqlite, SqliteConnection};
use state::VaultState;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use tokio::sync::broadcast;

/// The schema of the lock's tables, step by step: `steps[i]` takes the
/// schema from version i to i+1. Whoever opens the data file names them
/// under the module `lock`.
pub const SCHEMA_STEPS: &[&str] = &[
    // `recovery_*` is the second wrap of the vault key, under the recovery
    // code. The lock hash travels with the wrap it matches, so peers never see
    // one without the other.
    "CREATE TABLE password_vault (
            id                   TEXT PRIMARY KEY NOT NULL,
            vault_id             TEXT NOT NULL,
            crypto_version       INTEGER NOT NULL DEFAULT 1,
            kdf_algorithm        TEXT NOT NULL DEFAULT 'argon2id',
            kdf_salt             TEXT NOT NULL,
            kdf_memory           INTEGER NOT NULL,
            kdf_iterations       INTEGER NOT NULL,
            kdf_parallelism      INTEGER NOT NULL,
            wrapped_key          TEXT NOT NULL,
            created_at           TEXT NOT NULL,
            updated_at           TEXT NOT NULL,
            recovery_salt        TEXT,
            recovery_wrapped_key TEXT,
            lock_hash            TEXT,
            lock_kind            TEXT,
            lock_hint            TEXT
        )",
    // The values of the secret boxes, under the vault key. Local to the device.
    "CREATE TABLE lock_secrets (
            box   TEXT NOT NULL,
            key   TEXT NOT NULL,
            value TEXT NOT NULL,
            PRIMARY KEY (box, key)
        )",
    // What the second wrap is made from: 1 a code of 24 chars, 2 the
    // Recovery Key of 12 words (platform-spec 8.1). Synced with the wrap; a
    // row from a client without the column keeps the default.
    "ALTER TABLE password_vault ADD COLUMN recovery_version INTEGER NOT NULL DEFAULT 1",
];

/// A table as sync carries it: the entity type, the table and its key, the
/// columns of the payload.
pub struct SyncTable {
    pub entity: &'static str,
    pub table: &'static str,
    pub pk: &'static str,
    pub columns: &'static [&'static str],
}

/// The key row as a sync entity. Part of the frozen vault format: the
/// columns are those 4.0.7 publishes and reads, plus `recovery_version`
/// (platform-spec 8.1, 9.5: a column with a default may be added).
pub const SYNC_TABLE: SyncTable = SyncTable {
    entity: "password_vault",
    table: "password_vault",
    pk: "id",
    columns: &[
        "vault_id",
        "crypto_version",
        "kdf_algorithm",
        "kdf_salt",
        "kdf_memory",
        "kdf_iterations",
        "kdf_parallelism",
        "wrapped_key",
        "recovery_salt",
        "recovery_wrapped_key",
        "recovery_version",
        "lock_hash",
        "lock_kind",
        "lock_hint",
        "created_at",
        "updated_at",
    ],
};

const KIND_PIN: &str = "pin";
const KIND_PASSWORD: &str = "password";
const MIN_SECRET_LEN: usize = 4;
/// Minutes of inactivity before the lock closes, until the shell says otherwise.
pub const DEFAULT_TIMEOUT_MIN: u32 = 5;

/// The secret given is not the lock's (`unlock`, `verify`): the text of
/// `Error::Other` the commands had before, kept for the screens. Callers
/// ask [`Error::is_wrong_secret`] rather than compare the text.
pub const ERR_WRONG: &str = "Wrong password";
/// `current` of [`Lock::set`] is not the secret in force; see [`ERR_WRONG`].
pub const ERR_CURRENT: &str = "Current password is incorrect";
const ERR_NOT_TAKEN_OVER: &str =
    "The lock of an earlier version could not be read; restart the app";
/// `recover` on a row whose second wrap is the Recovery Key: that key is
/// kept, so the path is [`Lock::recover_keep`] (platform-spec 8.1).
pub const ERR_RECOVERY_V2: &str = "recovery key v2: use recover_keep";
/// `recovery_regenerate` on a row of version 2: a new Recovery Key goes
/// through the reissue flow of the module `cloud`, or the escrow and the
/// shares would silently die (cloud-recovery.md §3).
pub const ERR_REGENERATE_V2: &str = "recovery key v2: reissue it with recovery_prepare_v2 and recovery_store_v2";

/// The second wrap is made from a 24-char code.
pub const RECOVERY_VERSION_CODE: u8 = recovery::VERSION_CODE;
/// The second wrap is made from the Recovery Key of 12 words.
pub const RECOVERY_VERSION_WORDS: u8 = recovery::VERSION_WORDS;
/// The entropy of a Recovery Key.
pub const RK_LEN: usize = recovery::RK_LEN;

/// What the caller of [`Lock::reset`] removes in its transaction: its own
/// rows under the old key. They go together with the key or not at all.
pub type ResetFuture<'c> = KeyFuture<'c, ()>;

/// The work a module does on the lock's behalf over a connection the lock
/// lends it for `'c`.
pub type KeyFuture<'c, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'c>>;

/// What a module keeps encrypted under the vault key in tables of its own,
/// beside the secret boxes the lock keeps itself. The lock asks it before a
/// key is given up and when what an older key encrypted moves to the open
/// one; it never reads those tables.
#[derive(Clone, Copy)]
pub struct KeyUser {
    /// Whether anything is kept under the key with this vault id.
    pub holds: for<'c> fn(&'c mut SqliteConnection, &'c str) -> KeyFuture<'c, bool>,
    /// Encrypt with `to` what is kept under `from`, in the lock's
    /// transaction. False when something could not be read and stays under
    /// the old key: the lock keeps that key for a later attempt.
    pub rekey: for<'c> fn(&'c mut SqliteConnection, Rekey<'c>) -> KeyFuture<'c, bool>,
}

/// An older key and the key in use, each with its vault id.
#[derive(Clone, Copy)]
pub struct Rekey<'a> {
    pub from: &'a SecretKey,
    pub from_id: &'a str,
    pub to: &'a SecretKey,
    pub to_id: &'a str,
}

/// What happened to the lock. The shell turns these into events of the UI;
/// modules subscribe to them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// The session closed: by the user, by the timeout, or because a synced
    /// row changed the lock.
    Locked,
    /// The session opened, or the lock was set, changed or taken off.
    Unlocked,
    /// The key row or what is under the key changed: a synced row arrived,
    /// or replaced keys opened.
    Changed,
    /// The key was replaced by a new one: what the old one encrypted, the
    /// secret boxes among it, is gone.
    Reset,
}

/// The lock as the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LockStatus {
    pub enabled: bool,
    pub locked: bool,
    pub timeout_min: u32,
    /// `none` | `ok` | `mismatch`
    pub vault: String,
    /// `pin` | `password`
    pub kind: String,
    pub hint: Option<String>,
    pub has_recovery: bool,
    /// What the second wrap is made from: `1` a code of 24 chars, `2` the
    /// Recovery Key of 12 words; `None` without a second wrap. The screen of
    /// the lock labels its field by it (platform-spec 8.1).
    pub recovery_version: Option<u8>,
}

/// The wrap of version 2 of the vault key under a Recovery Key the module
/// `cloud` drew, prepared in memory by [`Lock::recovery_prepare_v2`] and
/// written by [`Lock::recovery_store_v2`] (platform-spec 8.1).
pub struct RecoveryPreparedV2 {
    /// The key row the wrap belongs to.
    vault_id: String,
    prepared: vault::RecoveryPrepared,
}

impl RecoveryPreparedV2 {
    /// The 12 words of the Recovery Key the wrap was made under.
    pub fn words(&self) -> &str {
        &self.prepared.code
    }

    /// The key row the wrap is for.
    pub fn vault_id(&self) -> &str {
        &self.vault_id
    }
}

impl std::fmt::Debug for RecoveryPreparedV2 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecoveryPreparedV2")
            .field("vault_id", &self.vault_id)
            .finish_non_exhaustive()
    }
}

/// The lock of the app: the key row in the data file, the key in memory,
/// the open session.
pub struct Lock {
    db: Pool<Sqlite>,
    vault: VaultState,
    session: Session,
    timeout_min: AtomicU32,
    /// The lock an earlier build kept in its settings, until the key row
    /// names a lock or none.
    legacy: Mutex<Option<LockMeta>>,
    /// [`Lock::upgrade`] failed: the lock stays closed for this run.
    not_taken_over: AtomicBool,
    events: broadcast::Sender<Event>,
    /// What the modules keep under the key.
    key_users: Vec<KeyUser>,
}

pub(crate) fn hash_password(password: &str) -> Result<String> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(Error::other)
}

pub(crate) fn verify_hash(password: &str, phc: &str) -> bool {
    PasswordHash::new(phc)
        .map(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}

fn parse_kind(kind: Option<String>) -> Result<String> {
    match kind.as_deref() {
        None | Some(KIND_PASSWORD) => Ok(KIND_PASSWORD.into()),
        Some(KIND_PIN) => Ok(KIND_PIN.into()),
        Some(_) => Err(Error::other("Unknown lock kind")),
    }
}

fn clean_hint(hint: Option<String>) -> Option<String> {
    hint.map(|h| h.trim().to_string()).filter(|h| !h.is_empty())
}

/// Trimmed secret that satisfies the rules for its kind.
fn validate_secret(kind: &str, secret: &str) -> Result<String> {
    let secret = secret.trim().to_string();
    if secret.chars().count() < MIN_SECRET_LEN {
        return Err(Error::other("Must be at least 4 characters"));
    }
    if kind == KIND_PIN && !secret.chars().all(|c| c.is_ascii_digit()) {
        return Err(Error::other("PIN must contain only digits"));
    }
    Ok(secret)
}

impl Lock {
    /// The lock of the data file behind `db`. Nothing is read yet: a start
    /// calls [`Lock::upgrade`], then [`Lock::open_default`].
    pub fn new(db: Pool<Sqlite>) -> Self {
        Self::with_key_users(db, Vec::new())
    }

    /// The lock of a product whose modules keep `key_users` under the key.
    /// They are known before anything opens: a key the lock gives up or
    /// moves away from must not hold what a module it does not know kept.
    pub fn with_key_users(db: Pool<Sqlite>, key_users: Vec<KeyUser>) -> Self {
        Self {
            db,
            vault: VaultState::default(),
            session: Session::default(),
            timeout_min: AtomicU32::new(DEFAULT_TIMEOUT_MIN),
            legacy: Mutex::new(None),
            not_taken_over: AtomicBool::new(false),
            events: broadcast::channel(16).0,
            key_users,
        }
    }

    /// The events from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    /// What the key row says about the lock; without a row, the lock an
    /// earlier build left in its settings. An error while what that build
    /// left is not taken over: nothing may open before it is.
    async fn form(&self) -> Result<SyncedLock> {
        if self.not_taken_over.load(Ordering::Relaxed) {
            return Err(Error::other(ERR_NOT_TAKEN_OVER));
        }
        match synced_lock(&self.db).await? {
            SyncedLock::NoRow => Ok(self.legacy().map_or(SyncedLock::NoRow, SyncedLock::Meta)),
            form => Ok(form),
        }
    }

    /// Whether a PIN or password guards the app. What is kept behind it
    /// stays out of reach of anything that runs without the user: the push
    /// handler gets no keys while this is true, nor when it cannot be read.
    pub async fn enabled(&self) -> bool {
        !matches!(
            self.form().await,
            Ok(SyncedLock::NoRow | SyncedLock::Default)
        )
    }

    /// A lock that cannot be read shows as closed.
    pub async fn status(&self) -> LockStatus {
        let form = self.form().await.unwrap_or(SyncedLock::Unstated);
        let enabled = matches!(form, SyncedLock::Meta(_) | SyncedLock::Unstated);
        let timeout_min = self.timeout();
        let locked = enabled && !self.session.is_unlocked(timeout_min);
        // A row that says nothing shows the lock this device had, if any.
        let meta = match form {
            SyncedLock::Meta(meta) => Some(meta),
            SyncedLock::Unstated => self.legacy(),
            _ => None,
        };
        let (kind, hint) = match meta {
            Some(meta) => (meta.kind.unwrap_or_else(|| KIND_PASSWORD.into()), meta.hint),
            None => (KIND_PASSWORD.into(), None),
        };
        let recovery_version = vault::recovery_version(&self.db).await.unwrap_or(None);
        LockStatus {
            enabled,
            locked,
            timeout_min,
            vault: self.vault_label().await,
            kind,
            hint,
            has_recovery: recovery_version.is_some(),
            recovery_version,
        }
    }

    /// True while what the lock guards must not be shown.
    pub async fn is_locked(&self) -> bool {
        self.status().await.locked
    }

    /// Minutes of inactivity before the session closes; 0 never.
    pub fn timeout(&self) -> u32 {
        self.timeout_min.load(Ordering::Relaxed)
    }

    pub fn set_timeout(&self, minutes: u32) {
        self.timeout_min.store(minutes, Ordering::Relaxed);
    }

    /// The user is active: the inactivity timer of an open session restarts.
    pub fn touch(&self) {
        if self.session.is_open() {
            self.session.touch();
        }
    }

    /// The auto-lock: closes a session idle for longer than the timeout.
    /// Returns whether it did.
    pub fn tick(&self) -> bool {
        if !self.session.is_open() {
            return false;
        }
        let timeout = self.timeout();
        if timeout == 0 || self.session.is_unlocked(timeout) {
            return false;
        }
        self.session.clear();
        self.vault.lock();
        self.emit(Event::Locked);
        true
    }

    /// Without a lock the vault opens by itself with the built-in secret. A
    /// row that says nothing about the lock is tried with it: the key opens —
    /// there is no lock, and the row says so from now on; it does not — the
    /// row is behind a lock, and the vault stays closed for its secret.
    pub async fn open_default(&self) -> Result<()> {
        match self.form().await? {
            SyncedLock::Meta(_) => Ok(()),
            SyncedLock::NoRow | SyncedLock::Default => {
                self.open_with_password(DEFAULT_SECRET).await
            }
            SyncedLock::Unstated => {
                // A session the lock of an earlier build opened keeps its mismatch.
                let mismatch = self.vault.is_mismatch();
                self.open_with_password(DEFAULT_SECRET).await?;
                if self.vault.open_key().is_some() {
                    store_lock_meta(&self.db, None).await?;
                    self.drop_legacy().await;
                } else if !mismatch {
                    self.vault.relock();
                }
                Ok(())
            }
        }
    }

    /// Enable, change or (with `password = None`) remove the lock.
    /// `current` is required whenever a secret is already set. The first
    /// enable also creates the recovery key and returns it; it is shown once.
    pub async fn set(
        &self,
        password: Option<String>,
        current: Option<String>,
        kind: Option<String>,
        hint: Option<String>,
    ) -> Result<Option<String>> {
        let form = self.form().await?;
        // A wrong `current` counts as a wrong secret of `unlock` and waits
        // as it does: changing the lock is no way around the pause.
        if matches!(form, SyncedLock::Meta(_) | SyncedLock::Unstated) {
            self.session.throttle().await;
        }
        // The secret the key is wrapped with now; without a lock the built-in one.
        let current_secret = match &form {
            SyncedLock::Meta(meta) => match current.as_deref() {
                Some(c) if verify_hash(c, &meta.hash) => c.to_string(),
                _ => {
                    self.session.record_failure();
                    return Err(Error::other(ERR_CURRENT));
                }
            },
            // Only the wrap can tell whether `current` is the secret.
            SyncedLock::Unstated => current.clone().ok_or_else(|| Error::other(ERR_CURRENT))?,
            SyncedLock::NoRow | SyncedLock::Default => DEFAULT_SECRET.to_string(),
        };
        let unstated = |e: Error| match (&form, e) {
            (SyncedLock::Unstated, Error::VaultMismatch) => {
                self.session.record_failure();
                Error::other(ERR_CURRENT)
            }
            (_, e) => e,
        };
        let password = password
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty());
        let mut recovery_key = None;
        match password {
            Some(p) => {
                let kind = parse_kind(kind)?;
                let p = validate_secret(&kind, &p)?;
                let meta = LockMeta {
                    hash: hash_password(&p)?,
                    kind: Some(kind),
                    hint: clean_hint(hint),
                };
                let prepared = vault::prepare_rewrap(&self.db, &current_secret, &p)
                    .await
                    .map_err(unstated)?;
                if prepared.has_row() {
                    let mut tx = self.db.begin().await.map_err(Error::db)?;
                    vault::apply_rewrap(&mut tx, &prepared).await?;
                    store_lock_meta(&mut *tx, Some(&meta)).await?;
                    tx.commit().await.map_err(Error::db)?;
                    self.vault.apply(prepared.memory);
                    self.session.open(meta);
                } else {
                    // The first key row of the device, made at once with its lock.
                    self.vault.apply(prepared.memory);
                    self.session.open(meta);
                    if let Err(e) = self.ensure_key().await {
                        self.session.clear();
                        return Err(e);
                    }
                }
                self.drop_legacy().await;
                self.reopen_replaced_keys(&current_secret).await;
                recovery_key = self.ensure_recovery().await?;
            }
            None => {
                // Turning the lock off keeps every password: the vault is rewrapped with the built-in secret.
                let prepared = vault::prepare_rewrap(&self.db, &current_secret, DEFAULT_SECRET)
                    .await
                    .map_err(unstated)?;
                let mut tx = self.db.begin().await.map_err(Error::db)?;
                vault::apply_rewrap(&mut tx, &prepared).await?;
                store_lock_meta(&mut *tx, None).await?;
                tx.commit().await.map_err(Error::db)?;
                self.vault.apply(prepared.memory);
                self.drop_legacy().await;
                self.reopen_replaced_keys(&current_secret).await;
                self.session.clear();
            }
        }
        self.emit(Event::Unlocked);
        Ok(recovery_key)
    }

    /// Create the Recovery Key (12 words) once the key row exists and has no
    /// second wrap yet. A mismatched vault leaves recovery to the device
    /// that owns it.
    async fn ensure_recovery(&self) -> Result<Option<String>> {
        let Ok((key, vault_id)) = self.ensure_key().await else {
            return Ok(None);
        };
        if vault::has_recovery(&self.db).await? {
            return Ok(None);
        }
        let words = vault::install_recovery(&self.db, &key, &vault_id).await?;
        Ok(Some(words.to_string()))
    }

    /// Open the session with `password`. False when there is no lock to open.
    pub async fn unlock(&self, password: &str) -> Result<bool> {
        self.session.throttle().await;
        match self.form().await? {
            SyncedLock::NoRow | SyncedLock::Default => return Ok(false),
            SyncedLock::Unstated => {
                // The row does not say what wraps the key: the secret that
                // opens it names the lock from now on, with the kind and hint
                // of this device's lock when it is that one. The lock of an
                // earlier build's settings that does not open the key opens
                // the session alone, as that build did, and is not written.
                let own = self.legacy_for(password);
                self.open_with_password(password).await?;
                if self.vault.open_key().is_some() {
                    let meta = match own {
                        Some(meta) => meta,
                        None => LockMeta {
                            hash: hash_password(password)?,
                            kind: None,
                            hint: None,
                        },
                    };
                    store_lock_meta(&self.db, Some(&meta)).await?;
                    self.drop_legacy().await;
                    self.session.open(meta);
                } else if let Some(meta) = own {
                    self.session.open(meta);
                } else {
                    self.vault.relock();
                    self.session.record_failure();
                    return Err(Error::other(ERR_WRONG));
                }
            }
            SyncedLock::Meta(meta) => {
                if !verify_hash(password, &meta.hash) {
                    self.session.record_failure();
                    return Err(Error::other(ERR_WRONG));
                }
                self.open_with_password(password).await?;
                self.session.open(meta);
                // The lock of an earlier build's settings makes its key row now.
                if self.legacy().is_some() {
                    self.ensure_key().await?;
                }
            }
        }
        self.emit(Event::Unlocked);
        Ok(true)
    }

    /// Close the session and drop the key from memory.
    pub fn lock(&self) {
        self.session.clear();
        self.vault.lock();
        self.emit(Event::Locked);
    }

    /// Check the secret of the lock without changing anything: the
    /// confirmation of a dangerous action. Fails when there is no lock.
    /// A wrong secret counts as a wrong one of `unlock` and waits as it does.
    pub async fn verify(&self, secret: &str) -> Result<()> {
        let form = self.form().await?;
        if matches!(form, SyncedLock::NoRow | SyncedLock::Default) {
            return Err(Error::VaultLocked);
        }
        self.session.throttle().await;
        let right = match form {
            SyncedLock::Meta(meta) => verify_hash(secret, &meta.hash),
            _ => self.legacy_for(secret).is_some() || vault::opens_row(&self.db, secret).await?,
        };
        if right {
            self.session.forgive();
            Ok(())
        } else {
            self.session.record_failure();
            Err(Error::other(ERR_WRONG))
        }
    }

    /// A row of version 1 moves to the Recovery Key: the 12 words are
    /// returned, the old code stops working. A row of version 2 is refused
    /// with [`ERR_REGENERATE_V2`]: its key is reissued only by the flow of
    /// the module `cloud` ([`Lock::recovery_prepare_v2`] and
    /// [`Lock::recovery_store_v2`]), or the escrow and the shares would
    /// silently die.
    pub async fn recovery_regenerate(&self, current: &str) -> Result<String> {
        self.session.throttle().await;
        if let Err(e) = self.verify(current).await {
            self.session.record_failure();
            return Err(e);
        }
        if vault::recovery_version(&self.db).await? == Some(RECOVERY_VERSION_WORDS) {
            return Err(Error::other(ERR_REGENERATE_V2));
        }
        let (key, vault_id) = self.ensure_key().await?;
        let words = vault::install_recovery(&self.db, &key, &vault_id).await?;
        Ok(words.to_string())
    }

    /// Step 1 of recovery: confirm the code or the words open the vault,
    /// nothing changes. The version is the row's (platform-spec 8.1).
    pub async fn recovery_check(&self, code: &str) -> Result<()> {
        self.session.throttle().await;
        match vault::open_with_recovery(&self.db, code).await {
            Ok(_) => Ok(()),
            Err(e) => {
                self.session.record_failure();
                Err(e)
            }
        }
    }

    /// Step 2 of recovery for a row of version 1: set a new secret with the
    /// code, unlock, and hand out the Recovery Key (12 words) that replaces
    /// the used code; the row is of version 2 from here. A row of version 2
    /// is refused with [`ERR_RECOVERY_V2`]: its key is kept by
    /// [`Lock::recover_keep`].
    pub async fn recover(
        &self,
        code: &str,
        password: String,
        kind: Option<String>,
        hint: Option<String>,
    ) -> Result<String> {
        self.session.throttle().await;
        if vault::recovery_version(&self.db).await? == Some(RECOVERY_VERSION_WORDS) {
            return Err(Error::other(ERR_RECOVERY_V2));
        }
        let (key, vault_id) = match vault::open_with_recovery(&self.db, code).await {
            Ok(opened) => opened,
            Err(e) => {
                self.session.record_failure();
                return Err(e);
            }
        };
        let (meta, prepared) = self.prepare_new_secret(&key, &vault_id, password, kind, hint).await?;
        let recovery = vault::prepare_recovery(&key, &vault_id).await?;

        let mut tx = self.db.begin().await.map_err(Error::db)?;
        vault::apply_rewrap(&mut tx, &prepared).await?;
        vault::store_recovery(&mut *tx, &recovery).await?;
        store_lock_meta(&mut *tx, Some(&meta)).await?;
        tx.commit().await.map_err(Error::db)?;

        self.finish_recovery(prepared, meta).await;
        Ok(recovery.code.to_string())
    }

    /// Step 2 of recovery for a row of version 2 (cloud-recovery.md §5, §7):
    /// open the key with the 12 words, rewrap it under the new secret, open
    /// the session. The second wrap and its version stay as they are: the
    /// same Recovery Key, the same generation, the escrow and the shares
    /// remain in force. A row of version 1 fails like a wrong key.
    pub async fn recover_keep(
        &self,
        words: &str,
        password: String,
        kind: Option<String>,
        hint: Option<String>,
    ) -> Result<()> {
        self.session.throttle().await;
        if vault::recovery_version(&self.db).await? != Some(RECOVERY_VERSION_WORDS) {
            self.session.record_failure();
            return Err(Error::RecoveryInvalid);
        }
        let (key, vault_id) = match vault::open_with_recovery(&self.db, words).await {
            Ok(opened) => opened,
            Err(e) => {
                self.session.record_failure();
                return Err(e);
            }
        };
        let (meta, prepared) = self.prepare_new_secret(&key, &vault_id, password, kind, hint).await?;

        let mut tx = self.db.begin().await.map_err(Error::db)?;
        vault::apply_rewrap(&mut tx, &prepared).await?;
        store_lock_meta(&mut *tx, Some(&meta)).await?;
        tx.commit().await.map_err(Error::db)?;

        self.finish_recovery(prepared, meta).await;
        Ok(())
    }

    /// The new lock of a recovery: its parameters and the wrap of the key
    /// under the new secret, in memory.
    async fn prepare_new_secret(
        &self,
        key: &SecretKey,
        vault_id: &str,
        password: String,
        kind: Option<String>,
        hint: Option<String>,
    ) -> Result<(LockMeta, vault::RewrapPrepared)> {
        let kind = parse_kind(kind)?;
        let password = validate_secret(&kind, &password)?;
        let meta = LockMeta {
            hash: hash_password(&password)?,
            kind: Some(kind),
            hint: clean_hint(hint),
        };
        let prepared =
            vault::prepare_rewrap_key(key.clone_key(), vault_id.to_string(), password).await?;
        Ok((meta, prepared))
    }

    /// After the transaction of a recovery: the key in memory, the session open.
    async fn finish_recovery(&self, prepared: vault::RewrapPrepared, meta: LockMeta) {
        self.vault.apply(prepared.memory);
        self.drop_legacy().await;
        self.reopen_replaced_keys(DEFAULT_SECRET).await;
        self.session.open(meta);
        self.emit(Event::Unlocked);
    }

    /// Step 3 of the reissue of the Recovery Key (platform-spec 8.1): the
    /// wrap of version 2 of the key under the 16 bytes `rk` the module
    /// `cloud` drew, in memory only; `current` is the secret of the lock,
    /// the confirmation of the owner. Nothing is written until
    /// [`Lock::recovery_store_v2`]. Also the way a row of version 1 moves to
    /// version 2 when the cloud is switched on.
    pub async fn recovery_prepare_v2(
        &self,
        current: &str,
        rk: &[u8; RK_LEN],
    ) -> Result<RecoveryPreparedV2> {
        self.session.throttle().await;
        if let Err(e) = self.verify(current).await {
            self.session.record_failure();
            return Err(e);
        }
        let (key, vault_id) = self.ensure_key().await?;
        let prepared = vault::prepare_recovery_with(&key, &vault_id, rk).await?;
        Ok(RecoveryPreparedV2 {
            vault_id,
            prepared,
        })
    }

    /// Step 6 of the reissue: one transaction puts the prepared wrap in
    /// place of the second wrap and sets `recovery_version = 2`; the first
    /// wrap and the lock fields stay. Refused when the key row changed
    /// since the wrap was prepared. The row goes to sync as usual.
    pub async fn recovery_store_v2(&self, prepared: &RecoveryPreparedV2) -> Result<()> {
        let current = key_id(&self.db).await?;
        if current.as_deref() != Some(prepared.vault_id.as_str()) {
            return Err(Error::other("the key row changed since the wrap was prepared"));
        }
        vault::store_recovery(&self.db, &prepared.prepared).await?;
        self.emit(Event::Changed);
        Ok(())
    }

    /// The lock of a restore whose key row did not open with the words
    /// (cloud-recovery.md §7, «строка не совпала»): the key the escrow
    /// carries, `vault_key` under its `vault_id`, takes the place of the key
    /// row — wrapped by `password`, with its second wrap under the Recovery
    /// Key `rk` — in one transaction, and the secret boxes of this device
    /// are emptied as at a reset. What the vault holds encrypted under that
    /// key (`decrypt_field`) still reads; the row goes to sync as usual.
    pub async fn install_key(
        &self,
        vault_key: SecretKey,
        vault_id: &str,
        rk: &[u8; RK_LEN],
        password: String,
        kind: Option<String>,
        hint: Option<String>,
    ) -> Result<()> {
        let kind = parse_kind(kind)?;
        let password = validate_secret(&kind, &password)?;
        let meta = LockMeta {
            hash: hash_password(&password)?,
            kind: Some(kind),
            hint: clean_hint(hint),
        };
        let fresh = vault::fresh_key_of(&password, vault_key, vault_id.to_string()).await?;
        let recovery = vault::prepare_recovery_with(&fresh.key, vault_id, rk).await?;
        let mut tx = self.db.begin().await.map_err(Error::db)?;
        vault::replace_row(&mut tx, &fresh, Some(&meta)).await?;
        vault::store_recovery(&mut *tx, &recovery).await?;
        sqlx::query("DELETE FROM lock_secrets")
            .execute(&mut *tx)
            .await
            .map_err(Error::db)?;
        vault::delete_setting(&mut *tx, vault::REPLACED_KEYS).await?;
        tx.commit().await.map_err(Error::db)?;
        self.vault.lock();
        self.vault.set_open(fresh.kek, fresh.key, fresh.vault_id);
        self.drop_legacy().await;
        self.session.open(meta);
        self.emit(Event::Reset);
        Ok(())
    }

    /// Check `secret`, then put a new key in place of the old one: the key
    /// row is made anew, wrapped by the same secret under the same lock, and
    /// every secret box is emptied — what the old key encrypted cannot be
    /// read any more. `clear` removes the caller's own rows under the old key
    /// in the same transaction; nothing is removed before the new key is ready.
    pub async fn reset<F>(&self, secret: &str, clear: F) -> Result<()>
    where
        F: for<'c> FnOnce(&'c mut SqliteConnection) -> ResetFuture<'c> + Send,
    {
        self.verify(secret).await?;
        let meta = match self.form().await? {
            SyncedLock::Meta(meta) => meta,
            _ => match self.legacy_for(secret) {
                Some(meta) => meta,
                None => LockMeta {
                    hash: hash_password(secret)?,
                    kind: None,
                    hint: None,
                },
            },
        };
        let fresh = vault::fresh_key(secret).await?;
        self.replace_key(Some(&fresh), Some(&meta), clear).await?;
        self.vault.lock();
        self.vault.set_open(fresh.kek, fresh.key, fresh.vault_id);
        self.drop_legacy().await;
        self.session.open(meta);
        self.emit(Event::Reset);
        Ok(())
    }

    /// Remove the key row, the secret boxes and the kept wraps of replaced
    /// keys without a secret: the demo data that replaces everything starts
    /// from here. The lock is gone with the row.
    pub async fn wipe(&self) -> Result<()> {
        self.replace_key(None, None, |_| Box::pin(async { Ok(()) }))
            .await?;
        self.session.clear();
        self.vault.lock();
        self.drop_legacy().await;
        self.open_default().await?;
        self.emit(Event::Reset);
        Ok(())
    }

    /// One transaction: the key row gives way to `fresh` (or goes), and
    /// nothing encrypted with any earlier key is left.
    async fn replace_key<F>(
        &self,
        fresh: Option<&vault::FreshKey>,
        meta: Option<&LockMeta>,
        clear: F,
    ) -> Result<()>
    where
        F: for<'c> FnOnce(&'c mut SqliteConnection) -> ResetFuture<'c> + Send,
    {
        let mut tx = self.db.begin().await.map_err(Error::db)?;
        clear(&mut tx).await?;
        match fresh {
            Some(fresh) => vault::replace_row(&mut tx, fresh, meta).await?,
            None => {
                sqlx::query("DELETE FROM password_vault")
                    .execute(&mut *tx)
                    .await
                    .map_err(Error::db)?;
            }
        }
        sqlx::query("DELETE FROM lock_secrets")
            .execute(&mut *tx)
            .await
            .map_err(Error::db)?;
        vault::delete_setting(&mut *tx, vault::REPLACED_KEYS).await?;
        tx.commit().await.map_err(Error::db)
    }

    /// The box `name`: values kept under the vault key on this device.
    pub fn secret_box<'a>(&'a self, name: &'a str) -> SecretBox<'a> {
        SecretBox { lock: self, name }
    }

    /// A synced key row arrived: reopen or close the key in memory and take
    /// over the lock the row carries. A row that says nothing about the lock
    /// and opens with the key of the session gets the session's lock. A
    /// session opened with another hash closes.
    pub async fn refresh_after_sync(&self) {
        if self.not_taken_over.load(Ordering::Relaxed) {
            return;
        }
        self.refresh_key().await;
        let Ok(mut form) = synced_lock(&self.db).await else {
            return;
        };
        if form == SyncedLock::Unstated && self.vault.open_key().is_some() {
            let meta = self.session.meta();
            if store_lock_meta(&self.db, meta.as_ref()).await.is_ok() {
                form = meta.map_or(SyncedLock::Default, SyncedLock::Meta);
            }
        }
        // The row names its lock, or none: the lock of the settings gave way.
        if matches!(form, SyncedLock::Default | SyncedLock::Meta(_)) {
            self.drop_legacy().await;
        }
        let current = match &form {
            SyncedLock::Meta(meta) => Some(meta.hash.clone()),
            SyncedLock::Unstated => self.legacy().map(|meta| meta.hash),
            _ => None,
        };
        if current != self.session.hash() {
            self.session.clear();
            self.vault.relock();
            self.emit(Event::Locked);
        }
        if let Err(e) = self.open_default().await {
            eprintln!("lock: {e}");
        }
        self.emit(Event::Changed);
    }
}

/// A data file with the lock's tables and what the lock reads of others.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use crate::crypto::{derive_kek, random_salt, wrap_key, KdfParams};
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};

    pub(crate) async fn pool() -> (Pool<Sqlite>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite:{}?mode=rwc", dir.path().join("app.db").display());
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .connect(&url)
            .await
            .unwrap();
        let others = [
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL)",
            "CREATE TABLE passwords (
                id TEXT PRIMARY KEY NOT NULL,
                password_enc TEXT NOT NULL,
                note_enc TEXT,
                vault_id TEXT NOT NULL
            )",
        ];
        for step in SCHEMA_STEPS.iter().chain(others.iter()) {
            sqlx::query(*step).execute(&db).await.unwrap();
        }
        (db, dir)
    }

    /// A module's table of entries under the key, as a module tells the lock of it.
    pub(crate) const PASSWORDS: KeyUser = KeyUser {
        holds: |conn, vault_id| {
            Box::pin(async move {
                let held: Option<i64> =
                    sqlx::query_scalar("SELECT 1 FROM passwords WHERE vault_id = ? LIMIT 1")
                        .bind(vault_id)
                        .fetch_optional(&mut *conn)
                        .await
                        .map_err(Error::db)?;
                Ok(held.is_some())
            })
        },
        rekey: |conn, rekey| {
            Box::pin(async move {
                let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
                    "SELECT id, password_enc, note_enc FROM passwords WHERE vault_id = ?",
                )
                .bind(rekey.from_id)
                .fetch_all(&mut *conn)
                .await
                .map_err(Error::db)?;
                let mut complete = true;
                for (id, password_enc, note_enc) in rows {
                    let recrypt = |field: &str, stored: &str| -> Result<String> {
                        let plain = decrypt_field(rekey.from, &id, field, stored)?;
                        encrypt_field(rekey.to, &id, field, &plain)
                    };
                    let moved = recrypt("password", &password_enc).and_then(|password| {
                        let note = note_enc
                            .as_deref()
                            .map(|n| recrypt("note", n))
                            .transpose()?;
                        Ok((password, note))
                    });
                    let Ok((password, note)) = moved else {
                        complete = false;
                        continue;
                    };
                    sqlx::query(
                        "UPDATE passwords SET password_enc = ?, note_enc = ?, vault_id = ? WHERE id = ?",
                    )
                    .bind(password)
                    .bind(note)
                    .bind(rekey.to_id)
                    .bind(&id)
                    .execute(&mut *conn)
                    .await
                    .map_err(Error::db)?;
                }
                Ok(complete)
            })
        },
    };

    /// The second wrap of version 1 as 4.0.7 and 5.x before C3 wrote it: a
    /// code of 24 chars, the production KDF, no version column written (the
    /// default of the column is 1). Returns the code in display form.
    pub(crate) async fn install_recovery_v1(
        db: &Pool<Sqlite>,
        key: &SecretKey,
        vault_id: &str,
    ) -> String {
        let code = crate::recovery::generate();
        let canonical = crate::recovery::normalize_code(&code).unwrap();
        let salt = random_salt();
        let kek = derive_kek(&canonical, &KdfParams::production(salt)).unwrap();
        let wrapped = wrap_key(key, &kek, vault_id).unwrap();
        sqlx::query(
            "UPDATE password_vault SET recovery_salt = ?, recovery_wrapped_key = ? WHERE id = ?",
        )
        .bind(B64.encode(salt))
        .bind(wrapped)
        .bind(ROW_ID)
        .execute(db)
        .await
        .unwrap();
        code
    }

    /// A key row wrapped by `secret` that says nothing about the lock, with a
    /// KDF cheap enough for a test.
    pub(crate) async fn key_row(db: &Pool<Sqlite>, vault_id: &str, secret: &str) -> SecretKey {
        let salt = random_salt();
        let params = KdfParams {
            memory_kib: 8,
            iterations: 1,
            parallelism: 1,
            salt,
        };
        let key = SecretKey::random();
        let wrapped = wrap_key(&key, &derive_kek(secret, &params).unwrap(), vault_id).unwrap();
        sqlx::query(
            "INSERT INTO password_vault (
                id, vault_id, kdf_salt, kdf_memory, kdf_iterations, kdf_parallelism,
                wrapped_key, created_at, updated_at
             ) VALUES (?, ?, ?, 8, 1, 1, ?, 't', 't')",
        )
        .bind(ROW_ID)
        .bind(vault_id)
        .bind(B64.encode(salt))
        .bind(wrapped)
        .execute(db)
        .await
        .unwrap();
        key
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{key_row, pool};
    use super::*;
    use std::time::Duration;

    const PIN: &str = "4821";

    #[test]
    fn hash_roundtrip() {
        let phc = hash_password("secret").unwrap();
        assert!(verify_hash("secret", &phc));
        assert!(!verify_hash("wrong", &phc));
    }

    fn drain(events: &mut broadcast::Receiver<Event>) -> Vec<Event> {
        std::iter::from_fn(|| events.try_recv().ok()).collect()
    }

    /// The row with lock fields: what the row names is the lock.
    #[tokio::test]
    async fn a_row_with_a_lock_is_that_lock() {
        let (db, _dir) = pool().await;
        let key = key_row(&db, "theirs", PIN).await;
        let meta = LockMeta {
            hash: hash_password(PIN).unwrap(),
            kind: Some("pin".into()),
            hint: Some("the usual".into()),
        };
        store_lock_meta(&db, Some(&meta)).await.unwrap();
        let lock = Lock::new(db.clone());
        lock.open_default().await.unwrap();

        assert!(lock.enabled().await);
        let status = lock.status().await;
        assert!(status.enabled && status.locked);
        assert_eq!(status.kind, "pin");
        assert_eq!(status.hint.as_deref(), Some("the usual"));
        assert_eq!(lock.require_open().unwrap_err(), Error::VaultLocked);
        assert_eq!(
            lock.verify("0000").await.unwrap_err().to_string(),
            ERR_WRONG
        );
        lock.verify(PIN).await.unwrap();

        assert!(lock.unlock("0000").await.is_err());
        assert!(lock.unlock(PIN).await.unwrap());
        assert!(!lock.status().await.locked);
        let (open, vault_id) = lock.require_open().unwrap();
        assert_eq!(vault_id, "theirs");
        let probe = encrypt_field(&key, "p", "password", "x").unwrap();
        assert_eq!(decrypt_field(&open, "p", "password", &probe).unwrap(), "x");
        db.close().await;
    }

    /// The row marked `none`: no lock, the key opens with the built-in secret.
    #[tokio::test]
    async fn a_row_without_a_lock_opens_by_itself() {
        let (db, _dir) = pool().await;
        key_row(&db, "theirs", DEFAULT_SECRET).await;
        store_lock_meta(&db, None).await.unwrap();
        let lock = Lock::new(db.clone());
        lock.open_default().await.unwrap();

        assert!(!lock.enabled().await);
        let status = lock.status().await;
        assert!(!status.enabled && !status.locked);
        assert_eq!(status.vault, "ok");
        assert!(!lock.unlock(PIN).await.unwrap());
        assert_eq!(lock.verify(PIN).await.unwrap_err(), Error::VaultLocked);
        assert_eq!(lock.ensure_key().await.unwrap().1, "theirs");
        db.close().await;
    }

    /// The row that says nothing and opens with the built-in secret: no lock,
    /// and the row says so from now on.
    #[tokio::test]
    async fn a_row_that_says_nothing_and_opens_by_itself_names_no_lock() {
        let (db, _dir) = pool().await;
        key_row(&db, "theirs", DEFAULT_SECRET).await;
        let lock = Lock::new(db.clone());
        assert_eq!(synced_lock(&db).await.unwrap(), SyncedLock::Unstated);
        assert!(lock.enabled().await);

        lock.open_default().await.unwrap();

        assert_eq!(synced_lock(&db).await.unwrap(), SyncedLock::Default);
        assert!(!lock.enabled().await);
        assert_eq!(lock.require_open().unwrap().1, "theirs");
        db.close().await;
    }

    /// The row that says nothing and is behind a PIN: locked, the kind is
    /// `password`, no hint; the secret that opens the key names the lock.
    #[tokio::test]
    async fn a_row_that_says_nothing_and_stays_closed_is_behind_a_lock() {
        let (db, _dir) = pool().await;
        key_row(&db, "theirs", PIN).await;
        let lock = Lock::new(db.clone());
        lock.open_default().await.unwrap();

        assert_eq!(synced_lock(&db).await.unwrap(), SyncedLock::Unstated);
        let status = lock.status().await;
        assert!(status.enabled && status.locked);
        assert_eq!((status.kind.as_str(), status.hint), ("password", None));
        assert_eq!(status.vault, "ok");
        assert_eq!(lock.require_open().unwrap_err(), Error::VaultLocked);
        assert!(lock.verify("0000").await.is_err());
        lock.verify(PIN).await.unwrap();

        assert_eq!(
            lock.unlock("0000").await.unwrap_err().to_string(),
            ERR_WRONG
        );
        assert_eq!(lock.require_open().unwrap_err(), Error::VaultLocked);
        assert!(lock.unlock(PIN).await.unwrap());

        let SyncedLock::Meta(meta) = synced_lock(&db).await.unwrap() else {
            panic!("the unlock did not name the lock");
        };
        assert!(verify_hash(PIN, &meta.hash));
        assert_eq!((meta.kind, meta.hint), (None, None));
        assert_eq!(lock.require_open().unwrap().1, "theirs");
        db.close().await;
    }

    /// 5.0 makes a key row that names its lock, or names none.
    #[tokio::test]
    async fn a_row_made_here_always_says_what_wraps_it() {
        let (db, _dir) = pool().await;
        let lock = Lock::new(db.clone());
        lock.open_default().await.unwrap();
        assert_eq!(synced_lock(&db).await.unwrap(), SyncedLock::NoRow);
        lock.ensure_key().await.unwrap();
        assert_eq!(synced_lock(&db).await.unwrap(), SyncedLock::Default);

        let (other, _other_dir) = pool().await;
        let fresh = Lock::new(other.clone());
        fresh.open_default().await.unwrap();
        let recovery = fresh
            .set(
                Some(PIN.into()),
                None,
                Some("pin".into()),
                Some(" hint ".into()),
            )
            .await
            .unwrap();
        assert!(recovery.is_some());
        let SyncedLock::Meta(meta) = synced_lock(&other).await.unwrap() else {
            panic!("the first set made no row with its lock");
        };
        assert_eq!(
            (meta.kind.as_deref(), meta.hint.as_deref()),
            (Some("pin"), Some("hint"))
        );
        db.close().await;
        other.close().await;
    }

    /// A wrong `current` of `set` and a wrong secret of `verify` count as
    /// wrong secrets of `unlock`: after the third, whichever asked, each
    /// attempt waits. Activity forgives none of them, the right secret all.
    #[tokio::test]
    async fn wrong_secrets_of_set_and_verify_are_counted_and_slowed() {
        let (db, _dir) = pool().await;
        let lock = Lock::new(db);
        lock.open_default().await.unwrap();
        lock.set(Some(PIN.into()), None, Some("pin".into()), None)
            .await
            .unwrap();
        for _ in 0..2 {
            assert!(lock
                .set(Some("5555".into()), Some("0000".into()), None, None)
                .await
                .is_err());
        }
        assert!(lock.verify("0000").await.is_err());
        lock.touch();
        assert_eq!(lock.session.failures(), 3);

        let started = std::time::Instant::now();
        assert!(lock
            .set(Some("5555".into()), Some("1111".into()), None, None)
            .await
            .is_err());
        assert!(started.elapsed() >= Duration::from_millis(900));
        assert_eq!(lock.session.failures(), 4);

        lock.verify(PIN).await.unwrap();
        assert_eq!(lock.session.failures(), 0);
    }

    #[tokio::test]
    async fn the_lock_is_set_changed_and_taken_off_with_the_key_kept() {
        let (db, _dir) = pool().await;
        let lock = Lock::new(db.clone());
        lock.open_default().await.unwrap();
        let (key, vault_id) = lock.ensure_key().await.unwrap();
        let probe = encrypt_field(&key, "p", "password", "x").unwrap();

        assert!(lock
            .set(Some("12".into()), None, Some("pin".into()), None)
            .await
            .is_err());
        assert!(lock
            .set(Some("12ab".into()), None, Some("pin".into()), None)
            .await
            .is_err());
        lock.set(Some(PIN.into()), None, Some("pin".into()), None)
            .await
            .unwrap();
        assert_eq!(
            lock.set(Some("5555".into()), Some("0000".into()), None, None)
                .await
                .unwrap_err()
                .to_string(),
            ERR_CURRENT
        );
        lock.set(
            Some("secret words".into()),
            Some(PIN.into()),
            None,
            Some("song".into()),
        )
        .await
        .unwrap();
        let status = lock.status().await;
        assert_eq!(
            (status.kind.as_str(), status.hint.as_deref()),
            ("password", Some("song"))
        );
        lock.lock();
        assert!(lock.unlock(PIN).await.is_err());
        assert!(lock.unlock("secret words").await.unwrap());
        assert_eq!(lock.require_open().unwrap().1, vault_id);

        lock.set(None, Some("secret words".into()), None, None)
            .await
            .unwrap();
        assert_eq!(synced_lock(&db).await.unwrap(), SyncedLock::Default);
        assert!(!lock.enabled().await);
        let (open, still) = lock.require_open().unwrap();
        assert_eq!(still, vault_id);
        assert_eq!(decrypt_field(&open, "p", "password", &probe).unwrap(), "x");
        db.close().await;
    }

    /// The first lock issues the Recovery Key of 12 words (version 2); a
    /// forgotten secret is replaced with the words, and the words stay: the
    /// same wrap, the same version, nothing a code could do.
    #[tokio::test]
    async fn the_recovery_key_of_12_words_sets_a_new_secret_and_stays() {
        let (db, _dir) = pool().await;
        let lock = Lock::new(db.clone());
        lock.open_default().await.unwrap();
        let words = lock
            .set(Some(PIN.into()), None, Some("pin".into()), None)
            .await
            .unwrap()
            .expect("recovery key");
        assert_eq!(words.split(' ').count(), 12);
        let status = lock.status().await;
        assert!(status.has_recovery);
        assert_eq!(status.recovery_version, Some(RECOVERY_VERSION_WORDS));
        let wrap_before: (String, String, i64) = sqlx::query_as(
            "SELECT recovery_salt, recovery_wrapped_key, recovery_version FROM password_vault",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(wrap_before.2, 2);
        lock.lock();

        // A code, wrong words, right words.
        assert_eq!(
            lock.recovery_check("0000-0000").await.unwrap_err(),
            Error::RecoveryInvalid
        );
        assert_eq!(
            lock.recovery_check(&recovery::generate()).await.unwrap_err(),
            Error::RecoveryInvalid
        );
        lock.recovery_check(&words.to_uppercase()).await.unwrap();

        // The old path is closed for a row of version 2, before any KDF.
        assert_eq!(
            lock.recover(&words, "7777".into(), Some("pin".into()), None)
                .await
                .unwrap_err(),
            Error::other(ERR_RECOVERY_V2)
        );
        assert!(lock.status().await.locked);

        lock.recover_keep(&words, "7777".into(), Some("pin".into()), None)
            .await
            .unwrap();
        assert!(!lock.status().await.locked);
        let wrap_after: (String, String, i64) = sqlx::query_as(
            "SELECT recovery_salt, recovery_wrapped_key, recovery_version FROM password_vault",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(wrap_after, wrap_before, "recover_keep changed the second wrap");
        lock.recovery_check(&words).await.unwrap();
        lock.lock();
        assert!(lock.unlock(PIN).await.is_err());
        assert!(lock.unlock("7777").await.unwrap());

        // No regeneration of a version 2 key outside the reissue flow.
        assert_eq!(
            lock.recovery_regenerate("7777").await.unwrap_err(),
            Error::other(ERR_REGENERATE_V2)
        );
        assert!(lock.recovery_regenerate("0000").await.is_err());
        lock.recovery_check(&words).await.unwrap();
        db.close().await;
    }

    /// A row a client wrote before C3 (or 4.0.7): the second wrap is a code
    /// of 24 chars and the column holds its default. The code still opens
    /// it; `recover` replaces it with the words, and the row is of version 2
    /// from then on; words against a row of version 1 fail like a wrong code.
    #[tokio::test]
    async fn a_row_of_version_1_recovers_with_its_code_and_moves_to_words() {
        let (db, _dir) = pool().await;
        let lock = Lock::new(db.clone());
        lock.open_default().await.unwrap();
        lock.set(Some(PIN.into()), None, Some("pin".into()), None)
            .await
            .unwrap();
        let (key, vault_id) = lock.require_open().unwrap();
        // The wrap 4.0.7 would have written, under its own code.
        sqlx::query("UPDATE password_vault SET recovery_version = 1")
            .execute(&db)
            .await
            .unwrap();
        let code = testing::install_recovery_v1(&db, &key, &vault_id).await;
        assert_eq!(lock.status().await.recovery_version, Some(RECOVERY_VERSION_CODE));
        lock.lock();

        let (_, words_elsewhere) = recovery::generate_words();
        assert_eq!(
            lock.recovery_check(&words_elsewhere).await.unwrap_err(),
            Error::RecoveryInvalid
        );
        assert_eq!(
            lock.recover_keep(&code, "7777".into(), Some("pin".into()), None)
                .await
                .unwrap_err(),
            Error::RecoveryInvalid
        );
        lock.recovery_check(&code).await.unwrap();
        let words = lock
            .recover(&code, "7777".into(), Some("pin".into()), None)
            .await
            .unwrap();
        assert!(!lock.status().await.locked);
        assert_eq!(words.split(' ').count(), 12);
        assert_eq!(lock.status().await.recovery_version, Some(RECOVERY_VERSION_WORDS));
        assert_eq!(
            lock.recovery_check(&code).await.unwrap_err(),
            Error::RecoveryInvalid
        );
        lock.recovery_check(&words).await.unwrap();

        // `recovery_regenerate` of a row of version 1 hands out the words as well.
        testing::install_recovery_v1(&db, &key, &vault_id).await;
        sqlx::query("UPDATE password_vault SET recovery_version = 1")
            .execute(&db)
            .await
            .unwrap();
        let regenerated = lock.recovery_regenerate("7777").await.unwrap();
        assert_eq!(regenerated.split(' ').count(), 12);
        assert_eq!(lock.status().await.recovery_version, Some(RECOVERY_VERSION_WORDS));
        lock.recovery_check(&regenerated).await.unwrap();
        db.close().await;
    }

    /// The reissue flow of the module `cloud`: the wrap under a key it drew
    /// is prepared in memory and written in one step; between the two the
    /// old words still open the row, after it the new ones do.
    #[tokio::test]
    async fn a_wrap_under_a_given_key_is_prepared_in_memory_and_stored_at_once() {
        let (db, _dir) = pool().await;
        let lock = Lock::new(db.clone());
        lock.open_default().await.unwrap();
        let old = lock
            .set(Some(PIN.into()), None, Some("pin".into()), None)
            .await
            .unwrap()
            .expect("recovery key");
        let (entropy, words) = recovery::generate_words();

        assert!(lock.recovery_prepare_v2("0000", &entropy).await.is_err());
        let prepared = lock.recovery_prepare_v2(PIN, &entropy).await.unwrap();
        assert_eq!(prepared.words(), words.as_str());
        assert_eq!(prepared.vault_id(), lock.require_open().unwrap().1);
        assert!(!format!("{prepared:?}").contains(words.split(' ').next().unwrap()));
        lock.recovery_check(&old).await.unwrap();
        assert_eq!(
            lock.recovery_check(&words).await.unwrap_err(),
            Error::RecoveryInvalid
        );

        let mut events = lock.subscribe();
        lock.recovery_store_v2(&prepared).await.unwrap();
        assert_eq!(drain(&mut events), [Event::Changed]);
        lock.recovery_check(&words).await.unwrap();
        assert_eq!(
            lock.recovery_check(&old).await.unwrap_err(),
            Error::RecoveryInvalid
        );
        assert_eq!(lock.status().await.recovery_version, Some(RECOVERY_VERSION_WORDS));

        // A wrap prepared for a row that is gone is not written.
        let (other_entropy, _) = recovery::generate_words();
        let stale = lock.recovery_prepare_v2(PIN, &other_entropy).await.unwrap();
        lock.reset(PIN, |_| Box::pin(async { Ok(()) })).await.unwrap();
        assert!(lock.recovery_store_v2(&stale).await.is_err());
        db.close().await;
    }

    /// The column comes by the schema step with its default: a data file of
    /// 5.x before C3 opens, and its row is of version 1.
    #[tokio::test]
    async fn the_version_column_is_added_by_the_schema_step_with_its_default() {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite:{}?mode=rwc", dir.path().join("app.db").display());
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .connect(&url)
            .await
            .unwrap();
        for step in &SCHEMA_STEPS[..2] {
            sqlx::query(*step).execute(&db).await.unwrap();
        }
        sqlx::query(
            "INSERT INTO password_vault (id, vault_id, kdf_salt, kdf_memory, kdf_iterations,
             kdf_parallelism, wrapped_key, created_at, updated_at, recovery_salt, recovery_wrapped_key)
             VALUES ('default', 'v', 's', 8, 1, 1, 'w', 't', 't', 'rs', 'rw')",
        )
        .execute(&db)
        .await
        .unwrap();
        sqlx::query(SCHEMA_STEPS[2]).execute(&db).await.unwrap();
        let version: i64 = sqlx::query_scalar("SELECT recovery_version FROM password_vault")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(version, 1);
        assert_eq!(
            vault::recovery_version(&db).await.unwrap(),
            Some(RECOVERY_VERSION_CODE)
        );
        assert!(SYNC_TABLE.columns.contains(&"recovery_version"));
        db.close().await;
    }

    #[tokio::test]
    async fn the_lock_tells_what_happens_to_it() {
        let (db, _dir) = pool().await;
        let lock = Lock::new(db.clone());
        lock.open_default().await.unwrap();
        let mut events = lock.subscribe();

        lock.set(Some(PIN.into()), None, Some("pin".into()), None)
            .await
            .unwrap();
        assert_eq!(drain(&mut events), [Event::Unlocked]);
        lock.lock();
        assert_eq!(drain(&mut events), [Event::Locked]);
        assert!(lock.unlock("0000").await.is_err());
        assert_eq!(drain(&mut events), []);
        assert!(lock.unlock(PIN).await.unwrap());
        assert_eq!(drain(&mut events), [Event::Unlocked]);

        // The timeout closes an idle session; without a timeout nothing does.
        assert!(!lock.tick());
        lock.session.idle_for(Duration::from_secs(6 * 60));
        lock.set_timeout(0);
        assert!(!lock.tick());
        lock.set_timeout(5);
        assert!(lock.tick());
        assert_eq!(drain(&mut events), [Event::Locked]);
        assert!(lock.status().await.locked);
        assert_eq!(lock.require_open().unwrap_err(), Error::VaultLocked);
        assert!(!lock.tick());

        // A touch keeps an open session open, and opens none.
        lock.touch();
        assert!(lock.status().await.locked);
        assert!(lock.unlock(PIN).await.unwrap());
        lock.session.idle_for(Duration::from_secs(6 * 60));
        lock.touch();
        assert!(!lock.tick());

        // A synced row with another lock closes the session.
        let other = LockMeta {
            hash: hash_password("7777").unwrap(),
            kind: Some("pin".into()),
            hint: None,
        };
        drain(&mut events);
        store_lock_meta(&db, Some(&other)).await.unwrap();
        lock.refresh_after_sync().await;
        assert_eq!(drain(&mut events), [Event::Locked, Event::Changed]);
        assert!(lock.status().await.locked);
        db.close().await;
    }

    #[tokio::test]
    async fn a_synced_row_that_says_nothing_takes_the_lock_of_the_open_session() {
        let (db, _dir) = pool().await;
        let lock = Lock::new(db.clone());
        lock.open_default().await.unwrap();
        lock.set(
            Some(PIN.into()),
            None,
            Some("pin".into()),
            Some("hint".into()),
        )
        .await
        .unwrap();
        let before = synced_lock(&db).await.unwrap();
        // A peer of 4.0.7 rewrote the row with the same wrap and no lock fields.
        sqlx::query(
            "UPDATE password_vault SET lock_hash = NULL, lock_kind = NULL, lock_hint = NULL",
        )
        .execute(&db)
        .await
        .unwrap();
        let mut events = lock.subscribe();

        lock.refresh_after_sync().await;

        assert_eq!(synced_lock(&db).await.unwrap(), before);
        assert_eq!(drain(&mut events), [Event::Changed]);
        assert!(!lock.status().await.locked);
        assert!(lock.require_open().is_ok());
        db.close().await;
    }

    /// A restore whose row did not open installs the key of the escrow:
    /// the row is made anew around that key under the new PIN, its second
    /// wrap under the words of the restore, and what another device
    /// encrypted under the key still reads; the boxes are emptied.
    #[tokio::test]
    async fn the_key_of_the_escrow_is_installed_as_the_lock_of_a_restore() {
        let (db_a, _dir_a) = pool().await;
        let a = Lock::new(db_a.clone());
        a.open_default().await.unwrap();
        a.set(Some(PIN.into()), None, Some("pin".into()), None)
            .await
            .unwrap();
        let (key, vault_id) = a.require_open().unwrap();
        let field = encrypt_field(&key, "row-1", "password", "hunter2").unwrap();
        db_a.close().await;

        let (db, _dir) = pool().await;
        let b = Lock::new(db.clone());
        b.open_default().await.unwrap();
        b.secret_box("messenger").put("x", b"old").await.unwrap();
        let mut events = b.subscribe();
        let (entropy, words) = recovery::generate_words();
        assert!(b
            .install_key(key.clone_key(), &vault_id, &entropy, "12".into(), Some("pin".into()), None)
            .await
            .unwrap_err()
            .to_string()
            .contains("at least 4"));
        b.install_key(key.clone_key(), &vault_id, &entropy, "7777".into(), Some("pin".into()), None)
            .await
            .unwrap();

        assert_eq!(drain(&mut events), [Event::Reset]);
        let (open, id) = b.require_open().unwrap();
        assert_eq!(id, vault_id);
        assert_eq!(decrypt_field(&open, "row-1", "password", &field).unwrap(), "hunter2");
        assert!(b.secret_box("messenger").get("x").await.unwrap().is_none());
        assert_eq!(b.status().await.recovery_version, Some(RECOVERY_VERSION_WORDS));
        b.recovery_check(&words).await.unwrap();
        b.lock();
        assert!(b.unlock(PIN).await.unwrap_err().is_wrong_secret());
        assert!(b.unlock("7777").await.unwrap());
        assert!(Error::other(ERR_CURRENT).is_wrong_secret());
        assert!(!Error::RecoveryInvalid.is_wrong_secret());
        db.close().await;
    }
}
