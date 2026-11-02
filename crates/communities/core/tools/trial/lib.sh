# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# Shared by the scenarios of the trial (internal/communities-stage1.md §7,
# row 1.6; the stand of ROO-397, D4): the binary, the network, the keeper
# and the swarm as long processes driven through fifos, the results as
# JSON lines. Sourced.
#
# The network comes from the environment: COMMUNITIES_TRIAL_RELAY,
# COMMUNITIES_TRIAL_RELAY_KEY (or the file COMMUNITIES_TRIAL_RELAY_KEY_FILE),
# COMMUNITIES_TRIAL_STORAGE (comma-separated blob hosts). Unset: the stand's
# test server test01 (its relay, strfry behind vguard, and its blossom).

set -uo pipefail

ROOT=$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)
TRIAL=$ROOT/crates/communities/core/tools/trial
# The toolchain of the repository, as the builds take it.
command -v cargo >/dev/null || . "$ROOT/scripts/build-env.sh" >/dev/null 2>&1 || true
: "${CARGO_TARGET_DIR:=$(cd "$ROOT" && cargo metadata --format-version 1 --no-deps 2>/dev/null | jq -r .target_directory)}"
BIN=${COMMUNITIES_CLI:-$CARGO_TARGET_DIR/release/communities-cli}
DIR=${TRIAL_DIR:-$ROOT/tmp/communities-trial}
# The logs and results, kept when the bots are removed.
OUT=${TRIAL_OUT:-$DIR-out}
# Live bots, each a device with its own database, in one process.
READERS=${READERS:-1000}
# The joins: started a second (RATE; each opens an inbox connection, the
# CLI paces new connections at 6 a second) and in flight at once (INBOXES:
# each on a connection of its own; a relay of the network takes 20 from
# one address, test01 has no limit by address). The waits of the
# scenarios grow with the live readers (SLOW, a thousand each).
RATE=${RATE:-8}
INBOXES=${INBOXES:-10}
SLOW=$(( (READERS + 999) / 1000 ))
JOIN_MINUTES=${JOIN_MINUTES:-$((60 * SLOW))}
# The stage (ROO-397 §2): the readers of the channel, live and synthetic
# (10k, 100k, 1m or a number; READERS by default). The rest past READERS
# are readers without a device the keeper lets in (scenario i).
SCALE=${SCALE:-$READERS}
case $SCALE in
  *[kK]) SCALE_N=$(( ${SCALE%[kK]} * 1000 )) ;;
  *[mM]) SCALE_N=$(( ${SCALE%[mM]} * 1000000 )) ;;
  *) SCALE_N=$SCALE ;;
esac
SYNTHETIC=$(( SCALE_N > READERS ? SCALE_N - READERS : 0 ))
# The keeper's limits on the stand only (the app keeps 120 joins a minute
# and 65 536 readers): JOINS_PER_MINUTE, MAX_READERS (past one snapshot
# when the stage is bigger than it).
[ -n "${JOINS_PER_MINUTE:-}" ] && export COMMUNITIES_TRIAL_JOINS_PER_MINUTE=$JOINS_PER_MINUTE
if [ -n "${MAX_READERS:-}" ]; then
  export COMMUNITIES_TRIAL_MAX_READERS=$MAX_READERS
elif [ "$SCALE_N" -gt 65000 ]; then
  export COMMUNITIES_TRIAL_MAX_READERS=$(( SCALE_N + 10000 ))
fi
# The traffic of the run (relay and blob hosts, as the processes count
# it) stops it near TRAFFIC_LIMIT_MB: the quota of the server is not
# known, the budget of the whole stand is ~200 GB.
TRAFFIC_LIMIT_MB=${TRAFFIC_LIMIT_MB:-20480}
export COMMUNITIES_TRIAL_TRAFFIC_LIMIT_MB=$TRAFFIC_LIMIT_MB
export COMMUNITIES_TRIAL_TRAFFIC_DIR=$DIR/traffic
export COMMUNITIES_TRIAL_UPLOADS=$DIR/uploads.jsonl
# The markers of this run: a run on a channel left by an earlier one
# (KEEP=1) counts its own posts and comments.
RUN=${RUN:-$(date +%H%M%S)}
PM=trial-post-$RUN
CM=trial-comment-$RUN

SECRETS=${TRIAL_SECRETS:-/srv/veydan/shared/secrets}
: "${COMMUNITIES_TRIAL_RELAY:=wss://test01-relay.veydan.net}"
: "${COMMUNITIES_TRIAL_STORAGE:=https://test01-media.veydan.net}"
: "${COMMUNITIES_TRIAL_RELAY_KEY_FILE:=$SECRETS/test01-relay.key}"
if [ -z "${COMMUNITIES_TRIAL_RELAY_KEY:-}" ] && [ -r "$COMMUNITIES_TRIAL_RELAY_KEY_FILE" ]; then
  COMMUNITIES_TRIAL_RELAY_KEY=$(tr -d '[:space:]' <"$COMMUNITIES_TRIAL_RELAY_KEY_FILE")
fi
export COMMUNITIES_TRIAL_RELAY COMMUNITIES_TRIAL_RELAY_KEY="${COMMUNITIES_TRIAL_RELAY_KEY:-}" COMMUNITIES_TRIAL_STORAGE
# test01's media takes uploads by upload keys of its server (wire §9.1):
# the owner's device holds the app key of the network, the relay's key.
if [ -z "${COMMUNITIES_APP_KEYS:-}" ] && [ -n "$COMMUNITIES_TRIAL_RELAY_KEY" ]; then
  case $COMMUNITIES_TRIAL_STORAGE in
    *https://test01-media.veydan.net*) export COMMUNITIES_APP_KEYS="https://test01-media.veydan.net=$COMMUNITIES_TRIAL_RELAY_KEY" ;;
  esac
fi
# The server of the relay by SSH (read-only checks, and the cleanup of
# the events of the run): test01 by its key, unless SERVER_SSH says.
if [ -z "${SERVER_SSH+x}" ]; then
  SERVER_SSH=
  case $COMMUNITIES_TRIAL_RELAY in
    *test01-relay.veydan.net*) [ -r "$SECRETS/test01-ssh" ] && SERVER_SSH="ssh -i $SECRETS/test01-ssh -o BatchMode=yes -o ConnectTimeout=15 root@test01.veydan.net" ;;
  esac
fi
STRFRY=${STRFRY:-runuser -u strfry -- /opt/strfry/bin/strfry --config /opt/strfry/etc/strfry.conf}

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
    # A run past its traffic limit waits for nothing more.
    if (( SECONDS % 30 < 2 )) && ! traffic_ok; then break; fi
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

keeper_cmd() { echo "$*" >&7 || say "the keeper is gone: $*"; }

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
  line=$(SWARM_LINE="$*" wait_line "$dir/swarm.log" '.line == env.SWARM_LINE' "${WAIT:-$((3600 * SLOW))}" "$from")
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

# ─── Before the start ───────────────────────────────────────────────────────

# The stand does not start while QA works in the shared copy or CI runs
# (ROO-397 §5): qa-dev status says nothing, no run of Gitea Actions is
# queued or running, no pregate or ship is running here.
preflight() {
  local why=() qa n
  qa=$(${QA_DEV:-/srv/veydan/shared/bin/qa-dev} status 2>&1) || why+=("qa-dev status failed")
  [ -z "$qa" ] || why+=("QA: $qa")
  local token=$SECRETS/gitea-agent.token api=${GITEA_API:-https://git.veydan.net/api/v1/repos/veydanproject/monorepo}
  if [ -r "$token" ]; then
    for st in in_progress queued waiting; do
      n=$(curl -fsS --max-time 20 -H "Authorization: token $(cat "$token")" "$api/actions/runs?status=$st&limit=1" | jq -r '.total_count // 0') || n=unknown
      [ "$n" = 0 ] || why+=("CI: $st $n")
    done
  else
    why+=("CI: no token to ask Gitea")
  fi
  pgrep -f 'scripts/ci/pregate\.sh|scripts/ship\.mjs' >/dev/null && why+=("CI: pregate or ship running here")
  if [ ${#why[@]} -gt 0 ]; then
    printf 'the stand does not start: %s\n' "${why[@]}" >&2
    return 1
  fi
}

# ─── The server ─────────────────────────────────────────────────────────────

server() { [ -n "$SERVER_SSH" ] && $SERVER_SSH "$@"; }

# What the server is before and after the run: when its services started
# and how often they restarted, the blobs on disk, the bytes of its link.
server_look() {
  [ -n "$SERVER_SSH" ] || { echo null; return; }
  server 'set -e
    s=$(for u in strfry vguard blossom nginx libsql; do systemctl show -p ActiveEnterTimestamp -p NRestarts --value "$u" | paste -sd" "; done | paste -sd"|")
    b=$(ls /opt/blossom/data/blobs | wc -l)
    i=$(ip route show default | awk "{print \$5; exit}")
    printf "{\"services\": \"%s\", \"blobs\": %s, \"rx\": %s, \"tx\": %s, \"at\": %s}\n" "$s" "$b" "$(cat /sys/class/net/$i/statistics/rx_bytes)" "$(cat /sys/class/net/$i/statistics/tx_bytes)" "$(date +%s)"' 2>/dev/null || echo null
}

# The events of the run on the relay, one filter a line on stdin: their
# count (`count`) or deleted (`delete`), by strfry on the server.
server_events() {
  local what=$1
  [ -n "$SERVER_SSH" ] || { echo null; return; }
  server "total=0; while IFS= read -r f; do
      if [ $what = delete ]; then $STRFRY delete --filter \"\$f\" >/dev/null 2>&1 || exit 1; fi
      n=\$($STRFRY scan --count \"\$f\" 2>/dev/null | tail -1 | tr -dc 0-9); total=\$((total + \${n:-0}))
    done; echo \$total" || echo null
}

# ─── Traffic ────────────────────────────────────────────────────────────────

# The traffic of the run so far, as the processes counted it.
traffic() {
  if compgen -G "$COMMUNITIES_TRIAL_TRAFFIC_DIR/*.json" >/dev/null; then
    jq -s '{processes: length, relay_mb: ((map(.relay_out + .relay_in) | add) / 1048576 * 10 | round / 10), media_mb: ((map(.media_out + .media_in) | add) / 1048576 * 10 | round / 10), total_mb: ((map(.total) | add) / 1048576 * 10 | round / 10)}' "$COMMUNITIES_TRIAL_TRAFFIC_DIR"/*.json
  else
    echo '{"processes": 0, "relay_mb": 0, "media_mb": 0, "total_mb": 0}'
  fi
}

# Past the limit: false (a process over it has ended by itself already).
traffic_ok() { [ "$(traffic | jq --argjson l "$TRAFFIC_LIMIT_MB" '.total_mb < $l')" = true ]; }

# ─── The end ────────────────────────────────────────────────────────────────

cleanup() {
  stop_swarm || true
  stop_keeper || true
  mkdir -p "$OUT"
  if [ -z "${KEEP:-}" ] && [ -d "$DIR" ]; then
    # What the run left on the servers: its blobs deleted by those who put
    # them, its events on the relay deleted on the server.
    local filters cleaned left before
    filters=$("$BIN" --dir "$DIR" cleanup 2>>"$DIR/cli.err" | tee -a "$DIR/cli.log")
    cleaned=$(jq -c 'select(.event == "cleanup")' <<<"$filters")
    before=$(jq -c 'select(.filter) | .filter' <<<"$filters" | server_events count)
    left=$(jq -c 'select(.filter) | .filter' <<<"$filters" | server_events delete)
    # Without SSH to the server the events stay there, told as null.
    [ "$(jq -r .ok <<<"${cleaned:-null}")" = true ] || CLEAN_FAIL=1
    [ -z "$SERVER_SSH" ] || [ "$left" = 0 ] || CLEAN_FAIL=1
    result cleanup "$( [ -z "${CLEAN_FAIL:-}" ] && echo true || echo false)" \
      "$(jq -cn --argjson c "${cleaned:-null}" --arg b "$before" --arg l "$left" '{blobs: $c, relay_events_of_run: ($b | tonumber? // null), relay_events_left: ($l | tonumber? // null)}')"
  fi
  if [ -n "${SERVER_BEFORE:-}" ] && [ "$SERVER_BEFORE" != null ]; then
    local after same; after=$(server_look)
    same=$(jq -n --argjson a "$SERVER_BEFORE" --argjson b "$after" '$a != null and $b != null and $a.services == $b.services and ($b.blobs <= $a.blobs)')
    [ "$same" = true ] || CLEAN_FAIL=1
    result server "$(jq -n --argjson a "$SERVER_BEFORE" --argjson b "$after" '$a != null and $b != null and $a.services == $b.services and ($b.blobs <= $a.blobs)')" \
      "$(jq -cn --argjson a "$SERVER_BEFORE" --argjson b "$after" '{before: $a, after: $b, services_same: ($a.services == $b.services), blobs_left: ($b.blobs - $a.blobs), link_mb: {rx: (($b.rx - $a.rx) / 1048576 | round), tx: (($b.tx - $a.tx) / 1048576 | round)}}')"
  fi
  [ -d "$DIR" ] && result traffic true "$(traffic)"
  (cd "$DIR" 2>/dev/null && find . \( -name "*.log" -o -name "*.err" -o -name "*.jsonl" -o -name "*-stats.json" -o -name channel.json \) -exec cp --parents {} "$OUT" \;) || true
  if [ -z "${KEEP:-}" ]; then rm -rf "$DIR"; say "removed $DIR, logs in $OUT"; fi
  say "results: $OUT/results.jsonl"
  jq -c '{scenario, ok}' "$OUT/results.jsonl" >&2 || true
  # What the run left on the servers fails it too.
  [ -z "${CLEAN_FAIL:-}" ] || exit 5
}
