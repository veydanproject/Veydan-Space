# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# (d) The keeper is off for ${OFF_MINUTES:-10} minutes: posts and a ban
# keep working by the journal; 20 newcomers wait and get their keys
# within 2 minutes after the keeper is back.

scenario_d() {
  local off=${OFF_MINUTES:-10}
  say "d: keeper off for $off minutes"
  local from paused posts seen pk ban forged refused joined waiting resumed back n first
  from=$(lines "$DIR/keeper.log")
  keeper_cmd "pause $off"
  paused=$(wait_line "$DIR/keeper.log" '.event == "paused"' 60 "$from")
  posts=$(cli post --count 5 --marker "off-post-$RUN" | tail -1)
  seen=$(swarm_cmd "expect bots=10-$((READERS - 1)) posts=5 post-marker=off-post-$RUN timeout=300" || true)
  pk=$(pubkeys 10)
  ban=$(cli ban $pk | tail -1)
  sleep 5
  forged=$(swarm_cmd "forge bots=10 marker=forged-off-$RUN" || true)
  sleep 20
  refused=$(swarm_cmd "expect bots=11-$((READERS - 1)) comments=0 comment-marker=forged-off-$RUN timeout=60" || true)
  # The joins wait for the keeper: asked for no longer than it is off
  # (OFF_MINUTES=3 is shorter than the 200 s a join is given).
  joined=$(swarm_cmd "join n=20 rate=5 timeout=$(( off * 60 - 60 < 200 ? off * 60 - 60 : 200 ))" || true)
  first=$(jq -r .first <<<"$joined"); n=$((first + 19))
  waiting=$(swarm_cmd "expect bots=$first-$n all=1 key=0 timeout=5" || true)
  resumed=$(wait_line "$DIR/keeper.log" '.event == "resumed"' $((off * 60 + 120)) "$from")
  back=$(swarm_cmd "inbox bots=$first-$n timeout=120" || true)
  local ok=false
  [ "$(jq -r .ok <<<"$seen")" = true ] && [ "$(jq -r .ok <<<"$ban")" = true ] && [ "$(jq -r .ok <<<"$refused")" = true ] \
    && [ "$(jq -r .ok <<<"$waiting")" = true ] && [ "$(jq -r .ok <<<"$back")" = true ] && ok=true
  result d "$ok" "$(jq -cn --argjson p "$paused" --argjson s "$seen" --argjson b "$ban" --argjson r "$refused" --argjson j "$joined" --argjson w "$waiting" --argjson q "$resumed" --argjson k "$back"     '{paused: $p, posts_while_off: {ok: $s.ok, bad: $s.bad, secs: $s.secs}, ban_while_off: $b.ok, forged_refused: {ok: $r.ok, bad: $r.bad}, newcomers_while_off: {asked: $j.asked, admitted: $j.admitted}, newcomers_wait: {ok: $w.ok, bad: $w.bad}, resumed: $q, keys_after_return: $k}')"
  SWARM_FIRST_FREE=$((n + 1))
}
