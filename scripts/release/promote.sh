#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# The last job of the release workflow of Gitea (veydanproject/release;
# docs/ci-cd.md, "Releases"): what was built becomes a published release.
#
#   KIND=dev       a DEV build: the Linux bundles and the APK Gitea built
#                  (ASSETS_DIR), every updater signature checked against the
#                  public key of the product, published on the releases page
#                  of the monorepo on Gitea under RELEASE_TAG
#                  (scripts/release/gitea.sh). Nothing goes to GitHub.
#   KIND=rc        a release of the product's repository on GitHub under
#   KIND=release   RELEASE_TAG (vX.Y.Z-rc.N or vX.Y.Z, the tag the snapshot
#                  brought there): Linux and Android are what this run built
#                  (ASSETS_DIR); Windows and macOS come from the DRAFT
#                  release GitHub builds under the tag (DRAFT_KEYS: the
#                  platforms it must hold; waited for). Every signature is
#                  checked, latest.json written over every platform
#                  (UPDATER_KEYS), and the release published with
#                  SHA256SUMS: an rc as a PRE-RELEASE (never the latest, the
#                  updater never sees it), a release as THE latest.
#
#   REPO, RELEASE_TAG, RELEASE_NAME, ASSET_PREFIX, VERSION, UPDATER_KEYS,
#   UPDATER_PUBKEY        as scripts/release/meta.sh gives them
#   GH_TOKEN              rc, release: a token of GitHub that may write releases in REPO
#   GITEA_TOKEN           dev: the releases of the monorepo
#   NOTES_FILE            rc, release: the release note of the version at the tag
#                         (apps/<product>/release-notes/X.Y.Z.md; meta.sh's notes_file):
#                         the text of the release and of latest.json. Empty: a
#                         line that points at the files (a tag made before the notes)
#   ASSETS_DIR            what this run built (may be empty)
#   WAIT_MINUTES          how long to wait for the draft (60)
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

fail() {
  echo "::error::$*"
  exit 1
}

: "${KIND:?}" "${RELEASE_TAG:?}" "${RELEASE_NAME:?}" "${ASSET_PREFIX:?}" "${VERSION:?}" "${UPDATER_PUBKEY:?}"
UPDATER_KEYS="${UPDATER_KEYS:-}"
DRAFT_KEYS="${DRAFT_KEYS:-}"
ASSETS_DIR="${ASSETS_DIR:-release-assets}"
WAIT_MINUTES="${WAIT_MINUTES:-60}"
mkdir -p "$ASSETS_DIR"

verify() {
  node "$HERE/assets.mjs" verify --dir "$1" --prefix "$ASSET_PREFIX" --version "$VERSION" --keys "$2" --pubkey "$UPDATER_PUBKEY"
}

case "$KIND" in
  dev)
    verify "$ASSETS_DIR" "$UPDATER_KEYS"
    export RELEASE_TAG RELEASE_NAME ASSETS_DIR
    bash "$HERE/gitea.sh" publish
    exit 0
    ;;
  rc) MODE=channel ;;
  release) MODE=release ;;
  *) fail "KIND=$KIND: dev, rc or release" ;;
esac

: "${REPO:?}"
[ -n "${GH_TOKEN:-}" ] || fail "No token to publish $RELEASE_TAG in $REPO with"

# The text of the release: the note of the version, an rc says what it is.
NOTES_FILE="${NOTES_FILE:-}"
if [ -n "$NOTES_FILE" ]; then
  [ -f "$NOTES_FILE" ] || fail "$NOTES_FILE: the release note of $VERSION is missing at the tag (make push refuses an rc or a release without it)"
  BODY="$(mktemp)"
  node "$HERE/notes.mjs" body "$NOTES_FILE" --kind "$KIND" > "$BODY" || fail "$NOTES_FILE: the note does not follow the rules (node scripts/release/notes.mjs check)"
  NOTES_FILE="$BODY"
fi

# Windows and macOS: the draft GitHub builds under the tag.
download_draft() {
  local id="$1" dir="$2" n aid
  mkdir -p "$dir"
  while IFS=$'\t' read -r n aid; do
    [ -n "$n" ] || continue
    case "$n" in latest.json|SHA256SUMS) continue ;; esac
    gh api -H "Accept: application/octet-stream" "repos/$REPO/releases/assets/$aid" > "$dir/$n"
  done <<<"$(gh api "repos/$REPO/releases/$id" --jq '.assets[] | "\(.name)\t\(.id)"')"
}
if [ -n "$DRAFT_KEYS" ]; then
  echo ">> waiting for the draft $RELEASE_TAG of $REPO with $DRAFT_KEYS (up to $WAIT_MINUTES min)"
  deadline=$(( $(date +%s) + WAIT_MINUTES * 60 ))
  while :; do
    ID="$(gh api "repos/$REPO/releases?per_page=50" --jq ".[] | select(.draft == true and .tag_name == \"$RELEASE_TAG\") | .id" 2>/dev/null | head -n1 || true)"
    if [ -n "$ID" ]; then
      rm -rf "$ASSETS_DIR/.draft" && download_draft "$ID" "$ASSETS_DIR/.draft"
      if verify "$ASSETS_DIR/.draft" "$DRAFT_KEYS" 2>/dev/null; then
        break
      fi
      echo ">> the draft is there but not complete yet"
    fi
    [ "$(date +%s)" -lt "$deadline" ] || fail "$REPO: no complete draft $RELEASE_TAG within $WAIT_MINUTES minutes (did the build on GitHub fail? its run is in Actions of $REPO)"
    sleep 60
  done
  verify "$ASSETS_DIR/.draft" "$DRAFT_KEYS"
  mv "$ASSETS_DIR"/.draft/* "$ASSETS_DIR"/ && rmdir "$ASSETS_DIR/.draft"
fi

# Every platform together, then the pre-release (rc) or THE release.
verify "$ASSETS_DIR" "$UPDATER_KEYS"
ls -la "$ASSETS_DIR"
export MODE NOTES_FILE REPO RELEASE_TAG RELEASE_NAME ASSET_PREFIX VERSION UPDATER_KEYS ASSETS_DIR
bash "$HERE/publish.sh"
(cd "$ASSETS_DIR" && rm -f SHA256SUMS latest.published.json && sha256sum -- * > SHA256SUMS) && gh release upload "$RELEASE_TAG" "$ASSETS_DIR/SHA256SUMS" --repo "$REPO" --clobber
echo ">> published ($KIND): https://github.com/$REPO/releases/tag/$RELEASE_TAG"
