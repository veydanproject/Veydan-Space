// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Pure key functions over `nostr::Keys`. No I/O, no storage.

use messenger_core::{MessengerError, Result};
use nostr::nips::nip06::FromMnemonic;
use nostr::nips::nip49::{EncryptedSecretKey, KeySecurity};
use nostr::key::{Keys, SecretKey};
use nostr::nips::nip19::{FromBech32, ToBech32};

pub fn generate() -> Keys {
    Keys::generate()
}

/// Accepts `nsec1…` or 64-char hex.
pub fn parse_secret(input: &str) -> Result<Keys> {
    let s = input.trim();
    if s.is_empty() {
        return Err(MessengerError::Invalid("secret key is empty".into()));
    }
    Keys::parse(s).map_err(|_| MessengerError::Invalid("not a valid nsec or hex secret key".into()))
}

pub fn from_secret_bytes(bytes: &[u8]) -> Result<Keys> {
    let sk = SecretKey::from_slice(bytes)
        .map_err(|_| MessengerError::Crypto("secret key must be 32 bytes".into()))?;
    Ok(Keys::new(sk))
}

/// NIP-06: BIP-39 mnemonic (+ optional passphrase) → key pair, account 0.
pub fn from_mnemonic(mnemonic: &str, passphrase: &str) -> Result<Keys> {
    let words = mnemonic.split_whitespace().collect::<Vec<_>>().join(" ");
    if words.is_empty() {
        return Err(MessengerError::Invalid("mnemonic is empty".into()));
    }
    let pass = if passphrase.is_empty() { None } else { Some(passphrase.to_string()) };
    Keys::from_mnemonic(words, pass)
        .map_err(|_| MessengerError::Invalid("not a valid BIP-39 mnemonic".into()))
}

pub fn npub(keys: &Keys) -> String {
    keys.public_key().to_bech32().expect("bech32 encoding of a public key cannot fail")
}

pub fn nsec(keys: &Keys) -> String {
    keys.secret_key().to_bech32().expect("bech32 encoding of a secret key cannot fail")
}

/// NIP-49 backup string. `log_n` is the scrypt cost exponent; 16 for real
/// backups, small values only in tests.
pub fn encrypt_ncryptsec(keys: &Keys, password: &str, log_n: u8) -> Result<String> {
    if password.is_empty() {
        return Err(MessengerError::Invalid("backup password is empty".into()));
    }
    let enc = EncryptedSecretKey::new(keys.secret_key(), password, log_n, KeySecurity::Unknown)
        .map_err(|e| MessengerError::Crypto(e.to_string()))?;
    enc.to_bech32().map_err(|e| MessengerError::Crypto(e.to_string()))
}

pub fn decrypt_ncryptsec(ncryptsec: &str, password: &str) -> Result<Keys> {
    let s = ncryptsec.trim();
    if !s.starts_with("ncryptsec1") {
        return Err(MessengerError::Invalid("backup must start with ncryptsec1".into()));
    }
    let enc = EncryptedSecretKey::from_bech32(s)
        .map_err(|_| MessengerError::Invalid("not a valid ncryptsec".into()))?;
    let sk = enc
        .decrypt(password)
        .map_err(|_| MessengerError::Crypto("wrong password or corrupted backup".into()))?;
    Ok(Keys::new(sk))
}

#[cfg(test)]
mod tests {
    use super::*;

    // NIP-06 test vector.
    const MNEMONIC: &str = "leader monkey parrot ring guide accident before fence cannon height naive bean";
    const MNEMONIC_HEX: &str = "7f7ff03d123792d6ac594bfa67bf6d0c0ab55b6b1fdb6249303fe861f1ccba9a";
    const MNEMONIC_NPUB: &str = "npub1zutzeysacnf9rru6zqwmxd54mud0k44tst6l70ja5mhv8jjumytsd2x7nu";

    #[test]
    fn generate_gives_distinct_keys() {
        let a = generate();
        let b = generate();
        assert_ne!(a.public_key(), b.public_key());
        assert!(npub(&a).starts_with("npub1"));
        assert!(nsec(&a).starts_with("nsec1"));
    }

    #[test]
    fn parse_secret_accepts_nsec_and_hex() {
        let k = generate();
        let from_nsec = parse_secret(&format!("  {}  ", nsec(&k))).unwrap();
        let from_hex = parse_secret(&k.secret_key().to_secret_hex()).unwrap();
        assert_eq!(from_nsec.public_key(), k.public_key());
        assert_eq!(from_hex.public_key(), k.public_key());
        assert!(parse_secret("").is_err());
        assert!(parse_secret("nsec1notakey").is_err());
        assert!(parse_secret(&npub(&k)).is_err());
    }

    #[test]
    fn secret_bytes_roundtrip() {
        let k = generate();
        let back = from_secret_bytes(&k.secret_key().to_secret_bytes()).unwrap();
        assert_eq!(back.public_key(), k.public_key());
        assert!(from_secret_bytes(&[1u8; 31]).is_err());
    }

    #[test]
    fn nip06_vector() {
        let k = from_mnemonic(MNEMONIC, "").unwrap();
        assert_eq!(k.secret_key().to_secret_hex(), MNEMONIC_HEX);
        assert_eq!(npub(&k), MNEMONIC_NPUB);
        assert!(from_mnemonic("", "").is_err());
        assert!(from_mnemonic("not a valid mnemonic at all", "").is_err());
    }

    #[test]
    fn ncryptsec_roundtrip_and_wrong_password() {
        let k = generate();
        let enc = encrypt_ncryptsec(&k, "correct horse", 8).unwrap();
        assert!(enc.starts_with("ncryptsec1"));
        let back = decrypt_ncryptsec(&enc, "correct horse").unwrap();
        assert_eq!(back.public_key(), k.public_key());
        assert!(decrypt_ncryptsec(&enc, "wrong").is_err());
        assert!(decrypt_ncryptsec("nsec1abc", "x").is_err());
        assert!(encrypt_ncryptsec(&k, "", 8).is_err());
    }

    // The backup password is normalized (NIP-49, NFKC) on both sides, so a
    // keyboard that types a letter composed or decomposed, or in fullwidth
    // form, still opens the backup. Pinned here in case the nostr crate is
    // bumped or replaced.
    #[test]
    fn ncryptsec_password_is_unicode_normalized() {
        let k = generate();
        let composed = "пароль-\u{0439}";
        let decomposed = "пароль-\u{0438}\u{0306}";
        assert_ne!(composed, decomposed);
        let enc = encrypt_ncryptsec(&k, composed, 8).unwrap();
        let back = decrypt_ncryptsec(&enc, decomposed).unwrap();
        assert_eq!(back.public_key(), k.public_key());

        let enc = encrypt_ncryptsec(&k, "\u{ff50}\u{ff41}\u{ff53}\u{ff53}word123", 8).unwrap();
        let back = decrypt_ncryptsec(&enc, "password123").unwrap();
        assert_eq!(back.public_key(), k.public_key());
        assert!(decrypt_ncryptsec(&enc, "Password123").is_err());
    }
}
