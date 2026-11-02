# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# (l) The keeper's device is lost (spec §7.6): a new device of the owner
# makes a new root and an emergency generation and hands every reader of
# the snapshot, synthetic ones too, a bundle of it (ROO-397 §2, 4: the
# notes and their bytes through the relay); the live readers take theirs
# by their inboxes and read the new epoch.

scenario_l() {
  say "l: emergency generation"
  local from taken e rest
  from=$(lines "$DIR/keeper.log")
  keeper_cmd emergency
  taken=$(wait_line "$DIR/keeper.log" '.event == "emergency"' 3600 "$from")
  e=$(published)
  rest=$(swarm_cmd "expect bots=11-$((READERS - 1)) epoch=$e open=1 timeout=$((1800 * SLOW))" || true)
  local ok=false
  [ "$(jq -r .ok <<<"$taken")" = true ] && [ "$(jq -r .ok <<<"$rest")" = true ] && ok=true
  result l "$ok" "$(jq -cn --argjson t "$taken" --argjson r "$rest" --arg e "$e" '{emergency: $t, epoch: ($e|tonumber? // null), live_read: {ok: $r.ok, bad: $r.bad, to_good_s: $r.to_good_s, sample: $r.sample}}')"
}
