#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# Create the Xcode project of a product, once (internal/platform-spec.md 13.6):
#   scripts/ios/init.sh [product]     # space by default
#
# The CLI runs from the root with TAURI_APP_PATH, like every wrapper (13.2),
# and writes apps/<product>/gen/apple: the XcodeGen spec (project.yml), the
# project it generates, the sources of the app, the icons from icons/ios.
# Then two things are put the way the Android project has them
# (scripts/android/init.sh):
#   - the build phase of Xcode that compiles the Rust crate calls the CLI
#     of the UI project by a path relative to the project
#     (ui/node_modules/.bin/tauri), not the absolute path of whatever ran
#     init: a project that names a folder of one machine builds nowhere
#     else, and a public snapshot refuses a local path (scripts/publish.mjs);
#   - gen/apple/.gitignore leaves out what a build writes.
# What it writes is reviewed and then tracked in git. The lab on GitHub
# (.github/workflows/ios.yml) runs it on every build while gen/apple is not
# tracked yet, and uploads the result for the review.
set -euo pipefail

IOS_SCRIPTS_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$IOS_SCRIPTS_DIR/env.sh"
cd "$ROOT_DIR"
source "$ROOT_DIR/scripts/product.sh"
product_env "${1:-space}"

GEN_APPLE="$APP_DIR/gen/apple"
if [ -d "$GEN_APPLE" ]; then
  echo ">> $GEN_APPLE exists; it is created once and then edited in git"
  exit 1
fi

tauri_cli_ready
# --ci: no questions; the targets of Rust are env.sh's. Without an Apple
# team the CLI warns and leaves DEVELOPMENT_TEAM unset: a build with
# --no-sign needs none, a signed one takes APPLE_DEVELOPMENT_TEAM (13.6).
"$TAURI_CLI" ios init --ci --skip-targets-install

spec="$GEN_APPLE/project.yml"
[ -f "$spec" ] || { echo ">> $spec not found: tauri ios init wrote something else"; ls -la "$GEN_APPLE"; exit 1; }

# The build phase: `<binary> <args> ios xcode-script …` → the CLI of the UI
# project, from the project's folder (gen/apple → the root is four levels up),
# unquoted: a YAML value that opens with a quote ends with it, and the flags
# after `xcode-script` stay as generated (the paths of a checkout hold no space).
if ! grep -q ' ios xcode-script ' "$spec"; then
  echo ">> $spec: no build phase that runs 'ios xcode-script'; the template changed — see the file:"
  cat "$spec"
  exit 1
fi
perl -pi -e 's|^(\s*-?\s*script:\s*).*?\bios xcode-script\b|$1\$SRCROOT/../../../../ui/node_modules/.bin/tauri ios xcode-script|' "$spec"
grep -qF 'ui/node_modules/.bin/tauri ios xcode-script' "$spec" || { echo ">> $spec: the build phase could not be pointed at ui/node_modules/.bin/tauri"; exit 1; }

# The Objective-C layer of libwebrtc, force-loaded (env.sh, webrtc_objc_layer:
# why not -ObjC), for the phone and for the simulator each, by a path of the
# checkout: the archives lie under data/toolchains/webrtc of the root, four
# levels above the project. A product without the engine (no archive) gets
# no flag. The frameworks come from bundle.iOS.frameworks of the config.
if [ -f "$WEBRTC_DIR/$WEBRTC_TAG/ios-device-arm64-release/lib/libwebrtc-objc.a" ]; then
  OBJC_LAYER="\$(SRCROOT)/../../../../data/toolchains/webrtc/$WEBRTC_TAG" perl -pi -e 's|^(\s*)ENABLE_BITCODE: false$|$1ENABLE_BITCODE: false\n$1OTHER_LDFLAGS[sdk=iphoneos*]: \$(inherited) -force_load $ENV{OBJC_LAYER}/ios-device-arm64-release/lib/libwebrtc-objc.a\n$1OTHER_LDFLAGS[sdk=iphonesimulator*]: \$(inherited) -force_load $ENV{OBJC_LAYER}/ios-simulator-arm64-release/lib/libwebrtc-objc.a|' "$spec"
  grep -qF 'libwebrtc-objc.a' "$spec" || { echo ">> $spec: OTHER_LDFLAGS could not be set (no ENABLE_BITCODE line)"; exit 1; }
fi

# What a build writes stays out of git (the template lists xcuserdata/,
# build/ and Externals/; the CLI's own folder and CocoaPods' are added).
for line in '.tauri/' 'Pods/' 'build/' 'Externals/' 'xcuserdata/'; do
  grep -qxF "$line" "$GEN_APPLE/.gitignore" 2>/dev/null || echo "$line" >> "$GEN_APPLE/.gitignore"
done

# The icons of the app: the CLI fills the icon set of the project with its
# own, not the product's (apps/<product>/icons/ios, which tauri icon wrote
# and scripts/icons-ios.mjs made opaque: the App Store takes no icon with an
# alpha channel). The names are the same.
if ls "$APP_DIR/icons/ios/"AppIcon-*.png >/dev/null 2>&1; then
  cp "$APP_DIR/icons/ios/"AppIcon-*.png "$GEN_APPLE/Assets.xcassets/AppIcon.appiconset/"
  echo ">> the icons of $PRODUCT ($(ls "$APP_DIR/icons/ios/"AppIcon-*.png | wc -l | tr -d ' ')) are in the project"
else
  echo ">> $APP_DIR/icons/ios holds no AppIcon-*.png: the project keeps the CLI's own icons; run scripts/icons.sh $PRODUCT" >&2
fi

# The app's strings of the push plugin: iOS shows an alert push that names
# a key (`loc-key` push.new, services/push/spec/protocol.md) in the words
# the app's own bundle has for it, and the key itself when it has none; a
# package linked as an archive brings no bundle of its own. The .lproj
# folders go beside Info.plist of the app's target, a folder of the spec's
# sources, where xcodegen makes them the app's Localizable.strings.
PUSH_STRINGS="$ROOT_DIR/crates/tauri-plugin-veydan-push/ios/AppResources"
if grep -q 'tauri-plugin-veydan-push' "$APP_DIR/Cargo.toml"; then
  app_target="$(find "$GEN_APPLE" -mindepth 1 -maxdepth 1 -type d -name '*_iOS' -print -quit)"
  if [ -z "$app_target" ] || [ ! -f "$app_target/Info.plist" ]; then
    echo ">> $GEN_APPLE: no <app>_iOS folder with Info.plist; the template changed — see the folder:"
    ls -la "$GEN_APPLE"
    exit 1
  fi
  cp -R "$PUSH_STRINGS/"*.lproj "$app_target/"
  if ! grep -qE "^[[:space:]]*-[[:space:]]*path:[[:space:]]*$(basename "$app_target")[[:space:]]*$" "$spec"; then
    echo ">> warning: $spec does not name $(basename "$app_target") as a folder of sources; a push would read 'push.new' until the strings are in the app" >&2
  fi
  echo ">> the strings of pushes ($(ls -d "$PUSH_STRINGS/"*.lproj | wc -l | tr -d ' ') languages) are in $(basename "$app_target")"
fi

# The project from the spec again, with the phase as patched.
(cd "$GEN_APPLE" && xcodegen generate --spec project.yml --quiet)

# A path of this machine in the project would build nowhere else and would
# stop the snapshot of the product (scripts/publish.mjs, 4d).
if local_paths="$(grep -rnE '/Users/[^/ ]+/|/home/[^/ ]+/|\.cargo/registry|/private/var/' "$GEN_APPLE" --exclude-dir=build --exclude-dir=.tauri --exclude-dir=Externals 2>/dev/null)" && [ -n "$local_paths" ]; then
  echo ">> $GEN_APPLE names folders of this machine; fix the template or the patch above:"
  echo "$local_paths" | head -n 20
  exit 1
fi
echo ">> $GEN_APPLE created for $PRODUCT_IOS_IDENTIFIER; review and commit it"
