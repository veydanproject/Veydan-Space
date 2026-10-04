// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The app of a product, built from its modules.

use crate::module::{Module, Setup};
use crate::router::{DuplicateCommand, Router};
use crate::services::{Part, Product, Shell};
use crate::start::{StartError, StartState};
use sqlx::{Pool, Sqlite};
use std::path::{Path, PathBuf};
use tauri::{Manager, Runtime, WebviewWindowBuilder};
use veydan_core::db::{self, OpenError};
use veydan_core::{Core, Directory, Schema};
use veydan_sync_host::{Registry, SyncManager};

/// Folder of the data file and everything stored beside it, inside the
/// directory the OS (or `--workdir`) gives the app.
const DATA_SUBDIR: &str = "data";

/// The data directory: `<app_data_dir>/data`, the same on every platform.
fn data_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(DATA_SUBDIR)
}

/// The shell's own module: its commands, the keys of the tray it syncs.
pub(crate) fn shell_module() -> Module {
    crate::commands::module()
}

/// The modules of a product with the shell's own in front.
fn with_shell(modules: Vec<Module>) -> Vec<Module> {
    let mut all = vec![shell_module()];
    all.extend(modules);
    all
}

fn router(product: Product, modules: Vec<Module>) -> Result<Router, DuplicateCommand> {
    Router::new(
        product.id,
        modules
            .into_iter()
            .map(|module| (module.id, module.commands, module.handler))
            .collect(),
    )
}

/// The schemas of the data file of a product made of `modules`: the core's,
/// the lock's, sync's, then each module's in the order of the list.
pub fn schemas(modules: &[Module]) -> Vec<Schema> {
    [
        veydan_core::SCHEMA,
        crate::lock::SCHEMA,
        veydan_sync_host::SCHEMA,
    ]
    .into_iter()
    .chain(modules.iter().filter_map(|module| module.schema))
    .collect()
}

/// What a product made of `modules` syncs: the system entities, then what
/// each module registers in the order of the list, the shell's first;
/// checked against the plan of the product.
pub fn sync_registry(product: Product, modules: Vec<Module>) -> Result<Registry, String> {
    registry(product, &with_shell(modules))
}

fn registry(product: Product, modules: &[Module]) -> Result<Registry, String> {
    let mut registry = Registry::new();
    for module in modules {
        if let Some(sync) = module.sync {
            registry.add(module.id, sync);
        }
    }
    registry.finish(product.sync)
}

/// Every command of a product made of `modules`, the shell's own included,
/// with the id of the module that answers it; ordered by the command.
pub fn command_table(
    product: Product,
    modules: Vec<Module>,
) -> Result<Vec<(&'static str, &'static str)>, DuplicateCommand> {
    Ok(router(product, with_shell(modules))?.table())
}

/// What the app's setup needs of the modules, taken before their handlers
/// move into the router.
struct Start {
    product: Product,
    schemas: Vec<Schema>,
    before_open: Vec<fn(&Path)>,
    directory: Vec<fn(&mut Directory)>,
    setups: Vec<(&'static str, Setup)>,
    parts: Vec<Part>,
    key_users: Vec<veydan_lock::KeyUser>,
    sync: Registry,
}

impl Start {
    /// Refused when the modules register for sync what the plan of the
    /// product does not hold.
    fn of(product: Product, modules: &[Module]) -> Result<Self, String> {
        Ok(Self {
            product,
            schemas: schemas(modules),
            before_open: modules.iter().filter_map(|m| m.before_open).collect(),
            directory: modules.iter().filter_map(|m| m.directory).collect(),
            setups: modules.iter().map(|m| (m.id, m.setup)).collect(),
            parts: modules.iter().map(Part::of).collect(),
            key_users: modules.iter().filter_map(|m| m.key_user).collect(),
            sync: registry(product, modules)?,
        })
    }

    /// Open the data in `data_dir` and start the modules on it; `false` when
    /// the data cannot be opened: nothing is started and the app says why.
    fn run(
        self,
        app: &mut tauri::App,
        data_dir: PathBuf,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        std::fs::create_dir_all(&data_dir)?;
        for before_open in &self.before_open {
            before_open(&data_dir);
        }
        let fresh = !data_dir.join(db::DB_FILE).exists();
        let Some(db) = open_data(app.handle(), &data_dir, &self.schemas) else {
            return Ok(false);
        };

        let mut directory = Directory::default();
        for provide in &self.directory {
            provide(&mut directory);
        }
        directory.attach(app.handle().clone());
        app.manage(Core::new(db, data_dir, directory));

        let shell = Shell::new(app.handle().clone(), self.product, self.parts);
        tauri::async_runtime::block_on(shell.load_switches(fresh));
        #[cfg(desktop)]
        tauri::async_runtime::block_on(shell.load_tray_settings());
        app.manage(shell);
        crate::lock::start(app, self.key_users);
        app.manage(SyncManager::new(self.sync));
        #[cfg(desktop)]
        crate::services::follow_synced_tray_settings(app);

        for (id, setup) in self.setups {
            setup(app).map_err(|e| format!("setup of module `{id}` failed: {e}"))?;
        }
        // The cycle carries the data of the modules: it starts once they are
        // set up, whether they are on or off — what is off keeps syncing.
        veydan_sync_host::start(app.handle());
        // The background work of the modules the user keeps on.
        tauri::async_runtime::block_on(app.state::<Shell>().start_modules());
        Ok(true)
    }
}

/// Take over the windows the config opens; returns their labels. Tauri
/// builds them before the setup of the app, and on a phone the webview loads
/// its page and sends its first commands while the setup still runs (the
/// lock derives its key there, the modules start): those commands were never
/// answered and the page stayed blank. The shell builds them once the setup
/// is done ([`open_windows`]), so no command can come before it.
fn hold_windows(config: &mut tauri::Config) -> Vec<String> {
    config
        .app
        .windows
        .iter_mut()
        .filter(|window| window.create)
        .map(|window| {
            window.create = false;
            window.label.clone()
        })
        .collect()
}

/// Build the windows [`hold_windows`] took over, each with the webview
/// storage of the `--workdir` profile when there is one.
fn open_windows<R: Runtime>(app: &tauri::AppHandle<R>, labels: &[String]) -> tauri::Result<()> {
    let windows = app.config().app.windows.clone();
    for config in windows.iter().filter(|w| labels.contains(&w.label)) {
        let builder = WebviewWindowBuilder::from_config(app, config)?;
        #[cfg(desktop)]
        let builder = crate::workdir::apply_to_window(builder);
        builder.build()?;
    }
    Ok(())
}

/// The tray and the main window as the user set them, once the window exists.
#[cfg(desktop)]
fn show_tray(app: &tauri::App) {
    let settings = app.state::<Shell>().tray_settings();
    if settings.wants_tray() {
        crate::tray::apply(app.handle(), true);
    }
    if settings.start_hidden {
        crate::tray::hide_main_window(app.handle());
    }
    crate::tray::watch_main_window(app.handle());
}

/// Open the data file in `data_dir`. `None` when what sits there is not a
/// data file this build opens: it is left alone, the reason is recorded for
/// `app_start_error`, and the caller must go no further.
fn open_data(app: &tauri::AppHandle, data_dir: &Path, schemas: &[Schema]) -> Option<Pool<Sqlite>> {
    let db_path = data_dir.join(db::DB_FILE);
    let error = match tauri::async_runtime::block_on(db::open(&db_path, schemas)) {
        Ok(db) => return Some(db),
        Err(OpenError::Foreign(path)) => {
            eprintln!("{} is not ours; left untouched", path.display());
            StartError::DbForeign {
                path: path.to_string_lossy().into_owned(),
            }
        }
        Err(OpenError::OtherSchema(why)) => {
            eprintln!(
                "data file {} has another schema: {why}; left untouched",
                db_path.display()
            );
            StartError::DbDevSchema {
                path: db_path.to_string_lossy().into_owned(),
            }
        }
        Err(OpenError::Failed(e)) => panic!("Failed to initialize database: {e:#}"),
    };
    app.state::<StartState>().fail(error);
    None
}

/// Starts the product. `context` comes from `tauri::generate_context!()`
/// expanded in the product crate: the macro reads the Tauri config, the
/// capabilities and the version of the crate it expands in.
///
/// Panics before any window exists when two modules declare one command, or
/// when the modules register for sync what the plan of the product does not
/// hold.
pub fn run(context: tauri::Context, product: Product, modules: Vec<Module>) {
    let mut context = context;
    #[cfg(desktop)]
    let workdir = match crate::workdir::init(&context.config().identifier) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("{}: {e}", context.package_info().crate_name);
            std::process::exit(2);
        }
    };
    #[cfg(desktop)]
    if let Some(w) = workdir {
        w.apply_to_context(&mut context, product.name);
    }
    let windows = hold_windows(context.config_mut());

    let modules = with_shell(modules);
    let start = match Start::of(product, &modules) {
        Ok(start) => start,
        Err(refused) => panic!("{}: sync: {refused}", product.id),
    };
    let router = match router(product, modules) {
        Ok(router) => router,
        Err(duplicate) => panic!("{}: {duplicate}", product.id),
    };

    let builder = tauri::Builder::default();
    #[cfg(desktop)]
    let builder = builder
        // Must be first: second launch is closed here before other plugins run.
        // No key of its own: the plugin takes the product's identifier.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            crate::tray::show_main_window(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init());
    #[cfg(mobile)]
    let builder = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init());

    let builder = builder
        .manage(StartState::default())
        .setup(move |app| {
            // A --workdir profile keeps the same layout inside its own directory.
            #[cfg(desktop)]
            let app_data_dir = match workdir {
                Some(workdir) => workdir.root.clone(),
                None => app.path().app_data_dir()?,
            };
            #[cfg(mobile)]
            let app_data_dir = app.path().app_data_dir()?;
            let started = start.run(app, data_dir(&app_data_dir))?;
            // Also when nothing started: the page shows why.
            open_windows(app.handle(), &windows)?;
            #[cfg(desktop)]
            if started {
                show_tray(app);
            }
            #[cfg(mobile)]
            let _ = started;
            Ok(())
        })
        .invoke_handler(move |invoke| router.dispatch(invoke));

    #[cfg(desktop)]
    builder
        .build(context)
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                // Nothing was started when the data could not be opened.
                let Some(shell) = app.try_state::<Shell>() else {
                    return;
                };
                shell.begin_exit();
                tauri::async_runtime::block_on(shell.stop_modules());
            }
        });
    #[cfg(mobile)]
    builder.run(context).expect("error while running veydan");
}

#[cfg(test)]
mod tests {
    use super::*;
    use veydan_sync_host::{Hooks, Plan, Step, TableSpec};

    #[tauri::command]
    fn note_ping() {}

    /// Named as a command of the shell is.
    #[tauri::command]
    fn host_info() {}

    /// The system entities of sync, in one step each way.
    const SYSTEM_ROWS: &[&str] = &["password_vault", "setting", "label"];
    const PLAN: Plan = Plan {
        collect: &[Step::rows("app rows", None, "app", SYSTEM_ROWS)],
        apply: &[Step::rows("app rows", None, "app", SYSTEM_ROWS)],
        late: &[],
    };

    const PRODUCT: Product = Product {
        id: "probe",
        name: "Veydan Probe",
        desktop_entry: "veydanprobe",
        icon: "veydanprobe",
        sync: &PLAN,
    };

    fn notes() -> Module {
        Module {
            schema: Some(Schema {
                module: "notes",
                steps: &["CREATE TABLE notes (id TEXT PRIMARY KEY NOT NULL)"],
            }),
            ..crate::module! { id: "notes", setup: |_app| Ok(()), commands: [note_ping] }
        }
    }

    #[test]
    fn the_shell_is_a_module_of_every_product() {
        let table = command_table(PRODUCT, vec![notes()]).unwrap();
        assert!(table.contains(&("note_ping", "notes")));
        assert!(table.contains(&("app_start_error", "shell")));
        assert!(table.contains(&("host_info", "shell")));
        assert!(table.contains(&("media_grant_access", "shell")));
        assert!(table.contains(&("lock_unlock", "shell")));
        assert!(table.contains(&("vault_replaced_keys_open", "shell")));
        assert!(table.contains(&("sync_status", "shell")));
        assert!(table.contains(&("demo_seed", "shell")));
        assert!(table.contains(&("app_clear_data", "shell")));
        assert!(table.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn a_module_may_not_take_a_command_of_the_shell() {
        let greedy = crate::module! { id: "greedy", setup: |_app| Ok(()), commands: [host_info] };
        assert_eq!(
            command_table(PRODUCT, vec![greedy]).unwrap_err(),
            DuplicateCommand {
                command: "host_info",
                first: "shell",
                second: "greedy"
            }
        );
    }

    #[test]
    fn the_schemas_are_the_cores_and_the_modules() {
        let quiet = crate::module! { id: "quiet", setup: |_app| Ok(()), commands: [] };
        let names: Vec<_> = schemas(&[quiet, notes()])
            .iter()
            .map(|schema| schema.module)
            .collect();
        assert_eq!(names, ["core", "lock", "sync", "notes"]);
    }

    #[test]
    fn the_shell_shares_the_settings_of_the_tray() {
        let registry = sync_registry(PRODUCT, vec![notes()]).unwrap();
        let keys = registry.setting_keys();
        #[cfg(desktop)]
        for key in ["minimize_to_tray", "close_to_tray", "start_hidden"] {
            assert!(keys.contains(&(key, "shell")), "{key}");
        }
        #[cfg(mobile)]
        assert!(keys.is_empty());
    }

    #[test]
    fn what_a_module_syncs_is_in_the_plan_of_the_product() {
        let tags = Module {
            sync: Some(|registry| {
                registry.table(
                    TableSpec::plain("note_tag", "note_tags", &["name"]),
                    Hooks::default(),
                )
            }),
            ..notes()
        };
        let refused = sync_registry(PRODUCT, vec![tags]).err().unwrap();
        assert!(
            refused.contains("`note_tag` of module `notes`"),
            "{refused}"
        );
    }

    /// The page cannot send a command before the setup is done: the window
    /// it lives in does not exist yet.
    #[test]
    fn the_windows_of_the_config_open_after_the_setup() {
        use std::sync::{Arc, Mutex};
        use tauri::utils::config::WindowConfig;

        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        context.config_mut().app.windows = vec![
            WindowConfig {
                label: "main".into(),
                ..Default::default()
            },
            WindowConfig {
                label: "on-demand".into(),
                create: false,
                ..Default::default()
            },
        ];
        let held = hold_windows(context.config_mut());
        assert_eq!(held, ["main"]);
        assert!(context.config().app.windows.iter().all(|w| !w.create));

        let during_setup = Arc::new(Mutex::new(None));
        let seen = during_setup.clone();
        let mut app = tauri::test::mock_builder()
            .setup(move |app| {
                *seen.lock().unwrap() = Some(app.webview_windows().len());
                open_windows(app.handle(), &held)?;
                Ok(())
            })
            .build(context)
            .unwrap();
        assert!(app.get_webview_window("main").is_none());
        #[allow(deprecated)]
        app.run_iteration(|_, _| {});
        assert_eq!(*during_setup.lock().unwrap(), Some(0));
        assert!(app.get_webview_window("main").is_some());
        assert!(app.get_webview_window("on-demand").is_none());
    }

    #[test]
    fn the_data_is_kept_in_a_folder_of_its_own() {
        assert_eq!(
            data_dir(Path::new("/home/u/.local/share/net.veydan.space")),
            Path::new("/home/u/.local/share/net.veydan.space/data")
        );
    }
}
