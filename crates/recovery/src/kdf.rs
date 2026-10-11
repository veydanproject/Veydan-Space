// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Argon2id and XChaCha20-Poly1305 as the lock uses them
//! (`crates/lock/src/crypto.rs`): the same parameters and the same
//! ceilings, copied so that this crate depends on nothing of the app.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

/// `KdfParams::production` of the lock: 64 MiB, 3 passes, 1 lane.
pub const KDF_MEMORY_KIB: u32 = 64 * 1024;
pub const KDF_ITERATIONS: u32 = 3;
pub const KDF_PARALLELISM: u32 = 1;
/// A header is plaintext: a tampered one must not be able to exhaust the
/// memory or the CPU of the reader before the AEAD fails.
pub const KDF_MAX_MEMORY_KIB: u32 = 1024 * 1024;
pub const KDF_MAX_ITERATIONS: u32 = 16;
pub const KDF_MAX_PARALLELISM: u32 = 8;

pub const SALT_LEN: usize = 16;
pub const NONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;
pub const KEY_LEN: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KdfError {
    #[error("kdf parameters out of range")]
    Params,
    #[error("kdf failed")]
    Kdf,
    #[error("encryption failed")]
    Encrypt,
    #[error("decryption failed")]
    Decrypt,
    #[error("the system random source failed")]
    Random,
}

/// Argon2id parameters without the salt: the salt goes beside them in
/// every format that carries them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl KdfParams {
    pub const fn production() -> Self {
        Self {
            memory_kib: KDF_MEMORY_KIB,
            iterations: KDF_ITERATIONS,
            parallelism: KDF_PARALLELISM,
        }
    }

    /// Whether a reader may run these parameters at all.
    pub fn within_ceilings(&self) -> bool {
        self.memory_kib > 0
            && self.memory_kib <= KDF_MAX_MEMORY_KIB
            && self.iterations > 0
            && self.iterations <= KDF_MAX_ITERATIONS
            && self.parallelism > 0
            && self.parallelism <= KDF_MAX_PARALLELISM
    }
}

/// Argon2id of `password` and `salt` into `out` (any length the
/// algorithm allows). Parameters above the ceilings are refused before
/// anything is allocated.
pub fn argon2id(
    password: &[u8],
    salt: &[u8],
    params: &KdfParams,
    out: &mut [u8],
) -> Result<(), KdfError> {
    if !params.within_ceilings() {
        return Err(KdfError::Params);
    }
    let argon = argon2_for(params, out.len())?;
    argon
        .hash_password_into(password, salt, out)
        .map_err(|_| KdfError::Kdf)
}

/// The Argon2id instance of `params` for an output of `out_len` bytes.
pub(crate) fn argon2_for(params: &KdfParams, out_len: usize) -> Result<Argon2<'static>, KdfError> {
    let argon_params = Params::new(
        params.memory_kib,
        params.iterations,
        params.parallelism,
        Some(out_len),
    )
    .map_err(|_| KdfError::Params)?;
    Ok(Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        argon_params,
    ))
}

/// XChaCha20-Poly1305: the ciphertext with its tag, no nonce inside.
pub fn seal(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, KdfError> {
    let cipher = XChaCha20Poly1305::new(key.into());
    cipher
        .encrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| KdfError::Encrypt)
}

pub fn open(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    ciphertext: &[u8],
) -> Result<Zeroizing<Vec<u8>>, KdfError> {
    if ciphertext.len() < TAG_LEN {
        return Err(KdfError::Decrypt);
    }
    let cipher = XChaCha20Poly1305::new(key.into());
    cipher
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| KdfError::Decrypt)
}

/// Bytes from the system random source.
pub fn random(out: &mut [u8]) -> Result<(), KdfError> {
    getrandom::fill(out).map_err(|_| KdfError::Random)
}

pub fn random_array<const N: usize>() -> Result<[u8; N], KdfError> {
    let mut out = [0u8; N];
    random(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_is_the_lock_parameters() {
        let p = KdfParams::production();
        assert_eq!((p.memory_kib, p.iterations, p.parallelism), (65536, 3, 1));
        assert!(p.within_ceilings());
    }

    #[test]
    fn ceilings_refuse_before_work() {
        let mut out = [0u8; 32];
        let too_much = KdfParams {
            memory_kib: 2 * 1024 * 1024,
            iterations: 1,
            parallelism: 1,
        };
        assert_eq!(
            argon2id(b"pw", &[0u8; 16], &too_much, &mut out),
            Err(KdfError::Params)
        );
        let zero = KdfParams {
            memory_kib: 0,
            iterations: 1,
            parallelism: 1,
        };
        assert!(!zero.within_ceilings());
    }

    #[test]
    fn argon2id_is_deterministic() {
        let p = KdfParams {
            memory_kib: 64,
            iterations: 1,
            parallelism: 1,
        };
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        argon2id(b"pw", &[7u8; 16], &p, &mut a).unwrap();
        argon2id(b"pw", &[7u8; 16], &p, &mut b).unwrap();
        assert_eq!(a, b);
        argon2id(b"pw", &[8u8; 16], &p, &mut b).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn aead_roundtrip_and_aad() {
        let key = [1u8; 32];
        let nonce = [2u8; 24];
        let ct = seal(&key, &nonce, b"aad", b"hello").unwrap();
        assert_eq!(ct.len(), 5 + TAG_LEN);
        assert_eq!(
            open(&key, &nonce, b"aad", &ct).unwrap().as_slice(),
            b"hello"
        );
        assert_eq!(open(&key, &nonce, b"other", &ct), Err(KdfError::Decrypt));
        assert_eq!(
            open(&[3u8; 32], &nonce, b"aad", &ct),
            Err(KdfError::Decrypt)
        );
        assert_eq!(
            open(&key, &nonce, b"aad", &ct[..10]),
            Err(KdfError::Decrypt)
        );
    }

    #[test]
    fn random_fills() {
        let a: [u8; 16] = random_array().unwrap();
        let b: [u8; 16] = random_array().unwrap();
        assert_ne!(a, b);
    }
}
