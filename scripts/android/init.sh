#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# Create the Android project of a product, once (docs/platform-spec.md 13.4):
#   scripts/android/init.sh [product]     # space by default
#
# The CLI runs from the root with TAURI_APP_PATH, like every wrapper (13.2).
# What it writes into apps/<product>/gen/android is tracked in git from then
# on; set bundle.android.minSdkVersion in the product's config before: init
# is the only time the value is read. Then the callback of Gradle into the
# CLI gets the form the other products have: BuildTask.kt runs the CLI of
# the UI project (ui/node_modules/.bin/tauri) from the root, pinned to the
# product by TAURI_APP_PATH and to ui/ by TAURI_FRONTEND_PATH (generated, it
# would run `pnpm tauri` and look for any product), and rootDirRel of
# app/build.gradle.kts leads to the crate of the product.
set -euo pipefail

ANDROID_SCRIPTS_DIR="$(cd "$(dirname "$0")" && pwd)"
# Init talks to no phone.
export VEYDAN_ANDROID_NO_ADB=1
source "$ANDROID_SCRIPTS_DIR/android-env.sh"
cd "$ROOT_DIR"
source "$ROOT_DIR/scripts/product.sh"
product_env "${1:-space}"

GEN_ANDROID="$APP_DIR/gen/android"
if [ -d "$GEN_ANDROID" ]; then
  echo ">> $GEN_ANDROID exists; it is created once and then edited in git"
  exit 1
fi

# The callback of a product made before, the same for every product.
model="$(find "$ROOT_DIR"/apps/*/gen/android/buildSrc -name BuildTask.kt -print -quit 2>/dev/null || true)"
if [ -z "$model" ]; then
  echo ">> no BuildTask.kt of another product under apps/*/gen/android to take the callback from"
  exit 1
fi

tauri_cli_ready
"$TAURI_CLI" android init --ci --skip-targets-install

task="$(find "$GEN_ANDROID/buildSrc" -name BuildTask.kt -print -quit)"
if [ -z "$task" ]; then
  echo ">> BuildTask.kt not found under $GEN_ANDROID/buildSrc"
  exit 1
fi
cp "$model" "$task"
gradle="$GEN_ANDROID/app/build.gradle.kts"
sed -i -E 's|^( *rootDirRel = ).*|\1"../../../"|' "$gradle"
grep -qF 'rootDirRel = "../../../"' "$gradle" || {
  echo ">> $gradle: rootDirRel could not be set"
  exit 1
}
echo ">> $GEN_ANDROID created for $PRODUCT_ANDROID_IDENTIFIER; review and commit it"
