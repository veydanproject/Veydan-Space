// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! GF(2^8) with the polynomial of SLIP-39, x^8 + x^4 + x^3 + x + 1 (0x11b,
//! the field of Rijndael too): exp and log tables built at compile time
//! with the generator 3. Multiplication takes no branch on the data (a
//! zero factor is handled by a mask), but the table lookups are
//! data-dependent memory accesses, so this is not a constant-time
//! multiplication: the shares are computed once, on the owner's device.

const POLY: u16 = 0x11b;

const fn build() -> ([u8; 256], [u8; 256]) {
    let mut exp = [0u8; 256];
    let mut log = [0u8; 256];
    let mut x: u16 = 1;
    let mut i = 0;
    while i < 255 {
        exp[i] = x as u8;
        log[x as usize] = i as u8;
        // x *= 3, that is x + x*2, reduced by the polynomial.
        x ^= x << 1;
        if x & 0x100 != 0 {
            x ^= POLY;
        }
        i += 1;
    }
    exp[255] = exp[0];
    (exp, log)
}

const TABLES: ([u8; 256], [u8; 256]) = build();
/// `EXP[i]` = 3^i; `EXP[255]` = `EXP[0]` = 1.
pub const EXP: [u8; 256] = TABLES.0;
/// `LOG[3^i]` = i for i in 0..255; `LOG[0]` is 0 and never meaningful.
pub const LOG: [u8; 256] = TABLES.1;

/// 0xff when `a` is zero, 0 otherwise, without a branch.
#[inline]
const fn zero_mask(a: u8) -> u8 {
    ((a as u16).wrapping_sub(1) >> 8) as u8
}

#[inline]
pub fn add(a: u8, b: u8) -> u8 {
    a ^ b
}

#[inline]
pub fn mul(a: u8, b: u8) -> u8 {
    let i = LOG[a as usize] as usize + LOG[b as usize] as usize;
    EXP[i % 255] & !(zero_mask(a) | zero_mask(b))
}

/// The inverse of `a`; `inv(0)` is 0, which no caller may rely on.
#[inline]
pub fn inv(a: u8) -> u8 {
    EXP[(255 - LOG[a as usize] as usize) % 255] & !zero_mask(a)
}

/// `a / b`; `div(a, 0)` is 0, which no caller may rely on.
#[inline]
pub fn div(a: u8, b: u8) -> u8 {
    mul(a, inv(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Schoolbook multiplication, the reference the tables are held to.
    fn slow_mul(mut a: u8, mut b: u8) -> u8 {
        let mut r = 0u8;
        while b != 0 {
            if b & 1 != 0 {
                r ^= a;
            }
            let carry = a & 0x80 != 0;
            a <<= 1;
            if carry {
                a ^= (POLY & 0xff) as u8;
            }
            b >>= 1;
        }
        r
    }

    #[test]
    fn fips_197_vectors() {
        // FIPS-197 §4.2: {57} • {83} = {c1}; §4.2.1: {57} • {13} = {fe}.
        assert_eq!(mul(0x57, 0x83), 0xc1);
        assert_eq!(mul(0x57, 0x13), 0xfe);
        // The inverse pair of the Rijndael S-box derivation: {53}^-1 = {ca}.
        assert_eq!(mul(0x53, 0xca), 0x01);
        assert_eq!(inv(0x53), 0xca);
        assert_eq!(inv(0xca), 0x53);
        assert_eq!(inv(1), 1);
    }

    #[test]
    fn slip39_reference_tables() {
        // The first entries of the exp table of the SLIP-39 reference
        // implementation (python-shamir-mnemonic, generator 3).
        let head = [
            1, 3, 5, 15, 17, 51, 85, 255, 26, 46, 114, 150, 161, 248, 19, 53,
        ];
        assert_eq!(&EXP[..16], &head);
        assert_eq!(EXP[255], 1);
        for i in 0..255u8 {
            assert_eq!(LOG[EXP[i as usize] as usize], i);
        }
        // 3 is a generator: the 255 powers are distinct.
        let mut seen = [false; 256];
        for v in &EXP[..255] {
            assert!(!seen[*v as usize]);
            seen[*v as usize] = true;
        }
        assert!(!seen[0]);
    }

    #[test]
    fn mul_matches_schoolbook_for_every_pair() {
        for a in 0..=255u8 {
            for b in 0..=255u8 {
                assert_eq!(mul(a, b), slow_mul(a, b), "{a} * {b}");
            }
        }
    }

    #[test]
    fn inverses_and_division() {
        for a in 1..=255u8 {
            assert_eq!(mul(a, inv(a)), 1, "{a}");
            assert_eq!(div(a, a), 1);
            assert_eq!(div(mul(a, 0x9d), 0x9d), a);
        }
        assert_eq!(inv(0), 0);
        assert_eq!(div(7, 0), 0);
        assert_eq!(mul(0, 0x80), 0);
        assert_eq!(mul(0x80, 0), 0);
    }

    #[test]
    fn distributive() {
        for a in [0u8, 1, 2, 0x53, 0x80, 0xff] {
            for b in [0u8, 7, 0x1b, 0xca, 0xfe] {
                for c in [1u8, 0x11, 0xa5] {
                    assert_eq!(mul(a, add(b, c)), add(mul(a, b), mul(a, c)));
                }
            }
        }
    }
}
