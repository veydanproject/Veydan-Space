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
    decrypt_field, encrypt_field, key_id, store_lock_meta, synced_lock, LockMeta, SyncedLock,
    DEFAULT_SECRET, ERR_JOIN_OWN_LOCK, ROW_ID,
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
/// columns are those 4.0.7 publishes and reads.
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

const ERR_WRONG: &str = "Wrong password";
const ERR_CURRENT: &str = "Current password is incorrect";
const ERR_NOT_TAKEN_OVER: &str =
    "The lock of an earlier version could not be read; restart the app";

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
        LockStatus {
            enabled,
            locked,
            timeout_min,
            vault: self.vault_label().await,
            kind,
            hint,
            has_recovery: vault::has_recovery(&self.db).await.unwrap_or(false),
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
        // The secret the key is wrapped with now; without a lock the built-in one.
        let current_secret = match &form {
            SyncedLock::Meta(meta) => match current.as_deref() {
                Some(c) if verify_hash(c, &meta.hash) => c.to_string(),
                _ => return Err(Error::other(ERR_CURRENT)),
            },
            // Only the wrap can tell whether `current` is the secret.
            SyncedLock::Unstated => current.clone().ok_or_else(|| Error::other(ERR_CURRENT))?,
            SyncedLock::NoRow | SyncedLock::Default => DEFAULT_SECRET.to_string(),
        };
        let unstated = |e: Error| match (&form, e) {
            (SyncedLock::Unstated, Error::VaultMismatch) => Error::other(ERR_CURRENT),
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

    /// Create the recovery key once the key row exists and has none yet.
    /// A mismatched vault leaves recovery to the device that owns it.
    async fn ensure_recovery(&self) -> Result<Option<String>> {
        let Ok((key, vault_id)) = self.ensure_key().await else {
            return Ok(None);
        };
        if vault::has_recovery(&self.db).await? {
            return Ok(None);
        }
        Ok(Some(
            vault::install_recovery(&self.db, &key, &vault_id).await?,
        ))
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
    pub async fn verify(&self, secret: &str) -> Result<()> {
        let right = match self.form().await? {
            SyncedLock::Meta(meta) => verify_hash(secret, &meta.hash),
            SyncedLock::Unstated => {
                self.legacy_for(secret).is_some() || vault::opens_row(&self.db, secret).await?
            }
            SyncedLock::NoRow | SyncedLock::Default => return Err(Error::VaultLocked),
        };
        if right {
            Ok(())
        } else {
            Err(Error::other(ERR_WRONG))
        }
    }

    /// Replace the recovery key. The old one stops working.
    pub async fn recovery_regenerate(&self, current: &str) -> Result<String> {
        self.session.throttle().await;
        if let Err(e) = self.verify(current).await {
            self.session.record_failure();
            return Err(e);
        }
        let (key, vault_id) = self.ensure_key().await?;
        vault::install_recovery(&self.db, &key, &vault_id).await
    }

    /// Step 1 of recovery: confirm the code opens the vault, nothing changes.
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

    /// Step 2 of recovery: set a new secret with the code, unlock, and hand
    /// out a fresh recovery key. The used code stops working.
    pub async fn recover(
        &self,
        code: &str,
        password: String,
        kind: Option<String>,
        hint: Option<String>,
    ) -> Result<String> {
        self.session.throttle().await;
        let (key, vault_id) = match vault::open_with_recovery(&self.db, code).await {
            Ok(opened) => opened,
            Err(e) => {
                self.session.record_failure();
                return Err(e);
            }
        };
        let kind = parse_kind(kind)?;
        let password = validate_secret(&kind, &password)?;
        let meta = LockMeta {
            hash: hash_password(&password)?,
            kind: Some(kind),
            hint: clean_hint(hint),
        };
        let prepared =
            vault::prepare_rewrap_key(key.clone_key(), vault_id.clone(), password).await?;
        let recovery = vault::prepare_recovery(&key, &vault_id).await?;

        let mut tx = self.db.begin().await.map_err(Error::db)?;
        vault::apply_rewrap(&mut tx, &prepared).await?;
        vault::store_recovery(&mut *tx, &recovery).await?;
        store_lock_meta(&mut *tx, Some(&meta)).await?;
        tx.commit().await.map_err(Error::db)?;

        self.vault.apply(prepared.memory);
        self.drop_legacy().await;
        self.reopen_replaced_keys(DEFAULT_SECRET).await;
        self.session.open(meta);
        self.emit(Event::Unlocked);
        Ok(recovery.code)
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

    #[tokio::test]
    async fn the_recovery_key_sets_a_new_secret() {
        let (db, _dir) = pool().await;
        let lock = Lock::new(db.clone());
        lock.open_default().await.unwrap();
        let code = lock
            .set(Some(PIN.into()), None, Some("pin".into()), None)
            .await
            .unwrap()
            .expect("recovery key");
        lock.lock();

        assert_eq!(
            lock.recovery_check("0000-0000").await.unwrap_err(),
            Error::RecoveryInvalid
        );
        lock.recovery_check(&code).await.unwrap();
        let next = lock
            .recover(&code, "7777".into(), Some("pin".into()), None)
            .await
            .unwrap();
        assert!(!lock.status().await.locked);
        assert_eq!(
            lock.recovery_check(&code).await.unwrap_err(),
            Error::RecoveryInvalid
        );
        lock.recovery_check(&next).await.unwrap();

        assert!(lock.recovery_regenerate("0000").await.is_err());
        let third = lock.recovery_regenerate("7777").await.unwrap();
        assert_eq!(
            lock.recovery_check(&next).await.unwrap_err(),
            Error::RecoveryInvalid
        );
        lock.lock();
        assert!(lock.unlock(PIN).await.is_err());
        assert!(lock.unlock("7777").await.unwrap());
        lock.recovery_check(&third).await.unwrap();
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
}
