# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# (m) Readers away (ROO-397 §2, 5): $AWAY live readers go off the network
# for $AWAY_MINUTES minutes (60 by default) while the admin posts and the
# epoch turns; back, each catches up on its own: the time to the newest
# epoch (p50/p95/p99), the GETs and events it took, and the posts.

scenario_m() {
  local k=${AWAY:-50} m=${AWAY_MINUTES:-60}
  local a=$((READERS - k)) b=$((READERS - 1))
  say "m: $k readers away for $m minutes"
  local off posts e0 e1 back seen
  off=$(swarm_cmd "offline bots=$a-$b" || true)
  sleep 5
  posts=$(cli post --count 3 --marker "away-post-$RUN" | tail -1)
  e0=$(published); e1=$((e0 + 1))
  cli rotate-now >/dev/null
  wait_line "$DIR/keeper.log" ".state.published == $e1" 600 "$KEEPER_FROM" >/dev/null || true
  sleep $(( m * 60 > 60 ? m * 60 - 60 : 0 ))
  back=$(swarm_cmd "online bots=$a-$b timeout=$((900 * SLOW))" || true)
  seen=$(swarm_cmd "expect bots=$a-$b posts=3 post-marker=away-post-$RUN timeout=300" || true)
  local ok=false
  [ "$(jq -r .sent <<<"$posts")" = 3 ] && [ "$(jq -r .ok <<<"$back")" = true ] && [ "$(jq -r '.target_epoch >= '"$e1" <<<"$back")" = true ] && [ "$(jq -r .ok <<<"$seen")" = true ] && ok=true
  result m "$ok" "$(jq -cn --argjson o "$off" --argjson b "$back" --argjson s "$seen" --arg m "$m" '{away: $o.bots, minutes: ($m|tonumber), back: $b, posts_read: {ok: $s.ok, bad: $s.bad}}')"
}
