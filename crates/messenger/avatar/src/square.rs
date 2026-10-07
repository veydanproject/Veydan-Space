// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Squares, previews and photos made smaller, always written as a fresh JPEG.
//!
//! Every output is encoded here from pixels, so nothing of the source file
//! (EXIF, GPS, ICC, comments) can reach it.

use image::codecs::jpeg::JpegEncoder;
use image::imageops::{self, FilterType};
use image::RgbImage;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::decode::{decode_limited, orient, Budget, Decoded, CARD, OTHERS};
use crate::{AvatarError, CARD_MAX_BYTES, CARD_SIDE};

/// The quality of every square and preview.
const QUALITY: u8 = 85;

/// The largest preview the crop UI gets.
const MAX_PREVIEW_SIDE: u32 = 1024;

/// The largest side a photo made smaller may ask for.
const MAX_PHOTO_SIDE: u32 = 4096;

/// The largest square anyone may ask for.
const MAX_SQUARE_SIDE: u32 = 2048;

/// How much a crop rectangle may reach past the picture for rounding.
const EPSILON: f64 = 1e-6;

/// Qualities tried in turn for a card, until the JPEG fits.
const CARD_QUALITIES: [u8; 7] = [80, 70, 60, 50, 40, 30, 20];

/// Smaller sides tried for a card when even the lowest quality is too big.
const CARD_FALLBACK_SIDES: [u32; 3] = [128, 96, 64];

/// A part of the picture, as fractions 0..1 of the upright picture.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct CropRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// A JPEG of the whole picture, its longer side at most `max_side` (and at
/// most 1024), never larger than the picture itself.
pub fn preview_jpeg(img: &Decoded, max_side: u32) -> Vec<u8> {
    whole_jpeg(img, max_side.clamp(1, MAX_PREVIEW_SIDE), QUALITY)
}

/// A photo made smaller to be sent in a chat: a JPEG of the whole picture
/// at `quality` (1..=100), its longer side at most `max_side` (and at most
/// 4096), never larger than the picture itself. Only pixels are written:
/// no EXIF, location or comment of the source survives.
pub fn photo_jpeg(img: &Decoded, max_side: u32, quality: u8) -> Vec<u8> {
    whole_jpeg(img, max_side.clamp(1, MAX_PHOTO_SIDE), quality.clamp(1, 100))
}

/// The whole picture as a JPEG, its longer side at most `max_side`.
fn whole_jpeg(img: &Decoded, max_side: u32, quality: u8) -> Vec<u8> {
    let (w, h) = (img.width(), img.height());
    let longer = w.max(h);
    if longer <= max_side {
        return jpeg(&img.rgb, quality);
    }
    let scale = f64::from(max_side) / f64::from(longer);
    let tw = ((f64::from(w) * scale).round() as u32).clamp(1, max_side);
    let th = ((f64::from(h) * scale).round() as u32).clamp(1, max_side);
    jpeg(&shrink(&img.rgb, tw, th), quality)
}

/// The part `rect` of the picture, made square around its center and
/// scaled to `side` by `side`.
pub fn crop_square(img: &Decoded, rect: CropRect, side: u32) -> Result<Vec<u8>, AvatarError> {
    if side == 0 || side > MAX_SQUARE_SIDE {
        return Err(AvatarError::BadCrop);
    }
    let CropRect { x, y, w, h } = rect;
    let finite = [x, y, w, h].iter().all(|v| v.is_finite());
    if !finite
        || x < -EPSILON
        || y < -EPSILON
        || w <= 0.0
        || h <= 0.0
        || x + w > 1.0 + EPSILON
        || y + h > 1.0 + EPSILON
    {
        return Err(AvatarError::BadCrop);
    }

    let (iw, ih) = (f64::from(img.width()), f64::from(img.height()));
    let (pw, ph) = (w * iw, h * ih);
    let size = pw.min(ph);
    if size < 1.0 {
        return Err(AvatarError::BadCrop);
    }
    let size = (size.round() as u32).clamp(1, img.width().min(img.height()));
    let half = f64::from(size) / 2.0;
    let cx = x.max(0.0) * iw + pw / 2.0;
    let cy = y.max(0.0) * ih + ph / 2.0;
    let left = (cx - half).round().clamp(0.0, f64::from(img.width() - size)) as u32;
    let top = (cy - half).round().clamp(0.0, f64::from(img.height() - size)) as u32;

    let part = imageops::crop_imm(&img.rgb, left, top, size, size).to_image();
    Ok(jpeg(&scale_square(&part, side), QUALITY))
}

/// Any picture as a square of its center, `side` by `side` or the size of
/// that square when the picture is smaller (never scaled up). Made for the
/// avatars of others, so decoded within their smaller budget.
pub fn square_thumb(bytes: &[u8], side: u32) -> Result<Vec<u8>, AvatarError> {
    if side == 0 || side > MAX_SQUARE_SIDE {
        return Err(AvatarError::BadCrop);
    }
    Ok(jpeg(&thumb(bytes, OTHERS, side)?, QUALITY))
}

/// A square of `CARD_SIDE` for a contact card, at the best quality that
/// keeps it within `CARD_MAX_BYTES`. Its input is a small picture (mine,
/// a cached avatar, or the one inside a received card), so it is decoded
/// within the smallest budget.
pub fn card_thumb(bytes: &[u8]) -> Result<Vec<u8>, AvatarError> {
    let square = thumb(bytes, CARD, CARD_SIDE)?;
    for quality in CARD_QUALITIES {
        let out = jpeg(&square, quality);
        if out.len() <= CARD_MAX_BYTES {
            return Ok(out);
        }
    }
    let lowest = CARD_QUALITIES[CARD_QUALITIES.len() - 1];
    for side in CARD_FALLBACK_SIDES {
        if side >= square.width() {
            continue;
        }
        let out = jpeg(&scale_square(&square, side), lowest);
        if out.len() <= CARD_MAX_BYTES {
            return Ok(out);
        }
    }
    Err(AvatarError::TooLarge)
}

/// The upright center square of a picture, at most `side`. The square is
/// cut and scaled as stored and only then turned, so a rotation copies the
/// small square instead of the whole picture.
fn thumb(bytes: &[u8], budget: Budget, side: u32) -> Result<RgbImage, AvatarError> {
    let (rgb, orientation) = decode_limited(bytes, budget, false)?;
    Ok(orient(center_square(&rgb, side), orientation))
}

/// The center square of the picture, scaled down to `side` if larger.
fn center_square(img: &RgbImage, side: u32) -> RgbImage {
    let (w, h) = img.dimensions();
    let size = w.min(h);
    let part = imageops::crop_imm(img, (w - size) / 2, (h - size) / 2, size, size).to_image();
    if size <= side {
        part
    } else {
        scale_square(&part, side)
    }
}

/// A square scaled to `side` by `side`, up or down.
fn scale_square(square: &RgbImage, side: u32) -> RgbImage {
    if square.width() == side && square.height() == side {
        return square.clone();
    }
    if square.width() > side {
        shrink(square, side, side)
    } else {
        imageops::resize(square, side, side, FilterType::Lanczos3)
    }
}

/// Scales down with Lanczos3; a picture many times larger than the target
/// is first brought near it by fast area averaging, which looks the same
/// and saves seconds on a large photo.
fn shrink(src: &RgbImage, width: u32, height: u32) -> RgbImage {
    if src.width() > width.saturating_mul(4) && src.height() > height.saturating_mul(4) {
        let near = imageops::thumbnail(src, width * 2, height * 2);
        imageops::resize(&near, width, height, FilterType::Lanczos3)
    } else {
        imageops::resize(src, width, height, FilterType::Lanczos3)
    }
}

/// A baseline JPEG of the pixels and nothing else.
fn jpeg(img: &RgbImage, quality: u8) -> Vec<u8> {
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, quality)
        .encode_image(img)
        .expect("an RGB picture within the limits of decoding encodes into memory");
    out
}
