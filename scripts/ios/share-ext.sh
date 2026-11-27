#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# The Rust of the share extension (crates/share-ext) for the phone or the
# simulator, as the build phase «Build Share Extension» of the Xcode
# project runs it (apps/<product>/gen/apple/project.yml):
#   scripts/ios/share-ext.sh <product> <iphoneos|iphonesimulator> <debug|release> <out dir>
# The archive lands in <out dir>/libveydan_share_ext.a, where the target of
# the extension looks for it. Built into data/target-ios/<product>, beside
# the app's own Rust (scripts/ios/build.sh), with the same toolchain
# (scripts/ios/env.sh); a release of the app takes a release of it.
set -euo pipefail
PRODUCT="${1:?product}"
PLATFORM="${2:?iphoneos|iphonesimulator}"
CONFIGURATION="${3:?debug|release}"
OUT="${4:?out dir}"
IOS_SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Xcode's own compilers and flags are for the app. The build scripts of the
# crates (sqlite among them) are compiled for this Mac: without an SDK of
# its own, the compiler of Xcode's environment finds no <stdio.h>; cc-rs
# ignores a macOS SDKROOT for the phone's target and asks xcrun for that one.
unset CPATH LIBRARY_PATH IPHONEOS_DEPLOYMENT_TARGET
SDKROOT="$(xcrun --sdk macosx --show-sdk-path)"
export SDKROOT
source "$IOS_SCRIPTS_DIR/env.sh"
cd "$ROOT_DIR"
case "$PLATFORM" in
  iphoneos) target=aarch64-apple-ios ;;
  iphonesimulator) target=aarch64-apple-ios-sim ;;
  *) echo ">> share-ext: unknown platform $PLATFORM"; exit 1 ;;
esac
export CARGO_TARGET_DIR="$ROOT_DIR/data/target-ios/$PRODUCT"
profile_flag=()
profile_dir=debug
if [ "$(printf '%s' "$CONFIGURATION" | tr '[:upper:]' '[:lower:]')" = release ]; then
  profile_flag=(--release)
  profile_dir=release
fi
echo ">> share-ext: cargo build -p veydan-share-ext --target $target ${profile_flag[*]:-} (data/target-ios/$PRODUCT)"
cargo build -p veydan-share-ext --target "$target" "${profile_flag[@]}" --locked
mkdir -p "$OUT"
cp "$CARGO_TARGET_DIR/$target/$profile_dir/libveydan_share_ext.a" "$OUT/libveydan_share_ext.a"
echo ">> share-ext: $OUT/libveydan_share_ext.a"
