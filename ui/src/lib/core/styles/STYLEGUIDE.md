# Veydan — UI style guide

The look of every product (Space, Chat, Pass, Notes; computer and phone) is
«Тихое стекло» (Quiet Glass): floating glass panes over a softly glowing
background, one purple accent, quiet text. This file and the tokens are the
single source of truth for styling. **Before adding CSS, check whether a
token or primitive already exists.**

## Layers

| File | Role |
|------|------|
| `src/lib/core/styles/tokens.css` | Design tokens — spacing, type, radius, z-index, sizes, motion. With the palettes, the **only** place raw values live. |
| `src/lib/core/styles/palettes/<name>.css` | The colours of a palette, dark and light (`<html data-theme data-palette>`). `quiet.css` is the default and answers to `[data-theme]` alone; another palette is `[data-palette=…][data-theme=…]`, imported by `tokens.css`, and may leave out what it shares with quiet. |
| `src/lib/core/styles/base.css` | Global reset + reusable primitives (`.btn`, `.icon-btn`, `.badge`, `.page`, `.card`, `.tab-bar`, `.empty-state`, `.spinner`, `.vtip`, …). |
| `src/lib/core/styles/mobile.css` | Phone layer on top of the two: `--m-*` colours, `--touch`, `--nav-h`, safe-area insets `--sat`/`--sab`/`--sal`/`--sar`, `.m-*` primitives. Loaded by the mobile shell only — its tokens do not exist on a computer. |
| `src/lib/core/theme.ts` | `data-theme` (dark / light) × `data-palette` (`quiet`), and the one colour a user may choose: the accent (`applyAccent`). |
| `src/lib/core/ui/*` | The shared components: `Dialog`, `Modal`, `Drawer`, `ContextMenu`, `CustomSelect`, `Pane`, `Skeleton`, `ToastHost`/`toast.svelte.ts`, `ConfirmHost`/`confirm.svelte.ts`, `tooltip.ts`, `motion.ts`. |

`tokens.css` and `base.css` are imported by each shell (`DesktopShell.svelte`,
`MobileShell.svelte`) and by `StartErrorScreen.svelte`; `mobile.css` by
`MobileShell.svelte` alone. A new palette is one file in `palettes/`; there is
no picker yet.

## Rules

1. **Colour** → always `var(--…)`. Never a raw hex/rgba in a component (except
   pure `#fff` on a coloured button or over a photo or video). If you need a
   new colour, add it to the palette (`palettes/quiet.css`) in **both** themes.
2. **Spacing / radius / font-size** → use `--sp-*`, `--radius*`, `--fs-*`. Don't
   invent `0.42rem`.
3. **z-index** → use `--z-*` (`--z-drawer`, `--z-modal`, `--z-toast`, …). Never a
   bare number.
4. **Never invent a token name.** Only names defined in `tokens.css`, the palettes
   (and, on the phone, `mobile.css`) exist. A
   `var(--made-up)` with no fallback renders as transparent/inherited. Run the
   guard (below) — it fails on undefined references. A custom property a
   component sets for itself (`--r` in its own `<style>`, `style:--c={…}` on
   an element) is fine as long as it is read in the same file.
5. **Reuse primitives and components.** Need a badge/tab/empty-state/card, a
   menu, a confirmation, a toast? Use the global class or the shared
   component. Only add scoped CSS for a genuine one-off delta.
6. **Icons** are `Icon.svelte`: Tabler Icons (MIT, outline, stroke 1.5), keyed
   by our own names. 18 in buttons, 22 in navigation. A new icon is a new key
   with the Tabler drawing nearest to its meaning; no other icon set.

## Tokens cheat-sheet

- Background: `--bg` with `--bg-image` (four soft glows, `--glow-1..3`) — the
  shells draw it; nothing on top of it sets a solid `--bg` again.
- Panes (the floating columns and panels of a computer, the cards of a phone):
  `--pane` / `--pane-head` / `--pane-border` / `--pane-highlight` /
  `--pane-shadow`, the denser `--pane-strong(-border)`, blurred by
  `--pane-filter`. `Pane.svelte` and `.card` take them.
- Glass (whatever floats over the page: menus, dialogs, sheets, toasts,
  tooltips, bars): `--glass-bg`, `--glass-bg-raised`, `--glass-bar` (bars over
  scrolling content), `--glass-border`, `--glass-highlight`, `--glass-filter`.
  With `prefers-reduced-transparency` or no `backdrop-filter`, all of them
  become opaque (the palette does it; components do nothing).
- Surfaces: `--chrome`/`--bg-2` < `--surface` < `--surface-2`; fields
  `--surface-3`; hovers `--surface-hover` / `--surface-row-hover`; drawers
  `--surface-drawer(-footer)`; borders `--border` / `--border-2`.
- Text: `--text` / `--text-2` / `--text-3`; long-form `--text-body`; extras
  `--text-soft` (icons, metrics) · `--text-faint` (field labels) · `--text-dim` (caps labels, «—»).
- Accent: `--accent` is the **fill** (primary buttons, the active switch, a
  countdown bar) — `#7066c6`; text in the accent is `--accent-text` (links,
  the active item, actions of a toast), with `--accent-text-2` (mono badges) /
  `--accent-text-3`; fills `--accent-bg` / `--accent-tint`(+`-border`) ·
  `--accent-border` (also the keyboard's focus ring) · `--accent-grad` ·
  `--shadow-accent`. A chosen accent recomputes all of them (`applyAccent`).
- Chat: `--bubble-in-*` / `--bubble-out-*`; a held row or bubble `--press-tint`.
- Semantic: `--success*` (+`-border`, `-grad`), `--danger*` (+`-border`, `--shadow-danger`), `--warn-*` (+`-border`).
- Category colours (workspaces / proxy types): `--cat-purple` / `--cat-blue` / `--cat-teal` / `--cat-pink`.
- Fonts: `--font-ui` (Manrope Variable) · `--font-mono` (JetBrains Mono Variable — secrets, hosts, versions, IDs).
- Spacing (4px grid): `--sp-1`=4 … `--sp-6`=24, `--sp-8`=32, `--sp-10`=40, `--sp-12`=48.
- Type: `--fs-xs` 12 (captions) · `--fs-sm` 13 (secondary) · `--fs-base` 14 (controls) ·
  `--fs-body` 15 (messages, long text; `--lh-body` 22) · `--fs-md` 16 (section titles) ·
  `--fs-lg` 18 (dialog titles) · `--fs-xl` 20 (pane titles) · `--fs-2xl` 28 (H1); weights up to `--fw-extrabold` 800.
- Radius: `--radius-xs` 4 · `--radius-badge` 6 · `--radius-sm` 8 (menu items, small buttons) ·
  `--radius` 10 (buttons, fields) · `--radius-md` 12 (menus, nested) · `--radius-lg` 16 (list rows) ·
  `--radius-head` 18 (header plates) · `--radius-pane` 20 (panes) · `--radius-card` 24 (cards, dialogs, sheets) ·
  `--radius-bar` 26 (the phone's floating bars) · `--radius-pill`.
- Size: `--topbar-h` 64 · `--control-h` 40 · `--control-h-sm` 32 · `--control-h-lg` 44 (touch) ·
  `--drawer-w` 440 · `--drawer-w-md` 480 · `--drawer-w-lg` 520 · `--dialog-w` 440.
- Motion: `--dur-press` 100 ms · `--dur-fast` 150 · `--dur-base` 200 · `--dur-drawer` 280; `--ease`, `--ease-drawer`.

### The language of the panes

A computer: the module's columns are panes 12 apart and 12 from the window's
edges (`fullWidth` routes), a pane's head is 64 high with its title 20/700,
the top navigation is one floating plate. A phone: a floating head, cards of
rows on the glowing background, a floating tab bar (`--nav-gap` keeps the last
row above it). A list row is 56–64 high with `--radius-lg`; the active one is
`--accent-bg` with `--accent-text`; segment controls are `.tab-bar`; a search
field is a pill.

## Primitives

`.page` (+`--page-max` override) · `.page-header`(+`.spacer`)/`.page-sub` · `.card`(+`.card--hover`)/`.card-title`
· `.section`/`.section-label` · `.muted` · `.btn`(+`-primary/-ghost/-success/-success-soft/-danger/-sm`)
· `.icon-btn`(+`.success/.danger/.accent-soft`) · `.badge`(+`-accent/-ok/-danger/-warn`) · `.chip`(+`.active`)
· `.tab-bar`/`.tab`(+`.active`)/`.tab-count` · `.seg`/`.seg-btn`/`.seg-btn-lg` · `.toggle`(+`.on`)
· `.mono`(text in the mono face) · `.mono-chip` · `.data-table`(+`-head/-row`) · `.empty-state`/`.empty-icon` · `.loading`/`.spinner`/`.spin`
· `.tinted`/`.tinted-text` (colour from data) · `.form-group`/`.form-row` · `.error-msg` · keyframes `vfade`/`vslide`.
A button and an icon button sink to .97 under the press (`--dur-press`); the
phone's `.m-ibtn` and `.m-btn-grad` too.
Checkboxes and radios are marks, not fields: base.css sizes them to content with
`accent-color` (20px on the phone); do not re-declare `width: auto` locally.
A value read or copied character by character — a password, a key, a secret,
a recovery code, a fingerprint — is `.mono` (or a `<code>`), never a local
`font-family: var(--font-mono)`: base.css turns ligatures off there (and in
`.mono-chip`, `code`, `kbd`, `samp`, `pre`), so JetBrains Mono never draws
`!=` as `≠` or `->` as an arrow. A component declares no ligature rules;
a hint in such a field stays in the UI font (`::placeholder { font-family:
var(--font-ui) }`).

## Feedback

- **Toast** — `toast.success/secret/undo/error/info` (`core/ui/toast.svelte.ts`),
  one at a time, shown by `ToastHost` in each shell (a computer: bottom right;
  a phone: at the top under the status bar). Copying says `copied()`; a secret
  goes through `copySecret` (Pass) or `secretCopied()` — 30 s with
  «Очистить сейчас». An undoable action shows `toast.undo` with its undo; a
  failed action `toast.error` with its retry. No local toasts.
- **Confirmation** — `ask({ title, message, confirmLabel, variant })`
  (`core/ui/confirm.svelte.ts`): a Modal on a computer, a sheet on the phone
  with the buttons stacked the whole width, the verb on top, no ×.
- **Tooltip** — a `title` is enough: on a computer `tooltip.ts` shows every
  title as the glass `.vtip` (500 ms for a mouse, at once for the keyboard's
  focus; a shortcut at the end, «Найти (Ctrl+F)», becomes a key chip). An
  icon button still has its `aria-label`.
- **Loading** — a list that is coming shows `<Skeleton>` rows of its shape,
  never «Загрузка…» or a spinner over the page.
- **Unsaved form** — its actions stay in a `--glass-bar` at the foot of the
  pane with «Есть несохранённые изменения» and a `--warn-text` dot.

## Motion

`core/ui/motion.ts` holds the ways in and out of the layers, as Svelte
transitions; reduced motion stops all of them:
dialogs, modals and the palette `in:dialogIn out:dialogOut` with
`in:backdropIn out:backdropOut` on the backdrop (200 / 150 ms); drawers
`out:drawerOut` (280 in by CSS, 200 out); the phone's sheets
`in:sheetIn out:sheetOut` (280 / 200, out from where the finger left them);
a computer's menu grows from the point in 150 ms and leaves by `menuOut`
(100). Hover and press colours take `--dur-fast`.

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
  the pickers' palettes, the accent presets of Settings, the colour tokens a
  chip takes, the extremes and a sweep of the RGB cube, resolves the mixes
  with the token values of both themes on every surface, pane and glass
  (with a selected row under each accent preset: 18% in the dark, 12% in the
  light, and 12%) and asserts WCAG AA (4.5:1) for the text, 3:1 for the
  picked ring against the surface and for the faded ×. It also checks each
  token is within 8 points of the largest safe share (the tokens take 30% in
  the dark, 32% in the light).
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


## Modals, drawers, menus

Use the shells instead of hand-rolling an overlay. A full-window backdrop is
placed with `inset: var(--overlay-inset, 0); border-radius: var(--overlay-radius, 0)`,
never `inset: 0`: on Linux the window frame is drawn by the UI, and the tokens
keep an overlay inside the rounded panel and below the title bar (the window
stays movable). On the phone `<Dialog>` presents itself as a bottom sheet (its
footer stacked, the verb on top; `closable={false}` for a question whose footer
has «Отмена»), and `<ContextMenu>` as a sheet of 52px rows (its x/y are ignored
there):

```svelte
<Drawer open title="Edit proxy" {onclose}>
  <form>…</form>            <!-- body: scrolls, gutter-stable, min-height:0 baked in -->
</Drawer>

<Dialog open title="Import" {onclose}>
  …body…
  {#snippet footer()}
    <button class="btn btn-ghost">Cancel</button>
    <button class="btn btn-primary">Save</button>
  {/snippet}
</Dialog>
```

They handle: portal to `<body>`, backdrop, Escape/backdrop close, tokenized
z-index (`--z-drawer` / `--z-modal` / `--z-popover`), `--drawer-w`/`--dialog-w`
width, the motion above. A computer's `<ContextMenu>` also takes the keys:
↓/↑ (round), Home/End, the first letters, Enter, Tab; Escape gives the focus
back.

## Guardrail

```
bash scripts/check-styles.sh
```

Fails on any `var(--name)` — in a component or a stylesheet — whose name is
neither a token of `tokens.css` / `palettes/` / `mobile.css`, nor set in the same file, nor
listed in the script as set at run time from another file; a fallback does
not excuse an unknown name. Fails on a ligature rule outside base.css and on
text in a colour from data set outside it (`.tinted`). Reports drift counts: raw hex, raw font-size,
mobile tokens used without a fallback outside the mobile folders.
`chip-contrast.test.ts` checks the contrast of the chips against the
palettes. Run both in CI / before committing style changes.

Deliberately left as raw values (correct, not debt): the colour pickers'
palettes and the workspace colours (data), white and black over photos and
video (calls, media, the camera), the inspector dev-tool overlay
(`src/lib/core/inspector/*`, not product surface).

When adding UI: use a shell + primitives + tokens. Run `scripts/check-styles.sh`.
