#!/usr/bin/env bash
# The key of a build that goes to the phone — meant to be *sourced*, after
# android-env.sh and product.sh.
#
# Android installs over an app only an APK signed by the same key, and the
# way out adb offers, removing the app first, wipes its data. An app that
# came from a release (or from `apk.sh release`) is signed by the key of the
# releases, data/toolchains/android-release.*; a build for the phone
# (`apk.sh test`, dev.sh) is signed by that key then, and by the debug key
# in every other case.

RELEASE_KEYSTORE="$ROOT_DIR/data/toolchains/android-release.keystore"
RELEASE_KEYSTORE_ENV="$ROOT_DIR/data/toolchains/android-release.env"

# The SHA-256 of the certificate of the key of the releases, as apksigner
# prints it. In a subshell: the password stays out of the caller.
release_key_digest() (
  # shellcheck disable=SC1090
  source "$RELEASE_KEYSTORE_ENV"
  export ANDROID_RELEASE_KS_PASS
  keytool -exportcert -keystore "$RELEASE_KEYSTORE" \
    -alias "${ANDROID_RELEASE_KEY_ALIAS:-veydan}" \
    -storepass:env ANDROID_RELEASE_KS_PASS 2>/dev/null | sha256sum | cut -d' ' -f1
)

# Is the app of the product on the phone signed by the key of the releases
# of this machine? No as well when there is no such key here, no app there
# or no phone to ask.
phone_has_release_key() {
  [ -f "$RELEASE_KEYSTORE" ] && [ -f "$RELEASE_KEYSTORE_ENV" ] || return 1
  local installed
  installed="$(ANDROID_PACKAGE="$PRODUCT_ANDROID_IDENTIFIER" \
    bash "$ANDROID_SCRIPTS_DIR/check-signing.sh" --installed 2>/dev/null)" || return 1
  [ -n "$installed" ] && [ "$installed" = "$(release_key_digest)" ]
}
