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
/// device encrypted with its key must stay readable. A row without the
/// column `recovery_version` gets it by the rule of [`recovery_version_of`].
fn keep_replaced_key<'a>(cx: &'a mut Upsert<'_>) -> BoxFuture<'a, CmdResult<()>> {
    Box::pin(async move {
        let incoming = cx.payload.get("vault_id").and_then(Value::as_str);
        lock(&cx.host)?.keep_replaced_key(incoming).await?;
        if !cx.payload.contains_key(RECOVERY_VERSION) {
            // The lock reads its row: the local second wrap the incoming one is compared with.
            let local = veydan_lock::recovery_wrap(cx.db).await?;
            if let Some(version) = recovery_version_of(local.as_ref(), cx.payload) {
                cx.payload.insert(RECOVERY_VERSION.into(), version);
            }
        }
        Ok(())
    })
}

/// The column of the key row that says what its second wrap is made from.
const RECOVERY_VERSION: &str = "recovery_version";
/// The version a row without the column can only have: a code of 24 chars.
const RECOVERY_VERSION_CODE: i64 = 1;

/// The version a key row without the column `recovery_version` is applied
/// with (platform-spec 8.1): the sender is 4.0.7 or 5.x before C3. While
/// `recovery_salt` and `recovery_wrapped_key` are the local row's, the row
/// keeps its own version (`None`: nothing to write, the column is not in the
/// payload); when either changed, the wrap was written by a client that
/// knows a code alone, and the version is 1.
pub fn recovery_version_of(
    local: Option<&(Option<String>, Option<String>)>,
    incoming: &serde_json::Map<String, Value>,
) -> Option<Value> {
    let (local_salt, local_wrap) = local?;
    let same = |column: &str, mine: &Option<String>| match incoming.get(column) {
        // A column the sender left out keeps its local value: unchanged.
        None => true,
        Some(theirs) => theirs.as_str() == mine.as_deref(),
    };
    if same("recovery_salt", local_salt) && same("recovery_wrapped_key", local_wrap) {
        return None;
    }
    Some(Value::from(RECOVERY_VERSION_CODE))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Plan, Step};
    use crate::rows::{put_row, read_row};
    use serde_json::{json, Map};
    use std::any::{Any, TypeId};
    use std::collections::HashMap;
    use veydan_core::{db, Directory, Schema};

    const LOCK: Schema = Schema {
        module: "lock",
        steps: veydan_lock::SCHEMA_STEPS,
    };
    const ROWS: &[&str] = &["password_vault", "setting", "label"];
    const PLAN: Plan = Plan {
        collect: &[Step::rows("rows", None, "app", ROWS)],
        apply: &[Step::rows("rows", None, "app", ROWS)],
        late: &[],
    };

    /// The lock of a device, where the hook of the key row finds it.
    struct Locked(Lock);

    impl crate::States for Locked {
        fn state(&self, ty: TypeId) -> Option<&(dyn Any + Send + Sync)> {
            (ty == TypeId::of::<Lock>()).then_some(&self.0 as _)
        }
    }

    async fn device() -> (Core, Locked, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::open(
            &dir.path().join(db::DB_FILE),
            &[veydan_core::SCHEMA, crate::SCHEMA, LOCK],
        )
        .await
        .unwrap();
        let lock = Lock::new(pool.clone());
        lock.open_default().await.unwrap();
        (
            Core::new(pool, dir.path().to_owned(), Directory::default()),
            Locked(lock),
            dir,
        )
    }

    fn registry() -> Registry {
        Registry::new().finish(&PLAN).unwrap()
    }

    async fn key_row(core: &Core, registry: &Registry) -> Map<String, Value> {
        let table = registry.table_of(VAULT_KEY_ENTITY).unwrap();
        read_row(&core.db, table, veydan_lock::ROW_ID)
            .await
            .unwrap()
            .and_then(|v| v.as_object().cloned())
            .expect("a key row")
    }

    async fn put(core: &Core, locked: &Locked, registry: &Registry, payload: Map<String, Value>) {
        let table = registry.table_of(VAULT_KEY_ENTITY).unwrap();
        put_row(
            &Host::States(locked),
            &core.db,
            &core.app_data_dir,
            table,
            veydan_lock::ROW_ID,
            payload,
            &mut HashMap::new(),
        )
        .await
        .unwrap();
        locked.0.refresh_after_sync().await;
    }

    fn pair(salt: &str, wrap: &str) -> (Option<String>, Option<String>) {
        (Some(salt.into()), Some(wrap.into()))
    }

    #[test]
    fn a_row_without_the_column_keeps_the_version_unless_its_wrap_changed() {
        let local = pair("salt", "wrap");
        let same = json!({ "recovery_salt": "salt", "recovery_wrapped_key": "wrap" });
        let same = same.as_object().unwrap();
        assert_eq!(recovery_version_of(Some(&local), same), None);
        // A payload that leaves the columns out changes nothing either.
        assert_eq!(recovery_version_of(Some(&local), &Map::new()), None);
        // No local row: the default of the column stands.
        assert_eq!(recovery_version_of(None, same), None);
        for changed in [
            json!({ "recovery_salt": "other", "recovery_wrapped_key": "wrap" }),
            json!({ "recovery_salt": "salt", "recovery_wrapped_key": "other" }),
            json!({ "recovery_salt": null, "recovery_wrapped_key": null }),
        ] {
            assert_eq!(
                recovery_version_of(Some(&local), changed.as_object().unwrap()),
                Some(json!(1)),
                "{changed}"
            );
        }
        let none = (None, None);
        let set = json!({ "recovery_salt": "s", "recovery_wrapped_key": "w" });
        assert_eq!(
            recovery_version_of(Some(&none), set.as_object().unwrap()),
            Some(json!(1))
        );
    }

    /// Platform-spec 8.1: a client without the column (4.0.7, 5.x before C3)
    /// changes the password; the row comes without `recovery_version` and
    /// with the same second wrap, and the Recovery Key of 12 words still
    /// opens the lock here. A second wrap it rewrote is one by a code.
    #[tokio::test]
    async fn a_password_changed_without_the_column_keeps_the_recovery_key_of_12_words() {
        let registry = registry();
        let (a, lock_a, _dir_a) = device().await;
        let (b, lock_b, _dir_b) = device().await;
        let words = lock_a
            .0
            .set(Some("4821".into()), None, Some("pin".into()), None)
            .await
            .unwrap()
            .expect("the recovery key");
        let published = key_row(&a, &registry).await;
        assert_eq!(published["recovery_version"], json!(2));

        // B takes the row of A as 5.x publishes it: the column travels.
        put(&b, &lock_b, &registry, published).await;
        assert_eq!(key_row(&b, &registry).await["recovery_version"], json!(2));
        assert_eq!(lock_b.0.status().await.recovery_version, Some(2));
        lock_b.0.recovery_check(&words).await.unwrap();

        // A changes its password on a client without the column.
        lock_a
            .0
            .set(Some("secret words".into()), Some("4821".into()), None, None)
            .await
            .unwrap();
        let mut from_old_client = key_row(&a, &registry).await;
        from_old_client.remove("recovery_version");
        assert_ne!(from_old_client["wrapped_key"], key_row(&b, &registry).await["wrapped_key"]);
        put(&b, &lock_b, &registry, from_old_client.clone()).await;
        let row = key_row(&b, &registry).await;
        assert_eq!(row["recovery_version"], json!(2));
        assert_eq!(row["wrapped_key"], from_old_client["wrapped_key"]);
        lock_b.0.recovery_check(&words).await.unwrap();
        assert!(lock_b.0.unlock("secret words").await.unwrap());

        // The old client rewrote the second wrap: only a code could have made it.
        from_old_client.insert("recovery_salt".into(), json!("b3RoZXItc2FsdC0xNi0xNjE2"));
        from_old_client.insert("recovery_wrapped_key".into(), json!("b3RoZXItd3JhcA=="));
        put(&b, &lock_b, &registry, from_old_client).await;
        let row = key_row(&b, &registry).await;
        assert_eq!(row["recovery_version"], json!(1));
        assert_eq!(lock_b.0.status().await.recovery_version, Some(1));
        assert_eq!(
            lock_b.0.recovery_check(&words).await.unwrap_err(),
            veydan_lock::Error::RecoveryInvalid
        );

        // What 5.x publishes carries every key of 4.0.7 and the column.
        let columns = veydan_lock::SYNC_TABLE.columns;
        for key in [
            "vault_id",
            "crypto_version",
            "kdf_algorithm",
            "kdf_salt",
            "kdf_memory",
            "kdf_iterations",
            "kdf_parallelism",
            "wrapped_key",
            "recovery_salt",
            "recovery_wrapped_key",
            "lock_hash",
            "lock_kind",
            "lock_hint",
            "created_at",
            "updated_at",
            "recovery_version",
        ] {
            assert!(columns.contains(&key), "{key}");
        }
        a.db.close().await;
        b.db.close().await;
    }
}
