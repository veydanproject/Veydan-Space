// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The rules of the web clipper: a captured page of a domain goes to a
//! folder, with tags and a template (`CaptureRule` of notes, which applies
//! them). The key is capture's and is shared between desktops.

use sqlx::{Pool, Sqlite};
use veydan_core::{settings, CmdResult, Core};
use veydan_notes::capture::CaptureRule;

pub(crate) const KEY: &str = "notes_capture_rules";

pub(crate) async fn load(db: &Pool<Sqlite>) -> Vec<CaptureRule> {
    settings::get_json(db, KEY).await.unwrap_or_default()
}

#[tauri::command]
pub async fn notes_capture_rules_get(core: tauri::State<'_, Core>) -> CmdResult<Vec<CaptureRule>> {
    Ok(load(&core.db).await)
}

#[tauri::command]
pub async fn notes_capture_rules_set(
    rules: Vec<CaptureRule>,
    core: tauri::State<'_, Core>,
) -> CmdResult<Vec<CaptureRule>> {
    let rules: Vec<CaptureRule> = rules
        .into_iter()
        .filter(|r| !r.domain.trim().is_empty())
        .map(|r| CaptureRule {
            domain: r.domain.trim().to_lowercase(),
            folder_id: r.folder_id.filter(|f| !f.is_empty()),
            tags: r
                .tags
                .into_iter()
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect(),
            template_id: r.template_id.filter(|t| !t.is_empty()),
        })
        .collect();
    settings::set_json(&core.db, KEY, &rules).await?;
    Ok(rules)
}
