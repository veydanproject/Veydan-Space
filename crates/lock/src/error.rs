// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/// Why an operation of the lock failed. The core turns it into the error of
/// the commands variant by variant, with the same text: the frontend reads
/// the codes and shows some of the texts.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("Database error: {0}")]
    Db(String),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Vault is locked")]
    VaultLocked,
    #[error("Password vault cannot be unlocked with the current lock")]
    VaultMismatch,
    #[error("Could not decrypt this entry")]
    DecryptFailed,
    #[error("Recovery key does not match")]
    RecoveryInvalid,
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// The secret given is not the lock's: `unlock`, `verify` and the
    /// `current` of `set` say so with the texts [`crate::ERR_WRONG`] and
    /// [`crate::ERR_CURRENT`]; this is the one place that knows them.
    pub fn is_wrong_secret(&self) -> bool {
        matches!(self, Self::Other(m) if m == crate::ERR_WRONG || m == crate::ERR_CURRENT)
    }

    pub fn db(e: impl std::fmt::Display) -> Self {
        Self::Db(e.to_string())
    }
    pub fn not_found(e: impl std::fmt::Display) -> Self {
        Self::NotFound(e.to_string())
    }
    pub fn other(e: impl std::fmt::Display) -> Self {
        Self::Other(e.to_string())
    }
}
