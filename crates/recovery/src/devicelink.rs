// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Linking a new device (`internal/cloud-recovery.md` §6): the inner wrap of
//! the nsec that the old device sends to the temporary key T of the new
//! one. The outer wrap (NIP-44 to T in a gift wrap) belongs to the
//! messenger; this crate makes and opens the inner one.
//!
//! ```text
//! code    = 8 words of the English BIP-39 list, 88 bits, no checksum
//! salt    = sha256(T.pub)[..16]
//! key     = Argon2id(password = canonical code, salt, production params)
//! aad     = "veydan-device-link/1" ‖ T.pub
//! blob    = v(1) ‖ nonce(24) ‖ XChaCha20-Poly1305(key, nonce, aad, cbor(payload))
//! payload = { v: 1, npub, nsec, created_at, sync: null | { vault_id, vmk, storage[, access] } }
//! ```
//!
//! The code is the key as well as the rendezvous: a T published under a
//! guessed `d` tag receives a blob it cannot open. The 4-word fingerprint
//! of T, the first 44 bits of sha256(T.pub), is what both screens show
//! for the person to compare.

use bip39::Language;
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::cbor::{Decoder, Encoder, Malformed, MapReader};
use crate::escrow::{decode_sync, encode_sync, SyncKeys};
use crate::kdf::{self, KdfError, KdfParams, KEY_LEN, NONCE_LEN, SALT_LEN, TAG_LEN};
use crate::rk;

pub const CODE_WORDS: usize = 8;
pub const FINGERPRINT_WORDS: usize = 4;
pub const LINK_VERSION: u8 = 1;
pub const LINK_AAD: &[u8] = b"veydan-device-link/1";
/// `veydan://link/<hex T.pub>/<words joined by ->`.
pub const LINK_URI_PREFIX: &str = "veydan://link/";
const BLOB_MIN_LEN: usize = 1 + NONCE_LEN + TAG_LEN;
const BITS_PER_WORD: usize = 11;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DeviceLinkError {
    #[error("a link code is 8 words, not {0}")]
    WordCount(usize),
    #[error("word {0} is not in the list")]
    UnknownWord(usize),
    #[error("the link uri is malformed")]
    Uri,
    #[error("the link blob is malformed")]
    Format,
    #[error("link blob version {0} is not 1")]
    Version(u8),
    #[error("the blob does not open with this code and key")]
    Decrypt,
    #[error("the nsec of the blob does not give its npub")]
    Identity,
    #[error("kdf failed")]
    Kdf,
    #[error("the system random source failed")]
    Random,
}

impl From<Malformed> for DeviceLinkError {
    fn from(_: Malformed) -> Self {
        DeviceLinkError::Format
    }
}

impl From<KdfError> for DeviceLinkError {
    fn from(e: KdfError) -> Self {
        match e {
            KdfError::Decrypt => DeviceLinkError::Decrypt,
            KdfError::Random => DeviceLinkError::Random,
            KdfError::Params | KdfError::Kdf | KdfError::Encrypt => DeviceLinkError::Kdf,
        }
    }
}

/// The code of 8 words in its canonical form: lower case, one space
/// between the words.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct LinkCode(String);

impl std::fmt::Debug for LinkCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LinkCode")
    }
}

impl LinkCode {
    /// 8 words from the system random source: 88 bits.
    pub fn generate() -> Result<Self, DeviceLinkError> {
        let mut raw = Zeroizing::new([0u8; 11]);
        kdf::random(raw.as_mut_slice()).map_err(|_| DeviceLinkError::Random)?;
        Ok(Self(words_of_bits(&raw[..], CODE_WORDS).join(" ")))
    }

    /// As a person typed or scanned it: any case, any whitespace.
    pub fn parse(text: &str) -> Result<Self, DeviceLinkError> {
        let normalized = rk::normalize_words(text);
        let words: Vec<&str> = normalized.split(' ').filter(|w| !w.is_empty()).collect();
        if words.len() != CODE_WORDS {
            return Err(DeviceLinkError::WordCount(words.len()));
        }
        for (i, w) in words.iter().enumerate() {
            if Language::English.find_word(w).is_none() {
                return Err(DeviceLinkError::UnknownWord(i));
            }
        }
        Ok(Self(words.join(" ")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn words(&self) -> Vec<&str> {
        self.0.split(' ').collect()
    }

    /// `sha256(canonical code)`: the `d` tag of the kind 30078 event the
    /// new device publishes when there is no camera.
    pub fn digest(&self) -> [u8; 32] {
        Sha256::digest(self.0.as_bytes()).into()
    }

    /// The `d` tag as hex.
    pub fn d_tag(&self) -> String {
        hex::encode(self.digest())
    }
}

/// `n` words from the first `11 * n` bits of `bits`, most significant
/// first.
fn words_of_bits(bits: &[u8], n: usize) -> Vec<&'static str> {
    let list = Language::English.word_list();
    (0..n)
        .map(|w| {
            let mut v = 0usize;
            for b in 0..BITS_PER_WORD {
                let bit = w * BITS_PER_WORD + b;
                v = (v << 1) | ((bits[bit / 8] >> (7 - bit % 8)) & 1) as usize;
            }
            list[v]
        })
        .collect()
}

/// The 4 words both screens show: the first 44 bits of `sha256(T.pub)`.
pub fn fingerprint(t_pub: &[u8; 32]) -> [&'static str; 4] {
    let digest = Sha256::digest(t_pub);
    let words = words_of_bits(&digest, FINGERPRINT_WORDS);
    [words[0], words[1], words[2], words[3]]
}

/// The Argon2id salt of the wrap to T: `sha256(T.pub)[..16]`.
pub fn link_salt(t_pub: &[u8; 32]) -> [u8; SALT_LEN] {
    let digest = Sha256::digest(t_pub);
    let mut out = [0u8; SALT_LEN];
    out.copy_from_slice(&digest[..SALT_LEN]);
    out
}

/// The content of the QR code.
pub fn link_uri(t_pub: &[u8; 32], code: &LinkCode) -> String {
    format!(
        "{LINK_URI_PREFIX}{}/{}",
        hex::encode(t_pub),
        code.as_str().replace(' ', "-")
    )
}

pub fn parse_link_uri(uri: &str) -> Result<([u8; 32], LinkCode), DeviceLinkError> {
    let rest = uri
        .strip_prefix(LINK_URI_PREFIX)
        .ok_or(DeviceLinkError::Uri)?;
    let (pub_hex, words) = rest.split_once('/').ok_or(DeviceLinkError::Uri)?;
    let t_pub: [u8; 32] = hex::decode(pub_hex)
        .map_err(|_| DeviceLinkError::Uri)?
        .try_into()
        .map_err(|_| DeviceLinkError::Uri)?;
    // The joined words are the password of the wrap: erased with the rest.
    let joined = Zeroizing::new(words.replace('-', " "));
    let code = LinkCode::parse(&joined)?;
    Ok((t_pub, code))
}

/// What the old device hands over.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct LinkPayload {
    pub npub: [u8; 32],
    pub nsec: [u8; 32],
    /// Unix seconds.
    pub created_at: u64,
    /// The sync vault of the old device and where it lives, so that the
    /// new one sets up the same storage (`internal/messenger-wire.md` §3);
    /// `None` when the old device is bound to no vault.
    pub sync: Option<SyncKeys>,
}

impl std::fmt::Debug for LinkPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkPayload")
            .field("npub", &hex::encode(self.npub))
            .field("created_at", &self.created_at)
            .field("sync", &self.sync)
            .finish_non_exhaustive()
    }
}

impl LinkPayload {
    pub fn to_cbor(&self) -> Zeroizing<Vec<u8>> {
        let mut e = Encoder::new();
        e.map(5)
            .text("v")
            .uint(LINK_VERSION as u64)
            .text("npub")
            .bytes(&self.npub)
            .text("nsec")
            .bytes(&self.nsec)
            .text("created_at")
            .uint(self.created_at)
            .text("sync");
        match &self.sync {
            None => {
                e.null();
            }
            Some(sync) => encode_sync(&mut e, sync),
        }
        Zeroizing::new(e.finish())
    }

    pub fn from_cbor(bytes: &[u8]) -> Result<Self, DeviceLinkError> {
        let mut dec = Decoder::new(bytes);
        let mut m = MapReader::open(&mut dec, 5)?;
        let v = m.key("v")?.u8()?;
        if v != LINK_VERSION {
            return Err(DeviceLinkError::Version(v));
        }
        let npub = m.key("npub")?.array::<32>()?;
        let nsec = m.key("nsec")?.array::<32>()?;
        let created_at = m.key("created_at")?.uint()?;
        let sync_dec = m.key("sync")?;
        let sync = if sync_dec.null_or_map()? {
            None
        } else {
            Some(decode_sync(sync_dec)?)
        };
        m.close()?;
        dec.end()?;
        Ok(LinkPayload {
            npub,
            nsec,
            created_at,
            sync,
        })
    }
}

fn aad(t_pub: &[u8; 32]) -> [u8; 52] {
    let mut out = [0u8; 52];
    out[..20].copy_from_slice(LINK_AAD);
    out[20..].copy_from_slice(t_pub);
    out
}

fn key_for(
    code: &LinkCode,
    t_pub: &[u8; 32],
    params: &KdfParams,
) -> Result<Zeroizing<[u8; KEY_LEN]>, DeviceLinkError> {
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    kdf::argon2id(
        code.as_str().as_bytes(),
        &link_salt(t_pub),
        params,
        key.as_mut_slice(),
    )?;
    Ok(key)
}

/// The inner wrap of the payload for T, with the production parameters and
/// a fresh nonce.
pub fn seal(
    code: &LinkCode,
    t_pub: &[u8; 32],
    payload: &LinkPayload,
) -> Result<Vec<u8>, DeviceLinkError> {
    let nonce = kdf::random_array()?;
    seal_with(code, t_pub, payload, &KdfParams::production(), nonce)
}

/// `seal` with the parameters and the nonce given: for test vectors. The
/// parameters are not written into the blob; both sides use the
/// production ones.
pub fn seal_with(
    code: &LinkCode,
    t_pub: &[u8; 32],
    payload: &LinkPayload,
    params: &KdfParams,
    nonce: [u8; NONCE_LEN],
) -> Result<Vec<u8>, DeviceLinkError> {
    let key = key_for(code, t_pub, params)?;
    let plaintext = payload.to_cbor();
    let body = kdf::seal(&key, &nonce, &aad(t_pub), &plaintext)?;
    let mut out = Vec::with_capacity(1 + NONCE_LEN + body.len());
    out.push(LINK_VERSION);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Opens the inner wrap with the code and T, production parameters.
pub fn open(
    code: &LinkCode,
    t_pub: &[u8; 32],
    blob: &[u8],
) -> Result<LinkPayload, DeviceLinkError> {
    open_with(code, t_pub, blob, &KdfParams::production())
}

pub fn open_with(
    code: &LinkCode,
    t_pub: &[u8; 32],
    blob: &[u8],
    params: &KdfParams,
) -> Result<LinkPayload, DeviceLinkError> {
    if blob.len() < BLOB_MIN_LEN {
        return Err(DeviceLinkError::Format);
    }
    if blob[0] != LINK_VERSION {
        return Err(DeviceLinkError::Version(blob[0]));
    }
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&blob[1..1 + NONCE_LEN]);
    let key = key_for(code, t_pub, params)?;
    let plaintext = kdf::open(&key, &nonce, &aad(t_pub), &blob[1 + NONCE_LEN..])?;
    let payload = LinkPayload::from_cbor(&plaintext)?;
    match rk::npub_of(&payload.nsec) {
        Some(npub) if npub == payload.npub => Ok(payload),
        _ => Err(DeviceLinkError::Identity),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NSEC: [u8; 32] = [0x42; 32];
    const T_PUB: [u8; 32] = [0xab; 32];
    const FAST: KdfParams = KdfParams {
        memory_kib: 64,
        iterations: 1,
        parallelism: 1,
    };

    fn payload(sync: bool) -> LinkPayload {
        LinkPayload {
            npub: rk::npub_of(&NSEC).unwrap(),
            nsec: NSEC,
            created_at: 1_791_000_000,
            sync: sync.then(|| SyncKeys::cloud("3f2a9c1e-5b7d-4e8a-9c0b-1d2e3f4a5b6c", [0x77; 32])),
        }
    }

    #[test]
    fn code_generate_parse_roundtrip() {
        let code = LinkCode::generate().unwrap();
        assert_eq!(code.words().len(), 8);
        assert!(code
            .words()
            .iter()
            .all(|w| Language::English.find_word(w).is_some()));
        let typed = format!(" {} ", code.as_str().to_uppercase().replace(' ', "  "));
        assert_eq!(LinkCode::parse(&typed).unwrap(), code);
        assert_ne!(LinkCode::generate().unwrap(), code);
        assert_eq!(format!("{code:?}"), "LinkCode");
        assert_eq!(
            code.d_tag(),
            hex::encode(Sha256::digest(code.as_str().as_bytes()))
        );
    }

    #[test]
    fn code_parse_errors() {
        assert_eq!(
            LinkCode::parse("abandon about"),
            Err(DeviceLinkError::WordCount(2))
        );
        assert_eq!(
            LinkCode::parse("abandon about zoo zone zero yellow year veydan"),
            Err(DeviceLinkError::UnknownWord(7))
        );
        // No checksum: any 8 words of the list are a code.
        assert!(
            LinkCode::parse("abandon abandon abandon abandon abandon abandon abandon abandon")
                .is_ok()
        );
    }

    #[test]
    fn words_of_bits_vector() {
        // 0x00.. -> abandon; the top 11 bits 0x001 -> ability (index 1).
        let bits = [
            0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let words = words_of_bits(&bits, 2);
        assert_eq!(words, ["ability", "abandon"]);
        let all_ones = [0xffu8; 11];
        assert_eq!(words_of_bits(&all_ones, 8), ["zoo"; 8]);
    }

    #[test]
    fn fingerprint_is_the_first_44_bits() {
        let fp = fingerprint(&T_PUB);
        let digest = Sha256::digest(T_PUB);
        // sha256(32 x 0xab) = 9a2db2e2 3f15...: 10011010 00101101 10110010 11100010 00111111 0001
        assert_eq!(&digest[..4], &[0x9a, 0x2d, 0xb2, 0xe2]);
        let list = Language::English.word_list();
        // 10011010001 = 1233, 01101101100 = 876, 10111000100 = 1476, 01111110001 = 1009
        assert_eq!(fp, [list[1233], list[876], list[1476], list[1009]]);
        assert_eq!(fingerprint(&T_PUB), fp);
        assert_ne!(fingerprint(&[0xac; 32]), fp);
        assert_eq!(link_salt(&T_PUB), digest[..16]);
    }

    #[test]
    fn uri_roundtrip() {
        let code =
            LinkCode::parse("abandon ability able about above absent absorb abstract").unwrap();
        let uri = link_uri(&T_PUB, &code);
        assert_eq!(
            uri,
            format!(
                "veydan://link/{}/abandon-ability-able-about-above-absent-absorb-abstract",
                "ab".repeat(32)
            )
        );
        let (t, c) = parse_link_uri(&uri).unwrap();
        assert_eq!(t, T_PUB);
        assert_eq!(c, code);
        assert_eq!(
            parse_link_uri("https://example.com/"),
            Err(DeviceLinkError::Uri)
        );
        assert_eq!(
            parse_link_uri("veydan://link/abcd/abandon"),
            Err(DeviceLinkError::Uri)
        );
        assert_eq!(
            parse_link_uri(&format!("veydan://link/{}/abandon", "ab".repeat(32))),
            Err(DeviceLinkError::WordCount(1))
        );
    }

    #[test]
    fn payload_cbor_roundtrip() {
        for sync in [true, false] {
            let p = payload(sync);
            let bytes = p.to_cbor();
            assert_eq!(bytes[0], 0xa5);
            assert_eq!(LinkPayload::from_cbor(&bytes).unwrap(), p);
        }
        assert_eq!(
            LinkPayload::from_cbor(&[0xa5, 0x61, 0x76, 0x02]),
            Err(DeviceLinkError::Version(2))
        );
        assert_eq!(LinkPayload::from_cbor(&[]), Err(DeviceLinkError::Format));
        assert!(!format!("{:?}", payload(true)).contains("4242"));
    }

    #[test]
    fn seal_open_roundtrip() {
        let code =
            LinkCode::parse("abandon ability able about above absent absorb abstract").unwrap();
        for sync in [true, false] {
            let p = payload(sync);
            let blob = seal_with(&code, &T_PUB, &p, &FAST, [3u8; 24]).unwrap();
            assert_eq!(blob[0], 1);
            assert_eq!(&blob[1..25], &[3u8; 24]);
            assert_eq!(blob.len(), 1 + 24 + p.to_cbor().len() + 16);
            assert_eq!(open_with(&code, &T_PUB, &blob, &FAST).unwrap(), p);
        }
    }

    #[test]
    fn seal_open_production() {
        let code = LinkCode::generate().unwrap();
        let p = payload(true);
        let blob = seal(&code, &T_PUB, &p).unwrap();
        assert_eq!(open(&code, &T_PUB, &blob).unwrap(), p);
        assert_ne!(seal(&code, &T_PUB, &p).unwrap(), blob);
    }

    #[test]
    fn wrong_code_key_or_byte_fails() {
        let code =
            LinkCode::parse("abandon ability able about above absent absorb abstract").unwrap();
        let p = payload(false);
        let blob = seal_with(&code, &T_PUB, &p, &FAST, [3u8; 24]).unwrap();
        let other = LinkCode::parse("zoo ability able about above absent absorb abstract").unwrap();
        assert_eq!(
            open_with(&other, &T_PUB, &blob, &FAST),
            Err(DeviceLinkError::Decrypt)
        );
        // Another T: salt and AAD differ.
        assert_eq!(
            open_with(&code, &[0xac; 32], &blob, &FAST),
            Err(DeviceLinkError::Decrypt)
        );
        for at in [1, 30, blob.len() - 1] {
            let mut t = blob.clone();
            t[at] ^= 1;
            assert_eq!(
                open_with(&code, &T_PUB, &t, &FAST),
                Err(DeviceLinkError::Decrypt),
                "{at}"
            );
        }
        let mut v = blob.clone();
        v[0] = 9;
        assert_eq!(
            open_with(&code, &T_PUB, &v, &FAST),
            Err(DeviceLinkError::Version(9))
        );
        assert_eq!(
            open_with(&code, &T_PUB, &blob[..30], &FAST),
            Err(DeviceLinkError::Format)
        );
    }

    #[test]
    fn identity_mismatch_refused() {
        let code =
            LinkCode::parse("abandon ability able about above absent absorb abstract").unwrap();
        let mut p = payload(false);
        p.npub = [7u8; 32];
        let blob = seal_with(&code, &T_PUB, &p, &FAST, [3u8; 24]).unwrap();
        assert_eq!(
            open_with(&code, &T_PUB, &blob, &FAST),
            Err(DeviceLinkError::Identity)
        );
    }
}
