// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The tables this app keeps in the data file, one schema per module that
//! owns them, and what every start does to their rows. The file itself — its
//! marker, the open rule, the versions of the schemas — is `veydan_core::db`.

#[cfg(desktop)]
use crate::modules::directory::{relabel, relabel_all, PROFILE, WORKSPACE};
use anyhow::Result;
use sqlx::{Pool, Sqlite};
#[cfg(any(desktop, test))]
use std::path::Path;
use veydan_core::{Directory, Schema};

#[cfg(test)]
use veydan_core::db::{OpenError, DB_FILE};

/// The schemas of this app, as the shell collects them from its modules: every
/// table once, with its owner.
///
/// Connections of the pool enforce the `REFERENCES` clauses (sqlx turns
/// `foreign_keys` on), but the delete paths do not lean on `ON DELETE`: each
/// clears its own references explicitly — `ssh_key_delete`,
/// `ssh_connection_delete`, `proxy_delete`, and the matching `Delete::Plain`
/// statements the modules register for sync. A *dangling* reference is
/// deliberately not repaired by nulling it: `proxies::resolve_required`
/// refuses to connect, because silently dropping a proxy reference would
/// downgrade the connection to a direct one and leak the real IP. See
/// `prune_orphan_links` for the rows that genuinely are garbage.
#[cfg(any(desktop, test))]
pub(crate) fn schemas() -> Vec<Schema> {
    veydan_shell::schemas(&crate::modules::all())
}

pub(crate) const BROWSER: Schema = Schema {
    module: "browser",
    steps: &[concat!(
        "CREATE TABLE workspaces (
            id TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            description TEXT,
            color TEXT NOT NULL DEFAULT '#6366f1',
            icon TEXT NOT NULL DEFAULT 'folder',
            notes TEXT,
            is_default INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
        "CREATE TABLE workspace_columns (
            id TEXT PRIMARY KEY NOT NULL,
            workspace_id TEXT NOT NULL REFERENCES workspaces(id),
            name TEXT NOT NULL,
            tag_name TEXT NOT NULL,
            color TEXT NOT NULL DEFAULT '#6366f1',
            position INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL
        );",
        "CREATE TABLE profiles (
            id TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'stopped',
            profile_path TEXT NOT NULL,
            browser_type TEXT NOT NULL DEFAULT 'camoufox',
            proxy_id TEXT,
            fingerprint_preset TEXT NOT NULL DEFAULT 'linux',
            user_agent TEXT,
            platform TEXT,
            timezone TEXT,
            locale TEXT NOT NULL DEFAULT 'en-US',
            languages TEXT NOT NULL DEFAULT 'en-US,en',
            screen_width INTEGER NOT NULL DEFAULT 1920,
            screen_height INTEGER NOT NULL DEFAULT 1080,
            webrtc_mode TEXT NOT NULL DEFAULT 'disable',
            geolocation_enabled INTEGER NOT NULL DEFAULT 0,
            latitude REAL,
            longitude REAL,
            notes TEXT,
            workspace_id TEXT REFERENCES workspaces(id),
            kanban_status TEXT NOT NULL DEFAULT 'new',
            kanban_order INTEGER NOT NULL DEFAULT 0,
            tags TEXT NOT NULL DEFAULT '[]',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            last_launch_at TEXT,
            webgl_vendor TEXT,
            webgl_renderer TEXT,
            default_search_engine TEXT NOT NULL DEFAULT 'ddg',
            history_enabled INTEGER NOT NULL DEFAULT 1
        );",
        "CREATE TABLE proxies (
            id TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            proxy_type TEXT NOT NULL DEFAULT 'socks5',
            host TEXT NOT NULL,
            port INTEGER NOT NULL,
            username TEXT,
            password TEXT,
            country TEXT,
            city TEXT,
            status TEXT NOT NULL DEFAULT 'unknown',
            last_ip TEXT,
            last_check_at TEXT,
            created_at TEXT NOT NULL,
            private_key TEXT,
            server_fingerprint TEXT,
            tags TEXT NOT NULL DEFAULT '[]'
        );",
        // Firefox profile files: last pushed/applied snapshot, lease holder, pending work.
        "CREATE TABLE sync_profile_files_state (
            profile_id       TEXT PRIMARY KEY NOT NULL,
            head_hlc         TEXT NOT NULL DEFAULT '',
            synced_hash      TEXT NOT NULL DEFAULT '',
            manifest_json    TEXT NOT NULL DEFAULT '',
            snapshot_at      TEXT NOT NULL DEFAULT '',
            dirty            INTEGER NOT NULL DEFAULT 0,
            pending_manifest TEXT NOT NULL DEFAULT '',
            lease_device     TEXT NOT NULL DEFAULT '',
            lease_name       TEXT NOT NULL DEFAULT '',
            lease_since      TEXT NOT NULL DEFAULT '',
            lease_hlc        TEXT NOT NULL DEFAULT '',
            lease_synced     INTEGER NOT NULL DEFAULT 1,
            diverged         INTEGER NOT NULL DEFAULT 0
        );",
    )],
};

/// Its tables reference those of `BROWSER` (workspaces, proxies): the two
/// modules live in this crate and exist only together.
pub(crate) const SSH: Schema = Schema {
    module: "ssh",
    steps: &[concat!(
        // Stored SSH keys (referenced by ssh_connections.ssh_key_id)
        "CREATE TABLE ssh_keys (
            id          TEXT PRIMARY KEY NOT NULL,
            name        TEXT NOT NULL,
            algorithm   TEXT NOT NULL,
            bits        INTEGER,
            comment     TEXT,
            private_key TEXT NOT NULL,
            public_key  TEXT NOT NULL,
            passphrase  TEXT,
            fingerprint TEXT,
            source      TEXT NOT NULL DEFAULT 'imported',
            created_at  TEXT NOT NULL,
            updated_at  TEXT NOT NULL
        );",
        // `server_fingerprint`: SHA256 of the server host key, pinned on first
        // successful connection (TOFU) — same scheme as proxies.server_fingerprint.
        "CREATE TABLE ssh_connections (
            id                  TEXT PRIMARY KEY NOT NULL,
            name                TEXT NOT NULL,
            host                TEXT NOT NULL,
            port                INTEGER NOT NULL DEFAULT 22,
            username            TEXT NOT NULL,
            auth_type           TEXT NOT NULL DEFAULT 'password',
            password            TEXT,
            private_key         TEXT,
            key_passphrase      TEXT,
            requires_2fa        INTEGER NOT NULL DEFAULT 0,
            totp_entry_id       TEXT,
            proxy_id            TEXT REFERENCES proxies(id) ON DELETE SET NULL,
            connect_timeout_sec INTEGER NOT NULL DEFAULT 15,
            keepalive_sec       INTEGER NOT NULL DEFAULT 30,
            terminal_theme      TEXT,
            default_cols        INTEGER NOT NULL DEFAULT 120,
            default_rows        INTEGER NOT NULL DEFAULT 32,
            last_connected_at   TEXT,
            created_at          TEXT NOT NULL,
            updated_at          TEXT NOT NULL,
            ssh_key_id          TEXT REFERENCES ssh_keys(id) ON DELETE SET NULL,
            server_fingerprint  TEXT
        );",
        "CREATE INDEX idx_ssh_conn_key ON ssh_connections(ssh_key_id);",
        // Many-to-many: SSH connection ↔ workspace
        "CREATE TABLE ssh_connection_workspaces (
            connection_id TEXT NOT NULL REFERENCES ssh_connections(id) ON DELETE CASCADE,
            workspace_id  TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            PRIMARY KEY (connection_id, workspace_id)
        );",
        "CREATE INDEX idx_ssh_conn_ws ON ssh_connection_workspaces(workspace_id);",
        // Many-to-many: SSH connection ↔ profile
        "CREATE TABLE ssh_connection_profiles (
            connection_id TEXT NOT NULL REFERENCES ssh_connections(id) ON DELETE CASCADE,
            profile_id    TEXT NOT NULL,
            PRIMARY KEY (connection_id, profile_id)
        );",
        "CREATE INDEX idx_ssh_conn_pr ON ssh_connection_profiles(profile_id);",
    )],
};

/// True when the file at `path` is a data file this build opens.
#[cfg(desktop)]
pub async fn is_data_file(path: &Path) -> bool {
    veydan_core::db::is_data_file(path, &schemas()).await
}

/// Open the data file with the schemas of this app, then do what every start
/// does to its rows (`prepare`): what the shell and the setup of the browser
/// module do in the app.
#[cfg(test)]
pub async fn open(db_path: &Path) -> Result<Pool<Sqlite>, OpenError> {
    let pool = veydan_core::db::open(db_path, &schemas()).await?;
    prepare(&pool, &Directory::default()).await?;
    Ok(pool)
}

/// What every start does to the rows; what it changes of a workspace or a
/// profile, the labels of `directory` follow.
#[cfg_attr(mobile, allow(unused_variables))]
pub(crate) async fn prepare(pool: &Pool<Sqlite>, directory: &Directory) -> Result<()> {
    // No browser is running yet, whatever the last run left in the table.
    sqlx::query("UPDATE profiles SET status = 'stopped' WHERE status = 'running'")
        .execute(pool)
        .await?;

    let mut tx = pool.begin().await?;
    // The desktop always has a workspace to put profiles into. A phone has
    // no workspaces of its own.
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM workspaces")
        .fetch_one(&mut *tx)
        .await?;
    if count == 0 && cfg!(desktop) {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO workspaces (id, name, description, color, icon, is_default, created_at, updated_at)
             VALUES ('default', 'Default', NULL, '#6366f1', 'folder', 1, ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await?;
        #[cfg(desktop)]
        relabel(directory, &mut tx, &WORKSPACE, "default").await?;
    }

    // Sync detaches a profile whose workspace is gone or has not arrived; the
    // lists are per workspace, so such a profile goes to the default one.
    let moved: Vec<String> = sqlx::query_scalar(
        "UPDATE profiles SET workspace_id = 'default'
         WHERE workspace_id IS NULL AND EXISTS (SELECT 1 FROM workspaces WHERE id = 'default')
         RETURNING id",
    )
    .fetch_all(&mut *tx)
    .await?;
    #[cfg(desktop)]
    relabel_all(directory, &mut tx, &PROFILE, &moved).await?;
    tx.commit().await?;

    prune_orphan_links(pool).await?;

    Ok(())
}

/// Drop link rows whose parent is gone.
///
/// `ssh_connection_profiles.profile_id` references nothing, so deleting a
/// profile leaves its link rows behind, and sync writes the link ids a peer
/// sent whether or not the profile is here. They are pure garbage: every read
/// joins through them, so they change no behaviour — but they keep
/// accumulating.
///
/// Deliberately *not* included: dangling `profiles.proxy_id` /
/// `ssh_connections.proxy_id`. Nulling those would turn "should use a proxy"
/// into "connects directly", which is the leak `proxies::resolve_required`
/// exists to prevent. Those rows stay as they are and refuse to connect until
/// the user picks a proxy or explicitly clears the setting.
async fn prune_orphan_links(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    for sql in [
        "DELETE FROM ssh_connection_workspaces
         WHERE connection_id NOT IN (SELECT id FROM ssh_connections)
            OR workspace_id   NOT IN (SELECT id FROM workspaces)",
        "DELETE FROM ssh_connection_profiles
         WHERE connection_id NOT IN (SELECT id FROM ssh_connections)
            OR profile_id     NOT IN (SELECT id FROM profiles)",
    ] {
        let removed = sqlx::query(sql).execute(pool).await?.rows_affected();
        if removed > 0 {
            eprintln!("[db] pruned {removed} orphaned SSH link row(s)");
        }
    }
    Ok(())
}

/// A fresh directory for a test's data file. The directory is the caller's to remove.
#[cfg(test)]
pub(crate) fn test_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("veydan-db-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A data file of its own, for tests of code that needs the real schema.
#[cfg(test)]
pub(crate) async fn test_pool() -> (Pool<Sqlite>, std::path::PathBuf) {
    let dir = test_dir();
    let pool = open(&dir.join(DB_FILE)).await.unwrap();
    (pool, dir)
}

/// The tables of pass as the product crate created them at platform-stage-4,
/// before they moved with their module into `veydan_pass`.
#[cfg(test)]
const STAGE_4_PASS: Schema = Schema {
    module: "pass",
    steps: &[concat!(
        "CREATE TABLE passwords (
            id           TEXT PRIMARY KEY NOT NULL,
            title        TEXT NOT NULL,
            username     TEXT,
            url          TEXT,
            password_enc TEXT NOT NULL,
            note_enc     TEXT,
            totp_ids     TEXT NOT NULL DEFAULT '[]',
            tags         TEXT NOT NULL DEFAULT '[]',
            vault_id     TEXT NOT NULL,
            created_at   TEXT NOT NULL,
            updated_at   TEXT NOT NULL
        );",
        "CREATE TABLE totp_entries (
            id          TEXT PRIMARY KEY NOT NULL,
            name        TEXT NOT NULL,
            issuer      TEXT,
            secret      TEXT NOT NULL,
            algorithm   TEXT NOT NULL DEFAULT 'SHA1',
            digits      INTEGER NOT NULL DEFAULT 6,
            period      INTEGER NOT NULL DEFAULT 30,
            tags        TEXT NOT NULL DEFAULT '[]',
            created_at  TEXT NOT NULL,
            updated_at  TEXT NOT NULL,
            last_used_at TEXT
        );",
        // Passwords the generator produced
        "CREATE TABLE password_history (
            id TEXT PRIMARY KEY NOT NULL,
            password TEXT NOT NULL,
            created_at TEXT NOT NULL
        );",
    )],
};

/// The tables of notes as the product crate created them at
/// platform-stage-5, before they moved with their module into `veydan_notes`.
#[cfg(test)]
const STAGE_5_NOTES: Schema = Schema {
    module: "notes",
    steps: &[concat!(
        "CREATE TABLE notes (
            id           TEXT PRIMARY KEY NOT NULL,
            title        TEXT NOT NULL,
            file_path    TEXT NOT NULL,
            format       TEXT NOT NULL DEFAULT 'md',
            pinned       INTEGER NOT NULL DEFAULT 0,
            archived     INTEGER NOT NULL DEFAULT 0,
            deleted      INTEGER NOT NULL DEFAULT 0,
            doc_status   TEXT NOT NULL DEFAULT 'active',
            version_base TEXT NULL,
            fts_rowid    INTEGER NULL,
            created_at   TEXT NOT NULL,
            updated_at   TEXT NOT NULL,
            file_mtime   TEXT NULL,
            content_hash TEXT NULL,
            preview      TEXT NOT NULL DEFAULT '',
            bindings     TEXT NOT NULL DEFAULT '[]'
        );",
        "CREATE TABLE note_tags (
            id         TEXT PRIMARY KEY NOT NULL,
            name       TEXT NOT NULL UNIQUE,
            color      TEXT NOT NULL DEFAULT '#6366f1',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
        "CREATE TABLE note_tag_links (
            note_id TEXT NOT NULL,
            tag_id  TEXT NOT NULL,
            PRIMARY KEY (note_id, tag_id)
        );",
        "CREATE VIRTUAL TABLE notes_fts
         USING fts5(note_id UNINDEXED, title, content, tags);",
        "CREATE TABLE note_folders (
            id         TEXT PRIMARY KEY NOT NULL,
            name       TEXT NOT NULL,
            parent_id  TEXT NULL REFERENCES note_folders(id),
            color      TEXT NOT NULL DEFAULT '#6366f1',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
        // Many-to-many notes ↔ folders
        "CREATE TABLE note_folder_links (
            note_id   TEXT NOT NULL,
            folder_id TEXT NOT NULL,
            PRIMARY KEY (note_id, folder_id)
        );",
        // Wiki links between notes, rebuilt from the body on save
        "CREATE TABLE note_links (
            from_id TEXT NOT NULL,
            to_id   TEXT NOT NULL,
            PRIMARY KEY (from_id, to_id)
        );",
        "CREATE INDEX idx_note_links_to ON note_links(to_id);",
        // Entity mentions `[[kind:id]]` in note bodies, rebuilt from the body on save
        "CREATE TABLE note_mentions (
            note_id TEXT NOT NULL,
            binding TEXT NOT NULL,
            PRIMARY KEY (note_id, binding)
        );",
        "CREATE INDEX idx_note_mentions_binding ON note_mentions(binding);",
        // Saved filters: conditions is a NoteFilter JSON
        "CREATE TABLE note_smart_views (
            id         TEXT PRIMARY KEY NOT NULL,
            name       TEXT NOT NULL,
            color      TEXT NOT NULL DEFAULT '#8b7bff',
            conditions TEXT NOT NULL DEFAULT '{}',
            sort_order INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
        // Note version history (DAG: parent_id links versions into a tree)
        "CREATE TABLE note_history (
            id           TEXT PRIMARY KEY,
            note_id      TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
            parent_id    TEXT NULL REFERENCES note_history(id),
            revision     INTEGER NOT NULL,
            version_type TEXT NOT NULL DEFAULT 'save',
            title        TEXT NOT NULL,
            content      BLOB NOT NULL,
            content_hash TEXT NOT NULL,
            author       TEXT NULL,
            device       TEXT NULL,
            created_at   TEXT NOT NULL
        );",
        "CREATE INDEX idx_nh_note_created ON note_history(note_id, created_at DESC);",
        "CREATE INDEX idx_nh_note_revision ON note_history(note_id, revision DESC);",
        // Per-note sync position: which vault version the local file corresponds
        // to. The `conflict_*` columns hold a pending conflict: history snapshots
        // of both sides and the remote blob to merge with.
        "CREATE TABLE sync_note_state (
            note_id              TEXT PRIMARY KEY NOT NULL,
            head_blob            TEXT NOT NULL DEFAULT '',
            head_parents         TEXT NOT NULL DEFAULT '[]',
            head_hlc             TEXT NOT NULL DEFAULT '',
            synced_hash          TEXT NOT NULL DEFAULT '',
            deleted              INTEGER NOT NULL DEFAULT 0,
            conflict             INTEGER NOT NULL DEFAULT 0,
            conflict_ancestor_id TEXT NOT NULL DEFAULT '',
            conflict_local_id    TEXT NOT NULL DEFAULT '',
            conflict_remote_id   TEXT NOT NULL DEFAULT '',
            conflict_remote_blob TEXT NOT NULL DEFAULT ''
        );",
        // Per-attachment sync position: which vault blob the local file corresponds
        // to. `deferred_ref` is a chunked attachment accepted from the vault but
        // not downloaded yet (LargeFileRef JSON).
        "CREATE TABLE sync_attachment_state (
            note_id      TEXT NOT NULL,
            name         TEXT NOT NULL,
            head_blob    TEXT NOT NULL DEFAULT '',
            head_hlc     TEXT NOT NULL DEFAULT '',
            synced_hash  TEXT NOT NULL DEFAULT '',
            deleted      INTEGER NOT NULL DEFAULT 0,
            deferred_ref TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (note_id, name)
        );",
    )],
};

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pragma(pool: &Pool<Sqlite>, name: &str) -> i64 {
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!("PRAGMA {name}")))
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// A data file written before the schemas were kept per module: this
    /// app's marker and tables, a number in `user_version`, no versions of
    /// modules.
    #[tokio::test]
    async fn a_file_of_an_earlier_build_is_not_opened() {
        let dir = test_dir();
        let path = dir.join(DB_FILE);
        let url = format!("sqlite://{}?mode=rwc", path.display());
        let pool = SqlitePoolOptions::new().connect(&url).await.unwrap();
        sqlx::raw_sql(
            "PRAGMA application_id = 1447385412;
             PRAGMA user_version = 1;
             CREATE TABLE app_settings (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
             CREATE TABLE workspaces (id TEXT PRIMARY KEY NOT NULL);
             CREATE TABLE profiles (id TEXT PRIMARY KEY NOT NULL, status TEXT, workspace_id TEXT);
             INSERT INTO profiles VALUES ('p1', 'running', NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;

        let before = std::fs::read(&path).unwrap();
        assert!(matches!(open(&path).await, Err(OpenError::OtherSchema(_))));
        #[cfg(desktop)]
        assert!(!is_data_file(&path).await);
        // Not even the rows every start touches were touched.
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The note above `schemas`: a row cannot point at a parent that is not there.
    #[tokio::test]
    async fn connections_enforce_references() {
        let (pool, dir) = test_pool().await;
        assert_eq!(pragma(&pool, "foreign_keys").await, 1);
        let orphan = sqlx::query(
            "INSERT INTO workspace_columns (id, workspace_id, name, tag_name, created_at)
             VALUES ('c1', 'no-such-workspace', 'New', 'new', 't')",
        )
        .execute(&pool)
        .await;
        assert!(orphan.is_err());
        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_fresh_desktop_file_has_the_default_workspace() {
        let (pool, dir) = test_pool().await;
        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM workspaces")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(ids, vec!["default".to_string()]);
        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The schema as SQLite sees it, in a form that does not depend on how the
    /// tables were created: the marker and the version of each module's
    /// schema, then columns in order with type, null rule, default and key
    /// position, foreign keys, indexes, then the text of what has no columns.
    async fn schema_dump(pool: &Pool<Sqlite>) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "pragma application_id = {}\n",
            pragma(pool, "application_id").await
        ));
        let modules: Vec<(String, i64)> =
            sqlx::query_as("SELECT module, version FROM schema_modules ORDER BY module")
                .fetch_all(pool)
                .await
                .unwrap();
        for (module, version) in modules {
            out.push_str(&format!("module {module} = {version}\n"));
        }
        let objects: Vec<(String, String, Option<String>)> =
            sqlx::query_as("SELECT type, name, sql FROM sqlite_master ORDER BY type, name")
                .fetch_all(pool)
                .await
                .unwrap();
        for (kind, name, sql) in &objects {
            let sql = sql.as_deref().unwrap_or("");
            if kind != "table" || sql.starts_with("CREATE VIRTUAL") {
                let text = sql.split_whitespace().collect::<Vec<_>>().join(" ");
                out.push_str(&format!("{kind} {name}: {text}\n"));
                continue;
            }
            out.push_str(&format!("table {name}\n"));
            let cols: Vec<(String, String, i64, Option<String>, i64)> = sqlx::query_as(
                "SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_xinfo(?) ORDER BY cid",
            )
            .bind(name)
            .fetch_all(pool)
            .await
            .unwrap();
            for (col, ty, notnull, dflt, pk) in cols {
                let dflt = dflt.unwrap_or_else(|| "-".into());
                out.push_str(&format!(
                    "  column {col} {ty} notnull={notnull} default={dflt} pk={pk}\n"
                ));
            }
            let fks: Vec<(String, String, Option<String>, String)> = sqlx::query_as(
                "SELECT \"from\", \"table\", \"to\", on_delete FROM pragma_foreign_key_list(?) ORDER BY \"from\"",
            )
            .bind(name)
            .fetch_all(pool)
            .await
            .unwrap();
            for (from, table, to, on_delete) in fks {
                let to = to.unwrap_or_else(|| "-".into());
                out.push_str(&format!(
                    "  foreign key {from} -> {table}({to}) on delete {on_delete}\n"
                ));
            }
            let indexes: Vec<(String, i64, String)> = sqlx::query_as(
                "SELECT name, \"unique\", origin FROM pragma_index_list(?) ORDER BY name",
            )
            .bind(name)
            .fetch_all(pool)
            .await
            .unwrap();
            for (index, unique, origin) in indexes {
                let cols: Vec<String> =
                    sqlx::query_scalar("SELECT name FROM pragma_index_info(?) ORDER BY seqno")
                        .bind(&index)
                        .fetch_all(pool)
                        .await
                        .unwrap();
                out.push_str(&format!(
                    "  index {index} ({}) unique={unique} origin={origin}\n",
                    cols.join(", ")
                ));
            }
        }
        out
    }

    /// The tables of pass changed owner, from the product crate to its own,
    /// and kept the name of their module and their step. A data file of
    /// platform-stage-4 opens as it is: nothing is created again, no row is
    /// lost, a password and a TOTP entry read back.
    #[tokio::test]
    async fn a_data_file_of_stage_4_keeps_the_tables_of_pass() {
        assert_eq!(veydan_pass::SCHEMA.module, STAGE_4_PASS.module);
        assert_eq!(veydan_pass::SCHEMA.steps, STAGE_4_PASS.steps);

        // The schemas of Space at platform-stage-4, in its order.
        let stage_4: Vec<Schema> = schemas()
            .into_iter()
            .map(|schema| {
                if schema.module == "pass" {
                    STAGE_4_PASS
                } else {
                    schema
                }
            })
            .collect();
        let dir = test_dir();
        let path = dir.join(DB_FILE);
        let pool = veydan_core::db::open(&path, &stage_4).await.unwrap();
        let lock = veydan_lock::Lock::new(pool.clone());
        lock.open_default().await.unwrap();
        let (key, vault_id) = lock.ensure_key().await.unwrap();
        let enc = |field, text| veydan_lock::encrypt_field(&key, "pw-1", field, text).unwrap();
        sqlx::query(
            "INSERT INTO passwords (id, title, username, password_enc, note_enc, totp_ids, tags, vault_id, created_at, updated_at)
             VALUES ('pw-1', 'GitHub', 'octocat', ?, ?, '[\"totp-1\"]', '[\"note:n1\"]', ?, 't', 't')",
        )
        .bind(enc("password", "hunter2"))
        .bind(enc("note", "the panel"))
        .bind(&vault_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO totp_entries (id, name, issuer, secret, created_at, updated_at)
             VALUES ('totp-1', 'github-work', 'GitHub', 'JBSWY3DPEHPK3PXP', 't', 't')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO password_history (id, password, created_at) VALUES ('h-1', 'x9!kPq', 't')",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
        let before = std::fs::read(&path).unwrap();

        let pool = veydan_core::db::open(&path, &schemas()).await.unwrap();
        let versions: Vec<(String, i64)> =
            sqlx::query_as("SELECT module, version FROM schema_modules WHERE module = 'pass'")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(versions, [("pass".to_string(), 1)]);
        pool.close().await;
        // Opened as it was: no step ran, nothing was written.
        assert_eq!(std::fs::read(&path).unwrap(), before);

        let pool = veydan_core::db::open(&path, &schemas()).await.unwrap();
        let lock = veydan_lock::Lock::new(pool.clone());
        lock.open_default().await.unwrap();
        let (key, _) = lock.require_open().unwrap();
        let (password, note, totp_ids, tags): (String, String, String, String) = sqlx::query_as(
            "SELECT password_enc, note_enc, totp_ids, tags FROM passwords WHERE id = 'pw-1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            veydan_lock::decrypt_field(&key, "pw-1", "password", &password).unwrap(),
            "hunter2"
        );
        assert_eq!(
            veydan_lock::decrypt_field(&key, "pw-1", "note", &note).unwrap(),
            "the panel"
        );
        assert_eq!(
            (totp_ids.as_str(), tags.as_str()),
            ("[\"totp-1\"]", "[\"note:n1\"]")
        );
        let secret: String =
            sqlx::query_scalar("SELECT secret FROM totp_entries WHERE id = 'totp-1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(secret, "JBSWY3DPEHPK3PXP");
        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The tables of notes changed owner, from the product crate to their
    /// own, and kept the name of their module and their step. A data file of
    /// platform-stage-5 opens as it is: nothing is created again, no row is
    /// lost; a note, its tag, folder, link, history and sync position read
    /// back, and the search finds it.
    #[tokio::test]
    async fn a_data_file_of_stage_5_keeps_the_tables_of_notes() {
        assert_eq!(veydan_notes::SCHEMA.module, STAGE_5_NOTES.module);
        assert_eq!(veydan_notes::SCHEMA.steps, STAGE_5_NOTES.steps);

        // The schemas of Space at platform-stage-5, in its order.
        let stage_5: Vec<Schema> = schemas()
            .into_iter()
            .map(|schema| {
                if schema.module == "notes" {
                    STAGE_5_NOTES
                } else {
                    schema
                }
            })
            .collect();
        let dir = test_dir();
        let path = dir.join(DB_FILE);
        let pool = veydan_core::db::open(&path, &stage_5).await.unwrap();
        for statement in [
            "INSERT INTO notes (id, title, file_path, pinned, created_at, updated_at, preview, bindings)
             VALUES ('n-1', 'Deploy runbook', 'n-1.md', 1, 't', 't', 'ssh prod', '[\"ssh:c-1\"]')",
            "INSERT INTO note_tags (id, name, created_at, updated_at) VALUES ('t-1', 'ops', 't', 't')",
            "INSERT INTO note_tag_links (note_id, tag_id) VALUES ('n-1', 't-1')",
            "INSERT INTO note_folders (id, name, created_at, updated_at) VALUES ('f-1', 'Runbooks', 't', 't')",
            "INSERT INTO note_folder_links (note_id, folder_id) VALUES ('n-1', 'f-1')",
            "INSERT INTO note_links (from_id, to_id) VALUES ('n-1', 'n-2')",
            "INSERT INTO note_mentions (note_id, binding) VALUES ('n-1', 'ssh:c-1')",
            "INSERT INTO note_smart_views (id, name, created_at, updated_at) VALUES ('v-1', 'Pinned', 't', 't')",
            "INSERT INTO note_history (id, note_id, revision, title, content, content_hash, created_at)
             VALUES ('h-1', 'n-1', 1, 'Deploy runbook', x'00', 'hash', 't')",
            "INSERT INTO notes_fts (note_id, title, content, tags) VALUES ('n-1', 'Deploy runbook', 'ssh prod-web-01', 'ops')",
            "INSERT INTO sync_note_state (note_id, head_blob) VALUES ('n-1', 'b-1')",
            "INSERT INTO sync_attachment_state (note_id, name, head_blob) VALUES ('n-1', 'scan.pdf', 'b-2')",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        pool.close().await;
        let before = std::fs::read(&path).unwrap();

        let pool = veydan_core::db::open(&path, &schemas()).await.unwrap();
        let versions: Vec<(String, i64)> =
            sqlx::query_as("SELECT module, version FROM schema_modules WHERE module = 'notes'")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(versions, [("notes".to_string(), 1)]);
        pool.close().await;
        // Opened as it was: no step ran, nothing was written.
        assert_eq!(std::fs::read(&path).unwrap(), before);

        let pool = veydan_core::db::open(&path, &schemas()).await.unwrap();
        let count = |table: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!(
                    "SELECT COUNT(*) FROM {table}"
                )))
                .fetch_one(&pool)
                .await
                .unwrap()
            }
        };
        for table in [
            "notes",
            "note_tags",
            "note_tag_links",
            "note_folders",
            "note_folder_links",
            "note_links",
            "note_mentions",
            "note_smart_views",
            "note_history",
            "sync_note_state",
            "sync_attachment_state",
        ] {
            assert_eq!(count(table).await, 1, "{table}");
        }
        let (title, pinned, bindings): (String, i64, String) =
            sqlx::query_as("SELECT title, pinned, bindings FROM notes WHERE id = 'n-1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            (title.as_str(), pinned, bindings.as_str()),
            ("Deploy runbook", 1, "[\"ssh:c-1\"]")
        );
        let found: Vec<String> =
            sqlx::query_scalar("SELECT note_id FROM notes_fts WHERE notes_fts MATCH 'prod*'")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(found, ["n-1"]);
        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The schema of a fresh file is pinned by `schema.golden.txt`. After an
    /// intended change run the test with `UPDATE_SCHEMA_GOLDEN=1` and review
    /// the diff of the golden file.
    #[tokio::test]
    async fn a_fresh_schema_matches_the_golden_file() {
        let (pool, dir) = test_pool().await;
        let dump = schema_dump(&pool).await;
        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);

        let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/schema.golden.txt");
        if std::env::var_os("UPDATE_SCHEMA_GOLDEN").is_some() {
            std::fs::write(&golden, &dump).unwrap();
        }
        let expected = std::fs::read_to_string(&golden).unwrap_or_default();
        assert!(
            dump == expected,
            "the schema differs from {}:\n{dump}",
            golden.display()
        );
    }

    /// Minimal shape of the tables `prune_orphan_links` touches.
    async fn link_test_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for sql in [
            "CREATE TABLE ssh_connections (id TEXT PRIMARY KEY NOT NULL)",
            "CREATE TABLE workspaces (id TEXT PRIMARY KEY NOT NULL)",
            "CREATE TABLE profiles (id TEXT PRIMARY KEY NOT NULL)",
            "CREATE TABLE ssh_connection_workspaces (connection_id TEXT NOT NULL, workspace_id TEXT NOT NULL)",
            "CREATE TABLE ssh_connection_profiles (connection_id TEXT NOT NULL, profile_id TEXT NOT NULL)",
            "INSERT INTO ssh_connections (id) VALUES ('c-live')",
            "INSERT INTO workspaces (id) VALUES ('w-live')",
            "INSERT INTO profiles (id) VALUES ('p-live')",
            // Kept: both ends exist.
            "INSERT INTO ssh_connection_workspaces VALUES ('c-live', 'w-live')",
            "INSERT INTO ssh_connection_profiles VALUES ('c-live', 'p-live')",
            // Pruned: the connection is gone.
            "INSERT INTO ssh_connection_workspaces VALUES ('c-gone', 'w-live')",
            "INSERT INTO ssh_connection_profiles VALUES ('c-gone', 'p-live')",
            // Pruned: the other end was deleted.
            "INSERT INTO ssh_connection_workspaces VALUES ('c-live', 'w-gone')",
            "INSERT INTO ssh_connection_profiles VALUES ('c-live', 'p-gone')",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        pool
    }

    async fn count(pool: &Pool<Sqlite>, table: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM {table}")))
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn prune_orphan_links_drops_only_broken_rows() {
        let pool = link_test_pool().await;

        prune_orphan_links(&pool).await.unwrap();

        assert_eq!(count(&pool, "ssh_connection_workspaces").await, 1);
        assert_eq!(count(&pool, "ssh_connection_profiles").await, 1);

        let survivor: (String, String) =
            sqlx::query_as("SELECT connection_id, workspace_id FROM ssh_connection_workspaces")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(survivor, ("c-live".into(), "w-live".into()));
    }

    #[tokio::test]
    async fn prune_orphan_links_is_idempotent() {
        let pool = link_test_pool().await;
        prune_orphan_links(&pool).await.unwrap();
        prune_orphan_links(&pool).await.unwrap();
        assert_eq!(count(&pool, "ssh_connection_workspaces").await, 1);
        assert_eq!(count(&pool, "ssh_connection_profiles").await, 1);
    }
}
