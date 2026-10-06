#!/usr/bin/env bash
# Standalone Android APK of a product (internal/platform-spec.md 13.4):
#   scripts/android/apk.sh [product] test      # debug-signed, installed on the phone
#   scripts/android/apk.sh [product] build     # debug-signed, not installed
#   scripts/android/apk.sh [product] release   # release-signed
# The product is space when none is given. Arguments after the mode go to
# `tauri android build` (for example `--target aarch64`).
#
# Each product builds into data/target-android/<product>: the build script of
# tauri writes into the gen/android of the product it runs for and would not
# rerun in a shared target directory. Never run two Android builds at once.
set -euo pipefail

case "${1:-}" in
  test | build | release) PRODUCT_ARG=space ;;
  *) PRODUCT_ARG="${1:-}"; shift || true ;;
esac
MODE="${1:-}"
if [ "$MODE" != "test" ] && [ "$MODE" != "build" ] && [ "$MODE" != "release" ]; then
  echo ">> usage: apk.sh [product] test|build|release [tauri android build arguments]"
  exit 1
fi
shift

ANDROID_SCRIPTS_DIR="$(cd "$(dirname "$0")" && pwd)"
# Only `test` talks to a phone.
if [ "$MODE" != test ]; then
  export VEYDAN_ANDROID_NO_ADB=1
fi
source "$ANDROID_SCRIPTS_DIR/android-env.sh"
cd "$ROOT_DIR"
source "$ROOT_DIR/scripts/product.sh"
product_env "$PRODUCT_ARG"
GEN_ANDROID="$APP_DIR/gen/android"
if [ ! -d "$GEN_ANDROID" ]; then
  echo ">> $GEN_ANDROID is missing: create it once with scripts/android/init.sh $PRODUCT"
  exit 1
fi
export CARGO_TARGET_DIR="$ROOT_DIR/data/target-android/$PRODUCT"

newest_apk() {
  find "$GEN_ANDROID/app/build/outputs/apk" \
    -name "$1" -printf "%T@\t%p\n" 2>/dev/null | sort -nr | cut -f2- | head -n1
}

ensure_debug_keystore() {
  local ks="$HOME/.android/debug.keystore"
  if [ -f "$ks" ]; then
    echo "$ks"
    return
  fi
  ks="$ROOT_DIR/data/toolchains/android-debug.keystore"
  if [ ! -f "$ks" ]; then
    mkdir -p "$(dirname "$ks")"
    keytool -genkeypair -keystore "$ks" -alias androiddebugkey \
      -keyalg RSA -keysize 2048 -validity 10000 \
      -storepass android -keypass android \
      -dname "CN=Android Debug,O=Android,C=US"
  fi
  echo "$ks"
}

ensure_release_keystore() {
  local ks="$ROOT_DIR/data/toolchains/android-release.keystore"
  local envf="$ROOT_DIR/data/toolchains/android-release.env"
  mkdir -p "$ROOT_DIR/data/toolchains"
  if [ ! -f "$ks" ] || [ ! -f "$envf" ]; then
    local pass
    pass="$(openssl rand -hex 16)"
    keytool -genkeypair -keystore "$ks" -alias veydan \
      -keyalg RSA -keysize 2048 -validity 10000 \
      -storepass "$pass" -keypass "$pass" \
      -dname "CN=Veydan,O=Veydan Project,C=US"
    cat > "$envf" <<EOT
ANDROID_RELEASE_KS_PASS=$pass
ANDROID_RELEASE_KEY_ALIAS=veydan
EOT
    echo ">> Created release keystore $ks (keep data/toolchains/android-release.* )"
  fi
  echo "$ks"
}

echo ">> Building standalone APK of $PRODUCT ($MODE) into data/target-android/$PRODUCT"
tauri_cli_ready
"$TAURI_CLI" android build "$@"

unsigned="$(newest_apk '*-unsigned.apk')"
if [ -z "$unsigned" ]; then
  echo ">> No unsigned APK. tauri android build failed?"
  exit 1
fi

signer="$(ls -1 "$ANDROID_HOME"/build-tools/*/apksigner 2>/dev/null | tail -n1)"
if [ -z "$signer" ]; then
  echo ">> apksigner not found"
  exit 1
fi

if [ "$MODE" = "release" ]; then
  ks="$(ensure_release_keystore)"
  # shellcheck disable=SC1091
  source "$ROOT_DIR/data/toolchains/android-release.env"
  alias="${ANDROID_RELEASE_KEY_ALIAS:-veydan}"
  storepass="${ANDROID_RELEASE_KS_PASS:?missing ANDROID_RELEASE_KS_PASS}"
  keypass="$storepass"
  signed="${unsigned%-unsigned.apk}-release.apk"
else
  ks="$(ensure_debug_keystore)"
  alias="androiddebugkey"
  storepass="android"
  keypass="android"
  signed="${unsigned%-unsigned.apk}-test.apk"
fi

echo ">> Signing $unsigned -> $signed"
"$signer" sign --ks "$ks" --ks-key-alias "$alias" \
  --ks-pass "pass:$storepass" --key-pass "pass:$keypass" \
  --out "$signed" "$unsigned"

if [ "$MODE" = "test" ]; then
  # Never install over an app signed by another key: see check-signing.sh.
  ANDROID_PACKAGE="$PRODUCT_ANDROID_IDENTIFIER" bash "$ANDROID_SCRIPTS_DIR/check-signing.sh" "$signed"
  echo ">> Installing $signed"
  adb install -r "$signed"
else
  echo ">> APK: $signed"
fi
