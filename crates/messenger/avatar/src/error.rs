// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Refusals of the avatar crate, as stable codes for the UI.

use std::fmt;

use image::ImageError;

/// Why a picture was not taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AvatarError {
    /// More bytes or more pixels than an avatar may come from.
    TooLarge,
    /// Not one of the formats we read (JPEG, PNG, WebP, GIF, BMP).
    Unsupported,
    /// One of those formats, but damaged or cut short.
    Corrupt,
    /// The crop rectangle is not a part of the picture.
    BadCrop,
}

impl AvatarError {
    /// The stable code the UI translates.
    pub fn code(&self) -> &'static str {
        match self {
            AvatarError::TooLarge => "avatar_too_large",
            AvatarError::Unsupported => "avatar_unsupported",
            AvatarError::Corrupt => "avatar_corrupt",
            AvatarError::BadCrop => "avatar_bad_crop",
        }
    }
}

impl fmt::Display for AvatarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for AvatarError {}

impl From<ImageError> for AvatarError {
    fn from(err: ImageError) -> Self {
        match err {
            ImageError::Limits(_) => AvatarError::TooLarge,
            ImageError::Unsupported(_) => AvatarError::Unsupported,
            ImageError::Decoding(_)
            | ImageError::Encoding(_)
            | ImageError::Parameter(_)
            | ImageError::IoError(_) => AvatarError::Corrupt,
        }
    }
}
