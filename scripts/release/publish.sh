#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# One GitHub release out of the files the build jobs gathered
# (docs/platform-spec.md 14.3). The publishing jobs of
# .github/workflows/release.yml run it; it publishes where it is told and
# knows no repository and no token of its own:
#
#   REPO           <owner>/<repo> of the release
#   RELEASE_TAG    the tag the release is attached to; it must exist there
#   RELEASE_NAME   the title
#   PRERELEASE     true: a channel build — a prerelease that is never "latest"
#   ASSET_PREFIX   every file of ASSETS_DIR starts with it (Veydan.Notes)
#   VERSION        X.Y.Z, the version of latest.json
#   UPDATER_KEYS   the platforms of latest.json (linux-x86_64 …); empty: none
#   ASSETS_DIR     the bundles and their signatures
#   GH_TOKEN       read by gh: a token that may write releases in REPO
#
# The release is created with every file at once and checked afterwards:
# each file is there, the prerelease flag is what was asked, a prerelease
# is not what /releases/latest answers.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

fail() {
  echo "::error::$*"
  exit 1
}

: "${REPO:?}" "${RELEASE_TAG:?}" "${RELEASE_NAME:?}" "${ASSET_PREFIX:?}" "${VERSION:?}" "${ASSETS_DIR:?}"
PRERELEASE="${PRERELEASE:-false}"
UPDATER_KEYS="${UPDATER_KEYS:-}"
[ -n "${GH_TOKEN:-}" ] || fail "No token to publish $RELEASE_TAG in $REPO with"
[ -d "$ASSETS_DIR" ] || fail "$ASSETS_DIR: no assets to publish"

# A bundle of another product never goes into this release.
shopt -s nullglob
FILES=()
for file in "$ASSETS_DIR"/*; do
  name="$(basename "$file")"
  [ "$name" = "latest.json" ] && continue
  case "$name" in
    "${ASSET_PREFIX}_${VERSION}_"*|"${ASSET_PREFIX}-${VERSION}-"*) FILES+=("$file") ;;
    *) fail "$name is not a bundle of ${ASSET_PREFIX} ${VERSION}" ;;
  esac
done
[ "${#FILES[@]}" -gt 0 ] || fail "$ASSETS_DIR holds no bundle of ${ASSET_PREFIX} ${VERSION}"

if [ -n "$UPDATER_KEYS" ]; then
  node "$HERE/assets.mjs" latest --dir "$ASSETS_DIR" --prefix "$ASSET_PREFIX" --version "$VERSION" \
    --keys "$UPDATER_KEYS" --base-url "https://github.com/$REPO/releases/download/$RELEASE_TAG"
  FILES+=("$ASSETS_DIR/latest.json")
fi

NOTES="See the assets below to download and install this version."
FLAGS=()
if [ "$PRERELEASE" = "true" ]; then
  FLAGS+=(--prerelease --latest=false)
fi

if gh release view "$RELEASE_TAG" --repo "$REPO" >/dev/null 2>&1; then
  # A run started again on the same tag: the files are replaced.
  echo ">> $REPO already has the release $RELEASE_TAG: its files are replaced"
  gh release upload "$RELEASE_TAG" "${FILES[@]}" --repo "$REPO" --clobber
  gh release edit "$RELEASE_TAG" --repo "$REPO" --draft=false --title "$RELEASE_NAME" "${FLAGS[@]}"
else
  # --verify-tag: without the tag GitHub would make one on the default branch.
  gh release create "$RELEASE_TAG" "${FILES[@]}" --repo "$REPO" --verify-tag \
    --title "$RELEASE_NAME" --notes "$NOTES" "${FLAGS[@]}"
fi

# What the release really holds.
STATE="$(gh release view "$RELEASE_TAG" --repo "$REPO" --json tagName,isDraft,isPrerelease,assets)"
jq -e --arg t "$RELEASE_TAG" '.tagName == $t and .isDraft == false' <<<"$STATE" >/dev/null \
  || fail "$REPO: the release $RELEASE_TAG is not published"
jq -e --argjson p "$PRERELEASE" '.isPrerelease == $p' <<<"$STATE" >/dev/null \
  || fail "$REPO: the release $RELEASE_TAG has prerelease != $PRERELEASE"
for file in "${FILES[@]}"; do
  name="$(basename "$file")"
  jq -e --arg n "$name" '[.assets[].name] | index($n) != null' <<<"$STATE" >/dev/null \
    || fail "$REPO: the release $RELEASE_TAG lacks $name"
done
if [ "$PRERELEASE" = "true" ]; then
  LATEST="$(gh api "repos/$REPO/releases/latest" --jq .tag_name 2>/dev/null || true)"
  [ "$LATEST" != "$RELEASE_TAG" ] || fail "$REPO: the prerelease $RELEASE_TAG became the latest release"
fi
if [ -n "$UPDATER_KEYS" ]; then
  gh release download "$RELEASE_TAG" --repo "$REPO" --pattern latest.json -O "$ASSETS_DIR/latest.published.json" --clobber
  jq -e --arg v "$VERSION" '.version == $v' "$ASSETS_DIR/latest.published.json" >/dev/null \
    || fail "latest.json is not of $VERSION"
  for key in $UPDATER_KEYS; do
    jq -e --arg k "$key" --arg pre "https://github.com/$REPO/releases/download/$RELEASE_TAG/$ASSET_PREFIX" \
      '.platforms[$k] | (.signature | length > 0) and (.url | startswith($pre))' "$ASSETS_DIR/latest.published.json" >/dev/null \
      || fail "latest.json offers $key no bundle of $ASSET_PREFIX in $REPO"
  done
  rm -f "$ASSETS_DIR/latest.published.json"
fi
echo ">> https://github.com/$REPO/releases/tag/$RELEASE_TAG: ${#FILES[@]} files"
