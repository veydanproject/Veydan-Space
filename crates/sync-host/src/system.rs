// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The system entities every product syncs, over the tables of the core and
//! of the lock: the key row of the lock, the labels of the entity directory,
//! and the settings shared between devices with the keys of the core.

use crate::registry::{Deletion, Hooks, Registry, TableSpec, Upsert};
use crate::Host;
use serde_json::Value;
use sqlx::{Pool, Sqlite};
use veydan_core::{settings, AppError, BoxFuture, CmdResult, Core};
use veydan_lock::Lock;

pub const SETTING_ENTITY: &str = "setting";
/// The row that holds the wrapped key of the password vault and the lock.
pub const VAULT_KEY_ENTITY: &str = veydan_lock::SYNC_TABLE.entity;
/// The name of an entity for a product without its owner (spec 10.2). New in
/// 5.x: 4.0.7 skips the type, and it keeps no blob.
pub const LABEL_ENTITY: &str = "label";

/// Minutes before the lock closes; a setting of the core, read by the shell.
#[cfg(desktop)]
const LOCK_TIMEOUT_KEY: &str = "notes_lock_timeout_min";

pub(crate) fn register(reg: &mut Registry) {
    let vault = veydan_lock::SYNC_TABLE;
    reg.table(
        TableSpec {
            pk: vault.pk,
            ..TableSpec::plain(vault.entity, vault.table, vault.columns)
        },
        Hooks {
            publish_new: Some(key_row_publish_new),
            before_upsert: Some(keep_replaced_key),
            on_delete: Some(keep_key_of_deleted_row),
            after_apply: Some(key_row_changed),
            ..Hooks::default()
        },
    );
    // Every product takes the labels; only the owners of their kinds write
    // them, through the entity directory.
    reg.table(
        TableSpec {
            pk: "key",
            ..TableSpec::plain(
                LABEL_ENTITY,
                veydan_core::LABELS_TABLE,
                veydan_core::LABEL_COLUMNS,
            )
        },
        Hooks {
            before_upsert: Some(label_of_its_key),
            ..Hooks::default()
        },
    );
    // User preferences shared across desktops; paths, credentials and sync
    // state stay local. The lock hash rides inside `password_vault`, next to
    // the wrap it belongs to. A key no module registered is neither published
    // nor taken, such as the lock keys a dev build once pushed as settings.
    #[cfg(desktop)]
    {
        reg.table(
            TableSpec {
                pk: "key",
                ..TableSpec::plain(SETTING_ENTITY, "app_settings", &["value"])
            },
            Hooks::default(),
        );
        reg.setting("ui_locale");
        reg.setting(LOCK_TIMEOUT_KEY);
    }
}

/// A key row made since a join that left the vault unread waits for
/// `join::settle_join_key` to compare it with the vault's.
fn key_row_publish_new<'a>(db: &'a Pool<Sqlite>, _row: &'a Value) -> BoxFuture<'a, bool> {
    Box::pin(async move {
        settings::get(db, crate::config::JOIN_PENDING)
            .await
            .is_none()
    })
}

/// A label names the entity its key names, whatever the payload says: the
/// directory finds it by the key, a list by the kind. A label always has a
/// name, if an empty one.
fn label_of_its_key<'a>(cx: &'a mut Upsert<'_>) -> BoxFuture<'a, CmdResult<()>> {
    Box::pin(async move {
        let (kind, id) = cx.id.split_once(':').unwrap_or((cx.id, ""));
        cx.payload.insert("kind".into(), Value::String(kind.into()));
        cx.payload.insert("id".into(), Value::String(id.into()));
        if !cx.payload.get("name").is_some_and(Value::is_string) {
            cx.payload
                .insert("name".into(), Value::String(String::new()));
        }
        Ok(())
    })
}

/// A row with another key takes the place of the local one: what this
/// device encrypted with its key must stay readable.
fn keep_replaced_key<'a>(cx: &'a mut Upsert<'_>) -> BoxFuture<'a, CmdResult<()>> {
    Box::pin(async move {
        let incoming = cx.payload.get("vault_id").and_then(Value::as_str);
        lock(&cx.host)?.keep_replaced_key(incoming).await?;
        Ok(())
    })
}

fn keep_key_of_deleted_row<'a>(
    host: &'a Host<'a>,
    _core: &'a Core,
    _id: &'a str,
) -> BoxFuture<'a, CmdResult<Deletion>> {
    Box::pin(async move {
        lock(host)?.keep_replaced_key(None).await?;
        Ok(Deletion::Plain)
    })
}

/// The lock decides which key to keep: only it knows what each module keeps
/// under the key.
pub(crate) fn lock<'a>(host: &Host<'a>) -> CmdResult<&'a Lock> {
    host.state::<Lock>()
        .ok_or_else(|| AppError::other("the lock is not in place"))
}

/// The key row came from another device.
fn key_row_changed<'a>(host: &'a Host<'a>) -> BoxFuture<'a, ()> {
    Box::pin(crate::join::key_row_changed(host))
}
