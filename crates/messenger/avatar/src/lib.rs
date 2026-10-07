// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Avatar pictures.
//!
//! A picture the user picks, or one fetched for somebody else, comes in
//! as bytes of any common format and leaves as a small square JPEG that
//! was encoded here from pixels alone: no EXIF, no location, no comments
//! of the source survive. Decoding is bounded by the size of the file, by
//! the pixels the header declares and by the memory of the whole way, in a
//! budget that depends on where the picture comes from, all checked before
//! decoding.
//!
//! A photo sent in a chat takes the same way when it is made smaller
//! (`photo_jpeg`): decoded within my own budget and written as a fresh
//! JPEG of its pixels.
//!
//! - `decode`: bytes to an upright RGB picture; what a header says.
//! - `square`: the crop of my own avatar, thumbnails of others, the tiny
//!   picture of a contact card, the preview of the crop UI and photos
//!   made smaller.
//! - `error`: refusals, as stable codes.
//!
//! Everything in this crate is pure: no network, no storage, no clock.

mod decode;
mod error;
mod square;

#[cfg(test)]
mod tests;

use base64::Engine;
use sha2::{Digest, Sha256};

pub use decode::{decode, still_photo, upright_size, Decoded};
pub use error::AvatarError;
pub use square::{card_thumb, crop_square, photo_jpeg, preview_jpeg, square_thumb, CropRect};

/// The largest file a picture is read from.
pub const MAX_INPUT_BYTES: usize = 40 * 1024 * 1024;
/// The most pixels a picture I pick myself may declare; the avatars of
/// others and the pictures of contact cards get far less.
pub const MAX_PIXELS: u64 = 50_000_000;
/// The side of my own avatar as uploaded.
pub const OWN_SIDE: u32 = 512;
/// The side of a cached avatar of somebody else.
pub const CACHE_SIDE: u32 = 256;
/// The side of the picture inside a contact card.
pub const CARD_SIDE: u32 = 160;
/// The most bytes the picture of a contact card may take.
pub const CARD_MAX_BYTES: usize = 12 * 1024;

/// SHA-256 of the bytes, lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// A `data:` URL of a JPEG, for the WebView.
pub fn data_url(jpeg: &[u8]) -> String {
    format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(jpeg)
    )
}
