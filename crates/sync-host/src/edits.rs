// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A local edit goes out soon, not at the next tick of the interval: a row
//! written to a table the cycle publishes (`Registry::edited_tables`) starts
//! a cycle once the edits have been quiet for `QUIET`, at the latest
//! `MAX_WAIT` after the first, so a series of edits goes in one cycle. An
//! edit while a cycle runs gives one cycle after it, never a queue. An edit
//! is heard when its transaction commits; the writes of the cycle's own task
//! are no edits (`Own`), those of the user beside it are. A write to the
//! settings is one only when a shared key differs from what was synced
//! (`Registry::checked_tables`): the cycle and the app write local keys
//! there too. A file of a note added or removed is told by `edited`. On a
//! phone the app going to the
//! background or coming back starts a cycle at once (`app_moved`): what was
//! just written leaves before the system stops the app, what the others
//! wrote comes in.

use crate::cycle::{run_cycle, vault_active};
use crate::debug::trace_skip;
use crate::registry::Registry;
use crate::rows;
use crate::SyncManager;
use sqlx::{Pool, Sqlite};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use tokio::sync::Notify;
use veydan_core::Core;

/// How long the edits stay quiet before their cycle.
pub const QUIET: Duration = Duration::from_secs(4);
/// The longest the first of edits that keep coming waits. With the 6 s the
/// editor waits at the latest before it saves typing that never pauses, a
/// key reaches the vault within 15 s.
pub const MAX_WAIT: Duration = Duration::from_secs(8);
/// How often a cycle that waits looks whether the running one ended.
const RUNNING_POLL: Duration = Duration::from_millis(500);

const MOVED_NONE: u8 = 0;
const MOVED_HIDDEN: u8 = 1;
const MOVED_SHOWN: u8 = 2;

/// The tables heard.
struct Tables {
    /// A write to these is an edit.
    edited: HashSet<&'static str>,
    /// A write to these may be one: their rows are compared with what was synced.
    checked: HashSet<&'static str>,
}

struct Edits {
    /// Set once the scheduler starts; before, no write is heard.
    tables: OnceLock<Tables>,
    /// Edits heard since the start.
    count: AtomicU64,
    /// `count` when the collect of the last cycle started: what it took.
    sent: AtomicU64,
    /// Writes to the checked tables heard since the start.
    checked: AtomicU64,
    /// `checked` when they were last compared or collected.
    checked_seen: AtomicU64,
    /// The app is in the background: an edit there goes at once.
    background: AtomicBool,
    /// The app went to the background or came back since the last cycle.
    moved: AtomicU8,
    wake: Notify,
}

static EDITS: Edits = Edits::new();

impl Edits {
    const fn new() -> Self {
        Self {
            tables: OnceLock::new(),
            count: AtomicU64::new(0),
            sent: AtomicU64::new(0),
            checked: AtomicU64::new(0),
            checked_seen: AtomicU64::new(0),
            background: AtomicBool::new(false),
            moved: AtomicU8::new(MOVED_NONE),
            wake: Notify::const_new(),
        }
    }

    fn hear(&self, table: &str) {
        let Some(tables) = self.tables.get() else {
            return;
        };
        if tables.edited.contains(table) {
            self.count.fetch_add(1, Ordering::SeqCst);
        } else if tables.checked.contains(table) {
            self.checked.fetch_add(1, Ordering::SeqCst);
        } else {
            return;
        }
        self.wake.notify_one();
    }

    /// An edit outside the data file: a file written or removed.
    fn edited(&self) {
        if self.tables.get().is_none() {
            return;
        }
        self.count.fetch_add(1, Ordering::SeqCst);
        self.wake.notify_one();
    }

    /// The edits heard so far are what the collect that starts now reads:
    /// each was heard as its commit began, and `settled` waits for the
    /// commits that began. Should it fail, they stay unsent.
    async fn collecting(&self, db: &Pool<Sqlite>) {
        let heard = self.count.load(Ordering::SeqCst);
        let checked = self.checked.load(Ordering::SeqCst);
        match veydan_core::db::settled(db).await {
            Ok(()) => {
                self.sent.store(heard, Ordering::SeqCst);
                self.checked_seen.store(checked, Ordering::SeqCst);
            }
            Err(e) => eprintln!("sync: waiting for the edits that commit: {e}"),
        }
    }

    fn unsent(&self) -> bool {
        self.count.load(Ordering::SeqCst) > self.sent.load(Ordering::SeqCst)
    }

    /// Whether the writes to the checked tables heard since they were last
    /// looked at left rows that differ from what was synced. Writes heard
    /// after the look began are looked at the next time.
    async fn checked_unsent(&self, db: &Pool<Sqlite>, tables: &[&crate::registry::Table]) -> bool {
        let heard = self.checked.load(Ordering::SeqCst);
        if self.checked_seen.swap(heard, Ordering::SeqCst) >= heard {
            return false;
        }
        if let Err(e) = veydan_core::db::settled(db).await {
            eprintln!("sync: waiting for the edits that commit: {e}");
            return true;
        }
        for table in tables {
            match rows::unsent(db, table).await {
                Ok(false) => {}
                Ok(true) => return true,
                Err(e) => {
                    eprintln!(
                        "sync: comparing {} with what was synced: {e}",
                        table.spec.table
                    );
                    return true;
                }
            }
        }
        false
    }
}

/// A transaction that wrote rows of `table` commits
/// (`veydan_core::db::on_write`). Inside the commit.
fn heard(table: &str) {
    EDITS.hear(table);
}

/// While held, the writes of the cycle's task are not edits: what it applies
/// from the vault. The cycle runs inside `veydan_core::db::hearing`.
pub(crate) type Own = veydan_core::db::Unheard;

/// The collect of a cycle starts: the edits committed so far go with it.
pub(crate) async fn collecting(db: &Pool<Sqlite>) {
    EDITS.collecting(db).await;
}

/// A local edit that is no row of the data file, such as a file of a note
/// added or removed, starts a cycle as one is.
pub fn edited() {
    EDITS.edited();
}

/// The app went to the background (`shown` false) or came back to the front.
/// A cycle starts at once, after the running one if one runs.
pub fn app_moved(shown: bool) {
    EDITS.background.store(!shown, Ordering::SeqCst);
    let moved = if shown { MOVED_SHOWN } else { MOVED_HIDDEN };
    EDITS.moved.store(moved, Ordering::SeqCst);
    EDITS.wake.notify_one();
}

/// Hear the writes to the tables the cycle publishes and start their cycles.
pub(crate) fn start(app: AppHandle, registry: &Registry) {
    let tables = Tables {
        edited: registry.edited_tables(),
        checked: registry
            .checked_tables()
            .iter()
            .map(|t| t.spec.table)
            .collect(),
    };
    if EDITS.tables.set(tables).is_err() {
        return;
    }
    veydan_core::db::on_write(heard);
    tauri::async_runtime::spawn(async move {
        let core = app.state::<Core>();
        let sync = app.state::<SyncManager>();
        loop {
            EDITS.wake.notified().await;
            let first = Instant::now();
            while EDITS.moved.load(Ordering::SeqCst) == MOVED_NONE
                && !EDITS.background.load(Ordering::SeqCst)
            {
                let left = MAX_WAIT.saturating_sub(first.elapsed()).min(QUIET);
                if left.is_zero()
                    || tokio::time::timeout(left, EDITS.wake.notified())
                        .await
                        .is_err()
                {
                    break;
                }
            }
            while sync.is_running() {
                tokio::time::sleep(RUNNING_POLL).await;
            }
            let source = match EDITS.moved.swap(MOVED_NONE, Ordering::SeqCst) {
                MOVED_HIDDEN => "app-hidden",
                MOVED_SHOWN => "app-shown",
                _ if EDITS.unsent() => "edit",
                _ if EDITS
                    .checked_unsent(&core.db, &sync.registry().checked_tables())
                    .await =>
                {
                    "edit"
                }
                _ => {
                    trace_skip(&app, "edit", "edit", "the last cycle took the edits");
                    continue;
                }
            };
            if !sync.is_ready() || !vault_active(&core).await {
                continue;
            }
            // A cycle the scheduler started meanwhile takes the edits as well.
            if let Err(e) = run_cycle(&app, source).await {
                if !sync.is_running() {
                    eprintln!("sync: {e}");
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Hooks, Table, TableSpec};
    use crate::state::{save_row_state, RowSyncState};
    use crate::system::SETTING_ENTITY;
    use veydan_core::db::{hearing, on_write, open, Schema, SCHEMA};
    use veydan_core::settings;

    static EDITS: Edits = Edits::new();

    fn heard(table: &str) {
        EDITS.hear(table);
    }

    const NOTES: Schema = Schema {
        module: "notes",
        steps: &["CREATE TABLE notes (id TEXT PRIMARY KEY); \
                  CREATE TABLE unpublished (id TEXT PRIMARY KEY);"],
    };

    async fn pool(dir: &std::path::Path) -> Pool<Sqlite> {
        let _ = EDITS.tables.set(Tables {
            edited: HashSet::from(["notes"]),
            checked: HashSet::from(["app_settings"]),
        });
        on_write(heard);
        open(
            &dir.join(veydan_core::db::DB_FILE),
            &[SCHEMA, crate::SCHEMA, NOTES],
        )
        .await
        .unwrap()
    }

    async fn insert(db: impl sqlx::SqliteExecutor<'_>, table: &str, id: &str) {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO {table} VALUES (?)"
        )))
        .bind(id.to_owned())
        .execute(db)
        .await
        .unwrap();
    }

    /// The settings as the registry of a desktop has them, with one shared key.
    fn settings_table() -> Table {
        Table {
            spec: TableSpec {
                pk: "key",
                ..TableSpec::plain(SETTING_ENTITY, "app_settings", &["value"])
            },
            hooks: Hooks::default(),
            push: true,
            owner: "sync",
            filter: Some("key IN ('ui_locale')".into()),
            ids: Some(vec!["ui_locale"]),
        }
    }

    // One test: `EDITS` and the listener are one for the whole run.
    #[tokio::test]
    async fn an_edit_counts_when_it_commits_and_the_cycles_own_writes_do_not() {
        let dir = tempfile::tempdir().unwrap();
        let db = pool(dir.path()).await;
        EDITS.collecting(&db).await;

        insert(&db, "unpublished", "k").await;
        assert!(!EDITS.unsent(), "a table the cycle does not publish");

        let mut tx = db.begin().await.unwrap();
        insert(&mut *tx, "notes", "rolled-back").await;
        tx.rollback().await.unwrap();
        assert!(!EDITS.unsent(), "a transaction rolled back");

        // The cycle applies while the user writes beside it.
        let (held, hold) = tokio::sync::oneshot::channel::<()>();
        let (wrote, written) = tokio::sync::oneshot::channel::<()>();
        let cycle_db = db.clone();
        let cycle = tokio::spawn(hearing(async move {
            let _own = Own::new();
            let mut conn = cycle_db.acquire().await.unwrap();
            held.send(()).unwrap();
            written.await.unwrap();
            insert(&mut *conn, "notes", "applied").await;
        }));
        hold.await.unwrap();
        insert(&db, "notes", "typed").await;
        wrote.send(()).unwrap();
        cycle.await.unwrap();
        assert_eq!(EDITS.count.load(Ordering::SeqCst), 1, "only the user's");
        assert!(EDITS.unsent());
        EDITS.collecting(&db).await;
        assert!(!EDITS.unsent(), "the collect took it");

        // Written before the collect, committed while it starts: its reads
        // wait for the commit, and the edit is not taken as sent.
        let mut tx = db.begin().await.unwrap();
        insert(&mut *tx, "notes", "late").await;
        let commit = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            tx.commit().await.unwrap();
        });
        EDITS.collecting(&db).await;
        let seen: Option<(String,)> = sqlx::query_as("SELECT id FROM notes WHERE id = 'late'")
            .fetch_optional(&db)
            .await
            .unwrap();
        assert!(seen.is_some());
        assert!(
            EDITS.unsent(),
            "heard after the collect began: one cycle more"
        );
        commit.await.unwrap();
        EDITS.collecting(&db).await;

        // A file of a note is an edit as a row is.
        EDITS.edited();
        assert!(EDITS.unsent(), "an attachment added");
        EDITS.collecting(&db).await;
        assert!(!EDITS.unsent());

        // The settings: a local key, such as the cycle's, is no edit; a
        // shared one is while it differs from what was synced.
        let table = settings_table();
        let tables = [&table];
        settings::set(&db, "sync_last_applied", "3").await.unwrap();
        assert!(!EDITS.unsent(), "a setting is no row edit");
        assert!(!EDITS.checked_unsent(&db, &tables).await, "a local key");
        settings::set(&db, "ui_locale", "ru").await.unwrap();
        assert!(EDITS.checked_unsent(&db, &tables).await, "the language set");
        assert!(
            !EDITS.checked_unsent(&db, &tables).await,
            "nothing written since it was looked at"
        );
        let payload = rows::read_row(&db, &table, "ui_locale")
            .await
            .unwrap()
            .unwrap();
        save_row_state(
            &db,
            &RowSyncState {
                entity: SETTING_ENTITY.into(),
                id: "ui_locale".into(),
                head_hlc: None,
                synced_hash: rows::payload_hash(&payload),
                deleted: false,
            },
        )
        .await
        .unwrap();
        settings::set(&db, "ui_locale", "ru").await.unwrap();
        settings::set(&db, "sync_last_applied", "4").await.unwrap();
        assert!(
            !EDITS.checked_unsent(&db, &tables).await,
            "the value the vault has"
        );
        settings::delete(&db, "ui_locale").await.unwrap();
        assert!(EDITS.checked_unsent(&db, &tables).await, "the key removed");
        settings::set(&db, "ui_locale", "en").await.unwrap();
        EDITS.collecting(&db).await;
        assert!(
            !EDITS.checked_unsent(&db, &tables).await,
            "the collect took it"
        );

        // The cycle that took it failed: the language is still unsent, and
        // the error the cycle keeps of itself starts no cycle after it.
        assert!(rows::unsent(&db, &table).await.unwrap());
        crate::cycle::keep(&db, "sync_last_error", "storage unreachable")
            .await
            .unwrap();
        assert!(
            !EDITS.checked_unsent(&db, &tables).await,
            "the cycle's error, the setting unsent"
        );
        db.close().await;
    }
}
