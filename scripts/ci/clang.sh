#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# clang of one major version from apt.llvm.org, for the C++ bridge of the
# engine of calls (crates/messenger/rtc/vendor/webrtc-sys needs clang >= 21
# for the hermetic libc++ of libwebrtc, which Ubuntu 22.04 has not):
#
#   bash scripts/ci/clang.sh 21      → /usr/bin/clang++-21
#
# Instead of `wget -qO- https://apt.llvm.org/llvm.sh | sudo bash`: no script
# from the network runs as root. The signing key of the repository is the
# one kept beside this file (scripts/ci/llvm-snapshot.asc, fingerprint
# 6084F3CF814B57C1CF12EFD515CF4D18AF4F7421, Sylvestre Ledru - Debian LLVM
# packages), checked by its hash before apt is told to trust it; apt then
# verifies every package against it. Ubuntu (the runners of GitHub) and Debian (the runner of Gitea).
set -euo pipefail

MAJOR="${1:?usage: scripts/ci/clang.sh <major version>}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
KEY="$HERE/llvm-snapshot.asc"
KEY_SHA256="8b2a587ffd672c4687e7581dad4b2f6c1bb2ad6b480cd9771ba2ff48e0b8c75d"

if command -v "clang++-$MAJOR" >/dev/null 2>&1; then
  echo ">> clang++-$MAJOR is already there: $(command -v "clang++-$MAJOR")"
  exit 0
fi

# Read in a subshell: os-release defines VERSION, ID… of its own.
OS_ID="$(. /etc/os-release && echo "${ID:-}")"
CODENAME="$(. /etc/os-release && echo "${VERSION_CODENAME:-}")"
if { [ "$OS_ID" != "ubuntu" ] && [ "$OS_ID" != "debian" ]; } || [ -z "$CODENAME" ]; then
  echo "::error::scripts/ci/clang.sh installs from apt.llvm.org for Ubuntu or Debian, not ${OS_ID:-unknown}"
  exit 1
fi

echo "$KEY_SHA256  $KEY" | sha256sum -c --quiet - || {
  echo "::error::$KEY is not the LLVM key this script was written for"
  exit 1
}

SUDO=""
[ "$(id -u)" = 0 ] || SUDO="sudo"
LIST="/etc/apt/sources.list.d/llvm-$MAJOR.list"
$SUDO install -d -m 0755 /etc/apt/keyrings
$SUDO install -m 0644 "$KEY" /etc/apt/keyrings/apt.llvm.org.asc
echo "deb [signed-by=/etc/apt/keyrings/apt.llvm.org.asc] https://apt.llvm.org/$CODENAME/ llvm-toolchain-$CODENAME-$MAJOR main" \
  | $SUDO tee "$LIST" >/dev/null
# Only this source is read again; the image's own lists stay as they are.
$SUDO apt-get update -qq -o Dir::Etc::sourcelist="$LIST" -o Dir::Etc::sourceparts="-" -o APT::Get::List-Cleanup="0"
$SUDO apt-get install -y -qq --no-install-recommends "clang-$MAJOR"
"clang++-$MAJOR" --version | head -n1
