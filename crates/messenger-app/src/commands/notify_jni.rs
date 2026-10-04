// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The push handler's way into the messenger, without the app.
//!
//! A push starts the app's process with no window, so no Tauri and no
//! runtime; the handler loads this library and calls the one function
//! below. It must count on nothing of the app's: not its state, not its
//! async runtime, not its crypto provider. It opens what it needs itself
//! and closes it before it returns.

use jni::objects::{JByteArray, JClass, JString};
use jni::sys::jstring;
use jni::JNIEnv;
use messenger_notify::{describe, KeyBundle, PushData};
use std::path::PathBuf;
use std::time::Duration;
use zeroize::Zeroize;

/// `net.veydan.push.Core.describe(dataDir, bundle, push)`: the outcome as
/// JSON, or an object with `outcome = "error"` and `error` saying why.
/// Nothing panics out of here: a panic would take the process down, and
/// the process may be the app.
#[no_mangle]
pub extern "system" fn Java_net_veydan_push_Core_describe<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    data_dir: JString<'l>,
    bundle: JByteArray<'l>,
    push: JString<'l>,
) -> jstring {
    let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let data_dir: String = env.get_string(&data_dir).map(Into::into).unwrap_or_default();
        let mut bundle = env.convert_byte_array(&bundle).unwrap_or_default();
        let push: String = env.get_string(&push).map(Into::into).unwrap_or_default();
        let out = run(PathBuf::from(data_dir), &bundle, &push);
        bundle.zeroize();
        out
    }))
    .unwrap_or_else(|_| error("panic"));
    match env.new_string(answer) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

fn error(message: impl std::fmt::Display) -> String {
    serde_json::json!({ "outcome": "error", "error": message.to_string() }).to_string()
}

fn run(data_dir: PathBuf, bundle: &[u8], push: &str) -> String {
    // No bytes: the handler has no keys, and only the chat is looked up.
    let bundle = match bundle {
        [] => None,
        bytes => match KeyBundle::from_json(bytes) {
            Ok(b) => Some(b),
            Err(e) => return error(e),
        },
    };
    let data: std::collections::BTreeMap<String, String> = match serde_json::from_str(push) {
        Ok(d) => d,
        Err(e) => return error(format!("push: {e}")),
    };
    let push = match PushData::parse(&data) {
        Ok(p) => p,
        Err(e) => return error(e),
    };
    // A runtime of its own, gone when the answer is: the app's, if the app
    // is up in this process, is on other threads and none of ours.
    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => return error(format!("runtime: {e}")),
    };
    let out = rt.block_on(async {
        match describe(&data_dir, bundle.as_ref(), &push).await {
            Ok(outcome) => serde_json::to_string(&outcome).unwrap_or_else(error),
            Err(e) => error(e),
        }
    });
    rt.shutdown_timeout(Duration::from_secs(1));
    out
}
