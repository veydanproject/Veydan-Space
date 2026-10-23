// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A file made ready before its upload, the same for a chat, a group and
//! a channel: a photo made smaller unless the user asks for the original
//! (`PHOTO_SIDE`, `PHOTO_QUALITY`), the size a picture is shown at, the
//! small preview the message carries (`thumb`), and the frame of a video
//! the UI took (`Poster`). Only pixels go: nothing of the source survives
//! in what is made here.

use crate::descriptor::MAX_THUMB_BYTES;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use std::path::Path;

/// A photo is sent with its longer side at most this, unless the user
/// asks for the original; as the earlier app did.
pub const PHOTO_SIDE: u32 = 1280;
/// The JPEG quality of a photo made smaller.
pub const PHOTO_QUALITY: u8 = 78;
/// A photo larger than this is made smaller even when its sides are
/// within `PHOTO_SIDE`.
pub const PHOTO_MAX_BYTES: u64 = 1024 * 1024;
/// The first bytes of a picture read for its size.
const HEADER_BYTES: u64 = 512 * 1024;
/// The longer side of a preview carried in the message, tried smaller
/// until it fits `MAX_THUMB_BYTES`.
const THUMB_SIDES: [u32; 3] = [160, 96, 64];
/// A frame the UI hands over is never larger than this (base64).
pub const MAX_POSTER_B64: usize = 1024 * 1024;
/// Photos made smaller at once. Each holds its file and the decoded
/// picture in memory (hundreds of megabytes for a large one), and an album
/// reaches the preparing stage all together: the others wait in it. A
/// phone has less to spare.
pub const PHOTO_SLOTS: usize = if cfg!(any(target_os = "android", target_os = "ios")) { 1 } else { 2 };

/// A frame of a video the UI took from the picked file, shown before the
/// file is fetched: a JPEG (base64) and the size of the video.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Poster {
    pub jpeg: String,
    pub width: u32,
    pub height: u32,
}

/// A preview of a picture (any the avatar crate reads), written anew as a
/// small JPEG: base64 within `MAX_THUMB_BYTES`; `None` when it cannot be
/// read.
pub fn thumb_of(bytes: &[u8]) -> Option<String> {
    let img = messenger_avatar::decode(bytes).ok()?;
    THUMB_SIDES
        .into_iter()
        .map(|side| messenger_avatar::preview_jpeg(&img, side))
        .find(|j| j.len() <= MAX_THUMB_BYTES)
        .map(|j| B64.encode(j))
}

/// `thumb_of` the picture at `path`, within the avatar crate's limits.
pub fn thumb_of_file(path: &Path) -> Option<String> {
    if std::fs::metadata(path).ok()?.len() > messenger_avatar::MAX_INPUT_BYTES as u64 {
        return None;
    }
    thumb_of(&std::fs::read(path).ok()?)
}

/// What the frame of a video the UI took gives the message: its preview
/// (`thumb_of`) and the size of the video, each when it holds. It
/// decodes: called off the runtime's threads.
pub fn of_poster(p: &Poster) -> (Option<String>, Option<(u32, u32)>) {
    if p.jpeg.len() > MAX_POSTER_B64 {
        return (None, None);
    }
    let jpeg = B64.decode(p.jpeg.trim()).unwrap_or_default();
    let dim = ((1..=16384).contains(&p.width) && (1..=16384).contains(&p.height)).then_some((p.width, p.height));
    (thumb_of(&jpeg), dim)
}

/// The upright size of the picture at `path`, from its first bytes;
/// `None` when they do not tell it.
pub async fn picture_size(path: &Path) -> Option<(u32, u32)> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let mut head = Vec::new();
        std::fs::File::open(path).ok()?.take(HEADER_BYTES).read_to_end(&mut head).ok()?;
        messenger_avatar::upright_size(&head)
    })
    .await
    .ok()
    .flatten()
}

/// The photo at `path` made smaller, when that is worth it: a still JPEG,
/// PNG or WebP whose longer side is over `PHOTO_SIDE` or whose file is
/// over `PHOTO_MAX_BYTES`, written again as a JPEG that is smaller than
/// the file. The JPEG and its size; `None` to send the file as it is
/// (also when it cannot be decoded). It decodes: called off the
/// runtime's threads.
pub fn smaller_photo(path: &Path) -> Option<(Vec<u8>, (u32, u32))> {
    let len = std::fs::metadata(path).ok()?.len();
    if len > messenger_avatar::MAX_INPUT_BYTES as u64 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if !messenger_avatar::still_photo(&bytes) {
        return None;
    }
    let (w, h) = messenger_avatar::upright_size(&bytes)?;
    if w.max(h) <= PHOTO_SIDE && len <= PHOTO_MAX_BYTES {
        return None;
    }
    let img = messenger_avatar::decode(&bytes).ok()?;
    let jpeg = messenger_avatar::photo_jpeg(&img, PHOTO_SIDE, PHOTO_QUALITY);
    let size = messenger_avatar::upright_size(&jpeg)?;
    ((jpeg.len() as u64) < len).then_some((jpeg, size))
}
