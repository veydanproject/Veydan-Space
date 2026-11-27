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
| `src/lib/core/styles/palettes/<name>.css` | The colours of a palette, dark and light (`<html data-theme data-palette>`). `quiet.css` is the default and answers to `[data-theme]` alone; `periwinkle.css` («Барвинок и индиго», РД-DS1-01) is imported right after it with the same selectors and wins by order: it holds every colour, quiet keeps the shadows, the geometry and the call's windows. Another palette is `[data-palette=…][data-theme=…]`, imported by `tokens.css`, and may leave out what it shares with them. |
| `src/lib/core/styles/base.css` | Global reset + reusable primitives (`.btn`, `.icon-btn`, `.badge`, `.page`, `.card`, `.tab-bar`, `.empty-state`, `.spinner`, `.vtip`, …). |
| `src/lib/core/styles/mobile.css` | Phone layer on top of the two: `--m-*` colours, `--touch`, `--nav-h`, safe-area insets `--sat`/`--sab`/`--sal`/`--sar`, `.m-*` primitives. Loaded by the mobile shell only — its tokens do not exist on a computer. |
| `src/lib/core/theme.ts` | `data-theme` (dark / light) × `data-palette` (`quiet`), and the one colour a user may choose: the accent (`applyAccent`). |
| `src/lib/core/ui/*` | The shared components: `Dialog`, `Modal`, `Drawer`, `ContextMenu`, `CustomSelect`, `Pane`, `Skeleton`, `ToastHost`/`toast.svelte.ts`, `ConfirmHost`/`confirm.svelte.ts`, `tooltip.ts`, `motion.ts`, and the shared rows (`NavRow`, `Tile`, `Chev`, `Pill`, `ToggleRow`, `ChoiceRow`, `TermHint`, the action `scrollTabs`, see «Shared rows»). |

`tokens.css` and `base.css` are imported by each shell (`DesktopShell.svelte`,
`MobileShell.svelte`) and by `StartErrorScreen.svelte`; `mobile.css` by
`MobileShell.svelte` alone. A new palette is one file in `palettes/`; there is
no picker yet.

## Rules

1. **Colour** → always `var(--…)`. Never a raw hex/rgba in a component (except
   pure `#fff` on a red or green fill or over a photo or video; on the accent it is
   `--on-accent`). If you need a new colour, add it to the palette
   (`palettes/periwinkle.css`) in **both** themes.
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
   Copying is `copy` everywhere (a password, a URL), a username alone `id`; a
   password of a locked vault `lock-square-rounded`. Each copy button names what
   it copies (`ctx_action_copy_username` / `_password` / `_url`), not `common_copy`.
   A secret field has its label above it, not a `placeholder`, and its own «eye» (`pw_btn_reveal` / `pw_btn_hide`).

## Tokens cheat-sheet

- Background: `--bg` with `--bg-image` (four soft glows, `--glow-1..3`) — the
  shells draw it; nothing on top of it sets a solid `--bg` again. «Барвинок
  и индиго» keeps quiet's glow and glass: it sets the colours, not the window
  (`--glow-*`, `--pane-*`, `--gap` are quiet's or unset).
- The window's parts (unset in this palette: they fall back to `--pane`,
  quiet's glass): `--pane-top` (the window's
  head), `--pane-side` (the column of Settings' sections), `--pane-chrome`,
  `--pane-list` (a column of a list), `--pane-island` (a page with no columns),
  `--pane-chat` (a conversation, a note); text on the chrome `--chrome-text` /
  `--chrome-muted`.
- Panes (the floating columns and panels of a computer, the cards of a phone):
  `--pane` / `--pane-head` / `--pane-border` / `--pane-highlight` /
  `--pane-shadow`, the denser `--pane-strong(-border)`, blurred by
  `--pane-filter`. `Pane.svelte` and `.card` take them.
- Glass (whatever floats over the page: menus, dialogs, sheets, toasts,
  tooltips, bars): `--glass-bg`, `--glass-bg-raised`, `--glass-bar` (bars over
  scrolling content), `--glass-border`, `--glass-highlight`, `--glass-filter`.
  Two levels: windows and dialogs `--glass-bg` (90 %, blur 20), menus, dropdown
  lists, suggestions and action sheets `--glass-menu-bg` (80 %, `#2E3449` in the
  dark) with `--glass-menu-filter` (blur 32); their edge and shadow are the
  panes' (`--pane-strong-border`, `--pane-highlight`, `--pane-shadow`). Glass
  that floats over a conversation: `--glass-float*`. The light glass is neutral
  white, no lilac.
  With `prefers-reduced-transparency` or no `backdrop-filter`, all of them
  become opaque (the palette does it; components do nothing).
- Surfaces: `--chrome`/`--bg-2` < `--surface` < `--surface-2`; fields
  `--surface-3`; hovers `--surface-hover` / `--surface-row-hover`; drawers
  `--surface-drawer(-footer)`; borders `--border` / `--border-2`; the frame of a
  field, a checkbox and a radio `--control-border` (3:1 or more on a field and the page;
  under the pointer a field's frame is `--text-3`, a disabled one fades to 0.45 as a button). A one-line field and
  a select are `--control-h` 40 on a computer, `--touch` 44 on a phone (`CustomSelect` too). In focus a field's
  frame is `--accent` whole with a ring of 2 of `--focus-ring` around it (3:1 or more in both themes), under the
  mouse as under the keyboard. In a dialog or a sheet of the light theme a field is white (`--field-on-panel`) in the
  `--control-border` frame; its focus as anywhere. A checkbox or a radio under the pointer takes the `--accent` frame and 10 % of it
  inside, a checked one `--accent-hover`; a disabled one stays the palest.
- Counts and names: a muted count `--count-quiet-bg` / `--count-quiet-text`;
  the lightness of a sender's name `--tint-l` (`messenger/shared/tint.ts`).
  The terminal: `--term-bg` / `--term-fg` / `--term-cursor`, one in both themes.
- An outgoing bubble is filled with the accent: inside `.line.out .bubble` the
  palette points the text, accent and border tokens at `--bubble-out-text` /
  `--bubble-out-meta`, and `--on-accent` at `--bubble-out-bg`; an accent
  button in it is `--bubble-out-text` with `--bubble-out-bg` on it, a quote
  lies in `--bubble-out-well`; a warning and an error there are `--warn-text` /
  `--danger-text` of their own (the light's error the bubble's text, semibold).
- What lies in a bubble or a channel's post (`.line .bubble`, `.post.card`: a
  file's and a call's circle, a link card's tiles, a quiet button, a voice's
  speed, a hovered row) takes the bubble's tone, never the page's panel:
  `--bubble-tone` (its `--text` 14 % over the fill; hover `--bubble-tone-hover`
  22 %, an edge `--bubble-edge` 26 %), which `--accent-bg`, `--warn-bg`,
  `--surface-2/-3` point at there; a quiet button is the tone with no frame.
  A card in a bubble (a link's, a contact's, a bot's) is as wide as its content, 320 to
  420, a phone's the bubble's width; a file and an album are 420. Every card is
  one frame: its title 16/600, its buttons the card's width one under the other,
  the main one first, «В контактах» under them in the middle, the time under
  them; an avatar 52, a mark on a plate 40 (`--radius-md`) of its own colour at
  14 %. A contact's site, phone and profiles, and several files of one message,
  are one list in `--bubble-tone` (`--radius-md`) with a line of `currentColor`
  10 % between rows: a row 40 (a phone's 44), its sign 16, its text 14; the time
  under the list. A file's name 15/600, its size and state 12. A sender's
  name is `hsl(h 60% --tint-l)`. A message's line runs to the column's edges
  with `--radius-bar`, its menu opens under the bubble. In the light
  theme, inside `.card`, `.data-table` and `.on-glass` the second surface
  (`--surface-2/-3`, `--m-field`, `--m-seg`) is `#F4F4F6`, darker than the pane.
- Text: `--text` / `--text-2` / `--text-3`; long-form `--text-body`; extras
  `--text-soft` (icons, metrics) · `--text-faint` (field labels) · `--text-dim` (caps labels, «—»).
- Accent: `--accent` is the **fill** (primary buttons, the active switch, a
  countdown bar) — `#685DDE` light, `#ACAEFF` dark; what stands on it (text,
  glyphs, a spinner) is `--on-accent` (white light, `#24234A` dark), never `#fff`;
  text in the accent is `--accent-text` (links,
  the active item, actions of a toast), with `--accent-text-2` (mono badges) /
  `--accent-text-3`; fills `--accent-bg` / `--accent-tint`(+`-border`) ·
  `--accent-border` (the frame of the chosen) · `--focus-ring` (the keyboard focus ring: solid, 3:1 or more on the backgrounds; the accent text colour) · `--accent-grad` ·
  `--shadow-accent`. A chosen accent recomputes all of them (`applyAccent`),
  `--accent-hover` / `--accent-active` and `--on-accent` too: white or `#24234A`,
  whichever reads better over the fill, its hover and its press (`onAccentFor`).
  The keyboard's ring is one for every control: 2px of `--focus-ring`, 2px off the control, rounded
  as it is — a zero-specificity rule in `base.css`; a field draws its own ring of focus (above) and no second one. A row that clips (a scrolling row of tabs, a list) leaves 4px of room
  around its controls so the ring is whole; no ring is drawn inside a control.
- Touch (phone layer): `--touch` 44 is the least size of a finger zone; a control smaller than it grows an invisible `::before` to it per axis (`mobile.css`), capped by `--hit-max`, which a control with close neighbours lowers.
- Chat: `--bubble-in-*` / `--bubble-out-*`; a held row or bubble `--press-tint`.
- Semantic: `--success*` (+`-border`, `-grad`), `--danger*` (+`-border`, `--shadow-danger`), `--warn-*` (+`-border`). White text or a mark on green lies on `--success-grad` (#1D7448, as `.btn-success`), never on `--success` (#5CCB8A in the dark, about 2:1); `--success` alone is a dot or an edge. `.btn-success` under the pointer and pressed is darker in both themes (`--success-hover` #19663F, `--success-active` #155837: white text 7.0 and 8.5).
- Category colours (workspaces / proxy types): `--cat-purple` / `--cat-blue` / `--cat-teal` / `--cat-pink`.
- Fonts: `--font-ui` (Manrope Variable) · `--font-mono` (JetBrains Mono Variable — secrets, hosts, versions, IDs).
- Spacing (4px grid): `--sp-1`=4 … `--sp-6`=24, `--sp-8`=32, `--sp-10`=40, `--sp-12`=48.
- Type: `--fs-xs` 12 (captions) · `--fs-sm` 13 (secondary) · `--fs-base` 14 (controls) ·
  `--fs-body` 15 (messages, long text; `--lh-body` 22) · `--fs-md` 16 (section titles) ·
  `--fs-lg` 18 (dialog titles) · `--fs-xl` 20 (pane titles) · `--fs-2xl` 28 (H1); weights up to `--fw-extrabold` 800.
- Radius: `--radius-xs` 4 · `--radius-badge` 6 · `--radius-sm` 8 (menu items) ·
  `--radius` 10 (every button, a small one and an icon button too; fields; a plate's quiet button is a pill) · `--radius-md` 12 (menus, nested) · `--radius-lg` 16 (list rows) ·
  `--radius-head` 18 (header plates) · `--radius-bubble` 18 (message bubbles; a picture in one: 15) · `--radius-pane` 20 (panes) · `--radius-card` 24 (cards, dialogs, sheets) ·
  `--radius-bar` 26 (the phone's floating bars) · `--radius-pill`.
- Size: `--topbar-h` 64 · `--control-h` 40 · `--control-h-sm` 32 · `--control-h-lg` 44 (touch) ·
  `--search-h` 40 / `--search-fs` 14 (a search field on every screen) ·
  `--drawer-w` 440 · `--drawer-w-md` 480 · `--drawer-w-lg` 520 · `--dialog-w` 440.
- Motion: `--dur-press` 100 ms · `--dur-fast` 150 · `--dur-base` 200 · `--dur-drawer` 280; `--ease`, `--ease-drawer`.

### The language of the panes

A computer: the module's columns are panes 12 apart and 12 from the window's
edges (`fullWidth` routes). A column's head (`Pane`, the messenger's `.rail-head`, the notes' list, Settings'
both heads) is one line of 36, 16 from the top, with no band and no rule under it: the title 18 semibold, the number
of entries `--fs-xs` `--text-2` on its line (`Pane`'s `subtitle`), a marker beside the title (`titleBadge`, the
passwords' lock 16), the buttons on the right 36 × 36 with 8 between; 16 from it to the first item (a search, a
banner, the first row). A Drawer's head and body start 16 from its edge, a Drawer's and a Dialog's body keep no gutter
for the scrollbar (`scrollbar-gutter: auto`), so the left and right insets are equal. The columns of navigation (the
messenger's rail `.sections`, the notes' `.sidebar`, Settings' `.settings-side`, the phone's ☰ of Notes, the rail of
an open Duo and an iPad) are the window's head (`--pane-top` / `--pane-side`, falling back to quiet's glass `--pane`);
a computer's bar of the modules (`DesktopShell .topbar`) and the messenger's two columns (`.sections`, `.rail`) are the
glass of the conversation's head: 60 % of `--glass-bar`, `--pane-strong-border`, `--pane-filter`, solid `--bg-2` where
the system asks for less transparency or the engine draws no blur; a phone's search pill of a list (Chats, Groups,
Channels, Contacts) is the same glass in the dark (the light keeps its `#ECECF2` field);
in the dark what is chosen and the search lie a step lighter than the column (one rule in `periwinkle.css`), and the
rail's chosen item has its icon and label in the accent. Between the groups of a navigation one rule (`--divider`), 8
over and 8 under it; a list of sections that scrolls under a title fades under it (`mask-image`) once scrolled;
the top navigation is one floating plate (`--pane-top`). Around the panes lies the
window's glow; a page with no columns is one island like them (`main.content--page`,
`--pane-island`, 12 from the frame, `--radius-pane`): no text lies on the frame. A conversation (a chat, a group, a channel's feed) is open canvas on the window: its column of the messenger draws no plate and no blur of its own (a blur inside a blur sees nothing of the feed in WebKit), only the glass floating in it blurs; a computer's feed keeps 12 for its scrollbar on the right (`scrollbar-gutter: stable`) and 12 on the left, so its column stays under the head and over the field, and «вниз» stands at the column's right edge. It is one column in the middle, the field's (`min(720, 100% − 2 × --col-gutter)`, the gutter 12, a phone's 8): its head a plate of glass 58 high with `--radius-bar` (`.head.glassfloat`, a phone's `.head.phone`) that the feed runs under, in the dark the head is `--glass-bar` at 60 %, so the feed shows through it, solid `--bg-2` where the system asks for less transparency or draws no blur (`periwinkle.css`); the search of the conversation a second plate of the same glass, of the head's width and edges, 8 under it, `--search-h` 40 (a phone's in the light `#ECECF2` as any phone's search), its cross 32 with a zone of 44; the plates under the head, the feed and the field floating at the foot; the field's pill (`.row::before`) is `--glass-bar` at 80 % with the blur, on a computer as on a phone; where nothing may be written a plate of the field's height 52, shape and fill (`--pane-strong`) stands in its place. No line runs across the pane. A card on the window background (`.card`, the phone's `.m-card`) is the glass `--pane-strong` on both a computer and a phone; what lies on it (fields, chips, tiles, quiet buttons) takes the same glass through one token rule in `base.css`; the same rule is given by the panes and floating layers themselves (`core/ui/Pane`, `Drawer`, `Dialog`, `Modal`, the messenger and notes columns, `.data-table`) and by a local block that draws its own look through the class `.on-glass` (tokens only, no box). A field, a quiet button (`.btn-ghost`), the tile of an empty state (`.empty-icon`), the segments (`.tab-bar`, `.seg`) and a checkbox are glass by default, not `--surface-3`: a solid surface is black over the glow. Their line is `--pane-strong-border` in the dark and `--border` in the light; a field's, a checkbox's and a radio's is `--control-border`. A phone: a floating head, cards of
rows on the glowing background, a floating tab bar, a 56 capsule (`.m-bar--nav`; `--nav-h` is its slot: 56 plus 12 of air; the bar stands on the field's column, `--col-gutter` or the system's insets where wider; the chosen tab has its icon and label in `--accent-text` on 24 % of the accent; a count stands at the icon's top right corner, inside the bar, in a ring of `--glass-bar`; the bar has no logo, a tab's header carries it (`core/ui/HubButton`), and Space's Home has no bar; `--nav-gap` keeps the last
row above it; a list over it dissolves in 24 at its foot, as under a head in 20 at its top). The first thing under a phone's head stands 8 from it (`.m-header + .m-search`, `.m-header + .m-seg`). A phone's search field in the light is `#ECECF2` in a frame of `rgba(20,21,31,.14)`, in focus the accent's frame (`periwinkle.css`; Chats' `.search` pill doubled as `.search.search:focus-within` to outweigh the field's `:is()`). A wide window with nothing open beside the list shows the list's `SplitDef.empty` in the middle (`.split-empty`: an icon, a title, a grey line); a note there is a column of 720 at most in the middle, its toolbar a group 8 apart. An iPhone Duo is told by the window (466 wide in portrait, 951 in landscape): `--duo-pill` (80) keeps heads, searches, tabs and the first blocks off its camera pill, the first rows start under it, beside a list only the detail moves (`mobile.css`, the end). A list row is 56–64 high with `--radius-lg`; the active one is
`--accent-bg` with `--accent-text`; segment controls are `.tab-bar`. One kind of switch everywhere: tabs with an
underline (`.tab-bar`, `.seg`, the phone's `.m-seg`) — words on the surface without a frame or fill, 20 between them,
the chosen word in `--accent-text` over a bar of 3 of `--accent`, the others `--text-2`, darker under the pointer.
Two sizes: a page's sections 44 (`core/ui/Tabs`, `.tab-bar.pages`: Terminal, Passwords, a workspace), panels,
filters and windows 32; the row starts at the title's edge, the bar runs from the first sign to the last (icon, word,
count), a row of equal parts (Plans' periods) keeps the bar under the word. In a
window the tabs stand at the left and the chosen one is bold (`.tab-bar.plain`). Only a form's control (the
Terminal's connection kind and key source: `:is(.form, .form-group) > .seg`) stays a capsule. A search
field is a pill. A list's search is opened by a lens in its head (Notes, Passwords, the messenger's lists on a
computer and in a wide window of a phone, `messenger/pages/ListSearch`): the lens 36 on `--surface-2` before the
accent's «new», of its size, 8 between; a press opens the field 16 under the head with the focus in it, the lens
marked while it is open or holds words; Esc and the cross clear the words, in an empty field close it; another
section closes it, its words gone. A phone in one column keeps the field always.

## Primitives

`.page` (+`--page-max` override) · `.page-header`(+`.spacer`)/`.page-sub` · `.card`/`.card-title`
· `.section`/`.section-label` · `.muted` · `.btn`(+`-primary/-ghost/-success/-success-soft/-danger/-danger-soft/-sm`)
· `.icon-btn`(+`.success/.danger/.danger-soft/.accent-soft/.create`) · `.badge`(+`-accent/-ok/-danger/-warn`; through `core/ui/Pill`) · `.win-close` (the × of Dialog, Drawer, BottomSheet) · `.chip`(+`.active`)
· `.tab-bar`(at the left, 20 between, 32; +`.pages`: a page's sections, 44; +`.plain`: a window's, the chosen one bold)/`.tab`(+`.active`: `--accent-text` over a bar of `--accent`; its count a `.count-pill.sm.quiet`) · `.seg`(tabs with an underline; +`.seg-fill`: across the card, options of one width, 8 between)/`.seg-btn` · `.toggle`(+`.on`; disabled, or inside a disabled row: an empty track with a frame of `--border-2` and a pale knob, no accent)
· `.mono`(text in the mono face) · `.mono-chip` · `.data-table`(+`-head/-row`) · `.empty-state`/`.empty-icon` · `.loading`/`.spinner`/`.spin`
· `.tinted`/`.tinted-text` (colour from data) · `.form-group`/`.form-row` · `.error-msg` · keyframes `vfade`/`vslide`
· `.list-rows` > `li` > `.list-row`(+`.active/.unread`) — the one row of a list pane (chats, groups, channels, calls,
  contacts, requests): 64 high, an avatar of 48, `.list-row-body` with `.list-row-top` (`.list-row-name`, then the
  marks `.list-row-mark` — kind, muted, pinned, 14 in `--text-3` — then `.list-row-time` at the right) over
  `.list-row-bottom` (`.list-row-line`, then the `.count-pill.lg`). A hairline of `--divider` between two rows, from
  the text (72 from the left) to 12 from the right, hidden beside a hovered or chosen row; an `li` without a row
  (a label, the rule after the pinned) breaks it. A row with buttons at its end: a box `.list-row` with a
  `.list-row-main` button. `.unread`: the name 800, the line `--text` 600, the time in `--accent-text`.
  What the line is (a photo, a file, a contact, a link) is an icon 14 before its words (`.line-icon`, `bodyParts`),
  in the line's colour, never an emoji; a notification keeps its emoji (`bodyLine`).
· `.search-field input` (the filter bar) — a radius `--radius` (10), like the lists beside it; `.m-search` and the
  chats' search stay a capsule. The space board: a column's count is plain `--text-2` text at the right edge, after
  its buttons; `.board` fades 28 at the right edge only while there is more to scroll (`.scrolls:not(.at-end)`).
· `.list-cover`(+`.active`)/`.list-cover-corner`(+`.muted/.missed`)/`.list-cover-empty` (a list pane at 72, a column of covers).
· `.count-pill` — the one counter everywhere (a section of the rail, a row of a list, a cover's corner, ⋮, a tile,
  a tab, a block's head): a pill never narrower than tall, the number centred, bold, tabular figures. Sizes:
  `.sm` 18 (`--fs-xs`: a corner, ⋮, a tab), 20 by default (`--fs-xs`: a block's head), `.lg` 22 (a row of a list).
  Colours: the accent with `--on-accent` (unread, the default), `.quiet` (`--count-quiet-bg` / `--count-quiet-text`: muted, or only a total),
  `.danger` (missed, errors). A corner count adds only its position and a 2px ring of `--chrome`.
  Never a rounded square (`--radius-badge` is for kind chips, not for numbers).
· `.menu-count` (the count on a head's ⋮: a compact `.count-pill` of 16 under its own name, `top: -10px; right: -4px` on the button's upper right corner, on computer and phone alike, the dots seen whole; the button is `position: relative`) · `.count-badge` (a filter row's count of results: plain `--text-2` words, `--fs-sm`, as high as the fields beside it, no frame).
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

## Shared rows

A row that repeats in several modules is one component of `core/ui`; a module
keeps no copy of its own.

- **`NavRow`** — a row that leads somewhere: a tile with the icon, the title,
  a line `sub` (text or a snippet), at its end `count` (a `.count-pill.sm.quiet`)
  or `end` (a snippet: a `Pill`, a word, a control), the chevron
  (`chevron`; by default when it has `href` or `onclick`). The whole row is the
  link (`href`) or the button (`onclick`); `disabled` fades it. `tone` colours
  the tile, `danger` the title and the icon (`--danger-text`, the hover
  `--danger-bg`, as a danger item of ContextMenu; every variant), `nowrap` cuts the title with «…», `page` sets
  `data-page` so the focus comes back to the row from its page, `id` is an
  anchor. `variant`: `panel` (the default: the contents of «О канале» /
  «О группе», 56 / 60 on touch, in a card of rows, the root `.nav`), `list`
  (the lists of the cloud: 64, a hairline under it, the tile in the accent,
  `end` in place of the chevron), `compact` (a line of a block, «Медиа и
  файлы»: 46 / 50, the icon without a tile).
- **`Tile`** — the square of an icon at the start of a row: 36, `--radius`,
  the icon 18; `tone` `plain` · `ok` · `warn` · `bad` · `accent` · `clear` ·
  `dashed` (the place of a new one).
- **`Chev`** — the chevron of a row that leads further: 16, `--text-3`.
- **`Pill`** — a word in a pill, the one for the whole interface: `kind`
  `badge` (the default: a mark beside a name or in a cell, framed, the look of
  `.badge` in base.css — markup goes through the component) or `state` (a state
  at the end of a row or a head: 22, `--fs-xs` semibold, no frame); `tone`
  `ok` · `warn` · `bad` · `accent` · `neutral`; `onclick` makes it a button.
- **`ToggleRow`** — a yes / no setting as a card, the whole card the
  `button role="switch"` (`aria-checked`), the `.toggle` inside only drawn:
  `title`, `hint`, `on` (bindable), `onclick` (what the press does; without it
  the press flips `on`), `disabled`, `label` (an accessible name of its own).
  64 high, `--radius-lg`, the title 14 semibold, the hint 13 `--text-2`;
  `surface` `strong` (a dialog, `--pane-strong`) or `panel` (`--surface-2`
  with `--border`). A switch that is a button of its own beside a label
  (`aria-pressed`, the settings of the messenger) stays as it is.
- **`ChoiceRow`** — one choice of a few, the whole row the `button role="radio"`
  (`aria-checked`), the caller wraps the rows in a `role="radiogroup"` with its
  label: `icon`, `title`, `hint`, `checked`, `onclick`, `disabled`. 60 high,
  `--radius-md`, a tile of 36 (`--radius-sm`), the title 14 bold, the hint 13
  `--text-2`, the check 18; the chosen one `--accent-bg` / `--accent-border`.
  `surface` `dialog` (`--surface-3`) or `panel` (`--surface-2`). A choice
  made of native radios (`<label>` + `<input type="radio">`) stays as it is.
- **`TermHint`** — the «?» beside a term where a thing is made (a new level, a
  new section): `term` and the note as its children; the note opens under the
  row of the term (the nearest `position: relative` box), up to 440 wide,
  pointing at the «?», and the same button closes it. In a dialog's title it
  goes into `titleAside`.
- **`scrollTabs`** (`core/ui/scrollTabs.ts`, an action) — a row of tabs that
  scrolls sideways (the row keeps its own `overflow-x: auto` and hidden
  scrollbar): `use:scrollTabs={{ selected, watch, wheel, fadeStart }}`. It marks
  a clipped edge with `fade-end` (and, with `fadeStart`, `fade-start`) for the
  row's own mask (`.tabs:global(.fade-end) { mask-image: … }`), keeps the chosen
  tab (`[aria-selected="true"]`) in view when `selected` changes by moving the
  row alone (`scrollTo`, never `scrollIntoView`), measures again on a resize and
  when `watch` changes (the tabs, their counts), and with `wheel` turns a
  mouse's wheel into sideways scrolling.

## Feedback

- **Toast** — `toast.success/secret/undo/error/info` (`core/ui/toast.svelte.ts`),
  one at a time, shown by `ToastHost` in each shell (a computer: bottom right;
  a phone: at the top under the status bar). Copying says `copied()`; a secret
  goes through `copySecret` (Pass) or `secretCopied()` — 30 s with
  «Очистить сейчас». An undoable action shows `toast.undo` with its undo; a
  failed action `toast.error` with its retry. No local toasts.
- **Confirmation** — `ask({ title, message, confirmLabel, variant, icon })`
  (`core/ui/confirm.svelte.ts`), the app's only one (the messenger's
  `confirmStore.ask` goes there too, no `<Modal>` of its own anywhere): the
  title (`confirmTitle(label)`, «Удалить чат?»), the text, «Отмена» on the
  left; a danger question has the red verb and the red tile with its action's
  icon (`trash-2`, `logout`, `user-minus`, `user-cancel`; default
  `alert-triangle`) beside the title. A Modal on a computer, a sheet on the
  phone with the buttons stacked the whole width, the verb on top, no ×.
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
there). A long press on a row opens that sheet everywhere, no other kind: its `header` names what the items act
on (the row's avatar 36 and its name in the body's size, semibold; a note's name alone), the plain items, one
line (`{ type: 'separator' }`), the red ones under it:

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
