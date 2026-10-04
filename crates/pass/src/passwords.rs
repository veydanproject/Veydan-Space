// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Password entries. Secrets are encrypted before they touch SQLite.

use crate::{directory, KIND_PASSWORD};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Sqlite, SqliteConnection, Transaction};
use uuid::Uuid;
use veydan_core::{AppError, BoxFuture, CmdResult, Core};
use veydan_lock::{KeyFuture, KeyUser, Lock, Rekey, SecretKey};
use veydan_sync_host::{Deletion, Host};
use zeroize::Zeroizing;

#[derive(FromRow)]
struct PasswordRow {
    id: String,
    title: String,
    username: Option<String>,
    url: Option<String>,
    password_enc: String,
    note_enc: Option<String>,
    /// JSON list of linked TOTP entry ids
    totp_ids: String,
    tags: String,
    vault_id: String,
    created_at: String,
    updated_at: String,
}

/// Metadata only. Ciphertext never leaves the backend through list/get.
#[derive(Serialize)]
pub struct PasswordPublic {
    pub id: String,
    pub title: String,
    pub username: Option<String>,
    pub url: Option<String>,
    pub has_note: bool,
    pub totp_ids: Vec<String>,
    pub tags: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize)]
pub struct RevealedSecret {
    pub value: String,
}

#[derive(Deserialize)]
pub struct PasswordCreate {
    pub title: String,
    pub username: Option<String>,
    pub url: Option<String>,
    pub password: String,
    pub note: Option<String>,
    pub totp_ids: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub struct PasswordUpdate {
    pub title: Option<String>,
    pub username: Option<String>,
    pub url: Option<String>,
    pub password: Option<String>,
    pub note: Option<String>,
    #[serde(default)]
    pub clear_note: bool,
    pub totp_ids: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
}

fn blank(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn parse_list(raw: &str) -> Vec<String> {
    serde_json::from_str(raw).unwrap_or_default()
}

/// Trimmed, non-empty, unique ids in the given order.
fn clean_ids(ids: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        let id = id.trim().to_string();
        if !id.is_empty() && !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

fn to_public(row: &PasswordRow) -> PasswordPublic {
    PasswordPublic {
        id: row.id.clone(),
        title: row.title.clone(),
        username: row.username.clone(),
        url: row.url.clone(),
        has_note: row.note_enc.as_ref().is_some_and(|s| !s.is_empty()),
        totp_ids: parse_list(&row.totp_ids),
        tags: parse_list(&row.tags),
        created_at: row.created_at.clone(),
        updated_at: row.updated_at.clone(),
    }
}

async fn load(core: &Core, id: &str) -> Result<PasswordRow, AppError> {
    sqlx::query_as::<_, PasswordRow>("SELECT * FROM passwords WHERE id = ?")
        .bind(id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("password"))
}

async fn totp_all_exist(core: &Core, ids: &[String]) -> Result<(), AppError> {
    for id in ids {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM totp_entries WHERE id = ?")
            .bind(id)
            .fetch_one(&core.db)
            .await
            .map_err(AppError::db)?;
        if n == 0 {
            return Err(AppError::not_found("TOTP"));
        }
    }
    Ok(())
}

fn encrypt_secret(
    key: &SecretKey,
    id: &str,
    field: &str,
    plaintext: &str,
) -> Result<String, AppError> {
    Ok(veydan_lock::encrypt_field(key, id, field, plaintext)?)
}

fn open_key(lock: &Lock) -> Result<(SecretKey, String), AppError> {
    Ok(lock.require_open()?)
}

#[tauri::command]
pub async fn password_list(core: tauri::State<'_, Core>) -> CmdResult<Vec<PasswordPublic>> {
    let rows = sqlx::query_as::<_, PasswordRow>("SELECT * FROM passwords ORDER BY updated_at DESC")
        .fetch_all(&core.db)
        .await
        .map_err(AppError::db)?;
    Ok(rows.iter().map(to_public).collect())
}

#[tauri::command]
pub async fn password_get(id: String, core: tauri::State<'_, Core>) -> CmdResult<PasswordPublic> {
    let row = load(&core, &id).await?;
    Ok(to_public(&row))
}

#[tauri::command]
pub async fn password_create(
    req: PasswordCreate,
    core: tauri::State<'_, Core>,
    lock: tauri::State<'_, Lock>,
) -> CmdResult<PasswordPublic> {
    let title = req.title.trim().to_string();
    if title.is_empty() {
        return Err(AppError::other("Title is required"));
    }
    if req.password.is_empty() {
        return Err(AppError::other("Password cannot be empty"));
    }
    let totp_ids = clean_ids(req.totp_ids.unwrap_or_default());
    totp_all_exist(&core, &totp_ids).await?;
    let (key, vault_id) = lock.ensure_key().await?;
    let id = Uuid::new_v4().to_string();
    let password_enc = encrypt_secret(&key, &id, "password", &req.password)?;
    let note_enc = match blank(req.note) {
        Some(note) => Some(encrypt_secret(&key, &id, "note", &note)?),
        None => None,
    };
    let tags = serde_json::to_string(&req.tags.unwrap_or_default())?;
    let totp_json = serde_json::to_string(&totp_ids)?;
    let now = Utc::now().to_rfc3339();
    let username = blank(req.username);
    let url = blank(req.url);
    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    sqlx::query(
        "INSERT INTO passwords (
            id, title, username, url, password_enc, note_enc, totp_ids, tags, vault_id, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&title)
    .bind(&username)
    .bind(&url)
    .bind(&password_enc)
    .bind(&note_enc)
    .bind(&totp_json)
    .bind(&tags)
    .bind(&vault_id)
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(AppError::db)?;
    directory::relabel(&core, &mut tx, KIND_PASSWORD, &id).await?;
    tx.commit().await.map_err(AppError::db)?;

    Ok(PasswordPublic {
        id,
        title,
        username,
        url,
        has_note: note_enc.is_some(),
        totp_ids,
        tags: parse_list(&tags),
        created_at: now.clone(),
        updated_at: now,
    })
}

#[tauri::command]
pub async fn password_update(
    id: String,
    req: PasswordUpdate,
    core: tauri::State<'_, Core>,
    lock: tauri::State<'_, Lock>,
) -> CmdResult<PasswordPublic> {
    let (key, vault_id) = open_key(&lock)?;
    let mut row = load(&core, &id).await?;
    if let Some(title) = req.title {
        let title = title.trim().to_string();
        if title.is_empty() {
            return Err(AppError::other("Title is required"));
        }
        row.title = title;
    }
    if let Some(username) = req.username {
        row.username = blank(Some(username));
    }
    if let Some(url) = req.url {
        row.url = blank(Some(url));
    }
    if let Some(ids) = req.totp_ids {
        let ids = clean_ids(ids);
        totp_all_exist(&core, &ids).await?;
        row.totp_ids = serde_json::to_string(&ids)?;
    }
    if let Some(tags) = req.tags {
        row.tags = serde_json::to_string(&tags)?;
    }

    let secrets = req.password.is_some() || req.note.is_some() || req.clear_note;
    if secrets {
        if row.vault_id != vault_id {
            return Err(AppError::DecryptFailed);
        }
        let password = if let Some(password) = req.password {
            if password.is_empty() {
                return Err(AppError::other("Password cannot be empty"));
            }
            password
        } else {
            veydan_lock::decrypt_field(&key, &row.id, "password", &row.password_enc)?
        };
        let note = if req.clear_note {
            None
        } else if let Some(note) = req.note {
            blank(Some(note))
        } else if let Some(stored) = &row.note_enc {
            blank(Some(veydan_lock::decrypt_field(
                &key, &row.id, "note", stored,
            )?))
        } else {
            None
        };
        row.password_enc = encrypt_secret(&key, &row.id, "password", &password)?;
        row.note_enc = match note {
            Some(note) => Some(encrypt_secret(&key, &row.id, "note", &note)?),
            None => None,
        };
        row.vault_id = vault_id.clone();
    }

    row.updated_at = Utc::now().to_rfc3339();
    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    sqlx::query(
        "UPDATE passwords
         SET title = ?, username = ?, url = ?, password_enc = ?, note_enc = ?,
             totp_ids = ?, tags = ?, vault_id = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(&row.title)
    .bind(&row.username)
    .bind(&row.url)
    .bind(&row.password_enc)
    .bind(&row.note_enc)
    .bind(&row.totp_ids)
    .bind(&row.tags)
    .bind(&row.vault_id)
    .bind(&row.updated_at)
    .bind(&row.id)
    .execute(&mut *tx)
    .await
    .map_err(AppError::db)?;
    directory::relabel(&core, &mut tx, KIND_PASSWORD, &row.id).await?;
    tx.commit().await.map_err(AppError::db)?;

    Ok(to_public(&row))
}

/// Metadata only; no vault key needed.
#[tauri::command]
pub async fn password_delete(id: String, core: tauri::State<'_, Core>) -> CmdResult<()> {
    if !delete(&core, &id).await? {
        return Err(AppError::not_found("password"));
    }
    Ok(())
}

/// Delete the entry `id` and report it on the deletion hooks in the same
/// transaction, whether the user or another device deleted it. `false` when
/// there is no such entry.
pub(crate) async fn delete(core: &Core, id: &str) -> Result<bool, AppError> {
    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    let n = sqlx::query("DELETE FROM passwords WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;
    if n.rows_affected() == 0 {
        return Ok(false);
    }
    core.deletions.emit(&mut tx, KIND_PASSWORD, id).await?;
    directory::relabel(core, &mut tx, KIND_PASSWORD, id).await?;
    tx.commit().await.map_err(AppError::db)?;
    Ok(true)
}

/// A tombstone from another device: the entry goes as when the user
/// deletes it here.
pub(crate) fn delete_synced<'a>(
    _host: &'a Host<'a>,
    core: &'a Core,
    id: &'a str,
) -> BoxFuture<'a, CmdResult<Deletion>> {
    Box::pin(async move {
        delete(core, id).await?;
        Ok(Deletion::Done)
    })
}

#[tauri::command]
pub async fn password_reveal(
    id: String,
    field: String,
    core: tauri::State<'_, Core>,
    lock: tauri::State<'_, Lock>,
) -> CmdResult<RevealedSecret> {
    let (key, vault_id) = open_key(&lock)?;
    let row = load(&core, &id).await?;
    if row.vault_id != vault_id {
        return Err(AppError::DecryptFailed);
    }
    let value = match field.as_str() {
        "password" => veydan_lock::decrypt_field(&key, &row.id, "password", &row.password_enc)?,
        "note" => match &row.note_enc {
            Some(stored) => veydan_lock::decrypt_field(&key, &row.id, "note", stored)?,
            None => String::new(),
        },
        _ => return Err(AppError::other("Unknown field")),
    };
    Ok(RevealedSecret { value })
}

#[cfg(desktop)]
#[tauri::command]
pub async fn password_copy(
    id: String,
    core: tauri::State<'_, Core>,
    lock: tauri::State<'_, Lock>,
) -> CmdResult<()> {
    let revealed = password_reveal(id, "password".into(), core, lock).await?;
    let secret = revealed.value;
    let to_write = secret.clone();
    tokio::task::spawn_blocking(move || {
        let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
        clipboard.set_text(to_write).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| AppError::other(e.to_string()))?
    .map_err(AppError::other)?;

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        let _ = tokio::task::spawn_blocking(move || {
            let mut clipboard = arboard::Clipboard::new().ok()?;
            if clipboard.get_text().ok()? == secret {
                let _ = clipboard.set_text(String::new());
            }
            Some(())
        })
        .await;
    });
    Ok(())
}

/// Let the lock put a new key in place of the old one, deleting every entry
/// and retracting its label in the same transaction. The lock password
/// stays; the secret boxes empty with the old key.
#[tauri::command]
pub async fn password_vault_reset(
    password: String,
    app: tauri::AppHandle,
    lock: tauri::State<'_, Lock>,
) -> CmdResult<()> {
    reset_vault(&password, app, &lock).await
}

async fn reset_vault<R: tauri::Runtime>(
    password: &str,
    app: tauri::AppHandle<R>,
    lock: &Lock,
) -> CmdResult<()> {
    Ok(lock
        .reset(password, move |conn| {
            Box::pin(async move {
                let core = tauri::Manager::state::<Core>(&app);
                let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM passwords")
                    .fetch_all(&mut *conn)
                    .await
                    .map_err(veydan_lock::Error::db)?;
                sqlx::query("DELETE FROM passwords")
                    .execute(&mut *conn)
                    .await
                    .map_err(veydan_lock::Error::db)?;
                for id in ids {
                    directory::relabel(&core, conn, KIND_PASSWORD, &id)
                        .await
                        .map_err(|e| veydan_lock::Error::Other(e.to_string()))?;
                }
                Ok(())
            })
        })
        .await?)
}

/// What pass keeps under the key of the vault: the fields `password` and
/// `note` of every entry, with the vault id of their key beside them.
pub(crate) const KEY_USER: KeyUser = KeyUser {
    holds: holds_key,
    rekey,
};

fn holds_key<'c>(conn: &'c mut SqliteConnection, vault_id: &'c str) -> KeyFuture<'c, bool> {
    Box::pin(async move {
        let held: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM passwords WHERE vault_id = ? LIMIT 1")
                .bind(vault_id)
                .fetch_optional(&mut *conn)
                .await
                .map_err(veydan_lock::Error::db)?;
        Ok(held.is_some())
    })
}

/// Encrypt the entries under the old key with the new one. An entry that
/// does not open stays under the old key, and the lock keeps that key.
fn rekey<'c>(conn: &'c mut SqliteConnection, rekey: Rekey<'c>) -> KeyFuture<'c, bool> {
    Box::pin(async move {
        let entries: Vec<(String, String, Option<String>)> =
            sqlx::query_as("SELECT id, password_enc, note_enc FROM passwords WHERE vault_id = ?")
                .bind(rekey.from_id)
                .fetch_all(&mut *conn)
                .await
                .map_err(veydan_lock::Error::db)?;
        let mut complete = true;
        for (id, password_enc, note_enc) in entries {
            let recrypt = |field: &str, stored: &str| -> veydan_lock::Result<String> {
                let plain =
                    Zeroizing::new(veydan_lock::decrypt_field(rekey.from, &id, field, stored)?);
                veydan_lock::encrypt_field(rekey.to, &id, field, &plain)
            };
            let moved = recrypt("password", &password_enc).and_then(|password| {
                let note = note_enc
                    .as_deref()
                    .map(|n| recrypt("note", n))
                    .transpose()?;
                Ok((password, note))
            });
            let Ok((password, note)) = moved else {
                complete = false;
                continue;
            };
            sqlx::query(
                "UPDATE passwords SET password_enc = ?, note_enc = ?, vault_id = ? WHERE id = ?",
            )
            .bind(password)
            .bind(note)
            .bind(rekey.to_id)
            .bind(&id)
            .execute(&mut *conn)
            .await
            .map_err(veydan_lock::Error::db)?;
        }
        Ok(complete)
    })
}

/// A note is gone: the entries that named it with a tag `note:{id}` lose
/// the tag, in the transaction that deletes the note.
pub(crate) fn forget_note<'a>(
    tx: &'a mut Transaction<'_, Sqlite>,
    id: &'a str,
) -> BoxFuture<'a, Result<(), AppError>> {
    Box::pin(async move {
        let needle = format!("\"note:{id}\"");
        let linked: Vec<(String, String)> =
            sqlx::query_as("SELECT id, tags FROM passwords WHERE instr(tags, ?) > 0")
                .bind(&needle)
                .fetch_all(&mut **tx)
                .await
                .map_err(AppError::db)?;
        let tag = format!("note:{id}");
        for (pw_id, raw) in linked {
            let mut tags: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
            tags.retain(|t| t != &tag);
            sqlx::query("UPDATE passwords SET tags = ?, updated_at = ? WHERE id = ?")
                .bind(serde_json::to_string(&tags).map_err(AppError::other)?)
                .bind(Utc::now().to_rfc3339())
                .bind(&pw_id)
                .execute(&mut **tx)
                .await
                .map_err(AppError::db)?;
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "VEYDAN_TEST_SECRET_72FC194";
    const SYNC_SECRET: &str = "VEYDAN_SYNC_SECRET_91AB";

    #[tokio::test]
    async fn plaintext_absent_from_db_and_wal() {
        let dir = std::env::temp_dir().join(format!("veydan-pw-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.db");
        let url = format!("sqlite:{}?mode=rwc", db_path.display());
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE passwords (
                id TEXT PRIMARY KEY NOT NULL,
                password_enc TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        let key = SecretKey::random();
        let id = "entry-1";
        let enc = veydan_lock::encrypt_field(&key, id, "password", SECRET).unwrap();
        assert!(!enc.contains(SECRET));
        sqlx::query("INSERT INTO passwords (id, password_enc) VALUES (?, ?)")
            .bind(id)
            .bind(&enc)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("PRAGMA wal_checkpoint(FULL)")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;

        let db_bytes = std::fs::read(&db_path).unwrap();
        assert!(!bytes_contain(&db_bytes, SECRET.as_bytes()));
        let wal_path = format!("{}-wal", db_path.display());
        if let Ok(wal) = std::fs::read(&wal_path) {
            assert!(!bytes_contain(&wal, SECRET.as_bytes()));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sync_row_holds_ciphertext_only() {
        let key = SecretKey::random();
        let enc = veydan_lock::encrypt_field(&key, "entry", "password", SYNC_SECRET).unwrap();
        let row = serde_json::json!({
            "id": "entry",
            "title": "GitHub",
            "password_enc": enc,
        });
        let serialized = row.to_string();
        assert!(!serialized.contains(SYNC_SECRET));
        assert!(serialized.contains(&enc));
    }

    fn bytes_contain(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    async fn insert(core: &Core, id: &str, enc: &str, vault_id: &str, tags: &str) {
        sqlx::query(
            "INSERT INTO passwords (id, title, password_enc, vault_id, tags, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 't', 't')",
        )
        .bind(id)
        .bind(id)
        .bind(enc)
        .bind(vault_id)
        .bind(tags)
        .execute(&core.db)
        .await
        .unwrap();
    }

    async fn tags(core: &Core, id: &str) -> (Vec<String>, String) {
        let (tags, updated_at): (String, String) =
            sqlx::query_as("SELECT tags, updated_at FROM passwords WHERE id = ?")
                .bind(id)
                .fetch_one(&core.db)
                .await
                .unwrap();
        (serde_json::from_str(&tags).unwrap(), updated_at)
    }

    /// The owner of notes reports a deletion in its transaction; the entries
    /// that named the note lose that tag and keep every other one.
    #[tokio::test]
    async fn a_deleted_note_leaves_the_tags_of_the_entries() {
        use tauri::Manager;
        let (app, _dir) = crate::testing::app().await;
        let core = app.state::<Core>();
        core.deletions.subscribe("note", forget_note);
        insert(&core, "pw-1", "x", "v", r#"["note:n1"]"#).await;
        insert(
            &core,
            "pw-2",
            "x",
            "v",
            r#"["work/dev","note:n1","note:n2","profile:p1"]"#,
        )
        .await;
        insert(&core, "pw-3", "x", "v", r#"["note:n10"]"#).await;

        let mut tx = core.db.begin().await.unwrap();
        core.deletions.emit(&mut tx, "note", "n1").await.unwrap();
        tx.commit().await.unwrap();

        let (one, at) = tags(&core, "pw-1").await;
        assert!(one.is_empty());
        assert_ne!(at, "t");
        assert_eq!(
            tags(&core, "pw-2").await.0,
            ["work/dev", "note:n2", "profile:p1"]
        );
        assert_eq!(
            tags(&core, "pw-3").await,
            (vec!["note:n10".to_string()], "t".to_string())
        );

        // A deletion the owner takes back leaves the tags as they were.
        let mut tx = core.db.begin().await.unwrap();
        core.deletions.emit(&mut tx, "note", "n2").await.unwrap();
        drop(tx);
        assert_eq!(
            tags(&core, "pw-2").await.0,
            ["work/dev", "note:n2", "profile:p1"]
        );
    }

    /// An entry linked to a TOTP entry, through the commands the UI calls.
    #[tokio::test]
    async fn an_entry_goes_through_the_commands() {
        use crate::totp::{totp_add, totp_delete, totp_generate_code, TotpAddRequest};
        use tauri::Manager;
        let (app, _dir) = crate::testing::app().await;
        let totp = totp_add(
            TotpAddRequest {
                name: "github-work".into(),
                issuer: Some("GitHub".into()),
                secret: Some("jbsw y3dp ehpk 3pxp".into()),
                uri: None,
                algorithm: None,
                digits: None,
                period: None,
                tags: vec![],
            },
            app.state(),
        )
        .await
        .unwrap();
        assert_eq!(
            totp_generate_code(totp.id.clone(), app.state())
                .await
                .unwrap()
                .code
                .len(),
            6
        );

        let created = password_create(
            PasswordCreate {
                title: " GitHub ".into(),
                username: Some("octocat".into()),
                url: None,
                password: "hunter2".into(),
                note: Some("the panel".into()),
                totp_ids: Some(vec![totp.id.clone()]),
                tags: Some(vec!["note:n1".into()]),
            },
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert_eq!(created.title, "GitHub");
        assert!(created.has_note);
        let reveal = |field: &str| {
            password_reveal(created.id.clone(), field.into(), app.state(), app.state())
        };
        assert_eq!(reveal("password").await.unwrap().value, "hunter2");
        assert_eq!(reveal("note").await.unwrap().value, "the panel");

        let update = PasswordUpdate {
            title: None,
            username: None,
            url: None,
            password: Some("hunter3".into()),
            note: None,
            clear_note: true,
            totp_ids: None,
            tags: None,
        };
        let updated = password_update(created.id.clone(), update, app.state(), app.state())
            .await
            .unwrap();
        assert!(!updated.has_note);
        assert_eq!(reveal("password").await.unwrap().value, "hunter3");

        // A TOTP entry that goes leaves the entries that linked it.
        totp_delete(totp.id.clone(), app.state()).await.unwrap();
        let listed = password_list(app.state()).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].totp_ids.is_empty());

        password_delete(created.id.clone(), app.state())
            .await
            .unwrap();
        assert!(password_list(app.state()).await.unwrap().is_empty());
        assert!(matches!(
            password_delete(created.id, app.state()).await,
            Err(AppError::NotFound(_))
        ));
    }

    /// The deletions the hooks below were told of, as `kind:id`.
    static REPORTED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

    fn reported_password<'a>(
        _: &'a mut Transaction<'_, Sqlite>,
        id: &'a str,
    ) -> BoxFuture<'a, Result<(), AppError>> {
        Box::pin(async move {
            if id == "pw-refused" {
                return Err(AppError::other("a subscriber failed"));
            }
            REPORTED.lock().unwrap().push(format!("password:{id}"));
            Ok(())
        })
    }

    fn reported_totp<'a>(
        _: &'a mut Transaction<'_, Sqlite>,
        id: &'a str,
    ) -> BoxFuture<'a, Result<(), AppError>> {
        Box::pin(async move {
            REPORTED.lock().unwrap().push(format!("totp:{id}"));
            Ok(())
        })
    }

    struct NoStates;

    impl veydan_sync_host::States for NoStates {
        fn state(&self, _: std::any::TypeId) -> Option<&(dyn std::any::Any + Send + Sync)> {
            None
        }
    }

    /// Pass reports the deletion of an entry on the deletion hooks in the
    /// transaction that deletes it: deleted here and deleted by a tombstone
    /// alike (spec 5.8).
    #[tokio::test]
    async fn a_deleted_entry_is_reported() {
        use crate::totp::totp_delete;
        use tauri::Manager;
        use veydan_sync_host::Registry;
        let (app, _dir) = crate::testing::app().await;
        let core = app.state::<Core>();
        core.deletions.subscribe(KIND_PASSWORD, reported_password);
        core.deletions.subscribe(crate::KIND_TOTP, reported_totp);
        for id in ["pw-1", "pw-2", "pw-refused"] {
            insert(&core, id, "x", "v", "[]").await;
        }
        sqlx::query(
            "INSERT INTO totp_entries (id, name, secret, created_at, updated_at)
             VALUES ('t-1', 'a', 'JBSWY3DPEHPK3PXP', 't', 't'), ('t-2', 'b', 'JBSWY3DPEHPK3PXP', 't', 't')",
        )
        .execute(&core.db)
        .await
        .unwrap();
        let count = |table: &'static str| {
            let db = core.db.clone();
            async move {
                let sql = format!("SELECT COUNT(*) FROM {table}");
                sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql))
                    .fetch_one(&db)
                    .await
                    .unwrap()
            }
        };

        password_delete("pw-1".into(), app.state()).await.unwrap();
        totp_delete("t-1".into(), app.state()).await.unwrap();
        // Nothing to delete, nothing to report.
        assert!(password_delete("pw-1".into(), app.state()).await.is_err());
        totp_delete("t-1".into(), app.state()).await.unwrap();
        // A subscriber that fails keeps the entry.
        assert!(password_delete("pw-refused".into(), app.state())
            .await
            .is_err());
        assert_eq!(count("passwords").await, 2);

        let mut registry = Registry::new();
        registry.add("pass", crate::sync);
        let host = veydan_sync_host::Host::States(&NoStates);
        for (entity, id) in [("password", "pw-2"), ("totp", "t-2")] {
            let on_delete = registry.table_of(entity).unwrap().hooks.on_delete.unwrap();
            let done = on_delete(&host, &core, id).await.unwrap();
            assert!(matches!(done, Deletion::Done));
        }
        assert_eq!(
            (count("passwords").await, count("totp_entries").await),
            (1, 0)
        );
        assert_eq!(
            *REPORTED.lock().unwrap(),
            ["password:pw-1", "totp:t-1", "password:pw-2", "totp:t-2"]
        );
    }

    /// What pass tells the lock: whether it keeps something under a key,
    /// and how its entries move to another key.
    #[tokio::test]
    async fn the_entries_move_from_one_key_to_another() {
        use tauri::Manager;
        let (app, _dir) = crate::testing::app().await;
        let core = app.state::<Core>();
        let (old, new) = (SecretKey::random(), SecretKey::random());
        let enc = |key: &SecretKey, id: &str, field: &str, text: &str| {
            veydan_lock::encrypt_field(key, id, field, text).unwrap()
        };
        insert(
            &core,
            "pw-1",
            &enc(&old, "pw-1", "password", "hunter2"),
            "old",
            "[]",
        )
        .await;
        sqlx::query("UPDATE passwords SET note_enc = ? WHERE id = 'pw-1'")
            .bind(enc(&old, "pw-1", "note", "the panel"))
            .execute(&core.db)
            .await
            .unwrap();
        // Under the old id, but not readable with the old key.
        insert(
            &core,
            "pw-2",
            &enc(&new, "pw-2", "password", "x"),
            "old",
            "[]",
        )
        .await;

        let mut conn = core.db.acquire().await.unwrap();
        assert!((KEY_USER.holds)(&mut conn, "old").await.unwrap());
        assert!(!(KEY_USER.holds)(&mut conn, "new").await.unwrap());
        let rekey = Rekey {
            from: &old,
            from_id: "old",
            to: &new,
            to_id: "new",
        };
        assert!(!(KEY_USER.rekey)(&mut conn, rekey).await.unwrap());
        drop(conn);

        let rows: Vec<(String, String, Option<String>, String)> = sqlx::query_as(
            "SELECT id, password_enc, note_enc, vault_id FROM passwords ORDER BY id",
        )
        .fetch_all(&core.db)
        .await
        .unwrap();
        let (_, password, note, vault_id) = &rows[0];
        assert_eq!(vault_id, "new");
        assert_eq!(
            veydan_lock::decrypt_field(&new, "pw-1", "password", password).unwrap(),
            "hunter2"
        );
        let note = note.as_deref().unwrap();
        assert_eq!(
            veydan_lock::decrypt_field(&new, "pw-1", "note", note).unwrap(),
            "the panel"
        );
        assert_eq!(rows[1].3, "old");
    }

    /// Every write to `labels` from here on, counted by triggers: a publish
    /// that writes nothing gives sync nothing to send.
    async fn count_label_writes(core: &Core) {
        sqlx::raw_sql(
            "CREATE TABLE label_writes (n INTEGER NOT NULL);
             INSERT INTO label_writes VALUES (0);
             CREATE TRIGGER label_put AFTER INSERT ON labels BEGIN UPDATE label_writes SET n = n + 1; END;
             CREATE TRIGGER label_set AFTER UPDATE ON labels BEGIN UPDATE label_writes SET n = n + 1; END;
             CREATE TRIGGER label_cut AFTER DELETE ON labels BEGIN UPDATE label_writes SET n = n + 1; END;",
        )
        .execute(&core.db)
        .await
        .unwrap();
    }

    async fn label_writes(core: &Core) -> i64 {
        sqlx::query_scalar("SELECT n FROM label_writes")
            .fetch_one(&core.db)
            .await
            .unwrap()
    }

    /// What sync would send of the labels: each row as its op carries it.
    async fn published(core: &Core) -> Vec<(String, serde_json::Value)> {
        let registry = veydan_sync_host::Registry::new();
        let labels = registry.table_of(veydan_sync_host::LABEL_ENTITY).unwrap();
        veydan_sync_host::rows::read_rows(&core.db, labels)
            .await
            .unwrap()
    }

    /// Spec 10.2: the label of an entry is its name and nothing else — no
    /// user name, address, issuer, secret, note or tag reaches the catalog
    /// sync carries to products without pass. It follows the name: a rename
    /// and a deletion publish, a change of anything else does not.
    #[tokio::test]
    async fn the_label_of_an_entry_is_its_name_alone() {
        use crate::totp::{totp_add, totp_delete, totp_update, TotpAddRequest, TotpUpdateRequest};
        use serde_json::json;
        use tauri::Manager;
        let (app, _dir) = crate::testing::app().await;
        let core = app.state::<Core>();
        count_label_writes(&core).await;
        let totp = totp_add(
            TotpAddRequest {
                name: "github-work".into(),
                issuer: Some("IssuerCorp".into()),
                secret: Some("JBSWY3DPEHPK3PXP".into()),
                uri: None,
                algorithm: None,
                digits: None,
                period: None,
                tags: vec!["totp-tag".into()],
            },
            app.state(),
        )
        .await
        .unwrap();
        let entry = password_create(
            PasswordCreate {
                title: "GitHub".into(),
                username: Some("octocat".into()),
                url: Some("https://github.example/login".into()),
                password: "hunter2-secret".into(),
                note: Some("panel-note".into()),
                totp_ids: Some(vec![totp.id.clone()]),
                tags: Some(vec!["note:n1".into()]),
            },
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        let label = |kind: &str, id: &str, name: &str| {
            (
                format!("{kind}:{id}"),
                json!({ "kind": kind, "id": id, "name": name,
                        "parent_kind": null, "parent_id": null, "color": null }),
            )
        };
        assert_eq!(
            published(&core).await,
            [
                label("totp", &totp.id, "github-work"),
                label("password", &entry.id, "GitHub"),
            ]
        );
        let wire = serde_json::to_string(&published(&core).await).unwrap();
        let stored: Vec<String> = sqlx::query_scalar(
            "SELECT json_array(key, kind, id, name, parent_kind, parent_id, color) FROM labels",
        )
        .fetch_all(&core.db)
        .await
        .unwrap();
        for private in [
            "octocat",
            "github.example",
            "IssuerCorp",
            "JBSWY3DPEHPK3PXP",
            "hunter2-secret",
            "panel-note",
            "note:n1",
            "totp-tag",
        ] {
            assert!(!wire.contains(private), "{private} in {wire}");
            assert!(
                !stored.concat().contains(private),
                "{private} in {stored:?}"
            );
        }
        let names_only = label_writes(&core).await;
        assert_eq!(names_only, 2);

        // Everything but the name changes: nothing is published.
        let update = |title: Option<&str>| PasswordUpdate {
            title: title.map(str::to_owned),
            username: Some("someone-else".into()),
            url: Some(String::new()),
            password: Some("hunter3".into()),
            note: Some("another note".into()),
            clear_note: false,
            totp_ids: Some(vec![]),
            tags: Some(vec![]),
        };
        password_update(entry.id.clone(), update(None), app.state(), app.state())
            .await
            .unwrap();
        let issuer = TotpUpdateRequest {
            name: Some("github-work".into()),
            issuer: Some("Elsewhere".into()),
            tags: Some(vec![]),
        };
        totp_update(totp.id.clone(), issuer, app.state())
            .await
            .unwrap();
        assert_eq!(label_writes(&core).await, names_only);

        password_update(
            entry.id.clone(),
            update(Some("GitHub work")),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        totp_delete(totp.id.clone(), app.state()).await.unwrap();
        assert_eq!(
            published(&core).await,
            [label("password", &entry.id, "GitHub work")]
        );
        password_delete(entry.id.clone(), app.state())
            .await
            .unwrap();
        assert!(published(&core).await.is_empty());
    }

    /// A reset of the vault deletes every entry and retracts their labels in
    /// the same transaction; the TOTP entries stay named.
    #[tokio::test]
    async fn a_reset_of_the_vault_retracts_the_labels_of_the_entries() {
        use tauri::Manager;
        let (app, _dir) = crate::testing::app().await;
        let lock = app.state::<Lock>();
        lock.set(Some("4821".into()), None, Some("pin".into()), None)
            .await
            .unwrap();
        let core = app.state::<Core>();
        for title in ["One", "Two"] {
            let create = PasswordCreate {
                title: title.into(),
                username: None,
                url: None,
                password: "x".into(),
                note: None,
                totp_ids: None,
                tags: None,
            };
            password_create(create, app.state(), app.state())
                .await
                .unwrap();
        }
        sqlx::query(
            "INSERT INTO labels (key, kind, id, name) VALUES ('totp:t-1', 'totp', 't-1', 'kept')",
        )
        .execute(&core.db)
        .await
        .unwrap();
        assert_eq!(published(&core).await.len(), 3);
        reset_vault("4821", app.handle().clone(), &lock)
            .await
            .unwrap();
        let keys: Vec<String> = published(&core)
            .await
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        assert_eq!(keys, ["totp:t-1"]);
    }
}
