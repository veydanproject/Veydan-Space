#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# `tauri ios dev` of a product on a Mac: the UI from its dev server with hot
# reload (internal/platform-spec.md 13.6):
#   scripts/ios/dev.sh [product] [phone|"<simulator>"|"<phone name>"|<UDID>]
#   make ios-dev [product] [DEVICE=phone|"iPhone Duo"|…]
#
# The device:
#   phone          the iPhone plugged into this Mac (IOS_DEVICE when several)
#   "<simulator>"  a simulator by name or UDID (`xcrun simctl list devices`):
#                  booted and shown (DeviceHub of Xcode 27)
#   another name   a phone by name or UDID
#   none           Xcode opens and waits: pick a simulator there, press Run
# With a device the app is built by xcodebuild against the running CLI (its
# Rust phase talks to it), installed and started by the script; Ctrl-C stops
# the dev server. tauri-cli 2.11 cannot run on a device itself: its export
# dies on Xcode 27, and its devicectl takes simulators for phones.
# A phone loads the UI over Wi-Fi from this Mac's address in the local
# network (IOS_DEV_HOST names another): it needs Settings → Privacy &
# Security → Local Network → on, once (else "did you grant local network
# permissions?"). A simulator loads it from this Mac itself.
# The build is net.veydan.mobile.dev unless IOS_BUNDLE_ID says otherwise,
# without push, beside the TestFlight build; the team, the registration of
# the phone and the rest: scripts/ios/device-env.sh.
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
source "$IOS_SCRIPTS_DIR/device-env.sh"

target=""
case "${1:-}" in
  "" | -*) ;;
  phone) target=phone; shift ;;
  *)
    if ios_simulator "$1"; then target=simulator; else target=phone; IOS_DEVICE="$1"; fi
    shift ;;
esac

export CARGO_TARGET_DIR="$ROOT_DIR/data/target-ios/$PRODUCT"

# The dev server of the iOS UI and its HMR port (the devUrl of
# tauri.ios.conf.json, iosDevPort of products.json; Android has ports of its
# own, so both run at the same time): a server left from an earlier run
# would serve the wrong build.
port="$PRODUCT_IOS_DEV_PORT"
for p in "$port" "$((port + 1))"; do
  for pid in $(lsof -nP -t -iTCP:"$p" -sTCP:LISTEN 2>/dev/null | sort -u); do
    kill "$pid" 2>/dev/null || true
  done
done

case "$target" in
  phone) ios_team; ios_phone ;;
  simulator) ios_simulator_boot ;;
esac
ios_keep_project
cli=""
# The CLI runs in a process group of its own (set -m below): when it stops
# it signals its whole group, which would cut this script's cleanup short.
stop() {
  trap '' INT TERM
  if [ -n "$cli" ]; then
    kill -TERM -- "-$cli" 2>/dev/null || kill -TERM "$cli" 2>/dev/null || true
    wait "$cli" 2>/dev/null || true
  fi
  ios_restore_project
}
trap stop EXIT
trap 'exit 130' INT TERM
ios_identifier
tauri_cli_ready

if [ -z "$target" ]; then
  "$TAURI_CLI" ios dev --open --config "$IOS_CONFIG" "$@"
  exit
fi

host_args=()
if [ "$target" = phone ]; then
  ios_register_phone
  host="${IOS_DEV_HOST:-$(ipconfig getifaddr "$(route -n get default 2>/dev/null | awk '/interface:/{print $2}')" 2>/dev/null || true)}"
  if [ -z "$host" ]; then
    echo ">> ios: this Mac has no address in the local network; IOS_DEV_HOST=<address> names one" >&2
    exit 1
  fi
  host_args=(--host "$host")
else
  host=localhost
fi
mkdir -p "$IOS_LOG_DIR"
log="$IOS_LOG_DIR/dev-$PRODUCT.log"
echo ">> the dev server: http://$host:$port (log: ${log#"$ROOT_DIR"/})"

# The CLI runs the dev server, writes the project, opens Xcode and waits
# for the Rust phase of a build ("Opening Xcode"). The window of Xcode is not
# needed, and an Xcode left open writes the scheme of the project in git in
# its own format: the Xcode it opened is closed again (the CLI keeps
# running); an Xcode that was open before is left alone.
xcode_running() { pgrep -qx Xcode; }
xcode_before=""
xcode_running && xcode_before=1
set -m
"$TAURI_CLI" ios dev --open "${host_args[@]}" --config "$IOS_CONFIG" "$@" > >(tee "$log") 2>&1 &
cli=$!
set +m
until grep -q "Opening Xcode" "$log" 2>/dev/null; do
  kill -0 "$cli" 2>/dev/null || { echo ">> ios: tauri ios dev stopped (${log#"$ROOT_DIR"/})" >&2; exit 1; }
  sleep 1
done
if [ -z "$xcode_before" ]; then
  for _ in $(seq 1 30); do xcode_running && break; sleep 1; done
  osascript -e 'tell application "Xcode" to quit' >/dev/null 2>&1 || true
  echo ">> Xcode closed: not needed here"
fi

ios_project
derived="$ROOT_DIR/data/target-ios/$PRODUCT/xcode"
build_log="$IOS_LOG_DIR/xcodebuild-dev-$PRODUCT-$target.log"
if [ "$target" = phone ]; then
  name="$IOS_PHONE_NAME"
  build_args=(-destination "id=$IOS_PHONE_UDID" -allowProvisioningUpdates DEVELOPMENT_TEAM="$APPLE_DEVELOPMENT_TEAM")
  products=debug-iphoneos
else
  name="$IOS_SIM_NAME"
  build_args=(-destination "id=$IOS_SIM_UDID")
  products=debug-iphonesimulator
fi
echo ">> building for $name (log: ${build_log#"$ROOT_DIR"/})"
if ! xcodebuild -project "$IOS_XCODEPROJ" -scheme "$IOS_SCHEME" -configuration debug \
  -derivedDataPath "$derived" "${build_args[@]}" PRODUCT_BUNDLE_IDENTIFIER="$IOS_BUNDLE_ID" \
  build >"$build_log" 2>&1; then
  echo ">> ios: the build failed:" >&2
  grep -E "error:|error\[" "$build_log" | sort -u | head -10 >&2
  exit 1
fi
app="$(ls -d "$derived/Build/Products/$products"/*.app | head -1)"
failed() { grep -c "the start of the page failed" "$log" 2>/dev/null || true; }
before="$(failed)"
if [ "$target" = phone ]; then ios_install_app "$app"; else ios_simulator_install "$app"; fi
# The first load may come while Vite still compiles the modules on demand
# ("Importing a module script failed", ROO-176): once they are compiled, a
# second start loads.
for _ in $(seq 1 40); do
  sleep 1
  if [ "$(failed)" -gt "$before" ]; then
    echo ">> the first load failed while Vite compiled the modules; starting the app again"
    if [ "$target" = phone ]; then ios_launch_app; else ios_simulator_launch; fi
    break
  fi
done
echo ">> hot reload is on: an edit of ui/ shows on $name; Ctrl-C stops the dev server"
if [ "$target" = phone ]; then
  echo ">> a blank page or \"local network permissions\": Settings → Privacy & Security → Local Network → on"
fi
wait "$cli"
