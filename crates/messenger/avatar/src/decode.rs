// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Reading a picture of any source into plain RGB pixels.
//!
//! The format is taken from the bytes, never from a file name. The size the
//! header declares is checked before a single pixel is decoded, so a small
//! file that claims to be enormous is refused at once. What the camera said
//! about the orientation is applied, transparency becomes white, and of an
//! animated GIF only the first frame is kept.
//!
//! What a picture may cost depends on where it comes from. A file I picked
//! myself may be a large photo; the avatar of somebody else, or the picture
//! inside a contact card, comes from anyone who can write to me and gets a
//! far smaller budget. Every budget bounds the pixels and the memory of the
//! whole way: the decoder, its output, and the copies made after it.

use std::io::Cursor;

use image::metadata::Orientation;
use image::{ColorType, DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, RgbImage};

use crate::{AvatarError, MAX_INPUT_BYTES, MAX_PIXELS};

/// The longest side a picture may declare.
const MAX_SIDE: u32 = 50_000;

/// Pixels any file may declare, however short it is.
const FREE_PIXELS: u64 = 4_000_000;

/// Beyond `FREE_PIXELS`, the pixels a file may declare per byte of its
/// length. Honest pictures stay far below it (a flat JPEG reaches about
/// 100, a PNG of a single color some 350); RLE bitmaps, GIF canvases and
/// patched headers that cost nothing to send do not.
const PIXELS_PER_BYTE: u64 = 1024;

const MIB: u64 = 1024 * 1024;

/// What decoding one picture may cost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Budget {
    /// The most pixels the header may declare.
    pub(crate) max_pixels: u64,
    /// The most memory the decoder, its output and the copies after it may
    /// hold at once.
    pub(crate) max_bytes: u64,
}

/// A picture I picked myself: a 50 MP photo, turned upright.
pub(crate) const OWN: Budget = Budget { max_pixels: MAX_PIXELS, max_bytes: 320 * MIB };

/// The avatar of somebody else (fetched, at most 2 MB).
pub(crate) const OTHERS: Budget = Budget { max_pixels: 16_000_000, max_bytes: 128 * MIB };

/// The picture of a contact card (at most 12 KB when received).
pub(crate) const CARD: Budget = Budget { max_pixels: 2_000_000, max_bytes: 32 * MIB };

/// A picture after decoding: upright, opaque, RGB.
#[derive(Clone)]
pub struct Decoded {
    pub(crate) rgb: RgbImage,
}

impl Decoded {
    /// Width in pixels, after the orientation was applied.
    pub fn width(&self) -> u32 {
        self.rgb.width()
    }

    /// Height in pixels, after the orientation was applied.
    pub fn height(&self) -> u32 {
        self.rgb.height()
    }
}

impl std::fmt::Debug for Decoded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decoded")
            .field("width", &self.width())
            .field("height", &self.height())
            .finish()
    }
}

/// Decodes a JPEG, PNG, WebP, GIF (first frame) or BMP that I picked
/// myself.
pub fn decode(bytes: &[u8]) -> Result<Decoded, AvatarError> {
    let (rgb, orientation) = decode_limited(bytes, OWN, true)?;
    debug_assert_eq!(orientation, Orientation::NoTransforms);
    Ok(Decoded { rgb })
}

/// Is this a still JPEG, PNG or WebP, a photo worth making smaller? Told
/// by the bytes, never by a name. A GIF, and a PNG or WebP that moves,
/// keeps its frames; a PNG or WebP that can be seen through keeps its
/// transparency (a JPEG would turn it white); other formats are left as
/// they are.
pub fn still_photo(bytes: &[u8]) -> bool {
    match image::guess_format(bytes) {
        Ok(ImageFormat::Jpeg) => true,
        Ok(ImageFormat::Png) => !png_moves(bytes) && !see_through(bytes),
        Ok(ImageFormat::WebP) => !webp_moves(bytes) && !webp_see_through(bytes) && !see_through(bytes),
        _ => false,
    }
}

/// The header says the picture has an alpha channel (or, in a PNG, a
/// transparent color). Not when it cannot be read.
fn see_through(bytes: &[u8]) -> bool {
    ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_decoder().ok())
        .is_some_and(|d| d.color_type().has_alpha())
}

/// A WebP whose extended header (`VP8X`) says it has an alpha channel.
fn webp_see_through(bytes: &[u8]) -> bool {
    bytes.get(12..16) == Some(b"VP8X") && bytes.get(20).is_some_and(|flags| flags & 0x10 != 0)
}

/// An animated PNG: an `acTL` chunk before the first `IDAT`.
fn png_moves(bytes: &[u8]) -> bool {
    let mut at = 8;
    while let Some(head) = bytes.get(at..at + 8) {
        let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        match &head[4..8] {
            b"acTL" => return true,
            b"IDAT" => return false,
            _ => at = at.saturating_add(12).saturating_add(len),
        }
    }
    false
}

/// An animated WebP: its extended header (`VP8X`) says so.
fn webp_moves(bytes: &[u8]) -> bool {
    bytes.get(12..16) == Some(b"VP8X") && bytes.get(20).is_some_and(|flags| flags & 0x02 != 0)
}

/// The size of a picture as it is shown, upright, from its header alone:
/// the first bytes of the file are enough. `None` when they cannot be
/// read as a JPEG, PNG, WebP, GIF or BMP.
pub fn upright_size(head: &[u8]) -> Option<(u32, u32)> {
    let reader = ImageReader::new(Cursor::new(head)).with_guessed_format().ok()?;
    if !matches!(
        reader.format(),
        Some(ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP | ImageFormat::Gif | ImageFormat::Bmp)
    ) {
        return None;
    }
    let mut decoder = reader.into_decoder().ok()?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 {
        return None;
    }
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    Some(if turns(orientation) { (height, width) } else { (width, height) })
}

/// Decodes within `budget`. With `upright`, the orientation is applied to
/// the whole picture; without it, the pixels stay as stored and the
/// orientation is returned for the caller to apply to a small part (a
/// rotation copies every pixel).
pub(crate) fn decode_limited(
    bytes: &[u8],
    budget: Budget,
    upright: bool,
) -> Result<(RgbImage, Orientation), AvatarError> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(AvatarError::TooLarge);
    }
    if bytes.is_empty() {
        return Err(AvatarError::Corrupt);
    }

    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| AvatarError::Corrupt)?;
    match reader.format() {
        Some(
            ImageFormat::Jpeg
            | ImageFormat::Png
            | ImageFormat::WebP
            | ImageFormat::Gif
            | ImageFormat::Bmp,
        ) => {}
        _ => return Err(AvatarError::Unsupported),
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(budget.max_bytes);
    reader.limits(limits.clone());

    // Only the header is read here; the limits on the sides are checked by it.
    let mut decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 {
        return Err(AvatarError::Corrupt);
    }
    let pixels = u64::from(width) * u64::from(height);
    let honest = FREE_PIXELS.max(bytes.len() as u64 * PIXELS_PER_BYTE);
    if pixels > budget.max_pixels || pixels > honest {
        return Err(AvatarError::TooLarge);
    }
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let rotate = upright && turns(orientation);
    let total = decoder.total_bytes();
    if peak_after(total, pixels, decoder.color_type(), rotate) > budget.max_bytes {
        return Err(AvatarError::TooLarge);
    }
    // The output buffer comes out of the budget first; the decoder gets
    // what is left for its own buffers, as `ImageReader::decode` does.
    limits.reserve(total)?;
    decoder.set_limits(limits)?;

    let image = DynamicImage::from_decoder(decoder)?;
    let rgb = flatten(image);
    if !upright {
        return Ok((rgb, orientation));
    }
    Ok((orient(rgb, orientation), Orientation::NoTransforms))
}

/// Whether the orientation swaps the sides, which copies the picture.
fn turns(orientation: Orientation) -> bool {
    matches!(
        orientation,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    )
}

/// The most memory held at once after the decoder is done, for a decoded
/// buffer of `total` bytes: the RGB copy of `flatten`, then the copy of a
/// rotation.
pub(crate) fn peak_after(total: u64, pixels: u64, color: ColorType, rotate: bool) -> u64 {
    let rgb = pixels * 3;
    let (flattened, held) = match color {
        ColorType::Rgb8 => (total, rgb),
        ColorType::Rgba8 => (total, total),
        c if c.has_alpha() => (total + pixels * 4, pixels * 4),
        _ => (total + rgb, rgb),
    };
    if rotate {
        flattened.max(held + rgb)
    } else {
        flattened
    }
}

/// Applies an orientation to RGB pixels.
pub(crate) fn orient(rgb: RgbImage, orientation: Orientation) -> RgbImage {
    if orientation == Orientation::NoTransforms {
        return rgb;
    }
    let mut image = DynamicImage::ImageRgb8(rgb);
    image.apply_orientation(orientation);
    image.into_rgb8()
}

/// RGB of the picture, with any transparency laid over white. RGBA is
/// turned into RGB in its own buffer, so no second copy is made.
fn flatten(image: DynamicImage) -> RgbImage {
    if !image.color().has_alpha() {
        return image.into_rgb8();
    }
    let rgba = image.into_rgba8();
    let (width, height) = rgba.dimensions();
    let mut buf = rgba.into_raw();
    let count = buf.len() / 4;
    for i in 0..count {
        let alpha = u32::from(buf[i * 4 + 3]);
        for c in 0..3 {
            let value = u32::from(buf[i * 4 + c]) * alpha + 255 * (255 - alpha);
            // Byte 3i+c lies before every byte still to be read (4i+c+1 on).
            buf[i * 3 + c] = ((value + 127) / 255) as u8;
        }
    }
    buf.truncate(count * 3);
    buf.shrink_to_fit();
    RgbImage::from_raw(width, height, buf).expect("three bytes for every pixel")
}
