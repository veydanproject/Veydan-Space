// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A WebM that says its video has alpha though it carries none.
//!
//! Chrome's MediaRecorder (the WebView of Android) marks the video track
//! of a circle `AlphaMode = 1` and writes no alpha plane. GStreamer, the
//! player of WebKitGTK, then decodes it through `alphacombine`, which stops
//! at once ("Cannot handle streams without an initial alpha buffer"): on
//! Linux the circle stayed black and did not play. A camera has nothing
//! transparent, so the mark is put to 0 in place; the size of the file and
//! everything else in it stay as they are.
//!
//! Only the known path is walked, Segment > Tracks > TrackEntry > Video >
//! AlphaMode, without recursion: a file of any nesting cannot run the
//! stack out. The clusters are read too, and a block with additions (the
//! alpha of a transparent WebM, BlockAddID 1) keeps the mark; so does a
//! file that is not read through to its end.
//!
//! Shared by chats and the channels of communities: the bytes of a file
//! read for a page (`clear_alpha_mode`), and a file the server of files
//! of the app gives out as it is on the disk (`alpha_marks_of_open`),
//! whose marks that server puts to 0 in what it sends.

use std::cell::RefCell;
use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;

const SEGMENT: u32 = 0x1853_8067;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const VIDEO: u32 = 0xE0;
const ALPHA_MODE: u32 = 0x53C0;
const CLUSTER: u32 = 0x1F43_B675;
const BLOCK_GROUP: u32 = 0xA0;
const BLOCK_ADDITIONS: u32 = 0x75A1;
/// What a cluster holds, with the global Void and CRC-32 that may stand anywhere.
const IN_CLUSTER: [u32; 9] = [0xE7, 0x58D7, 0xA7, 0xAB, 0xA3, BLOCK_GROUP, 0xAF, 0xEC, 0xBF];
/// What may follow a cluster of an unknown size in its segment, so ends it;
/// any other id is a doubt.
const IN_SEGMENT: [u32; 8] = [0x114D_9B74, 0x1549_A966, TRACKS, CLUSTER, 0x1C53_BB6B, 0x1941_A469, 0x1043_A770, 0x1254_C367];
/// The piece of a file read at once: the heads of the blocks of a cluster
/// lie a few kilobytes apart, so most of them are in the piece read last.
const WINDOW: usize = 64 * 1024;

/// Puts the `AlphaMode` of the video tracks to 0 when no block carries
/// alpha. True when one was set.
pub fn clear_alpha_mode(bytes: &mut [u8]) -> bool {
    let Some(marks) = alpha_marks(&Slice(bytes)) else { return false };
    for mark in &marks {
        bytes[mark.start as usize..mark.end as usize].fill(0);
    }
    !marks.is_empty()
}

/// Where in the file at `path` the set `AlphaMode`s lie, to be sent as 0:
/// none when it has alpha, is not read through, or cannot be read. The
/// file is read a piece at a time and not changed.
pub fn alpha_marks_of_file(path: &std::path::Path) -> Vec<Range<u64>> {
    let Ok(file) = std::fs::File::open(path) else { return Vec::new() };
    let Ok(len) = file.metadata().map(|m| m.len()) else { return Vec::new() };
    alpha_marks_of_open(&file, len)
}

/// The same of a file already open, of `len` bytes: the marks of the very
/// file read through this handle, whatever lies under its path by now.
/// Its offset is moved.
pub fn alpha_marks_of_open(file: &std::fs::File, len: u64) -> Vec<Range<u64>> {
    alpha_marks(&Reader::new(file, len)).unwrap_or_default()
}

/// Bytes read where the walk asks: the whole file in memory or a file
/// read a piece at a time. `None` past the end or on an error.
trait Source {
    fn len(&self) -> u64;
    fn read(&self, at: u64, out: &mut [u8]) -> Option<()>;
}

struct Slice<'a>(&'a [u8]);

impl Source for Slice<'_> {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn read(&self, at: u64, out: &mut [u8]) -> Option<()> {
        let at = usize::try_from(at).ok()?;
        out.copy_from_slice(self.0.get(at..at.checked_add(out.len())?)?);
        Some(())
    }
}

struct Reader<R> {
    len: u64,
    inner: RefCell<(R, u64, Vec<u8>)>,
}

impl<R: Read + Seek> Reader<R> {
    fn new(r: R, len: u64) -> Self {
        Reader { len, inner: RefCell::new((r, 0, Vec::new())) }
    }
}

impl<R: Read + Seek> Source for Reader<R> {
    fn len(&self) -> u64 {
        self.len
    }

    fn read(&self, at: u64, out: &mut [u8]) -> Option<()> {
        let end = at.checked_add(out.len() as u64).filter(|&e| e <= self.len)?;
        let mut inner = self.inner.borrow_mut();
        let (r, from, piece) = &mut *inner;
        if at < *from || end > *from + piece.len() as u64 {
            let n = (self.len - at).min(WINDOW.max(out.len()) as u64) as usize;
            piece.resize(n, 0);
            r.seek(SeekFrom::Start(at)).ok()?;
            if r.read_exact(piece).is_err() {
                piece.clear();
                return None;
            }
            *from = at;
        }
        let i = (at - *from) as usize;
        out.copy_from_slice(&piece[i..i + out.len()]);
        Some(())
    }
}

/// The bodies of the set `AlphaMode`s; none when the file has alpha or is not read through.
fn alpha_marks(bytes: &impl Source) -> Option<Vec<Range<u64>>> {
    let mut at = 0;
    let segment = loop {
        let e = element(bytes, at, bytes.len())?;
        if e.id == SEGMENT {
            break e;
        }
        at = e.end?;
    };
    // Chrome writes the segment of an unknown size: it runs to the end.
    let end = segment.end.unwrap_or(bytes.len());
    let mut marks = Vec::new();
    let mut at = segment.body;
    while at < end {
        let e = element(bytes, at, end)?;
        at = match (e.id, e.end) {
            (TRACKS, Some(tracks_end)) => {
                tracks(bytes, e.body, tracks_end, &mut marks)?;
                tracks_end
            }
            (CLUSTER, cluster_end) => cluster(bytes, e.body, cluster_end.unwrap_or(end), cluster_end.is_none())?,
            (_, e_end) => e_end?,
        };
    }
    Some(marks)
}

fn tracks(bytes: &impl Source, body: u64, end: u64, marks: &mut Vec<Range<u64>>) -> Option<()> {
    children(bytes, body, end, |entry| {
        if entry.id != TRACK_ENTRY {
            return Some(());
        }
        children(bytes, entry.body, entry.end?, |video| {
            if video.id != VIDEO {
                return Some(());
            }
            children(bytes, video.body, video.end?, |mode| {
                let body = mode.body..mode.end?;
                if mode.id == ALPHA_MODE {
                    // An unsigned integer of at most 8 bytes.
                    let mut v = vec![0; usize::try_from(body.end - body.start).ok().filter(|&n| n <= 8)?];
                    bytes.read(body.start, &mut v)?;
                    if v.iter().any(|&b| b != 0) {
                        marks.push(body);
                    }
                }
                Some(())
            })
        })
    })
}

/// Where a cluster ends; none when a block of it carries additions.
fn cluster(bytes: &impl Source, mut at: u64, end: u64, unknown: bool) -> Option<u64> {
    while at < end {
        let e = element(bytes, at, end)?;
        if unknown && !IN_CLUSTER.contains(&e.id) {
            return IN_SEGMENT.contains(&e.id).then_some(at);
        }
        let e_end = e.end?;
        if e.id == BLOCK_GROUP {
            children(bytes, e.body, e_end, |c| (c.id != BLOCK_ADDITIONS).then_some(()))?;
        }
        at = e_end;
    }
    Some(end)
}

/// Every child of a body, each of a known size, the body through.
fn children<S: Source>(bytes: &S, mut at: u64, end: u64, mut each: impl FnMut(&Element) -> Option<()>) -> Option<()> {
    while at < end {
        let e = element(bytes, at, end)?;
        each(&e)?;
        at = e.end?;
    }
    Some(())
}

struct Element {
    /// With its marker bit, as the ids above.
    id: u32,
    body: u64,
    /// None for an unknown size (all ones).
    end: Option<u64>,
}

/// The element at `at`, which has to end by `end`.
fn element(bytes: &impl Source, at: u64, end: u64) -> Option<Element> {
    let first = byte(bytes, at, end)?;
    let id_len = first.leading_zeros() as u64 + 1;
    if id_len > 4 || at + id_len > end {
        return None;
    }
    let mut id = [0; 4];
    bytes.read(at, &mut id[..id_len as usize])?;
    let id = id[..id_len as usize].iter().fold(0, |v, &b| v << 8 | u32::from(b));
    let (size, size_len) = vint(bytes, at + id_len, end)?;
    let body = at + id_len + size_len;
    if size == (1u64 << (7 * size_len)) - 1 {
        return Some(Element { id, body, end: None });
    }
    let e_end = body.checked_add(size).filter(|&e| e <= end)?;
    Some(Element { id, body, end: Some(e_end) })
}

/// An EBML variable-size integer at `at`, without its marker bit, and its length.
fn vint(bytes: &impl Source, at: u64, end: u64) -> Option<(u64, u64)> {
    let first = byte(bytes, at, end)?;
    let len = first.leading_zeros() as u64 + 1;
    if len > 8 || at + len > end {
        return None;
    }
    let mut all = [0; 8];
    bytes.read(at, &mut all[..len as usize])?;
    let mut v = u64::from(first) & (0xFF >> len);
    for &b in &all[1..len as usize] {
        v = (v << 8) | u64::from(b);
    }
    Some((v, len))
}

fn byte(bytes: &impl Source, at: u64, end: u64) -> Option<u8> {
    let mut b = [0];
    (at < end).then_some(())?;
    bytes.read(at, &mut b)?;
    Some(b[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The head of a circle Android's WebView recorded, up to its first
    /// cluster: the segment of an unknown size, an Opus track, then the
    /// VP8 track with `AlphaMode = 1` (`53 C0 81 01`) at its very end.
    const CHROME_HEAD: &str = "1a45dfa39f4286810142f7810142f2810442f381084282847765626d42878104428581021853806701ffffffffffffff\
        1549a966992ad7b1830f42404d80864368726f6d655741864368726f6d651654ae6beaaebdd7810173c587abe892ff80d7cb8381028686415f4f505553\
        63a2934f707573486561640101000080bb0000000000e18db584473b80009f810162648120aea9d7810273c587169f141982783a83810155ee8101868556\
        5f565038e08cb08201e0ba8201e053c08101";

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// The head and a cluster whose frame happens to hold the bytes of an `AlphaMode = 1`.
    fn chrome_file() -> Vec<u8> {
        let mut f = hex(CHROME_HEAD);
        f.extend(hex("1f43b67501ffffffffffffffe78100a38b53c0810181000053c08101"));
        f
    }

    #[test]
    fn the_alpha_mode_of_chrome_is_put_to_zero_and_nothing_else_changes() {
        let before = chrome_file();
        let mut after = before.clone();
        assert!(clear_alpha_mode(&mut after));
        let head = hex(CHROME_HEAD).len();
        assert_eq!(after.len(), before.len());
        assert_eq!(&after[head - 4..head], &[0x53, 0xC0, 0x81, 0x00]);
        assert_eq!(after[..head - 1], before[..head - 1]);
        assert_eq!(after[head..], before[head..], "a frame is not read as a header");
        assert!(!clear_alpha_mode(&mut after), "once is enough");
    }

    #[test]
    fn a_segment_of_a_known_size_is_walked_as_well() {
        // Segment (size 17) > Tracks (12) > TrackEntry (10) > Video (8) > PixelWidth 480, AlphaMode 1.
        let mut f = hex("18538067911654ae6b8cae8ae088b08201e053c08101");
        assert!(clear_alpha_mode(&mut f));
        assert_eq!(f[f.len() - 1], 0);
    }

    /// An element of a known size, written in 8 bytes.
    fn el(id: u32, body: &[u8]) -> Vec<u8> {
        let mut v = id.to_be_bytes().into_iter().skip_while(|&b| b == 0).collect::<Vec<_>>();
        v.push(0x01);
        v.extend(&(body.len() as u64).to_be_bytes()[1..]);
        v.extend(body);
        v
    }

    /// An element of an unknown size.
    fn open(id: u32, body: &[u8]) -> Vec<u8> {
        let mut v = el(id, &[]);
        v.truncate(v.len() - 8);
        v.extend(hex("01ffffffffffffff"));
        v.extend(body);
        v
    }

    /// A block group whose block carries alpha: BlockAdditions > BlockMore > BlockAddID 1, BlockAdditional.
    fn alpha_block() -> Vec<u8> {
        let more = el(0xA6, &[hex("ee8101"), el(0xA5, b"alpha")].concat());
        el(BLOCK_GROUP, &[el(0xA1, &hex("8100000080")), el(BLOCK_ADDITIONS, &more)].concat())
    }

    fn unchanged(mut f: Vec<u8>) {
        let before = f.clone();
        assert!(!clear_alpha_mode(&mut f));
        assert_eq!(f, before);
    }

    #[test]
    fn a_circle_of_several_clusters_is_read_through() {
        let mut f = chrome_file();
        f.extend(open(CLUSTER, &[hex("bf8400000000e78164ec80"), el(0xA3, &hex("8100000080")), el(BLOCK_GROUP, &el(0xA1, &hex("81000080")))].concat()));
        f.extend(el(0xEC, &[0; 4]));
        f.extend(el(CLUSTER, &hex("e781c8a38481000080")));
        assert!(clear_alpha_mode(&mut f));
    }

    #[test]
    fn a_webm_with_alpha_keeps_its_mark() {
        let mut first = chrome_file();
        first.extend(alpha_block());
        unchanged(first);
        // Not in the first cluster but in a later one.
        let mut later = chrome_file();
        later.extend(open(CLUSTER, &[hex("e78164"), alpha_block()].concat()));
        unchanged(later);
        // An unknown size of the group itself is a doubt as well.
        let mut group = chrome_file();
        group.extend(open(BLOCK_GROUP, &el(0xA1, &hex("81000080"))));
        unchanged(group);
        // A Void or a CRC-32 within a cluster of an unknown size does not end it.
        let mut void = chrome_file();
        void.extend(hex("ec80"));
        void.extend(alpha_block());
        unchanged(void);
        let mut crc = chrome_file();
        crc.extend(open(CLUSTER, &[hex("bf8400000000"), hex("e78164"), alpha_block()].concat()));
        unchanged(crc);
        // Nor does an id that has no place there: it is a doubt.
        let mut stranger = chrome_file();
        stranger.extend(hex("86815a"));
        stranger.extend(alpha_block());
        unchanged(stranger);
        let mut alone = chrome_file();
        alone.extend(hex("86815a"));
        unchanged(alone);
    }

    #[test]
    fn a_nesting_of_any_depth_does_not_run_the_stack_out() {
        let deep = hex("e0ff").repeat(100_000);
        unchanged(deep.clone());
        let video = [deep.clone(), hex("53c08101")].concat();
        unchanged(open(SEGMENT, &el(TRACKS, &el(TRACK_ENTRY, &el(VIDEO, &video)))));
        let mut after = chrome_file();
        after.extend(&deep);
        unchanged(after);
    }

    #[test]
    fn a_broken_or_unknown_sized_file_keeps_its_mark() {
        let video = hex("b08201e053c08101");
        let tracks = |entry: Vec<u8>| open(SEGMENT, &el(TRACKS, &entry));
        assert!(clear_alpha_mode(&mut tracks(el(TRACK_ENTRY, &el(VIDEO, &video)))), "the same file whole");
        unchanged(tracks(open(TRACK_ENTRY, &el(VIDEO, &video))));
        unchanged(tracks(el(TRACK_ENTRY, &open(VIDEO, &video))));
        unchanged(open(SEGMENT, &open(TRACKS, &el(TRACK_ENTRY, &el(VIDEO, &video)))));
        // A size past its parent, cut short, or a stray byte after the frame.
        let mut long = tracks(el(TRACK_ENTRY, &el(VIDEO, &video)));
        let at = long.len() - video.len() - 1;
        long[at] += 1;
        unchanged(long);
        let mut cut = chrome_file();
        cut.truncate(cut.len() - 1);
        unchanged(cut);
        let mut stray = chrome_file();
        stray.push(0x81);
        unchanged(stray);
    }

    /// A file on the disk, larger than a piece read at once, gives the
    /// marks its bytes give, and is not changed.
    #[test]
    fn a_file_is_read_a_piece_at_a_time() {
        let frame = el(0xA3, &[hex("81000080"), vec![7; 3000]].concat());
        let mut f = chrome_file();
        for _ in 0..3 {
            f.extend(open(CLUSTER, &[hex("e78164"), frame.repeat(40)].concat()));
        }
        assert!(f.len() > 3 * WINDOW);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.webm");
        std::fs::write(&path, &f).unwrap();
        let head = hex(CHROME_HEAD).len() as u64;
        assert_eq!(alpha_marks_of_file(&path), vec![head - 1..head]);
        assert_eq!(std::fs::read(&path).unwrap(), f, "the file stays");
        let mut cleared = f.clone();
        assert!(clear_alpha_mode(&mut cleared));
        assert_eq!(cleared[head as usize - 1], 0);

        // Alpha in the last cluster, past every piece read before.
        f.extend(alpha_block());
        std::fs::write(&path, &f).unwrap();
        assert_eq!(alpha_marks_of_file(&path), Vec::<Range<u64>>::new());
        // Cut short within a frame.
        f.truncate(f.len() - alpha_block().len() - 10);
        std::fs::write(&path, &f).unwrap();
        assert!(alpha_marks_of_file(&path).is_empty());
        assert!(alpha_marks_of_file(&dir.path().join("none")).is_empty());
    }

    #[test]
    fn what_is_not_a_webm_is_left_alone() {
        for mut f in [Vec::new(), b"OggS\0\x02".to_vec(), vec![0; 16], hex("1853806701ff"), hex("18538067ff1654ae6b")] {
            let before = f.clone();
            assert!(!clear_alpha_mode(&mut f));
            assert_eq!(f, before);
        }
    }
}
