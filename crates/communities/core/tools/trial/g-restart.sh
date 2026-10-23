# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# (g) The keeper restarts from its rows: in the process (the service made
# anew) and as a new process; the state is the one it had, and the next
# epoch goes on from it.

scenario_g() {
  say "g: keeper restart"
  local from inproc loaded e0 e1 rot rest
  from=$(lines "$DIR/keeper.log")
  keeper_cmd restart
  inproc=$(wait_line "$DIR/keeper.log" '.event == "restart"' 120 "$from")
  stop_keeper
  from=$(lines "$DIR/keeper.log")
  start_keeper
  loaded=$(wait_line "$DIR/keeper.log" '.event == "loaded"' 120 "$from")
  e0=$(published); e1=$((e0 + 1))
  rot=$(cli rotate-now | tail -1)
  wait_line "$DIR/keeper.log" ".state.published == $e1" 600 "$KEEPER_FROM" >/dev/null || true
  rest=$(swarm_cmd "expect bots=11-$((READERS - 1)) epoch=$e1 timeout=$((900 * SCALE))" || true)
  local ok=false
  [ "$(jq -r .equal <<<"$inproc")" = true ] && [ "$(jq -r .equal_to_saved <<<"$loaded")" = true ] && [ "$(jq -r .ok <<<"$rest")" = true ] && ok=true
  result g "$ok" "$(jq -cn --argjson i "$inproc" --argjson l "$loaded" --argjson r "$rot" --argjson s "$rest" --arg e "$e1"     '{in_process: $i, new_process: $l, rotate: $r, epoch: ($e|tonumber), readers: {ok: $s.ok, bad: $s.bad, secs: $s.secs, sample: $s.sample}}')"
}
