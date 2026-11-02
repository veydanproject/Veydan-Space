# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# (j) A wave of bans (ROO-397 §2, 2): the owner bans $WAVE synthetic
# readers over $WAVE_MINUTES minutes (2000 in an hour by default), spread
# over the tree; from each ban to the first epoch that leaves it out
# (p50/p95/p99), the epochs and their bytes; the live readers hold the
# newest epoch after it, and the time each took.

scenario_j() {
  local n=${WAVE:-2000} m=${WAVE_MINUTES:-60}
  [ "$n" -le "$SYNTHETIC" ] || n=$SYNTHETIC
  if [ "$n" = 0 ]; then
    result j true '{"skipped": "no synthetic readers to ban"}'
    return 0
  fi
  say "j: $n bans in $m minutes"
  local from wave e rest
  from=$(lines "$DIR/keeper.log")
  keeper_cmd "ban-wave n=$n minutes=$m"
  wave=$(wait_line "$DIR/keeper.log" '.event == "ban_wave"' $(( m * 60 + 1800 )) "$from")
  e=$(published)
  rest=$(swarm_cmd "expect bots=11-$((READERS - 1)) epoch=$e timeout=$((900 * SLOW))" || true)
  local ok=false
  [ "$(jq -r .ok <<<"$wave")" = true ] && [ "$(jq -r .ok <<<"$rest")" = true ] && ok=true
  result j "$ok" "$(jq -cn --argjson w "$wave" --argjson r "$rest" --arg e "$e" '{wave: $w, epoch: ($e|tonumber? // null), live_read: {ok: $r.ok, bad: $r.bad, to_good_s: $r.to_good_s, sample: $r.sample}}')"
}
