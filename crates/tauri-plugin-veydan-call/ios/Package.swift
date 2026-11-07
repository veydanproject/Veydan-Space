// swift-tools-version:5.5
// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The phone's side of a call on iOS: CallKit, the audio session, the
// camera. tauri-plugin builds this package as a static library named after
// the crate (build.rs: `.ios_path("ios")`) and links it into the app, where
// the Rust side of the plugin lives in the same binary. The Tauri API is the
// copy tauri-plugin puts beside this folder at build time (`../.tauri`).
//
// The first line must stay the tools version: SwiftPM reads it before
// anything else. 5.5 is the first that names iOS 15, the app's minimum
// (tauri.ios.conf.json, bundle.iOS.minimumSystemVersion).

import PackageDescription

let package = Package(
  name: "tauri-plugin-veydan-call",
  platforms: [
    .iOS(.v15)
  ],
  products: [
    .library(
      name: "tauri-plugin-veydan-call",
      type: .static,
      targets: ["tauri-plugin-veydan-call"])
  ],
  dependencies: [
    .package(name: "Tauri", path: "../.tauri/tauri-api")
  ],
  targets: [
    .target(
      name: "tauri-plugin-veydan-call",
      dependencies: [
        .byName(name: "Tauri")
      ],
      path: "Sources/VeydanCall",
      // Named as well as imported: the app links this library as an
      // archive, and the frameworks are what the final link must add.
      // PushKit is not yet: a call rings only while the app runs.
      linkerSettings: [
        .linkedFramework("AVFoundation"),
        .linkedFramework("CallKit"),
        .linkedFramework("CoreMedia"),
        .linkedFramework("CoreVideo"),
        .linkedFramework("Network"),
        .linkedFramework("UserNotifications"),
      ])
  ]
)
