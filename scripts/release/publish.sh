#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# The GitHub release of one product out of the files the build jobs
# gathered (docs/ci-cd.md, "Releases"; internal/platform-spec.md 14.3): the
# job `publish` of release.yml in the product's repository. It publishes
# where it is told and knows no repository and no token of its own:
#
#   MODE           channel  — every file of ASSETS_DIR (the bundles of every
#                             platform and their signatures, the APK) plus
#                             latest.json over UPDATER_KEYS and SHA256SUMS
#                             over all of them, published as a PRERELEASE,
#                             never the latest
#                  release  — the same files, published as THE release (latest)
#   REPO           <owner>/<repo> of the release
#   RELEASE_TAG    the tag the release is attached to; it must exist there
#   RELEASE_NAME   the title
#   ASSET_PREFIX   every file of ASSETS_DIR starts with it (Veydan.Notes)
#   VERSION        X.Y.Z, the version of latest.json
#   UPDATER_KEYS   the platforms of latest.json (linux-x86_64 …); empty: none
#   ASSETS_DIR     the bundles and their signatures
#   NOTES_FILE     the text of the release (scripts/release/notes.mjs body):
#                  its description on GitHub and the notes of latest.json;
#                  a line that points at the files when none is given
#   GH_TOKEN       read by gh: a token that may write releases in REPO
#
# The release is created with every file at once and checked afterwards:
# each file is there, the flags are what was asked, a prerelease is not
# what /releases/latest answers, latest.json names this release. A run
# started again replaces the files of the release in place.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

fail() {
  echo "::error::$*"
  exit 1
}

: "${REPO:?}" "${RELEASE_TAG:?}" "${RELEASE_NAME:?}" "${ASSET_PREFIX:?}" "${VERSION:?}" "${ASSETS_DIR:?}"
MODE="${MODE:-channel}"
UPDATER_KEYS="${UPDATER_KEYS:-}"
case "$MODE" in channel|release) ;; *) fail "MODE=$MODE: channel or release" ;; esac
[ -n "${GH_TOKEN:-}" ] || fail "No token to publish $RELEASE_TAG in $REPO with"
[ -d "$ASSETS_DIR" ] || fail "$ASSETS_DIR: no assets to publish"

# A bundle of another product never goes into this release.
shopt -s nullglob
FILES=()
for file in "$ASSETS_DIR"/*; do
  name="$(basename "$file")"
  case "$name" in latest.json|SHA256SUMS|latest.published.json) continue ;; esac
  case "$name" in
    "${ASSET_PREFIX}_${VERSION}_"*|"${ASSET_PREFIX}-${VERSION}-"*) FILES+=("$file") ;;
    *) fail "$name is not a bundle of ${ASSET_PREFIX} ${VERSION}" ;;
  esac
done
[ "${#FILES[@]}" -gt 0 ] || fail "$ASSETS_DIR holds no bundle of ${ASSET_PREFIX} ${VERSION}"

NOTES_FILE="${NOTES_FILE:-}"
if [ -n "$UPDATER_KEYS" ]; then
  node "$HERE/assets.mjs" latest --dir "$ASSETS_DIR" --prefix "$ASSET_PREFIX" --version "$VERSION" \
    --keys "$UPDATER_KEYS" --base-url "https://github.com/$REPO/releases/download/$RELEASE_TAG" \
    ${NOTES_FILE:+--notes-file "$NOTES_FILE"}
  FILES+=("$ASSETS_DIR/latest.json")
fi
# The sums of every file of the release, latest.json included, written
# before anything is published: the release is complete from its first second.
(cd "$ASSETS_DIR" && rm -f SHA256SUMS latest.published.json && sha256sum -- * > SHA256SUMS)
FILES+=("$ASSETS_DIR/SHA256SUMS")

if [ -n "$NOTES_FILE" ]; then
  [ -s "$NOTES_FILE" ] || fail "NOTES_FILE=$NOTES_FILE: no text of the release"
  NOTES_ARGS=(--notes-file "$NOTES_FILE")
else
  NOTES_ARGS=(--notes "See the assets below to download and install this version.")
fi
FLAGS=()
PRERELEASE="false"
case "$MODE" in
  channel) FLAGS+=(--prerelease --latest=false) ; PRERELEASE="true" ;;
  release) FLAGS+=(--latest) ;;
esac

if gh release view "$RELEASE_TAG" --repo "$REPO" >/dev/null 2>&1; then
  # A run started again: the files are replaced.
  echo ">> $REPO already has the release $RELEASE_TAG: its files are replaced"
  gh release upload "$RELEASE_TAG" "${FILES[@]}" --repo "$REPO" --clobber
  EDIT=(--title "$RELEASE_NAME")
  [ -z "$NOTES_FILE" ] || EDIT+=(--notes-file "$NOTES_FILE")
  case "$MODE" in
    channel) EDIT+=(--draft=false --prerelease --latest=false) ;;
    release) EDIT+=(--draft=false --prerelease=false --latest) ;;
  esac
  gh release edit "$RELEASE_TAG" --repo "$REPO" "${EDIT[@]}"
else
  # --verify-tag: without the tag GitHub would make one on the default branch.
  gh release create "$RELEASE_TAG" "${FILES[@]}" --repo "$REPO" --verify-tag \
    --title "$RELEASE_NAME" "${NOTES_ARGS[@]}" "${FLAGS[@]}"
fi

# What the release really holds.
STATE="$(gh release view "$RELEASE_TAG" --repo "$REPO" --json tagName,isDraft,isPrerelease,assets)"
jq -e --arg t "$RELEASE_TAG" '.tagName == $t and .isDraft == false' <<<"$STATE" >/dev/null \
  || fail "$REPO: the release $RELEASE_TAG is not published (a draft, or another tag)"
jq -e --argjson p "$PRERELEASE" '.isPrerelease == $p' <<<"$STATE" >/dev/null \
  || fail "$REPO: the release $RELEASE_TAG has prerelease != $PRERELEASE"
for file in "${FILES[@]}"; do
  name="$(basename "$file")"
  jq -e --arg n "$name" '[.assets[].name] | index($n) != null' <<<"$STATE" >/dev/null \
    || fail "$REPO: the release $RELEASE_TAG lacks $name"
done
# What /releases/latest answers may lag a moment behind the edit: asked a few times.
for attempt in 1 2 3 4 5; do
  LATEST="$(gh api "repos/$REPO/releases/latest" --jq .tag_name 2>/dev/null || true)"
  if [ "$MODE" = "channel" ] && [ "$LATEST" != "$RELEASE_TAG" ]; then break; fi
  if [ "$MODE" = "release" ] && [ "$LATEST" = "$RELEASE_TAG" ]; then break; fi
  [ "$attempt" -lt 5 ] && sleep 5
done
if [ "$MODE" = "channel" ]; then
  [ "$LATEST" != "$RELEASE_TAG" ] || fail "$REPO: the prerelease $RELEASE_TAG became the latest release"
else
  [ "$LATEST" = "$RELEASE_TAG" ] || fail "$REPO: the release $RELEASE_TAG is not what /releases/latest answers ($LATEST)"
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
echo ">> https://github.com/$REPO/releases/tag/$RELEASE_TAG ($MODE): ${#FILES[@]} files"
