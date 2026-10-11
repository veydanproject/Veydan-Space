// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The little of CBOR (RFC 8949) the escrow and the device-link payload
//! need: unsigned integers, byte and text strings, null and maps with text
//! keys, all of definite length. Written here rather than taken from a
//! crate: the two formats are fixed, and the reader must accept nothing
//! else.

const MAJOR_UINT: u8 = 0;
const MAJOR_BYTES: u8 = 2;
const MAJOR_TEXT: u8 = 3;
const MAJOR_MAP: u8 = 5;
const NULL: u8 = 0xf6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Malformed;

#[derive(Default)]
pub struct Encoder(Vec<u8>);

impl Encoder {
    pub fn new() -> Self {
        Self::default()
    }

    fn head(&mut self, major: u8, value: u64) {
        let m = major << 5;
        if value < 24 {
            self.0.push(m | value as u8);
        } else if value <= u8::MAX as u64 {
            self.0.push(m | 24);
            self.0.push(value as u8);
        } else if value <= u16::MAX as u64 {
            self.0.push(m | 25);
            self.0.extend_from_slice(&(value as u16).to_be_bytes());
        } else if value <= u32::MAX as u64 {
            self.0.push(m | 26);
            self.0.extend_from_slice(&(value as u32).to_be_bytes());
        } else {
            self.0.push(m | 27);
            self.0.extend_from_slice(&value.to_be_bytes());
        }
    }

    pub fn uint(&mut self, v: u64) -> &mut Self {
        self.head(MAJOR_UINT, v);
        self
    }

    pub fn bytes(&mut self, v: &[u8]) -> &mut Self {
        self.head(MAJOR_BYTES, v.len() as u64);
        self.0.extend_from_slice(v);
        self
    }

    pub fn text(&mut self, v: &str) -> &mut Self {
        self.head(MAJOR_TEXT, v.len() as u64);
        self.0.extend_from_slice(v.as_bytes());
        self
    }

    pub fn null(&mut self) -> &mut Self {
        self.0.push(NULL);
        self
    }

    /// A map of `len` pairs; the pairs follow as key, value, key, value.
    pub fn map(&mut self, len: usize) -> &mut Self {
        self.head(MAJOR_MAP, len as u64);
        self
    }

    pub fn finish(self) -> Vec<u8> {
        self.0
    }
}

pub struct Decoder<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn byte(&mut self) -> Result<u8, Malformed> {
        let b = *self.buf.get(self.pos).ok_or(Malformed)?;
        self.pos += 1;
        Ok(b)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Malformed> {
        let end = self.pos.checked_add(n).ok_or(Malformed)?;
        let s = self.buf.get(self.pos..end).ok_or(Malformed)?;
        self.pos = end;
        Ok(s)
    }

    /// The head of the next item: its major type and its argument. Only
    /// the shortest encoding of the argument is accepted.
    fn head(&mut self) -> Result<(u8, u64), Malformed> {
        let b = self.byte()?;
        let major = b >> 5;
        let info = b & 0x1f;
        let value = match info {
            0..=23 => info as u64,
            24 => {
                let v = self.byte()? as u64;
                if v < 24 {
                    return Err(Malformed);
                }
                v
            }
            25 => {
                let v = u16::from_be_bytes(self.take(2)?.try_into().map_err(|_| Malformed)?) as u64;
                if v <= u8::MAX as u64 {
                    return Err(Malformed);
                }
                v
            }
            26 => {
                let v = u32::from_be_bytes(self.take(4)?.try_into().map_err(|_| Malformed)?) as u64;
                if v <= u16::MAX as u64 {
                    return Err(Malformed);
                }
                v
            }
            27 => {
                let v = u64::from_be_bytes(self.take(8)?.try_into().map_err(|_| Malformed)?);
                if v <= u32::MAX as u64 {
                    return Err(Malformed);
                }
                v
            }
            _ => return Err(Malformed),
        };
        Ok((major, value))
    }

    fn expect_major(&mut self, major: u8) -> Result<u64, Malformed> {
        let (m, v) = self.head()?;
        if m != major {
            return Err(Malformed);
        }
        Ok(v)
    }

    pub fn uint(&mut self) -> Result<u64, Malformed> {
        self.expect_major(MAJOR_UINT)
    }

    pub fn u32(&mut self) -> Result<u32, Malformed> {
        u32::try_from(self.uint()?).map_err(|_| Malformed)
    }

    pub fn u8(&mut self) -> Result<u8, Malformed> {
        u8::try_from(self.uint()?).map_err(|_| Malformed)
    }

    pub fn bytes(&mut self) -> Result<&'a [u8], Malformed> {
        let len = self.expect_major(MAJOR_BYTES)?;
        self.take(usize::try_from(len).map_err(|_| Malformed)?)
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N], Malformed> {
        self.bytes()?.try_into().map_err(|_| Malformed)
    }

    pub fn text(&mut self) -> Result<&'a str, Malformed> {
        let len = self.expect_major(MAJOR_TEXT)?;
        let raw = self.take(usize::try_from(len).map_err(|_| Malformed)?)?;
        std::str::from_utf8(raw).map_err(|_| Malformed)
    }

    /// The number of pairs of the map that starts here.
    pub fn map(&mut self) -> Result<usize, Malformed> {
        usize::try_from(self.expect_major(MAJOR_MAP)?).map_err(|_| Malformed)
    }

    /// `Ok(true)` when the next item is null (consumed), `Ok(false)` when
    /// it is a map (its length is left for `map`).
    pub fn null_or_map(&mut self) -> Result<bool, Malformed> {
        match self.buf.get(self.pos) {
            Some(&NULL) => {
                self.pos += 1;
                Ok(true)
            }
            Some(b) if b >> 5 == MAJOR_MAP => Ok(false),
            _ => Err(Malformed),
        }
    }

    /// Nothing may follow the item.
    pub fn end(&self) -> Result<(), Malformed> {
        if self.pos == self.buf.len() {
            Ok(())
        } else {
            Err(Malformed)
        }
    }
}

/// A map read field by field: the keys come in the order they were
/// written, each exactly once.
pub struct MapReader<'d, 'a> {
    dec: &'d mut Decoder<'a>,
    left: usize,
}

impl<'d, 'a> MapReader<'d, 'a> {
    pub fn open(dec: &'d mut Decoder<'a>, len: usize) -> Result<Self, Malformed> {
        let have = dec.map()?;
        if have != len {
            return Err(Malformed);
        }
        Ok(Self { dec, left: len })
    }

    /// A map of `min` to `max` pairs: the last keys are there or not, and
    /// a key that is asked for and is not there is malformed.
    pub fn open_between(dec: &'d mut Decoder<'a>, min: usize, max: usize) -> Result<Self, Malformed> {
        let have = dec.map()?;
        if have < min || have > max {
            return Err(Malformed);
        }
        Ok(Self { dec, left: have })
    }

    /// The next key must be `key`; the value is left for the caller.
    pub fn key(&mut self, key: &str) -> Result<&mut Decoder<'a>, Malformed> {
        if self.left == 0 {
            return Err(Malformed);
        }
        self.left -= 1;
        if self.dec.text()? != key {
            return Err(Malformed);
        }
        Ok(self.dec)
    }

    pub fn close(self) -> Result<(), Malformed> {
        if self.left == 0 {
            Ok(())
        } else {
            Err(Malformed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(f: impl FnOnce(&mut Encoder)) -> Vec<u8> {
        let mut e = Encoder::new();
        f(&mut e);
        e.finish()
    }

    #[test]
    fn rfc_8949_appendix_a_vectors() {
        assert_eq!(
            enc(|e| {
                e.uint(0);
            }),
            [0x00]
        );
        assert_eq!(
            enc(|e| {
                e.uint(23);
            }),
            [0x17]
        );
        assert_eq!(
            enc(|e| {
                e.uint(24);
            }),
            [0x18, 0x18]
        );
        assert_eq!(
            enc(|e| {
                e.uint(100);
            }),
            [0x18, 0x64]
        );
        assert_eq!(
            enc(|e| {
                e.uint(1000);
            }),
            [0x19, 0x03, 0xe8]
        );
        assert_eq!(
            enc(|e| {
                e.uint(1000000);
            }),
            [0x1a, 0x00, 0x0f, 0x42, 0x40]
        );
        assert_eq!(
            enc(|e| {
                e.uint(1000000000000);
            }),
            [0x1b, 0x00, 0x00, 0x00, 0xe8, 0xd4, 0xa5, 0x10, 0x00]
        );
        assert_eq!(
            enc(|e| {
                e.bytes(&[1, 2, 3, 4]);
            }),
            [0x44, 1, 2, 3, 4]
        );
        assert_eq!(
            enc(|e| {
                e.text("IETF");
            }),
            [0x64, 0x49, 0x45, 0x54, 0x46]
        );
        assert_eq!(
            enc(|e| {
                e.null();
            }),
            [0xf6]
        );
        // {"a": 1, "b": [2, 3]} is an array; the map head alone: {} = 0xa0,
        // {1: 2, 3: 4} = a2 01 02 03 04 with uint keys.
        assert_eq!(
            enc(|e| {
                e.map(2).uint(1).uint(2).uint(3).uint(4);
            }),
            [0xa2, 1, 2, 3, 4]
        );
    }

    #[test]
    fn roundtrip_map() {
        let bytes = enc(|e| {
            e.map(3)
                .text("v")
                .uint(1)
                .text("id")
                .bytes(&[9u8; 32])
                .text("name")
                .text("vault");
        });
        let mut d = Decoder::new(&bytes);
        let mut m = MapReader::open(&mut d, 3).unwrap();
        assert_eq!(m.key("v").unwrap().u8().unwrap(), 1);
        assert_eq!(m.key("id").unwrap().array::<32>().unwrap(), [9u8; 32]);
        assert_eq!(m.key("name").unwrap().text().unwrap(), "vault");
        m.close().unwrap();
        d.end().unwrap();
    }

    #[test]
    fn strict_reader() {
        // A longer-than-needed encoding of 1 is refused.
        assert_eq!(Decoder::new(&[0x18, 0x01]).uint(), Err(Malformed));
        assert_eq!(Decoder::new(&[0x19, 0x00, 0x01]).uint(), Err(Malformed));
        // Wrong major type.
        assert_eq!(Decoder::new(&[0x41, 0x00]).uint(), Err(Malformed));
        // Truncated.
        assert_eq!(Decoder::new(&[0x44, 1, 2]).bytes(), Err(Malformed));
        assert_eq!(Decoder::new(&[]).uint(), Err(Malformed));
        // Indefinite length.
        assert_eq!(Decoder::new(&[0x5f]).bytes(), Err(Malformed));
        // Bad UTF-8.
        assert_eq!(Decoder::new(&[0x61, 0xff]).text(), Err(Malformed));
        // Wrong key or wrong map length.
        let bytes = enc(|e| {
            e.map(1).text("a").uint(1);
        });
        let mut d = Decoder::new(&bytes);
        assert!(MapReader::open(&mut d, 2).is_err());
        let mut d = Decoder::new(&bytes);
        let mut m = MapReader::open(&mut d, 1).unwrap();
        assert!(m.key("b").is_err());
        // Trailing bytes.
        let mut d = Decoder::new(&[0x01, 0x02]);
        d.uint().unwrap();
        assert_eq!(d.end(), Err(Malformed));
        // u8 and u32 ranges.
        assert_eq!(Decoder::new(&[0x19, 0x01, 0x00]).u8(), Err(Malformed));
        assert_eq!(
            Decoder::new(&[0x1b, 1, 0, 0, 0, 0, 0, 0, 0]).u32(),
            Err(Malformed)
        );
    }

    #[test]
    fn null_or_map() {
        let mut d = Decoder::new(&[0xf6]);
        assert_eq!(d.null_or_map(), Ok(true));
        let mut d = Decoder::new(&[0xa0]);
        assert_eq!(d.null_or_map(), Ok(false));
        assert_eq!(d.map(), Ok(0));
        let mut d = Decoder::new(&[0x01]);
        assert_eq!(d.null_or_map(), Err(Malformed));
    }
}
