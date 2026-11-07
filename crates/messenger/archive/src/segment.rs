// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The file of a segment, version 1 (`internal/messenger-wire.md` §8):
//!
//! ```text
//! header:     "VMA1" | version u16 LE | owner 32 bytes | scope_len u16 LE | scope | month "yyyy-mm"
//! record ×N:  len u32 LE | json(ArchivedEvent) of len bytes
//! ```
//!
//! The owner is the public key of the identity whose archive it is: a
//! reader takes no file of another's. Records go in the order they were
//! appended, not by time: the file only grows, so a month that grew changes
//! the last chunk of its large file and the engine deduplicates the rest. A
//! reader sorts by `(at, id)` itself. There is no compression inside: the
//! envelope of the engine compresses the chunk. A record whose `len` runs
//! past the end is where a file broke off: what came before it is read, and
//! the read says it was truncated. Any other version in the header is
//! refused ([`crate::format_error`]).

use crate::{
    format_error, too_large, ArchiveScope, ArchivedEvent, Month, FORMAT_VERSION, MAX_RECORDS, MAX_RECORD_BYTES, MAX_SEGMENT_BYTES,
    SEGMENT_MAGIC,
};
use messenger_core::{PubKey, Result};
use sha2::{Digest, Sha256};

/// The length of the month in the header: `yyyy-mm`.
const MONTH_LEN: usize = 7;
/// The owner's key in the header.
const OWNER_LEN: usize = 32;
/// Magic, version, owner, scope length.
const FIXED_LEN: usize = 4 + 2 + OWNER_LEN + 2;

/// A segment file, read.
#[derive(Clone, Debug)]
pub struct Read {
    pub owner: PubKey,
    pub scope: ArchiveScope,
    pub month: Month,
    /// In the order of the file.
    pub events: Vec<ArchivedEvent>,
    /// The file broke off inside a record: `events` is what came before.
    pub truncated: bool,
}

/// A segment file split into its records, not parsed: the archive keeps a
/// record as the file has it, so a rewrite by any device repeats its bytes.
#[derive(Clone, Debug)]
pub struct RawRead<'a> {
    pub owner: PubKey,
    pub scope: ArchiveScope,
    pub month: Month,
    pub records: Vec<&'a [u8]>,
    pub truncated: bool,
}

/// sha256 of the plaintext file, hex: `SegmentMeta.hash`.
pub fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// The header of the file of `owner`'s `scope` and `month`.
pub fn header(owner: &PubKey, scope: &ArchiveScope, month: Month) -> Result<Vec<u8>> {
    let key = scope.as_key();
    let scope_len = u16::try_from(key.len()).map_err(|_| format_error("scope too long"))?;
    let month = month.to_string();
    if month.len() != MONTH_LEN {
        return Err(format_error(format!("month {month} out of range")));
    }
    let owner = hex::decode(owner.as_hex()).map_err(|_| format_error("owner"))?;
    if owner.len() != OWNER_LEN {
        return Err(format_error("owner"));
    }
    let mut out = Vec::with_capacity(FIXED_LEN + key.len() + MONTH_LEN);
    out.extend_from_slice(SEGMENT_MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&owner);
    out.extend_from_slice(&scope_len.to_le_bytes());
    out.extend_from_slice(key.as_bytes());
    out.extend_from_slice(month.as_bytes());
    Ok(out)
}

/// The bytes of the header of a file of `scope`: a file is this and its
/// records, each with its length (4 bytes) before it.
pub fn header_len(scope: &ArchiveScope) -> u64 {
    (FIXED_LEN + scope.as_key().len() + MONTH_LEN) as u64
}

/// A file whole over [`MAX_SEGMENT_BYTES`] is not made: neither this
/// device nor another would read it.
fn bounded(out: Vec<u8>) -> Result<Vec<u8>> {
    if out.len() as u64 > MAX_SEGMENT_BYTES {
        return Err(too_large(format!("a file of {} bytes", out.len())));
    }
    Ok(out)
}

/// Append one record to a file.
pub fn push_record(out: &mut Vec<u8>, json: &[u8]) -> Result<()> {
    if json.len() > MAX_RECORD_BYTES {
        return Err(too_large(format!("a record of {} bytes", json.len())));
    }
    let len = u32::try_from(json.len()).map_err(|_| format_error("record too long"))?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(json);
    Ok(())
}

/// The file of records already serialized, in the given order.
pub fn write_records<'a>(
    owner: &PubKey,
    scope: &ArchiveScope,
    month: Month,
    records: impl IntoIterator<Item = &'a [u8]>,
) -> Result<Vec<u8>> {
    let mut out = header(owner, scope, month)?;
    for json in records {
        push_record(&mut out, json)?;
    }
    bounded(out)
}

/// The file of `events`, in the given order.
pub fn write<'a>(
    owner: &PubKey,
    scope: &ArchiveScope,
    month: Month,
    events: impl IntoIterator<Item = &'a ArchivedEvent>,
) -> Result<Vec<u8>> {
    let mut out = header(owner, scope, month)?;
    for event in events {
        push_record(&mut out, &serde_json::to_vec(event)?)?;
    }
    bounded(out)
}

/// Split a file into its records. A wrong magic, version, owner, scope or
/// month is an error; a record cut short ends the read, `truncated`. A file,
/// a record or a count of records over the bounds of the format
/// ([`MAX_SEGMENT_BYTES`], [`MAX_RECORD_BYTES`], [`MAX_RECORDS`]) is
/// refused ([`too_large`]) before any record is kept.
pub fn read_raw(bytes: &[u8]) -> Result<RawRead<'_>> {
    if bytes.len() as u64 > MAX_SEGMENT_BYTES {
        return Err(too_large(format!("a file of {} bytes", bytes.len())));
    }
    if bytes.len() < FIXED_LEN || &bytes[..4] != SEGMENT_MAGIC {
        return Err(format_error("not a segment file"));
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != FORMAT_VERSION {
        return Err(format_error(format!("version {version}, this build reads {FORMAT_VERSION}")));
    }
    let owner = PubKey::parse(&hex::encode(&bytes[6..6 + OWNER_LEN])).ok_or_else(|| format_error("owner"))?;
    let scope_len = usize::from(u16::from_le_bytes([bytes[FIXED_LEN - 2], bytes[FIXED_LEN - 1]]));
    let body = FIXED_LEN + scope_len + MONTH_LEN;
    if bytes.len() < body {
        return Err(format_error("short header"));
    }
    let scope = std::str::from_utf8(&bytes[FIXED_LEN..FIXED_LEN + scope_len])
        .ok()
        .and_then(ArchiveScope::parse)
        .ok_or_else(|| format_error("scope"))?;
    let month = std::str::from_utf8(&bytes[FIXED_LEN + scope_len..body])
        .ok()
        .and_then(Month::parse)
        .ok_or_else(|| format_error("month"))?;

    let mut records = Vec::new();
    let mut pos = body;
    let mut truncated = false;
    while pos < bytes.len() {
        let Some(len) = bytes.get(pos..pos + 4) else {
            truncated = true;
            break;
        };
        let len = u32::from_le_bytes([len[0], len[1], len[2], len[3]]) as usize;
        if len > MAX_RECORD_BYTES {
            return Err(too_large(format!("a record of {len} bytes")));
        }
        if records.len() == MAX_RECORDS {
            return Err(too_large(format!("over {MAX_RECORDS} records")));
        }
        let start = pos + 4;
        let Some(record) = start.checked_add(len).and_then(|end| bytes.get(start..end)) else {
            truncated = true;
            break;
        };
        records.push(record);
        pos = start + len;
    }
    if truncated {
        tracing::warn!(scope = %scope, month = %month, kept = records.len(), "archive segment is cut short; reading what came before");
    }
    Ok(RawRead { owner, scope, month, records, truncated })
}

/// Read a file. A record that is not an [`ArchivedEvent`] is an error:
/// a file this build does not understand is not merged into, lest a
/// rewrite lose what it holds.
pub fn read(bytes: &[u8]) -> Result<Read> {
    let raw = read_raw(bytes)?;
    let events = raw
        .records
        .iter()
        .map(|r| serde_json::from_slice::<ArchivedEvent>(r).map_err(|e| format_error(format!("record: {e}"))))
        .collect::<Result<Vec<_>>>()?;
    Ok(Read { owner: raw.owner, scope: raw.scope, month: raw.month, events, truncated: raw.truncated })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{is_format_error, Body};
    use messenger_core::{DmInbound, EventId, EventSource, RawEvent, Timestamp};

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const D: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

    fn owner() -> PubKey {
        PubKey::parse(D).unwrap()
    }

    fn dm() -> ArchivedEvent {
        let inbound = DmInbound {
            envelope: messenger_core::inbound::Envelope {
                wire_id: EventId::parse(A).unwrap(),
                source: EventSource::Sync { url: messenger_core::RelayUrl::parse("wss://r.example").unwrap() },
                wire_created_at: Timestamp(1_793_700_000),
            },
            rumor_id: EventId::parse(B).unwrap(),
            sender: PubKey::parse(C).unwrap(),
            recipients: vec![PubKey::parse(D).unwrap()],
            created_at: Timestamp(1_793_793_600),
            content: r#"{"v":1,"t":"text","text":"hi"}"#.into(),
            reply_to: None,
            rumor_kind: 14,
            srv: None,
        };
        ArchivedEvent::dm(inbound, Timestamp(1_793_793_602))
    }

    fn group() -> ArchivedEvent {
        // Keys in order, so that the bytes do not depend on how serde_json
        // orders a map.
        let json = serde_json::from_str(&format!(
            r#"{{"content":"sealed","created_at":1793793700,"id":"{D}","kind":9,"pubkey":"{C}","sig":"00","tags":[["h","g1"]]}}"#
        ))
        .unwrap();
        let raw = RawEvent {
            id: EventId::parse(D).unwrap(),
            kind: 9,
            pubkey: PubKey::parse(C).unwrap(),
            created_at: Timestamp(1_793_793_700),
            json,
            source: EventSource::Archive,
        };
        ArchivedEvent::wire(raw, ArchiveScope::Group { id: "g1".into() }, Timestamp(1_793_793_701))
    }

    /// The frozen reference of version 1: these bytes are what every build
    /// writes and reads. Changing them is a new version of the format.
    #[test]
    fn version_1_bytes_are_frozen() {
        let nov = Month { year: 2026, month: 11 };
        let bytes = write(&owner(), &ArchiveScope::Dm, nov, [&dm()]).unwrap();
        let record = format!(
            concat!(
                r#"{{"id":"{a}","kind":1059,"at":1793793600,"received_at":1793793602,"scope":{{"scope":"dm"}},"#,
                r#""body":{{"form":"dm","inbound":{{"envelope":{{"wire_id":"{a}","source":{{"kind":"archive"}},"wire_created_at":1793700000}},"#,
                r#""rumor_id":"{b}","sender":"{c}","recipients":["{d}"],"created_at":1793793600,"#,
                r#""content":"{{\"v\":1,\"t\":\"text\",\"text\":\"hi\"}}","reply_to":null,"rumor_kind":14}}}}}}"#
            ),
            a = A,
            b = B,
            c = C,
            d = D
        );
        let mut want: Vec<u8> = b"VMA1\x01\x00".to_vec();
        want.extend_from_slice(&[0xdd; 32]); // the owner, D
        want.extend_from_slice(b"\x02\x00dm2026-11");
        want.extend_from_slice(&[0xa2, 0x02, 0x00, 0x00]); // len 674
        want.extend_from_slice(record.as_bytes());
        assert_eq!(record.len(), 674);
        assert_eq!(bytes, want);
        assert_eq!(hash(&bytes).len(), 64);
        assert_eq!(hash(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");

        let back = read(&bytes).unwrap();
        assert_eq!((back.owner, back.scope, back.month, back.truncated), (owner(), ArchiveScope::Dm, nov, false));
        let Body::Dm { inbound } = &back.events[0].body else { panic!("a dm body") };
        assert_eq!(inbound.envelope.source, EventSource::Archive);
        assert_eq!(inbound.rumor_id.as_hex(), B);
    }

    #[test]
    fn a_file_round_trips_in_its_order() {
        let scope = ArchiveScope::Group { id: "g1".into() };
        let nov = Month { year: 2026, month: 11 };
        let mut later = group();
        later.id = EventId::parse(A).unwrap();
        later.at = Timestamp(1);
        let bytes = write(&owner(), &scope, nov, [&group(), &later]).unwrap();
        let read = read(&bytes).unwrap();
        assert_eq!(read.scope, scope);
        assert_eq!(read.events.iter().map(|e| e.id.as_hex()).collect::<Vec<_>>(), [D, A]);
        let Body::Wire { raw } = &read.events[0].body else { panic!("a wire body") };
        assert_eq!(raw.kind, 9);
        assert_eq!(raw.json["tags"][0][1], "g1");
    }

    #[test]
    fn a_cut_file_keeps_what_came_before_and_other_versions_are_refused() {
        let nov = Month { year: 2026, month: 11 };
        let bytes = write(&owner(), &ArchiveScope::Dm, nov, [&dm(), &dm()]).unwrap();
        let cut = &bytes[..bytes.len() - 10];
        let read = read(cut).unwrap();
        assert!(read.truncated);
        assert_eq!(read.events.len(), 1);
        // A length alone, with nothing after it.
        let head = header(&owner(), &ArchiveScope::Dm, nov).unwrap();
        let mut short = head.clone();
        short.extend_from_slice(&[5, 0]);
        assert!(super::read(&short).unwrap().truncated);
        assert!(!super::read(&head).unwrap().truncated);

        let mut v2 = bytes.clone();
        v2[4] = 2;
        let refused = super::read(&v2).unwrap_err();
        assert!(is_format_error(&refused), "{refused}");
        let mut bad = head.clone();
        bad[..4].copy_from_slice(b"VMA0");
        assert!(super::read(&bad).is_err());
        assert!(super::read(&head[..head.len() - 3]).is_err(), "a short header");
        let mut chat = b"VMA1\x01\x00".to_vec();
        chat.extend_from_slice(&[0xdd; 32]);
        chat.extend_from_slice(b"\x04\x00chat2026-11");
        assert!(super::read(&chat).is_err(), "not a scope");
        let mut junk = head;
        push_record(&mut junk, b"{}").unwrap();
        let refused = super::read(&junk).unwrap_err();
        assert!(is_format_error(&refused), "a record this build does not understand: {refused}");
    }

    /// The header counts (ROO-692): records of exactly [`MAX_SEGMENT_BYTES`]
    /// with their lengths make a file over it, which is not made; a byte
    /// fewer than the header leaves room, and that file is read back.
    #[test]
    fn a_file_is_bounded_with_its_header() {
        let (scope, month) = (ArchiveScope::Dm, Month { year: 2026, month: 11 });
        let head = header_len(&scope);
        assert_eq!(head as usize, header(&owner(), &scope, month).unwrap().len());
        let record = vec![b'x'; MAX_RECORD_BYTES - 4];
        let full: Vec<&[u8]> = (0..16).map(|_| record.as_slice()).collect();
        assert_eq!(full.iter().map(|r| r.len() as u64 + 4).sum::<u64>(), MAX_SEGMENT_BYTES);
        let e = write_records(&owner(), &scope, month, full.iter().copied()).unwrap_err();
        assert!(crate::is_too_large(&e), "{e}");

        let last = vec![b'x'; MAX_RECORD_BYTES - 4 - head as usize];
        let fits: Vec<&[u8]> = (0..15).map(|_| record.as_slice()).chain([last.as_slice()]).collect();
        let bytes = write_records(&owner(), &scope, month, fits.iter().copied()).unwrap();
        assert_eq!(bytes.len() as u64, MAX_SEGMENT_BYTES);
        assert_eq!(read_raw(&bytes).unwrap().records.len(), 16);
    }

    /// Over the bounds of the format, a file is refused before a record
    /// of it is kept: too many bytes, a record too long, too many records.
    #[test]
    fn a_file_over_the_bounds_is_refused_whole() {
        let head = header(&owner(), &ArchiveScope::Dm, Month { year: 2026, month: 11 }).unwrap();
        let mut big = head.clone();
        big.resize(MAX_SEGMENT_BYTES as usize + 1, 0);
        assert!(crate::is_too_large(&read_raw(&big).unwrap_err()));

        let mut long = head.clone();
        long.extend_from_slice(&(MAX_RECORD_BYTES as u32 + 1).to_le_bytes());
        long.extend_from_slice(&[b'x'; 16]);
        assert!(crate::is_too_large(&read_raw(&long).unwrap_err()), "a length past the bound is not taken");
        let mut out = head.clone();
        assert!(crate::is_too_large(&push_record(&mut out, &vec![b'x'; MAX_RECORD_BYTES + 1]).unwrap_err()));

        let mut many = head.clone();
        for _ in 0..=MAX_RECORDS {
            push_record(&mut many, b"{}").unwrap();
        }
        assert!(crate::is_too_large(&read_raw(&many).unwrap_err()));
        many.truncate(head.len() + 6 * MAX_RECORDS);
        assert_eq!(read_raw(&many).unwrap().records.len(), MAX_RECORDS, "at the bound it is read");
    }
}
