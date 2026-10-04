// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The kinds `password` and `totp` in the entity directory: what another
//! module may know of an entry without reading the tables of pass. A label
//! is the title of a password or the name of a TOTP entry and nothing more —
//! the synced label too; the action `field` gives a public field, `subtitle`
//! the line a picker shows under the name (the user name of a password, the
//! issuer of a TOTP entry), `code` the current code of a TOTP entry. No
//! action gives a secret.

use crate::totp::{self, TotpEntry};
use crate::{KIND_PASSWORD, KIND_TOTP};
use sqlx::SqliteConnection;
use tauri::{AppHandle, Manager, Runtime};
use veydan_core::{AppError, BoxFuture, CmdResult, Core, Label, Provider};
use veydan_lock::Lock;

/// The fields of a password another module may show or put into a note.
const PASSWORD_FIELDS: &[&str] = &["title", "username", "url"];
/// The same of a TOTP entry.
const TOTP_FIELDS: &[&str] = &["name", "issuer"];

pub(crate) fn passwords<R: Runtime>() -> Provider<R> {
    Provider {
        label: |app, id| Box::pin(async move { label(&app, KIND_PASSWORD, &id).await }),
        search: |app, query, limit| {
            Box::pin(async move { search(&app, KIND_PASSWORD, &query, limit).await })
        },
        list: |app| Box::pin(async move { list(&app, KIND_PASSWORD).await }),
        action: Some(|app, id, action, arg| {
            Box::pin(async move {
                match action.as_str() {
                    "field" => field(&app, KIND_PASSWORD, &id, arg.as_deref()).await,
                    "subtitle" => subtitle(&app, KIND_PASSWORD, &id).await,
                    _ => Err(no_action(KIND_PASSWORD, &action)),
                }
            })
        }),
    }
}

pub(crate) fn totp<R: Runtime>() -> Provider<R> {
    Provider {
        label: |app, id| Box::pin(async move { label(&app, KIND_TOTP, &id).await }),
        search: |app, query, limit| {
            Box::pin(async move { search(&app, KIND_TOTP, &query, limit).await })
        },
        list: |app| Box::pin(async move { list(&app, KIND_TOTP).await }),
        action: Some(|app, id, action, arg| {
            Box::pin(async move {
                match action.as_str() {
                    "field" => field(&app, KIND_TOTP, &id, arg.as_deref()).await,
                    "subtitle" => subtitle(&app, KIND_TOTP, &id).await,
                    "code" => code(&app, &id).await,
                    _ => Err(no_action(KIND_TOTP, &action)),
                }
            })
        }),
    }
}

/// The table of a kind and the column its label shows.
fn table(kind: &str) -> (&'static str, &'static str) {
    if kind == KIND_PASSWORD {
        ("passwords", "title")
    } else {
        ("totp_entries", "name")
    }
}

/// The subtitle of an entry, as SQL over its row: what a picker
/// shows under the name and what its search matches besides the name.
fn subtitle_sql(kind: &str) -> &'static str {
    if kind == KIND_PASSWORD {
        "coalesce(username, '')"
    } else {
        "coalesce(issuer, '')"
    }
}

fn label_of(kind: &str, id: String, name: String) -> Label {
    Label {
        kind: kind.to_owned(),
        id,
        name,
        parent: None,
        color: None,
    }
}

/// Publish the label of the entry `id` as its row is now, or retract it when
/// the row is gone: on the connection of the transaction that changed it.
pub(crate) async fn relabel(
    core: &Core,
    conn: &mut SqliteConnection,
    kind: &str,
    id: &str,
) -> Result<(), AppError> {
    let (table, name) = table(kind);
    // Safe: table and column are constants of this file; the id is bound.
    let sql = format!("SELECT {name} FROM {table} WHERE id = ?");
    let found: Option<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(AppError::db)?;
    match found {
        Some(name) => {
            let label = label_of(kind, id.to_owned(), name);
            core.directory.publish(conn, &label).await
        }
        None => core.directory.retract(conn, kind, id).await,
    }
}

/// `relabel` in a transaction of its own, after sync wrote or removed a row.
async fn relabel_synced(core: &Core, kind: &str, id: &str) -> CmdResult<()> {
    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    relabel(core, &mut tx, kind, id).await?;
    tx.commit().await.map_err(AppError::db)
}

/// The hooks `after_row` of the entities `password` and `totp`.
pub(crate) fn password_synced<'a>(core: &'a Core, id: &'a str) -> BoxFuture<'a, CmdResult<()>> {
    Box::pin(relabel_synced(core, KIND_PASSWORD, id))
}

pub(crate) fn totp_synced<'a>(core: &'a Core, id: &'a str) -> BoxFuture<'a, CmdResult<()>> {
    Box::pin(relabel_synced(core, KIND_TOTP, id))
}

async fn label<R: Runtime>(app: &AppHandle<R>, kind: &str, id: &str) -> Option<Label> {
    let (table, name) = table(kind);
    // Safe: table and column are constants of this file; the id is bound.
    let sql = format!("SELECT {name} FROM {table} WHERE id = ?");
    let found: Option<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(&app.state::<Core>().db)
        .await
        .ok()
        .flatten();
    Some(label_of(kind, id.to_owned(), found?))
}

/// Every entry of the kind, by name.
async fn list<R: Runtime>(app: &AppHandle<R>, kind: &str) -> Vec<Label> {
    let (table, name) = table(kind);
    // Safe: table and column are constants of this file.
    let sql = format!("SELECT id, {name} FROM {table} ORDER BY {name} COLLATE NOCASE, id");
    rows(app, kind, sql, &[]).await
}

/// Up to `limit` entries whose name or subtitle matches `query` as
/// `LIKE '%query%'` does, by name byte by byte and then by id
/// (`Provider::search`): one query.
async fn search<R: Runtime>(
    app: &AppHandle<R>,
    kind: &str,
    query: &str,
    limit: usize,
) -> Vec<Label> {
    let (table, name) = table(kind);
    let sql = format!(
        "SELECT id, {name} FROM {table} WHERE lower({name}) LIKE ? OR lower({}) LIKE ?
         ORDER BY {name}, id LIMIT {}",
        subtitle_sql(kind),
        i64::try_from(limit).unwrap_or(i64::MAX)
    );
    let pattern = veydan_core::like_pattern(query);
    rows(app, kind, sql, &[&pattern, &pattern]).await
}

async fn rows<R: Runtime>(
    app: &AppHandle<R>,
    kind: &str,
    sql: String,
    binds: &[&str],
) -> Vec<Label> {
    // Safe: table and columns are constants of this file; values are bound.
    let mut query = sqlx::query_as::<_, (String, String)>(sqlx::AssertSqlSafe(sql));
    for value in binds {
        query = query.bind(*value);
    }
    query
        .fetch_all(&app.state::<Core>().db)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(id, name)| label_of(kind, id, name))
        .collect()
}

/// The subtitle of the entry `id`.
async fn subtitle<R: Runtime>(
    app: &AppHandle<R>,
    kind: &str,
    id: &str,
) -> Result<String, AppError> {
    let (table, _) = table(kind);
    // Safe: table and expression are constants of this file; the id is bound.
    let sql = format!("SELECT {} FROM {table} WHERE id = ?", subtitle_sql(kind));
    sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(&app.state::<Core>().db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found(format!("{kind} {id}")))
}

/// A public field of the entry `id`; an empty string where it has none.
async fn field<R: Runtime>(
    app: &AppHandle<R>,
    kind: &str,
    id: &str,
    field: Option<&str>,
) -> Result<String, AppError> {
    let (table, fields) = if kind == KIND_PASSWORD {
        ("passwords", PASSWORD_FIELDS)
    } else {
        ("totp_entries", TOTP_FIELDS)
    };
    let Some(field) = field.and_then(|f| fields.iter().find(|known| **known == f)) else {
        return Err(AppError::other(format!(
            "{kind} has no field {}",
            field.unwrap_or_default()
        )));
    };
    // Safe: table and column come from the lists of this file; the id is bound.
    let sql = format!("SELECT {field} FROM {table} WHERE id = ?");
    let value: Option<Option<String>> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(&app.state::<Core>().db)
        .await
        .map_err(AppError::db)?;
    match value {
        Some(value) => Ok(value.unwrap_or_default()),
        None => Err(AppError::not_found(format!("{kind} {id}"))),
    }
}

/// The current code of the TOTP entry `id`. Not while the app is locked.
async fn code<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<String, AppError> {
    if app.state::<Lock>().is_locked().await {
        return Err(AppError::VaultLocked);
    }
    let entry = sqlx::query_as::<_, TotpEntry>("SELECT * FROM totp_entries WHERE id = ?")
        .bind(id)
        .fetch_optional(&app.state::<Core>().db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found(format!("TOTP entry {id}")))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| AppError::other(e.to_string()))?;
    totp::code_at(&entry, now.as_secs())
}

fn no_action(kind: &str, action: &str) -> AppError {
    AppError::other(format!("entity kind `{kind}` has no action `{action}`"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;
    use tauri::test::MockRuntime;
    use veydan_core::Directory;

    async fn sql(app: &tauri::App<MockRuntime>, statement: &'static str) {
        sqlx::query(statement)
            .execute(&app.state::<Core>().db)
            .await
            .unwrap();
    }

    /// Two passwords and two TOTP entries; the secret of `t-rfc` is the one of
    /// RFC 6238, Appendix B.
    async fn directory() -> (
        Directory<MockRuntime>,
        tauri::App<MockRuntime>,
        tempfile::TempDir,
    ) {
        let (app, dir) = testing::app().await;
        sql(
            &app,
            "INSERT INTO passwords (id, title, username, url, password_enc, vault_id, created_at, updated_at) VALUES
             ('p-gh', 'GitHub', 'octocat', 'https://github.com', 'x', 'v', 't', 't'),
             ('p-aws', 'AWS root', NULL, NULL, 'x', 'v', 't', 't')",
        )
        .await;
        sql(
            &app,
            "INSERT INTO totp_entries (id, name, issuer, secret, algorithm, digits, period, created_at, updated_at) VALUES
             ('t-rfc', 'rfc-6238', 'IETF', 'GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ', 'SHA1', 8, 30, 't', 't'),
             ('t-gh', 'github-work', NULL, 'JBSWY3DPEHPK3PXP', 'SHA1', 6, 30, 't', 't')",
        )
        .await;
        let mut directory = Directory::default();
        directory.provide(KIND_PASSWORD, passwords());
        directory.provide(KIND_TOTP, totp());
        directory.attach(app.handle().clone());
        (directory, app, dir)
    }

    fn names(labels: &[Label]) -> Vec<&str> {
        labels.iter().map(|l| l.name.as_str()).collect()
    }

    #[tokio::test]
    async fn every_entry_has_a_label_with_its_name_alone() {
        let (directory, _app, _dir) = directory().await;
        assert_eq!(directory.kinds(), ["password", "totp"]);

        assert_eq!(
            names(&directory.list(KIND_PASSWORD).await),
            ["AWS root", "GitHub"]
        );
        assert_eq!(
            names(&directory.list(KIND_TOTP).await),
            ["github-work", "rfc-6238"]
        );
        assert_eq!(
            directory.label(KIND_PASSWORD, "p-gh").await,
            Some(Label {
                kind: "password".into(),
                id: "p-gh".into(),
                name: "GitHub".into(),
                parent: None,
                color: None,
            })
        );
        assert_eq!(
            directory.label(KIND_TOTP, "t-gh").await.map(|l| l.name),
            Some("github-work".into())
        );
        assert_eq!(directory.label(KIND_PASSWORD, "t-gh").await, None);
        assert_eq!(directory.label(KIND_TOTP, "nope").await, None);
    }

    #[tokio::test]
    async fn a_search_finds_names_regardless_of_case() {
        let (directory, _app, _dir) = directory().await;
        assert_eq!(
            names(&directory.search(KIND_PASSWORD, "git", 10).await),
            ["GitHub"]
        );
        assert_eq!(
            names(&directory.search(KIND_TOTP, "-", 1).await),
            ["github-work"]
        );
        assert!(directory.search(KIND_TOTP, "zzz", 10).await.is_empty());
    }

    /// A search orders by name byte by byte, as the picker of notes always
    /// showed it, and entries of one name by id.
    #[tokio::test]
    async fn a_search_orders_by_name_and_then_by_id() {
        let (directory, app, _dir) = directory().await;
        sql(
            &app,
            "INSERT INTO passwords (id, title, username, url, password_enc, vault_id, created_at, updated_at) VALUES
             ('p-m2', 'Mail', NULL, NULL, 'x', 'v', 't', 't'),
             ('p-az', 'azure', NULL, NULL, 'x', 'v', 't', 't'),
             ('p-m1', 'Mail', NULL, NULL, 'x', 'v', 't', 't')",
        )
        .await;
        let found = directory.search(KIND_PASSWORD, "", 10).await;
        assert_eq!(
            names(&found),
            ["AWS root", "GitHub", "Mail", "Mail", "azure"]
        );
        let ids: Vec<&str> = found.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids[2..4], ["p-m1", "p-m2"]);
        let first_mail = directory.search(KIND_PASSWORD, "mail", 1).await;
        assert_eq!(first_mail[0].id, "p-m1");
    }

    /// The picker of notes finds an entry by the line under its name too,
    /// and reads that line from the action `subtitle`.
    #[tokio::test]
    async fn a_search_finds_the_subtitle_and_reads_the_query_as_like() {
        let (directory, _app, _dir) = directory().await;
        assert_eq!(
            names(&directory.search(KIND_PASSWORD, "octo", 10).await),
            ["GitHub"]
        );
        assert_eq!(
            names(&directory.search(KIND_TOTP, "ietf", 10).await),
            ["rfc-6238"]
        );
        assert_eq!(
            names(&directory.search(KIND_TOTP, "rfc_6238", 10).await),
            ["rfc-6238"]
        );
        assert_eq!(
            names(&directory.search(KIND_PASSWORD, "", 10).await),
            ["AWS root", "GitHub"]
        );
        let subtitle = |kind, id| directory.action(kind, id, "subtitle", None);
        assert_eq!(subtitle(KIND_PASSWORD, "p-gh").await.unwrap(), "octocat");
        assert_eq!(subtitle(KIND_PASSWORD, "p-aws").await.unwrap(), "");
        assert_eq!(subtitle(KIND_TOTP, "t-rfc").await.unwrap(), "IETF");
        assert_eq!(subtitle(KIND_TOTP, "t-gh").await.unwrap(), "");
        assert!(matches!(
            subtitle(KIND_TOTP, "nope").await,
            Err(AppError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn the_public_fields_and_no_secret() {
        let (directory, _app, _dir) = directory().await;
        let field = |kind, id, name| directory.action(kind, id, "field", Some(name));
        assert_eq!(
            field(KIND_PASSWORD, "p-gh", "username").await.unwrap(),
            "octocat"
        );
        assert_eq!(
            field(KIND_PASSWORD, "p-gh", "url").await.unwrap(),
            "https://github.com"
        );
        assert_eq!(field(KIND_PASSWORD, "p-aws", "username").await.unwrap(), "");
        assert_eq!(field(KIND_TOTP, "t-rfc", "issuer").await.unwrap(), "IETF");
        for (kind, id, secret) in [
            (KIND_PASSWORD, "p-gh", "password_enc"),
            (KIND_PASSWORD, "p-gh", "note_enc"),
            (KIND_TOTP, "t-rfc", "secret"),
        ] {
            assert!(
                matches!(field(kind, id, secret).await, Err(AppError::Other(_))),
                "{secret}"
            );
        }
        assert!(matches!(
            field(KIND_PASSWORD, "nope", "title").await,
            Err(AppError::NotFound(_))
        ));
        assert!(directory
            .action(KIND_PASSWORD, "p-gh", "code", None)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn the_code_of_a_totp_entry_is_the_one_of_this_moment() {
        let (directory, _app, _dir) = directory().await;
        let entry = |secret: &str| TotpEntry {
            id: String::new(),
            name: String::new(),
            issuer: None,
            secret: secret.into(),
            algorithm: "SHA1".into(),
            digits: 6,
            period: 30,
            tags: "[]".into(),
            created_at: String::new(),
            updated_at: String::new(),
            last_used_at: None,
        };
        let now = || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        };
        // A step may turn while the action runs.
        let before = totp::code_at(&entry("JBSWY3DPEHPK3PXP"), now()).unwrap();
        let code = directory
            .action(KIND_TOTP, "t-gh", "code", None)
            .await
            .unwrap();
        let after = totp::code_at(&entry("JBSWY3DPEHPK3PXP"), now()).unwrap();
        assert!(code == before || code == after, "{code}");
        assert_eq!(code.len(), 6);
        assert!(matches!(
            directory.action(KIND_TOTP, "nope", "code", None).await,
            Err(AppError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn no_code_while_the_app_is_locked() {
        let (directory, app, _dir) = directory().await;
        let lock = app.state::<Lock>();
        lock.set(Some("4821".into()), None, Some("pin".into()), None)
            .await
            .unwrap();
        assert!(directory
            .action(KIND_TOTP, "t-gh", "code", None)
            .await
            .is_ok());
        lock.lock();
        assert!(matches!(
            directory.action(KIND_TOTP, "t-gh", "code", None).await,
            Err(AppError::VaultLocked)
        ));
        // Names are no secret.
        assert_eq!(directory.list(KIND_TOTP).await.len(), 2);
    }
}
