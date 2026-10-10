#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# What a tag asks a release workflow for (docs/ci-cd.md, "Releases";
# internal/platform-spec.md 14.1, 14.3). Two workflows run it:
#
#   - the release workflow of veydanproject/release on Gitea, over a
#     checkout of the monorepo at the tag (RELEASE_BUILDER=gitea):
#       <product>-vX.Y.Z-dev.N[-linux|-android]…
#         → kind=dev: a build for ourselves. Linux and Android (both when the
#           tag names neither) are built on our runner and published on the
#           releases page of the monorepo on Gitea under the tag. Nothing
#           goes to GitHub.
#       <product>-vX.Y.Z-rc.N
#         → kind=rc: every platform, published as a PRE-RELEASE vX.Y.Z-rc.N
#           in the product's repository on GitHub (the updater never offers it)
#       <product>-vX.Y.Z
#         → kind=release: every platform built anew, published as THE release
#           vX.Y.Z (the latest: what every installed app updates to)
#     For rc and release the snapshot of the tagged commit goes to the
#     product's repository on GitHub under that tag, where Windows and macOS
#     are built into a draft; Linux and Android are built on our runner;
#     Gitea checks every signature, writes latest.json and publishes.
#   - the release.yml of a product's repository on GitHub (its snapshot):
#       GITHUB_REF_NAME=vX.Y.Z[-rc.N] GITHUB_REPOSITORY=veydanproject/Veydan-Chat
#         → own=true: Windows and macOS into a DRAFT release under the tag.
#
# By hand: GITHUB_OUTPUT=/dev/stdout prints the outputs. It reads
# products.json and the product's crate, calls nothing and writes nothing
# but the outputs.
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
OWN_PRODUCTS="$(jq -r '[.targets | to_entries[] | select(.value.kind == "product")] | length' products.json)"
BUILDER="${RELEASE_BUILDER:-github}"

if [ "${GITHUB_REF_TYPE:-}" != "tag" ]; then
  fail "Run a release on a tag, not on the branch ${GITHUB_REF_NAME:-}: make push <product> … starts it"
fi
TAG="${GITHUB_REF_NAME:-}"

# The tag with the product in front, as the monorepo names it.
if [ "$OWN_PRODUCTS" = "1" ] && [ "$BUILDER" = "github" ]; then
  [[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-rc\.[0-9]+)?$ ]] || fail "$TAG: the repository of a product takes vX.Y.Z and vX.Y.Z-rc.N alone; a dev build stays on Gitea"
  FULL="$PRODUCTS-$TAG"
  OWN="true"
elif [ "$BUILDER" = "gitea" ]; then
  FULL="$TAG"
  OWN="false"
else
  fail "$TAG: the monorepo builds nothing on GitHub; make push <product> rc|release starts a build on Gitea"
fi

DEV_RE='^([a-z]+)-v([0-9]+\.[0-9]+\.[0-9]+)-dev\.([0-9]+)((-(linux|windows|macos|android|ios))*)$'
RC_RE='^([a-z]+)-v([0-9]+\.[0-9]+\.[0-9]+)-rc\.([0-9]+)$'
RELEASE_RE='^([a-z]+)-v([0-9]+\.[0-9]+\.[0-9]+)$'
if [[ "$FULL" =~ $DEV_RE ]]; then
  PRODUCT="${BASH_REMATCH[1]}"
  VERSION="${BASH_REMATCH[2]}"
  CHANNEL="dev"
  KIND="dev"
  PRERELEASE="true"
  SUFFIX="${BASH_REMATCH[4]}"
  if [ -z "$SUFFIX" ]; then
    PLATFORMS="linux android"
  else
    # shellcheck disable=SC2086
    PLATFORMS="$(echo ${SUFFIX//-/ })"
    seen=" "
    for p in $PLATFORMS; do
      case "$seen" in
        *" $p "*) fail "$FULL names $p twice" ;;
      esac
      seen="$seen$p "
      case "$p" in
        windows|macos) fail "$FULL: a dev build is built on Gitea, Linux and Android alone; Windows and macOS are built for an rc and a release" ;;
        ios) fail "$FULL: ios is not built yet" ;;
      esac
    done
  fi
elif [[ "$FULL" =~ $RC_RE ]]; then
  PRODUCT="${BASH_REMATCH[1]}"
  VERSION="${BASH_REMATCH[2]}"
  CHANNEL="rc"
  KIND="rc"
  PRERELEASE="true"
  PLATFORMS="linux windows macos android"
elif [[ "$FULL" =~ $RELEASE_RE ]]; then
  PRODUCT="${BASH_REMATCH[1]}"
  VERSION="${BASH_REMATCH[2]}"
  CHANNEL="stable"
  KIND="release"
  PRERELEASE="false"
  PLATFORMS="linux windows macos android"
else
  fail "Tag $FULL does not match <product>-vX.Y.Z, <product>-vX.Y.Z-rc.N or <product>-vX.Y.Z-dev.N[-linux|-android]"
fi
case " $PRODUCTS " in
  *" $PRODUCT "*) ;;
  *) fail "$FULL: '$PRODUCT' is not a product of products.json ($PRODUCTS)" ;;
esac

APP="$(jq -r --arg p "$PRODUCT" '.targets[$p].app' products.json)"
REPO="$(jq -r --arg p "$PRODUCT" '.targets[$p].repo' products.json)"
if [ "$OWN" = "true" ] && [ "$GITHUB_REPOSITORY" != "$REPO" ]; then
  fail "$TAG is a tag of the repository $REPO of $PRODUCT, not of $GITHUB_REPOSITORY"
fi
FILE_VERSION="$(tr -d '[:space:]' < "$APP/VERSION")"
CONF_VERSION="$(jq -r .version "$APP/tauri.conf.json")"
if [ "$VERSION" != "$FILE_VERSION" ] || [ "$VERSION" != "$CONF_VERSION" ]; then
  fail "Tag version $VERSION does not match $APP/VERSION ($FILE_VERSION) and tauri.conf.json ($CONF_VERSION)"
fi
# The updater of an installed product reads the latest.json of the product's
# own repository on GitHub.
ENDPOINT="$(jq -r '.plugins.updater.endpoints[0]' "$APP/tauri.conf.json")"
if [ "$ENDPOINT" != "https://github.com/$REPO/releases/latest/download/latest.json" ]; then
  fail "$APP/tauri.conf.json: the updater endpoint $ENDPOINT is not the one of $REPO"
fi
PRODUCT_NAME="$(jq -r .productName "$APP/tauri.conf.json")"
# The public half of the updater key: scripts/release/assets.mjs checks that
# every signature of a build was made by its private half.
UPDATER_PUBKEY="$(jq -r '.plugins.updater.pubkey // empty' "$APP/tauri.conf.json")"
[ -n "$UPDATER_PUBKEY" ] || fail "$APP/tauri.conf.json: no plugins.updater.pubkey"
# GitHub stores a space of an asset name as a dot.
ASSET_PREFIX="${PRODUCT_NAME// /.}"
LIB_NAME="$(sed -n '/^\[lib\]/,/^\[/s/^name = "\(.*\)"/\1/p' "$APP/Cargo.toml" | head -n1)"
MESSENGER="$(jq -r --arg p "$PRODUCT" '.targets[$p].modules | index("messenger") != null' products.json)"

# A dev build is published on Gitea under the tag of the monorepo; an rc
# and a release in the product's repository on GitHub, under the tag
# without the product.
if [ "$KIND" = "dev" ]; then
  RELEASE_TAG="$FULL"
else
  RELEASE_TAG="${FULL#"$PRODUCT"-}"
fi
RELEASE_NAME="$PRODUCT_NAME ${FULL#"$PRODUCT"-}"
# The release note of the version (scripts/release/notes.mjs): the text of
# an rc and a release.
NOTES_FILE="$APP/release-notes/$VERSION.md"

# The matrix of GitHub (Windows, macOS), what latest.json names, what the
# draft must hold, what Gitea builds.
include='[]'
add() {
  include=$(jq -c --arg p "$1" --arg a "$2" --arg t "$3" --arg n "$4" \
    '. + [{"platform":$p,"args":$a,"target":$t,"artifact":$n}]' <<<"$include")
}
keys=""
draft_keys=""
BUILD_DESKTOP="false"
BUILD_LINUX="false"
BUILD_ANDROID="false"
for p in $PLATFORMS; do
  case "$p" in
    linux)
      keys="$keys linux-x86_64"
      BUILD_LINUX="true"
      ;;
    windows)
      add "windows-latest" "" "" "windows-x86_64"
      keys="$keys windows-x86_64"
      draft_keys="$draft_keys windows-x86_64"
      BUILD_DESKTOP="true"
      ;;
    macos)
      add "macos-latest" "--target aarch64-apple-darwin" "aarch64-apple-darwin" "darwin-aarch64"
      add "macos-latest" "--target x86_64-apple-darwin" "x86_64-apple-darwin" "darwin-x86_64"
      keys="$keys darwin-aarch64 darwin-x86_64"
      draft_keys="$draft_keys darwin-aarch64 darwin-x86_64"
      BUILD_DESKTOP="true"
      ;;
    android)
      BUILD_ANDROID="true"
      ;;
  esac
done
# In the product's repository on GitHub only Windows and macOS are built.
if [ "$OWN" = "true" ]; then
  BUILD_LINUX="false"
  BUILD_ANDROID="false"
fi
# A matrix may not be empty; the job that reads it is skipped then.
if [ "$include" = "[]" ]; then
  include='[{"platform":"ubuntu-22.04","args":"","target":"","artifact":"none"}]'
fi
# shellcheck disable=SC2086
keys="$(echo $keys)"
# shellcheck disable=SC2086
draft_keys="$(echo $draft_keys)"
MATRIX=$(jq -c '{include:.}' <<<"$include")

echo "product=$PRODUCT  version=$VERSION  kind=$KIND  channel=$CHANNEL  platforms=$PLATFORMS  → $RELEASE_TAG" >&2
{
  echo "product=$PRODUCT"
  echo "version=$VERSION"
  echo "kind=$KIND"
  echo "channel=$CHANNEL"
  echo "own=$OWN"
  echo "release_tag=$RELEASE_TAG"
  echo "notes_file=$NOTES_FILE"
  echo "release_name=$RELEASE_NAME"
  echo "product_repo=$REPO"
  echo "product_name=$PRODUCT_NAME"
  echo "asset_prefix=$ASSET_PREFIX"
  echo "messenger=$MESSENGER"
  echo "lib_name=$LIB_NAME"
  echo "prerelease=$PRERELEASE"
  echo "matrix=$MATRIX"
  echo "updater_keys=$keys"
  echo "draft_keys=$draft_keys"
  echo "updater_pubkey=$UPDATER_PUBKEY"
  echo "build_desktop=$BUILD_DESKTOP"
  echo "build_linux=$BUILD_LINUX"
  echo "build_android=$BUILD_ANDROID"
} >> "$GITHUB_OUTPUT"
