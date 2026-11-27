// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The messenger inside the share extension of the iPhone («Поделиться»
//! into Veydan from another app, internal/messenger-wire.md §5). The
//! extension is a process of its own with no page and no Tauri: it gets
//! what the user shares, shows the chats and sends by itself, the app
//! closed or not. For that it opens the same data file the app keeps in
//! the container of the app group (`crates/shell/src/ios_group.rs`), opens
//! the lock of the app with its default secret or the PIN the user types,
//! takes the messenger's key from the lock's secret box as the app does
//! (`HostSecretStore` of messenger-app) and starts the runtime without the
//! engine of calls. The C functions (`include/veydan_share.h`) are what the
//! Swift of the extension calls; one session at a time.
//!
//! The app and the extension write one SQLite file: SQLite's lock keeps
//! them apart, and the app reads the messages the extension wrote when it
//! comes back to the front (its chat reloads on visibility).

use messenger_core::{MessengerConfig, SecretStore, TextFormat};
use messenger_runtime::MessengerRuntime;
use serde::Serialize;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use veydan_lock::Lock;
use zeroize::Zeroizing;

/// What `veydan_share_open` answers (`veydan_share.h`).
pub const OK: i32 = 0;
pub const LOCKED: i32 = 1;
pub const NO_IDENTITY: i32 = 2;
pub const ERROR: i32 = 3;

/// The stages of a send (`veydan_share.h`).
pub const STAGE_PREPARING: i32 = 1;
pub const STAGE_UPLOADING: i32 = 2;
pub const STAGE_SENDING: i32 = 3;
pub const STAGE_DONE: i32 = 4;

/// The data file of the app under its data folder (`veydan_core::db::DB_FILE`).
const DB_FILE: &str = "app.db";
/// The messenger's folder under the data folder (`DATA_SUBDIR` of messenger-app).
const MESSENGER_SUBDIR: &str = "messenger";
/// The secret box of the lock the messenger keeps its key in (`SECRET_BOX` of messenger-app).
const SECRET_BOX: &str = "messenger";
/// The longest a send waits for its files to go up and its messages to go out.
const SEND_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// How often the send looks at its transfers and messages.
const POLL: Duration = Duration::from_millis(400);

struct Session {
    rt: tokio::runtime::Runtime,
    #[allow(dead_code)]
    lock: Arc<Lock>,
    messenger: Arc<MessengerRuntime>,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

fn fail(e: impl std::fmt::Display) -> i32 {
    *LAST_ERROR.lock().unwrap_or_else(|p| p.into_inner()) = Some(e.to_string());
    ERROR
}

/// The messenger's key, read from the lock's secret box as the app reads it.
struct LockSecrets(Arc<Lock>);

fn locked(e: veydan_lock::Error) -> messenger_core::MessengerError {
    match e {
        veydan_lock::Error::VaultLocked | veydan_lock::Error::VaultMismatch => messenger_core::MessengerError::SecretsLocked,
        veydan_lock::Error::Db(m) => messenger_core::MessengerError::Storage(m),
        other => messenger_core::MessengerError::Crypto(other.to_string()),
    }
}

#[async_trait::async_trait]
impl SecretStore for LockSecrets {
    async fn get(&self, key: &str) -> messenger_core::Result<Option<Zeroizing<Vec<u8>>>> {
        self.0.secret_box(SECRET_BOX).get(key).await.map_err(locked)
    }
    async fn put(&self, key: &str, value: &[u8]) -> messenger_core::Result<()> {
        self.0.secret_box(SECRET_BOX).put(key, value).await.map_err(locked)
    }
    async fn delete(&self, key: &str) -> messenger_core::Result<()> {
        self.0.secret_box(SECRET_BOX).delete(key).await.map_err(locked)
    }
    async fn is_unlocked(&self) -> bool {
        self.0.secret_box(SECRET_BOX).is_unlocked()
    }
}

/// A chat a share may go to, as the extension lists it.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ChatRow {
    pub id: String,
    pub title: String,
    /// `dm` or `group`.
    pub kind: String,
    pub last_message_at: Option<i64>,
}

/// The chats a share may go to: those a forward may go to (`forwardTarget`
/// of the page), the newest first. Shaped from what the runtime lists.
pub fn targets(chats: impl IntoIterator<Item = messenger_dm_view::ChatView>) -> Vec<ChatRow> {
    let mut rows: Vec<ChatRow> = chats
        .into_iter()
        .filter(|c| c.can_send && !c.archived && !c.banned && !c.bot)
        .filter(|c| match c.kind.as_str() {
            "group" => c.id.starts_with("group:"),
            "dm" => c.id.starts_with("dm:") && c.is_contact && c.mode == "full_chat",
            _ => false,
        })
        .map(|c| ChatRow { id: c.id, title: c.title, kind: c.kind, last_message_at: c.last_message_at })
        .collect();
    rows.sort_by(|a, b| b.last_message_at.cmp(&a.last_message_at).then_with(|| a.title.cmp(&b.title)));
    rows
}

/// The view of a chat as the dm crate names it, reached through the runtime's re-export.
mod messenger_dm_view {
    pub use messenger_runtime::ChatView;
}

/// The session opened over the data at `data_dir` (what `veydan_share_open` does); the example `probe` runs it on a computer.
#[doc(hidden)]
pub async fn open(data_dir: &Path, secret: Option<&str>) -> Result<(Arc<Lock>, Arc<MessengerRuntime>), (i32, String)> {
    let db = data_dir.join(DB_FILE);
    if !db.is_file() {
        return Err((ERROR, format!("no data file at {}", db.display())));
    }
    let pool = SqlitePoolOptions::new()
        .max_connections(3)
        .connect_with(SqliteConnectOptions::new().filename(&db))
        .await
        .map_err(|e| (ERROR, format!("the data file could not be opened: {e}")))?;
    let lock = Arc::new(Lock::new(pool));
    lock.open_default().await.map_err(|e| (ERROR, format!("the lock could not be read: {e}")))?;
    if !lock.secret_box(SECRET_BOX).is_unlocked() {
        let Some(secret) = secret.filter(|s| !s.is_empty()) else {
            return Err((LOCKED, "the app is locked".into()));
        };
        match lock.unlock(secret).await {
            Ok(true) => {}
            Ok(false) => return Err((LOCKED, "the secret does not open the lock".into())),
            Err(e) => return Err((LOCKED, format!("the lock could not be opened: {e}"))),
        }
        if !lock.secret_box(SECRET_BOX).is_unlocked() {
            return Err((LOCKED, "the secret does not open the lock".into()));
        }
    }
    let secrets: Arc<dyn SecretStore> = Arc::new(LockSecrets(lock.clone()));
    let config = MessengerConfig::new(data_dir.join(MESSENGER_SUBDIR));
    let messenger = MessengerRuntime::start(config, secrets)
        .await
        .map_err(|e| (ERROR, format!("the messenger could not start: {e}")))?;
    match messenger.identity().get().await {
        Ok(Some(_)) => {}
        Ok(None) => return Err((NO_IDENTITY, "no identity".into())),
        Err(e) => return Err((ERROR, format!("the identity could not be read: {e}"))),
    }
    Ok((lock, Arc::new(messenger)))
}

/// Where the send stands, told to the extension.
pub type Progress<'a> = &'a mut dyn FnMut(i32, u64, u64);

/// A send and its wait (what `veydan_share_send` does); the example `probe` runs it on a computer.
#[doc(hidden)]
pub async fn send(messenger: &MessengerRuntime, chat_id: &str, text: Option<&str>, paths: &[PathBuf], progress: Progress<'_>) -> Result<(), String> {
    progress(STAGE_PREPARING, 0, 0);
    let text = text.map(str::trim).filter(|t| !t.is_empty());
    // The views the sends answer are placeholders: a file's message gets its
    // id when it goes out. What is watched instead: the uploads under way,
    // then the messages of mine the chat holds from this moment on.
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) - 5;
    let expected = if paths.is_empty() {
        let Some(text) = text else {
            return Err("nothing to send".into());
        };
        messenger.dm_send_text(chat_id, text, None).await.map_err(|e| e.to_string())?;
        1
    } else {
        let batch = (paths.len() > 1).then(uuid_like);
        for (i, path) in paths.iter().enumerate() {
            let last = i + 1 == paths.len();
            messenger
                .dm_send_file_with(
                    chat_id,
                    path,
                    if last { text } else { None },
                    TextFormat::Plain,
                    batch.as_deref().map(|b| (b, Some(i as u32))),
                    false,
                    None,
                )
                .await
                .map_err(|e| e.to_string())?;
        }
        paths.len()
    };
    // The files go up, then the messages go out: watched until both are through.
    let started = Instant::now();
    loop {
        if started.elapsed() > SEND_TIMEOUT {
            return Err("the send did not finish in time".into());
        }
        let uploads: Vec<_> = messenger
            .media_transfers()
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|t| t.direction == "up")
            .collect();
        if let Some(failed) = uploads.iter().find(|t| t.status == "failed") {
            return Err(failed.failure_reason.clone().unwrap_or_else(|| "the upload failed".into()));
        }
        if !uploads.is_empty() {
            let total: u64 = uploads.iter().map(|t| t.size).sum();
            let done: u64 = uploads.iter().map(|t| t.done_bytes.min(t.size)).sum();
            progress(STAGE_UPLOADING, done, total);
        } else {
            progress(STAGE_SENDING, 0, 0);
            let recent = messenger.messages(chat_id, None, 100).await.map_err(|e| e.to_string())?;
            let mine: Vec<_> = recent.iter().filter(|m| m.direction == "out" && m.created_at >= since).collect();
            if mine.iter().any(|m| m.status == "failed") {
                return Err("the message could not be sent".into());
            }
            let out = mine.iter().filter(|m| m.status == "sent" || m.status == "received").count();
            if out >= expected {
                progress(STAGE_DONE, 0, 0);
                return Ok(());
            }
        }
        tokio::time::sleep(POLL).await;
    }
}

/// A batch id of the album: what `valid_batch` of the media crate takes (a UUID).
fn uuid_like() -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&now.to_le_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let h: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

fn c_str<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    // SAFETY: the caller hands a NUL-terminated string that outlives the call.
    unsafe { CStr::from_ptr(p) }.to_str().ok()
}

#[doc(hidden)]
pub fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()
}

/// # Safety
/// `data_dir` is a NUL-terminated path; `secret` the same or NULL.
#[no_mangle]
pub unsafe extern "C" fn veydan_share_open(data_dir: *const c_char, secret: *const c_char) -> i32 {
    veydan_share_close();
    let Some(dir) = c_str(data_dir) else {
        return fail("no data folder given");
    };
    let rt = match runtime() {
        Ok(rt) => rt,
        Err(e) => return fail(e),
    };
    match rt.block_on(open(Path::new(dir), c_str(secret))) {
        Ok((lock, messenger)) => {
            *SESSION.lock().unwrap_or_else(|p| p.into_inner()) = Some(Session { rt, lock, messenger });
            OK
        }
        Err((code, why)) => {
            *LAST_ERROR.lock().unwrap_or_else(|p| p.into_inner()) = Some(why);
            code
        }
    }
}

/// The chats a share may go to, as JSON; NULL without a session.
#[no_mangle]
pub extern "C" fn veydan_share_chats() -> *mut c_char {
    let guard = SESSION.lock().unwrap_or_else(|p| p.into_inner());
    let Some(s) = guard.as_ref() else {
        fail("no session");
        return std::ptr::null_mut();
    };
    match s.rt.block_on(s.messenger.chats(false)) {
        Ok(chats) => match serde_json::to_string(&targets(chats)) {
            Ok(json) => CString::new(json).map(CString::into_raw).unwrap_or(std::ptr::null_mut()),
            Err(e) => {
                fail(e);
                std::ptr::null_mut()
            }
        },
        Err(e) => {
            fail(e);
            std::ptr::null_mut()
        }
    }
}

/// # Safety
/// The strings are NUL-terminated (`text` and `paths_json` may be NULL); `progress` may be NULL.
#[no_mangle]
pub unsafe extern "C" fn veydan_share_send(
    chat_id: *const c_char,
    text: *const c_char,
    paths_json: *const c_char,
    progress: Option<unsafe extern "C" fn(*mut c_void, i32, u64, u64)>,
    ctx: *mut c_void,
) -> i32 {
    let Some(chat_id) = c_str(chat_id) else {
        return fail("no chat");
    };
    let paths: Vec<PathBuf> = match c_str(paths_json) {
        Some(json) if !json.trim().is_empty() => match serde_json::from_str::<Vec<String>>(json) {
            Ok(list) => list.into_iter().map(PathBuf::from).collect(),
            Err(e) => return fail(format!("the files are not a list: {e}")),
        },
        _ => Vec::new(),
    };
    let guard = SESSION.lock().unwrap_or_else(|p| p.into_inner());
    let Some(s) = guard.as_ref() else {
        return fail("no session");
    };
    let mut report = |stage: i32, done: u64, total: u64| {
        if let Some(f) = progress {
            // SAFETY: the extension's callback, with the context it gave.
            unsafe { f(ctx, stage, done, total) };
        }
    };
    match s.rt.block_on(send(&s.messenger, chat_id, c_str(text), &paths, &mut report)) {
        Ok(()) => OK,
        Err(e) => fail(e),
    }
}

#[no_mangle]
pub extern "C" fn veydan_share_error() -> *mut c_char {
    let e = LAST_ERROR.lock().unwrap_or_else(|p| p.into_inner()).clone();
    e.and_then(|e| CString::new(e).ok()).map(CString::into_raw).unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "C" fn veydan_share_close() {
    let taken = SESSION.lock().unwrap_or_else(|p| p.into_inner()).take();
    if let Some(s) = taken {
        s.rt.block_on(s.messenger.shutdown());
        // The runtime's threads end with it; a hanging task is given a moment.
        s.rt.shutdown_timeout(Duration::from_secs(3));
    }
}

/// # Safety
/// `s` came from this library and is freed once.
#[no_mangle]
pub unsafe extern "C" fn veydan_share_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(id: &str, kind: &str, over: impl FnOnce(&mut messenger_dm_view::ChatView)) -> messenger_dm_view::ChatView {
        let mut c: messenger_dm_view::ChatView = serde_json::from_value(serde_json::json!({
            "id": id, "kind": kind, "peer_pubkey": null, "peer_npub": null, "title": id, "picture": null, "is_contact": true, "is_muted": false, "unread": 0,
            "last_message_at": null, "last_preview": null, "pinned": false, "archived": false, "mode": if kind == "group" { "group" } else { "full_chat" },
            "can_send": true, "bot": false, "banned": false,
        }))
        .expect("a chat view from its JSON");
        over(&mut c);
        c
    }

    #[test]
    fn the_chats_of_a_forward_are_offered_the_newest_first() {
        let rows = targets(vec![
            chat("dm:a", "dm", |c| c.last_message_at = Some(10)),
            chat("dm:b", "dm", |c| c.last_message_at = Some(20)),
            chat("group:g", "group", |c| c.last_message_at = None),
            chat("dm:x", "dm", |c| c.archived = true),
            chat("dm:y", "dm", |c| c.can_send = false),
            chat("dm:z", "dm", |c| c.mode = "request".into()),
            chat("channel:c", "channel", |_| {}),
        ]);
        assert_eq!(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["dm:b", "dm:a", "group:g"]);
        assert_eq!(rows[2].kind, "group");
    }

    #[test]
    fn a_batch_id_has_the_shape_of_a_uuid() {
        let b = uuid_like();
        assert_eq!(b.len(), 36);
        assert_eq!(b.split('-').map(str::len).collect::<Vec<_>>(), [8, 4, 4, 4, 12]);
    }

    #[test]
    fn a_missing_data_file_is_refused_before_anything_opens() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime().unwrap();
        let err = rt.block_on(open(dir.path(), None)).err().unwrap();
        assert_eq!(err.0, ERROR);
        assert!(err.1.contains("no data file"));
    }
}
