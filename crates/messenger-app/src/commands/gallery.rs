// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The phone's gallery in the sheet of the paperclip (ROO-110 1b): what
//! stands between the page and the gallery plugin
//! (`tauri-plugin-veydan-media`). The page never talks to the plugin; it
//! calls the commands here, as it does for pushes (push.rs), so no
//! capability names the plugin. A thumbnail goes to the page as an address
//! of the app's server of files, from the pool of thumbnails
//! (`MediaServer::thumb_url`), never as base64.
//!
//! Also the platform's decoder for the common formats of pictures
//! (`messenger_media::common`): on a phone a HEIC picked as a photo is
//! written as a JPEG by the plugin; a computer has none.
//!
//! On a computer the commands exist and say the gallery is not there: the
//! sheet of a computer is the system's file dialog.

use serde::Serialize;
#[cfg(target_os = "android")]
use std::sync::Arc;
use veydan_core::{AppError, CmdResult};

#[cfg(target_os = "android")]
use crate::media_server::MediaServer;
#[cfg(target_os = "android")]
use tauri::Manager;
#[cfg(target_os = "android")]
use tauri_plugin_veydan_media::VeydanMediaExt;

/// What a command answers where there is no gallery (a computer).
#[cfg_attr(target_os = "android", allow(dead_code))]
const NO_GALLERY: &str = "the gallery of the phone is not here";

/// A photo or a video of the grid, as the page draws it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryItem {
    pub id: String,
    /// `image` or `video`.
    pub kind: String,
    /// The `content://` address `importPicked` takes.
    pub path: String,
    pub width: u32,
    pub height: u32,
    /// Seconds, for a video.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GalleryPage {
    pub items: Vec<GalleryItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GalleryAlbum {
    pub id: String,
    pub title: String,
    pub count: u32,
}

#[cfg(target_os = "android")]
fn bridge(e: tauri_plugin_veydan_media::Error) -> AppError {
    AppError::Other(e.to_string())
}

/// `full`, `partial` or `denied`; asks nothing.
#[tauri::command]
pub async fn messenger_gallery_access(app: tauri::AppHandle) -> CmdResult<String> {
    #[cfg(target_os = "android")]
    return app.veydan_media().access().await.map_err(bridge);
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok("denied".into())
    }
}

/// The user tapped «Разрешить доступ»: the system's question, or the app's
/// settings when it was refused for good. The access after it.
#[tauri::command]
pub async fn messenger_gallery_request(app: tauri::AppHandle) -> CmdResult<String> {
    #[cfg(target_os = "android")]
    return app.veydan_media().request().await.map_err(bridge);
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok("denied".into())
    }
}

/// A page of the newest photos and videos after `cursor`.
#[tauri::command]
pub async fn messenger_gallery_recent(
    cursor: Option<String>,
    limit: u32,
    kind: Option<String>,
    album: Option<String>,
    app: tauri::AppHandle,
) -> CmdResult<GalleryPage> {
    #[cfg(target_os = "android")]
    {
        let kind = kind.filter(|k| k == "image" || k == "video");
        let page = app
            .veydan_media()
            .recent(cursor.as_deref(), limit, kind.as_deref(), album.as_deref())
            .await
            .map_err(bridge)?;
        Ok(GalleryPage {
            items: page
                .items
                .into_iter()
                .map(|i| GalleryItem {
                    duration: i.duration_ms.map(|ms| ms as f64 / 1000.0),
                    id: i.id,
                    kind: i.kind,
                    path: i.path,
                    width: i.width,
                    height: i.height,
                })
                .collect(),
            cursor: page.cursor,
        })
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (cursor, limit, kind, album, app);
        Ok(GalleryPage { items: vec![], cursor: None })
    }
}

#[tauri::command]
pub async fn messenger_gallery_albums(app: tauri::AppHandle) -> CmdResult<Vec<GalleryAlbum>> {
    #[cfg(target_os = "android")]
    {
        let albums = app.veydan_media().albums().await.map_err(bridge)?;
        Ok(albums.into_iter().map(|a| GalleryAlbum { id: a.id, title: a.title, count: a.count }).collect())
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok(vec![])
    }
}

/// The address of the thumbnail of an item of the grid, `side` pixels at
/// most: a JPEG of the plugin's in the app's cache, named from the pool of
/// thumbnails of the server of files. An item the app may no longer read
/// (a partial access that changed) is refused with `not_accessible`: the
/// plugin deleted what it kept of it, and its addresses answer 404.
#[tauri::command]
pub async fn messenger_gallery_thumb(uri: String, side: u32, app: tauri::AppHandle) -> CmdResult<String> {
    #[cfg(target_os = "android")]
    {
        let server = app.state::<MediaServer>();
        match app.veydan_media().thumb(&uri, side).await.map_err(bridge)? {
            tauri_plugin_veydan_media::Thumb::File(path) => {
                server.thumb_url(&path).await.ok_or_else(|| AppError::Other("the server of files does not listen".into()))
            }
            tauri_plugin_veydan_media::Thumb::Gone(paths) => {
                server.forget_thumb(&paths);
                Err(AppError::Other(NOT_ACCESSIBLE.into()))
            }
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (uri, side, app);
        Err(AppError::Other(NO_GALLERY.into()))
    }
}

/// What `messenger_gallery_thumb` refuses an item out of the access with.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
const NOT_ACCESSIBLE: &str = "not_accessible";

/// The page reads the gallery anew (it opened, or the app came back to the
/// screen: the access may have changed meanwhile): every address of a
/// thumbnail handed out before answers 404; the page asks those it shows
/// again, and each is asked of MediaStore with the access of now.
#[tauri::command]
pub async fn messenger_gallery_forget_thumbs(app: tauri::AppHandle) -> CmdResult<()> {
    #[cfg(target_os = "android")]
    app.state::<MediaServer>().forget_thumbs();
    #[cfg(not(target_os = "android"))]
    let _ = app;
    Ok(())
}

/// Which of the items chosen in the grid (`content://media/` addresses) the
/// app may still read now, in their order: each asked of MediaStore by its
/// own row, whatever album or page the grid shows, so an item merely out of
/// the album in view stays chosen and one a changed partial access no longer
/// lets in leaves. What was kept of the others is deleted, its addresses 404.
#[tauri::command]
pub async fn messenger_gallery_readable(uris: Vec<String>, app: tauri::AppHandle) -> CmdResult<Vec<String>> {
    #[cfg(target_os = "android")]
    {
        let (readable, gone) = app.veydan_media().readable(&uris).await.map_err(bridge)?;
        app.state::<MediaServer>().forget_thumb(&gone);
        Ok(readable)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (uris, app);
        Err(AppError::Other(NO_GALLERY.into()))
    }
}

/// The system camera, `photo` or `video`: the address of the shot for
/// `importPicked` (which deletes the shot once it is copied), `null` when
/// it was given up. `camera_denied`: the user refused the camera.
#[tauri::command]
pub async fn messenger_gallery_capture(kind: String, app: tauri::AppHandle) -> CmdResult<Option<String>> {
    #[cfg(target_os = "android")]
    {
        let kind = if kind == "video" { "video" } else { "photo" };
        app.veydan_media().capture(kind).await.map_err(bridge)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (kind, app);
        Err(AppError::Other(NO_GALLERY.into()))
    }
}

/// The system's Photo Picker: what was picked (`content://`), none when given up.
#[tauri::command]
pub async fn messenger_gallery_pick(multiple: bool, kind: String, app: tauri::AppHandle) -> CmdResult<Vec<String>> {
    #[cfg(target_os = "android")]
    {
        let kind = match kind.as_str() {
            "image" | "video" => kind.as_str(),
            _ => "media",
        };
        app.veydan_media().pick_system(multiple, kind).await.map_err(bridge)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (multiple, kind, app);
        Err(AppError::Other(NO_GALLERY.into()))
    }
}

/// The shot of the camera behind `picked`, once it is imported: its file
/// in the cache goes (the plugin deletes only a shot of `attach-capture`).
pub(crate) async fn forget_capture(app: &tauri::AppHandle, picked: &str) {
    #[cfg(target_os = "android")]
    if picked.starts_with("content://") && picked.contains(".fileprovider/") {
        let _ = app.veydan_media().forget_capture(picked).await;
    }
    #[cfg(not(target_os = "android"))]
    let _ = (app, picked);
}

/// The decoder of the platform for the common formats of pictures, kept
/// by the app for the chats and the channels alike
/// (`messenger_media::common::PlatformDecoder`); none on a computer (a
/// HEIC goes there as a file, as before).
pub(crate) fn offer_decoder(app: &tauri::AppHandle) {
    #[cfg(target_os = "android")]
    {
        let decoder: Arc<dyn messenger_media::common::PictureConverter> = Arc::new(PhoneDecoder(app.clone()));
        app.manage(messenger_media::common::PlatformDecoder(decoder));
    }
    #[cfg(not(target_os = "android"))]
    let _ = app;
}

/// `ImageDecoder` of the phone, through the plugin.
#[cfg(target_os = "android")]
struct PhoneDecoder(tauri::AppHandle);

#[cfg(target_os = "android")]
#[async_trait::async_trait]
impl messenger_media::common::PictureConverter for PhoneDecoder {
    async fn to_jpeg(&self, src: &std::path::Path, dst: &std::path::Path, max_pixels: u64, quality: u8) -> Result<(), String> {
        self.0.veydan_media().convert(src, dst, max_pixels, quality).await.map_err(|e| e.to_string())
    }
}
