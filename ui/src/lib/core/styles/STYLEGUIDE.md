# Veydan Space — UI style guide

Single source of truth for styling. **Before adding CSS, check whether a token or
primitive already exists.** New ad-hoc colours/sizes/components are how the UI
drifted in the first place.

## Layers

| File | Role |
|------|------|
| `src/lib/styles/tokens.css` | Design tokens — colours (dark/light), spacing, type, radius, z-index, sizes, motion. The **only** place raw values live. |
| `src/lib/styles/base.css` | Global reset + reusable primitives (`.btn`, `.badge`, `.page`, `.card`, `.tab-bar`, `.empty-state`, `.spinner`, …). |
| `src/lib/styles/mobile.css` | Mobile layer on top of the two: `--m-*` colours, `--touch`, `--nav-h`, safe-area insets `--sat`/`--sab`/`--sal`/`--sar`, `.m-*` primitives. Loaded by the mobile shell only — its tokens do not exist on the desktop. |
| `src/lib/components/ui/Dialog.svelte` | Centered modal shell. |
| `src/lib/components/ui/Drawer.svelte` | Right/left side-panel shell. |

`tokens.css` and `base.css` are imported by each shell (`DesktopShell.svelte`,
`MobileShell.svelte`) and by `StartErrorScreen.svelte`; `mobile.css` by
`MobileShell.svelte` alone.

## Rules

1. **Colour** → always `var(--…)`. Never a raw hex/rgba in a component (except pure
   `#fff` on a coloured button). If you need a new colour, add it to `tokens.css`
   in **both** themes.
2. **Spacing / radius / font-size** → use `--sp-*`, `--radius*`, `--fs-*`. Don't
   invent `0.42rem`.
3. **z-index** → use `--z-*` (`--z-drawer`, `--z-modal`, `--z-toast`, …). Never a
   bare number.
4. **Never invent a token name.** Only names defined in `tokens.css` (and, on
   mobile, `mobile.css`) exist. A
   `var(--made-up)` with no fallback renders as transparent/inherited. Run the
   guard (below) — it fails on undefined references. A custom property a
   component sets for itself (`--r` in its own `<style>`, `style:--c={…}` on
   an element) is fine as long as it is read in the same file.
5. **Reuse primitives.** Need a badge/tab/empty-state/card? Use the global class.
   Only add scoped CSS for a genuine one-off delta.

## Tokens cheat-sheet

- Surfaces: `--bg` < `--bg-2` < `--surface` < `--surface-2`; nested/fields `--surface-3`;
  hovers `--surface-hover` / `--surface-row-hover`; drawers `--surface-drawer(-footer)`;
  borders `--border` / `--border-2`.
- Text: `--text` / `--text-2` / `--text-3`; long-form `--text-body`; extras
  `--text-soft` (metrics/icons) · `--text-faint` (field labels) · `--text-dim` (caps labels, «—»).
- Accent (purple): `--accent` / `--accent-hover` / `--accent-grad` (primary buttons, logo)
  · text tiers `--accent-text` (active nav/tabs) / `--accent-text-2` (mono badges) / `--accent-text-3` (links)
  · fills `--accent-bg` / `--accent-tint`(+`-border`) · `--accent-border` · `--shadow-accent`.
- Semantic: `--success*` (+`-border`, `-grad`), `--danger*` (+`-border`, `--shadow-danger`), `--warn-*` (+`-border`).
- Category colours (workspaces / proxy types): `--cat-purple` / `--cat-blue` / `--cat-teal` / `--cat-pink`.
- Fonts: `--font-ui` (Manrope Variable) · `--font-mono` (JetBrains Mono Variable — hosts, versions, IDs).
- Spacing (4px grid): `--sp-1`=4 … `--sp-6`=24, `--sp-8`=32.
- Type: `--fs-xs` 0.75 · `--fs-sm` 0.8 · `--fs-base` 0.875 · `--fs-md` 1 · `--fs-lg` 1.15 · `--fs-xl` 1.4 · `--fs-2xl` 1.75 (page h1) · `--fs-3xl` 2rem; weights up to `--fw-extrabold` 800.
- Radius: `--radius-xs` 4 · `--radius-sm` 8 (chips/badges/icon-btns) · `--radius` 10 (buttons/inputs) ·
  `--radius-field` 11 (46px drawer fields) · `--radius-md` 12 (nested cards) · `--radius-lg` 16 (cards/tables) · `--radius-pill` 999.
- Size: `--topbar-h` 64 · `--dock-h`/`--bar-h` 36 · `--control-h` 38 · `--control-h-lg` 46 ·
  `--drawer-w` 440 · `--drawer-w-md` 480 · `--drawer-w-lg` 520 · `--dialog-w` 440.

### Extrapolation rules (redesign «Variant A»)

Surfaces not covered by the design handoff follow the same language:
cards `--surface`+`--radius-lg` (padding 22–24) · nested `--surface-2`+`--radius-md` ·
fields `--surface-3`+`--radius`/`--radius-field` (46px in drawers) · caps section labels
11px/700/letter-spacing `--text-dim` · technical values in `--font-mono` (often as `.mono-chip`) ·
status pills = tint bg + coloured text + 6px dot · active nav/tab = `--accent-bg` + `--accent-text` ·
segment controls = `.seg`/`.seg-btn` (or `.seg-btn-lg` standalone) · toggles = `.toggle` 54×30.

## Primitives

`.page` (+`--page-max` override) · `.page-header`(+`.spacer`)/`.page-sub` · `.card`(+`.card--hover`)/`.card-title`
· `.section`/`.section-label` · `.muted` · `.btn`(+`-primary/-ghost/-success/-success-soft/-danger/-sm`)
· `.icon-btn`(+`.success/.danger/.accent-soft`) · `.badge`(+`-accent/-ok/-danger/-warn`) · `.chip`(+`.active`)
· `.tab-bar`/`.tab`(+`.active`)/`.tab-count` · `.seg`/`.seg-btn`/`.seg-btn-lg` · `.toggle`(+`.on`)
· `.mono`(text in the mono face) · `.mono-chip` · `.data-table`(+`-head/-row`) · `.empty-state`/`.empty-icon` · `.loading`/`.spinner`/`.spin`
· `.tinted`/`.tinted-text` (colour from data) · `.form-group`/`.form-row` · `.error-msg` · keyframes `vfade`/`vslide`.
Checkboxes and radios are marks, not fields: base.css sizes them to content with
`accent-color` (20px on the phone); do not re-declare `width: auto` locally.
A value read or copied character by character — a password, a key, a secret,
a recovery code, a fingerprint — is `.mono` (or a `<code>`), never a local
`font-family: var(--font-mono)`: base.css turns ligatures off there (and in
`.mono-chip`, `code`, `kbd`, `samp`, `pre`), so JetBrains Mono never draws
`!=` as `≠` or `->` as an arrow. A component declares no ligature rules;
a hint in such a field stays in the UI font (`::placeholder { font-family:
var(--font-ui) }`).

### A colour from data: `.tinted`

A chip, badge or name coloured from data — a workspace's colour, a label's
(Notes tags, Pass labels), a kanban column's, a foreign workspace link — is
drawn by one rule of base.css. The element takes the class and sets the
colour, nothing else:

```svelte
<span class="tag tinted" style:--chip={tag.color}>{tag.name}</span>
<span class="scope tinted-text" style:--chip={scopeColor}>{scopeName}</span>
```

- The colour is the tint under the chip (`--chip-tint` of the colour over the
  surface) and its border (`--chip-edge` of the colour over `--border-2`, so
  a colour close to the surface keeps the neutral chip's outline); the text
  is the colour mixed towards `--text` (`--chip-ink`). `.active` (a picked
  one): `--chip-tint-strong`, and the border and a 1px ring in the ink
  (`currentColor`) — the raw colour can vanish on the surface (yellow on
  white is 1.6:1), the ink cannot. A picker that hangs on the picked state
  adds a mark too (the kanban column has a check and `aria-pressed`).
  `.tinted-text`: the text alone, for a coloured name with no chip around it.
- Inside a chip, a × or a secondary text inherits the ink and fades by
  `opacity: var(--chip-fade)`; never a fixed `--text-2`/`--text-3`, which
  does not follow the tint under it.
- On the phone an `.m-chip` has no border, so its tint is mixed over
  `--m-seg` (the neutral chip) instead of the page (mobile.css).
- The ink share is computed, not chosen by eye: `chip-contrast.test.ts` takes
  the pickers' palettes, the accent and background presets of Settings, the
  colour tokens a chip takes, the extremes and a sweep of the RGB cube,
  resolves the mixes with the token values of both themes on every surface
  (with a selected row under each accent preset, at 15% and 12%) and asserts
  WCAG AA (4.5:1) for the text, 3:1 for the picked ring against the surface
  and for the faded ×. It also checks each token is within a step of the
  largest safe share (light 35%, dark 39% today; the tokens take 32% and
  36%).
- No colour (`style:--chip={undefined}`) → the neutral chip. Set the colour
  with `style:--chip`, never `style="--chip: {…}"`: an undefined value would
  still write the property.
- The component keeps the shape (padding, radius, `border-width` and
  `border-style` — not the `border` shorthand, which sets a colour) and sets
  no colour on the element. A dot, stripe or icon with no text keeps the raw
  colour. On the phone it is `.m-chip.tinted`; a workspace with no colour of
  its own passes `var(--success)`.
- Without `color-mix` (an old WebView) the chip is neutral with a border of
  the raw colour (`@supports`).
- The guard (below) fails on text in a colour from data set anywhere else:
  `style:color={…}`, a `color:` with an interpolation in a style attribute
  (over any number of lines), a `color:` built into a string (`color: ${…}`,
  `'color:' + …`, also in .ts), a `color:` reading a property some component
  sets from data. A colour handed over in an object is out of its reach.
  The few icons and state colours it excuses are listed in the script, each
  as its whole line.


## Modals & drawers

Use the shells instead of hand-rolling an overlay (that gave us z-index 20→1000 and
inconsistent widths/backdrops). A full-window backdrop is placed with
`inset: var(--overlay-inset, 0); border-radius: var(--overlay-radius, 0)`, never
`inset: 0`: on Linux the window frame is drawn by the UI, and the tokens keep an
overlay inside the rounded panel and below the title bar (the window stays movable).
On the phone `<Dialog>` presents itself as a bottom sheet with full-width buttons, and
`<ContextMenu>` as a bottom sheet of 52px rows (its x/y are ignored there):

```svelte
<Drawer open title="Edit proxy" {onclose}>
  <form>…</form>            <!-- body: scrolls, gutter-stable, min-height:0 baked in -->
</Drawer>

<Dialog open title="Import" {onclose}>
  …body…
  {#snippet footer()}<button class="btn btn-primary">Save</button>{/snippet}
</Dialog>
```

Both handle: portal to `<body>`, backdrop, Escape/backdrop close, tokenized
z-index (`--z-drawer` / `--z-modal`), `--drawer-w`/`--dialog-w` width, slide/scale
animation with `prefers-reduced-motion`.

## Guardrail

```
bash scripts/check-styles.sh
```

Fails on any `var(--name)` — in a component or a stylesheet — whose name is
neither a token of `tokens.css` / `mobile.css`, nor set in the same file, nor
listed in the script as set at run time from another file; a fallback does
not excuse an unknown name. Fails on a ligature rule outside base.css and on
text in a colour from data set outside it (`.tinted`). Reports drift counts: raw hex, raw font-size,
mobile tokens used without a fallback outside the mobile folders.
Run it in CI / before committing style changes.

## Migration status — FINALIZED

Product UI is fully on the system:
- **All 10 overlays** use the shells: `<Drawer>` (ProxyPanel, CreateProfilePanel,
  EditProfilePanel, ProfileSidePanel, RawDataPanel, PasswordGenerator, TotpGenerator)
  and `<Dialog>` (TotpAddModal, ImportProfileModal, ExportProfileModal). Fixed
  toolbars/tabs go in the Drawer `subheader` snippet; header controls in `actions`;
  bottom buttons in `footer`.
- **All routes + components** reuse the primitives (`.page`/`.page-header`/`.card`/
  `.badge`/`.chip`/`.empty-state`/`.tab-bar`/`.loading`/`.muted`), local duplicates deleted.
- **Values tokenized**: font-size snapped to `--fs-*` (raw count 358 → 9, the 9 being
  inspector dev-tooling + one inline style); exact-grid spacing → `--sp-*`; palette
  hex → colour tokens. Guard passes (every `var()` resolves).

Deliberately left local (correct, not debt): per-item **dynamic-colour** dots,
stripes and icons (the colour from data via inline style, no text; a chip or a
name in a colour from data is `.tinted`, above); bespoke tight **icon buttons** in notes/editor toolbars; the inspector
dev-tool overlay (`src/lib/inspector/*`, not product surface); off-grid spacing and
non-palette semantic hex that are intentional.

When adding UI: use a shell + primitives + tokens. Run `scripts/check-styles.sh`.
