#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# The last job of a DEV build in the release workflow of Gitea
# (veydanproject/release; docs/ci-cd.md, "Releases"): the Linux bundles and
# the APK Gitea built (ASSETS_DIR), every updater signature checked against
# the public key of the product, published on the releases page of the
# monorepo on Gitea under RELEASE_TAG (scripts/release/gitea.sh). Nothing
# goes to GitHub. An rc and a release never come here: the product's own
# repository on GitHub builds and publishes them (scripts/release/publish.sh).
#
#   KIND                  dev; anything else is refused
#   RELEASE_TAG, RELEASE_NAME, ASSET_PREFIX, VERSION, UPDATER_KEYS,
#   UPDATER_PUBKEY        as scripts/release/meta.sh gives them
#   GITEA_TOKEN           the releases of the monorepo
#   ASSETS_DIR            what this run built
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

fail() {
  echo "::error::$*"
  exit 1
}

: "${KIND:?}" "${RELEASE_TAG:?}" "${RELEASE_NAME:?}" "${ASSET_PREFIX:?}" "${VERSION:?}" "${UPDATER_PUBKEY:?}"
UPDATER_KEYS="${UPDATER_KEYS:-}"
ASSETS_DIR="${ASSETS_DIR:-release-assets}"
mkdir -p "$ASSETS_DIR"

case "$KIND" in
  dev) ;;
  rc|release) fail "KIND=$KIND: an rc and a release are built and published by the product's repository on GitHub; Gitea only makes the snapshot" ;;
  *) fail "KIND=$KIND: dev" ;;
esac

node "$HERE/assets.mjs" verify --dir "$ASSETS_DIR" --prefix "$ASSET_PREFIX" --version "$VERSION" --keys "$UPDATER_KEYS" --pubkey "$UPDATER_PUBKEY"
export RELEASE_TAG RELEASE_NAME ASSETS_DIR
bash "$HERE/gitea.sh" publish
