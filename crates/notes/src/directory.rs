// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The kind `note` in the entity directory: what another module may know of
//! a note — its title, nothing of its text. A note in the trash is still a
//! note: it has a label and is listed; a search for a link to make finds the
//! notes that are not in the trash. A note has no actions.

use crate::KIND_NOTE;
use sqlx::SqliteConnection;
use tauri::{AppHandle, Manager, Runtime};
use veydan_core::{AppError, Core, Label, Provider};

pub(crate) fn notes<R: Runtime>() -> Provider<R> {
    Provider {
        label: |app, id| Box::pin(async move { label(&app, &id).await }),
        search: |app, query, limit| Box::pin(async move { search(&app, &query, limit).await }),
        list: |app| Box::pin(async move { list(&app).await }),
        action: None,
    }
}

fn label_of((id, title): (String, String)) -> Label {
    Label {
        kind: KIND_NOTE.to_owned(),
        id,
        name: title,
        parent: None,
        color: None,
    }
}

/// Publish the label of the note `id` as its row is now — in the trash too —
/// or retract it when the row is gone: on the connection of the transaction
/// that changed the row, whether the change was made here, came from a file
/// or from sync.
pub(crate) async fn relabel(
    core: &Core,
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<(), AppError> {
    let title: Option<String> = sqlx::query_scalar("SELECT title FROM notes WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(AppError::db)?;
    match title {
        Some(title) => {
            let label = label_of((id.to_owned(), title));
            core.directory.publish(conn, &label).await
        }
        None => core.directory.retract(conn, KIND_NOTE, id).await,
    }
}

async fn label<R: Runtime>(app: &AppHandle<R>, id: &str) -> Option<Label> {
    sqlx::query_as::<_, (String, String)>("SELECT id, title FROM notes WHERE id = ?")
        .bind(id)
        .fetch_optional(&app.state::<Core>().db)
        .await
        .ok()
        .flatten()
        .map(label_of)
}

async fn list<R: Runtime>(app: &AppHandle<R>) -> Vec<Label> {
    sqlx::query_as::<_, (String, String)>(
        "SELECT id, title FROM notes ORDER BY title COLLATE NOCASE, id",
    )
    .fetch_all(&app.state::<Core>().db)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(label_of)
    .collect()
}

/// Up to `limit` notes out of the trash whose title matches `query` as
/// `LIKE '%query%'` does (a note has no short description), by title.
async fn search<R: Runtime>(app: &AppHandle<R>, query: &str, limit: usize) -> Vec<Label> {
    sqlx::query_as::<_, (String, String)>(
        "SELECT id, title FROM notes WHERE deleted = 0 AND lower(title) LIKE ?
         ORDER BY title COLLATE NOCASE, id LIMIT ?",
    )
    .bind(veydan_core::like_pattern(query))
    .bind(i64::try_from(limit).unwrap_or(i64::MAX))
    .fetch_all(&app.state::<Core>().db)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(label_of)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;
    use tauri::test::MockRuntime;
    use veydan_core::Directory;

    #[tokio::test]
    async fn a_note_is_named_by_its_title_and_has_no_actions() {
        let (app, _dir) = testing::app().await;
        sqlx::query(
            "INSERT INTO notes (id, title, file_path, deleted, created_at, updated_at) VALUES
             ('n-1', 'Deploy runbook', 'n-1.md', 0, 't', 't'),
             ('n-2', 'deploy checklist', 'n-2.md', 0, 't', 't'),
             ('n-3', 'Old deploy', 'n-3.md', 1, 't', 't')",
        )
        .execute(&app.state::<Core>().db)
        .await
        .unwrap();
        let mut directory = Directory::<MockRuntime>::default();
        crate::provide(&mut directory);
        directory.attach(app.handle().clone());

        assert_eq!(directory.kinds(), [KIND_NOTE]);
        let names = |labels: Vec<Label>| labels.into_iter().map(|l| l.name).collect::<Vec<_>>();
        assert_eq!(
            directory.label(KIND_NOTE, "n-1").await,
            Some(Label {
                kind: "note".into(),
                id: "n-1".into(),
                name: "Deploy runbook".into(),
                parent: None,
                color: None,
            })
        );
        // In the trash, still a note.
        assert_eq!(
            directory.label(KIND_NOTE, "n-3").await.map(|l| l.name),
            Some("Old deploy".to_string())
        );
        assert_eq!(directory.label(KIND_NOTE, "nope").await, None);
        assert_eq!(
            names(directory.list(KIND_NOTE).await),
            ["deploy checklist", "Deploy runbook", "Old deploy"]
        );
        assert_eq!(
            names(directory.search(KIND_NOTE, "DEPLOY", 10).await),
            ["deploy checklist", "Deploy runbook"]
        );
        assert_eq!(
            names(directory.search(KIND_NOTE, "deploy", 1).await),
            ["deploy checklist"]
        );
        assert!(directory
            .action(KIND_NOTE, "n-1", "field", Some("title"))
            .await
            .is_err());
    }

    async fn labels(core: &Core) -> Vec<(String, String)> {
        sqlx::query_as("SELECT key, name FROM labels ORDER BY key")
            .fetch_all(&core.db)
            .await
            .unwrap()
    }

    /// Spec 10.2: a note is named in the labels when it is written, renamed,
    /// changed back from its history; the trash keeps the label, a deletion
    /// for good retracts it.
    #[tokio::test]
    async fn a_note_publishes_its_title_until_it_is_deleted_for_good() {
        use crate::crud::{note_create, note_delete, note_restore, note_update};
        use crate::models::{NoteCreateInput, NoteUpdateInput};
        let (app, _dir) = testing::app().await;
        let core = app.state::<Core>();
        let note = note_create(
            NoteCreateInput {
                title: "Runbook".into(),
                format: None,
                bindings: None,
                tag_names: None,
                content: Some("first".into()),
                template_id: None,
            },
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        let key = format!("note:{}", note.id);
        assert_eq!(labels(&core).await, [(key.clone(), "Runbook".into())]);

        let rename = |title: &str| NoteUpdateInput {
            title: Some(title.into()),
            content: None,
            pinned: None,
            base_hash: None,
        };
        note_update(
            note.id.clone(),
            rename("Deploy runbook"),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert_eq!(
            labels(&core).await,
            [(key.clone(), "Deploy runbook".into())]
        );

        note_delete(note.id.clone(), None, app.state())
            .await
            .unwrap();
        assert_eq!(
            labels(&core).await,
            [(key.clone(), "Deploy runbook".into())]
        );
        note_restore(note.id.clone(), app.state()).await.unwrap();
        note_delete(note.id.clone(), Some(true), app.state())
            .await
            .unwrap();
        assert!(labels(&core).await.is_empty());
    }
}
