<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The one list of suggestions under a field while it is typed in (design kit, Unify-suggest): labels,
  links to profiles, workspaces, notes and TOTP, a server, an SSH connection, `[[` and `{{` of a note.
  The glass of the dropdown list (CustomSelect .dropdown) over everything, in the body of the page:
  nothing under it moves. 7 under its field and as wide as it; under the line of the caret instead
  when `caret` gives one. Six rows of 36, the rest scroll inside it (the app's bar, base.css), the
  scroll does not go on to the page under it. The room under the field ends at the top of a pinned
  footer (a dialog's, a drawer's, a pane's): fewer rows where six do not fit, but no fewer than
  three — else it opens over the field. Rows of several kinds have a small quiet heading per kind.
  The field keeps the focus and gives its keys to `keydown`: ↑/↓ choose a row, Enter takes the
  chosen one (none chosen: the field's own Enter), Escape closes.
-->
<script module lang="ts">
  export interface SuggestItem {
    /** Unique in the list. */
    key: string;
    text: string;
    /** The heading of the row's kind; shown where the list has more than one kind. */
    group?: string;
    /** A colour dot before the text (a label, a folder, a workspace). */
    dot?: string;
    /** The mark of an entity kind before the text (ChipMark). */
    mark?: string;
    /** An icon before the text. */
    icon?: string;
    /** Quieter words at the end of the row (a host, a value). */
    hint?: string;
    mono?: boolean;
    /** «Create …»: the accent's words. */
    create?: boolean;
    /** The value the field has now. */
    current?: boolean;
  }
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';
  import { portal } from '$lib/core/portal';
  import Icon from '$lib/core/Icon.svelte';
  import ChipMark from './ChipMark.svelte';

  interface Props {
    items: SuggestItem[];
    /** The field the list is laid under; its dialog's footer is the floor of the list. */
    anchor: HTMLElement | null | undefined;
    /** The caret's line in the screen (`[[`, `{{`): the list goes under it, as wide as its rows. */
    caret?: () => { left: number; top: number; bottom: number } | null;
    /** The chosen row; -1: none. */
    active?: number;
    /** The row chosen when the rows change: -1 where Enter writes what is typed. */
    first?: number;
    label?: string;
    /** The words of an empty list; none: an empty list is not shown. */
    empty?: string;
    onpick: (item: SuggestItem, index: number) => void;
    onclose?: () => void;
    /** Inside the list before its «create» row (or at its end): the colours of a new label. */
    extra?: Snippet;
    /** At the end of the list. */
    foot?: Snippet;
  }

  let {
    items, anchor, caret, active = $bindable(-1), first = -1, label, empty,
    onpick, onclose, extra, foot,
  }: Props = $props();

  /** Rows shown before the list scrolls; the fewest it opens under its field with. */
  const ROWS = 6;
  const LEAST = 3;
  const GAP = 7;
  const EDGE = 4;

  const uid = $props.id();
  let el = $state<HTMLDivElement | null>(null);
  let place = $state('visibility: hidden');

  const grouped = $derived(new Set(items.map((i) => i.group ?? '')).size > 1);
  const firstCreate = $derived(items.findIndex((i) => i.create));

  // New rows (another query): the chosen one is the first again.
  let seen = '';
  $effect.pre(() => {
    const sig = items.map((i) => i.key).join('\n');
    if (sig === seen) return;
    seen = sig;
    active = first < items.length ? first : -1;
  });

  $effect(() => {
    void active;
    el?.querySelector('.sg-row.on')?.scrollIntoView({ block: 'nearest' });
  });

  /** The field that keeps the focus: the anchor itself or the input inside it (or the focused one in it). */
  function fieldOf(node: HTMLElement): HTMLElement | null {
    const sel = 'input, textarea, [contenteditable="true"]';
    if (node.matches(sel)) return node;
    const focused = document.activeElement;
    if (focused instanceof HTMLElement && node.contains(focused) && focused.matches(sel)) return focused;
    return node.querySelector<HTMLElement>(sel);
  }

  // A screen reader hears the chosen row while the focus stays in the field (aria-activedescendant).
  const shown = $derived(items.length > 0 || !!empty || !!foot);
  $effect(() => {
    if (!anchor || !shown) return;
    const field = fieldOf(anchor);
    if (!field) return;
    field.setAttribute('aria-controls', `${uid}-list`);
    field.setAttribute('aria-expanded', 'true');
    field.setAttribute('aria-autocomplete', 'list');
    return () => {
      for (const a of ['aria-controls', 'aria-expanded', 'aria-autocomplete', 'aria-activedescendant']) field.removeAttribute(a);
    };
  });
  $effect(() => {
    if (!anchor || !shown) return;
    const field = fieldOf(anchor);
    if (!field) return;
    if (active >= 0 && active < items.length) field.setAttribute('aria-activedescendant', `${uid}-${active}`);
    else field.removeAttribute('aria-activedescendant');
  });

  /** ↑/↓, Enter and Escape of the field; true when the key was the list's. */
  export function keydown(e: KeyboardEvent): boolean {
    if (e.isComposing) return false;
    const n = items.length;
    if (e.key === 'Escape') {
      if (!onclose) return false;
      e.preventDefault();
      e.stopPropagation();
      onclose();
      return true;
    }
    if (!n) return false;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      e.stopPropagation();
      const by = e.key === 'ArrowDown' ? 1 : -1;
      active = active < 0 ? (by > 0 ? 0 : n - 1) : (((active + by) % n) + n) % n;
      return true;
    }
    if (e.key === 'Enter' && active >= 0 && active < n) {
      e.preventDefault();
      e.stopPropagation();
      onpick(items[active], active);
      return true;
    }
    return false;
  }

  /** Where the room under the field ends: the top of its dialog's (drawer's, pane's) footer, or the window's bottom. */
  function floor(node: Element): number {
    const vh = document.documentElement.clientHeight;
    const box = node.closest('.dialog, .drawer, .pane, .modal');
    const footer = box?.querySelector(':scope > .dialog-footer, :scope > .drawer-footer, :scope > .pane-footer, :scope > .modal-footer');
    return Math.min(vh, footer?.getBoundingClientRect().top ?? vh) - EDGE;
  }

  /** The list's height down to its n-th row, with the kinds' headings above it, its padding and frame. */
  function heightTo(rows: HTMLElement[], n: number): number {
    const row = rows[n - 1];
    return row.offsetTop + row.offsetHeight + 4 + 2;
  }

  /** n rows' height without the headings: they scroll away, and the same room must not show n + 1 rows then. */
  function roomFor(rows: HTMLElement[], n: number): number {
    return Math.min(heightTo(rows, n), 4 + rows[0].offsetHeight * n + 4 + 2);
  }

  function layout() {
    if (!el || !anchor) return;
    const vw = document.documentElement.clientWidth;
    const field = anchor.getBoundingClientRect();
    const box = caret?.() ?? field;
    const rows = [...el.querySelectorAll<HTMLElement>('.sg-row')];
    const natural = el.scrollHeight + 2;
    const want = rows.length > ROWS ? roomFor(rows, ROWS) : natural;
    const least = rows.length > LEAST ? Math.min(heightTo(rows, LEAST), want) : want;
    const below = floor(anchor) - box.bottom - GAP;
    const above = box.top - GAP - EDGE;
    const down = below >= least || below >= above;
    const h = Math.max(0, Math.min(want, down ? below : above));
    const top = down ? box.bottom + GAP : box.top - GAP - h;
    const w = caret ? el.offsetWidth : field.width;
    const left = Math.max(EDGE, Math.min(box.left, vw - w - EDGE));
    place = `top: ${top}px; left: ${left}px; max-height: ${h}px;${caret ? '' : ` width: ${field.width}px;`}`;
  }

  $effect(() => {
    void items;
    void anchor;
    layout();
  });

  // The field moves with its page or panel: the list follows it. A scroll of the list itself is its own.
  $effect(() => {
    if (!anchor) return;
    const onScroll = (e: Event) => {
      if (el && e.target instanceof Node && el.contains(e.target)) return;
      layout();
    };
    const ro = new ResizeObserver(() => layout());
    ro.observe(anchor);
    document.addEventListener('scroll', onScroll, true);
    window.addEventListener('resize', layout);
    return () => {
      ro.disconnect();
      document.removeEventListener('scroll', onScroll, true);
      window.removeEventListener('resize', layout);
    };
  });
</script>

{#if shown}
  <!-- The field keeps the focus (mousedown); a click here is not a click outside the field's popup. -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    class="suggest"
    class:at-caret={!!caret}
    id="{uid}-list"
    role="listbox"
    tabindex="-1"
    aria-label={label}
    style={place}
    bind:this={el}
    use:portal
    onmousedown={(e) => e.preventDefault()}
    onclick={(e) => e.stopPropagation()}
  >
    {#each items as item, i (item.key)}
      {#if grouped && item.group && item.group !== items[i - 1]?.group}
        <div class="sg-group" role="presentation">{item.group}</div>
      {/if}
      {#if i === firstCreate}{@render extra?.()}{/if}
      <button
        type="button"
        class="sg-row"
        class:on={i === active}
        class:create={item.create}
        class:current={item.current}
        id="{uid}-{i}"
        role="option"
        aria-selected={i === active}
        tabindex="-1"
        onclick={() => onpick(item, i)}
      >
        {#if item.dot !== undefined}<span class="sg-dot" style:background={item.dot || undefined}></span>{/if}
        {#if item.mark}<ChipMark kind={item.mark} />{/if}
        {#if item.icon}<span class="sg-icon"><Icon name={item.icon} size={14} /></span>{/if}
        <span class="sg-text" class:mono={item.mono}>{item.text}</span>
        {#if item.hint}<span class="sg-hint">{item.hint}</span>{/if}
      </button>
    {/each}
    {#if !items.length && empty}<div class="sg-empty">{empty}</div>{/if}
    {#if firstCreate < 0}{@render extra?.()}{/if}
    {@render foot?.()}
  </div>
{/if}

<style>
  /* The glass of the dropdown list (CustomSelect .dropdown). */
  .suggest {
    --opt-hover: var(--surface-hover);
    position: fixed;
    z-index: var(--z-popover);
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    overflow-x: hidden;
    overscroll-behavior: contain;
    padding: 4px;
    background: var(--glass-bg);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-md);
    box-shadow: var(--glass-highlight), var(--shadow-lg);
    -webkit-backdrop-filter: var(--glass-filter);
    backdrop-filter: var(--glass-filter);
    outline: none;
    animation: suggest-in var(--dur-fast) var(--ease) both;
  }
  :global([data-theme='dark']) .suggest { --opt-hover: var(--pane-strong-border); }
  /* `[[` and `{{`: as wide as their rows were. */
  .suggest.at-caret { min-width: 240px; max-width: 420px; }
  @keyframes suggest-in { from { opacity: 0; } }
  @media (prefers-reduced-motion: reduce) { .suggest { animation: none; } }

  .sg-group {
    flex-shrink: 0;
    padding: 6px var(--sp-2) 2px;
    font-size: var(--fs-xs);
    color: var(--text-3);
  }
  /* A row of CustomSelect (.option): 36, grey under the pointer; the one chosen by the arrows grey with the keyboard's ring. */
  .sg-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex-shrink: 0;
    width: 100%;
    min-height: 36px;
    padding: var(--sp-2);
    line-height: 20px;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-base);
    text-align: left;
    cursor: pointer;
  }
  .sg-row:hover { background: var(--opt-hover); }
  .sg-row.on { background: var(--opt-hover); box-shadow: inset 0 0 0 1.5px var(--focus-ring); }
  .sg-row.create { color: var(--accent-text); font-weight: var(--fw-semibold); }
  .sg-row.current .sg-text { font-weight: var(--fw-medium); color: var(--accent-text-2); }
  .sg-dot { width: 8px; height: 8px; border-radius: 50%; flex-shrink: 0; background: var(--text-3); }
  .sg-icon { display: inline-flex; flex-shrink: 0; color: var(--text-3); }
  .sg-text { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sg-text.mono { font-family: var(--font-mono); font-size: var(--fs-sm); }
  .sg-hint {
    flex-shrink: 1;
    min-width: 0;
    max-width: 50%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-xs);
    color: var(--text-3);
  }
  .sg-empty { padding: var(--sp-2); font-size: var(--fs-sm); color: var(--text-3); }
  /* A finger's rows. */
  @media (pointer: coarse) { .sg-row { min-height: 44px; } }
</style>
