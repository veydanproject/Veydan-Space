#!/usr/bin/env bash
# The version of one product (internal/platform-spec.md 14.1):
#   scripts/set-version.sh <product> <version>
# writes it into apps/<product>/VERSION, the product's Cargo.toml and its
# entry in Cargo.lock, its tauri.conf.json and the version code of its
# Android build. ui/package.json (the UI project) carries no product's version.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
if [ $# -ne 2 ]; then
  echo ">> usage: set-version.sh <product> <version>" >&2
  exit 1
fi
TO="$2"
if ! [[ "$TO" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo ">> version '$TO' is not X.Y.Z" >&2
  exit 1
fi
ROOT_DIR="$ROOT"
source "$ROOT/scripts/product.sh"
product_env "$1"

echo "$TO" > "$APP_DIR/VERSION"
# perl -pi, not sed -i: the same on Linux and on macOS (whose sed takes
# -i with a suffix and has no 0,/re/ address).
export TO PRODUCT_BIN
perl -pi -e 's/"version": "[^"]*"/"version": "$ENV{TO}"/' "$APP_DIR/tauri.conf.json"
perl -pi -e '$done ||= s/^version = "[^"]*"/version = "$ENV{TO}"/' "$APP_DIR/Cargo.toml"
# The lock file carries the version of the product's package; CI builds with --locked.
perl -pi -e 'if ($next) { s/^version = "[^"]*"/version = "$ENV{TO}"/; $next = 0 } $next = 1 if /^name = "\Q$ENV{PRODUCT_BIN}\E"$/' "$ROOT/Cargo.lock"

IFS=. read -r NMAJOR NMINOR NPATCH <<<"$TO"
CODE=$((2000000 + NMAJOR * 10000 + NMINOR * 100 + NPATCH))
export CODE
perl -pi -e 's/"versionCode": [0-9]*/"versionCode": $ENV{CODE}/' "$APP_DIR/tauri.android.conf.json"
