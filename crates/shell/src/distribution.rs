// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Who installed the app.
//!
//! One build goes to GitHub (an APK) and to Google Play (an AAB). Play forbids
//! an app it installed to update itself or to sell anything outside its own
//! billing, so the app asks the system for the installer and, when it is
//! Play, the UI stops checking for updates and shows the plans without
//! prices and buttons. Nowhere else does the answer change anything.

use std::sync::OnceLock;

/// The package name of the Google Play store app.
pub const PLAY_INSTALLER: &str = "com.android.vending";

/// The store the installer package stands for; `None` for any other source
/// (a file, the shell of `adb`, another market).
pub fn is_store_installer(installer: Option<&str>) -> Option<&'static str> {
    match installer {
        Some(PLAY_INSTALLER) => Some("play"),
        _ => None,
    }
}

/// The package that installed the app: `InstallSourceInfo` where the system
/// has it (Android 11), the older `getInstallerPackageName` before.
#[cfg(target_os = "android")]
fn installer_package() -> Option<String> {
    use jni::objects::{JObject, JString, JValue};
    let (tx, rx) = std::sync::mpsc::channel();
    tauri::wry::prelude::dispatch(move |env, activity, _webview| {
        let mut ask = || -> jni::errors::Result<Option<String>> {
            let name = env
                .call_method(activity, "getPackageName", "()Ljava/lang/String;", &[])?
                .l()?;
            let manager = env
                .call_method(activity, "getPackageManager", "()Landroid/content/pm/PackageManager;", &[])?
                .l()?;
            let sdk = env.get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?.i()?;
            let installer: JObject = if sdk >= 30 {
                let info = env
                    .call_method(
                        &manager,
                        "getInstallSourceInfo",
                        "(Ljava/lang/String;)Landroid/content/pm/InstallSourceInfo;",
                        &[JValue::Object(&name)],
                    )?
                    .l()?;
                env.call_method(&info, "getInstallingPackageName", "()Ljava/lang/String;", &[])?
                    .l()?
            } else {
                env.call_method(
                    &manager,
                    "getInstallerPackageName",
                    "(Ljava/lang/String;)Ljava/lang/String;",
                    &[JValue::Object(&name)],
                )?
                .l()?
            };
            if installer.is_null() {
                return Ok(None);
            }
            let installer = JString::from(installer);
            let text: String = env.get_string(&installer)?.into();
            Ok(Some(text))
        };
        let answer = match ask() {
            Ok(answer) => answer,
            Err(_) => {
                let _ = env.exception_clear();
                None
            }
        };
        let _ = tx.send(answer);
    });
    rx.recv_timeout(std::time::Duration::from_secs(5)).ok().flatten()
}

#[cfg(not(target_os = "android"))]
fn installer_package() -> Option<String> {
    None
}

/// The store that installed the app (`"play"`), `None` for every other
/// source and every computer. Asked once: the installer does not change while
/// the app runs.
pub fn store() -> Option<&'static str> {
    static STORE: OnceLock<Option<&'static str>> = OnceLock::new();
    *STORE.get_or_init(|| is_store_installer(installer_package().as_deref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_play_is_a_store() {
        assert_eq!(is_store_installer(None), None);
        assert_eq!(is_store_installer(Some("")), None);
        assert_eq!(is_store_installer(Some("com.android.shell")), None);
        assert_eq!(is_store_installer(Some("com.android.vending")), Some("play"));
    }
}
