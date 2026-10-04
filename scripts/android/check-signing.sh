#!/usr/bin/env bash
# Is the APK signed by the same key as the app on the phone?
#
#   scripts/android/check-signing.sh <apk>
#
# Android installs over an app only when the signatures match. When they do
# not, the way out that `adb` suggests is to uninstall first, and that wipes
# the app's data: the vault, the notes, the messenger keys. So a mismatch
# stops here, and nothing in these scripts ever uninstalls.
#
# Exit codes: 0 same key or the app is not installed, 1 another key,
# 2 the check itself could not be done.
set -euo pipefail

APK="${1:-}"
PACKAGE="${ANDROID_PACKAGE:-net.veydan.mobile}"

if [ -z "$APK" ] || [ ! -f "$APK" ]; then
  echo ">> usage: check-signing.sh <apk>" >&2
  exit 2
fi

ANDROID_SCRIPTS_DIR="$(cd "$(dirname "$0")" && pwd)"
if [ -z "${ANDROID_HOME:-}" ]; then
  source "$ANDROID_SCRIPTS_DIR/android-env.sh"
fi

signer="$(ls -1 "$ANDROID_HOME"/build-tools/*/apksigner 2>/dev/null | tail -n1)"
if [ -z "$signer" ]; then
  echo ">> apksigner not found" >&2
  exit 2
fi

# The whole answer is taken first and cut afterwards: cutting a pipe short
# would end apksigner with an error, and the script with it.
digest() {
  local out
  out="$("$signer" verify --print-certs "$1" 2>/dev/null || true)"
  sed -n 's/^Signer #1 certificate SHA-256 digest: //p' <<<"$out"
}

new="$(digest "$APK")"
if [ -z "$new" ]; then
  echo ">> $APK is not signed" >&2
  exit 2
fi

# "No answer from the phone" must not pass for "the app is not installed".
if [ "$(adb get-state 2>/dev/null | tr -d '\r' || true)" != "device" ]; then
  echo ">> no phone on adb (or more than one): cannot compare signatures" >&2
  exit 2
fi
packages="$(adb shell pm list packages "$PACKAGE" 2>/dev/null | tr -d '\r' || true)"
if [ -z "$packages" ] && ! adb shell true >/dev/null 2>&1; then
  echo ">> the phone does not answer: cannot compare signatures" >&2
  exit 2
fi

# An app may be several files (splits); base.apk carries the signature.
paths="$(adb shell pm path "$PACKAGE" 2>/dev/null | tr -d '\r' || true)"
installed_path="$(sed -n 's/^package://p' <<<"$paths" | sed -n '1p')"
if [ -z "$installed_path" ] && grep -qx "package:$PACKAGE" <<<"$packages"; then
  echo ">> $PACKAGE is on the phone, but its file cannot be found" >&2
  exit 2
fi
if [ -z "$installed_path" ]; then
  echo ">> $PACKAGE is not on the phone yet: nothing to compare with"
  exit 0
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
if ! adb pull "$installed_path" "$tmp/installed.apk" >/dev/null 2>&1; then
  echo ">> cannot read the installed app from the phone" >&2
  exit 2
fi
old="$(digest "$tmp/installed.apk")"
if [ -z "$old" ]; then
  echo ">> cannot read the signature of the installed app" >&2
  exit 2
fi

if [ "$old" = "$new" ]; then
  echo ">> Same signing key as the app on the phone (${new:0:16}…)"
  exit 0
fi

cat >&2 <<EOT
>> STOP: the app on the phone is signed by another key.
>>   on the phone: $old
>>   this build:   $new
>> Installing over it is impossible, and removing the app first would wipe
>> its data, the messenger keys included. Nothing was installed.
>> Sign this build with the key the phone's app was signed with.
EOT
exit 1
