// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Navigation tree for the notes screens: counts per section plus the
//! workspace / profile / folder / tag / smart view / site catalogs, in one call.
//! Workspaces and profiles are those the entity directory names: by their
//! owner, or by the labels where the product has no browser.

use super::binding::BindingKind;
use super::entities;
use super::models::NoteRow;
use super::tags::{fetch_all_note_folder_ids_map, fetch_all_note_tags_map};
use serde::Serialize;
use sqlx::{Pool, Sqlite};
use std::collections::HashMap;
use tauri::Runtime;
use veydan_core::{AppError, CmdResult};
use veydan_core::{Core, Directory};

#[derive(Debug, Serialize, Clone)]
pub struct NavChild {
    pub id: String,
    pub name: String,
    pub color: String,
    pub count: i64,
    pub parent_id: Option<String>,
    /// Profiles with notes, nested under their workspace.
    pub profiles: Vec<NavChild>,
}

#[derive(Debug, Serialize)]
pub struct NavTag {
    pub id: String,
    pub name: String,
    pub color: String,
    pub count: i64,
}

#[derive(Debug, Serialize)]
pub struct NavCounts {
    pub all: i64,
    pub global: i64,
    pub pinned: i64,
    pub archived: i64,
    pub trash: i64,
}

#[derive(Debug, Serialize)]
pub struct NoteNav {
    pub counts: NavCounts,
    /// Workspaces that have notes (directly or through a profile).
    pub workspaces: Vec<NavChild>,
    pub folders: Vec<NavChild>,
    pub tags: Vec<NavTag>,
    pub smart: Vec<NavChild>,
    pub sites: Vec<NavChild>,
    /// Full catalogs for pickers, no counts.
    pub all_workspaces: Vec<NavChild>,
    pub all_profiles: Vec<NavChild>,
}

fn child(
    id: String,
    name: String,
    color: String,
    count: i64,
    parent_id: Option<String>,
) -> NavChild {
    NavChild {
        id,
        name,
        color,
        count,
        parent_id,
        profiles: Vec::new(),
    }
}

#[tauri::command]
pub async fn note_nav(core: tauri::State<'_, Core>) -> CmdResult<NoteNav> {
    nav(&core.db, &core.directory).await
}

/// The tree over the notes of `db`, with the workspaces and profiles
/// `directory` names: their owner's, or the labels where the product has no
/// browser.
pub async fn nav<R: Runtime>(db: &Pool<Sqlite>, directory: &Directory<R>) -> CmdResult<NoteNav> {
    let rows = sqlx::query_as::<_, NoteRow>("SELECT * FROM notes")
        .fetch_all(db)
        .await
        .map_err(AppError::db)?;
    let tags_map = fetch_all_note_tags_map(db).await?;
    let folders_of = fetch_all_note_folder_ids_map(db).await?;

    let mut counts = NavCounts {
        all: 0,
        global: 0,
        pinned: 0,
        archived: 0,
        trash: 0,
    };
    let mut ws_direct: HashMap<String, i64> = HashMap::new();
    let mut pr_count: HashMap<String, i64> = HashMap::new();
    let mut folder_count: HashMap<String, i64> = HashMap::new();
    let mut site_count: HashMap<String, i64> = HashMap::new();
    let mut tag_count: HashMap<String, i64> = HashMap::new();

    for r in &rows {
        if r.deleted != 0 {
            counts.trash += 1;
            continue;
        }
        if r.archived != 0 {
            counts.archived += 1;
            continue;
        }
        counts.all += 1;
        if r.pinned != 0 {
            counts.pinned += 1;
        }
        let bindings: Vec<String> = serde_json::from_str(&r.bindings).unwrap_or_default();
        let parsed: Vec<(BindingKind, &str)> = bindings
            .iter()
            .filter_map(|b| BindingKind::parse(b))
            .collect();
        let has_ws = parsed.iter().any(|(k, _)| *k == BindingKind::Workspace);
        let has_pr = parsed.iter().any(|(k, _)| *k == BindingKind::Profile);
        if !has_ws && !has_pr {
            counts.global += 1;
        }
        for (kind, value) in &parsed {
            match kind {
                // A profile-bound note counts under its profile, not the workspace itself.
                BindingKind::Workspace if !has_pr => {
                    *ws_direct.entry(value.to_string()).or_default() += 1;
                }
                BindingKind::Profile => {
                    *pr_count.entry(value.to_string()).or_default() += 1;
                }
                BindingKind::Domain if !value.is_empty() => {
                    *site_count.entry(value.to_string()).or_default() += 1;
                }
                _ => {}
            }
        }
        if let Some(ids) = folders_of.get(&r.id) {
            for fid in ids {
                *folder_count.entry(fid.clone()).or_default() += 1;
            }
        }
        if let Some(tags) = tags_map.get(&r.id) {
            for t in tags {
                *tag_count.entry(t.id.clone()).or_default() += 1;
            }
        }
    }

    let ws_rows = entities::workspaces(directory).await;
    let pr_rows = entities::profiles(directory).await;

    let all_workspaces: Vec<NavChild> = ws_rows
        .iter()
        .map(|(id, name, color)| child(id.clone(), name.clone(), color.clone(), 0, None))
        .collect();
    let all_profiles: Vec<NavChild> = pr_rows
        .iter()
        .map(|(id, name, ws)| child(id.clone(), name.clone(), String::new(), 0, ws.clone()))
        .collect();

    let mut profiles_of: HashMap<String, Vec<NavChild>> = HashMap::new();
    for (id, name, ws_id) in pr_rows {
        let count = *pr_count.get(&id).unwrap_or(&0);
        if let Some(ws_id) = ws_id {
            profiles_of
                .entry(ws_id)
                .or_default()
                .push(child(id, name, String::new(), count, None));
        }
    }
    let workspaces = ws_rows
        .into_iter()
        .filter_map(|(id, name, color)| {
            let count = *ws_direct.get(&id).unwrap_or(&0);
            let profiles = profiles_of.remove(&id).unwrap_or_default();
            if count == 0 && profiles.is_empty() {
                return None;
            }
            Some(NavChild {
                id,
                name,
                color,
                count,
                parent_id: None,
                profiles,
            })
        })
        .collect();

    let folders = sqlx::query_as::<_, (String, String, Option<String>, String)>(
        "SELECT id, name, parent_id, color FROM note_folders ORDER BY name",
    )
    .fetch_all(db)
    .await
    .map_err(AppError::db)?
    .into_iter()
    .map(|(id, name, parent_id, color)| {
        let count = *folder_count.get(&id).unwrap_or(&0);
        child(id, name, color, count, parent_id)
    })
    .collect();

    let tags = sqlx::query_as::<_, (String, String, String)>(
        "SELECT id, name, color FROM note_tags ORDER BY name",
    )
    .fetch_all(db)
    .await
    .map_err(AppError::db)?
    .into_iter()
    .map(|(id, name, color)| {
        let count = *tag_count.get(&id).unwrap_or(&0);
        NavTag {
            id,
            name,
            color,
            count,
        }
    })
    .collect();

    let smart = sqlx::query_as::<_, (String, String, String)>(
        "SELECT id, name, color FROM note_smart_views ORDER BY sort_order ASC, created_at ASC",
    )
    .fetch_all(db)
    .await
    .map_err(AppError::db)?
    .into_iter()
    .map(|(id, name, color)| child(id, name, color, 0, None))
    .collect();

    let mut sites: Vec<NavChild> = site_count
        .into_iter()
        .map(|(id, count)| child(id.clone(), id, String::new(), count, None))
        .collect();
    sites.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));

    Ok(NoteNav {
        counts,
        workspaces,
        folders,
        tags,
        smart,
        sites,
        all_workspaces,
        all_profiles,
    })
}
