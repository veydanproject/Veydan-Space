// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The presence key: made from the user's secret and an epoch, the same on
//! every device of the user, and of no use to find the user's own key.

use hmac::{Hmac, KeyInit, Mac};
use nostr::key::{Keys, SecretKey};
use sha2::Sha256;

/// The key of the HMAC; changing it changes every presence key.
pub const CONTEXT: &[u8] = b"veydan-presence-v1";

type HmacSha256 = Hmac<Sha256>;

/// The presence key of `epoch`: HMAC-SHA256 with key [`CONTEXT`] of the
/// user's 32 secret bytes followed by the epoch as four bytes, little
/// endian. Should the 32 bytes not be a secret of secp256k1 (zero, or not
/// below the order of the curve: about one in 2^128), a counter byte 1, 2,
/// … is put after the epoch until they are.
pub fn derive(keys: &Keys, epoch: u32) -> Keys {
    let mut message = Vec::with_capacity(37);
    message.extend_from_slice(keys.secret_key().as_secret_bytes());
    message.extend_from_slice(&epoch.to_le_bytes());
    let mut counter: u8 = 0;
    loop {
        let mut mac = HmacSha256::new_from_slice(CONTEXT).expect("hmac takes a key of any length");
        mac.update(&message);
        let bytes = mac.finalize().into_bytes();
        if let Ok(secret) = SecretKey::from_slice(&bytes) {
            return Keys::new(secret);
        }
        // 255 misses in a row cannot happen; wrapping keeps the loop total.
        counter = counter.wrapping_add(1);
        message.truncate(36);
        message.push(counter);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_every_time_another_for_each_epoch() {
        let me = Keys::generate();
        let a = derive(&me, 0);
        assert_eq!(a.secret_key().to_secret_hex(), derive(&me, 0).secret_key().to_secret_hex());
        assert_eq!(a.public_key(), derive(&me, 0).public_key());
        let b = derive(&me, 1);
        assert_ne!(a.public_key(), b.public_key());
        assert_ne!(a.public_key(), derive(&me, u32::MAX).public_key());
        assert_ne!(a.public_key(), derive(&Keys::generate(), 0).public_key(), "another user, another key");
    }

    #[test]
    fn never_the_users_own_key() {
        let me = Keys::generate();
        for epoch in [0, 1, 2, 1000] {
            let p = derive(&me, epoch);
            assert_ne!(p.public_key(), me.public_key());
            assert_ne!(p.secret_key().to_secret_hex(), me.secret_key().to_secret_hex());
        }
    }

    /// Computed outside Rust, with OpenSSL 3.5.7:
    ///
    /// ```sh
    /// S=$(printf '\\x01%.0s' $(seq 32))
    /// printf "$S\x00\x00\x00\x00" | openssl dgst -sha256 -mac HMAC -macopt key:veydan-presence-v1
    /// printf "$S\x01\x00\x00\x00" | openssl dgst -sha256 -mac HMAC -macopt key:veydan-presence-v1
    /// ```
    ///
    /// that is, HMAC-SHA256 keyed by the ASCII of `veydan-presence-v1` over
    /// 32 bytes 0x01 and the epoch as four bytes little endian. Both are
    /// below the order of the curve, so no counter byte is added.
    #[test]
    fn the_way_the_key_is_made_is_fixed() {
        let me = Keys::parse(&"01".repeat(32)).unwrap();
        assert_eq!(
            derive(&me, 0).secret_key().to_secret_hex(),
            "cf749c034f1fd11e16624ddc6869fa92cb92cf299cd45753c555cc0b929d7fcf"
        );
        assert_eq!(
            derive(&me, 1).secret_key().to_secret_hex(),
            "e4f7065ebb4701417c67ef536d8cd7e67393080ea89c1a3373712d938748b7d9"
        );
    }
}
