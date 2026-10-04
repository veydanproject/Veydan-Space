// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Password-vault cryptography: Argon2id KEK, random vault key, XChaCha20-Poly1305.
//! Keys are zeroized on drop and never printed.

use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand::Rng;
use zeroize::Zeroizing;

const FIELD_PREFIX: &str = "v1:";
const WRAP_AAD: &str = "veydan/password-vault/v1";

pub const KDF_MEMORY_KIB: u32 = 64 * 1024;
pub const KDF_ITERATIONS: u32 = 3;
pub const KDF_PARALLELISM: u32 = 1;

const KDF_MAX_MEMORY_KIB: u32 = 1024 * 1024;
const KDF_MAX_ITERATIONS: u32 = 16;
const KDF_MAX_PARALLELISM: u32 = 8;

#[derive(Debug)]
pub enum CryptoError {
    Kdf,
    Encrypt,
    Decrypt,
}

/// 256-bit key. Debug output does not include the bytes.
pub struct SecretKey(Zeroizing<[u8; 32]>);

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretKey")
    }
}

impl SecretKey {
    pub fn random() -> Self {
        Self(Zeroizing::new(random_array()))
    }

    pub fn clone_key(&self) -> Self {
        Self(Zeroizing::new(*self.0))
    }

    fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Clone)]
pub struct KdfParams {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
    pub salt: [u8; 16],
}

impl KdfParams {
    pub fn production(salt: [u8; 16]) -> Self {
        Self {
            memory_kib: KDF_MEMORY_KIB,
            iterations: KDF_ITERATIONS,
            parallelism: KDF_PARALLELISM,
            salt,
        }
    }
}

pub fn random_salt() -> [u8; 16] {
    random_array()
}

fn random_array<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    rand::rng().fill_bytes(&mut out);
    out
}

/// Argon2id -> 32-byte KEK. Rejects parameters that could exhaust memory.
pub fn derive_kek(password: &str, params: &KdfParams) -> Result<SecretKey, CryptoError> {
    if params.memory_kib == 0
        || params.memory_kib > KDF_MAX_MEMORY_KIB
        || params.iterations == 0
        || params.iterations > KDF_MAX_ITERATIONS
        || params.parallelism == 0
        || params.parallelism > KDF_MAX_PARALLELISM
    {
        return Err(CryptoError::Kdf);
    }
    let argon_params = Params::new(
        params.memory_kib,
        params.iterations,
        params.parallelism,
        Some(32),
    )
    .map_err(|_| CryptoError::Kdf)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon_params);
    let mut out = [0u8; 32];
    argon
        .hash_password_into(password.as_bytes(), &params.salt, &mut out)
        .map_err(|_| CryptoError::Kdf)?;
    Ok(SecretKey(Zeroizing::new(out)))
}

pub fn field_aad(id: &str, field: &str) -> String {
    format!("veydan/password/v1/{id}/{field}")
}

/// `v1:` + base64(nonce || ciphertext). A fresh nonce every call.
pub fn encrypt(key: &SecretKey, aad: &str, plaintext: &[u8]) -> Result<String, CryptoError> {
    let cipher = XChaCha20Poly1305::new(key.as_bytes().into());
    let nonce: [u8; 24] = random_array();
    let ct = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| CryptoError::Encrypt)?;
    let mut raw = Vec::with_capacity(nonce.len() + ct.len());
    raw.extend_from_slice(&nonce);
    raw.extend_from_slice(&ct);
    Ok(format!("{FIELD_PREFIX}{}", B64.encode(raw)))
}

pub fn decrypt(key: &SecretKey, aad: &str, stored: &str) -> Result<Vec<u8>, CryptoError> {
    let raw = decode_envelope(stored)?;
    if raw.len() < 24 + 16 {
        return Err(CryptoError::Decrypt);
    }
    let (nonce, ct) = raw.split_at(24);
    let cipher = XChaCha20Poly1305::new(key.as_bytes().into());
    cipher
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ct,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| CryptoError::Decrypt)
}

/// Wrap the vault key under the KEK. AAD binds the blob to `vault_id`.
pub fn wrap_key(
    vault_key: &SecretKey,
    kek: &SecretKey,
    vault_id: &str,
) -> Result<String, CryptoError> {
    let aad = format!("{WRAP_AAD}|{vault_id}");
    encrypt(kek, &aad, vault_key.as_bytes())
}

pub fn unwrap_key(stored: &str, kek: &SecretKey, vault_id: &str) -> Result<SecretKey, CryptoError> {
    let aad = format!("{WRAP_AAD}|{vault_id}");
    let pt = decrypt(kek, &aad, stored)?;
    let bytes: [u8; 32] = pt.try_into().map_err(|_| CryptoError::Decrypt)?;
    Ok(SecretKey(Zeroizing::new(bytes)))
}

fn decode_envelope(stored: &str) -> Result<Vec<u8>, CryptoError> {
    let b64 = stored
        .strip_prefix(FIELD_PREFIX)
        .ok_or(CryptoError::Decrypt)?;
    B64.decode(b64).map_err(|_| CryptoError::Decrypt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> SecretKey {
        SecretKey::random()
    }

    #[test]
    fn roundtrip() {
        let key = key();
        let aad = field_aad("id-1", "password");
        let ct = encrypt(&key, &aad, b"secret").unwrap();
        assert_eq!(decrypt(&key, &aad, &ct).unwrap(), b"secret");
    }

    #[test]
    fn wrong_key_fails() {
        let ct = encrypt(&key(), &field_aad("id", "password"), b"secret").unwrap();
        assert!(decrypt(&key(), &field_aad("id", "password"), &ct).is_err());
    }

    #[test]
    fn tamper_fails() {
        let key = key();
        let aad = field_aad("id", "password");
        let mut ct = encrypt(&key, &aad, b"secret").unwrap();
        let last = ct.pop().unwrap();
        ct.push(if last == 'A' { 'B' } else { 'A' });
        assert!(decrypt(&key, &aad, &ct).is_err());
    }

    #[test]
    fn nonce_differs() {
        let key = key();
        let aad = field_aad("id", "password");
        let a = encrypt(&key, &aad, b"password123").unwrap();
        let b = encrypt(&key, &aad, b"password123").unwrap();
        assert_ne!(a, b);
        assert_eq!(decrypt(&key, &aad, &a).unwrap(), b"password123");
        assert_eq!(decrypt(&key, &aad, &b).unwrap(), b"password123");
    }

    #[test]
    fn aad_binds_record() {
        let key = key();
        let ct = encrypt(&key, &field_aad("a", "password"), b"secret").unwrap();
        assert!(decrypt(&key, &field_aad("b", "password"), &ct).is_err());
        assert!(decrypt(&key, &field_aad("a", "note"), &ct).is_err());
    }

    #[test]
    fn wrap_roundtrip_and_wrong_vault() {
        let kek = key();
        let vault = key();
        let wrapped = wrap_key(&vault, &kek, "vault-1").unwrap();
        let opened = unwrap_key(&wrapped, &kek, "vault-1").unwrap();
        assert_eq!(opened.as_bytes(), vault.as_bytes());
        assert!(unwrap_key(&wrapped, &kek, "vault-2").is_err());
        assert!(unwrap_key(&wrapped, &key(), "vault-1").is_err());
    }

    #[test]
    fn debug_hides_key() {
        let rendered = format!("{:?}", key());
        assert_eq!(rendered, "SecretKey");
    }
}
