#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Veydan Project
# SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
#
# The simulator app of a product on an iPhone simulator: boot, install,
# launch, a screenshot after a pause and a late one, shut down
# (internal/platform-spec.md 13.6):
#   scripts/ios/simulator.sh [product] [seconds]      # after scripts/ios/build.sh <product> simulator
# The screenshot goes to tmp/ios/<product>-<n>.png (the next free number),
# where make android-screenshot keeps its own, and a second one 35 s later
# to <product>-<n>-late.png (a slow start against a dead one). On a runner
# with no window the simulator still renders: the picture shows what the
# first start drew. Beside them: the log of the process (<n>.log), the
# app's own lines of it (<n>-app.log: the setup markers, the plugins) and
# two samples of its threads at 5 s and 15 s (<n>-sample-<s>.txt): what the
# main thread waits for while the screen stays white.
set -euo pipefail

case "${1:-}" in
  '' | [0-9]*) PRODUCT_ARG=space ;;
  *) PRODUCT_ARG="$1"; shift || true ;;
esac
WAIT="${1:-20}"
LATE=35

IOS_SCRIPTS_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$IOS_SCRIPTS_DIR/env.sh"
cd "$ROOT_DIR"
source "$ROOT_DIR/scripts/product.sh"
product_env "$PRODUCT_ARG"

OUT="$ROOT_DIR/release-assets/ios"
app="$(cat "$OUT/.app-path" 2>/dev/null || true)"
[ -n "$app" ] && [ -d "$app" ] || { echo ">> no simulator app: run scripts/ios/build.sh $PRODUCT simulator first"; exit 1; }

# The simulator: IOS_SIMULATOR (a name or a UDID) when set; otherwise the
# newest numbered iPhone of the newest runtime ("iPhone 18 Pro Max"), then
# any other iPhone. A numbered one first: a sort by name alone put "iPhone
# Duo" on top, a phone of two screens whose screenshot asks which one.
udid="$(xcrun simctl list devices available -j | IOS_SIMULATOR="${IOS_SIMULATOR:-}" node -e '
  const d = JSON.parse(require("fs").readFileSync(0, "utf8")).devices;
  const want = process.env.IOS_SIMULATOR;
  const runtimes = Object.keys(d).filter((r) => /iOS/.test(r)).sort();
  if (want) {
    for (const r of runtimes.reverse()) {
      const hit = d[r].find((x) => x.isAvailable && (x.udid === want || x.name === want));
      if (hit) { console.log(hit.udid); process.exit(0); }
    }
    process.exit(1);
  }
  const byName = (a, b) => b.name.localeCompare(a.name, undefined, { numeric: true });
  // A runtime of one device (iOS-27-2-iPhone19-4) holds that device alone:
  // the numbered phones of every runtime are looked at before any other.
  const phones = runtimes.reverse().flatMap((r) => d[r].filter((x) => x.isAvailable && /^iPhone/.test(x.name)));
  const pick = phones.filter((x) => /^iPhone \d/.test(x.name)).sort(byName)[0] || phones.sort(byName)[0];
  if (pick) { console.log(pick.udid); process.exit(0); }
  process.exit(1);')" || { echo ">> no iPhone simulator ${IOS_SIMULATOR:+named $IOS_SIMULATOR }is available"; xcrun simctl list devices available; exit 1; }
echo ">> simulator $udid"
xcrun simctl boot "$udid" 2>/dev/null || true
xcrun simctl bootstatus "$udid" -b
xcrun simctl install "$udid" "$app"
mkdir -p tmp/ios
n=1; while [ -e "tmp/ios/$PRODUCT-$n.png" ]; do n=$((n + 1)); done
launched="$(xcrun simctl launch "$udid" "$PRODUCT_IOS_IDENTIFIER")"
echo "$launched"
pid="${launched##*: }"
# The threads of the process at 5 s and 15 s (the host's `sample`: a
# simulator process is a process of the Mac): the stack of the main thread
# names what a white screen waits for. Sampling takes 2 s, naming the
# symbols of a 100 MB binary takes minutes: each runs in the background,
# the screenshots keep their times, the files are waited for at the end.
samples=""
for at in 5 15; do
  ( sleep "$at"; sample "$pid" 2 -mayDie -file "tmp/ios/$PRODUCT-$n-sample-$at.txt" >/dev/null 2>&1 || true ) &
  samples="$samples $!"
done
sleep "$WAIT"
xcrun simctl io "$udid" screenshot "tmp/ios/$PRODUCT-$n.png"
echo ">> tmp/ios/$PRODUCT-$n.png"
sleep "$LATE"
xcrun simctl io "$udid" screenshot "tmp/ios/$PRODUCT-$n-late.png"
echo ">> tmp/ios/$PRODUCT-$n-late.png"
# The app's own log of the start, for the report, and the verdict: a start
# that died (an uncaught exception, a crash) fails the smoke, whatever the
# screenshot shows.
proc="$(basename "$app" .app)"
xcrun simctl spawn "$udid" log show --last 10m --predicate "process == \"$proc\"" --style compact 2>/dev/null | grep -v "com.apple.WebKit" | tail -n 3000 > "tmp/ios/$PRODUCT-$n.log" || true
grep -F "[net.veydan:veydan]" "tmp/ios/$PRODUCT-$n.log" > "tmp/ios/$PRODUCT-$n-app.log" || true
alive="$(xcrun simctl spawn "$udid" launchctl list 2>/dev/null | grep -c "$PRODUCT_IOS_IDENTIFIER" || true)"
# shellcheck disable=SC2086
wait $samples
xcrun simctl shutdown "$udid" || true
if grep -qE "Terminating app due to uncaught exception|EXC_BAD_ACCESS|SIGABRT|panicked at" "tmp/ios/$PRODUCT-$n.log"; then
  echo ">> the app died at the start:"
  grep -E "Terminating app|EXC_BAD_ACCESS|SIGABRT|panicked at" "tmp/ios/$PRODUCT-$n.log" | head -n 5 | cut -c1-300
  exit 1
fi
[ "$alive" != 0 ] || { echo ">> the app is not running $((WAIT + LATE)) s after the launch"; exit 1; }
echo ">> the app is running ($proc)"
