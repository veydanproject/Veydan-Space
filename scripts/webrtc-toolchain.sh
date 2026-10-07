#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# The prebuilt libwebrtc the calls engine (crates/messenger/rtc) links:
# one archive per target from the releases of livekit/rust-sdks, pinned by
# tag and by sha256, unpacked into data/toolchains/webrtc/<tag>/<target>/.
# Meant to be *sourced* by build-env.sh and android-env.sh (it exports
# VEYDAN_WEBRTC_DIR) or run by hand:
#
#   scripts/webrtc-toolchain.sh [linux-x64|android-arm64 ...]
#
# Idempotent: an archive that is there and whose hash is right is not
# fetched again; an unpacked folder is left alone. A target without a
# pinned hash is refused: the hash is the only thing that ties a build to
# the bytes the stage-0 spike was measured with (tmp/calls-spike/REPORT.md).
#
# The tag is the one webrtc-sys-build 0.3.19 names (WEBRTC_TAG); move the
# crate and this tag together, with new hashes.

VEYDAN_ROOT="${VEYDAN_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
WEBRTC_TAG="webrtc-89d790b"
WEBRTC_DIR="$VEYDAN_ROOT/data/toolchains/webrtc"
export VEYDAN_WEBRTC_DIR="$WEBRTC_DIR"

# sha256 of webrtc-<target>-release.zip of the tag: linux-x64 and
# android-arm64 from tmp/calls-spike/toolchains/SHA256SUMS (measured
# 2026-10-07), the others measured the same day from the same release.
# The names are those of webrtc-sys-build's webrtc_triple(): win, mac,
# linux, android; x64, arm64.
webrtc_sha256() {
  case "$1" in
    linux-x64)     echo "b167adad5291cea0e4d66a0454d9d52d2ad714e6b0ed70f4410317d3ebde70c5" ;;
    linux-arm64)   echo "f716b10eade18dd11b03b9f93b50cee2ca2eec73b975a63f2ae9ea2a35706420" ;;
    android-arm64) echo "81880a4cda27474497ac5277e841ae110ede73e81d0e767606de4bdace8ac82c" ;;
    win-x64)       echo "5c2349c960bae4f06f71c102f58552d0d09c912a142811acfcadfb7c883cbf58" ;;
    mac-x64)       echo "04eea79951eaefc1054099464f949f6dcfab9bc7460c16004811abc4f9ec15d4" ;;
    mac-arm64)     echo "9f25fea48588deac18d68e120d18b33af7ef10f921f74b7c57f66b642262c348" ;;
    *) return 1 ;;
  esac
}

# sha256 of a file, where the system has sha256sum (Linux, Git Bash) or
# only shasum (macOS).
webrtc_file_sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# Unpacks a zip into a folder with what the system has: unzip, or 7z
# (the Windows runners of CI), or a tar that reads zip (bsdtar of macOS
# and Windows).
webrtc_unzip() {
  if command -v unzip >/dev/null 2>&1; then
    unzip -q "$1" -d "$2"
  elif command -v 7z >/dev/null 2>&1; then
    7z x -bso0 -bsp0 -o"$2" "$1"
  else
    tar -xf "$1" -C "$2"
  fi
}

# Fetches, checks and unpacks one target. Returns 1 on a refusal, so that a
# sourced caller goes on without the engine rather than losing its shell.
webrtc_toolchain() {
  local target="$1" sum zip dir url tmp
  if ! sum=$(webrtc_sha256 "$target"); then
    echo ">> webrtc: no pinned sha256 for $target; add it to scripts/webrtc-toolchain.sh" >&2
    return 1
  fi
  dir="$WEBRTC_DIR/$WEBRTC_TAG/$target-release"
  if [ -f "$dir/lib/libwebrtc.a" ] && [ -f "$dir/webrtc.ninja" ]; then
    return 0
  fi
  mkdir -p "$WEBRTC_DIR/$WEBRTC_TAG"
  zip="$WEBRTC_DIR/$WEBRTC_TAG/webrtc-$target-release.zip"
  url="https://github.com/livekit/rust-sdks/releases/download/$WEBRTC_TAG/webrtc-$target-release.zip"
  if [ -f "$zip" ] && [ "$(webrtc_file_sha256 "$zip")" != "$sum" ]; then
    echo ">> webrtc: $zip has the wrong hash, fetching again" >&2
    rm -f "$zip"
  fi
  if [ ! -f "$zip" ]; then
    echo ">> Fetching libwebrtc $WEBRTC_TAG for $target into data/toolchains/webrtc/ (150-300 MB) ..."
    if ! curl -sSfL --proto '=https' --tlsv1.2 "$url" -o "$zip.part"; then
      rm -f "$zip.part"
      echo ">> webrtc: cannot fetch $url" >&2
      return 1
    fi
    mv "$zip.part" "$zip"
  fi
  if [ "$(webrtc_file_sha256 "$zip")" != "$sum" ]; then
    echo ">> webrtc: $zip does not match the pinned sha256 $sum; not unpacked" >&2
    return 1
  fi
  # The archive holds one folder, <target>-release/, with include/, lib/
  # and webrtc.ninja. Unpacked beside the zip under a scratch name and moved
  # into place whole: a half-unpacked folder is never taken for a good one.
  tmp="$WEBRTC_DIR/$WEBRTC_TAG/.unpack-$target"
  rm -rf "$tmp" "$dir"
  mkdir -p "$tmp"
  if ! webrtc_unzip "$zip" "$tmp"; then
    rm -rf "$tmp"
    echo ">> webrtc: cannot unpack $zip" >&2
    return 1
  fi
  mv "$tmp/$target-release" "$dir"
  rm -rf "$tmp"
  echo ">> libwebrtc $WEBRTC_TAG for $target is in $dir"
}

# Run by hand: the targets named, or linux-x64.
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  set -euo pipefail
  rc=0
  for target in "${@:-linux-x64}"; do
    webrtc_toolchain "$target" || rc=1
  done
  exit $rc
fi
