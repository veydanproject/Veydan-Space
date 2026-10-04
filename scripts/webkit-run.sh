#!/usr/bin/env bash
# Cargo "runner" wrapper (wired up via CARGO_TARGET_*_RUNNER in build-env.sh).
#
# WebKitGTK spawns its helper binaries (WebKitNetworkProcess, WebKitWebProcess,
# WebKitGPUProcess) from a path compiled into the library:
#   /usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/
# There is no env var to override it in 2.52. Since we keep WebKit project-local
# in data/dev-prefix/ (and can't write to /usr without root), we launch the app in a
# bubblewrap namespace that bind-mounts our copy onto that path. The overlay is
# only visible inside the sandbox — the real /usr is never touched.
set -e

# The root of the repository, from where this file is (scripts/).
VEYDAN_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LIBDIR="$VEYDAN_ROOT/data/dev-prefix/usr/lib/x86_64-linux-gnu"
WK="$LIBDIR/webkit2gtk-4.1"
SYS_WK="/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1"

# WSLg sets WAYLAND_DISPLAY; WebKit then inits Wayland EGL and abort()s.
export GDK_BACKEND=x11
unset WAYLAND_DISPLAY

# If the host already has WebKit installed at the system path, or bwrap isn't
# available, just run the binary directly.
if [ -e "$SYS_WK/WebKitNetworkProcess" ] || ! command -v bwrap >/dev/null 2>&1; then
  exec "$@"
fi

# Make /usr/lib/x86_64-linux-gnu writable (tmpfs overlay over the host dir) so
# bwrap can create the mount point, then bind our WebKit libexec dir onto it.
# --die-with-parent: when the tauri CLI kills bwrap, the app dies too instead
# of surviving as an orphan that holds the single-instance lock.
exec bwrap \
  --die-with-parent \
  --dev-bind / / \
  --overlay-src /usr/lib/x86_64-linux-gnu \
  --tmp-overlay /usr/lib/x86_64-linux-gnu \
  --bind "$WK" "$SYS_WK" \
  --chdir "$PWD" \
  "$@"
