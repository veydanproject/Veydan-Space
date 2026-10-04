// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What notes know of the entities of other modules — passwords, TOTP
//! entries, workspaces, profiles, proxies, SSH connections: their names, the
//! public fields a picker or a template shows, the workspace of a profile.
//! All of it comes from the entity directory, by the kind a binding names;
//! notes read no table of another module. An entity nobody in the product
//! provides is not there.

use super::binding::{BindingKind, BindingSummary};
use tauri::Runtime;
use veydan_core::{Directory, Label};

/// How many entities a picker's search shows.
const SEARCH_LIMIT: usize = 30;

/// The name of the entity `id`; empty when it is not there.
pub(crate) async fn name<R: Runtime>(
    directory: &Directory<R>,
    kind: BindingKind,
    id: &str,
) -> String {
    directory
        .label(kind.name(), id)
        .await
        .map(|label| label.name)
        .unwrap_or_default()
}

/// A public field of the entity `id`; empty when it has none or is not there.
pub(crate) async fn field<R: Runtime>(
    directory: &Directory<R>,
    kind: BindingKind,
    id: &str,
    field: &str,
) -> String {
    directory
        .action(kind.name(), id, "field", Some(field))
        .await
        .unwrap_or_default()
}

/// What a picker shows under the name of an entity: its owner's action
/// `subtitle` (the user name of a password, `user@host` of a connection);
/// empty where the kind has no owner here.
async fn subtitle<R: Runtime>(directory: &Directory<R>, kind: BindingKind, id: &str) -> String {
    directory
        .action(kind.name(), id, "subtitle", None)
        .await
        .unwrap_or_default()
}

fn summary(kind: BindingKind, label: Label, subtitle: String) -> BindingSummary {
    BindingSummary {
        binding: kind.with(&label.id),
        kind: kind.name().to_string(),
        name: label.name,
        subtitle,
    }
}

/// Names and subtitles of the entities behind `bindings`; value bindings and
/// entities that are not there are skipped.
pub async fn summaries<R: Runtime>(
    directory: &Directory<R>,
    bindings: &[String],
) -> Vec<BindingSummary> {
    let mut out = Vec::new();
    for binding in bindings {
        let Some((kind, id)) = BindingKind::parse(binding).filter(|(k, _)| k.is_entity()) else {
            continue;
        };
        if let Some(label) = directory.label(kind.name(), id).await {
            let subtitle = subtitle(directory, kind, &label.id).await;
            out.push(summary(kind, label, subtitle));
        }
    }
    out
}

/// Entities of `kind` whose name or subtitle holds `query` — as SQLite's
/// `LIKE` matches, ASCII letters in either case — by name, at most
/// [`SEARCH_LIMIT`]. The owner searches in one query (`Provider::search`);
/// the subtitles of what it found follow, one action each.
pub async fn search<R: Runtime>(
    directory: &Directory<R>,
    kind: &str,
    query: &str,
) -> Vec<BindingSummary> {
    let Some(kind) = BindingKind::from_name(kind).filter(|k| k.is_entity()) else {
        return Vec::new();
    };
    // Lowered here as a whole, letters outside ASCII too, as the query of
    // stage 5 was; `LIKE` then folds ASCII letters only.
    let query = query.trim().to_lowercase();
    let found = directory.search(kind.name(), &query, SEARCH_LIMIT).await;
    let mut out = Vec::with_capacity(found.len());
    for label in found {
        let subtitle = subtitle(directory, kind, &label.id).await;
        out.push(summary(kind, label, subtitle));
    }
    out
}

/// Every workspace with its name and color, in the order of its owner.
pub async fn workspaces<R: Runtime>(directory: &Directory<R>) -> Vec<(String, String, String)> {
    directory
        .list(BindingKind::Workspace.name())
        .await
        .into_iter()
        .map(|label| (label.id, label.name, label.color.unwrap_or_default()))
        .collect()
}

/// Every profile with its name and workspace, in the order of its owner.
pub async fn profiles<R: Runtime>(
    directory: &Directory<R>,
) -> Vec<(String, String, Option<String>)> {
    directory
        .list(BindingKind::Profile.name())
        .await
        .into_iter()
        .map(|label| {
            let workspace = workspace_of(label.parent);
            (label.id, label.name, workspace)
        })
        .collect()
}

/// The workspace the profile `id` sits in: where a captured page goes.
#[cfg(desktop)]
pub async fn workspace_of_profile<R: Runtime>(
    directory: &Directory<R>,
    id: &str,
) -> Option<String> {
    workspace_of(
        directory
            .label(BindingKind::Profile.name(), id)
            .await?
            .parent,
    )
}

fn workspace_of(parent: Option<(String, String)>) -> Option<String> {
    parent
        .filter(|(kind, _)| kind == BindingKind::Workspace.name())
        .map(|(_, id)| id)
}

#[cfg(test)]
mod tests {
    /// What an owner's search matches: `lower(name) LIKE` the pattern of the
    /// query the picker lowered, as the picker of stage 5 matched — `%` is
    /// any run of characters, `_` one character, ASCII letters match in
    /// either case and other letters as written.
    #[tokio::test]
    async fn a_query_reads_as_like_does() {
        let db = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        for (text, query, found) in [
            ("GitHub", "git", true),
            ("GitHub", "", true),
            ("", "", true),
            ("root@10.0.0.1", "t@1", true),
            ("a_b", "a_b", true),
            ("axb", "a_b", true),
            ("100% sure", "0%", true),
            ("GitHub", "gitlab", false),
            ("ab", "a_b", false),
            // Only ASCII letters fold; Cyrillic matches as written.
            ("Разработка", "работ", true),
            ("Разработка", "раз", false),
            ("разработка", "раз", true),
        ] {
            let matched: bool = sqlx::query_scalar("SELECT lower(?) LIKE ?")
                .bind(text)
                .bind(veydan_core::like_pattern(&query.to_lowercase()))
                .fetch_one(&db)
                .await
                .unwrap();
            assert_eq!(matched, found, "{text:?} {query:?}");
        }
    }
}
