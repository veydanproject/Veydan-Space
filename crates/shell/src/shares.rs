// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What another app shares into Veydan through the system's share sheet: a
//! text, files, or both (internal/platform-spec.md 13.4, «Поделиться в
//! Veydan»). The phone's side is `ShareActivity` of the shell's Android
//! library: it copies the files into `share-inbox/<id>/` of the app's cache
//! while the system still lets it read them, keeps the share in memory
//! (`IncomingShares`, as `IncomingLinks` keeps a link) and starts the app.
//! Here the share is taken from there (`take`) when the page asks, after the
//! app is unlocked; the module that shows it (the messenger's chooser of a
//! chat, the notes' new note) reads the copies by their paths, which
//! `handed_over` vouches for, and deletes the folder when it is done
//! (`forget`). An iPhone sends from its share extension itself and gives the
//! app nothing; a computer has no share sheet that feeds apps: there `take`
//! answers nothing.
//!
//! Nothing of a share is logged or written outside the inbox, and the inbox
//! is swept of what no one took (a day later, by the activity).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[cfg(target_os = "android")]
use tauri::Emitter;
use tauri::{AppHandle, Manager};

/// Emitted with nothing when a share came: the page asks for it.
pub const EVENT_SHARE: &str = "shares://incoming";

/// The folder of the app's cache the copies wait in (`ShareRules.INBOX`).
const INBOX: &str = "share-inbox";

/// One shared file, a copy in the inbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedFile {
    /// The copy's path, inside the inbox.
    pub path: String,
    /// The name its source gave it, cleaned (`ShareRules.fileName`).
    pub name: String,
    /// The kind, when the source said it.
    #[serde(default)]
    pub mime: Option<String>,
    pub size: u64,
}

/// What another app shared, as `IncomingShares` holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Share {
    /// The inbox folder of its files (a hex stamp): `forget` takes it.
    pub id: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    /// The chat chosen in the system's sheet itself (Direct Share: the id
    /// of a conversation's shortcut), when one was.
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub files: Vec<SharedFile>,
    /// Names of files that could not be copied: too large, gone before read.
    #[serde(default)]
    pub refused: Vec<String>,
}

impl Share {
    #[cfg(any(target_os = "android", test))]
    fn is_empty(&self) -> bool {
        self.text.is_none() && self.files.is_empty() && self.refused.is_empty()
    }
}

/// Puts the state in place; called once the plugins are set up.
pub(crate) fn setup(app: &AppHandle) {
    #[cfg(target_os = "android")]
    let _ = RUNNING.set(app.clone());
    let _ = app;
}

/// The app came back to the front (`WindowEvent::Resumed`): a share may wait
/// with `ShareActivity`. The page is told, without it; `take` gets it.
#[cfg(target_os = "android")]
pub(crate) fn resumed(app: &AppHandle) {
    let _ = app.emit(EVENT_SHARE, ());
}

/// The app that takes shares, for `ShareActivity` to tell: set once it can.
#[cfg(target_os = "android")]
static RUNNING: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();

/// `IncomingShares.arrived()` of the shell's Android library: `ShareActivity`
/// put a share aside while the app runs. The page is told, without it.
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_net_veydan_shell_IncomingShares_arrived<'l>(_env: jni::JNIEnv<'l>, _class: jni::objects::JClass<'l>) {
    if let Some(app) = RUNNING.get() {
        resumed(app);
    }
}

/// The inbox of this app: `share-inbox/` of its cache.
fn inbox_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_cache_dir().ok().map(|c| c.join(INBOX))
}

/// Whether `path` names a copy inside the inbox, as it stands now: a module
/// reads such a file as one the user handed over (as a `content://` of the
/// gallery), never any other path a page names.
pub fn handed_over(app: &AppHandle, path: &str) -> bool {
    inbox_dir(app).is_some_and(|inbox| inside(&inbox, Path::new(path)))
}

/// Whether `path` is a file inside `inbox`, both resolved: a link or a `..` that leads out does not count.
fn inside(inbox: &Path, path: &Path) -> bool {
    let (Ok(inbox), Ok(real)) = (std::fs::canonicalize(inbox), std::fs::canonicalize(path)) else {
        return false;
    };
    real != inbox && real.starts_with(&inbox) && real.is_file()
}

/// The share `ShareActivity` keeps, if one waits; none on any other platform.
pub fn take(app: &AppHandle) -> Option<Share> {
    #[cfg(target_os = "android")]
    {
        let inbox = inbox_dir(app)?;
        return waiting_android().and_then(|json| parse(&json, &inbox));
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        None
    }
}

/// The folder of share `id` deleted, with its copies: the module that showed
/// the share is done with it (sent, or given up). An id that is no stamp of
/// the inbox names nothing and deletes nothing.
pub fn forget(app: &AppHandle, id: &str) {
    let Some(inbox) = inbox_dir(app) else {
        return;
    };
    if let Some(dir) = folder(&inbox, id) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// `inbox/<id>` for an id of `ShareRules.folderName` (hex, 1–16 chars); none for anything else.
fn folder(inbox: &Path, id: &str) -> Option<PathBuf> {
    let ok = !id.is_empty() && id.len() <= 16 && id.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
    ok.then(|| inbox.join(id))
}

/// The share as `Inbox.take` of the activity wrote it, checked: its id names
/// a folder of the inbox, every file is a copy inside that very folder (a
/// path elsewhere is dropped, with its name said as refused). None when
/// nothing is left of it.
#[cfg(any(target_os = "android", test))]
fn parse(json: &str, inbox: &Path) -> Option<Share> {
    let mut share: Share = serde_json::from_str(json).ok()?;
    let dir = folder(inbox, &share.id)?;
    let (kept, dropped): (Vec<_>, Vec<_>) = std::mem::take(&mut share.files)
        .into_iter()
        .partition(|f| inside(&dir, Path::new(&f.path)));
    share.files = kept;
    share.refused.extend(dropped.into_iter().map(|f| f.name));
    if share.text.as_deref().is_some_and(|t| t.trim().is_empty()) {
        share.text = None;
    }
    if share.is_empty() {
        return None;
    }
    Some(share)
}

/// `IncomingShares.take()` of the shell's Android library, through the class
/// loader of the activity: the class is the app's, not the system's.
#[cfg(target_os = "android")]
fn waiting_android() -> Option<String> {
    use jni::objects::{JClass, JString, JValue};
    let (tx, rx) = std::sync::mpsc::channel();
    tauri::wry::prelude::dispatch(move |env, activity, _webview| {
        let mut ask = || -> jni::errors::Result<Option<String>> {
            let loader = env
                .call_method(activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
                .l()?;
            let name = env.new_string("net.veydan.shell.IncomingShares")?;
            let class = env
                .call_method(
                    &loader,
                    "loadClass",
                    "(Ljava/lang/String;)Ljava/lang/Class;",
                    &[JValue::Object(&name)],
                )?
                .l()?;
            let share = env
                .call_static_method(JClass::from(class), "take", "()Ljava/lang/String;", &[])?
                .l()?;
            if share.is_null() {
                return Ok(None);
            }
            let share = JString::from(share);
            let text: String = env.get_string(&share)?.into();
            Ok(Some(text))
        };
        let answer = ask().ok().flatten();
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        let _ = tx.send(answer);
    });
    rx.recv_timeout(std::time::Duration::from_secs(5)).ok().flatten()
}

/// The share that waits, taken by the page: none when nothing came (or on a
/// computer). Asked once the app is unlocked, by the core's inbox of shares.
#[tauri::command]
pub async fn shares_take(app: AppHandle) -> Option<Share> {
    tauri::async_runtime::spawn_blocking(move || take(&app)).await.ok().flatten()
}

/// The share is sent or given up: its copies go.
#[tauri::command]
pub async fn shares_forget(id: String, app: AppHandle) {
    let _ = tauri::async_runtime::spawn_blocking(move || forget(&app, &id)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inbox() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn file(dir: &Path, name: &str) -> String {
        std::fs::create_dir_all(dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, b"x").unwrap();
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn a_share_of_files_and_text_is_read_with_its_copies_checked() {
        let inbox = inbox();
        let dir = inbox.path().join("18c1a2b3c4d");
        let photo = file(&dir, "IMG_1.jpg");
        let json = serde_json::json!({
            "id": "18c1a2b3c4d", "text": "see this", "subject": null, "target": "dm:ab",
            "files": [{ "path": photo, "name": "IMG_1.jpg", "mime": "image/jpeg", "size": 1 }],
            "refused": ["huge.mp4"],
        });
        let share = parse(&json.to_string(), inbox.path()).unwrap();
        assert_eq!(share.id, "18c1a2b3c4d");
        assert_eq!(share.text.as_deref(), Some("see this"));
        assert_eq!(share.target.as_deref(), Some("dm:ab"));
        assert_eq!(share.files.len(), 1);
        assert_eq!(share.files[0].name, "IMG_1.jpg");
        assert_eq!(share.refused, vec!["huge.mp4".to_string()]);
    }

    #[test]
    fn a_file_outside_the_folder_of_the_share_is_dropped_and_said() {
        let inbox = inbox();
        let own = file(&inbox.path().join("1a"), "ok.txt");
        let other = file(&inbox.path().join("2b"), "other.txt");
        let outside = file(&inbox.path().parent().unwrap().join("elsewhere-shares-test"), "secret.txt");
        let missing = inbox.path().join("1a").join("gone.txt").to_string_lossy().into_owned();
        let json = serde_json::json!({
            "id": "1a", "files": [
                { "path": own, "name": "ok.txt", "size": 1 },
                { "path": other, "name": "other.txt", "size": 1 },
                { "path": outside, "name": "secret.txt", "size": 1 },
                { "path": missing, "name": "gone.txt", "size": 1 },
            ],
        });
        let share = parse(&json.to_string(), inbox.path()).unwrap();
        assert_eq!(share.files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["ok.txt"]);
        assert_eq!(share.refused, ["other.txt", "secret.txt", "gone.txt"]);
        let _ = std::fs::remove_dir_all(inbox.path().parent().unwrap().join("elsewhere-shares-test"));
    }

    #[test]
    fn a_share_with_nothing_left_is_none() {
        let inbox = inbox();
        assert!(parse(r#"{"id":"1a","text":"  ","files":[]}"#, inbox.path()).is_none());
        assert!(parse("not json", inbox.path()).is_none());
        // An id that is no stamp of the inbox names no folder.
        assert!(parse(r#"{"id":"../x","text":"hi"}"#, inbox.path()).is_none());
        assert!(parse(r#"{"id":"","text":"hi"}"#, inbox.path()).is_none());
        // A text alone is a share.
        let share = parse(r#"{"id":"ff","text":"hi"}"#, inbox.path()).unwrap();
        assert_eq!(share.text.as_deref(), Some("hi"));
        assert!(share.files.is_empty());
    }

    #[test]
    fn only_a_folder_of_the_inbox_is_forgotten() {
        let inbox = inbox();
        assert_eq!(folder(inbox.path(), "18c1a2b3c4d"), Some(inbox.path().join("18c1a2b3c4d")));
        assert_eq!(folder(inbox.path(), ".."), None);
        assert_eq!(folder(inbox.path(), "a/b"), None);
        assert_eq!(folder(inbox.path(), "ABC"), None);
        assert_eq!(folder(inbox.path(), &"f".repeat(17)), None);
        assert_eq!(folder(inbox.path(), ""), None);
    }

    #[test]
    fn a_path_is_handed_over_only_as_a_file_inside_the_inbox() {
        let inbox = inbox();
        let own = file(&inbox.path().join("1a"), "ok.txt");
        assert!(inside(inbox.path(), Path::new(&own)));
        assert!(!inside(inbox.path(), inbox.path()));
        assert!(!inside(inbox.path(), &inbox.path().join("1a")), "a folder is not a file");
        assert!(!inside(inbox.path(), Path::new("/etc/passwd")));
        let traversal = inbox.path().join("1a").join("..").join("..").join("x");
        assert!(!inside(inbox.path(), &traversal));
    }
}
