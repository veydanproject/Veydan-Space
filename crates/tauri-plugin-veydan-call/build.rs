// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Commands of the Kotlin side. The web page never calls them: the app's Rust
// code does. They are listed because the plugin build wants the list.
const COMMANDS: &[&str] = &[
    "show_incoming",
    "dismiss_incoming",
    "start_ongoing",
    "stop",
    "set_audio_route",
    "list_audio_routes",
    "keep_awake",
    "start_camera",
    "stop_camera",
    "switch_camera",
    "camera_stats",
    "request_permission",
    "take_actions",
    "register_listener",
    "remove_listener",
];

fn main() {
    veydan_build_cfg::windows_test_manifest();
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
