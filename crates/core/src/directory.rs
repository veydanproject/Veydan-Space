// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The entity directory: how a module names, searches and uses an entity of
//! another module without knowing which module answers.
//!
//! The owner of a kind registers a [`Provider`] for it; a consumer asks the
//! directory by (kind, id). A kind nobody in the product provides has no
//! actions, and its labels are those the table `labels` keeps.
//!
//! The table `labels` is the synced catalog of names (spec 10.2): the owner
//! publishes the label of an entity in the transaction that creates,
//! renames or deletes it — here or from sync — and a product without the
//! owner shows what sync brought. Only the name travels, with the parent
//! and the color the navigation of notes needs.

use crate::error::AppError;
use crate::BoxFuture;
use sqlx::{Pool, Sqlite, SqliteConnection};
use std::collections::HashMap;
use tauri::{AppHandle, Runtime, Wry};

/// The table of the labels, and the columns of it that travel: the payload
/// of the sync entity `label`, keyed by [`label_key`].
pub const LABELS_TABLE: &str = "labels";
pub const LABEL_COLUMNS: &[&str] = &["kind", "id", "name", "parent_kind", "parent_id", "color"];

/// The key of the label of the entity `id` of `kind`: its row in `labels`
/// and the id of its op of sync.
pub fn label_key(kind: &str, id: &str) -> String {
    format!("{kind}:{id}")
}

type LabelRow = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

const SELECT_LABELS: &str = "SELECT kind, id, name, parent_kind, parent_id, color FROM labels";

fn label_of((kind, id, name, parent_kind, parent_id, color): LabelRow) -> Label {
    Label {
        kind,
        id,
        name,
        parent: parent_kind.zip(parent_id),
        color,
    }
}

/// The pattern of `LIKE` that finds `query` anywhere in a text.
pub fn like_pattern(query: &str) -> String {
    format!("%{query}%")
}

/// What a consumer may show of an entity it does not own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    pub kind: String,
    pub id: String,
    pub name: String,
    /// Kind and id of the entity this one is shown under.
    pub parent: Option<(String, String)>,
    pub color: Option<String>,
}

/// The owner's side of one kind. Every function gets the app handle and
/// takes the state it needs from it.
pub struct Provider<R: Runtime = Wry> {
    /// The entity `id`, if it exists.
    pub label: fn(AppHandle<R>, String) -> BoxFuture<'static, Option<Label>>,
    /// Up to `limit` entities whose name or subtitle (the line a picker shows
    /// under the name, the owner's action `subtitle`) matches the
    /// query as SQLite's `LIKE '%' || query || '%'` does — `%` and `_` are
    /// wildcards, ASCII letters match in either case — ordered by name byte
    /// by byte (`ORDER BY name`: capitals first, as the picker of notes has
    /// always shown them), then by id. One query of the owner, however many
    /// entities the kind has.
    pub search: fn(AppHandle<R>, String, usize) -> BoxFuture<'static, Vec<Label>>,
    /// Every entity of the kind.
    pub list: fn(AppHandle<R>) -> BoxFuture<'static, Vec<Label>>,
    /// `None` for a kind without actions.
    pub action: Option<Action<R>>,
}

/// Perform an action (the third argument) on the entity `id` (the second)
/// with an optional argument, and return its result.
pub type Action<R = Wry> = fn(
    AppHandle<R>,
    String,
    String,
    Option<String>,
) -> BoxFuture<'static, Result<String, AppError>>;

pub struct Directory<R: Runtime = Wry> {
    providers: HashMap<&'static str, Provider<R>>,
    app: Option<AppHandle<R>>,
    /// The data file with the table `labels`.
    labels: Option<Pool<Sqlite>>,
}

impl<R: Runtime> Default for Directory<R> {
    fn default() -> Self {
        Self {
            providers: HashMap::new(),
            app: None,
            labels: None,
        }
    }
}

impl<R: Runtime> Directory<R> {
    /// Register the owner of `kind`. Called while the product is assembled;
    /// two owners of one kind are a mistake in the product and stop the start.
    pub fn provide(&mut self, kind: &'static str, provider: Provider<R>) {
        if self.providers.insert(kind, provider).is_some() {
            panic!("entity kind `{kind}` is provided twice");
        }
    }

    /// Give the directory the handle its providers are called with. Until
    /// then a kind with an owner answers as if it had no entities.
    pub fn attach(&mut self, app: AppHandle<R>) {
        self.app = Some(app);
    }

    /// Give the directory the data file whose table `labels` it answers from
    /// for kinds without an owner here; `Core::new` does. Without it those
    /// kinds have no labels and the fills write nothing.
    pub fn keep_labels(&mut self, db: Pool<Sqlite>) {
        self.labels = Some(db);
    }

    /// The kinds that have an owner in this product.
    pub fn kinds(&self) -> Vec<&'static str> {
        let mut kinds: Vec<_> = self.providers.keys().copied().collect();
        kinds.sort_unstable();
        kinds
    }

    /// Who answers for `kind`: its owner here, with the handle to call it
    /// with once there is one, or the table `labels`.
    fn source(&self, kind: &str) -> Source<'_, R> {
        match self.providers.get(kind) {
            Some(provider) => Source::Owner(self.app.clone().map(|app| (provider, app))),
            None => Source::Labels,
        }
    }

    /// The owner's label of the entity; for a kind without an owner here,
    /// the one sync brought.
    pub async fn label(&self, kind: &str, id: &str) -> Option<Label> {
        match self.source(kind) {
            Source::Owner(Some((provider, app))) => (provider.label)(app, id.to_owned()).await,
            Source::Owner(None) => None,
            Source::Labels => {
                let sql = format!("{SELECT_LABELS} WHERE key = ?");
                self.stored(sql, &[&label_key(kind, id)]).await.pop()
            }
        }
    }

    /// Up to `limit` entities matching `query` as `LIKE '%query%'` matches
    /// (see [`Provider::search`]): the owner's, on the name and the
    /// subtitle; from `labels`, on the name.
    pub async fn search(&self, kind: &str, query: &str, limit: usize) -> Vec<Label> {
        match self.source(kind) {
            Source::Owner(Some((provider, app))) => {
                (provider.search)(app, query.to_owned(), limit).await
            }
            Source::Owner(None) => Vec::new(),
            Source::Labels => {
                let sql = format!(
                    "{SELECT_LABELS} WHERE kind = ? AND lower(name) LIKE ?
                     ORDER BY name COLLATE NOCASE, id LIMIT {}",
                    i64::try_from(limit).unwrap_or(i64::MAX)
                );
                self.stored(sql, &[kind, &like_pattern(query)]).await
            }
        }
    }

    /// Every entity of the kind, in the owner's order; from `labels`, by name.
    pub async fn list(&self, kind: &str) -> Vec<Label> {
        match self.source(kind) {
            Source::Owner(Some((provider, app))) => (provider.list)(app).await,
            Source::Owner(None) => Vec::new(),
            Source::Labels => {
                let sql =
                    format!("{SELECT_LABELS} WHERE kind = ? ORDER BY name COLLATE NOCASE, id");
                self.stored(sql, &[kind]).await
            }
        }
    }

    /// Rows of `labels`; none without the data file or when it cannot be read.
    async fn stored(&self, sql: String, binds: &[&str]) -> Vec<Label> {
        let Some(db) = &self.labels else {
            return Vec::new();
        };
        // Safe: the statement is built from the constants of this file; values are bound.
        let mut query = sqlx::query_as::<_, LabelRow>(sqlx::AssertSqlSafe(sql));
        for value in binds {
            query = query.bind(*value);
        }
        query
            .fetch_all(db)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(label_of)
            .collect()
    }

    pub async fn action(
        &self,
        kind: &str,
        id: &str,
        action: &str,
        arg: Option<&str>,
    ) -> Result<String, AppError> {
        let Source::Owner(Some((provider, app))) = self.source(kind) else {
            return Err(AppError::not_found(format!("{kind} {id}")));
        };
        let Some(run) = provider.action else {
            return Err(AppError::other(format!(
                "entity kind `{kind}` has no action `{action}`"
            )));
        };
        run(
            app,
            id.to_owned(),
            action.to_owned(),
            arg.map(str::to_owned),
        )
        .await
    }

    /// The owner's side: `label` is what the entity is called now. Called on
    /// the connection of the transaction that creates or changes the entity,
    /// here or from sync. Nothing is written when the stored label is the
    /// same, so no op of sync follows; nothing either for a kind without an
    /// owner in this product: its labels are the owner's to publish.
    pub async fn publish(
        &self,
        conn: &mut SqliteConnection,
        label: &Label,
    ) -> Result<(), AppError> {
        self.put(conn, label).await.map(|_| ())
    }

    /// `publish`, telling whether it wrote.
    async fn put(&self, conn: &mut SqliteConnection, label: &Label) -> Result<bool, AppError> {
        if !self.providers.contains_key(label.kind.as_str()) {
            return Ok(false);
        }
        let key = label_key(&label.kind, &label.id);
        let stored: Option<LabelRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "{SELECT_LABELS} WHERE key = ?"
        )))
        .bind(&key)
        .fetch_optional(&mut *conn)
        .await
        .map_err(AppError::db)?;
        if stored.map(label_of).as_ref() == Some(label) {
            return Ok(false);
        }
        let (parent_kind, parent_id) = label.parent.clone().unzip();
        sqlx::query(
            "INSERT INTO labels (key, kind, id, name, parent_kind, parent_id, color)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(key) DO UPDATE SET kind = excluded.kind, id = excluded.id,
               name = excluded.name, parent_kind = excluded.parent_kind,
               parent_id = excluded.parent_id, color = excluded.color",
        )
        .bind(&key)
        .bind(&label.kind)
        .bind(&label.id)
        .bind(&label.name)
        .bind(parent_kind)
        .bind(parent_id)
        .bind(&label.color)
        .execute(&mut *conn)
        .await
        .map_err(AppError::db)?;
        Ok(true)
    }

    /// The owner's side: the entity is gone. Called on the connection of the
    /// transaction that deletes it, here or from sync; sync then carries the
    /// tombstone of the label. Nothing for a kind without an owner here.
    pub async fn retract(
        &self,
        conn: &mut SqliteConnection,
        kind: &str,
        id: &str,
    ) -> Result<(), AppError> {
        if !self.providers.contains_key(kind) {
            return Ok(());
        }
        sqlx::query("DELETE FROM labels WHERE key = ?")
            .bind(label_key(kind, id))
            .execute(&mut *conn)
            .await
            .map_err(AppError::db)?;
        Ok(())
    }

    /// The first fill of `labels`, on the first start with a registry of sync
    /// other than the last start's — so on the first start of a build with
    /// labels over data of one without: every entity each owner here lists,
    /// in one transaction. A label of an entity that is not here stays:
    /// another device may hold the entity. Returns how many labels it wrote.
    pub async fn publish_all(&self) -> Result<usize, AppError> {
        self.fill(false).await
    }

    /// The same after owners put or removed their rows wholesale — the demo
    /// data, the wipe of the data — which also retracts the labels of their
    /// kinds whose entity is gone.
    pub async fn republish_all(&self) -> Result<usize, AppError> {
        self.fill(true).await
    }

    /// Without the handle the owners list nothing, which is not that they
    /// have nothing: no fill then.
    async fn fill(&self, prune: bool) -> Result<usize, AppError> {
        let (Some(db), Some(_)) = (&self.labels, &self.app) else {
            return Ok(0);
        };
        let mut listed = Vec::new();
        for kind in self.kinds() {
            listed.push((kind, self.list(kind).await));
        }
        let mut tx = db.begin().await.map_err(AppError::db)?;
        let mut written = 0;
        for (kind, labels) in &listed {
            for label in labels {
                written += usize::from(self.put(&mut tx, label).await?);
            }
            if prune {
                let ids: Vec<&str> = labels.iter().map(|label| label.id.as_str()).collect();
                let gone = sqlx::query(
                    "DELETE FROM labels WHERE kind = ? AND id NOT IN (SELECT value FROM json_each(?))",
                )
                .bind(kind)
                .bind(serde_json::to_string(&ids).map_err(AppError::other)?)
                .execute(&mut *tx)
                .await
                .map_err(AppError::db)?;
                written += usize::try_from(gone.rows_affected()).unwrap_or(usize::MAX);
            }
        }
        tx.commit().await.map_err(AppError::db)?;
        Ok(written)
    }
}

enum Source<'a, R: Runtime> {
    /// The owner, and the handle once the shell gave one.
    Owner(Option<(&'a Provider<R>, AppHandle<R>)>),
    Labels,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::test::MockRuntime;
    use tauri::Manager;

    /// State of the owning module, the way a provider finds it.
    struct Colors(std::sync::Mutex<Vec<(&'static str, &'static str)>>);

    fn color(id: &str, name: &str) -> Label {
        Label {
            kind: "color".into(),
            id: id.into(),
            name: name.into(),
            parent: None,
            color: None,
        }
    }

    fn all(app: &AppHandle<MockRuntime>) -> Vec<Label> {
        app.state::<Colors>()
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|(id, name)| color(id, name))
            .collect()
    }

    fn colors() -> Provider<MockRuntime> {
        Provider {
            label: |app, id| Box::pin(async move { all(&app).into_iter().find(|l| l.id == id) }),
            search: |app, query, limit| {
                Box::pin(async move {
                    all(&app)
                        .into_iter()
                        .filter(|l| l.name.contains(&query))
                        .take(limit)
                        .collect()
                })
            },
            list: |app| Box::pin(async move { all(&app) }),
            action: Some(|app, id, action, arg| {
                Box::pin(async move {
                    let label = all(&app)
                        .into_iter()
                        .find(|l| l.id == id)
                        .ok_or_else(|| AppError::not_found(&id))?;
                    match action.as_str() {
                        "shout" => Ok(format!(
                            "{}{}",
                            label.name.to_uppercase(),
                            arg.unwrap_or_default()
                        )),
                        other => Err(AppError::other(format!("no action {other}"))),
                    }
                })
            }),
        }
    }

    /// A kind that can be named but not acted on.
    fn shapes() -> Provider<MockRuntime> {
        Provider {
            label: |_, _| Box::pin(async { None }),
            search: |_, _, _| Box::pin(async { Vec::new() }),
            list: |_| Box::pin(async { Vec::new() }),
            action: None,
        }
    }

    fn directory() -> (Directory<MockRuntime>, tauri::App<MockRuntime>) {
        let app = tauri::test::mock_app();
        app.manage(Colors(std::sync::Mutex::new(vec![
            ("r", "red"),
            ("g", "green"),
            ("b", "blue"),
        ])));
        let mut directory = Directory::default();
        directory.provide("color", colors());
        directory.provide("shape", shapes());
        directory.attach(app.handle().clone());
        (directory, app)
    }

    #[tokio::test]
    async fn the_owner_of_a_kind_answers_for_it() {
        let (directory, _app) = directory();
        assert_eq!(directory.kinds(), vec!["color", "shape"]);

        assert_eq!(
            directory.label("color", "g").await,
            Some(color("g", "green"))
        );
        assert_eq!(directory.label("color", "x").await, None);
        assert_eq!(
            directory.search("color", "re", 10).await,
            vec![color("r", "red"), color("g", "green")]
        );
        assert_eq!(
            directory.search("color", "re", 1).await,
            vec![color("r", "red")]
        );
        assert_eq!(directory.list("color").await.len(), 3);
        assert_eq!(
            directory
                .action("color", "b", "shout", Some("!"))
                .await
                .unwrap(),
            "BLUE!"
        );
        assert!(matches!(
            directory.action("color", "x", "shout", None).await,
            Err(AppError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn a_kind_nobody_provides_has_no_labels_and_no_actions() {
        let (directory, _app) = directory();
        assert_eq!(directory.label("note", "n1").await, None);
        assert!(directory.search("note", "", 10).await.is_empty());
        assert!(directory.list("note").await.is_empty());
        assert!(matches!(
            directory.action("note", "n1", "open", None).await,
            Err(AppError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn a_kind_without_actions_refuses_them() {
        let (directory, _app) = directory();
        assert!(matches!(
            directory.action("shape", "s1", "open", None).await,
            Err(AppError::Other(_))
        ));
    }

    #[tokio::test]
    async fn a_directory_without_the_app_answers_for_no_kind() {
        let mut directory = Directory::<MockRuntime>::default();
        directory.provide("color", colors());
        assert_eq!(directory.kinds(), vec!["color"]);
        assert_eq!(directory.label("color", "g").await, None);
        assert!(directory.list("color").await.is_empty());
    }

    #[test]
    #[should_panic(expected = "entity kind `color` is provided twice")]
    fn a_kind_has_one_owner() {
        let mut directory = Directory::<MockRuntime>::default();
        directory.provide("color", colors());
        directory.provide("color", shapes());
    }

    /// The directory of `directory()` over a data file of the core, on one
    /// connection: `total_changes()` then counts every write.
    async fn with_labels() -> (
        Directory<MockRuntime>,
        tauri::App<MockRuntime>,
        Pool<Sqlite>,
        tempfile::TempDir,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(crate::db::DB_FILE);
        crate::db::open(&path, &[crate::SCHEMA])
            .await
            .unwrap()
            .close()
            .await;
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&format!("sqlite://{}", path.display()))
            .await
            .unwrap();
        let (mut directory, app) = directory();
        directory.keep_labels(db.clone());
        (directory, app, db, dir)
    }

    async fn writes(db: &Pool<Sqlite>) -> i64 {
        sqlx::query_scalar("SELECT total_changes()")
            .fetch_one(db)
            .await
            .unwrap()
    }

    async fn rows(db: &Pool<Sqlite>) -> Vec<(String, String, String, String)> {
        sqlx::query_as(
            "SELECT key, kind, id, name || '|' || coalesce(parent_kind, '') || '|' ||
                    coalesce(parent_id, '') || '|' || coalesce(color, '')
             FROM labels ORDER BY key",
        )
        .fetch_all(db)
        .await
        .unwrap()
    }

    fn row(key: &str, kind: &str, id: &str, rest: &str) -> (String, String, String, String) {
        (key.into(), kind.into(), id.into(), rest.into())
    }

    #[tokio::test]
    async fn an_owner_publishes_in_its_transaction_and_only_what_changed() {
        let (directory, _app, db, _dir) = with_labels().await;
        let mut red = Label {
            color: Some("#f00".into()),
            parent: Some(("palette".into(), "warm".into())),
            ..color("r", "red")
        };

        // Rolled back with the owner's change.
        let mut tx = db.begin().await.unwrap();
        directory.publish(&mut tx, &red).await.unwrap();
        drop(tx);
        assert!(rows(&db).await.is_empty());

        let mut tx = db.begin().await.unwrap();
        directory.publish(&mut tx, &red).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(
            rows(&db).await,
            [row("color:r", "color", "r", "red|palette|warm|#f00")]
        );

        // The same label again writes nothing, so sync has nothing to send.
        let before = writes(&db).await;
        let mut tx = db.begin().await.unwrap();
        directory.publish(&mut tx, &red).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(writes(&db).await, before);

        red.name = "crimson".into();
        red.parent = None;
        let mut tx = db.begin().await.unwrap();
        directory.publish(&mut tx, &red).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(
            rows(&db).await,
            [row("color:r", "color", "r", "crimson|||#f00")]
        );

        let mut tx = db.begin().await.unwrap();
        directory.retract(&mut tx, "color", "r").await.unwrap();
        directory.retract(&mut tx, "color", "never").await.unwrap();
        tx.commit().await.unwrap();
        assert!(rows(&db).await.is_empty());
    }

    /// Spec 10.2: a product without the owner of a kind takes its labels from
    /// sync and neither publishes nor retracts them; it shows them, with no
    /// action.
    #[tokio::test]
    async fn a_kind_without_its_owner_here_is_named_from_the_labels() {
        let (directory, _app, db, _dir) = with_labels().await;
        sqlx::query(
            "INSERT INTO labels (key, kind, id, name, parent_kind, parent_id, color) VALUES
             ('note:n1', 'note', 'n1', 'Runbook', NULL, NULL, NULL),
             ('note:n2', 'note', 'n2', 'agenda', 'folder', 'f1', '#0f0'),
             ('workspace:w1', 'workspace', 'w1', 'Run', NULL, NULL, NULL)",
        )
        .execute(&db)
        .await
        .unwrap();

        let n2 = Label {
            kind: "note".into(),
            id: "n2".into(),
            name: "agenda".into(),
            parent: Some(("folder".into(), "f1".into())),
            color: Some("#0f0".into()),
        };
        assert_eq!(directory.label("note", "n2").await, Some(n2.clone()));
        assert_eq!(directory.label("note", "n3").await, None);
        let names = |labels: Vec<Label>| labels.into_iter().map(|l| l.name).collect::<Vec<_>>();
        assert_eq!(names(directory.list("note").await), ["agenda", "Runbook"]);
        assert_eq!(
            names(directory.search("note", "RUN", 10).await),
            ["Runbook"]
        );
        assert!(directory.search("note", "run", 0).await.is_empty());
        assert!(matches!(
            directory.action("note", "n1", "open", None).await,
            Err(AppError::NotFound(_))
        ));
        // A kind with an owner answers from the owner alone.
        assert_eq!(directory.label("color", "w1").await, None);

        let before = writes(&db).await;
        let mut tx = db.begin().await.unwrap();
        directory
            .publish(
                &mut tx,
                &Label {
                    name: "renamed".into(),
                    ..n2
                },
            )
            .await
            .unwrap();
        directory.retract(&mut tx, "note", "n1").await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(writes(&db).await, before);
        assert_eq!(rows(&db).await.len(), 3);
    }

    #[tokio::test]
    async fn the_fill_publishes_what_the_owners_list_once() {
        let (directory, app, db, _dir) = with_labels().await;
        sqlx::query(
            "INSERT INTO labels (key, kind, id, name) VALUES
             ('color:gone', 'color', 'gone', 'grey'),
             ('note:n1', 'note', 'n1', 'Runbook')",
        )
        .execute(&db)
        .await
        .unwrap();
        // Before the shell gives the handle, the owners cannot tell what they have.
        let mut unattached = Directory::<MockRuntime>::default();
        unattached.provide("color", colors());
        unattached.keep_labels(db.clone());
        assert_eq!(unattached.republish_all().await.unwrap(), 0);
        assert_eq!(rows(&db).await.len(), 2);

        assert_eq!(directory.publish_all().await.unwrap(), 3);
        assert_eq!(directory.publish_all().await.unwrap(), 0);
        let keys = |rows: Vec<(String, String, String, String)>| {
            rows.into_iter().map(|(key, ..)| key).collect::<Vec<_>>()
        };
        // Another device may hold the entity of a label that is not here.
        assert_eq!(
            keys(rows(&db).await),
            ["color:b", "color:g", "color:gone", "color:r", "note:n1"]
        );

        app.state::<Colors>().0.lock().unwrap().truncate(1);
        assert_eq!(directory.republish_all().await.unwrap(), 3);
        // The labels of a kind without an owner here are not the fill's.
        assert_eq!(keys(rows(&db).await), ["color:r", "note:n1"]);
        assert_eq!(directory.republish_all().await.unwrap(), 0);
    }
}
