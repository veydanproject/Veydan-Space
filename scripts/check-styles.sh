#!/usr/bin/env bash
# ============================================================================
# check-styles.sh — design-system guardrail (bash, grep, awk, perl: no npm).
#
#   FAIL: a component or stylesheet references a CSS custom property that
#         nothing defines. This is the exact class of bug that made panels
#         render transparent/inherited. A reference resolves when the name is
#           1. a token: declared in a `:root` or `[data-theme=…]` block of
#              ui/src/lib/core/styles/tokens.css or ui/src/lib/core/styles/mobile.css;
#           2. local: declared in the same file — in its CSS, in a `style="…"`
#              attribute or by a `style:--name` directive;
#           3. listed in RUNTIME_VARS below: set from another file at run time.
#         A fallback does not excuse an unknown name: `var(--typo, 4px)` fails.
#   FAIL: a ligature rule (font-variant-ligatures, liga/calt) outside base.css:
#         text read character by character is `.mono` (STYLEGUIDE.md).
#   FAIL: text in a colour from data set outside base.css (an inline text
#         colour from an expression, over any number of lines, a `color:`
#         built into a string, a `color:` reading a property set from
#         data): such a chip or name is `.tinted` / `.tinted-text`. Read by
#         perl, a file at a time; a colour passed in an object is out of
#         reach. Cases: scripts/tests/check-styles.tests.mjs.
#   WARN: raw hex colours and raw font-sizes inside component <style> blocks
#         (should use tokens), and mobile tokens used with no fallback outside
#         the mobile folders. Non-failing — a drift meter, not a gate.
#
# Usage:  bash scripts/check-styles.sh          (from repo root)
# Exit:   non-zero if any undefined token reference is found.
# ============================================================================
set -uo pipefail
cd "$(dirname "$0")/.." || exit 2

SRC="ui/src"
TOKENS="$SRC/lib/core/styles/tokens.css"
# Loaded by MobileShell only: its tokens exist on the phone, not on the desk.
# The Android insets --sat/--sab/--sal/--sar are declared there through env();
# MainActivity overwrites them with the real values.
MOBILE="$SRC/lib/core/styles/mobile.css"
MOBILE_PATHS="^$SRC/(lib/[a-z]+/mobile/|lib/[a-z]+/routes/Mobile|routes/\(mobile\)/)|^$MOBILE:"

# Set at run time from a file other than the one that reads them.
RUNTIME_VARS="
  --chip
  --inspector-bar-height
"
#   --chip                  style:--chip on a `.tinted` / `.tinted-text`
#                           element (a colour from data); read by base.css
#   --inspector-bar-height  UIInspector.svelte, setProperty on <html>; read by
#                           DesktopShell.svelte

# Declarations of the root and theme blocks only: a custom property declared
# inside a class rule is local to that rule's file, not a token.
tokens_of() {
  awk '/^(:root|\[data-theme=[^]]*\])[ \t]*\{/ { on = 1 } /^\}/ { on = 0 } on' "$1" \
    | grep -oE '^\s*--[a-z0-9-]+:' | tr -d ' \t:' | sort -u
}

for f in "$TOKENS" "$MOBILE"; do
  [ -f "$f" ] || { echo "check-styles: $f not found"; exit 2; }
done

desk_tokens=$(tokens_of "$TOKENS")
mobile_tokens=$(tokens_of "$MOBILE")
defined=$(printf '%s\n' "$desk_tokens" "$mobile_tokens" $RUNTIME_VARS | sort -u)

TAB=$'\t'
# "file<TAB>--name" for every match of a pattern in the checked tree.
pairs() {
  grep -rHoE --include='*.svelte' --include='*.css' -- "$1" "$SRC" \
    | sed -E "s/^([^:]+):[^-]*(--[a-z0-9-]+).*\$/\\1${TAB}\\2/" | sort -u
}

used=$(pairs 'var\(--[a-z0-9-]+')
local_defs=$(pairs '(^|style:|[^A-Za-z0-9_:-])--[a-z0-9-]+[[:space:]]*:|style:--[a-z0-9-]+')

undefined=$(awk -F'\t' '
  FILENAME == ARGV[1] { token[$1] = 1; next }
  FILENAME == ARGV[2] { here[$0] = 1; next }
  !($2 in token) && !($0 in here) { print }
' <(printf '%s\n' "$defined") <(printf '%s\n' "$local_defs") <(printf '%s\n' "$used"))

fail=0
echo "── Undefined token references (FAIL) ──"
if [ -n "$undefined" ]; then
  fail=1
  while IFS=$'\t' read -r file v; do
    echo "  ✗ $v"
    grep -nE -- "var\($v[,)]" "$file" | sed "s|^|      $file:|" | head -4
  done <<< "$undefined"
else
  echo "  ✓ none — every var() resolves to a defined token"
fi

echo ""
echo "── Ligature rules outside base.css (FAIL) ──"
# Text read character by character is `.mono` (or a code element): base.css
# turns ligatures off there, once (STYLEGUIDE.md, Primitives). A component
# that declares its own rule drifts from it.
ligatures=$(grep -rnE --include='*.svelte' --include='*.css' "font-variant-ligatures|font-feature-settings:[^;]*(liga|calt)" "$SRC" \
  | grep -v "^$SRC/lib/core/styles/base.css:")
if [ -n "$ligatures" ]; then
  fail=1
  printf '%s\n' "$ligatures" | sed 's/^/  ✗ /'
  echo "    use the .mono class of base.css instead"
else
  echo "  ✓ none — ligatures are turned off by base.css alone"
fi

echo ""
echo "── Text in a colour from data outside base.css (FAIL) ──"
# A chip, badge or name coloured from data (a workspace's colour, a label's)
# is `.tinted` / `.tinted-text` with `style:--chip`: base.css mixes its text
# towards --text so it reads in both themes (STYLEGUIDE.md, Primitives). A
# component that sets such a text colour itself drifts from it. Found, in
# each file read whole (an attribute broken over lines counts):
#   - `style:color={…}` and its shorthand `style:color`;
#   - a `color:` with an interpolation in a `style="…"` attribute;
#   - a `color:` built into a string: `color: ${…}` in a template string,
#     `'color:' + …` (also in .ts files);
#   - a `color:` that reads a custom property some component sets from data
#     (`style:--x={…}`, `style="--x: {…}"`).
# Out of reach: a colour handed over in an object (`{ color: c }`) or a
# string assembled in pieces elsewhere — review catches those.
# Excused: icons and marks with no text, and colours that are tokens chosen
# by state. Each excuse is the whole line, whitespace collapsed, exactly as
# it stands in its file: any other line of the same file still fails.
DATA_COLOR_OK="
ui/src/lib/core/styles/mobile.css	color: color-mix(in srgb, var(--c, var(--accent)) calc(100% - var(--m-icon-glyph-mix)), #fff);
ui/src/lib/notes/mobile/NotesNav.svelte	color: color-mix(in srgb, var(--c) 70%, var(--text-2));
ui/src/lib/browser/desktop/HomePage.svelte	color: var(--ws-color, var(--accent));
ui/src/lib/notes/components/NotesTable.svelte	<span class=\"entity\" style=\"color:{c.color}\" title={c.name}><Icon name={c.icon} size={12} /></span>
ui/src/lib/pass/components/TotpLiveCode.svelte	<span class=\"code\" class:pop={copied} style:color={copied ? 'var(--success-text)' : color}>
ui/src/lib/pass/components/TotpList.svelte	style:color={copiedId === entry.id ? 'var(--success-text)' : code ? ringColor(code.seconds_left) : undefined}
ui/src/lib/messenger/dm/MessageBubble.svelte	{#if author && showAuthor}<span class=\"author\" style=\"color: {tint(m.sender_pubkey)}\">{author(m.sender_pubkey)}</span>{/if}
ui/src/lib/messenger/dm/AlbumBubble.svelte	{#if author}<span class=\"author\" style=\"color: {authorTint}\">{author}</span>{/if}
"
#   mobile.css      .m-doc: an icon tile, a glyph in its colour, no text
#   NotesNav        a folder's icon in the phone's navigation, no text
#   HomePage        a workspace card's icon, no text
#   NotesTable      the context column: kind icons, the name in the tooltip
#   TotpLiveCode,   a code's countdown colour: tokens (accent, warn, danger)
#   TotpList          by the seconds left, not data
#   MessageBubble,  a chat author's name in a hue of their key: a name in a
#   AlbumBubble       bubble, not a chip (Chat's own palette)
data_vars=$( { grep -rhoE --include='*.svelte' 'style:--[a-z0-9-]+=' "$SRC" | sed -E 's/^style:(--[a-z0-9-]+)=$/\1/'
               grep -rhoE --include='*.svelte' 'style="[^"]*--[a-z0-9-]+:[[:space:]]*\{' "$SRC" | grep -oE -- '--[a-z0-9-]+:[[:space:]]*\{' | sed -E 's/:.*//'
               printf '%s\n' $RUNTIME_VARS | grep -x -- '--chip'; } | sort -u | paste -sd'|')
data_text=$(find "$SRC" -type f \( -name '*.svelte' -o -name '*.css' -o -name '*.ts' \) ! -name '*.test.ts' ! -path "$SRC/lib/core/styles/base.css" -print0 \
  | sort -z | DATA_VARS="$data_vars" DATA_COLOR_OK="$DATA_COLOR_OK" xargs -0 perl -e '
  use strict; use warnings;
  my %ok;
  for (split /\n/, $ENV{DATA_COLOR_OK}) { my ($f, $t) = split /\t/, $_, 2; $ok{"$f\t$t"} = 1 if defined $t; }
  my $vars = $ENV{DATA_VARS};
  my @markup = (qr/style:color(?=[\s=>\/]|$)/m,
                qr/\bstyle="(?:[^"]*?[;\s])?color:\s*\{/,
                qr/\bstyle=\x27(?:[^\x27]*?[;\s])?color:\s*\{/);
  my @strings = (qr/(?<![\w-])color:[ \t]*\$\{/,
                 qr/(?<![\w-])color:[ \t]*[\x27"`][ \t]*\+/);
  my @css = $vars ne "" ? (qr/(?<![\w-])color:[^;{}]*?var\((?:$vars)[,)]/) : ();
  for my $f (@ARGV) {
    open my $fh, "<", $f or next; local $/; my $s = <$fh>; close $fh;
    my @pats = $f =~ /\.svelte$/ ? (@markup, @strings, @css) : $f =~ /\.ts$/ ? @strings : @css;
    my %seen;
    for my $re (@pats) {
      while ($s =~ /$re/g) {
        my ($a, $b) = ($-[0], $+[0]);
        my $from = rindex($s, "\n", $a - 1) + 1;
        my $to = index($s, "\n", $b > $a ? $b - 1 : $b); $to = length $s if $to < 0;
        my $line = 1 + (substr($s, 0, $a) =~ tr/\n//);
        (my $text = substr($s, $from, $to - $from)) =~ s/\s+/ /g;
        $text =~ s/^ | $//g;
        next if $seen{$line}++ || $ok{"$f\t$text"};
        print "$f:$line: $text\n";
      }
    }
  }')
if [ -n "$data_text" ]; then
  fail=1
  printf '%s\n' "$data_text" | sed 's/^/  ✗ /'
  echo "    use class=\"tinted\" (a chip) or \"tinted-text\" (a name) with style:--chip={colour}"
else
  echo "  ✓ none — text in a colour from data goes through .tinted alone"
fi

echo ""
echo "── Drift meter (WARN, non-failing) ──"
hex=$(grep -rhoiE --include='*.svelte' '#[0-9a-f]{3,8}\b' "$SRC" | grep -vi '%23' | wc -l | tr -d ' ')
fs=$(grep -rhoE --include='*.svelte' 'font-size:\s*[0-9.]+(rem|px)' "$SRC" | wc -l | tr -d ' ')
mobile_only=$(comm -23 <(printf '%s\n' "$mobile_tokens") <(printf '%s\n' "$desk_tokens") | paste -sd'|')
bare=0
if [ -n "$mobile_only" ]; then
  bare=$(grep -rHoE --include='*.svelte' --include='*.css' -- "var\(($mobile_only)\)" "$SRC" \
    | grep -vE "$MOBILE_PATHS" | wc -l | tr -d ' ')
fi
echo "  raw hex colours in components:  $hex   (prefer var(--…))"
echo "  raw font-size values:           $fs   (prefer var(--fs-*))"
echo "  mobile tokens, no fallback, outside the mobile folders:  $bare   (not defined on the desk)"

echo ""
if [ "$fail" -ne 0 ]; then
  echo "RESULT: FAIL — fix the findings above."
  exit 1
fi
echo "RESULT: OK"
