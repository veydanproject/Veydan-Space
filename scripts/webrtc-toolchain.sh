#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# The prebuilt libwebrtc the calls engine (crates/messenger/rtc) links:
# one archive per target from a release on GitHub, pinned by tag and by
# sha256, unpacked into data/toolchains/webrtc/<tag>/<target>/.
# Meant to be *sourced* by build-env.sh and android-env.sh (it exports
# VEYDAN_WEBRTC_DIR) or run by hand:
#
#   [WEBRTC_SOURCE=livekit|veydan] scripts/webrtc-toolchain.sh [linux-x64|android-arm64 ...]
#
# Idempotent: an archive that is there and whose hash is right is not
# fetched again; an unpacked folder of the same source is left alone. A
# target without a pinned hash is refused: the hash is the only thing that
# ties a build to the bytes the stage-0 spike was measured with
# (tmp/calls-spike/REPORT.md).
#
# Two sources carry the same archives, webrtc-<target>-release.zip of one
# layout, built from one commit of WebRTC (WEBRTC_SOURCE):
#   veydan   the releases of veydanproject/Veydan-WebRTC (services/webrtc)
#            at WEBRTC_VEYDAN_TAG — our build without the software H.264
#            (no FFmpeg, no OpenH264); the default since 1.0.10;
#   livekit  the releases of livekit/rust-sdks at WEBRTC_TAG — the archives
#            the stage-0 spike was measured with; they hold FFmpeg and
#            OpenH264, so a product is not shipped on them.
# Whichever the source, the unpacked folder is <WEBRTC_TAG>/<target>-release/:
# that is the path webrtc-sys-build 0.3.19 looks for (its WEBRTC_TAG; move
# the crate and this tag together, with new hashes). A file SOURCE in the
# folder says whose archive it is; a folder of the other source is unpacked
# again, one without the file is livekit's (unpacked before the file was).
# veydan became the default on 2026-10-08, after the engine built and its
# tests ran on our archives (linux-x64 and android-arm64 unpacked by hand).

VEYDAN_ROOT="${VEYDAN_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
WEBRTC_TAG="webrtc-89d790b"
WEBRTC_SOURCE="${WEBRTC_SOURCE:-veydan}"
WEBRTC_VEYDAN_TAG="v1.0.10"
WEBRTC_DIR="$VEYDAN_ROOT/data/toolchains/webrtc"
export VEYDAN_WEBRTC_DIR="$WEBRTC_DIR"

# sha256 of webrtc-<target>-release.zip of the release WEBRTC_VEYDAN_TAG of
# Veydan-WebRTC: the numbers of its SHA256SUMS. A target missing here is
# refused rather than taken on trust.
webrtc_veydan_sha256() {
  case "$1" in
    linux-x64)     echo "ab8a85ee303fc3a3b30c04b162f8b81cbf44c69c733faf8173f3d39d3598d658" ;;
    linux-arm64)   echo "ef777c06aff50cd0800d6dbc33f3841d02301d36e884ef23552191442e906d42" ;;
    android-arm64) echo "99785c71b842036ee25bcd52896d5519b4b41851cdf0d68918f3cf72e6693347" ;;
    win-x64)       echo "24e38264d6d005a6c78e1391c1495b00dffd98dd1ef8869d81080377ab1504c7" ;;
    win-arm64)     echo "f40cef8e24bbeba111fca4178739cba4edbcf8a2275e029d68348677cfe21c93" ;;
    mac-x64)       echo "838ec87de7dd35693970660bd2007ebb0fce31c27ac4929fd25ccfcd2f014f1f" ;;
    mac-arm64)     echo "97b206139d082da6bbf20417eec1c60c112474e05d0e6fcb5cb8db7d967dd008" ;;
    *) return 1 ;;
  esac
}

# sha256 of webrtc-<target>-release.zip of the release WEBRTC_TAG of
# livekit/rust-sdks: linux-x64 and android-arm64 from
# tmp/calls-spike/toolchains/SHA256SUMS (measured 2026-10-07), the others
# measured the same day from the same release. The names are those of
# webrtc-sys-build's webrtc_triple(): win, mac, linux, android; x64, arm64.
webrtc_livekit_sha256() {
  case "$1" in
    linux-x64)     echo "ab8a85ee303fc3a3b30c04b162f8b81cbf44c69c733faf8173f3d39d3598d658" ;;
    linux-arm64)   echo "ef777c06aff50cd0800d6dbc33f3841d02301d36e884ef23552191442e906d42" ;;
    android-arm64) echo "99785c71b842036ee25bcd52896d5519b4b41851cdf0d68918f3cf72e6693347" ;;
    win-x64)       echo "24e38264d6d005a6c78e1391c1495b00dffd98dd1ef8869d81080377ab1504c7" ;;
    win-arm64)     echo "f40cef8e24bbeba111fca4178739cba4edbcf8a2275e029d68348677cfe21c93" ;;
    mac-x64)       echo "838ec87de7dd35693970660bd2007ebb0fce31c27ac4929fd25ccfcd2f014f1f" ;;
    mac-arm64)     echo "97b206139d082da6bbf20417eec1c60c112474e05d0e6fcb5cb8db7d967dd008" ;;
    *) return 1 ;;
  esac
}

# The pinned sha256 of a target from the source, or 1.
webrtc_sha256() {
  case "$WEBRTC_SOURCE" in
    livekit) webrtc_livekit_sha256 "$1" ;;
    veydan) webrtc_veydan_sha256 "$1" ;;
    *) return 1 ;;
  esac
}

# The release the archives of the source come from: its tag, and the URL of
# the archive of a target.
webrtc_release_tag() {
  case "$WEBRTC_SOURCE" in
    livekit) echo "$WEBRTC_TAG" ;;
    veydan) echo "$WEBRTC_VEYDAN_TAG" ;;
  esac
}
webrtc_url() {
  local target="$1"
  case "$WEBRTC_SOURCE" in
    livekit) echo "https://github.com/livekit/rust-sdks/releases/download/$WEBRTC_TAG/webrtc-$target-release.zip" ;;
    veydan) echo "https://github.com/veydanproject/Veydan-WebRTC/releases/download/$WEBRTC_VEYDAN_TAG/webrtc-$target-release.zip" ;;
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
  local target="$1" sum zip dir url tmp stamp release
  case "$WEBRTC_SOURCE" in
    livekit|veydan) ;;
    *) echo ">> webrtc: WEBRTC_SOURCE is '$WEBRTC_SOURCE'; it is livekit or veydan" >&2; return 1 ;;
  esac
  if ! sum=$(webrtc_sha256 "$target"); then
    echo ">> webrtc: no pinned sha256 for $target from the source $WEBRTC_SOURCE; add it to scripts/webrtc-toolchain.sh" >&2
    return 1
  fi
  release="$(webrtc_release_tag)"
  stamp="$WEBRTC_SOURCE $release"
  dir="$WEBRTC_DIR/$WEBRTC_TAG/$target-release"
  # The folder is good when it is whole (the library is webrtc.lib on
  # Windows) and of this source; one without SOURCE was unpacked from
  # livekit's archive before the file was written.
  if { [ -f "$dir/lib/libwebrtc.a" ] || [ -f "$dir/lib/webrtc.lib" ]; } && [ -f "$dir/webrtc.ninja" ]; then
    local have
    have="$(cat "$dir/SOURCE" 2>/dev/null || echo "livekit $WEBRTC_TAG")"
    if [ "$have" = "$stamp" ]; then
      return 0
    fi
    echo ">> webrtc: $dir holds the archive of $have, not of $stamp; unpacking again" >&2
  fi
  # The archive of livekit lies beside the folder, as it always did; ours
  # under the tag of our release, so that both can be kept.
  case "$WEBRTC_SOURCE" in
    livekit) zip="$WEBRTC_DIR/$WEBRTC_TAG/webrtc-$target-release.zip" ;;
    veydan) zip="$WEBRTC_DIR/veydan-$WEBRTC_VEYDAN_TAG/webrtc-$target-release.zip" ;;
  esac
  mkdir -p "$WEBRTC_DIR/$WEBRTC_TAG" "$(dirname "$zip")"
  url="$(webrtc_url "$target")"
  if [ -f "$zip" ] && [ "$(webrtc_file_sha256 "$zip")" != "$sum" ]; then
    echo ">> webrtc: $zip has the wrong hash, fetching again" >&2
    rm -f "$zip"
  fi
  if [ ! -f "$zip" ]; then
    echo ">> Fetching libwebrtc $release ($WEBRTC_SOURCE) for $target into data/toolchains/webrtc/ (150-300 MB) ..."
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
  echo "$stamp" > "$tmp/$target-release/SOURCE"
  mv "$tmp/$target-release" "$dir"
  rm -rf "$tmp"
  echo ">> libwebrtc $release ($WEBRTC_SOURCE) for $target is in $dir"
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
