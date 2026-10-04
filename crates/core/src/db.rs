// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The data file `<data dir>/app.db`: one SQLite database carrying the app's
//! marker in `PRAGMA application_id` and, in the table `schema_modules`, the
//! version of the schema of every module that keeps tables in it.
//!
//! Open rule: no file — it is created with the marker and the schemas; a
//! marked file whose modules are not ahead of this build — it is opened and
//! the steps it lacks are applied; anything else is neither opened nor
//! modified: an unmarked SQLite database, a journal left without its database
//! (`OpenError::Foreign`), a file of ours that keeps no module versions or
//! holds a module newer than this build knows (`OpenError::OtherSchema`).

use anyhow::Result;
use sqlx::{sqlite::SqlitePoolOptions, Connection, Pool, Sqlite, SqliteConnection};
use std::path::{Path, PathBuf};

/// Name of the data file inside the data directory.
pub const DB_FILE: &str = "app.db";
/// `PRAGMA application_id` of a data file written by this app: ASCII "VEYD".
pub const APPLICATION_ID: u32 = 0x5645_5944;

/// The tables of one module. `steps[i]` takes the module's schema from
/// version `i` to `i + 1`; a step may hold several statements. Steps create
/// and alter structure only and, once released, are never edited: a change is
/// a new step.
#[derive(Debug, Clone, Copy)]
pub struct Schema {
    pub module: &'static str,
    pub steps: &'static [&'static str],
}

/// The tables of the core itself.
pub const SCHEMA: Schema = Schema {
    module: "core",
    steps: &[
        // Generic key-value settings table
        "CREATE TABLE app_settings (
            key   TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL
        )",
        // The names of entities, kept for a product without their owner
        // (`directory`); `key` is `<kind>:<id>`.
        "CREATE TABLE labels (
            key         TEXT PRIMARY KEY NOT NULL,
            kind        TEXT NOT NULL DEFAULT '',
            id          TEXT NOT NULL DEFAULT '',
            name        TEXT NOT NULL DEFAULT '',
            parent_kind TEXT,
            parent_id   TEXT,
            color       TEXT
        );
        CREATE INDEX labels_kind ON labels (kind);",
    ],
};

/// Which version of its schema each module has reached in this file. Created
/// with the file, in the transaction that sets the marker.
const VERSIONS_TABLE: &str = "CREATE TABLE schema_modules (
    module  TEXT PRIMARY KEY NOT NULL,
    version INTEGER NOT NULL
)";

/// The SQLite header: magic string first, then big-endian `application_id` at 68.
const HEADER_LEN: usize = 100;
const HEADER_MAGIC: &[u8; 16] = b"SQLite format 3\0";
const APPLICATION_ID_OFFSET: usize = 68;

/// Files SQLite keeps beside a database and reads when it opens one.
const SIDECARS: [&str; 3] = ["-journal", "-wal", "-shm"];

#[derive(Debug)]
pub enum OpenError {
    /// The file at this path is not this app's data: the data file itself, or
    /// a journal of a database that is no longer there. It was left as it is.
    Foreign(PathBuf),
    /// The data file is ours but its schema is not one this build opens; the
    /// text says why. It was left as it is.
    OtherSchema(String),
    Failed(anyhow::Error),
}

impl From<anyhow::Error> for OpenError {
    fn from(e: anyhow::Error) -> Self {
        Self::Failed(e)
    }
}

impl From<std::io::Error> for OpenError {
    fn from(e: std::io::Error) -> Self {
        Self::Failed(e.into())
    }
}

impl From<sqlx::Error> for OpenError {
    fn from(e: sqlx::Error) -> Self {
        Self::Failed(e.into())
    }
}

#[derive(Debug, PartialEq, Eq)]
enum FileKind {
    Absent,
    Marked,
    Foreign,
}

/// What sits at `path`, told from the file header alone: SQLite is not asked,
/// so nothing is created, replayed or locked.
fn classify(path: &Path) -> std::io::Result<FileKind> {
    use std::io::Read;
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(FileKind::Absent),
        Err(e) => return Err(e),
    };
    if !meta.is_file() {
        return Ok(FileKind::Foreign);
    }
    let mut header = Vec::with_capacity(HEADER_LEN);
    std::fs::File::open(path)?
        .take(HEADER_LEN as u64)
        .read_to_end(&mut header)?;
    if header.len() < HEADER_LEN || &header[..HEADER_MAGIC.len()] != HEADER_MAGIC {
        return Ok(FileKind::Foreign);
    }
    let mut marker = [0u8; 4];
    marker.copy_from_slice(&header[APPLICATION_ID_OFFSET..APPLICATION_ID_OFFSET + 4]);
    Ok(if u32::from_be_bytes(marker) == APPLICATION_ID {
        FileKind::Marked
    } else {
        FileKind::Foreign
    })
}

/// True when the file at `path` is a data file a build with these schemas
/// opens. The file is read, never written.
pub async fn is_data_file(path: &Path, schemas: &[Schema]) -> bool {
    if !matches!(classify(path), Ok(FileKind::Marked)) {
        return false;
    }
    let url = format!("sqlite://{}?mode=ro", path.display());
    let Ok(mut conn) = SqliteConnection::connect(&url).await else {
        return false;
    };
    let opens = pending(&mut conn, schemas).await.is_ok();
    let _ = conn.close().await;
    opens
}

/// Open the data file by the rule above and bring the schema of every module
/// in `schemas` to the version this build knows. A module the file knows and
/// `schemas` does not name keeps its tables and its version.
pub async fn open(db_path: &Path, schemas: &[Schema]) -> Result<Pool<Sqlite>, OpenError> {
    for (i, schema) in schemas.iter().enumerate() {
        if schemas[..i].iter().any(|s| s.module == schema.module) {
            return Err(
                anyhow::anyhow!("schema of module `{}` is listed twice", schema.module).into(),
            );
        }
    }
    match classify(db_path)? {
        FileKind::Foreign => return Err(OpenError::Foreign(db_path.to_owned())),
        FileKind::Absent => {
            // A journal without its database belongs to a file that was moved
            // away. SQLite would replay it into the new file, and deleting it
            // would lose what the moved database had not yet taken in.
            if let Some(orphan) = SIDECARS
                .iter()
                .map(|sidecar| with_suffix(db_path, sidecar))
                .find(|path| path.exists())
            {
                return Err(OpenError::Foreign(orphan));
            }
            create(db_path, schemas).await?
        }
        FileKind::Marked => {}
    }
    let pool = connect(db_path).await?;
    match migrate(&pool, schemas).await {
        Ok(()) => Ok(pool),
        Err(e) => {
            pool.close().await;
            Err(e)
        }
    }
}

async fn connect(db_path: &Path) -> Result<Pool<Sqlite>> {
    let url = format!("sqlite://{}?mode=rwc", db_path.display());
    Ok(SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await?)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// A new data file. It is built under another name and renamed into place, so
/// a start that dies half-way leaves no unmarked `app.db` behind.
async fn create(db_path: &Path, schemas: &[Schema]) -> Result<()> {
    let staging = with_suffix(db_path, ".new");
    // What an interrupted creation left. Only the staging name is ours to remove.
    let stale = std::iter::once(staging.clone()).chain(
        SIDECARS
            .iter()
            .map(|sidecar| with_suffix(&staging, sidecar)),
    );
    for path in stale {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }

    let pool = connect(&staging).await?;
    let built = create_schema(&pool, schemas).await;
    pool.close().await;
    built?;
    if classify(&staging)? != FileKind::Marked {
        anyhow::bail!("new data file {} has no marker", staging.display());
    }
    std::fs::rename(&staging, db_path)?;
    Ok(())
}

/// Marker, tables and versions in one transaction.
async fn create_schema(pool: &Pool<Sqlite>, schemas: &[Schema]) -> Result<()> {
    let mut tx = pool.begin().await?;
    // Safe: the statement is built from a constant of this file; a PRAGMA
    // takes no bound parameter.
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "PRAGMA application_id = {APPLICATION_ID}"
    )))
    .execute(&mut *tx)
    .await?;
    sqlx::query(VERSIONS_TABLE).execute(&mut *tx).await?;
    for schema in schemas {
        apply(&mut tx, schema, 0).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Apply the steps each module lacks, all in one transaction. Nothing is
/// written when the file is already where this build is, or when it is a
/// file this build does not open.
async fn migrate(pool: &Pool<Sqlite>, schemas: &[Schema]) -> Result<(), OpenError> {
    let mut tx = pool.begin().await?;
    for (schema, version) in pending(&mut tx, schemas).await? {
        apply(&mut tx, &schema, version).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// The modules whose schema in the file is behind this build, each with the
/// version the file holds. `OtherSchema` when the file keeps no module
/// versions or holds a module newer than this build knows.
async fn pending(
    conn: &mut SqliteConnection,
    schemas: &[Schema],
) -> Result<Vec<(Schema, usize)>, OpenError> {
    let has_versions: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_modules'",
    )
    .fetch_optional(&mut *conn)
    .await?;
    if has_versions.is_none() {
        return Err(OpenError::OtherSchema(
            "it keeps no versions of module schemas".into(),
        ));
    }

    let mut behind = Vec::new();
    for schema in schemas {
        let stored: Option<i64> =
            sqlx::query_scalar("SELECT version FROM schema_modules WHERE module = ?")
                .bind(schema.module)
                .fetch_optional(&mut *conn)
                .await?;
        let version = usize::try_from(stored.unwrap_or(0)).map_err(|_| {
            anyhow::anyhow!("module `{}` has a negative schema version", schema.module)
        })?;
        if version > schema.steps.len() {
            return Err(OpenError::OtherSchema(format!(
                "module `{}` is at schema version {version}, this build knows {}",
                schema.module,
                schema.steps.len()
            )));
        }
        if version < schema.steps.len() {
            behind.push((*schema, version));
        }
    }
    Ok(behind)
}

/// Run the steps of `schema` from version `from` on and record the version reached.
async fn apply(conn: &mut SqliteConnection, schema: &Schema, from: usize) -> Result<()> {
    for (index, step) in schema.steps.iter().enumerate().skip(from) {
        sqlx::raw_sql(*step)
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                anyhow::anyhow!("schema of module `{}`, step {index}: {e}", schema.module)
            })?;
    }
    sqlx::query(
        "INSERT INTO schema_modules (module, version) VALUES (?, ?)
         ON CONFLICT(module) DO UPDATE SET version = excluded.version",
    )
    .bind(schema.module)
    .bind(schema.steps.len() as i64)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ITEMS: Schema = Schema {
        module: "items",
        steps: &["CREATE TABLE items (id TEXT PRIMARY KEY NOT NULL, title TEXT NOT NULL)"],
    };
    /// `ITEMS` one release later.
    const ITEMS_2: Schema = Schema {
        module: "items",
        steps: &[
            ITEMS.steps[0],
            "ALTER TABLE items ADD COLUMN color TEXT NOT NULL DEFAULT 'red';
             CREATE INDEX idx_items_color ON items(color)",
        ],
    };
    const EXTRA: Schema = Schema {
        module: "extra",
        steps: &["CREATE TABLE extras (id TEXT PRIMARY KEY NOT NULL)"],
    };

    async fn pragma(pool: &Pool<Sqlite>, name: &str) -> i64 {
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!("PRAGMA {name}")))
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn versions(pool: &Pool<Sqlite>) -> Vec<(String, i64)> {
        sqlx::query_as("SELECT module, version FROM schema_modules ORDER BY module")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    async fn tables(pool: &Pool<Sqlite>) -> Vec<String> {
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    fn names(dir: &Path) -> Vec<std::ffi::OsString> {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        names.sort();
        names
    }

    /// A SQLite database written by something else, with the given marker.
    async fn foreign_db(path: &Path, application_id: u32) {
        let pool = connect(path).await.unwrap();
        for sql in [
            format!("PRAGMA application_id = {application_id}"),
            "CREATE TABLE notes (id TEXT PRIMARY KEY, title TEXT)".into(),
            "INSERT INTO notes VALUES ('n1', 'kept')".into(),
        ] {
            sqlx::query(sqlx::AssertSqlSafe(sql))
                .execute(&pool)
                .await
                .unwrap();
        }
        pool.close().await;
    }

    #[tokio::test]
    async fn an_absent_file_is_created_with_marker_tables_and_versions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DB_FILE);

        let pool = open(&path, &[SCHEMA, ITEMS_2]).await.unwrap();
        assert_eq!(
            pragma(&pool, "application_id").await,
            i64::from(APPLICATION_ID)
        );
        assert_eq!(
            versions(&pool).await,
            vec![("core".to_string(), 2), ("items".to_string(), 2)]
        );
        assert_eq!(
            tables(&pool).await,
            vec!["app_settings", "items", "labels", "schema_modules"]
        );
        pool.close().await;

        assert_eq!(classify(&path).unwrap(), FileKind::Marked);
        assert_eq!(names(dir.path()), vec![std::ffi::OsString::from(DB_FILE)]);
        assert!(is_data_file(&path, &[SCHEMA, ITEMS_2]).await);
    }

    #[tokio::test]
    async fn our_file_is_opened_with_its_rows_and_left_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DB_FILE);
        let pool = open(&path, &[SCHEMA, ITEMS]).await.unwrap();
        sqlx::query("INSERT INTO app_settings (key, value) VALUES ('ui_locale', 'ru')")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        let before = std::fs::read(&path).unwrap();

        let pool = open(&path, &[SCHEMA, ITEMS]).await.unwrap();
        let value: String =
            sqlx::query_scalar("SELECT value FROM app_settings WHERE key = 'ui_locale'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(value, "ru");
        assert_eq!(
            versions(&pool).await,
            vec![("core".to_string(), 2), ("items".to_string(), 1)]
        );
        pool.close().await;
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    /// Not opened and not modified: same bytes, no new file beside it.
    async fn assert_left_alone(dir: &Path, path: &Path) {
        let before = std::fs::read(path).unwrap();
        assert!(
            matches!(open(path, &[SCHEMA]).await, Err(OpenError::Foreign(named)) if named == path)
        );
        assert_eq!(std::fs::read(path).unwrap(), before);
        assert_eq!(names(dir), vec![std::ffi::OsString::from(DB_FILE)]);
        assert!(!is_data_file(path, &[SCHEMA]).await);
    }

    #[tokio::test]
    async fn a_database_without_a_marker_is_foreign() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DB_FILE);
        foreign_db(&path, 0).await;
        assert_left_alone(dir.path(), &path).await;
    }

    #[tokio::test]
    async fn a_database_with_another_marker_is_foreign() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DB_FILE);
        foreign_db(&path, 0x1234_5678).await;
        assert_left_alone(dir.path(), &path).await;
    }

    #[tokio::test]
    async fn a_file_that_is_no_database_is_foreign() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DB_FILE);
        for content in [&b""[..], &b"not a database"[..]] {
            std::fs::write(&path, content).unwrap();
            assert_left_alone(dir.path(), &path).await;
        }
    }

    /// The journals of a database that was moved away: deleting them would
    /// lose what that database had not yet taken in.
    #[tokio::test]
    async fn a_journal_without_its_database_is_foreign() {
        for sidecar in SIDECARS {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(DB_FILE);
            let orphan = with_suffix(&path, sidecar);
            std::fs::write(&orphan, b"frames of a database that is gone").unwrap();

            assert!(
                matches!(open(&path, &[SCHEMA]).await, Err(OpenError::Foreign(named)) if named == orphan)
            );
            assert_eq!(
                std::fs::read(&orphan).unwrap(),
                b"frames of a database that is gone"
            );
            assert_eq!(names(dir.path()), vec![orphan.file_name().unwrap()]);
        }
    }

    #[tokio::test]
    async fn a_start_that_died_while_creating_starts_over() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DB_FILE);
        std::fs::write(with_suffix(&path, ".new"), b"half written").unwrap();
        std::fs::write(with_suffix(&path, ".new-journal"), b"of the half").unwrap();

        let pool = open(&path, &[SCHEMA]).await.unwrap();
        assert_eq!(
            pragma(&pool, "application_id").await,
            i64::from(APPLICATION_ID)
        );
        assert_eq!(versions(&pool).await, vec![("core".to_string(), 2)]);
        pool.close().await;
        assert_eq!(names(dir.path()), vec![std::ffi::OsString::from(DB_FILE)]);
    }

    /// A file of ours from before module versions were kept: the marker, the
    /// tables, a number in `user_version`, no `schema_modules`.
    #[tokio::test]
    async fn our_file_without_module_versions_is_not_opened() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DB_FILE);
        let pool = connect(&path).await.unwrap();
        for sql in [
            format!("PRAGMA application_id = {APPLICATION_ID}"),
            "PRAGMA user_version = 1".into(),
            SCHEMA.steps[0].into(),
            "INSERT INTO app_settings (key, value) VALUES ('ui_locale', 'ru')".into(),
        ] {
            sqlx::query(sqlx::AssertSqlSafe(sql))
                .execute(&pool)
                .await
                .unwrap();
        }
        pool.close().await;
        assert_eq!(classify(&path).unwrap(), FileKind::Marked);

        let before = std::fs::read(&path).unwrap();
        assert!(matches!(
            open(&path, &[SCHEMA]).await,
            Err(OpenError::OtherSchema(_))
        ));
        assert!(!is_data_file(&path, &[SCHEMA]).await);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(names(dir.path()), vec![std::ffi::OsString::from(DB_FILE)]);
    }

    #[tokio::test]
    async fn a_step_added_to_a_module_is_applied_to_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DB_FILE);
        let pool = open(&path, &[SCHEMA, ITEMS]).await.unwrap();
        sqlx::query("INSERT INTO items (id, title) VALUES ('i1', 'kept')")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        assert!(is_data_file(&path, &[SCHEMA, ITEMS_2]).await);

        let pool = open(&path, &[SCHEMA, ITEMS_2]).await.unwrap();
        assert_eq!(
            versions(&pool).await,
            vec![("core".to_string(), 2), ("items".to_string(), 2)]
        );
        let row: (String, String) = sqlx::query_as("SELECT title, color FROM items")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row, ("kept".to_string(), "red".to_string()));
        pool.close().await;
    }

    #[tokio::test]
    async fn a_module_added_to_the_build_gets_its_tables_in_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DB_FILE);
        open(&path, &[SCHEMA]).await.unwrap().close().await;

        let pool = open(&path, &[SCHEMA, EXTRA]).await.unwrap();
        assert_eq!(
            versions(&pool).await,
            vec![("core".to_string(), 2), ("extra".to_string(), 1)]
        );
        assert_eq!(
            tables(&pool).await,
            vec!["app_settings", "extras", "labels", "schema_modules"]
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn a_module_removed_from_the_build_keeps_its_tables_and_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DB_FILE);
        let pool = open(&path, &[SCHEMA, ITEMS, EXTRA]).await.unwrap();
        sqlx::query("INSERT INTO extras (id) VALUES ('e1')")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        let before = std::fs::read(&path).unwrap();

        let pool = open(&path, &[SCHEMA, ITEMS]).await.unwrap();
        assert_eq!(
            versions(&pool).await,
            vec![
                ("core".to_string(), 2),
                ("extra".to_string(), 1),
                ("items".to_string(), 1)
            ]
        );
        let kept: String = sqlx::query_scalar("SELECT id FROM extras")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(kept, "e1");
        pool.close().await;
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    /// An older build started on the data of a newer one.
    #[tokio::test]
    async fn a_file_ahead_of_the_build_is_not_opened() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DB_FILE);
        open(&path, &[SCHEMA, ITEMS_2]).await.unwrap().close().await;

        let before = std::fs::read(&path).unwrap();
        let refused = open(&path, &[SCHEMA, ITEMS, EXTRA]).await;
        assert!(
            matches!(&refused, Err(OpenError::OtherSchema(why)) if why.contains("`items`")),
            "{refused:?}"
        );
        assert!(!is_data_file(&path, &[SCHEMA, ITEMS, EXTRA]).await);
        // Not even the module the file lacks was added.
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(names(dir.path()), vec![std::ffi::OsString::from(DB_FILE)]);
    }

    #[tokio::test]
    async fn a_failing_step_leaves_the_file_where_it_was() {
        const BROKEN: Schema = Schema {
            module: "items",
            steps: &[
                ITEMS.steps[0],
                "ALTER TABLE items ADD COLUMN color TEXT",
                "ALTER TABLE no_such_table ADD COLUMN x TEXT",
            ],
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DB_FILE);
        open(&path, &[SCHEMA, ITEMS]).await.unwrap().close().await;

        let failed = open(&path, &[SCHEMA, EXTRA, BROKEN]).await;
        assert!(
            matches!(&failed, Err(OpenError::Failed(e)) if e.to_string().contains("module `items`, step 2")),
            "{failed:?}"
        );

        let pool = open(&path, &[SCHEMA, ITEMS]).await.unwrap();
        assert_eq!(
            versions(&pool).await,
            vec![("core".to_string(), 2), ("items".to_string(), 1)]
        );
        assert_eq!(
            tables(&pool).await,
            vec!["app_settings", "items", "labels", "schema_modules"]
        );
        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info('items')")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(columns, vec!["id", "title"]);
        pool.close().await;
    }

    /// A new file that cannot be built leaves nothing at the path of the data file.
    #[tokio::test]
    async fn a_failing_step_leaves_no_new_file() {
        const BROKEN: Schema = Schema {
            module: "broken",
            steps: &["CREATE TABLE"],
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DB_FILE);
        assert!(matches!(
            open(&path, &[SCHEMA, BROKEN]).await,
            Err(OpenError::Failed(_))
        ));
        assert!(!path.exists());

        open(&path, &[SCHEMA]).await.unwrap().close().await;
        assert_eq!(names(dir.path()), vec![std::ffi::OsString::from(DB_FILE)]);
    }

    #[tokio::test]
    async fn a_module_listed_twice_is_refused_before_the_file_is_touched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DB_FILE);
        assert!(matches!(
            open(&path, &[SCHEMA, ITEMS, ITEMS_2]).await,
            Err(OpenError::Failed(_))
        ));
        assert!(names(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn connections_enforce_references() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open(&dir.path().join(DB_FILE), &[SCHEMA]).await.unwrap();
        assert_eq!(pragma(&pool, "foreign_keys").await, 1);
        pool.close().await;
    }
}
