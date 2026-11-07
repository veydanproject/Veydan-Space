// swift-tools-version:5.5
// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The phone's gallery on iOS for the sheet of the paperclip: PhotoKit, the
// camera, the system's Photo Picker, HEIC written as JPEG by ImageIO, the
// share sheet. tauri-plugin builds this package as a static library named
// after the crate (build.rs: `.ios_path("ios")`) and links it into the
// app, where the Rust side of the plugin lives in the same binary. The
// Tauri API is the copy tauri-plugin puts beside this folder at build time
// (`../.tauri`).
//
// The first line must stay the tools version: SwiftPM reads it before
// anything else. 5.5 is the first that names iOS 15, the app's minimum
// (tauri.ios.conf.json, bundle.iOS.minimumSystemVersion).

import PackageDescription

let package = Package(
  name: "tauri-plugin-veydan-media",
  platforms: [
    .iOS(.v15)
  ],
  products: [
    .library(
      name: "tauri-plugin-veydan-media",
      type: .static,
      targets: ["tauri-plugin-veydan-media"])
  ],
  dependencies: [
    .package(name: "Tauri", path: "../.tauri/tauri-api")
  ],
  targets: [
    .target(
      name: "tauri-plugin-veydan-media",
      dependencies: [
        .byName(name: "Tauri")
      ],
      path: "Sources/VeydanMedia",
      // Named as well as imported: the app links this library as an
      // archive, and the frameworks are what the final link must add
      // (the products name them in bundle.iOS.frameworks too).
      linkerSettings: [
        .linkedFramework("AVFoundation"),
        .linkedFramework("CryptoKit"),
        .linkedFramework("ImageIO"),
        .linkedFramework("Photos"),
        .linkedFramework("PhotosUI"),
        .linkedFramework("UIKit"),
        .linkedFramework("UniformTypeIdentifiers"),
      ])
  ]
)
