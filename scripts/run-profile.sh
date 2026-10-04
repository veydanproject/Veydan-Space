#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# One more instance of the dev build of a product with its own data directory
# (--workdir), next to a running `make dev`: it reuses that build and its
# vite server, and unlike dev.sh kills neither.
#
# usage: scripts/run-profile.sh <workdir> [product]
#        (or: make dev-profile [product] WORKDIR=<dir>; the product is space by default)
# No -u: build-env.sh reads variables that may be unset.
set -eo pipefail

dir="${1:-${WORKDIR:-}}"
if [ -z "$dir" ]; then
  echo ">> usage: make dev-profile [product] WORKDIR=<dir>" >&2
  exit 1
fi
ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT_DIR/scripts/product.sh"
# Relative to where the command was typed, not to the repo.
dev_workdir "$dir" || exit 1
product_env "${2:-space}"
source "$ROOT_DIR/scripts/build-env.sh" >/dev/null

BIN="$ROOT_DIR/data/target/debug/$PRODUCT_BIN"
if [ ! -x "$BIN" ]; then
  echo ">> no dev build of $PRODUCT yet: start 'make dev $PRODUCT' first" >&2
  exit 1
fi
if ! (exec 3<>/dev/tcp/127.0.0.1/"$PRODUCT_DEV_PORT") 2>/dev/null; then
  echo ">> the dev server of $PRODUCT is not running: start 'make dev $PRODUCT' first" >&2
  exit 1
fi

echo ">> profile: $VEYDAN_WORKDIR"
exec "$ROOT_DIR/scripts/webkit-run.sh" "$BIN"
