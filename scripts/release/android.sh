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
#
#   scripts/release/android.sh bundle    PRODUCT, ASSET_PREFIX, VERSION,
#     JARSIGNER, KEYTOOL, RUNNER_TEMP, ANDROID_UPLOAD_KEYSTORE (base64),
#     ANDROID_UPLOAD_KS_PASS, ANDROID_UPLOAD_KEY_ALIAS (upload): the newest
#     release app bundle of the product (the .aab Gradle writes beside the
#     APK) becomes play-bundle/<prefix>_<version>_android.aab, signed with
#     the UPLOAD key — the key Google Play accepts uploads with; the key of
#     the APK never goes to Google, and a lost upload key is replaced in
#     the Play Console. The signature is checked and its certificate must
#     be the one of the keystore. The bundle is an artifact of the run,
#     never a file of the release: the owner uploads it to the Play Console.
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
  bundle)
    : "${PRODUCT:?}" "${ASSET_PREFIX:?}" "${VERSION:?}" "${JARSIGNER:?}" "${KEYTOOL:?}" "${RUNNER_TEMP:?}"
    if [ -z "${ANDROID_UPLOAD_KEYSTORE:-}" ] || [ -z "${ANDROID_UPLOAD_KS_PASS:-}" ]; then
      fail "ANDROID_UPLOAD_KEYSTORE and ANDROID_UPLOAD_KS_PASS secrets are required"
    fi
    ALIAS="${ANDROID_UPLOAD_KEY_ALIAS:-upload}"
    KS="$RUNNER_TEMP/android-upload.keystore"
    # Gone on every way out: a wrong password or alias stops jarsigner and
    # keytool, and set -e with them.
    trap 'rm -f "$KS"' EXIT
    printf '%s' "$ANDROID_UPLOAD_KEYSTORE" | base64 -d > "$KS"
    UNSIGNED="$(find "apps/$PRODUCT/gen/android/app/build/outputs/bundle" \
      -name '*-release.aab' -printf '%T@\t%p\n' | sort -nr | sed -n '1s/^[^\t]*\t//p')"
    if [ -z "$UNSIGNED" ]; then
      rm -f "$KS"
      fail "No release app bundle (.aab) after tauri android build"
    fi
    rm -rf play-bundle && mkdir -p play-bundle
    SIGNED="play-bundle/${ASSET_PREFIX}_${VERSION}_android.aab"
    cp "$UNSIGNED" "$SIGNED"
    # jarsigner signs in place; the passwords come from the environment.
    "$JARSIGNER" -sigalg SHA256withRSA -digestalg SHA-256 \
      -keystore "$KS" -storepass:env ANDROID_UPLOAD_KS_PASS -keypass:env ANDROID_UPLOAD_KS_PASS \
      "$SIGNED" "$ALIAS" >/dev/null
    # The certificate of the keystore is the one the bundle carries. -v:
    # without it keytool -list prints "Certificate fingerprint (SHA-256): …",
    # not the "SHA256: …" line that -printcert prints.
    WANT="$("$KEYTOOL" -list -v -keystore "$KS" -storepass:env ANDROID_UPLOAD_KS_PASS -alias "$ALIAS" 2>/dev/null | sed -n 's/.*SHA256: *//p' | head -n1 || true)"
    rm -f "$KS"
    [ -n "$WANT" ] || fail "the upload keystore has no key '$ALIAS'"
    # The answer is taken whole, then searched (the trap of `symbol` above).
    VERIFIED="$("$JARSIGNER" -verify "$SIGNED")" || fail "$SIGNED: jarsigner does not verify the bundle"
    grep -q 'jar verified' <<<"$VERIFIED" || fail "$SIGNED: jarsigner does not verify the bundle"
    HAVE="$("$KEYTOOL" -printcert -jarfile "$SIGNED" | sed -n 's/.*SHA256: *//p' | head -n1)"
    [ "$HAVE" = "$WANT" ] || fail "$SIGNED is signed with $HAVE, and the upload key is $WANT"
    if [ "$(ls -A play-bundle)" != "$(basename "$SIGNED")" ]; then
      echo "::error::play-bundle holds more than the signed bundle"
      ls -A play-bundle
      exit 1
    fi
    echo ">> $SIGNED signed with the upload key $WANT"
    ;;
  *) fail "usage: scripts/release/android.sh symbol|sign|bundle" ;;
esac
