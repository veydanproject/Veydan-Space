// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! NIP-44 v2, the half of it that opens one payload with the keys of that
//! payload alone (<https://github.com/nostr-protocol/nips/blob/master/44.md>).
//!
//! nostr keeps the message keys to itself: its `decrypt` takes the
//! conversation key and derives them inside. A proof hands over the message
//! keys and never the conversation key, so the server needs the steps
//! after the derivation on their own, and the client needs the derivation
//! on its own. Both are here, written after the NIP and checked against its
//! official vectors (`tests/nip44.vectors.json`).
//!
//! The length prefix is read as nostr 0.45 writes it: two bytes, or for a
//! plaintext of 64 KiB and more two zero bytes and four of length. The NIP
//! itself stops at 65 535 bytes; a seal our own clients made of a longer
//! rumor must verify all the same.

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chacha20::cipher::{KeyIvInit, StreamCipher};
use chacha20::ChaCha20;
use hkdf::Hkdf;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::wire::MessageKeys;

/// The version byte of a v2 payload.
pub const VERSION: u8 = 2;
const NONCE_LEN: usize = 32;
const MAC_LEN: usize = 32;
const LEGACY_PREFIX: usize = 2;
const EXTENDED_PREFIX: usize = 6;
const EXTENDED_THRESHOLD: usize = 65_536;
/// The smallest padded plaintext, and so the smallest ciphertext is this
/// plus a prefix.
const MIN_PADDED: usize = 32;
/// Version, nonce, the shortest ciphertext, the MAC.
const MIN_PAYLOAD: usize = 1 + NONCE_LEN + LEGACY_PREFIX + MIN_PADDED + MAC_LEN;

/// Why a payload does not open. Every variant is a short stable word, as
/// the outcome of a proof names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{}", self.code())]
pub enum PayloadError {
    /// Not base64, or starts with `#` (a version the NIP reserves).
    Base64,
    /// A version byte other than 2.
    Version,
    /// Too short to hold a nonce, the shortest ciphertext and a MAC.
    Length,
    /// The MAC is not the one of the nonce and the ciphertext under this
    /// HMAC key.
    Mac,
    /// The plaintext does not carry a length and the padding the NIP
    /// prescribes for it.
    Padding,
}

impl PayloadError {
    /// The word of the outcome.
    pub fn code(self) -> &'static str {
        match self {
            PayloadError::Base64 => "payload_base64",
            PayloadError::Version => "payload_version",
            PayloadError::Length => "payload_length",
            PayloadError::Mac => "payload_mac",
            PayloadError::Padding => "payload_padding",
        }
    }
}

/// A v2 payload taken apart: what the MAC covers and the MAC.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Payload {
    pub nonce: [u8; NONCE_LEN],
    pub ciphertext: Vec<u8>,
    pub mac: [u8; MAC_LEN],
}

impl Payload {
    /// Read the base64 form, as it stands in an event's `content`. Nothing
    /// is checked but the shape: the MAC is checked by [`Payload::open`].
    pub fn parse(encoded: &str) -> Result<Self, PayloadError> {
        if encoded.starts_with('#') {
            return Err(PayloadError::Base64);
        }
        let raw = B64.decode(encoded).map_err(|_| PayloadError::Base64)?;
        Self::from_bytes(&raw)
    }

    /// Read the decoded form.
    pub fn from_bytes(raw: &[u8]) -> Result<Self, PayloadError> {
        let (&version, rest) = raw.split_first().ok_or(PayloadError::Length)?;
        if version != VERSION {
            return Err(PayloadError::Version);
        }
        if raw.len() < MIN_PAYLOAD {
            return Err(PayloadError::Length);
        }
        let (nonce, rest) = rest.split_at(NONCE_LEN);
        let (ciphertext, mac) = rest.split_at(rest.len() - MAC_LEN);
        Ok(Payload {
            nonce: nonce.try_into().map_err(|_| PayloadError::Length)?,
            ciphertext: ciphertext.to_vec(),
            mac: mac.try_into().map_err(|_| PayloadError::Length)?,
        })
    }

    /// Check the MAC with `keys.hmac_key`, decrypt with the ChaCha20 key and
    /// nonce, and take the plaintext out of its padding. The MAC is checked
    /// first and in constant time, so that nothing is decrypted that its
    /// author did not encrypt.
    pub fn open(&self, keys: &MessageKeys) -> Result<Vec<u8>, PayloadError> {
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&keys.hmac_key)
            .map_err(|_| PayloadError::Mac)?;
        mac.update(&self.nonce);
        mac.update(&self.ciphertext);
        mac.verify_slice(&self.mac).map_err(|_| PayloadError::Mac)?;

        let mut buffer = self.ciphertext.clone();
        let mut cipher = ChaCha20::new((&keys.chacha_key).into(), (&keys.chacha_nonce).into());
        cipher.apply_keystream(&mut buffer);
        unpad(&buffer)
    }
}

/// The message keys of one payload: HKDF-expand with SHA-256 of the
/// conversation key, the payload's nonce as the info, 76 bytes cut into
/// the ChaCha20 key, the ChaCha20 nonce and the HMAC key (`get_message_keys`
/// of the NIP). The expansion is one-way: the keys of one payload tell
/// nothing of the conversation key nor of the keys of any other payload.
pub fn message_keys(conversation_key: &[u8; 32], nonce: &[u8; 32]) -> MessageKeys {
    let mut okm = [0u8; 76];
    // A pseudorandom key of 32 bytes is what HKDF-SHA256 takes, and 76
    // bytes are far below its limit of 255 blocks: neither call can fail.
    let hkdf =
        Hkdf::<Sha256>::from_prk(conversation_key).expect("a 32-byte PRK is valid for HKDF-SHA256");
    hkdf.expand(nonce, &mut okm)
        .expect("76 bytes are within the HKDF-SHA256 limit");
    let mut keys = MessageKeys {
        chacha_key: [0; 32],
        chacha_nonce: [0; 12],
        hmac_key: [0; 32],
    };
    keys.chacha_key.copy_from_slice(&okm[0..32]);
    keys.chacha_nonce.copy_from_slice(&okm[32..44]);
    keys.hmac_key.copy_from_slice(&okm[44..76]);
    keys
}

/// The padded length of a plaintext of `len` bytes (`calc_padded_len` of
/// the NIP): 32 at least, then steps of 32 up to 256 and of an eighth of
/// the next power of two above that.
///
/// Counted in 64 bits whatever the platform, so that a length a hostile
/// prefix claims cannot overflow; a length whose padding does not fit in
/// `usize` gets `usize::MAX`, which no buffer is long.
pub fn calc_padded_len(len: usize) -> usize {
    let len = len as u64;
    if len <= 32 {
        return 32;
    }
    let Some(next_power) = 1u64.checked_shl(u64::BITS - (len - 1).leading_zeros()) else {
        return usize::MAX;
    };
    let chunk = if next_power <= 256 {
        32
    } else {
        next_power / 8
    };
    ((len - 1) / chunk + 1)
        .checked_mul(chunk)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(usize::MAX)
}

/// Read the length prefix and check that exactly the prescribed padding
/// follows the plaintext.
fn unpad(padded: &[u8]) -> Result<Vec<u8>, PayloadError> {
    let first: [u8; 2] = padded
        .get(..LEGACY_PREFIX)
        .and_then(|b| b.try_into().ok())
        .ok_or(PayloadError::Padding)?;
    let (prefix, len) = match u16::from_be_bytes(first) {
        0 => {
            let wide: [u8; 4] = padded
                .get(LEGACY_PREFIX..EXTENDED_PREFIX)
                .and_then(|b| b.try_into().ok())
                .ok_or(PayloadError::Padding)?;
            let len =
                usize::try_from(u32::from_be_bytes(wide)).map_err(|_| PayloadError::Padding)?;
            // A shorter plaintext has a two-byte prefix: a zero one with a
            // small length behind it is no form a writer produces.
            if len < EXTENDED_THRESHOLD {
                return Err(PayloadError::Padding);
            }
            (EXTENDED_PREFIX, len)
        }
        n => (LEGACY_PREFIX, usize::from(n)),
    };
    let expected = prefix
        .checked_add(calc_padded_len(len))
        .ok_or(PayloadError::Padding)?;
    if padded.len() != expected {
        return Err(PayloadError::Padding);
    }
    Ok(padded[prefix..prefix + len].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padding_steps() {
        assert_eq!(calc_padded_len(1), 32);
        assert_eq!(calc_padded_len(33), 64);
        assert_eq!(calc_padded_len(257), 320);
        assert_eq!(calc_padded_len(65_536), 65_536);
        assert_eq!(calc_padded_len(65_537), 65_536 + 16_384);
    }

    #[test]
    fn short_and_strange_payloads_are_refused() {
        assert_eq!(Payload::from_bytes(&[]), Err(PayloadError::Length));
        assert_eq!(Payload::from_bytes(&[2; 98]), Err(PayloadError::Length));
        assert_eq!(Payload::from_bytes(&[1; 200]), Err(PayloadError::Version));
        assert_eq!(Payload::parse("#abc"), Err(PayloadError::Base64));
        assert_eq!(Payload::parse("not base64!"), Err(PayloadError::Base64));
        assert!(Payload::from_bytes(&[2; 99]).is_ok());
    }

    #[test]
    fn padding_is_checked_exactly() {
        assert_eq!(unpad(&[]), Err(PayloadError::Padding));
        assert_eq!(unpad(&[0, 0, 0]), Err(PayloadError::Padding));
        // A two-byte length of 1 with 32 bytes of padding is right, 31 or 33 is not.
        let mut good = vec![0, 1];
        good.extend([7u8; 32]);
        assert_eq!(unpad(&good).unwrap(), vec![7]);
        assert_eq!(unpad(&good[..33]), Err(PayloadError::Padding));
        good.push(0);
        assert_eq!(unpad(&good), Err(PayloadError::Padding));
        // A zero prefix with a small length behind it is refused.
        let mut wide = vec![0, 0, 0, 0, 0, 1];
        wide.extend([0u8; 32]);
        assert_eq!(unpad(&wide), Err(PayloadError::Padding));
        // The largest length a prefix may claim must not overflow.
        let mut huge = vec![0, 0, 0xff, 0xff, 0xff, 0xff];
        huge.extend([0u8; 32]);
        assert_eq!(unpad(&huge), Err(PayloadError::Padding));
    }
}
