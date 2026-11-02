# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# (i) The stage past the live bots (SCALE): $SYNTHETIC readers without a
# device come by the link at once and the keeper lets them in as fast as
# it takes them (JOINS_PER_MINUTE; the app's 120 by default); each issue
# is a note on the relay, each epoch after is built and uploaded for
# them. From their coming to the issue on the relay: p50/p95/p99. The
# live readers read on.

scenario_i() {
  local n=$SYNTHETIC
  if [ "$n" = 0 ]; then
    result i true '{"skipped": "SCALE is the live readers: no synthetic ones"}'
    return 0
  fi
  say "i: $n synthetic readers join"
  local from done jpm members e rest
  jpm=${JOINS_PER_MINUTE:-120}
  from=$(lines "$DIR/keeper.log")
  keeper_cmd "synthetic n=$n"
  done=$(wait_line "$DIR/keeper.log" '.event == "synthetic_done"' $(( n * 60 / jpm + 1800 )) "$from")
  keeper_cmd state; sleep 3
  members=$(jq -r 'select(.event == "state") | .state.members' "$DIR/keeper.log" | tail -1)
  e=$(published)
  rest=$(swarm_cmd "expect bots=0-$((READERS - 1)) epoch=$e timeout=$((900 * SLOW))" || true)
  local ok=false
  [ "$(jq -r .issued <<<"$done")" = "$n" ] && [ "$(jq -r .issues_waiting <<<"$done")" = 0 ] && [ "$members" = "$((READERS + 2 + n))" ] && [ "$(jq -r .ok <<<"$rest")" = true ] && ok=true
  result i "$ok" "$(jq -cn --argjson d "$done" --arg m "$members" --argjson r "$rest" --arg j "$jpm" \
    '{synthetic: $d, joins_per_minute: ($j|tonumber), keeper_members: ($m|tonumber? // null), live_read: {ok: $r.ok, bad: $r.bad, to_good_s: $r.to_good_s, sample: $r.sample}}')"
}
