#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# iOS build environment — meant to be *sourced*, on macOS only
# (internal/platform-spec.md 13.6). Unlike scripts/android/android-env.sh it
# installs nothing: Xcode, Node, pnpm and rustup are the machine's (the
# runner's image, or Homebrew on a Mac), scripts/toolchain.sh downloads a
# Linux Node and is not sourced here. It names what is missing, adds the
# Rust targets of the phone and the simulator, and fetches the prebuilt
# libwebrtc of the phone when scripts/webrtc-toolchain.sh pins one for it
# (the engine of calls, crates/messenger/rtc, links it; without an archive
# the build runs without the engine). Idempotent; safe to source every run.

IOS_SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$IOS_SCRIPTS_DIR/../.." && pwd)"

if [ "$(uname -s)" != Darwin ]; then
  echo ">> ios: a build for iOS runs on macOS with Xcode alone (this is $(uname -s)); the lab builds it on GitHub (.github/workflows/ios.yml)" >&2
  return 1 2>/dev/null || exit 1
fi
# The bash of macOS is 3.2 and cannot read the wrappers (scripts/product.sh);
# Homebrew's is first on the PATH once installed.
if [ "${BASH_VERSINFO[0]:-0}" -lt 4 ]; then
  echo ">> ios: bash ${BASH_VERSION:-?} is too old for the scripts of the repository: 'brew install bash' (the one of Homebrew comes first on the PATH)" >&2
  return 1 2>/dev/null || exit 1
fi

# Rust, Node and pnpm: the project-local ones of data/toolchains (the same
# versions as CI), which scripts/toolchain.sh installs for a Mac too. On the
# runner of the lab, whose image has its own, VEYDAN_IOS_SYSTEM_TOOLS=1 keeps
# those (and so does a machine where the script cannot fetch).
if [ -z "${VEYDAN_IOS_SYSTEM_TOOLS:-}" ] && [ -z "${CI:-}" ]; then
  source "$ROOT_DIR/scripts/toolchain.sh"
fi

ios_missing=""
for tool in xcodebuild xcrun xcodegen node pnpm rustup cargo; do
  command -v "$tool" >/dev/null 2>&1 || ios_missing="$ios_missing $tool"
done
if [ -n "$ios_missing" ]; then
  echo ">> ios: missing:$ios_missing — Xcode from the App Store (then xcode-select --install), 'brew install xcodegen node pnpm rustup'" >&2
  return 1 2>/dev/null || exit 1
fi

# swift build with the native build system (scripts/ios/swift-shim/swift):
# the default one of Xcode 27 compiles the Swift packages for the host.
case ":$PATH:" in
  *":$IOS_SCRIPTS_DIR/swift-shim:"*) ;;
  *) export PATH="$IOS_SCRIPTS_DIR/swift-shim:$PATH" ;;
esac

# Desktop flags of a dev prefix break the cross compilers of the SDKs.
unset CFLAGS CXXFLAGS CPPFLAGS C_INCLUDE_PATH CPLUS_INCLUDE_PATH
unset BINDGEN_EXTRA_CLANG_ARGS PKG_CONFIG_PATH PKG_CONFIG_LIBDIR
unset CARGO_TARGET_DIR

# The phone and its simulator on Apple silicon; the Intel simulator is not
# built (the engine's archive naming has no place for it). To the Rust of
# rust-toolchain.toml, asked from the root as android-env.sh does.
RUST_IOS_TARGETS="aarch64-apple-ios aarch64-apple-ios-sim"
for t in $RUST_IOS_TARGETS; do
  if ! (cd "$ROOT_DIR" && rustup target list --installed 2>/dev/null) | grep -qx "$t"; then
    echo ">> Adding Rust target $t ..."
    (cd "$ROOT_DIR" && rustup target add "$t")
  fi
done

# The Objective-C layer of libwebrtc (sdk/objc: RTCAudioSession and its
# categories, the codec factories, the frame buffers) is reached at run time
# by name and by selector. The linker keeps an object of a static library only
# for a symbol something names, so the categories would be left out, and a
# call would die on the first selector of one. -ObjC would keep them all, and
# with them every Objective-C object of every static library of the app — the
# copy of Tauri's Swift module each plugin carries among them (428 duplicate
# symbols). So the layer alone is force-loaded: its objects, by their names
# (RTC*.o, and every category: *+*.o), in an archive of their own beside
# libwebrtc.a, which init.sh names in the Xcode project for the phone and for
# the simulator each.
webrtc_objc_layer() {
  local dir="$WEBRTC_DIR/$WEBRTC_TAG/$1-release/lib" tmp names dup
  [ -f "$dir/libwebrtc.a" ] || return 0
  [ -f "$dir/libwebrtc-objc.a" ] && return 0
  # The classes (RTC*.o) and every category, the helpers of the layer among
  # them (NSString+StdString.o, UIDevice+RTCDevice.o): a category is an object
  # named with a plus.
  names="$(ar -t "$dir/libwebrtc.a" | grep -E '^RTC.*\.o$|\+.*\.o$' | sort)"
  if [ -z "$names" ]; then
    echo ">> webrtc: no RTC*.o in $dir/libwebrtc.a: the Objective-C layer cannot be force-loaded" >&2
    return 1
  fi
  dup="$(printf '%s\n' "$names" | uniq -d)"
  if [ -n "$dup" ]; then
    echo ">> webrtc: objects of the Objective-C layer named twice in libwebrtc.a (ar -x takes one): $(echo $dup)" >&2
    return 1
  fi
  tmp="$(mktemp -d)"
  (cd "$tmp" && printf '%s\n' "$names" | xargs ar -x "$dir/libwebrtc.a" && libtool -static -no_warning_for_no_symbols -o "$dir/libwebrtc-objc.a" ./*.o) || { rm -rf "$tmp"; return 1; }
  rm -rf "$tmp"
  echo ">> webrtc: $(printf '%s\n' "$names" | wc -l | tr -d ' ') objects of the Objective-C layer of $1 in $dir/libwebrtc-objc.a"
}

# The engine of calls: the archives of the phone and the simulator, once
# scripts/webrtc-toolchain.sh pins them (Veydan-WebRTC with build/ios.sh).
# Until then the script says so and the build goes on without the engine.
if grep -q 'ios-device-arm64' "$ROOT_DIR/scripts/webrtc-toolchain.sh"; then
  # The archives of the phone are pinned for LiveKit's release alone until
  # Veydan-WebRTC publishes ours (services/webrtc/build/ios.sh); the lab's
  # workflow names the same source.
  WEBRTC_SOURCE="${WEBRTC_SOURCE:-livekit}"
  source "$ROOT_DIR/scripts/webrtc-toolchain.sh"
  for t in ios-device-arm64 ios-simulator-arm64; do
    webrtc_toolchain "$t" && webrtc_objc_layer "$t" || echo ">> webrtc: the engine of calls will not build for $t without its archive" >&2
  done
else
  echo ">> webrtc: no archive of libwebrtc is pinned for iOS yet; the engine of calls is left out (crates/messenger-app, feature rtc)" >&2
fi

export VEYDAN_IOS_ENV=1
