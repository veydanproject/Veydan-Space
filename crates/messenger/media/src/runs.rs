// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Runs seen from another process with the same data folder (the CLI
//! beside the app).
//!
//! A run holds `<cache>/runs/<id>.lock` for as long as it lives, so any
//! process can tell whether a transfer runs somewhere now. A word for it
//! from a process that does not run it (pause, cancel, retry now) is left
//! in `runs/<id>.ask`, which the run reads within moments. The run shows
//! its last event in `runs/<id>.json` about once a second, for the views
//! of the others. A system without file locks knows none of this.

use crate::progress::Progress;
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A view older than this is of a run that is gone.
const VIEW_FRESH: Duration = Duration::from_secs(5);

/// A word for a run from another process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Word {
    Pause,
    Cancel,
    Retry,
}

impl Word {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Cancel => "cancel",
            Self::Retry => "retry",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "pause" => Some(Self::Pause),
            "cancel" => Some(Self::Cancel),
            "retry" => Some(Self::Retry),
            _ => None,
        }
    }
}

/// Another run holds the lock.
#[derive(Debug)]
pub struct Held;

/// The lock of one run. Dropped, its files go and the lock is given up.
pub struct RunLock {
    dir: PathBuf,
    id: String,
    _file: File,
}

impl Drop for RunLock {
    fn drop(&mut self) {
        // The files go while the lock still keeps others out. The lock file
        // goes only where one who opened it just before can tell it was
        // removed (`same_file`); elsewhere it stays, empty, until a start
        // alone in the data folder (`sweep`).
        let exts: &[&str] = if cfg!(unix) { &["json", "ask", "lock"] } else { &["json", "ask"] };
        for ext in exts {
            let _ = std::fs::remove_file(self.dir.join(format!("{}.{ext}", self.id)));
        }
    }
}

fn dir(cache: &Path) -> PathBuf {
    cache.join("runs")
}

/// Transfer ids name files: only ours (letters, digits, `-`, `_`) do.
fn valid(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn file_of(cache: &Path, id: &str, ext: &str) -> PathBuf {
    dir(cache).join(format!("{id}.{ext}"))
}

/// Is `file` the file `path` names now (not one removed meanwhile)?
#[cfg(unix)]
fn same_file(file: &File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (file.metadata(), std::fs::metadata(path)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// Elsewhere the lock file is never removed (`RunLock`): the path names
/// the file it named when it was opened.
#[cfg(not(unix))]
fn same_file(_: &File, _: &Path) -> bool {
    true
}

/// Take the lock of the run of `id`. `Ok(None)` when nothing can be
/// known (no file locks, or an id that is not ours); `Err(Held)` while
/// another run holds it. One that only looks (`held`) is waited out.
pub async fn take(cache: &Path, id: &str) -> Result<Option<RunLock>, Held> {
    if !valid(id) || std::fs::create_dir_all(dir(cache)).is_err() {
        return Ok(None);
    }
    let path = file_of(cache, id, "lock");
    for _ in 0..20 {
        let Ok(file) = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&path) else {
            return Ok(None);
        };
        match file.try_lock() {
            // The lock must be on the file the path names now: the run
            // before may have removed it after this one opened it. A word or
            // a view left for a run that has ended means nothing to this one.
            Ok(()) if same_file(&file, &path) => {
                for ext in ["ask", "ask.tmp", "json"] {
                    let _ = std::fs::remove_file(file_of(cache, id, ext));
                }
                return Ok(Some(RunLock { dir: dir(cache), id: id.to_string(), _file: file }));
            }
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => tokio::time::sleep(Duration::from_millis(10)).await,
            // No file locks here: nothing can hold this file, nor need it.
            Err(TryLockError::Error(_)) => {
                let _ = std::fs::remove_file(&path);
                return Ok(None);
            }
        }
    }
    Err(Held)
}

/// Remove what runs that are gone left: the files of every run no one
/// holds (a process killed, or a lock file that stays; see `RunLock`).
/// For a start with nobody else in the data folder.
pub fn sweep(cache: &Path) {
    let Ok(entries) = std::fs::read_dir(dir(cache)) else { return };
    for path in entries.flatten().map(|e| e.path()) {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        let id = name.split('.').next().unwrap_or_default();
        if !valid(id) {
            continue;
        }
        if name.ends_with(".lock") {
            // Removed while it is locked, as a run removes its own.
            if let Ok(file) = OpenOptions::new().read(true).write(true).open(&path) {
                if file.try_lock().is_ok() {
                    let _ = std::fs::remove_file(&path);
                }
            }
        } else if held(cache, id) == Some(false) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Does a run of `id` hold its lock now, in this process or another?
/// `None` when that cannot be told.
pub fn held(cache: &Path, id: &str) -> Option<bool> {
    if !valid(id) {
        return None;
    }
    let file = match File::open(file_of(cache, id, "lock")) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some(false),
        Err(_) => return None,
    };
    match file.try_lock_shared() {
        Ok(()) => {
            let _ = file.unlock();
            Some(false)
        }
        Err(TryLockError::WouldBlock) => Some(true),
        Err(TryLockError::Error(_)) => None,
    }
}

/// Write `bytes` to `path` whole: a reader never sees half of them.
fn put_whole(path: &Path, bytes: &[u8]) -> bool {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, path)).is_ok()
}

/// Leave `word` for the run of `id`. Whether a run holds the lock to
/// read it.
pub fn ask(cache: &Path, id: &str, word: Word) -> bool {
    held(cache, id) == Some(true) && put_whole(&file_of(cache, id, "ask"), word.as_str().as_bytes())
}

/// The word left for the run of `id`, once.
pub fn take_ask(cache: &Path, id: &str) -> Option<Word> {
    if !valid(id) {
        return None;
    }
    let path = file_of(cache, id, "ask");
    let word = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    Word::parse(&word)
}

/// Show the last event of the run of `id` to the others.
pub fn write_view(cache: &Path, id: &str, p: &Progress) {
    if valid(id) {
        if let Ok(json) = serde_json::to_vec(p) {
            put_whole(&file_of(cache, id, "json"), &json);
        }
    }
}

/// The last event of a run of `id` in another process, while it is
/// fresh.
pub fn read_view(cache: &Path, id: &str) -> Option<Progress> {
    if !valid(id) {
        return None;
    }
    let path = file_of(cache, id, "json");
    let age = std::fs::metadata(&path).ok()?.modified().ok()?.elapsed().unwrap_or_default();
    if age > VIEW_FRESH {
        return None;
    }
    serde_json::from_slice(&std::fs::read(&path).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn one_run_holds_a_transfer_and_hears_words_left_for_it() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path();
        assert_eq!(held(cache, "up-1"), Some(false), "nothing runs it");
        assert!(!ask(cache, "up-1", Word::Pause), "nobody to ask");

        let lock = take(cache, "up-1").await.unwrap().unwrap();
        assert_eq!(held(cache, "up-1"), Some(true));
        assert!(take(cache, "up-1").await.is_err(), "one run at a time");
        assert!(ask(cache, "up-1", Word::Cancel));
        assert_eq!(take_ask(cache, "up-1"), Some(Word::Cancel));
        assert_eq!(take_ask(cache, "up-1"), None, "a word is read once");

        let mut p = crate::progress::tests::base(10);
        p.chunks_done = 3;
        write_view(cache, "up-1", &p);
        assert_eq!(read_view(cache, "up-1"), Some(p.clone()));

        drop(lock);
        assert_eq!(held(cache, "up-1"), Some(false));
        assert_eq!(read_view(cache, "up-1"), None, "the view goes with the run");
        // A word that came as the run ended (it was asked while it held the
        // lock) is not for the next run.
        assert!(put_whole(&file_of(cache, "up-1", "ask"), b"cancel"));
        write_view(cache, "up-1", &p);
        let next = take(cache, "up-1").await.unwrap();
        assert!(next.is_some(), "the next run takes it");
        assert_eq!(take_ask(cache, "up-1"), None, "nothing left for it");
        assert_eq!(read_view(cache, "up-1"), None, "nor a view of the one before");
        drop(next);

        for bad in ["", "../x", "a/b", "a.b"] {
            assert!(take(cache, bad).await.unwrap().is_none());
            assert_eq!(held(cache, bad), None);
        }
    }

    /// What runs that are gone left goes at a start alone; what a run that
    /// lives holds stays.
    #[tokio::test]
    async fn what_runs_that_are_gone_left_is_swept() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path();
        let live = take(cache, "live").await.unwrap().unwrap();
        write_view(cache, "live", &crate::progress::tests::base(10));
        // A lock file that stays (no unix), a view and a word of a killed
        // run, and a view with no lock at all.
        for (id, ext) in [("gone", "lock"), ("gone", "json"), ("gone", "ask"), ("gone", "ask.tmp"), ("lost", "json")] {
            std::fs::write(file_of(cache, id, ext), b"").unwrap();
        }
        sweep(cache);
        let mut left: Vec<String> =
            std::fs::read_dir(dir(cache)).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["live.json", "live.lock"]);
        drop(live);
    }
}
