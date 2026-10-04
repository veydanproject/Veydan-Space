// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use crate::error::{AppError, CmdResult};
use chrono::Utc;
use russh::keys::ssh_key::private::{EcdsaKeypair, Ed25519Keypair, KeypairData, RsaKeypair};
use russh::keys::ssh_key::{EcdsaCurve, LineEnding};
use russh::keys::{decode_secret_key, Algorithm, HashAlg, PrivateKey};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use tauri::State;
use uuid::Uuid;
use veydan_core::Core;

// Stable error prefixes the frontend maps to specific i18n messages.
pub const ERR_KEY_ENCRYPTED: &str = "key_encrypted";
pub const ERR_KEY_WRONG_PASSPHRASE: &str = "key_wrong_passphrase";

// ── Models ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct SshKey {
    pub id: String,
    pub name: String,
    pub algorithm: String,
    pub bits: Option<i64>,
    pub comment: Option<String>,
    // Private material is only handed out by `ssh_key_export_private`.
    #[serde(skip_serializing)]
    pub private_key: String,
    pub public_key: String,
    #[serde(skip_serializing)]
    pub passphrase: Option<String>,
    #[sqlx(default)]
    pub has_passphrase: bool,
    pub fingerprint: Option<String>,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
    pub usage_count: i64,
}

#[derive(Debug, Deserialize)]
pub struct SshKeyImportInput {
    pub name: String,
    pub private_key: String,
    pub passphrase: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SshKeyGenerateInput {
    pub name: String,
    pub algorithm: String,
    pub bits: Option<u32>,
    pub comment: Option<String>,
    pub passphrase: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SshKeyUpdateInput {
    pub name: Option<String>,
    pub comment: Option<String>,
}

const SELECT_KEY: &str = "SELECT k.*, \
    (k.passphrase IS NOT NULL AND k.passphrase != '') AS has_passphrase, \
    (SELECT COUNT(*) FROM ssh_connections c WHERE c.ssh_key_id = k.id) AS usage_count \
    FROM ssh_keys k";

// ── Key parsing / metadata ─────────────────────────────────────────────────────

/// Parse a pasted private key, decrypting it when a passphrase is supplied.
/// Mirrors the parse chain in `ssh::do_authenticate`.
pub fn parse_imported(pem: &str, passphrase: Option<&str>) -> Result<PrivateKey, AppError> {
    let pem = pem.trim();
    let pass = passphrase.filter(|p| !p.is_empty());
    if let Ok(key) = PrivateKey::from_openssh(pem) {
        if key.is_encrypted() {
            let pass = pass.ok_or_else(|| AppError::other(ERR_KEY_ENCRYPTED))?;
            return key
                .decrypt(pass)
                .map_err(|_| AppError::other(ERR_KEY_WRONG_PASSPHRASE));
        }
        return Ok(key);
    }
    // PKCS#8 / PKCS#1 / SEC1 PEM fallback
    decode_secret_key(pem, pass).map_err(|e| {
        if pass.is_some() {
            AppError::other(ERR_KEY_WRONG_PASSPHRASE)
        } else {
            AppError::other(format!("Failed to parse private key: {e}"))
        }
    })
}

/// Derive the (algorithm, bits) pair stored alongside a key.
fn algo_meta(key: &PrivateKey) -> (String, Option<i64>) {
    match key.algorithm() {
        Algorithm::Ed25519 => ("ed25519".into(), None),
        Algorithm::Rsa { .. } => {
            let bits = key
                .public_key()
                .key_data()
                .rsa()
                .map(|r| r.key_size() as i64);
            ("rsa".into(), bits)
        }
        Algorithm::Ecdsa { curve } => (
            match curve {
                EcdsaCurve::NistP256 => "ecdsa-p256",
                EcdsaCurve::NistP384 => "ecdsa-p384",
                EcdsaCurve::NistP521 => "ecdsa-p521",
            }
            .into(),
            None,
        ),
        other => (other.to_string(), None),
    }
}

pub struct KeyMaterial {
    pub algorithm: String,
    pub bits: Option<i64>,
    pub comment: Option<String>,
    pub private_pem: String,
    pub public_openssh: String,
    pub fingerprint: String,
}

pub fn derive_material(key: &PrivateKey, private_pem: String) -> Result<KeyMaterial, AppError> {
    let (algorithm, bits) = algo_meta(key);
    let public_openssh = key
        .public_key()
        .to_openssh()
        .map_err(|e| AppError::other(format!("Failed to encode public key: {e}")))?;
    let comment = key.comment().to_string();
    Ok(KeyMaterial {
        algorithm,
        bits,
        comment: (!comment.is_empty()).then_some(comment),
        private_pem,
        public_openssh,
        fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
    })
}

/// Generate a fresh keypair. Blocking (RSA takes seconds) — callers wrap in
/// `spawn_blocking`. Shared with `examples/ssh_keys_smoke.rs`.
pub fn generate_key_material(
    algorithm: &str,
    bits: Option<u32>,
    comment: String,
    passphrase: Option<&str>,
) -> Result<KeyMaterial, AppError> {
    let mut rng = rand::rng();
    let key_data = match algorithm {
        "ed25519" => KeypairData::Ed25519(Ed25519Keypair::random(&mut rng)),
        "rsa" => {
            let bits = bits.unwrap_or(4096);
            if !matches!(bits, 2048 | 3072 | 4096) {
                return Err(AppError::other(format!("Unsupported RSA size: {bits}")));
            }
            KeypairData::Rsa(
                RsaKeypair::random(&mut rng, bits as usize)
                    .map_err(|e| AppError::other(format!("RSA generation failed: {e}")))?,
            )
        }
        "ecdsa" => {
            let curve = match bits.unwrap_or(256) {
                256 => EcdsaCurve::NistP256,
                384 => EcdsaCurve::NistP384,
                521 => EcdsaCurve::NistP521,
                other => return Err(AppError::other(format!("Unsupported ECDSA curve: {other}"))),
            };
            KeypairData::Ecdsa(
                EcdsaKeypair::random(&mut rng, curve)
                    .map_err(|e| AppError::other(format!("ECDSA generation failed: {e}")))?,
            )
        }
        other => return Err(AppError::other(format!("Unknown algorithm: {other}"))),
    };

    let key = PrivateKey::new(key_data, comment)
        .map_err(|e| AppError::other(format!("Key creation failed: {e}")))?;

    let private_pem = if let Some(pass) = passphrase.filter(|p| !p.is_empty()) {
        key.encrypt(&mut rng, pass)
            .map_err(|e| AppError::other(format!("Key encryption failed: {e}")))?
            .to_openssh(LineEnding::LF)
    } else {
        key.to_openssh(LineEnding::LF)
    }
    .map_err(|e| AppError::other(format!("Failed to encode private key: {e}")))?
    .to_string();

    derive_material(&key, private_pem)
}

async fn insert_key(
    db: &sqlx::SqlitePool,
    name: &str,
    material: &KeyMaterial,
    passphrase: Option<&str>,
    source: &str,
) -> Result<String, AppError> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO ssh_keys (
            id, name, algorithm, bits, comment,
            private_key, public_key, passphrase, fingerprint, source,
            created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(name)
    .bind(&material.algorithm)
    .bind(material.bits)
    .bind(&material.comment)
    .bind(&material.private_pem)
    .bind(&material.public_openssh)
    .bind(passphrase.filter(|p| !p.is_empty()))
    .bind(&material.fingerprint)
    .bind(source)
    .bind(&now)
    .bind(&now)
    .execute(db)
    .await
    .map_err(AppError::db)?;
    Ok(id)
}

async fn get_key(db: &sqlx::SqlitePool, id: &str) -> CmdResult<SshKey> {
    sqlx::query_as::<_, SshKey>(sqlx::AssertSqlSafe(format!("{SELECT_KEY} WHERE k.id = ?")))
        .bind(id)
        .fetch_optional(db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found(format!("SSH key {id}")))
}

// ── Commands ───────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn ssh_key_list(core: State<'_, Core>) -> CmdResult<Vec<SshKey>> {
    sqlx::query_as::<_, SshKey>(sqlx::AssertSqlSafe(format!("{SELECT_KEY} ORDER BY k.name")))
        .fetch_all(&core.db)
        .await
        .map_err(AppError::db)
}

#[tauri::command]
pub async fn ssh_key_get(core: State<'_, Core>, id: String) -> CmdResult<SshKey> {
    get_key(&core.db, &id).await
}

/// Private key PEM as stored; only called from an explicit "show / copy" action.
#[tauri::command]
pub async fn ssh_key_export_private(core: State<'_, Core>, id: String) -> CmdResult<String> {
    Ok(get_key(&core.db, &id).await?.private_key)
}

#[tauri::command]
pub async fn ssh_key_import(core: State<'_, Core>, input: SshKeyImportInput) -> CmdResult<SshKey> {
    let key = parse_imported(&input.private_key, input.passphrase.as_deref())?;
    // Store the PEM verbatim as pasted; the passphrase column decrypts it at connect time.
    let material = derive_material(&key, input.private_key.trim().to_string())?;
    let id = insert_key(
        &core.db,
        &input.name,
        &material,
        input.passphrase.as_deref(),
        "imported",
    )
    .await?;
    get_key(&core.db, &id).await
}

#[tauri::command]
pub async fn ssh_key_generate(
    core: State<'_, Core>,
    input: SshKeyGenerateInput,
) -> CmdResult<SshKey> {
    let algorithm = input.algorithm.clone();
    let bits = input.bits;
    let comment = input.comment.clone().unwrap_or_default();
    let passphrase = input.passphrase.clone().filter(|p| !p.is_empty());
    let pass_for_encrypt = passphrase.clone();

    // RSA generation takes seconds — keep it off the async executor.
    let material = tokio::task::spawn_blocking(move || {
        generate_key_material(&algorithm, bits, comment, pass_for_encrypt.as_deref())
    })
    .await
    .map_err(|e| AppError::other(format!("Key generation task failed: {e}")))??;

    let id = insert_key(
        &core.db,
        &input.name,
        &material,
        passphrase.as_deref(),
        "generated",
    )
    .await?;
    get_key(&core.db, &id).await
}

#[tauri::command]
pub async fn ssh_key_update(
    core: State<'_, Core>,
    id: String,
    input: SshKeyUpdateInput,
) -> CmdResult<SshKey> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE ssh_keys SET
            name       = COALESCE(?, name),
            comment    = COALESCE(?, comment),
            updated_at = ?
         WHERE id = ?",
    )
    .bind(&input.name)
    .bind(&input.comment)
    .bind(&now)
    .bind(&id)
    .execute(&core.db)
    .await
    .map_err(AppError::db)?;
    get_key(&core.db, &id).await
}

#[tauri::command]
pub async fn ssh_key_delete(core: State<'_, Core>, id: String) -> CmdResult<()> {
    // Delete paths do not lean on the schema's ON DELETE SET NULL (see
    // `db::schemas`) — clear references explicitly before deleting.
    sqlx::query("UPDATE ssh_connections SET ssh_key_id = NULL WHERE ssh_key_id = ?")
        .bind(&id)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;
    sqlx::query("DELETE FROM ssh_keys WHERE id = ?")
        .bind(&id)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;
    Ok(())
}
