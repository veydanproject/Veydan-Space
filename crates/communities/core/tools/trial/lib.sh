# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# Shared by the scenarios of the trial (internal/communities-stage1.md §7,
# row 1.6): the binary, the network, the keeper and the swarm as long
# processes driven through fifos, the results as JSON lines. Sourced.
#
# The network comes from the environment: COMMUNITIES_TRIAL_RELAY,
# COMMUNITIES_TRIAL_RELAY_KEY, COMMUNITIES_TRIAL_STORAGE (comma-separated
# blob hosts). Unset, they are read from the manifest the app ships with
# (its first relay with its key, every media host of it).

set -uo pipefail

ROOT=$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)
TRIAL=$ROOT/crates/communities/core/tools/trial
MANIFEST=$ROOT/crates/messenger/transport/manifest/embedded.json
: "${CARGO_TARGET_DIR:=$ROOT/target}"
BIN=${COMMUNITIES_CLI:-$CARGO_TARGET_DIR/release/communities-cli}
DIR=${TRIAL_DIR:-$ROOT/tmp/communities-trial}
# The logs and results, kept when the bots are removed.
OUT=${TRIAL_OUT:-$DIR-out}
READERS=${READERS:-1000}
# The joins: started a second (RATE; each opens an inbox connection, and
# the relay takes 10 new connections a second from one address, the CLI
# paces them at 6) and in flight at once (INBOXES; the relay takes 20
# connections from one address: 3 shared pools of the swarm, 2 of the
# keeper, 2 of a command, the rest for the inboxes). The waits of the
# scenarios grow with the readers (SCALE, a thousand each).
RATE=${RATE:-8}
INBOXES=${INBOXES:-10}
SCALE=$(( (READERS + 999) / 1000 ))
JOIN_MINUTES=${JOIN_MINUTES:-$((60 * SCALE))}
# The markers of this run: a run on a channel left by an earlier one
# (KEEP=1) counts its own posts and comments.
RUN=${RUN:-$(date +%H%M%S)}
PM=trial-post-$RUN
CM=trial-comment-$RUN

if [ -z "${COMMUNITIES_TRIAL_RELAY:-}" ]; then
  COMMUNITIES_TRIAL_RELAY=$(jq -r '.relays[0].url' "$MANIFEST")
  COMMUNITIES_TRIAL_RELAY_KEY=$(jq -r '.relays[0].auth.key // empty' "$MANIFEST")
fi
: "${COMMUNITIES_TRIAL_STORAGE:=$(jq -r '[.media[].url] | join(",")' "$MANIFEST")}"
export COMMUNITIES_TRIAL_RELAY COMMUNITIES_TRIAL_RELAY_KEY COMMUNITIES_TRIAL_STORAGE

build() {
  [ -n "${COMMUNITIES_CLI:-}" ] && return
  (cd "$ROOT" && cargo build --release --locked -p communities-core --features tools --bin communities-cli)
}

say() { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }

# A command of one bot (or create, relay-stats): its JSON lines on stdout.
cli() { { "$BIN" --dir "$DIR" "$@" 2>>"$DIR/cli.err" || true; } | tee -a "$DIR/cli.log"; }

# A result of the trial: name, pass (true/false), the JSON it rests on.
result() {
  local name=$1 ok=$2 data=${3:-null}
  jq -cn --arg n "$name" --argjson ok "$ok" --argjson d "$data" '{scenario: $n, ok: $ok, data: $d}' | tee -a "$DIR/results.jsonl" >&2
}

# Waits until the log $1 holds a line that jq filter $2 selects (at most
# $3 seconds); prints it. Lines before line $4 do not count.
wait_line() {
  local log=$1 filter=$2 secs=${3:-120} from=${4:-0} line
  local end=$((SECONDS + secs))
  while [ $SECONDS -lt $end ]; do
    line=$(tail -n +"$((from + 1))" "$log" 2>/dev/null | jq -c "select($filter)" 2>/dev/null | head -1 || true)
    if [ -n "$line" ]; then echo "$line"; return 0; fi
    sleep 2
  done
  echo null
}

lines() { if [ -f "$1" ]; then wc -l <"$1"; else echo 0; fi; }

start_keeper() {
  local dir=${1:-$DIR}
  KEEPER_FROM=$(lines "$dir/keeper.log")
  rm -f "$dir/keeper.in"; mkfifo "$dir/keeper.in"
  "$BIN" --dir "$dir" keeper <"$dir/keeper.in" >>"$dir/keeper.log" 2>>"$dir/keeper.err" &
  KEEPER_PID=$!
  exec 7>"$dir/keeper.in"
  wait_line "$dir/keeper.log" '.event == "loaded"' 120 "$(($(lines "$dir/keeper.log") - 1))" >/dev/null || true
}

keeper_cmd() { echo "$*" >&7; }

stop_keeper() {
  local dir=${1:-$DIR}
  [ -n "${KEEPER_PID:-}" ] || return 0
  local from; from=$(lines "$dir/keeper.log")
  keeper_cmd quit
  wait_line "$dir/keeper.log" '.event == "stopped"' 120 "$from" >/dev/null || true
  exec 7>&-
  wait "$KEEPER_PID" 2>/dev/null || true
  KEEPER_PID=
}

# The swarm of readers with its commands on stdin.
start_swarm() {
  local dir=${SWARM_DIR:-$DIR}
  SWARM_FROM=$(lines "$dir/swarm.log")
  rm -f "$dir/swarm.in"; mkfifo "$dir/swarm.in"
  "$BIN" --dir "$dir" swarm --stdin "$@" <"$dir/swarm.in" >>"$dir/swarm.log" 2>>"$dir/swarm.err" &
  SWARM_PID=$!
  exec 8>"$dir/swarm.in"
}

# One command to the swarm: waits for its one answer line and prints it;
# false when its "ok" is false.
swarm_cmd() {
  local dir=${SWARM_DIR:-$DIR} from line
  from=$(lines "$dir/swarm.log")
  echo "$*" >&8
  line=$(SWARM_LINE="$*" wait_line "$dir/swarm.log" '.line == env.SWARM_LINE' "${WAIT:-$((3600 * SCALE))}" "$from")
  echo "$line"
  [ "$(jq -r 'if has("ok") then .ok else true end' <<<"$line")" = true ]
}

stop_swarm() {
  [ -n "${SWARM_PID:-}" ] || return 0
  echo quit >&8 || true
  exec 8>&-
  wait "$SWARM_PID" 2>/dev/null || true
  SWARM_PID=
}

# The pubkeys of swarm bots $1 (a-b), space-separated.
pubkeys() { swarm_cmd "pubkeys bots=$1" | jq -r '.pubkeys | join(" ")'; }

# The epoch the keeper published last.
published() { keeper_cmd state; sleep 2; jq -r 'select(.event == "state" or .event == "keeper") | .state.published' "$DIR/keeper.log" | tail -1; }

cleanup() {
  stop_swarm || true
  stop_keeper || true
  mkdir -p "$OUT"
  (cd "$DIR" 2>/dev/null && find . \( -name "*.log" -o -name "*.err" -o -name "*.jsonl" -o -name "*-stats.json" -o -name channel.json \) -exec cp --parents {} "$OUT" \;) || true
  if [ -z "${KEEP:-}" ]; then rm -rf "$DIR"; say "removed $DIR, logs in $OUT"; fi
}
