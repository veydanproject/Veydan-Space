// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The proof of work of an activation and of a recovery request
//! (`internal/cloud-recovery.md` §9, `services/cloud/spec/cloud-api.md` §3):
//! the client solves, vcloud verifies, both with the same code.
//!
//! ```text
//! challenge = ts u64 BE ‖ d u8 ‖ purpose u8 ‖ r[8] ‖ HMAC-SHA256(server_key, ts ‖ d ‖ purpose ‖ r)[..16]
//! h         = Argon2id(password = nonce u64 BE, salt = challenge, m = 16 MiB, t = 1, p = 1, out = 32)
//! solved    = the first d bits of h are zero
//! ```
//!
//! Argon2id rather than SHA-256: with memory in the loop a GPU gains
//! little over a phone. The check code of a recovery request lives here
//! too (`services/cloud/spec/recovery.md` §3): both sides compute it from
//! `temp_pub`, the server never sees it.

use argon2::Block;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::kdf::{self, KdfParams};

pub const POW_MEMORY_KIB: u32 = 16 * 1024;
pub const POW_ITERATIONS: u32 = 1;
pub const POW_PARALLELISM: u32 = 1;
pub const POW_OUT_LEN: usize = 32;
/// `ts ‖ d ‖ purpose ‖ r ‖ mac`.
pub const CHALLENGE_LEN: usize = 8 + 1 + 1 + 8 + 16;
const SIGNED_LEN: usize = 18;
const MAC_LEN: usize = 16;
/// The base difficulty of each purpose (§9).
pub const ACTIVATE_DIFFICULTY: u8 = 7;
pub const RECOVERY_REQUEST_DIFFICULTY: u8 = 8;
/// Above it the server answers 429 instead of asking for more work.
pub const MAX_DIFFICULTY: u8 = 12;
/// A challenge is fresh when `ts` is not older than this (`spec/cloud-api.md`
/// §3.2 step 2: `pow_expired`) ...
pub const CHALLENGE_MAX_AGE_SECS: u64 = 120;
/// ... and not further than this in the future.
pub const CHALLENGE_MAX_FUTURE_SECS: u64 = 60;

const POW_PARAMS: KdfParams = KdfParams {
    memory_kib: POW_MEMORY_KIB,
    iterations: POW_ITERATIONS,
    parallelism: POW_PARALLELISM,
};

/// The words of the server (`spec/cloud-api.md` §12) are named at each
/// variant; `pow_expired` is `!Challenge::is_fresh`, `pow_replayed` a hit
/// of `replay_key` in the server's memory, `pow_insufficient` the server's
/// own comparison of `d` with what it requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PowError {
    /// The server asks for more than the ceiling: `429`, not a PoW word.
    #[error("difficulty {0} is out of range (0..=12)")]
    Difficulty(u8),
    /// Not 34 bytes, an unknown purpose, or an HMAC no server key made:
    /// `pow_invalid`.
    #[error("the challenge is malformed or not signed by this server")]
    Invalid,
    /// `pow_wrong`.
    #[error("the solution has {have} leading zero bits, {need} are needed")]
    Wrong { need: u8, have: u8 },
    /// Argon2id refused its parameters or its memory: an internal error.
    #[error("kdf failed")]
    Kdf,
}

/// What the work is for; the byte is in the challenge under the HMAC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Purpose {
    Activate = 1,
    RecoveryRequest = 2,
}

impl Purpose {
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            1 => Some(Purpose::Activate),
            2 => Some(Purpose::RecoveryRequest),
            _ => None,
        }
    }

    /// The value of the `purpose` query parameter.
    pub fn as_str(self) -> &'static str {
        match self {
            Purpose::Activate => "activate",
            Purpose::RecoveryRequest => "recovery.request",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "activate" => Some(Purpose::Activate),
            "recovery.request" => Some(Purpose::RecoveryRequest),
            _ => None,
        }
    }

    pub fn base_difficulty(self) -> u8 {
        match self {
            Purpose::Activate => ACTIVATE_DIFFICULTY,
            Purpose::RecoveryRequest => RECOVERY_REQUEST_DIFFICULTY,
        }
    }
}

/// A stateless challenge: the server signs it and forgets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Challenge {
    /// Unix seconds of issue.
    pub ts: u64,
    pub d: u8,
    pub purpose: Purpose,
    /// 8 random bytes: no two clients get one challenge.
    pub r: [u8; 8],
}

impl Challenge {
    fn signed(&self) -> [u8; SIGNED_LEN] {
        let mut out = [0u8; SIGNED_LEN];
        out[..8].copy_from_slice(&self.ts.to_be_bytes());
        out[8] = self.d;
        out[9] = self.purpose as u8;
        out[10..].copy_from_slice(&self.r);
        out
    }

    /// The 34 bytes with the HMAC of `server_key`.
    pub fn encode(&self, server_key: &[u8]) -> [u8; CHALLENGE_LEN] {
        let mut out = [0u8; CHALLENGE_LEN];
        let signed = self.signed();
        out[..SIGNED_LEN].copy_from_slice(&signed);
        out[SIGNED_LEN..].copy_from_slice(&mac(server_key, &signed));
        out
    }

    /// The fields without checking the HMAC: what the client reads to
    /// learn `d` and the purpose.
    pub fn peek(bytes: &[u8]) -> Result<Self, PowError> {
        if bytes.len() != CHALLENGE_LEN {
            return Err(PowError::Invalid);
        }
        let mut ts = [0u8; 8];
        ts.copy_from_slice(&bytes[..8]);
        let purpose = Purpose::from_byte(bytes[9]).ok_or(PowError::Invalid)?;
        let mut r = [0u8; 8];
        r.copy_from_slice(&bytes[10..18]);
        Ok(Challenge {
            ts: u64::from_be_bytes(ts),
            d: bytes[8],
            purpose,
            r,
        })
    }

    /// The fields, with the HMAC checked against any of `server_keys`
    /// (two live during a rotation).
    pub fn decode(bytes: &[u8], server_keys: &[&[u8]]) -> Result<Self, PowError> {
        let challenge = Self::peek(bytes)?;
        let signed = challenge.signed();
        let given = &bytes[SIGNED_LEN..];
        let ok = server_keys
            .iter()
            .any(|key| bool::from(mac(key, &signed).ct_eq(given)));
        if ok {
            Ok(challenge)
        } else {
            Err(PowError::Invalid)
        }
    }

    /// Whether `ts` is within `CHALLENGE_MAX_AGE_SECS` behind `now` and
    /// `CHALLENGE_MAX_FUTURE_SECS` ahead of it; `false` is `pow_expired`.
    pub fn is_fresh(&self, now: u64) -> bool {
        self.ts.saturating_add(CHALLENGE_MAX_AGE_SECS) >= now
            && self.ts <= now.saturating_add(CHALLENGE_MAX_FUTURE_SECS)
    }
}

/// The HMAC part of an encoded challenge, bytes 18..34: what the server
/// remembers for `CHALLENGE_MAX_AGE_SECS` once the challenge is solved, so
/// a second solution of it is `pow_replayed`. `None` for a wrong length.
pub fn replay_key(challenge: &[u8]) -> Option<[u8; MAC_LEN]> {
    if challenge.len() != CHALLENGE_LEN {
        return None;
    }
    let mut out = [0u8; MAC_LEN];
    out.copy_from_slice(&challenge[SIGNED_LEN..]);
    Some(out)
}

fn mac(key: &[u8], signed: &[u8]) -> [u8; MAC_LEN] {
    let mut out = [0u8; MAC_LEN];
    // HMAC takes a key of any length.
    if let Ok(mut m) = Hmac::<Sha256>::new_from_slice(key) {
        m.update(signed);
        out.copy_from_slice(&m.finalize().into_bytes()[..MAC_LEN]);
    }
    out
}

/// base64url without padding, as the challenge travels.
pub fn to_base64url(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn from_base64url(text: &str) -> Result<Vec<u8>, PowError> {
    URL_SAFE_NO_PAD.decode(text).map_err(|_| PowError::Invalid)
}

fn check_inputs(challenge: &[u8], d: u8) -> Result<(), PowError> {
    if d > MAX_DIFFICULTY {
        return Err(PowError::Difficulty(d));
    }
    if challenge.len() != CHALLENGE_LEN {
        return Err(PowError::Invalid);
    }
    Ok(())
}

/// `h` of a challenge and a nonce.
pub fn hash(challenge: &[u8], nonce: u64) -> Result<[u8; POW_OUT_LEN], PowError> {
    if challenge.len() != CHALLENGE_LEN {
        return Err(PowError::Invalid);
    }
    let mut out = [0u8; POW_OUT_LEN];
    kdf::argon2id(&nonce.to_be_bytes(), challenge, &POW_PARAMS, &mut out)
        .map_err(|_| PowError::Kdf)?;
    Ok(out)
}

/// The number of zero bits before the first one bit.
pub fn leading_zero_bits(h: &[u8]) -> u32 {
    let mut n = 0u32;
    for b in h {
        if *b == 0 {
            n += 8;
        } else {
            n += b.leading_zeros();
            break;
        }
    }
    n
}

/// The server side: `Ok` when the first `d` bits of `h` are zero.
pub fn verify(challenge: &[u8], nonce: u64, d: u8) -> Result<(), PowError> {
    check_inputs(challenge, d)?;
    let h = hash(challenge, nonce)?;
    let have = leading_zero_bits(&h);
    if have >= d as u32 {
        Ok(())
    } else {
        Err(PowError::Wrong {
            need: d,
            have: have as u8,
        })
    }
}

/// The client side: the first nonce from 0 whose `h` has `d` leading zero
/// bits. One thread; about 2^d attempts of one Argon2id each.
pub fn solve(challenge: &[u8], d: u8) -> Result<u64, PowError> {
    match solve_while(challenge, d, &mut || true)? {
        Some(nonce) => Ok(nonce),
        // `keep_going` never said stop.
        None => Err(PowError::Kdf),
    }
}

/// `solve` that asks `keep_going` before every attempt and gives
/// `Ok(None)` when it says no: for a background thread that the screen
/// may cancel. The memory of Argon2id is allocated once for all attempts.
pub fn solve_while(
    challenge: &[u8],
    d: u8,
    keep_going: &mut dyn FnMut() -> bool,
) -> Result<Option<u64>, PowError> {
    check_inputs(challenge, d)?;
    let argon = kdf::argon2_for(&POW_PARAMS, POW_OUT_LEN).map_err(|_| PowError::Kdf)?;
    let mut memory = vec![Block::new(); POW_MEMORY_KIB as usize];
    let mut out = [0u8; POW_OUT_LEN];
    let mut nonce = 0u64;
    while keep_going() {
        argon
            .hash_password_into_with_memory(
                &nonce.to_be_bytes(),
                challenge,
                &mut out,
                memory.as_mut_slice(),
            )
            .map_err(|_| PowError::Kdf)?;
        if leading_zero_bits(&out) >= d as u32 {
            return Ok(Some(nonce));
        }
        nonce = nonce.checked_add(1).ok_or(PowError::Kdf)?;
    }
    Ok(None)
}

/// The alphabet of Crockford base32, the one of the lock's recovery code.
pub const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
pub const CHECK_CODE_CHARS: usize = 12;

/// The check code of a recovery request: the first 60 bits of
/// `sha256(temp_pub)` as 12 Crockford base32 characters in groups of
/// four, `K8PV-5RHZ-2M2C` for `temp_pub` of 32 bytes `0xab`.
pub fn check_code(temp_pub: &[u8; 32]) -> String {
    let digest = Sha256::digest(temp_pub);
    let mut head = [0u8; 8];
    head.copy_from_slice(&digest[..8]);
    // 64 bits, the top 60 of them in 12 groups of 5.
    let bits = u64::from_be_bytes(head);
    let mut out = String::with_capacity(CHECK_CODE_CHARS + 2);
    for i in 0..CHECK_CODE_CHARS {
        if i > 0 && i % 4 == 0 {
            out.push('-');
        }
        let v = (bits >> (59 - 5 * i)) & 0x1f;
        out.push(CROCKFORD[v as usize] as char);
    }
    out
}

/// Whether a typed code is the code of `temp_pub`: case, hyphens and
/// spaces do not matter, and `O`, `I`, `L`, `U` are read as `0`, `1`, `1`,
/// `V`.
pub fn check_code_matches(temp_pub: &[u8; 32], typed: &str) -> bool {
    let expected = check_code(temp_pub).replace('-', "");
    let mut canonical = String::with_capacity(CHECK_CODE_CHARS);
    for ch in typed.chars() {
        if ch.is_whitespace() || ch == '-' {
            continue;
        }
        canonical.push(match ch.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            'U' => 'V',
            c => c,
        });
    }
    canonical.len() == expected.len() && bool::from(canonical.as_bytes().ct_eq(expected.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    const KEY: &[u8] = b"server-key-of-the-test";

    fn challenge(d: u8) -> [u8; CHALLENGE_LEN] {
        Challenge {
            ts: 1_791_000_000,
            d,
            purpose: Purpose::RecoveryRequest,
            r: [1, 2, 3, 4, 5, 6, 7, 8],
        }
        .encode(KEY)
    }

    #[test]
    fn challenge_layout_and_signature() {
        let c = Challenge {
            ts: 1_791_000_000,
            d: 8,
            purpose: Purpose::RecoveryRequest,
            r: [1, 2, 3, 4, 5, 6, 7, 8],
        };
        let bytes = c.encode(KEY);
        assert_eq!(bytes.len(), 34);
        assert_eq!(&bytes[..8], &1_791_000_000u64.to_be_bytes());
        assert_eq!(bytes[8], 8);
        assert_eq!(bytes[9], 2);
        assert_eq!(&bytes[10..18], &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(Challenge::peek(&bytes).unwrap(), c);
        assert_eq!(Challenge::decode(&bytes, &[KEY]).unwrap(), c);
        assert_eq!(Challenge::decode(&bytes, &[b"old", KEY]).unwrap(), c);
        assert_eq!(
            Challenge::decode(&bytes, &[b"other"]),
            Err(PowError::Invalid)
        );
        assert_eq!(Challenge::decode(&bytes, &[]), Err(PowError::Invalid));
        // The client cannot lower d: the HMAC covers it.
        let mut lowered = bytes;
        lowered[8] = 1;
        assert_eq!(Challenge::decode(&lowered, &[KEY]), Err(PowError::Invalid));
        let mut purpose = bytes;
        purpose[9] = 1;
        assert_eq!(Challenge::decode(&purpose, &[KEY]), Err(PowError::Invalid));
        purpose[9] = 3;
        assert_eq!(Challenge::peek(&purpose), Err(PowError::Invalid));
        assert_eq!(Challenge::peek(&bytes[..33]), Err(PowError::Invalid));
        let text = to_base64url(&bytes);
        assert_eq!(text.len(), 46);
        assert!(!text.contains('='));
        assert_eq!(from_base64url(&text).unwrap(), bytes);
        assert_eq!(from_base64url("not base64!"), Err(PowError::Invalid));
    }

    #[test]
    fn freshness_window() {
        let c = Challenge::peek(&challenge(8)).unwrap();
        let ts = c.ts;
        assert!(c.is_fresh(ts));
        assert!(c.is_fresh(ts + CHALLENGE_MAX_AGE_SECS));
        assert!(!c.is_fresh(ts + CHALLENGE_MAX_AGE_SECS + 1));
        assert!(c.is_fresh(ts - CHALLENGE_MAX_FUTURE_SECS));
        assert!(!c.is_fresh(ts - CHALLENGE_MAX_FUTURE_SECS - 1));
        assert_eq!(
            (CHALLENGE_MAX_AGE_SECS, CHALLENGE_MAX_FUTURE_SECS),
            (120, 60)
        );
        // The edges of the clock do not overflow.
        let late = Challenge { ts: u64::MAX, ..c };
        assert!(!late.is_fresh(0));
        let early = Challenge { ts: 0, ..c };
        assert!(!early.is_fresh(u64::MAX));
        assert!(early.is_fresh(100));
    }

    #[test]
    fn replay_key_is_the_mac() {
        let bytes = challenge(8);
        let key = replay_key(&bytes).unwrap();
        assert_eq!(&key[..], &bytes[18..34]);
        assert_eq!(key, mac(KEY, &bytes[..18]));
        // Another r, another key; the same challenge, the same key.
        let other = Challenge {
            r: [8, 7, 6, 5, 4, 3, 2, 1],
            ..Challenge::peek(&bytes).unwrap()
        }
        .encode(KEY);
        assert_ne!(replay_key(&other).unwrap(), key);
        assert_eq!(replay_key(&challenge(8)).unwrap(), key);
        assert_eq!(replay_key(&bytes[..33]), None);
        assert_eq!(replay_key(&[]), None);
    }

    #[test]
    fn purpose_names() {
        assert_eq!(Purpose::parse("activate"), Some(Purpose::Activate));
        assert_eq!(
            Purpose::parse("recovery.request"),
            Some(Purpose::RecoveryRequest)
        );
        assert_eq!(Purpose::parse("other"), None);
        assert_eq!(Purpose::Activate.as_str(), "activate");
        assert_eq!(Purpose::from_byte(0), None);
        assert_eq!(Purpose::Activate.base_difficulty(), 7);
        assert_eq!(Purpose::RecoveryRequest.base_difficulty(), 8);
    }

    #[test]
    fn hash_is_deterministic() {
        let c = challenge(8);
        let a = hash(&c, 5).unwrap();
        let b = hash(&c, 5).unwrap();
        assert_eq!(a, b);
        assert_ne!(hash(&c, 6).unwrap(), a);
        assert_ne!(hash(&challenge(7), 5).unwrap(), a);
        assert_eq!(hash(&c[..20], 5), Err(PowError::Invalid));
    }

    #[test]
    fn leading_zeros() {
        assert_eq!(leading_zero_bits(&[0, 0, 0x10]), 19);
        assert_eq!(leading_zero_bits(&[0x80]), 0);
        assert_eq!(leading_zero_bits(&[0x01]), 7);
        assert_eq!(leading_zero_bits(&[0, 0]), 16);
        assert_eq!(leading_zero_bits(&[]), 0);
    }

    #[test]
    fn solve_and_verify_small_difficulty() {
        for d in [2u8, 3] {
            let c = challenge(d);
            let started = Instant::now();
            let nonce = solve(&c, d).unwrap();
            let took = started.elapsed();
            assert!(took.as_secs() < 30, "d={d} took {took:?}");
            verify(&c, nonce, d).unwrap();
            // The solution is the first one: no smaller nonce works.
            for smaller in 0..nonce {
                assert!(matches!(
                    verify(&c, smaller, d),
                    Err(PowError::Wrong { .. })
                ));
            }
            // d = 0 accepts anything.
            verify(&c, nonce, 0).unwrap();
        }
    }

    #[test]
    fn verify_refuses_lower_work_and_bad_inputs() {
        let c = challenge(3);
        let nonce = solve(&c, 3).unwrap();
        let h = hash(&c, nonce).unwrap();
        let have = leading_zero_bits(&h);
        // Asking for more than it has.
        if have < 12 {
            let need = (have + 1) as u8;
            assert_eq!(
                verify(&c, nonce, need),
                Err(PowError::Wrong {
                    need,
                    have: have as u8
                })
            );
        }
        assert_eq!(verify(&c, nonce, 13), Err(PowError::Difficulty(13)));
        assert_eq!(solve(&c, 13), Err(PowError::Difficulty(13)));
        assert_eq!(verify(&c[..10], nonce, 3), Err(PowError::Invalid));
        assert_eq!(solve(&c[..10], 3), Err(PowError::Invalid));
    }

    #[test]
    fn solve_can_be_cancelled() {
        let c = challenge(12);
        let mut budget = 3;
        let mut keep_going = || {
            budget -= 1;
            budget >= 0
        };
        // Three attempts at d = 12 almost surely find nothing.
        let r = solve_while(&c, 12, &mut keep_going).unwrap();
        if let Some(nonce) = r {
            verify(&c, nonce, 12).unwrap();
        }
        let mut never = || false;
        assert_eq!(solve_while(&c, 1, &mut never).unwrap(), None);
    }

    #[test]
    #[ignore = "about 2^7 Argon2id of 16 MiB: seconds; run by hand to time a machine"]
    fn solve_d7_timing() {
        let c = Challenge {
            ts: 1_791_000_000,
            d: 7,
            purpose: Purpose::Activate,
            r: [9, 8, 7, 6, 5, 4, 3, 2],
        }
        .encode(KEY);
        let started = Instant::now();
        let nonce = solve(&c, 7).unwrap();
        let took = started.elapsed();
        verify(&c, nonce, 7).unwrap();
        eprintln!(
            "d=7: nonce {nonce} after {took:?} ({:.1} ms per attempt)",
            took.as_secs_f64() * 1000.0 / (nonce + 1) as f64
        );
    }

    #[test]
    fn check_code_vector_of_the_contract() {
        let temp_pub = [0xabu8; 32];
        assert_eq!(
            hex::encode(Sha256::digest(temp_pub)),
            "9a2db2e23f1504cd056606553ac049c5e718e8f9ce9233876df1a7a1821af885"
        );
        assert_eq!(check_code(&temp_pub), "K8PV-5RHZ-2M2C");
        assert!(check_code_matches(&temp_pub, "K8PV-5RHZ-2M2C"));
        assert!(check_code_matches(&temp_pub, "k8pv 5rhz 2m2c"));
        assert!(check_code_matches(&temp_pub, "K8PV5RHZ2M2C"));
        assert!(!check_code_matches(&temp_pub, "K8PV-5RHZ-2M2D"));
        assert!(!check_code_matches(&temp_pub, "K8PV-5RHZ-2M2"));
        assert!(!check_code_matches(&[0xacu8; 32], "K8PV-5RHZ-2M2C"));
        // Look-alikes fold.
        let code = check_code(&[0x01u8; 32]);
        let folded: String = code
            .chars()
            .map(|c| match c {
                '0' => 'O',
                '1' => 'I',
                'V' => 'U',
                c => c.to_ascii_lowercase(),
            })
            .collect();
        assert!(check_code_matches(&[0x01u8; 32], &folded));
    }

    #[test]
    fn check_code_shape() {
        for b in [0u8, 1, 0x7f, 0xff] {
            let code = check_code(&[b; 32]);
            assert_eq!(code.len(), 14);
            assert_eq!(code.as_bytes()[4], b'-');
            assert_eq!(code.as_bytes()[9], b'-');
            assert!(code
                .bytes()
                .filter(|c| *c != b'-')
                .all(|c| CROCKFORD.contains(&c)));
        }
    }
}
