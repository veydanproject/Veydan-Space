# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# (b) 50 posts by the admin, 500 comments by the readers; every reader
# holds all of them (a comment of a new reader waits for the snapshot
# that names it: up to 5 minutes).

scenario_b() {
  say "b: 50 posts, 500 comments"
  local posts seen sent all t0=$SECONDS
  posts=$(cli post --count 50 --marker "$PM" | tail -1)
  seen=$(swarm_cmd "expect posts=50 post-marker=$PM timeout=$((600 * SCALE))" || true)
  sent=$(swarm_cmd "comment n=500 rate=8 marker=$CM post-marker=$PM" || true)
  local n; n=$(jq -r .sent <<<"$sent")
  all=$(WAIT=$((3600 * SCALE)) swarm_cmd "expect comments=$n comment-marker=$CM timeout=$((1500 * SCALE))" || true)
  local ok=false
  [ "$(jq -r .sent <<<"$posts")" = 50 ] && [ "$(jq -r .ok <<<"$seen")" = true ] && [ "$n" -ge 495 ] && [ "$(jq -r .ok <<<"$all")" = true ] && ok=true
  result b "$ok" "$(jq -cn --argjson p "$posts" --argjson s "$seen" --argjson c "$sent" --argjson a "$all" --arg t "$((SECONDS - t0))"     '{posts: $p, posts_seen: {ok: $s.ok, bad: $s.bad, not_in: $s.not_in, secs: $s.secs}, comments_sent: $c, comments_seen: {ok: $a.ok, bad: $a.bad, not_in: $a.not_in, secs: $a.secs, sample: $a.sample}, secs: ($t|tonumber)}')"
}
