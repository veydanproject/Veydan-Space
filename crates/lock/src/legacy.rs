// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a data file of an earlier 5.0 build keeps outside the lock's tables:
//! the lock in `app_settings`, mirrored into the key row, and the messenger's
//! secrets as `app_settings` values. `Lock::upgrade` takes both over once.

use crate::error::{Error, Result};
use crate::vault::{delete_setting, read_setting, synced_lock, LockMeta, SyncedLock};
use crate::Lock;
use sqlx::{Sqlite, Transaction};
use std::sync::atomic::Ordering;

/// The keys the lock was kept under: hash, kind, hint.
const HASH_KEY: &str = "notes_lock_hash";
const KIND_KEY: &str = "lock_kind";
const HINT_KEY: &str = "lock_hint";
/// Prefix of the `app_settings` keys that held the values of the box `messenger`.
const SECRET_PREFIX: &str = "messenger_secret:";
const SECRET_BOX: &str = "messenger";

async fn read_lock(tx: &mut Transaction<'_, Sqlite>) -> Result<Option<LockMeta>> {
    let Some(hash) = read_setting(&mut **tx, HASH_KEY).await? else {
        return Ok(None);
    };
    Ok(Some(LockMeta {
        hash,
        kind: read_setting(&mut **tx, KIND_KEY).await?,
        hint: read_setting(&mut **tx, HINT_KEY).await?,
    }))
}

async fn delete_lock(tx: &mut Transaction<'_, Sqlite>) -> Result<()> {
    for key in [HASH_KEY, KIND_KEY, HINT_KEY] {
        delete_setting(&mut **tx, key).await?;
    }
    Ok(())
}

impl Lock {
    /// Take over what an earlier 5.0 build kept in `app_settings`, before
    /// anything reads the lock. Nothing is left to do on a file written by
    /// this build.
    ///
    /// The lock: the key row decides. A row that names a lock or names none
    /// is what the settings were last made equal to, so they go. Without a
    /// row the settings stay until the first unlock with their secret makes
    /// the row (or a sync brings one): a PIN set on this device stands
    /// meanwhile. A row that says nothing keeps saying it: the settings'
    /// hash goes into the row only once their secret has opened the key it
    /// wraps, since the row is synced and a hash beside a wrap it does not
    /// match locks every device out of that key. Until then the settings'
    /// lock opens the session, as it did in the earlier build.
    ///
    /// The messenger's secrets move into its box as they are: the box binds
    /// a value to the same name, so the stored ciphertext stays valid.
    ///
    /// When this fails, the lock stays closed for the run: a data file of the
    /// earlier build must not be taken for one without a lock.
    pub async fn upgrade(&self) -> Result<()> {
        let taken = self.take_over().await;
        if taken.is_err() {
            self.not_taken_over.store(true, Ordering::Relaxed);
        }
        taken
    }

    async fn take_over(&self) -> Result<()> {
        let mut tx = self.db.begin().await.map_err(Error::db)?;
        if let Some(meta) = read_lock(&mut tx).await? {
            match synced_lock(&mut *tx).await? {
                SyncedLock::NoRow | SyncedLock::Unstated => {
                    *self.legacy.lock().unwrap() = Some(meta)
                }
                SyncedLock::Default | SyncedLock::Meta(_) => delete_lock(&mut tx).await?,
            }
        }
        sqlx::query(
            "INSERT OR IGNORE INTO lock_secrets (box, key, value)
             SELECT ?, substr(key, length(?) + 1), value FROM app_settings
             WHERE substr(key, 1, length(?)) = ?",
        )
        .bind(SECRET_BOX)
        .bind(SECRET_PREFIX)
        .bind(SECRET_PREFIX)
        .bind(SECRET_PREFIX)
        .execute(&mut *tx)
        .await
        .map_err(Error::db)?;
        sqlx::query("DELETE FROM app_settings WHERE substr(key, 1, length(?)) = ?")
            .bind(SECRET_PREFIX)
            .bind(SECRET_PREFIX)
            .execute(&mut *tx)
            .await
            .map_err(Error::db)?;
        tx.commit().await.map_err(Error::db)
    }

    /// The lock an earlier build kept in its settings, while the key row
    /// names no lock: there is none, or it says nothing.
    pub(crate) fn legacy(&self) -> Option<LockMeta> {
        self.legacy.lock().unwrap().clone()
    }

    /// That lock, when `secret` is its secret.
    pub(crate) fn legacy_for(&self, secret: &str) -> Option<LockMeta> {
        self.legacy()
            .filter(|meta| crate::verify_hash(secret, &meta.hash))
    }

    /// The key row names a lock or none now: the lock of the settings is in
    /// it or gave way to it.
    pub(crate) async fn drop_legacy(&self) {
        if self.legacy.lock().unwrap().take().is_none() {
            return;
        }
        let dropped = async {
            let mut tx = self.db.begin().await.map_err(Error::db)?;
            delete_lock(&mut tx).await?;
            tx.commit().await.map_err(Error::db)
        };
        if let Err(e) = dropped.await {
            eprintln!("lock: the lock of the settings stays: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{key_row, pool};
    use crate::{decrypt_field, encrypt_field, secrets, store_lock_meta, DEFAULT_SECRET};
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    use sqlx::{Pool, Sqlite};

    const PIN: &str = "4821";

    async fn setting(db: &Pool<Sqlite>, key: &str, value: &str) {
        sqlx::query("INSERT INTO app_settings (key, value) VALUES (?, ?)")
            .bind(key)
            .bind(value)
            .execute(db)
            .await
            .unwrap();
    }

    async fn settings_left(db: &Pool<Sqlite>) -> Vec<String> {
        sqlx::query_scalar("SELECT key FROM app_settings ORDER BY key")
            .fetch_all(db)
            .await
            .unwrap()
    }

    /// The lock as the stage-2 build kept it: hash, kind and hint in the settings.
    async fn settings_lock(db: &Pool<Sqlite>, secret: &str) -> LockMeta {
        let meta = LockMeta {
            hash: crate::hash_password(secret).unwrap(),
            kind: Some("pin".into()),
            hint: Some("the usual".into()),
        };
        setting(db, HASH_KEY, &meta.hash).await;
        setting(db, KIND_KEY, "pin").await;
        setting(db, HINT_KEY, "the usual").await;
        meta
    }

    #[tokio::test]
    async fn a_pin_and_passwords_of_the_earlier_build_open_with_this_one() {
        let (db, _dir) = pool().await;
        let key = key_row(&db, "own", PIN).await;
        let meta = settings_lock(&db, PIN).await;
        store_lock_meta(&db, Some(&meta)).await.unwrap();
        sqlx::query("INSERT INTO passwords (id, password_enc, vault_id) VALUES ('pw-1', ?, 'own')")
            .bind(encrypt_field(&key, "pw-1", "password", "hunter2").unwrap())
            .execute(&db)
            .await
            .unwrap();
        // The messenger's secret as `HostSecretStore` wrote it.
        let stored = encrypt_field(
            &key,
            &secrets::aad("messenger", "nsec"),
            secrets::FIELD,
            &B64.encode(b"the messenger key"),
        )
        .unwrap();
        setting(&db, "messenger_secret:nsec", &stored).await;
        setting(&db, "ui_locale", "ru").await;

        let lock = Lock::new(db.clone());
        lock.upgrade().await.unwrap();
        lock.open_default().await.unwrap();

        assert_eq!(settings_left(&db).await, ["ui_locale"]);
        let status = lock.status().await;
        assert!(status.enabled && status.locked);
        assert_eq!(
            (status.kind.as_str(), status.hint.as_deref()),
            ("pin", Some("the usual"))
        );
        assert!(lock.unlock("0000").await.is_err());
        assert!(lock.unlock(PIN).await.unwrap());
        let (open, vault_id) = lock.require_open().unwrap();
        assert_eq!(vault_id, "own");
        let enc: String = sqlx::query_scalar("SELECT password_enc FROM passwords")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(
            decrypt_field(&open, "pw-1", "password", &enc).unwrap(),
            "hunter2"
        );
        assert_eq!(
            lock.secret_box("messenger")
                .get("nsec")
                .await
                .unwrap()
                .unwrap()
                .as_slice(),
            b"the messenger key"
        );

        // A second start finds nothing more to do.
        lock.upgrade().await.unwrap();
        assert_eq!(settings_left(&db).await, ["ui_locale"]);
        db.close().await;
    }

    #[tokio::test]
    async fn a_row_that_says_nothing_takes_the_lock_of_the_settings_once_it_opens_the_key() {
        let (db, _dir) = pool().await;
        key_row(&db, "own", PIN).await;
        let meta = settings_lock(&db, PIN).await;

        let lock = Lock::new(db.clone());
        lock.upgrade().await.unwrap();
        lock.open_default().await.unwrap();

        assert_eq!(synced_lock(&db).await.unwrap(), SyncedLock::Unstated);
        let status = lock.status().await;
        assert!(status.enabled && status.locked);
        assert_eq!(
            (status.kind.as_str(), status.hint.as_deref()),
            ("pin", Some("the usual"))
        );
        assert!(lock.unlock(PIN).await.unwrap());

        assert_eq!(synced_lock(&db).await.unwrap(), SyncedLock::Meta(meta));
        assert!(settings_left(&db).await.is_empty());
        assert_eq!(lock.require_open().unwrap().1, "own");
        db.close().await;
    }

    /// A peer of 4.0.7 wrapped the key with its secret and said nothing; the
    /// earlier build kept another PIN in its settings. That hash must not go
    /// into the synced row: the peer would no longer open its own key.
    #[tokio::test]
    async fn a_lock_of_the_settings_that_does_not_open_the_key_stays_out_of_the_row() {
        let (db, _dir) = pool().await;
        key_row(&db, "theirs", "2222").await;
        settings_lock(&db, "1111").await;

        let lock = Lock::new(db.clone());
        lock.upgrade().await.unwrap();
        lock.open_default().await.unwrap();
        assert_eq!(synced_lock(&db).await.unwrap(), SyncedLock::Unstated);
        assert_eq!(settings_left(&db).await.len(), 3);

        // The PIN of this device opens the session, as in the earlier
        // build, and not the key; the row stays as it was.
        let mut events = lock.subscribe();
        assert!(lock.unlock("1111").await.unwrap());
        let status = lock.status().await;
        assert!(!status.locked);
        assert_eq!(status.vault, "mismatch");
        assert_eq!(lock.require_open().unwrap_err(), Error::VaultMismatch);
        lock.verify("1111").await.unwrap();
        lock.refresh_after_sync().await;
        assert_eq!(
            std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>(),
            [crate::Event::Unlocked, crate::Event::Changed]
        );
        assert!(!lock.status().await.locked);
        assert_eq!(lock.status().await.vault, "mismatch");
        assert_eq!(synced_lock(&db).await.unwrap(), SyncedLock::Unstated);

        // The secret that wraps the key names the lock; the settings give way.
        lock.lock();
        assert!(lock.unlock("2222").await.unwrap());
        let SyncedLock::Meta(meta) = synced_lock(&db).await.unwrap() else {
            panic!("the unlock did not name the lock");
        };
        assert!(crate::verify_hash("2222", &meta.hash));
        assert_eq!((meta.kind, meta.hint), (None, None));
        assert!(settings_left(&db).await.is_empty());
        assert!(lock.require_open().is_ok());
        lock.lock();
        assert!(lock.unlock("1111").await.is_err());

        // The row as the peer gets it opens with the peer's own secret.
        let peer = Lock::new(db.clone());
        peer.open_default().await.unwrap();
        assert!(peer.unlock("2222").await.unwrap());
        assert!(peer.require_open().is_ok());
        db.close().await;
    }

    #[tokio::test]
    async fn an_upgrade_that_fails_leaves_the_lock_closed() {
        let (db, _dir) = pool().await;
        settings_lock(&db, PIN).await;
        sqlx::query("DROP TABLE lock_secrets")
            .execute(&db)
            .await
            .unwrap();

        let lock = Lock::new(db.clone());
        assert!(lock.upgrade().await.is_err());
        assert!(lock.open_default().await.is_err());

        assert!(lock.enabled().await);
        let status = lock.status().await;
        assert!(status.enabled && status.locked);
        assert!(lock.unlock(PIN).await.is_err());
        assert_eq!(lock.require_open().unwrap_err(), Error::VaultLocked);
        assert_eq!(lock.ensure_key().await.unwrap_err(), Error::VaultLocked);
        lock.refresh_after_sync().await;
        assert!(lock.status().await.locked);
        assert_eq!(synced_lock(&db).await.unwrap(), SyncedLock::NoRow);
        assert_eq!(settings_left(&db).await.len(), 3);
        db.close().await;
    }

    #[tokio::test]
    async fn a_row_that_names_no_lock_wins_over_the_settings() {
        let (db, _dir) = pool().await;
        key_row(&db, "own", DEFAULT_SECRET).await;
        store_lock_meta(&db, None).await.unwrap();
        settings_lock(&db, PIN).await;

        let lock = Lock::new(db.clone());
        lock.upgrade().await.unwrap();
        lock.open_default().await.unwrap();

        assert_eq!(synced_lock(&db).await.unwrap(), SyncedLock::Default);
        assert!(settings_left(&db).await.is_empty());
        assert!(!lock.enabled().await);
        assert!(lock.require_open().is_ok());
        db.close().await;
    }

    #[tokio::test]
    async fn without_a_row_the_lock_of_the_settings_stands_until_its_first_unlock() {
        let (db, _dir) = pool().await;
        let meta = settings_lock(&db, PIN).await;

        let lock = Lock::new(db.clone());
        lock.upgrade().await.unwrap();
        lock.open_default().await.unwrap();

        assert!(lock.enabled().await);
        let status = lock.status().await;
        assert!(status.locked);
        assert_eq!(status.kind, "pin");
        assert_eq!(lock.require_open().unwrap_err(), Error::VaultLocked);
        assert!(lock.unlock("0000").await.is_err());
        assert!(lock.unlock(PIN).await.unwrap());

        assert_eq!(synced_lock(&db).await.unwrap(), SyncedLock::Meta(meta));
        assert!(settings_left(&db).await.is_empty());
        lock.lock();
        assert!(lock.unlock(PIN).await.unwrap());
        assert!(lock.require_open().is_ok());
        db.close().await;
    }
}
