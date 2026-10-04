// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Extra demo volume (~5×) generated on top of the handcrafted pack: bulk
//! notes in the workspaces and on the bulk profiles of the demo data of
//! browser, known by their ids.

use crate::{insert_note, NewNote, NotesState};
use veydan_core::{AppError, CmdResult, Core};

/// The workspaces of the demo data of browser.
const WS: &[&str] = &[
    "default",
    "demo-ws-smm",
    "demo-ws-dev",
    "demo-ws-devops",
    "demo-ws-biz",
    "demo-ws-freelance",
    "demo-ws-research",
];

/// The bulk profiles of browser: `demo-pr-bulk-01` to `demo-pr-bulk-56`.
const BULK_PROFILES: u32 = 56;

fn bulk_profile_ids() -> Vec<String> {
    (1..=BULK_PROFILES)
        .map(|i| format!("demo-pr-bulk-{i:02}"))
        .collect()
}

pub(super) async fn seed_notes(
    core: &Core,
    notes: &NotesState,
    locale: &str,
    now: &str,
) -> CmdResult<()> {
    let ru = locale.eq_ignore_ascii_case("ru");
    seed_bulk_notes(core, notes, ru, &bulk_profile_ids(), now).await
}

async fn seed_bulk_notes(
    core: &Core,
    notes: &NotesState,
    ru: bool,
    profile_ids: &[String],
    now: &str,
) -> CmdResult<()> {
    let folders = [
        "demo-folder-smm-content",
        "demo-folder-smm-clients",
        "demo-folder-dev",
        "demo-folder-dev-arch",
        "demo-folder-dev-snip",
        "demo-folder-devops-run",
        "demo-folder-devops-check",
        "demo-folder-biz-meet",
        "demo-folder-biz-fin",
        "demo-folder-personal",
        "demo-folder-inbox",
    ];
    let tag_sets: &[&[&str]] = &[
        &["work/smm", "status/todo"],
        &["work/smm", "status/wip", "client/brand-a"],
        &["work/dev", "status/todo"],
        &["work/dev", "status/wip"],
        &["work/devops", "infra/prod"],
        &["work/devops", "infra/staging", "status/todo"],
        &["work/biz", "status/wip"],
        &["work/biz", "status/todo"],
        &["personal"],
        &["personal", "status/todo"],
        &["status/wip"],
        &["status/done"],
    ];

    let titles_en: &[&str] = &[
        "Standup notes",
        "Retrospective",
        "Bug triage",
        "Copy draft",
        "Campaign idea",
        "Server checklist",
        "Client call",
        "Invoice reminder",
        "Research scrap",
        "Snippet dump",
        "Weekly goals",
        "Risk log",
        "Design feedback",
        "SEO notes",
        "A/B test plan",
        "Onboarding checklist",
        "Vendor comparison",
        "Security review",
        "Cost estimate",
        "Travel plan",
    ];
    let titles_ru: &[&str] = &[
        "Заметки стендапа",
        "Ретроспектива",
        "Триаж багов",
        "Черновик текста",
        "Идея кампании",
        "Чеклист сервера",
        "Звонок клиенту",
        "Напоминание по счёту",
        "Черновик ресёрча",
        "Свалка сниппетов",
        "Цели недели",
        "Журнал рисков",
        "Фидробек по дизайну",
        "Заметки SEO",
        "План A/B теста",
        "Чеклист онбординга",
        "Сравнение вендоров",
        "Security review",
        "Оценка стоимости",
        "План поездки",
    ];
    let titles = if ru { titles_ru } else { titles_en };
    // The Context block names a workspace of browser: only where one exists.
    let space = super::names_workspaces(core);

    // 36 handcrafted + 144 bulk ≈ 180
    for i in 1..=144 {
        let title_base = titles[(i as usize - 1) % titles.len()];
        let title = format!("{title_base} #{i:03}");
        let folder = folders[(i as usize - 1) % folders.len()];
        let tags = tag_sets[(i as usize - 1) % tag_sets.len()];
        let ws = WS[(i as usize - 1) % WS.len()];
        let mut bindings = vec![format!("workspace:{ws}")];
        if i % 4 == 0 && !profile_ids.is_empty() {
            bindings.push(format!(
                "profile:{}",
                profile_ids[(i as usize - 1) % profile_ids.len()]
            ));
        }
        if i % 11 == 0 {
            bindings.push("domain:github.com".into());
        }
        if i % 13 == 0 {
            bindings.push("domain:instagram.com".into());
        }

        let context = match (space, ru) {
            (true, true) => format!("## Контекст\nWorkspace `{ws}`.\n\n"),
            (true, false) => format!("## Context\nWorkspace `{ws}`.\n\n"),
            (false, _) => String::new(),
        };
        let content = if ru {
            format!(
                "# {title}\n\nАвтосгенерированная демо-заметка #{i}.\n\n{context}## Задачи\n- [ ] Пункт A\n- [x] Пункт B\n- [ ] Пункт C\n\n## Заметки\nКраткий черновик для видео-демо. Можно править или удалить.\n\n```\necho demo-{i}\n```\n"
            )
        } else {
            format!(
                "# {title}\n\nAuto-generated demo note #{i}.\n\n{context}## Tasks\n- [ ] Item A\n- [x] Item B\n- [ ] Item C\n\n## Notes\nShort draft for the video demo. Safe to edit or delete.\n\n```\necho demo-{i}\n```\n"
            )
        };

        let note = insert_note(
            NewNote {
                id: format!("demo-note-bulk-{i:03}"),
                title,
                format: "md".into(),
                bindings: super::bindable(core, bindings),
                tags: tags.iter().map(|s| s.to_string()).collect(),
                content,
                created_at: now.to_string(),
                updated_at: now.to_string(),
            },
            core,
            notes,
        )
        .await?;

        sqlx::query("INSERT OR IGNORE INTO note_folder_links (note_id, folder_id) VALUES (?, ?)")
            .bind(&note.id)
            .bind(folder)
            .execute(&core.db)
            .await
            .map_err(AppError::db)?;

        // Sprinkle pinned / archived
        if i % 37 == 0 {
            sqlx::query("UPDATE notes SET pinned = 1 WHERE id = ?")
                .bind(&note.id)
                .execute(&core.db)
                .await
                .map_err(AppError::db)?;
        }
        if i % 41 == 0 {
            sqlx::query("UPDATE notes SET archived = 1 WHERE id = ?")
                .bind(&note.id)
                .execute(&core.db)
                .await
                .map_err(AppError::db)?;
        }
        if i % 73 == 0 {
            let fts: Option<(Option<i64>,)> =
                sqlx::query_as("SELECT fts_rowid FROM notes WHERE id = ?")
                    .bind(&note.id)
                    .fetch_optional(&core.db)
                    .await
                    .map_err(AppError::db)?;
            sqlx::query("UPDATE notes SET deleted = 1, updated_at = ? WHERE id = ?")
                .bind(now)
                .bind(&note.id)
                .execute(&core.db)
                .await
                .map_err(AppError::db)?;
            if let Some((Some(rowid),)) = fts {
                let _ = sqlx::query("DELETE FROM notes_fts WHERE rowid = ?")
                    .bind(rowid)
                    .execute(&core.db)
                    .await;
            }
        }
    }
    Ok(())
}
