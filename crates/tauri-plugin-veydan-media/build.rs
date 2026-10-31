// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Commands of the Kotlin side. The web page never calls them: the app's Rust
// code does. They are listed because the plugin build wants the list.
const COMMANDS: &[&str] = &[
    "access",
    "request",
    "recent",
    "albums",
    "thumb",
    "readable",
    "capture",
    "forget_capture",
    "pick_system",
    "convert",
    "share_text",
    "save_picture",
];

fn main() {
    veydan_build_cfg::windows_test_manifest();
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
