// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A file the user picked, as something to read from: a local path, or an
//! Android `content://` URI that only the system can open.

use std::path::PathBuf;
use std::sync::Arc;
use veydan_core::AppError;

#[cfg(target_os = "android")]
mod content_uri;

/// Opens the file anew on each call, so a reader can start over.
pub type Opener = Arc<dyn Fn() -> std::io::Result<std::fs::File> + Send + Sync>;

/// Streaming input: nothing of the file is read until `open` is called.
pub struct FileSource {
    /// The name to show and to store the file under.
    pub name: String,
    /// The size, when the source tells it.
    pub len: Option<u64>,
    pub open: Opener,
}

pub(crate) fn open(app: &tauri::AppHandle, src: &str) -> Result<FileSource, AppError> {
    #[cfg(target_os = "android")]
    if src.starts_with("content://") {
        return content_uri::source(app, src);
    }
    #[cfg(not(target_os = "android"))]
    let _ = app;
    let path = PathBuf::from(src);
    let len = std::fs::metadata(&path).map_err(AppError::io)?.len();
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    Ok(FileSource {
        name,
        len: Some(len),
        open: Arc::new(move || std::fs::File::open(&path)),
    })
}
