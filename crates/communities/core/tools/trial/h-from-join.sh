# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# (h) A channel with history from_join: 5 readers (each reads from the
# first epoch after its join, within the join window of 5 minutes), 3
# posts, two rotations; a late reader holds no key of the epochs before its join and reads none
# of the old posts, then reads the next post. Its own folder, keeper and
# swarm (the connections of one address are few).

scenario_h() {
  say "h: from_join"
  local main=$DIR
  DIR=$main/from-join; mkdir -p "$DIR"
  local created join e late check new seen
  created=$(cli create --name "Trial from_join" --history from_join | jq -c 'select(.event == "created")')
  start_keeper
  start_swarm --readers 5 --minutes 10 --rate 5
  join=$(wait_line "$DIR/swarm.log" '.phase == "join"' 900 "$SWARM_FROM")
  wait_line "$DIR/swarm.log" '.ready' 900 "$SWARM_FROM" >/dev/null
  # The first epoch after the joins (the join window): the admin, a
  # newcomer too, holds no key before it.
  wait_line "$DIR/keeper.log" '.state.published == 2' 900 "$KEEPER_FROM" >/dev/null
  cli post --count 3 --marker old-post >/dev/null
  swarm_cmd "expect bots=0-4 posts=3 post-marker=old-post timeout=900" >/dev/null || true
  for _ in 1 2; do
    e=$(published)
    cli rotate-now >/dev/null
    wait_line "$DIR/keeper.log" ".state.published == $((e + 1))" 600 "$KEEPER_FROM" >/dev/null || true
  done
  e=$(published)
  late=$(swarm_cmd "join n=1 timeout=200" || true)
  cli rotate-now >/dev/null
  wait_line "$DIR/keeper.log" ".state.published == $((e + 1))" 600 "$KEEPER_FROM" >/dev/null || true
  check=$(swarm_cmd "expect bots=5 key=1 posts=0 post-marker=old-post min-epoch=$((e + 1)) timeout=300" || true)
  cli post --count 1 --marker new-post >/dev/null
  seen=$(swarm_cmd "expect bots=0-5 posts=1 post-marker=new-post timeout=300" || true)
  stop_swarm; stop_keeper
  local ok=false
  [ "$(jq -r .ok <<<"$late")" = true ] && [ "$(jq -r .ok <<<"$check")" = true ] && [ "$(jq -r .ok <<<"$seen")" = true ] && ok=true
  DIR=$main
  result h "$ok" "$(jq -cn --argjson c "$created" --argjson j "$join" --argjson l "$late" --argjson k "$check" --argjson s "$seen" --arg e "$e"     '{create_secs: $c.secs, join: $j, epoch_at_late_join: ($e|tonumber), late_join: $l, late_reader_old: {ok: $k.ok, sample: $k.sample}, new_post_read: {ok: $s.ok, bad: $s.bad}}')"
}
