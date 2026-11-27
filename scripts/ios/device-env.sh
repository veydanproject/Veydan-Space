#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# What a build for the iPhone plugged into this Mac needs, shared by
# scripts/ios/device.sh (a build of its own) and scripts/ios/dev.sh (hot
# reload): the team that signs, the phone, the identifier of the build, the
# Xcode project kept as git has it, the phone registered with the team, and
# the app installed and started by devicectl. Meant to be *sourced* after
# env.sh and product_env, from the root.
#
#   APPLE_DEVELOPMENT_TEAM  the team; else the one of the "Apple Development"
#                           certificates in the keychain, when there is one
#   IOS_DEVICE              the phone (name or UDID) when several are plugged in
#   IOS_BUNDLE_ID           the identifier; by default the product's with
#                           `.dev` (net.veydan.mobile.dev), an app of its own
#                           beside the TestFlight build, which keeps its data.
#                           The product's own identifier only when named here
#   IOS_FREE_TEAM=1         the personal team of a free account: no push
#
# Push reaches only the product's own identifier (vpush knows that topic), and
# a free team signs no push capability: any other build goes without the
# entitlement aps-environment (the push plugin leaves it out when the Xcode
# project holds .tauri/no-push, since xcodebuild runs the Rust phase with an
# environment of its own).

IOS_APPLE_DIR="$APP_DIR/gen/apple"
IOS_LOG_DIR="$ROOT_DIR/tmp/ios"

# The team: given, or read from the development certificates (their OU).
ios_team() {
  local teams=""
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
    echo "   + → Apple ID) and set APPLE_DEVELOPMENT_TEAM=<Team ID>${teams:+ (the certificates name: $(echo $teams))}" >&2
    return 1
  fi
  export APPLE_DEVELOPMENT_TEAM
  echo ">> signing with the team $APPLE_DEVELOPMENT_TEAM"
}

# The phone: the one plugged in and paired (IOS_DEVICE when several are).
# Sets IOS_PHONE_NAME and IOS_PHONE_UDID.
ios_phone() {
  local json line
  json="$(mktemp)"
  xcrun devicectl list devices --json-output "$json" >/dev/null 2>&1 || true
  line="$(IOS_DEVICE="${IOS_DEVICE:-}" node -e '
    const all = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8")).result?.devices ?? [];
    const phones = all.filter((d) => d.hardwareProperties?.platform === "iOS" && d.hardwareProperties?.reality !== "simulated"
      && d.connectionProperties?.pairingState === "paired");
    const want = process.env.IOS_DEVICE;
    const pick = want ? phones.find((d) => d.deviceProperties?.name === want || d.hardwareProperties?.udid === want) : phones.length === 1 ? phones[0] : undefined;
    if (pick) { console.log(pick.hardwareProperties.udid + "\t" + pick.deviceProperties.name); process.exit(0); }
    console.error(phones.length ? ">> ios: several phones: IOS_DEVICE=" + phones.map((d) => JSON.stringify(d.deviceProperties.name)).join(" or ") : ">> ios: no paired iPhone is plugged in (trust this Mac on the phone, Developer Mode on)");
    process.exit(1);' "$json")" || { rm -f "$json"; return 1; }
  rm -f "$json"
  IOS_PHONE_UDID="${line%%$'\t'*}"
  IOS_PHONE_NAME="${line#*$'\t'}"
  echo ">> the phone: $IOS_PHONE_NAME ($IOS_PHONE_UDID)"
}

# The identifier of this build and whether it carries push. Sets
# IOS_BUNDLE_ID and IOS_CONFIG (the --config of the tauri CLI).
ios_identifier() {
  IOS_BUNDLE_ID="${IOS_BUNDLE_ID:-$PRODUCT_IOS_IDENTIFIER.dev}"
  IOS_CONFIG="{\"identifier\":\"$IOS_BUNDLE_ID\"}"
  echo ">> the identifier of this build: $IOS_BUNDLE_ID"
  if [ -n "${IOS_FREE_TEAM:-}" ] || [ "$IOS_BUNDLE_ID" != "$PRODUCT_IOS_IDENTIFIER" ]; then
    mkdir -p "$IOS_APPLE_DIR/.tauri" && : > "$IOS_APPLE_DIR/.tauri/no-push"
    local f
    for f in "$IOS_APPLE_DIR"/*_iOS/*_iOS.entitlements; do
      [ -f "$f" ] && perl -0pi -e 's{\s*<key>aps-environment</key>\s*<string>[^<]*</string>}{}' "$f"
    done
    echo ">> built without push (push reaches $PRODUCT_IOS_IDENTIFIER only)"
  fi
  # The app group of this build is its own (group.<identifier>): the app and
  # its share extension keep the data in it (crates/shell/src/ios_group.rs),
  # and a dev build beside the TestFlight app must not open the other's.
  if [ "$IOS_BUNDLE_ID" != "$PRODUCT_IOS_IDENTIFIER" ]; then
    local f
    for f in "$IOS_APPLE_DIR"/*_iOS/*.entitlements "$IOS_APPLE_DIR"/ShareExtension/*.entitlements; do
      [ -f "$f" ] && IOS_GROUP_FROM="group.$PRODUCT_IOS_IDENTIFIER" IOS_GROUP_TO="group.$IOS_BUNDLE_ID" \
        perl -pi -e 's{\Q$ENV{IOS_GROUP_FROM}\E(?![.\w])}{$ENV{IOS_GROUP_TO}}g' "$f"
    done
    echo ">> the app group of this build: group.$IOS_BUNDLE_ID"
  fi
}

# The tauri CLI writes the team and the identifier of a run into the Xcode
# project (project.pbxproj, Info.plist, the entitlements; `ios dev` writes
# the scheme anew in its own older format), which is in git:
# the files are kept here and put back by ios_restore_project, so a run
# leaves the tree clean. Call ios_keep_project before ios_identifier.
ios_keep_project() {
  # A run killed outright (SIGKILL) could not put them back: kept now, the
  # changes of that run would stay. Said, not undone: they may be wanted.
  if [ -n "$(git -C "$ROOT_DIR" status --porcelain -- "$IOS_APPLE_DIR" 2>/dev/null)" ]; then
    echo ">> ios: ${IOS_APPLE_DIR#"$ROOT_DIR"/} differs from git (a run killed before it cleaned up?);" >&2
    echo "   the run keeps it as it is; 'git restore ${IOS_APPLE_DIR#"$ROOT_DIR"/}' puts git's back" >&2
  fi
  IOS_KEEP_DIR="$(mktemp -d)"
  IOS_KEPT=()
  local f
  for f in "$IOS_APPLE_DIR"/*.xcodeproj/project.pbxproj "$IOS_APPLE_DIR"/*.xcodeproj/xcshareddata/xcschemes/*.xcscheme \
    "$IOS_APPLE_DIR"/*_iOS/Info.plist "$IOS_APPLE_DIR"/*_iOS/*_iOS.entitlements \
    "$IOS_APPLE_DIR"/ShareExtension/Info.plist "$IOS_APPLE_DIR"/ShareExtension/*.entitlements; do
    [ -f "$f" ] || continue
    mkdir -p "$IOS_KEEP_DIR/$(dirname "${f#"$IOS_APPLE_DIR"/}")"
    cp -p "$f" "$IOS_KEEP_DIR/${f#"$IOS_APPLE_DIR"/}"
    IOS_KEPT+=("${f#"$IOS_APPLE_DIR"/}")
  done
}

ios_restore_project() {
  [ -n "${IOS_KEEP_DIR:-}" ] || return 0
  local f
  for f in "${IOS_KEPT[@]}"; do cp -p "$IOS_KEEP_DIR/$f" "$IOS_APPLE_DIR/$f"; done
  rm -rf "$IOS_KEEP_DIR" "$IOS_APPLE_DIR/.tauri/no-push"
  IOS_KEEP_DIR=""
}

# The Xcode project and its scheme (veydanspace.xcodeproj, veydanspace_iOS).
ios_project() {
  IOS_XCODEPROJ="$(ls -d "$IOS_APPLE_DIR"/*.xcodeproj | head -1)"
  IOS_SCHEME="$(basename "$IOS_XCODEPROJ" .xcodeproj)_iOS"
}

# Whether a provisioning profile of the team names this phone: the phone is
# registered with the team, and Xcode can make the profile of any identifier.
ios_phone_registered() {
  local p
  for p in "$HOME/Library/Developer/Xcode/UserData/Provisioning Profiles"/*.mobileprovision \
    "$HOME/Library/MobileDevice/Provisioning Profiles"/*.mobileprovision; do
    [ -f "$p" ] || continue
    security cms -D -i "$p" 2>/dev/null |
      awk -v team="$APPLE_DEVELOPMENT_TEAM" -v udid="$IOS_PHONE_UDID" '
        index($0, "<string>" team "</string>") { t = 1 }
        index($0, "<string>" udid "</string>") { u = 1 }
        END { exit !(t && u) }' && return 0
  done
  return 1
}

# The tauri CLI passes -allowProvisioningUpdates but not
# -allowProvisioningDeviceRegistration, so a phone the team does not know yet
# gets no profile ("Your team has no devices"). One build of xcodebuild with
# that flag registers it, makes the development certificate when there is
# none and the profile of this identifier; its Rust phase fails without the
# CLI, which is expected: the signing comes before it.
ios_register_phone() {
  ios_phone_registered && return 0
  ios_project
  mkdir -p "$IOS_LOG_DIR"
  local log="$IOS_LOG_DIR/register-$IOS_PHONE_UDID.log"
  echo ">> registering the phone with the team $APPLE_DEVELOPMENT_TEAM (once; log: ${log#"$ROOT_DIR"/})"
  xcodebuild -project "$IOS_XCODEPROJ" -scheme "$IOS_SCHEME" -configuration debug \
    -destination "id=$IOS_PHONE_UDID" -allowProvisioningUpdates -allowProvisioningDeviceRegistration \
    DEVELOPMENT_TEAM="$APPLE_DEVELOPMENT_TEAM" PRODUCT_BUNDLE_IDENTIFIER="$IOS_BUNDLE_ID" \
    CODE_SIGN_STYLE=Automatic build >"$log" 2>&1 || true
  if ! ios_phone_registered; then
    echo ">> ios: the phone could not be registered with the team $APPLE_DEVELOPMENT_TEAM:" >&2
    grep -E "error:" "$log" | sort -u | head -5 >&2
    return 1
  fi
  echo ">> the phone is registered"
}

# Start the app on the phone (again, when it runs).
ios_launch_app() {
  xcrun devicectl device process launch --terminate-existing --device "$IOS_PHONE_UDID" "$IOS_BUNDLE_ID" >/dev/null
  echo ">> started $IOS_BUNDLE_ID on $IOS_PHONE_NAME"
}

# Install the app on the phone and start it.
ios_install_app() {
  local app="$1"
  echo ">> installing ${app#"$ROOT_DIR"/} on $IOS_PHONE_NAME"
  xcrun devicectl device install app --device "$IOS_PHONE_UDID" "$app" >/dev/null
  ios_launch_app
}

# A simulator by name or UDID ("iPhone Duo"), of the newest runtime that has
# it. Sets IOS_SIM_UDID and IOS_SIM_NAME; fails quietly when there is none.
ios_simulator() {
  local line
  line="$(xcrun simctl list devices available -j 2>/dev/null | IOS_SIMULATOR="$1" node -e '
    const d = JSON.parse(require("fs").readFileSync(0, "utf8")).devices;
    const want = process.env.IOS_SIMULATOR;
    for (const r of Object.keys(d).filter((r) => /iOS/.test(r)).sort().reverse()) {
      const hit = d[r].find((x) => x.isAvailable && (x.udid === want || x.name === want));
      if (hit) { console.log(hit.udid + "\t" + hit.name); process.exit(0); }
    }
    process.exit(1);')" || return 1
  IOS_SIM_UDID="${line%%$'\t'*}"
  IOS_SIM_NAME="${line#*$'\t'}"
}

# Boot the simulator and show it: DeviceHub of Xcode 27, Simulator before it.
ios_simulator_boot() {
  echo ">> the simulator: $IOS_SIM_NAME ($IOS_SIM_UDID)"
  xcrun simctl boot "$IOS_SIM_UDID" 2>/dev/null || true
  local dev app
  dev="$(xcode-select -p)"
  for app in "$dev/../Applications/DeviceHub.app" "$dev/Applications/Simulator.app"; do
    if [ -d "$app" ]; then open "$app"; break; fi
  done
  xcrun simctl bootstatus "$IOS_SIM_UDID" -b >/dev/null
}

# Install the app on the simulator and start it (again, when it runs).
ios_simulator_launch() {
  xcrun simctl launch --terminate-running-process "$IOS_SIM_UDID" "$IOS_BUNDLE_ID" >/dev/null
  echo ">> started $IOS_BUNDLE_ID on $IOS_SIM_NAME"
}

ios_simulator_install() {
  local app="$1"
  echo ">> installing ${app#"$ROOT_DIR"/} on $IOS_SIM_NAME"
  xcrun simctl install "$IOS_SIM_UDID" "$app"
  ios_simulator_launch
}
