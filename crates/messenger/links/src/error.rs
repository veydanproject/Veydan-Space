// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use messenger_core::MessengerError;

/// Why a text is not a link. The code is stable: the UI translates it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkError {
    /// Does not start with `veydan://`.
    Scheme,
    /// Longer than a link may be.
    TooLong,
    /// Control characters or spaces inside.
    Characters,
    /// The type is missing or is not a word of small latin letters.
    Type,
    /// The identifier is missing or is not what this type expects.
    Id,
    /// A parameter is given twice, badly encoded, or has a value this type refuses.
    Param,
    /// A parameter this type cannot do without is absent.
    Missing,
}

impl LinkError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Scheme => "link_bad_scheme",
            Self::TooLong => "link_too_long",
            Self::Characters => "link_bad_characters",
            Self::Type => "link_bad_type",
            Self::Id => "link_bad_id",
            Self::Param => "link_bad_param",
            Self::Missing => "link_missing_param",
        }
    }
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for LinkError {}

impl From<LinkError> for MessengerError {
    fn from(e: LinkError) -> Self {
        MessengerError::Invalid(e.code().into())
    }
}
