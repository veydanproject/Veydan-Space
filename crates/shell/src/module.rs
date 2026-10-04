// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a module hands the shell: its commands, its schema, its part of the
//! tray and of the demo data, what to do when the app starts and stops.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tauri::ipc::Invoke;
use tauri::{AppHandle, Runtime, Wry};
use veydan_core::{AppError, BoxFuture, Directory, Schema};

pub type SetupResult = Result<(), Box<dyn std::error::Error>>;

/// What a module does when the app starts.
pub type Setup = fn(&mut tauri::App) -> SetupResult;

/// What `tauri::generate_handler!` expands to, boxed. It takes `Invoke` by
/// value and returns `false` for a name it does not know, so the router picks
/// the module before it calls.
pub type Handler<R = tauri::Wry> = Box<dyn Fn(Invoke<R>) -> bool + Send + Sync + 'static>;

/// Something a module does on the app's behalf, with the handle to reach its
/// state.
pub type Hook = fn(tauri::AppHandle) -> BoxFuture<'static, Result<(), AppError>>;

/// One module of a product. Built with [`module!`](crate::module!), then
/// completed with the fields the module has something to say in.
pub struct Module {
    /// One word, the same in Rust, in the UI and in `products.json`.
    pub id: &'static str,
    /// Names exactly as the frontend invokes them.
    pub commands: &'static [&'static str],
    pub handler: Handler,
    /// Runs inside the app's setup once the data file is open and `Core`,
    /// `Shell` and `veydan_lock::Lock` are managed, in the order the product
    /// lists its modules. The
    /// module manages its own state here and registers the Tauri plugins
    /// only it uses.
    pub setup: Setup,
    /// The tables the module keeps in the data file.
    pub schema: Option<Schema>,
    /// Registers the entity kinds the module owns.
    pub directory: Option<fn(&mut Directory)>,
    pub tray: Option<TrayPart>,
    /// Stops the module's background work: when the user switches the module
    /// off and when the app exits, in the order of the product's list. Runs
    /// on a module that is stopped already too, and does nothing then.
    pub stop: Option<Hook>,
    /// Runs with the data directory before the data file in it is opened,
    /// also in a start that then fails to open it.
    pub before_open: Option<fn(&Path)>,
    /// Registers the entities the module syncs, the setting keys it shares
    /// and what it adds to the status of sync.
    pub sync: Option<fn(&mut veydan_sync_host::Registry)>,
    /// What the module keeps encrypted under the vault key in its own
    /// tables. The lock is made knowing it, before anything opens the key.
    pub key_user: Option<veydan_lock::KeyUser>,
    /// The module's part of the demo data.
    pub demo: Option<DemoPart>,
    /// Starts the module's background work once the app is set up, when the
    /// module is switched on; again when the user switches it on. Runs on a
    /// module that runs already too, and does nothing then.
    pub start: Option<Hook>,
    /// Who switches the module on and off.
    pub switch: Switch,
    /// What a backup of the product takes of the module's data. Read by the
    /// driver of a product that has one.
    pub backup: Option<BackupPart>,
}

/// Who switches a module on and off (platform-spec 12). A module that is
/// off is hidden and its background work is stopped; its data stays and
/// keeps syncing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    /// The user's own switch, in the Modules section of the settings.
    Own,
    /// On and off together with the named module of the product: the web
    /// clipper writes into the notes.
    Follows(&'static str),
    /// Never off: the shell, a service of the product.
    Always,
}

/// A module's part of the demo data: the driver of the shell clears the
/// parts of the product's modules and seeds them, and knows none by name.
pub struct DemoPart<R: Runtime = Wry> {
    /// Fill the tables and folders of the module with the demo data of a
    /// locale (`ru` or `en`; anything else is `en`). A link to an entity of
    /// another module is the id that module's part gives that entity.
    pub seed: fn(AppHandle<R>, String) -> BoxFuture<'static, Result<(), AppError>>,
    /// Empty them.
    pub clear: fn(AppHandle<R>) -> BoxFuture<'static, Result<(), AppError>>,
}

impl<R: Runtime> Clone for DemoPart<R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R: Runtime> Copy for DemoPart<R> {}

/// A module's part of a backup: the driver copies the data file and the
/// paths of the parts of the product's modules, and knows none by name.
/// Generic over the runtime like [`DemoPart`]: in `Module::backup` it is on
/// the app's, tests of a driver take the same parts on a mock one.
pub struct BackupPart<R: Runtime = Wry> {
    /// The folders and files of the module: inside the data directory of the
    /// product (a relative path of one folder or file) or outside it
    /// (absolute, `external`). Called each time a copy is made.
    pub paths: fn(&AppHandle<R>) -> Vec<BackupPath>,
}

impl<R: Runtime> Clone for BackupPart<R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R: Runtime> Copy for BackupPart<R> {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupPath {
    /// What the copy calls it: the name of its folder in the archive.
    pub name: &'static str,
    pub path: PathBuf,
    /// Outside the data directory: the path is absolute.
    pub external: bool,
}

/// A module's entries in the tray menu.
#[derive(Clone, Copy)]
pub struct TrayPart {
    /// The module's entries; called every time the menu is rebuilt.
    pub items: fn(tauri::AppHandle, TrayLabels) -> BoxFuture<'static, Vec<TrayItem>>,
    /// A click on an entry of this module; `id` is the one of its `TrayItem`.
    pub on_click: fn(&tauri::AppHandle, &str),
    /// The first line of the tray's tooltip. The first module of the product
    /// that has one gives it; without any it is the product's name.
    pub tooltip: Option<fn(tauri::AppHandle, TrayLabels) -> BoxFuture<'static, String>>,
}

/// Where the tray puts an entry of a module. The groups follow each other in
/// this order with a line between them; inside a group the entries keep the
/// order of the product's module list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TrayGroup {
    /// Lists that change while the app runs.
    Lists,
    /// Places of the app to go to.
    Sections,
    /// Things to do at once.
    Actions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayItem {
    /// Unique among the entries of the module.
    pub id: String,
    pub label: String,
    pub enabled: bool,
    /// Not empty: the entry is a submenu of these.
    pub children: Vec<TrayItem>,
    /// Read on the module's top entries only.
    pub group: TrayGroup,
    /// A line instead of an entry.
    pub separator: bool,
}

impl TrayItem {
    pub fn entry(group: TrayGroup, id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            enabled: true,
            children: Vec::new(),
            group,
            separator: false,
        }
    }

    /// An entry that says something and cannot be clicked.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    pub fn submenu(mut self, children: Vec<TrayItem>) -> Self {
        self.children = children;
        self
    }

    pub fn separator(group: TrayGroup) -> Self {
        Self {
            separator: true,
            ..Self::entry(group, "", "")
        }
    }
}

/// The translated labels the UI hands over with `tray_set_labels`. The shell
/// keeps them as they come and does not know the keys of the modules.
pub type TrayLabels = HashMap<String, String>;

/// The label under `key`, or the English `default` until the UI has handed
/// the active locale over.
pub fn tray_label(labels: &TrayLabels, key: &str, default: &str) -> String {
    labels
        .get(key)
        .cloned()
        .unwrap_or_else(|| default.to_string())
}

/// Builds a [`Module`] from one list of command paths: the same tokens go to
/// `tauri::generate_handler!` and to the list of names, so the two cannot
/// drift. `#[cfg(..)]` on an entry applies to both. The fields a module may
/// leave out are `None`; set them with struct update syntax.
///
/// The name of a command is the last segment of its path, so a command
/// declared with `#[tauri::command(rename = "..")]` would be listed under
/// its function name while the handler matches the renamed string. A path
/// leads to the module the command is defined in: a re-export of the function
/// alone does not carry the macros `#[tauri::command]` puts beside it.
#[macro_export]
macro_rules! module {
    (
        id: $id:expr,
        setup: $setup:expr,
        commands: [ $( $(#[$attr:meta])* $($seg:ident)::+ ),* $(,)? ] $(,)?
    ) => {
        $crate::Module {
            id: $id,
            commands: &[ $( $(#[$attr])* $crate::__last_segment!($($seg)::+) ),* ],
            handler: ::std::boxed::Box::new(
                ::tauri::generate_handler![ $( $(#[$attr])* $($seg)::+ ),* ]
            ),
            setup: $setup,
            schema: ::std::option::Option::None,
            directory: ::std::option::Option::None,
            tray: ::std::option::Option::None,
            stop: ::std::option::Option::None,
            before_open: ::std::option::Option::None,
            sync: ::std::option::Option::None,
            key_user: ::std::option::Option::None,
            demo: ::std::option::Option::None,
            start: ::std::option::Option::None,
            switch: $crate::Switch::Own,
            backup: ::std::option::Option::None,
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __last_segment {
    ($last:ident) => { ::std::stringify!($last) };
    ($head:ident :: $($rest:ident)::+) => { $crate::__last_segment!($($rest)::+) };
}

#[cfg(test)]
mod tests {
    use super::*;

    mod commands {
        #[tauri::command]
        pub fn one() {}

        #[tauri::command]
        pub fn two() {}

        #[cfg(desktop)]
        #[tauri::command]
        pub fn desktop_only() {}

        #[cfg(mobile)]
        #[tauri::command]
        pub fn mobile_only() {}
    }

    fn module() -> Module {
        crate::module! {
            id: "probe",
            setup: |_app| Ok(()),
            commands: [
                commands::one,
                self::commands::two,
                #[cfg(desktop)]
                commands::desktop_only,
                #[cfg(mobile)]
                commands::mobile_only,
            ],
        }
    }

    #[test]
    fn the_name_of_a_command_is_the_last_segment_of_its_path() {
        assert_eq!(crate::__last_segment!(a::b::c_d), "c_d");
        assert_eq!(crate::__last_segment!(solo), "solo");
        assert_eq!(crate::__last_segment!(crate::a::b), "b");
    }

    #[test]
    fn the_names_follow_the_cfg_of_the_handler_list() {
        let module = module();
        assert_eq!(module.id, "probe");
        #[cfg(desktop)]
        assert_eq!(module.commands, &["one", "two", "desktop_only"]);
        #[cfg(mobile)]
        assert_eq!(module.commands, &["one", "two", "mobile_only"]);
        assert!(module.schema.is_none() && module.tray.is_none() && module.stop.is_none());
        assert!(module.key_user.is_none() && module.demo.is_none() && module.backup.is_none());
        assert!(module.start.is_none());
        assert_eq!(module.switch, Switch::Own);
    }

    #[test]
    fn a_module_may_have_no_commands() {
        let module = crate::module! { id: "quiet", setup: |_app| Ok(()), commands: [] };
        assert!(module.commands.is_empty());
    }

    /// A build script that forgot `veydan_build_cfg` would leave both unset.
    #[test]
    fn exactly_one_platform_is_set() {
        assert!(cfg!(desktop) != cfg!(mobile));
    }

    #[test]
    fn a_label_falls_back_to_its_default() {
        let mut labels = TrayLabels::new();
        assert_eq!(tray_label(&labels, "quit", "Quit"), "Quit");
        labels.insert("quit".into(), "Выход".into());
        assert_eq!(tray_label(&labels, "quit", "Quit"), "Выход");
    }

    #[test]
    fn a_separator_is_not_an_entry() {
        let line = TrayItem::separator(TrayGroup::Lists);
        assert!(line.separator && line.children.is_empty());
        let menu = TrayItem::entry(TrayGroup::Lists, "m", "Menu").submenu(vec![TrayItem::entry(
            TrayGroup::Lists,
            "a",
            "A",
        )
        .disabled()]);
        assert!(!menu.separator && menu.enabled && !menu.children[0].enabled);
    }
}
