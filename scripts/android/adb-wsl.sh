#!/usr/bin/env bash
# Under WSL2 the Linux adb server cannot see Wi-Fi/USB phones (NAT, no mDNS),
# so we run the adb server on the Windows host and point the WSL adb client
# at it. Pairing, connect, install and `tauri android dev` then all work from
# WSL. Meant to be *sourced* from android-env.sh. No-op outside WSL.

[ -f /proc/version ] && grep -qi microsoft /proc/version || return 0
export PATH="${PATH:-/usr/bin}:/mnt/c/Windows/System32:/mnt/c/Windows/System32/WindowsPowerShell/v1.0"
command -v cmd.exe >/dev/null 2>&1 || return 0

: "${SDK_DIR:?SDK_DIR must be set}"
: "${TC_DIR:?TC_DIR must be set}"

WIN_PT_SRC="$TC_DIR/win-platform-tools"

# Windows platform-tools, same version line as the WSL one.
if [ ! -f "$WIN_PT_SRC/adb.exe" ]; then
  echo ">> Installing Windows platform-tools into data/toolchains/ ..."
  curl -sSL "https://dl.google.com/android/repository/platform-tools-latest-windows.zip" -o /tmp/_vm_pt_win.zip
  rm -rf /tmp/_vm_pt_win "$WIN_PT_SRC"
  unzip -q /tmp/_vm_pt_win.zip -d /tmp/_vm_pt_win
  mv /tmp/_vm_pt_win/platform-tools "$WIN_PT_SRC"
  rm -rf /tmp/_vm_pt_win /tmp/_vm_pt_win.zip
fi

# adb.exe must live on a Windows drive to run reliably.
WIN_LOCALAPPDATA="$(cd /mnt/c && cmd.exe /c 'echo %LOCALAPPDATA%' 2>/dev/null | tr -d '\r')"
[ -n "$WIN_LOCALAPPDATA" ] || return 0
WIN_TOOLS_WIN="$WIN_LOCALAPPDATA\\veydan-tools"
WIN_TOOLS_LNX="$(wslpath -u "$WIN_TOOLS_WIN")"
mkdir -p "$WIN_TOOLS_LNX"
for f in adb.exe AdbWinApi.dll AdbWinUsbApi.dll; do
  cmp -s "$WIN_PT_SRC/$f" "$WIN_TOOLS_LNX/$f" 2>/dev/null || cp -f "$WIN_PT_SRC/$f" "$WIN_TOOLS_LNX/$f"
done

# Windows host as seen from WSL (NAT gateway).
WIN_HOST="$(ip route 2>/dev/null | awk '/^default/{print $3; exit}')"
[ -n "$WIN_HOST" ] || return 0
export ADB_SERVER_SOCKET="tcp:$WIN_HOST:5037"

# Never run a second server inside WSL.
"$SDK_DIR/platform-tools/adb" -L tcp:127.0.0.1:5037 kill-server >/dev/null 2>&1 || true

# Start the Windows server bound to all interfaces if WSL cannot reach it.
win_adb() { (cd /mnt/c && cmd.exe /c "cd /d $WIN_TOOLS_WIN && adb.exe $*" 2>/dev/null | tr -d '\r'); }
if ! timeout 5 "$SDK_DIR/platform-tools/adb" devices >/dev/null 2>&1; then
  win_adb kill-server >/dev/null 2>&1 || true
  win_adb -a start-server >/dev/null 2>&1 || true
fi
