// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The switches of the modules (platform-spec 12): which modules of the
//! product the user keeps on, on this device.
//!
//! A module that is off is hidden by the UI and its background work is
//! stopped (`Module::stop`); its data stays where it is and keeps syncing —
//! the registry of sync does not look at the switches. The setting is local:
//! `modules_enabled` in `app_settings`, which sync does not carry. A module
//! the setting does not name is on, so an install made before the switches,
//! and a module a later version adds, start with everything on. At least one
//! module with a switch of its own stays on.

use crate::module::Switch;
use crate::services::{Part, Shell};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use tauri::{Emitter, Manager};
use veydan_core::{settings, AppError, CmdResult, Core};

/// The local setting: `{"<module>": true | false, …}` over the modules with
/// a switch of their own.
pub(crate) const KEY: &str = "modules_enabled";
/// Set by the first start over a new data file of a product with more than
/// one switch, and taken away once the user said which modules to use: until
/// then the UI asks.
pub(crate) const FIRST_RUN_KEY: &str = "modules_first_run";
/// Emitted with [`ModulesView`] whenever the set of modules that are on changes.
pub const EVENT_MODULES_CHANGED: &str = "modules://changed";

/// The refusal to switch the last module off.
const LAST: &str = "modules_last";

/// A module with a switch of its own, as the UI lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModuleState {
    pub id: &'static str,
    pub enabled: bool,
}

/// The answer of `modules_list` and the payload of [`EVENT_MODULES_CHANGED`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModulesView {
    /// The modules with a switch of their own, in the order of the product.
    pub modules: Vec<ModuleState>,
    /// The user has not yet said which modules to use on a new data file.
    pub first_run: bool,
}

/// What the shell keeps of the switches while the app runs.
#[derive(Default)]
pub(crate) struct Switches {
    /// The modules with a switch of their own that are off.
    off: RwLock<BTreeSet<&'static str>>,
    first_run: AtomicBool,
    /// The app is on its way out: what `stop` does then may differ from what
    /// it does when the user switches the module off.
    exiting: AtomicBool,
    /// One change at a time: the hooks of a change end before another begins.
    changing: tokio::sync::Mutex<()>,
}

impl Switches {
    fn off(&self) -> BTreeSet<&'static str> {
        self.off.read().map(|off| off.clone()).unwrap_or_default()
    }

    fn set_off(&self, off: BTreeSet<&'static str>) {
        if let Ok(mut held) = self.off.write() {
            *held = off;
        }
    }
}

/// The ids of the modules with a switch of their own, in the order of the list.
fn own(parts: &[Part]) -> impl Iterator<Item = &'static str> + '_ {
    parts
        .iter()
        .filter(|part| part.switch == Switch::Own)
        .map(|part| part.id)
}

/// Whether the module `id` is on when the modules in `off` are off. A module
/// that follows another is on with it; one that is not in the product is
/// not on.
fn is_on(parts: &[Part], off: &BTreeSet<&'static str>, id: &str) -> bool {
    let mut id = id;
    // A chain of followers ends: each step names another module of the list.
    for _ in 0..=parts.len() {
        let Some(part) = parts.iter().find(|part| part.id == id) else {
            return false;
        };
        match part.switch {
            Switch::Own => return !off.contains(part.id),
            Switch::Always => return true,
            Switch::Follows(leader) => id = leader,
        }
    }
    false
}

/// The modules that are off by the stored setting: those it says `false`
/// of. Ids that are not modules with a switch of their own are left out;
/// a setting that leaves none on is not followed.
fn parse(parts: &[Part], stored: Option<BTreeMap<String, bool>>) -> BTreeSet<&'static str> {
    let stored = stored.unwrap_or_default();
    let off: BTreeSet<&'static str> = own(parts)
        .filter(|id| stored.get(*id) == Some(&false))
        .collect();
    if own(parts).all(|id| off.contains(id)) {
        return BTreeSet::new();
    }
    off
}

/// The setting as it is stored: every module with a switch of its own.
fn stored(parts: &[Part], off: &BTreeSet<&'static str>) -> BTreeMap<&'static str, bool> {
    own(parts).map(|id| (id, !off.contains(id))).collect()
}

/// What a change of the switches does: the modules whose hooks run, as
/// indexes into the list, in its order.
#[derive(Debug, PartialEq, Eq)]
struct Change {
    starting: Vec<usize>,
    stopping: Vec<usize>,
}

/// From the modules in `before` being off to those in `after`: each module
/// that goes on or off, with the modules that follow it. Refused when no
/// module with a switch of its own would stay on.
fn plan(
    parts: &[Part],
    before: &BTreeSet<&'static str>,
    after: &BTreeSet<&'static str>,
) -> Result<Change, AppError> {
    if own(parts).all(|id| after.contains(id)) {
        return Err(AppError::Other(LAST.into()));
    }
    let mut change = Change {
        starting: Vec::new(),
        stopping: Vec::new(),
    };
    for (index, part) in parts.iter().enumerate() {
        match (is_on(parts, before, part.id), is_on(parts, after, part.id)) {
            (false, true) => change.starting.push(index),
            (true, false) => change.stopping.push(index),
            _ => {}
        }
    }
    Ok(change)
}

/// The modules off after a change from `before` to `after` in which the
/// modules `failed` did not start: each module switched on goes back off
/// when it, or a module that follows it, is among them.
fn undo_failed(
    parts: &[Part],
    before: &BTreeSet<&'static str>,
    after: &BTreeSet<&'static str>,
    failed: &[&str],
) -> BTreeSet<&'static str> {
    let mut back = after.clone();
    for id in own(parts).filter(|id| before.contains(id) && !after.contains(id)) {
        let of_it = |module: &&str| {
            *module == id
                || parts
                    .iter()
                    .any(|part| part.id == *module && part.switch == Switch::Follows(id))
        };
        if failed.iter().any(of_it) {
            back.insert(id);
        }
    }
    back
}

/// The id as the product lists it, if it is a module with a switch of its own.
fn own_id(parts: &[Part], id: &str) -> Result<&'static str, AppError> {
    own(parts)
        .find(|own| *own == id)
        .ok_or_else(|| AppError::Other(format!("modules_unknown: {id}")))
}

impl Part {
    /// `Module::start`; a failure is the caller's to tell.
    async fn run_start(&self, app: &tauri::AppHandle) -> Result<(), AppError> {
        match self.start {
            Some(start) => start(app.clone()).await,
            None => Ok(()),
        }
    }

    /// `Module::stop`; a failure is told here and stops nothing else.
    pub(crate) async fn run_stop(&self, app: &tauri::AppHandle) {
        let Some(stop) = self.stop else { return };
        if let Err(e) = stop(app.clone()).await {
            eprintln!("{}: stop failed: {e}", self.id);
        }
    }
}

impl Shell {
    /// Whether the module `id` is on: switched on by the user, following a
    /// module that is, or never off. A module the product does not have is
    /// not on.
    pub fn module_enabled(&self, id: &str) -> bool {
        is_on(&self.parts, &self.switches.off(), id)
    }

    /// Switch the module `id` on or off, as the Modules section of the
    /// settings does: its `start` or `stop` runs, with those of the modules
    /// that follow it. Refused for a module without a switch of its own and
    /// for the last module that is on. A module whose start fails stays off,
    /// and the failure is the answer.
    pub async fn set_module_enabled(&self, id: &str, on: bool) -> Result<(), AppError> {
        let _changing = self.switches.changing.lock().await;
        let id = own_id(&self.parts, id)?;
        let mut off = self.switches.off();
        if on {
            off.remove(id);
        } else {
            off.insert(id);
        }
        self.switch_to(off).await
    }

    /// The first start's choice: the modules named are on, the others off,
    /// and the UI asks no more.
    pub(crate) async fn choose_modules(&self, on: &[String]) -> Result<(), AppError> {
        let _changing = self.switches.changing.lock().await;
        for id in on {
            own_id(&self.parts, id)?;
        }
        let off = own(&self.parts)
            .filter(|id| !on.iter().any(|on| on == id))
            .collect();
        self.switch_to(off).await?;
        settings::delete(&self.app.state::<Core>().db, FIRST_RUN_KEY).await?;
        self.switches.first_run.store(false, Ordering::SeqCst);
        self.changed();
        Ok(())
    }

    /// Make the modules in `off` the ones that are off. The setting is kept
    /// first, so that a hook already sees the new state; a module that fails
    /// to start goes back off.
    async fn switch_to(&self, off: BTreeSet<&'static str>) -> Result<(), AppError> {
        let before = self.switches.off();
        let change = plan(&self.parts, &before, &off)?;
        if change.starting.is_empty() && change.stopping.is_empty() {
            return Ok(());
        }
        self.keep(off.clone()).await?;
        for index in &change.stopping {
            self.parts[*index].run_stop(&self.app).await;
        }
        let mut failed: Vec<(&'static str, AppError)> = Vec::new();
        for index in &change.starting {
            let part = &self.parts[*index];
            if let Err(e) = part.run_start(&self.app).await {
                eprintln!("{}: start failed: {e}", part.id);
                failed.push((part.id, e));
            }
        }
        let failed_ids: Vec<&'static str> = failed.iter().map(|(id, _)| *id).collect();
        let Some((_, first)) = failed.into_iter().next() else {
            self.changed();
            return Ok(());
        };
        // A module switched on whose start, or a follower's, failed goes back
        // off, and what of it started stops again — unless nothing would be
        // left on: then it stays on, as after a start of the app that failed.
        let back = undo_failed(&self.parts, &before, &off, &failed_ids);
        if let Ok(undo) = plan(&self.parts, &off, &back) {
            self.keep(back).await?;
            for index in undo.stopping {
                self.parts[index].run_stop(&self.app).await;
            }
        }
        self.changed();
        Err(first)
    }

    /// Store the modules that are off, here and in the setting.
    async fn keep(&self, off: BTreeSet<&'static str>) -> Result<(), AppError> {
        let value = stored(&self.parts, &off);
        settings::set_json(&self.app.state::<Core>().db, KEY, &value).await?;
        self.switches.set_off(off);
        Ok(())
    }

    /// The UI and the tray follow.
    fn changed(&self) {
        let _ = self.app.emit(EVENT_MODULES_CHANGED, self.modules());
        #[cfg(desktop)]
        self.tray_refresh();
    }

    /// The modules with a switch of their own and whether each is on.
    pub fn modules(&self) -> ModulesView {
        let off = self.switches.off();
        ModulesView {
            modules: own(&self.parts)
                .map(|id| ModuleState {
                    id,
                    enabled: !off.contains(id),
                })
                .collect(),
            first_run: self.switches.first_run.load(Ordering::SeqCst),
        }
    }

    /// Take over the switches the last run left, before the modules are set
    /// up. Over a new data file (`fresh`) of a product with more than one
    /// switch the UI is to ask which modules to use.
    pub(crate) async fn load_switches(&self, fresh: bool) {
        let db = &self.app.state::<Core>().db;
        let choice = own(&self.parts).count() > 1;
        if fresh && choice {
            if let Err(e) = settings::set(db, FIRST_RUN_KEY, "1").await {
                eprintln!("modules: the first start was not noted: {e}");
            }
        }
        let first_run = choice && settings::get(db, FIRST_RUN_KEY).await.is_some();
        self.switches.first_run.store(first_run, Ordering::SeqCst);
        let stored = settings::get_json(db, KEY).await;
        self.switches.set_off(parse(&self.parts, stored));
    }

    /// Run `Module::start` of every module that is on, in the order of the
    /// list, once the app is set up. A module that fails does not keep the
    /// others from starting.
    pub(crate) async fn start_modules(&self) {
        for part in &self.parts {
            if !self.module_enabled(part.id) {
                continue;
            }
            if let Err(e) = part.run_start(&self.app).await {
                eprintln!("{}: start failed: {e}", part.id);
            }
        }
    }

    /// The app is on its way out: `stop` then stops what a module that is
    /// switched off may leave running (the browsers it started).
    pub fn exiting(&self) -> bool {
        self.switches.exiting.load(Ordering::SeqCst)
    }

    /// The app is about to go: on exit, and before a restore of a backup
    /// stops the modules and swaps their data under them. Set before
    /// `stop_modules`, so `stop` stops all a module left running.
    #[cfg(desktop)]
    pub fn begin_exit(&self) {
        self.switches.exiting.store(true, Ordering::SeqCst);
    }
}

/// The modules with a switch of their own and whether each is on.
#[tauri::command]
pub fn modules_list(shell: tauri::State<'_, Shell>) -> ModulesView {
    shell.modules()
}

/// Switch a module on or off; `modules_last` when it is the last one on.
#[tauri::command]
pub async fn modules_set_enabled(
    id: String,
    enabled: bool,
    shell: tauri::State<'_, Shell>,
) -> CmdResult<ModulesView> {
    shell.set_module_enabled(&id, enabled).await?;
    Ok(shell.modules())
}

/// The answer to the first start's question: the modules to keep on.
#[tauri::command]
pub async fn modules_choose(
    enabled: Vec<String>,
    shell: tauri::State<'_, Shell>,
) -> CmdResult<ModulesView> {
    shell.choose_modules(&enabled).await?;
    Ok(shell.modules())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(id: &'static str, switch: Switch) -> Part {
        let module = crate::Module {
            switch,
            ..crate::module! { id: id, setup: |_app| Ok(()), commands: [] }
        };
        Part::of(&module)
    }

    /// The modules of Space, in its order.
    fn space() -> Vec<Part> {
        vec![
            part("shell", Switch::Always),
            part("pass", Switch::Own),
            part("browser", Switch::Own),
            part("ssh", Switch::Own),
            part("notes", Switch::Own),
            part("capture", Switch::Follows("notes")),
            part("backup", Switch::Always),
            part("messenger", Switch::Own),
        ]
    }

    fn set(ids: &[&'static str]) -> BTreeSet<&'static str> {
        ids.iter().copied().collect()
    }

    #[test]
    fn a_module_is_on_by_its_switch_by_its_leader_or_always() {
        let parts = space();
        let off = set(&["notes", "messenger"]);
        assert!(is_on(&parts, &off, "pass"));
        assert!(!is_on(&parts, &off, "notes"));
        assert!(!is_on(&parts, &off, "capture"));
        assert!(is_on(&parts, &BTreeSet::new(), "capture"));
        assert!(is_on(&parts, &off, "backup"));
        assert!(is_on(
            &parts,
            &set(&["pass", "browser", "ssh", "notes", "messenger"]),
            "shell"
        ));
        assert!(!is_on(&parts, &off, "nothing"));
        // A module that follows one the product lacks is not on.
        let lonely = vec![part("capture", Switch::Follows("notes"))];
        assert!(!is_on(&lonely, &BTreeSet::new(), "capture"));
    }

    /// The setting as `get_json` reads it.
    fn json(text: &str) -> Option<BTreeMap<String, bool>> {
        serde_json::from_str(text).ok()
    }

    #[test]
    fn the_setting_names_every_own_switch_and_a_missing_one_is_on() {
        let parts = space();
        assert_eq!(parse(&parts, None), BTreeSet::new());
        assert_eq!(parse(&parts, json("not json")), BTreeSet::new());
        // An install before the switches, and a module a later version adds.
        assert_eq!(parse(&parts, json(r#"{"pass":false}"#)), set(&["pass"]));
        // Unknown ids and modules without a switch of their own are not followed.
        assert_eq!(
            parse(
                &parts,
                json(r#"{"capture":false,"backup":false,"gone":false,"ssh":true}"#)
            ),
            BTreeSet::new()
        );
        // A setting that leaves nothing on leaves everything on.
        let none = r#"{"pass":false,"browser":false,"ssh":false,"notes":false,"messenger":false}"#;
        assert_eq!(parse(&parts, json(none)), BTreeSet::new());

        let off = set(&["browser", "messenger"]);
        let value = serde_json::to_string(&stored(&parts, &off)).unwrap();
        assert_eq!(
            value,
            r#"{"browser":false,"messenger":false,"notes":true,"pass":true,"ssh":true}"#
        );
        assert_eq!(parse(&parts, json(&value)), off);
    }

    #[test]
    fn a_change_runs_the_hooks_of_the_module_and_of_those_that_follow_it() {
        let parts = space();
        let ids =
            |indexes: &[usize]| -> Vec<&str> { indexes.iter().map(|i| parts[*i].id).collect() };
        let change = plan(&parts, &BTreeSet::new(), &set(&["notes"])).unwrap();
        assert_eq!(ids(&change.stopping), ["notes", "capture"]);
        assert!(change.starting.is_empty());
        let change = plan(&parts, &set(&["notes", "ssh"]), &set(&["messenger"])).unwrap();
        assert_eq!(ids(&change.starting), ["ssh", "notes", "capture"]);
        assert_eq!(ids(&change.stopping), ["messenger"]);
        let change = plan(&parts, &set(&["pass"]), &set(&["pass"])).unwrap();
        assert_eq!(
            change,
            Change {
                starting: vec![],
                stopping: vec![]
            }
        );
    }

    #[test]
    fn the_last_module_stays_on() {
        let parts = space();
        let all = set(&["pass", "browser", "ssh", "notes", "messenger"]);
        assert!(matches!(
            plan(&parts, &set(&["pass", "browser", "ssh", "notes"]), &all),
            Err(AppError::Other(code)) if code == LAST
        ));
        // A product of one module: its module cannot go off.
        let chat = vec![
            part("shell", Switch::Always),
            part("messenger", Switch::Own),
        ];
        assert!(plan(&chat, &BTreeSet::new(), &set(&["messenger"])).is_err());
        assert!(own_id(&chat, "messenger").is_ok());
        assert!(own_id(&chat, "shell").is_err());
        assert!(own_id(&space(), "capture").is_err());
    }

    #[test]
    fn a_module_that_fails_to_start_goes_back_off() {
        let parts = space();
        let before = set(&["notes", "messenger"]);
        let after = BTreeSet::new();
        assert_eq!(
            undo_failed(&parts, &before, &after, &["messenger"]),
            set(&["messenger"])
        );
        // A follower that fails takes its leader back off.
        assert_eq!(
            undo_failed(&parts, &before, &after, &["capture"]),
            set(&["notes"])
        );
        // A module that was on already stays as it was.
        assert_eq!(
            undo_failed(&parts, &before, &after, &["pass"]),
            BTreeSet::new()
        );
        assert_eq!(undo_failed(&parts, &before, &after, &[]), BTreeSet::new());
    }
}
