#!/usr/bin/env bash
# Run a product on a connected Android device (or emulator).
#   scripts/android/dev.sh [product]            # picks the connected device
#   scripts/android/dev.sh [product] "<name>"   # explicit device/emulator name
# The product is space when none is given (docs/platform-spec.md 13.2).
set -e

ANDROID_SCRIPTS_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$ANDROID_SCRIPTS_DIR/android-env.sh"
cd "$ROOT_DIR"
source "$ROOT_DIR/scripts/product.sh"
product_env "${1:-space}"
[ $# -gt 0 ] && shift
export CARGO_TARGET_DIR="$ROOT_DIR/data/target-android/$PRODUCT"

port="$PRODUCT_ANDROID_DEV_PORT"
hmr=$((port + 1))
# Free the product's android-only ports if a stale session holds them (the
# desktop dev server of the product keeps its own)
for p in "$port" "$hmr"; do
  pid=$(ss -ltnp 2>/dev/null | awk -v p=":$p" '$4 ~ p"$" {match($0, /pid=[0-9]+/); print substr($0, RSTART+4, RLENGTH-4)}' | head -n1)
  [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
done

# First run: the Android project of the product is created once and tracked in git.
if [ ! -d "$APP_DIR/gen/android" ]; then
  bash "$ANDROID_SCRIPTS_DIR/init.sh" "$PRODUCT"
fi

# Physical device: Tauri would embed the WSL NAT IP, which the phone cannot reach.
# Tunnel the android vite server and its HMR through adb; the desktop keeps its ports.
adb reverse "tcp:$port" "tcp:$port"
adb reverse "tcp:$hmr" "tcp:$hmr"

tauri_cli_ready
exec "$TAURI_CLI" android dev --host 127.0.0.1 "$@"
