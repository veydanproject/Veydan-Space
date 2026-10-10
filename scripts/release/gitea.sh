#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# The releases page of the monorepo on Gitea, where the dev builds live
# (docs/ci-cd.md, "Releases"):
#
#   scripts/release/gitea.sh publish    RELEASE_TAG RELEASE_NAME ASSETS_DIR
#     a prerelease under RELEASE_TAG (a tag of the monorepo: the tag must
#     exist) with every file of ASSETS_DIR and their SHA256SUMS. A run started
#     again replaces the files.
#
#   GITEA_API      https://git.veydan.net/api/v1 by default
#   GITEA_REPO     veydanproject/monorepo by default
#   GITEA_TOKEN    a token that may write the releases of GITEA_REPO
set -euo pipefail

fail() {
  echo "::error::$*"
  exit 1
}

API="${GITEA_API:-https://git.veydan.net/api/v1}"
REPO="${GITEA_REPO:-veydanproject/monorepo}"
[ -n "${GITEA_TOKEN:-}" ] || fail "No token of Gitea to reach the releases of $REPO"
AUTH=(-H "Authorization: token $GITEA_TOKEN")

# curl that fails on an HTTP error and says which.
call() {
  local out code
  out="$(mktemp)"
  code="$(curl -sS -o "$out" -w '%{http_code}' "${AUTH[@]}" "$@")" || { rm -f "$out"; fail "curl $*"; }
  if [ "${code:0:1}" != 2 ]; then
    echo "::error::$* → HTTP $code: $(head -c 300 "$out")" >&2
    rm -f "$out"
    return 1
  fi
  cat "$out"
  rm -f "$out"
}

release_of() {
  curl -s "${AUTH[@]}" "$API/repos/$REPO/releases/tags/$1"
}

case "${1:-}" in
  publish)
    : "${RELEASE_TAG:?}" "${RELEASE_NAME:?}" "${ASSETS_DIR:?}"
    [ -d "$ASSETS_DIR" ] && [ -n "$(ls -A "$ASSETS_DIR")" ] || fail "$ASSETS_DIR: nothing to publish"
    (cd "$ASSETS_DIR" && rm -f SHA256SUMS && sha256sum -- * > SHA256SUMS)
    REL="$(release_of "$RELEASE_TAG")"
    ID="$(jq -r '.id // empty' <<<"$REL")"
    if [ -z "$ID" ]; then
      BODY="$(jq -nc --arg t "$RELEASE_TAG" --arg n "$RELEASE_NAME" \
        '{tag_name: $t, name: $n, body: "A dev build: Linux and Android, not published on GitHub. Install it by hand to test.", draft: false, prerelease: true}')"
      ID="$(call -X POST -H "Content-Type: application/json" "$API/repos/$REPO/releases" -d "$BODY" | jq -r .id)"
      echo ">> $REPO: the release $RELEASE_TAG made (#$ID)"
    else
      echo ">> $REPO already has the release $RELEASE_TAG (#$ID): its files are replaced"
    fi
    EXISTING="$(call "$API/repos/$REPO/releases/$ID/assets")"
    for file in "$ASSETS_DIR"/*; do
      name="$(basename "$file")"
      old="$(jq -r --arg n "$name" '.[] | select(.name == $n) | .id' <<<"$EXISTING")"
      [ -z "$old" ] || call -X DELETE "$API/repos/$REPO/releases/$ID/assets/$old" >/dev/null
      call -X POST -F "attachment=@$file" "$API/repos/$REPO/releases/$ID/assets?name=$name" >/dev/null
      echo "   $name"
    done
    # What the release really holds.
    HAS="$(call "$API/repos/$REPO/releases/$ID/assets" | jq -r '.[].name')"
    for file in "$ASSETS_DIR"/*; do
      grep -qxF "$(basename "$file")" <<<"$HAS" || fail "$REPO: the release $RELEASE_TAG lacks $(basename "$file")"
    done
    echo ">> ${API%/api/v1}/$REPO/releases/tag/$RELEASE_TAG: $(wc -l <<<"$HAS") files"
    ;;
  *) fail "usage: scripts/release/gitea.sh publish" ;;
esac
