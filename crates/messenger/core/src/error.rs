// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use serde::{Deserialize, Serialize};

/// Error shared by all messenger crates. Serializable so the host can forward
/// it to the UI unchanged; `code` is stable, `message` is for humans.
#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "code", content = "message", rename_all = "snake_case")]
pub enum MessengerError {
    #[error("storage error: {0}")]
    Storage(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("secrets are locked")]
    SecretsLocked,
    #[error("secret not found: {0}")]
    SecretMissing(String),
    #[error("crypto error: {0}")]
    Crypto(String),
    #[error("transport error: {0}")]
    Transport(String),
    #[error("not logged in")]
    NotLoggedIn,
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, MessengerError>;

impl From<std::io::Error> for MessengerError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

impl From<serde_json::Error> for MessengerError {
    fn from(e: serde_json::Error) -> Self {
        Self::Invalid(e.to_string())
    }
}
