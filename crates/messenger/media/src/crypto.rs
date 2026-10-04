// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! AES-256-GCM per chunk. One random key and one random base nonce per
//! file; the nonce of chunk `i` is the base nonce with its last four bytes
//! XORed with `i` (little endian), so no nonce repeats under a key as
//! long as a file has fewer than 2^32 chunks. Output is `ciphertext ‖ tag`.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use messenger_core::{MessengerError, Result};
use sha2::{Digest, Sha256};

pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 12;
pub const TAG_LEN: usize = 16;

#[derive(Clone, PartialEq, Eq)]
pub struct FileKey {
    pub key: [u8; KEY_LEN],
    pub base_nonce: [u8; NONCE_LEN],
}

impl std::fmt::Debug for FileKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FileKey(…)")
    }
}

fn crypto(e: impl std::fmt::Display) -> MessengerError {
    MessengerError::Crypto(e.to_string())
}

impl FileKey {
    pub fn generate() -> Result<Self> {
        let mut key = [0u8; KEY_LEN];
        let mut base_nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut key).map_err(crypto)?;
        getrandom::fill(&mut base_nonce).map_err(crypto)?;
        Ok(Self { key, base_nonce })
    }

    pub fn from_parts(key: &[u8], base_nonce: &[u8]) -> Result<Self> {
        Ok(Self {
            key: key.try_into().map_err(|_| MessengerError::Crypto("file key must be 32 bytes".into()))?,
            base_nonce: base_nonce.try_into().map_err(|_| MessengerError::Crypto("nonce must be 12 bytes".into()))?,
        })
    }

    pub fn chunk_nonce(&self, index: u32) -> [u8; NONCE_LEN] {
        let mut n = self.base_nonce;
        for (b, i) in n[NONCE_LEN - 4..].iter_mut().zip(index.to_le_bytes()) {
            *b ^= i;
        }
        n
    }

    fn cipher(&self) -> Result<Aes256Gcm> {
        Aes256Gcm::new_from_slice(&self.key).map_err(crypto)
    }

    pub fn encrypt_chunk(&self, index: u32, plaintext: &[u8]) -> Result<Vec<u8>> {
        let nonce = self.chunk_nonce(index);
        self.cipher()?
            .encrypt(&Nonce::from(nonce), plaintext)
            .map_err(|_| MessengerError::Crypto("chunk encryption failed".into()))
    }

    /// Fails when the ciphertext was altered, truncated, reordered (wrong
    /// index) or made with another key.
    pub fn decrypt_chunk(&self, index: u32, ciphertext: &[u8]) -> Result<Vec<u8>> {
        let nonce = self.chunk_nonce(index);
        self.cipher()?
            .decrypt(&Nonce::from(nonce), ciphertext)
            .map_err(|_| MessengerError::Crypto("chunk does not authenticate".into()))
    }
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonce_derivation_vector() {
        let k = FileKey { key: [7; 32], base_nonce: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11] };
        assert_eq!(k.chunk_nonce(0), [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
        assert_eq!(k.chunk_nonce(1), [0, 1, 2, 3, 4, 5, 6, 7, 9, 9, 10, 11]);
        assert_eq!(k.chunk_nonce(0x0102_0304), [0, 1, 2, 3, 4, 5, 6, 7, 8 ^ 4, 9 ^ 3, 10 ^ 2, 11 ^ 1]);
        let nonces: std::collections::HashSet<_> = (0..1000u32).map(|i| k.chunk_nonce(i)).collect();
        assert_eq!(nonces.len(), 1000);
    }

    #[test]
    fn golden_ciphertext_vector() {
        // Fixed key and nonce: the format must never drift.
        let k = FileKey { key: [0x11; 32], base_nonce: [0x22; 12] };
        let ct = k.encrypt_chunk(0, b"veydan").unwrap();
        assert_eq!(ct.len(), 6 + TAG_LEN);
        assert_eq!(hex::encode(&ct), GOLDEN);
        assert_eq!(k.decrypt_chunk(0, &ct).unwrap(), b"veydan");
    }

    /// Cross-checked with an independent AES-256-GCM implementation.
    const GOLDEN: &str = "61927e2da1a15b34e55df263531d0c65bf097b3b60aa";

    #[test]
    fn tampering_wrong_index_and_wrong_key_fail() {
        let k = FileKey::generate().unwrap();
        let mut ct = k.encrypt_chunk(3, b"some chunk of a file").unwrap();
        assert_eq!(k.decrypt_chunk(3, &ct).unwrap(), b"some chunk of a file");
        assert!(k.decrypt_chunk(4, &ct).is_err(), "chunks cannot be reordered");
        assert!(FileKey::generate().unwrap().decrypt_chunk(3, &ct).is_err());
        ct[0] ^= 1;
        assert!(k.decrypt_chunk(3, &ct).is_err());
        assert!(k.decrypt_chunk(3, &ct[..4]).is_err());
        assert_ne!(FileKey::generate().unwrap(), FileKey::generate().unwrap());
        assert!(FileKey::from_parts(&[0; 31], &[0; 12]).is_err());
    }

    #[test]
    fn sha256_vector() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
