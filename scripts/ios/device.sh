#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# A product on the iPhone plugged into this Mac: built, signed with the
# development certificate of the Apple account in Xcode, installed and
# started (`tauri ios run`; internal/platform-spec.md 13.6):
#   scripts/ios/device.sh [product] [tauri ios run arguments]   # --release, --features devtools
#   make ios-device [product]                                   # IOS_FREE_TEAM=1 with a free account
#
# What it needs, once:
#   - the phone trusts this Mac and has Developer Mode on
#     (Settings → Privacy & Security → Developer Mode);
#   - an Apple account in Xcode (Settings → Accounts), whose team signs the
#     app; a free account works, its builds expire after 7 days.
# The team is APPLE_DEVELOPMENT_TEAM, or the one of the "Apple Development"
# certificate in the keychain when there is exactly one team.
# IOS_BUNDLE_ID gives the app another identifier: a free account cannot sign
# net.veydan.mobile when that identifier belongs to another team.
# IOS_DEVICE names the phone (as `xcrun devicectl list devices` shows it)
# when more than one is plugged in.
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

# The team: given, or read from the development certificates (their OU).
if [ -z "${APPLE_DEVELOPMENT_TEAM:-}" ]; then
  teams="$(security find-certificate -a -c "Apple Development" -p 2>/dev/null |
    awk '/BEGIN CERT/{c=""} {c=c $0 "\n"} /END CERT/{print c | "openssl x509 -noout -subject -nameopt multiline"; close("openssl x509 -noout -subject -nameopt multiline")}' |
    sed -n 's/^ *organizationalUnitName *= *//p' | sort -u)"
  if [ "$(printf '%s\n' "$teams" | grep -c .)" = 1 ]; then
    APPLE_DEVELOPMENT_TEAM="$teams"
  fi
fi
if [ -z "${APPLE_DEVELOPMENT_TEAM:-}" ]; then
  echo ">> ios: no team to sign with. Add your Apple account in Xcode (Settings → Accounts →" >&2
  echo "   + → Apple ID), then Manage Certificates → + → Apple Development; or set" >&2
  echo "   APPLE_DEVELOPMENT_TEAM=<Team ID>${teams:+ (the certificates name: $(echo $teams))}" >&2
  exit 1
fi
export APPLE_DEVELOPMENT_TEAM
echo ">> signing with the team $APPLE_DEVELOPMENT_TEAM"

# The phone: the one plugged in and paired (IOS_DEVICE when several are).
devices_json="$(mktemp)"
trap 'rm -f "$devices_json"' EXIT
xcrun devicectl list devices --json-output "$devices_json" >/dev/null 2>&1 || true
device="$(IOS_DEVICE="${IOS_DEVICE:-}" node -e '
  const all = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8")).result?.devices ?? [];
  const phones = all.filter((d) => d.hardwareProperties?.platform === "iOS" && d.hardwareProperties?.reality !== "simulated"
    && d.connectionProperties?.pairingState === "paired");
  const want = process.env.IOS_DEVICE;
  const pick = want ? phones.find((d) => d.deviceProperties?.name === want || d.hardwareProperties?.udid === want) : phones.length === 1 ? phones[0] : undefined;
  if (pick) { console.log(pick.deviceProperties.name); process.exit(0); }
  console.error(phones.length ? ">> ios: several phones: IOS_DEVICE=" + phones.map((d) => JSON.stringify(d.deviceProperties.name)).join(" or ") : ">> ios: no paired iPhone is plugged in (trust this Mac on the phone, Developer Mode on)");
  process.exit(1);' "$devices_json")"
echo ">> the phone: $device"

export CARGO_TARGET_DIR="$ROOT_DIR/data/target-ios/$PRODUCT"

# `tauri ios run` writes the team and the identifier of this run into the
# Xcode project (project.pbxproj, Info.plist), which is in git: the files are
# kept here and put back when the run ends, so a run leaves the tree clean.
keep="$(mktemp -d)"
apple="$APP_DIR/gen/apple"
kept=()
for f in "$apple"/*.xcodeproj/project.pbxproj "$apple"/*_iOS/Info.plist "$apple"/*_iOS/*_iOS.entitlements; do
  [ -f "$f" ] || continue
  mkdir -p "$keep/$(dirname "${f#"$apple"/}")"
  cp -p "$f" "$keep/${f#"$apple"/}"
  kept+=("${f#"$apple"/}")
done
restore() {
  for f in "${kept[@]}"; do cp -p "$keep/$f" "$apple/$f"; done
  rm -rf "$keep" "$devices_json" "$apple/.tauri/no-push"
}
trap restore EXIT

# IOS_FREE_TEAM=1: the personal team of a free account, which signs no push
# (aps-environment) and no identifier of another team. The build goes without
# the push entitlement and, unless IOS_BUNDLE_ID says otherwise, as
# net.veydan.mobile.dev: an app of its own beside any other build of Space on
# the phone. Pushes do not reach it.
if [ -n "${IOS_FREE_TEAM:-}" ]; then
  IOS_BUNDLE_ID="${IOS_BUNDLE_ID:-net.veydan.mobile.dev}"
  # The push plugin writes the entitlement during the build: told not to by
  # a file, since xcodebuild runs the Rust phase with an environment of its own.
  mkdir -p "$apple/.tauri" && : > "$apple/.tauri/no-push"
  for f in "$apple"/*_iOS/*_iOS.entitlements; do
    [ -f "$f" ] && perl -0pi -e 's{\s*<key>aps-environment</key>\s*<string>[^<]*</string>}{}' "$f"
  done
  echo ">> a free team: built without push"
fi

config=()
if [ -n "${IOS_BUNDLE_ID:-}" ]; then
  config=(--config "{\"identifier\":\"$IOS_BUNDLE_ID\"}")
  echo ">> the identifier of this build: $IOS_BUNDLE_ID"
fi

tauri_cli_ready
# Not exec: the trap puts the project back when the run ends.
"$TAURI_CLI" ios run "$device" "${config[@]}" "$@"
