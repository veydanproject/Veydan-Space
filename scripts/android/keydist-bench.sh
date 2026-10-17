#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# The benchmark of the key distribution (crates/keydist, tools feature) on
# the test phone: build for aarch64-linux-android with the NDK of
# scripts/android/android-env.sh, push to /data/local/tmp, run, pull the
# JSON. Nothing of the apps is touched; the phone keeps what it has.
#
#   scripts/android/keydist-bench.sh [-s <serial>] [-- <args of keydist-bench>]
#
# Run from a fresh shell: not after `source scripts/build-env.sh` (its host
# flags break the Android link, as for scripts/android/apk.sh).
# Output: tmp/keydist/phone.json (and the stdout of the run).
set -euo pipefail
cd "$(dirname "$0")/../.."

serial=
while [ $# -gt 0 ]; do
  case "$1" in
    -s) serial=$2; shift 2 ;;
    --) shift; break ;;
    *) echo "usage: $0 [-s <serial>] [-- <args>]" >&2; exit 2 ;;
  esac
done

# shellcheck source=android-env.sh
source scripts/android/android-env.sh >/dev/null
ndk="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin"
cc="$ndk/aarch64-linux-android26-clang"
[ -x "$cc" ] || { echo "::error::no NDK clang at $cc"; exit 1; }

export CARGO_TARGET_DIR=data/target-android/bench
CC_aarch64_linux_android="$cc" AR_aarch64_linux_android="$ndk/llvm-ar" \
CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$cc" \
  cargo build -p keydist --release --locked --target aarch64-linux-android \
    --features tools --bin keydist-bench
bin="$CARGO_TARGET_DIR/aarch64-linux-android/release/keydist-bench"

adb=(adb)
[ -z "$serial" ] || adb+=(-s "$serial")
n=$("${adb[@]}" devices | awk 'NR>1 && $2=="device"' | wc -l)
[ "$n" -ge 1 ] || { echo "::error::no phone in 'adb devices' (pair it first; see memory android-adb-testing)"; exit 1; }
[ "$n" -eq 1 ] || [ -n "$serial" ] || { echo "::error::$n devices; pass -s <serial>"; "${adb[@]}" devices; exit 1; }

mkdir -p tmp/keydist
"${adb[@]}" push "$bin" /data/local/tmp/keydist-bench >/dev/null
"${adb[@]}" shell chmod 755 /data/local/tmp/keydist-bench
"${adb[@]}" shell /data/local/tmp/keydist-bench --out /data/local/tmp/keydist-bench.json "$@"
"${adb[@]}" pull /data/local/tmp/keydist-bench.json tmp/keydist/phone.json >/dev/null
"${adb[@]}" shell rm -f /data/local/tmp/keydist-bench /data/local/tmp/keydist-bench.json
echo "keydist-bench: tmp/keydist/phone.json"
