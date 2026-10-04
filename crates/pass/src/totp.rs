// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! TOTP entries: the secret stays in the backend, the codes leave it.

use crate::{directory, KIND_TOTP};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Sqlite, Transaction};
use totp_rs::{Algorithm, Secret, TOTP};
use uuid::Uuid;
use veydan_core::{AppError, BoxFuture, CmdResult, Core};
use veydan_sync_host::{Deletion, Host};

#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct TotpEntry {
    pub id: String,
    pub name: String,
    pub issuer: Option<String>,
    // secret is never returned to frontend — see TotpEntryPublic
    #[serde(skip_serializing)]
    pub secret: String,
    pub algorithm: String,
    pub digits: i64,
    pub period: i64,
    pub tags: String, // JSON array
    pub created_at: String,
    pub updated_at: String,
    pub last_used_at: Option<String>,
}

/// Public representation — no secret field
#[derive(Debug, Serialize, Clone)]
pub struct TotpEntryPublic {
    pub id: String,
    pub name: String,
    pub issuer: Option<String>,
    pub algorithm: String,
    pub digits: i64,
    pub period: i64,
    pub tags: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_used_at: Option<String>,
}

impl TotpEntry {
    fn to_public(&self) -> Result<TotpEntryPublic, serde_json::Error> {
        let tags: Vec<String> = serde_json::from_str(&self.tags)?;
        Ok(TotpEntryPublic {
            id: self.id.clone(),
            name: self.name.clone(),
            issuer: self.issuer.clone(),
            algorithm: self.algorithm.clone(),
            digits: self.digits,
            period: self.period,
            tags,
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
            last_used_at: self.last_used_at.clone(),
        })
    }
}

#[derive(Debug, Serialize)]
pub struct TotpCode {
    pub id: String,
    pub code: String,
    pub seconds_left: u64,
}

#[derive(Debug, Serialize)]
pub struct TotpPreview {
    pub name: String,
    pub issuer: Option<String>,
    pub secret_masked: String,
    pub algorithm: String,
    pub digits: u32,
    pub period: u64,
}

fn parse_algorithm(alg: &str) -> Algorithm {
    match alg.to_uppercase().as_str() {
        "SHA256" => Algorithm::SHA256,
        "SHA512" => Algorithm::SHA512,
        _ => Algorithm::SHA1,
    }
}

/// Normalise a user-supplied secret for storage.
/// Strips spaces, dashes, colons; uppercases.
/// Tries Base32 decode first, then hex — stores the raw cleaned string.
/// build_totp later decodes the same way via decode_secret.
fn normalize_secret(raw: &str) -> Result<String, AppError> {
    let cleaned: String = raw
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != ':')
        .collect::<String>()
        .to_uppercase();

    if cleaned.is_empty() {
        return Err(AppError::other("Secret cannot be empty"));
    }

    // Try Base32 — works if all chars are A-Z / 2-7
    if Secret::Encoded(cleaned.clone()).to_bytes().is_ok() {
        return Ok(cleaned);
    }

    // Try hex — even length, all 0-9 / A-F
    if cleaned.len().is_multiple_of(2) {
        let hex_ok = (0..cleaned.len())
            .step_by(2)
            .all(|i| u8::from_str_radix(&cleaned[i..i + 2], 16).is_ok());
        if hex_ok {
            return Ok(cleaned);
        }
    }

    Err(AppError::other(
        "Invalid secret: not recognised as Base32 or hex. Check the key and try again.",
    ))
}

/// Decode a stored secret (Base32 or HEX) to raw bytes.
fn decode_secret(s: &str) -> Result<Vec<u8>, AppError> {
    // Try Base32 first
    if let Ok(bytes) = Secret::Encoded(s.to_string()).to_bytes() {
        return Ok(bytes);
    }
    // Try hex
    if s.len().is_multiple_of(2) && s.chars().all(|c| c.is_ascii_hexdigit()) {
        let bytes: Result<Vec<u8>, _> = (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16))
            .collect();
        if let Ok(b) = bytes {
            return Ok(b);
        }
    }
    Err(AppError::other("Could not decode TOTP secret"))
}

/// `new_unchecked`: TOTP::new rejects secrets under 128 bits, but many
/// services issue 80-bit (16-char Base32) secrets that work fine.
pub(crate) fn build_totp(entry: &TotpEntry) -> Result<TOTP, AppError> {
    if !(6..=8).contains(&entry.digits) {
        return Err(AppError::other("Digits must be between 6 and 8"));
    }
    if entry.period < 1 {
        return Err(AppError::other("Period must be positive"));
    }
    Ok(TOTP::new_unchecked(
        parse_algorithm(&entry.algorithm),
        entry.digits as usize,
        1,
        entry.period as u64,
        decode_secret(&entry.secret)?,
        entry.issuer.clone(),
        entry.name.clone(),
    ))
}

/// The code of `entry` at the Unix time `unix`.
pub(crate) fn code_at(entry: &TotpEntry, unix: u64) -> Result<String, AppError> {
    Ok(build_totp(entry)?.generate(unix))
}

fn generate_code_for(entry: &TotpEntry) -> Result<TotpCode, AppError> {
    let totp = build_totp(entry)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| AppError::other(e.to_string()))?;

    let code = totp.generate(now.as_secs());

    let step = entry.period as u64;
    let elapsed = now.as_secs() % step;
    let seconds_left = step - elapsed;

    Ok(TotpCode {
        id: entry.id.clone(),
        code,
        seconds_left,
    })
}

#[tauri::command]
pub async fn totp_list(core: tauri::State<'_, Core>) -> CmdResult<Vec<TotpEntryPublic>> {
    let rows =
        sqlx::query_as::<_, TotpEntry>("SELECT * FROM totp_entries ORDER BY created_at DESC")
            .fetch_all(&core.db)
            .await
            .map_err(AppError::db)?;

    rows.iter()
        .map(|r| r.to_public().map_err(AppError::other))
        .collect()
}

#[derive(Debug, Deserialize)]
pub struct TotpAddRequest {
    pub name: String,
    pub issuer: Option<String>,
    /// Raw base32 secret OR full otpauth:// URI (uri takes priority)
    pub secret: Option<String>,
    pub uri: Option<String>,
    pub algorithm: Option<String>,
    pub digits: Option<i64>,
    pub period: Option<i64>,
    pub tags: Vec<String>,
}

#[tauri::command]
pub async fn totp_add(
    req: TotpAddRequest,
    core: tauri::State<'_, Core>,
) -> CmdResult<TotpEntryPublic> {
    let (name, issuer, secret, algorithm, digits, period) = if let Some(uri) = &req.uri {
        let totp = TOTP::from_url(uri).map_err(|e| AppError::other(format!("Invalid URI: {e}")))?;
        let secret_b32 = totp.get_secret_base32();
        let alg = match totp.algorithm {
            Algorithm::SHA256 => "SHA256",
            Algorithm::SHA512 => "SHA512",
            _ => "SHA1",
        };
        let issuer = totp.issuer.clone();
        let account = totp.account_name.clone();
        (
            if req.name.is_empty() {
                account
            } else {
                req.name.clone()
            },
            issuer,
            secret_b32,
            alg.to_string(),
            totp.digits as i64,
            totp.step as i64,
        )
    } else {
        let raw = req
            .secret
            .ok_or_else(|| AppError::other("secret or uri required"))?;
        let normalized = normalize_secret(&raw)?;
        (
            req.name.clone(),
            req.issuer.clone(),
            normalized,
            req.algorithm.unwrap_or_else(|| "SHA1".into()),
            req.digits.unwrap_or(6),
            req.period.unwrap_or(30),
        )
    };

    // Validate secret can build a valid TOTP
    let test_entry = TotpEntry {
        id: String::new(),
        name: name.clone(),
        issuer: issuer.clone(),
        secret: secret.clone(),
        algorithm: algorithm.clone(),
        digits,
        period,
        tags: "[]".into(),
        created_at: String::new(),
        updated_at: String::new(),
        last_used_at: None,
    };
    build_totp(&test_entry)?;

    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let tags_json = serde_json::to_string(&req.tags).map_err(AppError::other)?;

    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    sqlx::query(
        "INSERT INTO totp_entries (id, name, issuer, secret, algorithm, digits, period, tags, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&name)
    .bind(&issuer)
    .bind(&secret)
    .bind(&algorithm)
    .bind(digits)
    .bind(period)
    .bind(&tags_json)
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(AppError::db)?;
    directory::relabel(&core, &mut tx, KIND_TOTP, &id).await?;
    tx.commit().await.map_err(AppError::db)?;

    Ok(TotpEntryPublic {
        id,
        name,
        issuer,
        algorithm,
        digits,
        period,
        tags: req.tags,
        created_at: now.clone(),
        updated_at: now,
        last_used_at: None,
    })
}

#[derive(Debug, Deserialize)]
pub struct TotpUpdateRequest {
    pub name: Option<String>,
    pub issuer: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[tauri::command]
pub async fn totp_update(
    id: String,
    req: TotpUpdateRequest,
    core: tauri::State<'_, Core>,
) -> CmdResult<TotpEntryPublic> {
    let now = Utc::now().to_rfc3339();

    let mut entry = sqlx::query_as::<_, TotpEntry>("SELECT * FROM totp_entries WHERE id = ?")
        .bind(&id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found(format!("TOTP entry {id}")))?;

    if let Some(name) = req.name {
        entry.name = name;
    }
    if let Some(issuer) = req.issuer {
        entry.issuer = if issuer.is_empty() {
            None
        } else {
            Some(issuer)
        };
    }
    if let Some(tags) = &req.tags {
        entry.tags = serde_json::to_string(tags).map_err(AppError::other)?;
    }
    entry.updated_at = now;

    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    sqlx::query("UPDATE totp_entries SET name=?, issuer=?, tags=?, updated_at=? WHERE id=?")
        .bind(&entry.name)
        .bind(&entry.issuer)
        .bind(&entry.tags)
        .bind(&entry.updated_at)
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;
    directory::relabel(&core, &mut tx, KIND_TOTP, &id).await?;
    tx.commit().await.map_err(AppError::db)?;

    entry.to_public().map_err(AppError::other)
}

#[tauri::command]
pub async fn totp_delete(id: String, core: tauri::State<'_, Core>) -> CmdResult<()> {
    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    delete(&core, &mut tx, &id).await?;
    // Drop the id from every password that linked it
    let needle = serde_json::to_string(&id).map_err(AppError::other)?;
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT id, totp_ids FROM passwords WHERE instr(totp_ids, ?) > 0")
            .bind(&needle)
            .fetch_all(&mut *tx)
            .await
            .map_err(AppError::db)?;
    for (pw_id, raw) in rows {
        let mut ids: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
        ids.retain(|x| x != &id);
        sqlx::query("UPDATE passwords SET totp_ids = ? WHERE id = ?")
            .bind(serde_json::to_string(&ids).map_err(AppError::other)?)
            .bind(&pw_id)
            .execute(&mut *tx)
            .await
            .map_err(AppError::db)?;
    }
    tx.commit().await.map_err(AppError::db)?;
    Ok(())
}

/// Delete the entry `id`, report it on the deletion hooks and retract its
/// label, in `tx`.
async fn delete(core: &Core, tx: &mut Transaction<'_, Sqlite>, id: &str) -> Result<(), AppError> {
    let n = sqlx::query("DELETE FROM totp_entries WHERE id = ?")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(AppError::db)?;
    if n.rows_affected() > 0 {
        core.deletions.emit(tx, KIND_TOTP, id).await?;
    }
    directory::relabel(core, tx, KIND_TOTP, id).await
}

/// A tombstone from another device: the entry goes and is reported. The
/// passwords keep their links, as 4.0.7 left them: the device that deleted
/// the entry sends the passwords it changed.
pub(crate) fn delete_synced<'a>(
    _host: &'a Host<'a>,
    core: &'a Core,
    id: &'a str,
) -> BoxFuture<'a, CmdResult<Deletion>> {
    Box::pin(async move {
        let mut tx = core.db.begin().await.map_err(AppError::db)?;
        delete(core, &mut tx, id).await?;
        tx.commit().await.map_err(AppError::db)?;
        Ok(Deletion::Done)
    })
}

#[tauri::command]
pub async fn totp_generate_code(id: String, core: tauri::State<'_, Core>) -> CmdResult<TotpCode> {
    let entry = sqlx::query_as::<_, TotpEntry>("SELECT * FROM totp_entries WHERE id = ?")
        .bind(&id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found(format!("TOTP entry {id}")))?;

    let code = generate_code_for(&entry)?;

    // Update last_used_at
    let now = Utc::now().to_rfc3339();
    let _ = sqlx::query("UPDATE totp_entries SET last_used_at=? WHERE id=?")
        .bind(&now)
        .bind(&id)
        .execute(&core.db)
        .await;

    Ok(code)
}

#[tauri::command]
pub async fn totp_generate_codes(
    ids: Vec<String>,
    core: tauri::State<'_, Core>,
) -> CmdResult<Vec<TotpCode>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }

    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!("SELECT * FROM totp_entries WHERE id IN ({})", placeholders);

    // Safe: only `?` placeholders are interpolated; ids are bound below.
    let mut q = sqlx::query_as::<_, TotpEntry>(sqlx::AssertSqlSafe(sql));
    for id in &ids {
        q = q.bind(id);
    }
    let rows = q.fetch_all(&core.db).await.map_err(AppError::db)?;

    Ok(rows
        .iter()
        .filter_map(|row| generate_code_for(row).ok())
        .collect())
}

#[tauri::command]
pub async fn totp_preview_uri(uri: String) -> CmdResult<TotpPreview> {
    let totp =
        TOTP::from_url(&uri).map_err(|e| AppError::other(format!("Invalid otpauth URI: {e}")))?;

    let secret_b32 = totp.get_secret_base32();
    let masked = if secret_b32.len() > 4 {
        format!(
            "{}…{}",
            &secret_b32[..2],
            &secret_b32[secret_b32.len() - 2..]
        )
    } else {
        "••••".to_string()
    };

    let alg = match totp.algorithm {
        Algorithm::SHA256 => "SHA256",
        Algorithm::SHA512 => "SHA512",
        _ => "SHA1",
    };

    Ok(TotpPreview {
        name: totp.account_name,
        issuer: totp.issuer,
        secret_masked: masked,
        algorithm: alg.to_string(),
        digits: totp.digits as u32,
        period: totp.step,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(secret: &str, algorithm: &str, digits: i64) -> TotpEntry {
        TotpEntry {
            id: "t".into(),
            name: "test".into(),
            issuer: None,
            secret: normalize_secret(secret).unwrap(),
            algorithm: algorithm.into(),
            digits,
            period: 30,
            tags: "[]".into(),
            created_at: String::new(),
            updated_at: String::new(),
            last_used_at: None,
        }
    }

    /// RFC 6238, Appendix B: the seeds as hex, eight digits, steps of 30 s.
    #[test]
    fn the_codes_of_rfc_6238() {
        let sha1 = "31323334353637383930".repeat(2);
        let sha256 = format!("{}{}", "31323334353637383930".repeat(3), "3132");
        let sha512 = format!("{}{}", "31323334353637383930".repeat(6), "31323334");
        let vectors: [(u64, &str, &str, &str); 6] = [
            (59, "94287082", "46119246", "90693936"),
            (1111111109, "07081804", "68084774", "25091201"),
            (1111111111, "14050471", "67062674", "99943326"),
            (1234567890, "89005924", "91819424", "93441116"),
            (2000000000, "69279037", "90698825", "38618901"),
            (20000000000, "65353130", "77737706", "47863826"),
        ];
        for (time, one, two_five_six, five_twelve) in vectors {
            assert_eq!(
                code_at(&entry(&sha1, "SHA1", 8), time).unwrap(),
                one,
                "{time}"
            );
            assert_eq!(
                code_at(&entry(&sha256, "SHA256", 8), time).unwrap(),
                two_five_six,
                "{time}"
            );
            assert_eq!(
                code_at(&entry(&sha512, "sha512", 8), time).unwrap(),
                five_twelve,
                "{time}"
            );
        }
    }

    /// The SHA-1 seed of the RFC as Base32, the way services hand secrets out,
    /// with the spaces and the lower case of a secret typed by hand.
    #[test]
    fn a_base32_secret_gives_the_same_codes() {
        let typed = entry("gezd gnbv gy3t qojq gezd gnbv gy3t qojq", "SHA1", 8);
        assert_eq!(typed.secret, "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
        assert_eq!(code_at(&typed, 59).unwrap(), "94287082");
        // Six digits are the last six of eight.
        assert_eq!(
            code_at(
                &entry("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", "SHA1", 6),
                1111111109
            )
            .unwrap(),
            "081804"
        );
    }

    #[test]
    fn a_secret_or_a_shape_that_cannot_make_codes_is_refused() {
        assert!(normalize_secret("").is_err());
        assert!(normalize_secret("not a secret!").is_err());
        let mut short = entry("JBSWY3DPEHPK3PXP", "SHA1", 6);
        short.digits = 5;
        assert!(build_totp(&short).is_err());
        short.digits = 6;
        short.period = 0;
        assert!(build_totp(&short).is_err());
    }
}
