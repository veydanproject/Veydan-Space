// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Shamir's shares of the Recovery Key over GF(256)
//! (`internal/cloud-recovery.md` §8.2). Each of the 16 bytes of the key is
//! shared by its own random polynomial of degree k-1; a share holds the
//! value at `x = index`, 1..=n, never 0; the key is the value at 0.
//!
//! Share v1, 88 bytes, binary, never shown to a person:
//!
//! ```text
//! v(1) ‖ npub(32) ‖ generation u32 BE ‖ k ‖ n ‖ index ‖ y(16) ‖ commit(16) ‖ mac(16)
//! ```
//!
//! `commit` = HKDF(RK, share-commit/1)[..16], the same in every share of a
//! generation: an answer to "do these shares belong together" before any
//! interpolation. `mac` = HMAC-SHA256(HKDF(RK, share-mac/1), the 72 bytes
//! before it)[..16]: checked by whoever holds the key again, it names a
//! forged share.
//!
//! The public API takes `2 <= k <= n <= 7`, the bounds of the contract
//! (§8.1, `services/cloud/spec/recovery.md` §1): with k = 1 every share
//! would be the key itself. The field-level functions `split_byte` and
//! `interpolate` take any `1 <= k <= n <= 255`, for tests of the scheme.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::gf256;
use crate::kdf;
use crate::rk::{self, Label, RecoveryKey, HKDF_SALT, RK_LEN};

pub const SHARE_VERSION: u8 = 1;
pub const SHARE_LEN: usize = 88;
pub const SECRET_LEN: usize = RK_LEN;
/// The smallest threshold: below it a share is the key in the clear.
pub const MIN_THRESHOLD: u8 = 2;
/// The most shares one key is split into, as the contract fixes it.
pub const MAX_SHARES: u8 = 7;
/// The most distinct shares `combine` looks at. An honest set holds at
/// most n of them, possibly repeated by the devices of one person; the
/// bound keeps the search over subsets of k small whatever comes in.
pub const MAX_COMBINE_SHARES: usize = 24;
/// The bytes under the mac.
const SIGNED_LEN: usize = SHARE_LEN - 16;
const COMMIT_LEN: usize = 16;
const MAC_LEN: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ShareError {
    #[error("a share is {0} bytes, not 88")]
    Length(usize),
    #[error("share version {0} is not 1")]
    Version(u8),
    #[error("share parameters k={k} n={n} index={index} are out of range")]
    Parameters { k: u8, n: u8, index: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ShamirError {
    #[error("the threshold k={k} of n={n} is out of range (2 <= k <= n <= 7)")]
    Threshold { k: u8, n: u8 },
    #[error("fewer distinct indexes than k: have {have}, need {need}")]
    NotEnoughShares { have: u8, need: u8 },
    #[error("{0} distinct shares are more than combine looks at")]
    TooManyShares(usize),
    #[error("the shares belong to different owners")]
    MixedOwners,
    #[error("the shares belong to different generations")]
    MixedGenerations,
    #[error("the shares of one generation carry different commits or parameters")]
    MixedCommits,
    #[error("no k shares agree on the key: too many are forged")]
    NoConsistentSubset,
    #[error("the recovered bytes are not a valid recovery key")]
    InvalidKey,
    #[error("the system random source failed")]
    Random,
    #[error(transparent)]
    Share(#[from] ShareError),
}

/// One share. `Debug` shows the public fields and hides `y`.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct Share {
    pub npub: [u8; 32],
    pub generation: u32,
    pub k: u8,
    pub n: u8,
    pub index: u8,
    y: [u8; SECRET_LEN],
    commit: [u8; COMMIT_LEN],
    mac: [u8; MAC_LEN],
}

impl std::fmt::Debug for Share {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Share")
            .field("npub", &hex::encode(self.npub))
            .field("generation", &self.generation)
            .field("k", &self.k)
            .field("n", &self.n)
            .field("index", &self.index)
            .field("commit", &hex::encode(self.commit))
            .finish_non_exhaustive()
    }
}

impl Share {
    pub fn commit(&self) -> &[u8; COMMIT_LEN] {
        &self.commit
    }

    /// The 88 bytes of the wire. They hold `y`, so they come erased when
    /// dropped; a copy the caller makes (base64 for a note) is the
    /// caller's to erase.
    pub fn to_bytes(&self) -> Zeroizing<[u8; SHARE_LEN]> {
        let mut out = Zeroizing::new([0u8; SHARE_LEN]);
        self.write_signed(&mut out);
        out[SIGNED_LEN..].copy_from_slice(&self.mac);
        out
    }

    fn write_signed(&self, out: &mut [u8; SHARE_LEN]) {
        out[0] = SHARE_VERSION;
        out[1..33].copy_from_slice(&self.npub);
        out[33..37].copy_from_slice(&self.generation.to_be_bytes());
        out[37] = self.k;
        out[38] = self.n;
        out[39] = self.index;
        out[40..56].copy_from_slice(&self.y);
        out[56..72].copy_from_slice(&self.commit);
    }

    /// Checks the form: the length, the version, `2 <= k <= n <= 7`,
    /// `1 <= index <= n`. The mac is checked by `combine`.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ShareError> {
        if bytes.len() != SHARE_LEN {
            return Err(ShareError::Length(bytes.len()));
        }
        if bytes[0] != SHARE_VERSION {
            return Err(ShareError::Version(bytes[0]));
        }
        let (k, n, index) = (bytes[37], bytes[38], bytes[39]);
        if !threshold_ok(k, n) || index == 0 || index > n {
            return Err(ShareError::Parameters { k, n, index });
        }
        let mut share = Share {
            npub: [0u8; 32],
            generation: u32::from_be_bytes([bytes[33], bytes[34], bytes[35], bytes[36]]),
            k,
            n,
            index,
            y: [0u8; SECRET_LEN],
            commit: [0u8; COMMIT_LEN],
            mac: [0u8; MAC_LEN],
        };
        share.npub.copy_from_slice(&bytes[1..33]);
        share.y.copy_from_slice(&bytes[40..56]);
        share.commit.copy_from_slice(&bytes[56..72]);
        share.mac.copy_from_slice(&bytes[72..]);
        Ok(share)
    }

    fn mac_of(&self, mac_key: &[u8; 32]) -> [u8; MAC_LEN] {
        let mut signed = Zeroizing::new([0u8; SHARE_LEN]);
        self.write_signed(&mut signed);
        mac_bytes(mac_key, &signed[..SIGNED_LEN])
    }

    /// Whether the mac was made with this key.
    fn mac_ok(&self, mac_key: &[u8; 32]) -> bool {
        self.mac_of(mac_key).ct_eq(&self.mac).into()
    }
}

fn threshold_ok(k: u8, n: u8) -> bool {
    k >= MIN_THRESHOLD && k <= n && n <= MAX_SHARES
}

fn mac_bytes(mac_key: &[u8; 32], signed: &[u8]) -> [u8; MAC_LEN] {
    // HMAC accepts a key of any length: new_from_slice cannot fail on 32 bytes.
    let mut out = [0u8; MAC_LEN];
    if let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(mac_key) {
        mac.update(signed);
        let full = mac.finalize().into_bytes();
        out.copy_from_slice(&full[..MAC_LEN]);
    }
    out
}

/// The `commit` of a candidate secret, without the key check of
/// `RecoveryKey::from_bytes` (a secp256k1 derivation): the search over
/// subsets computes it for every candidate.
fn commit_of(secret: &[u8; SECRET_LEN]) -> [u8; COMMIT_LEN] {
    let full = rk::hkdf(secret, HKDF_SALT, Label::ShareCommit.info());
    let mut out = [0u8; COMMIT_LEN];
    out.copy_from_slice(&full[..COMMIT_LEN]);
    out
}

/// The key put back together, with what the shares said about it.
#[derive(Debug)]
pub struct Recovered {
    pub rk: RecoveryKey,
    pub npub: [u8; 32],
    pub generation: u32,
    pub k: u8,
    pub n: u8,
    /// Positions in the `shares` given to `combine` whose mac does not
    /// match the recovered key: shares someone changed. The caller maps a
    /// position to the sender of the seal it came in; the `index` inside
    /// a forged share is the forger's and names nobody. The key itself
    /// is right regardless.
    pub forged: Vec<usize>,
}

/// Splits the key into `n` shares of which any `k` give it back, with
/// coefficients from the system random source.
pub fn split(
    rk: &RecoveryKey,
    npub: &[u8; 32],
    generation: u32,
    k: u8,
    n: u8,
) -> Result<Vec<Share>, ShamirError> {
    split_with(rk, npub, generation, k, n, &mut |buf| {
        kdf::random(buf).map_err(|_| ShamirError::Random)
    })
}

/// `split` with the coefficients from `fill`: for test vectors and for a
/// caller with its own source. `fill` is asked once for `16 * (k - 1)`
/// bytes, the coefficients of the 16 polynomials in turn.
pub fn split_with(
    rk: &RecoveryKey,
    npub: &[u8; 32],
    generation: u32,
    k: u8,
    n: u8,
    fill: &mut dyn FnMut(&mut [u8]) -> Result<(), ShamirError>,
) -> Result<Vec<Share>, ShamirError> {
    if !threshold_ok(k, n) {
        return Err(ShamirError::Threshold { k, n });
    }
    let degree = (k - 1) as usize;
    let mut coefficients = Zeroizing::new(vec![0u8; SECRET_LEN * degree]);
    fill(&mut coefficients)?;
    let commit = rk.share_commit();
    let mac_key = rk.share_mac_key();
    let secret = rk.as_bytes();
    let mut shares = Vec::with_capacity(n as usize);
    for index in 1..=n {
        let mut y = [0u8; SECRET_LEN];
        for (b, slot) in y.iter_mut().enumerate() {
            let coeffs = &coefficients[b * degree..(b + 1) * degree];
            *slot = evaluate(secret[b], coeffs, index);
        }
        let mut share = Share {
            npub: *npub,
            generation,
            k,
            n,
            index,
            y,
            commit,
            mac: [0u8; MAC_LEN],
        };
        share.mac = share.mac_of(&mac_key);
        shares.push(share);
    }
    Ok(shares)
}

/// The polynomial `secret + c[0] x + c[1] x^2 + ...` at `x` (Horner from
/// the top coefficient).
fn evaluate(secret: u8, coefficients: &[u8], x: u8) -> u8 {
    let mut acc = 0u8;
    for c in coefficients.iter().rev() {
        acc = gf256::add(gf256::mul(acc, x), *c);
    }
    gf256::add(gf256::mul(acc, x), secret)
}

/// Shares of one byte: `(index, y)` for `index` in `1..=n`, with the given
/// `k - 1` coefficients. The field-level view of `split`, for tests of
/// the scheme itself: `1 <= k <= n <= 255`, k = 1 included.
pub fn split_byte(
    secret: u8,
    k: u8,
    n: u8,
    coefficients: &[u8],
) -> Result<Vec<(u8, u8)>, ShamirError> {
    if k == 0 || k > n || coefficients.len() != (k - 1) as usize {
        return Err(ShamirError::Threshold { k, n });
    }
    Ok((1..=n)
        .map(|x| (x, evaluate(secret, coefficients, x)))
        .collect())
}

/// Lagrange interpolation of the polynomial through `points` at `at`.
/// `None` for no points or two points with one `x`.
pub fn interpolate(points: &[(u8, u8)], at: u8) -> Option<u8> {
    let xs: Vec<u8> = points.iter().map(|p| p.0).collect();
    let coefficients = lagrange_coefficients(&xs, at)?;
    Some(
        points
            .iter()
            .zip(coefficients.iter())
            .fold(0u8, |acc, (p, c)| gf256::add(acc, gf256::mul(p.1, *c))),
    )
}

/// `l_i(at)` for every `x_i`: the weights of the values in the
/// interpolation at `at`. The same for all 16 bytes of a key.
fn lagrange_coefficients(xs: &[u8], at: u8) -> Option<Vec<u8>> {
    if xs.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(xs.len());
    for (i, xi) in xs.iter().enumerate() {
        let mut num = 1u8;
        let mut den = 1u8;
        for (j, xj) in xs.iter().enumerate() {
            if i == j {
                continue;
            }
            if xi == xj {
                return None;
            }
            num = gf256::mul(num, gf256::add(at, *xj));
            den = gf256::mul(den, gf256::add(*xi, *xj));
        }
        out.push(gf256::div(num, den));
    }
    Some(out)
}

/// Puts the key back together from the shares of one owner and one
/// generation. The shares must agree on owner, generation, commit and
/// parameters. Shares equal byte for byte count once; two different
/// shares with one index both stay, since the index is written by whoever
/// sent the share, and neither is believed over the other. Every subset
/// of `k` shares with distinct indexes is tried until the recovered key
/// reproduces the commit; then the mac of every share given is checked
/// and the forged ones are named by their position in `shares`. A forger
/// can thus delay the answer, never veto it: with k honest shares in the
/// set the key comes out.
pub fn combine(shares: &[Share]) -> Result<Recovered, ShamirError> {
    let first = shares.first().ok_or(ShamirError::NotEnoughShares {
        have: 0,
        need: MIN_THRESHOLD,
    })?;
    for s in shares {
        if s.npub != first.npub {
            return Err(ShamirError::MixedOwners);
        }
        if s.generation != first.generation {
            return Err(ShamirError::MixedGenerations);
        }
        if s.commit.ct_ne(&first.commit).into() || s.k != first.k || s.n != first.n {
            return Err(ShamirError::MixedCommits);
        }
    }
    // Each distinct share once, in the order given.
    let mut distinct: Vec<&Share> = Vec::with_capacity(shares.len());
    for s in shares {
        if !distinct.contains(&s) {
            distinct.push(s);
        }
    }
    if distinct.len() > MAX_COMBINE_SHARES {
        return Err(ShamirError::TooManyShares(distinct.len()));
    }
    let mut indexes: Vec<u8> = distinct.iter().map(|s| s.index).collect();
    indexes.sort_unstable();
    indexes.dedup();
    let k = first.k as usize;
    if indexes.len() < k {
        return Err(ShamirError::NotEnoughShares {
            have: indexes.len() as u8,
            need: first.k,
        });
    }
    let secret = find_consistent_subset(&distinct, k, &first.commit)?;
    let rk = RecoveryKey::from_bytes(*secret).map_err(|_| ShamirError::InvalidKey)?;
    let mac_key = rk.share_mac_key();
    let forged = shares
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.mac_ok(&mac_key))
        .map(|(position, _)| position)
        .collect();
    Ok(Recovered {
        rk,
        npub: first.npub,
        generation: first.generation,
        k: first.k,
        n: first.n,
        forged,
    })
}

fn find_consistent_subset(
    shares: &[&Share],
    k: usize,
    commit: &[u8; COMMIT_LEN],
) -> Result<Zeroizing<[u8; SECRET_LEN]>, ShamirError> {
    let mut chosen: Vec<usize> = (0..k).collect();
    loop {
        let xs: Vec<u8> = chosen.iter().map(|i| shares[*i].index).collect();
        // Distinct indexes: the coefficients exist. A subset with a
        // repeated index is skipped, not an error.
        if let Some(coefficients) = lagrange_coefficients(&xs, 0) {
            let mut secret = Zeroizing::new([0u8; SECRET_LEN]);
            for (b, slot) in secret.iter_mut().enumerate() {
                *slot = chosen
                    .iter()
                    .zip(coefficients.iter())
                    .fold(0u8, |acc, (i, c)| {
                        gf256::add(acc, gf256::mul(shares[*i].y[b], *c))
                    });
            }
            if commit_of(&secret).ct_eq(commit).into() {
                return Ok(secret);
            }
        }
        if !next_combination(&mut chosen, shares.len()) {
            return Err(ShamirError::NoConsistentSubset);
        }
    }
}

/// The next k-combination of `0..m` in lexicographic order; `false` after
/// the last one.
fn next_combination(chosen: &mut [usize], m: usize) -> bool {
    let k = chosen.len();
    let mut i = k;
    while i > 0 {
        i -= 1;
        if chosen[i] < m - k + i {
            chosen[i] += 1;
            for j in i + 1..k {
                chosen[j] = chosen[j - 1] + 1;
            }
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const RK: [u8; 16] = [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
        0xff,
    ];
    const NPUB: [u8; 32] = [0xab; 32];

    fn rk() -> RecoveryKey {
        RecoveryKey::from_bytes(RK).unwrap()
    }

    /// The share with `index` written over, as a forger relabels one.
    fn relabelled(share: &Share, index: u8) -> Share {
        let mut b = share.to_bytes();
        b[39] = index;
        Share::from_bytes(&b[..]).unwrap()
    }

    #[test]
    fn share_bytes_layout() {
        let shares = split(&rk(), &NPUB, 7, 3, 5).unwrap();
        assert_eq!(shares.len(), 5);
        let bytes = shares[2].to_bytes();
        assert_eq!(bytes.len(), SHARE_LEN);
        assert_eq!(bytes[0], 1);
        assert_eq!(&bytes[1..33], &NPUB);
        assert_eq!(&bytes[33..37], &7u32.to_be_bytes());
        assert_eq!(&bytes[37..40], &[3, 5, 3]);
        assert_eq!(&bytes[56..72], &rk().share_commit());
        let parsed = Share::from_bytes(&bytes[..]).unwrap();
        assert_eq!(parsed, shares[2]);
        assert!(format!("{:?}", shares[2]).contains("index: 3"));
        assert!(!format!("{:?}", shares[2]).contains("y:"));
    }

    #[test]
    fn share_form_errors() {
        let shares = split(&rk(), &NPUB, 1, 2, 3).unwrap();
        let bytes = shares[0].to_bytes();
        assert_eq!(Share::from_bytes(&bytes[..87]), Err(ShareError::Length(87)));
        let mut v = bytes.clone();
        v[0] = 2;
        assert_eq!(Share::from_bytes(&v[..]), Err(ShareError::Version(2)));
        let mut idx0 = bytes.clone();
        idx0[39] = 0;
        assert!(matches!(
            Share::from_bytes(&idx0[..]),
            Err(ShareError::Parameters { index: 0, .. })
        ));
        let mut idx_hi = bytes.clone();
        idx_hi[39] = 4;
        assert!(matches!(
            Share::from_bytes(&idx_hi[..]),
            Err(ShareError::Parameters { index: 4, .. })
        ));
        let mut k_hi = bytes.clone();
        k_hi[37] = 4;
        assert!(matches!(
            Share::from_bytes(&k_hi[..]),
            Err(ShareError::Parameters { k: 4, .. })
        ));
        // k = 1 would be the key in the clear: refused at the parser too.
        let mut k_one = bytes.clone();
        k_one[37] = 1;
        assert!(matches!(
            Share::from_bytes(&k_one[..]),
            Err(ShareError::Parameters { k: 1, .. })
        ));
        let mut n_hi = bytes.clone();
        n_hi[38] = 8;
        n_hi[37] = 8;
        n_hi[39] = 8;
        assert!(matches!(
            Share::from_bytes(&n_hi[..]),
            Err(ShareError::Parameters { n: 8, .. })
        ));
    }

    #[test]
    fn threshold_bounds() {
        assert_eq!(
            split(&rk(), &NPUB, 1, 0, 3).unwrap_err(),
            ShamirError::Threshold { k: 0, n: 3 }
        );
        assert_eq!(
            split(&rk(), &NPUB, 1, 1, 3).unwrap_err(),
            ShamirError::Threshold { k: 1, n: 3 }
        );
        assert_eq!(
            split(&rk(), &NPUB, 1, 1, 1).unwrap_err(),
            ShamirError::Threshold { k: 1, n: 1 }
        );
        assert_eq!(
            split(&rk(), &NPUB, 1, 4, 3).unwrap_err(),
            ShamirError::Threshold { k: 4, n: 3 }
        );
        assert_eq!(
            split(&rk(), &NPUB, 1, 2, 8).unwrap_err(),
            ShamirError::Threshold { k: 2, n: 8 }
        );
        assert!(split(&rk(), &NPUB, 1, 2, 2).is_ok());
        assert!(split(&rk(), &NPUB, 1, 7, 7).is_ok());
        assert_eq!(MIN_THRESHOLD, 2);
        assert_eq!(MAX_SHARES, 7);
    }

    #[test]
    fn roundtrip_all_k_n_of_the_contract() {
        for n in MIN_THRESHOLD..=MAX_SHARES {
            for k in MIN_THRESHOLD..=n {
                let shares = split(&rk(), &NPUB, 3, k, n).unwrap();
                let got = combine(&shares).unwrap();
                assert_eq!(got.rk, rk(), "k={k} n={n}");
                assert_eq!((got.k, got.n, got.generation, got.npub), (k, n, 3, NPUB));
                assert!(got.forged.is_empty());
                // The first k alone are enough.
                let got = combine(&shares[..k as usize]).unwrap();
                assert_eq!(got.rk, rk());
            }
        }
    }

    /// §13: roundtrip for all 1 <= k <= n <= 8, at the field level, where
    /// k = 1 and n = 8 are allowed (the API stops at 2 <= k <= n <= 7).
    #[test]
    fn field_roundtrip_all_k_n_up_to_8() {
        for n in 1..=8u8 {
            for k in 1..=n {
                for secret in [0u8, 1, 0x5a, 0xff] {
                    let coefficients: Vec<u8> =
                        (0..k - 1).map(|i| 0x9d ^ (i * 41) ^ secret).collect();
                    let shares = split_byte(secret, k, n, &coefficients).unwrap();
                    assert_eq!(shares.len(), n as usize);
                    assert_eq!(
                        interpolate(&shares[..k as usize], 0),
                        Some(secret),
                        "k={k} n={n}"
                    );
                    assert_eq!(
                        interpolate(&shares[(n - k) as usize..], 0),
                        Some(secret),
                        "k={k} n={n}"
                    );
                    if k == 1 {
                        assert!(shares.iter().all(|(_, y)| *y == secret));
                    }
                }
            }
        }
    }

    #[test]
    fn every_subset_of_k_for_n_up_to_5() {
        for n in MIN_THRESHOLD..=5u8 {
            for k in MIN_THRESHOLD..=n {
                let shares = split(&rk(), &NPUB, 1, k, n).unwrap();
                let mut chosen: Vec<usize> = (0..k as usize).collect();
                loop {
                    let subset: Vec<Share> = chosen.iter().map(|i| shares[*i].clone()).collect();
                    assert_eq!(combine(&subset).unwrap().rk, rk(), "k={k} n={n} {chosen:?}");
                    if !next_combination(&mut chosen, n as usize) {
                        break;
                    }
                }
            }
        }
    }

    #[test]
    fn fewer_than_k_is_an_error() {
        let shares = split(&rk(), &NPUB, 1, 3, 5).unwrap();
        assert_eq!(
            combine(&shares[..2]).unwrap_err(),
            ShamirError::NotEnoughShares { have: 2, need: 3 }
        );
        assert_eq!(
            combine(&[]).unwrap_err(),
            ShamirError::NotEnoughShares { have: 0, need: 2 }
        );
        // Duplicates of one share do not count twice.
        let dup = vec![shares[0].clone(), shares[0].clone(), shares[1].clone()];
        assert_eq!(
            combine(&dup).unwrap_err(),
            ShamirError::NotEnoughShares { have: 2, need: 3 }
        );
        // Nor do two different shares with one index.
        let same_index = vec![
            shares[0].clone(),
            relabelled(&shares[2], 1),
            shares[1].clone(),
        ];
        assert_eq!(
            combine(&same_index).unwrap_err(),
            ShamirError::NotEnoughShares { have: 2, need: 3 }
        );
        let ok = vec![
            shares[0].clone(),
            shares[0].clone(),
            shares[1].clone(),
            shares[2].clone(),
        ];
        let got = combine(&ok).unwrap();
        assert_eq!(got.rk, rk());
        assert!(got.forged.is_empty());
    }

    #[test]
    fn k_minus_one_shares_say_nothing_about_one_byte() {
        // For every k, any k-1 shares of a byte are consistent with each of
        // the 256 candidate secrets: for each candidate exactly one
        // polynomial of degree k-1 passes through the k-1 shares and (0,
        // candidate), and its values at the remaining indexes are valid
        // shares. The map candidate -> value at a hidden index is a
        // bijection for every k: l_0(x) != 0 for a non-zero x outside the
        // known indexes, so one more share leaves no freedom and one less
        // leaves all 256 secrets possible.
        for k in 2..=5u8 {
            let n = k + 2;
            let coefficients: Vec<u8> = (0..k - 1).map(|i| 0x37 ^ (i * 29)).collect();
            let shares = split_byte(0x5a, k, n, &coefficients).unwrap();
            let known = &shares[..(k - 1) as usize];
            for hidden in k..=n {
                let mut seen_at_hidden = [false; 256];
                for candidate in 0..=255u8 {
                    let mut points = known.to_vec();
                    points.push((0, candidate));
                    let at_hidden = interpolate(&points, hidden).unwrap();
                    seen_at_hidden[at_hidden as usize] = true;
                    // And it still passes through the known shares.
                    for (x, y) in known {
                        assert_eq!(interpolate(&points, *x), Some(*y));
                    }
                }
                assert!(
                    seen_at_hidden.iter().all(|s| *s),
                    "k={k} hidden index {hidden}: not every byte is a possible share"
                );
            }
            // With the k-th share the real secret and nothing else comes out.
            assert_eq!(interpolate(&shares[..k as usize], 0), Some(0x5a));
        }
    }

    #[test]
    fn split_byte_and_interpolate_agree_with_split() {
        let shares = split(&rk(), &NPUB, 1, 2, 3).unwrap();
        for (b, expected) in RK.iter().enumerate() {
            let points: Vec<(u8, u8)> = shares.iter().map(|s| (s.index, s.y[b])).collect();
            assert_eq!(interpolate(&points, 0), Some(*expected));
        }
        assert_eq!(interpolate(&[], 0), None);
        assert_eq!(interpolate(&[(1, 5), (1, 6)], 0), None);
        assert_eq!(interpolate(&[(3, 9)], 0), Some(9));
        assert!(split_byte(1, 3, 5, &[1]).is_err());
        assert!(split_byte(1, 0, 5, &[]).is_err());
        assert!(split_byte(1, 6, 5, &[0; 5]).is_err());
        // k = 1 at the field level: every share is the secret.
        assert_eq!(
            split_byte(0x42, 1, 3, &[]).unwrap(),
            vec![(1, 0x42), (2, 0x42), (3, 0x42)]
        );
    }

    #[test]
    fn tampered_bit_names_the_share() {
        let shares = split(&rk(), &NPUB, 1, 3, 5).unwrap();
        for victim in 0..5 {
            for bit in 0..(SHARE_LEN * 8) {
                let byte = bit / 8;
                // Not the fields that make the share another group or
                // malformed: version, npub, generation, k, n, commit. The
                // index is included when the flip leaves it in 1..=n.
                if byte < 39 || (56..72).contains(&byte) {
                    continue;
                }
                let mut bytes = shares[victim].to_bytes();
                bytes[byte] ^= 1 << (bit % 8);
                let Ok(forged) = Share::from_bytes(&bytes[..]) else {
                    assert_eq!(byte, 39, "victim {victim} bit {bit}");
                    continue;
                };
                let mut set: Vec<Share> = shares.clone();
                set[victim] = forged;
                let got = combine(&set).unwrap();
                assert_eq!(got.rk, rk(), "victim {victim} bit {bit}");
                assert_eq!(got.forged, vec![victim], "victim {victim} bit {bit}");
            }
        }
    }

    #[test]
    fn swapped_index_is_named_by_position() {
        let shares = split(&rk(), &NPUB, 1, 3, 5).unwrap();
        // The holder of index 2 answers relabelled as the absent index 4:
        // the position of the answer is named, not "index 4", whose
        // holder did nothing.
        let set = vec![
            shares[0].clone(),
            relabelled(&shares[1], 4),
            shares[2].clone(),
            shares[4].clone(),
        ];
        let got = combine(&set).unwrap();
        assert_eq!(got.rk, rk());
        assert_eq!(got.forged, vec![1]);
    }

    #[test]
    fn copied_index_does_not_veto() {
        let shares = split(&rk(), &NPUB, 1, 3, 5).unwrap();
        // Exactly k honest shares and a forger who copies index 1.
        let set = vec![
            shares[0].clone(),
            shares[1].clone(),
            shares[2].clone(),
            relabelled(&shares[3], 1),
        ];
        let got = combine(&set).unwrap();
        assert_eq!(got.rk, rk());
        assert_eq!(got.forged, vec![3]);
        // The forger first: the honest share of index 1 is still believed.
        let set = vec![
            relabelled(&shares[3], 1),
            shares[0].clone(),
            shares[1].clone(),
            shares[2].clone(),
        ];
        let got = combine(&set).unwrap();
        assert_eq!(got.rk, rk());
        assert_eq!(got.forged, vec![0]);
        // Two forgers with the same copied index, and one with changed y.
        let mut y_changed = shares[4].to_bytes();
        y_changed[45] ^= 0x80;
        let set = vec![
            shares[0].clone(),
            relabelled(&shares[3], 2),
            shares[1].clone(),
            relabelled(&shares[4], 2),
            shares[2].clone(),
            Share::from_bytes(&y_changed[..]).unwrap(),
        ];
        let got = combine(&set).unwrap();
        assert_eq!(got.rk, rk());
        assert_eq!(got.forged, vec![1, 3, 5]);
    }

    #[test]
    fn too_many_shares_is_an_error() {
        let shares = split(&rk(), &NPUB, 1, 3, 5).unwrap();
        let mut flood: Vec<Share> = Vec::new();
        for i in 0..MAX_COMBINE_SHARES as u8 + 1 {
            let mut b = shares[0].to_bytes();
            b[40] = i;
            flood.push(Share::from_bytes(&b[..]).unwrap());
        }
        assert_eq!(
            combine(&flood).unwrap_err(),
            ShamirError::TooManyShares(MAX_COMBINE_SHARES + 1)
        );
        // The same shares repeated are not many.
        let repeated: Vec<Share> = (0..40).map(|i| shares[i % 5].clone()).collect();
        assert_eq!(combine(&repeated).unwrap().rk, rk());
    }

    #[test]
    fn too_many_forged_is_an_error() {
        let shares = split(&rk(), &NPUB, 1, 3, 3).unwrap();
        let mut bytes = shares[0].to_bytes();
        bytes[40] ^= 0xff;
        let set = vec![
            Share::from_bytes(&bytes[..]).unwrap(),
            shares[1].clone(),
            shares[2].clone(),
        ];
        assert_eq!(combine(&set).unwrap_err(), ShamirError::NoConsistentSubset);
    }

    #[test]
    fn generations_and_owners_do_not_mix() {
        let gen1 = split(&rk(), &NPUB, 1, 2, 3).unwrap();
        let gen2 = split(&rk(), &NPUB, 2, 2, 3).unwrap();
        let mixed = vec![gen1[0].clone(), gen2[1].clone()];
        assert_eq!(combine(&mixed).unwrap_err(), ShamirError::MixedGenerations);
        let other_owner = split(&rk(), &[0xcd; 32], 1, 2, 3).unwrap();
        let mixed = vec![gen1[0].clone(), other_owner[1].clone()];
        assert_eq!(combine(&mixed).unwrap_err(), ShamirError::MixedOwners);
        // Same owner and generation, another key: the commit differs
        // before any interpolation.
        let other_key = RecoveryKey::from_bytes([9u8; 16]).unwrap();
        let other = split(&other_key, &NPUB, 1, 2, 3).unwrap();
        assert_ne!(other[0].commit(), gen1[0].commit());
        let mixed = vec![gen1[0].clone(), other[1].clone()];
        assert_eq!(combine(&mixed).unwrap_err(), ShamirError::MixedCommits);
        // Same commit, different k: refused too.
        let mut b = gen1[1].to_bytes();
        b[37] = 3;
        let mixed = vec![gen1[0].clone(), Share::from_bytes(&b[..]).unwrap()];
        assert_eq!(combine(&mixed).unwrap_err(), ShamirError::MixedCommits);
    }

    #[test]
    fn split_is_randomized_and_split_with_is_deterministic() {
        let a = split(&rk(), &NPUB, 1, 2, 3).unwrap();
        let b = split(&rk(), &NPUB, 1, 2, 3).unwrap();
        assert_ne!(a[0].y, b[0].y);
        assert_eq!(a[0].commit, b[0].commit);
        let mut fill = |buf: &mut [u8]| {
            for (i, b) in buf.iter_mut().enumerate() {
                *b = i as u8;
            }
            Ok(())
        };
        let c = split_with(&rk(), &NPUB, 1, 2, 3, &mut fill).unwrap();
        let d = split_with(&rk(), &NPUB, 1, 2, 3, &mut fill).unwrap();
        assert_eq!(c, d);
        assert_eq!(combine(&c).unwrap().rk, rk());
        // No share is the key.
        assert!(c.iter().all(|s| s.y != RK));
    }

    #[test]
    fn commit_of_matches_the_key() {
        assert_eq!(commit_of(&RK), rk().share_commit());
        assert_ne!(commit_of(&[1u8; 16]), rk().share_commit());
    }

    #[test]
    fn combinations_enumerate() {
        let mut c = vec![0, 1];
        let mut count = 1;
        while next_combination(&mut c, 5) {
            count += 1;
        }
        assert_eq!(count, 10);
        assert_eq!(c, vec![3, 4]);
        let mut all = vec![0, 1, 2];
        assert!(!next_combination(&mut all, 3));
    }
}
