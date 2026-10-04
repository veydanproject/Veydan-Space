// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The names of entities by kind and id, for the UI of a module that links
//! to entities of other modules (docs/platform-spec.md 10.3): the tag
//! `profile:<id>` of a password is shown by the name of the profile, also in
//! a product that has no browser. The directory answers: the owner of the
//! kind where the product has one, the synced table `labels` where it has
//! not. The name leaves, and the label's color where it is a hex color (a
//! workspace's: a password linked to it shows the color it has in Space) —
//! no parent, no field of the owner.
//!
//! The lock is not asked, as the directory does not ask it: a name is not
//! under the key of the vault (10.2), and the commands that list entities
//! (`password_list`, `note_binding_summaries`) answer the same names with
//! the lock closed. The command reads and never writes.

use tauri::Runtime;
use veydan_core::{AppError, CmdResult, Core, Directory};

/// The most references one call may ask for; the UI sends its questions in
/// batches of this size (`LABELS_BATCH` in `ui/src/lib/core/foreign-labels.svelte.ts`).
pub const MAX_ITEMS: usize = 1000;

/// The longest kind or id that is looked up, as the ids of sync (`valid_id`
/// in sync-host): a longer one names nothing and costs no query.
pub const MAX_LEN: usize = 128;

/// A reference to an entity: the two halves of a tag `kind:id`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct LabelRef {
    pub kind: String,
    pub id: String,
}

/// The name of the entity a reference points at; `None` when the directory
/// has none — sync has not brought the label yet, the entity is gone, or
/// the kind is not one of the directory. `color` is the label's color when
/// it is one the app writes (`#rgb` or `#rrggbb`, see [`hex_color`]) and the
/// entity has a name; `None` otherwise.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LabelName {
    pub kind: String,
    pub id: String,
    pub name: Option<String>,
    pub color: Option<String>,
}

/// The color when it is one the app writes — a workspace's is `#rrggbb`,
/// from its palette or the color picker; `#rgb` is accepted too — as it is;
/// `None` for anything else. A label comes from sync: whatever another
/// device put in it, only a plain hex color reaches a `style` of the UI.
pub fn hex_color(color: &str) -> Option<String> {
    let hex = color.strip_prefix('#')?;
    let ok = matches!(hex.len(), 3 | 6) && hex.bytes().all(|b| b.is_ascii_hexdigit());
    ok.then(|| color.to_string())
}

/// The names (and colors) of `items`, one answer per item, in their order.
#[tauri::command]
pub async fn labels_resolve(
    items: Vec<LabelRef>,
    core: tauri::State<'_, Core>,
) -> CmdResult<Vec<LabelName>> {
    resolve(&core.directory, items).await
}

/// The body of the command. A batch above [`MAX_ITEMS`] is refused whole; a
/// kind or an id above [`MAX_LEN`] bytes has no name. A label whose name is
/// empty (a mangled op of sync leaves one, 10.2) names nothing and gives no
/// color.
pub async fn resolve<R: Runtime>(
    directory: &Directory<R>,
    items: Vec<LabelRef>,
) -> CmdResult<Vec<LabelName>> {
    if items.len() > MAX_ITEMS {
        return Err(AppError::Other(format!(
            "labels_resolve takes at most {MAX_ITEMS} references, got {}",
            items.len()
        )));
    }
    let mut out = Vec::with_capacity(items.len());
    for LabelRef { kind, id } in items {
        let label = if kind.len() > MAX_LEN || id.len() > MAX_LEN {
            None
        } else {
            directory
                .label(&kind, &id)
                .await
                .filter(|label| !label.name.trim().is_empty())
        };
        let (name, color) = match label {
            Some(label) => (Some(label.name), label.color.as_deref().and_then(hex_color)),
            None => (None, None),
        };
        out.push(LabelName {
            kind,
            id,
            name,
            color,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::test::MockRuntime;
    use veydan_core::{db, Label, Provider};
    use veydan_lock::Lock;

    fn item(kind: &str, id: &str) -> LabelRef {
        LabelRef {
            kind: kind.into(),
            id: id.into(),
        }
    }

    fn named(kind: &str, id: &str, name: Option<&str>) -> LabelName {
        colored(kind, id, name, None)
    }

    fn colored(kind: &str, id: &str, name: Option<&str>, color: Option<&str>) -> LabelName {
        LabelName {
            kind: kind.into(),
            id: id.into(),
            name: name.map(str::to_string),
            color: color.map(str::to_string),
        }
    }

    /// The owner of the kind `password` here: one entry, `pw-1`.
    fn passwords() -> Provider<MockRuntime> {
        Provider {
            label: |_, id| {
                Box::pin(async move {
                    (id == "pw-1").then(|| Label {
                        kind: "password".into(),
                        id,
                        name: "Mail".into(),
                        parent: None,
                        color: None,
                    })
                })
            },
            search: |_, _, _| Box::pin(async { Vec::new() }),
            list: |_| Box::pin(async { Vec::new() }),
            action: None,
        }
    }

    /// A product like Pass: it owns `password`, and sync brought the labels
    /// of a workspace, of a profile in it and of a note.
    struct Product {
        directory: Directory<MockRuntime>,
        db: sqlx::Pool<sqlx::Sqlite>,
        _app: tauri::App<MockRuntime>,
        _dir: tempfile::TempDir,
    }

    async fn app() -> Product {
        let dir = tempfile::tempdir().unwrap();
        let schemas = [veydan_core::SCHEMA, crate::lock::SCHEMA];
        let db = db::open(&dir.path().join(db::DB_FILE), &schemas)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO labels (key, kind, id, name, parent_kind, parent_id, color) VALUES
             ('workspace:ws-1', 'workspace', 'ws-1', 'SMM', NULL, NULL, '#22c55e'),
             ('profile:pr-1', 'profile', 'pr-1', 'Brand A', 'workspace', 'ws-1', NULL),
             ('note:n-1', 'note', 'n-1', '', NULL, NULL, NULL),
             ('password:pw-9', 'password', 'pw-9', 'Stale', NULL, NULL, NULL)",
        )
        .execute(&db)
        .await
        .unwrap();
        let app = tauri::test::mock_app();
        let mut directory = Directory::default();
        directory.provide("password", passwords());
        directory.attach(app.handle().clone());
        directory.keep_labels(db.clone());
        Product {
            directory,
            db,
            _app: app,
            _dir: dir,
        }
    }

    async fn ask(app: &Product, items: Vec<LabelRef>) -> Vec<LabelName> {
        resolve(&app.directory, items).await.unwrap()
    }

    #[tokio::test]
    async fn a_kind_without_its_owner_is_named_from_the_labels() {
        let app = app().await;
        assert_eq!(
            ask(
                &app,
                vec![item("profile", "pr-1"), item("workspace", "ws-1")]
            )
            .await,
            [
                named("profile", "pr-1", Some("Brand A")),
                colored("workspace", "ws-1", Some("SMM"), Some("#22c55e")),
            ]
        );
        assert!(ask(&app, Vec::new()).await.is_empty());
    }

    #[tokio::test]
    async fn a_reference_without_a_label_has_no_name() {
        let app = app().await;
        assert_eq!(
            ask(&app, vec![item("profile", "pr-2"), item("note", "n-1")]).await,
            [named("profile", "pr-2", None), named("note", "n-1", None)]
        );
    }

    /// The color of a label reaches the UI when it is a hex color the app
    /// writes, as it is; anything else sync may bring — a name of a color,
    /// CSS after the color, a variable, no `#`, a wrong length — and the
    /// color of a label without a name stay behind. A label without a color
    /// has none.
    #[tokio::test]
    async fn a_label_gives_its_hex_color_and_nothing_else() {
        let app = app().await;
        let colors = [
            ("c-1", "#22C55E"),
            ("c-2", "#abc"),
            ("c-3", "red"),
            ("c-4", "#22c55e; background: url(https://x.example)"),
            ("c-5", "var(--accent)"),
            ("c-6", "22c55e"),
            ("c-7", "#22c55e0"),
            ("c-8", "#ggg"),
            ("c-9", " #22c55e"),
            ("c-10", ""),
        ];
        for (id, color) in colors {
            sqlx::query(
                "INSERT INTO labels (key, kind, id, name, parent_kind, parent_id, color)
                 VALUES (?, 'workspace', ?, 'WS', NULL, NULL, ?)",
            )
            .bind(format!("workspace:{id}"))
            .bind(id)
            .bind(color)
            .execute(&app.db)
            .await
            .unwrap();
        }
        sqlx::query(
            "INSERT INTO labels (key, kind, id, name, parent_kind, parent_id, color)
             VALUES ('workspace:c-nameless', 'workspace', 'c-nameless', ' ', NULL, NULL, '#123456')",
        )
        .execute(&app.db)
        .await
        .unwrap();
        let mut items: Vec<LabelRef> = colors.iter().map(|(id, _)| item("workspace", id)).collect();
        items.push(item("workspace", "c-nameless"));
        items.push(item("profile", "pr-1"));
        let answers = ask(&app, items).await;
        let got: Vec<(&str, Option<&str>)> = answers
            .iter()
            .map(|a| (a.id.as_str(), a.color.as_deref()))
            .collect();
        assert_eq!(
            got,
            [
                ("c-1", Some("#22C55E")),
                ("c-2", Some("#abc")),
                ("c-3", None),
                ("c-4", None),
                ("c-5", None),
                ("c-6", None),
                ("c-7", None),
                ("c-8", None),
                ("c-9", None),
                ("c-10", None),
                ("c-nameless", None),
                ("pr-1", None),
            ]
        );
        assert_eq!(answers[10].name, None);
        assert!(answers[..10]
            .iter()
            .all(|a| a.name.as_deref() == Some("WS")));
    }

    #[test]
    fn hex_color_takes_three_or_six_hex_digits_after_a_hash() {
        for ok in ["#fff", "#FFF", "#6366f1", "#8B7BFF"] {
            assert_eq!(hex_color(ok).as_deref(), Some(ok), "{ok}");
        }
        for bad in [
            "",
            "#",
            "#ff",
            "#ffff",
            "#fffffff",
            "fff",
            "#ggg",
            "#fff ",
            "red",
            "#ff0000)",
            "\u{ff03}fff",
        ] {
            assert_eq!(hex_color(bad), None, "{bad:?}");
        }
    }

    /// One answer per item, in the order asked: the owner's, the labels',
    /// none, a repeated one.
    #[tokio::test]
    async fn a_batch_is_answered_item_by_item() {
        let app = app().await;
        assert_eq!(
            ask(
                &app,
                vec![
                    item("password", "pw-1"),
                    item("proxy", "px-1"),
                    item("profile", "pr-1"),
                    // The owner answers for its kind, not a label left behind.
                    item("password", "pw-9"),
                    item("profile", "pr-1"),
                ]
            )
            .await,
            [
                named("password", "pw-1", Some("Mail")),
                named("proxy", "px-1", None),
                named("profile", "pr-1", Some("Brand A")),
                named("password", "pw-9", None),
                named("profile", "pr-1", Some("Brand A")),
            ]
        );
    }

    /// A kind the directory does not know, a key of `labels` passed as an
    /// id, wildcards: nothing is found and nothing fails.
    #[tokio::test]
    async fn an_unknown_kind_has_no_name() {
        let app = app().await;
        assert_eq!(
            ask(
                &app,
                vec![
                    item("domain", "example.com"),
                    item("", "profile:pr-1"),
                    item("profile", "%"),
                    item("workspace:ws-1", ""),
                ]
            )
            .await,
            [
                named("domain", "example.com", None),
                named("", "profile:pr-1", None),
                named("profile", "%", None),
                named("workspace:ws-1", "", None),
            ]
        );
    }

    /// A batch of [`MAX_ITEMS`] is answered, one more is refused whole; a
    /// kind or an id longer than [`MAX_LEN`] names nothing, even when a
    /// label with that key exists.
    #[tokio::test]
    async fn a_batch_and_its_references_are_bounded() {
        let app = app().await;
        let full = vec![item("profile", "pr-1"); MAX_ITEMS];
        let answers = ask(&app, full.clone()).await;
        assert_eq!(answers.len(), MAX_ITEMS);
        assert!(answers.iter().all(|a| a.name.as_deref() == Some("Brand A")));

        let mut over = full;
        over.push(item("workspace", "ws-1"));
        let err = resolve(&app.directory, over).await.unwrap_err();
        assert!(matches!(err, AppError::Other(_)), "{err:?}");

        let long = "x".repeat(MAX_LEN + 1);
        let edge = "y".repeat(MAX_LEN);
        for id in [&long, &edge] {
            sqlx::query(
                "INSERT INTO labels (key, kind, id, name, parent_kind, parent_id, color)
                 VALUES (?, 'profile', ?, 'Long', NULL, NULL, NULL)",
            )
            .bind(format!("profile:{id}"))
            .bind(id)
            .execute(&app.db)
            .await
            .unwrap();
        }
        assert_eq!(
            ask(
                &app,
                vec![
                    item("profile", &long),
                    item(&long, "pr-1"),
                    item("profile", &edge),
                ]
            )
            .await,
            [
                named("profile", &long, None),
                named(&long, "pr-1", None),
                named("profile", &edge, Some("Long")),
            ]
        );
    }

    /// With the lock closed the answer is the directory's, as with it open:
    /// names are not under the key. Nothing but the name and the color is in
    /// the answer, and nothing is written.
    #[tokio::test]
    async fn the_closed_lock_changes_nothing_and_only_names_and_colors_leave() {
        let app = app().await;
        let lock = Lock::new(app.db.clone());
        lock.open_default().await.unwrap();
        lock.set(Some("4821".into()), None, Some("pin".into()), None)
            .await
            .unwrap();
        let items = || vec![item("profile", "pr-1"), item("workspace", "ws-1")];
        let open = ask(&app, items()).await;
        lock.lock();
        assert!(lock.is_locked().await);
        let rows = || async {
            sqlx::query_as::<_, (String, String)>("SELECT key, name FROM labels ORDER BY key")
                .fetch_all(&app.db)
                .await
                .unwrap()
        };
        let before = rows().await;
        let closed = ask(&app, items()).await;
        assert_eq!(closed, open);
        for (answer, item) in closed.iter().zip(items()) {
            let label = app.directory.label(&item.kind, &item.id).await;
            assert_eq!(answer.name, label.map(|l| l.name));
        }
        assert_eq!(rows().await, before);
        assert_eq!(
            serde_json::to_value(&closed).unwrap(),
            serde_json::json!([
                { "kind": "profile", "id": "pr-1", "name": "Brand A", "color": null },
                { "kind": "workspace", "id": "ws-1", "name": "SMM", "color": "#22c55e" },
            ])
        );
    }
}
