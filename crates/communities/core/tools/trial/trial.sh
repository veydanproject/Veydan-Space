#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# The trial of the channels engine on the test servers (sub-stage 1.6) and
# the stand of the load test (ROO-397, D4), on test01 by default:
#
#   crates/communities/core/tools/trial/trial.sh [a b c d e f g h i j k l m]
#
# All scenarios by default, in this order: a i b c d e f g m j k l share
# one channel, one keeper and one swarm of $READERS live readers (1000),
# h has its own channel. a: joins; i: synthetic readers up to SCALE; b:
# posts and comments; c: bans; d: the keeper off; e: a file; f: the
# snapshot; g: restarts; m: readers away and back; j: a wave of bans; k:
# the keeper to the owner's other device; l: an emergency generation; h:
# history from_join. Results: $TRIAL_DIR/results.jsonl (default
# tmp/communities-trial), the traffic of the run and what the servers
# were before and after in its last lines; at the end the blobs and the
# events of the run are deleted on the servers and the folder of the bots
# is removed, unless KEEP=1 (a later run without a then goes on with the
# channel left there).
#
# Knobs (lib.sh): READERS, the live bots; SCALE (10k, 100k, 1m), the
# readers of the stage; JOINS_PER_MINUTE and MAX_READERS, the keeper's
# limits on the stand only; RATE and INBOXES, the joins of the swarm;
# JOIN_MINUTES; TRAFFIC_LIMIT_MB, where the run stops; WAVE and
# WAVE_MINUTES (j), AWAY and AWAY_MINUTES (m), OFF_MINUTES (d); the
# relay and blob hosts by COMMUNITIES_TRIAL_RELAY and _STORAGE. The stand
# does not start while QA or CI runs (preflight).

. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
for s in a-join b-content c-ban d-keeper-off e-media f-snapshot g-restart h-from-join i-synthetic j-ban-wave k-handover l-emergency m-offline; do
  . "$TRIAL/$s.sh"
done

# Every bot holds its database files and sockets: a thousand of them
# are past the soft limit of a unit of systemd (1024).
ulimit -n "$(ulimit -Hn)" 2>/dev/null || true
want=" ${*:-a b c d e f g h i j k l m} "
preflight || exit 4
build
mkdir -p "$DIR"
SERVER_BEFORE=$(server_look)
trap cleanup EXIT
trap 'exit 130' INT TERM
# A process gone (its fifo without a reader) fails a command, not the run.
trap '' PIPE
say "relay $COMMUNITIES_TRIAL_RELAY, storage $COMMUNITIES_TRIAL_STORAGE, folder $DIR, $READERS live readers, stage $SCALE ($SYNTHETIC synthetic), traffic limit $TRAFFIC_LIMIT_MB MB"
fail=0
run() {
  scenario_$1 || true
  [ "$(tail -1 "$DIR/results.jsonl" | jq -r .ok)" = true ] || fail=1
  if ! traffic_ok; then
    result traffic_limit false "$(traffic)"
    say "the traffic of the run reached $TRAFFIC_LIMIT_MB MB: stopped"
    exit 3
  fi
}

if [[ $want == *" a "* ]]; then
  run a
  # Without the channel, its keeper and its swarm nothing else can run.
  [ "$fail" = 0 ] || exit 1
elif [[ $want =~ [b-gi-m] ]]; then
  start_keeper
  start_swarm --readers "$READERS" --minutes "$JOIN_MINUTES" --rate "$RATE" --inboxes "$INBOXES"
  wait_line "$DIR/swarm.log" '.ready' $((JOIN_MINUTES * 60 + 1800)) "$SWARM_FROM" >/dev/null
fi
for s in i b c d e f g m j k l; do
  [[ $want == *" $s "* ]] && run $s
done
if [[ $want =~ [a-gi-m] ]]; then
  swarm_cmd stats >"$DIR/swarm-stats.json" || true
  keeper_cmd stats; sleep 3
  stop_swarm
  stop_keeper
  cli relay-stats >/dev/null || true
  jq -c 'select(.event == "stopped")' "$DIR/keeper.log" | tail -1 >"$DIR/keeper-stats.json" || true
fi
[[ $want == *" h "* ]] && run h
exit $fail
