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
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{ConnectOptions, Connection, Pool, Sqlite, SqliteConnection};
use std::cell::Cell;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

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
    let Ok(mut conn) = options(path).read_only(true).connect().await else {
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

/// Who hears of the rows written to the data file, by the name of their
/// table: once for every table a transaction wrote, when it commits. A
/// transaction rolled back and the writes of a task inside `unheard` are not
/// told. It is called inside the commit, on the thread of the connection: it
/// may run no query and must be quick.
static ON_WRITE: OnceLock<fn(&str)> = OnceLock::new();

/// Set who hears of the rows written (`ON_WRITE`): once, by the sync of the
/// product, which starts a cycle soon after a local edit. A second call
/// changes nothing.
pub fn on_write(listener: fn(&str)) {
    let _ = ON_WRITE.set(listener);
}

tokio::task_local! {
    /// The task writes what is no edit of the user (`Unheard`).
    static UNHEARD: Cell<bool>;
}

/// Run `task` so that `Unheard` inside it keeps its writes from `ON_WRITE`.
/// A task spawned from it is outside: its writes are heard.
pub async fn hearing<F: Future>(task: F) -> F::Output {
    UNHEARD.scope(Cell::new(false), task).await
}

/// While held, the queries of this task, inside `hearing`, are not told to
/// `ON_WRITE`. A connection takes it when the task gets it from the pool.
pub struct Unheard(bool);

impl Unheard {
    pub fn new() -> Self {
        Unheard(UNHEARD.try_with(|own| own.replace(true)).unwrap_or(false))
    }
}

impl Default for Unheard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Unheard {
    fn drop(&mut self) {
        let _ = UNHEARD.try_with(|own| own.set(self.0));
    }
}

/// The writes a transaction keeps until it commits.
#[derive(Default)]
struct Writes {
    tables: Vec<String>,
}

/// Hooks on `conn` for the task that got it: the tables a transaction writes
/// go to `ON_WRITE` when it commits, none when it is rolled back, and none
/// for a task inside `Unheard`.
async fn listen(conn: &mut SqliteConnection) -> sqlx::Result<()> {
    if ON_WRITE.get().is_none() {
        return Ok(());
    }
    let mut handle = conn.lock_handle().await?;
    if UNHEARD.try_with(Cell::get).unwrap_or(false) {
        handle.remove_update_hook();
        handle.remove_commit_hook();
        handle.remove_rollback_hook();
        return Ok(());
    }
    let writes = Arc::new(Mutex::new(Writes::default()));
    let on_update = writes.clone();
    handle.set_update_hook(move |row| {
        let mut writes = on_update.lock().unwrap_or_else(|e| e.into_inner());
        if !writes.tables.iter().any(|t| t == row.table) {
            writes.tables.push(row.table.to_owned());
        }
    });
    let on_commit = writes.clone();
    handle.set_commit_hook(move || {
        let tables =
            std::mem::take(&mut on_commit.lock().unwrap_or_else(|e| e.into_inner()).tables);
        if let Some(listener) = ON_WRITE.get() {
            for table in &tables {
                listener(table);
            }
        }
        // The commit goes on.
        true
    });
    handle.set_rollback_hook(move || {
        writes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tables
            .clear();
    });
    Ok(())
}

/// Wait until every transaction that has begun to commit is seen by the next
/// read: `ON_WRITE` hears a commit just before it is done, holding the lock
/// of the writer, which this takes and lets go.
pub async fn settled(pool: &Pool<Sqlite>) -> sqlx::Result<()> {
    pool.begin_with("BEGIN IMMEDIATE").await?.rollback().await
}

async fn connect(db_path: &Path) -> Result<Pool<Sqlite>> {
    Ok(SqlitePoolOptions::new()
        .max_connections(5)
        .after_connect(|conn, _| Box::pin(listen(conn)))
        .before_acquire(|conn, _| {
            Box::pin(async move {
                listen(conn).await?;
                Ok(true)
            })
        })
        .connect_with(options(db_path).create_if_missing(true))
        .await?)
}

/// The file at `path` as SQLite gets it: the path itself, not a URL, which
/// sqlx would percent-decode and cut at the first `?` — a `--workdir` or a
/// folder of the user with `%41` or `?` in its name would send the data to
/// another place than the one `classify` looked at.
pub fn options(path: &Path) -> SqliteConnectOptions {
    SqliteConnectOptions::new().filename(path)
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

    static WRITTEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

    fn written(table: &str) {
        WRITTEN.lock().unwrap().push(table.to_string());
    }

    /// How often `table` was heard. Every test writes tables of its own:
    /// the listener is one for the whole run.
    fn heard(table: &str) -> usize {
        WRITTEN
            .lock()
            .unwrap()
            .iter()
            .filter(|t| *t == table)
            .count()
    }

    const HEARD: Schema = Schema {
        module: "heard",
        steps: &["CREATE TABLE heard_commit (id TEXT PRIMARY KEY); \
                  CREATE TABLE heard_unheard (id TEXT PRIMARY KEY); \
                  CREATE TABLE heard_other (id TEXT PRIMARY KEY);"],
    };

    async fn heard_pool(dir: &Path) -> Pool<Sqlite> {
        on_write(written);
        open(&dir.join(DB_FILE), &[SCHEMA, HEARD]).await.unwrap()
    }

    async fn insert(db: impl sqlx::SqliteExecutor<'_>, table: &str, id: &str) {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO {table} (id) VALUES (?)"
        )))
        .bind(id.to_owned())
        .execute(db)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_transaction_is_heard_once_by_each_table_when_it_commits_and_never_when_rolled_back()
    {
        let dir = tempfile::tempdir().unwrap();
        let pool = heard_pool(dir.path()).await;

        let mut tx = pool.begin().await.unwrap();
        insert(&mut *tx, "heard_commit", "a").await;
        insert(&mut *tx, "heard_commit", "b").await;
        assert_eq!(heard("heard_commit"), 0, "not before COMMIT");
        tx.commit().await.unwrap();
        assert_eq!(heard("heard_commit"), 1);

        let mut tx = pool.begin().await.unwrap();
        insert(&mut *tx, "heard_commit", "c").await;
        tx.rollback().await.unwrap();
        // A statement on its own is a transaction of its own.
        insert(&pool, "heard_commit", "d").await;
        assert_eq!(heard("heard_commit"), 2, "the rollback is not heard");
        pool.close().await;
    }

    #[tokio::test]
    async fn the_writes_of_a_task_inside_unheard_are_not_heard_and_another_tasks_are() {
        let dir = tempfile::tempdir().unwrap();
        let pool = heard_pool(dir.path()).await;

        // Outside `hearing` an `Unheard` changes nothing.
        {
            let _unheard = Unheard::new();
            insert(&pool, "heard_other", "outside").await;
        }
        assert_eq!(heard("heard_other"), 1);

        let (held, hold) = tokio::sync::oneshot::channel::<()>();
        let (wrote, written_) = tokio::sync::oneshot::channel::<()>();
        let quiet = pool.clone();
        let task = tokio::spawn(hearing(async move {
            let _unheard = Unheard::new();
            // It holds a connection of its own while the other task writes.
            let mut conn = quiet.acquire().await.unwrap();
            held.send(()).unwrap();
            written_.await.unwrap();
            insert(&mut *conn, "heard_unheard", "a").await;
            drop(conn);
            let mut tx = quiet.begin().await.unwrap();
            insert(&mut *tx, "heard_unheard", "b").await;
            tx.commit().await.unwrap();
        }));
        hold.await.unwrap();
        insert(&pool, "heard_other", "beside").await;
        wrote.send(()).unwrap();
        task.await.unwrap();
        assert_eq!(heard("heard_unheard"), 0);
        assert_eq!(heard("heard_other"), 2, "the write beside it is heard");

        // Let go, the same connections are heard again.
        hearing(async {
            drop(Unheard::new());
            insert(&pool, "heard_unheard", "c").await;
        })
        .await;
        assert_eq!(heard("heard_unheard"), 1);
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

    /// A folder whose name reads as a URL — a percent escape, a `?` — holds
    /// the data file itself: nothing goes to the folder the escape names,
    /// and the file is opened again where it was made.
    #[tokio::test]
    async fn the_data_file_stays_in_a_folder_whose_name_looks_like_a_url() {
        let root = tempfile::tempdir().unwrap();
        let mut odd = vec!["a%62", "x%2541"];
        if cfg!(unix) {
            odd.push("a?b");
        }
        for name in odd {
            let dir = root.path().join(name);
            std::fs::create_dir(&dir).unwrap();
            let path = dir.join(DB_FILE);
            open(&path, &[SCHEMA]).await.unwrap().close().await;
            assert_eq!(classify(&path).unwrap(), FileKind::Marked, "{name}");
            assert_eq!(names(&dir), vec![std::ffi::OsString::from(DB_FILE)]);
            assert!(is_data_file(&path, &[SCHEMA]).await, "{name}");
            open(&path, &[SCHEMA]).await.unwrap().close().await;
        }
        // Only the folders made here: none that a decoded name points at.
        let mut made = names(root.path());
        made.sort();
        let mut expected: Vec<std::ffi::OsString> = ["a%62", "x%2541"]
            .into_iter()
            .chain(cfg!(unix).then_some("a?b"))
            .map(Into::into)
            .collect();
        expected.sort();
        assert_eq!(made, expected);
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
