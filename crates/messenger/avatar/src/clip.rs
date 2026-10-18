// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Pictures that come from the clipboard: what a pasted file is by its
//! bytes, and the rows of pixels a computer's clipboard hands over written
//! as a PNG. Nothing here decodes a picture: `picture_ext` only reads its
//! signature, and `png_of_rgba` encodes rows it was given.

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageEncoder, ImageFormat};

use crate::AvatarError;

/// The most pixels a picture of the clipboard may have (a screenshot of
/// three 8K screens fits; the rows take four bytes a pixel).
pub const MAX_CLIP_PIXELS: u64 = 120_000_000;

/// The extension a picture is saved under, by its first bytes: the app
/// tells a file's type by its name, and a pasted file may have none or a
/// wrong one. `None` for anything that is not one of these pictures.
pub fn picture_ext(bytes: &[u8]) -> Option<&'static str> {
    match image::guess_format(bytes).ok()? {
        ImageFormat::Png => Some("png"),
        ImageFormat::Jpeg => Some("jpg"),
        ImageFormat::WebP => Some("webp"),
        ImageFormat::Gif => Some("gif"),
        ImageFormat::Bmp => Some("bmp"),
        ImageFormat::Avif => Some("avif"),
        _ => None,
    }
}

/// Rows of RGBA pixels as a PNG. Fast compression: a screenshot is written
/// while the user waits, and a photo made of it is made smaller later
/// anyway.
pub fn png_of_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, AvatarError> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || pixels > MAX_CLIP_PIXELS {
        return Err(AvatarError::TooLarge);
    }
    if rgba.len() as u64 != pixels * 4 {
        return Err(AvatarError::Corrupt);
    }
    let mut out = Vec::new();
    PngEncoder::new_with_quality(&mut out, CompressionType::Fast, FilterType::Adaptive)
        .write_image(rgba, width, height, ExtendedColorType::Rgba8)
        .map_err(|_| AvatarError::Corrupt)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_told_by_its_bytes() {
        let png = png_of_rgba(2, 1, &[255, 0, 0, 255, 0, 0, 255, 128]).unwrap();
        assert_eq!(picture_ext(&png), Some("png"));
        assert_eq!(picture_ext(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10]), Some("jpg"));
        assert_eq!(picture_ext(b"GIF89a......"), Some("gif"));
        assert_eq!(picture_ext(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(picture_ext(b"%PDF-1.7"), None, "not a picture");
        assert_eq!(picture_ext(b""), None);
    }

    #[test]
    fn rows_become_a_png_that_reads_back() {
        let rgba: Vec<u8> = (0..4 * 3 * 2).map(|i| i as u8).collect();
        let png = png_of_rgba(3, 2, &rgba).unwrap();
        let back = image::load_from_memory_with_format(&png, ImageFormat::Png).unwrap().to_rgba8();
        assert_eq!((back.width(), back.height()), (3, 2));
        assert_eq!(back.into_raw(), rgba);
    }

    #[test]
    fn odd_rows_are_refused() {
        assert_eq!(png_of_rgba(0, 5, &[]), Err(AvatarError::TooLarge));
        assert_eq!(png_of_rgba(20_000, 20_000, &[]), Err(AvatarError::TooLarge));
        assert_eq!(png_of_rgba(2, 2, &[0; 15]), Err(AvatarError::Corrupt), "rows shorter than the size");
    }
}
