# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# (f) The snapshot of the readers is on the relay; a fresh reader that
# never saw anybody join takes the comments of the readers by the shards
# of the snapshot, as many as an old reader holds, but those of the
# readers banned since (0-10): the fresh one refuses them, the old one
# keeps what it took before the ban.

scenario_f() {
  say "f: a fresh reader judges commenters by the shards"
  local stats joined i loaded same shards
  stats=$(cli relay-stats | tail -1)
  joined=$(swarm_cmd "join n=1 timeout=200" || true)
  i=$(jq -r .first <<<"$joined")
  loaded=$(swarm_cmd "load bots=$i" || true)
  same=$(swarm_cmd "expect bots=$i same-as=20 except=0-10 post-marker=$PM comment-marker=$CM timeout=300" || true)
  shards=$(swarm_cmd "shards bots=$i" || true)
  local ok=false
  [ "$(jq -r '.kinds.snapshots.events > 0' <<<"$stats")" = true ] && [ "$(jq -r .ok <<<"$joined")" = true ] && [ "$(jq -r .ok <<<"$same")" = true ] \
    && [ "$(jq -r '.bots[0].cached > 0' <<<"$shards")" = true ] && ok=true
  result f "$ok" "$(jq -cn --argjson s "$stats" --argjson j "$joined" --argjson l "$loaded" --argjson m "$same" --argjson h "$shards"     '{snapshots_on_relay: $s.kinds.snapshots, fresh_join: $j, load: $l, same_as_old_reader: {ok: $m.ok, check: $m.check, sample: $m.sample}, shards: $h.bots[0]}')"
}
