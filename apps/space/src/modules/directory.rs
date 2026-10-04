// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The kinds browser (`workspace`, `profile`, `proxy`) and ssh (`ssh`) own in
//! the entity directory: what another module may know of them without
//! reading their tables. A label is the name, with the color of a workspace
//! and the workspace of a profile; the action `field` gives a public field,
//! `subtitle` the line a picker shows under the name — what
//! a search matches besides the name.
//! No action gives a secret — a proxy's credentials, a connection's password
//! or key.
//!
//! The owners publish these labels in the transaction of each change
//! ([`relabel`]), and on a desktop sync carries them to products without
//! browser and ssh. A phone has neither module's data: its notes name
//! workspaces and profiles by the labels sync brought.

use sqlx::SqliteConnection;
use tauri::{AppHandle, Manager, Runtime};
use veydan_core::{AppError, BoxFuture, CmdResult, Core, Directory, Label, Provider};

/// One kind over the table of its owner.
pub(crate) struct Kind {
    kind: &'static str,
    table: &'static str,
    /// The order of `list`: the one the owner's own pages show the rows in.
    order: &'static str,
    /// The fields another module may read; the name is one of them.
    fields: &'static [&'static str],
    /// The line a picker shows under the name, as SQL over the row of the table.
    subtitle: &'static str,
    /// The column of the label's color.
    color: Option<&'static str>,
    /// The kind of the parent and the column of its id.
    parent: Option<(&'static str, &'static str)>,
}

pub(crate) const WORKSPACE: Kind = Kind {
    kind: "workspace",
    table: "workspaces",
    order: "is_default DESC, created_at ASC",
    fields: &["name", "description", "color", "icon"],
    subtitle: "''",
    color: Some("color"),
    parent: None,
};

pub(crate) const PROFILE: Kind = Kind {
    kind: "profile",
    table: "profiles",
    order: "name",
    fields: &["name", "browser_type"],
    subtitle: "browser_type",
    color: None,
    parent: Some(("workspace", "workspace_id")),
};

pub(crate) const PROXY: Kind = Kind {
    kind: "proxy",
    table: "proxies",
    order: "name",
    fields: &["name", "proxy_type", "host", "port", "country", "city"],
    // `SOCKS5 · DE`, or the type alone without a country.
    subtitle: "upper(proxy_type) || CASE WHEN coalesce(country, '') = '' THEN '' ELSE ' · ' || country END",
    color: None,
    parent: None,
};

pub(crate) const SSH: Kind = Kind {
    kind: "ssh",
    table: "ssh_connections",
    order: "name",
    fields: &["name", "host", "port", "username"],
    subtitle: "username || '@' || host",
    color: None,
    parent: None,
};

/// The provider of one kind: the functions of a provider are plain pointers,
/// so each kind gets closures of its own over its constant.
macro_rules! provider {
    ($kind:expr) => {
        Provider {
            label: |app, id| Box::pin(async move { label(&app, &$kind, &id).await }),
            search: |app, query, limit| {
                Box::pin(async move { search(&app, &$kind, &query, limit).await })
            },
            list: |app| Box::pin(async move { list(&app, &$kind).await }),
            action: Some(|app, id, action, arg| {
                Box::pin(async move {
                    match action.as_str() {
                        "field" => field(&app, &$kind, &id, arg.as_deref()).await,
                        "subtitle" => subtitle(&app, &$kind, &id).await,
                        _ => Err(AppError::other(format!(
                            "entity kind `{}` has no action `{action}`",
                            $kind.kind
                        ))),
                    }
                })
            }),
        }
    };
}

/// The kinds of browser, on any runtime; `Module::directory` holds it on the app's.
pub(crate) fn browser<R: Runtime>(directory: &mut Directory<R>) {
    directory.provide(WORKSPACE.kind, provider!(WORKSPACE));
    directory.provide(PROFILE.kind, provider!(PROFILE));
    directory.provide(PROXY.kind, provider!(PROXY));
}

/// The kind of ssh, on any runtime.
pub(crate) fn ssh<R: Runtime>(directory: &mut Directory<R>) {
    directory.provide(SSH.kind, provider!(SSH));
}

/// Publish the label of the row `id` of `kind` as it is now, or retract it
/// when the row is gone: on the connection of the transaction that changed
/// the row.
pub(crate) async fn relabel(
    directory: &Directory,
    conn: &mut SqliteConnection,
    kind: &Kind,
    id: &str,
) -> CmdResult<()> {
    // Safe: table and columns are constants of this file; the id is bound.
    let sql = format!("{} WHERE id = ?", select(kind));
    let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(AppError::db)?;
    match row {
        Some(row) => directory.publish(conn, &label_of(kind, row)).await,
        None => directory.retract(conn, kind.kind, id).await,
    }
}

/// `relabel` of each of `ids`.
pub(crate) async fn relabel_all(
    directory: &Directory,
    conn: &mut SqliteConnection,
    kind: &Kind,
    ids: &[String],
) -> CmdResult<()> {
    for id in ids {
        relabel(directory, conn, kind, id).await?;
    }
    Ok(())
}

/// `relabel` in a transaction of its own, after sync wrote or removed a row.
async fn relabel_synced(core: &Core, kind: &Kind, id: &str) -> CmdResult<()> {
    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    relabel(&core.directory, &mut tx, kind, id).await?;
    if kind.kind == WORKSPACE.kind {
        // A workspace that goes leaves its profiles in none.
        let orphans: Vec<String> =
            sqlx::query_scalar("SELECT id FROM profiles WHERE workspace_id IS NULL")
                .fetch_all(&mut *tx)
                .await
                .map_err(AppError::db)?;
        relabel_all(&core.directory, &mut tx, &PROFILE, &orphans).await?;
    }
    tx.commit().await.map_err(AppError::db)
}

/// The hooks `after_row` of the entities `workspace`, `profile`, `proxy`
/// and `ssh_connection`.
pub(crate) fn workspace_synced<'a>(core: &'a Core, id: &'a str) -> BoxFuture<'a, CmdResult<()>> {
    Box::pin(relabel_synced(core, &WORKSPACE, id))
}

pub(crate) fn profile_synced<'a>(core: &'a Core, id: &'a str) -> BoxFuture<'a, CmdResult<()>> {
    Box::pin(relabel_synced(core, &PROFILE, id))
}

pub(crate) fn proxy_synced<'a>(core: &'a Core, id: &'a str) -> BoxFuture<'a, CmdResult<()>> {
    Box::pin(relabel_synced(core, &PROXY, id))
}

pub(crate) fn ssh_synced<'a>(core: &'a Core, id: &'a str) -> BoxFuture<'a, CmdResult<()>> {
    Box::pin(relabel_synced(core, &SSH, id))
}

/// `SELECT id, name, color, parent FROM <table>`; the last two are NULL for
/// a kind without them.
fn select(kind: &Kind) -> String {
    format!(
        "SELECT id, name, {}, {} FROM {}",
        kind.color.unwrap_or("NULL"),
        kind.parent.map_or("NULL", |(_, column)| column),
        kind.table
    )
}

type Row = (String, String, Option<String>, Option<String>);

fn label_of(kind: &Kind, (id, name, color, parent): Row) -> Label {
    Label {
        kind: kind.kind.to_owned(),
        id,
        name,
        parent: kind
            .parent
            .zip(parent)
            .map(|((parent_kind, _), id)| (parent_kind.to_owned(), id)),
        color,
    }
}

async fn rows<R: Runtime>(
    app: &AppHandle<R>,
    kind: &Kind,
    sql: String,
    binds: &[&str],
) -> Vec<Label> {
    // Safe: table and columns are constants of this file; values are bound.
    let mut query = sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(sql));
    for value in binds {
        query = query.bind(*value);
    }
    query
        .fetch_all(&app.state::<Core>().db)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|row| label_of(kind, row))
        .collect()
}

async fn label<R: Runtime>(app: &AppHandle<R>, kind: &Kind, id: &str) -> Option<Label> {
    let sql = format!("{} WHERE id = ?", select(kind));
    rows(app, kind, sql, &[id]).await.pop()
}

async fn list<R: Runtime>(app: &AppHandle<R>, kind: &Kind) -> Vec<Label> {
    let sql = format!("{} ORDER BY {}", select(kind), kind.order);
    rows(app, kind, sql, &[]).await
}

/// Up to `limit` rows whose name or subtitle matches `query` as
/// `LIKE '%query%'` does, by name byte by byte and then by id
/// (`Provider::search`): one query.
async fn search<R: Runtime>(
    app: &AppHandle<R>,
    kind: &Kind,
    query: &str,
    limit: usize,
) -> Vec<Label> {
    let sql = format!(
        "{} WHERE lower(name) LIKE ? OR lower({}) LIKE ? ORDER BY name, id LIMIT {}",
        select(kind),
        kind.subtitle,
        i64::try_from(limit).unwrap_or(i64::MAX)
    );
    let pattern = veydan_core::like_pattern(query);
    rows(app, kind, sql, &[&pattern, &pattern]).await
}

/// The subtitle of the row `id`.
async fn subtitle<R: Runtime>(
    app: &AppHandle<R>,
    kind: &Kind,
    id: &str,
) -> Result<String, AppError> {
    // Safe: table and expression are constants of this file; the id is bound.
    let sql = format!("SELECT {} FROM {} WHERE id = ?", kind.subtitle, kind.table);
    sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(&app.state::<Core>().db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found(format!("{} {id}", kind.kind)))
}

/// A public field of the row `id` as text; an empty string where it has none.
async fn field<R: Runtime>(
    app: &AppHandle<R>,
    kind: &Kind,
    id: &str,
    field: Option<&str>,
) -> Result<String, AppError> {
    let Some(field) = field.and_then(|f| kind.fields.iter().find(|known| **known == f)) else {
        return Err(AppError::other(format!(
            "{} has no field {}",
            kind.kind,
            field.unwrap_or_default()
        )));
    };
    // Safe: table and column come from the constants of this file; the id is bound.
    let sql = format!(
        "SELECT CAST({field} AS TEXT) FROM {} WHERE id = ?",
        kind.table
    );
    let value: Option<Option<String>> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(&app.state::<Core>().db)
        .await
        .map_err(AppError::db)?;
    match value {
        Some(value) => Ok(value.unwrap_or_default()),
        None => Err(AppError::not_found(format!("{} {id}", kind.kind))),
    }
}
