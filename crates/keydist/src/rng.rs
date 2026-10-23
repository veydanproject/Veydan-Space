// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The random source of a keeper: epoch keys, the nonce seeds of epochs,
//! the nonces of chains. A keeper runs on the system source; the seeded
//! generator and the way to put it into a keeper (`with_rng`) exist only
//! in the crate's own tests and under the feature `tools` (the simulator
//! and the benchmark), so that no production build can run a keeper on a
//! predictable source.

/// A source of random bytes.
pub trait Rng: Send {
    /// Fills `buf` with random bytes.
    fn fill(&mut self, buf: &mut [u8]);
}

/// The system random source (`getrandom`). A keeper without one must
/// not make keys, so a failure of the system source panics instead of
/// handing out a predictable key.
#[derive(Clone, Copy, Debug, Default)]
pub struct OsRng;

impl Rng for OsRng {
    fn fill(&mut self, buf: &mut [u8]) {
        getrandom::fill(buf).expect("the system random source failed");
    }
}

/// A seeded xorshift64* generator for tests and the simulator: the same
/// seed gives the same run. Not a cryptographic generator; a keeper that
/// guards real data never runs on it (it does not exist in a production
/// build).
#[cfg(any(test, feature = "tools"))]
#[derive(Clone, Debug)]
pub struct XorShift64(u64);

#[cfg(any(test, feature = "tools"))]
impl XorShift64 {
    /// A generator from a seed (zero is replaced, xorshift has no zero
    /// state).
    pub fn new(seed: u64) -> Self {
        XorShift64(if seed == 0 { 0x9e37_79b9_7f4a_7c15 } else { seed })
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// A number in `0..n` (`n` > 0), with the negligible bias of a modulo.
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

#[cfg(any(test, feature = "tools"))]
impl Rng for XorShift64 {
    fn fill(&mut self, buf: &mut [u8]) {
        for chunk in buf.chunks_mut(8) {
            let v = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&v[..chunk.len()]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_runs_repeat_and_os_differs() {
        let (mut a, mut b) = (XorShift64::new(5), XorShift64::new(5));
        let (mut x, mut y) = ([0u8; 13], [0u8; 13]);
        a.fill(&mut x);
        b.fill(&mut y);
        assert_eq!(x, y);
        let (mut p, mut q) = ([0u8; 32], [0u8; 32]);
        OsRng.fill(&mut p);
        OsRng.fill(&mut q);
        assert_ne!(p, q);
    }
}
