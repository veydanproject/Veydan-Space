#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# HEAD of the branch ios on the owner's iPhone, from WSL, with no Mac
# (docs/ci-cd.md, "The iOS lab"):
#   scripts/ios/phone.sh status          # HEAD built or not, the runs, the minutes of the month, the app on the phone
#   scripts/ios/phone.sh build [--dry-run] # the snapshot of HEAD to the lab (scripts/ios/lab-push.sh), which builds it
#   scripts/ios/phone.sh wait [run]      # the run to its end; on a failure the failing step and its log
#   scripts/ios/phone.sh fetch [run]     # the .ipa of the run to Downloads\veydan-ios of Windows
#   scripts/ios/phone.sh install [--dry-run] [ipa]  # signed for the phone, installed over the app (an update), launched
#   scripts/ios/phone.sh all             # build, wait, fetch, install
#   scripts/ios/phone.sh login           # once, by the owner in his terminal: plumesign's Apple ID session
# A run is its number (#21), its id, or nothing: the run of HEAD, else the
# latest. Space alone: the app on the phone is its bundle.
#
# A build costs: the lab is private since 2026-11-04 and a minute of macOS
# counts ten of the 2000 free ones (a build is 10–25 min). So nothing here
# builds by itself: `build` (and `all`) alone push, never for a commit the
# lab built already (the run is found by "iOS lab: space of <sha> — …", the
# message lab-push.sh writes), never over a run in progress (the workflow
# cancels it), never from a tree with changes or a HEAD that origin/ios
# does not hold; it lists the commits since the last build and the minutes
# of the month, so a build is a batch.
#
# The signature: the lab's .ipa is unsigned, and iOS installs what a
# profile of the phone's team signs. The app is installed by Sideloadly with
# the owner's free Apple ID (the personal team, bundle net.veydan.mobile.
# <team>, a profile of 7 days); an install here is an update of that bundle by
# the same team, so the data stays. plumesign (Impactor's CLI, pinned by
# sha256 into data/toolchains/plumesign) signs it in one of two ways
# (IOS_SIGN, or the first that is ready):
#   apple-id    plumesign's own session of the Apple ID (`phone.sh login`
#               once, 2FA; kept in ~/.veydan/ios-sign): a fresh profile at
#               every install. Its first certificate may revoke Sideloadly's
#               (a free team holds few): the app then starts only once this
#               install is done, and Sideloadly makes its own again next time.
#   sideloadly  the certificate and key Sideloadly keeps in %APPDATA%\sideloadly
#               and its profile on the phone (pymobiledevice3 provision dump),
#               while that profile lives (7 days from Sideloadly's last run).
# pymobiledevice3 on Windows installs and launches (the phone is on the USB
# of Windows); the token of the lab is lab-push.sh's (~/.veydan/ios-lab.token),
# which reads the runs, the jobs, their logs and the artifacts.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT_DIR"
PRODUCT=space
LAB_REPO="${IOS_LAB_REPO:-veydanproject/Veydan-iOS-Lab}"
TOKEN_FILE="${IOS_LAB_TOKEN_FILE:-$HOME/.veydan/ios-lab.token}"
BRANCH="${IOS_BRANCH:-ios}"
BUNDLE="${IOS_BUNDLE:-net.veydan.mobile.9233V6SRGP}"
SIGN_HOME="${IOS_SIGN_HOME:-$HOME/.veydan/ios-sign}"
PLUME_VERSION=2.7.2
PLUME_SHA256=daaa6f296247a501c899de6167ebcf1b03ab2a4c8d3ca3ec6ec4c27613701aa2
PLUME="$ROOT_DIR/data/toolchains/plumesign/$PLUME_VERSION/plumesign"
MINUTE_RATE=10 FREE_MINUTES=2000 PRIVATE_SINCE=2026-11-04

die() { echo ">> $*" >&2; exit 1; }
command -v jq >/dev/null || die "jq is missing (apt install jq)"

# --- the lab on GitHub ------------------------------------------------------
token() {
  [ -s "$TOKEN_FILE" ] || die "$TOKEN_FILE is missing: the token of $LAB_REPO (scripts/ios/lab-push.sh)"
  tr -d '[:space:]' < "$TOKEN_FILE"
}
api() { # api <path> [curl args]: a GET of the lab's REST API
  local p="$1"; shift
  curl -sSL --fail-with-body -H "Authorization: Bearer $(token)" -H "Accept: application/vnd.github+json" \
    -H "X-GitHub-Api-Version: 2022-11-28" "$@" "https://api.github.com/repos/$LAB_REPO/$p"
}
RUNS_JSON=""
runs_json() { [ -n "$RUNS_JSON" ] || RUNS_JSON="$(api 'actions/runs?per_page=100')"; printf '%s' "$RUNS_JSON"; }
# One line a run, newest first: id number status conclusion sha-of-the-monorepo url.
runs_tsv() {
  runs_json | jq -r --arg p "$PRODUCT" '.workflow_runs[] | [.id, .run_number, .status, (.conclusion // "-"),
    (((.head_commit.message // "") | capture("^iOS lab: (?<p>[a-z]+) of (?<s>[0-9a-f]+)")) // {p: "", s: "-"}
      | if .p == $p then .s else "-" end), .html_url] | @tsv'
}
run_of_sha() { # the newest run of a commit of the monorepo
  local full; full="$(git rev-parse "$1")"
  runs_tsv | while IFS=$'\t' read -r id n st co sha url; do
    if [ "$sha" != - ] && [[ "$full" == "$sha"* ]]; then printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$id" "$n" "$st" "$co" "$sha" "$url"; break; fi
  done
}
resolve_run() { # resolve_run [number|id] → the line of runs_tsv
  local arg="${1#\#}" line
  if [ -z "$arg" ]; then
    line="$(run_of_sha HEAD)"
    [ -n "$line" ] || { line="$(runs_tsv | head -n1)"; echo ">> HEAD ($(git rev-parse --short HEAD)) has no run; the latest is taken" >&2; }
  else
    line="$(runs_tsv | awk -F'\t' -v a="$arg" '$1 == a || $2 == a' | head -n1)"
  fi
  [ -n "$line" ] || die "no run ${arg:+$arg }in $LAB_REPO"
  printf '%s\n' "$line"
}
# The minutes the plan counts: this month's, and only since the lab is private
# (a public repository builds for free).
billed_since() { local m; m="$(date -u +%Y-%m)-01"; [[ "$m" > "$PRIVATE_SINCE" ]] && echo "$m" || echo "$PRIVATE_SINCE"; }
month_minutes() { # "<runs> <minutes>" since billed_since (a run without a job is 0)
  runs_json | jq -r --arg s "$(billed_since)" '[.workflow_runs[] | select(.created_at >= $s)]
    | "\(length) \([.[] | select(.status == "completed") | (((.updated_at | fromdateiso8601)
      - ((.run_started_at // .created_at) | fromdateiso8601)) / 60 | ceil)] | add // 0)"'
}
print_minutes() {
  local runs mins; read -r runs mins < <(month_minutes)
  echo ">> since $(billed_since): $runs runs of the lab, ~$mins min of macOS ≈ $((mins * MINUTE_RATE)) of the $FREE_MINUTES free minutes of the month"
}

# --- the phone, through pymobiledevice3 on Windows --------------------------
# PowerShell by its path: a shell of WSL without the PATH of Windows (the
# session of an agent) found no powershell.exe, and the .ipa went to a
# Downloads of the worktree.
POWERSHELL="${POWERSHELL:-/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe}"
WIN_HOME=""
win_home() {
  [ -n "$WIN_HOME" ] || WIN_HOME="$(wslpath "$("$POWERSHELL" -NoProfile -Command '[Environment]::GetFolderPath("UserProfile")' | tr -d '\r')")"
  printf '%s' "$WIN_HOME"
}
phone_dir() { local d="${IOS_PHONE_DIR:-$(win_home)/Downloads/veydan-ios}"; mkdir -p "$d"; printf '%s' "$d"; }
pmd() { # pmd <args>: pymobiledevice3 of Windows, each argument quoted for PowerShell
  local exe="${IOS_PMD:-$(wslpath -w "$(win_home)")\\.local\\bin\\pymobiledevice3.exe}" a="" x
  for x in "$@"; do a="$a '${x//\'/\'\'}'"; done
  "$POWERSHELL" -NoProfile -Command "\$env:PYTHONIOENCODING='utf-8'; & '$exe'$a; exit \$LASTEXITCODE" | tr -d '\r'
}
app_info() { pmd apps query "$BUNDLE" 2>/dev/null | jq -c --arg b "$BUNDLE" '.[$b] // empty'; }
# The plist of a .mobileprovision (CMS around XML) and one value of it.
profile_xml() { LC_ALL=C tr -c '\11\12\15\40-\176' '\n' | sed -n '/<?xml/,/<\/plist>/p'; }
plist_value() { awk -v k="<key>$1</key>" 'f && /<(string|date)>/ { gsub(/.*<(string|date)>|<\/(string|date)>.*/, ""); print; exit } index($0, k) { f = 1 }'; }

# --- the commands -------------------------------------------------------------
cmd_status() {
  local head line; head="$(git rev-parse --short HEAD)"
  line="$(run_of_sha HEAD)"
  if [ -n "$line" ]; then IFS=$'\t' read -r _ n st co _ url <<<"$line"; echo ">> HEAD $head: run #$n $st $co $url"
  else echo ">> HEAD $head: not built by the lab"; fi
  echo ">> the last runs:"; runs_tsv | head -n5 | awk -F'\t' '{ printf "   #%s %s %s of %s %s\n", $2, $3, $4, $5, $6 }'
  print_minutes
  local info; info="$(app_info || true)"
  if [ -n "$info" ]; then
    echo ">> on the phone: $BUNDLE $(jq -r '.CFBundleShortVersionString' <<<"$info"), $(jq -r '.SignerIdentity // "?"' <<<"$info"), team $(jq -r '.Entitlements["com.apple.developer.team-identifier"] // "?"' <<<"$info")"
  else
    echo ">> on the phone: $BUNDLE is not installed, or the phone is not on the USB of Windows"
  fi
}

cmd_build() {
  [ "$(git rev-parse --abbrev-ref HEAD)" = "$BRANCH" ] || die "HEAD is not the branch $BRANCH"
  [ -z "$(git status --porcelain)" ] || die "the tree has changes: the lab builds what git holds; commit first"
  git fetch -q origin "$BRANCH"
  git merge-base --is-ancestor HEAD "origin/$BRANCH" || die "origin/$BRANCH does not hold HEAD: push it first (the lab builds what Gitea holds)"
  local head line n st co sha url
  head="$(git rev-parse --short HEAD)"
  line="$(run_of_sha HEAD)"
  if [ -n "$line" ]; then
    IFS=$'\t' read -r _ n st co _ url <<<"$line"
    case "$st/$co" in
      completed/success) echo ">> the lab built $head already: run #$n $url — next: phone.sh fetch $n"; return 0 ;;
      completed/*) die "the lab built $head and it ended $co (run #$n $url): a fix is a new commit; a failure of the runner is re-run in Actions of the lab (the token cannot)" ;;
      *) echo ">> the lab is building $head: run #$n $url — next: phone.sh wait $n"; return 0 ;;
    esac
  fi
  line="$(runs_tsv | awk -F'\t' '$3 != "completed"' | head -n1)"
  if [ -n "$line" ]; then
    IFS=$'\t' read -r _ n st _ sha url <<<"$line"
    die "run #$n ($sha) is $st: a push would cancel it (the workflow's concurrency) — phone.sh wait $n first"
  fi
  # The batch: what this build adds to the last one.
  sha="$(runs_tsv | awk -F'\t' '$5 != "-"' | head -n1 | cut -f5)"
  if [ -n "$sha" ] && git merge-base --is-ancestor "$sha" HEAD 2>/dev/null; then
    echo ">> $(git rev-list --count "$sha..HEAD") commits since the last build of the lab ($sha):"
    git log --oneline --no-decorate "$sha..HEAD" | sed 's/^/   /'
  else
    echo ">> the last build of the lab (${sha:-none}) is not an ancestor of HEAD"
  fi
  print_minutes
  [ "${1:-}" != --dry-run ] || { echo ">> --dry-run: scripts/ios/lab-push.sh $PRODUCT is not run"; return 0; }
  local out; out="$(bash scripts/ios/lab-push.sh "$PRODUCT" 2>&1 | tee /dev/stderr)"
  if grep -q 'already holds this snapshot' <<<"$out"; then
    echo ">> the snapshot of $head is the one the lab built last: nothing was built (phone.sh fetch takes the latest run)"
    return 0
  fi
  local i
  for i in $(seq 1 18); do
    sleep 5; RUNS_JSON=""
    line="$(run_of_sha HEAD)"
    [ -n "$line" ] && { IFS=$'\t' read -r _ n _ _ _ url <<<"$line"; echo ">> run #$n: $url — next: phone.sh wait $n"; return 0; }
  done
  echo ">> pushed, but no run of $head showed in 90 s: https://github.com/$LAB_REPO/actions"
}

report_failure() { # the failing steps of a run and the log around its first error
  local id="$1" jobs jid
  jobs="$(api "actions/runs/$id/jobs")"
  if [ "$(jq '.total_count' <<<"$jobs")" = 0 ]; then
    echo ">> no job ran: .github/workflows/ios.yml did not parse or was refused (the page of the run says why)"; return
  fi
  jq -r '.jobs[] | select(.conclusion != "success") | "   job \(.name): \(.conclusion)", (.steps[] | select(.conclusion == "failure") | "     step \(.number) \(.name): failure")' <<<"$jobs"
  for jid in $(jq -r '.jobs[] | select(.conclusion == "failure") | .id' <<<"$jobs"); do
    echo ">> the log of job $jid, up to its first error:"
    api "actions/jobs/$jid/logs" | tr -d '\r' | sed -E 's/^[0-9]{4}-[0-9-]+T[0-9:.]+Z //' | awk '
      { l[NR] = $0 } /##\[error\]/ && !e { e = NR }
      END { if (!e) { s = NR - 40; t = NR } else { s = e - 40; t = e + 5 } if (s < 1) s = 1; for (i = s; i <= t && i <= NR; i++) print "   " l[i] }'
  done
}

cmd_wait() {
  local line id n st co url step last="" t0; line="$(resolve_run "${1:-}")"
  IFS=$'\t' read -r id n st co _ url <<<"$line"
  echo ">> run #$n $url"
  t0=$(date +%s)
  while [ "$st" != completed ]; do
    [ $(( $(date +%s) - t0 )) -lt 7200 ] || die "run #$n is still $st after 2 h"
    step="$(api "actions/runs/$id/jobs" | jq -r '[.jobs[].steps[]? | select(.status == "in_progress") | .name] | first // empty')"
    [ "$step" = "$last" ] || { echo "   $(date +%H:%M) ${step:-$st}"; last="$step"; }
    sleep 30
    read -r st co < <(api "actions/runs/$id" | jq -r '"\(.status) \(.conclusion // "-")"')
  done
  local mins; mins="$(api "actions/runs/$id" | jq -r '(((.updated_at | fromdateiso8601) - ((.run_started_at // .created_at) | fromdateiso8601)) / 60 | ceil)')"
  if [ "$co" = success ]; then echo ">> run #$n: success in ~$mins min — next: phone.sh fetch $n"; return 0; fi
  echo ">> run #$n: $co after ~$mins min"
  report_failure "$id"
  return 1
}

cmd_fetch() {
  local line id n st co sha dir ipa art aid tmp
  line="$(resolve_run "${1:-}")"
  IFS=$'\t' read -r id n st co sha _ <<<"$line"
  [ "$st/$co" = completed/success ] || die "run #$n is $st $co: there is nothing to fetch"
  dir="$(phone_dir)"
  ipa="$dir/$PRODUCT-$sha-run$n.ipa"
  if [ ! -s "$ipa" ]; then
    art="$(api "actions/runs/$id/artifacts" | jq -r --arg p "ios-$PRODUCT-device-" '[.artifacts[] | select((.name | startswith($p)) and (.expired | not))] | first | "\(.id) \(.name)" // empty')"
    [ -n "$art" ] && [ "$art" != "null null" ] || die "run #$n has no .ipa left (an artifact lives 14 days)"
    read -r aid art <<<"$art"
    tmp="$(mktemp -d "$dir/.run$n.XXXX")"
    echo ">> downloading $art of run #$n"
    api "actions/artifacts/$aid/zip" -o "$tmp/a.zip"
    unzip -q -o "$tmp/a.zip" -d "$tmp/x"
    local found; found="$(find "$tmp/x" -name '*.ipa' | sort | head -n1)"
    [ -n "$found" ] || { rm -rf "$tmp"; die "$art holds no .ipa"; }
    mv "$found" "$ipa"
    rm -rf "$tmp"
  fi
  echo ">> $(wslpath -w "$ipa")"
}

plume() {
  if [ ! -x "$PLUME" ]; then
    local tmp; tmp="$(mktemp)"
    echo ">> downloading plumesign $PLUME_VERSION (Impactor's CLI) into data/toolchains/plumesign" >&2
    curl -sSfL -o "$tmp" "https://github.com/claration/Impactor/releases/download/v$PLUME_VERSION/plumesign-linux-x86_64"
    echo "$PLUME_SHA256  $tmp" | sha256sum -c --quiet - || { rm -f "$tmp"; die "plumesign: the sha256 differs from the pinned one"; }
    mkdir -p "$(dirname "$PLUME")"; install -m 0755 "$tmp" "$PLUME"; rm -f "$tmp"
  fi
  # Its data (the session of the Apple ID, its key, anisette) in ~/.veydan/ios-sign.
  mkdir -p "$SIGN_HOME"; chmod 700 "$SIGN_HOME"
  XDG_CONFIG_HOME="$SIGN_HOME" RUST_LOG="${RUST_LOG:-info}" "$PLUME" "$@"
}
# install --dry-run stops here: the phone and its profiles are read, nothing is signed or installed.
sign_ipa() {
  [ -z "${dry:-}" ] || { echo ">> --dry-run: plumesign sign $*"; exit 0; }
  plume sign "$@"
}

cmd_login() {
  [ -t 0 ] || die "login asks for the password and the code of 2FA: the owner runs it in his terminal"
  plume account login ${1:+-u "$1"}
}

cmd_install() {
  local dry="" ipa dir info team method signed
  [ "${1:-}" != --dry-run ] || { dry=1; shift; }
  ipa="${1:-}"
  dir="$(phone_dir)"
  if [ -z "$ipa" ]; then ipa="$(ls -t "$dir"/"$PRODUCT"-*-run*.ipa 2>/dev/null | head -n1)"; fi
  [[ "$ipa" =~ ^[A-Za-z]:\\ ]] && ipa="$(wslpath -u "$ipa")"
  [ -s "$ipa" ] || die "no .ipa${ipa:+ at $ipa}: phone.sh fetch first"
  info="$(app_info)" || true
  [ -n "$info" ] || die "$BUNDLE is not on the phone (or the phone is not on the USB of Windows): this updates the app Sideloadly installed, never a new one"
  team="$(jq -r '.Entitlements["com.apple.developer.team-identifier"]' <<<"$info")"
  local sl_dir; sl_dir="$(win_home)/AppData/Roaming/sideloadly"
  local sl_cert; sl_cert="$(ls "$sl_dir"/cert-*.pem 2>/dev/null | head -n1)"
  method="${IOS_SIGN:-}"
  if [ -z "$method" ]; then
    if [ -s "$SIGN_HOME/PlumeImpactor/accounts.json" ]; then method=apple-id
    elif [ -n "$sl_cert" ] && [ -s "$sl_dir/key.pem" ]; then method=sideloadly
    else method=none; fi
  fi
  mkdir -p "$dir/signed"
  signed="$dir/signed/$(basename "$ipa" .ipa)-signed.ipa"
  case "$method" in
    apple-id)
      sign_ipa -p "$ipa" --apple-id --custom-identifier "$BUNDLE" -o "$signed" ;;
    sideloadly)
      [ -n "$sl_cert" ] && [ -s "$sl_dir/key.pem" ] || die "no certificate of Sideloadly in $sl_dir"
      # The newest live profile of the bundle that holds Sideloadly's certificate.
      local pdir="$dir/profiles" f x exp best="" best_exp=0 cert flat
      rm -rf "$pdir"; mkdir -p "$pdir"
      pmd provision dump "$(wslpath -w "$pdir")" >/dev/null 2>&1 || die "pymobiledevice3 could not read the profiles of the phone"
      cert="$(sed '/-----/d' "$sl_cert" | tr -d '[:space:]')"
      for f in "$pdir"/*.mobileprovision; do
        [ -e "$f" ] || continue
        x="$(profile_xml < "$f")"
        [ "$(plist_value application-identifier <<<"$x")" = "$team.$BUNDLE" ] || continue
        flat="$(tr -d '[:space:]' <<<"$x")"; [[ "$flat" == *"$cert"* ]] || continue
        exp="$(date -u -d "$(plist_value ExpirationDate <<<"$x")" +%s)"
        [ "$exp" -gt $(( $(date +%s) + 3600 )) ] && [ "$exp" -gt "$best_exp" ] && { best="$f"; best_exp="$exp"; }
      done
      [ -n "$best" ] || die "no live profile of Sideloadly for $BUNDLE on the phone (it lives 7 days): sign once with Sideloadly, or log plumesign in (phone.sh login)"
      echo ">> signing with Sideloadly's certificate and its profile until $(date -u -d "@$best_exp" '+%F %R UTC')"
      sign_ipa -p "$ipa" --pem "$sl_cert" "$sl_dir/key.pem" --provision "$best" --custom-identifier "$BUNDLE" -o "$signed" ;;
    *)
      die "no way to sign yet. Once, by the owner: either 'scripts/ios/phone.sh login' in his terminal (the Apple ID of Sideloadly, 2FA), or Sideloadly on Windows (it keeps its certificate in %APPDATA%\\sideloadly and puts a 7-day profile on the phone). Meanwhile Sideloadly installs $(wslpath -w "$ipa") itself: Advanced options → the bundle id stays $BUNDLE" ;;
  esac
  [ -s "$signed" ] || die "plumesign wrote no $signed"
  # What the phone will check: the profile in the app is of this team and bundle.
  local p; p="$(unzip -p "$signed" 'Payload/*.app/embedded.mobileprovision' | profile_xml)" || true
  [ "$(plist_value application-identifier <<<"$p")" = "$team.$BUNDLE" ] || die "$signed carries a profile of $(plist_value application-identifier <<<"$p"), not $team.$BUNDLE: not installed"
  echo ">> installing $(wslpath -w "$signed") over $BUNDLE (an update: the data stays)"
  pmd apps install "$(wslpath -w "$signed")"
  pmd mounter auto-mount >/dev/null 2>&1 || true
  pmd developer dvt launch "$BUNDLE" >/dev/null || echo ">> installed; the launch failed (developer mode, the image of the phone): start it by hand"
  info="$(app_info || true)"
  echo ">> on the phone: $(jq -r '"\(.CFBundleShortVersionString), \(.SignerIdentity // "?")"' <<<"$info"); it starts until $(plist_value ExpirationDate <<<"$p") (the profile of a free team: 7 days)"
}

cmd_all() {
  cmd_build
  RUNS_JSON=""
  cmd_wait
  RUNS_JSON=""
  cmd_fetch
  cmd_install
}

case "${1:-}" in
  status | build | wait | fetch | install | all | login) c="$1"; shift; "cmd_$c" "$@" ;;
  *) sed -n "5,15p" "$0" | sed 's/^# \{0,1\}//'; exit 1 ;;
esac
