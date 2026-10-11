// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The Recovery Key (`internal/cloud-recovery.md` §3): 16 random bytes shown
//! once as 12 English BIP-39 words, and the keys derived from them. Every
//! derivation is HKDF-SHA256 of the 16 bytes (never of the words) with the
//! salt `veydan-recovery` and one label per purpose, so that a leaked
//! derived key says nothing about the others.
//!
//! The passphrase of the `cloud` sync vault is derived here as well: from
//! the nsec, not from the key, as `internal/vcloud-plan.md` 4.8 defines.

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use bip39::{Language, Mnemonic};
use hkdf::Hkdf;
use secp256k1::{Secp256k1, SecretKey};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::kdf;

pub const RK_LEN: usize = 16;
pub const RK_WORDS: usize = 12;
/// The salt of every label derived from the key.
pub const HKDF_SALT: &[u8] = b"veydan-recovery";
/// The salt and the info of the sync-vault passphrase, derived from the nsec.
pub const CLOUD_SALT: &[u8] = b"veydan-cloud";
pub const SYNC_VAULT_PASSPHRASE_INFO: &[u8] = b"sync-vault-passphrase/1";
/// Bytes a derivation yields.
pub const DERIVED_LEN: usize = 32;
/// Draws of the random source before `generate` gives up: a draw fails
/// only when it gives no secp256k1 scalar, about once in 2^128.
const GENERATE_ATTEMPTS: usize = 8;

/// The labels of §3, without the common prefix in their names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Label {
    /// A secp256k1 scalar: the key `auth` that reads the escrow.
    EscrowAuth,
    /// The password of the Argon2id that keys the escrow.
    EscrowEnc,
    /// The first 16 bytes are the `commit` of every share of a generation.
    ShareCommit,
    /// The HMAC key of the `mac` of a share.
    ShareMac,
    /// Its hex is the input of the lock's `derive_kek` for the wrap v2.
    LockWrap,
}

impl Label {
    pub const ALL: [Label; 5] = [
        Label::EscrowAuth,
        Label::EscrowEnc,
        Label::ShareCommit,
        Label::ShareMac,
        Label::LockWrap,
    ];

    /// The `info` of HKDF.
    pub const fn info(self) -> &'static [u8] {
        match self {
            Label::EscrowAuth => b"veydan-recovery/escrow-auth/1",
            Label::EscrowEnc => b"veydan-recovery/escrow-enc/1",
            Label::ShareCommit => b"veydan-recovery/share-commit/1",
            Label::ShareMac => b"veydan-recovery/share-mac/1",
            Label::LockWrap => b"veydan-recovery/lock-wrap/1",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RkError {
    #[error("a recovery key is 12 words, not {0}")]
    WordCount(usize),
    #[error("word {0} is not in the list")]
    UnknownWord(usize),
    #[error("the words do not pass the checksum")]
    Checksum,
    #[error("the entropy gives no escrow key")]
    InvalidScalar,
    #[error("the system random source failed")]
    Random,
}

/// The 16 bytes of entropy. Debug names the type and nothing else.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct RecoveryKey([u8; RK_LEN]);

impl std::fmt::Debug for RecoveryKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RecoveryKey")
    }
}

impl RecoveryKey {
    /// A fresh key from the system random source. A draw whose
    /// `escrow-auth` scalar is not a valid secp256k1 key is thrown away.
    pub fn generate() -> Result<Self, RkError> {
        for _ in 0..GENERATE_ATTEMPTS {
            let bytes: [u8; RK_LEN] = kdf::random_array().map_err(|_| RkError::Random)?;
            if let Ok(rk) = Self::from_bytes(bytes) {
                return Ok(rk);
            }
        }
        Err(RkError::Random)
    }

    /// From the entropy, checked for a valid `escrow-auth` scalar. The
    /// array is moved in and erased here; a copy the caller kept (a `Copy`
    /// array passed from a variable) is the caller's to erase.
    pub fn from_bytes(mut bytes: [u8; RK_LEN]) -> Result<Self, RkError> {
        let rk = Self(bytes);
        bytes.zeroize();
        let check = rk.escrow_auth()?;
        drop(check);
        Ok(rk)
    }

    /// From the 12 words as a person typed them: any case, any whitespace.
    pub fn from_words(text: &str) -> Result<Self, RkError> {
        let normalized = normalize_words(text);
        let count = normalized.split(' ').filter(|w| !w.is_empty()).count();
        if count != RK_WORDS {
            return Err(RkError::WordCount(count));
        }
        let mnemonic =
            Mnemonic::parse_in_normalized(Language::English, &normalized).map_err(|e| match e {
                bip39::Error::UnknownWord(i) => RkError::UnknownWord(i),
                bip39::Error::BadWordCount(n) => RkError::WordCount(n),
                _ => RkError::Checksum,
            })?;
        let (mut entropy, len) = mnemonic.to_entropy_array();
        if len != RK_LEN {
            entropy.zeroize();
            return Err(RkError::WordCount(count));
        }
        let mut bytes = Zeroizing::new([0u8; RK_LEN]);
        bytes.copy_from_slice(&entropy[..RK_LEN]);
        entropy.zeroize();
        Self::from_bytes(*bytes)
    }

    pub fn as_bytes(&self) -> &[u8; RK_LEN] {
        &self.0
    }

    /// The 12 words, lower case, one space between them.
    pub fn words(&self) -> Zeroizing<String> {
        let list = Language::English.word_list();
        let indices = word_indices(&self.0);
        let mut out = String::with_capacity(RK_WORDS * 9);
        for (i, idx) in indices.iter().enumerate() {
            if i > 0 {
                out.push(' ');
            }
            out.push_str(list[*idx as usize]);
        }
        Zeroizing::new(out)
    }

    /// The 12 words one by one, for a screen that numbers them. The list
    /// holds pointers into the static word list; the caller drops it when
    /// the screen goes.
    pub fn word_list(&self) -> Vec<&'static str> {
        let list = Language::English.word_list();
        word_indices(&self.0)
            .iter()
            .map(|i| list[*i as usize])
            .collect()
    }

    /// HKDF-SHA256(key, salt `veydan-recovery`, info of the label), 32 bytes.
    pub fn derive(&self, label: Label) -> Zeroizing<[u8; DERIVED_LEN]> {
        hkdf(&self.0, HKDF_SALT, label.info())
    }

    /// The key `auth` of the escrow.
    pub fn escrow_auth(&self) -> Result<AuthKey, RkError> {
        AuthKey::from_scalar(self.derive(Label::EscrowAuth))
    }

    /// The password of the Argon2id that keys the escrow.
    pub fn escrow_enc(&self) -> Zeroizing<[u8; DERIVED_LEN]> {
        self.derive(Label::EscrowEnc)
    }

    /// The `commit` field of every share of a generation.
    pub fn share_commit(&self) -> [u8; 16] {
        let full = self.derive(Label::ShareCommit);
        let mut out = [0u8; 16];
        out.copy_from_slice(&full[..16]);
        out
    }

    /// The HMAC-SHA256 key of the `mac` of a share.
    pub fn share_mac_key(&self) -> Zeroizing<[u8; DERIVED_LEN]> {
        self.derive(Label::ShareMac)
    }

    /// The input of the lock's `derive_kek` for the recovery wrap v2: the
    /// hex of the derivation, 64 lower-case characters.
    pub fn lock_wrap_input(&self) -> Zeroizing<String> {
        Zeroizing::new(hex::encode(self.derive(Label::LockWrap).as_slice()))
    }
}

/// Lower case, trimmed, one space between the words.
pub fn normalize_words(text: &str) -> Zeroizing<String> {
    let mut out = String::with_capacity(text.len());
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        for ch in word.chars() {
            out.extend(ch.to_lowercase());
        }
    }
    Zeroizing::new(out)
}

/// 16 bytes of entropy and the 4-bit BIP-39 checksum as 12 indices of 11
/// bits.
fn word_indices(entropy: &[u8; RK_LEN]) -> Zeroizing<[u16; RK_WORDS]> {
    let checksum = Sha256::digest(entropy)[0] >> 4;
    // 132 bits in a 17-byte buffer: the 16 bytes and the checksum nibble.
    let mut bits = Zeroizing::new([0u8; 17]);
    bits[..RK_LEN].copy_from_slice(entropy);
    bits[RK_LEN] = checksum << 4;
    let mut out = Zeroizing::new([0u16; RK_WORDS]);
    for (w, slot) in out.iter_mut().enumerate() {
        let mut v = 0u16;
        for b in 0..11 {
            let bit = w * 11 + b;
            v = (v << 1) | ((bits[bit / 8] >> (7 - bit % 8)) & 1) as u16;
        }
        *slot = v;
    }
    out
}

pub(crate) fn hkdf(ikm: &[u8], salt: &[u8], info: &[u8]) -> Zeroizing<[u8; DERIVED_LEN]> {
    let mut out = Zeroizing::new([0u8; DERIVED_LEN]);
    // 32 bytes is far below the 255 * 32 HKDF-SHA256 allows: expand cannot fail.
    if Hkdf::<Sha256>::new(Some(salt), ikm)
        .expand(info, out.as_mut_slice())
        .is_err()
    {
        out.zeroize();
    }
    out
}

/// The secp256k1 key `auth` derived from the Recovery Key: its x-only
/// public key is the address of the escrow and signs the request that
/// reads it.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct AuthKey {
    secret: [u8; 32],
    #[zeroize(skip)]
    public: [u8; 32],
}

impl std::fmt::Debug for AuthKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The public key gives the escrow address, which no log may hold.
        f.write_str("AuthKey")
    }
}

impl AuthKey {
    fn from_scalar(scalar: Zeroizing<[u8; 32]>) -> Result<Self, RkError> {
        let sk = SecretKey::from_byte_array(&scalar).map_err(|_| RkError::InvalidScalar)?;
        let secp = Secp256k1::new();
        let (xonly, _parity) = sk.x_only_public_key(&secp);
        Ok(Self {
            secret: *scalar,
            public: xonly.serialize(),
        })
    }

    /// The x-only public key, 32 bytes, as the pubkey of a nostr event.
    pub fn public_x_only(&self) -> [u8; 32] {
        self.public
    }

    pub fn secret_bytes(&self) -> &[u8; 32] {
        &self.secret
    }

    /// The secp256k1 secret key, to sign the NIP-98 request that reads the
    /// escrow. The caller erases it after use.
    pub fn secret_key(&self) -> Result<SecretKey, RkError> {
        SecretKey::from_byte_array(&self.secret).map_err(|_| RkError::InvalidScalar)
    }

    /// `hex(sha256(auth_pub))`: the `<id>` of `escrow/<id>`.
    pub fn escrow_address(&self) -> String {
        escrow_address(&self.public)
    }
}

/// The address of the escrow of an x-only public key: what vstore derives
/// from the signer of the request as well.
pub fn escrow_address(auth_pub: &[u8; 32]) -> String {
    hex::encode(Sha256::digest(auth_pub))
}

/// The x-only public key of an nsec: the npub as 32 bytes.
pub fn npub_of(nsec: &[u8; 32]) -> Option<[u8; 32]> {
    let sk = SecretKey::from_byte_array(nsec).ok()?;
    let secp = Secp256k1::new();
    Some(sk.x_only_public_key(&secp).0.serialize())
}

/// The passphrase of the `cloud` sync vault (`internal/vcloud-plan.md` 4.8):
/// base64 of HKDF-SHA256(nsec, salt `veydan-cloud`, info
/// `sync-vault-passphrase/1`). The manifest of the vault does not change.
pub fn sync_vault_passphrase(nsec: &[u8; 32]) -> Zeroizing<String> {
    let okm = hkdf(nsec, CLOUD_SALT, SYNC_VAULT_PASSPHRASE_INFO);
    Zeroizing::new(B64.encode(okm.as_slice()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RK: [u8; 16] = [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
        0xff,
    ];

    #[test]
    fn words_roundtrip_and_match_bip39() {
        let rk = RecoveryKey::from_bytes(RK).unwrap();
        let words = rk.words();
        assert_eq!(words.split(' ').count(), 12);
        let reference = Mnemonic::from_entropy(&RK).unwrap().to_string();
        assert_eq!(words.as_str(), reference);
        assert_eq!(RecoveryKey::from_words(&words).unwrap(), rk);
        assert_eq!(rk.word_list().len(), 12);
        assert_eq!(rk.word_list().join(" "), words.as_str());
    }

    #[test]
    fn bip39_test_vector() {
        // Trezor's BIP-39 vector for 16 zero bytes.
        let rk = RecoveryKey::from_bytes([0u8; 16]).unwrap();
        assert_eq!(
            rk.words().as_str(),
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
        );
        let rk = RecoveryKey::from_bytes([0x7f; 16]).unwrap();
        assert_eq!(
            rk.words().as_str(),
            "legal winner thank year wave sausage worth useful legal winner thank yellow"
        );
    }

    #[test]
    fn parse_normalizes_case_and_whitespace() {
        let rk = RecoveryKey::from_bytes(RK).unwrap();
        let words = rk.words();
        let messy = format!("  {}\n", words.replace(' ', "   ").to_uppercase());
        assert_eq!(RecoveryKey::from_words(&messy).unwrap(), rk);
        assert_eq!(
            normalize_words(" Abandon\tABOUT  ").as_str(),
            "abandon about"
        );
    }

    #[test]
    fn parse_errors() {
        assert_eq!(
            RecoveryKey::from_words("abandon abandon"),
            Err(RkError::WordCount(2))
        );
        let fifteen = ["abandon"; 15].join(" ");
        assert_eq!(
            RecoveryKey::from_words(&fifteen),
            Err(RkError::WordCount(15))
        );
        let bad_word = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon veydan";
        assert_eq!(
            RecoveryKey::from_words(bad_word),
            Err(RkError::UnknownWord(11))
        );
        let bad_checksum = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon";
        assert_eq!(
            RecoveryKey::from_words(bad_checksum),
            Err(RkError::Checksum)
        );
    }

    #[test]
    fn generate_gives_distinct_valid_keys() {
        let a = RecoveryKey::generate().unwrap();
        let b = RecoveryKey::generate().unwrap();
        assert_ne!(a, b);
        assert!(a.escrow_auth().is_ok());
        assert_eq!(format!("{a:?}"), "RecoveryKey");
    }

    #[test]
    fn labels_are_distinct_and_deterministic() {
        let rk = RecoveryKey::from_bytes(RK).unwrap();
        let mut seen = Vec::new();
        for label in Label::ALL {
            let a = rk.derive(label);
            let b = rk.derive(label);
            assert_eq!(a, b);
            assert!(!seen.contains(&*a), "{label:?}");
            seen.push(*a);
        }
        assert!(Label::ALL
            .iter()
            .all(|l| l.info().starts_with(b"veydan-recovery/") && l.info().ends_with(b"/1")));
        // A different key gives different derivations.
        let other = RecoveryKey::from_bytes([1u8; 16]).unwrap();
        assert_ne!(other.derive(Label::EscrowEnc), rk.derive(Label::EscrowEnc));
    }

    #[test]
    fn derived_shapes() {
        let rk = RecoveryKey::from_bytes(RK).unwrap();
        assert_eq!(rk.share_commit(), rk.derive(Label::ShareCommit)[..16]);
        assert_eq!(*rk.share_mac_key(), *rk.derive(Label::ShareMac));
        assert_eq!(*rk.escrow_enc(), *rk.derive(Label::EscrowEnc));
        let wrap = rk.lock_wrap_input();
        assert_eq!(wrap.len(), 64);
        assert_eq!(
            hex::decode(wrap.as_str()).unwrap(),
            rk.derive(Label::LockWrap).to_vec()
        );
    }

    #[test]
    fn auth_key_and_escrow_address() {
        let rk = RecoveryKey::from_bytes(RK).unwrap();
        let auth = rk.escrow_auth().unwrap();
        assert_eq!(auth.secret_bytes(), &*rk.derive(Label::EscrowAuth));
        let pub_from_sk = {
            let secp = Secp256k1::new();
            auth.secret_key()
                .unwrap()
                .x_only_public_key(&secp)
                .0
                .serialize()
        };
        assert_eq!(auth.public_x_only(), pub_from_sk);
        let address = auth.escrow_address();
        assert_eq!(address.len(), 64);
        assert_eq!(address, escrow_address(&auth.public_x_only()));
        assert_eq!(address, hex::encode(Sha256::digest(auth.public_x_only())));
        assert_eq!(format!("{auth:?}"), "AuthKey");
    }

    #[test]
    fn npub_of_nsec() {
        let nsec = [0x42u8; 32];
        let npub = npub_of(&nsec).unwrap();
        let secp = Secp256k1::new();
        let expected = SecretKey::from_byte_array(&nsec)
            .unwrap()
            .x_only_public_key(&secp)
            .0
            .serialize();
        assert_eq!(npub, expected);
        assert!(npub_of(&[0u8; 32]).is_none());
    }

    #[test]
    fn sync_vault_passphrase_vector() {
        let nsec = [0x42u8; 32];
        let a = sync_vault_passphrase(&nsec);
        let b = sync_vault_passphrase(&nsec);
        assert_eq!(a, b);
        // 32 bytes in standard base64 with padding.
        assert_eq!(a.len(), 44);
        assert!(a.ends_with('='));
        let raw = B64.decode(a.as_str()).unwrap();
        assert_eq!(raw.len(), 32);
        assert_eq!(
            raw,
            hkdf(&nsec, b"veydan-cloud", b"sync-vault-passphrase/1").to_vec()
        );
        assert_ne!(sync_vault_passphrase(&[0x43u8; 32]), a);
    }
}
