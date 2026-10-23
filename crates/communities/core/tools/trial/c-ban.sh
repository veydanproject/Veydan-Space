# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# (c) The admin bans readers 0-9: the journal acts at once (their comments
# are refused by their own app, and sealed by hand past it, by every other
# reader); an urgent rotation gives a new epoch: the banned are out, the
# rest read it.

scenario_c() {
  say "c: ban 10, rotate now"
  local pks ban own forged refused e0 rot e1 out rest
  pks=$(pubkeys 0-9)
  e0=$(published)
  ban=$(cli ban $pks | tail -1)
  sleep 5
  own=$(swarm_cmd "comment n=10 bots=0-9 marker=after-ban-$RUN" || true)
  forged=$(swarm_cmd "forge bots=0-9 marker=forged-ban-$RUN" || true)
  sleep 20
  refused=$(swarm_cmd "expect bots=10-$((READERS - 1)) comments=0 comment-marker=forged-ban-$RUN timeout=60" || true)
  rot=$(cli rotate-now | tail -1)
  e1=$((e0 + 1))
  wait_line "$DIR/keeper.log" ".state.published == $e1" 600 "$KEEPER_FROM" >/dev/null || true
  rest=$(swarm_cmd "expect bots=10-$((READERS - 1)) epoch=$e1 timeout=$((900 * SCALE))" || true)
  out=$(swarm_cmd "expect bots=0-9 membership=left no-epoch=$e1 timeout=300" || true)
  local ok=false
  [ "$(jq -r .ok <<<"$ban")" = true ] && [ "$(jq -r ".done | length" <<<"$ban")" = 10 ] && [ "$(jq -r .refused <<<"$own")" = 10 ] && [ "$(jq -r .sent <<<"$forged")" = 10 ] \
    && [ "$(jq -r .ok <<<"$refused")" = true ] && [ "$(jq -r .ok <<<"$rest")" = true ] && [ "$(jq -r .ok <<<"$out")" = true ] && ok=true
  result c "$ok" "$(jq -cn --argjson b "$ban" --argjson o "$own" --argjson f "$forged" --argjson r "$refused" --argjson x "$rot" --argjson s "$rest" --argjson u "$out" --arg e "$e1"     '{ban: {ok: $b.ok, ops: ($b.done | map(.ms))}, own_app_refused: $o, forged: $f, forged_refused_by_others: {ok: $r.ok, bad: $r.bad}, rotate: $x, epoch: ($e|tonumber), rest_read: {ok: $s.ok, bad: $s.bad, secs: $s.secs, sample: $s.sample}, banned_out: {ok: $u.ok, bad: $u.bad, sample: $u.sample}}')"
}
