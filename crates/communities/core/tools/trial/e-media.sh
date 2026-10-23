# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# (e) A file of 5 MB posted by the admin, fetched by 20 readers: the same
# hash.

scenario_e() {
  say "e: a 5 MB file to 20 readers"
  local file="$DIR/media.bin" posted sha fetched
  head -c $((5 * 1024 * 1024)) /dev/urandom >"$file"
  posted=$(cli post --file "$file" --marker "media-$RUN" | tail -1)
  sha=$(jq -r .sha256 <<<"$posted")
  swarm_cmd "expect bots=20-39 posts=1 post-marker=media-$RUN timeout=300" >/dev/null || true
  fetched=$(swarm_cmd "fetch bots=20-39 post-marker=media-$RUN sha=$sha" || true)
  local ok=false
  [ "$(jq -r .ok <<<"$posted")" = true ] && [ "$(jq -r .ok <<<"$fetched")" = true ] && ok=true
  result e "$ok" "$(jq -cn --argjson p "$posted" --argjson f "$fetched" '{posted: $p, fetched: $f}')"
  rm -f "$file"
}
