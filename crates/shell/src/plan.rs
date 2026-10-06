// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The plan of the sync cycle of a product that has no order of its own to
//! keep (internal/platform-spec.md 9.1): derived from what its modules register.
//!
//! Space keeps its explicit plan: its push is frozen byte for byte with
//! 4.0.7, which interleaves the rows of several modules in one pass and
//! collects in another order than it applies (18.1, № 62). A product made
//! later has no such past; its order follows from the registry:
//!
//! - collect: every table entity in the order it was registered (the
//!   system's first, then the modules' in the order of the list), then each
//!   handler, then the labels;
//! - apply: the table entities that may insert a row, in the same order;
//!   then each handler, whose entities may name those rows (a note its tags
//!   and folders); then the entities that only update a row another step
//!   brings (`TableSpec::insert` false, the flags of a note); then the
//!   labels, after the rows they name; a handler with a late part
//!   (`Handler::has_late`) is left out;
//! - late: the handlers with a late part, after the apply, as Space's plan
//!   runs its profile files. Each handler is so applied once.

use crate::module::Module;
use veydan_sync_host::{Plan, Registry, Step};

/// The labels of the directory: last each way, as in Space.
const LABEL: &str = "label";

/// A plan for a product made of `modules`, the shell's own included, from
/// what they register on this platform. Built once when the product starts;
/// the plan lives as long as the process.
pub fn default_plan(modules: &[Module]) -> &'static Plan {
    let mut registry = Registry::new();
    for (id, sync) in std::iter::once(crate::app::shell_module())
        .map(|shell| (shell.id, shell.sync))
        .chain(modules.iter().map(|m| (m.id, m.sync)))
    {
        if let Some(sync) = sync {
            registry.add(id, sync);
        }
    }
    plan_of(&registry)
}

fn plan_of(registry: &Registry) -> &'static Plan {
    let tables: Vec<(&'static str, bool)> = registry
        .tables()
        .iter()
        .map(|t| (t.spec.entity, t.spec.insert))
        .filter(|(entity, _)| *entity != LABEL)
        .collect();
    let has_label = registry.table_of(LABEL).is_some();
    let handlers = registry.handler_names();
    let has_late = |name: &&'static str| registry.handler_named(name).is_some_and(|h| h.has_late());
    let (late, applied): (Vec<&'static str>, Vec<&'static str>) =
        handlers.iter().copied().partition(has_late);

    let all: Vec<&'static str> = tables.iter().map(|(e, _)| *e).collect();
    let inserting: Vec<&'static str> = tables.iter().filter(|t| t.1).map(|t| t.0).collect();
    let updating: Vec<&'static str> = tables.iter().filter(|t| !t.1).map(|t| t.0).collect();

    let mut collect = Vec::new();
    if !all.is_empty() {
        collect.push(Step::rows("rows", None, "app", leak(all)));
    }
    collect.extend(handlers.iter().map(|&name| Step::handler(name, None, name)));
    let mut apply = Vec::new();
    if !inserting.is_empty() {
        apply.push(Step::rows("rows", None, "app", leak(inserting)));
    }
    apply.extend(applied.iter().map(|&name| Step::handler(name, None, name)));
    if !updating.is_empty() {
        apply.push(Step::rows("updated rows", None, "app", leak(updating)));
    }
    if has_label {
        collect.push(Step::rows("labels", None, "app", &[LABEL]));
        apply.push(Step::rows("labels", None, "app", &[LABEL]));
    }
    Box::leak(Box::new(Plan {
        collect: leak(collect),
        apply: leak(apply),
        late: leak(late),
    }))
}

fn leak<T>(items: Vec<T>) -> &'static [T] {
    Box::leak(items.into_boxed_slice())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::Product;
    use veydan_sync_host::{Handler, Hooks, Part, TableSpec};

    struct Files;

    impl Handler for Files {
        fn entities(&self) -> &'static [&'static str] {
            &["file"]
        }
    }

    /// A handler with a late part, like the profile files of Space.
    struct Leases;

    impl Handler for Leases {
        fn entities(&self) -> &'static [&'static str] {
            &["lease"]
        }

        fn has_late(&self) -> bool {
            true
        }
    }

    /// A module like the notes: a catalog, rows that only update, files.
    fn files() -> Module {
        Module {
            sync: Some(|registry| {
                registry.table(
                    TableSpec::plain("folder", "folders", &["name"]),
                    Hooks::default(),
                );
                registry.table(
                    TableSpec {
                        insert: false,
                        ..TableSpec::plain("flag", "flags", &["pinned"])
                    },
                    Hooks::default(),
                );
                registry.handler("files", Files);
            }),
            ..crate::module! { id: "files", setup: |_app| Ok(()), commands: [] }
        }
    }

    fn rows(steps: &[Step]) -> Vec<(&str, Vec<&str>)> {
        steps
            .iter()
            .map(|step| match step.part {
                Part::Rows { entities, .. } => (step.label, entities.to_vec()),
                Part::Handler { name, .. } => (step.label, vec![name]),
            })
            .collect()
    }

    #[test]
    fn the_default_plan_puts_the_catalog_before_the_files_and_the_flags_after() {
        let plan = default_plan(&[files()]);
        #[cfg(desktop)]
        let system = ["password_vault", "setting"].as_slice();
        #[cfg(mobile)]
        let system = ["password_vault"].as_slice();
        let with = |extra: &[&'static str]| -> Vec<&str> {
            system.iter().copied().chain(extra.iter().copied()).collect()
        };
        assert_eq!(
            rows(plan.collect),
            [
                ("rows", with(&["folder", "flag"])),
                ("files", vec!["files"]),
                ("labels", vec!["label"]),
            ]
        );
        assert_eq!(
            rows(plan.apply),
            [
                ("rows", with(&["folder"])),
                ("files", vec!["files"]),
                ("updated rows", vec!["flag"]),
                ("labels", vec!["label"]),
            ]
        );
        assert_eq!(plan.late, [] as [&str; 0]);
    }

    /// A handler with a late part runs after the apply, once: it is not
    /// applied with the others.
    #[test]
    fn a_handler_with_a_late_part_is_applied_late_only() {
        let leases = || Module {
            sync: Some(|registry| registry.handler("leases", Leases)),
            ..crate::module! { id: "leases", setup: |_app| Ok(()), commands: [] }
        };
        let plan = default_plan(&[files(), leases()]);
        assert_eq!(plan.late, ["leases"]);
        let handlers = |steps: &[Step]| -> Vec<&str> {
            steps
                .iter()
                .filter_map(|step| match step.part {
                    Part::Handler { name, .. } => Some(name),
                    Part::Rows { .. } => None,
                })
                .collect()
        };
        assert_eq!(handlers(plan.collect), ["files", "leases"]);
        assert_eq!(handlers(plan.apply), ["files"]);
        let product = Product {
            id: "probe",
            name: "Veydan Probe",
            desktop_entry: "veydanprobe",
            icon: "veydanprobe",
            sync: plan,
        };
        let registry = crate::sync_registry(product, vec![files(), leases()]).unwrap();
        let (puts, deletes) = registry.apply_order();
        for entity in ["file", "lease"] {
            assert_eq!(puts.iter().filter(|e| **e == entity).count(), 1, "{puts:?}");
            assert_eq!(
                deletes.iter().filter(|e| **e == entity).count(),
                1,
                "{deletes:?}"
            );
        }
        assert_eq!(puts.last(), Some(&"lease"));
    }

    /// The registry of the product takes the plan: every entity in one
    /// collect and one apply step, every handler named.
    #[test]
    fn the_registry_of_the_product_accepts_its_default_plan() {
        let modules = [files()];
        let product = Product {
            id: "probe",
            name: "Veydan Probe",
            desktop_entry: "veydanprobe",
            icon: "veydanprobe",
            sync: default_plan(&modules),
        };
        let registry = crate::sync_registry(product, vec![files()]).unwrap();
        let (puts, _) = registry.apply_order();
        assert!(puts.contains(&"file") && puts.contains(&"label"), "{puts:?}");
    }

    /// A product whose modules sync nothing still syncs the system entities.
    #[test]
    fn a_product_without_entities_syncs_the_system_ones() {
        let quiet = crate::module! { id: "quiet", setup: |_app| Ok(()), commands: [] };
        let plan = default_plan(&[quiet]);
        assert_eq!(plan.late, [] as [&str; 0]);
        assert_eq!(plan.collect.last().map(|s| s.label), Some("labels"));
        assert!(rows(plan.apply)[0].1.contains(&"password_vault"));
    }
}
