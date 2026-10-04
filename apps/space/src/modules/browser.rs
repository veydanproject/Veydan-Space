// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Browser profiles, proxies, workspaces and the Camoufox download.

use tauri::Manager;
use veydan_core::Core;
use veydan_shell::{Module, SetupResult};

#[cfg(desktop)]
use self::desktop::{stop, tray_click, tray_items, tray_tooltip};

pub(crate) fn module() -> Module {
    Module {
        schema: Some(crate::db::BROWSER),
        #[cfg(desktop)]
        directory: Some(super::directory::browser),
        #[cfg(desktop)]
        tray: Some(veydan_shell::TrayPart {
            items: tray_items,
            on_click: tray_click,
            tooltip: Some(tray_tooltip),
        }),
        #[cfg(desktop)]
        stop: Some(stop),
        // A phone has no browsers: nothing to switch.
        #[cfg(mobile)]
        switch: veydan_shell::Switch::Always,
        sync: Some(sync::register),
        demo: Some(crate::commands::demo::browser()),
        #[cfg(desktop)]
        backup: Some(backup()),
        ..commands()
    }
}

/// The part of browser in a backup, on any runtime; `module()` holds it on
/// the app's: the folder `profiles/` of the data directory, without the
/// caches of the browsers (the driver leaves those out). A phone keeps no
/// profiles and has no backups.
#[cfg(desktop)]
pub(crate) fn backup<R: tauri::Runtime>() -> veydan_shell::BackupPart<R> {
    veydan_shell::BackupPart {
        paths: |_app| {
            vec![veydan_shell::BackupPath {
                name: "profiles",
                path: std::path::PathBuf::from("profiles"),
                external: false,
            }]
        },
    }
}

#[cfg(desktop)]
fn commands() -> Module {
    veydan_shell::module! {
        id: "browser",
        setup: setup,
        commands: [
            crate::fingerprint::fingerprint_presets,
            crate::commands::profiles::sync::sync_profile_files_take_remote,
            crate::commands::profiles::sync::sync_profile_files_push_mine,
            crate::commands::profiles::crud::profiles_list,
            crate::commands::profiles::crud::profile_get,
            crate::commands::profiles::crud::profile_create,
            crate::commands::profiles::crud::profile_update,
            crate::commands::profiles::crud::profile_delete,
            crate::commands::profiles::crud::profile_clone,
            crate::commands::profiles::launch::profile_launch,
            crate::commands::profiles::launch::profile_stop,
            crate::commands::profiles::launch::profile_is_running,
            crate::commands::profiles::launch::profiles_running_ids,
            crate::commands::proxies::proxies_list,
            crate::commands::proxies::proxy_get,
            crate::commands::proxies::proxy_create,
            crate::commands::proxies::proxies_bulk_create,
            crate::commands::proxies::proxy_update,
            crate::commands::proxies::proxy_delete,
            crate::commands::proxies::proxy_usage,
            crate::commands::proxies::proxy_check,
            crate::commands::proxies::proxy_export_url,
            crate::commands::proxies::proxy_trust_fingerprint,
            crate::commands::workspaces::workspace_list,
            crate::commands::workspaces::workspace_get,
            crate::commands::workspaces::workspace_create,
            crate::commands::workspaces::workspace_update,
            crate::commands::workspaces::workspace_delete,
            crate::commands::workspaces::workspace_stats,
            crate::commands::workspaces::profiles_list_by_workspace,
            crate::commands::workspaces::workspace_column_list,
            crate::commands::workspaces::workspace_column_create,
            crate::commands::workspaces::workspace_column_update,
            crate::commands::workspaces::workspace_column_delete,
            crate::commands::workspaces::profile_set_tags,
            crate::commands::workspaces::profile_move_to_kanban_column,
            crate::commands::profiles::crud::profile_raw_data,
            crate::commands::profiles::import_export::profile_export_json,
            crate::commands::profiles::import_export::profile_export_zip,
            crate::commands::profiles::import_export::profile_import_json,
            crate::commands::profiles::import_export::profile_import_zip,
            crate::commands::profiles::import_export::profile_import_zip_data,
            crate::commands::profiles::cookies::profile_import_cookies,
            crate::commands::profiles::cookies::profile_export_cookies,
            crate::commands::profiles::cookies::profile_export_cookies_to_file,
            crate::commands::profiles::import_export::profile_export_json_to_file,
            crate::commands::camoufox::camoufox_status,
            crate::commands::camoufox::camoufox_download,
            crate::commands::camoufox::camoufox_download_state,
            crate::commands::camoufox::camoufox_download_cancel,
            crate::commands::camoufox::camoufox_latest_version,
        ],
    }
}

/// A phone keeps the tables and has no commands for them. Rows an earlier
/// build mirrored into them stay; nothing reads them.
#[cfg(mobile)]
fn commands() -> Module {
    veydan_shell::module! { id: "browser", setup: setup, commands: [] }
}

/// What every start does to the rows of the catalog, then what the desktop
/// keeps of the browsers.
fn setup(app: &mut tauri::App) -> SetupResult {
    let db = app.state::<Core>().db.clone();
    tauri::async_runtime::block_on(crate::db::prepare(&db, &app.state::<Core>().directory))
        .map_err(|e| format!("Failed to initialize database: {e:#}"))?;
    #[cfg(desktop)]
    desktop::setup(app)?;
    Ok(())
}

#[cfg(desktop)]
mod desktop {
    use crate::browser::BrowserState;
    use std::collections::HashSet;
    use tauri::{AppHandle, Emitter, Listener, Manager};
    use veydan_core::{AppError, BoxFuture, Core};
    use veydan_shell::{tray_label, SetupResult, Shell, TrayGroup, TrayItem, TrayLabels};

    pub(super) fn setup(app: &mut tauri::App) -> SetupResult {
        std::fs::create_dir_all(app.state::<Core>().app_data_dir.join("profiles"))?;
        app.manage(BrowserState::default());
        // Keep the tray's running-profiles submenu + tooltip in sync.
        let handle = app.handle().clone();
        app.listen("profiles://running-changed", move |_| {
            handle.state::<Shell>().tray_refresh();
        });
        Ok(())
    }

    /// The app quits: `stop_all` signals every browser and then awaits the
    /// monitor tasks (bounded) so the kills actually complete before the
    /// process exits — otherwise children would be orphaned.
    ///
    /// The user switches the module off: the browsers that run keep running
    /// (platform-spec 20.7) — the app still watches them and stops them when
    /// it quits, and switched on again the module shows them. Off, the module
    /// has no entries in the tray; that is the shell's to leave out.
    pub(super) fn stop(app: AppHandle) -> BoxFuture<'static, Result<(), AppError>> {
        Box::pin(async move {
            if app.state::<Shell>().exiting() {
                crate::browser::launch::stop_all(&app.state::<BrowserState>().running).await;
            }
            Ok(())
        })
    }

    /// (id, name) lists for the dynamic submenus.
    struct MenuData {
        /// Currently running profiles.
        running: Vec<(String, String)>,
        /// Recently-updated, non-running profiles for quick launch.
        recent: Vec<(String, String)>,
    }

    /// Pull running profiles (with names) and a handful of recent non-running
    /// profiles from the browser state + DB.
    async fn load_menu_data(app: &AppHandle) -> MenuData {
        let (db, browser) = {
            let db = app.state::<Core>().db.clone();
            (db, app.state::<BrowserState>().running.clone())
        };
        let running_ids = browser.running_ids().await;
        let running_set: HashSet<String> = running_ids.iter().cloned().collect();

        let mut running = Vec::with_capacity(running_ids.len());
        for id in &running_ids {
            let name: Option<String> = sqlx::query_scalar("SELECT name FROM profiles WHERE id = ?")
                .bind(id)
                .fetch_optional(&db)
                .await
                .ok()
                .flatten();
            running.push((id.clone(), name.unwrap_or_else(|| id.clone())));
        }

        let recent = sqlx::query_as::<_, (String, String)>(
            "SELECT id, name FROM profiles ORDER BY updated_at DESC LIMIT 20",
        )
        .fetch_all(&db)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|(id, _)| !running_set.contains(id))
        .take(8)
        .collect();

        MenuData { running, recent }
    }

    /// The two submenus, then the pages of the module. Starting and stopping
    /// a profile is dispatched back to the webview via events so the heavy
    /// launch/teardown logic stays in the frontend `api` layer.
    pub(super) fn tray_items(
        app: AppHandle,
        labels: TrayLabels,
    ) -> BoxFuture<'static, Vec<TrayItem>> {
        Box::pin(async move {
            let data = load_menu_data(&app).await;
            let entry = |id: String, label: String| TrayItem::entry(TrayGroup::Lists, id, label);

            let running = if data.running.is_empty() {
                let label = tray_label(&labels, "no_running", "No running profiles");
                vec![entry("noop:running".into(), label).disabled()]
            } else {
                let mut items: Vec<TrayItem> = data
                    .running
                    .iter()
                    .map(|(id, name)| entry(format!("stop:{id}"), format!("● {name}")))
                    .collect();
                items.push(TrayItem::separator(TrayGroup::Lists));
                let label = tray_label(&labels, "stop_all", "Stop all");
                items.push(entry("stop_all".into(), label));
                items
            };

            let launch = if data.recent.is_empty() {
                let label = tray_label(&labels, "no_profiles", "No profiles");
                vec![entry("noop:launch".into(), label).disabled()]
            } else {
                data.recent
                    .iter()
                    .map(|(id, name)| entry(format!("launch:{id}"), name.clone()))
                    .collect()
            };

            // "Running profiles ({n})" — `{n}` is replaced with the live count.
            let running_label = tray_label(&labels, "running", "Running profiles ({n})")
                .replace("{n}", &data.running.len().to_string());
            let launch_label = tray_label(&labels, "launch_profile", "Launch profile");
            let section =
                |route: &str, label: String| TrayItem::entry(TrayGroup::Sections, route, label);
            vec![
                entry("running".into(), running_label).submenu(running),
                entry("launch".into(), launch_label).submenu(launch),
                section(
                    "nav:/",
                    tray_label(&labels, "section_workspaces", "Workspaces"),
                ),
                section(
                    "nav:/proxies",
                    tray_label(&labels, "section_proxies", "Proxies"),
                ),
            ]
        })
    }

    pub(super) fn tray_click(app: &AppHandle, id: &str) {
        if id == "stop_all" {
            let _ = app.emit("tray://stop-all", ());
        } else if let Some(profile) = id.strip_prefix("stop:") {
            let _ = app.emit("tray://stop-profile", profile.to_string());
        } else if let Some(profile) = id.strip_prefix("launch:") {
            let _ = app.emit("tray://launch-profile", profile.to_string());
        } else {
            super::super::tray_navigate(app, id);
        }
    }

    /// "Veydan Space — {n} running" — `{n}` is replaced with the live count.
    pub(super) fn tray_tooltip(app: AppHandle, labels: TrayLabels) -> BoxFuture<'static, String> {
        Box::pin(async move {
            let browser = app.state::<BrowserState>().running.clone();
            let running = browser.running_ids().await.len();
            let default = format!("{} — {{n}} running", app.state::<Shell>().product().name);
            tray_label(&labels, "tooltip", &default).replace("{n}", &running.to_string())
        })
    }
}

/// Workspaces, their kanban columns, proxies and profiles as sync entities,
/// and the files of profiles, between desktops. A phone syncs none of them:
/// its notes know workspaces and profiles by the labels a desktop publishes.
mod sync {
    use veydan_sync_host::Registry;
    #[cfg(desktop)]
    use {
        super::super::directory,
        serde_json::{Map, Value},
        sqlx::{Pool, Sqlite},
        std::collections::HashMap,
        veydan_core::{AppError, BoxFuture, CmdResult},
        veydan_sync_host::{Delete, Deletion, Hooks, Host, Ref, TableSpec, Upsert},
    };

    #[cfg(desktop)]
    const WORKSPACE: TableSpec = TableSpec {
        delete: Delete::Plain(&[
            "DELETE FROM workspace_columns WHERE workspace_id = ?",
            "DELETE FROM ssh_connection_workspaces WHERE workspace_id = ?",
            "UPDATE profiles SET workspace_id = NULL WHERE workspace_id = ?",
        ]),
        ..TableSpec::plain(
            "workspace",
            "workspaces",
            &[
                "name",
                "description",
                "color",
                "icon",
                "notes",
                "is_default",
                "created_at",
                "updated_at",
            ],
        )
    };

    #[cfg(desktop)]
    const WORKSPACE_COLUMN: TableSpec = TableSpec {
        unique: Some(&["workspace_id", "name"]),
        requires: &[Ref {
            key: "workspace_id",
            table: "workspaces",
        }],
        // The tags of the profiles follow a column that merged with a local one.
        also_changes: &["profile"],
        ..TableSpec::plain(
            "workspace_column",
            "workspace_columns",
            &[
                "workspace_id",
                "name",
                "tag_name",
                "color",
                "position",
                "created_at",
            ],
        )
    };

    #[cfg(desktop)]
    const PROXY: TableSpec = TableSpec {
        // Must detach every referencing table, exactly like `proxy_delete` does
        // locally. Missing `ssh_connections` here left connections pointing at a
        // deleted proxy on every other device.
        delete: Delete::Plain(&[
            "UPDATE profiles SET proxy_id = NULL WHERE proxy_id = ?",
            "UPDATE ssh_connections SET proxy_id = NULL WHERE proxy_id = ?",
        ]),
        ..TableSpec::plain(
            "proxy",
            "proxies",
            &[
                "name",
                "proxy_type",
                "host",
                "port",
                "username",
                "password",
                "country",
                "city",
                "private_key",
                "server_fingerprint",
                "tags",
                "created_at",
            ],
        )
    };

    /// `updated_at` is bumped by launch/stop; syncing it would turn every
    /// launch into a row op.
    #[cfg(desktop)]
    const PROFILE: TableSpec = TableSpec {
        refs: &[Ref {
            key: "workspace_id",
            table: "workspaces",
        }],
        ..TableSpec::plain(
            "profile",
            "profiles",
            &[
                "name",
                "browser_type",
                "proxy_id",
                "fingerprint_preset",
                "user_agent",
                "platform",
                "timezone",
                "locale",
                "languages",
                "screen_width",
                "screen_height",
                "webrtc_mode",
                "geolocation_enabled",
                "latitude",
                "longitude",
                "notes",
                "workspace_id",
                "kanban_status",
                "kanban_order",
                "tags",
                "webgl_vendor",
                "webgl_renderer",
                "default_search_engine",
                "history_enabled",
                "created_at",
            ],
        )
    };

    /// What sync writes or removes of a workspace, a proxy or a profile is
    /// named in the labels, as the commands name what they change.
    pub(super) fn register(registry: &mut Registry) {
        #[cfg(desktop)]
        {
            registry.table(
                WORKSPACE,
                Hooks {
                    after_row: Some(directory::workspace_synced),
                    ..Hooks::default()
                },
            );
            registry.table(
                WORKSPACE_COLUMN,
                Hooks {
                    before_upsert: Some(column_before_upsert),
                    ..Hooks::default()
                },
            );
            registry.table(
                PROXY,
                Hooks {
                    after_row: Some(directory::proxy_synced),
                    ..Hooks::default()
                },
            );
            registry.table(
                PROFILE,
                Hooks {
                    before_upsert: Some(profile_before_upsert),
                    on_delete: Some(profile_on_delete),
                    after_row: Some(directory::profile_synced),
                    ..Hooks::default()
                },
            );
            registry.handler("profile files", crate::sync::profile_files::ProfileFiles);
        }
        registry.profile_conflicts(crate::sync::state::profile_conflicts);
        registry.profile_leases(crate::sync::state::profile_leases);
    }

    /// The tags a profile carries are renamed as earlier ops of the pull
    /// renamed them; a new row gets the device-local columns and its folder.
    #[cfg(desktop)]
    fn profile_before_upsert<'a>(cx: &'a mut Upsert<'_>) -> BoxFuture<'a, CmdResult<()>> {
        Box::pin(async move {
            rewrite_json_tags(cx.payload, "tags", cx.aliases);
            let dir = cx.data_dir.join("profiles").join(cx.id);
            std::fs::create_dir_all(&dir).map_err(AppError::io)?;
            cx.defaults = vec![
                (
                    "profile_path",
                    Value::String(dir.to_string_lossy().into_owned()),
                ),
                ("status", Value::String("stopped".into())),
                ("updated_at", Value::String(chrono::Utc::now().to_rfc3339())),
            ];
            Ok(())
        })
    }

    /// Row and directory through `remove_profile`; not while the browser runs it.
    #[cfg(desktop)]
    fn profile_on_delete<'a>(
        host: &'a Host<'a>,
        core: &'a veydan_core::Core,
        id: &'a str,
    ) -> BoxFuture<'a, CmdResult<Deletion>> {
        Box::pin(async move {
            let browser = host
                .state::<crate::browser::BrowserState>()
                .ok_or_else(|| AppError::other("the browser is not set up"))?;
            if browser.running.is_running(id).await {
                return Ok(Deletion::Later);
            }
            crate::commands::profiles::crud::remove_profile(id, core).await?;
            Ok(Deletion::Done)
        })
    }

    /// A column with the name of a local one is that column: the values of the
    /// row whose id sorts first win, and the tag of the losing column is
    /// renamed on every profile — and in the profiles of the ops that follow.
    #[cfg(desktop)]
    fn column_before_upsert<'a>(cx: &'a mut Upsert<'_>) -> BoxFuture<'a, CmdResult<()>> {
        Box::pin(async move {
            let Some(twin) = &cx.twin else {
                return Ok(());
            };
            let remote_wins = twin.remote_wins;
            let remote_tag = cx
                .payload
                .get("tag_name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let local_tag = column_tag_name(cx.db, &twin.id).await?.unwrap_or_default();
            if remote_wins && !remote_tag.is_empty() && remote_tag != local_tag {
                retarget_profile_tags(cx.db, &local_tag, &remote_tag).await?;
                if !local_tag.is_empty() {
                    cx.aliases.insert(local_tag, remote_tag);
                }
            } else if !remote_wins && !remote_tag.is_empty() && remote_tag != local_tag {
                retarget_profile_tags(cx.db, &remote_tag, &local_tag).await?;
                cx.aliases.insert(remote_tag, local_tag);
            }
            if !remote_wins {
                cx.keep_local.push("tag_name");
            }
            Ok(())
        })
    }

    #[cfg(desktop)]
    async fn column_tag_name(db: &Pool<Sqlite>, id: &str) -> CmdResult<Option<String>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT tag_name FROM workspace_columns WHERE id = ?")
                .bind(id)
                .fetch_optional(db)
                .await
                .map_err(AppError::db)?;
        Ok(row.map(|(t,)| t))
    }

    /// Rewrite a tag string on every profile that still uses it.
    #[cfg(desktop)]
    async fn retarget_profile_tags(db: &Pool<Sqlite>, from: &str, to: &str) -> CmdResult<()> {
        if from == to || from.is_empty() {
            return Ok(());
        }
        let rows: Vec<(String, String)> = sqlx::query_as("SELECT id, tags FROM profiles")
            .fetch_all(db)
            .await
            .map_err(AppError::db)?;
        for (pid, tags) in rows {
            let mut v: Vec<String> = serde_json::from_str(&tags).unwrap_or_default();
            let mut changed = false;
            for t in &mut v {
                if t == from {
                    *t = to.to_string();
                    changed = true;
                }
            }
            if !changed {
                continue;
            }
            let json = serde_json::to_string(&v).unwrap_or_else(|_| "[]".into());
            sqlx::query("UPDATE profiles SET tags = ? WHERE id = ?")
                .bind(json)
                .bind(pid)
                .execute(db)
                .await
                .map_err(AppError::db)?;
        }
        Ok(())
    }

    #[cfg(desktop)]
    fn rewrite_json_tags(
        payload: &mut Map<String, Value>,
        key: &str,
        aliases: &HashMap<String, String>,
    ) {
        if aliases.is_empty() {
            return;
        }
        let Some(v) = payload.get(key) else { return };
        let mut tags: Vec<String> = match v {
            Value::String(s) => serde_json::from_str(s).unwrap_or_default(),
            Value::Array(a) => a
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect(),
            _ => return,
        };
        let mut changed = false;
        for t in &mut tags {
            if let Some(n) = aliases.get(t) {
                *t = n.clone();
                changed = true;
            }
        }
        if !changed {
            return;
        }
        payload.insert(
            key.into(),
            Value::String(serde_json::to_string(&tags).unwrap_or_else(|_| "[]".into())),
        );
    }
}
