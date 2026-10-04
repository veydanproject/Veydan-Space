// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! File-backed source and path sink. Platform adapters (Android content URI)
//! reuse [`FileSource`] with their own opener.

use super::{LargeFileSink, LargeFileSource};
use crate::{Result, SyncError};
use async_trait::async_trait;
use std::io::{Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};

/// Opens a fresh read handle each time; must be repeatable for resume.
pub type Opener = Arc<dyn Fn() -> std::io::Result<std::fs::File> + Send + Sync>;

/// Any source that yields a `std::fs::File` (path, Android fd, temp file).
pub struct FileSource {
    open: Opener,
    len: Option<u64>,
}

impl FileSource {
    pub fn new(open: Opener, len: Option<u64>) -> Self {
        Self { open, len }
    }
}

#[async_trait]
impl LargeFileSource for FileSource {
    fn len_hint(&self) -> Option<u64> {
        self.len
    }

    async fn open_at(&self, offset: u64) -> Result<Box<dyn AsyncRead + Send + Unpin>> {
        let open = self.open.clone();
        let mut file = tokio::task::spawn_blocking(move || (open)())
            .await
            .map_err(|e| SyncError::Source(e.to_string()))?
            .map_err(|e| SyncError::Source(e.to_string()))?;
        if offset == 0 {
            return Ok(Box::new(tokio::fs::File::from_std(file)));
        }
        // Pipes (some content providers) cannot seek: skip instead.
        if file.seek(SeekFrom::Start(offset)).is_ok() {
            return Ok(Box::new(tokio::fs::File::from_std(file)));
        }
        let mut reader = tokio::fs::File::from_std(file);
        let skipped = tokio::io::copy(&mut (&mut reader).take(offset), &mut tokio::io::sink())
            .await
            .map_err(|e| SyncError::Source(e.to_string()))?;
        if skipped != offset {
            return Err(SyncError::Source(format!(
                "source shorter than offset {offset}"
            )));
        }
        Ok(Box::new(reader))
    }
}

/// Plain filesystem path.
pub struct PathSource(FileSource);

impl PathSource {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self> {
        let path: PathBuf = path.into();
        let len = std::fs::metadata(&path)
            .map_err(|e| SyncError::Source(format!("{}: {e}", path.display())))?
            .len();
        let opener: Opener = Arc::new(move || std::fs::File::open(&path));
        Ok(Self(FileSource::new(opener, Some(len))))
    }
}

#[async_trait]
impl LargeFileSource for PathSource {
    fn len_hint(&self) -> Option<u64> {
        self.0.len_hint()
    }

    async fn open_at(&self, offset: u64) -> Result<Box<dyn AsyncRead + Send + Unpin>> {
        self.0.open_at(offset).await
    }
}

pub const PART_SUFFIX: &str = ".veydanpart";
const SIDECAR_SUFFIX: &str = ".veydanpart.json";
/// Free space kept after a download completes.
const DISK_RESERVE: u64 = 64 * 1024 * 1024;

#[derive(serde::Serialize, serde::Deserialize)]
struct Sidecar {
    manifest_id: String,
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Destination path with `.veydanpart` staging and a sidecar naming the manifest.
pub struct PathSink {
    dest: PathBuf,
    part: PathBuf,
    sidecar: PathBuf,
}

impl PathSink {
    pub fn new(dest: impl Into<PathBuf>) -> Self {
        let dest: PathBuf = dest.into();
        Self {
            part: with_suffix(&dest, PART_SUFFIX),
            sidecar: with_suffix(&dest, SIDECAR_SUFFIX),
            dest,
        }
    }

    pub fn is_staging_file(name: &str) -> bool {
        name.ends_with(PART_SUFFIX) || name.ends_with(SIDECAR_SUFFIX)
    }

    fn remove_quiet(path: &Path) {
        let _ = std::fs::remove_file(path);
    }

    fn disk(e: std::io::Error) -> SyncError {
        if e.kind() == std::io::ErrorKind::StorageFull {
            SyncError::Disk("no space left".into())
        } else {
            SyncError::Disk(e.to_string())
        }
    }
}

#[async_trait]
impl LargeFileSink for PathSink {
    async fn staged_len(&self, manifest_id: &str) -> Result<u64> {
        let same = std::fs::read(&self.sidecar)
            .ok()
            .and_then(|b| serde_json::from_slice::<Sidecar>(&b).ok())
            .is_some_and(|s| s.manifest_id == manifest_id);
        if !same {
            Self::remove_quiet(&self.part);
            Self::remove_quiet(&self.sidecar);
            return Ok(0);
        }
        Ok(std::fs::metadata(&self.part).map(|m| m.len()).unwrap_or(0))
    }

    async fn open_staging(
        &self,
        manifest_id: &str,
        offset: u64,
        total: u64,
    ) -> Result<Box<dyn AsyncWrite + Send + Unpin>> {
        if let Some(parent) = self.dest.parent() {
            std::fs::create_dir_all(parent).map_err(Self::disk)?;
        }
        if let Some(free) = available_space(self.dest.parent().unwrap_or(Path::new("."))) {
            let need = total.saturating_sub(offset).saturating_add(DISK_RESERVE);
            if free < need {
                return Err(SyncError::Disk(format!("need {need} bytes, {free} free")));
            }
        }
        let sidecar = serde_json::to_vec(&Sidecar {
            manifest_id: manifest_id.to_string(),
        })?;
        std::fs::write(&self.sidecar, sidecar).map_err(Self::disk)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&self.part)
            .map_err(Self::disk)?;
        file.set_len(offset).map_err(Self::disk)?;
        let mut file = file;
        file.seek(SeekFrom::Start(offset)).map_err(Self::disk)?;
        Ok(Box::new(tokio::fs::File::from_std(file)))
    }

    async fn commit(&self) -> Result<()> {
        let file = std::fs::File::open(&self.part).map_err(Self::disk)?;
        file.sync_all().map_err(Self::disk)?;
        drop(file);
        // Windows rename does not replace an existing destination.
        if self.dest.exists() {
            std::fs::remove_file(&self.dest).map_err(Self::disk)?;
        }
        std::fs::rename(&self.part, &self.dest).map_err(Self::disk)?;
        Self::remove_quiet(&self.sidecar);
        Ok(())
    }

    async fn discard(&self) -> Result<()> {
        Self::remove_quiet(&self.part);
        Self::remove_quiet(&self.sidecar);
        Ok(())
    }
}

/// Free bytes on the volume holding `dir`; `None` where the OS query is unavailable.
#[cfg(unix)]
pub fn available_space(dir: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(dir.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    Some(st.f_bavail as u64 * st.f_frsize as u64)
}

#[cfg(windows)]
pub fn available_space(dir: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = dir
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut free_to_caller: u64 = 0;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_to_caller,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(free_to_caller)
}

#[cfg(not(any(unix, windows)))]
pub fn available_space(_dir: &Path) -> Option<u64> {
    None
}
