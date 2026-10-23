// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `Issued` as bytes: what the host seals to one member (NIP-44 in the
//! messenger) when it hands out a bundle or a page of missed keys.
//!
//! Layout (version 1), integers little-endian:
//!
//! | bytes | field |
//! |-------|-------|
//! | 4     | magic `VKA1` |
//! | 1     | version (1) |
//! | 1     | scheme (`SchemeId::code`) |
//! | 32    | community |
//! | 32    | scope |
//! | 4     | generation |
//! | 4     | leaf |
//! | 1     | kind: 0 initial, 1 planned, 2 emergency |
//! | 9     | planned only: reason (`MigrationReason::code`), grace end (u64) |
//! | 4     | bundle length (0: none) |
//! | n     | bundle (`Bundle::to_bytes`) |
//! | 1     | current: 0 none, 1 present |
//! | 36    | present only: epoch (u32), key (32) |
//! | 4     | count of missed keys |
//! | 36 × count | missed keys: epoch (u32), key (32), ascending |
//! | 1     | more missed keys follow: 0 or 1 |
//!
//! A page holds at most `MISSED_PAGE` keys: with an `sd-v1` bundle of
//! depth 24 it stays under 64 KiB, the plaintext limit of a NIP-44
//! envelope.

use crate::bundle::Bundle;
use crate::generation::{GenerationKind, MigrationReason};
use crate::{EpochKey, Error, Issued, Key32, SchemeId, ScopeRef, SecretBytes};

/// The magic of `Issued` bytes.
pub const ISSUED_MAGIC: &[u8; 4] = b"VKA1";
/// The version of the layout.
pub const ISSUED_VERSION: u8 = 1;
/// The missed keys of one page, at most.
pub const MISSED_PAGE: usize = 1000;

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.b.len()).ok_or(Error::Invalid("issued"))?;
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("4 bytes")))
    }

    fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("8 bytes")))
    }

    fn key(&mut self) -> Result<EpochKey, Error> {
        let epoch = self.u32()?;
        let key = Key32(self.take(32)?.try_into().expect("32 bytes"));
        Ok(EpochKey { epoch, key })
    }

    fn flag(&mut self) -> Result<bool, Error> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::Invalid("issued flag")),
        }
    }
}

impl Issued {
    /// The bytes of the layout above. They hold secrets.
    pub fn to_bytes(&self) -> SecretBytes {
        let mut out = SecretBytes::with_capacity(100 + self.bundle.len() + 36 * self.missed.len());
        out.extend(ISSUED_MAGIC);
        out.push(ISSUED_VERSION);
        out.push(self.scheme.code());
        out.extend(&self.scope.community);
        out.extend(&self.scope.scope);
        out.extend(&self.gen.to_le_bytes());
        out.extend(&self.leaf.to_le_bytes());
        match &self.kind {
            GenerationKind::Initial => out.push(0),
            GenerationKind::Planned { reason, grace_until } => {
                out.push(1);
                out.push(reason.code());
                out.extend(&grace_until.to_le_bytes());
            }
            GenerationKind::Emergency => out.push(2),
        }
        out.extend(&(self.bundle.len() as u32).to_le_bytes());
        out.extend(&self.bundle);
        let key = |out: &mut SecretBytes, k: &EpochKey| {
            out.extend(&k.epoch.to_le_bytes());
            out.extend(&k.key.0);
        };
        match &self.current {
            Some(k) => {
                out.push(1);
                key(&mut out, k);
            }
            None => out.push(0),
        }
        out.extend(&(self.missed.len() as u32).to_le_bytes());
        for k in &self.missed {
            key(&mut out, k);
        }
        out.push(u8::from(self.missed_more));
        out
    }

    /// Parses the layout above, refusing every length, kind or bundle
    /// that does not fit (a bundle must name the same scheme, scope,
    /// generation and leaf).
    pub fn from_bytes(b: &[u8]) -> Result<Self, Error> {
        let mut r = Reader { b, at: 0 };
        if r.take(4)? != ISSUED_MAGIC || r.u8()? != ISSUED_VERSION {
            return Err(Error::Invalid("issued"));
        }
        let scheme = SchemeId::from_code(r.u8()?).ok_or(Error::Invalid("issued scheme"))?;
        let scope = ScopeRef {
            community: r.take(32)?.try_into().expect("32 bytes"),
            scope: r.take(32)?.try_into().expect("32 bytes"),
        };
        let gen = r.u32()?;
        let leaf = r.u32()?;
        let kind = match r.u8()? {
            0 => GenerationKind::Initial,
            1 => {
                let reason = MigrationReason::from_code(r.u8()?).ok_or(Error::Invalid("issued kind"))?;
                GenerationKind::Planned { reason, grace_until: r.u64()? }
            }
            2 => GenerationKind::Emergency,
            _ => return Err(Error::Invalid("issued kind")),
        };
        let blen = r.u32()? as usize;
        let bundle = SecretBytes::new(r.take(blen)?.to_vec());
        if !bundle.is_empty() {
            let parsed = Bundle::from_bytes(&bundle)?;
            if parsed.scheme != scheme || parsed.scope != scope || parsed.gen != gen || parsed.leaf != leaf {
                return Err(Error::Invalid("issued bundle"));
            }
        }
        let current = if r.flag()? { Some(r.key()?) } else { None };
        let count = r.u32()? as usize;
        if count > MISSED_PAGE || count * 36 > b.len() - r.at {
            return Err(Error::Invalid("issued missed keys"));
        }
        let mut missed = Vec::with_capacity(count);
        for _ in 0..count {
            missed.push(r.key()?);
        }
        if missed.windows(2).any(|w| w[0].epoch >= w[1].epoch) {
            return Err(Error::Invalid("issued missed keys"));
        }
        let missed_more = r.flag()?;
        if r.at != b.len() {
            return Err(Error::Invalid("issued length"));
        }
        Ok(Issued { scheme, scope, gen, leaf, kind, bundle, current, missed, missed_more })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kdf::NodeKeys;
    use crate::tree::Tree;

    #[test]
    fn issued_round_trips_and_refuses_garbage() {
        let scope = ScopeRef { community: [1; 32], scope: [2; 32] };
        let tree = Tree::new(8).unwrap();
        let bundle = Bundle::issue(&NodeKeys::new(&Key32([3; 32]), &scope, 4), scope, 4, tree, 17).to_bytes();
        let k = |e| EpochKey { epoch: e, key: Key32([e as u8; 32]) };
        let full = Issued {
            scheme: SchemeId::CsV1,
            scope,
            gen: 4,
            leaf: 17,
            kind: GenerationKind::Planned { reason: MigrationReason::ShardSize, grace_until: 99 },
            bundle,
            current: Some(k(9)),
            missed: vec![k(5), k(6), k(8)],
            missed_more: true,
        };
        let page = Issued {
            kind: GenerationKind::Emergency,
            bundle: SecretBytes::default(),
            current: None,
            missed: vec![],
            missed_more: false,
            ..full.clone()
        };
        for i in [&full, &page] {
            let b = i.to_bytes();
            assert_eq!(&b[..4], ISSUED_MAGIC);
            assert_eq!(&Issued::from_bytes(&b).unwrap(), i);
            assert!(Issued::from_bytes(&b[..b.len() - 1]).is_err());
            let mut long = b.to_vec();
            long.push(0);
            assert!(Issued::from_bytes(&long).is_err());
        }
        // A bundle of another leaf, missed keys out of order, a bad flag.
        let mut other = full.clone();
        other.leaf = 18;
        assert!(Issued::from_bytes(&other.to_bytes()).is_err());
        let mut unordered = full.clone();
        unordered.missed.swap(0, 1);
        assert!(Issued::from_bytes(&unordered.to_bytes()).is_err());
        let mut b = page.to_bytes().to_vec();
        let last = b.len() - 1;
        b[last] = 2;
        assert!(Issued::from_bytes(&b).is_err());
        assert!(!format!("{full:?}").contains("[3, 3, 3"));
    }
}
