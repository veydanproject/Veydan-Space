// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Registry of note binding kinds. A binding is stored as `kind:value`
//! (JSON array in `notes.bindings` and frontmatter). Entity kinds point at a
//! Veydan object by id; value kinds carry the value itself (url, domain).
//! The name of an entity kind is its kind in the entity directory, which
//! names the entities for the UI (`entities`).

use super::entities;
use serde::Serialize;
use veydan_core::{CmdResult, Core};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum BindingKind {
    Workspace,
    Profile,
    Proxy,
    Ssh,
    Totp,
    Password,
    Url,
    Domain,
}

impl BindingKind {
    /// Most specific first; used to rank related notes.
    pub const ALL: [BindingKind; 8] = [
        BindingKind::Url,
        BindingKind::Domain,
        BindingKind::Ssh,
        BindingKind::Proxy,
        BindingKind::Totp,
        BindingKind::Password,
        BindingKind::Profile,
        BindingKind::Workspace,
    ];

    pub fn prefix(self) -> &'static str {
        match self {
            BindingKind::Workspace => "workspace:",
            BindingKind::Profile => "profile:",
            BindingKind::Proxy => "proxy:",
            BindingKind::Ssh => "ssh:",
            BindingKind::Totp => "totp:",
            BindingKind::Password => "password:",
            BindingKind::Url => "url:",
            BindingKind::Domain => "domain:",
        }
    }

    /// True for a kind that points at an entity of another module; false for value kinds.
    pub fn is_entity(self) -> bool {
        !matches!(self, BindingKind::Url | BindingKind::Domain)
    }

    pub fn name(self) -> &'static str {
        self.prefix().trim_end_matches(':')
    }

    pub fn from_name(name: &str) -> Option<BindingKind> {
        BindingKind::ALL.iter().copied().find(|k| k.name() == name)
    }

    /// Workspace/profile scope a note "belongs to"; notes without one are global.
    pub fn is_scope(self) -> bool {
        matches!(self, BindingKind::Workspace | BindingKind::Profile)
    }

    /// Split `kind:value` into its kind and value.
    pub fn parse(binding: &str) -> Option<(BindingKind, &str)> {
        BindingKind::ALL
            .iter()
            .find_map(|k| binding.strip_prefix(k.prefix()).map(|v| (*k, v)))
    }

    pub fn with(self, value: &str) -> String {
        format!("{}{}", self.prefix(), value)
    }
}

/// True when the string is an entity binding, e.g. `ssh:{id}`.
pub(crate) fn is_entity_binding(s: &str) -> bool {
    BindingKind::parse(s).is_some_and(|(k, v)| k.is_entity() && !v.is_empty())
}

/// True when the binding scopes the note to a workspace or profile.
pub(crate) fn is_scope_binding(s: &str) -> bool {
    BindingKind::parse(s).is_some_and(|(k, _)| k.is_scope())
}

/// Public description of an entity behind a binding.
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct BindingSummary {
    pub binding: String,
    pub kind: String,
    pub name: String,
    pub subtitle: String,
}

/// Names and subtitles of the entities behind `bindings`; unknown or deleted ones are skipped.
#[tauri::command]
pub async fn note_binding_summaries(
    bindings: Vec<String>,
    core: tauri::State<'_, Core>,
) -> CmdResult<Vec<BindingSummary>> {
    Ok(entities::summaries(&core.directory, &bindings).await)
}

/// Entities of `kind` whose name contains `query`, for binding pickers.
#[tauri::command]
pub async fn note_entity_search(
    kind: String,
    query: String,
    core: tauri::State<'_, Core>,
) -> CmdResult<Vec<BindingSummary>> {
    Ok(entities::search(&core.directory, &kind, &query).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_prefixes() {
        assert_eq!(
            BindingKind::parse("ssh:abc"),
            Some((BindingKind::Ssh, "abc"))
        );
        assert_eq!(
            BindingKind::parse("domain:x.com"),
            Some((BindingKind::Domain, "x.com"))
        );
        assert_eq!(BindingKind::parse("other:1"), None);
    }

    #[test]
    fn entity_vs_scope() {
        assert!(is_entity_binding("proxy:1"));
        assert!(!is_entity_binding("url:https://a"));
        assert!(!is_entity_binding("ssh:"));
        assert!(is_scope_binding("workspace:1"));
        assert!(!is_scope_binding("proxy:1"));
    }
}
