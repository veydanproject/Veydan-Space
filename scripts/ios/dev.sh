#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# `tauri ios dev` of a product on a Mac: the UI from its dev server with hot
# reload, the app on a simulator or the phone (internal/platform-spec.md 13.6):
#   scripts/ios/dev.sh [product] ["<phone>"] [tauri ios dev arguments]
#   make ios-dev [product] [DEVICE="<phone>"]
# Without a device it opens Xcode (--open) and waits: pick a simulator or the
# phone there and press Run; the build phase of Rust talks to this CLI, and
# the UI is served with hot reload. A device by name is for the phone only:
# from Xcode 27 `devicectl` lists the simulators too, and tauri-cli 2.11
# takes a simulator named so for a phone (it archives and asks for a team).
# A simulator needs no signature; the phone needs the Apple account of Xcode
# (scripts/ios/device.sh says what).
# The Xcode project is made once by scripts/ios/init.sh when it is missing.
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

export CARGO_TARGET_DIR="$ROOT_DIR/data/target-ios/$PRODUCT"

# The dev server of the phone's UI and its HMR port (the devUrl of
# tauri.ios.conf.json, the Android dev port of the product): a server left
# from an earlier run would serve the wrong build.
port="$PRODUCT_ANDROID_DEV_PORT"
for p in "$port" "$((port + 1))"; do
  for pid in $(lsof -nP -t -iTCP:"$p" -sTCP:LISTEN 2>/dev/null | sort -u); do
    kill "$pid" 2>/dev/null || true
  done
done

tauri_cli_ready
if [ $# -eq 0 ] || [ "${1#-}" != "$1" ]; then
  exec "$TAURI_CLI" ios dev --open "$@"
fi
exec "$TAURI_CLI" ios dev "$@"
