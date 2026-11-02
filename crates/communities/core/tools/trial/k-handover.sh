# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# (k) The keeper to the owner's other device (wire §15): a device of the
# owner in a folder of its own takes it, the first one hands its state
# over; the take, the hand-over, the first epoch of the new series, and
# the live readers read on with the bundles they hold (no new one).

scenario_k() {
  say "k: the keeper to the owner's other device"
  local from moved e rest post seen
  from=$(lines "$DIR/keeper.log")
  keeper_cmd handover
  moved=$(wait_line "$DIR/keeper.log" '.event == "handover"' 2400 "$from")
  e=$(published)
  rest=$(swarm_cmd "expect bots=11-$((READERS - 1)) epoch=$e timeout=$((900 * SLOW))" || true)
  post=$(cli post --count 1 --marker "after-handover-$RUN" | tail -1)
  seen=$(swarm_cmd "expect bots=11-$((READERS - 1)) posts=1 post-marker=after-handover-$RUN timeout=$((600 * SLOW))" || true)
  local ok=false
  [ "$(jq -r .ok <<<"$moved")" = true ] && [ "$(jq -r .ok <<<"$rest")" = true ] && [ "$(jq -r .ok <<<"$seen")" = true ] && ok=true
  result k "$ok" "$(jq -cn --argjson h "$moved" --argjson r "$rest" --argjson p "$post" --argjson s "$seen" --arg e "$e" \
    '{handover: $h, epoch: ($e|tonumber? // null), live_read: {ok: $r.ok, bad: $r.bad, to_good_s: $r.to_good_s, sample: $r.sample}, post: $p.sent, post_read: {ok: $s.ok, bad: $s.bad}}')"
}
