#!/usr/bin/env bash
# Shared build environment for dev.sh and update.sh (both in scripts/, like this
# file) — meant to be *sourced*, from anywhere; it ends in the root.
#
# Does the one-time data/dev-prefix/ setup (headers + WebKit runtime extracted from
# debs), exports the build env vars, and bootstraps the project-local
# Rust/Node/pnpm toolchain. Keeping this in one file guarantees dev and release
# builds run with identical environments.

# The root of the repository, from where this file is (scripts/), whoever
# sources it and from wherever.
VEYDAN_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export VEYDAN_ROOT
DEV_PREFIX="$VEYDAN_ROOT/data/dev-prefix"
LIBDIR="$DEV_PREFIX/usr/lib/x86_64-linux-gnu"
PKG_DIR="$LIBDIR/pkgconfig"

# One-time setup: extract deb packages into data/dev-prefix/ inside the project.
# We fetch both the -dev packages (headers + .pc files, needed to build) and
# the webkit runtime packages (the .so.0 shared objects, needed to run) so the
# project is fully self-contained even on hosts where WebKitGTK isn't installed.
if [ ! -f "$PKG_DIR/webkit2gtk-4.1.pc" ]; then
  echo ">> First-time setup: extracting dev packages into data/dev-prefix/ ..."
  mkdir -p "$DEV_PREFIX" /tmp/_rb_debs

  # Root -dev packages the build links against. We resolve their full recursive
  # -dev dependency closure below so every header + .pc file in the GTK/WebKit
  # stack lands in the prefix (on Debian trixie these are split across many
  # packages, e.g. glib's .pc moved into libgio-2.0-dev).
  dev_roots=(
    libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev
    libsoup-3.0-dev libgtk-3-dev librsvg2-dev
    libayatana-appindicator3-dev
    libsqlite3-dev libpsl-dev libkrb5-dev libnghttp2-dev
    libbrotli-dev libsysprof-capture-4-dev
    libssl-dev libdbus-1-dev libsystemd-dev
  )

  # Runtime packages: shared objects not part of a typical desktop base install
  # (WebKit + its private deps). Everything else (GTK, cairo, glib, openssl, …)
  # already ships as runtime on the host; we only add the missing .so symlinks.
  runtime_pkgs=(
    libwebkit2gtk-4.1-0 libjavascriptcoregtk-4.1-0
    libmanette-0.2-0 libenchant-2-2 libhidapi-hidraw0
  )

  echo ">> Resolving recursive -dev dependency closure ..."
  dev_closure=$(apt-cache depends --recurse --no-recommends --no-suggests \
    --no-conflicts --no-breaks --no-replaces --no-enhances "${dev_roots[@]}" \
    2>/dev/null | grep -E '^\w' | grep -E '\-dev$' | sort -u)

  cd /tmp/_rb_debs
  echo ">> Downloading packages ..."
  apt-get download $dev_closure "${runtime_pkgs[@]}"
  for f in *.deb; do
    dpkg-deb -x "$f" "$DEV_PREFIX/"
  done
  cd "$VEYDAN_ROOT"

  # krb5-gssapi.pc is a dangling symlink in the deb — create the real file
  mkdir -p "$PKG_DIR/mit-krb5"
  cat > "$PKG_DIR/mit-krb5/krb5-gssapi.pc" << 'EOF'
prefix=/usr
libdir=${prefix}/lib/x86_64-linux-gnu
includedir=${prefix}/include
Name: krb5-gssapi
Description: Kerberos 5 GSSAPI
Version: 1.21.3
Libs: -L${libdir} -lgssapi_krb5
Cflags: -I${includedir}
EOF

  # The linker needs bare lib*.so files. The -dev debs ship them as relative
  # symlinks (lib*.so -> lib*.so.N), but for libs whose runtime lives on the
  # host the target .so.N isn't in the prefix. Repoint every such dangling
  # symlink at the host's runtime copy so -L$LIBDIR resolves it.
  for l in "$LIBDIR"/*.so; do
    [ -L "$l" ] || continue
    base=$(basename "$(readlink "$l")")
    if [ ! -e "$LIBDIR/$base" ] && [ -e "/usr/lib/x86_64-linux-gnu/$base" ]; then
      ln -sf "/usr/lib/x86_64-linux-gnu/$base" "$l"
    fi
  done

  rm -rf /tmp/_rb_debs
  echo ">> Setup complete."
fi

# Build environment
export PKG_CONFIG_PATH="$PKG_DIR:$DEV_PREFIX/usr/share/pkgconfig:/usr/lib/x86_64-linux-gnu/pkgconfig:/usr/share/pkgconfig"
# Include both the plain and the multiarch header dirs — Debian ships some
# arch-specific headers (e.g. openssl/opensslconf.h) under x86_64-linux-gnu/.
export CFLAGS="-I$DEV_PREFIX/usr/include -I$DEV_PREFIX/usr/include/x86_64-linux-gnu"
export CXXFLAGS="$CFLAGS"
export RUSTFLAGS="-L $DEV_PREFIX/usr/lib/x86_64-linux-gnu -L /usr/lib/x86_64-linux-gnu"
# Project-local compiler/linker/pkg-config extracted into the prefix
export PATH="$DEV_PREFIX/usr/bin:$PATH"
export GCC_EXEC_PREFIX="$DEV_PREFIX/usr/lib/gcc/"
# Runtime: let the loader find the WebKit .so.0 we extracted into the prefix
export LD_LIBRARY_PATH="$LIBDIR:$LD_LIBRARY_PATH"
export XKB_CONFIG_ROOT="${XKB_CONFIG_ROOT:-$DEV_PREFIX/usr/share/X11/xkb}"
export FONTCONFIG_PATH="${FONTCONFIG_PATH:-$DEV_PREFIX/etc/fonts}"
export FONTCONFIG_FILE="${FONTCONFIG_FILE:-$DEV_PREFIX/etc/fonts/fonts.conf}"
# WSL/WebKit: skip GPU compositing (EGL often aborts under WSLg)
export WEBKIT_DISABLE_COMPOSITING_MODE="${WEBKIT_DISABLE_COMPOSITING_MODE:-1}"
export WEBKIT_DISABLE_DMABUF_RENDERER="${WEBKIT_DISABLE_DMABUF_RENDERER:-1}"
# Prefix Mesa: glvnd has no /usr/share/glvnd on this host, and WebKit aborts
# if WAYLAND_DISPLAY points at WSLg's broken EGL.
export __EGL_VENDOR_LIBRARY_DIRS="$DEV_PREFIX/usr/share/glvnd/egl_vendor.d"
export LIBGL_DRIVERS_PATH="$LIBDIR/dri"
export LIBGL_ALWAYS_SOFTWARE=1
export GDK_BACKEND=x11
export GSETTINGS_SCHEMA_DIR="$DEV_PREFIX/usr/share/glib-2.0/schemas"
export XDG_DATA_DIRS="$DEV_PREFIX/usr/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
# WebKit spawns helper binaries (WebKitNetworkProcess, WebKitWebProcess, …) from
# a /usr path compiled into the library with no env override. We run the app in
# a bubblewrap sandbox that bind-mounts our prefix copy onto that path — wired
# in as cargo's "runner" so only the app (not the build) is sandboxed.
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="$VEYDAN_ROOT/scripts/webkit-run.sh"
# Don't let WebKit start its own (nested) bwrap sandbox inside ours.
export WEBKIT_DISABLE_SANDBOX=1
# WebKit's media player needs the fakevideosink element, which ships inside
# gst-plugins-bad's debugutils plugin. Without it HTML audio and video stay silent.
debugutils="$LIBDIR/gstreamer-1.0/libgstdebugutilsbad.so"
if [ ! -f "$debugutils" ]; then
  echo ">> WebKit media: extracting fakevideosink ..."
  tmp=$(mktemp -d)
  if ( cd "$tmp" && apt-get download gstreamer1.0-plugins-bad && dpkg-deb -x ./*.deb "$tmp/root" ); then
    found=$(find "$tmp/root" -name 'libgstdebugutilsbad.so' -print -quit)
    if [ -n "$found" ]; then
      mkdir -p "$(dirname "$debugutils")"
      cp -f "$found" "$debugutils"
    fi
  fi
  rm -rf "$tmp"
fi
# A failed first scan blacklists pulsesink forever, and WebKit then plays
# through OSS, which is silent under WSLg. Drop that registry so the next
# launch retries PulseAudio.
gst_reg="${XDG_CACHE_HOME:-$HOME/.cache}/gstreamer-1.0/registry.x86_64.bin"
if [ -f "$gst_reg" ] && strings "$gst_reg" | grep -q 'libgstpulseaudio.so' \
  && strings "$gst_reg" | grep -q 'BLACKLIST'; then
  rm -f "$gst_reg"
fi
# OpenSSL: use the headers from the prefix and the host's runtime .so, and
# skip the vendored build. Setting both dirs makes openssl-sys skip pkg-config
# (whose prefix=/usr would otherwise point at non-existent system headers).
export OPENSSL_LIB_DIR="$LIBDIR"
export OPENSSL_INCLUDE_DIR="$DEV_PREFIX/usr/include"
export OPENSSL_NO_VENDOR=1

# Project-local Rust + Node + pnpm (installs into data/toolchains/ on first run)
source "$VEYDAN_ROOT/scripts/toolchain.sh"

cd "$VEYDAN_ROOT"
