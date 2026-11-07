#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# The iOS app of a product (internal/platform-spec.md 13.6):
#   scripts/ios/build.sh [product] simulator         # a debug .app for the simulator of Apple silicon
#   scripts/ios/build.sh [product] device-unsigned   # a release .ipa for the phone, not signed:
#                                                    # the owner signs it with his key, or the lab does (device-signed)
#   scripts/ios/build.sh [product] device-inspect    # the same, with the web inspector of the webview on
#                                                    # (the feature devtools): tests on the phone through Safari or pymobiledevice3
#   scripts/ios/build.sh [product] device-signed     # a release .ipa signed and exported as EXPORT_METHOD
#                                                    # (release-testing by default; app-store-connect for TestFlight)
#                                                    # with the Apple variables the CLI reads (13.6)
# The product is space when none is given. Arguments after the mode go to
# `tauri ios build`. What is built lands in release-assets/ios/, named for
# a release (Veydan_5.2.0_ios-arm64-unsigned.ipa).
#
# Each product builds into data/target-ios/<product>, as Android does
# (scripts/android/apk.sh). gen/apple/build is removed first: the CLI of
# this version fails a second build of the simulator over the first one
# ("failed to rename app: Directory not empty").
set -euo pipefail

case "${1:-}" in
  simulator | device-unsigned | device-inspect | device-signed) PRODUCT_ARG=space ;;
  *) PRODUCT_ARG="${1:-}"; shift || true ;;
esac
MODE="${1:-}"
case "$MODE" in
  simulator | device-unsigned | device-inspect | device-signed) ;;
  *) echo ">> usage: build.sh [product] simulator|device-unsigned|device-inspect|device-signed [tauri ios build arguments]"; exit 1 ;;
esac
shift

IOS_SCRIPTS_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$IOS_SCRIPTS_DIR/env.sh"
cd "$ROOT_DIR"
source "$ROOT_DIR/scripts/product.sh"
product_env "$PRODUCT_ARG"
GEN_APPLE="$APP_DIR/gen/apple"
if [ ! -d "$GEN_APPLE" ]; then
  echo ">> $GEN_APPLE is missing: create it once with scripts/ios/init.sh $PRODUCT"
  exit 1
fi
export CARGO_TARGET_DIR="$ROOT_DIR/data/target-ios/$PRODUCT"
tauri_cli_ready

# The name of the app as Xcode writes it: productName of the iOS config
# (tauri.ios.conf.json), the product's otherwise.
APP_NAME="$(node -e '
  const fs = require("fs"), p = process.argv[1];
  const c = (f) => fs.existsSync(f) ? JSON.parse(fs.readFileSync(f, "utf8")).productName : undefined;
  console.log(c(p + "/tauri.ios.conf.json") || c(p + "/tauri.conf.json"));' "$APP_DIR")"
ASSET_NAME="$(printf '%s' "$APP_NAME" | tr ' ' '.')"
OUT="$ROOT_DIR/release-assets/ios"
mkdir -p "$OUT"
rm -rf "$GEN_APPLE/build"

echo ">> Building $PRODUCT for iOS ($MODE) into data/target-ios/$PRODUCT"
case "$MODE" in
  simulator)
    "$TAURI_CLI" ios build --target aarch64-sim --debug "$@"
    app="$(find "$GEN_APPLE/build" -type d -name "*.app" -path "*sim*" -print -quit)"
    [ -n "$app" ] || app="$(find "$GEN_APPLE/build" -type d -name "*.app" -print -quit)"
    [ -n "$app" ] || { echo ">> no .app under $GEN_APPLE/build"; find "$GEN_APPLE/build" -maxdepth 3 | head -n 40; exit 1; }
    zip_name="${ASSET_NAME}_${PRODUCT_VERSION}_ios-arm64-sim.app.zip"
    (cd "$(dirname "$app")" && rm -f "$OUT/$zip_name" && zip -qry "$OUT/$zip_name" "$(basename "$app")")
    echo "$app" > "$OUT/.app-path"
    echo ">> simulator app: $OUT/$zip_name"
    ;;
  device-unsigned)
    # xcodebuild archives with CODE_SIGNING_ALLOWED=NO and the CLI zips the
    # .app of the archive as Payload/ into an .ipa (tauri-cli 2.11, --no-sign).
    "$TAURI_CLI" ios build --target aarch64 --no-sign "$@"
    ipa="$(find "$GEN_APPLE/build" -type f -name "*.ipa" -print -quit)"
    [ -n "$ipa" ] || { echo ">> no .ipa under $GEN_APPLE/build"; find "$GEN_APPLE/build" -maxdepth 3 | head -n 40; exit 1; }
    cp "$ipa" "$OUT/${ASSET_NAME}_${PRODUCT_VERSION}_ios-arm64-unsigned.ipa"
    echo ">> unsigned ipa: $OUT/${ASSET_NAME}_${PRODUCT_VERSION}_ios-arm64-unsigned.ipa"
    ;;
  device-inspect)
    # A release build, so the app runs as people get it, with the inspector of
    # WKWebView open (isInspectable) and the demo data of the products
    # (Settings, "Load demo data"; crates/shell/src/demo.rs): never a build
    # for anyone but us.
    "$TAURI_CLI" ios build --target aarch64 --no-sign --features devtools,demo "$@"
    ipa="$(find "$GEN_APPLE/build" -type f -name "*.ipa" -print -quit)"
    [ -n "$ipa" ] || { echo ">> no .ipa under $GEN_APPLE/build"; find "$GEN_APPLE/build" -maxdepth 3 | head -n 40; exit 1; }
    cp "$ipa" "$OUT/${ASSET_NAME}_${PRODUCT_VERSION}_ios-arm64-inspect-unsigned.ipa"
    echo ">> unsigned ipa with the inspector: $OUT/${ASSET_NAME}_${PRODUCT_VERSION}_ios-arm64-inspect-unsigned.ipa"
    ;;
  device-signed)
    : "${APPLE_DEVELOPMENT_TEAM:?APPLE_DEVELOPMENT_TEAM (the Team ID) is required for a signed build}"
    if [ -z "${APPLE_API_KEY:-}" ] && [ -z "${IOS_CERTIFICATE:-}" ]; then
      echo ">> a signed build needs APPLE_API_ISSUER, APPLE_API_KEY and APPLE_API_KEY_PATH (automatic signing) or IOS_CERTIFICATE, IOS_CERTIFICATE_PASSWORD and IOS_MOBILE_PROVISION (13.6)"
      exit 1
    fi
    "$TAURI_CLI" ios build --target aarch64 --export-method "${EXPORT_METHOD:-release-testing}" "$@"
    ipa="$(find "$GEN_APPLE/build" -type f -name "*.ipa" -print -quit)"
    [ -n "$ipa" ] || { echo ">> no .ipa under $GEN_APPLE/build"; find "$GEN_APPLE/build" -maxdepth 3 | head -n 40; exit 1; }
    cp "$ipa" "$OUT/${ASSET_NAME}_${PRODUCT_VERSION}_ios-arm64.ipa"
    echo ">> signed ipa: $OUT/${ASSET_NAME}_${PRODUCT_VERSION}_ios-arm64.ipa"
    ;;
esac
du -sh "$OUT"/* 2>/dev/null || true
