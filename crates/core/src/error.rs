// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use serde::Serialize;

/// The error of every command. The frontend reads `code` and `message`, so
/// the variants and their serialized form are a contract.
#[derive(Debug, thiserror::Error, Serialize)]
#[serde(tag = "code", content = "message", rename_all = "snake_case")]
pub enum AppError {
    #[error("Database error: {0}")]
    Db(String),
    #[error("IO error: {0}")]
    Io(String),
    #[error("Browser error: {0}")]
    Browser(String),
    #[error("Proxy error: {0}")]
    Proxy(String),
    #[error("Not found: {0}")]
    NotFound(String),
    /// The conflict the caller resolved no longer matches the stored one.
    #[error("Conflict changed: {0}")]
    ConflictChanged(String),
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

/// Standard result type for all Tauri commands.
pub type CmdResult<T> = Result<T, AppError>;

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        Self::Other(e.to_string())
    }
}

/// The lock fails with the codes and texts the commands had before it was a
/// crate of its own.
impl From<veydan_lock::Error> for AppError {
    fn from(e: veydan_lock::Error) -> Self {
        use veydan_lock::Error as Lock;
        match e {
            Lock::Db(m) => Self::Db(m),
            Lock::NotFound(m) => Self::NotFound(m),
            Lock::VaultLocked => Self::VaultLocked,
            Lock::VaultMismatch => Self::VaultMismatch,
            Lock::DecryptFailed => Self::DecryptFailed,
            Lock::RecoveryInvalid => Self::RecoveryInvalid,
            Lock::Other(m) => Self::Other(m),
        }
    }
}

impl AppError {
    pub fn db(e: impl std::fmt::Display) -> Self {
        Self::Db(e.to_string())
    }
    pub fn io(e: impl std::fmt::Display) -> Self {
        Self::Io(e.to_string())
    }
    pub fn browser(e: impl std::fmt::Display) -> Self {
        Self::Browser(e.to_string())
    }
    pub fn proxy(e: impl std::fmt::Display) -> Self {
        Self::Proxy(e.to_string())
    }
    pub fn not_found(e: impl std::fmt::Display) -> Self {
        Self::NotFound(e.to_string())
    }
    pub fn conflict_changed(e: impl std::fmt::Display) -> Self {
        Self::ConflictChanged(e.to_string())
    }
    pub fn other(e: impl std::fmt::Display) -> Self {
        Self::Other(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// What the frontend receives for each variant.
    #[test]
    fn every_variant_serializes_as_the_frontend_reads_it() {
        let cases = [
            (AppError::db("d"), json!({ "code": "db", "message": "d" })),
            (AppError::io("i"), json!({ "code": "io", "message": "i" })),
            (
                AppError::browser("b"),
                json!({ "code": "browser", "message": "b" }),
            ),
            (
                AppError::proxy("p"),
                json!({ "code": "proxy", "message": "p" }),
            ),
            (
                AppError::not_found("n"),
                json!({ "code": "not_found", "message": "n" }),
            ),
            (
                AppError::conflict_changed("c"),
                json!({ "code": "conflict_changed", "message": "c" }),
            ),
            (AppError::VaultLocked, json!({ "code": "vault_locked" })),
            (AppError::VaultMismatch, json!({ "code": "vault_mismatch" })),
            (AppError::DecryptFailed, json!({ "code": "decrypt_failed" })),
            (
                AppError::RecoveryInvalid,
                json!({ "code": "recovery_invalid" }),
            ),
            (
                AppError::other("o"),
                json!({ "code": "other", "message": "o" }),
            ),
        ];
        for (error, expected) in cases {
            // A variant added to the enum must be added to the list above.
            match error {
                AppError::Db(_)
                | AppError::Io(_)
                | AppError::Browser(_)
                | AppError::Proxy(_)
                | AppError::NotFound(_)
                | AppError::ConflictChanged(_)
                | AppError::VaultLocked
                | AppError::VaultMismatch
                | AppError::DecryptFailed
                | AppError::RecoveryInvalid
                | AppError::Other(_) => {}
            }
            assert_eq!(serde_json::to_value(&error).unwrap(), expected);
        }
    }

    #[test]
    fn foreign_errors_keep_their_codes() {
        let io = std::io::Error::other("disk");
        assert!(matches!(AppError::from(io), AppError::Io(m) if m == "disk"));
        let db = sqlx::Error::RowNotFound;
        assert!(matches!(AppError::from(db), AppError::Db(_)));
        let json = serde_json::from_str::<u8>("x").unwrap_err();
        assert!(matches!(AppError::from(json), AppError::Other(_)));
    }

    #[test]
    fn errors_of_the_lock_keep_their_codes_and_texts() {
        use veydan_lock::Error as Lock;
        let cases = [
            (Lock::db("d"), "db"),
            (Lock::not_found("recovery key"), "not_found"),
            (Lock::VaultLocked, "vault_locked"),
            (Lock::VaultMismatch, "vault_mismatch"),
            (Lock::DecryptFailed, "decrypt_failed"),
            (Lock::RecoveryInvalid, "recovery_invalid"),
            (Lock::other("Wrong password"), "other"),
        ];
        for (error, code) in cases {
            let text = error.to_string();
            let app = AppError::from(error);
            assert_eq!(app.to_string(), text);
            assert_eq!(serde_json::to_value(&app).unwrap()["code"], code);
        }
    }
}
