// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a module takes from the host it runs in, without knowing which
//! product that is: `app.state::<veydan_shell::Shell>()`.

#[cfg(desktop)]
use crate::module::TrayPart;
use crate::module::{BackupPart, BackupPath, DemoPart, Hook, Module, Switch};
use crate::source::{self, FileSource};
use crate::switches::Switches;
#[cfg(desktop)]
use crate::tray::{self, TraySettings, TrayState};
use tauri::AppHandle;
#[cfg(desktop)]
use tauri::Manager;
use veydan_core::AppError;
#[cfg(desktop)]
use veydan_core::{settings, Core};

/// The strings of a product and the order of its sync. Its identifier and
/// its version are not here: they come from the product's Tauri config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Product {
    /// One word, as in `products.json`: `space`.
    pub id: &'static str,
    /// As the user reads it: `Veydan Space`.
    pub name: &'static str,
    /// The desktop entry it is installed under, without `.desktop`: the
    /// notifications of a Linux desktop are grouped by it.
    pub desktop_entry: &'static str,
    /// The name of its icon in the icon theme (Linux).
    pub icon: &'static str,
    /// The order of its sync cycle over the entities of its modules.
    pub sync: &'static veydan_sync_host::Plan,
}

/// What the shell keeps of a module once the app runs.
pub(crate) struct Part {
    pub(crate) id: &'static str,
    /// The tray exists on a computer only.
    #[cfg(desktop)]
    tray: Option<TrayPart>,
    pub(crate) start: Option<Hook>,
    pub(crate) stop: Option<Hook>,
    pub(crate) switch: Switch,
    demo: Option<DemoPart>,
    backup: Option<BackupPart>,
}

impl Part {
    pub(crate) fn of(module: &Module) -> Self {
        Self {
            id: module.id,
            #[cfg(desktop)]
            tray: module.tray,
            start: module.start,
            stop: module.stop,
            switch: module.switch,
            demo: module.demo,
            backup: module.backup,
        }
    }
}

/// The settings of the tray. Shared between desktops: sync registers them
/// for the shell, and a value another desktop set is followed here.
#[cfg(desktop)]
pub(crate) const TRAY_KEYS: [&str; 3] = ["minimize_to_tray", "close_to_tray", "start_hidden"];

/// Follow a tray setting sync applied: the watchers of `Core::settings` hear
/// of it once the apply is committed.
#[cfg(desktop)]
pub(crate) fn follow_synced_tray_settings(app: &tauri::App) {
    let core = app.state::<Core>();
    for key in TRAY_KEYS {
        let handle = app.handle().clone();
        core.settings.subscribe(key, move |_| {
            let app = handle.clone();
            Box::pin(async move {
                if let Some(shell) = app.try_state::<Shell>() {
                    shell.reload_tray_settings().await;
                }
            })
        });
    }
}

/// The state of the shell; managed before any module is set up.
pub struct Shell {
    pub(crate) app: AppHandle,
    product: Product,
    /// In the order of the product's list.
    pub(crate) parts: Vec<Part>,
    /// Which modules the user keeps on (section 12).
    pub(crate) switches: Switches,
    #[cfg(desktop)]
    pub(crate) tray: TrayState,
}

impl Shell {
    pub(crate) fn new(app: AppHandle, product: Product, parts: Vec<Part>) -> Self {
        Self {
            app,
            product,
            parts,
            switches: Switches::default(),
            #[cfg(desktop)]
            tray: TrayState::default(),
        }
    }

    pub fn product(&self) -> &Product {
        &self.product
    }

    /// Bring the main window back to the foreground, out of the tray if it
    /// was stashed there. Safe from any thread.
    #[cfg(desktop)]
    pub fn show_main_window(&self) {
        tray::show_main_window(&self.app);
    }

    /// What waits in `module`. For the sum over the modules the tray shows a
    /// dot and a line in its tooltip, and the icon of the app a count. They
    /// are redrawn when the sum changes and at a module's first count, so a
    /// count left on the icon by the last run goes at start.
    #[cfg(desktop)]
    pub fn set_unread(&self, module: &'static str, n: usize) {
        if self.tray.set_unread(module, n) {
            tray::refresh(&self.app);
            tray::show_badge(&self.app);
        }
    }

    /// The main window of the product, while it exists.
    #[cfg(desktop)]
    pub fn main_window(&self) -> Option<tauri::WebviewWindow> {
        self.app.get_webview_window("main")
    }

    /// Rebuild the tray menu from `Module::tray` of the product's modules.
    #[cfg(desktop)]
    pub fn tray_refresh(&self) {
        tray::refresh(&self.app);
    }

    #[cfg(desktop)]
    pub fn tray_settings(&self) -> TraySettings {
        self.tray.settings()
    }

    /// Store the tray behavior and follow it at once.
    #[cfg(desktop)]
    pub async fn set_tray_settings(&self, s: TraySettings) -> Result<(), AppError> {
        let db = &self.app.state::<Core>().db;
        settings::set_bool(db, "minimize_to_tray", s.minimize_to_tray).await?;
        settings::set_bool(db, "close_to_tray", s.close_to_tray).await?;
        settings::set_bool(db, "start_hidden", s.start_hidden).await?;
        self.follow_tray_settings(s);
        Ok(())
    }

    /// Re-read the tray behavior from `app_settings` (sync applied one of its
    /// keys) and follow it when it changed: a pull that brings all three keys
    /// tells the watcher of each, and the tray follows once.
    #[cfg(desktop)]
    pub async fn reload_tray_settings(&self) {
        let settings = self.read_tray_settings().await;
        if self.tray.replace(settings) {
            tray::apply(&self.app, settings.wants_tray());
            tray::sync_taskbar_to_visibility(&self.app);
        }
    }

    #[cfg(desktop)]
    async fn read_tray_settings(&self) -> TraySettings {
        let db = &self.app.state::<Core>().db;
        TraySettings {
            minimize_to_tray: settings::get_bool(db, "minimize_to_tray").await,
            close_to_tray: settings::get_bool(db, "close_to_tray").await,
            start_hidden: settings::get_bool(db, "start_hidden").await,
        }
    }

    #[cfg(desktop)]
    fn follow_tray_settings(&self, settings: TraySettings) {
        self.tray.store(settings);
        tray::apply(&self.app, settings.wants_tray());
        // Visible window stays in the taskbar; skip only while actually stashed.
        tray::sync_taskbar_to_visibility(&self.app);
    }

    /// The tray behavior the last run left in `app_settings`, taken over
    /// before the modules are set up.
    #[cfg(desktop)]
    pub(crate) async fn load_tray_settings(&self) {
        self.tray.store(self.read_tray_settings().await);
    }

    /// The modules that are on and have entries in the tray, each with its
    /// index: a module that is off is not in the tray.
    #[cfg(desktop)]
    pub(crate) fn tray_parts(&self) -> impl Iterator<Item = (usize, TrayPart)> + '_ {
        self.parts
            .iter()
            .enumerate()
            .filter(|(_, part)| self.module_enabled(part.id))
            .filter_map(|(index, part)| Some((index, part.tray?)))
    }

    #[cfg(desktop)]
    pub(crate) fn tray_part(&self, index: usize) -> Option<TrayPart> {
        self.parts.get(index)?.tray
    }

    /// The parts of the demo data, in the order of the product's list.
    pub(crate) fn demo_parts(&self) -> Vec<DemoPart> {
        self.parts.iter().filter_map(|part| part.demo).collect()
    }

    /// What a backup of the product takes besides the data file: the paths
    /// of the modules' parts, in the order of the product's list. Read by
    /// the driver of a product that has one (Space's `backup` service).
    pub fn backup_paths(&self) -> Vec<BackupPath> {
        self.parts
            .iter()
            .filter_map(|part| part.backup)
            .flat_map(|part| (part.paths)(&self.app))
            .collect()
    }

    /// A file the user picked, to read from: a path, or a `content://` URI
    /// on Android.
    pub fn open_source(&self, src: &str) -> Result<FileSource, AppError> {
        source::open(&self.app, src)
    }

    /// Let the pages of the app in `window` use the camera and the microphone.
    pub fn grant_media_access(&self, window: &tauri::WebviewWindow) -> Result<(), AppError> {
        crate::commands::media::grant_access(window).map_err(AppError::Other)
    }

    /// Run `Module::stop` of every module, in the order of the product's
    /// list, whether it is on or off. A module that fails does not keep the
    /// others from stopping.
    pub async fn stop_modules(&self) {
        for part in &self.parts {
            part.run_stop(&self.app).await;
        }
    }
}
