// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A photo in a format every client of ours shows (JPEG, PNG, WebP, GIF)
//! before it is sent (platform-spec, «Общие форматы вложений»). A picture
//! of another format that the phone can read (HEIC/HEIF of iPhones, Oppo,
//! Samsung, Xiaomi) is written anew as a JPEG by the platform's own
//! decoder (`PictureConverter`) as it is picked: the composer shows it,
//! and it goes as a photo under a `.jpg` name. The other side never gets
//! a HEIC to decode, so nothing new is read on receipt.
//!
//! The format is told by the bytes (`ftyp` of ISO BMFF), never by the
//! name. The JPEG carries its pixels alone, upright: nothing of the source
//! (place, phone, time) goes with it. A picture the decoder cannot read is
//! left as it is and goes as a file, as before. `mime_for` does not make a
//! HEIC a picture: one that was not converted stays a file, or the other
//! side would get an empty bubble.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use async_trait::async_trait;
use tokio::sync::Semaphore;

/// The JPEG quality of a converted photo: it is the original the user
/// picked, made smaller later only as any photo is (`prepare`).
pub const COMMON_QUALITY: u8 = 92;
/// A converted photo has at most this many pixels; a larger one is made
/// smaller to it, as the decoder of a picked photo allows no more.
pub const MAX_COMMON_PIXELS: u64 = messenger_avatar::MAX_PIXELS;
/// The first bytes read to tell the format.
const HEAD_BYTES: usize = 64;

/// A picture format the clients do not all show, which is converted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Uncommon {
    /// HEIC/HEIF (an HEVC picture in ISO BMFF).
    Heif,
}

/// The brands of `ftyp` that say HEIF: `heic`, `heix` (HEVC pictures),
/// `mif1` (an image file of HEIF), `msf1` (an image sequence), `heif`.
const HEIF_BRANDS: [&[u8; 4]; 5] = [b"heic", b"heix", b"mif1", b"msf1", b"heif"];
/// AVIF is HEIF with another codec and names `mif1` too; it is shown as a
/// picture already and is not converted.
const AVIF_BRANDS: [&[u8; 4]; 2] = [b"avif", b"avis"];

/// The uncommon picture format of a file, from its first bytes: the
/// major brand of its `ftyp` box and the compatible ones after it.
pub fn uncommon(head: &[u8]) -> Option<Uncommon> {
    if head.len() < 16 || &head[4..8] != b"ftyp" {
        return None;
    }
    let size = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
    if size < 16 {
        return None;
    }
    let end = size.min(head.len());
    // The major brand, then (past the minor version) the compatible ones.
    let brands = std::iter::once(&head[8..12]).chain(head[16..end].as_chunks::<4>().0.iter().map(|b| &b[..]));
    let mut heif = false;
    for brand in brands {
        if AVIF_BRANDS.iter().any(|b| &b[..] == brand) {
            return None;
        }
        heif |= HEIF_BRANDS.iter().any(|b| &b[..] == brand);
    }
    heif.then_some(Uncommon::Heif)
}

/// The name of the converted photo: `IMG_1234.heic` → `IMG_1234.jpg`.
pub fn common_name(name: &str) -> String {
    let stem = match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    };
    let stem = if stem.trim().is_empty() { "photo" } else { stem };
    format!("{stem}.jpg")
}

/// The platform's decoder of pictures (Android: `ImageDecoder`); a
/// platform without one has no converter, and the picture goes as it is.
#[async_trait]
pub trait PictureConverter: Send + Sync {
    /// Writes the picture at `src` as a JPEG of `quality` at `dst`: turned
    /// upright as its camera said, at most `max_pixels` (made smaller to
    /// that), of its pixels alone. `Err`: it could not be read.
    async fn to_jpeg(&self, src: &Path, dst: &Path, max_pixels: u64, quality: u8) -> Result<(), String>;
}

/// The decoder of the platform, as the host keeps it for every module that
/// sends pictures (a chat, a channel); absent where there is none.
#[derive(Clone)]
pub struct PlatformDecoder(pub std::sync::Arc<dyn PictureConverter>);

/// What became of a picked file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Common {
    /// It goes as it is: of a common format, not a picture, or there is no
    /// converter here.
    AsIs,
    /// It was converted: the JPEG; the source is gone.
    Converted(PathBuf),
    /// Of an uncommon format the converter could not read: it goes as it
    /// is, as a file, and the user is told.
    Unsupported,
}

/// Pictures converted at once: each holds a decoded photo in memory, as a
/// photo made smaller does (`prepare::PHOTO_SLOTS`).
fn slots() -> &'static Semaphore {
    static SLOTS: OnceLock<Semaphore> = OnceLock::new();
    SLOTS.get_or_init(|| Semaphore::new(crate::prepare::PHOTO_SLOTS))
}

/// The file at `path` in a common format. Only a copy the app made of a
/// picked file is given here: the converted JPEG takes its place in the
/// same folder and the copy goes; the user's own file is never touched.
pub async fn to_common(path: &Path, converter: Option<&dyn PictureConverter>) -> Common {
    let Some(converter) = converter else { return Common::AsIs };
    if head_of(path).await.as_deref().and_then(uncommon).is_none() {
        return Common::AsIs;
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let dst = path.with_file_name(common_name(&name));
    // Written aside first: a misnamed `x.jpg` that is a HEIC is replaced only once its JPEG is whole.
    let part = path.with_file_name(format!(".{}.part", common_name(&name)));
    let made = {
        let _slot = slots().acquire().await;
        converter.to_jpeg(path, &part, MAX_COMMON_PIXELS, COMMON_QUALITY).await
    };
    let placed = match made {
        Ok(()) => tokio::fs::rename(&part, &dst).await.map_err(|e| e.to_string()),
        Err(e) => Err(e),
    };
    if placed.is_err() {
        let _ = tokio::fs::remove_file(&part).await;
        return Common::Unsupported;
    }
    if dst != path {
        let _ = tokio::fs::remove_file(path).await;
    }
    Common::Converted(dst)
}

async fn head_of(path: &Path) -> Option<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path).await.ok()?;
    let mut head = vec![0u8; HEAD_BYTES];
    let mut got = 0;
    while got < HEAD_BYTES {
        match file.read(&mut head[got..]).await {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(_) => return None,
        }
    }
    head.truncate(got);
    Some(head)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// An `ftyp` box: its major brand, a minor version, the compatible brands.
    fn ftyp(major: &[u8; 4], compatible: &[&[u8; 4]]) -> Vec<u8> {
        let size = 16 + 4 * compatible.len() as u32;
        let mut b = size.to_be_bytes().to_vec();
        b.extend_from_slice(b"ftyp");
        b.extend_from_slice(major);
        b.extend_from_slice(&[0, 0, 0, 0]);
        for c in compatible {
            b.extend_from_slice(&c[..]);
        }
        b.extend_from_slice(b"\0\0\0\x08meta");
        b
    }

    /// Each brand of HEIF is told, as the major one or a compatible one;
    /// AVIF (which names `mif1` too), video and other pictures are not.
    #[test]
    fn heif_is_told_by_its_brands() {
        for brand in HEIF_BRANDS {
            assert_eq!(uncommon(&ftyp(brand, &[])), Some(Uncommon::Heif), "major {brand:?}");
            assert_eq!(uncommon(&ftyp(b"isom", &[brand])), Some(Uncommon::Heif), "compatible {brand:?}");
        }
        // As an iPhone writes it, and as Samsung and Oppo do.
        assert_eq!(uncommon(&ftyp(b"heic", &[b"mif1", b"heic"])), Some(Uncommon::Heif));
        assert_eq!(uncommon(&ftyp(b"mif1", &[b"heic", b"miaf"])), Some(Uncommon::Heif));
        assert_eq!(uncommon(&ftyp(b"msf1", &[b"hevc"])), Some(Uncommon::Heif));

        assert_eq!(uncommon(&ftyp(b"avif", &[b"mif1", b"miaf"])), None, "AVIF");
        assert_eq!(uncommon(&ftyp(b"mif1", &[b"avif"])), None, "AVIF by a compatible brand");
        assert_eq!(uncommon(&ftyp(b"isom", &[b"mp41", b"avc1"])), None, "MP4");
        assert_eq!(uncommon(&ftyp(b"qt  ", &[b"qt  "])), None, "MOV");
        assert_eq!(uncommon(b"\xFF\xD8\xFF\xE0\0\x10JFIF\0\x01\x01\0\0\x01\0\x01"), None, "JPEG");
        assert_eq!(uncommon(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"), None, "PNG");
        assert_eq!(uncommon(b"RIFF\0\0\0\0WEBPVP8 "), None, "WebP");
        assert_eq!(uncommon(b""), None);
        assert_eq!(uncommon(b"\0\0\0\x18ftyp"), None, "cut short");
    }

    /// Only the brands within the box count: a brand past its declared
    /// size is the next box, and a size too small for a box is nothing.
    #[test]
    fn the_brands_end_with_the_box() {
        let mut b = ftyp(b"isom", &[]);
        b.truncate(16);
        b.extend_from_slice(b"heic");
        assert_eq!(uncommon(&b), None, "past the box");
        let mut small = ftyp(b"heic", &[]);
        small[3] = 8;
        assert_eq!(uncommon(&small), None, "a box of 8 bytes");
    }

    /// The name of a converted photo is the name of its source as a JPEG.
    #[test]
    fn a_converted_photo_is_named_jpg() {
        assert_eq!(common_name("IMG_1234.heic"), "IMG_1234.jpg");
        assert_eq!(common_name("IMG_1234.HEIF"), "IMG_1234.jpg");
        assert_eq!(common_name("photo.from.oppo.heif"), "photo.from.oppo.jpg");
        assert_eq!(common_name("1000012345"), "1000012345.jpg");
        assert_eq!(common_name(".heic"), ".heic.jpg");
        assert_eq!(common_name(""), "photo.jpg");
        // And the app takes it for a photo from then on.
        let name = common_name("IMG_1234.heic");
        assert_eq!(crate::descriptor::mime_for(&name), "image/jpeg");
        assert_eq!(crate::MediaKind::from_mime(crate::descriptor::mime_for(&name)), crate::MediaKind::Image);
        // A HEIC that was not converted is not taken for a picture.
        assert_eq!(crate::descriptor::mime_for("IMG_1234.heic"), "application/octet-stream");
        assert_eq!(crate::descriptor::mime_for("IMG_1234.heif"), "application/octet-stream");
    }

    /// A decoder that writes what it is told, or fails, and counts how
    /// many it runs at once.
    #[derive(Default)]
    struct Fake {
        fail: bool,
        seen: Mutex<Vec<(u64, u8)>>,
        now: AtomicUsize,
        most: AtomicUsize,
    }

    #[async_trait]
    impl PictureConverter for Fake {
        async fn to_jpeg(&self, src: &Path, dst: &Path, max_pixels: u64, quality: u8) -> Result<(), String> {
            let n = self.now.fetch_add(1, Ordering::SeqCst) + 1;
            self.most.fetch_max(n, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            self.now.fetch_sub(1, Ordering::SeqCst);
            self.seen.lock().unwrap().push((max_pixels, quality));
            assert!(src.exists());
            if self.fail {
                std::fs::write(dst, b"half").unwrap();
                return Err("not readable".into());
            }
            std::fs::write(dst, b"\xFF\xD8\xFF\xE0jpeg").map_err(|e| e.to_string())
        }
    }

    fn heic_at(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, ftyp(b"heic", &[b"mif1", b"heic"])).unwrap();
        path
    }

    /// A HEIC becomes a JPEG beside it, under its name as a JPEG, at
    /// quality 92 within the pixels of a picked photo; the copy goes.
    #[tokio::test]
    async fn a_heic_becomes_a_jpeg_in_its_place() {
        let dir = tempfile::tempdir().unwrap();
        let src = heic_at(dir.path(), "IMG_1.heic");
        let fake = Fake::default();
        let out = to_common(&src, Some(&fake)).await;
        let jpg = dir.path().join("IMG_1.jpg");
        assert_eq!(out, Common::Converted(jpg.clone()));
        assert!(std::fs::read(&jpg).unwrap().starts_with(b"\xFF\xD8"));
        assert!(!src.exists(), "the copy of the HEIC goes");
        assert_eq!(*fake.seen.lock().unwrap(), vec![(50_000_000, 92)]);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "nothing else is left");
    }

    /// A HEIC named as a JPEG is told by its bytes and replaced in place.
    #[tokio::test]
    async fn a_heic_named_jpg_is_converted_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let src = heic_at(dir.path(), "IMG_2.jpg");
        assert_eq!(to_common(&src, Some(&Fake::default())).await, Common::Converted(src.clone()));
        assert!(std::fs::read(&src).unwrap().starts_with(b"\xFF\xD8"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    /// One the decoder cannot read stays as it was, whole, and nothing
    /// half written is left; without a decoder nothing is even looked at.
    #[tokio::test]
    async fn an_unreadable_heic_stays_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let src = heic_at(dir.path(), "broken.heic");
        let before = std::fs::read(&src).unwrap();
        let fake = Fake { fail: true, ..Fake::default() };
        assert_eq!(to_common(&src, Some(&fake)).await, Common::Unsupported);
        assert_eq!(std::fs::read(&src).unwrap(), before);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "the half JPEG goes");

        assert_eq!(to_common(&src, None).await, Common::AsIs, "a computer: no decoder");
    }

    /// A common picture, a video, a missing file: nothing to do.
    #[tokio::test]
    async fn common_files_go_as_they_are() {
        let dir = tempfile::tempdir().unwrap();
        let jpg = dir.path().join("a.jpg");
        std::fs::write(&jpg, b"\xFF\xD8\xFF\xE0\0\x10JFIF\0\x01\x01\0\0\x01\0\x01").unwrap();
        let mp4 = dir.path().join("v.mp4");
        std::fs::write(&mp4, ftyp(b"isom", &[b"mp41"])).unwrap();
        let fake = Fake::default();
        assert_eq!(to_common(&jpg, Some(&fake)).await, Common::AsIs);
        assert_eq!(to_common(&mp4, Some(&fake)).await, Common::AsIs);
        assert_eq!(to_common(&dir.path().join("gone.heic"), Some(&fake)).await, Common::AsIs);
        assert!(fake.seen.lock().unwrap().is_empty());
    }

    /// Photos are converted one at a time on a phone (`PHOTO_SLOTS`), as
    /// many as a computer makes smaller at once elsewhere.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn conversions_take_the_slots_of_photos() {
        let dir = tempfile::tempdir().unwrap();
        let fake = std::sync::Arc::new(Fake::default());
        let mut all = Vec::new();
        for i in 0..6 {
            let src = heic_at(dir.path(), &format!("p{i}.heic"));
            let fake = fake.clone();
            all.push(tokio::spawn(async move { to_common(&src, Some(&*fake)).await }));
        }
        for t in all {
            assert!(matches!(t.await.unwrap(), Common::Converted(_)));
        }
        assert!(fake.most.load(Ordering::SeqCst) <= crate::prepare::PHOTO_SLOTS);
    }
}
