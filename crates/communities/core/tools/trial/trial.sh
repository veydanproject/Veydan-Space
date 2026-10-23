#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# The trial of the channels engine on the test servers (sub-stage 1.6):
#
#   crates/communities/core/tools/trial/trial.sh [a b c d e f g h]
#
# All scenarios by default, in order; a-g share one channel, one keeper and
# one swarm of $READERS readers (1000), h has its own channel. Results:
# $TRIAL_DIR/results.jsonl (default tmp/communities-trial); the folder of
# the bots is removed at the end unless KEEP=1 (a later run without a then
# goes on with the channel left there).
#
# Knobs: READERS; RATE and INBOXES, the joins started a second and in
# flight at once (lib.sh); JOIN_MINUTES; the waits grow with READERS.

. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
for s in a-join b-content c-ban d-keeper-off e-media f-snapshot g-restart h-from-join; do
  . "$TRIAL/$s.sh"
done

want=" ${*:-a b c d e f g h} "
build
mkdir -p "$DIR"
trap cleanup EXIT
say "relay $COMMUNITIES_TRIAL_RELAY, storage $COMMUNITIES_TRIAL_STORAGE, folder $DIR"
fail=0
run() { scenario_$1 || true; [ "$(tail -1 "$DIR/results.jsonl" | jq -r .ok)" = true ] || fail=1; }

if [[ $want == *" a "* ]]; then
  run a
elif [[ $want =~ [b-g] ]]; then
  start_keeper
  start_swarm --readers "$READERS" --minutes "$JOIN_MINUTES" --rate "$RATE" --inboxes "$INBOXES"
  wait_line "$DIR/swarm.log" '.ready' $((JOIN_MINUTES * 60 + 1800)) "$SWARM_FROM" >/dev/null
fi
for s in b c d e f g; do
  [[ $want == *" $s "* ]] && run $s
done
if [[ $want =~ [a-g] ]]; then
  swarm_cmd stats >"$DIR/swarm-stats.json" || true
  stop_swarm
  stop_keeper
  cli relay-stats >/dev/null || true
  jq -c 'select(.event == "stopped")' "$DIR/keeper.log" | tail -1 >"$DIR/keeper-stats.json" || true
fi
[[ $want == *" h "* ]] && run h
say "results: $OUT/results.jsonl"
jq -c '{scenario, ok}' "$DIR/results.jsonl" >&2
exit $fail
