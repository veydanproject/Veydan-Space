// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Commands of the phone's side (Kotlin on Android, Swift on iOS). The web
// page never calls them: the app's Rust code does. They are listed because
// the plugin build wants the list.
const COMMANDS: &[&str] = &[
    "get_state",
    "get_token",
    "delete_token",
    "permission_state",
    "request_permission",
    "set_context",
    "set_muted",
    "take_tap",
    "cancel",
    "store_keys",
    "clear_keys",
    "register_listener",
    "remove_listener",
    "keeper_foreground",
    "keeper_periodic",
    "open_settings",
];

fn main() {
    veydan_build_cfg::windows_test_manifest();
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        // The Swift package; tauri-plugin copies the Tauri API beside it
        // (`.tauri/tauri-api`, which ios/Package.swift names) and links it
        // when the target is iOS and the build runs on a Mac.
        .ios_path("ios")
        .build();
    // The Xcode project exists only on a Mac, and the API that writes into
    // it with it.
    #[cfg(target_os = "macos")]
    ios_project();
}

/// What iOS asks of an app that takes pushes, written into the Xcode project
/// the build runs from: the CLI's build phase (`tauri ios xcode-script`)
/// names it in `TAURI_IOS_PROJECT_PATH` and `TAURI_IOS_APP_NAME`, and
/// `tauri_plugin::mobile` writes nothing without them (a plain `cargo
/// build`). It runs before Xcode reads the two files: the Rust phase is
/// the first of the target.
///
/// - `aps-environment` in the entitlements: without it APNs gives the app
///   no token at all. `development` for a debug build, `production` for a
///   release one, the same split as the environment the app tells the push
///   server (`commands/push.rs`, `channel`). A build script cannot see the
///   profile Xcode signs with; a signed export (`build.sh device-signed`)
///   gets the value of its provisioning profile, which Xcode writes over
///   this one, and an unsigned .ipa signed later takes the entitlements its
///   signer gives.
/// - `remote-notification` in `UIBackgroundModes` of Info.plist, beside
///   what the product's Info.ios.plist names (`audio`, `voip`): iOS wakes
///   the app for a background push (the keeper of a channel) only with it.
#[cfg(target_os = "macos")]
fn ios_project() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("ios") {
        return;
    }
    println!("cargo:rerun-if-env-changed=TAURI_IOS_PROJECT_PATH");
    println!("cargo:rerun-if-env-changed=TAURI_IOS_APP_NAME");
    // A project made anew (scripts/ios/init.sh, every run of the lab while
    // gen/apple is not tracked) has neither line: the files are watched,
    // so a build that kept this script's last output writes them again.
    if let (Some(project), Ok(app)) =
        (std::env::var_os("TAURI_IOS_PROJECT_PATH"), std::env::var("TAURI_IOS_APP_NAME"))
    {
        let dir = std::path::PathBuf::from(project).join(format!("{app}_iOS"));
        println!("cargo:rerun-if-changed={}", dir.join(format!("{app}_iOS.entitlements")).display());
        println!("cargo:rerun-if-changed={}", dir.join("Info.plist").display());
    }

    // A build signed by the personal team of a free Apple account
    // (scripts/ios/device.sh, IOS_FREE_TEAM=1), which cannot sign the push
    // capability: the entitlement is taken out, and the app gets no push
    // token. The sign is the file `.tauri/no-push` in the Xcode project
    // (ignored by git), since xcodebuild runs this phase with an environment
    // of its own; VEYDAN_IOS_NO_PUSH=1 says the same to a plain cargo build.
    let marker = std::env::var_os("TAURI_IOS_PROJECT_PATH")
        .map(|project| std::path::PathBuf::from(project).join(".tauri").join("no-push"));
    if let Some(marker) = &marker {
        println!("cargo:rerun-if-changed={}", marker.display());
    }
    println!("cargo:rerun-if-env-changed=VEYDAN_IOS_NO_PUSH");
    let no_push = std::env::var_os("VEYDAN_IOS_NO_PUSH").is_some_and(|v| !v.is_empty())
        || marker.is_some_and(|m| m.exists());
    let environment = if std::env::var("PROFILE").as_deref() == Ok("release") { "production" } else { "development" };
    tauri_plugin::mobile::update_entitlements(|entitlements| {
        if no_push {
            entitlements.remove("aps-environment");
        } else {
            entitlements.insert("aps-environment".into(), plist::Value::String(environment.into()));
        }
    })
    .expect("the entitlement aps-environment could not be written into the Xcode project");

    tauri_plugin::mobile::update_info_plist(|info| {
        // `Dictionary::entry` of plist is behind an unstable feature: get and insert.
        if !info.contains_key("UIBackgroundModes") {
            info.insert("UIBackgroundModes".into(), plist::Value::Array(Vec::new()));
        }
        if let Some(modes) = info.get_mut("UIBackgroundModes").and_then(plist::Value::as_array_mut) {
            if !modes.iter().any(|mode| mode.as_string() == Some("remote-notification")) {
                modes.push(plist::Value::String("remote-notification".into()));
            }
        }
    })
    .expect("the background mode remote-notification could not be written into Info.plist");
}
