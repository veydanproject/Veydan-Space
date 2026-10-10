#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# The two steps after `tauri android build` of the release workflow of
# Gitea (veydanproject/release; docs/ci-cd.md, "Releases"), kept here so
# that the tests of the monorepo run them (scripts/tests/release.tests.mjs):
#
#   scripts/release/android.sh symbol    PRODUCT, LIB_NAME, NDK_LLVM_BIN (the
#     bin/ of the NDK's llvm): the push handler of the messenger is called by
#     name from Java; a library that lost the symbol builds and fails on the
#     phone (14.3, step 3). The list of llvm-nm is taken whole and searched
#     afterwards: piped into `grep -q` under pipefail, the check failed the
#     first alphas on a library that had the symbol (grep leaves at the
#     match, llvm-nm fails on the closed pipe, and its status became the answer).
#
#   scripts/release/android.sh sign      PRODUCT, ASSET_PREFIX, VERSION,
#     APKSIGNER, RUNNER_TEMP, ANDROID_RELEASE_KEYSTORE (base64),
#     ANDROID_RELEASE_KS_PASS, ANDROID_RELEASE_KEY_ALIAS (veydan): the newest
#     unsigned APK of the product becomes release-assets/<prefix>_<version>_android.apk,
#     signed with the release key. The passwords reach apksigner through
#     the environment (env:NAME), never through a command line; the
#     keystore lives for this step; without the v4 scheme (its .idsig was
#     once published next to the APK). The folder holds the signed APK and
#     nothing else at the end.
set -euo pipefail

fail() {
  echo "::error::$*"
  exit 1
}

case "${1:-}" in
  symbol)
    : "${PRODUCT:?}" "${LIB_NAME:?}" "${NDK_LLVM_BIN:?}"
    LIB="data/target-android/$PRODUCT/aarch64-linux-android/release/lib$LIB_NAME.so"
    NM="$NDK_LLVM_BIN/llvm-nm"
    SYMBOL=Java_net_veydan_push_Core_describe
    if ! SYMBOLS="$("$NM" -D "$LIB")"; then
      fail "llvm-nm could not list the symbols of $LIB"
    fi
    if ! grep " T $SYMBOL\$" <<<"$SYMBOLS"; then
      echo "::error::$LIB does not export $SYMBOL"
      echo "The Java_ symbols it exports:"
      grep ' Java_' <<<"$SYMBOLS" || echo "  (none)"
      exit 1
    fi
    ;;
  sign)
    : "${PRODUCT:?}" "${ASSET_PREFIX:?}" "${VERSION:?}" "${APKSIGNER:?}" "${RUNNER_TEMP:?}"
    if [ -z "${ANDROID_RELEASE_KEYSTORE:-}" ] || [ -z "${ANDROID_RELEASE_KS_PASS:-}" ]; then
      fail "ANDROID_RELEASE_KEYSTORE and ANDROID_RELEASE_KS_PASS secrets are required"
    fi
    ALIAS="${ANDROID_RELEASE_KEY_ALIAS:-veydan}"
    KS="$RUNNER_TEMP/android-release.keystore"
    printf '%s' "$ANDROID_RELEASE_KEYSTORE" | base64 -d > "$KS"
    UNSIGNED="$(find "apps/$PRODUCT/gen/android/app/build/outputs/apk" \
      -name '*-unsigned.apk' -printf '%T@\t%p\n' | sort -nr | sed -n '1s/^[^\t]*\t//p')"
    if [ -z "$UNSIGNED" ]; then
      rm -f "$KS"
      fail "No unsigned APK after tauri android build"
    fi
    rm -rf release-assets && mkdir -p release-assets
    SIGNED="release-assets/${ASSET_PREFIX}_${VERSION}_android.apk"
    "$APKSIGNER" sign --ks "$KS" --ks-key-alias "$ALIAS" \
      --ks-pass env:ANDROID_RELEASE_KS_PASS --key-pass env:ANDROID_RELEASE_KS_PASS \
      --v4-signing-enabled false \
      --out "$SIGNED" "$UNSIGNED"
    rm -f "$KS"
    "$APKSIGNER" verify "$SIGNED"
    if [ "$(ls -A release-assets)" != "$(basename "$SIGNED")" ]; then
      echo "::error::release-assets holds more than the signed APK"
      ls -A release-assets
      exit 1
    fi
    ;;
  *) fail "usage: scripts/release/android.sh symbol|sign" ;;
esac
