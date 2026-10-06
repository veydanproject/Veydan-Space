#!/usr/bin/env bash
# Android build environment — meant to be *sourced*.
#
# Installs JDK + Android SDK/NDK into <repo>/data/toolchains/ (git-ignored) on top
# of the shared Rust/Node toolchain, so the host system stays untouched.
# Idempotent; safe to source on every run.

ANDROID_SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$ANDROID_SCRIPTS_DIR/../.." && pwd)"

# Shared Rust + Node + pnpm (installs into data/toolchains/ on first run)
source "$ROOT_DIR/scripts/toolchain.sh"

# Desktop data/dev-prefix flags (CFLAGS / pkg-config) break NDK clang.
unset CFLAGS CXXFLAGS CPPFLAGS C_INCLUDE_PATH CPLUS_INCLUDE_PATH
unset BINDGEN_EXTRA_CLANG_ARGS PKG_CONFIG_PATH PKG_CONFIG_LIBDIR
unset CARGO_TARGET_DIR

TC_DIR="$ROOT_DIR/data/toolchains"
JDK_DIR="$TC_DIR/jdk"
SDK_DIR="$TC_DIR/android-sdk"

# Pinned versions
JDK_MAJOR="17"
CMDLINE_TOOLS_BUILD="16111833"
ANDROID_PLATFORMS="platforms;android-34 platforms;android-35 platforms;android-36"
ANDROID_BUILD_TOOLS="build-tools;36.0.0"
ANDROID_NDK_VERSION="30.0.16248370"
RUST_ANDROID_TARGETS="aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android"

# --- JDK (Temurin, portable tarball) ---
if [ ! -x "$JDK_DIR/bin/java" ]; then
  echo ">> Installing JDK $JDK_MAJOR into data/toolchains/jdk ..."
  curl -sSL "https://api.adoptium.net/v3/binary/latest/$JDK_MAJOR/ga/linux/x64/jdk/hotspot/normal/eclipse?project=jdk" \
    -o /tmp/_vm_jdk.tar.gz
  mkdir -p "$JDK_DIR"
  tar -xzf /tmp/_vm_jdk.tar.gz -C "$JDK_DIR" --strip-components=1
  rm -f /tmp/_vm_jdk.tar.gz
fi
export JAVA_HOME="$JDK_DIR"
export PATH="$JAVA_HOME/bin:$PATH"

# --- Android command-line tools ---
if [ ! -x "$SDK_DIR/cmdline-tools/latest/bin/sdkmanager" ]; then
  echo ">> Installing Android cmdline-tools into data/toolchains/android-sdk ..."
  curl -sSL "https://dl.google.com/android/repository/commandlinetools-linux-${CMDLINE_TOOLS_BUILD}_latest.zip" \
    -o /tmp/_vm_cmdline.zip
  mkdir -p "$SDK_DIR/cmdline-tools"
  rm -rf /tmp/_vm_cmdline
  unzip -q /tmp/_vm_cmdline.zip -d /tmp/_vm_cmdline
  rm -rf "$SDK_DIR/cmdline-tools/latest"
  mv /tmp/_vm_cmdline/cmdline-tools "$SDK_DIR/cmdline-tools/latest"
  rm -rf /tmp/_vm_cmdline /tmp/_vm_cmdline.zip
fi
export ANDROID_HOME="$SDK_DIR"
export ANDROID_SDK_ROOT="$SDK_DIR"
export PATH="$SDK_DIR/cmdline-tools/latest/bin:$SDK_DIR/platform-tools:$PATH"

# --- SDK packages (platform-tools, platforms, build-tools, NDK) ---
if [ ! -d "$SDK_DIR/ndk/$ANDROID_NDK_VERSION" ] || [ ! -x "$SDK_DIR/platform-tools/adb" ]; then
  echo ">> Installing Android SDK packages (this downloads a few GB) ..."
  yes | sdkmanager --sdk_root="$SDK_DIR" --licenses >/dev/null 2>&1 || true
  sdkmanager --sdk_root="$SDK_DIR" --install \
    "platform-tools" $ANDROID_PLATFORMS "$ANDROID_BUILD_TOOLS" "ndk;$ANDROID_NDK_VERSION"
fi
export NDK_HOME="$SDK_DIR/ndk/$ANDROID_NDK_VERSION"
export ANDROID_NDK_HOME="$NDK_HOME"

# --- Rust targets for Android ---
# To the Rust of rust-toolchain.toml (internal/platform-spec.md 14.6): rustup
# reads the file from the working directory upwards, and this file may be
# sourced from anywhere (the Gradle project of a product, a script of
# another folder), so it asks from the root of the repository.
for t in $RUST_ANDROID_TARGETS; do
  if ! (cd "$ROOT_DIR" && rustup target list --installed 2>/dev/null) | grep -qx "$t"; then
    echo ">> Adding Rust target $t ..."
    (cd "$ROOT_DIR" && rustup target add "$t")
  fi
done

# Gradle: keep caches inside the project too
export GRADLE_USER_HOME="$TC_DIR/gradle"

# WSL: share the Windows adb server so phones are visible from here. A build
# that talks to no phone (VEYDAN_ANDROID_NO_ADB=1) leaves the adb servers alone.
if [ "${VEYDAN_ANDROID_NO_ADB:-}" != 1 ]; then
  source "$ANDROID_SCRIPTS_DIR/adb-wsl.sh"
fi
