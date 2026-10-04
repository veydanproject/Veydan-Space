// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Named secret boxes: values a module keeps under the vault key on this
//! device only. A value is bound to the name of its box and to its key, is
//! stored in `lock_secrets` and is never synced.

use crate::error::{Error, Result};
use crate::vault::{decrypt_field, encrypt_field};
use crate::Lock;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use zeroize::Zeroizing;

/// The field every value of a box is encrypted as.
pub(crate) const FIELD: &str = "secret";

/// What binds a value to its box and key. For the box `messenger` it is what
/// the messenger's secrets were bound to before the boxes existed.
pub(crate) fn aad(name: &str, key: &str) -> String {
    format!("{name}:{key}")
}

/// One box of the lock; `Lock::secret_box` hands it out.
pub struct SecretBox<'a> {
    pub(crate) lock: &'a Lock,
    pub(crate) name: &'a str,
}

impl SecretBox<'_> {
    /// The value under `key`, `None` when there is none. Fails while the vault is closed.
    pub async fn get(&self, key: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        let (vault_key, _) = self.lock.require_open()?;
        let stored: Option<String> =
            sqlx::query_scalar("SELECT value FROM lock_secrets WHERE box = ? AND key = ?")
                .bind(self.name)
                .bind(key)
                .fetch_optional(&self.lock.db)
                .await
                .map_err(Error::db)?;
        let Some(stored) = stored else {
            return Ok(None);
        };
        let b64 = Zeroizing::new(decrypt_field(
            &vault_key,
            &aad(self.name, key),
            FIELD,
            &stored,
        )?);
        let bytes = B64
            .decode(b64.as_bytes())
            .map_err(|_| Error::DecryptFailed)?;
        Ok(Some(Zeroizing::new(bytes)))
    }

    /// Store `value` under `key`. The first value of a device without a key
    /// row makes the row; fails while the vault is closed.
    pub async fn put(&self, key: &str, value: &[u8]) -> Result<()> {
        let (vault_key, _) = self.lock.ensure_key().await?;
        let b64 = Zeroizing::new(B64.encode(value));
        let stored = encrypt_field(&vault_key, &aad(self.name, key), FIELD, &b64)?;
        sqlx::query(
            "INSERT INTO lock_secrets (box, key, value) VALUES (?, ?, ?)
             ON CONFLICT(box, key) DO UPDATE SET value = excluded.value",
        )
        .bind(self.name)
        .bind(key)
        .bind(stored)
        .execute(&self.lock.db)
        .await
        .map_err(Error::db)?;
        Ok(())
    }

    /// Remove the value under `key`. Needs no key, so it works while the vault is closed.
    pub async fn delete(&self, key: &str) -> Result<()> {
        sqlx::query("DELETE FROM lock_secrets WHERE box = ? AND key = ?")
            .bind(self.name)
            .bind(key)
            .execute(&self.lock.db)
            .await
            .map_err(Error::db)?;
        Ok(())
    }

    /// Whether `get` and `put` would find the vault key: the vault is open,
    /// or no lock guards it and the key is made on first use.
    pub fn is_unlocked(&self) -> bool {
        self.lock.vault.open_key().is_some() || self.lock.vault.pending().is_some()
    }
}

#[cfg(test)]
mod tests {
    use crate::testing::pool;
    use crate::{Error, Event, Lock};

    #[tokio::test]
    async fn a_value_goes_into_its_box_and_comes_back() {
        let (db, _dir) = pool().await;
        let lock = Lock::new(db.clone());
        lock.open_default().await.unwrap();
        let messenger = lock.secret_box("messenger");
        let other = lock.secret_box("other");

        assert!(messenger.is_unlocked());
        messenger.put("nsec", b"the messenger key").await.unwrap();
        assert_eq!(messenger.get("npub").await.unwrap(), None);
        assert_eq!(
            messenger.get("nsec").await.unwrap().unwrap().as_slice(),
            b"the messenger key"
        );
        assert_eq!(other.get("nsec").await.unwrap(), None);

        // A value moved to another box or key does not open there.
        sqlx::query("UPDATE lock_secrets SET box = 'other'")
            .execute(&db)
            .await
            .unwrap();
        assert_eq!(other.get("nsec").await.unwrap_err(), Error::DecryptFailed);

        other.delete("nsec").await.unwrap();
        assert_eq!(other.get("nsec").await.unwrap(), None);
        db.close().await;
    }

    #[tokio::test]
    async fn a_closed_vault_keeps_the_boxes_closed() {
        let (db, _dir) = pool().await;
        let lock = Lock::new(db.clone());
        lock.open_default().await.unwrap();
        lock.set(Some("4821".into()), None, Some("pin".into()), None)
            .await
            .unwrap();
        let messenger = lock.secret_box("messenger");
        messenger.put("nsec", b"the messenger key").await.unwrap();

        lock.lock();
        assert!(!messenger.is_unlocked());
        assert_eq!(messenger.get("nsec").await.unwrap_err(), Error::VaultLocked);
        assert_eq!(
            messenger.put("nsec", b"another").await.unwrap_err(),
            Error::VaultLocked
        );
        assert!(lock.unlock("4821").await.unwrap());
        assert_eq!(
            messenger.get("nsec").await.unwrap().unwrap().as_slice(),
            b"the messenger key"
        );
        db.close().await;
    }

    #[tokio::test]
    async fn a_reset_empties_every_box_and_says_so() {
        let (db, _dir) = pool().await;
        let lock = Lock::new(db.clone());
        lock.open_default().await.unwrap();
        lock.set(Some("4821".into()), None, Some("pin".into()), None)
            .await
            .unwrap();
        lock.secret_box("messenger")
            .put("nsec", b"the messenger key")
            .await
            .unwrap();
        lock.secret_box("other").put("token", b"t").await.unwrap();
        let old_key = crate::key_id(&db).await.unwrap();
        sqlx::query("INSERT INTO passwords (id, password_enc, vault_id) VALUES ('pw-1', 'x', ?)")
            .bind(old_key.as_deref())
            .execute(&db)
            .await
            .unwrap();
        let count = |table: &'static str| {
            let db = db.clone();
            async move {
                sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!(
                    "SELECT COUNT(*) FROM {table}"
                )))
                .fetch_one(&db)
                .await
                .unwrap()
            }
        };
        let mut events = lock.subscribe();

        // A wrong secret, or a caller whose own removal fails: nothing goes.
        assert!(lock
            .reset("1111", |_| Box::pin(async { Ok(()) }))
            .await
            .is_err());
        let failing = lock
            .reset("4821", |conn| {
                Box::pin(async move {
                    sqlx::query("DELETE FROM passwords")
                        .execute(&mut *conn)
                        .await
                        .map_err(Error::db)?;
                    Err(Error::other("the caller failed"))
                })
            })
            .await;
        assert_eq!(failing.unwrap_err(), Error::other("the caller failed"));
        assert_eq!(crate::key_id(&db).await.unwrap(), old_key);
        assert_eq!(
            (count("passwords").await, count("lock_secrets").await),
            (1, 2)
        );
        assert!(events.try_recv().is_err());

        lock.reset("4821", |conn| {
            Box::pin(async move {
                sqlx::query("DELETE FROM passwords")
                    .execute(conn)
                    .await
                    .map(|_| ())
                    .map_err(Error::db)
            })
        })
        .await
        .unwrap();

        assert_eq!(events.try_recv().unwrap(), Event::Reset);
        assert_eq!(
            (count("passwords").await, count("lock_secrets").await),
            (0, 0)
        );
        assert_ne!(crate::key_id(&db).await.unwrap(), old_key);
        // The lock stays and opens the new key.
        assert!(lock.enabled().await);
        lock.lock();
        assert!(lock.unlock("4821").await.unwrap());
        lock.secret_box("messenger")
            .put("nsec", b"new")
            .await
            .unwrap();
        db.close().await;
    }
}
