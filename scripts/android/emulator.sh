#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# An Android emulator of this machine, made once and started when it is not
# running (internal/platform-spec.md 13.4):
#   scripts/android/emulator.sh [fold|phone|<avd>]     # fold when none is given
#   make android-emulator [DEVICE=fold|phone|<avd>]
# fold  = veydan-fold, a Pixel 10 Pro Fold (two postures: folded, unfolded)
# phone = veydan-phone, a Pixel of one screen
# The emulator, the system image (Android 16, Google APIs, the CPU of this
# machine) and the virtual devices live in data/toolchains (android-sdk,
# android-avd on a Mac); the emulator's log in tmp/android/. The serial of
# the running emulator ("emulator-5554") is the one line on stdout, the rest
# goes to stderr: `serial="$(scripts/android/emulator.sh fold)"`.
set -euo pipefail

ANDROID_SCRIPTS_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$ANDROID_SCRIPTS_DIR/android-env.sh" >&2
cd "$ROOT_DIR"

ANDROID_API=36
case "$(uname -m)" in arm64 | aarch64) abi=arm64-v8a ;; *) abi=x86_64 ;; esac
IMAGE="system-images;android-$ANDROID_API;google_apis;$abi"

case "${1:-fold}" in
  fold) avd=veydan-fold; profile=pixel_10_pro_fold ;;
  phone) avd=veydan-phone; profile=pixel_9 ;;
  *) avd="$1"; profile="" ;;
esac

say() { echo ">> $*" >&2; }

if [ ! -x "$SDK_DIR/emulator/emulator" ] || [ ! -d "$SDK_DIR/system-images/android-$ANDROID_API/google_apis/$abi" ]; then
  say "installing the emulator and $IMAGE into data/toolchains/android-sdk (about 2 GB) ..."
  yes | sdkmanager --sdk_root="$SDK_DIR" --licenses >/dev/null 2>&1 || true
  sdkmanager --sdk_root="$SDK_DIR" --install emulator "$IMAGE" >&2
fi

avd_home="${ANDROID_AVD_HOME:-$HOME/.android/avd}"
if [ ! -f "$avd_home/$avd.ini" ]; then
  if [ -z "$profile" ]; then
    say "no virtual device $avd (made here: fold, phone; avdmanager list avd lists the others)"
    exit 1
  fi
  say "making the virtual device $avd ($profile, $IMAGE)"
  echo no | avdmanager create avd -n "$avd" -k "$IMAGE" -d "$profile" >&2
  # A hardware keyboard (typing from the Mac) and room for the app's data.
  config="$avd_home/$avd.avd/config.ini"
  sed -i '' -e '/^hw.keyboard=/d' -e '/^disk.dataPartition.size=/d' "$config" 2>/dev/null ||
    sed -i -e '/^hw.keyboard=/d' -e '/^disk.dataPartition.size=/d' "$config"
  printf 'hw.keyboard=yes\ndisk.dataPartition.size=8G\n' >>"$config"
fi

# The serial of the running emulator of this virtual device, if any.
running() {
  local s
  for s in $(adb devices 2>/dev/null | awk '/^emulator-[0-9]+\tdevice$/{print $1}'); do
    if [ "$(adb -s "$s" emu avd name 2>/dev/null | head -1 | tr -d '\r')" = "$avd" ]; then
      echo "$s"
      return 0
    fi
  done
  return 1
}

adb start-server >/dev/null 2>&1 || true
if serial="$(running)"; then
  say "$avd is running: $serial"
  echo "$serial"
  exit 0
fi

mkdir -p tmp/android
log="tmp/android/emulator-$avd.log"
say "starting $avd (log: $log)"
# Detached from this script: the emulator outlives it, and Ctrl-C of a dev
# session does not close it. Closed from its window, or `adb -s <serial> emu kill`.
nohup "$SDK_DIR/emulator/emulator" -avd "$avd" -no-boot-anim -netdelay none -netspeed full \
  >"$log" 2>&1 </dev/null &
emu=$!
disown "$emu" 2>/dev/null || true

for _ in $(seq 1 180); do
  if serial="$(running)"; then break; fi
  kill -0 "$emu" 2>/dev/null || { say "the emulator stopped:"; tail -5 "$log" >&2; exit 1; }
  sleep 1
done
[ -n "${serial:-}" ] || { say "$avd did not show up in adb in 3 minutes ($log)"; exit 1; }
say "waiting for Android to boot on $serial ..."
for _ in $(seq 1 300); do
  [ "$(adb -s "$serial" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = 1 ] && break
  sleep 1
done
say "$avd is up: $serial"
echo "$serial"
