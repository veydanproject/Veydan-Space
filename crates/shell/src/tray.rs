// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! System-tray integration: the frame every product has — the icon, show,
//! hide, quit, the tooltip, the dot for what waits — around the entries the
//! product's modules supply through `Module::tray`.
//!
//! Two backends, one API (`apply` / `refresh`):
//!
//! - **Linux** uses [`ksni`] — a native StatusNotifierItem implementation.
//!   Unlike Tauri's bundled libappindicator backend (menu-only), ksni exposes
//!   separate `activate` (primary/left click) and context-menu (right click)
//!   handlers, which KDE Plasma honors. So on Linux left-click opens the app
//!   and right-click opens the context menu.
//! - **Windows / macOS** use Tauri's `tray-icon`, which already delivers
//!   left-click events (window toggle) and shows the menu on right-click.

use crate::module::{tray_label, TrayGroup, TrayItem, TrayLabels};
use crate::services::Shell;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};
use veydan_core::AppError;

/// Per profile under `--workdir`: every running instance owns its tray item.
fn tray_id(product: &str) -> String {
    let id = format!("veydan-{product}-tray");
    match crate::workdir::current() {
        Some(w) => format!("{id}-{}", w.tag),
        None => id,
    }
}

/// Tray behavior as the settings screen reads and writes it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraySettings {
    pub minimize_to_tray: bool,
    pub close_to_tray: bool,
    pub start_hidden: bool,
}

impl TraySettings {
    /// Any of the three needs the icon: it is the way back to a hidden window.
    pub(crate) fn wants_tray(&self) -> bool {
        self.minimize_to_tray || self.close_to_tray || self.start_hidden
    }
}

/// The tray's part of `Shell`. The settings are atomics for cheap access
/// from the window event handler; they mirror the `minimize_to_tray` /
/// `close_to_tray` / `start_hidden` keys in `app_settings`.
#[derive(Default)]
pub(crate) struct TrayState {
    minimize_to_tray: AtomicBool,
    close_to_tray: AtomicBool,
    start_hidden: AtomicBool,
    /// Supplied by the frontend via `tray_set_labels` so the i18n catalog is
    /// never duplicated in Rust. English defaults are used until the frontend
    /// hands over the active locale.
    labels: Mutex<TrayLabels>,
    /// What waits in each module that counts such things: a dot on the icon
    /// and a line in the tooltip for the sum.
    unread: Mutex<HashMap<&'static str, usize>>,
    #[cfg(not(target_os = "linux"))]
    icon: Mutex<Option<tauri::tray::TrayIcon>>,
}

impl TrayState {
    pub(crate) fn settings(&self) -> TraySettings {
        TraySettings {
            minimize_to_tray: self.minimize_to_tray.load(Ordering::Relaxed),
            close_to_tray: self.close_to_tray.load(Ordering::Relaxed),
            start_hidden: self.start_hidden.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn store(&self, settings: TraySettings) {
        self.minimize_to_tray
            .store(settings.minimize_to_tray, Ordering::Relaxed);
        self.close_to_tray
            .store(settings.close_to_tray, Ordering::Relaxed);
        self.start_hidden
            .store(settings.start_hidden, Ordering::Relaxed);
    }

    /// Store `settings`; whether they are not those stored already.
    pub(crate) fn replace(&self, settings: TraySettings) -> bool {
        let changed = self.settings() != settings;
        self.store(settings);
        changed
    }

    fn labels(&self) -> TrayLabels {
        self.labels.lock().unwrap().clone()
    }

    pub(crate) fn set_labels(&self, labels: TrayLabels) -> Result<(), AppError> {
        // A poisoned lock would otherwise panic every future call; surface it as an error.
        *self
            .labels
            .lock()
            .map_err(|e| AppError::other(e.to_string()))? = labels;
        Ok(())
    }

    /// Records the count of one module; true when the sum is to be shown
    /// again: it changed, or the module counts for the first time. A first
    /// count of none shows the sum too, because a launcher may keep the
    /// count on the icon of the app from the last run.
    pub(crate) fn set_unread(&self, module: &'static str, n: usize) -> bool {
        let mut unread = self.unread.lock().unwrap();
        let before: usize = unread.values().sum();
        let first = unread.insert(module, n).is_none();
        first || before != unread.values().sum::<usize>()
    }

    fn unread(&self) -> usize {
        self.unread.lock().unwrap().values().sum()
    }
}

fn unread(app: &AppHandle) -> usize {
    app.try_state::<Shell>()
        .map_or(0, |shell| shell.tray.unread())
}

/// The tooltip: the headline (with the profile name under `--workdir`), and
/// what waits.
fn tooltip_text(headline: &str, labels: &TrayLabels, waiting: usize) -> String {
    let mut text = headline.to_string();
    if waiting > 0 {
        let line = tray_label(labels, "unread", "{n} unread").replace("{n}", &waiting.to_string());
        text = format!("{text}\n{line}");
    }
    crate::workdir::caption(text)
}

/// An RGBA icon with a dot in its lower right corner: something waits.
fn with_dot(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut out = rgba.to_vec();
    let r = (width.min(height) as f32) * 0.22;
    let (cx, cy) = (width as f32 - r - 0.5, height as f32 - r - 0.5);
    for y in 0..height {
        for x in 0..width {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let i = ((y * width + x) * 4) as usize;
            if d <= r {
                out[i..i + 4].copy_from_slice(&[0xE5, 0x48, 0x4D, 0xFF]);
            } else if d <= r + 1.0 * (width as f32 / 32.0).max(1.0) {
                // A ring in the icon's background lets the dot stand apart.
                out[i + 3] = 0;
            }
        }
    }
    out
}

/// What the tray shows between two refreshes, gathered from the modules.
#[derive(Default, Clone)]
struct Content {
    labels: TrayLabels,
    /// The top entries of the modules, each with the index of its module, in
    /// the order of the product's list.
    items: Vec<(usize, TrayItem)>,
    /// First line of the tooltip.
    headline: String,
}

impl Content {
    fn tooltip(&self, waiting: usize) -> String {
        tooltip_text(&self.headline, &self.labels, waiting)
    }

    fn show_label(&self, product_name: &str) -> String {
        tray_label(&self.labels, "show", &format!("Show {product_name}"))
    }

    fn hide_label(&self) -> String {
        tray_label(&self.labels, "hide", "Hide window")
    }

    fn quit_label(&self) -> String {
        tray_label(&self.labels, "quit", "Quit")
    }
}

/// Ask every module of the product for its entries.
async fn load_content(app: &AppHandle) -> Content {
    let shell = app.state::<Shell>();
    let labels = shell.tray.labels();
    let mut items = Vec::new();
    let mut headline = None;
    for (index, part) in shell.tray_parts() {
        let entries = (part.items)(app.clone(), labels.clone()).await;
        items.extend(entries.into_iter().map(|item| (index, item)));
        if let (None, Some(tooltip)) = (&headline, part.tooltip) {
            headline = Some(tooltip(app.clone(), labels.clone()).await);
        }
    }
    Content {
        headline: headline.unwrap_or_else(|| shell.product().name.to_string()),
        labels,
        items,
    }
}

/// One line of the menu, top to bottom.
#[derive(Debug, PartialEq, Eq)]
enum Row<'a> {
    Show,
    Hide,
    Line,
    /// An entry of the module with this index.
    Item(usize, &'a TrayItem),
    Quit,
}

/// The menu: the frame's entries around the groups of the modules' ones.
fn rows(items: &[(usize, TrayItem)]) -> Vec<Row<'_>> {
    let mut rows = vec![Row::Show, Row::Hide];
    for group in [TrayGroup::Lists, TrayGroup::Sections, TrayGroup::Actions] {
        let mut of_group = items
            .iter()
            .filter(|(_, item)| item.group == group)
            .peekable();
        if of_group.peek().is_some() {
            rows.push(Row::Line);
            rows.extend(of_group.map(|(module, item)| Row::Item(*module, item)));
        }
    }
    rows.push(Row::Line);
    rows.push(Row::Quit);
    rows
}

/// A click on an entry of a module goes to that module.
fn clicked(app: &AppHandle, module: usize, id: &str) {
    let on_click = app
        .state::<Shell>()
        .tray_part(module)
        .map(|part| part.on_click);
    if let Some(on_click) = on_click {
        on_click(app, id);
    }
}

// ── Window helpers (marshalled to the main thread; GTK-safe) ────────────────
//
// Stashing to the tray uses hide()+skip_taskbar so the window leaves the
// taskbar only while hidden. A visible window stays in the taskbar even when
// minimize-to-tray is enabled (otherwise Alt-Tab makes it look "gone").
// Client-side decorations (decorations:false + custom titlebar) avoid the
// old KWin hide()/show() decoration remap bug on Wayland.

/// Restore taskbar entry, show and focus the main window.
fn restore_window(w: &tauri::WebviewWindow) {
    let _ = w.set_skip_taskbar(false);
    if w.is_minimized().unwrap_or(false) {
        let _ = w.unminimize();
    }
    let _ = w.show();
    let _ = w.set_focus();
}

/// Hide to tray and drop the taskbar entry.
fn stash_window(w: &tauri::WebviewWindow) {
    let _ = w.set_skip_taskbar(true);
    let _ = w.hide();
}

/// Show the main window and restore its taskbar entry. Safe from any thread.
pub(crate) fn show_main_window(app: &AppHandle) {
    let a = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = a.get_webview_window("main") {
            restore_window(&w);
        }
    });
}

/// What waits, on the icon of the app.
pub(crate) fn show_badge(app: &AppHandle) {
    let n = unread(app);
    let Some(w) = app.get_webview_window("main") else {
        return;
    };
    // macOS: the Dock; Linux: launchers that read the Unity count (KDE, Ubuntu).
    #[cfg(not(windows))]
    let _ = w.set_badge_count((n > 0).then_some(n as i64));
    // Windows has no count on the taskbar button, only a small overlay icon.
    #[cfg(windows)]
    let _ = w.set_overlay_icon((n > 0).then(dot_image));
}

/// A red dot for the taskbar button.
#[cfg_attr(not(windows), allow(dead_code))]
fn dot_image() -> tauri::image::Image<'static> {
    const SIZE: u32 = 16;
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    let c = SIZE as f32 / 2.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
            if d <= c - 0.5 {
                let i = ((y * SIZE + x) * 4) as usize;
                rgba[i..i + 4].copy_from_slice(&[0xE5, 0x48, 0x4D, 0xFF]);
            }
        }
    }
    tauri::image::Image::new_owned(rgba, SIZE, SIZE)
}

/// Stash the main window to the tray — it fully leaves the taskbar. Safe
/// from any thread.
pub(crate) fn hide_main_window(app: &AppHandle) {
    let a = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = a.get_webview_window("main") {
            stash_window(&w);
        }
    });
}

/// Primary click on the tray icon (both platforms). Toggle:
/// - hidden → show + focus;
/// - visible but not focused → raise + focus (it's behind other windows);
/// - visible and focused → hide to the tray.
fn win_primary(app: &AppHandle) {
    let a = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = a.get_webview_window("main") {
            let visible = w.is_visible().unwrap_or(false);
            if !visible {
                restore_window(&w);
            } else if w.is_focused().unwrap_or(false) {
                stash_window(&w);
            } else {
                let _ = w.set_focus();
            }
        }
    });
}

// ════════════════════════════════════════════════════════════════════════════
// Linux backend — ksni (StatusNotifierItem)
// ════════════════════════════════════════════════════════════════════════════
#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use ksni::menu::{MenuItem, StandardItem, SubMenu};
    use ksni::{Category, Handle, Icon, Status, ToolTip, Tray, TrayMethods};

    static TRAY: Mutex<Option<Handle<ShellTray>>> = Mutex::new(None);

    pub struct ShellTray {
        app: AppHandle,
        product: crate::Product,
        content: Content,
        icon: Vec<Icon>,
        /// The same with a dot: something waits.
        icon_dot: Vec<Icon>,
        visible: bool,
    }

    fn item(
        label: String,
        activate: impl Fn(&mut ShellTray) + Send + 'static,
    ) -> MenuItem<ShellTray> {
        StandardItem {
            label,
            activate: Box::new(activate),
            ..Default::default()
        }
        .into()
    }

    /// An entry of a module, with what is under it.
    fn module_item(module: usize, entry: &TrayItem) -> MenuItem<ShellTray> {
        if entry.separator {
            return MenuItem::Separator;
        }
        if !entry.children.is_empty() {
            return SubMenu {
                label: entry.label.clone(),
                enabled: entry.enabled,
                submenu: entry
                    .children
                    .iter()
                    .map(|child| module_item(module, child))
                    .collect(),
                ..Default::default()
            }
            .into();
        }
        let id = entry.id.clone();
        StandardItem {
            label: entry.label.clone(),
            enabled: entry.enabled,
            activate: Box::new(move |t: &mut ShellTray| clicked(&t.app, module, &id)),
            ..Default::default()
        }
        .into()
    }

    impl Tray for ShellTray {
        fn id(&self) -> String {
            tray_id(self.product.id)
        }

        fn title(&self) -> String {
            crate::workdir::caption(self.product.name.into())
        }

        fn category(&self) -> Category {
            Category::ApplicationStatus
        }

        fn status(&self) -> Status {
            if !self.visible {
                Status::Passive
            } else if unread(&self.app) > 0 {
                Status::NeedsAttention
            } else {
                Status::Active
            }
        }

        fn icon_pixmap(&self) -> Vec<Icon> {
            if unread(&self.app) > 0 {
                self.icon_dot.clone()
            } else {
                self.icon.clone()
            }
        }

        fn attention_icon_pixmap(&self) -> Vec<Icon> {
            self.icon_dot.clone()
        }

        fn tool_tip(&self) -> ToolTip {
            ToolTip {
                title: self.content.tooltip(unread(&self.app)),
                description: String::new(),
                icon_name: String::new(),
                icon_pixmap: Vec::new(),
            }
        }

        /// Left / primary click → focus-aware toggle (show / raise / hide).
        fn activate(&mut self, _x: i32, _y: i32) {
            win_primary(&self.app);
        }

        /// Right click → this menu (rendered by the SNI host as the context menu).
        fn menu(&self) -> Vec<MenuItem<Self>> {
            rows(&self.content.items)
                .into_iter()
                .map(|row| match row {
                    Row::Show => item(self.content.show_label(self.product.name), |t| {
                        show_main_window(&t.app)
                    }),
                    Row::Hide => item(self.content.hide_label(), |t| hide_main_window(&t.app)),
                    Row::Line => MenuItem::Separator,
                    Row::Item(module, entry) => module_item(module, entry),
                    Row::Quit => item(self.content.quit_label(), |t| t.app.exit(0)),
                })
                .collect()
        }
    }

    /// The app's icon as SNI wants it (ARGB32, network byte order), plain and
    /// with the dot.
    fn build_icons(app: &AppHandle) -> (Vec<Icon>, Vec<Icon>) {
        let Some(img) = app.default_window_icon() else {
            return (Vec::new(), Vec::new());
        };
        let argb = |rgba: &[u8]| {
            let mut data = Vec::with_capacity(rgba.len());
            for px in rgba.as_chunks::<4>().0 {
                data.push(px[3]); // A
                data.push(px[0]); // R
                data.push(px[1]); // G
                data.push(px[2]); // B
            }
            vec![Icon {
                width: img.width() as i32,
                height: img.height() as i32,
                data,
            }]
        };
        let dot = with_dot(img.rgba(), img.width(), img.height());
        (argb(img.rgba()), argb(&dot))
    }

    fn current_handle() -> Option<Handle<ShellTray>> {
        TRAY.lock().unwrap().clone()
    }

    pub fn apply(app: &AppHandle, want: bool) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if !want {
                if let Some(h) = current_handle() {
                    let _ = h.update(|t| t.visible = false).await;
                }
                return;
            }
            if let Some(h) = current_handle() {
                let content = load_content(&app).await;
                let _ = h
                    .update(move |t| {
                        t.visible = true;
                        t.content = content;
                    })
                    .await;
                return;
            }
            // First activation: build and register the SNI service.
            let icons = build_icons(&app);
            let tray = ShellTray {
                app: app.clone(),
                product: *app.state::<Shell>().product(),
                content: load_content(&app).await,
                icon: icons.0,
                icon_dot: icons.1,
                visible: true,
            };
            match tray.spawn().await {
                Ok(handle) => *TRAY.lock().unwrap() = Some(handle),
                Err(e) => {
                    eprintln!("tray: ksni spawn failed: {e}");
                    // Hide-on-close would stash the window with no way to restore it
                    let state = &app.state::<Shell>().tray;
                    state.close_to_tray.store(false, Ordering::Relaxed);
                    state.minimize_to_tray.store(false, Ordering::Relaxed);
                }
            }
        });
    }

    pub fn refresh(app: &AppHandle) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Some(h) = current_handle() {
                let content = load_content(&app).await;
                let _ = h.update(move |t| t.content = content).await;
            }
        });
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Windows / macOS backend — Tauri tray-icon
// ════════════════════════════════════════════════════════════════════════════
#[cfg(not(target_os = "linux"))]
mod imp {
    use super::*;
    use tauri::menu::{
        IsMenuItem, Menu, MenuBuilder, MenuItem, MenuItemBuilder, PredefinedMenuItem, Submenu,
        SubmenuBuilder,
    };
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use tauri::Wry;

    /// An entry of a module as the menu holds it.
    enum Built {
        Entry(MenuItem<Wry>),
        Submenu(Submenu<Wry>),
        Line(PredefinedMenuItem<Wry>),
    }

    impl Built {
        fn as_item(&self) -> &dyn IsMenuItem<Wry> {
            match self {
                Built::Entry(item) => item,
                Built::Submenu(item) => item,
                Built::Line(item) => item,
            }
        }
    }

    /// The id of a module's entry in the menu: the index of the module, a
    /// colon, the id the module gave. The frame's own ids have no colon.
    fn menu_id(module: usize, id: &str) -> String {
        format!("{module}:{id}")
    }

    fn module_item(app: &AppHandle, module: usize, entry: &TrayItem) -> tauri::Result<Built> {
        if entry.separator {
            return Ok(Built::Line(PredefinedMenuItem::separator(app)?));
        }
        if !entry.children.is_empty() {
            let mut submenu = SubmenuBuilder::new(app, &entry.label).enabled(entry.enabled);
            for child in &entry.children {
                submenu = submenu.item(module_item(app, module, child)?.as_item());
            }
            return Ok(Built::Submenu(submenu.build()?));
        }
        Ok(Built::Entry(
            MenuItemBuilder::with_id(menu_id(module, &entry.id), &entry.label)
                .enabled(entry.enabled)
                .build(app)?,
        ))
    }

    fn build_menu(app: &AppHandle, content: &Content) -> tauri::Result<Menu<Wry>> {
        let product_name = app.state::<Shell>().product().name;
        let frame = |id: &str, label: String| MenuItemBuilder::with_id(id, label).build(app);
        let mut menu = MenuBuilder::new(app);
        for row in rows(&content.items) {
            menu = match row {
                Row::Show => menu.item(&frame("show", content.show_label(product_name))?),
                Row::Hide => menu.item(&frame("hide", content.hide_label())?),
                Row::Line => menu.separator(),
                Row::Item(module, entry) => menu.item(module_item(app, module, entry)?.as_item()),
                Row::Quit => menu.item(&frame("quit", content.quit_label())?),
            };
        }
        menu.build()
    }

    /// A click in the menu: the frame's own entries, or an entry of a module.
    fn dispatch(app: &AppHandle, id: &str) {
        match id {
            "show" => show_main_window(app),
            "hide" => hide_main_window(app),
            "quit" => app.exit(0),
            other => {
                let Some((module, id)) = other.split_once(':') else {
                    return;
                };
                if let Ok(module) = module.parse() {
                    clicked(app, module, id);
                }
            }
        }
    }

    fn show_tray(app: &AppHandle) -> tauri::Result<()> {
        let shell = app.state::<Shell>();
        {
            let guard = shell.tray.icon.lock().unwrap();
            if let Some(tray) = guard.as_ref() {
                let _ = tray.set_visible(true);
                drop(guard);
                refresh_sync(app);
                return Ok(());
            }
        }

        let content = tauri::async_runtime::block_on(load_content(app));
        let menu = build_menu(app, &content)?;
        let tooltip = content.tooltip(unread(app));

        let mut builder = TrayIconBuilder::with_id(tray_id(shell.product().id))
            .tooltip(&tooltip)
            .menu(&menu)
            .show_menu_on_left_click(false)
            .on_menu_event(|app, event| dispatch(app, event.id.as_ref()))
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    win_primary(tray.app_handle());
                }
            });
        if let Some(icon) = current_icon(app) {
            builder = builder.icon(icon);
        }
        let tray = builder.build(app)?;
        *shell.tray.icon.lock().unwrap() = Some(tray);
        Ok(())
    }

    /// The app's icon, with the dot while something waits.
    fn current_icon(app: &AppHandle) -> Option<tauri::image::Image<'static>> {
        let img = app.default_window_icon()?;
        if unread(app) == 0 {
            return Some(img.clone().to_owned());
        }
        Some(tauri::image::Image::new_owned(
            with_dot(img.rgba(), img.width(), img.height()),
            img.width(),
            img.height(),
        ))
    }

    fn hide_tray(app: &AppHandle) {
        let shell = app.state::<Shell>();
        let guard = shell.tray.icon.lock().unwrap();
        if let Some(tray) = guard.as_ref() {
            let _ = tray.set_visible(false);
        }
    }

    fn refresh_sync(app: &AppHandle) {
        let shell = app.state::<Shell>();
        let guard = shell.tray.icon.lock().unwrap();
        let Some(tray) = guard.as_ref() else { return };
        let content = tauri::async_runtime::block_on(load_content(app));
        if let Ok(menu) = build_menu(app, &content) {
            let _ = tray.set_menu(Some(menu));
            let tooltip = content.tooltip(unread(app));
            let _ = tray.set_tooltip(Some(&tooltip));
        }
        let _ = tray.set_icon(current_icon(app));
    }

    pub fn apply(app: &AppHandle, want: bool) {
        let a = app.clone();
        let _ = app.run_on_main_thread(move || {
            if want {
                if let Err(e) = show_tray(&a) {
                    eprintln!("tray: failed to create icon: {e}");
                }
            } else {
                hide_tray(&a);
            }
        });
    }

    pub fn refresh(app: &AppHandle) {
        let a = app.clone();
        let _ = app.run_on_main_thread(move || refresh_sync(&a));
    }
}

/// Show or hide the tray icon, from any thread.
pub(crate) fn apply(app: &AppHandle, want: bool) {
    imp::apply(app, want);
}

/// Rebuild the tray menu/tooltip against the current state, from any thread.
pub(crate) fn refresh(app: &AppHandle) {
    imp::refresh(app);
}

/// Align skip_taskbar with current visibility (visible → in taskbar).
/// Call after tray-setting changes so a still-open window is not orphaned.
pub(crate) fn sync_taskbar_to_visibility(app: &AppHandle) {
    let a = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = a.get_webview_window("main") {
            let skip = !w.is_visible().unwrap_or(true);
            let _ = w.set_skip_taskbar(skip);
        }
    });
}

/// Hide-on-close / minimize-to-tray for the main window. skip_taskbar only
/// while stashed. On Linux, OS minimize is handled by the custom titlebar
/// button (`window_minimize`); WindowEvent::Resized / is_minimized() are
/// unreliable on GTK/Wayland.
pub(crate) fn watch_main_window(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let handle = app.clone();
    win.on_window_event(move |event| {
        let settings = handle.state::<Shell>().tray.settings();
        match event {
            #[cfg(not(target_os = "linux"))]
            tauri::WindowEvent::Resized(_) => {
                if settings.minimize_to_tray {
                    if let Some(w) = handle.get_webview_window("main") {
                        if w.is_minimized().unwrap_or(false) {
                            let _ = w.unminimize();
                            let _ = w.set_skip_taskbar(true);
                            let _ = w.hide();
                        }
                    }
                }
            }
            tauri::WindowEvent::CloseRequested { api, .. } if settings.close_to_tray => {
                api.prevent_close();
                hide_main_window(&handle);
            }
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dot_sits_in_the_lower_right_corner() {
        let (w, h) = (32u32, 32u32);
        let icon = vec![0x10u8; (w * h * 4) as usize];
        let out = with_dot(&icon, w, h);
        let px = |x: u32, y: u32| &out[((y * w + x) * 4) as usize..((y * w + x) * 4 + 4) as usize];
        assert_eq!(px(w - 4, h - 4), &[0xE5, 0x48, 0x4D, 0xFF], "the dot");
        assert_eq!(px(2, 2), &[0x10; 4], "the icon elsewhere is as it was");
    }

    #[test]
    fn the_tooltip_says_what_waits() {
        let labels = TrayLabels::new();
        assert_eq!(
            tooltip_text("Veydan Space — 1 running", &labels, 0),
            "Veydan Space — 1 running"
        );
        assert_eq!(
            tooltip_text("Veydan Space — 1 running", &labels, 3),
            "Veydan Space — 1 running\n3 unread"
        );
        let labels = TrayLabels::from([("unread".to_string(), "Непрочитано: {n}".to_string())]);
        assert!(tooltip_text("Veydan Space", &labels, 2).ends_with("Непрочитано: 2"));
    }

    #[test]
    fn the_sum_of_what_waits_is_what_counts() {
        let state = TrayState::default();
        assert!(state.set_unread("messenger", 2));
        assert!(!state.set_unread("messenger", 2), "the same count again");
        assert!(state.set_unread("notes", 1));
        assert_eq!(state.unread(), 3);
        assert!(state.set_unread("messenger", 0));
        assert_eq!(state.unread(), 1);
    }

    #[test]
    fn the_first_count_of_a_module_is_shown_even_when_nothing_waits() {
        let state = TrayState::default();
        assert!(
            state.set_unread("messenger", 0),
            "the icon may hold the count of the last run"
        );
        assert!(!state.set_unread("messenger", 0), "nothing new");
        assert!(state.set_unread("notes", 0), "another module's first count");
        assert_eq!(state.unread(), 0);
    }

    #[test]
    fn the_settings_are_read_back_as_stored() {
        let state = TrayState::default();
        assert_eq!(state.settings(), TraySettings::default());
        assert!(!state.settings().wants_tray());
        let settings = TraySettings {
            minimize_to_tray: false,
            close_to_tray: true,
            start_hidden: false,
        };
        state.store(settings);
        assert_eq!(state.settings(), settings);
        assert!(settings.wants_tray());
    }

    /// Sync tells the watcher of each tray key it applied; a pull that brings
    /// all three finds the settings changed once.
    #[test]
    fn settings_stored_already_are_no_change() {
        let state = TrayState::default();
        let settings = TraySettings {
            minimize_to_tray: true,
            close_to_tray: true,
            start_hidden: false,
        };
        assert!(state.replace(settings));
        assert!(!state.replace(settings), "the same settings again");
        assert_eq!(state.settings(), settings);
        assert!(state.replace(TraySettings::default()));
    }

    fn entry(group: TrayGroup, id: &str) -> TrayItem {
        TrayItem::entry(group, id, id)
    }

    #[test]
    fn the_frame_stands_around_the_groups_of_the_modules() {
        // Two modules; the second has an action that goes before the first's.
        let items = vec![
            (1, entry(TrayGroup::Actions, "pwgen")),
            (2, entry(TrayGroup::Lists, "running")),
            (2, entry(TrayGroup::Sections, "nav:/")),
            (3, entry(TrayGroup::Sections, "nav:/notes")),
            (3, entry(TrayGroup::Actions, "quick_capture")),
        ];
        let ids: Vec<String> = rows(&items)
            .into_iter()
            .map(|row| match row {
                Row::Show => "show".into(),
                Row::Hide => "hide".into(),
                Row::Line => "-".into(),
                Row::Item(module, item) => format!("{module}:{}", item.id),
                Row::Quit => "quit".into(),
            })
            .collect();
        assert_eq!(
            ids,
            [
                "show",
                "hide",
                "-",
                "2:running",
                "-",
                "2:nav:/",
                "3:nav:/notes",
                "-",
                "1:pwgen",
                "3:quick_capture",
                "-",
                "quit"
            ]
        );
    }

    #[test]
    fn a_product_without_tray_entries_has_the_frame_alone() {
        assert_eq!(rows(&[]), vec![Row::Show, Row::Hide, Row::Line, Row::Quit]);
    }

    #[test]
    fn the_frame_labels_name_the_product_until_the_ui_names_them() {
        let mut content = Content::default();
        assert_eq!(content.show_label("Veydan Notes"), "Show Veydan Notes");
        assert_eq!(content.hide_label(), "Hide window");
        assert_eq!(content.quit_label(), "Quit");
        content.labels.insert("show".into(), "Показать".into());
        assert_eq!(content.show_label("Veydan Notes"), "Показать");
    }

    #[test]
    fn the_tray_id_carries_the_product() {
        assert_eq!(tray_id("space"), "veydan-space-tray");
    }
}
