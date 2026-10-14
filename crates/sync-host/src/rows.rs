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
/// The colon stays for the ids of `label` (`kind:id`); an id of dots alone
/// (`.`, `..`) is refused: a hook that joins the id to a folder would climb.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
        && !id.chars().all(|c| c == '.')
}

// ── Reading rows ─────────────────────────────────────────────────────────────

/// `SELECT pk, json_object('c', c, ..) FROM table`: SQLite keeps the column types.
fn select_sql(table: &Table) -> String {
    select_columns(table, table.spec.columns)
}

/// The same over `columns`, which may be a set the table synced before.
fn select_columns(table: &Table, columns: &[&str]) -> String {
    let spec = &table.spec;
    let pairs: Vec<String> = columns.iter().map(|c| format!("'{c}', {c}")).collect();
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

/// Rows matching `select` with the arrays of `links` folded in, as `(id, payload)`.
async fn read_payloads(
    db: &Pool<Sqlite>,
    select: String,
    links: &[&LinkSpec],
    id: Option<&str>,
) -> CmdResult<Vec<(String, Value)>> {
    let mut q = sqlx::query_as::<_, (String, String)>(AssertSqlSafe(select));
    if let Some(id) = id {
        q = q.bind(id.to_string());
    }
    let rows = q.fetch_all(db).await.map_err(AppError::db)?;
    let mut maps: Vec<(&LinkSpec, HashMap<String, Vec<String>>)> = Vec::new();
    for link in links {
        maps.push((link, link_map(db, link, id).await?));
    }
    Ok(rows
        .into_iter()
        .map(|(row_id, json)| {
            let mut payload = parse_object(&json);
            for (link, map) in &maps {
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
    read_payloads(db, select_sql(table), &all_links(table), None).await
}

fn all_links(table: &Table) -> Vec<&LinkSpec> {
    table.spec.links.iter().collect()
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
    Ok(read_payloads(db, sql, &all_links(table), Some(id))
        .await?
        .pop()
        .map(|(_, v)| v))
}

/// serde_json sorts object keys, so equal rows hash equal.
pub(crate) fn payload_hash(payload: &Value) -> String {
    sha256_hex(payload.to_string().as_bytes())
}

// ── The shape of a row ───────────────────────────────────────────────────────

/// The local setting that names the shape the states of `entity` were hashed by.
fn shape_key(entity: &str) -> String {
    format!("sync_row_shape:{entity}")
}

/// What the hash of a row is made of: the synced columns and the link keys,
/// each sorted, as `col,col;key,key`.
fn row_shape(table: &Table) -> String {
    let mut columns: Vec<&str> = table.spec.columns.to_vec();
    columns.sort_unstable();
    let mut keys: Vec<&str> = table.spec.links.iter().map(|l| l.key).collect();
    keys.sort_unstable();
    format!("{};{}", columns.join(","), keys.join(","))
}

/// The columns and links to read the rows by `shape`; `None` when the rows
/// cannot be read so: a name that is not an identifier, a link this build
/// does not know how to read.
fn shape_parts<'a>(table: &'a Table, shape: &'a str) -> Option<(Vec<&'a str>, Vec<&'a LinkSpec>)> {
    let (columns, keys) = shape.split_once(';')?;
    let names = |list: &'a str| list.split(',').filter(|n| !n.is_empty());
    let columns: Vec<&str> = names(columns).collect();
    if !columns
        .iter()
        .all(|c| c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_'))
    {
        return None;
    }
    let links = names(keys)
        .map(|key| table.spec.links.iter().find(|l| l.key == key))
        .collect::<Option<Vec<_>>>()?;
    Some((columns, links))
}

/// The states of a table hold the hash of each row by the columns and link
/// keys it synced then. A build that syncs another set (a new column) would
/// see every row as changed and publish all of them with new HLCs, over the
/// edits of the devices that did not update yet. So when the set differs from
/// the one kept, a state whose row still hashes as its `synced_hash` by the
/// old set takes the hash by the new one without an op: a new column with a
/// default changes nothing; a row changed since goes out as before. The set
/// is local and kept per entity; the vault format (spec 9.5) does not change.
async fn adopt_row_shape(db: &Pool<Sqlite>, table: &Table) -> CmdResult<()> {
    let entity = table.spec.entity;
    let key = shape_key(entity);
    let shape = row_shape(table);
    let kept = settings::get(db, &key).await;
    if kept.as_deref() == Some(shape.as_str()) {
        return Ok(());
    }
    // No set kept: the states were hashed by this build's set.
    if let Some(old) = kept.as_deref() {
        match shape_parts(table, old) {
            Some((columns, links)) => {
                match read_payloads(db, select_columns(table, &columns), &links, None).await {
                    Ok(before) => rehash_unchanged(db, table, before).await?,
                    Err(e) => eprintln!("sync: {entity} cannot be read as it synced before: {e}"),
                }
            }
            None => eprintln!("sync: {entity} cannot be read as it synced before ({old})"),
        }
    }
    settings::set(db, &key, &shape).await
}

/// The new hash for each state whose row `before` (read by the old set)
/// still matches what was synced.
async fn rehash_unchanged(
    db: &Pool<Sqlite>,
    table: &Table,
    before: Vec<(String, Value)>,
) -> CmdResult<()> {
    let before: HashMap<String, String> = before
        .into_iter()
        .map(|(id, payload)| (id, payload_hash(&payload)))
        .collect();
    let now: HashMap<String, String> = read_rows(db, table)
        .await?
        .into_iter()
        .map(|(id, payload)| (id, payload_hash(&payload)))
        .collect();
    for (id, mut st) in load_row_states(db, table.spec.entity).await? {
        if st.deleted || before.get(&id) != Some(&st.synced_hash) {
            continue;
        }
        if let Some(hash) = now.get(&id).filter(|h| **h != st.synced_hash) {
            st.synced_hash = hash.clone();
            save_row_state(db, &st).await?;
        }
    }
    Ok(())
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
        adopt_row_shape(db, table).await?;
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
        /// The twin is newer than the op and kept its values.
        kept: bool,
    },
    Skip,
}

/// Whether the local row `twin`, which takes in an op of another id under
/// the same unique key, is newer than the op (LWW, audit A2-16): its last op
/// is not older, or it was edited here since it was last synced (the next
/// collect publishes it). A row with no state was never synced and takes
/// the op, as before.
async fn twin_is_newer(db: &Pool<Sqlite>, table: &Table, twin: &str, hlc: &Hlc) -> CmdResult<bool> {
    let Some(st) = load_row_state(db, table.spec.entity, twin).await? else {
        return Ok(false);
    };
    if st.head_hlc.as_ref().is_some_and(|head| head >= hlc) {
        return Ok(true);
    }
    Ok(!st.deleted && changed_here(db, table, &st).await?)
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
/// With the op's `hlc`, a twin takes the op's values only when the op is
/// newer ([`twin_is_newer`]); without one (a write that is not an op's), it
/// always does and the id that sorts first wins.
#[allow(clippy::too_many_arguments)]
async fn upsert(
    host: &Host<'_>,
    db: &Pool<Sqlite>,
    data_dir: &Path,
    table: &Table,
    id: &str,
    payload: &mut Map<String, Value>,
    aliases: &mut HashMap<String, String>,
    hlc: Option<&Hlc>,
) -> CmdResult<Upserted> {
    let spec = &table.spec;
    // Same unique key under another id: keep that row, absorb the remote id.
    let mut twin = None;
    let mut newer: Option<(String, bool)> = None;
    if let Some(keys) = spec.unique {
        if let Some(vals) = unique_vals(keys, payload) {
            if let Some(other_id) = find_unique_other(db, table, keys, &vals, id).await? {
                let remote_wins = match hlc {
                    Some(hlc) => {
                        let kept = twin_is_newer(db, table, &other_id, hlc).await?;
                        newer = Some((other_id.clone(), kept));
                        !kept
                    }
                    None => id < other_id.as_str(),
                };
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
        // The twin a hook found was not compared yet.
        let kept = match (hlc, newer) {
            (Some(_), Some((checked, kept))) if checked == twin.id => kept,
            (Some(hlc), _) => twin_is_newer(db, table, &twin.id, hlc).await?,
            (None, _) => false,
        };
        if !kept {
            let skip: HashSet<&str> = keys.iter().copied().collect();
            let rest: Vec<(&str, Value)> = cols
                .into_iter()
                .filter(|(c, _)| !skip.contains(c))
                .collect();
            update_row(db, table, &twin.id, &rest).await?;
        }
        return Ok(Upserted::Done {
            id: twin.id,
            absorbed: Some(id.into()),
            kept,
        });
    }

    if !spec.insert {
        return if update_row(db, table, id, &cols).await? {
            Ok(Upserted::Done {
                id: id.into(),
                absorbed: None,
                kept: false,
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
        kept: false,
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
pub(crate) async fn delete_row(
    host: &Host<'_>,
    core: &Core,
    table: &Table,
    id: &str,
) -> CmdResult<Deletion> {
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
        /// The local row that took the op's id in is newer than the op and
        /// kept its values (LWW); its state stays as it is.
        kept: bool,
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
    payload: Map<String, Value>,
    aliases: &mut HashMap<String, String>,
) -> CmdResult<Put> {
    put_row_at(host, db, data_dir, table, id, payload, aliases, None).await
}

/// [`put_row`] of an op stamped `hlc`: a local twin newer than it keeps
/// its values ([`twin_is_newer`]).
#[allow(clippy::too_many_arguments)]
async fn put_row_at(
    host: &Host<'_>,
    db: &Pool<Sqlite>,
    data_dir: &Path,
    table: &Table,
    id: &str,
    mut payload: Map<String, Value>,
    aliases: &mut HashMap<String, String>,
    hlc: Option<&Hlc>,
) -> CmdResult<Put> {
    let entity = table.spec.entity;
    if !sanitize_payload(db, table, &mut payload).await? {
        return Ok(Put::Retry(format!("{entity} {id} waiting for parent")));
    }
    let applied = match upsert(host, db, data_dir, table, id, &mut payload, aliases, hlc).await {
        Ok(v) => v,
        Err(e) if is_fk_error(&e) => {
            return Ok(Put::Retry(format!("{entity} {id}: {e}")));
        }
        Err(e) => return Err(e),
    };
    match applied {
        Upserted::Skip => Ok(Put::Skip),
        Upserted::Done { id, absorbed, kept } => {
            if !kept {
                apply_links(db, table, &id, &payload).await?;
            }
            let synced_hash = match read_row(db, table, &id).await? {
                Some(local) => payload_hash(&local),
                None => payload_hash(&Value::Object(payload)),
            };
            Ok(Put::Done {
                id,
                absorbed,
                synced_hash,
                kept,
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

/// The rows `ops` would write that were edited here since they were last
/// pushed or applied: the edits made after the collect of the cycle.
#[derive(Default)]
pub struct Edited(HashSet<(String, String)>);

impl Edited {
    fn has(&self, entity: &str, id: &str) -> bool {
        self.0.contains(&(entity.to_string(), id.to_string()))
    }
}

/// Find the [`Edited`] rows among those of `tables` that `ops` would write.
/// Read before any op of the pull is written, so that what an op does to
/// another row (a deleted folder clears the parent of its children, a
/// handler writes a column) is not taken for an edit. Only the tables that
/// publish count: a mirrored row is never published from here.
pub async fn edited_rows(db: &Pool<Sqlite>, tables: &[&Table], ops: &[Op]) -> CmdResult<Edited> {
    let mut checked = HashSet::new();
    let mut edited = HashSet::new();
    for op in ops {
        let Some(table) = tables
            .iter()
            .find(|t| t.spec.entity == op.entity_type && t.push)
        else {
            continue;
        };
        let key = (op.entity_type.clone(), op.entity_id.clone());
        if checked.contains(&key) || !table.takes(&op.entity_id) || !valid_id(&op.entity_id) {
            continue;
        }
        let Some(st) = load_row_state(db, table.spec.entity, &op.entity_id).await? else {
            checked.insert(key);
            continue;
        };
        // An op the row is newer than writes nothing; a later op of the
        // same row may.
        if st.head_hlc.as_ref().is_some_and(|h| *h >= op.hlc) {
            continue;
        }
        if !st.deleted && changed_here(db, table, &st).await? {
            edited.insert(key.clone());
        }
        checked.insert(key);
    }
    Ok(Edited(edited))
}

/// Apply remote row ops of `tables` (LWW by HLC). Parents first so FK checks
/// pass. Hooks reach the states of the modules through `host`. A row edited
/// here since the last push or apply keeps its value ([`edited_rows`]).
pub async fn apply_ops(
    host: &Host<'_>,
    core: &Core,
    tables: &[&Table],
    ops: &[Op],
) -> CmdResult<ApplyOutcome> {
    let edited = edited_rows(&core.db, tables, ops).await?;
    apply_ops_except(host, core, tables, ops, &edited).await
}

/// [`apply_ops`] with the edited rows found before: a cycle finds them once,
/// before the first of its apply steps writes anything.
pub async fn apply_ops_except(
    host: &Host<'_>,
    core: &Core,
    tables: &[&Table],
    ops: &[Op],
    edited: &Edited,
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
    let mut twins: BTreeMap<String, String> =
        settings::get_json(db, TWINS_KEY).await.unwrap_or_default();
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
        // A row edited here since it was last pushed or applied keeps its
        // value: the next collect publishes it with a newer HLC, which wins
        // on every device (LWW). Written over, the edit would be lost
        // without an op. A tombstone waits the same way.
        if edited.has(spec.entity, id) {
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
            // The tombstone of an id a local row took in under its unique
            // key is that row's (audit A2-16).
            let alias = twin_key(spec.entity, id);
            if let Some(twin) = twins.get(&alias).cloned() {
                if read_row(db, table, id).await?.is_none() {
                    match delete_twin(host, core, table, &twin, &op.hlc).await? {
                        TwinTombstone::Retry(reason) => {
                            outcome.retry = Some(reason);
                            continue;
                        }
                        TwinTombstone::Kept => {}
                        TwinTombstone::Deleted => changed(&mut outcome, table, &twin),
                    }
                    st.deleted = true;
                    save_row_state(db, &st).await?;
                    twins.remove(&alias);
                    save_twins(db, &twins).await?;
                    continue;
                }
                twins.remove(&alias);
                save_twins(db, &twins).await?;
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
        match put_row_at(
            host,
            db,
            &core.app_data_dir,
            table,
            id,
            payload,
            &mut aliases,
            Some(&op.hlc),
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
                kept,
            } => {
                after_row(core, table, &applied_id).await?;
                if let Some(old) = &absorbed {
                    after_row(core, table, old).await?;
                }
                // A twin newer than the op keeps its values and its state.
                if !kept {
                    st.id = applied_id.clone();
                    st.synced_hash = synced_hash;
                    save_row_state(db, &st).await?;
                }
                // Recorded before the state of the alias, op by op: an apply
                // that stops halfway leaves no alias whose twin is unknown.
                match &absorbed {
                    Some(old) => {
                        twins.insert(twin_key(spec.entity, old), applied_id);
                        save_twins(db, &twins).await?;
                    }
                    None => {
                        if twins.remove(&twin_key(spec.entity, &applied_id)).is_some() {
                            save_twins(db, &twins).await?;
                        }
                    }
                }
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

/// Write what [`TWINS_KEY`] holds.
async fn save_twins(db: &Pool<Sqlite>, twins: &BTreeMap<String, String>) -> CmdResult<()> {
    if twins.is_empty() {
        settings::delete(db, TWINS_KEY).await
    } else {
        settings::set_json(db, TWINS_KEY, twins).await
    }
}

/// The ids a local row took in under its unique key ([`Twin`]): `<entity>/<id>`
/// of the op's id → the id of the row. A tombstone of such an id deletes
/// that row (audit A2-16). Device-local, cleared with the binding.
pub const TWINS_KEY: &str = "sync_twins";

fn twin_key(entity: &str, id: &str) -> String {
    format!("{entity}/{id}")
}

/// What a tombstone of an id `twin` took in did to `twin`.
enum TwinTombstone {
    /// Not now (the row is in use, a reference fails): the op comes again.
    Retry(String),
    /// The twin stays: it is newer, already gone, or its hook keeps it
    /// (then the next collect publishes it again).
    Kept,
    /// The twin is deleted. Its state stays without `deleted` and with the
    /// tombstone's HLC, so the next collect publishes its own tombstone for
    /// the devices that know it by its id.
    Deleted,
}

/// The tombstone of an id `twin` took in, stamped `hlc`, applied to `twin`
/// as if it were its own: LWW against its state, through the table's hook.
async fn delete_twin(
    host: &Host<'_>,
    core: &Core,
    table: &Table,
    twin: &str,
    hlc: &Hlc,
) -> CmdResult<TwinTombstone> {
    let db = &core.db;
    let entity = table.spec.entity;
    let Some(mut st) = load_row_state(db, entity, twin).await? else {
        return Ok(TwinTombstone::Kept);
    };
    if st.deleted
        || read_row(db, table, twin).await?.is_none()
        || twin_is_newer(db, table, twin, hlc).await?
    {
        return Ok(TwinTombstone::Kept);
    }
    match delete_row(host, core, table, twin).await {
        Ok(Deletion::Later) => Ok(TwinTombstone::Retry(format!("{entity} {twin} is in use"))),
        Ok(Deletion::Keep) => {
            after_row(core, table, twin).await?;
            st.synced_hash = String::new();
            save_row_state(db, &st).await?;
            Ok(TwinTombstone::Kept)
        }
        Ok(Deletion::Plain | Deletion::Done) => {
            after_row(core, table, twin).await?;
            st.head_hlc = Some(hlc.clone());
            save_row_state(db, &st).await?;
            Ok(TwinTombstone::Deleted)
        }
        Err(e) if is_fk_error(&e) => Ok(TwinTombstone::Retry(format!("{entity} {twin}: {e}"))),
        Err(e) => Err(e),
    }
}

/// Whether the next collect would publish the row of `st`: its synced columns
/// differ from what was last pushed or applied (an empty hash, left by a
/// kept tombstone, always does), or it is gone and the table publishes
/// tombstones.
async fn changed_here(db: &Pool<Sqlite>, table: &Table, st: &RowSyncState) -> CmdResult<bool> {
    Ok(match read_row(db, table, &st.id).await? {
        Some(row) => payload_hash(&row) != st.synced_hash,
        None => !matches!(table.spec.delete, Delete::Ignore),
    })
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
        steps: &[
            "CREATE TABLE things (
            id   TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            flag INTEGER NOT NULL DEFAULT 0
        )",
            "CREATE TABLE tags (
            id    TEXT PRIMARY KEY NOT NULL,
            name  TEXT NOT NULL UNIQUE,
            color TEXT NOT NULL DEFAULT ''
        )",
        ],
    };
    const ROWS: &[&str] = &[
        "password_vault",
        "setting",
        "label",
        "thing",
        "thing_flag",
        "tag",
    ];
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
        // A tag is its name: one of another id with the same name is a twin.
        registry.table(
            TableSpec {
                unique: Some(&["name"]),
                ..TableSpec::plain("tag", "tags", &["name", "color"])
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

    /// An id of dots alone reaches neither a row nor a hook: a hook that
    /// joins `..` to a folder names the folder above it. The colon of the
    /// ids of `label` stays.
    #[tokio::test]
    async fn an_id_of_dots_alone_is_not_applied() {
        let (core, registry, _dir) = setup().await;
        let db = &core.db;
        let ops = [
            op("thing", "..", 1000, Some(json!({ "name": "up" }))),
            op("thing", ".", 1001, Some(json!({ "name": "here" }))),
            op("thing", "...", 1002, Some(json!({ "name": "dots" }))),
            op("thing", "..", 1003, None),
        ];
        let outcome = apply(&core, &registry, &ops).await;
        assert!(outcome.changed.is_empty());
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM things")
            .fetch_one(db)
            .await
            .unwrap();
        assert_eq!(rows, 0);
        for id in ["..", ".", "..."] {
            assert!(load_row_state(db, "thing", id).await.unwrap().is_none());
        }
        assert_eq!(settings::get(db, "heard").await, None);

        assert!(["", ".", "..", "..."].iter().all(|id| !valid_id(id)));
        assert!(["a.b", "..a", "tag:t-1", "default"]
            .iter()
            .all(|id| valid_id(id)));
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

    /// A build that syncs one more column (with a default) publishes no row
    /// nobody changed, and still the row changed here before it came.
    #[tokio::test]
    async fn a_new_synced_column_republishes_no_row() {
        let (core, registry, _dir) = setup().await;
        let db = &core.db;
        let puts = [
            op("thing", "t-1", 1000, Some(json!({ "name": "one" }))),
            op("thing", "t-2", 1000, Some(json!({ "name": "two" }))),
        ];
        apply(&core, &registry, &puts).await;
        let mut clock = HlcClock::new("here", None);
        let changes = collect_local_changes(&core, &mut clock, &registry.rows(&["thing"]), None)
            .await
            .unwrap();
        assert!(changes.ops.is_empty(), "{:?}", changes.ops);
        assert_eq!(
            settings::get(db, "sync_row_shape:thing").await.as_deref(),
            Some("name;")
        );

        sqlx::query("UPDATE things SET name = 'two, edited' WHERE id = 't-2'")
            .execute(db)
            .await
            .unwrap();
        sqlx::query("ALTER TABLE things ADD COLUMN color TEXT NOT NULL DEFAULT 'grey'")
            .execute(db)
            .await
            .unwrap();
        let mut wider = Registry::new();
        wider.add("things", |registry| {
            registry.table(
                TableSpec::plain("thing", "things", &["name", "color"]),
                Hooks::default(),
            )
        });
        let wider = wider.finish(&PLAN).unwrap();
        let tables = wider.rows(&["thing"]);

        let changes = collect_local_changes(&core, &mut clock, &tables, None)
            .await
            .unwrap();
        let pushed: Vec<_> = changes
            .ops
            .iter()
            .map(|op| (op.entity_id.as_str(), op.payload.clone()))
            .collect();
        assert_eq!(
            pushed,
            [("t-2", json!({ "name": "two, edited", "color": "grey" }))]
        );
        assert_eq!(
            settings::get(db, "sync_row_shape:thing").await.as_deref(),
            Some("color,name;")
        );
        for st in &changes.states {
            save_row_state(db, st).await.unwrap();
        }
        let changes = collect_local_changes(&core, &mut clock, &tables, None)
            .await
            .unwrap();
        assert!(changes.ops.is_empty(), "{:?}", changes.ops);

        // The new column changes like any other once it is synced.
        sqlx::query("UPDATE things SET color = 'red' WHERE id = 't-1'")
            .execute(db)
            .await
            .unwrap();
        let changes = collect_local_changes(&core, &mut clock, &tables, None)
            .await
            .unwrap();
        let pushed: Vec<_> = changes.ops.iter().map(|op| op.entity_id.as_str()).collect();
        assert_eq!(pushed, ["t-1"]);
    }

    /// An edit made after the collect, before the apply of the same cycle,
    /// is neither written over by a newer remote op nor deleted by a
    /// tombstone: the next collect publishes it. A row nobody changed here
    /// takes the op as before.
    #[tokio::test]
    async fn a_row_changed_since_the_collect_is_not_overwritten() {
        let (core, registry, _dir) = setup().await;
        let db = &core.db;
        let puts = [
            op("thing", "t-1", 1000, Some(json!({ "name": "one" }))),
            op("thing", "t-2", 1000, Some(json!({ "name": "two" }))),
            op("thing", "t-3", 1000, Some(json!({ "name": "three" }))),
            op("thing", "t-4", 1000, Some(json!({ "name": "four" }))),
        ];
        apply(&core, &registry, &puts).await;
        for statement in [
            "UPDATE things SET name = 'one, mine' WHERE id = 't-1'",
            "UPDATE things SET name = 'two, mine' WHERE id = 't-2'",
            "DELETE FROM things WHERE id = 't-4'",
        ] {
            sqlx::query(statement).execute(db).await.unwrap();
        }
        let newer = [
            op("thing", "t-1", 2000, Some(json!({ "name": "one, theirs" }))),
            op("thing", "t-2", 2000, None),
            op(
                "thing",
                "t-3",
                2000,
                Some(json!({ "name": "three, theirs" })),
            ),
            op(
                "thing",
                "t-4",
                2000,
                Some(json!({ "name": "four, theirs" })),
            ),
        ];
        let outcome = apply(&core, &registry, &newer).await;
        assert_eq!(outcome.retry, None);

        let names: Vec<(String, String)> =
            sqlx::query_as("SELECT id, name FROM things ORDER BY id")
                .fetch_all(db)
                .await
                .unwrap();
        let names: Vec<(&str, &str)> = names
            .iter()
            .map(|(id, name)| (id.as_str(), name.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("t-1", "one, mine"),
                ("t-2", "two, mine"),
                ("t-3", "three, theirs")
            ]
        );
        let kept = load_row_state(db, "thing", "t-1").await.unwrap().unwrap();
        assert_eq!(kept.head_hlc.map(|h| h.wall_ms), Some(1000));

        let mut clock = HlcClock::new("here", None);
        let changes = collect_local_changes(&core, &mut clock, &registry.rows(&["thing"]), None)
            .await
            .unwrap();
        let mut pushed: Vec<_> = changes
            .ops
            .iter()
            .map(|op| (op.entity_id.as_str(), op.deleted, op.payload.clone()))
            .collect();
        pushed.sort_by(|a, b| a.0.cmp(b.0));
        assert_eq!(
            pushed,
            [
                ("t-1", false, json!({ "name": "one, mine" })),
                ("t-2", false, json!({ "name": "two, mine" })),
                ("t-4", true, json!({})),
            ]
        );
        assert!(changes.ops.iter().all(|op| op.hlc.wall_ms > 2000));
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

    /// A local tag `t-here` named "work", synced at `wall_ms`.
    async fn local_tag(core: &Core, registry: &Registry, color: &str, wall_ms: u64) {
        let db = &core.db;
        sqlx::query("INSERT INTO tags (id, name, color) VALUES ('t-here', 'work', ?)")
            .bind(color)
            .execute(db)
            .await
            .unwrap();
        let table = registry.table_of("tag").unwrap();
        let row = read_row(db, table, "t-here").await.unwrap().unwrap();
        save_row_state(
            db,
            &RowSyncState {
                entity: "tag".into(),
                id: "t-here".into(),
                head_hlc: Some(Hlc {
                    wall_ms,
                    counter: 0,
                    device_id: "here".into(),
                }),
                synced_hash: payload_hash(&row),
                deleted: false,
            },
        )
        .await
        .unwrap();
    }

    async fn tag_color(core: &Core) -> Option<String> {
        sqlx::query_scalar("SELECT color FROM tags WHERE id = 't-here'")
            .fetch_optional(&core.db)
            .await
            .unwrap()
    }

    /// Audit A2-16: a row of another id with the same unique key writes over
    /// the local twin only when it is newer (LWW), and never over an edit
    /// made here since the twin was synced; the twin's clock never goes back.
    #[tokio::test]
    async fn a_twin_takes_the_values_of_a_newer_op_only() {
        let (core, registry, _dir) = setup().await;
        let db = &core.db;
        local_tag(&core, &registry, "red", 2000).await;

        let older = op(
            "tag",
            "t-there",
            1000,
            Some(json!({ "name": "work", "color": "blue" })),
        );
        apply(&core, &registry, &[older]).await;
        assert_eq!(tag_color(&core).await.as_deref(), Some("red"));
        let twin = load_row_state(db, "tag", "t-here").await.unwrap().unwrap();
        assert_eq!(twin.head_hlc.unwrap().wall_ms, 2000);
        let alias = load_row_state(db, "tag", "t-there").await.unwrap().unwrap();
        assert!(alias.deleted);

        let newer = op(
            "tag",
            "t-there",
            3000,
            Some(json!({ "name": "work", "color": "green" })),
        );
        apply(&core, &registry, &[newer]).await;
        assert_eq!(tag_color(&core).await.as_deref(), Some("green"));
        let twin = load_row_state(db, "tag", "t-here").await.unwrap().unwrap();
        assert_eq!(twin.head_hlc.unwrap().wall_ms, 3000);
        let mut clock = HlcClock::new("here", None);
        let changes = collect_local_changes(&core, &mut clock, &registry.rows(&["tag"]), None)
            .await
            .unwrap();
        assert!(changes.ops.is_empty(), "{:?}", changes.ops);

        sqlx::query("UPDATE tags SET color = 'pink' WHERE id = 't-here'")
            .execute(db)
            .await
            .unwrap();
        let later = op(
            "tag",
            "t-there",
            4000,
            Some(json!({ "name": "work", "color": "black" })),
        );
        apply(&core, &registry, &[later]).await;
        assert_eq!(tag_color(&core).await.as_deref(), Some("pink"));
        let changes = collect_local_changes(&core, &mut clock, &registry.rows(&["tag"]), None)
            .await
            .unwrap();
        let pushed: Vec<_> = changes
            .ops
            .iter()
            .map(|op| (op.entity_id.as_str(), op.deleted, op.hlc.wall_ms > 4000))
            .collect();
        assert_eq!(pushed, [("t-here", false, true)]);
    }

    /// Audit A2-16: the tombstone of an id a local twin took in deletes the
    /// twin (LWW against its state), and the next collect publishes the
    /// twin's own tombstone for the devices that know it by its id; an
    /// older tombstone leaves a newer twin alone.
    #[tokio::test]
    async fn the_tombstone_of_an_absorbed_id_deletes_its_twin_everywhere() {
        let (core, registry, _dir) = setup().await;
        let db = &core.db;
        local_tag(&core, &registry, "red", 2000).await;
        let put = op(
            "tag",
            "t-there",
            1000,
            Some(json!({ "name": "work", "color": "blue" })),
        );
        apply(&core, &registry, &[put]).await;

        // Older than the twin: it stays.
        apply(&core, &registry, &[op("tag", "t-there", 1500, None)]).await;
        assert_eq!(tag_color(&core).await.as_deref(), Some("red"));
        assert!(settings::get(db, TWINS_KEY).await.is_none());

        // Taken in again, then deleted on the device it came from.
        let put = op(
            "tag",
            "t-there",
            2500,
            Some(json!({ "name": "work", "color": "blue" })),
        );
        apply(&core, &registry, &[put]).await;
        assert_eq!(tag_color(&core).await.as_deref(), Some("blue"));
        assert!(settings::get(db, TWINS_KEY).await.is_some());
        let outcome = apply(&core, &registry, &[op("tag", "t-there", 5000, None)]).await;
        assert!(outcome.changed.contains("tag"));
        assert_eq!(tag_color(&core).await, None);
        assert!(settings::get(db, TWINS_KEY).await.is_none());
        let alias = load_row_state(db, "tag", "t-there").await.unwrap().unwrap();
        assert!(alias.deleted);
        assert_eq!(alias.head_hlc.unwrap().wall_ms, 5000);

        let mut clock = HlcClock::new("here", None);
        let changes = collect_local_changes(&core, &mut clock, &registry.rows(&["tag"]), None)
            .await
            .unwrap();
        let pushed: Vec<_> = changes
            .ops
            .iter()
            .map(|op| (op.entity_id.as_str(), op.deleted, op.hlc.wall_ms > 5000))
            .collect();
        assert_eq!(pushed, [("t-here", true, true)]);
        save_states(db, &changes.states).await;
        let changes = collect_local_changes(&core, &mut clock, &registry.rows(&["tag"]), None)
            .await
            .unwrap();
        assert!(changes.ops.is_empty(), "{:?}", changes.ops);
    }

    async fn save_states(db: &Pool<Sqlite>, states: &[RowSyncState]) {
        for st in states {
            save_row_state(db, st).await.unwrap();
        }
    }
}
