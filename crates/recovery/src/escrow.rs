// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Escrow v1 (`internal/cloud-recovery.md` §4): what the owner's device
//! puts into vstore at `escrow/<hex sha256(auth_pub)>` so that the Recovery
//! Key alone brings the account back.
//!
//! ```text
//! header: v(1) ‖ kdf_m_kib u32 ‖ kdf_t u32 ‖ kdf_p u32 ‖ salt(16) ‖ nonce(24)
//! body:   XChaCha20-Poly1305(key, nonce, aad, cbor(plaintext))
//! key  =  Argon2id(password = HKDF(RK, escrow-enc/1), salt, header params)
//! aad  =  "veydan-escrow/1" ‖ auth_pub (32 bytes x-only)
//! ```
//!
//! The reader knows no npub before it opens the escrow, so the AAD binds
//! the ciphertext to its address instead: moved to another address, it
//! does not open.

use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::cbor::{Decoder, Encoder, Malformed, MapReader};
use crate::kdf::{self, KdfError, KdfParams, KEY_LEN, NONCE_LEN, SALT_LEN, TAG_LEN};
use crate::rk::{self, RecoveryKey, RkError};

pub const ESCROW_VERSION: u8 = 1;
pub const ESCROW_AAD: &[u8] = b"veydan-escrow/1";
/// vstore refuses a larger body (`spec/vstore.md` §8).
pub const ESCROW_MAX_BYTES: usize = 4096;
/// The names of the storage of the sync vault the escrow describes
/// (`sync.storage`): Veydan Cloud, and the user's own.
pub const STORAGE_CLOUD: &str = "cloud";
pub const STORAGE_S3: &str = "s3";
pub const STORAGE_WEBDAV: &str = "webdav";
pub const STORAGE_FOLDER: &str = "folder";
const HEADER_LEN: usize = 1 + 12 + SALT_LEN + NONCE_LEN;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EscrowError {
    #[error("the escrow is malformed")]
    Format,
    #[error("escrow version {0} is not 1")]
    Version(u8),
    #[error("the escrow is larger than 4 KiB")]
    TooLarge,
    #[error("the kdf parameters of the escrow are above the ceilings")]
    KdfParams,
    #[error("the escrow does not open with this recovery key")]
    Decrypt,
    #[error("the nsec of the escrow does not give its npub")]
    Identity,
    #[error("kdf failed")]
    Kdf,
    #[error("the system random source failed")]
    Random,
    #[error(transparent)]
    Key(#[from] RkError),
}

impl From<Malformed> for EscrowError {
    fn from(_: Malformed) -> Self {
        EscrowError::Format
    }
}

impl From<KdfError> for EscrowError {
    fn from(e: KdfError) -> Self {
        match e {
            KdfError::Params => EscrowError::KdfParams,
            KdfError::Kdf | KdfError::Encrypt => EscrowError::Kdf,
            KdfError::Decrypt => EscrowError::Decrypt,
            KdfError::Random => EscrowError::Random,
        }
    }
}

/// Where the sync vault lives, with what a clean device needs to reach
/// it (§4): nothing for Veydan Cloud (the hosts are the manifest's, the
/// requests are signed by the identity), the bucket and its keys for S3,
/// the address and the login for WebDAV, the path for a folder — which is
/// the old device's, and may not exist on the new one.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub enum Storage {
    Cloud,
    S3 {
        endpoint: String,
        region: String,
        bucket: String,
        prefix: String,
        access_key: String,
        secret_key: String,
        path_style: bool,
    },
    WebDav {
        url: String,
        username: String,
        password: String,
    },
    Folder {
        path: String,
    },
}

impl Storage {
    /// The name of the storage on the wire, `sync.storage`.
    pub fn name(&self) -> &'static str {
        match self {
            Storage::Cloud => STORAGE_CLOUD,
            Storage::S3 { .. } => STORAGE_S3,
            Storage::WebDav { .. } => STORAGE_WEBDAV,
            Storage::Folder { .. } => STORAGE_FOLDER,
        }
    }
}

/// The name alone: the access is a secret.
impl std::fmt::Debug for Storage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// The sync vault: its id, its master key and where it lives.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct SyncKeys {
    pub vault_id: String,
    pub vmk: [u8; 32],
    pub storage: Storage,
}

impl SyncKeys {
    pub fn cloud(vault_id: impl Into<String>, vmk: [u8; 32]) -> Self {
        Self {
            vault_id: vault_id.into(),
            vmk,
            storage: Storage::Cloud,
        }
    }
}

impl std::fmt::Debug for SyncKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncKeys")
            .field("vault_id", &self.vault_id)
            .field("storage", &self.storage)
            .finish_non_exhaustive()
    }
}

/// The lock: its vault id and its key in the clear, the insurance for the
/// case the lock row in the vault is another one (§7).
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct LockKey {
    pub vault_id: String,
    pub vault_key: [u8; 32],
}

impl std::fmt::Debug for LockKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LockKey")
            .field("vault_id", &self.vault_id)
            .finish_non_exhaustive()
    }
}

/// What the restored device knows about the trusted people without asking
/// vcloud: the threshold and the generation of the shares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Zeroize)]
pub struct RecoveryParams {
    pub k: u8,
    pub n: u8,
    pub generation: u32,
}

/// The plaintext of the escrow, the fields of §4 exactly.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct Escrow {
    /// The owner, x-only.
    pub npub: [u8; 32],
    /// Unix seconds of the write.
    pub created_at: u64,
    /// The generation of the Recovery Key it is sealed under.
    pub generation: u32,
    pub nsec: [u8; 32],
    pub sync: SyncKeys,
    /// `None` when the lock was never set up.
    pub lock: Option<LockKey>,
    pub recovery: RecoveryParams,
}

impl std::fmt::Debug for Escrow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Escrow")
            .field("npub", &hex::encode(self.npub))
            .field("created_at", &self.created_at)
            .field("generation", &self.generation)
            .field("sync", &self.sync)
            .field("lock", &self.lock)
            .field("recovery", &self.recovery)
            .finish_non_exhaustive()
    }
}

impl Escrow {
    /// The CBOR of the plaintext, keys in a fixed order.
    pub fn to_cbor(&self) -> Zeroizing<Vec<u8>> {
        let mut e = Encoder::new();
        e.map(8)
            .text("v")
            .uint(ESCROW_VERSION as u64)
            .text("npub")
            .bytes(&self.npub)
            .text("created_at")
            .uint(self.created_at)
            .text("generation")
            .uint(self.generation as u64)
            .text("nsec")
            .bytes(&self.nsec)
            .text("sync");
        encode_sync(&mut e, &self.sync);
        e.text("lock");
        match &self.lock {
            None => {
                e.null();
            }
            Some(lock) => {
                e.map(2)
                    .text("vault_id")
                    .text(&lock.vault_id)
                    .text("vault_key")
                    .bytes(&lock.vault_key);
            }
        }
        e.text("recovery")
            .map(3)
            .text("k")
            .uint(self.recovery.k as u64)
            .text("n")
            .uint(self.recovery.n as u64)
            .text("generation")
            .uint(self.recovery.generation as u64);
        Zeroizing::new(e.finish())
    }

    pub fn from_cbor(bytes: &[u8]) -> Result<Self, EscrowError> {
        let mut dec = Decoder::new(bytes);
        let mut m = MapReader::open(&mut dec, 8)?;
        let v = m.key("v")?.u8()?;
        if v != ESCROW_VERSION {
            return Err(EscrowError::Version(v));
        }
        let npub = m.key("npub")?.array::<32>()?;
        let created_at = m.key("created_at")?.uint()?;
        let generation = m.key("generation")?.u32()?;
        let nsec = m.key("nsec")?.array::<32>()?;
        let sync = decode_sync(m.key("sync")?)?;
        let lock_dec = m.key("lock")?;
        let lock = if lock_dec.null_or_map()? {
            None
        } else {
            let mut lm = MapReader::open(lock_dec, 2)?;
            let vault_id = lm.key("vault_id")?.text()?.to_owned();
            let vault_key = lm.key("vault_key")?.array::<32>()?;
            lm.close()?;
            Some(LockKey {
                vault_id,
                vault_key,
            })
        };
        let mut rm = MapReader::open(m.key("recovery")?, 3)?;
        let k = rm.key("k")?.u8()?;
        let n = rm.key("n")?.u8()?;
        let rgen = rm.key("generation")?.u32()?;
        rm.close()?;
        m.close()?;
        dec.end()?;
        Ok(Escrow {
            npub,
            created_at,
            generation,
            nsec,
            sync,
            lock,
            recovery: RecoveryParams {
                k,
                n,
                generation: rgen,
            },
        })
    }
}

/// `sync` on the wire: three keys for Veydan Cloud, as the first escrows
/// were written, and a fourth, `access`, for a storage of the user's own.
pub(crate) fn encode_sync(e: &mut Encoder, sync: &SyncKeys) {
    let own = !matches!(sync.storage, Storage::Cloud);
    e.map(if own { 4 } else { 3 })
        .text("vault_id")
        .text(&sync.vault_id)
        .text("vmk")
        .bytes(&sync.vmk)
        .text("storage")
        .text(sync.storage.name());
    if own {
        e.text("access");
    }
    match &sync.storage {
        Storage::Cloud => {}
        Storage::S3 {
            endpoint,
            region,
            bucket,
            prefix,
            access_key,
            secret_key,
            path_style,
        } => {
            e.map(7)
                .text("endpoint")
                .text(endpoint)
                .text("region")
                .text(region)
                .text("bucket")
                .text(bucket)
                .text("prefix")
                .text(prefix)
                .text("access_key")
                .text(access_key)
                .text("secret_key")
                .text(secret_key)
                .text("path_style")
                .uint(*path_style as u64);
        }
        Storage::WebDav {
            url,
            username,
            password,
        } => {
            e.map(3)
                .text("url")
                .text(url)
                .text("username")
                .text(username)
                .text("password")
                .text(password);
        }
        Storage::Folder { path } => {
            e.map(1).text("path").text(path);
        }
    }
}

pub(crate) fn decode_sync(dec: &mut Decoder<'_>) -> Result<SyncKeys, Malformed> {
    let mut sm = MapReader::open_between(dec, 3, 4)?;
    let vault_id = sm.key("vault_id")?.text()?.to_owned();
    let vmk = sm.key("vmk")?.array::<32>()?;
    let name = sm.key("storage")?.text()?.to_owned();
    let storage = if name == STORAGE_CLOUD {
        Storage::Cloud
    } else {
        decode_access(&name, sm.key("access")?)?
    };
    sm.close()?;
    Ok(SyncKeys {
        vault_id,
        vmk,
        storage,
    })
}

/// The access of a storage of the user's own, by its name; a name this
/// version does not know is malformed.
fn decode_access(name: &str, dec: &mut Decoder<'_>) -> Result<Storage, Malformed> {
    match name {
        STORAGE_S3 => {
            let mut m = MapReader::open(dec, 7)?;
            let endpoint = m.key("endpoint")?.text()?.to_owned();
            let region = m.key("region")?.text()?.to_owned();
            let bucket = m.key("bucket")?.text()?.to_owned();
            let prefix = m.key("prefix")?.text()?.to_owned();
            let access_key = m.key("access_key")?.text()?.to_owned();
            let secret_key = m.key("secret_key")?.text()?.to_owned();
            let path_style = match m.key("path_style")?.uint()? {
                0 => false,
                1 => true,
                _ => return Err(Malformed),
            };
            m.close()?;
            Ok(Storage::S3 {
                endpoint,
                region,
                bucket,
                prefix,
                access_key,
                secret_key,
                path_style,
            })
        }
        STORAGE_WEBDAV => {
            let mut m = MapReader::open(dec, 3)?;
            let url = m.key("url")?.text()?.to_owned();
            let username = m.key("username")?.text()?.to_owned();
            let password = m.key("password")?.text()?.to_owned();
            m.close()?;
            Ok(Storage::WebDav {
                url,
                username,
                password,
            })
        }
        STORAGE_FOLDER => {
            let mut m = MapReader::open(dec, 1)?;
            let path = m.key("path")?.text()?.to_owned();
            m.close()?;
            Ok(Storage::Folder { path })
        }
        _ => Err(Malformed),
    }
}

/// The open header of a sealed escrow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub version: u8,
    pub kdf: KdfParams,
    pub salt: [u8; SALT_LEN],
    pub nonce: [u8; NONCE_LEN],
}

impl Header {
    pub fn to_bytes(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[0] = self.version;
        out[1..5].copy_from_slice(&self.kdf.memory_kib.to_be_bytes());
        out[5..9].copy_from_slice(&self.kdf.iterations.to_be_bytes());
        out[9..13].copy_from_slice(&self.kdf.parallelism.to_be_bytes());
        out[13..29].copy_from_slice(&self.salt);
        out[29..53].copy_from_slice(&self.nonce);
        out
    }

    /// The header of `bytes`, checked for the version and the ceilings; the
    /// body follows it.
    pub fn parse(bytes: &[u8]) -> Result<(Self, &[u8]), EscrowError> {
        if bytes.len() > ESCROW_MAX_BYTES {
            return Err(EscrowError::TooLarge);
        }
        if bytes.len() < HEADER_LEN + TAG_LEN {
            return Err(EscrowError::Format);
        }
        if bytes[0] != ESCROW_VERSION {
            return Err(EscrowError::Version(bytes[0]));
        }
        let u32_at = |at: usize| {
            u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        let kdf = KdfParams {
            memory_kib: u32_at(1),
            iterations: u32_at(5),
            parallelism: u32_at(9),
        };
        if !kdf.within_ceilings() {
            return Err(EscrowError::KdfParams);
        }
        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(&bytes[13..29]);
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&bytes[29..53]);
        Ok((
            Header {
                version: bytes[0],
                kdf,
                salt,
                nonce,
            },
            &bytes[HEADER_LEN..],
        ))
    }
}

fn aad(auth_pub: &[u8; 32]) -> [u8; 47] {
    let mut out = [0u8; 47];
    out[..15].copy_from_slice(ESCROW_AAD);
    out[15..].copy_from_slice(auth_pub);
    out
}

fn key_for(
    rk: &RecoveryKey,
    params: &KdfParams,
    salt: &[u8; SALT_LEN],
) -> Result<Zeroizing<[u8; KEY_LEN]>, EscrowError> {
    let password = rk.escrow_enc();
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    kdf::argon2id(password.as_slice(), salt, params, key.as_mut_slice())?;
    Ok(key)
}

/// Seals the escrow under the key with the production parameters, a fresh
/// salt and a fresh nonce.
pub fn seal(rk: &RecoveryKey, escrow: &Escrow) -> Result<Vec<u8>, EscrowError> {
    let salt = kdf::random_array()?;
    let nonce = kdf::random_array()?;
    seal_with(rk, escrow, &KdfParams::production(), salt, nonce)
}

/// `seal` with the parameters, the salt and the nonce given: for test
/// vectors.
pub fn seal_with(
    rk: &RecoveryKey,
    escrow: &Escrow,
    params: &KdfParams,
    salt: [u8; SALT_LEN],
    nonce: [u8; NONCE_LEN],
) -> Result<Vec<u8>, EscrowError> {
    let auth_pub = rk.escrow_auth()?.public_x_only();
    let key = key_for(rk, params, &salt)?;
    let plaintext = escrow.to_cbor();
    let body = kdf::seal(&key, &nonce, &aad(&auth_pub), &plaintext)?;
    let header = Header {
        version: ESCROW_VERSION,
        kdf: *params,
        salt,
        nonce,
    };
    let mut out = Vec::with_capacity(HEADER_LEN + body.len());
    out.extend_from_slice(&header.to_bytes());
    out.extend_from_slice(&body);
    if out.len() > ESCROW_MAX_BYTES {
        return Err(EscrowError::TooLarge);
    }
    Ok(out)
}

/// Opens a sealed escrow with the key. The header is checked before any
/// work; a wrong key, a wrong address and a changed byte all end in
/// `Decrypt`.
pub fn open(rk: &RecoveryKey, bytes: &[u8]) -> Result<Escrow, EscrowError> {
    let (header, body) = Header::parse(bytes)?;
    let auth_pub = rk.escrow_auth()?.public_x_only();
    let key = key_for(rk, &header.kdf, &header.salt)?;
    let plaintext = kdf::open(&key, &header.nonce, &aad(&auth_pub), body)?;
    let escrow = Escrow::from_cbor(&plaintext)?;
    match rk::npub_of(&escrow.nsec) {
        Some(npub) if npub == escrow.npub => Ok(escrow),
        _ => Err(EscrowError::Identity),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RK: [u8; 16] = [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
        0xff,
    ];
    const NSEC: [u8; 32] = [0x42; 32];
    /// Small parameters: the format is under test, not the cost.
    const FAST: KdfParams = KdfParams {
        memory_kib: 64,
        iterations: 1,
        parallelism: 1,
    };

    fn rk() -> RecoveryKey {
        RecoveryKey::from_bytes(RK).unwrap()
    }

    fn sample(lock: bool) -> Escrow {
        Escrow {
            npub: rk::npub_of(&NSEC).unwrap(),
            created_at: 1_791_000_000,
            generation: 2,
            nsec: NSEC,
            sync: SyncKeys::cloud("3f2a9c1e-5b7d-4e8a-9c0b-1d2e3f4a5b6c", [0x77; 32]),
            lock: lock.then(|| LockKey {
                vault_id: "b1c2d3e4-f5a6-4b7c-8d9e-0f1a2b3c4d5e".to_owned(),
                vault_key: [0x99; 32],
            }),
            recovery: RecoveryParams {
                k: 3,
                n: 5,
                generation: 2,
            },
        }
    }

    #[test]
    fn cbor_roundtrip() {
        for lock in [true, false] {
            let e = sample(lock);
            let bytes = e.to_cbor();
            assert_eq!(bytes[0], 0xa8);
            let back = Escrow::from_cbor(&bytes).unwrap();
            assert_eq!(back, e);
        }
        let e = sample(false);
        assert!(format!("{e:?}").contains("lock: None"));
        assert!(!format!("{e:?}").contains("4242"));
    }

    fn own_storages() -> Vec<Storage> {
        vec![
            Storage::S3 {
                endpoint: "https://s3.example.com".into(),
                region: "eu-central-1".into(),
                bucket: "notes".into(),
                prefix: "veydan".into(),
                access_key: "AKIAEXAMPLE".into(),
                secret_key: "s3-secret-of-the-user".into(),
                path_style: true,
            },
            Storage::WebDav {
                url: "https://dav.example.com/veydan".into(),
                username: "anna".into(),
                password: "dav-secret-of-the-user".into(),
            },
            Storage::Folder {
                path: "/home/anna/Sync/veydan".into(),
            },
        ]
    }

    #[test]
    fn a_storage_of_the_user_travels_with_its_access() {
        // Veydan Cloud is written as the first escrows were: three keys.
        let cloud = sample(true).to_cbor();
        assert!(!cloud.windows(6).any(|w| w == b"access"));
        for storage in own_storages() {
            let mut e = sample(true);
            e.sync.storage = storage.clone();
            let bytes = e.to_cbor();
            assert!(bytes.windows(6).any(|w| w == b"access"), "{storage:?}");
            let back = Escrow::from_cbor(&bytes).unwrap();
            assert_eq!(back, e);
            assert_eq!(back.sync.storage, storage);
            let sealed = seal_with(&rk(), &e, &FAST, [1u8; 16], [2u8; 24]).unwrap();
            assert_eq!(open(&rk(), &sealed).unwrap(), e);
            // The access is a secret: the name alone is printed.
            let printed = format!("{e:?}");
            assert!(printed.contains(storage.name()), "{printed}");
            assert!(!printed.contains("secret-of-the-user"), "{printed}");
            assert!(!printed.contains("example.com"), "{printed}");
        }
    }

    #[test]
    fn a_storage_without_its_access_or_of_an_unknown_name_is_refused() {
        let named = |name: &str, access: Option<&dyn Fn(&mut Encoder)>| {
            let mut e = Encoder::new();
            e.map(if access.is_some() { 4 } else { 3 })
                .text("vault_id")
                .text("3f2a9c1e")
                .text("vmk")
                .bytes(&[0x77; 32])
                .text("storage")
                .text(name);
            if let Some(access) = access {
                e.text("access");
                access(&mut e);
            }
            let bytes = e.finish();
            decode_sync(&mut Decoder::new(&bytes)).map(|s| s.storage.clone())
        };
        let path = |e: &mut Encoder| {
            e.map(1).text("path").text("/sync");
        };
        assert_eq!(named(STORAGE_CLOUD, None), Ok(Storage::Cloud));
        assert_eq!(named(STORAGE_FOLDER, Some(&path)), Ok(Storage::Folder { path: "/sync".into() }));
        assert_eq!(named(STORAGE_S3, None), Err(Malformed));
        assert_eq!(named(STORAGE_CLOUD, Some(&path)), Err(Malformed));
        assert_eq!(named(STORAGE_WEBDAV, Some(&path)), Err(Malformed));
        assert_eq!(named("ftp", Some(&path)), Err(Malformed));
        assert_eq!(named("ftp", None), Err(Malformed));
    }

    #[test]
    fn cbor_rejects_wrong_shapes() {
        let mut bytes = sample(true).to_cbor().to_vec();
        assert!(Escrow::from_cbor(&bytes[..bytes.len() - 1]).is_err());
        bytes.push(0);
        assert_eq!(Escrow::from_cbor(&bytes), Err(EscrowError::Format));
        let mut v2 = sample(true).to_cbor().to_vec();
        // "v" is the first key: a8 61 76 01 -> version byte at 3.
        assert_eq!(v2[3], 1);
        v2[3] = 2;
        assert_eq!(Escrow::from_cbor(&v2), Err(EscrowError::Version(2)));
        assert_eq!(Escrow::from_cbor(&[]), Err(EscrowError::Format));
    }

    #[test]
    fn seal_open_roundtrip_fast() {
        let e = sample(true);
        let sealed = seal_with(&rk(), &e, &FAST, [1u8; 16], [2u8; 24]).unwrap();
        let (header, body) = Header::parse(&sealed).unwrap();
        assert_eq!(header.version, 1);
        assert_eq!(header.kdf, FAST);
        assert_eq!(header.salt, [1u8; 16]);
        assert_eq!(header.nonce, [2u8; 24]);
        assert_eq!(body.len(), e.to_cbor().len() + TAG_LEN);
        assert_eq!(open(&rk(), &sealed).unwrap(), e);
        // Deterministic with fixed salt and nonce.
        assert_eq!(
            seal_with(&rk(), &e, &FAST, [1u8; 16], [2u8; 24]).unwrap(),
            sealed
        );
    }

    #[test]
    fn seal_open_production_size() {
        let e = sample(true);
        let sealed = seal(&rk(), &e).unwrap();
        let (header, _) = Header::parse(&sealed).unwrap();
        assert_eq!(header.kdf, KdfParams::production());
        assert_eq!(&sealed[1..13], &[0, 1, 0, 0, 0, 0, 0, 3, 0, 0, 0, 1]);
        // About 300 bytes: the two uuids and four 32-byte keys.
        assert!((300..=450).contains(&sealed.len()), "{}", sealed.len());
        assert_eq!(open(&rk(), &sealed).unwrap(), e);
        // Two seals differ in salt and nonce.
        assert_ne!(seal(&rk(), &e).unwrap(), sealed);
    }

    #[test]
    fn wrong_key_and_tamper_fail() {
        let e = sample(false);
        let sealed = seal_with(&rk(), &e, &FAST, [1u8; 16], [2u8; 24]).unwrap();
        let other = RecoveryKey::from_bytes([5u8; 16]).unwrap();
        assert_eq!(open(&other, &sealed), Err(EscrowError::Decrypt));
        for at in [HEADER_LEN - 1, HEADER_LEN, sealed.len() - 1, 20] {
            let mut t = sealed.clone();
            t[at] ^= 1;
            assert_eq!(open(&rk(), &t), Err(EscrowError::Decrypt), "byte {at}");
        }
        let mut v = sealed.clone();
        v[0] = 2;
        assert_eq!(open(&rk(), &v), Err(EscrowError::Version(2)));
        assert_eq!(
            open(&rk(), &sealed[..HEADER_LEN + 3]),
            Err(EscrowError::Format)
        );
    }

    #[test]
    fn foreign_address_in_aad_fails() {
        // The body sealed for one auth_pub, opened as if it sat at another
        // address: the AAD differs.
        let e = sample(false);
        let sealed = seal_with(&rk(), &e, &FAST, [1u8; 16], [2u8; 24]).unwrap();
        let (header, body) = Header::parse(&sealed).unwrap();
        let key = key_for(&rk(), &header.kdf, &header.salt).unwrap();
        let right = rk().escrow_auth().unwrap().public_x_only();
        assert!(kdf::open(&key, &header.nonce, &aad(&right), body).is_ok());
        let wrong = [0xee; 32];
        assert_eq!(
            kdf::open(&key, &header.nonce, &aad(&wrong), body),
            Err(KdfError::Decrypt)
        );
    }

    #[test]
    fn kdf_ceilings_refused_before_work() {
        let e = sample(false);
        let mut sealed = seal_with(&rk(), &e, &FAST, [1u8; 16], [2u8; 24]).unwrap();
        // 2 GiB of memory.
        sealed[1..5].copy_from_slice(&(2u32 * 1024 * 1024).to_be_bytes());
        let started = std::time::Instant::now();
        assert_eq!(open(&rk(), &sealed), Err(EscrowError::KdfParams));
        assert!(started.elapsed().as_millis() < 100);
        let mut t = seal_with(&rk(), &e, &FAST, [1u8; 16], [2u8; 24]).unwrap();
        t[5..9].copy_from_slice(&17u32.to_be_bytes());
        assert_eq!(open(&rk(), &t), Err(EscrowError::KdfParams));
        let mut p = seal_with(&rk(), &e, &FAST, [1u8; 16], [2u8; 24]).unwrap();
        p[9..13].copy_from_slice(&0u32.to_be_bytes());
        assert_eq!(open(&rk(), &p), Err(EscrowError::KdfParams));
    }

    #[test]
    fn too_large_refused() {
        let big = vec![1u8; ESCROW_MAX_BYTES + 1];
        assert_eq!(open(&rk(), &big), Err(EscrowError::TooLarge));
        let mut e = sample(false);
        e.sync.vault_id = "x".repeat(ESCROW_MAX_BYTES);
        assert_eq!(
            seal_with(&rk(), &e, &FAST, [1u8; 16], [2u8; 24]),
            Err(EscrowError::TooLarge)
        );
    }

    #[test]
    fn identity_mismatch_refused() {
        let mut e = sample(false);
        e.npub = [0x01; 32];
        let sealed = seal_with(&rk(), &e, &FAST, [1u8; 16], [2u8; 24]).unwrap();
        assert_eq!(open(&rk(), &sealed), Err(EscrowError::Identity));
    }
}
