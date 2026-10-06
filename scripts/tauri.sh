#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# The Tauri CLI for one product (internal/platform-spec.md 13.2):
#
#   scripts/tauri.sh <product> <args…>
#   scripts/tauri.sh notes build --no-bundle      # after `source scripts/build-env.sh`
#   scripts/tauri.sh notes inspect wix-upgrade-code
#
# The product is required (no default: the first word could be a command of
# the CLI). It runs ui/node_modules/.bin/tauri from the repository root with
# TAURI_APP_PATH=apps/<product> and TAURI_FRONTEND_PATH=ui (scripts/product.sh);
# the product's beforeDevCommand and beforeBuildCommand then run in ui/.
# The other wrappers (dev.sh, update.sh, android/*.sh) do the same for their
# command; this one is for everything else.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [ $# -lt 1 ]; then
  echo ">> usage: scripts/tauri.sh <product> <args…>" >&2
  exit 2
fi
source "$ROOT_DIR/scripts/product.sh"
product_env "$1"
shift
tauri_cli_ready
cd "$ROOT_DIR"
exec "$TAURI_CLI" "$@"
