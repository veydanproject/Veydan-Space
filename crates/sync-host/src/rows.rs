// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Table rows as sync entities. One implementation serves every registered
//! table: the synced columns of a row travel inline as a JSON object, local
//! changes are found by hashing that object, and the newer remote row wins
//! (LWW by HLC). What one type does differently is a hook of its
//! registration ([`crate::Hooks`]); nothing here names a table of a module.
//!
//! Op payload: `{ "<column>": <value>, .., "<link key>": [<id>..] }`.
//! A delete op carries an empty payload.

use crate::registry::{Delete, Deletion, LinkSpec, Table, Twin, Upsert};
use crate::state::{load_row_state, load_row_states, save_row_state, took, RowSyncState};
use crate::Host;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sqlx::sqlite::SqliteArguments;
use sqlx::{query::Query, AssertSqlSafe, Pool, Sqlite};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use veydan_core::{settings, AppError, CmdResult, Core};
use veydan_sync::{sha256_hex, Hlc, HlcClock, Op};

pub const EVENT_CHANGED: &str = "sync://data-changed";

/// Ids are UUIDs, setting keys or the literal `default`; nothing else is accepted.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

// ── Reading rows ─────────────────────────────────────────────────────────────

/// `SELECT pk, json_object('c', c, ..) FROM table`: SQLite keeps the column types.
fn select_sql(table: &Table) -> String {
    let spec = &table.spec;
    let pairs: Vec<String> = spec.columns.iter().map(|c| format!("'{c}', {c}")).collect();
    let mut sql = format!(
        "SELECT {}, json_object({}) FROM {}",
        spec.pk,
        pairs.join(", "),
        spec.table
    );
    if let Some(f) = table.filter() {
        sql.push_str(" WHERE ");
        sql.push_str(f);
    }
    sql
}

/// Child ids per parent for a link table, one query.
async fn link_map(
    db: &Pool<Sqlite>,
    link: &LinkSpec,
    parent: Option<&str>,
) -> CmdResult<HashMap<String, Vec<String>>> {
    let mut sql = format!(
        "SELECT {}, {} FROM {}",
        link.parent_col, link.child_col, link.table
    );
    if parent.is_some() {
        sql.push_str(&format!(" WHERE {} = ?", link.parent_col));
    }
    sql.push_str(&format!(
        " ORDER BY {}, {}",
        link.parent_col, link.child_col
    ));
    let mut q = sqlx::query_as::<_, (String, String)>(AssertSqlSafe(sql));
    if let Some(p) = parent {
        q = q.bind(p.to_string());
    }
    let rows = q.fetch_all(db).await.map_err(AppError::db)?;
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for (p, c) in rows {
        map.entry(p).or_default().push(c);
    }
    Ok(map)
}

fn parse_object(json: &str) -> Map<String, Value> {
    serde_json::from_str::<Value>(json)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// Rows matching `select` with their link arrays folded in, as `(id, payload)`.
async fn read_payloads(
    db: &Pool<Sqlite>,
    table: &Table,
    select: String,
    id: Option<&str>,
) -> CmdResult<Vec<(String, Value)>> {
    let mut q = sqlx::query_as::<_, (String, String)>(AssertSqlSafe(select));
    if let Some(id) = id {
        q = q.bind(id.to_string());
    }
    let rows = q.fetch_all(db).await.map_err(AppError::db)?;
    let mut links: Vec<(&LinkSpec, HashMap<String, Vec<String>>)> = Vec::new();
    for link in table.spec.links {
        links.push((link, link_map(db, link, id).await?));
    }
    Ok(rows
        .into_iter()
        .map(|(row_id, json)| {
            let mut payload = parse_object(&json);
            for (link, map) in &links {
                let ids = map.get(&row_id).cloned().unwrap_or_default();
                payload.insert(
                    link.key.into(),
                    serde_json::to_value(ids).unwrap_or_default(),
                );
            }
            (row_id, Value::Object(payload))
        })
        .collect())
}

/// Every synced row of a table.
pub async fn read_rows(db: &Pool<Sqlite>, table: &Table) -> CmdResult<Vec<(String, Value)>> {
    read_payloads(db, table, select_sql(table), None).await
}

pub async fn read_row(db: &Pool<Sqlite>, table: &Table, id: &str) -> CmdResult<Option<Value>> {
    let sql = format!(
        "{} {} {} = ?",
        select_sql(table),
        if table.filter().is_some() {
            "AND"
        } else {
            "WHERE"
        },
        table.spec.pk
    );
    Ok(read_payloads(db, table, sql, Some(id))
        .await?
        .pop()
        .map(|(_, v)| v))
}

/// serde_json sorts object keys, so equal rows hash equal.
fn payload_hash(payload: &Value) -> String {
    sha256_hex(payload.to_string().as_bytes())
}

// ── Push ─────────────────────────────────────────────────────────────────────

pub struct LocalChanges {
    pub ops: Vec<Op>,
    pub states: Vec<RowSyncState>,
}

fn make_op(table: &Table, id: &str, hlc: Hlc, payload: Value, deleted: bool) -> Op {
    Op {
        entity_type: table.spec.entity.into(),
        entity_id: id.into(),
        hlc,
        deleted,
        payload,
    }
}

/// Rows of `tables` whose synced columns differ from what the vault has,
/// plus tombstones for rows that disappeared. Tables this build only
/// mirrors publish nothing. While the vault is read again for a new
/// registry, `reread` holds what the registries before it took: a row of a
/// type or setting key they did not take waits until the vault was read,
/// since the vault may hold a newer value of it (spec 9.2, rule 3).
pub async fn collect_local_changes(
    core: &Core,
    clock: &mut HlcClock,
    tables: &[&Table],
    reread: Option<&[String]>,
) -> CmdResult<LocalChanges> {
    let db = &core.db;
    let mut out = LocalChanges {
        ops: Vec::new(),
        states: Vec::new(),
    };

    for table in tables.iter().filter(|t| t.push) {
        let spec = &table.spec;
        let waits = |id: &str| reread.is_some_and(|known| !took(known, spec.entity, id));
        let mut states = load_row_states(db, spec.entity).await?;
        // The state of an id the table does not take, such as a setting key
        // no module registers any more, is not of a row that disappeared: its
        // tombstone would delete the value on every device (spec 9.2, rule 1).
        states.retain(|id, _| table.takes(id));
        for (id, payload) in read_rows(db, table).await? {
            let hash = payload_hash(&payload);
            let prev = states.remove(&id);
            if prev
                .as_ref()
                .map(|s| s.synced_hash == hash && !s.deleted)
                .unwrap_or(false)
                || waits(&id)
            {
                continue;
            }
            if prev.is_none() {
                if let Some(publish_new) = table.hooks.publish_new {
                    if !publish_new(db, &payload).await {
                        continue;
                    }
                }
            }
            let hlc = clock.now();
            out.ops
                .push(make_op(table, &id, hlc.clone(), payload, false));
            out.states.push(RowSyncState {
                entity: spec.entity.into(),
                id,
                head_hlc: Some(hlc),
                synced_hash: hash,
                deleted: false,
            });
        }
        if matches!(spec.delete, Delete::Ignore) {
            continue;
        }
        // Whatever is left in `states` has no row any more.
        for (_, mut st) in states
            .into_iter()
            .filter(|(id, s)| !s.deleted && !waits(id))
        {
            let hlc = clock.now();
            out.ops.push(make_op(
                table,
                &st.id,
                hlc.clone(),
                Value::Object(Map::new()),
                true,
            ));
            st.deleted = true;
            st.head_hlc = Some(hlc);
            out.states.push(st);
        }
    }
    Ok(out)
}

// ── Pull ─────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ApplyOutcome {
    /// Entities with at least one applied op; the UI reloads their stores.
    pub changed: BTreeSet<String>,
    /// Setting keys with an applied op; their watchers are told.
    pub settings: BTreeSet<String>,
    /// Set when an op was skipped on a transient condition; peer heads must not advance.
    pub retry: Option<String>,
}

fn bind_json<'q>(
    q: Query<'q, Sqlite, SqliteArguments>,
    v: &Value,
) -> Query<'q, Sqlite, SqliteArguments> {
    match v {
        Value::Null => q.bind(None::<String>),
        Value::Bool(b) => q.bind(*b as i64),
        Value::Number(n) => match n.as_i64() {
            Some(i) => q.bind(i),
            None => q.bind(n.as_f64()),
        },
        Value::String(s) => q.bind(s.clone()),
        other => q.bind(other.to_string()),
    }
}

/// Run a statement built from code constants with JSON-typed bind values.
pub async fn exec(db: &Pool<Sqlite>, sql: String, values: &[Value]) -> CmdResult<u64> {
    let mut q = sqlx::query(AssertSqlSafe(sql));
    for v in values {
        q = bind_json(q, v);
    }
    Ok(q.execute(db).await.map_err(AppError::db)?.rows_affected())
}

/// Columns the remote row carries and we sync; anything else keeps its local value.
fn known_columns(table: &Table, payload: &Map<String, Value>) -> Vec<(&'static str, Value)> {
    table
        .spec
        .columns
        .iter()
        .filter_map(|c| payload.get(*c).map(|v| (*c, v.clone())))
        .collect()
}

enum Upserted {
    Done {
        id: String,
        absorbed: Option<String>,
    },
    Skip,
}

fn unique_vals<'a>(keys: &[&str], payload: &'a Map<String, Value>) -> Option<Vec<&'a str>> {
    keys.iter()
        .map(|k| payload.get(*k).and_then(Value::as_str))
        .collect()
}

async fn find_unique_other(
    db: &Pool<Sqlite>,
    table: &Table,
    keys: &[&str],
    vals: &[&str],
    id: &str,
) -> CmdResult<Option<String>> {
    let where_sql = keys
        .iter()
        .map(|k| format!("{k} = ?"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let sql = format!(
        "SELECT {} FROM {} WHERE {} AND {} != ?",
        table.spec.pk, table.spec.table, where_sql, table.spec.pk
    );
    let mut q = sqlx::query_as::<_, (String,)>(AssertSqlSafe(sql));
    for v in vals {
        q = q.bind(*v);
    }
    q = q.bind(id);
    Ok(q.fetch_optional(db)
        .await
        .map_err(AppError::db)?
        .map(|(x,)| x))
}

/// Write the row. `Skip` when the table forbids inserts and the row is absent.
async fn upsert(
    host: &Host<'_>,
    db: &Pool<Sqlite>,
    data_dir: &Path,
    table: &Table,
    id: &str,
    payload: &mut Map<String, Value>,
    aliases: &mut HashMap<String, String>,
) -> CmdResult<Upserted> {
    let spec = &table.spec;
    // Same unique key under another id: keep that row, absorb the remote id.
    let mut twin = None;
    if let Some(keys) = spec.unique {
        if let Some(vals) = unique_vals(keys, payload) {
            if let Some(other_id) = find_unique_other(db, table, keys, &vals, id).await? {
                let remote_wins = id < other_id.as_str();
                twin = Some(Twin {
                    id: other_id,
                    remote_wins,
                });
            }
        }
    }
    let mut cx = Upsert {
        host: *host,
        db,
        data_dir,
        id,
        payload,
        twin,
        aliases,
        keep_local: Vec::new(),
        defaults: Vec::new(),
    };
    if let Some(before_upsert) = table.hooks.before_upsert {
        before_upsert(&mut cx).await?;
    }
    let Upsert {
        payload,
        twin,
        keep_local,
        defaults,
        ..
    } = cx;
    let cols: Vec<(&str, Value)> = known_columns(table, payload)
        .into_iter()
        .filter(|(c, _)| !keep_local.contains(c))
        .collect();

    if let (Some(twin), Some(keys)) = (twin, spec.unique) {
        let skip: HashSet<&str> = keys.iter().copied().collect();
        let rest: Vec<(&str, Value)> = cols
            .into_iter()
            .filter(|(c, _)| !skip.contains(c))
            .collect();
        update_row(db, table, &twin.id, &rest).await?;
        return Ok(Upserted::Done {
            id: twin.id,
            absorbed: Some(id.into()),
        });
    }

    if !spec.insert {
        return if update_row(db, table, id, &cols).await? {
            Ok(Upserted::Done {
                id: id.into(),
                absorbed: None,
            })
        } else {
            Ok(Upserted::Skip)
        };
    }

    let mut names = vec![spec.pk.to_string()];
    let mut values = vec![Value::String(id.into())];
    for (c, v) in cols.iter().chain(defaults.iter()) {
        names.push((*c).into());
        values.push(v.clone());
    }
    let placeholders = vec!["?"; names.len()].join(", ");
    let set: Vec<String> = cols
        .iter()
        .map(|(c, _)| format!("{c} = excluded.{c}"))
        .collect();
    let on_conflict = if set.is_empty() {
        "DO NOTHING".to_string()
    } else {
        format!("DO UPDATE SET {}", set.join(", "))
    };
    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({}) ON CONFLICT({}) {}",
        spec.table,
        names.join(", "),
        placeholders,
        spec.pk,
        on_conflict
    );
    exec(db, sql, &values).await?;
    Ok(Upserted::Done {
        id: id.into(),
        absorbed: None,
    })
}

async fn update_row(
    db: &Pool<Sqlite>,
    table: &Table,
    id: &str,
    cols: &[(&str, Value)],
) -> CmdResult<bool> {
    let spec = &table.spec;
    if cols.is_empty() {
        let sql = format!("SELECT 1 FROM {} WHERE {} = ?", spec.table, spec.pk);
        let exists: Option<(i64,)> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(id)
            .fetch_optional(db)
            .await
            .map_err(AppError::db)?;
        return Ok(exists.is_some());
    }
    let set: Vec<String> = cols.iter().map(|(c, _)| format!("{c} = ?")).collect();
    let sql = format!(
        "UPDATE {} SET {} WHERE {} = ?",
        spec.table,
        set.join(", "),
        spec.pk
    );
    let mut values: Vec<Value> = cols.iter().map(|(_, v)| v.clone()).collect();
    values.push(Value::String(id.into()));
    Ok(exec(db, sql, &values).await? > 0)
}

pub fn is_fk_error(e: &AppError) -> bool {
    let s = e.to_string();
    s.contains("787") || s.contains("FOREIGN KEY")
}

async fn row_exists(db: &Pool<Sqlite>, table: &str, id: &str) -> CmdResult<bool> {
    let sql = format!("SELECT 1 FROM {table} WHERE id = ? LIMIT 1");
    let row: Option<(i64,)> = sqlx::query_as(AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(db)
        .await
        .map_err(AppError::db)?;
    Ok(row.is_some())
}

async fn drop_missing_ref(
    db: &Pool<Sqlite>,
    payload: &mut Map<String, Value>,
    key: &str,
    table: &str,
) -> CmdResult<()> {
    let Some(id) = payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    else {
        return Ok(());
    };
    if !row_exists(db, table, id).await? {
        payload.insert(key.into(), Value::Null);
    }
    Ok(())
}

/// Clear or wait for references whose row is not on this device yet.
async fn sanitize_payload(
    db: &Pool<Sqlite>,
    table: &Table,
    payload: &mut Map<String, Value>,
) -> CmdResult<bool> {
    let spec = &table.spec;
    for required in spec.requires {
        let Some(id) = payload
            .get(required.key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            return Ok(false);
        };
        if !row_exists(db, required.table, id).await? {
            return Ok(false);
        }
    }
    for r in spec.refs {
        drop_missing_ref(db, payload, r.key, r.table).await?;
    }
    match table.hooks.sanitize {
        Some(sanitize) => sanitize(db, payload).await,
        None => Ok(true),
    }
}

/// Puts before deletes; puts in the order of `tables`, a row that names a
/// parent of its own table after one that does not; deletes in reverse.
fn apply_order(tables: &[&Table], op: &Op) -> (u8, usize, u8, Hlc) {
    let idx = tables
        .iter()
        .position(|t| t.spec.entity == op.entity_type)
        .unwrap_or(usize::MAX);
    let n = tables.len();
    if op.deleted {
        (
            1,
            n.saturating_sub(idx.saturating_add(1)),
            0,
            op.hlc.clone(),
        )
    } else {
        let names_parent = tables.get(idx).is_some_and(|t| {
            t.spec.refs.iter().any(|r| {
                r.table == t.spec.table
                    && matches!(op.payload.get(r.key), Some(Value::String(s)) if !s.is_empty())
            })
        });
        (0, idx, u8::from(names_parent), op.hlc.clone())
    }
}

async fn apply_links(
    db: &Pool<Sqlite>,
    table: &Table,
    id: &str,
    payload: &Map<String, Value>,
) -> CmdResult<()> {
    for link in table.spec.links {
        let Some(ids) = payload.get(link.key).and_then(Value::as_array) else {
            continue;
        };
        let del = format!("DELETE FROM {} WHERE {} = ?", link.table, link.parent_col);
        exec(db, del, &[Value::String(id.into())]).await?;
        let ins = format!(
            "INSERT OR IGNORE INTO {} ({}, {}) VALUES (?, ?)",
            link.table, link.parent_col, link.child_col
        );
        for child in ids.iter().filter_map(Value::as_str).filter(|c| valid_id(c)) {
            if let Some(child_table) = link.child_table {
                if !row_exists(db, child_table, child).await? {
                    continue;
                }
            }
            exec(
                db,
                ins.clone(),
                &[Value::String(id.into()), Value::String(child.into())],
            )
            .await?;
        }
    }
    Ok(())
}

/// Delete a row the vault deleted, through the table's hook when it has one.
async fn delete_row(host: &Host<'_>, core: &Core, table: &Table, id: &str) -> CmdResult<Deletion> {
    let decision = match table.hooks.on_delete {
        Some(on_delete) => on_delete(host, core, id).await?,
        None => Deletion::Plain,
    };
    if let (Deletion::Plain, Delete::Plain(extra)) = (&decision, &table.spec.delete) {
        delete_plain(&core.db, table, extra, id).await?;
    }
    Ok(decision)
}

/// The spec's own statements, the link rows, then the row itself.
pub async fn delete_plain(
    db: &Pool<Sqlite>,
    table: &Table,
    extra: &[&str],
    id: &str,
) -> CmdResult<()> {
    let spec = &table.spec;
    let id_arg = [Value::String(id.into())];
    for sql in extra.iter() {
        exec(db, sql.to_string(), &id_arg).await?;
    }
    for link in spec.links {
        exec(
            db,
            format!("DELETE FROM {} WHERE {} = ?", link.table, link.parent_col),
            &id_arg,
        )
        .await?;
    }
    exec(
        db,
        format!("DELETE FROM {} WHERE {} = ?", spec.table, spec.pk),
        &id_arg,
    )
    .await?;
    Ok(())
}

/// What became of a remote row.
pub enum Put {
    /// Skipped on a transient condition; the op must be delivered again.
    Retry(String),
    /// The table takes no new rows and has none under this id.
    Skip,
    Done {
        /// The row that holds the values now.
        id: String,
        /// The op's id, when a local row with the same unique key took it in.
        absorbed: Option<String>,
        /// Hash of what the table now holds, so the next collect sees no change.
        synced_hash: String,
    },
}

/// Write one remote row: references to rows that are not here are cleared or
/// waited for, the row is upserted, its link rows replaced. Works on the
/// database and the data directory, without the running app; the hook
/// `before_upsert` reaches the states it needs through `host`.
pub async fn put_row(
    host: &Host<'_>,
    db: &Pool<Sqlite>,
    data_dir: &Path,
    table: &Table,
    id: &str,
    mut payload: Map<String, Value>,
    aliases: &mut HashMap<String, String>,
) -> CmdResult<Put> {
    let entity = table.spec.entity;
    if !sanitize_payload(db, table, &mut payload).await? {
        return Ok(Put::Retry(format!("{entity} {id} waiting for parent")));
    }
    let applied = match upsert(host, db, data_dir, table, id, &mut payload, aliases).await {
        Ok(v) => v,
        Err(e) if is_fk_error(&e) => {
            return Ok(Put::Retry(format!("{entity} {id}: {e}")));
        }
        Err(e) => return Err(e),
    };
    match applied {
        Upserted::Skip => Ok(Put::Skip),
        Upserted::Done { id, absorbed } => {
            apply_links(db, table, &id, &payload).await?;
            let synced_hash = match read_row(db, table, &id).await? {
                Some(local) => payload_hash(&local),
                None => payload_hash(&Value::Object(payload)),
            };
            Ok(Put::Done {
                id,
                absorbed,
                synced_hash,
            })
        }
    }
}

/// The newest op of each row that was not here when the op came, for a
/// table that takes no inserts (`TableSpec::hold`): a row another device
/// brings back later. A pull brings an op once, so it is kept in a setting
/// that is not synced, and applied when the row appears.
struct Held<'t> {
    table: &'t Table,
    key: &'static str,
    by_id: BTreeMap<String, HeldOp>,
    changed: bool,
}

#[derive(Serialize, Deserialize)]
struct HeldOp {
    hlc: String,
    payload: Value,
}

impl<'t> Held<'t> {
    /// What is kept. A value that cannot be read counts as nothing kept.
    async fn load(db: &Pool<Sqlite>, table: &'t Table, key: &'static str) -> Self {
        let by_id = settings::get(db, key)
            .await
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        Self {
            table,
            key,
            by_id,
            changed: false,
        }
    }

    /// Take out the ops of the rows that are here now.
    async fn take_present(&mut self, db: &Pool<Sqlite>) -> CmdResult<Vec<Op>> {
        if self.by_id.is_empty() {
            return Ok(Vec::new());
        }
        let spec = &self.table.spec;
        let ids: Vec<&String> = self.by_id.keys().collect();
        let sql = format!(
            "SELECT {pk} FROM {table} WHERE {pk} IN (SELECT value FROM json_each(?))",
            pk = spec.pk,
            table = spec.table
        );
        let present: Vec<String> = sqlx::query_scalar(AssertSqlSafe(sql))
            .bind(serde_json::to_string(&ids).map_err(AppError::other)?)
            .fetch_all(db)
            .await
            .map_err(AppError::db)?;
        let mut ops = Vec::new();
        for id in present {
            let Some(held) = self.by_id.remove(&id) else {
                continue;
            };
            self.changed = true;
            if let Some(hlc) = Hlc::decode(&held.hlc) {
                ops.push(Op {
                    entity_type: spec.entity.into(),
                    entity_id: id,
                    hlc,
                    deleted: false,
                    payload: held.payload,
                });
            }
        }
        Ok(ops)
    }

    /// Keep `op` unless a newer one is kept for its row.
    fn hold(&mut self, op: &Op) {
        let newer_kept = self
            .by_id
            .get(&op.entity_id)
            .and_then(|held| Hlc::decode(&held.hlc))
            .is_some_and(|hlc| hlc >= op.hlc);
        if newer_kept {
            return;
        }
        self.by_id.insert(
            op.entity_id.clone(),
            HeldOp {
                hlc: op.hlc.encode(),
                payload: op.payload.clone(),
            },
        );
        self.changed = true;
    }

    async fn save(&self, db: &Pool<Sqlite>) -> CmdResult<()> {
        if !self.changed {
            return Ok(());
        }
        if self.by_id.is_empty() {
            return settings::delete(db, self.key).await;
        }
        let json = serde_json::to_string(&self.by_id).map_err(AppError::other)?;
        settings::set(db, self.key, &json).await
    }
}

/// Apply remote row ops of `tables` (LWW by HLC). Parents first so FK checks
/// pass. Hooks reach the states of the modules through `host`.
pub async fn apply_ops(
    host: &Host<'_>,
    core: &Core,
    tables: &[&Table],
    ops: &[Op],
) -> CmdResult<ApplyOutcome> {
    let db = &core.db;
    let mut outcome = ApplyOutcome::default();
    let mut held = Vec::new();
    for table in tables {
        if let Some(key) = table.spec.hold {
            held.push(Held::load(db, table, key).await);
        }
    }
    let mut returned = Vec::new();
    for h in &mut held {
        returned.extend(h.take_present(db).await?);
    }

    let table_of = |entity: &str| tables.iter().find(|t| t.spec.entity == entity).copied();
    // Ops of a type or a setting key this build does not register leave no
    // state behind (spec 9.2, rule 1).
    let mut pending: Vec<&Op> = ops
        .iter()
        .chain(&returned)
        .filter(|op| {
            table_of(&op.entity_type).is_some_and(|t| t.takes(&op.entity_id))
                && valid_id(&op.entity_id)
        })
        .collect();
    pending.sort_by_key(|op| apply_order(tables, op));

    let mut aliases: HashMap<String, String> = HashMap::new();
    for op in pending {
        let table = table_of(&op.entity_type).expect("filtered");
        let spec = &table.spec;
        let id = op.entity_id.as_str();
        let prev = load_row_state(db, spec.entity, id).await?;
        if prev
            .as_ref()
            .and_then(|s| s.head_hlc.as_ref())
            .map(|h| *h >= op.hlc)
            .unwrap_or(false)
        {
            continue;
        }
        let mut st = RowSyncState {
            entity: spec.entity.into(),
            id: id.into(),
            head_hlc: Some(op.hlc.clone()),
            ..Default::default()
        };

        if op.deleted {
            if matches!(spec.delete, Delete::Ignore) {
                continue;
            }
            match delete_row(host, core, table, id).await {
                Ok(Deletion::Later) => {
                    outcome.retry = Some(format!("{} {id} is in use", spec.entity));
                    continue;
                }
                // The row stays; an empty hash makes the next cycle publish it again.
                Ok(Deletion::Keep) => {
                    after_row(core, table, id).await?;
                    st.synced_hash = String::new();
                    save_row_state(db, &st).await?;
                    continue;
                }
                Ok(Deletion::Plain | Deletion::Done) => after_row(core, table, id).await?,
                Err(e) if is_fk_error(&e) => {
                    outcome.retry = Some(format!("{} {id}: {e}", spec.entity));
                    continue;
                }
                Err(e) => return Err(e),
            }
            st.deleted = true;
            save_row_state(db, &st).await?;
            changed(&mut outcome, table, id);
            continue;
        }

        let payload = op.payload.as_object().cloned().unwrap_or_default();
        match put_row(
            host,
            db,
            &core.app_data_dir,
            table,
            id,
            payload,
            &mut aliases,
        )
        .await?
        {
            Put::Retry(reason) => {
                outcome.retry = Some(reason);
                continue;
            }
            Put::Skip => {
                if let Some(h) = held.iter_mut().find(|h| h.table.spec.entity == spec.entity) {
                    h.hold(op);
                }
                continue;
            }
            Put::Done {
                id: applied_id,
                absorbed,
                synced_hash,
            } => {
                after_row(core, table, &applied_id).await?;
                if let Some(old) = &absorbed {
                    after_row(core, table, old).await?;
                }
                st.id = applied_id;
                st.synced_hash = synced_hash;
                save_row_state(db, &st).await?;
                if let Some(old) = absorbed {
                    save_row_state(
                        db,
                        &RowSyncState {
                            entity: spec.entity.into(),
                            id: old,
                            head_hlc: Some(op.hlc.clone()),
                            deleted: true,
                            ..Default::default()
                        },
                    )
                    .await?;
                }
                changed(&mut outcome, table, id);
                outcome
                    .changed
                    .extend(spec.also_changes.iter().map(|e| e.to_string()));
            }
        }
    }
    for h in &held {
        h.save(db).await?;
    }
    Ok(outcome)
}

/// The hook `after_row` of the table, if it has one.
async fn after_row(core: &Core, table: &Table, id: &str) -> CmdResult<()> {
    match table.hooks.after_row {
        Some(after_row) => after_row(core, id).await,
        None => Ok(()),
    }
}

fn changed(outcome: &mut ApplyOutcome, table: &Table, id: &str) {
    outcome.changed.insert(table.spec.entity.into());
    if table.spec.entity == crate::system::SETTING_ENTITY {
        outcome.settings.insert(id.into());
    }
}

/// Once the cycle applied the rows of one stream: the watchers of changed
/// settings, the hooks of changed entities, then the UI hears which stores to
/// reload.
pub async fn finish_apply(host: &Host<'_>, core: &Core, tables: &[&Table], outcome: &ApplyOutcome) {
    for key in &outcome.settings {
        core.settings.changed(key).await;
    }
    for table in tables {
        if let Some(after_apply) = table.hooks.after_apply {
            if outcome.changed.contains(table.spec.entity) {
                after_apply(host).await;
            }
        }
    }
    if !outcome.changed.is_empty() {
        host.emit(
            EVENT_CHANGED,
            outcome.changed.iter().cloned().collect::<Vec<_>>(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Hooks, Plan, Registry, Step, TableSpec};
    use serde_json::json;
    use std::any::{Any, TypeId};
    use veydan_core::{db, BoxFuture, Directory, Schema};

    const THINGS: Schema = Schema {
        module: "things",
        steps: &["CREATE TABLE things (
            id   TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            flag INTEGER NOT NULL DEFAULT 0
        )"],
    };
    const ROWS: &[&str] = &["password_vault", "setting", "label", "thing", "thing_flag"];
    const PLAN: Plan = Plan {
        collect: &[Step::rows("rows", None, "app", ROWS)],
        apply: &[Step::rows("rows", None, "app", ROWS)],
        late: &[],
    };

    /// A thing survives a tombstone while its name says so.
    fn keep_the_kept<'a>(
        _host: &'a Host<'a>,
        core: &'a Core,
        id: &'a str,
    ) -> BoxFuture<'a, CmdResult<Deletion>> {
        Box::pin(async move {
            let name: String = sqlx::query_scalar("SELECT name FROM things WHERE id = ?")
                .bind(id)
                .fetch_one(&core.db)
                .await
                .map_err(AppError::db)?;
            Ok(if name == "kept" {
                Deletion::Keep
            } else {
                Deletion::Plain
            })
        })
    }

    /// What the owner of things heard of the rows sync wrote or removed, in
    /// a setting: the ids, with whether the row is there.
    fn heard<'a>(core: &'a Core, id: &'a str) -> BoxFuture<'a, CmdResult<()>> {
        Box::pin(async move {
            let here: Option<i64> = sqlx::query_scalar("SELECT 1 FROM things WHERE id = ?")
                .bind(id)
                .fetch_optional(&core.db)
                .await
                .map_err(AppError::db)?;
            let line = format!("{id}:{}", if here.is_some() { "here" } else { "gone" });
            let mut seen = settings::get(&core.db, "heard").await.unwrap_or_default();
            seen.push_str(&line);
            seen.push(' ');
            settings::set(&core.db, "heard", &seen).await
        })
    }

    fn things(registry: &mut Registry) {
        registry.setting("thing_key");
        registry.table(
            TableSpec::plain("thing", "things", &["name"]),
            Hooks {
                on_delete: Some(keep_the_kept),
                after_row: Some(heard),
                ..Hooks::default()
            },
        );
        registry.table(
            TableSpec {
                insert: false,
                delete: Delete::Ignore,
                hold: Some("held_flags"),
                ..TableSpec::plain("thing_flag", "things", &["flag"])
            },
            Hooks::default(),
        );
    }

    struct NoStates;

    impl crate::States for NoStates {
        fn state(&self, _: TypeId) -> Option<&(dyn Any + Send + Sync)> {
            None
        }
    }

    async fn setup() -> (Core, Registry, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let schemas = [veydan_core::SCHEMA, crate::SCHEMA, THINGS];
        let pool = db::open(&dir.path().join(db::DB_FILE), &schemas)
            .await
            .unwrap();
        let mut registry = Registry::new();
        registry.add("things", things);
        let registry = registry.finish(&PLAN).unwrap();
        (
            Core::new(pool, dir.path().to_owned(), Directory::default()),
            registry,
            dir,
        )
    }

    fn op(entity: &str, id: &str, wall_ms: u64, payload: Option<Value>) -> Op {
        Op {
            entity_type: entity.into(),
            entity_id: id.into(),
            hlc: Hlc {
                wall_ms,
                counter: 0,
                device_id: "peer".into(),
            },
            deleted: payload.is_none(),
            payload: payload.unwrap_or_else(|| json!({})),
        }
    }

    async fn apply(core: &Core, registry: &Registry, ops: &[Op]) -> ApplyOutcome {
        let host = Host::States(&NoStates);
        apply_ops(&host, core, &registry.rows(ROWS), ops)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn an_op_for_a_row_that_is_not_here_waits_for_the_row() {
        let (core, registry, _dir) = setup().await;
        let db = &core.db;
        let flag = op("thing_flag", "t-1", 1000, Some(json!({ "flag": 1 })));
        let outcome = apply(&core, &registry, std::slice::from_ref(&flag)).await;
        assert!(outcome.changed.is_empty());
        assert!(load_row_state(db, "thing_flag", "t-1")
            .await
            .unwrap()
            .is_none());
        assert!(settings::get(db, "held_flags").await.is_some());

        sqlx::query("INSERT INTO things (id, name) VALUES ('t-1', 'back')")
            .execute(db)
            .await
            .unwrap();
        let outcome = apply(&core, &registry, &[]).await;
        assert!(outcome.changed.contains("thing_flag"));
        let stored: i64 = sqlx::query_scalar("SELECT flag FROM things WHERE id = 't-1'")
            .fetch_one(db)
            .await
            .unwrap();
        assert_eq!(stored, 1);
        assert_eq!(settings::get(db, "held_flags").await, None);
        let st = load_row_state(db, "thing_flag", "t-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(st.head_hlc, Some(flag.hlc));
    }

    /// Spec 9.2, rule 1: an op of a type or a setting key no module
    /// registered leaves neither a row nor a state; a tombstone would follow.
    #[cfg(desktop)]
    #[tokio::test]
    async fn what_nobody_registered_leaves_no_trace() {
        let (core, registry, _dir) = setup().await;
        let db = &core.db;
        let ops = [
            op("setting", "thing_key", 1000, Some(json!({ "value": "1" }))),
            op("setting", "other_key", 1000, Some(json!({ "value": "1" }))),
            op("stranger", "s-1", 1000, Some(json!({ "name": "x" }))),
        ];
        let outcome = apply(&core, &registry, &ops).await;

        assert_eq!(
            outcome.settings.into_iter().collect::<Vec<_>>(),
            ["thing_key"]
        );
        assert_eq!(settings::get(db, "thing_key").await.as_deref(), Some("1"));
        assert_eq!(settings::get(db, "other_key").await, None);
        for (entity, id) in [("setting", "other_key"), ("stranger", "s-1")] {
            assert!(load_row_state(db, entity, id).await.unwrap().is_none());
        }
        let mut clock = HlcClock::new("here", None);
        let changes = collect_local_changes(
            &core,
            &mut clock,
            &registry.rows(&["setting", "thing"]),
            None,
        )
        .await
        .unwrap();
        assert!(changes.ops.is_empty(), "{:?}", changes.ops);
    }

    /// Spec 9.2, rule 1, on the side of the push: a setting key the registry
    /// took once and no longer registers keeps its value here and is not
    /// deleted on every other device.
    #[cfg(desktop)]
    #[tokio::test]
    async fn a_key_no_longer_registered_is_not_deleted_everywhere() {
        let (core, registry, _dir) = setup().await;
        apply(
            &core,
            &registry,
            &[op(
                "setting",
                "thing_key",
                1000,
                Some(json!({ "value": "1" })),
            )],
        )
        .await;
        let mut narrower = Registry::new();
        narrower.add("things", |registry| {
            registry.table(
                TableSpec::plain("thing", "things", &["name"]),
                Hooks::default(),
            )
        });
        const NARROWER: Plan = Plan {
            collect: &[Step::rows("rows", None, "app", ROWS)],
            apply: &[Step::rows("rows", None, "app", ROWS)],
            late: &[],
        };
        let narrower = narrower.finish(&NARROWER).unwrap();

        let mut clock = HlcClock::new("here", None);
        let changes = collect_local_changes(&core, &mut clock, &narrower.rows(&["setting"]), None)
            .await
            .unwrap();
        assert!(changes.ops.is_empty(), "{:?}", changes.ops);
        assert_eq!(
            settings::get(&core.db, "thing_key").await.as_deref(),
            Some("1")
        );
    }

    #[tokio::test]
    async fn a_row_its_hook_keeps_is_published_again() {
        let (core, registry, _dir) = setup().await;
        let db = &core.db;
        let puts = [
            op("thing", "t-1", 1000, Some(json!({ "name": "kept" }))),
            op("thing", "t-2", 1000, Some(json!({ "name": "gone" }))),
        ];
        apply(&core, &registry, &puts).await;
        let deletes = [
            op("thing", "t-1", 2000, None),
            op("thing", "t-2", 2000, None),
        ];
        let outcome = apply(&core, &registry, &deletes).await;

        assert_eq!(outcome.changed.into_iter().collect::<Vec<_>>(), ["thing"]);
        let table = registry.table_of("thing").unwrap();
        assert!(read_row(db, table, "t-1").await.unwrap().is_some());
        assert!(read_row(db, table, "t-2").await.unwrap().is_none());
        let kept = load_row_state(db, "thing", "t-1").await.unwrap().unwrap();
        assert!(!kept.deleted && kept.synced_hash.is_empty());

        let mut clock = HlcClock::new("here", None);
        let changes = collect_local_changes(&core, &mut clock, &registry.rows(&["thing"]), None)
            .await
            .unwrap();
        let pushed: Vec<_> = changes
            .ops
            .iter()
            .map(|op| (op.entity_type.as_str(), op.entity_id.as_str(), op.deleted))
            .collect();
        assert_eq!(pushed, [("thing", "t-1", false)]);
    }

    #[tokio::test]
    async fn the_owner_hears_of_each_row_sync_wrote_or_removed() {
        let (core, registry, _dir) = setup().await;
        let puts = [
            op("thing", "t-1", 1000, Some(json!({ "name": "kept" }))),
            op("thing", "t-2", 1000, Some(json!({ "name": "gone" }))),
        ];
        apply(&core, &registry, &puts).await;
        // Older than what is here: not applied, not heard.
        apply(
            &core,
            &registry,
            &[op("thing", "t-1", 500, Some(json!({ "name": "old" })))],
        )
        .await;
        let deletes = [
            op("thing", "t-1", 2000, None),
            op("thing", "t-2", 2000, None),
        ];
        apply(&core, &registry, &deletes).await;
        assert_eq!(
            settings::get(&core.db, "heard").await.as_deref(),
            Some("t-1:here t-2:here t-1:here t-2:gone ")
        );
    }

    /// Spec 10.2: a label is taken as its key names it, and what a device
    /// only took from sync it does not publish again.
    #[tokio::test]
    async fn a_label_names_what_its_key_names_and_is_not_sent_back() {
        let (core, registry, _dir) = setup().await;
        let label = registry.table_of("label").unwrap();
        let ops = [
            op(
                "label",
                "workspace:w-1",
                1000,
                Some(json!({ "kind": "profile", "id": "p-9", "name": null, "color": "#f80" })),
            ),
            op(
                "label",
                "profile:p-1",
                1000,
                Some(json!({ "kind": "profile", "id": "p-1", "name": "Shop",
                             "parent_kind": "workspace", "parent_id": "w-1", "color": null })),
            ),
        ];
        let host = Host::States(&NoStates);
        let outcome = apply_ops(&host, &core, &[label], &ops).await.unwrap();
        assert!(outcome.changed.contains("label"));
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT key || ' ' || kind || ' ' || id || ' ' || name || ' ' ||
                    coalesce(parent_kind, '-') || ' ' || coalesce(parent_id, '-') || ' ' ||
                    coalesce(color, '-')
             FROM labels ORDER BY key",
        )
        .fetch_all(&core.db)
        .await
        .unwrap();
        assert_eq!(
            rows,
            [
                "profile:p-1 profile p-1 Shop workspace w-1 -",
                "workspace:w-1 workspace w-1  - - #f80",
            ]
        );

        let mut clock = HlcClock::new("here", None);
        let changes = collect_local_changes(&core, &mut clock, &[label], None)
            .await
            .unwrap();
        assert!(changes.ops.is_empty(), "{:?}", changes.ops);

        apply_ops(
            &host,
            &core,
            &[label],
            &[op("label", "profile:p-1", 2000, None)],
        )
        .await
        .unwrap();
        let keys: Vec<String> = sqlx::query_scalar("SELECT key FROM labels")
            .fetch_all(&core.db)
            .await
            .unwrap();
        assert_eq!(keys, ["workspace:w-1"]);
        let changes = collect_local_changes(&core, &mut clock, &[label], None)
            .await
            .unwrap();
        assert!(changes.ops.is_empty(), "{:?}", changes.ops);
    }
}
