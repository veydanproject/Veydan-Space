#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# What a tag asks the release workflow for (internal/platform-spec.md 14.1, 14.3):
#
#   GITHUB_REF_TYPE=tag GITHUB_REF_NAME=notes-v5.0.0-alpha.1 \
#   GITHUB_REPOSITORY=<owner>/<repo> GITHUB_OUTPUT=<file> scripts/release/meta.sh
#
# The job `meta` of .github/workflows/release.yml runs it; by hand it prints
# the same lines (GITHUB_OUTPUT=/dev/stdout). It reads products.json and the
# product's crate, calls nothing and writes nothing but the outputs.
#
#   <product>-vX.Y.Z                              kind=release: every platform,
#                                                 published in the product's
#                                                 repository under vX.Y.Z
#   <product>-vX.Y.Z-<channel>.N[-<platform>]…    kind=channel: the platforms
#                                                 named or the three desktop
#                                                 ones, a prerelease of THIS
#                                                 repository under the tag itself
#
# A channel build gets no `product_repo`: nothing after this job can address
# the product's repository for it.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  echo "::error::$*"
  exit 1
}

: "${GITHUB_OUTPUT:?GITHUB_OUTPUT must name the file of the outputs}"
: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY must name the repository of the run}"

PRODUCTS="$(jq -r '.targets | to_entries[] | select(.value.kind == "product") | .key' products.json | xargs)"

if [ "${GITHUB_REF_TYPE:-}" != "tag" ]; then
  fail "Run a release on a product tag (<product>-vX.Y.Z…), not on the branch ${GITHUB_REF_NAME:-}: make push <product> … starts it"
fi

TAG="${GITHUB_REF_NAME:-}"
# In the product's own repository (its snapshot: products.json holds one
# product) the tag is vX.Y.Z: a release built and published right there.
OWN_PRODUCTS="$(jq -r '[.targets | to_entries[] | select(.value.kind == "product")] | length' products.json)"
if [ "$OWN_PRODUCTS" = "1" ] && [[ "$TAG" =~ ^v([0-9]+\.[0-9]+\.[0-9]+)$ ]]; then
  TAG="$PRODUCTS-$TAG"
  OWN="true"
else
  OWN="false"
fi
if [[ "$TAG" =~ ^([a-z]+)-v([0-9]+\.[0-9]+\.[0-9]+)$ ]]; then
  PRODUCT="${BASH_REMATCH[1]}"
  VERSION="${BASH_REMATCH[2]}"
  KIND="release"
  CHANNEL="stable"
  PRERELEASE="false"
  PLATFORMS="linux windows macos android"
elif [[ "$TAG" =~ ^([a-z]+)-v([0-9]+\.[0-9]+\.[0-9]+)-(alpha|beta|rc)\.([0-9]+)((-(linux|windows|macos|android|ios))*)$ ]]; then
  PRODUCT="${BASH_REMATCH[1]}"
  VERSION="${BASH_REMATCH[2]}"
  KIND="channel"
  CHANNEL="${BASH_REMATCH[3]}"
  PRERELEASE="true"
  SUFFIX="${BASH_REMATCH[5]}"
  if [ -z "$SUFFIX" ]; then
    PLATFORMS="linux windows macos"
  else
    # shellcheck disable=SC2086
    PLATFORMS="$(echo ${SUFFIX//-/ })"
    # Twice the same platform would make two jobs that upload one artifact.
    seen=" "
    for p in $PLATFORMS; do
      case "$seen" in
        *" $p "*) fail "$TAG names $p twice" ;;
      esac
      seen="$seen$p "
    done
  fi
else
  fail "Tag $TAG does not match <product>-vX.Y.Z or <product>-vX.Y.Z-(alpha|beta|rc).N[-linux|-windows|-macos|-android|-ios...]"
fi
case " $PRODUCTS " in
  *" $PRODUCT "*) ;;
  *) fail "$TAG: '$PRODUCT' is not a product of products.json ($PRODUCTS)" ;;
esac

APP="$(jq -r --arg p "$PRODUCT" '.targets[$p].app' products.json)"
REPO="$(jq -r --arg p "$PRODUCT" '.targets[$p].repo' products.json)"
FILE_VERSION="$(tr -d '[:space:]' < "$APP/VERSION")"
CONF_VERSION="$(jq -r .version "$APP/tauri.conf.json")"
if [ "$VERSION" != "$FILE_VERSION" ] || [ "$VERSION" != "$CONF_VERSION" ]; then
  fail "Tag version $VERSION does not match $APP/VERSION ($FILE_VERSION) and tauri.conf.json ($CONF_VERSION)"
fi
# The updater of an installed product reads the latest.json of the product's
# own repository, whatever built it: a channel build too (14.3).
ENDPOINT="$(jq -r '.plugins.updater.endpoints[0]' "$APP/tauri.conf.json")"
if [ "$ENDPOINT" != "https://github.com/$REPO/releases/latest/download/latest.json" ]; then
  fail "$APP/tauri.conf.json: the updater endpoint $ENDPOINT is not the one of $REPO"
fi
PRODUCT_NAME="$(jq -r .productName "$APP/tauri.conf.json")"
# The public half of the updater key: scripts/release/assets.mjs checks that
# every signature of the build was made by its private half (the secret).
UPDATER_PUBKEY="$(jq -r '.plugins.updater.pubkey // empty' "$APP/tauri.conf.json")"
[ -n "$UPDATER_PUBKEY" ] || fail "$APP/tauri.conf.json: no plugins.updater.pubkey"
# GitHub stores a space of an asset name as a dot; scripts/release/assets.mjs
# names the assets so before they are uploaded.
ASSET_PREFIX="${PRODUCT_NAME// /.}"
LIB_NAME="$(sed -n '/^\[lib\]/,/^\[/s/^name = "\(.*\)"/\1/p' "$APP/Cargo.toml" | head -n1)"
MESSENGER="$(jq -r --arg p "$PRODUCT" '.targets[$p].modules | index("messenger") != null' products.json)"

if [ "$OWN" = "true" ]; then
  # The snapshot publishes its own release, under its own tag (vX.Y.Z).
  [ "$GITHUB_REPOSITORY" = "$REPO" ] || fail "$GITHUB_REF_NAME: a vX.Y.Z tag is a release of the product repository $REPO, not of $GITHUB_REPOSITORY"
  KIND="own"
  PUBLISH_REPO="$REPO"
  PRODUCT_REPO=""
  RELEASE_TAG="$GITHUB_REF_NAME"
elif [ "$KIND" = "release" ]; then
  fail "$TAG: a release is built by $REPO from its snapshot (make push $PRODUCT release publishes it there); the tag of the monorepo only marks the commit"
  # In the product's repository the tag has no prefix: v5.0.0.
  PUBLISH_REPO="$REPO"
  PRODUCT_REPO="$REPO"
  RELEASE_TAG="${TAG#"$PRODUCT"-}"
else
  # A channel build stays here, under the tag that was pushed.
  PUBLISH_REPO="$GITHUB_REPOSITORY"
  PRODUCT_REPO=""
  RELEASE_TAG="$TAG"
  if [ "$PUBLISH_REPO" = "$REPO" ]; then
    fail "$TAG: a channel build is published in the repository it is built in, and this one is the product's own ($REPO), which takes releases only"
  fi
fi
RELEASE_NAME="$PRODUCT_NAME ${TAG#"$PRODUCT"-}"

include='[]'
add() {
  include=$(jq -c --arg p "$1" --arg a "$2" --arg t "$3" --arg n "$4" \
    '. + [{"platform":$p,"args":$a,"target":$t,"artifact":$n}]' <<<"$include")
}
keys=""
BUILD_DESKTOP="false"
BUILD_ANDROID="false"
for p in $PLATFORMS; do
  case "$p" in
    linux)
      add "ubuntu-22.04" "" "" "linux-x86_64"
      keys="$keys linux-x86_64"
      BUILD_DESKTOP="true"
      ;;
    windows)
      add "windows-latest" "" "" "windows-x86_64"
      keys="$keys windows-x86_64"
      BUILD_DESKTOP="true"
      ;;
    macos)
      add "macos-latest" "--target aarch64-apple-darwin" "aarch64-apple-darwin" "darwin-aarch64"
      add "macos-latest" "--target x86_64-apple-darwin" "x86_64-apple-darwin" "darwin-x86_64"
      keys="$keys darwin-aarch64 darwin-x86_64"
      BUILD_DESKTOP="true"
      ;;
    android)
      BUILD_ANDROID="true"
      ;;
    ios)
      echo "::notice::Skipping ios (not in CI yet)"
      ;;
  esac
done
if [ "$BUILD_DESKTOP" != "true" ] && [ "$BUILD_ANDROID" != "true" ]; then
  fail "No platforms to build"
fi
# A matrix may not be empty; the job that reads it is skipped then.
if [ "$include" = "[]" ]; then
  include='[{"platform":"ubuntu-22.04","args":"","target":"","artifact":"none"}]'
fi
# shellcheck disable=SC2086
keys="$(echo $keys)"
MATRIX=$(jq -c '{include:.}' <<<"$include")

echo "product=$PRODUCT  version=$VERSION  kind=$KIND  channel=$CHANNEL  platforms=$PLATFORMS  → $PUBLISH_REPO $RELEASE_TAG" >&2
{
  echo "product=$PRODUCT"
  echo "version=$VERSION"
  echo "kind=$KIND"
  echo "channel=$CHANNEL"
  echo "release_tag=$RELEASE_TAG"
  echo "release_name=$RELEASE_NAME"
  echo "product_repo=$PRODUCT_REPO"
  echo "product_name=$PRODUCT_NAME"
  echo "asset_prefix=$ASSET_PREFIX"
  echo "messenger=$MESSENGER"
  echo "lib_name=$LIB_NAME"
  echo "prerelease=$PRERELEASE"
  echo "matrix=$MATRIX"
  echo "updater_keys=$keys"
  echo "updater_pubkey=$UPDATER_PUBKEY"
  echo "build_desktop=$BUILD_DESKTOP"
  echo "build_android=$BUILD_ANDROID"
} >> "$GITHUB_OUTPUT"
