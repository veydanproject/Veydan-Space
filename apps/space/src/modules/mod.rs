// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The modules of Veydan Space that have no crate of their own — browser,
//! ssh and the services backup and capture — and the tests of the product
//! as a whole. Which command belongs to which module is section 22 of
//! internal/platform-spec.md; `commands.golden.txt` pins it.

#[cfg(desktop)]
pub(crate) mod backup;
pub(crate) mod browser;
#[cfg(desktop)]
pub(crate) mod capture;
#[cfg(desktop)]
pub(crate) mod directory;
#[cfg(all(test, desktop))]
mod entities_tests;
pub(crate) mod ssh;

// The list and the plan are the product's description in lib.rs.
#[cfg(any(desktop, test))]
pub(crate) use crate::module_list as all;
#[cfg(test)]
pub(crate) use crate::SYNC_PLAN;
#[cfg(test)]
use veydan_shell::Module;

/// A click on a tray entry that leads to a page of the app: the id of such
/// an entry is `nav:<route>`. Any other id is not for this function.
#[cfg(desktop)]
fn tray_navigate(app: &tauri::AppHandle, id: &str) {
    use tauri::{Emitter, Manager};
    let Some(route) = id.strip_prefix("nav:") else {
        return;
    };
    app.state::<veydan_shell::Shell>().show_main_window();
    let _ = app.emit("tray://navigate", route.to_string());
}

/// What the modules keep under the key of the vault, as the shell hands it
/// to the lock.
#[cfg(test)]
pub(crate) fn key_users() -> Vec<veydan_lock::KeyUser> {
    all().iter().filter_map(|module| module.key_user).collect()
}

/// The parts of the demo data with the id of each module, in the order of
/// `all()`, on any runtime: what `Module::demo` holds on the app's.
#[cfg(test)]
pub(crate) fn demo_parts<R: tauri::Runtime>() -> Vec<veydan_shell::DemoPart<R>> {
    demo_parts_of().into_iter().map(|(_, part)| part).collect()
}

#[cfg(test)]
fn demo_parts_of<R: tauri::Runtime>() -> Vec<(&'static str, veydan_shell::DemoPart<R>)> {
    use crate::commands::demo;
    let mut parts = vec![("pass", veydan_pass::demo()), ("browser", demo::browser())];
    #[cfg(desktop)]
    parts.push(("ssh", demo::ssh()));
    parts.push(("notes", veydan_notes::demo()));
    #[cfg(desktop)]
    parts.push(("capture", demo::capture()));
    parts
}

/// The parts of a backup with the id of each module, in the order of
/// `all()`, on any runtime: what `Module::backup` holds on the app's. A
/// phone has no backups.
#[cfg(all(test, desktop))]
pub(crate) fn backup_parts<R: tauri::Runtime>() -> Vec<(&'static str, veydan_shell::BackupPart<R>)>
{
    vec![
        ("browser", browser::backup()),
        ("notes", veydan_notes::backup()),
        ("messenger", veydan_messenger_app::backup()),
    ]
}

/// What a module provides to the entity directory, on any runtime.
#[cfg(test)]
pub(crate) type Provide<R> = fn(&mut veydan_core::Directory<R>);

/// What the modules provide to the entity directory, with the id of each
/// module, in the order of `all()`, on any runtime: what `Module::directory`
/// holds on the app's. A phone has no browser and no ssh to answer for
/// their kinds: the labels do.
#[cfg(test)]
pub(crate) fn directory_parts<R: tauri::Runtime>() -> Vec<(&'static str, Provide<R>)> {
    let mut parts: Vec<(&'static str, Provide<R>)> = vec![("pass", veydan_pass::provide::<R>)];
    #[cfg(desktop)]
    parts.extend([
        (
            "browser",
            directory::browser::<R> as Provide<R>,
        ),
        ("ssh", directory::ssh::<R>),
    ]);
    parts.push(("notes", veydan_notes::provide::<R>));
    parts
}

/// The states the setups put in place, over a data file of its own in a
/// fresh directory and without the app: for tests of code that takes them.
#[cfg(all(test, desktop))]
pub(crate) struct TestStates {
    pub core: veydan_core::Core,
    pub lock: veydan_lock::Lock,
    pub notes: veydan_notes::NotesState,
    pub browser: crate::browser::BrowserState,
    pub sync: veydan_sync_host::SyncManager,
}

#[cfg(all(test, desktop))]
impl TestStates {
    pub(crate) async fn new() -> Self {
        Self::syncing(sync_registry()).await
    }

    /// The states of a product whose modules registered `registry`; no
    /// module owns a kind of the directory, so none publishes a label.
    pub(crate) async fn syncing(registry: veydan_sync_host::Registry) -> Self {
        Self::owning(registry, &[]).await
    }

    /// The same where the modules named `owners` own their kinds of the
    /// directory: what they change, here or from sync, they name in the
    /// labels. Without the app their providers are never called.
    pub(crate) async fn owning(registry: veydan_sync_host::Registry, owners: &[&str]) -> Self {
        let (db, data_dir) = crate::db::test_pool().await;
        std::fs::create_dir_all(data_dir.join("profiles")).unwrap();
        veydan_notes::ensure_dirs(&data_dir).unwrap();
        let mut directory = veydan_core::Directory::default();
        for (_, provide) in directory_parts()
            .into_iter()
            .filter(|(id, _)| owners.contains(id))
        {
            provide(&mut directory);
        }
        Self {
            lock: veydan_lock::Lock::with_key_users(db.clone(), key_users()),
            core: veydan_core::Core::new(db, data_dir, directory),
            notes: veydan_notes::NotesState::new(None),
            browser: crate::browser::BrowserState::default(),
            sync: veydan_sync_host::SyncManager::new(registry),
        }
    }

    /// What the first start of a build with labels does over this data
    /// file: every owner of Space lists its entities and publishes them.
    pub(crate) async fn publish_labels(&self) -> usize {
        let app = tauri::test::mock_app();
        tauri::Manager::manage(
            &app,
            veydan_core::Core::new(
                self.core.db.clone(),
                self.core.app_data_dir.clone(),
                veydan_core::Directory::default(),
            ),
        );
        let mut directory = veydan_core::Directory::<tauri::test::MockRuntime>::default();
        for (_, provide) in directory_parts() {
            provide(&mut directory);
        }
        directory.attach(app.handle().clone());
        directory.keep_labels(self.core.db.clone());
        directory.publish_all().await.unwrap()
    }

    /// Where the hooks and handlers of sync find these states.
    pub(crate) fn host(&self) -> veydan_sync_host::Host<'_> {
        veydan_sync_host::Host::States(self)
    }
}

#[cfg(all(test, desktop))]
impl veydan_sync_host::States for TestStates {
    fn state(&self, ty: std::any::TypeId) -> Option<&(dyn std::any::Any + Send + Sync)> {
        let states: [&(dyn std::any::Any + Send + Sync); 5] = [
            &self.core,
            &self.lock,
            &self.notes,
            &self.browser,
            &self.sync,
        ];
        states
            .into_iter()
            .find(|state| std::any::Any::type_id(*state) == ty)
    }
}

/// What Space syncs, as the shell builds it when the app starts.
#[cfg(test)]
pub(crate) fn sync_registry() -> veydan_sync_host::Registry {
    veydan_shell::sync_registry(crate::PRODUCT, all()).unwrap()
}

/// What a product made of the modules of Space named `ids` syncs: the system
/// entities and theirs, in the order of Space's plan, which skips what is
/// not registered. Without `shell` it has no keys of the tray.
#[cfg(test)]
pub(crate) fn sync_registry_of(ids: &[&str]) -> veydan_sync_host::Registry {
    let mut registry = veydan_sync_host::Registry::new();
    for module in all() {
        if let (true, Some(sync)) = (ids.contains(&module.id), module.sync) {
            registry.add(module.id, sync);
        }
    }
    registry.finish(&SYNC_PLAN).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `commands.golden.txt`: every command of the product on a line with the
    /// module that answers it and where it exists (`all` platforms,
    /// `desktop` or `mobile` only). The names are what the frontend invokes: a line may
    /// move to another module, but a name neither appears nor disappears
    /// without a matching change in the UI.
    #[test]
    fn every_command_has_the_owner_the_golden_file_names() {
        let expected: Vec<(&str, &str)> = include_str!("commands.golden.txt")
            .lines()
            .map(|line| {
                let mut words = line.split(' ');
                let (command, module, platform) = (
                    words.next().unwrap(),
                    words.next().unwrap(),
                    words.next().unwrap(),
                );
                assert!(matches!(platform, "all" | "desktop" | "mobile"), "{line}");
                (command, module, platform)
            })
            .filter(|(_, _, platform)| match *platform {
                "desktop" => cfg!(desktop),
                "mobile" => cfg!(mobile),
                _ => true,
            })
            .map(|(command, module, _)| (command, module))
            .collect();
        let table = veydan_shell::command_table(crate::PRODUCT, all()).unwrap();
        for line in &table {
            assert!(expected.contains(line), "not in the golden file: {line:?}");
        }
        for line in &expected {
            assert!(table.contains(line), "the router does not know {line:?}");
        }
        assert_eq!(table.len(), expected.len());
    }

    /// The modules of the crate are those `products.json` gives Space: the
    /// UI is built from that list, the app from `module_list()`. The list of
    /// the manifest is unordered; `capture`, a service without a UI, is not
    /// in it (11.3). A phone has every module of the list but the desktop
    /// ones, with their tables.
    #[test]
    fn the_module_list_is_that_of_products_json() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../../../../products.json")).unwrap();
        let mut listed: Vec<&str> = manifest["targets"]["space"]["modules"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap())
            .collect();
        listed.sort_unstable();
        let mut ours: Vec<&str> = all()
            .iter()
            .map(|module| module.id)
            .filter(|id| *id != "capture")
            .collect();
        ours.sort_unstable();
        #[cfg(mobile)]
        listed.retain(|id| *id != "backup");
        assert_eq!(ours, listed);
    }

    /// The order platform-stage-3 applied a pull in: `cycle_inner` ran the
    /// attachments, the tags, folders and smart views, the notes, their flags,
    /// then the other rows in the order of `rows::SPECS`, and the files of
    /// profiles last. Inside a pass of rows the puts went first in the order
    /// of `SPECS`, then the tombstones in reverse; `note_meta` takes none.
    /// A handler takes both in the order of the pull. The labels of stage 6
    /// come after the rows they name; a phone takes no workspace and no
    /// profile since.
    #[test]
    fn space_applies_a_pull_in_the_order_of_stage_3() {
        #[cfg(desktop)]
        let (puts, deletes) = (
            [
                "note_attachment",
                "note_attachment_v2",
                "note_tag",
                "note_folder",
                "note_smart_view",
                "note",
                "note_meta",
                "workspace",
                "workspace_column",
                "proxy",
                "ssh_key",
                "profile",
                "ssh_connection",
                "totp",
                "password_vault",
                "password",
                "pw_history",
                "setting",
                "label",
                "profile_lease",
                "profile_snapshot",
            ],
            [
                "note_attachment",
                "note_attachment_v2",
                "note_smart_view",
                "note_folder",
                "note_tag",
                "note",
                "setting",
                "pw_history",
                "password",
                "password_vault",
                "totp",
                "ssh_connection",
                "profile",
                "ssh_key",
                "proxy",
                "workspace_column",
                "workspace",
                "label",
                "profile_lease",
                "profile_snapshot",
            ],
        );
        // A phone took the rows of `MOBILE_ENTITIES` and had no profile files.
        #[cfg(mobile)]
        let (puts, deletes) = (
            [
                "note_attachment",
                "note_attachment_v2",
                "note_tag",
                "note_folder",
                "note_smart_view",
                "note",
                "note_meta",
                "totp",
                "password_vault",
                "password",
                "label",
            ],
            [
                "note_attachment",
                "note_attachment_v2",
                "note_smart_view",
                "note_folder",
                "note_tag",
                "note",
                "password",
                "password_vault",
                "totp",
                "label",
            ],
        );
        let (registry_puts, registry_deletes) = sync_registry().apply_order();
        assert_eq!(registry_puts, puts);
        assert_eq!(registry_deletes, deletes);
    }

    /// The shell derives a plan from a list of modules for a product
    /// without an order of its own to keep (9.1). Over Space's modules that
    /// plan takes what Space's takes — the same registry, so the same
    /// fingerprint, each entity in one collect and one apply step, the same
    /// handlers, the profile files late — but not in Space's order. Space
    /// keeps its explicit plan: it collects the rows of notes before the
    /// files of the notes and the other rows after them, and its apply
    /// interleaves the rows of pass, browser and ssh in the order of 4.0.7
    /// (`totp`, `password_vault`, `password` among them), which a plan built
    /// module by module in the order of the list cannot give (18.1, № 62):
    /// the order and the clock of a push are frozen byte for byte (9.4).
    #[test]
    fn the_default_plan_of_spaces_modules_takes_what_spaces_plan_takes_in_another_order() {
        let product = veydan_shell::Product {
            sync: veydan_shell::default_plan(&all()),
            ..crate::PRODUCT
        };
        let derived = veydan_shell::sync_registry(product, all()).unwrap();
        let explicit = sync_registry();
        assert_eq!(derived.fingerprint(), explicit.fingerprint());
        let set = |order: Vec<&'static str>| -> std::collections::BTreeSet<&'static str> {
            order.into_iter().collect()
        };
        let (derived_puts, derived_deletes) = derived.apply_order();
        let (explicit_puts, explicit_deletes) = explicit.apply_order();
        assert_eq!(set(derived_puts.clone()), set(explicit_puts.clone()));
        assert_eq!(set(derived_deletes), set(explicit_deletes));
        let mut handlers = derived.handler_names();
        let mut theirs = explicit.handler_names();
        handlers.sort_unstable();
        theirs.sort_unstable();
        assert_eq!(handlers, theirs);
        assert_ne!(derived_puts, explicit_puts);
        // The profile files apply late in both, and only there.
        assert_eq!(product.sync.late, SYNC_PLAN.late);
        let collected = |plan: &veydan_shell::Plan| -> Vec<&'static str> {
            plan.collect
                .iter()
                .flat_map(|step| match step.part {
                    veydan_sync_host::Part::Rows { entities, .. } => entities.to_vec(),
                    veydan_sync_host::Part::Handler { name, .. } => vec![name],
                })
                .collect()
        };
        assert_ne!(collected(product.sync), collected(&SYNC_PLAN));
    }

    /// What platform-stage-3 synced: the fifteen tables of `rows::SPECS`, the
    /// five entities of the notes, attachments and profile files, the seven
    /// keys of `SETTING_FILTER`. A phone took `MOBILE_ENTITIES` and the notes
    /// with their attachments, and published no workspace and no profile.
    /// Stage 6 adds the labels everywhere, in place of the workspaces and
    /// profiles a phone mirrored.
    #[test]
    fn space_syncs_the_entities_and_settings_of_stage_3() {
        let registry = sync_registry();
        let mut entities = registry.entities();
        entities.sort_unstable();
        let mut keys: Vec<&str> = registry
            .setting_keys()
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        keys.sort_unstable();
        let mirrored: Vec<&str> = registry
            .tables()
            .iter()
            .filter(|t| !t.push)
            .map(|t| t.spec.entity)
            .collect();
        #[cfg(desktop)]
        {
            let mut synced = vec![
                "label",
                "workspace",
                "workspace_column",
                "proxy",
                "ssh_key",
                "profile",
                "ssh_connection",
                "totp",
                "password_vault",
                "password",
                "pw_history",
                "note_tag",
                "note_folder",
                "note_smart_view",
                "note_meta",
                "setting",
                "note",
                "note_attachment",
                "note_attachment_v2",
                "profile_lease",
                "profile_snapshot",
            ];
            synced.sort_unstable();
            assert_eq!(entities, synced);
            assert_eq!(
                keys,
                [
                    "close_to_tray",
                    "minimize_to_tray",
                    "notes_capture_rules",
                    "notes_lock_timeout_min",
                    "quick_capture_shortcut",
                    "start_hidden",
                    "ui_locale",
                ]
            );
            assert!(mirrored.is_empty(), "{mirrored:?}");
        }
        #[cfg(mobile)]
        {
            let mut synced = vec![
                "label",
                "totp",
                "password",
                "password_vault",
                "note_tag",
                "note_folder",
                "note_smart_view",
                "note_meta",
                "note",
                "note_attachment",
                "note_attachment_v2",
            ];
            synced.sort_unstable();
            assert_eq!(entities, synced);
            assert!(keys.is_empty(), "{keys:?}");
            assert!(mirrored.is_empty(), "{mirrored:?}");
        }
    }

    /// The driver of the shell takes the parts from `Module::demo`; the
    /// tests of the demo data from `demo_parts`. Both name the same modules.
    #[test]
    fn the_parts_of_the_demo_data_are_those_of_the_modules() {
        let with_demo: Vec<&str> = all()
            .iter()
            .filter(|module| module.demo.is_some())
            .map(|module| module.id)
            .collect();
        let parts: Vec<&str> = demo_parts_of::<tauri::test::MockRuntime>()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(with_demo, parts);
    }

    /// The driver of backups takes the parts from `Module::backup` through
    /// the shell; its tests from `backup_parts`. Both name the same modules,
    /// and since stage 10 a backup holds the messenger's folder.
    #[cfg(desktop)]
    #[test]
    fn the_parts_of_a_backup_are_those_of_the_modules() {
        let with_backup: Vec<&str> = all()
            .iter()
            .filter(|module| module.backup.is_some())
            .map(|module| module.id)
            .collect();
        let parts: Vec<&str> = backup_parts::<tauri::test::MockRuntime>()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(with_backup, parts);
        assert_eq!(parts, ["browser", "notes", "messenger"]);
    }

    /// The shell fills the directory from `Module::directory`; the tests of
    /// what the notes read through it from `directory_parts`. Both name the
    /// same modules, and the directory of Space knows these kinds.
    #[test]
    fn the_kinds_of_the_directory_are_those_of_the_modules() {
        let with_kinds: Vec<&str> = all()
            .iter()
            .filter(|module| module.directory.is_some())
            .map(|module| module.id)
            .collect();
        let parts = directory_parts::<tauri::test::MockRuntime>();
        let ids: Vec<&str> = parts.iter().map(|(id, _)| *id).collect();
        assert_eq!(with_kinds, ids);
        let mut directory = veydan_core::Directory::<tauri::test::MockRuntime>::default();
        for (_, provide) in parts {
            provide(&mut directory);
        }
        #[cfg(desktop)]
        let kinds = [
            "note",
            "password",
            "profile",
            "proxy",
            "ssh",
            "totp",
            "workspace",
        ]
        .as_slice();
        #[cfg(mobile)]
        let kinds = ["note", "password", "totp"].as_slice();
        assert_eq!(directory.kinds(), kinds);
    }

    #[test]
    fn every_schema_belongs_to_one_module() {
        let schemas = veydan_shell::schemas(&all());
        let mut names: Vec<_> = schemas.iter().map(|schema| schema.module).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            ["browser", "core", "lock", "notes", "pass", "ssh", "sync"]
        );
    }

    /// Each schema is applied to a file of its own, and nothing it leaves
    /// there — table, index, trigger — carries a name another module's
    /// schema leaves too. `IF NOT EXISTS` would let two modules share a table
    /// without either start failing, so no step uses it.
    #[tokio::test]
    async fn every_table_belongs_to_one_module() {
        let mut owners = std::collections::HashMap::new();
        for schema in veydan_shell::schemas(&all()) {
            for step in schema.steps {
                let words: Vec<_> = step.split_whitespace().collect();
                assert!(
                    !words
                        .join(" ")
                        .to_ascii_uppercase()
                        .contains("IF NOT EXISTS"),
                    "a step of module `{}` uses IF NOT EXISTS",
                    schema.module
                );
            }
            let dir = crate::db::test_dir();
            let pool = veydan_core::db::open(&dir.join(veydan_core::db::DB_FILE), &[schema])
                .await
                .unwrap_or_else(|e| panic!("schema of `{}` alone: {e:?}", schema.module));
            let names: Vec<String> = sqlx::query_scalar(
                "SELECT name FROM sqlite_master
                 WHERE name NOT LIKE 'sqlite_%' AND name != 'schema_modules'",
            )
            .fetch_all(&pool)
            .await
            .unwrap();
            pool.close().await;
            std::fs::remove_dir_all(&dir).unwrap();
            assert!(!names.is_empty(), "`{}` creates nothing", schema.module);
            for name in names {
                if let Some(first) = owners.insert(name.clone(), schema.module) {
                    panic!(
                        "`{name}` is created by module `{first}` and by module `{}`",
                        schema.module
                    );
                }
            }
        }
    }

    /// A product without a module answers the module's commands as unknown,
    /// and the UI of the messenger reads "not compiled" from the words of
    /// that answer: the pattern in its `api.ts` has to keep matching it.
    /// Space has the messenger; a product made of its other modules has none
    /// of its commands, and the router answers `messenger_status` so.
    #[test]
    fn the_ui_reads_a_missing_module_from_the_answer_to_an_unknown_command() {
        let api = include_str!("../../../../ui/src/lib/messenger/api.ts");
        assert!(
            api.contains("/command .* not found/i.test(msg)"),
            "isUnknownCommand of ui/src/lib/messenger/api.ts changed its pattern"
        );
        let without: Vec<Module> = all()
            .into_iter()
            .filter(|module| module.id != "messenger")
            .collect();
        let table = veydan_shell::command_table(crate::PRODUCT, without).unwrap();
        assert!(!table.is_empty());
        let messenger = table.iter().filter(|(command, module)| {
            command.starts_with("messenger_") || *module == "messenger"
        });
        assert_eq!(messenger.count(), 0, "{table:?}");
        let answer =
            veydan_shell::unknown_command(crate::PRODUCT.id, "messenger_status").to_lowercase();
        let after = answer
            .split_once("command ")
            .map(|(_, after)| after)
            .unwrap_or_default();
        assert!(after.contains(" not found"), "{answer}");
    }
}
