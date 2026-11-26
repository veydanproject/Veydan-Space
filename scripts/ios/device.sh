#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# A product on the iPhone plugged into this Mac: built, signed with the
# development certificate of the Apple account in Xcode, installed and
# started (`tauri ios run`; internal/platform-spec.md 13.6):
#   scripts/ios/device.sh [product] [tauri ios run arguments]   # --release, --features devtools
#   make ios-device [product]
#
# What it needs, once:
#   - the phone trusts this Mac and has Developer Mode on
#     (Settings → Privacy & Security → Developer Mode);
#   - an Apple account in Xcode (Settings → Accounts), whose team signs the
#     app; a free account works (IOS_FREE_TEAM=1), its builds expire after 7 days.
# The build is net.veydan.mobile.dev unless IOS_BUNDLE_ID says otherwise: an
# app of its own beside the TestFlight build, without push. The team, the
# phone, the registration of the phone and the rest: scripts/ios/device-env.sh.
set -euo pipefail

IOS_SCRIPTS_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$IOS_SCRIPTS_DIR/env.sh"
cd "$ROOT_DIR"
source "$ROOT_DIR/scripts/product.sh"
case "${1:-}" in
  space | notes | pass | chat) product_env "$1"; shift ;;
  *) product_env space ;;
esac

[ -d "$APP_DIR/gen/apple" ] || bash "$IOS_SCRIPTS_DIR/init.sh" "$PRODUCT"
source "$IOS_SCRIPTS_DIR/device-env.sh"

ios_team
ios_phone
export CARGO_TARGET_DIR="$ROOT_DIR/data/target-ios/$PRODUCT"

ios_keep_project
trap ios_restore_project EXIT
ios_identifier
ios_register_phone

tauri_cli_ready
# `tauri ios run` builds, archives, exports and installs. Its export of
# tauri-cli 2.11 dies on Xcode 27 (the temporary exportOptionsPlist is gone,
# code 70) after the archive is made and signed: then the app of the archive
# is installed and started here.
started="$(mktemp)"
status=0
"$TAURI_CLI" ios run "$IOS_PHONE_NAME" --config "$IOS_CONFIG" "$@" || status=$?
if [ "$status" -ne 0 ]; then
  app="$(ls -d "$IOS_APPLE_DIR"/build/*.xcarchive/Products/Applications/*.app 2>/dev/null | head -1)"
  if [ -n "$app" ] && [ "$app" -nt "$started" ] &&
    [ "$(/usr/libexec/PlistBuddy -c 'Print CFBundleIdentifier' "$app/Info.plist" 2>/dev/null)" = "$IOS_BUNDLE_ID" ]; then
    echo ">> tauri ios run failed after the archive; installing the archived app"
    ios_install_app "$app"
    status=0
  fi
fi
rm -f "$started"
exit "$status"
