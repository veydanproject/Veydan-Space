# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# (a) A channel, $READERS readers join by its link (at most 10 waiting at
# once: each waits on a connection of its own), the keeper issues every
# bundle, every reader holds the key of the first epoch. Sourced by trial.sh.

scenario_a() {
  say "a: create, $READERS readers join"
  local created join ready check members
  created=$(cli create --name "Trial $(date +%F)" | jq -c 'select(.event == "created")')
  [ -n "$created" ] || { result a false null; return 1; }
  start_keeper
  start_swarm --readers "$READERS" --minutes "$JOIN_MINUTES" --rate "$RATE" --inboxes "$INBOXES"
  join=$(wait_line "$DIR/swarm.log" '.phase == "join"' $((JOIN_MINUTES * 60 + 600)) "$SWARM_FROM")
  ready=$(wait_line "$DIR/swarm.log" '.ready' $((1800 * SCALE)) "$SWARM_FROM")
  check=$(swarm_cmd "expect key=1 min-epoch=1 timeout=120" || true)
  keeper_cmd state; sleep 3
  members=$(jq -r 'select(.event == "state") | .state.members' "$DIR/keeper.log" | tail -1)
  local ok=false
  [ "$(jq -r .ok <<<"$join")" = true ] && [ "$(jq -r .ok <<<"$check")" = true ] && [ "$members" = "$((READERS + 2))" ] && ok=true
  result a "$ok" "$(jq -cn --argjson c "$created" --argjson j "$join" --argjson k "$check" --arg m "$members" --argjson s "$(swarm_cmd status)"     '{create_secs: $c.secs, join: $j, check: {ok: $k.ok, bad: $k.bad, sample: $k.sample}, keeper_members: ($m|tonumber), swarm: $s}')"
}
