// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Android `content://` URIs as a streaming source: display name and
//! size come from the ContentResolver, bytes from a file descriptor the fs
//! plugin opens. Nothing is base64-encoded or held in memory.

use super::{FileSource, Opener};
use veydan_core::{AppError, CmdResult};
use jni::objects::{JObject, JString, JValue};
use jni::JNIEnv;
use std::sync::Arc;
use std::time::Duration;
use tauri_plugin_fs::{FilePath, FsExt, OpenOptions};

const DISPLAY_NAME: &str = "_display_name";
const SIZE: &str = "_size";

pub(super) fn source(app: &tauri::AppHandle, uri: &str) -> CmdResult<FileSource> {
    let (name, len) = query_info(uri)?;
    let url = tauri::Url::parse(uri).map_err(AppError::other)?;
    let app = app.clone();
    let open: Opener = Arc::new(move || {
        let mut opts = OpenOptions::new();
        opts.read(true);
        app.fs().open(FilePath::Url(url.clone()), opts)
    });
    Ok(FileSource { name, len, open })
}

/// Display name and size via `ContentResolver.query` on the Android main thread.
fn query_info(uri: &str) -> CmdResult<(String, Option<u64>)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let uri = uri.to_string();
    tauri::wry::prelude::dispatch(move |env, activity, _webview| {
        let _ = tx.send(query(env, activity, &uri).map_err(|e| e.to_string()));
    });
    rx.recv_timeout(Duration::from_secs(10))
        .map_err(|_| AppError::io("content resolver did not answer"))?
        .map_err(AppError::io)
}

fn query(
    env: &mut JNIEnv,
    activity: &JObject,
    uri: &str,
) -> jni::errors::Result<(String, Option<u64>)> {
    let juri = env.new_string(uri)?;
    let uri_obj = env
        .call_static_method(
            "android/net/Uri",
            "parse",
            "(Ljava/lang/String;)Landroid/net/Uri;",
            &[JValue::Object(&juri)],
        )?
        .l()?;
    let resolver = env
        .call_method(
            activity,
            "getContentResolver",
            "()Landroid/content/ContentResolver;",
            &[],
        )?
        .l()?;
    let string_class = env.find_class("java/lang/String")?;
    let projection = env.new_object_array(2, &string_class, JObject::null())?;
    env.set_object_array_element(&projection, 0, env.new_string(DISPLAY_NAME)?)?;
    env.set_object_array_element(&projection, 1, env.new_string(SIZE)?)?;
    let null = JObject::null();
    let cursor = env
        .call_method(
            &resolver,
            "query",
            "(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;",
            &[
                JValue::Object(&uri_obj),
                JValue::Object(&projection),
                JValue::Object(&null),
                JValue::Object(&null),
                JValue::Object(&null),
            ],
        )?
        .l()?;
    let fallback = uri.rsplit('/').next().unwrap_or("file").to_string();
    if cursor.is_null() || !env.call_method(&cursor, "moveToFirst", "()Z", &[])?.z()? {
        let name = with_extension(env, &resolver, &uri_obj, fallback);
        return Ok((name, None));
    }
    let name = column_string(env, &cursor, DISPLAY_NAME)?.unwrap_or(fallback);
    let size = column_long(env, &cursor, SIZE)?;
    env.call_method(&cursor, "close", "()V", &[])?;
    let name = with_extension(env, &resolver, &uri_obj, name);
    Ok((name, size))
}

/// A name without a dot gets the extension of the file's content type:
/// some providers give a bare name, and without the extension the file
/// goes out, and is saved, as an unknown one. Never fails the pick.
fn with_extension(env: &mut JNIEnv, resolver: &JObject, uri: &JObject, name: String) -> String {
    if name.contains('.') {
        return name;
    }
    match extension_of(env, resolver, uri) {
        Ok(Some(ext)) => format!("{name}.{ext}"),
        Ok(None) => name,
        Err(_) => {
            let _ = env.exception_clear();
            name
        }
    }
}

/// `ContentResolver.getType` through `MimeTypeMap`.
fn extension_of(env: &mut JNIEnv, resolver: &JObject, uri: &JObject) -> jni::errors::Result<Option<String>> {
    let mime = env
        .call_method(resolver, "getType", "(Landroid/net/Uri;)Ljava/lang/String;", &[JValue::Object(uri)])?
        .l()?;
    if mime.is_null() {
        return Ok(None);
    }
    let map = env
        .call_static_method("android/webkit/MimeTypeMap", "getSingleton", "()Landroid/webkit/MimeTypeMap;", &[])?
        .l()?;
    let ext = env
        .call_method(&map, "getExtensionFromMimeType", "(Ljava/lang/String;)Ljava/lang/String;", &[JValue::Object(&mime)])?
        .l()?;
    if ext.is_null() {
        return Ok(None);
    }
    let ext: String = env.get_string(&JString::from(ext))?.into();
    Ok((!ext.is_empty()).then_some(ext))
}

fn column_index(
    env: &mut JNIEnv,
    cursor: &JObject,
    column: &str,
) -> jni::errors::Result<Option<i32>> {
    let jcol = env.new_string(column)?;
    let idx = env
        .call_method(
            cursor,
            "getColumnIndex",
            "(Ljava/lang/String;)I",
            &[JValue::Object(&jcol)],
        )?
        .i()?;
    if idx < 0 {
        return Ok(None);
    }
    let is_null = env
        .call_method(cursor, "isNull", "(I)Z", &[JValue::Int(idx)])?
        .z()?;
    Ok((!is_null).then_some(idx))
}

fn column_string(
    env: &mut JNIEnv,
    cursor: &JObject,
    column: &str,
) -> jni::errors::Result<Option<String>> {
    let Some(idx) = column_index(env, cursor, column)? else {
        return Ok(None);
    };
    let value = env
        .call_method(
            cursor,
            "getString",
            "(I)Ljava/lang/String;",
            &[JValue::Int(idx)],
        )?
        .l()?;
    if value.is_null() {
        return Ok(None);
    }
    let s: String = env.get_string(&JString::from(value))?.into();
    Ok((!s.is_empty()).then_some(s))
}

fn column_long(
    env: &mut JNIEnv,
    cursor: &JObject,
    column: &str,
) -> jni::errors::Result<Option<u64>> {
    let Some(idx) = column_index(env, cursor, column)? else {
        return Ok(None);
    };
    let value = env
        .call_method(cursor, "getLong", "(I)J", &[JValue::Int(idx)])?
        .j()?;
    Ok(u64::try_from(value).ok())
}
