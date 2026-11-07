// swift-tools-version:5.5
// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The phone's side of pushes on iOS: the token of APNs, the permission,
// the tap, the background push of the keeper. tauri-plugin builds this
// package as a static library named after the crate (build.rs:
// `.ios_path("ios")`) and links it into the app, where the Rust side of the
// plugin lives in the same binary. The Tauri API is the copy tauri-plugin
// puts beside this folder at build time (`../.tauri`).
//
// No resources: the app links this package as an archive, so a bundle of
// its own would not be copied into the app, and the text of an alert push
// (`loc-key` push.new) is looked up in the app's bundle alone. Those
// strings are in AppResources/, which scripts/ios/init.sh puts into the
// Xcode project of the app.
//
// The first line must stay the tools version: SwiftPM reads it before
// anything else. 5.5 is the first that names iOS 15, the app's minimum
// (tauri.ios.conf.json, bundle.iOS.minimumSystemVersion).

import PackageDescription

let package = Package(
  name: "tauri-plugin-veydan-push",
  platforms: [
    .iOS(.v15)
  ],
  products: [
    .library(
      name: "tauri-plugin-veydan-push",
      type: .static,
      targets: ["tauri-plugin-veydan-push"])
  ],
  dependencies: [
    .package(name: "Tauri", path: "../.tauri/tauri-api")
  ],
  targets: [
    .target(
      name: "tauri-plugin-veydan-push",
      dependencies: [
        .byName(name: "Tauri")
      ],
      path: "Sources/VeydanPush",
      // Named as well as imported: the app links this library as an
      // archive, and the frameworks are what the final link must add.
      linkerSettings: [
        .linkedFramework("Security"),
        .linkedFramework("UserNotifications"),
      ])
  ]
)
