// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Tests of the avatar crate, on tiny pictures built in memory.

use std::time::{Duration, Instant};

use base64::Engine;
use image::codecs::bmp::BmpEncoder;
use image::codecs::gif::GifEncoder;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::webp::WebPEncoder;
use image::{
    load_from_memory, ExtendedColorType, Frame, ImageEncoder, Rgb, RgbImage, Rgba, RgbaImage,
};

use crate::*;

// Pictures.

/// Left half red, right half blue.
fn two_tone(width: u32, height: u32) -> RgbImage {
    RgbImage::from_fn(width, height, |x, _| {
        if x < width / 2 {
            Rgb([255, 0, 0])
        } else {
            Rgb([0, 0, 255])
        }
    })
}

/// Pseudo-random pixels, the worst case for a JPEG's size.
fn noise(width: u32, height: u32) -> RgbImage {
    let mut state: u32 = 0x1234_5678;
    RgbImage::from_fn(width, height, |_, _| {
        let mut next = || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        };
        Rgb([next(), next(), next()])
    })
}

fn jpeg_of(img: &RgbImage) -> Vec<u8> {
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 95)
        .encode_image(img)
        .unwrap();
    out
}

fn png_rgba(img: &RgbaImage) -> Vec<u8> {
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(img.as_raw(), img.width(), img.height(), ExtendedColorType::Rgba8)
        .unwrap();
    out
}

fn png_rgb(img: &RgbImage) -> Vec<u8> {
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(img.as_raw(), img.width(), img.height(), ExtendedColorType::Rgb8)
        .unwrap();
    out
}

/// A JPEG with an EXIF APP1 segment that carries only an orientation tag
/// (big-endian TIFF, one IFD entry), put right after the JFIF segment.
fn jpeg_with_orientation(img: &RgbImage, orientation: u16) -> Vec<u8> {
    let plain = jpeg_of(img);
    let mut payload = b"Exif\0\0".to_vec();
    payload.extend_from_slice(b"MM\0\x2a");
    payload.extend_from_slice(&8u32.to_be_bytes());
    payload.extend_from_slice(&1u16.to_be_bytes());
    payload.extend_from_slice(&0x0112u16.to_be_bytes()); // Orientation
    payload.extend_from_slice(&3u16.to_be_bytes()); // SHORT
    payload.extend_from_slice(&1u32.to_be_bytes());
    payload.extend_from_slice(&orientation.to_be_bytes());
    payload.extend_from_slice(&[0, 0]);
    payload.extend_from_slice(&0u32.to_be_bytes());

    assert_eq!(&plain[2..4], &[0xFF, 0xE0], "JFIF segment first");
    let app0_end = 4 + usize::from(u16::from_be_bytes([plain[4], plain[5]]));
    let mut out = plain[..app0_end].to_vec();
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(&payload);
    out.extend_from_slice(&plain[app0_end..]);
    out
}

/// The markers of a JPEG up to the start of the scan.
fn markers(jpeg: &[u8]) -> Vec<u8> {
    assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "a JPEG starts with SOI");
    let mut out = Vec::new();
    let mut i = 2;
    while i + 4 <= jpeg.len() {
        assert_eq!(jpeg[i], 0xFF, "a marker at {i}");
        let marker = jpeg[i + 1];
        out.push(marker);
        if marker == 0xDA {
            break;
        }
        i += 2 + usize::from(u16::from_be_bytes([jpeg[i + 2], jpeg[i + 3]]));
    }
    out
}

fn assert_clean_jpeg(jpeg: &[u8]) {
    let found = markers(jpeg);
    assert!(found.contains(&0xDA), "a scan follows the header");
    for bad in [0xE1, 0xE2, 0xED, 0xFE] {
        assert!(!found.contains(&bad), "no metadata segment {bad:#x} in {found:x?}");
    }
}

fn is_red(p: &Rgb<u8>) -> bool {
    p[0] > 200 && p[1] < 70 && p[2] < 70
}

fn is_blue(p: &Rgb<u8>) -> bool {
    p[0] < 70 && p[1] < 70 && p[2] > 200
}

fn rgb_of(jpeg: &[u8]) -> RgbImage {
    load_from_memory(jpeg).unwrap().into_rgb8()
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// A tiny PNG whose header claims `width` by `height`.
fn png_claiming(width: u32, height: u32) -> Vec<u8> {
    let mut png = png_rgb(&two_tone(4, 4));
    assert_eq!(&png[12..16], b"IHDR");
    png[16..20].copy_from_slice(&width.to_be_bytes());
    png[20..24].copy_from_slice(&height.to_be_bytes());
    let crc = crc32(&png[12..29]);
    png[29..33].copy_from_slice(&crc.to_be_bytes());
    png
}

/// A tiny JPEG whose frame header claims `width` by `height`.
fn jpeg_claiming(width: u16, height: u16) -> Vec<u8> {
    let mut jpeg = jpeg_of(&two_tone(8, 8));
    let sof = jpeg
        .windows(2)
        .position(|w| w == [0xFF, 0xC0])
        .expect("a baseline frame header");
    jpeg[sof + 5..sof + 7].copy_from_slice(&height.to_be_bytes());
    jpeg[sof + 7..sof + 9].copy_from_slice(&width.to_be_bytes());
    jpeg
}

/// A 35-byte GIF: a 7000x7000 logical screen with a single 1x1 frame.
fn gif_screen_bomb() -> Vec<u8> {
    let mut gif = b"GIF89a".to_vec();
    gif.extend_from_slice(&7000u16.to_le_bytes());
    gif.extend_from_slice(&7000u16.to_le_bytes());
    gif.extend_from_slice(&[0x80, 0, 0]); // a global table of 2 colors
    gif.extend_from_slice(&[0, 0, 0, 255, 255, 255]);
    gif.push(0x2C);
    gif.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0, 0]); // at 0,0, 1x1
    gif.extend_from_slice(&[2, 2, 0x44, 0x01, 0]); // LZW: clear, 0, end
    gif.push(0x3B);
    assert_eq!(gif.len(), 35);
    gif
}

/// A 64-byte RLE8 BMP of 7000x7000 whose data is only the end marker.
fn bmp_rle_bomb() -> Vec<u8> {
    let mut bmp = b"BM".to_vec();
    for value in [64u32, 0, 62, 40, 7000, 7000] {
        bmp.extend_from_slice(&value.to_le_bytes());
    }
    bmp.extend_from_slice(&1u16.to_le_bytes()); // planes
    bmp.extend_from_slice(&8u16.to_le_bytes()); // bits per pixel
    // BI_RLE8, data size, resolution, colors used, colors important.
    for value in [1u32, 2, 2835, 2835, 2, 0] {
        bmp.extend_from_slice(&value.to_le_bytes());
    }
    bmp.extend_from_slice(&[0, 0, 0, 0, 255, 255, 255, 0]);
    bmp.extend_from_slice(&[0, 1]); // end of bitmap
    assert_eq!(bmp.len(), 64);
    bmp
}

/// A tiny PNG whose header claims `width` by `height` of `depth` bits and
/// PNG color type `color`, padded with zeros to `len` bytes.
fn png_header_claiming(width: u32, height: u32, depth: u8, color: u8, len: usize) -> Vec<u8> {
    let mut png = png_claiming(width, height);
    png[24] = depth;
    png[25] = color;
    let crc = crc32(&png[12..29]);
    png[29..33].copy_from_slice(&crc.to_be_bytes());
    if png.len() < len {
        png.resize(len, 0);
    }
    png
}

// Errors.

#[test]
fn error_codes_are_stable() {
    assert_eq!(AvatarError::TooLarge.code(), "avatar_too_large");
    assert_eq!(AvatarError::Unsupported.code(), "avatar_unsupported");
    assert_eq!(AvatarError::Corrupt.code(), "avatar_corrupt");
    assert_eq!(AvatarError::BadCrop.code(), "avatar_bad_crop");
    assert_eq!(AvatarError::BadCrop.to_string(), "avatar_bad_crop");
}

// Decoding.

#[test]
fn decodes_every_format() {
    let img = two_tone(30, 20);
    let rgba = RgbaImage::from_fn(30, 20, |x, _| {
        if x < 15 {
            Rgba([255, 0, 0, 255])
        } else {
            Rgba([0, 0, 255, 255])
        }
    });

    let mut webp = Vec::new();
    WebPEncoder::new_lossless(&mut webp)
        .write_image(rgba.as_raw(), 30, 20, ExtendedColorType::Rgba8)
        .unwrap();
    let mut bmp = Vec::new();
    BmpEncoder::new(&mut bmp)
        .write_image(img.as_raw(), 30, 20, ExtendedColorType::Rgb8)
        .unwrap();
    let mut gif = Vec::new();
    GifEncoder::new(&mut gif)
        .encode_frame(Frame::new(rgba.clone()))
        .unwrap();

    for (name, bytes) in [
        ("jpeg", jpeg_of(&img)),
        ("png", png_rgb(&img)),
        ("png rgba", png_rgba(&rgba)),
        ("webp", webp),
        ("bmp", bmp),
        ("gif", gif),
    ] {
        let decoded = decode(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!((decoded.width(), decoded.height()), (30, 20), "{name}");
        assert!(is_red(decoded.rgb.get_pixel(2, 10)), "{name}");
        assert!(is_blue(decoded.rgb.get_pixel(27, 10)), "{name}");
    }
}

#[test]
fn exif_orientation_6_turns_clockwise() {
    let bytes = jpeg_with_orientation(&two_tone(40, 20), 6);
    assert!(markers(&bytes).contains(&0xE1), "the source carries EXIF");
    let decoded = decode(&bytes).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (20, 40));
    // The left (red) half of the source is now on top.
    assert!(is_red(decoded.rgb.get_pixel(10, 3)));
    assert!(is_blue(decoded.rgb.get_pixel(10, 36)));
}

#[test]
fn exif_orientation_8_turns_counterclockwise() {
    let bytes = jpeg_with_orientation(&two_tone(40, 20), 8);
    let decoded = decode(&bytes).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (20, 40));
    // The left (red) half of the source is now at the bottom.
    assert!(is_blue(decoded.rgb.get_pixel(10, 3)));
    assert!(is_red(decoded.rgb.get_pixel(10, 36)));
}

#[test]
fn exif_orientation_1_keeps_the_picture() {
    let decoded = decode(&jpeg_with_orientation(&two_tone(40, 20), 1)).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (40, 20));
    assert!(is_red(decoded.rgb.get_pixel(3, 10)));
}

#[test]
fn exif_does_not_survive_any_output() {
    let bytes = jpeg_with_orientation(&two_tone(400, 200), 6);
    let decoded = decode(&bytes).unwrap();
    let full = CropRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
    assert_clean_jpeg(&crop_square(&decoded, full, OWN_SIDE).unwrap());
    assert_clean_jpeg(&preview_jpeg(&decoded, 1024));
    assert_clean_jpeg(&square_thumb(&bytes, CACHE_SIDE).unwrap());
    assert_clean_jpeg(&card_thumb(&bytes).unwrap());
}

#[test]
fn alpha_is_laid_over_white() {
    let transparent = RgbaImage::from_pixel(4, 4, Rgba([0, 0, 0, 0]));
    let decoded = decode(&png_rgba(&transparent)).unwrap();
    assert!(decoded.rgb.pixels().all(|p| *p == Rgb([255, 255, 255])));

    let half_red = RgbaImage::from_pixel(4, 4, Rgba([255, 0, 0, 128]));
    let decoded = decode(&png_rgba(&half_red)).unwrap();
    assert_eq!(*decoded.rgb.get_pixel(1, 1), Rgb([255, 127, 127]));

    let opaque_blue = RgbaImage::from_pixel(4, 4, Rgba([0, 0, 255, 255]));
    let decoded = decode(&png_rgba(&opaque_blue)).unwrap();
    assert_eq!(*decoded.rgb.get_pixel(1, 1), Rgb([0, 0, 255]));
}

#[test]
fn gif_keeps_the_first_frame() {
    let red = RgbaImage::from_pixel(8, 8, Rgba([255, 0, 0, 255]));
    let blue = RgbaImage::from_pixel(8, 8, Rgba([0, 0, 255, 255]));
    let mut gif = Vec::new();
    {
        let mut encoder = GifEncoder::new(&mut gif);
        encoder
            .encode_frames(vec![Frame::new(red), Frame::new(blue)])
            .unwrap();
    }
    let decoded = decode(&gif).unwrap();
    assert!(decoded.rgb.pixels().all(is_red));
}

#[test]
fn a_decompression_bomb_is_refused_before_decoding() {
    for bytes in [
        png_claiming(100_000, 100_000),
        png_claiming(40_000, 40_000),
        jpeg_claiming(65_000, 65_000),
        jpeg_claiming(10_000, 6_000),
    ] {
        let started = Instant::now();
        assert_eq!(decode(&bytes).unwrap_err(), AvatarError::TooLarge);
        assert!(bytes.len() < 4096);
        assert!(started.elapsed() < Duration::from_millis(500), "refused fast");
    }
    assert_eq!(square_thumb(&png_claiming(100_000, 100_000), 256), Err(AvatarError::TooLarge));
    assert_eq!(card_thumb(&jpeg_claiming(65_000, 65_000)), Err(AvatarError::TooLarge));
}

#[test]
fn a_tiny_file_declaring_a_huge_canvas_is_refused() {
    for (name, bytes) in [
        ("gif screen", gif_screen_bomb()),
        ("bmp rle", bmp_rle_bomb()),
        ("jpeg sof", jpeg_claiming(7000, 7000)),
    ] {
        let started = Instant::now();
        assert_eq!(decode(&bytes).unwrap_err(), AvatarError::TooLarge, "{name}");
        assert_eq!(square_thumb(&bytes, CACHE_SIDE).unwrap_err(), AvatarError::TooLarge, "{name}");
        assert_eq!(card_thumb(&bytes).unwrap_err(), AvatarError::TooLarge, "{name}");
        assert!(started.elapsed() < Duration::from_millis(100), "{name}: refused unread");
    }
}

#[test]
fn a_small_canvas_of_a_tiny_file_still_decodes() {
    // Up to the pixels any file may declare, its length is not asked.
    let mut gif = gif_screen_bomb();
    gif[6..8].copy_from_slice(&1000u16.to_le_bytes());
    gif[8..10].copy_from_slice(&1000u16.to_le_bytes());
    let decoded = decode(&gif).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (1000, 1000));
    let card = rgb_of(&card_thumb(&gif).unwrap());
    assert_eq!(card.dimensions(), (CARD_SIDE, CARD_SIDE));
}

#[test]
fn pixels_are_budgeted_per_entry() {
    // 17.6 MP with bytes enough for its size: my own pick may be that large
    // (it fails later only because the data is not there), somebody else's
    // avatar may not.
    let others = png_header_claiming(4200, 4200, 8, 2, 40_000);
    assert_eq!(decode(&others).unwrap_err(), AvatarError::Corrupt);
    assert_eq!(square_thumb(&others, CACHE_SIDE).unwrap_err(), AvatarError::TooLarge);
    assert_eq!(card_thumb(&others).unwrap_err(), AvatarError::TooLarge);

    // 2.25 MP: a thumbnail of somebody else, not the picture of a card.
    let card = png_header_claiming(1500, 1500, 8, 2, 4_000);
    assert_eq!(square_thumb(&card, CACHE_SIDE).unwrap_err(), AvatarError::Corrupt);
    assert_eq!(card_thumb(&card).unwrap_err(), AvatarError::TooLarge);
}

#[test]
fn the_copies_after_decoding_count_toward_the_budget() {
    // 31 MP of 16-bit RGBA: under the pixel limit, but with the 8-bit copy
    // made after decoding the memory would go past the budget.
    let deep = png_header_claiming(5600, 5600, 16, 6, 64_000);
    assert_eq!(decode(&deep).unwrap_err(), AvatarError::TooLarge);
    // The same size in 8-bit RGB fits.
    let plain = png_header_claiming(5600, 5600, 8, 2, 64_000);
    assert_eq!(decode(&plain).unwrap_err(), AvatarError::Corrupt);
}

#[test]
fn budgets_fit_what_each_entry_must_take() {
    use crate::decode::{peak_after, CARD, OTHERS, OWN};
    use image::ColorType;
    const MP50: u64 = 50_000_000;

    // My own 50 MP photo, turned upright: decoded RGB and its rotation.
    assert_eq!(peak_after(MP50 * 3, MP50, ColorType::Rgb8, true), MP50 * 6);
    assert!(peak_after(MP50 * 3, MP50, ColorType::Rgb8, true) <= OWN.max_bytes);
    // A grayscale one too.
    assert!(peak_after(MP50, MP50, ColorType::L8, true) <= OWN.max_bytes);
    // RGBA becomes RGB in its own buffer: no copy without a rotation.
    assert_eq!(peak_after(MP50 * 4, MP50, ColorType::Rgba8, false), MP50 * 4);
    // 16-bit RGBA is converted to 8 bits first, then laid over white.
    assert_eq!(peak_after(MP50 * 8, MP50, ColorType::Rgba16, false), MP50 * 12);
    assert!(peak_after(MP50 * 8, MP50, ColorType::Rgba16, false) > OWN.max_bytes);

    // The largest picture of others and of cards fits with any 8-bit color.
    for (budget, pixels) in [(OTHERS, OTHERS.max_pixels), (CARD, CARD.max_pixels)] {
        for (color, per_pixel) in
            [(ColorType::Rgb8, 3), (ColorType::Rgba8, 4), (ColorType::La8, 2), (ColorType::L8, 1)]
        {
            let peak = peak_after(pixels * per_pixel, pixels, color, false);
            assert!(peak <= budget.max_bytes, "{budget:?} {color:?}");
        }
    }
    const { assert!(CARD.max_pixels < OTHERS.max_pixels && OTHERS.max_pixels < OWN.max_pixels) };
    const { assert!(CARD.max_bytes < OTHERS.max_bytes && OTHERS.max_bytes < OWN.max_bytes) };
}

#[test]
fn thumbnails_turn_the_square_after_cutting_it() {
    // 80x40 stored, upright 40x80 by orientation 8: the center square of
    // the upright picture spans y 20..60, blue above red.
    let bytes = jpeg_with_orientation(&two_tone(80, 40), 8);
    let out = rgb_of(&square_thumb(&bytes, 40).unwrap());
    assert_eq!(out.dimensions(), (40, 40));
    assert!(is_blue(out.get_pixel(20, 3)));
    assert!(is_red(out.get_pixel(20, 36)));

    let bytes = jpeg_with_orientation(&two_tone(400, 200), 6);
    let out = rgb_of(&card_thumb(&bytes).unwrap());
    assert_eq!(out.dimensions(), (CARD_SIDE, CARD_SIDE));
    assert!(is_red(out.get_pixel(80, 10)));
    assert!(is_blue(out.get_pixel(80, 150)));
}

#[test]
fn alpha_is_laid_over_white_in_every_layout() {
    // RGBA8 is flattened in its own buffer; check the first and last pixels.
    let img = RgbaImage::from_fn(5, 3, |x, y| {
        if (x, y) == (4, 2) {
            Rgba([0, 255, 0, 255])
        } else {
            Rgba([0, 0, 255, 0])
        }
    });
    let decoded = decode(&png_rgba(&img)).unwrap();
    assert_eq!(*decoded.rgb.get_pixel(0, 0), Rgb([255, 255, 255]));
    assert_eq!(*decoded.rgb.get_pixel(4, 2), Rgb([0, 255, 0]));

    // Gray with alpha takes the other way.
    let mut la = Vec::new();
    PngEncoder::new(&mut la)
        .write_image(&[0, 0, 0, 255, 200, 128, 50, 0], 2, 2, ExtendedColorType::La8)
        .unwrap();
    let decoded = decode(&la).unwrap();
    assert_eq!(*decoded.rgb.get_pixel(0, 0), Rgb([255, 255, 255]));
    assert_eq!(*decoded.rgb.get_pixel(1, 0), Rgb([0, 0, 0]));
    assert_eq!(*decoded.rgb.get_pixel(0, 1), Rgb([227, 227, 227]));
    assert_eq!(*decoded.rgb.get_pixel(1, 1), Rgb([255, 255, 255]));
}

#[test]
fn too_many_bytes_are_refused_unread() {
    let mut bytes = vec![0u8; MAX_INPUT_BYTES + 1];
    bytes[..3].copy_from_slice(&[0xFF, 0xD8, 0xFF]);
    assert_eq!(decode(&bytes).unwrap_err(), AvatarError::TooLarge);
}

#[test]
fn garbage_with_a_known_signature_is_corrupt() {
    let mut fake_jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
    fake_jpeg.extend(std::iter::repeat_n(0x5A, 200));
    assert_eq!(decode(&fake_jpeg).unwrap_err(), AvatarError::Corrupt);

    let mut fake_png = b"\x89PNG\r\n\x1a\n".to_vec();
    fake_png.extend(std::iter::repeat_n(0xA5, 200));
    assert_eq!(decode(&fake_png).unwrap_err(), AvatarError::Corrupt);

    let png = png_rgb(&noise(64, 64));
    assert_eq!(decode(&png[..png.len() / 2]).unwrap_err(), AvatarError::Corrupt);

    let gif = b"GIF89a\x10\x00".to_vec();
    assert_eq!(decode(&gif).unwrap_err(), AvatarError::Corrupt);

    assert_eq!(decode(&[]).unwrap_err(), AvatarError::Corrupt);
}

#[test]
fn a_truncated_jpeg_is_refused() {
    // zune-jpeg is lenient and may fill a cut scan with gray; it must never
    // succeed with the wrong size or panic.
    let jpeg = jpeg_of(&noise(64, 64));
    match decode(&jpeg[..jpeg.len() / 2]) {
        Ok(decoded) => assert_eq!((decoded.width(), decoded.height()), (64, 64)),
        Err(err) => assert_eq!(err, AvatarError::Corrupt),
    }
    assert_eq!(decode(&jpeg[..20]).unwrap_err(), AvatarError::Corrupt);
}

#[test]
fn other_formats_are_unsupported() {
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10"/></svg>"#;
    let svg_decl = b"<?xml version=\"1.0\"?><svg xmlns=\"http://www.w3.org/2000/svg\"/>";
    let html = b"<!DOCTYPE html><html><body><img src=x onerror=alert(1)></body></html>";
    let tiff = b"II*\0\x08\0\0\0\0\0\0\0\0\0\0\0";
    let heic = b"\0\0\0\x18ftypheic\0\0\0\0mif1heic";
    let text = b"just some words, not a picture at all";
    let random: Vec<u8> = noise(16, 16).into_raw();
    for bytes in [&svg[..], svg_decl, html, tiff, heic, text, &random] {
        assert_eq!(decode(bytes).unwrap_err(), AvatarError::Unsupported);
        assert_eq!(square_thumb(bytes, 256).unwrap_err(), AvatarError::Unsupported);
    }
}

// Cropping.

#[test]
fn crop_square_makes_a_clean_square() {
    let decoded = decode(&png_rgb(&two_tone(200, 100))).unwrap();
    // The left half of the picture is a 100 px square of red.
    let rect = CropRect { x: 0.0, y: 0.0, w: 0.5, h: 1.0 };
    let jpeg = crop_square(&decoded, rect, OWN_SIDE).unwrap();
    assert_clean_jpeg(&jpeg);
    let out = rgb_of(&jpeg);
    assert_eq!(out.dimensions(), (OWN_SIDE, OWN_SIDE));
    assert!(is_red(out.get_pixel(10, 10)));
    assert!(is_red(out.get_pixel(500, 500)));
}

#[test]
fn crop_square_forces_a_square_around_the_center() {
    let decoded = decode(&png_rgb(&two_tone(200, 100))).unwrap();
    // The whole picture: a 100 px square in the middle, half red, half blue.
    let full = CropRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
    let out = rgb_of(&crop_square(&decoded, full, 64).unwrap());
    assert_eq!(out.dimensions(), (64, 64));
    assert!(is_red(out.get_pixel(5, 32)));
    assert!(is_blue(out.get_pixel(58, 32)));

    // A wide strip on the right: the square is cut from its center.
    let strip = CropRect { x: 0.5, y: 0.25, w: 0.5, h: 0.5 };
    let out = rgb_of(&crop_square(&decoded, strip, 64).unwrap());
    assert!(out.pixels().all(is_blue));
}

#[test]
fn crop_square_scales_a_small_part_up() {
    let decoded = decode(&png_rgb(&two_tone(20, 20))).unwrap();
    let rect = CropRect { x: 0.5, y: 0.5, w: 0.5, h: 0.5 };
    let out = rgb_of(&crop_square(&decoded, rect, OWN_SIDE).unwrap());
    assert_eq!(out.dimensions(), (OWN_SIDE, OWN_SIDE));
    assert!(is_blue(out.get_pixel(256, 256)));
}

#[test]
fn crop_square_accepts_rounding_at_the_edges() {
    let decoded = decode(&png_rgb(&two_tone(30, 30))).unwrap();
    let rect = CropRect { x: 1.0 / 3.0, y: 1.0 / 3.0, w: 2.0 / 3.0 + 1e-9, h: 2.0 / 3.0 };
    assert!(crop_square(&decoded, rect, 32).is_ok());
    let rect = CropRect { x: -1e-9, y: 0.0, w: 1.0, h: 1.0 };
    assert!(crop_square(&decoded, rect, 32).is_ok());
}

#[test]
fn crop_outside_the_picture_is_refused() {
    let decoded = decode(&png_rgb(&two_tone(100, 100))).unwrap();
    let bad = [
        CropRect { x: 0.6, y: 0.0, w: 0.5, h: 0.5 },
        CropRect { x: 0.0, y: 0.7, w: 0.5, h: 0.5 },
        CropRect { x: -0.1, y: 0.0, w: 0.5, h: 0.5 },
        CropRect { x: 0.0, y: -0.1, w: 0.5, h: 0.5 },
        CropRect { x: 0.0, y: 0.0, w: 0.0, h: 0.5 },
        CropRect { x: 0.0, y: 0.0, w: 0.5, h: -0.5 },
        CropRect { x: f64::NAN, y: 0.0, w: 0.5, h: 0.5 },
        CropRect { x: 0.0, y: 0.0, w: f64::INFINITY, h: 0.5 },
        CropRect { x: 0.0, y: 0.0, w: 1.5, h: 1.5 },
        // Less than a pixel.
        CropRect { x: 0.0, y: 0.0, w: 0.001, h: 0.001 },
    ];
    for rect in bad {
        assert_eq!(crop_square(&decoded, rect, OWN_SIDE), Err(AvatarError::BadCrop), "{rect:?}");
    }
    let ok = CropRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
    assert_eq!(crop_square(&decoded, ok, 0), Err(AvatarError::BadCrop));
    assert_eq!(crop_square(&decoded, ok, 100_000), Err(AvatarError::BadCrop));
}

#[test]
fn crop_rect_reads_the_ui_json() {
    let rect: CropRect =
        serde_json::from_str(r#"{"x":0.1,"y":0.2,"w":0.5,"h":0.25}"#).unwrap();
    assert_eq!(rect, CropRect { x: 0.1, y: 0.2, w: 0.5, h: 0.25 });
}

// Previews and thumbnails.

#[test]
fn preview_fits_its_side_and_never_grows() {
    let big = decode(&png_rgb(&two_tone(1536, 512))).unwrap();
    let out = rgb_of(&preview_jpeg(&big, 1024));
    assert_eq!(out.dimensions(), (1024, 341));
    let out = rgb_of(&preview_jpeg(&big, 5000));
    assert_eq!(out.dimensions(), (1024, 341), "capped at 1024");
    let out = rgb_of(&preview_jpeg(&big, 300));
    assert_eq!(out.dimensions(), (300, 100));
    assert!(is_red(out.get_pixel(20, 50)));
    assert!(is_blue(out.get_pixel(280, 50)));

    let small = decode(&png_rgb(&two_tone(50, 80))).unwrap();
    let out = rgb_of(&preview_jpeg(&small, 1024));
    assert_eq!(out.dimensions(), (50, 80));
    let out = rgb_of(&preview_jpeg(&small, 0));
    assert_eq!(out.dimensions(), (1, 1));
}

#[test]
fn square_thumb_takes_the_center() {
    // 600x300: the center square spans x 150..450, half red, half blue.
    let out = rgb_of(&square_thumb(&png_rgb(&two_tone(600, 300)), CACHE_SIDE).unwrap());
    assert_eq!(out.dimensions(), (CACHE_SIDE, CACHE_SIDE));
    assert!(is_red(out.get_pixel(10, 128)));
    assert!(is_blue(out.get_pixel(245, 128)));

    // Smaller than the side: not scaled up.
    let out = rgb_of(&square_thumb(&png_rgb(&two_tone(50, 80)), CACHE_SIDE).unwrap());
    assert_eq!(out.dimensions(), (50, 50));

    assert_eq!(square_thumb(&png_rgb(&two_tone(8, 8)), 0), Err(AvatarError::BadCrop));
}

#[test]
fn square_thumb_applies_exif_orientation() {
    let bytes = jpeg_with_orientation(&two_tone(80, 40), 6);
    let out = rgb_of(&square_thumb(&bytes, 40).unwrap());
    // Upright 40x80; the center square spans y 20..60, red above blue.
    assert_eq!(out.dimensions(), (40, 40));
    assert!(is_red(out.get_pixel(20, 3)));
    assert!(is_blue(out.get_pixel(20, 36)));
}

#[test]
fn card_thumb_fits_twelve_kilobytes() {
    for img in [noise(800, 800), noise(161, 400), two_tone(1000, 700)] {
        let card = card_thumb(&png_rgb(&img)).unwrap();
        assert!(card.len() <= CARD_MAX_BYTES, "{} bytes", card.len());
        assert_clean_jpeg(&card);
        let out = rgb_of(&card);
        assert_eq!(out.width(), out.height());
        assert!(out.width() <= CARD_SIDE);
    }
    let out = rgb_of(&card_thumb(&png_rgb(&two_tone(1000, 700))).unwrap());
    assert_eq!(out.dimensions(), (CARD_SIDE, CARD_SIDE));
}

#[test]
fn a_large_photo_is_handled() {
    let bytes = jpeg_of(&two_tone(2400, 1600));
    let decoded = decode(&bytes).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (2400, 1600));
    // Many times the target: brought near it by area averaging first.
    let full = CropRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
    let out = rgb_of(&crop_square(&decoded, full, 128).unwrap());
    assert_eq!(out.dimensions(), (128, 128));
    assert!(is_red(out.get_pixel(10, 64)));
    assert!(is_blue(out.get_pixel(118, 64)));
    let rect = CropRect { x: 0.1, y: 0.1, w: 0.6, h: 0.8 };
    let out = rgb_of(&crop_square(&decoded, rect, OWN_SIDE).unwrap());
    assert_eq!(out.dimensions(), (OWN_SIDE, OWN_SIDE));
    let out = rgb_of(&preview_jpeg(&decoded, 1024));
    assert_eq!(out.dimensions(), (1024, 683));
}

// Photos sent in a chat.

/// A PNG with an animation control chunk (`acTL`) right after its header.
fn apng_of(img: &RgbImage) -> Vec<u8> {
    let png = png_rgb(img);
    let ihdr_end = 8 + 12 + 13;
    let body = [0u32.to_be_bytes(), 0u32.to_be_bytes()].concat();
    let mut chunk = 8u32.to_be_bytes().to_vec();
    chunk.extend_from_slice(b"acTL");
    chunk.extend_from_slice(&body);
    chunk.extend_from_slice(&crc32(&chunk[4..]).to_be_bytes());
    [&png[..ihdr_end], &chunk, &png[ihdr_end..]].concat()
}

/// The start of a WebP with an extended header, its flags as given.
fn webp_extended(flags: u8) -> Vec<u8> {
    let mut webp = b"RIFF\x1a\0\0\0WEBPVP8X\x0a\0\0\0".to_vec();
    webp.push(flags);
    webp.extend_from_slice(&[0; 9]);
    webp
}

#[test]
fn a_photo_is_made_smaller_upright_and_clean() {
    let bytes = jpeg_with_orientation(&two_tone(2400, 1600), 6);
    let decoded = decode(&bytes).unwrap();
    let photo = photo_jpeg(&decoded, 1280, 78);
    assert_clean_jpeg(&photo);
    let out = rgb_of(&photo);
    // Upright 1600x2400, red above blue, its longer side 1280.
    assert_eq!(out.dimensions(), (853, 1280));
    assert!(is_red(out.get_pixel(426, 20)));
    assert!(is_blue(out.get_pixel(426, 1260)));
    assert!(photo.len() < bytes.len());

    // Never larger than the picture; a lower quality takes fewer bytes.
    let small = decode(&png_rgb(&noise(300, 200))).unwrap();
    assert_eq!(rgb_of(&photo_jpeg(&small, 1280, 78)).dimensions(), (300, 200));
    assert!(photo_jpeg(&small, 1280, 40).len() < photo_jpeg(&small, 1280, 95).len());
    // Sides and qualities out of range are brought into it.
    assert_eq!(rgb_of(&photo_jpeg(&small, 0, 0)).dimensions(), (1, 1));
}

#[test]
fn still_photos_are_told_from_moving_pictures_by_their_bytes() {
    let img = two_tone(30, 20);
    let mut webp = Vec::new();
    WebPEncoder::new_lossless(&mut webp)
        .write_image(img.as_raw(), 30, 20, ExtendedColorType::Rgb8)
        .unwrap();
    let mut gif = Vec::new();
    GifEncoder::new(&mut gif)
        .encode_frame(Frame::new(RgbaImage::from_pixel(4, 4, Rgba([0, 0, 0, 255]))))
        .unwrap();
    let mut bmp = Vec::new();
    BmpEncoder::new(&mut bmp)
        .write_image(img.as_raw(), 30, 20, ExtendedColorType::Rgb8)
        .unwrap();

    assert!(still_photo(&jpeg_of(&img)));
    assert!(still_photo(&png_rgb(&img)));
    assert!(still_photo(&webp));
    assert!(!still_photo(&webp_extended(0x10)), "an extended WebP with alpha keeps it");
    assert!(still_photo(&webp_extended(0x00)), "an extended WebP, opaque and still");
    assert!(!still_photo(&apng_of(&img)), "an animated PNG");
    assert!(!still_photo(&webp_extended(0x02)), "an animated WebP");
    assert!(!still_photo(&gif));
    assert!(!still_photo(&bmp));
    assert!(!still_photo(b"not a picture at all"));
    assert!(!still_photo(&[]));
}

/// A picture that can be seen through is never made a JPEG: that would
/// turn it white. Opaque, the same picture is a photo.
#[test]
fn a_picture_that_can_be_seen_through_is_no_photo_to_make_smaller() {
    let clear = RgbaImage::from_fn(30, 20, |x, _| Rgba([200, 30, 30, if x < 15 { 0 } else { 255 }]));
    let mut webp = Vec::new();
    WebPEncoder::new_lossless(&mut webp)
        .write_image(clear.as_raw(), 30, 20, ExtendedColorType::Rgba8)
        .unwrap();
    assert!(!still_photo(&png_rgba(&clear)), "a PNG with alpha");
    assert!(!still_photo(&webp), "a WebP with alpha");
    assert!(still_photo(&png_rgb(&two_tone(30, 20))), "the same, opaque");
}

#[test]
fn the_upright_size_is_read_from_the_header() {
    assert_eq!(upright_size(&png_rgb(&two_tone(40, 20))), Some((40, 20)));
    assert_eq!(upright_size(&jpeg_with_orientation(&two_tone(40, 20), 6)), Some((20, 40)));
    assert_eq!(upright_size(&jpeg_with_orientation(&two_tone(40, 20), 1)), Some((40, 20)));
    // The first bytes are enough.
    let big = jpeg_of(&noise(1200, 800));
    assert_eq!(upright_size(&big[..4096]), Some((1200, 800)));
    assert_eq!(upright_size(b"not a picture at all"), None);
    assert_eq!(upright_size(&[]), None);
}

// Hashes and URLs.

#[test]
fn sha256_hex_is_lowercase_hex() {
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(sha256_hex(b"").len(), 64);
}

#[test]
fn data_url_carries_the_jpeg() {
    let jpeg = square_thumb(&png_rgb(&two_tone(8, 8)), 8).unwrap();
    let url = data_url(&jpeg);
    let body = url.strip_prefix("data:image/jpeg;base64,").unwrap();
    let back = base64::engine::general_purpose::STANDARD.decode(body).unwrap();
    assert_eq!(back, jpeg);
}
