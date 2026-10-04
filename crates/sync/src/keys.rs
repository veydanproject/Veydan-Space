// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Key hierarchy:
//!
//! ```text
//! passphrase -> Argon2id -> KEK -> unwrap -> VaultMasterKey (random 256 bit)
//! VaultMasterKey -> HKDF-SHA256 -> { log key, blob key, id key }
//!                                -> { large-file/chunk, large-file/manifest, large-file/id }
//! ```
//!
//! Large-file keys are separate labels so v2 objects never share key material
//! with v1 logs and blobs. `large-file/id` is vault-wide: equal plaintext chunks
//! in any file of the vault get the same chunk id (shared chunks).
//!
//! The master key is random; the passphrase only protects it, so changing the
//! passphrase rewrites `manifest.json` and nothing else.

use crate::{random_bytes, Result, SyncError};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

pub const MANIFEST_KEY: &str = "manifest.json";
const MANIFEST_VERSION: u32 = 1;
const MANIFEST_AAD: &str = "veydan-sync/manifest/v1";

const ARGON_M_COST_KIB: u32 = 64 * 1024;
const ARGON_T_COST: u32 = 3;
const ARGON_P_COST: u32 = 1;
// Upper bounds for parameters read from a manifest: it is plaintext, so a
// tampered file must not be able to exhaust memory or CPU before AEAD fails.
const ARGON_MAX_M_COST_KIB: u32 = 1024 * 1024;
const ARGON_MAX_T_COST: u32 = 16;
const ARGON_MAX_P_COST: u32 = 8;

/// Vault master key. Never leaves the device unwrapped.
#[derive(Clone)]
pub struct Vmk(pub [u8; 32]);

impl Vmk {
    pub fn to_base64(&self) -> String {
        B64.encode(self.0)
    }

    pub fn from_base64(s: &str) -> Result<Vmk> {
        let raw = B64
            .decode(s)
            .map_err(|e| SyncError::Format(e.to_string()))?;
        let arr: [u8; 32] = raw
            .try_into()
            .map_err(|_| SyncError::Format("master key must be 32 bytes".into()))?;
        Ok(Vmk(arr))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KdfParams {
    pub algo: String,
    pub m_cost_kib: u32,
    pub t_cost: u32,
    pub p_cost: u32,
    /// base64, 16 bytes
    pub salt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrappedKey {
    /// base64, 24 bytes
    pub nonce: String,
    /// base64, 32 + 16 bytes
    pub ct: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    /// 32 hex chars, random per vault
    pub vault_id: String,
    pub kdf: KdfParams,
    pub wrapped_vmk: WrappedKey,
}

/// Sub-keys derived from the master key for one vault.
#[derive(Clone)]
pub struct Keys {
    pub log: [u8; 32],
    pub blob: [u8; 32],
    pub id: [u8; 32],
    /// Large files v2: chunk ciphertext.
    pub lf_chunk: [u8; 32],
    /// Large files v2: manifest ciphertext.
    pub lf_manifest: [u8; 32],
    /// Large files v2: HMAC key for chunk / file / manifest ids, one per vault.
    pub lf_id: [u8; 32],
}

fn derive_kek(passphrase: &str, kdf: &KdfParams) -> Result<[u8; 32]> {
    if kdf.algo != "argon2id" {
        return Err(SyncError::Format(format!("unsupported kdf {}", kdf.algo)));
    }
    if kdf.m_cost_kib > ARGON_MAX_M_COST_KIB
        || kdf.t_cost > ARGON_MAX_T_COST
        || kdf.p_cost > ARGON_MAX_P_COST
    {
        return Err(SyncError::Format("kdf parameters out of range".into()));
    }
    let salt = B64
        .decode(&kdf.salt)
        .map_err(|e| SyncError::Format(e.to_string()))?;
    let params = Params::new(kdf.m_cost_kib, kdf.t_cost, kdf.p_cost, Some(32))
        .map_err(|e| SyncError::Crypto(e.to_string()))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    argon
        .hash_password_into(passphrase.as_bytes(), &salt, &mut out)
        .map_err(|e| SyncError::Crypto(e.to_string()))?;
    Ok(out)
}

fn wrap_vmk(vmk: &Vmk, kek: &[u8; 32], vault_id: &str) -> Result<WrappedKey> {
    let cipher = XChaCha20Poly1305::new(kek.into());
    let nonce_bytes = random_bytes(24);
    let nonce = XNonce::from_slice(&nonce_bytes);
    let aad = format!("{MANIFEST_AAD}|{vault_id}");
    let ct = cipher
        .encrypt(
            nonce,
            Payload {
                msg: &vmk.0,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| SyncError::Crypto("wrap failed".into()))?;
    Ok(WrappedKey {
        nonce: B64.encode(nonce_bytes),
        ct: B64.encode(ct),
    })
}

fn unwrap_vmk(wrapped: &WrappedKey, kek: &[u8; 32], vault_id: &str) -> Result<Vmk> {
    let cipher = XChaCha20Poly1305::new(kek.into());
    let nonce_bytes = B64
        .decode(&wrapped.nonce)
        .map_err(|e| SyncError::Format(e.to_string()))?;
    let ct = B64
        .decode(&wrapped.ct)
        .map_err(|e| SyncError::Format(e.to_string()))?;
    if nonce_bytes.len() != 24 {
        return Err(SyncError::Format("bad nonce length".into()));
    }
    let aad = format!("{MANIFEST_AAD}|{vault_id}");
    let pt = cipher
        .decrypt(
            XNonce::from_slice(&nonce_bytes),
            Payload {
                msg: &ct,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| SyncError::WrongPassphrase)?;
    let arr: [u8; 32] = pt
        .try_into()
        .map_err(|_| SyncError::Format("bad key length".into()))?;
    Ok(Vmk(arr))
}

impl Manifest {
    /// New vault: random id, random master key, wrapped under the passphrase.
    pub fn create(passphrase: &str) -> Result<(Manifest, Vmk)> {
        let vault_id = hex::encode(random_bytes(16));
        let kdf = KdfParams {
            algo: "argon2id".into(),
            m_cost_kib: ARGON_M_COST_KIB,
            t_cost: ARGON_T_COST,
            p_cost: ARGON_P_COST,
            salt: B64.encode(random_bytes(16)),
        };
        let vmk = Vmk(random_bytes(32).try_into().expect("32 bytes"));
        let kek = derive_kek(passphrase, &kdf)?;
        let wrapped_vmk = wrap_vmk(&vmk, &kek, &vault_id)?;
        Ok((
            Manifest {
                version: MANIFEST_VERSION,
                vault_id,
                kdf,
                wrapped_vmk,
            },
            vmk,
        ))
    }

    pub fn unlock(&self, passphrase: &str) -> Result<Vmk> {
        if self.version != MANIFEST_VERSION {
            return Err(SyncError::Format(format!(
                "unsupported manifest version {}",
                self.version
            )));
        }
        let kek = derive_kek(passphrase, &self.kdf)?;
        unwrap_vmk(&self.wrapped_vmk, &kek, &self.vault_id)
    }

    /// Rewrap the same master key under a new passphrase.
    pub fn rewrap(&mut self, vmk: &Vmk, new_passphrase: &str) -> Result<()> {
        self.kdf.salt = B64.encode(random_bytes(16));
        let kek = derive_kek(new_passphrase, &self.kdf)?;
        self.wrapped_vmk = wrap_vmk(vmk, &kek, &self.vault_id)?;
        Ok(())
    }

    pub fn to_json(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec_pretty(self)?)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Manifest> {
        Ok(serde_json::from_slice(bytes)?)
    }
}

impl Keys {
    pub fn derive(vmk: &Vmk, vault_id: &str) -> Keys {
        let hk = Hkdf::<Sha256>::new(Some(vault_id.as_bytes()), &vmk.0);
        let expand = |info: &[u8]| {
            let mut out = [0u8; 32];
            hk.expand(info, &mut out)
                .expect("32 bytes is a valid HKDF length");
            out
        };
        Keys {
            log: expand(b"veydan-sync/log"),
            blob: expand(b"veydan-sync/blob"),
            id: expand(b"veydan-sync/id"),
            lf_chunk: expand(b"large-file/chunk"),
            lf_manifest: expand(b"large-file/manifest"),
            lf_id: expand(b"large-file/id"),
        }
    }
}
