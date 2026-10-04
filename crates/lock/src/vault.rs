// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The key row: one random key wrapped by the secret of the lock and,
//! optionally, by a recovery code, with the parameters of the lock next to
//! the wrap they match; and the wraps of keys that a synced row replaced.

use crate::crypto::{
    decrypt, derive_kek, encrypt, field_aad, random_salt, unwrap_key, wrap_key, CryptoError,
    KdfParams, SecretKey, KDF_ITERATIONS, KDF_MEMORY_KIB, KDF_PARALLELISM,
};
use crate::error::{Error, Result};
use crate::state::MemoryUpdate;
use crate::{recovery, secrets, Event, KeyUser, Lock, Rekey};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite, SqliteExecutor, Transaction};
use uuid::Uuid;
use zeroize::Zeroizing;

/// Id of the only `password_vault` row.
pub const ROW_ID: &str = "default";
/// Wraps the vault while no PIN or password is set. Known from the sources:
/// it only keeps the storage format uniform, it does not protect the data.
pub const DEFAULT_SECRET: &str = "veydan-no-lock";
/// `lock_kind` value marking a row wrapped with `DEFAULT_SECRET`.
pub(crate) const KIND_NONE: &str = "none";

/// `app_settings` key of the key wraps that a synced row took the place of. Not synced.
pub(crate) const REPLACED_KEYS: &str = "vault_replaced_keys";
/// Why a device is not let into a vault: see [`Lock::check_replaceable`].
pub const ERR_JOIN_OWN_LOCK: &str = "Passwords or messenger keys on this device are behind the lock it has now: one set here, or that of a vault it was connected to before. Turn the lock off on this device, then connect to the vault again.";

pub(crate) struct VaultRow {
    vault_id: String,
    crypto_version: i64,
    kdf_memory: i64,
    kdf_iterations: i64,
    kdf_parallelism: i64,
    kdf_salt: String,
    wrapped_key: String,
    recovery_salt: Option<String>,
    recovery_wrapped_key: Option<String>,
    lock_hash: Option<String>,
    lock_kind: Option<String>,
    lock_hint: Option<String>,
}

/// The lock as the key row carries it, so every device checks the same secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockMeta {
    /// PHC string of Argon2, as 4.0.7 writes and checks it.
    pub hash: String,
    /// `pin` | `password`; missing means `password`.
    pub kind: Option<String>,
    pub hint: Option<String>,
}

/// What the key row says about the lock: the three forms 4.0.7 writes, or no row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncedLock {
    /// No key row yet.
    NoRow,
    /// Row without lock fields: 4.0.7 creates the row so (`ensure_key`) and
    /// fills the fields later, and only while the vault is open. Says nothing
    /// about the lock: the default secret is tried, and a key it does not
    /// open is behind a lock.
    Unstated,
    /// Wrapped with `DEFAULT_SECRET`: no PIN on any device.
    Default,
    Meta(LockMeta),
}

fn decode_salt(b64: &str) -> std::result::Result<[u8; 16], CryptoError> {
    let raw = B64.decode(b64).map_err(|_| CryptoError::Kdf)?;
    raw.try_into().map_err(|_| CryptoError::Kdf)
}

fn kdf_params(
    memory: i64,
    iterations: i64,
    parallelism: i64,
    salt: &str,
) -> std::result::Result<KdfParams, CryptoError> {
    Ok(KdfParams {
        memory_kib: u32::try_from(memory).unwrap_or(u32::MAX),
        iterations: u32::try_from(iterations).unwrap_or(u32::MAX),
        parallelism: u32::try_from(parallelism).unwrap_or(u32::MAX),
        salt: decode_salt(salt)?,
    })
}

fn kdf_from_row(row: &VaultRow) -> std::result::Result<KdfParams, CryptoError> {
    kdf_params(
        row.kdf_memory,
        row.kdf_iterations,
        row.kdf_parallelism,
        &row.kdf_salt,
    )
}

type RawRow = (
    String,
    i64,
    i64,
    i64,
    i64,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

pub(crate) async fn load_row(executor: impl SqliteExecutor<'_>) -> Result<Option<VaultRow>> {
    let row = sqlx::query_as::<_, RawRow>(
        "SELECT vault_id, crypto_version, kdf_memory, kdf_iterations, kdf_parallelism, kdf_salt,
                wrapped_key, recovery_salt, recovery_wrapped_key, lock_hash, lock_kind, lock_hint
         FROM password_vault WHERE id = ?",
    )
    .bind(ROW_ID)
    .fetch_optional(executor)
    .await
    .map_err(Error::db)?;
    Ok(row.map(|r| VaultRow {
        vault_id: r.0,
        crypto_version: r.1,
        kdf_memory: r.2,
        kdf_iterations: r.3,
        kdf_parallelism: r.4,
        kdf_salt: r.5,
        wrapped_key: r.6,
        recovery_salt: r.7,
        recovery_wrapped_key: r.8,
        lock_hash: r.9,
        lock_kind: r.10,
        lock_hint: r.11,
    }))
}

fn form_of(row: Option<VaultRow>) -> SyncedLock {
    let Some(row) = row else {
        return SyncedLock::NoRow;
    };
    match (row.lock_hash, row.lock_kind) {
        (Some(hash), kind) => SyncedLock::Meta(LockMeta {
            hash,
            kind,
            hint: row.lock_hint,
        }),
        (None, Some(kind)) if kind == KIND_NONE => SyncedLock::Default,
        (None, _) => SyncedLock::Unstated,
    }
}

/// The form of the key row.
pub async fn synced_lock(executor: impl SqliteExecutor<'_>) -> Result<SyncedLock> {
    Ok(form_of(load_row(executor).await?))
}

/// Write the lock into the key row; `None` marks it as wrapped with `DEFAULT_SECRET`.
/// No-op without a row.
pub async fn store_lock_meta(
    executor: impl SqliteExecutor<'_>,
    meta: Option<&LockMeta>,
) -> Result<()> {
    let (hash, kind, hint) = match meta {
        Some(m) => (Some(m.hash.as_str()), m.kind.as_deref(), m.hint.as_deref()),
        None => (None, Some(KIND_NONE), None),
    };
    sqlx::query(
        "UPDATE password_vault
         SET lock_hash = ?, lock_kind = ?, lock_hint = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(hash)
    .bind(kind)
    .bind(hint)
    .bind(Utc::now().to_rfc3339())
    .bind(ROW_ID)
    .execute(executor)
    .await
    .map_err(Error::db)?;
    Ok(())
}

/// A new key row wrapped by `wrapped`, with the lock of `meta` or, without
/// one, marked as wrapped with `DEFAULT_SECRET`: 5.0 never writes a row that
/// says nothing about the lock.
async fn insert_row(
    executor: impl SqliteExecutor<'_>,
    vault_id: &str,
    salt: [u8; 16],
    wrapped: &str,
    meta: Option<&LockMeta>,
) -> std::result::Result<(), sqlx::Error> {
    let (hash, kind, hint) = match meta {
        Some(m) => (Some(m.hash.as_str()), m.kind.as_deref(), m.hint.as_deref()),
        None => (None, Some(KIND_NONE), None),
    };
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO password_vault (
            id, vault_id, crypto_version, kdf_algorithm, kdf_salt,
            kdf_memory, kdf_iterations, kdf_parallelism, wrapped_key, created_at, updated_at,
            lock_hash, lock_kind, lock_hint
         ) VALUES (?, ?, 1, 'argon2id', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(ROW_ID)
    .bind(vault_id)
    .bind(B64.encode(salt))
    .bind(i64::from(KDF_MEMORY_KIB))
    .bind(i64::from(KDF_ITERATIONS))
    .bind(i64::from(KDF_PARALLELISM))
    .bind(wrapped)
    .bind(&now)
    .bind(&now)
    .bind(hash)
    .bind(kind)
    .bind(hint)
    .execute(executor)
    .await?;
    Ok(())
}

pub(crate) async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> std::result::Result<T, CryptoError> + Send + 'static,
) -> std::result::Result<T, CryptoError> {
    match tokio::task::spawn_blocking(job).await {
        Ok(r) => r,
        Err(_) => Err(CryptoError::Kdf),
    }
}

/// Whether `secret` unwraps the key of the row. False without a row.
pub(crate) async fn opens_row(db: &Pool<Sqlite>, secret: &str) -> Result<bool> {
    let Some(row) = load_row(db).await? else {
        return Ok(false);
    };
    if row.crypto_version != 1 {
        return Ok(false);
    }
    let Ok(params) = kdf_from_row(&row) else {
        return Ok(false);
    };
    let secret = secret.to_string();
    let (wrapped, vault_id) = (row.wrapped_key, row.vault_id);
    let opened = blocking(move || {
        let kek = derive_kek(&secret, &params)?;
        unwrap_key(&wrapped, &kek, &vault_id)
    })
    .await;
    Ok(opened.is_ok())
}

/// The value of an `app_settings` key of the lock. The lock keeps one key
/// there, `vault_replaced_keys`, and reads those an older data file left.
pub(crate) async fn read_setting(
    executor: impl SqliteExecutor<'_>,
    key: &str,
) -> Result<Option<String>> {
    sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
        .bind(key)
        .fetch_optional(executor)
        .await
        .map_err(Error::db)
}

async fn write_setting(executor: impl SqliteExecutor<'_>, key: &str, value: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO app_settings (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(executor)
    .await
    .map_err(Error::db)?;
    Ok(())
}

pub(crate) async fn delete_setting(executor: impl SqliteExecutor<'_>, key: &str) -> Result<()> {
    sqlx::query("DELETE FROM app_settings WHERE key = ?")
        .bind(key)
        .execute(executor)
        .await
        .map_err(Error::db)?;
    Ok(())
}

/// The wrap of a vault key whose row a synced row replaced. Kept on this
/// device until what the key encrypted has moved to the key in use.
#[derive(Serialize, Deserialize)]
pub(crate) struct ReplacedKey {
    vault_id: String,
    kdf_salt: String,
    kdf_memory: i64,
    kdf_iterations: i64,
    kdf_parallelism: i64,
    wrapped_key: String,
    /// The recovery wrap goes along: nothing else holds it.
    #[serde(default)]
    recovery_salt: Option<String>,
    #[serde(default)]
    recovery_wrapped_key: Option<String>,
    /// The row named a lock: its wrap is not one by the built-in secret,
    /// which is then not tried.
    #[serde(default)]
    behind_lock: bool,
}

impl ReplacedKey {
    fn of(row: &VaultRow) -> Self {
        Self {
            vault_id: row.vault_id.clone(),
            kdf_salt: row.kdf_salt.clone(),
            kdf_memory: row.kdf_memory,
            kdf_iterations: row.kdf_iterations,
            kdf_parallelism: row.kdf_parallelism,
            wrapped_key: row.wrapped_key.clone(),
            recovery_salt: row.recovery_salt.clone(),
            recovery_wrapped_key: row.recovery_wrapped_key.clone(),
            behind_lock: row.lock_hash.is_some(),
        }
    }

    /// The key itself, if `secret` is what wrapped it.
    async fn open(&self, secret: &str) -> Option<SecretKey> {
        let params = kdf_params(
            self.kdf_memory,
            self.kdf_iterations,
            self.kdf_parallelism,
            &self.kdf_salt,
        )
        .ok()?;
        let secret = secret.to_string();
        let wrapped = self.wrapped_key.clone();
        let vault_id = self.vault_id.clone();
        blocking(move || {
            let kek = derive_kek(&secret, &params)?;
            unwrap_key(&wrapped, &kek, &vault_id)
        })
        .await
        .ok()
    }

    /// The key itself, if `code` is the recovery key of the replaced row.
    async fn open_with_recovery(&self, code: &str) -> Option<SecretKey> {
        let canonical = recovery::normalize(code)?;
        let salt_b64 = self.recovery_salt.clone()?;
        let wrapped = self.recovery_wrapped_key.clone()?;
        let vault_id = self.vault_id.clone();
        blocking(move || {
            let salt = decode_salt(&salt_b64)?;
            let kek = derive_kek(&canonical, &KdfParams::production(salt))?;
            unwrap_key(&wrapped, &kek, &vault_id)
        })
        .await
        .ok()
    }
}

async fn load_replaced(db: &Pool<Sqlite>) -> Result<Vec<ReplacedKey>> {
    match read_setting(db, REPLACED_KEYS).await? {
        Some(json) => serde_json::from_str(&json).map_err(Error::other),
        None => Ok(Vec::new()),
    }
}

async fn save_replaced(db: &Pool<Sqlite>, keys: &[ReplacedKey]) -> Result<()> {
    if keys.is_empty() {
        return delete_setting(db, REPLACED_KEYS).await;
    }
    let json = serde_json::to_string(keys).map_err(Error::other)?;
    write_setting(db, REPLACED_KEYS, &json).await
}

/// Whether this device holds anything encrypted with the key of `vault_id`:
/// something a module keeps under that key, or any value of a secret box
/// (those do not name their key).
async fn key_in_use(db: &Pool<Sqlite>, users: &[KeyUser], vault_id: &str) -> Result<bool> {
    let mut conn = db.acquire().await.map_err(Error::db)?;
    for user in users {
        if (user.holds)(&mut conn, vault_id).await? {
            return Ok(true);
        }
    }
    let used: Option<i64> = sqlx::query_scalar("SELECT 1 FROM lock_secrets LIMIT 1")
        .fetch_optional(&mut *conn)
        .await
        .map_err(Error::db)?;
    Ok(used.is_some())
}

/// Vault id of the local row, if there is one.
pub async fn key_id(db: &Pool<Sqlite>) -> Result<Option<String>> {
    Ok(load_row(db).await?.map(|row| row.vault_id))
}

/// What became of the replaced keys this device kept.
#[derive(Debug, Default, PartialEq, Eq)]
struct ReplacedKeys {
    /// Keys that opened; what they encrypted is under the key in use now.
    opened: usize,
    /// Keys nothing at hand opens. They stay kept.
    closed: usize,
}

/// Re-encrypt what the modules keep and the secret boxes of this device from
/// one vault key to another. False when something of a module could not be
/// read and stays under the old key.
async fn reencrypt(
    db: &Pool<Sqlite>,
    users: &[KeyUser],
    old_key: &SecretKey,
    old_id: &str,
    new_key: &SecretKey,
    new_id: &str,
) -> Result<bool> {
    let mut tx = db.begin().await.map_err(Error::db)?;
    let mut complete = true;

    let rekey = Rekey {
        from: old_key,
        from_id: old_id,
        to: new_key,
        to_id: new_id,
    };
    for user in users {
        if !(user.rekey)(&mut tx, rekey).await? {
            complete = false;
        }
    }

    let stored: Vec<(String, String, String)> =
        sqlx::query_as("SELECT box, key, value FROM lock_secrets")
            .fetch_all(&mut *tx)
            .await
            .map_err(Error::db)?;
    for (name, key, value) in stored {
        let aad = secrets::aad(&name, &key);
        // Not under the old key: another replaced key, or the open one already.
        let Ok(plain) = decrypt_field(old_key, &aad, secrets::FIELD, &value) else {
            continue;
        };
        let plain = Zeroizing::new(plain);
        let moved = encrypt_field(new_key, &aad, secrets::FIELD, &plain)?;
        sqlx::query("UPDATE lock_secrets SET value = ? WHERE box = ? AND key = ?")
            .bind(moved)
            .bind(&name)
            .bind(&key)
            .execute(&mut *tx)
            .await
            .map_err(Error::db)?;
    }

    tx.commit().await.map_err(Error::db)?;
    Ok(complete)
}

/// New vault wrap, computed without holding a database lock.
/// Argon2 takes seconds; a transaction open that long makes the next write fail with "database is locked".
pub(crate) struct RewrapPrepared {
    pub(crate) memory: MemoryUpdate,
    write: Option<RewrapWrite>,
}

struct RewrapWrite {
    salt_b64: String,
    wrapped: String,
}

/// Read the key row and derive the new wrap. No transaction. Without a row
/// the result is the pending key of `new_password`.
pub(crate) async fn prepare_rewrap(
    db: &Pool<Sqlite>,
    current: &str,
    new_password: &str,
) -> Result<RewrapPrepared> {
    let new_password = new_password.to_string();
    let Some(row) = load_row(db).await? else {
        let salt = random_salt();
        let params = KdfParams::production(salt);
        let kek = blocking(move || derive_kek(&new_password, &params))
            .await
            .map_err(|_| Error::VaultMismatch)?;
        return Ok(RewrapPrepared {
            memory: MemoryUpdate::Pending { kek, salt },
            write: None,
        });
    };
    if row.crypto_version != 1 {
        return Err(Error::VaultMismatch);
    }
    let current = current.to_string();
    let params = kdf_from_row(&row).map_err(|_| Error::VaultMismatch)?;
    let wrapped = row.wrapped_key.clone();
    let vault_id = row.vault_id.clone();
    let (key, vault_id) = blocking(move || {
        let kek = derive_kek(&current, &params)?;
        let key = unwrap_key(&wrapped, &kek, &vault_id)?;
        Ok((key, vault_id))
    })
    .await
    .map_err(|_| Error::VaultMismatch)?;
    prepare_rewrap_key(key, vault_id, new_password).await
}

/// Wrap an already open vault key. No database access.
pub(crate) async fn prepare_rewrap_key(
    key: SecretKey,
    vault_id: String,
    new_password: String,
) -> Result<RewrapPrepared> {
    let salt = random_salt();
    let new_params = KdfParams::production(salt);
    let key_for_wrap = key.clone_key();
    let vault_for_wrap = vault_id.clone();
    let (kek, wrapped) = blocking(move || {
        let kek = derive_kek(&new_password, &new_params)?;
        let wrapped = wrap_key(&key_for_wrap, &kek, &vault_for_wrap)?;
        Ok((kek, wrapped))
    })
    .await
    .map_err(|_| Error::VaultMismatch)?;
    Ok(RewrapPrepared {
        memory: MemoryUpdate::Open { kek, key, vault_id },
        write: Some(RewrapWrite {
            salt_b64: B64.encode(salt),
            wrapped,
        }),
    })
}

impl RewrapPrepared {
    /// Whether there is a row to write the wrap into.
    pub(crate) fn has_row(&self) -> bool {
        self.write.is_some()
    }
}

/// Store a prepared wrap. The statement is the whole critical section.
pub(crate) async fn apply_rewrap(
    tx: &mut Transaction<'_, Sqlite>,
    prepared: &RewrapPrepared,
) -> Result<()> {
    let Some(write) = &prepared.write else {
        return Ok(());
    };
    sqlx::query(
        "UPDATE password_vault
         SET kdf_salt = ?, kdf_memory = ?, kdf_iterations = ?, kdf_parallelism = ?,
             wrapped_key = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(&write.salt_b64)
    .bind(i64::from(KDF_MEMORY_KIB))
    .bind(i64::from(KDF_ITERATIONS))
    .bind(i64::from(KDF_PARALLELISM))
    .bind(&write.wrapped)
    .bind(Utc::now().to_rfc3339())
    .bind(ROW_ID)
    .execute(&mut **tx)
    .await
    .map_err(Error::db)?;
    Ok(())
}

/// Recovery wrap ready to store. The code is shown once and never written.
pub(crate) struct RecoveryPrepared {
    pub(crate) code: String,
    salt_b64: String,
    wrapped: String,
}

/// Derive a recovery wrap. No database access.
pub(crate) async fn prepare_recovery(key: &SecretKey, vault_id: &str) -> Result<RecoveryPrepared> {
    let key = key.clone_key();
    let vault_id = vault_id.to_string();
    let (code, salt, wrapped) = blocking(move || {
        let code = recovery::generate();
        let canonical = recovery::normalize(&code).ok_or(CryptoError::Kdf)?;
        let salt = random_salt();
        let kek = derive_kek(&canonical, &KdfParams::production(salt))?;
        let wrapped = wrap_key(&key, &kek, &vault_id)?;
        Ok((code, salt, wrapped))
    })
    .await
    .map_err(|_| Error::DecryptFailed)?;
    Ok(RecoveryPrepared {
        code,
        salt_b64: B64.encode(salt),
        wrapped,
    })
}

pub(crate) async fn store_recovery(
    executor: impl SqliteExecutor<'_>,
    prepared: &RecoveryPrepared,
) -> Result<()> {
    sqlx::query(
        "UPDATE password_vault
         SET recovery_salt = ?, recovery_wrapped_key = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(&prepared.salt_b64)
    .bind(&prepared.wrapped)
    .bind(Utc::now().to_rfc3339())
    .bind(ROW_ID)
    .execute(executor)
    .await
    .map_err(Error::db)?;
    Ok(())
}

/// Wrap `key` under a fresh recovery code and store it as the second wrap.
/// Returns the code in display form; it is never stored.
pub(crate) async fn install_recovery(
    executor: impl SqliteExecutor<'_>,
    key: &SecretKey,
    vault_id: &str,
) -> Result<String> {
    let prepared = prepare_recovery(key, vault_id).await?;
    store_recovery(executor, &prepared).await?;
    Ok(prepared.code)
}

pub(crate) async fn has_recovery(db: &Pool<Sqlite>) -> Result<bool> {
    Ok(load_row(db)
        .await?
        .is_some_and(|row| row.recovery_wrapped_key.is_some()))
}

/// Unwrap the vault key with a recovery code. Wrong or malformed codes fail alike.
pub(crate) async fn open_with_recovery(
    db: &Pool<Sqlite>,
    code: &str,
) -> Result<(SecretKey, String)> {
    let row = load_row(db).await?.ok_or(Error::RecoveryInvalid)?;
    let (Some(salt_b64), Some(wrapped)) = (row.recovery_salt, row.recovery_wrapped_key) else {
        return Err(Error::not_found("recovery key"));
    };
    let canonical = recovery::normalize(code).ok_or(Error::RecoveryInvalid)?;
    let vault_id = row.vault_id;
    let id_job = vault_id.clone();
    let key = blocking(move || {
        let salt = decode_salt(&salt_b64)?;
        let kek = derive_kek(&canonical, &KdfParams::production(salt))?;
        unwrap_key(&wrapped, &kek, &id_job)
    })
    .await
    .map_err(|_| Error::RecoveryInvalid)?;
    Ok((key, vault_id))
}

/// A new key, wrapped by `secret`: what a reset puts in place of the row.
pub(crate) struct FreshKey {
    pub(crate) kek: SecretKey,
    pub(crate) salt: [u8; 16],
    pub(crate) key: SecretKey,
    pub(crate) vault_id: String,
    pub(crate) wrapped: String,
}

pub(crate) async fn fresh_key(secret: &str) -> Result<FreshKey> {
    let secret = secret.to_string();
    blocking(move || {
        let salt = random_salt();
        let kek = derive_kek(&secret, &KdfParams::production(salt))?;
        let key = SecretKey::random();
        let vault_id = Uuid::new_v4().to_string();
        let wrapped = wrap_key(&key, &kek, &vault_id)?;
        Ok(FreshKey {
            kek,
            salt,
            key,
            vault_id,
            wrapped,
        })
    })
    .await
    .map_err(|_| Error::DecryptFailed)
}

/// Replace the key row by one of a fresh key, in the caller's transaction.
pub(crate) async fn replace_row(
    tx: &mut Transaction<'_, Sqlite>,
    fresh: &FreshKey,
    meta: Option<&LockMeta>,
) -> Result<()> {
    sqlx::query("DELETE FROM password_vault")
        .execute(&mut **tx)
        .await
        .map_err(Error::db)?;
    insert_row(&mut **tx, &fresh.vault_id, fresh.salt, &fresh.wrapped, meta)
        .await
        .map_err(Error::db)
}

pub fn encrypt_field(key: &SecretKey, id: &str, field: &str, plaintext: &str) -> Result<String> {
    encrypt(key, &field_aad(id, field), plaintext.as_bytes()).map_err(|_| Error::DecryptFailed)
}

pub fn decrypt_field(key: &SecretKey, id: &str, field: &str, stored: &str) -> Result<String> {
    let bytes = decrypt(key, &field_aad(id, field), stored).map_err(|_| Error::DecryptFailed)?;
    String::from_utf8(bytes).map_err(|_| Error::DecryptFailed)
}

/// The key in memory, as the operations of the lock open, close and move it.
impl Lock {
    /// Before a synced row replaces the local one (`incoming` is its vault id) or
    /// removes it (`None`): keep the wrap of a key this device still has data
    /// under. Without it that data could never be read again.
    pub async fn keep_replaced_key(&self, incoming: Option<&str>) -> Result<()> {
        let db = &self.db;
        let Some(row) = load_row(db).await? else {
            return Ok(());
        };
        if incoming == Some(row.vault_id.as_str())
            || !key_in_use(db, &self.key_users, &row.vault_id).await?
        {
            return Ok(());
        }
        let mut kept = load_replaced(db).await?;
        kept.retain(|k| k.vault_id != row.vault_id);
        kept.push(ReplacedKey::of(&row));
        save_replaced(db, &kept).await
    }

    /// Whether a joining device may give its key row up for the vault's row with
    /// the key `incoming`. It may not while its own key holds data and is wrapped
    /// by a lock: the vault's lock takes over, and nothing could open that key later.
    pub async fn check_replaceable(&self, incoming: &str) -> Result<()> {
        let db = &self.db;
        let Some(row) = load_row(db).await? else {
            return Ok(());
        };
        if row.vault_id == incoming || !key_in_use(db, &self.key_users, &row.vault_id).await? {
            return Ok(());
        }
        match ReplacedKey::of(&row).open(DEFAULT_SECRET).await {
            Some(_) => Ok(()),
            None => Err(Error::other(ERR_JOIN_OWN_LOCK)),
        }
    }

    /// Open the vault with `password` once the lock accepted it, or stage it
    /// when there is no row yet. A wrong wrap is no error: the phase becomes mismatch.
    pub(crate) async fn open_with_password(&self, password: &str) -> Result<()> {
        let row = load_row(&self.db).await?;
        let password = password.to_string();
        match row {
            None => {
                let salt = random_salt();
                let params = KdfParams::production(salt);
                let kek = blocking(move || derive_kek(&password, &params))
                    .await
                    .map_err(|_| Error::VaultMismatch)?;
                self.vault.set_pending(kek, salt);
                Ok(())
            }
            Some(row) => {
                if row.crypto_version != 1 {
                    self.vault.set_mismatch();
                    return Ok(());
                }
                let params = match kdf_from_row(&row) {
                    Ok(p) => p,
                    Err(_) => {
                        self.vault.set_mismatch();
                        return Ok(());
                    }
                };
                let wrapped = row.wrapped_key.clone();
                let vault_id = row.vault_id.clone();
                let secret = password.clone();
                let opened = blocking(move || {
                    let kek = derive_kek(&password, &params)?;
                    let key = unwrap_key(&wrapped, &kek, &vault_id)?;
                    Ok((kek, key, vault_id))
                })
                .await;
                match opened {
                    Ok((kek, key, vault_id)) => {
                        self.vault.set_open(kek, key, vault_id);
                        // The unlock stands even when a replaced key stays closed.
                        self.reopen_replaced_keys(&secret).await;
                    }
                    Err(_) => self.vault.set_mismatch(),
                }
                Ok(())
            }
        }
    }

    /// After the vault opened: what a replaced key encrypted moves to the open
    /// key. A replaced key opens when this session held it, with the built-in
    /// secret, or with `secret`: the one that opened the vault, or, with
    /// `previous`, one the user names for the replaced keys, which may also be a
    /// recovery key. A key that stays closed is kept for a later attempt.
    async fn adopt_replaced_keys(&self, secret: &str, previous: bool) -> Result<ReplacedKeys> {
        let mut done = ReplacedKeys::default();
        let kept = load_replaced(&self.db).await?;
        if kept.is_empty() {
            self.vault.retain_replaced(&[]);
            return Ok(done);
        }
        let Some((key, vault_id)) = self.vault.open_key() else {
            done.closed = kept.len();
            return Ok(done);
        };
        let mut closed = Vec::new();
        for old in kept {
            // The row it gave way to is gone again: this is the key in use.
            if old.vault_id == vault_id {
                continue;
            }
            let mut old_key = self.vault.replaced_key(&old.vault_id);
            if old_key.is_none() && !old.behind_lock {
                old_key = old.open(DEFAULT_SECRET).await;
            }
            if old_key.is_none() && secret != DEFAULT_SECRET {
                old_key = old.open(secret).await;
            }
            if old_key.is_none() && previous {
                old_key = old.open_with_recovery(secret).await;
            }
            let moved = match old_key {
                Some(old_key) => {
                    reencrypt(
                        &self.db,
                        &self.key_users,
                        &old_key,
                        &old.vault_id,
                        &key,
                        &vault_id,
                    )
                    .await?
                }
                None => false,
            };
            if moved {
                done.opened += 1;
            } else {
                closed.push(old);
            }
        }
        done.closed = closed.len();
        save_replaced(&self.db, &closed).await?;
        let waiting: Vec<&str> = closed.iter().map(|k| k.vault_id.as_str()).collect();
        self.vault.retain_replaced(&waiting);
        Ok(done)
    }

    /// `adopt_replaced_keys` where the vault key became open by other means than
    /// an unlock, and the caller's work stands whether or not a replaced key opens.
    pub(crate) async fn reopen_replaced_keys(&self, secret: &str) {
        if let Err(e) = self.adopt_replaced_keys(secret, false).await {
            eprintln!("vault: {e}");
        }
    }

    /// How many replaced keys wait for the secret that opens them: what they
    /// encrypted on this device cannot be read until then.
    pub async fn closed_keys(&self) -> Result<usize> {
        let own = key_id(&self.db).await?;
        let kept = load_replaced(&self.db).await?;
        Ok(kept
            .iter()
            .filter(|k| Some(&k.vault_id) != own.as_ref())
            .count())
    }

    /// Open the replaced keys with a secret the user names for them: the PIN or
    /// password this device had before a synced row brought another lock, or the
    /// recovery key that went with it. The vault must be open. Returns how many
    /// keys stay closed; a secret that opens none is an error.
    pub async fn open_replaced_keys(&self, secret: &str) -> Result<usize> {
        self.session.throttle().await;
        let opened = async {
            self.require_open()?;
            let done = self.adopt_replaced_keys(secret, true).await?;
            if done.opened == 0 && done.closed > 0 {
                return Err(Error::other("Wrong password"));
            }
            Ok(done.closed)
        }
        .await;
        match &opened {
            Ok(_) => self.emit(Event::Changed),
            Err(_) => self.session.record_failure(),
        }
        opened
    }

    /// `none` | `ok` | `mismatch` for the lock status payload.
    pub(crate) async fn vault_label(&self) -> String {
        match self.vault.label() {
            "ok" => "ok".into(),
            "mismatch" => "mismatch".into(),
            "none" => "none".into(),
            _ => match load_row(&self.db).await {
                Ok(Some(_)) => "ok".into(),
                _ => "none".into(),
            },
        }
    }

    /// The vault key, when the vault is open.
    pub fn require_open(&self) -> Result<(SecretKey, String)> {
        if let Some(open) = self.vault.open_key() {
            return Ok(open);
        }
        if self.vault.is_mismatch() {
            Err(Error::VaultMismatch)
        } else {
            Err(Error::VaultLocked)
        }
    }

    /// The vault key, made on first use: without a row the staged key goes
    /// into a new row, wrapped by the secret that staged it — the session's,
    /// or the built-in one without a lock. Returns the key and the vault id.
    pub async fn ensure_key(&self) -> Result<(SecretKey, String)> {
        if let Some(open) = self.vault.open_key() {
            return Ok(open);
        }
        if self.vault.is_mismatch() {
            return Err(Error::VaultMismatch);
        }
        let Some((kek, salt)) = self.vault.pending() else {
            return Err(Error::VaultLocked);
        };
        if let Some(row) = load_row(&self.db).await? {
            return self.adopt_existing(kek, row).await;
        }

        let vault_id = Uuid::new_v4().to_string();
        let key = SecretKey::random();
        let wrapped = wrap_key(&key, &kek, &vault_id).map_err(|_| Error::DecryptFailed)?;
        let meta = self.session.meta();
        let inserted = insert_row(&self.db, &vault_id, salt, &wrapped, meta.as_ref()).await;

        if let Err(e) = inserted {
            if e.to_string().contains("UNIQUE") {
                if let Some(row) = load_row(&self.db).await? {
                    return self.adopt_existing(kek, row).await;
                }
            }
            return Err(Error::db(e));
        }
        self.vault.set_open(kek, key.clone_key(), vault_id.clone());
        self.drop_legacy().await;
        self.reopen_replaced_keys(DEFAULT_SECRET).await;
        Ok((key, vault_id))
    }

    async fn adopt_existing(&self, kek: SecretKey, row: VaultRow) -> Result<(SecretKey, String)> {
        let vault_id = row.vault_id;
        let wrapped = row.wrapped_key;
        let kek_job = kek.clone_key();
        let id_job = vault_id.clone();
        let key = match blocking(move || unwrap_key(&wrapped, &kek_job, &id_job)).await {
            Ok(key) => key,
            Err(_) => {
                self.vault.set_mismatch();
                return Err(Error::VaultMismatch);
            }
        };
        self.vault.set_open(kek, key.clone_key(), vault_id.clone());
        self.reopen_replaced_keys(DEFAULT_SECRET).await;
        Ok((key, vault_id))
    }

    /// Drop or refresh the key in memory after a synced key row arrived.
    pub(crate) async fn refresh_key(&self) {
        let row = match load_row(&self.db).await {
            Ok(row) => row,
            Err(_) => return,
        };
        // The open key is that of a row which is gone. No secret at hand may wrap
        // it, so it stays in memory until what it encrypted can move to the new key.
        if let Some((key, vault_id)) = self.vault.open_key() {
            if row.as_ref().map(|r| r.vault_id.as_str()) != Some(vault_id.as_str()) {
                self.vault.hold_replaced(key, vault_id);
            }
        }
        let Some(kek) = self.vault.kek() else { return };
        let Some(row) = row else {
            self.vault.set_pending(kek, random_salt());
            return;
        };
        if row.crypto_version != 1 {
            self.vault.set_mismatch();
            return;
        }
        let wrapped = row.wrapped_key;
        let vault_id = row.vault_id;
        let kek_job = kek.clone_key();
        let id_job = vault_id.clone();
        match blocking(move || unwrap_key(&wrapped, &kek_job, &id_job)).await {
            Ok(key) => {
                self.vault.set_open(kek, key, vault_id);
                self.reopen_replaced_keys(DEFAULT_SECRET).await;
            }
            Err(_) => self.vault.set_mismatch(),
        }
    }
}

/// What happens to the key of a row that a synced row takes the place of.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{key_row, pool, PASSWORDS};

    /// The lock of a product whose module keeps passwords under the key.
    fn with_passwords(db: &Pool<Sqlite>) -> Lock {
        Lock::with_key_users(db.clone(), vec![PASSWORDS])
    }

    async fn store_password(db: &Pool<Sqlite>, key: &SecretKey, vault_id: &str) {
        sqlx::query(
            "INSERT INTO passwords (id, password_enc, note_enc, vault_id)
             VALUES ('pw-1', ?, ?, ?)",
        )
        .bind(encrypt_field(key, "pw-1", "password", "hunter2").unwrap())
        .bind(encrypt_field(key, "pw-1", "note", "the panel").unwrap())
        .bind(vault_id)
        .execute(db)
        .await
        .unwrap();
    }

    async fn store_secret(db: &Pool<Sqlite>, key: &SecretKey) {
        let value = encrypt_field(
            key,
            &secrets::aad("messenger", "nsec"),
            secrets::FIELD,
            "the messenger key",
        );
        sqlx::query("INSERT INTO lock_secrets (box, key, value) VALUES ('messenger', 'nsec', ?)")
            .bind(value.unwrap())
            .execute(db)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_key_that_holds_nothing_is_not_kept() {
        let (db, _dir) = pool().await;
        key_row(&db, "own", "1234").await;

        with_passwords(&db)
            .check_replaceable("theirs")
            .await
            .unwrap();
        with_passwords(&db)
            .keep_replaced_key(Some("theirs"))
            .await
            .unwrap();
        with_passwords(&db).keep_replaced_key(None).await.unwrap();

        assert!(load_replaced(&db).await.unwrap().is_empty());
        db.close().await;
    }

    /// The lock reads no table of a module: what a module keeps under the key
    /// counts once the module tells the lock of it.
    #[tokio::test]
    async fn a_key_a_module_keeps_entries_under_is_kept() {
        let (db, _dir) = pool().await;
        let key = key_row(&db, "own", "1234").await;
        store_password(&db, &key, "own").await;

        Lock::new(db.clone())
            .keep_replaced_key(Some("theirs"))
            .await
            .unwrap();
        assert!(load_replaced(&db).await.unwrap().is_empty());

        let refused = with_passwords(&db)
            .check_replaceable("theirs")
            .await
            .unwrap_err();
        assert_eq!(refused.to_string(), ERR_JOIN_OWN_LOCK);
        with_passwords(&db)
            .keep_replaced_key(Some("theirs"))
            .await
            .unwrap();
        let kept = load_replaced(&db).await.unwrap();
        assert_eq!(kept.len(), 1);
        assert!(kept[0].open("1234").await.is_some());
        db.close().await;
    }

    #[tokio::test]
    async fn a_row_with_the_same_key_replaces_nothing() {
        let (db, _dir) = pool().await;
        let key = key_row(&db, "own", "1234").await;
        store_password(&db, &key, "own").await;

        with_passwords(&db).check_replaceable("own").await.unwrap();
        with_passwords(&db)
            .keep_replaced_key(Some("own"))
            .await
            .unwrap();

        assert!(load_replaced(&db).await.unwrap().is_empty());
        db.close().await;
    }

    #[tokio::test]
    async fn what_a_replaced_key_encrypted_moves_to_the_new_key() {
        let (db, _dir) = pool().await;
        let own = key_row(&db, "own", DEFAULT_SECRET).await;
        store_password(&db, &own, "own").await;
        store_secret(&db, &own).await;

        with_passwords(&db)
            .check_replaceable("theirs")
            .await
            .unwrap();
        with_passwords(&db)
            .keep_replaced_key(Some("theirs"))
            .await
            .unwrap();
        let kept = load_replaced(&db).await.unwrap();
        assert_eq!(kept.len(), 1);
        assert!(kept[0].open("1234").await.is_none());
        let reopened = kept[0]
            .open(DEFAULT_SECRET)
            .await
            .expect("the kept wrap opens");

        let theirs = SecretKey::random();
        assert!(
            reencrypt(&db, &[PASSWORDS], &reopened, "own", &theirs, "theirs")
                .await
                .unwrap()
        );

        let (vault_id, password, note): (String, String, String) = sqlx::query_as(
            "SELECT vault_id, password_enc, note_enc FROM passwords WHERE id = 'pw-1'",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(vault_id, "theirs");
        assert_eq!(
            decrypt_field(&theirs, "pw-1", "password", &password).unwrap(),
            "hunter2"
        );
        assert_eq!(
            decrypt_field(&theirs, "pw-1", "note", &note).unwrap(),
            "the panel"
        );
        assert!(decrypt_field(&own, "pw-1", "password", &password).is_err());
        let secret: String = sqlx::query_scalar(
            "SELECT value FROM lock_secrets WHERE box = 'messenger' AND key = 'nsec'",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        let aad = secrets::aad("messenger", "nsec");
        assert_eq!(
            decrypt_field(&theirs, &aad, secrets::FIELD, &secret).unwrap(),
            "the messenger key"
        );

        db.close().await;
    }

    #[tokio::test]
    async fn a_key_behind_a_lock_is_not_given_up_at_a_join_but_is_kept_when_replaced() {
        let (db, _dir) = pool().await;
        let own = key_row(&db, "own", "1234").await;
        store_secret(&db, &own).await;

        let refused = with_passwords(&db)
            .check_replaceable("theirs")
            .await
            .unwrap_err();
        assert_eq!(refused.to_string(), ERR_JOIN_OWN_LOCK);

        // A peer's row or tombstone asks nobody: the wrap stays for the secret that opens it.
        with_passwords(&db).keep_replaced_key(None).await.unwrap();
        let kept = load_replaced(&db).await.unwrap();
        assert_eq!(kept.len(), 1);
        assert!(kept[0].open(DEFAULT_SECRET).await.is_none());
        assert!(kept[0].open("1234").await.is_some());

        db.close().await;
    }

    #[tokio::test]
    async fn a_kept_key_opens_with_the_recovery_key_of_its_row() {
        let (db, _dir) = pool().await;
        let own = key_row(&db, "own", "1234").await;
        store_secret(&db, &own).await;
        let code = install_recovery(&db, &own, "own").await.unwrap();

        with_passwords(&db)
            .keep_replaced_key(Some("theirs"))
            .await
            .unwrap();
        let kept = load_replaced(&db).await.unwrap();
        assert!(kept[0].open_with_recovery(&code).await.is_some());
        assert!(kept[0]
            .open_with_recovery(&recovery::generate())
            .await
            .is_none());
        assert!(kept[0].open_with_recovery("1234").await.is_none());

        db.close().await;
    }

    #[tokio::test]
    async fn a_kept_key_whose_row_named_a_lock_is_not_tried_with_the_built_in_secret() {
        let (db, _dir) = pool().await;
        let own = key_row(&db, "own", "1234").await;
        store_secret(&db, &own).await;
        with_passwords(&db)
            .keep_replaced_key(Some("theirs"))
            .await
            .unwrap();
        assert!(!load_replaced(&db).await.unwrap()[0].behind_lock);

        let lock = LockMeta {
            hash: "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA".into(),
            kind: Some("pin".into()),
            hint: None,
        };
        store_lock_meta(&db, Some(&lock)).await.unwrap();
        with_passwords(&db)
            .keep_replaced_key(Some("theirs"))
            .await
            .unwrap();
        let kept = load_replaced(&db).await.unwrap();
        assert_eq!(kept.len(), 1);
        assert!(kept[0].behind_lock);

        db.close().await;
    }
}
