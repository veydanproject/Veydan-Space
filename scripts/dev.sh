#!/usr/bin/env bash
# Run a product in development: the dev server and a debug build.
#
#   scripts/dev.sh [product]  # space (default), notes, pass or chat;  make dev [product]
#
# WORKDIR=<dir>: run this instance as a separate profile (--workdir).
set -e

# The root of the repository: this file is in scripts/.
ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

# The product: its crate, binary, names and dev port (internal/platform-spec.md 13.2).
source "$ROOT_DIR/scripts/product.sh"
product_env "${1:-space}"

# Resolved here because `tauri dev` starts the app from the product's crate.
if [ -n "${WORKDIR:-}" ]; then
  dev_workdir "$WORKDIR" || exit 1
  # A dev environment of this product is already up: join it as one more
  # instance instead of replacing it (one dev server per product, one debug build).
  if (exec 3<>/dev/tcp/127.0.0.1/"$PRODUCT_DEV_PORT") 2>/dev/null &&
    [ -x "$ROOT_DIR/data/target/debug/$PRODUCT_BIN" ]; then
    exec bash "$ROOT_DIR/scripts/run-profile.sh" "$VEYDAN_WORKDIR" "$PRODUCT"
  fi
fi

# One-time data/dev-prefix setup + build env vars + project-local toolchain.
source "$ROOT_DIR/scripts/build-env.sh"

# Free the product's dev server and HMR ports if a stale process holds them.
# `fuser` is not on every machine (psmisc); without it the holder is found
# through `ss`. Other products keep theirs.
for port in "$PRODUCT_DEV_PORT" "$((PRODUCT_DEV_PORT + 1))"; do
  if command -v fuser >/dev/null 2>&1; then
    fuser -k "$port/tcp" 2>/dev/null || true
  else
    for pid in $(ss -Hltnp "sport = :$port" 2>/dev/null | grep -o 'pid=[0-9]*' | cut -d= -f2 | sort -u); do
      kill "$pid" 2>/dev/null || true
    done
  fi
done

# Kill an orphaned dev instance of this product; it holds the single-instance
# lock and the new launch would silently exit after just poking the hidden
# old window. The pattern is the product's own binary.
pkill -f "data/target/debug/$PRODUCT_BIN$" 2>/dev/null || true

# --- Taskbar icon for the dev build (Wayland/KDE, GNOME, …) ------------------
# Wayland compositors don't read a window's embedded icon like X11 did; they
# resolve the taskbar icon purely by matching the window's app_id to an
# installed .desktop file. `tauri dev` installs nothing, so the app_id
# (the binary basename) matches no desktop file and the DE falls back to a
# generated letter-avatar placeholder instead of our logo. Drop a desktop
# file + icons into the user's XDG dirs so the logo resolves.
# Idempotent and best-effort — never abort the dev run over it.
install_dev_icon() {
  local app_id="$PRODUCT_BIN"
  local apps_dir="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
  local icons_base="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor"
  local src="$APP_DIR/icons"

  mkdir -p "$apps_dir"
  # size -> source png (hicolor needs each size in its own dir)
  local map=(
    "32x32:$src/32x32.png"
    "64x64:$src/64x64.png"
    "128x128:$src/128x128.png"
    "256x256:$src/128x128@2x.png"
    "512x512:$src/icon.png"
  )
  local entry size file
  for entry in "${map[@]}"; do
    size="${entry%%:*}"; file="${entry#*:}"
    [ -f "$file" ] || continue
    mkdir -p "$icons_base/$size/apps"
    cp -f "$file" "$icons_base/$size/apps/$app_id.png"
  done

  cat > "$apps_dir/$app_id.desktop" << EOF
[Desktop Entry]
Type=Application
Name=$PRODUCT_NAME
Comment=$(product_desktop_field Comment) (dev)
Exec=$ROOT_DIR/scripts/dev.sh $PRODUCT
Icon=$app_id
Terminal=false
Categories=$(product_desktop_field Categories)
StartupNotify=true
StartupWMClass=$app_id
NoDisplay=true
EOF

  # Refresh caches where the tools exist (KDE reads live, GNOME likes a nudge)
  command -v update-desktop-database >/dev/null 2>&1 && \
    update-desktop-database "$apps_dir" >/dev/null 2>&1 || true
  command -v gtk-update-icon-cache >/dev/null 2>&1 && \
    gtk-update-icon-cache -q -t -f "$icons_base" >/dev/null 2>&1 || true
}
install_dev_icon || true

# The CLI of the UI project runs from the root and finds the product through
# TAURI_APP_PATH, the UI project through TAURI_FRONTEND_PATH (product.sh).
tauri_cli_ready
cd "$ROOT_DIR"
exec "$TAURI_CLI" dev
