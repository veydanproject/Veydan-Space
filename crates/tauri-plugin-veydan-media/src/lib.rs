// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Bridge to the phone's gallery for the sheet of the paperclip (ROO-110,
//! stage 1b; internal/platform-spec.md 13.4).
//!
//! The Kotlin side reads MediaStore (pages of the newest photos and videos
//! by a cursor, the albums), writes thumbnails as files into the app's
//! cache, starts the system camera and the system's Photo Picker, and
//! writes a HEIC photo anew as a JPEG with the phone's own decoder. It
//! reads rows, never whole files, and asks the user for nothing before a
//! tap. It also hands a text to another app (the system's list of apps,
//! SMS, a letter) for «Поделиться» (ROO-173), and puts a picture of the
//! app (a QR code of «Сохранить», ROO-297) into the gallery. The app's
//! Rust code calls it; nothing is open to the page.
//!
//! The plugin knows nothing about the messenger; it carries strings.

#![cfg(target_os = "android")]

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{
    plugin::{Builder, PluginHandle, TauriPlugin},
    Manager, Runtime,
};

const PLUGIN_IDENTIFIER: &str = "net.veydan.media";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("media bridge: {0}")]
    Invoke(#[from] tauri::plugin::mobile::PluginInvokeError),
}

pub type Result<T> = std::result::Result<T, Error>;

/// What the grid may show: `full`, `partial` (what the user chose,
/// Android 14+), `denied`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Access {
    pub access: String,
}

/// One photo or video of the grid.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    /// `image` or `video`.
    pub kind: String,
    /// The `content://` address the import reads.
    pub path: String,
    pub width: u32,
    pub height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// A page of the grid; `cursor` is where the next one starts, none after the last.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Page {
    pub items: Vec<Item>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Album {
    pub id: String,
    pub title: String,
    pub count: u32,
}

#[derive(Deserialize)]
struct Albums {
    albums: Vec<Album>,
}

#[derive(Deserialize)]
struct PathAnswer {
    path: Option<String>,
}

/// What `thumb` answers: the file, or what was kept of an item the app
/// may no longer read (deleted already).
#[derive(Deserialize)]
struct ThumbAnswer {
    path: Option<String>,
    #[serde(default)]
    gone: Vec<String>,
}

/// The thumbnail of an item of the grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Thumb {
    /// Its file in `cacheDir/attach-thumbs/`.
    File(PathBuf),
    /// The app may no longer read the item (a partial access that changed):
    /// its thumbnails kept on the disk were deleted, these were their files.
    Gone(Vec<PathBuf>),
}

/// What `readable` answers: the addresses still readable, and the files of
/// the thumbnails deleted of the others.
#[derive(Deserialize)]
struct ReadableAnswer {
    #[serde(default)]
    readable: Vec<String>,
    #[serde(default)]
    gone: Vec<String>,
}

#[derive(Deserialize)]
struct PathsAnswer {
    paths: Vec<String>,
}

/// Access to the gallery bridge.
pub struct VeydanMedia<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> VeydanMedia<R> {
    async fn call<T: DeserializeOwned>(&self, command: &str, payload: impl Serialize) -> Result<T> {
        Ok(self.0.run_mobile_plugin_async(command, payload).await?)
    }

    /// The access now; asks nothing.
    pub async fn access(&self) -> Result<String> {
        Ok(self.call::<Access>("access", ()).await?.access)
    }

    /// The system's question (or the app's settings, when the user refused
    /// for good); the access after it.
    pub async fn request(&self) -> Result<String> {
        Ok(self.call::<Access>("request", ()).await?.access)
    }

    /// The newest photos and videos after `cursor`; `kind` `image`/`video`
    /// or both, `album` a bucket of [`VeydanMedia::albums`].
    pub async fn recent(&self, cursor: Option<&str>, limit: u32, kind: Option<&str>, album: Option<&str>) -> Result<Page> {
        let args = serde_json::json!({ "cursor": cursor, "limit": limit, "kind": kind, "album": album });
        self.call("recent", args).await
    }

    pub async fn albums(&self) -> Result<Vec<Album>> {
        Ok(self.call::<Albums>("albums", ()).await?.albums)
    }

    /// The file of the thumbnail of an item, `side` pixels at most, in
    /// `cacheDir/attach-thumbs/`; asked of MediaStore anew each time, so an
    /// item out of the access now gives `Thumb::Gone`, never a file kept
    /// from before.
    pub async fn thumb(&self, uri: &str, side: u32) -> Result<Thumb> {
        let a: ThumbAnswer = self.call("thumb", serde_json::json!({ "uri": uri, "side": side })).await?;
        Ok(match a.path {
            Some(path) => Thumb::File(PathBuf::from(path)),
            None => Thumb::Gone(a.gone.into_iter().map(PathBuf::from).collect()),
        })
    }

    /// Which of `uris` (items of the grid) the app may still read, each
    /// asked of MediaStore by its own row whatever the grid shows; and the
    /// files of the thumbnails deleted of the others.
    pub async fn readable(&self, uris: &[String]) -> Result<(Vec<String>, Vec<PathBuf>)> {
        let a: ReadableAnswer = self.call("readable", serde_json::json!({ "uris": uris })).await?;
        Ok((a.readable, a.gone.into_iter().map(PathBuf::from).collect()))
    }

    /// The system camera (`photo` or `video`): the `content://` address of
    /// the shot, `None` when it was given up. Refused with `camera_denied`
    /// or `no_camera_app`.
    pub async fn capture(&self, kind: &str) -> Result<Option<String>> {
        Ok(self.call::<PathAnswer>("capture", serde_json::json!({ "kind": kind })).await?.path)
    }

    /// The shot at `uri` was imported: its file in the cache goes. Anything
    /// that is not a shot of `attach-capture` is left alone.
    pub async fn forget_capture(&self, uri: &str) -> Result<()> {
        self.call::<serde_json::Value>("forgetCapture", serde_json::json!({ "uri": uri })).await.map(|_| ())
    }

    /// The system's Photo Picker; the addresses picked, none when given up.
    pub async fn pick_system(&self, multiple: bool, kind: &str) -> Result<Vec<String>> {
        Ok(self.call::<PathsAnswer>("pickSystem", serde_json::json!({ "multiple": multiple, "kind": kind })).await?.paths)
    }

    /// The photo at `src` written as a JPEG of `quality` at `dst`: upright,
    /// at most `max_pixels`, its pixels alone.
    pub async fn convert(&self, src: &Path, dst: &Path, max_pixels: u64, quality: u8) -> Result<()> {
        let args = serde_json::json!({
            "src": src.to_string_lossy(),
            "dst": dst.to_string_lossy(),
            "maxPixels": max_pixels,
            "quality": quality,
        });
        self.call::<serde_json::Value>("convert", args).await.map(|_| ())
    }

    /// `text` into another app: `send` the system's list of apps, `sms`
    /// the SMS app without an addressee, `mail` a letter with `subject`.
    /// Refused with `no_app` when the phone has none for it.
    pub async fn share_text(&self, via: &str, text: &str, subject: Option<&str>) -> Result<()> {
        let args = serde_json::json!({ "via": via, "text": text, "subject": subject });
        self.call::<serde_json::Value>("shareText", args).await.map(|_| ())
    }

    /// A PNG (`base64`) into the gallery, album Pictures/Veydan, as `name`.
    /// Android 9 and older ask to write the storage first: refused with
    /// `storage_denied` when the user says no.
    pub async fn save_picture(&self, base64: &str, name: &str) -> Result<()> {
        let args = serde_json::json!({ "data": base64, "name": name });
        self.call::<serde_json::Value>("savePicture", args).await.map(|_| ())
    }
}

/// `app.veydan_media()`.
pub trait VeydanMediaExt<R: Runtime> {
    fn veydan_media(&self) -> &VeydanMedia<R>;
}

impl<R: Runtime, T: Manager<R>> VeydanMediaExt<R> for T {
    fn veydan_media(&self) -> &VeydanMedia<R> {
        self.state::<VeydanMedia<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("veydan-media")
        .setup(|app, api| {
            let handle = api.register_android_plugin(PLUGIN_IDENTIFIER, "VeydanMediaPlugin")?;
            app.manage(VeydanMedia(handle));
            Ok(())
        })
        .build()
}
