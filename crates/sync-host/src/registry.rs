// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a product syncs: the entities its modules register, the setting
//! keys they share, what they add to `sync_status`, and the order of the
//! cycle the product declares ([`Plan`]).
//!
//! A table entity is described by a [`TableSpec`] — data, read by the
//! generic machinery of [`crate::rows`] — plus optional [`Hooks`] for what
//! one type does differently. An entity with a [`Handler`] of its own does
//! its collect, apply and garbage collection itself.

use crate::{Host, SyncConfig, SyncManager};
use async_trait::async_trait;
use serde::Serialize;
use serde_json::{Map, Value};
use sqlx::{Pool, Sqlite};
use std::any::Any;
use std::collections::HashMap;
use std::path::Path;
use veydan_core::{BoxFuture, CmdResult, Core};
use veydan_sync::{sha256_hex, Engine, HlcClock, LocalState, Op};

/// Link table folded into the parent payload as an id array.
pub struct LinkSpec {
    pub table: &'static str,
    pub parent_col: &'static str,
    pub child_col: &'static str,
    /// Payload key holding the child ids.
    pub key: &'static str,
    /// Table (keyed by `id`) the child ids name: a child that is not on this
    /// device is left out of the link table.
    pub child_table: Option<&'static str>,
}

/// A payload key naming a row of a table keyed by `id`.
pub struct Ref {
    pub key: &'static str,
    pub table: &'static str,
}

pub enum Delete {
    /// `DELETE FROM table WHERE pk = ?` after the listed statements (`?` = id)
    /// and the link rows.
    Plain(&'static [&'static str]),
    /// Deletion belongs to another entity; tombstones are neither sent nor applied.
    Ignore,
}

/// A table as sync carries it. The entity name, the columns and the link
/// keys are the payload of the frozen vault format (spec 9.5).
pub struct TableSpec {
    pub entity: &'static str,
    pub table: &'static str,
    pub pk: &'static str,
    pub columns: &'static [&'static str],
    pub links: &'static [LinkSpec],
    /// SQL condition selecting the rows to sync.
    pub filter: Option<&'static str>,
    /// A local row with the same values here is the same thing under another id.
    pub unique: Option<&'static [&'static str]>,
    /// Remote rows may create local rows; `false` for tables owned by another entity.
    pub insert: bool,
    pub delete: Delete,
    /// References cleared when the row they name is not on this device. A
    /// reference into the entity's own table also orders the apply: rows
    /// that name a parent come after those that do not.
    pub refs: &'static [Ref],
    /// References a row cannot do without: until the row they name is here,
    /// the op waits for the next cycle.
    pub requires: &'static [Ref],
    /// Entities whose rows an applied op of this one changes too; the UI
    /// reloads them as well.
    pub also_changes: &'static [&'static str],
    /// A setting that keeps the newest op of each row that is not here
    /// (`insert: false` only), applied once the row appears. It is local and
    /// a new binding drops it.
    pub hold: Option<&'static str>,
}

impl TableSpec {
    /// A table with no links, filter, unique key, references or holds, whose
    /// rows sync both ways and whose delete is plain.
    pub const fn plain(
        entity: &'static str,
        table: &'static str,
        columns: &'static [&'static str],
    ) -> Self {
        Self {
            entity,
            table,
            pk: "id",
            columns,
            links: &[],
            filter: None,
            unique: None,
            insert: true,
            delete: Delete::Plain(&[]),
            refs: &[],
            requires: &[],
            also_changes: &[],
            hold: None,
        }
    }
}

/// What the hook `before_upsert` sees of a remote row about to be written.
pub struct Upsert<'a> {
    /// Where the hook finds the states of the modules and of the lock.
    pub host: Host<'a>,
    pub db: &'a Pool<Sqlite>,
    /// The data directory of the app.
    pub data_dir: &'a Path,
    /// The id the op names.
    pub id: &'a str,
    pub payload: &'a mut Map<String, Value>,
    /// The local row with the same unique key under another id; it takes the
    /// values, and the op's id becomes an alias of it.
    pub twin: Option<Twin>,
    /// Values one op of this apply renamed, for the hooks of the ops after it.
    pub aliases: &'a mut HashMap<String, String>,
    /// Columns that keep their local value.
    pub keep_local: Vec<&'static str>,
    /// Device-local columns a row is inserted with.
    pub defaults: Vec<(&'static str, Value)>,
}

pub struct Twin {
    pub id: String,
    /// The op's id sorts first: its values win over the twin's own.
    pub remote_wins: bool,
}

/// What a hook `on_delete` made of a tombstone.
pub enum Deletion {
    /// Go on with the plain delete of the spec.
    Plain,
    /// The hook removed the row.
    Done,
    /// Not now (the row is in use): the op comes again next cycle.
    Later,
    /// The row stays and is published again on the next cycle.
    Keep,
}

/// Whether a row this device never published goes out now.
pub type PublishNew = for<'a> fn(&'a Pool<Sqlite>, &'a Value) -> BoxFuture<'a, bool>;
/// Rules over the payload beyond `refs` and `requires`; `false` waits for the next cycle.
pub type Sanitize =
    for<'a> fn(&'a Pool<Sqlite>, &'a mut Map<String, Value>) -> BoxFuture<'a, CmdResult<bool>>;
pub type BeforeUpsert = for<'a, 'b> fn(&'a mut Upsert<'b>) -> BoxFuture<'a, CmdResult<()>>;
pub type OnDelete =
    for<'a> fn(&'a Host<'a>, &'a Core, &'a str) -> BoxFuture<'a, CmdResult<Deletion>>;
pub type AfterApply = for<'a> fn(&'a Host<'a>) -> BoxFuture<'a, ()>;
/// The row `id` was written or deleted by an op of sync; `Err` stops the
/// apply before the op counts as applied, so it comes again.
pub type AfterRow = for<'a> fn(&'a Core, &'a str) -> BoxFuture<'a, CmdResult<()>>;

/// What one table entity does differently from the rest. Every hook is optional.
#[derive(Default, Clone, Copy)]
pub struct Hooks {
    /// Collect: asked for a row with no sync state yet.
    pub publish_new: Option<PublishNew>,
    pub sanitize: Option<Sanitize>,
    /// Before the row is written: change the payload, keep columns local,
    /// give the defaults of an insert, merge with a twin.
    pub before_upsert: Option<BeforeUpsert>,
    /// Instead of the plain delete.
    pub on_delete: Option<OnDelete>,
    /// Once after the cycle, when an op of the entity was applied.
    pub after_apply: Option<AfterApply>,
    /// Right after an op of a row was applied — the row written, or gone —
    /// with its id: where the owner publishes or retracts its label.
    pub after_row: Option<AfterRow>,
}

/// A registered table entity.
pub struct Table {
    pub spec: TableSpec,
    pub hooks: Hooks,
    /// The rows are published; `false` mirrors what other devices publish.
    pub push: bool,
    /// The module that registered it.
    pub owner: &'static str,
    /// `spec.filter`, or the registered keys for the settings.
    pub(crate) filter: Option<String>,
    /// The only ids the table takes: the registered keys for the settings.
    pub(crate) ids: Option<Vec<&'static str>>,
}

impl Table {
    pub fn filter(&self) -> Option<&str> {
        self.filter.as_deref()
    }

    /// Whether an op for `id` is this build's to apply. One that is not
    /// leaves no trace: a row and a state of it would make this device
    /// publish its tombstone, since the row is out of the filter (spec 9.2).
    pub fn takes(&self, id: &str) -> bool {
        self.ids.as_ref().is_none_or(|ids| ids.contains(&id))
    }
}

/// Local changes of a handler, ready to push.
#[derive(Default)]
pub struct Collected {
    pub ops: Vec<Op>,
    /// Saves what the ops stand for; run once the push went through.
    pub commit: Option<Commit>,
    /// Failures that do not stop the cycle: written to the developer log
    /// under the step's label once the apply ran.
    pub errors: Vec<String>,
    /// The same failures as the warnings of the cycle.
    pub warnings: Vec<String>,
}

pub type Commit = Box<dyn FnOnce(Pool<Sqlite>) -> BoxFuture<'static, CmdResult<()>> + Send>;

/// What a handler made of a pull.
#[derive(Default)]
pub struct Applied {
    /// Set when an op was skipped on a transient condition; peer heads must not advance.
    pub retry: Option<String>,
    /// Ops whose blobs are gone; reported as warnings, peer heads still advance.
    pub skipped: Vec<String>,
    /// What the handler's `finish` and `notify` read.
    pub detail: Option<Box<dyn Any + Send + Sync>>,
}

/// One cycle as a handler sees it.
pub struct Cycle<'a> {
    pub host: Host<'a>,
    pub core: &'a Core,
    pub sync: &'a SyncManager,
    pub engine: &'a Engine,
    pub config: &'a SyncConfig,
    pub clock: &'a mut HlcClock,
    pub local: &'a mut LocalState,
    pub warnings: &'a mut Vec<String>,
}

/// An entity type, or a few, whose ops a module collects and applies itself.
#[async_trait]
pub trait Handler: Send + Sync {
    /// The entity types whose ops it takes.
    fn entities(&self) -> &'static [&'static str];

    /// Its tables of sync positions; a new binding empties them.
    fn state_tables(&self) -> &'static [&'static str] {
        &[]
    }

    /// Local changes as ops; blobs are uploaded here.
    async fn collect(&self, _cx: &mut Cycle<'_>) -> CmdResult<Collected> {
        Ok(Collected::default())
    }

    /// Remote ops of every type; the handler takes its own.
    async fn apply(&self, _cx: &mut Cycle<'_>, _ops: &[Op]) -> CmdResult<Applied> {
        Ok(Applied::default())
    }

    /// Right after `apply`, with what it returned.
    async fn finish(&self, _cx: &mut Cycle<'_>, _applied: &Applied) -> CmdResult<()> {
        Ok(())
    }

    /// Once every step of the apply ran.
    fn notify(&self, _host: &Host<'_>, _applied: &Applied) {}

    /// Whether it has a late part ([`Handler::late`]). A plan derived from
    /// the registry (`veydan_shell::default_plan`) names such a handler in
    /// `late` and leaves it out of the apply, as Space's plan does with its
    /// profile files; a handler without one is applied in the apply.
    fn has_late(&self) -> bool {
        false
    }

    /// After the apply and the finish of the rows, with a push of its own.
    /// Returns why the peer heads must not advance, if they must not.
    async fn late(&self, _cx: &mut Cycle<'_>, _ops: &[Op]) -> CmdResult<Option<String>> {
        Ok(None)
    }

    /// Blobs of format v1 an op of its types keeps alive.
    async fn blob_refs(&self, _engine: &Engine, _op: &Op) -> CmdResult<Vec<String>> {
        Ok(Vec::new())
    }
}

/// A registered handler.
struct HandlerEntry {
    pub name: &'static str,
    pub owner: &'static str,
    pub handler: Box<dyn Handler>,
}

/// A note in conflict, or a profile whose files diverged.
#[derive(Debug, Serialize)]
pub struct ConflictInfo {
    pub note_id: String,
    pub title: String,
}

/// Profile currently leased by some device.
#[derive(Debug, Serialize)]
pub struct LeaseInfo {
    pub profile_id: String,
    pub device_id: String,
    pub device_name: String,
    pub own: bool,
}

pub type Conflicts = for<'a> fn(&'a Core) -> BoxFuture<'a, CmdResult<Vec<ConflictInfo>>>;
/// The leases with the id of this device, which owns the ones it holds.
pub type Leases = for<'a> fn(&'a Core, &'a str) -> BoxFuture<'a, CmdResult<Vec<LeaseInfo>>>;

/// What the modules add to `sync_status`; a field nobody provides is empty.
#[derive(Default)]
pub struct StatusParts {
    pub conflicts: Option<Conflicts>,
    pub profile_conflicts: Option<Conflicts>,
    pub profile_leases: Option<Leases>,
}

/// One step of a cycle.
#[derive(Debug, PartialEq, Eq)]
pub struct Step {
    /// What the developer log and the warnings call it.
    pub label: &'static str,
    /// Where the progress bar stands when the step starts.
    pub progress: Option<u32>,
    pub part: Part,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Part {
    /// Table entities in this order. Apply steps of one `stream` are
    /// finished together, once every step ran: one event names what changed.
    Rows {
        stream: &'static str,
        entities: &'static [&'static str],
    },
    /// The handler registered under `name`. `finish` is the label its
    /// `finish` runs under; without one it runs under the step's.
    Handler {
        name: &'static str,
        finish: Option<&'static str>,
    },
}

impl Step {
    pub const fn rows(
        label: &'static str,
        progress: Option<u32>,
        stream: &'static str,
        entities: &'static [&'static str],
    ) -> Self {
        Self {
            label,
            progress,
            part: Part::Rows { stream, entities },
        }
    }

    pub const fn handler(label: &'static str, progress: Option<u32>, name: &'static str) -> Self {
        Self {
            label,
            progress,
            part: Part::Handler { name, finish: None },
        }
    }

    pub const fn handler_then(
        label: &'static str,
        progress: Option<u32>,
        name: &'static str,
        finish: &'static str,
    ) -> Self {
        Self {
            label,
            progress,
            part: Part::Handler {
                name,
                finish: Some(finish),
            },
        }
    }
}

/// The order of a product's cycle. Entities and handlers the plan names
/// and the build does not register (the platform leaves them out) are
/// skipped; a registered one the plan does not name is refused.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    /// Collected in this order into one push.
    pub collect: &'static [Step],
    /// Applied in this order.
    pub apply: &'static [Step],
    /// Handlers whose `late` runs after the apply, in this order.
    pub late: &'static [&'static str],
}

/// The owner of the system entities, as the messages of a refusal name it.
const SYSTEM: &str = "sync";

/// Everything the modules of a product registered, checked against its plan.
pub struct Registry {
    tables: Vec<Table>,
    handlers: Vec<HandlerEntry>,
    settings: Vec<(&'static str, &'static str)>,
    status: StatusParts,
    owner: &'static str,
    plan: Option<&'static Plan>,
}

impl Registry {
    /// A registry with the system entities in it: the key row of the lock
    /// and the shared settings.
    pub fn new() -> Self {
        let mut registry = Self {
            tables: Vec::new(),
            handlers: Vec::new(),
            settings: Vec::new(),
            status: StatusParts::default(),
            owner: SYSTEM,
            plan: None,
        };
        crate::system::register(&mut registry);
        registry
    }

    /// Run the registration of module `owner`.
    pub fn add(&mut self, owner: &'static str, register: fn(&mut Registry)) {
        self.owner = owner;
        register(self);
        self.owner = SYSTEM;
    }

    /// A table entity whose rows sync both ways.
    pub fn table(&mut self, spec: TableSpec, hooks: Hooks) {
        self.push_table(spec, hooks, true);
    }

    /// A table entity this build takes from the vault and never publishes.
    pub fn mirror(&mut self, spec: TableSpec, hooks: Hooks) {
        self.push_table(spec, hooks, false);
    }

    fn push_table(&mut self, spec: TableSpec, hooks: Hooks, push: bool) {
        self.refuse_twice(spec.entity);
        self.tables.push(Table {
            filter: spec.filter.map(str::to_owned),
            ids: None,
            spec,
            hooks,
            push,
            owner: self.owner,
        });
    }

    /// The entities of `handler`, registered under `name` for the plan.
    pub fn handler(&mut self, name: &'static str, handler: impl Handler + 'static) {
        for entity in handler.entities() {
            self.refuse_twice(entity);
        }
        if let Some(other) = self.handlers.iter().find(|h| h.name == name) {
            panic!(
                "sync: handler `{name}` is registered by module `{}` and by module `{}`",
                other.owner, self.owner
            );
        }
        self.handlers.push(HandlerEntry {
            name,
            owner: self.owner,
            handler: Box::new(handler),
        });
    }

    /// A setting key synced between devices. Part of the frozen vault format:
    /// no key is added to the seven of 4.0.7 (spec 9.5).
    pub fn setting(&mut self, key: &'static str) {
        assert!(
            key.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
            "sync: setting key `{key}`"
        );
        if let Some((_, other)) = self.settings.iter().find(|(k, _)| *k == key) {
            panic!(
                "sync: setting `{key}` is registered by module `{other}` and by module `{}`",
                self.owner
            );
        }
        self.settings.push((key, self.owner));
    }

    /// The notes in conflict, for `sync_status`.
    pub fn conflicts(&mut self, provide: Conflicts) {
        self.status.conflicts = Some(provide);
    }

    /// The profiles whose files diverged, for `sync_status`.
    pub fn profile_conflicts(&mut self, provide: Conflicts) {
        self.status.profile_conflicts = Some(provide);
    }

    /// The leases of profiles, for `sync_status`.
    pub fn profile_leases(&mut self, provide: Leases) {
        self.status.profile_leases = Some(provide);
    }

    fn refuse_twice(&self, entity: &str) {
        let owner = self
            .tables
            .iter()
            .find(|t| t.spec.entity == entity)
            .map(|t| t.owner)
            .or_else(|| {
                self.handlers
                    .iter()
                    .find(|h| h.handler.entities().contains(&entity))
                    .map(|h| h.owner)
            });
        if let Some(owner) = owner {
            panic!(
                "sync: entity `{entity}` is registered by module `{owner}` and by module `{}`",
                self.owner
            );
        }
    }

    /// Check what was registered against the product's plan: every table in
    /// one collect step and one apply step, every handler named, nothing
    /// named twice. The settings select the registered keys.
    pub fn finish(mut self, plan: &'static Plan) -> Result<Self, String> {
        for table in &self.tables {
            let entity = table.spec.entity;
            for (what, steps) in [("collect", plan.collect), ("apply", plan.apply)] {
                let n = steps
                    .iter()
                    .filter(|step| match step.part {
                        Part::Rows { entities, .. } => entities.contains(&entity),
                        Part::Handler { .. } => false,
                    })
                    .count();
                if n != 1 {
                    return Err(format!(
                        "entity `{entity}` of module `{}` is in {n} {what} steps of the plan, not one",
                        table.owner
                    ));
                }
            }
        }
        for entry in &self.handlers {
            let named =
                plan.collect.iter().chain(plan.apply).any(
                    |step| matches!(step.part, Part::Handler { name, .. } if name == entry.name),
                ) || plan.late.contains(&entry.name);
            if !named {
                return Err(format!(
                    "handler `{}` of module `{}` is not in the plan",
                    entry.name, entry.owner
                ));
            }
        }
        if let Some(table) = self
            .tables
            .iter_mut()
            .find(|t| t.spec.entity == crate::system::SETTING_ENTITY)
        {
            let keys: Vec<String> = self
                .settings
                .iter()
                .map(|(k, _)| format!("'{k}'"))
                .collect();
            table.filter = Some(format!("key IN ({})", keys.join(", ")));
            table.ids = Some(self.settings.iter().map(|(k, _)| *k).collect());
        }
        self.plan = Some(plan);
        Ok(self)
    }

    pub fn plan(&self) -> &'static Plan {
        self.plan.expect("a finished registry")
    }

    pub fn tables(&self) -> &[Table] {
        &self.tables
    }

    pub fn table_of(&self, entity: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.spec.entity == entity)
    }

    /// The registered tables of a rows step, in its order.
    pub fn rows(&self, entities: &[&str]) -> Vec<&Table> {
        entities.iter().filter_map(|e| self.table_of(e)).collect()
    }

    /// The registered tables of the step called `label`, in its order.
    pub fn step_rows(&self, steps: &[Step], label: &str) -> Vec<&Table> {
        steps
            .iter()
            .filter(|step| step.label == label)
            .flat_map(|step| match step.part {
                Part::Rows { entities, .. } => self.rows(entities),
                Part::Handler { .. } => Vec::new(),
            })
            .collect()
    }

    /// The names of the registered handlers, in the order they registered.
    pub fn handler_names(&self) -> Vec<&'static str> {
        self.handlers.iter().map(|h| h.name).collect()
    }

    pub fn handler_named(&self, name: &str) -> Option<&dyn Handler> {
        self.handlers
            .iter()
            .find(|h| h.name == name)
            .map(|h| h.handler.as_ref())
    }

    /// The handler that takes ops of `entity`.
    pub fn handler_of(&self, entity: &str) -> Option<&dyn Handler> {
        self.handlers
            .iter()
            .find(|h| h.handler.entities().contains(&entity))
            .map(|h| h.handler.as_ref())
    }

    /// Every registered entity type: the tables, then the handlers'.
    pub fn entities(&self) -> Vec<&'static str> {
        self.tables
            .iter()
            .map(|t| t.spec.entity)
            .chain(
                self.handlers
                    .iter()
                    .flat_map(|h| h.handler.entities().iter().copied()),
            )
            .collect()
    }

    /// The synced setting keys, with the module that registered each. Only
    /// a build that syncs the settings has them.
    pub fn setting_keys(&self) -> Vec<(&'static str, &'static str)> {
        if self.table_of(crate::system::SETTING_ENTITY).is_none() {
            return Vec::new();
        }
        self.settings.clone()
    }

    /// What the fingerprint of the registry is made of, sorted: every entity
    /// type as `entity:<type>`, every synced setting key as `setting:<key>`.
    pub fn synced(&self) -> Vec<String> {
        let mut synced: Vec<String> = self
            .entities()
            .into_iter()
            .map(|e| format!("entity:{e}"))
            .chain(
                self.setting_keys()
                    .into_iter()
                    .map(|(k, _)| format!("setting:{k}")),
            )
            .collect();
        synced.sort_unstable();
        synced
    }

    /// What this build syncs, as a hash (spec 9.2, rule 3): the same while
    /// the types and keys are, in whatever order the modules register them.
    pub fn fingerprint(&self) -> String {
        sha256_hex(self.synced().join("\n").as_bytes())
    }

    pub fn status(&self) -> &StatusParts {
        &self.status
    }

    /// The settings that keep ops for rows that are not here.
    pub fn hold_keys(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.tables.iter().filter_map(|t| t.spec.hold)
    }

    /// The tables of sync positions the handlers keep.
    pub fn handler_state_tables(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.handlers
            .iter()
            .flat_map(|h| h.handler.state_tables().iter().copied())
    }

    /// The order the plan applies the registered entities in: the puts and
    /// the deletes, step after step. Inside a rows step the deletes go in
    /// reverse, after its puts; a handler takes both in the order of the
    /// pull.
    pub fn apply_order(&self) -> (Vec<&'static str>, Vec<&'static str>) {
        let (mut puts, mut deletes) = (Vec::new(), Vec::new());
        let plan = self.plan();
        for step in plan.apply {
            match step.part {
                Part::Rows { entities, .. } => {
                    let tables = self.rows(entities);
                    puts.extend(tables.iter().map(|t| t.spec.entity));
                    deletes.extend(
                        tables
                            .iter()
                            .rev()
                            .filter(|t| !matches!(t.spec.delete, Delete::Ignore))
                            .map(|t| t.spec.entity),
                    );
                }
                Part::Handler { name, .. } => {
                    let entities = self.handler_named(name).map(|h| h.entities());
                    puts.extend(entities.unwrap_or_default());
                    deletes.extend(entities.unwrap_or_default());
                }
            }
        }
        for name in plan.late {
            let entities = self.handler_named(name).map(|h| h.entities());
            puts.extend(entities.unwrap_or_default());
            deletes.extend(entities.unwrap_or_default());
        }
        (puts, deletes)
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROWS: &[&str] = &["password_vault", "setting", "label", "thing", "part"];
    const PLAN: Plan = Plan {
        collect: &[Step::rows("rows", None, "app", ROWS)],
        apply: &[Step::rows("rows", None, "app", ROWS)],
        late: &[],
    };

    fn things(registry: &mut Registry) {
        registry.table(
            TableSpec::plain("thing", "things", &["name"]),
            Hooks::default(),
        );
        registry.table(
            TableSpec {
                delete: Delete::Ignore,
                ..TableSpec::plain("part", "parts", &["thing_id"])
            },
            Hooks::default(),
        );
        registry.setting("thing_key");
    }

    #[test]
    fn sync_registers_the_system_entities_itself() {
        let registry = Registry::new();
        assert_eq!(
            registry.table_of("password_vault").map(|t| t.owner),
            Some("sync")
        );
        let label = registry.table_of("label").unwrap();
        assert_eq!((label.owner, label.push), ("sync", true));
        assert_eq!(
            (label.spec.table, label.spec.pk, label.spec.columns),
            (
                "labels",
                "key",
                &["kind", "id", "name", "parent_kind", "parent_id", "color"][..]
            )
        );
        assert_eq!(
            registry.table_of("setting").is_some(),
            cfg!(desktop),
            "the settings sync between desktops"
        );
    }

    #[cfg(desktop)]
    #[test]
    fn the_settings_select_the_registered_keys() {
        let mut registry = Registry::new();
        registry.add("things", things);
        let registry = registry.finish(&PLAN).unwrap();
        assert_eq!(
            registry.table_of("setting").unwrap().filter(),
            Some("key IN ('ui_locale', 'notes_lock_timeout_min', 'thing_key')")
        );
        assert!(registry.setting_keys().contains(&("thing_key", "things")));
        let settings = registry.table_of("setting").unwrap();
        assert!(settings.takes("thing_key") && settings.takes("ui_locale"));
        assert!(!settings.takes("sync_folder_path"));
        assert!(registry.table_of("thing").unwrap().takes("any id"));
    }

    /// Spec 9.2, rule 3: what a build syncs, whatever order its modules
    /// register in.
    #[test]
    fn the_fingerprint_is_the_types_and_keys_alone() {
        fn more(registry: &mut Registry) {
            registry.table(
                TableSpec::plain("more", "mores", &["name"]),
                Hooks::default(),
            );
        }
        const MORE: Plan = Plan {
            collect: &[
                Step::rows("rows", None, "app", ROWS),
                Step::rows("more", None, "app", &["more"]),
            ],
            apply: &[
                Step::rows("rows", None, "app", ROWS),
                Step::rows("more", None, "app", &["more"]),
            ],
            late: &[],
        };
        let build = |owners: &[&'static str]| {
            let mut registry = Registry::new();
            for owner in owners {
                registry.add(owner, if *owner == "more" { more } else { things });
            }
            registry.finish(&MORE).unwrap()
        };
        let one = build(&["things", "more"]);
        let other = build(&["more", "things"]);
        assert_eq!(one.fingerprint(), other.fingerprint());
        let fewer = build(&["things"]);
        assert_ne!(one.fingerprint(), fewer.fingerprint());
        let mut expected = vec![
            "entity:label",
            "entity:more",
            "entity:part",
            "entity:password_vault",
            "entity:thing",
        ];
        if cfg!(desktop) {
            expected.extend([
                "entity:setting",
                "setting:notes_lock_timeout_min",
                "setting:thing_key",
                "setting:ui_locale",
            ]);
        }
        expected.sort_unstable();
        assert_eq!(one.synced(), expected);
    }

    #[test]
    fn an_entity_the_plan_does_not_name_is_refused() {
        const SHORT: Plan = Plan {
            collect: &[Step::rows(
                "rows",
                None,
                "app",
                &["password_vault", "setting", "label", "thing"],
            )],
            apply: &[Step::rows("rows", None, "app", ROWS)],
            late: &[],
        };
        let mut registry = Registry::new();
        registry.add("things", things);
        let refused = registry.finish(&SHORT).err().unwrap();
        assert_eq!(
            refused,
            "entity `part` of module `things` is in 0 collect steps of the plan, not one"
        );
    }

    #[test]
    #[should_panic(
        expected = "entity `thing` is registered by module `things` and by module `more`"
    )]
    fn an_entity_registered_twice_is_refused() {
        let mut registry = Registry::new();
        registry.add("things", things);
        registry.add("more", |registry| {
            registry.table(
                TableSpec::plain("thing", "things", &["name"]),
                Hooks::default(),
            )
        });
    }

    #[test]
    fn the_deletes_of_a_step_go_in_reverse_after_its_puts() {
        let mut registry = Registry::new();
        registry.add("things", things);
        let registry = registry.finish(&PLAN).unwrap();
        let (puts, deletes) = registry.apply_order();
        let system: &[&str] = if cfg!(desktop) {
            &["password_vault", "setting", "label"]
        } else {
            &["password_vault", "label"]
        };
        let expected_puts: Vec<&str> = system.iter().copied().chain(["thing", "part"]).collect();
        assert_eq!(puts, expected_puts);
        let expected_deletes: Vec<&str> = ["thing"]
            .into_iter()
            .chain(system.iter().rev().copied())
            .collect();
        assert_eq!(deletes, expected_deletes);
    }
}
