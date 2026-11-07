// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Messenger identity.
//!
//! One Nostr key pair per profile. The private key lives only in the host
//! `SecretStore` (never in `messenger.db`); the database keeps the public
//! half and a reference to the secret. Backup/export is NIP-49 (`ncryptsec`,
//! scrypt + XChaCha20-Poly1305); import accepts `nsec`, `ncryptsec` and a
//! BIP-39 mnemonic (NIP-06).
//!
//! `keys` holds the pure functions; `IdentityService` glues them to storage.

pub mod keys;

use messenger_core::{MessengerError, PubKey, Result, SecretStore, Timestamp};
use messenger_store::{account, identity as repo, Store};
use nostr::key::Keys;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use zeroize::Zeroizing;

/// `SecretStore` key under which the 32-byte secret key is kept.
pub const SECRET_KEY_REF: &str = "identity.nsec";

/// Scrypt cost for exported backups (2^16). Import accepts any cost.
pub const EXPORT_LOG_N: u8 = 16;

/// Public identity facts, safe for the UI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub npub: String,
    pub pubkey: PubKey,
    pub created_at: Timestamp,
}

/// Result of creating a fresh identity: the backup string is shown once.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreatedIdentity {
    pub identity: Identity,
    pub ncryptsec: String,
}

pub struct IdentityService {
    store: Store,
    secrets: Arc<dyn SecretStore>,
    /// The last create or import wiped the data of another identity
    /// (`take_wiped`): the files and the memory of it are the runtime's.
    wiped: AtomicBool,
}

impl IdentityService {
    pub fn new(store: Store, secrets: Arc<dyn SecretStore>) -> Self {
        Self { store, secrets, wiped: AtomicBool::new(false) }
    }

    /// Whether a create or an import wiped the data of another identity
    /// since the last call (`account::wipe`).
    pub fn take_wiped(&self) -> bool {
        self.wiped.swap(false, Ordering::SeqCst)
    }

    /// The current identity, if one was created or imported.
    pub async fn get(&self) -> Result<Option<Identity>> {
        Ok(repo::get(&self.store).await?.map(row_to_identity))
    }

    /// Generate a key pair, encrypt it for backup with `password`, persist the
    /// secret and return the backup string. `password` is only used for the
    /// backup; the stored secret is protected by the host.
    pub async fn create(&self, password: &str) -> Result<CreatedIdentity> {
        self.ensure_absent().await?;
        let keys = keys::generate();
        let ncryptsec = spawn_kdf(keys.clone(), password.to_string(), EXPORT_LOG_N).await?;
        let identity = self.persist(&keys).await?;
        Ok(CreatedIdentity { identity, ncryptsec })
    }

    pub async fn import_nsec(&self, nsec: &str) -> Result<Identity> {
        self.ensure_absent().await?;
        let keys = keys::parse_secret(nsec)?;
        self.persist(&keys).await
    }

    pub async fn import_ncryptsec(&self, ncryptsec: &str, password: &str) -> Result<Identity> {
        self.ensure_absent().await?;
        let s = ncryptsec.trim().to_string();
        let pw = password.to_string();
        let keys = tokio::task::spawn_blocking(move || keys::decrypt_ncryptsec(&s, &pw))
            .await
            .map_err(|e| MessengerError::Other(e.to_string()))??;
        self.persist(&keys).await
    }

    pub async fn import_mnemonic(&self, mnemonic: &str, passphrase: &str) -> Result<Identity> {
        self.ensure_absent().await?;
        let keys = keys::from_mnemonic(mnemonic, passphrase)?;
        self.persist(&keys).await
    }

    /// NIP-49 backup of the stored secret, encrypted with `password`.
    pub async fn export_ncryptsec(&self, password: &str) -> Result<String> {
        let keys = self.load_keys().await?;
        spawn_kdf(keys, password.to_string(), EXPORT_LOG_N).await
    }

    /// Remove the identity and its secret. Chats and contacts stay, and so
    /// does the record of whose they are (`account::KEY_OWNER`): the same
    /// key imported again finds them, another key wipes them (`persist`).
    pub async fn delete(&self) -> Result<()> {
        let Some(row) = repo::get(&self.store).await? else {
            return Ok(());
        };
        account::set_owner(&self.store, &row.pubkey_hex).await?;
        self.secrets.delete(SECRET_KEY_REF).await?;
        repo::delete(&self.store).await
    }

    /// Signing keys for the session. Fails with `NotLoggedIn` when no
    /// identity exists and with `SecretsLocked` when the host is locked.
    pub async fn load_keys(&self) -> Result<Keys> {
        let row = repo::get(&self.store).await?.ok_or(MessengerError::NotLoggedIn)?;
        let bytes = self
            .secrets
            .get(&row.nsec_ref)
            .await?
            .ok_or_else(|| MessengerError::SecretMissing(row.nsec_ref.clone()))?;
        let keys = keys::from_secret_bytes(&bytes)?;
        if keys.public_key().to_hex() != row.pubkey_hex {
            return Err(MessengerError::Crypto("stored secret does not match identity".into()));
        }
        Ok(keys)
    }

    async fn ensure_absent(&self) -> Result<()> {
        if repo::get(&self.store).await?.is_some() {
            return Err(MessengerError::Invalid("an identity already exists".into()));
        }
        if !self.secrets.is_unlocked().await {
            return Err(MessengerError::SecretsLocked);
        }
        Ok(())
    }

    /// The key becomes the identity. The data the store holds is of one
    /// identity (`account::owner`): that of another key, or of nobody known,
    /// is wiped first (nobody known: when it holds any); the same key
    /// keeps it.
    async fn persist(&self, keys: &Keys) -> Result<Identity> {
        let npub = keys::npub(keys);
        let pubkey_hex = keys.public_key().to_hex();
        let theirs = match account::owner(&self.store).await? {
            Some(owner) => owner != pubkey_hex,
            None => account::holds_data(&self.store).await?,
        };
        if theirs {
            account::wipe(&self.store).await?;
            self.wiped.store(true, Ordering::SeqCst);
        }
        let secret: Zeroizing<[u8; 32]> = Zeroizing::new(keys.secret_key().to_secret_bytes());
        self.secrets.put(SECRET_KEY_REF, secret.as_slice()).await?;
        let created_at = now();
        let row = repo::IdentityRow {
            id: "primary".to_string(),
            npub: npub.clone(),
            pubkey_hex: pubkey_hex.clone(),
            nsec_ref: SECRET_KEY_REF.to_string(),
            created_at: created_at.secs(),
        };
        if let Err(e) = repo::insert(&self.store, &row).await {
            // Do not leave an orphan secret behind.
            let _ = self.secrets.delete(SECRET_KEY_REF).await;
            return Err(e);
        }
        account::set_owner(&self.store, &pubkey_hex).await?;
        Ok(row_to_identity(row))
    }
}

async fn spawn_kdf(keys: Keys, password: String, log_n: u8) -> Result<String> {
    tokio::task::spawn_blocking(move || keys::encrypt_ncryptsec(&keys, &password, log_n))
        .await
        .map_err(|e| MessengerError::Other(e.to_string()))?
}

fn row_to_identity(row: repo::IdentityRow) -> Identity {
    Identity {
        npub: row.npub,
        pubkey: PubKey::parse(&row.pubkey_hex).expect("stored pubkey is valid hex"),
        created_at: Timestamp(row.created_at),
    }
}

fn now() -> Timestamp {
    use messenger_core::Clock;
    messenger_core::traits::SystemClock.now()
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_testkit::MemorySecretStore;

    async fn service(locked: bool) -> (IdentityService, Arc<MemorySecretStore>) {
        let store = Store::open_in_memory().await.unwrap();
        let secrets = Arc::new(if locked { MemorySecretStore::locked() } else { MemorySecretStore::unlocked() });
        (IdentityService::new(store, secrets.clone()), secrets)
    }

    #[tokio::test]
    async fn create_persists_public_half_and_secret_separately() {
        let (svc, secrets) = service(false).await;
        assert!(svc.get().await.unwrap().is_none());

        let created = svc.create("backup-pw").await.unwrap();
        assert!(created.identity.npub.starts_with("npub1"));
        assert!(created.ncryptsec.starts_with("ncryptsec1"));

        let got = svc.get().await.unwrap().unwrap();
        assert_eq!(got, created.identity);
        assert!(secrets.get(SECRET_KEY_REF).await.unwrap().is_some());

        let keys = svc.load_keys().await.unwrap();
        assert_eq!(keys.public_key().to_hex(), created.identity.pubkey.as_hex());
    }

    #[tokio::test]
    async fn second_identity_is_refused() {
        let (svc, _) = service(false).await;
        svc.create("x").await.unwrap();
        let err = svc.create("y").await.unwrap_err();
        assert!(matches!(err, MessengerError::Invalid(_)));
    }

    #[tokio::test]
    async fn locked_secrets_block_creation() {
        let (svc, _) = service(true).await;
        assert!(matches!(svc.create("x").await, Err(MessengerError::SecretsLocked)));
    }

    #[tokio::test]
    async fn export_import_roundtrip_through_ncryptsec() {
        let (a, _) = service(false).await;
        let created = a.create("pw-a").await.unwrap();
        let backup = a.export_ncryptsec("pw-b").await.unwrap();

        let (b, _) = service(false).await;
        assert!(b.import_ncryptsec(&backup, "wrong").await.is_err());
        let imported = b.import_ncryptsec(&backup, "pw-b").await.unwrap();
        assert_eq!(imported.npub, created.identity.npub);
    }

    #[tokio::test]
    async fn import_nsec_and_delete() {
        let (svc, secrets) = service(false).await;
        let keys = keys::generate();
        let nsec = keys::nsec(&keys);
        let id = svc.import_nsec(&nsec).await.unwrap();
        assert_eq!(id.npub, keys::npub(&keys));

        svc.delete().await.unwrap();
        assert!(svc.get().await.unwrap().is_none());
        assert!(secrets.get(SECRET_KEY_REF).await.unwrap().is_none());
        assert!(matches!(svc.load_keys().await, Err(MessengerError::NotLoggedIn)));
    }

    async fn chats(svc: &IdentityService) -> usize {
        messenger_store::chats::list(&svc.store, true).await.unwrap().len()
    }

    /// The key went (a reset of the lock, the demo data): the chats stay
    /// for the same key imported again, and go before another key is made
    /// or imported — another account never sees them.
    #[tokio::test]
    async fn another_key_wipes_the_data_of_the_one_before_and_the_same_key_keeps_it() {
        let (svc, _) = service(false).await;
        let a = keys::generate();
        svc.import_nsec(&keys::nsec(&a)).await.unwrap();
        assert!(!svc.take_wiped(), "a fresh store has nothing to wipe");
        messenger_store::chats::ensure_dm(&svc.store, "bb", None).await.unwrap();
        svc.delete().await.unwrap();

        // The same key keeps everything.
        svc.import_nsec(&keys::nsec(&a)).await.unwrap();
        assert!(!svc.take_wiped());
        assert_eq!(chats(&svc).await, 1);
        svc.delete().await.unwrap();

        // Another key, created: the chats go.
        let created = svc.create("pw").await.unwrap();
        assert!(svc.take_wiped());
        assert!(!svc.take_wiped(), "told once");
        assert_eq!(chats(&svc).await, 0);
        assert_eq!(
            messenger_store::account::owner(&svc.store).await.unwrap().as_deref(),
            Some(created.identity.pubkey.as_hex())
        );

        // And imported: the chats of the created one go too.
        messenger_store::chats::ensure_dm(&svc.store, "cc", None).await.unwrap();
        svc.delete().await.unwrap();
        svc.import_nsec(&keys::nsec(&a)).await.unwrap();
        assert!(svc.take_wiped());
        assert_eq!(chats(&svc).await, 0);
    }

    /// A store from before the record of its owner: the identity went with
    /// nothing written; the author of a message sent from here tells whose
    /// the data is, and with nothing to tell the data goes.
    #[tokio::test]
    async fn a_store_from_before_the_record_is_told_by_what_it_sent() {
        let (svc, _) = service(false).await;
        let a = keys::generate();
        let sent = |id: &'static str, author: String| {
            let store = svc.store.clone();
            async move {
                messenger_store::chats::ensure_dm(&store, "bb", None).await.unwrap();
                sqlx_insert_sent(&store, id, &author).await;
            }
        };
        sent("m1", a.public_key().to_hex()).await;
        svc.import_nsec(&keys::nsec(&a)).await.unwrap();
        assert!(!svc.take_wiped());
        assert_eq!(chats(&svc).await, 1);

        // Gone without a record (as before this version), another key comes.
        forget_unrecorded(&svc).await;
        svc.create("pw").await.unwrap();
        assert!(svc.take_wiped());
        assert_eq!(chats(&svc).await, 0);

        // Nothing sent: nobody is known and the data goes.
        messenger_store::chats::ensure_dm(&svc.store, "dd", None).await.unwrap();
        forget_unrecorded(&svc).await;
        svc.import_nsec(&keys::nsec(&a)).await.unwrap();
        assert!(svc.take_wiped());
        assert_eq!(chats(&svc).await, 0);
    }

    /// The identity removed as the builds before removed it: no record of the owner.
    async fn forget_unrecorded(svc: &IdentityService) {
        svc.delete().await.unwrap();
        messenger_store::settings::delete(&svc.store, messenger_store::account::KEY_OWNER).await.unwrap();
    }

    async fn sqlx_insert_sent(store: &Store, id: &str, author: &str) {
        let m = messenger_store::messages::NewMessage {
            id: id.into(),
            chat_id: "dm:bb".into(),
            wire_id: None,
            direction: "out".into(),
            status: "sent".into(),
            content_type: "text".into(),
            text: Some("hi".into()),
            envelope_json: "{}".into(),
            sender_pubkey: author.into(),
            reply_to_id: None,
            target_id: None,
            created_at: 1,
            is_hidden: false,
            outbox_local_id: None,
            media_json: None,
        };
        assert!(messenger_store::messages::insert(store, &m).await.unwrap());
    }

    #[tokio::test]
    async fn tampered_secret_is_detected() {
        let (svc, secrets) = service(false).await;
        svc.create("x").await.unwrap();
        let other = keys::generate();
        secrets.put(SECRET_KEY_REF, &other.secret_key().to_secret_bytes()).await.unwrap();
        assert!(matches!(svc.load_keys().await, Err(MessengerError::Crypto(_))));
    }
}
